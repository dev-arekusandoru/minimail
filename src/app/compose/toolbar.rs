//! The composer's compact Markdown toolbar: the everyday formatting inline, the rest one
//! menu away.
//!
//! Every button and every keyboard chord runs the same action, so what the button does to
//! the selection is exactly what the shortcut does — only the route to it differs.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::*;

/// One formatting control in the toolbar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tool {
    /// Element id and the test hook that finds it.
    pub id: &'static str,
    pub icon: IconName,
    pub label: &'static str,
    pub tip: &'static str,
    pub key: &'static str,
}

/// The everyday tools, shown inline. Order is display order.
pub const INLINE_TOOLS: [Tool; 4] = [
    Tool {
        id: "compose-bold",
        icon: IconName::Bold,
        label: "Bold",
        tip: "Bold",
        key: "cmd-b",
    },
    Tool {
        id: "compose-italic",
        icon: IconName::Italic,
        label: "Italic",
        tip: "Italic",
        key: "cmd-i",
    },
    Tool {
        id: "compose-link",
        icon: IconName::Link,
        label: "Link",
        tip: "Insert link",
        key: "cmd-k",
    },
    Tool {
        id: "compose-bullet-list",
        icon: IconName::List,
        label: "Bulleted list",
        tip: "Bulleted list",
        key: "cmd-shift-8",
    },
];

/// What the overflow menu holds, in display order.
pub const MORE_TOOLS: [Tool; 6] = [
    Tool {
        id: "compose-numbered-list",
        icon: IconName::ListOrdered,
        label: "Numbered list",
        tip: "Numbered list",
        key: "cmd-shift-7",
    },
    Tool {
        id: "compose-heading",
        icon: IconName::Heading,
        label: "Heading",
        tip: "Heading",
        key: "cmd-shift-h",
    },
    Tool {
        id: "compose-quote",
        icon: IconName::Quote,
        label: "Quote",
        tip: "Quote",
        key: "cmd-shift-.",
    },
    Tool {
        id: "compose-code",
        icon: IconName::Code,
        label: "Code block",
        tip: "Code block",
        key: "cmd-shift-c",
    },
    Tool {
        id: "compose-strike",
        icon: IconName::Strikethrough,
        label: "Strikethrough",
        tip: "Strikethrough",
        key: "cmd-shift-x",
    },
    Tool {
        id: "compose-table",
        icon: IconName::Table,
        label: "Table",
        tip: "Table",
        key: "cmd-shift-t",
    },
];

/// The overflow menu's tools, in display order. The menu lists the same tools the row
/// holds, so a format that is not inline can still be found by name.
pub fn more_tools() -> &'static [Tool] {
    &MORE_TOOLS
}

impl Tool {
    /// The toolbar button for this tool: an icon, a tooltip naming the chord, and the
    /// format's own action, so a click runs exactly what the chord runs.
    ///
    /// A tool with no action still renders, disabled: a control that cannot do anything is
    /// worse than one that says so.
    pub fn button(&self, disabled: bool, cx: &App) -> Button {
        let t = cx.theme();
        let button = crate::app::ui::key_tooltip(
            Button::new(self.id)
                .icon(self.icon)
                .ghost()
                .xsmall()
                .h(px(22.))
                .w(px(24.))
                .px_0()
                .text_color(t.foreground),
            self.tip,
            self.key,
        )
        .disabled(disabled);
        match super::tool_action(self.id) {
            // The action is built per click rather than cloned: it is a unit-ish action
            // with nothing to carry, and the keyboard path dispatches the same one.
            Some(action) => {
                button.on_click(move |_, window, cx| window.dispatch_action(action(), cx))
            }
            None => button,
        }
    }
}

/// The toolbar row: the inline tools, then the overflow trigger, then whatever the owner
/// puts on the right (the attachment button, preview toggle).
pub fn bar<'a>(
    tools: impl IntoIterator<Item = &'a Tool>,
    disabled: bool,
    overflow: AnyElement,
    trailing: Option<AnyElement>,
    cx: &App,
) -> impl IntoElement {
    let t = cx.theme();
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .h(px(26.))
        .px_2()
        .border_b(px(1.))
        .border_color(t.border)
        .children(
            tools
                .into_iter()
                .map(|tool| tool.button(disabled, cx).into_any_element()),
        )
        .child(overflow)
        .child(div().flex_1())
        .when_some(trailing, |d, el| d.child(el))
}
