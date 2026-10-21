//! Triage mode's pure session logic: the options chosen on the Start screen, the queue they
//! build, the running [`Run`] with its tallies, and the [`SessionSummary`] shown at the end.
//!
//! Counting rules:
//! * Acting on the current message (archive, file, snooze, delete, reply sent, or anything
//!   else that takes it out of the queue) handles it: `handled` goes up by one and the matching
//!   [`Tally`] field by one, exactly once per message.
//! * Skip takes the current message out of the queue and bumps `skipped`; nothing is handled,
//!   the message keeps its state, and it never comes round again in this session. Each message
//!   is therefore skipped at most once, and `handled + skipped` is the session's queue size.
//! * Progress is `(handled + 1, queue length)`: the position of the message on screen in what
//!   is left of the queue.
//!
//! No UI types live here, so all of it is headlessly testable. Time is passed in, never read.

use crate::clock::Timestamp;
use crate::model::{Location, Mailbox, MessageId, Tag, TriageState};
use crate::tz::Now;

/// Which of the location's messages the session works through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionFilter {
    All,
    NeedsReply,
    FollowUp,
    Urgent,
    PossibleSpam,
}

impl SessionFilter {
    pub const ALL: [SessionFilter; 5] = [Self::All, Self::NeedsReply, Self::FollowUp, Self::Urgent, Self::PossibleSpam];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::NeedsReply => "Needs reply",
            Self::FollowUp => "Follow up",
            Self::Urgent => "Urgent",
            Self::PossibleSpam => "Possible spam",
        }
    }

    pub fn matches(self, mailbox: &Mailbox, id: MessageId) -> bool {
        let tags = mailbox.tags(id);
        match self {
            Self::All => true,
            Self::NeedsReply => tags.contains(&Tag::NeedsReply),
            Self::FollowUp => tags.contains(&Tag::FollowUp),
            Self::Urgent => tags.iter().any(|t| matches!(t, Tag::Urgent(_))),
            Self::PossibleSpam => tags.contains(&Tag::PossibleSpam),
        }
    }
}

/// How many messages one session takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionSize {
    N25,
    N50,
    N100,
    All,
}

impl SessionSize {
    pub const ALL: [SessionSize; 4] = [Self::N25, Self::N50, Self::N100, Self::All];

    pub fn label(self) -> &'static str {
        match self {
            Self::N25 => "25",
            Self::N50 => "50",
            Self::N100 => "100",
            Self::All => "All",
        }
    }

    fn limit(self) -> usize {
        match self {
            Self::N25 => 25,
            Self::N50 => 50,
            Self::N100 => 100,
            Self::All => usize::MAX,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionOrder {
    NewestFirst,
    OldestFirst,
}

impl SessionOrder {
    pub const ALL: [SessionOrder; 2] = [Self::NewestFirst, Self::OldestFirst];

    pub fn label(self) -> &'static str {
        match self {
            Self::NewestFirst => "Newest first",
            Self::OldestFirst => "Oldest first",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionOptions {
    pub location: Location,
    pub filter: SessionFilter,
    pub size: SessionSize,
    pub order: SessionOrder,
    /// Show the action suggestion card.
    pub suggestions: bool,
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            location: Location::AllInboxes,
            filter: SessionFilter::All,
            size: SessionSize::All,
            order: SessionOrder::NewestFirst,
            suggestions: true,
        }
    }
}

/// What the Start screen shows for the options chosen so far.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetupView {
    pub options: SessionOptions,
    /// Messages in the location, whatever the filter.
    pub total: usize,
    /// Messages the session would take: filtered, ordered, limited.
    pub queue_len: usize,
    /// How many of the location's messages each filter matches (the chips' counts).
    pub filter_counts: [(SessionFilter, usize); 5],
}

impl SetupView {
    pub fn new(mailbox: &Mailbox, options: SessionOptions, now: &Now) -> Self {
        let ids = location_ids(mailbox, &options.location, now);
        Self {
            total: ids.len(),
            queue_len: queue_of(mailbox, &options, ids.clone()).len(),
            filter_counts: SessionFilter::ALL
                .map(|f| (f, ids.iter().filter(|id| f.matches(mailbox, **id)).count())),
            options,
        }
    }
}

/// Newest first, like the list.
fn location_ids(mailbox: &Mailbox, location: &Location, now: &Now) -> Vec<MessageId> {
    mailbox.ids_matching(&mailbox.location_query(location), now)
}

fn queue_of(mailbox: &Mailbox, options: &SessionOptions, mut ids: Vec<MessageId>) -> Vec<MessageId> {
    ids.retain(|id| options.filter.matches(mailbox, *id));
    if options.order == SessionOrder::OldestFirst {
        ids.reverse();
    }
    ids.truncate(options.size.limit());
    ids
}

/// The messages `options` selects, in the order they are shown.
pub fn build_queue(mailbox: &Mailbox, options: &SessionOptions, now: &Now) -> Vec<MessageId> {
    queue_of(mailbox, options, location_ids(mailbox, &options.location, now))
}

/// How an acted-on message left the queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Archived,
    Filed,
    Snoozed,
    Deleted,
    Replied,
    /// Handled without a tally of its own (moved back to the inbox, muted).
    Other,
}

