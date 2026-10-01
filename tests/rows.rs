//! Row design: icon language, overflow, preview setting, open vs cursor states.

use gpui_kit::{
    AppContext, Bounds, Entity, Focusable, Point, TestAppContext, WindowBounds, WindowOptions,
    base::Root, px, size,
};
use mail_classifier::app::actions::{CyclePreviewLines, bind_keys};
use mail_classifier::app::icons::{
    Glyph, GlyphInputs, glyphs_for, legend, max_icons, split_overflow,
};
use mail_classifier::app::row::{
    BarState, LINE_H, PREVIEW_LINE_H, RowVisual, message_row_height, sender_label,
    shows_account_icon, thread_preview_lines, thread_row_height,
};
use mail_classifier::app::MailApp;
use mail_classifier::judge::Kind;
use mail_classifier::model::{Location, Mailbox, MessageId, Tag};

struct Harness<'a> {
    cx: &'a mut TestAppContext,
    window: gpui_kit::AnyWindowHandle,
    app: Entity<MailApp>,
}

fn harness(cx: &mut TestAppContext) -> Harness<'_> {
    cx.update(|cx| {
        gpui_kit::init(cx);
        bind_keys(cx);
    });
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
                let view = cx.new(|cx| {
                    MailApp::new(mail_classifier::model::Mailbox::load_default(), window, cx)
                });
                window.focus(&view.focus_handle(cx), cx);
                view
            },
        )
        .expect("open window");
        (window.downcast::<Root>().expect("root").into(), content)
    });
    let mut h = Harness { cx, window, app };
    h.keys("");
    h
}

impl Harness<'_> {
    fn keys(&mut self, keys: &str) {
        if !keys.is_empty() {
            self.cx.simulate_keystrokes(self.window, keys);
        }
        self.cx.run_until_parked();
    }
    fn read<R>(&mut self, f: impl FnOnce(&MailApp) -> R) -> R {
        self.cx.read_entity(&self.app, |a, _| f(a))
    }
    fn ids(&mut self) -> Vec<MessageId> {
        self.read(|a| a.visible_ids())
    }
}

#[gpui_kit::gpui::test]
fn preview_setting_cycles_from_off_to_five(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    assert_eq!(h.read(|a| a.preview_lines), 2, "default is two lines");
    let mut seen = vec![2];
    for _ in 0..5 {
        h.cx.dispatch_action(h.window, CyclePreviewLines);
        h.cx.run_until_parked();
        seen.push(h.read(|a| a.preview_lines));
    }
    assert_eq!(seen, [2, 3, 4, 5, 0, 1], "cycle wraps 5 -> Off -> 1");
}

#[test]
fn message_row_height_grows_by_one_preview_line() {
    let base = message_row_height(0);
    for n in 1..=5u8 {
        assert_eq!(message_row_height(n) - base, PREVIEW_LINE_H * f32::from(n));
    }
    assert!(base >= 44., "room for sender and subject even with previews off");
    assert!(base > 2. * LINE_H, "previews never squeeze out a line");
}

#[test]
fn thread_header_sizes_to_its_own_content_and_never_matches_a_message_row() {
    for n in 0..=5u8 {
        let header = thread_row_height(n);
        let message = message_row_height(n);
        assert!(
            header < message,
            "a thread header at {n} preview lines ({header}) must be shorter than a message row ({message})"
        );
        assert_eq!(header, 24. + PREVIEW_LINE_H * f32::from(thread_preview_lines(n)));
    }
    assert!(thread_row_height(1) > thread_row_height(0), "it follows the preview setting");
    assert_eq!(thread_preview_lines(0), 0, "no preview lines, no preview block");
    assert_eq!(thread_row_height(5), thread_row_height(2), "a header caps its preview");
}

#[test]
fn overflow_keeps_top_priority_icons_and_lists_the_rest() {
    let tags = [Tag::PossibleSpam, Tag::NeedsReply, Tag::Urgent(5), Tag::Kind(Kind::Receipt), Tag::Urgent(1)];
    let glyphs = glyphs_for(&GlyphInputs {
        tags: &tags,
        pending: &[],
        muted: true,
        new_sender: false,
        snoozed: false,
        attachment: true,
    });
    assert_eq!(glyphs[0], Glyph::PossibleSpam, "most important first");
    let (shown, hidden) = split_overflow(&glyphs, 3);
    assert_eq!(shown, [Glyph::PossibleSpam, Glyph::UrgentHigh, Glyph::NeedsReply]);
    assert!(hidden.contains(&Glyph::UrgentLow), "low-signal glyphs only appear in +N");
    assert!(hidden.contains(&Glyph::KindReceipt));
    assert_eq!(shown.len() + hidden.len(), glyphs.len());
}

