//! Gmail response decoding and uncertainty boundaries. No network.

use base64::Engine;
use mail_classifier::provider::ProviderError;
use mail_classifier::provider::compose::{ComposeError, SendStatus};
use mail_classifier::provider::gmail::drafts;
use serde_json::{Value, json};

const NOW: i64 = 1_790_086_400;

fn b64(bytes: &[u8]) -> String {
    drafts::encode_engine().encode(bytes)
}

fn header(name: &str, value: &str) -> Value {
    json!({"name": name, "value": value})
}

/// A `drafts.get?format=full` response for a two-part draft with one file.
fn draft_resource(draft: &str, message: &str, raw: &[u8]) -> Value {
    json!({
        "id": draft,
        "message": {
            "id": message,
            "threadId": "t-1",
            "snippet": "Hello there",
            "internalDate": "1790086400000",
            "sizeEstimate": raw.len(),
            "payload": {
                "mimeType": "multipart/mixed",
                "headers": [
                    header("To", "One <one@x.io>, two@x.io"),
                    header("Message-ID", "<stored@x.io>"),
                    header("Cc", "cc@x.io"),
                    header("Subject", "Lunch"),
                ],
                "parts": [
                    {
                        "mimeType": "multipart/alternative",
                        "parts": [
                            {"mimeType": "text/plain", "body": {"size": 12, "data": b64(b"Hello there")}},
                            {"mimeType": "text/html", "body": {"size": 30, "data": b64(b"<b>Hello there</b>")}},
                        ],
                    },
                    {
                        "mimeType": "application/pdf",
                        "filename": "menu.pdf",
                        "body": {"size": 4, "attachmentId": "att-1"},
                    },
                ],
            },
        },
    })
}

// ------------------------------------------------------------ gmail parsing

#[test]
fn a_full_draft_yields_headers_body_and_revision() {
    let raw = b"raw message";
    let draft = drafts::draft(&draft_resource("d-1", "m-1", raw)).expect("draft");
    assert_eq!(draft.id, "d-1");
    // The revision is the underlying message id, which Gmail replaces on every
    // content replacement: that is what makes a stale update detectable.
    assert_eq!(draft.revision, "m-1");
    assert_eq!(draft.message, "m-1");
    // The RFC id of the stored message: what a send of these exact bytes would
    // be reconciled by, and what a fallback import must reuse.
    assert_eq!(draft.message_id.as_deref(), Some("stored@x.io"));
    assert_eq!(draft.thread, "t-1");
    assert_eq!(draft.to, "One <one@x.io>, two@x.io");
    assert_eq!(draft.cc, "cc@x.io");
    assert_eq!(draft.subject, "Lunch");
    assert_eq!(draft.body.text, "Hello there");
    assert_eq!(draft.body.html.as_deref(), Some("<b>Hello there</b>"));
    assert_eq!(draft.updated, NOW);
    assert!(draft.has_attachments);
    assert!(
        draft.attachments_known,
        "a full draft has its payload walked"
    );
    let file = &draft.attachments[0];
    assert_eq!(file.name, "menu.pdf");
    assert_eq!(file.id.as_deref(), Some("att-1"));
    assert_eq!(file.content_type, "application/pdf");
    assert_eq!(file.size, 4);
    // Not inline, so the bytes still need fetching.
    assert_eq!(file.inline_data, None);
}

#[test]
fn a_size_estimate_alone_does_not_make_a_draft_look_like_it_has_files() {
    // `sizeEstimate` counts the body, so every draft has one. Reading it as
    // "has attachments" would put a phantom file chip on every draft.
    let json = json!({
        "id": "d-1",
        "message": {
            "id": "m-1",
            "threadId": "t-1",
            "sizeEstimate": 2048,
            "snippet": "Hello there",
            "payload": {
                "mimeType": "text/plain",
                "headers": [header("Subject", "Lunch")],
                "body": {"size": 12, "data": b64(b"Hello there")},
            },
        },
    });
    let draft = drafts::draft(&json).expect("draft");
    assert!(!draft.has_attachments);
    assert!(draft.attachments_known);
    assert!(draft.attachments.is_empty());
    assert_eq!(draft.body.text, "Hello there");
    assert_eq!(draft.body.html, None);
}

