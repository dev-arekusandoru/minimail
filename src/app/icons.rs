//! The row icon language: ONE table maps every classifier label and message state to an
//! icon, a semantic theme token, a short label and a description. Rows, tooltips, the
//! legend in the `?` help and the README all read from [`Glyph::spec`].
//!
//! Icons are Lucide SVGs from `gpui-kit-assets`; [`AppAssets`] embeds just the ones
//! listed in `icon_assets!` and must be registered with `Application::with_assets`.
//! To add a concept: add a [`Glyph`] variant, list it in [`Glyph::ALL`], add its arm in
//! [`Glyph::spec`] (and the SVG to `icon_assets!` when it is new).

use crate::judge::{Kind, QuestionKey, Suggestion};
use crate::model::Tag;
use crate::theme::Theme;
use gpui_kit::assets::{Assets, IconName};
use gpui_kit::component::{Icon, Sizable as _, tooltip::Tooltip};
use gpui_kit::*;

gpui_kit::assets::icon_assets!(
    pub RowIcons,
    [
        ShieldAlert, Reply, Siren, Flame, Zap, User, Receipt, Newspaper, Bell, Tag, Sparkles,
        BellOff, UserPlus, Hourglass, AlarmClock, Paperclip, CornerUpRight, Clock,
        // Sidebar locations.
        Mails, Inbox, Send, Archive, Trash, Folder, FolderOpen, Folders
    ]
);

/// Asset source for the app: the row icons plus gpui-kit's component icons.
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<std::borrow::Cow<'static, [u8]>>> {
        if let Some(bytes) = RowIcons.load(path)? {
            return Ok(Some(bytes));
        }
        Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = Assets.list(path)?;
        paths.extend(RowIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

/// Semantic theme token an icon is tinted with.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Token {
    Accent,
    Muted,
    NeedsReply,
    AwaitingReply,
    FollowUp,
    Reminder,
    PossibleSpam,
    NewSender,
    Snoozed,
    Urgent,
    Kind,
}

impl Token {
    pub fn color(self, t: &Theme) -> Hsla {
        match self {
            Token::Accent => t.accent,
            Token::Muted => t.text_muted,
            Token::NeedsReply => t.needs_reply,
            Token::AwaitingReply => t.awaiting,
            Token::FollowUp => t.follow_up,
            Token::Reminder => t.reminder,
            Token::PossibleSpam => t.possible_spam,
            Token::NewSender => t.new_sender,
            Token::Snoozed => t.state_snoozed,
            Token::Urgent => t.urgent,
            Token::Kind => t.kind,
        }
    }
}

/// One concept of the icon language.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Glyph {
    Suggestion,
    PossibleSpam,
    UrgentHigh,
    NeedsReply,
    AwaitingReply,
    FollowUp,
    Reminder,
    NewSender,
    Snoozed,
    UrgentMid,
    Muted,
    Attachment,
    KindPerson,
    KindReceipt,
    KindNewsletter,
    KindNotification,
    UrgentLow,
    KindOther,
}


/// Everything the UI needs to draw and explain a [`Glyph`].
#[derive(Clone, Copy, Debug)]
pub struct Spec {
    pub icon: IconName,
    pub token: Token,
    /// Short name, used as tooltip and legend title.
    pub label: &'static str,
    /// One-sentence meaning, shown in the legend.
    pub description: &'static str,
    /// Legend section.
    pub group: &'static str,
    /// Lower sorts first in the cluster and survives narrow widths longer.
    pub priority: u8,
    /// Low-signal glyphs never take a cluster slot; they only appear in the `+N` tooltip.
    pub low_signal: bool,
}

const fn spec(
    icon: IconName,
    token: Token,
    label: &'static str,
    description: &'static str,
    group: &'static str,
    priority: u8,
    low_signal: bool,
) -> Spec {
    Spec { icon, token, label, description, group, priority, low_signal }
}

impl Glyph {
    pub const ALL: [Glyph; 18] = [
        Glyph::Suggestion,
        Glyph::PossibleSpam,
        Glyph::UrgentHigh,
        Glyph::NeedsReply,
        Glyph::AwaitingReply,
        Glyph::FollowUp,
        Glyph::Reminder,
        Glyph::NewSender,
        Glyph::Snoozed,
        Glyph::UrgentMid,
        Glyph::Muted,
        Glyph::Attachment,
        Glyph::KindPerson,
        Glyph::KindReceipt,
        Glyph::KindNewsletter,
        Glyph::KindNotification,
        Glyph::UrgentLow,
        Glyph::KindOther,
    ];

