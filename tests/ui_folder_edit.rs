//! Renaming and deleting folders from the sidebar: the row's right-click menu, the palette
//! commands for the folder the list shows, the delete confirmation, and what the list and its
//! tabs do when their folder is renamed or goes.

use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext, ElementId, InputEvent as _, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, TestAppContext, point,
    px,
};
use mail_classifier::model::{Location, Mailbox, TriageState};

#[path = "ui_ext/harness.rs"]
#[allow(dead_code)]
mod harness;
use harness::{Harness, harness_with};

// The work account's fixture folders: Projects (4) > Northwind (5), and Recruiting (6).
const PROJECTS: u32 = 4;
const NORTHWIND: u32 = 5;
const RECRUITING: u32 = 6;

/// Work mail: 1 in the inbox, 2 in Projects, 3 in Northwind (under Projects), 4 in Recruiting.
fn mailbox() -> Mailbox {
    let message = |id: u32, state: serde_json::Value| {
        serde_json::json!({
            "id": id, "thread_id": id, "from_name": "sam", "from_email": format!("sam{id}@example.com"),
            "to": "you@acme.co", "subject": format!("Mail {id}"), "body": "Body",
            "received": format!("2026-09-{id:02}T09:00:00Z"), "state": state, "account": "work",
        })
    };
    let all = [
        message(1, "Inbox".into()),
        message(2, serde_json::json!({ "Filed": PROJECTS })),
        message(3, serde_json::json!({ "Filed": NORTHWIND })),
        message(4, serde_json::json!({ "Filed": RECRUITING })),
    ];
    Mailbox::from_json(&serde_json::Value::from(all.to_vec()).to_string()).expect("valid mailbox json")
}

fn on_work(cx: &mut TestAppContext) -> Harness<'_> {
    let mut h = harness_with(cx, mailbox());
    h.click("sidebar-rail-work");
    h
}

fn name_of(h: &mut Harness<'_>, id: u32) -> Option<String> {
    h.read(|a| a.mailbox.folder(id).map(|f| f.name.clone()))
}

fn folder_ids(h: &mut Harness<'_>) -> Vec<u32> {
    h.read(|a| a.mailbox.folders("work").iter().map(|f| f.id).collect())
}

fn state_of(h: &mut Harness<'_>, id: u32) -> TriageState {
    h.read(|a| a.mailbox.state_of(id).expect("message"))
}

/// Setup steps would otherwise be undone by `u`: drop them so an undo means the action under test.
fn undo_depth(h: &mut Harness<'_>) -> usize {
    h.read(|a| a.mailbox.undo_depth())
}

#[gpui_kit::gpui::test]
fn a_folder_rows_right_click_offers_rename_and_delete(cx: &mut TestAppContext) {
    let mut h = on_work(cx);
    h.right_click("sidebar-folder-6");
    let mut rows = h.rows();
    rows.sort(); // the harness reads rows from an unordered snapshot
    assert_eq!(rows, ["Delete…", "Rename…"]);
}

#[gpui_kit::gpui::test]
fn rename_prefills_the_name_and_enter_renames_in_one_undo_step(cx: &mut TestAppContext) {
    let mut h = on_work(cx);
    let depth = undo_depth(&mut h);

    h.right_click("sidebar-folder-6");
    h.click_row("Rename…");
    assert!(h.read(|a| a.modal_open()), "the name prompt is up");
    // The current name is selected, so typing replaces it.
    h.type_text("Hiring");
    h.keys("enter");

    assert!(!h.read(|a| a.modal_open()));
    assert_eq!(name_of(&mut h, RECRUITING).as_deref(), Some("Hiring"));
    assert!(h.toast().contains("Hiring"), "toast: {}", h.toast());
    assert_eq!(undo_depth(&mut h), depth + 1);

    h.keys("u");
    assert_eq!(name_of(&mut h, RECRUITING).as_deref(), Some("Recruiting"));
}

#[gpui_kit::gpui::test]
fn a_clashing_or_blank_name_keeps_the_prompt_and_escape_cancels(cx: &mut TestAppContext) {
    let mut h = on_work(cx);
    let depth = undo_depth(&mut h);

    h.right_click("sidebar-folder-6");
    h.click_row("Rename…");
    h.type_text("PROJECTS");
    h.keys("enter");
    assert!(h.read(|a| a.modal_open()), "a clash keeps the prompt open");
    assert!(h.toast().contains("already exists"), "toast: {}", h.toast());
    assert_eq!(name_of(&mut h, RECRUITING).as_deref(), Some("Recruiting"));

    h.type_text("   ");
    h.keys("enter");
    assert!(h.read(|a| a.modal_open()), "a blank name keeps it too");

    h.keys("escape");
    assert!(!h.read(|a| a.modal_open()));
    assert_eq!(name_of(&mut h, RECRUITING).as_deref(), Some("Recruiting"));
    assert_eq!(undo_depth(&mut h), depth, "nothing was renamed");
}

