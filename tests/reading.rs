use mail_classifier::model::{Attachment, Message, TriageState};
use mail_classifier::reading::{
    Recipient, ReaderView, format_size, html_to_text, initials, parse_recipients, reader_text,
    received_label, recipient_line, safe_html, split_quoted, thread_others, thread_position,
};

fn msg(id: u32, thread_id: u32, received: &str) -> Message {
    Message {
        id,
        thread_id,
        from_name: "Dana Whitfield".into(),
        from_email: "dana@example.com".into(),
        to: "me@example.com".into(),
        subject: "Subject".into(),
        body: "Body".into(),
        received: received.into(),
        state: TriageState::Inbox,
        account: "personal".into(),
        outgoing: false,
        snooze: None,
        cc: String::new(),
        bcc: String::new(),
        html: None,
        attachments: Vec::new(),
        read: false,
        partial: false,
    }
}

// ---------------------------------------------------------------- recipients

#[test]
fn parse_recipients_is_tolerant() {
    let got = parse_recipients(
        r#""Chen, Alice" <alice@x.io>, ,, bob@x.io , 'Carol' <carol@x.io>, Dave <dave@x.io"#,
    );
    let pairs: Vec<(Option<&str>, &str)> =
        got.iter().map(|r| (r.name.as_deref(), r.email.as_str())).collect();
    assert_eq!(
        pairs,
        vec![
            (Some("Chen, Alice"), "alice@x.io"),
            (None, "bob@x.io"),
            (Some("Carol"), "carol@x.io"),
            (Some("Dave"), "dave@x.io"),
        ]
    );
    assert!(parse_recipients("").is_empty());
    assert!(parse_recipients(" , ,").is_empty());
    assert_eq!(
        parse_recipients("<solo@x.io>"),
        vec![Recipient { name: None, email: "solo@x.io".into() }]
    );
}

#[test]
fn recipient_line_groups_by_field_and_names_you() {
    let mut m = msg(1, 1, "2026-09-01T10:00:00Z");
    assert_eq!(recipient_line(&m, "me@example.com"), "to You");
    // `me` matches case-insensitively, whatever the display name says.
    m.to = "Somebody <ME@Example.com>, Ben Ito <ben@x.io>".into();
    m.cc = "Dev Rao <dev@x.io>".into();
    assert_eq!(recipient_line(&m, "me@example.com"), "to You, Ben Ito · cc Dev Rao");
    m.cc = String::new();
    m.bcc = "hid@x.io".into();
    assert_eq!(recipient_line(&m, "me@example.com"), "to You, Ben Ito · bcc hid@x.io");
    // An empty `me` never matches an empty address.
    m.to = "bob@x.io".into();
    m.bcc = String::new();
    assert_eq!(recipient_line(&m, ""), "to bob@x.io");
}

#[test]
fn recipient_line_counts_an_address_once_and_handles_none() {
    let mut m = msg(1, 1, "2026-09-01T10:00:00Z");
    m.to = "a@x.io".into();
    m.cc = "A@X.io".into();
    m.bcc = "a@x.io".into();
    assert_eq!(recipient_line(&m, "me@example.com"), "to a@x.io");
    m.to = String::new();
    m.cc = String::new();
    m.bcc = String::new();
    assert_eq!(recipient_line(&m, "me@example.com"), "");
}

#[test]
fn received_label_formats_as_written() {
    assert_eq!(received_label("2026-09-30T20:46:00Z"), "Sep 30, 2026 · 20:46");
    assert_eq!(received_label("2026-01-05T07:03:00+02:00"), "Jan 5, 2026 · 07:03");
    assert_eq!(received_label("garbage"), "garbage");
}

// -------------------------------------------------------------------- quoted

#[test]
fn split_quoted_at_on_wrote_line() {
    let body = "Sounds good, see you then.\n\nOn Mon, Sep 28, 2026 at 9:00 AM Alice <a@x.io> wrote:\n> Are we on?\n> Yes.\n";
    let q = split_quoted(body);
    assert_eq!(q.main, "Sounds good, see you then.");
    assert_eq!(
        q.quoted,
        Some("On Mon, Sep 28, 2026 at 9:00 AM Alice <a@x.io> wrote:\n> Are we on?\n> Yes.\n")
    );
}

