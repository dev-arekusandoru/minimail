//! Triage mode's takeover ("Focus Reader"): while a session's Start screen, run or Summary is
//! open it replaces everything under the title bar. Layout and px values follow
//! `mockups/triage-focus.html`; colors are theme tokens, and the suggestion card's tints are
//! the primary color blended over the background.
//!
//! The running view is: a 3px progress bar, the End / count / Undo row, the normal reader
//! (the same thread, rail and message surface every tab renders) and a fixed bottom dock. The
//! email body is a native view that paints over GPUI, so the dock is a reserved band below the
//! scroll area rather than something laid over it.
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _};

use super::*;
use crate::app::ui::{mono_font, Observable};
use crate::theme::ThemeColor;
use crate::triage_session::{SessionFilter, SessionOrder, SessionSize};

const TRIAGE_READER_MAX_W: f32 = 720.;

/// The mockup's palette, mapped onto theme tokens.
pub(super) struct Pal {
    pub bg: Hsla,
    pub deep: Hsla,
    pub fg: Hsla,
    pub muted: Hsla,
    pub border: Hsla,
    pub active: Hsla,
    pub surface: Hsla,
    pub primary: Hsla,
    pub on_primary: Hsla,
    primary_hover: Hsla,
    key_primary: Hsla,
    sugg_bg: Hsla,
    sugg_border: Hsla,
    sugg_tile: Hsla,
    ok: Hsla,
    ok_tile: Hsla,
    danger: Hsla,
}

impl Pal {
    pub(super) fn new(t: &ThemeColor) -> Self {
        let tint = |c: Hsla, a: f32| t.background.blend(c.opacity(a));
        Self {
            bg: t.background,
            deep: t.title_bar,
            fg: t.foreground,
            muted: t.muted_foreground,
            border: t.border,
            active: t.list_active,
            surface: t.secondary,
            primary: t.primary,
            on_primary: t.primary_foreground,
            primary_hover: t.primary_hover,
            key_primary: Hsla { l: t.primary.l * 0.84, ..t.primary },
            sugg_bg: tint(t.primary, 0.08),
            sugg_border: tint(t.primary, 0.35),
            sugg_tile: tint(t.primary, 0.22),
            ok: t.success,
            ok_tile: tint(t.success, 0.22),
            danger: t.danger,
        }
    }
}

fn ic(name: IconName, size: f32, color: Hsla) -> Icon {
    Icon::new(name).with_size(px(size)).text_color(color)
}

/// A 20px key badge (`e`, `⏎`, `esc`); `primary` is the filled one on a primary button.
pub(super) fn key_badge(p: &Pal, mono: &SharedString, label: &'static str, primary: bool) -> Div {
    let (bg, border, fg) = if primary { (p.key_primary, p.key_primary, p.on_primary) } else { (p.deep, p.border, p.muted) };
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .min_w(px(20.))
        .h(px(20.))
        .px(px(6.))
        .rounded(px(4.))
        .border_1()
        .border_color(border)
        .bg(bg)
        .font_family(mono.clone())
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(px(11.))
        .line_height(px(11.))
        .text_color(fg)
        .child(label)
}

/// `.btn`: 32px tall, 13px text, active-colored hover.
fn btn(id: &'static str, color: Hsla, p: &Pal) -> Observable {
    let hover = p.active;
    div()
        .id(id)
        .test_support()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.))
        .h(px(32.))
        .px(px(10.))
        .rounded(px(6.))
        .text_size(px(13.))
        .text_color(color)
        .whitespace_nowrap()
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
}

