//! Shared mouse-driven widgets: buttons that dispatch the very same actions the keys do.

use gpui_kit::assets::IconName;
use gpui_kit::component::{Icon, Sizable as _, tooltip::Tooltip};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

/// A stateful div that headless tests can find by id (a plain `Stateful<Div>` in release builds).
pub type Observable = gpui_kit::base::ObservedElement<Stateful<Div>>;

/// Click handler that dispatches `action` through the focused element's key path, exactly like
/// pressing its shortcut.
pub fn run<A: Action + Clone>(
    action: A,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    move |_, window, cx| window.dispatch_action(Box::new(action.clone()), cx)
}

/// Monospace family for metadata. Read from the kit theme, which keeps it on a font the machine
/// has (GPUI panics when it lays text out in a missing family).
pub fn mono_font(cx: &App) -> SharedString {
    cx.try_global::<gpui_kit::component::theme::Theme>()
        .map_or_else(|| SharedString::from(".SystemUIFont"), |k| k.mono_font_family.clone())
}

/// Small clickable button. `key` (a shortcut such as `"shift-e"`, or empty) is shown in the
/// tooltip. Attach behavior with `.on_click(..)` or [`run`].
pub fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    tip: &str,
    key: &str,
    cx: &App,
) -> Observable {
    let t = crate::theme::active(cx);
    styled_button(id, label, tip, key, Fill { bg: None, border: t.border, fg: t.text, hover: t.hover }, cx)
}

/// Resting and hover colours of a [`styled_button`].
struct Fill {
    bg: Option<Hsla>,
    border: Hsla,
    fg: Hsla,
    hover: Hsla,
}

fn styled_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    tip: &str,
    key: &str,
    fill: Fill,
    cx: &App,
) -> Observable {
    let Fill { bg, border, fg, hover } = fill;
    let active = crate::theme::active(cx).selection;
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
        .when_some(bg, |d, bg| d.bg(bg))
        .text_size(px(11.))
        .text_color(fg)
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .active(move |s| s.bg(active))
        .when(!tip.is_empty(), |d| {
            d.tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        })
        .child(label)
        .test_support()
}

/// Icon-only variant of [`button`] for toolbars; `tip`/`key` are the tooltip, so keep them set.
pub fn icon_button(
    id: impl Into<ElementId>,
    icon: IconName,
    tip: &str,
    key: &str,
    cx: &App,
) -> Observable {
    button(id, "", tip, key, cx).px_0().w(px(26.)).justify_center().child(
        Icon::new(icon).with_size(px(14.)).text_color(crate::theme::active(cx).text),
    )
}

/// Accent-filled primary action: an icon, plus `label` unless it is empty (icon-only).
pub fn primary_button(
    id: impl Into<ElementId>,
    icon: IconName,
    label: &str,
    tip: &str,
    key: &str,
    cx: &App,
) -> Observable {
    let t = crate::theme::active(cx);
    let (accent, on_accent) = (t.accent, t.on_accent);
    let fill = Fill { bg: Some(accent), border: accent, fg: on_accent, hover: accent.opacity(0.85) };
    let mut b = styled_button(id, "", tip, key, fill, cx)
        .gap_1()
        .child(Icon::new(icon).with_size(px(14.)).text_color(on_accent));
    if label.is_empty() {
        b = b.px_0().w(px(26.)).justify_center();
    } else {
        b = b.child(label.to_owned());
    }
    b
}
