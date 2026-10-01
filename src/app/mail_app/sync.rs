//! Remote mail sync: a background loop that pushes pending triage moves and
//! pulls changes, plus the "Add Gmail account" sign-in flow.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use super::*;
use crate::model::{Account, ProviderKind};
use crate::provider::gmail::{ClientConfig, GmailProvider, auth};
use crate::provider::secrets::{KeyringStore, SecretStore};
use crate::provider::{MailProvider, ProviderError, RemoteId};
use crate::sync::cache::Cache;
use crate::sync::{self, Move, Pull, ReadChange};

pub(super) type SharedProvider = Arc<Mutex<Box<dyn MailProvider>>>;

const PALETTE: [&str; 6] = ["#61afef", "#c678dd", "#98c379", "#e5c07b", "#e06c75", "#56b6c2"];
/// Loop period; a pull happens every `PULL_EVERY` iterations (60 s).
const TICK: Duration = Duration::from_secs(2);
const PULL_EVERY: u32 = 30;
/// Loop iterations to pause after a rate-limit response (~1 minute).
const RATE_LIMIT_BACKOFF: u32 = 30;
/// Cached messages re-fetched per tick to learn their server read state.
const READ_BACKFILL: usize = 25;

/// Work for one account, run on a background thread.
struct AccountJob {
    account: AccountId,
    provider: SharedProvider,
    moves: Vec<Move>,
    reads: Vec<ReadChange>,
    pull: Option<PullJob>,
}

struct PullJob {
    /// `None` = initial import (or its continuation).
    cursor: Option<String>,
    /// Remote ids already cached, skipped by the import.
    known: HashSet<RemoteId>,
    /// Cached messages to fetch again (read state unknown).
    refresh: Vec<RemoteId>,
}

struct AccountOutcome {
    account: AccountId,
    results: Vec<(Move, Result<Option<crate::provider::RemoteFolder>, ProviderError>)>,
    reads: Vec<(ReadChange, Result<(), ProviderError>)>,
    pull: Option<Result<Pull, ProviderError>>,
}

fn is_throttled<T>(result: &Result<T, ProviderError>) -> bool {
    matches!(result, Err(ProviderError::RateLimited))
}

fn run_job(job: AccountJob) -> AccountOutcome {
    let mut provider = job.provider.lock().unwrap_or_else(|e| e.into_inner());
    let results = sync::run_moves(provider.as_mut(), job.moves);
    // Throttled while pushing: don't spend more quota this round.
    let mut stop = results.iter().any(|(_, r)| is_throttled(r));
    let reads = if stop { Vec::new() } else { sync::run_reads(provider.as_mut(), job.reads) };
    stop |= reads.iter().any(|(_, r)| is_throttled(r));
    let pull = job
        .pull
        .filter(|_| !stop)
        .map(|pull| sync::background_pull(provider.as_mut(), pull.cursor, &pull.known, &pull.refresh));
    AccountOutcome { account: job.account, results, reads, pull }
}