/// `.btn.primary`.
fn primary_btn(id: &'static str, p: &Pal) -> Observable {
    let hover = p.primary_hover;
    div()
        .id(id)
        .test_support()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.))
        .h(px(32.))
        .pl(px(12.))
        .pr(px(8.))
        .rounded(px(6.))
        .bg(p.primary)
        .text_size(px(13.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(p.on_primary)
        .whitespace_nowrap()
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
}

/// Thousands separators: `2292` → `2,292`.
fn group(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `18m 20s`, or `26s` under a minute.
fn duration(secs: i64) -> String {
    if secs >= 60 { format!("{}m {}s", secs / 60, secs % 60) } else { format!("{secs}s") }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{} {}", group(n), if n == 1 { one } else { many })
}

/// Mono-less `.pill`: a filter chip (bordered) or a segment of a `.seg2` (borderless).
fn pill(id: SharedString, label: &str, count: Option<usize>, on: bool, segment: bool, p: &Pal) -> Observable {
    let border = if segment { gpui_kit::transparent_black() } else if on { p.active } else { p.border };
    div()
        .id(id)
        .test_support()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.))
        .h(px(if segment { 24. } else { 28. }))
        .px(px(10.))
        .rounded(px(6.))
        .border_1()
        .border_color(border)
        .when(on, |d| d.bg(p.active))
        .text_size(px(12.))
        .text_color(p.fg)
        .whitespace_nowrap()
        .cursor_pointer()
        .child(label.to_owned())
        .when_some(count, |d, n| d.child(div().text_size(px(11.)).text_color(p.muted).child(group(n))))
}

/// `.seg2`: a bordered strip of segments.
fn segments(p: &Pal) -> Div {
    div().flex().gap(px(2.)).p(px(2.)).rounded(px(8.)).border_1().border_color(p.border)
}

/// One setup row: 88px label, then the controls.
fn field(label: &'static str, p: &Pal) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(16.))
        .min_h(px(56.))
        .px(px(20.))
        .py(px(12.))
        .border_t_1()
        .border_color(p.border)
        .child(div().w(px(88.)).flex_none().text_size(px(12.)).line_height(px(16.)).text_color(p.muted).child(label))
}

/// The 520px centred card.
fn card(p: &Pal) -> Div {
    div()
        .flex()
        .flex_col()
        .w(px(520.))
        .rounded(px(12.))
        .border_1()
        .border_color(p.border)
        .bg(p.surface)
        .overflow_hidden()
}

/// A card header: tile, title, subtitle.
fn card_head(tile_bg: Hsla, icon: IconName, icon_color: Hsla, title: &'static str, sub: String, p: &Pal) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .pt(px(20.))
        .px(px(20.))
        .pb(px(16.))
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(40.))
                .rounded(px(10.))
                .bg(tile_bg)
                .child(ic(icon, 16., icon_color)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .child(div().text_size(px(16.)).line_height(px(22.)).font_weight(FontWeight::SEMIBOLD).text_color(p.fg).child(title))
                .child(div().text_size(px(12.)).line_height(px(16.)).text_color(p.muted).child(sub)),
        )
}

/// A card footer: `grow` text on the left, buttons after it.
fn card_foot(p: &Pal) -> Div {
    div().flex().items_center().gap(px(12.)).px(px(20.)).py(px(16.)).border_t_1().border_color(p.border).bg(p.bg)
}

fn bold(text: String, p: &Pal) -> Div {
    div().font_weight(FontWeight::SEMIBOLD).text_color(p.fg).child(text)
}

impl MailApp {
    /// Everything under the title bar while Triage mode has the window.
    pub(super) fn render_triage(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = Pal::new(&cx.theme().colors);
        let mono = mono_font(cx);
        let (frac, done, body) = if let Some(setup) = self.session_setup().cloned() {
            self.hide_session_views();
            (0., false, self.setup_screen(&setup, &p, &mono, cx))
        } else if let Some(summary) = self.session_summary().copied() {
            self.hide_session_views();
            (1., true, self.summary_screen(&summary, &p, &mono, cx))
        } else {
            let (pos, total) = self.session_progress().unwrap_or((0, 1));
            (pos as f32 / total.max(1) as f32, false, self.running_screen(&p, &mono, cx))
        };
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(p.bg)
            .child(progress(frac, if done { p.ok } else { p.primary }, &p))
            .child(body)
            .into_any_element()
    }

