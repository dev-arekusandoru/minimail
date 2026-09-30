//! App-owned, Zed-style window titlebar.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{ListMode, MailApp, MenuKind};
use crate::app::actions::*;
use crate::app::ui::{icon_button, primary_button, run, shortcut};

impl MailApp {
    pub(super) fn render_titlebar(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = crate::theme::active(cx);
        let searching = matches!(self.mode, ListMode::Search(_));
        let title_fg = if window.is_window_active() { t.text } else { t.text_muted };
        let search_text = self.search_header().unwrap_or_else(|| "Search…".into());
        let moving = window.use_keyed_state("titlebar-moving", cx, |_, _| Rc::new(Cell::new(false))).read(cx).clone();
        let view: SharedString = match &self.mode {
            ListMode::State => self.location_label().into(),
            ListMode::Search(_) => "Search".into(),
        };
        let search_tip: SharedString = "Search mail".into();
        let search = div()
            .id("search-box")
            .test_support()
            .flex()
            .flex_none()
            .w(px(260.))
            .max_w_full()
            .min_w(px(80.))
            .items_center()
            .justify_between()
            .h(px(22.))
            .px_2()
            .rounded_sm()
            .border_1()
            .border_color(t.border)
            .bg(t.surface)
            .text_size(px(11.))
            .text_color(if searching { t.text } else { t.text_muted })
            .cursor_pointer()
            .tooltip(move |window, cx| {
                Tooltip::new(search_tip.clone())
                    .key_binding(Some(shortcut("/")))
                    .build(window, cx)
            })
            .child(search_text)
            .child(if searching {
                icon_button("search-clear", IconName::Close, "Clear search", "escape", cx)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(|event, window, cx| {
                        run(ClearSelection)(event, window, cx);
                        cx.stop_propagation();
                    })
                    .into_any_element()
            } else {
                div().flex_none().text_color(t.text_muted).child("/").into_any_element()
            })
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(run(OpenSearch));

        let width = f32::from(window.viewport_size().width);
        let left = div().flex().flex_1().flex_basis(px(0.)).min_w_0().overflow_hidden().h_full().child(
            div()
                .flex()
                .items_center()
                .min_w_0()
                .h_full()
                .pl(px(if cfg!(target_os = "macos") && !window.is_fullscreen() { 80. } else { 12. }))
                .pr_3()
                .text_size(px(12.))
                .text_color(title_fg)
                .child(div().min_w_0().overflow_hidden().text_ellipsis().whitespace_nowrap().child(view)),
        );
        let right_buttons = div()
            .flex()
            .flex_none()
            .items_center()
            .gap_3()
            .h_full()
            .pl_3()
            .pr_3()
            .child(no_drag(
                primary_button(
                    "btn-session",
                    IconName::Play,
                    if width >= 800. { "Triage" } else { "" },
                    "Start a triage session",
                    "t",
                    cx,
                )
                .on_click(run(StartSession)),
            ))
            .child(no_drag(
                icon_button("btn-layout", self.layout_icon(), "Stack the panes the other way", "alt-l", cx)
                    .on_click(run(TogglePaneLayout)),
            ))
            .child(no_drag(icon_button("tb-settings", IconName::Settings, "Settings", "cmd-,", cx).on_click(run(ToggleSettings))))
            .child(
                no_drag(
                    icon_button("btn-more", IconName::Ellipsis, "More actions", "", cx)
                        .when(self.menu_is(MenuKind::Global), |b| b.bg(t.selection))
                        .on_click(cx.listener(|this, _, window, cx| this.toggle_menu(MenuKind::Global, window, cx))),
                )
                .relative()
                .child(self.anchor_probe(MenuKind::Global)),
            );
        let right = div()
            .flex()
            .flex_1()
            .flex_basis(px(0.))
            .min_w_0()
            .overflow_hidden()
            .justify_end()
            .h_full()
            .child(right_buttons);
        // Left and right regions share the leftover width equally (flex-basis 0), so the fixed-size
        // search box between them is always centered in the bar.
        let titlebar = div()
            .id("mail-titlebar")
            .test_support()
            .flex()
            .flex_none()
            .items_center()
            .h(px(36.))
            .bg(t.sidebar)
            .border_b_1()
            .border_color(t.border)
            .child(left)
            .child(search)
            .child(right);
        drag_region(titlebar, &moving).into_any_element()
    }
}

/// Makes the whole bar move the window on drag and zoom it on double-click, like gpui-component's
/// `TitleBar`. Interactive children must call [`no_drag`] so pressing them never starts a move.
/// `moving` must outlive a single render: a re-render between mouse-down and the first mouse-move
/// (hover, tooltip, tick) would otherwise drop the pending-drag flag.
fn drag_region<E: InteractiveElement>(bar: E, moving: &Rc<Cell<bool>>) -> E {
    let (down, up, motion, out) = (moving.clone(), moving.clone(), moving.clone(), moving.clone());
    bar.on_mouse_down_out(move |_, _, _| out.set(false))
        .on_mouse_down(MouseButton::Left, move |event, window, _| {
            if event.click_count == 2 {
                #[cfg(target_os = "macos")]
                window.titlebar_double_click();
                #[cfg(not(target_os = "macos"))]
                window.zoom_window();
                return;
            }
            down.set(true);
        })
        .on_mouse_up(MouseButton::Left, move |_, _, _| up.set(false))
        .on_mouse_move(move |_, window, _| {
            if motion.replace(false) {
                window.start_window_move();
            }
        })
}

/// Wraps an interactive titlebar child so presses on it are not treated as a bar drag.
fn no_drag(child: impl IntoElement) -> Div {
    div().flex_none().on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).child(child)
}
