//! Menu wiring for the root view: what each menu holds right now, and how a choice
//! is dispatched.
//!
//! Every entry is an ordinary [`Action`], so a menu click, a submenu click and the
//! keyboard shortcut all reach the same handler on the root view.

use super::*;

use crate::app::menu::{MenuEvent, MenuItem, MenuPanel};

/// Which menu the open panel belongs to; decides what it holds, where it is anchored (its
/// trigger's bounds) and which trigger shows itself as open.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum MenuKind {
    /// Overflow of the quiet header.
    Global,
    /// Actions for one message of the reader; they apply to that message only.
    Message(MessageId),
    /// Actions for the list's selection.
    Selection,
    /// The list header's Filter ▾ menu.
    Filter,
}

/// An open menu: which one it is, the panel that draws it, and the messages its actions
/// apply to (`None`: the usual cursor/selection targets).
pub(super) struct OpenMenu {
    pub kind: MenuKind,
    pub panel: Entity<MenuPanel>,
    pub target: Option<Vec<MessageId>>,
    /// Estimated panel height, to keep it inside the window.
    pub height: f32,
}

impl MailApp {
    /// Open a menu (or close it when it is already the open one), or do nothing when
    /// the view has nothing to offer: a menu of dead rows is worse than no menu.
    pub(super) fn toggle_menu(
        &mut self,
        kind: MenuKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.menu_open() {
            self.close_menu(window, cx);
            return;
        }
        if self.modal_open() {
            return;
        }
        let (items, target) = match kind {
            MenuKind::Global => (global_items(), None),
            MenuKind::Filter => (self.filter_items(), None),
            MenuKind::Message(id) => (self.message_items(id), Some(vec![id])),
            MenuKind::Selection => (self.selection_items(), None),
        };
        if items.is_empty() {
            return;
        }
        let height = menu_height(&items);
        let panel = cx.new(|cx| MenuPanel::new(items, cx));
        self._menu_sub = Some(cx.subscribe_in(
            &panel,
            window,
            |this, _, event: &MenuEvent, window, cx| match event {
                // Dispatch through the window (deferred, like every other button) so
                // the action runs after this update releases the view. A message menu's
                // target stands for exactly that dispatch: the deferred clear below is
                // queued after it. Flows that outlive it (pickers, dialogs) capture their
                // ids when they open.
                MenuEvent::Run(action) => {
                    let target = this.menu.as_ref().and_then(|m| m.target.clone());
                    this.close_menu(window, cx);
                    this.menu_target = target;
                    window.dispatch_action(action.boxed_clone(), cx);
                    cx.defer_in(window, |this, _, _| this.menu_target = None);
                }
                MenuEvent::Cancel => this.close_menu(window, cx),
            },
        ));
        window.focus(&panel.focus_handle(cx), cx);
        self.menu = Some(OpenMenu {
            kind,
            panel: panel.clone(),
            target,
            height,
        });
        cx.notify();
    }

