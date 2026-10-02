# AGENTS.md

Keyboard-first triage mail client in Rust (edition 2024) + GPUI via `gpui-kit` 0.7. Mock data by default (`fixtures/`); optional Gmail accounts sync through `src/provider/` + `src/sync/`. AI providers are deterministic stubs.

- **Workflow**: prefer a dedicated worktree per unit of work (`wt switch --create <branch>`, `wt list`, `wt merge`, `wt remove`) over sharing one tree — especially for larger features and whenever more than one agent is working at a time, since concurrent edits in a single worktree clobber each other. Small, single-threaded changes on `main` are fine. Commit one verified step at a time, each with `cargo build`, `cargo clippy --all-targets -- -D warnings`, and `cargo test` green plus a descriptive message; commit after parallel subagents finish, not while they run.
- **Layout**: pure logic in `src/*.rs` (no GPUI), views in `src/app/`.
- **Invariants**: every message has exactly one `TriageState`, and `Snoozed` iff it has a wake time; muted threads are excluded from counts. Every user action is one undo step; `tick(now)` pushes none.
- **Time**: never read the system clock in logic — take `now: Timestamp`, and use `clock::FakeClock` in tests.
- **gpui-kit**: re-exports GPUI (`use gpui_kit::*`). Check it for an existing component (Button, Kbd, Tab, Tooltip, …) before writing a custom one, and verify APIs against `~/.cargo/registry/src/*/gpui-{kit,pre}-*`, not memory. `.when`/`.when_some` need `use gpui_kit::prelude::*`; in tests import names explicitly, as the glob shadows `#[test]`.
- **Keybindings**: bind bare-letter keys under `"MailApp && !Input"`, or they fire while typing in inputs.
- **Actions**: new ones go in `src/app/actions.rs` (`bind_keys` + `commands()` with a palette `Category`), which puts them in the palette and help automatically. Palette and folder-picker matching lives in `src/fuzzy.rs`.
- **Tests**: UI tests are headless keystroke tests (`cx.simulate_keystrokes`) in `tests/ui*.rs`. Test behavior, not wiring.
- **Providers**: new backends go behind `MailProvider` (`src/provider/mod.rs` — blocking calls, run via `cx.background_spawn`, never see local ids); new classifiers behind `Judge` (`src/judge.rs` — bool/choice/score questions → probabilities + confidence). Tests use fakes, never the network; no real providers unless explicitly asked.

<!-- hippo:start -->
## Project Memory (Hippo)

At the start of every task, run:
```bash
hippo context --auto --budget 1500
```
Read the output before writing any code.

On errors or unexpected behaviour:
```bash
hippo remember "<description of what went wrong>" --error
```

On task completion:
```bash
hippo outcome --good
```

When Hippo's Codex wrapper is installed, session-end capture runs automatically.
If the wrapper is not installed, capture a brief summary manually:
```bash
hippo capture --stdin <<< '<decisions, errors, lessons — 2-5 bullets>'
```
<!-- hippo:end -->
