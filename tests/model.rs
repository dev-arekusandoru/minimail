use mail_classifier::model::{Mailbox, Triage, TriageState};

type State = TriageState;

fn state_of(n: u64) -> State {
    State::ALL[(n % 4) as usize]
}

/// Build a mailbox from `(id, from_email, received, state)` tuples.
fn mailbox(specs: &[(u32, &str, &str, State)]) -> Mailbox {
    let messages: Vec<serde_json::Value> = specs
        .iter()
        .map(|(id, from, received, state)| {
            serde_json::json!({
                "id": id,
                "thread_id": id,
                "from_name": from,
                "from_email": from,
                "to": "you@example.com",
                "subject": format!("subject {id}"),
                "body": format!("body {id}\n\nsecond paragraph"),
                "received": received,
                "state": state,
            })
        })
        .collect();
    Mailbox::from_json(&serde_json::to_string(&messages).unwrap()).unwrap()
}

/// Five messages, four senders, one of them (a@x.test) spread over three states.
fn sample() -> Mailbox {
    mailbox(&[
        (1, "a@x.test", "2026-09-01T10:00:00Z", State::Inbox),
        (2, "b@x.test", "2026-09-02T10:00:00Z", State::Inbox),
        (3, "a@x.test", "2026-09-03T10:00:00Z", State::Later),
        (4, "c@x.test", "2026-09-04T10:00:00Z", State::Done),
        (5, "a@x.test", "2026-09-05T10:00:00Z", State::Waiting),
    ])
}

fn assert_invariant(mb: &Mailbox) {
    let total = mb.messages().len();
    let sum: usize = State::ALL.iter().map(|s| mb.count(*s)).sum::<usize>()
        + mb.screener_ids().len()
        + mb.hidden_count();
    assert_eq!(sum, total, "every message is visible, screened or hidden");
    let mut all: Vec<u32> = Vec::new();
    for state in State::ALL {
        let ids = mb.ids_in(state);
        assert_eq!(
            ids.len(),
            mb.count(state),
            "{state:?} count disagrees with ids_in"
        );
        assert!(ids.iter().all(|id| mb.state_of(*id) == Some(state)));
        all.extend(ids);
    }
    all.extend(mb.screener_ids());
    all.sort_unstable();
    all.dedup();
    assert_eq!(all.len(), total - mb.hidden_count(), "views must not overlap");
}

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

// ------------------------------------------------------------------ cursor

#[test]
fn move_cursor_clamps_at_both_ends() {
    let mb = sample();
    let mut t = Triage::new(State::Inbox);
    assert_eq!(t.cursor_index(), 0);
    assert_eq!(t.cursor(&mb), Some(2));

    t.move_cursor(&mb, 99);
    assert_eq!(t.cursor_index(), 1);
    assert_eq!(t.cursor(&mb), Some(1));

    t.move_cursor(&mb, -99);
    assert_eq!(t.cursor_index(), 0);
    assert_eq!(t.cursor(&mb), Some(2));
}

#[test]
fn an_empty_view_has_no_cursor_and_moving_is_safe() {
    let empty = mailbox(&[]);
    let mut t = Triage::new(State::Done);
    assert_eq!(t.cursor(&empty), None);
    t.move_cursor(&empty, 3);
    t.extend(&empty, -2);
    t.toggle_select(&empty);
    assert_eq!(t.cursor_index(), 0);
    assert!(t.selected().is_empty());
    assert!(t.targets(&empty).is_empty());
}

#[test]
fn the_cursor_clamps_when_the_view_shrinks_under_it() {
    let mut mb = sample();
    let mut t = Triage::new(State::Inbox);
    t.move_cursor(&mb, 1); // index 1: the oldest Inbox message, id 1
    assert_eq!(t.cursor(&mb), Some(1));

    // The message under the cursor leaves the view; the position holds.
    mb.set_state(&[1], State::Done);
    assert_eq!(t.cursor_index(), 1, "the raw index is not rewritten");
    assert_eq!(t.cursor(&mb), Some(2), "the cursor message is clamped");
    assert_eq!(t.cursor(&mb), mb.ids_in(State::Inbox).last().copied());

    // Emptying the view must not panic.
    mb.set_state(&[2], State::Done);
    assert!(mb.ids_in(State::Inbox).is_empty());
    assert_eq!(t.cursor(&mb), None);
    assert_eq!(t.cursor_index(), 1);
}

