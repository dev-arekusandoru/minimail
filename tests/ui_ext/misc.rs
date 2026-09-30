use gpui_kit::{TestAppContext};
use mail_classifier::app::actions::{commands};
use mail_classifier::clock::{DAY};
use mail_classifier::judge::{Mode, QuestionKey};
use mail_classifier::model::{Mailbox, Tag};
use mail_classifier::model::TriageState::*;
use crate::harness::{Harness, harness, harness_with, mailbox, msg};
use crate::session_search::{search, search_box};

// ---------------------------------------------------------------- Settings / summaries

#[gpui_kit::gpui::test]
pub fn settings_switch_question_to_review(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let auto = |h: &mut Harness<'_>| {
        h.read(|a| matches!(a.policy.mode(QuestionKey::Spam), Mode::Auto { .. }))
    };
    assert!(auto(&mut h), "Spam defaults to Auto");
    h.keys("cmd-,");
    assert!(h.read(|a| a.settings_open()));
    h.keys("space");
    assert!(!auto(&mut h), "space toggles the selected question to Review");
    h.keys("space");
    assert!(auto(&mut h), "and back to Auto");
    h.keys("escape");
    assert!(!h.read(|a| a.settings_open()));
}

/// A mailbox whose only inbox mail is an obvious spam message (id 1), classified on startup.
pub fn spam_box() -> Mailbox {
    mailbox(&[
        msg(1, 1, "recruiter@talentloop.com", "quick question", 25, "Inbox"),
        msg(2, 2, "friend@a.io", "lunch", 24, "Inbox"),
    ])
}

/// Undo startup auto-labels until message 1 is a plain Inbox message again.
pub fn reset_spam(h: &mut Harness<'_>) {
    for _ in 0..20 {
        if h.state_of(1) == Inbox && !h.has_tag(1, Tag::PossibleSpam) {
            return;
        }
        h.keys("u");
    }
    panic!("could not restore message 1");
}

#[gpui_kit::gpui::test]
pub fn spam_in_auto_mode_is_applied(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, spam_box());
    assert!(h.has_tag(1, Tag::PossibleSpam), "startup auto-applies the spam label");
    assert_eq!(h.state_of(1), Inbox);
    reset_spam(&mut h);
    assert!(h.read(|a| a.mailbox.pending(1).iter().all(|s| s.key != QuestionKey::Spam)));
    h.keys("c");
    assert!(h.has_tag(1, Tag::PossibleSpam), "Auto: c applies the PossibleSpam tag");
    assert_eq!(h.state_of(1), Inbox);
}

#[gpui_kit::gpui::test]
pub fn spam_in_review_mode_is_queued_not_applied(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, spam_box());
    reset_spam(&mut h);
    h.keys("cmd-, space escape");
    assert!(h.read(|a| matches!(a.policy.mode(QuestionKey::Spam), Mode::Review)));
    h.keys("c");
    assert!(!h.has_tag(1, Tag::PossibleSpam), "Review: no auto-applied label");
    assert_eq!(h.state_of(1), Inbox);
    assert!(
        h.read(|a| a.mailbox.pending(1).iter().any(|s| s.key == QuestionKey::Spam)),
        "Review: a pending Spam suggestion (badge) is queued"
    );
    // Accepting it applies the label.
    h.keys("y");
    assert!(h.has_tag(1, Tag::PossibleSpam));
    assert_eq!(h.state_of(1), Inbox);
}

#[gpui_kit::gpui::test]
pub fn search_excludes_muted_threads(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, search_box());
    assert_eq!(search(&mut h, "roadmap"), vec![2]);
    h.goto(2);
    h.keys("m");
    assert_eq!(h.read(|a| a.mailbox.hidden_count()), 1);
    assert!(search(&mut h, "roadmap").is_empty(), "muted mail is not searchable");
    assert_eq!(search(&mut h, "from:dana"), vec![1]);
}

#[gpui_kit::gpui::test]
pub fn summary_is_opt_in(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    assert!(!h.read(|a| a.summaries_enabled));
    h.keys("z");
    assert!(h.toast().contains("Enable summaries"), "toast was {:?}", h.toast());
    assert!(h.read(|a| a.summary_shown()).is_none());

    h.keys("cmd-,");
    h.click(("settings-section", 0usize));
    h.click("summaries-row");
    assert!(h.read(|a| a.summaries_enabled));
    h.keys("escape");
    assert!(!h.read(|a| a.settings_open()));
    h.keys("z");
    let summary = h.read(|a| a.summary_shown()).expect("summary shown after enabling");
    assert!(!summary.summary.is_empty());
}

// ---------------------------------------------------------------- Palette / help

/// Keys introduced by the v2 features.
pub const NEW_KEYS: [&str; 21] = [
    "e", "d", "i", "shift-e", "shift-d", "shift-i", "s", "z", "ctrl-g", "y", "n",
    "shift-y", "shift-n", "shift-r", "5", "m", "shift-u", "cmd-,", "t", "/", "c",
];

#[gpui_kit::gpui::test]
pub fn every_new_command_is_in_palette_search_and_help(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let specs = commands();
    let mut names = Vec::new();
    for key in NEW_KEYS {
        let spec = specs
            .iter()
            .find(|c| c.key == key)
            .unwrap_or_else(|| panic!("no command with key {key:?}"));
        names.push(spec.name);
    }

    h.keys("?");
    assert!(h.read(|a| a.help_open()));
    let help = h.read(|a| a.help_lines());
    for name in &names {
        assert!(help.iter().any(|l| l.contains(name)), "help lacks {name:?}");
    }
    h.keys("?");

    for name in names {
        h.keys("cmd-k");
        assert!(h.read(|a| a.palette_open()));
        h.type_text(name);
        let rows = h.palette_rows();
        assert!(rows.iter().any(|r| r.contains(name)), "palette search for {name:?}: {rows:?}");
        h.keys("escape");
        assert!(!h.read(|a| a.palette_open()));
    }
}

// ---------------------------------------------------------------- Invariant

#[gpui_kit::gpui::test]
pub fn counts_plus_hidden_equal_total_over_mixed_sequence(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.assert_invariant("start");
    let steps = [
        "e", "j j x j x e", "shift-e", "u", "s 1", "m", "1",
        "j shift-u", "u u", "e i e", "escape", "c", "y", "n", "2", "i",
        "shift-i", "1", "u u u u", "cmd-z",
    ];
    for step in steps {
        h.keys(step);
        // Panels/pickers left open by a step must not leak into the next.
        h.keys("escape");
        h.assert_invariant(step);
    }
    h.advance(DAY);
    h.assert_invariant("after a day");
    h.advance(3 * DAY);
    h.assert_invariant("after 4 days");
}
