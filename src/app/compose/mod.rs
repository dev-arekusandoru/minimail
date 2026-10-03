//! The composer: recipients, subject, a Markdown body with a rendered preview, a compact
//! formatting toolbar, managed attachments, and an honest save/sync status line.
//!
//! The view owns only form state (inputs, focus, scroll). Every durable fact belongs to
//! the [`crate::outgoing::DraftRecord`] the owner hands in and gets back through
//! [`ComposeEvent::Edited`], so a crash, a restart or another surface editing the same
//! draft all agree on what it says.

pub mod drafts;
pub mod recipients;
pub mod toolbar;

use gpui_kit::component::Disableable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _, DropdownButton};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::{
    ActiveTheme as _, Sizable as _, WindowExt as _, scroll::ScrollableElement as _,
};
use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::app::actions::*;
use crate::app::compose::recipients::{Field, RecipientEvent, Recipients};
use crate::outgoing::body::{BodyMode, ConversionWarning, DraftBody};
use crate::outgoing::markdown::{BlockFormat, InlineFormat};
use crate::outgoing::{
    AttachmentId, Clipboard, DraftId, DraftKind, DraftRecord, DraftState, ManagedAttachment,
    markdown,
};

/// Where the composer is shown, which changes the controls around it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Placement {
    /// Under the conversation it answers.
    Inline,
    /// In its own tab of the shared strip.
    #[default]
    Tab,
}

/// Where the caret should land when a composer opens.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Focus {
    /// The body: a reply starts here, because its recipients are already right.
    #[default]
    Body,
    /// The recipient input: a new message, or a forward that has no recipient yet.
    Recipients,
}

/// What the composer asks its owner to do. The view never persists or sends anything
/// itself: both are the owner's, on the worker.
#[derive(Clone)]
pub enum ComposeEvent {
    /// A field changed; the owner saves locally immediately and debounces the remote mirror.
    Edited,
    /// Send was asked for. The owner validates, then holds.
    Send,
    /// Attach files were asked for; the owner opens the picker.
    Attach,
    /// Remove one attachment from the record.
    RemoveAttachment(AttachmentId),
    /// Close the surface, keeping the draft.
    Close,
    /// Discard the draft: the one action that deletes it.
    Discard,
    /// Expand (inline → tab) or collapse (tab → inline) the same draft.
    Expand,
    Collapse,
    /// Switch the sending account.
    SetAccount(String),
    /// The provider's copy moved on. The user decides what to keep: their own text, the
    /// server's, or both as separate drafts. Nothing is overwritten without this.
    ResolveConflict,
    /// Query delivery without ever sending another copy.
    CheckDelivery,
}

/// The MIME type a clipboard entry declares, when it declares one exactly.
///
/// The metadata is opaque private JSON, so this reads the declared `mimeType`/`contentType`
/// field and accepts nothing else: only a real flavour marker counts, never a mention of
/// one. A body that merely quotes `text/html` inside another field stays plain text.
fn mime_flavor(metadata: Option<&str>) -> Option<String> {
    let raw = metadata?;
    let parsed: serde_json::Value = serde_json::from_str(raw).ok()?;
    ["mimeType", "contentType", "mime_type"]
        .iter()
        .find_map(|key| parsed.get(*key).and_then(|value| value.as_str()))
        .map(ToOwned::to_owned)
}

/// The rich flavour of a clipboard payload, read from the platform's own pasteboard.
///
/// GPUI reads the plain string and its private metadata, never the system `text/html`
/// flavour, so a copy made in a browser or another mail client arrives with the HTML
/// missing. The helper takes the plain text that was already captured and returns the HTML
/// only when the pasteboard still holds that same text, so a clipboard changed underneath
/// the paste cannot substitute a different payload for it.
#[cfg(target_os = "macos")]
fn pasteboard_html(captured_plain: &str) -> Option<String> {
    if captured_plain.trim().is_empty() {
        return None;
    }
    crate::native_clipboard::html_for_text(captured_plain)
}

/// The rich flavour of a clipboard payload, where the clipboard hands out no HTML of its
/// own. Nothing to read, so a plain paste stays plain.
#[cfg(not(target_os = "macos"))]
fn pasteboard_html(_captured_plain: &str) -> Option<String> {
    None
}

/// A rich paste whose converted form is waiting for the user's decision.
struct PendingPaste {
    /// The clipboard's plain text, offered as "paste as text".
    plain: String,
    /// What the conversion dropped, for the banner.
    warnings: Vec<String>,
    /// The range the converted text occupies in the body source.
    range: std::ops::Range<usize>,
    /// The body source as it stood right after the insertion.
    ///
    /// "Paste as text" replaces that range and nothing else, so the offer is only good
    /// while the document is still exactly this one: anything typed since has moved the
    /// text, and a stale range would swallow whatever was typed in its place.
    source: String,
}

impl PendingPaste {
    /// One line naming what the conversion could not keep.
    fn detail(&self) -> String {
        match self.warnings.len() {
            0 => "Pasted as Markdown; formatting was simplified.".to_owned(),
            1 => format!("Pasted as Markdown; {} was dropped.", self.warnings[0]),
            n => format!(
                "Pasted as Markdown; {} and {} were dropped.",
                self.warnings[0],
                self.warnings[n - 1]
            ),
        }
    }

    /// The range to replace with the plain text, or `None` when the document has moved on
    /// since the paste and the offer no longer describes anything.
    fn plain_range(&self, current: &str) -> Option<std::ops::Range<usize>> {
        if current == self.source {
            Some(self.range.clone())
        } else {
            None
        }
    }
}

/// The byte offset of the nearest character boundary at or before `at`.
fn boundary_floor(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while at > 0 && !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// A selection snapped to character boundaries, so a range recorded for a later edit
/// addresses the same text the insertion did.
fn boundaries(text: &str, range: std::ops::Range<usize>) -> std::ops::Range<usize> {
    let start = boundary_floor(text, range.start);
    let end = boundary_floor(text, range.end.clamp(start, text.len()));
    start..end
}

/// A link whose address is being asked for. The selection is held here while the dialog is
/// open, so the words chosen stay the link's text and the caret lands inside them.
struct PendingLink {
    /// The Markdown source when the link was asked for.
    source: String,
    /// The selection the link wraps, in `source`.
    selection: std::ops::Range<usize>,
}

/// The link URL dialog: one field, and a real Insert. An empty or unusable address inserts
/// nothing — the dialog never leaves a placeholder link behind pretending to be a URL.
pub struct LinkDialog {
    url: Entity<InputState>,
    pending: Option<PendingLink>,
    _subs: Vec<Subscription>,
}

impl LinkDialog {
    fn new(pending: PendingLink, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let url = cx.new(|cx| InputState::new(window, cx).placeholder("https://example.com"));
        // `enter` in the field confirms, the way every other one-field dialog in the app does.
        let subs = vec![
            cx.subscribe_in(&url, window, |_, url, event: &InputEvent, _, cx| {
                if matches!(
                    event,
                    InputEvent::PressEnter {
                        secondary: false,
                        ..
                    }
                ) {
                    let value = url.read(cx).value().to_string();
                    cx.emit(LinkDialogEvent::Confirm(value));
                }
            }),
        ];
        Self {
            url,
            pending: Some(pending),
            _subs: subs,
        }
    }

    /// The pending selection and the address typed, once the user confirmed. An address
    /// that cannot be linked to comes back empty, and the caller inserts nothing.
    fn confirm(&mut self, url: &str) -> Option<(PendingLink, String)> {
        let pending = self.pending.take()?;
        let url = url.trim();
        if url.is_empty() || !markdown::safe_url(url) {
            return Some((pending, String::new()));
        }
        Some((pending, url.to_owned()))
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.url.read(cx).focus_handle(cx), cx);
    }
}

