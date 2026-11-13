//! Clean rules resolve where they should, and the guards refuse what must never be deleted.

use std::path::{Path, PathBuf};

use silt_core::known::KnownDirs;
use silt_core::rules::{self, Refusal, guard, protected_folders};

fn fake_known() -> KnownDirs {
    KnownDirs::from_vars([
        ("USERPROFILE".into(), r"C:\Users\me".into()),
        ("LOCALAPPDATA".into(), r"C:\Users\me\AppData\Local".into()),
        ("APPDATA".into(), r"C:\Users\me\AppData\Roaming".into()),
        ("TEMP".into(), r"C:\Users\me\AppData\Local\Temp".into()),
        ("SystemRoot".into(), r"C:\Windows".into()),
        ("npm_config_cache".into(), r"D:\Dev\Caches\npm".into()),
        ("CARGO_HOME".into(), r"D:\Dev\Tools\cargo".into()),
    ])
}

#[test]
fn builtin_rules_parse_and_ids_are_unique() {
    let all = rules::builtin();
    assert!(all.len() >= 30, "only {} rules", all.len());
    let mut ids: Vec<&str> = all.iter().map(|r| r.id.as_str()).collect();
    ids.sort();
    let n = ids.len();
    ids.dedup();
    assert_eq!(ids.len(), n, "duplicate rule ids");
    for r in &all {
        assert!(!r.note.is_empty(), "{} has no note", r.id);
        assert!(r.note_pt_br.is_some(), "{} has no pt-BR note", r.id);
    }
}

#[test]
fn rules_reject_bad_files() {
    assert!(
        rules::parse(
            "[[rule]]\nid='x'\nname='x'\ngroup='nope'\npaths=['C:\\\\x']\nsafety='safe'\nnote='x'"
        )
        .is_err()
    );
    assert!(
        rules::parse(
            "[[rule]]\nid='x'\nname='x'\ngroup='system'\npaths=[]\nsafety='safe'\nnote='x'"
        )
        .is_err()
    );
    assert!(rules::parse("[[rule]]\nid='x'\nname='x'\ngroup='system'\npaths=['C:\\\\x']\nsafety='safe'\nnote='x'\nextra=1").is_err());
}

#[test]
fn env_paths_follow_relocated_caches() {
    let known = fake_known();
    let all = rules::builtin();
    let npm = all
        .iter()
        .find(|r| r.id == "npm-cache")
        .expect("npm-cache rule");
    let p = npm.patterns(&known);
    assert!(
        p.iter()
            .any(|x| x.eq_ignore_ascii_case(r"D:\Dev\Caches\npm")),
        "{p:?}"
    );
    let cargo = all
        .iter()
        .find(|r| r.id == "cargo-registry")
        .expect("cargo-registry rule");
    let p = cargo.patterns(&known);
    assert!(
        p.iter()
            .any(|x| x.eq_ignore_ascii_case(r"D:\Dev\Tools\cargo\registry\cache")),
        "{p:?}"
    );
    let temp = all
        .iter()
        .find(|r| r.id == "user-temp")
        .expect("user-temp rule");
    assert!(
        temp.patterns(&known)
            .iter()
            .any(|x| x.eq_ignore_ascii_case(r"C:\Users\me\AppData\Local\Temp"))
    );
}

#[test]
fn no_rule_points_at_a_protected_folder() {
    let known = fake_known();
    let protected = protected_folders(&known);
    for r in rules::builtin() {
        for p in r.patterns(&known) {
            if p.contains('*') {
                continue;
            }
            assert!(
                guard(Path::new(&p), &protected, false).is_ok(),
                "rule {} resolves to guarded {p}",
                r.id
            );
        }
    }
}

#[test]
fn guards_refuse_roots_profiles_and_system_folders() {
    let known = fake_known();
    let protected = protected_folders(&known);
    let refuse = |p: &str| guard(Path::new(p), &protected, true).unwrap_err();
    assert_eq!(refuse(r"relative\path"), Refusal::NotAbsolute);
    assert_eq!(refuse(r"C:\"), Refusal::DriveRoot);
    assert!(matches!(refuse(r"C:\Users\me"), Refusal::Protected(_)));
    assert!(matches!(
        refuse(r"C:\USERS\ME\Documents\"),
        Refusal::Protected(_)
    ));
    assert!(matches!(refuse(r"C:\Users"), Refusal::Protected(_)));
    assert!(matches!(
        refuse(r"C:\Windows\System32"),
        Refusal::Protected(_)
    ));
    assert!(matches!(
        refuse(r"C:\Users\me\AppData"),
        Refusal::ContainsProtected(_)
    ));
    assert!(matches!(refuse(r"C:\Program Files"), Refusal::Protected(_)));
    assert!(
        guard(
            Path::new(r"C:\Users\me\AppData\Local\Temp"),
            &protected,
            false
        )
        .is_ok()
    );
    assert!(
        guard(
            Path::new(r"C:\Users\me\Downloads\old.iso"),
            &protected,
            true
        )
        .is_ok()
    );
}

#[test]
fn guards_refuse_to_empty_a_junction() {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("guard-junction");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("real")).unwrap();
    let link = base.join("link");
    let ok = std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&link)
        .arg(base.join("real"))
        .output()
        .unwrap()
        .status
        .success();
    assert!(ok, "mklink /J failed");
    let protected = protected_folders(&KnownDirs::current());
    assert_eq!(guard(&link, &protected, false), Err(Refusal::Link));
    // Sending the junction itself to the Recycle Bin is fine: only the link goes.
    assert!(guard(&link, &protected, true).is_ok());
    std::fs::remove_dir(&link).unwrap();
    std::fs::remove_dir_all(&base).unwrap();
}
