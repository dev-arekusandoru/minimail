//! The composer's bottom edge: the status line (draft state, content warning, toolbar
//! toggle, Send), the `/` menu under the body's caret and the one-time formatting hint.
//!
//! The formatting bar is a user preference, shared by every composer and kept across
//! restarts; it lives in [`FooterPrefs`], a global the owner fills from the settings store.

use std::ops::Range;
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::base::ElementExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::separator::Separator;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, Selectable as _};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::toolbar;
use super::{ComposeEvent, Composer};
use crate::outgoing::markdown;
use crate::outgoing::DraftState;

/// The user's composer preferences, readable without the settings store (tests and a first
/// frame before preferences load see the defaults: bar hidden, hint unseen).
#[derive(Default)]
pub struct FooterPrefs {
    toolbar: bool,
    hint_seen: bool,
    store: Option<Rc<crate::contacts::ContactStore>>,
}

impl Global for FooterPrefs {}

impl FooterPrefs {
    /// Load the stored choices and keep `store` for writing them back.
    pub fn install(store: Rc<crate::contacts::ContactStore>, cx: &mut App) {
        use crate::app_settings::{COMPOSE_FORMATTING_BAR, COMPOSE_FORMATTING_HINT_SEEN};
        use crate::prefs::Scope;
        cx.set_global(Self {
            toolbar: COMPOSE_FORMATTING_BAR.get(store.as_ref(), &Scope::Global),
            hint_seen: COMPOSE_FORMATTING_HINT_SEEN.get(store.as_ref(), &Scope::Global),
            store: Some(store),
        });
    }

    fn toolbar(cx: &App) -> bool {
        cx.try_global::<Self>().is_some_and(|p| p.toolbar)
    }

    fn set_toolbar(on: bool, cx: &mut App) {
        let prefs = cx.default_global::<Self>();
        prefs.toolbar = on;
        if let Some(store) = &prefs.store {
            crate::app_settings::COMPOSE_FORMATTING_BAR.set(
                store.as_ref(),
                &crate::prefs::Scope::Global,
                on,
            );
        }
    }

    fn hint_seen(cx: &App) -> bool {
        cx.try_global::<Self>().is_some_and(|p| p.hint_seen)
    }

    /// The hint is shown until it is dismissed once; then it never returns.
    fn mark_hint_seen(cx: &mut App) {
        let prefs = cx.default_global::<Self>();
        prefs.hint_seen = true;
        if let Some(store) = &prefs.store {
            crate::app_settings::COMPOSE_FORMATTING_HINT_SEEN.set(
                store.as_ref(),
                &crate::prefs::Scope::Global,
                true,
            );
        }
    }
}

/// Per-composer footer state.
pub struct FooterState {
    /// The `/query` that opened the slash menu, and the bar commands it still matches.
    slash: Option<(Range<usize>, Vec<&'static toolbar::Tool>)>,
    /// The highlighted slash row.
    slash_sel: usize,
    /// Where the menu opened: the `/`'s line box, kept so a frame without layout can't move it.
    slash_at: Option<Point<Pixels>>,
    /// The first-run hint is on screen.
    hint: bool,
}

impl FooterState {
    pub fn new(cx: &mut App) -> Self {
        Self {
            slash: None,
            slash_sel: 0,
            slash_at: None,
            hint: !FooterPrefs::toolbar(cx) && !FooterPrefs::hint_seen(cx),
        }
    }
}

const STATUS_H: f32 = 38.;

impl Composer {
    /// Whether the formatting bar is shown.
    pub fn toolbar_visible(&self, cx: &App) -> bool {
        FooterPrefs::toolbar(cx)
    }

    /// A reason to hesitate before sending, if the draft has one. Send reads "Send anyway"
    /// while this is `Some`.
    pub fn content_warning(&self) -> Option<SharedString> {
        (self.draft.attachments.is_empty()
            && crate::send_checks::mentions_attachment(&self.draft.body.source))
        .then(|| "Attachment mentioned, none attached".into())
    }

