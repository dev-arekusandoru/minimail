//! Headless keystroke tests for the choice dialogs, the folder picker and the
//! reader's message menu and banners.

use std::rc::Rc;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, Entity, Focusable, Point, TestAppContext, WindowBounds,
    WindowOptions, base::Root, px, size,
};
use mail_classifier::app::MailApp;
use mail_classifier::app::actions::bind_keys;
use mail_classifier::clock::{Clock, FakeClock, Timestamp};
use mail_classifier::judge::{Answer, AnswerValue, QuestionKey, Suggestion};
use mail_classifier::model::{FolderId, Mailbox, MessageId, Tag, TriageState};

const NOON: Timestamp = 1_790_683_200;

struct Harness<'a> {
    cx: &'a mut TestAppContext,
    window: AnyWindowHandle,
    app: Entity<MailApp>,
    clock: Rc<FakeClock>,
}

fn harness_with(cx: &mut TestAppContext, mailbox: Mailbox) -> Harness<'_> {
    cx.update(|cx| {
        gpui_kit::init(cx);
        bind_keys(cx);
    });
    let clock = Rc::new(FakeClock::new(NOON));
    let c2: Rc<dyn Clock> = clock.clone();
    let (window, app) = cx.update(|cx| {
        let (window, content) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(1200.), px(800.)),
                })),
                ..Default::default()
            },
            cx,
            |window, cx| {
                let view = cx.new(|cx| MailApp::new_with_clock(mailbox, c2, window, cx));
                window.focus(&view.focus_handle(cx), cx);
                view
            },
        )
        .expect("open window");
        (window.downcast::<Root>().expect("root").into(), content)
    });
    let mut h = Harness {
        cx,
        window,
        app,
        clock,
    };
    h.keys("");
    h
}

/// One Inbox message from `email` (state defaults to Inbox).
fn msg(id: u32, thread: u32, email: &str, subject: &str, day: u32) -> String {
    let name = email.split('@').next().unwrap();
    format!(
        r#"{{"id":{id},"thread_id":{thread},"from_name":"{name}","from_email":"{email}","to":"you@example.com","subject":"{subject}","body":"Just checking in about this.","received":"2026-09-{day:02}T09:00:00Z"}}"#
    )
}

fn mailbox(msgs: &[String]) -> Mailbox {
    Mailbox::from_json(&format!("[{}]", msgs.join(","))).expect("valid mailbox json")
}

/// Tag `id` PossibleSpam through the classifier's own path.
fn tag_possible_spam(mb: &mut Mailbox, id: MessageId) {
    mb.add_suggestions(vec![Suggestion {
        message: id,
        key: QuestionKey::Spam,
        answer: Answer {
            probabilities: vec![0.05, 0.95],
            value: AnswerValue::Bool(true),
            confidence: 0.95,
        },
    }]);
    mb.accept_suggestions(id, NOON);
}

