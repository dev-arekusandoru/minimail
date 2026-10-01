//! Entry points for syncing remote mail into a [`Mailbox`]. None push undo.

use super::*;

impl Mailbox {
    pub fn next_message_id(&self) -> MessageId {
        self.messages.iter().map(|m| m.id).max().map_or(1, |m| m + 1)
    }

    pub fn next_folder_id(&self) -> FolderId {
        self.folders.iter().map(|f| f.id).max().map_or(1, |m| m + 1)
    }

    pub fn next_thread_id(&self) -> u32 {
        self.messages.iter().map(|m| m.thread_id).max().map_or(1, |m| m + 1)
    }

    pub fn all_folders(&self) -> &[Folder] {
        &self.folders
    }

    /// Adds `account`, replacing any account with the same id.
    pub fn add_account(&mut self, account: Account) {
        match self.accounts.iter_mut().find(|a| a.id == account.id) {
            Some(slot) => *slot = account,
            None => self.accounts.push(account),
        }
    }

    pub fn upsert_folder(&mut self, folder: Folder) {
        match self.folders.iter_mut().find(|f| f.id == folder.id) {
            Some(slot) => *slot = folder,
            None => self.folders.push(folder),
        }
    }

    pub fn upsert_remote(&mut self, mut message: Message) {
        let until = message.snooze.as_deref().and_then(parse_rfc3339);
        if message.state == TriageState::Snoozed && until.is_none() {
            message.state = TriageState::Inbox;
        }
        let id = message.id;
        let state = message.state;
        let stamp = |m: &Message| m.received_at();
        let new_stamp = stamp(&message);
        if let Some(&i) = self.index.get(&id) {
            let old_stamp = stamp(&self.messages[i]);
            self.messages[i] = message;
            if old_stamp != new_stamp {
                self.newest_first.retain(|x| *x != id);
                self.insert_sorted(id, new_stamp);
            }
        } else {
            self.messages.push(message);
            self.index.insert(id, self.messages.len() - 1);
            self.insert_sorted(id, new_stamp);
        }
        let meta = self.meta.entry(id).or_default();
        if state == TriageState::Snoozed {
            meta.snoozed_until = until;
        } else {
            meta.snoozed_until = None;
            meta.awaiting_since = None;
        }
    }

    fn insert_sorted(&mut self, id: MessageId, stamp: Option<Timestamp>) {
        let key = |other: &MessageId| {
            self.index
                .get(other)
                .and_then(|i| parse_rfc3339(&self.messages[*i].received))
        };
        // `newest_first` is sorted by (stamp desc, id desc); find the first
        // entry that sorts after the new one.
        let pos = self.newest_first.partition_point(|other| {
            key(other).cmp(&stamp).then_with(|| other.cmp(&id)).is_gt()
        });
        self.newest_first.insert(pos, id);
    }

    pub fn remove_message(&mut self, id: MessageId) {
        let Some(&i) = self.index.get(&id) else {
            return;
        };
        self.messages.remove(i);
        self.index.clear();
        for (i, m) in self.messages.iter().enumerate() {
            self.index.entry(m.id).or_insert(i);
        }
        self.newest_first.retain(|x| *x != id);
        self.meta.remove(&id);
    }

    /// Removes the account's messages and folders (not the account itself).
    pub fn remove_account_data(&mut self, account: &str) {
        let ids: Vec<MessageId> = self
            .messages
            .iter()
            .filter(|m| m.account == account)
            .map(|m| m.id)
            .collect();
        self.messages.retain(|m| m.account != account);
        self.index.clear();
        for (i, m) in self.messages.iter().enumerate() {
            self.index.entry(m.id).or_insert(i);
        }
        for id in &ids {
            self.meta.remove(id);
        }
        self.newest_first.retain(|x| !ids.contains(x));
        self.folders.retain(|f| f.account != account);
    }

    pub fn remove_account(&mut self, id: &str) {
        self.accounts.retain(|a| a.id != id);
    }

    /// Set an account's icon key and `#rrggbb` color. Returns whether the account exists.
    pub fn set_account_style(&mut self, id: &str, icon: &str, color: &str) -> bool {
        let Some(account) = self.accounts.iter_mut().find(|a| a.id == id) else {
            return false;
        };
        account.icon = Some(icon.to_owned());
        account.color = color.to_owned();
        true
    }
}
