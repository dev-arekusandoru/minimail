//! Sidebar navigation: the accounts/folders sidebar, the chip row, the Filter ▾ menu
//! and the `g`-prefix jumps.

use std::rc::Rc;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, ElementId, Entity, Focusable, Point, TestAppContext,
    WindowBounds, WindowOptions, base::Root, px, size,
};
use mail_classifier::app::MailApp;
use mail_classifier::app::actions::bind_keys;
use mail_classifier::clock::{Clock, FakeClock, Timestamp};
use mail_classifier::contacts::{ContactStore, NewContact};
use mail_classifier::model::{
    Chip, Filter, Location, Mailbox, MessageId, OUTBOX_DELAY, Tag, TriageState,
};

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
    let ticker: Rc<dyn Clock> = clock.clone();
    let (window, app) = cx.update(|cx| {
        let (window, content) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(1400.), px(900.)),
                })),
                ..Default::default()
            },
            cx,
            |window, cx| {
                let view = cx.new(|cx| MailApp::new_with_clock(mailbox, ticker, window, cx));
                window.focus(&view.focus_handle(cx), cx);
                view
            },
        )
        .expect("open window");
        (window.downcast::<Root>().expect("root").into(), content)
    });
    let mut h = Harness { cx, window, app, clock };
    h.keys("");
    h
}

fn harness(cx: &mut TestAppContext) -> Harness<'_> {
    harness_with(cx, nav_box())
}

/// One JSON message belonging to `account`.
fn msg(id: u32, account: &str, email: &str, subject: &str, day: u32, state: &str) -> String {
    format!(
        r#"{{"id":{id},"thread_id":{id},"from_name":"{name}","from_email":"{email}","to":"you@example.com","subject":"{subject}","body":"Just checking in about this.","received":"2026-09-{day:02}T09:00:00Z","state":"{state}","account":"{account}","snooze":null}}"#,
        name = email.split('@').next().unwrap()
    )
}

fn json(msgs: &[String]) -> String {
    format!("[{}]", msgs.join(","))
}

/// Two accounts, one message each in Inbox, Archive, Trash and Sent-worthy Inbox mail.
fn nav_box() -> Mailbox {
    Mailbox::from_json(&json(&[
        msg(1, "personal", "ann@x.io", "Personal inbox", 4, "Inbox"),
        msg(2, "work", "bob@acme.co", "Work inbox", 3, "Inbox"),
        msg(3, "personal", "ann@x.io", "Personal archive", 2, "Archived"),
        msg(4, "work", "bob@acme.co", "Work archive", 1, "Archived"),
    ]))
    .expect("valid mailbox json")
}

/// The same shape, but everyone from `known@a.io` is in the address book.
fn new_sender_box() -> Mailbox {
    let store = ContactStore::open_in_memory().unwrap();
    store.create(NewContact::from_email("known@a.io", "Known")).unwrap();
    Mailbox::from_json_with_contacts(
        &json(&[
            msg(1, "personal", "known@a.io", "Known one", 4, "Inbox"),
            msg(2, "personal", "known@a.io", "Known two", 3, "Inbox"),
            msg(3, "personal", "stranger@b.io", "From a stranger", 2, "Inbox"),
        ]),
        Rc::new(store),
    )
    .expect("valid mailbox json")
}

impl Harness<'_> {
    fn keys(&mut self, keys: &str) {
        if !keys.is_empty() {
            self.cx.simulate_keystrokes(self.window, keys);
        }
        self.cx.run_until_parked();
    }
    fn click(&mut self, id: impl Into<ElementId>) {
        let id = id.into();
        self.cx.update_window(self.window, |_, window, cx| window.click(id, cx)).expect("window alive");
        self.cx.run_until_parked();
    }
    fn type_text(&mut self, text: &str) {
        let window = self.window;
        self.cx.update_window(window, |_, window, cx| window.input(text, cx)).unwrap();
        self.cx.run_until_parked();
    }
    fn read<R>(&mut self, f: impl FnOnce(&MailApp) -> R) -> R {
        self.cx.read_entity(&self.app, |a, _| f(a))
    }
    fn location(&mut self) -> Location {
        self.read(|a| a.triage.view.location.clone())
    }
    fn chip(&mut self) -> Chip {
        self.read(|a| a.triage.view.chip)
    }
    fn filter(&mut self) -> Filter {
        self.read(|a| a.triage.view.filter.clone())
    }
    fn visible(&mut self) -> Vec<MessageId> {
        self.read(|a| a.visible_ids())
    }
    /// Whether an element is in the tree of the last completed frame.
    fn has(&mut self, id: impl Into<ElementId>) -> bool {
        let id = id.into();
        self.cx
            .update_window(self.window, |_, window, _| window.try_find(id).is_some())
            .unwrap_or(false)
    }
    fn chip_row(&mut self) -> bool {
        self.has("chip-row")
    }
    fn advance(&mut self, secs: Timestamp) {
        self.clock.advance(secs);
        self.app.update(self.cx, |a, cx| a.tick(cx));
        self.cx.run_until_parked();
    }
    fn outgoing(&mut self) -> Vec<MessageId> {
        self.read(|a| {
            a.mailbox
                .messages()
                .iter()
                .filter(|m| m.outgoing)
                .map(|m| m.id)
                .collect()
        })
    }
    fn state_of(&mut self, id: MessageId) -> mail_classifier::model::TriageState {
        self.read(|a| a.mailbox.state_of(id).unwrap())
    }
}

