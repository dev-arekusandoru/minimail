//! The sidebar's account rail: a fixed 56px column left of the folder panel (see
//! [`super::sidebar`]).
//!
//! Top to bottom: the compose button (disabled: there is no composer for new mail yet), a
//! divider, one tile per account (All first) in a region of its own that scrolls when there are
//! many, the add-account tile, then the tools pinned to the bottom — fetch mail, the
//! classifier, sender rules and settings. Account tiles only choose which account the panel
//! shows (see [`MailApp::pick_sidebar_account`]); they never move the list.
//!
//! The tools are ghost buttons with no fill, so they read as tools and not as accounts, and
//! each carries its state on the icon: a spinning primary icon while fetching, a dot for a
//! stale or failing sync, a progress ring while the classifier works.
use gpui_kit::component::ActiveTheme as _;

use super::panes::RAIL_W;
use super::sidebar::rail_bg;
use super::*;
use crate::app::ui::{Observable, key_tooltip};
use crate::sync_status::Attention;
use crate::theme::ThemeColor;
use gpui_kit::assets::IconName;
use gpui_kit::component::plot::shape::{Arc, ArcData};
use gpui_kit::component::{Icon, Sizable as _};

/// Side of a rail tile or tool button, and the radius of its corners.
const TILE: f32 = 38.;
const TILE_RADIUS: f32 = 10.;
/// The classifier ring: a 2px band around the button, between these two radii.
const RING_INNER: f32 = 15.;
const RING_OUTER: f32 = 17.;

/// A rail tile: a square with a tooltip, its content centred. `key` is the tooltip's shortcut.
fn tile(id: impl Into<ElementId>, tip: &str, key: &str) -> Observable {
    key_tooltip(div().id(id), tip, key)
        .test_support()
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .w(px(TILE))
        .h(px(TILE))
        .rounded(px(TILE_RADIUS))
}

/// A ghost tool button: no fill at rest, a muted icon that lifts to the foreground with the
/// secondary fill under the pointer, and the active fill while `active`.
fn tool_button(id: &'static str, tip: &str, key: &str, active: bool, t: &ThemeColor) -> Observable {
    tile(id, tip, key)
        .text_color(t.muted_foreground)
        .when(active, |d| d.bg(t.secondary_active).text_color(t.foreground))
        .when(!active, |d| d.hover(|s| s.bg(t.secondary).text_color(t.foreground)))
}

/// A status dot on a tool button's top-right corner, ringed in the rail colour.
fn status_dot(color: Hsla, t: &ThemeColor) -> Div {
    div()
        .absolute()
        .top(px(6.))
        .right(px(6.))
        .size(px(11.))
        .rounded_full()
        .bg(color)
        .border_2()
        .border_color(rail_bg(t))
}

/// A progress ring around a button: the whole circle in `track`, `done` of `total` of it in
/// `fill` from the top, clockwise.
fn progress_ring(done: usize, total: usize, track: Hsla, fill: Hsla) -> impl IntoElement {
    let share = if total == 0 { 0. } else { (done as f32 / total as f32).clamp(0., 1.) };
    canvas(
        |_, _, _| (),
        move |bounds, (), window, _| {
            let ring = Arc::new().inner_radius(RING_INNER).outer_radius(RING_OUTER);
            ring.paint(&ArcData::new(&(), 0, 100., 0., std::f32::consts::TAU), track, &bounds, window);
            if share > 0. {
                let end = share * std::f32::consts::TAU;
                ring.paint(&ArcData::new(&(), 1, share * 100., 0., end), fill, &bounds, window);
            }
        },
    )
    .absolute()
    .top(px(2.))
    .left(px(2.))
    .right(px(2.))
    .bottom(px(2.))
}

