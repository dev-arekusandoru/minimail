use mail_classifier::model::{Mailbox, Triage};
use crate::helpers::{assert_invariant, mailbox, sample, state_of};
use crate::helpers::State;

// ------------------------------------------------------------------ ordering

#[test]
fn ids_in_is_newest_first_across_timezone_offsets() {
    // 10:00+05:00 is 05:00Z, so it is *older* than 06:00Z even though its
    // string sorts later.
    let mb = mailbox(&[
        (1, "a@x.test", "2026-09-20T10:00:00+05:00", State::Inbox),
        (2, "b@x.test", "2026-09-20T06:00:00Z", State::Inbox),
        (3, "c@x.test", "2026-09-19T23:00:00Z", State::Inbox),
    ]);
    assert_eq!(mb.ids_in(State::Inbox), vec![2, 1, 3]);
}

#[test]
fn ids_in_filters_by_state() {
    let mb = sample();
    assert_eq!(mb.ids_in(State::Inbox), vec![2, 1]);
    assert_eq!(mb.ids_in(State::Later), vec![3]);
    assert_eq!(mb.ids_in(State::Done), vec![4]);
    assert_eq!(mb.ids_in(State::Waiting), vec![5]);
}

// ------------------------------------------------------------------ transitions

#[test]
fn set_state_moves_the_message_and_updates_counts() {
    let mut mb = sample();
    assert_eq!(mb.set_state(&[2], State::Done), 1);
    assert_eq!(mb.state_of(2), Some(State::Done));
    assert_eq!(mb.count(State::Inbox), 1);
    assert_eq!(mb.count(State::Done), 2);
    assert_invariant(&mb);
}

#[test]
fn set_state_counts_only_real_changes() {
    let mut mb = sample();
    // 4 is already Done, 99 does not exist, 1 appears twice.
    assert_eq!(mb.set_state(&[4, 99], State::Done), 0);
    assert_eq!(mb.set_state(&[1, 1, 99], State::Later), 1);
    assert_invariant(&mb);
}

#[test]
fn a_noop_set_state_leaves_no_undo_entry() {
    let mut mb = sample();
    mb.set_state(&[1], State::Done);
    mb.set_state(&[2], State::Done);
    mb.set_state(&[2, 404], State::Done); // no-op
    assert!(mb.undo());
    assert!(mb.undo());
    assert_eq!(mb.state_of(1), Some(State::Inbox));
    assert_eq!(mb.state_of(2), Some(State::Inbox));
    assert!(
        !mb.undo(),
        "the no-op must not have pushed an undo entry"
    );
}

#[test]
fn set_state_batch_is_one_undo_entry() {
    let mut mb = sample();
    assert_eq!(mb.set_state(&[1, 3, 5], State::Done), 3);
    assert!(mb.undo());
    assert_eq!(mb.state_of(1), Some(State::Inbox));
    assert_eq!(mb.state_of(3), Some(State::Later));
    assert_eq!(mb.state_of(5), Some(State::Waiting));
    assert!(!mb.undo());
    assert_invariant(&mb);
}

#[test]
fn set_state_for_sender_covers_messages_in_every_state() {
    let mut mb = sample();
    // a@x.test owns 1 (Inbox), 3 (Later) and 5 (Waiting).
    assert_eq!(mb.set_state_for_sender("a@x.test", State::Done), 3);
    assert_eq!(mb.count(State::Done), 4);
    assert!(!mb.ids_in(State::Later).contains(&3));
    assert!(!mb.ids_in(State::Waiting).contains(&5));
    assert_invariant(&mb);
}

#[test]
fn set_state_for_sender_is_one_undo_entry() {
    let mut mb = sample();
    mb.set_state_for_sender("a@x.test", State::Done);
    assert!(mb.undo());
    assert_eq!(mb.state_of(1), Some(State::Inbox));
    assert_eq!(mb.state_of(3), Some(State::Later));
    assert_eq!(mb.state_of(5), Some(State::Waiting));
    assert!(!mb.undo());
}

#[test]
fn repeated_sender_ops_do_not_stack_undo_entries() {
    let mut mb = sample();
    mb.set_state_for_sender("a@x.test", State::Done);
    assert_eq!(mb.set_state_for_sender("a@x.test", State::Done), 0);
    assert_eq!(mb.set_state_for_sender("nobody@x.test", State::Done), 0);
    assert!(mb.undo());
    assert!(!mb.undo());
}

/// Deterministic xorshift64, so the mixed-run test is reproducible.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

