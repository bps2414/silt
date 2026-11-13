//! The dither bar (docs/brand/README.md, "The bar"): the drive as a grid of cells, drawn into
//! `rune::Pixels` cell by cell so the window and the headless pictures match exactly.

use std::cell::{Cell, RefCell};
use std::hash::{Hash, Hasher};

use rune::Pixels;
use silt_core::Category;

use crate::style;

pub const BAYER: [[u8; 8]; 8] = [
    [0, 32, 8, 40, 2, 34, 10, 42],
    [48, 16, 56, 24, 50, 18, 58, 26],
    [12, 44, 4, 36, 14, 46, 6, 38],
    [60, 28, 52, 20, 62, 30, 54, 22],
    [3, 35, 11, 43, 1, 33, 9, 41],
    [51, 19, 59, 27, 49, 17, 57, 25],
    [15, 47, 7, 39, 13, 45, 5, 37],
    [63, 31, 55, 23, 61, 29, 53, 21],
];

/// One category's bytes, split by how they draw: kept (every cell lit), reclaimable (a quarter
/// lit) and reclaimable under preview (going dark as `Look::fade` runs to 1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Hash)]
pub struct Seg {
    pub cat: Option<Category>,
    pub solid: u64,
    pub soft: u64,
    pub fading: u64,
}

impl Seg {
    pub fn total(&self) -> u64 {
        self.solid + self.soft + self.fading
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Look {
    /// A scan in progress: `found` (0..1 of the capacity) lights up in ink-2 behind a grainy
    /// eight-column front.
    Settling { found: f32 },
    /// Classified. `color` (0..1) of the cells have taken their category colour; `fade` (0..1)
    /// of the previewed cells have gone dark.
    Sorted { color: f32, fade: f32 },
}

/// Cell size and count for a width in points (pitch `round(5 x scale)`, gap `max(1, round(scale))`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Geom {
    pub cols: u32,
    pub rows: u32,
    pub pitch: u32,
    pub gap: u32,
}

impl Geom {
    pub fn fit(width_pt: f32, rows: u32, scale: f32) -> Geom {
        let pitch = (5.0 * scale).round().max(3.0) as u32;
        let gap = scale.round().max(1.0) as u32;
        let px = (width_pt.max(0.0) * scale).floor() as u32;
        Geom {
            cols: ((px + gap) / pitch).max(8),
            rows,
            pitch,
            gap,
        }
    }

    pub fn px(&self) -> (u32, u32) {
        (
            self.cols * self.pitch - self.gap,
            self.rows * self.pitch - self.gap,
        )
    }

