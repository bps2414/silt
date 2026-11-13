//! The drive as a sediment core: how full it is, what fills it, and the four ways back to space.

use rune::{Align, Color, Cx, El, col, el, row};

use silt_core::largest::Filter;
use silt_core::scan::Fallback;

use crate::app::{App, Confirm, Phase, Reclaim, Screen};
use crate::bar::{Geom, Ink, Look, Seg};
use crate::i18n::{self, t, tp};
use crate::style::{self, BAR_ORDER, Pal};
use crate::ui;

pub fn view(s: &App, p: Pal, cx: &mut Cx<App>) -> El {
    let scale = cx.env().scale;
    let width = cx.layout_size("bar-slot").map_or(1064.0, |[w, _]| w);
    let g = Geom::fit(width, 18, scale);
    let (bw, bh) = g.points(scale);
    let cap = s.volume().map_or(1, |v| v.total);
    let drive = s.volume().map(|v| v.drive()).unwrap_or_default();
    let ink = Ink {
        free: p.free8,
        ink2: p.ink2_8,
    };

    let mut page = col()
        .key("overview")
        .w_full()
        .px(style::PAD)
        .pt(26.0)
        .pb(28.0);

    if s.fallback.is_some() && !s.fallback_hidden {
        let body = match s.fallback {
            Some(Fallback::NotNtfs) => t!("slow-not-ntfs"),
            _ => t!("slow-not-admin"),
        };
        let mut actions = Vec::new();
        if matches!(s.fallback, Some(Fallback::NotAdmin)) {
            actions.push(ui::button(
                p,
                "elevate",
                t!("restart-admin"),
                ui::Kind::Primary,
                Some(|_: &mut App| {}),
            ));
        }
        actions.push(ui::button(
            p,
            "dismiss",
            t!("dismiss"),
            ui::Kind::Quiet,
            Some(|s: &mut App| s.fallback_hidden = true),
        ));
        page = page.child(ui::notice(p, "slow", p.flour, t!("slow-title"), body, actions).mb(22.0));
    }

    // The figure: what is used, set wide; its context set narrow.
    let (big, context, side) = match (&s.phase, &s.data) {
        (Phase::Ready, Some(d)) => {
            let r = &d.report;
            let secs = r.elapsed_ms as f64 / 1000.0;
            let time = if secs < 10.0 {
                format!("{secs:.1}")
            } else {
                format!("{secs:.0}")
            };
            let time = if I18N_COMMA() {
                time.replace('.', ",")
            } else {
                time
            };
            (
                i18n::size(r.used),
                t!("used-of", total = i18n::size(cap)),
                col()
                    .items_end()
                    .gap(2.0)
                    .child(ui::txt(
                        t!("scanned", files = i18n::int(r.files), time = time),
                        style::BODY,
                        p.ink2,
                    ))
                    .child(ui::txt(
                        t!("free", size = i18n::size(cap.saturating_sub(r.used))),
                        style::BODY,
                        p.ink2,
                    )),
            )
        }
        (
            Phase::Scanning {
                found,
                entries,
                used,
            },
            _,
        ) => (
            i18n::size(*found),
            t!("found-of", used = i18n::size(*used)),
            row()
                .gap(14.0)
                .center_y()
                .child(
                    col()
                        .items_end()
                        .gap(2.0)
                        .child(ui::txt(
                            t!("reading", drive = drive.clone()),
                            style::LABEL,
                            p.ink,
                        ))
                        .child(ui::txt(tp!("files-so-far", *entries), style::BODY, p.ink2)),
                )
                .child(ui::button(
                    p,
                    "stop-scan",
                    t!("stop"),
                    ui::Kind::Quiet,
                    Some(|_: &mut App| {}),
                )),
        ),
        (Phase::Failed { drive, .. }, _) => (drive.clone(), t!("unreadable").to_string(), col()),
        _ => (String::new(), String::new(), col()),
    };
    page = page.child(
        row()
            .key("figure")
            .w_full()
            .items_end()
            .gap(14.0)
            .child(ui::txt(big, style::DISPLAY, p.ink))
            .child(ui::txt(context, style::DISPLAY_CONTEXT, p.ink2).pb(8.0))
            .child(el().grow(1.0))
            .child(side.pb(6.0)),
    );

    if let Phase::Failed { drive, message } = &s.phase {
        return page.child(
            ui::notice(
                p,
                "failed",
                p.danger,
                &t!("failed-title", drive = drive.clone()),
                &t!("failed-body", message = message.clone()),
                vec![ui::button(
                    p,
                    "retry",
                    t!("try-again"),
                    ui::Kind::Primary,
                    Some(|_: &mut App| {}),
                )],
            )
            .mt(22.0),
        );
    }

    // The core.
    let (segs, look) = match (&s.phase, &s.data) {
        (Phase::Ready, Some(d)) => (
            d.segs(if s.preview_t > 0.0 {
                s.preview_last
            } else {
                None
            }),
            Look::Sorted {
                color: s.color_t,
                fade: s.preview_t,
            },
        ),
        (Phase::Scanning { found, used, .. }, _) => (
            vec![Seg {
                cat: None,
                solid: *used,
                soft: 0,
                fading: 0,
            }],
            Look::Settling {
                found: *found as f32 / cap as f32,
            },
        ),
        _ => (Vec::new(), Look::Settling { found: 0.0 }),
    };
    s.bar.draw(g, cap, &segs, look, &ink);
    let ready = s.data.is_some() && matches!(s.phase, Phase::Ready);
    let mut core = el()
        .key("bar")
        .role("img")
        .label(t!("bar-label"))
        .size(bw, bh)
        .pixels(&s.bar.pixels);
    if ready {
        core = core
            .on_pointer_move_local(move |s: &mut App, e, _| {
                let c = s.bar.category_at(e.x, g, scale);
                if s.bar_hover != c {
                    s.bar_hover = c;
                }
            })
            .on_hover_change(|s: &mut App, inside, _| {
                if !inside {
                    s.bar_hover = None;
                }
            })
            .on_press(|s: &mut App| {
                if let Some(c) = s.bar_hover {
                    s.go(Screen::Category(c));
                }
            })
            .cursor(if s.bar_hover.is_some() {
                rune::Cursor::Pointer
            } else {
                rune::Cursor::Default
            });
    }
    page = page.child(row().key("bar-slot").w_full().h(bh).mt(18.0).child(core));

    let Some(d) = s.data.as_ref().filter(|_| ready) else {
        return page.child(ui::txt(t!("scanning-later"), style::BODY, p.ink2).mt(18.0));
    };
    let r = &d.report;

    // The legend names every segment and its size.
    let mut legend = row()
        .key("legend")
        .w_full()
        .flex_wrap()
        .gap(4.0)
        .mt(14.0)
        .ml(-6.0);
    for c in BAR_ORDER {
        let bytes = r.categories[c.index()].bytes;
        if bytes == 0 {
            continue;
        }
        let on = s.bar_hover == Some(c);
        legend = legend.child(
            row()
                .key(c.id())
                .role("link")
                .label(i18n::category(c))
                .h(28.0)
                .px(6.0)
                .gap(7.0)
                .center_y()
                .radius(5.0)
                .fill(if on { p.hover } else { Color::TRANSPARENT })
                .focusable(true)
                .hover(p.hover)
                .press(p.press)
                .on_press(move |s: &mut App| s.go(Screen::Category(c)))
                .child(ui::swatch(c, 10.0))
                .child(ui::txt(i18n::category(c), style::LABEL, p.ink).a11y_hidden())
                .child(ui::txt(i18n::size(bytes), style::FIGURE, p.ink2).a11y_hidden()),
        );
    }
    legend = legend.child(
        row()
            .key("free")
            .h(28.0)
            .px(6.0)
            .gap(7.0)
            .center_y()
            .child(
                el().size(10.0, 10.0)
                    .radius(1.5)
                    .fill(p.free)
                    .border(p.hair, p.line),
            )
            .child(ui::txt(t!("free-label"), style::LABEL, p.ink))
            .child(ui::txt(
                i18n::size(cap.saturating_sub(r.used)),
                style::FIGURE,
                p.ink2,
            )),
    );
    page = page.child(legend);

    // Reclaim: four ways back, least destructive first.
    let (lo, hi) = r.shrink_range();
    let shrink_n = r.shrink.iter().filter(|x| x.estimate.is_some()).count();
    let mut list = col()
        .key("reclaim")
        .w_full()
        .mt(34.0)
        .child(
            row()
                .w_full()
                .items_end()
                .pb(10.0)
                .child(
                    col()
                        .grow(1.0)
                        .gap(2.0)
                        .child(ui::txt(t!("reclaim-title"), style::TITLE, p.ink))
                        .child(ui::txt(
                            t!("reclaim-sub", size = i18n::size(r.without_deleting())),
                            style::BODY,
                            p.ink2,
                        )),
                )
                .child(
                    ui::txt(i18n::size(r.recoverable()), style::HEADLINE, p.ink).align(Align::End),
                ),
        )
        .child(ui::hairline(p));

    list = list.child(reclaim_row(
        s,
        p,
        Reclaim::Shrink,
        t!("nav-shrink"),
        &tp!("reclaim-shrink", shrink_n),
        i18n::range(lo, hi),
        ui::button(
            p,
            "review-shrink",
            t!("review"),
            ui::Kind::Quiet,
            Some(|s: &mut App| s.go(Screen::Shrink)),
        ),
    ));
    list = list.child(reclaim_row(
        s,
        p,
        Reclaim::Clean,
        t!("nav-clean"),
        &tp!("reclaim-clean", r.clean.len()),
        i18n::size(r.clean_total()),
        ui::button(
            p,
            "review-clean",
            t!("review"),
            ui::Kind::Quiet,
            Some(|s: &mut App| s.go(Screen::Clean)),
        ),
    ));
    let lo_f = Filter::large_and_old();
    list = list.child(reclaim_row(
        s,
        p,
        Reclaim::Large,
        t!("nav-large"),
        &tp!(
            "reclaim-large",
            r.large_old.files,
            size = i18n::size(lo_f.min_size)
        ),
        i18n::size(r.large_old.bytes),
        ui::button(
            p,
            "review-large",
            t!("review"),
            ui::Kind::Quiet,
            Some(|s: &mut App| {
                s.large.folders = false;
                s.large.cats = Filter::large_and_old().categories;
                s.large.min = 0;
                s.large.age = 1;
                s.refresh_large();
                s.go(Screen::Large);
            }),
        ),
    ));
    let bin_btn = if r.bin.bytes > 0 {
        ui::button(
            p,
            "empty-bin",
            t!("empty"),
            ui::Kind::Quiet,
            Some(|s: &mut App| s.confirm = Some(Confirm::EmptyBin)),
        )
    } else {
        el().w(1.0)
    };
    let bin_desc = if r.bin.items > 0 {
        tp!("reclaim-bin", r.bin.items)
    } else {
        t!("bin-empty").to_string()
    };
    list = list.child(reclaim_row(
        s,
        p,
        Reclaim::Bin,
        t!("empty-bin"),
        &bin_desc,
        i18n::size(r.bin.bytes),
        bin_btn,
    ));
    if s.confirm == Some(Confirm::EmptyBin) {
        list = list.child(
            row()
                .key("confirm-bin")
                .w_full()
                .gap(10.0)
                .py(14.0)
                .center_y()
                .child(
                    ui::txt(
                        tp!("confirm-bin", r.bin.items, size = i18n::size(r.bin.bytes)),
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
                    t!("empty-bin"),
                    ui::Kind::Danger,
                    Some(|s: &mut App| s.confirm = None),
                )),
        );
    }
    page.child(list)
}

