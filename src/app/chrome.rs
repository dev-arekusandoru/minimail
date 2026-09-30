//! Stateless chrome: hint bar, help overlay, view tabs, empty state.

use crate::app::actions::commands;
use crate::clock::{Timestamp, DAY};
use crate::model::TriageState;
use crate::theme::{self, Theme};
use gpui_kit::{
    component::{kbd::Kbd, label::Label, separator::Separator},
    prelude::FluentBuilder as _,
    *,
};

fn kbd(key: &str) -> Kbd {
    Kbd::new(Keystroke::parse(key).unwrap_or_else(|_| Keystroke::parse("space").unwrap()))
        .appearance(false)
}

fn hint(t: &Theme, key: &str, what: &str) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_1()
        .child(kbd(key))
        .child(div().text_xs().text_color(t.text_muted).child(SharedString::from(what.to_owned())))
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
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme::active(cx);
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
            .border_color(t.border)
            .bg(t.sidebar)
            .when_some(
                match self.mode {
                    HintMode::Selection(n) => Some(n),
                    _ => None,
                },
                |el, n| {
                    el.child(
                        div()
                            .text_xs()
                            .text_color(t.accent)
                            .child(format!("{n} selected")),
                    )
                },
            )
            .children(hints.iter().map(|(k, w)| hint(&t, k, w).into_any_element()))
    }
}

/// Preferred width of a single help column, and of the whole panel when wide.
const HELP_COLUMN_W: f32 = 300.;
const HELP_MAX_W: f32 = 940.;
const HELP_PAD: f32 = 12.;

/// Number of help columns that fit in a panel of `panel_w` px.
pub fn help_columns(panel_w: f32) -> usize {
    (((panel_w - 2. * HELP_PAD) / HELP_COLUMN_W).floor() as usize).clamp(1, 3)
}

type CloseHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// Shortcut overlay panel: constrained to the window, multi-column when wide,
/// scrolling its body (via `scroll`) when the content is taller than the window.
#[derive(IntoElement)]
pub struct HelpOverlay {
    scroll: ScrollHandle,
    on_close: CloseHandler,
}

impl HelpOverlay {
    pub fn new(
        scroll: ScrollHandle,
        on_close: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self { scroll, on_close: Box::new(on_close) }
    }
}

impl RenderOnce for HelpOverlay {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme::active(cx);
        let size = window.viewport_size();
        let (vw, vh) = (f32::from(size.width), f32::from(size.height));
        let width = (vw * 0.9).min(HELP_MAX_W);
        let cols = help_columns(width);
        let specs = commands();
        let per_col = specs.len().div_ceil(cols).max(1);
        let columns = specs.chunks(per_col).map(|chunk| {
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap_1()
                .children(chunk.iter().map(|c| {
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .text_sm()
                        .text_color(t.text)
                        .child(Label::new(c.name))
                        .when(!c.key.is_empty(), |el| el.child(kbd(c.key)))
                }))
        });
        div()
            .id("help-panel")
            .flex()
            .flex_col()
            .w(px(width))
            .max_h(px(vh * 0.9))
            .p(px(HELP_PAD))
            .gap_1()
            .bg(t.surface)
            .border_1()
            .border_color(t.border)
            .rounded_md()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_between()
                    .child(div().text_sm().text_color(t.accent).child("Keyboard"))
                    .child(
                        div()
                            .id("help-close")
                            .px_2()
                            .text_sm()
                            .text_color(t.text_muted)
                            .cursor_pointer()
                            .child("✕")
                            .on_click(self.on_close),
                    ),
            )
            .child(div().flex_none().child(Separator::horizontal()))
            .child(
                div()
                    .id("help-body")
                    .flex()
                    .flex_col()
                    .gap_3()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(div().flex().gap_4().children(columns))
                    .child(div().flex_none().child(Separator::horizontal()))
                    .child(div().text_sm().text_color(t.accent).child("Icon legend"))
                    .child(crate::app::icons::legend_view(&t)),
            )
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

/// The action behind tab `i` (the same one its number key triggers).
fn tab_action(i: usize) -> Box<dyn Action> {
    use crate::app::actions::{ShowDone, ShowInbox, ShowLater, ShowScreener, ShowWaiting};
    match i {
        0 => Box::new(ShowInbox),
        1 => Box::new(ShowWaiting),
        2 => Box::new(ShowLater),
        3 => Box::new(ShowDone),
        _ => Box::new(ShowScreener),
    }
}

fn tab(t: &Theme, i: usize, name: &'static str, n: usize, active: bool, color: Hsla) -> impl IntoElement {
    let hover = t.hover;
    div()
        .id(("view-tab", i))
        .test_support()
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .px_2()
        .py_1()
        .rounded_sm()
        .text_sm()
        .text_color(if active { color } else { t.text })
        .hover(move |el| el.bg(hover))
        .cursor_pointer()
        .on_click(move |_, window, cx| window.dispatch_action(tab_action(i), cx))
        .when(active, |el| el.bg(t.selection))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(div().w(px(6.)).h(px(6.)).rounded_full().bg(color))
                .child(kbd(&(i + 1).to_string()))
                .child(name),
        )
        .child(div().text_xs().text_color(if active { color } else { t.text_muted }).child(n.to_string()))
}

impl RenderOnce for ViewTabs {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme::active(cx);
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
                    .text_color(t.text_muted)
                    .child(format!("{to_triage} to triage")),
            )
            .children(self.counts.iter().enumerate().map(|(i, &(state, n))| {
                let name = match state {
                    TriageState::Inbox => "To triage",
                    other => other.label(),
                };
                tab(&t, i, name, n, self.screener.is_none() && state == self.active, t.state_color(state))
            }))
            .when_some(self.screener, |el, (n, active)| {
                el.child(tab(&t, 4, "Screener", n, active, t.state_screener))
            })
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
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme::active(cx);
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
            .child(div().text_color(t.state_color(self.view)).child(title))
            .child(div().text_xs().text_color(t.text_muted).child(sub))
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