#[gpui_kit::gpui::test]
fn sidebar_click_switches_location_and_resets_chip_and_filter(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    assert_eq!(h.location(), Location::AllInboxes);

    // A chip and a filter are both set before the click.
    h.keys("2");
    assert_eq!(h.chip(), Chip::NeedsReply);
    h.click("btn-filter");
    h.click("filter-tag-reminder");
    assert_eq!(h.filter().tags.len(), 1);

    h.click("nav-archive-work");
    assert_eq!(h.location(), Location::Archive("work".into()));
    assert_eq!(h.chip(), Chip::All, "clicking a location resets the chip");
    assert_eq!(h.filter(), Filter::default(), "clicking a location clears filters");
    assert_eq!(h.visible(), vec![4], "only work's archived message");
    assert_eq!(h.state_of(4), TriageState::Archived);
    assert_eq!(h.read(|a| a.location_label()), "Work · Archive");

    h.click("nav-inbox-personal");
    assert_eq!(h.location(), Location::Inbox("personal".into()));
    assert_eq!(h.visible(), vec![1]);
    assert_eq!(h.read(|a| a.location_label()), "Personal · Inbox");

    h.click("nav-sent-personal");
    assert_eq!(h.location(), Location::Sent("personal".into()));
    assert!(h.visible().is_empty(), "no replies sent yet");

    h.click("nav-folder-4");
    assert_eq!(h.location(), Location::Folder(4), "a folder of the work account");
    assert_eq!(h.read(|a| a.location_label()), "Projects");
    h.click("nav-folder-5");
    assert_eq!(h.read(|a| a.location_label()), "Projects/Northwind");

    h.click("nav-all-inboxes");
    assert_eq!(h.read(|a| a.location_label()), "All Inboxes");
}

#[gpui_kit::gpui::test]
fn folder_rows_fold_their_children(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    assert!(h.has("nav-folder-5"), "Northwind nests under Projects");
    h.click(("folder-caret", 4usize));
    assert!(!h.has("nav-folder-5"), "folding Projects hides Northwind");
    assert!(h.has("nav-folder-4"), "the folder itself stays");
    assert_eq!(h.location(), Location::AllInboxes, "folding does not navigate");
    h.click(("folder-caret", 4usize));
    assert!(h.has("nav-folder-5"), "unfolding brings the child back");
}

#[gpui_kit::gpui::test]
fn account_headers_fold_their_section(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    assert!(h.has("nav-inbox-work") && h.has("nav-folder-4"));
    assert!(!h.has("account-count-work"), "unfolded headers show no count");
    h.click("account-work");
    assert!(!h.has("nav-inbox-work"), "folding hides the account's rows");
    assert!(!h.has("nav-folder-4"), "including its folders");
    assert!(h.has("nav-inbox-personal"), "other accounts are untouched");
    assert!(h.has("account-count-work"), "the work Inbox holds unread mail");
    assert_eq!(h.location(), Location::AllInboxes, "folding does not navigate");
    h.click("account-work");
    assert!(h.has("nav-inbox-work") && h.has("nav-folder-4"), "unfolding restores the rows");
    assert!(!h.has("account-count-work"));
}

#[gpui_kit::gpui::test]
fn chips_exist_on_inbox_views_only(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    assert!(h.chip_row(), "All Inboxes is an Inbox view");
    assert!(h.chip_row() && h.has(("chip", 0usize)));

    h.click("nav-archive-personal");
    assert!(!h.chip_row(), "no chips on a non-Inbox location");
    let all = h.visible();
    h.keys("2");
    assert_eq!(h.chip(), Chip::All, "chip keys are ignored outside Inbox views");
    assert_eq!(h.visible(), all);

    h.click("nav-inbox-work");
    assert!(h.chip_row(), "an account's Inbox has chips");
    h.keys("2");
    assert_eq!(h.chip(), Chip::NeedsReply);
    assert_eq!(h.location(), Location::Inbox("work".into()));
}

