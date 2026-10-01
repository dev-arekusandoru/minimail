//! What the Accounts settings say about an account's sync: when it last completed, whether it is
//! running now, or why it is not working. Pure: callers pass `now` and the facts they know.

use crate::clock::{DAY, HOUR, MINUTE, Timestamp};

/// Prefix of [`crate::provider::ProviderError::Auth`]'s message, how a failed round reports that
/// the stored sign-in no longer works.
const AUTH_ERROR_PREFIX: &str = "authentication failed";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncStatus {
    /// A round is running.
    Syncing,
    /// The sign-in is missing or rejected; the user must re-authenticate the same account.
    SignInAgain,
    /// The last round failed with this message.
    Failed(String),
    /// The last completed server check, if any ever finished.
    Synced(Option<Timestamp>),
}

/// The one action the UI offers next to a status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncAction {
    /// Re-run sign-in for the same account.
    SignInAgain,
    /// Try the failed round again.
    Retry,
}

impl SyncStatus {
    /// What the user can do about this status; `None` when nothing is wrong or a round is running.
    pub fn action(&self) -> Option<SyncAction> {
        match self {
            SyncStatus::SignInAgain => Some(SyncAction::SignInAgain),
            SyncStatus::Failed(_) => Some(SyncAction::Retry),
            SyncStatus::Syncing | SyncStatus::Synced(_) => None,
        }
    }
}

/// What the app knows about one account's sync.
#[derive(Clone, Debug, Default)]
pub struct SyncFacts<'a> {
    /// When a server check last completed without error.
    pub last_synced: Option<Timestamp>,
    /// A round for the account is running.
    pub syncing: bool,
    /// The account has a working provider (a stored sign-in was found at startup).
    pub signed_in: bool,
    /// The error of the account's latest round, if it failed.
    pub error: Option<&'a str>,
}

pub fn is_auth_error(message: &str) -> bool {
    message.starts_with(AUTH_ERROR_PREFIX)
}

pub fn status(facts: &SyncFacts) -> SyncStatus {
    if !facts.signed_in || facts.error.is_some_and(is_auth_error) {
        return SyncStatus::SignInAgain;
    }
    if facts.syncing {
        return SyncStatus::Syncing;
    }
    match facts.error {
        Some(message) => SyncStatus::Failed(message.to_owned()),
        None => SyncStatus::Synced(facts.last_synced),
    }
}

/// The footer text for `status`, and whether it describes a problem.
pub fn describe(status: &SyncStatus, now: Timestamp) -> (String, bool) {
    match status {
        SyncStatus::Syncing => ("Syncing…".to_owned(), false),
        SyncStatus::SignInAgain => ("Sign-in expired".to_owned(), true),
        SyncStatus::Failed(message) => (format!("Sync failed: {message}"), true),
        SyncStatus::Synced(Some(at)) => (format!("Synced {}", ago(now - at)), false),
        SyncStatus::Synced(None) => ("Not synced yet".to_owned(), false),
    }
}

/// `secs` as a coarse age: "just now", "2 min ago", "3 h ago", "yesterday", "5 days ago".
/// A negative age (clock moved back) reads as "just now".
pub fn ago(secs: Timestamp) -> String {
    match secs {
        s if s < MINUTE => "just now".to_owned(),
        s if s < HOUR => format!("{} min ago", s / MINUTE),
        s if s < DAY => format!("{} h ago", s / HOUR),
        s if s < 2 * DAY => "yesterday".to_owned(),
        s => format!("{} days ago", s / DAY),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ProviderError;

    const NOW: Timestamp = 1_000_000;

    fn facts(last_synced: Option<Timestamp>) -> SyncFacts<'static> {
        SyncFacts { last_synced, syncing: false, signed_in: true, error: None }
    }

    fn text(facts: &SyncFacts) -> (String, bool) {
        describe(&status(facts), NOW)
    }

    #[test]
    fn a_completed_check_reads_as_its_age() {
        assert_eq!(text(&facts(Some(NOW - 30))), ("Synced just now".into(), false));
        assert_eq!(text(&facts(Some(NOW - 2 * MINUTE - 5))).0, "Synced 2 min ago");
        assert_eq!(text(&facts(Some(NOW - 3 * HOUR))).0, "Synced 3 h ago");
        assert_eq!(text(&facts(Some(NOW - DAY - HOUR))).0, "Synced yesterday");
        assert_eq!(text(&facts(Some(NOW - 5 * DAY))).0, "Synced 5 days ago");
        assert_eq!(text(&facts(Some(NOW + 90))).0, "Synced just now", "clock skew is not negative");
        assert_eq!(text(&facts(None)).0, "Not synced yet");
    }

    #[test]
    fn a_running_round_wins_over_the_last_time_but_not_over_a_dead_sign_in() {
        let running = SyncFacts { syncing: true, ..facts(Some(NOW - 5 * DAY)) };
        assert_eq!(text(&running), ("Syncing…".into(), false));
        let signed_out = SyncFacts { signed_in: false, ..running };
        assert_eq!(text(&signed_out), ("Sign-in expired".into(), true));
    }

    #[test]
    fn auth_failures_ask_to_sign_in_and_other_failures_show_the_message() {
        let auth = ProviderError::Auth("token revoked".into()).to_string();
        let rejected = SyncFacts { error: Some(&auth), ..facts(Some(NOW)) };
        assert_eq!(status(&rejected), SyncStatus::SignInAgain);
        let net = ProviderError::Network("offline".into()).to_string();
        let failing = SyncFacts { error: Some(&net), ..facts(Some(NOW)) };
        assert_eq!(text(&failing), (format!("Sync failed: {net}"), true));
    }

    #[test]
    fn only_broken_statuses_offer_an_action() {
        assert_eq!(SyncStatus::SignInAgain.action(), Some(SyncAction::SignInAgain));
        assert_eq!(SyncStatus::Failed("x".into()).action(), Some(SyncAction::Retry));
        assert_eq!(SyncStatus::Syncing.action(), None);
        assert_eq!(SyncStatus::Synced(Some(5)).action(), None);
        assert_eq!(SyncStatus::Synced(None).action(), None);
    }
}