    pub(super) fn toggle_toolbar(&mut self, cx: &mut Context<Self>) {
        FooterPrefs::set_toolbar(!FooterPrefs::toolbar(cx), cx);
        self.retire_hint(cx);
        cx.notify();
    }

    /// Escape dismisses the slash menu and the hint; it never closes the composer.
    pub(super) fn dismiss_footer_overlays(&mut self, cx: &mut Context<Self>) {
        if self.footer.slash.take().is_some() {
            cx.notify();
        }
        self.retire_hint(cx);
    }

    /// The hint goes away for good once dismissed.
    fn retire_hint(&mut self, cx: &mut App) {
        if std::mem::take(&mut self.footer.hint) {
            FooterPrefs::mark_hint_seen(cx);
        }
    }

    /// Re-evaluate the slash menu after the body changed: the `/query` under the caret
    /// decides both whether it opens and which rows it keeps.
    pub(super) fn update_slash(&mut self, cx: &mut Context<Self>) {
        let body = self.body.read(cx);
        let sel = body.selected_range();
        let open = if sel.is_empty() {
            crate::send_checks::slash_trigger(body.value().as_str(), sel.start).and_then(|range| {
                let query = &body.value()[range.start + 1..range.end];
                let mut ranked = toolbar::commands()
                    .filter_map(|tool| {
                        crate::fuzzy::rank(tool.label, tool.key, query).map(|rank| (rank, tool))
                    })
                    .collect::<Vec<_>>();
                ranked.sort_by_key(|(rank, _)| *rank);
                let rows = ranked.into_iter().map(|(_, tool)| tool).collect::<Vec<_>>();
                (!rows.is_empty()).then_some((range, rows))
            })
        } else {
            None
        };
        let changed =
            open.as_ref().map(|(range, _)| range) != self.footer.slash.as_ref().map(|(range, _)| range);
        if self.footer.slash.is_none() {
            self.footer.slash_sel = 0;
            self.footer.slash_at = None;
        }
        if let Some((_, rows)) = &open {
            self.footer.slash_sel = self.footer.slash_sel.min(rows.len() - 1);
        }
        self.footer.slash = open;
        if changed {
            cx.notify();
        }
    }

    /// Whether the slash menu is up, and so owns Up/Down/Enter.
    pub(super) fn slash_open(&self) -> bool {
        self.footer.slash.is_some() && !self.body_locked()
    }

    pub(super) fn slash_move(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some((_, rows)) = &self.footer.slash else {
            return;
        };
        let n = rows.len() as isize;
        self.footer.slash_sel = (self.footer.slash_sel as isize + delta).rem_euclid(n) as usize;
        cx.notify();
    }

    pub(super) fn pick_selected_slash(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pick_slash(self.footer.slash_sel, window, cx);
    }

    /// Choosing a slash row removes the `/query` it was opened with, then runs that row's
    /// bar command, so the menu does exactly what its button does.
    pub(super) fn pick_slash(&mut self, row: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some((range, rows)) = self.footer.slash.take() else {
            return;
        };
        let mut text = self.body.read(cx).value().to_string();
        if text.get(range.clone()).is_some_and(|span| span.starts_with('/')) {
            text.replace_range(range.clone(), "");
            self.apply(
                markdown::Edit { text, selection: range.start..range.start },
                window,
                cx,
            );
        }
        if let Some(action) = rows.get(row).and_then(|tool| super::tool_action(tool.id)) {
            window.dispatch_action(action(), cx);
        }
        cx.notify();
    }