#[test]
fn split_quoted_handles_wrapped_attribution() {
    let body = "Thanks!\n\nOn Mon, Sep 28, 2026 at 9:00 AM Alice Chen <alice.chen@example.com>\nwrote:\n> hi";
    let q = split_quoted(body);
    assert_eq!(q.main, "Thanks!");
    assert!(q.quoted.unwrap().starts_with("On Mon"));
}

#[test]
fn split_quoted_at_original_message_line() {
    let body = "Approved.\n\n-----Original Message-----\nFrom: Bob\nSent: Monday\n\nPlease approve.";
    let q = split_quoted(body);
    assert_eq!(q.main, "Approved.");
    assert_eq!(
        q.quoted,
        Some("-----Original Message-----\nFrom: Bob\nSent: Monday\n\nPlease approve.")
    );
}

#[test]
fn split_quoted_at_trailing_angle_block() {
    let body = "Reply text\nmore text\n> quoted one\n> quoted two\n\n";
    let q = split_quoted(body);
    assert_eq!(q.main, "Reply text\nmore text");
    assert_eq!(q.quoted, Some("> quoted one\n> quoted two\n\n"));
}

#[test]
fn split_quoted_earliest_marker_wins() {
    let body = "Top\nOn Tue, Bob wrote:\n> a\n> b";
    assert_eq!(split_quoted(body).main, "Top");
    // A quote starting on the very first line leaves an empty main.
    let q = split_quoted("> only quoted\n> lines");
    assert_eq!(q.main, "");
    assert_eq!(q.quoted, Some("> only quoted\n> lines"));
}

#[test]
fn split_quoted_without_quote_returns_whole_body() {
    let body = "Just a note.\n\nNo history here.\n\n";
    let q = split_quoted(body);
    assert_eq!(q.main, "Just a note.\n\nNo history here.");
    assert_eq!(q.quoted, None);
    // Quoting mid-body followed by new text is not a trailing block.
    let inline = "> earlier point\nMy answer to that.";
    let q = split_quoted(inline);
    assert_eq!(q.main, inline);
    assert_eq!(q.quoted, None);
    // "On" sentences that are not attributions stay in the body.
    assert_eq!(split_quoted("On reflection, yes.").quoted, None);
    assert_eq!(split_quoted("").quoted, None);
}

// ---------------------------------------------------------------------- HTML

#[test]
fn safe_html_blocks_remote_images_and_keeps_data_images() {
    let html = r#"<p>Hi</p><img src="https://t.example.com/pixel.gif"><img src="data:image/png;base64,AAAA" alt="kept"><img src="cid:logo" alt="Logo &amp; Co">"#;
    let safe = safe_html(html);
    assert_eq!(safe.blocked_images, 2);
    assert_eq!(
        safe.html,
        r#"<p>Hi</p><img src="data:image/png;base64,AAAA" alt="kept">Logo &amp; Co"#
    );
}

#[test]
fn safe_html_tolerates_broken_markup() {
    // Uppercase tag, unquoted src, single quotes, slash separators, no src.
    let html = "<IMG SRC=http://a.example/1.png ALT=one>\
                <img src='http://a.example/2.png' alt='<b>two</b>'/>\
                <img/src=http://a.example/3.png>\
                <Img alt=\"nosrc\">\
                <IMG SRC=DATA:image/gif;base64,R0lG>";
    let safe = safe_html(html);
    assert_eq!(safe.blocked_images, 4);
    assert_eq!(
        safe.html,
        "one&lt;b&gt;two&lt;/b&gt;nosrc<IMG SRC=DATA:image/gif;base64,R0lG>"
    );
}

