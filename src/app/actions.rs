//! Actions (namespace `mail`), keymap and the command list shown in the palette/help.

use crate::app::palette::{PaletteDismiss, PaletteMoveDown, PaletteMoveUp, PaletteRun};
use crate::app::panels::{RULES_CONTEXT, RulesClose, RulesNext, RulesPrev, RulesRevoke};
use crate::app::settings::{
    SETTINGS_CONTEXT, SettingsClose, SettingsNext, SettingsPrev, SettingsSearch,
    SettingsSectionNext, SettingsSectionPrev, SettingsThresholdDown, SettingsThresholdUp,
    SettingsToggleMode,
};
use crate::app::snooze::{
    SNOOZE_CONTEXT, SnoozeCancel, SnoozeCustom, SnoozePreset1, SnoozePreset2, SnoozePreset3,
};
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
        ToggleHelp,
        HelpPageUp,
        HelpPageDown,
        OpenSnoozePicker,
        AcceptSuggestions,
        RejectSuggestions,
        AcceptRule,
        DismissRule,
        ToggleRules,
        ShowScreener,
        AllowSender,
        BlockSender,
        MuteThread,
        Unsubscribe,
        SummarizeThread,
        ToggleSettings,
        StartSession,
        OpenSearch,
        ClassifyVisible,
        ToggleGrouping,
        CyclePreviewLines,
        ExpandThread,
        CollapseThread,
        NextInThread,
        PrevInThread
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
        KeyBinding::new("l", OpenSnoozePicker, m),
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
        KeyBinding::new("pageup", HelpPageUp, m),
        KeyBinding::new("pagedown", HelpPageDown, m),
        KeyBinding::new("y", AcceptSuggestions, m),
        KeyBinding::new("n", RejectSuggestions, m),
        KeyBinding::new("shift-y", AcceptRule, m),
        KeyBinding::new("shift-n", DismissRule, m),
        KeyBinding::new("shift-r", ToggleRules, m),
        KeyBinding::new("5", ShowScreener, m),
        KeyBinding::new("a", AllowSender, m),
        KeyBinding::new("b", BlockSender, m),
        KeyBinding::new("m", MuteThread, m),
        KeyBinding::new("shift-u", Unsubscribe, m),
        KeyBinding::new("s", SummarizeThread, m),
        KeyBinding::new("cmd-,", ToggleSettings, m),
        KeyBinding::new("t", StartSession, m),
        KeyBinding::new("/", OpenSearch, m),
        KeyBinding::new("c", ClassifyVisible, m),
        KeyBinding::new("g", ToggleGrouping, m),
        KeyBinding::new("right", ExpandThread, m),
        KeyBinding::new("left", CollapseThread, m),
        KeyBinding::new("]", NextInThread, m),
        KeyBinding::new("[", PrevInThread, m),
        // Snooze picker.
        KeyBinding::new("1", SnoozePreset1, Some("SnoozePicker && !Input")),
        KeyBinding::new("2", SnoozePreset2, Some("SnoozePicker && !Input")),
        KeyBinding::new("3", SnoozePreset3, Some("SnoozePicker && !Input")),
        KeyBinding::new("4", SnoozeCustom, Some("SnoozePicker && !Input")),
        KeyBinding::new("escape", SnoozeCancel, Some(SNOOZE_CONTEXT)),
        // Settings panel.
        KeyBinding::new("j", SettingsNext, Some(SETTINGS_CONTEXT)),
        KeyBinding::new("down", SettingsNext, Some(SETTINGS_CONTEXT)),
        KeyBinding::new("k", SettingsPrev, Some(SETTINGS_CONTEXT)),
        KeyBinding::new("up", SettingsPrev, Some(SETTINGS_CONTEXT)),
        KeyBinding::new("space", SettingsToggleMode, Some(SETTINGS_CONTEXT)),
        KeyBinding::new("enter", SettingsToggleMode, Some(SETTINGS_CONTEXT)),
        KeyBinding::new("=", SettingsThresholdUp, Some(SETTINGS_CONTEXT)),
        KeyBinding::new("-", SettingsThresholdDown, Some(SETTINGS_CONTEXT)),
        KeyBinding::new("escape", SettingsClose, Some(SETTINGS_CONTEXT)),
        KeyBinding::new("/", SettingsSearch, Some(SETTINGS_CONTEXT)),
        KeyBinding::new("ctrl-tab", SettingsSectionNext, Some(SETTINGS_CONTEXT)),
        KeyBinding::new("ctrl-shift-tab", SettingsSectionPrev, Some(SETTINGS_CONTEXT)),
        // Rules panel.
        KeyBinding::new("j", RulesNext, Some(RULES_CONTEXT)),
        KeyBinding::new("k", RulesPrev, Some(RULES_CONTEXT)),
        KeyBinding::new("backspace", RulesRevoke, Some(RULES_CONTEXT)),
        KeyBinding::new("d", RulesRevoke, Some(RULES_CONTEXT)),
        KeyBinding::new("escape", RulesClose, Some(RULES_CONTEXT)),
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
        cmd!("Mark later", "", MarkLater),
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
        cmd!("Snooze…", "l", OpenSnoozePicker),
        cmd!("Accept suggestions", "y", AcceptSuggestions),
        cmd!("Reject suggestions", "n", RejectSuggestions),
        cmd!("Accept rule suggestion", "shift-y", AcceptRule),
        cmd!("Dismiss rule suggestion", "shift-n", DismissRule),
        cmd!("Toggle rules panel", "shift-r", ToggleRules),
        cmd!("Show screener", "5", ShowScreener),
        cmd!("Allow sender (screener)", "a", AllowSender),
        cmd!("Block sender (screener)", "b", BlockSender),
        cmd!("Mute thread", "m", MuteThread),
        cmd!("Unsubscribe from sender", "shift-u", Unsubscribe),
        cmd!("Summarize thread", "s", SummarizeThread),
        cmd!("Toggle settings", "cmd-,", ToggleSettings),
        cmd!("Start triage session", "t", StartSession),
        cmd!("Search", "/", OpenSearch),
        cmd!("Classify visible mail", "c", ClassifyVisible),
        cmd!("Toggle group by thread", "g", ToggleGrouping),
        cmd!("Cycle preview lines", "", CyclePreviewLines),
        cmd!("Expand thread", "right", ExpandThread),
        cmd!("Collapse thread", "left", CollapseThread),
        cmd!("Next message in thread", "]", NextInThread),
        cmd!("Previous message in thread", "[", PrevInThread),
    ]
}
