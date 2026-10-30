//! The header's recipient clauses: ghost words, their order, the chip `x`, and the subject
//! field's scroll.

use crate::harness::{Harness, harness};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, ElementId, TestAppContext};

/// Where an element sits, so clause order can be read off the layout: rows first, then the
/// position along the row, exactly the order a wrap preserves.
fn at(h: &mut Harness<'_>, id: impl Into<ElementId>) -> (i32, i32) {
    let id = id.into();
    let window = h.window;
    let bounds = h
        .cx
        .update_window(window, |_, window, _| window.try_find(id))
        .expect("window alive")
        .expect("element on screen")
        .bounds();
    (f32::from(bounds.origin.y) as i32, f32::from(bounds.origin.x) as i32)
}

/// A revealed Cc that is left empty goes back to its ghost word, and the clauses always read
/// To, Cc, Bcc — never the Bcc ghost before the Cc clause.
#[gpui_kit::gpui::test]
pub fn an_empty_cc_falls_back_to_its_ghost_in_order(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.new_draft();
    h.click("compose-show-cc");
    h.settle();
    assert!(h.has("compose-cc"), "the reveal shows the Cc clause");
    assert!(
        at(&mut h, "compose-cc") < at(&mut h, "compose-show-bcc"),
        "Cc comes before Bcc"
    );

    // Leave the field without committing anyone.
    h.click("compose-subject");
    h.settle();
    assert!(!h.has("compose-cc"), "an empty Cc does not stay on screen");
    assert!(h.has("compose-show-cc"), "its ghost word is back");
    assert!(h.has("compose-show-bcc"), "and so is Bcc's");
    assert!(
        at(&mut h, "compose-to") < at(&mut h, "compose-show-cc")
            && at(&mut h, "compose-show-cc") < at(&mut h, "compose-show-bcc"),
        "the ghosts stay in To, Cc, Bcc order"
    );
}

/// Clicking a chip is not a way to edit it: the chip stays, and what is typed after becomes
/// a second recipient.
#[gpui_kit::gpui::test]
pub fn clicking_a_chip_does_not_turn_it_into_a_field(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.new_draft();
    h.click("compose-to");
    h.type_text("maya@example.com,");
    h.settle();
    assert_eq!(h.draft_fields().1, "maya@example.com");

    h.click(("compose-chip", 0usize));
    h.settle();
    assert_eq!(
        h.draft_fields().1,
        "maya@example.com",
        "a click must not take the recipient out of the draft"
    );
    assert!(h.has(("compose-chip", 0usize)), "the chip is still a chip");

    h.type_text("jon@example.com,");
    h.settle();
    assert_eq!(
        h.draft_fields().1,
        "maya@example.com, jon@example.com",
        "typing adds a recipient; it does not replace the clicked one"
    );
}

/// The `x` on a chip removes that recipient from the draft.
#[gpui_kit::gpui::test]
pub fn the_chip_x_removes_the_recipient(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.new_draft();
    h.click("compose-to");
    h.type_text("maya@example.com, jon@example.com,");
    h.settle();
    assert_eq!(h.draft_fields().1, "maya@example.com, jon@example.com");

    // The `x` only shows on hover, so the pointer goes to the chip first.
    let window = h.window;
    h.cx.update_window(window, |_, window, cx| {
        window.hover(("compose-chip", 0usize), cx)
    })
    .expect("window alive");
    h.settle();
    h.click(("compose-chip-remove", 0usize));
    h.settle();
    assert_eq!(
        h.draft_fields().1,
        "jon@example.com",
        "removing the first chip drops only that recipient"
    );
    assert!(
        !h.has(("compose-chip", 1usize)),
        "the other chip moved into the free place"
    );
}

/// The subject field never scrolls sideways while its whole text fits, so no leading
/// character can disappear under the field's padding.
#[gpui_kit::gpui::test]
pub fn the_subject_never_scrolls_while_it_fits(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.new_draft();
    h.click("compose-subject");
    h.type_text("Quarterly planning notes for the launch");
    h.settle();
    let scroll = h.cx.read_entity(&h.app, |a, cx| a.subject_scroll(cx));
    assert_eq!(scroll, Some(0.), "the subject sits flushed left, unscrolled");
}
