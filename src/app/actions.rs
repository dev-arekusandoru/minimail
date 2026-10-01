//! Actions (namespace `mail`), keymap and the command list shown in the palette/help.

use crate::app::chrome::HELP_CONTEXT;
use crate::app::panels::{RULES_CONTEXT, RulesNext, RulesPrev, RulesRevoke};
use crate::app::snooze::{SnoozeCustom, SnoozePreset1, SnoozePreset2, SnoozePreset3};
use crate::app::dialog::{Choice1, Choice2, Choice3, Choice4, Choice5, DialogConfirm};
use crate::judge::Kind;
use crate::model::{AccountId, Location, TagFilter};
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
        Archive,
        Delete,
        File,
        MoveToInbox,
        SenderArchive,
        SenderDelete,
        SenderFile,
        SenderSnooze,
        SenderInbox,
        Undo,
        ToggleCommandPalette,
        Reply,
        ReplyAll,
        Forward,
        SendReply,
        CancelCompose,
        ToggleHelp,
        HelpPageUp,
        HelpPageDown,
        OpenSnoozePicker,
        AcceptSuggestions,
        RejectSuggestions,
        AcceptRule,
        DismissRule,
        ToggleRules,
        AllowSender,
        BlockSender,
        MarkSpam,
        SpamBlock,
        MuteThread,
        Unsubscribe,
        SummarizeThread,
        ToggleSettings,
        AddGmailAccount,
        StartSession,
        OpenSearch,
        ClassifyVisible,
        ToggleGrouping,
        CyclePreviewLines,
        ExpandThread,
        CollapseThread,
        NextInThread,
        PrevInThread,
        CloseTab,
        NextTab,
        PrevTab,
        OpenFind,
        FindNext,
        FindPrev,
        CloseFind,
        ToggleFindCase,
        ToggleFindWord,
        ToggleFindRegex,
        ToggleReaderMode,
        ToggleThreadExpansion,
        GrowListPane,
        ShrinkListPane,
        ResetPanes,
        TogglePaneLayout,
        ToggleSidebar
    ]
);

