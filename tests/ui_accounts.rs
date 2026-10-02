//! Account icons in the unified inbox and their picker on the settings Accounts page.

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext};
use mail_classifier::account_style::{COLORS, ICONS};

#[path = "ui_ext/harness.rs"]
#[allow(dead_code)]
mod harness;
use harness::{Harness, harness};

fn has_account_icon(h: &mut Harness<'_>, message: usize) -> bool {
    h.cx.update_window(h.window, |_, window, _| window.try_find(("row-account-icon", message)).is_some())
        .expect("window alive")
}

fn visible_ids(h: &mut Harness<'_>) -> Vec<usize> {
    h.visible().into_iter().map(|id| id as usize).collect()
}

#[gpui_kit::gpui::test]
fn rows_show_the_account_icon_only_in_all_inboxes(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let ids = visible_ids(&mut h);
    assert!(ids.iter().any(|&id| has_account_icon(&mut h, id)), "rendered unified rows carry one");

    // The sidebar's first account inbox: the account is implied, so no icon.
    h.click("1-0");
    let ids = visible_ids(&mut h);
    let rendered: Vec<usize> = ids.into_iter().filter(|&id| has_row(&mut h, id)).collect();
    assert!(!rendered.is_empty(), "the inbox renders rows");
    assert!(rendered.iter().all(|&id| !has_account_icon(&mut h, id)));
}

fn has_row(h: &mut Harness<'_>, message: usize) -> bool {
    h.cx.update_window(h.window, |_, window, _| window.try_find(("row", message)).is_some())
        .expect("window alive")
}

#[gpui_kit::gpui::test]
fn picking_an_icon_and_color_in_the_style_popover_stores_them_on_the_account(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let style = |h: &mut Harness<'_>| {
        h.read(|a| a.mailbox.accounts().iter().map(|a| (a.icon.clone(), a.color.clone())).collect::<Vec<_>>())
    };
    let before = style(&mut h);
    assert_eq!(before[0].0.as_deref(), Some("mail"));

    // Settings has a window of its own, and opens on Accounts.
    h.open_settings();
    // The picker only exists once the first account's tile has opened its popover.
    h.settings_click(("account-style", 0usize));
    h.cx.run_until_parked();
    let (icon, color) = (ICONS[3].0, COLORS[4]);
    h.settings_click(("account-icon", 3usize));
    h.settings_click(("account-color", 4usize));
    h.cx.run_until_parked();

    let after = style(&mut h);
    assert_eq!(after[0], (Some(icon.to_owned()), color.to_owned()));
    assert_eq!(after[1], before[1], "other accounts keep theirs");
}

fn sidebar_names(h: &mut Harness<'_>) -> Vec<String> {
    h.read(|a| a.mailbox.accounts().iter().map(|a| mail_classifier::account_style::display_name(a).to_owned()).collect())
}

#[gpui_kit::gpui::test]
fn typing_a_nickname_in_settings_renames_the_account_and_blank_restores_it(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    assert_eq!(sidebar_names(&mut h), vec!["Personal", "Work"]);

    h.open_settings();
    let focus_nickname = |h: &mut Harness<'_>| {
        let window = h.settings_window().expect("the settings window is open");
        h.cx.update_window(window, |_, window, cx| {
            window.within(("account-card", 0usize)).click(("account-rename", 0usize), cx)
        })
        .expect("settings window alive");
        h.cx.run_until_parked();
    };
    focus_nickname(&mut h);
    h.settings_type("  Home  ");
    assert_eq!(h.read(|a| a.mailbox.accounts()[0].nickname.clone()).as_deref(), Some("Home"));
    assert_eq!(sidebar_names(&mut h), vec!["Home", "Work"]);
    assert_eq!(h.read(|a| a.mailbox.accounts()[1].nickname.clone()), None, "other accounts untouched");

    // Clearing the text unsets it.
    h.settings_keys("cmd-a backspace");
    assert_eq!(h.read(|a| a.mailbox.accounts()[0].nickname.clone()), None);
    assert_eq!(sidebar_names(&mut h), vec!["Personal", "Work"]);
}

#[gpui_kit::gpui::test]
fn an_app_without_accounts_shows_the_add_gmail_onboarding(cx: &mut TestAppContext) {
    let store = std::rc::Rc::new(mail_classifier::contacts::ContactStore::open_in_memory().expect("store"));
    let empty = mail_classifier::model::Mailbox::from_parts(Vec::new(), Vec::new(), Vec::new(), store);
    let mut h = harness::harness_with(cx, empty);
    assert!(h.has("onboarding-add-gmail"), "no accounts: the list area offers to add one");

    let mut h = harness(h.cx);
    assert!(!h.has("onboarding-add-gmail"), "with accounts the onboarding is gone");
}
