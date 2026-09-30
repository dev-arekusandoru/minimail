//! Contact store: CRUD, normalization, search, groups, migrations, and seeding.

use std::rc::Rc;

use mail_classifier::clock::FakeClock;
use mail_classifier::contacts::{
    ContactQuery, ContactSource, ContactStore, ContactUrl, EmailAddress, Error, Label,
    NewContact, PhoneNumber, PostalAddress, seed_if_empty,
};
use mail_classifier::model::Mailbox;

const T0: i64 = 1_790_812_800;

fn store() -> ContactStore {
    let mut store = ContactStore::open_in_memory().expect("in-memory db");
    store.set_clock(Rc::new(FakeClock::new(T0)));
    store
}

fn ada() -> NewContact {
    NewContact::new("Ada Nkemelu")
        .with(EmailAddress::primary("Ada@TypeFoundry.example"))
        .organization("Type Foundry")
        .job_title("Art Director")
}

fn names(contacts: &[mail_classifier::contacts::Contact]) -> Vec<String> {
    contacts.iter().map(|c| c.display_name.clone()).collect()
}

#[test]
fn create_read_update_delete() {
    let store = store();
    let new = ada();
    let mut new = new.clone();
    new.given_name = Some("Ada".into());
    new.family_name = Some("Nkemelu".into());
    new.department = Some("Design".into());
    new.birthday = Some("1988-04-12".into());
    new.notes = Some("Prefers email".into());
    new.photo = Some("https://cdn.example/ada.jpg".into());
    new.nickname = Some("Addy".into());
    new.favorite = true;
    new.groups.push("Work".into());

    let created = store.create(new).expect("create");
    assert_eq!(created.display_name, "Ada Nkemelu");
    assert_eq!(created.emails[0].address, "ada@typefoundry.example", "stored lowercased");
    assert!(created.emails[0].primary);
    assert_eq!(created.groups, ["Work"]);
    assert_eq!(created.created_at, T0);
    assert_eq!(created.updated_at, T0);
    assert_eq!(created.last_contacted_at, None);

    let fetched = store.get(created.id).unwrap().expect("get by id");
    assert_eq!(fetched.job_title.as_deref(), Some("Art Director"));
    assert_eq!(fetched.birthday.as_deref(), Some("1988-04-12"));
    assert!(fetched.favorite);
    assert_eq!(fetched.source, ContactSource::Manual);

    let mut patch = NewContact::new("Ada N. Nkemelu");
    patch.job_title = Some("Creative Director".into());
    let updated = store.update(created.id, patch).expect("update");
    assert_eq!(updated.display_name, "Ada N. Nkemelu");
    assert_eq!(updated.job_title.as_deref(), Some("Creative Director"));
    assert_eq!(updated.created_at, T0, "created_at survives an update");
    assert_eq!(updated.emails.len(), 1, "an empty list leaves children alone");
    assert_eq!(updated.groups, ["Work"]);

    assert!(store.delete(created.id).expect("delete"));
    assert!(!store.delete(created.id).expect("delete twice"));
    assert!(store.get(created.id).unwrap().is_none());
}

#[test]
fn emails_are_unique_case_insensitively() {
    let store = store();
    let ada = store.create(ada()).expect("create");
    let err = store.add_email(ada.id, EmailAddress::new("ADA@typefoundry.example")).unwrap_err();
    assert!(matches!(&err, Error::DuplicateEmail(a) if a == "ada@typefoundry.example"), "{err}");

    let other = store.create(NewContact::new("Ines")).expect("create");
    let err = store.add_email(other.id, EmailAddress::new("ada@typefoundry.example")).unwrap_err();
    assert!(matches!(err, Error::DuplicateEmail(_)), "{err}");
    assert_eq!(store.get_by_email("ada@TYPEFOUNDRY.example").unwrap().unwrap().id, ada.id);
    assert!(store.get_by_email("nobody@example.com").unwrap().is_none());
    assert!(matches!(
        store.create(NewContact::new("Broken").with(EmailAddress::new("not-an-email"))),
        Err(Error::Invalid(_))
    ));
}

