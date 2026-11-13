//! A made-up but plausible C: drive for the scenes and the README's pictures. It goes through the
//! same tree builder, classifier and report as a real scan, so a scene shows what the real code
//! would make of it.

use std::collections::HashMap;
use std::time::Duration;

use silt_core::category::Classifier;
use silt_core::journal::{Action, Entry};
use silt_core::known::KnownDirs;
use silt_core::rules::Rule;
use silt_core::scan::{ScanResult, ScannerKind};
use silt_core::shrink::Estimate;
use silt_core::trash::BinInfo;
use silt_core::tree::{RawEntry, TreeBuilder, flags};
use silt_core::volume::Volume;
use silt_core::{Category, report::Report};

/// 2026-11-13, so ages in the scenes never drift.
pub const NOW: u64 = 1_794_528_000;
const DAY: u64 = 86_400;
const MB: u64 = 1 << 20;
const GB: u64 = 1 << 30;

pub fn known() -> KnownDirs {
    KnownDirs::from_vars(
        [
            ("USERPROFILE", r"C:\Users\ana"),
            ("LOCALAPPDATA", r"C:\Users\ana\AppData\Local"),
            ("APPDATA", r"C:\Users\ana\AppData\Roaming"),
            ("TEMP", r"C:\Users\ana\AppData\Local\Temp"),
            ("SYSTEMROOT", r"C:\Windows"),
            ("PROGRAMFILES", r"C:\Program Files"),
            ("PROGRAMFILES(X86)", r"C:\Program Files (x86)"),
            ("PROGRAMDATA", r"C:\ProgramData"),
        ]
        .map(|(k, v)| (k.to_string(), v.to_string())),
    )
}

pub fn drives() -> Vec<Volume> {
    vec![
        volume('C', "Windows", 511_000_000_000, 0),
        volume('D', "Data", 1_000_000_000_000, 612 * GB),
    ]
}

fn volume(letter: char, label: &str, total: u64, free: u64) -> Volume {
    Volume {
        letter,
        root: format!("{letter}:\\"),
        label: label.into(),
        fs: "NTFS".into(),
        total,
        free,
        cluster: 4096,
        removable: false,
    }
}

struct Fake {
    entries: Vec<RawEntry>,
    dirs: HashMap<String, u64>,
    next: u64,
    seed: u64,
}

impl Fake {
    fn rand(&mut self) -> u64 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        self.seed
    }

    fn dir(&mut self, path: &str) -> u64 {
        if path.is_empty() {
            return 0;
        }
        if let Some(&id) = self.dirs.get(path) {
            return id;
        }
        let (parent, name) = match path.rsplit_once('\\') {
            Some((p, n)) => (self.dir(p), n),
            None => (0, path),
        };
        let id = self.next;
        self.next += 1;
        self.entries.push(RawEntry {
            id,
            parent,
            name: name.into(),
            alloc: 0,
            logical: 0,
            mtime: (NOW - 40 * DAY) as u32,
            flags: flags::DIR,
        });
        self.dirs.insert(path.to_string(), id);
        id
    }

    fn file(&mut self, path: &str, size: u64, age_days: u64) {
        let (dir, name) = path.rsplit_once('\\').unwrap_or(("", path));
        let parent = self.dir(dir);
        let id = self.next;
        self.next += 1;
        self.entries.push(RawEntry {
            id,
            parent,
            name: name.into(),
            alloc: size.div_ceil(4096) * 4096,
            logical: size,
            mtime: (NOW - age_days * DAY) as u32,
            flags: 0,
        });
    }

    /// `n` files adding up to about `total`, sizes skewed the way real folders are.
    fn spread(
        &mut self,
        dir: &str,
        stem: &str,
        exts: &[&str],
        n: u32,
        total: u64,
        age: (u64, u64),
    ) {
        let weights: Vec<u64> = (0..n)
            .map(|_| {
                let r = self.rand() % 1000 + 1;
                r * r
            })
            .collect();
        let sum: u64 = weights.iter().sum();
        for (i, w) in weights.iter().enumerate() {
            let size = ((total as u128 * *w as u128) / sum as u128) as u64;
            let span = age.1.saturating_sub(age.0) + 1;
            let days = age.0 + self.rand() % span;
            let ext = exts[i % exts.len()];
            self.file(
                &format!("{dir}\\{stem}{:04}.{ext}", i + 1),
                size.max(1),
                days,
            );
        }
    }
}

