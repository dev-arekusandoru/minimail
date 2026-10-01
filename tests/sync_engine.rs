//! Sync engine against an in-memory cache and an in-file provider.

use std::rc::Rc;
use std::sync::{Arc, Mutex};

use mail_classifier::clock::{Clock, DAY, FakeClock, Timestamp};
use mail_classifier::contacts;
use mail_classifier::model::{
    Account, Folder, Location, Mailbox, Message, MessageId, ProviderKind, TriageState, View,
};
use mail_classifier::provider::{
    Body, Changes, MailProvider, Page, ProviderError, RemoteFlags, RemoteFolder, RemoteId,
    RemoteMessage, RemoteState, Scope, Window,
};
use mail_classifier::sync::cache::Cache;
use mail_classifier::sync::{
    RoundSummary, SharedProvider, apply_body, apply_moves, apply_reads, apply_round, body_request,
    has_older, pending_moves, pending_reads, persist_local, plan_round, run_body, run_moves,
    run_reads, run_round, scopes_for,
};
use parking_lot::Mutex as TestMutex;

const ACCOUNT: &str = "gmail:a@x.io";
/// A day after the test mail, so all of it falls inside the 30-day window.
const NOW: Timestamp = 1_790_086_400;
/// The cutoff every test's backfill is measured against.
const CUTOFF: Timestamp = NOW - 30 * DAY;

// ------------------------------------------------------------------ provider

/// One `list` call, as the tests want to assert on it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ListCall {
    scope: Scope,
    window: Window,
    page: Option<String>,
    max: usize,
}

/// Everything the fake provider both serves and records.
#[derive(Default)]
struct Recorded {
    lists: Vec<ListCall>,
    fetches: Vec<Vec<RemoteId>>,
    bodies: Vec<RemoteId>,
    snapshots: Vec<Timestamp>,
    cursors: usize,
}

struct State {
    messages: Vec<RemoteMessage>,
    folders: Vec<RemoteFolder>,
    cursor: String,
    next_label: usize,
    moves: Vec<(RemoteId, RemoteState, RemoteState)>,
    created: Vec<String>,
    reads: Vec<(RemoteId, bool)>,
    fail_move: Option<ProviderError>,
    expire_cursor: bool,
    /// What `changes` reports; `None` means "nothing changed".
    pending: Option<Changes>,
    fail_list: Option<ProviderError>,
    page_size: usize,
    recorded: Recorded,
}

/// The fake's state, shared with the test so a test can inspect and change it
/// without reaching through the provider trait object.
type Handle = Arc<TestMutex<State>>;

/// A provider over a fixed set of remote messages that records every call.
struct FakeProvider {
    state: Handle,
}

impl FakeProvider {
    fn without_folders(messages: Vec<RemoteMessage>) -> (Handle, SharedProvider) {
        Self::build(messages, Vec::new())
    }

    fn with_folders(messages: Vec<RemoteMessage>, folders: Vec<RemoteFolder>) -> (Handle, SharedProvider) {
        Self::build(messages, folders)
    }

    fn build(messages: Vec<RemoteMessage>, folders: Vec<RemoteFolder>) -> (Handle, SharedProvider) {
        let state = Handle::new(TestMutex::new(State {
            messages,
            folders,
            cursor: "1".to_owned(),
            next_label: 0,
            moves: Vec::new(),
            created: Vec::new(),
            reads: Vec::new(),
            fail_move: None,
            expire_cursor: false,
            pending: None,
            fail_list: None,
            page_size: 50,
            recorded: Recorded::default(),
        }));
        let provider = Arc::new(Mutex::new(Box::new(FakeProvider { state: state.clone() }) as Box<dyn MailProvider>));
        (state, provider)
    }
}

/// Change a remote message's state, as the provider would see it.
fn remote_set(state: &Handle, id: &str, new_state: RemoteState) {
    let mut s = state.lock();
    let m = s.messages.iter_mut().find(|m| m.id == id).expect("remote message");
    m.state = new_state;
}

fn remote_unread(state: &Handle, id: &str, unread: bool) {
    let mut s = state.lock();
    let m = s.messages.iter_mut().find(|m| m.id == id).expect("remote message");
    m.unread = unread;
}

fn flags(state: &Handle, id: &str) -> RemoteFlags {
    let s = state.lock();
    let m = s.messages.iter().find(|m| m.id == id).expect("remote message");
    RemoteFlags { state: m.state.clone(), unread: m.unread }
}

fn moves(state: &Handle) -> Vec<(RemoteId, RemoteState, RemoteState)> {
    state.lock().moves.clone()
}

fn created(state: &Handle) -> Vec<String> {
    state.lock().created.clone()
}

fn reads(state: &Handle) -> Vec<(RemoteId, bool)> {
    state.lock().reads.clone()
}

fn lists(state: &Handle) -> Vec<ListCall> {
    state.lock().recorded.lists.clone()
}

/// The scopes listed so far, in call order.
fn listed_scopes(state: &Handle) -> Vec<Scope> {
    lists(state).into_iter().map(|c| c.scope).collect()
}

fn fetched(state: &Handle) -> Vec<Vec<RemoteId>> {
    state.lock().recorded.fetches.clone()
}

/// Every id passed to `fetch_headers`, in order.
fn fetched_ids(state: &Handle) -> Vec<RemoteId> {
    fetched(state).concat()
}

fn bodies(state: &Handle) -> Vec<RemoteId> {
    state.lock().recorded.bodies.clone()
}

fn snapshots(state: &Handle) -> Vec<Timestamp> {
    state.lock().recorded.snapshots.clone()
}

fn cursors(state: &Handle) -> usize {
    state.lock().recorded.cursors
}

/// Queue a history delta, so the next `changes` reports it.
fn with_updates(
    state: &Handle,
    added: &[&str],
    updates: &[(&str, RemoteFlags)],
    removed: &[&str],
) {
    let mut s = state.lock();
    let cursor = s.cursor.clone();
    s.pending = Some(Changes {
        added: added.iter().map(|id| (*id).to_owned()).collect(),
        updated: updates.iter().map(|(id, f)| ((*id).to_owned(), f.clone())).collect(),
        removed: removed.iter().map(|id| (*id).to_owned()).collect(),
        cursor,
    });
}

