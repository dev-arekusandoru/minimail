use super::*;

impl Mailbox {
    pub(super) fn is_hidden_msg(&self, m: &Message) -> bool {
        self.muted.contains(&m.thread_id)
            || self
                .blocked
                .iter()
                .any(|email| email.eq_ignore_ascii_case(&m.from_email))
            || self
                .unsubscribed
                .iter()
                .any(|email| email.eq_ignore_ascii_case(&m.from_email))
    }
    pub(super) fn is_visible_msg(&self, m: &Message) -> bool {
        !self.is_hidden_msg(m)
    }
    pub fn is_hidden(&self, id: MessageId) -> bool {
        self.get(id).is_some_and(|m| self.is_hidden_msg(m))
    }
    pub fn mute_thread(&mut self, thread_id: u32) -> bool {
        if !self.muted.insert(thread_id) {
            return false;
        }
        self.push_undo(vec![Change::Muted(thread_id, false)]);
        true
    }
    pub fn is_muted(&self, thread_id: u32) -> bool {
        self.muted.contains(&thread_id)
    }
    pub fn ids_in(&self, state: TriageState) -> Vec<MessageId> {
        self.newest_first
            .iter()
            .copied()
            .filter(|id| {
                self.get(*id)
                    .is_some_and(|m| m.state == state && self.is_visible_msg(m))
            })
            .collect()
    }
    pub fn count(&self, state: TriageState) -> usize {
        self.messages
            .iter()
            .filter(|m| m.state == state && self.is_visible_msg(m))
            .count()
    }
    pub fn hidden_count(&self) -> usize {
        self.messages
            .iter()
            .filter(|m| self.is_hidden_msg(m))
            .count()
    }
    pub fn allow_sender(&mut self, email: &str) -> bool {
        if !crate::known_senders::KNOWN_SENDERS_ENABLED {
            return false;
        }
        let e = lower(email);
        if !self.known.insert(e.clone()) {
            return false;
        }
        let display_name = self
            .messages
            .iter()
            .find(|message| message.from_email.eq_ignore_ascii_case(&e))
            .map(|message| message.from_name.as_str());
        let created = self
            .contacts
            .as_ref()
            .and_then(|store| {
                store
                    .upsert_from_email(&e, display_name, ContactSource::Screener)
                    .ok()
            })
            .is_some_and(|(_, created)| created);
        self.push_undo(vec![Change::Known(e, false, created)]);
        true
    }
    pub fn block_sender(&mut self, email: &str, move_to: Option<TriageState>) -> usize {
        let e = lower(email);
        if self.blocked.contains(&e) {
            return 0;
        }
        self.blocked.insert(e.clone());
        let ids: Vec<_> = self
            .messages
            .iter()
            .filter(|m| {
                lower(&m.from_email) == e && move_to.is_some() && m.state == TriageState::Inbox
            })
            .map(|m| m.id)
            .collect();
        let mut changes = vec![Change::Blocked(e, false)];
        for id in &ids {
            if let Some(s) = move_to {
                changes.push(self.snapshot(*id));
                if let Some(&i) = self.index.get(id) {
                    self.messages[i].state = s;
                    let meta = self.meta.entry(*id).or_default();
                    meta.snoozed_until = None;
                    meta.awaiting_since = None;
                    meta.tags.retain(|t| {
                        !matches!(t, Tag::Reminder | Tag::FollowUp | Tag::PossibleSpam)
                    });
                }
            }
        }
        self.push_undo(changes);
        ids.len()
    }
    pub fn unblock_sender(&mut self, email: &str) -> bool {
        let e = lower(email);
        if !self.blocked.remove(&e) {
            return false;
        }
        self.push_undo(vec![Change::Blocked(e, true)]);
        true
    }
    pub fn blocked(&self) -> Vec<String> {
        let mut v: Vec<_> = self.blocked.iter().cloned().collect();
        v.sort();
        v
    }
    pub fn unsubscribe(&mut self, email: &str, move_to: Option<TriageState>) -> usize {
        let e = lower(email);
        if self.unsubscribed.iter().any(|u| lower(u) == e) {
            return 0;
        }
        self.unsubscribed.push(e.clone());
        let ids: Vec<_> = self
            .messages
            .iter()
            .filter(|m| {
                lower(&m.from_email) == e && move_to.is_some() && m.state == TriageState::Inbox
            })
            .map(|m| m.id)
            .collect();
        let mut changes = vec![Change::Unsubscribed(e)];
        for id in &ids {
            if let Some(s) = move_to {
                changes.push(self.snapshot(*id));
                if let Some(&i) = self.index.get(id) {
                    self.messages[i].state = s;
                    let meta = self.meta.entry(*id).or_default();
                    meta.snoozed_until = None;
                    meta.awaiting_since = None;
                    meta.tags.retain(|t| {
                        !matches!(t, Tag::Reminder | Tag::FollowUp | Tag::PossibleSpam)
                    });
                }
            }
        }
        self.push_undo(changes);
        ids.len()
    }
    pub fn unsubscribed(&self) -> &[String] {
        &self.unsubscribed
    }
    /// Mail you sent is never from a new sender. Always `false` while the
    /// known/unknown sender distinction is off (see [`crate::known_senders`]).
    pub fn is_new_sender(&self, id: MessageId) -> bool {
        if !crate::known_senders::KNOWN_SENDERS_ENABLED {
            return false;
        }
        self.get(id)
            .is_some_and(|m| !m.outgoing && !self.known.contains(&lower(&m.from_email)))
    }
    pub fn accounts(&self) -> &[Account] {
        &self.accounts
    }
    pub fn account(&self, id: &str) -> Option<&Account> {
        self.accounts.iter().find(|a| a.id == id)
    }
    pub fn folders(&self, account: &str) -> Vec<&Folder> {
        self.folders
            .iter()
            .filter(|f| f.account == account)
            .collect()
    }
    pub fn folder(&self, id: FolderId) -> Option<&Folder> {
        self.folders.iter().find(|f| f.id == id)
    }
    pub fn create_folder(
        &mut self,
        account: &str,
        name: &str,
        parent: Option<FolderId>,
    ) -> FolderId {
        let id = self.folders.iter().map(|f| f.id).max().unwrap_or(0) + 1;
        self.folders.push(Folder {
            id,
            account: account.to_owned(),
            name: name.to_owned(),
            parent,
        });
        self.push_undo(vec![Change::FolderPush(id)]);
        id
    }

