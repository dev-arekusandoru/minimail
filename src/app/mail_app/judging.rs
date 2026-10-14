//! The classifier's background worker: a serial queue draining one judge request at a time.
//!
//! `classify_visible` routes cached answers immediately and, when automatic classification is
//! on, queues the rest; this module runs each queued request off-thread, stores the answers, and
//! applies the routing back on the UI thread. One request is in flight at a time. A message is
//! only sent once its full body has landed (see [`MailApp::body_ready`] / [`MailApp::classify_message`]).

use std::sync::Arc;

use serde_json::{Value, json};

use crate::app_settings::{
    AUTO_CLASSIFY, JEV_BASE_URL, JEV_MODEL, MODE_EXPECTS_REPLY, MODE_FOLDER, MODE_KIND,
    MODE_NEEDS_REPLY, MODE_SPAM, MODE_URGENCY, SPAM_COLD_OUTREACH,
};
use crate::judge::{
    Answer, AnswerValue, ClassifyState, Judge, JudgeError, JudgeStatus, Question, QuestionKey,
    QuestionSpec, StoredAnswer, incoming_questions, legacy_judgement_key, message_state,
    questions_to_ask, reply_question, route,
};
use crate::prefs::Scope;

use super::*;

/// Keychain entry holding the Jev API key. Accounts are named by their email, so it cannot collide.
const JEV_SECRET: &str = "jev";

/// One unit of classifier work.
#[derive(Clone, Debug)]
pub(super) enum JudgeJob {
    /// Classify the Inbox message `id`. `force` re-asks every enabled question (a manual
    /// "Classify this message") instead of only the missing or stale-version ones.
    Message { id: MessageId, force: bool },
    /// Judge the reply sent in answer to `in_reply_to`; `seq` is its outbox sequence and
    /// `sent_at` is when it was queued.
    Reply {
        in_reply_to: MessageId,
        seq: u64,
        sent_at: Timestamp,
        body: String,
    },
}

impl MailApp {
    /// Replace the classifier. `None` means no key, so classification is off. Clears the queue
    /// and the error and re-runs classification over the visible Inbox.
    pub fn set_judge(&mut self, judge: Option<Arc<dyn Judge + Send + Sync>>, cx: &mut Context<Self>) {
        self.judge = judge;
        // Any request still in flight belongs to the old judge; its result is discarded when it
        // lands, but the physical slot stays occupied until then, so requests stay serial. The
        // message jobs are rebuilt by `classify_visible`; reply jobs cannot be, so they stay.
        self.judge_gen += 1;
        self.judge_queue.retain(|j| matches!(j, JudgeJob::Reply { .. }));
        self.judge_in_flight = None;
        self.classify_visible(cx);
        cx.notify();
    }

    /// Build the judge from the `"jev"` secret plus the Jev preferences, then install it.
    pub fn configure_jev(&mut self, cx: &mut Context<Self>) {
        let judge = self.secrets.get(JEV_SECRET).map(|key| {
            let (base, model) = self.jev_settings();
            (self.judge_factory)(&base, &model, &key)
        });
        self.set_judge(judge, cx);
    }

    /// Replace the saved Jev key. An empty `key` leaves the saved one alone; a blank field on
    /// Save means "I only changed the endpoint".
    pub(super) fn save_jev_key(&mut self, key: &str) -> Result<(), String> {
        if !key.is_empty() {
            self.secrets.set(JEV_SECRET, key)?;
        }
        Ok(())
    }

    /// Forget the saved Jev key and turn classification off. The keychain outcome goes back to
    /// the panel, which only stops claiming a saved key once the deletion is confirmed.
    pub(super) fn remove_jev_key(
        &mut self,
        panel: WeakEntity<SettingsPanel>,
        cx: &mut Context<Self>,
    ) {
        // Stop sending immediately, even if the keychain then refuses to delete the key.
        self.set_judge(None, cx);
        let result = self.secrets.delete(JEV_SECRET);
        // Report on the next tick: the panel owns the click that got us here.
        cx.spawn(async move |_this, cx| {
            panel.update(cx, |panel, cx| panel.set_jev_removed(result, cx)).ok();
        })
        .detach();
    }

