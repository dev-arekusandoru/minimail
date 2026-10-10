# Backlog

Work that was asked for, proposed, or deferred between 2026-09-30 and 2026-10-06 and is not on `main` as of `622e905`. Sources: omp session transcripts, hippo memory, and `docs/*.md`; each item was checked against the code on `main`.

Status: **not started** · **partial** (some of it landed) · **unverified** (landed, never confirmed on screen or against the real service).

## Unlanded branches and worktrees

Finished or half-finished work that only exists outside `main`. Land or discard these first; several items below depend on them.

| Branch / worktree | vs `main` | What's there |
|---|---|---|
| `email-composer` | 11 ahead, 27 behind; 14 uncommitted files | Gmail sending (new, reply, reply-all, forward), durable drafts with Gmail sync, Markdown body + preview, attachments, inline reply and draft tabs, 10 s undo hold. `docs/composer-deferred.md` and `docs/sigill-crash-investigation.md` live only here. |
| `email-composer-drafts` | 11 ahead, 27 behind | Per-account Drafts pane replacing the drafts picker. Never merged into `email-composer`. |
| `nightly-build` | 2 ahead, 27 behind | `scripts/package.sh` (DMG), GitHub nightly workflow (cron `0 7 * * *`), bundled OAuth fallback, `docs/in-app-update.md`. |
| `add-llms` worktree | branch 0 ahead; **37 uncommitted files** | Real BYOK thread summaries. The branch ref never moved; the whole implementation is uncommitted in the worktree. Commit it before removing that worktree. |

## Composer (on `email-composer`)

- **Answer the five Drafts-pane decisions** (not started). Search filtering vs hiding pills; ordering and excluding sent drafts; `cmd-shift-d` renamed "Go to drafts" under Navigate; Drafts above Sent in the sidebar; discard only via `d`/`#` with confirm.
- **Fix body typing lag** (partial). Every keystroke re-renders the whole window and copies the body about four times. The notify guard landed; the missing `.cached()` boundary at Root → MailApp → Composer did not.
- **Finish the second feedback batch** (partial). The worker died mid-batch. Confirm on the branch: toolbar styled like the kit example, footer Close removed, light-mode To/Cc/Bcc/Subject backgrounds, draft tab icon, discard shortcut and hint, kit-style attachment cards.
- **Hard flush of drafts on quit** (partial). `flush_dirty_drafts` starts writes but never joins the lane, so the last keystrokes can be lost on quit. You chose best effort for now.
- **Signatures** (not started). Open: storage, per-account or not, placement relative to the quote, interaction with Markdown.
- **Live formatted (WYSIWYG) editor as a setting** (not started).
- **Keep pasted rich-text styling** (not started). Fonts, colours and sizes are lost because paste converts to Markdown; this needs a richer body format or an HTML part carried alongside.
- **Send later** (not started). Only the 10 s undo hold exists.
- **Templates and snippets** (not started).
- **Attach files dropped from Finder** (not started; dock slice S5).

## AI summaries (in the `add-llms` worktree)

- **Land real BYOK summaries** (partial). Named OpenAI-compatible and native Anthropic connections, keys in the keychain, HTTPS-only except loopback, a connection test. `main` still runs `StubSummarizer` synchronously (`src/app/mail_app/actions.rs`).
- **Persistent, async summary refresh** (partial). A per-thread cache kept across restarts; refresh only opened, stale threads; keep the old output on failure; drop stale results.
- **Instruction-change banner** (partial). Regenerate / Keep existing at the top of the reader, not a popup, remembered per instruction revision.
- **Check live model quality** (unverified). Only synthetic responses were tested.
- **Provider sign-in** (not started, optional). For example OpenRouter PKCE, after BYOK.

## Gmail and sync

- **Send from Gmail** (not started on `main`). Sending from a Gmail account is blocked with a toast in `modals.rs`; the `email-composer` branch implements it.
- **Move OAuth credentials off env vars** (not started). `ClientConfig::from_env` is the only path. `nightly-build` adds a bundled fallback.
- **Server-side search** (not started). Search phase 4: local results instantly, then merge server results. Needs a `MailProvider` search method with a Gmail implementation and a fake.
- **Answer the expired-cursor question** (unverified). You asked whether an expired Gmail history cursor will be a problem once the app leaves Google's testing mode; the session died before replying. The code reconciles `CursorExpired`. The related 7-day refresh-token expiry in testing mode was never covered.
- **Confirm the empty-sync root cause** (unverified). Fix `8a6d0b0` worked, but the bug was never reproduced, so the cause is inferred.
- **Confirm the keychain prompt stays away across rebuilds** (unverified). Both fixes landed (`207be29`, `81f3ab2`); neither was checked against the real keychain.
- **Linux and Windows auth/keychain** (not started). Dependencies are configured; nothing has run outside macOS.
- **Provider-level block and unsubscribe** (partial). Sender actions only hide mail locally.

## Search and list

- **Inline search input in the title bar** (not started). Search phase 3: autocomplete dropdown and live token highlighting. The title-bar search is still a button that opens the palette.
- **Move the result count to the title bar** (not started). Show it after the folder name and drop "All Inboxes · 100" from the list header (`filters.rs`).
- **Redesign mute** (not started). Mute still hides threads through a separate set, outside the triage-state model (`triage-spec.md`).

