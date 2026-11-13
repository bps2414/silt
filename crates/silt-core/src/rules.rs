//! Clean rules: caches that rebuild themselves, one TOML file per group in `rules/`, built into
//! the binary. See CONTRIBUTING.md for the format.
//!
//! Path forms: `env:VAR` (or `env:VAR\sub`) for a cache a variable relocates, `%VAR%\...` for
//! known folders, and `*` within one segment for browser profiles. Every form that resolves is
//! used: a relocated cache and an old one left in the default place are both found.
//!
//! The guards ([`guard`]) live in code and hold whatever a rule says.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::category::wild;
use crate::known::KnownDirs;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Safety {
    /// Rebuilt automatically, nothing of the user's is lost.
    Safe,
    /// Rebuilt, but at a cost the user should know (a sign-in, a long re-download, a slow first run).
    Caution,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    pub name: String,
    /// `system`, `browsers`, `gpu` or `developer`.
    pub group: String,
    pub paths: Vec<String>,
    pub safety: Safety,
    pub note: String,
    #[serde(
        rename = "note-pt-BR",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub note_pt_br: Option<String>,
}

#[derive(Deserialize)]
struct RuleFile {
    rule: Vec<Rule>,
}

/// The rule files shipped in the binary, in display order.
pub const FILES: [(&str, &str); 4] = [
    ("system", include_str!("../../../rules/system.toml")),
    ("browsers", include_str!("../../../rules/browsers.toml")),
    ("gpu", include_str!("../../../rules/gpu.toml")),
    ("developer", include_str!("../../../rules/developer.toml")),
];

/// Parses the built-in rules. A malformed file is a build-time mistake, so tests cover it.
pub fn builtin() -> Vec<Rule> {
    FILES
        .iter()
        .flat_map(|(name, text)| parse(text).unwrap_or_else(|e| panic!("rules/{name}.toml: {e}")))
        .collect()
}

pub fn parse(text: &str) -> Result<Vec<Rule>, String> {
    let f: RuleFile = toml::from_str(text).map_err(|e| e.to_string())?;
    for r in &f.rule {
        if r.paths.is_empty() {
            return Err(format!("rule {} has no paths", r.id));
        }
        if !matches!(
            r.group.as_str(),
            "system" | "browsers" | "gpu" | "developer"
        ) {
            return Err(format!("rule {} has unknown group {}", r.id, r.group));
        }
    }
    Ok(f.rule)
}

impl Rule {
    /// The rule's paths with variables expanded and globs kept (`C:\Users\me\...\*\Cache`).
    pub fn patterns(&self, known: &KnownDirs) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for p in &self.paths {
            let resolved = if let Some(rest) = p.strip_prefix("env:") {
                let (var, sub) = rest.split_once('\\').map_or((rest, ""), |(v, s)| (v, s));
                known.var(var).map(|v| {
                    if sub.is_empty() {
                        v.to_string()
                    } else {
                        format!("{}\\{sub}", v.trim_end_matches('\\'))
                    }
                })
            } else {
                known.expand(p)
            };
            let Some(r) = resolved else { continue };
            let r = r.replace('/', "\\").trim_end_matches('\\').to_string();
            if Path::new(&r).is_absolute() && !out.iter().any(|o| o.eq_ignore_ascii_case(&r)) {
                out.push(r);
            }
        }
        out
    }

    /// Paths on disk the rule points at now (globs expanded), none inside another.
    pub fn targets(&self, known: &KnownDirs) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = self
            .patterns(known)
            .iter()
            .flat_map(|p| expand_glob(p))
            .collect();
        out.sort_by_key(|p| p.to_string_lossy().to_lowercase());
        out.dedup_by(|a, b| {
            a.to_string_lossy()
                .eq_ignore_ascii_case(&b.to_string_lossy())
        });
        let all = out.clone();
        out.retain(|p| {
            let s = p.to_string_lossy();
            !all.iter()
                .any(|o| o != p && crate::win::path_within(&s, &o.to_string_lossy()))
        });
        out
    }

    pub fn note_for(&self, lang: &str) -> &str {
        if lang.starts_with("pt") {
            self.note_pt_br.as_deref().unwrap_or(&self.note)
        } else {
            &self.note
        }
    }
}

/// Existing paths matching `pattern` (`*` within a segment).
fn expand_glob(pattern: &str) -> Vec<PathBuf> {
    let segs: Vec<&str> = pattern.split('\\').collect();
    let mut cur = vec![PathBuf::from(format!("{}\\", segs[0]))];
    for seg in &segs[1..] {
        let mut next = Vec::new();
        for base in &cur {
            if seg.contains('*') {
                let lseg = seg.to_lowercase();
                if let Ok(rd) = fs::read_dir(base) {
                    for e in rd.flatten() {
                        if wild(&lseg, &e.file_name().to_string_lossy().to_lowercase()) {
                            next.push(e.path());
                        }
                    }
                }
            } else {
                let p = base.join(seg);
                if fs::symlink_metadata(&p).is_ok() {
                    next.push(p);
                }
            }
        }
        cur = next;
    }
    cur
}

/// Why a path may not be cleaned or trashed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum Refusal {
    NotAbsolute,
    DriveRoot,
    Protected(String),
    ContainsProtected(String),
    Link,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::NotAbsolute => write!(f, "not an absolute path"),
            Refusal::DriveRoot => write!(f, "a drive root"),
            Refusal::Protected(p) => write!(f, "a protected folder ({p})"),
            Refusal::ContainsProtected(p) => write!(f, "contains a protected folder ({p})"),
            Refusal::Link => write!(f, "a junction or symbolic link"),
        }
    }
}

/// Folders Silt never deletes, empties or sends to the Recycle Bin, whatever a rule says.
pub fn protected_folders(known: &KnownDirs) -> Vec<String> {
    let w = known.windows();
    let mut v = vec![
        known.users(),
        known.profile(),
        known.local(),
        known.roaming(),
        known.profile().join("Documents"),
        known.profile().join("Desktop"),
        known.profile().join("Downloads"),
        known.profile().join("Pictures"),
        known.profile().join("Videos"),
        known.profile().join("Music"),
        known.profile().join("OneDrive"),
        w.clone(),
        w.join("System32"),
        w.join("SysWOW64"),
        w.join("WinSxS"),
        known.program_files(),
        known.program_files_x86(),
        known.program_data(),
    ];
    // Every profile folder, not only this user's.
    if let Ok(rd) = fs::read_dir(known.users()) {
        v.extend(rd.flatten().map(|e| e.path()));
    }
    v.into_iter()
        .map(|p| p.to_string_lossy().trim_end_matches('\\').to_string())
        .collect()
}

/// Checks a path against the guards. `deleting_folder` is false when only its contents go.
pub fn guard(path: &Path, protected: &[String], deleting_folder: bool) -> Result<(), Refusal> {
    let s = path.to_string_lossy();
    let s = s.trim_end_matches('\\');
    if !path.is_absolute() {
        return Err(Refusal::NotAbsolute);
    }
    if s.len() <= 2 || s.split('\\').filter(|x| !x.is_empty()).count() < 2 {
        return Err(Refusal::DriveRoot);
    }
    for p in protected {
        if s.eq_ignore_ascii_case(p) {
            return Err(Refusal::Protected(p.clone()));
        }
        if crate::win::path_within(p, s) {
            return Err(Refusal::ContainsProtected(p.clone()));
        }
    }
    if let Ok(m) = fs::symlink_metadata(path)
        && m.file_type().is_symlink()
        && !deleting_folder
    {
        return Err(Refusal::Link);
    }
    Ok(())
}
