//! Clean: empties the folders of a rule. The folder itself stays; junctions and symlinks are
//! neither followed nor removed; files in use are skipped and listed. "Freed" is measured after
//! the deletion, never estimated.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;

use serde::Serialize;

use crate::known::KnownDirs;
use crate::rules::{Refusal, Rule, guard, protected_folders};
use crate::scan::{Progress, ScanMsg, walk};

#[derive(Clone, Debug, Default, Serialize)]
pub struct Skipped {
    pub path: String,
    pub reason: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct CleanReport {
    pub rule: String,
    pub targets: Vec<String>,
    /// Allocated bytes before and after, measured on disk.
    pub before: u64,
    pub after: u64,
    pub deleted_files: u64,
    pub skipped: Vec<Skipped>,
    pub refused: Vec<Skipped>,
    pub cancelled: bool,
}

impl CleanReport {
    pub fn freed(&self) -> u64 {
        self.before.saturating_sub(self.after)
    }
}

/// Allocated bytes and file count under `path` (a folder or a file), links not followed.
pub fn measure(path: &Path) -> (u64, u64) {
    let cluster = crate::volume::info(path.to_string_lossy().chars().next().unwrap_or('C'))
        .map(|v| v.cluster)
        .unwrap_or(4096);
    let Ok(meta) = fs::symlink_metadata(path) else {
        return (0, 0);
    };
    if !meta.is_dir() {
        return (
            crate::win::compressed_size(path).map_or(0, |s| crate::win::round_up(s, cluster)),
            1,
        );
    }
    let (tx, rx) = channel();
    let progress = Arc::new(Progress::default());
    walk::run_from(
        path.to_path_buf(),
        cluster,
        tx,
        progress,
        Arc::new(AtomicBool::new(false)),
    );
    let (mut bytes, mut files) = (0, 0);
    while let Ok(ScanMsg::Batch(b)) = rx.recv() {
        for e in b {
            bytes += e.alloc;
            files += u64::from(e.flags & crate::tree::flags::DIR == 0);
        }
    }
    (bytes, files)
}

/// Runs one rule. `cancel` stops between files.
pub fn clean_rule(rule: &Rule, known: &KnownDirs, cancel: &AtomicBool) -> CleanReport {
    let targets = rule.targets(known);
    clean_paths(&rule.id, &targets, known, cancel)
}

/// Empties each folder in `targets` (or deletes a target that is a file).
pub fn clean_paths(
    rule: &str,
    targets: &[PathBuf],
    known: &KnownDirs,
    cancel: &AtomicBool,
) -> CleanReport {
    let protected = protected_folders(known);
    let mut r = CleanReport {
        rule: rule.to_string(),
        ..Default::default()
    };
    for t in targets {
        if let Err(why) = guard(t, &protected, false) {
            r.refused.push(Skipped {
                path: t.to_string_lossy().into_owned(),
                reason: why.to_string(),
            });
            continue;
        }
        r.targets.push(t.to_string_lossy().into_owned());
        r.before += measure(t).0;
        match fs::symlink_metadata(t) {
            Ok(m) if m.is_dir() => empty_dir(t, &mut r, cancel),
            Ok(_) => delete_file(t, &mut r),
            Err(_) => {}
        }
        r.after += measure(t).0;
        if cancel.load(Ordering::Relaxed) {
            r.cancelled = true;
            break;
        }
    }
    r
}

fn empty_dir(dir: &Path, r: &mut CleanReport, cancel: &AtomicBool) {
    let Ok(rd) = fs::read_dir(dir) else {
        r.skipped.push(Skipped {
            path: dir.to_string_lossy().into_owned(),
            reason: "cannot list".into(),
        });
        return;
    };
    for e in rd.flatten() {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let p = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_symlink() {
            // A junction or symlink: its target is someone else's. Leave it.
            r.skipped.push(Skipped {
                path: p.to_string_lossy().into_owned(),
                reason: Refusal::Link.to_string(),
            });
        } else if ft.is_dir() {
            empty_dir(&p, r, cancel);
            let _ = fs::remove_dir(&p); // only succeeds when everything inside went
        } else {
            delete_file(&p, r);
        }
    }
}

fn delete_file(p: &Path, r: &mut CleanReport) {
    let result = fs::remove_file(p).or_else(|e| {
        // Read-only files: clear the flag and retry once.
        if e.kind() == io::ErrorKind::PermissionDenied
            && let Ok(m) = fs::metadata(p)
            && m.permissions().readonly()
        {
            let mut perm = m.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            perm.set_readonly(false);
            fs::set_permissions(p, perm)?;
            return fs::remove_file(p);
        }
        Err(e)
    });
    match result {
        Ok(()) => r.deleted_files += 1,
        Err(e) => r.skipped.push(Skipped {
            path: p.to_string_lossy().into_owned(),
            reason: match crate::win::os_code(&e) {
                32 | 33 => "in use".into(),
                5 => "access denied".into(),
                _ => e.to_string(),
            },
        }),
    }
}
