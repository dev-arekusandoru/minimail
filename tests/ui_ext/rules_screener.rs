use gpui_kit::{TestAppContext};
use mail_classifier::model::{Mailbox};
use mail_classifier::model::TriageState::*;
use crate::harness::{Harness, harness_with, json, mailbox, msg};
use mail_classifier::contacts::{ContactStore, NewContact};

// ---------------------------------------------------------------- Rule suggestions

pub fn rules_box() -> Mailbox {
    mailbox(&[
        msg(1, 1, "sam@news.io", "n1", 25, "Inbox"),
        msg(2, 2, "sam@news.io", "n2", 24, "Inbox"),
        msg(3, 3, "sam@news.io", "n3", 23, "Inbox"),
        msg(4, 4, "tom@news.io", "t1", 22, "Inbox"),
        msg(5, 5, "tom@news.io", "t2", 21, "Inbox"),
        msg(6, 6, "zed@news.io", "z1", 20, "Inbox"),
    ])
}

/// Sender-wide Done from the Inbox cursor, then bring one message back so it can be repeated.
pub fn repeat_sender_done(h: &mut Harness<'_>) {
    h.keys("shift-e 4 i 1");
}

#[gpui_kit::gpui::test]
pub fn second_sender_wide_action_suggests_rule_and_accept_applies(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, rules_box());
    h.keys("shift-e");
    assert_eq!(h.read(|a| a.pending_rule()), None, "no suggestion after the first action");
    assert_eq!(h.count(Done), 3);

    repeat_sender_done(&mut h);
    assert_eq!(h.read(|a| a.pending_rule()), None);
    h.keys("shift-e");
    let rule = h.read(|a| a.pending_rule()).expect("suggested on the 2nd identical action");
    assert_eq!(rule.sender, "sam@news.io");
    assert_eq!(rule.state, Done);

    // Bring another message from the sender back, then accept: rule applies to it.
    h.keys("4");
    let id = h.cursor().unwrap();
    h.keys("i 1");
    assert_eq!(h.state_of(id), Inbox);
    h.keys("shift-y");
    assert_eq!(h.read(|a| a.pending_rule()), None);
    assert_eq!(h.read(|a| a.rules.rule_for("sam@news.io")), Some(Done));
    assert_eq!(h.state_of(id), Done, "accepted rule is applied to the sender's inbox mail");
    h.assert_invariant("accept rule");
}

#[gpui_kit::gpui::test]
pub fn dismissed_rule_is_never_suggested_again(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, rules_box());
    h.keys("shift-e");
    repeat_sender_done(&mut h);
    h.keys("shift-e");
    assert!(h.read(|a| a.pending_rule()).is_some());
    h.keys("shift-n");
    assert_eq!(h.read(|a| a.pending_rule()), None);
    assert_eq!(h.read(|a| a.rules.rule_for("sam@news.io")), None);

    for _ in 0..3 {
        repeat_sender_done(&mut h);
        h.keys("shift-e");
        assert_eq!(h.read(|a| a.pending_rule()), None, "dismissed rules stay dismissed");
    }
    assert!(h.read(|a| a.rules.rules().is_empty()));
}

#[gpui_kit::gpui::test]
pub fn rules_panel_revokes_accepted_rule(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, rules_box());
    h.keys("shift-e");
    repeat_sender_done(&mut h);
    h.keys("shift-e shift-y");
    assert_eq!(h.read(|a| a.rules.rules().len()), 1);

    h.keys("shift-r");
    assert!(h.read(|a| a.rules_open()));
    h.keys("backspace");
    assert!(h.read(|a| a.rules.rules().is_empty()), "backspace revokes the selected rule");
    assert_eq!(h.read(|a| a.rules.rule_for("sam@news.io")), None);
    h.keys("escape");
    assert!(!h.read(|a| a.rules_open()));
}

// ---------------------------------------------------------------- Screener

pub fn screener_box() -> Mailbox {
    let msgs = json(&[
        msg(1, 1, "known@a.io", "k1", 25, "Inbox"),
        msg(2, 2, "new1@a.io", "n1a", 24, "Inbox"),
        msg(3, 3, "new1@a.io", "n1b", 23, "Inbox"),
        msg(4, 4, "new2@a.io", "n2", 22, "Inbox"),
    ]);
    let store = ContactStore::open_in_memory().unwrap();
    store.create(NewContact::from_email("known@a.io", "Known")).unwrap();
    Mailbox::from_json_with_contacts(&msgs, std::rc::Rc::new(store)).unwrap()
}

#[gpui_kit::gpui::test]
pub fn screener_allow_and_block(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, screener_box());
    assert_eq!(h.count(Inbox), 1, "unscreened mail is not in the inbox");
    h.assert_invariant("start");

    h.keys("5");
    assert!(h.read(|a| a.screener_open()));
    assert_eq!(h.visible(), vec![2, 3, 4]);

    // Allow: the whole sender moves to the inbox.
    h.keys("a");
    assert_eq!(h.visible(), vec![4]);
    assert_eq!(h.count(Inbox), 3);
    h.assert_invariant("allow");

    // Block: hidden, not deleted.
    h.keys("b");
    assert!(h.visible().is_empty());
    assert_eq!(h.total(), 4);
    assert_eq!(h.read(|a| a.mailbox.hidden_count()), 1);
    assert_eq!(h.count(Inbox), 3);
    h.assert_invariant("block");

    // Undo restores the blocked sender to the screener.
    h.keys("u");
    assert_eq!(h.visible(), vec![4]);
    h.assert_invariant("undo block");
}