#[test]
fn a_listed_draft_reports_its_files_as_unknown_rather_than_as_none() {
    let page = drafts::draft_page(&json!({
        "drafts": [{"id": "d-1", "message": {"id": "m-1", "threadId": "t-1", "snippet": "Hi"}}],
    }));
    let draft = &page.drafts[0];
    // A listed draft carries no payload, so the provider genuinely does not
    // know; the caller must not read that as "this draft has no files".
    assert!(!draft.attachments_known);
    assert!(!draft.has_attachments);
    assert_eq!(draft.body.text, "Hi");
    // No headers means no RFC id either: a caller must not guess one.
    assert_eq!(draft.message_id, None);
}

#[test]
fn draft_raw_round_trips_the_exact_bytes_that_went_in() {
    let raw = b"To: one@x.io\r\nSubject: Lunch\r\n\r\nbody with \xe2\x9c\x93".to_vec();
    let json = json!({"id": "d-1", "message": {"id": "m-1", "raw": b64(&raw)}});
    assert_eq!(drafts::draft_raw(&json).as_deref(), Some(raw.as_slice()));
}

#[test]
fn a_send_response_is_a_confirmed_sent_message() {
    let json = json!({
        "id": "sent-1",
        "threadId": "t-1",
        "internalDate": "1790086400000",
        "payload": {"headers": [header("Message-ID", "<mine@x.io>")]},
    });
    match drafts::send_status(&json) {
        SendStatus::Sent(sent) => {
            assert_eq!(sent.id, "sent-1");
            assert_eq!(sent.thread, "t-1");
            assert_eq!(sent.message_id, "mine@x.io");
            assert_eq!(sent.sent_at, NOW);
        }
        other => panic!("expected a confirmed send, got {other:?}"),
    }
}

#[test]
fn a_send_response_without_a_message_is_not_a_confirmed_send() {
    let status = drafts::send_status(&json!({}));
    assert!(matches!(status, SendStatus::Uncertain { .. }));
}

#[test]
fn reconciliation_requires_the_message_id_header_to_match() {
    let hit = json!({
        "id": "sent-1",
        "threadId": "t-1",
        "internalDate": "1790086400000",
        "payload": {"headers": [header("Message-ID", "<mine@x.io>")]},
    });
    assert_eq!(
        drafts::reconcile_sent(&hit, "<mine@x.io>").map(|s| s.id),
        Some("sent-1".into())
    );
    // A search hit that carries somebody else's id is not this send.
    assert_eq!(drafts::reconcile_sent(&hit, "other@x.io"), None);
    assert_eq!(drafts::sent_query("<mine@x.io>"), "rfc822msgid:mine@x.io");
}

#[test]
fn threading_chains_references_and_appends_the_message_itself() {
    let json = json!({"payload": {"headers": [
        header("Message-Id", "<m2@x.io>"),
        header("References", "<m0@x.io> <m1@x.io>"),
        header("Subject", "Re: Lunch"),
    ]}});
    let threading = drafts::threading(&json);
    assert_eq!(threading.message_id, "m2@x.io");
    // A reply to a reply must reference the whole conversation, so the
    // message's own id closes the chain.
    assert_eq!(threading.references, ["m0@x.io", "m1@x.io", "m2@x.io"]);
    assert_eq!(threading.subject, "Re: Lunch");
}

#[test]
fn threading_of_a_message_with_no_references_is_just_itself() {
    let json = json!({"payload": {"headers": [header("Message-ID", "<solo@x.io>")]}});
    assert_eq!(drafts::threading(&json).references, ["solo@x.io"]);
}

#[test]
fn attachment_bytes_come_back_from_the_response_data() {
    let payload = b"PDF bytes";
    let json = json!({"size": 9, "data": b64(payload)});
    assert_eq!(
        drafts::attachment_data(&json).as_deref(),
        Some(payload.as_slice())
    );
    assert_eq!(drafts::attachment_data(&json!({"size": 0})), None);
}

