//! Compact rows: which cluster icons survive in a narrow list.
use mail_classifier::app::icons::Glyph::*;
use mail_classifier::compact::cluster_icons;

#[test]
fn wide_lists_keep_every_icon() {
    let all = [FolderSuggestion, PossibleSpam, Muted, Attachment, KindNewsletter];
    assert_eq!(cluster_icons(&all, false), all);
}

#[test]
fn narrow_lists_keep_the_kind_and_the_two_most_severe() {
    let all = [FolderSuggestion, PossibleSpam, UrgentMid, NewSender, Snoozed, Attachment, KindReceipt];
    // severity: spam, very urgent, new sender, urgent, attachment, folder, muted, snoozed
    assert_eq!(cluster_icons(&all, true), [PossibleSpam, NewSender, KindReceipt]);
}

#[test]
fn narrow_severity_order_below_the_top() {
    let all = [FolderSuggestion, Muted, Attachment, KindPerson];
    assert_eq!(cluster_icons(&all, true), [FolderSuggestion, Attachment, KindPerson]);
    let few = [Snoozed, KindOther];
    assert_eq!(cluster_icons(&few, true), few);
}
