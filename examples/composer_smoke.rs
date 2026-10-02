use gpui_kit::*;
use mail_classifier::app::{MailApp, actions};
use mail_classifier::model::Mailbox;

fn main() {
    gpui_kit::application()
        .with_assets(mail_classifier::app::icons::AppAssets)
        .run(|cx| {
            gpui_kit::init(cx);
            mail_classifier::theme::init(cx);
            actions::bind_keys(cx);
            gpui_kit::open_window(
                gpui_kit::component::TitleBar::window_options(),
                cx,
                |window, cx| {
                    let view = cx.new(|cx| {
                        let mut app = MailApp::new(Mailbox::load_default(), window, cx);
                        app.attach_composer_in_memory(window, cx);
                        app
                    });
                    let focus = view.read(cx).focus_handle().clone();
                    window.focus(&focus, cx);
                    cx.activate(true);
                    view
                },
            )
            .expect("native composer smoke window");
            println!("COMPOSER_SMOKE_READY");
        });
}
