//! The row's classifier menu: opened from the row icons, it sets a label the classifier will
//! not overwrite, resets one back to the classifier's value, and is where a folder suggestion
//! is accepted or rejected.

use gpui_kit::TestAppContext;
use mail_classifier::app::menu::MenuItem;
use mail_classifier::judge::{Answer, AnswerValue, Kind, QuestionKey, Suggestion, urgency_label};
use mail_classifier::model::{Mailbox, MessageId, Tag, TriageState};

use crate::harness::{Harness, harness_with, html_msg, mailbox, msg};

/// Tag `id` NeedsReply through the classifier's own path, so clearing it is the first override.
fn tag_needs_reply(mb: &mut Mailbox, id: MessageId) {
    mb.add_suggestions(vec![Suggestion {
        message: id,
        key: QuestionKey::NeedsReply,
        answer: Answer {
            probabilities: vec![0.1, 0.9],
            value: AnswerValue::Bool(true),
            confidence: 0.9,
        },
    }]);
    mb.accept_suggestions(id, crate::harness::NOON);
}

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

/// A Kind's menu label, from the table the options are named by.
fn kind_label(kind: Kind) -> String {
    KINDS.iter().find(|(_, k)| *k == kind).expect("every Kind has a row").0.to_owned()
}

/// The label of the row that opens the Kind options: the section, and the value in force while
/// message 1 carries one.
fn kind_row(h: &mut Harness<'_>) -> String {
    kind_of(h).map_or_else(|| "Kind".to_owned(), |kind| format!("Kind · {}", kind_label(kind)))
}

/// The label of the row that opens the Urgency options, the same way.
fn urgency_row(h: &mut Harness<'_>) -> String {
    let level = h.read(|a| {
        a.mailbox.tags(1).iter().find_map(|tag| match tag {
            Tag::Urgent(level) => Some(*level),
            _ => None,
        })
    });
    level.map_or_else(
        || "Urgency".to_owned(),
        |level| format!("Urgency · {}", urgency_label(level)),
    )
}

