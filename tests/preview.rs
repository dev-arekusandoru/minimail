use mail_classifier::model::Mailbox;
use mail_classifier::preview::*;

#[test]
fn collapses_whitespace_and_blank_lines() {
    assert_eq!(snippet("  Hello   there \n\n\n  second\tline  \n"), "Hello there second line");
}

#[test]
fn drops_quoted_text_and_signature() {
    let body = "Sounds good.\n> earlier text\n> more\nSee you Friday.\n\n-- \nMarta\nDesigner";
    assert_eq!(snippet(body), "Sounds good. See you Friday.");
}

#[test]
fn stops_at_reply_header() {
    let body = "Yes please.\n\nOn Mon, 5 Oct 2026, Sam wrote:\n> old\nnot included";
    assert_eq!(snippet(body), "Yes please.");
    assert_eq!(snippet("Thanks\nSent from my iPhone\nignored"), "Thanks");
}

#[test]
fn empty_and_quote_only_bodies_give_empty_snippet() {
    assert_eq!(snippet(""), "");
    assert_eq!(snippet("> a\n> b\n"), "");
}

#[test]
fn snippet_is_capped_on_char_boundaries() {
    let body = "é".repeat(2000);
    assert_eq!(snippet(&body).chars().count(), 480);
}

#[test]
fn preview_lines_options_and_steps() {
    assert_eq!(OPTIONS, [0, 1, 2, 3, 4, 5]);
    assert_eq!(DEFAULT_LINES, 2);
    assert_eq!(lines_label(0), "Off");
    assert_eq!(lines_label(1), "1 line");
    assert_eq!(lines_label(5), "5 lines");
    assert_eq!(step_lines(0, -1), 0);
    assert_eq!(step_lines(5, 1), 5);
    assert_eq!(step_lines(2, 1), 3);
    assert_eq!(cycle_lines(5), 0);
    assert_eq!(cycle_lines(0), 1);
}

#[test]
fn attachment_heuristic() {
    assert!(mentions_attachment("Lisbon print run — proofs attached", "Four proofs."));
    assert!(mentions_attachment("Boarding", "Boarding pass attached.\n\nGate B22"));
    assert!(!mentions_attachment("Statement", "Download the PDF; we no longer attach statements."));
    assert!(!mentions_attachment("Hi", "Lunch?"));
}

#[test]
fn fixture_snippets_are_plain_single_paragraph_text() {
    let mailbox = Mailbox::load_default();
    for m in mailbox.messages() {
        let s = snippet(&m.body);
        assert!(!s.contains('\n'), "message {}", m.id);
        assert!(!s.starts_with('>'));
    }
}