/// Nothing changed since the last check.
fn with_no_changes(state: &Handle) {
    let mut s = state.lock();
    let cursor = s.cursor.clone();
    s.pending = Some(Changes { added: Vec::new(), updated: Vec::new(), removed: Vec::new(), cursor });
}

fn expire_cursor(state: &Handle) {
    state.lock().expire_cursor = true;
}

fn set_page_size(state: &Handle, size: usize) {
    state.lock().page_size = size;
}

fn in_scope(m: &RemoteMessage, scope: &Scope) -> bool {
    match scope {
        Scope::Inbox => m.state == RemoteState::Inbox,
        Scope::Trash => m.state == RemoteState::Trash,
        Scope::Archive => m.state == RemoteState::Archived,
        Scope::Folder(id) => m.state == RemoteState::Folder(id.clone()),
    }
}

/// The ids a listing returns: everything in the scope and window, newest first.
fn matches(state: &State, scope: &Scope, window: Window) -> Vec<RemoteId> {
    let mut ids: Vec<(Timestamp, RemoteId)> = state
        .messages
        .iter()
        .filter(|m| in_scope(m, scope))
        .filter(|m| match window {
            Window::Since(since) => m.received >= since,
            Window::Before(before) => m.received < before,
        })
        .map(|m| (m.received, m.id.clone()))
        .collect();
    ids.sort_by_key(|&(received, _)| std::cmp::Reverse(received));
    ids.into_iter().map(|(_, id)| id).collect()
}

fn clone_error(e: &ProviderError) -> ProviderError {
    match e {
        ProviderError::Auth(m) => ProviderError::Auth(m.clone()),
        ProviderError::Network(m) => ProviderError::Network(m.clone()),
        ProviderError::CursorExpired => ProviderError::CursorExpired,
        ProviderError::RateLimited => ProviderError::RateLimited,
        ProviderError::Api { status, message } => {
            ProviderError::Api { status: *status, message: message.clone() }
        }
    }
}

impl MailProvider for FakeProvider {
    fn folders(&mut self) -> Result<Vec<RemoteFolder>, ProviderError> {
        Ok(self.state.lock().folders.clone())
    }

    fn create_folder(&mut self, path: &str) -> Result<RemoteFolder, ProviderError> {
        let mut s = self.state.lock();
        s.next_label += 1;
        let id = format!("Label_{}", s.next_label);
        s.created.push(path.to_owned());
        let folder = RemoteFolder { id, path: path.to_owned() };
        s.folders.push(folder.clone());
        Ok(folder)
    }

    fn cursor(&mut self) -> Result<String, ProviderError> {
        let mut s = self.state.lock();
        s.recorded.cursors += 1;
        Ok(s.cursor.clone())
    }

    fn list(
        &mut self,
        scope: &Scope,
        window: Window,
        page: Option<&str>,
        max: usize,
    ) -> Result<Page, ProviderError> {
        let mut s = self.state.lock();
        s.recorded.lists.push(ListCall {
            scope: scope.clone(),
            window,
            page: page.map(str::to_owned),
            max,
        });
        if let Some(e) = &s.fail_list {
            return Err(clone_error(e));
        }
        let all = matches(&s, scope, window);
        let skip: usize = page.and_then(|p| p.parse().ok()).unwrap_or(0);
        let ids: Vec<RemoteId> = all.iter().skip(skip).take(max).cloned().collect();
        let next = (skip + ids.len() < all.len()).then(|| (skip + ids.len()).to_string());
        Ok(Page { ids, next })
    }

    fn snapshot(&mut self, since: Timestamp) -> Result<Vec<(RemoteId, RemoteFlags)>, ProviderError> {
        let mut s = self.state.lock();
        s.recorded.snapshots.push(since);
        Ok(s.messages
            .iter()
            .filter(|m| m.received >= since)
            .map(|m| (m.id.clone(), RemoteFlags { state: m.state.clone(), unread: m.unread }))
            .collect())
    }

    fn changes(&mut self, _cursor: &str) -> Result<Changes, ProviderError> {
        let s = self.state.lock();
        if s.expire_cursor {
            return Err(ProviderError::CursorExpired);
        }
        let mut changes = s.pending.clone().unwrap_or_default();
        changes.cursor = s.cursor.clone();
        Ok(changes)
    }

    fn fetch_headers(&mut self, ids: &[RemoteId]) -> Result<Vec<RemoteMessage>, ProviderError> {
        let mut s = self.state.lock();
        s.recorded.fetches.push(ids.to_vec());
        Ok(ids.iter().filter_map(|id| s.messages.iter().find(|m| &m.id == id).cloned()).collect())
    }

    fn fetch_body(&mut self, id: &RemoteId) -> Result<Body, ProviderError> {
        let mut s = self.state.lock();
        s.recorded.bodies.push(id.clone());
        Ok(Body { body: format!("full body of {id}"), html: None, attachments: Vec::new() })
    }

    fn move_message(
        &mut self,
        id: &RemoteId,
        from: &RemoteState,
        to: &RemoteState,
    ) -> Result<(), ProviderError> {
        let mut s = self.state.lock();
        if let Some(e) = &s.fail_move {
            return Err(clone_error(e));
        }
        s.moves.push((id.clone(), from.clone(), to.clone()));
        if let Some(m) = s.messages.iter_mut().find(|m| &m.id == id) {
            m.state = to.clone();
        }
        Ok(())
    }

    fn set_read(&mut self, id: &RemoteId, read: bool) -> Result<(), ProviderError> {
        let mut s = self.state.lock();
        s.reads.push((id.clone(), read));
        if let Some(m) = s.messages.iter_mut().find(|m| &m.id == id) {
            m.unread = !read;
        }
        Ok(())
    }
}

// -------------------------------------------------------------------- setup