/// The scan of the made-up C:.
pub fn scan(rules: &[Rule]) -> ScanResult {
    let mut f = Fake {
        entries: Vec::new(),
        dirs: HashMap::new(),
        next: 1,
        seed: 0x9E37_79B9_7F4A_7C15,
    };
    let ana = r"Users\ana";
    let local = r"Users\ana\AppData\Local";

    // System.
    f.file("pagefile.sys", 16 * GB, 1);
    f.file("hiberfil.sys", 6_800 * MB, 1);
    f.spread(
        r"Windows\System32",
        "sys",
        &["dll", "exe", "mui"],
        900,
        9 * GB,
        (20, 900),
    );
    f.spread(
        r"Windows\WinSxS",
        "pkg",
        &["dll", "manifest"],
        700,
        11 * GB,
        (20, 900),
    );
    f.spread(r"Windows\SysWOW64", "sys", &["dll"], 300, 2 * GB, (20, 900));
    f.spread(
        r"Windows\SoftwareDistribution\Download",
        "kb",
        &["cab", "esd"],
        40,
        2_900 * MB,
        (3, 60),
    );
    f.spread(
        r"Windows\Temp",
        "tmp",
        &["tmp", "log"],
        60,
        640 * MB,
        (1, 90),
    );
    f.spread(
        "System Volume Information",
        "rp",
        &["dat"],
        6,
        4_100 * MB,
        (2, 30),
    );

    // Apps.
    for (name, files, size) in [
        ("Adobe\\Adobe Photoshop 2025", 2_400, 4_600 * MB),
        ("Microsoft Office", 3_100, 3_900 * MB),
        ("Blackmagic Design\\DaVinci Resolve", 1_900, 6_200 * MB),
        ("Blender Foundation\\Blender 4.2", 1_200, 1_400 * MB),
        ("NVIDIA Corporation", 600, 2_100 * MB),
        ("Docker", 400, 3_300 * MB),
        ("Google\\Chrome", 160, 640 * MB),
    ] {
        f.spread(
            &format!(r"Program Files\{name}"),
            "lib",
            &["dll", "exe", "pak", "bin"],
            files,
            size,
            (30, 400),
        );
    }
    f.spread(
        r"Program Files (x86)\Microsoft\EdgeWebView",
        "lib",
        &["dll", "exe"],
        300,
        900 * MB,
        (10, 200),
    );
    f.spread(
        r"Program Files\WindowsApps\Microsoft.Photos",
        "app",
        &["dll", "pri"],
        200,
        2_400 * MB,
        (10, 200),
    );
    f.spread(
        r"Program Files\WindowsApps\Microsoft.Teams",
        "app",
        &["dll", "pri"],
        200,
        1_700 * MB,
        (10, 200),
    );
    for (name, files, size) in [
        ("Discord", 300, 480 * MB),
        ("Microsoft VS Code", 1_500, 560 * MB),
        ("Figma", 200, 340 * MB),
        ("Obsidian", 120, 290 * MB),
    ] {
        f.spread(
            &format!(r"{local}\Programs\{name}"),
            "lib",
            &["dll", "exe", "asar", "node"],
            files,
            size,
            (5, 120),
        );
    }
    f.spread(
        r"ProgramData\Package Cache",
        "pkg",
        &["msi", "cab"],
        120,
        3_200 * MB,
        (60, 900),
    );

    // Games.
    f.spread(
        r"Program Files\Epic Games\Northwind Saga",
        "data",
        &["pak", "utoc", "ucas", "exe"],
        900,
        41 * GB,
        (20, 300),
    );
    f.spread(
        r"XboxGames\Tidewater Racing\Content",
        "data",
        &["pak", "bin", "dll"],
        1_200,
        58 * GB,
        (40, 400),
    );
    f.spread(
        r"XboxGames\Hollow Lantern\Content",
        "data",
        &["assets", "resource", "dll"],
        400,
        7_300 * MB,
        (90, 500),
    );

    // Developer.
    f.spread(
        &format!(r"{ana}\.rustup\toolchains\stable-x86_64-pc-windows-msvc"),
        "rust",
        &["dll", "rlib", "exe", "lib"],
        700,
        1_900 * MB,
        (10, 120),
    );
    f.spread(
        &format!(r"{ana}\.cargo\registry\src"),
        "crate",
        &["rs", "toml"],
        2_000,
        1_100 * MB,
        (10, 200),
    );
    f.spread(
        &format!(r"{ana}\.cargo\registry\cache"),
        "crate",
        &["crate"],
        900,
        900 * MB,
        (10, 200),
    );
    f.spread(
        &format!(r"{local}\npm-cache\_cacache"),
        "content",
        &["tgz", "json"],
        3_000,
        3_400 * MB,
        (5, 400),
    );
    f.spread(
        &format!(r"{local}\ms-playwright"),
        "chromium",
        &["dll", "exe", "pak"],
        300,
        1_600 * MB,
        (60, 200),
    );
    f.spread(
        &format!(r"{ana}\.gradle\caches"),
        "jar",
        &["jar", "bin"],
        900,
        2_700 * MB,
        (30, 600),
    );
    for (proj, nm, tgt) in [
        ("atlas", 900 * MB, 6_400 * MB),
        ("harbor-api", 620 * MB, 0),
        ("tideline", 1_300 * MB, 2_100 * MB),
    ] {
        f.spread(
            &format!(r"{ana}\code\{proj}\node_modules"),
            "mod",
            &["js", "map", "node", "json"],
            1_800,
            nm,
            (2, 90),
        );
        if tgt > 0 {
            f.spread(
                &format!(r"{ana}\code\{proj}\target\debug"),
                "build",
                &["rlib", "pdb", "exe", "rmeta"],
                700,
                tgt,
                (1, 30),
            );
        }
        f.spread(
            &format!(r"{ana}\code\{proj}\src"),
            "file",
            &["rs", "ts", "tsx"],
            120,
            9 * MB,
            (1, 60),
        );
    }

    // Caches.
    let chrome = format!(r"{local}\Google\Chrome\User Data");
    f.spread(
        &format!(r"{chrome}\Default\Cache\Cache_Data"),
        "f_",
        &["bin"],
        1_400,
        2_300 * MB,
        (0, 40),
    );
    f.spread(
        &format!(r"{chrome}\Default\Code Cache\js"),
        "js",
        &["bin"],
        600,
        640 * MB,
        (0, 40),
    );
    f.spread(
        &format!(r"{chrome}\Default\GPUCache"),
        "data",
        &["bin"],
        20,
        120 * MB,
        (0, 40),
    );
    f.spread(
        &format!(r"{local}\Microsoft\Edge\User Data\Default\Cache\Cache_Data"),
        "f_",
        &["bin"],
        500,
        820 * MB,
        (0, 60),
    );
    f.spread(
        &format!(r"{local}\NVIDIA\DXCache"),
        "shader",
        &["nvph"],
        300,
        3_100 * MB,
        (0, 200),
    );
    f.spread(
        &format!(r"{local}\Temp"),
        "tmp",
        &["tmp", "log", "msi"],
        800,
        1_800 * MB,
        (0, 300),
    );
    f.spread(
        &format!(r"{local}\Microsoft\Windows\Explorer"),
        "thumbcache_",
        &["db"],
        12,
        420 * MB,
        (0, 10),
    );
    f.spread(
        &format!(r"{local}\CrashDumps"),
        "app",
        &["dmp"],
        9,
        730 * MB,
        (10, 200),
    );

    // Documents, photos, videos, music.
    f.spread(
        &format!(r"{ana}\Documents\Work"),
        "report-",
        &["pdf", "docx", "xlsx", "pptx"],
        420,
        3_100 * MB,
        (5, 900),
    );
    f.spread(
        &format!(r"{ana}\Documents\Taxes"),
        "tax-",
        &["pdf", "xlsx"],
        60,
        380 * MB,
        (100, 1_400),
    );
    f.spread(
        &format!(r"{ana}\Documents\Books"),
        "book-",
        &["epub", "pdf"],
        90,
        1_400 * MB,
        (200, 1_500),
    );
    f.spread(
        &format!(r"{ana}\Pictures\Camera Roll"),
        "IMG_",
        &["jpg", "heic"],
        4_200,
        19 * GB,
        (5, 1_500),
    );
    f.spread(
        &format!(r"{ana}\Pictures\Raw 2024"),
        "DSC",
        &["cr3"],
        900,
        23 * GB,
        (300, 700),
    );
    f.spread(
        &format!(r"{ana}\Pictures\Screenshots"),
        "Screenshot ",
        &["png"],
        1_100,
        1_900 * MB,
        (0, 600),
    );
    f.spread(
        &format!(r"{ana}\Music\Library"),
        "track",
        &["flac", "mp3"],
        1_600,
        9_400 * MB,
        (200, 2_000),
    );
    f.spread(
        &format!(r"{ana}\Videos\Captures"),
        "Recording ",
        &["mp4"],
        70,
        18 * GB,
        (2, 400),
    );
    for (name, size, age) in [
        (r"Videos\Lisbon 2024\day-1.mov", 4_300 * MB, 620),
        (r"Videos\Lisbon 2024\day-2.mov", 3_800 * MB, 619),
        (r"Videos\Lisbon 2024\day-3.mov", 5_100 * MB, 618),
        (r"Videos\Lisbon 2024\drone-coast.mp4", 2_700 * MB, 617),
        (
            r"Videos\Wedding Rui e Bia\ceremony-full.mp4",
            7_900 * MB,
            880,
        ),
        (r"Videos\Wedding Rui e Bia\party.mp4", 6_200 * MB, 880),
        (r"Videos\Exports\showreel-v3-final.mp4", 1_900 * MB, 40),
        (r"Downloads\ubuntu-24.04-desktop-amd64.iso", 5_900 * MB, 540),
        (r"Downloads\Win11_24H2_English_x64.iso", 5_400 * MB, 410),
        (r"Downloads\project-assets-backup.zip", 3_600 * MB, 300),
        (r"Downloads\Blender-4.2-windows-x64.msi", 380 * MB, 200),
        (r"Downloads\old-laptop-backup.7z", 12_400 * MB, 760),
        (r"Downloads\course-recordings.zip", 2_200 * MB, 250),
        (r"Documents\Scans\house-contract.pdf", 210 * MB, 500),
    ] {
        f.file(&format!(r"{ana}\{name}"), size, age);
    }
    f.spread(
        &format!(r"{ana}\Downloads"),
        "download-",
        &["pdf", "zip", "exe", "png"],
        300,
        2_600 * MB,
        (1, 700),
    );
    f.spread(
        &format!(r"{ana}\AppData\Roaming\Code\User"),
        "state",
        &["json", "vscdb"],
        300,
        700 * MB,
        (0, 30),
    );
    f.spread(
        &format!(r"{ana}\AppData\Roaming\Spotify\Data"),
        "data",
        &["file"],
        900,
        2_300 * MB,
        (0, 60),
    );

    // The Recycle Bin.
    f.spread(
        r"$Recycle.Bin\S-1-5-21-3623811015-3361044348-30300820-1001",
        "$R",
        &["mp4", "zip", "jpg"],
        140,
        4_200 * MB,
        (2, 30),
    );

    let mut b = TreeBuilder::new("C:", 0);
    for e in f.entries {
        b.push(e);
    }
    let mut tree = b.finish();
    let known = known();
    let mut volume = drives().remove(0);
    let classification = tree.classify(&Classifier::new(&known, 'C', rules));
    let metadata = 2_300 * MB;
    let used = tree.total() + metadata;
    volume.free = volume.total - used;
    tree.add_synthetic(tree.root, "Filesystem metadata", metadata, Category::System);
    ScanResult {
        tree,
        volume,
        kind: ScannerKind::Mft,
        elapsed: Duration::from_millis(1_180),
        metadata,
        overcount: 0,
        classification,
    }
}

