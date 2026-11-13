//! Small helpers over the Win32 calls the core needs.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows::Win32::Foundation::{FILETIME, HANDLE};
use windows::core::PCWSTR;

/// A NUL-terminated UTF-16 copy of `s`.
pub fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(Some(0)).collect()
}

/// A NUL-terminated `\\?\` path, so paths longer than MAX_PATH work.
pub fn verbatim(path: &Path) -> Vec<u16> {
    let s = path.as_os_str().to_string_lossy();
    if s.starts_with(r"\\?\") {
        wide(&*s)
    } else if let Some(unc) = s.strip_prefix(r"\\") {
        wide(format!(r"\\?\UNC\{unc}"))
    } else if s.len() >= 2 && s.as_bytes()[1] == b':' {
        wide(format!(r"\\?\{s}"))
    } else {
        wide(&*s)
    }
}

pub fn pcwstr(w: &[u16]) -> PCWSTR {
    PCWSTR(w.as_ptr())
}

/// Owned handle that closes on drop.
pub struct Handle(pub HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(self.0);
            }
        }
    }
}

/// FILETIME (100 ns since 1601) to Unix seconds, clamped to u32.
pub fn filetime_unix(ft: FILETIME) -> u32 {
    ticks_unix(((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64)
}

/// NT ticks (100 ns since 1601) to Unix seconds, clamped to u32.
pub fn ticks_unix(ticks: u64) -> u32 {
    const EPOCH: u64 = 116_444_736_000_000_000;
    (ticks.saturating_sub(EPOCH) / 10_000_000).min(u32::MAX as u64) as u32
}

/// True when this process runs elevated (needed to read the MFT).
pub fn is_elevated() -> bool {
    unsafe { windows::Win32::UI::Shell::IsUserAnAdmin().as_bool() }
}

/// The win32 error of an `io::Error`, if it has one.
pub fn os_code(e: &std::io::Error) -> u32 {
    e.raw_os_error().unwrap_or(0) as u32
}

pub fn io_err(e: windows::core::Error) -> std::io::Error {
    std::io::Error::from_raw_os_error(e.code().0 & 0xFFFF)
}

/// Rounds `n` up to a whole number of `cluster` bytes.
pub fn round_up(n: u64, cluster: u64) -> u64 {
    if cluster == 0 {
        n
    } else {
        n.div_ceil(cluster) * cluster
    }
}

/// The allocated size of a file as Windows reports it (compressed, sparse and WOF aware).
pub fn compressed_size(path: &Path) -> Option<u64> {
    let w = verbatim(path);
    let mut high = 0u32;
    let low = unsafe {
        windows::Win32::Foundation::SetLastError(windows::Win32::Foundation::WIN32_ERROR(0));
        windows::Win32::Storage::FileSystem::GetCompressedFileSizeW(pcwstr(&w), Some(&mut high))
    };
    if low == u32::MAX && std::io::Error::last_os_error().raw_os_error().unwrap_or(0) != 0 {
        return None;
    }
    Some(((high as u64) << 32) | low as u64)
}

/// Case-insensitive "is `child` equal to or inside `parent`" for Windows paths.
pub fn path_within(child: &str, parent: &str) -> bool {
    let c = child.trim_end_matches('\\').to_lowercase();
    let p = parent.trim_end_matches('\\').to_lowercase();
    c == p || (c.starts_with(&p) && c.as_bytes().get(p.len()) == Some(&b'\\'))
}

/// Reparse tags that point somewhere else (junctions, symlinks): never descend into these.
pub fn is_name_surrogate(tag: u32) -> bool {
    tag & 0x2000_0000 != 0
}

pub const TAG_WOF: u32 = 0x8000_0017;

/// OneDrive and other cloud-files placeholders (`IO_REPARSE_TAG_CLOUD_*`).
pub fn is_cloud_tag(tag: u32) -> bool {
    tag & 0xFFFF_0FFF == 0x9000_001A
}
