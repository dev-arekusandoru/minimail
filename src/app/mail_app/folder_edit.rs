//! Renaming and deleting folders: the name prompt, the delete itself, and keeping the list
//! tabs and the sidebar pointing at folders that still exist.
//!
//! The model does the edit as one undo step ([`Mailbox::rename_folder`],
//! [`Mailbox::delete_folder`]); the delete's confirmation dialog lives with the other
//! confirms in `modals`.

use super::sidebar::folder_fold_key;
use super::*;
use crate::model::FolderNameError;
use crate::search::Field;

/// Where one list tab pointed before a folder change: the folder its query lists, with its
/// account and that query, and the folder its view was opened at.
pub(super) struct TabAnchor {
    listed: Option<(FolderId, AccountId, Query)>,
    opened_at: Option<(FolderId, AccountId)>,
}

impl MailApp {
    /// The folder `folder` names, else the one the list is showing; `None` when that is not a
    /// folder (or is gone).
    pub(super) fn target_folder(&self, folder: Option<FolderId>) -> Option<FolderId> {
        let id = folder.or_else(|| match self.location() {
            Some(Location::Folder(id)) => Some(id),
            _ => None,
        })?;
        self.mailbox.folder(id).map(|f| f.id)
    }

    /// `Rename…`: a prompt prefilled with the current name. Enter renames (a clash toasts and
    /// keeps the prompt), escape cancels.
    pub(super) fn open_rename_folder(
        &mut self,
        folder: Option<FolderId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.modal_open() {
            return;
        }
        let Some(id) = self.target_folder(folder) else { return };
        let Some(current) = self.mailbox.folder(id).map(|f| f.name.clone()) else { return };
        let prompt = cx.new(|cx| NamePrompt::new(format!("Rename “{current}”"), &current, window, cx));
        self._modal_sub = Some(cx.subscribe_in(
            &prompt,
            window,
            move |this, _, event: &NewFolderEvent, window, cx| {
                if let NewFolderEvent::Name(name) = event {
                    this.rename_folder_to(id, name, window, cx);
                }
            },
        ));
        self.new_folder = Some(prompt.clone().into());
        self.host_in_dialog(prompt.clone(), |vw| 420f32.min(vw * 0.9), window, cx);
        prompt.update(cx, |p, cx| p.focus(window, cx));
        cx.notify();
    }

    fn rename_folder_to(&mut self, id: FolderId, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let name = name.trim();
        let Some(old) = self.mailbox.folder(id).map(|f| f.name.clone()) else {
            return self.close_modals(window, cx);
        };
        let before = self.tab_anchors();
        match self.mailbox.rename_folder(id, name) {
            // A blank name keeps the prompt, as it does for a new folder.
            Err(FolderNameError::Blank) => {}
            Err(FolderNameError::Taken) => {
                self.show_toast(format!("A folder named “{name}” already exists here"), window, cx);
            }
            Err(FolderNameError::Missing) => self.close_modals(window, cx),
            Ok(()) => {
                self.close_modals(window, cx);
                if old != name {
                    self.follow_folders(before, window, cx);
                    self.show_undo_toast(format!("Renamed “{old}” to “{name}”"), window, cx);
                }
            }
        }
        cx.notify();
    }

    /// Delete the confirmed folder (and its subfolders) and tidy what pointed at it: the list
    /// moves to the account's Inbox when it was showing one of them, other tabs follow, and the
    /// sidebar forgets the folds and the rail pick it held for them.
    pub(super) fn delete_folder_now(&mut self, id: FolderId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(folder) = self.mailbox.folder(id).cloned() else { return };
        let tree = self.mailbox.folder_subtree(id);
        let before = self.tab_anchors();
        let showing = matches!(self.location(), Some(Location::Folder(f)) if tree.contains(&f))
            || matches!(self.folder(), Location::Folder(f) if tree.contains(f));
        let archived = self.mailbox.delete_folder(id);
        for gone in &tree {
            for in_all in [false, true] {
                self.sidebar_folds.remove(&folder_fold_key(*gone, in_all));
            }
        }
        if matches!(&self.sidebar_pick, Some((_, Location::Folder(f))) if tree.contains(f)) {
            self.sidebar_pick = None;
        }
        if showing {
            self.show_location(Location::Inbox(folder.account.clone()), false, cx);
        }
        self.follow_folders(before, window, cx);
        let moved = match archived {
            0 => String::new(),
            1 => " · 1 message archived".to_owned(),
            n => format!(" · {n} messages archived"),
        };
        self.show_undo_toast(format!("Deleted folder “{}”{moved}", folder.name), window, cx);
        self.scroll_to_cursor();
        cx.notify();
    }

    /// Note which folder every list tab lists and was opened at, before a change that can
    /// rename or remove folders (see [`Self::follow_folders`]).
    pub(super) fn tab_anchors(&self) -> Vec<TabAnchor> {
        let account_of = |id: FolderId| self.mailbox.folder(id).map(|f| (id, f.account.clone()));
        self.list_views
            .iter()
            .map(|view| TabAnchor {
                listed: match self.mailbox.query_location(&view.triage.query) {
                    Some(Location::Folder(id)) => {
                        account_of(id).map(|(id, account)| (id, account, view.triage.query.clone()))
                    }
                    _ => None,
                },
                opened_at: match view.folder {
                    Location::Folder(id) => account_of(id),
                    _ => None,
                },
            })
            .collect()
    }

    /// After a folder was renamed, deleted, or either undone: a tab listing a renamed folder
    /// follows it to its new path, and one whose folder is gone shows the account's Inbox
    /// instead (or closes, when that view is already open). A view opened at a gone folder
    /// highlights the Inbox row instead. Tabs the change did not touch are left alone.
    pub(super) fn follow_folders(&mut self, before: Vec<TabAnchor>, window: &mut Window, cx: &mut Context<Self>) {
        for (ix, anchor) in before.into_iter().enumerate().rev() {
            if ix >= self.list_views.len() {
                continue;
            }
            if let Some((id, account)) = &anchor.opened_at
                && self.mailbox.folder(*id).is_none()
            {
                self.list_views[ix].folder = Location::Inbox(account.clone());
            }
            let Some((id, account, was)) = anchor.listed else { continue };
            if self.list_views[ix].triage.query != was {
                continue;
            }
            let (query, gone) = match self.mailbox.folder(id) {
                Some(_) => {
                    let now = self.mailbox.location_query(&Location::Folder(id));
                    let (Some(old), Some(new)) = (was.values(Field::In).first(), now.values(Field::In).first()) else {
                        continue;
                    };
                    if old == new {
                        continue;
                    }
                    let mut query = was.clone();
                    query.replace(Field::In, old, new);
                    (query, false)
                }
                None => (self.mailbox.location_query(&Location::Inbox(account.clone())), true),
            };
            if !self.list_tabs.retarget(ix, query.clone()) {
                // That view is already open in another tab.
                self.close_list_tab(ix, window, cx);
            } else if gone {
                self.list_views[ix] = ListView::new(query, Location::Inbox(account));
            } else {
                self.list_views[ix].triage.query = query;
            }
        }
        cx.notify();
    }
}