#[test]
fn exactly_one_state_survives_a_long_mixed_run_of_operations() {
    let mut mb = Mailbox::load_default();
    let total = mb.messages().len() as u64;
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    for step in 0..300 {
        let ids: Vec<u32> = mb.messages().iter().map(|m| m.id).collect();
        match rng.next() % 6 {
            0 => {
                let id = ids[(rng.next() % total) as usize];
                mb.set_state(&[id], state_of(rng.next()));
            }
            1 => {
                let batch: Vec<u32> = (0..3)
                    .map(|_| ids[(rng.next() % total) as usize])
                    .collect();
                mb.set_state(&batch, state_of(rng.next()));
            }
            2 => {
                let id = ids[(rng.next() % total) as usize];
                let sender = mb.get(id).unwrap().from_email.clone();
                mb.set_state_for_sender(&sender, state_of(rng.next()));
            }
            3 => {
                let id = ids[(rng.next() % total) as usize];
                mb.send_reply(id, format!("reply {step}"));
            }
            4 => {
                mb.undo();
            }
            _ => {
                let mut t = Triage::new(state_of(rng.next()));
                t.move_cursor(&mb, rng.next() as isize % 7 - 3);
                if rng.next().is_multiple_of(2) {
                    t.extend(&mb, rng.next() as isize % 5 - 2);
                }
                t.apply(&mut mb, state_of(rng.next()));
            }
        }
        assert_invariant(&mb);
    }
}

// ------------------------------------------------------------------ replies

#[test]
fn send_reply_records_the_reply_and_parks_the_original() {
    let mut mb = sample();
    mb.send_reply(1, "Thanks — agreed on Waiting.".to_string());
    assert_eq!(mb.sent().len(), 1);
    assert_eq!(mb.sent()[0].in_reply_to, 1);
    assert_eq!(mb.sent()[0].body, "Thanks — agreed on Waiting.");
    assert_eq!(mb.state_of(1), Some(State::Waiting));
    assert_invariant(&mb);
}

#[test]
fn undo_retracts_the_reply_and_restores_the_original() {
    let mut mb = sample();
    mb.send_reply(1, "first".to_string());
    mb.send_reply(2, "second".to_string());
    assert_eq!(mb.sent().len(), 2);

    assert!(mb.undo());
    assert_eq!(mb.sent().len(), 1);
    assert_eq!(mb.sent()[0].in_reply_to, 1);
    assert_eq!(mb.state_of(2), Some(State::Inbox));

    assert!(mb.undo());
    assert!(mb.sent().is_empty());
    assert_eq!(mb.state_of(1), Some(State::Inbox));
    assert!(!mb.undo());
    assert_invariant(&mb);
}

#[test]
fn replying_to_an_already_waiting_message_still_undoes_once() {
    let mut mb = sample();
    mb.send_reply(5, "reply to a waiting message".to_string());
    assert_eq!(mb.state_of(5), Some(State::Waiting));
    assert!(mb.undo(), "the reply itself must be undoable");
    assert!(mb.sent().is_empty());
    assert_eq!(mb.state_of(5), Some(State::Waiting));
    assert!(
        !mb.undo(),
        "undoing a reply must not rewind earlier operations"
    );
}

#[test]
fn send_reply_to_an_unknown_message_does_nothing() {
    let mut mb = sample();
    mb.send_reply(999, "nobody".to_string());
    assert!(mb.sent().is_empty());
    assert!(!mb.undo());
}

// ------------------------------------------------------------------ fixture

#[test]
fn the_default_fixture_is_large_varied_and_consistent() {
    let mb = Mailbox::load_default();
    assert!(
        mb.messages().len() >= 50,
        "fixture must be big enough to triage"
    );

    let mut ids: Vec<u32> = mb.messages().iter().map(|m| m.id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), mb.messages().len(), "ids must be unique");

    assert!(mb.messages().iter().all(|m| m.to == "you@example.com"));
    assert!(
        mb.messages()
            .iter()
            .all(|m| m.body.contains("\n\n") && m.subject.len() > 3)
    );
    assert!(mb.messages().iter().all(|m| m.received.starts_with("2026-09-")));

    let mut per_sender: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for m in mb.messages() {
        *per_sender.entry(&m.from_email).or_default() += 1;
    }
    let chatty = per_sender.values().filter(|n| **n >= 3).count();
    assert!(chatty >= 8, "want several recurring senders, got {chatty}");

    let mut per_thread: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    for m in mb.messages() {
        *per_thread.entry(m.thread_id).or_default() += 1;
    }
    assert!(
        per_thread.values().any(|n| *n > 1),
        "the fixture should contain multi-message threads"
    );

    for state in State::ALL {
        assert!(mb.count(state) > 0, "{state:?} view must not be empty");
    }
    assert_invariant(&mb);
}
