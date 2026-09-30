# AGENTS.md

Keyboard-first triage mail client in Rust (edition 2024) + GPUI via `gpui-kit` 0.7. Mock data only (`fixtures/`); AI providers are deterministic stubs.

- Verify with: `cargo build`, `cargo clippy --all-targets -- -D warnings`, `cargo test`. All must pass before committing.
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
- No network calls or real providers unless explicitly asked.
