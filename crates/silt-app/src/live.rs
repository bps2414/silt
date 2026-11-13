//! The real drive: the scan, the Shrink estimate and every action run on their own threads and
//! report back through a channel; the waker brings the window back to read it in `poll`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use rune::Waker;
use serde::{Deserialize, Serialize};
use silt_core::journal::{Action, Entry, Journal};
use silt_core::known::KnownDirs;
use silt_core::report::{EstimateJob, Report, run_estimates};
use silt_core::rules::Rule;
use silt_core::scan::{self, Fallback, Progress};
use silt_core::shrink::{self, Algorithm, Estimate, Excludes};
use silt_core::volume::Volume;
use silt_core::{clean, now_unix, trash};

use crate::app::{App, Job, JobKind, Loaded, Outcome, Phase, Settings, Theme};
use crate::i18n::{Lang, t};

/// What the threads send back.
pub enum Msg {
    Scanned {
        generation: u64,
        quiet: bool,
        result: Result<Box<Scanned>, String>,
    },
    Estimated {
        generation: u64,
        index: usize,
        estimate: Estimate,
    },
    EstimatesDone {
        generation: u64,
    },
    JobDone {
        outcome: Outcome,
        /// Shrink folders that went through, for the Restore that follows.
        shrunk: Vec<String>,
        /// Paths whose estimate is stale now.
        touched: Vec<String>,
    },
}

pub struct Scanned {
    pub loaded: Loaded,
    pub jobs: Vec<EstimateJob>,
    pub fallback: Option<Fallback>,
}

struct Running {
    progress: Arc<Progress>,
    cancel: Arc<AtomicBool>,
    used: u64,
}

/// An action's progress, shared with its thread.
#[derive(Default)]
pub struct JobCtl {
    pub cancel: AtomicBool,
    pub done: AtomicU64,
    pub total: AtomicU64,
    pub current: Mutex<String>,
    pub shrink: shrink::Control,
}

impl JobCtl {
    fn stopped(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
    fn at(&self, what: &str) {
        *self.current.lock().unwrap_or_else(|e| e.into_inner()) = what.to_string();
    }
}

pub struct Live {
    pub known: Arc<KnownDirs>,
    pub rules: Arc<Vec<Rule>>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    waker: Option<Waker>,
    generation: u64,
    scan: Option<Running>,
    estimate_cancel: Option<Arc<AtomicBool>>,
    /// Folders estimated and to estimate, while the estimate runs.
    pub estimating: Option<(usize, usize)>,
    estimate_algo: Algorithm,
    job: Option<(JobKind, Arc<JobCtl>)>,
    saved: Settings,
    /// The UAC prompt was declined at launch.
    pub declined: bool,
}

impl Live {
    pub fn new(known: KnownDirs, settings: Settings, declined: bool) -> Self {
        let (tx, rx) = channel();
        Live {
            known: Arc::new(known),
            rules: Arc::new(silt_core::rules::builtin()),
            tx,
            rx,
            waker: None,
            generation: 0,
            scan: None,
            estimate_cancel: None,
            estimating: None,
            estimate_algo: settings.algo,
            job: None,
            saved: settings,
            declined,
        }
    }

    fn sender(&self) -> (Sender<Msg>, Waker) {
        let waker = self.waker.clone().unwrap_or_else(|| Arc::new(|| {}));
        (self.tx.clone(), waker)
    }

    pub fn journal(&self) -> Journal {
        Journal::for_user(&self.known)
    }

