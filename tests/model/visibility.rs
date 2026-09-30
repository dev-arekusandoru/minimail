use mail_classifier::model::{Mailbox};
use crate::helpers::{assert_invariant, sample, threaded, msg};
use crate::helpers::State;

#[test]
fn default_fixture_has_a_small_screener_and_keeps_the_invariant() {
    let mb = Mailbox::load_default();
    assert!(mb.messages().len() >= 60);
    let mut senders: Vec<String> = mb
        .screener_ids()
        .iter()
        .map(|id| mb.get(*id).unwrap().from_email.clone())
        .collect();
    senders.sort();
    senders.dedup();
    assert!((3..=5).contains(&senders.len()), "{senders:?}");
    assert_eq!(mb.hidden_count(), 0);
    assert_invariant(&mb);
}

#[test]
fn screener_is_newest_first_and_allow_moves_mail_to_its_state_view() {
    let mut mb = Mailbox::from_json_with_contacts(
        &serde_json::to_string(&serde_json::json!([
            msg(1, 1, "known@x.test", "2026-09-01T00:00:00Z"),
            msg(2, 2, "new@x.test", "2026-09-02T00:00:00Z"),
            msg(3, 3, "New@x.test", "2026-09-03T00:00:00Z"),
        ]))
        .unwrap(),
        r#"["KNOWN@x.test"]"#,
    )
    .unwrap();
    assert_eq!(mb.screener_ids(), vec![3, 2]);
    assert_eq!(mb.ids_in(State::Inbox), vec![1]);
    assert!(mb.allow_sender("new@x.test"));
    assert!(!mb.allow_sender("new@x.test"));
    assert!(mb.screener_ids().is_empty());
    assert_eq!(mb.ids_in(State::Inbox), vec![3, 2, 1]);
    assert!(mb.undo());
    assert_eq!(mb.screener_ids(), vec![3, 2]);
    assert_invariant(&mb);
#[test]
fn block_hides_without_deleting_and_beats_the_screener() {
    let mut mb = Mailbox::from_json_with_contacts(
        &serde_json::to_string(&serde_json::json!([
            msg(1, 1, "new@x.test", "2026-09-01T00:00:00Z"),
            msg(2, 2, "ok@x.test", "2026-09-02T00:00:00Z"),
        ]))
        .unwrap(),
        r#"["ok@x.test"]"#,
    )
    .unwrap();
    assert!(mb.block_sender("new@x.test"));
    assert!(!mb.block_sender("new@x.test"));
    assert!(mb.screener_ids().is_empty(), "blocked is hidden, not screened");
    assert_eq!(mb.hidden_count(), 1);
    assert_eq!(mb.messages().len(), 2);
    // Allowing a blocked sender does not resurrect it.
    mb.allow_sender("new@x.test");
    assert_eq!(mb.ids_in(State::Inbox), vec![2]);
    assert_invariant(&mb);
    assert!(mb.undo()); // allow
    assert!(mb.undo()); // block
    assert_eq!(mb.hidden_count(), 0);
    assert_eq!(mb.screener_ids(), vec![1]);
}

#[test]
fn mute_and_unsubscribe_hide_and_undo() {
    let mut mb = threaded(&[
        (1, 10, "a@x.test", "2026-09-01T00:00:00Z", State::Inbox),
        (2, 10, "b@x.test", "2026-09-02T00:00:00Z", State::Later),
        (3, 11, "a@x.test", "2026-09-03T00:00:00Z", State::Inbox),
        (4, 12, "c@x.test", "2026-09-04T00:00:00Z", State::Inbox),
    ]);
    assert!(mb.mute_thread(10));
    assert!(!mb.mute_thread(10));
    assert!(mb.is_muted(10) && !mb.is_muted(11));
    assert_eq!(mb.hidden_count(), 2);
    assert_eq!(mb.state_of(2), Some(State::Later), "hidden mail keeps its state");
    assert_eq!(mb.count(State::Later), 0);

    assert!(mb.unsubscribe("A@x.test"));
    assert!(!mb.unsubscribe("a@x.test"));
    assert_eq!(mb.unsubscribed(), ["A@x.test"]);
    assert_eq!(mb.ids_in(State::Inbox), vec![4]);
    assert_eq!(mb.hidden_count(), 3);
    assert_invariant(&mb);

    assert!(mb.undo());
    assert!(mb.unsubscribed().is_empty());
    assert_eq!(mb.ids_in(State::Inbox), vec![4, 3]);
    assert!(mb.undo());
    assert!(!mb.is_muted(10));
    assert_eq!(mb.hidden_count(), 0);
}

#[test]
fn sender_wide_state_change_includes_hidden_mail() {
    let mut mb = sample();
    mb.mute_thread(1);
    assert_eq!(mb.set_state_for_sender("a@x.test", State::Done), 3);
    assert_invariant(&mb);
}
