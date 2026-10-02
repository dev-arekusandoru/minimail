use gpui_kit::*;
use mail_classifier::app::{MailApp, actions};
use mail_classifier::model::Mailbox;
use mail_classifier::sync::cache::{Cache, default_cache_path};
use mail_classifier::theme;

fn main() {
    let store = match mail_classifier::contacts::open_default() {
        Ok(store) => std::rc::Rc::new(store),
        Err(e) => {
            eprintln!(
                "cannot open contacts database {}: {e}",
                mail_classifier::contacts::default_db_path().display()
            );
            std::process::exit(1);
        }
    };
    gpui_kit::application().with_assets(mail_classifier::app::icons::AppAssets).run(move |cx| {
        gpui_kit::init(cx);
        theme::init(cx);
        theme::watch_user_themes(cx);
        actions::bind_keys(cx);
        gpui_kit::open_window(
            gpui_kit::component::TitleBar::window_options(),
            cx,
            move |window, cx| {
                let cache = match Cache::open(&default_cache_path()) {
                    Ok(cache) => Some(std::rc::Rc::new(cache)),
                    Err(e) => {
                        eprintln!("mail cache unavailable: {e}");
                        None
                    }
                };
                let cached = cache.as_ref().and_then(|c| match c.load() {
                    Ok((accounts, folders, messages)) if !accounts.is_empty() => {
                        Some((accounts, folders, messages))
                    }
                    Ok(_) => None,
                    Err(e) => {
                        eprintln!("mail cache unreadable: {e}");
                        None
                    }
                });
                let demo = cached.is_none();
                let mailbox = match cached {
                    Some((accounts, folders, messages)) => {
                        Mailbox::from_parts(messages, accounts, folders, store)
                    }
                    None => Mailbox::load_default_with(store),
                };
                let view = cx.new(|cx| MailApp::new(mailbox, window, cx));
                if let Some(cache) = cache {
                    view.update(cx, |app, cx| app.attach_sync(cache, demo, window, cx));
                }
                let handle = view.read(cx).focus_handle().clone();
                window.focus(&handle, cx);
                cx.activate(true);
                view
            },
        )
        .expect("failed to open window");
    });
}
