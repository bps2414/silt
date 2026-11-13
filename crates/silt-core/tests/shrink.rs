//! Shrink round trip: files compress, read back byte for byte, take less space, and restore to
//! exactly what they were. Excluded files are left alone.

use std::fs;
use std::path::{Path, PathBuf};

use silt_core::shrink::{self, Algorithm, Control, Excludes};
use silt_core::win;

fn noise(n: usize, mut seed: u64) -> Vec<u8> {
    (0..n)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed as u8
        })
        .collect()
}

fn text(n: usize) -> Vec<u8> {
    let line = b"fn main() { println!(\"compressible text compresses well, again and again\"); }\n";
    line.iter().copied().cycle().take(n).collect()
}

fn setup(name: &str) -> (PathBuf, Vec<(PathBuf, Vec<u8>)>) {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(base.join("bin")).unwrap();
    fs::create_dir_all(base.join("EasyAntiCheat")).unwrap();
    let files = vec![
        (base.join("bin").join("app.dll"), text(3 << 20)),
        (base.join("bin").join("data.json"), text(700_000)),
        (base.join("random.bin"), noise(1 << 20, 7)),
        (base.join("tiny.txt"), b"too small to matter".to_vec()),
        (base.join("movie.mp4"), text(1 << 20)),
        (base.join("EasyAntiCheat").join("eac.dll"), text(1 << 20)),
    ];
    for (p, d) in &files {
        fs::write(p, d).unwrap();
    }
    (base, files)
}

fn alloc(p: &Path) -> u64 {
    win::compressed_size(p).unwrap()
}

#[test]
fn shrink_and_restore_round_trip() {
    let (base, files) = setup("shrink-roundtrip");
    let before: u64 = files.iter().map(|(p, _)| alloc(p)).sum();

    let ctl = Control::default();
    let r = shrink::shrink_folder(&base, Algorithm::Xpress8k, &Excludes::builtin(), &ctl);
    assert!(r.refused.is_none(), "{:?}", r.refused);
    assert!(r.skipped.is_empty(), "{:?}", r.skipped);
    assert_eq!(r.files, 2, "app.dll and data.json");
    assert_eq!(r.not_beneficial, 1, "random.bin");
    assert!(
        r.after * 4 < r.before,
        "before {} after {}",
        r.before,
        r.after
    );

    let q = |name: &str| shrink::query(&files.iter().find(|(p, _)| p.ends_with(name)).unwrap().0);
    assert_eq!(q("app.dll"), Some(Algorithm::Xpress8k));
    assert_eq!(q("data.json"), Some(Algorithm::Xpress8k));
    assert_eq!(q("random.bin"), None, "nothing to gain");
    assert_eq!(q("tiny.txt"), None, "below the minimum size");
    assert_eq!(q("movie.mp4"), None, "excluded extension");
    assert_eq!(q("eac.dll"), None, "anti-cheat folder");
    for (p, d) in &files {
        assert_eq!(
            &fs::read(p).unwrap(),
            d,
            "{} reads back the same",
            p.display()
        );
    }
    let shrunk: u64 = files.iter().map(|(p, _)| alloc(p)).sum();
    assert!(shrunk < before, "{shrunk} < {before}");

    // The scan-free check the app uses before acting sees the compressed files.
    let on_disk = shrink::files_on_disk(&base, &Excludes::builtin());
    assert!(on_disk.already > 0);

    let ctl = Control::default();
    let r = shrink::restore_folder(&base, &ctl);
    assert!(r.refused.is_none(), "{:?}", r.refused);
    assert_eq!(r.files, 2);
    for (p, d) in &files {
        assert_eq!(shrink::query(p), None);
        assert_eq!(
            &fs::read(p).unwrap(),
            d,
            "{} is identical after restore",
            p.display()
        );
    }
    let restored: u64 = files.iter().map(|(p, _)| alloc(p)).sum();
    assert_eq!(restored, before);
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn every_algorithm_compresses_and_reads_back() {
    let (base, files) = setup("shrink-algorithms");
    let (p, d) = &files[0];
    for algo in Algorithm::ALL {
        assert!(shrink::compress_file(p, algo).unwrap(), "{algo:?}");
        assert_eq!(shrink::query(p), Some(algo));
        assert_eq!(&fs::read(p).unwrap(), d, "{algo:?} reads back the same");
        assert!(shrink::restore_file(p).unwrap());
        assert!(
            !shrink::restore_file(p).unwrap(),
            "restoring twice is a no-op"
        );
    }
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn estimate_brackets_the_real_saving() {
    let (base, _) = setup("shrink-estimate");
    let ex = Excludes::builtin();
    let found = shrink::files_on_disk(&base, &ex);
    let cluster = silt_core::volume::info(base.to_string_lossy().chars().next().unwrap())
        .unwrap()
        .cluster;
    let e = shrink::Sampler::new(Algorithm::Xpress8k).estimate(
        "t",
        &found,
        cluster,
        64 << 20,
        &Default::default(),
    );
    let r = shrink::shrink_folder(&base, Algorithm::Xpress8k, &ex, &Control::default());
    let saved = r.saved() as u64;
    let (lo, hi) = e.saving();
    let slack = e.eligible / 10;
    assert!(
        lo.saturating_sub(slack) <= saved && saved <= hi + slack,
        "estimate {lo}..{hi}, saved {saved}"
    );
    shrink::restore_folder(&base, &Control::default());
    fs::remove_dir_all(&base).unwrap();
}
