# Dock refinements: honest drop affordances, and tabs on the message list

Follows `docs/dock-migration.md`. Branch `dock-dnd`. Nothing here is implemented yet.

## 1 + 2. Stop advertising drops that do nothing

Today every group in the dock draws the dock's drop preview — the split-placement
indicator — whenever a message drag hovers it, because that is what the skin does for any
`AnyDrag`. Our handler then ignores everything but the reader group
(`MailApp::drop_on_reader`, `src/app/mail_app/drag.rs`; the guard is
`Dock::is_reader_node`, `src/app/mail_app/dock.rs`). So two panes lie about accepting mail:

| Group | What it shows | What it does |
|---|---|---|
| list | split preview, "drop to make a new pane" | nothing |
| sidebar | same | nothing (until S4, and then the affordance should be per row, not per group) |

**Why we cannot just lock those two groups.** Droppability is `!is_locked()`
(`gpui-base-0.7.0/src/dock/tab_group.rs:458`), and `is_locked()` is "constraints, or zoomed"
(`:443`). Constraints are set by the container, not the app: `TabGroup::set_constraints` is
`pub(crate)` (`:357`); the `TabGroupConstraints` builders (`:69`) are public but nothing
app-facing consumes them per group. The only app-facing lock is dock-wide —
`DockArea::set_locked` (`dock_area.rs:217`) — and that would also stop the reader accepting
drops, which is the one thing we want.

**The lever we do have is the renderer.** Base installs its drop handling on whatever
`TabGroupRenderer::content_frame` returns (`tab_group.rs:917`) and draws the preview through
`TabGroupRenderer::render_drop_indicator` (`:946`, default `None` — `DockSkin` is what
overrides it into the split preview). Both traits are public, and `TabGroupContext` exposes
what a renderer needs to tell the groups apart: `panels()`, `active_ix()`, `drop_indicator()`,
`is_droppable()`, `node()` (`:789-845`).

Plan: give the app its own `DockAreaRenderer` + `TabGroupRenderer`.

- Identify the group by its panels — "holds a panel named `mail.reader`" is already how
  `Dock::is_reader_node` identifies it for drops.
- Reader group: draw our own affordance, a plain accent outline on the group, and *not* a
  split preview — a message drop never splits a pane here, so the placement geometry the dock
  computes is meaningless to us. Also the natural place to show "open here" on a specific tab
  slot later.
- List and sidebar groups: no indicator, no drop styling. Panel/tab drags (the dock's own
  rearrangement) are a different payload and keep working.
- Same change fixes two things we already owe: the strip is then always ours, so a single open
  tab draws our tab bar again instead of the skin's plain title bar (migration gap #1), and the
  skin's `⋯` button goes away.
- Cost: the renderer also owns the tab-level drag, because base does not enforce it
  (`gpui-component-0.7.0/src/dock/tab_panel.rs:943`). The hooks are public —
  `TabGroupContext::drag_panel` / `drop_panel` / `drop_item` / `select_tab` / `close` /
  `toggle_zoom` (`tab_group.rs:849-890`) — so this is lifting the skin's logic onto our own
  chrome, with tab contents still coming from `Panel::title()` as they do now.

Cheaper interim, if we would rather not write a renderer yet: make the sidebar honest early by
giving rows their own drop targets (S4). That still leaves the group-level preview on the
sidebar, and does nothing for the list.

## 3. Tabs on the message list

The list pane becomes a tab group like the reader, so different mailboxes and folders sit side
by side. Same temporary/pinned semantics as message tabs.

**What a tab is.** Almost all of a list view already exists as one app-global set:

| State | Where it lives now |
|---|---|
| location + filter (the `in:`/`account:` part of the query) | `Triage::query` (`src/model/triage.rs:5`), read back as `location()` (`src/app/mail_app/filters.rs:63`) |
| model cursor and selection | `Triage::cursor` / `Triage::selected` (`triage.rs:6-7`) |
| view cursor and anchor | `MailApp::row_cursor` / `row_anchor` (`mod.rs:123-124`) |
| scroll and row shape | `MailApp::list_state` / `list_shape` / `list_visible_end` (`mod.rs:144-147`) |

So a list tab is that bundle — call it a `ListView` — and the feature is: hold
`list_tabs: ListTabs` with one `ListView` per tab instead of one of each field. `ListTabs`
mirrors `src/tabs.rs`: one preview tab plus pinned tabs, with
`open`/`activate`/`cycle`/`close`/`reorder`/`reorder_to`, keyed by location + query rather than
by thread. `MailApp` keeps a `triage()`/`triage_mut()` pair that reaches the *active* tab, so
the 45 existing `self.triage` call sites across 11 files stay mechanical.

**Panels.** One `ListPanel` entity per list tab, exactly as `ReaderPanel` works now, so the
dock's tab group draws the strip and S3's mirror (`Dock::sync` → `DraggedTabs`) applies to the
list group unchanged.

**Behaviour to settle**

- Sidebar click opens the location in the list strip's *preview* tab, replacing it — the same
  rule that opens a message in the reader's preview. Cmd-click or double-click pins.
- The sidebar highlight follows the active list tab, not the last click.
- `cmd-w` currently closes the reader tab. With two strips it has to be decided: either
  "close the active tab of the focused pane" (the dock already routes focus to the active
  panel, so this is well defined), or `cmd-w` stays the reader's and the list gets its own key.
- A triage session takes over the active list tab (the tab says so) and restores its location
  when the session ends; `session: Option<Session>` moves into the tab.
- Switching a list tab does not touch the reader's tabs — two panes, two strips.
- Counts and the status bar follow the active tab.
- Dropping a message on a list tab means "file it under that tab's location" — cheap and very
  natural once the strip exists; extends S2.

## Measured notes

- Panels observe the app because they are separate entities now. That is the same amount of
  re-rendering as before — the panes used to be rebuilt inside `MailApp::render` on exactly the
  same notifications. An idle app sits at ~0.9% CPU (sampled twice, 18s apart), so there is no
  render loop; the thing to watch is a panel render that notifies the app, which nothing does.
- `alt-l` rebuilds the centre split. Per keypress in the headless harness: ~12.5 ms for an
  ordinary keypress (cursor move) versus ~18.7 ms for a toggle, so the rebuild adds ~6 ms at
  keypress rate, not frame rate, and the open tab survives it. Worth revisiting only if the kit
  grows a way to re-axis a split in place.

## Open questions

1. `cmd-w` on two strips: focused pane, or a separate key for list tabs?
2. Does a triage session take over the active list tab, or always the preview tab?
3. Should a sidebar click ever open a *pinned* tab directly (e.g. middle-click), or is the
   preview-then-pin flow enough?
