# Dock migration + drag & drop — design spec

Branch `dock-dnd`, worktree `~/Programming/mail-classifier.dock-dnd`, cut from `main`.

Goal: move the pane layout onto `gpui_kit::component::dock` and add drag & drop, with the app
looking and feeling the same. Prototype scope: S0 + S1 + S2 below.

## 1. Why the dock is the only path to dock-quality DnD

- A dock `TabGroup` is the only drop surface that reports foreign drags: base attaches
  `on_drag_move`/`on_drop` for `AnyDrag` to the group content frame itself
  (`gpui-base-0.7.0/src/dock/tab_group.rs:728-741`) and re-emits as
  `DockEvent::DragDrop { item, target }` (`dock/dock_area.rs:44`, emitted `:1112`).
- That machinery is not reachable standalone: `TabGroup::new` is `pub(crate)`
  (`tab_group.rs:165`), `DropTarget::new`/`DropIndicator::new` are `pub(crate)` (`drag.rs:134,165`),
  and the hit-zone function `split_placement_at` is deliberately unexported (`dock/mod.rs:184-187`).
- The styled skin already wires tab drag, reorder and drops with **zero app-side glue**:
  `TabGroupSkin` calls `group.drag_panel`/`drop_panel`/`drop_item`
  (`gpui-component-0.7.0/src/dock/tab_panel.rs:574-652`).

App-facing API (all via the component layer; `gpui-kit-0.7.0/src/lib.rs:133`):

```rust
use gpui_kit::component::dock::{
    BasePanel, DockArea, DockEvent, DockLayout, DockPlacement, DockSkin, Panel, PanelControl,
    PanelEvent, PanelStyle, panel_handle,
};
```

A real app-side usage example ships in the kit: `gpui-kit-0.7.0/tests/dock.rs` (including
`window.within("tab-bar").drag_to(1, 0, cx)` — headless DnD is testable).

## 2. Target layout

```
┌ titlebar ─────────────────────────────────────────────┐
├ sidebar (left dock) │ list (tabs) │ reader (tabs) ─────┤
├ hint bar ─────────────────────────────────────────────┤
```

- `DockSkin::dock_area("mail", None, window, cx)` for the body region; it stays a child of the
  existing root div, in the same slot `render_panes` occupies today
  (`src/app/mail_app/render.rs:237`), under the titlebar and above the hint bar.
- Skin config: `set_panel_style(PanelStyle::TabBar, cx)`,
  `set_close_button_visible(false, cx)` (we draw our own close button inside `Panel::title`, so
  the existing close behaviour and its test survive), `set_toggle_button_visible(false, cx)`.
- Left dock = sidebar, one panel. `set_dock_collapsible(Left, true)`,
  `set_dock_size(Left, px(SIDEBAR_W))`. This region keeps a real size API
  (`dock_area.rs:344-355`) — narrow it with `toggle_dock`, resize with the divider.
- Center = `h_split` (side by side) or `v_split` (stacked) of two tab groups: list, reader.

Panels (all `Entity<T>` implementing `BasePanel` + `Panel` + `EventEmitter<PanelEvent>` +
`Focusable` + `Render`):

