//! Keyboard-first command palette.

use crate::app::overlay::FitViewport as _;
use crate::app::actions::{self, CommandSpec, PALETTE_CONTEXT};
use crate::search::Query;
use gpui_kit::{
    prelude::FluentBuilder as _,
    component::input::{Input, InputEvent, InputState},
    *,
};

use crate::theme;

pub struct CommandPalette {
    input: Entity<InputState>,
    query: String,
    selected: usize,
    commands: Vec<CommandSpec>,
}

pub enum PaletteEvent {
    Run(Box<dyn Action>),
    Dismiss,
    /// Enter on a search query (`/…` or containing `from:` etc.); carries the raw text.
    Search(String),
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

    /// Names of the commands currently listed (empty while a search query is shown).
    pub fn rows(&self) -> Vec<String> {
        if Query::is_search(&self.query) {
            return Vec::new();
        }
        self.visible().into_iter().map(|i| self.commands[i].name.to_owned()).collect()
    }
}

impl Focusable for CommandPalette {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl EventEmitter<PaletteEvent> for CommandPalette {}

impl Render for CommandPalette {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let indexes = self.visible();
        let selected = self.selected;
        let t = theme::active(cx);
        let hover = t.hover;
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
                if Query::is_search(&this.query) {
                    cx.emit(PaletteEvent::Search(this.query.clone()));
                } else if let Some(&index) = this.visible().get(this.selected) {
                    cx.emit(PaletteEvent::Run((this.commands[index].action)()));
                }
            }))
            .on_action(cx.listener(|_, _: &PaletteDismiss, _, cx| cx.emit(PaletteEvent::Dismiss)))
            .flex()
            .flex_col()
            .fit_viewport(window, 440.)
            .p_2()
            .gap_1()
            .bg(t.surface)
            .border_1()
            .border_color(t.border)
            .rounded_md()
            .child(Input::new(&self.input).appearance(false))
            .child(div().h(px(1.)).bg(t.border))
            .child(if Query::is_search(&self.query) {
                div()
                    .id("palette-search")
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.emit(PaletteEvent::Search(this.query.clone()));
                    }))
                    .px_2()
                    .py_1()
                    .text_sm()
                    .text_color(t.accent)
                    .bg(t.selection)
                    .rounded_sm()
                    .child(SharedString::from(format!(
                        "Search: {}",
                        Query::parse(&self.query).describe()
                    )))
                    .into_any_element()
            } else {
                div()
                    .id("palette-list")
                    .flex()
                    .flex_col()
                    .overflow_y_scroll()
                    .children(indexes.iter().enumerate().map(|(row, &index)| {
                        let command = &self.commands[index];
                        div()
                            .id(("command", index))
                            .test_support()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px_2()
                            .py_1()
                            .text_sm()
                            .rounded_sm()
                            .text_color(t.text)
                            .hover(move |el| el.bg(hover))
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.emit(PaletteEvent::Run((this.commands[index].action)()));
                            }))
                            .when(row == selected, |el| {
                                el.bg(t.selection).text_color(t.accent)
                            })
                            .child(command.name)
                            .when(!command.key.is_empty(), |el| {
                                el.child(crate::app::ui::shortcut(command.key))
                            })
                    }))
                    .into_any_element()
            })
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
        let n = names("ARCHIVE");
        assert!(n.contains(&"Archive"));
        assert!(!n.contains(&"Undo"));
    }

    #[test]
    fn exact_key_matches() {
        assert!(names("cmd-k").contains(&"Toggle command palette"));
    }

    #[test]
    fn subsequence_matches_and_gibberish_does_not() {
        assert!(names("archv").contains(&"Archive"));
        assert!(filter_commands("zzzqqq").is_empty());
    }
}