fn account() -> Account {
    Account {
        id: ACCOUNT.to_owned(),
        name: "a@x.io".to_owned(),
        email: "a@x.io".to_owned(),
        color: "#61afef".to_owned(),
        icon: None,
        nickname: None,
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
        snippet: format!("snippet of {id}"),
        received,
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

fn seeded() -> Cache {
    let cache = Cache::open_in_memory().unwrap();
    cache.upsert_account(&account()).unwrap();
    cache
}

fn inbox(mb: &Mailbox) -> Vec<MessageId> {
    mb.ids_in_view(&View { location: inbox_location(), ..View::default() })
}

fn inbox_location() -> Location {
    Location::Inbox(ACCOUNT.to_owned())
}

fn remote_id_of(cache: &Cache, id: MessageId) -> RemoteId {
    cache.remote_state(id).unwrap().expect("cached").1
}

fn local_id(cache: &Cache, rid: &str) -> MessageId {
    cache.message_by_remote(ACCOUNT, rid).unwrap().expect("cached")
}

/// Plan, run and apply one round, the way the app does.
fn round(mb: &mut Mailbox, cache: &Cache, p: &SharedProvider, check: bool, older: &[Scope]) -> RoundSummary {
    let planned = plan_round(mb, cache, ACCOUNT, NOW, check, older);
    assert_eq!(planned.account(), ACCOUNT);
    apply_round(mb, cache, run_round(p, planned))
}

/// Check once, then round until the backfill has nothing left, as the loop does
/// while it wakes itself again.
fn drain(mb: &mut Mailbox, cache: &Cache, p: &SharedProvider, check: bool) {
    if check {
        round(mb, cache, p, true, &[]);
    }
    for _ in 0..40 {
        if !round(mb, cache, p, false, &[]).more {
            return;
        }
    }
    panic!("backfill never finished");
}

/// Push every pending move and read, then record the confirmations.
fn flush(mb: &mut Mailbox, cache: &Cache, p: &SharedProvider) {
    let move_results = {
        let mut provider = p.lock().unwrap_or_else(|e| e.into_inner());
        run_moves(provider.as_mut(), pending_moves(mb, cache, ACCOUNT))
    };
    let read_results = {
        let mut provider = p.lock().unwrap_or_else(|e| e.into_inner());
        run_reads(provider.as_mut(), pending_reads(mb, cache, ACCOUNT))
    };
    assert_eq!(apply_moves(mb, cache, &move_results), None, "apply_moves failed");
    assert_eq!(apply_reads(cache, &read_results), None, "apply_reads failed");
}

// --------------------------------------------------------------------- read

#[test]
fn server_read_state_is_imported_and_follows_server_changes() {
    let cache = seeded();
    let mut mb = mailbox(Vec::new());
    let mut seen = remote("m1", "t1", 1_790_000_000);
    seen.unread = false;
    let (state, provider) = FakeProvider::without_folders(vec![seen, remote("m2", "t2", 1_790_000_100)]);
    drain(&mut mb, &cache, &provider, true);

    let (m1, m2) = (local_id(&cache, "m1"), local_id(&cache, "m2"));
    assert!(mb.is_read(m1) && !mb.is_read(m2));

    // Read m2 and mark m1 unread in Gmail web.
    remote_unread(&state, "m1", true);
    remote_unread(&state, "m2", false);
    with_updates(&state, &[], &[("m1", flags(&state, "m1")), ("m2", flags(&state, "m2"))], &[]);
    assert!(round(&mut mb, &cache, &provider, true, &[]).checked);

    assert!(!mb.is_read(m1) && mb.is_read(m2));
    assert!(pending_reads(&mb, &cache, ACCOUNT).is_empty(), "server changes are not echoed back");
}

#[test]
fn reading_locally_is_pushed_once_and_survives_a_check_before_the_push() {
    let cache = seeded();
    let mut mb = mailbox(Vec::new());
    let (state, provider) = FakeProvider::without_folders(vec![remote("m1", "t1", 1_790_000_000)]);
    drain(&mut mb, &cache, &provider, true);
    let m1 = local_id(&cache, "m1");

    mb.mark_read(m1);
    // A check lands before the push: the server still says unread, the local read wins.
    round(&mut mb, &cache, &provider, true, &[]);
    assert!(mb.is_read(m1));

    flush(&mut mb, &cache, &provider);
    assert_eq!(reads(&state), [("m1".to_owned(), true)]);
    assert!(pending_reads(&mb, &cache, ACCOUNT).is_empty());
}

#[test]
fn account_style_and_nickname_round_trip_through_the_cache() {
    let cache = Cache::open_in_memory().unwrap();
    let mut a = account();
    a.icon = Some("briefcase".to_owned());
    a.nickname = Some("Side gig".to_owned());
    cache.upsert_account(&a).unwrap();
    assert_eq!(cache.load().unwrap().0, vec![a.clone()]);

    a.icon = Some("rocket".to_owned());
    a.color = "#98c379".to_owned();
    a.nickname = None;
    cache.upsert_account(&a).unwrap();
    assert_eq!(cache.load().unwrap().0, vec![a]);
}

#[test]
fn a_cache_from_before_account_icons_loads_with_no_icon_and_keeps_its_color() {
    let path = std::env::temp_dir().join(format!("mc-icon-migration-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    {
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch(
            "CREATE TABLE accounts(id TEXT PRIMARY KEY, provider TEXT NOT NULL, name TEXT NOT NULL,
                email TEXT NOT NULL, color TEXT NOT NULL, cursor TEXT);
             INSERT INTO accounts VALUES('a', '\"Gmail\"', 'A', 'a@x.io', '#e06c75', 'c1');",
        )
        .unwrap();
    }
    let cache = Cache::open(&path).unwrap();
    let accounts = cache.load().unwrap().0;
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].icon, None);
    assert_eq!(accounts[0].color, "#e06c75");
    assert_eq!(cache.cursor("a").unwrap().as_deref(), Some("c1"), "the cursor survives");
    let _ = std::fs::remove_file(&path);
    assert_eq!(accounts[0].nickname, None);
}