#[test]
fn one_primary_email_per_contact() {
    let store = store();
    let ada = store.create(ada()).expect("create");
    store.add_email(ada.id, EmailAddress::new("ada@home.example")).expect("add");
    let ada = store.get(ada.id).unwrap().unwrap();
    assert_eq!(ada.emails.len(), 2);
    assert_eq!(
        ada.emails.iter().filter(|e| e.primary).count(),
        1,
        "adding a second address must not create two primaries"
    );

    store
        .add_email(ada.id, EmailAddress { address: "ada@studio.example".into(), label: Label::Work, primary: true })
        .expect("add primary");
    let ada = store.get(ada.id).unwrap().unwrap();
    assert_eq!(ada.primary_email(), Some("ada@studio.example"));
    assert_eq!(ada.emails.iter().filter(|e| e.primary).count(), 1);

    assert!(store.remove_email(ada.id, "ada@studio.example").expect("remove"));
    assert!(!store.remove_email(ada.id, "ada@studio.example").expect("remove twice"));
    let ada = store.get(ada.id).unwrap().unwrap();
    assert_eq!(ada.emails.len(), 2);
    assert_eq!(ada.emails.iter().filter(|e| e.primary).count(), 1, "the survivor is promoted");
}

#[test]
fn delete_cascades_to_children_and_groups() {
    let store = store();
    let ada = store.create(ada()).expect("create");
    let id = ada.id;
    store.add_phone(id, PhoneNumber::new("+44 20 7946 0102")).expect("phone");
    store
        .add_address(
            id,
            PostalAddress {
                street: "18 Harbour Street".into(),
                city: "Bristol".into(),
                region: "Somerset".into(),
                postal_code: "BS1 4RN".into(),
                country: "UK".into(),
                label: Label::Work,
            },
        )
        .expect("address");
    store.add_url(id, ContactUrl { url: "https://typefoundry.example/ada".into(), label: Label::Work }).expect("url");
    store.add_field(id, "pronouns", "they/them").expect("field");
    let group = store.create_group("Work").expect("group");
    store.assign_group(id, group.id).expect("assign");
    assert_eq!(store.fields(id).unwrap().len(), 1);
    assert_eq!(store.get(id).unwrap().unwrap().fields[0].key, "pronouns");

    assert!(store.delete(id).expect("delete"));
    assert_eq!(store.fields(id).unwrap(), []);
    assert!(store.get(id).unwrap().is_none());
    assert_eq!(store.groups().unwrap().len(), 1, "the group itself survives");
    assert_eq!(
        store.search(&ContactQuery::new()).unwrap().len(),
        0,
        "no orphan rows are left behind"
    );
    assert!(store.get_by_email("ada@typefoundry.example").unwrap().is_none());
}

#[test]
fn favorite_and_touch_move_timestamps() {
    let store = store();
    let ada = store.create(ada()).expect("create");
    assert!(!ada.favorite);

    store.set_favorite(ada.id, true).expect("favorite");
    let ada = store.get(ada.id).unwrap().unwrap();
    assert!(ada.favorite);
    assert_eq!(ada.updated_at, T0);
    assert_eq!(ada.last_contacted_at, None, "favoriting is not contacting");

    store.touch(ada.id).expect("touch");
    let ada = store.get(ada.id).unwrap().unwrap();
    assert_eq!(ada.last_contacted_at, Some(T0));

    assert!(matches!(store.touch(9999), Err(Error::NotFound(_))));
    assert!(matches!(store.set_favorite(9999, true), Err(Error::NotFound(_))));
    assert!(matches!(store.set_favorite(9999, true), Err(Error::NotFound(_))));
}

#[test]
fn search_filters_and_paginates() {
    let store = store();
    let ada = store.create(ada().organization("Type Foundry")).expect("create");
    store.create(
        NewContact::new("Dana Whitfield")
            .with(EmailAddress::primary("dana@whitfield.dev"))
            .organization("Whitfield")
            .favorite(true)
            .group("Work"),
    )
    .expect("create");
    store
        .create(
            NewContact::new("Mum")
                .with(EmailAddress::primary("mum@whitfield.home"))
                .group("Family"),
        )
        .expect("create");

    assert_eq!(names(&store.search(&ContactQuery::new()).unwrap()), ["Ada Nkemelu", "Dana Whitfield", "Mum"]);
    assert_eq!(names(&store.search(&ContactQuery::new().text("ada")).unwrap()), ["Ada Nkemelu"]);
    assert_eq!(names(&store.search(&ContactQuery::new().text("dANA@whit")).unwrap()), ["Dana Whitfield"]);
    assert_eq!(names(&store.search(&ContactQuery::new().text("Whitfield")).unwrap()), ["Dana Whitfield"], "organization prefix");
    assert_eq!(names(&store.search(&ContactQuery::new().favorites(true)).unwrap()), ["Dana Whitfield"]);
    assert_eq!(names(&store.search(&ContactQuery::new().group("family")).unwrap()), ["Mum"], "group name is case-insensitive");
    assert!(store.search(&ContactQuery::new().text("a%")).unwrap().is_empty(), "wildcards are literal");

    let page1 = store.search(&ContactQuery::new().page(2, 0)).unwrap();
    let page2 = store.search(&ContactQuery::new().page(2, 2)).unwrap();
    assert_eq!(names(&page1), ["Ada Nkemelu", "Dana Whitfield"]);
    assert_eq!(names(&page2), ["Mum"]);
    assert_eq!(page1[0].id, ada.id);
    assert!(page1[0].emails.iter().any(|e| e.address == "ada@typefoundry.example"), "children load with search results");
    assert_eq!(page1[1].groups, ["Work"]);
}

