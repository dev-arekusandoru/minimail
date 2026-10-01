//! Helpers for the kit popup menus in headless UI tests: rows have no ids of their own, so a
//! row is found by its label. A submenu is a second `popup-menu`; each is told apart by the
//! view that draws it.
#![allow(dead_code)]

use gpui_kit::base::test_support::snapshots;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, ElementId, TestAppContext, Window};

/// One row of an open popup menu: the view drawing the menu, its index and its label.
struct Row {
    menu: ElementId,
    ix: usize,
    label: String,
}

/// Every row of every open popup menu.
fn rows(window: &Window) -> Vec<Row> {
    let popup: ElementId = "popup-menu".into();
    snapshots(window)
        .into_iter()
        .filter_map(|row| {
            let path = row.path();
            let at = path.iter().rposition(|id| *id == popup)?;
            let ElementId::Integer(ix) = path.last()? else { return None };
            // popup-menu > items > MenuItemElement > <ix>: a row, not something inside one.
            if at == 0 || path.len() != at + 4 {
                return None;
            }
            Some(Row {
                menu: path[at - 1].clone(),
                ix: usize::try_from(*ix).ok()?,
                label: row.label()?.to_owned(),
            })
        })
        .collect()
}

/// Labels of the rows of every open popup menu.
pub fn row_labels(cx: &mut TestAppContext, window: AnyWindowHandle) -> Vec<String> {
    cx.update_window(window, |_, window, _| rows(window).into_iter().map(|r| r.label).collect())
        .expect("window alive")
}

/// Whether an open popup menu has a row labelled `label`.
pub fn has_row(cx: &mut TestAppContext, window: AnyWindowHandle, label: &str) -> bool {
    row_labels(cx, window).iter().any(|l| l == label)
}

/// Click the row labelled `label` of an open popup menu.
pub fn click_row(cx: &mut TestAppContext, window: AnyWindowHandle, label: &str) {
    cx.update_window(window, |_, window, cx| {
        let row = rows(window)
            .into_iter()
            .find(|r| r.label == label)
            .unwrap_or_else(|| panic!("no menu row labelled {label:?}"));
        window.within(row.menu).within("popup-menu").click(row.ix, cx);
    })
    .expect("window alive");
    cx.run_until_parked();
}

/// Hover the row labelled `label` of an open popup menu (this is what opens a submenu).
pub fn hover_row(cx: &mut TestAppContext, window: AnyWindowHandle, label: &str) {
    cx.update_window(window, |_, window, cx| {
        let row = rows(window)
            .into_iter()
            .find(|r| r.label == label)
            .unwrap_or_else(|| panic!("no menu row labelled {label:?}"));
        window.within(row.menu).within("popup-menu").hover(row.ix, cx);
    })
    .expect("window alive");
    cx.run_until_parked();
}
