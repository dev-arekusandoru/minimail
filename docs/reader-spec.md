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
- Sender block: 40px initials mark on the left; line 1 is name (semibold) and address (muted, UI font); line 2 is the recipient line.
- Right of line 1, one cluster: message index (`MSG 04 / 04`, mono, threads only; chronological, 04 = newest), `COLLAPSE` (collapsed thread messages), absolute date (`Sep 30, 2026 · 20:46`, humanized time in its tooltip), Reply, `⋯`. No separate row above the subject.
- Labels row, reusing the row badge language (`src/app/icons.rs`):
  - Triage state: `Inbox`, `Snoozed · <wake time>`, `Archived`, `Filed · <folder>`, `Deleted`, tinted by `Theme::state_color`.
  - Classifier chips, icon + word, never a confidence percent: Kind (`Person`, `Receipt`, `Newsletter`, `Notification`, `Other`), Urgency (`None`, `Low`, `Normal`, `Urgent`, `Very urgent`; Siren/Flame for 4–5 and a muted `Gauge` for 1–3, never `n/5`), Possible spam (`ShieldAlert`). A cleared or unclassified value shows no chip.
  - Reply status: Needs Reply, Awaiting Reply, Follow Up, Reminder; New Sender (off in v1 — see `src/known_senders.rs`).
  - Kind, Urgency and Spam chips are clickable, opening a menu of just that section (hover tint only, clean at rest); the header `⋯` menu carries the same sections for values with no chip.
- Possible Spam keeps its banner (triage spec), with buttons: Block & Delete / Delete `d`. The New Sender banner (Allow `a` / Block `b`) is off in v1.
- Recipient line (`to You, Ben Ito · cc Dev Rao ▾`, UI font, muted) sits under the sender name and expands to full-width rows **below** the header:
  - `To:` / `Cc:` / `Bcc:`, one row each, wrapping long lists.
  - Omit empty rows. Bcc appears only when known, which usually means sent mail.
  - Expanding must not move or resize any header element; only content below moves down.

## Body

- Plain text and HTML are both first-class. Emails may be text only, HTML only, or both.
- Branded HTML, such as newsletters, renders as HTML by default. Per-message **Reader mode** (`v`) swaps it for the text part (or text extracted from the HTML) in reader typography.
  - Original HTML is drawn by a native `WKWebView` (see `src/app/email_webview/`), loaded once per message and kept while its tab lives (at most 8 views; hidden, not reloaded, on tab switches). It is as tall as its document (measured by a ResizeObserver script in an isolated content world) so the reader's scroll container scrolls it; it is hidden behind a skeleton until loaded and measured. The app theme sets default colors and `color-scheme`; authored colors are not inverted. Text is selectable with the mouse and copied with `cmd-c`; the view never takes key focus.
- Safety: page JavaScript never runs; navigation inside the view is cancelled and HTTP(S)/`mailto:` links open externally; downloads and new windows are refused; the document has no base URL. Remote images load by default. **Settings → Reader → Block remote images** opts into blocking (session-scoped) with a WebKit content rule list over all remote subresources, reloads open readers, and shows a blocked-image note. Data images remain available.
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

## Find in thread

Pure matching and per-tab query state live in `src/find.rs` (`find_in_thread`, `Find`, `Options`; uses the `regex` crate); the tab's state, landing and scrolling in `src/app/mail_app/reader_find.rs`, the bar in `reader_findbar.rs`.

