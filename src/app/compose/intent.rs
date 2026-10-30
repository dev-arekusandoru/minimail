//! What happens after Send: the `then …` clause of the sentence, the intent menu that
//! confirms it, and the frozen container with its "Sending…" pill while the message is held.
//!
//! The view only chooses and shows. The chosen [`SendIntent`] lives on the draft record
//! (saved with it), and the owner carries it out once the provider confirms the send.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::base::{Align, ElementExt as _, Placement, Positioner};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::{ComposeEvent, Composer};
use crate::app::actions::UndoSendHold;
use crate::app::chrome::format_time;
use crate::app::ui::shortcut;
use crate::clock::{Clock, Timestamp};
use crate::model::Mailbox;
use crate::outgoing::{
    BadTime, DEFAULT_REMIND_DAYS, DraftState, PendingHold, SendIntent, hold_progress,
    parse_send_later,
};
use crate::tz::{Now, Zone};

gpui_kit::actions!(
    compose_intent,
    [
        IntentPick1,
        IntentPick2,
        IntentPick3,
        IntentCustom,
        IntentJustSend,
        IntentConfirm,
        IntentNext,
        IntentPrev,
        IntentFewerDays,
        IntentMoreDays,
        IntentCancel
    ]
);

/// Key context of the open intent menu (digits, enter, escape and arrows bind under it).
pub const INTENT_CONTEXT: &str = "ComposeIntent";

/// The same, minus keys typed into the custom time field, which owns its digits and Enter.
pub const INTENT_KEYS: &str = "ComposeIntent && !Input";

/// The day counts the Remind row steps through with ← →.
const REMIND_DAYS: [u32; 6] = [1, 2, 3, 5, 7, 14];

const DIM: f32 = 0.38;

/// Which page the menu is showing.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Main,
    Later,
    Custom,
}

/// The menu's own state: which page, which row is lit, whether picking also sends, the
/// Remind day count, and what the custom entry last complained about.
struct Menu {
    page: Page,
    selected: usize,
    and_send: bool,
    days: u32,
    error: Option<&'static str>,
}

/// Everything intent-related the composer keeps: the open menu, the hold it is frozen
/// under, the one-line "restored" notice, the clock presets are read from, the custom
/// time field, and where the token and Send were last painted.
pub struct IntentState {
    focus: FocusHandle,
    menu: Option<Menu>,
    hold: Option<PendingHold>,
    restored: bool,
    time: Option<(Rc<dyn Clock>, Rc<dyn Zone>)>,
    custom_input: Option<Entity<InputState>>,
    token_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    send_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
}

impl IntentState {
    pub fn new(cx: &mut App) -> Self {
        Self {
            focus: cx.focus_handle(),
            menu: None,
            hold: None,
            restored: false,
            time: None,
            custom_input: None,
            token_bounds: Rc::new(Cell::new(None)),
            send_bounds: Rc::new(Cell::new(None)),
        }
    }
}

/// One row of the menu.
struct Row {
    icon: IconName,
    label: String,
    detail: String,
    key: &'static str,
}

fn plural_days(days: u32) -> String {
    format!("{days} day{}", if days == 1 { "" } else { "s" })
}

/// The row an intent lights on the main page.
fn row_of(intent: SendIntent) -> usize {
    match intent {
        SendIntent::Archive => 0,
        SendIntent::RemindIfNoReply { .. } => 1,
        SendIntent::SendLater { .. } => 2,
        SendIntent::JustSend => 3,
    }
}

/// The next day count after `days` in `direction` (`+1` / `-1`), staying inside the list.
fn step_days(days: u32, direction: i32) -> u32 {
    let at = REMIND_DAYS.iter().position(|d| *d >= days).unwrap_or(2);
    let to = (at as i32 + direction).clamp(0, REMIND_DAYS.len() as i32 - 1);
    REMIND_DAYS[to as usize]
}

impl Composer {
    /// Give the composer the clock its send-later presets and countdown read.
    pub fn set_clock(&mut self, clock: Rc<dyn Clock>, zone: Rc<dyn Zone>) {
        self.intent.time = Some((clock, zone));
    }