#[test]
fn the_last_completed_check_is_stored_and_old_caches_start_with_none() {
    let path = std::env::temp_dir().join(format!("mc-synced-migration-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    {
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch(
            "CREATE TABLE accounts(id TEXT PRIMARY KEY, provider TEXT NOT NULL, name TEXT NOT NULL,
                email TEXT NOT NULL, color TEXT NOT NULL, icon TEXT, nickname TEXT, cursor TEXT);
             INSERT INTO accounts VALUES('a', '\"Gmail\"', 'A', 'a@x.io', '#e06c75', NULL, NULL, 'c1');",
        )
        .unwrap();
    }
    let cache = Cache::open(&path).unwrap();
    assert_eq!(cache.synced_at("a").unwrap(), None, "an old account never recorded a check");
    cache.set_synced_at("a", 1_790_000_000).unwrap();
    assert_eq!(cache.synced_at("a").unwrap(), Some(1_790_000_000));
    cache.upsert_account(&Account { id: "a".into(), ..account() }).unwrap();
    assert_eq!(cache.synced_at("a").unwrap(), Some(1_790_000_000), "restyling keeps it");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_cache_from_before_read_sync_is_reconciled_against_the_window() {
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
    cache.set_cursor(ACCOUNT, "stale").unwrap();
    let (_, _, messages) = cache.load().unwrap();
    let mut mb = mailbox(messages);
    assert!(pending_reads(&mb, &cache, ACCOUNT).is_empty(), "unknown rows are never pushed");

    let mut read_in_gmail = remote("m1", "t1", 1_790_000_000);
    read_in_gmail.unread = false;
    let (state, provider) = FakeProvider::without_folders(vec![read_in_gmail]);
    expire_cursor(&state);

    let summary = round(&mut mb, &cache, &provider, true, &[]);

    assert!(summary.checked, "the round still reports a completed check");
    assert!(mb.is_read(1), "the server's read state replaces the unknown one");
    assert!(!cache.has_unknown_read(ACCOUNT).unwrap());
    assert_eq!(snapshots(&state), [CUTOFF], "one listing of the 30-day window");
}

// ---------------------------------------------------------------- backfill

#[test]
fn the_inbox_is_backfilled_before_archive_trash_and_labels() {
    let cache = seeded();
    let mut mb = mailbox(Vec::new());
    let (state, provider) = FakeProvider::with_folders(
        vec![
            remote("i1", "t1", 1_790_000_000),
            remote("i2", "t2", 1_790_000_100),
            remote("a1", "t3", 1_790_000_200),
            remote("x1", "t4", 1_790_000_300),
            remote("f1", "t5", 1_790_000_400),
        ],
        vec![
            RemoteFolder { id: "Label_9".to_owned(), path: "Work".to_owned() },
            // A nested label whose parent is not listed is its own scope.
            RemoteFolder { id: "Label_7".to_owned(), path: "Work/Reports".to_owned() },
        ],
    );
    remote_set(&state, "a1", RemoteState::Archived);
    remote_set(&state, "x1", RemoteState::Trash);
    remote_set(&state, "f1", RemoteState::Folder("Label_9".to_owned()));

    drain(&mut mb, &cache, &provider, true);

    assert_eq!(
        listed_scopes(&state),
        [
            Scope::Inbox,
            Scope::Archive,
            Scope::Trash,
            Scope::Folder("Label_9".to_owned()),
            Scope::Folder("Label_7".to_owned()),
        ],
        "one page per scope, in priority order"
    );
    assert_eq!(fetched_ids(&state), ["i2", "i1", "a1", "x1", "f1"], "listings are newest-first");
    assert_eq!(mb.messages().len(), 5);
    assert_eq!(inbox(&mb).len(), 2);
}

#[test]
fn the_cursor_is_stored_before_any_backfill_page() {
    let cache = seeded();
    let mut mb = mailbox(Vec::new());
    let (state, provider) = FakeProvider::without_folders(vec![remote("i1", "t1", 1_790_000_000)]);

    // The first round only takes a cursor; the import starts once it is stored.
    round(&mut mb, &cache, &provider, true, &[]);
    assert_eq!(cache.cursor(ACCOUNT).unwrap().as_deref(), Some("1"));
    assert_eq!(cursors(&state), 1);
    assert!(lists(&state).is_empty(), "nothing is listed before the cursor is stored");

    let second = round(&mut mb, &cache, &provider, false, &[]);
    assert_eq!(listed_scopes(&state), [Scope::Inbox]);
    assert_eq!(second.new_messages, 1);
}

#[test]
fn a_backfill_page_imports_fifty_ids_at_a_time() {
    let cache = seeded();
    let mut mb = mailbox(Vec::new());
    let remotes: Vec<RemoteMessage> = (0..120)
        .map(|i| remote(&format!("m{i:03}"), &format!("t{i:03}"), 1_790_000_000 + i))
        .collect();
    let (state, provider) = FakeProvider::without_folders(remotes);
    // The first round only stores the cursor; the import follows it.
    round(&mut mb, &cache, &provider, true, &[]);

    let mut sizes = Vec::new();
    for _ in 0..5 {
        let seen = fetched(&state).len();
        if !round(&mut mb, &cache, &provider, false, &[]).more {
            break;
        }
        // Later scopes list nothing new, so they fetch nothing at all.
        sizes.extend(fetched(&state)[seen..].iter().map(|ids| ids.len()));
    }

    assert_eq!(sizes, [50, 50, 20], "each round fetches only the next uncached page");
    assert_eq!(mb.messages().len(), 120);
    assert_eq!(
        listed_scopes(&state).first(),
        Some(&Scope::Inbox),
        "the inbox is listed first, one page per round"
    );
}

#[test]
fn load_more_lists_before_the_cutoff_and_stops_when_done() {
    let cache = seeded();
    let mut mb = mailbox(Vec::new());
    let old: Vec<RemoteMessage> = (0..70)
        .map(|i| remote(&format!("old{i:02}"), &format!("s{i:02}"), 1_000_000 + i))
        .collect();
    let (state, provider) = FakeProvider::without_folders([vec![remote("new", "t1", 1_790_000_000)], old].concat());
    set_page_size(&state, 50);

    drain(&mut mb, &cache, &provider, true);

    let summary = round(&mut mb, &cache, &provider, false, &[Scope::Inbox]);
    assert!(summary.more, "a second older page remains");
    let calls = lists(&state);
    let last = calls.last().unwrap();
    assert_eq!(last.window, Window::Before(CUTOFF), "load-more walks backwards in time");
    assert_eq!(last.max, 50);

    round(&mut mb, &cache, &provider, false, &[Scope::Inbox]);
    assert_eq!(mb.messages().len(), 71);
    assert!(
        !has_older(&cache, ACCOUNT, &Scope::Inbox),
        "nothing older than the oldest page is left"
    );
}

#[test]
fn load_more_finishes_the_recent_pass_first() {
    let cache = seeded();
    let mut mb = mailbox(Vec::new());
    let remotes: Vec<RemoteMessage> = (0..80)
        .map(|i| remote(&format!("m{i:03}"), &format!("t{i:03}"), 1_790_000_000 + i))
        .collect();
    let (state, provider) = FakeProvider::without_folders(remotes);
    set_page_size(&state, 50);

    // The scope is still being imported, so a load-more request serves the
    // remaining pages of the window rather than reaching past it.
    round(&mut mb, &cache, &provider, true, &[]);
    let window = round(&mut mb, &cache, &provider, false, &[Scope::Inbox]);
    assert!(window.more, "the window still has a page left");
    assert_eq!(lists(&state).last().unwrap().window, Window::Since(CUTOFF));
    assert!(
        !lists(&state).iter().any(|c| c.window == Window::Before(CUTOFF)),
        "nothing older is listed before the window is done"
    );
}

// ------------------------------------------------------------ remote changes

#[test]
fn a_history_label_change_updates_read_state_and_triage_without_a_fetch() {
    let cache = seeded();
    let mut mb = mailbox(Vec::new());
    let (state, provider) = FakeProvider::without_folders(vec![remote("m1", "t1", 1_790_000_000)]);
    drain(&mut mb, &cache, &provider, true);
    let id = local_id(&cache, "m1");
    let fetches = fetched(&state).len();

    remote_set(&state, "m1", RemoteState::Trash);
    remote_unread(&state, "m1", false);
    with_updates(&state, &[], &[("m1", flags(&state, "m1"))], &[]);

    round(&mut mb, &cache, &provider, true, &[]);

    assert_eq!(mb.state_of(id), Some(TriageState::Deleted), "the label change is applied");
    assert!(mb.is_read(id), "the read flag from the same history entry lands too");
    assert_eq!(fetched(&state).len(), fetches, "a cached message is never downloaded again");
    assert!(pending_moves(&mb, &cache, ACCOUNT).is_empty(), "the applied change is not pushed back");
}

#[test]
fn an_expired_cursor_reconciles_the_window() {
    let cache = seeded();
    let mut mb = mailbox(Vec::new());
    let (state, provider) = FakeProvider::without_folders(vec![
        remote("m1", "t1", 1_790_000_000),
        remote("m2", "t2", 1_790_000_100),
    ]);
    drain(&mut mb, &cache, &provider, true);
    let (m1, m2) = (local_id(&cache, "m1"), local_id(&cache, "m2"));

    // m1 is trashed and m2 disappears in Gmail, and the cursor expires.
    remote_set(&state, "m1", RemoteState::Trash);
    state.lock().messages.retain(|m| m.id != "m2");
    expire_cursor(&state);

    let summary = round(&mut mb, &cache, &provider, true, &[]);

    assert!(summary.checked);
    assert_eq!(snapshots(&state), [CUTOFF], "the window is listed by id only");
    assert_eq!(mb.state_of(m1), Some(TriageState::Deleted), "the reconcile applies the flags");
    assert!(mb.get(m2).is_none(), "a message missing from the window is dropped");
    assert_eq!(cache.message_by_remote(ACCOUNT, "m2").unwrap(), None);
    assert_eq!(cache.cursor(ACCOUNT).unwrap().as_deref(), Some("1"), "a fresh cursor is stored");
    assert_eq!(fetched(&state).len(), 1, "the reconcile itself downloads no message");
}

#[test]
fn a_reconcile_imports_mail_the_window_listing_found() {
    let cache = seeded();
    let mut mb = mailbox(Vec::new());
    let (state, provider) = FakeProvider::without_folders(vec![remote("m1", "t1", 1_790_000_000)]);
    drain(&mut mb, &cache, &provider, true);

    state.lock().messages.push(remote("m2", "t2", 1_790_000_100));
    expire_cursor(&state);
    // The reconcile itself only knows ids: it resets the backfill, and the page
    // that re-lists the window is what imports m2.
    assert_eq!(fetched(&state).len(), 1, "the reconcile downloads no message");
    round(&mut mb, &cache, &provider, true, &[]);
    assert_eq!(inbox(&mb).len(), 1, "the reset progress is planned into the next round");
    round(&mut mb, &cache, &provider, false, &[]);
    assert_eq!(inbox(&mb).len(), 2, "the reset backfill imports what the window listed");
    assert!(
        listed_scopes(&state).contains(&Scope::Inbox),
        "the reset progress makes the backfill re-list the window"
    );
}

#[test]
fn a_remote_trash_takes_effect_unless_a_local_change_is_pending() {
    let cache = seeded();
    let mut mb = mailbox(Vec::new());
    let (state, provider) = FakeProvider::without_folders(vec![
        remote("m1", "t1", 1_790_000_000),
        remote("m2", "t2", 1_790_000_100),
    ]);
    drain(&mut mb, &cache, &provider, true);
    let (quiet, busy) = (local_id(&cache, "m2"), local_id(&cache, "m1"));

    // One message has a local archive the provider has not taken yet: the push
    // is rejected, so the change is still pending when the check lands.
    mb.set_state(&[busy], TriageState::Archived);
    state.lock().fail_move = Some(ProviderError::Api { status: 500, message: "no".to_owned() });
    remote_set(&state, "m1", RemoteState::Trash);
    remote_set(&state, "m2", RemoteState::Trash);
    with_updates(&state, &[], &[("m1", flags(&state, "m1")), ("m2", flags(&state, "m2"))], &[]);

    let summary = round(&mut mb, &cache, &provider, true, &[]);

    assert!(summary.error.is_some(), "the rejected push is reported");
    assert_eq!(mb.state_of(quiet), Some(TriageState::Deleted), "the server's state wins");
    assert_eq!(mb.state_of(busy), Some(TriageState::Archived), "the pending local change is not clobbered");
    assert_eq!(pending_moves(&mb, &cache, ACCOUNT).len(), 1, "it is still waiting to be pushed");
}

#[test]
fn a_removed_id_disappears_locally() {
    let cache = seeded();
    let mut mb = mailbox(Vec::new());
    let (state, provider) = FakeProvider::without_folders(vec![
        remote("m1", "t1", 1_790_000_000),
        remote("m2", "t2", 1_790_000_100),
    ]);
    drain(&mut mb, &cache, &provider, true);
    let ids = inbox(&mb);
    let (kept, gone) = (ids[0], remote_id_of(&cache, ids[1]));

    state.lock().cursor = "2".to_owned();
    with_updates(&state, &[], &[], &[gone.as_str()]);
    round(&mut mb, &cache, &provider, true, &[]);

    assert_eq!(mb.messages().len(), 1);
    assert!(mb.get(ids[1]).is_none());
    assert!(inbox(&mb).contains(&kept));
    assert_eq!(cache.message_by_remote(ACCOUNT, &gone).unwrap(), None);
    assert_eq!(cache.cursor(ACCOUNT).unwrap().as_deref(), Some("2"));
}

#[test]
fn a_check_with_no_changes_changes_nothing() {
    let cache = seeded();
    let mut mb = mailbox(Vec::new());
    let (state, provider) = FakeProvider::without_folders(vec![remote("m1", "t1", 1_790_000_000)]);
    drain(&mut mb, &cache, &provider, true);
    let before = (mb.messages().len(), fetched(&state).len());
    with_no_changes(&state);

    let summary = round(&mut mb, &cache, &provider, true, &[]);

    assert!(summary.checked);
    assert_eq!(summary.new_messages, 0);
    assert_eq!((mb.messages().len(), fetched(&state).len()), before, "nothing changed");
}

// ------------------------------------------------------------------ folders

#[test]
fn filing_creates_the_remote_label_once_per_batch() {
    let cache = seeded();
    let mut mb = mailbox(Vec::new());
    let (state, provider) = FakeProvider::without_folders(vec![
        remote("m1", "t1", 1_790_000_000),
        remote("m2", "t2", 1_790_000_100),
    ]);
    drain(&mut mb, &cache, &provider, true);
    let ids = inbox(&mb);

    let parent = mb.create_folder(ACCOUNT, "Parent", None);
    cache.upsert_folder(mb.folder(parent).unwrap(), None).unwrap();
    mb.set_state(&ids, TriageState::Filed(parent));
    persist_local(&mb, &cache);

    let planned = plan_round(&mb, &cache, ACCOUNT, NOW, false, &[]);
    assert!(!planned.is_empty(), "an unpushed file is work for the next round");
    let results = {
        let mut p = provider.lock().unwrap_or_else(|e| e.into_inner());
        run_moves(p.as_mut(), pending_moves(&mb, &cache, ACCOUNT))
    };

    assert_eq!(created(&state), ["Parent".to_owned()], "one label per batch");
    let calls = moves(&state);
    assert_eq!(calls.len(), 2);
    assert!(calls.iter().all(|(_, _, to)| *to == RemoteState::Folder("Label_1".to_owned())));
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
    let cache = seeded();
    let mut mb = mailbox(Vec::new());
    let (state, provider) = FakeProvider::without_folders(vec![remote("m1", "t1", 1_790_000_000)]);
    drain(&mut mb, &cache, &provider, true);
    let id = inbox(&mb)[0];

    let parent = mb.create_folder(ACCOUNT, "Work", None);
    let child = mb.create_folder(ACCOUNT, "Reports", Some(parent));
    mb.set_state(&[id], TriageState::Filed(child));

    flush(&mut mb, &cache, &provider);
    assert_eq!(created(&state), ["Work/Reports".to_owned()]);
    assert_eq!(moves(&state)[0].2, RemoteState::Folder("Label_1".to_owned()));
    assert_eq!(cache.folder_remote_id(child).unwrap().as_deref(), Some("Label_1"));
    assert_eq!(cache.folder_remote_id(parent).unwrap(), None, "only the filed folder gets a label");
    assert!(pending_moves(&mb, &cache, ACCOUNT).is_empty());
}

#[test]
fn a_folder_from_the_provider_becomes_a_local_folder() {
    let cache = seeded();
    let mut mb = mailbox(Vec::new());
    let (state, provider) = FakeProvider::with_folders(
        vec![remote("m1", "t1", 1_790_000_000)],
        vec![
            RemoteFolder { id: "Label_9".to_owned(), path: "Work/Reports".to_owned() },
            RemoteFolder { id: "Label_8".to_owned(), path: "Work".to_owned() },
        ],
    );
    remote_set(&state, "m1", RemoteState::Folder("Label_9".to_owned()));

    drain(&mut mb, &cache, &provider, true);

    let id = mb.messages().iter().find(|m| m.account == ACCOUNT).unwrap().id;
    let TriageState::Filed(folder) = mb.state_of(id).unwrap() else {
        panic!("expected the message to be filed");
    };
    let folder = mb.folder(folder).expect("folder exists locally");
    assert_eq!(folder.name, "Reports");
    let parent = mb.folder(folder.parent.unwrap()).expect("parent exists");
    assert_eq!(parent.name, "Work");
    assert_eq!(parent.parent, None);
    assert_eq!(cache.folder_remote_id(folder.id).unwrap().as_deref(), Some("Label_9"));
    assert_eq!(cache.folder_by_remote(ACCOUNT, "Label_8").unwrap(), Some(parent.id));
}

#[test]
fn empty_remote_folders_are_imported_and_survive_a_restart() {
    let cache = seeded();
    let mut mb = mailbox(Vec::new());
    let (_state, provider) = FakeProvider::with_folders(
        Vec::new(),
        vec![RemoteFolder { id: "Label_3".to_owned(), path: "Receipts".to_owned() }],
    );

    drain(&mut mb, &cache, &provider, true);

    let names = |folders: &[Folder]| -> Vec<String> {
        folders.iter().filter(|f| f.account == ACCOUNT).map(|f| f.name.clone()).collect()
    };
    assert_eq!(names(mb.all_folders()), ["Receipts"]);
    let (_, cached, _) = cache.load().unwrap();
    assert_eq!(names(&cached), ["Receipts"]);
}

// ------------------------------------------------------------------- snoozes

#[test]
fn snooze_archives_remotely_and_survives_a_check_until_it_wakes() {
    let cache = seeded();
    let mut mb = mailbox(Vec::new());
    let (state, provider) = FakeProvider::without_folders(vec![remote("m1", "t1", 1_790_000_000)]);
    drain(&mut mb, &cache, &provider, true);
    let id = inbox(&mb)[0];

    let clock = FakeClock::new(1_790_000_000);
    let until = clock.now() + 3_600;
    mb.snooze(&[id], until, clock.now());

    let planned = pending_moves(&mb, &cache, ACCOUNT);
    assert_eq!(planned.len(), 1);
    assert_eq!(planned[0].to, RemoteState::Archived, "no snooze exists remotely");
    flush(&mut mb, &cache, &provider);

    // A check reporting Archived must not wake the message early.
    with_updates(&state, &[], &[("m1", flags(&state, "m1"))], &[]);
    round(&mut mb, &cache, &provider, true, &[]);
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

#[test]
fn a_check_never_pushes_the_archived_copy_of_a_snoozed_message() {
    // Regression guard: the snoozed branch must not mark the snooze as
    // confirmed, which would strand the message in the archive forever.
    let cache = seeded();
    let mut mb = mailbox(Vec::new());
    let (state, provider) = FakeProvider::without_folders(vec![remote("m1", "t1", 1_790_000_000)]);
    drain(&mut mb, &cache, &provider, true);
    let id = inbox(&mb)[0];
    let clock = FakeClock::new(1_790_000_000);
    mb.snooze(&[id], clock.now() + 60, clock.now());

    // The push never lands, and a check reports the message read while it is
    // still in the inbox. The snooze must survive that, and stay queued: the
    // server has not seen the archive yet.
    state.lock().fail_move = Some(ProviderError::Api { status: 500, message: "no".to_owned() });
    remote_unread(&state, "m1", false);
    with_updates(&state, &[], &[("m1", flags(&state, "m1"))], &[]);
    round(&mut mb, &cache, &provider, true, &[]);
    persist_local(&mb, &cache);

    assert_eq!(mb.state_of(id), Some(TriageState::Snoozed), "a flag change is not a wake-up");
    assert_eq!(mb.snoozed_until(id), Some(clock.now() + 60), "the deadline survives the check");
    let moves = pending_moves(&mb, &cache, ACCOUNT);
    assert_eq!(moves.len(), 1, "the snooze is still waiting to be pushed");
    assert_eq!(moves[0].to, RemoteState::Archived);
}

// -------------------------------------------------------------------- bodies

#[test]
fn a_body_download_clears_partial_and_survives_a_later_flag_change() {
    let cache = seeded();
    let mut mb = mailbox(Vec::new());
    let (state, provider) = FakeProvider::without_folders(vec![remote("m1", "t1", 1_790_000_000)]);
    drain(&mut mb, &cache, &provider, true);
    let id = local_id(&cache, "m1");

    let msg = mb.get(id).unwrap();
    assert!(msg.partial, "a header-only message starts partial");
    assert_eq!(msg.body, "snippet of m1");

    let req = body_request(&mb, &cache, id).expect("a body is needed");
    let (req, body) = run_body(&provider, req);
    let body = body.expect("body");
    assert_eq!(apply_body(&mut mb, &cache, &req, body), Ok(()));
    let msg = mb.get(id).unwrap();
    assert!(!msg.partial);
    assert_eq!(msg.body, "full body of m1");
    assert!(body_request(&mb, &cache, id).is_none(), "a downloaded body is not requested again");
    assert_eq!(bodies(&state), ["m1".to_owned()]);

    // A later label change must not put the snippet back.
    remote_set(&state, "m1", RemoteState::Trash);
    with_updates(&state, &[], &[("m1", flags(&state, "m1"))], &[]);
    round(&mut mb, &cache, &provider, true, &[]);
    let msg = mb.get(id).unwrap();
    assert_eq!(msg.state, TriageState::Deleted);
    assert_eq!(msg.body, "full body of m1", "the downloaded body survives");
    assert!(!msg.partial);

    // And it is still there after a restart.
    let (_, _, messages) = cache.load().unwrap();
    let restored = mailbox(messages);
    assert_eq!(restored.get(id).unwrap().body, "full body of m1");
}

#[test]
fn a_local_message_never_asks_for_a_body() {
    let cache = seeded();
    let mb = mailbox(vec![local(7, "personal")]);
    assert!(body_request(&mb, &cache, 7).is_none());
    assert!(body_request(&mb, &cache, 99).is_none());
}

// --------------------------------------------------------------- rate limits

#[test]
fn a_rate_limited_provider_stops_the_round_and_pauses() {
    let cache = seeded();
    let mut mb = mailbox(Vec::new());
    let (state, provider) = FakeProvider::without_folders(vec![remote("i1", "t1", 1_790_000_000)]);
    round(&mut mb, &cache, &provider, true, &[]);
    state.lock().fail_list = Some(ProviderError::RateLimited);

    let summary = round(&mut mb, &cache, &provider, false, &[]);

    assert!(summary.throttled, "the loop pauses instead of toasting");
    assert_eq!(summary.error, None, "a rate limit is not a user-facing error");
    assert_eq!(lists(&state).len(), 1, "no further pages are tried");
    assert!(mb.messages().is_empty());
}

#[test]
fn a_network_error_stops_the_round_and_is_reported() {
    let cache = seeded();
    let mut mb = mailbox(Vec::new());
    let (state, provider) = FakeProvider::without_folders(vec![remote("i1", "t1", 1_790_000_000)]);
    round(&mut mb, &cache, &provider, true, &[]);
    state.lock().fail_list = Some(ProviderError::Network("offline".to_owned()));

    let summary = round(&mut mb, &cache, &provider, false, &[]);

    assert!(!summary.throttled);
    assert!(summary.error.unwrap().contains("offline"));
    assert_eq!(lists(&state).len(), 1);
}

// --------------------------------------------------------------- load-more

#[test]
fn scopes_follow_the_view_and_only_cached_accounts() {
    let cache = seeded();
    let mb = mailbox(Vec::new());

    assert_eq!(scopes_for(&mb, &cache, &Location::AllInboxes), [(ACCOUNT.to_owned(), Scope::Inbox)]);
    assert_eq!(scopes_for(&mb, &cache, &inbox_location()), [(ACCOUNT.to_owned(), Scope::Inbox)]);
    assert_eq!(
        scopes_for(&mb, &cache, &Location::Archive(ACCOUNT.to_owned())),
        [(ACCOUNT.to_owned(), Scope::Archive)]
    );
    assert_eq!(
        scopes_for(&mb, &cache, &Location::Trash(ACCOUNT.to_owned())),
        [(ACCOUNT.to_owned(), Scope::Trash)]
    );
    assert!(
        scopes_for(&mb, &cache, &Location::Snoozed(ACCOUNT.to_owned())).is_empty(),
        "snoozed mail is inbox mail locally"
    );
    assert!(scopes_for(&mb, &cache, &Location::Inbox("other".to_owned())).is_empty());
}

#[test]
fn a_folder_view_loads_more_under_its_label() {
    let cache = seeded();
    let mut mb = mailbox(Vec::new());
    let (state, provider) = FakeProvider::with_folders(
        vec![remote("f1", "t1", 1_790_000_000)],
        vec![RemoteFolder { id: "Label_5".to_owned(), path: "Work".to_owned() }],
    );
    remote_set(&state, "f1", RemoteState::Folder("Label_5".to_owned()));
    drain(&mut mb, &cache, &provider, true);

    let folder = mb.folders(ACCOUNT)[0].id;
    assert_eq!(
        scopes_for(&mb, &cache, &Location::Folder(folder)),
        [(ACCOUNT.to_owned(), Scope::Folder("Label_5".to_owned()))]
    );
    assert!(has_older(&cache, ACCOUNT, &Scope::Folder("Label_5".to_owned())));
}

// ---------------------------------------------------------------- persistence

#[test]
fn pending_states_and_snoozes_survive_a_restart() {
    let cache = seeded();
    let mut mb = mailbox(Vec::new());
    let (_state, provider) = FakeProvider::without_folders(vec![
        remote("m1", "t1", 1_790_000_000),
        remote("m2", "t2", 1_790_000_100),
    ]);
    drain(&mut mb, &cache, &provider, true);
    let ids = inbox(&mb);

    let clock = FakeClock::new(1_790_000_000);
    mb.snooze(&[ids[0]], clock.now() + 7_200, clock.now());
    mb.set_state(&[ids[1]], TriageState::Archived);
    persist_local(&mb, &cache);

    let (accounts, folders, messages) = cache.load().unwrap();
    assert_eq!(accounts, vec![account()]);
    assert!(folders.is_empty());
    let restored =
        Mailbox::from_parts(messages, accounts, folders, Rc::new(contacts::open_seeded_in_memory().unwrap()));
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
fn locally_created_mail_is_never_pushed() {
    let cache = seeded();
    let mut mb = mailbox(vec![local(3, ACCOUNT)]);
    let (state, provider) = FakeProvider::without_folders(Vec::new());

    assert!(
        pending_moves(&mb, &cache, ACCOUNT).is_empty(),
        "a message the provider never saw has nothing to push"
    );
    flush(&mut mb, &cache, &provider);
    assert!(moves(&state).is_empty());
}

#[test]
fn a_network_failure_stops_the_push_batch_and_keeps_the_rest_pending() {
    let cache = seeded();
    let mut mb = mailbox(Vec::new());
    let (state, provider) = FakeProvider::without_folders(vec![
        remote("m1", "t1", 1_790_000_000),
        remote("m2", "t2", 1_790_000_100),
    ]);
    drain(&mut mb, &cache, &provider, true);
    let ids = inbox(&mb);
    mb.set_state(&ids, TriageState::Archived);

    state.lock().fail_move = Some(ProviderError::Network("offline".to_owned()));
    let results = {
        let mut p = provider.lock().unwrap_or_else(|e| e.into_inner());
        run_moves(p.as_mut(), pending_moves(&mb, &cache, ACCOUNT))
    };
    assert_eq!(results.len(), 1, "the batch stops at the first failure");
    assert!(apply_moves(&mb, &cache, &results).unwrap().contains("offline"));
    assert_eq!(pending_moves(&mb, &cache, ACCOUNT).len(), 2);

    state.lock().fail_move = None;
    flush(&mut mb, &cache, &provider);
    assert_eq!(moves(&state).len(), 2);
    assert!(pending_moves(&mb, &cache, ACCOUNT).is_empty());
}

#[test]
fn a_re_listed_page_is_not_downloaded_twice() {
    let cache = seeded();
    let mut mb = mailbox(Vec::new());
    let (state, provider) = FakeProvider::without_folders(vec![remote("m1", "t1", 1_790_000_000)]);

    drain(&mut mb, &cache, &provider, true);
    let before = fetched_ids(&state);
    assert_eq!(before, ["m1".to_owned()]);
    cache.reset_recent(ACCOUNT).unwrap();
    round(&mut mb, &cache, &provider, false, &[]);
    assert_eq!(fetched_ids(&state), before, "a re-listed page is not downloaded again");
    assert_eq!(mb.messages().len(), 1);
}

#[test]
fn a_local_message_with_a_high_id_is_not_reused_by_an_import() {
    let cache = seeded();
    let mut mb = mailbox(vec![local(7, "personal")]);
    let (_state, provider) = FakeProvider::without_folders(vec![
        remote("m1", "t1", 1_790_000_000),
        remote("m2", "t2", 1_790_000_100),
    ]);

    drain(&mut mb, &cache, &provider, true);

    let ids = inbox(&mb);
    assert_eq!(ids.len(), 2, "both pulled messages land in the gmail inbox");
    assert!(ids.iter().all(|id| *id != 7), "local id 7 is untouched");
    assert_eq!(mb.get(7).map(|m| m.account.clone()), Some("personal".to_owned()));
}
