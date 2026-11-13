//! Large files (the biggest files and folders, filtered) and a category's own screen. Both end
//! in the Recycle Bin, never in a permanent delete.

use std::borrow::Cow;

use rune::{Align, Color, Cx, El, GrowthSpec, col, el, row};
use silt_core::tree::flags;
use silt_core::{Category, NodeId, Tree};

use crate::app::{AGES, App, Confirm, JobKind, SIZES, Screen};
use crate::i18n::{self, t, tp};
use crate::screens::clean::action_row;
use crate::style::{self, Pal};
use crate::{screens, ui};

const ROW_H: f32 = 52.0;

/// Folders where `c` holds most of the bytes (no folder that is mostly one subfolder), or the
/// files of `c`, biggest first.
pub fn category_items(tree: &Tree, dominant: &[Category], c: Category) -> Vec<NodeId> {
    let folders = matches!(
        c,
        Category::Apps
            | Category::Games
            | Category::Developer
            | Category::Caches
            | Category::System
    );
    let mut v: Vec<NodeId> = tree
        .descendants(tree.root)
        .filter(|&id| {
            let n = tree.node(id);
            if id == tree.root || n.has(flags::LINK) || n.alloc == 0 {
                return false;
            }
            if folders {
                if !n.is_dir() && !n.has(flags::SYNTHETIC) {
                    return n.category == c && tree.node(n.parent).category != c;
                }
                if n.has(flags::SYNTHETIC) {
                    return n.category == c;
                }
                dominant[id as usize] == c
                    && tree.node(n.parent).alloc > 0
                    && dominant[n.parent as usize] != c
            } else {
                !n.is_dir() && n.category == c
            }
        })
        .collect();
    v.sort_unstable_by_key(|&id| std::cmp::Reverse(tree.node(id).alloc));
    v.truncate(300);
    v
}

