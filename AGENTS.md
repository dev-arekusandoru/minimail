# AGENTS.md

Keyboard-first triage mail client in Rust (edition 2024) + GPUI via `gpui-kit` 0.7. Mock data by default (`fixtures/`); optional Gmail accounts sync through `src/provider/` + `src/sync/`. AI providers are deterministic stubs.

- Never work on `main`. Do all work on a dedicated branch in its own worktree, managed with the `wt` CLI (`wt switch --create <branch>`, `wt list`, `wt merge`, `wt remove`). Don't hand-roll `git worktree`/`git branch` commands.
- Verify with: `cargo build`, `cargo clippy --all-targets -- -D warnings`, `cargo test`. All must pass before committing.
- Commit as you go: one commit per completed, verified step (build, clippy, tests green), with a descriptive message. Don't batch a whole feature into one commit or leave finished work uncommitted. When parallel subagents edit disjoint files, commit only after they finish and the tree is green.
- Pure logic lives in `src/*.rs` (no GPUI); views live in `src/app/`. Keep business logic out of views.
- Invariant: every message has exactly one `TriageState`; `Snoozed` iff it has a wake time; muted threads are excluded from visible state counts.
- Every user action is one undo step. `tick(now)` never pushes undo.
- Never read the system clock in logic: take `now: Timestamp` and use `clock::FakeClock` in tests.
- `gpui-kit` re-exports GPUI (`use gpui_kit::*`). Verify APIs against `~/.cargo/registry/src/*/gpui-{kit,pre}-*`, not memory.
- Before writing a custom element, check gpui-kit for an existing component (Button, Kbd, Tab, Tooltip, …) and use it.
- Bind bare-letter keys under `"MailApp && !Input"`, or they fire while typing in inputs.
- `.when`/`.when_some` need `use gpui_kit::prelude::*`. In test files import names explicitly (a `gpui_kit::*` glob shadows `#[test]`).
- New actions go in `src/app/actions.rs` (`bind_keys` + `commands()`, with a palette `Category`), which makes them appear in the palette and help automatically. The palette and folder picker are the kit's `Command` in a `window.open_dialog`; our matching lives in `src/fuzzy.rs`.
- UI tests are headless keystroke tests (`cx.simulate_keystrokes`) in `tests/ui*.rs`. Test behavior, not wiring.
- Classifier (`src/judge.rs`) mirrors TypeSafe Jev's API (bool/choice/score questions → probabilities + confidence). Keep new providers behind the `Judge` trait.
- Mail backends implement `MailProvider` (`src/provider/mod.rs`): blocking calls, run via `cx.background_spawn`; providers never see local ids. Tests use fake providers, never the network. No other network calls or real providers unless explicitly asked.

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
