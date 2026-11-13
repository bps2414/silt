//! The Explore map: a folder's contents as squarified tiles of cells, each in the colour of the
//! category that fills it, on the same grid as the bar.

use std::hash::{Hash, Hasher};

use silt_core::NodeId;

use crate::bar::{Bar, Geom};

/// A tile in cell units.
#[derive(Clone, Copy, Debug)]
pub struct Tile {
    pub id: NodeId,
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
}

/// Lays `items` (biggest first) into a `w` x `h` cell rectangle, keeping tiles close to square.
pub fn squarify(items: &[(NodeId, u64)], w: u32, h: u32) -> Vec<Tile> {
    let total: f64 = items.iter().map(|i| i.1 as f64).sum();
    if total <= 0.0 || w == 0 || h == 0 {
        return Vec::new();
    }
    let k = (w as f64 * h as f64) / total;
    let areas: Vec<(NodeId, f64)> = items.iter().map(|&(id, b)| (id, b as f64 * k)).collect();
    let (mut x, mut y, mut rw, mut rh) = (0.0f64, 0.0f64, w as f64, h as f64);
    let mut out = Vec::new();
    let mut i = 0;
    while i < areas.len() {
        let side = rw.min(rh).max(1e-9);
        let mut j = i + 1;
        let mut sum = areas[i].1;
        let worst = |sum: f64, max: f64, min: f64| {
            let s2 = sum * sum;
            let side2 = side * side;
            (side2 * max / s2).max(s2 / (side2 * min.max(1e-9)))
        };
        let mut best = worst(sum, areas[i].1, areas[i].1);
        while j < areas.len() {
            let next = sum + areas[j].1;
            let w2 = worst(next, areas[i].1, areas[j].1);
            if w2 > best {
                break;
            }
            best = w2;
            sum = next;
            j += 1;
        }
        // Place areas[i..j] as one strip along the shorter side.
        let horizontal = rw >= rh;
        let thick = sum / side;
        let mut pos = 0.0;
        for &(id, a) in &areas[i..j] {
            let len = a / thick;
            let (tx, ty, tw, th) = if horizontal {
                (x, y + pos, thick, len)
            } else {
                (x + pos, y, len, thick)
            };
            pos += len;
            let (x0, y0) = (tx.round() as u32, ty.round() as u32);
            let (x1, y1) = ((tx + tw).round() as u32, (ty + th).round() as u32);
            if x1 > x0 && y1 > y0 {
                out.push(Tile { id, x0, y0, x1, y1 });
            }
        }
        if horizontal {
            x += thick;
            rw -= thick;
        } else {
            y += thick;
            rh -= thick;
        }
        i = j;
    }
    out
}

/// Draws the tiles: every cell of a tile lit in its colour, with one unlit cell between tiles.
pub fn draw(bar: &Bar, g: Geom, tiles: &[(Tile, [u8; 3])], free: [u8; 3]) {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    g.hash(&mut h);
    free.hash(&mut h);
    for (t, c) in tiles {
        (t.id, t.x0, t.y0, t.x1, t.y1, c).hash(&mut h);
    }
    let key = h.finish() | 2;
    if bar.drawn.get() == key {
        return;
    }
    bar.drawn.set(key);
    let (w, hgt) = g.px();
    let mut grid = vec![free; (g.cols * g.rows) as usize];
    for (t, c) in tiles {
        let x_end = if t.x1 - t.x0 >= 2 { t.x1 - 1 } else { t.x1 };
        let y_end = if t.y1 - t.y0 >= 2 { t.y1 - 1 } else { t.y1 };
        for y in t.y0..y_end.min(g.rows) {
            for x in t.x0..x_end.min(g.cols) {
                grid[(y * g.cols + x) as usize] = *c;
            }
        }
    }
    let cell = g.pitch - g.gap;
    let mut rgba = vec![0u8; (w * hgt * 4) as usize];
    for r in 0..g.rows {
        for c in 0..g.cols {
            let px = grid[(r * g.cols + c) as usize];
            for y in r * g.pitch..r * g.pitch + cell {
                let row = (y * w) as usize;
                for x in c * g.pitch..c * g.pitch + cell {
                    let i = (row + x as usize) * 4;
                    rgba[i..i + 4].copy_from_slice(&[px[0], px[1], px[2], 255]);
                }
            }
        }
    }
    bar.pixels.set(w, hgt, rgba);
}
