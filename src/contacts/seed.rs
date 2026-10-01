//! The shipped address book: an embedded fixture that seeds an empty
//! database, and where the on-disk database lives.

use std::path::PathBuf;

use serde::Deserialize;

use super::error::{Error, Result};
use super::model::{
    ContactSource, ContactUrl, EmailAddress, Label, NewContact, PhoneNumber, PostalAddress,
};
use super::ContactStore;

/// Seed fixture, embedded at build time.
pub const SEED_JSON: &str = include_str!("../../fixtures/contacts_seed.json");

/// Bumped when the fixture changes, so an existing database is not re-seeded
/// over contacts the user has since edited.
const SEED_VERSION: u32 = 1;

/// `$MAIL_CLASSIFIER_DB`, else `~/Library/Application Support/mail-classifier/contacts.db`.
pub fn default_db_path() -> PathBuf {
    if let Ok(path) = std::env::var("MAIL_CLASSIFIER_DB")
        && !path.trim().is_empty()
    {
        return PathBuf::from(path);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    home.join("Library/Application Support/mail-classifier/contacts.db")
}

/// Open the database the app uses, creating the directory. Starts empty: the
/// shipped contacts are fixtures for tests only.
pub fn open_default() -> Result<ContactStore> {
    let path = default_db_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| Error::Invalid(format!("cannot create {}: {e}", dir.display())))?;
    }
    ContactStore::open(&path)
}

/// A seeded in-memory address book from the shipped fixture, for tests.
pub fn open_seeded_in_memory() -> Result<ContactStore> {
    let store = ContactStore::open_in_memory()?;
    seed_if_empty(&store)?;
    Ok(store)
}

/// Insert the shipped contacts when this database has never been seeded.
/// Idempotent: re-running on a seeded database is a no-op.
pub fn seed_if_empty(store: &ContactStore) -> Result<usize> {
    let key = format!("seed_version_{SEED_VERSION}");
    let seeded = store
        .conn()
        .query_row(
            "SELECT 1 FROM store_meta WHERE key = ?1",
            [&key],
            |r| r.get::<_, i64>(0),
        )
        .is_ok();
    if seeded {
        return Ok(0);
    }
    let seed: Seed = serde_json::from_str(SEED_JSON)
        .map_err(|e| Error::Invalid(format!("seed fixture: {e}")))?;
    if seed.version != SEED_VERSION {
        return Err(Error::Invalid(format!(
            "seed fixture is version {}, this build wants {SEED_VERSION}",
            seed.version
        )));
    }
    let mut added = 0;
    for contact in &seed.contacts {
        // A half-applied seed run leaves duplicates; skipping them keeps this
        // idempotent without a rollback of the whole file.
        match store.create(contact.to_new()) {
            Ok(_) => added += 1,
            Err(Error::DuplicateEmail(_)) => continue,
            Err(e) => return Err(e),
        }
    }
    store.conn().execute(
        "INSERT OR REPLACE INTO store_meta (key, value) VALUES (?1, ?2)",
        rusqlite::params![key, SEED_VERSION.to_string()],
    )?;
    Ok(added)
}

#[derive(Deserialize)]
struct Seed {
    version: u32,
    contacts: Vec<SeedContact>,
}

#[derive(Deserialize)]
struct SeedContact {
    #[serde(default)]
    prefix: Option<String>,
    #[serde(default)]
    given_name: Option<String>,
    #[serde(default)]
    middle_name: Option<String>,
    #[serde(default)]
    family_name: Option<String>,
    #[serde(default)]
    suffix: Option<String>,
    #[serde(default)]
    nickname: Option<String>,
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    organization: Option<String>,
    #[serde(default)]
    department: Option<String>,
    #[serde(default)]
    job_title: Option<String>,
    #[serde(default)]
    birthday: Option<String>,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    photo: Option<String>,
    #[serde(default)]
    favorite: bool,
    #[serde(default)]
    emails: Vec<SeedEmail>,
    #[serde(default)]
    phones: Vec<SeedPhone>,
    #[serde(default)]
    addresses: Vec<SeedAddress>,
    #[serde(default)]
    urls: Vec<SeedUrl>,
    #[serde(default)]
    groups: Vec<String>,
}

#[derive(Deserialize)]
struct SeedEmail {
    address: String,
    #[serde(default)]
    label: String,
    #[serde(default)]
    primary: bool,
}

#[derive(Deserialize)]
struct SeedPhone {
    number: String,
    #[serde(default)]
    label: String,
    #[serde(default)]
    primary: bool,
}

#[derive(Deserialize)]
struct SeedAddress {
    #[serde(default)]
    street: String,
    #[serde(default)]
    city: String,
    #[serde(default)]
    region: String,
    #[serde(default)]
    postal_code: String,
    #[serde(default)]
    country: String,
    #[serde(default)]
    label: String,
}

#[derive(Deserialize)]
struct SeedUrl {
    url: String,
    #[serde(default)]
    label: String,
}

impl SeedContact {
    fn to_new(&self) -> NewContact {
        NewContact {
            prefix: self.prefix.clone(),
            given_name: self.given_name.clone(),
            middle_name: self.middle_name.clone(),
            family_name: self.family_name.clone(),
            suffix: self.suffix.clone(),
            nickname: self.nickname.clone(),
            display_name: self.display_name.clone(),
            organization: self.organization.clone(),
            department: self.department.clone(),
            job_title: self.job_title.clone(),
            birthday: self.birthday.clone(),
            notes: self.notes.clone(),
            photo: self.photo.clone(),
            favorite: self.favorite,
            source: ContactSource::Seed,
            emails: self
                .emails
                .iter()
                .map(|e| EmailAddress {
                    address: e.address.clone(),
                    label: Label::parse(&e.label),
                    primary: e.primary,
                })
                .collect(),
            phones: self
                .phones
                .iter()
                .map(|p| PhoneNumber {
                    number: p.number.clone(),
                    label: Label::parse(&p.label),
                    primary: p.primary,
                })
                .collect(),
            addresses: self
                .addresses
                .iter()
                .map(|a| PostalAddress {
                    street: a.street.clone(),
                    city: a.city.clone(),
                    region: a.region.clone(),
                    postal_code: a.postal_code.clone(),
                    country: a.country.clone(),
                    label: Label::parse(&a.label),
                })
                .collect(),
            urls: self
                .urls
                .iter()
                .map(|u| ContactUrl { url: u.url.clone(), label: Label::parse(&u.label) })
                .collect(),
            groups: self.groups.clone(),
        }
    }
}
