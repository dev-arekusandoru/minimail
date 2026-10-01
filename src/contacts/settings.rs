//! Scoped key/value settings rows (`settings` table, JSON values).

use rusqlite::{OptionalExtension, params};

use super::ContactStore;
use super::error::Result;

impl ContactStore {
    /// Raw JSON stored for `key` in `scope` (`"global"`, `"account:<id>"`).
    pub fn setting_get(&self, scope: &str, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT value FROM settings WHERE scope = ?1 AND key = ?2",
                params![scope, key],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Insert or replace the JSON value for `key` in `scope`.
    pub fn setting_set(&self, scope: &str, key: &str, json: &str) -> Result<()> {
        self.conn().execute(
            "INSERT INTO settings (scope, key, value) VALUES (?1, ?2, ?3)
             ON CONFLICT(scope, key) DO UPDATE SET value = excluded.value",
            params![scope, key, json],
        )?;
        Ok(())
    }

    /// Remove one row; missing rows are not an error.
    pub fn setting_delete(&self, scope: &str, key: &str) -> Result<()> {
        self.conn().execute(
            "DELETE FROM settings WHERE scope = ?1 AND key = ?2",
            params![scope, key],
        )?;
        Ok(())
    }

    /// Remove every setting in every scope.
    pub fn settings_clear(&self) -> Result<()> {
        self.conn().execute("DELETE FROM settings", [])?;
        Ok(())
    }
}
