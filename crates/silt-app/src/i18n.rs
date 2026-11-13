//! Every word the window shows, from `locales/app/*.json` (rune-i18n: keys checked at compile
//! time), plus sizes, counts and dates in the current language.

use rune_i18n::I18n;
use silt_core::Category;

include!(concat!(env!("OUT_DIR"), "/i18n.rs"));

pub static I18N: I18n = I18n::new(&CATALOG);

/// `t!("key")`: the message, `t!("key", name = value, ..)`: the message with `{{name}}` filled.
macro_rules! t {
    ($k:literal) => {{
        const K: rune_i18n::Key = rune_i18n::Key::named($crate::i18n::KEYS, $k);
        $crate::i18n::I18N.text(K)
    }};
    ($k:literal, $($n:ident = $v:expr),+ $(,)?) => {{
        const K: rune_i18n::Key = rune_i18n::Key::named($crate::i18n::KEYS, $k);
        $crate::i18n::I18N.format(K, &[$((stringify!($n), &$v as &dyn std::fmt::Display)),+])
    }};
}

/// `tp!("key", count, name = value, ..)`: the plural form for `count`, with `{{count}}` filled.
macro_rules! tp {
    ($k:literal, $count:expr $(, $n:ident = $v:expr)* $(,)?) => {{
        const K: rune_i18n::PluralKey = rune_i18n::PluralKey::named($crate::i18n::PLURALS, $k);
        let c: u64 = $count as u64;
        let shown = $crate::i18n::int(c);
        $crate::i18n::I18N.plural_format(K, c, &[("count", &shown as &dyn std::fmt::Display) $(, (stringify!($n), &$v as &dyn std::fmt::Display))*])
    }};
}

pub(crate) use {t, tp};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Lang {
    #[default]
    System,
    English,
    Portuguese,
}

/// Points every later lookup at `lang`.
pub fn apply(lang: Lang) {
    let i = match lang {
        Lang::System => {
            let langs = rune_i18n::system_languages();
            I18N.negotiate(langs.iter().map(String::as_str))
        }
        Lang::English => I18N.find_locale("en").unwrap_or(0),
        Lang::Portuguese => I18N.find_locale("pt-BR").unwrap_or(0),
    };
    I18N.set_locale(i);
}

fn decimal_comma() -> bool {
    I18N.info().id() == "pt-BR"
}

/// `1.2 GB` (or `1,2 GB`), as the CLI prints it.
pub fn size(bytes: u64) -> String {
    let s = silt_core::fmt_bytes(bytes);
    if decimal_comma() {
        s.replace('.', ",")
    } else {
        s
    }
}

/// `1.2 to 1.5 GB`, sharing the unit when both ends have it.
pub fn range(lo: u64, hi: u64) -> String {
    let (a, b) = (size(lo), size(hi));
    if a == b {
        return a;
    }
    match (a.rsplit_once(' '), b.rsplit_once(' ')) {
        (Some((na, ua)), Some((nb, ub))) if ua == ub => t!("range", from = na, to = nb, unit = ub),
        _ => t!("range-units", from = a, to = b),
    }
}

/// `12,345` (or `12.345`).
pub fn int(n: u64) -> String {
    I18N.integer(n as i64)
}

pub fn category(c: Category) -> &'static str {
    match c {
        Category::Apps => t!("cat-apps"),
        Category::Games => t!("cat-games"),
        Category::Developer => t!("cat-developer"),
        Category::Caches => t!("cat-caches"),
        Category::Documents => t!("cat-documents"),
        Category::Photos => t!("cat-photos"),
        Category::Videos => t!("cat-videos"),
        Category::Music => t!("cat-music"),
        Category::System => t!("cat-system"),
        Category::Trash => t!("cat-trash"),
        Category::Other => t!("cat-other"),
    }
}

/// `12 Nov 2026` / `12 de nov. de 2026`, from Unix seconds.
pub fn date(ts: u64) -> String {
    let (y, m, d) = civil(ts / 86_400);
    let month = match m {
        1 => t!("month-1"),
        2 => t!("month-2"),
        3 => t!("month-3"),
        4 => t!("month-4"),
        5 => t!("month-5"),
        6 => t!("month-6"),
        7 => t!("month-7"),
        8 => t!("month-8"),
        9 => t!("month-9"),
        10 => t!("month-10"),
        11 => t!("month-11"),
        _ => t!("month-12"),
    };
    t!("date", day = d, month = month, year = y)
}

/// Days since 1970-01-01 to (year, month, day), proleptic Gregorian.
fn civil(days: u64) -> (i64, u32, u32) {
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// "3 months ago", "2 years ago": how long a file has sat unchanged.
pub fn age(mtime: u32, now: u64) -> String {
    let days = now.saturating_sub(u64::from(mtime)) / 86_400;
    if days < 1 {
        t!("age-today").to_string()
    } else if days < 45 {
        tp!("age-days", days)
    } else if days < 548 {
        tp!("age-months", (days + 15) / 30)
    } else {
        tp!("age-years", (days + 182) / 365)
    }
}