    /// No reader is drawn this frame, so every email view of the Triage slot hides.
    fn hide_session_views(&self) {
        self.webviews.begin_pass(true);
        self.webviews.end_pass(true);
    }

    /// The End/Cancel · count · Undo row.
    fn top_row(&self, left: Option<&'static str>, mid: Option<Div>, undo: bool, p: &Pal, mono: &SharedString) -> Div {
        let side = || div().flex().flex_none().w(px(120.));
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .h(px(44.))
            .px(px(12.))
            .child(side().when_some(left, |d, label| {
                d.child(
                    btn("triage-end", p.muted, p)
                        .px(px(8.))
                        .on_click(crate::app::ui::run(ClearSelection))
                        .child(ic(IconName::Close, 16., p.muted))
                        .child(label)
                        .child(key_badge(p, mono, "esc", false)),
                )
            }))
            .child(div().text_size(px(12.)).line_height(px(16.)).text_color(p.muted).children(mid))
            .child(side().justify_end().when(undo, |d| {
                d.child(
                    btn("triage-undo", p.muted, p)
                        .px(px(8.))
                        .on_click(crate::app::ui::run(Undo))
                        .child(ic(IconName::Undo2, 16., p.muted))
                        .child("Undo")
                        .child(key_badge(p, mono, "⌘Z", false)),
                )
            }))
    }

    // ------------------------------------------------------------------ Start

