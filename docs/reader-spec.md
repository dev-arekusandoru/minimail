# Reader spec (opened message)

Status: implemented 2026-09-30 (`src/reading.rs`, `src/app/mail_app/reader*.rs`); triage re-review done against `docs/triage-spec.md`.

## Direction

- Minimal, technical, keyboard-first. Not a generic mail client look.
- Quiet reading surface with a compact header.
- Hierarchy: subject > sender > body > metadata.
- Monospace for metadata: timestamps, labels, message index (`MSG 01 / 04`), addresses, keycaps, section titles. Proportional font for subject and body.
- Typography, spacing, and restrained borders carry the hierarchy, not color.

## Header

- Subject: largest text, wraps and never truncates.
- Sender name and address on one line; initials mark on the left.
- Timestamp and message index in mono, top row. Index is chronological within the thread (`MSG 04 / 04` = newest).
- Labels row, reusing the row badge language (`src/app/icons.rs`):
  - Triage state: `Inbox`, `Snoozed · <wake time>`, `Archived`, `Filed · <folder>`, `Deleted`, tinted by `Theme::state_color`.
  - Tags: Needs Reply, Awaiting Reply, Follow Up, Reminder, Urgent, Possible Spam, New Sender.
  - Category: the `Kind` tag (for example Newsletter or Receipt).
- New Sender and Possible Spam keep their banners (triage spec), with buttons: Allow `a` / Block `b`; Block & Delete / Delete `d`.
- Recipient summary (`to me, +3 others`) expands to full-width rows **below** the header:
  - `To:` / `Cc:` / `Bcc:`, one row each, wrapping long lists.
  - Omit empty rows. Bcc appears only when known, which usually means sent mail.
  - Expanding must not move or resize any header element; only content below moves down.

## Body

- Plain text and HTML are both first-class. Emails may be text only, HTML only, or both.
- Branded HTML, such as newsletters, renders as HTML by default. Per-message **Reader mode** (`v`) swaps it for the text part (or text extracted from the HTML) in reader typography.
  - Constraint: GPUI's HTML renderer (`gpui_kit::component::text::TextView`) renders structure only: headings, emphasis, links, lists, tables, and images. CSS colors, backgrounds, fonts, spacing, and table widths are dropped, so the "original" view is structural, not a pixel-faithful brand render. Exceptions: image `width`/`height` and the `<mark>` highlight color. A faithful render would need a web engine, which is out of scope. User-facing details: README → *HTML mail*.
- Safety: scripts and styles are dropped by the renderer. Remote (`http(s)`) images are blocked and replaced by their alt text, with a "N remote images blocked" note. `data:` images render inline.
- Quoted history inside a body is collapsed ("Show quoted text").
- Attachments appear as chips with the filename and size.

## Thread

- The thread renders as one timeline, oldest first, and the order never changes: the opened message sits at its chronological place, expanded; the other messages are collapsed lines unless toggled. Section title: `THREAD · N MORE` above the rail.
- Each message is its own surface, at the same width and visual level, never nested inside another.
- Threads only (2+ messages): a vertical thread line in the left margin connects all messages, with one dot per message colored by triage state (`Theme::state_color`). A single message has no rail and uses the full width.
- Collapsed earlier message: one compact line (~46px) with the sender, a muted snippet, an attachment indicator, and a mono date aligned right.
- Expanded earlier message: the same header as the opened message (subject, sender and address, labels, recipient disclosure), then the body.
- Multiple messages can be open at once. `shift-o` expands all, or collapses all when every message is already expanded.
- Expanding or collapsing never moves surfaces above the toggled one, and there is no height animation (the demo glitched with one).
- `]` / `[` open the next or previous message in the thread. The target expands in place and the message left behind stays expanded (stepping never collapses anything; it goes through the same expansion state as clicking a line). The reader scrolls so the newly opened message's top is visible (`ScrollHandle::scroll_to_top_of_item`). Opening a message from the list does the same.
- Expansion, recipient, and quoted-text disclosure are view state, not undo steps. Each tab keeps its own; they are dropped when the tab closes (or the preview is replaced).

