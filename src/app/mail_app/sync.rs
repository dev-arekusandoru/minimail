//! Remote mail sync: a wakeable background loop that pushes pending triage and
//! runs one sync round per account (server check, load-more, one backfill page),
//! downloads message bodies as they are opened, plus the sign-in flow.

use super::*;
use crate::model::{Account, ProviderKind};
use crate::provider::gmail::{ClientConfig, GmailProvider, auth};
use std::sync::{Arc, Mutex};

use futures::StreamExt as _;

use crate::provider::secrets::{KeyringStore, SecretStore};
use crate::provider::{Body, MailProvider, ProviderError, Scope};
use crate::sync::cache::Cache;
use crate::sync::{self, SharedProvider};

/// How long the loop waits before running a round that has nothing due.
const TICK: Duration = Duration::from_secs(2);
/// How often the server is checked for changes.
const CHECK_EVERY: Duration = Duration::from_secs(60);
/// How long rounds pause after the provider reported a rate limit.
const THROTTLE_BACKOFF: Duration = Duration::from_secs(60);
/// Messages after the opened one whose body is prefetched.
const BODY_PREFETCH: usize = 2;

/// One account's round, run on a background thread.
struct AccountJob {
    account: AccountId,
    provider: SharedProvider,
    round: sync::Round,
}

fn run_job(job: AccountJob) -> (AccountId, sync::RoundResult) {
    let account = job.account;
    (account.clone(), sync::run_round(&job.provider, job.round))
}

/// What a finished browser sign-in hands back: the token, the address it belongs to, and the
/// account it is replacing, if the user was signing an existing one back in.
struct SignIn {
    client: ClientConfig,
    cache: Rc<Cache>,
    refresh: String,
    email: String,
    /// The account to re-attach the token to; `None` adds a new account.
    existing: Option<AccountId>,
}

impl MailApp {
    /// Adopt the cache and start the sync loop for every cached Gmail account.
    pub fn attach_sync(&mut self, cache: Rc<Cache>, window: &mut Window, cx: &mut Context<Self>) {
        self.cache = Some(cache);
        let client = ClientConfig::from_env();
        let accounts: Vec<Account> = self
            .mailbox
            .accounts()
            .iter()
            .filter(|a| a.provider == ProviderKind::Gmail)
            .cloned()
            .collect();
        for account in accounts {
            let token = KeyringStore.get(&account.email);
            match (&client, token) {
                (Some(client), Some(token)) => {
                    self.register_provider(&account.id, Box::new(GmailProvider::new(client.clone(), token)));
                }
                _ => {
                    // Called while the window is being built: the kit's root (which hosts
                    // notifications) doesn't exist yet, so toast after this frame's setup.
                    let shown = crate::account_style::nickname_or(&account, &account.email);
                    let text = format!("Gmail ({shown}): sign in again via “Add Gmail account”");
                    cx.defer_in(window, move |this, window, cx| this.show_toast(text, window, cx));
                }
            }
        }
        let (wake, mut rx) = futures::channel::mpsc::unbounded::<()>();
        self.sync_wake = Some(wake);
        let executor = cx.background_executor().clone();
        cx.spawn_in(window, async move |this, cx| {
            // The first round runs at once, with a check: cached mail shows
            // instantly and the server is asked what changed since.
            let mut wait = Duration::ZERO;
            loop {
                // Sleep until the next round is due, or until something wakes us.
                let _ = futures::future::select(Box::pin(executor.timer(wait)), Box::pin(rx.next())).await;
                // The loop stops only when the view is gone. An idle app (`Ok(None)`)
                // means there is nothing due yet, not that the loop should end: it
                // must stay alive to wake on the next `wake_sync` (Fetch mail, sign-in)
                // or scheduled check.
                let step = match wake_step(this.update(cx, |this, cx| {
                    let jobs = this.sync_prepare();
                    if jobs.is_some() {
                        this.refresh_account_rows(cx);
                    }
                    jobs
                })) {
                    Wake::Run(jobs) => jobs,
                    Wake::Idle => {
                        wait = TICK;
                        continue;
                    }
                    Wake::Stop => break,
                };
                let results = executor
                    .spawn(async move { step.into_iter().map(run_job).collect::<Vec<_>>() })
                    .await;
                let Ok(step) = this.update_in(cx, |this, window, cx| this.sync_finish(results, window, cx)) else {
                    break;
                };
                wait = step.wait;
            }
        })
        .detach();
    }

    fn register_provider(&mut self, account: &str, provider: Box<dyn MailProvider>) {
        self.providers.insert(account.to_owned(), Arc::new(Mutex::new(provider)));
    }

