//! Keyboard-first command palette.

use crate::app::actions::{self, CommandSpec, PALETTE_CONTEXT};
use gpui_kit::{component::{input::{Input, InputEvent, InputState}, kbd::Kbd}, *};

pub struct CommandPalette {
    focus_handle: FocusHandle,
    input: Entity<InputState>,
    query: String,
    selected: usize,
    commands: Vec<CommandSpec>,
}

pub enum PaletteEvent {
    Run(Box<dyn Action>),
    Dismiss,
}

actions!(palette, [PaletteMoveUp, PaletteMoveDown, PaletteRun, PaletteDismiss]);

impl CommandPalette {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        let input = cx.new(|cx| InputState::new(window, cx));
        let commands = actions::commands();
        cx.subscribe(&input, |this, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.query = input.read(cx).value().to_string();
                this.selected = 0;
                cx.notify();
            }
        }).detach();
        cx.bind_keys([
            KeyBinding::new("up", PaletteMoveUp, Some(PALETTE_CONTEXT)),
            KeyBinding::new("ctrl-p", PaletteMoveUp, Some(PALETTE_CONTEXT)),
            KeyBinding::new("down", PaletteMoveDown, Some(PALETTE_CONTEXT)),
            KeyBinding::new("ctrl-n", PaletteMoveDown, Some(PALETTE_CONTEXT)),
            KeyBinding::new("enter", PaletteRun, Some(PALETTE_CONTEXT)),
            KeyBinding::new("escape", PaletteDismiss, Some(PALETTE_CONTEXT)),
        ]);
        cx.on_action(|this: &mut Self, _: &PaletteMoveUp, _, cx| {
            let count = filter_commands(&this.query, &this.commands).len();
            if count > 0 { this.selected = this.selected.saturating_sub(1); }
            cx.notify();
        });
        cx.on_action(|this: &mut Self, _: &PaletteMoveDown, _, cx| {
            let count = filter_commands(&this.query, &this.commands).len();
            if count > 0 { this.selected = (this.selected + 1).min(count - 1); }
            cx.notify();
        });
        cx.on_action(|this: &mut Self, _: &PaletteRun, _, cx| {
            let indexes = filter_commands(&this.query, &this.commands);
            if let Some(index) = indexes.get(this.selected) {
                cx.emit(PaletteEvent::Run((this.commands[*index].action)()));
            }
        });
        cx.on_action(|_, _: &PaletteDismiss, _, cx| cx.emit(PaletteEvent::Dismiss));
        Self { focus_handle, input, query: String::new(), selected: 0, commands }
    }

    pub fn query(&self) -> String { self.query.clone() }

    pub fn set_query(&mut self, q: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.query = q.to_owned();
        self.selected = 0;
        self.input.update(cx, |input, cx| input.set_value(q, window, cx));
        cx.notify();
    }
}

impl Focusable for CommandPalette {
    fn focus_handle(&self, _: &App) -> FocusHandle { self.focus_handle.clone() }
}

impl EventEmitter<PaletteEvent> for CommandPalette {}

impl Render for CommandPalette {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let indexes = filter_commands(&self.query, &self.commands);
        div().key_context(PALETTE_CONTEXT).track_focus(&self.focus_handle)
            .flex().flex_col().w(px(440.)).max_h(px(520.)).p_3().gap_2()
            .bg(rgb(0x202124)).border_1().border_color(rgb(0x44464a)).rounded_md()
            .child(Input::new(&self.input).placeholder("Filter commands…"))
            .child(div().flex().flex_col().gap_1().children(indexes.iter().enumerate().map(|(row, &index)| {
                let command = &self.commands[index];
                div().id(format!("command-{index}"))
                    .flex().items_center().justify_between().px_2().py_1().rounded_sm()
                    .when(row == self.selected, |el| el.bg(rgb(0x34383d)))
                    .child(div().text_sm().text_color(rgb(0xe6e6e6)).child(command.name))
                    .child(Kbd::new(Keystroke::parse(command.key).unwrap_or_else(|_| Keystroke::parse("space").unwrap()))
                        .appearance(false))
            })))
    }
}

fn filter_commands(query: &str, commands: &[CommandSpec]) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    if q.is_empty() { return (0..commands.len()).collect(); }
    let mut matches = Vec::new();
    for (index, command) in commands.iter().enumerate() {
        let name = command.name.to_lowercase();
        if name.contains(&q) || command.key.to_lowercase().contains(&q) {
            matches.push(index);
        } else {
            // Subsequence fuzzy match keeps typo-tolerant, keyboard-oriented search useful.
            let mut chars = q.chars();
            let mut next = chars.next();
            for ch in name.chars() {
                if Some(ch) == next { next = chars.next(); }
                if next.is_none() { matches.push(index); break; }
            }
        }
    }
    matches
}
