//! Folder tree edits: rename, delete (both one undo step) and the load-time repair merge.

use super::*;

/// Why a folder could not be renamed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FolderNameError {
    Blank,
    /// A sibling already has the name (compared case-insensitively).
    Taken,
    Missing,
}

impl Mailbox {
    /// `id` and every folder below it, parents first; empty when `id` is unknown.
    pub fn folder_subtree(&self, id: FolderId) -> Vec<FolderId> {
        let mut out = Vec::new();
        if self.folder(id).is_none() {
            return out;
        }
        out.push(id);
        let mut at = 0;
        while at < out.len() {
            let parent = out[at];
            let kids: Vec<FolderId> = self
                .folders
                .iter()
                .filter(|f| f.parent == Some(parent) && !out.contains(&f.id))
                .map(|f| f.id)
                .collect();
            out.extend(kids);
            at += 1;
        }
        out
    }

    /// How many messages are filed in `id` or any folder below it.
    pub fn folder_message_count(&self, id: FolderId) -> usize {
        let tree = self.folder_subtree(id);
        self.messages
            .iter()
            .filter(|m| matches!(m.state, TriageState::Filed(f) if tree.contains(&f)))
            .count()
    }

    /// Rename `id` to the trimmed `new_name`. One undo step; descendants follow because paths
    /// are derived. Renaming to the same name is a no-op, not a clash with itself.
    pub fn rename_folder(&mut self, id: FolderId, new_name: &str) -> Result<(), FolderNameError> {
        let name = new_name.trim();
        if name.is_empty() {
            return Err(FolderNameError::Blank);
        }
        let Some(folder) = self.folder(id) else {
            return Err(FolderNameError::Missing);
        };
        if folder.name == name {
            return Ok(());
        }
        let clash = self.folders.iter().any(|o| {
            o.id != id
                && o.account == folder.account
                && o.parent == folder.parent
                && o.name.eq_ignore_ascii_case(name)
        });
        if clash {
            return Err(FolderNameError::Taken);
        }
        let old = folder.name.clone();
        if let Some(f) = self.folders.iter_mut().find(|f| f.id == id) {
            f.name = name.to_owned();
        }
        self.push_undo(vec![Change::FolderRename(id, old)]);
        Ok(())
    }

    /// Delete `id` and every folder below it. Mail filed in any of them is archived, never
    /// deleted. One undo step restores the folders and the mail's states. Returns how many
    /// messages were archived.
    pub fn delete_folder(&mut self, id: FolderId) -> usize {
        let doomed = self.folder_subtree(id);
        if doomed.is_empty() {
            return 0;
        }
        let filed: Vec<MessageId> = self
            .messages
            .iter()
            .filter(|m| matches!(m.state, TriageState::Filed(f) if doomed.contains(&f)))
            .map(|m| m.id)
            .collect();
        self.grouped(|mb| {
            let archived = mb.set_state(&filed, TriageState::Archived);
            let removed: Vec<(usize, Folder)> = mb
                .folders
                .iter()
                .enumerate()
                .filter(|(_, f)| doomed.contains(&f.id))
                .map(|(i, f)| (i, f.clone()))
                .collect();
            mb.folders.retain(|f| !doomed.contains(&f.id));
            mb.folder_floor = mb.folder_floor.max(doomed.iter().copied().max().unwrap_or(0));
            mb.push_undo(vec![Change::FolderRestore(removed)]);
            archived
        })
    }

    /// Never hand out an id at or below `upto` (the highest id the cache still holds).
    pub fn reserve_folder_ids(&mut self, upto: FolderId) {
        self.folder_floor = self.folder_floor.max(upto);
    }

    /// Remove `dup` as a repair, with no undo step: mail filed in it moves to `into` (archived
    /// when `None`) and its subfolders go to `into`, or to `dup`'s own parent when `None`.
    /// Returns the messages that changed state. Only for load-time repair: older undo steps
    /// could still name `dup`.
    pub fn fold_folder(&mut self, dup: FolderId, into: Option<FolderId>) -> Vec<MessageId> {
        let Some(parent) = self.folder(dup).map(|f| f.parent) else {
            return Vec::new();
        };
        let to = into.map_or(TriageState::Archived, TriageState::Filed);
        let mut moved = Vec::new();
        for m in &mut self.messages {
            if m.state == TriageState::Filed(dup) {
                m.state = to;
                moved.push(m.id);
            }
        }
        for f in &mut self.folders {
            if f.parent == Some(dup) {
                f.parent = into.or(parent);
            }
        }
        self.folders.retain(|f| f.id != dup);
        moved
    }
}
