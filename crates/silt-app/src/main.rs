//! Silt's window.

#![windows_subsystem = "windows"]

mod app;
mod bar;
mod fixture;
mod i18n;
mod live;
mod screens;
mod style;
mod treemap;
mod ui;

use rune::{Align, AppSpec, Color, Cx, El, Titlebar, Tx, col, el, row};
use silt_core::Category;
use silt_core::known::KnownDirs;
use silt_core::report::Report;
use silt_core::scan::Fallback;

use crate::app::{App, Confirm, Job, JobKind, Loaded, Outcome, Phase, Reclaim, Screen, Theme};
use crate::i18n::{t, tp};
use crate::style::Pal;

pub const TITLEBAR: f32 = 44.0;

fn view(s: &App, cx: &mut Cx<App>) -> El {
    i18n::apply(s.settings.lang);
    let dark = match s.settings.theme {
        Theme::System => cx.system_dark(),
        Theme::Light => false,
        Theme::Dark => true,
    };
    let scale = cx.env().scale;
    let p = style::pal(dark, scale);
    s.pal.set(p);
    let body = match s.screen {
        Screen::Overview => col()
            .key("overview-scroll")
            .w_full()
            .h_full()
            .min_h(0.0)
            .scroll(rune::ScrollSpec {
                stick_bottom: false,
            })
            .child(screens::overview::view(s, p, cx)),
        Screen::Clean => screens::clean::view(s, p, cx),
        Screen::Shrink => screens::shrink::view(s, p, cx),
        Screen::Large => screens::large::view(s, p, cx, None),
        Screen::Category(c) => screens::large::view(s, p, cx, Some(c)),
        Screen::Explore => screens::explore::view(s, p, cx),
        Screen::History => screens::history::view(s, p, cx),
        Screen::Settings => screens::settings::view(s, p, cx),
    };
    let mut root = col()
        .key("root")
        .w_full()
        .h_full()
        .fill(p.bed)
        .child(titlebar(s, p, cx))
        .child(col().key("body").w_full().grow(1.0).min_h(0.0).child(body));
    if s.drive_menu {
        root = root.child(drive_menu(s, p, cx));
    }
    root
}

fn mark(p: Pal) -> El {
    let (grain, pitch, pad) = (2.0, 2.5, 2.0);
    let size = pad * 2.0 + pitch * 8.0 - (pitch - grain);
    let mut tile = el()
        .key("mark")
        .size(size, size)
        .radius(5.0)
        .fill(style::rgb(0x1D3138))
        .a11y_hidden();
    for (y, line) in bar::MARK.iter().enumerate() {
        for (x, ch) in line.chars().enumerate() {
            if ch == '#' {
                tile = tile.child(
                    el().size(grain, grain)
                        .absolute(pad + x as f32 * pitch, pad + y as f32 * pitch)
                        .fill(style::rgb(0x5CC6BA)),
                );
            }
        }
    }
    let _ = p;
    tile
}

fn titlebar(s: &App, p: Pal, cx: &mut Cx<App>) -> El {
    let insets = cx.chrome_insets();
    let tabs: [(Screen, &str); 6] = [
        (Screen::Overview, t!("nav-overview")),
        (Screen::Clean, t!("nav-clean")),
        (Screen::Shrink, t!("nav-shrink")),
        (Screen::Large, t!("nav-large")),
        (Screen::Explore, t!("nav-explore")),
        (Screen::History, t!("nav-history")),
    ];
    let current = match s.screen {
        Screen::Category(_) => Screen::Overview,
        other => other,
    };
    let mut nav = row()
        .key("nav")
        .role("tablist")
        .gap(2.0)
        .h_full()
        .center_y();
    for (i, (screen, label)) in tabs.into_iter().enumerate() {
        nav = nav.child(tab(p, i, label, screen == current, move |s: &mut App| {
            s.go(screen)
        }));
    }
    let drive = s
        .volume()
        .map(|v| format!("{}  {}", v.drive(), v.label))
        .unwrap_or_default();
    let drive_btn = row()
        .key("drive")
        .role("button")
        .label(t!("drive-pick"))
        .h(30.0)
        .px(10.0)
        .gap(8.0)
        .center_y()
        .radius(6.0)
        .border(p.hair.max(1.0), p.line)
        .focusable(true)
        .hover(p.hover)
        .press(p.press)
        .on_press(|s: &mut App| s.drive_menu = !s.drive_menu)
        .child(ui::txt(drive, style::LABEL, p.ink).a11y_hidden())
        .child(chevron(p.ink2));
    let settings_on = s.screen == Screen::Settings;
    row()
        .key("titlebar")
        .w_full()
        .h(TITLEBAR)
        .center_y()
        .pl(insets.leading + 16.0)
        .pr(insets.trailing + 10.0)
        .gap(10.0)
        .window_drag()
        .child(mark(p))
        .child(
            ui::txt("silt", style::WORDMARK, p.ink)
                .a11y_hidden()
                .mr(18.0),
        )
        .child(nav)
        .child(el().key("drag").grow(1.0).h_full().window_drag())
        .child(drive_btn)
        .child(tab(p, 9, t!("nav-settings"), settings_on, |s: &mut App| {
            s.go(Screen::Settings)
        }))
}

