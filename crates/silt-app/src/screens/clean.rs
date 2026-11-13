//! Clean: caches that rebuild themselves, by group, each with what it is and what cleaning costs.

use rune::{Color, Cx, El, col, el, row};
use silt_core::rules::Safety;

use crate::app::{App, Confirm, JobKind};
use crate::i18n::{self, I18N, t, tp};
use crate::style::{self, Pal};
use crate::{screens, ui};

const GROUPS: [&str; 4] = ["system", "browsers", "gpu", "developer"];

pub fn view(s: &App, p: Pal, cx: &mut Cx<App>) -> El {
    let d = match screens::loaded(s, p) {
        Ok(d) => d,
        Err(e) => return e,
    };
    let r = &d.report;
    let drive = s.volume().map(|v| v.drive()).unwrap_or_default();
    let chosen: Vec<_> = r
        .clean
        .iter()
        .filter(|c| s.clean_sel.contains(&c.rule))
        .collect();
    let total: u64 = chosen.iter().map(|c| c.bytes).sum();
    let header = crate::header(
        p,
        t!("nav-clean"),
        t!("clean-sub"),
        (!r.clean.is_empty()).then(|| {
            (
                i18n::size(total),
                tp!(
                    "clean-chosen",
                    chosen.len(),
                    of = i18n::int(r.clean.len() as u64)
                ),
            )
        }),
    );
    if r.clean.is_empty() {
        let body = ui::empty(
            p,
            "empty",
            &t!("clean-empty-title", drive = drive),
            t!("clean-empty-body"),
        );
        return crate::page(
            p,
            "clean",
            header,
            None,
            body,
            crate::job_line(s, p, &[JobKind::Clean]),
        );
    }

    let lang = I18N.info().id();
    let busy = s.job.is_some();
    let mut list = col().key("rules").w_full();
    for g in GROUPS {
        let rows: Vec<_> = r.clean.iter().filter(|c| c.group == g).collect();
        if rows.is_empty() {
            continue;
        }
        let name = match g {
            "system" => t!("group-system"),
            "browsers" => t!("group-browsers"),
            "gpu" => t!("group-gpu"),
            _ => t!("group-developer"),
        };
        list = list.child(
            ui::txt(name, style::LABEL, p.ink2)
                .key(format!("g-{g}"))
                .pt(18.0)
                .pb(6.0),
        );
        for c in rows {
            let on = s.clean_sel.contains(&c.rule);
            let id = c.rule.clone();
            let note = d
                .rule(&c.rule)
                .map(|rule| rule.note_for(lang))
                .unwrap_or("");
            let where_ = match c.folders.len() {
                0 => String::new(),
                1 => c.folders[0].clone(),
                n => t!(
                    "path-and-more",
                    path = c.folders[0].clone(),
                    count = i18n::int(n as u64 - 1)
                ),
            };
            let mut line = row()
                .key(format!("r-{}", c.rule))
                .role("checkbox")
                .label(c.name.clone())
                .checked(on)
                .w_full()
                .py(12.0)
                .px(10.0)
                .ml(-10.0)
                .mr(-10.0)
                .gap(14.0)
                .items_start()
                .radius(8.0);
            if !busy {
                line = line.focusable(true).hover(p.hover).press(p.press).on_press(
                    move |s: &mut App| {
                        if !s.clean_sel.remove(&id) {
                            s.clean_sel.insert(id.clone());
                        }
                        s.confirm = None;
                    },
                );
            }
            line = line
                .child(ui::check(p, on).mt(2.0))
                .child(
                    col()
                        .grow(1.0)
                        .gap(3.0)
                        .child(
                            row()
                                .gap(10.0)
                                .center_y()
                                .child(ui::txt(c.name.clone(), style::LABEL, p.ink))
                                .child(if c.safety == Safety::Caution {
                                    tag(p, t!("safety-caution"))
                                } else {
                                    el()
                                }),
                        )
                        .child(ui::txt(note, style::BODY, p.ink2).wrap().max_w(660.0))
                        .child(ui::txt(where_, style::PATH, p.ink2).truncate().max_w(660.0)),
                )
                .child(ui::figure(i18n::size(c.bytes), p.ink, 96.0));
            list = list.child(line).child(ui::hairline(p));
        }
    }

    let footer = crate::job_line(s, p, &[JobKind::Clean]).map_or_else(
        || {
            if s.confirm == Some(Confirm::Clean) {
                row()
                    .key("confirm")
                    .grow(1.0)
                    .gap(10.0)
                    .center_y()
                    .child(
                        ui::txt(
                            tp!("confirm-clean", chosen.len(), size = i18n::size(total)),
                            style::BODY,
                            p.ink,
                        )
                        .wrap()
                        .grow(1.0),
                    )
                    .child(ui::button(
                        p,
                        "cancel",
                        t!("cancel"),
                        ui::Kind::Quiet,
                        Some(|s: &mut App| s.confirm = None),
                    ))
                    .child(ui::button(
                        p,
                        "confirm",
                        &tp!("clean-n", chosen.len()),
                        ui::Kind::Danger,
                        Some(|s: &mut App| s.clean_now()),
                    ))
            } else {
                action_row(
                    p,
                    &tp!("clean-n", chosen.len()),
                    !chosen.is_empty(),
                    |s: &mut App| s.confirm = Some(Confirm::Clean),
                )
            }
        },
        |line| {
            if s.job.is_some() {
                line
            } else {
                row()
                    .grow(1.0)
                    .gap(16.0)
                    .center_y()
                    .child(line)
                    .child(ui::button(
                        p,
                        "go",
                        &tp!("clean-n", chosen.len()),
                        ui::Kind::Primary,
                        (!chosen.is_empty()).then_some(
                            (|s: &mut App| s.confirm = Some(Confirm::Clean)) as fn(&mut App),
                        ),
                    ))
            }
        },
    );
    crate::page(
        p,
        "clean",
        header,
        Some(crate::strip(s, p, cx)),
        list,
        Some(footer),
    )
}

/// A footer with its action button at the trailing edge.
pub fn action_row(p: Pal, label: &str, enabled: bool, f: fn(&mut App)) -> El {
    row()
        .key("actions")
        .grow(1.0)
        .gap(12.0)
        .center_y()
        .child(el().grow(1.0))
        .child(ui::button(
            p,
            "go",
            label,
            ui::Kind::Primary,
            enabled.then_some(f),
        ))
}

/// A small outlined word beside a name.
pub fn tag(p: Pal, word: &str) -> El {
    row()
        .h(20.0)
        .px(7.0)
        .center_y()
        .radius(4.0)
        .border(p.hair.max(1.0), p.line)
        .fill(Color::TRANSPARENT)
        .child(ui::txt(word, style::PATH, p.ink2))
}
