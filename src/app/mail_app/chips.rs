//! The chip row at the top of the list: quick single-select filters.
//!
//! Chips only make sense on an Inbox view, so the row (and the `1`–`6` keys behind it)
//! exists only there.

use super::*;
use super::sidebar::is_inbox_location;

/// The action the chip at index `i` of [`Chip::ALL`] runs (the same one its number key runs).
fn chip_action(i: usize) -> Box<dyn Action> {
    use crate::app::actions::{SelectChip1, SelectChip2, SelectChip3, SelectChip4, SelectChip5, SelectChip6};
    match i {
        0 => Box::new(SelectChip1),
        1 => Box::new(SelectChip2),
        2 => Box::new(SelectChip3),
        3 => Box::new(SelectChip4),
        4 => Box::new(SelectChip5),
        _ => Box::new(SelectChip6),
    }
}

impl MailApp {
    /// The chip row, or `None` on locations that are not Inbox views.
    pub(super) fn render_chips(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if self.mode != ListMode::State || !is_inbox_location(&self.triage.view.location) {
            return None;
        }
        let t = theme::active(cx);
        let active = self.triage.view.chip;
        let hover = t.hover;
        Some(
            div()
                .id("chip-row")
                .test_support()
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .px_3()
                .pb_2()
                .children(Chip::ALL.into_iter().enumerate().map(|(i, chip)| {
                    let on = chip == active;
                    div()
                        .id(("chip", i))
                        .test_support()
                        .flex_none()
                        .items_center()
                        .h(px(20.))
                        .px_2()
                        .rounded_full()
                        .border_1()
                        .border_color(if on { t.accent } else { t.border })
                        .bg(if on { t.selection } else { t.surface })
                        .text_size(px(11.))
                        .text_color(if on { t.accent } else { t.text_muted })
                        .cursor_pointer()
                        .when(!on, |d| d.hover(move |d| d.bg(hover)))
                        .on_click(move |_, window, cx| window.dispatch_action(chip_action(i), cx))
                        .child(chip.label())
                        .into_any_element()
                }))
                .into_any_element(),
        )
    }
}
