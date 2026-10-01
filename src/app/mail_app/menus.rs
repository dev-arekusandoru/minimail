//! Menu wiring for the root view: what each menu holds right now, and how a choice
//! is dispatched.
//!
//! Every entry is an ordinary [`Action`], so a menu click, a submenu click and the
//! keyboard shortcut all reach the same handler on the root view. The menus themselves are
//! the kit's `PopupMenu`, opened by a dropdown trigger button.

use super::*;

use crate::app::menu::{MenuItem, MenuRun, populate};
use gpui_kit::component::Selectable as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::menu::DropdownMenu as _;

/// Which menu a trigger belongs to; decides what it holds and which trigger shows itself as
/// open.
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

impl MailApp {
    /// Turn `trigger` into the opener of the `kind` menu, hanging right-aligned under it. The
    /// menu's rows are built when it opens, so they reflect the state at that moment.
    pub(super) fn menu_trigger(
        &self,
        kind: MenuKind,
        trigger: Button,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let app = cx.weak_entity();
        let on_open = cx.weak_entity();
        trigger
            .selected(self.menu_is(kind))
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, window, cx| {
                let Some(view) = app.upgrade() else { return menu };
                let (items, target) = view.read(cx).menu_items(kind);
                let owner = app.clone();
                let run: MenuRun = Rc::new(move |action, window, cx| {
                    let target = target.clone();
                    owner
                        .update(cx, |this, cx| this.run_menu_action(action, target, window, cx))
                        .ok();
                });
                populate(menu, &items, &run, window, cx)
            })
            .on_open_change(move |open, _, cx| {
                on_open
                    .update(cx, |this, cx| {
                        this.open_menu = open.then_some(kind);
                        cx.notify();
                    })
                    .ok();
            })
    }

    /// Dispatch a chosen row's action through the window (deferred, like every other button)
    /// so it runs after this update releases the view. A message menu's `target` stands for
    /// exactly that dispatch: the deferred clear below is queued after it. Flows that outlive
    /// it (pickers, dialogs) capture their ids when they open.
    fn run_menu_action(
        &mut self,
        action: Box<dyn Action>,
        target: Option<Vec<MessageId>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = target.as_ref().and_then(|ids| ids.first()) {
            self.pin_thread_of(*id);
        }
        self.menu_target = target;
        window.dispatch_action(action, cx);
        cx.defer_in(window, |this, _, _| this.menu_target = None);
    }

    /// What the `kind` menu holds right now, and the messages its actions apply to (`None`:
    /// the usual cursor/selection targets).
    pub(super) fn menu_items(&self, kind: MenuKind) -> (Vec<MenuItem>, Option<Vec<MessageId>>) {
        match kind {
            MenuKind::Global => (global_items(), None),
            MenuKind::Filter => (self.filter_items(), None),
            MenuKind::Message(id) => (self.message_items(id), Some(vec![id])),
            MenuKind::Selection => (self.selection_items(), None),
        }
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
            items.push(MenuItem::action("Accept AI suggestions", || Box::new(AcceptSuggestions)));
            items.push(MenuItem::action("Reject AI suggestions", || Box::new(RejectSuggestions)));
        }
        items.push(MenuItem::separator());
        items.push(MenuItem::action("File…", || Box::new(File)));
        items.push(MenuItem::action("Mark spam…", || Box::new(MarkSpam)));
        if self.can_toggle_select(id) {
            items.push(MenuItem::action("Toggle select", || Box::new(ToggleSelect)));
        }
        items.push(MenuItem::separator());
        items.push(MenuItem::action("Summarize thread", || Box::new(SummarizeThread)));
        items.push(MenuItem::action("Mute thread", || Box::new(MuteThread)));
        items.push(MenuItem::action(format!("Unsubscribe {}", sender_domain(msg)), || {
            Box::new(Unsubscribe)
        }));
        items.push(MenuItem::separator());
        items.push(MenuItem::submenu(
            "Sender actions",
            vec![
                MenuItem::action("Archive from sender", || Box::new(SenderArchive)),
                MenuItem::action("Delete from sender", || Box::new(SenderDelete)),
                MenuItem::action("File from sender", || Box::new(SenderFile)),
                MenuItem::action("Move sender to inbox", || Box::new(SenderInbox)),
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
        items.push(MenuItem::action("File…", || Box::new(File)));
        items.push(MenuItem::action("Mark spam…", || Box::new(MarkSpam)));
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
        items.push(MenuItem::action("Archive", || Box::new(Archive)));
    }
    if moves(TriageState::Deleted) {
        items.push(MenuItem::action("Delete", || Box::new(Delete)));
    }
    items.push(MenuItem::action("Snooze…", || Box::new(OpenSnoozePicker)));
    if moves(TriageState::Inbox) {
        items.push(MenuItem::action("Move to inbox", || Box::new(MoveToInbox)));
    }
    items
}

/// The quiet header's overflow: everything that is not about the current message.
fn global_items() -> Vec<MenuItem> {
    vec![
        MenuItem::action("Commands…", || Box::new(ToggleCommandPalette)),
        MenuItem::separator(),
        MenuItem::action("Undo", || Box::new(Undo)),
        MenuItem::action("Classify visible mail", || Box::new(ClassifyVisible)),
        MenuItem::separator(),
        MenuItem::action("Sender rules…", || Box::new(ToggleRules)),
        MenuItem::action("Settings…", || Box::new(ToggleSettings)),
        MenuItem::action("Shortcuts", || Box::new(ToggleHelp)),
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