    pub fn busy(&self) -> bool {
        self.job.is_some()
    }
}

// ---------------------------------------------------------------- launch and administrator

/// The original user's folders: from `--user-env` when this process was elevated by Silt.
pub fn known_dirs() -> KnownDirs {
    let args = rune::extra_args();
    if let Some(i) = args.iter().position(|a| a == "--user-env")
        && let Some(path) = args.get(i + 1)
    {
        let path = Path::new(path);
        if let Ok(k) = KnownDirs::from_snapshot(path) {
            let _ = std::fs::remove_file(path);
            return k;
        }
    }
    KnownDirs::current()
}

/// Whether this launch came from an elevation Silt asked for (never ask twice).
pub fn relaunched() -> bool {
    rune::extra_args().iter().any(|a| a == "--user-env")
}

/// Starts Silt again as administrator, handing it this user's folders. Ok means the elevated
/// copy is starting and this one should close.
pub fn elevate(known: &KnownDirs) -> std::io::Result<()> {
    let snap = known
        .temp()
        .join(format!("silt-user-env-{}.json", std::process::id()));
    known.write_snapshot(&snap)?;
    let args = format!("--user-env \"{}\"", snap.display());
    silt_core::win::relaunch_elevated(&args).inspect_err(|_| {
        let _ = std::fs::remove_file(&snap);
    })
}

// ---------------------------------------------------------------- settings

#[derive(Serialize, Deserialize)]
struct SettingsFile {
    lang: String,
    theme: String,
    ask_admin: bool,
    algorithm: String,
}

fn settings_path(known: &KnownDirs) -> PathBuf {
    known.local().join("Silt").join("settings.json")
}

pub fn load_settings(known: &KnownDirs) -> Settings {
    let mut s = Settings::default();
    let Ok(text) = std::fs::read_to_string(settings_path(known)) else {
        return s;
    };
    let Ok(f) = serde_json::from_str::<SettingsFile>(&text) else {
        return s;
    };
    s.lang = match f.lang.as_str() {
        "en" => Lang::English,
        "pt-BR" => Lang::Portuguese,
        _ => Lang::System,
    };
    s.theme = match f.theme.as_str() {
        "light" => Theme::Light,
        "dark" => Theme::Dark,
        _ => Theme::System,
    };
    s.ask_admin = f.ask_admin;
    s.algo = Algorithm::from_id(&f.algorithm).unwrap_or_default();
    s
}

fn save_settings(known: &KnownDirs, s: &Settings) -> std::io::Result<()> {
    let f = SettingsFile {
        lang: match s.lang {
            Lang::System => "system",
            Lang::English => "en",
            Lang::Portuguese => "pt-BR",
        }
        .into(),
        theme: match s.theme {
            Theme::System => "system",
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
        .into(),
        ask_admin: s.ask_admin,
        algorithm: s.algo.id().into(),
    };
    let path = settings_path(known);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(&f)?)
}

// ---------------------------------------------------------------- the app's side

impl App {
    /// The window on the real drives, starting with the system drive.
    pub fn real(known: KnownDirs, settings: Settings, declined: bool) -> App {
        let mut drives = silt_core::volume::list();
        let system = known
            .windows()
            .to_string_lossy()
            .chars()
            .next()
            .unwrap_or('C')
            .to_ascii_uppercase();
        if let Some(i) = drives.iter().position(|v| v.letter == system) {
            let v = drives.remove(i);
            drives.insert(0, v);
        }
        let mut s = App::new(drives);
        s.settings = settings.clone();
        s.live = Some(Box::new(Live::new(known, settings, declined)));
        if s.drives.is_empty() {
            s.phase = Phase::Failed {
                drive: "C:".into(),
                message: "No local drive found.".into(),
            };
        } else {
            s.start_scan(false);
        }
        s
    }

    pub fn set_waker(&mut self, w: Waker) {
        if let Some(l) = self.live.as_mut() {
            l.waker = Some(w);
        }
    }