/// The option rows of the classifier submenu whose label starts with `section` on message 1,
/// each with whether the menu marks it as the choice in force.
fn choices(h: &mut Harness<'_>, section: &str) -> Vec<(String, bool)> {
    h.read(|a| {
        a.classifier_items(1)
            .iter()
            .find_map(|item| match item {
                MenuItem::Submenu { label, items } if label.starts_with(section) => {
                    Some(items.clone())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("no classifier submenu for {section:?}"))
            .into_iter()
            .filter_map(|item| match item {
                MenuItem::Action { label, checked, .. } => Some((label.to_string(), checked)),
                _ => None,
            })
            .collect()
    })
}

/// The one option the `section` menu marks as the choice in force, if any.
fn marked(h: &mut Harness<'_>, section: &str) -> Option<String> {
    let mut marked: Vec<String> =
        choices(h, section).into_iter().filter(|(_, on)| *on).map(|(label, _)| label).collect();
    assert!(marked.len() <= 1, "at most one option can be the choice in force: {marked:?}");
    marked.pop()
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
    let row = kind_row(&mut h);
    assert!(h.has_row(&row), "the Kind submenu is there");
    let row = urgency_row(&mut h);
    assert!(h.has_row(&row), "and Urgency");
    assert!(!h.has_row("Person") && !h.has_row("Very urgent"), "their options are one level down");
    assert!(
        h.has_row(if flagged { "Not spam" } else { "Mark as spam" }),
        "and the spam flag, which way round follows the flag"
    );
    assert!(!h.has_row("Accept folder"), "nothing is suggested yet");
}

/// The classifier menus mark the option in force, and nothing while the label is unset. The
/// reader `⋯` and the row's right-click build from one place, so both mark it alike, and the
/// mark follows a choice and a reset.
#[gpui_kit::gpui::test]
fn the_classifier_menu_marks_the_option_in_force(cx: &mut TestAppContext) {
    let mut h = harness_with(
        cx,
        mailbox(&[msg(1, 1, "receipts@stripe.com", "Receipt from X", 1, "Inbox")]),
    );
    let classified = kind_of(&mut h).expect("the stub labels a kind");
    let named = kind_label(classified);
    assert_eq!(marked(&mut h, "Kind").as_deref(), Some(named.as_str()), "the Kind in force");
    assert_eq!(marked(&mut h, "Urgency"), None, "no urgency tag marks no option");

    // The reader `⋯` reaches the options through its Classifier labels submenu, naming the value
    // in force on the row that opens them.
    h.keys("enter");
    h.click(("btn-message-more", 1usize));
    h.hover_row("Classifier labels");
    assert!(h.has_row(&format!("Kind · {named}")), "the reader menu names the Kind");
    assert!(h.has_row("Urgency"), "an unset Urgency has no value to name");
    h.keys("escape");

    // The row's right-click shows the same rows, and choosing one moves the mark with it.
    h.right_click(("row", 1usize));
    h.hover_row("Classifier labels");
    assert!(h.has_row(&format!("Kind · {named}")), "the row menu names it too");
    let row = kind_row(&mut h);
    h.hover_row(&row);
    assert!(h.has_row("Person") && h.has_row("Receipt"), "the options are open");
    h.click_row("Person");
    assert_eq!(kind_of(&mut h), Some(Kind::Person), "the menu set the label");
    assert_eq!(marked(&mut h, "Kind").as_deref(), Some("Person"), "and the mark moved with it");

    // Clearing the Kind leaves the options unmarked, the row naming no value.
    h.right_click(("row", 1usize));
    h.hover_row("Classifier labels");
    assert!(h.has_row("Kind · Person"), "the trigger names the new choice");
    let row = kind_row(&mut h);
    h.hover_row(&row);
    h.click_row("No kind");
    assert_eq!(kind_of(&mut h), None);
    assert_eq!(marked(&mut h, "Kind"), None, "no Kind marks no option");
    h.right_click(("row", 1usize));
    h.hover_row("Classifier labels");
    assert!(h.has_row("Kind"), "and the trigger names no value");

    // Resetting restores the classifier's own Kind, marked and named again.
    let row = kind_row(&mut h);
    h.hover_row(&row);
    h.click_row("Reset Kind to suggestion");
    assert_eq!(kind_of(&mut h), Some(classified), "reset restored the classifier's own Kind");
    assert_eq!(marked(&mut h, "Kind").as_deref(), Some(named.as_str()));
    h.right_click(("row", 1usize));
    h.hover_row("Classifier labels");
    assert!(h.has_row(&format!("Kind · {named}")), "the trigger names it again");
}

/// Hovering a section opens its options in a nested menu; picking one runs it as a row of the
/// outer menu's action would.
#[gpui_kit::gpui::test]
fn the_classifier_menu_nests_the_kind_and_urgency_options(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, spam_box());
    open_row_menu(&mut h);

    let row = kind_row(&mut h);
    h.hover_row(&row);
    assert!(h.has_row("Person") && h.has_row("No kind"), "hovering Kind opens its options");
    assert!(!h.has_row("Very urgent"), "the Urgency options are still closed");
    h.click_row("Person");
    assert!(h.has_tag(1, Tag::Kind(Kind::Person)), "the nested option set the label");
}

/// The nested menus are keyboard-navigable: right steps into one, down moves inside it and enter
/// picks the highlighted row; left steps back out, where enter runs nothing.
#[gpui_kit::gpui::test]
fn the_kind_submenu_is_keyboard_navigable(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, spam_box());

    open_row_menu(&mut h);
    let row = kind_row(&mut h);
    h.hover_row(&row);
    h.keys("right");
    assert!(h.has_row("Person"), "right stepped into the submenu");
    h.keys("down");
    h.keys("enter");
    assert!(h.has_tag(1, Tag::Kind(Kind::ALL[1])), "down moved inside it and enter picked");

    open_row_menu(&mut h);
    let row = kind_row(&mut h);
    h.hover_row(&row);
    h.keys("right");
    h.keys("left");
    h.keys("enter");
    assert!(h.has_tag(1, Tag::Kind(Kind::ALL[1])), "left left the submenu, so enter ran nothing");
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
    let row = kind_row(&mut h);
    h.hover_row(&row);
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
    let row = kind_row(&mut h);
    h.hover_row(&row);
    assert!(!h.has_row("Reset Kind to suggestion"), "automatic labels have nothing to reset");
    h.click_row(label);
    open_row_menu(&mut h);
    let row = kind_row(&mut h);
    h.hover_row(&row);
    assert!(h.has_row("Reset Kind to suggestion"), "manual labels can be reset");
    h.click_row("Reset Kind to suggestion");
    assert_eq!(kind_of(&mut h), Some(classified), "the classifier's value is back");
    open_row_menu(&mut h);
    let row = kind_row(&mut h);
    h.hover_row(&row);
    assert!(!h.has_row("Reset Kind to suggestion"), "reset removes the override");
    h.keys("escape");

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
    h.keys("escape"); // decline "Never mark mail from … as spam?"; keys must still work

    h.keys("c");
    assert!(!h.has_tag(1, Tag::PossibleSpam), "reclassification respects the flag");
    h.keys("u");
    assert!(h.has_tag(1, Tag::PossibleSpam), "one undo step puts the flag back");
}

