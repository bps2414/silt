//! Silt core: scanning, the in-memory tree, categories, clean rules, shrink (WOF), clean, the
//! Recycle Bin and the journal. No UI: `silt-cli` and the `silt` app both sit on top of it.

pub mod category;
pub mod clean;
pub mod journal;
pub mod known;
pub mod largest;
pub mod report;
pub mod rules;
pub mod scan;
pub mod shrink;
pub mod trash;
pub mod tree;
pub mod volume;
pub mod win;

pub use category::Category;
pub use tree::{NodeId, Tree};

/// Formats a byte count the way the UI and the CLI show it: 1 decimal under 10, none above.
pub fn fmt_bytes(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1000.0 && u + 1 < UNITS.len() {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{bytes} B")
    } else if v < 10.0 {
        format!("{v:.1} {}", UNITS[u])
    } else {
        format!("{v:.0} {}", UNITS[u])
    }
}

/// Seconds since the Unix epoch, now.
pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
