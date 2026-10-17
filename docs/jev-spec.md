# Jev classifier (BYOK) — spec

Status: implemented on branch `jev-classifier` — see `docs/jev-review-notes.md` for the
decisions to review and what is still unverified (the provider has never been run against the
live Jev API).

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
| Trigger | Automatic classification is the `auto_classify` setting (default **off**): when on, new arrivals, startup/backfill and opening a message queue work; when off, nothing is sent unless the user asks. Manual classification works with auto off, re-asks every enabled question and jumps the queue: `c` (or the reader button) for the selected message(s), `shift-c` for every incoming message in the current list after a Confirm/Cancel dialog. |
| Pacing | One request per message, serial, on one background worker. A message is only sent once its full body is downloaded. |
| Failure | Message stays unlabeled; error shown in the status bar center; retried on next arrival/launch. |
| Persistence | SQLite table of raw answers, keyed by `message_id`. Each question's answer is stored once as a `StoredAnswer { key, version, answer }`; a question is re-asked only when its version changes — so wording changes re-bill only the affected question, never a model/endpoint/key change. |

## API mapping

`POST {base}/systemone`, `Authorization: Bearer <key>`, body
`{ state, model, questions: { <QuestionKey>: {...} } }`. All triage questions go in one request.

| `Question` | Jev `type` | answer → `Answer` |
|---|---|---|
| `Bool` | `noul` | `p = noul`; value `p > 0.5`; confidence `max(p, 1-p)` (Jev returns none) |
| `Choice` | `choice` (`criteria` = `{option: description}`) | `choice`, `confidence` as returned |
| `Score` | `score` (`criteria` = the level descriptions, **0-based**) | `score + 1` (ours is 1-based), `confidence` as returned |

`state` = the incoming-message JSON: sender, `to`, `cc`, `recipient_role` (to/cc/other vs the
account's address), subject, received date, `bulk` (List-Unsubscribe/List-Id), and the body
(quoted replies kept, cut to 8000 chars from the end). Incoming mail is asked Spam, NeedsReply,
Urgency, Kind and Folder; ExpectsReply is asked only on the reply path (`reply_question`). HTTP
401/403 → `JudgeError::Unavailable("invalid key")`; other non-2xx/transport → `Unavailable`;
shape mismatch → `Malformed`. The `src/judge/mod.rs` module doc is corrected.

Question wording carries a manual `version` (`incoming_questions`): Spam (`"2"`, or `"2c"` with
the cold-outreach checkbox) and Urgency (`"2"`) changed wording in this work; NeedsReply/Kind are
`"1"`; Folder's version is a fingerprint of the account's folder list, so a folder-list change
re-asks only Folder, only for Inbox mail. Legacy `[key, answer]` rows read as version `"1"`.

## Components

1. **`src/judge/jev.rs`** — `JevJudge { base_url, model, key }` implementing `Judge` with `ureq`.
   Pure request/response mapping is unit-tested against recorded JSON; no network in tests.
2. **`src/contacts/judgements.rs`** — table `judgements(message_id, answers, model, judged_at)`
   in the existing contacts DB (`MIGRATION_4`). Each answer is a `StoredAnswer { key, version,
   answer }`. `MIGRATION_4` parks the old hash-keyed rows in `judgements_legacy`; at startup
   `MailApp::carry_over_judgements` copies the ones it can map (old hash under the configured
   model, via the frozen `legacy_judgement_key`) and drops the legacy table. All rows are loaded
   into memory at startup so `cached_answers` is a map lookup.
3. **Worker** — `classify_visible` stops calling the judge inline. It routes stored answers
   through `JudgePolicy` immediately and, when `auto_classify` is on, enqueues Inbox messages
   with due questions. A message is sent only once its full body is in hand (`body_ready` /
   `classify_message`). A background task drains the queue serially (gpui
   `background_executor`), merges each answer into the store per key, then applies routing on the
   main thread. Tests drive it with a fake `Judge`.
4. **Settings → Classifier** — key field (masked; blank keeps the saved key; explicit Remove),
   base URL, model, **Test connection** (one synthetic call), the automatic-classification and
   cold-outreach toggles, and a mode per question (including Folder).
5. **Status bar** — center region shows the classifier state: missing-key prompt,
   `Classifying X/Y` (X completed, Y the current batch's total), or the last error (cleared on
   the next success). A per-message status icon (`classification_state`) shows not-classified /
   in-progress / failed.

## Invariants kept

- Routing still one undo step per user action (accepting a Folder suggestion files the message
  and reverts as a whole, and so does setting or resetting a label); background auto-applies push
  no undo step (same as `tick`).
- No system clock in logic: `judged_at` takes `now`.
- Tests use `MemorySecrets` and a fake `Judge`; never the network.

## Resolved

- **Only a folder move is reviewed.** Every other answer (Spam, NeedsReply, Urgency, Kind) is
  applied straight away, whatever its confidence: `route` returns `Auto` for them and `Review`
  only for Folder, which files the message. The per-question mode is therefore Off / Auto for a
  label question, and Off / Review for Folder. `y`/`n` and the reader strip now only ever act on
  a folder suggestion.
- **Hand-set labels stick.** The row's classifier menu (`SetKind` / `SetUrgency` / `SetSpam`, one
  `PopupMenu` opened by any row icon or a right-click) stores the value as a user override on the
  message (`Meta::overrides`); `Mailbox::apply` skips an overridden question, so no later
  classification overwrites it. Overrides look exactly like classifier values. "Reset Kind/Urgency
  to suggestion" clears the override and puts the classifier's stored answer back; spam has no
  reset row. Every one of these actions is one undo step. The same actions are palette commands
  (`Classify`) with no key bindings.
- **The classifier menus mark the choice in force.** The Kind and Urgency submenus carry the value
  on their own row (`Kind · Receipt`), and the option it names is the one the menu ticks — at the
  row's end, because the option's glyph fills the kit's left check slot. A message with no such
  label marks no option and leaves the row unlabelled, so clearing a label clears the mark. The
  reader `⋯`, a row's right-click, its classifier icons and a reader chip all build from
  `MailApp::classifier_items`, so they show the same rows.
- `expects_reply` on send: the reply sends immediately; with `auto_classify` on, Jev judges it on
  the worker and tags the thread *Awaiting Reply* when the answer arrives (no undo step; no key or
  auto off → no tag). Reply requests contain only the ExpectsReply question.
- `StubJudge` becomes test-only. Without a key the app shows no classifier labels.
- Folder suggestions are never auto-applied (Review unless Off); accepting one files the message
  through the same path as manual filing.

## Out of scope (v1)

Other providers (OpenAI-compatible, Ollama), parallel requests, classifying non-Inbox mail,
usage/cost display.