#[gpui_kit::gpui::test]
fn renaming_the_folder_the_list_shows_keeps_the_list_on_it(cx: &mut TestAppContext) {
    let mut h = on_work(cx);
    h.click("sidebar-folder-6");
    assert_eq!(h.read(|a| a.location()), Some(Location::Folder(RECRUITING)));
    assert_eq!(h.visible(), vec![4]);

    h.keys("cmd-k");
    h.type_text("Rename folder");
    h.keys("enter");
    h.type_text("Hiring");
    h.keys("enter");

    assert_eq!(name_of(&mut h, RECRUITING).as_deref(), Some("Hiring"));
    assert_eq!(h.read(|a| a.location()), Some(Location::Folder(RECRUITING)));
    assert_eq!(h.visible(), vec![4], "the tab follows the new path, not an empty list");

    h.keys("u");
    assert_eq!(h.visible(), vec![4], "and follows an undone rename back");
}

#[gpui_kit::gpui::test]
fn the_palette_offers_folder_commands_only_inside_a_folder(cx: &mut TestAppContext) {
    let mut h = on_work(cx);
    h.keys("cmd-k");
    h.type_text("folder");
    let rows = h.palette_rows();
    assert!(rows.iter().any(|r| r == "New folder"), "{rows:?}");
    assert!(!rows.iter().any(|r| r.starts_with("Rename folder") || r.starts_with("Delete folder")), "{rows:?}");
    h.keys("escape");
    if h.read(|a| a.modal_open()) {
        h.keys("escape"); // the first one only clears the typed text
    }
    assert!(!h.read(|a| a.modal_open()), "the palette closed");

    h.click("sidebar-folder-6");
    assert_eq!(h.read(|a| a.location()), Some(Location::Folder(RECRUITING)));
    h.keys("cmd-k");
    h.type_text("folder");
    let rows = h.palette_rows();
    assert!(rows.iter().any(|r| r.starts_with("Rename folder")), "{rows:?}");
    assert!(rows.iter().any(|r| r.starts_with("Delete folder")), "{rows:?}");
}

#[gpui_kit::gpui::test]
fn delete_asks_first_and_enter_cancels(cx: &mut TestAppContext) {
    let mut h = on_work(cx);
    let depth = undo_depth(&mut h);

    h.right_click("sidebar-folder-4");
    h.click_row("Delete…");
    assert!(h.read(|a| a.dialog_open()));
    let title = h.app.read_with(h.cx, |a, cx| a.dialog_title(cx)).unwrap();
    assert!(title.contains("Projects"), "{title}");
    let options = h.app.read_with(h.cx, |a, cx| a.dialog_options(cx));
    assert_eq!(options, ["Delete folder", "Cancel"]);

    h.keys("enter");
    assert!(!h.read(|a| a.modal_open()));
    assert_eq!(folder_ids(&mut h), [PROJECTS, NORTHWIND, RECRUITING]);
    assert_eq!(state_of(&mut h, 2), TriageState::Filed(PROJECTS));

    h.right_click("sidebar-folder-4");
    h.click_row("Delete…");
    h.keys("escape");
    assert_eq!(folder_ids(&mut h), [PROJECTS, NORTHWIND, RECRUITING], "escape cancels too");
    assert_eq!(undo_depth(&mut h), depth);
}

#[gpui_kit::gpui::test]
fn deleting_a_folder_archives_the_mail_of_it_and_its_subfolders_and_undo_restores_all(cx: &mut TestAppContext) {
    let mut h = on_work(cx);
    let depth = undo_depth(&mut h);

    h.right_click("sidebar-folder-4");
    h.click_row("Delete…");
    h.keys("1");

    assert!(!h.read(|a| a.modal_open()));
    assert_eq!(folder_ids(&mut h), [RECRUITING], "Projects and Northwind are gone");
    assert_eq!(state_of(&mut h, 2), TriageState::Archived, "mail is archived, never deleted");
    assert_eq!(state_of(&mut h, 3), TriageState::Archived);
    assert_eq!(state_of(&mut h, 4), TriageState::Filed(RECRUITING));
    assert!(!h.has("sidebar-folder-4") && !h.has("sidebar-folder-5"));
    assert_eq!(undo_depth(&mut h), depth + 1, "one user action, one undo step");
    assert!(h.toast().contains("Projects"), "toast: {}", h.toast());

    h.keys("u");
    assert_eq!(folder_ids(&mut h), [PROJECTS, NORTHWIND, RECRUITING]);
    assert_eq!(state_of(&mut h, 2), TriageState::Filed(PROJECTS));
    assert_eq!(state_of(&mut h, 3), TriageState::Filed(NORTHWIND));
    assert_eq!(undo_depth(&mut h), depth);
    assert!(h.has("sidebar-folder-4"));
}

