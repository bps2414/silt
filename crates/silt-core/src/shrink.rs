//! Shrink: transparent compression through the Windows Overlay Filter's file provider, what
//! `compact /c /exe:<algorithm>` does. Files keep their names and open as before; Windows
//! decompresses them in memory on read. Restore removes the compression.
//!
//! - Compress: `FSCTL_SET_EXTERNAL_BACKING` with `WOF_EXTERNAL_INFO` + `FILE_PROVIDER_EXTERNAL_INFO_V1`.
//! - Restore: `FSCTL_DELETE_EXTERNAL_BACKING`.
//! - Query: `FSCTL_GET_EXTERNAL_BACKING`.
//!
//! The estimate samples blocks per extension, compresses them with the Compression API
//! (`XPRESS_HUFF`, raw) and projects the allocated size cluster by cluster, as a range.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::GENERIC_READ;
use windows::Win32::Storage::Compression::{
    COMPRESS_ALGORITHM, COMPRESS_ALGORITHM_XPRESS_HUFF, COMPRESS_RAW, COMPRESSOR_HANDLE,
    CloseCompressor, Compress, CreateCompressor,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, OPEN_EXISTING,
};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::{
    FILE_PROVIDER_EXTERNAL_INFO_V1, FSCTL_DELETE_EXTERNAL_BACKING, FSCTL_GET_EXTERNAL_BACKING,
    FSCTL_SET_EXTERNAL_BACKING, WOF_EXTERNAL_INFO,
};
use windows::Win32::System::Threading::{
    GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN, THREAD_MODE_BACKGROUND_END,
};

use crate::clean::Skipped;
use crate::tree::{NodeId, Tree, flags};
use crate::win::{self, Handle};

const WOF_PROVIDER_FILE: u32 = 2;
const ERROR_COMPRESSION_NOT_BENEFICIAL: u32 = 344;
const ERROR_OBJECT_NOT_EXTERNALLY_BACKED: u32 = 342;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Algorithm {
    Xpress4k,
    /// "Balanced": the default.
    #[default]
    Xpress8k,
    Xpress16k,
    /// "Maximum": for what is rarely opened.
    Lzx,
}

impl Algorithm {
    pub const ALL: [Algorithm; 4] = [
        Algorithm::Xpress4k,
        Algorithm::Xpress8k,
        Algorithm::Xpress16k,
        Algorithm::Lzx,
    ];

    fn code(self) -> u32 {
        match self {
            Algorithm::Xpress4k => 0,
            Algorithm::Lzx => 1,
            Algorithm::Xpress8k => 2,
            Algorithm::Xpress16k => 3,
        }
    }

    fn from_code(c: u32) -> Option<Self> {
        Algorithm::ALL.into_iter().find(|a| a.code() == c)
    }

    /// WOF compresses each chunk of this size on its own.
    pub fn chunk(self) -> u64 {
        match self {
            Algorithm::Xpress4k => 4096,
            Algorithm::Xpress8k => 8192,
            Algorithm::Xpress16k => 16384,
            Algorithm::Lzx => 32768,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Algorithm::Xpress4k => "xpress4k",
            Algorithm::Xpress8k => "xpress8k",
            Algorithm::Xpress16k => "xpress16k",
            Algorithm::Lzx => "lzx",
        }
    }

    pub fn from_id(s: &str) -> Option<Self> {
        Algorithm::ALL
            .into_iter()
            .find(|a| a.id().eq_ignore_ascii_case(s))
    }

    /// XPRESS_HUFF on a chunk predicts XPRESS well; LZX packs tighter than it.
    fn sample_factor(self) -> f64 {
        match self {
            Algorithm::Lzx => 0.88,
            _ => 1.0,
        }
    }
}

#[derive(Deserialize)]
struct ExcludeFile {
    min_size: u64,
    extensions: Vec<String>,
    folders: Vec<String>,
    directstorage: Vec<String>,
}

/// `rules/shrink-exclude.toml`.
pub struct Excludes {
    pub min_size: u64,
    pub extensions: HashSet<String>,
    pub folders: Vec<String>,
    pub directstorage: Vec<String>,
}

