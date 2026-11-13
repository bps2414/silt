//! The journal: `%LOCALAPPDATA%\Silt\journal.jsonl`, one line per Shrink, Restore, Clean, Trash
//! and Empty Recycle Bin. It feeds the History screen. The real compression state always comes
//! from the scan, never from here.

use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::known::KnownDirs;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    Shrink,
    Restore,
    Clean,
    Trash,
    EmptyBin,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    /// Unix seconds.
    pub ts: u64,
    pub action: Action,
    /// A folder, a rule id, or a count of trashed items.
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub algorithm: Option<String>,
    /// Allocated bytes before and after (for Trash: what went to the bin).
    pub before: u64,
    pub after: u64,
    pub files: u64,
    pub skipped: u64,
}

impl Entry {
    pub fn saved(&self) -> i64 {
        self.before as i64 - self.after as i64
    }
}

pub struct Journal {
    pub path: PathBuf,
}

impl Journal {
    pub fn for_user(known: &KnownDirs) -> Self {
        let path = std::env::var_os("SILT_JOURNAL")
            .map(PathBuf::from)
            .unwrap_or_else(|| known.local().join("Silt").join("journal.jsonl"));
        Journal { path }
    }

    pub fn append(&self, e: &Entry) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(f, "{}", serde_json::to_string(e)?)
    }

    /// Every entry, oldest first. Lines that don't parse are skipped.
    pub fn read(&self) -> Vec<Entry> {
        let Ok(f) = fs::File::open(&self.path) else {
            return Vec::new();
        };
        BufReader::new(f)
            .lines()
            .map_while(Result::ok)
            .filter_map(|l| serde_json::from_str(&l).ok())
            .collect()
    }
}
