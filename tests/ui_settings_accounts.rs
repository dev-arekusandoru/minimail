//! The Accounts settings page: removing an account asks first, and each card's footer offers
//! exactly the action its sync status allows. Everything here runs against fake accounts and
//! fake sync facts; nothing touches the network.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::base::test_support::snapshots;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, ElementId, Focusable as _, Point, TestAppContext, Window,
    WindowBounds, WindowOptions, px, size,
};
use mail_classifier::app::mail_app::panes::Orientation;
use mail_classifier::app::settings::{AccountRow, SettingsEvent, SettingsPanel};
use mail_classifier::judge::JudgePolicy;
use mail_classifier::contacts::ContactStore;
use mail_classifier::model::{Account, Mailbox, ProviderKind};
use mail_classifier::sync_status::SyncAction;

#[path = "ui_ext/harness.rs"]
#[allow(dead_code)]
mod harness;
use harness::{Harness, harness_with};

/// The Gmail account every app-level test starts with: linked, but with no working token.
const GMAIL_ID: &str = "gmail:you@gmail.com";

/// A mailbox holding one linked Gmail account and nothing else, so its card is the only one on
/// the page. The account has no stored token: the app calls that an expired sign-in.
fn with_gmail() -> Mailbox {
    let store = Rc::new(ContactStore::open_in_memory().expect("contact store"));
    Mailbox::from_parts(
        Vec::new(),
        vec![Account {
            id: GMAIL_ID.to_owned(),
            name: "you@gmail.com".to_owned(),
            email: "you@gmail.com".to_owned(),
            color: "#56b6c2".to_owned(),
            icon: Some("mail".to_owned()),
            nickname: None,
            provider: ProviderKind::Gmail,
        }],
        Vec::new(),
        store,
    )
}

fn app(cx: &mut TestAppContext) -> Harness<'_> {
    harness_with(cx, with_gmail())
}


fn click(cx: &mut TestAppContext, window: AnyWindowHandle, id: impl Into<ElementId>) {
    let id = id.into();
    cx.update_window(window, |_, window, cx| window.click(id, cx)).expect("window alive");
    cx.run_until_parked();
}

fn has(cx: &mut TestAppContext, window: AnyWindowHandle, id: impl Into<ElementId>) -> bool {
    let id = id.into();
    cx.update_window(window, |_, window, _| window.try_find(id).is_some()).expect("window alive")
}

/// Every string the last completed frame of `window` draws.
fn text(cx: &mut TestAppContext, window: AnyWindowHandle) -> String {
    cx.update_window(window, |_, window, _| {
        snapshots(window).iter().filter_map(|row| row.label()).collect::<Vec<_>>().join(" ")
    })
    .expect("window alive")
}

/// The rows of a popup menu open in `window`, as (the view drawing it, index, label).
fn menu_rows(window: &Window) -> Vec<(ElementId, usize, String)> {
    let popup: ElementId = "popup-menu".into();
    snapshots(window)
        .into_iter()
        .filter_map(|row| {
            let path = row.path();
            let at = path.iter().rposition(|id| *id == popup)?;
            let ElementId::Integer(ix) = *path.last()? else { return None };
            if at == 0 || path.len() != at + 4 {
                return None;
            }
            Some((path[at - 1].clone(), usize::try_from(ix).ok()?, row.label()?.to_owned()))
        })
        .collect()
}

/// Click the row labelled `label` of the popup menu open in `window`.
fn click_menu_row(cx: &mut TestAppContext, window: AnyWindowHandle, label: &str) {
    cx.update_window(window, |_, window, cx| {
        let (menu, ix, _) = menu_rows(window)
            .into_iter()
            .find(|(_, _, text)| text == label)
            .unwrap_or_else(|| panic!("no menu row labelled {label:?}"));
        window.within(menu).within("popup-menu").click(ix, cx);
    })
    .expect("window alive");
    cx.run_until_parked();
}

fn click_dialog_button(cx: &mut TestAppContext, window: AnyWindowHandle, button: &'static str) {
    cx.update_window(window, |_, window, cx| window.within("dialog").click(button, cx))
        .expect("window alive");
    cx.run_until_parked();
}