impl Focusable for LinkDialog {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.url.read(cx).focus_handle(cx).clone()
    }
}

impl EventEmitter<LinkDialogEvent> for LinkDialog {}

/// What the link dialog reports.
pub enum LinkDialogEvent {
    Confirm(String),
    Cancel,
}

impl Render for LinkDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_4()
            .child(div().text_sm().child("Link address"))
            .child(Input::new(&self.url).h(px(26.)).w_full())
            .child(
                div().text_size(px(11.)).text_color(muted).child(
                    "The selected words stay as the link's text. Leave this empty to cancel.",
                ),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .mt_1()
                    .child(
                        crate::app::ui::button(
                            "compose-link-cancel",
                            "Cancel",
                            "Leave the text as it was",
                            "escape",
                            cx,
                        )
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(LinkDialogEvent::Cancel))),
                    )
                    .child(
                        crate::app::ui::primary_button(
                            "compose-link-insert",
                            gpui_kit::assets::IconName::Link,
                            "Insert",
                            "Make the selected words a link to this address",
                            "enter",
                            cx,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            let url = this.url.read(cx).value().to_string();
                            cx.emit(LinkDialogEvent::Confirm(url));
                        })),
                    ),
            )
    }
}

/// What the conversion preview reports.
pub enum ConversionPreviewEvent {
    /// Adopt the converted Markdown; the body becomes editable as Markdown.
    Adopt,
    /// Leave the rich original in place; nothing changes.
    Cancel,
}

/// The preview of what an imported rich body would become as Markdown.
///
/// It shows the *converted* text, rendered exactly as the send will render it, not the
/// original's HTML: the question being answered is what this message will look like after
/// conversion, and the original is what the user already read. Nothing is converted until
/// "Use Markdown" is pressed, and "Keep the original" leaves the message exactly as it
/// arrived.
pub struct ConversionPreview {
    title: SharedString,
    /// The converted Markdown, rendered as it would be sent.
    html: SharedString,
    /// Each thing the conversion could not keep, named.
    warnings: Vec<String>,
    /// The dialog has no field, but it is focusable so `escape` and the keys reach it.
    focus: FocusHandle,
}

impl ConversionPreview {
    /// The preview of `body` as Markdown, or `None` when there is nothing to preview.
    pub fn of(subject: &str, body: &DraftBody, cx: &mut App) -> Option<Self> {
        let conversion = &body.original.as_ref()?.conversion;
        let source = conversion.markdown.as_str();
        if source.trim().is_empty() {
            return None;
        }
        Some(Self {
            title: subject.to_owned().into(),
            html: markdown::render_markdown(source).into(),
            warnings: describe_warnings(&conversion.warnings),
            focus: cx.focus_handle(),
        })
    }
}

impl Focusable for ConversionPreview {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl EventEmitter<ConversionPreviewEvent> for ConversionPreview {}

impl Render for ConversionPreview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Copied out rather than borrowed: the buttons below need the context mutably.
        let warning = cx.theme().warning;
        let border = cx.theme().border;
        let scroll = ScrollHandle::new();
        let detail = if self.warnings.is_empty() {
            "This message used formatting Markdown cannot keep.".to_owned()
        } else {
            format!("Markdown cannot keep: {}.", self.warnings.join(", "))
        };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_4()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!("As Markdown: {}", self.title)),
            )
            .child(div().text_size(px(11.)).text_color(warning).child(detail))
            .child(
                div()
                    .id("compose-conversion-preview")
                    .test_support()
                    .track_scroll(&scroll)
                    .h(px(240.))
                    .overflow_y_scroll()
                    .rounded_sm()
                    .border_1()
                    .border_color(border)
                    .p_3()
                    .child(
                        gpui_kit::component::text::TextView::html(
                            "compose-conversion-preview-body",
                            self.html.clone(),
                        )
                        .selectable(true)
                        .scrollable(false),
                    ),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        crate::app::ui::button(
                            "compose-conversion-cancel",
                            "Keep the original",
                            "Close this and leave the message exactly as it arrived",
                            "escape",
                            cx,
                        )
                        .on_click(
                            cx.listener(|_, _, _, cx| cx.emit(ConversionPreviewEvent::Cancel)),
                        ),
                    )
                    .child(
                        crate::app::ui::primary_button(
                            "compose-conversion-adopt",
                            gpui_kit::assets::IconName::Check,
                            "Use Markdown",
                            "Convert the message and make the body editable",
                            "enter",
                            cx,
                        )
                        .on_click(
                            cx.listener(|_, _, _, cx| cx.emit(ConversionPreviewEvent::Adopt)),
                        ),
                    ),
            )
    }
}

/// The composer.
pub struct Composer {
    draft: DraftRecord,
    window: AnyWindowHandle,
    placement: Placement,
    save_status: SharedString,
    attachments_busy: bool,
    to: Entity<Recipients>,
    cc: Entity<Recipients>,
    bcc: Entity<Recipients>,
    bcc_shown: bool,
    subject: Entity<InputState>,
    body: Entity<TextareaState>,
    preview_focus: FocusHandle,
    /// The scroll of the preview surface, so a preview body scrolls like the editor does.
    scroll: ScrollHandle,
    mode: BodyMode,
    pending_paste: Option<PendingPaste>,
    /// The subscription to the open link dialog. It is dropped when the next link is asked
    /// for, so a dialog that was dismissed cannot act on a document it no longer describes.
    link_sub: Option<Subscription>,
    /// The subscription to the open conversion preview, dropped with the next one.
    conversion_sub: Option<Subscription>,
    /// The rendered preview, cached: producing it parses and sanitizes the whole body.
    preview_html: SharedString,
    /// Set by every path that changes the body, cleared when the preview is rebuilt.
    preview_dirty: bool,
    /// The accounts the selector offers, as `(id, label)`.
    accounts: Vec<(String, String)>,
    /// An acknowledged record, applied after the current entity borrow is released.
    pending_record: Option<DraftRecord>,
    /// A hold or a send is in flight; the controls stop offering another one rather than
    /// pretending the message has not left.
    sending: bool,
    _subs: Vec<Subscription>,
}

