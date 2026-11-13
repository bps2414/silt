//! Where things live on this machine: the user's folders, relocated caches (through environment
//! variables) and game libraries.
//!
//! When the app elevates into another account, the original user's variables come in as a file
//! (`--user-env`), so rules resolve the right profile.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default)]
pub struct KnownDirs {
    vars: HashMap<String, String>,
}

impl KnownDirs {
    /// This process's environment.
    pub fn current() -> Self {
        Self::from_vars(std::env::vars())
    }

    pub fn from_vars(vars: impl IntoIterator<Item = (String, String)>) -> Self {
        KnownDirs {
            vars: vars
                .into_iter()
                .map(|(k, v)| (k.to_ascii_uppercase(), long_path(v)))
                .collect(),
        }
    }

    /// Reads a snapshot written by [`KnownDirs::write_snapshot`].
    pub fn from_snapshot(path: &Path) -> std::io::Result<Self> {
        let text = fs::read_to_string(path)?;
        let map: HashMap<String, String> = serde_json::from_str(&text)?;
        Ok(Self::from_vars(map))
    }

    pub fn write_snapshot(&self, path: &Path) -> std::io::Result<()> {
        fs::write(path, serde_json::to_string(&self.vars)?)
    }

    pub fn var(&self, name: &str) -> Option<&str> {
        self.vars
            .get(&name.to_ascii_uppercase())
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    }

    fn dir(&self, name: &str, fallback: impl FnOnce() -> PathBuf) -> PathBuf {
        self.var(name).map(PathBuf::from).unwrap_or_else(fallback)
    }

