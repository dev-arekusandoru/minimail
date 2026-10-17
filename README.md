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
> Early MVP. By default it starts with an empty mailbox. The classifier is off until you add a TypeSafe Jev API key (Settings → AI → **Jev connection**), and thread summaries are still a stub. You can optionally link a Gmail account (see [Gmail accounts](#gmail-accounts)) to triage real mail.

## ✨ Features

| | |
|---|---|
| ⌨️ **Keyboard-first** | Every action has a key. `cmd-k` opens a sectioned, fuzzy-searchable command palette (type a name, a few letters of it, or its key) and `?` shows every shortcut. |
| 🖱️ **Mouse and keyboard driven** | Everything the keys do is one click away, and keys keep working after clicks. The titlebar holds search, **Triage**, the pane-layout toggle, a Settings gear and **More** (the global overflow: Commands, Undo, Classify, Rules, Settings, Shortcuts). Every expanded message in the reader has a **Reply** button and a **⋯** menu with Archive · Delete · Snooze (plus Inbox when it is not already there), Accept/Reject AI (only while a suggestion is pending), File, Mark spam, Select, Summarize, Mute, Unsubscribe and a **Sender actions** submenu that acts on every message from one sender at once; with two or more messages selected the list header gets the same **⋯** for the whole selection. Hover anything to see its shortcut. Click a row to open it, click its left edge (or cmd-click) to select, shift-click for a range, click a pending AI icon to accept it (right-click rejects), click sidebar locations, palette and snooze entries, click outside a menu or panel to close it, and press Escape to dismiss it. |
| ✅ **Inbox as a to-do list** | Every message is in exactly one of Inbox, Snoozed, Archived, Filed or Deleted. Every action can be undone. |
| 📦 **Batch triage** | Multi-select with `shift-j`/`shift-k` or `x`, or act on everything from one sender at once. |
| 🧠 **System 1 classifier** | Labels possible spam, needs-reply, urgency and kind. Each label either applies itself above a confidence threshold or shows as a badge for you to accept or reject. Runs on [TypeSafe's Jev](https://typesafe.ai/blog/introducing-system-one-models-and-jev) with your own API key (Settings → AI → **Jev connection**): new mail is judged one message at a time on a background worker, its state in the status bar, and the raw answers are cached in the contacts DB so changing a threshold never re-bills. With no key the classifier is off and nothing is labelled. |
| 📝 **Thread summaries** | Opt-in. Shows the summary, action items and dates above the thread. |
| ⏰ **Snooze with a return time** | Tonight, tomorrow, Monday or a custom time (`3h`, `2d`). The message returns to the Inbox tagged *Reminder* when it's due. |
| 🔁 **Follow-ups that come back** | Send a reply that expects an answer and the thread is tagged *Awaiting Reply*. With no answer after the *Follow-up after* timeout (Settings, default 3 days) it returns to the Inbox tagged *Follow Up*. |
| 🛡️ **Blocked mail** | `b` blocks a sender after a confirm dialog; blocked senders are listed in Settings → **Blocked senders**, each with an undoable *Unblock*. Accepting spam — **Mark spam** (`!`), or **Delete** on a message flagged Possible spam — and then deleting without blocking asks *Delete all mail from <address>?* (when more of their mail is in the Inbox) and then *Always mark mail from <address> as spam?*; marking a flagged message **Not spam** asks *Never mark mail from <address> as spam?*. Enter confirms, Esc skips (at the first spam question Esc ends the sequence). Yes records the exact address (case-insensitive, never the domain) as an always-spam or never-spam sender: Jev is no longer asked the spam question for them, their new mail arrives flagged when always-spam, and never-spam mail is never auto-flagged. Nothing is asked when the sender is blocked or already in either list; the last choice wins. The answers undo with the action in one step, and both lists are kept in preferences and managed in Settings → Senders. Unsubscribing (`shift-u`) is one-way: resubscribing happens at the source. (The known/unknown *screener* — the New Sender badge and `a` allow — is off in v1; see `src/known_senders.rs`.) |
| 🪄 **Rules from your habits** | Do the same thing to one sender twice and it offers to make it a rule. |
| 🔍 **Search** | `from:`, `to:`, `cc:`, `bcc:`, `subject:`, `body:`, `is:`, `in:`, `tag:`, `kind:`, `account:`, `before:` / `after:` / `on:` (a date or `7d`, `2w`, `3m`, `1y`) and free text. Searching runs inside the folder you are in; remove its `in:` pill to search everywhere. |
| 🎯 **Triage sessions** | Go through the Inbox one message at a time and finish with a count and the time taken. |
| ↩️ **Undo send** | Replies wait 10 seconds in an outbox before they go. |
| 🔕 **Mute & unsubscribe** | One key each. |
| 🧵 **Group by thread** | `ctrl-g` (or Settings, or the palette) shows one row per conversation: latest sender and subject, message count, participants and newest date, sized to that content (plus the newest snippet while previews are on). `right`/`enter` or the chevron expands it inline, `left` collapses. Actions (`e` `f` `d` `s` `i`, `x`, shift-selection) on a thread row apply to every message of that thread *in the current panel* and undo as one step; sender-wide actions and mute are unchanged. A thread split across panels shows only that panel's messages in each. `]` / `[` step through the thread in date order in either mode (also clickable in the reader). |
| 🎨 **Themes** | Atom One Dark Pro by default, plus the full [GPUI Kit theme collection](https://gpui-kit.com/component/theme/) (all light/dark variants), Tokyo Night and the kit defaults: 39 options bundled offline. In Settings (`cmd-,`, Appearance page) pick a mode (Light, Dark or System, which follows macOS) plus the theme used for each appearance. Your own gpui-kit theme files load (and hot-reload) from `~/.config/mail-classifier/themes/` (or `$MAIL_CLASSIFIER_THEMES`). |
| 👁️ **Readable rows** | Each row is sender + date on the first line, the **full-width subject** on the second, then an optional muted preview. In **All Inboxes** the sender line starts with the account's icon in its color (choose both, plus an optional nickname shown instead of the account name in the sidebar, filters and toasts, per account in Settings → Accounts); outgoing mail is labelled `To: <recipient>` instead of a sender. Triage tags (Needs Reply, Awaiting Reply, Follow Up, Reminder, Possible Spam, Urgent, Kind) are distinct icons in their own theme colors, not text badges: at most a few fit (fewer in narrow windows) and the rest fold into a `+N` chip whose tooltip lists them. Hover any icon for its name; the `?` help ends with an **icon legend**. Unread mail is bold with a yellow left bar (red when urgent); an unaccepted AI suggestion is a single sparkle icon (click or `y` accepts, right-click or `n` rejects). Snoozed rows show their wake time as text (`↩ Mon 5 Oct 08:00`). |
| 🔎 **Preview lines** | Settings → *Preview lines* (or palette → *Cycle preview lines*): Off, 1–5 lines of plain-text snippet under the subject, like Apple Mail. Quoted text (`>`), signatures and reply headers are stripped. Message rows are that tall, and a thread header sizes to its own content: one compact line (sender, subject, participants, count, date) plus at most two preview lines, so a collapsed thread reads as a group header rather than a full row. Rows are measured, not padded, and scrolling, `j`/`k` and the wheel follow the cursor. Default 2. |
| 🗂️ **Reader tabs** | The reader has editor-style tabs, one per thread. Opening a message (click or `enter`) shows it in the *preview* tab (italic title), which the next open replaces; open a message of a thread that already has a tab and that tab comes forward instead. Double-click a tab, click inside the message, expand a collapsed thread message, reply, use a message **⋯** action, or press `enter` on the previewed message to pin it. `cmd-w` closes the active tab (or click its ×); `ctrl-tab` / `ctrl-shift-tab` and `cmd-shift-]` / `cmd-shift-[` switch, and the list cursor follows (selection untouched). Archiving, deleting or snoozing the last message of a previewed thread out of the list closes the preview; a pinned tab stays. Each tab keeps its own expansion, quoted-text, Reader-mode and scroll state; nothing persists across launches. A triage session hides the tab bar and shows only its message; the tabs come back afterwards. Each tab shows the monogram of the thread's latest sender (Settings → Appearance → *Show sender avatar in tabs*, on by default). |
| 🔍 **Find in thread** | `cmd-f` opens a find bar under the reader tab bar (like Zed's buffer search): live match count (`3/12`), *match case* `alt-c`, *whole word* `alt-w` and *regex* `alt-r` toggles, `enter` / `cmd-g` next and `shift-enter` / `cmd-shift-g` previous (wrapping), `esc` hides it (the tab keeps its query, shown selected on the next `cmd-f`). Typing only updates the count and highlights; landing on a match happens on `enter`. It searches the subject and every message body of the tab's thread, collapsed messages and folded quoted text included; landing on a match expands its message (which pins the tab) or reveals its quote, and scrolls it into view. All matches are highlighted, the current one in the accent colour; an invalid regex finds nothing and turns the field red. Each tab keeps its own query until the tab closes. HTML bodies cannot be highlighted, so while they have matches they are shown in text form. `cmd-shift-f` focuses the global search, like `/`. |
| 📍 **Row states** | The left edge of a row is its status bar: blue for selected (and for the row open in the reader), dimmed blue when only part of a thread is selected. Unread mail is marked by its bold sender and subject, not by the bar. The row under the cursor has its own ring and background, and both cues combine when they coincide. Click the left edge to select without opening; cmd-click toggles and shift-click range-selects. A collapsed thread row shows the open state when the open message is inside it. |
| ↔️ **Resizable panes** | Three panes (sidebar, inbox, reader) sit in gpui-kit resizable groups. Hover a divider for a resize cursor and highlight, drag it to give either pane more room, double-click it to go back to the default (each divider resets itself). `alt-left` / `alt-right` move the inbox/reader divider from the keyboard and `alt-r` resets it; every pane keeps a usable minimum, and resizing the window keeps the proportion. Rows re-measure themselves, so the icon cluster follows the new width. |
| ◧ **Collapsible sidebar** | `cmd-b`, the titlebar button at the far left, or the palette (*Toggle sidebar*) hides the sidebar and gives its room to the panes; showing it again restores the width it had. |
| ⬓ **Pane layout** | List left with the reader right, or list on top with the reader below: `alt-l`, the titlebar button (its icon shows the current layout), the palette (*Toggle pane layout*) or Settings → *Pane layout*. Each orientation remembers its own size and the divider works in both. |
| 🪟 **Custom titlebar** | An app-owned, Zed-style top bar with the macOS traffic lights inset into it. It shows the current view, search and the global controls; drag its empty space to move the window and double-click it to zoom. The search box stays centered at any width. |

### Keys at a glance

| Key | Action | Key | Action |
|---|---|---|---|
| `j` / `k` | Move down / up | `cmd-k` | Command palette |
| `enter` | Open message | `/` | Search |
| `e` | Archive | `f` | File (folder picker; can create a folder) |
| `d` | Delete | `s` | Snooze, with a return time |
| `i` | Move to Inbox | `r` | Reply (`cmd-enter` sends → post-send dialog) |
| `shift-a` | Reply all (sender + To + Cc, minus your address) | `w` | Forward (empty editable To, `Fwd:` subject, forwarded-message block; `cmd-enter` sends) |
| `b` | Block sender | `!` | Mark spam (Block & Delete / Delete) |
| `shift-e` `shift-d` `shift-f` `shift-s` | Archive / Delete / File / Snooze **all** from the sender (`enter` confirms, `esc` cancels) | `shift-u` | Unsubscribe (dialog: `enter` = Leave, `1`–`3` move the mail first) |
| `1`–`4`, `6` | Filter pills: clear · toggle Needs Reply · Follow Up · Urgent · Possible Spam | `g` then `i` `s` `t` `a` `d` | Go to Inbox / Snoozed / Sent / Archive / Trash |
| `l` | Add a filter pill (`+ Filter`) | `p` | Open the sender's mail (exact address, every folder) in a new list tab |
| `u` / `cmd-z` | Undo | `z` | Summarize thread |
| `ctrl-g` | Group by thread on/off | `m` | Mute |
| `x` | Select | `y` / `n` | Accept / reject AI badges |
| `c` | Classify again | `t` | Start a triage session |
| `cmd-,` | Settings | `?` | All shortcuts |
| `]` / `[` | Next / previous message in thread | `right` / `left` | Expand / collapse thread |
| `alt-l` | Toggle pane layout (side by side / stacked) | `alt-left` / `alt-right` | Shrink / grow the list pane |
| `alt-r` | Reset pane sizes | `v` | Reader mode for the open HTML message |
| `cmd-b` | Toggle the sidebar | `cmd-q` | Quit (also from inputs and Settings) |
| `shift-o` | Expand / collapse the other messages of the open thread | `cmd-w` | Close the active reader tab |
| `ctrl-tab` / `ctrl-shift-tab` | Next / previous reader tab (also `cmd-shift-]` / `cmd-shift-[`) | `cmd-f` | Find in the open thread (`enter` / `shift-enter` or `cmd-g` / `cmd-shift-g` step, `esc` closes) |
| `cmd-shift-f` | Global search (same as `/`) | | |

The reader shows the keys that apply to the current message in the footer (up to five; `?` shows the rest); `u` undo appears on the toast after an action. Each expanded message has Reply and **⋯** buttons in its header, and the Possible Spam banners and the suggestion strip carry their own buttons.

#### HTML mail

HTML mail is drawn by a native **WKWebView** (WebKit, via `gpui-wry`), so mail renders the way it was written for. The view is a real native view placed over the reader; a message loads into it once and the view stays until its tab is closed.

- **Persistence:** each message keeps its view while its tab lives — switching tabs only hides and shows it, never reloads. It reloads only when the HTML, the **Block remote images** setting or the theme changes. At most 8 views stay alive (least recently shown are dropped and rebuilt on their next show); closing a tab frees its views. All views share one web process.
- **Scrolling and height:** the view is as tall as its document, so the reader's own scroll container scrolls lone messages and threads alike, wheel events over the page are forwarded to it, and the page never scrolls internally (wide content is clipped). The height comes from a script in an isolated WebKit content world (page JavaScript stays off) that watches layout with a `ResizeObserver` and reports changes.
- **Loading:** the view stays hidden, with a skeleton in its place, until the document has loaded and been measured (at most one second), then appears in one frame at its final height. Re-showing a loaded tab is instant.
- **Theme:** the app's light/dark preference sets the default text and background colors and the page's `color-scheme`; colors the email sets itself are not inverted.
- **Safety:** JavaScript never runs in page content, there is no in-view navigation, downloads and new windows are refused, and the message is loaded without a base URL (no local file access).
- **Images:** remote images load by default. **Settings → Reader → Block remote images** blocks every remote subresource (images, CSS, fonts, media) with a WebKit content rule list, compiled once and cached, and shows the count of blocked `<img>` elements. Changing the setting reloads open readers.
- **Links:** HTTP(S) and `mailto:` links open in the system browser; other schemes are ignored.
- **Keyboard and copy:** the view never takes key focus, so every shortcut stays with the app. Select text with the mouse and press `cmd-c`: the app asks WebKit for the selection and copies it. A click elsewhere in the window clears the selection.
- **Overlays and drags:** GPUI cannot paint over a native view, so while the command palette, a dialog, a menu, a popover, a toast or a drag is up, the view is swapped, in the same frame, for a snapshot at the same bounds and hidden; it comes back when they close. Snapshots of the visible part are taken ahead of time once the view settles, kept only for views on screen, and freed on hide; with none ready the page background is shown instead.
- **Clicks:** a click inside the page pins a preview tab, like a click anywhere else in the reader.
- **Reader mode (`v`)** shows the text part, or text extracted from HTML-only mail. Find automatically uses the text path for messages with matches.

CSS-referenced remote content other than `<img>` is blocked silently when blocking is on. See `docs/reader-spec.md`.

### Where the controls live

The chrome stays out of the way: a sidebar of locations, one titlebar, and actions next to what they act on.

**Sidebar** — a small muted **Mailboxes** heading over *All Inboxes* (the default landing view), then one foldable row per account (its icon, its name, and its Inbox count once folded). Below an open account sit Inbox, Snoozed, Sent, Archive and Trash, then its folders: tall (36px) single-line rows with a theme-tinted outline icon (Mails, Inbox, AlarmClock, Send, Archive, Trash, Folder/FolderOpen), the label, and — for All Inboxes, Inbox and Snoozed when they are not empty — a bold label and a compact count capsule whose colours are the theme's foreground and background, inverted, at the far right of the row. Hover and the active row are the same rounded, full-width highlight, hover the lighter one. Names stay on one line and ellipsize when the pane is narrow. Only subfolders get tree rails (faint grey, accent along the path to the active folder); a folder with children has a fold caret at the right end of its row. `g`-prefixed jumps and the Filter ▾ menu reach every location.

**Titlebar** — the sidebar toggle (`cmd-b`) and the current view on the left, `Search…` in the middle (`/`), then **Triage** (`t`), the pane-layout toggle (`alt-l`), a Settings gear (`cmd-,`) and **More**. Below 800px wide Triage shows only its icon. Commands, Undo and Shortcuts live in **More**, the palette and their own shortcuts.

**Message menu (`⋯`)** — every expanded message in the reader has a **Reply** button (`r`) and a **⋯** menu, and both act on *that* message, not on whichever one the reader is opened on. The menu holds *Archive* (`e`), *Delete* (`d`), *Snooze…* (`s`) and *Move to inbox* (`i`) (each only when it would change something), *Accept AI* (`y`) / *Reject AI* (`n`) while a suggestion is pending, *File…* (`f`), *Mark spam…* (`!`), *Toggle select* (`x`), *Summarize thread* (`z`), *Mute thread* (`m`), *Unsubscribe* (`shift-u`), *Open sender in new tab* (`p`: a pinned list tab with `from:<address>` — an exact, case-insensitive address match across every account and folder, like a search with its `in:` removed; pressing it again focuses that tab) and the **Sender actions** submenu: *Archive · Delete · File · Move to inbox from this sender* (`shift-e` / `shift-d` / `shift-f` / `shift-i`).

Kind and Urgency menus (including reader chips) and palette reset commands appear only for user-overridden fields. An explicit manual choice counts as an override even when it matches the classifier; resetting without a cached suggestion clears the label.

**List header** — one row that wraps to the list's width: the folder name and how many messages it shows (`Inbox · 100`, muted threads not counted), then the filters as removable pills, `+ Filter` (`l`) and `Clear`. The list is always one query over all mail and the pills *are* that query: there is no hidden filter. The folder is an `in:` value, hidden while it is the only one and shown as a pill as soon as anything else is applied, so removing it turns the list into a search over every folder. Selecting a folder in the sidebar starts over with just its `in:`. Values of one field sit side by side with an `and`/`or` toggle between them (click it to switch); click a value to edit it (dates get a calendar, and typed values like `7d` still work) and its × to remove it. `+ Filter` asks for a field (Tag, From, To, Cc, Bcc, Subject, Body, Date before/after/on, Is, Kind, Account, In) and then a value: a fuzzy list, free text or a date. `1` clears the filters (the folder stays), `2`–`4` and `6` toggle the Needs Reply, Follow Up, Urgent and Possible Spam pills, and `esc` clears them too. Filter changes are view state, not undo steps.

**Selection menu (`⋯` in the list header)** — with two or more messages selected, a **⋯** next to the selection count opens the actions that make sense for several messages: Archive, Delete, Snooze…, Move to inbox, File… and Mark spam…. They apply to the whole selection in one undo step.

Nothing was dropped: every action is still reachable from the titlebar, the menus, the command palette (`cmd-k`) and its own shortcut.

### Settings

Settings (`cmd-,` or the titlebar gear) is built on gpui-kit's `Settings` component: a sidebar of pages (Accounts, Appearance, Inbox, Reader, AI, Senders) with a search box, over grouped, described setting rows (switches, dropdowns, number inputs). Search matches titles, descriptions and keywords such as *avatar*, *stacked* or *unblock*. Settings are **mouse/Tab-driven**: click or Tab to a control and change it. The old settings shortcuts (`j`/`k` row cursor, Space/Enter, `=`/`-`, Ctrl-Tab sections, `/` search) were removed with the hand-built panel; the only settings key is **Esc**, which closes the panel. Controls cover the **Jev connection** on the AI page (the API key — masked, stored in the OS keychain and never shown again, with an explicit **Remove key** — the base URL and model, **Save** and **Test connection**, which sends one request and saves nothing), per-question classifier handling (Auto/Review) and confidence threshold (0–100%, step 5; locked in Review), thread summaries, theme, pane layout, sender avatars in tabs, grouping, preview lines, **Follow-up after** (1–14 days) and the Blocked senders list (each with **Unblock**, plus the read-only unsubscribed list).

To add a setting, add a `SettingItem` (title, description, `.keywords(..)`, and a `SettingField` switch/dropdown/number input) to a page in `src/app/settings.rs`, connect its getter/setter to a `SettingsPanel` field through the captured `WeakEntity`, and emit the matching `SettingsEvent` for `MailApp` to consume.

### Icon legend

Defined once in `src/app/icons.rs` (`Glyph::spec`: icon, theme token, label, description) and shown at the end of the `?` help. Lucide icons, tinted with theme tokens.

| Icon | Meaning | Token |
|---|---|---|
| FolderInput | Suggested folder, shown while pending (`y` accept, `n` reject) | accent |
| ShieldAlert | Possible spam | possible_spam |
| Reply | Needs reply | needs_reply |
| Hourglass | Awaiting reply (a reply was sent, an answer is expected) | awaiting |
| CornerUpRight | Follow up (the await passed the timeout) | follow_up |
| AlarmClock | Reminder (a snooze woke the message) | reminder |
| Clock | Snoozed (wake time shown as text on the row) | state_snoozed |
| Siren / Flame | Urgency 5 (very urgent) / 4 (urgent); levels 1–3 show no row icon | urgent |
| Gauge | Urgency 1–3 as a reader chip (rows show nothing) | muted |
| BellOff | Muted thread | muted |
| Paperclip | Subject or opening lines mention an attachment | muted |
| User / Receipt / Newspaper / Bell / Tag | Kind: person / receipt / newsletter / notification / other (other only in `+N`) | kind |

To add a concept: add a `Glyph` variant, list it in `Glyph::ALL`, fill its `spec` arm and add the SVG name to `icon_assets!`. The reader's **Classify** button and the global *Classify* command use the `ScanSearch` icon.
The reader hides **Classify** once the message has cached answers; `c` and the global command can still classify it again.

**Controlling cost.** Automatic classification is opt-in (Settings → AI → *Classify new mail automatically*) and only considers mail newer than *Don't auto-classify mail older than* (Never skip · 1 day · **1 week, the default** · 1 month · 3 months · 1 year; compared against the message's received time). Older mail is never queued automatically but `c` / *Classify every message in this list…* still classify it. The palette commands **Stop classifying** and **Start classifying**, or a click on the status bar text (*Classifying n/m · click to stop* / *Auto-classify off · click to start*), turn automatic classification off or on; the choice is stored like the switch, so a stopped classifier stays stopped after a restart. Stop drops every queued request; a request already sent cannot be aborted, its answer is discarded when it lands. Manual classify works while stopped. Neither command has a default key.

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
its directory) on first launch and migrated forward through `PRAGMA user_version`.
Schema 2 is the scoped settings table; schema 3 adds a `judgements` table caching the classifier's raw
answers, and schema 4 re-keys it by `message_id` so each message is billed at
most once — no model, endpoint or key change re-classifies anything. On the
first launch after that migration, the old hash-keyed rows are copied onto their
message ids (under the configured model, `jev-latest` by default) and the legacy
table is dropped; a message whose model changed before the upgrade has no
matching row and is judged once more.
Databases previously opened by the settings or classifier branch are upgraded in
place; existing contacts and settings are preserved. Unknown newer schemas
are rejected with a diagnostic and exit status 1 before the UI starts.
The app starts with an empty mailbox and contacts database; mail appears after
linking a Gmail account. Sample mailbox and contacts data lives under
`tests/fixtures/` for tests only.

### Nightly builds

`scripts/package.sh` builds `dist/Minimail-<version>-arm64.dmg` (version
`<Cargo version>-nightly.<YYYYMMDD>`). Run it locally with `.envrc` loaded;
without a Developer ID identity (`MAIL_CLASSIFIER_SIGN_IDENTITY`) it signs
ad-hoc and skips notarization. `.github/workflows/nightly.yml` runs it daily
(skipping when `main` is unchanged) and replaces the rolling `nightly`
prerelease. Repo secrets: `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET`,
`APPLE_CERT_P12_BASE64`, `APPLE_CERT_PASSWORD`, `APPLE_ID`, `APPLE_TEAM_ID`,
`APPLE_APP_PASSWORD`.

### Gmail accounts

Gmail is optional; without it the app runs on the mock mailbox. To connect it,
create a Google Cloud OAuth client of type **Desktop app**, enable the Gmail API,
add yourself as a test user, then launch with:

```sh
MAIL_CLASSIFIER_GOOGLE_CLIENT_ID=… MAIL_CLASSIFIER_GOOGLE_CLIENT_SECRET=… cargo run
```

In the palette (`cmd-k`), run **Add Gmail account**. It signs you in with the
browser over a loopback redirect (PKCE, `gmail.modify` scope) and saves the
refresh token in the OS keychain (service `mail-classifier`). After the first
account is linked, the demo mail is dropped. Fetched mail is cached in
`$MAIL_CLASSIFIER_MAIL_DB`, otherwise in
`~/Library/Application Support/mail-classifier/mail.db`, and later launches
start from that cache.

Nightly builds ship with the OAuth client baked in (the secret of a Desktop-app
client is not confidential; PKCE protects the flow), so testers only need to be
added as test users in Google Cloud. The runtime variables above still override it.

#### How syncing works

Sync is built to stay fast and inside Gmail's quotas, so it is deliberately
incremental rather than a full refresh:

- **Inbox first.** On connect the app backfills the **last 30 days**, one page
  per round, and walks the scopes in order: inbox, then Archive, then Trash,
  then each user label sorted by path. The inbox fills in first, so the app is
  useful long before the rest has landed.
- **A check at startup, then every 60 s.** Each check takes a fresh cursor and
  reads Gmail's history from the last one. The loop otherwise wakes every 2 s
  to push local triage and to continue the backfill; *Fetch mail* forces a
  check right away.
- **History deltas, no re-downloading.** Read/unread, archive, trash and label
  changes made elsewhere are applied straight from the history's label ids.
  Cached messages are never fetched again, and a downloaded body is never
  replaced by a snippet.
- **A reconcile when the cursor expires.** If Gmail no longer knows the cursor
  (or the cache predates read sync), the app lists the ids of the whole 30-day
  window, applies their flags, drops what has vanished, takes a fresh cursor
  and re-lists what the window shows. Still no per-message gets.
- **Headers-only lists.** Listings are id pages; bodies are downloaded when a
  message is opened, with the next two rows of the list prefetched. Each list
  shows a muted *Loading message…* until the body lands.
- **Scroll to load older mail.** Reaching the end of a list (or moving the
  cursor near it) asks for the next page *before* the 30-day cutoff, for that
  view's scope, while the list shows a muted *Loading older mail…* footer.
- **A client-side quota budget.** A round is deliberately small — pending
  moves and reads, one check, one load-more page, one backfill page — and the
  provider batches its gets, tracks a quota budget and retries with backoff
  before reporting a rate limit. When it still comes back throttled, rounds
  pause for about a minute without a toast.

Triage pushes back to Gmail as before: archive removes `INBOX`, delete moves
the message to Trash, filing applies a label (created if missing; nested
folders become `Parent/Child`), and snooze archives remotely and wakes locally.
Not supported yet: sending from Gmail.

### Project layout

```
src/
├── model/        # triage states, undo, snooze, outbox (pure logic; split by concern)
├── contacts/     # SQLite address book: schema + migrations, CRUD, search, groups, seeding, cached classifier answers
├── judge/        # System 1 classifier interface, Jev provider + wire mapping, routing policy
├── summary.rs    # thread summarizer interface + stub
├── search.rs     # structured query: parse, display, edit, match
├── filters.rs    # what the filter pills and picker offer (fields, values, quick filters); pure
├── provider/     # MailProvider trait, keychain secrets, Gmail adapter (OAuth, REST, label/MIME conversion)
├── sync/         # SQLite mail cache and the sync engine (pending moves, pull merge); pure
├── rules.rs      # rule suggestions
├── clock.rs      # injectable clock
├── theme.rs      # gpui-kit theme setup (built-ins, user dir watcher, apply) and derived triage colors
├── find.rs       # find in a thread: matching (case / word / regex) and the per-tab query state (pure)
├── tabs.rs       # reader tab state: preview and pinned tabs, one per thread (pure)
├── preview.rs    # plain-text snippets and the Preview lines setting (pure)
└── app/          # GPUI views: main window, palette, compose, panels; icons.rs is the icon language table, row.rs the row frame
    └── mail_app/ # MailApp split by concern: accessors, actions, modals, help, rows, list, reader (+ reader_tabs, reader_tabbar, reader_find, reader_findbar), render (+ grouping, mouse)
themes/           # built-in themes (JSON, embedded at build time)
tests/            # integration and headless UI tests; model/ and ui_ext/ are multi-module test crates (shared helpers in helpers.rs / harness.rs); fixtures/ is the test-only mailbox and address book (the `fixtures` feature)
```

### Window layout: Root, Resizable and Dock

The window's root view is gpui-kit's `Root` (`gpui_kit::open_window` wraps `MailApp` in it), which hosts the dialog, sheet and notification layers. An outer **Resizable** (`src/app/mail_app/panes.rs`) holds the sidebar beside the centre **Dock** (`src/app/mail_app/dock.rs`). The sidebar stays outside Dock so zooming a centre group does not hide it.

**Dock owns the list, reader and Triage tab groups.** Every group uses `PanelStyle::TabBar`, including singleton groups, so typography, borders and active-tab colors do not change when a second tab opens. Reader titles retain the sender avatar, preview italics and close button; preview/pin state remains in `src/tabs.rs`. The sidebar is not a dock group and keeps its existing presentation.

**Modals are kit dialogs.** The palette, folder picker, `?` help, snooze picker, Settings, rules panel and the choice/confirm dialogs are all views hosted by `MailApp::host_in_dialog` (`window.open_dialog`): the kit owns the backdrop, the surface, the slide-in and `escape`/backdrop-click dismissal (which lands in `dialog_dismissed`), while each hosted view keeps its own key context (`SnoozePicker`, `ChoiceDialog`, `RulesPanel`, `SettingsPanel`, `HelpPanel`) for its keys. `modal_open()` / `dialog_hosted()` still gate the `MailApp` actions. Toasts are kit notifications (`show_toast` → `window.push_notification`, bottom-centre, with an Undo action beside the message when the action is undoable); the `⋯`, More and Filter ▾ menus are the kit's `PopupMenu`, opened by a `dropdown_menu_with_anchor` trigger (`MailApp::menu_trigger`): the kit draws the popover, the keyboard navigation (arrows, `enter`, `escape`, `right` into a submenu; `j`/`k` are bound to its down/up too) and each row's keycap, taken from the row's action binding. `src/app/menu.rs` is only the row description and the `PopupMenu` population. The window titlebar is the kit `TitleBar` (window options from `TitleBar::window_options()`).

### Adding a theme

Triage colors (state, urgent, spam, …) are derived from the kit's `primary`, `danger`, `warning` and `base.red/yellow/magenta/cyan/blue` colors, so a theme restyles them through those keys.

- **Built-in:** add `themes/<name>.json` and list it in `BUILTIN` in `src/theme.rs`.
  The upstream collection is vendored from [GPUI Kit revision `f6438b84`](https://github.com/longbridge/gpui-kit/tree/f6438b84de2d727f6a689fa0f9cf0f5915ce0584/themes), under `themes/LICENSE-APACHE`. Every entry in each file's `themes` array is registered; the app's existing Tokyo Night palette takes precedence over the upstream variant with the same name.
- **User:** put the file in `~/.config/mail-classifier/themes/` (or `$MAIL_CLASSIFIER_THEMES`); it is watched and appears in the Settings picker. The kit does not replace a theme name that is already registered.

## 🗺️ Roadmap

- [x] Real classifier provider: TypeSafe Jev (BYOK; key in the OS keychain, endpoint and model configurable)
- [ ] More classifier providers: Ollama and OpenAI-compatible APIs
- [ ] Bring-your-own provider for summaries
- [ ] Review queue alongside the inline badges
- [ ] Folders or streams driven by classifier labels
- [ ] IMAP/SMTP accounts (behind `MailProvider`, like Gmail)
- [ ] Sending from Gmail
