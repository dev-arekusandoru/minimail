# Jev classifier: decisions to review

Design calls made while building `docs/jev-spec.md`, grouped by topic. Each says what was
decided, why, and how to flip it. Flip any you disagree with.

## Unverified

- **Never run against the live Jev API.** No key was available, so the wire mapping is only
  tested against recorded JSON. **Do this first:** Settings → AI → **Test connection** with a
  real key, then **Save**, and watch the status bar. If a request succeeds but answers look
  wrong, the fix is in `parse_answers`/`parse_one` (`src/judge/jev.rs`) — check that a `noul`
  answer reads `noul`, `choice` reads `choice`/`probabilities`, and `score` reads `score`
  (0-based, +1 here).
- **Request shape assumed, not confirmed** — one `POST {base}/systemone` with
  `Authorization: Bearer <key>` and `{state, model, questions}` where each question key is the
  `Debug` name of `QuestionKey` (e.g. `"Spam"`). If Jev names them differently, change
  `request_body`/`parse_answers` together with `judgement_key`.
- **Rate limits and long messages untested.** One request per message, serial, 30 s timeout,
  body trimmed to 1500 chars. No batching or backoff beyond "stop the worker on an error".
- **Local loopback `http://`** is allowed for a local proxy but has never been exercised.
- Score levels are sent as `"1"`…`"5"` and choice criteria as `{option: option}`; no per-level
  descriptions exist in `Question`.
- The window did not open against the real keychain for this branch; the delete-failure path is
  covered by a fake `SecretStore` in tests, not the real one.

## Transport and secrets

- **HTTPS required, loopback `http://` allowed.** `jev::valid_base_url` accepts `https://`
  anywhere, and `http://` only for `localhost`, `127.0.0.1`, `::1`. The agent sets
  `https_only` accordingly and `max_redirects(0)`, so a response cannot downgrade the
  transport. *Why:* the bearer key and message text must not go in the clear. *Flip:* loosen
  `is_loopback`/`valid_base_url` in `src/judge/jev.rs` if you want plain HTTP elsewhere.
- **Non-2xx failures are status-only.** `failure(status)` never echoes the server body, so an
  endpoint that mirrors the request cannot smuggle the key into the status bar. *Flip:* only by
  re-adding a body excerpt that is scrubbed of the key first (see the `Wave 4` note).
- **Keys are redacted in `Debug`.** `JevJudge`'s manual `Debug` and the `JevKey` newtype on
  `SettingsEvent` both print `<redacted>`. *Flip:* nothing to flip; this is a safety invariant.

## Cache and persistence

- **Cache key = content hash, not message id.** `judgement_key` is `sha256(model +
  questions JSON + state JSON)`; `MessageId` is a per-load `u32`, so it is not stable across
  restarts. *Why:* answers must not be reused after a question or the message text changes.
  *Flip:* change the hash inputs in `judge::judgement_key`.
- **Table is `judgements(key, answers, judged_at)` in the contacts DB** (`src/contacts/`,
  `MIGRATION_3`), not the `(message_id, questions_hash, model, …)` shape the spec sketched.
  The key covers all of it, so one column suffices. *Flip:* a schema migration if you want the
  parts queryable.
- **The cache never prunes** — one small row per classified message. *Flip:* add a job if it
  ever grows.
- **A corrupt row is a cache miss**, not an error: judging again overwrites it.
- **Thresholds/modes apply at read time**, so changing policy never re-bills.
- **A message whose body downloads mid-request is re-classified, not applied.** After a
  successful request the worker recomputes the current key; if it differs the stale answers are
  dropped and the message is re-queued (this bills the full body once more). *Why:* a
  snippet-based answer must not be applied to the full message, and a pending suggestion would
  otherwise block the full-content pass. *Flip:* the check in `MailApp::message_changed`.

## Worker and lifecycle

- **One serial worker** (`judge_queue` + one in-flight request). Reply jobs cannot be rebuilt
  from the mailbox, so they survive both a judge swap (`set_judge` keeps `Reply` jobs) and an
  error (`finish_judge` keeps queued and failed reply jobs for the next explicit retry, while
  message jobs are rebuilt by `classify_visible`).
