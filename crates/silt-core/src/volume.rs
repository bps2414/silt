//! Drives: letter, file system, size, free space and cluster size.

use std::io;

use serde::Serialize;
use windows::Win32::Storage::FileSystem::{
    GetDiskFreeSpaceExW, GetDiskFreeSpaceW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW,
};

use crate::win::{io_err, pcwstr, wide};

#[derive(Clone, Debug, Serialize)]
pub struct Volume {
    /// `C`
    pub letter: char,
    /// `C:\`
    pub root: String,
    pub label: String,
    /// `NTFS`, `exFAT`, `ReFS`, ...
    pub fs: String,
    pub total: u64,
    pub free: u64,
    pub cluster: u64,
    pub removable: bool,
}

impl Volume {
    /// Bytes in use as the file system counts them (total minus free).
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.free)
    }

    pub fn is_ntfs(&self) -> bool {
        self.fs.eq_ignore_ascii_case("NTFS")
    }

    /// `C:`
    pub fn drive(&self) -> String {
        format!("{}:", self.letter)
    }

    /// Re-reads free space (after an action).
    pub fn refresh(&mut self) {
        if let Ok(v) = info(self.letter) {
            *self = v;
        }
    }
}

/// Fixed and removable drives with a mounted file system.
pub fn list() -> Vec<Volume> {
    let mask = unsafe { GetLogicalDrives() };
    (0..26u8)
        .filter(|i| mask & (1 << i) != 0)
        .filter_map(|i| info((b'A' + i) as char).ok())
        .filter(|v| v.total > 0)
        .collect()
}

pub fn info(letter: char) -> io::Result<Volume> {
    let letter = letter.to_ascii_uppercase();
    let root = format!("{letter}:\\");
    let w = wide(&root);
    let kind = unsafe { GetDriveTypeW(pcwstr(&w)) };
    // 2 removable, 3 fixed. Network (4), CD (5) and RAM disks (6) are out of scope.
    if kind != 2 && kind != 3 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "not a local disk",
        ));
    }
    let mut label = [0u16; 261];
    let mut fs = [0u16; 261];
    unsafe {
        GetVolumeInformationW(
            pcwstr(&w),
            Some(&mut label),
            None,
            None,
            None,
            Some(&mut fs),
        )
        .map_err(io_err)?;
    }
    let (mut avail, mut total, mut free) = (0u64, 0u64, 0u64);
    unsafe {
        GetDiskFreeSpaceExW(
            pcwstr(&w),
            Some(&mut avail),
            Some(&mut total),
            Some(&mut free),
        )
        .map_err(io_err)?;
    }
    let (mut spc, mut bps) = (0u32, 0u32);
    unsafe {
        GetDiskFreeSpaceW(pcwstr(&w), Some(&mut spc), Some(&mut bps), None, None)
            .map_err(io_err)?;
    }
    let s = |b: &[u16]| {
        String::from_utf16_lossy(&b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())])
    };
    Ok(Volume {
        letter,
        root,
        label: s(&label),
        fs: s(&fs),
        total,
        free,
        cluster: (spc as u64 * bps as u64).max(512),
        removable: kind == 2,
    })
}

/// Free bytes on the volume holding `path`.
pub fn free_for(path: &str) -> u64 {
    let letter = path.chars().next().unwrap_or('C');
    info(letter).map(|v| v.free).unwrap_or(0)
}
