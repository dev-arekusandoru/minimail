use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::*;
use mail_classifier::app::MailApp;

fn main() {
    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);
        Theme::change(ThemeMode::Dark, None, cx);
        gpui_kit::open_window(
            WindowOptions {
                title: "Mail".into(),
                ..Default::default()
            },
            cx,
            |window, cx| {
                let view = cx.new(MailApp::new);
                window.focus(view.read(cx).focus_handle());
                view
            },
        )
        .expect("failed to open window");
    });
}
