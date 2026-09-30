//! Emails, phones, postal addresses, URLs and custom fields of a contact.

use std::collections::HashMap;

use rusqlite::{Connection, params, params_from_iter};

use super::error::{Result, invalid};
use super::model::{ContactUrl, EmailAddress, Field, Label, PhoneNumber, PostalAddress};
use super::write::insert_email;
use super::ContactStore;

/// Everything that hangs off a contact, loaded in bulk.
#[derive(Debug, Default)]
pub(crate) struct Children {
    pub emails: Vec<EmailAddress>,
    pub phones: Vec<PhoneNumber>,
    pub addresses: Vec<PostalAddress>,
    pub urls: Vec<ContactUrl>,
    pub groups: Vec<String>,
    pub fields: Vec<Field>,
}

/// `?,?,?` for an `IN (...)` list of `ids.len()` parameters.
fn placeholders(ids: &[i64]) -> String {
    std::iter::repeat_n("?", ids.len()).collect::<Vec<_>>().join(",")
}

/// Load all children of `ids` with one query per table, so listing contacts
/// costs the same six statements whatever the page size.
pub(crate) fn load_children(conn: &Connection, ids: &[i64]) -> Result<HashMap<i64, Children>> {
    let mut out: HashMap<i64, Children> = ids.iter().map(|id| (*id, Children::default())).collect();
    if ids.is_empty() {
        return Ok(out);
    }
    let marks = placeholders(ids);
    let sql = params_from_iter(ids.iter());
    fn push(out: &mut HashMap<i64, Children>, id: i64) -> &mut Children {
        out.entry(id).or_default()
    }

    let mut stmt = conn.prepare(&format!(
        "SELECT contact_id, address, label, is_primary FROM contact_emails
         WHERE contact_id IN ({marks}) ORDER BY contact_id, is_primary DESC, id"
    ))?;
    let rows = stmt.query_map(sql, |r| {
        Ok((r.get::<_, i64>(0)?, EmailAddress {
            address: r.get(1)?,
            label: Label::parse(&r.get::<_, String>(2)?),
            primary: r.get::<_, i64>(3)? != 0,
        }))
    })?;
    for row in rows {
        let (id, email) = row?;
        push(&mut out, id).emails.push(email);
    }

    let mut stmt = conn.prepare(&format!(
        "SELECT contact_id, number, label, is_primary FROM contact_phones
         WHERE contact_id IN ({marks}) ORDER BY contact_id, is_primary DESC, id"
    ))?;
    let rows = stmt.query_map(params_from_iter(ids.iter()), |r| {
        Ok((r.get::<_, i64>(0)?, PhoneNumber {
            number: r.get(1)?,
            label: Label::parse(&r.get::<_, String>(2)?),
            primary: r.get::<_, i64>(3)? != 0,
        }))
    })?;
    for row in rows {
        let (id, phone) = row?;
        push(&mut out, id).phones.push(phone);
    }

    let mut stmt = conn.prepare(&format!(
        "SELECT contact_id, street, city, region, postal_code, country, label FROM contact_addresses
         WHERE contact_id IN ({marks}) ORDER BY contact_id, id"
    ))?;
    let rows = stmt.query_map(params_from_iter(ids.iter()), |r| {
        Ok((r.get::<_, i64>(0)?, PostalAddress {
            street: r.get(1)?,
            city: r.get(2)?,
            region: r.get(3)?,
            postal_code: r.get(4)?,
            country: r.get(5)?,
            label: Label::parse(&r.get::<_, String>(6)?),
        }))
    })?;
    for row in rows {
        let (id, address) = row?;
        push(&mut out, id).addresses.push(address);
    }

    let mut stmt = conn.prepare(&format!(
        "SELECT contact_id, url, label FROM contact_urls WHERE contact_id IN ({marks})
         ORDER BY contact_id, id"
    ))?;
    let rows = stmt.query_map(params_from_iter(ids.iter()), |r| {
        Ok((r.get::<_, i64>(0)?, ContactUrl {
            url: r.get(1)?,
            label: Label::parse(&r.get::<_, String>(2)?),
        }))
    })?;
    for row in rows {
        let (id, url) = row?;
        push(&mut out, id).urls.push(url);
    }

    let mut stmt = conn.prepare(&format!(
        "SELECT m.contact_id, g.name FROM contact_group_members m
         JOIN contact_groups g ON g.id = m.group_id
         WHERE m.contact_id IN ({marks}) ORDER BY m.contact_id, g.name"
    ))?;
    let rows = stmt.query_map(params_from_iter(ids.iter()), |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (id, name) = row?;
        push(&mut out, id).groups.push(name);
    }

    let mut stmt = conn.prepare(&format!(
        "SELECT contact_id, key, value FROM contact_fields WHERE contact_id IN ({marks})
         ORDER BY contact_id, key"
    ))?;
    let rows = stmt.query_map(params_from_iter(ids.iter()), |r| {
        Ok((r.get::<_, i64>(0)?, Field { key: r.get(1)?, value: r.get(2)? }))
    })?;
    for row in rows {
        let (id, field) = row?;
        push(&mut out, id).fields.push(field);
    }
    Ok(out)
}