/// Key context of the root view.
pub const MAIL_CONTEXT: &str = "MailApp";
/// Binding predicate for the list keymap: `MailApp` but not while a text input has focus
/// (a focused kit `Input` sets context `Input`; bare-letter bindings would otherwise steal typing).
const MAIL_BINDING: &str = "MailApp && !Input && !PopupMenu";
/// Same, for bindings that may fire while an input has focus. `!PopupMenu`: the app keeps its
/// `MailApp` context while a menu is open (the menu's keycaps are resolved through it), so
/// the menu itself is what keeps list keys inert.
const MAIL_SCOPE: &str = "MailApp && !PopupMenu";
/// Key context of the command palette.
pub const PALETTE_CONTEXT: &str = "CommandPalette";
/// Key context of the reader's find bar.
pub const FIND_CONTEXT: &str = "FindBar";
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
        KeyBinding::new("e", Archive, m),
        KeyBinding::new("d", Delete, m),
        // `#` (shift-3 on most layouts) is the second Delete key from the key table.
        KeyBinding::new("#", Delete, m),
        KeyBinding::new("f", File, m),
        KeyBinding::new("s", OpenSnoozePicker, m),
        KeyBinding::new("i", MoveToInbox, m),
        KeyBinding::new("shift-e", SenderArchive, m),
        KeyBinding::new("shift-d", SenderDelete, m),
        KeyBinding::new("shift-f", SenderFile, m),
        KeyBinding::new("shift-s", SenderSnooze, m),
        KeyBinding::new("shift-i", SenderInbox, m),
        KeyBinding::new("u", Undo, m),
        KeyBinding::new("cmd-z", Undo, m),
        KeyBinding::new("cmd-k", ToggleCommandPalette, m),
        KeyBinding::new("r", Reply, m),
        KeyBinding::new("shift-a", ReplyAll, m),
        KeyBinding::new("w", Forward, m),
        // Sidebar navigation: chips (Inbox views only) and `g`-prefix jumps.
        KeyBinding::new("1", SelectChip1, m),
        KeyBinding::new("2", SelectChip2, m),
        KeyBinding::new("3", SelectChip3, m),
        KeyBinding::new("4", SelectChip4, m),
        KeyBinding::new("5", SelectChip5, m),
        KeyBinding::new("6", SelectChip6, m),
        KeyBinding::new("g i", GoInbox, m),
        KeyBinding::new("g s", GoSnoozed, m),
        KeyBinding::new("g t", GoSent, m),
        KeyBinding::new("g a", GoArchive, m),
        KeyBinding::new("g d", GoTrash, m),
        KeyBinding::new("?", ToggleHelp, m),
        KeyBinding::new("y", AcceptSuggestions, m),
        KeyBinding::new("n", RejectSuggestions, m),
        KeyBinding::new("shift-y", AcceptRule, m),
        KeyBinding::new("shift-n", DismissRule, m),
        KeyBinding::new("shift-r", ToggleRules, m),
        KeyBinding::new("a", AllowSender, m),
        KeyBinding::new("b", BlockSender, m),
        KeyBinding::new("!", MarkSpam, m),
        KeyBinding::new("m", MuteThread, m),
        KeyBinding::new("shift-u", Unsubscribe, m),
        KeyBinding::new("z", SummarizeThread, m),
        KeyBinding::new("cmd-,", ToggleSettings, m),
        KeyBinding::new("t", StartSession, m),
        KeyBinding::new("/", OpenSearch, m),
        KeyBinding::new("c", ClassifyVisible, m),
        KeyBinding::new("ctrl-g", ToggleGrouping, m),
        KeyBinding::new("right", ExpandThread, m),
        KeyBinding::new("left", CollapseThread, m),
        KeyBinding::new("]", NextInThread, m),
        KeyBinding::new("[", PrevInThread, m),
        KeyBinding::new("v", ToggleReaderMode, m),
        KeyBinding::new("shift-o", ToggleThreadExpansion, m),
        // Reader tabs. Modifier chords, so they work from any focus inside the app.
        KeyBinding::new("cmd-w", CloseTab, Some(MAIL_SCOPE)),
        KeyBinding::new("ctrl-tab", NextTab, Some(MAIL_SCOPE)),
        KeyBinding::new("cmd-shift-]", NextTab, Some(MAIL_SCOPE)),
        KeyBinding::new("ctrl-shift-tab", PrevTab, Some(MAIL_SCOPE)),
        KeyBinding::new("cmd-shift-[", PrevTab, Some(MAIL_SCOPE)),
        // Global search (the titlebar's search, like `/`) and find in the active reader tab.
        KeyBinding::new("cmd-shift-f", OpenSearch, Some(MAIL_SCOPE)),
        KeyBinding::new("cmd-f", OpenFind, Some(MAIL_SCOPE)),
        KeyBinding::new("cmd-g", FindNext, Some(MAIL_SCOPE)),
        KeyBinding::new("cmd-shift-g", FindPrev, Some(MAIL_SCOPE)),
        // Find bar (its input has focus, so these are not under `m`).
        KeyBinding::new("enter", FindNext, Some(FIND_CONTEXT)),
        KeyBinding::new("shift-enter", FindPrev, Some(FIND_CONTEXT)),
        KeyBinding::new("escape", CloseFind, Some(FIND_CONTEXT)),
        KeyBinding::new("alt-c", ToggleFindCase, Some(FIND_CONTEXT)),
        KeyBinding::new("alt-w", ToggleFindWord, Some(FIND_CONTEXT)),
        KeyBinding::new("alt-r", ToggleFindRegex, Some(FIND_CONTEXT)),
        // Pane divider.
        KeyBinding::new("alt-right", GrowListPane, m),
        KeyBinding::new("alt-left", ShrinkListPane, m),
        KeyBinding::new("alt-r", ResetPanes, m),
        KeyBinding::new("alt-l", TogglePaneLayout, m),
        KeyBinding::new("cmd-b", ToggleSidebar, m),
        // Snooze picker.
        KeyBinding::new("1", SnoozePreset1, Some("SnoozePicker && !Input")),
        KeyBinding::new("2", SnoozePreset2, Some("SnoozePicker && !Input")),
        KeyBinding::new("3", SnoozePreset3, Some("SnoozePicker && !Input")),
        KeyBinding::new("4", SnoozeCustom, Some("SnoozePicker && !Input")),
        // Choice dialog: one key per option, enter = default; escape is the dialog's.
        KeyBinding::new("1", Choice1, Some("ChoiceDialog && !Input")),
        KeyBinding::new("2", Choice2, Some("ChoiceDialog && !Input")),
        KeyBinding::new("3", Choice3, Some("ChoiceDialog && !Input")),
        KeyBinding::new("4", Choice4, Some("ChoiceDialog && !Input")),
        KeyBinding::new("5", Choice5, Some("ChoiceDialog && !Input")),
        KeyBinding::new("enter", DialogConfirm, Some("ChoiceDialog && !Input")),
        // Help panel: scroll keys and `?` to close; escape is the dialog's.
        KeyBinding::new("?", ToggleHelp, Some(HELP_CONTEXT)),
        KeyBinding::new("j", SelectNext, Some(HELP_CONTEXT)),
        KeyBinding::new("down", SelectNext, Some(HELP_CONTEXT)),
        KeyBinding::new("k", SelectPrev, Some(HELP_CONTEXT)),
        KeyBinding::new("up", SelectPrev, Some(HELP_CONTEXT)),
        KeyBinding::new("pageup", HelpPageUp, Some(HELP_CONTEXT)),
        KeyBinding::new("pagedown", HelpPageDown, Some(HELP_CONTEXT)),
        // Popup menus: the kit owns arrows, enter and escape; `j`/`k` are the list's keys.
        KeyBinding::new("j", gpui_kit::base::actions::SelectDown, Some("PopupMenu")),
        KeyBinding::new("k", gpui_kit::base::actions::SelectUp, Some("PopupMenu")),
        // Rules panel.
        KeyBinding::new("j", RulesNext, Some(RULES_CONTEXT)),
        KeyBinding::new("k", RulesPrev, Some(RULES_CONTEXT)),
        KeyBinding::new("backspace", RulesRevoke, Some(RULES_CONTEXT)),
        KeyBinding::new("d", RulesRevoke, Some(RULES_CONTEXT)),
        // Compose context.
        KeyBinding::new("cmd-enter", SendReply, Some(COMPOSE_CONTEXT)),
        KeyBinding::new("escape", CancelCompose, Some(COMPOSE_CONTEXT)),
        // Palette dialog: the kit's Command owns navigation, enter and escape.
        KeyBinding::new("cmd-k", ToggleCommandPalette, p),
    ]);
}

