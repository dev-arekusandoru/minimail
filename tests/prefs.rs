//! Scoped settings: SQLite persistence, migration, and typed resolution.

use mail_classifier::contacts::{ContactStore, EmailAddress, NewContact};
use mail_classifier::prefs::{MemoryPrefs, PrefStore, Scope, Setting};

const PAGE: Setting<u8> = Setting::new("test.page", 3);
const NAME: Setting<String> = Setting::new("test.name", String::new());

fn acct() -> Scope {
    Scope::Account("work".into())
}

fn stores() -> Vec<Box<dyn PrefStore>> {
    vec![
        Box::new(ContactStore::open_in_memory().expect("db")),
        Box::new(MemoryPrefs::new()),
    ]
}

#[test]
fn migrating_a_v1_database_keeps_contacts_and_adds_settings() {
    let dir = std::env::temp_dir().join(format!("prefs-mig-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("contacts.db");
    let _ = std::fs::remove_file(&path);
    {
        let store = ContactStore::open(&path).expect("open");
        store.create(NewContact::new("Ada").with(EmailAddress::primary("ada@x.example"))).unwrap();
    }
    {
        // Rewind the file to schema v1.
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch("DROP TABLE settings; PRAGMA user_version = 1;").unwrap();
    }
    let store = ContactStore::open(&path).expect("reopen migrates");
    assert_eq!(store.schema_version().unwrap(), 2);
    assert!(store.get_by_email("ada@x.example").unwrap().is_some(), "contact survives");
    PAGE.set(&store, &Scope::Global, 5);
    assert_eq!(PAGE.get(&store, &Scope::Global), 5);
    drop(store);
    let store = ContactStore::open(&path).unwrap();
    assert_eq!(PAGE.get(&store, &Scope::Global), 5, "persisted across reopen");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn setting_the_default_deletes_the_row() {
    for s in stores() {
        PAGE.set(&*s, &Scope::Global, 5);
        assert!(PAGE.is_modified(&*s, &Scope::Global));
        PAGE.set(&*s, &Scope::Global, 3);
        assert!(!PAGE.is_modified(&*s, &Scope::Global));
        assert_eq!(s.get_raw(&Scope::Global, PAGE.key), None);
        assert_eq!(PAGE.get(&*s, &Scope::Global), 3);
    }
}

#[test]
fn account_scope_overrides_global_and_falls_back() {
    for s in stores() {
        PAGE.set(&*s, &Scope::Global, 5);
        assert_eq!(PAGE.get(&*s, &acct()), 5, "falls back to global");
        assert!(!PAGE.is_modified(&*s, &acct()));
        PAGE.set(&*s, &acct(), 7);
        assert_eq!(PAGE.get(&*s, &acct()), 7);
        assert_eq!(PAGE.get(&*s, &Scope::Global), 5, "global untouched");
        assert_eq!(PAGE.get(&*s, &Scope::Account("other".into())), 5);
        // Overriding back to the built-in default must still shadow global.
        PAGE.set(&*s, &acct(), 3);
        assert_eq!(PAGE.get(&*s, &acct()), 3);
        PAGE.reset(&*s, &acct());
        assert_eq!(PAGE.get(&*s, &acct()), 5, "reset inherits again");
    }
}

#[test]
fn malformed_or_mistyped_json_falls_back() {
    for s in stores() {
        s.set_raw(&Scope::Global, PAGE.key, "{not json");
        assert_eq!(PAGE.get(&*s, &Scope::Global), 3);
        s.set_raw(&Scope::Global, PAGE.key, "\"five\"");
        assert_eq!(PAGE.get(&*s, &Scope::Global), 3);
        // A bad account row falls through to a valid global one.
        PAGE.set(&*s, &Scope::Global, 4);
        s.set_raw(&acct(), PAGE.key, "null");
        assert_eq!(PAGE.get(&*s, &acct()), 4);
    }
}

#[test]
fn clear_removes_every_scope() {
    for s in stores() {
        PAGE.set(&*s, &Scope::Global, 5);
        NAME.set(&*s, &acct(), "Work".to_string());
        s.clear();
        assert!(!PAGE.is_modified(&*s, &Scope::Global));
        assert!(!NAME.is_modified(&*s, &acct()));
        assert_eq!(NAME.get(&*s, &acct()), "");
    }
}