impl Excludes {
    pub fn builtin() -> Self {
        let f: ExcludeFile = toml::from_str(include_str!("../../../rules/shrink-exclude.toml"))
            .expect("rules/shrink-exclude.toml");
        Excludes {
            min_size: f.min_size,
            extensions: f
                .extensions
                .into_iter()
                .map(|e| e.to_ascii_lowercase())
                .collect(),
            folders: f.folders.into_iter().map(|e| e.to_lowercase()).collect(),
            directstorage: f
                .directstorage
                .into_iter()
                .map(|e| e.to_lowercase())
                .collect(),
        }
    }

    pub fn folder_excluded(&self, name: &str) -> bool {
        let n = name.to_lowercase();
        self.folders.contains(&n)
    }

    pub fn ext_excluded(&self, ext: &str) -> bool {
        self.extensions.contains(ext)
    }
}

fn ext_of(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 && name.len() - i <= 12 => name[i + 1..].to_ascii_lowercase(),
        _ => String::new(),
    }
}

/// One file Shrink would compress.
#[derive(Clone, Debug)]
pub struct Candidate {
    pub path: PathBuf,
    pub logical: u64,
    pub alloc: u64,
    pub ext: String,
}

/// What a folder holds, as Shrink sees it.
#[derive(Clone, Debug, Default)]
pub struct FolderFiles {
    pub eligible: Vec<Candidate>,
    /// Allocated bytes of files already compressed by WOF.
    pub already: u64,
    pub already_logical: u64,
    pub directstorage: bool,
}

/// Walks the folder's files in the scan: `f(node, Some(ext))` for an eligible file,
/// `f(node, None)` for one WOF already compressed. Returns whether DirectStorage ships there.
fn visit_tree(
    tree: &Tree,
    folder: NodeId,
    ex: &Excludes,
    mut f: impl FnMut(NodeId, Option<String>),
) -> bool {
    let mut directstorage = false;
    let mut stack = vec![folder];
    while let Some(d) = stack.pop() {
        for c in tree.children(d) {
            let n = tree.node(c);
            let name = tree.name(c);
            if n.has(flags::LINK) {
                continue;
            }
            if n.is_dir() {
                if !ex.folder_excluded(name) {
                    stack.push(c);
                }
                continue;
            }
            if ex
                .directstorage
                .iter()
                .any(|d| name.eq_ignore_ascii_case(d))
            {
                directstorage = true;
            }
            if n.has(flags::WOF) {
                f(c, None);
                continue;
            }
            if n.logical < ex.min_size
                || n.has(
                    flags::NTFS_COMPRESSED
                        | flags::ENCRYPTED
                        | flags::CLOUD
                        | flags::SPARSE
                        | flags::HARDLINK
                        | flags::SYNTHETIC,
                )
            {
                continue;
            }
            let ext = ext_of(name);
            if !ex.ext_excluded(&ext) {
                f(c, Some(ext));
            }
        }
    }
    directstorage
}

/// The folder's files from the scan.
pub fn files_in_tree(tree: &Tree, folder: NodeId, ex: &Excludes) -> FolderFiles {
    let mut out = FolderFiles::default();
    out.directstorage = visit_tree(tree, folder, ex, |c, ext| {
        let n = tree.node(c);
        match ext {
            Some(ext) => out.eligible.push(Candidate {
                path: PathBuf::from(tree.path(c)),
                logical: n.logical,
                alloc: n.alloc,
                ext,
            }),
            None => {
                out.already += n.alloc;
                out.already_logical += n.logical;
            }
        }
    });
    out
}

/// Totals of [`files_in_tree`] without building paths.
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct FolderSums {
    pub eligible: u64,
    pub files: u64,
    pub already: u64,
    pub directstorage: bool,
}

pub fn sums_in_tree(tree: &Tree, folder: NodeId, ex: &Excludes) -> FolderSums {
    let mut s = FolderSums::default();
    s.directstorage = visit_tree(tree, folder, ex, |c, ext| {
        let n = tree.node(c);
        if ext.is_some() {
            s.eligible += n.alloc;
            s.files += 1;
        } else {
            s.already += n.alloc;
        }
    });
    s
}