#[test]
fn a_parts_declared_media_type_survives_the_fetch() {
    // The attachment fetch response carries no type at all, so the listing is
    // the only authority on what a part is. Parameters are not the media type
    // and are dropped; an absent declaration stays absent so the caller's file
    // name can answer it, rather than being invented into octet-stream here.
    assert_eq!(drafts::media_type("image/png"), "image/png");
    assert_eq!(
        drafts::media_type("text/plain; charset=utf-8"),
        "text/plain"
    );
    assert_eq!(drafts::media_type(""), "");

    let full = draft_resource("d-1", "m-1", b"raw");
    let files = drafts::attachments(full.get("message").expect("message"));
    let pdf = files
        .iter()
        .find(|f| f.name == "menu.pdf")
        .expect("the pdf part");
    assert_eq!(pdf.content_type, "application/pdf");
}

#[test]
fn a_part_with_no_declared_type_keeps_its_bytes_and_gets_a_name() {
    // An extensionless attachment the provider did not describe: dropping it,
    // or leaving it nameless, loses the user's file. The type stays undeclared
    // so the caller's file name can decide it.
    let full = json!({
        "payload": {
            "mimeType": "multipart/mixed",
            "parts": [
                {"mimeType": "text/plain", "body": {"size": 2, "data": b64(b"hi")}},
                {"filename": "capture", "body": {"size": 3, "data": b64(b"raw")}},
            ],
        },
    });
    let files = drafts::attachments(&full);
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].name, "capture");
    assert_eq!(files[0].content_type, "");
    assert_eq!(files[0].inline_data.as_deref(), Some(&b"raw"[..]));
}

#[test]
fn a_reply_to_a_message_that_omitted_references_still_carries_its_parent() {
    // `References` is optional and some clients omit it while still naming the
    // parent in `In-Reply-To`. Building the reply from the id alone would put
    // this message at the head of a new conversation.
    let json = json!({"payload": {"headers": [
        header("Message-ID", "<child@x.io>"),
        header("In-Reply-To", "<parent@x.io>"),
        header("Subject", "Re: Lunch"),
    ]}});
    let threading = drafts::threading(&json);
    assert_eq!(threading.references, ["parent@x.io", "child@x.io"]);
}

#[test]
fn the_parent_is_not_invented_when_the_message_declares_none() {
    // A fresh message has no parent, so a reply to it must not be handed one.
    let json = json!({"payload": {"headers": [header("Message-ID", "<solo@x.io>")]}});
    assert_eq!(drafts::threading(&json).references, ["solo@x.io"]);
}

#[test]
fn a_declared_reference_chain_wins_over_a_bare_in_reply_to() {
    // The chain is the whole conversation; `In-Reply-To` names only the direct
    // parent, so it must not truncate a longer chain into a single link.
    let json = json!({"payload": {"headers": [
        header("Message-ID", "<m2@x.io>"),
        header("References", "<m0@x.io> <m1@x.io>"),
        header("In-Reply-To", "<m1@x.io>"),
    ]}});
    assert_eq!(
        drafts::threading(&json).references,
        ["m0@x.io", "m1@x.io", "m2@x.io"]
    );
}

#[test]
fn an_interrupted_save_is_uncertain_but_never_a_possible_delivery() {
    let error = ComposeError::AutosaveUncertain {
        detail: "response lost".into(),
    };
    assert!(error.is_uncertain(), "the save may not have landed");
    assert!(!error.needs_send_reconciliation(), "but no mail was sent");
    assert!(
        !error.is_retryable(),
        "writing again could duplicate a draft"
    );
}

#[test]
fn in_reply_to_is_read_out_of_the_mime_itself() {
    assert!(drafts::is_reply(
        b"Subject: x\r\nIn-Reply-To: <m0@x.io>\r\n\r\nbody"
    ));
    assert!(drafts::is_reply(
        b"subject: x\r\nin-reply-to: <m0@x.io>\r\n\r\nbody"
    ));
    assert!(!drafts::is_reply(b"Subject: x\r\n\r\nbody"));
}

#[test]
fn uploads_are_base64url_without_padding() {
    // 0xfb 0xff encodes to "-_" and nothing else, so padding would show here.
    let body = drafts::raw_request(&[0xfb, 0xff, 0xfe], None);
    assert_eq!(body["message"]["raw"], "-__-");
    assert!(!body["message"]["raw"].as_str().unwrap().contains('='));
}