    /// The single table: enum -> icon, color token, label, description.
    pub const fn spec(self) -> Spec {
        use Token::*;
        match self {
            Glyph::Suggestion => spec(IconName::Sparkles, Accent, "AI suggestion", "A label the classifier proposes. y or click accepts, n or right-click rejects.", "Classifier labels", 0, false),
            Glyph::PossibleSpam => spec(IconName::ShieldAlert, PossibleSpam, "Possible spam", "Classified as possible spam.", "Classifier labels", 1, false),
            Glyph::UrgentHigh => spec(IconName::Siren, Urgent, "Very urgent", "Urgency 4 or 5 out of 5.", "Urgency", 2, false),
            Glyph::NeedsReply => spec(IconName::Reply, NeedsReply, "Needs reply", "The sender expects an answer.", "Classifier labels", 3, false),
            Glyph::AwaitingReply => spec(IconName::Hourglass, AwaitingReply, "Awaiting reply", "A reply has been sent and an answer is expected.", "Reply status", 4, false),
            Glyph::FollowUp => spec(IconName::CornerUpRight, FollowUp, "Follow up", "A reply is overdue: follow up with this thread.", "Reply status", 5, false),
            Glyph::Reminder => spec(IconName::AlarmClock, Reminder, "Reminder", "A snoozed message woke back up in the inbox.", "Reply status", 6, false),
            Glyph::NewSender => spec(IconName::UserPlus, NewSender, "New sender", "First mail from this sender.", "Mail state", 7, false),
            Glyph::Snoozed => spec(IconName::Clock, Snoozed, "Snoozed", "Hidden until the wake time shown on the row.", "Mail state", 8, false),
            Glyph::UrgentMid => spec(IconName::Flame, Urgent, "Urgent", "Urgency 3 out of 5.", "Urgency", 7, false),
            Glyph::Muted => spec(IconName::BellOff, Muted, "Muted thread", "New replies in this thread are hidden from the inbox.", "Mail state", 8, false),
            Glyph::Attachment => spec(IconName::Paperclip, Muted, "Attachment", "The subject or opening lines mention an attachment.", "Mail state", 9, false),
            Glyph::KindPerson => spec(IconName::User, Kind, "Person", "Written by a person.", "Kind", 10, false),
            Glyph::KindReceipt => spec(IconName::Receipt, Kind, "Receipt", "A receipt or statement.", "Kind", 10, false),
            Glyph::KindNewsletter => spec(IconName::Newspaper, Kind, "Newsletter", "A newsletter or bulk mailing.", "Kind", 10, false),
            Glyph::KindNotification => spec(IconName::Bell, Kind, "Notification", "An automatic notification.", "Kind", 10, false),
            Glyph::UrgentLow => spec(IconName::Zap, Muted, "Low urgency", "Urgency 1 or 2 out of 5 (low signal, folded into +N).", "Urgency", 11, true),
            Glyph::KindOther => spec(IconName::Tag, Muted, "Other", "Uncategorised (low signal, folded into +N).", "Kind", 12, true),
        }
    }

    pub fn of_kind(kind: Kind) -> Glyph {
        match kind {
            Kind::Person => Glyph::KindPerson,
            Kind::Receipt => Glyph::KindReceipt,
            Kind::Newsletter => Glyph::KindNewsletter,
            Kind::Notification => Glyph::KindNotification,
            Kind::Other => Glyph::KindOther,
        }
    }

    pub fn of_urgency(level: u8) -> Glyph {
        match level {
            4.. => Glyph::UrgentHigh,
            3 => Glyph::UrgentMid,
            _ => Glyph::UrgentLow,
        }
    }
}

/// Legend sections in display order: `(group, glyphs)`.
pub fn legend() -> Vec<(&'static str, Vec<Glyph>)> {
    let mut groups: Vec<(&'static str, Vec<Glyph>)> = Vec::new();
    for g in Glyph::ALL {
        let group = g.spec().group;
        match groups.iter_mut().find(|(name, _)| *name == group) {
            Some((_, list)) => list.push(g),
            None => groups.push((group, vec![g])),
        }
    }
    groups
}

/// Facts about one message that decide its icon cluster.
pub struct GlyphInputs<'a> {
    pub tags: &'a [Tag],
    pub pending: &'a [&'a Suggestion],
    pub muted: bool,
    pub new_sender: bool,
    pub snoozed: bool,
    pub attachment: bool,
}

/// Cluster glyphs of a message, most important first, without duplicates.
pub fn glyphs_for(input: &GlyphInputs) -> Vec<Glyph> {
    let mut out = Vec::new();
    if !input.pending.is_empty() {
        out.push(Glyph::Suggestion);
    }
    for tag in input.tags {
        out.push(match tag {
            Tag::NeedsReply => Glyph::NeedsReply,
            Tag::AwaitingReply => Glyph::AwaitingReply,
            Tag::FollowUp => Glyph::FollowUp,
            Tag::Reminder => Glyph::Reminder,
            Tag::PossibleSpam => Glyph::PossibleSpam,
            Tag::Urgent(n) => Glyph::of_urgency(*n),
            Tag::Kind(k) => Glyph::of_kind(*k),
        });
    }
    let flags = [
        (input.muted, Glyph::Muted),
        (input.new_sender, Glyph::NewSender),
        (input.snoozed, Glyph::Snoozed),
        (input.attachment, Glyph::Attachment),
    ];
    out.extend(flags.into_iter().filter(|(on, _)| *on).map(|(_, g)| g));
    out.sort_by_key(|g| g.spec().priority);
    out.dedup();
    out
}

