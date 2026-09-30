<div align="center">

# ✉️ mail-classifier

**A keyboard-first, mouse-friendly email client that treats your inbox as a to-do list.**

Built in Rust with [GPUI](https://www.gpui.rs/) and [gpui-kit](https://gpui-kit.com/).

![Rust](https://img.shields.io/badge/rust-2024-orange?logo=rust)
![GPUI](https://img.shields.io/badge/UI-GPUI%20%2B%20gpui--kit-5b5bd6)
![Status](https://img.shields.io/badge/status-MVP-yellow)

</div>

---

Most mail apps hand you a pile and a mouse. **mail-classifier** gives every message exactly one state (**Inbox**, **Waiting**, **Later** or **Done**), lets you triage everything from the keyboard, and uses a fast "System 1" classifier to flag spam, replies you owe and urgent mail. Nothing is sorted away where you can't see it.

> [!NOTE]
> Early MVP. It runs on local mock data (`fixtures/`) and the AI providers are deterministic stubs. No real accounts are connected and no network calls are made.

## ✨ Features

| | |
|---|---|
| ⌨️ **Keyboard-first** | Every action has a key. `cmd-k` opens a command palette and `?` shows every shortcut. |
| 🖱️ **Mouse and keyboard driven** | Everything the keys do is one click away, and keys keep working after clicks. The header stays quiet: search on the left, **Triage** and **More** on the right. Triage acts on the selected messages (Done · Waiting · Later…, plus Inbox when you are not already in it); **More** holds Commands, Undo, Classify, Rules, Settings and Help. The list carries its own **More** menu for Select, Summarize, Mute, Unsubscribe and the AI Accept/Reject pair (the AI pair only while a suggestion is pending), and a **Sender actions** submenu that acts on every message from one sender at once. Hover anything to see its shortcut. Click a row to open it, tick its checkbox (or cmd-click) to select, shift-click for a range, click a pending AI badge to accept it (right-click rejects), click sidebar tabs, palette and snooze entries, click outside a menu or panel to close it, and press Escape to dismiss it. |
| ✅ **Inbox as a to-do list** | Every message is in exactly one of Inbox, Waiting, Later or Done. Every action can be undone. |
| 📦 **Batch triage** | Multi-select with `shift-j`/`shift-k` or `x`, or act on everything from one sender at once. |
| 🧠 **System 1 classifier** | Labels spam, needs-reply, urgency, kind and a suggested state. Each label either applies itself above a confidence threshold or shows as a badge for you to accept or reject. The interface mirrors [TypeSafe's Jev](https://typesafe.ai/blog/introducing-system-one-models-and-jev). |
| 📝 **Thread summaries** | Opt-in. Shows the summary, action items and dates above the thread. |
| ⏰ **Snooze with a return time** | Tonight, tomorrow, Monday or a custom time (`3h`, `2d`). The message returns to the Inbox when it's due. |
| 🔁 **Follow-ups that come back** | Waiting threads with no reply after 3 days return to the Inbox, tagged *no reply*. |
| 🛡️ **Screener** | First-time senders wait until you allow or block them. Blocking hides mail and never deletes it.  Allowing a sender adds them to your address book, so the decision survives a restart.
| 🪄 **Rules from your habits** | Do the same thing to one sender twice and it offers to make it a rule. |
| 🔍 **Search** | `from:`, `subject:`, `is:`, `before:`, `after:` and free text. |
| 🎯 **Triage sessions** | Go through the Inbox one message at a time and finish with a count and the time taken. |
| ↩️ **Undo send** | Replies wait 10 seconds in an outbox before they go. |
| 🔕 **Mute & unsubscribe** | One key each. |
| 🧵 **Group by thread** | `g` (or Settings, or the palette) shows one row per conversation: latest sender and subject, message count, participants and newest date, sized to that content (plus the newest snippet while previews are on). `right`/`enter` or the chevron expands it inline, `left` collapses. Actions (`e` `w` `i` `l`, `x`, shift-selection) on a thread row apply to every message of that thread *in the current panel* and undo as one step; sender-wide actions and mute are unchanged. A thread split across panels shows only that panel's messages in each. `]` / `[` step through the thread in date order in either mode (also clickable in the reader). |
| 🎨 **Themes** | Atom One Dark Pro by default, plus Tokyo Night. Pick one in Settings (`cmd-,`, Appearance / Theme section; click or use space / `=` / `-`). Your own JSON themes load from `~/.config/mail-classifier/themes/` (or `$MAIL_CLASSIFIER_THEMES`). |
| 👁️ **Readable rows** | Each row is sender + date on the first line, the **full-width subject** on the second, then an optional muted preview. Labels are icons, not text badges: at most a few fit (fewer in narrow windows) and the rest fold into a `+N` chip whose tooltip lists them. Hover any icon for its name; the `?` help ends with an **icon legend**. Unread mail is bold with a closed-envelope icon; an unaccepted AI suggestion is a single sparkle icon (click or `y` accepts, right-click or `n` rejects). Return times stay as text (`↩ Mon 5 Oct 08:00`). |
| 🔎 **Preview lines** | Settings → *Preview lines* (or palette → *Cycle preview lines*): Off, 1–5 lines of plain-text snippet under the subject, like Apple Mail. Quoted text (`>`), signatures and reply headers are stripped. Message rows are that tall, and a thread header sizes to its own content: one compact line (sender, subject, participants, count, date) plus at most two preview lines, so a collapsed thread reads as a group header rather than a full row. Rows are measured, not padded, and scrolling, `j`/`k` and the wheel follow the cursor. Default 2. |
| 📍 **Open vs. cursor** | The row under the cursor has a ring and a highlight; the row open in the reader keeps a tint, a solid accent bar on its left edge and an open-envelope icon; both cues combine when they coincide. Both stay distinct from checked rows (accent wash + filled checkbox). A collapsed thread row shows the open indicator when the open message is inside it. |
| ↔️ **Resizable panes** | The hairline between the inbox and the reader is a divider: hover it for a resize cursor and a highlight, drag it to give either pane more room, double-click it to go back to the default. `alt-left` / `alt-right` move it from the keyboard and `alt-r` resets it; both panes keep a usable minimum, and resizing the window keeps the proportion. Rows re-measure themselves, so the icon cluster follows the new width. |
| ⬓ **Pane layout** | List left with the reader right, or list on top with the reader below: `alt-l`, the toolbar button (its glyph shows the layout you switch to), the palette (*Toggle pane layout*) or Settings → *Pane layout*. Each orientation remembers its own size and the divider works in both. |

### Keys at a glance

| Key | Action | Key | Action |
|---|---|---|---|
| `j` / `k` | Move down / up | `cmd-k` | Command palette |
| `enter` | Open message | `/` | Search |
| `e` `w` `i` | Done · Waiting · Inbox | `l` | Later, with a return time |
| `shift-e`… | Same, for all mail from the sender | `x` | Select |
| `u` | Undo (recalls an unsent reply first) | `r` | Reply (`cmd-enter` sends) |
| `y` / `n` | Accept / reject AI badges | `c` | Classify again |
| `s` | Summarize thread | `t` | Start a triage session |
| `m` / `shift-u` | Mute / unsubscribe | `1`–`5` | Inbox · Waiting · Later · Done · Screener |
| `cmd-,` | Settings | `?` | All shortcuts |
| `g` | Group by thread on/off | `]` / `[` | Next / previous message in thread |
| `right` / `left` | Expand / collapse thread | | |
| `alt-l` | Toggle pane layout (side by side / stacked) | `alt-left` / `alt-right` | Shrink / grow the list pane |
| `alt-r` | Reset pane sizes | | |

### Where the controls live

The chrome stays out of the way: one restrained header row, and actions next to what they act on.

**Header** — `Search…` on the left (`/`); `Triage` and `More` on the right.

- **Triage** starts a session (`t`). With one or more messages selected, the context row above the list shows *N selected*, the core state buttons and *Reply* (only while a message is open in the reader).
- **More** is the global overflow: **Commands** (`cmd-k`), **Undo** (`u`), **Classify** (`c`), **Rules** (`shift-r`), **Settings** (`cmd-,`) and **Help** (`?`).

**Context row above the list** — *Done* (`e`), *Waiting* (`w`), *Later…* (`l`) and, when you are not already in the Inbox, *Inbox* (`i`), plus the selection count. The buttons only appear when there is something valid to act on.

**More menu beside the context row** — the secondary actions: *Select* (`x`), *Summarize* (`s`), *Mute* (`m`), *Unsubscribe* (`shift-u`), *Accept AI* (`y`) and *Reject AI* (`n`), shown only while a suggestion is pending, and the **Sender actions** submenu: *All from sender: Done · Waiting · Inbox · Later* (`shift-e` / `shift-w` / `shift-i` / `shift-l`).

Nothing was dropped: every action is still reachable from the header, the context row, the menus, the command palette (`cmd-k`) and its own shortcut.

### Settings

Settings (`cmd-,`) uses a searchable two-pane layout: sections on the left and described setting rows on the right. Search with `/`; use `j`/`k` to move, Space to change values, and Ctrl-Tab / Ctrl-Shift-Tab to change sections. Escape clears the search before closing the panel. Current controls cover classifier modes and thresholds, thread summaries, theme, pane layout, grouping, and preview lines.

To add a setting, add its metadata (section, stable key, title, description, control type and options/range) to the settings schema in `src/app/settings.rs`, then connect its getter/setter to the existing owning model and emit the matching `SettingsEvent`. Search and section navigation should derive from that schema; keep persistence behavior unchanged.

### Icon legend

Defined once in `src/app/icons.rs` (`Glyph::spec`: icon, theme token, label, description) and shown at the end of the `?` help. Lucide icons, tinted with theme tokens.

| Icon | Meaning | Token |
|---|---|---|
| Mail / MailOpen | Unread / open in the reader | accent |
| SquareCheck | Checked for a bulk action | accent |
| Sparkles | Pending AI suggestion (`y` accept, `n` reject) | accent |
| ShieldAlert | Spam | spam |
| Reply | Needs reply | needs_reply |
| Siren / Flame / Zap | Urgency 4–5 / 3 / 1–2 (1–2 only in `+N`) | urgent / muted |
| UserPlus | New sender (screener) | state_screener |
| AlarmClock | Snoozed (return time shown as text in Later) | state_later |
| Hourglass | No reply yet, follow up | state_waiting |
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
yours, so deleting it just re-seeds on the next start. Allowing a sender in the
Screener writes a contact there (undo takes it back out), and everything else
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

A theme is a JSON file: `{ "name": "My Theme", "colors": { "<token>": "#rrggbb", … } }` with every token from `themes/one-dark-pro.json` (background, surface, sidebar, border, text, text_muted, accent, on_accent, selection, hover, row_cursor, row_open, spam, needs_reply, urgent, kind, state_*, success, warning, error, info).

- **Built-in:** add `themes/<name>.json` and list it in `BUILTIN` in `src/theme.rs`.
- **User:** put the file in `~/.config/mail-classifier/themes/`; it appears in the Settings picker. A theme with an existing name replaces it; an unknown theme name falls back to One Dark Pro.

## 🗺️ Roadmap

- [ ] Real classifier providers: TypeSafe Jev, then Ollama and OpenAI-compatible APIs
- [ ] Bring-your-own provider for summaries
- [ ] Review queue alongside the inline badges
- [ ] Folders or streams driven by classifier labels
- [ ] IMAP/SMTP accounts
