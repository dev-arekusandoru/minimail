//! Stateless chrome: hint bar, help overlay, view tabs, empty state.
use gpui_kit::component::ActiveTheme as _;

use crate::app::actions::{HelpPageDown, HelpPageUp, SelectNext, SelectPrev, ToggleHelp, commands};
use crate::app::ui::shortcut_chips;
use crate::clock::Timestamp;
use crate::hints::{fit_hints, HintContext, HintMode};
use crate::theme::ThemeColor;
use crate::tz::{MONTHS, Now, WEEKDAYS};
use gpui_kit::{
    assets::IconName,
    component::{button::{Button, ButtonVariants as _}, label::Label, scroll::ScrollableElement as _, separator::Separator, status_bar::StatusBar, Sizable as _},
    prelude::FluentBuilder as _,
    *,
};

fn hint(t: &ThemeColor, key: &str, what: &str, cx: &App) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_1()
        .child(shortcut_chips(key, cx))
        .child(div().text_xs().text_color(t.muted_foreground).child(SharedString::from(what.to_owned())))
}

/// Wall-clock label for a return time on the user's own clock, e.g. `"Mon 5 Oct 08:00"`.
pub fn format_time(ts: Timestamp, now: &Now) -> String {
    let (days, _) = now.parts(ts);
    let (_, month, day) = now.civil(ts);
    format!(
        "{} {} {} {}",
        WEEKDAYS[Now::weekday(days)],
        day,
        MONTHS[(month - 1) as usize],
        now.hhmm(ts)
    )
}

