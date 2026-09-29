//! Stateless chrome: hint bar, help overlay, view tabs, empty state.

use crate::app::actions::commands;
use crate::clock::{Timestamp, DAY};
use crate::judge::{QuestionKey, Suggestion};
use crate::model::{Tag, TriageState};
use gpui_kit::{
    component::{kbd::Kbd, label::Label, separator::Separator, tag::Tag as TagChip, Sizable as _},
    prelude::FluentBuilder as _,
    *,
};

const BG: u32 = 0x16171a;
const BORDER: u32 = 0x2a2c31;
const ROW_ACTIVE: u32 = 0x25272c;
const MUTED: u32 = 0x80838a;
const TEXT: u32 = 0xd9dadd;
const ACCENT: u32 = 0x7dd3a8;

fn kbd(key: &str) -> Kbd {
    Kbd::new(Keystroke::parse(key).unwrap_or_else(|_| Keystroke::parse("space").unwrap()))
        .appearance(false)
}

fn hint(key: &str, what: &str) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_1()
        .child(kbd(key))
        .child(div().text_xs().text_color(rgb(MUTED)).child(SharedString::from(what.to_owned())))
}

/// UTC wall-clock label for a return time, e.g. `"Mon 5 Oct 08:00"`.
pub fn format_time(ts: Timestamp) -> String {
    const WEEKDAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let days = ts.div_euclid(DAY);
    let secs = ts.rem_euclid(DAY);
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    format!(
        "{} {} {} {:02}:{:02}",
        WEEKDAYS[days.rem_euclid(7) as usize],
        day,
        MONTHS[(month - 1) as usize],
        secs / 3600,
        secs % 3600 / 60
    )
}

fn tag_label(tag: &Tag) -> String {
    match tag {
        Tag::NoReply => "no reply".into(),
        Tag::NeedsReply => "needs reply".into(),
        Tag::Spam => "spam".into(),
        Tag::Urgent(n) => format!("urgent {n}"),
        Tag::Kind(k) => k.label().into(),
    }
}

/// Label of a not-yet-accepted suggestion, always ending in `?`.
fn suggestion_label(s: &Suggestion) -> String {
    match s.key {
        QuestionKey::Spam => "spam?".into(),
        QuestionKey::NeedsReply => "reply?".into(),
        QuestionKey::SuggestedState => {
            s.state().map_or("→ state?".into(), |st| format!("→ {}?", st.label().to_lowercase()))
        }
        QuestionKey::Urgency => s.urgency().map_or("urgent?".into(), |n| format!("urgent {n}?")),
        QuestionKey::Kind => s.kind().map_or("kind?".into(), |k| format!("{}?", k.label())),
    }
}

/// Inline badge strip for a message row: accepted tags solid, pending suggestions dimmed with `?`.
pub fn badges(tags: &[Tag], pending: &[&Suggestion]) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_1()
        .children(
            tags.iter()
                .map(|t| TagChip::secondary().small().child(SharedString::from(tag_label(t))).into_any_element()),
        )
        .children(pending.iter().map(|s| {
            div()
                .opacity(0.55)
                .child(TagChip::secondary().small().outline().child(SharedString::from(suggestion_label(s))))
                .into_any_element()
        }))
}

/// Which keys the bottom bar advertises.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HintMode {
    List,
    /// Number of selected messages.
    Selection(usize),
    Reader,
    Compose,
    Palette,
    /// Triage session; `true` once the end card is showing.
    Session(bool),
    Screener,
    Settings,
    Rules,
    Snooze,
}

#[derive(IntoElement)]
pub struct HintBar {
    mode: HintMode,
}

impl HintBar {
    pub fn new(mode: HintMode) -> Self {
        Self { mode }
    }
}

impl RenderOnce for HintBar {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let hints: &[(&str, &str)] = match self.mode {
            HintMode::List => &[
                ("j", "next"),
                ("k", "prev"),
                ("x", "select"),
                ("e", "done"),
                ("w", "waiting"),
                ("l", "later"),
                ("r", "reply"),
                ("u", "undo"),
                ("cmd-k", "commands"),
                ("?", "help"),
            ],
            HintMode::Selection(_) => &[
                ("x", "toggle"),
                ("shift-j", "extend"),
                ("e", "done"),
                ("w", "waiting"),
                ("l", "later"),
                ("i", "inbox"),
                ("escape", "clear"),
                ("u", "undo"),
            ],
            HintMode::Reader => &[
                ("r", "reply"),
                ("e", "done"),
                ("w", "waiting"),
                ("l", "later"),
                ("j", "next"),
                ("u", "undo"),
            ],
            HintMode::Compose => &[("cmd-enter", "send"), ("escape", "cancel")],
            HintMode::Palette => &[("enter", "run"), ("up", "prev"), ("down", "next"), ("escape", "close")],
            HintMode::Session(false) => &[
                ("e", "done"),
                ("w", "waiting"),
                ("l", "later"),
                ("i", "inbox"),
                ("escape", "end"),
            ],
            HintMode::Session(true) => &[("escape", "close")],
            HintMode::Screener => &[("a", "allow"), ("b", "block"), ("j", "next"), ("k", "prev"), ("u", "undo")],
            HintMode::Settings => &[
                ("j", "next"),
                ("k", "prev"),
                ("space", "auto/review"),
                ("=", "threshold +"),
                ("-", "threshold -"),
                ("escape", "close"),
            ],
            HintMode::Rules => &[("j", "next"), ("k", "prev"), ("backspace", "revoke"), ("escape", "close")],
            HintMode::Snooze => &[("1", "tonight"), ("2", "tomorrow"), ("3", "monday"), ("4", "custom"), ("escape", "cancel")],
        };
        div()
            .flex()
            .items_center()
            .gap_3()
            .px_3()
            .h(px(28.))
            .border_t_1()
            .border_color(rgb(BORDER))
            .when_some(
                match self.mode {
                    HintMode::Selection(n) => Some(n),
                    _ => None,
                },
                |el, n| {
                    el.child(
                        div()
                            .text_xs()
                            .text_color(rgb(ACCENT))
                            .child(format!("{n} selected")),
                    )
                },
            )
            .children(hints.iter().map(|(k, w)| hint(k, w).into_any_element()))
    }
}

