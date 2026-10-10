//! The composer's Markdown toolbar: every format inline while the row has room, the ones
//! that no longer fit one menu away.
//!
//! Every button and every keyboard chord runs the same action, so what the button does to
//! the selection is exactly what the shortcut does — only the route to it differs. The bar
//! is one rounded, bordered container spanning the composer's width, with a filled active
//! state for a format already applied at the caret.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::separator::Separator;
use gpui_kit::component::toolbar::Toolbar;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Selectable as _};
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
    /// Tools sharing a group render next to one another; a thin separator sits between
    /// groups.
    pub group: u8,
}

/// Every format, in display order, grouped so a vertical separator falls between kinds of
/// formatting. Nothing is put behind the overflow while the row can hold it.
pub const TOOLS: [Tool; 10] = [
    Tool {
        id: "compose-bold",
        icon: IconName::Bold,
        label: "Bold",
        tip: "Bold",
        key: "cmd-b",
        group: 0,
    },
    Tool {
        id: "compose-italic",
        icon: IconName::Italic,
        label: "Italic",
        tip: "Italic",
        key: "cmd-i",
        group: 0,
    },
    Tool {
        id: "compose-strike",
        icon: IconName::Strikethrough,
        label: "Strikethrough",
        tip: "Strikethrough",
        key: "cmd-shift-x",
        group: 0,
    },
    Tool {
        id: "compose-heading",
        icon: IconName::Heading,
        label: "Heading",
        tip: "Heading",
        key: "cmd-shift-h",
        group: 1,
    },
    Tool {
        id: "compose-quote",
        icon: IconName::Quote,
        label: "Quote",
        tip: "Quote",
        key: "cmd-shift-.",
        group: 1,
    },
    Tool {
        id: "compose-code",
        icon: IconName::Code,
        label: "Code block",
        tip: "Code block",
        key: "cmd-shift-c",
        group: 1,
    },
    Tool {
        id: "compose-bullet-list",
        icon: IconName::List,
        label: "Bulleted list",
        tip: "Bulleted list",
        key: "cmd-shift-8",
        group: 2,
    },
    Tool {
        id: "compose-numbered-list",
        icon: IconName::ListOrdered,
        label: "Numbered list",
        tip: "Numbered list",
        key: "cmd-shift-7",
        group: 2,
    },
    Tool {
        id: "compose-link",
        icon: IconName::Link,
        label: "Link",
        tip: "Insert link",
        key: "cmd-k",
        group: 3,
    },
    Tool {
        id: "compose-table",
        icon: IconName::Table,
        label: "Table",
        tip: "Table",
        key: "cmd-shift-t",
        group: 3,
    },
];

/// A tool button is 36 wide with an 8px gap after it.
const TOOL_SLOT: f32 = 44.;
/// The container's padding and the separators between groups.
const BAR_PADDING: f32 = 48.;
/// What the right-hand controls (Preview and Attach) take, plus the spacer and gaps.
const TRAILING_WIDTH: f32 = 128.;

/// How many tools fit in `width` pixels before the overflow trigger is needed.
///
/// When not all of them fit, one more slot is given up to the trigger itself, so the count
/// is one short of the raw fit. A width of zero (not laid out yet) keeps every tool.
pub fn visible_tools(width: f32, total: usize) -> usize {
    if width <= 0. {
        return total;
    }
    let room = width - BAR_PADDING - TRAILING_WIDTH;
    if room <= 0. {
        return 0;
    }
    let fit = ((room + 4.) / TOOL_SLOT).floor() as usize;
    if fit >= total { total } else { fit.saturating_sub(1) }
}

impl Tool {
    /// The toolbar button for this tool: an icon, a tooltip naming the chord, and the
    /// format's own action, so a click runs exactly what the chord runs.
    ///
    /// `active` fills the button for a format already applied at the caret, which is what
    /// the toggle would turn off.
    ///
    /// A tool with no action still renders, disabled: a control that cannot do anything is
    /// worse than one that says so.
    pub fn button(&self, disabled: bool, active: bool, cx: &App) -> Button {
        let t = cx.theme();
        let button = crate::app::ui::key_tooltip(
            Button::new(self.id)
                .icon(self.icon)
                .ghost()
                .h(px(36.))
                .w(px(36.))
                .px_0()
                .rounded_md()
                .text_color(t.foreground)
                .selected(active)
                .toggled(active),
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

/// A thin divider between toolbar groups.
fn separator() -> AnyElement {
    Separator::vertical().h(px(20.)).into_any_element()
}

/// The toolbar row: the format tools that fit, an overflow trigger when some do not, a
/// spacer, then the trailing controls (Preview, Attach).
///
/// It spans the composer's full width as one rounded, bordered container and carries the
/// kit's [`Toolbar`] semantics, so the arrow keys move along it and it reads as one bar.
pub fn bar(
    visible: usize,
    disabled: bool,
    active: &[&'static str],
    overflow: Option<AnyElement>,
    trailing: Vec<AnyElement>,
    cx: &App,
) -> Toolbar {
    let t = cx.theme();
    let mut toolbar = Toolbar::new("compose-toolbar")
        .w_full()
        .h(px(44.))
        .p_1()
        .gap_1()
        .rounded_lg()
        .border_1()
        .border_color(t.border)
        .bg(t.background);
    let mut last_group: Option<u8> = None;
    for tool in TOOLS.iter().take(visible.min(TOOLS.len())) {
        if last_group.is_some_and(|group| group != tool.group) {
            toolbar = toolbar.content(separator());
        }
        last_group = Some(tool.group);
        toolbar = toolbar.content(tool.button(disabled, active.contains(&tool.id), cx));
    }
    if let Some(overflow) = overflow {
        toolbar = toolbar.content(overflow);
    }
    toolbar = toolbar.content(div().flex_1().min_w_0());
    if !trailing.is_empty() {
        toolbar = toolbar.content(separator());
    }
    for el in trailing {
        toolbar = toolbar.content(el);
    }
    toolbar
}

#[cfg(test)]
mod tests {
    use super::{TOOLS, visible_tools};

    /// A wide row keeps every format inline, and as the row narrows the last ones move to
    /// the overflow one at a time — never all at once, and never extra room for the trigger.
    #[test]
    fn tools_move_to_the_overflow_only_as_width_runs_out() {
        let total = TOOLS.len();
        assert_eq!(visible_tools(1200., total), total, "room for everything");
        assert_eq!(
            visible_tools(0., total),
            total,
            "a row that has not been laid out yet shows everything"
        );
        assert_eq!(visible_tools(1., total), 0, "no room at all");
        // A wide-enough row keeps all ten; narrower starts spilling into the menu, and does
        // not suddenly drop more than the room lost.
        assert_eq!(visible_tools(800., total), total, "all ten still fit");
        let narrower = visible_tools(560., total);
        assert!(narrower < total, "a narrower row hides some");
        assert!(narrower >= visible_tools(500., total));
        assert!(narrower < total - 1, "the overflow keeps its own room");
        // The count never exceeds the tools that exist.
        assert_eq!(visible_tools(10_000., total), total);
    }
}