/// Humanized label for `ts` relative to `now`, both on the user's own clock, e.g.
/// `"Today 14:32"`, `"Yesterday 09:10"`, `"Mon 08:00"` (within the last six days),
/// `"5 Oct"` (this year) or `"5 Oct 2025"`. Future instants fall back to the absolute
/// [`format_time`].
pub fn humanize_time(ts: Timestamp, now: &Now) -> String {
    if ts > now.at() {
        return format_time(ts, now);
    }
    let (ts_day, _) = now.parts(ts);
    let (now_day, _) = now.parts(now.at());
    let hhmm = now.hhmm(ts);
    match now_day - ts_day {
        0 => format!("Today {hhmm}"),
        1 => format!("Yesterday {hhmm}"),
        2..=6 => format!("{} {hhmm}", WEEKDAYS[Now::weekday(ts_day)]),
        _ => {
            let (year, month, day) = now.civil(ts);
            let (now_year, ..) = now.civil(now.at());
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

/// Horizontal padding and inter-item gaps the [`StatusBar`] spends before hints get width:
/// its `px_3` sides plus the two region gaps either side of the empty center.
const BAR_PAD: f32 = 40.;
/// Estimated width of the "N selected" label, in px.
const SELECTED_LABEL_W: f32 = 80.;

impl RenderOnce for HintBar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.theme();
        let selected = match self.ctx.mode {
            HintMode::Selection(n) => Some(n),
            _ => None,
        };
        let budget = f32::from(window.viewport_size().width) - BAR_PAD;
        let reserved = if selected.is_some() { SELECTED_LABEL_W } else { 0. };
        let shown = fit_hints(&self.ctx, budget, reserved);
        let hints = div()
            .flex()
            .items_center()
            .gap_3()
            .min_w_0()
            .overflow_hidden()
            .children(shown.iter().map(|h| hint(t, h.key, h.label, cx).into_any_element()));
        StatusBar::new()
            .h(px(28.))
            .px_3()
            .gap_3()
            .flex_none()
            .border_color(t.border)
            .bg(t.sidebar)
            .left(hints)
            .when_some(selected, |bar, n| {
                bar.right(div().text_xs().text_color(t.primary).child(format!("{n} selected")))
            })
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

/// Width of the help dialog in a window of `viewport_w` px.
pub fn help_width(viewport_w: f32) -> f32 {
    (viewport_w * 0.9).min(HELP_MAX_W)
}

/// Key context of the help panel (`?`, arrows/`j`/`k` and page keys, bound under it).
pub const HELP_CONTEXT: &str = "HelpPanel";

pub enum HelpEvent {
    Close,
}

/// Shortcut panel hosted in a kit `Dialog`: multi-column when wide, scrolling its body (via
/// the shared `scroll` handle) when the content is taller than the window. The dialog owns
/// the backdrop and `escape`; the panel owns the scroll keys.
pub struct HelpPanel {
    scroll: ScrollHandle,
    focus: FocusHandle,
}

impl HelpPanel {
    pub fn new(scroll: ScrollHandle, cx: &mut Context<Self>) -> Self {
        scroll.set_offset(point(px(0.), px(0.)));
        Self { scroll, focus: cx.focus_handle() }
    }

    fn scroll_by(&mut self, dy: f32, cx: &mut Context<Self>) {
        let max = f32::from(self.scroll.max_offset().y);
        let y = (f32::from(self.scroll.offset().y) - dy).clamp(-max, 0.);
        self.scroll.set_offset(point(px(0.), px(y)));
        cx.notify();
    }

    fn scroll_pages(&mut self, pages: f32, cx: &mut Context<Self>) {
        let page = f32::from(self.scroll.bounds().size.height) * 0.9;
        self.scroll_by(pages * page, cx);
    }
}

impl Focusable for HelpPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EventEmitter<HelpEvent> for HelpPanel {}

impl Render for HelpPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let size = window.viewport_size();
        let (vw, vh) = (f32::from(size.width), f32::from(size.height));
        let width = help_width(vw);
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
                        .text_color(t.foreground)
                        .child(Label::new(c.name))
                        .when(!c.key.is_empty(), |el| el.child(shortcut_chips(c.key, cx)))
                }))
        });
        div()
            .id("help-panel")
            .key_context(HELP_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|_, _: &ToggleHelp, _, cx| cx.emit(HelpEvent::Close)))
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.scroll_by(28., cx)))
            .on_action(cx.listener(|this, _: &SelectPrev, _, cx| this.scroll_by(-28., cx)))
            .on_action(cx.listener(|this, _: &HelpPageDown, _, cx| this.scroll_pages(1., cx)))
            .on_action(cx.listener(|this, _: &HelpPageUp, _, cx| this.scroll_pages(-1., cx)))
            .flex()
            .flex_col()
            .w_full()
            .max_h(px(vh * 0.8))
            .p(px(HELP_PAD))
            .gap_1()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_between()
                    .child(div().text_sm().text_color(t.primary).child("Keyboard"))
                    .child(
                        Button::new("help-close")
                            .icon(IconName::Close)
                            .ghost()
                            .xsmall()
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(HelpEvent::Close))),
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
                    .vertical_scrollbar(&self.scroll)
                    .child(div().flex().gap_4().children(columns))
                    .child(div().flex_none().child(Separator::horizontal()))
                    .child(div().text_sm().text_color(t.primary).child("Icon legend"))
                    .child(crate::app::icons::legend_view(t)),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{format_time, humanize_time};
    use crate::tz::{FixedZone, Now, Utc};
    use std::rc::Rc;

    /// `2026-10-05 14:32 UTC` — a Monday.
    const NOW: i64 = 1_791_210_720;

    fn utc() -> Now {
        Now::new(NOW, Rc::new(Utc))
    }

    /// UTC+13 (Auckland-ish), the far side of the date line from UTC-10.
    fn east13() -> Now {
        Now::new(NOW, Rc::new(FixedZone(13 * 3600)))
    }

    #[test]
    fn formats_the_weekday_date_and_time_on_the_users_clock() {
        let now = utc();
        assert_eq!(format_time(0, &now), "Thu 1 Jan 00:00");
        assert_eq!(format_time(1_000_000_000, &now), "Sun 9 Sep 01:46");
        // Leap day, and a pre-epoch instant.
        assert_eq!(format_time(1_709_164_800 + 8 * 3600, &now), "Thu 29 Feb 08:00");
        assert_eq!(format_time(-60, &now), "Wed 31 Dec 23:59");
    }

    #[test]
    fn a_local_zone_rolls_the_label_over_midnight() {
        let now = east13();
        // 5 Oct 14:32 UTC is already 6 Oct 03:32 on a UTC+13 clock.
        assert_eq!(format_time(NOW, &now), "Tue 6 Oct 03:32");
        assert_eq!(format_time(NOW - 12 * 3600, &now), "Mon 5 Oct 15:32");
        assert_eq!(format_time(-60, &now), "Thu 1 Jan 12:59");
    }

    #[test]
    fn humanizes_each_bucket() {
        let now = utc();
        assert_eq!(humanize_time(1_791_191_400, &now), "Today 09:10"); // 5 Oct 09:10
        assert_eq!(humanize_time(1_791_105_000, &now), "Yesterday 09:10"); // 4 Oct 09:10
        assert_eq!(humanize_time(1_790_838_000, &now), "Thu 07:00"); // 1 Oct, 4 days
        assert_eq!(humanize_time(1_790_755_200, &now), "Wed 08:00"); // 30 Sep, 5 days
        assert_eq!(humanize_time(1_790_668_800, &now), "Tue 08:00"); // 29 Sep, exactly 6 days
        assert_eq!(humanize_time(1_790_582_400, &now), "28 Sep"); // 28 Sep, exactly 7 days
        assert_eq!(humanize_time(1_785_571_200, &now), "1 Aug"); // this year, older
        assert_eq!(humanize_time(1_759_651_200, &now), "5 Oct 2025"); // previous year
    }

    #[test]
    fn humanize_buckets_are_day_based() {
        // 00:05 vs the previous 23:55 is "Yesterday", not "Today".
        let midnight = 1_791_158_400; // 2026-10-05 00:00
        let now = Now::new(midnight + 300, Rc::new(Utc));
        assert_eq!(humanize_time(midnight, &now), "Today 00:00");
        assert_eq!(humanize_time(1_791_158_100, &now), "Yesterday 23:55");
    }

    #[test]
    fn humanize_buckets_follow_the_local_day() {
        // On a UTC+13 clock the local day rolls over 11 hours before UTC's does.
        let midnight = 1_791_158_400; // 2026-10-05 00:00 UTC
        let now = Now::new(midnight, Rc::new(FixedZone(13 * 3600)));
        // 12 hours earlier is already 5 Oct 01:00 locally, the same local day.
        assert_eq!(humanize_time(midnight - 12 * 3600, &now), "Today 01:00");
        // 14 hours earlier is 4 Oct 23:00 locally: yesterday.
        assert_eq!(humanize_time(midnight - 14 * 3600, &now), "Yesterday 23:00");
    }

    #[test]
    fn humanize_crosses_the_year_boundary() {
        let new_year = 1_767_261_600; // 2026-01-01 10:00, a Thursday
        let now = Now::new(new_year, Rc::new(Utc));
        assert_eq!(humanize_time(1_767_222_000, &now), "Yesterday 23:00"); // 31 Dec
        assert_eq!(humanize_time(1_767_081_600, &now), "Tue 08:00"); // 30 Dec, 2 days
        assert_eq!(humanize_time(1_766_649_600, &now), "25 Dec 2025"); // 7 days, last year
    }

    #[test]
    fn humanize_falls_back_to_absolute_for_the_future() {
        let now = utc();
        assert_eq!(humanize_time(NOW + 3600, &now), "Mon 5 Oct 15:32");
    }
}