impl Harness<'_> {
    fn keys(&mut self, keys: &str) {
        if !keys.is_empty() {
            self.cx.simulate_keystrokes(self.window, keys);
        }
        self.cx.run_until_parked();
    }

    fn type_text(&mut self, text: &str) {
        let window = self.window;
        self.cx
            .update_window(window, |_, window, cx| window.input(text, cx))
            .unwrap();
        self.cx.run_until_parked();
    }

    fn click(&mut self, id: impl Into<gpui_kit::ElementId>) {
        let id = id.into();
        self.cx
            .update_window(self.window, |_, window, cx| window.click(id, cx))
            .unwrap();
        self.cx.run_until_parked();
    }

    fn read<R>(&mut self, f: impl FnOnce(&MailApp) -> R) -> R {
        self.cx.read_entity(&self.app, |a, _| f(a))
    }

    fn dialog_open(&mut self) -> bool {
        self.read(|a| a.dialog_open())
    }

    fn dialog_options(&mut self) -> Vec<String> {
        self.app.read_with(self.cx, |a, cx| a.dialog_options(cx))
    }

    fn dialog_title(&mut self) -> String {
        self.app
            .read_with(self.cx, |a, cx| a.dialog_title(cx))
            .unwrap_or_default()
    }

    fn folder_rows(&mut self) -> Vec<String> {
        self.app.read_with(self.cx, |a, cx| a.folder_rows(cx))
    }

    fn state_of(&mut self, id: MessageId) -> TriageState {
        self.read(|a| a.mailbox.state_of(id).unwrap())
    }

    fn count(&mut self, state: TriageState) -> usize {
        self.read(|a| a.mailbox.count(state))
    }

    fn cursor(&mut self) -> Option<MessageId> {
        self.read(|a| a.triage.cursor(&a.mailbox))
    }

    /// Move the cursor with `j` until it reaches `id`.
    fn goto(&mut self, id: MessageId) {
        for _ in 0..200 {
            if self.cursor() == Some(id) {
                return;
            }
            self.keys("j");
        }
        panic!("could not reach message {id}");
    }

    fn blocked(&mut self) -> Vec<String> {
        self.read(|a| a.mailbox.blocked())
    }

    fn is_new_sender(&mut self, id: MessageId) -> bool {
        self.read(|a| a.mailbox.is_new_sender(id))
    }

    fn has_tag(&mut self, id: MessageId, tag: Tag) -> bool {
        self.read(|a| a.mailbox.tags(id).contains(&tag))
    }

    fn folder(&mut self, id: FolderId) -> Option<String> {
        self.read(|a| a.mailbox.folder(id).map(|f| f.name.clone()))
    }

    fn folder_count(&mut self) -> usize {
        self.read(|a| a.mailbox.folders("personal").len())
    }

    /// The outgoing message materialised by the outbox flush.
    fn outgoing_ids(&mut self) -> Vec<MessageId> {
        self.read(|a| {
            a.mailbox
                .messages()
                .iter()
                .filter(|m| m.outgoing)
                .map(|m| m.id)
                .collect()
        })
    }

    fn advance(&mut self, secs: Timestamp) {
        self.clock.advance(secs);
        self.app.update(self.cx, |a, cx| a.tick(cx));
        self.cx.run_until_parked();
    }

    fn send_reply(&mut self, body: &str) {
        self.keys("r");
        self.type_text(body);
        self.keys("cmd-enter");
    }
}

#[gpui_kit::gpui::test]
fn post_send_dialog_files_original_and_reply_in_one_undo(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox(&[msg(1, 1, "alice@example.com", "Lunch?", 1)]));
    // First reply: keep it and let the outbox send it, so the message exists.
    h.send_reply("First");
    h.keys("4");
    h.advance(11);
    let first = h.outgoing_ids();
    assert_eq!(first.len(), 1, "the first reply was sent");
    assert_eq!(h.state_of(first[0]), TriageState::Inbox);

    // Second reply: the dialog files the original and the sent reply in one step.
    h.send_reply("Second, let me know when");
    assert!(h.dialog_open(), "the post-send dialog opens");
    assert_eq!(
        h.dialog_options(),
        vec!["Archive", "File…", "Delete", "Keep in Inbox"]
    );
    assert_eq!(h.state_of(1), TriageState::Inbox, "nothing filed yet");

    h.keys("1");
    assert!(!h.dialog_open());
    assert_eq!(h.state_of(1), TriageState::Archived);
    assert_eq!(h.state_of(first[0]), TriageState::Archived, "the sent reply follows");

    h.advance(11);
    h.keys("u");
    assert_eq!(h.state_of(1), TriageState::Inbox);
    assert_eq!(h.state_of(first[0]), TriageState::Inbox, "one undo reverts the filing");
}

#[gpui_kit::gpui::test]
fn post_send_dialog_files_the_pending_reply_when_it_sends(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox(&[msg(1, 1, "alice@example.com", "Lunch?", 1)]));
    h.send_reply("Sure, let me know when");
    h.keys("1");
    assert_eq!(h.state_of(1), TriageState::Archived);

    // The outbox flush materialises the reply; it mirrors the original's state.
    h.advance(11);
    let outgoing = h.outgoing_ids();
    assert_eq!(outgoing.len(), 1, "the reply was sent");
    assert_eq!(h.state_of(outgoing[0]), TriageState::Archived);
}

#[gpui_kit::gpui::test]
fn post_send_dialog_keep_leaves_everything(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox(&[msg(1, 1, "alice@example.com", "Lunch?", 1)]));
    h.send_reply("Sure");
    h.keys("4");
    assert!(!h.dialog_open());
    assert_eq!(h.state_of(1), TriageState::Inbox);
}

#[gpui_kit::gpui::test]
fn post_send_dialog_files_into_a_chosen_folder(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox(&[msg(1, 1, "alice@example.com", "Lunch?", 1)]));
    h.send_reply("Sure");
    h.keys("2");
    assert!(!h.dialog_open());
    assert_eq!(h.folder_rows(), vec!["Receipts", "Travel", "Family"]);
    h.keys("down");
    h.keys("enter");
    assert_eq!(h.state_of(1), TriageState::Filed(2), "filed into Travel");
}