#[test]
fn a_reply_carries_the_requested_thread_but_a_new_message_does_not() {
    let reply =
        b"Subject: Re: Lunch\r\nIn-Reply-To: <m0@x.io>\r\nReferences: <m0@x.io>\r\n\r\nreply";
    let fresh = b"Subject: Lunch\r\n\r\nnew message";
    // Gmail threads a message only when threadId, RFC In-Reply-To/References
    // and the Subject all line up, so the hint rides along with a real reply.
    assert_eq!(
        drafts::raw_request(reply, Some("t-1"))["message"]["threadId"],
        "t-1"
    );
    // A message with no In-Reply-To is not a reply, and forcing it into a
    // thread would misfile it.
    assert_eq!(
        drafts::raw_request(fresh, Some("t-1"))["message"].get("threadId"),
        None
    );
}

#[test]
fn send_names_the_draft_in_the_body_not_the_path() {
    let raw = b"Subject: Re: Lunch\r\nIn-Reply-To: <m0@x.io>\r\n\r\nreply";
    // Without replacement content the draft is sent as it stands, and the id
    // is the only thing in the body.
    assert_eq!(drafts::send_request("d-1", None), json!({"id": "d-1"}));
    let with = drafts::send_request("d-1", Some((raw, Some("t-1"))));
    assert_eq!(with["id"], "d-1");
    assert_eq!(with["message"]["threadId"], "t-1");
    assert!(with["message"]["raw"].is_string());
}

// ------------------------------------------------------------ error verdict

#[test]
fn a_rejected_request_is_certain_but_not_worth_retrying() {
    let rejected = ComposeError::Provider(ProviderError::Api {
        status: 400,
        message: "invalid recipient".into(),
    });
    // The server said no before acting, so nothing was applied…
    assert!(!rejected.is_uncertain());
    // …but asking again unchanged will be rejected again.
    assert!(!rejected.is_retryable());
    // A throttle can recover; revoked credentials require reconnecting first.
    assert!(!ComposeError::Provider(ProviderError::Auth("revoked".into())).is_retryable());
    assert!(ComposeError::Provider(ProviderError::RateLimited).is_retryable());
}

#[test]
fn going_offline_is_retryable_and_carries_nothing_to_reconcile() {
    let offline = ComposeError::Offline;
    assert!(offline.is_retryable());
    assert!(!offline.is_uncertain());
}

#[test]
fn a_lost_response_is_uncertain_and_never_retryable() {
    for error in [
        ComposeError::Uncertain {
            detail: "response lost".into(),
        },
        ComposeError::Provider(ProviderError::Network("reset".into())),
        ComposeError::Provider(ProviderError::Api {
            status: 503,
            message: "backend".into(),
        }),
    ] {
        assert!(error.is_uncertain(), "{error} should be uncertain");
        // Repeating a request that may have been applied is how mail gets sent
        // twice, so uncertainty must never read as "just try again".
        assert!(!error.is_retryable(), "{error} must not be retryable");
    }
}

#[test]
fn a_conflict_is_neither_retryable_nor_uncertain() {
    let conflict = ComposeError::Conflict {
        draft: "d-1".into(),
        expected: "m-1".into(),
        actual: "m-2".into(),
    };
    // The caller's copy is still good; only the merge is the caller's problem.
    assert!(!conflict.is_retryable());
    assert!(!conflict.is_uncertain());
}

// ------------------------------------- decoding a raw message, shared by the
// mock, the imported-draft path and anything else that reads stored MIME.

use mail_classifier::provider::mime::{ParsedMessage, parse_message};