## Contacts and senders

- **Import contacts from the provider** (not started). Every sender shows as unknown; the contacts DB is only seeded from fixtures.
- **Sender profile photos** (not started). Avatars are initials only.
- **Re-enable the Screener after v1** (not started). Flip `KNOWN_SENDERS_ENABLED` in `src/known_senders.rs` and re-verify.

## Reader and email webview

- **Search inside HTML emails** (partial). Find switches HTML mail to plain text; highlight matches in the rendered page instead.
- **Keep highlighted text selectable** (partial). Highlighting swaps `SelectableText` for `StyledText`, which breaks selection while find is active.
- **Right-click Copy in the reader** (not started). `cmd-c` already works.
- **Search message headers in find** (not started). Find covers the subject and bodies but not From/To/Cc.
- **Tooltips and hover cards under the email** (not started). GPUI tooltips and hover cards can still be hidden by the native webview; the snapshot swap only covers overlays, toasts, menus and drags.
- **Wide emails are clipped** (not started). Emails wider than the reader are cut off instead of scrolling sideways. Consider zoom-to-fit.
- **Inner email backgrounds** (partial; you deferred it). Only the outer wrapper's background is removed. Inner boxes keep theirs, and dark text an email sets can land on the dark theme.
- **Headless test for the reader scrollbar** (not started). It was attempted, but gpui-kit's test API couldn't resolve `reader-scrollbar`. The fix is only verified by hand.
- **Confirm on screen** (unverified): outer-background strip, skeleton colour, help-panel scrollbar, find bar layout and highlight colours.

## Dock and drag-and-drop

- **Drop messages on sidebar folders** (not started; dock slice S4). Rows can be dragged but don't accept drops. Planned: `drop_messages` → file as one undo step; Snoozed opens the wake-time picker; no-op moves add no undo step.
- **Drag attachments and messages out to Finder** (not started; dock slice S6).
- **Folder dropped on the reader strip still previews** (partial). You rejected a custom drop renderer; pick one of the options in `dock-refinements.md` §1.
- **Triage tab open questions** (not started). Can a session outlive its zoom (pinnable Triage tab)? Should middle-clicking a sidebar folder open a pinned list tab? Should the list group get a zoom control? `dock-refinements.md` says it already has one, but `zoom_control` returns `None`.
- **Session state bleeds into an unzoomed split** (partial). With Triage unzoomed mid-session, clicks in your own list act on the session (`list_strip.rs` `slot()` defaults to Triage). Both readers share one scroll handle per thread, and archiving in a session closes your preview reader tab.
- **Center-pane minimum sizes on drag** (partial). Keyboard resizing clamps, but dragging the native divider can still shrink the list or reader too far.
- **Persist pane sizes and dock layout** (not started). Explicitly out of scope "until we want it".
- **Verify the Triage zoom layout and zoom button on screen** (unverified).

## Sidebar and settings

- **Style account headers as section labels** (partial). The FOLDERS label is gone, but account titles still look like ordinary rows (`sidebar.rs`).
- **Full Lucide icon search in the account icon picker** (not started). It shows a fixed 16-icon grid.
- **Keep the settings search text across page changes** (not started). Changing page rebuilds the kit Settings tree and clears the query.
- **Test confirming account removal** (not started). Only the cancel path is tested because confirming hits the real keychain; route it through the `MemorySecrets` fake.

## Release, build and crashes

- **Run the first nightly** (not started). Land `nightly-build`, then confirm certificate import, notarization and Gatekeeper on a real run.
- **In-app updates** (not started). The plan is in `docs/in-app-update.md` on `nightly-build`: stage 1 checks every 24 h and shows a toast plus a "Check for updates" command; stage 2 auto-updates via `latest.json`.
- **App icon** (not started). Add `AppIcon.icns` and `CFBundleIconFile`; about 5 lines in `package.sh`.
- **Proper `.app` bundle for local builds** (not started). Covered by `package.sh` if `nightly-build` lands.
- **SIGILL hardening** (not started). Make `scripts/sign-and-run.sh` abort when `codesign --verify --strict` fails instead of warning and running anyway; give concurrent agents separate `CARGO_TARGET_DIR`s; read the `.ips` report. The email-HTML stack overflows are fixed by the WebKit reader.
- **`block v0.1.6` future-incompatibility warning** (not started). A transitive dependency; find which crate pulls it in and upgrade or patch it.
- **Shared Cargo target dir across worktrees** (not started; never answered). It would save 10–20 GB per worktree, at the cost of builds queueing on the lock.

## Verification setup

- **Let agents drive the running app** (not started). Screen recording works, but clicks need macOS Accessibility permission for the agent's terminal. Most "unverified" items above stem from this.
- **Visual check of the 39 bundled themes** (unverified).

## Docs

- **Refresh `README.md` and `AGENTS.md`** (partial).
  - `AGENTS.md` says mock data is the default; the app now starts empty.
  - README still says the dock was deferred.
  - README still describes Settings as a dialog with only an Esc key at `src/app/settings.rs`.
  - README doesn't mention list tabs, the Triage tab or drag-and-drop.
  - `dock-refinements.md` still opens with "Nothing here is implemented yet".
- **Bring `composer-deferred.md` and `sigill-crash-investigation.md` to `main`** with the composer branch.
