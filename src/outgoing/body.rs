//! The composer's body: Markdown source, a preserved imported rich body, the
//! approval gate in front of lossy conversion, and rich-clipboard paste.

use std::fmt;
use std::ops::Range;

use serde::{Deserialize, Serialize};

use super::html;
use super::markdown::{self, Edit};

/// Whether the source the user edits is Markdown or plain text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BodyFormat {
    #[default]
    Markdown,
    Plain,
}

/// Which side of the source/preview toggle is showing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BodyMode {
    #[default]
    Source,
    Preview,
}

/// Styling or structure that Markdown cannot keep, reported so the user can
/// decide before the rich body is converted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConversionWarning {
    DroppedStyle { property: String },
    UnsupportedElement { element: String },
    InlineHtml,
    Image,
    UnsafeLink { url: String },
    TableWithoutHeader,
}

impl fmt::Display for ConversionWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConversionWarning::DroppedStyle { property } => write!(f, "dropped {property}"),
            ConversionWarning::UnsupportedElement { element } => {
                write!(f, "unsupported <{element}>")
            }
            ConversionWarning::InlineHtml => write!(f, "inline HTML"),
            ConversionWarning::Image => write!(f, "remote image"),
            ConversionWarning::UnsafeLink { url } => write!(f, "unsafe link {url}"),
            ConversionWarning::TableWithoutHeader => write!(f, "table without a header row"),
        }
    }
}

/// The result of converting HTML to Markdown, kept with the draft so the
/// decision about the conversion can be shown and audited.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HtmlConversion {
    pub markdown: String,
    pub text: String,
    pub warnings: Vec<ConversionWarning>,
    pub lossy: bool,
}

/// An imported rich body, preserved verbatim for as long as the draft lives.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportedOriginal {
    /// The provider's HTML, unmodified.
    pub html: String,
    /// Its text part, used as the editable fallback before approval.
    pub text: String,
    pub conversion: HtmlConversion,
}

/// One rendering of a body: the HTML and its plain-text alternative.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderedBody {
    pub html: String,
    pub text: String,
}

/// One paste: Markdown if the source offers it, else converted rich HTML, else
/// plain text. `conversion` is present when rich content was converted, so the
/// caller can offer a plain-text paste instead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasteResult {
    pub edit: Edit,
    pub conversion: Option<HtmlConversion>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Clipboard {
    pub markdown: Option<String>,
    pub html: Option<String>,
    pub text: String,
}

impl Clipboard {
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Self::default()
        }
    }

    pub fn rich(html: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            html: Some(html.into()),
            text: text.into(),
            ..Self::default()
        }
    }
}

/// What [`DraftBody::approve_conversion`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConversionApproval {
    /// Nothing was waiting for a decision.
    NothingToApprove,
    /// The converted Markdown became the source.
    Adopted,
    /// The plain-text fallback had already been edited, so those edits stand
    /// and the rich original is kept for reference only.
    KeptPlainText,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftBody {
    pub format: BodyFormat,
    /// The Markdown (or plain text) the user edits.
    pub source: String,
    pub mode: BodyMode,
    /// The rich body this draft was imported from, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original: Option<ImportedOriginal>,
    /// The user accepted (or rejected) the conversion of `original`.
    #[serde(default)]
    pub approved: bool,
}

impl DraftBody {
    pub fn markdown(source: impl Into<String>) -> Self {
        Self {
            format: BodyFormat::Markdown,
            source: source.into(),
            ..Self::default()
        }
    }

    pub fn plain(source: impl Into<String>) -> Self {
        Self {
            format: BodyFormat::Plain,
            source: source.into(),
            ..Self::default()
        }
    }

    /// A body that came in as Markdown (a reply quote, a provider draft whose
    /// source we already have): nothing to approve.
    pub fn import_markdown(text: impl Into<String>) -> Self {
        Self::markdown(text)
    }

    /// A body that came in as rich HTML from a provider or `draft_raw`. The
    /// original is preserved; a lossy conversion waits for
    /// [`DraftBody::approve_conversion`], a lossless one is adopted at once.
    pub fn import_rich(body_html: &str, text: &str) -> Self {
        let conversion = html::html_to_markdown(body_html);
        let text = if text.trim().is_empty() {
            conversion.text.clone()
        } else {
            text.to_owned()
        };
        let original = ImportedOriginal {
            html: body_html.to_owned(),
            text: text.clone(),
            conversion: conversion.clone(),
        };
        let mut body = Self {
            format: BodyFormat::Plain,
            source: text,
            mode: BodyMode::default(),
            original: Some(original),
            approved: false,
        };
        if !conversion.lossy {
            body.approve_conversion();
        }
        body
    }

