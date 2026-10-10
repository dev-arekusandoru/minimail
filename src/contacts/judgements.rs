//! Cached judge answers (`judgements` table), keyed by [`MessageId`]. A message is judged at most
//! once, ever: the answer is reused whatever changes about its text, the model, the endpoint or
//! the key. Answers are stored as JSON so routing can be re-applied at read time without another
//! API call. The pre-`MIGRATION_4` hash-keyed rows live in `judgements_legacy` until
//! [`ContactStore::carry_over_judgements`] maps and drops them.

use rusqlite::{OptionalExtension, params};

use super::ContactStore;
use super::error::{Error, Result};
use crate::clock::Timestamp;
use crate::judge::{Answer, QuestionKey};
use crate::model::MessageId;

impl ContactStore {
    /// The cached answers for `message_id`, or `None` if never judged.
    pub fn judgement_get(&self, message_id: MessageId) -> Result<Option<Vec<(QuestionKey, Answer)>>> {
        let json: Option<String> = self
            .conn()
            .query_row(
                "SELECT answers FROM judgements WHERE message_id = ?1",
                [message_id],
                |r| r.get(0),
            )
            .optional()?;
        let Some(json) = json else {
            return Ok(None);
        };
        let answers = serde_json::from_str(&json)
            .map_err(|e| Error::Invalid(format!("stored judgement is not JSON: {e}")))?;
        Ok(Some(answers))
    }

    /// Insert or replace `message_id`'s answers, recording `model` and `now` as `judged_at`.
    /// `model` is informational: the lookup ignores it, so a model change never re-bills.
    pub fn judgement_put(
        &self,
        message_id: MessageId,
        model: &str,
        answers: &[(QuestionKey, Answer)],
        now: Timestamp,
    ) -> Result<()> {
        let json = serde_json::to_string(answers)
            .map_err(|e| Error::Invalid(format!("cannot encode judgement: {e}")))?;
        self.conn().execute(
            "INSERT INTO judgements (message_id, answers, model, judged_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(message_id) DO UPDATE SET answers = excluded.answers, model = excluded.model, judged_at = excluded.judged_at",
            params![message_id, json, model, now],
        )?;
        Ok(())
    }

    /// Whether the pre-`MIGRATION_4` hash-keyed table still needs carrying over.
    pub fn legacy_judgements_present(&self) -> Result<bool> {
        let count: i64 = self.conn().query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'judgements_legacy'",
            [],
            |r| r.get(0),
        )?;
        Ok(count > 0)
    }

    /// One-time carry-over from the hash-keyed legacy table: for each `(message_id, legacy_key)`
    /// whose message has no id-keyed row, copy the legacy answer (with its original `judged_at`)
    /// under the message id, then drop the legacy table. A message whose legacy key does not
    /// match (e.g. its model changed before the upgrade) is left unjudged.
    pub fn carry_over_judgements(
        &self,
        model: &str,
        entries: &[(MessageId, String)],
    ) -> Result<()> {
        if !self.legacy_judgements_present()? {
            return Ok(());
        }
        for (message_id, key) in entries {
            let has_row: Option<i64> = self
                .conn()
                .query_row(
                    "SELECT 1 FROM judgements WHERE message_id = ?1",
                    [message_id],
                    |r| r.get(0),
                )
                .optional()?;
            if has_row.is_some() {
                continue;
            }
            let legacy: Option<(String, Timestamp)> = self
                .conn()
                .query_row(
                    "SELECT answers, judged_at FROM judgements_legacy WHERE key = ?1",
                    [key],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if let Some((answers, judged_at)) = legacy {
                self.conn().execute(
                    "INSERT OR IGNORE INTO judgements (message_id, answers, model, judged_at)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![message_id, answers, model, judged_at],
                )?;
            }
        }
        // Dropped only after the copy ran, so a failure retries on the next launch.
        self.conn().execute("DROP TABLE judgements_legacy", [])?;
        Ok(())
    }
}