#[gpui_kit::gpui::test]
fn chip_keys_select_each_chip_and_filter_the_list(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, new_sender_box());
    let all = h.visible();
    assert_eq!(all.len(), 3);

    for (i, expected) in Chip::ALL.into_iter().enumerate() {
        h.keys(&(i + 1).to_string());
        assert_eq!(h.chip(), expected, "key {} selects {}", i + 1, expected.label());
    }

    h.keys("5");
    assert_eq!(h.visible(), vec![3], "the New Senders chip keeps the stranger's mail");
    h.keys("1");
    assert_eq!(h.chip(), Chip::All);
    assert_eq!(h.visible(), all, "the All chip drops the filter");

    // Clicking a chip does exactly what its key does.
    h.click(("chip", 4usize));
    assert_eq!(h.chip(), Chip::NewSenders);
    assert_eq!(h.visible(), vec![3]);
}

#[gpui_kit::gpui::test]
fn filter_menu_narrows_the_list_by_account_and_tag(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    assert_eq!(h.visible(), vec![1, 2], "both inboxes, newest first");

    h.click("btn-filter");
    assert!(h.read(|a| a.menu_open()));
    h.click("filter-account");
    h.click("filter-account-work");
    assert_eq!(h.filter().account.as_deref(), Some("work"));
    assert_eq!(h.visible(), vec![2]);
    assert!(h.read(|a| a.filter_label()).contains("(1)"), "the button shows the filter count");

    // Clear, then filter by tag: a snoozed message that woke up carries Reminder.
    h.click("btn-filter");
    h.click("filter-clear");
    assert_eq!(h.filter(), Filter::default());
    assert_eq!(h.visible(), vec![1, 2]);

    let id = *h.visible().last().unwrap();
    h.app.update(h.cx, |a, _| a.mailbox.snooze(&[id], NOON + 60, NOON));
    h.advance(120);
    assert!(h.read(|a| a.mailbox.tags(id).contains(&Tag::Reminder)));

    h.click("btn-filter");
    h.click("filter-tag-reminder");
    assert_eq!(h.visible(), vec![id], "only the message tagged Reminder");
    assert!(h.read(|a| a.filter_label()).contains("(1)"));

    h.click("btn-filter");
    h.click("filter-clear");
    assert_eq!(h.filter(), Filter::default());
    assert_eq!(h.visible().len(), 2);

    // The account submenu belongs to All Inboxes only.
    h.click("btn-filter");
    assert!(h.has("filter-account"), "All Inboxes can filter by account");
    h.keys("escape");
    assert!(!h.read(|a| a.menu_open()));
    h.click("nav-inbox-personal");
    h.click("btn-filter");
    assert!(!h.has("filter-account"), "an account's Inbox need not filter by account");
    h.keys("escape");
}

#[gpui_kit::gpui::test]
fn g_prefix_jumps_to_the_current_accounts_locations(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.keys("g s");
    assert_eq!(h.location(), Location::Snoozed("personal".into()));
    h.keys("g a");
    assert_eq!(h.location(), Location::Archive("personal".into()));
    h.keys("g d");
    assert_eq!(h.location(), Location::Trash("personal".into()));
    h.keys("g t");
    assert_eq!(h.location(), Location::Sent("personal".into()));
    h.keys("g i");
    assert_eq!(h.location(), Location::Inbox("personal".into()));

    // From All Inboxes the first account answers, and `g i` keeps it unified.
    h.click("nav-all-inboxes");
    assert_eq!(h.location(), Location::AllInboxes);
    h.keys("g a");
    assert_eq!(h.location(), Location::Archive("personal".into()));
    h.click("nav-all-inboxes");
    h.keys("g i");
    assert_eq!(h.location(), Location::AllInboxes, "`g i` stays on All Inboxes");

    // From an account's location the jumps stay in that account.
    h.click("nav-inbox-work");
    h.keys("g a");
    assert_eq!(h.location(), Location::Archive("work".into()));
    h.keys("g i");
    assert_eq!(h.location(), Location::Inbox("work".into()));
}

#[gpui_kit::gpui::test]
fn sent_shows_the_outgoing_message_after_a_flushed_reply(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.click("nav-inbox-personal");
    assert!(h.outgoing().is_empty());

    h.keys("r");
    assert!(h.read(|a| a.compose_open()));
    h.type_text("Sure, let me know.");
    h.keys("cmd-enter");
    assert!(!h.read(|a| a.compose_open()), "sending closes the composer");
    h.keys("escape");
    assert!(h.outgoing().is_empty(), "the reply waits in the outbox");

    h.advance(OUTBOX_DELAY + 1);
    let sent = h.outgoing();
    assert_eq!(sent.len(), 1, "tick flushes the outbox into a sent message");

    h.keys("g t");
    assert_eq!(h.location(), Location::Sent("personal".into()));
    assert_eq!(h.visible(), sent, "the Sent location lists the outgoing message");
}
