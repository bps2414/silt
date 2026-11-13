//! Explore: any folder's contents by size, as a list or as a map of cells.

use std::borrow::Cow;

use rune::{Align, Color, Cx, El, GrowthSpec, col, el, row};
use silt_core::tree::NONE;
use silt_core::{NodeId, Tree};

use crate::app::App;
use crate::bar::Geom;
use crate::i18n::{self, t, tp};
use crate::style::{self, Pal};
use crate::treemap::{self, Tile};
use crate::{screens, ui};

const ROW_H: f32 = 44.0;
const MAP_TILES: usize = 120;

pub fn view(s: &App, p: Pal, cx: &mut Cx<App>) -> El {
    let d = match screens::loaded(s, p) {
        Ok(d) => d,
        Err(e) => return e,
    };
    let tree = &d.scan.tree;
    let folder = s.explore.folder;
    let node = tree.node(folder);

    let toggle = ui::segmented(
        p,
        "view",
        vec![
            (
                t!("as-list").into(),
                !s.explore.map,
                Box::new(|s: &mut App| s.explore.map = false),
            ),
            (
                t!("as-map").into(),
                s.explore.map,
                Box::new(|s: &mut App| s.explore.map = true),
            ),
        ],
    );
    let header = col()
        .key("header")
        .w_full()
        .gap(8.0)
        .child(
            row()
                .w_full()
                .gap(24.0)
                .items_end()
                .child(
                    col()
                        .grow(1.0)
                        .gap(4.0)
                        .child(ui::txt(t!("nav-explore"), style::TITLE, p.ink))
                        .child(ui::txt(
                            t!(
                                "explore-sub",
                                size = i18n::size(node.alloc),
                                files = tp!("files", node.files)
                            ),
                            style::BODY,
                            p.ink2,
                        )),
                )
                .child(toggle),
        )
        .child(crumbs(tree, folder, p));

    let children: Vec<NodeId> = tree
        .children(folder)
        .filter(|&c| tree.node(c).alloc > 0)
        .collect();
    if children.is_empty() {
        let body = ui::empty(
            p,
            "empty",
            t!("folder-empty-title"),
            t!("folder-empty-body"),
        );
        return crate::page(p, "explore", header, None, body, None);
    }

    if s.explore.map {
        return col()
            .key("explore")
            .w_full()
            .h_full()
            .min_h(0.0)
            .child(header.px(style::PAD).pt(18.0).pb(14.0))
            .child(
                col()
                    .key("map-pad")
                    .w_full()
                    .grow(1.0)
                    .min_h(0.0)
                    .px(style::PAD)
                    .pb(style::PAD)
                    .child(map(s, p, cx, tree, &children)),
            );
    }

    let total = node.alloc.max(1);
    let widest = tree.node(children[0]).alloc.max(1);
    let keys = children.clone();
    let body = col()
        .key("rows")
        .w_full()
        .growth(GrowthSpec::default())
        .virtual_rows(
            children.len(),
            ROW_H,
            move |_s: &App, i| Cow::Owned(format!("c{}", keys[i])),
            move |s: &App, i| child_row(s, children[i], total, widest),
        );
    crate::page(p, "explore", header, None, body, None)
}

fn crumbs(tree: &Tree, folder: NodeId, p: Pal) -> El {
    let mut chain = Vec::new();
    let mut cur = folder;
    while cur != NONE {
        chain.push(cur);
        let parent = tree.node(cur).parent;
        if parent == cur {
            break;
        }
        cur = parent;
    }
    chain.reverse();
    let mut r = row().key("crumbs").flex_wrap().gap(0.0).center_y().ml(-6.0);
    let last = chain.len().saturating_sub(1);
    for (i, &id) in chain.iter().enumerate() {
        let name = tree.name(id).to_string();
        if i > 0 {
            r = r.child(ui::txt("\\", style::PATH, p.ink2).px(1.0));
        }
        let mut seg = row()
            .key(format!("crumb{id}"))
            .h(24.0)
            .px(6.0)
            .center_y()
            .radius(4.0)
            .child(ui::txt(
                name.clone(),
                style::PATH,
                if i == last { p.ink } else { p.flour },
            ));
        if i != last {
            seg = seg
                .role("link")
                .label(name)
                .focusable(true)
                .hover(p.hover)
                .press(p.press)
                .on_press(move |s: &mut App| s.explore.folder = id);
        }
        r = r.child(seg);
    }
    r
}

