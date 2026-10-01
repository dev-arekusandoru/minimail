//! The window titlebar: the kit `TitleBar` holding the sidebar toggle and view title, the
//! search box and the global buttons.
use gpui_kit::component::ActiveTheme as _;

use gpui_kit::assets::IconName;
use gpui_kit::component::TitleBar;
use gpui_kit::*;

use super::{MailApp, MenuKind};
use crate::app::actions::*;
use crate::app::ui::{button, icon_button, primary_button, run, shortcut};

/// Width kept clear on each side of the search box for the sidebar toggle and title (left, past
/// the traffic lights) and the global buttons (right).
const SEARCH_SIDE: f32 = 190.;

impl MailApp {
    pub(super) fn render_titlebar(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme();
        let searching = self.is_filtered();
        let title_fg = if window.is_window_active() { t.foreground } else { t.muted_foreground };
        let search_text = self.search_header().unwrap_or_else(|| "Search…".into());
        let view: SharedString = self.location_label().into();
        // The box is a kit outline `Button` (icon + placeholder/query label) that opens the
        // palette; the clear button, or the `/` keycap while idle, sits over its right edge.
        let search = div()
            .id("search-box")
            .test_support()
            .relative()
            .flex()
            .flex_none()
            .w(px(260.))
            .max_w_full()
            .min_w(px(80.))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                button("search-open", search_text, "Search mail", "/", cx)
                    .outline()
                    .icon(IconName::Search)
                    .w_full()
                    .justify_start()
                    .text_color(if searching { t.foreground } else { t.muted_foreground })
                    .on_click(run(OpenSearch)),
            )
            .child(
                div().absolute().right_1().top_0().bottom_0().flex().items_center().child(if searching {
                    icon_button("search-clear", IconName::Close, "Clear search", "escape", cx)
                        .on_click(|event, window, cx| {
                            run(ClearSelection)(event, window, cx);
                            cx.stop_propagation();
                        })
                        .into_any_element()
                } else {
                    shortcut("/").into_any_element()
                }),
            );

        let width = f32::from(window.viewport_size().width);
        let sidebar_icon = if self.sidebar_visible() { IconName::PanelLeftClose } else { IconName::PanelLeftOpen };
        let left = div().flex().flex_1().flex_basis(px(0.)).min_w_0().overflow_hidden().h_full().child(
            div()
                .flex()
                .items_center()
                .min_w_0()
                .h_full()
                .pl(px(if cfg!(target_os = "macos") && !window.is_fullscreen() { 80. } else { 12. }))
                .gap_2()
                .child(no_drag(
                    icon_button("btn-sidebar", sidebar_icon, "Toggle the sidebar", "cmd-b", cx).on_click(run(ToggleSidebar)),
                ))
                .pr_3()
                .text_size(px(12.))
                .text_color(title_fg)
                .child(div().min_w_0().overflow_hidden().text_ellipsis().whitespace_nowrap().child(view)),
        );
        let right_buttons = div()
            .flex()
            .flex_none()
            .items_center()
            .gap_3()
            .h_full()
            .pl_3()
            .pr_3()
            .child(no_drag(
                primary_button(
                    "btn-session",
                    IconName::Play,
                    if width >= 800. { "Triage" } else { "" },
                    "Start a triage session",
                    "t",
                    cx,
                )
                .on_click(run(StartSession)),
            ))
            .child(no_drag(
                icon_button("btn-layout", self.layout_icon(), "Stack the panes the other way", "alt-l", cx)
                    .on_click(run(TogglePaneLayout)),
            ))
            .child(no_drag(icon_button("tb-settings", IconName::Settings, "Settings", "cmd-,", cx).on_click(run(ToggleSettings))))
            .child(no_drag(self.menu_trigger(
                MenuKind::Global,
                icon_button("btn-more", IconName::Ellipsis, "More actions", "", cx),
                cx,
            )));
        let right = div()
            .flex()
            .flex_1()
            .flex_basis(px(0.))
            .min_w_0()
            .overflow_hidden()
            .justify_end()
            .h_full()
            .child(right_buttons);
        // The search box is laid over the whole bar rather than placed between the left and
        // right regions, so it is centred on the window whatever the kit's inset (fullscreen)
        // or platform window controls take. The overlay reserves `SEARCH_SIDE` on both sides
        // for the regions it must not cover, and has no handlers, so it never blocks a drag.
        // The kit `TitleBar` owns the bar itself (window move, double-click zoom, window
        // controls, background); its own left inset is dropped (`pl_0`) so `left` keeps the
        // traffic-light clearance.
        let titlebar = div()
            .id("mail-titlebar")
            .test_support()
            .relative()
            .flex_none()
            .child(TitleBar::new().pl_0().child(left).child(right))
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .px(px(SEARCH_SIDE))
                    .child(search),
            );
        titlebar.into_any_element()
    }
}

/// Wraps an interactive titlebar child so presses on it are not treated as a bar drag.
fn no_drag(child: impl IntoElement) -> Div {
    div().flex_none().on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).child(child)
}
