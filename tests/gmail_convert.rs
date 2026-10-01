use std::collections::HashSet;
use std::time::{Duration, Instant};

use mail_classifier::provider::{Changes, RemoteFlags, RemoteState, Scope, Window};
use mail_classifier::provider::gmail::convert::{
    batch_body, is_rate_limited, label_ops, list_ids, list_query, merge_history, message,
    parse_batch, state_from_labels, body,
};
use mail_classifier::provider::gmail::quota::Quota;
use serde_json::{Value, json};

fn strings(list: &[&str]) -> Vec<String> { list.iter().map(|s| (*s).to_owned()).collect() }
fn set(list: &[&str]) -> HashSet<String> { list.iter().map(|s| (*s).to_owned()).collect() }

#[test]
fn list_ids_reads_message_objects() {
    let page = json!({"messages":[{"id":"18a1","threadId":"t1"},{"id":"18a2","threadId":"t1"}],"resultSizeEstimate":2});
    assert_eq!(list_ids(&page), strings(&["18a1", "18a2"]));
    assert!(list_ids(&json!({"resultSizeEstimate":0})).is_empty());
}

#[test]
fn quota_and_rate_errors_are_rate_limits_but_permission_errors_are_not() {
    let quota = json!({"error":{"code":403,"message":"Quota exceeded for quota metric 'Total Query Cost' and limit 'Units per minute per user'.","status":"PERMISSION_DENIED"}});
    assert!(is_rate_limited(403, &quota));
    assert!(is_rate_limited(429, &Value::Null));
    let reason = json!({"error":{"errors":[{"reason":"userRateLimitExceeded"}]}});
    assert!(is_rate_limited(403, &reason));
    let denied = json!({"error":{"message":"Request had insufficient authentication scopes.","status":"PERMISSION_DENIED"}});
    assert!(!is_rate_limited(403, &denied));
}

#[test]
fn quota_reserves_burst_refills_and_drains() {
    let start = Instant::now();
    let mut q = Quota::new(10, 25);
    assert_eq!(q.reserve(25, start), Duration::ZERO);
    assert_eq!(q.reserve(1, start), Duration::from_millis(100));
    assert_eq!(q.reserve(5, start), Duration::from_millis(600));
    assert_eq!(q.reserve(5, start + Duration::from_secs(1)), Duration::from_millis(100));
    q.drain(start + Duration::from_secs(1));
    assert_eq!(q.reserve(1, start + Duration::from_secs(1)), Duration::from_millis(100));
}

#[test]
fn label_operations_preserve_other_labels() {
    assert_eq!(label_ops(&RemoteState::Folder("L1".into()), &RemoteState::Inbox), (strings(&["INBOX"]), strings(&["L1"])));
    assert_eq!(label_ops(&RemoteState::Inbox, &RemoteState::Archived), (vec![], strings(&["INBOX"])));
    assert_eq!(label_ops(&RemoteState::Archived, &RemoteState::Folder("L9".into())), (strings(&["L9"]), strings(&["INBOX"])));
}

#[test]
fn states_follow_trash_inbox_then_first_user_label() {
    assert_eq!(state_from_labels(&strings(&["TRASH", "INBOX"]), &set(&[])), RemoteState::Trash);
    assert_eq!(state_from_labels(&strings(&["Label_7", "IMPORTANT"]), &set(&["Label_7"])), RemoteState::Folder("Label_7".into()));
    assert_eq!(state_from_labels(&strings(&["IMPORTANT", "UNREAD"]), &set(&[])), RemoteState::Archived);
}

fn sample_message(labels: &[&str]) -> Value {
    json!({"id":"18ab","threadId":"18ab","internalDate":"1790000000000","snippet":"Hello &amp; &#39; &#x27;",
        "labelIds":labels,"payload":{"headers":[
            {"name":"From","value":"\"Ann Lee\" <ann@x.io>"},{"name":"To","value":"me@x.io"},
            {"name":"Cc","value":"bob@x.io"},{"name":"Bcc","value":"carol@x.io"},{"name":"Subject","value":"Quarterly numbers"}
        ]}})
}

#[test]
fn metadata_message_parses_headers_flags_timestamp_and_entities() {
    let msg = message(&sample_message(&["INBOX", "UNREAD", "Label_7"]), &set(&["Label_7"])).expect("message");
    assert_eq!(msg.id, "18ab");
    assert_eq!(msg.thread, "18ab");
    assert_eq!(msg.from_name, "Ann Lee");
    assert_eq!(msg.from_email, "ann@x.io");
    assert_eq!(msg.to, "me@x.io");
    assert_eq!(msg.cc, "bob@x.io");
    assert_eq!(msg.bcc, "carol@x.io");
    assert_eq!(msg.subject, "Quarterly numbers");
    assert_eq!(msg.snippet, "Hello & ' '");
    assert_eq!(msg.received, 1_790_000_000);
    assert!(msg.unread);
    assert_eq!(msg.state, RemoteState::Inbox);
}

