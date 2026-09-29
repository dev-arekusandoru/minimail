//! Actions (namespace `mail`), keymap and the command list shown in the palette/help.

use crate::app::palette::{PaletteDismiss, PaletteMoveDown, PaletteMoveUp, PaletteRun};
use gpui_kit::*;

gpui_kit::actions!(
    mail,
    [
        SelectNext,
        SelectPrev,
        ExtendNext,
        ExtendPrev,
        ToggleSelect,
        ClearSelection,
        OpenMessage,
        MarkDone,
        MarkWaiting,
        MarkLater,
        MoveToInbox,
        SenderDone,
        SenderWaiting,
        SenderLater,
        SenderInbox,
        Undo,
        ToggleCommandPalette,
        Reply,
        SendReply,
        CancelCompose,
        ShowInbox,
        ShowWaiting,
        ShowLater,
        ShowDone,
        ToggleHelp
    ]
);

/// Key context of the root view.
pub const MAIL_CONTEXT: &str = "MailApp";
/// Binding predicate for the list keymap: `MailApp` but not while a text input has focus
/// (a focused kit `Input` sets context `Input`; bare-letter bindings would otherwise steal typing).
const MAIL_BINDING: &str = "MailApp && !Input";
/// Key context of the command palette.
pub const PALETTE_CONTEXT: &str = "CommandPalette";
/// Key context of the compose pane.
pub const COMPOSE_CONTEXT: &str = "Compose";

/// Register the whole keymap.
pub fn bind_keys(cx: &mut App) {
    let m = Some(MAIL_BINDING);
    let p = Some(PALETTE_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("j", SelectNext, m),
        KeyBinding::new("down", SelectNext, m),
        KeyBinding::new("k", SelectPrev, m),
        KeyBinding::new("up", SelectPrev, m),
        KeyBinding::new("shift-j", ExtendNext, m),
        KeyBinding::new("shift-k", ExtendPrev, m),
        KeyBinding::new("x", ToggleSelect, m),
        KeyBinding::new("escape", ClearSelection, m),
        KeyBinding::new("enter", OpenMessage, m),
        KeyBinding::new("o", OpenMessage, m),
        KeyBinding::new("e", MarkDone, m),
        KeyBinding::new("w", MarkWaiting, m),
        KeyBinding::new("l", MarkLater, m),
        KeyBinding::new("i", MoveToInbox, m),
        KeyBinding::new("shift-e", SenderDone, m),
        KeyBinding::new("shift-w", SenderWaiting, m),
        KeyBinding::new("shift-l", SenderLater, m),
        KeyBinding::new("shift-i", SenderInbox, m),
        KeyBinding::new("u", Undo, m),
        KeyBinding::new("cmd-z", Undo, m),
        KeyBinding::new("cmd-k", ToggleCommandPalette, m),
        KeyBinding::new("r", Reply, m),
        KeyBinding::new("1", ShowInbox, m),
        KeyBinding::new("2", ShowWaiting, m),
        KeyBinding::new("3", ShowLater, m),
        KeyBinding::new("4", ShowDone, m),
        KeyBinding::new("?", ToggleHelp, m),
        // Compose context.
        KeyBinding::new("cmd-enter", SendReply, Some(COMPOSE_CONTEXT)),
        KeyBinding::new("escape", CancelCompose, Some(COMPOSE_CONTEXT)),
        // Palette context.
        KeyBinding::new("up", PaletteMoveUp, p),
        KeyBinding::new("ctrl-p", PaletteMoveUp, p),
        KeyBinding::new("down", PaletteMoveDown, p),
        KeyBinding::new("ctrl-n", PaletteMoveDown, p),
        KeyBinding::new("enter", PaletteRun, p),
        KeyBinding::new("escape", PaletteDismiss, p),
        KeyBinding::new("cmd-k", ToggleCommandPalette, p),
    ]);
}

/// One user-facing command.
pub struct CommandSpec {
    pub name: &'static str,
    /// Human-readable key hint, e.g. `"e"`, `"shift-e"`, `"cmd-k"`.
    pub key: &'static str,
    pub action: fn() -> Box<dyn Action>,
}

macro_rules! cmd {
    ($name:expr, $key:expr, $a:ident) => {
        CommandSpec {
            name: $name,
            key: $key,
            action: || Box::new($a),
        }
    };
}

/// Every action except palette/compose-internal ones.
pub fn commands() -> Vec<CommandSpec> {
    vec![
        cmd!("Next message", "j", SelectNext),
        cmd!("Previous message", "k", SelectPrev),
        cmd!("Extend selection down", "shift-j", ExtendNext),
        cmd!("Extend selection up", "shift-k", ExtendPrev),
        cmd!("Toggle select", "x", ToggleSelect),
        cmd!("Clear selection", "escape", ClearSelection),
        cmd!("Open message", "enter", OpenMessage),
        cmd!("Mark done", "e", MarkDone),
        cmd!("Mark waiting", "w", MarkWaiting),
        cmd!("Mark later", "l", MarkLater),
        cmd!("Move to inbox", "i", MoveToInbox),
        cmd!("Mark all from sender done", "shift-e", SenderDone),
        cmd!("Mark all from sender waiting", "shift-w", SenderWaiting),
        cmd!("Mark all from sender later", "shift-l", SenderLater),
        cmd!("Move all from sender to inbox", "shift-i", SenderInbox),
        cmd!("Undo", "u", Undo),
        cmd!("Reply", "r", Reply),
        cmd!("Show inbox", "1", ShowInbox),
        cmd!("Show waiting", "2", ShowWaiting),
        cmd!("Show later", "3", ShowLater),
        cmd!("Show done", "4", ShowDone),
        cmd!("Toggle command palette", "cmd-k", ToggleCommandPalette),
        cmd!("Toggle help", "?", ToggleHelp),
    ]
}