/// A composer-shaped message: multipart/mixed wrapping a multipart/alternative
/// with both parts base64, plus one file part.
const MULTIPART: &str = concat!(
    "To: one@x.io\r\nCc: two@x.io\r\nSubject: Lunch\r\n",
    "Message-ID: <m2@x.io>\r\nReferences: <m0@x.io>\r\n",
    "MIME-Version: 1.0\r\n",
    "Content-Type: multipart/mixed; boundary=\"OUTER\"\r\n\r\n",
    "--OUTER\r\nContent-Type: multipart/alternative; boundary=\"INNER\"\r\n\r\n",
    "--INNER\r\nContent-Type: text/plain\r\nContent-Transfer-Encoding: base64\r\n\r\n",
    "SGVsbG8gdGhlcmU=\r\n",
    "--INNER\r\nContent-Type: text/html\r\nContent-Transfer-Encoding: base64\r\n\r\n",
    "PGI+SGVsbG8gdGhlcmU8L2I+\r\n",
    "--INNER--\r\n",
    "--OUTER\r\nContent-Type: application/pdf\r\n",
    "Content-Disposition: attachment; filename=\"menu.pdf\"\r\n",
    "Content-Transfer-Encoding: base64\r\n\r\n",
    "JVBERi0xLjQK\r\n",
    "--OUTER--\r\n",
);

fn value_of(parsed: &ParsedMessage, name: &str) -> String {
    parsed
        .headers
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.clone())
        .unwrap_or_default()
}

#[test]
fn a_nested_multipart_message_yields_both_alternatives_and_its_file() {
    let parsed = parse_message(MULTIPART.as_bytes()).expect("valid MIME");
    // Both alternatives come back decoded, never as transfer-encoded text.
    assert_eq!(parsed.text, "Hello there");
    assert_eq!(parsed.html.as_deref(), Some("<b>Hello there</b>"));
    assert_eq!(parsed.files.len(), 1);
    assert_eq!(parsed.files[0].name, "menu.pdf");
    assert_eq!(parsed.files[0].content_type, "application/pdf");
    assert_eq!(parsed.files[0].bytes, b"%PDF-1.4\n");
    assert_eq!(value_of(&parsed, "subject"), "Lunch");
    assert_eq!(value_of(&parsed, "to"), "one@x.io");
    assert_eq!(value_of(&parsed, "cc"), "two@x.io");
    assert_eq!(value_of(&parsed, "message-id"), "<m2@x.io>");
    assert_eq!(value_of(&parsed, "references"), "<m0@x.io>");
}

#[test]
fn quoted_printable_and_encoded_words_are_decoded() {
    let raw = concat!(
        "Subject: =?UTF-8?Q?Caf=C3=A9?=\r\n",
        "Content-Type: text/plain; charset=utf-8\r\n",
        "Content-Transfer-Encoding: quoted-printable\r\n\r\n",
        "Caf=C3=A9=\r\n next line\r\n"
    );
    let parsed = parse_message(raw.as_bytes()).expect("valid MIME");
    assert_eq!(value_of(&parsed, "subject"), "Café");
    // The soft line break is a line break in the encoded text, not a stray `=`.
    assert_eq!(parsed.text, "Café next line\r\n");
}

#[test]
fn a_single_part_html_body_is_kept_verbatim_for_preservation() {
    let raw = "Content-Type: text/html; charset=utf-8\r\n\r\n<p style=\"color:red\">hi</p>";
    let parsed = parse_message(raw.as_bytes()).expect("valid MIME");
    // Styling the app cannot represent has to survive the read, or an imported
    // Gmail body would be silently flattened.
    assert_eq!(
        parsed.html.as_deref(),
        Some("<p style=\"color:red\">hi</p>")
    );
    assert!(parsed.text.is_empty());
    assert!(parsed.files.is_empty());
}

#[test]
fn a_base64_body_longer_than_one_line_is_reassembled() {
    // 76-column wrapping is what a real composer emits; a decoder that stops at
    // the first newline would truncate every long body.
    let body = "a".repeat(300);
    let encoded = {
        let raw = body.as_bytes();
        let chunks: Vec<String> = raw.chunks(57).map(super_b64).collect();
        chunks.join("\r\n")
    };
    let raw = format!(
        "Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n{encoded}"
    );
    let parsed = parse_message(raw.as_bytes()).expect("valid MIME");
    assert_eq!(parsed.text, body);
}

