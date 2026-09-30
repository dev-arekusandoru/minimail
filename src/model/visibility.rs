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
    /// Mail you sent is never from a new sender.
    pub fn is_new_sender(&self, id: MessageId) -> bool {
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
    pub fn ids_in_view(&self, view: &View) -> Vec<MessageId> {
        self.newest_first
            .iter()
            .copied()
            .filter(|id| {
                let Some(m) = self.get(*id) else {
                    return false;
                };
                if self.is_hidden_msg(m) {
                    return false;
                }
                let location_ok = match &view.location {
                    Location::AllInboxes => !m.outgoing && m.state == TriageState::Inbox,
                    Location::Inbox(a) => {
                        !m.outgoing && m.account == *a && m.state == TriageState::Inbox
                    }
                    Location::Snoozed(a) => m.account == *a && m.state == TriageState::Snoozed,
                    Location::Sent(a) => {
                        m.account == *a && m.outgoing && m.state != TriageState::Deleted
                    }
                    Location::Archive(a) => m.account == *a && m.state == TriageState::Archived,
                    Location::Trash(a) => m.account == *a && m.state == TriageState::Deleted,
                    Location::Folder(f) => m.state == TriageState::Filed(*f),
                };
                location_ok && view.filter.account.as_ref().is_none_or(|a| &m.account == a)
            })
            .filter(|id| {
                let tags = self.tags(*id);
                let matches = |t: TagFilter| match t {
                    TagFilter::NewSender => self.is_new_sender(*id),
                    TagFilter::NeedsReply => tags.contains(&Tag::NeedsReply),
                    TagFilter::AwaitingReply => tags.contains(&Tag::AwaitingReply),
                    TagFilter::FollowUp => tags.contains(&Tag::FollowUp),
                    TagFilter::Reminder => tags.contains(&Tag::Reminder),
                    TagFilter::PossibleSpam => tags.contains(&Tag::PossibleSpam),
                    TagFilter::Urgent => tags.iter().any(|x| matches!(x, Tag::Urgent(_))),
                };
                view.filter.tags.iter().all(|t| matches(*t))
                    && view
                        .filter
                        .kind
                        .is_none_or(|k| tags.contains(&Tag::Kind(k)))
            })
            .filter(|id| {
                !matches!(&view.location, Location::AllInboxes | Location::Inbox(_))
                    || match view.chip {
                        Chip::All => true,
                        Chip::NeedsReply => self.tags(*id).contains(&Tag::NeedsReply),
                        Chip::FollowUp => self.tags(*id).contains(&Tag::FollowUp),
                        Chip::Urgent => self.tags(*id).iter().any(|x| matches!(x, Tag::Urgent(_))),
                        Chip::NewSenders => self.is_new_sender(*id),
                        Chip::PossibleSpam => self.tags(*id).contains(&Tag::PossibleSpam),
                    }
            })
            .collect()
    }
    pub fn count_at(&self, location: &Location) -> usize {
        self.ids_in_view(&View {
            location: location.clone(),
            ..View::default()
        })
        .len()
    }
    pub fn follow_up_timeout(&self) -> Timestamp {
        self.follow_up_timeout
    }
    pub fn set_follow_up_timeout(&mut self, secs: Timestamp) {
        self.follow_up_timeout = secs.max(0);
    }
}
