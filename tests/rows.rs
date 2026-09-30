//! Row design: icon language, overflow, preview setting, open vs cursor states.

use gpui_kit::{
    AppContext, Bounds, Entity, Focusable, Point, TestAppContext, WindowBounds, WindowOptions,
    base::Root, px, size,
};
use mail_classifier::app::actions::{CyclePreviewLines, bind_keys};
use mail_classifier::app::icons::{
    Glyph, GlyphInputs, glyphs_for, legend, max_icons, split_overflow,
};
use mail_classifier::app::row::{RowVisual, row_height};
use mail_classifier::app::MailApp;
use mail_classifier::judge::Kind;
use mail_classifier::model::{MessageId, Tag};

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
fn row_height_is_uniform_per_setting_and_grows_by_line() {
    let base = row_height(0);
    for n in 1..=5u8 {
        assert_eq!(row_height(n) - base, 16. * f32::from(n));
    }
    assert!(base >= 44., "room for sender and subject even with previews off");
}

#[test]
fn overflow_keeps_top_priority_icons_and_lists_the_rest() {
    let tags = [Tag::Spam, Tag::NeedsReply, Tag::Urgent(5), Tag::Kind(Kind::Receipt), Tag::Urgent(1)];
    let glyphs = glyphs_for(&GlyphInputs {
        tags: &tags,
        pending: &[],
        muted: true,
        new_sender: false,
        snoozed: false,
        attachment: true,
    });
    assert_eq!(glyphs[0], Glyph::Spam, "most important first");
    let (shown, hidden) = split_overflow(&glyphs, 3);
    assert_eq!(shown, [Glyph::Spam, Glyph::UrgentHigh, Glyph::NeedsReply]);
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
    let glyphs = [Glyph::UrgentLow, Glyph::KindOther, Glyph::Spam];
    let (shown, hidden) = split_overflow(&glyphs, 4);
    assert_eq!(shown, [Glyph::Spam]);
    assert_eq!(hidden, [Glyph::UrgentLow, Glyph::KindOther]);
    assert_eq!(Glyph::Suggestion.spec().priority, 0);
}

#[test]
fn every_glyph_is_documented_once_in_the_legend() {
    let listed: Vec<Glyph> = legend().into_iter().flat_map(|(_, g)| g).collect();
    assert_eq!(listed.len(), Glyph::ALL.len());
    for g in Glyph::ALL {
        let s = g.spec();
        assert!(!s.label.is_empty() && !s.description.is_empty(), "{g:?}");
        assert_eq!(listed.iter().filter(|x| **x == g).count(), 1, "{g:?}");
    }
}

#[gpui_kit::gpui::test]
fn row_is_selectable_and_openable(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let first = h.ids()[0];
    h.keys("x");
    assert!(h.read(|a| a.row_visual(first).checked), "x checks the cursor row");
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
        RowVisual { cursor: true, open: true, checked: false },
        "cursor on the open row combines both cues"
    );
    assert!(h.read(|a| a.row_visual(first).combined()));

    h.keys("j");
    let open_only = h.read(|a| a.row_visual(first));
    let cursor_only = h.read(|a| a.row_visual(second));
    assert_eq!(open_only, RowVisual { cursor: false, open: true, checked: false });
    assert_eq!(cursor_only, RowVisual { cursor: true, open: false, checked: false });
    assert_ne!(open_only, cursor_only);
    assert_eq!(h.read(|a| a.opened()), Some(first), "moving the cursor keeps the reader open");

    h.keys("k");
    assert!(h.read(|a| a.row_visual(first).combined()));
    assert_eq!(h.read(|a| a.row_visual(second)), RowVisual::default());
}

#[gpui_kit::gpui::test]
fn checked_state_is_separate_from_cursor_and_open(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let ids = h.ids();
    h.keys("enter x j");
    let v = h.read(|a| a.row_visual(ids[0]));
    assert_eq!(v, RowVisual { cursor: false, open: true, checked: true });
}
