//! The window's state: what was scanned, what each screen has selected, and what is running.

use std::cell::OnceCell;
use std::collections::BTreeSet;

use silt_core::journal::{Action, Entry};
use silt_core::largest::{self, Filter};
use silt_core::report::Report;
use silt_core::rules::{Rule, Safety};
use silt_core::scan::{Fallback, ScanResult};
use silt_core::shrink::Algorithm;
use silt_core::volume::Volume;
use silt_core::{Category, NodeId};

use crate::bar::{Bar, Seg};
use crate::i18n::Lang;
use crate::style::BAR_ORDER;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Overview,
    Clean,
    Shrink,
    Large,
    Category(Category),
    Explore,
    History,
    Settings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub lang: Lang,
    pub theme: Theme,
    pub ask_admin: bool,
    pub algo: Algorithm,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            lang: Lang::System,
            theme: Theme::System,
            ask_admin: true,
            algo: Algorithm::Xpress8k,
        }
    }
}

pub enum Phase {
    Scanning { found: u64, entries: u64, used: u64 },
    Ready,
    Failed { drive: String, message: String },
}

/// One of the four ways back to space on the Overview.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reclaim {
    Shrink,
    Clean,
    Large,
    Bin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobKind {
    Clean,
    Shrink,
    Restore,
    Trash,
    EmptyBin,
}

/// An action in progress.
pub struct Job {
    pub kind: JobKind,
    pub done: u64,
    pub total: u64,
    /// What it works on now (a folder or a cache's name).
    pub current: String,
}

/// What the last action did, shown until the next one.
pub enum Outcome {
    Done {
        kind: JobKind,
        count: u64,
        bytes: u64,
        skipped: u64,
    },
    Stopped {
        kind: JobKind,
        done: u64,
        total: u64,
        bytes: u64,
    },
    Failed {
        kind: JobKind,
        message: String,
    },
}

/// A confirmation that is open (the destructive actions ask once).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confirm {
    Clean,
    Trash,
    EmptyBin,
}

/// A finished scan with everything the screens read from it.
pub struct Loaded {
    pub scan: ScanResult,
    pub report: Report,
    pub rules: Vec<Rule>,
    /// Per node, the category holding most of its bytes.
    pub dominant: Vec<Category>,
    /// Bytes of large and old files, per category.
    pub large_old: [u64; 11],
    pub journal: Vec<Entry>,
    pub now: u64,
    /// Each category's own list, built the first time it is shown.
    cat_items: [OnceCell<Vec<NodeId>>; 11],
}

impl Loaded {
    pub fn new(
        scan: ScanResult,
        report: Report,
        rules: Vec<Rule>,
        journal: Vec<Entry>,
        now: u64,
    ) -> Self {
        let tree = &scan.tree;
        let dominant = largest::dominant_categories(tree);
        let mut large_old = [0u64; 11];
        for id in largest::files(tree, &Filter::large_and_old(), now) {
            let n = tree.node(id);
            large_old[n.category.index()] += n.alloc;
        }
        Loaded {
            scan,
            report,
            rules,
            dominant,
            large_old,
            journal,
            now,
            cat_items: Default::default(),
        }
    }

    /// What a category's screen lists (computed once per scan).
    pub fn category_items(&self, c: Category) -> &[NodeId] {
        self.cat_items[c.index()].get_or_init(|| {
            crate::screens::large::category_items(&self.scan.tree, &self.dominant, c)
        })
    }

    pub fn rule(&self, id: &str) -> Option<&Rule> {
        self.rules.iter().find(|r| r.id == id)
    }

    /// The bar's segments in their fixed order. `preview` turns that action's part dark.
    pub fn segs(&self, preview: Option<Reclaim>) -> Vec<Seg> {
        let r = &self.report;
        BAR_ORDER
            .iter()
            .map(|&c| {
                let row = &r.categories[c.index()];
                let bytes = row.bytes;
                let (clean, shrink) = (row.clean.min(bytes), row.shrink.min(bytes));
                let (large, bin) = match c {
                    Category::Trash => (0, bytes),
                    _ => (self.large_old[c.index()].min(bytes), 0),
                };
                let reclaim = (clean + shrink + bin).min(bytes);
                let fading = match preview {
                    Some(Reclaim::Clean) => clean,
                    Some(Reclaim::Shrink) => shrink,
                    Some(Reclaim::Bin) => bin,
                    Some(Reclaim::Large) => large,
                    None => 0,
                }
                .min(bytes);
                // Large files are kept bytes (nothing marks them reclaimable until chosen).
                let soft = reclaim.saturating_sub(if preview == Some(Reclaim::Large) {
                    0
                } else {
                    fading
                });
                Seg {
                    cat: Some(c),
                    solid: bytes.saturating_sub(soft + fading),
                    soft,
                    fading,
                }
            })
            .collect()
    }

