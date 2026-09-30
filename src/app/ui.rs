//! Shared mouse-driven widgets: buttons that dispatch the very same actions the keys do.

use gpui_kit::assets::IconName;
use gpui_kit::component::{kbd::Kbd, Icon, Sizable as _, tooltip::Tooltip};
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

/// Font size of keycap pills: one step down from the kit default, so a pill fits a 24–28px row.
const KEYCAP_TEXT: f32 = 10.;
/// Minimum width of a keycap pill, so a one-character key still reads as a key.
const KEYCAP_MIN_W: f32 = 14.;

/// The one keycap every shortcut display uses: a [`Kbd`] pill for a single keystroke
/// (`"e"`, `"shift-e"`, `"cmd-,"`). An unparseable key falls back to `space`, as the kit does.
pub fn shortcut(key: &str) -> Kbd {
    let stroke = Keystroke::parse(key).unwrap_or_else(|_| Keystroke::parse("space").unwrap());
    Kbd::new(stroke).text_size(px(KEYCAP_TEXT)).py_0().px(px(4.)).min_w(px(KEYCAP_MIN_W))
}

/// Keycaps for a possibly multi-stroke binding (`"g i"`): one pill per stroke, joined by
/// `then`, in the muted text colour the hint bar and help overlay use.
pub fn shortcut_chips(key: &str, cx: &App) -> Div {
    let then = crate::theme::active(cx).text_muted;
    let mut el = div().flex().items_center().gap_1();
    for (i, part) in key.split_whitespace().enumerate() {
        if i > 0 {
            el = el.child(div().text_size(px(KEYCAP_TEXT)).text_color(then).child("then"));
        }
        el = el.child(shortcut(part));
    }
    el
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
    let has_tip = !tip.is_empty();
    let tip: SharedString = tip.to_owned().into();
    let key: SharedString = key.to_owned().into();
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
        .when(has_tip, |d| {
            d.tooltip(move |window, cx| {
                let tooltip = Tooltip::new(tip.clone());
                let tooltip = if key.is_empty() {
                    tooltip
                } else {
                    tooltip.key_binding(Some(shortcut(&key)))
                };
                tooltip.build(window, cx)
            })
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
