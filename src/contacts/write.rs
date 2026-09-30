//! Creating, replacing and deleting contacts.

use rusqlite::{Connection, Transaction, params};

use super::error::{Error, invalid, Result};
use super::model::{Contact, ContactSource, EmailAddress, NewContact};
use super::{ContactStore, children, groups, normalize_email};

impl ContactStore {
    /// Insert a contact with its children. Emails are stored lowercased and
    /// must be unique across the store.
    pub fn create(&self, new: NewContact) -> Result<Contact> {
        if new.display_name.trim().is_empty() && new.emails.is_empty() {
            return Err(invalid("a contact needs a display name or an email"));
        }
        let now = self.now();
        let tx = self.conn.unchecked_transaction()?;
        let id = insert_contact(&tx, &new, now, None)?;
        write_children(&tx, id, &new)?;
        tx.commit()?;
        self.require(id)
    }

    /// Replace the stored contact with `new`, keeping its id and
    /// `created_at`. A non-empty child collection in `new` replaces the stored
    /// children; an empty one leaves them untouched.
    pub fn update(&self, id: i64, new: NewContact) -> Result<Contact> {
        if new.display_name.trim().is_empty() && new.emails.is_empty() {
            return Err(invalid("a contact needs a display name or an email"));
        }
        let now = self.now();
        let tx = self.conn.unchecked_transaction()?;
        let changed = tx.execute(
            "UPDATE contacts SET
                prefix = ?2, given_name = ?3, middle_name = ?4, family_name = ?5, suffix = ?6,
                nickname = ?7, display_name = ?8, organization = ?9, department = ?10,
                job_title = ?11, birthday = ?12, notes = ?13, photo = ?14, favorite = ?15,
                source = ?16, updated_at = ?17
             WHERE id = ?1",
            params![id, new.prefix, new.given_name, new.middle_name, new.family_name, new.suffix,
                new.nickname, new.display_name, new.organization, new.department, new.job_title,
                new.birthday, new.notes, new.photo, new.favorite, new.source.as_str(), now],
        )?;
        if changed == 0 {
            return Err(Error::NotFound(format!("contact {id}")));
        }
        write_children(&tx, id, &new)?;
        tx.commit()?;
        self.require(id)
    }

    /// Delete a contact and everything hanging off it. False if there was no
    /// such contact.
    pub fn delete(&self, id: i64) -> Result<bool> {
        let tx = self.conn.unchecked_transaction()?;
        let changed = tx.execute("DELETE FROM contacts WHERE id = ?1", params![id])?;
        tx.commit()?;
        Ok(changed > 0)
    }

    /// Find a contact by address, creating a minimal one when the address is
    /// unknown. Returns the contact and whether it was created; this is the
    /// path the Screener uses when a sender is allowed.
    pub fn upsert_from_email(
        &self,
        email: &str,
        display_name: Option<&str>,
        source: ContactSource,
    ) -> Result<(Contact, bool)> {
        let address = normalize_email(email)?;
        if let Some(existing) = self.get_by_email(&address)? {
            return Ok((existing, false));
        }
        let fallback = address.split('@').next().unwrap_or_default().to_string();
        let name = display_name.map(str::trim).filter(|n| !n.is_empty()).unwrap_or(&fallback);
        let contact = self.create(NewContact::from_email(address, name).source(source))?;
        Ok((contact, true))
    }

    /// Mark or unmark a contact as a favorite.
    pub fn set_favorite(&self, id: i64, favorite: bool) -> Result<()> {
        self.touch_row(id, "UPDATE contacts SET favorite = ?2 WHERE id = ?1", favorite as i64)
    }

    /// Record that the contact was contacted just now.
    pub fn touch(&self, id: i64) -> Result<()> {
        let now = self.now();
        self.touch_row(id, "UPDATE contacts SET last_contacted_at = ?2, updated_at = ?2 WHERE id = ?1", now)
    }

    fn touch_row(&self, id: i64, sql: &str, value: impl rusqlite::ToSql) -> Result<()> {
        let changed = self.conn.execute(sql, params![id, value])?;
        if changed == 0 {
            return Err(Error::NotFound(format!("contact {id}")));
        }
        Ok(())
    }

    fn require(&self, id: i64) -> Result<Contact> {
        self.get(id)?.ok_or_else(|| Error::NotFound(format!("contact {id}")))
    }
}