    /// Segments with `chosen` bytes per category drawn sparse: what an action would take.
    pub fn segs_chosen(&self, chosen: &[u64; 11]) -> Vec<Seg> {
        BAR_ORDER
            .iter()
            .map(|&c| {
                let bytes = self.report.categories[c.index()].bytes;
                let soft = chosen[c.index()].min(bytes);
                Seg {
                    cat: Some(c),
                    solid: bytes - soft,
                    soft,
                    fading: 0,
                }
            })
            .collect()
    }

    /// Shrink entries in the journal not undone by a later Restore.
    pub fn restorable(&self, i: usize) -> bool {
        let e = &self.journal[i];
        e.action == Action::Shrink
            && !self.journal[i + 1..]
                .iter()
                .any(|l| l.action == Action::Restore && l.target.eq_ignore_ascii_case(&e.target))
    }
}

pub const SIZES: [u64; 4] = [100 << 20, 500 << 20, 1 << 30, 5 << 30];
pub const AGES: [u32; 4] = [0, 180, 365, 730];

pub struct LargeView {
    pub folders: bool,
    pub cats: Vec<Category>,
    pub min: usize,
    pub age: usize,
    pub ids: Vec<NodeId>,
    pub sel: BTreeSet<NodeId>,
}

pub struct ExploreView {
    pub folder: NodeId,
    pub map: bool,
}

pub struct App {
    pub screen: Screen,
    pub phase: Phase,
    pub data: Option<Box<Loaded>>,
    pub drives: Vec<Volume>,
    pub drive: usize,
    pub drive_menu: bool,
    pub fallback: Option<Fallback>,
    pub fallback_hidden: bool,
    pub settings: Settings,
    pub clean_sel: BTreeSet<String>,
    pub shrink_sel: BTreeSet<String>,
    pub large: LargeView,
    pub explore: ExploreView,
    pub job: Option<Job>,
    pub outcome: Option<Outcome>,
    pub confirm: Option<Confirm>,
    /// Overview hover: the action whose cells go dark, and how far (0..1).
    pub preview: Option<Reclaim>,
    /// The action still drawn while its preview fades back.
    pub preview_last: Option<Reclaim>,
    pub preview_t: f32,
    /// 0..1 of the cells in their category colour after a scan.
    pub color_t: f32,
    pub clock: f64,
    pub bar: Bar,
    pub strip: Bar,
    pub map: Bar,
    pub bar_hover: Option<Category>,
    /// The palette of the last view, for rows built outside it (virtual lists).
    pub pal: std::cell::Cell<crate::style::Pal>,
    /// The real drives (None on the fixed drive of the scenes).
    pub live: Option<Box<crate::live::Live>>,
    /// Close the window on the next frame (an elevated copy took over).
    pub quit: bool,
    /// Folders the last Shrink compressed, for its Restore button.
    pub last_shrunk: Vec<String>,
    /// The user picked Shrink rows themselves: estimates stop adding to the selection.
    pub shrink_touched: bool,
    /// The fixed drive replayed as a scan (the README's recording): loads once the bar is full.
    pub replay: Option<Box<Loaded>>,
}

impl App {
    pub fn new(drives: Vec<Volume>) -> Self {
        App {
            screen: Screen::Overview,
            phase: Phase::Scanning {
                found: 0,
                entries: 0,
                used: drives.first().map_or(0, Volume::used),
            },
            data: None,
            drives,
            drive: 0,
            drive_menu: false,
            fallback: None,
            fallback_hidden: false,
            settings: Settings::default(),
            clean_sel: BTreeSet::new(),
            shrink_sel: BTreeSet::new(),
            large: LargeView {
                folders: false,
                cats: Vec::new(),
                min: 0,
                age: 0,
                ids: Vec::new(),
                sel: BTreeSet::new(),
            },
            explore: ExploreView {
                folder: 0,
                map: false,
            },
            job: None,
            outcome: None,
            confirm: None,
            preview: None,
            preview_last: None,
            preview_t: 0.0,
            color_t: 1.0,
            clock: 0.0,
            bar: Bar::default(),
            strip: Bar::default(),
            map: Bar::default(),
            bar_hover: None,
            pal: std::cell::Cell::new(crate::style::pal(false, 1.0)),
            live: None,
            quit: false,
            last_shrunk: Vec::new(),
            shrink_touched: false,
            replay: None,
        }
    }