| Panel | Content | Chrome |
|---|---|---|
| `SidebarPanel` | today's `render_sidebar` | `title_bar() == false`, not closable, not zoomable |
| `ListPanel` | today's list (`rows.rs`) | `title_bar() == false`, not closable, not zoomable |
| `ReaderPanel` | one open message (today's reader) | tab: avatar + italic-when-preview title + our close button |

## 3. Tab model stays ours

`src/tabs.rs` remains the model of record — preview/pin/cycle/preview-retention semantics and
its 12 tests (`tests/tabs.rs`) are unchanged. The dock panel set mirrors it:

| Model | Dock |
|---|---|
| `Tabs::open` / `open_pinned` | `add_panel_view(panel_handle(reader_panel), ..)` |
| `Tabs::close` | `remove_panel(PanelId, ..)` |
| `Tabs::activate` / `cycle` | `select_panel(PanelId, ..)` |
| `Tabs::set_msg` (preview follows the cursor) | update the panel entity |
| `Tabs::retain_previews` | remove preview panels whose thread left the list |
| `Tabs::reorder` (new, pure) | `move_panel(id, InsertTarget::Tabs { node, ix, .. })` |

Tab visuals come from `Panel::title()` (`gpui-component-0.7.0/src/dock/panel.rs:82`) — that is
the per-tab element the skin renders, and it is where the monogram, italic preview title and
close button go (`src/app/mail_app/reader_tabbar.rs` is the current look to reproduce).

Panels are **thin proxies**, not owners of state: each holds a `WeakEntity<MailApp>` plus the
identity it renders (a folder, a thread), and its `Render`/`title` call back into the existing
`MailApp` render helpers. The mailbox, selection, tabs and reader state stay exactly where they
are, so look and feel (and the existing render code) survive the migration.

## 4. Parity: what the dock cannot express

| Today | Dock | Handling |
|---|---|---|
| `size_range(px(MIN_LIST_W)..)` etc. (`panes.rs:225-260`) | slots built without `size_range` (`dock_area.rs:1310-1335`) | sidebar keeps a real size API (clamp on `LayoutChanged` + `set_dock_size`); **center split has no minimum** → accept for the prototype |
| `grow/shrink_list_pane`, double-click divider reset (`panes.rs:175-212`) | no programmatic divider move; `tree_mut` private, `render_split_handle` is appearance-only | sidebar: native. Center: rebuild `set_center` with explicit sizes from `dump(cx).center` sizes, or drop the feature |
| Two orientations, each remembering its size (`ui_panes.rs:199`) | one `PaneTree` per region, no reaxis | hold two `DockAreaState`s (dump/load) and swap on toggle |
| Sidebar hidden keeps its width (`ui_panes.rs:257`) | `set_dock_collapsible` + `toggle_dock`; size survives collapse | native |
| Keyboard-first focus; bindings under `MAIL_SCOPE` (`actions.rs:180`) | dock tracks focus on area/groups and routes it to the active panel (`dock_area.rs:1468`, `tab_group.rs:696`) | keep our actions on the root; verify bare-letter bindings still fire when a panel holds focus. No kit key binding collides with `cmd-w`/`alt-l` (checked: the kit registers none for `ClosePanel`/`ToggleZoom`) |
| Session mode: `list = None` (`render_panes`) | a hidden panel drops its whole slot | keep the list panel and make it `visible() == false` during a session |

Not required: layout persistence. Pane sizes are session-only today (nothing in `src/prefs.rs`
stores them), so `register_panel`/`state_convert` is out of scope until we want it.

## 5. Drag & drop design

Payload — one type, dragged from any list row:

```rust
#[derive(Clone)]
pub struct MailDrag {
    pub ids: Vec<MessageId>,   // snapshot, from target_ids()
    pub thread: u32,           // the row's thread, for the tab-strip target
    pub anchor: MessageId,     // cursor message
    pub pinned: bool,          // modifier held at drag start -> open_pinned
}
```

- Snapshot at drag start from `target_ids()` (`src/app/mail_app/accessors.rs:191`), which already
  encodes the chosen rule (menu target → grouped cursor row → session cursor → selection).
- Wrapped as `AnyDrag::new(MailDrag { .. })` — required for a dock tab group to accept it
  (`dock/drag.rs:109`).
- Preview: a small view (`Render`) drawing the sender/subject and `+N` when multi.

Sources: `rows.rs:154` (message rows) and `grouping.rs:276` (thread headers) via
`.on_drag(AnyDrag::new(payload), ctor)`. GPUI's 2px `DRAG_THRESHOLD` (`gpui-pre-0.3.7/src/elements/div.rs:52`)
keeps click and drag from fighting.

Targets:

1. **Reader tab strip** (S2) — `DockEvent::DragDrop { item, .. }` → open the message an `AnyDrag`
   carries, pinned when the modifier was held (`Tabs::open`/`open_pinned`, `src/tabs.rs:62,80`).
   A `Some(placement)` on the event means an edge drop: ignore for the prototype.
2. **Sidebar folder / Archive / Trash / Snoozed rows** (S4) — need our own rows, because
   `SidebarMenuItem` exposes no drag hook (`gpui-component-0.7.0/src/sidebar/menu.rs:94-161`).
   Drop → `mailbox.set_state(&ids, TriageState::Filed(folder.id))` (`model/mod.rs:36`).
   Snoozed reuses the snooze popover for the wake time.
3. **Tab reorder** (S3) — free from the skin, but must be mirrored into `Tabs` so the model and
   the strip cannot disagree.
4. **Finder → composer**, **drag out to Finder** (S5/S6) — `external_drag_payload`
   (`div.rs:645`), later.

Each drop is one user action = one undo step: go through the same tail as `mark()`
(`actions.rs:84`) — `set_state` → `clear_selection` → toast → `session_advance`. A drop that
moves nothing (same folder) must not push undo or toast.

## 6. Test strategy

- Pure logic: `Tabs::reorder`, `MailDrag` construction from selection, drop-target →
  `TriageState` mapping (`tests/dnd.rs`, `tests/tabs.rs`).
- UI: headless drags. `TestWindowExt::drag(from, to, cx)` and `drag_to(from_id, to_id, cx)`
  (`gpui-kit-0.7.0/src/test.rs:43-45`) drive a whole gesture; the pane tests already use
  `window.drag(from, to, cx)` (`tests/ui_panes.rs:86-96`). A drag with no drop target has no
  observable effect, so the drag source itself is proven end to end by the drop tests below
  rather than by a test of its own.
- `tests/ui_panes.rs`: keep the behaviour the dock expresses (sidebar clamp/reset/collapse,
  orientation memory via the two dumps, rows spanning the pane); rewrite or delete the ones it
  cannot (`drag_is_clamped_to_usable_panes`, `keyboard_shrink_stops_at_the_minimum`,
  `double_click_on_the_divider_resets_the_size`) — with the loss stated, not silently.

## 7. Slices

| # | Slice | Acceptance |
|---|---|---|
| S0 | Dock migration: three panels, `DockSkin`, orientation toggle, sidebar collapse | `cargo build`, `clippy -D warnings`, `test` green; app runs with the same three panes |
| S1 | `MailDrag` + drag source on rows and thread headers | drag shows the preview chip, no behaviour change on click |
| S2 | Drop on the reader tab group opens a tab | dropping a row on the strip opens/pins it, `cmd-w` still closes |
| S3 | Tab reorder mirrored into `Tabs` | reordering keeps model and strip in sync |
| S4 | Sidebar rows rewritten as our own + folder drops | drop on a folder files the messages, one undo step |
| S5 | Finder → composer attach | files dropped on the window attach to the draft |

## 8. Open questions

1. Center-split minimums: accept the loss, or clamp by rebuilding `set_center` from `dump()`?
2. Do we keep `GrowListPane`/`ShrinkListPane`/`ResetPanes` (no dock API for center dividers)?
   The sidebar equivalents can stay.
3. Should the tab strip's own close button (skin) stay hidden in favour of ours, or do we adopt
   the skin's chrome and drop ours?
