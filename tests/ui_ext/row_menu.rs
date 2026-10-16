//! The row's classifier menu: opened from the row icons, it sets a label the classifier will
//! not overwrite, resets one back to the classifier's value, and is where a folder suggestion
//! is accepted or rejected.

use gpui_kit::TestAppContext;
use mail_classifier::judge::{Answer, AnswerValue, Kind, QuestionKey, Suggestion};
use mail_classifier::model::{Tag, TriageState};

use crate::harness::{Harness, harness_with, html_msg, mailbox, msg};

/// The five Kind rows of the menu, as the menu labels them.
const KINDS: [(&str, Kind); 5] = [
    ("Person", Kind::Person),
    ("Receipt", Kind::Receipt),
    ("Newsletter", Kind::Newsletter),
    ("Notification", Kind::Notification),
    ("Other", Kind::Other),
];

/// The Kind on message 1, if it carries one.
fn kind_of(h: &mut Harness<'_>) -> Option<Kind> {
    h.read(|a| {
        a.mailbox.tags(1).iter().find_map(|tag| match tag {
            Tag::Kind(kind) => Some(*kind),
            _ => None,
        })
    })
}

/// Open the classifier menu from the row's icon cluster.
fn open_row_menu(h: &mut Harness<'_>) {
    h.click(("row-menu", 1usize));
}

/// A message the stub judge labels `Possible spam`, so it carries the flag at startup.
fn spam_box() -> mail_classifier::model::Mailbox {
    mailbox(&[msg(1, 1, "deals@talentloop.com", "Is this a fit?", 1, "Inbox")])
}

/// A message with a folder suggestion waiting, seeded by hand: the offline stub never files mail.
fn seed_folder_suggestion(h: &mut Harness<'_>) {
    h.app.update(h.cx, |a, _| {
        let options = a.mailbox.folder_options("personal");
        a.mailbox.add_suggestions(vec![Suggestion {
            message: 1,
            key: QuestionKey::Folder,
            answer: Answer {
                probabilities: vec![1.0; options.len() + 1],
                value: AnswerValue::Choice(0),
                confidence: 0.9,
            },
        }]);
    });
}

#[gpui_kit::gpui::test]
fn the_row_icons_open_the_classifier_menu(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, spam_box());
    let flagged = h.has_tag(1, Tag::PossibleSpam);
    open_row_menu(&mut h);
    assert!(h.has_row("Kind"), "the Kind sections are there");
    assert!(h.has_row("Urgency"), "and Urgency");
    assert!(
        h.has_row(if flagged { "Not spam" } else { "Mark as spam" }),
        "and the spam flag, which way round follows the flag"
    );
    assert!(!h.has_row("Accept folder"), "nothing is suggested yet");
}

#[gpui_kit::gpui::test]
fn a_kind_set_in_the_menu_survives_reclassification(cx: &mut TestAppContext) {
    let mut h = harness_with(
        cx,
        mailbox(&[msg(1, 1, "receipts@stripe.com", "Receipt from X", 1, "Inbox")]),
    );
    let classified = kind_of(&mut h).expect("the stub labels a kind");
    let (label, chosen) = KINDS.into_iter().find(|(_, kind)| *kind != classified).unwrap();

    open_row_menu(&mut h);
    h.click_row(label);
    assert_eq!(kind_of(&mut h), Some(chosen), "the menu set the label");

    h.keys("c");
    assert_eq!(kind_of(&mut h), Some(chosen), "reclassification leaves the label alone");

    // One undo step puts the classifier's own value back.
    h.keys("u");
    assert_eq!(kind_of(&mut h), Some(classified));
}

#[gpui_kit::gpui::test]
fn reset_to_suggestion_restores_the_classifier_value(cx: &mut TestAppContext) {
    let mut h = harness_with(
        cx,
        mailbox(&[msg(1, 1, "receipts@stripe.com", "Receipt from X", 1, "Inbox")]),
    );
    let classified = kind_of(&mut h).expect("the stub labels a kind");
    let (label, _chosen) = KINDS.into_iter().find(|(_, kind)| *kind != classified).unwrap();

    open_row_menu(&mut h);
    h.click_row(label);
    open_row_menu(&mut h);
    h.click_row("Reset Kind to suggestion");
    assert_eq!(kind_of(&mut h), Some(classified), "the classifier's value is back");

    // Not sticky any more: the next classification may overwrite it again.
    h.keys("c");
    assert_eq!(kind_of(&mut h), Some(classified));
}

