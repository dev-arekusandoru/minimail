//! Remote mail sync: a background loop that pushes pending triage moves and
//! pulls changes, plus the "Add Gmail account" sign-in flow.

use std::sync::{Arc, Mutex};

use super::*;
use crate::model::{Account, ProviderKind};
use crate::provider::gmail::{ClientConfig, GmailProvider, auth};
use crate::provider::secrets::{KeyringStore, SecretStore};
use crate::provider::{MailProvider, ProviderError};
use crate::sync::cache::Cache;
use crate::sync::{self, Move, Pull};

pub(super) type SharedProvider = Arc<Mutex<Box<dyn MailProvider>>>;

const PALETTE: [&str; 6] = ["#61afef", "#c678dd", "#98c379", "#e5c07b", "#e06c75", "#56b6c2"];
/// Loop period; a pull happens every `PULL_EVERY` iterations (60 s).
const TICK: Duration = Duration::from_secs(2);
const PULL_EVERY: u32 = 30;

/// Work for one account, run on a background thread.
struct AccountJob {
    account: AccountId,
    provider: SharedProvider,
    moves: Vec<Move>,
    /// `Some(cursor)` when a pull is due (`None` inside = initial import).
    pull: Option<Option<String>>,
}

struct AccountOutcome {
    account: AccountId,
    results: Vec<(Move, Result<Option<crate::provider::RemoteFolder>, ProviderError>)>,
    pull: Option<Result<Pull, ProviderError>>,
}

fn run_job(job: AccountJob) -> AccountOutcome {
    let mut provider = job.provider.lock().unwrap_or_else(|e| e.into_inner());
    let results = sync::run_moves(provider.as_mut(), job.moves);
    let pull = job.pull.map(|cursor| sync::background_pull(provider.as_mut(), cursor));
    AccountOutcome { account: job.account, results, pull }
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
                _ => self.show_toast(
                    format!("Gmail ({}): sign in again via “Add Gmail account”", account.email),
                    window,
                    cx,
                ),
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
        let force = std::mem::take(&mut self.pull_now);
        let mut jobs = Vec::new();
        for (account, provider) in &self.providers {
            let moves = sync::pending_moves(&self.mailbox, &cache, account);
            let pull = (pull_due || force)
                .then(|| cache.cursor(account).ok().flatten());
            if moves.is_empty() && pull.is_none() {
                continue;
            }
            jobs.push(AccountJob { account: account.clone(), provider: provider.clone(), moves, pull });
        }
        jobs
    }

    /// Main-thread half: fold results into the mailbox and cache, toast new errors.
    fn sync_finish(&mut self, outcomes: Vec<AccountOutcome>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(cache) = self.cache.clone() else {
            return;
        };
        let mut error: Option<String> = None;
        for outcome in outcomes {
            if let Some(e) = sync::apply_moves(&self.mailbox, &cache, &outcome.results) {
                error.get_or_insert(e);
            }
            match outcome.pull {
                Some(Ok(pull)) => {
                    if let Err(e) = sync::apply_fetched(&mut self.mailbox, &cache, &outcome.account, &pull) {
                        error.get_or_insert(e);
                    }
                }
                Some(Err(e)) => {
                    error.get_or_insert(e.to_string());
                }
                None => {}
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

    /// Drop everything shown for demo data and reset view state.
    fn reset_view_state(&mut self) {
        self.triage = Triage::new(View::default());
        self.tabs = Tabs::default();
        self.read.clear();
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
        self.classify_visible();
        cx.notify();
    }
}
