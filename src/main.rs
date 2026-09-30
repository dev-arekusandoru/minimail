use gpui_kit::*;
use mail_classifier::app::{MailApp, actions};
use mail_classifier::model::Mailbox;
use mail_classifier::theme;

fn main() {
    gpui_kit::application().with_assets(mail_classifier::app::icons::AppAssets).run(|cx| {
        gpui_kit::init(cx);
        theme::init(cx);
        theme::watch_user_themes(cx);
        actions::bind_keys(cx);
        gpui_kit::open_window(
            WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: None,
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(9.), px(9.))),
                }),
                app_owns_titlebar_drag: true,
                ..Default::default()
            },
            cx,
            |window, cx| {
                let store = mail_classifier::contacts::open_default()
                    .expect("contacts database at $MAIL_CLASSIFIER_DB");
                let view = cx.new(|cx| {
                    MailApp::new(Mailbox::load_default_with(std::rc::Rc::new(store)), window, cx)
                });
                let handle = view.read(cx).focus_handle().clone();
                window.focus(&handle, cx);
                cx.activate(true);
                view
            },
        )
        .expect("failed to open window");
    });
}