/// Complete a frame, so the element registry matches what is on screen.
fn redraw(cx: &mut TestAppContext, window: AnyWindowHandle) {
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| window.draw(cx).clear(cx)).expect("window alive");
}

/// A dialog slides in for 250ms and its controls move meanwhile; wait that out before the
/// next click aims at them.
fn settle(cx: &mut TestAppContext, window: AnyWindowHandle) {
    cx.run_until_parked();
    std::thread::sleep(std::time::Duration::from_millis(300));
    cx.update_window(window, |_, window, cx| window.draw(cx).clear(cx)).expect("window alive");
}

fn linked(h: &mut Harness<'_>) -> Vec<String> {
    h.read(|a| a.mailbox.accounts().iter().map(|a| a.id.clone()).collect())
}

/// Keep the OAuth client out of the environment, so a sign-in can never leave the app.
fn without_oauth_client() {
    unsafe {
        std::env::remove_var("MAIL_CLASSIFIER_GOOGLE_CLIENT_ID");
        std::env::remove_var("MAIL_CLASSIFIER_GOOGLE_CLIENT_SECRET");
    }
}

#[gpui_kit::gpui::test]
fn removing_an_account_asks_first_and_cancel_keeps_it(cx: &mut TestAppContext) {
    let mut h = app(cx);
    let before = linked(&mut h);
    let settings = h.open_settings();
    h.settings_click(("account-overflow", 0usize));
    click_menu_row(&mut *h.cx, settings, "Remove…");
    settle(&mut *h.cx, settings);

    assert!(h.settings_has("dialog"), "removing asks first");
    let shown = text(&mut *h.cx, settings);
    assert!(shown.contains("Remove"), "the dialog offers Remove: {shown}");
    assert!(shown.contains("Cancel"), "and Cancel: {shown}");
    assert_eq!(linked(&mut h), before, "the account is still linked while it asks");

    click_dialog_button(&mut *h.cx, settings, "cancel");
    redraw(&mut *h.cx, settings);
    assert_eq!(linked(&mut h), before, "Cancel keeps the account");
    assert!(!h.settings_has("dialog"), "and the dialog closes");
}

#[gpui_kit::gpui::test]
fn an_expired_sign_in_offers_signing_in_again_and_keeps_the_account(cx: &mut TestAppContext) {
    without_oauth_client();
    let mut h = app(cx);
    let before = linked(&mut h);
    h.open_settings();
    // No stored token for the linked account: the app calls that an expired sign-in, and the
    // card's one action is a sign-in for that same account.
    assert!(h.settings_has(("account-sign-in", 0usize)), "an expired sign-in offers one");
    assert!(!h.settings_has(("account-retry", 0usize)), "a sign-in is not a failed round");

    h.settings_click(("account-sign-in", 0usize));
    assert_eq!(linked(&mut h), before, "signing in again neither adds nor drops an account");
    assert_eq!(h.read(|a| a.mailbox.account(GMAIL_ID).unwrap().nickname.clone()), None, "the nickname stays unset");
    assert!(
        h.toast().contains("MAIL_CLASSIFIER_GOOGLE_CLIENT_ID"),
        "that account's own Gmail sign-in ran, and asked for the OAuth client: {}",
        h.toast()
    );
}

#[gpui_kit::gpui::test]
fn the_add_button_starts_a_sign_in_only_with_an_oauth_client(cx: &mut TestAppContext) {
    let (window, seen) = panel(cx, Vec::new(), false);
    assert!(has(cx, window, "account-add-gmail"), "the page offers to add a Gmail account");
    click(cx, window, "account-add-gmail");
    assert!(
        seen.borrow().is_empty(),
        "a sign-in that cannot work is not started: {:?}",
        seen.borrow()
    );

    let (window, seen) = panel(cx, Vec::new(), true);
    click(cx, window, "account-add-gmail");
    assert!(
        seen.borrow().iter().any(|e| matches!(e, SettingsEvent::AddGmail)),
        "with the OAuth client configured it starts one: {:?}",
        seen.borrow()
    );
}