/// Fills the Shrink estimates the way a run on these folders would (no files to read).
pub fn estimate(report: &mut Report) {
    for r in &mut report.shrink {
        let (lo, hi) = match r.category {
            Category::Apps => (0.50, 0.58),
            Category::Games => (0.62, 0.70),
            Category::Developer => (0.40, 0.48),
            _ => (0.55, 0.65),
        };
        let eligible = r.eligible;
        r.estimate = Some(Estimate {
            folder: r.path.clone(),
            files: r.files,
            eligible,
            after_low: (eligible as f64 * lo) as u64,
            after_high: (eligible as f64 * hi) as u64,
            already: r.already,
            directstorage: r.directstorage,
        });
    }
    for c in &mut report.categories {
        c.shrink = 0;
    }
    for i in 0..report.shrink.len() {
        let (cat, mid) = (
            report.shrink[i].category,
            report.shrink[i]
                .estimate
                .as_ref()
                .map_or(0, Estimate::saving_mid),
        );
        report.categories[cat.index()].shrink += mid;
    }
}

pub fn bin() -> BinInfo {
    BinInfo {
        bytes: 4_200 * MB,
        items: 140,
    }
}

/// What a few weeks of use would have left in the journal.
pub fn journal() -> Vec<Entry> {
    let e =
        |days: u64, action, target: &str, algo: Option<&str>, before: u64, after: u64, files| {
            Entry {
                ts: NOW - days * DAY + 3_600 * 14,
                action,
                target: target.into(),
                algorithm: algo.map(Into::into),
                before,
                after,
                files,
                skipped: 0,
            }
        };
    vec![
        e(
            41,
            Action::Clean,
            "windows-update-download",
            None,
            3_400 * MB,
            0,
            212,
        ),
        e(
            41,
            Action::Clean,
            "nvidia-shader-cache",
            None,
            2_800 * MB,
            0,
            640,
        ),
        e(
            33,
            Action::Shrink,
            r"C:\Program Files\Microsoft Office",
            Some("xpress8k"),
            6_300 * MB,
            3_900 * MB,
            3_100,
        ),
        e(
            33,
            Action::Shrink,
            r"C:\Users\ana\.rustup\toolchains\stable-x86_64-pc-windows-msvc",
            Some("xpress8k"),
            3_600 * MB,
            1_900 * MB,
            700,
        ),
        e(19, Action::Trash, "38 items", None, 9_200 * MB, 0, 38),
        e(12, Action::EmptyBin, "Recycle Bin", None, 9_200 * MB, 0, 38),
        e(
            6,
            Action::Shrink,
            r"C:\Program Files\Blender Foundation\Blender 4.2",
            Some("lzx"),
            2_600 * MB,
            1_400 * MB,
            1_200,
        ),
        e(2, Action::Clean, "chrome-cache", None, 1_900 * MB, 0, 1_120),
    ]
}