    /// Scans the current drive. `quiet` keeps what is on screen until the new scan is ready
    /// (after an action).
    pub fn start_scan(&mut self, quiet: bool) {
        let Some(vol) = self.volume().cloned() else {
            return;
        };
        let Some(live) = self.live.as_mut() else {
            return;
        };
        if let Some(r) = live.scan.take() {
            r.cancel.store(true, Ordering::Relaxed);
        }
        if let Some(c) = live.estimate_cancel.take() {
            c.store(true, Ordering::Relaxed);
        }
        live.estimating = None;
        live.generation += 1;
        let generation = live.generation;
        let (handle, fallback) = scan::start(&vol, true);
        live.scan = Some(Running {
            progress: handle.progress.clone(),
            cancel: handle.cancel.clone(),
            used: vol.used(),
        });
        // Estimates still good after an action: every folder it didn't touch.
        let keep: HashMap<String, Estimate> = if quiet {
            self.data
                .as_ref()
                .map(|d| {
                    d.report
                        .shrink
                        .iter()
                        .filter_map(|r| Some((r.path.clone(), r.estimate.clone()?)))
                        .collect()
                })
                .unwrap_or_default()
        } else {
            HashMap::new()
        };
        if !quiet {
            self.data = None;
            self.phase = Phase::Scanning {
                found: 0,
                entries: 0,
                used: vol.used(),
            };
            self.outcome = None;
            self.confirm = None;
        }
        let (tx, wake) = live.sender();
        let (known, rules) = (live.known.clone(), live.rules.clone());
        let journal = live.journal();
        std::thread::Builder::new()
            .name("silt-scan".into())
            .spawn(move || {
                let result = handle
                    .collect(&known, &rules)
                    .map_err(|e| {
                        if e.kind() == std::io::ErrorKind::Interrupted {
                            t!("scan-stopped").to_string()
                        } else {
                            e.to_string()
                        }
                    })
                    .map(|scan| {
                        let entries = journal.read();
                        let now = now_unix();
                        let bin = trash::bin_info_drive(&scan.volume.root);
                        let mut report = Report::build(&scan, &rules, &entries, bin, now);
                        for i in 0..report.shrink.len() {
                            if let Some(e) = keep.get(&report.shrink[i].path) {
                                report.set_estimate(i, e.clone());
                            }
                        }
                        let jobs = report.estimate_jobs(&scan.tree);
                        let loaded = Loaded::new(scan, report, rules.to_vec(), entries, now);
                        Box::new(Scanned {
                            loaded,
                            jobs,
                            fallback,
                        })
                    });
                let _ = tx.send(Msg::Scanned {
                    generation,
                    quiet,
                    result,
                });
                wake();
            })
            .ok();
    }

    pub fn stop_scan(&mut self) {
        if let Some(r) = self.live.as_ref().and_then(|l| l.scan.as_ref()) {
            r.cancel.store(true, Ordering::Relaxed);
        }
    }

    pub fn switch_drive(&mut self, i: usize) {
        self.drive_menu = false;
        if i == self.drive && self.live.is_some() {
            return;
        }
        self.drive = i;
        self.start_scan(false);
    }

