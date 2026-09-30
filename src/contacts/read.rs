//! Reading contacts: by id, by address, by search, and the address set the
//! mailbox uses for its Screener.

use std::collections::HashSet;

use rusqlite::{OptionalExtension, Row, params};

use super::children::{self, Children};
use super::error::Result;
use super::model::{Contact, ContactQuery, ContactSource};
use super::{ContactStore, normalize_email};

/// `contacts` columns in the order [`contact_from_row`] expects.
const COLUMNS: &str = "id, prefix, given_name, middle_name, family_name, suffix, nickname,
    display_name, organization, department, job_title, birthday, notes, photo, favorite, source,
    created_at, updated_at, last_contacted_at";

/// The contact columns of one row; children are loaded in bulk afterwards.
fn contact_from_row(row: &Row) -> rusqlite::Result<Contact> {
    Ok(Contact {
        id: row.get(0)?,
        prefix: row.get(1)?,
        given_name: row.get(2)?,
        middle_name: row.get(3)?,
        family_name: row.get(4)?,
        suffix: row.get(5)?,
        nickname: row.get(6)?,
        display_name: row.get(7)?,
        organization: row.get(8)?,
        department: row.get(9)?,
        job_title: row.get(10)?,
        birthday: row.get(11)?,
        notes: row.get(12)?,
        photo: row.get(13)?,
        favorite: row.get::<_, i64>(14)? != 0,
        source: ContactSource::parse(&row.get::<_, String>(15)?),
        created_at: row.get(16)?,
        updated_at: row.get(17)?,
        last_contacted_at: row.get(18)?,
        emails: Vec::new(),
        phones: Vec::new(),
        addresses: Vec::new(),
        urls: Vec::new(),
        groups: Vec::new(),
    })
}

/// Fold the child collections into the contact.
fn assemble(mut contact: Contact, children: Children) -> Contact {
    contact.emails = children.emails;
    contact.phones = children.phones;
    contact.addresses = children.addresses;
    contact.urls = children.urls;
    contact.groups = children.groups;
    contact
}

/// Escape LIKE wildcards in a user-supplied search term.
fn like_prefix(text: &str) -> String {
    let escaped = text
        .trim()
        .to_lowercase()
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("{escaped}%")
}

impl ContactStore {
    /// One contact with its children, or `None`.
    pub fn get(&self, id: i64) -> Result<Option<Contact>> {
        let sql = format!("SELECT {COLUMNS} FROM contacts WHERE id = ?1");
        let found = self
            .conn
            .query_row(&sql, params![id], contact_from_row)
            .optional()?;
        let Some(contact) = found else {
            return Ok(None);
        };
        let mut kids = self.children_of(&[contact.id])?;
        let children = kids.remove(&contact.id).unwrap_or_default();
        Ok(Some(assemble(contact, children)))
    }

    /// One contact by email address; matching is case-insensitive.
    pub fn get_by_email(&self, email: &str) -> Result<Option<Contact>> {
        let address = normalize_email(email)?;
        let id = self
            .conn
            .query_row(
                "SELECT contact_id FROM contact_emails WHERE address = ?1",
                params![address],
                |r| r.get::<_, i64>(0),
            )
            .optional()?;
        match id {
            Some(id) => self.get(id),
            None => Ok(None),
        }
    }

    /// Contacts matching `query`, in display-name order, then by id.
    pub fn search(&self, query: &ContactQuery) -> Result<Vec<Contact>> {
        let text = query.text.as_deref().map(like_prefix);
        let group = query
            .group
            .as_deref()
            .map(str::trim)
            .filter(|g| !g.is_empty())
            .map(|g| g.to_lowercase());
        // LIMIT -1 is SQLite's "no limit".
        let limit = query.limit.map_or(-1i64, |l| l as i64);
        let sql = format!(
            "SELECT {COLUMNS} FROM contacts c
             WHERE (?1 IS NULL
                    OR LOWER(c.display_name) LIKE ?1 ESCAPE '\\'
                    OR LOWER(COALESCE(c.organization, '')) LIKE ?1 ESCAPE '\\'
                    OR EXISTS (SELECT 1 FROM contact_emails e
                               WHERE e.contact_id = c.id AND e.address LIKE ?1 ESCAPE '\\'))
               AND (?2 = 0 OR c.favorite = 1)
               AND (?3 IS NULL OR EXISTS (
                        SELECT 1 FROM contact_group_members m
                        JOIN contact_groups g ON g.id = m.group_id
                        WHERE m.contact_id = c.id AND LOWER(g.name) = ?3))
             ORDER BY LOWER(c.display_name), c.id
             LIMIT ?4 OFFSET ?5"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(
            params![text, query.favorites_only as i64, group, limit, query.offset as i64],
            contact_from_row,
        )?;
        let mut contacts: Vec<Contact> = Vec::new();
        for row in rows {
            contacts.push(row?);
        }
        drop(stmt);

        let ids: Vec<i64> = contacts.iter().map(|c| c.id).collect();
        let mut children = self.children_of(&ids)?;
        Ok(contacts
            .into_iter()
            .map(|contact| {
                let kids = children.remove(&contact.id).unwrap_or_default();
                assemble(contact, kids)
            })
            .collect())
    }

    /// Every stored address, lowercased. This is the Screener's "known" set.
    pub fn known_addresses(&self) -> Result<HashSet<String>> {
        let mut stmt = self.conn.prepare("SELECT address FROM contact_emails")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<rusqlite::Result<HashSet<_>>>()?)
    }

    /// Whether the address belongs to a contact.
    pub fn is_known(&self, email: &str) -> Result<bool> {
        let address = normalize_email(email)?;
        let found = self.conn.query_row(
            "SELECT 1 FROM contact_emails WHERE address = ?1",
            params![address],
            |r| r.get::<_, i64>(0),
        );
        Ok(found.is_ok())
    }

    /// Undo of a Screener allow: drop the address, and with it a contact the
    /// Screener created itself. False when the address was not stored.
    pub fn forget_address(&self, email: &str) -> Result<bool> {
        let address = normalize_email(email)?;
        let owner = self
            .conn
            .query_row(
                "SELECT c.id, c.source FROM contact_emails e
                 JOIN contacts c ON c.id = e.contact_id WHERE e.address = ?1",
                params![address],
                |r| Ok((r.get::<_, i64>(0)?, ContactSource::parse(&r.get::<_, String>(1)?))),
            )
            .optional()?;
        let Some((id, source)) = owner else {
            return Ok(false);
        };
        // A contact that existed before the Screener only loses this address.
        if source == ContactSource::Screener {
            return self.delete(id);
        }
        self.conn.execute("DELETE FROM contact_emails WHERE address = ?1", params![address])?;
        Ok(true)
    }

    fn children_of(&self, ids: &[i64]) -> Result<std::collections::HashMap<i64, Children>> {
        children::load_children(&self.conn, ids)
    }
}