/// The folder's files read from disk now (for an action: the scan may be stale).
pub fn files_on_disk(folder: &Path, ex: &Excludes) -> FolderFiles {
    let mut out = FolderFiles::default();
    let mut stack = vec![folder.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            let name = e.file_name().to_string_lossy().into_owned();
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                if !ex.folder_excluded(&name) {
                    stack.push(e.path());
                }
                continue;
            }
            if ex
                .directstorage
                .iter()
                .any(|d| name.eq_ignore_ascii_case(d))
            {
                out.directstorage = true;
            }
            let Ok(m) = e.metadata() else { continue };
            use std::os::windows::fs::MetadataExt;
            let a = m.file_attributes();
            let p = e.path();
            if query(&p).is_some() {
                let alloc = win::compressed_size(&p).unwrap_or(m.len());
                out.already += alloc;
                out.already_logical += m.len();
                continue;
            }
            // Compressed, encrypted, sparse, offline or cloud: leave alone.
            if m.len() < ex.min_size
                || a & (0x800 | 0x4000 | 0x200 | 0x1000 | 0x40_0000 | 0x4_0000) != 0
            {
                continue;
            }
            let ext = ext_of(&name);
            if ex.ext_excluded(&ext) {
                continue;
            }
            out.eligible.push(Candidate {
                path: p.clone(),
                logical: m.len(),
                alloc: win::compressed_size(&p).unwrap_or(m.len()),
                ext,
            });
        }
    }
    out
}

// ---------------------------------------------------------------- estimate

struct Compressor(COMPRESSOR_HANDLE);

impl Compressor {
    fn new() -> Option<Self> {
        let mut h = COMPRESSOR_HANDLE::default();
        unsafe {
            CreateCompressor(
                COMPRESS_ALGORITHM(COMPRESS_ALGORITHM_XPRESS_HUFF.0 | COMPRESS_RAW),
                None,
                &mut h,
            )
        }
        .ok()?;
        Some(Compressor(h))
    }

    /// Compressed size of `data`, capped at its own size (WOF stores such a chunk raw).
    fn size(&self, data: &[u8], out: &mut Vec<u8>) -> usize {
        out.resize(data.len() * 2 + 1024, 0);
        let mut n = 0usize;
        let ok = unsafe {
            Compress(
                self.0,
                Some(data.as_ptr().cast()),
                data.len(),
                Some(out.as_mut_ptr().cast()),
                out.len(),
                &mut n,
            )
        };
        if ok.is_err() {
            data.len()
        } else {
            n.min(data.len())
        }
    }
}

impl Drop for Compressor {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseCompressor(self.0);
        }
    }
}

/// Compression ratios seen for one extension.
#[derive(Clone, Copy, Default, Debug)]
pub struct Stat {
    n: u64,
    sum: f64,
    sumsq: f64,
}

impl Stat {
    fn add(&mut self, r: f64) {
        self.n += 1;
        self.sum += r;
        self.sumsq += r * r;
    }
    fn mean(&self) -> f64 {
        if self.n == 0 {
            1.0
        } else {
            self.sum / self.n as f64
        }
    }
    /// Standard error of the mean.
    fn se(&self) -> f64 {
        if self.n < 2 {
            return 0.25;
        }
        let m = self.mean();
        let var = (self.sumsq / self.n as f64 - m * m).max(0.0);
        (var / self.n as f64).sqrt()
    }
}

/// Ratios per extension that several samplers (threads, folders) learn together.
pub type SharedStats = Arc<Mutex<HashMap<String, Stat>>>;

/// Extensions that say nothing about the bytes inside: learned per folder even when shared.
const OPAQUE: [&str; 6] = ["", "dat", "bin", "data", "bundle", "res"];