#[gpui_kit::gpui::test]
fn block_dialog_moves_sender_mail_and_undo_restores(cx: &mut TestAppContext) {
    let mut h = harness_with(
        cx,
        mailbox(&[
            msg(1, 1, "spam@example.com", "Offer", 1),
            msg(2, 2, "spam@example.com", "More", 2),
            msg(3, 3, "friend@example.com", "Hi", 3),
        ]),
    );
    h.goto(1);
    h.keys("b");
    assert!(h.dialog_open());
    assert!(h.dialog_title().starts_with("Block spam"), "{}", h.dialog_title());
    assert_eq!(
        h.dialog_options(),
        vec!["Delete", "Archive", "File…", "Leave"]
    );

    h.keys("1");
    assert_eq!(h.blocked(), vec!["spam@example.com"]);
    assert_eq!(h.state_of(1), TriageState::Deleted);
    assert_eq!(h.state_of(2), TriageState::Deleted);
    assert_eq!(h.state_of(3), TriageState::Inbox, "other senders untouched");

    h.keys("u");
    assert!(h.blocked().is_empty(), "undo unblocks");
    assert_eq!(h.state_of(1), TriageState::Inbox);
    assert_eq!(h.state_of(2), TriageState::Inbox);
}

#[gpui_kit::gpui::test]
fn block_dialog_leave_keeps_mail_and_escape_changes_nothing(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox(&[msg(1, 1, "spam@example.com", "Offer", 1)]));

    h.keys("b");
    h.keys("escape");
    assert!(!h.dialog_open());
    assert!(h.blocked().is_empty(), "escape changes nothing");

    h.keys("b");
    h.keys("4");
    assert_eq!(h.blocked(), vec!["spam@example.com"]);
    assert_eq!(h.state_of(1), TriageState::Inbox, "Leave keeps the mail");
}

#[gpui_kit::gpui::test]
fn block_dialog_files_sender_mail(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox(&[msg(1, 1, "spam@example.com", "Offer", 1)]));
    h.keys("b");
    h.keys("3");
    assert!(!h.dialog_open());
    assert!(h.folder_rows().contains(&"Receipts".to_owned()));
    h.keys("enter");
    assert_eq!(h.state_of(1), TriageState::Filed(1));
    assert_eq!(h.blocked(), vec!["spam@example.com"]);
}

#[gpui_kit::gpui::test]
fn unsubscribe_dialog_moves_mail(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox(&[msg(1, 1, "news@example.com", "Digest", 1)]));
    h.keys("shift-u");
    assert!(h.dialog_title().starts_with("Unsubscribe from news"));
    h.keys("2");
    assert_eq!(h.state_of(1), TriageState::Archived);
    assert_eq!(
        h.read(|a| a.mailbox.unsubscribed().to_vec()),
        vec!["news@example.com"]
    );
}

#[gpui_kit::gpui::test]
fn spam_dialog_blocks_and_deletes_or_just_deletes(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox(&[msg(1, 1, "spam@example.com", "Offer", 1)]));
    h.keys("!");
    assert!(h.dialog_open());
    assert_eq!(h.dialog_options(), vec!["Block & Delete", "Delete"]);
    h.keys("1");
    assert_eq!(h.state_of(1), TriageState::Deleted);
    assert_eq!(h.blocked(), vec!["spam@example.com"]);

    let mut h = harness_with(cx, mailbox(&[msg(1, 1, "spam@example.com", "Offer", 1)]));
    h.keys("!");
    h.keys("2");
    assert_eq!(h.state_of(1), TriageState::Deleted);
    assert!(h.blocked().is_empty(), "plain Delete does not block");
}

#[gpui_kit::gpui::test]
fn sender_wide_confirm_cancel_changes_nothing(cx: &mut TestAppContext) {
    let mut h = harness_with(
        cx,
        mailbox(&[
            msg(1, 1, "alice@example.com", "One", 1),
            msg(2, 2, "alice@example.com", "Two", 2),
        ]),
    );
    h.keys("shift-e");
    assert!(h.dialog_open());
    assert_eq!(h.dialog_title(), "Archive all 2 messages from alice in Inbox?");
    h.keys("2");
    assert!(!h.dialog_open());
    assert_eq!(h.count(TriageState::Archived), 0, "cancel archives nothing");
    assert_eq!(h.count(TriageState::Inbox), 2);

    h.keys("escape");
    h.keys("shift-e");
    h.keys("1");
    assert_eq!(h.count(TriageState::Archived), 2, "confirm archives both");
}

