use gpui_kit::TestAppContext;

#[path = "ui_ext/harness.rs"]
#[allow(dead_code)]
mod harness;
use harness::{harness_with, mailbox, msg};

#[gpui_kit::gpui::test]
fn unread_filed_message_remains_unread_in_folder_view(cx: &mut TestAppContext) {
    let message = msg(1, 1, "alice@example.com", "Q", 1, "Inbox")
        .replace(r#""state":"Inbox""#, r#""state":{"Filed":1}"#);
    let mut h = harness_with(cx, mailbox(&[message]));
    h.click("sidebar-folder-1");
    assert_eq!(h.read(|app| app.location()), Some(mail_classifier::model::Location::Folder(1)));
    assert!(h.read(|app| app.row_visual(1).unread));
}
