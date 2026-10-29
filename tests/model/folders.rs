use crate::helpers::{State, assert_invariant, sample};
use mail_classifier::model::{FolderId, FolderNameError, Mailbox};

/// `Work` > `Reports` > `Q3`, plus a loose `Other`, all in the personal account, with mail
/// filed in `Work`, `Reports` and `Q3` (messages 1, 2 and 4) and one archived message left alone.
fn tree() -> (Mailbox, [FolderId; 4]) {
    let mut mb = sample();
    let work = mb.create_folder("personal", "Work", None);
    let reports = mb.create_folder("personal", "Reports", Some(work));
    let q3 = mb.create_folder("personal", "Q3", Some(reports));
    let other = mb.create_folder("personal", "Other", None);
    mb.set_state(&[1], State::Filed(work));
    mb.set_state(&[2], State::Filed(reports));
    mb.set_state(&[4], State::Filed(q3));
    (mb, [work, reports, q3, other])
}

#[test]
fn renaming_trims_and_children_follow_in_one_undo_step() {
    let (mut mb, [work, reports, q3, _]) = tree();
    let depth = mb.undo_depth();

    assert_eq!(mb.rename_folder(work, "  Jobs "), Ok(()));

    assert_eq!(mb.folder(work).unwrap().name, "Jobs");
    assert_eq!(mb.folder_path(q3), "Jobs/Reports/Q3", "paths are derived, so children follow");
    assert_eq!(mb.undo_depth(), depth + 1);
    assert!(mb.undo());
    assert_eq!(mb.folder_path(reports), "Work/Reports");
    assert_eq!(mb.undo_depth(), depth);
}

#[test]
fn renaming_rejects_blank_and_sibling_clashes_without_a_step() {
    let (mut mb, [work, reports, _, other]) = tree();
    let depth = mb.undo_depth();

    assert_eq!(mb.rename_folder(work, "   "), Err(FolderNameError::Blank));
    assert_eq!(mb.rename_folder(work, "OTHER"), Err(FolderNameError::Taken), "case-insensitive");
    assert_eq!(mb.rename_folder(999, "Anything"), Err(FolderNameError::Missing));
    assert_eq!(mb.folder(work).unwrap().name, "Work");
    assert_eq!(mb.undo_depth(), depth, "a rejected rename leaves no undo step");

    // The same name is not a clash with itself, and another parent's children don't count.
    assert_eq!(mb.rename_folder(other, "other"), Ok(()));
    assert_eq!(mb.rename_folder(reports, "Other"), Ok(()));
}

#[test]
fn deleting_archives_the_mail_of_the_folder_and_its_subfolders_then_undo_restores_everything() {
    let (mut mb, [work, reports, q3, other]) = tree();
    let before_folders = mb.folders("personal").into_iter().cloned().collect::<Vec<_>>();
    let depth = mb.undo_depth();

    assert_eq!(mb.folder_message_count(work), 3);
    assert_eq!(mb.delete_folder(work), 3);

    for gone in [work, reports, q3] {
        assert!(mb.folder(gone).is_none());
    }
    assert!(mb.folder(other).is_some(), "an unrelated folder stays");
    for id in [1, 2, 4] {
        assert_eq!(mb.state_of(id), Some(State::Archived), "mail is archived, never deleted");
    }
    assert_eq!(mb.state_of(3), Some(State::Archived));
    assert_eq!(mb.undo_depth(), depth + 1, "one user action, one undo step");
    assert_invariant(&mb);

    assert!(mb.undo());
    assert_eq!(mb.undo_depth(), depth);
    let after: Vec<_> = mb.folders("personal").into_iter().cloned().collect();
    assert_eq!(after, before_folders, "same folders in the same order");
    assert_eq!(mb.state_of(1), Some(State::Filed(work)));
    assert_eq!(mb.state_of(2), Some(State::Filed(reports)));
    assert_eq!(mb.state_of(4), Some(State::Filed(q3)));
    assert_eq!(mb.state_of(3), Some(State::Archived));
}

#[test]
fn deleting_a_subfolder_leaves_its_parent() {
    let (mut mb, [work, reports, q3, _]) = tree();
    assert_eq!(mb.delete_folder(reports), 2);
    assert!(mb.folder(work).is_some());
    assert_eq!(mb.state_of(1), Some(State::Filed(work)));
    assert_eq!(mb.state_of(4), Some(State::Archived));
    assert!(mb.folder(q3).is_none());
    assert_eq!(mb.delete_folder(reports), 0, "already gone");
}

#[test]
fn a_deleted_folders_id_is_not_handed_out_again() {
    let (mut mb, [_, _, _, other]) = tree();
    mb.delete_folder(other);
    let fresh = mb.create_folder("personal", "Fresh", None);
    assert!(fresh > other, "the cache may still hold a row for the old id");
}

#[test]
fn folding_a_folder_refiles_its_mail_without_an_undo_step() {
    let (mut mb, [work, reports, q3, _]) = tree();
    let depth = mb.undo_depth();

    let moved = mb.fold_folder(reports, Some(work));

    assert_eq!(moved, vec![2]);
    assert_eq!(mb.state_of(2), Some(State::Filed(work)));
    assert_eq!(mb.folder(q3).unwrap().parent, Some(work), "subfolders follow the keeper");
    assert!(mb.folder(reports).is_none());
    assert_eq!(mb.undo_depth(), depth);
}
