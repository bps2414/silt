//! `WalkScanner`: lists folders in parallel with `FindFirstFileExW` (large fetch, basic info).
//! Works on any file system and without admin. Allocated size is the logical size rounded up
//! to a cluster, or what Windows reports for compressed, sparse and cloud files.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex};

use windows::Win32::Storage::FileSystem::{
    FIND_FIRST_EX_LARGE_FETCH, FindClose, FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW,
    FindNextFileW, WIN32_FIND_DATAW,
};

use super::{Progress, ScanMsg, flush};
use crate::tree::{RawEntry, flags};
use crate::win;

pub const ROOT: u64 = 0;

struct Queue {
    dirs: Mutex<(Vec<(u64, PathBuf)>, usize)>, // (pending folders, folders being listed)
    cv: Condvar,
}

pub fn run(
    root: &str,
    cluster: u64,
    tx: Sender<ScanMsg>,
    progress: Arc<Progress>,
    cancel: Arc<AtomicBool>,
) {
    run_from(PathBuf::from(root), cluster, tx, progress, cancel);
}

/// Walks `root` (a drive root or any folder). Ids are dense, `0` is the root.
pub fn run_from(
    root: PathBuf,
    cluster: u64,
    tx: Sender<ScanMsg>,
    progress: Arc<Progress>,
    cancel: Arc<AtomicBool>,
) {
    let q = Arc::new(Queue {
        dirs: Mutex::new((vec![(ROOT, root)], 0)),
        cv: Condvar::new(),
    });
    let next_id = Arc::new(AtomicU64::new(1));
    let threads = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .clamp(2, 8);
    let workers: Vec<_> = (0..threads)
        .map(|_| {
            let (q, tx, progress, cancel, next_id) = (
                q.clone(),
                tx.clone(),
                progress.clone(),
                cancel.clone(),
                next_id.clone(),
            );
            std::thread::spawn(move || worker(&q, cluster, &tx, &progress, &cancel, &next_id))
        })
        .collect();
    for w in workers {
        let _ = w.join();
    }
    let _ = tx.send(ScanMsg::Done);
}

fn worker(
    q: &Queue,
    cluster: u64,
    tx: &Sender<ScanMsg>,
    progress: &Progress,
    cancel: &AtomicBool,
    next_id: &AtomicU64,
) {
    let mut batch = Vec::with_capacity(4096);
    loop {
        let job = {
            let mut g = q.dirs.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if cancel.load(Ordering::Relaxed) {
                    q.cv.notify_all();
                    return;
                }
                if let Some(job) = g.0.pop() {
                    g.1 += 1;
                    break job;
                }
                if g.1 == 0 {
                    q.cv.notify_all();
                    drop(g);
                    flush(tx, &mut batch, progress);
                    return;
                }
                g = q.cv.wait(g).unwrap_or_else(|e| e.into_inner());
            }
        };
        let mut found = Vec::new();
        list(&job.1, job.0, cluster, next_id, &mut batch, &mut found);
        if batch.len() >= 2048 && !flush(tx, &mut batch, progress) {
            cancel.store(true, Ordering::Relaxed);
        }
        let mut g = q.dirs.lock().unwrap_or_else(|e| e.into_inner());
        g.0.extend(found);
        g.1 -= 1;
        q.cv.notify_all();
    }
}

fn list(
    dir: &Path,
    dir_id: u64,
    cluster: u64,
    next_id: &AtomicU64,
    out: &mut Vec<RawEntry>,
    found: &mut Vec<(u64, PathBuf)>,
) {
    let pattern = win::verbatim(&dir.join("*"));
    let mut data = WIN32_FIND_DATAW::default();
    let Ok(h) = (unsafe {
        FindFirstFileExW(
            win::pcwstr(&pattern),
            FindExInfoBasic,
            (&mut data as *mut WIN32_FIND_DATAW).cast(),
            FindExSearchNameMatch,
            None,
            FIND_FIRST_EX_LARGE_FETCH,
        )
    }) else {
        return;
    };
    loop {
        let len = data.cFileName.iter().position(|&c| c == 0).unwrap_or(260);
        let name = String::from_utf16_lossy(&data.cFileName[..len]);
        if name != "." && name != ".." {
            let a = data.dwFileAttributes;
            let tag = if a & 0x400 != 0 { data.dwReserved0 } else { 0 };
            let mut f = 0u16;
            if a & 0x10 != 0 {
                f |= flags::DIR;
            }
            if a & 0x2 != 0 {
                f |= flags::HIDDEN;
            }
            if a & 0x4 != 0 {
                f |= flags::SYSTEM_ATTR;
            }
            if a & 0x800 != 0 {
                f |= flags::NTFS_COMPRESSED;
            }
            if a & 0x200 != 0 {
                f |= flags::SPARSE;
            }
            if a & 0x4000 != 0 {
                f |= flags::ENCRYPTED;
            }
            if win::is_cloud_tag(tag) || a & (0x1000 | 0x40_0000) != 0 {
                f |= flags::CLOUD;
            }
            if tag == win::TAG_WOF {
                f |= flags::WOF;
            }
            if win::is_name_surrogate(tag) {
                f |= flags::LINK;
            }
            let logical = ((data.nFileSizeHigh as u64) << 32) | data.nFileSizeLow as u64;
            let path = dir.join(&name);
            let alloc = if f & flags::DIR != 0 {
                0
            } else if f & (flags::NTFS_COMPRESSED | flags::SPARSE | flags::CLOUD | flags::WOF) != 0
            {
                win::compressed_size(&path).map_or(0, |s| win::round_up(s, cluster))
            } else if logical > cluster {
                // WOF hides its reparse point from directory listings: ask for the real size.
                match win::compressed_size(&path) {
                    Some(s) if s < logical => {
                        f |= flags::WOF;
                        win::round_up(s, cluster)
                    }
                    _ => win::round_up(logical, cluster),
                }
            } else {
                win::round_up(logical, cluster)
            };
            let id = next_id.fetch_add(1, Ordering::Relaxed);
            out.push(RawEntry {
                id,
                parent: dir_id,
                name,
                alloc,
                logical,
                mtime: win::filetime_unix(data.ftLastWriteTime),
                flags: f,
            });
            if f & flags::DIR != 0 && f & flags::LINK == 0 {
                found.push((id, path));
            }
        }
        if unsafe { FindNextFileW(h, &mut data) }.is_err() {
            break;
        }
    }
    unsafe {
        let _ = FindClose(h);
    }
}