    /// Main-thread half of one loop iteration: persist local state, then plan
    /// every account's round. `None` means there is nothing to do.
    fn sync_prepare(&mut self) -> Option<Vec<AccountJob>> {
        let cache = self.cache.clone()?;
        sync::persist_local(&self.mailbox, &cache);
        let now = self.now();
        if let Some(until) = self.throttled_until
            && now < until
        {
            self.fetch_baseline = None;
            return None;
        }
        let check = self.force_check || now >= self.check_at;
        if check {
            self.force_check = false;
            self.check_at = now + CHECK_EVERY.as_secs() as i64;
        }
        let mut jobs = Vec::new();
        for (account, provider) in &self.providers {
            let older = self.older_queue.get(account).cloned().unwrap_or_default();
            // A retried account asks the server now, even when nothing else is due for it.
            let check = check || self.retry_accounts.contains(account);
            let round = sync::plan_round(&self.mailbox, &cache, account, now, check, &older);
            if round.is_empty() && older.is_empty() {
                continue;
            }
            self.retry_accounts.remove(account);
            self.older_in_flight.insert(account.clone());
            jobs.push(AccountJob { account: account.clone(), provider: provider.clone(), round });
        }
        (!jobs.is_empty()).then_some(jobs)
    }

    /// Main-thread half: fold the rounds into the mailbox and cache, toast new
    /// errors, and say when to run the next round.
    fn sync_finish(
        &mut self,
        results: Vec<(AccountId, sync::RoundResult)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Step {
        let Some(cache) = self.cache.clone() else {
            return Step { wait: TICK };
        };
        let mut more = false;
        let mut throttled = false;
        let mut error: Option<String> = None;
        for (account, result) in results {
            // The account was removed while this round was in flight.
            if !self.providers.contains_key(&account) {
                continue;
            }
            let summary = sync::apply_round(&mut self.mailbox, &cache, result);
            more |= summary.more;
            throttled |= summary.throttled;
            match &summary.error {
                Some(e) => {
                    self.account_errors.insert(account.clone(), e.clone());
                }
                None if summary.checked => {
                    self.account_errors.remove(&account);
                    // A failed write only loses the "Synced …" time; syncing itself is unaffected.
                    cache.set_synced_at(&account, self.now()).ok();
                }
                None => {}
            }
            if let Some(e) = summary.error {
                error.get_or_insert(e);
            }
            self.older_in_flight.remove(&account);
            // A load-more request is answered by its page; the next one is made
            // when the list is scrolled near its end again.
            self.older_queue.remove(&account);
        }
        self.throttled_until = throttled.then(|| self.now() + THROTTLE_BACKOFF.as_secs() as i64);
        // Report a requested fetch once the rounds it asked for have landed.
        if let Some(baseline) = self.fetch_baseline
            && (error.is_some() || !more)
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
        let bodies = self.body_targets();
        self.fetch_bodies(bodies, cx);
        self.refresh_account_rows(cx);
        cx.notify();
        let wait = if more {
            Duration::ZERO
        } else if throttled {
            THROTTLE_BACKOFF
        } else {
            TICK
        };
        Step { wait }
    }

    pub(super) fn has_remote_accounts(&self) -> bool {
        !self.providers.is_empty()
    }

    /// Ask the server for changes on the next round (within ~2 s), then report the result.
    pub(super) fn fetch_mail(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.providers.is_empty() {
            self.show_toast("No Gmail account linked — add one with the sidebar +".into(), window, cx);
            return;
        }
        self.force_check = true;
        self.fetch_baseline = Some(self.mailbox.messages().len());
        self.fetch_toast = true;
        window.push_notification(
            Notification::info("Fetching mail…")
                .id::<FetchToastId>()
                .placement(Anchor::BottomCenter)
                .autohide(false),
            cx,
        );
        self.wake_sync();
    }

    /// Whether a requested fetch (or a fresh sign-in's first sync) is still running.
    pub(super) fn is_fetching(&self) -> bool {
        self.fetch_baseline.is_some()
    }

    /// Dismiss the persistent fetch toast once the fetch has landed or failed; runs every frame
    /// so no exit path (error, throttle, removed account) can leave it stuck.
    pub(super) fn settle_fetch_toast(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.fetch_toast && !self.is_fetching() {
            self.fetch_toast = false;
            window.remove_notification::<FetchToastId>(cx);
        }
    }

    /// The list is near its end: queue a page of older mail and wake the loop.
    pub(super) fn maybe_load_older(&mut self, near_end: bool) {
        if !near_end || self.loading_older() {
            return;
        }
        let Some(cache) = self.cache.clone() else {
            return;
        };
        let Some(location) = self.location() else { return };
        let scopes: Vec<(AccountId, Scope)> = sync::scopes_for(&self.mailbox, &cache, &location)
            .into_iter()
            .filter(|(account, scope)| sync::has_older(&cache, account, scope))
            .collect();
        for (account, scope) in scopes {
            if self.older_in_flight.contains(&account) {
                continue;
            }
            let queue = self.older_queue.entry(account).or_default();
            if !queue.contains(&scope) {
                queue.push(scope);
            }
        }
        self.wake_sync();
    }

    /// Whether a load-more page is queued or running: the footer says so.
    pub(super) fn loading_older(&self) -> bool {
        !self.older_queue.is_empty() || !self.older_in_flight.is_empty()
    }

    fn wake_sync(&mut self) {
        if let Some(wake) = &self.sync_wake {
            wake.unbounded_send(()).ok();
        }
    }

    /// The opened message and the next few rows of the list.
    pub(super) fn body_targets(&self) -> Vec<MessageId> {
        let opened = self.opened();
        let visible = self.visible_ids();
        let start = opened.and_then(|id| visible.iter().position(|v| *v == id)).unwrap_or(0);
        let mut ids: Vec<MessageId> = visible[start..].iter().take(BODY_PREFETCH + 1).copied().collect();
        if let Some(id) = opened
            && !ids.contains(&id)
        {
            ids.insert(0, id);
        }
        ids
    }

    /// Download the bodies of `ids` in the background, storing each as it lands.
    pub(super) fn fetch_bodies(&mut self, ids: Vec<MessageId>, cx: &mut Context<Self>) {
        let Some(cache) = self.cache.clone() else {
            return;
        };
        for id in ids {
            if self.bodies_in_flight.contains(&id) {
                continue;
            }
            let Some(req) = sync::body_request(&self.mailbox, &cache, id) else {
                continue;
            };
            let Some(provider) = self.providers.get(&req.account).cloned() else {
                continue;
            };
            self.bodies_in_flight.insert(id);
            // The blocking get runs off-thread; the merge happens back on the UI thread.
            let task = cx.background_spawn(async move { sync::run_body(&provider, req) });
            cx.spawn(async move |this, cx| {
                let (req, result) = task.await;
                this.update(cx, |this, cx| this.finish_body(req, result, cx)).ok();
            })
            .detach();
        }
    }

    fn finish_body(
        &mut self,
        req: sync::BodyRequest,
        result: Result<Body, ProviderError>,
        cx: &mut Context<Self>,
    ) {
        let Some(cache) = self.cache.clone() else {
            return;
        };
        let cache = cache.as_ref();
        self.bodies_in_flight.remove(&req.id);
        match result {
            Ok(body) => {
                if let Err(e) = sync::apply_body(&mut self.mailbox, cache, &req, body) {
                    self.toast_sync_error(e);
                }
            }
            // A rate limit is expected and self-healing: the body is requested again.
            Err(ProviderError::RateLimited) => {}
            Err(e) => self.toast_sync_error(e.to_string()),
        }
        cx.notify();
    }

    fn toast_sync_error(&mut self, e: String) {
        if self.sync_error.as_deref() == Some(&e) {
            return;
        }
        self.sync_error = Some(e.clone());
        self.pending_toast = Some(format!("Gmail sync: {e}"));
    }

    /// Reset the list, tabs and reader to a fresh state after accounts were removed.
    fn reset_view_state(&mut self) {
        self.folder = Location::AllInboxes;
        self.triage = Triage::new(self.mailbox.location_query(&Location::AllInboxes));
        self.tabs = Tabs::default();
        self.expanded.clear();
        self.reader_panes.borrow_mut().clear();
        self.finds.clear();
        self.summary = None;
        self.session = None;
        self.menu_target = None;
        self.row_cursor = 0;
    }


    pub(super) fn add_gmail_account(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.start_gmail_sign_in(None, window, cx);
    }

    /// Sign `id` in again after its token stopped working: the account keeps its id, nickname,
    /// icon, color and cached mail; only its provider is replaced.
    pub(super) fn sign_in_again(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.mailbox.account(id).is_none() {
            return;
        }
        self.start_gmail_sign_in(Some(id.to_owned()), window, cx);
    }

    /// Run one more server round for `id` right away, dropping the error that made the card offer
    /// Retry.
    pub(super) fn retry_sync(&mut self, id: &str, cx: &mut Context<Self>) {
        if !self.providers.contains_key(id) {
            return;
        }
        self.account_errors.remove(id);
        self.retry_accounts.insert(id.to_owned());
        self.wake_sync();
        self.refresh_account_rows(cx);
        cx.notify();
    }

    /// Open the browser sign-in, for a new account (`existing` is `None`) or for one whose
    /// sign-in expired.
    fn start_gmail_sign_in(
        &mut self,
        existing: Option<AccountId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
                    Ok((tokens, email)) => this.finish_gmail_sign_in(
                        SignIn {
                            client,
                            cache,
                            refresh: tokens.refresh,
                            email,
                            existing,
                        },
                        window,
                        cx,
                    ),
                    Err(e) => this.show_toast(format!("Gmail sign-in failed: {e}"), window, cx),
                }
            })
            .ok();
        })
        .detach();
    }

    /// Store the new token. A sign-in for an already linked account re-attaches it to that
    /// account instead of creating a second one.
    fn finish_gmail_sign_in(&mut self, sign: SignIn, window: &mut Window, cx: &mut Context<Self>) {
        let SignIn { client, cache, refresh, email, existing } = sign;
        if let Some(id) = existing {
            return self.finish_gmail_reauth(&id, client, refresh, email, window, cx);
        }
        if let Err(e) = KeyringStore.set(&email, &refresh) {
            self.show_toast(format!("Gmail sign-in failed: keychain: {e}"), window, cx);
            return;
        }
        let style = crate::account_style::style_for_index(self.mailbox.accounts().len());
        let account = Account {
            id: format!("gmail:{email}"),
            name: email.clone(),
            email,
            color: style.color.to_owned(),
            icon: Some(style.icon.to_owned()),
            nickname: None,
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
        self.force_check = true;
        self.fetch_baseline = Some(self.mailbox.messages().len());
        self.wake_sync();
        self.show_toast(format!("Gmail connected: {label}"), window, cx);
        self.refresh_account_rows(cx);
        self.classify_visible();
        cx.notify();
    }

    /// A fresh token for an account that is already linked. The account keeps its id, nickname,
    /// icon, color and downloaded mail; only its provider is replaced. A sign-in that came back
    /// for a different address is refused, so no token is stored under the wrong account.
    fn finish_gmail_reauth(
        &mut self,
        id: &str,
        client: ClientConfig,
        refresh: String,
        email: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(account) = self.mailbox.account(id) else { return };
        let (address, label) = (account.email.clone(), crate::account_style::nickname_or(account, &account.email).to_owned());
        if !address.eq_ignore_ascii_case(&email) {
            self.show_toast(
                format!("That sign-in is for {email}, not {label}. Add it as a separate account."),
                window,
                cx,
            );
            return;
        }
        if let Err(e) = KeyringStore.set(&address, &refresh) {
            self.show_toast(format!("Gmail sign-in failed: keychain: {e}"), window, cx);
            return;
        }
        self.register_provider(id, Box::new(GmailProvider::new(client, refresh)));
        self.account_errors.remove(id);
        self.force_check = true;
        self.fetch_baseline = Some(self.mailbox.messages().len());
        self.wake_sync();
        self.show_toast(format!("Gmail connected: {label}"), window, cx);
        self.refresh_account_rows(cx);
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
        let Some((email, label)) = self.mailbox.account(id).map(|a| {
            (a.email.clone(), crate::account_style::nickname_or(a, &a.email).to_owned())
        }) else {
            return;
        };
        self.providers.remove(id);
        if self.providers.is_empty() {
            self.fetch_baseline = None;
        }
        self.older_queue.remove(id);
        self.older_in_flight.remove(id);
        self.account_errors.remove(id);
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
            format!("Removed {label}")
        } else {
            format!("Removed {label} ({})", problems.join("; "))
        };
        self.show_toast(text, window, cx);
        self.refresh_account_rows(cx);
        cx.notify();
    }
}