#[test]
fn narrower_panels_show_fewer_icons() {
    assert!(max_icons(560.) > max_icons(360.));
    assert!(max_icons(300.) >= 1);
}

#[test]
fn suggestion_is_first_and_low_signal_never_takes_a_slot() {
    let glyphs = [Glyph::UrgentLow, Glyph::KindOther, Glyph::PossibleSpam];
    let (shown, hidden) = split_overflow(&glyphs, 4);
    assert_eq!(shown, [Glyph::PossibleSpam]);
    assert_eq!(hidden, [Glyph::UrgentLow, Glyph::KindOther]);
    assert_eq!(Glyph::Suggestion.spec().priority, 0);
}

#[test]
fn every_glyph_is_documented_once_in_the_legend() {
    let listed: Vec<Glyph> = legend().into_iter().flat_map(|(_, g)| g).collect();
    // The New Sender glyph drops out of the legend while the feature is off.
    let documented =
        |g: Glyph| g != Glyph::NewSender || mail_classifier::known_senders::KNOWN_SENDERS_ENABLED;
    assert_eq!(listed.len(), Glyph::ALL.iter().filter(|g| documented(**g)).count());
    for g in Glyph::ALL {
        let s = g.spec();
        assert!(!s.label.is_empty() && !s.description.is_empty(), "{g:?}");
        assert_eq!(
            listed.iter().filter(|x| **x == g).count(),
            documented(g) as usize,
            "{g:?}"
        );
    }
}

#[gpui_kit::gpui::test]
fn row_is_selectable_and_openable(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let first = h.ids()[0];
    h.keys("x");
    assert!(h.read(|a| a.row_visual(first).selected), "x selects the cursor row");
    h.keys("x enter");
    assert_eq!(h.read(|a| a.opened()), Some(first));
}

#[gpui_kit::gpui::test]
fn open_and_cursor_states_are_independent_and_combine(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let ids = h.ids();
    let (first, second) = (ids[0], ids[1]);
    h.keys("enter");
    assert_eq!(
        h.read(|a| a.row_visual(first)),
        RowVisual { cursor: true, open: true, selected: false, ..RowVisual::default() },
        "cursor on the open row combines both cues"
    );
    assert!(h.read(|a| a.row_visual(first).combined()));

    h.keys("j");
    let open_only = h.read(|a| a.row_visual(first));
    let cursor_only = h.read(|a| a.row_visual(second));
    assert!(open_only.open && !open_only.cursor && !open_only.selected);
    assert!(cursor_only.cursor && !cursor_only.open && !cursor_only.selected);
    assert_ne!(open_only, cursor_only);
    assert_eq!(h.read(|a| a.opened()), Some(first), "moving the cursor keeps the reader open");

    h.keys("k");
    assert!(h.read(|a| a.row_visual(first).combined()));
    let second_after_return = h.read(|a| a.row_visual(second));
    assert!(second_after_return.unread, "moving the cursor does not mark other rows read");
}

#[gpui_kit::gpui::test]
fn selected_state_is_separate_from_cursor_and_open(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let ids = h.ids();
    h.keys("enter x j");
    let v = h.read(|a| a.row_visual(ids[0]));
    assert_eq!(v, RowVisual { cursor: false, open: true, selected: true, ..RowVisual::default() });
}

#[gpui_kit::gpui::test]
fn unread_selected_open_and_cursor_states_remain_distinct(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let first = h.ids()[0];
    let unread = h.read(|a| a.row_visual(first));
    assert!(unread.unread, "new inbox messages expose the unread bar state");
    assert!(!unread.selected);

    h.keys("x");
    let selected = h.read(|a| a.row_visual(first));
    assert!(selected.selected, "selection uses the full accent bar");
    assert!(selected.unread, "unread remains in the visual state while selected");

    h.keys("enter");
    let open_selected_cursor = h.read(|a| a.row_visual(first));
    assert!(open_selected_cursor.open && open_selected_cursor.cursor && open_selected_cursor.selected);
    assert_ne!(open_selected_cursor, unread);
    h.keys("x");
    let open_deselected_cursor = h.read(|a| a.row_visual(first));
    assert!(open_deselected_cursor.open && open_deselected_cursor.cursor && !open_deselected_cursor.selected);
    assert_eq!(open_deselected_cursor.bar_state(), BarState::Selected);
    h.keys("j");
    let open_deselected = h.read(|a| a.row_visual(first));
    assert!(open_deselected.open && !open_deselected.selected && !open_deselected.cursor);
    assert_eq!(open_deselected.bar_state(), BarState::Selected);
    assert!(!open_deselected.unread, "opening marks the message read without restoring the unread bar");
}