- `cmd-f` opens a bar directly under the tab bar of the active tab (nothing without a tab, during a session, or behind a modal). It holds an input, the match count (`3/12`, `No results`, `Invalid regex`), icon toggles for case, whole word and regex (tooltips; `alt-c` / `alt-w` / `alt-r` while the bar is focused) and previous / next / close buttons. Bindings live in `FindBar` key context in `src/app/actions.rs`; `cmd-g` / `cmd-shift-g` work from anywhere in the app.
- Searched text: the opened message's subject line, then each message of the thread oldest first: its body and, as a separate segment, its folded quoted history. These are exactly the strings the reader renders (`reader_text` / `split_quoted`), collapsed messages included. Other messages' own subject lines are not searched (they repeat the thread subject).
- Matching is live and non-overlapping; empty matches are skipped. Case-insensitive by default (Unicode case folding); whole word uses `\b` on the sides that are word characters; regex mode takes the query as a regex, and an invalid one yields no matches and a red field, never a panic.
- Typing (and toggling an option) only updates the count and the highlights: it never expands, reveals, scrolls or pins. The current match is the first one at or after the reader's position (the opened message; its subject counts as it), else the first. Landing happens only on `enter` / `cmd-g` (next), `shift-enter` / `cmd-shift-g` (previous) or the prev/next buttons, wrapping; the first forward step after typing lands on the current match itself. Landing on a match in a collapsed message expands it (and so pins the tab); landing in folded quoted text shows the quote. The reader scrolls the match's message to the top, then nudges the match's own line into the viewport once its text is painted.
- All matches are highlighted in the body, quote and opened subject (theme `warning` tint); the current one uses `accent` with `on_accent` text.
- `esc` hides the bar and the highlights and returns focus to the app, but keeps the tab's query and options: `cmd-f` again shows them with the query selected, so typing replaces it. Query state is per tab (switching tabs keeps each one's query and position) and is dropped when the tab closes or the preview is replaced. View state: no undo step, no persistence.
- `cmd-shift-f` focuses the global search (the titlebar's, same as `/`).

Known limitations:

- HTML bodies cannot be highlighted because WebKit draws them. While a message has matches it is shown in its text (Reader mode) form instead; its own Reader mode setting is untouched, and the HTML view returns when the bar is hidden or the matches go.
- Only the opened message's subject line is searched and highlighted; the other messages' subject lines repeat the thread subject.

## Actions

- **Per message** (every expanded surface, in the header's right cluster; they act on that message, not on the opened one):
  - Reply `r`, Reply all `shift-a`, Forward `w`: icon buttons, in that order.
  - `⋯` menu: Archive `e`, Delete `d`, Snooze… `s`, Move to inbox `i` (each only when it changes something), Accept / Reject AI `y` / `n` while suggestions are pending, File… `f`, Mark spam… `!`, Toggle select `x`, Summarize `z`, Mute `m`, Unsubscribe `shift-u`, Sender actions ▸. It hangs under the button.
- Banner and suggestion buttons sit in their banners; each shows its shortcut key.
- Keyboard shortcuts also appear in the footer hint bar (`HintMode::Reader`, currently unreachable and must be wired): `r` reply, `e` archive, `f` file, `d` delete, `s` snooze, `]`/`[` thread, `v` reader mode, `shift-o` expand thread, `u` undo, `?` help; plus `y`/`n` while suggestions are pending. Keys act on the opened message or the list selection.
- Keys come from `src/app/actions.rs`; never hard-code a second map.

## AI suggestion

- When a Folder suggestion is pending (`Mailbox::pending`), a slim strip sits under the labels row and above the body: accent tint and hairline dashed accent border, `FolderInput` icon, muted `Suggested folder` then the folder path, a solid **Move** `y` and a ghost **Dismiss** `n` on the right. No confidence percent.
- Move accepts (files the message), Dismiss rejects; either removes the strip. Accepted Kind/Urgency/Spam answers appear as labels-row chips.

## Seed data coverage (`tests/fixtures/`)

Audit existing coverage first, then fill gaps:

- Short personal note; long email; long subject; international text.
- Multi-person thread (To/Cc); sent mail with Bcc; repeated quoted history.
- HTML only; text only; text + HTML alternatives; branded newsletter; receipt with tables; inline and blocked remote images; broken markup; wide layouts.
- Attachments, including on earlier messages; links; lists.
- Read and unread; every triage state (Inbox, Snoozed with a wake time, Archived, Filed, Deleted); every tag, including Possible Spam (New Sender is off in v1).