/// Ratios learned per extension, shared across folders so each extension is sampled once.
pub struct Sampler {
    algo: Algorithm,
    stats: SharedStats,
    /// With shared stats, the folder opaque extensions are learned in.
    scope: Option<String>,
    comp: Option<Compressor>,
    buf: Vec<u8>,
    out: Vec<u8>,
    /// Bytes read so far.
    pub read: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Estimate {
    pub folder: String,
    pub files: u64,
    /// Allocated bytes of the files Shrink would compress.
    pub eligible: u64,
    /// Projected allocated bytes after, as a range.
    pub after_low: u64,
    pub after_high: u64,
    /// Already compressed by WOF.
    pub already: u64,
    pub directstorage: bool,
}

impl Estimate {
    /// Saving range (low, high).
    pub fn saving(&self) -> (u64, u64) {
        (
            self.eligible.saturating_sub(self.after_high),
            self.eligible.saturating_sub(self.after_low),
        )
    }
    pub fn saving_mid(&self) -> u64 {
        let (l, h) = self.saving();
        (l + h) / 2
    }
}

/// Samples per extension before an extension is considered known.
const SAMPLES_PER_EXT: u64 = 48;
/// Files sampled per extension.
const FILES_PER_EXT: usize = 12;

/// Files at `n` evenly spaced points along their bytes, each with how many points fell in it: a
/// file is picked in proportion to its size, so a thousand small files weigh what they weigh
/// against one big bundle, and a huge file gets as many samples as the bytes it holds.
fn pick_by_bytes<'a>(files: &[&'a Candidate], n: usize) -> Vec<(&'a Candidate, u64)> {
    let total: u64 = files.iter().map(|c| c.logical).sum();
    let mut picks: Vec<(&Candidate, u64)> = Vec::new();
    if total == 0 {
        return picks;
    }
    let (mut i, mut acc) = (0, 0u64);
    for k in 0..n as u64 {
        let target = total / n as u64 * k + total / n as u64 / 2;
        while i < files.len() && acc + files[i].logical <= target {
            acc += files[i].logical;
            i += 1;
        }
        let Some(c) = files.get(i) else { break };
        match picks.last_mut() {
            Some((p, count)) if std::ptr::eq(*p, *c) => *count += 1,
            _ => picks.push((c, 1)),
        }
    }
    picks
}

impl Sampler {
    pub fn new(algo: Algorithm) -> Self {
        Sampler {
            algo,
            stats: SharedStats::default(),
            scope: None,
            comp: Compressor::new(),
            buf: Vec::new(),
            out: Vec::new(),
            read: 0,
        }
    }

    /// A sampler that learns into `stats` with others: an extension sampled in one folder isn't
    /// sampled again in the next, except the opaque ones (no extension, `.dat`, ...).
    pub fn shared(algo: Algorithm, stats: SharedStats) -> Self {
        Sampler {
            stats,
            scope: Some(String::new()),
            ..Self::new(algo)
        }
    }

    fn key(&self, ext: &str) -> String {
        match &self.scope {
            Some(folder) if OPAQUE.contains(&ext) => format!("{folder}|{ext}"),
            _ => ext.to_string(),
        }
    }

    fn sample_file(&mut self, c: &Candidate, blocks: u64) -> u64 {
        let Some(comp) = &self.comp else { return 0 };
        let chunk = self.algo.chunk().min(16384);
        let Ok(mut f) = fs::OpenOptions::new()
            .read(true)
            .share_mode(7)
            .open(&c.path)
        else {
            return 0;
        };
        let chunks = c.logical / chunk;
        let blocks = blocks.min(chunks.max(1));
        let mut read = 0;
        self.buf.resize(chunk as usize, 0);
        for i in 0..blocks {
            let at = if blocks <= 1 {
                0
            } else {
                (chunks.saturating_sub(1) * i / (blocks - 1)) * chunk
            };
            if f.seek(SeekFrom::Start(at)).is_err() {
                break;
            }
            let Ok(n) = f.read(&mut self.buf) else { break };
            if n == 0 {
                break;
            }
            read += n as u64;
            let size = comp.size(&self.buf[..n], &mut self.out);
            let ratio = (size as f64 / n as f64 * self.algo.sample_factor()).min(1.0);
            let key = self.key(&c.ext);
            lock(&self.stats).entry(key).or_default().add(ratio);
        }
        self.read += read;
        read
    }