#[allow(non_snake_case)]
fn I18N_COMMA() -> bool {
    i18n::size(1536).contains(',')
}

fn reclaim_row(
    s: &App,
    p: Pal,
    kind: Reclaim,
    name: &str,
    desc: &str,
    figure: String,
    action: El,
) -> El {
    let on = s.preview == Some(kind);
    let key = match kind {
        Reclaim::Shrink => "r-shrink",
        Reclaim::Clean => "r-clean",
        Reclaim::Large => "r-large",
        Reclaim::Bin => "r-bin",
    };
    col()
        .key(key)
        .w_full()
        .child(
            row()
                .w_full()
                .h(66.0)
                .gap(20.0)
                .center_y()
                .px(10.0)
                .ml(-10.0)
                .mr(-10.0)
                .radius(8.0)
                .fill(if on { p.hover } else { Color::TRANSPARENT })
                .on_hover_change(move |s: &mut App, inside, _| {
                    if inside {
                        s.preview = Some(kind);
                        s.preview_last = Some(kind);
                    } else if s.preview == Some(kind) {
                        s.preview = None;
                    }
                })
                .child(
                    col()
                        .grow(1.0)
                        .gap(2.0)
                        .child(ui::txt(name, style::LABEL, p.ink))
                        .child(ui::txt(desc, style::BODY, p.ink2).truncate()),
                )
                .child(ui::figure(figure, p.ink, 170.0))
                .child(row().w(96.0).justify_end().child(action)),
        )
        .child(ui::hairline(p))
}
