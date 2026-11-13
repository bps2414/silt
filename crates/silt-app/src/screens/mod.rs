pub mod clean;
pub mod explore;
pub mod history;
pub mod large;
pub mod overview;
pub mod settings;
pub mod shrink;

use rune::El;

use crate::app::{App, Loaded, Phase};
use crate::i18n::t;
use crate::style::Pal;
use crate::ui;

/// The scan's results, or the screen to show while there are none.
#[allow(clippy::result_large_err)]
pub fn loaded(s: &App, p: Pal) -> Result<&Loaded, El> {
    match (&s.phase, &s.data) {
        (Phase::Ready, Some(d)) => Ok(d),
        (Phase::Failed { drive, .. }, _) => Err(ui::empty(
            p,
            "not-ready",
            &t!("failed-title", drive = drive.clone()),
            t!("failed-elsewhere"),
        )
        .px(crate::style::PAD)),
        _ => Err(ui::empty(
            p,
            "not-ready",
            &t!(
                "reading",
                drive = s.volume().map(|v| v.drive()).unwrap_or_default()
            ),
            t!("scanning-elsewhere"),
        )
        .px(crate::style::PAD)),
    }
}

/// The last folder or file name of a path.
pub fn leaf(path: &str) -> &str {
    path.trim_end_matches('\\')
        .rsplit('\\')
        .next()
        .unwrap_or(path)
}

/// The folder a path sits in.
pub fn parent(path: &str) -> &str {
    path.trim_end_matches('\\')
        .rsplit_once('\\')
        .map_or("", |(p, _)| p)
}