#[gpui_kit::gpui::test]
fn deleting_the_folder_the_list_shows_moves_the_list_to_the_accounts_inbox(cx: &mut TestAppContext) {
    let mut h = on_work(cx);
    // A descendant of the deleted folder is the one on show.
    h.click("sidebar-folder-5");
    assert_eq!(h.read(|a| a.location()), Some(Location::Folder(NORTHWIND)));

    h.keys("cmd-k");
    h.type_text("Delete folder");
    h.keys("enter");
    h.keys("1");

    assert_eq!(folder_ids(&mut h), [PROJECTS, RECRUITING], "only the folder shown and below it go");
    assert_eq!(h.read(|a| a.location()), Some(Location::Inbox("work".into())));
    assert_eq!(h.visible(), vec![1], "the work inbox, not an empty folder list");
}

#[gpui_kit::gpui::test]
fn a_pinned_tab_on_a_deleted_folder_becomes_the_accounts_inbox(cx: &mut TestAppContext) {
    let mut h = on_work(cx);
    h.cmd_click("sidebar-folder-4");
    assert_eq!(h.tabs(), ["in:inbox", "in:projects account:work"]);
    h.click(("list-tab", 0usize));

    h.right_click("sidebar-folder-4");
    h.click_row("Delete…");
    h.keys("1");

    assert_eq!(h.tabs(), ["in:inbox", "in:inbox account:work"], "the tab was retargeted, not left empty");
}

#[gpui_kit::gpui::test]
fn tabs_on_deleted_folders_close_when_the_inbox_is_already_open(cx: &mut TestAppContext) {
    let mut h = on_work(cx);
    h.cmd_click("sidebar-folder-4");
    h.cmd_click("sidebar-folder-5");
    h.cmd_click("sidebar-inbox-work");
    assert_eq!(h.tabs().len(), 4);
    // Four tabs overflow the strip, so tab 0 is not clickable: show it through the model.
    h.app.update(h.cx, |a, cx| {
        a.list_tabs.activate(0);
        cx.notify();
    });

    h.right_click("sidebar-folder-4");
    h.click_row("Delete…");
    h.keys("1");

    assert_eq!(h.tabs(), ["in:inbox", "in:inbox account:work"], "no view is open twice, none lists a gone folder");
    assert_eq!(h.read(|a| a.list_tabs.active_index()), 0, "the tab the user was on stays");

    h.keys("u");
    assert!(h.has("sidebar-folder-4"), "the folders are back; closed tabs stay closed");
}

impl Harness<'_> {
    /// The strip's views, left to right.
    fn tabs(&mut self) -> Vec<String> {
        self.read(|a| a.list_tabs.tabs().iter().map(|t| t.query.describe()).collect())
    }

    /// Click the sidebar row `id` with the command key held, the way the app pins a view.
    fn cmd_click(&mut self, id: impl Into<ElementId>) {
        let id = id.into();
        let bounds = self
            .cx
            .update_window(self.window, |_, window, _| window.find(id).bounds())
            .expect("window alive");
        self.cx.run_until_parked();
        let at = point(
            px(f32::from(bounds.origin.x) + f32::from(bounds.size.width) / 2.),
            px(f32::from(bounds.origin.y) + f32::from(bounds.size.height) / 2.),
        );
        let modifiers = Modifiers { platform: true, ..Default::default() };
        self.cx
            .update_window(self.window, |_, window, cx| {
                window.dispatch_event(
                    MouseDownEvent { button: MouseButton::Left, position: at, modifiers, click_count: 1, first_mouse: false }
                        .to_platform_input(),
                    cx,
                );
                window.dispatch_event(
                    MouseUpEvent { button: MouseButton::Left, position: at, modifiers, click_count: 1 }.to_platform_input(),
                    cx,
                );
            })
            .expect("window alive");
        self.cx.run_until_parked();
    }
}
