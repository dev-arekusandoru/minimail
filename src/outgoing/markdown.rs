//! Markdown rendering and pure source edits for the composer.
//!
//! Block and inline structure comes from pulldown-cmark. This module owns the
//! two sinks the product needs: HTML (used unchanged for the preview *and* for
//! the HTML part of the sent message) and plain text (the MIME alternative).
//! Raw HTML written in the source is dropped rather than passed through, so a
//! pasted or imported body can never smuggle markup into a sent message.

use std::ops::Range;

use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

/// Tables, strikethrough and task lists are the everyday extras. Footnotes,
/// definition lists, metadata blocks, math and smart punctuation stay off.
pub fn parser_options() -> Options {
    Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS
}

/// The HTML the preview shows and the HTML part of the sent message. Both
/// callers get this from here, so they cannot drift apart.
pub fn render_markdown(src: &str) -> String {
    render(src, false)
}

/// The plain-text alternative of [`render_markdown`].
pub fn markdown_to_text(src: &str) -> String {
    render(src, true)
}

fn render(src: &str, text: bool) -> String {
    let mut r = Renderer {
        out: String::with_capacity(src.len() * 3 / 2),
        text,
        lists: Vec::new(),
        quote: 0,
        line_start: true,
        aligns: Vec::new(),
        in_head: false,
        cell: 0,
        links: Vec::new(),
        in_image: false,
        image_alt: String::new(),
        image_title: String::new(),
    };
    for ev in Parser::new_ext(src, parser_options()) {
        r.event(ev);
    }
    if r.text {
        r.blank_line();
    } else if !r.out.is_empty() {
        r.out.push('\n');
    }
    r.out
}

struct Renderer {
    out: String,
    /// True renders the plain-text alternative instead of HTML.
    text: bool,
    /// One entry per open list: `true` for ordered, plus the next item number.
    lists: Vec<(bool, usize)>,
    quote: usize,
    /// True when nothing but a line prefix has been written on this line.
    line_start: bool,
    aligns: Vec<Alignment>,
    /// True inside `<thead>`, where cells are headings.
    in_head: bool,
    /// Index of the next cell in the current table row.
    cell: usize,
    /// Destinations of open links, for the plain-text alternative.
    links: Vec<String>,
    /// True between an image's start and end tag: the alt text is a child
    /// event in pulldown-cmark, and it belongs in the attribute.
    in_image: bool,
    /// The alt text collected for the image being written.
    image_alt: String,
    /// The title of the image being written.
    image_title: String,
}

impl Renderer {
    fn indent(&self) -> String {
        let mut s = "> ".repeat(self.quote);
        for _ in self.lists.iter().skip(1) {
            s.push_str("  ");
        }
        s
    }

    /// Plain text: write `s`, prefixing the start of every line.
    fn write(&mut self, s: &str) {
        if !self.text {
            self.out.push_str(s);
            return;
        }
        for c in s.chars() {
            if self.line_start {
                let prefix = self.indent();
                if !prefix.is_empty() {
                    self.out.push_str(&prefix);
                }
                self.line_start = false;
            }
            self.out.push(c);
            if c == '\n' {
                self.line_start = true;
            }
        }
    }

    fn newline(&mut self) {
        if self.text {
            self.write("\n");
        }
    }

    /// Plain text: end the current block with exactly one blank line.
    fn blank_line(&mut self) {
        if !self.text {
            return;
        }
        let trimmed = self.out.trim_end();
        self.out.truncate(trimmed.len());
        if !self.out.is_empty() {
            self.out.push_str("\n\n");
        }
        self.line_start = true;
    }

    /// `<tag attrs>`, or nothing at all in the plain-text alternative.
    fn open_tag(&mut self, tag: &str, attrs: &str) {
        if self.text {
            return;
        }
        self.out.push('<');
        self.out.push_str(tag);
        if !attrs.is_empty() {
            self.out.push(' ');
            self.out.push_str(attrs);
        }
        self.out.push('>');
    }

    fn open(&mut self, tag: &str) {
        if !self.text {
            self.out.push('<');
            self.out.push_str(tag);
            self.out.push('>');
        }
    }

    fn close(&mut self, tag: &str) {
        if !self.text {
            self.out.push_str("</");
            self.out.push_str(tag);
            self.out.push('>');
        }
    }

    fn attrs(&mut self, attrs: &str) {
        if !self.text {
            self.out.push(' ');
            self.out.push_str(attrs);
        }
    }

