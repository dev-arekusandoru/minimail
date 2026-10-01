//! Sync engine against an in-memory cache and an in-file provider.

use std::collections::HashSet;
use std::rc::Rc;

use mail_classifier::clock::{Clock, FakeClock, Timestamp};
use mail_classifier::contacts;
use mail_classifier::model::{
    Account, Location, Mailbox, Message, MessageId, ProviderKind, TriageState, View,
};
use mail_classifier::provider::{
    Changes, MailProvider, ProviderError, RemoteFolder, RemoteId, RemoteMessage, RemoteState,
};
use mail_classifier::sync::cache::Cache;
use mail_classifier::sync::{
    Pull, apply_fetched, apply_moves, apply_reads, background_pull, pending_moves, pending_reads,
    persist_local, run_moves, run_reads,
};

const ACCOUNT: &str = "gmail:a@x.io";

// ------------------------------------------------------------------ provider

/// Records every call, so the tests can assert on what sync pushed.
struct FakeProvider {
    messages: Vec<RemoteMessage>,
    folders: Vec<RemoteFolder>,
    cursor: String,
    next_label: usize,
    moves: Vec<(RemoteId, RemoteState, RemoteState)>,
    created: Vec<String>,
    fail_move: Option<ProviderError>,
    expire_cursor: bool,
    reads: Vec<(RemoteId, bool)>,
}

impl FakeProvider {
    fn new(messages: Vec<RemoteMessage>) -> Self {
        Self {
            messages,
            folders: Vec::new(),
            cursor: "1".to_owned(),
            next_label: 0,
            moves: Vec::new(),
            created: Vec::new(),
            fail_move: None,
            expire_cursor: false,
            reads: Vec::new(),
        }
    }

    fn with(mut self, folders: Vec<RemoteFolder>) -> Self {
        self.folders = folders;
        self
    }

    fn find(&self, id: &str) -> Option<&RemoteMessage> {
        self.messages.iter().find(|m| m.id == id)
    }

    /// Change a remote message's state, as the provider would see it.
    fn remote_set(&mut self, id: &str, state: RemoteState) {
        let Some(m) = self.messages.iter_mut().find(|m| m.id == id) else {
            panic!("no such remote message {id}");
        };
        m.state = state;
    }
}

impl MailProvider for FakeProvider {
    fn folders(&mut self) -> Result<Vec<RemoteFolder>, ProviderError> {
        Ok(self.folders.clone())
    }

    fn create_folder(&mut self, path: &str) -> Result<RemoteFolder, ProviderError> {
        self.next_label += 1;
        let id = format!("Label_{}", self.next_label);
        self.created.push(path.to_owned());
        let folder = RemoteFolder {
            id: id.clone(),
            path: path.to_owned(),
        };
        self.folders.push(folder.clone());
        Ok(folder)
    }

    fn recent(&mut self, limit: usize) -> Result<(Vec<RemoteId>, String), ProviderError> {
        let ids = self
            .messages
            .iter()
            .rev()
            .take(limit)
            .map(|m| m.id.clone())
            .collect();
        Ok((ids, self.cursor.clone()))
    }

    fn changes(&mut self, _cursor: &str) -> Result<Changes, ProviderError> {
        if self.expire_cursor {
            return Err(ProviderError::CursorExpired);
        }
        Ok(Changes {
            changed: self.messages.iter().map(|m| m.id.clone()).collect(),
            removed: Vec::new(),
            cursor: self.cursor.clone(),
        })
    }

    fn fetch(&mut self, ids: &[RemoteId]) -> Result<Vec<RemoteMessage>, ProviderError> {
        Ok(ids
            .iter()
            .filter_map(|id| self.find(id).cloned())
            .collect())
    }

    fn move_message(
        &mut self,
        id: &RemoteId,
        from: &RemoteState,
        to: &RemoteState,
    ) -> Result<(), ProviderError> {
        if let Some(e) = &self.fail_move {
            return Err(clone_error(e));
        }
        self.moves.push((id.clone(), from.clone(), to.clone()));
        if let Some(m) = self.messages.iter_mut().find(|m| &m.id == id) {
            m.state = to.clone();
        }
        Ok(())
    }