#[test]
fn safe_html_ignores_img_text_that_is_not_an_element() {
    let html = concat!(
        r#"<a title="<img src=http://evil.example/x.png>" href=#>link</a>"#,
        "<!-- <img src=http://evil.example/c.png> -->",
        "<script>var s = '<img src=http://evil.example/s.png>';</script>",
        "<style>/* <img src=x> */</style>",
        "<p>1 < 2 and 3 > 2</p>",
    );
    let safe = safe_html(html);
    assert_eq!(safe.blocked_images, 0);
    assert_eq!(safe.html, html);
}

#[test]
fn safe_html_blocks_images_hidden_behind_unclosed_markup() {
    // Unclosed quote: a browser would swallow the rest, so the tag is dropped.
    let safe = safe_html(r#"<p>Hello</p><img src="http://a.example/x.png alt=broken <b>tail</b>"#);
    assert_eq!(safe.blocked_images, 1);
    assert_eq!(safe.html, "<p>Hello</p>");

    // Unclosed <p> and <b> around an image: the image is still found, and the
    // surrounding markup is not rewritten.
    let safe = safe_html("<p>one<b>two<img src=http://a.example/y.png alt=Y>three");
    assert_eq!(safe.blocked_images, 1);
    assert_eq!(safe.html, "<p>one<b>twoYthree");

    // Entity-obfuscated data: prefix is decoded before the check.
    let safe = safe_html(r#"<img src="&#100;ata:image/png;base64,AA">"#);
    assert_eq!(safe.blocked_images, 0);
}

#[test]
fn safe_html_without_images_is_identity() {
    let html = "<table><tr><td>Total</td><td>$5</td></tr></table>é<br>";
    assert_eq!(safe_html(html), mail_classifier::reading::SafeHtml { html: html.into(), blocked_images: 0 });
}

#[test]
fn html_to_text_lists_paragraphs_and_breaks() {
    let html = "<h1>Receipt</h1><p>Thanks for your order.</p><ul><li>Widget</li><li>Gadget <b>Pro</b></li></ul><p>Line one<br>Line two</p>";
    assert_eq!(
        html_to_text(html),
        "Receipt\n\nThanks for your order.\n\n- Widget\n- Gadget Pro\n\nLine one\nLine two"
    );
}

#[test]
fn html_to_text_decodes_entities() {
    assert_eq!(
        html_to_text("<p>Fish &amp; chips &lt;3 &quot;quoted&quot; &#8364;5 &#x41; &mdash; done&nbsp;&nbsp;ok &bogus; &#xZZ;</p>"),
        "Fish & chips <3 \"quoted\" €5 A — done ok &bogus; &#xZZ;"
    );
}

#[test]
fn html_to_text_drops_script_style_and_head() {
    let html = "<html><head><title>Hidden</title><style>p{color:red}</style></head><body><script>alert('x <b>')</script><p>Visible</p><style>.a{}</style></body></html>";
    assert_eq!(html_to_text(html), "Visible");
    assert_eq!(html_to_text("<script>never closed <p>still script"), "");
    assert_eq!(html_to_text("a<!-- <p>comment</p> -->b"), "ab");
}

#[test]
fn html_to_text_tolerates_broken_markup() {
    // Unclosed head still ends when body content starts.
    assert_eq!(html_to_text("<head><title>T</title><p>Body text"), "Body text");
    // Uppercase tags, unclosed tags, lone '<', trailing unterminated tag.
    assert_eq!(
        html_to_text("<DIV>one<P>two &lt; three<br/>four<B>five</DIV><span"),
        "one\n\ntwo < three\nfourfive"
    );
    assert_eq!(html_to_text("a < b and c"), "a < b and c");
    assert_eq!(html_to_text(r#"<a title="x > y" href=z>label</a>"#), "label");
}

#[test]
fn html_to_text_separates_table_cells_and_collapses_blank_runs() {
    let html = "<table><tr><td>Item</td><td>Qty</td></tr><tr><td>Widget</td><td>2</td></tr></table><div></div><div></div><p>   </p><p>End</p>";
    assert_eq!(html_to_text(html), "Item  Qty\nWidget  2\n\nEnd");
}

#[test]
fn html_to_text_keeps_preformatted_whitespace() {
    assert_eq!(
        html_to_text("<pre>fn main() {\n    run();\n}</pre><p>after</p>"),
        "fn main() {\n    run();\n}\n\nafter"
    );
}

#[test]
fn reader_text_prefers_text_part_then_html() {
    let mut m = msg(1, 1, "2026-09-01T10:00:00Z");
    m.html = Some("<p>From html</p>".into());
    assert_eq!(reader_text(&m), "Body");
    m.body = " \n\t".into();
    assert_eq!(reader_text(&m), "From html");
    m.html = None;
    assert_eq!(reader_text(&m), "");
}

// ------------------------------------------------------------------- labels

#[test]
fn format_size_scales_units() {
    assert_eq!(format_size(0), "0 B");
    assert_eq!(format_size(812), "812 B");
    assert_eq!(format_size(1023), "1023 B");
    assert_eq!(format_size(1024), "1 KB");
    assert_eq!(format_size(1536), "1.5 KB");
    assert_eq!(format_size(14 * 1024), "14 KB");
    assert_eq!(format_size(1_258_291), "1.2 MB");
    assert_eq!(format_size(1024 * 1024 - 1), "1 MB");
    assert_eq!(format_size(3 * 1024 * 1024 * 1024), "3 GB");
}

#[test]
fn initials_use_name_then_email_and_are_unicode_safe() {
    assert_eq!(initials("Dana Whitfield", "x@y.z"), "DW");
    assert_eq!(initials("dana", "x@y.z"), "D");
    assert_eq!(initials("Dana Q. Whitfield", "x@y.z"), "DW");
    assert_eq!(initials("\"Dana\" (Ops) Whitfield", "x@y.z"), "DW");
    assert_eq!(initials("", "dana.whitfield@example.com"), "DW");
    assert_eq!(initials("  ", "billing@example.com"), "B");
    assert_eq!(initials("noreply@example.com", "noreply@example.com"), "N");
    assert_eq!(initials("łukasz żak", "x@y.z"), "ŁŻ");
    assert_eq!(initials("", ""), "?");
    assert_eq!(initials("Ünal", ""), "Ü");
}

// ------------------------------------------------------------------- threads

#[test]
fn thread_position_is_chronological_by_received_then_id() {
    let messages = vec![
        msg(5, 1, "2026-09-03T10:00:00Z"),
        msg(2, 1, "2026-09-01T10:00:00Z"),
        msg(9, 2, "2026-09-02T10:00:00Z"),
        msg(4, 1, "2026-09-03T10:00:00Z"),
        msg(7, 1, "2026-09-02T10:00:00Z"),
    ];
    let pos = |id: u32| {
        let opened = messages.iter().find(|m| m.id == id).unwrap();
        thread_position(&messages, opened)
    };
    assert_eq!(pos(2), (1, 4));
    assert_eq!(pos(7), (2, 4));
    assert_eq!(pos(4), (3, 4));
    assert_eq!(pos(5), (4, 4));
    assert_eq!(pos(9), (1, 1));
}

#[test]
fn thread_position_counts_an_opened_message_outside_the_slice() {
    let messages = vec![msg(2, 1, "2026-09-01T10:00:00Z"), msg(4, 1, "2026-09-03T10:00:00Z")];
    let outsider = msg(3, 1, "2026-09-02T10:00:00Z");
    assert_eq!(thread_position(&messages, &outsider), (2, 3));
}

#[test]
fn thread_others_are_same_thread_newest_first_without_opened() {
    let messages = vec![
        msg(5, 1, "2026-09-03T10:00:00Z"),
        msg(2, 1, "2026-09-01T10:00:00Z"),
        msg(9, 2, "2026-09-02T10:00:00Z"),
        msg(4, 1, "2026-09-03T10:00:00Z"),
        msg(7, 1, "2026-09-02T10:00:00Z"),
    ];
    let opened = messages.iter().find(|m| m.id == 7).unwrap();
    // 4 and 5 share a timestamp: the higher id is newer.
    assert_eq!(thread_others(&messages, opened), vec![5, 4, 2]);
    let lonely = messages.iter().find(|m| m.id == 9).unwrap();
    assert!(thread_others(&messages, lonely).is_empty());
}

// --------------------------------------------------------------- view state

#[test]
fn reader_view_toggles_each_set_independently() {
    let mut v = ReaderView::default();
    assert!(!v.is_expanded(1, 10));
    v.toggle_expanded(1, 10);
    v.toggle_recipients(1, 11);
    v.toggle_quoted(1, 12);
    v.toggle_plain(1, 13);
    assert!(v.is_expanded(1, 10) && !v.is_expanded(1, 11));
    assert!(v.recipients_open(1, 11) && !v.recipients_open(1, 10));
    assert!(v.quoted_open(1, 12) && !v.quoted_open(1, 10));
    assert!(v.plain(1, 13) && !v.plain(1, 10));
    v.toggle_expanded(1, 10);
    assert!(!v.is_expanded(1, 10));
    assert!(v.plain(1, 13), "other sets are untouched");
}

#[test]
fn reader_view_keeps_each_threads_state_and_forgets_on_request() {
    let mut v = ReaderView::default();
    v.toggle_expanded(1, 10);
    v.toggle_recipients(1, 10);
    v.toggle_quoted(1, 10);
    v.toggle_plain(1, 10);

    // Another thread sees nothing of it, and toggling there leaves thread 1 alone.
    assert!(!v.is_expanded(2, 10) && !v.recipients_open(2, 10) && !v.quoted_open(2, 10) && !v.plain(2, 10));
    v.toggle_expanded(2, 20);
    assert!(v.is_expanded(2, 20) && v.is_expanded(1, 10));
    assert!(v.recipients_open(1, 10) && v.quoted_open(1, 10) && v.plain(1, 10));

    v.forget(1);
    assert!(!v.is_expanded(1, 10) && !v.recipients_open(1, 10) && !v.quoted_open(1, 10) && !v.plain(1, 10));
    assert!(v.is_expanded(2, 20), "forgetting one thread keeps the others");
}

#[test]
fn reader_view_toggle_all_expands_then_collapses() {
    let mut v = ReaderView::default();
    let ids = [1, 2, 3];
    v.toggle_all(7, &ids);
    assert!(ids.iter().all(|&id| v.is_expanded(7, id)));
    v.toggle_all(7, &ids);
    assert!(ids.iter().all(|&id| !v.is_expanded(7, id)));

    // A partly expanded thread expands fully first.
    v.toggle_expanded(7, 2);
    v.toggle_all(7, &ids);
    assert!(ids.iter().all(|&id| v.is_expanded(7, id)));

    // Collapse-all leaves messages outside `ids` alone; an empty list is a no-op.
    v.toggle_expanded(7, 99);
    v.toggle_all(7, &ids);
    assert!(ids.iter().all(|&id| !v.is_expanded(7, id)) && v.is_expanded(7, 99));
    v.toggle_all(7, &[]);
    assert!(v.is_expanded(7, 99));

    // toggle_all on another thread leaves this one alone.
    v.toggle_all(8, &[5]);
    assert!(v.is_expanded(8, 5) && v.is_expanded(7, 99));
}

#[test]
fn attachment_round_trips_through_message_json() {
    let mut m = msg(1, 1, "2026-09-01T10:00:00Z");
    m.cc = "c@x.io".into();
    m.html = Some("<p>x</p>".into());
    m.attachments = vec![Attachment { name: "a.pdf".into(), size: 2048 }];
    let json = serde_json::to_string(&m).unwrap();
    assert_eq!(serde_json::from_str::<Message>(&json).unwrap(), m);

    // Old fixtures without the new fields still load, with empty defaults.
    let old = r#"{"id":1,"thread_id":1,"from_name":"A","from_email":"a@x.io","to":"b@x.io",
        "subject":"s","body":"b","received":"2026-09-01T10:00:00Z"}"#;
    let m: Message = serde_json::from_str(old).unwrap();
    assert!(m.cc.is_empty() && m.bcc.is_empty() && m.html.is_none() && m.attachments.is_empty());
}