impl Composer {
    /// Build the composer over `draft`. `focus` decides where the caret lands.
    pub fn new(
        draft: DraftRecord,
        placement: Placement,
        focus: Focus,
        accounts: Vec<(String, String)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let to = cx.new(|cx| Recipients::new(Field::To, &draft.to, "Recipients", window, cx));
        let cc = cx.new(|cx| Recipients::new(Field::Cc, &draft.cc, "Cc", window, cx));
        let bcc = cx.new(|cx| Recipients::new(Field::Bcc, &draft.bcc, "Bcc", window, cx));
        let subject = cx.new(|cx| InputState::new(window, cx).placeholder("Subject"));
        subject.update(cx, |s, cx| s.set_value(draft.subject.as_str(), window, cx));
        let placeholder = match draft.kind {
            DraftKind::Forward => "Add a note…",
            DraftKind::New => "Write your message…",
            _ => "Write your reply…",
        };
        let body = cx.new(|cx| TextareaState::new(window, cx).placeholder(placeholder));
        body.update(cx, |b, cx| {
            b.set_value(draft.body.source.as_str(), window, cx)
        });
        let mode = draft.body.mode;
        let bcc_shown = !draft.bcc.trim().is_empty();
        let mut subs = Vec::new();
        for entity in [&to, &cc, &bcc] {
            subs.push(
                cx.subscribe_in(entity, window, |this, _, event: &RecipientEvent, _, cx| {
                    if this.frozen() {
                        return;
                    }
                    let RecipientEvent::Changed { field, value } = event;
                    let current = match field {
                        Field::To => &mut this.draft.to,
                        Field::Cc => &mut this.draft.cc,
                        Field::Bcc => &mut this.draft.bcc,
                    };
                    if current != value {
                        current.clone_from(value);
                        cx.emit(ComposeEvent::Edited);
                    }
                }),
            );
        }
        subs.push(cx.subscribe_in(
            &subject,
            window,
            |this, subject, event: &InputEvent, _, cx| {
                if !matches!(event, InputEvent::Change) || this.frozen() {
                    return;
                }
                let value = subject.read(cx).value();
                if value.as_str() != this.draft.subject.as_str() {
                    this.draft.subject = value.to_string();
                    cx.emit(ComposeEvent::Edited);
                }
            },
        ));
        subs.push(
            cx.subscribe_in(&body, window, |this, body, event: &InputEvent, _, cx| {
                if !matches!(event, InputEvent::Change) || this.body_locked() {
                    return;
                }
                let text = body.read(cx).value();
                if text.as_str() == this.draft.body.source.as_str() {
                    return;
                }
                // Ignore our own formatting/hydration acknowledgements; only a new value is
                // an edit. This preserves undo history and avoids duplicate durable writes.
                this.retract_plain_offer(text.as_str());
                this.draft.body.set_source(text.to_string());
                this.invalidate_preview();
                cx.emit(ComposeEvent::Edited);
            }),
        );
        let mut this = Self {
            draft,
            window: window.window_handle(),
            placement,
            save_status: "Saved on this Mac".into(),
            attachments_busy: false,
            to,
            cc,
            bcc,
            bcc_shown,
            subject,
            body,
            preview_focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            mode,
            pending_paste: None,
            link_sub: None,
            conversion_sub: None,
            // The cache starts empty and stale, so the first preview is built from the body
            // rather than from a blank.
            preview_html: SharedString::default(),
            preview_dirty: true,
            accounts,
            pending_record: None,
            sending: false,
            _subs: subs,
        };
        this.sync_frozen_fields(cx);
        match focus {
            Focus::Body => this.focus_body(window, cx),
            Focus::Recipients => this.focus_recipients(window, cx),
        }
        this
    }

    pub fn draft(&self) -> &DraftRecord {
        &self.draft
    }

    pub fn id(&self) -> DraftId {
        self.draft.id
    }

    pub fn placement(&self) -> Placement {
        self.placement
    }

    pub fn set_placement(&mut self, placement: Placement, cx: &mut Context<Self>) {
        self.placement = placement;
        cx.notify();
    }

    /// The record as the form currently reads it.
    pub fn record(&self) -> DraftRecord {
        self.draft.clone()
    }

