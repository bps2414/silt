//! Shrink: apps, games and developer folders, what Windows' compression would save on each, the
//! method, and a way back.

use rune::{Align, Cx, El, col, el, row};
use silt_core::shrink::Algorithm;

use crate::app::{App, JobKind, Outcome};
use crate::i18n::{self, t, tp};
use crate::screens::clean::{action_row, tag};
use crate::style::{self, Pal};
use crate::{screens, ui};

pub fn algo_name(a: Algorithm) -> &'static str {
    match a {
        Algorithm::Xpress4k => t!("algo-fast"),
        Algorithm::Xpress8k => t!("algo-balanced"),
        Algorithm::Xpress16k => t!("algo-smaller"),
        Algorithm::Lzx => t!("algo-smallest"),
    }
}

pub fn algo_note(a: Algorithm) -> &'static str {
    match a {
        Algorithm::Xpress4k => t!("algo-fast-note"),
        Algorithm::Xpress8k => t!("algo-balanced-note"),
        Algorithm::Xpress16k => t!("algo-smaller-note"),
        Algorithm::Lzx => t!("algo-smallest-note"),
    }
}

pub fn algo_picker(p: Pal, key: &str, current: Algorithm) -> El {
    ui::segmented(
        p,
        key,
        Algorithm::ALL
            .into_iter()
            .map(|a| {
                (
                    algo_name(a).to_string(),
                    a == current,
                    Box::new(move |s: &mut App| s.settings.algo = a) as Box<dyn Fn(&mut App)>,
                )
            })
            .collect(),
    )
}