pub fn view(s: &App, p: Pal, cx: &mut Cx<App>, cat: Option<Category>) -> El {
    let d = match screens::loaded(s, p) {
        Ok(d) => d,
        Err(e) => return e,
    };
    let tree = &d.scan.tree;
    let drive = s.volume().map(|v| v.drive()).unwrap_or_default();
    let ids: Cow<[NodeId]> = match cat {
        Some(c) => Cow::Owned(category_items(tree, &d.dominant, c)),
        None => Cow::Borrowed(&s.large.ids),
    };
    let chosen_bytes: u64 = s.large.sel.iter().map(|&id| tree.node(id).alloc).sum();
    let actionable = cat.is_none_or(Category::is_actionable);

    let header = match cat {
        None => crate::header(
            p,
            t!("nav-large"),
            &t!("large-sub", drive = drive.clone()),
            Some((i18n::size(chosen_bytes), tp!("selected", s.large.sel.len()))),
        ),
        Some(c) => {
            let row_ = &d.report.categories[c.index()];
            let back = row().gap(6.0).ml(-8.0).child(ui::link(
                p,
                "back",
                t!("back-overview"),
                |s: &mut App| s.go(Screen::Overview),
            ));
            let reclaim = row_.clean + row_.shrink;
            let sub = if reclaim > 0 {
                t!(
                    "category-sub-reclaim",
                    size = i18n::size(row_.bytes),
                    drive = drive.clone(),
                    reclaim = i18n::size(reclaim)
                )
            } else {
                t!(
                    "category-sub",
                    size = i18n::size(row_.bytes),
                    drive = drive.clone()
                )
            };
            col().key("header").w_full().gap(6.0).child(back).child(
                row()
                    .w_full()
                    .gap(24.0)
                    .items_end()
                    .child(
                        col()
                            .grow(1.0)
                            .gap(4.0)
                            .child(
                                row()
                                    .gap(10.0)
                                    .center_y()
                                    .child(ui::swatch(c, 14.0))
                                    .child(ui::txt(i18n::category(c), style::TITLE, p.ink)),
                            )
                            .child(ui::txt(sub, style::BODY, p.ink2).wrap().max_w(620.0)),
                    )
                    .child(
                        ui::txt(i18n::size(row_.bytes), style::HEADLINE, p.ink).align(Align::End),
                    ),
            )
        }
    };

    let mut top = col().key("top").w_full().gap(12.0).child(header);
    if cat.is_none() {
        top = top.child(filters(s, p));
    }

    let body = if ids.is_empty() {
        match cat {
            None => ui::empty(p, "empty", t!("large-empty-title"), t!("large-empty-body")),
            Some(_) => ui::empty(
                p,
                "empty",
                t!("category-empty-title"),
                t!("category-empty-body"),
            ),
        }
    } else {
        let widest = tree.node(ids[0]).alloc.max(1);
        let ids_owned: Vec<NodeId> = ids.to_vec();
        let ids_for_key = ids_owned.clone();
        let screen_cat = cat;
        // The scroll viewport's direct child, so only the rows on screen are built.
        col()
            .key("rows")
            .w_full()
            .growth(GrowthSpec::default())
            .virtual_rows(
                ids_owned.len(),
                ROW_H,
                move |_s: &App, i| Cow::Owned(format!("n{}", ids_for_key[i])),
                move |s: &App, i| item_row(s, ids_owned[i], widest, screen_cat, actionable),
            )
    };

    let n = s.large.sel.len();
    let footer = match crate::job_line(s, p, &[JobKind::Trash]) {
        Some(line) => line,
        None if s.confirm == Some(Confirm::Trash) => row()
            .key("confirm")
            .grow(1.0)
            .gap(10.0)
            .center_y()
            .child(
                ui::txt(
                    tp!("confirm-trash", n, size = i18n::size(chosen_bytes)),
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
                t!("move-to-bin"),
                ui::Kind::Danger,
                Some(|s: &mut App| s.confirm = None),
            )),
        None => {
            let hint = cat.and_then(crate::category_hint);
            let mut r = row().grow(1.0).gap(12.0).center_y();
            if let Some((text, screen)) = hint {
                r = r.child(ui::txt(text, style::BODY, p.ink2)).child(ui::link(
                    p,
                    "hint",
                    match screen {
                        Screen::Clean => t!("open-clean"),
                        _ => t!("open-shrink"),
                    },
                    move |s: &mut App| s.go(screen),
                ));
            }
            if actionable {
                r.child(action_row(p, t!("move-to-bin"), n > 0, |s: &mut App| {
                    s.confirm = Some(Confirm::Trash)
                }))
            } else {
                r.child(ui::txt(t!("system-hands-off"), style::BODY, p.ink2).grow(1.0))
            }
        }
    };

    let key = if cat.is_some() { "category" } else { "large" };
    crate::page(
        p,
        key,
        top,
        Some(crate::strip(s, p, cx)),
        body,
        Some(footer),
    )
}

fn item_row(s: &App, id: NodeId, widest: u64, cat: Option<Category>, actionable: bool) -> El {
    let p = s.pal.get();
    let Some(d) = &s.data else { return el() };
    let tree = &d.scan.tree;
    let n = tree.node(id);
    let path = tree.path(id);
    let c = if n.is_dir() {
        d.dominant[id as usize]
    } else {
        n.category
    };
    let on = s.large.sel.contains(&id);
    let mut line = row()
        .key(format!("n{id}"))
        .role(if actionable { "checkbox" } else { "row" })
        .label(screens::leaf(&path).to_string())
        .checked(on)
        .w_full()
        .h(ROW_H)
        .px(10.0)
        .gap(14.0)
        .center_y()
        .radius(8.0)
        .fill(if on { p.hover } else { Color::TRANSPARENT });
    if actionable && s.job.is_none() {
        line = line
            .focusable(true)
            .hover(p.hover)
            .press(p.press)
            .on_press(move |s: &mut App| {
                if !s.large.sel.remove(&id) {
                    s.large.sel.insert(id);
                }
                s.confirm = None;
            })
            .child(ui::check(p, on));
    }
    let mut name =
        row()
            .gap(8.0)
            .center_y()
            .child(ui::txt(screens::leaf(&path), style::LABEL, p.ink));
    if n.is_dir() {
        name = name.child(ui::txt(tp!("files", n.files), style::BODY, p.ink2));
    }
    line.child(ui::swatch(c, 8.0))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(name)
                .child(ui::txt(screens::parent(&path), style::PATH, p.ink2).truncate()),
        )
        .child(if cat.is_some() {
            ui::meter(p, c, n.alloc as f32 / widest as f32, 120.0)
        } else {
            ui::txt(i18n::age(n.mtime, d.now), style::BODY, p.ink2)
                .w(130.0)
                .align(Align::End)
        })
        .child(ui::figure(i18n::size(n.alloc), p.ink, 90.0))
}