    /// Adopt a record the owner changed underneath us (a recovered hold, a sync result).
    /// The form follows; the user is not typing into it at this point.
    /// Widget updates are deferred into the owning window, never performed in render.
    pub fn apply_record(&mut self, draft: DraftRecord, cx: &mut Context<Self>) {
        let handle = self.window;
        let view = cx.weak_entity();
        self.pending_record = Some(draft);
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| {
                let _ = view.update(cx, |this, cx| this.apply_pending_record(window, cx));
            });
        });
        cx.notify();
    }

    /// Fill the inputs from a record the owner adopted.
    fn apply_pending_record(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = self.pending_record.take() else {
            return;
        };
        let to_changed = self.draft.to != draft.to;
        let cc_changed = self.draft.cc != draft.cc;
        let bcc_changed = self.draft.bcc != draft.bcc;
        let subject_changed = self.draft.subject != draft.subject;
        let source_changed = self.draft.body.source != draft.body.source;
        let body_changed = self.draft.body != draft.body;
        self.draft = draft;
        if to_changed {
            self.to
                .update(cx, |r, cx| r.set_value(self.draft.to.as_str(), window, cx));
        }
        if cc_changed {
            self.cc
                .update(cx, |r, cx| r.set_value(self.draft.cc.as_str(), window, cx));
        }
        if bcc_changed {
            self.bcc
                .update(cx, |r, cx| r.set_value(self.draft.bcc.as_str(), window, cx));
        }
        if subject_changed {
            self.subject.update(cx, |s, cx| {
                s.set_value(self.draft.subject.as_str(), window, cx)
            });
        }
        if source_changed {
            self.body.update(cx, |b, cx| {
                b.set_value(self.draft.body.source.as_str(), window, cx)
            });
            self.pending_paste = None;
        }
        self.mode = self.draft.body.mode;
        self.bcc_shown = self.bcc_shown || !self.draft.bcc.trim().is_empty();
        if body_changed {
            self.invalidate_preview();
        }
        self.sync_frozen_fields(cx);
        cx.notify();
    }

    /// The HTML the preview draws. It is derived from the body — the same bytes the send
    /// will carry, with a preserved rich original shown through the safety filter — and
    /// producing it parses and sanitizes the whole message, so it is cached and rebuilt only
    /// when the body actually changed.
    fn preview_html(&mut self) -> SharedString {
        if self.preview_dirty {
            self.preview_html = self.draft.body.preview().html.into();
            self.preview_dirty = false;
        }
        self.preview_html.clone()
    }

    /// Mark the cached preview stale. Every path that changes the body calls this — a
    /// keystroke, a format edit, an approved conversion, an adopted record — so an idle
    /// repaint never re-parses the message and a stale preview is never drawn.
    fn invalidate_preview(&mut self) {
        self.preview_dirty = true;
    }

    /// The accounts the selector offers, refreshed by the owner as accounts come and go.
    pub fn set_accounts(&mut self, accounts: Vec<(String, String)>, cx: &mut Context<Self>) {
        self.accounts = accounts;
        cx.notify();
    }

    /// The owner's durability status, independent of the editable message body.
    pub fn set_save_status(
        &mut self,
        status: String,
        attachments_busy: bool,
        cx: &mut Context<Self>,
    ) {
        let status: SharedString = status.into();
        if self.save_status != status || self.attachments_busy != attachments_busy {
            self.save_status = status;
            self.attachments_busy = attachments_busy;
            cx.notify();
        }
    }

    /// The body surface's focus handle, so the owner can put the caret back after a send,
    /// an undo or an import.
    pub fn body_focus(&self, cx: &App) -> FocusHandle {
        if self.mode == BodyMode::Preview || self.draft.body.needs_conversion() {
            self.preview_focus.clone()
        } else {
            self.body.read(cx).focus_handle(cx).clone()
        }
    }

    pub fn focus_body(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.body_focus(cx), cx);
    }

    fn focus_recipients(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.to.update(cx, |r, cx| r.focus(window, cx));
    }

    /// The owner marks a send request in flight, so the controls stop offering one.
    pub fn set_sending(&mut self, sending: bool, cx: &mut Context<Self>) {
        self.sending = sending;
        self.sync_frozen_fields(cx);
        cx.notify();
    }

    /// Whether the body is editable right now. A held, sending or unconfirmed draft is not,
    /// and neither is an imported rich body whose conversion has not been decided: its text
    /// would be sent as the untouched original, so typing would be a lie.
    pub fn body_locked(&self) -> bool {
        self.frozen() || self.draft.body.needs_conversion()
    }

    /// The current Markdown source and the selection the format helpers act on.
    fn selection(&self, cx: &App) -> (String, std::ops::Range<usize>) {
        let state = self.body.read(cx);
        (state.value().to_string(), state.selected_range())
    }

    /// Run a pure Markdown edit over the current selection, put the text and the caret
    /// back where the helper says they belong, and report the edit.
    fn apply(&mut self, edit: markdown::Edit, window: &mut Window, cx: &mut Context<Self>) {
        let markdown::Edit { text, selection } = edit;
        self.body.update(cx, |b, cx| {
            b.replace_all(text, window, cx);
            b.set_selected_range(selection, cx);
        });
        let source = self.body.read(cx).value().to_string();
        self.retract_plain_offer(&source);
        self.draft.body.set_source(source);
        self.invalidate_preview();
        cx.emit(ComposeEvent::Edited);
        window.focus(&self.body.read(cx).focus_handle(cx), cx);
    }

    /// Drop a plain-paste offer whose document is no longer the one on screen.
    fn retract_plain_offer(&mut self, source: &str) {
        if self
            .pending_paste
            .as_ref()
            .is_some_and(|p| p.plain_range(source).is_none())
        {
            self.pending_paste = None;
        }
    }

    fn apply_inline(&mut self, format: InlineFormat, window: &mut Window, cx: &mut Context<Self>) {
        if self.body_locked() || self.mode == BodyMode::Preview {
            return;
        }
        let (src, sel) = self.selection(cx);
        let edit = markdown::toggle_inline(&src, sel, format);
        self.apply(edit, window, cx);
    }

    fn apply_block(&mut self, format: BlockFormat, window: &mut Window, cx: &mut Context<Self>) {
        if self.body_locked() || self.mode == BodyMode::Preview {
            return;
        }
        let (src, sel) = self.selection(cx);
        let edit = markdown::toggle_block(&src, sel, format);
        self.apply(edit, window, cx);
    }

    /// A link needs a real address, so it is asked for.
    ///
    /// A selection that is already a URL becomes the link in one step — there is nothing to
    /// ask. Anything else opens the URL dialog with the selected words as the link's text:
    /// no placeholder goes into the document, because an ellipsis standing in for a URL
    /// would be sent as a broken link.
    fn apply_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.body_locked() || self.mode == BodyMode::Preview {
            return;
        }
        let (src, sel) = self.selection(cx);
        let selected = src.get(sel.clone()).unwrap_or_default().trim().to_owned();
        if markdown::safe_url(&selected) {
            let bare = selected
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .to_owned();
            let text = (selected != bare).then_some(selected.as_str());
            let edit = markdown::insert_link(&src, sel, &bare, text);
            self.apply(edit, window, cx);
            return;
        }
        self.open_link_dialog(src, sel, window, cx);
    }

    /// Open the URL dialog for a pending selection. The selection is held here so the words
    /// stay the link's text however long the dialog is open.
    fn open_link_dialog(
        &mut self,
        source: String,
        selection: std::ops::Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Only one link is asked for at a time: the previous question goes with its
        // subscription, so a dismissed dialog cannot act on this document later.
        drop(self.link_sub.take());
        let pending = PendingLink { source, selection };
        let dialog = cx.new(|cx| LinkDialog::new(pending, window, cx));
        self.link_sub = Some(cx.subscribe_in(
            &dialog,
            window,
            |this, dialog, event: &LinkDialogEvent, window, cx| {
                let LinkDialogEvent::Confirm(typed) = event else {
                    // Cancelled, by button or by escape: the document is exactly as it was.
                    return;
                };
                let confirmed = dialog.update(cx, |d, _| d.confirm(typed));
                window.close_dialog(cx);
                let Some((pending, url)) = confirmed else {
                    return;
                };
                if url.is_empty() {
                    // Nothing usable was typed: no link, and nothing said otherwise.
                    return;
                }
                // The document the words were chosen from must still be the one on screen,
                // or the link would be written over whatever happened in between.
                if this.draft.body.source != pending.source {
                    return;
                }
                let selected = pending
                    .source
                    .get(pending.selection.clone())
                    .unwrap_or_default()
                    .trim()
                    .to_owned();
                // With nothing selected the link has no words yet, so the caret goes in
                // between the brackets and the label can be typed next.
                let edit = if selected.is_empty() {
                    markdown::insert_link(&pending.source, pending.selection, &url, None)
                } else {
                    markdown::insert_link(&pending.source, pending.selection, &url, Some(&selected))
                };
                this.apply(edit, window, cx);
            },
        ));
        // The kit builds this dialog as an `Fn`, so each inner handler takes its own clone
        // of what it needs: moving one out of the outer closure would make it `FnOnce`.
        let focus_dialog = dialog.clone();
        window.open_dialog(cx, move |kit, _, _| {
            let dismissed = dialog.clone();
            let content_view = dialog.clone();
            kit.close_button(false)
                // `enter` belongs to the field, which emits its own confirmation, and the
                // buttons act on the view rather than the dialog.
                .on_ok(|_, _, _| false)
                .on_close(move |_, _, cx| {
                    dismissed.update(cx, |_, cx| cx.emit(LinkDialogEvent::Cancel));
                })
                .p_0()
                .w(px(420.))
                .title("Insert link")
                .content(move |content, _, _| content.child(content_view.clone()))
        });
        focus_dialog.update(cx, |d, cx| d.focus(window, cx));
        cx.notify();
    }

    /// A table with one header row and two body rows is what the control promises; a
    /// Markdown table the user can then edit.
    fn apply_table(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.body_locked() || self.mode == BodyMode::Preview {
            return;
        }
        let (src, sel) = self.selection(cx);
        let edit = markdown::insert_table(&src, sel, 3, 2);
        self.apply(edit, window, cx);
    }

    /// Switch between editing the Markdown source and the rendered preview.
    fn toggle_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = match self.mode {
            BodyMode::Source => BodyMode::Preview,
            BodyMode::Preview => BodyMode::Source,
        };
        self.draft.body.set_mode(self.mode);
        cx.emit(ComposeEvent::Edited);
        self.focus_body(window, cx);
        cx.notify();
    }

    /// Paste a clipboard payload through the body's own conversion, so a rich paste
    /// becomes Markdown — and says what it could not keep — instead of raw HTML.
    ///
    /// `false` when there is nothing this path can paste — an image or a file — so the
    /// platform inserts it itself. A paste into a body that is not editable is consumed
    /// rather than declined: letting the platform write it would edit text that is not the
    /// text this message will carry.
    fn paste_item(
        &mut self,
        item: gpui_kit::ClipboardItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.body_locked() {
            return true;
        }
        let mut clip = Clipboard {
            text: item.text().unwrap_or_default(),
            ..Default::default()
        };
        for entry in item.entries() {
            // Rich clipboard flavours carry their flavour in the metadata; `text/html` is
            // the one worth converting, `text/markdown` is already what we want.
            let gpui_kit::ClipboardEntry::String(text) = entry else {
                continue;
            };
            // The flavour is read as an exact MIME type. The metadata is opaque private
            // JSON — a substring match would convert plain text into HTML because some
            // unrelated field happened to name that type.
            match mime_flavor(text.metadata.as_deref()).as_deref() {
                Some("text/html") => clip.html = Some(text.text.clone()),
                Some("text/markdown") => clip.markdown = Some(text.text.clone()),
                _ => {}
            }
        }
        // Nothing declared an HTML flavour, but the copy may well have one: a rich selection
        // from another application reaches us as its plain text alone. The pasteboard is
        // asked for the HTML of exactly that text.
        if clip.html.is_none() && clip.markdown.is_none() {
            clip.html = pasteboard_html(&clip.text);
        }
        if clip.text.trim().is_empty() && clip.html.is_none() {
            return false;
        }
        // The body state is the source of truth for what is on screen; the record's copy is
        // brought level with it before the range of the insertion is worked out.
        let source = self.body.read(cx).value().to_string();
        self.draft.body.set_source(source.clone());
        let sel = boundaries(&source, self.body.read(cx).selected_range());
        let result = self.draft.body.paste(sel.clone(), &clip);
        let edit = result.edit;
        // The paste is applied through the state's own edit so the caret, the undo stack
        // and the emitted `Change` all behave as if the user had typed it.
        self.body.update(cx, |b, cx| {
            b.replace_all(edit.text.clone(), window, cx);
            b.set_selected_range(edit.selection.clone(), cx);
        });
        let pasted = self.body.read(cx).value().to_string();
        self.draft.body.set_source(pasted.clone());
        self.invalidate_preview();
        // The converted text runs from where the insertion started to the caret it left
        // behind, which is exactly what "paste as text" has to be able to take back.
        if let Some(conversion) = result.conversion {
            self.pending_paste = Some(PendingPaste {
                plain: clip.text.clone(),
                warnings: describe_warnings(&conversion.warnings),
                range: sel.start..edit.selection.start.max(sel.start),
                source: pasted,
            });
        } else {
            self.pending_paste = None;
        }
        cx.emit(ComposeEvent::Edited);
        true
    }

    /// Paste the last rich paste as plain text instead of its conversion, replacing the
    /// converted insertion itself rather than adding the plain text wherever the caret has
    /// since moved to. The escape hatch for a conversion that lost something worth keeping.
    fn paste_plain(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.body_locked() {
            return;
        }
        let Some(pending) = self.pending_paste.take() else {
            return;
        };
        let source = self.body.read(cx).value().to_string();
        // The offer describes one document. If this is another one, the offer was retracted
        // already and nothing is inserted: a range from before would swallow what is there.
        let Some(range) = pending.plain_range(&source) else {
            return;
        };
        self.draft.body.set_source(source);
        // The same body rules, with no rich flavour: plain text goes in as prose, keeping
        // its line breaks, with no Markdown markup invented from it.
        let result = self
            .draft
            .body
            .paste(range, &Clipboard::plain(pending.plain));
        self.apply(result.edit, window, cx);
    }

    /// Show what this imported message becomes as Markdown, before anything is converted.
    fn open_conversion_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.frozen() {
            return;
        }
        let Some(preview) = ConversionPreview::of(&self.draft.subject, &self.draft.body, cx) else {
            return;
        };
        // One question at a time: the previous preview goes with its subscription.
        drop(self.conversion_sub.take());
        let view = cx.new(|_cx| preview);
        self.conversion_sub = Some(cx.subscribe_in(
            &view,
            window,
            |this, _, event: &ConversionPreviewEvent, window, cx| {
                match event {
                    // Only the preview's own decision converts the body.
                    ConversionPreviewEvent::Adopt => {
                        window.close_dialog(cx);
                        this.approve_conversion(window, cx);
                    }
                    // Kept the original: the message stays exactly as it arrived, and the
                    // kit closes the dialog itself.
                    ConversionPreviewEvent::Cancel => {}
                }
                cx.notify();
            },
        ));
        // As with the link dialog, the kit builds this as an `Fn`: each handler takes its
        // own clone rather than moving one out of the builder.
        let focus = view.read(cx).focus_handle(cx);
        window.open_dialog(cx, move |kit, _, _| {
            let adopting = view.clone();
            let dismissed = view.clone();
            let content_view = view.clone();
            kit.close_button(false)
                // Escape keeps the original; enter adopts the conversion, the same as the
                // button it is the keycap for. Neither closes the dialog itself: the view
                // acts first, and closes only once it has.
                .on_ok(move |_, _, cx| {
                    adopting.update(cx, |_, cx| cx.emit(ConversionPreviewEvent::Adopt));
                    false
                })
                .on_cancel(|_, _, _| true)
                .on_close(move |_, _, cx| {
                    dismissed.update(cx, |_, cx| cx.emit(ConversionPreviewEvent::Cancel));
                })
                .p_0()
                .w(px(560.))
                .title("Convert to Markdown")
                .content(move |content, _, _| content.child(content_view.clone()))
        });
        // The dialog focuses its content rather than a field of its own, so the keys reach
        // it with nothing between.
        window.focus(&focus, cx);
        cx.notify();
    }

    /// Approve the Markdown conversion of an imported rich body. The original is kept on
    /// the record either way, so nothing is lost by approving.
    fn approve_conversion(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.draft.body.approve_conversion();
        let source = self.draft.body.source.clone();
        // A conversion is one undoable editor replacement; the original remains retained.
        self.body
            .update(cx, |b, cx| b.replace_all(source, window, cx));
        self.invalidate_preview();
        cx.emit(ComposeEvent::Edited);
    }

    /// Show or hide Cc and Bcc, keeping whatever is already typed there.
    fn toggle_bcc(&mut self, cx: &mut Context<Self>) {
        self.bcc_shown = !self.bcc_shown;
        cx.notify();
    }

    /// The one-line status under the composer: what is true about this draft right now.
    fn status(&self) -> SharedString {
        match &self.draft.state {
            DraftState::Editing | DraftState::Failed { .. } | DraftState::Conflict => {
                self.save_status.clone()
            }
            DraftState::Held => "Held — undo restores it".into(),
            DraftState::Sending => "Sending…".into(),
            DraftState::Sent { .. } => "Sent".into(),
            DraftState::Uncertain => self.save_status.clone(),
        }
    }

    /// Whether the draft may not be touched right now. A held send is waiting out its
    /// countdown and a sending or unconfirmed one may already have left, so editing it
    /// would be editing a message that is not the one being sent.
    fn frozen(&self) -> bool {
        self.sending
            || matches!(
                self.draft.state,
                DraftState::Held
                    | DraftState::Sending
                    | DraftState::Uncertain
                    | DraftState::Sent { .. }
            )
    }

    /// Whether Send may be offered: the draft is not held, not in flight, and its state
    /// says a fresh request is meaningful.
    fn can_send(&self) -> bool {
        !self.frozen()
            && matches!(
                self.draft.state,
                DraftState::Editing | DraftState::Failed { .. }
            )
    }

    /// Closing is always allowed: a closed draft is kept, never deleted.
    fn can_close(&self) -> bool {
        true
    }
}