#[gpui_kit::gpui::test]
fn an_urgency_set_in_the_menu_survives_reclassification(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, spam_box());
    open_row_menu(&mut h);
    let row = urgency_row(&mut h);
    h.hover_row(&row);
    assert!(!h.has_row("Reset Urgency to suggestion"));
    h.click_row("Very urgent");
    assert!(h.has_tag(1, Tag::Urgent(5)));

    h.keys("c");
    assert!(h.has_tag(1, Tag::Urgent(5)), "the hand-set level sticks");

    // "None" (level 1) clears the tag but still sticks.
    open_row_menu(&mut h);
    let row = urgency_row(&mut h);
    h.hover_row(&row);
    assert!(h.has_row("Reset Urgency to suggestion"));
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
    let row = kind_row(&mut h);
    h.hover_row(&row);
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

/// The needs-reply flag can be cleared from the shared menu, which carries the row only while
/// the message has the flag; the clear is one user override and one undo step.
#[gpui_kit::gpui::test]
fn the_shared_menu_clears_needs_reply_only_while_it_is_set(cx: &mut TestAppContext) {
    let mut mb = mailbox(&[msg(1, 1, "alice@example.com", "Question", 1, "Inbox")]);
    tag_needs_reply(&mut mb, 1);
    let mut h = harness_with(cx, mb);
    assert!(h.has_tag(1, Tag::NeedsReply), "the classifier tagged it");

    h.right_click(("row", 1usize));
    h.hover_row("Classifier labels");
    assert!(h.has_row("Doesn't need reply"), "the clear row is there while the flag is set");
    h.click_row("Doesn't need reply");
    assert!(!h.has_tag(1, Tag::NeedsReply), "the row cleared the flag");
    assert!(
        h.read(|a| a.mailbox.is_overridden(1, QuestionKey::NeedsReply)),
        "the clear is a user override, so reclassification cannot put it back"
    );

    h.right_click(("row", 1usize));
    h.hover_row("Classifier labels");
    assert!(!h.has_row("Doesn't need reply"), "and the row goes with the flag");
    h.keys("escape");

    // The reader's `⋯` builds the same rows, so the restored flag is clearable from there too.
    h.keys("u");
    assert!(h.has_tag(1, Tag::NeedsReply), "one undo step brings the flag back");
    h.keys("enter");
    h.click(("btn-message-more", 1usize));
    h.hover_row("Classifier labels");
    assert!(h.has_row("Doesn't need reply"), "the reader menu shares the row");
}

#[gpui_kit::gpui::test]
fn escape_on_a_menu_launched_dialog_returns_focus_to_the_app(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, spam_box());
    open_row_menu(&mut h);
    h.click_row("Not spam");
    assert!(h.read(|a| a.dialog_open()));
    h.keys("escape");
    assert!(!h.read(|a| a.dialog_open()));
    h.keys("u");
    assert!(h.has_tag(1, Tag::PossibleSpam), "u undoes: the app has keyboard focus");
}