    /// Samples `files` (no more than `cap` bytes read) and projects their size after Shrink.
    pub fn estimate(
        &mut self,
        folder: &str,
        files: &FolderFiles,
        cluster: u64,
        cap: u64,
        cancel: &AtomicBool,
    ) -> Estimate {
        if self.scope.is_some() {
            self.scope = Some(folder.to_string());
        }
        let chunk = self.algo.chunk().min(16384);
        let mut by_ext: HashMap<&str, Vec<&Candidate>> = HashMap::new();
        for c in &files.eligible {
            by_ext.entry(&c.ext).or_default().push(c);
        }
        // Biggest extensions first, so the cap is spent where the bytes are.
        let mut exts: Vec<(&str, Vec<&Candidate>)> = by_ext.into_iter().collect();
        exts.sort_by_key(|(_, v)| std::cmp::Reverse(v.iter().map(|c| c.logical).sum::<u64>()));
        let mut spent = 0u64;
        for (ext, mut v) in exts {
            if cancel.load(Ordering::Relaxed) || spent >= cap {
                break;
            }
            let have = lock(&self.stats).get(&self.key(ext)).map_or(0, |s| s.n);
            if have >= SAMPLES_PER_EXT {
                continue;
            }
            v.sort_by_key(|c| std::cmp::Reverse(c.logical));
            let want = SAMPLES_PER_EXT - have;
            let per_pick = (want / FILES_PER_EXT as u64).max(2);
            for (c, count) in pick_by_bytes(&v, FILES_PER_EXT) {
                if spent >= cap {
                    break;
                }
                spent += self.sample_file(c, (per_pick * count).min((cap - spent) / chunk + 1));
            }
        }
        let stats = lock(&self.stats).clone();
        let fallback = {
            let all: Stat = stats.values().fold(Stat::default(), |mut a, s| {
                a.n += s.n;
                a.sum += s.sum;
                a.sumsq += s.sumsq;
                a
            });
            if all.n == 0 {
                Stat {
                    n: 1,
                    sum: 0.6,
                    sumsq: 0.36,
                }
            } else {
                all
            }
        };
        let mut e = Estimate {
            folder: folder.to_string(),
            files: files.eligible.len() as u64,
            already: files.already,
            directstorage: files.directstorage,
            ..Default::default()
        };
        for c in &files.eligible {
            e.eligible += c.alloc;
            let s = stats
                .get(&self.key(&c.ext))
                .filter(|s| s.n > 0)
                .unwrap_or(&fallback);
            let spread = (2.0 * s.se()).max(0.03);
            let project = |r: f64| -> u64 {
                let r = r.clamp(0.02, 1.0);
                if r > 0.97 {
                    return c.alloc; // WOF refuses files it can't shrink
                }
                let chunks = c.logical.div_ceil(self.algo.chunk());
                let bytes = (c.logical as f64 * r) as u64 + chunks * 8;
                win::round_up(bytes, cluster).min(c.alloc)
            };
            e.after_low += project(s.mean() - spread);
            e.after_high += project(s.mean() + spread);
        }
        e
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Lowers this thread's I/O and CPU priority while it lives (estimates run behind the UI).
pub struct Background;

impl Background {
    pub fn enter() -> Self {
        unsafe {
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN);
        }
        Background
    }
}

impl Drop for Background {
    fn drop(&mut self) {
        unsafe {
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_END);
        }
    }
}

// ---------------------------------------------------------------- compress, restore, query