impl Focusable for Composer {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.body_focus(cx)
    }
}

impl EventEmitter<ComposeEvent> for Composer {}

impl Composer {
    fn header_row(&self, label: &'static str, field: Entity<Recipients>, cx: &App) -> Div {
        let t = cx.theme();
        div()
            .flex()
            .flex_none()
            .items_start()
            .gap_2()
            .py_1()
            .child(
                div()
                    .w(px(44.))
                    .flex_none()
                    .pt_1()
                    .text_sm()
                    .text_color(t.muted_foreground)
                    .child(label),
            )
            .child(div().flex_1().min_w_0().child(field))
    }

    fn subject_row(&self, cx: &App) -> Div {
        let t = cx.theme();
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .py_1()
            .border_b(px(1.))
            .border_color(t.border)
            .child(
                div()
                    .w(px(56.))
                    .flex_none()
                    .text_sm()
                    .text_color(t.muted_foreground)
                    .child("Subject"),
            )
            .child(
                div()
                    .id("compose-subject")
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .child(
                        Input::new(&self.subject)
                            .h(px(22.))
                            .bordered(false)
                            .disabled(self.frozen()),
                    ),
            )
    }

    /// The sending account, as a menu of the accounts this mailbox holds. Choosing one is
    /// an edit like any other, so the record and the send follow it.
    ///
    /// With one account there is nothing to choose, so the control is not even shown: a
    /// selector that cannot change anything only takes room. While the message is held or
    /// on its way the account it goes from is still shown, but it cannot be changed: the
    /// message is that account's now.
    fn account_selector(&self, cx: &App) -> Option<DropdownButton> {
        if self.accounts.len() < 2 {
            return None;
        }
        let current = self
            .accounts
            .iter()
            .find(|(id, _)| *id == self.draft.account)
            .map(|(_, label)| label.clone())
            .unwrap_or_else(|| self.draft.account.clone());
        let button = crate::app::ui::button(
            "compose-account-label",
            current,
            "Send this message from a different account",
            "",
            cx,
        )
        .disabled(self.frozen());
        Some(
            DropdownButton::new("compose-account")
                .disabled(self.frozen())
                .button(button),
        )
    }