    fn event(&mut self, ev: Event<'_>) {
        match ev {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => {
                let s: &str = &t;
                if self.in_image {
                    self.image_alt.push_str(s);
                } else if self.text {
                    self.write(s);
                } else {
                    self.out.push_str(&escape_html(s));
                }
            }
            Event::Code(t) => {
                let s: &str = &t;
                if self.in_image {
                    self.image_alt.push_str(s);
                } else if self.text {
                    self.write(s);
                } else {
                    self.out.push_str("<code>");
                    self.out.push_str(&escape_html(s));
                    self.out.push_str("</code>");
                }
            }
            Event::SoftBreak => {
                if self.in_image {
                    // An attribute cannot carry the line break; the words it
                    // separated are still the words the alt text is.
                    self.image_alt.push(' ');
                } else if self.text {
                    self.write(" ");
                } else {
                    self.out.push('\n');
                }
            }
            Event::HardBreak => {
                if self.in_image {
                    self.image_alt.push(' ');
                } else if self.text {
                    self.write("\n");
                } else {
                    self.out.push_str("<br />\n");
                }
            }
            Event::Rule => {
                if self.text {
                    self.write("---");
                    self.blank_line();
                } else {
                    self.out.push_str("<hr />\n");
                }
            }
            // HTML written in the source is not passed through: the composer
            // only ever emits markup built here.
            Event::Html(_) | Event::InlineHtml(_) => {}
            // A task item's state is part of what the item says. Dropping the
            // marker would make the HTML body disagree with the source and
            // with the plain-text alternative, which keeps it.
            Event::TaskListMarker(done) => {
                if self.text {
                    self.write(if done { "[x] " } else { "[ ] " });
                } else if !self.in_image {
                    self.out.push_str(if done {
                        "<input disabled=\"\" type=\"checkbox\" checked=\"\" />"
                    } else {
                        "<input disabled=\"\" type=\"checkbox\" />"
                    });
                }
            }
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        // Markup inside an alt text is text, not markup in the output.
        if self.in_image && !matches!(tag, Tag::Image { .. }) {
            return;
        }
        match tag {
            Tag::Paragraph => self.open("p"),
            Tag::Heading { level, .. } => {
                let n = heading_level(level);
                self.open(&format!("h{n}"));
            }
            Tag::BlockQuote(_) => {
                self.quote += 1;
                self.open("blockquote");
            }
            Tag::CodeBlock(kind) => {
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => {
                        info.split_whitespace().next().unwrap_or("").to_owned()
                    }
                    CodeBlockKind::Indented => String::new(),
                };
                if self.text {
                    self.write("```\n");
                } else {
                    self.out.push_str("<pre><code");
                    if !lang.is_empty() && is_plain_language(&lang) {
                        self.out
                            .push_str(&format!(" class=\"language-{}\"", escape_html(&lang)));
                    }
                    self.out.push('>');
                }
            }
            Tag::List(start) => {
                let ordered = start.is_some();
                let first = start.unwrap_or(1) as usize;
                self.lists.push((ordered, first));
                if ordered {
                    let attrs = if first == 1 {
                        String::new()
                    } else {
                        format!("start=\"{first}\"")
                    };
                    self.open_tag("ol", &attrs);
                } else {
                    self.open("ul");
                }
            }
            Tag::Item => {
                let depth = self.lists.len().saturating_sub(1) * 2;
                let marker = match self.lists.last_mut() {
                    Some((true, n)) => {
                        let marker = format!("{n}. ");
                        *n += 1;
                        marker
                    }
                    _ => "- ".to_owned(),
                };
                if self.text {
                    if !self.out.is_empty() && !self.out.ends_with('\n') {
                        self.out.push('\n');
                    }
                    self.write(&" ".repeat(depth));
                    self.write(&marker);
                } else {
                    self.open("li");
                }
            }
            Tag::Emphasis => self.open("em"),
            Tag::Strong => self.open("strong"),
            Tag::Strikethrough => self.open("del"),
            Tag::Superscript => self.open("sup"),
            Tag::Subscript => self.open("sub"),
            Tag::Link {
                dest_url, title, ..
            } => {
                if self.text {
                    let url: &str = &dest_url;
                    self.links.push(url.trim().to_owned());
                    return;
                }
                let url: &str = &dest_url;
                if safe_url(url) {
                    self.out.push_str("<a href=\"");
                    self.out.push_str(&escape_html(url.trim()));
                    self.out.push('"');
                    let title: &str = &title;
                    if !title.is_empty() {
                        self.attrs(&format!(" title=\"{}\"", escape_html(title)));
                    }
                    self.out.push('>');
                } else {
                    // Unsafe scheme: keep the words, drop the destination.
                    self.out.push_str("<span>");
                }
            }
            Tag::Image {
                dest_url, title, ..
            } => {
                let url: &str = &dest_url;
                let title: &str = &title;
                if self.text {
                    // The alternative keeps the words the image stands for.
                    return;
                }
                if !safe_url(url) {
                    // An unusable destination is dropped on send, so the
                    // alt text stands in for the image here too.
                    return;
                }
                self.in_image = true;
                self.image_alt.clear();
                self.image_title = title.to_owned();
                self.out.push_str("<img src=\"");
                self.out.push_str(&escape_html(url.trim()));
                self.out.push('"');
            }
            Tag::Table(aligns) => {
                self.aligns = aligns;
                self.cell = 0;
                self.open("table");
            }
            Tag::TableHead => {
                self.in_head = true;
                self.open("thead");
            }
            Tag::TableRow => {
                self.cell = 0;
                self.open("tr");
            }
            Tag::TableCell => {
                let tag = if self.in_head { "th" } else { "td" };
                if !self.text {
                    let align = match self.aligns.get(self.cell) {
                        Some(Alignment::Left) => "align=\"left\"",
                        Some(Alignment::Center) => "align=\"center\"",
                        Some(Alignment::Right) => "align=\"right\"",
                        _ => "",
                    };
                    self.open_tag(tag, align);
                }
                self.cell += 1;
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        // The image's closing attribute comes from the alt text collected
        // between its tags, so it is handled before anything else.
        if let TagEnd::Image = tag {
            self.close_image();
            return;
        }
        if self.in_image {
            // Markup inside an alt text is not markup in the output.
            return;
        }
        match tag {
            TagEnd::Paragraph => {
                self.close("p");
                self.blank_line();
            }
            TagEnd::Heading(level) => {
                self.close(&format!("h{}", heading_level(level)));
                self.blank_line();
            }
            TagEnd::BlockQuote(_) => {
                self.quote = self.quote.saturating_sub(1);
                self.close("blockquote");
                self.blank_line();
            }
            TagEnd::CodeBlock => {
                if self.text {
                    self.write("\n```");
                    self.blank_line();
                } else {
                    self.out.push_str("</code></pre>\n");
                }
            }
            TagEnd::List(_) => {
                let (ordered, _) = self.lists.pop().unwrap_or((false, 1));
                if !self.text {
                    self.close(if ordered { "ol" } else { "ul" });
                }
                if self.text {
                    self.blank_line();
                }
            }
            TagEnd::Item => {
                if self.text {
                    self.newline();
                } else {
                    self.close("li");
                }
            }
            TagEnd::Emphasis => self.close("em"),
            TagEnd::Strong => self.close("strong"),
            TagEnd::Strikethrough => self.close("del"),
            TagEnd::Superscript => self.close("sup"),
            TagEnd::Subscript => self.close("sub"),
            TagEnd::Link => {
                if self.text {
                    let url = self.links.pop().unwrap_or_default();
                    if !url.is_empty() {
                        self.write(&format!(" ({url})"));
                    }
                } else {
                    self.close("a");
                }
            }
            TagEnd::Table => {
                self.aligns.clear();
                self.in_head = false;
                self.close("table");
                self.blank_line();
            }
            TagEnd::TableHead => {
                self.in_head = false;
                self.close("thead");
            }
            TagEnd::TableRow => {
                if self.text {
                    self.newline();
                } else {
                    self.close("tr");
                }
            }
            TagEnd::TableCell => {
                if self.text {
                    self.write("\t");
                } else {
                    self.close(if self.in_head { "th" } else { "td" });
                }
            }
            _ => {}
        }
    }

    /// An image's alt text is a child event, so the tag is only finished when
    /// its end arrives: without this the words leak into the body and the
    /// attribute is empty, which is exactly what a preview must not show.
    fn close_image(&mut self) {
        if !self.in_image {
            return;
        }
        self.in_image = false;
        self.out.push_str(" alt=\"");
        self.out.push_str(&escape_html(&self.image_alt));
        if !self.image_title.is_empty() {
            self.out.push_str("\" title=\"");
            self.out.push_str(&escape_html(&self.image_title));
        }
        self.out.push_str("\" />");
    }
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn is_plain_language(lang: &str) -> bool {
    lang.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '#' || c == '-')
}

/// Schemes a composed message may link to. Anything else (notably `javascript:`
/// and `data:` documents) is dropped while its text is kept.
pub fn safe_url(url: &str) -> bool {
    let u = url.trim();
    let lower = u.to_ascii_lowercase();
    if lower.starts_with('#') || lower.starts_with('/') {
        return true;
    }
    [
        "http://", "https://", "mailto:", "tel:", "cid:", "ftp://", "ftps://",
    ]
    .iter()
    .any(|p| lower.starts_with(p))
}

pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Plain text as Markdown: blank lines stay paragraph breaks, and a single
/// newline becomes a hard break so pasted text keeps the line breaks it had.
pub fn plain_text_to_markdown(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::with_capacity(normalized.len() + 8);
    let mut paragraph_break = false;
    let mut written = false;
    for line in normalized.split('\n') {
        if line.trim().is_empty() {
            paragraph_break = written;
            continue;
        }
        let literal = escape_markdown_literal(line);
        if !written {
            out.push_str(&literal);
            written = true;
        } else if paragraph_break {
            out.push_str("\n\n");
            out.push_str(&literal);
            paragraph_break = false;
        } else {
            out.push_str("\\\n");
            out.push_str(&literal);
        }
    }
    out
}

// --------------------------------------------------------------------- edits

/// A new source string plus the caret (or selection) it should leave behind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub text: String,
    pub selection: Range<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InlineFormat {
    Bold,
    Italic,
    Code,
    Strike,
}

impl InlineFormat {
    pub fn markers(self) -> &'static str {
        match self {
            InlineFormat::Bold => "**",
            InlineFormat::Italic => "*",
            InlineFormat::Code => "`",
            InlineFormat::Strike => "~~",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockFormat {
    /// `Heading(0)` removes any heading level.
    Heading(u8),
    BulletList,
    NumberedList,
    Quote,
    CodeBlock,
}

/// Wrap or unwrap the selection. Toggling off is exact: a selection that is
/// already exactly `**text**` becomes `text`. With no selection the markers go
/// in and the caret lands between them; otherwise the words stay selected, so
/// a second toggle removes the formatting again.
pub fn toggle_inline(src: &str, sel: Range<usize>, format: InlineFormat) -> Edit {
    let (start, end) = clamp(src, sel);
    let marker = format.markers();
    let selected = &src[start..end];
    // Markers just outside the selection mean the formatting is already there:
    // toggling again takes it off, and the selection stays on the same words.
    // `src[..start]` and `src[end..]` are already on character boundaries, so
    // look at the markers there rather than slicing bytes out of the middle of a
    // character.
    let wrapped_before = src[..start].ends_with(marker);
    let wrapped_after = src[end..].starts_with(marker);
    if wrapped_before && wrapped_after {
        let from = start - marker.len();
        let mut text = String::with_capacity(src.len());
        text.push_str(&src[..from]);
        text.push_str(&src[start..end]);
        text.push_str(&src[end + marker.len()..]);
        return Edit {
            selection: from..from + selected.len(),
            text,
        };
    }
    // A selection that came in already wrapped (a paste, another tool) is
    // unwrapped in place.
    let trimmed = selected.trim();
    let trimmed_start = start + (selected.len() - selected.trim_start().len());
    if trimmed.len() >= 2 * marker.len() && trimmed.starts_with(marker) && trimmed.ends_with(marker)
    {
        let inner = &trimmed[marker.len()..trimmed.len() - marker.len()];
        let mut text = String::with_capacity(src.len());
        text.push_str(&src[..trimmed_start]);
        text.push_str(inner);
        text.push_str(&src[trimmed_start + trimmed.len()..]);
        return Edit {
            selection: trimmed_start..trimmed_start + inner.len(),
            text,
        };
    }
    let mut text = String::with_capacity(src.len() + 2 * marker.len());
    text.push_str(&src[..start]);
    text.push_str(marker);
    text.push_str(selected);
    text.push_str(marker);
    text.push_str(&src[end..]);
    let inner = start + marker.len();
    Edit {
        selection: inner..inner + selected.len(),
        text,
    }
}

/// Add or remove a line prefix on every line the selection touches. Toggling
/// the format a block already has removes it; switching formats replaces the
/// old prefix instead of stacking on it.
pub fn toggle_block(src: &str, sel: Range<usize>, format: BlockFormat) -> Edit {
    let (start, end) = clamp(src, sel);
    let line_start = src[..start].rfind('\n').map_or(0, |i| i + 1);
    let line_end = src[end..].find('\n').map_or(src.len(), |i| end + i);
    let block = &src[line_start..line_end];
    let lines: Vec<&str> = block.split('\n').collect();

    if format == BlockFormat::CodeBlock {
        // A trailing newline leaves an empty last line, which is not part of the
        // block the user is toggling.
        let mut block_lines = lines.clone();
        while block_lines.last() == Some(&"") {
            block_lines.pop();
        }
        return toggle_code_block(src, &block_lines, line_start, line_end, start, end);
    }

    let prefixed: fn(&str) -> bool = match format {
        BlockFormat::Heading(_) => is_heading,
        BlockFormat::BulletList => is_bullet,
        BlockFormat::NumberedList => is_numbered,
        BlockFormat::Quote => is_quote,
        BlockFormat::CodeBlock => is_fence,
    };
    let all_have = !lines.is_empty() && lines.iter().all(|l| prefixed(l));
    let mut out = String::with_capacity(block.len() + 16);
    // Where each original line ends up, so the selection follows the words.
    let mut placed: Vec<(usize, usize, usize)> = Vec::with_capacity(lines.len());
    let mut offset = 0usize;
    for (i, line) in lines.iter().enumerate() {
        let added = if all_have {
            String::new()
        } else {
            prefix_for(format).unwrap_or_default()
        };
        let bare = if all_have {
            strip_prefix(line)
        } else {
            strip_prefix(&strip_prefix(line))
        };
        placed.push((offset, out.len(), added.len()));
        out.push_str(&added);
        out.push_str(&bare);
        offset += line.len() + 1;
        if i + 1 < lines.len() {
            out.push('\n');
        }
    }
    let map = |pos: usize| -> usize {
        let within_block = pos.saturating_sub(line_start);
        let mut offset = 0usize;
        for (i, line) in lines.iter().enumerate() {
            if within_block <= offset + line.len() {
                let (_, new_start, added) = placed[i];
                return line_start + new_start + added + (within_block - offset);
            }
            offset += line.len() + 1;
        }
        line_start + out.len()
    };
    let text = format!("{}{}{}", &src[..line_start], out, &src[line_end..]);
    Edit {
        selection: map(start).min(text.len())..map(end).min(text.len()),
        text,
    }
}

/// `[text](url)` around the selection, or an empty link with the caret inside
/// the brackets when there is no selection. `text` overrides the selected words.
pub fn insert_link(src: &str, sel: Range<usize>, url: &str, text: Option<&str>) -> Edit {
    let (start, end) = clamp(src, sel);
    let url = url.trim();
    let selected = &src[start..end];
    let label = text.unwrap_or(selected);
    let mut new = String::with_capacity(src.len() + label.len() + url.len() + 4);
    new.push_str(&src[..start]);
    new.push('[');
    new.push_str(label);
    new.push_str("](");
    new.push_str(url);
    new.push(')');
    new.push_str(&src[end..]);
    let label_start = start + 1;
    Edit {
        selection: label_start..label_start + label.len(),
        text: new,
    }
}

/// A Markdown table with `cols` columns and `rows` empty body rows, inserted on
/// its own lines with the caret in the first body cell.
pub fn insert_table(src: &str, sel: Range<usize>, rows: usize, cols: usize) -> Edit {
    let (start, _) = clamp(src, sel);
    let cols = cols.clamp(1, 12);
    let rows = rows.clamp(1, 40);
    let line_start = src[..start].rfind('\n').map_or(0, |i| i + 1);
    let mut block = String::new();
    let header: Vec<String> = (1..=cols).map(|c| format!("Column {c}")).collect();
    block.push_str(&markdown_row(&header));
    block.push('|');
    for _ in 0..cols {
        block.push_str(" --- |");
    }
    block.push('\n');
    let body_start = line_start + block.len();
    for _ in 0..rows {
        block.push('|');
        for _ in 0..cols {
            block.push_str("  |");
        }
        block.push('\n');
    }
    let prefix = if line_start > 0 && !src[..line_start].ends_with("\n\n") {
        "\n"
    } else {
        ""
    };
    let suffix = if src[line_start..].starts_with('\n') {
        ""
    } else {
        "\n"
    };
    let block = format!("{prefix}{block}{suffix}");
    let mut text = String::with_capacity(src.len() + block.len());
    text.push_str(&src[..line_start]);
    text.push_str(&block);
    text.push_str(&src[line_start..]);
    let caret = body_start + prefix.len() + 2;
    Edit {
        selection: caret..caret,
        text,
    }
}

/// Escape what Markdown would otherwise read as syntax, so pasted prose stays
/// prose: a literal `<div>` must not become a dropped raw-HTML block, and
/// `**stars**` must not turn bold.
fn escape_markdown_literal(line: &str) -> String {
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    // A line like "1. thing" would start a list.
    let dotted_list = digits > 0 && line[digits..].starts_with(". ");
    let mut out = String::with_capacity(line.len() + 8);
    let mut seen = 0usize;
    for c in line.chars() {
        // Inline syntax is escaped wherever it appears; the block markers only
        // matter at the start of a line, so ordinary words stay readable.
        let block_start = out.is_empty();
        match c {
            '\\' | '`' | '*' | '_' | '[' | ']' | '~' | '<' => {
                out.push('\\');
                out.push(c);
            }
            '#' | '>' | '-' | '+' | '=' | '|' if block_start => {
                out.push('\\');
                out.push(c);
            }
            '.' if dotted_list && seen == digits && block_start => out.push_str("\\."),
            _ => out.push(c),
        }
        if c.is_ascii_digit() {
            seen += 1;
        }
    }
    out
}

/// Plain text as HTML: the words, escaped, with the line breaks the text had.
pub fn plain_text_to_html(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::from("<p>");
    for (i, line) in normalized.split('\n').enumerate() {
        if i > 0 {
            out.push_str("<br />\n");
        }
        out.push_str(&escape_html(line));
    }
    out.push_str("</p>");
    out
}

fn markdown_row(cells: &[String]) -> String {
    let mut row = String::from("|");
    for c in cells {
        row.push(' ');
        row.push_str(c);
        row.push_str(" |");
    }
    row.push('\n');
    row
}

/// A code block needs one matched fence pair around the whole selection, not a
/// fence in front of every line, so it is handled on its own.
fn toggle_code_block(
    src: &str,
    lines: &[&str],
    line_start: usize,
    line_end: usize,
    start: usize,
    end: usize,
) -> Edit {
    let opening = lines.first().map(|l| fence_of(l)).unwrap_or(None);
    let closing = lines.last().map(|l| fence_of(l)).unwrap_or(None);
    let mut text = String::with_capacity(src.len() + 8);
    match (opening, closing) {
        // Already a block: drop both fences and keep the code.
        (Some(open), Some(close)) if open == close && lines.len() >= 2 => {
            let inner = lines[1..lines.len() - 1].join("\n");
            text.push_str(&src[..line_start]);
            text.push_str(&inner);
            text.push_str(&src[line_end..]);
            let from = line_start;
            Edit {
                selection: from..from + inner.len(),
                text,
            }
        }
        // An unpaired opening fence is closed by what follows: finish the block.
        (Some(open), None) if !open.is_empty() => {
            text.push_str(&src[..line_start]);
            text.push_str(&lines.join("\n"));
            if !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(&open);
            text.push_str(&src[line_end..]);
            Edit {
                selection: start.min(text.len())..end.min(text.len()),
                text,
            }
        }
        _ => {
            // Fence long enough that the code cannot close it.
            let fence = "`".repeat(
                lines
                    .iter()
                    .filter_map(|l| fence_of(l))
                    .map(|f| f.len())
                    .max()
                    .unwrap_or(2)
                    .max(2)
                    + 1,
            );
            let inner = lines.join("\n");
            text.push_str(&src[..line_start]);
            text.push_str(&fence);
            text.push('\n');
            text.push_str(&inner);
            if !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(&fence);
            text.push('\n');
            text.push_str(&src[line_end..]);
            let from = line_start + fence.len() + 1;
            Edit {
                selection: from..(from + inner.len()).min(text.len()),
                text,
            }
        }
    }
}

/// The fence a line opens or closes with, if it is only a fence.
fn fence_of(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.len() >= 3 && trimmed.chars().all(|c| c == '`') {
        Some(trimmed.to_owned())
    } else {
        None
    }
}

fn prefix_for(format: BlockFormat) -> Option<String> {
    match format {
        BlockFormat::Heading(level) if (1..=6).contains(&level) => {
            Some(format!("{} ", "#".repeat(level as usize)))
        }
        BlockFormat::Heading(_) => None,
        BlockFormat::BulletList => Some("- ".to_owned()),
        BlockFormat::NumberedList => Some("1. ".to_owned()),
        BlockFormat::Quote => Some("> ".to_owned()),
        BlockFormat::CodeBlock => Some("```\n".to_owned()),
    }
}

fn strip_prefix(line: &str) -> String {
    if let Some(rest) = line.strip_prefix("```") {
        return rest.to_owned();
    }
    if line.trim_end().ends_with("```") {
        return line
            .trim_end()
            .trim_end_matches("```")
            .trim_end()
            .to_owned();
    }
    for p in ["# ", "## ", "### ", "#### ", "##### ", "###### "] {
        if let Some(rest) = line.strip_prefix(p) {
            return rest.to_owned();
        }
    }
    if let Some(rest) = line.strip_prefix('>') {
        return rest.strip_prefix(' ').unwrap_or(rest).to_owned();
    }
    if let Some(rest) = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .or_else(|| line.strip_prefix("+ "))
    {
        return rest.to_owned();
    }
    if let Some(rest) = line
        .strip_prefix("1. ")
        .or_else(|| line.strip_prefix("1) "))
    {
        return rest.to_owned();
    }
    line.to_owned()
}

fn is_heading(line: &str) -> bool {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    (1..=6).contains(&hashes) && line[hashes..].starts_with(' ')
}

/// Whether `format` is already applied at the selection — the toggle would take it off.
///
/// The mirror of the wrapping check in [`toggle_inline`], for the toolbar's active state.
pub fn inline_active(src: &str, sel: Range<usize>, format: InlineFormat) -> bool {
    let (start, end) = clamp(src, sel);
    let marker = format.markers();
    if start == end {
        return src[..start].ends_with(marker) && src[end..].starts_with(marker);
    }
    if src[..start].ends_with(marker) && src[end..].starts_with(marker) {
        return true;
    }
    let selected = &src[start..end];
    let trimmed = selected.trim();
    trimmed.len() >= 2 * marker.len()
        && trimmed.starts_with(marker)
        && trimmed.ends_with(marker)
}

/// Whether every line the selection touches already carries `format`'s block prefix, so the
/// toolbar can show it as on.
pub fn block_active(src: &str, sel: Range<usize>, format: BlockFormat) -> bool {
    let (start, end) = clamp(src, sel);
    let line_start = src[..start].rfind('\n').map_or(0, |i| i + 1);
    let line_end = src[end..].find('\n').map_or(src.len(), |i| end + i);
    let lines: Vec<&str> = src[line_start..line_end].split('\n').collect();
    match format {
        BlockFormat::CodeBlock => lines.iter().any(|l| is_fence(l)),
        BlockFormat::Heading(_) => !lines.is_empty() && lines.iter().all(|l| is_heading(l)),
        BlockFormat::BulletList => !lines.is_empty() && lines.iter().all(|l| is_bullet(l)),
        BlockFormat::NumberedList => !lines.is_empty() && lines.iter().all(|l| is_numbered(l)),
        BlockFormat::Quote => !lines.is_empty() && lines.iter().all(|l| is_quote(l)),
    }
}

fn is_bullet(line: &str) -> bool {
    ["- ", "* ", "+ "].iter().any(|p| line.starts_with(p))
}

fn is_numbered(line: &str) -> bool {
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    digits > 0
        && digits + 1 < line.len()
        && matches!(&line[digits..digits + 1], "." | ")")
        && line[digits + 1..].starts_with(' ')
}

fn is_quote(line: &str) -> bool {
    line.starts_with('>')
}

fn is_fence(line: &str) -> bool {
    line.trim_start().starts_with("```")
}

/// Clamp a selection to the source and to character boundaries.
fn clamp(src: &str, sel: Range<usize>) -> (usize, usize) {
    let start = floor_boundary(src, sel.start.min(src.len()));
    let end = floor_boundary(src, sel.end.clamp(start, src.len()));
    (start, end)
}

fn floor_boundary(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sel(a: usize, b: usize) -> Range<usize> {
        a..b
    }

    /// The active-state mirror of the toggles: what the toolbar shows as on is exactly what
    /// a second click would turn off.
    #[test]
    fn active_matches_what_a_toggle_would_remove() {
        // Inline: markers wrapping the selection.
        assert!(inline_active("**bold**", sel(2, 6), InlineFormat::Bold));
        assert!(!inline_active("plain", sel(0, 5), InlineFormat::Bold));
        // With no selection, the caret between the markers counts as active.
        assert!(inline_active("****", sel(2, 2), InlineFormat::Bold));
        assert!(!inline_active("word", sel(2, 2), InlineFormat::Bold));
        // A selection that brought its own markers is active.
        assert!(inline_active("**bold**", sel(0, 8), InlineFormat::Bold));
        assert!(!inline_active("*italic*", sel(1, 7), InlineFormat::Bold));

        // Blocks: every line the selection touches carries the prefix.
        assert!(block_active("# Title", sel(2, 2), BlockFormat::Heading(2)));
        assert!(!block_active("# Title", sel(2, 2), BlockFormat::Quote));
        assert!(block_active("- one\n- two", sel(0, 11), BlockFormat::BulletList));
        assert!(!block_active("- one\ntwo", sel(0, 10), BlockFormat::BulletList));
        assert!(block_active("> quoted", sel(3, 3), BlockFormat::Quote));
        assert!(block_active("```\nx\n```", sel(0, 9), BlockFormat::CodeBlock));
    }

    #[test]
    fn renders_everyday_markup() {
        let html = render_markdown(
            "# Title\n\nSome **bold** and *italic* and `code`.\n\n- one\n- two\n\n1. first\n2. second\n\n> quoted\n\n[link](https://example.com)\n\n```rust\nlet x = 1;\n```\n",
        );
        assert!(html.contains("<h1>Title</h1>"), "{html}");
        assert!(html.contains("<strong>bold</strong>"), "{html}");
        assert!(html.contains("<em>italic</em>"), "{html}");
        assert!(html.contains("<code>code</code>"), "{html}");
        assert!(html.contains("<ul><li>one</li><li>two</li></ul>"), "{html}");
        assert!(
            html.contains("<ol><li>first</li><li>second</li></ol>"),
            "{html}"
        );
        assert!(
            html.contains("<blockquote><p>quoted</p></blockquote>"),
            "{html}"
        );
        assert!(
            html.contains("<a href=\"https://example.com\">link</a>"),
            "{html}"
        );
        assert!(
            html.contains("<pre><code class=\"language-rust\">"),
            "{html}"
        );
    }

    #[test]
    fn an_image_alt_belongs_in_the_attribute_not_the_body() {
        // The alt text arrives as a child event, so a renderer that writes the
        // tag at the start event puts the words in the body and leaves the
        // attribute empty: the preview would show the caption as a paragraph.
        let html = render_markdown("![the logo](cid:logo@x.test)");
        assert!(
            html.contains(r#"<img src="cid:logo@x.test" alt="the logo" />"#),
            "{html}"
        );
        assert_eq!(
            html.matches("the logo").count(),
            1,
            "the alt text must not also be body text: {html}"
        );
    }

    #[test]
    fn an_image_alt_is_escaped_and_keeps_its_title() {
        let html = render_markdown(
            r#"![a "quoted" & <b>markup</b> alt](https://x.test/i.png "the title")"#,
        );
        assert!(html.contains(r#"src="https://x.test/i.png""#), "{html}");
        // A quote in the alt would end the attribute early, and the tag inside
        // the alt is not markup in the output either.
        assert!(
            html.contains(r#"alt="a &quot;quoted&quot; &amp; markup alt""#),
            "{html}"
        );
        assert!(!html.contains("<b>"), "the alt's markup leaked out: {html}");
        assert!(html.contains(r#"title="the title""#), "{html}");
    }

    #[test]
    fn markup_inside_an_alt_stays_text() {
        let html = render_markdown("![a **bold** and `code` word](https://x.test/i.png)");
        assert!(html.contains(r#"alt="a bold and code word""#), "{html}");
        assert!(
            !html.contains("<strong>"),
            "the alt's markup leaked out: {html}"
        );
        assert!(
            !html.contains("<code>"),
            "the alt's markup leaked out: {html}"
        );
    }

    #[test]
    fn an_image_with_an_unusable_destination_keeps_its_words() {
        // The destination would be dropped on send, so the words stand in for
        // the image rather than vanishing with it.
        let html = render_markdown("![the logo](javascript:alert(1))");
        assert!(!html.contains("javascript:"), "{html}");
        assert!(html.contains("the logo"), "{html}");
    }

    #[test]
    fn the_plain_alternative_keeps_the_words_an_image_stands_for() {
        let text = markdown_to_text("before ![the logo](cid:logo@x.test) after");
        assert!(text.contains("the logo"), "{text}");
        assert!(
            !text.contains("cid:"),
            "the destination is not a word: {text}"
        );
    }

    #[test]
    fn a_task_item_carries_its_state_into_both_bodies() {
        let html = render_markdown("- [x] done\n- [ ] todo");
        assert!(
            html.contains(r#"<input disabled="" type="checkbox" checked="" />"#),
            "{html}"
        );
        assert!(
            html.contains(r#"<input disabled="" type="checkbox" />"#),
            "{html}"
        );
        // The state is part of the item, so the HTML must not contradict the
        // plain-text alternative, which has always carried it.
        let text = markdown_to_text("- [x] done\n- [ ] todo");
        assert!(text.contains("[x] done"), "{text}");
        assert!(text.contains("[ ] todo"), "{text}");
    }

    #[test]
    fn renders_tables_with_alignment() {
        let html = render_markdown("| a | b | c |\n| :- | :-: | -: |\n| 1 | 2 | 3 |\n");
        assert!(html.contains("<table>"), "{html}");
        assert!(html.contains("<thead><th align=\"left\">a</th>"), "{html}");
        assert!(html.contains("<td align=\"right\">3</td>"), "{html}");
        assert!(html.contains("align=\"center\""), "{html}");
        assert!(html.contains("align=\"right\""), "{html}");
    }

    #[test]
    fn escapes_source_html_and_keeps_words() {
        let html = render_markdown("hello <script>alert(1)</script> world");
        assert!(!html.contains("<script>"), "{html}");
        assert!(html.contains("hello"), "{html}");
        assert!(html.contains("world"), "{html}");
    }

    #[test]
    fn drops_unsafe_link_but_keeps_text() {
        let html = render_markdown("[click](javascript:alert(1))");
        assert!(!html.contains("javascript:"), "{html}");
        assert!(html.contains("click"), "{html}");
    }

    #[test]
    fn html_escapes_source_text() {
        let html = render_markdown("5 < 6 & \"quoted\"");
        assert!(html.contains("5 &lt; 6 &amp; &quot;quoted&quot;"), "{html}");
    }

    #[test]
    fn plain_text_is_the_readable_alternative() {
        let text = markdown_to_text("# Title\n\n- one\n- two\n\n[link](https://example.com)\n");
        assert!(text.contains("Title"), "{text}");
        assert!(text.contains("- one"), "{text}");
        assert!(text.contains("link (https://example.com)"), "{text}");
        assert!(!text.contains('<'), "{text}");
    }

    #[test]
    fn plain_text_table_rows_are_tab_separated() {
        let text = markdown_to_text("| a | b |\n| --- | --- |\n| 1 | 2 |\n");
        assert!(text.contains("a\tb"), "{text}");
        assert!(text.contains("1\t2"), "{text}");
    }

    #[test]
    fn inline_toggle_wraps_then_unwraps() {
        let src = "hello world";
        let on = toggle_inline(src, sel(6, 11), InlineFormat::Bold);
        assert_eq!(on.text, "hello **world**");
        assert_eq!(&on.text[on.selection.clone()], "world");
        let off = toggle_inline(&on.text, on.selection, InlineFormat::Bold);
        assert_eq!(off.text, src);
    }

    #[test]
    fn inline_toggle_with_empty_selection_parks_the_caret() {
        let edit = toggle_inline("ab", sel(1, 1), InlineFormat::Italic);
        assert_eq!(edit.text, "a**b");
        assert_eq!(edit.selection, 2..2);
    }

    #[test]
    fn inline_toggle_survives_unicode_selection() {
        let src = "héllo wörld";
        let edit = toggle_inline(src, sel(7, 13), InlineFormat::Bold);
        assert_eq!(edit.text, "héllo **wörld**");
    }

    #[test]
    fn block_toggle_replaces_other_formats() {
        let heading = toggle_block("- one\n- two", sel(0, 11), BlockFormat::Heading(2));
        assert_eq!(heading.text, "## one\n## two");
        let bullet = toggle_block(&heading.text, sel(0, 13), BlockFormat::BulletList);
        assert_eq!(bullet.text, "- one\n- two");
        let none = toggle_block(&bullet.text, sel(0, 11), BlockFormat::BulletList);
        assert_eq!(none.text, "one\ntwo");
    }

    #[test]
    fn block_toggle_moves_the_selection_with_the_prefix() {
        let both = toggle_block("one\ntwo", sel(0, 7), BlockFormat::Quote);
        assert_eq!(both.text, "> one\n> two");
        assert!(
            both.text[both.selection.clone()].ends_with("two"),
            "{:?}",
            both.selection
        );

        // A selection on the second line only prefixes that line, and keeps
        // pointing at the words the user had selected.
        let second = toggle_block("one\ntwo", sel(4, 7), BlockFormat::Quote);
        assert_eq!(second.text, "one\n> two");
        assert_eq!(&second.text[second.selection.clone()], "two");
    }

    #[test]
    fn link_insertion_wraps_selection_or_parks_caret() {
        let wrapped = insert_link("see docs", sel(4, 8), "https://example.com", None);
        assert_eq!(wrapped.text, "see [docs](https://example.com)");
        assert_eq!(&wrapped.text[wrapped.selection.clone()], "docs");
        let empty = insert_link("ab", sel(1, 1), "https://example.com", None);
        assert_eq!(empty.text, "a[](https://example.com)b");
        assert_eq!(empty.selection, 2..2);
    }
    #[test]
    fn inline_toggle_beside_multibyte_characters_is_safe() {
        // Byte offsets that fall inside a character are moved onto a character
        // boundary, so no edit ever slices a character in half.
        let src = "héllo";
        assert!(!src.is_char_boundary(2), "the caret is inside `é`");
        let edit = toggle_inline(src, sel(2, 2), InlineFormat::Italic);
        assert_eq!(
            edit.text.chars().filter(|c| *c == 'é').count(),
            1,
            "{}",
            edit.text
        );
        assert!(edit.text.is_char_boundary(edit.selection.start));
        assert!(edit.text.is_char_boundary(edit.selection.end));

        // The marker goes on a boundary and the words around it are untouched.
        let emoji = "☕ ok";
        let wrapped = toggle_inline(emoji, sel(4, 6), InlineFormat::Bold);
        assert_eq!(wrapped.text, "☕ **ok**");
        assert_eq!(&wrapped.text[wrapped.selection.clone()], "ok");

        // A caret inside the emoji cannot split it either.
        let split = toggle_inline(emoji, sel(2, 2), InlineFormat::Bold);
        assert!(split.text.chars().any(|c| c == '☕'), "{}", split.text);
        assert!(split.text.is_char_boundary(split.selection.start));
    }

    #[test]
    fn code_block_toggle_wraps_once_and_unwraps() {
        let wrapped = toggle_block("alpha\nbeta", sel(0, 10), BlockFormat::CodeBlock);
        assert_eq!(wrapped.text, "```\nalpha\nbeta\n```\n");
        let unwrapped = toggle_block(
            &wrapped.text,
            sel(0, wrapped.text.len()),
            BlockFormat::CodeBlock,
        );
        assert_eq!(unwrapped.text, "alpha\nbeta");
    }

    #[test]
    fn a_code_block_fence_grows_past_a_fence_in_the_code() {
        let code = "here\n```\nthere";
        let edit = toggle_block(code, sel(0, code.len()), BlockFormat::CodeBlock);
        assert!(edit.text.starts_with("````\n"), "{}", edit.text);
        assert!(edit.text.contains("\n````\n"), "{}", edit.text);
    }

    #[test]
    fn plain_paste_keeps_its_characters_literal() {
        let pasted = plain_text_to_markdown("<div>Important</div>\n**literal**");
        assert_eq!(pasted, "\\<div>Important\\</div>\\\n\\*\\*literal\\*\\*");
        let html = render_markdown(&pasted);
        assert!(html.contains("&lt;div&gt;Important&lt;/div&gt;"), "{html}");
        assert!(html.contains("**literal**"), "{html}");
        assert!(!html.contains("<div>"), "{html}");
    }

    #[test]
    fn a_plain_body_is_sent_as_itself() {
        let html = plain_text_to_html("**literal**\nsecond line <tag>");
        assert_eq!(html, "<p>**literal**<br />\nsecond line &lt;tag&gt;</p>");
    }

    #[test]
    fn table_insertion_builds_a_usable_table() {
        let edit = insert_table("text\n", sel(0, 0), 2, 2);
        assert!(
            edit.text.contains("| Column 1 | Column 2 |"),
            "{}",
            edit.text
        );
        assert!(edit.text.contains("| --- | --- |"), "{}", edit.text);
        assert_eq!(edit.text.matches("|  |  |").count(), 2, "{}", edit.text);
        let md = &edit.text[edit.selection.start..];
        assert!(
            md.starts_with(" |  |"),
            "caret inside the first cell: {}",
            edit.text
        );
    }
}