#[gpui_kit::gpui::test]
fn not_spam_clears_the_flag_and_sticks(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, spam_box());
    assert!(h.has_tag(1, Tag::PossibleSpam), "the stub flags it at startup");

    open_row_menu(&mut h);
    h.click_row("Not spam");
    assert!(!h.has_tag(1, Tag::PossibleSpam));

    h.keys("c");
    assert!(!h.has_tag(1, Tag::PossibleSpam), "reclassification respects the flag");

    h.keys("u");
    assert!(h.has_tag(1, Tag::PossibleSpam), "one undo step puts the flag back");
}

#[gpui_kit::gpui::test]
fn an_urgency_set_in_the_menu_survives_reclassification(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, spam_box());
    open_row_menu(&mut h);
    h.click_row("Very urgent");
    assert!(h.has_tag(1, Tag::Urgent(5)));

    h.keys("c");
    assert!(h.has_tag(1, Tag::Urgent(5)), "the hand-set level sticks");

    // "None" (level 1) clears the tag but still sticks.
    open_row_menu(&mut h);
    h.click_row("None");
    assert!(!h.has_tag(1, Tag::Urgent(5)));
    h.keys("c");
    assert!(
        h.read(|a| !a.mailbox.tags(1).iter().any(|t| matches!(t, Tag::Urgent(_)))),
        "a cleared urgency stays cleared"
    );
}

/// The path of the folder the seeded suggestion points at: the personal account's first option.
fn suggested_path(h: &mut Harness<'_>) -> String {
    h.read(|a| {
        let options = a.mailbox.folder_options("personal");
        a.mailbox.folder_path(options[0].0)
    })
}

#[gpui_kit::gpui::test]
fn the_folder_suggestion_is_accepted_from_the_row_menu(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, spam_box());
    seed_folder_suggestion(&mut h);
    let before = h.state_of(1);
    let path = suggested_path(&mut h);

    open_row_menu(&mut h);
    assert!(h.has_row(&format!("Accept folder \"{path}\"")), "the suggestion is named");
    h.click_row(&format!("Accept folder \"{path}\""));
    assert!(h.read(|a| a.mailbox.pending(1).is_empty()), "accepting clears the suggestion");
    assert_ne!(h.state_of(1), before, "accepting files the message");

    h.keys("u");
    assert_eq!(h.state_of(1), before, "one undo step reverts the move");
    assert!(!h.read(|a| a.mailbox.pending(1).is_empty()), "and brings the suggestion back");
}

#[gpui_kit::gpui::test]
fn the_folder_suggestion_is_rejected_from_the_row_menu(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, spam_box());
    seed_folder_suggestion(&mut h);
    let before = h.state_of(1);

    open_row_menu(&mut h);
    h.click_row("Reject");
    assert!(h.read(|a| a.mailbox.pending(1).is_empty()), "reject drops the suggestion");
    assert_eq!(h.state_of(1), before, "and files nothing");
    open_row_menu(&mut h);
    assert!(!h.has_row("Reject"), "the section is gone with it");
}

#[gpui_kit::gpui::test]
fn a_hand_set_label_is_one_undo_step(cx: &mut TestAppContext) {
    let mut h = harness_with(
        cx,
        mailbox(&[msg(1, 1, "alice@example.com", "Question", 1, "Inbox")]),
    );
    let classified = kind_of(&mut h);
    open_row_menu(&mut h);
    h.click_row("Newsletter");
    assert_eq!(kind_of(&mut h), Some(Kind::Newsletter));

    // The label and the archive are each their own step.
    h.keys("e");
    assert_eq!(h.state_of(1), TriageState::Archived);
    h.keys("u");
    assert_eq!(h.state_of(1), TriageState::Inbox, "the archive undoes first");
    assert_eq!(kind_of(&mut h), Some(Kind::Newsletter), "the label stays");
    h.keys("u");
    assert_eq!(kind_of(&mut h), classified, "the label came off in one step");
}

/// The row's right-click shows the reader `⋯` menu item for item: both build from `menu_items`.
#[gpui_kit::gpui::test]
fn the_row_right_click_opens_the_reader_menu(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox(&[html_msg()]));
    h.keys("enter");
    let id = h.read(|a| a.opened()).expect("a message is open");

    h.click(("btn-message-more", id as usize));
    let mut reader_menu = h.rows();
    h.keys("escape");
    assert!(h.rows().is_empty(), "the reader menu closed");

    h.right_click(("row", id as usize));
    let mut list_menu = h.rows();
    // The snapshot walk does not report rows in the order the menu draws them, so compare as
    // sets: the order itself comes from the one builder both menus share.
    list_menu.sort();
    reader_menu.sort();
    assert_eq!(list_menu, reader_menu, "the right-click menu is the reader's, item for item");
}