/// A nav item. The current one is ink, with four grains under it.
fn tab(p: Pal, i: usize, label: &str, on: bool, f: impl Fn(&mut App) + 'static) -> El {
    let mut grains = row().key("grains").gap(2.0).h(3.0).a11y_hidden();
    for g in 0..4 {
        grains = grains.child(el().key(format!("g{g}")).size(3.0, 3.0).fill(if on {
            p.flour
        } else {
            Color::TRANSPARENT
        }));
    }
    col()
        .key(format!("tab{i}"))
        .role("tab")
        .label(label)
        .checked(on)
        .h(32.0)
        .px(10.0)
        .pt(7.0)
        .gap(3.0)
        .items_start()
        .radius(6.0)
        .focusable(true)
        .hover(p.hover)
        .press(p.press)
        .on_press(f)
        .child(ui::txt(label, style::LABEL, if on { p.ink } else { p.ink2 }).a11y_hidden())
        .child(grains)
}

fn chevron(c: Color) -> El {
    el().size(10.0, 10.0).a11y_hidden().shape(
        rune::Shape::new()
            .stroke(vec![[2.0, 3.5], [5.0, 6.5], [8.0, 3.5]], 1.4, c)
            .view_box(10.0, 10.0),
    )
}

fn drive_menu(s: &App, p: Pal, cx: &mut Cx<App>) -> El {
    let w = 300.0;
    let win = cx.layout_size("root").map_or(1080.0, |[w, _]| w);
    let insets = cx.chrome_insets();
    let x = (win - insets.trailing - 10.0 - 90.0 - w).max(8.0);
    let mut list = col()
        .key("drive-menu")
        .role("menu")
        .w(w)
        .padding(6.0)
        .gap(2.0)
        .absolute(x, TITLEBAR + 2.0)
        .radius(10.0)
        .fill(p.surface)
        .border(p.hair.max(1.0), p.line)
        .shadow(
            8.0,
            24.0,
            Color::BLACK.alpha(if p.dark { 0.45 } else { 0.14 }),
        );
    for (i, v) in s.drives.iter().enumerate() {
        let on = i == s.drive;
        list = list.child(
            row()
                .key(format!("drive{i}"))
                .role("menuitem")
                .label(v.drive())
                .checked(on)
                .h(48.0)
                .px(10.0)
                .gap(12.0)
                .center_y()
                .radius(7.0)
                .focusable(true)
                .hover(p.hover)
                .press(p.press)
                .on_press(move |s: &mut App| s.switch_drive(i))
                .child(ui::txt(v.drive(), style::TITLE, p.ink).w(34.0))
                .child(
                    col()
                        .grow(1.0)
                        .child(ui::txt(v.label.clone(), style::LABEL, p.ink))
                        .child(ui::txt(
                            t!(
                                "drive-used",
                                used = i18n::size(v.used()),
                                total = i18n::size(v.total)
                            ),
                            style::BODY,
                            p.ink2,
                        )),
                )
                .child(if on {
                    ui::check(p, true)
                } else {
                    el().size(16.0, 16.0)
                }),
        );
    }
    list
}

