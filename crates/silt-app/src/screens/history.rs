//! History: what Silt has changed on this PC, newest first, with Restore on shrunk folders.

use rune::{Cx, El, col, el, row};
use silt_core::Category;
use silt_core::journal::{Action, Entry};
use silt_core::shrink::Algorithm;

use crate::app::{App, JobKind, Loaded};
use crate::i18n::{self, t, tp};
use crate::screens::shrink::algo_name;
use crate::style::{self, Pal};
use crate::{screens, ui};

pub fn view(s: &App, p: Pal, _cx: &mut Cx<App>) -> El {
    let d = match screens::loaded(s, p) {
        Ok(d) => d,
        Err(e) => return e,
    };
    let saved: i64 = d
        .journal
        .iter()
        .filter(|e| e.action != Action::Trash)
        .map(|e| e.saved().max(0))
        .sum();
    let figure = d.journal.first().map(|first| {
        (
            i18n::size(saved as u64),
            t!("history-since", date = i18n::date(first.ts)),
        )
    });
    let header = crate::header(p, t!("nav-history"), t!("history-sub"), figure);
    if d.journal.is_empty() {
        let body = ui::empty(
            p,
            "empty",
            t!("history-empty-title"),
            t!("history-empty-body"),
        );
        return crate::page(p, "history", header, None, body, None);
    }

    let mut list = col().key("rows").w_full();
    let mut last_day = u64::MAX;
    for i in (0..d.journal.len()).rev() {
        let e = &d.journal[i];
        let day = e.ts / 86_400;
        if day != last_day {
            last_day = day;
            list = list.child(
                ui::txt(i18n::date(e.ts), style::LABEL, p.ink2)
                    .key(format!("day{day}"))
                    .pt(if i + 1 == d.journal.len() { 4.0 } else { 22.0 })
                    .pb(6.0),
            );
        }
        list = list.child(ui::hairline(p)).child(entry(s, d, p, i, e));
    }
    let footer = crate::job_line(s, p, &[JobKind::Restore]);
    crate::page(p, "history", header, None, list, footer)
}

fn entry(s: &App, d: &Loaded, p: Pal, i: usize, e: &Entry) -> El {
    let leaf = screens::leaf(&e.target).to_string();
    let (line, cat) = match e.action {
        Action::Shrink => (t!("did-shrink", what = leaf), Some(Category::Apps)),
        Action::Restore => (t!("did-restore", what = leaf), None),
        Action::Clean => {
            let name = d
                .rule(&e.target)
                .map_or(e.target.clone(), |r| r.name.clone());
            (t!("did-clean", what = name), Some(Category::Caches))
        }
        Action::Trash => (tp!("did-trash", e.files), Some(Category::Trash)),
        Action::EmptyBin => (tp!("did-empty", e.files), Some(Category::Trash)),
    };
    let detail = match e.action {
        Action::Shrink | Action::Restore => {
            let mut parts = vec![screens::parent(&e.target).to_string()];
            if let Some(a) = e.algorithm.as_deref().and_then(Algorithm::from_id) {
                parts.push(algo_name(a).to_string());
            }
            parts.push(tp!("files", e.files));
            parts.join(", ")
        }
        _ => tp!("files", e.files),
    };
    let detail = if e.skipped > 0 {
        format!("{detail}, {}", tp!("skipped-short", e.skipped))
    } else {
        detail
    };
    let saved = e.saved();
    let (amount, tone) = match e.action {
        Action::Shrink => (
            t!("history-saved", size = i18n::size(saved.max(0) as u64)),
            p.ink,
        ),
        Action::Restore => (
            t!("history-gave-back", size = i18n::size(saved.unsigned_abs())),
            p.ink2,
        ),
        Action::Trash => (t!("history-moved", size = i18n::size(e.before)), p.ink2),
        _ => (t!("history-freed", size = i18n::size(e.before)), p.ink),
    };
    let mark = match cat {
        Some(c) => ui::swatch(c, 8.0),
        None => el()
            .size(8.0, 8.0)
            .radius(1.5)
            .border(1.2, p.ink2)
            .a11y_hidden(),
    };
    let restorable = d.restorable(i);
    let busy = s.job.is_some();
    let action = if restorable {
        let target = e.target.clone();
        restore_button(p, i, busy, target)
    } else if e.action == Action::Shrink {
        ui::txt(t!("restored"), style::BODY, p.ink2).w(96.0)
    } else {
        el().w(96.0)
    };
    row()
        .key(format!("e{i}"))
        .w_full()
        .min_h(58.0)
        .py(10.0)
        .gap(14.0)
        .center_y()
        .child(ui::txt(time_of(e.ts), style::FIGURE, p.ink2).w(44.0))
        .child(mark)
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(3.0)
                .child(ui::txt(line, style::LABEL, p.ink).truncate())
                .child(ui::txt(detail, style::PATH, p.ink2).truncate()),
        )
        .child(
            ui::txt(amount, style::FIGURE, tone)
                .w(150.0)
                .align(rune::Align::End),
        )
        .child(action)
}

fn restore_button(p: Pal, i: usize, busy: bool, target: String) -> El {
    let label = t!("restore");
    let mut b = row()
        .key(format!("restore{i}"))
        .role("button")
        .label(label)
        .w(96.0)
        .h(30.0)
        .justify_center()
        .center_y()
        .radius(6.0)
        .border(p.hair.max(1.0), p.line)
        .child(ui::txt(label, style::LABEL, p.ink).a11y_hidden());
    if busy {
        b = b.opacity(0.4);
    } else {
        b = b
            .focusable(true)
            .hover(p.hover)
            .press(p.press)
            .on_press(move |s: &mut App| {
                s.outcome = None;
                s.job = Some(crate::app::Job {
                    kind: JobKind::Restore,
                    done: 0,
                    total: 1,
                    current: screens::leaf(&target).to_string(),
                });
            });
    }
    b
}

/// `14:00`, the entry's time of day (UTC until the core knows the time zone).
fn time_of(ts: u64) -> String {
    let m = ts % 86_400 / 60;
    format!("{:02}:{:02}", m / 60, m % 60)
}
