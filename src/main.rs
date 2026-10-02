use gpui_kit::*;
use mail_classifier::app::{MailApp, actions};
use mail_classifier::model::Mailbox;
use mail_classifier::prefs::Scope;
use mail_classifier::sync::cache::{Cache, default_cache_path};
use mail_classifier::{app_settings, theme};
use std::rc::Rc;

fn main() {
    let store = match mail_classifier::contacts::open_default() {
        Ok(store) => Rc::new(store),
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
        let scope = Scope::Global;
        let mode = app_settings::THEME_MODE.get(store.as_ref(), &scope);
        let light = app_settings::LIGHT_THEME.get(store.as_ref(), &scope);
        let dark = app_settings::DARK_THEME.get(store.as_ref(), &scope);
        let light = if light.is_empty() {
            theme::names_for(cx, true).first().map(ToString::to_string).unwrap_or_default()
        } else {
            light
        };
        let dark = if dark.is_empty() { app_settings::DEFAULT_DARK_THEME.to_owned() } else { dark };
        let system_is_dark = matches!(cx.window_appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark);
        let active = theme::active_theme_name(mode, &light, &dark, system_is_dark);
        theme::apply(cx, active);
        gpui_kit::open_window(
            gpui_kit::component::TitleBar::window_options(),
            cx,
            move |window, cx| {
                let cache = match Cache::open(&default_cache_path()) {
                    Ok(cache) => Some(Rc::new(cache)),
                    Err(e) => {
                        eprintln!("mail cache unavailable: {e}");
                        None
                    }
                };
                let cached = cache.as_ref().and_then(|c| match c.load() {
                    Ok((accounts, folders, messages)) if !accounts.is_empty() => Some((accounts, folders, messages)),
                    Ok(_) => None,
                    Err(e) => {
                        eprintln!("mail cache unreadable: {e}");
                        None
                    }
                });
                let mailbox = match cached {
                    Some((accounts, folders, messages)) => {
                        Mailbox::from_parts(messages, accounts, folders, store.clone())
                    }
                    None => Mailbox::from_parts(Vec::new(), Vec::new(), Vec::new(), store.clone()),
                };
                let prefs = store.clone();
                let view = cx.new(|cx| {
                    let mut app = MailApp::new(mailbox, window, cx);
                    app.load_preferences(prefs, window, cx);
                    app
                });
                if let Some(cache) = cache {
                    view.update(cx, |app, cx| app.attach_sync(cache, window, cx));
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