- **The in-flight slot stays occupied across a judge swap.** Changing the key, endpoint or model
  bumps `judge_gen` but does not free the slot; the stale result is discarded when it lands and
  the replacement's queue drains then. *Why:* repeated Saves must not create concurrent
  requests or re-bill a message in flight. *Flip:* `set_judge`/`finish_judge` in
  `src/app/mail_app/judging.rs`.
- **Reply classification is per send, not per message.** `queue_reply_at` returns the outbox
  sequence; the `JudgeJob::Reply` carries it, and `mark_awaiting_reply(original, seq, now)` tags
  only while *that* send is still in the outbox or sent (`send_in_flight`). *Why:* an earlier
  reply to the same message must not make a recalled later reply look in-flight.
- **An error stops the worker and shows in the status bar**; the next `classify_visible`
  (sync, `c`, settings change, relaunch) retries.
- **`Working(n)` counts the queue plus the one in flight**, so the last message still reads
  "Classifying 1…" while its request is out.
- **Background auto-applies keep the existing undo behavior** (`Mailbox::apply_auto` pushes a
  step), so `u` can undo a label that arrived in the background — the spec said "no undo step";
  the model's behavior was kept instead of changing it.

## Settings and UX

- **Removing the key disables the judge immediately and only clears the panel's saved-key state
  once the keychain confirms deletion**; a failed deletion shows inline and leaves the key
  visible. *Why:* the user's removal must never leave mail being sent with the old key while the
  UI says "No key". *Flip:* `MailApp::remove_jev_key` / `SettingsPanel::set_jev_removed`.
- **Reset all rebuilds the judge** through `configure_jev`, so the worker stops using a custom
  endpoint/model the reset just dropped.
- **The Jev key status is the field's placeholder** ("No key" / "Key saved"): a row control only
  gets the space left after the 60%-max text column, and a status label plus input plus button
  overflowed the window.
- **The settings window grew to 820×760**: the AI page is a virtual list, so four new rows
  pushed the Tagging confidence row out of the viewport. AI page groups are Jev connection (0),
  Thread summaries (1), Tagging (2); tests that click by group index were renumbered.
- **Test connection runs on the app**, not the panel: the panel emits the typed values, the app
  builds a judge through the factory, sends one synthetic `Bool` request in the background, and
  writes the result back. Nothing is saved.
- **A blank key field on Save means "endpoint/model only"**; the key is never redisplayed.
- **The panel's Jev rows have no row ↺** (their values live in `InputState`, unreadable without
  `cx`); the page reset and Reset all still put the endpoint/model back.
- **No-key prompt opens Settings on the AI page**: `HintBar::on_status_click` carries a callback
  and `open_settings(page)` replaced the old `accounts` bool.

## Mapping and policy

- **`JevJudge` builds its `ureq::Agent` per request** (the struct fields are fixed to
  `{base_url, model, key}`; one request per message makes reuse moot).
- **Score answers are shifted to 1-based** (`score + 1.0`) and their probabilities indexed
  `"0".."levels-1"`; missing keys read 0. **noul confidence is `max(p, 1-p)`** (Jev returns
  none); choice/score confidence is passed through.
- **`route` is a public function** in `src/judge/policy.rs`; `classify` delegates per message.
- **`Judge::model` defaults to `"stub"`** and `JevJudge` overrides it with its alias; it is part
  of `judgement_key`, so answers never cross models.
- **`StubJudge` stays `pub`** but production never constructs it (`main` installs `JevJudge`);
  test harnesses inject it.
- **Rejected suggestions can come back** — `classify_visible` re-routes every Inbox message with
  no pending suggestion (from the cache, no new API call). Same as the stub's behavior.

## Wave 4: review follow-ups

The code review returned nine findings, all marked must-fix; all nine are addressed above. No
unresolved must-fix or nice-to-have remains. Deliberately left for later:

- **A re-classify command** and classifying non-Inbox mail are out of scope for v1 (spec).
- **Error bodies are dropped entirely.** If a future provider needs the server's message, add it
  through a scrubber that removes the key before it crosses the provider boundary.
- **No paragraph-level tests for the worker races** (judge swap mid-flight, body download
  mid-flight, reply-job retention): the model-level reply test exists, the rest are reasoned,
  not exercised. Add a controllable (oneshot) fake judge in `tests/ui_ext/` if these need
  coverage.
