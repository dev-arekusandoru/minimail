//! Account icons in the unified inbox and their picker on the settings Accounts page.

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext};
use mail_classifier::account_style::ICONS;

#[path = "ui_ext/harness.rs"]
#[allow(dead_code)]
mod harness;
use harness::{Harness, harness};

/// Sidebar page index of Accounts.
const ACCOUNTS: usize = 5;

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
fn picking_an_icon_in_settings_stores_it_on_the_account(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let before = h.read(|a| a.mailbox.accounts().iter().map(|a| a.icon.clone()).collect::<Vec<_>>());
    assert_eq!(before[0].as_deref(), Some("mail"));

    h.keys("cmd-,");
    h.click(format!("0-{ACCOUNTS}"));
    // Item 1 is the first account's style picker; choose its fourth icon.
    let picked = ICONS[3].0;
    h.cx.update_window(h.window, |_, window, cx| {
        window.within("group-0").within("item-1").click(("account-icon", 3usize), cx)
    })
    .expect("window alive");
    h.cx.run_until_parked();

    let after = h.read(|a| a.mailbox.accounts().iter().map(|a| a.icon.clone()).collect::<Vec<_>>());
    assert_eq!(after[0].as_deref(), Some(picked));
    assert_eq!(after[1], before[1], "other accounts keep theirs");
}

fn sidebar_names(h: &mut Harness<'_>) -> Vec<String> {
    h.read(|a| a.mailbox.accounts().iter().map(|a| mail_classifier::account_style::display_name(a).to_owned()).collect())
}

#[gpui_kit::gpui::test]
fn typing_a_nickname_in_settings_renames_the_account_and_blank_restores_it(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    assert_eq!(sidebar_names(&mut h), vec!["Personal", "Work"]);

    h.keys("cmd-,");
    h.click(format!("0-{ACCOUNTS}"));
    let focus_nickname = |h: &mut Harness<'_>| {
        h.cx.update_window(h.window, |_, window, cx| {
            window.within("group-0").within("item-1").click(("account-nickname", 0usize), cx)
        })
        .expect("window alive");
        h.cx.run_until_parked();
    };
    focus_nickname(&mut h);
    h.type_text("  Home  ");
    assert_eq!(h.read(|a| a.mailbox.accounts()[0].nickname.clone()).as_deref(), Some("Home"));
    assert_eq!(sidebar_names(&mut h), vec!["Home", "Work"]);
    assert_eq!(h.read(|a| a.mailbox.accounts()[1].nickname.clone()), None, "other accounts untouched");

    // Clearing the text unsets it.
    h.keys("cmd-a backspace");
    assert_eq!(h.read(|a| a.mailbox.accounts()[0].nickname.clone()), None);
    assert_eq!(sidebar_names(&mut h), vec!["Personal", "Work"]);
}