impl Outcome {
    pub fn of(state: TriageState) -> Self {
        match state {
            TriageState::Archived => Self::Archived,
            TriageState::Filed(_) => Self::Filed,
            TriageState::Snoozed => Self::Snoozed,
            TriageState::Deleted => Self::Deleted,
            TriageState::Inbox => Self::Other,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tally {
    pub archived: usize,
    pub filed: usize,
    pub snoozed: usize,
    pub deleted: usize,
    pub replied: usize,
    pub skipped: usize,
    /// Handled messages whose action was the suggested one.
    pub suggestions_taken: usize,
}

impl Tally {
    fn record(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Archived => self.archived += 1,
            Outcome::Filed => self.filed += 1,
            Outcome::Snoozed => self.snoozed += 1,
            Outcome::Deleted => self.deleted += 1,
            Outcome::Replied => self.replied += 1,
            Outcome::Other => {}
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionSummary {
    pub handled: usize,
    pub elapsed_secs: i64,
    pub tally: Tally,
    /// Messages still in the session's location when it ended.
    pub remaining: usize,
}

impl SessionSummary {
    /// Average seconds per handled message (0 when none was).
    pub fn avg_secs(&self) -> i64 {
        self.elapsed_secs / self.handled.max(1) as i64
    }
}

/// A running session: the queue, how far it got, and what was done.
#[derive(Clone, Debug)]
pub struct Run {
    pub options: SessionOptions,
    /// Handled messages first, then the current one, then the rest.
    ids: Vec<MessageId>,
    handled: usize,
    started: Timestamp,
    tally: Tally,
}

impl Run {
    pub fn new(options: SessionOptions, ids: Vec<MessageId>, started: Timestamp) -> Self {
        Self { options, ids, handled: 0, started, tally: Tally::default() }
    }

    pub fn current(&self) -> Option<MessageId> {
        self.ids.get(self.handled).copied()
    }

    /// The current message and the next `n` still queued, in the order the session shows them.
    pub fn upcoming(&self, n: usize) -> &[MessageId] {
        let rest = &self.ids[self.handled.min(self.ids.len())..];
        &rest[..rest.len().min(n)]
    }

    /// `(1-based position, queue length)`.
    pub fn progress(&self) -> (usize, usize) {
        ((self.handled + 1).min(self.ids.len()), self.ids.len())
    }

    pub fn handled(&self) -> usize {
        self.handled
    }

    pub fn skipped(&self) -> usize {
        self.tally.skipped
    }

    pub fn is_finished(&self) -> bool {
        self.handled >= self.ids.len()
    }

    /// The current message was acted on; `taken` when the action was the suggested one.
    pub fn handle(&mut self, outcome: Outcome, taken: bool) {
        if self.is_finished() {
            return;
        }
        self.tally.record(outcome);
        self.tally.suggestions_taken += usize::from(taken);
        self.handled += 1;
    }

    /// Drop the current message from this session's queue: its mailbox state is untouched.
    pub fn skip(&mut self) {
        if self.is_finished() {
            return;
        }
        self.ids.remove(self.handled);
        self.tally.skipped += 1;
    }

    pub fn summary(&self, now: Timestamp, remaining: usize) -> SessionSummary {
        SessionSummary { handled: self.handled, elapsed_secs: now - self.started, tally: self.tally, remaining }
    }
}