    /// Whether a Jev key is saved (never the key itself).
    pub(super) fn has_jev_key(&self) -> bool {
        self.secrets.get(JEV_SECRET).is_some()
    }

    /// Send one synthetic request with the form's values and report the model or the error back
    /// to the settings panel. Nothing is saved.
    pub(super) fn test_jev(
        &mut self,
        key: &str,
        base: &str,
        model: &str,
        panel: WeakEntity<SettingsPanel>,
        cx: &mut Context<Self>,
    ) {
        let key = if key.is_empty() {
            self.secrets.get(JEV_SECRET).unwrap_or_default()
        } else {
            key.to_owned()
        };
        let judge = (self.judge_factory)(base, model, &key);
        let model_name = judge.model().to_owned();
        let task = cx.background_spawn(async move {
            let state = json!({
                "sender": "",
                "subject": "Jev connection test",
                "body": "A one-line request to check the key and endpoint."
            });
            let questions = vec![(
                QuestionKey::NeedsReply,
                Question::Bool {
                    criteria: "Is this a test message?".into(),
                },
            )];
            judge.judge(&state, &questions).map(|_| model_name)
        });
        cx.spawn(async move |_this, cx| {
            let result = match task.await {
                Ok(model) => Ok(model),
                Err(e) => Err(e.to_string()),
            };
            panel.update(cx, |panel, cx| panel.set_jev_test(result, cx)).ok();
        })
        .detach();
    }

    /// The classifier's current state (status bar, settings).
    pub fn judge_status(&self) -> &JudgeStatus {
        &self.judge_status
    }

    /// Whether automatic classification is on (`AUTO_CLASSIFY`).
    pub(super) fn auto_classify(&self) -> bool {
        self.auto_classify
    }

    /// The configured Jev endpoint and model, falling back to the shipped defaults.
    pub(super) fn jev_settings(&self) -> (String, String) {
        let (default_base, default_model) = (
            crate::judge::jev::DEFAULT_BASE_URL.to_owned(),
            crate::judge::jev::DEFAULT_MODEL.to_owned(),
        );
        let Some(store) = &self.preferences else {
            return (default_base, default_model);
        };
        let scope = Scope::Global;
        let base = JEV_BASE_URL.get(store.as_ref(), &scope);
        let model = JEV_MODEL.get(store.as_ref(), &scope);
        let base = if base.is_empty() { default_base } else { base };
        let model = if model.is_empty() { default_model } else { model };
        (base, model)
    }

    /// The stored answers for `id`, from the cache preloaded at startup. `None` on a miss.
    pub(super) fn cached_answers(&self, id: MessageId) -> Option<&[StoredAnswer]> {
        self.judgements.get(&id).map(Vec::as_slice)
    }

    /// Load every stored judgement into memory (once, at startup), so [`Self::cached_answers`]
    /// is a cheap map lookup per rendered row.
    pub(super) fn preload_judgements(&mut self) {
        let Some(store) = self.preferences.clone() else {
            return;
        };
        if let Ok(rows) = store.judgements_all() {
            self.judgements = rows.into_iter().collect();
        }
    }

    /// One-time carry-over from the pre-`MIGRATION_4` hash-keyed judgements: for every message
    /// with no id-keyed row, look its legacy hash up under the configured model and copy the
    /// answer under the message id. Runs at startup, where the mailbox and the store meet; a
    /// message whose model changed before the upgrade has no matching row and is judged again.
    pub(super) fn carry_over_judgements(&mut self) {
        let Some(store) = self.preferences.clone() else {
            return;
        };
        if !matches!(store.legacy_judgements_present(), Ok(true)) {
            return;
        }
        if self.mailbox.messages().is_empty() {
            // No messages loaded (e.g. an unreadable cache): keep the legacy table, so the copy
            // can still map rows on a later launch instead of losing them.
            return;
        }
        let (_, model) = self.jev_settings();
        let entries: Vec<(MessageId, String)> = self
            .mailbox
            .messages()
            .iter()
            .map(|m| (m.id, legacy_judgement_key(&model, m)))
            .collect();
        // A failure leaves the legacy table in place, to retry on the next launch.
        let _ = store.carry_over_judgements(&model, &entries);
    }