    fn set_read(&mut self, id: &RemoteId, read: bool) -> Result<(), ProviderError> {
        self.reads.push((id.clone(), read));
        if let Some(m) = self.messages.iter_mut().find(|m| &m.id == id) {
            m.unread = !read;
        }
        Ok(())
    }
}

fn clone_error(e: &ProviderError) -> ProviderError {
    match e {
        ProviderError::Auth(m) => ProviderError::Auth(m.clone()),
        ProviderError::Network(m) => ProviderError::Network(m.clone()),
        ProviderError::CursorExpired => ProviderError::CursorExpired,
        ProviderError::RateLimited => ProviderError::RateLimited,
        ProviderError::Api { status, message } => ProviderError::Api {
            status: *status,
            message: message.clone(),
        },
    }
}

// -------------------------------------------------------------------- setup

fn account() -> Account {
    Account {
        id: ACCOUNT.to_owned(),
        name: "a@x.io".to_owned(),
        email: "a@x.io".to_owned(),
        color: "#61afef".to_owned(),
        provider: ProviderKind::Gmail,
    }
}

fn local(id: MessageId, account: &str) -> Message {
    Message {
        id,

        thread_id: id,
        from_name: "Local".to_owned(),
        from_email: "local@x.io".to_owned(),
        to: "a@x.io".to_owned(),
        subject: "Local only".to_owned(),
        body: "body".to_owned(),
        received: "2026-01-01T00:00:00Z".to_owned(),
        state: TriageState::Inbox,
        account: account.to_owned(),
        outgoing: false,
        snooze: None,
        cc: String::new(),
        bcc: String::new(),
        html: None,
        attachments: Vec::new(),
        read: false,
        partial: false,
    }
}

fn remote(id: &str, thread: &str, received: Timestamp) -> RemoteMessage {
    RemoteMessage {
        id: id.to_owned(),
        thread: thread.to_owned(),
        from_name: "Ann Lee".to_owned(),
        from_email: "ann@x.io".to_owned(),
        to: "a@x.io".to_owned(),
        cc: String::new(),
        bcc: String::new(),
        subject: format!("Remote {id}"),
        body: "hello".to_owned(),
        html: None,
        received,
        attachments: Vec::new(),
        state: RemoteState::Inbox,
        outgoing: false,
        unread: true,
    }
}

fn mailbox(messages: Vec<Message>) -> Mailbox {
    Mailbox::from_parts(
        messages,
        vec![account()],
        Vec::new(),
        Rc::new(contacts::open_seeded_in_memory().unwrap()),
    )
}

fn inbox(mb: &Mailbox) -> Vec<MessageId> {
    mb.ids_in_view(&View {
        location: Location::Inbox(ACCOUNT.to_owned()),
        ..View::default()
    })
}

/// Pull from the provider into the mailbox, the way the app does: without a
/// cursor, cached messages are skipped (the initial import resumes).
fn pull_all(mb: &mut Mailbox, cache: &Cache, p: &mut FakeProvider, cursor: Option<String>) {
    let known = match cursor {
        None => cache.remote_ids(ACCOUNT).unwrap(),
        Some(_) => HashSet::new(),
    };
    let pull = background_pull(p, cursor, &known, &[]).expect("pull");
    apply_fetched(mb, cache, ACCOUNT, &pull).expect("apply");
}

fn remote_id_of(cache: &Cache, id: MessageId) -> RemoteId {
    cache
        .remote_state(id)
        .unwrap()
        .expect("cached")
        .1
}

/// Push every pending move and record the confirmation.
fn flush(mb: &Mailbox, cache: &Cache, p: &mut FakeProvider) {
    let moves = pending_moves(mb, cache, ACCOUNT);
    let results = run_moves(p, moves);
    let error = apply_moves(mb, cache, &results);
    assert_eq!(error, None, "apply_moves failed");
}

// --------------------------------------------------------------------- read

