//! Trash (send to the Recycle Bin) and Empty Recycle Bin. There is no direct permanent delete
//! of a user's file: the way out is Trash, then Empty Recycle Bin.

use std::path::PathBuf;

use serde::Serialize;
use windows::Win32::UI::Shell::{
    SHERB_NOCONFIRMATION, SHERB_NOPROGRESSUI, SHERB_NOSOUND, SHEmptyRecycleBinW, SHQUERYRBINFO,
    SHQueryRecycleBinW,
};
use windows::core::PCWSTR;

use crate::clean::{Skipped, measure};
use crate::known::KnownDirs;
use crate::rules::{guard, protected_folders};

#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq)]
pub struct BinInfo {
    /// Bytes the Recycle Bin holds, on every drive.
    pub bytes: u64,
    pub items: u64,
}

/// What sits in the Recycle Bin right now, on every drive.
pub fn bin_info() -> BinInfo {
    query_bin(PCWSTR::null())
}

/// What sits in the Recycle Bin of one drive (`C:\`).
pub fn bin_info_drive(root: &str) -> BinInfo {
    let w = crate::win::wide(root);
    query_bin(crate::win::pcwstr(&w))
}

fn query_bin(root: PCWSTR) -> BinInfo {
    let mut q = SHQUERYRBINFO {
        cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32,
        ..Default::default()
    };
    match unsafe { SHQueryRecycleBinW(root, &mut q) } {
        Ok(()) => BinInfo {
            bytes: q.i64Size.max(0) as u64,
            items: q.i64NumItems.max(0) as u64,
        },
        Err(_) => BinInfo::default(),
    }
}

/// Empties the Recycle Bin of every drive. Returns what it held.
pub fn empty_bin() -> std::io::Result<BinInfo> {
    let before = bin_info();
    if before.items == 0 {
        return Ok(before);
    }
    unsafe {
        SHEmptyRecycleBinW(
            None,
            PCWSTR::null(),
            SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND,
        )
    }
    .map_err(crate::win::io_err)?;
    Ok(before)
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct TrashReport {
    pub trashed: Vec<String>,
    /// Allocated bytes of what went to the Recycle Bin (freed once it is emptied).
    pub bytes: u64,
    pub refused: Vec<Skipped>,
    pub failed: Vec<Skipped>,
}

/// Sends files and folders to the Recycle Bin, one by one so one failure doesn't stop the rest.
pub fn send_to_trash(paths: &[PathBuf], known: &KnownDirs) -> TrashReport {
    let protected = protected_folders(known);
    let mut r = TrashReport::default();
    let all: Vec<String> = paths
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    for p in paths {
        let s = p.to_string_lossy().into_owned();
        // Inside another selected folder: it goes with that folder.
        if all
            .iter()
            .any(|o| o.len() < s.len() && crate::win::path_within(&s, o))
        {
            continue;
        }
        if let Err(why) = guard(p, &protected, true) {
            r.refused.push(Skipped {
                path: s,
                reason: why.to_string(),
            });
            continue;
        }
        let size = measure(p).0;
        match trash::delete(p) {
            Ok(()) => {
                r.bytes += size;
                r.trashed.push(s);
            }
            Err(e) => r.failed.push(Skipped {
                path: s,
                reason: e.to_string(),
            }),
        }
    }
    r
}
