//! Categories and the classifier that assigns them.
//!
//! A path rule beats an extension rule, and the first rule that matches wins. Path rules match
//! from the drive root down, so a deeper rule (`Windows\Temp`, a Steam library inside
//! `Program Files (x86)`) overrides the one above it.

use serde::{Deserialize, Serialize};

use crate::known::KnownDirs;
use crate::rules::Rule;

#[derive(
    Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord, Serialize, Deserialize, Default,
)]
#[serde(rename_all = "lowercase")]
#[repr(u8)]
pub enum Category {
    Apps,
    Games,
    Developer,
    Caches,
    Documents,
    Photos,
    Videos,
    Music,
    System,
    Trash,
    #[default]
    Other,
}

impl Category {
    pub const ALL: [Category; 11] = [
        Category::Apps,
        Category::Games,
        Category::Developer,
        Category::Caches,
        Category::Documents,
        Category::Photos,
        Category::Videos,
        Category::Music,
        Category::System,
        Category::Trash,
        Category::Other,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn id(self) -> &'static str {
        match self {
            Category::Apps => "apps",
            Category::Games => "games",
            Category::Developer => "developer",
            Category::Caches => "caches",
            Category::Documents => "documents",
            Category::Photos => "photos",
            Category::Videos => "videos",
            Category::Music => "music",
            Category::System => "system",
            Category::Trash => "trash",
            Category::Other => "other",
        }
    }

    pub fn from_id(id: &str) -> Option<Category> {
        Category::ALL
            .into_iter()
            .find(|c| c.id().eq_ignore_ascii_case(id))
    }

    /// Category of a file by its extension (lowercase, without the dot).
    pub fn from_ext(ext: &str) -> Option<Category> {
        Some(match ext {
            "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" | "heif" | "avif" | "bmp" | "tif"
            | "tiff" | "raw" | "cr2" | "cr3" | "nef" | "arw" | "dng" | "orf" | "rw2" | "psd"
            | "xcf" | "svg" | "jxl" => Category::Photos,
            "mp4" | "mkv" | "mov" | "avi" | "wmv" | "webm" | "m4v" | "flv" | "mts" | "m2ts"
            | "mpg" | "mpeg" | "3gp" | "vob" => Category::Videos,
            "mp3" | "flac" | "wav" | "aac" | "m4a" | "ogg" | "opus" | "wma" | "aiff" | "aif"
            | "alac" | "mid" | "midi" => Category::Music,
            "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "odt" | "ods" | "odp"
            | "txt" | "md" | "rtf" | "epub" | "csv" | "pages" | "numbers" | "key" | "tex" => {
                Category::Documents
            }
            _ => return None,
        })
    }

    /// System items are shown but never get an action button.
    pub fn is_actionable(self) -> bool {
        !matches!(self, Category::System)
    }
}

/// What Shrink offers for a matched folder.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shrink {
    No,
    /// The folder itself is one candidate (`node_modules`, a toolchain).
    Itself,
    /// Each folder inside is a candidate (one per app or game).
    Children,
}

#[derive(Clone, Debug)]
pub(crate) struct Pattern {
    pub segs: Vec<String>,
    pub category: Option<Category>,
    pub rule: Option<u16>,
    pub shrink: Shrink,
}

/// Folder names that mark developer build output anywhere on disk.
pub(crate) const DEV_NAMES: [&str; 4] = ["node_modules", ".venv", ".gradle", "target"];

pub struct Classifier {
    pub(crate) patterns: Vec<Pattern>,
}