#[test]
fn every_triage_tag_picks_its_own_badge() {
    let cases = [
        (Tag::NeedsReply, Glyph::NeedsReply),
        (Tag::AwaitingReply, Glyph::AwaitingReply),
        (Tag::FollowUp, Glyph::FollowUp),
        (Tag::Reminder, Glyph::Reminder),
        (Tag::PossibleSpam, Glyph::PossibleSpam),
        (Tag::Urgent(5), Glyph::UrgentHigh),
        (Tag::Urgent(3), Glyph::UrgentMid),
        (Tag::Kind(Kind::Receipt), Glyph::KindReceipt),
    ];
    for (tag, expected) in cases {
        let glyphs = glyphs_for(&GlyphInputs {
            tags: std::slice::from_ref(&tag),
            pending: &[],
            muted: false,
            new_sender: false,
            snoozed: false,
            attachment: false,
        });
        assert_eq!(glyphs, vec![expected], "{tag:?} shows exactly its badge");
    }
}

#[test]
fn new_sender_badge_is_derived_from_the_mailbox() {
    let glyphs = glyphs_for(&GlyphInputs {
        tags: &[],
        pending: &[],
        muted: false,
        new_sender: true,
        snoozed: false,
        attachment: false,
    });
    assert_eq!(glyphs, vec![Glyph::NewSender], "New Sender is not a stored tag");
}

#[test]
fn account_icon_shows_only_in_the_unified_inbox() {
    assert!(shows_account_icon(&Location::AllInboxes));
    assert!(!shows_account_icon(&Location::Inbox("personal".into())));
    assert!(!shows_account_icon(&Location::Snoozed("personal".into())));
    assert!(!shows_account_icon(&Location::Sent("personal".into())));
    assert!(!shows_account_icon(&Location::Archive("personal".into())));
    assert!(!shows_account_icon(&Location::Trash("personal".into())));
    assert!(!shows_account_icon(&Location::Folder(1)));

    let mailbox = Mailbox::load_default();
    for account in mailbox.accounts() {
        assert!(
            mail_classifier::theme::parse_color(&account.color).is_some(),
            "account {} has a parseable color",
            account.id
        );
    }
}

#[test]
fn every_selectable_account_icon_has_a_lucide_glyph() {
    for (key, _) in mail_classifier::account_style::ICONS {
        assert!(mail_classifier::app::icons::account_icon_name(key).is_some(), "{key}");
    }
    assert!(mail_classifier::app::icons::account_icon_name("nope").is_none());
}

#[test]
fn outgoing_rows_are_labelled_by_recipient() {
    let mut message = Mailbox::load_default().messages()[0].clone();
    message.outgoing = true;
    message.from_name = "You".into();
    message.to = "alice@example.com".into();
    assert_eq!(sender_label(&message), "To: alice@example.com");

    message.outgoing = false;
    message.from_name = "Alice".into();
    assert_eq!(sender_label(&message), "Alice", "incoming mail keeps its sender");
}

#[test]
fn row_bar_state_uses_the_required_precedence() {
    let state = |unread, urgent, selected, open| {
        RowVisual { unread, urgent, selected, open, ..RowVisual::default() }.bar_state()
    };
    assert_eq!(state(true, false, false, false), BarState::Unread);
    assert_eq!(state(true, true, false, false), BarState::Urgent);
    assert_eq!(state(false, true, false, false), BarState::None);
    assert_eq!(state(true, false, true, false), BarState::Selected);
    assert_eq!(state(false, false, false, true), BarState::Selected);
    assert_eq!(state(true, true, true, false), BarState::Selected);
    assert_eq!(state(false, false, false, false), BarState::None);
    assert_eq!(state(false, false, false, true), BarState::Selected);
    assert_eq!(
        RowVisual { partial: true, ..RowVisual::default() }.bar_state(),
        BarState::Partial
    );
    for bar in [
        RowVisual::default(),
        RowVisual { unread: true, ..RowVisual::default() },
        RowVisual { unread: true, urgent: true, ..RowVisual::default() },
        RowVisual { selected: true, ..RowVisual::default() },
        RowVisual { open: true, ..RowVisual::default() },
        RowVisual { selected: true, urgent: true, ..RowVisual::default() },
    ] {
        let focused = RowVisual { cursor: true, ..bar };
        assert_eq!(focused.bar_state(), bar.bar_state(), "cursor does not replace bar state");
        assert_ne!(focused, bar, "cursor remains independently represented");
    }
}