#[derive(IntoElement)]
pub struct HelpOverlay;

impl HelpOverlay {
    pub fn new() -> Self {
        Self
    }
}

impl Default for HelpOverlay {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for HelpOverlay {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .w(px(420.))
            .p_3()
            .gap_1()
            .bg(rgb(BG))
            .border_1()
            .border_color(rgb(BORDER))
            .rounded_md()
            .child(div().text_sm().text_color(rgb(ACCENT)).child("Keyboard"))
            .child(Separator::horizontal())
            .children(commands().into_iter().map(|c| {
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_sm()
                    .text_color(rgb(TEXT))
                    .child(Label::new(c.name))
                    .when(!c.key.is_empty(), |el| el.child(kbd(c.key)))
            }))
    }
}

#[derive(IntoElement)]
pub struct ViewTabs {
    active: TriageState,
    counts: [(TriageState, usize); 4],
    screener: Option<(usize, bool)>,
}

impl ViewTabs {
    pub fn new(active: TriageState, counts: [(TriageState, usize); 4]) -> Self {
        Self { active, counts, screener: None }
    }

    /// Adds the fifth "Screener" tab; when `active`, no state tab is highlighted.
    pub fn screener(mut self, count: usize, active: bool) -> Self {
        self.screener = Some((count, active));
        self
    }
}

fn tab(i: usize, name: &'static str, n: usize, active: bool) -> impl IntoElement {
    div()
        .id(("view-tab", i))
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .px_2()
        .py_1()
        .rounded_sm()
        .text_sm()
        .text_color(rgb(if active { ACCENT } else { TEXT }))
        .when(active, |el| el.bg(rgb(ROW_ACTIVE)))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(kbd(&(i + 1).to_string()))
                .child(name),
        )
        .child(div().text_xs().text_color(rgb(MUTED)).child(n.to_string()))
}

impl RenderOnce for ViewTabs {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let to_triage = self
            .counts
            .iter()
            .find(|(s, _)| *s == TriageState::Inbox)
            .map_or(0, |(_, n)| *n);
        div()
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .child(
                div()
                    .px_2()
                    .pb_2()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child(format!("{to_triage} to triage")),
            )
            .children(self.counts.iter().enumerate().map(|(i, &(state, n))| {
                let name = match state {
                    TriageState::Inbox => "To triage",
                    other => other.label(),
                };
                tab(i, name, n, self.screener.is_none() && state == self.active)
            }))
            .when_some(self.screener, |el, (n, active)| el.child(tab(4, "Screener", n, active)))
    }
}

#[derive(IntoElement)]
pub struct EmptyState {
    view: TriageState,
}

impl EmptyState {
    pub fn new(view: TriageState) -> Self {
        Self { view }
    }
}

impl RenderOnce for EmptyState {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let (title, sub) = match self.view {
            TriageState::Inbox => ("Inbox zero", "Everything is triaged."),
            TriageState::Waiting => ("Nothing waiting", "Replies you send land here."),
            TriageState::Later => ("Nothing for later", "Press l to defer a message."),
            TriageState::Done => ("Nothing done yet", "Press e to finish a message."),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_1()
            .child(div().text_color(rgb(ACCENT)).child(title))
            .child(div().text_xs().text_color(rgb(MUTED)).child(sub))
    }
}

#[cfg(test)]
mod tests {
    use super::format_time;

    #[test]
    fn formats_utc_weekday_date_and_time() {
        assert_eq!(format_time(0), "Thu 1 Jan 00:00");
        assert_eq!(format_time(1_000_000_000), "Sun 9 Sep 01:46");
        // Leap day, and a pre-epoch instant.
        assert_eq!(format_time(1_709_164_800 + 8 * 3600), "Thu 29 Feb 08:00");
        assert_eq!(format_time(-60), "Wed 31 Dec 23:59");
    }
}
