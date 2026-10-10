//! The classifier's background worker: a serial queue draining one judge request at a time.
//!
//! `classify_visible` routes cached answers immediately and queues the rest; this module runs
//! each queued request off-thread, stores the answers, and applies the routing back on the UI
//! thread. One request is in flight at a time.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::app_settings::{JEV_BASE_URL, JEV_MODEL};
use crate::judge::{
    Answer, AnswerValue, Judge, JudgeError, JudgeStatus, Question, QuestionKey,
    judgement_key, message_state, route, triage_questions,
};
use crate::prefs::Scope;

use super::*;

/// Keychain entry holding the Jev API key. Accounts are named by their email, so it cannot collide.
const JEV_SECRET: &str = "jev";

/// One unit of classifier work.
#[derive(Clone, Debug)]
pub(super) enum JudgeJob {
    /// Classify the Inbox message `id`.
    Message(MessageId),
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

    /// Cached answers for `id`: memory first, then the store. `None` on a miss or a bad row.
    pub(super) fn cached_answers(&mut self, id: MessageId) -> Option<Vec<(QuestionKey, Answer)>> {
        if let Some(answers) = self.judgements.get(&id) {
            return Some(answers.clone());
        }
        let store = self.preferences.clone()?;
        // A corrupt row is a cache miss: judging again overwrites it.
        match store.judgement_get(id) {
            Ok(Some(answers)) => {
                self.judgements.insert(id, answers.clone());
                Some(answers)
            }
            Ok(None) | Err(_) => None,
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
        let (_, model) = self.jev_settings();
        let questions = triage_questions();
        let entries: Vec<(MessageId, String)> = self
            .mailbox
            .messages()
            .iter()
            .map(|m| (m.id, judgement_key(&model, &message_state(m), &questions)))
            .collect();
        // A failure leaves the legacy table in place, to retry on the next launch.
        let _ = store.carry_over_judgements(&model, &entries);
    }

    /// Route `answers` for `id` and apply them, if it is still a visible Inbox candidate.
    pub(super) fn apply_answers(
        &mut self,
        id: MessageId,
        answers: &[(QuestionKey, Answer)],
    ) -> (usize, usize) {
        let Some(message) = self.mailbox.get(id).cloned() else {
            return (0, 0);
        };
        if message.state != TriageState::Inbox || !self.mailbox.pending(id).is_empty() {
            return (0, 0);
        }
        let now = self.now();
        let (mut auto, mut review) = (0, Vec::new());
        for routed in route(&self.policy, &message, answers.to_vec()) {
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

    /// Queue `job`, unless the same message is already waiting or in flight. Starting a batch
    /// (queue empty, nothing in flight) resets the footer counters; anything queued later grows
    /// the batch's total.
    pub(super) fn enqueue_judge(&mut self, job: JudgeJob) {
        if let JudgeJob::Message(id) = &job
            && (self.judge_in_flight == Some(*id)
                || self
                    .judge_queue
                    .iter()
                    .any(|j| matches!(j, JudgeJob::Message(queued) if queued == id)))
        {
            return;
        }
        if self.judge_queue.is_empty() && !self.judging {
            self.judge_batch_done = 0;
            self.judge_batch_total = 0;
        }
        self.judge_batch_total += 1;
        self.judge_queue.push_back(job);
    }

    /// Queue a background judge of the reply sent in answer to `in_reply_to`.
    pub(super) fn judge_reply(
        &mut self,
        in_reply_to: MessageId,
        seq: u64,
        sent_at: Timestamp,
        body: String,
        cx: &mut Context<Self>,
    ) {
        if self.judge.is_none() {
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
                continue; // the message went away; try the next one
            };
            self.judging = true;
            self.judge_in_flight = match &kind {
                JudgeJob::Message(id) => Some(*id),
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
            JudgeJob::Message(id) => {
                let message = self.mailbox.get(*id).cloned()?;
                let state = message_state(&message);
                let questions = triage_questions();
                Some((job, state, questions))
            }
            JudgeJob::Reply { body, .. } => {
                let state = json!({"sender": "", "subject": "", "body": body.trim()});
                let questions = vec![(
                    QuestionKey::ExpectsReply,
                    Question::Bool {
                        criteria: "Does this outgoing reply ask the recipient for a response?".into(),
                    },
                )];
                Some((job, state, questions))
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
                JudgeJob::Message(id) => {
                    let now = self.now();
                    self.store_answers(id, &model, &answers, now);
                    self.apply_answers(id, &answers);
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

    /// Cache `answers` in memory and, when a store exists, in SQLite.
    fn store_answers(
        &mut self,
        id: MessageId,
        model: &str,
        answers: &[(QuestionKey, Answer)],
        now: Timestamp,
    ) {
        self.judgements.insert(id, answers.to_vec());
        if let Some(store) = self.preferences.clone() {
            // A cache write failure is not worth stopping classification over.
            let _ = store.judgement_put(id, model, answers, now);
        }
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