/// Standard base64, matching what a MIME encoder writes.
fn super_b64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(TABLE[(n >> 18 & 63) as usize] as char);
        out.push(TABLE[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6 & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[test]
fn undecodable_input_is_an_error_and_never_a_half_read_import() {
    // A message that cannot be read must not come back as an empty one: the
    // caller would import a body it believes the message never had.
    assert!(parse_message(b"").is_err());
}

#[test]
fn a_multipart_container_is_never_read_as_the_message_body() {
    // The container's own body is the raw source of its children; decoding it
    // would fill `text` with boundary markers and shadow the real alternative.
    let parsed = parse_message(MULTIPART.as_bytes()).expect("valid MIME");
    assert_eq!(parsed.text, "Hello there");
    assert!(!parsed.text.contains("--OUTER"));
    assert!(!parsed.text.contains("--INNER"));
    assert!(!parsed.html.unwrap().contains("boundary"));
}

#[test]
fn an_inline_cid_part_is_carried_with_the_id_the_body_refers_to() {
    // A body that points at an inline image by content id: dropping the part,
    // or renaming its id, silently breaks the image in an imported message.
    let raw = concat!(
        "Content-Type: multipart/related; boundary=\"REL\"\r\n\r\n",
        "--REL\r\nContent-Type: text/html; charset=utf-8\r\n\r\n",
        "<p><img src=\"cid:logo@x.io\"></p>\r\n",
        "--REL\r\nContent-Type: image/png\r\nContent-ID: <logo@x.io>\r\n",
        "Content-Transfer-Encoding: base64\r\n\r\n",
        "iVBORw0KGgo=\r\n",
        "--REL--\r\n",
    );
    let parsed = parse_message(raw.as_bytes()).expect("valid MIME");
    assert_eq!(
        parsed.html.as_deref(),
        Some("<p><img src=\"cid:logo@x.io\"></p>")
    );
    assert_eq!(parsed.files.len(), 1);
    let inline = &parsed.files[0];
    // The angle brackets are stripped, so it matches the `cid:` reference.
    assert_eq!(inline.content_id.as_deref(), Some("logo@x.io"));
    // An inline part has no file name and still needs a stable one to show.
    assert_eq!(inline.name, "image.png");
    assert_eq!(inline.content_type, "image/png");
    // Bytes are preserved exactly; the id does not replace them.
    assert_eq!(inline.bytes, b"\x89PNG\r\n\x1a\n");

    let attachments = mail_classifier::provider::mime::attachments(&parsed);
    assert_eq!(attachments[0].content_id.as_deref(), Some("logo@x.io"));
    assert_eq!(
        attachments[0].inline_data.as_deref(),
        Some(inline.bytes.as_slice())
    );
}

#[test]
fn an_ordinary_attachment_carries_no_content_id() {
    let parsed = parse_message(MULTIPART.as_bytes()).expect("valid MIME");
    assert_eq!(parsed.files.len(), 1);
    assert_eq!(parsed.files[0].name, "menu.pdf");
    // Only inline parts have one, so a rebuild must not start inventing ids.
    assert_eq!(parsed.files[0].content_id, None);
}

// ------------------------------------- how a failed send is classified.

use mail_classifier::provider::gmail::compose::{SendPhase, classify_send_failure};

#[test]
fn a_missing_draft_during_a_send_is_a_delivery_question_not_a_missing_object() {
    // Both moments a send can find the draft gone mean the same thing: it may
    // have just been sent from another client, to the same people.
    for phase in [SendPhase::Preflight, SendPhase::Dispatch] {
        let error = classify_send_failure(
            phase,
            ProviderError::Api {
                status: 404,
                message: "Requested entity was not found.".into(),
            },
        );
        assert!(
            error.needs_send_reconciliation(),
            "{phase:?} 404 must be reconciled with find_sent, not retried"
        );
        assert!(matches!(error, ComposeError::Uncertain { .. }));
        // Critically not NotFound: a caller that sees a missing draft clears
        // its attempt and may send again, and this compose would arrive twice.
        assert!(!matches!(error, ComposeError::NotFound(_)));
        assert!(!error.is_retryable());
    }
}

#[test]
fn a_rejected_send_is_still_a_plain_error() {
    // Nothing was sent; revoked credentials need reconnecting rather than a retry.
    let error = classify_send_failure(
        SendPhase::Dispatch,
        ProviderError::Auth("token revoked".into()),
    );
    assert!(!error.needs_send_reconciliation());
    assert!(!error.is_retryable());
}
