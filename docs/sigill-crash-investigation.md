# SIGILL crash investigation

Investigation of the `cargo run` "illegal hardware instruction" crash on macOS
arm64. Status: likely cause identified but not proven; proposed fixes are not
applied. No code was changed for this write-up.

## Summary

`cargo run` (which runs `scripts/sign-and-run.sh target/debug/mail-classifier`)
died with SIGILL (exit 132) while the app was running, and relaunching crashed
too. The working hypothesis is that `target/debug/mail-classifier` was a
corrupt or half-written executable that `codesign -f` then re-signed, so the
runner happily exec'd broken content with a valid signature. A later clean
rebuild replaced the artifact and launches worked again. The corruption race
itself was never reproduced, and the system crash report was not readable, so
this stays a likely cause rather than a confirmed one.

## Symptom

- Running `cargo run` crashed with "illegal hardware instruction" (SIGILL,
  exit 132) on macOS arm64.
- The app had been running normally and crashed suddenly; relaunching
  crashed immediately as well.
- Cargo reported `Finished dev in 0.47s` (no rebuild) — from Cargo's point of
  view the target was fresh, so it re-ran the same binary.

## Likely cause (not proven)

`target/debug/mail-classifier` was a corrupt or half-written executable that
still carried a valid signature. Contributing factors:

- The target dir was shared by multiple concurrent workers building in the
  same worktree, so several builds could be writing the same output path.
- `scripts/sign-and-run.sh` runs
  `codesign -f -s <identity> target/debug/mail-classifier >/dev/null 2>&1 || echo "warning: codesign failed..."`
  and then `exec "$@"` unconditionally. It verifies nothing, swallows signing
  failure, and re-signs whatever currently sits at that path — bad content
  included.
- Cargo fingerprints inputs, not output bytes, so a bad artifact stays
  "fresh" and gets re-run without a rebuild.
- A later clean rebuild replaced the artifact and launches worked afterwards.

## Evidence

1. Measured exit codes on this machine: Rust panic = 101, `abort` = 134
   (SIGABRT), stack overflow = 134, `brk #1` = 133 (SIGTRAP), `udf #0` = 132
   (SIGILL). So SIGILL means an undefined instruction / non-code was executed,
   not a normal Rust failure mode.
2. Reproduced in isolation: zeroing the 64 bytes at the Mach-O entry point of
   a test binary and re-signing it exits 132 (SIGILL); the same file unsigned
   exits 137 (SIGKILL). The crashing binary therefore had corrupt content
   *and* a valid signature — i.e. the runner signed bad content.
3. No `unsafe`, `unreachable_unchecked`, `get_unchecked`, `transmute`, or
   `assume` in the new composer/drafts code (only the pre-existing
   `native_clipboard.rs` has `unsafe`). Debug panics exit 101 and panics
   across `extern "C"` abort with 134 — neither is SIGILL.
4. `codesign -f` replaces the file (inode changes) and re-stamps Signed Time
   on every launch (observed 16:53:25 then 17:16:13 with an identical CDHash).
5. Persisted state ruled out: `PRAGMA integrity_check` = ok for `mail.db`,
   `contacts.db`, and `drafts/drafts.sqlite`; all 10 draft rows parse as valid
   JSON.
6. Not reproducible later: the current binary launched 8 times and stayed
   alive; `cargo run --example composer_smoke` printed `COMPOSER_SMOKE_READY`
   and ran until timeout.
7. Concurrent `codesign -f` on one path can emit errors under contention
   ("error: interrupted", "No such file or directory"), but isolated 60- and
   200-iteration races never produced a crashing binary.

## What is NOT confirmed

- The `.ips` crash report in `/Library/Logs/DiagnosticReports`
  (owner `root:_analyticsusers`, mode `drwxrwx---`) was unreadable without
  sudo / `_analyticsusers` membership, so the faulting address was never
  checked.
- The race that corrupted the binary was never reproduced.
- If the crash recurs after a clean build, treat it as a real bug (possible
  use-after-free or corrupted ObjC / GPUI pointer) rather than the artifact
  race.

## Proposed fixes (not applied)

- **(a) Immediate:** `rm -f target/debug/mail-classifier && cargo build`, then
  `codesign --verify --verbose=4 target/debug/mail-classifier`.
- **(b)** Give each concurrent agent its own `CARGO_TARGET_DIR` or, per
  `AGENTS.md`, its own worktree.
- **(c)** Harden `scripts/sign-and-run.sh`: verify the binary/signature first
  (`codesign --verify --strict`), capture `codesign`'s exit status and abort
  instead of warn-and-exec, and avoid blind `codesign -f`.
- **(d)** Optional guard: compare the hash of the freshly built artifact with
  the file about to be exec'd.
- **(e)** Read the `.ips` report with sudo to see whether the fault is in
  `__TEXT` at a zero/`udf` word (corruption) or a bad pointer.

## Open questions

- Whether to apply (b) and (c) now.
- Whether to read the `.ips` report.