    pub fn points(&self, scale: f32) -> (f32, f32) {
        let (w, h) = self.px();
        (w as f32 / scale, h as f32 / scale)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Part {
    Free,
    Solid,
    Soft,
    Fading,
}

#[derive(Clone, Copy)]
struct Side {
    seg: usize,
    part: Part,
}

/// A bar's pixels, redrawn only when what it shows changes.
pub struct Bar {
    pub pixels: Pixels,
    pub(crate) drawn: Cell<u64>,
    /// The category under each column, for hover and press.
    cols: RefCell<Vec<Option<Category>>>,
}

impl Default for Bar {
    fn default() -> Self {
        Bar {
            pixels: Pixels::new(),
            drawn: Cell::new(0),
            cols: RefCell::new(Vec::new()),
        }
    }
}

pub struct Ink {
    pub free: [u8; 3],
    pub ink2: [u8; 3],
}

impl Bar {
    /// The category drawn at `x` points from the bar's left edge.
    pub fn category_at(&self, x: f32, g: Geom, scale: f32) -> Option<Category> {
        let col = (x * scale / g.pitch as f32).floor();
        if col < 0.0 {
            return None;
        }
        self.cols.borrow().get(col as usize).copied().flatten()
    }

    pub fn draw(&self, g: Geom, capacity: u64, segs: &[Seg], look: Look, ink: &Ink) {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (g, capacity, segs, ink.free, ink.ink2).hash(&mut h);
        match look {
            Look::Settling { found } => (0u8, found.to_bits()).hash(&mut h),
            Look::Sorted { color, fade } => (1u8, color.to_bits(), fade.to_bits()).hash(&mut h),
        }
        let key = h.finish() | 1;
        if self.drawn.get() == key {
            return;
        }
        self.drawn.set(key);

        let cols = g.cols as usize;
        let map = layout(capacity, segs, cols);
        let (w, hgt) = g.px();
        let mut rgba = vec![0u8; (w * hgt * 4) as usize];
        let cell = g.pitch - g.gap;
        let used_cols = map.iter().filter(|m| m.0.part != Part::Free).count() as f32;
        for (c, &(left, right, t)) in map.iter().enumerate() {
            for r in 0..g.rows as usize {
                let thr = BAYER[r % 8][c % 8] as f32;
                let side = if thr < 64.0 * (1.0 - t) { left } else { right };
                let (lit, colour) = match look {
                    Look::Settling { found } => {
                        let front = found * cols as f32;
                        let ahead = front - c as f32;
                        let lit = c < used_cols as usize
                            && (ahead >= 8.0 || (ahead > 0.0 && thr < 64.0 * ahead / 8.0));
                        (lit, ink.ink2)
                    }
                    Look::Sorted { color, fade } => {
                        let lit = match side.part {
                            Part::Free => false,
                            Part::Solid => true,
                            Part::Soft => thr < 16.0,
                            Part::Fading => thr < 16.0 * (1.0 - fade),
                        };
                        let cat = segs.get(side.seg).and_then(|s| s.cat);
                        let colour = match cat {
                            Some(cat) if thr < 64.0 * color => style::rgb8(style::cat_hex(cat)),
                            _ => ink.ink2,
                        };
                        (lit, colour)
                    }
                };
                let px = if lit { colour } else { ink.free };
                let (x0, y0) = (c as u32 * g.pitch, r as u32 * g.pitch);
                for y in y0..y0 + cell {
                    let row = (y * w) as usize;
                    for x in x0..x0 + cell {
                        let i = (row + x as usize) * 4;
                        rgba[i..i + 4].copy_from_slice(&[px[0], px[1], px[2], 255]);
                    }
                }
            }
        }
        *self.cols.borrow_mut() = map
            .iter()
            .map(|m| match m.0.part {
                Part::Free => None,
                _ => segs.get(m.0.seg).and_then(|s| s.cat),
            })
            .collect();
        self.pixels.set(w, hgt, rgba);
    }
}

/// Each column's segment and part, with its seam neighbour and blend `t` (0 = all left).
fn layout(capacity: u64, segs: &[Seg], cols: usize) -> Vec<(Side, Side, f32)> {
    let free = Side {
        seg: usize::MAX,
        part: Part::Free,
    };
    let cap = capacity.max(1) as f64;
    // Whole columns per segment by largest remainder, at least one for any segment with bytes.
    let exact: Vec<f64> = segs
        .iter()
        .map(|s| s.total() as f64 / cap * cols as f64)
        .collect();
    let mut n: Vec<usize> = exact
        .iter()
        .zip(segs)
        .map(|(e, s)| {
            if s.total() > 0 {
                (e.floor() as usize).max(1)
            } else {
                0
            }
        })
        .collect();
    let target = (exact.iter().sum::<f64>().round() as usize).min(cols);
    let mut order: Vec<usize> = (0..segs.len()).collect();
    order.sort_by(|&a, &b| (exact[b] - exact[b].floor()).total_cmp(&(exact[a] - exact[a].floor())));
    let mut sum: usize = n.iter().sum();
    for &i in order.iter().cycle().take(segs.len() * 2) {
        if sum >= target {
            break;
        }
        if segs[i].total() > 0 {
            n[i] += 1;
            sum += 1;
        }
    }

    let mut out: Vec<(Side, Side, f32)> = Vec::with_capacity(cols);
    let mut starts = Vec::new();
    for (i, s) in segs.iter().enumerate() {
        starts.push(out.len());
        let k = n[i];
        let tot = s.total().max(1) as f64;
        let soft = (s.soft as f64 / tot * k as f64).round() as usize;
        let fading = (s.fading as f64 / tot * k as f64).round() as usize;
        let solid = k.saturating_sub(soft + fading);
        let soft = soft.min(k - solid);
        for j in 0..k {
            let part = if j < solid {
                Part::Solid
            } else if j < solid + soft {
                Part::Soft
            } else {
                Part::Fading
            };
            let side = Side { seg: i, part };
            out.push((side, side, 0.0));
        }
    }
    out.truncate(cols);
    while out.len() < cols {
        out.push((free, free, 0.0));
    }
    // Seams: three columns blend where two segments of two or more columns meet.
    for i in 1..segs.len() {
        let k = starts[i];
        if n[i] < 2 || n[i - 1] < 2 || k + 1 >= out.len() {
            continue;
        }
        let (l, r) = (out[k - 1].0, out[k].0);
        for (j, col) in [k - 1, k, k + 1].into_iter().enumerate() {
            let own = out[col].0;
            let (left, right) = if col < k { (own, r) } else { (l, own) };
            out[col] = (left, right, (j as f32 + 0.5) / 3.0);
        }
    }
    out
}

/// The brand mark: grains settling on an 8x8 grid (`docs/brand/make.py`).
pub const MARK: [&str; 8] = [
    ".....#..", ".#......", "...#...#", "#.#..#..", ".#.#.#.#", "#.#.##.#", "###.####", "########",
];
