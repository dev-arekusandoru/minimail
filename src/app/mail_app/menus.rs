//! Menu wiring for the root view: what each menu holds right now, and how a choice
//! is dispatched.
//!
//! Every entry is an ordinary [`Action`], so a menu click, a submenu click and the
//! keyboard shortcut all reach the same handler on the root view.

use super::*;

use crate::app::menu::{MenuEvent, MenuItem, MenuPanel};

/// Which menu the open panel belongs to; decides where it is anchored and which
/// trigger shows itself as open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MenuKind {
    /// Overflow of the quiet header.
    Global,
    /// Secondary actions for the message under the cursor.
    Message,
}

/// An open menu: which one it is, and the panel that draws it.
pub(super) struct OpenMenu {
    pub kind: MenuKind,
    pub panel: Entity<MenuPanel>,
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
        let items = match kind {
            MenuKind::Global => global_items(),
            MenuKind::Message => {
                let items = self.message_items();
                if items.is_empty() {
                    return;
                }
                items
            }
        };
        if items.is_empty() {
            return;
        }
        let panel = cx.new(|cx| MenuPanel::new(items, cx));
        self._menu_sub = Some(cx.subscribe_in(
            &panel,
            window,
            |this, _, event: &MenuEvent, window, cx| match event {
                // Dispatch through the window (deferred, like every other button) so
                // the action runs after this update releases the view.
                MenuEvent::Run(action) => {
                    this.close_menu(window, cx);
                    window.dispatch_action(action.boxed_clone(), cx);
                }
                MenuEvent::Cancel => this.close_menu(window, cx),
            },
        ));
        window.focus(&panel.focus_handle(cx), cx);
        self.menu = Some(OpenMenu {
            kind,
            panel: panel.clone(),
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

    /// Secondary actions for the message under the cursor (or the session's message).
    fn message_items(&self) -> Vec<MenuItem> {
        let Some(msg) = self.cursor_id().and_then(|id| self.mailbox.get(id)) else {
            return Vec::new();
        };
        let mut items = vec![
            MenuItem::action("btn-select", "Toggle select", "x", || Box::new(ToggleSelect)),
            MenuItem::separator(),
            MenuItem::action(
                "btn-summarize",
                "Summarize thread",
                "s",
                || Box::new(SummarizeThread),
            ),
            MenuItem::action("btn-mute", "Mute thread", "m", || Box::new(MuteThread)),
            MenuItem::action(
                "btn-unsubscribe",
                format!("Unsubscribe {}", sender_domain(msg)),
                "shift-u",
                || Box::new(Unsubscribe),
            ),
        ];
        items.push(MenuItem::separator());
        items.push(MenuItem::submenu(
            "btn-sender-actions",
            "Sender actions",
            vec![
                MenuItem::action(
                    "btn-sender-done",
                    "All done",
                    "shift-e",
                    || Box::new(SenderDone),
                ),
                MenuItem::action(
                    "btn-sender-waiting",
                    "All waiting",
                    "shift-w",
                    || Box::new(SenderWaiting),
                ),
                MenuItem::action(
                    "btn-sender-inbox",
                    "All to inbox",
                    "shift-i",
                    || Box::new(SenderInbox),
                ),
                MenuItem::action(
                    "btn-sender-later",
                    "All later",
                    "shift-l",
                    || Box::new(SenderLater),
                ),
            ],
        ));
        items
    }
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