    /// Create a folder in `account` and file `ids` into it as a single undo
    /// step (the folder creation and the moves revert together).
    /// Returns the new folder's id and how many messages changed.
    pub fn create_folder_and_file(
        &mut self,
        account: &str,
        name: &str,
        parent: Option<FolderId>,
        ids: &[MessageId],
    ) -> (FolderId, usize) {
        self.grouped(|mb| {
            let id = mb.create_folder(account, name, parent);
            (id, mb.set_state(ids, TriageState::Filed(id)))
        })
    }
    /// Ids of every visible message `query` matches, newest first. Muted, blocked and
    /// unsubscribed mail never matches. Relative dates resolve against `now`.
    pub fn ids_matching(&self, query: &Query, now: Timestamp) -> Vec<MessageId> {
        self.newest_first
            .iter()
            .copied()
            .filter(|id| {
                self.get(*id)
                    .is_some_and(|m| !self.is_hidden_msg(m) && query.matches(m, self, now))
            })
            .collect()
    }
    /// How many visible messages `location` holds.
    pub fn count_at(&self, location: &Location, now: Timestamp) -> usize {
        self.ids_matching(&self.location_query(location), now).len()
    }
    /// A folder's name including its parents, e.g. `"Projects/Northwind"`.
    pub fn folder_path(&self, id: FolderId) -> String {
        let mut names = Vec::new();
        let mut cursor = Some(id);
        while let Some(next) = cursor {
            let Some(folder) = self.folder(next) else {
                break;
            };
            names.push(folder.name.clone());
            cursor = folder.parent;
        }
        names.reverse();
        names.join("/")
    }
    /// The query that lists exactly `location`: an `in:` value, plus `account:` for every
    /// location that belongs to one account.
    pub fn location_query(&self, location: &Location) -> Query {
        let (place, account) = match location {
            Location::AllInboxes => ("inbox".to_owned(), None),
            Location::Inbox(a) => ("inbox".to_owned(), Some(a.as_str())),
            Location::Snoozed(a) => ("snoozed".to_owned(), Some(a.as_str())),
            Location::Sent(a) => ("sent".to_owned(), Some(a.as_str())),
            Location::Archive(a) => ("archived".to_owned(), Some(a.as_str())),
            Location::Trash(a) => ("deleted".to_owned(), Some(a.as_str())),
            Location::Folder(id) => (
                self.folder_path(*id),
                self.folder(*id).map(|f| f.account.as_str()),
            ),
        };
        let mut q = Query::default();
        q.add(Field::In, &place);
        if let Some(account) = account {
            q.add(Field::Account, account);
        }
        q
    }
    /// The location `query` names through its `in:` (and `account:`) values: exactly one of
    /// each at most, the account required for everything but All Inboxes. `None` for a
    /// query that is not anchored to one place (global search, several `in:` values).
    pub fn query_location(&self, query: &Query) -> Option<Location> {
        let [place] = query.values(Field::In) else {
            return None;
        };
        let account = match query.values(Field::Account) {
            [] => None,
            [a] => Some(a),
            _ => return None,
        };
        let account_id = |a: &String| {
            self.accounts
                .iter()
                .find(|x| x.id.to_lowercase() == *a)
                .map(|x| x.id.clone())
        };
        let account = match account {
            Some(a) => Some(account_id(a)?),
            None => None,
        };
        match (place.as_str(), account) {
            ("inbox", None) => Some(Location::AllInboxes),
            ("inbox", Some(a)) => Some(Location::Inbox(a)),
            ("snoozed", Some(a)) => Some(Location::Snoozed(a)),
            ("sent", Some(a)) => Some(Location::Sent(a)),
            ("archived", Some(a)) => Some(Location::Archive(a)),
            ("deleted", Some(a)) => Some(Location::Trash(a)),
            (_, None) => None,
            (folder, Some(a)) => {
                // A full path wins over a bare leaf name, so `in:projects/recruiting` picks
                // the nested folder when two folders share a name.
                let in_account = || self.folders.iter().filter(|f| f.account == a);
                in_account()
                    .find(|f| self.folder_path(f.id).to_lowercase() == folder)
                    .or_else(|| in_account().find(|f| f.name.to_lowercase() == folder))
                    .map(|f| Location::Folder(f.id))
            }
        }
    }
    pub fn follow_up_timeout(&self) -> Timestamp {
        self.follow_up_timeout
    }
    pub fn set_follow_up_timeout(&mut self, secs: Timestamp) {
        self.follow_up_timeout = secs.max(0);
    }
}