/// What the sync loop should do after a round.
struct Step {
    wait: Duration,
}

/// The sync loop's next move after asking the app for work.
enum Wake<T> {
    Run(T),
    /// Nothing is due; stay alive and check again later.
    Idle,
    /// The app entity is gone; end the loop.
    Stop,
}

/// Decide the loop's next move. An idle app (`Ok(None)`) must keep the loop
/// alive: ending it here would stop every future sync until the process
/// restarts, which is why a signed-in account showed no mail until reload.
fn wake_step<T, E>(prepared: Result<Option<T>, E>) -> Wake<T> {
    match prepared {
        Ok(Some(work)) => Wake::Run(work),
        Ok(None) => Wake::Idle,
        Err(_) => Wake::Stop,
    }
}

#[cfg(test)]
mod tests {
    use super::{Wake, wake_step};

    #[test]
    fn an_idle_app_keeps_the_sync_loop_alive() {
        assert!(matches!(wake_step::<u8, ()>(Ok(None)), Wake::Idle));
    }

    #[test]
    fn planned_work_runs_and_a_gone_view_stops() {
        assert!(matches!(wake_step::<u8, ()>(Ok(Some(7))), Wake::Run(7)));
        assert!(matches!(wake_step::<u8, ()>(Err(())), Wake::Stop));
    }
}
