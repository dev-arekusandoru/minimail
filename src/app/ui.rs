//! Shared mouse-driven widgets: buttons that dispatch the very same actions the keys do.
use gpui_kit::component::ActiveTheme as _;

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    kbd::Kbd,
    tooltip::Tooltip,
    Sizable as _,
};
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
    let then = cx.theme().muted_foreground;
    let mut el = div().flex().items_center().gap_1();
    for (i, part) in key.split_whitespace().enumerate() {
        if i > 0 {
            el = el.child(div().text_size(px(KEYCAP_TEXT)).text_color(then).child("then"));
        }
        el = el.child(shortcut(part));
    }
    el
}

/// Small clickable button, a gpui-kit [`Button`]. `key` (a shortcut such as `"shift-e"`, or
/// empty) is shown in the tooltip as a keycap. Attach behavior with `.on_click(..)` or [`run`].
pub fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    tip: &str,
    key: &str,
    cx: &App,
) -> Button {
    let t = cx.theme();
    key_tooltip(
        Button::new(id)
            .label(label)
            .xsmall()
            .h(px(22.))
            .px_2()
            .text_size(px(11.))
            .border_color(t.border)
            .text_color(t.foreground),
        tip,
        key,
    )
}

/// Attach the app's tooltip — text plus the shortcut as a [`Kbd`] pill — to a kit [`Button`].
/// Uses GPUI's interaction tooltip so the kit Button keeps its own look.
pub fn key_tooltip<B: InteractiveElement>(mut el: B, tip: &str, key: &str) -> B {
    if !tip.is_empty() {
        let tip: SharedString = tip.to_owned().into();
        let key: SharedString = key.to_owned().into();
        el.interactivity().tooltip(move |window, cx| {
            let tooltip = Tooltip::new(tip.clone());
            let tooltip = if key.is_empty() {
                tooltip
            } else {
                tooltip.key_binding(Some(shortcut(&key)))
            };
            tooltip.build(window, cx)
        });
    }
    el
}

/// Icon-only variant of [`button`] for toolbars; `tip`/`key` are the tooltip, so keep them set.
pub fn icon_button(
    id: impl Into<ElementId>,
    icon: IconName,
    tip: &str,
    key: &str,
    cx: &App,
) -> Button {
    let t = cx.theme();
    key_tooltip(
        Button::new(id)
            .icon(icon)
            .xsmall()
            .h(px(22.))
            .w(px(24.))
            .px_0()
            .text_color(t.foreground),
        tip,
        key,
    )
}

/// Accent-filled primary action: an icon, plus `label` unless it is empty (icon-only).
pub fn primary_button(
    id: impl Into<ElementId>,
    icon: IconName,
    label: &str,
    tip: &str,
    key: &str,
    _cx: &App,
) -> Button {
    let mut b = Button::new(id)
        .icon(icon)
        .primary()
        .xsmall()
        .h(px(22.))
        .px_2()
        .text_size(px(11.))
        .gap_1();
    if label.is_empty() {
        b = b.w(px(24.)).px_0();
    } else {
        b = b.label(label.to_owned());
    }
    key_tooltip(b, tip, key)
}
