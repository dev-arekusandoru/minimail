//! Mouse entry points for row clicks, left-edge selection, suggestion badges and the
//! popup menu layer.
use gpui_kit::component::ActiveTheme as _;

use gpui_kit::*;

use super::{HEADER_H, ListMode, MENU_W, MailApp, PaneLayout};

impl MailApp {
    /// Move the cursor to visible row `ix` (either list mode), clamped.
    pub(super) fn cursor_to(&mut self, ix: usize) {
        let delta = ix as isize - self.cursor_ix() as isize;
        self.move_cursor(delta);
    }

    /// Click on row `ix`: plain selects it and opens it, shift extends the selection from the
    /// cursor (like `shift-j`), cmd toggles it (like `x`). Double-click opens like `enter`.
    pub(super) fn click_row(
        &mut self,
        ix: usize,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mods = event.modifiers();
        let selectable = self.mode == ListMode::State && !self.in_session();
        if selectable && mods.shift {
            let delta = ix as isize - self.cursor_ix() as isize;
            self.extend_by(delta);
        } else if selectable && mods.platform {
            self.cursor_to(ix);
            self.toggle_select_cursor();
        } else {
            self.triage.clear_selection();
            self.cursor_to(ix);
            if let Some(id) = self.cursor_id() {
                self.open_message(id, event.click_count() >= 2);
            }
        }
        self.scroll_to_cursor();
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// Click on the row's left edge: like moving there and pressing `x`.
    pub(super) fn toggle_row(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.cursor_to(ix);
        if self.mode == ListMode::State && !self.in_session() {
            self.toggle_select_cursor();
        }
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// `y`: accept the AI suggestions pending on the message under the cursor.
    pub(super) fn accept_suggestions(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.cursor_id() {
            let now = self.now();
            self.mailbox.accept_suggestions(id, now);
            cx.notify();
        }
    }

    /// `n`: reject the AI suggestions pending on the message under the cursor.
    pub(super) fn reject_suggestions(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.cursor_id() {
            self.mailbox.reject_suggestions(id);
            cx.notify();
        }
    }

    /// Click (`accept`) or right-click (reject) on the badges of row `ix`.
    pub(super) fn suggestions_at(
        &mut self,
        ix: usize,
        accept: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cursor_to(ix);
        if accept {
            self.accept_suggestions(cx);
        } else {
            self.reject_suggestions(cx);
        }
        window.focus(&self.focus_handle, cx);
    }

    /// Full-height selection hit target at the row's left edge.
    pub(super) fn row_selection_target(&self, ix: usize, cx: &Context<Self>) -> crate::app::ui::Observable {
        let t = cx.theme();
        let hover = t.primary.opacity(0.2);
        div()
            .id(("row-select", ix))
            .absolute()
            .left_0()
            .top_0()
            .bottom_0()
            .w(px(10.))
            .cursor_pointer()
            .hover(move |d| d.bg(hover))
            .tooltip(|window, cx| {
                gpui_kit::component::tooltip::Tooltip::new(
                    "Select (click) · cmd-click / shift-click on row",
                )
                .build(window, cx)
            })
            .test_support()
            .on_click(cx.listener(move |this, _, window, cx| this.toggle_row(ix, window, cx)))
    }


    /// The popup menu layer: a backdrop that dismisses on a click, plus the open panel
    /// hanging under the trigger that opened it, right-aligned to it and kept inside the
    /// window.
    pub(super) fn render_menu(&self, window: &Window, cx: &Context<Self>) -> Option<AnyElement> {
        const MARGIN: f32 = 8.;
        let panel = self.menu_panel()?;
        let open = self.menu.as_ref()?;
        let viewport = window.viewport_size();
        let (vw, vh) = (f32::from(viewport.width), f32::from(viewport.height));
        let (left, top) = match self.anchors.borrow().get(&open.kind) {
            Some(b) => (
                f32::from(b.right()) - MENU_W,
                f32::from(b.bottom()) + 4.,
            ),
            None => (vw - MENU_W - 12., HEADER_H),
        };
        let left = left.clamp(MARGIN, (vw - MENU_W - MARGIN).max(MARGIN));
        let top = top.min(vh - open.height - MARGIN).max(MARGIN);
        Some(
            div()
                .id("menu-backdrop")
                .test_support()
                .absolute()
                .inset_0()
                .occlude()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| this.close_menu(window, cx)),
                )
                .child(
                    div()
                        .absolute()
                        .top(px(top))
                        .left(px(left))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(panel),
                )
                .into_any_element(),
        )
    }

    /// Toolbar icon for the pane layout button: the current orientation.
    pub(super) fn layout_icon(&self) -> gpui_kit::assets::IconName {
        match self.panes.orientation() {
            PaneLayout::SideBySide => gpui_kit::assets::IconName::PanelLeft,
            PaneLayout::Stacked => gpui_kit::assets::IconName::PanelBottom,
        }
    }
}