// ------------------------------------------------------------------ selection

/// Four Inbox messages; the view is ordered 4, 3, 2, 1.
fn four() -> Mailbox {
    mailbox(&[
        (1, "a@x.test", "2026-09-01T10:00:00Z", State::Inbox),
        (2, "b@x.test", "2026-09-02T10:00:00Z", State::Inbox),
        (3, "c@x.test", "2026-09-03T10:00:00Z", State::Inbox),
        (4, "d@x.test", "2026-09-04T10:00:00Z", State::Inbox),
    ])
}

#[test]
fn extend_selects_an_inclusive_range_from_the_anchor() {
    let mb = four();
    let mut t = Triage::new(State::Inbox);
    t.extend(&mb, 1);
    assert_eq!(t.cursor_index(), 1);
    assert_eq!(t.selected(), vec![4, 3], "anchor and cursor, inclusive");

    t.extend(&mb, 1);
    assert_eq!(t.selected(), vec![4, 3, 2]);

    t.extend(&mb, -2);
    assert_eq!(t.selected(), vec![4], "the anchor never moves");
}

#[test]
fn extending_backwards_keeps_the_selection_in_view_order() {
    let mb = four();
    let mut t = Triage::new(State::Inbox);
    t.move_cursor(&mb, 2);
    t.extend(&mb, -1);
    assert_eq!(t.selected(), vec![3, 2]);
    assert!(t.is_selected(3) && t.is_selected(2) && !t.is_selected(1));
}

#[test]
fn clearing_the_selection_resets_the_anchor() {
    let mb = four();
    let mut t = Triage::new(State::Inbox);
    t.extend(&mb, 2);
    assert_eq!(t.selected(), vec![4, 3, 2]);
    t.clear_selection();
    assert!(t.selected().is_empty());

    t.move_cursor(&mb, -1);
    t.extend(&mb, 1);
    assert_eq!(t.selected(), vec![3, 2], "a new range, not the old one");
}

#[test]
fn toggle_select_adds_then_removes_and_keeps_view_order() {
    let mb = four();
    let mut t = Triage::new(State::Inbox);
    t.toggle_select(&mb);
    t.move_cursor(&mb, 2);
    t.toggle_select(&mb);
    assert_eq!(t.selected(), vec![4, 2]);
    t.toggle_select(&mb);
    assert_eq!(t.selected(), vec![4]);
    assert!(!t.is_selected(2));
}

#[test]
fn move_cursor_does_not_disturb_the_selection() {
    let mb = four();
    let mut t = Triage::new(State::Inbox);
    t.toggle_select(&mb);
    t.move_cursor(&mb, 1);
    assert_eq!(t.selected(), vec![4]);
    assert_eq!(t.cursor(&mb), Some(3));
}

#[test]
fn targets_prefer_the_selection_and_fall_back_to_the_cursor() {
    let mb = four();
    let mut t = Triage::new(State::Inbox);
    assert_eq!(t.targets(&mb), vec![4], "no selection: just the cursor");
    t.toggle_select(&mb);
    t.move_cursor(&mb, 1);
    assert_eq!(t.targets(&mb), vec![4], "the selection wins");
}

#[test]
fn apply_moves_the_targets_clears_the_selection_and_clamps() {
    let mut mb = four();
    let mut t = Triage::new(State::Inbox);
    t.extend(&mb, 1); // selects 4 and 3
    assert_eq!(t.apply(&mut mb, State::Done), 2);
    assert!(t.selected().is_empty());
    assert_eq!(t.cursor_index(), 1, "the cursor keeps its position");
    assert_eq!(t.cursor(&mb), Some(1), "the cursor now sits on the last remaining message");
    assert_invariant(&mb);

    // One undo rewinds the whole batch.
    assert!(mb.undo());
    assert_eq!(mb.count(State::Inbox), 4);
    assert_eq!(mb.count(State::Done), 0);
}

