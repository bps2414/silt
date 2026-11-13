use rune::tok::{self, type_};
use rune::{App, AppSpec, Cx, El, Titlebar, col, row, text};

#[derive(Default)]
struct State;

fn view(_: &State, cx: &mut Cx<State>) -> El {
    let t = tok::theme(cx.system_dark());
    let insets = cx.chrome_insets();
    let bar = row()
        .key("titlebar")
        .w_full()
        .h(40.0)
        .center_y()
        .window_drag()
        .pl(insets.leading + 14.0)
        .child(text("Silt").font(type_::LABEL).color(t.text));
    col().key("root").w_full().h_full().fill(t.bg).child(bar)
}

fn main() {
    App::new(AppSpec::new("Silt", (1080.0, 720.0), State::default, view))
        .titlebar(Titlebar::new(40.0))
        .run();
}
