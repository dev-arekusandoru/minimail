use gpui_kit::TestAppContext;
use mail_classifier::model::Mailbox;
use mail_classifier::model::TriageState::*;
use crate::harness::{Harness, harness_with, mailbox, msg};

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

/// Sender-wide archive from the Inbox cursor, then bring one message back so it can be repeated.
/// The sender-wide keys confirm first, so `1` answers the dialog.
pub fn repeat_sender_archive(h: &mut Harness<'_>) {
    h.keys("shift-e 1 g a i");
    h.click(0usize);
}

#[gpui_kit::gpui::test]
pub fn second_sender_wide_action_suggests_rule_and_accept_applies(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, rules_box());
    h.keys("shift-e 1");
    assert_eq!(h.read(|a| a.pending_rule()), None, "no suggestion after the first action");
    assert_eq!(h.count(Archived), 3);

    repeat_sender_archive(&mut h);
    assert_eq!(h.read(|a| a.pending_rule()), None);
    h.keys("shift-e 1");
    let rule = h.read(|a| a.pending_rule()).expect("suggested on the 2nd identical action");
    assert_eq!(rule.sender, "sam@news.io");
    assert_eq!(rule.state, Archived);

    // Bring another message from the sender back, then accept: rule applies to it.
    h.keys("g a");
    let id = h.cursor().unwrap();
    h.keys("i");
    assert_eq!(h.state_of(id), Inbox);
    h.keys("shift-y");
    assert_eq!(h.read(|a| a.pending_rule()), None);
    assert_eq!(h.read(|a| a.rules.rule_for("sam@news.io")), Some(Archived));
    assert_eq!(h.state_of(id), Archived, "accepted rule is applied to the sender's inbox mail");
    h.assert_invariant("accept rule");
}

#[gpui_kit::gpui::test]
pub fn dismissed_rule_is_never_suggested_again(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, rules_box());
    h.keys("shift-e 1");
    repeat_sender_archive(&mut h);
    h.keys("shift-e 1");
    assert!(h.read(|a| a.pending_rule()).is_some());
    h.keys("shift-n");
    assert_eq!(h.read(|a| a.pending_rule()), None);
    assert_eq!(h.read(|a| a.rules.rule_for("sam@news.io")), None);

    for _ in 0..3 {
        repeat_sender_archive(&mut h);
        h.keys("shift-e 1");
        assert_eq!(h.read(|a| a.pending_rule()), None, "dismissed rules stay dismissed");
    }
    assert!(h.read(|a| a.rules.rules().is_empty()));
}

#[gpui_kit::gpui::test]
pub fn rules_panel_revokes_accepted_rule(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, rules_box());
    h.keys("shift-e 1");
    repeat_sender_archive(&mut h);
    h.keys("shift-e 1 shift-y");
    assert_eq!(h.read(|a| a.rules.rules().len()), 1);

    h.keys("shift-r");
    assert!(h.read(|a| a.rules_open()));
    h.keys("backspace");
    assert!(h.read(|a| a.rules.rules().is_empty()), "backspace revokes the selected rule");
    assert_eq!(h.read(|a| a.rules.rule_for("sam@news.io")), None);
    h.keys("escape");
    assert!(!h.read(|a| a.rules_open()));
}