/// Page frame: header, an optional bar strip, the scrolling body and a footer.
pub fn page(p: Pal, key: &str, header: El, strip: Option<El>, body: El, footer: Option<El>) -> El {
    let mut c = col()
        .key(key)
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(header.px(style::PAD).pt(18.0).pb(14.0));
    if let Some(sb) = strip {
        c = c.child(sb.px(style::PAD).pb(8.0));
    }
    c = c.child(
        col()
            .key("scroll")
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .scroll(rune::ScrollSpec {
                stick_bottom: false,
            })
            .child(body.px(style::PAD).pb(24.0)),
    );
    if let Some(f) = footer {
        c = c.child(ui::hairline(p)).child(
            row()
                .key("footer")
                .w_full()
                .min_h(64.0)
                .px(style::PAD)
                .py(12.0)
                .gap(16.0)
                .center_y()
                .child(f),
        );
    }
    c
}

/// A screen's title block, with what it adds up to on the right.
pub fn header(p: Pal, title: &str, sub: &str, figure: Option<(String, String)>) -> El {
    let mut r = row().key("header").w_full().gap(24.0).items_end().child(
        col()
            .grow(1.0)
            .gap(4.0)
            .child(ui::txt(title, style::TITLE, p.ink))
            .child(ui::txt(sub, style::BODY, p.ink2).wrap().max_w(620.0)),
    );
    if let Some((big, small)) = figure {
        r = r.child(
            col()
                .items_end()
                .gap(2.0)
                .child(ui::txt(big, style::HEADLINE, p.ink).align(Align::End))
                .child(ui::txt(small, style::BODY, p.ink2).align(Align::End)),
        );
    }
    r
}

/// The compact bar under a screen's header, with what the screen has chosen drawn sparse.
pub fn strip(s: &App, p: Pal, cx: &mut Cx<App>) -> El {
    let scale = cx.env().scale;
    // The inner row: the slot itself carries the page's padding.
    let width = cx.layout_size("strip-inner").map_or(1020.0, |[w, _]| w);
    let g = bar::Geom::fit(width, 6, scale);
    let (w, h) = g.points(scale);
    if let (Some(d), Some(v)) = (&s.data, s.volume()) {
        let chosen = s.chosen();
        s.strip.draw(
            g,
            v.total,
            &d.segs_chosen(&chosen),
            bar::Look::Sorted {
                color: 1.0,
                fade: 0.0,
            },
            &bar::Ink {
                free: p.free8,
                ink2: p.ink2_8,
            },
        );
    }
    row().key("strip-slot").w_full().child(
        row().key("strip-inner").w_full().h(h).child(
            el().key("strip")
                .size(w, h)
                .pixels(&s.strip.pixels)
                .label(t!("bar-label")),
        ),
    )
}

