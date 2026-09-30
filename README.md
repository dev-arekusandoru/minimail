<div align="center">

# ✉️ mail-classifier

**A keyboard-first, mouse-friendly email client that treats your inbox as a to-do list.**

Built in Rust with [GPUI](https://www.gpui.rs/) and [gpui-kit](https://gpui-kit.com/).

![Rust](https://img.shields.io/badge/rust-2024-orange?logo=rust)
![GPUI](https://img.shields.io/badge/UI-GPUI%20%2B%20gpui--kit-5b5bd6)
![Status](https://img.shields.io/badge/status-MVP-yellow)

</div>

---

Most mail apps hand you a pile and a mouse. **mail-classifier** gives every message exactly one state (**Inbox**, **Snoozed**, **Archived**, **Filed** or **Deleted**), lets you triage everything from the keyboard, and uses a fast "System 1" classifier to flag spam, replies you owe and urgent mail. Nothing is sorted away where you can't see it.

> [!NOTE]
> Early MVP. It runs on local mock data (`fixtures/`) and the AI providers are deterministic stubs. No real accounts are connected and no network calls are made.

## ✨ Features

| | |
|---|---|
| ⌨️ **Keyboard-first** | Every action has a key. `cmd-k` opens a command palette and `?` shows every shortcut. |
| 🖱️ **Mouse and keyboard driven** | Everything the keys do is one click away, and keys keep working after clicks. The titlebar holds search, **Triage**, the pane-layout toggle, a Settings gear and **More** (the global overflow: Commands, Undo, Classify, Rules, Settings, Shortcuts). Every expanded message in the reader has a **Reply** button and a **⋯** menu with Archive · Delete · Snooze (plus Inbox when it is not already there), Accept/Reject AI (only while a suggestion is pending), File, Mark spam, Select, Summarize, Mute, Unsubscribe and a **Sender actions** submenu that acts on every message from one sender at once; with two or more messages selected the list header gets the same **⋯** for the whole selection. Hover anything to see its shortcut. Click a row to open it, click its left edge (or cmd-click) to select, shift-click for a range, click a pending AI icon to accept it (right-click rejects), click sidebar locations, palette and snooze entries, click outside a menu or panel to close it, and press Escape to dismiss it. |
| ✅ **Inbox as a to-do list** | Every message is in exactly one of Inbox, Snoozed, Archived, Filed or Deleted. Every action can be undone. |
| 📦 **Batch triage** | Multi-select with `shift-j`/`shift-k` or `x`, or act on everything from one sender at once. |
| 🧠 **System 1 classifier** | Labels possible spam, needs-reply, urgency and kind. Each label either applies itself above a confidence threshold or shows as a badge for you to accept or reject. The interface mirrors [TypeSafe's Jev](https://typesafe.ai/blog/introducing-system-one-models-and-jev). |
| 📝 **Thread summaries** | Opt-in. Shows the summary, action items and dates above the thread. |
| ⏰ **Snooze with a return time** | Tonight, tomorrow, Monday or a custom time (`3h`, `2d`). The message returns to the Inbox tagged *Reminder* when it's due. |
| 🔁 **Follow-ups that come back** | Send a reply that expects an answer and the thread is tagged *Awaiting Reply*. With no answer after the *Follow-up after* timeout (Settings, default 3 days) it returns to the Inbox tagged *Follow Up*. |
| 🛡️ **New senders & blocked mail** | First-time senders sit in the Inbox with a *New Sender* badge: `a` allows (writes the contact), `b` blocks after a confirm dialog. Blocked senders are listed in Settings → **Blocked senders**, each with an undoable *Unblock*. Unsubscribing (`shift-u`) is one-way: resubscribing happens at the source. |
| 🪄 **Rules from your habits** | Do the same thing to one sender twice and it offers to make it a rule. |
| 🔍 **Search** | `from:`, `subject:`, `is:`, `before:`, `after:` and free text. |
| 🎯 **Triage sessions** | Go through the Inbox one message at a time and finish with a count and the time taken. |
| ↩️ **Undo send** | Replies wait 10 seconds in an outbox before they go. |
| 🔕 **Mute & unsubscribe** | One key each. |
| 🧵 **Group by thread** | `ctrl-g` (or Settings, or the palette) shows one row per conversation: latest sender and subject, message count, participants and newest date, sized to that content (plus the newest snippet while previews are on). `right`/`enter` or the chevron expands it inline, `left` collapses. Actions (`e` `f` `d` `s` `i`, `x`, shift-selection) on a thread row apply to every message of that thread *in the current panel* and undo as one step; sender-wide actions and mute are unchanged. A thread split across panels shows only that panel's messages in each. `]` / `[` step through the thread in date order in either mode (also clickable in the reader). |
| 🎨 **Themes** | Atom One Dark Pro by default, plus Tokyo Night. Pick one in Settings (`cmd-,`, Appearance / Theme section; click or use space / `=` / `-`). Your own JSON themes load from `~/.config/mail-classifier/themes/` (or `$MAIL_CLASSIFIER_THEMES`). |
| 👁️ **Readable rows** | Each row is sender + date on the first line, the **full-width subject** on the second, then an optional muted preview. In **All Inboxes** each row leads with its account's color dot; outgoing mail is labelled `To: <recipient>` instead of a sender. Triage tags (Needs Reply, Awaiting Reply, Follow Up, Reminder, New Sender, Possible Spam, Urgent, Kind) are distinct icons in their own theme colors, not text badges: at most a few fit (fewer in narrow windows) and the rest fold into a `+N` chip whose tooltip lists them. Hover any icon for its name; the `?` help ends with an **icon legend**. Unread mail is bold with a yellow left bar (red when urgent); an unaccepted AI suggestion is a single sparkle icon (click or `y` accepts, right-click or `n` rejects). Snoozed rows show their wake time as text (`↩ Mon 5 Oct 08:00`). |
| 🔎 **Preview lines** | Settings → *Preview lines* (or palette → *Cycle preview lines*): Off, 1–5 lines of plain-text snippet under the subject, like Apple Mail. Quoted text (`>`), signatures and reply headers are stripped. Message rows are that tall, and a thread header sizes to its own content: one compact line (sender, subject, participants, count, date) plus at most two preview lines, so a collapsed thread reads as a group header rather than a full row. Rows are measured, not padded, and scrolling, `j`/`k` and the wheel follow the cursor. Default 2. |
| 📍 **Row states** | The left edge of a row is its status bar: yellow for unread (red when urgent), blue for selected, dimmed blue when only part of a thread is selected. The row open in the reader uses the same blue bar and tint as a one-item selection; the row under the cursor has its own ring and background, and both cues combine when they coincide. Click the left edge to select without opening; cmd-click toggles and shift-click range-selects. A collapsed thread row shows the open state when the open message is inside it. |
| ↔️ **Resizable panes** | The hairline between the inbox and the reader is a divider: hover it for a resize cursor and a highlight, drag it to give either pane more room, double-click it to go back to the default. `alt-left` / `alt-right` move it from the keyboard and `alt-r` resets it; both panes keep a usable minimum, and resizing the window keeps the proportion. Rows re-measure themselves, so the icon cluster follows the new width. |
| ⬓ **Pane layout** | List left with the reader right, or list on top with the reader below: `alt-l`, the titlebar button (its icon shows the current layout), the palette (*Toggle pane layout*) or Settings → *Pane layout*. Each orientation remembers its own size and the divider works in both. |
| 🪟 **Custom titlebar** | An app-owned, Zed-style top bar with the macOS traffic lights inset into it. It shows the current view, search and the global controls; drag its empty space to move the window and double-click it to zoom. The search box stays centered at any width. |

### Keys at a glance

| Key | Action | Key | Action |
|---|---|---|---|
| `j` / `k` | Move down / up | `cmd-k` | Command palette |
| `enter` | Open message | `/` | Search |
| `e` | Archive | `f` | File (folder picker; can create a folder) |
| `d` / `#` | Delete | `s` | Snooze, with a return time |
| `i` | Move to Inbox | `r` | Reply (`cmd-enter` sends → post-send dialog) |
| `a` / `b` | Allow / block sender | `!` | Mark spam (Block & Delete / Delete) |
| `shift-e` `shift-d` `shift-f` `shift-s` | Archive / Delete / File / Snooze **all** from the sender (confirm first) | `shift-u` | Unsubscribe (confirm first) |
| `1`–`6` | Chips: All · Needs Reply · Follow Up · Urgent · New Senders · Possible Spam | `g` then `i` `s` `t` `a` `d` | Go to Inbox / Snoozed / Sent / Archive / Trash |
| `u` / `cmd-z` | Undo | `z` | Summarize thread |
| `ctrl-g` | Group by thread on/off | `m` | Mute |
| `x` | Select | `y` / `n` | Accept / reject AI badges |
| `c` | Classify again | `t` | Start a triage session |
| `cmd-,` | Settings | `?` | All shortcuts |
| `]` / `[` | Next / previous message in thread | `right` / `left` | Expand / collapse thread |
| `alt-l` | Toggle pane layout (side by side / stacked) | `alt-left` / `alt-right` | Shrink / grow the list pane |
| `alt-r` | Reset pane sizes | `v` | Reader mode for the open HTML message |
| `shift-o` | Expand / collapse the other messages of the open thread | | |

The reader shows the keys that apply to the current message in the footer (up to five; `?` shows the rest); `u` undo appears on the toast after an action. Each expanded message has Reply and **⋯** buttons in its header, and the New Sender / Possible Spam banners and the suggestion strip carry their own buttons.

#### HTML mail

HTML mail is rendered by gpui-kit's `TextView` (`gpui_kit::component::text`), which keeps **structure only**: headings, bold/italic, links, lists, tables and images.

- **Dropped:** CSS in `<style>` blocks and nearly all inline `style`, including colors, backgrounds, fonts, spacing and table widths. Branded newsletters and receipts show their layout, not their brand styling. Only two things are honored: image `width`/`height`, and the `<mark>` highlight color.
- **Not run:** scripts. `<script>` and `<style>` are removed before rendering.
- **Images:**
  - `data:` images render inline.
  - Remote `http(s)` images are blocked by `reading::safe_html` and replaced by their alt text, with a count shown under the body. There is no option to load them yet.
- **Reader mode (`v`)** shows the message's text part instead. For HTML-only mail it shows text extracted by `reading::html_to_text`.

A faithful, styled render would need an embedded web engine. This is out of scope while the app uses mock data only. See `docs/reader-spec.md`.

### Where the controls live

The chrome stays out of the way: a sidebar of locations, one titlebar, and actions next to what they act on.

**Sidebar** — *All Inboxes* (the default landing view), then one section per account. The account header is a small label (the name); hover it for a chevron and click to fold the account, which then shows its Inbox count on the header. Below it sit Inbox, Snoozed, Sent, Archive and Trash, and the top-level folders, all as plain rows with theme-tinted icons (Mails, Inbox, AlarmClock, Send, Archive, Trash, Folder/FolderOpen). Inbox and Snoozed (and All Inboxes) show a count badge when non-zero. Hover and the active row use a rounded pill; the active one has a thin bar in the account's colour. Only subfolders get tree rails (faint grey, accent along the path to the active folder); a folder with children has a fold caret at the right end of its row. `g`-prefixed jumps and the Filter ▾ menu reach every location.

**Titlebar** — the current view on the left, `Search…` in the middle (`/`), then **Triage** (`t`), the pane-layout toggle (`alt-l`), a Settings gear (`cmd-,`) and **More**. Below 800px wide Triage shows only its icon. Commands, Undo and Shortcuts live in **More**, the palette and their own shortcuts.

**Message menu (`⋯`)** — every expanded message in the reader has a **Reply** button (`r`) and a **⋯** menu, and both act on *that* message, not on whichever one the reader is opened on. The menu holds *Archive* (`e`), *Delete* (`d`), *Snooze…* (`s`) and *Move to inbox* (`i`) (each only when it would change something), *Accept AI* (`y`) / *Reject AI* (`n`) while a suggestion is pending, *File…* (`f`), *Mark spam…* (`!`), *Toggle select* (`x`), *Summarize thread* (`z`), *Mute thread* (`m`), *Unsubscribe* (`shift-u`) and the **Sender actions** submenu: *Archive · Delete · File · Move to inbox from this sender* (`shift-e` / `shift-d` / `shift-f` / `shift-i`).

**Selection menu (`⋯` in the list header)** — with two or more messages selected, a **⋯** next to *Filter ▾* opens the actions that make sense for several messages: Archive, Delete, Snooze…, Move to inbox, File… and Mark spam…. They apply to the whole selection in one undo step.

Nothing was dropped: every action is still reachable from the titlebar, the menus, the command palette (`cmd-k`) and its own shortcut.

### Settings

Settings (`cmd-,`) uses a searchable two-pane layout: sections on the left and described setting rows on the right. Search with `/`; use `j`/`k` to move, Space to change values, and Ctrl-Tab / Ctrl-Shift-Tab to change sections. Escape clears the search before closing the panel. Current controls cover classifier modes and thresholds, thread summaries, theme, pane layout, grouping, preview lines, **Follow-up after** (1–14 days, the timeout before an awaiting-reply thread returns tagged *Follow Up*) and **Blocked senders** (each with an undoable *Unblock*, plus a read-only *Unsubscribed* list — resubscribing happens at the source).

To add a setting, add its metadata (section, stable key, title, description, control type and options/range) to the settings schema in `src/app/settings.rs`, then connect its getter/setter to the existing owning model and emit the matching `SettingsEvent`. Search and section navigation should derive from that schema; keep persistence behavior unchanged.

### Icon legend

Defined once in `src/app/icons.rs` (`Glyph::spec`: icon, theme token, label, description) and shown at the end of the `?` help. Lucide icons, tinted with theme tokens.

| Icon | Meaning | Token |
|---|---|---|
| Sparkles | Pending AI suggestion (`y` accept, `n` reject) | accent |
| ShieldAlert | Possible spam | possible_spam |
| Reply | Needs reply | needs_reply |
| Hourglass | Awaiting reply (a reply was sent, an answer is expected) | awaiting |
| CornerUpRight | Follow up (the await passed the timeout) | follow_up |
| AlarmClock | Reminder (a snooze woke the message) | reminder |
| Clock | Snoozed (wake time shown as text on the row) | state_snoozed |
| UserPlus | New sender | new_sender |
| Siren / Flame / Zap | Urgency 4–5 / 3 / 1–2 (1–2 only in `+N`) | urgent / muted |
| BellOff | Muted thread | muted |
| Paperclip | Subject or opening lines mention an attachment | muted |
| User / Receipt / Newspaper / Bell / Tag | Kind: person / receipt / newsletter / notification / other (other only in `+N`) | kind |

To add a concept: add a `Glyph` variant, list it in `Glyph::ALL`, fill its `spec` arm and add the SVG name to `icon_assets!`.

## 🛠️ Development

**Requirements:** Rust (edition 2024) and, on macOS, Xcode with the command-line tools, which GPUI needs to build. So far it has only been built and run on macOS (Apple Silicon).

```sh
cargo run                                   # launch the app
cargo test                                  # model, classifier, search and headless UI tests
cargo clippy --all-targets -- -D warnings   # lint
```

The UI tests run headless. They drive the real views with simulated keystrokes and a fake clock, so no window opens.

### Where the data lives

Contacts live in one SQLite database: `$MAIL_CLASSIFIER_DB` if set, otherwise
`~/Library/Application Support/mail-classifier/contacts.db`. It is created (with
its directory) on first launch, migrated forward through `PRAGMA user_version`,
and seeded once from `fixtures/contacts_seed.json`; afterwards the file is
yours, so deleting it just re-seeds on the next start. Allowing a new sender
writes a contact there (undo takes it back out), and everything else
the app tracks — messages, triage, rules, themes — still comes from the
fixtures.

### Project layout

```
src/
├── model/        # triage states, undo, snooze, screener, outbox (pure logic; split by concern)
├── contacts/     # SQLite address book: schema + migrations, CRUD, search, groups, seeding
├── judge/        # System 1 classifier interface, stub provider, routing policy
├── summary.rs    # thread summarizer interface + stub
├── search.rs     # query parser
├── rules.rs      # rule suggestions
├── clock.rs      # injectable clock
├── theme.rs      # semantic color tokens, theme registry, JSON loader, active theme
├── preview.rs    # plain-text snippets and the Preview lines setting (pure)
└── app/          # GPUI views: main window, palette, compose, panels; icons.rs is the icon language table, row.rs the row frame
    └── mail_app/ # MailApp split by concern: accessors, actions, modals, help, rows, list, reader, render (+ grouping, mouse)
fixtures/         # mock mailbox + the seed address book (contacts_seed.json, embedded at build time)
themes/           # built-in themes (JSON, embedded at build time)
tests/            # integration and headless UI tests; model/ and ui_ext/ are multi-module test crates (shared helpers in helpers.rs / harness.rs)
```

### Adding a theme

A theme is a JSON file: `{ "name": "My Theme", "colors": { "<token>": "#rrggbb", … } }` with every token from `themes/one-dark-pro.json` (background, surface, sidebar, border, text, text_muted, accent, on_accent, selection, hover, row_cursor, selected, unread, needs_reply, awaiting, follow_up, reminder, possible_spam, new_sender, urgent, kind, state_*, success, warning, error, info).

- **Built-in:** add `themes/<name>.json` and list it in `BUILTIN` in `src/theme.rs`.
- **User:** put the file in `~/.config/mail-classifier/themes/`; it appears in the Settings picker. A theme with an existing name replaces it; an unknown theme name falls back to One Dark Pro.

## 🗺️ Roadmap

- [ ] Real classifier providers: TypeSafe Jev, then Ollama and OpenAI-compatible APIs
- [ ] Bring-your-own provider for summaries
- [ ] Review queue alongside the inline badges
- [ ] Folders or streams driven by classifier labels
- [ ] IMAP/SMTP accounts
