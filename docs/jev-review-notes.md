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