    /// The account menu's rows, so a click runs exactly what choosing the account does.
    fn account_rows(&self) -> Vec<(String, String)> {
        self.accounts
            .iter()
            .map(|(id, label)| (label.clone(), id.clone()))
            .collect()
    }

    fn render_headers(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> Div {
        // The colour is copied out rather than the theme borrowed: the row's own click
        // handler needs the context mutably, and a live theme borrow would forbid it.
        let muted = cx.theme().muted_foreground;
        let frozen = self.frozen();
        let app = cx.weak_entity();
        let account = self.account_selector(cx).map(|selector| {
            // The rows are read when the menu opens, so the closure owns nothing that would
            // have to outlive the frame.
            selector.dropdown_menu_with_anchor(Anchor::BottomRight, move |menu, window, cx| {
                let Some(this) = app.upgrade() else {
                    return menu;
                };
                let rows: Vec<(String, String)> = this.read(cx).account_rows();
                let owner = app.clone();
                // A chosen row dispatches exactly what the action would, so the composer
                // hears it through the same handler as any other key or click.
                let run: crate::app::menu::MenuRun = Rc::new(move |action, window, cx| {
                    owner
                        .update(cx, |_, cx| window.dispatch_action(action, cx))
                        .ok();
                });
                let items: Vec<crate::app::menu::MenuItem> = rows
                    .into_iter()
                    .map(|(label, id)| {
                        crate::app::menu::MenuItem::action_fn(label, move || {
                            Box::new(ComposeAccount {
                                account: id.clone(),
                            }) as Box<dyn Action>
                        })
                    })
                    .collect();
                crate::app::menu::populate(menu, &items, &run, window, cx)
            })
        });
        div()
            .flex()
            .flex_none()
            .flex_col()
            .px_3()
            .pt_2()
            .child(self.header_row("To", self.to.clone(), cx))
            .child(self.header_row("Cc", self.cc.clone(), cx))
            .when(self.bcc_shown, |d| {
                d.child(self.header_row("Bcc", self.bcc.clone(), cx))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pb_1()
                    .child(
                        crate::app::ui::button(
                            "compose-show-cc",
                            if self.bcc_shown {
                                "Hide Cc/Bcc"
                            } else {
                                "Cc/Bcc"
                            },
                            "Show or hide the carbon-copy fields",
                            "",
                            cx,
                        )
                        .disabled(frozen)
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_bcc(cx))),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(muted)
                            .child(self.draft.kind.label()),
                    )
                    .when_some(account, |d, el| d.child(el)),
            )
            .child(self.subject_row(cx))
    }

    /// The compact toolbar: the everyday formats inline, the rest behind the overflow.
    fn render_toolbar(&mut self, cx: &mut Context<Self>) -> Div {
        // A held or in-flight message is not edited, and neither is an imported rich body
        // whose conversion is still undecided, so the formatting controls do not pretend to
        // be available.
        let locked = self.body_locked() || self.mode == BodyMode::Preview;
        let tools = toolbar::INLINE_TOOLS.iter();
        // The overflow's rows dispatch the same actions the chords do, through the same
        // weak handle the account menu uses.
        let app = cx.weak_entity();
        let overflow = DropdownButton::new("compose-more-format")
            .disabled(locked)
            .button(
                Button::new("compose-more-format-label")
                    .icon(gpui_kit::assets::IconName::Ellipsis)
                    .ghost()
                    .xsmall()
                    .h(px(22.))
                    .w(px(24.))
                    .px_0(),
            )
            .dropdown_menu_with_anchor(Anchor::BottomLeft, move |menu, window, cx| {
                let owner = app.clone();
                let run: crate::app::menu::MenuRun = Rc::new(move |action, window, cx| {
                    owner
                        .update(cx, |_, cx| window.dispatch_action(action, cx))
                        .ok();
                });
                crate::app::menu::populate(
                    menu,
                    &toolbar::more_tools()
                        .iter()
                        .filter_map(|tool| {
                            let action = tool_action(tool.id)?;
                            Some(crate::app::menu::MenuItem::action_fn(
                                tool.label.to_owned(),
                                move || action(),
                            ))
                        })
                        .collect::<Vec<_>>(),
                    &run,
                    window,
                    cx,
                )
            })
            .into_any_element();
        div()
            .flex()
            .flex_none()
            .child(toolbar::bar(tools, locked, overflow, None, cx))
    }