/// The local id of remote message `rid`.
fn local_id(cache: &Cache, rid: &str) -> MessageId {
    cache.message_by_remote(ACCOUNT, rid).unwrap().expect("cached")
}

#[test]
fn server_read_state_is_imported_and_follows_server_changes() {
    let cache = Cache::open_in_memory().unwrap();
    let mut mb = mailbox(Vec::new());
    let mut seen = remote("m1", "t1", 1_790_000_000);
    seen.unread = false;
    let mut p = FakeProvider::new(vec![seen, remote("m2", "t2", 1_790_000_100)]);
    cache.upsert_account(&account()).unwrap();

    pull_all(&mut mb, &cache, &mut p, None);
    let (m1, m2) = (local_id(&cache, "m1"), local_id(&cache, "m2"));
    assert!(mb.is_read(m1) && !mb.is_read(m2));

    // Read m2 and mark m1 unread in Gmail web.
    p.messages[1].unread = false;
    p.messages[0].unread = true;
    pull_all(&mut mb, &cache, &mut p, Some("1".to_owned()));
    assert!(!mb.is_read(m1) && mb.is_read(m2));
    assert!(pending_reads(&mb, &cache, ACCOUNT).is_empty(), "server changes are not echoed back");
}

#[test]
fn reading_locally_is_pushed_once_and_survives_a_pull_before_the_push() {
    let cache = Cache::open_in_memory().unwrap();
    let mut mb = mailbox(Vec::new());
    let mut p = FakeProvider::new(vec![remote("m1", "t1", 1_790_000_000)]);
    cache.upsert_account(&account()).unwrap();
    pull_all(&mut mb, &cache, &mut p, None);
    let m1 = local_id(&cache, "m1");

    mb.mark_read(m1);
    // A pull lands before the push: the server still says unread, the local read wins.
    pull_all(&mut mb, &cache, &mut p, Some("1".to_owned()));
    assert!(mb.is_read(m1));

    let results = run_reads(&mut p, pending_reads(&mb, &cache, ACCOUNT));
    assert_eq!(apply_reads(&cache, &results), None);
    assert_eq!(p.reads, [("m1".to_owned(), true)]);
    assert!(pending_reads(&mb, &cache, ACCOUNT).is_empty());
}

