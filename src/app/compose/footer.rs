//! The composer's bottom edge: the status line (draft state, content warning, toolbar
//! toggle, ⋯ menu, Send), the `/` menu in the body and the one-time formatting hint.
//!
//! The formatting bar is a user preference, shared by every composer and kept across
//! restarts; it lives in [`FooterPrefs`], a global the owner fills from the settings store.

use std::ops::Range;
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, Selectable as _};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::{ComposeEvent, Composer, Placement};
use crate::app::actions::*;
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
    /// The `/` that opened the slash menu, while it is still the whole line.
    slash: Option<Range<usize>>,
    /// The highlighted slash row.
    slash_sel: usize,
    /// The first-run hint is on screen.
    hint: bool,
}

const SLASH_ROWS: usize = 2;

impl FooterState {
    pub fn new(cx: &mut App) -> Self {
        Self {
            slash: None,
            slash_sel: 0,
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

    /// Escape and Close dismiss the slash menu and the hint before they close anything.
    /// Returns whether the slash menu was dismissed.
    pub(super) fn dismiss_footer_overlays(&mut self, cx: &mut Context<Self>) -> bool {
        let was = self.footer.slash.take().is_some();
        self.retire_hint(cx);
        if was {
            cx.notify();
        }
        was
    }

    /// The hint goes away for good once dismissed.
    fn retire_hint(&mut self, cx: &mut App) {
        if std::mem::take(&mut self.footer.hint) {
            FooterPrefs::mark_hint_seen(cx);
        }
    }

    /// Re-evaluate the slash menu after the body changed.
    pub(super) fn update_slash(&mut self, cx: &mut Context<Self>) {
        let body = self.body.read(cx);
        let sel = body.selected_range();
        let open = if sel.is_empty() {
            crate::send_checks::slash_trigger(body.value().as_str(), sel.start)
        } else {
            None
        };
        if open != self.footer.slash {
            if self.footer.slash.is_none() {
                self.footer.slash_sel = 0;
            }
            self.footer.slash = open;
            cx.notify();
        }
    }

    /// Whether the slash menu is up, and so owns Up/Down/Enter.
    pub(super) fn slash_open(&self) -> bool {
        self.footer.slash.is_some() && !self.body_locked()
    }

    pub(super) fn slash_move(&mut self, delta: isize, cx: &mut Context<Self>) {
        let n = SLASH_ROWS as isize;
        self.footer.slash_sel = (self.footer.slash_sel as isize + delta).rem_euclid(n) as usize;
        cx.notify();
    }

    pub(super) fn pick_selected_slash(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pick_slash(self.footer.slash_sel, window, cx);
    }

    /// Choosing a slash row removes the `/` it was opened with, then runs the row.
    pub(super) fn pick_slash(&mut self, row: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(range) = self.footer.slash.take() else {
            return;
        };
        let mut text = self.body.read(cx).value().to_string();
        if text.get(range.clone()) == Some("/") {
            text.replace_range(range.clone(), "");
            self.apply(
                markdown::Edit { text, selection: range.start..range.start },
                window,
                cx,
            );
        }
        match row {
            0 => cx.emit(ComposeEvent::Attach),
            // Choosing the Send-later row only sets the intent; Send still sends.
            _ => self.open_menu(false, Some(2), window, cx),
        }
        cx.notify();
    }

    fn render_more_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let app = cx.weak_entity();
        let inline = self.placement == Placement::Inline;
        let collapsible = !inline && self.draft.source_message.is_some();
        let bar_on = FooterPrefs::toolbar(cx);
        let frozen = self.frozen();
        let danger = cx.theme().danger;
        crate::app::ui::key_tooltip(
            Button::new("compose-more")
                .icon(IconName::Ellipsis)
                .ghost()
                .h(px(26.))
                .w(px(26.))
                .px_0()
                .rounded(px(5.))
                .text_color(cx.theme().muted_foreground),
            "More",
            "",
        )
        .dropdown_menu_with_anchor(Anchor::BottomRight, move |menu, window, cx| {
            use crate::app::menu::MenuItem;
            let owner = app.clone();
            let run: crate::app::menu::MenuRun = Rc::new(move |action, window, cx| {
                owner.update(cx, |_, cx| window.dispatch_action(action, cx)).ok();
            });
            let mut rows = vec![MenuItem::header("Composer")];
            if inline {
                rows.push(
                    MenuItem::action_fn("Expand", || Box::new(ExpandComposer))
                        .icon(IconName::Maximize2),
                );
            }
            if collapsible {
                rows.push(
                    MenuItem::action_fn("Collapse", || Box::new(CollapseComposer))
                        .icon(IconName::Minimize2),
                );
            }
            rows.extend([
                MenuItem::action_fn("Preview as sent", || Box::new(ToggleComposePreview))
                    .icon(IconName::Eye),
                MenuItem::action_fn("Attach file", || Box::new(AddAttachment))
                    .icon(IconName::Paperclip),
                MenuItem::action_fn(
                    if bar_on { "Hide formatting bar" } else { "Show formatting bar" },
                    || Box::new(ToggleFormattingBar),
                )
                .icon(IconName::SlidersHorizontal),
                MenuItem::separator(),
            ]);
            let owner = app.clone();
            let menu = crate::app::menu::populate(menu, &rows, &run, window, cx);
            menu.item(
                PopupMenuItem::element(move |_, _| {
                    div().text_color(danger).child("Discard draft")
                })
                .icon(Icon::new(IconName::Trash).text_color(danger))
                .action(Box::new(DiscardDraft))
                .disabled(frozen)
                .on_click(move |_, window, cx| {
                    owner
                        .update(cx, |_, cx| window.dispatch_action(Box::new(DiscardDraft), cx))
                        .ok();
                }),
            )
        })
        .into_any_element()
    }

    /// The status line: draft state, then the controls.
    pub(super) fn render_footer(&mut self, cx: &mut Context<Self>) -> Div {
        // Colours are copied out: every button below needs the context mutably.
        let (border, warning, muted, success, primary) = {
            let t = cx.theme();
            (t.border, t.warning, t.muted_foreground, t.success, t.primary)
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
            .child(self.render_more_button(cx))
            // The hold is a fixed ten seconds with an Undo: while a message is held the
            // control offers nothing, so pressing Send again cannot send it early.
            .child(
                crate::app::ui::key_tooltip(
                    Button::new("compose-send")
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
                                    d.child(crate::app::ui::shortcut("cmd-enter"))
                                }),
                        ),
                    "Send; the message is held for ten seconds so it can be recalled",
                    "",
                )
                .disabled(!self.can_send() || frozen)
                .on_click(cx.listener(|this, _, window, cx| this.open_intent_menu(window, cx))),
            )
    }

    /// Overlays that float over the body: the slash menu and the first-run hint. Absolute,
    /// so nothing under them moves.
    pub(super) fn render_footer_overlays(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = Vec::new();
        let (border, popover, fg, muted, hover, primary) = {
            let t = cx.theme();
            (t.border, t.popover, t.popover_foreground, t.muted_foreground, t.secondary_hover, t.primary)
        };
        // Everything below the body: the status line and, when shown, the toolbar strip.
        let below = STATUS_H + if FooterPrefs::toolbar(cx) { 37. } else { 0. };
        if self.footer.slash.is_some() && !self.body_locked() {
            let sel = self.footer.slash_sel;
            let row = |id: &'static str, icon: IconName, label: &'static str, key: &'static str, on: bool| {
                div()
                    .id(id)
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
            out.push(
                div()
                    .id("compose-slash-menu")
                    .test_support()
                    .absolute()
                    .left(px(14.))
                    .bottom(px(below + 6.))
                    .w(px(250.))
                    .p_1()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(border)
                    .bg(popover)
                    .text_color(fg)
                    .shadow_lg()
                    .child(
                        div()
                            .px_2()
                            .pt_1()
                            .text_size(px(9.))
                            .text_color(muted)
                            .child("INSERT"),
                    )
                    .child(
                        row("compose-slash-attach", IconName::Paperclip, "Attach file", "cmd-shift-a", sel == 0)
                            .on_click(cx.listener(|this, _, window, cx| this.pick_slash(0, window, cx))),
                    )
                    .child(
                        row("compose-slash-later", IconName::Clock, "Send later", "", sel == 1)
                            .on_click(cx.listener(|this, _, window, cx| this.pick_slash(1, window, cx))),
                    )
                    .into_any_element(),
            );
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
                    .child("Formatting is hidden. Click the sliders (or ⌘⌥T) to show it. Type / in the body for attach and send later.")
                    .into_any_element(),
            );
        }
        out
    }
}