    /// The banner shown while an imported rich body has unapproved, lossy Markdown.
    ///
    /// It offers one way through: the preview. The conversion is never adopted from here,
    /// because what is being lost is only visible in what the message would become.
    fn render_conversion(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let border = cx.theme().border;
        if !self.draft.body.needs_conversion() {
            return None;
        }
        let original = self.draft.body.original.as_ref()?;
        let dropped = describe_warnings(&original.conversion.warnings);
        let detail = if dropped.is_empty() {
            "This message arrived with formatting Markdown cannot keep.".to_owned()
        } else {
            format!(
                "Arrived with formatting Markdown cannot keep: {}.",
                dropped.join(", ")
            )
        };
        Some(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_1()
                .border_b(px(1.))
                .border_color(border)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(11.))
                        .child(format!("{detail} It is sent as it arrived.")),
                )
                .child(
                    crate::app::ui::button(
                        "compose-preview-conversion",
                        "Preview Markdown…",
                        "See what this message becomes as Markdown, then decide",
                        "",
                        cx,
                    )
                    .disabled(self.frozen())
                    .on_click(
                        cx.listener(|this, _, window, cx| this.open_conversion_preview(window, cx)),
                    ),
                )
                .into_any_element(),
        )
    }

    /// The banner shown while a rich paste's conversion waits for a decision.
    fn render_pending_paste(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let border = cx.theme().border;
        if self.frozen() {
            return None;
        }
        let pending = self.pending_paste.as_ref()?;
        Some(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_1()
                .border_b(px(1.))
                .border_color(border)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(11.))
                        .child(pending.detail()),
                )
                .child(
                    crate::app::ui::button(
                        "compose-paste-plain",
                        "Paste as text",
                        "Replace what was pasted with its plain text instead",
                        "",
                        cx,
                    )
                    .on_click(cx.listener(|this, _, window, cx| this.paste_plain(window, cx))),
                )
                .into_any_element(),
        )
    }

    fn render_attachments(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        // No attachments means no strip: an empty bar would leave a divider and a blank
        // line between the editor and the footer.
        if self.draft.attachments.is_empty() {
            return None;
        }
        // The chip's own remove handler needs the context mutably, so the border colour is
        // copied out rather than borrowed from the theme for the length of the map.
        let border = cx.theme().border;
        let frozen = self.frozen() || self.attachments_busy;
        let attachments = &self.draft.attachments;
        let chips = attachments.iter().map(|a: &ManagedAttachment| {
            let id = a.id.clone();
            div()
                .flex()
                .flex_none()
                .max_w(px(240.))
                .h(px(20.))
                .pl_2()
                .pr_1()
                .gap_1()
                .items_center()
                .rounded_sm()
                .border_1()
                .border_color(border)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_size(px(11.))
                        .child(format!("{} · {}", a.file_name, size_label(a.size))),
                )
                .child(
                    Button::new("compose-attachment-remove")
                        .icon(gpui_kit::assets::IconName::Close)
                        .ghost()
                        .xsmall()
                        .w(px(14.))
                        .h(px(14.))
                        .px_0()
                        .disabled(frozen)
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(ComposeEvent::RemoveAttachment(id.clone()))
                        })),
                )
        });
        let bar = div()
            .flex_none()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .px_3()
            .py_1()
            .border_t(px(1.))
            .border_color(border)
            .children(chips)
            .into_any_element();
        Some(bar)
    }

    fn render_footer(&mut self, cx: &mut Context<Self>) -> Div {
        // The colours are copied out rather than the theme borrowed: every button below
        // needs the context mutably, which a live theme borrow would forbid.
        let border = cx.theme().border;
        let warning = cx.theme().warning;
        let muted = cx.theme().muted_foreground;
        let status = self.status();
        let frozen = self.frozen();
        let conflicted = matches!(self.draft.state, DraftState::Conflict);
        let previewing = self.mode == BodyMode::Preview;
        let inline = self.placement == Placement::Inline;
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .border_t(px(1.))
            .border_color(border)
            .child(
                div()
                    .id("compose-status")
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(11.))
                    .text_color(match self.draft.state {
                        DraftState::Failed { .. }
                        | DraftState::Uncertain
                        | DraftState::Conflict => warning,
                        _ => muted,
                    })
                    .child(status),
            )
            .child(
                crate::app::ui::icon_button(
                    "compose-attach",
                    gpui_kit::assets::IconName::Paperclip,
                    "Attach files",
                    "cmd-shift-a",
                    cx,
                )
                .disabled(frozen || self.attachments_busy)
                .on_click(cx.listener(|_, _, _, cx| cx.emit(ComposeEvent::Attach))),
            )
            .child(
                crate::app::ui::icon_button(
                    "compose-preview",
                    if previewing {
                        gpui_kit::assets::IconName::FileText
                    } else {
                        gpui_kit::assets::IconName::Eye
                    },
                    if previewing {
                        "Edit the Markdown source"
                    } else {
                        "Preview the outgoing message"
                    },
                    "cmd-shift-v",
                    cx,
                )
                .on_click(cx.listener(|this, _, window, cx| this.toggle_preview(window, cx))),
            )
            .child(if inline {
                crate::app::ui::button(
                    "compose-expand",
                    "Expand",
                    "Move this draft into its own tab",
                    "",
                    cx,
                )
                .on_click(cx.listener(|_, _, _, cx| cx.emit(ComposeEvent::Expand)))
            } else {
                crate::app::ui::button(
                    "compose-collapse",
                    "Collapse",
                    "Move this draft back under its thread",
                    "",
                    cx,
                )
                .disabled(self.draft.source_message.is_none())
                .on_click(cx.listener(|_, _, _, cx| cx.emit(ComposeEvent::Collapse)))
            })
            // A conflicted draft is not left in a dead end: the choice is right here, and
            // the local snapshot is kept until it is made.
            .when(conflicted, |d| {
                d.child(
                    crate::app::ui::button(
                        "compose-resolve-conflict",
                        "Resolve…",
                        "Choose between your copy, the server's, or keeping both",
                        "",
                        cx,
                    )
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(ComposeEvent::ResolveConflict))),
                )
            })
            .when(matches!(self.draft.state, DraftState::Uncertain), |d| {
                d.child(
                    crate::app::ui::button(
                        "compose-check-delivery",
                        "Check delivery",
                        "Look in Sent without sending again",
                        "",
                        cx,
                    )
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(ComposeEvent::CheckDelivery))),
                )
            })
            .child(
                crate::app::ui::button(
                    "compose-discard",
                    "Discard",
                    "Delete this draft; it asks first",
                    "",
                    cx,
                )
                .disabled(frozen)
                .on_click(cx.listener(|_, _, _, cx| cx.emit(ComposeEvent::Discard))),
            )
            .child(
                crate::app::ui::button(
                    "compose-close",
                    "Close",
                    "Close the composer; the draft is kept",
                    "escape",
                    cx,
                )
                .disabled(!self.can_close())
                .on_click(cx.listener(|_, _, _, cx| cx.emit(ComposeEvent::Close))),
            )
            // The hold is a fixed ten seconds with an Undo, not a delay to skip: while a
            // message is held the control offers nothing, so pressing Send again cannot
            // send it early. Undo is on the countdown toast, per draft.
            .child(
                crate::app::ui::primary_button(
                    "compose-send",
                    gpui_kit::assets::IconName::Send,
                    "Send",
                    "Send; the message is held for ten seconds so it can be recalled",
                    "cmd-enter",
                    cx,
                )
                .disabled(!self.can_send())
                .on_click(cx.listener(|_, _, _, cx| cx.emit(ComposeEvent::Send))),
            )
    }

    /// The body surface: the editor, or the rendered preview of what will be sent.
    fn render_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if self.mode == BodyMode::Preview {
            // The preview draws the very HTML the message carries, through the kit's own
            // renderer, so what is previewed is what a recipient reads — including the
            // styling Markdown cannot express, when the body is a preserved rich original.
            return div()
                .id("compose-preview")
                .test_support()
                .track_scroll(&self.scroll)
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .p_3()
                .child(
                    gpui_kit::component::text::TextView::html(
                        "compose-preview-body",
                        self.preview_html(),
                    )
                    .selectable(true)
                    .scrollable(false),
                )
                .vertical_scrollbar(&self.scroll)
                .into_any_element();
        }
        // The textarea owns the paste so a rich clipboard payload is converted by the
        // body's own rules instead of being dropped to plain text by the platform.
        let app = cx.weak_entity();
        Textarea::new(&self.body)
            .h_full()
            .flex_1()
            .min_h_0()
            .appearance(false)
            .bordered(false)
            .p_3()
            .disabled(self.body_locked())
            .on_paste(move |item, window, cx| {
                app.update(cx, |this, cx| this.paste_item(item.clone(), window, cx))
                    .unwrap_or(false)
            })
            .into_any_element()
    }

    /// Keep the recipient fields' disabled state in step with the draft's: a held or
    /// in-flight message stops accepting new recipients rather than typing them into a
    /// draft that is already going out.
    fn sync_frozen_fields(&mut self, cx: &mut Context<Self>) {
        let frozen = self.frozen();
        for field in [&self.to, &self.cc, &self.bcc] {
            field.update(cx, |r, cx| {
                if r.disabled != frozen {
                    r.set_disabled(frozen, cx);
                }
            });
        }
    }
}