/// Split `glyphs` (already priority-sorted) into the at most `max` shown icons and the rest.
/// Low-signal glyphs are always in the rest.
pub fn split_overflow(glyphs: &[Glyph], max: usize) -> (Vec<Glyph>, Vec<Glyph>) {
    let mut shown = Vec::new();
    let mut hidden = Vec::new();
    for &g in glyphs {
        if shown.len() < max && !g.spec().low_signal {
            shown.push(g);
        } else {
            hidden.push(g);
        }
    }
    (shown, hidden)
}

/// How many cluster icons fit a list panel of `width` pixels.
pub fn max_icons(width: f32) -> usize {
    match width {
        w if w >= 500. => 4,
        w if w >= 430. => 3,
        w if w >= 370. => 2,
        _ => 1,
    }
}

/// One-line description of a message's pending suggestions, e.g. `spam?, urgent 4?`.
pub fn suggestion_summary(pending: &[&Suggestion]) -> String {
    let parts: Vec<String> = pending
        .iter()
        .map(|s| match s.key {
            QuestionKey::Spam => "spam?".to_owned(),
            QuestionKey::NeedsReply => "needs reply?".to_owned(),
            QuestionKey::ExpectsReply => "expects reply?".to_owned(),
            QuestionKey::Urgency => s.urgency().map_or("urgent?".into(), |n| format!("urgent {n}?")),
            QuestionKey::Kind => s.kind().map_or("kind?".into(), |k| format!("{}?", k.label())),
        })
        .collect();
    parts.join(", ")
}

/// The themed icon of a glyph at `size` pixels.
pub fn icon(glyph: Glyph, t: &Theme, size: f32) -> Icon {
    let spec = glyph.spec();
    Icon::new(spec.icon).with_size(px(size)).text_color(spec.token.color(t))
}

fn tip(text: String) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    move |window, cx| Tooltip::new(text.clone()).build(window, cx)
}

/// The right-aligned icon cluster: `shown` icons with tooltips plus a `+N` chip whose tooltip
/// lists `hidden`. `suggestion_hint` is the tooltip of the suggestion icon. `id` must be unique
/// per row.
pub fn cluster(
    shown: &[Glyph],
    hidden: &[Glyph],
    suggestion_hint: &str,
    t: &Theme,
    id: usize,
) -> Div {
    let icons = shown.iter().enumerate().map(|(i, &g)| {
        let label = match g {
            Glyph::Suggestion => format!(
                "AI suggestion: {suggestion_hint}. y / click accepts, n / right-click rejects"
            ),
            _ => g.spec().label.to_owned(),
        };
        div()
            .id(("glyph", id * 32 + i))
            .flex_none()
            .child(icon(g, t, 13.))
            .tooltip(tip(label))
    });
    let more = (!hidden.is_empty()).then(|| {
        let list = hidden.iter().map(|g| g.spec().label).collect::<Vec<_>>().join(" · ");
        div()
            .id(("glyph-more", id))
            .flex_none()
            .text_size(px(10.))
            .text_color(t.text_muted)
            .child(format!("+{}", hidden.len()))
            .tooltip(tip(list))
    });
    div().flex().items_center().gap(px(4.)).children(icons).children(more)
}

/// The legend: every glyph with icon, label and description, grouped. Used by the `?` help.
pub fn legend_view(t: &Theme) -> Div {
    let row_states = [
        ("Unread", t.unread, "Yellow bar; sender and subject are bold."),
        ("Urgent unread", t.urgent, "Red bar; the urgency icon remains in the icon cluster."),
        ("Selected / open", t.selected, "Blue bar; opening a message uses the same treatment as a one-item selection."),
        ("Partial thread selection", t.selected.opacity(0.45), "Dimmed blue bar when only some messages are selected."),
    ];
    let row_state_section = div()
        .flex()
        .flex_col()
        .gap_1()
        .child(div().text_xs().text_color(t.accent).child("Row states"))
        .children(row_states.into_iter().map(|(label, color, description)| {
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_xs()
                .child(div().w(px(3.)).h(px(12.)).bg(color))
                .child(div().w(px(112.)).flex_none().text_color(t.text).child(label))
                .child(div().flex_1().text_color(t.text_muted).child(description))
        }))
        .child(
            div()
                .text_xs()
                .text_color(t.text_muted)
                .child("Cursor: row_cursor background and outline. Click the left edge to select; cmd-click toggles and shift-click range-selects."),
        );
    let sections = legend().into_iter().map(|(group, glyphs)| {
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(div().text_xs().text_color(t.accent).child(group))
            .children(glyphs.into_iter().map(|g| {
                let s = g.spec();
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_xs()
                    .child(div().w(px(16.)).flex_none().child(icon(g, t, 13.)))
                    .child(div().w(px(96.)).flex_none().text_color(t.text).child(s.label))
                    .child(div().flex_1().text_color(t.text_muted).child(s.description))
            }))
    });
    div().flex().flex_col().gap_2().child(row_state_section).children(sections)
}
