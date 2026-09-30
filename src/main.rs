use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::*;
use mail_classifier::app::{MailApp, actions};
use mail_classifier::model::Mailbox;
use mail_classifier::theme;

fn main() {
    gpui_kit::application().with_assets(mail_classifier::app::icons::AppAssets).run(|cx| {
        gpui_kit::init(cx);
        Theme::change(ThemeMode::Dark, None, cx);
        let mut registry = theme::ThemeRegistry::builtin();
        if let Some(dir) = theme::user_themes_dir() {
            for err in registry.load_dir(&dir) {
                eprintln!("theme skipped: {err}");
            }
        }
        theme::install(cx, registry, theme::DEFAULT_THEME);
        actions::bind_keys(cx);
        gpui_kit::open_window(
            WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("Mail".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            cx,
            |window, cx| {
                let view = cx.new(|cx| MailApp::new(Mailbox::load_default(), window, cx));
                let handle = view.read(cx).focus_handle().clone();
                window.focus(&handle, cx);
                cx.activate(true);
                view
            },
        )
        .expect("failed to open window");
    });
}