/// A settings window holding one card per sync fact, plus the events the panel emits.
fn panel(
    cx: &mut TestAppContext,
    rows: Vec<AccountRow>,
    gmail_configured: bool,
) -> (AnyWindowHandle, Rc<RefCell<Vec<SettingsEvent>>>) {
    cx.update(gpui_kit::init);
    let seen: Rc<RefCell<Vec<SettingsEvent>>> = Rc::new(RefCell::new(Vec::new()));
    let window = {
        let seen = seen.clone();
        cx.update(|cx| {
            let (window, _panel) = gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: Point::default(),
                        size: size(px(820.), px(650.)),
                    })),
                    ..Default::default()
                },
                cx,
                move |window, cx| {
                    let panel = cx.new(|cx| {
                        SettingsPanel::new(
                            JudgePolicy::default(),
                            false,
                            false,
                            2,
                            Orientation::SideBySide,
                            window,
                            cx,
                        )
                        .accounts(rows, gmail_configured)
                    });
                    cx.subscribe(&panel, move |_, event: &SettingsEvent, _| {
                        seen.borrow_mut().push(event.clone());
                    })
                    .detach();
                    let focus = panel.read(cx).focus_handle(cx).clone();
                    window.focus(&focus, cx);
                    panel
                },
            )
            .expect("panel window");
            window.downcast::<gpui_kit::base::Root>().expect("root").into()
        })
    };
    cx.run_until_parked();
    (window, seen)
}

/// A linked account in the sync state `action` came from.
fn row(id: &str, status: &str, problem: bool, action: Option<SyncAction>) -> AccountRow {
    AccountRow {
        id: id.to_owned(),
        name: id.to_owned(),
        email: format!("{id}@example.com"),
        color: "#61afef".to_owned(),
        icon: "mail",
        nickname: String::new(),
        gmail: true,
        sync: status.to_owned(),
        sync_problem: problem,
        sync_action: action,
    }
}

#[gpui_kit::gpui::test]
fn the_footer_offers_the_one_action_its_status_allows(cx: &mut TestAppContext) {
    let (window, seen) = panel(
        cx,
        vec![
            row("expired", "Sign-in expired", true, Some(SyncAction::SignInAgain)),
            row("failed", "Sync failed: the server said no", true, Some(SyncAction::Retry)),
            row("healthy", "Synced 2 min ago", false, None),
        ],
        true,
    );

    assert!(has(cx, window, ("account-sign-in", 0usize)), "an expired sign-in offers a sign-in");
    assert!(has(cx, window, ("account-retry", 1usize)), "a failed round offers a retry");
    assert!(!has(cx, window, ("account-retry", 0usize)), "and only that one");
    assert!(!has(cx, window, ("account-sign-in", 1usize)), "one action, never two");
    assert!(!has(cx, window, ("account-sign-in", 2usize)), "a healthy account offers nothing");
    assert!(!has(cx, window, ("account-retry", 2usize)), "a healthy account offers nothing");

    click(cx, window, ("account-retry", 1usize));
    click(cx, window, ("account-sign-in", 0usize));
    let events = seen.borrow().clone();
    assert!(
        events.iter().any(|e| matches!(e, SettingsEvent::SyncAction { id, action: SyncAction::Retry } if id == "failed")),
        "Retry asks to run that account's sync again: {events:?}"
    );
    assert!(
        events.iter().any(
            |e| matches!(e, SettingsEvent::SyncAction { id, action: SyncAction::SignInAgain } if id == "expired")
        ),
        "Sign in again asks for that same account's sign-in: {events:?}"
    );
}

#[gpui_kit::gpui::test]
fn the_rename_pencil_focuses_the_nickname_input(cx: &mut TestAppContext) {
    let (window, seen) = panel(cx, vec![row("work", "Synced just now", false, None)], true);
    // Typing before the pencil goes nowhere near the nickname.
    cx.update_window(window, |_, window, cx| window.input("Home", cx)).expect("window alive");
    cx.run_until_parked();
    assert!(seen.borrow().is_empty(), "the title only takes text once it has focus");

    click(cx, window, ("account-rename", 0usize));
    cx.update_window(window, |_, window, cx| window.input("Home", cx)).expect("window alive");
    cx.run_until_parked();
    assert!(
        seen.borrow()
            .iter()
            .any(|e| matches!(e, SettingsEvent::AccountNickname { id, nickname } if id == "work" && nickname == "Home")),
        "the pencil focuses the nickname, so typing renames the account: {:?}",
        seen.borrow()
    );
}
