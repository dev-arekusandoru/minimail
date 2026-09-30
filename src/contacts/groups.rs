//! Groups: named labels many contacts can share ("Work", "Family").

use rusqlite::{Connection, OptionalExtension, params};

use super::error::{Error, Result, invalid};
use super::model::Group;
use super::ContactStore;

/// Look the group up by case-insensitive name, creating it when missing.
/// Works on a `Transaction` too, which derefs to `&Connection`.
pub(crate) fn ensure_group(conn: &Connection, name: &str) -> Result<i64> {
    let name = clean_name(name)?;
    if let Some(id) = conn
        .query_row(
            "SELECT id FROM contact_groups WHERE LOWER(name) = LOWER(?1)",
            params![name],
            |r| r.get(0),
        )
        .optional()?
    {
        return Ok(id);
    }
    conn.execute("INSERT INTO contact_groups (name) VALUES (?1)", params![name])?;
    Ok(conn.last_insert_rowid())
}

fn clean_name(name: &str) -> Result<&str> {
    let name = name.trim();
    if name.is_empty() {
        return Err(invalid("empty group name"));
    }
    Ok(name)
}

fn take_name(name: &str) -> Result<&str> {
    clean_name(name)
}

impl ContactStore {
    /// Create a group. Names are unique, case-insensitively.
    pub fn create_group(&self, name: &str) -> Result<Group> {
        let name = take_name(name)?;
        match self.conn.execute(
            "INSERT INTO contact_groups (name) VALUES (?1)",
            params![name],
        ) {
            Ok(_) => Ok(Group { id: self.conn.last_insert_rowid(), name: name.to_string() }),
            Err(e) if e.to_string().contains("UNIQUE constraint failed: contact_groups.name") => {
                Err(Error::DuplicateGroup(name.to_string()))
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Every group, alphabetically.
    pub fn groups(&self) -> Result<Vec<Group>> {
        let mut stmt = self.conn.prepare("SELECT id, name FROM contact_groups ORDER BY name")?;
        let rows = stmt.query_map([], |r| {
            Ok(Group { id: r.get(0)?, name: r.get(1)? })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn group_id(&self, name: &str) -> Result<Option<i64>> {
        let name = take_name(name)?;
        Ok(self
            .conn
            .query_row(
                "SELECT id FROM contact_groups WHERE LOWER(name) = LOWER(?1)",
                params![name],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Rename a group, keeping its members.
    pub fn rename_group(&self, id: i64, name: &str) -> Result<Group> {
        let name = take_name(name)?;
        let changed = self.conn.execute(
            "UPDATE contact_groups SET name = ?2 WHERE id = ?1",
            params![id, name],
        )?;
        if changed == 0 {
            return Err(Error::NotFound(format!("group {id}")));
        }
        Ok(Group { id, name: name.to_string() })
    }

    /// Delete a group; its contacts keep everything else. False if there was
    /// no such group.
    pub fn delete_group(&self, id: i64) -> Result<bool> {
        Ok(self.conn.execute("DELETE FROM contact_groups WHERE id = ?1", params![id])? > 0)
    }

    /// Put a contact in a group. False when it was already a member.
    pub fn assign_group(&self, contact_id: i64, group_id: i64) -> Result<bool> {
        self.require_contact(contact_id)?;
        let changed = self.conn.execute(
            "INSERT OR IGNORE INTO contact_group_members (contact_id, group_id) VALUES (?1, ?2)",
            params![contact_id, group_id],
        )?;
        Ok(changed > 0)
    }

    /// Take a contact out of a group. False when it was not a member.
    pub fn remove_group(&self, contact_id: i64, group_id: i64) -> Result<bool> {
        Ok(self.conn.execute(
            "DELETE FROM contact_group_members WHERE contact_id = ?1 AND group_id = ?2",
            params![contact_id, group_id],
        )? > 0)
    }

    fn require_contact(&self, id: i64) -> Result<()> {
        let found = self.conn.query_row(
            "SELECT 1 FROM contacts WHERE id = ?1",
            params![id],
            |r| r.get::<_, i64>(0),
        );
        match found {
            Ok(_) => Ok(()),
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                Err(Error::NotFound(format!("contact {id}")))
            }
            Err(e) => Err(e.into()),
        }
    }
}