#[test]
fn a_cache_from_before_read_sync_is_migrated_and_backfilled_from_the_server() {
    // A cache file in the old schema (no remote_read column) holding one message.
    let path = std::env::temp_dir().join(format!("mc-read-migration-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let mut old = local(1, ACCOUNT);
    old.read = false;
    {
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch(
            "CREATE TABLE messages(id INTEGER PRIMARY KEY, account TEXT NOT NULL,
                remote_id TEXT NOT NULL, json TEXT NOT NULL, remote_state TEXT NOT NULL,
                UNIQUE(account, remote_id));",
        )
        .unwrap();
        db.execute(
            "INSERT INTO messages VALUES(1, ?1, 'm1', ?2, '\"Inbox\"')",
            (ACCOUNT, serde_json::to_string(&old).unwrap()),
        )
        .unwrap();
    }
    let cache = Cache::open(&path).unwrap();
    cache.upsert_account(&account()).unwrap();
    let (_, _, messages) = cache.load().unwrap();
    let mut mb = mailbox(messages);
    assert!(pending_reads(&mb, &cache, ACCOUNT).is_empty(), "unknown rows are never pushed");

    let refresh = cache.unknown_read(ACCOUNT, 25).unwrap();
    assert_eq!(refresh, ["m1"]);
    let mut read_in_gmail = remote("m1", "t1", 1_790_000_000);
    read_in_gmail.unread = false;
    let mut p = FakeProvider::new(vec![read_in_gmail]);
    let pull = background_pull(&mut p, Some("1".to_owned()), &HashSet::new(), &refresh).unwrap();
    apply_fetched(&mut mb, &cache, ACCOUNT, &pull).unwrap();

    assert!(mb.is_read(1), "the server's read state replaces the unknown one");
    assert!(cache.unknown_read(ACCOUNT, 25).unwrap().is_empty());
    drop(cache);
    let _ = std::fs::remove_file(&path);
}

// --------------------------------------------------------------------- pull

#[test]
fn pull_imports_mail_and_avoids_id_collisions() {
    let cache = Cache::open_in_memory().unwrap();
    // A local message with a high id: pulled mail must not reuse it.
    let mut mb = mailbox(vec![local(7, "personal")]);
    let mut p = FakeProvider::new(vec![
        remote("m1", "t1", 1_790_000_000),
        remote("m2", "t2", 1_790_000_100),
    ]);
    cache.upsert_account(&account()).unwrap();

    pull_all(&mut mb, &cache, &mut p, None);

    let ids = inbox(&mb);
    assert_eq!(ids.len(), 2, "both pulled messages land in the gmail inbox");
    assert!(ids.iter().all(|id| *id != 7), "local id 7 is untouched");
    assert_eq!(
        mb.get(7).map(|m| m.account.clone()),
        Some("personal".to_owned())
    );
    assert_eq!(
        mb.messages().iter().filter(|m| m.account == ACCOUNT).count(),
        2
    );
    assert_eq!(
        cache.cursor(ACCOUNT).unwrap().as_deref(),
        Some("1"),
        "cursor is stored"
    );
}

#[test]
fn pull_is_idempotent() {
    let cache = Cache::open_in_memory().unwrap();
    let mut mb = mailbox(Vec::new());
    let mut p = FakeProvider::new(vec![remote("m1", "t1", 1_790_000_000)]);
    cache.upsert_account(&account()).unwrap();

    pull_all(&mut mb, &cache, &mut p, None);
    pull_all(&mut mb, &cache, &mut p, Some("1".to_owned()));

    assert_eq!(inbox(&mb).len(), 1);
    assert_eq!(mb.messages().len(), 1);
}

// --------------------------------------------------------------------- moves

#[test]
fn archive_then_undo_round_trips_through_the_provider() {
    let cache = Cache::open_in_memory().unwrap();
    let mut mb = mailbox(Vec::new());
    let mut p = FakeProvider::new(vec![remote("m1", "t1", 1_790_000_000)]);
    cache.upsert_account(&account()).unwrap();
    pull_all(&mut mb, &cache, &mut p, None);
    let id = inbox(&mb)[0];

    mb.set_state(&[id], TriageState::Archived);
    let moves = pending_moves(&mb, &cache, ACCOUNT);
    assert_eq!(moves.len(), 1);
    assert_eq!(moves[0].from, RemoteState::Inbox);
    assert_eq!(moves[0].to, RemoteState::Archived);

    flush(&mb, &cache, &mut p);
    assert_eq!(p.moves.len(), 1);
    assert_eq!(
        cache.remote_state(id).unwrap().unwrap().2,
        RemoteState::Archived
    );
    assert!(
        pending_moves(&mb, &cache, ACCOUNT).is_empty(),
        "confirmed moves are not pending again"
    );

    assert!(mb.undo());
    let moves = pending_moves(&mb, &cache, ACCOUNT);
    assert_eq!(moves.len(), 1, "undo leaves an unconfirmed diff");
    assert_eq!(moves[0].from, RemoteState::Archived);
    assert_eq!(moves[0].to, RemoteState::Inbox);

    flush(&mb, &cache, &mut p);
    assert_eq!(p.moves[1].1, RemoteState::Archived);
    assert_eq!(p.moves[1].2, RemoteState::Inbox);
    assert!(pending_moves(&mb, &cache, ACCOUNT).is_empty());
}

#[test]
fn locally_created_mail_is_never_pushed() {
    let cache = Cache::open_in_memory().unwrap();
    let mb = mailbox(vec![local(3, ACCOUNT)]);
    let mut p = FakeProvider::new(Vec::new());
    cache.upsert_account(&account()).unwrap();

    assert!(
        pending_moves(&mb, &cache, ACCOUNT).is_empty(),
        "a message the provider never saw has nothing to push"
    );
    flush(&mb, &cache, &mut p);
    assert!(p.moves.is_empty());
}

#[test]
fn a_network_failure_stops_the_batch_and_keeps_the_rest_pending() {
    let cache = Cache::open_in_memory().unwrap();
    let mut mb = mailbox(Vec::new());
    let mut p = FakeProvider::new(vec![
        remote("m1", "t1", 1_790_000_000),
        remote("m2", "t2", 1_790_000_100),
    ]);
    cache.upsert_account(&account()).unwrap();
    pull_all(&mut mb, &cache, &mut p, None);
    let ids = inbox(&mb);
    mb.set_state(&ids, TriageState::Archived);

    p.fail_move = Some(ProviderError::Network("offline".to_owned()));
    let results = run_moves(&mut p, pending_moves(&mb, &cache, ACCOUNT));
    assert_eq!(results.len(), 1, "the batch stops at the first failure");
    let error = apply_moves(&mb, &cache, &results);
    assert!(error.unwrap().contains("offline"));
    assert_eq!(pending_moves(&mb, &cache, ACCOUNT).len(), 2);

    p.fail_move = None;
    flush(&mb, &cache, &mut p);
    assert_eq!(p.moves.len(), 2);
    assert!(pending_moves(&mb, &cache, ACCOUNT).is_empty());
}

// ------------------------------------------------------------------- snoozes

#[test]
fn snooze_archives_remotely_and_survives_a_pull_until_it_wakes() {
    let cache = Cache::open_in_memory().unwrap();
    let mut mb = mailbox(Vec::new());
    let mut p = FakeProvider::new(vec![remote("m1", "t1", 1_790_000_000)]);
    cache.upsert_account(&account()).unwrap();
    pull_all(&mut mb, &cache, &mut p, None);
    let id = inbox(&mb)[0];

    let clock = FakeClock::new(1_790_000_000);
    let until = clock.now() + 3_600;
    mb.snooze(&[id], until, clock.now());

    let moves = pending_moves(&mb, &cache, ACCOUNT);
    assert_eq!(moves.len(), 1);
    assert_eq!(moves[0].to, RemoteState::Archived, "no snooze exists remotely");
    flush(&mb, &cache, &mut p);

    // A pull reporting Archived must not wake the message early.
    pull_all(&mut mb, &cache, &mut p, Some("1".to_owned()));
    assert_eq!(mb.state_of(id), Some(TriageState::Snoozed));
    assert_eq!(mb.snoozed_until(id), Some(until));

    // Waking locally is what produces the move back to the inbox.
    clock.advance(4_000);
    mb.tick(clock.now());
    assert_eq!(mb.state_of(id), Some(TriageState::Inbox));
    let moves = pending_moves(&mb, &cache, ACCOUNT);
    assert_eq!(moves.len(), 1);
    assert_eq!(moves[0].from, RemoteState::Archived);
    assert_eq!(moves[0].to, RemoteState::Inbox);
}

// ------------------------------------------------------------ remote changes

#[test]
fn a_remote_trash_takes_effect_unless_a_local_change_is_pending() {
    let cache = Cache::open_in_memory().unwrap();
    let mut mb = mailbox(Vec::new());
    let mut p = FakeProvider::new(vec![
        remote("m1", "t1", 1_790_000_000),
        remote("m2", "t2", 1_790_000_100),
    ]);
    cache.upsert_account(&account()).unwrap();
    pull_all(&mut mb, &cache, &mut p, None);
    let ids = inbox(&mb);
    let (quiet, busy) = (ids[0], ids[1]);

    // One message has an unconfirmed local archive; the other does not.
    mb.set_state(&[busy], TriageState::Archived);

    p.remote_set("m1", RemoteState::Trash);
    p.remote_set("m2", RemoteState::Trash);
    pull_all(&mut mb, &cache, &mut p, Some("1".to_owned()));

    assert_eq!(mb.state_of(quiet), Some(TriageState::Deleted));
    assert_eq!(
        mb.state_of(busy),
        Some(TriageState::Archived),
        "the pending local change is not clobbered"
    );
}

#[test]
fn waking_a_snooze_outranks_a_stale_remote_archive() {
    let cache = Cache::open_in_memory().unwrap();
    let mut mb = mailbox(Vec::new());
    let mut p = FakeProvider::new(vec![remote("m1", "t1", 1_790_000_000)]);
    cache.upsert_account(&account()).unwrap();
    pull_all(&mut mb, &cache, &mut p, None);
    let id = inbox(&mb)[0];

    let clock = FakeClock::new(1_790_000_000);
    mb.snooze(&[id], clock.now() + 60, clock.now());
    flush(&mb, &cache, &mut p);

    clock.advance(120);
    mb.tick(clock.now());
    // The provider archived it (there is no snooze to push) and still reports
    // Archived; the local wake is an unconfirmed change and must survive it.
    pull_all(&mut mb, &cache, &mut p, Some("1".to_owned()));
    assert_eq!(mb.state_of(id), Some(TriageState::Inbox));
    assert_eq!(mb.snoozed_until(id), None);
    let moves = pending_moves(&mb, &cache, ACCOUNT);
    assert_eq!(moves.len(), 1, "the wake is pushed back to the inbox");
    assert_eq!(moves[0].to, RemoteState::Inbox);
}

#[test]
fn a_removed_id_disappears_locally() {
    let cache = Cache::open_in_memory().unwrap();
    let mut mb = mailbox(Vec::new());
    let mut p = FakeProvider::new(vec![
        remote("m1", "t1", 1_790_000_000),
        remote("m2", "t2", 1_790_000_100),
    ]);
    cache.upsert_account(&account()).unwrap();
    pull_all(&mut mb, &cache, &mut p, None);
    let ids = inbox(&mb);
    let kept = ids[0];
    let gone = remote_id_of(&cache, ids[1]);

    let pull = Pull {
        folders: p.folders().unwrap(),
        fetched: Vec::new(),
        removed: vec![gone.clone()],
        cursor: Some("2".to_owned()),
    };
    apply_fetched(&mut mb, &cache, ACCOUNT, &pull).expect("apply");

    assert_eq!(mb.messages().len(), 1);
    assert!(mb.get(ids[1]).is_none());
    assert!(inbox(&mb).contains(&kept));
    assert_eq!(cache.message_by_remote(ACCOUNT, &gone).unwrap(), None);
    assert_eq!(cache.cursor(ACCOUNT).unwrap().as_deref(), Some("2"));
}

// ------------------------------------------------------------------ folders

#[test]
fn filing_creates_the_remote_label_once_per_batch() {
    let cache = Cache::open_in_memory().unwrap();
    let mut mb = mailbox(Vec::new());
    let mut p = FakeProvider::new(vec![
        remote("m1", "t1", 1_790_000_000),
        remote("m2", "t2", 1_790_000_100),
    ]);
    cache.upsert_account(&account()).unwrap();
    pull_all(&mut mb, &cache, &mut p, None);
    let ids = inbox(&mb);

    let parent = mb.create_folder(ACCOUNT, "Parent", None);
    cache
        .upsert_folder(mb.folder(parent).unwrap(), None)
        .unwrap();
    mb.set_state(&ids, TriageState::Filed(parent));
    persist_local(&mb, &cache);

    let results = run_moves(&mut p, pending_moves(&mb, &cache, ACCOUNT));
    assert_eq!(p.created, vec!["Parent".to_owned()], "one label per batch");
    assert_eq!(p.moves.len(), 2);
    assert!(
        p.moves.iter().all(|(_, _, to)| *to == RemoteState::Folder("Label_1".to_owned())),
        "both moves use the created label"
    );
    assert_eq!(apply_moves(&mb, &cache, &results), None);

    assert_eq!(
        cache.folder_remote_id(parent).unwrap().as_deref(),
        Some("Label_1"),
        "the label is remembered, so later moves skip creation"
    );
    assert!(pending_moves(&mb, &cache, ACCOUNT).is_empty());
}

#[test]
fn a_nested_folder_is_created_under_its_full_path() {
    let cache = Cache::open_in_memory().unwrap();
    let mut mb = mailbox(Vec::new());
    let mut p = FakeProvider::new(vec![remote("m1", "t1", 1_790_000_000)]);
    cache.upsert_account(&account()).unwrap();
    pull_all(&mut mb, &cache, &mut p, None);
    let id = inbox(&mb)[0];

    let parent = mb.create_folder(ACCOUNT, "Work", None);
    let child = mb.create_folder(ACCOUNT, "Reports", Some(parent));
    mb.set_state(&[id], TriageState::Filed(child));

    flush(&mb, &cache, &mut p);
    assert_eq!(p.created, vec!["Work/Reports".to_owned()]);
    assert_eq!(p.moves[0].2, RemoteState::Folder("Label_1".to_owned()));
    assert_eq!(
        cache.folder_remote_id(child).unwrap().as_deref(),
        Some("Label_1")
    );
    assert_eq!(
        cache.folder_remote_id(parent).unwrap(),
        None,
        "only the filed folder gets a label"
    );
    assert!(pending_moves(&mb, &cache, ACCOUNT).is_empty());
}

#[test]
fn a_folder_from_the_provider_becomes_a_local_folder() {
    let cache = Cache::open_in_memory().unwrap();
    let mut mb = mailbox(Vec::new());
    let mut p = FakeProvider::new(vec![remote("m1", "t1", 1_790_000_000)]).with(vec![
        RemoteFolder {
            id: "Label_9".to_owned(),
            path: "Work/Reports".to_owned(),
        },
        RemoteFolder {
            id: "Label_8".to_owned(),
            path: "Work".to_owned(),
        },
    ]);
    cache.upsert_account(&account()).unwrap();
    p.remote_set("m1", RemoteState::Folder("Label_9".to_owned()));

    pull_all(&mut mb, &cache, &mut p, None);

    let id = mb
        .messages()
        .iter()
        .find(|m| m.account == ACCOUNT)
        .unwrap()
        .id;
    let TriageState::Filed(folder) = mb.state_of(id).unwrap() else {
        panic!("expected the message to be filed");
    };
    let folder = mb.folder(folder).expect("folder exists locally");
    assert_eq!(folder.name, "Reports");
    let parent = mb.folder(folder.parent.unwrap()).expect("parent exists");
    assert_eq!(parent.name, "Work");
    assert_eq!(parent.parent, None);
    assert_eq!(
        cache.folder_remote_id(folder.id).unwrap().as_deref(),
        Some("Label_9")
    );
    assert_eq!(
        cache.folder_by_remote(ACCOUNT, "Label_8").unwrap(),
        Some(parent.id)
    );
}

#[test]
fn empty_remote_folders_are_imported_and_survive_a_restart() {
    let cache = Cache::open_in_memory().unwrap();
    let mut mb = mailbox(Vec::new());
    let mut p = FakeProvider::new(Vec::new()).with(vec![RemoteFolder {
        id: "Label_3".to_owned(),
        path: "Receipts".to_owned(),
    }]);
    cache.upsert_account(&account()).unwrap();

    pull_all(&mut mb, &cache, &mut p, None);
    pull_all(&mut mb, &cache, &mut p, Some("1".to_owned()));

    let names = |folders: &[mail_classifier::model::Folder]| -> Vec<String> {
        folders.iter().filter(|f| f.account == ACCOUNT).map(|f| f.name.clone()).collect()
    };
    assert_eq!(names(mb.all_folders()), ["Receipts"]);
    let (_, cached, _) = cache.load().unwrap();
    assert_eq!(names(&cached), ["Receipts"]);
}

// ------------------------------------------------------------------- cursors

#[test]
fn a_large_initial_import_lands_in_chunks_and_resumes() {
    let cache = Cache::open_in_memory().unwrap();
    let mut mb = mailbox(Vec::new());
    let remotes = (0..120)
        .map(|i| remote(&format!("m{i}"), &format!("t{i}"), 1_790_000_000 + i))
        .collect();
    let mut p = FakeProvider::new(remotes);
    cache.upsert_account(&account()).unwrap();

    let mut sizes = Vec::new();
    while cache.cursor(ACCOUNT).unwrap().is_none() {
        let known = cache.remote_ids(ACCOUNT).unwrap();
        let pull = background_pull(&mut p, None, &known, &[]).expect("pull");
        sizes.push(pull.fetched.len());
        apply_fetched(&mut mb, &cache, ACCOUNT, &pull).expect("apply");
        assert!(sizes.len() <= 3, "import never finished: {sizes:?}");
    }
    assert_eq!(sizes, [50, 50, 20], "each pull fetches only the next uncached chunk");
    assert_eq!(inbox(&mb).len(), 120);
    assert_eq!(cache.cursor(ACCOUNT).unwrap().as_deref(), Some("1"));
}

#[test]
fn an_expired_cursor_falls_back_to_a_recent_listing() {
    let cache = Cache::open_in_memory().unwrap();
    let mut mb = mailbox(Vec::new());
    let mut p = FakeProvider::new(vec![remote("m1", "t1", 1_790_000_000)]);
    cache.upsert_account(&account()).unwrap();
    p.expire_cursor = true;

    let pull = background_pull(&mut p, Some("stale".to_owned()), &HashSet::new(), &[]).expect("pull");
    assert_eq!(pull.fetched.len(), 1);
    assert!(pull.removed.is_empty());
    apply_fetched(&mut mb, &cache, ACCOUNT, &pull).expect("apply");
    assert_eq!(inbox(&mb).len(), 1);
}

// ---------------------------------------------------------------- persistence

#[test]
fn pending_states_and_snoozes_survive_a_restart() {
    let cache = Cache::open_in_memory().unwrap();
    let mut mb = mailbox(Vec::new());
    let mut p = FakeProvider::new(vec![
        remote("m1", "t1", 1_790_000_000),
        remote("m2", "t2", 1_790_000_100),
    ]);
    cache.upsert_account(&account()).unwrap();
    pull_all(&mut mb, &cache, &mut p, None);
    let ids = inbox(&mb);

    let clock = FakeClock::new(1_790_000_000);
    mb.snooze(&[ids[0]], clock.now() + 7_200, clock.now());
    mb.set_state(&[ids[1]], TriageState::Archived);
    persist_local(&mb, &cache);

    let (accounts, folders, messages) = cache.load().unwrap();
    assert_eq!(accounts, vec![account()]);
    assert!(folders.is_empty());
    let restored: Mailbox = Mailbox::from_parts(
        messages,
        accounts,
        folders,
        Rc::new(contacts::open_seeded_in_memory().unwrap()),
    );
    assert_eq!(restored.state_of(ids[0]), Some(TriageState::Snoozed));
    assert_eq!(
        restored.snoozed_until(ids[0]),
        Some(clock.now() + 7_200),
        "the snooze deadline is rebuilt from the stored string"
    );
    assert_eq!(restored.state_of(ids[1]), Some(TriageState::Archived));

    // The restored mailbox still sees both changes as unconfirmed.
    assert_eq!(pending_moves(&restored, &cache, ACCOUNT).len(), 2);
}

#[test]
fn a_pull_never_pushes_the_archived_copy_of_a_snoozed_message() {
    // Regression guard: the snoozed branch must not mark the snooze as
    // confirmed, which would strand the message in the archive forever.
    let cache = Cache::open_in_memory().unwrap();
    let mut mb = mailbox(Vec::new());
    let mut p = FakeProvider::new(vec![remote("m1", "t1", 1_790_000_000)]);
    cache.upsert_account(&account()).unwrap();
    pull_all(&mut mb, &cache, &mut p, None);
    let id = inbox(&mb)[0];
    let clock = FakeClock::new(1_790_000_000);
    mb.snooze(&[id], clock.now() + 60, clock.now());

    // Applied without pushing, as the app loop does between iterations.
    pull_all(&mut mb, &cache, &mut p, Some("1".to_owned()));
    persist_local(&mb, &cache);

    assert_eq!(mb.state_of(id), Some(TriageState::Snoozed));
    let moves = pending_moves(&mb, &cache, ACCOUNT);
    assert_eq!(moves.len(), 1);
    assert_eq!(moves[0].to, RemoteState::Archived);
}