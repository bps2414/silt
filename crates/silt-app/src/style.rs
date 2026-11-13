//! The brand's tokens (docs/brand/README.md): colours for light and dark, the category hues and
//! the Archivo type roles.

use rune::{Color, TextStyle};
use silt_core::Category;

/// Archivo, variable in weight and width.
pub static FONTS: &[&[u8]] = &[include_bytes!("../assets/fonts/Archivo.ttf")];
pub const FAMILIES: (&str, &str) = ("Archivo", "Archivo");

/// sRGB `0xRRGGBB` to the linear colour Rune paints with.
pub fn rgb(hex: u32) -> Color {
    let ch = |shift: u32| {
        let v = ((hex >> shift) & 0xFF) as f32 / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    Color {
        r: ch(16),
        g: ch(8),
        b: ch(0),
        a: 1.0,
    }
}

pub fn rgb8(hex: u32) -> [u8; 3] {
    [(hex >> 16) as u8, (hex >> 8) as u8, hex as u8]
}

#[derive(Clone, Copy)]
pub struct Pal {
    pub dark: bool,
    pub bed: Color,
    pub surface: Color,
    pub ink: Color,
    pub ink2: Color,
    pub line: Color,
    /// Unlit cells.
    pub free: Color,
    pub flour: Color,
    pub danger: Color,
    /// Text on a flour or danger button.
    pub on_accent: Color,
    pub hover: Color,
    pub press: Color,
    /// Raw sRGB for the bar's pixels.
    pub free8: [u8; 3],
    pub ink2_8: [u8; 3],
    pub flour8: [u8; 3],
    /// One device pixel, in points.
    pub hair: f32,
}

pub fn pal(dark: bool, scale: f32) -> Pal {
    let (bed, surface, ink, ink2, line, free, flour, danger) = if dark {
        (
            0x142024, 0x1B2A2F, 0xE3EAE7, 0x93A3A5, 0x2A3A3F, 0x22343A, 0x4CC0B5, 0xE0705C,
        )
    } else {
        (
            0xEEF1EE, 0xF8FAF8, 0x1D3138, 0x56676C, 0xC9D1CF, 0xD5DCD9, 0x1E8A86, 0xB3412F,
        )
    };
    let ink_c = rgb(ink);
    Pal {
        dark,
        bed: rgb(bed),
        surface: rgb(surface),
        ink: ink_c,
        ink2: rgb(ink2),
        line: rgb(line),
        free: rgb(free),
        flour: rgb(flour),
        danger: rgb(danger),
        on_accent: if dark { rgb(0x142024) } else { rgb(0xFFFFFF) },
        hover: ink_c.alpha(if dark { 0.06 } else { 0.05 }),
        press: ink_c.alpha(if dark { 0.10 } else { 0.09 }),
        free8: rgb8(free),
        ink2_8: rgb8(ink2),
        flour8: rgb8(flour),
        hair: 1.0 / scale.max(1.0),
    }
}

/// Category hues, indexed by `Category::index()`.
const CATEGORY_HEX: [u32; 11] = [
    0x3F78B0, // Apps
    0x7D5BC4, // Games
    0x5A9A48, // Developer
    0xD2A23C, // Caches
    0xD9773F, // Documents
    0xC9518A, // Photos
    0xC9473E, // Videos
    0x3BA3C4, // Music
    0x7D8A8E, // System
    0xA9B2B0, // Trash
    0xA69A7C, // Other
];

pub fn cat_hex(c: Category) -> u32 {
    CATEGORY_HEX[c.index()]
}

pub fn cat_color(c: Category) -> Color {
    rgb(cat_hex(c))
}

/// The bar's fixed order: segments never swap places.
pub const BAR_ORDER: [Category; 11] = [
    Category::Apps,
    Category::Games,
    Category::Developer,
    Category::Caches,
    Category::Documents,
    Category::Photos,
    Category::Videos,
    Category::Music,
    Category::Other,
    Category::System,
    Category::Trash,
];

// Type roles. Weight and width go through the variation axes so the variable font lands on
// exactly these instances; every figure uses tabular numerals.
pub const DISPLAY: TextStyle = TextStyle::new(56.0, 720.0)
    .lh(60.0)
    .variations("'wght' 720, 'wdth' 125")
    .features("'tnum' 1");
pub const DISPLAY_CONTEXT: TextStyle = TextStyle::new(20.0, 450.0)
    .lh(24.0)
    .variations("'wght' 450, 'wdth' 75")
    .features("'tnum' 1");
pub const TITLE: TextStyle = TextStyle::new(20.0, 640.0)
    .lh(26.0)
    .variations("'wght' 640, 'wdth' 112")
    .features("'tnum' 1");
pub const BODY: TextStyle = TextStyle::new(14.0, 420.0)
    .lh(20.0)
    .variations("'wght' 420, 'wdth' 100")
    .features("'tnum' 1");
pub const LABEL: TextStyle = TextStyle::new(13.0, 560.0)
    .lh(18.0)
    .variations("'wght' 560, 'wdth' 100")
    .features("'tnum' 1");
pub const FIGURE: TextStyle = TextStyle::new(14.0, 520.0)
    .lh(20.0)
    .variations("'wght' 520, 'wdth' 92")
    .features("'tnum' 1");
pub const PATH: TextStyle = TextStyle::new(13.0, 420.0)
    .lh(18.0)
    .variations("'wght' 420, 'wdth' 75")
    .features("'tnum' 1");
/// The wordmark beside the mark in the titlebar.
pub const WORDMARK: TextStyle = TextStyle::new(17.0, 760.0)
    .lh(20.0)
    .variations("'wght' 760, 'wdth' 125");
/// A large figure in a screen header (what a screen would recover).
pub const HEADLINE: TextStyle = TextStyle::new(32.0, 700.0)
    .lh(36.0)
    .variations("'wght' 700, 'wdth' 125")
    .features("'tnum' 1");
/// The same figure set narrow: what an action saved ("the type does what the files did").
pub const HEADLINE_NARROW: TextStyle = TextStyle::new(32.0, 700.0)
    .lh(36.0)
    .variations("'wght' 700, 'wdth' 62")
    .features("'tnum' 1");

pub const PAD: f32 = 28.0;
