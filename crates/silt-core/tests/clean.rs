//! Clean on a fake tree: contents go, the folder stays, junctions are neither followed nor
//! removed, read-only files go, and "freed" is measured.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use silt_core::clean::clean_paths;
use silt_core::known::KnownDirs;

#[test]
fn clean_empties_the_folder_and_leaves_junction_targets_alone() {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("clean-tree");
    let _ = fs::remove_dir_all(&base);
    let cache = base.join("cache");
    let outside = base.join("outside");
    fs::create_dir_all(cache.join("sub").join("deeper")).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(cache.join("a.txt"), vec![b'a'; 100_000]).unwrap();
    fs::write(cache.join("sub").join("b.bin"), vec![b'b'; 50_000]).unwrap();
    fs::write(cache.join("sub").join("deeper").join("c"), b"c").unwrap();
    let ro = cache.join("sub").join("readonly.dat");
    fs::write(&ro, vec![0u8; 10_000]).unwrap();
    let mut perm = fs::metadata(&ro).unwrap().permissions();
    perm.set_readonly(true);
    fs::set_permissions(&ro, perm).unwrap();
    fs::write(outside.join("keep.txt"), b"precious").unwrap();
    let link = cache.join("link");
    let ok = std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&link)
        .arg(&outside)
        .output()
        .unwrap()
        .status
        .success();
    assert!(ok, "mklink /J failed");

    let r = clean_paths(
        "test",
        std::slice::from_ref(&cache),
        &KnownDirs::current(),
        &AtomicBool::new(false),
    );

    assert!(r.refused.is_empty(), "{:?}", r.refused);
    assert_eq!(r.deleted_files, 4);
    assert!(cache.is_dir(), "the folder itself stays");
    assert!(!cache.join("sub").exists(), "emptied subfolders go");
    assert!(link.exists(), "the junction stays");
    assert_eq!(
        fs::read(outside.join("keep.txt")).unwrap(),
        b"precious",
        "the junction's target is untouched"
    );
    assert!(
        r.skipped.iter().any(|s| s.path.ends_with("link")),
        "the junction is reported as skipped"
    );
    assert!(
        r.before >= 160_000 && r.after == 0,
        "before {} after {}",
        r.before,
        r.after
    );
    assert_eq!(r.freed(), r.before);

    fs::remove_dir(&link).unwrap();
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn clean_refuses_protected_folders() {
    let known = KnownDirs::current();
    let targets = [
        known.profile(),
        known.local(),
        known.windows(),
        PathBuf::from(r"C:\"),
    ];
    let r = clean_paths("test", &targets, &known, &AtomicBool::new(false));
    assert_eq!(r.refused.len(), targets.len(), "{:?}", r.refused);
    assert_eq!(r.deleted_files, 0);
    assert!(r.targets.is_empty());
}

#[test]
fn clean_skips_files_in_use() {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("clean-in-use");
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    let busy = base.join("busy.log");
    fs::write(&busy, b"open").unwrap();
    fs::write(base.join("free.tmp"), b"go").unwrap();
    // Opened without FILE_SHARE_DELETE: Windows refuses to delete it while it is open.
    let held = {
        use std::os::windows::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&busy)
            .unwrap()
    };
    let r = clean_paths(
        "test",
        std::slice::from_ref(&base),
        &KnownDirs::current(),
        &AtomicBool::new(false),
    );
    assert_eq!(r.deleted_files, 1);
    assert!(
        r.skipped
            .iter()
            .any(|s| s.path.ends_with("busy.log") && s.reason == "in use"),
        "{:?}",
        r.skipped
    );
    drop(held);
    fs::remove_dir_all(&base).unwrap();
}
