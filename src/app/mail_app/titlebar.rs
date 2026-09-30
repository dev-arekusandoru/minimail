//! App-owned, Zed-style window titlebar.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::InteractiveElementExt as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{ListMode, MailApp, MenuKind};
use crate::app::actions::*;
use crate::app::ui::{button, run};

impl MailApp {
    pub(super) fn render_titlebar(&self, window: &mut Window, cx: &Context<Self>) -> AnyElement {
        let t = crate::theme::active(cx);
        let searching = matches!(self.mode, ListMode::Search(_));
        let title_fg = if window.is_window_active() { t.text } else { t.text_muted };
        let search_text = self.search_header().unwrap_or_else(|| "Search…".into());
        let moving = Rc::new(Cell::new(false));
        let view: SharedString = match &self.mode {
            ListMode::State => self.triage.view.label().into(),
            ListMode::Screener => "Screener".into(),
            ListMode::Search(_) => "Search".into(),
        };
        let search_tip: SharedString = "Search mail (/)".into();
        let search = div()
            .id("search-box")
            .test_support()
            .flex()
            .flex_1()
            .min_w(px(80.))
            .max_w(px(260.))
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
            .tooltip(move |window, cx| Tooltip::new(search_tip.clone()).build(window, cx))
            .child(search_text)
            .child(div().text_color(t.text_muted).child("/"))
            .on_click(run(OpenSearch));

        let width = f32::from(window.viewport_size().width);
        let titlebar = div()
            .id("mail-titlebar")
            .test_support()
            .flex()
            .flex_none()
            .items_center()
            .gap_3()
            .h(px(36.))
            .pl(px(if cfg!(target_os = "macos") && !window.is_fullscreen() { 80. } else { 12. }))
            .pr_3()
            .bg(t.sidebar)
            .border_b_1()
            .border_color(t.border)
            .child(drag_region(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_2()
                    .text_size(px(12.))
                    .text_color(title_fg)
                    .child("Mail")
                    .child(div().text_color(t.text_muted).child("·"))
                    .child(div().text_color(t.text_muted).child(view)),
                &moving,
            ))
            .child(drag_region(div().flex_1().h_full(), &moving))
            .child(search)
            .when(searching, |d| {
                d.child(button("search-clear", "×", "Leave search", "escape", cx).on_click(run(ClearSelection)))
            })
            .child(drag_region(div().flex_1().h_full(), &moving))
            .child(button("btn-palette", "Commands", "Command palette", "cmd-k", cx).on_click(run(ToggleCommandPalette)))
            .child(button("btn-undo", "Undo", "Undo", "u", cx).on_click(run(Undo)))
            .when(width >= 760., |d| {
                d.child(button("btn-settings", "Settings", "Settings", "cmd-,", cx).on_click(run(ToggleSettings)))
            })
            .when(width >= 690., |d| {
                d.child(button("btn-help", "?", "Keyboard shortcuts", "?", cx).on_click(run(ToggleHelp)))
            })
            .child(
                button("btn-layout", self.layout_glyph(), "Stack the panes the other way", "alt-l", cx)
                    .on_click(run(TogglePaneLayout)),
            )
            .when(width >= 800., |d| {
                d.child(button("btn-session", "Triage", "Start a triage session", "t", cx).on_click(run(StartSession)))
            })
            .when(width >= 670., |d| {
                d.child(
                    button("btn-more", "More ▾", "More actions", "", cx)
                        .when(self.menu_is(MenuKind::Global), |b| b.bg(t.selection))
                        .on_click(cx.listener(|this, _, window, cx| this.toggle_menu(MenuKind::Global, window, cx))),
                )
            });
        titlebar.into_any_element()
    }
}

/// Empty titlebar space that moves the window on drag and zooms it on double-click. Buttons and
/// the search box are siblings of these regions, so pressing them never starts a window move.
fn drag_region(region: Div, moving: &Rc<Cell<bool>>) -> Div {
    let (down, up, motion, out) = (moving.clone(), moving.clone(), moving.clone(), moving.clone());
    region
        .when(cfg!(target_os = "macos"), |d| d.window_control_area(WindowControlArea::Drag))
        .on_double_click(|_, window, _| {
            #[cfg(target_os = "macos")]
            window.titlebar_double_click();
            #[cfg(not(target_os = "macos"))]
            window.zoom_window();
        })
        .on_mouse_down_out(move |_, _, _| out.set(false))
        .on_mouse_down(MouseButton::Left, move |_, _, _| down.set(true))
        .on_mouse_up(MouseButton::Left, move |_, _, _| up.set(false))
        .on_mouse_move(move |_, window, _| {
            if motion.replace(false) {
                window.start_window_move();
            }
        })
}