    /// True while a lossy conversion of the imported body is still undecided:
    /// the body renders from the preserved HTML, not from Markdown.
    pub fn needs_conversion(&self) -> bool {
        self.original.is_some() && !self.approved
    }

    /// Accept the Markdown conversion of the imported body. Edits already made
    /// to the plain-text fallback win over the conversion, so approval never
    /// silently discards typing.
    pub fn approve_conversion(&mut self) -> ConversionApproval {
        let Some(original) = self.original.clone() else {
            return ConversionApproval::NothingToApprove;
        };
        if self.approved {
            return ConversionApproval::NothingToApprove;
        }
        self.approved = true;
        // The editable fallback is the imported text part, which need not match
        // the text the DOM conversion produced: only typing makes it the user's.
        if self.source.trim() == original.text.trim() {
            self.source = original.conversion.markdown;
            self.format = BodyFormat::Markdown;
            ConversionApproval::Adopted
        } else {
            ConversionApproval::KeptPlainText
        }
    }

    /// Keep the plain text as it is and stop asking; the rich original stays
    /// preserved on the draft.
    pub fn reject_conversion(&mut self) -> bool {
        if !self.needs_conversion() {
            return false;
        }
        self.approved = true;
        true
    }

    /// What the message carries: the HTML part and its plain-text alternative.
    ///
    /// While a lossy conversion is still undecided this is the imported body
    /// exactly as it arrived, byte for byte, because that is what the user
    /// read; converting it is their call. Everything else is rendered from the
    /// Markdown source, so preview and send agree.
    pub fn render(&self) -> RenderedBody {
        if self.needs_conversion() {
            let original = self.original.as_ref().expect("checked by needs_conversion");
            return RenderedBody {
                html: original.html.clone(),
                text: if original.text.trim().is_empty() {
                    markdown::markdown_to_text(&self.source)
                } else {
                    original.text.clone()
                },
            };
        }
        self.rendered_source()
    }

    /// What the preview shows. Same as [`DraftBody::render`] once the body is
    /// Markdown; before that it is the imported body with remote images and
    /// their loaders replaced by their alt text, so opening the composer never
    /// fetches anything. Sending is unaffected.
    pub fn preview(&self) -> RenderedBody {
        if self.needs_conversion() {
            let original = self.original.as_ref().expect("checked by needs_conversion");
            let blocked = crate::reading::safe_html(&original.html);
            return RenderedBody {
                html: blocked.html,
                text: self.render().text,
            };
        }
        self.rendered_source()
    }

    fn rendered_source(&self) -> RenderedBody {
        if self.format == BodyFormat::Plain {
            // Plain text is not Markdown: it is sent as itself, escaped for
            // HTML, with the line breaks it had.
            return RenderedBody {
                html: markdown::plain_text_to_html(&self.source),
                text: self.source.clone(),
            };
        }
        RenderedBody {
            html: markdown::render_markdown(&self.source),
            text: markdown::markdown_to_text(&self.source),
        }
    }

    pub fn set_source(&mut self, source: impl Into<String>) {
        self.source = source.into();
    }

    pub fn set_mode(&mut self, mode: BodyMode) {
        self.mode = mode;
    }

    /// Paste at `sel`. Markdown on the clipboard is inserted as-is; rich HTML is
    /// converted (and the conversion reported, so a plain paste can be offered
    /// instead); plain text keeps its paragraph breaks and gets hard breaks for
    /// single newlines.
    pub fn paste(&self, sel: Range<usize>, clip: &Clipboard) -> PasteResult {
        if let Some(md) = clip.markdown.as_ref().filter(|m| !m.trim().is_empty()) {
            return PasteResult {
                edit: insert(&self.source, sel, md),
                conversion: None,
            };
        }
        if let Some(body_html) = clip.html.as_ref().filter(|h| !h.trim().is_empty()) {
            let conversion = html::html_to_markdown(body_html);
            let text = if clip.text.trim().is_empty() {
                conversion.text.clone()
            } else {
                clip.text.clone()
            };
            let markdown_source = if conversion.markdown.trim().is_empty() {
                text
            } else {
                conversion.markdown.clone()
            };
            return PasteResult {
                edit: insert(&self.source, sel, &markdown_source),
                conversion: Some(conversion),
            };
        }
        let plain = markdown::plain_text_to_markdown(&clip.text);
        PasteResult {
            edit: insert(&self.source, sel, &plain),
            conversion: None,
        }
    }
}

