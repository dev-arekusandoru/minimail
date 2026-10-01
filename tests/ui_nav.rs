//! Sidebar navigation: the accounts/folders sidebar, the list header's filter pills and the
//! `g`-prefix jumps.

use std::rc::Rc;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, ElementId, Entity, Focusable, Point, TestAppContext,
    WindowBounds, WindowOptions, base::Root, px, size,
};
use mail_classifier::app::MailApp;
use mail_classifier::app::actions::bind_keys;
use mail_classifier::clock::{Clock, FakeClock, Timestamp};

use mail_classifier::model::{Location, Mailbox, MessageId, OUTBOX_DELAY, TriageState};
use mail_classifier::app::filter_popover::Step;
use mail_classifier::search::Field;

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
    fn location(&mut self) -> Option<Location> {
        self.read(|a| a.location())
    }
    /// The query the list is showing, as text.
    fn query(&mut self) -> String {
        self.read(|a| a.query().describe())
    }
    /// Rows of the open filter picker, top to bottom.
    fn picker_rows(&mut self) -> Vec<String> {
        self.app.read_with(self.cx, |a, cx| a.filter_popover_rows(cx))
    }
    fn picker_step(&mut self) -> Option<Step> {
        self.app.read_with(self.cx, |a, cx| a.filter_popover_step(cx))
    }
    fn pills(&mut self) -> Vec<String> {
        self.read(|a| a.pill_texts())
    }
    fn header(&mut self) -> String {
        self.read(|a| a.header_title(a.visible_ids().len()))
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
    fn pill_row(&mut self) -> bool {
        self.has("list-pills")
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
fn sidebar_click_sets_the_location_and_clears_the_filters(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    assert_eq!(h.location(), Some(Location::AllInboxes));
    assert!(h.query().starts_with("in:inbox"), "the list starts on every inbox");

    // Two filter pills are set before the click.
    h.keys("2");
    h.click("btn-add-filter");
    h.keys("t a g");
    h.keys("enter");
    h.keys("r e m");
    h.keys("enter");
    assert_eq!(h.pills(), vec!["in:inbox", "tag:needs-reply", "and", "tag:reminder"]);

    h.click("2-3");
    assert_eq!(h.location(), Some(Location::Archive("work".into())));
    assert_eq!(h.query(), "in:archived account:work", "clicking a location clears the filters");
    assert!(h.pills().is_empty(), "no pills left, so no in: pill either");
    assert_eq!(h.visible(), vec![4], "only work's archived message");
    assert_eq!(h.state_of(4), TriageState::Archived);
    assert_eq!(h.read(|a| a.location_label()), "Work · Archive");

    h.click("1-0");
    assert_eq!(h.location(), Some(Location::Inbox("personal".into())));
    assert_eq!(h.visible(), vec![1]);
    assert_eq!(h.read(|a| a.location_label()), "Personal · Inbox");

    h.click("1-2");
    assert_eq!(h.location(), Some(Location::Sent("personal".into())));
    assert!(h.visible().is_empty(), "no replies sent yet");

    assert!(h.has("2-5"), "Projects is a work folder");
    assert!(h.has("2-5-0"), "Northwind nests under Projects");
    h.click("2-6");
    assert_eq!(h.location(), Some(Location::Folder(6)), "a folder of the work account");
    assert_eq!(h.read(|a| a.location_label()), "Recruiting");

    h.click(0usize);
    assert_eq!(h.read(|a| a.location_label()), "All Inboxes");
}

#[gpui_kit::gpui::test]
fn the_in_pill_is_hidden_while_alone_and_shown_once_anything_else_is_applied(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    assert!(h.pill_row());
    assert!(h.pills().is_empty(), "browsing a folder needs no pill");
    assert!(!h.has("pill-in-inbox"), "the folder itself is not a pill");
    assert!(!h.has("btn-clear-filters"), "nothing to clear");

    h.keys("2");
    assert_eq!(h.pills(), vec!["in:inbox", "tag:needs-reply"], "the folder pill appears with the first filter");
    assert!(h.has("pill-in-inbox"), "the location becomes removable once it filters");
    assert!(h.has("btn-clear-filters"), "there is something to clear");
    assert_eq!(h.header(), "All Inboxes · 0", "no mail carries that tag here");
}

#[gpui_kit::gpui::test]
fn removing_the_in_pill_from_a_filtered_folder_goes_global(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.click("2-0");
    assert_eq!(h.location(), Some(Location::Inbox("work".into())));
    h.keys("2");
    assert_eq!(h.pills(), vec!["in:inbox", "account:Work", "tag:needs-reply"]);
    assert_eq!(h.visible(), Vec::<MessageId>::new());

    h.click("pill-remove-in-inbox");
    assert_eq!(h.pills(), vec!["account:Work", "tag:needs-reply"], "the folder pill is gone");
    assert_eq!(h.location(), None, "nothing anchors the list any more");
    assert_eq!(h.read(|a| a.location_label()), "All mail");
    assert_eq!(h.query(), "account:work tag:needs-reply");
}

#[gpui_kit::gpui::test]
fn the_number_keys_toggle_quick_filter_pills_everywhere(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    assert_eq!(h.visible(), vec![1, 2], "both inbox messages");

    h.keys("2");
    assert_eq!(h.pills(), vec!["in:inbox", "tag:needs-reply"]);
    h.keys("3");
    assert_eq!(
        h.pills(),
        vec!["in:inbox", "tag:needs-reply", "and", "tag:follow-up"],
        "keys add pills, they do not replace"
    );
    h.keys("2");
    assert_eq!(h.pills(), vec!["in:inbox", "tag:follow-up"], "the same key removes its pill again");
    h.keys("3");
    assert!(h.pills().is_empty(), "back to the bare folder");

    // v1 disables the known-sender distinction, so key `5` (new senders) does nothing.
    h.keys("5");
    assert!(h.pills().is_empty(), "the New Senders key is inert");

    h.keys("4");
    h.keys("6");
    assert_eq!(h.pills(), vec!["in:inbox", "tag:urgent", "and", "tag:spam"]);

    // Quick filters are not Inbox-only: they work in the archive too.
    h.click("2-3");
    h.keys("1");
    assert_eq!(h.query(), "in:archived account:work", "key 1 drops every filter but the folder");
    h.keys("6");
    assert_eq!(h.pills(), vec!["in:archived", "account:Work", "tag:spam"]);
}

#[gpui_kit::gpui::test]
fn and_or_toggles_between_the_values_of_one_group(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    for who in ["a n n", "b o b"] {
        h.click("btn-add-filter");
        h.keys("f r o m");
        h.keys("enter");
        assert_eq!(h.picker_step(), Some(Step::Value(Field::From)), "people take free text");
        h.keys(who);
        h.keys("enter");
    }
    assert_eq!(h.pills(), vec!["in:inbox", "from:ann", "and", "from:bob"]);
    assert!(h.has("group-op-from-1"), "the second value of the group offers a toggle");
    assert!(h.visible().is_empty(), "no message is from both");

    h.click("group-op-from-1");
    assert_eq!(h.query(), "in:inbox from:ann,bob", "the group is now an OR");
    assert_eq!(h.pills(), vec!["in:inbox", "from:ann", "or", "from:bob"]);
    assert_eq!(h.visible(), vec![1, 2], "either sender's inbox mail");
    h.click("group-op-from-1");
    assert_eq!(h.query(), "in:inbox from:ann from:bob", "and back to AND");
    assert!(h.visible().is_empty());
}

#[gpui_kit::gpui::test]
fn the_picker_adds_pills_and_clear_takes_them_away(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.click("btn-add-filter");
    assert!(h.read(|a| a.filter_popover_open()));
    assert!(h.picker_rows().contains(&"Kind".to_string()));

    h.keys("a c c o u n t");
    h.keys("enter");
    let rows = h.picker_rows();
    assert_eq!(rows, vec!["Personal".to_string(), "Work".to_string()], "accounts come from the mailbox");
    h.keys("w o r k");
    h.keys("enter");
    assert_eq!(h.query(), "in:inbox account:work");
    assert_eq!(h.pills(), vec!["in:inbox", "account:Work"], "an account pill reads by name");
    assert_eq!(h.header(), "All Inboxes · 1", "still All Inboxes, narrowed to one account");
    assert_eq!(h.visible(), vec![2], "only work's inbox message");
    assert!(!h.read(|a| a.filter_popover_open()), "the picker closes once a pill is added");

    h.click("btn-add-filter");
    h.keys("i s");
    h.keys("enter");
    h.keys("a r c");
    h.keys("enter");
    assert_eq!(h.query(), "in:inbox account:work is:archived");
    assert!(h.visible().is_empty(), "nothing is both archived and in work's inbox");

    h.click("pill-remove-in-inbox");
    assert_eq!(h.query(), "account:work is:archived", "without the folder the search is global");
    assert_eq!(h.visible(), vec![4], "work's archived message");

    h.click("btn-clear-filters");
    assert_eq!(h.query(), "in:inbox");
    assert!(h.pills().is_empty());
    assert_eq!(h.visible(), vec![1, 2]);
}

#[gpui_kit::gpui::test]
fn clicking_a_pill_value_edits_it_and_saving_replaces_it(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.keys("2");
    h.keys("3");
    assert_eq!(h.visible(), Vec::<MessageId>::new());

    h.click("pill-value-tag-follow-up");
    assert!(h.read(|a| a.filter_popover_open()));
    assert_eq!(h.picker_step(), Some(Step::Value(Field::Tag)), "it opens on the value");
    assert!(h.has("pill-tag-follow-up"), "the pill stays in the header");

    h.keys("u r g e n t");
    h.keys("enter");
    assert!(!h.read(|a| a.filter_popover_open()), "applying closes the editor");
    assert_eq!(h.query(), "in:inbox tag:needs-reply tag:urgent", "the pill was replaced, not added");
    h.keys("2");
    assert_eq!(h.query(), "in:inbox tag:urgent", "the list keys work again after editing");
}

#[gpui_kit::gpui::test]
fn g_prefix_jumps_to_the_current_accounts_locations(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.keys("g s");
    assert_eq!(h.location(), Some(Location::Snoozed("personal".into())));
    h.keys("g a");
    assert_eq!(h.location(), Some(Location::Archive("personal".into())));
    h.keys("g d");
    assert_eq!(h.location(), Some(Location::Trash("personal".into())));
    h.keys("g t");
    assert_eq!(h.location(), Some(Location::Sent("personal".into())));
    h.keys("g i");
    assert_eq!(h.location(), Some(Location::Inbox("personal".into())));

    // From All Inboxes the first account answers, and `g i` keeps it unified.
    h.click(0usize);
    assert_eq!(h.location(), Some(Location::AllInboxes));
    h.keys("g a");
    assert_eq!(h.location(), Some(Location::Archive("personal".into())));
    h.click(0usize);
    h.keys("g i");
    assert_eq!(h.location(), Some(Location::AllInboxes), "`g i` stays on All Inboxes");

    // From an account's location the jumps stay in that account.
    h.click("2-0");
    h.keys("g a");
    assert_eq!(h.location(), Some(Location::Archive("work".into())));
    h.keys("g i");
    assert_eq!(h.location(), Some(Location::Inbox("work".into())));
}

#[gpui_kit::gpui::test]
fn sent_shows_the_outgoing_message_after_a_flushed_reply(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.click("1-0");
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
    assert_eq!(h.location(), Some(Location::Sent("personal".into())));
    assert_eq!(h.visible(), sent, "the Sent location lists the outgoing message");
}

#[gpui_kit::gpui::test]
fn a_search_from_a_folder_keeps_the_folder_until_its_pill_is_removed(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.click("2-3");
    h.keys("/");
    h.type_text("bob");
    h.keys("enter");
    assert_eq!(h.location(), Some(Location::Archive("work".into())), "the search runs inside the folder");
    assert_eq!(h.pills(), vec!["in:archived", "account:Work", "bob"], "and shows it as removable pills");
    assert_eq!(h.visible(), vec![4]);

    h.click("pill-remove-in-archived");
    assert_eq!(h.location(), None, "without its folder the search is global");
    assert_eq!(h.visible(), vec![2, 4], "bob's mail in every folder of the account");
    h.click("pill-remove-account-work");
    assert_eq!(h.query(), "\"bob\"");
    assert_eq!(h.read(|a| a.location_label()), "All mail");

    h.keys("escape");
    assert_eq!(h.query(), "in:archived account:work", "escape returns to the folder being browsed");
    assert!(h.pills().is_empty());
}

#[gpui_kit::gpui::test]
fn date_pills_take_typed_dates_the_calendar_and_relative_values(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.click("btn-add-filter");
    h.keys("a f t e r");
    h.keys("enter");
    assert_eq!(h.picker_step(), Some(Step::Value(Field::After)));
    h.type_text("2026-09-04");
    h.keys("enter");
    assert_eq!(h.query(), "in:inbox after:2026-09-04");
    assert_eq!(h.visible(), vec![1], "only mail from the 4th on");

    // The pill's editor shows its date on a calendar; picking a day replaces the value.
    h.click("pill-value-after-2026-09-04");
    let day = (0..6usize)
        .map(|week| format!("calendar-2026-09-03-0-{week}"))
        .find(|id| h.has(id.clone()))
        .expect("the calendar shows September 2026");
    h.click(day);
    assert_eq!(h.query(), "in:inbox after:2026-09-03", "the calendar sets an absolute date");
    assert_eq!(h.visible(), vec![1, 2]);

    // Relative values stay valid when typed.
    h.click("btn-add-filter");
    h.keys("b e f o r e");
    h.keys("enter");
    h.type_text("7d");
    h.keys("enter");
    assert_eq!(h.pills(), vec!["in:inbox", "after:2026-09-03", "before:7d"]);
    assert_eq!(h.visible(), vec![1, 2], "both are older than a week");

    // An invalid date keeps the picker open instead of adding a pill.
    h.click("btn-add-filter");
    h.keys("o n");
    h.keys("enter");
    h.type_text("soon");
    h.keys("enter");
    assert!(h.read(|a| a.filter_popover_open()), "an invalid date is not applied");
    assert_eq!(h.pills().len(), 3);
    h.keys("escape");
    assert!(!h.read(|a| a.filter_popover_open()), "escape cancels the picker");
    assert_eq!(h.pills().len(), 3, "and leaves the filters alone");
}

#[gpui_kit::gpui::test]
fn the_header_counts_what_the_list_shows_without_muted_threads(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    assert_eq!(h.header(), "All Inboxes · 2");
    h.keys("m");
    assert_eq!(h.header(), "All Inboxes · 1", "a muted thread leaves the count");
    h.click("1-0");
    assert_eq!(h.header(), "Personal · Inbox · 0");
}

#[gpui_kit::gpui::test]
fn filter_changes_are_view_state_not_undo_steps(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.keys("e");
    assert_eq!(h.state_of(1), TriageState::Archived);
    h.keys("4");
    assert_eq!(h.pills(), vec!["in:inbox", "tag:urgent"]);
    h.keys("u");
    assert_eq!(h.state_of(1), TriageState::Inbox, "undo reverts the archive, the last mail change");
    assert_eq!(h.pills(), vec!["in:inbox", "tag:urgent"], "the filter stays as it is");

    h.keys("escape");
    assert!(h.pills().is_empty(), "escape drops every filter but the folder");
    assert_eq!(h.visible(), vec![1, 2]);
}
