//! Shared mouse-driven widgets: buttons that dispatch the very same actions the keys do.

use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

/// Click handler that dispatches `action` through the focused element's key path, exactly like
/// pressing its shortcut.
pub fn run<A: Action + Clone>(
    action: A,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    move |_, window, cx| window.dispatch_action(Box::new(action.clone()), cx)
}

/// Small clickable button. `key` (a shortcut such as `"shift-e"`, or empty) is shown in the
/// tooltip. Attach behavior with `.on_click(..)` or [`run`].
pub fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    tip: &str,
    key: &str,
    cx: &App,
) -> Stateful<Div> {
    let t = crate::theme::active(cx);
    let (border, fg, hover, active) = (t.border, t.text, t.hover, t.selection);
    let tip: SharedString = if key.is_empty() {
        tip.to_owned().into()
    } else {
        format!("{tip} ({key})").into()
    };
    let label: SharedString = label.into();
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .h(px(22.))
        .px_2()
        .rounded_sm()
        .border_1()
        .border_color(border)
        .text_size(px(11.))
        .text_color(fg)
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .active(move |s| s.bg(active))
        .when(!tip.is_empty(), |d| {
            d.tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        })
        .child(label)
}
