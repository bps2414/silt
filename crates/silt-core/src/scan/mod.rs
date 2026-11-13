//! Scanning a drive into a [`Tree`].
//!
//! A scanner runs on its own thread and sends batches of [`RawEntry`] over a channel, so the app
//! can light the bar while bytes are found. Two scanners: [`mft`] reads the NTFS Master File
//! Table (admin, seconds) and [`walk`] lists folders in parallel (anything else, slower).

pub mod mft;
pub mod walk;

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::category::Classifier;
use crate::known::KnownDirs;
use crate::rules::Rule;
use crate::tree::{Classification, RawEntry, Tree, TreeBuilder};
use crate::volume::Volume;
use crate::{Category, win};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ScannerKind {
    Mft,
    Walk,
}

pub enum ScanMsg {
    Batch(Vec<RawEntry>),
    Done,
    Error(String),
}

/// Live counters the UI reads while a scan runs.
#[derive(Default)]
pub struct Progress {
    /// Allocated bytes found so far.
    pub bytes: AtomicU64,
    pub entries: AtomicU64,
}

pub struct ScanHandle {
    pub rx: Receiver<ScanMsg>,
    pub progress: Arc<Progress>,
    pub cancel: Arc<AtomicBool>,
    pub kind: ScannerKind,
    pub volume: Volume,
    root_id: u64,
    started: Instant,
}

/// Why the scan fell back to the slow walk, if it did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Fallback {
    NotAdmin,
    NotNtfs,
    MftFailed(String),
}

/// Starts scanning `volume`: the MFT when allowed and possible, otherwise the walk.
pub fn start(volume: &Volume, allow_mft: bool) -> (ScanHandle, Option<Fallback>) {
    let progress = Arc::new(Progress::default());
    let cancel = Arc::new(AtomicBool::new(false));
    let (tx, rx) = channel();
    let fallback = if !allow_mft || !win::is_elevated() {
        Some(Fallback::NotAdmin)
    } else if !volume.is_ntfs() {
        Some(Fallback::NotNtfs)
    } else {
        match mft::open(volume) {
            Ok(m) => {
                let (p, c) = (progress.clone(), cancel.clone());
                std::thread::Builder::new()
                    .name("silt-mft".into())
                    .spawn(move || mft::run(m, tx, p, c))
                    .ok();
                return (
                    ScanHandle {
                        rx,
                        progress,
                        cancel,
                        kind: ScannerKind::Mft,
                        volume: volume.clone(),
                        root_id: mft::ROOT,
                        started: Instant::now(),
                    },
                    None,
                );
            }
            Err(e) => Some(Fallback::MftFailed(e.to_string())),
        }
    };
    let (p, c, root, cluster) = (
        progress.clone(),
        cancel.clone(),
        volume.root.clone(),
        volume.cluster,
    );
    std::thread::Builder::new()
        .name("silt-walk".into())
        .spawn(move || walk::run(&root, cluster, tx, p, c))
        .ok();
    (
        ScanHandle {
            rx,
            progress,
            cancel,
            kind: ScannerKind::Walk,
            volume: volume.clone(),
            root_id: walk::ROOT,
            started: Instant::now(),
        },
        fallback,
    )
}

/// A finished scan.
pub struct ScanResult {
    pub tree: Tree,
    pub volume: Volume,
    pub kind: ScannerKind,
    pub elapsed: Duration,
    /// Used bytes the scan could not attribute to a file (shown under System).
    pub metadata: u64,
    /// Bytes the scan found beyond what the volume reports used (hard links in a walk).
    pub overcount: u64,
    pub classification: Classification,
}

impl ScanHandle {
    /// Drains the scanner and builds, reconciles and classifies the tree. Blocks; the app calls
    /// it on a worker thread while it reads [`ScanHandle::progress`].
    pub fn collect(self, known: &KnownDirs, rules: &[Rule]) -> io::Result<ScanResult> {
        let mut b = TreeBuilder::new(&self.volume.drive(), self.root_id);
        b.reserve(1 << 20);
        loop {
            match self.rx.recv() {
                Ok(ScanMsg::Batch(batch)) => {
                    for e in batch {
                        b.push(e);
                    }
                }
                Ok(ScanMsg::Done) => break,
                Ok(ScanMsg::Error(e)) => return Err(io::Error::other(e)),
                Err(_) => return Err(io::Error::other("scanner stopped")),
            }
        }
        if self.cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        let mut tree = b.finish();
        let mut volume = self.volume;
        volume.refresh();
        let found = tree.total();
        let used = volume.used();
        let (metadata, overcount) = (used.saturating_sub(found), found.saturating_sub(used));
        let classification = tree.classify(&Classifier::new(known, volume.letter, rules));
        if metadata > 0 {
            tree.add_synthetic(tree.root, "Filesystem metadata", metadata, Category::System);
        }
        Ok(ScanResult {
            tree,
            volume,
            kind: self.kind,
            elapsed: self.started.elapsed(),
            metadata,
            overcount,
            classification,
        })
    }
}

/// Sends `batch` and starts a new one; false when the receiver is gone.
pub(crate) fn flush(tx: &Sender<ScanMsg>, batch: &mut Vec<RawEntry>, progress: &Progress) -> bool {
    if batch.is_empty() {
        return true;
    }
    let bytes: u64 = batch.iter().map(|e| e.alloc).sum();
    progress.bytes.fetch_add(bytes, Ordering::Relaxed);
    progress
        .entries
        .fetch_add(batch.len() as u64, Ordering::Relaxed);
    tx.send(ScanMsg::Batch(std::mem::replace(
        batch,
        Vec::with_capacity(4096),
    )))
    .is_ok()
}