/// What an action did or is doing, for a footer.
pub fn job_line(s: &App, p: Pal, kinds: &[JobKind]) -> Option<El> {
    if let Some(j) = s.job.as_ref().filter(|j| kinds.contains(&j.kind)) {
        let frac = if j.total == 0 {
            0.0
        } else {
            j.done as f32 / j.total as f32
        };
        let what = match j.kind {
            JobKind::Clean => t!("job-cleaning", what = j.current.clone()),
            JobKind::Shrink => t!("job-shrinking", what = j.current.clone()),
            JobKind::Restore => t!("job-restoring", what = j.current.clone()),
            JobKind::Trash => t!("job-trashing").to_string(),
            JobKind::EmptyBin => t!("job-emptying").to_string(),
        };
        return Some(
            row()
                .key("job")
                .grow(1.0)
                .gap(16.0)
                .center_y()
                .child(
                    col()
                        .grow(1.0)
                        .gap(8.0)
                        .child(
                            row()
                                .gap(12.0)
                                .child(ui::txt(what, style::LABEL, p.ink).grow(1.0).truncate())
                                .child(ui::txt(
                                    t!(
                                        "job-count",
                                        done = i18n::int(j.done),
                                        total = i18n::int(j.total)
                                    ),
                                    style::FIGURE,
                                    p.ink2,
                                )),
                        )
                        .child(ui::progress(p, frac)),
                )
                .child(ui::button(
                    p,
                    "stop",
                    t!("stop"),
                    ui::Kind::Quiet,
                    Some(|s: &mut App| s.stop_job()),
                )),
        );
    }
    let o = s.outcome.as_ref()?;
    let (kind, line, tone) = match o {
        Outcome::Done {
            kind,
            count,
            bytes,
            skipped,
        } => {
            let mut line = match kind {
                JobKind::Clean => tp!("done-clean", *count, size = i18n::size(*bytes)),
                JobKind::Shrink => tp!("done-shrink", *count, size = i18n::size(*bytes)),
                JobKind::Restore => t!("done-restore", size = i18n::size(*bytes)),
                JobKind::Trash => tp!("done-trash", *count),
                JobKind::EmptyBin => t!("done-empty", size = i18n::size(*bytes)),
            };
            if *skipped > 0 {
                line.push(' ');
                line.push_str(&tp!("skipped", *skipped));
            }
            (*kind, line, p.flour)
        }
        Outcome::Stopped {
            kind,
            done,
            total,
            bytes,
        } => {
            let line = match kind {
                JobKind::Shrink => t!(
                    "stopped-shrink",
                    done = i18n::int(*done),
                    total = i18n::int(*total),
                    size = i18n::size(*bytes)
                ),
                _ => t!(
                    "stopped",
                    done = i18n::int(*done),
                    total = i18n::int(*total)
                ),
            };
            (*kind, line, p.ink2)
        }
        Outcome::Failed { kind, message } => (*kind, message.clone(), p.danger),
    };
    if !kinds.contains(&kind) {
        return None;
    }
    Some(
        row()
            .key("outcome")
            .grow(1.0)
            .gap(10.0)
            .center_y()
            .child(el().size(8.0, 8.0).radius(1.5).fill(tone))
            .child(ui::txt(line, style::LABEL, p.ink).wrap().grow(1.0)),
    )
}

/// What to say for each category's own screen.
pub fn category_hint(c: Category) -> Option<(&'static str, Screen)> {
    match c {
        Category::Caches => Some((t!("hint-caches"), Screen::Clean)),
        Category::Apps | Category::Games | Category::Developer => {
            Some((t!("hint-shrink"), Screen::Shrink))
        }
        _ => None,
    }
}

fn poll(s: &mut App, clock: f64, tx: &mut Tx) -> bool {
    let dt = (clock - s.clock).clamp(0.0, 0.1) as f32;
    s.clock = clock;
    let changed = s.pump();
    if s.quit {
        tx.quit();
    }
    // A scan or an action runs: read its counters again soon. Idle, nothing wakes the window.
    if s.working() {
        tx.wake_after(std::time::Duration::from_millis(50));
    }
    let mut moving = false;
    let target = if s.preview.is_some() { 1.0 } else { 0.0 };
    if s.preview_t != target {
        let step = dt / 0.12;
        s.preview_t = if target > s.preview_t {
            (s.preview_t + step).min(1.0)
        } else {
            (s.preview_t - step).max(0.0)
        };
        moving = true;
    }
    if s.color_t < 1.0 {
        s.color_t = (s.color_t + dt / 0.24).min(1.0);
        moving = true;
    }
    if moving {
        tx.wake_after(std::time::Duration::ZERO);
    }
    moving || changed
}

fn on_key(s: &mut App, k: &rune::KeyEvent, _tx: &mut Tx) -> bool {
    if k.key == rune::Key::Escape {
        if s.drive_menu || s.confirm.is_some() {
            s.drive_menu = false;
            s.confirm = None;
            return true;
        }
        if let Screen::Category(_) = s.screen {
            s.go(Screen::Overview);
            return true;
        }
    }
    false
}

// --- Scenes: the fixed drive in every screen and state, for `--snap` and `--script`. ---

fn demo() -> App {
    let rules = silt_core::rules::builtin();
    let scan = fixture::scan(&rules);
    let journal = fixture::journal();
    let mut report = Report::build(&scan, &rules, &journal, fixture::bin(), fixture::NOW);
    fixture::estimate(&mut report);
    let mut s = App::new(fixture::drives());
    s.drives[0] = scan.volume.clone();
    s.load(Loaded::new(scan, report, rules, journal, fixture::NOW));
    s
}

