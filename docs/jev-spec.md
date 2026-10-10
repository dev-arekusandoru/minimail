# Jev classifier (BYOK) — spec

Status: approved. Branch `jev-classifier`.

## Goal

Replace `StubJudge` in the triage pipeline with TypeSafe Jev using the user's own key.
Classification runs in the background, results persist in SQLite, and failures show in
the status bar. Other providers come later behind `Judge`; v1 only makes the endpoint
configurable.

## Decisions

| Topic | Decision |
|---|---|
| Provider | Jev only. Configurable base URL (default `https://api.typesafe.ai/v1`) and model (default `jev-latest`). |
| Key | OS keychain via existing `SecretStore` (`src/provider/secrets.rs`), entry `jev`. |
| No key | Classifier off: no labels. Status bar prompts "Add a Jev key in Settings → Classifier". |
| Trigger | New arrivals queued on arrival. First key added → one pass over **Inbox-state** messages without a stored result. |
| Pacing | One request per message, serial, on one background worker. |
| Failure | Message stays unlabeled; error shown in the status bar center; retried on next arrival/launch. |
| Persistence | SQLite table of raw answers. Thresholds/modes are applied at read time, so changing policy never re-bills. |

## API mapping

`POST {base}/systemone`, `Authorization: Bearer <key>`, body
`{ state, model, questions: { <QuestionKey>: {...} } }`. All triage questions go in one request.

| `Question` | Jev `type` | answer → `Answer` |
|---|---|---|
| `Bool` | `noul` | `p = noul`; value `p > 0.5`; confidence `max(p, 1-p)` (Jev returns none) |
| `Choice` | `choice` (`criteria` = `{option: description}`) | `choice`, `confidence` as returned |
| `Score` | `score` (`criteria` = level list, **0-based**) | `score + 1` (ours is 1-based), `confidence` as returned |

`state` = existing `message_state(m)` JSON. HTTP 401/403 → `JudgeError::Unavailable("invalid key")`;
other non-2xx/transport → `Unavailable`; shape mismatch → `Malformed`. The `src/judge/mod.rs`
module doc (which assumes per-type endpoints) is corrected.

## Components

1. **`src/judge/jev.rs`** — `JevJudge { base_url, model, key }` implementing `Judge` with `ureq`.
   Pure request/response mapping is unit-tested against recorded JSON; no network in tests.
2. **`src/judge/store.rs`** — table `judgements(message_id, questions_hash, model, answers_json,
   judged_at)` in the existing contacts DB (new `user_version` migration). `questions_hash`
   covers criteria text so editing a question invalidates old results.
3. **Worker** — `classify_visible` stops calling the judge inline. It routes stored answers
   through `JudgePolicy` immediately and enqueues Inbox messages with no stored result. A
   background task drains the queue serially (gpui `background_executor`), writes the store,
   then applies routing on the main thread. Tests drive it with a fake `Judge`.
4. **Settings → Classifier** — key field (masked; blank keeps the saved key; explicit Remove),
   base URL, model, **Test connection** (one synthetic call). Saving a key triggers the Inbox backfill.
5. **Status bar** — center region shows the classifier state: missing-key prompt,
   "Classifying N…", or the last error (cleared on the next success).

## Invariants kept

- Routing still one undo step per user action; background auto-applies push no undo step
  (same as `tick`).
- No system clock in logic: `judged_at` takes `now`.
- Tests use `MemorySecrets` and a fake `Judge`; never the network.

## Resolved

- `expects_reply` on send: the reply sends immediately; Jev judges it on the worker and tags
  the thread *Awaiting Reply* when the answer arrives (no undo step; no key → no tag).
- `StubJudge` becomes test-only. Without a key the app shows no classifier labels.

## Out of scope (v1)

Other providers (OpenAI-compatible, Ollama), parallel requests, re-classify command,
classifying non-Inbox mail, usage/cost display.
