//! What a scan adds up to: bytes per category, what Clean and Shrink would recover, large and
//! old files, the Recycle Bin. `silt-cli report` prints it; the Overview draws it.

use std::collections::HashSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use serde::Serialize;

use crate::Category;
use crate::journal::{Action, Entry};
use crate::largest::{self, Filter};
use crate::rules::{Rule, Safety};
use crate::scan::{ScanResult, ScannerKind};
use crate::shrink::{
    Algorithm, Background, Estimate, Excludes, FolderFiles, Sampler, SharedStats, files_in_tree,
    sums_in_tree,
};
use crate::trash::BinInfo;
use crate::tree::{NONE, NodeId, Tree};

#[derive(Clone, Debug, Default, Serialize)]
pub struct CategoryRow {
    pub category: Category,
    pub bytes: u64,
    /// What Clean would free inside it.
    pub clean: u64,
    /// What Shrink would save inside it (middle of the estimate; 0 before the estimate runs).
    pub shrink: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct CleanRow {
    pub rule: String,
    pub name: String,
    pub group: String,
    pub safety: Safety,
    pub bytes: u64,
    pub folders: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ShrinkRow {
    #[serde(skip)]
    pub node: NodeId,
    pub path: String,
    pub category: Category,
    pub alloc: u64,
    /// Allocated bytes of the files Shrink would compress.
    pub eligible: u64,
    pub files: u64,
    /// Already compressed.
    pub already: u64,
    pub directstorage: bool,
    pub estimate: Option<Estimate>,
    /// Silt shrank it before and it has uncompressed files again (an update rewrote them).
    pub shrunk_before: bool,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct LargeOld {
    pub bytes: u64,
    pub files: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub drive: String,
    pub label: String,
    pub fs: String,
    pub capacity: u64,
    pub used: u64,
    pub free: u64,
    pub scanner: ScannerKind,
    pub elapsed_ms: u64,
    pub files: u64,
    pub folders: u64,
    /// Allocated bytes the scan attributed to files and folders.
    pub found: u64,
    /// Used bytes no file accounts for (shown under System).
    pub metadata: u64,
    pub overcount: u64,
    pub categories: Vec<CategoryRow>,
    pub clean: Vec<CleanRow>,
    pub shrink: Vec<ShrinkRow>,
    pub large_old: LargeOld,
    /// This drive's Recycle Bin.
    pub bin: BinInfo,
}

impl Report {
    /// Everything but the Shrink estimate (which reads files; see [`Report::estimate`]).
    pub fn build(
        scan: &ScanResult,
        rules: &[Rule],
        journal: &[Entry],
        bin: BinInfo,
        now: u64,
    ) -> Report {
        let tree = &scan.tree;
        let totals = tree.category_totals();
        let mut categories: Vec<CategoryRow> = Category::ALL
            .iter()
            .map(|&c| CategoryRow {
                category: c,
                bytes: totals[c.index()],
                ..Default::default()
            })
            .collect();

        // Clean: one row per rule, nested hits counted once.
        let hit_nodes: HashSet<NodeId> = scan
            .classification
            .rule_hits
            .iter()
            .map(|&(_, n)| n)
            .collect();
        let mut clean: Vec<CleanRow> = rules
            .iter()
            .map(|r| CleanRow {
                rule: r.id.clone(),
                name: r.name.clone(),
                group: r.group.clone(),
                safety: r.safety,
                bytes: 0,
                folders: Vec::new(),
            })
            .collect();
        for &(rule, node) in &scan.classification.rule_hits {
            if has_ancestor_in(tree, node, &hit_nodes) {
                continue;
            }
            let n = tree.node(node);
            let row = &mut clean[rule as usize];
            row.bytes += n.alloc;
            row.folders.push(tree.path(node));
            categories[n.category.index()].clean += n.alloc;
        }
        clean.retain(|r| r.bytes > 0);
        clean.sort_by_key(|r| std::cmp::Reverse(r.bytes));

        // Shrink: one row per candidate folder.
        let ex = Excludes::builtin();
        let mut shrunk: Vec<(String, bool)> = Vec::new();
        for e in journal {
            match e.action {
                Action::Shrink => shrunk.push((e.target.clone(), true)),
                Action::Restore => shrunk.push((e.target.clone(), false)),
                _ => {}
            }
        }
        let mut shrink: Vec<ShrinkRow> = scan
            .classification
            .shrink_candidates
            .iter()
            .filter(|&&c| !ex.folder_excluded(tree.name(c)))
            .filter_map(|&c| {
                let s = sums_in_tree(tree, c, &ex);
                if s.eligible == 0 && s.already == 0 {
                    return None;
                }
                let path = tree.path(c);
                let shrunk_before = s.eligible > 0
                    && shrunk
                        .iter()
                        .rev()
                        .find(|(t, _)| t.eq_ignore_ascii_case(&path))
                        .is_some_and(|&(_, shrink)| shrink);
                let n = tree.node(c);
                Some(ShrinkRow {
                    node: c,
                    path,
                    category: n.category,
                    alloc: n.alloc,
                    eligible: s.eligible,
                    files: s.files,
                    already: s.already,
                    directstorage: s.directstorage,
                    estimate: None,
                    shrunk_before,
                })
            })
            .collect();
        shrink.sort_by_key(|r| std::cmp::Reverse(r.eligible));

        let (lo_bytes, lo_files) = largest::total(tree, &Filter::large_and_old(), now);
        let root = tree.node(tree.root);
        let v = &scan.volume;
        Report {
            drive: v.drive(),
            label: v.label.clone(),
            fs: v.fs.clone(),
            capacity: v.total,
            used: v.used(),
            free: v.free,
            scanner: scan.kind,
            elapsed_ms: scan.elapsed.as_millis() as u64,
            files: u64::from(root.files),
            folders: tree.nodes.iter().filter(|n| n.is_dir()).count() as u64,
            found: tree.total() - scan.metadata,
            metadata: scan.metadata,
            overcount: scan.overcount,
            categories,
            clean,
            shrink,
            large_old: LargeOld {
                bytes: lo_bytes,
                files: lo_files,
            },
            bin,
        }
    }

    /// Samples every Shrink candidate (low I/O priority) and fills the estimates. `done` counts
    /// finished folders for a progress line.
    pub fn estimate(
        &mut self,
        tree: &Tree,
        cluster: u64,
        algo: Algorithm,
        cancel: &AtomicBool,
        done: &AtomicUsize,
    ) {
        let jobs = self.estimate_jobs(tree);
        let results = Mutex::new(Vec::new());
        run_estimates(&jobs, cluster, algo, cancel, |i, e| {
            results
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push((i, e));
            done.fetch_add(1, Ordering::Relaxed);
        });
        for (i, e) in results.into_inner().unwrap_or_else(|e| e.into_inner()) {
            self.set_estimate(i, e);
        }
    }

    /// The files of every Shrink row, read from the tree so the estimate can run without it.
    pub fn estimate_jobs(&self, tree: &Tree) -> Vec<EstimateJob> {
        let ex = Excludes::builtin();
        self.shrink
            .iter()
            .enumerate()
            .filter(|(_, r)| r.estimate.is_none())
            .map(|(index, r)| EstimateJob {
                index,
                path: r.path.clone(),
                eligible: r.eligible,
                files: files_in_tree(tree, r.node, &ex),
            })
            .collect()
    }

    /// Fills one row's estimate and the category totals it feeds.
    pub fn set_estimate(&mut self, i: usize, e: Estimate) {
        let Some(row) = self.shrink.get_mut(i) else {
            return;
        };
        row.estimate = Some(e);
        for c in &mut self.categories {
            c.shrink = 0;
        }
        for r in &self.shrink {
            if let Some(e) = &r.estimate {
                self.categories[r.category.index()].shrink += e.saving_mid();
            }
        }
    }

    pub fn clean_total(&self) -> u64 {
        self.clean.iter().map(|r| r.bytes).sum()
    }

    /// Shrink saving range (low, high) over the estimated folders.
    pub fn shrink_range(&self) -> (u64, u64) {
        self.shrink
            .iter()
            .filter_map(|r| r.estimate.as_ref())
            .fold((0, 0), |(l, h), e| {
                let (a, b) = e.saving();
                (l + a, h + b)
            })
    }

    pub fn shrink_total(&self) -> u64 {
        self.shrink
            .iter()
            .filter_map(|r| r.estimate.as_ref())
            .map(Estimate::saving_mid)
            .sum()
    }

    /// "Recoverable": Clean + Shrink + the Recycle Bin.
    pub fn recoverable(&self) -> u64 {
        self.clean_total() + self.shrink_total() + self.bin.bytes
    }

    /// "Without deleting anything": Shrink alone.
    pub fn without_deleting(&self) -> u64 {
        self.shrink_total()
    }

    /// What folders Silt shrank before could save again ("X GB compressible again").
    pub fn compressible_again(&self) -> u64 {
        self.shrink
            .iter()
            .filter(|r| r.shrunk_before)
            .filter_map(|r| r.estimate.as_ref())
            .map(Estimate::saving_mid)
            .sum()
    }
}

fn has_ancestor_in(tree: &Tree, node: NodeId, set: &HashSet<NodeId>) -> bool {
    let mut cur = tree.node(node).parent;
    while cur != NONE {
        if set.contains(&cur) {
            return true;
        }
        cur = tree.node(cur).parent;
    }
    false
}

/// One Shrink row to estimate: its index in [`Report::shrink`] and the files the scan found.
pub struct EstimateJob {
    pub index: usize,
    pub path: String,
    pub eligible: u64,
    pub files: FolderFiles,
}

/// Estimates `jobs` biggest first, on every core but one, at low I/O priority. Extensions are
/// learned once across folders ([`Sampler::shared`]), so a `.dll` read in one program isn't read
/// again in the next. `on_done(index, estimate)` runs as each folder finishes.
pub fn run_estimates(
    jobs: &[EstimateJob],
    cluster: u64,
    algo: Algorithm,
    cancel: &AtomicBool,
    on_done: impl Fn(usize, Estimate) + Sync,
) {
    let mut order: Vec<&EstimateJob> = jobs.iter().collect();
    order.sort_by_key(|j| std::cmp::Reverse(j.eligible));
    let next = AtomicUsize::new(0);
    let stats = SharedStats::default();
    let threads = (std::thread::available_parallelism().map_or(4, |n| n.get()) - 1)
        .clamp(1, order.len().max(1));
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                let _bg = Background::enter();
                let mut sampler = Sampler::shared(algo, stats.clone());
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= order.len() || cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let j = order[i];
                    // Read at most 1/64 of the folder, between 2 and 16 MB.
                    let cap = (j.eligible / 64).clamp(2 << 20, 16 << 20);
                    let e = sampler.estimate(&j.path, &j.files, cluster, cap, cancel);
                    if !cancel.load(Ordering::Relaxed) {
                        on_done(j.index, e);
                    }
                }
            });
        }
    });
}