    pub fn profile(&self) -> PathBuf {
        self.dir("USERPROFILE", || PathBuf::from(r"C:\Users\Default"))
    }
    pub fn local(&self) -> PathBuf {
        self.dir("LOCALAPPDATA", || self.profile().join(r"AppData\Local"))
    }
    pub fn roaming(&self) -> PathBuf {
        self.dir("APPDATA", || self.profile().join(r"AppData\Roaming"))
    }
    pub fn temp(&self) -> PathBuf {
        self.dir("TEMP", || self.local().join("Temp"))
    }
    pub fn windows(&self) -> PathBuf {
        self.dir("SYSTEMROOT", || {
            self.dir("WINDIR", || PathBuf::from(r"C:\Windows"))
        })
    }
    pub fn program_files(&self) -> PathBuf {
        self.dir("PROGRAMW6432", || {
            self.dir("PROGRAMFILES", || PathBuf::from(r"C:\Program Files"))
        })
    }
    pub fn program_files_x86(&self) -> PathBuf {
        self.dir("PROGRAMFILES(X86)", || {
            PathBuf::from(r"C:\Program Files (x86)")
        })
    }
    pub fn program_data(&self) -> PathBuf {
        self.dir("PROGRAMDATA", || PathBuf::from(r"C:\ProgramData"))
    }
    /// `C:\Users`
    pub fn users(&self) -> PathBuf {
        self.profile()
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(r"C:\Users"))
    }

    /// Expands `%VAR%` references. `None` when a variable is unset.
    pub fn expand(&self, s: &str) -> Option<String> {
        let mut out = String::new();
        let mut rest = s;
        while let Some(start) = rest.find('%') {
            out.push_str(&rest[..start]);
            let after = &rest[start + 1..];
            let end = after.find('%')?;
            let name = &after[..end];
            let value = match name.to_ascii_uppercase().as_str() {
                "USERPROFILE" => self.profile().to_string_lossy().into_owned(),
                "LOCALAPPDATA" => self.local().to_string_lossy().into_owned(),
                "APPDATA" => self.roaming().to_string_lossy().into_owned(),
                "TEMP" | "TMP" => self.temp().to_string_lossy().into_owned(),
                "WINDIR" | "SYSTEMROOT" => self.windows().to_string_lossy().into_owned(),
                "PROGRAMFILES" => self.program_files().to_string_lossy().into_owned(),
                "PROGRAMFILES(X86)" => self.program_files_x86().to_string_lossy().into_owned(),
                "PROGRAMDATA" => self.program_data().to_string_lossy().into_owned(),
                "USERS" => self.users().to_string_lossy().into_owned(),
                _ => self.var(name)?.to_string(),
            };
            out.push_str(&value);
            rest = &after[end + 1..];
        }
        out.push_str(rest);
        Some(out)
    }

    /// Folders that hold installed games, from the launchers' own records where they keep one.
    pub fn game_libraries(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        // Steam: every library listed in libraryfolders.vdf.
        let steam = registry_string(r"Software\Valve\Steam", "SteamPath")
            .map(|p| PathBuf::from(p.replace('/', "\\")))
            .unwrap_or_else(|| self.program_files_x86().join("Steam"));
        out.push(steam.join(r"steamapps\common"));
        if let Ok(vdf) = fs::read_to_string(steam.join(r"steamapps\libraryfolders.vdf")) {
            for line in vdf.lines() {
                let parts: Vec<&str> = line.split('"').filter(|s| !s.trim().is_empty()).collect();
                if parts.len() == 2 && parts[0].eq_ignore_ascii_case("path") {
                    out.push(
                        PathBuf::from(parts[1].replace("\\\\", "\\")).join(r"steamapps\common"),
                    );
                }
            }
        }
        // Epic: one manifest per installed game.
        if let Ok(dir) = fs::read_dir(
            self.program_data()
                .join(r"Epic\EpicGamesLauncher\Data\Manifests"),
        ) {
            for e in dir.flatten() {
                if e.path().extension().is_some_and(|x| x == "item")
                    && let Ok(text) = fs::read_to_string(e.path())
                    && let Ok(v) = serde_json::from_str::<serde_json::Value>(&text)
                    && let Some(loc) = v["InstallLocation"].as_str()
                {
                    out.push(PathBuf::from(loc));
                }
            }
        }
        out.push(self.program_files().join("Epic Games"));
        // GOG, EA, Ubisoft: default folders.
        out.push(self.program_files_x86().join(r"GOG Galaxy\Games"));
        out.push(self.program_files().join("EA Games"));
        out.push(
            self.program_files_x86()
                .join(r"Ubisoft\Ubisoft Game Launcher\games"),
        );
        // Riot, GOG and Xbox put a folder at the root of each drive.
        for v in crate::volume::list() {
            for name in [
                "Riot Games",
                "GOG Games",
                "XboxGames",
                "SteamLibrary\\steamapps\\common",
            ] {
                out.push(PathBuf::from(format!("{}{}", v.root, name)));
            }
        }
        out.sort();
        out.dedup_by(|a, b| {
            a.to_string_lossy()
                .eq_ignore_ascii_case(&b.to_string_lossy())
        });
        out
    }
}

/// HKCU string value, if present.
fn registry_string(key: &str, value: &str) -> Option<String> {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW};
    let k = crate::win::wide(key);
    let v = crate::win::wide(value);
    let mut buf = vec![0u16; 1024];
    let mut len = (buf.len() * 2) as u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            crate::win::pcwstr(&k),
            crate::win::pcwstr(&v),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut len),
        )
    };
    if r.is_err() {
        return None;
    }
    let n = (len as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buf[..n]))
}

/// `C:\Users\LONGUS~1\AppData\Local\Temp` (an 8.3 short name, as `%TEMP%` often is) becomes the
/// long path the scan sees. Anything else, or a path that doesn't exist, stays as it is.
fn long_path(v: String) -> String {
    if !v.contains('~') || v.contains(';') {
        return v;
    }
    let w = crate::win::wide(&v);
    let mut buf = vec![0u16; 1024];
    let n = unsafe {
        windows::Win32::Storage::FileSystem::GetLongPathNameW(
            crate::win::pcwstr(&w),
            Some(&mut buf),
        )
    } as usize;
    if n == 0 || n > buf.len() {
        return v;
    }
    String::from_utf16_lossy(&buf[..n])
}