#[test]
fn groups_are_created_renamed_and_membership_toggled() {
    let store = store();
    let ada = store.create(ada()).expect("create");
    let work = store.create_group("Work").expect("create");
    assert!(matches!(store.create_group("work"), Err(Error::DuplicateGroup(_))));
    assert_eq!(store.groups().unwrap().len(), 1);
    assert_eq!(store.group_id("WORK").unwrap(), Some(work.id));

    assert!(store.assign_group(ada.id, work.id).expect("assign"));
    assert!(!store.assign_group(ada.id, work.id).expect("assign twice"));
    assert!(matches!(store.assign_group(9999, work.id), Err(Error::NotFound(_))));
    assert_eq!(store.get(ada.id).unwrap().unwrap().groups, ["Work"]);

    assert_eq!(store.rename_group(work.id, "Studio").expect("rename").name, "Studio");
    assert_eq!(store.get(ada.id).unwrap().unwrap().groups, ["Studio"], "members keep the group");
    assert!(matches!(store.rename_group(9999, "Ghost"), Err(Error::NotFound(_))));

    assert!(store.remove_group(ada.id, work.id).expect("remove"));
    assert!(!store.remove_group(ada.id, work.id).expect("remove twice"));
    assert!(store.get(ada.id).unwrap().unwrap().groups.is_empty());
    assert!(store.delete_group(work.id).expect("delete group"));
    assert!(!store.delete_group(work.id).expect("delete twice"));
    assert!(matches!(store.create_group("  "), Err(Error::Invalid(_))));
}

#[test]
fn upsert_from_email_creates_minimal_contact() {
    let store = store();
    let (contact, created) = store
        .upsert_from_email("New@Sender.example", Some("New Sender"), ContactSource::Manual)
        .expect("upsert");
    assert!(created);
    assert_eq!(contact.display_name, "New Sender");
    assert_eq!(contact.emails[0].address, "new@sender.example");
    assert_eq!(contact.source, ContactSource::Manual);

    let (again, created) = store
        .upsert_from_email("new@sender.example", Some("Renamed"), ContactSource::Manual)
        .expect("upsert again");
    assert!(!created, "an existing address is never re-created");
    assert_eq!(again.id, contact.id);
    assert_eq!(again.display_name, "New Sender");

    let (nameless, created) = store
        .upsert_from_email("bare@sender.example", None, ContactSource::Manual)
        .expect("upsert without a name");
    assert!(created);
    assert_eq!(nameless.display_name, "bare", "falls back to the local part");
}

#[test]
fn forget_address_removes_a_manual_contact_address() {
    let store = store();
    let seeded = store.create(NewContact::from_email("ada@typefoundry.example", "Ada").source(ContactSource::Seed)).expect("create");
    let contact = store
        .upsert_from_email("new@sender.example", Some("New"), ContactSource::Manual)
        .expect("upsert")
        .0;

    assert!(store.forget_address("new@sender.example").expect("forget"));
    assert!(!store.is_known("new@sender.example").expect("known"));
    assert!(
        store.get(contact.id).unwrap().unwrap().emails.is_empty(),
        "manual contacts remain after an address is forgotten"
    );

    assert!(store.forget_address("ada@typefoundry.example").expect("forget"));
    assert!(store.get(seeded.id).unwrap().is_some(), "an existing contact only loses the address");
    assert!(store.get(seeded.id).unwrap().unwrap().emails.is_empty());
    assert!(!store.forget_address("gone@example.com").expect("forget twice"));
}

/// A private database file, removed when the test ends.
struct TempDb(std::path::PathBuf);

impl TempDb {
    fn new(name: &str) -> Self {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let path = std::env::temp_dir().join(format!("mail-classifier-{name}-{unique}.db"));
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
        Self(path)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.0.display()));
        }
    }
}