fn insert(src: &str, sel: Range<usize>, text: &str) -> Edit {
    let start = sel.start.min(src.len());
    let end = sel.end.clamp(start, src.len());
    let (start, end) = (floor(src, start), floor(src, end));
    let mut out = String::with_capacity(src.len() + text.len());
    out.push_str(&src[..start]);
    out.push_str(text);
    out.push_str(&src[end..]);
    let caret = start + text.len();
    Edit {
        text: out,
        selection: caret..caret,
    }
}

fn floor(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_and_send_agree_once_the_body_is_markdown() {
        let body = DraftBody::markdown("**bold** and [a](https://x.test)");
        let rendered = body.render();
        assert!(
            rendered.html.contains("<strong>bold</strong>"),
            "{}",
            rendered.html
        );
        assert!(
            rendered.html.contains("<a href=\"https://x.test\">a</a>"),
            "{}",
            rendered.html
        );
        assert!(
            rendered.text.contains("bold and a (https://x.test)"),
            "{}",
            rendered.text
        );
    }

    #[test]
    fn a_sends_the_imported_body_unchanged_while_the_preview_stays_safe() {
        let rich = "<p style=\"color:red\">Hello <font color=\"blue\">there</font></p>\
                     <img src=\"https://track.test/pixel.gif\" alt=\"pixel\"></p>";
        let body = DraftBody::import_rich(rich, "Hello there");
        assert!(body.needs_conversion(), "a lossy import waits for the user");

        // Sending an undecided body must not rewrite what the user read.
        assert_eq!(body.render().html, rich);
        assert_eq!(body.render().text, "Hello there");

        let preview = body.preview();
        assert!(preview.html.contains("Hello"), "{}", preview.html);
        assert!(
            !preview.html.contains("track.test"),
            "the preview loads nothing: {}",
            preview.html
        );
        assert!(
            preview.html.contains("pixel"),
            "the alt text stands in: {}",
            preview.html
        );

        assert_eq!(body.original.as_ref().expect("original").html, rich);
        assert!(body.original.as_ref().expect("original").conversion.lossy);
    }

    #[test]
    fn approval_switches_to_the_converted_markdown() {
        let mut body = DraftBody::import_rich(
            "<p>Hello <b>there</b></p><p style=\"color:red\">styled</p>",
            "Hello there\n\nstyled",
        );
        assert!(body.needs_conversion());
        assert_eq!(body.approve_conversion(), ConversionApproval::Adopted);
        assert!(!body.needs_conversion());
        assert_eq!(body.format, BodyFormat::Markdown);
        assert!(body.source.contains("**there**"), "{}", body.source);
        let rendered = body.render();
        assert!(
            rendered.html.contains("<strong>there</strong>"),
            "{}",
            rendered.html
        );
        // Approval switches rendering over; the imported HTML stays preserved.
        assert_eq!(body.preview().html, rendered.html);
        assert!(body.original.is_some());
    }

    #[test]
    fn an_untouched_import_adopts_the_conversion_even_with_its_own_plain_part() {
        // The provider's plain alternative spells the link out; the DOM
        // conversion writes Markdown. Neither was typed, so approval adopts.
        let mut body = DraftBody::import_rich(
            "<p>See <a href=\"https://x.test/docs\">the docs</a></p><p style=\"color:red\">x</p>",
            "See https://x.test/docs\n\nx",
        );
        assert!(body.needs_conversion());
        assert_eq!(body.approve_conversion(), ConversionApproval::Adopted);
        assert_eq!(body.format, BodyFormat::Markdown);
        assert_eq!(body.source, "See [the docs](https://x.test/docs)\n\nx");
    }

    #[test]
    fn approval_never_discards_edits_made_to_the_plain_fallback() {
        let mut body =
            DraftBody::import_rich("<p>Hello <b>there</b></p><p style=\"color:red\">x</p>", "");
        assert!(body.needs_conversion(), "a lossy import waits for the user");
        body.set_source("Hello there, I will be there");
        assert_eq!(body.approve_conversion(), ConversionApproval::KeptPlainText);
        assert_eq!(body.source, "Hello there, I will be there");
        assert!(!body.needs_conversion());
    }

    #[test]
    fn lossless_import_is_adopted_without_a_question() {
        let body = DraftBody::import_rich("<p>Hello <b>there</b></p>", "Hello there");
        assert!(!body.needs_conversion());
        assert_eq!(body.format, BodyFormat::Markdown);
        assert!(body.source.contains("**there**"), "{}", body.source);
    }

    #[test]
    fn rejecting_the_conversion_keeps_plain_text() {
        let mut body = DraftBody::import_rich("<p>Hello <b>there</b></p><p style=\"x\">s</p>", "");
        assert!(body.reject_conversion());
        assert!(!body.needs_conversion());
        assert_eq!(body.format, BodyFormat::Plain);
        assert!(!body.reject_conversion());
    }

    #[test]
    fn rich_paste_converts_and_reports_the_conversion() {
        let body = DraftBody::markdown("");
        let clip = Clipboard::rich("<p>Hi <b>there</b></p><ul><li>a</li></ul>", "Hi there\n\na");
        let pasted = body.paste(0..0, &clip);
        assert_eq!(pasted.edit.text, "Hi **there**\n\n- a");
        let conversion = pasted.conversion.expect("conversion reported");
        assert!(!conversion.lossy, "{:?}", conversion.warnings);
    }

    #[test]
    fn rich_paste_keeps_plain_text_available() {
        let body = DraftBody::markdown("");
        let clip = Clipboard::rich("<p>Hi <b>there</b></p>", "Hi there");
        let plain = body.paste(0..0, &Clipboard::plain(clip.text.clone()));
        assert_eq!(plain.edit.text, "Hi there");
        assert!(plain.conversion.is_none());
    }

    #[test]
    fn markdown_clipboard_wins_over_html() {
        let body = DraftBody::markdown("");
        let mut clip = Clipboard::rich("<p>ignored</p>", "ignored");
        clip.markdown = Some("# from markdown".to_owned());
        let pasted = body.paste(0..0, &clip);
        assert_eq!(pasted.edit.text, "# from markdown");
        assert!(pasted.conversion.is_none());
    }

    #[test]
    fn plain_paste_keeps_paragraph_breaks_and_hard_breaks() {
        let body = DraftBody::markdown("");
        let pasted = body.paste(0..0, &Clipboard::plain("one\ntwo\n\nthree"));
        assert_eq!(pasted.edit.text, "one\\\ntwo\n\nthree");
    }

    #[test]
    fn paste_replaces_the_selection() {
        let body = DraftBody::markdown("keep this");
        let pasted = body.paste(0..4, &Clipboard::plain("that"));
        assert_eq!(pasted.edit.text, "that this");
    }

    #[test]
    fn paste_of_unicode_selection_keeps_boundaries() {
        let body = DraftBody::markdown("héllo");
        let pasted = body.paste(1..3, &Clipboard::plain("x"));
        assert_eq!(pasted.edit.text, "hxllo");
    }

    #[test]
    fn a_plain_body_is_not_parsed_as_markdown() {
        let body = DraftBody::plain("**literal**\nsecond line");
        let rendered = body.render();
        assert_eq!(rendered.text, "**literal**\nsecond line");
        assert!(
            rendered.html.contains("**literal**"),
            "the asterisks are kept: {}",
            rendered.html
        );
        assert!(
            rendered.html.contains("<br />"),
            "the line break is kept: {}",
            rendered.html
        );
        assert_eq!(body.preview().html, rendered.html);
    }

    #[test]
    fn a_plain_paste_keeps_html_and_asterisks_literal() {
        let body = DraftBody::markdown("");
        let pasted = body.paste(0..0, &Clipboard::plain("<div>Important</div> **bold?**"));
        let rendered = DraftBody {
            source: pasted.edit.text,
            ..body.clone()
        }
        .render();
        assert!(
            rendered.html.contains("&lt;div&gt;Important&lt;/div&gt;"),
            "{}",
            rendered.html
        );
        assert!(rendered.html.contains("**bold?**"), "{}", rendered.html);
    }
}
