//! Settings: language, theme, administrator, the default Shrink method.

use rune::{Cx, El, col, row};

use crate::app::{App, Theme};
use crate::i18n::{Lang, t};
use crate::screens::shrink::{algo_note, algo_picker};
use crate::style::{self, Pal};
use crate::ui;

pub fn view(s: &App, p: Pal, _cx: &mut Cx<App>) -> El {
    let header = crate::header(p, t!("nav-settings"), t!("settings-sub"), None);
    let st = &s.settings;

    let lang = ui::segmented(
        p,
        "lang",
        vec![
            choice(t!("same-as-windows"), st.lang == Lang::System, |s| {
                s.settings.lang = Lang::System
            }),
            choice("English", st.lang == Lang::English, |s| {
                s.settings.lang = Lang::English
            }),
            choice("Português", st.lang == Lang::Portuguese, |s| {
                s.settings.lang = Lang::Portuguese
            }),
        ],
    );
    let theme = ui::segmented(
        p,
        "theme",
        vec![
            choice(t!("same-as-windows"), st.theme == Theme::System, |s| {
                s.settings.theme = Theme::System
            }),
            choice(t!("theme-light"), st.theme == Theme::Light, |s| {
                s.settings.theme = Theme::Light
            }),
            choice(t!("theme-dark"), st.theme == Theme::Dark, |s| {
                s.settings.theme = Theme::Dark
            }),
        ],
    );
    let admin = row()
        .key("admin")
        .role("switch")
        .label(t!("set-admin"))
        .checked(st.ask_admin)
        .padding(4.0)
        .radius(12.0)
        .focusable(true)
        .on_press(|s: &mut App| s.settings.ask_admin = !s.settings.ask_admin)
        .child(ui::switch(p, st.ask_admin));

    let body = col()
        .key("settings")
        .w_full()
        .child(setting(
            p,
            "s-lang",
            t!("set-language"),
            t!("set-language-note"),
            lang,
        ))
        .child(setting(
            p,
            "s-theme",
            t!("set-theme"),
            t!("set-theme-note"),
            theme,
        ))
        .child(setting(
            p,
            "s-admin",
            t!("set-admin"),
            t!("set-admin-note"),
            admin,
        ))
        .child(setting(
            p,
            "s-algo",
            t!("set-method"),
            algo_note(st.algo),
            algo_picker(p, "algo", st.algo),
        ))
        .child(ui::hairline(p))
        .child(
            col()
                .key("about")
                .pt(28.0)
                .gap(4.0)
                .child(ui::txt(
                    t!("about", version = env!("CARGO_PKG_VERSION")),
                    style::LABEL,
                    p.ink,
                ))
                .child(
                    ui::txt(t!("about-journal"), style::BODY, p.ink2)
                        .wrap()
                        .max_w(520.0),
                )
                .child(ui::txt(r"%LOCALAPPDATA%\Silt\journal.jsonl", style::PATH, p.ink2).pt(4.0)),
        );
    crate::page(p, "settings", header, None, body, None)
}

use crate::ui::Choice;

fn choice(label: &str, on: bool, f: fn(&mut App)) -> Choice {
    (label.to_string(), on, Box::new(f))
}

fn setting(p: Pal, key: &str, title: &str, note: &str, control: El) -> El {
    col().key(key).w_full().child(ui::hairline(p)).child(
        row()
            .w_full()
            .py(20.0)
            .gap(40.0)
            .items_start()
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(4.0)
                    .child(ui::txt(title, style::LABEL, p.ink))
                    .child(ui::txt(note, style::BODY, p.ink2).wrap().max_w(460.0)),
            )
            .child(control),
    )
}
