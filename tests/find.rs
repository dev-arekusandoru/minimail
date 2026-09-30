//! Pure find-in-thread matching: options, ordering, segments and the current-match cursor.

use mail_classifier::find::{
    Find, Match, Options, Segment, Source, find, find_in_thread, highlights, thread_sources,
};
use mail_classifier::model::Mailbox;

fn src(msg: u32, segment: Segment, text: &str) -> Source {
    Source { msg, segment, text: text.to_owned() }
}

fn ranges(matches: &[Match]) -> Vec<(u32, std::ops::Range<usize>)> {
    matches.iter().map(|m| (m.msg, m.range.clone())).collect()
}

fn plain() -> Options {
    Options::default()
}

#[test]
fn matching_ignores_case_unless_asked() {
    let s = [src(1, Segment::Main, "Plan plan PLAN")];
    assert_eq!(find(&s, "plan", plain()).unwrap().len(), 3);
    let cased = Options { case_sensitive: true, ..plain() };
    assert_eq!(ranges(&find(&s, "plan", cased).unwrap()), vec![(1, 5..9)]);
}

#[test]
fn whole_word_skips_matches_inside_words() {
    let s = [src(1, Segment::Main, "plan planet replan plan_b plan.")];
    let word = Options { whole_word: true, ..plain() };
    assert_eq!(ranges(&find(&s, "plan", word).unwrap()), vec![(1, 0..4), (1, 26..30)]);
    // A query ending in punctuation still matches as a word on its word-character side.
    let s = [src(1, Segment::Main, "use c++ or c")];
    assert_eq!(ranges(&find(&s, "c++", word).unwrap()), vec![(1, 4..7)]);
}

#[test]
fn regex_mode_interprets_the_query_and_plain_mode_escapes_it() {
    let s = [src(1, Segment::Main, "a1 b22 c333 (x)")];
    let re = Options { regex: true, ..plain() };
    assert_eq!(ranges(&find(&s, r"\d+", re).unwrap()), vec![(1, 1..2), (1, 4..6), (1, 8..11)]);
    assert_eq!(ranges(&find(&s, "(x)", plain()).unwrap()), vec![(1, 12..15)], "metacharacters are literal");
    let both = Options { regex: true, whole_word: true, ..plain() };
    assert_eq!(ranges(&find(&s, r"b\d", both).unwrap()), vec![], "whole word applies to regexes too");
}

#[test]
fn an_invalid_regex_is_an_error_not_a_panic() {
    let s = [src(1, Segment::Main, "anything")];
    let re = Options { regex: true, ..plain() };
    assert!(find(&s, "(unclosed", re).is_err());
    assert!(find(&s, "(unclosed", plain()).unwrap().is_empty(), "plain mode takes it literally");
}

#[test]
fn an_empty_query_and_empty_matches_find_nothing() {
    let s = [src(1, Segment::Main, "abc")];
    assert!(find(&s, "", plain()).unwrap().is_empty());
    let re = Options { regex: true, ..plain() };
    assert!(find(&s, "x*", re).unwrap().is_empty(), "zero-length matches are skipped");
}

#[test]
fn unicode_matches_fold_case_and_report_byte_ranges() {
    let s = [src(1, Segment::Main, "Ünï café ÜNÏ")];
    let found = find(&s, "üNï", plain()).unwrap();
    assert_eq!(ranges(&found), vec![(1, 0..5), (1, 12..17)]);
    for m in &found {
        assert!(s[0].text.is_char_boundary(m.range.start) && s[0].text.is_char_boundary(m.range.end));
    }
    assert_eq!(ranges(&find(&s, "café", plain()).unwrap()), vec![(1, 6..11)]);
}

#[test]
fn matches_come_in_source_order_then_text_order() {
    let s = [
        src(2, Segment::Subject, "go go"),
        src(1, Segment::Main, "go"),
        src(1, Segment::Quoted, "go go"),
        src(3, Segment::Main, "go"),
    ];
    let found = find(&s, "go", plain()).unwrap();
    let order: Vec<_> = found.iter().map(|m| (m.msg, m.segment, m.range.start)).collect();
    assert_eq!(
        order,
        vec![
            (2, Segment::Subject, 0),
            (2, Segment::Subject, 3),
            (1, Segment::Main, 0),
            (1, Segment::Quoted, 0),
            (1, Segment::Quoted, 3),
            (3, Segment::Main, 0),
        ]
    );
}

fn thread() -> Mailbox {
    let m = |id: u32, day: u32, subject: &str, body: &str| {
        serde_json::json!({
            "id": id, "thread_id": 7, "from_name": "A", "from_email": "a@x.com", "to": "me@x.com",
            "subject": subject, "body": body, "received": format!("2026-09-{day:02}T09:00:00Z"),
        })
    };
    Mailbox::from_json(
        &serde_json::json!([
            m(2, 2, "Re: Budget", "Agreed on the budget.\n\n> The budget is tight."),
            m(1, 1, "Budget", "Draft budget attached."),
            m(3, 3, "Re: Budget", "Thanks"),
        ])
        .to_string(),
    )
    .expect("valid mailbox")
}

#[test]
fn thread_sources_cover_subject_every_body_and_hidden_quotes_oldest_first() {
    let mb = thread();
    let sources = thread_sources(mb.messages(), 7, 3);
    let shape: Vec<_> = sources.iter().map(|s| (s.msg, s.segment)).collect();
    assert_eq!(
        shape,
        vec![
            (3, Segment::Subject),
            (1, Segment::Main),
            (2, Segment::Main),
            (2, Segment::Quoted),
            (3, Segment::Main),
        ]
    );
    assert_eq!(sources[0].text, "Re: Budget", "the opened message's subject");
}

#[test]
fn finding_in_a_thread_spans_messages_segments_and_collapsed_text() {
    let mb = thread();
    let found = find_in_thread(mb.messages(), 7, 3, "budget", plain()).unwrap();
    let shape: Vec<_> = found.iter().map(|m| (m.msg, m.segment)).collect();
    assert_eq!(
        shape,
        vec![
            (3, Segment::Subject),
            (1, Segment::Main),
            (2, Segment::Main),
            (2, Segment::Quoted),
        ]
    );
    let by_msg = highlights(&found, 2, 2, Segment::Main);
    assert_eq!(by_msg, vec![(found[2].range.clone(), true)], "flags the current match");
    assert!(highlights(&found, 2, 1, Segment::Main).iter().all(|(_, current)| !current));
}

#[test]
fn the_current_match_wraps_and_clamps() {
    let mut f = Find::default();
    f.set_query("x");
    assert_eq!(f.current(0), 0);
    f.step(0, 1);
    assert_eq!(f.current(0), 0, "no matches: nothing to step to");
    f.step(3, 1);
    f.step(3, 1);
    assert_eq!(f.current(3), 2);
    f.step(3, 1);
    assert_eq!(f.current(3), 0, "wraps forward");
    f.step(3, -1);
    assert_eq!(f.current(3), 2, "wraps backward");
    assert_eq!(f.current(2), 1, "clamped when matches disappear");
    f.set_query("x");
    assert_eq!(f.current(3), 2, "same text keeps the position");
    f.set_query("xy");
    assert_eq!(f.current(3), 0, "new text starts over");
    f.step(3, 1);
    f.set_options(Options { regex: true, ..Options::default() });
    assert_eq!(f.current(3), 0, "new options start over");
}
