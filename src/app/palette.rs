//! Keyboard-first command palette.

use crate::app::actions::{self, CommandSpec, PALETTE_CONTEXT};
use gpui_kit::{
    prelude::FluentBuilder as _,
    component::{
        input::{Input, InputEvent, InputState},
        kbd::Kbd,
    },
    *,
};

const BG: u32 = 0x16171a;
const BORDER: u32 = 0x2a2c31;
const ROW_SELECTED: u32 = 0x25272c;
const TEXT: u32 = 0xd9dadd;
const ACCENT: u32 = 0x7dd3a8;

pub struct CommandPalette {
    input: Entity<InputState>,
    query: String,
    selected: usize,
    commands: Vec<CommandSpec>,
}

pub enum PaletteEvent {
    Run(Box<dyn Action>),
    Dismiss,
}

actions!(
    palette,
    [PaletteMoveUp, PaletteMoveDown, PaletteRun, PaletteDismiss]
);

impl CommandPalette {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Type a command…"));
        cx.subscribe(&input, |this, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.query = input.read(cx).value().to_string();
                this.selected = 0;
                cx.notify();
            }
        })
        .detach();
        cx.bind_keys([
            KeyBinding::new("up", PaletteMoveUp, Some(PALETTE_CONTEXT)),
            KeyBinding::new("ctrl-p", PaletteMoveUp, Some(PALETTE_CONTEXT)),
            KeyBinding::new("down", PaletteMoveDown, Some(PALETTE_CONTEXT)),
            KeyBinding::new("ctrl-n", PaletteMoveDown, Some(PALETTE_CONTEXT)),
            KeyBinding::new("enter", PaletteRun, Some(PALETTE_CONTEXT)),
            KeyBinding::new("escape", PaletteDismiss, Some(PALETTE_CONTEXT)),
        ]);
        Self {
            input,
            query: String::new(),
            selected: 0,
            commands: actions::commands(),
        }
    }

    pub fn query(&self) -> String {
        self.query.clone()
    }

    pub fn set_query(&mut self, q: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.query = q.to_owned();
        self.selected = 0;
        self.input
            .update(cx, |input, cx| input.set_value(q, window, cx));
        cx.notify();
    }

    fn visible(&self) -> Vec<usize> {
        filter_specs(&self.query, &self.commands)
    }
}

impl Focusable for CommandPalette {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl EventEmitter<PaletteEvent> for CommandPalette {}

impl Render for CommandPalette {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let indexes = self.visible();
        let selected = self.selected;
        div()
            .key_context(PALETTE_CONTEXT)
            .on_action(cx.listener(|this, _: &PaletteMoveUp, _, cx| {
                this.selected = this.selected.saturating_sub(1);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &PaletteMoveDown, _, cx| {
                let count = this.visible().len();
                if count > 0 {
                    this.selected = (this.selected + 1).min(count - 1);
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &PaletteRun, _, cx| {
                if let Some(&index) = this.visible().get(this.selected) {
                    cx.emit(PaletteEvent::Run((this.commands[index].action)()));
                }
            }))
            .on_action(cx.listener(|_, _: &PaletteDismiss, _, cx| cx.emit(PaletteEvent::Dismiss)))
            .flex()
            .flex_col()
            .w(px(440.))
            .max_h(px(520.))
            .p_2()
            .gap_1()
            .bg(rgb(BG))
            .border_1()
            .border_color(rgb(BORDER))
            .rounded_md()
            .child(Input::new(&self.input).appearance(false))
            .child(div().h(px(1.)).bg(rgb(BORDER)))
            .child(
                div()
                    .id("palette-list")
                    .flex()
                    .flex_col()
                    .overflow_y_scroll()
                    .children(indexes.iter().enumerate().map(|(row, &index)| {
                        let command = &self.commands[index];
                        div()
                            .id(("command", index))
                            .flex()
                            .items_center()
                            .justify_between()
                            .px_2()
                            .py_1()
                            .text_sm()
                            .rounded_sm()
                            .text_color(rgb(TEXT))
                            .when(row == selected, |el| {
                                el.bg(rgb(ROW_SELECTED)).text_color(rgb(ACCENT))
                            })
                            .child(command.name)
                            .child(
                                Kbd::new(
                                    Keystroke::parse(command.key)
                                        .unwrap_or_else(|_| Keystroke::parse("space").unwrap()),
                                )
                                .appearance(false),
                            )
                    })),
            )
    }
}

fn filter_specs(query: &str, commands: &[CommandSpec]) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return (0..commands.len()).collect();
    }
    commands
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            let name = c.name.to_lowercase();
            if name.contains(&q) || c.key.to_lowercase() == q {
                return true;
            }
            // Subsequence match keeps typo-tolerant, keyboard-oriented search useful.
            let mut chars = q.chars().filter(|c| !c.is_whitespace()).peekable();
            for ch in name.chars() {
                if chars.peek() == Some(&ch) {
                    chars.next();
                }
            }
            chars.peek().is_none()
        })
        .map(|(i, _)| i)
        .collect()
}

/// Indexes into [`actions::commands`] matching `q` (empty query matches all).
pub fn filter_commands(q: &str) -> Vec<usize> {
    filter_specs(q, &actions::commands())
}

#[cfg(test)]
mod tests {
    use super::{actions, filter_commands};

    fn names(q: &str) -> Vec<&'static str> {
        let cmds = actions::commands();
        filter_commands(q).into_iter().map(|i| cmds[i].name).collect()
    }

    #[test]
    fn empty_query_matches_everything() {
        assert_eq!(filter_commands("  ").len(), actions::commands().len());
    }

    #[test]
    fn substring_is_case_insensitive() {
        let n = names("MARK DONE");
        assert!(n.contains(&"Mark done"));
        assert!(!n.contains(&"Undo"));
    }

    #[test]
    fn exact_key_matches() {
        assert!(names("cmd-k").contains(&"Toggle command palette"));
    }

    #[test]
    fn subsequence_matches_and_gibberish_does_not() {
        assert!(names("mkdn").contains(&"Mark done"));
        assert!(filter_commands("zzzqqq").is_empty());
    }
}