fn first_shrink(s: &App) -> String {
    s.data
        .as_ref()
        .and_then(|d| d.report.shrink.first())
        .map(|r| r.path.rsplit('\\').next().unwrap_or("").to_string())
        .unwrap_or_default()
}

type Scene = (&'static str, fn(&mut App));

const SCENES: &[Scene] = &[
    ("overview", |_| {}),
    ("overview-scanning", |s| {
        let used = s.volume().map_or(0, |v| v.used());
        s.data = None;
        s.phase = Phase::Scanning {
            found: used * 46 / 100,
            entries: 412_330,
            used,
        };
    }),
    ("overview-preview-shrink", |s| {
        s.preview = Some(Reclaim::Shrink);
        s.preview_last = Some(Reclaim::Shrink);
        s.preview_t = 1.0;
    }),
    ("overview-preview-clean", |s| {
        s.preview = Some(Reclaim::Clean);
        s.preview_last = Some(Reclaim::Clean);
        s.preview_t = 1.0;
    }),
    ("overview-no-admin", |s| {
        s.fallback = Some(Fallback::NotAdmin);
        if let Some(d) = s.data.as_mut() {
            d.report.scanner = silt_core::scan::ScannerKind::Walk;
            d.report.elapsed_ms = 14_200;
        }
    }),
    ("overview-error", |s| {
        s.data = None;
        s.phase = Phase::Failed {
            drive: "C:".into(),
            message: "Access is denied. (os error 5)".into(),
        };
    }),
    ("overview-confirm-bin", |s| {
        s.confirm = Some(Confirm::EmptyBin)
    }),
    ("drive-menu", |s| s.drive_menu = true),
    ("clean", |s| s.go(Screen::Clean)),
    ("clean-confirm", |s| {
        s.go(Screen::Clean);
        s.confirm = Some(Confirm::Clean);
    }),
    ("clean-running", |s| {
        s.go(Screen::Clean);
        s.job = Some(Job {
            kind: JobKind::Clean,
            done: 2,
            total: 5,
            current: "NVIDIA shader cache".into(),
        });
    }),
    ("clean-done", |s| {
        s.go(Screen::Clean);
        s.outcome = Some(Outcome::Done {
            kind: JobKind::Clean,
            count: 5,
            bytes: 9_100 << 20,
            skipped: 12,
        });
    }),
    ("clean-empty", |s| {
        s.go(Screen::Clean);
        if let Some(d) = s.data.as_mut() {
            d.report.clean.clear();
        }
    }),
    ("shrink", |s| s.go(Screen::Shrink)),
    ("shrink-running", |s| {
        s.go(Screen::Shrink);
        let what = first_shrink(s);
        s.job = Some(Job {
            kind: JobKind::Shrink,
            done: 840,
            total: 3_120,
            current: what,
        });
    }),
    ("shrink-done", |s| {
        s.go(Screen::Shrink);
        s.outcome = Some(Outcome::Done {
            kind: JobKind::Shrink,
            count: 3,
            bytes: 41_300 << 20,
            skipped: 0,
        });
    }),
    ("shrink-stopped", |s| {
        s.go(Screen::Shrink);
        s.outcome = Some(Outcome::Stopped {
            kind: JobKind::Shrink,
            done: 2,
            total: 5,
            bytes: 27_400 << 20,
        });
    }),
    ("large", |s| s.go(Screen::Large)),
    ("large-selected", |s| {
        s.go(Screen::Large);
        let pick: Vec<_> = s.large.ids.iter().take(3).copied().collect();
        s.large.sel.extend(pick);
    }),
    ("large-folders", |s| {
        s.go(Screen::Large);
        s.large.folders = true;
        s.refresh_large();
    }),
    ("large-confirm", |s| {
        s.go(Screen::Large);
        let pick: Vec<_> = s.large.ids.iter().take(3).copied().collect();
        s.large.sel.extend(pick);
        s.confirm = Some(Confirm::Trash);
    }),
    ("large-empty", |s| {
        s.go(Screen::Large);
        s.large.min = 3;
        s.large.age = 3;
        s.refresh_large();
    }),
    ("category", |s| s.go(Screen::Category(Category::Developer))),
    ("category-videos", |s| {
        s.go(Screen::Category(Category::Videos))
    }),
    ("explore", |s| s.go(Screen::Explore)),
    ("explore-folder", |s| {
        s.go(Screen::Explore);
        if let Some(id) = s
            .data
            .as_ref()
            .and_then(|d| d.scan.tree.find(r"C:\Users\ana"))
        {
            s.explore.folder = id;
        }
    }),
    ("explore-map", |s| {
        s.go(Screen::Explore);
        s.explore.map = true;
    }),
    ("history", |s| s.go(Screen::History)),
    ("history-empty", |s| {
        s.go(Screen::History);
        if let Some(d) = s.data.as_mut() {
            d.journal.clear();
        }
    }),
    ("settings", |s| s.go(Screen::Settings)),
    ("pt-br", |s| s.settings.lang = i18n::Lang::Portuguese),
    ("pt-br-shrink", |s| {
        s.settings.lang = i18n::Lang::Portuguese;
        s.go(Screen::Shrink);
    }),
    // Two million files on the made-up drive: memory and frame times with a big tree.
    ("big", |s| {
        let t = std::time::Instant::now();
        let rules = silt_core::rules::builtin();
        let scan = fixture::scan_with(&rules, 2_000_000);
        let built = t.elapsed();
        let mut report = Report::build(&scan, &rules, &[], fixture::bin(), fixture::NOW);
        fixture::estimate(&mut report);
        let heap = scan.tree.heap_bytes();
        let nodes = scan.tree.nodes.len();
        s.drives[0] = scan.volume.clone();
        s.load(Loaded::new(scan, report, rules, Vec::new(), fixture::NOW));
        eprintln!(
            "big: {nodes} nodes, tree {} MB, built in {:.2} s, report and lists {:.2} s",
            heap >> 20,
            built.as_secs_f64(),
            (t.elapsed() - built).as_secs_f64()
        );
    }),
    ("big-category", |s| {
        (SCENES.iter().find(|x| x.0 == "big").unwrap().1)(s);
        s.go(Screen::Category(Category::Developer));
    }),
    ("big-large", |s| {
        (SCENES.iter().find(|x| x.0 == "big").unwrap().1)(s);
        s.go(Screen::Large);
    }),
    // The real system drive through the window's own threads: checks the wiring, not a look.
    ("live-scan", |s| {
        *s = App::real(live::known_dirs(), app::Settings::default(), false);
        settle(s);
    }),
];

/// Pumps the threads' messages until the scan, the estimate and any action are done.
fn settle(s: &mut App) {
    let start = std::time::Instant::now();
    while (s.working() || s.live.as_ref().is_some_and(|l| l.estimating.is_some()))
        && start.elapsed() < std::time::Duration::from_secs(300)
    {
        s.pump();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    s.pump();
}

/// What `main` learned before the window opened: whose folders, the saved settings, and whether
/// the UAC prompt was declined.
static BOOT: std::sync::Mutex<Option<(KnownDirs, app::Settings, bool)>> =
    std::sync::Mutex::new(None);

/// The real drives in a window; the fixed drive for `--snap`, `--script`, `--tree` and the like,
/// so every scene stays the same.
fn init() -> App {
    let boot = BOOT.lock().unwrap_or_else(|e| e.into_inner()).take();
    match boot {
        Some((known, settings, declined)) if !rune::is_headless() => {
            App::real(known, settings, declined)
        }
        _ => demo(),
    }
}

fn main() {
    if !rune::is_headless() {
        let known = live::known_dirs();
        let settings = live::load_settings(&known);
        let mut declined = false;
        if settings.ask_admin && !live::relaunched() && !silt_core::win::is_elevated() {
            match live::elevate(&known) {
                Ok(()) => return,
                Err(_) => declined = true,
            }
        }
        *BOOT.lock().unwrap_or_else(|e| e.into_inner()) = Some((known, settings, declined));
    }
    let spec = AppSpec {
        min_size: Some((880.0, 600.0)),
        scenes: SCENES,
        fonts: style::FONTS,
        families: style::FAMILIES,
        poll: Some(poll),
        on_key: Some(on_key),
        setup: Some(|s: &mut App, w| s.set_waker(w)),
        ..AppSpec::new("Silt", (1120.0, 760.0), init, view)
    };
    rune::App::new(spec).titlebar(Titlebar::new(TITLEBAR)).run();
}