impl MailApp {
    /// Adopt the cache and start the sync loop for every cached Gmail account.
    pub fn attach_sync(&mut self, cache: Rc<Cache>, demo: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.cache = Some(cache);
        self.demo = demo;
        let client = ClientConfig::from_env();
        let accounts: Vec<Account> =
            self.mailbox.accounts().iter().filter(|a| a.provider == ProviderKind::Gmail).cloned().collect();
        for account in accounts {
            let token = KeyringStore.get(&account.email);
            match (&client, token) {
                (Some(client), Some(token)) => {
                    self.register_provider(&account.id, Box::new(GmailProvider::new(client.clone(), token)));
                }
                _ => {
                    // Called while the window is being built: the kit's root (which hosts
                    // notifications) doesn't exist yet, so toast after this frame's setup.
                    let text = format!("Gmail ({}): sign in again via “Add Gmail account”", account.email);
                    cx.defer_in(window, move |this, window, cx| this.show_toast(text, window, cx));
                }
            }
        }
        let executor = cx.background_executor().clone();
        cx.spawn_in(window, async move |this, cx| {
            let mut iteration = 0u32;
            loop {
                executor.timer(TICK).await;
                let due = iteration.is_multiple_of(PULL_EVERY);
                iteration += 1;
                let Ok(jobs) = this.update(cx, |this, _| this.sync_prepare(due)) else {
                    break;
                };
                if jobs.is_empty() {
                    continue;
                }
                let outcomes = executor
                    .spawn(async move { jobs.into_iter().map(run_job).collect::<Vec<_>>() })
                    .await;
                if this.update_in(cx, |this, window, cx| this.sync_finish(outcomes, window, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn register_provider(&mut self, account: &str, provider: Box<dyn MailProvider>) {
        self.providers.insert(account.to_owned(), Arc::new(Mutex::new(provider)));
    }

    /// Main-thread half of one loop iteration: persist local state, then compute the work.
    fn sync_prepare(&mut self, pull_due: bool) -> Vec<AccountJob> {
        let Some(cache) = self.cache.clone() else {
            return Vec::new();
        };
        sync::persist_local(&self.mailbox, &cache);
        if self.sync_backoff > 0 {
            // Rate limited: send nothing until the quota window has passed.
            self.sync_backoff -= 1;
            return Vec::new();
        }
        let force = std::mem::take(&mut self.pull_now);
        // A forced pull (Fetch mail, or the first import after sign-in) reports what it brought in.
        if force && !self.providers.is_empty() && self.fetch_baseline.is_none() {
            self.fetch_baseline = Some(self.mailbox.messages().len());
        }
        let mut jobs = Vec::new();
        for (account, provider) in &self.providers {
            let moves = sync::pending_moves(&self.mailbox, &cache, account);
            let reads = sync::pending_reads(&self.mailbox, &cache, account);
            let cursor = cache.cursor(account).ok().flatten();
            // Cached rows whose server read state is unknown get re-fetched a few at a time.
            let refresh = cache.unknown_read(account, READ_BACKFILL).unwrap_or_default();
            // An unfinished initial import (or read backfill) continues every tick, one chunk at a time.
            let pull = (pull_due || force || cursor.is_none() || !refresh.is_empty()).then(|| PullJob {
                known: if cursor.is_none() { cache.remote_ids(account).unwrap_or_default() } else { HashSet::new() },
                cursor,
                refresh,
            });
            if moves.is_empty() && reads.is_empty() && pull.is_none() {
                continue;
            }
            jobs.push(AccountJob { account: account.clone(), provider: provider.clone(), moves, reads, pull });
        }
        jobs
    }

    /// Main-thread half: fold results into the mailbox and cache, toast new errors.
    fn sync_finish(&mut self, outcomes: Vec<AccountOutcome>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(cache) = self.cache.clone() else {
            return;
        };
        let mut error: Option<String> = None;
        let mut importing = false;
        let mut throttled = false;
        for outcome in outcomes {
            // The account was removed while this batch was in flight.
            if !self.providers.contains_key(&outcome.account) {
                continue;
            }
            throttled |= outcome.results.iter().any(|(_, r)| is_throttled(r))
                || outcome.reads.iter().any(|(_, r)| is_throttled(r));
            if let Some(e) = sync::apply_moves(&self.mailbox, &cache, &outcome.results) {
                error.get_or_insert(e);
            }
            if let Some(e) = sync::apply_reads(&cache, &outcome.reads) {
                error.get_or_insert(e);
            }
            match outcome.pull {
                Some(Ok(pull)) => {
                    importing |= pull.importing();
                    if let Err(e) = sync::apply_fetched(&mut self.mailbox, &cache, &outcome.account, &pull) {
                        error.get_or_insert(e);
                    }
                }
                Some(Err(e)) => {
                    throttled |= matches!(e, ProviderError::RateLimited);
                    error.get_or_insert(e.to_string());
                }
                None => {}
            }
        }
        if throttled {
            self.sync_backoff = RATE_LIMIT_BACKOFF;
        }
        // Report a requested fetch once it has finished (an initial import spans many pulls).
        if let Some(baseline) = self.fetch_baseline
            && (error.is_some() || !importing)
        {
            self.fetch_baseline = None;
            if error.is_none() {
                let text = match self.mailbox.messages().len().saturating_sub(baseline) {
                    0 => "No new mail".to_owned(),
                    1 => "1 new message".to_owned(),
                    n => format!("{n} new messages"),
                };
                self.show_toast(text, window, cx);
            }
        }
        match error {
            Some(e) if self.sync_error.as_deref() != Some(&e) => {
                self.show_toast(format!("Gmail sync: {e}"), window, cx);
                self.sync_error = Some(e);
            }
            Some(_) => {}
            None => self.sync_error = None,
        }
        self.classify_visible();
        cx.notify();
    }

    pub(super) fn has_remote_accounts(&self) -> bool {
        !self.providers.is_empty()
    }

    /// Pull every linked account on the next sync tick (within ~2 s), then report the result.
    pub(super) fn fetch_mail(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.providers.is_empty() {
            self.show_toast("No Gmail account linked — add one with the sidebar +".into(), window, cx);
            return;
        }
        self.pull_now = true;
        self.show_toast("Fetching mail…".into(), window, cx);
    }

    /// Drop everything shown for demo data and reset view state.
    fn reset_view_state(&mut self) {
        self.triage = Triage::new(View::default());
        self.tabs = Tabs::default();
        self.expanded.clear();
        self.reader_panes.borrow_mut().clear();
        self.finds.clear();
        self.summary = None;
        self.session = None;
        self.menu_target = None;
        self.row_cursor = 0;
        self.alt_cursor = 0;
        self.demo = false;
    }

    pub(super) fn add_gmail_account(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(client) = ClientConfig::from_env() else {
            self.show_toast(
                "Set MAIL_CLASSIFIER_GOOGLE_CLIENT_ID and MAIL_CLASSIFIER_GOOGLE_CLIENT_SECRET".into(),
                window,
                cx,
            );
            return;
        };
        let Some(cache) = self.cache.clone() else {
            self.show_toast("Mail cache unavailable".into(), window, cx);
            return;
        };
        self.show_toast("Opening browser to sign in…".into(), window, cx);
        let (url_tx, url_rx) = futures::channel::oneshot::channel::<String>();
        let sign_in_client = client.clone();
        let task = cx.background_spawn(async move {
            auth::sign_in(&sign_in_client, move |url| {
                url_tx.send(url.to_owned()).ok();
            })
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(url) = url_rx.await {
                cx.update(|_, app| app.open_url(&url)).ok();
            }
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                match result {
                    Ok((tokens, email)) => this.finish_gmail_sign_in(client, cache, tokens.refresh, email, window, cx),
                    Err(e) => this.show_toast(format!("Gmail sign-in failed: {e}"), window, cx),
                }
            })
            .ok();
        })
        .detach();
    }

    fn finish_gmail_sign_in(
        &mut self,
        client: ClientConfig,
        cache: Rc<Cache>,
        refresh: String,
        email: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(e) = KeyringStore.set(&email, &refresh) {
            self.show_toast(format!("Gmail sign-in failed: keychain: {e}"), window, cx);
            return;
        }
        if self.demo {
            let mocks: Vec<AccountId> = self
                .mailbox
                .accounts()
                .iter()
                .filter(|a| a.provider == ProviderKind::Mock)
                .map(|a| a.id.clone())
                .collect();
            for id in mocks {
                self.mailbox.remove_account_data(&id);
                self.mailbox.remove_account(&id);
            }
            self.reset_view_state();
        }
        let account = Account {
            id: format!("gmail:{email}"),
            name: email.clone(),
            email,
            color: PALETTE[self.mailbox.accounts().len() % PALETTE.len()].to_owned(),
            provider: ProviderKind::Gmail,
        };
        if let Err(e) = cache.upsert_account(&account) {
            self.show_toast(format!("Gmail sign-in failed: cache: {e}"), window, cx);
            return;
        }
        let id = account.id.clone();
        let label = account.email.clone();
        self.mailbox.add_account(account);
        self.register_provider(&id, Box::new(GmailProvider::new(client, refresh)));
        self.pull_now = true;
        self.show_toast(format!("Gmail connected: {label}"), window, cx);
        self.refresh_account_rows(cx);
        self.classify_visible();
        cx.notify();
    }

    /// Push the current account list into an open settings panel.
    fn refresh_account_rows(&mut self, cx: &mut Context<Self>) {
        if let Some(panel) = self.settings.clone() {
            let rows = self.account_rows();
            panel.update(cx, |panel, cx| panel.set_accounts(rows, cx));
        }
    }

    /// Unlink an account: stop syncing it, forget its token, and drop its cached and shown mail.
    /// Mail on the server is untouched.
    pub(super) fn remove_linked_account(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(email) = self.mailbox.account(id).map(|a| a.email.clone()) else {
            return;
        };
        self.providers.remove(id);
        let mut problems = Vec::new();
        if let Some(cache) = &self.cache
            && let Err(e) = cache.delete_account(id)
        {
            problems.push(format!("cache: {e}"));
        }
        if let Err(e) = KeyringStore.delete(&email) {
            problems.push(format!("keychain: {e}"));
        }
        self.mailbox.remove_account_data(id);
        self.mailbox.remove_account(id);
        self.reset_view_state();
        let text = if problems.is_empty() {
            format!("Removed {email}")
        } else {
            format!("Removed {email} ({})", problems.join("; "))
        };
        self.show_toast(text, window, cx);
        self.refresh_account_rows(cx);
        cx.notify();
    }
}