    /// The hold this draft is waiting out, pushed by the owner (`None` once it is over).
    pub fn set_hold(&mut self, hold: Option<PendingHold>, cx: &mut Context<Self>) {
        if self.intent.hold != hold {
            self.intent.hold = hold;
            cx.notify();
        }
    }

    /// An undo put the draft back: say so once, until the next edit.
    pub fn show_restored_notice(&mut self, cx: &mut Context<Self>) {
        self.intent.restored = true;
        cx.notify();
    }

    pub fn dismiss_restored_notice(&mut self, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.intent.restored) {
            cx.notify();
        }
    }

    pub fn intent(&self) -> SendIntent {
        self.draft.intent
    }

    /// Change what sending does to the thread. An edit like any other: the draft saves it.
    pub fn set_intent(&mut self, intent: SendIntent, cx: &mut Context<Self>) {
        if self.frozen() || self.draft.intent == intent {
            return;
        }
        self.draft.intent = intent;
        cx.emit(ComposeEvent::Edited);
        cx.notify();
    }

    fn now(&self) -> Option<Now> {
        let (clock, zone) = self.intent.time.as_ref()?;
        Some(Now::new(clock.now(), zone.clone()))
    }

    /// Icon and words for an intent, as the token and the menu say them.
    fn intent_words(&self, intent: SendIntent) -> (IconName, String) {
        match intent {
            SendIntent::JustSend => (IconName::Send, "just send".into()),
            SendIntent::Archive => (IconName::Archive, "archive thread".into()),
            SendIntent::RemindIfNoReply { days } => (IconName::Bell, format!("remind in {days}d")),
            SendIntent::SendLater { at } => {
                let when = self
                    .now()
                    .map_or_else(|| "later".to_owned(), |now| format_time(at, &now));
                (IconName::Clock, format!("send at {when}"))
            }
        }
    }

    // ------------------------------------------------------------------ the clause

    /// `then [token]` plus quiet words for the alternatives still unset. Any click opens
    /// the menu; picking there changes the sentence and does not send.
    pub fn render_intent_clause(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let frozen = self.frozen();
        let open = self.intent.menu.is_some();
        let intent = self.draft.intent;
        let (muted, border, field, strong, primary) = {
            let t = cx.theme();
            (t.muted_foreground, t.border, t.secondary, t.foreground, t.primary)
        };
        let (icon, words) = self.intent_words(intent);
        // Measured each frame so the menu this opens hangs under it, wherever it wraps to.
        let anchor = self.intent.token_bounds.clone();
        let token = div()
            .id("compose-intent")
            .debug_selector(|| "compose-intent".into())
            .test_support()
            .flex()
            .items_center()
            .gap(px(6.))
            .h(px(24.))
            .px(px(7.))
            .rounded(px(6.))
            .border_1()
            .border_color(if open { primary } else { border })
            .bg(field)
            .text_size(px(13.))
            .line_height(px(20.))
            .text_color(strong)
            .on_prepaint(move |bounds, _, _| anchor.set(Some(bounds)))
            .child(Icon::new(icon).with_size(px(13.)).text_color(muted))
            .child(words)
            .when(!frozen, |d| {
                d.cursor_pointer().on_click(cx.listener(|this, _, window, cx| {
                    this.open_menu(false, None, window, cx)
                }))
            });

        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(6.))
            .child(div().text_size(px(13.)).line_height(px(20.)).text_color(muted).child("then"))
            .child(token)
            .into_any_element()
    }

    // ------------------------------------------------------------------- the menu

    /// Open the menu from Send: picking a row also sends, and the menu hangs under the
    /// Send control.
    pub fn open_intent_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_menu(true, None, window, cx);
    }

    /// Where the Send control was last painted. The Send menu is anchored to it, so it
    /// follows the footer instead of a fixed corner.
    pub(super) fn send_anchor(&self) -> Rc<Cell<Option<Bounds<Pixels>>>> {
        self.intent.send_bounds.clone()
    }

    /// Open the menu, anchored under the token — or under Send, when picking from it also
    /// sends.
    pub(super) fn open_menu(
        &mut self,
        and_send: bool,
        row: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.frozen() || (and_send && !self.can_send()) {
            return;
        }
        let days = match self.draft.intent {
            SendIntent::RemindIfNoReply { days } => days,
            _ => DEFAULT_REMIND_DAYS,
        };
        self.intent.menu = Some(Menu {
            page: Page::Main,
            selected: row.unwrap_or_else(|| row_of(self.draft.intent)),
            and_send,
            days,
            error: None,
        });
        window.focus(&self.intent.focus, cx);
        cx.notify();
    }

    fn close_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.intent.menu.take().is_some() {
            self.focus_body(window, cx);
            cx.notify();
        }
    }

    /// Apply a choice: the sentence changes, and Send goes ahead when the menu came from it.
    fn choose(&mut self, intent: SendIntent, window: &mut Window, cx: &mut Context<Self>) {
        let and_send = self.intent.menu.take().is_some_and(|m| m.and_send);
        self.set_intent(intent, cx);
        self.focus_body(window, cx);
        if and_send && self.can_send() {
            cx.emit(ComposeEvent::Send);
        }
        cx.notify();
    }

    fn later_presets(&self) -> Vec<(&'static str, Timestamp)> {
        self.now()
            .map(|now| Mailbox::snooze_presets(&now).to_vec())
            .unwrap_or_default()
    }

    /// Take row `ix` of the current page: a digit key, a click or Enter on the lit row.
    fn pick(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = self.intent.menu.as_ref() else {
            return;
        };
        if menu.page == Page::Custom {
            return;
        }
        if menu.page == Page::Later {
            let presets = self.later_presets();
            match presets.get(ix) {
                Some((_, at)) => self.choose(SendIntent::SendLater { at: *at }, window, cx),
                None if ix == presets.len() => self.open_custom(window, cx),
                None => {}
            }
            return;
        }
        let days = menu.days;
        let and_send = menu.and_send;
        let intent = self.draft.intent;
        match ix {
            0 => self.choose(SendIntent::Archive, window, cx),
            1 => self.choose(SendIntent::RemindIfNoReply { days }, window, cx),
            // Send later: Send confirms a time already chosen; the token's menu asks when.
            2 if and_send && matches!(intent, SendIntent::SendLater { .. }) => {
                self.choose(intent, window, cx)
            }
            2 if self.now().is_some() => {
                if let Some(menu) = self.intent.menu.as_mut() {
                    menu.page = Page::Later;
                    menu.selected = 1;
                }
                cx.notify();
            }
            3 => self.choose(SendIntent::JustSend, window, cx),
            _ => {}
        }
    }

    /// The custom send-later page: a field for a date and time, focused so it can be typed
    /// straight away.
    fn open_custom(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = match self.intent.custom_input.clone() {
            Some(input) => input,
            None => {
                let input = cx.new(|cx| InputState::new(window, cx).placeholder("tomorrow 09:00"));
                cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        this.submit_custom(window, cx);
                    }
                })
                .detach();
                self.intent.custom_input = Some(input.clone());
                input
            }
        };
        if let Some(menu) = self.intent.menu.as_mut() {
            menu.page = Page::Custom;
            menu.error = None;
        }
        window.focus(&input.focus_handle(cx), cx);
        cx.notify();
    }

    /// Enter in the custom field: take the typed time, or say why it will not do.
    fn submit_custom(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(input), Some(now)) = (self.intent.custom_input.clone(), self.now()) else {
            return;
        };
        let text = input.read(cx).value().to_string();
        let error = match parse_send_later(text.trim(), &now) {
            Ok(at) => return self.choose(SendIntent::SendLater { at }, window, cx),
            Err(BadTime::Past) => "That time has passed",
            Err(BadTime::Unreadable) => "Try 14:30, tomorrow 9:00 or 2026-11-03 08:15",
        };
        if let Some(menu) = self.intent.menu.as_mut() {
            menu.error = Some(error);
        }
        cx.notify();
    }

    /// `4` on the send-later page: type the time instead of taking a preset.
    fn custom_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.intent.menu.as_ref().is_some_and(|m| m.page == Page::Later) {
            self.open_custom(window, cx);
        }
    }

    fn move_selection(&mut self, direction: i32, cx: &mut Context<Self>) {
        let rows = match self.intent.menu.as_ref() {
            Some(menu) if menu.page == Page::Later => self.later_presets().len() + 1,
            Some(menu) if menu.page == Page::Main => 4,
            _ => return,
        };
        if let Some(menu) = self.intent.menu.as_mut() {
            menu.selected = (menu.selected as i32 + direction).rem_euclid(rows as i32) as usize;
        }
        cx.notify();
    }

    fn step_remind_days(&mut self, direction: i32, cx: &mut Context<Self>) {
        if let Some(menu) = self.intent.menu.as_mut()
            && menu.page == Page::Main
        {
            menu.days = step_days(menu.days, direction);
            cx.notify();
        }
    }

    fn main_rows(&self, days: u32) -> Vec<Row> {
        let mut rows = vec![
            Row {
                icon: IconName::Archive,
                label: "Archive thread".into(),
                detail: "leaves the inbox".into(),
                key: "1",
            },
            Row {
                icon: IconName::Bell,
                label: "Remind if no reply".into(),
                detail: plural_days(days),
                key: "2",
            },
        ];
        let tomorrow = self
            .now()
            .map(|now| {
                let presets = Mailbox::snooze_presets(&now);
                let (label, at) = presets[1];
                format!("{} {}", label.to_lowercase(), now.hhmm(at))
            })
            .unwrap_or_default();
        rows.push(Row {
            icon: IconName::Clock,
            label: "Send later".into(),
            detail: tomorrow,
            key: "3",
        });
        rows.push(Row {
            icon: IconName::Send,
            label: "Just send".into(),
            detail: String::new(),
            key: "cmd-enter",
        });
        rows
    }

    fn later_rows(&self) -> Vec<Row> {
        let Some(now) = self.now() else {
            return Vec::new();
        };
        let mut rows: Vec<Row> = self
            .later_presets()
            .into_iter()
            .zip(["1", "2", "3"])
            .map(|((label, at), key)| Row {
                icon: IconName::Clock,
                label: label.into(),
                detail: format_time(at, &now),
                key,
            })
            .collect();
        rows.push(Row {
            icon: IconName::Calendar,
            label: "Custom…".into(),
            detail: "a date and time".into(),
            key: "4",
        });
        rows
    }

    /// The floating menu, anchored under whatever opened it: the `then` token, or Send.
    /// `None` while it is closed.
    pub(super) fn render_intent_menu(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.intent.menu.as_ref()?;
        let (page, selected, days, error) = (menu.page, menu.selected, menu.days, menu.error);
        let anchor = if menu.and_send {
            self.intent.send_bounds.get()
        } else {
            self.intent.token_bounds.get()
        };
        let mono = crate::app::ui::mono_font(cx);
        let t = cx.theme();
        let (bg, border, fg, strong, muted, hover, primary, danger) = (
            t.popover,
            t.border,
            t.popover_foreground,
            t.foreground,
            t.muted_foreground,
            t.secondary_hover,
            t.primary,
            t.danger,
        );
        let mut list = div()
            .id("compose-intent-menu")
            .debug_selector(|| "compose-intent-menu".into())
            .test_support()
            .key_context(INTENT_CONTEXT)
            .track_focus(&self.intent.focus)
            .w(px(340.))
            .p(px(4.))
            .flex()
            .flex_col()
            .rounded(px(8.))
            .border_1()
            .border_color(border)
            .bg(bg)
            .shadow_lg()
            .occlude()
            .on_mouse_down_out(cx.listener(|this, _, window, cx| this.close_menu(window, cx)))
            .on_action(cx.listener(|this, _: &IntentPick1, w, cx| this.pick(0, w, cx)))
            .on_action(cx.listener(|this, _: &IntentPick2, w, cx| this.pick(1, w, cx)))
            .on_action(cx.listener(|this, _: &IntentPick3, w, cx| this.pick(2, w, cx)))
            .on_action(cx.listener(|this, _: &IntentCustom, w, cx| this.custom_key(w, cx)))
            .on_action(cx.listener(|this, _: &IntentJustSend, w, cx| {
                if this.intent.menu.as_ref().is_some_and(|m| m.page == Page::Main) {
                    this.pick(3, w, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &IntentConfirm, w, cx| {
                let selected = this.intent.menu.as_ref().map(|m| m.selected);
                if let Some(ix) = selected {
                    this.pick(ix, w, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &IntentNext, _, cx| this.move_selection(1, cx)))
            .on_action(cx.listener(|this, _: &IntentPrev, _, cx| this.move_selection(-1, cx)))
            .on_action(cx.listener(|this, _: &IntentFewerDays, _, cx| this.step_remind_days(-1, cx)))
            .on_action(cx.listener(|this, _: &IntentMoreDays, _, cx| this.step_remind_days(1, cx)))
            .on_action(cx.listener(|this, _: &IntentCancel, w, cx| this.close_menu(w, cx)))
            .child(
                div()
                    .px(px(8.))
                    .pt(px(4.))
                    .pb(px(2.))
                    .font_family(mono.clone())
                    .text_size(px(9.))
                    .line_height(px(16.))
                    .text_color(muted)
                    .child(match page {
                        Page::Main => "AFTER SENDING",
                        Page::Later => "SEND LATER",
                        Page::Custom => "SEND LATER · CUSTOM",
                    }),
            );
        if page == Page::Custom {
            list = list.child(
                div()
                    .px(px(8.))
                    .pb(px(6.))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .when_some(self.intent.custom_input.clone(), |d, input| {
                        d.child(Input::new(&input))
                    })
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(muted)
                            .child("14:30 · tomorrow 9:00 · 2026-11-03 08:15"),
                    )
                    .when_some(error, |d, why| {
                        d.child(div().text_size(px(11.)).text_color(danger).child(why))
                    }),
            );
        } else {
            let rows = if page == Page::Later {
                self.later_rows()
            } else {
                self.main_rows(days)
            };
            let last = rows.len().saturating_sub(1);
            for (ix, row) in rows.into_iter().enumerate() {
                let lit = ix == selected;
                // Just send sits apart, under a rule.
                if page == Page::Main && ix == last {
                    list = list.child(div().h(px(1.)).mx(px(6.)).my(px(4.)).bg(border));
                }
                let quiet = page == Page::Main && ix == last;
                list = list.child(
                    div()
                        .id(("compose-intent-row", ix))
                        .test_support()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .h(px(28.))
                        .px(px(8.))
                        .rounded(px(5.))
                        .text_size(px(12.))
                        .cursor_pointer()
                        .text_color(if quiet { muted } else { fg })
                        .when(lit, |d| d.bg(hover).text_color(strong))
                        .hover(move |s| s.bg(hover))
                        .on_click(cx.listener(move |this, _, w, cx| this.pick(ix, w, cx)))
                        .child(
                            Icon::new(row.icon)
                                .with_size(px(15.))
                                .text_color(if lit { primary } else { muted }),
                        )
                        .child(
                            div()
                                .when(!quiet, |d| {
                                    d.font_weight(FontWeight::MEDIUM).text_color(strong)
                                })
                                .child(row.label),
                        )
                        .child(div().text_size(px(11.)).text_color(muted).child(row.detail))
                        .child(div().ml_auto().child(shortcut(row.key))),
                );
            }
        }
        Some(
            Positioner::side(anchor?)
                .placement(Placement::Bottom)
                .align(Align::Start)
                .offset(px(6.))
                .occlude()
                .child(list)
                .into_any_element(),
        )
    }

    // ------------------------------------------------------------- the frozen state

    /// Dim a region while the message is held or on its way, and make it inert.
    pub(super) fn dimmed(&self, el: impl IntoElement, grow: bool) -> AnyElement {
        if !self.frozen() {
            return el.into_any_element();
        }
        div()
            .relative()
            .opacity(DIM)
            .when(grow, |d| d.flex_1().min_h_0().flex().flex_col())
            .child(el)
            .child(div().absolute().inset_0().occlude())
            .into_any_element()
    }

    /// "Draft restored — nothing was sent", above the sentence until the next edit.
    pub(super) fn render_restored_notice(&self, cx: &App) -> Option<AnyElement> {
        if !self.intent.restored {
            return None;
        }
        let t = cx.theme();
        Some(
            div()
                .id("compose-restored")
                .test_support()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(7.))
                .px(px(14.))
                .py(px(6.))
                .text_size(px(11.))
                .text_color(t.success)
                .bg(t.success.opacity(0.08))
                .border_b_1()
                .border_color(t.border)
                .child(Icon::new(IconName::Check).with_size(px(12.)))
                .child("Draft restored — nothing was sent")
                .into_any_element(),
        )
    }

    /// The pill inside the body: send icon, "Sending…" (or when a later send goes), the
    /// shrinking track, seconds left and Undo. Only while a hold or a send is in flight.
    pub(super) fn render_send_pill(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let hold = self.intent.hold.clone();
        if hold.is_none() && !matches!(self.draft.state, DraftState::Sending) {
            return None;
        }
        let now = self.now();
        let progress = hold
            .as_ref()
            .zip(now.as_ref())
            .map(|(hold, now)| hold_progress(hold, now.at()));
        // Past due the send is already going out, and Undo no longer recalls it.
        let waiting = progress.is_some_and(|p| !p.due);
        let scheduled = hold.as_ref().is_some_and(|h| h.is_scheduled());
        let title = match (&hold, &now) {
            (Some(h), Some(now)) if scheduled && waiting => {
                format!("Sends {}", format_time(h.hold_until, now))
            }
            _ => "Sending…".to_owned(),
        };
        let draft = self.draft.id;
        let mono = crate::app::ui::mono_font(cx);
        let t = cx.theme();
        let (field, border, strong, fg, muted, primary) = (
            t.secondary,
            t.border,
            t.foreground,
            t.secondary_foreground,
            t.muted_foreground,
            t.primary,
        );
        let mut pill = div()
            .id("compose-send-pill")
            .test_support()
            .flex()
            .flex_none()
            .items_center()
            .self_start()
            .gap(px(9.))
            .h(px(34.))
            .pl(px(11.))
            .pr(px(5.))
            .rounded(px(9.))
            .bg(field)
            .border_1()
            .border_color(border)
            .text_size(px(12.))
            .text_color(fg)
            .child(Icon::new(IconName::Send).with_size(px(15.)).text_color(primary))
            .child(div().font_weight(FontWeight::SEMIBOLD).text_color(strong).child(title));
        if let (Some(progress), false) = (progress.filter(|p| !p.due), scheduled) {
            pill = pill
                .child(
                    div()
                        .w(px(60.))
                        .h(px(3.))
                        .flex_none()
                        .rounded(px(2.))
                        .overflow_hidden()
                        .bg(border)
                        .child(
                            div()
                                .h_full()
                                .w(relative(progress.fraction))
                                .bg(primary),
                        ),
                )
                .child(
                    div()
                        .id("compose-hold-countdown")
                        .test_support()
                        .font_family(mono.clone())
                        .text_size(px(10.))
                        .text_color(muted)
                        .child(format!("{}s", progress.remaining.as_secs())),
                );
        }
        if waiting {
            pill = pill.child(
                div()
                    .id("compose-hold-undo")
                    .test_support()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .h(px(24.))
                    .px(px(8.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(primary)
                    .bg(primary.opacity(0.18))
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(strong)
                    .cursor_pointer()
                    .on_click(cx.listener(move |_, _, window, cx| {
                        window.dispatch_action(Box::new(UndoSendHold { draft }), cx)
                    }))
                    .child(Icon::new(IconName::Undo2).with_size(px(12.)))
                    .child("Undo")
                    .child(shortcut("cmd-z")),
            );
        }
        Some(div().px(px(14.)).pb(px(12.)).child(pill).into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::step_days;

    #[test]
    fn remind_days_step_through_the_list_and_stop_at_its_ends() {
        assert_eq!(step_days(3, 1), 5);
        assert_eq!(step_days(3, -1), 2);
        assert_eq!(step_days(1, -1), 1);
        assert_eq!(step_days(14, 1), 14);
    }
}
