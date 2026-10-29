//! The standalone "New folder" flow: pick an account (skipped with one), name it, enter.

use gpui_kit::TestAppContext;

#[path = "ui_ext/harness.rs"]
#[allow(dead_code)]
mod harness;
use harness::{Harness, harness, mailbox, msg, harness_with};

fn folder_names(h: &mut Harness<'_>, account: &str) -> Vec<String> {
    h.read(|a| a.mailbox.folders(account).iter().map(|f| f.name.clone()).collect())
}

fn run_new_folder(h: &mut Harness<'_>) {
    h.keys("cmd-k");
    h.type_text("New folder");
    h.keys("enter");
}

#[gpui_kit::gpui::test]
fn picks_the_second_account_then_names_the_folder(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let (personal, work) = (folder_names(&mut h, "personal"), folder_names(&mut h, "work"));
    run_new_folder(&mut h);
    assert!(h.read(|a| a.modal_open()), "the account picker is up");

    h.keys("down enter");
    h.type_text("Receipts");
    h.keys("enter");
    assert!(!h.read(|a| a.modal_open()));
    assert_eq!(folder_names(&mut h, "personal"), personal);
    let mut expected = work;
    expected.push("Receipts".into());
    assert_eq!(folder_names(&mut h, "work"), expected);

    // One user action, one undo step.
    h.keys("u");
    assert_eq!(folder_names(&mut h, "work").len(), expected.len() - 1);
}

#[gpui_kit::gpui::test]
fn blank_and_duplicate_names_create_nothing(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    run_new_folder(&mut h);
    h.keys("down enter");
    h.type_text("   ");
    h.keys("enter");
    assert!(h.read(|a| a.modal_open()), "a blank name keeps the prompt open");
    h.keys("escape");

    let work = folder_names(&mut h, "work");
    let existing = work.first().cloned().expect("the fixture has a work folder");
    run_new_folder(&mut h);
    h.keys("down enter");
    h.type_text(&existing.to_uppercase());
    h.keys("enter");
    assert_eq!(folder_names(&mut h, "work"), work, "no duplicate");
}

#[gpui_kit::gpui::test]
fn escape_at_either_step_creates_nothing(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let (personal, work) = (folder_names(&mut h, "personal"), folder_names(&mut h, "work"));

    run_new_folder(&mut h);
    h.keys("escape");
    assert!(!h.read(|a| a.modal_open()));

    run_new_folder(&mut h);
    h.keys("enter");
    h.type_text("Nope");
    h.keys("escape");
    assert!(!h.read(|a| a.modal_open()));

    assert_eq!(folder_names(&mut h, "personal"), personal);
    assert_eq!(folder_names(&mut h, "work"), work);
}

#[gpui_kit::gpui::test]
fn a_single_account_skips_the_picker(cx: &mut TestAppContext) {
    let mut mb = mailbox(&[msg(1, 1, "alice@example.com", "One", 1, "Inbox")]);
    mb.remove_account("work");
    let mut h = harness_with(cx, mb);
    let before = folder_names(&mut h, "personal");

    run_new_folder(&mut h);
    h.type_text("Solo");
    h.keys("enter");

    let mut expected = before;
    expected.push("Solo".into());
    assert_eq!(folder_names(&mut h, "personal"), expected);
}