fn open(path: &Path) -> std::io::Result<Handle> {
    let w = win::verbatim(path);
    let h = unsafe {
        CreateFileW(
            win::pcwstr(&w),
            GENERIC_READ.0,
            FILE_SHARE_READ | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
    }
    .map_err(win::io_err)?;
    Ok(Handle(h))
}

#[repr(C)]
struct SetInput {
    wof: WOF_EXTERNAL_INFO,
    file: FILE_PROVIDER_EXTERNAL_INFO_V1,
}

/// Compresses one file. `Ok(false)`: Windows found nothing to gain and left it as it was.
pub fn compress_file(path: &Path, algo: Algorithm) -> std::io::Result<bool> {
    let h = open(path)?;
    let input = SetInput {
        wof: WOF_EXTERNAL_INFO {
            Version: 1,
            Provider: WOF_PROVIDER_FILE,
        },
        file: FILE_PROVIDER_EXTERNAL_INFO_V1 {
            Version: 1,
            Algorithm: algo.code(),
            Flags: 0,
        },
    };
    let r = unsafe {
        DeviceIoControl(
            h.0,
            FSCTL_SET_EXTERNAL_BACKING,
            Some((&input as *const SetInput).cast()),
            std::mem::size_of::<SetInput>() as u32,
            None,
            0,
            None,
            None,
        )
    };
    match r {
        Ok(()) => Ok(true),
        Err(e) if (e.code().0 & 0xFFFF) as u32 == ERROR_COMPRESSION_NOT_BENEFICIAL => Ok(false),
        Err(e) => Err(win::io_err(e)),
    }
}

/// Removes WOF compression from one file. `Ok(false)`: it wasn't compressed.
pub fn restore_file(path: &Path) -> std::io::Result<bool> {
    let h = open(path)?;
    let r = unsafe {
        DeviceIoControl(
            h.0,
            FSCTL_DELETE_EXTERNAL_BACKING,
            None,
            0,
            None,
            0,
            None,
            None,
        )
    };
    match r {
        Ok(()) => Ok(true),
        Err(e) if (e.code().0 & 0xFFFF) as u32 == ERROR_OBJECT_NOT_EXTERNALLY_BACKED => Ok(false),
        Err(e) => Err(win::io_err(e)),
    }
}

/// The WOF algorithm a file is compressed with, if any.
pub fn query(path: &Path) -> Option<Algorithm> {
    let h = open(path).ok()?;
    let mut out = SetInput {
        wof: WOF_EXTERNAL_INFO::default(),
        file: FILE_PROVIDER_EXTERNAL_INFO_V1::default(),
    };
    let mut got = 0u32;
    unsafe {
        DeviceIoControl(
            h.0,
            FSCTL_GET_EXTERNAL_BACKING,
            None,
            0,
            Some((&mut out as *mut SetInput).cast()),
            std::mem::size_of::<SetInput>() as u32,
            Some(&mut got),
            None,
        )
    }
    .ok()?;
    if out.wof.Provider != WOF_PROVIDER_FILE {
        return None;
    }
    Algorithm::from_code(out.file.Algorithm)
}

/// Pause, cancel and progress shared with the UI.
#[derive(Default)]
pub struct Control {
    pub cancel: AtomicBool,
    pub pause: AtomicBool,
    pub done_bytes: AtomicU64,
    pub total_bytes: AtomicU64,
    pub done_files: AtomicU64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ShrinkReport {
    pub folder: String,
    pub algorithm: Option<Algorithm>,
    /// Allocated bytes of the touched files, before and after, measured.
    pub before: u64,
    pub after: u64,
    pub files: u64,
    pub not_beneficial: u64,
    pub skipped: Vec<Skipped>,
    pub cancelled: bool,
    pub refused: Option<String>,
}

impl ShrinkReport {
    pub fn saved(&self) -> i64 {
        self.before as i64 - self.after as i64
    }
}

/// Free space to keep while compressing or restoring.
const MARGIN: u64 = 512 << 20;

/// Compresses every eligible file under `folder`, in parallel (half the cores).
pub fn shrink_folder(folder: &Path, algo: Algorithm, ex: &Excludes, ctl: &Control) -> ShrinkReport {
    let mut r = ShrinkReport {
        folder: folder.to_string_lossy().into_owned(),
        algorithm: Some(algo),
        ..Default::default()
    };
    let files = files_on_disk(folder, ex);
    let largest = files.eligible.iter().map(|c| c.logical).max().unwrap_or(0);
    let free = crate::volume::free_for(&r.folder);
    if free < largest + MARGIN {
        r.refused = Some(format!(
            "needs {} free, {} available",
            crate::fmt_bytes(largest + MARGIN),
            crate::fmt_bytes(free)
        ));
        return r;
    }
    let cluster = crate::volume::info(r.folder.chars().next().unwrap_or('C'))
        .map(|v| v.cluster)
        .unwrap_or(4096);
    run_parallel(files.eligible, ctl, &mut r, |c, r| {
        match compress_file(&c.path, algo) {
            Ok(true) => {
                r.before += c.alloc;
                r.after +=
                    win::compressed_size(&c.path).map_or(c.alloc, |s| win::round_up(s, cluster));
                r.files += 1;
            }
            Ok(false) => r.not_beneficial += 1,
            Err(e) => r.skipped.push(skip(&c.path, &e)),
        }
    });
    r
}

/// Restores every WOF-compressed file under `folder`.
pub fn restore_folder(folder: &Path, ctl: &Control) -> ShrinkReport {
    let mut r = ShrinkReport {
        folder: folder.to_string_lossy().into_owned(),
        ..Default::default()
    };
    let files = files_on_disk(
        folder,
        &Excludes {
            min_size: 0,
            extensions: HashSet::new(),
            folders: Vec::new(),
            directstorage: Vec::new(),
        },
    );
    let grow = files.already_logical.saturating_sub(files.already);
    let free = crate::volume::free_for(&r.folder);
    if free < grow + MARGIN {
        r.refused = Some(format!(
            "needs {} free, {} available",
            crate::fmt_bytes(grow + MARGIN),
            crate::fmt_bytes(free)
        ));
        return r;
    }
    let cluster = crate::volume::info(r.folder.chars().next().unwrap_or('C'))
        .map(|v| v.cluster)
        .unwrap_or(4096);
    // Only compressed files: list them again with their sizes.
    let mut wof = Vec::new();
    let mut stack = vec![folder.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                stack.push(e.path());
            } else if query(&e.path()).is_some() {
                let p = e.path();
                let logical = e.metadata().map(|m| m.len()).unwrap_or(0);
                let alloc = win::compressed_size(&p).map_or(logical, |s| win::round_up(s, cluster));
                wof.push(Candidate {
                    path: p,
                    logical,
                    alloc,
                    ext: String::new(),
                });
            }
        }
    }
    run_parallel(wof, ctl, &mut r, |c, r| match restore_file(&c.path) {
        Ok(_) => {
            r.before += c.alloc;
            r.after +=
                win::compressed_size(&c.path).map_or(c.logical, |s| win::round_up(s, cluster));
            r.files += 1;
        }
        Err(e) => r.skipped.push(skip(&c.path, &e)),
    });
    r
}

