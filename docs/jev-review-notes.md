# Jev classifier: decisions to review

Assumptions and design calls made while building `docs/jev-spec.md` unattended. Each entry
says what was decided and why. Flip any you disagree with.

## Planning (orchestrator)

- **Cache key = content hash, not message id.** `MessageId` is a `u32` assigned at load, so
  it may not be stable across restarts or syncs. Results are keyed by
  `sha256(model + questions JSON + state JSON)`. Side effect: when a partial (snippet-only)
  message's full body downloads, its state changes and it is classified again (billed twice).
- **Background auto-applies keep today's undo behavior.** `Mailbox::apply_auto` already pushes
  an undo step for each auto-applied label. The worker reuses it, so `u` can undo a label that
  arrived in the background. The spec said "no undo step"; I kept existing behavior instead of
  changing the model.
- **Rejected suggestions can come back.** Same as with the stub today: `classify_visible`
  re-routes every Inbox message that has no pending suggestion, so a rejected suggestion
  reappears on the next pass (from the cache, with no new API call). Unchanged.
- **`StubJudge` stays `pub`.** Integration tests in `tests/` can only see public items, so
  "tests only" means the app never constructs it; tests inject it.
- **Score levels are sent as `"1"`…`"5"`.** Our `Question::Score` has no per-level
  descriptions; the criteria sentence carries the meaning.
- **Judgement cache never prunes.** One small row per classified message.

## Wave 1: core

- **`route` is now a public function** in `src/judge/policy.rs`; `classify` delegates to it per
  message. The routing rules were copied verbatim, so `classify`'s behavior is unchanged. Wave 2
  (the worker) can replay cached answers without a judge call.
- **`JevJudge` builds its `ureq::Agent` per request.** The contract fixes the struct fields to
  `{base_url, model, key}`, so the agent is not stored; one request per message makes reuse moot.
- **`judgement_key` hashes `model + "\n" + questions-JSON + "\n" + state-JSON`.** The questions
  JSON is `request_body(...)["questions"]` (so it covers criteria/options), and the newlines keep
  the three parts unambiguous under concatenation.
- **Score probabilities are indexed `"0".."levels-1"` in order**, missing keys → 0, and the value
  is `score + 1.0` (ours is 1-based). Confidence is passed through, not recomputed.
- **noul confidence is `max(p, 1-p)`**, since Jev returns no confidence for noul answers.
- **Corrupt stored JSON is `contacts::Error::Invalid`**, not a silent `None`: a row that cannot be
  decoded is a bug worth surfacing, not a cache miss.
- **`MIGRATION_3` required editing an existing contacts test.** `upgrading_schema_one_...` rolls a
  fresh file back to v1 by dropping later tables; it now also drops `judgements`, otherwise the
  replay fails with "table judgements already exists".
- **`mod jev` is `pub`** so `crate::judge::jev::{request_body, parse_answers}` and the
  `DEFAULT_*` constants are reachable by name, as the contract requires.

## Wave 2: app

- **`JudgeStatus` lives in `src/judge/mod.rs`**, not the app: chrome.rs and the settings UI both
  need it and must not depend on `mail_app`.
- **The worker writes through to SQLite on every success, and reads the store only on an in-memory
  miss.** The in-memory map makes a second `classify_visible` (sync, `c`) free; the store carries
  it across restarts. A failed `judgement_put` is ignored: it is a cache, not the source of truth.
- **`classify_visible` now takes `cx`** (it can start the worker). Every caller — `new_with_clock`,
  `load_preferences`, `sync`, and the `c` action — passes it.
- **`Working(n)` counts the queue plus the one in flight**, so the last message still reads
  "Classifying 1…" while its request is out rather than jumping to `Idle`.
- **An error clears the queue, stops the worker, and shows in the status bar.** The next
  `classify_visible` (sync, `c`, settings change, relaunch) retries the un-cached messages; a
  later success recomputes the status and so clears the error. Nothing extra is stored per
  failure.
- **A stale in-flight result is dropped by a generation counter** (`judge_gen`), bumped by
  `set_judge`: replacing the judge while a request runs must not apply the old judge's answers.
- **`StubJudge` is gone from production; the test harnesses install it.** Each `tests/` harness
  now calls `set_judge(Some(Arc::new(StubJudge)))` (the shared `ui_ext` harness by default, with
  `harness_no_judge` / `harness_with_judge` as opt-outs), so the existing suites keep their
  labels while the real app no longer constructs the stub.
- **`c`'s toast adapts to the queue**: with work queued it says "{n} auto-applied · classifying
  {q}…", with no key "No Jev key · add one in Settings", otherwise the old
  "{n} auto-applied · {r} to review" (the last now counts cache hits only).
- **Status bar center is text-only.** The missing-key prompt ("Add a Jev API key in Settings →
  Classifier") is not clickable: the settings panel owns its own page state, so wave 3 can wire
  the click (and the AI page) when it lands. The center's estimated width is reserved from the
  hint budget in `chrome.rs`, so hints drop before the status is clipped.
- **`expects_reply` is judged on the same serial worker as an `JudgeJob::Reply`**, after the reply
  is queued with `expects_reply = false`. If the answer is yes, `Mailbox::mark_awaiting_reply`
  adds the tag **without its own undo step**: it does nothing unless a reply to that message is
  still in the outbox or sent (`reply_in_flight`), which is exactly "the send was recalled or
  undone in the meantime". Because the send's undo snapshot predates the tag, undoing the send
  still removes the tag.
- **`Judge::model` defaults to `"stub"`** and `JevJudge` overrides it with its alias; it is part
  of `judgement_key`, so answers are never shared between models.