    /// The enabled incoming questions for an account, each with its version.
    fn classify_specs(&self, account: &str) -> Vec<QuestionSpec> {
        incoming_questions(
            &self.policy,
            self.spam_cold_outreach,
            &self.mailbox.folder_options(account),
        )
    }

    /// The enabled questions for `id` whose stored answer is missing or stale.
    pub(super) fn missing_questions(&self, id: MessageId) -> Vec<(QuestionKey, Question)> {
        let Some(m) = self.mailbox.get(id) else {
            return Vec::new();
        };
        if m.outgoing || m.state != TriageState::Inbox {
            return Vec::new();
        }
        let specs = self.classify_specs(&m.account);
        let stored = self.judgements.get(&id).map(Vec::as_slice).unwrap_or(&[]);
        questions_to_ask(&specs, stored)
    }

    /// Whether `id` has enabled questions to ask and its full body is in hand.
    pub(super) fn needs_questions(&self, id: MessageId) -> bool {
        let Some(m) = self.mailbox.get(id) else {
            return false;
        };
        !m.partial && !self.missing_questions(id).is_empty()
    }

    /// The list row / reader status icon for `id`. `None` means nothing to show: outgoing or
    /// non-Inbox mail, or every enabled question answered at the current version.
    pub(super) fn classification_state(&self, id: MessageId) -> Option<ClassifyState> {
        let m = self.mailbox.get(id)?;
        if m.outgoing || m.state != TriageState::Inbox {
            return None;
        }
        if self.classify_failed.contains(&id) {
            return Some(ClassifyState::Failed);
        }
        if self.pending_classify.contains_key(&id)
            || self.judge_in_flight == Some(id)
            || self
                .judge_queue
                .iter()
                .any(|j| matches!(j, JudgeJob::Message { id: queued, .. } if *queued == id))
        {
            return Some(ClassifyState::InProgress);
        }
        (!self.missing_questions(id).is_empty()).then_some(ClassifyState::NotClassified)
    }

    /// Re-ask every enabled question for `id` now (manual "Classify this message"). Works with
    /// automatic classification off, and jumps the queue. A partial message is downloaded first
    /// and sent when its body lands.
    pub(super) fn classify_message(&mut self, id: MessageId, cx: &mut Context<Self>) {
        if self.judge.is_none() {
            return;
        }
        let Some(m) = self.mailbox.get(id) else {
            return;
        };
        if m.outgoing {
            return;
        }
        let (partial, account) = (m.partial, m.account.clone());
        if self.classify_specs(&account).is_empty() {
            return;
        }
        if partial {
            self.pending_classify.insert(id, true);
            self.prioritize_body(id, cx);
            return;
        }
        self.classify_failed.remove(&id);
        self.enqueue_judge_front(JudgeJob::Message { id, force: true });
        self.drain_judge_queue(cx);
        cx.notify();
    }

    /// [`Self::classify_message`] for each of `ids`, keeping their order at the queue front.
    pub(super) fn classify_messages(&mut self, ids: &[MessageId], cx: &mut Context<Self>) {
        for id in ids.iter().rev() {
            self.classify_message(*id, cx);
        }
    }