    pub fn volume(&self) -> Option<&Volume> {
        self.drives.get(self.drive)
    }

    /// Takes a finished scan: default selections, lists, the Explore root.
    pub fn load(&mut self, data: Loaded) {
        self.clean_sel = data
            .report
            .clean
            .iter()
            .filter(|r| r.safety == Safety::Safe)
            .map(|r| r.rule.clone())
            .collect();
        self.shrink_sel = data
            .report
            .shrink
            .iter()
            .filter(|r| {
                r.estimate
                    .as_ref()
                    .is_some_and(|e| e.saving_mid() >= 256 << 20)
            })
            .take(5)
            .map(|r| r.path.clone())
            .collect();
        self.explore.folder = data.scan.tree.root;
        self.phase = Phase::Ready;
        self.data = Some(Box::new(data));
        self.refresh_large();
    }

    /// Takes a scan made after an action, keeping the screen where it was.
    pub fn reload(&mut self, data: Loaded) {
        let folder = self
            .data
            .as_ref()
            .map(|d| d.scan.tree.path(self.explore.folder));
        let (clean, shrink) = (self.clean_sel.clone(), self.shrink_sel.clone());
        self.load(data);
        let Some(d) = self.data.as_ref() else { return };
        if let Some(id) = folder.and_then(|f| d.scan.tree.find(&f)) {
            self.explore.folder = id;
        }
        self.clean_sel = clean
            .into_iter()
            .filter(|r| d.report.clean.iter().any(|c| &c.rule == r))
            .collect();
        self.shrink_sel = shrink;
    }

    pub fn filter(&self) -> Filter {
        Filter {
            categories: self.large.cats.clone(),
            min_size: SIZES[self.large.min],
            min_age_days: AGES[self.large.age],
            limit: 500,
        }
    }

    pub fn refresh_large(&mut self) {
        let f = self.filter();
        let Some(d) = &self.data else { return };
        let tree = &d.scan.tree;
        self.large.ids = if self.large.folders {
            largest::folders(tree, &d.dominant, &f, d.now)
        } else {
            largest::files(tree, &f, d.now)
        };
        let keep: BTreeSet<NodeId> = self.large.ids.iter().copied().collect();
        self.large.sel.retain(|id| keep.contains(id));
    }

    /// Bytes per category of what the current screen has selected.
    pub fn chosen(&self) -> [u64; 11] {
        let mut out = [0u64; 11];
        let Some(d) = &self.data else { return out };
        let tree = &d.scan.tree;
        match self.screen {
            Screen::Clean => {
                for r in d
                    .report
                    .clean
                    .iter()
                    .filter(|r| self.clean_sel.contains(&r.rule))
                {
                    for f in &r.folders {
                        if let Some(id) = tree.find(f) {
                            let n = tree.node(id);
                            out[n.category.index()] += n.alloc;
                        }
                    }
                }
            }
            Screen::Shrink => {
                for r in d
                    .report
                    .shrink
                    .iter()
                    .filter(|r| self.shrink_sel.contains(&r.path))
                {
                    if let Some(e) = &r.estimate {
                        out[r.category.index()] += e.saving_mid();
                    }
                }
            }
            Screen::Large | Screen::Category(_) => {
                for &id in &self.large.sel {
                    let cat = d.dominant[id as usize];
                    out[cat.index()] += tree.node(id).alloc;
                }
            }
            _ => {}
        }
        out
    }

    pub fn go(&mut self, screen: Screen) {
        if self.screen != screen {
            self.confirm = None;
            if matches!(screen, Screen::Category(_)) || matches!(self.screen, Screen::Category(_)) {
                self.large.sel.clear();
            }
        }
        self.screen = screen;
        self.drive_menu = false;
        self.preview = None;
        self.preview_t = 0.0;
    }
}