pub fn view(s: &App, p: Pal, cx: &mut Cx<App>) -> El {
    let d = match screens::loaded(s, p) {
        Ok(d) => d,
        Err(e) => return e,
    };
    let r = &d.report;
    let drive = s.volume().map(|v| v.drive()).unwrap_or_default();
    let rows: Vec<_> = r.shrink.iter().filter(|x| x.estimate.is_some()).collect();
    let chosen: Vec<_> = rows
        .iter()
        .filter(|x| s.shrink_sel.contains(&x.path))
        .collect();
    let (lo, hi) = chosen
        .iter()
        .filter_map(|x| x.estimate.as_ref())
        .fold((0, 0), |(l, h), e| {
            let (a, b) = e.saving();
            (l + a, h + b)
        });

    // After a run, the saved figure is set narrow: the type does what the files did.
    let figure = match &s.outcome {
        Some(Outcome::Done {
            kind: JobKind::Shrink,
            bytes,
            ..
        }) => Some((i18n::size(*bytes), t!("shrink-saved").to_string(), true)),
        _ if !rows.is_empty() => Some((
            i18n::range(lo, hi),
            tp!("shrink-chosen", chosen.len()),
            false,
        )),
        _ => None,
    };
    let mut header = row().key("header").w_full().gap(24.0).items_end().child(
        col()
            .grow(1.0)
            .gap(4.0)
            .child(ui::txt(t!("nav-shrink"), style::TITLE, p.ink))
            .child(
                ui::txt(t!("shrink-sub"), style::BODY, p.ink2)
                    .wrap()
                    .max_w(620.0),
            ),
    );
    if let Some((big, small, narrow)) = figure {
        header = header.child(
            col()
                .items_end()
                .gap(2.0)
                .child(
                    ui::txt(
                        big,
                        if narrow {
                            style::HEADLINE_NARROW
                        } else {
                            style::HEADLINE
                        },
                        if narrow { p.flour } else { p.ink },
                    )
                    .align(Align::End),
                )
                .child(ui::txt(small, style::BODY, p.ink2).align(Align::End)),
        );
    }

    if rows.is_empty() {
        let body = ui::empty(
            p,
            "empty",
            &t!("shrink-empty-title", drive = drive),
            t!("shrink-empty-body"),
        );
        return crate::page(p, "shrink", header, None, body, None);
    }

    let busy = s.job.is_some();
    let widest = rows.iter().map(|x| x.alloc).max().unwrap_or(1).max(1);
    let mut list = col().key("folders").w_full().child(
        row()
            .key("cols")
            .w_full()
            .gap(14.0)
            .pb(8.0)
            .child(el().w(16.0))
            .child(ui::txt(t!("col-folder"), style::LABEL, p.ink2).grow(1.0))
            .child(
                ui::txt(t!("col-size"), style::LABEL, p.ink2)
                    .w(96.0)
                    .align(Align::End),
            )
            .child(
                ui::txt(t!("col-saves"), style::LABEL, p.ink2)
                    .w(150.0)
                    .align(Align::End),
            ),
    );
    list = list.child(ui::hairline(p));
    for x in &rows {
        let on = s.shrink_sel.contains(&x.path);
        let path = x.path.clone();
        let e = x.estimate.as_ref().expect("estimated");
        let (a, b) = e.saving();
        let mut tags = row().gap(6.0).center_y();
        if x.shrunk_before {
            tags = tags.child(tag(p, t!("tag-updated")));
        }
        if x.already > 0 && x.already * 4 > x.alloc {
            tags = tags.child(tag(p, t!("tag-partly")));
        }
        if x.directstorage {
            tags = tags.child(tag(p, t!("tag-directstorage")));
        }
        let mut line = row()
            .key(format!("f-{}", x.node))
            .role("checkbox")
            .label(screens::leaf(&x.path).to_string())
            .checked(on)
            .w_full()
            .h(58.0)
            .px(10.0)
            .ml(-10.0)
            .mr(-10.0)
            .gap(14.0)
            .center_y()
            .radius(8.0);
        if !busy {
            line =
                line.focusable(true)
                    .hover(p.hover)
                    .press(p.press)
                    .on_press(move |s: &mut App| {
                        if !s.shrink_sel.remove(&path) {
                            s.shrink_sel.insert(path.clone());
                        }
                    });
        }
        line = line
            .child(ui::check(p, on))
            .child(ui::swatch(x.category, 8.0))
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(2.0)
                    .child(
                        row()
                            .gap(10.0)
                            .center_y()
                            .child(ui::txt(screens::leaf(&x.path), style::LABEL, p.ink))
                            .child(tags),
                    )
                    .child(ui::txt(screens::parent(&x.path), style::PATH, p.ink2).truncate()),
            )
            .child(
                col()
                    .w(96.0)
                    .items_end()
                    .gap(5.0)
                    .child(ui::figure(i18n::size(x.alloc), p.ink2, 96.0))
                    .child(ui::meter(
                        p,
                        x.category,
                        x.alloc as f32 / widest as f32,
                        64.0,
                    )),
            )
            .child(ui::figure(i18n::range(a, b), p.ink, 150.0));
        list = list.child(line).child(ui::hairline(p));
    }

    let method = row()
        .key("method")
        .gap(12.0)
        .center_y()
        .child(ui::txt(t!("method"), style::LABEL, p.ink2))
        .child(algo_picker(p, "algo", s.settings.algo))
        .child(
            ui::txt(algo_note(s.settings.algo), style::BODY, p.ink2)
                .truncate()
                .grow(1.0),
        );
    let footer = match crate::job_line(s, p, &[JobKind::Shrink, JobKind::Restore]) {
        Some(line) if s.job.is_some() => line,
        Some(line) => {
            let done = matches!(
                s.outcome,
                Some(Outcome::Done {
                    kind: JobKind::Shrink,
                    ..
                })
            );
            let mut r = row().grow(1.0).gap(12.0).center_y().child(line);
            if done {
                r = r.child(ui::button(
                    p,
                    "restore",
                    t!("restore"),
                    ui::Kind::Quiet,
                    Some(|_: &mut App| {}),
                ));
            }
            r.child(ui::button(
                p,
                "go",
                &tp!("shrink-n", chosen.len()),
                ui::Kind::Primary,
                (!chosen.is_empty()).then_some((|_: &mut App| {}) as fn(&mut App)),
            ))
        }
        None => row()
            .grow(1.0)
            .gap(20.0)
            .center_y()
            .child(method.grow(1.0))
            .child(
                action_row(
                    p,
                    &tp!("shrink-n", chosen.len()),
                    !chosen.is_empty(),
                    |_| {},
                )
                .grow(0.0),
            ),
    };
    crate::page(
        p,
        "shrink",
        header,
        Some(crate::strip(s, p, cx)),
        list,
        Some(footer),
    )
}