#[test]
fn apply_on_an_empty_view_changes_nothing() {
    let mut mb = four();
    let mut t = Triage::new(State::Later);
    t.move_cursor(&mb, 0);
    assert_eq!(t.apply(&mut mb, State::Done), 0);
    assert!(!mb.undo());
    assert_eq!(t.apply_to_sender(&mut mb, State::Done), 0);
    assert_invariant(&mb);
}

#[test]
fn apply_to_sender_moves_every_message_from_that_sender() {
    let mut mb = sample();
    let mut t = Triage::new(State::Inbox);
    t.move_cursor(&mb, 1); // id 1, from a@x.test
    assert_eq!(t.apply_to_sender(&mut mb, State::Done), 3);
    assert_eq!(mb.ids_in(State::Inbox), vec![2]);
    assert_eq!(mb.count(State::Done), 4);
    assert_invariant(&mb);

    assert!(mb.undo());
    assert_eq!(mb.count(State::Done), 1);
    assert_eq!(mb.state_of(3), Some(State::Later));
}

#[test]
fn switch_view_resets_the_cursor_and_the_selection() {
    let mb = mailbox(&[
        (1, "a@x.test", "2026-09-01T10:00:00Z", State::Inbox),
        (2, "b@x.test", "2026-09-02T10:00:00Z", State::Inbox),
        (3, "c@x.test", "2026-09-03T10:00:00Z", State::Later),
    ]);
    let mut t = Triage::new(State::Inbox);
    t.extend(&mb, 1);
    t.switch_view(State::Later);
    assert_eq!(t.view, State::Later);
    assert_eq!(t.cursor_index(), 0);
    assert!(t.selected().is_empty());
    assert_eq!(t.cursor(&mb), Some(3));
    assert_eq!(t.targets(&mb), vec![3]);
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

// ------------------------------------------------------- v2: visibility etc.

mod v2 {
    use super::*;
    use mail_classifier::clock::{DAY, HOUR};
    use mail_classifier::judge::{Answer, AnswerValue, Kind, QuestionKey, Suggestion};
    use mail_classifier::model::{OUTBOX_DELAY, Tag};

    /// 2026-10-01T00:00:00Z, a Thursday.
    const T0: i64 = 1_790_812_800;

    /// `(id, thread, from, received, state)`.
    fn threaded(specs: &[(u32, u32, &str, &str, State)]) -> Mailbox {
        let msgs: Vec<serde_json::Value> = specs
            .iter()
            .map(|(id, th, from, received, state)| {
                serde_json::json!({
                    "id": id, "thread_id": th, "from_name": from, "from_email": from,
                    "to": "you@example.com", "subject": format!("s{id}"),
                    "body": "b\n\nc", "received": received, "state": state,
                })
            })
            .collect();
        Mailbox::from_json(&serde_json::to_string(&msgs).unwrap()).unwrap()
    }

    fn sug(message: u32, key: QuestionKey, value: AnswerValue) -> Suggestion {
        Suggestion {
            message,
            key,
            answer: Answer {
                probabilities: vec![],
                value,
                confidence: 0.9,
            },
        }
    }

    type Snap = (u32, State, Vec<Tag>, Option<i64>, Option<i64>);
    fn snapshot(mb: &Mailbox) -> Vec<Snap> {
        mb.messages()
            .iter()
            .map(|m| {
                (
                    m.id,
                    m.state,
                    mb.tags(m.id).to_vec(),
                    mb.waiting_since(m.id),
                    mb.snoozed_until(m.id),
                )
            })
            .collect()
    }

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
    }

    fn msg(id: u32, thread: u32, from: &str, received: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id, "thread_id": thread, "from_name": from, "from_email": from,
            "to": "you@example.com", "subject": "s", "body": "b\n\nc", "received": received,
        })
    }

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
    fn set_state_at_stamps_waiting_and_clears_it_on_leave() {
        let mut mb = sample();
        assert_eq!(mb.set_state_at(&[1], State::Waiting, 500), 1);
        assert_eq!(mb.waiting_since(1), Some(500));
        mb.set_state(&[2], State::Waiting);
        assert_eq!(mb.waiting_since(2), None, "plain set_state records no time");
        mb.set_state_at(&[1], State::Done, 600);
        assert_eq!(mb.waiting_since(1), None);
        assert!(mb.undo());
        assert_eq!(mb.waiting_since(1), Some(500));
        assert!(mb.undo() && mb.undo());
        assert_eq!(mb.waiting_since(1), None);
        assert_eq!(mb.state_of(1), Some(State::Inbox));
    }

    #[test]
    fn waiting_resurfaces_after_three_days_and_tick_is_idempotent() {
        let mut mb = threaded(&[(1, 1, "a@x.test", "2026-09-01T00:00:00Z", State::Inbox)]);
        mb.set_state_at(&[1], State::Waiting, T0);
        let undo_depth_probe = snapshot(&mb);

        let r = mb.tick(T0 + 3 * DAY - 1);
        assert!(r.resurfaced.is_empty());
        assert_eq!(mb.state_of(1), Some(State::Waiting));

        let r = mb.tick(T0 + 3 * DAY);
        assert_eq!(r.resurfaced, vec![1]);
        assert_eq!(mb.state_of(1), Some(State::Inbox));
        assert_eq!(mb.tags(1), [Tag::NoReply]);
        assert_eq!(mb.waiting_since(1), None);

        let after = snapshot(&mb);
        let again = mb.tick(T0 + 3 * DAY);
        assert_eq!(again, Default::default());
        assert_eq!(snapshot(&mb), after);
        assert_ne!(after, undo_depth_probe);

        // tick pushed no undo step: undo reverts the earlier set_state_at.
        assert!(mb.undo());
        assert_eq!(mb.state_of(1), Some(State::Inbox));
        assert!(!mb.undo());
    }

    #[test]
    fn waiting_does_not_resurface_when_someone_replied_after() {
        // Thread 1: reply arrives 2026-10-02, after we started waiting on T0.
        let mut mb = threaded(&[
            (1, 1, "a@x.test", "2026-09-01T00:00:00Z", State::Inbox),
            (2, 1, "a@x.test", "2026-10-01T12:00:00Z", State::Inbox),
            // Thread 2: the only other message is older than waiting_since.
            (3, 2, "a@x.test", "2026-09-01T00:00:00Z", State::Inbox),
            (4, 2, "a@x.test", "2026-09-02T00:00:00Z", State::Inbox),
        ]);
        mb.set_state_at(&[1, 3], State::Waiting, T0);
        let r = mb.tick(T0 + 4 * DAY);
        assert_eq!(r.resurfaced, vec![3]);
        assert_eq!(mb.state_of(1), Some(State::Waiting));
    }

    #[test]
    fn leaving_and_reentering_waiting_drops_the_no_reply_tag() {
        let mut mb = threaded(&[(1, 1, "a@x.test", "2026-09-01T00:00:00Z", State::Inbox)]);
        mb.set_state_at(&[1], State::Waiting, T0);
        mb.tick(T0 + 3 * DAY);
        mb.set_state_at(&[1], State::Waiting, T0 + 3 * DAY);
        assert!(mb.tags(1).is_empty());
        assert_eq!(mb.waiting_since(1), Some(T0 + 3 * DAY));
    }

    #[test]
    fn snooze_wakes_exactly_at_the_return_time_and_undoes() {
        let mut mb = sample();
        assert_eq!(mb.snooze(&[1, 2, 3], T0 + HOUR, T0), 3);
        assert_eq!(mb.state_of(1), Some(State::Later));
        assert_eq!(mb.snoozed_until(3), Some(T0 + HOUR));
        assert_eq!(mb.snooze(&[1], T0 + HOUR, T0), 0, "same time is a no-op");

        assert!(mb.tick(T0 + HOUR - 1).woken.is_empty());
        let r = mb.tick(T0 + HOUR);
        assert_eq!(r.woken.len(), 3);
        assert_eq!(mb.state_of(3), Some(State::Inbox), "message 3 was Later, now Inbox");
        assert_eq!(mb.snoozed_until(3), None);
        assert!(mb.tick(T0 + HOUR).woken.is_empty());

        assert!(mb.undo());
        assert_eq!(mb.state_of(1), Some(State::Inbox));
        assert_eq!(mb.state_of(3), Some(State::Later));
        assert_eq!(mb.snoozed_until(3), None);
    }

    #[test]
    fn manually_leaving_later_clears_the_snooze() {
        let mut mb = sample();
        mb.snooze(&[1], T0 + HOUR, T0);
        mb.set_state(&[1], State::Done);
        assert_eq!(mb.snoozed_until(1), None);
        mb.tick(T0 + 2 * HOUR);
        assert_eq!(mb.state_of(1), Some(State::Done));
    }

    #[test]
    fn snooze_presets_roll_over_and_land_on_the_right_weekday() {
        let mb = sample();
        // T0 = Thursday 00:00.
        let [tonight, tomorrow, monday] = mb.snooze_presets(T0 + 9 * HOUR);
        assert_eq!(tonight, ("Tonight", T0 + 18 * HOUR));
        assert_eq!(tomorrow, ("Tomorrow", T0 + DAY + 8 * HOUR));
        assert_eq!(monday, ("Monday", T0 + 4 * DAY + 8 * HOUR));
        // Past 18:00 (and exactly 18:00): Tonight becomes tomorrow evening.
        assert_eq!(mb.snooze_presets(T0 + 18 * HOUR)[0].1, T0 + DAY + 18 * HOUR);
        assert_eq!(mb.snooze_presets(T0 + 20 * HOUR)[0].1, T0 + DAY + 18 * HOUR);
        // On a Monday, "Monday" means next week.
        let mon = T0 + 4 * DAY + 10 * HOUR;
        assert_eq!(mb.snooze_presets(mon)[2].1, T0 + 11 * DAY + 8 * HOUR);
        // Sunday -> next day.
        assert_eq!(mb.snooze_presets(T0 + 3 * DAY)[2].1, T0 + 4 * DAY + 8 * HOUR);
    }

    #[test]
    fn parse_snooze_accepts_units_and_rejects_junk() {
        let mb = sample();
        let _ = &mb;
        use mail_classifier::model::parse_snooze;
        assert_eq!(parse_snooze("30m", T0), Some(T0 + 1800));
        assert_eq!(parse_snooze(" 3H ", T0), Some(T0 + 3 * HOUR));
        assert_eq!(parse_snooze("2d", T0), Some(T0 + 2 * DAY));
        for bad in ["", "0m", "m", "3", "3w", "-1h", "1.5h", "3 h", "99999999999999999999d"] {
            assert_eq!(parse_snooze(bad, T0), None, "{bad:?}");
        }
    }

    #[test]
    fn outbox_holds_reply_until_due_and_tick_flushes_once() {
        let mut mb = sample();
        mb.send_reply_at(1, "hi".into(), T0);
        assert_eq!(mb.state_of(1), Some(State::Waiting));
        assert_eq!(mb.waiting_since(1), Some(T0));
        assert_eq!(mb.outbox().len(), 1);
        assert_eq!(mb.outbox()[0].due, T0 + OUTBOX_DELAY);
        assert!(mb.sent().is_empty());

        assert_eq!(mb.tick(T0 + OUTBOX_DELAY - 1).flushed, 0);
        assert_eq!(mb.tick(T0 + OUTBOX_DELAY).flushed, 1);
        assert_eq!(mb.tick(T0 + OUTBOX_DELAY).flushed, 0);
        assert!(mb.outbox().is_empty());
        assert_eq!(mb.sent().len(), 1);
        assert_eq!(mb.sent()[0].body, "hi");
    }

    #[test]
    fn undo_after_flush_reverts_state_only() {
        let mut mb = sample();
        mb.send_reply_at(1, "hi".into(), T0);
        mb.tick(T0 + 60);
        assert!(mb.undo());
        assert_eq!(mb.state_of(1), Some(State::Inbox));
        assert_eq!(mb.sent().len(), 1, "the sent mail already left");
    }

    #[test]
    fn undo_before_flush_pulls_the_reply_back() {
        let mut mb = sample();
        mb.send_reply_at(1, "hi".into(), T0);
        assert!(mb.undo());
        assert!(mb.outbox().is_empty());
        assert_eq!(mb.state_of(1), Some(State::Inbox));
        assert_eq!(mb.tick(T0 + 60).flushed, 0);
        assert!(mb.sent().is_empty());
    }

    #[test]
    fn recall_last_takes_the_newest_and_restores_prior_state() {
        let mut mb = sample();
        assert_eq!(mb.recall_last(T0), None);
        mb.send_reply_at(1, "first".into(), T0);
        mb.send_reply_at(3, "second".into(), T0 + 1);
        let r = mb.recall_last(T0 + 2).unwrap();
        assert_eq!((r.in_reply_to, r.body.as_str()), (3, "second"));
        assert_eq!(mb.state_of(3), Some(State::Later), "prior state restored");
        assert_eq!(mb.state_of(1), Some(State::Waiting));
        assert_eq!(mb.outbox().len(), 1);
        // Its undo step is consumed: undo now reverts the *first* send.
        assert!(mb.undo());
        assert_eq!(mb.state_of(1), Some(State::Inbox));
        assert!(mb.outbox().is_empty());
        assert!(!mb.undo());
    }

    #[test]
    fn recall_keeps_unrelated_later_undo_steps_intact() {
        let mut mb = sample();
        mb.send_reply_at(1, "x".into(), T0);
        mb.set_state(&[2], State::Done);
        assert!(mb.recall_last(T0).is_some());
        assert_eq!(mb.state_of(2), Some(State::Done));
        assert!(mb.undo());
        assert_eq!(mb.state_of(2), Some(State::Inbox));
    }

    #[test]
    fn immediate_send_reply_still_undoes_sent() {
        let mut mb = sample();
        mb.send_reply(1, "now".into());
        assert_eq!(mb.sent().len(), 1);
        assert!(mb.outbox().is_empty());
        assert!(mb.undo());
        assert!(mb.sent().is_empty());
        assert_eq!(mb.state_of(1), Some(State::Inbox));
    }

    #[test]
    fn accept_applies_each_kind_and_undoes_as_one_step() {
        let mut mb = sample();
        let before = snapshot(&mb);
        mb.add_suggestions(vec![
            sug(1, QuestionKey::SuggestedState, AnswerValue::Choice(2)),
            sug(1, QuestionKey::NeedsReply, AnswerValue::Bool(true)),
            sug(1, QuestionKey::Urgency, AnswerValue::Score(3.6)),
            sug(1, QuestionKey::Kind, AnswerValue::Choice(1)),
            sug(2, QuestionKey::Spam, AnswerValue::Bool(true)),
        ]);
        assert_eq!(mb.pending(1).len(), 4);
        assert_eq!(mb.accept_suggestions(1), 4);
        assert!(mb.pending(1).is_empty());
        assert_eq!(mb.pending(2).len(), 1, "other messages untouched");
        assert_eq!(mb.state_of(1), Some(State::Later));
        let tags = mb.tags(1);
        assert!(tags.contains(&Tag::NeedsReply));
        assert!(tags.contains(&Tag::Urgent(4)));
        assert!(tags.contains(&Tag::Kind(Kind::Receipt)));
        assert_eq!(mb.accept_suggestions(1), 0);

        assert!(mb.undo());
        assert_eq!(snapshot(&mb), before);
        assert_eq!(mb.pending(1).len(), 4, "undo restores the pending badges");
        assert!(!mb.undo(), "accept was exactly one step");

        assert_eq!(mb.accept_suggestions(2), 1);
        assert_eq!(mb.state_of(2), Some(State::Done));
        assert_eq!(mb.tags(2), [Tag::Spam]);
    }

    #[test]
    fn negative_answers_change_nothing_and_urgency_replaces() {
        let mut mb = sample();
        mb.add_suggestions(vec![
            sug(1, QuestionKey::Spam, AnswerValue::Bool(false)),
            sug(1, QuestionKey::Urgency, AnswerValue::Score(2.0)),
        ]);
        mb.accept_suggestions(1);
        assert_eq!(mb.state_of(1), Some(State::Inbox));
        assert_eq!(mb.tags(1), [Tag::Urgent(2)]);
        assert!(mb.apply_auto(sug(1, QuestionKey::Urgency, AnswerValue::Score(5.0))));
        assert_eq!(mb.tags(1), [Tag::Urgent(5)]);
        assert!(!mb.apply_auto(sug(1, QuestionKey::Urgency, AnswerValue::Score(5.0))));
        assert!(mb.undo());
        assert_eq!(mb.tags(1), [Tag::Urgent(2)]);
    }

    #[test]
    fn reject_drops_pending_and_undo_restores() {
        let mut mb = sample();
        mb.add_suggestions(vec![
            sug(1, QuestionKey::Spam, AnswerValue::Bool(true)),
            sug(2, QuestionKey::Spam, AnswerValue::Bool(true)),
        ]);
        assert_eq!(mb.reject_suggestions(1), 1);
        assert_eq!(mb.reject_suggestions(1), 0);
        assert!(mb.pending(1).is_empty());
        assert_eq!(mb.state_of(1), Some(State::Inbox));
        assert!(mb.undo());
        assert_eq!(mb.pending(1).len(), 1);
        assert_eq!(mb.pending(2).len(), 1);
    }

    #[test]
    fn newer_suggestion_replaces_the_pending_one_for_same_question() {
        let mut mb = sample();
        mb.add_suggestions(vec![sug(1, QuestionKey::SuggestedState, AnswerValue::Choice(3))]);
        mb.add_suggestions(vec![sug(1, QuestionKey::SuggestedState, AnswerValue::Choice(2))]);
        assert_eq!(mb.pending(1).len(), 1);
        mb.accept_suggestions(1);
        assert_eq!(mb.state_of(1), Some(State::Later));
    }

    #[test]
    fn apply_auto_clears_matching_pending_and_undoes() {
        let mut mb = sample();
        mb.add_suggestions(vec![sug(1, QuestionKey::Spam, AnswerValue::Bool(true))]);
        assert!(mb.apply_auto(sug(1, QuestionKey::Spam, AnswerValue::Bool(true))));
        assert!(mb.pending(1).is_empty());
        assert_eq!(mb.state_of(1), Some(State::Done));
        assert!(mb.undo());
        assert_eq!(mb.state_of(1), Some(State::Inbox));
        assert!(mb.tags(1).is_empty());
        assert_eq!(mb.pending(1).len(), 1);
        assert!(!mb.apply_auto(sug(99, QuestionKey::Spam, AnswerValue::Bool(true))));
    }

    #[test]
    fn out_of_range_choices_are_ignored() {
        let mut mb = sample();
        assert!(!mb.apply_auto(sug(1, QuestionKey::SuggestedState, AnswerValue::Choice(9))));
        assert!(!mb.apply_auto(sug(1, QuestionKey::Kind, AnswerValue::Choice(9))));
        assert!(!mb.undo());
    }

    #[test]
    fn sender_wide_state_change_includes_hidden_mail() {
        let mut mb = sample();
        mb.mute_thread(1);
        assert_eq!(mb.set_state_for_sender("a@x.test", State::Done), 3);
        assert_invariant(&mb);
    }
}