    /// The status line: draft state, then the controls.
    pub(super) fn render_footer(&mut self, cx: &mut Context<Self>) -> Div {
        // Colours are copied out: every button below needs the context mutably.
        let (border, warning, muted, success, primary, on_primary) = {
            let t = cx.theme();
            (
                t.border,
                t.warning,
                t.muted_foreground,
                t.success,
                t.primary,
                t.primary_foreground,
            )
        };
        let status = self.status();
        let frozen = self.frozen();
        let conflicted = matches!(self.draft.state, DraftState::Conflict);
        let troubled = matches!(
            self.draft.state,
            DraftState::Failed { .. } | DraftState::Uncertain | DraftState::Conflict
        );
        let content_warning = self.content_warning().filter(|_| self.can_send());
        let bar_on = FooterPrefs::toolbar(cx);
        let dot = if troubled || content_warning.is_some() {
            warning
        } else if ["Saving", "Unsaved", "Sending", "Scheduled"].iter().any(|s| status.starts_with(s)) {
            primary
        } else {
            success
        };
        let send_label = if content_warning.is_some() { "Send anyway" } else { "Send" };
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .h(px(STATUS_H))
            .pl(px(14.))
            .pr(px(8.))
            .border_t(px(1.))
            .border_color(border)
            .text_size(px(12.))
            .child(div().size(px(6.)).flex_none().rounded_full().bg(dot))
            .child(match content_warning {
                Some(text) => div()
                    .id("compose-warning")
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .text_color(warning)
                    .child(Icon::new(IconName::TriangleAlert).size(px(12.)))
                    .child(text),
                None => div()
                    .id("compose-status")
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .text_color(if troubled { warning } else { muted })
                    .child(status),
            })
            // A conflicted draft is not left in a dead end: the choice is right here, and
            // the local snapshot is kept until it is made.
            .when(conflicted, |d| {
                d.child(
                    crate::app::ui::button(
                        "compose-resolve-conflict",
                        "Resolve…",
                        "Choose between your copy, the server's, or keeping both",
                        "",
                        cx,
                    )
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(ComposeEvent::ResolveConflict))),
                )
            })
            .when(matches!(self.draft.state, DraftState::Uncertain), |d| {
                d.child(
                    crate::app::ui::button(
                        "compose-check-delivery",
                        "Check delivery",
                        "Look in Sent without sending again",
                        "",
                        cx,
                    )
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(ComposeEvent::CheckDelivery))),
                )
            })
            .child(
                crate::app::ui::key_tooltip(
                    Button::new("compose-format-toggle")
                        .icon(IconName::SlidersHorizontal)
                        .ghost()
                        .h(px(26.))
                        .w(px(26.))
                        .px_0()
                        .rounded(px(5.))
                        .text_color(if bar_on { cx.theme().foreground } else { muted })
                        .selected(bar_on)
                        .toggled(bar_on),
                    if bar_on { "Hide formatting bar" } else { "Show formatting bar" },
                    "cmd-alt-t",
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_toolbar(cx))),
            )
            // The hold is a fixed ten seconds with an Undo: while a message is held the
            // control offers nothing, so pressing Send again cannot send it early.
            .child({
                // Where the intent menu opened from Send hangs from.
                let anchor = self.send_anchor();
                crate::app::ui::key_tooltip(
                    Button::new("compose-send")
                        .debug_selector(|| "compose-send".into())
                        .primary()
                        .h(px(28.))
                        .px(px(11.))
                        .rounded(px(6.))
                        .text_size(px(12.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(7.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(
                                    div()
                                        .id(if send_label != "Send" {
                                            "compose-send-anyway"
                                        } else {
                                            "compose-send-label"
                                        })
                                        .test_support()
                                        .child(send_label),
                                )
                                .when(send_label == "Send", |d| {
                                    // On the primary fill the keycap is plain text at reduced
                                    // contrast: a bordered box of muted grey muddies the blue.
                                    d.child(
                                        crate::app::ui::shortcut("cmd-enter")
                                            .text_color(on_primary.opacity(0.7))
                                            .bg(gpui_kit::transparent_black()),
                                    )
                                }),
                        ),
                    "Send; the message is held for ten seconds so it can be recalled",
                    "",
                )
                .disabled(!self.can_send() || frozen)
                .on_prepaint(move |bounds, _, _| anchor.set(Some(bounds)))
                .on_click(cx.listener(|this, _, window, cx| this.open_intent_menu(window, cx)))
            })
    }

    /// Under the `/` that opened the menu: fixed while the query grows, and kept once laid out.
    fn slash_anchor(&mut self, cx: &App) -> Option<Point<Pixels>> {
        let at = self.footer.slash.as_ref()?.0.start;
        if self.footer.slash_at.is_none() {
            self.footer.slash_at = self
                .body
                .read(cx)
                .range_to_bounds(&(at..at))
                .map(|caret| point(caret.origin.x, caret.bottom() + px(4.)));
        }
        self.footer.slash_at
    }

    /// Overlays that float over the body: the slash menu and the first-run hint.
    pub(super) fn render_footer_overlays(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = Vec::new();
        let (border, popover, fg, muted, hover, primary) = {
            let t = cx.theme();
            (t.border, t.popover, t.popover_foreground, t.muted_foreground, t.secondary_hover, t.primary)
        };
        // Everything below the body: the status line and, when shown, the toolbar strip. Only
        // the fallback placement needs it, for a body whose caret is not laid out yet.
        let below = STATUS_H + if FooterPrefs::toolbar(cx) { 37. } else { 0. };
        if self.footer.slash.is_some() && !self.body_locked() {
            let sel = self.footer.slash_sel;
            let row = |id: ElementId,
                       icon: IconName,
                       label: &'static str,
                       key: &'static str,
                       on: bool| {
                div()
                    .id(id)
                    .test_support()
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(px(28.))
                    .px_2()
                    .rounded(px(5.))
                    .text_size(px(12.))
                    .cursor_pointer()
                    .when(on, |d| d.bg(hover))
                    .hover(move |s| s.bg(hover))
                    .child(Icon::new(icon).size(px(14.)).text_color(muted))
                    .child(label)
                    .when(!key.is_empty(), |d| {
                        d.child(
                            div()
                                .ml_auto()
                                .child(crate::app::ui::shortcut(key)),
                        )
                    })
            };
            // The bar's own commands, so the menu never drifts from what the toolbar offers.
            let mut menu = div()
                .id("compose-slash-menu")
                .test_support()
                .w(px(250.))
                .p_1()
                .rounded(px(8.))
                .border_1()
                .border_color(border)
                .bg(popover)
                .text_color(fg)
                .shadow_lg();
            let mut group = None;
            let rows: &[&'static toolbar::Tool] =
                self.footer.slash.as_ref().map_or(&[], |(_, rows)| rows);
            for (ix, tool) in rows.iter().enumerate() {
                if group.is_some_and(|group| group != tool.group) {
                    menu = menu.child(Separator::horizontal().my(px(4.)));
                }
                group = Some(tool.group);
                menu = menu.child(
                    row(("compose-slash", ix).into(), tool.icon, tool.label, tool.key, sel == ix)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.pick_slash(ix, window, cx)
                        })),
                );
            }
            menu = menu.max_h(px(320.)).overflow_y_scroll();
            // Just under the `/`; anchored flips it above when the body ends too near the
            // window edge. Before the first layout there is no caret to anchor to.
            out.push(match self.slash_anchor(cx) {
                Some(at) => anchored()
                    .anchor(Anchor::TopLeft)
                    .position(at)
                    .snap_to_window_with_margin(px(8.))
                    .child(menu)
                    .into_any_element(),
                None => menu
                    .absolute()
                    .left(px(14.))
                    .bottom(px(below + 6.))
                    .into_any_element(),
            });
        }
        if self.footer.hint && !FooterPrefs::toolbar(cx) {
            out.push(
                div()
                    .id("compose-format-hint")
                    .test_support()
                    .absolute()
                    .right(px(120.))
                    .bottom(px(STATUS_H + 2.))
                    .w(px(272.))
                    .px(px(11.))
                    .py(px(9.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(primary)
                    .bg(popover)
                    .text_color(fg)
                    .text_size(px(12.))
                    .line_height(px(17.))
                    .shadow_lg()
                    .child("Formatting is hidden. Click the sliders (or ⌘⌥T) to show it. Type / in the body to run any formatting command or attach a file.")
                    .into_any_element(),
            );
        }
        out
    }
}
