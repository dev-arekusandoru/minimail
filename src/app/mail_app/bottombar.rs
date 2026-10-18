//! The bottom bar's tool buttons: the sidebar, palette and global commands, Settings, the pane
//! layout and the shortcut sheet. The hint bar pins them to its edges (see `HintBar::leading`).
use gpui_kit::component::{ActiveTheme as _, Selectable as _, button::{Button, ButtonVariants as _}};
use gpui_kit::assets::IconName;
use gpui_kit::*;

use super::MailApp;
use crate::app::actions::*;
use crate::app::ui::{icon_button, run};

/// Footprint of one tool button and the gap between items in a cluster, px.
const TOOL_W: f32 = 24.;
const GAP: f32 = 4.;

/// Width of a cluster of `buttons` tool buttons with `dividers` hairlines between its groups.
fn cluster_width(buttons: usize, dividers: usize) -> f32 {
    let items = buttons + dividers;
    buttons as f32 * TOOL_W + items.saturating_sub(1) as f32 * GAP + dividers as f32
}

/// One icon button with its tooltip and keycap, lit while `active`.
fn tool<A: Action + Clone>(
    id: &'static str,
    icon: IconName,
    tip: &str,
    key: &str,
    active: bool,
    action: A,
    cx: &App,
) -> Button {
    icon_button(id, icon, tip, key, cx).ghost().selected(active).on_click(run(action))
}

/// A hairline between two groups of tool buttons.
fn divider(color: Hsla) -> Div {
    div().w(px(1.)).h(px(14.)).bg(color)
}

impl MailApp {
    /// The left cluster and its width: the sidebar toggle and palette, then the list commands
    /// (Undo, Classify). Sender rules and Settings live on the sidebar rail.
    pub(super) fn bottom_leading(&self, cx: &App) -> (AnyElement, f32) {
        let border = cx.theme().border;
        let sidebar = self.sidebar_visible();
        let tools = div()
            .flex()
            .items_center()
            .gap_1()
            .child(tool(
                "btn-sidebar",
                if sidebar { IconName::PanelLeftClose } else { IconName::PanelLeftOpen },
                "Toggle the sidebar",
                "cmd-b",
                sidebar,
                ToggleSidebar,
                cx,
            ))
            .child(tool("btn-commands", IconName::Command, "Command palette", "cmd-k", self.palette_open(), ToggleCommandPalette, cx))
            .child(divider(border))
            .child(tool("btn-undo", IconName::Undo2, "Undo", "u", false, Undo, cx))
            .child(tool(
                "btn-classify-list",
                IconName::Sparkles,
                "Classify every message in this list",
                "shift-c",
                false,
                ClassifyVisible,
                cx,
            ));
        (tools.into_any_element(), cluster_width(4, 1))
    }

    /// The right cluster and its width, after the status text: the pane layout and the shortcut
    /// sheet.
    pub(super) fn bottom_trailing(&self, cx: &App) -> (AnyElement, f32) {
        let tools = div()
            .flex()
            .items_center()
            .gap_1()
            .child(tool(
                "btn-layout",
                self.layout_icon(),
                "Stack the panes the other way",
                "alt-l",
                false,
                TogglePaneLayout,
                cx,
            ))
            .child(tool("btn-help", IconName::Keyboard, "Keyboard shortcuts", "?", self.help_open(), ToggleHelp, cx));
        (tools.into_any_element(), cluster_width(2, 0))
    }
}
