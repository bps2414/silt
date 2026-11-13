//! The few controls Silt draws: buttons, check boxes, segmented choices, notices. Flat, on the
//! bed colour, with flour for what you can do and danger only on the confirm of a deletion.

use rune::{Align, Color, El, Shape, col, el, row, text};
use silt_core::Category;

use crate::app::App;
use crate::style::{self, Pal};

pub fn txt(s: impl Into<String>, style: rune::TextStyle, color: Color) -> El {
    text(s.into()).font(style).color(color)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Primary,
    Danger,
    Quiet,
}

/// A button. `on` is `None` when it can't be used right now.
pub fn button(p: Pal, key: &str, label: &str, kind: Kind, on: Option<fn(&mut App)>) -> El {
    let (fill, ink, border) = match kind {
        Kind::Primary => (p.flour, p.on_accent, Color::TRANSPARENT),
        Kind::Danger => (p.danger, p.on_accent, Color::TRANSPARENT),
        Kind::Quiet => (Color::TRANSPARENT, p.ink, p.line),
    };
    let mut b = row()
        .key(key)
        .role("button")
        .label(label)
        .h(32.0)
        .px(14.0)
        .center_y()
        .radius(6.0)
        .fill(fill)
        .border(p.hair.max(1.0), border)
        .child(txt(label, style::LABEL, ink).a11y_hidden());
    match on {
        Some(f) => {
            b = b
                .focusable(true)
                .hover(if kind == Kind::Quiet {
                    p.hover
                } else {
                    Color::WHITE.alpha(0.10)
                })
                .press(if kind == Kind::Quiet {
                    p.press
                } else {
                    Color::BLACK.alpha(0.10)
                })
                .on_press(f);
        }
        None => b = b.opacity(0.4),
    }
    b
}

/// A text button that reads as a link (navigation inside the window).
pub fn link(p: Pal, key: &str, label: &str, f: impl Fn(&mut App) + 'static) -> El {
    row()
        .key(key)
        .role("button")
        .label(label)
        .h(28.0)
        .px(8.0)
        .center_y()
        .radius(5.0)
        .focusable(true)
        .hover(p.hover)
        .press(p.press)
        .on_press(f)
        .child(txt(label, style::LABEL, p.flour).a11y_hidden())
}

/// A check box: a square cell that fills with flour and takes a tick.
pub fn check(p: Pal, on: bool) -> El {
    let s = 16.0;
    let b = el()
        .size(s, s)
        .radius(4.0)
        .a11y_hidden()
        .motion(rune::tok::motion::SNAPPY);
    if on {
        b.fill(p.flour).child(
            el().size(s, s).shape(
                Shape::new()
                    .stroke(vec![[4.0, 8.4], [6.9, 11.2], [12.2, 5.2]], 1.9, p.on_accent)
                    .view_box(s, s),
            ),
        )
    } else {
        b.border(1.5, p.ink2)
    }
}

/// A switch for Settings.
pub fn switch(p: Pal, on: bool) -> El {
    let (w, h, k) = (36.0, 20.0, 14.0);
    let inset = (h - k) / 2.0;
    el().size(w, h)
        .radius(h / 2.0)
        .fill(if on { p.flour } else { p.line })
        .motion(rune::tok::motion::SNAPPY)
        .a11y_hidden()
        .child(
            el().size(k, k)
                .absolute(if on { w - k - inset } else { inset }, inset)
                .radius(k / 2.0)
                .fill(if on { p.on_accent } else { p.surface }),
        )
}

/// A segmented choice: its label, whether it is chosen, and what choosing it does.
pub type Choice = (String, bool, Box<dyn Fn(&mut App)>);

/// Choices side by side; the chosen one sits on the surface colour with an ink label.
pub fn segmented(p: Pal, key: &str, items: Vec<Choice>) -> El {
    let mut r = row()
        .key(key)
        .role("radiogroup")
        .padding(3.0)
        .gap(2.0)
        .radius(8.0)
        .fill(p.ink.alpha(if p.dark { 0.07 } else { 0.06 }));
    for (i, (label, on, f)) in items.into_iter().enumerate() {
        let f: std::rc::Rc<dyn Fn(&mut App)> = f.into();
        r = r.child(
            row()
                .key(format!("seg{i}"))
                .role("radio")
                .label(label.clone())
                .checked(on)
                .h(26.0)
                .px(12.0)
                .center_y()
                .radius(6.0)
                .fill(if on { p.surface } else { Color::TRANSPARENT })
                .border(p.hair, if on { p.line } else { Color::TRANSPARENT })
                .focusable(true)
                .hover(if on { Color::TRANSPARENT } else { p.hover })
                .on_press(move |s: &mut App| f(s))
                .child(txt(label, style::LABEL, if on { p.ink } else { p.ink2 }).a11y_hidden()),
        );
    }
    r
}

/// A category's swatch: one square cell of its colour.
pub fn swatch(c: Category, size: f32) -> El {
    el().size(size, size)
        .radius(1.5)
        .fill(style::cat_color(c))
        .a11y_hidden()
}

/// A thin proportional bar for list rows (`frac` 0..1 of the widest row).
pub fn meter(p: Pal, c: Category, frac: f32, w: f32) -> El {
    el().w(w)
        .h(4.0)
        .radius(2.0)
        .fill(p.line.alpha(0.6))
        .a11y_hidden()
        .child(
            el().w((w * frac.clamp(0.0, 1.0)).max(2.0))
                .h(4.0)
                .radius(2.0)
                .fill(style::cat_color(c)),
        )
}

/// A one-line message strip with optional actions, for states that need the person.
pub fn notice(p: Pal, key: &str, tone: Color, title: &str, body: &str, actions: Vec<El>) -> El {
    let mut acts = row().gap(8.0).center_y();
    for a in actions {
        acts = acts.child(a);
    }
    row()
        .key(key)
        .w_full()
        .gap(16.0)
        .px(16.0)
        .py(12.0)
        .center_y()
        .radius(8.0)
        .fill(tone.alpha(if p.dark { 0.14 } else { 0.10 }))
        .child(el().w(4.0).h(32.0).radius(2.0).fill(tone))
        .child(
            col()
                .grow(1.0)
                .gap(2.0)
                .child(txt(title, style::LABEL, p.ink))
                .child(txt(body, style::BODY, p.ink2).wrap()),
        )
        .child(acts)
}

/// An empty list: what it means and what to do next.
pub fn empty(p: Pal, key: &str, title: &str, body: &str) -> El {
    col()
        .key(key)
        .w_full()
        .py(48.0)
        .gap(6.0)
        .child(txt(title, style::TITLE, p.ink))
        .child(txt(body, style::BODY, p.ink2).wrap().max_w(520.0))
}

/// Progress of an action: a thin track that fills with flour.
pub fn progress(p: Pal, frac: f32) -> El {
    let f = frac.clamp(0.0, 1.0);
    row()
        .w_full()
        .h(4.0)
        .radius(2.0)
        .fill(p.line)
        .child(
            el().key("done")
                .grow(f.max(0.001))
                .h(4.0)
                .radius(2.0)
                .fill(p.flour)
                .motion(rune::tok::motion::DEFAULT),
        )
        .child(el().key("left").grow((1.0 - f).max(0.001)).h(4.0))
}

/// A row of figures aligned right in a fixed column.
pub fn figure(s: impl Into<String>, color: Color, w: f32) -> El {
    txt(s, style::FIGURE, color).w(w).align(Align::End)
}

pub fn hairline(p: Pal) -> El {
    el().w_full().h(p.hair).fill(p.line).a11y_hidden()
}