#[gpui_kit::gpui::test]
fn sender_wide_delete_and_inbox_confirm(cx: &mut TestAppContext) {
    let mut h = harness_with(
        cx,
        mailbox(&[
            msg(1, 1, "alice@example.com", "One", 1),
            msg(2, 2, "alice@example.com", "Two", 2),
        ]),
    );
    h.keys("shift-d");
    assert_eq!(h.dialog_title(), "Delete all 2 messages from alice in Inbox?");
    h.keys("1");
    assert_eq!(h.count(TriageState::Deleted), 2);

    // From Trash, the sender-wide move brings the sender's trashed mail back.
    h.keys("g d");
    h.keys("shift-i");
    assert_eq!(h.dialog_title(), "Move to Inbox all 2 messages from alice in Trash?");
    h.keys("1");
    assert_eq!(h.count(TriageState::Inbox), 2);
    assert_eq!(h.count(TriageState::Deleted), 0);
    h.keys("u");
    assert_eq!(h.count(TriageState::Deleted), 2, "one undo step restores both");
}

#[gpui_kit::gpui::test]
fn sender_wide_file_and_snooze_confirm(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox(&[msg(1, 1, "alice@example.com", "One", 1)]));
    h.keys("shift-f");
    assert_eq!(h.dialog_title(), "File all 1 message from alice in Inbox?");
    h.keys("1");
    assert!(h.folder_rows().contains(&"Receipts".to_owned()));
    h.keys("enter");
    assert_eq!(h.state_of(1), TriageState::Filed(1));

    let mut h = harness_with(cx, mailbox(&[msg(1, 1, "alice@example.com", "One", 1)]));
    h.keys("shift-s");
    assert_eq!(h.dialog_title(), "Snooze all 1 message from alice in Inbox?");
    h.keys("1");
    assert!(h.read(|a| a.snooze_open()), "confirm opens the snooze picker");
    h.keys("1");
    assert_eq!(h.state_of(1), TriageState::Snoozed);
}

#[gpui_kit::gpui::test]
fn folder_picker_files_into_an_existing_folder(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox(&[msg(1, 1, "alice@example.com", "One", 1)]));
    h.keys("f");
    assert!(h.read(|a| a.folder_picker_open()));
    assert_eq!(h.folder_rows(), vec!["Receipts", "Travel", "Family"]);

    h.keys("enter");
    assert!(!h.read(|a| a.folder_picker_open()));
    assert_eq!(h.state_of(1), TriageState::Filed(1));

    h.keys("u");
    assert_eq!(h.state_of(1), TriageState::Inbox);
}

#[gpui_kit::gpui::test]
fn folder_picker_creates_a_folder_in_one_undo(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox(&[msg(1, 1, "alice@example.com", "One", 1)]));
    let before = h.folder_count();
    h.keys("f");
    h.type_text("Zebra");
    assert_eq!(h.folder_rows(), vec!["Create “Zebra”"]);

    h.keys("enter");
    assert_eq!(h.folder_count(), before + 1);
    let state = h.state_of(1);
    let TriageState::Filed(folder) = state else {
        panic!("expected Filed, got {state:?}");
    };
    assert_eq!(h.folder(folder).as_deref(), Some("Zebra"));

    // Folder creation and the move revert together.
    h.keys("u");
    assert_eq!(h.state_of(1), TriageState::Inbox);
    assert_eq!(h.folder_count(), before);
    assert!(h.folder(folder).is_none());
}

#[gpui_kit::gpui::test]
fn folder_picker_filters_by_typed_name(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox(&[msg(1, 1, "alice@example.com", "One", 1)]));
    h.keys("f");
    h.type_text("fam");
    assert_eq!(h.folder_rows(), vec!["Family", "Create “fam”"]);
    h.keys("enter");
    assert_eq!(h.state_of(1), TriageState::Filed(3));
}

#[gpui_kit::gpui::test]
fn hash_deletes_and_the_message_menu_archives_and_files(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox(&[msg(1, 1, "alice@example.com", "One", 1)]));
    h.keys("#");
    assert_eq!(h.state_of(1), TriageState::Deleted);
    h.keys("u");
    assert_eq!(h.state_of(1), TriageState::Inbox);

    h.keys("enter");
    h.click(("btn-message-more", 1usize));
    h.click("btn-archive");
    assert_eq!(h.state_of(1), TriageState::Archived);

    h.keys("u");
    h.keys("enter");
    h.click(("btn-message-more", 1usize));
    h.click("btn-file");
    assert!(h.read(|a| a.folder_picker_open()));
    h.keys("enter");
    assert_eq!(h.state_of(1), TriageState::Filed(1));
}

