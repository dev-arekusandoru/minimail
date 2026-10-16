# Dock refinements: list tabs, a Triage tab, and the drop-affordance question

Follows `docs/dock-migration.md`. Branch `dock-dnd`. Nothing here is implemented yet.

## 1. Drop affordances that lie (accepted for now)

Every dock group draws the split-placement preview when a message drag hovers it, because that
is what the skin does for any `AnyDrag`; our handler then ignores everything but the reader
group (`MailApp::drop_on_reader`, `src/app/mail_app/drag.rs`, guarded by
`Dock::is_reader_node`, `src/app/mail_app/dock.rs`). So the list and the sidebar both look like
they will take a message, and neither does.

Decision: **no custom renderer**, so this stays as it is for now. Recorded because it is not
fixable from the app side:

- Droppability is `!is_locked()` (`gpui-base-0.7.0/src/dock/tab_group.rs:458`), `is_locked()`
  is constraints-or-zoomed (`:443`), and constraints belong to the container:
  `TabGroup::set_constraints` is `pub(crate)` (`:357`). The only app-facing lock is dock-wide
  (`DockArea::set_locked`, `dock_area.rs:217`), which would also stop the reader accepting
  drops.
- The preview is drawn by `TabGroupRenderer::render_drop_indicator` (`tab_group.rs:946`), i.e.
  only a renderer can suppress it — which we have decided against.

What that leaves, if it keeps bothering us:

1. Make the drop *mean* something on the list (file under the active tab's location) rather
   than suppressing the affordance.
2. Give sidebar rows their own drop targets (S4) — the group preview would still show, but the
   drop would at least do what it looks like it will do.

The strip is still the skin's, but all dock groups now use `PanelStyle::TabBar`, including
single-tab groups: the same tab chrome replaces the skin's plain singleton title bar.
Preview italics, sender avatars and our reader close button are unchanged. The sidebar
is outside Dock and keeps its existing presentation.

## 2. Tabs on the message list

The list pane becomes a tab group like the reader, so different mailboxes and folders sit side
by side. Same temporary/pinned semantics as message tabs.

**What a tab is.** Almost all of a list view already exists as one app-global set:

| State | Where it lives now |
|---|---|
| location + filter (the `in:`/`account:` part of the query) | `Triage::query` (`src/model/triage.rs:5`), read back as `location()` (`src/app/mail_app/filters.rs:63`) |
| model cursor and selection | `Triage::cursor` / `Triage::selected` (`triage.rs:6-7`) |
| view cursor and anchor | `MailApp::row_cursor` / `row_anchor` (`mod.rs:123-124`) |
| scroll and row shape | `MailApp::list_state` / `list_shape` / `list_visible_end` (`mod.rs:144-147`) |

So a list tab is that bundle — a `ListView` — and the feature is: hold `list_tabs: ListTabs`
with one `ListView` per tab instead of one of each field. `ListTabs` mirrors `src/tabs.rs`: one
preview tab plus pinned tabs, with `open`/`activate`/`cycle`/`close`/`reorder`/`reorder_to`,
keyed by location + query rather than by thread. `MailApp` keeps a `triage()`/`triage_mut()`
pair that reaches the *active* tab, so the 45 existing `self.triage` call sites across 11 files
stay mechanical. Land it in two steps: bundle the state first (no behaviour change, existing
tests untouched), then add the multi-tab model.

**Panels.** One `ListPanel` entity per list tab, exactly as `ReaderPanel` works now, so the
dock's tab group draws the strip and S3's mirror (`Dock::sync` → `DraggedTabs`) applies to the
list group unchanged.

**Settled**

- `cmd-w` closes the active tab of the *focused* pane. The dock already routes focus to the
  active panel, so "focused" is well defined.
- A list tab is a whole view: location **plus** the filter query, so "Inbox" and
  "Inbox + unread + from:alice" are two tabs. That is exactly what `Triage::query` already
  holds, so a tab is that bundle and filters stop following the user across folders.
- Sidebar click opens the location in the list strip's preview tab, replacing it — the same
  rule that opens a message in the reader's preview. Cmd-click or double-click pins. A click on
  a location already open as a pinned tab activates that tab instead of opening a second one.
- `ctrl-tab` / `ctrl-shift-tab` cycle the **focused** strip, one rule with `cmd-w` closing the
  focused pane's tab. No new keys.
- Switching list tabs moves the cursor, so the reader's preview tab follows that tab's cursor;
  pinned reader tabs are untouched. (Per-tab reader state is explicitly not wanted.)
- The sidebar highlight follows the active list tab, not the last click.
- Switching a list tab does not touch the reader's tabs: two panes, two strips.
- Counts and the status bar follow the active tab.
- Dropping a message on a list tab files it under that tab's location: Inbox / Archive / Trash /
  a folder map to `Inbox` / `Archived` / `Deleted` / `Filed(folder)`. A Snoozed tab opens the
  snooze picker instead (the invariant needs a wake time, so a drop cannot pick one); Sent and
  All Inboxes do nothing. A drop elsewhere on the list pane files under the *active* tab.
  Targets are our own elements — the tab titles — because the dock's drop event names the group,
  not the tab.
- A folder dropped on the list strip opens as a **new pinned tab**; if that location is already
  open, its tab is activated instead of a second one opening. (Accepted limits of the drop
  event: it reports the group, not a tab slot, so the tab lands at the end; and it cannot tell
  payloads apart, so a folder dropped on the reader strip shows the preview and does nothing.)

## 3. The Triage tab, zoomed

A triage session becomes its own list tab that zooms in to take the view.

**Mechanism (decided).** The dock's zoom cannot keep the sidebar: `Zoomed` has one variant,
`Group(NodeId)` (`dock_area.rs:57-60`), and a zoomed group is rendered as the whole dock body
(`zoomed_view`, `:708-712`, `:1469`). So the sidebar comes **out of the dock** and becomes a
resizable panel of ours, leaving the dock as the centre alone; zooming the list group then
expands it over the centre — list and viewer area — while the sidebar stays outside.

**The dock already has exactly this**, and the fit is good:

- A zoom names a *container* — the tab group — never a panel inside it
  (`dock_area.rs:49-53`), and a zoomed group is drawn as the whole dock body:
  `frame.child(view)` from `zoomed_view()`, with no left dock and no centre split
  (`dock_area.rs:1469-1471`). So "the Triage tab takes over the view" is one call:
  `DockArea::set_zoomed_in(list_group_node, window, cx)` (`dock_area.rs:631`), and
  `set_zoomed_out` (`:640`) gives the dock back.
- The zoomed group keeps its own chrome, tab bar included (`dock_area.rs:3961`), which is what
  we want: the Triage tab sits in the list strip with the other list tabs, and the strip is how
  you leave it.
- A group only zooms if its *active* panel says it may: `set_zoomed` refuses when the panel
  is not `zoomable` (`tab_group.rs:396`). Our panels currently answer `zoomable() -> false`
  (`dock.rs`, `ListPanel`/`ReaderPanel`), so enabling it for the list panel is the one-line
  prerequisite. `Panel::set_zoomed` is delivered to the panel (`tab_group.rs:412`) if we ever
  want the tab to react to being zoomed.
- Zoom implies lock: `is_locked()` includes `zoomed` (`tab_group.rs:443`), so a zoomed group is
  neither draggable nor droppable — the right shape for a session, and it means no drop
  affordance is shown anywhere inside it while it owns the view.
- Unzooming needs no state of ours: `set_zoomed_out` restores the dock's own tree
  (`dock_area.rs:640`), so as long as the session only ever zooms its group, the layout the user
  had comes back by itself.
- The kit binds no key to zoom, so we own both directions: zoom in when the Triage tab becomes
  active, and zoom out on whatever we bind. `Panel::zoom_control` (`PanelControl`) is what draws
  a zoom affordance in the skin's tab bar if we would rather have the control than a key.

**Shape**

- `TriageTab` is a list tab whose `ListView` is the session: the same `Session` we hold today,
  just owned by a tab instead of by `MailApp`.
- Entering a session opens the tab, activates it, and zooms the list group in; the sidebar and
  reader step aside, and the strip still shows the other list tabs.
- Ending the session (triaged through, or abandoned) closes the tab and un-zooms back to the
  layout the user had.
- The tab is temporary in the same sense a preview tab is: it is not part of what the user
  arranged, so it should not survive the session.

**Settled**

- The Triage tab is a panel that renders the list **and** the session reader together, in a
  third dock group of its own that exists only for the session. That group is zoomed, so it
  fills the dock body (the sidebar sits outside the dock and stays visible) and its strip shows
  only "Triage": it acts as the only tab. Ending the session un-zooms and removes the group;
  the user's list and reader tabs, cursors, selections and scroll are untouched throughout.
- Zoom in and out happens automatically around the session; the skin's zoom control is turned on
  for the list group, and `alt-z` toggles zoom on the focused group (so it zooms any list tab in,
  not only the Triage tab).
- The Triage tab is temporary: it dies with the session, like a preview tab.
- The list strip never has fewer than one tab: the last one cannot be closed, and a fresh window
  starts with one (All Inboxes).
- A list tab is labelled with the location label alone ("Inbox", "All Inboxes") — the same string
  the sidebar row shows.
- Preview list tabs look like preview reader tabs: italic, no close button.

**Open**

- Does zoom-in happen automatically on entering a session, or is the Triage tab just another
  tab the user may zoom? (Settled: automatic.)
- Should the Triage tab be pinnable, i.e. can a session outlive its zoom?
- Do we turn on the skin's zoom control (`PanelControl`) for the list group, or bind our own
  key and leave the chrome alone?
- Should a sidebar click ever open a *pinned* list tab directly (middle-click), or is the
  preview-then-pin flow enough?

## Measured notes

- Panels observe the app because they are separate entities now. That is the same amount of
  re-rendering as before — the panes used to be rebuilt inside `MailApp::render` on exactly the
  same notifications. An idle app sits at ~0.9% CPU (sampled twice, 18s apart), so there is no
  render loop; the thing to watch is a panel render that notifies the app, which nothing does.
- `alt-l` rebuilds the centre split. Per keypress in the headless harness: ~12.5 ms for an
  ordinary keypress (cursor move) versus ~18.7 ms for a toggle, so the rebuild adds ~6 ms at
  keypress rate, not frame rate, and the open tab survives it. Worth revisiting only if the kit
  grows a way to re-axis a split in place.

## Status

Done: sidebar out of the dock; list tabs; focused strip follows the panes; a folder dropped on the
list strip opens a pinned tab; a message dropped on a list tab (or the list background, for the
active tab) files it there. The Triage tab: a session runs in its own zoomed third group and no longer
touches the user's tabs; alt-z toggles zoom on the focused group.

## Order of work

0. **Done.** The sidebar is out of the dock: an `h_resizable("mail-sidebar")` group of ours with
   one shared width, `size_range(120..320)` for the drag and the clamp, `sidebar-divider` for the
   double-click reset (nothing rendered at all while collapsed, band included), and the width
   feeding the list's limits. `Dock` no longer knows the sidebar exists. `tests/ui_panes.rs`
   passes unchanged.
1. Bundle the list view state into `ListView` (no behaviour change).
2. `ListTabs` + one `ListPanel` per tab + the sidebar/strip/keys wiring. The pure model is in
   `src/list_tabs.rs` (preview replaced in place, an open view activated rather than opened
   twice, `open_pinned` promoting a preview, the last tab unclosable), tested in
   `tests/list_tabs.rs`.
3. The Triage tab: session as a tab, `zoomable` for the list panel, zoom in/out around it.