    pub(super) fn close_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu.take().is_none() {
            return;
        }
        self._menu_sub = None;
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// An invisible probe that records its parent's window bounds under `kind` every frame,
    /// so a menu opened from that trigger can hang under it. Add it as a child of a
    /// `relative()` wrapper around the trigger.
    pub(super) fn anchor_probe(&self, kind: MenuKind) -> impl IntoElement {
        let anchors = self.anchors.clone();
        canvas(
            move |bounds, _, _| {
                anchors.borrow_mut().insert(kind, bounds);
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full()
    }

    /// Actions for one message: triage, then the secondary actions. Every row acts on `id`
    /// alone (per-message state, like the model's invariant), never on the cursor.
    fn message_items(&self, id: MessageId) -> Vec<MenuItem> {
        let Some(msg) = self.mailbox.get(id) else {
            return Vec::new();
        };
        let mut items = triage_items(&[msg.state]);
        if !self.mailbox.pending(id).is_empty() {
            items.push(MenuItem::separator());
            items.push(MenuItem::action(
                "btn-accept",
                "Accept AI suggestions",
                "y",
                || Box::new(AcceptSuggestions),
            ));
            items.push(MenuItem::action(
                "btn-reject",
                "Reject AI suggestions",
                "n",
                || Box::new(RejectSuggestions),
            ));
        }
        items.push(MenuItem::separator());
        items.push(MenuItem::action("btn-file", "File…", "f", || Box::new(File)));
        items.push(MenuItem::action("btn-spam", "Mark spam…", "!", || Box::new(MarkSpam)));
        if self.can_toggle_select(id) {
            items.push(MenuItem::action("btn-select", "Toggle select", "x", || Box::new(ToggleSelect)));
        }
        items.push(MenuItem::separator());
        items.push(MenuItem::action(
            "btn-summarize",
            "Summarize thread",
            "z",
            || Box::new(SummarizeThread),
        ));
        items.push(MenuItem::action("btn-mute", "Mute thread", "m", || Box::new(MuteThread)));
        items.push(MenuItem::action(
            "btn-unsubscribe",
            format!("Unsubscribe {}", sender_domain(msg)),
            "shift-u",
            || Box::new(Unsubscribe),
        ));
        items.push(MenuItem::separator());
        items.push(MenuItem::submenu(
            "btn-sender-actions",
            "Sender actions",
            vec![
                MenuItem::action(
                    "btn-sender-archive",
                    "Archive from sender",
                    "shift-e",
                    || Box::new(SenderArchive),
                ),
                MenuItem::action(
                    "btn-sender-delete",
                    "Delete from sender",
                    "shift-d",
                    || Box::new(SenderDelete),
                ),
                MenuItem::action(
                    "btn-sender-file",
                    "File from sender",
                    "shift-f",
                    || Box::new(SenderFile),
                ),
                MenuItem::action(
                    "btn-sender-inbox",
                    "Move sender to inbox",
                    "shift-i",
                    || Box::new(SenderInbox),
                ),
            ],
        ));
        items
    }

    /// Actions for the selected messages: only what makes sense for several at once.
    fn selection_items(&self) -> Vec<MenuItem> {
        let states: Vec<TriageState> = self
            .triage
            .selected()
            .iter()
            .filter_map(|id| self.mailbox.get(*id))
            .map(|m| m.state)
            .collect();
        if states.is_empty() {
            return Vec::new();
        }
        let mut items = triage_items(&states);
        items.push(MenuItem::separator());
        items.push(MenuItem::action("btn-file", "File…", "f", || Box::new(File)));
        items.push(MenuItem::action("btn-spam", "Mark spam…", "!", || Box::new(MarkSpam)));
        items
    }

    /// Whether `x` can toggle `id` in the selection: it is a row of the state list.
    fn can_toggle_select(&self, id: MessageId) -> bool {
        self.mode == ListMode::State && !self.in_session() && self.visible_ids().contains(&id)
    }

    /// Toggle `ids` in the selection: all selected already → deselect them, else select them.
    pub(super) fn toggle_select_ids(&mut self, ids: &[MessageId]) {
        let mut selected = self.triage.selected();
        if ids.iter().all(|id| selected.contains(id)) {
            selected.retain(|id| !ids.contains(id));
        } else {
            let fresh: Vec<MessageId> = ids.iter().copied().filter(|id| !selected.contains(id)).collect();
            selected.extend(fresh);
        }
        self.triage.set_selection(selected);
    }
}

/// Archive / Delete / Snooze… / Inbox, leaving out a move that would change nothing for
/// any of the messages in `states`.
fn triage_items(states: &[TriageState]) -> Vec<MenuItem> {
    let moves = |to: TriageState| states.iter().any(|s| *s != to);
    let mut items = Vec::new();
    if moves(TriageState::Archived) {
        items.push(MenuItem::action("btn-archive", "Archive", "e", || Box::new(Archive)));
    }
    if moves(TriageState::Deleted) {
        items.push(MenuItem::action("btn-delete", "Delete", "d", || Box::new(Delete)));
    }
    items.push(MenuItem::action("btn-snooze", "Snooze…", "s", || Box::new(OpenSnoozePicker)));
    if moves(TriageState::Inbox) {
        items.push(MenuItem::action("btn-inbox", "Move to inbox", "i", || Box::new(MoveToInbox)));
    }
    items
}

/// Rough panel height for clamping: 24px rows, 9px separators, the panel's padding, capped
/// at its `max_h`.
fn menu_height(items: &[MenuItem]) -> f32 {
    let rows: f32 = items
        .iter()
        .map(|i| if matches!(i, MenuItem::Separator) { 9. } else { 24. })
        .sum();
    (rows + 10.).min(360.)
}

/// The quiet header's overflow: everything that is not about the current message.
fn global_items() -> Vec<MenuItem> {
    vec![
        MenuItem::action(
            "btn-palette",
            "Commands…",
            "cmd-k",
            || Box::new(ToggleCommandPalette),
        ),
        MenuItem::separator(),
        MenuItem::action("btn-undo", "Undo", "u", || Box::new(Undo)),
        MenuItem::action(
            "btn-classify",
            "Classify visible mail",
            "c",
            || Box::new(ClassifyVisible),
        ),
        MenuItem::separator(),
        MenuItem::action("btn-rules", "Sender rules…", "shift-r", || Box::new(ToggleRules)),
        MenuItem::action("btn-settings", "Settings…", "cmd-,", || Box::new(ToggleSettings)),
        MenuItem::action("btn-help", "Shortcuts", "?", || Box::new(ToggleHelp)),
    ]
}

/// Short sender label for a menu row, e.g. `"acme.com"`.
fn sender_domain(msg: &Message) -> String {
    msg.from_email
        .split('@')
        .next_back()
        .unwrap_or(&msg.from_email)
        .to_owned()
}