/// Palette section a command is listed under.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Category {
    Navigate,
    Triage,
    Sender,
    Classify,
    Tabs,
    Find,
    View,
    App,
}

impl Category {
    /// Palette section order.
    pub const ALL: [Category; 8] = [
        Category::Navigate,
        Category::Triage,
        Category::Sender,
        Category::Classify,
        Category::Tabs,
        Category::Find,
        Category::View,
        Category::App,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::Navigate => "Navigate",
            Category::Triage => "Triage",
            Category::Sender => "Sender",
            Category::Classify => "Classify",
            Category::Tabs => "Tabs",
            Category::Find => "Find",
            Category::View => "View",
            Category::App => "App",
        }
    }
}

/// One user-facing command.
pub struct CommandSpec {
    pub category: Category,
    pub name: &'static str,
    /// Human-readable key hint, e.g. `"e"`, `"shift-e"`, `"cmd-k"`.
    pub key: &'static str,
    pub action: fn() -> Box<dyn Action>,
}

macro_rules! cmd {
    ($cat:ident, $name:expr, $key:expr, $a:ident) => {
        CommandSpec {
            category: Category::$cat,
            name: $name,
            key: $key,
            action: || Box::new($a),
        }
    };
}

/// Every action except palette/compose-internal ones.
pub fn commands() -> Vec<CommandSpec> {
    vec![
        cmd!(Navigate, "Next message", "j", SelectNext),
        cmd!(Navigate, "Previous message", "k", SelectPrev),
        cmd!(Navigate, "Extend selection down", "shift-j", ExtendNext),
        cmd!(Navigate, "Extend selection up", "shift-k", ExtendPrev),
        cmd!(Navigate, "Toggle select", "x", ToggleSelect),
        cmd!(Navigate, "Clear selection", "escape", ClearSelection),
        cmd!(Navigate, "Open message", "enter", OpenMessage),
        cmd!(Triage, "Archive", "e", Archive),
        cmd!(Triage, "Delete", "d", Delete),
        cmd!(Triage, "File…", "f", File),
        cmd!(Triage, "Move to inbox", "i", MoveToInbox),
        cmd!(Sender, "Archive all from sender…", "shift-e", SenderArchive),
        cmd!(Sender, "Delete all from sender…", "shift-d", SenderDelete),
        cmd!(Sender, "File all from sender…", "shift-f", SenderFile),
        cmd!(Sender, "Snooze all from sender…", "shift-s", SenderSnooze),
        cmd!(Sender, "Move all from sender to inbox…", "shift-i", SenderInbox),
        cmd!(Triage, "Undo", "u", Undo),
        cmd!(Triage, "Reply", "r", Reply),
        cmd!(Triage, "Reply all", "shift-a", ReplyAll),
        cmd!(Triage, "Forward", "w", Forward),
        cmd!(App, "Toggle command palette", "cmd-k", ToggleCommandPalette),
        cmd!(App, "Toggle help", "?", ToggleHelp),
        cmd!(Triage, "Snooze…", "s", OpenSnoozePicker),
        cmd!(Classify, "Accept suggestions", "y", AcceptSuggestions),
        cmd!(Classify, "Reject suggestions", "n", RejectSuggestions),
        cmd!(Classify, "Accept rule suggestion", "shift-y", AcceptRule),
        cmd!(Classify, "Dismiss rule suggestion", "shift-n", DismissRule),
        cmd!(Classify, "Toggle rules panel", "shift-r", ToggleRules),
        cmd!(Triage, "Allow sender", "a", AllowSender),
        cmd!(Triage, "Block sender…", "b", BlockSender),
        cmd!(Triage, "Mark spam…", "!", MarkSpam),
        cmd!(Triage, "Block sender and delete", "", SpamBlock),
        cmd!(Triage, "Mute thread", "m", MuteThread),
        cmd!(Triage, "Unsubscribe from sender…", "shift-u", Unsubscribe),
        cmd!(Classify, "Summarize thread", "z", SummarizeThread),
        cmd!(App, "Toggle settings", "cmd-,", ToggleSettings),
        cmd!(App, "Add Gmail account", "", AddGmailAccount),
        cmd!(App, "Start triage session", "t", StartSession),
        cmd!(Navigate, "Search", "/", OpenSearch),
        cmd!(Classify, "Classify visible mail", "c", ClassifyVisible),
        cmd!(View, "Toggle group by thread", "ctrl-g", ToggleGrouping),
        cmd!(View, "Cycle preview lines", "", CyclePreviewLines),
        cmd!(Navigate, "Expand thread", "right", ExpandThread),
        cmd!(Navigate, "Collapse thread", "left", CollapseThread),
        cmd!(Navigate, "Next message in thread", "]", NextInThread),
        cmd!(Navigate, "Previous message in thread", "[", PrevInThread),
        cmd!(Tabs, "Close tab", "cmd-w", CloseTab),
        cmd!(Tabs, "Next tab", "ctrl-tab", NextTab),
        cmd!(Tabs, "Previous tab", "ctrl-shift-tab", PrevTab),
        cmd!(Find, "Find in thread", "cmd-f", OpenFind),
        cmd!(Find, "Next match", "cmd-g", FindNext),
        cmd!(Find, "Previous match", "cmd-shift-g", FindPrev),
        cmd!(Find, "Find: match case", "alt-c", ToggleFindCase),
        cmd!(Find, "Find: whole word", "alt-w", ToggleFindWord),
        cmd!(Find, "Find: regex", "alt-r", ToggleFindRegex),
        cmd!(View, "Toggle reader mode", "v", ToggleReaderMode),
        cmd!(View, "Expand or collapse thread messages", "shift-o", ToggleThreadExpansion),
        cmd!(View, "Grow list pane", "alt-right", GrowListPane),
        cmd!(View, "Shrink list pane", "alt-left", ShrinkListPane),
        cmd!(View, "Reset pane sizes", "alt-r", ResetPanes),
        cmd!(View, "Toggle pane layout", "alt-l", TogglePaneLayout),
        cmd!(View, "Toggle sidebar", "cmd-b", ToggleSidebar),
        // Sidebar navigation.
        cmd!(Navigate, "Go to inbox", "g i", GoInbox),
        cmd!(Navigate, "Go to snoozed", "g s", GoSnoozed),
        cmd!(Navigate, "Go to sent", "g t", GoSent),
        cmd!(Navigate, "Go to archive", "g a", GoArchive),
        cmd!(Navigate, "Go to trash", "g d", GoTrash),
        cmd!(Navigate, "Chip: all", "1", SelectChip1),
        cmd!(Navigate, "Chip: needs reply", "2", SelectChip2),
        cmd!(Navigate, "Chip: follow up", "3", SelectChip3),
        cmd!(Navigate, "Chip: urgent", "4", SelectChip4),
        cmd!(Navigate, "Chip: new senders", "5", SelectChip5),
        cmd!(Navigate, "Chip: possible spam", "6", SelectChip6),
        cmd!(Navigate, "Clear filters", "", ClearFilters),
    ]
}