    fn start_estimates(&mut self, jobs: Vec<EstimateJob>) {
        let (Some(live), Some(d)) = (self.live.as_mut(), self.data.as_ref()) else {
            return;
        };
        if let Some(c) = live.estimate_cancel.take() {
            c.store(true, Ordering::Relaxed);
        }
        if jobs.is_empty() {
            live.estimating = None;
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        live.estimate_cancel = Some(cancel.clone());
        live.estimating = Some((0, jobs.len()));
        live.estimate_algo = self.settings.algo;
        let (tx, wake) = live.sender();
        let (generation, algo, cluster) =
            (live.generation, self.settings.algo, d.scan.volume.cluster);
        std::thread::Builder::new()
            .name("silt-estimate".into())
            .spawn(move || {
                run_estimates(&jobs, cluster, algo, &cancel, |index, estimate| {
                    let _ = tx.send(Msg::Estimated {
                        generation,
                        index,
                        estimate,
                    });
                    wake();
                });
                if !cancel.load(Ordering::Relaxed) {
                    let _ = tx.send(Msg::EstimatesDone { generation });
                    wake();
                }
            })
            .ok();
    }

    /// Estimates every folder again (the method changed).
    fn restart_estimates(&mut self) {
        let Some(d) = self.data.as_mut() else { return };
        for i in 0..d.report.shrink.len() {
            d.report.shrink[i].estimate = None;
        }
        for c in &mut d.report.categories {
            c.shrink = 0;
        }
        let jobs = d.report.estimate_jobs(&d.scan.tree);
        self.shrink_sel.clear();
        self.start_estimates(jobs);
    }

    /// Reads what the threads sent; true when something changed. Called from `poll`.
    pub fn pump(&mut self) -> bool {
        let mut changed = false;
        // Live counters of the running scan.
        if let Some(live) = self.live.as_ref()
            && let Some(r) = &live.scan
            && let Phase::Scanning {
                found,
                entries,
                used,
            } = &mut self.phase
        {
            let (f, e) = (
                r.progress.bytes.load(Ordering::Relaxed),
                r.progress.entries.load(Ordering::Relaxed),
            );
            if (*found, *entries) != (f, e) {
                (*found, *entries, *used) = (f.min(r.used), e, r.used);
                changed = true;
            }
        }
        if let Some((kind, ctl)) = self.live.as_ref().and_then(|l| l.job.clone()) {
            let (done, total) = if matches!(kind, JobKind::Shrink) {
                (
                    ctl.shrink.done_files.load(Ordering::Relaxed),
                    ctl.total.load(Ordering::Relaxed),
                )
            } else {
                (
                    ctl.done.load(Ordering::Relaxed),
                    ctl.total.load(Ordering::Relaxed),
                )
            };
            let current = ctl
                .current
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            let job = Job {
                kind,
                done: done.min(total),
                total,
                current,
            };
            let same = self.job.as_ref().is_some_and(|j| {
                (j.kind, j.done, j.total, &j.current)
                    == (job.kind, job.done, job.total, &job.current)
            });
            if !same {
                self.job = Some(job);
                changed = true;
            }
        }
        while let Some(msg) = self.live.as_ref().and_then(|l| l.rx.try_recv().ok()) {
            changed = true;
            self.take(msg);
        }
        // Settings: save what changed; a new method needs new estimates.
        if let Some(live) = self.live.as_mut()
            && live.saved != self.settings
        {
            let _ = save_settings(&live.known, &self.settings);
            live.saved = self.settings.clone();
            if live.estimate_algo != self.settings.algo && self.data.is_some() {
                live.estimate_algo = self.settings.algo;
                self.restart_estimates();
                changed = true;
            }
        }
        changed
    }

    /// True while something runs that the window should keep checking on.
    pub fn working(&self) -> bool {
        self.live
            .as_ref()
            .is_some_and(|l| l.scan.is_some() || l.job.is_some())
    }

    fn take(&mut self, msg: Msg) {
        let generation = self.live.as_ref().map_or(0, |l| l.generation);
        match msg {
            Msg::Scanned {
                generation: g,
                quiet,
                result,
            } if g == generation => {
                if let Some(l) = self.live.as_mut() {
                    l.scan = None;
                }
                match result {
                    Ok(sc) => {
                        let Scanned {
                            loaded,
                            jobs,
                            fallback,
                        } = *sc;
                        if let Some(v) = self.drives.get_mut(self.drive) {
                            *v = loaded.scan.volume.clone();
                        }
                        if quiet {
                            self.reload(loaded);
                        } else {
                            self.load(loaded);
                            self.color_t = 0.0;
                            self.fallback = fallback;
                            self.fallback_hidden = false;
                        }
                        self.start_estimates(jobs);
                    }
                    Err(message) if !quiet => {
                        self.phase = Phase::Failed {
                            drive: self.volume().map(Volume::drive).unwrap_or_default(),
                            message,
                        };
                    }
                    Err(_) => {}
                }
            }
            Msg::Estimated {
                generation: g,
                index,
                estimate,
            } if g == generation => {
                let pick = estimate.saving_mid() >= 256 << 20;
                if let Some(d) = self.data.as_mut() {
                    d.report.set_estimate(index, estimate);
                    if pick
                        && !self.shrink_touched
                        && self.shrink_sel.len() < 5
                        && let Some(r) = d.report.shrink.get(index)
                    {
                        self.shrink_sel.insert(r.path.clone());
                    }
                }
                if let Some((done, _)) = self.live.as_mut().and_then(|l| l.estimating.as_mut()) {
                    *done += 1;
                }
            }
            Msg::EstimatesDone { generation: g } if g == generation => {
                if let Some(l) = self.live.as_mut() {
                    l.estimating = None;
                    l.estimate_cancel = None;
                }
            }
            Msg::JobDone {
                outcome,
                shrunk,
                touched,
            } => {
                if let Some(l) = self.live.as_mut() {
                    l.job = None;
                }
                self.job = None;
                self.outcome = Some(outcome);
                if !shrunk.is_empty() {
                    self.last_shrunk = shrunk;
                }
                if let Some(d) = self.data.as_mut() {
                    for r in &mut d.report.shrink {
                        if touched.iter().any(|t| t.eq_ignore_ascii_case(&r.path)) {
                            r.estimate = None;
                        }
                    }
                }
                self.shrink_sel
                    .retain(|p| !touched.iter().any(|t| t.eq_ignore_ascii_case(p)));
                self.large.sel.clear();
                self.start_scan(true);
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------ actions

    fn run_job(
        &mut self,
        kind: JobKind,
        total: u64,
        work: impl FnOnce(&JobCtl, &KnownDirs, &Journal) -> Msg + Send + 'static,
    ) {
        let Some(live) = self.live.as_mut() else {
            return;
        };
        if live.job.is_some() {
            return;
        }
        let ctl = Arc::new(JobCtl::default());
        ctl.total.store(total, Ordering::Relaxed);
        live.job = Some((kind, ctl.clone()));
        self.confirm = None;
        self.outcome = None;
        self.job = Some(Job {
            kind,
            done: 0,
            total,
            current: String::new(),
        });
        let (tx, wake) = live.sender();
        let known = live.known.clone();
        let journal = live.journal();
        std::thread::Builder::new()
            .name("silt-action".into())
            .spawn(move || {
                let msg = work(&ctl, &known, &journal);
                let _ = tx.send(msg);
                wake();
            })
            .ok();
    }

    pub fn stop_job(&mut self) {
        match self.live.as_ref().and_then(|l| l.job.as_ref()) {
            Some((_, ctl)) => {
                ctl.cancel.store(true, Ordering::Relaxed);
                ctl.shrink.cancel.store(true, Ordering::Relaxed);
            }
            // The fixed drive: nothing runs behind the scene's job.
            None => self.job = None,
        }
    }

    pub fn clean_now(&mut self) {
        let Some(d) = self.data.as_ref() else { return };
        let rules: Vec<(Rule, Vec<String>)> = d
            .report
            .clean
            .iter()
            .filter(|r| self.clean_sel.contains(&r.rule))
            .filter_map(|r| Some((d.rule(&r.rule)?.clone(), r.folders.clone())))
            .collect();
        if rules.is_empty() {
            return;
        }
        let total = rules.len() as u64;
        self.run_job(JobKind::Clean, total, move |ctl, known, journal| {
            let (mut freed, mut done, mut skipped) = (0u64, 0u64, 0u64);
            let mut touched = Vec::new();
            let mut failed = None;
            for (rule, folders) in &rules {
                if ctl.stopped() {
                    break;
                }
                ctl.at(&rule.name);
                let r = clean::clean_rule(rule, known, &ctl.cancel);
                freed += r.freed();
                skipped += (r.skipped.len() + r.refused.len()) as u64;
                touched.extend(folders.iter().cloned());
                let _ = journal.append(&Entry {
                    ts: now_unix(),
                    action: Action::Clean,
                    target: rule.id.clone(),
                    algorithm: None,
                    before: r.before,
                    after: r.after,
                    files: r.deleted_files,
                    skipped: (r.skipped.len() + r.refused.len()) as u64,
                });
                if r.cancelled {
                    break;
                }
                if r.targets.is_empty() && !r.refused.is_empty() && failed.is_none() {
                    failed = Some(r.refused[0].reason.clone());
                }
                done += 1;
                ctl.done.store(done, Ordering::Relaxed);
            }
            let outcome = if ctl.stopped() {
                Outcome::Stopped {
                    kind: JobKind::Clean,
                    done,
                    total,
                    bytes: freed,
                }
            } else if let (Some(message), 0) = (failed, freed) {
                Outcome::Failed {
                    kind: JobKind::Clean,
                    message,
                }
            } else {
                Outcome::Done {
                    kind: JobKind::Clean,
                    count: done,
                    bytes: freed,
                    skipped,
                }
            };
            Msg::JobDone {
                outcome,
                shrunk: Vec::new(),
                touched,
            }
        });
    }

    pub fn shrink_now(&mut self) {
        let Some(d) = self.data.as_ref() else { return };
        let rows: Vec<(String, u64)> = d
            .report
            .shrink
            .iter()
            .filter(|r| self.shrink_sel.contains(&r.path))
            .map(|r| (r.path.clone(), r.files))
            .collect();
        if rows.is_empty() {
            return;
        }
        let algo = self.settings.algo;
        let files: u64 = rows.iter().map(|r| r.1).sum();
        self.run_job(JobKind::Shrink, files.max(1), move |ctl, _, journal| {
            shrink_folders(ctl, journal, rows.into_iter().map(|r| r.0).collect(), algo)
        });
    }

    /// Restores `folders` (History's Restore, or the one after a Shrink).
    pub fn restore_now(&mut self, folders: Vec<String>) {
        if folders.is_empty() || self.live.is_none() {
            return;
        }
        let total = folders.len() as u64;
        self.run_job(JobKind::Restore, total, move |ctl, _, journal| {
            let (mut grew, mut done, mut skipped) = (0i64, 0u64, 0u64);
            let mut failed = None;
            for f in &folders {
                if ctl.stopped() {
                    break;
                }
                ctl.at(crate::screens::leaf(f));
                let r = shrink::restore_folder(Path::new(f), &ctl.shrink);
                if let Some(why) = r.refused {
                    failed = Some(why);
                    break;
                }
                grew -= r.saved();
                skipped += r.skipped.len() as u64;
                let _ = journal.append(&Entry {
                    ts: now_unix(),
                    action: Action::Restore,
                    target: f.clone(),
                    algorithm: None,
                    before: r.before,
                    after: r.after,
                    files: r.files,
                    skipped: r.skipped.len() as u64,
                });
                if r.cancelled {
                    break;
                }
                done += 1;
                ctl.done.store(done, Ordering::Relaxed);
            }
            let outcome = match failed {
                Some(message) => Outcome::Failed {
                    kind: JobKind::Restore,
                    message,
                },
                None if ctl.stopped() => Outcome::Stopped {
                    kind: JobKind::Restore,
                    done,
                    total,
                    bytes: grew.max(0) as u64,
                },
                None => Outcome::Done {
                    kind: JobKind::Restore,
                    count: done,
                    bytes: grew.max(0) as u64,
                    skipped,
                },
            };
            Msg::JobDone {
                outcome,
                shrunk: Vec::new(),
                touched: folders,
            }
        });
    }

    pub fn trash_now(&mut self) {
        let Some(d) = self.data.as_ref() else { return };
        let tree = &d.scan.tree;
        let mut paths: Vec<String> = self.large.sel.iter().map(|&id| tree.path(id)).collect();
        // Inside another chosen folder: it goes with that folder.
        let all = paths.clone();
        paths.retain(|p| {
            !all.iter()
                .any(|o| o.len() < p.len() && silt_core::win::path_within(p, o))
        });
        if paths.is_empty() {
            return;
        }
        let total = paths.len() as u64;
        self.run_job(JobKind::Trash, total, move |ctl, known, journal| {
            let (mut bytes, mut done, mut skipped) = (0u64, 0u64, 0u64);
            let mut failed = None;
            for p in &paths {
                if ctl.stopped() {
                    break;
                }
                ctl.at(crate::screens::leaf(p));
                let r = trash::send_to_trash(&[PathBuf::from(p)], known);
                bytes += r.bytes;
                done += r.trashed.len() as u64;
                skipped += (r.refused.len() + r.failed.len()) as u64;
                if let Some(s) = r.refused.first().or(r.failed.first())
                    && failed.is_none()
                {
                    failed = Some(format!("{}: {}", crate::screens::leaf(&s.path), s.reason));
                }
                ctl.done.store(done + skipped, Ordering::Relaxed);
            }
            if done > 0 {
                let _ = journal.append(&Entry {
                    ts: now_unix(),
                    action: Action::Trash,
                    target: format!("{done} items"),
                    algorithm: None,
                    before: bytes,
                    after: 0,
                    files: done,
                    skipped,
                });
            }
            let outcome = if ctl.stopped() {
                Outcome::Stopped {
                    kind: JobKind::Trash,
                    done,
                    total,
                    bytes,
                }
            } else if let (0, Some(message)) = (done, failed) {
                Outcome::Failed {
                    kind: JobKind::Trash,
                    message,
                }
            } else {
                Outcome::Done {
                    kind: JobKind::Trash,
                    count: done,
                    bytes,
                    skipped,
                }
            };
            Msg::JobDone {
                outcome,
                shrunk: Vec::new(),
                touched: paths,
            }
        });
    }

    pub fn empty_bin_now(&mut self) {
        self.run_job(JobKind::EmptyBin, 1, |_, _, journal| {
            let outcome = match trash::empty_bin() {
                Ok(before) => {
                    if before.items > 0 {
                        let _ = journal.append(&Entry {
                            ts: now_unix(),
                            action: Action::EmptyBin,
                            target: format!("{} items", before.items),
                            algorithm: None,
                            before: before.bytes,
                            after: 0,
                            files: before.items,
                            skipped: 0,
                        });
                    }
                    Outcome::Done {
                        kind: JobKind::EmptyBin,
                        count: before.items,
                        bytes: before.bytes,
                        skipped: 0,
                    }
                }
                Err(e) => Outcome::Failed {
                    kind: JobKind::EmptyBin,
                    message: e.to_string(),
                },
            };
            Msg::JobDone {
                outcome,
                shrunk: Vec::new(),
                touched: Vec::new(),
            }
        });
    }

    /// "Restart as administrator": hands over to an elevated copy, or stays if UAC says no.
    pub fn restart_admin(&mut self) {
        let Some(live) = self.live.as_ref() else {
            return;
        };
        if elevate(&live.known).is_ok() {
            self.quit = true;
        }
    }
}

fn shrink_folders(ctl: &JobCtl, journal: &Journal, folders: Vec<String>, algo: Algorithm) -> Msg {
    let ex = Excludes::builtin();
    let (mut saved, mut done, mut skipped) = (0i64, 0u64, 0u64);
    let mut shrunk = Vec::new();
    let mut failed = None;
    for f in &folders {
        if ctl.stopped() {
            break;
        }
        ctl.at(crate::screens::leaf(f));
        let r = shrink::shrink_folder(Path::new(f), algo, &ex, &ctl.shrink);
        if let Some(why) = r.refused {
            failed = Some(why);
            break;
        }
        saved += r.saved();
        skipped += r.skipped.len() as u64;
        let _ = journal.append(&Entry {
            ts: now_unix(),
            action: Action::Shrink,
            target: f.clone(),
            algorithm: Some(algo.id().to_string()),
            before: r.before,
            after: r.after,
            files: r.files,
            skipped: r.skipped.len() as u64,
        });
        if r.files > 0 {
            shrunk.push(f.clone());
        }
        if r.cancelled {
            break;
        }
        done += 1;
        ctl.done.store(done, Ordering::Relaxed);
    }
    let bytes = saved.max(0) as u64;
    let total = folders.len() as u64;
    let outcome = match failed {
        Some(message) if done == 0 => Outcome::Failed {
            kind: JobKind::Shrink,
            message,
        },
        _ if ctl.stopped() => Outcome::Stopped {
            kind: JobKind::Shrink,
            done,
            total,
            bytes,
        },
        _ => Outcome::Done {
            kind: JobKind::Shrink,
            count: done,
            bytes,
            skipped,
        },
    };
    Msg::JobDone {
        outcome,
        shrunk,
        touched: folders,
    }
}