#[test]
fn a_fresh_file_is_migrated_and_keeps_its_data_across_reopen() {
    let db = TempDb::new("contacts");

    let store = ContactStore::open(db.path()).expect("open a new file");
    assert_eq!(store.schema_version().expect("version"), 1, "every migration is applied");
    let ada = store.create(ada()).expect("create");
    drop(store);

    let store = ContactStore::open(db.path()).expect("reopen");
    assert_eq!(store.schema_version().expect("version after reopen"), 1, "reopening applies nothing");
    assert_eq!(store.journal_mode().expect("journal mode").to_lowercase(), "wal");
    assert!(store.foreign_keys_enabled().expect("foreign keys"));
    assert_eq!(store.get(ada.id).unwrap().unwrap().display_name, "Ada Nkemelu");
    assert!(store.is_known("ada@typefoundry.example").expect("known"));
}

#[test]
fn seeding_is_idempotent() {
    let db = TempDb::new("seed");
    let store = ContactStore::open(db.path()).expect("open");

    let added = seed_if_empty(&store).expect("seed");
    assert!(added > 20, "the fixture seeds a full address book, got {added}");
    let seeded = store.known_addresses().expect("addresses");
    assert!(seeded.contains("dana@whitfield.dev"));
    assert!(seeded.contains("mum@whitfield.home"));

    assert_eq!(seed_if_empty(&store).expect("seed again"), 0);
    assert_eq!(store.known_addresses().expect("addresses").len(), seeded.len());

    let dana = store.get_by_email("dana@whitfield.dev").expect("lookup").expect("seeded contact");
    assert_eq!(dana.display_name, "Dana Whitfield");
    assert_eq!(dana.organization.as_deref(), Some("Whitfield"));
    assert_eq!(dana.department.as_deref(), Some("Product"));
    assert_eq!(dana.birthday.as_deref(), Some("1988-04-12"));
    assert_eq!(dana.emails[0].label, Label::Work);
    assert!(dana.favorite);
    assert_eq!(dana.groups, ["Work"]);
    assert_eq!(store.search(&ContactQuery::new().group("Family")).unwrap().len(), 1);
    assert_eq!(store.search(&ContactQuery::new().favorites(true)).unwrap().len(), 4);

    drop(store);
    let reopened = ContactStore::open(db.path()).expect("reopen");
    assert_eq!(seed_if_empty(&reopened).expect("seed after reopen"), 0);
    assert_eq!(reopened.known_addresses().expect("addresses").len(), seeded.len());
}

#[test]
fn allowing_a_new_sender_is_persisted_and_undo_removes_it() {
    let store = Rc::new(ContactStore::open_in_memory().expect("in-memory"));
    let messages = serde_json::json!([
        {"id": 1, "thread_id": 1, "from_name": "Known Sender", "from_email": "known@a.test",
         "to": "you@example.com", "subject": "s1", "body": "b", "received": "2026-09-01T00:00:00Z"},
        {"id": 2, "thread_id": 2, "from_name": "New Sender", "from_email": "New@a.test",
         "to": "you@example.com", "subject": "s2", "body": "b", "received": "2026-09-02T00:00:00Z"},
    ]);
    store.create(NewContact::from_email("known@a.test", "Known Sender")).expect("seed one contact");

    let json = serde_json::to_string(&messages).unwrap();
    let mut mb = Mailbox::from_json_with_contacts(&json, store.clone()).expect("mailbox");
    assert!(mb.is_new_sender(2));

    assert!(mb.allow_sender("new@a.test"));
    assert!(!mb.allow_sender("new@a.test"), "already known");
    assert!(!mb.is_new_sender(2));
    assert!(store.is_known("new@a.test").expect("known"), "the decision is in the address book");
    let contact = store.get_by_email("new@a.test").unwrap().expect("contact");
    assert_eq!(contact.display_name, "New Sender", "the sender name carries over");

    let restarted = Mailbox::from_json_with_contacts(&json, store.clone()).expect("mailbox");
    assert!(!restarted.is_new_sender(2));
    assert_eq!(restarted.ids_in(mail_classifier::model::TriageState::Inbox), vec![2, 1]);

    assert!(mb.undo());
    assert!(mb.is_new_sender(2), "undo restores the new sender classification");
    assert!(!store.is_known("new@a.test").expect("known"), "undo also rewinds the address book");
    assert_eq!(store.search(&ContactQuery::new()).expect("all").len(), 1);
}
