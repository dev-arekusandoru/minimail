use mail_classifier::judge::{Answer, AnswerValue, QuestionKey, Suggestion};
use mail_classifier::model::{Mailbox, MessageId, TriageState};
use mail_classifier::tz::{FixedZone, Now, Utc};
use std::rc::Rc;
/// The clock a test judges at, read in UTC so a test's instants are its own labels.
pub fn utc(at: i64) -> Now {
    Now::new(at, Rc::new(Utc))
}
/// The same clock on a machine east of UTC: UTC+13, where the local day rolls over early.
pub fn east13(at: i64) -> Now {
    Now::new(at, Rc::new(FixedZone(13 * 3600)))
}
pub type State = TriageState;
pub const T0: i64 = 1_790_812_800;
pub fn mailbox(specs: &[(u32, &str, &str, State)]) -> Mailbox {
    let values: Vec<_> = specs
        .iter()
        .map(|(id, from, received, state)| msg(*id, *id, from, received, *state))
        .collect();
    Mailbox::from_json(&serde_json::to_string(&values).unwrap()).unwrap()
}
pub fn threaded(specs: &[(u32, u32, &str, &str, State)]) -> Mailbox {
    let values: Vec<_> = specs
        .iter()
        .map(|(id, thread, from, received, state)| {
            let mut m = msg(*id, *thread, from, received, *state);
            m["account"] = "personal".into();
            m
        })
        .collect();
    Mailbox::from_json(&serde_json::to_string(&values).unwrap()).unwrap()
}
pub fn msg(id: u32, thread: u32, from: &str, received: &str, state: State) -> serde_json::Value {
    serde_json::json!({"id":id,"thread_id":thread,"from_name":from,"from_email":from,"to":"you@example.com","subject":format!("m{id}"),"body":"body","received":received,"state":state,"account":"personal"})
}
pub fn sample() -> Mailbox {
    mailbox(&[
        (1, "a@x.test", "2026-09-29T00:00:00Z", State::Inbox),
        (2, "b@x.test", "2026-09-28T00:00:00Z", State::Inbox),
        (3, "a@x.test", "2026-09-27T00:00:00Z", State::Archived),
        (4, "c@x.test", "2026-09-26T00:00:00Z", State::Inbox),
        (5, "a@x.test", "2026-09-25T00:00:00Z", State::Deleted),
    ])
}
pub fn sug(message: MessageId, key: QuestionKey, value: AnswerValue) -> Suggestion {
    Suggestion {
        message,
        key,
        answer: Answer {
            probabilities: vec![],
            value,
            confidence: 1.0,
        },
    }
}
pub fn assert_invariant(mb: &Mailbox) {
    let mut states = std::collections::HashSet::new();
    for m in mb.messages() {
        states.insert(m.state);
        if m.state == State::Snoozed {
            assert!(mb.snoozed_until(m.id).is_some());
        } else {
            assert!(mb.snoozed_until(m.id).is_none());
        }
    }
    let visible: usize = states.into_iter().map(|state| mb.count(state)).sum();
    assert_eq!(visible + mb.hidden_count(), mb.messages().len());
}