#[test]
fn body_conversion_decodes_multipart_content_and_attachments() {
    let raw = json!({"payload":{"mimeType":"multipart/mixed","parts":[
        {"mimeType":"multipart/alternative","parts":[
            {"mimeType":"text/plain","body":{"size":20,"data":"SGVsbG8sIHdvcmxkIQo="}},
            {"mimeType":"text/html","body":{"size":25,"data":"PGI-SGVsbG88L2I-"}}
        ]},
        {"mimeType":"application/pdf","filename":"q3.pdf","body":{"size":4096}}
    ]}});
    let parsed = body(&raw);
    assert_eq!(parsed.body, "Hello, world!\n");
    assert_eq!(parsed.html.as_deref(), Some("<b>Hello</b>"));
    assert_eq!(parsed.attachments[0].name, "q3.pdf");
    assert_eq!(parsed.attachments[0].size, 4096);
}

#[test]
fn message_filters_spam_drafts_and_chats() {
    for label in ["SPAM", "DRAFT", "CHAT"] { assert!(message(&sample_message(&[label]), &set(&[])).is_none()); }
}

#[test]
fn batch_body_has_multipart_http_parts() {
    let body = batch_body("boundary", &[("item-0".into(), "/gmail/v1/messages/a?format=metadata".into())]);
    assert!(body.starts_with("--boundary\r\nContent-Type: application/http\r\nContent-ID: <item-0>\r\n\r\nGET /gmail/v1/messages/a?format=metadata HTTP/1.1\r\n"));
    assert!(body.ends_with("--boundary--\r\n"));
}

#[test]
fn parse_batch_handles_crlf_and_out_of_order_parts() {
    let body = "--xyz\r\nContent-Type: application/http\r\nContent-ID: <response-item-1>\r\n\r\nHTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\n\r\n{\"error\":{}}\r\n--xyz\r\nContent-Type: application/http\r\nContent-ID: <response-item-0>\r\n\r\nHTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"id\":\"a\"}\r\n--xyz--\r\n";
    let parts = parse_batch("multipart/mixed; boundary=xyz", body);
    assert_eq!(parts.len(), 2);
    assert_eq!((parts[0].id.as_str(), parts[0].status), ("response-item-1", 404));
    assert_eq!((parts[1].id.as_str(), parts[1].status), ("response-item-0", 200));
    assert_eq!(parts[1].json["id"], "a");
}

#[test]
fn merge_history_last_flags_win_and_removed_wins() {
    let mut out = Changes::default();
    let folders = set(&["L1"]);
    merge_history(&json!({"historyId":"10","history":[
        {"messagesAdded":[{"message":{"id":"a","labelIds":["INBOX"]}}]},
        {"labelsAdded":[{"message":{"id":"a","labelIds":["UNREAD","L1"]}}]},
        {"labelsRemoved":[{"message":{"id":"a","labelIds":["INBOX"]}}]},
        {"messagesDeleted":[{"message":{"id":"a"}}]},
        {"messagesAdded":[{"message":{"id":"b","labelIds":["INBOX"]}}]},
        {"labelsAdded":[{"message":{"id":"b","labelIds":["SPAM"]}}]}
    ]}), &folders, &mut out);
    assert_eq!(out.cursor, "10");
    assert!(out.added.is_empty());
    assert!(out.updated.is_empty());
    assert_eq!(out.removed, strings(&["a", "b"]));

    let mut flags = Changes::default();
    merge_history(&json!({"history":[
        {"labelsAdded":[{"message":{"id":"c","labelIds":["INBOX"]}}]},
        {"labelsRemoved":[{"message":{"id":"c","labelIds":["UNREAD"]}}]}
    ]}), &folders, &mut flags);
    assert_eq!(flags.updated, vec![("c".into(), RemoteFlags { state: RemoteState::Archived, unread: true })]);
}

#[test]
fn list_query_encodes_every_scope_and_window() {
    let since = Window::Since(1_700_000_000);
    let before = Window::Before(1_700_000_000);
    assert!(list_query(&Scope::Inbox, since).q.ends_with("in:inbox after:1699999999"));
    assert!(list_query(&Scope::Archive, before).q.ends_with("-in:inbox -in:trash has:nouserlabels before:1700000000"));
    let trash = list_query(&Scope::Trash, since);
    assert!(trash.q.ends_with("in:trash after:1699999999"));
    assert!(trash.include_trash);
    let folder = list_query(&Scope::Folder("L7".into()), before);
    assert_eq!(folder.label.as_deref(), Some("L7"));
    assert!(folder.q.ends_with("before:1700000000"));
}