    /// A full body landed for `id` (called by the body downloader). Sends the message to Jev
    /// when it was requested while partial, or when automatic classification is on and questions
    /// are due.
    pub(super) fn body_ready(&mut self, id: MessageId, cx: &mut Context<Self>) {
        if self.judge.is_none() {
            return;
        }
        let Some(m) = self.mailbox.get(id) else {
            return;
        };
        if m.partial || m.outgoing || m.state != TriageState::Inbox {
            return;
        }
        let account = m.account.clone();
        let pending = self.pending_classify.remove(&id);
        if pending.is_none() && !self.auto_classify {
            return;
        }
        if self.classify_specs(&account).is_empty() {
            return;
        }
        let force = pending.unwrap_or(false);
        if !force && self.missing_questions(id).is_empty() {
            return;
        }
        let job = JudgeJob::Message { id, force };
        if pending.is_some() {
            self.enqueue_judge_front(job);
        } else {
            self.enqueue_judge(job);
        }
        self.drain_judge_queue(cx);
    }

    /// Re-read the classifier settings from preferences into the app: automatic classification,
    /// the cold-outreach checkbox, and every question's mode. Drops pending suggestions for
    /// questions that are now Off, then re-routes cached answers and enqueues what is due.
    pub(super) fn apply_classifier_settings(&mut self, cx: &mut Context<Self>) {
        // With no store, keep the in-memory policy/auto state the settings UI just wrote.
        if let Some(store) = self.preferences.clone() {
            let scope = Scope::Global;
            self.auto_classify = AUTO_CLASSIFY.get(store.as_ref(), &scope);
            self.spam_cold_outreach = SPAM_COLD_OUTREACH.get(store.as_ref(), &scope);
            self.policy.set_mode(QuestionKey::Spam, MODE_SPAM.get(store.as_ref(), &scope));
            self.policy
                .set_mode(QuestionKey::NeedsReply, MODE_NEEDS_REPLY.get(store.as_ref(), &scope));
            self.policy.set_mode(QuestionKey::Urgency, MODE_URGENCY.get(store.as_ref(), &scope));
            self.policy.set_mode(QuestionKey::Kind, MODE_KIND.get(store.as_ref(), &scope));
            self.policy
                .set_mode(QuestionKey::ExpectsReply, MODE_EXPECTS_REPLY.get(store.as_ref(), &scope));
            self.policy.set_mode(QuestionKey::Folder, MODE_FOLDER.get(store.as_ref(), &scope));
        }
        self.mailbox.drop_suggestions_off(&self.policy);
        self.classify_visible(cx);
        cx.notify();
    }

    /// Route `answers` for `id` and apply them, if it is still a visible Inbox candidate.
    pub(super) fn apply_answers(&mut self, id: MessageId, answers: &[StoredAnswer]) -> (usize, usize) {
        let Some(message) = self.mailbox.get(id).cloned() else {
            return (0, 0);
        };
        if message.state != TriageState::Inbox || !self.mailbox.pending(id).is_empty() {
            return (0, 0);
        }
        let now = self.now();
        // A Folder answer indexes the folder list it was asked with; after a folder change the
        // index may point elsewhere, so only a current-version Folder answer is routed.
        let folders = crate::judge::folder_question_version(&self.mailbox.folder_options(&message.account));
        let pairs: Vec<(QuestionKey, Answer)> = answers
            .iter()
            .filter(|a| a.key != QuestionKey::Folder || a.version == folders)
            .map(|a| (a.key, a.answer.clone()))
            .collect();
        let (mut auto, mut review) = (0, Vec::new());
        for routed in route(&self.policy, &message, pairs) {
            match routed {
                Routed::Auto(s) => {
                    if self.mailbox.apply_auto(s, now) {
                        auto += 1;
                    }
                }
                Routed::Review(s) => review.push(s),
                Routed::Drop => {}
            }
        }
        let queued = review.len();
        self.mailbox.add_suggestions(review);
        (auto, queued)
    }

