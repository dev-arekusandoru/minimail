use mail_classifier::model::{Mailbox, TriageState, Tag};
use mail_classifier::judge::{Answer, AnswerValue, QuestionKey, Suggestion};

pub type State = TriageState;

pub fn state_of(n: u64) -> State {
    State::ALL[(n % 4) as usize]
}

/// Build a mailbox from `(id, from_email, received, state)` tuples.
pub fn mailbox(specs: &[(u32, &str, &str, State)]) -> Mailbox {
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
pub fn sample() -> Mailbox {
    mailbox(&[
        (1, "a@x.test", "2026-09-01T10:00:00Z", State::Inbox),
        (2, "b@x.test", "2026-09-02T10:00:00Z", State::Inbox),
        (3, "a@x.test", "2026-09-03T10:00:00Z", State::Later),
        (4, "c@x.test", "2026-09-04T10:00:00Z", State::Done),
        (5, "a@x.test", "2026-09-05T10:00:00Z", State::Waiting),
    ])
}

pub fn assert_invariant(mb: &Mailbox) {
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


/// 2026-10-01T00:00:00Z, a Thursday.
pub const T0: i64 = 1_790_812_800;

/// `(id, thread, from, received, state)`.
pub fn threaded(specs: &[(u32, u32, &str, &str, State)]) -> Mailbox {
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

pub fn sug(message: u32, key: QuestionKey, value: AnswerValue) -> Suggestion {
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

pub type Snap = (u32, State, Vec<Tag>, Option<i64>, Option<i64>);
pub fn snapshot(mb: &Mailbox) -> Vec<Snap> {
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


pub fn msg(id: u32, thread: u32, from: &str, received: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id, "thread_id": thread, "from_name": from, "from_email": from,
        "to": "you@example.com", "subject": "s", "body": "b\n\nc", "received": received,
    })
}