fn child_row(s: &App, id: NodeId, total: u64, widest: u64) -> El {
    let p = s.pal.get();
    let Some(d) = &s.data else { return el() };
    let tree = &d.scan.tree;
    let n = tree.node(id);
    let c = if n.is_dir() {
        d.dominant[id as usize]
    } else {
        n.category
    };
    let name = tree.name(id).to_string();
    let pct = n.alloc as f64 * 100.0 / total as f64;
    let pct = if pct >= 9.95 {
        format!("{pct:.0}%")
    } else if pct >= 0.05 {
        format!("{pct:.1}%")
    } else {
        "<0.1%".into()
    };
    let pct = if i18n::size(1536).contains(',') {
        pct.replace('.', ",")
    } else {
        pct
    };
    let mut r = row()
        .key(format!("c{id}"))
        .label(name.clone())
        .w_full()
        .h(ROW_H)
        .px(10.0)
        .gap(14.0)
        .center_y()
        .radius(8.0);
    let mut title = row()
        .gap(8.0)
        .center_y()
        .child(ui::txt(name, style::LABEL, p.ink).truncate());
    if n.is_dir() {
        r = r
            .role("link")
            .focusable(true)
            .hover(p.hover)
            .press(p.press)
            .on_press(move |s: &mut App| s.explore.folder = id);
        title = title.child(ui::txt(tp!("files", n.files), style::BODY, p.ink2));
    }
    r.child(ui::swatch(c, 8.0))
        .child(title.grow(1.0).min_w(0.0))
        .child(ui::meter(p, c, n.alloc as f32 / widest as f32, 160.0))
        .child(
            ui::txt(pct, style::FIGURE, p.ink2)
                .w(56.0)
                .align(Align::End),
        )
        .child(ui::figure(i18n::size(n.alloc), p.ink, 90.0))
}

fn map(s: &App, p: Pal, cx: &mut Cx<App>, tree: &Tree, children: &[NodeId]) -> El {
    let Some(d) = &s.data else { return el() };
    let scale = cx.env().scale;
    let [w, h] = cx.layout_size("map-slot").unwrap_or([1060.0, 500.0]);
    let g0 = Geom::fit(w, 1, scale);
    let rows = ((h * scale).floor() as u32 + g0.gap) / g0.pitch;
    let g = Geom {
        rows: rows.max(8),
        ..g0
    };
    let items: Vec<(NodeId, u64)> = children
        .iter()
        .take(MAP_TILES)
        .map(|&c| (c, tree.node(c).alloc))
        .collect();
    let tiles = treemap::squarify(&items, g.cols, g.rows);
    let coloured: Vec<(Tile, [u8; 3])> = tiles
        .iter()
        .map(|t| {
            let n = tree.node(t.id);
            let c = if n.is_dir() {
                d.dominant[t.id as usize]
            } else {
                n.category
            };
            (*t, style::rgb8(style::cat_hex(c)))
        })
        .collect();
    treemap::draw(&s.map, g, &coloured, p.free8);
    let (pw, ph) = g.points(scale);
    let pt = g.pitch as f32 / scale;
    let mut surface = el()
        .key("map")
        .role("img")
        .label(t!("map-label"))
        .size(pw, ph)
        .pixels(&s.map.pixels);
    for t in &tiles {
        let (tw, th) = (t.x1 - t.x0, t.y1 - t.y0);
        if tw < 3 || th < 3 {
            continue;
        }
        let n = tree.node(t.id);
        let id = t.id;
        let mut hit = col()
            .key(format!("t{id}"))
            .absolute(t.x0 as f32 * pt, t.y0 as f32 * pt)
            .size(
                (tw - 1) as f32 * pt + pt - g.gap as f32 / scale,
                (th - 1) as f32 * pt,
            )
            .padding(6.0)
            .items_start()
            .label(tree.name(id).to_string());
        if n.is_dir() {
            hit = hit
                .role("link")
                .focusable(true)
                .hover(Color::WHITE.alpha(0.16))
                .press(Color::BLACK.alpha(0.12))
                .on_press(move |s: &mut App| s.explore.folder = id);
        }
        if tw >= 16 && th >= 8 {
            hit = hit.child(
                col()
                    .padding(5.0)
                    .px(7.0)
                    .radius(4.0)
                    .fill(p.bed.alpha(0.9))
                    .child(
                        ui::txt(tree.name(id).to_string(), style::LABEL, p.ink)
                            .truncate()
                            .max_w((tw as f32 * pt - 30.0).max(40.0)),
                    )
                    .child(ui::txt(i18n::size(n.alloc), style::FIGURE, p.ink2))
                    .a11y_hidden(),
            );
        }
        surface = surface.child(hit);
    }
    col()
        .key("map-slot")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .child(surface)
}