impl Classifier {
    /// Rules for the drive `letter`. `rules` are the clean rules (their folders become Caches or
    /// Developer and report as reclaimable).
    pub fn new(known: &KnownDirs, letter: char, rules: &[Rule]) -> Self {
        let mut c = Classifier {
            patterns: Vec::new(),
        };
        let drive = format!("{}:", letter.to_ascii_lowercase());
        let root = format!("{}:\\", letter.to_ascii_uppercase());
        let mut add =
            |path: &str, category: Option<Category>, rule: Option<u16>, shrink: Shrink| {
                let segs: Vec<String> = path
                    .trim_end_matches('\\')
                    .split('\\')
                    .map(str::to_lowercase)
                    .collect();
                if segs.first().map(String::as_str) == Some(drive.as_str()) && segs.len() > 1 {
                    c.patterns.push(Pattern {
                        segs,
                        category,
                        rule,
                        shrink,
                    });
                }
            };
        use Category::*;
        add(
            &format!("{root}$Recycle.Bin"),
            Some(Trash),
            None,
            Shrink::No,
        );
        for name in [
            "pagefile.sys",
            "hiberfil.sys",
            "swapfile.sys",
            "System Volume Information",
            "Recovery",
            "Windows.old",
            "$*",
        ] {
            add(&format!("{root}{name}"), Some(System), None, Shrink::No);
        }
        // Clean rules: the reclaimable folders.
        for (i, rule) in rules.iter().enumerate() {
            let cat = if rule.group == "developer" {
                Developer
            } else {
                Caches
            };
            for p in rule.patterns(known) {
                add(&p, Some(cat), Some(i as u16), Shrink::No);
            }
        }
        // Developer toolchains and package stores (relocated ones come from their variables).
        for var in [
            "CARGO_HOME",
            "RUSTUP_HOME",
            "UV_PYTHON_INSTALL_DIR",
            "UV_TOOL_DIR",
            "UV_CACHE_DIR",
            "ANDROID_HOME",
            "ANDROID_SDK_ROOT",
            "GOPATH",
            "GOMODCACHE",
            "GOCACHE",
            "GRADLE_USER_HOME",
            "NUGET_PACKAGES",
            "FNM_DIR",
            "PNPM_HOME",
            "NPM_CONFIG_PREFIX",
            "NPM_CONFIG_CACHE",
            "PLAYWRIGHT_BROWSERS_PATH",
            "HF_HOME",
            "JAVA_HOME",
            "DENO_DIR",
            "BUN_INSTALL",
            "COREPACK_HOME",
            "PIP_CACHE_DIR",
            "YARN_CACHE_FOLDER",
        ] {
            if let Some(v) = known.var(var) {
                add(v, Some(Developer), None, Shrink::Itself);
            }
        }
        for p in [
            r"%USERPROFILE%\.cargo",
            r"%USERPROFILE%\.rustup",
            r"%USERPROFILE%\.gradle",
            r"%USERPROFILE%\.nuget",
            r"%USERPROFILE%\.m2",
            r"%USERPROFILE%\.android",
            r"%USERPROFILE%\go",
            r"%USERPROFILE%\.cache",
            r"%LOCALAPPDATA%\Android\Sdk",
            r"%LOCALAPPDATA%\ms-playwright",
            r"%LOCALAPPDATA%\uv",
            r"%APPDATA%\npm",
            r"%LOCALAPPDATA%\pnpm",
            r"%LOCALAPPDATA%\fnm_multishells",
            r"%APPDATA%\fnm",
        ] {
            if let Some(p) = known.expand(p) {
                add(&p, Some(Developer), None, Shrink::Itself);
            }
        }
        for lib in known.game_libraries() {
            add(&lib.to_string_lossy(), Some(Games), None, Shrink::Children);
        }
        add(
            &format!(r"{}\WindowsApps", known.program_files().to_string_lossy()),
            Some(Apps),
            None,
            Shrink::No,
        );
        for p in [
            known.program_files(),
            known.program_files_x86(),
            known.local().join("Programs"),
        ] {
            add(&p.to_string_lossy(), Some(Apps), None, Shrink::Children);
        }
        add(
            &format!("{}\\Package Cache", known.program_data().to_string_lossy()),
            Some(Apps),
            None,
            Shrink::No,
        );
        add(
            &known.windows().to_string_lossy(),
            Some(System),
            None,
            Shrink::No,
        );
        c
    }

    /// Patterns that start at the drive root (`c:`).
    pub(crate) fn root_states(&self) -> Vec<(u32, u16)> {
        (0..self.patterns.len() as u32).map(|p| (p, 1)).collect()
    }
}

/// `*` wildcard match within one path segment (both sides lowercase).
pub(crate) fn wild(p: &str, s: &str) -> bool {
    if !p.contains('*') {
        return p == s;
    }
    let (p, s) = (p.as_bytes(), s.as_bytes());
    let (mut pi, mut si, mut star, mut mark) = (0, 0, usize::MAX, 0);
    while si < s.len() {
        if pi < p.len() && p[pi] != b'*' && p[pi] == s[si] {
            pi += 1;
            si += 1;
        } else if pi < p.len() && p[pi] == b'*' {
            star = pi;
            mark = si;
            pi += 1;
        } else if star != usize::MAX {
            pi = star + 1;
            mark += 1;
            si = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == b'*' {
        pi += 1;
    }
    pi == p.len()
}
