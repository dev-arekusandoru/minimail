//! Menu wiring for the root view: what each menu holds right now, and how a choice
//! is dispatched.
//!
//! Every entry is an ordinary [`Action`], so a menu click, a submenu click and the
//! keyboard shortcut all reach the same handler on the root view. The menus themselves are
//! the kit's `PopupMenu`, opened by a dropdown trigger button.

use super::*;

use crate::app::menu::{MenuItem, MenuRun, populate};
use crate::judge::urgency_label;
use gpui_kit::component::Selectable as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::menu::DropdownMenu as _;
use gpui_kit::component::menu::PopupMenu;

/// Which menu a trigger belongs to; decides what it holds and which trigger shows itself as
/// open.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum MenuKind {
    /// Overflow of the quiet header.
    Global,
    /// Actions for one message of the reader; they apply to that message only.
    Message(MessageId),
    /// The classifier labels of one message, opened from its row's icons or a right-click.
    Classifier(MessageId),
    /// Actions for the list's selection.
    Selection,
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
        let owner = cx.weak_entity();
        let on_open = cx.weak_entity();
        trigger
            .selected(self.menu_is(kind))
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, window, cx| {
                Self::populate_menu(owner.clone(), kind, menu, window, cx)
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

    /// Fill `menu` with the rows `kind` holds right now. Any trigger can use this as its builder:
    /// a button's `dropdown_menu`, a row's `context_menu`, or a chip's.
    pub(super) fn populate_menu(
        owner: WeakEntity<Self>,
        kind: MenuKind,
        menu: PopupMenu,
        window: &mut Window,
        cx: &mut Context<PopupMenu>,
    ) -> PopupMenu {
        let Some(view) = owner.upgrade() else {
            return menu;
        };
        let (items, target) = view.read(cx).menu_items(kind);
        Self::populate_items(owner, items, target, menu, window, cx)
    }

    /// Fill `menu` with `items`, dispatching every choice back to `owner`.
    pub(super) fn populate_items(
        owner: WeakEntity<Self>,
        items: Vec<MenuItem>,
        target: Option<Vec<MessageId>>,
        menu: PopupMenu,
        window: &mut Window,
        cx: &mut Context<PopupMenu>,
    ) -> PopupMenu {
        let run: MenuRun = Rc::new(move |action, window, cx| {
            owner
                .update(cx, |this, cx| this.run_menu_action(action, target.clone(), window, cx))
                .ok();
        });
        populate(menu, &items, &run, window, cx)
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
            MenuKind::Message(id) => (self.message_items(id), Some(vec![id])),
            MenuKind::Classifier(id) => (self.classifier_items(id), Some(vec![id])),
            MenuKind::Selection => (self.selection_items(), None),
        }
    }

    /// The whole classifier menu of one message: Kind, Urgency, spam, then a pending folder
    /// suggestion.
    pub(super) fn classifier_items(&self, id: MessageId) -> Vec<MenuItem> {
        let mut items = self.kind_items(id);
        for section in [self.urgency_items(id), self.spam_items(id), self.folder_items(id)] {
            if !section.is_empty() {
                items.push(MenuItem::separator());
                items.extend(section);
            }
        }
        items
    }

    /// The Kind section: the five kinds, "Reset to suggestion", then "None" (cleared).
    pub(super) fn kind_items(&self, id: MessageId) -> Vec<MenuItem> {
        let current = self.kind_of(id);
        let mut items = vec![MenuItem::header("Kind")];
        for kind in Kind::ALL {
            items.push(
                MenuItem::action_fn(Self::kind_name(kind), move || {
                    Box::new(SetKind { message: Some(id), kind: Some(kind) })
                })
                .checked(current == Some(kind)),
            );
        }
        items.push(MenuItem::action_fn("Reset Kind to suggestion", move || {
            Box::new(ResetLabel { message: Some(id), key: QuestionKey::Kind })
        }));
        items.push(
            MenuItem::action_fn("No kind", move || Box::new(SetKind { message: Some(id), kind: None }))
                .checked(current.is_none()),
        );
        items
    }

    /// The Urgency section: levels 1..=5 by name, then "Reset to suggestion".
    pub(super) fn urgency_items(&self, id: MessageId) -> Vec<MenuItem> {
        let current = self.urgency_of(id);
        let mut items = vec![MenuItem::header("Urgency")];
        for level in 1..=5 {
            items.push(
                MenuItem::action_fn(urgency_label(level), move || {
                    Box::new(SetUrgency { message: Some(id), level })
                })
                .checked(current == level),
            );
        }
        items.push(MenuItem::action_fn("Reset Urgency to suggestion", move || {
            Box::new(ResetLabel { message: Some(id), key: QuestionKey::Urgency })
        }));
        items
    }

    /// The spam section: one row, which way round depends on the flag. Spam never moves mail.
    pub(super) fn spam_items(&self, id: MessageId) -> Vec<MenuItem> {
        let spam = self.mailbox.tags(id).contains(&Tag::PossibleSpam);
        vec![if spam {
            MenuItem::action_fn("Not spam", move || Box::new(SetSpam { message: Some(id), spam: false }))
        } else {
            MenuItem::action_fn("Mark as spam", move || {
                Box::new(SetSpam { message: Some(id), spam: true })
            })
        }]
    }

    /// The folder section, empty unless a folder suggestion is waiting on `id`.
    pub(super) fn folder_items(&self, id: MessageId) -> Vec<MenuItem> {
        let Some(folder) = self.suggested_folder(id) else {
            return Vec::new();
        };
        let path = self.mailbox.folder_path(folder);
        vec![
            MenuItem::action_fn(format!("Accept folder \"{path}\""), || Box::new(AcceptSuggestions)),
            MenuItem::action("Reject", || Box::new(RejectSuggestions)),
        ]
    }

    /// The Kind tag on `id`, if it carries one.
    fn kind_of(&self, id: MessageId) -> Option<Kind> {
        self.mailbox.tags(id).iter().find_map(|tag| match tag {
            Tag::Kind(kind) => Some(*kind),
            _ => None,
        })
    }

    /// The Urgency level on `id`: 1 when it carries no urgency tag.
    fn urgency_of(&self, id: MessageId) -> u8 {
        self.mailbox
            .tags(id)
            .iter()
            .find_map(|tag| match tag {
                Tag::Urgent(level) => Some(*level),
                _ => None,
            })
            .unwrap_or(1)
    }

    /// A kind's row-menu name, from the one icon table.
    fn kind_name(kind: Kind) -> &'static str {
        Glyph::of_kind(kind).spec().label
    }

    /// The folder a pending Folder suggestion points at, if one is waiting on `id`.
    fn suggested_folder(&self, id: MessageId) -> Option<FolderId> {
        self.mailbox
            .pending(id)
            .into_iter()
            .find(|s| s.key == QuestionKey::Folder)
            .and_then(|s| self.mailbox.suggested_folder(s))
    }

    /// Actions for one message: triage, then the secondary actions. Every row acts on `id`
    /// alone (per-message state, like the model's invariant), never on the cursor.
    fn message_items(&self, id: MessageId) -> Vec<MenuItem> {
        let Some(msg) = self.mailbox.get(id) else {
            return Vec::new();
        };
        let mut items = triage_items(&[msg.state]);
        items.push(MenuItem::separator());
        items.extend(self.classifier_items(id));
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
            .triage()
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
        !self.in_session() && self.visible_ids().contains(&id)
    }

    /// Toggle `ids` in the selection: all selected already → deselect them, else select them.
    pub(super) fn toggle_select_ids(&mut self, ids: &[MessageId]) {
        let mut selected = self.triage().selected();
        if ids.iter().all(|id| selected.contains(id)) {
            selected.retain(|id| !ids.contains(id));
        } else {
            let fresh: Vec<MessageId> = ids.iter().copied().filter(|id| !selected.contains(id)).collect();
            selected.extend(fresh);
        }
        self.triage_mut().set_selection(selected);
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
        MenuItem::action("Classify every message in this list…", || Box::new(ClassifyVisible)),
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
