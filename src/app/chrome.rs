//! Stateless chrome: hint bar, help overlay, view tabs, empty state.

use crate::app::actions::commands;
use crate::app::ui::shortcut_chips;
use crate::clock::{Timestamp, DAY};
use crate::hints::{fit_hints, HintContext, HintMode};
use crate::theme::{self, Theme};
use gpui_kit::{
    assets::IconName,
    component::{button::{Button, ButtonVariants as _}, label::Label, separator::Separator, Sizable as _},
    prelude::FluentBuilder as _,
    *,
};

fn hint(t: &Theme, key: &str, what: &str, cx: &App) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_1()
        .child(shortcut_chips(key, cx))
        .child(div().text_xs().text_color(t.text_muted).child(SharedString::from(what.to_owned())))
}

/// Weekday names indexed by `days.rem_euclid(7)` for a Unix-epoch day count (0 = Thursday).
const WEEKDAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// UTC wall-clock label for a return time, e.g. `"Mon 5 Oct 08:00"`.
pub fn format_time(ts: Timestamp) -> String {
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

/// Civil `(year, month, day)` for a Unix-epoch day count (Howard Hinnant's algorithm).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    (year, month as u32, day as u32)
}

/// Humanized UTC label for `ts` relative to `now`, e.g. `"Today 14:32"`, `"Yesterday 09:10"`,
/// `"Mon 08:00"` (within the last six days), `"5 Oct"` (this year) or `"5 Oct 2025"`.
/// Future instants fall back to the absolute [`format_time`].
pub fn humanize_time(ts: Timestamp, now: Timestamp) -> String {
    if ts > now {
        return format_time(ts);
    }
    let ts_day = ts.div_euclid(DAY);
    let now_day = now.div_euclid(DAY);
    let secs = ts.rem_euclid(DAY);
    let hhmm = format!("{:02}:{:02}", secs / 3600, secs % 3600 / 60);
    match now_day - ts_day {
        0 => format!("Today {hhmm}"),
        1 => format!("Yesterday {hhmm}"),
        2..=6 => format!("{} {hhmm}", WEEKDAYS[ts_day.rem_euclid(7) as usize]),
        _ => {
            let (year, month, day) = civil(ts_day);
            let (now_year, ..) = civil(now_day);
            if year == now_year {
                format!("{day} {}", MONTHS[(month - 1) as usize])
            } else {
                format!("{day} {} {year}", MONTHS[(month - 1) as usize])
            }
        }
    }
}

#[derive(IntoElement)]
pub struct HintBar {
    ctx: HintContext,
}

impl HintBar {
    pub fn new(ctx: HintContext) -> Self {
        Self { ctx }
    }
}

/// Horizontal padding (both sides) and gap before the first hint, in px.
const BAR_PAD: f32 = 24.;
/// Estimated width of the "N selected" label, in px.
const SELECTED_LABEL_W: f32 = 80.;

impl RenderOnce for HintBar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = theme::active(cx);
        let selected = match self.ctx.mode {
            HintMode::Selection(n) => Some(n),
            _ => None,
        };
        let budget = f32::from(window.viewport_size().width) - BAR_PAD;
        let reserved = if selected.is_some() { SELECTED_LABEL_W } else { 0. };
        let shown = fit_hints(&self.ctx, budget, reserved);
        div()
            .flex()
            .items_center()
            .gap_3()
            .px_3()
            .h(px(28.))
            .flex_none()
            .overflow_hidden()
            .border_t_1()
            .border_color(t.border)
            .bg(t.sidebar)
            .when_some(selected, |el, n| {
                el.child(div().text_xs().text_color(t.accent).child(format!("{n} selected")))
            })
            .children(shown.iter().map(|h| hint(&t, h.key, h.label, cx).into_any_element()))
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
                        .when(!c.key.is_empty(), |el| el.child(shortcut_chips(c.key, cx)))
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
                        Button::new("help-close")
                            .icon(IconName::Close)
                            .ghost()
                            .xsmall()
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

#[cfg(test)]
mod tests {
    use super::{civil, format_time, humanize_time};

    #[test]
    fn formats_utc_weekday_date_and_time() {
        assert_eq!(format_time(0), "Thu 1 Jan 00:00");
        assert_eq!(format_time(1_000_000_000), "Sun 9 Sep 01:46");
        // Leap day, and a pre-epoch instant.
        assert_eq!(format_time(1_709_164_800 + 8 * 3600), "Thu 29 Feb 08:00");
        assert_eq!(format_time(-60), "Wed 31 Dec 23:59");
    }

    /// `2026-10-05 14:32 UTC` — a Monday.
    const NOW: i64 = 1_791_210_720;

    #[test]
    fn humanizes_each_bucket() {
        assert_eq!(humanize_time(1_791_191_400, NOW), "Today 09:10"); // 5 Oct 09:10
        assert_eq!(humanize_time(1_791_105_000, NOW), "Yesterday 09:10"); // 4 Oct 09:10
        assert_eq!(humanize_time(1_790_838_000, NOW), "Thu 07:00"); // 1 Oct, 4 days
        assert_eq!(humanize_time(1_790_755_200, NOW), "Wed 08:00"); // 30 Sep, 5 days
        assert_eq!(humanize_time(1_790_668_800, NOW), "Tue 08:00"); // 29 Sep, exactly 6 days
        assert_eq!(humanize_time(1_790_582_400, NOW), "28 Sep"); // 28 Sep, exactly 7 days
        assert_eq!(humanize_time(1_785_571_200, NOW), "1 Aug"); // this year, older
        assert_eq!(humanize_time(1_759_651_200, NOW), "5 Oct 2025"); // previous year
    }

    #[test]
    fn humanize_buckets_are_day_based() {
        // 00:05 vs the previous 23:55 is "Yesterday", not "Today".
        let midnight = 1_791_158_400; // 2026-10-05 00:00
        assert_eq!(humanize_time(midnight, midnight + 300), "Today 00:00");
        assert_eq!(humanize_time(1_791_158_100, midnight + 300), "Yesterday 23:55");
    }

    #[test]
    fn humanize_crosses_the_year_boundary() {
        let new_year = 1_767_261_600; // 2026-01-01 10:00, a Thursday
        assert_eq!(humanize_time(1_767_222_000, new_year), "Yesterday 23:00"); // 31 Dec
        assert_eq!(humanize_time(1_767_081_600, new_year), "Tue 08:00"); // 30 Dec, 2 days
        assert_eq!(humanize_time(1_766_649_600, new_year), "25 Dec 2025"); // 7 days, last year
    }

    #[test]
    fn humanize_falls_back_to_absolute_for_the_future() {
        assert_eq!(humanize_time(NOW + 3600, NOW), "Mon 5 Oct 15:32");
    }

    #[test]
    fn civil_dates_are_exact() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(20_731), (2026, 10, 5));
        assert_eq!(civil(-1), (1969, 12, 31));
    }
}
