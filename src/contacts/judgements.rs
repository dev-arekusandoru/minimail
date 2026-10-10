//! Cached judge answers (`judgements` table), keyed by
//! [`crate::judge::judgement_key`]. Answers are stored as JSON so routing can
//! be re-applied at read time without another API call.

use rusqlite::{OptionalExtension, params};

use super::ContactStore;
use super::error::{Error, Result};
use crate::clock::Timestamp;
use crate::judge::{Answer, QuestionKey};

impl ContactStore {
    /// The cached answers for `key`, or `None` if never judged.
    pub fn judgement_get(&self, key: &str) -> Result<Option<Vec<(QuestionKey, Answer)>>> {
        let json: Option<String> = self
            .conn()
            .query_row("SELECT answers FROM judgements WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()?;
        let Some(json) = json else {
            return Ok(None);
        };
        let answers = serde_json::from_str(&json)
            .map_err(|e| Error::Invalid(format!("stored judgement is not JSON: {e}")))?;
        Ok(Some(answers))
    }

    /// Insert or replace the answers for `key`, recording `now` as `judged_at`.
    pub fn judgement_put(
        &self,
        key: &str,
        answers: &[(QuestionKey, Answer)],
        now: Timestamp,
    ) -> Result<()> {
        let json = serde_json::to_string(answers)
            .map_err(|e| Error::Invalid(format!("cannot encode judgement: {e}")))?;
        self.conn().execute(
            "INSERT INTO judgements (key, answers, judged_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET answers = excluded.answers, judged_at = excluded.judged_at",
            params![key, json, now],
        )?;
        Ok(())
    }
}