    fn setup_screen(&self, setup: &crate::triage_session::SetupView, p: &Pal, mono: &SharedString, cx: &mut Context<Self>) -> AnyElement {
        let o = &setup.options;
        let owner = cx.weak_entity();
        let place = sidebar::location_label(&o.location, &self.mailbox);
        let mut places = vec![Location::AllInboxes];
        places.extend(self.mailbox.accounts().iter().map(|a| Location::Inbox(a.id.clone())));
        let places: Vec<(Location, SharedString)> = places
            .into_iter()
            .map(|l| {
                let label = sidebar::location_label(&l, &self.mailbox);
                (l, label.into())
            })
            .collect();
        let current = o.location.clone();
        let sel = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .w_full()
            .h(px(32.))
            .px(px(10.))
            .rounded(px(6.))
            .border_1()
            .border_color(p.border)
            .text_size(px(13.))
            .text_color(p.fg)
            .child(ic(IconName::Inbox, 16., p.fg))
            .child(place)
            .child(div().font_family(mono.clone()).text_size(px(11.)).text_color(p.muted).child(group(setup.total)))
            .child(div().flex_1())
            .child(ic(IconName::ChevronDown, 16., p.muted));
        let inbox = Button::new("setup-inbox")
            .ghost()
            .flex_1()
            .h(px(32.))
            .px_0()
            .rounded(px(6.))
            .child(sel)
            .dropdown_menu_with_anchor(Anchor::TopLeft, move |menu, _, _| {
                places.iter().fold(menu, |menu, (loc, label)| {
                    let (owner, loc) = (owner.clone(), loc.clone());
                    menu.item(PopupMenuItem::new(label.clone()).checked(loc == current).on_click({
                        let loc = loc.clone();
                        move |_, _, cx| {
                            let loc = loc.clone();
                            owner.update(cx, |this, cx| this.setup_set_location(loc, cx)).ok();
                        }
                    }))
                })
            });

        let filters = SessionFilter::ALL.iter().fold(div().flex().flex_wrap().items_center().gap(px(6.)).flex_1().min_w_0(), |d, f| {
            let f = *f;
            let count = setup.filter_counts.iter().find(|(k, _)| *k == f).map_or(0, |(_, n)| *n);
            d.child(
                pill(format!("setup-filter-{}", f.label()).into(), f.label(), Some(count), o.filter == f, false, p)
                    .on_click(cx.listener(move |this, _, _, cx| this.setup_set_filter(f, cx))),
            )
        });
        let sizes = SessionSize::ALL.iter().fold(segments(p), |d, s| {
            let s = *s;
            d.child(
                pill(format!("setup-size-{}", s.label()).into(), s.label(), None, o.size == s, true, p)
                    .on_click(cx.listener(move |this, _, _, cx| this.setup_set_size(s, cx))),
            )
        });
        let orders = SessionOrder::ALL.iter().fold(segments(p), |d, s| {
            let s = *s;
            d.child(
                pill(format!("setup-order-{}", s.label()).into(), s.label(), None, o.order == s, true, p)
                    .on_click(cx.listener(move |this, _, _, cx| this.setup_set_order(s, cx))),
            )
        });
        let on = o.suggestions;
        let switch = div()
            .id("setup-suggestions")
            .test_support()
            .relative()
            .flex_none()
            .w(px(32.))
            .h(px(20.))
            .rounded(px(10.))
            .bg(if on { p.primary } else { p.active })
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| this.setup_set_suggestions(!on, cx)))
            .child(
                div()
                    .absolute()
                    .top(px(2.))
                    .when(on, |d| d.right(px(2.)))
                    .when(!on, |d| d.left(px(2.)))
                    .size(px(16.))
                    .rounded(px(8.))
                    .bg(if on { p.on_primary } else { p.muted }),
            );

        let summary = div()
            .flex_1()
            .text_size(px(12.))
            .text_color(p.muted)
            .flex()
            .gap(px(4.))
            .child(bold(group(setup.queue_len), p))
            .child(format!("{} · {}", if setup.queue_len == 1 { "message" } else { "messages" }, o.order.label().to_lowercase()));

        let card = card(p)
            .child(card_head(p.sugg_tile, IconName::Inbox, p.primary, "Triage", "Work through your inbox one message at a time.".into(), p))
            .child(field("Inbox", p).child(div().flex().flex_1().min_w_0().child(inbox)))
            .child(field("Filter", p).child(filters))
            .child(field("Session", p).child(div().flex().flex_1().child(sizes)))
            .child(field("Order", p).child(div().flex().flex_1().child(orders)))
            .child(
                field("AI suggestions", p)
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .items_center()
                            .justify_between()
                            .child(div().text_size(px(12.)).text_color(p.muted).child("Show a suggested action on each message"))
                            .child(switch),
                    ),
            )
            .child(
                card_foot(p).child(summary).child(
                    primary_btn("triage-start", p)
                        .on_click(cx.listener(|this, _, _, cx| this.start_session_from_setup(cx)))
                        .child(ic(IconName::Play, 16., p.on_primary))
                        .child("Start")
                        .child(key_badge(p, mono, "⏎", true)),
                ),
            );
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(self.top_row(Some("Cancel"), None, false, p, mono))
            .child(centered(card))
            .into_any_element()
    }

    // ---------------------------------------------------------------- Summary

    fn summary_screen(
        &self,
        s: &crate::triage_session::SessionSummary,
        p: &Pal,
        mono: &SharedString,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let options = self.session_summary.as_ref().map(|(_, o)| o.clone()).unwrap_or_default();
        let t = s.tally;
        let cell = |icon: IconName, n: usize, label: &'static str, col: usize, row: usize| {
            div()
                .flex()
                .flex_1()
                .items_center()
                .gap(px(12.))
                .h(px(68.))
                .px(px(20.))
                .border_color(p.border)
                .when(col < 2, |d| d.border_r_1())
                .when(row == 0, |d| d.border_b_1())
                .child(ic(icon, 16., p.muted))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(div().text_size(px(20.)).line_height(px(24.)).font_weight(FontWeight::SEMIBOLD).text_color(p.fg).child(n.to_string()))
                        .child(div().text_size(px(12.)).line_height(px(16.)).text_color(p.muted).child(label)),
                )
        };
        let grid_row = |cells: [(IconName, usize, &'static str); 3], row: usize| {
            cells.into_iter().enumerate().fold(div().flex(), |d, (col, (icon, n, label))| d.child(cell(icon, n, label, col, row)))
        };
        let stats = div()
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(p.border)
            .child(grid_row([(IconName::Archive, t.archived, "Archived"), (IconName::Folder, t.filed, "Filed"), (IconName::Clock, t.snoozed, "Snoozed")], 0))
            .child(grid_row([(IconName::Trash, t.deleted, "Deleted"), (IconName::Reply, t.replied, "Replied"), (IconName::SkipForward, t.skipped, "Skipped")], 1));
        let note = (options.suggestions && s.handled > 0).then(|| {
            let pct = (t.suggestions_taken * 100 + s.handled / 2) / s.handled;
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .h(px(44.))
                .px(px(20.))
                .border_t_1()
                .border_color(p.border)
                .text_size(px(12.))
                .text_color(p.muted)
                .child(ic(IconName::Sparkles, 16., p.primary))
                .child("You took the suggestion on")
                .child(bold(format!("{} of {}", group(t.suggestions_taken), group(s.handled)), p))
                .child(format!("({pct}%)"))
        });
        let place = sidebar::location_label(&options.location, &self.mailbox);
        let sub = format!(
            "{} in {} · about {} each",
            plural(s.handled, "message", "messages"),
            duration(s.elapsed_secs),
            duration(s.avg_secs())
        );
        let card = card(p)
            .child(card_head(p.ok_tile, IconName::Check, p.ok, "Session complete", sub, p))
            .child(stats)
            .children(note)
            .child(
                card_foot(p)
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .gap(px(4.))
                            .text_size(px(12.))
                            .text_color(p.muted)
                            .child(bold(group(s.remaining), p))
                            .child(format!("left in {place}")),
                    )
                    .child(
                        btn("triage-done", p.fg, p)
                            .border_1()
                            .border_color(p.border)
                            .px(px(12.))
                            .on_click(crate::app::ui::run(ClearSelection))
                            .child("Done")
                            .child(key_badge(p, mono, "esc", false)),
                    )
                    .child(
                        primary_btn("triage-keep-going", p)
                            .on_click(cx.listener(|this, _, _, cx| this.keep_going(cx)))
                            .child(ic(IconName::Play, 16., p.on_primary))
                            .child("Keep going")
                            .child(key_badge(p, mono, "⏎", true)),
                    ),
            );
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(self.top_row(None, None, false, p, mono))
            .child(centered(card))
            .into_any_element()
    }

    // ---------------------------------------------------------------- Running

    fn running_screen(&mut self, p: &Pal, mono: &SharedString, cx: &mut Context<Self>) -> AnyElement {
        let (pos, total) = self.session_progress().unwrap_or((0, 0));
        let place = self
            .session_options()
            .map(|o| sidebar::location_label(&o.location, &self.mailbox))
            .unwrap_or_default();
        let count = div()
            .flex()
            .gap(px(4.))
            .child(bold(group(pos), p))
            .child(format!("of {} · {place}", group(total)));
        let composing = self.compose.is_some();
        let reader = dock::reader_content(self, Slot::Triage, cx);
        let dock = (!composing).then(|| self.dock_band(p, mono, cx));
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(self.top_row(Some("End"), Some(count), true, p, mono))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .max_w(px(TRIAGE_READER_MAX_W))
                            .flex()
                            .flex_col()
                            .child(reader),
                    ),
            )
            .children(dock)
            .into_any_element()
    }

    /// The fixed bottom band: the suggestion card (when there is one) over the toolbar.
    fn dock_band(&self, p: &Pal, mono: &SharedString, cx: &mut Context<Self>) -> Div {
        let suggestion = self.session_suggestion().map(|s| {
            let (verb, icon, title) = match &s.action {
                SuggestedAction::Archive => ("Archive", IconName::Archive, "Suggested: Archive".to_owned()),
                SuggestedAction::Delete => ("Delete", IconName::Trash, "Suggested: Delete".to_owned()),
                SuggestedAction::File(folder) => (
                    "File",
                    IconName::Folder,
                    format!("Suggested: File to {}", self.mailbox.folder(*folder).map_or("folder", |f| f.name.as_str())),
                ),
            };
            let kind = self
                .session_current()
                .and_then(|id| self.mailbox.tags(id).iter().find_map(|t| if let Tag::Kind(k) = t { Some(k.label()) } else { None }));
            let reason = match kind {
                Some(kind) => format!("{kind} · {}", s.reason),
                None => s.reason.clone(),
            };
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .h(px(56.))
                .pl(px(10.))
                .pr(px(12.))
                .rounded(px(12.))
                .bg(p.sugg_bg)
                .border_1()
                .border_color(p.sugg_border)
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .size(px(32.))
                        .rounded(px(8.))
                        .bg(p.sugg_tile)
                        .child(ic(IconName::Sparkles, 16., p.primary)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(div().truncate().text_size(px(13.)).line_height(px(18.)).font_weight(FontWeight::SEMIBOLD).text_color(p.fg).child(title))
                        .child(div().truncate().text_size(px(12.)).line_height(px(16.)).text_color(p.muted).child(reason)),
                )
                .child(
                    primary_btn("triage-accept", p)
                        .h(px(36.))
                        .on_click(crate::app::ui::run(AcceptSessionSuggestion))
                        .child(ic(icon, 16., p.on_primary))
                        .child(verb)
                        .child(key_badge(p, mono, "⏎", true)),
                )
        });

        let tool = |id: &'static str, icon: IconName, label: &'static str, key: &'static str, color: Hsla| {
            btn(id, p.fg, p)
                .child(ic(icon, 16., color))
                .child(label)
                .child(div().ml(px(2.)).child(key_badge(p, mono, key, false)))
        };
        let toolbar = div()
            .flex()
            .items_center()
            .gap(px(2.))
            .p(px(4.))
            .rounded(px(12.))
            .bg(p.surface)
            .border_1()
            .border_color(p.border)
            .child(tool("triage-archive", IconName::Archive, "Archive", "e", p.fg).on_click(crate::app::ui::run(Archive)))
            .child(tool("triage-snooze", IconName::Clock, "Snooze", "s", p.fg).on_click(crate::app::ui::run(OpenSnoozePicker)))
            .child(tool("triage-file", IconName::Folder, "File", "f", p.fg).on_click(crate::app::ui::run(File)))
            .child(tool("triage-reply", IconName::Reply, "Reply", "r", p.fg).on_click(crate::app::ui::run(Reply)))
            .child(div().w(px(1.)).h(px(20.)).mx(px(6.)).bg(p.border))
            .child(tool("triage-delete", IconName::Trash, "Delete", "d", p.danger).on_click(crate::app::ui::run(Delete)))
            .child(tool("triage-mute", IconName::BellOff, "Mute", "m", p.fg).on_click(crate::app::ui::run(MuteThread)))
            .child(tool("triage-skip", IconName::SkipForward, "Skip", "␣", p.fg).on_click(cx.listener(|this, _, w, cx| this.skip_current(w, cx))));
        div()
            .flex()
            .flex_none()
            .justify_center()
            .pt(px(12.))
            .pb(px(24.))
            .border_t_1()
            .border_color(p.border)
            .child(div().flex().flex_col().gap(px(8.)).children(suggestion).child(toolbar))
    }
}

/// The 3px progress bar under the title bar.
fn progress(frac: f32, color: Hsla, p: &Pal) -> Div {
    div()
        .flex_none()
        .w_full()
        .h(px(3.))
        .bg(p.active)
        .child(div().h(px(3.)).w(relative(frac.clamp(0., 1.))).bg(color))
}

/// `.center`: the card in the middle of the space under the top row, 40px above centre.
fn centered(card: Div) -> Div {
    div().flex_1().min_h_0().flex().items_center().justify_center().pb(px(40.)).child(card)
}