// ---------------------------------------------------------------- Sidebar navigation

gpui_kit::actions!(
    mail,
    [
        GoInbox,
        GoSnoozed,
        GoSent,
        GoArchive,
        GoTrash,
        SelectChip1,
        SelectChip2,
        SelectChip3,
        SelectChip4,
        SelectChip5,
        SelectChip6,
        ClearFilters
    ]
);

/// Jump the sidebar to one location (a sidebar row click).
#[derive(Clone, PartialEq, gpui_kit::Action)]
#[action(namespace = mail, no_json)]
pub struct ShowLocation {
    pub location: Location,
}

/// Add or remove one tag from the Filter ▾ menu's tag list.
#[derive(Clone, PartialEq, gpui_kit::Action)]
#[action(namespace = mail, no_json)]
pub struct ToggleTagFilter {
    pub tag: TagFilter,
}

/// Pick the Filter ▾ menu's Kind (`None` = any kind).
#[derive(Clone, PartialEq, gpui_kit::Action)]
#[action(namespace = mail, no_json)]
pub struct SetFilterKind {
    pub kind: Option<Kind>,
}

/// Pick the Filter ▾ menu's account (`None` = every account).
#[derive(Clone, PartialEq, gpui_kit::Action)]
#[action(namespace = mail, no_json)]
pub struct SetFilterAccount {
    pub account: Option<AccountId>,
}