fn skip(p: &Path, e: &std::io::Error) -> Skipped {
    Skipped {
        path: p.to_string_lossy().into_owned(),
        reason: match win::os_code(e) {
            32 | 33 => "in use".into(),
            5 => "access denied".into(),
            _ => e.to_string(),
        },
    }
}

fn run_parallel(
    files: Vec<Candidate>,
    ctl: &Control,
    r: &mut ShrinkReport,
    f: impl Fn(&Candidate, &mut ShrinkReport) + Sync,
) {
    ctl.total_bytes
        .fetch_add(files.iter().map(|c| c.logical).sum(), Ordering::Relaxed);
    let threads = (std::thread::available_parallelism().map_or(4, |n| n.get()) / 2).max(1);
    let queue = Mutex::new(files);
    let shared = Mutex::new(std::mem::take(r));
    let f = &f;
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                let mut local = ShrinkReport::default();
                loop {
                    while ctl.pause.load(Ordering::Relaxed) && !ctl.cancel.load(Ordering::Relaxed) {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    if ctl.cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let Some(c) = queue.lock().unwrap_or_else(|e| e.into_inner()).pop() else {
                        break;
                    };
                    f(&c, &mut local);
                    ctl.done_bytes.fetch_add(c.logical, Ordering::Relaxed);
                    ctl.done_files.fetch_add(1, Ordering::Relaxed);
                }
                let mut g = shared.lock().unwrap_or_else(|e| e.into_inner());
                g.before += local.before;
                g.after += local.after;
                g.files += local.files;
                g.not_beneficial += local.not_beneficial;
                g.skipped.extend(local.skipped);
            });
        }
    });
    *r = shared.into_inner().unwrap_or_else(|e| e.into_inner());
    r.cancelled = ctl.cancel.load(Ordering::Relaxed);
}

/// Shared handle for a running action.
pub type SharedControl = Arc<Control>;