fn filters(s: &App, p: Pal) -> El {
    let seg = |key: &str, items: Vec<ui::Choice>| ui::segmented(p, key, items);
    let kind = seg(
        "kind",
        vec![
            (
                t!("files-label").into(),
                !s.large.folders,
                Box::new(|s: &mut App| {
                    s.large.folders = false;
                    s.large.sel.clear();
                    s.refresh_large();
                }),
            ),
            (
                t!("folders-label").into(),
                s.large.folders,
                Box::new(|s: &mut App| {
                    s.large.folders = true;
                    s.large.sel.clear();
                    s.refresh_large();
                }),
            ),
        ],
    );
    let sizes = seg(
        "min",
        SIZES
            .iter()
            .enumerate()
            .map(|(i, &b)| {
                (
                    i18n::size(b),
                    s.large.min == i,
                    Box::new(move |s: &mut App| {
                        s.large.min = i;
                        s.refresh_large();
                    }) as Box<dyn Fn(&mut App)>,
                )
            })
            .collect(),
    );
    let ages = seg(
        "age",
        AGES.iter()
            .enumerate()
            .map(|(i, &days)| {
                let label = match days {
                    0 => t!("age-any").to_string(),
                    d if d < 365 => tp!("months", d / 30),
                    d => tp!("years", d / 365),
                };
                (
                    label,
                    s.large.age == i,
                    Box::new(move |s: &mut App| {
                        s.large.age = i;
                        s.refresh_large();
                    }) as Box<dyn Fn(&mut App)>,
                )
            })
            .collect(),
    );
    let mut chips = row().key("cats").flex_wrap().gap(6.0);
    let all = s.large.cats.is_empty();
    chips = chips.child(chip(p, "all", t!("all"), None, all, |s: &mut App| {
        s.large.cats.clear();
        s.refresh_large();
    }));
    for c in [
        Category::Documents,
        Category::Photos,
        Category::Videos,
        Category::Music,
        Category::Other,
        Category::Apps,
        Category::Games,
        Category::Developer,
        Category::Caches,
    ] {
        let on = s.large.cats.contains(&c);
        chips = chips.child(chip(
            p,
            c.id(),
            i18n::category(c),
            Some(c),
            on,
            move |s: &mut App| {
                if let Some(i) = s.large.cats.iter().position(|&x| x == c) {
                    s.large.cats.remove(i);
                } else {
                    s.large.cats.push(c);
                }
                s.refresh_large();
            },
        ));
    }
    col()
        .key("filters")
        .w_full()
        .gap(10.0)
        .child(
            row()
                .flex_wrap()
                .gap(18.0)
                .center_y()
                .child(kind)
                .child(
                    row()
                        .gap(8.0)
                        .center_y()
                        .child(ui::txt(t!("bigger-than"), style::LABEL, p.ink2))
                        .child(sizes),
                )
                .child(
                    row()
                        .gap(8.0)
                        .center_y()
                        .child(ui::txt(t!("unchanged-for"), style::LABEL, p.ink2))
                        .child(ages),
                ),
        )
        .child(chips)
}

fn chip(
    p: Pal,
    key: &str,
    label: &str,
    c: Option<Category>,
    on: bool,
    f: impl Fn(&mut App) + 'static,
) -> El {
    let mut r = row()
        .key(format!("chip-{key}"))
        .role("checkbox")
        .label(label)
        .checked(on)
        .h(26.0)
        .px(10.0)
        .gap(6.0)
        .center_y()
        .radius(13.0)
        .border(p.hair.max(1.0), if on { p.ink2 } else { p.line })
        .fill(if on { p.surface } else { Color::TRANSPARENT })
        .focusable(true)
        .hover(p.hover)
        .press(p.press)
        .on_press(f);
    if let Some(c) = c {
        r = r.child(ui::swatch(c, 8.0));
    }
    r.child(ui::txt(label, style::LABEL, if on { p.ink } else { p.ink2 }).a11y_hidden())
}