    /// Attach the current version to each fresh answer, merge it into the stored set per key,
    /// keep the memory cache and SQLite in step, and return the merged set.
    fn merge_and_store(
        &mut self,
        id: MessageId,
        model: &str,
        answers: &[(QuestionKey, Answer)],
        now: Timestamp,
    ) -> Vec<StoredAnswer> {
        let account = self.mailbox.get(id).map(|m| m.account.clone()).unwrap_or_default();
        let specs = self.classify_specs(&account);
        let mut merged = self.judgements.get(&id).cloned().unwrap_or_default();
        for (key, answer) in answers {
            let version = specs
                .iter()
                .find(|s| s.key == *key)
                .map(|s| s.version.clone())
                .unwrap_or_else(|| "1".to_owned());
            merged.retain(|a| a.key != *key);
            merged.push(StoredAnswer {
                key: *key,
                version,
                answer: answer.clone(),
            });
        }
        merged.sort_by_key(|a| QuestionKey::ALL.iter().position(|k| *k == a.key).unwrap_or(usize::MAX));
        self.judgements.insert(id, merged.clone());
        if let Some(store) = self.preferences.clone() {
            // A cache write failure is not worth stopping classification over.
            let _ = store.judgement_put(id, model, &merged, now);
        }
        merged
    }

    /// Queue `job` at the back, unless the same message is already waiting or in flight.
    pub(super) fn enqueue_judge(&mut self, job: JudgeJob) {
        self.enqueue_judge_at(job, false);
    }

    /// Queue `job` at the front (an open or a manual classify jumps the queue).
    pub(super) fn enqueue_judge_front(&mut self, job: JudgeJob) {
        self.enqueue_judge_at(job, true);
    }

    /// Queue `job`, unless the same message is already waiting or in flight. Starting a batch
    /// (queue empty, nothing in flight) resets the footer counters; anything queued later grows
    /// the batch's total.
    fn enqueue_judge_at(&mut self, job: JudgeJob, front: bool) {
        if let JudgeJob::Message { id, .. } = &job
            && (self.judge_in_flight == Some(*id)
                || self
                    .judge_queue
                    .iter()
                    .any(|j| matches!(j, JudgeJob::Message { id: queued, .. } if queued == id)))
        {
            return;
        }
        if self.judge_queue.is_empty() && !self.judging {
            self.judge_batch_done = 0;
            self.judge_batch_total = 0;
        }
        self.judge_batch_total += 1;
        if front {
            self.judge_queue.push_front(job);
        } else {
            self.judge_queue.push_back(job);
        }
    }

    /// Queue a background judge of the reply sent in answer to `in_reply_to`. Only when
    /// automatic classification is on; with it off, no reply is judged.
    pub(super) fn judge_reply(
        &mut self,
        in_reply_to: MessageId,
        seq: u64,
        sent_at: Timestamp,
        body: String,
        cx: &mut Context<Self>,
    ) {
        if self.judge.is_none() || !self.auto_classify {
            return;
        }
        self.enqueue_judge(JudgeJob::Reply { in_reply_to, seq, sent_at, body });
        self.drain_judge_queue(cx);
    }

    /// Start the next request if the worker is idle; one request runs at a time.
    pub(super) fn drain_judge_queue(&mut self, cx: &mut Context<Self>) {
        loop {
            if self.judging {
                return;
            }
            let Some(judge) = self.judge.clone() else {
                break;
            };
            let Some(job) = self.judge_queue.pop_front() else {
                break;
            };
            let model = judge.model().to_owned();
            let Some((kind, state, questions)) = self.prepare_job(job) else {
                continue; // nothing to ask (or the message went away); try the next one
            };
            self.judging = true;
            self.judge_in_flight = match &kind {
                JudgeJob::Message { id, .. } => Some(*id),
                JudgeJob::Reply { .. } => None,
            };
            self.refresh_judge_status();
            let generation = self.judge_gen;
            let task = cx.background_spawn(async move { judge.judge(&state, &questions) });
            cx.spawn(async move |this, cx| {
                let result = task.await;
                this.update(cx, |this, cx| this.finish_judge(generation, kind, model, result, cx)).ok();
            })
            .detach();
            return;
        }
        self.refresh_judge_status();
    }

