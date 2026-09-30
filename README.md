<div align="center">

# ✉️ mail-classifier

**A keyboard-first email client that treats your inbox as a to-do list.**

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
| ✅ **Inbox as a to-do list** | Every message is in exactly one of Inbox, Waiting, Later or Done. Every action can be undone. |
| 📦 **Batch triage** | Multi-select with `shift-j`/`shift-k` or `x`, or act on everything from one sender at once. |
| 🧠 **System 1 classifier** | Labels spam, needs-reply, urgency, kind and a suggested state. Each label either applies itself above a confidence threshold or shows as a badge for you to accept or reject. The interface mirrors [TypeSafe's Jev](https://typesafe.ai/blog/introducing-system-one-models-and-jev). |
| 📝 **Thread summaries** | Opt-in. Shows the summary, action items and dates above the thread. |
| ⏰ **Snooze with a return time** | Tonight, tomorrow, Monday or a custom time (`3h`, `2d`). The message returns to the Inbox when it's due. |
| 🔁 **Follow-ups that come back** | Waiting threads with no reply after 3 days return to the Inbox, tagged *no reply*. |
| 🛡️ **Screener** | First-time senders wait until you allow or block them. Blocking hides mail and never deletes it. |
| 🪄 **Rules from your habits** | Do the same thing to one sender twice and it offers to make it a rule. |
| 🔍 **Search** | `from:`, `subject:`, `is:`, `before:`, `after:` and free text. |
| 🎯 **Triage sessions** | Go through the Inbox one message at a time and finish with a count and the time taken. |
| ↩️ **Undo send** | Replies wait 10 seconds in an outbox before they go. |
| 🔕 **Mute & unsubscribe** | One key each. |
| 🧵 **Group by thread** | `g` (or Settings, or the palette) shows one row per conversation: latest sender and subject, message count, participants and newest date. `right`/`enter` or the chevron expands it inline, `left` collapses. Actions (`e` `w` `i` `l`, `x`, shift-selection) on a thread row apply to every message of that thread *in the current panel* and undo as one step; sender-wide actions and mute are unchanged. A thread split across panels shows only that panel's messages in each. `]` / `[` step through the thread in date order in either mode (also clickable in the reader). |
| 🎨 **Themes** | Atom One Dark Pro by default, plus Tokyo Night. Pick one in Settings (`cmd-,`, last row: space or `=`/`-` by keyboard, click by mouse). Your own JSON themes load from `~/.config/mail-classifier/themes/` (or `$MAIL_CLASSIFIER_THEMES`). |

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

## 🛠️ Development

**Requirements:** Rust (edition 2024) and, on macOS, Xcode with the command-line tools, which GPUI needs to build. So far it has only been built and run on macOS (Apple Silicon).

```sh
cargo run                                   # launch the app
cargo test                                  # model, classifier, search and headless UI tests
cargo clippy --all-targets -- -D warnings   # lint
```

The UI tests run headless. They drive the real views with simulated keystrokes and a fake clock, so no window opens.

### Project layout

```
src/
├── model/        # triage states, undo, snooze, screener, outbox (pure logic; split by concern)
├── judge/        # System 1 classifier interface, stub provider, routing policy
├── summary.rs    # thread summarizer interface + stub
├── search.rs     # query parser
├── rules.rs      # rule suggestions
├── clock.rs      # injectable clock
├── theme.rs      # semantic color tokens, theme registry, JSON loader, active theme
└── app/          # GPUI views: main window, palette, compose, panels
fixtures/         # mock mailbox + known contacts
themes/           # built-in themes (JSON, embedded at build time)
tests/            # integration and headless UI tests
```

### Adding a theme

A theme is a JSON file: `{ "name": "My Theme", "colors": { "<token>": "#rrggbb", … } }` with every token from `themes/one-dark-pro.json` (background, surface, sidebar, border, text, text_muted, accent, on_accent, selection, hover, spam, needs_reply, urgent, kind, state_*, success, warning, error, info).

- **Built-in:** add `themes/<name>.json` and list it in `BUILTIN` in `src/theme.rs`.
- **User:** put the file in `~/.config/mail-classifier/themes/`; it appears in the Settings picker. A theme with an existing name replaces it; an unknown theme name falls back to One Dark Pro.

## 🗺️ Roadmap

- [ ] Real classifier providers: TypeSafe Jev, then Ollama and OpenAI-compatible APIs
- [ ] Bring-your-own provider for summaries
- [ ] Review queue alongside the inline badges
- [ ] Folders or streams driven by classifier labels
- [ ] IMAP/SMTP accounts