fn insert_contact(
    tx: &Transaction,
    new: &NewContact,
    now: crate::clock::Timestamp,
    id: Option<i64>,
) -> Result<i64> {
    tx.execute(
        "INSERT INTO contacts (id, prefix, given_name, middle_name, family_name, suffix, nickname,
            display_name, organization, department, job_title, birthday, notes, photo, favorite,
            source, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?17)",
        params![id, new.prefix, new.given_name, new.middle_name, new.family_name, new.suffix,
            new.nickname, new.display_name, new.organization, new.department, new.job_title,
            new.birthday, new.notes, new.photo, new.favorite, new.source.as_str(), now],
    )?;
    Ok(tx.last_insert_rowid())
}

/// Replace every child collection of `id` with the ones in `new`, leaving a
/// collection alone when `new` leaves it empty.
fn write_children(tx: &Transaction, id: i64, new: &NewContact) -> Result<()> {
    if !new.emails.is_empty() {
        tx.execute("DELETE FROM contact_emails WHERE contact_id = ?1", params![id])?;
        for email in &new.emails {
            insert_email(tx, id, email)?;
        }
        enforce_one_primary(tx, id)?;
    }
    if !new.phones.is_empty() {
        tx.execute("DELETE FROM contact_phones WHERE contact_id = ?1", params![id])?;
        for phone in &new.phones {
            children::insert_phone(tx, id, phone)?;
        }
    }
    if !new.addresses.is_empty() {
        tx.execute("DELETE FROM contact_addresses WHERE contact_id = ?1", params![id])?;
        for address in &new.addresses {
            children::insert_address(tx, id, address)?;
        }
    }
    if !new.urls.is_empty() {
        tx.execute("DELETE FROM contact_urls WHERE contact_id = ?1", params![id])?;
        for url in &new.urls {
            children::insert_url(tx, id, url)?;
        }
    }
    if !new.groups.is_empty() {
        tx.execute("DELETE FROM contact_group_members WHERE contact_id = ?1", params![id])?;
        for name in &new.groups {
            let group = groups::ensure_group(tx, name)?;
            tx.execute(
                "INSERT OR IGNORE INTO contact_group_members (contact_id, group_id) VALUES (?1, ?2)",
                params![id, group],
            )?;
        }
    }
    Ok(())
}

/// Keep exactly one primary email per contact: an explicitly flagged address
/// (see [`insert_email`]) wins, otherwise the oldest one is promoted.
pub(crate) fn enforce_one_primary(conn: &Connection, contact_id: i64) -> Result<()> {
    conn.execute(
        "UPDATE contact_emails SET is_primary = 0
         WHERE contact_id = ?1 AND is_primary = 1
           AND id <> (SELECT MIN(id) FROM contact_emails WHERE contact_id = ?1 AND is_primary = 1)",
        params![contact_id],
    )?;
    conn.execute(
        "UPDATE contact_emails SET is_primary = 1
         WHERE contact_id = ?1 AND id = (SELECT MIN(id) FROM contact_emails WHERE contact_id = ?1)
           AND NOT EXISTS (SELECT 1 FROM contact_emails WHERE contact_id = ?1 AND is_primary = 1)",
        params![contact_id],
    )?;
    Ok(())
}

/// Turn a unique-violation on `contact_emails.address` into a typed error.
pub(crate) fn map_email_error(e: rusqlite::Error, address: &str) -> Error {
    let msg = e.to_string();
    if msg.contains("UNIQUE constraint failed: contact_emails.address") {
        Error::DuplicateEmail(address.to_string())
    } else {
        Error::Sqlite(e)
    }
}

/// Insert one email row, mapping duplicates. A flagged primary outranks the
/// contact's previous one, so the most recently designated address wins.
pub(crate) fn insert_email(conn: &Connection, contact_id: i64, email: &EmailAddress) -> Result<()> {
    let address = normalize_email(&email.address)?;
    if email.primary {
        conn.execute(
            "UPDATE contact_emails SET is_primary = 0 WHERE contact_id = ?1",
            params![contact_id],
        )?;
    }
    conn.execute(
        "INSERT INTO contact_emails (contact_id, address, label, is_primary) VALUES (?1, ?2, ?3, ?4)",
        params![contact_id, address, email.label.as_str(), email.primary as i64],
    )
    .map_err(|e| map_email_error(e, &address))?;
    Ok(())
}
