use std::collections::HashSet;

use mail_classifier::provider::RemoteState;
use mail_classifier::provider::gmail::convert::{label_ops, list_ids, message, state_from_labels};
use serde_json::{Value, json};

fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

fn set(list: &[&str]) -> HashSet<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn list_ids_reads_message_objects() {
    let page = json!({
        "messages": [{"id": "18a1", "threadId": "t1"}, {"id": "18a2", "threadId": "t1"}],
        "resultSizeEstimate": 2
    });
    assert_eq!(list_ids(&page), strings(&["18a1", "18a2"]));
    assert!(list_ids(&json!({"resultSizeEstimate": 0})).is_empty());
}

#[test]
fn label_ops_file_to_inbox_adds_inbox_and_drops_label() {
    let (add, remove) = label_ops(&RemoteState::Folder("L1".into()), &RemoteState::Inbox);
    assert_eq!(add, strings(&["INBOX"]));
    assert_eq!(remove, strings(&["L1"]));
}

#[test]
fn label_ops_archive_only_strips_inbox() {
    let (add, remove) = label_ops(&RemoteState::Inbox, &RemoteState::Archived);
    assert!(add.is_empty());
    assert_eq!(remove, strings(&["INBOX"]));
}

#[test]
fn label_ops_to_trash_adds_nothing() {
    let (add, remove) = label_ops(&RemoteState::Inbox, &RemoteState::Trash);
    assert!(add.is_empty());
    assert_eq!(remove, strings(&["INBOX"]));
}

#[test]
fn label_ops_into_folder_adds_folder_only() {
    let (add, remove) = label_ops(&RemoteState::Archived, &RemoteState::Folder("L9".into()));
    assert_eq!(add, strings(&["L9"]));
    assert_eq!(remove, strings(&["INBOX"]));
}

#[test]
fn state_from_labels_prefers_trash_then_inbox() {
    assert_eq!(
        state_from_labels(&strings(&["TRASH", "INBOX"]), &set(&[])),
        RemoteState::Trash
    );
    assert_eq!(
        state_from_labels(&strings(&["INBOX", "Label_3"]), &set(&["Label_3"])),
        RemoteState::Inbox
    );
}

#[test]
fn state_from_labels_picks_first_user_label_or_archived() {
    assert_eq!(
        state_from_labels(&strings(&["Label_7", "IMPORTANT"]), &set(&["Label_7"])),
        RemoteState::Folder("Label_7".into())
    );
    assert_eq!(
        state_from_labels(&strings(&["IMPORTANT", "UNREAD"]), &set(&["Label_7"])),
        RemoteState::Archived
    );
}

fn sample_message(extra_labels: &[&str]) -> Value {
    let mut labels = vec!["INBOX", "UNREAD", "Label_7"];
    labels.extend_from_slice(extra_labels);
    json!({
        "id": "18ab",
        "threadId": "18ab",
        "internalDate": "1790000000000",
        "labelIds": labels,
        "payload": {
            "mimeType": "multipart/mixed",
            "headers": [
                {"name": "Delivered-To", "value": "me@x.io"},
                {"name": "From", "value": "\"Ann Lee\" <ann@x.io>"},
                {"name": "To", "value": "me@x.io"},
                {"name": "cc", "value": "bob@x.io"},
                {"name": "Bcc", "value": "carol@x.io"},
                {"name": "Subject", "value": "Quarterly numbers"}
            ],
            "parts": [
                {
                    "mimeType": "multipart/alternative",
                    "headers": [{"name": "Content-Type", "value": "multipart/alternative"}],
                    "parts": [
                        {
                            "mimeType": "text/plain",
                            "headers": [{"name": "Content-Type", "value": "text/plain"}],
                            "body": {"size": 20, "data": "SGVsbG8sIHdvcmxkIQo="}
                        },
                        {
                            "mimeType": "text/html",
                            "headers": [{"name": "Content-Type", "value": "text/html"}],
                            "body": {"size": 25, "data": "PGI-SGVsbG88L2I-"}
                        }
                    ]
                },
                {
                    "mimeType": "application/pdf",
                    "filename": "q3.pdf",
                    "headers": [{"name": "Content-Disposition", "value": "attachment; filename=q3.pdf"}],
                    "body": {"attachmentId": "att-1", "size": 4096}
                }
            ]
        }
    })
}