    /// The request body for `job`: its kind, state and questions.
    #[allow(clippy::type_complexity)]
    fn prepare_job(
        &self,
        job: JudgeJob,
    ) -> Option<(JudgeJob, Value, Vec<(QuestionKey, Question)>)> {
        match &job {
            JudgeJob::Message { id, force } => {
                let message = self.mailbox.get(*id).cloned()?;
                // Never send a partial message: `body_ready` sends it once the body lands.
                if message.partial {
                    return None;
                }
                let specs = self.classify_specs(&message.account);
                let stored = self.judgements.get(id).map(Vec::as_slice).unwrap_or(&[]);
                let questions: Vec<(QuestionKey, Question)> = if *force {
                    specs.iter().map(|s| (s.key, s.question.clone())).collect()
                } else {
                    questions_to_ask(&specs, stored)
                };
                if questions.is_empty() {
                    return None;
                }
                let me = self
                    .mailbox
                    .account(&message.account)
                    .map(|a| a.email.clone())
                    .unwrap_or_default();
                Some((job, message_state(&message, &me), questions))
            }
            JudgeJob::Reply { body, .. } => {
                let state = json!({"sender": "", "subject": "", "body": body.trim()});
                Some((job, state, vec![reply_question()]))
            }
        }
    }

    /// A finished request: cache and apply it, or stop on error and show it.
    fn finish_judge(
        &mut self,
        generation: u64,
        kind: JudgeJob,
        model: String,
        result: Result<Vec<(QuestionKey, Answer)>, JudgeError>,
        cx: &mut Context<Self>,
    ) {
        let stale = generation != self.judge_gen;
        // Free the physical slot in every case; `set_judge` keeps it occupied across a swap.
        self.judging = false;
        self.judge_in_flight = None;
        self.judge_batch_done += 1;
        if stale {
            // The judge was replaced while this ran: drop the result, then start whatever the
            // replacement's queue holds.
            self.drain_judge_queue(cx);
            cx.notify();
            return;
        }
        match result {
            Err(e) => {
                if let JudgeJob::Message { id, .. } = &kind {
                    self.classify_failed.insert(*id);
                }
                // Stop the worker, but keep reply jobs: unlike messages, `classify_visible`
                // cannot rebuild them, and the user's send still waits on its classification.
                self.judge_queue.retain(|j| matches!(j, JudgeJob::Reply { .. }));
                if matches!(kind, JudgeJob::Reply { .. }) {
                    self.judge_queue.push_front(kind);
                }
                self.judge_status = JudgeStatus::Error(e.to_string());
                cx.notify();
                return;
            }
            Ok(answers) => match kind {
                JudgeJob::Message { id, .. } => {
                    self.classify_failed.remove(&id);
                    let now = self.now();
                    let stored = self.merge_and_store(id, &model, &answers, now);
                    self.apply_answers(id, &stored);
                }
                JudgeJob::Reply { in_reply_to, seq, sent_at, .. } => {
                    let expects = answers.iter().any(|(key, answer)| {
                        *key == QuestionKey::ExpectsReply
                            && answer.value == AnswerValue::Bool(true)
                    });
                    if expects {
                        self.mailbox.mark_awaiting_reply(in_reply_to, seq, sent_at);
                    }
                }
            },
        }
        self.refresh_judge_status();
        cx.notify();
        self.drain_judge_queue(cx);
    }

    /// Recompute the status from the judge and the queue; a success clears any error.
    pub(super) fn refresh_judge_status(&mut self) {
        self.judge_status = if self.judge.is_none() {
            JudgeStatus::NoKey
        } else {
            let waiting = self.judge_queue.len() + usize::from(self.judging);
            if waiting == 0 {
                // The batch drained: the next enqueue starts a fresh one.
                self.judge_batch_done = 0;
                self.judge_batch_total = 0;
                JudgeStatus::Idle
            } else {
                JudgeStatus::Working {
                    done: self.judge_batch_done,
                    total: self.judge_batch_total,
                }
            }
        };
    }
}