impl MailApp {
    /// The rail: compose, accounts, add-account, and the tools.
    pub(super) fn render_rail(&self, cx: &Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let now = self.local_now();
        let shown = self.sidebar_account();
        let all = self.account_tile(
            "sidebar-rail-all".into(),
            Icon::new(IconName::Mails).with_size(px(16.)).text_color(t.primary),
            "All Inboxes".to_owned(),
            shown.is_none(),
            self.mailbox.unread_count_at(&Location::AllInboxes, &now) > 0,
            None,
            cx,
        );
        let accounts = self.mailbox.accounts().iter().map(|a| {
            self.account_tile(
                format!("sidebar-rail-{}", a.id).into(),
                icons::account_icon(crate::account_style::icon_key(a), &a.color, t, 16.),
                crate::account_style::display_name(a).to_owned(),
                shown.as_deref() == Some(a.id.as_str()),
                self.mailbox.unread_count_at(&Location::Inbox(a.id.clone()), &now) > 0,
                Some(a.id.clone()),
                cx,
            )
        });
        let add = tile("sidebar-add-account", "Add or manage accounts", "")
            .border(px(1.5))
            .border_dashed()
            .border_color(t.sidebar_border)
            .text_color(t.muted_foreground)
            .child(Icon::new(IconName::Plus).with_size(px(16.)))
            .on_click(run(ManageAccounts));
        // The compose button is a placeholder until there is a composer for new mail.
        let compose = tile("sidebar-compose", "New message — coming soon", "")
            .bg(t.primary)
            .opacity(0.45)
            .child(Icon::new(IconName::SquarePen).with_size(px(18.)).text_color(t.primary_foreground));
        div()
            .w(px(RAIL_W))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(10.))
            .pt(px(12.))
            .pb(px(10.))
            .bg(rail_bg(t))
            .border_r_1()
            .border_color(t.sidebar_border)
            .child(compose)
            .child(div().w(px(24.)).h(px(1.)).flex_none().bg(t.sidebar_border))
            // Only the accounts scroll. The region reaches 6px up into the gap above it so a
            // badge hanging off the first tile is not clipped.
            .child(
                div()
                    .id("sidebar-rail-accounts")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .mt(px(-6.))
                    .pt(px(6.))
                    .overflow_y_scroll()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(10.))
                            .child(all)
                            .children(accounts)
                            .child(add),
                    ),
            )
            .child(self.rail_tools(cx))
    }

    /// An account tile: the account's icon, a dot on the corner while its inbox has unread mail
    /// (always, selected or not; real counts live in the panel), and the primary bar on the
    /// rail's edge while selected. `target` is the account it shows, `None` for All.
    #[allow(clippy::too_many_arguments)]
    fn account_tile(
        &self,
        id: ElementId,
        icon: Icon,
        name: String,
        selected: bool,
        unread: bool,
        target: Option<AccountId>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let t = cx.theme();
        let dot_id = ElementId::NamedChild(std::sync::Arc::new(id.clone()), "unread".into());
        tile(id, &name, "")
            .bg(if selected { t.secondary_active } else { t.secondary })
            .when(!selected, |d| d.hover(|s| s.bg(t.secondary_hover)))
            .child(icon)
            .when(selected, |d| {
                // The tile sits (rail width - 1px border - tile) / 2 in from the rail's left
                // edge; the bar starts on that edge.
                d.child(
                    div()
                        .absolute()
                        .left(px(-(RAIL_W - 1. - TILE) / 2.))
                        .top(px(10.))
                        .bottom(px(10.))
                        .w(px(3.))
                        .rounded_r(px(2.))
                        .bg(t.primary),
                )
            })
            .when(unread, |d| {
                // An 8px dot ringed by 2px of rail colour, so it cuts out of the tile.
                d.child(
                    div()
                        .id(dot_id)
                        .test_support()
                        .absolute()
                        .top(px(-4.))
                        .right(px(-4.))
                        .size(px(12.))
                        .rounded_full()
                        .bg(t.primary)
                        .border_2()
                        .border_color(rail_bg(t)),
                )
            })
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.pick_sidebar_account(target.clone(), cx);
            }))
            .into_any_element()
    }

    /// The tools pinned to the rail's bottom.
    fn rail_tools(&self, cx: &Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        div()
            .flex()
            .flex_col()
            .flex_none()
            .items_center()
            .gap(px(10.))
            .when(self.has_remote_accounts(), |d| d.child(self.sync_tool(cx)))
            .child(self.classifier_tool(cx))
            .child(
                tool_button("sidebar-rules", "Sender rules", "shift-r", self.rules_open(), t)
                    .child(Icon::new(IconName::ListFilter).with_size(px(18.)))
                    .on_click(run(ToggleRules)),
            )
            .child(
                tool_button("sidebar-settings", "Settings", "cmd-,", self.settings_open(), t)
                    .child(Icon::new(IconName::Settings).with_size(px(18.)))
                    .on_click(run(ToggleSettings)),
            )
    }

    /// Fetch mail. Idle shows nothing, since a dot would read as something to do; a running
    /// fetch spins the icon in the primary colour; otherwise a dot flags a stale or failing sync.
    fn sync_tool(&self, cx: &Context<Self>) -> AnyElement {
        let t = cx.theme();
        let fetching = self.is_fetching();
        let attention = self.sync_attention();
        let icon = Icon::new(IconName::RefreshCw).with_size(px(18.));
        let (tip, dot, icon) = if fetching {
            let spin = Animation::new(Duration::from_secs(1)).repeat();
            let icon = icon
                .text_color(t.primary)
                .with_animation("sidebar-sync-spin", spin, |icon, turn| icon.rotate(percentage(turn)));
            ("Fetching mail…", None, icon.into_any_element())
        } else {
            match attention {
                Attention::Fine => ("Fetch mail", None, icon.into_any_element()),
                Attention::Stale => ("Fetch mail — not synced recently", Some(t.warning), icon.into_any_element()),
                Attention::Error => ("Fetch mail — sync needs attention", Some(t.danger), icon.into_any_element()),
            }
        };
        tool_button("sidebar-fetch-mail", tip, "cmd-shift-n", false, t)
            .child(icon)
            .when_some(dot, |d, color| d.child(status_dot(color, t)))
            .on_click(run(FetchMail))
            .into_any_element()
    }

    /// The classifier: its state on the icon and in the tooltip. A press does what the status
    /// bar's text does — stop a running batch, start a paused one, otherwise open the AI
    /// settings, where the key and the auto-classify switch live.
    fn classifier_tool(&self, cx: &Context<Self>) -> AnyElement {
        let t = cx.theme();
        let status = self.judge_status().clone();
        let tip = match &status {
            JudgeStatus::Working { done, total } => format!("Classifying {done} of {total} · click to stop"),
            JudgeStatus::NoKey => "Classifier off — add a Jev key in Settings".to_owned(),
            JudgeStatus::Paused => "Classifier paused — click to start".to_owned(),
            JudgeStatus::Error(e) => format!("Classifier: {e}"),
            JudgeStatus::Idle => "Classifier".to_owned(),
        };
        let dim = matches!(status, JudgeStatus::NoKey | JudgeStatus::Paused);
        let icon = Icon::new(IconName::ScanSearch).with_size(px(18.));
        let icon = if dim { icon.text_color(t.muted_foreground.opacity(0.6)) } else { icon };
        tool_button("sidebar-classifier", &tip, "", false, t)
            .when_some(
                match status {
                    JudgeStatus::Working { done, total } => Some((done, total)),
                    _ => None,
                },
                |d, (done, total)| d.child(progress_ring(done, total, t.sidebar_border, t.primary)),
            )
            .child(icon)
            .when(matches!(self.judge_status(), JudgeStatus::Error(_)), |d| d.child(status_dot(t.danger, t)))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| match this.judge_status() {
                JudgeStatus::Working { .. } => this.stop_classifying(cx),
                JudgeStatus::Paused => this.start_classifying(cx),
                _ => this.open_settings_ai(window, cx),
            }))
            .into_any_element()
    }
}