#[test]
fn message_reads_multipart_headers_body_and_attachment() {
    let msg = message(&sample_message(&[]), &set(&["Label_7"])).expect("message");
    assert_eq!(msg.id, "18ab");
    assert_eq!(msg.thread, "18ab");
    assert_eq!(msg.from_name, "Ann Lee");
    assert_eq!(msg.from_email, "ann@x.io");
    assert_eq!(msg.to, "me@x.io");
    assert_eq!(msg.cc, "bob@x.io");
    assert_eq!(msg.bcc, "carol@x.io");
    assert_eq!(msg.subject, "Quarterly numbers");
    assert_eq!(msg.body, "Hello, world!\n");
    assert_eq!(msg.html.as_deref(), Some("<b>Hello</b>"));
    assert_eq!(msg.attachments.len(), 1);
    assert_eq!(msg.attachments[0].name, "q3.pdf");
    assert_eq!(msg.attachments[0].size, 4096);
    assert_eq!(msg.received, 1_790_000_000);
    assert!(!msg.outgoing);
    assert_eq!(msg.state, RemoteState::Inbox);
}

#[test]
fn message_without_user_label_reports_folder_state() {
    let raw = sample_message(&[]);
    let mut raw = raw;
    raw["labelIds"] = json!(["Label_7", "IMPORTANT"]);
    let msg = message(&raw, &set(&["Label_7"])).expect("message");
    assert_eq!(msg.state, RemoteState::Folder("Label_7".into()));
}

#[test]
fn message_sent_flag_follows_sent_label() {
    let mut raw = sample_message(&[]);
    raw["labelIds"] = json!(["SENT", "INBOX"]);
    let msg = message(&raw, &set(&[])).expect("message");
    assert!(msg.outgoing);
}

#[test]
fn message_rejects_spam_drafts_and_chats() {
    for filtered in ["SPAM", "DRAFT", "CHAT"] {
        let raw = sample_message(&[filtered]);
        assert!(
            message(&raw, &set(&["Label_7"])).is_none(),
            "{filtered} should be filtered"
        );
    }
}

#[test]
fn message_handles_bare_and_plain_display_names() {
    let cases = [
        ("ann@x.io", "ann@x.io", "ann@x.io"),
        ("Ann <ann@x.io>", "Ann", "ann@x.io"),
        ("\"Ann Lee\" <ann@x.io>", "Ann Lee", "ann@x.io"),
        ("Ann Lee <ann@x.io>", "Ann Lee", "ann@x.io"),
    ];
    for (header, name, email) in cases {
        let mut raw = sample_message(&[]);
        raw["payload"]["headers"][1] = json!({"name": "from", "value": header});
        let msg = message(&raw, &set(&["Label_7"])).expect("message");
        assert_eq!((msg.from_name.as_str(), msg.from_email.as_str()), (name, email));
    }
}

#[test]
fn message_tolerates_single_part_body_and_missing_headers() {
    let raw = json!({
        "id": "solo",
        "threadId": "t1",
        "internalDate": "1790000000000",
        "labelIds": ["INBOX"],
        "payload": {
            "headers": [{"name": "Subject", "value": "Solo"}],
            "body": {"size": 5, "data": "aGVsbG8"}
        }
    });
    let msg = message(&raw, &set(&[])).expect("message");
    assert_eq!(msg.body, "hello");
    assert_eq!(msg.html, None);
    assert!(msg.attachments.is_empty());
    assert_eq!(msg.from_name, "");
    assert_eq!(msg.thread, "t1");
}