/// The action a formatting control dispatches, by the tool's id.
///
/// The id is what both the button and the menu row carry, so a click and a chord reach the
/// same action; a tool with no action returns `None` and its button says so by being
/// disabled rather than doing nothing when pressed.
pub(crate) fn tool_action(id: &str) -> Option<fn() -> Box<dyn Action>> {
    Some(match id {
        "compose-bold" => || Box::new(ComposeBold),
        "compose-italic" => || Box::new(ComposeItalic),
        "compose-link" => || Box::new(ComposeLink),
        "compose-bullet-list" => || Box::new(ComposeBulletedList),
        "compose-numbered-list" => || Box::new(ComposeNumberedList),
        "compose-heading" => || Box::new(ComposeHeading),
        "compose-quote" => || Box::new(ComposeQuote),
        "compose-code" => || Box::new(ComposeCodeBlock),
        "compose-strike" => || Box::new(ComposeStrike),
        "compose-table" => || Box::new(ComposeTable),
        _ => return None,
    })
}

/// `1.2 MB` / `512 KB` / `90 B`, for an attachment chip.
fn size_label(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1 << 20) as f64),
        b if b >= 1 << 10 => format!("{} KB", b >> 10),
        b => format!("{b} B"),
    }
}

/// A short, readable line for each thing a conversion dropped.
pub fn describe_warnings(warnings: &[ConversionWarning]) -> Vec<String> {
    warnings.iter().map(ToString::to_string).collect()
}

impl Render for Composer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = self.render_body(cx);
        // The background is copied out rather than the theme borrowed: every `on_action`
        // below needs the context mutably, which a live theme borrow would forbid.
        let background = cx.theme().background;
        div()
            .key_context(COMPOSE_CONTEXT)
            .track_focus(&self.preview_focus)
            // Child controls own focus; the mailbox's background click must not steal it.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            // While a message is held or in flight only what is still true of it is offered:
            // closing keeps it, undo recalls it, and nothing pretends it can be edited or
            // sent again.
            .on_action(cx.listener(|this, _: &SendDraft, _, cx| {
                if this.can_send() {
                    cx.emit(ComposeEvent::Send)
                }
            }))
            .on_action(cx.listener(|_, _: &CloseComposer, _, cx| cx.emit(ComposeEvent::Close)))
            .on_action(cx.listener(|this, _: &DiscardDraft, _, cx| {
                if !this.frozen() {
                    cx.emit(ComposeEvent::Discard)
                }
            }))
            .on_action(
                cx.listener(|_, _: &ResolveConflict, _, cx| cx.emit(ComposeEvent::ResolveConflict)),
            )
            .on_action(cx.listener(|this, _: &CheckDelivery, _, cx| {
                if matches!(this.draft.state, DraftState::Uncertain) {
                    cx.emit(ComposeEvent::CheckDelivery);
                }
            }))
            .on_action(cx.listener(|_, _: &ExpandComposer, _, cx| cx.emit(ComposeEvent::Expand)))
            .on_action(
                cx.listener(|_, _: &CollapseComposer, _, cx| cx.emit(ComposeEvent::Collapse)),
            )
            .on_action(cx.listener(|_, ev: &ComposeAccount, _, cx| {
                cx.emit(ComposeEvent::SetAccount(ev.account.clone()))
            }))
            .on_action(cx.listener(|this, _: &ToggleComposePreview, window, cx| {
                this.toggle_preview(window, cx)
            }))
            .on_action(cx.listener(|this, _: &AddAttachment, _, cx| {
                if !this.frozen() {
                    cx.emit(ComposeEvent::Attach)
                }
            }))
            // There is no shortcut that converts an imported rich body behind the user's
            // back: the action leads to the preview, which is where the decision is made.
            .on_action(cx.listener(|this, _: &ApproveConversion, window, cx| {
                this.open_conversion_preview(window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &PasteAsPlain, window, cx| this.paste_plain(window, cx)),
            )
            .on_action(cx.listener(|this, _: &ComposeBold, window, cx| {
                this.apply_inline(InlineFormat::Bold, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ComposeItalic, window, cx| {
                this.apply_inline(InlineFormat::Italic, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ComposeStrike, window, cx| {
                this.apply_inline(InlineFormat::Strike, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ComposeLink, window, cx| this.apply_link(window, cx)))
            .on_action(cx.listener(|this, _: &ComposeBulletedList, window, cx| {
                this.apply_block(BlockFormat::BulletList, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ComposeNumberedList, window, cx| {
                this.apply_block(BlockFormat::NumberedList, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ComposeHeading, window, cx| {
                this.apply_block(BlockFormat::Heading(2), window, cx)
            }))
            .on_action(cx.listener(|this, _: &ComposeQuote, window, cx| {
                this.apply_block(BlockFormat::Quote, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ComposeCodeBlock, window, cx| {
                this.apply_block(BlockFormat::CodeBlock, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &ComposeTable, window, cx| this.apply_table(window, cx)),
            )
            .size_full()
            .flex()
            .flex_col()
            .min_w_0()
            .min_h_0()
            .bg(background)
            .child(self.render_headers(window, cx))
            .child(self.render_toolbar(cx))
            .when_some(self.render_conversion(cx), |d, el| d.child(el))
            .when_some(self.render_pending_paste(cx), |d, el| d.child(el))
            .child(
                div()
                    .id("compose-body")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(body),
            )
            .when_some(self.render_attachments(cx), |d, el| d.child(el))
            .child(self.render_footer(cx))
    }
}

#[cfg(test)]
mod tests {
    use super::{PendingPaste, boundaries, mime_flavor};

    /// The flavour is a declared MIME type, not a word that happens to appear in the
    /// metadata: a body that quotes `text/html` in some other field stays plain text.
    #[test]
    fn a_clipboard_flavour_is_read_from_the_declared_mime_field_only() {
        assert_eq!(
            mime_flavor(Some(r#"{"mimeType":"text/html","charset":"utf-8"}"#)).as_deref(),
            Some("text/html")
        );
        assert_eq!(
            mime_flavor(Some(r#"{"contentType":"text/markdown"}"#)).as_deref(),
            Some("text/markdown")
        );
        assert_eq!(
            mime_flavor(Some(r#"{"mime_type":"text/plain"}"#)).as_deref(),
            Some("text/plain")
        );
        assert_eq!(
            mime_flavor(Some(r#"{"note":"pasted text/html from a page"}"#)),
            None
        );
        assert_eq!(
            mime_flavor(Some("text/html")),
            None,
            "metadata is JSON, not a bare type"
        );
        assert_eq!(mime_flavor(Some("not json")), None);
        assert_eq!(mime_flavor(None), None);
    }

    /// A plain paste replaces the converted insertion it is an alternative to, and only
    /// while the document is still the one that paste was made into.
    #[test]
    fn a_plain_paste_offer_describes_one_document_and_one_range() {
        let offer = PendingPaste {
            plain: "Hi there".to_owned(),
            warnings: Vec::new(),
            range: 6..15,
            source: "Hello Hi there!".to_owned(),
        };
        assert_eq!(offer.plain_range("Hello Hi there!"), Some(6..15));
        assert_eq!(
            offer.plain_range("Hello Hi there, I said."),
            None,
            "typing since the paste"
        );
        assert_eq!(
            offer.plain_range("Hello Hi there"),
            None,
            "the text was edited"
        );
    }

    /// Byte offsets from a selection can land inside a character; a recorded range has to
    /// address the same text the insertion did.
    #[test]
    fn a_recorded_range_addresses_whole_characters() {
        assert_eq!(boundaries("héllo", 2..4), 1..4);
        assert_eq!(
            boundaries("héllo", 0..99),
            0..6,
            "clamped to the text's own bytes"
        );
        assert_eq!(boundaries("", 4..9), 0..0);
    }
}