#[gpui_kit::gpui::test]
fn new_sender_banner_allows_and_blocks(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox(&[msg(1, 1, "stranger@example.com", "Hello", 1)]));
    h.keys("enter");
    assert!(h.is_new_sender(1), "the fixture sender is unknown");

    h.click("btn-banner-allow");
    assert!(!h.is_new_sender(1), "Allow removes New Sender");
    assert_eq!(h.state_of(1), TriageState::Inbox);

    let mut h = harness_with(cx, mailbox(&[msg(1, 1, "stranger@example.com", "Hello", 1)]));
    h.keys("enter");
    h.click("btn-banner-block");
    assert!(h.dialog_open(), "Block opens the block dialog");
    h.keys("4");
    assert_eq!(h.blocked(), vec!["stranger@example.com"]);
}

#[gpui_kit::gpui::test]
fn spam_banner_blocks_and_deletes_or_just_deletes(cx: &mut TestAppContext) {
    let mut mb = mailbox(&[msg(1, 1, "spam@example.com", "Offer", 1)]);
    tag_possible_spam(&mut mb, 1);
    let mut h = harness_with(cx, mb);
    h.keys("enter");
    assert!(h.has_tag(1, Tag::PossibleSpam));

    h.click("btn-banner-spam-block");
    assert_eq!(h.state_of(1), TriageState::Deleted);
    assert_eq!(h.blocked(), vec!["spam@example.com"]);

    let mut mb = mailbox(&[msg(1, 1, "spam@example.com", "Offer", 1)]);
    tag_possible_spam(&mut mb, 1);
    let mut h = harness_with(cx, mb);
    h.keys("enter");
    h.click("btn-banner-spam-delete");
    assert_eq!(h.state_of(1), TriageState::Deleted);
    assert!(h.blocked().is_empty());
}

#[gpui_kit::gpui::test]
fn undo_after_the_flush_reverts_the_filed_reply_too(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox(&[msg(1, 1, "alice@example.com", "Lunch?", 1)]));
    h.send_reply("Sure, let me know when");
    h.keys("1");
    assert_eq!(h.state_of(1), TriageState::Archived);

    h.advance(11);
    let reply = h.outgoing_ids()[0];
    assert_eq!(h.state_of(reply), TriageState::Archived, "the reply materialises filed");

    h.keys("u");
    assert_eq!(h.state_of(1), TriageState::Inbox);
    assert_eq!(h.state_of(reply), TriageState::Inbox, "the later-sent reply reverts too");
}

#[gpui_kit::gpui::test]
fn folder_creation_in_the_sender_wide_and_block_flows_is_one_undo(cx: &mut TestAppContext) {
    // Sender-wide file into a folder that does not exist yet.
    let mut h = harness_with(cx, mailbox(&[msg(1, 1, "alice@example.com", "One", 1)]));
    let before = h.folder_count();
    h.keys("shift-f 1");
    h.type_text("Zebra");
    h.keys("enter");
    let TriageState::Filed(folder) = h.state_of(1) else {
        panic!("expected Filed");
    };
    assert_eq!(h.folder(folder).as_deref(), Some("Zebra"));
    h.keys("u");
    assert_eq!(h.state_of(1), TriageState::Inbox);
    assert!(h.folder(folder).is_none(), "the folder and the move revert together");
    assert_eq!(h.folder_count(), before);

    // Block and file into a folder that does not exist yet.
    let mut h = harness_with(cx, mailbox(&[msg(1, 1, "spam@example.com", "Offer", 1)]));
    let before = h.folder_count();
    h.keys("b 3");
    h.type_text("Junk");
    h.keys("enter");
    assert_eq!(h.blocked(), vec!["spam@example.com"]);
    let TriageState::Filed(folder) = h.state_of(1) else {
        panic!("expected Filed");
    };
    assert_eq!(h.folder(folder).as_deref(), Some("Junk"));
    h.keys("u");
    assert_eq!(h.state_of(1), TriageState::Inbox);
    assert!(h.blocked().is_empty(), "the block reverts too");
    assert!(h.folder(folder).is_none());
    assert_eq!(h.folder_count(), before);
}

#[gpui_kit::gpui::test]
fn allow_key_works_outside_the_new_senders_view(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, mailbox(&[msg(1, 1, "stranger@example.com", "Hello", 1)]));
    assert_eq!(h.cursor(), Some(1));
    h.keys("a");
    assert!(!h.is_new_sender(1));
}