## Tabs

State is pure (`src/tabs.rs`: `Tabs`, one `Tab { thread, msg, pinned }` per thread); operations live in `src/app/mail_app/reader_tabs.rs`, the bar in `reader_tabbar.rs` (gpui-component `TabBar` / `Tab`, each tab with its own `on_click` so a double-click can be told from a click). `MailApp::opened()` is the active tab's message; a triage session bypasses the tabs and shows its own message.

- One tab per thread. Title: the thread's latest subject, truncated to 28 characters. Icon: the monogram of the thread's latest sender (the same `Look::monogram` the message header uses), unless Settings → *Show sender avatar in tabs* is off. A `×` in each tab closes it.
- Opening a message from the list (click or `enter`) shows it in the *preview* tab (italic title). There is at most one; the next open replaces it in place. If the thread already has a tab, that tab is activated and shows the opened message; its pin state is kept.
- Pinned (permanent, upright title) by: double-clicking the tab; any click inside the reader content; expanding or collapsing a thread message (`shift-o` too); replying (`r` or Reply); using an action of a message's **⋯** menu; `enter` (or a double-click in the list) on the message the preview tab already shows.
- `cmd-w` closes the active tab; the active tab falls to its right neighbour, else its left. `ctrl-tab` / `cmd-shift-]` and `ctrl-shift-tab` / `cmd-shift-[` cycle, wrapping. `]` / `[` stay in-thread navigation. The shortcuts do nothing behind a modal or menu, or during a session.
- Switching tabs moves the list cursor to the tab's message when the list has it; the selection is untouched.
- Triage: a preview tab whose thread no longer has any message in the current list (archive, delete, snooze, mute, a view or search change) closes; a pinned tab stays and keeps showing its thread. Acting on a message never closes a pinned tab.
- Per tab: expansion, recipients, quoted text, Reader mode and scroll position survive switches (`ReaderView` is keyed by thread; scroll handles by thread).
- Session mode hides the tab bar and shows only the session message; the tabs are untouched and return afterwards.
- Tabs are view state: no undo step, no persistence.

## Actions

- **Per message** (every expanded surface, in the header's right cluster; they act on that message, not on the opened one):
  - Reply `r`, an icon button.
  - `⋯` menu: Archive `e`, Delete `d`, Snooze… `s`, Move to inbox `i` (each only when it changes something), Accept / Reject AI `y` / `n` while suggestions are pending, File… `f`, Mark spam… `!`, Toggle select `x`, Summarize `z`, Mute `m`, Unsubscribe `shift-u`, Sender actions ▸. It hangs under the button.
- Banner and suggestion buttons sit in their banners; each shows its shortcut key.
- Keyboard shortcuts also appear in the footer hint bar (`HintMode::Reader`, currently unreachable and must be wired): `r` reply, `e` archive, `f` file, `d` delete, `s` snooze, `]`/`[` thread, `v` reader mode, `shift-o` expand thread, `u` undo, `?` help; plus `y`/`n` while suggestions are pending. Keys act on the opened message or the list selection.
- Reply all and Forward are omitted because no such actions exist; no dead buttons.
- Keys come from `src/app/actions.rs`; never hard-code a second map.

## AI suggestion

- A strip under the header lists the pending review suggestions for the message (`Mailbox::pending`), one per question key, for example `Suggested: Needs Reply 87% · Newsletter 92%`.
- The strip carries Accept `y` / Reject `n` buttons.
- Resolving removes the strip; accepted tags appear in the labels row.

## Seed data coverage (`fixtures/`)

Audit existing coverage first, then fill gaps:

- Short personal note; long email; long subject; international text.
- Multi-person thread (To/Cc); sent mail with Bcc; repeated quoted history.
- HTML only; text only; text + HTML alternatives; branded newsletter; receipt with tables; inline and blocked remote images; broken markup; wide layouts.
- Attachments, including on earlier messages; links; lists.
- Read and unread; every triage state (Inbox, Snoozed with a wake time, Archived, Filed, Deleted); every tag, including New Sender and Possible Spam.