impl ContactStore {
    /// Add an email to a contact. A duplicate address is a typed error.
    pub fn add_email(&self, contact_id: i64, email: EmailAddress) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        insert_email(&tx, contact_id, &email)?;
        super::write::enforce_one_primary(&tx, contact_id)?;
        tx.commit()?;
        Ok(())
    }

    /// Drop an email from a contact. False if the contact did not have it.
    pub fn remove_email(&self, contact_id: i64, address: &str) -> Result<bool> {
        let address = super::normalize_email(address)?;
        let tx = self.conn.unchecked_transaction()?;
        let changed = tx.execute(
            "DELETE FROM contact_emails WHERE contact_id = ?1 AND address = ?2",
            params![contact_id, address],
        )?;
        super::write::enforce_one_primary(&tx, contact_id)?;
        tx.commit()?;
        Ok(changed > 0)
    }

    pub fn add_phone(&self, contact_id: i64, phone: PhoneNumber) -> Result<()> {
        insert_phone(&self.conn, contact_id, &phone)
    }

    /// Drop a phone number. False if the contact did not have it.
    pub fn remove_phone(&self, contact_id: i64, number: &str) -> Result<bool> {
        self.conn
            .execute(
                "DELETE FROM contact_phones WHERE contact_id = ?1 AND number = ?2",
                params![contact_id, number],
            )
            .map(|n| n > 0)
            .map_err(Into::into)
    }

    pub fn add_address(&self, contact_id: i64, address: PostalAddress) -> Result<()> {
        insert_address(&self.conn, contact_id, &address)
    }

    /// Drop a postal address. False if the contact did not have it.
    pub fn remove_address(&self, contact_id: i64, street: &str) -> Result<bool> {
        self.conn
            .execute(
                "DELETE FROM contact_addresses WHERE contact_id = ?1 AND street = ?2",
                params![contact_id, street],
            )
            .map(|n| n > 0)
            .map_err(Into::into)
    }

    pub fn add_url(&self, contact_id: i64, url: ContactUrl) -> Result<()> {
        insert_url(&self.conn, contact_id, &url)
    }

    /// Drop a URL. False if the contact did not have it.
    pub fn remove_url(&self, contact_id: i64, url: &str) -> Result<bool> {
        self.conn
            .execute(
                "DELETE FROM contact_urls WHERE contact_id = ?1 AND url = ?2",
                params![contact_id, url],
            )
            .map(|n| n > 0)
            .map_err(Into::into)
    }

    /// Store a provider-specific key/value on a contact.
    pub fn add_field(&self, contact_id: i64, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO contact_fields (contact_id, key, value) VALUES (?1, ?2, ?3)",
            params![contact_id, key, value],
        )?;
        Ok(())
    }

    /// Custom fields of a contact, ordered by key.
    pub fn fields(&self, contact_id: i64) -> Result<Vec<Field>> {
        let mut stmt =
            self.conn
                .prepare("SELECT key, value FROM contact_fields WHERE contact_id = ?1 ORDER BY key")?;
        let rows = stmt.query_map(params![contact_id], |r| {
            Ok(Field { key: r.get(0)?, value: r.get(1)? })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
    }
}

pub(crate) fn insert_phone(conn: &Connection, contact_id: i64, phone: &PhoneNumber) -> Result<()> {
    if phone.number.trim().is_empty() {
        return Err(invalid("empty phone number"));
    }
    conn.execute(
        "INSERT INTO contact_phones (contact_id, number, label, is_primary) VALUES (?1, ?2, ?3, ?4)",
        params![contact_id, phone.number, phone.label.as_str(), phone.primary as i64],
    )?;
    Ok(())
}

pub(crate) fn insert_address(conn: &Connection, contact_id: i64, a: &PostalAddress) -> Result<()> {
    if a.street.trim().is_empty() && a.city.trim().is_empty() && a.country.trim().is_empty() {
        return Err(invalid("empty postal address"));
    }
    conn.execute(
        "INSERT INTO contact_addresses (contact_id, street, city, region, postal_code, country, label)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![contact_id, a.street, a.city, a.region, a.postal_code, a.country, a.label.as_str()],
    )?;
    Ok(())
}

pub(crate) fn insert_url(conn: &Connection, contact_id: i64, u: &ContactUrl) -> Result<()> {
    if u.url.trim().is_empty() {
        return Err(invalid("empty url"));
    }
    conn.execute(
        "INSERT INTO contact_urls (contact_id, url, label) VALUES (?1, ?2, ?3)",
        params![contact_id, u.url, u.label.as_str()],
    )?;
    Ok(())
}
