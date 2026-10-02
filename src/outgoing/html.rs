//! HTML → Markdown conversion, HTML sanitising and HTML → text, all over an
//! html5ever DOM (through `scraper`).
//!
//! The parser does the structural work; the conversion rules here decide what
//! survives into Markdown and what is reported as lossy, which is what the
//! composer's "this body has styling Markdown cannot keep" approval is built on.

use ego_tree::NodeRef;
use scraper::node::Element;
use scraper::{Html, Node};

use super::body::{ConversionWarning, HtmlConversion};

/// Convert an HTML fragment (a rich body imported from a provider, or pasted
/// from a browser) to Markdown. Styles, fonts and colours are dropped and
/// reported; the words and the supported structure are kept.
pub fn html_to_markdown(html: &str) -> HtmlConversion {
    let dom = Html::parse_fragment(html);
    let mut w = MdWriter::default();
    w.blocks(dom.tree.root());
    w.flush_para();
    let mut markdown = w.out;
    while markdown.ends_with('\n') {
        markdown.pop();
    }
    let lossy = !w.warnings.is_empty();
    HtmlConversion {
        markdown,
        text: super::html_text(html),
        warnings: w.warnings,
        lossy,
    }
}

/// Readable plain text of an HTML fragment.
pub fn html_text(html: &str) -> String {
    crate::reading::html_to_text(html)
}

// -------------------------------------------------------------- conversion

#[derive(Default)]
struct MdWriter {
    out: String,
    warnings: Vec<ConversionWarning>,
    /// Inline content collected so far, flushed as one paragraph.
    para: String,
    quote: usize,
    lists: usize,
    in_table: bool,
}

impl MdWriter {
    fn warn(&mut self, w: ConversionWarning) {
        if !self.warnings.contains(&w) {
            self.warnings.push(w);
        }
    }

    fn flush_para(&mut self) {
        if self.para.trim().is_empty() {
            self.para.clear();
            return;
        }
        let text = std::mem::take(&mut self.para);
        let text = text.trim_end();
        for line in text.split('\n') {
            self.out.push_str(line);
            self.out.push('\n');
        }
        self.out.push('\n');
    }

    fn blocks(&mut self, node: NodeRef<'_, Node>) {
        for child in node.children() {
            self.block(child);
        }
    }

    fn block(&mut self, node: NodeRef<'_, Node>) {
        if node.value().is_text() {
            self.push_text(&self.text_of(node));
            return;
        }
        if !node.value().is_element() {
            return;
        }
        let el = node.value().as_element().expect("element");
        let name = el.name().to_ascii_lowercase();
        match name.as_str() {
            "script" | "style" | "head" | "meta" | "link" | "noscript" | "title" | "iframe"
            | "object" | "embed" | "applet" | "svg" | "canvas" | "video" | "audio" | "form"
            | "button" | "select" | "textarea" | "template" => {
                self.warn(ConversionWarning::UnsupportedElement { element: name });
            }
            "input" => {
                // A checkbox is a list marker, handled by its list item.
                if is_checkbox(node).is_none() {
                    self.warn(ConversionWarning::UnsupportedElement { element: name });
                }
            }
            "p" | "div" | "section" | "article" | "header" | "footer" | "main" | "aside"
            | "nav" | "figure" | "figcaption" | "center" | "body" | "html" | "fieldset"
            | "details" | "summary" | "address" | "dl" | "dt" | "dd" => {
                self.flush_para();
                self.warn_presentation(el);
                self.blocks(node);
                self.flush_para();
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.flush_para();
                let level = name[1..].parse::<usize>().unwrap_or(1).clamp(1, 6);
                self.out.push_str(&"#".repeat(level));
                self.out.push(' ');
                let mut inner = String::new();
                self.inline_children(node, &mut inner);
                self.out.push_str(inner.trim());
                self.out.push_str("\n\n");
            }
            "blockquote" => {
                self.flush_para();
                let start = self.out.len();
                self.quote += 1;
                self.blocks(node);
                self.quote -= 1;
                let inner = self.out.split_off(start);
                let inner = inner.trim_matches('\n');
                for line in inner.split('\n') {
                    self.out.push_str("> ");
                    self.out.push_str(line);
                    self.out.push('\n');
                }
                self.out.push('\n');
            }
            "pre" => {
                self.flush_para();
                // Whitespace and punctuation inside code are literal, so the
                // text is taken raw and fenced with a run the code cannot close.
                let code = raw_text(node);
                let fence = "`".repeat(longest_backtick_run(&code).max(2) + 1);
                self.out.push_str(&fence);
                self.out.push('\n');
                for line in code.trim_matches('\n').split('\n') {
                    self.out.push_str(line);
                    self.out.push('\n');
                }
                self.out.push_str(&fence);
                self.out.push_str("\n\n");
            }
            "hr" => {
                self.flush_para();
                self.out.push_str("---\n\n");
            }
            "ul" => {
                self.flush_para();
                self.list(node, false, 1);
            }
            "ol" => {
                self.flush_para();
                let start = el
                    .attr("start")
                    .and_then(|s| s.trim().parse().ok())
                    .unwrap_or(1);
                self.list(node, true, start);
            }
            "li" => {
                // A stray list item outside a list: keep its text.
                self.flush_para();
                self.blocks(node);
            }
            "table" => {
                self.flush_para();
                self.warn_presentation(el);
                self.table(node);
            }
            _ if INLINE.contains(&name.as_str()) => {
                self.warn_presentation(el);
                let mut inner = String::new();
                self.inline(node, &mut inner);
                self.para.push_str(&inner);
            }
            _ => {
                self.warn_presentation(el);
                self.blocks(node);
                self.warn(ConversionWarning::UnsupportedElement { element: name });
            }
        }
    }

    /// Record styling Markdown has no equivalent for.
    fn warn_presentation(&mut self, el: &Element) {
        for attr in [
            "style", "class", "id", "align", "width", "height", "bgcolor", "color",
        ] {
            if el.attr(attr).is_some_and(|v| !v.trim().is_empty()) {
                self.warn(ConversionWarning::DroppedStyle {
                    property: attr.to_owned(),
                });
            }
        }
        for (name, _) in el.attrs() {
            let name = name.to_ascii_lowercase();
            if name.starts_with("on") {
                self.warn(ConversionWarning::DroppedStyle { property: name });
            }
        }
    }

    fn list(&mut self, node: NodeRef<'_, Node>, ordered: bool, start: usize) {
        self.lists += 1;
        let indent = "  ".repeat(self.lists - 1);
        let mut number = start;
        for item in node.children() {
            if !item.value().is_element() {
                continue;
            }
            let el = item.value().as_element().expect("element");
            if !el.name().eq_ignore_ascii_case("li") {
                continue;
            }
            self.flush_para();
            let task = checkbox(item);
            let marker = if task.is_some() {
                "- ".to_owned()
            } else if ordered {
                let m = format!("{number}. ");
                number += 1;
                m
            } else {
                "- ".to_owned()
            };
            let marker = match task {
                Some(true) => "- [x] ".to_owned(),
                Some(false) => "- [ ] ".to_owned(),
                None => marker,
            };
            let start_at = self.out.len();
            self.blocks(item);
            self.flush_para();
            let body = self.out.split_off(start_at);
            // Blank lines inside an item would become stray indentation.
            let body: Vec<&str> = body
                .lines()
                .filter(|line| !line.trim().is_empty())
                .collect();
            let hang = " ".repeat(indent.len() + marker.chars().count());
            for (i, line) in body.iter().enumerate() {
                if i > 0 {
                    self.out.push('\n');
                    self.out.push_str(&hang);
                }
                if i == 0 {
                    self.out.push_str(&indent);
                    self.out.push_str(&marker);
                }
                self.out.push_str(line);
            }
            self.out.push('\n');
        }
        self.lists -= 1;
        if self.out.ends_with("\n\n") {
            self.out.pop();
        } else {
            self.out.push('\n');
        }
    }

    fn table(&mut self, node: NodeRef<'_, Node>) {
        let mut rows: Vec<(bool, Vec<(String, Align)>)> = Vec::new();
        let mut warnings = Vec::new();
        collect_rows(node, false, &mut rows, &mut warnings);
        for warning in warnings {
            self.warn(warning);
        }
        let Some((first_head, _)) = rows.first().cloned() else {
            self.warn(ConversionWarning::UnsupportedElement {
                element: "table".to_owned(),
            });
            return;
        };
        let columns = rows.iter().map(|(_, cells)| cells.len()).max().unwrap_or(0);
        if columns == 0 {
            self.warn(ConversionWarning::UnsupportedElement {
                element: "table".to_owned(),
            });
            return;
        }
        if !first_head {
            self.warn(ConversionWarning::TableWithoutHeader);
        }
        let align_of = |i: usize| rows[0].1.get(i).map_or(Align::None, |(_, a)| *a);
        let cell = |i: usize| {
            rows[0]
                .1
                .get(i)
                .map_or(String::new(), |(text, _)| text.clone())
        };
        self.out.push('|');
        for i in 0..columns {
            self.out.push(' ');
            self.out.push_str(&cell(i));
            self.out.push_str(" |");
        }
        self.out.push('\n');
        self.out.push('|');
        for i in 0..columns {
            self.out.push(' ');
            self.out.push_str(match align_of(i) {
                Align::None => "---",
                Align::Left => ":--",
                Align::Center => ":-:",
                Align::Right => "--:",
            });
            self.out.push_str(" |");
        }
        self.out.push('\n');
        for (_, cells) in rows.iter().skip(1) {
            self.out.push('|');
            for i in 0..columns {
                self.out.push(' ');
                self.out
                    .push_str(&cells.get(i).map_or(String::new(), |(t, _)| t.clone()));
                self.out.push_str(" |");
            }
            self.out.push('\n');
        }
    }

    fn inline_children(&mut self, node: NodeRef<'_, Node>, out: &mut String) {
        for child in node.children() {
            self.inline(child, out);
        }
    }

    fn inline(&mut self, node: NodeRef<'_, Node>, out: &mut String) {
        if node.value().is_text() {
            out.push_str(&self.text_of(node));
            return;
        }
        if !node.value().is_element() {
            return;
        }
        let el = node.value().as_element().expect("element");
        let name = el.name().to_ascii_lowercase();
        match name.as_str() {
            "b" | "strong" => {
                let mut inner = String::new();
                self.inline_children(node, &mut inner);
                out.push_str(&emphasize(&inner, "**"));
            }
            "i" | "em" | "cite" | "var" => {
                let mut inner = String::new();
                self.inline_children(node, &mut inner);
                out.push_str(&emphasize(&inner, "*"));
            }
            "s" | "strike" | "del" => {
                let mut inner = String::new();
                self.inline_children(node, &mut inner);
                out.push_str(&emphasize(&inner, "~~"));
            }
            "code" | "kbd" | "samp" | "tt" => {
                let mut inner = String::new();
                self.inline_children(node, &mut inner);
                let inner = inner.trim();
                out.push('`');
                out.push_str(&inner.replace('`', "\\`"));
                out.push('`');
            }
            "a" => {
                let href = el.attr("href").unwrap_or("").trim().to_owned();
                let mut inner = String::new();
                self.inline_children(node, &mut inner);
                if super::markdown::safe_url(&href) {
                    out.push('[');
                    out.push_str(inner.trim());
                    out.push_str("](");
                    out.push_str(&href.replace(')', "%29"));
                    out.push(')');
                } else {
                    out.push_str(inner.trim());
                    if !href.is_empty() {
                        self.warn(ConversionWarning::UnsafeLink { url: href });
                    }
                }
            }
            "img" => {
                let alt = el.attr("alt").unwrap_or("").trim();
                let src = el.attr("src").unwrap_or("").trim();
                if src.is_empty() {
                    out.push_str(alt);
                } else {
                    out.push('[');
                    out.push_str(if alt.is_empty() { "image" } else { alt });
                    out.push_str("](");
                    out.push_str(src);
                    out.push(')');
                }
                // Remote images are trackers in mail: the user should know the
                // body referenced one.
                self.warn(ConversionWarning::Image);
            }
            "br" => out.push_str("\\\n"),
            "u" | "ins" => {
                self.warn(ConversionWarning::DroppedStyle {
                    property: "underline".to_owned(),
                });
                self.inline_children(node, out);
            }
            "mark" => {
                self.warn(ConversionWarning::DroppedStyle {
                    property: "background-color".to_owned(),
                });
                self.inline_children(node, out);
            }
            "span" | "font" | "big" | "small" | "sub" | "sup" | "abbr" | "time" | "label" | "q" => {
                self.warn_presentation(el);
                self.inline_children(node, out);
            }
            "script" | "style" | "head" | "noscript" | "iframe" | "object" | "svg" | "form"
            | "button" | "select" | "textarea" | "template" | "input" => {
                self.warn(ConversionWarning::UnsupportedElement { element: name });
            }
            _ => {
                // A block element reached in inline context (inside a cell or
                // a heading) loses the structure Markdown would have given
                // it, so the words are kept but the loss is reported.
                let block = BLOCK_ONLY.contains(&name.as_str());
                self.warn_presentation(el);
                if block {
                    self.warn(ConversionWarning::UnsupportedElement { element: name });
                }
                self.inline_children(node, out);
            }
        }
    }

    fn text_of(&self, node: NodeRef<'_, Node>) -> String {
        let text = node
            .value()
            .as_text()
            .map(|t| t.to_string())
            .unwrap_or_default();
        let at_line_start = self.para.is_empty() || self.para.ends_with('\n');
        normalize_text(&text, self.in_table, at_line_start)
    }

    fn push_text(&mut self, text: &str) {
        self.para.push_str(text);
    }
}

/// Elements handled inline wherever they appear, so a `<b>` that ends up
/// directly inside a block still keeps its Markdown markers.
const INLINE: &[&str] = &[
    "b", "strong", "i", "em", "cite", "var", "s", "strike", "del", "code", "kbd", "samp", "tt",
    "a", "img", "br", "u", "ins", "mark", "span", "font", "big", "small", "sub", "sup", "abbr",
    "time", "label", "q",
];

/// Block-level elements that carry no inline Markdown. Reaching one while
/// collecting inline content (a table cell, a heading) means its structure
/// cannot be represented there, which is a lossy conversion to report.
const BLOCK_ONLY: &[&str] = &[
    "p",
    "div",
    "section",
    "article",
    "header",
    "footer",
    "main",
    "aside",
    "nav",
    "figure",
    "figcaption",
    "center",
    "body",
    "html",
    "fieldset",
    "details",
    "summary",
    "address",
    "dl",
    "dt",
    "dd",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "blockquote",
    "pre",
    "hr",
    "ul",
    "ol",
    "li",
    "table",
];

/// Emphasis with the whitespace that surrounded it kept outside the
/// delimiters: `<b> two </b>` must not become `one**two**three`, which would
/// join two words that were visibly apart.
fn emphasize(inner: &str, marker: &str) -> String {
    let lead_len = inner.len() - inner.trim_start().len();
    let (lead, rest) = inner.split_at(lead_len);
    let trail_len = rest.len() - rest.trim_end().len();
    let (core, trail) = rest.split_at(rest.len() - trail_len);
    if core.is_empty() {
        return inner.to_owned();
    }
    format!("{lead}{marker}{core}{marker}{trail}")
}

/// Presentation a table cell carries beyond its alignment, which Markdown has
/// no place for: it makes the whole conversion lossy.
fn warn_cell_presentation(el: &Element, warnings: &mut Vec<ConversionWarning>) {
    let mut push = |property: String| {
        let warning = ConversionWarning::DroppedStyle { property };
        if !warnings.contains(&warning) {
            warnings.push(warning);
        }
    };
    // `colspan`/`rowspan` are structure Markdown tables cannot express, so a
    // spanned cell is reported rather than silently narrowed to one column.
    for attr in [
        "bgcolor",
        "background",
        "width",
        "height",
        "class",
        "id",
        "nowrap",
        "colspan",
        "rowspan",
    ] {
        if el.attr(attr).is_some_and(|v| !v.trim().is_empty()) {
            push(attr.to_owned());
        }
    }
    // Only `text-align` survives; any other declaration is lost styling.
    if let Some(style) = el.attr("style").filter(|v| !v.trim().is_empty()) {
        let extra: Vec<&str> = style
            .split(';')
            .filter_map(|decl| decl.split_once(':'))
            .filter(|(prop, _)| !prop.trim().eq_ignore_ascii_case("text-align"))
            .map(|(prop, _)| prop.trim())
            .filter(|prop| !prop.is_empty())
            .collect();
        if !extra.is_empty() {
            push(format!("style ({})", extra.join(", ")));
        }
    }
}

/// The text of a node exactly as the parser decoded it, whitespace intact.
fn raw_text(node: NodeRef<'_, Node>) -> String {
    let mut out = String::new();
    for child in node.descendants() {
        if let Some(text) = child.value().as_text() {
            out.push_str(text);
        }
    }
    out
}

fn longest_backtick_run(text: &str) -> usize {
    let mut longest = 0usize;
    let mut current = 0usize;
    for c in text.chars() {
        if c == '`' {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    longest
}

fn checkbox(item: NodeRef<'_, Node>) -> Option<bool> {
    let first = item.children().find(|c| c.value().is_element())?;
    is_checkbox(first)
}

fn is_checkbox(node: NodeRef<'_, Node>) -> Option<bool> {
    let el = node.value().as_element()?;
    if !el.name().eq_ignore_ascii_case("input") {
        return None;
    }
    if !el
        .attr("type")
        .is_some_and(|t| t.eq_ignore_ascii_case("checkbox"))
    {
        return None;
    }
    Some(el.attr("checked").is_some())
}

fn collect_rows(
    node: NodeRef<'_, Node>,
    in_head: bool,
    rows: &mut Vec<(bool, Vec<(String, Align)>)>,
    warnings: &mut Vec<ConversionWarning>,
) {
    for child in node.children() {
        if !child.value().is_element() {
            continue;
        }
        let el = child.value().as_element().expect("element");
        match el.name().to_ascii_lowercase().as_str() {
            "thead" => collect_rows(child, true, rows, warnings),
            "tbody" | "tfoot" => collect_rows(child, in_head, rows, warnings),
            "tr" => {
                // A row's own presentation is dropped too, not only its cells'.
                warn_cell_presentation(el, warnings);
                let mut cells = Vec::new();
                for cell in child.children() {
                    if !cell.value().is_element() {
                        continue;
                    }
                    let cel = cell.value().as_element().expect("element");
                    let name = cel.name().to_ascii_lowercase();
                    if name != "td" && name != "th" {
                        continue;
                    }
                    let mut text = String::new();
                    let mut sub = MdWriter {
                        in_table: true,
                        ..MdWriter::default()
                    };
                    sub.inline_children(cell, &mut text);
                    // Styling dropped inside a cell is still a lossy conversion.
                    warnings.extend(sub.warnings);
                    warn_cell_presentation(cel, warnings);
                    let align = cel
                        .attr("align")
                        .map(Align::parse)
                        .or_else(|| {
                            cel.attr("style")
                                .and_then(|s| alignment_from_style(s))
                                .map(Align::parse)
                        })
                        .unwrap_or(Align::None);
                    cells.push((text.trim().to_owned(), align));
                }
                if !cells.is_empty() {
                    rows.push((in_head || cells_look_like_header(child), cells));
                }
            }
            _ => collect_rows(child, in_head, rows, warnings),
        }
    }
}

fn cells_look_like_header(row: NodeRef<'_, Node>) -> bool {
    let cells: Vec<_> = row
        .children()
        .filter(|c| c.value().is_element())
        .filter_map(|c| c.value().as_element())
        .collect();
    !cells.is_empty() && cells.iter().all(|c| c.name().eq_ignore_ascii_case("th"))
}

fn alignment_from_style(style: &str) -> Option<&str> {
    for decl in style.split(';') {
        let Some((prop, value)) = decl.split_once(':') else {
            continue;
        };
        if prop.trim().eq_ignore_ascii_case("text-align") {
            return Some(value.trim());
        }
    }
    None
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Align {
    None,
    Left,
    Center,
    Right,
}

impl Align {
    fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "left" => Align::Left,
            "center" => Align::Center,
            "right" => Align::Right,
            _ => Align::None,
        }
    }
}

/// Collapse HTML whitespace, keep a block marker from turning into Markdown
/// structure, and escape the characters Markdown would otherwise read as
/// syntax.
fn normalize_text(text: &str, in_table: bool, at_line_start: bool) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_space = false;
    for c in text.chars() {
        if c.is_whitespace() || c == '\u{a0}' {
            if !last_space {
                out.push(' ');
                last_space = true;
            }
            continue;
        }
        last_space = false;
        if at_line_start && out.is_empty() && matches!(c, '#' | '>' | '-' | '+' | '=' | '`' | '|') {
            out.push('\\');
        }
        match c {
            '\\' | '`' | '*' | '_' | '[' | ']' => {
                out.push('\\');
                out.push(c);
            }
            '|' if in_table => out.push_str("\\|"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_headings_emphasis_and_links() {
        let c = html_to_markdown(
            "<h1>Title</h1><p>Some <b>bold</b> and <i>italic</i> and <a href=\"https://x.test/a\">link</a></p>",
        );
        assert!(c.markdown.contains("# Title"), "{}", c.markdown);
        assert!(c.markdown.contains("**bold**"), "{}", c.markdown);
        assert!(c.markdown.contains("*italic*"), "{}", c.markdown);
        assert!(
            c.markdown.contains("[link](https://x.test/a)"),
            "{}",
            c.markdown
        );
        assert!(!c.lossy, "{:?}", c.warnings);
    }

    #[test]
    fn strips_fonts_and_colours_but_keeps_words() {
        let c = html_to_markdown(
            "<p><span style=\"color:#ff0000;font-family:Arial\">Hello</span> <font color=\"red\" size=\"3\">there</font></p>",
        );
        assert_eq!(c.markdown.trim(), "Hello there");
        assert!(c.lossy);
        assert!(
            c.warnings
                .iter()
                .any(|w| matches!(w, ConversionWarning::DroppedStyle { .. }))
        );
    }

    #[test]
    fn converts_lists_including_nesting_and_tasks() {
        let c = html_to_markdown(
            "<ul><li>one</li><li>two<ul><li>nested</li></ul></li></ul><ol><li>first</li></ol>",
        );
        assert!(c.markdown.contains("- one"), "{}", c.markdown);
        assert!(c.markdown.contains("  - nested"), "{}", c.markdown);
        assert!(c.markdown.contains("1. first"), "{}", c.markdown);
    }

    #[test]
    fn converts_checkbox_items() {
        let c = html_to_markdown(
            "<ul><li><input type=\"checkbox\" checked>done</li><li><input type=\"checkbox\">todo</li></ul>",
        );
        assert!(c.markdown.contains("- [x] done"), "{}", c.markdown);
        assert!(c.markdown.contains("- [ ] todo"), "{}", c.markdown);
    }

    #[test]
    fn converts_tables_with_alignment() {
        let c = html_to_markdown(
            "<table><thead><tr><th>a</th><th style=\"text-align:right\">b</th></tr></thead><tbody><tr><td>1</td><td>2</td></tr></tbody></table>",
        );
        assert!(c.markdown.contains("| a | b |"), "{}", c.markdown);
        assert!(c.markdown.contains("| --- | --: |"), "{}", c.markdown);
        assert!(c.markdown.contains("| 1 | 2 |"), "{}", c.markdown);
    }

    #[test]
    fn reports_headers_dropped_from_headerless_tables() {
        let c = html_to_markdown(
            "<table><tr><td>1</td><td>2</td></tr><tr><td>3</td><td>4</td></tr></table>",
        );
        assert!(
            c.warnings.contains(&ConversionWarning::TableWithoutHeader),
            "{:?}",
            c.warnings
        );
    }

    #[test]
    fn reports_unsupported_elements() {
        let c =
            html_to_markdown("<div><p>keep</p><iframe src=\"https://ads.test\"></iframe></div>");
        assert!(c.markdown.contains("keep"), "{}", c.markdown);
        assert!(
            c.warnings.contains(&ConversionWarning::UnsupportedElement {
                element: "iframe".to_owned()
            }),
            "{:?}",
            c.warnings
        );
        assert!(c.lossy);
    }

    #[test]
    fn reports_remote_images() {
        let c = html_to_markdown("<p><img src=\"https://track.test/p.gif\" alt=\"pixel\"></p>");
        assert!(
            c.markdown.contains("[pixel](https://track.test/p.gif)"),
            "{}",
            c.markdown
        );
        assert!(c.warnings.contains(&ConversionWarning::Image));
    }

    #[test]
    fn drops_javascript_links_but_keeps_their_words() {
        let c = html_to_markdown("<p><a href=\"javascript:alert(1)\">press</a></p>");
        assert!(c.markdown.contains("press"), "{}", c.markdown);
        assert!(!c.markdown.contains("javascript:"), "{}", c.markdown);
        assert!(
            c.warnings
                .iter()
                .any(|w| matches!(w, ConversionWarning::UnsafeLink { .. }))
        );
    }

    #[test]
    fn escapes_markdown_syntax_in_words() {
        let c = html_to_markdown("<p>2 * 3 _ 4 [5] 6</p>");
        assert!(c.markdown.contains("\\*"), "{}", c.markdown);
        assert!(c.markdown.contains("\\_"), "{}", c.markdown);
        assert!(c.markdown.contains("\\[5\\]"), "{}", c.markdown);
    }

    #[test]
    fn text_alternative_is_readable() {
        let c = html_to_markdown("<p>One</p><p>Two</p>");
        assert!(c.text.contains("One"), "{}", c.text);
        assert!(c.text.contains("Two"), "{}", c.text);
    }

    #[test]
    fn imported_code_keeps_its_shape() {
        let c = html_to_markdown("<pre>if (x) {\n  x++;\n  let _y = 1;\n}</pre>");
        assert!(c.markdown.contains("if (x) {"), "{}", c.markdown);
        assert!(
            c.markdown.contains("  x++;"),
            "indentation is kept: {}",
            c.markdown
        );
        assert!(
            c.markdown.contains("let _y = 1;"),
            "underscores are literal: {}",
            c.markdown
        );
        assert!(
            !c.markdown.contains("\\_"),
            "nothing is escaped inside a fence: {}",
            c.markdown
        );
    }

    #[test]
    fn a_fence_in_the_code_pushes_the_delimiter_out() {
        let c = html_to_markdown("<pre>before\n```\nafter</pre>");
        // Three backticks in the code, so the fence is four. The returned
        // Markdown has no trailing newline.
        assert!(c.markdown.starts_with("````\n"), "{}", c.markdown);
        assert!(c.markdown.ends_with("\n````"), "{}", c.markdown);
    }

    #[test]
    fn emphasis_keeps_the_space_that_separated_the_words() {
        let c = html_to_markdown("<p>one<b> two </b>three</p>");
        assert!(c.markdown.contains("one **two** three"), "{:?}", c.markdown);
    }

    #[test]
    fn styling_dropped_inside_a_cell_makes_the_conversion_lossy() {
        let c = html_to_markdown(
            "<table><thead><tr><th>a</th></tr></thead><tbody><tr><td><span style=\"color:red\">red</span></td></tr></tbody></table>",
        );
        assert!(c.lossy, "the cell lost its style: {:?}", c.warnings);
        assert!(c.markdown.contains("red"), "{}", c.markdown);
    }

    #[test]
    fn a_cell_span_is_reported_as_lost_structure() {
        // A spanned cell becomes one column, so the table the user ends up
        // with is not the table they pasted: that has to be reported.
        let c = html_to_markdown(
            "<table><thead><tr><th colspan=\"2\">wide</th></tr></thead><tbody><tr><td>a</td><td>b</td></tr></tbody></table>",
        );
        assert!(c.lossy, "the span was dropped: {:?}", c.warnings);
        assert!(
            c.warnings.iter().any(
                |w| matches!(w, ConversionWarning::DroppedStyle { property } if property == "colspan")
            ),
            "{:?}",
            c.warnings
        );
    }

    #[test]
    fn a_tables_own_presentation_is_reported() {
        let c = html_to_markdown(
            "<table style=\"border-collapse:collapse\"><tbody><tr><td>1</td></tr></tbody></table>",
        );
        assert!(c.lossy, "the table lost its styling: {:?}", c.warnings);
        assert!(c.markdown.contains('1'), "{}", c.markdown);
    }

    #[test]
    fn a_row_presentation_is_reported() {
        let c = html_to_markdown(
            "<table><tbody><tr bgcolor=\"#eeeeee\"><td>1</td></tr></tbody></table>",
        );
        assert!(c.lossy, "the row lost its colour: {:?}", c.warnings);
    }

    #[test]
    fn a_block_element_inside_a_cell_is_reported() {
        // The words survive, but the list they were in cannot be a cell.
        let c = html_to_markdown(
            "<table><thead><tr><th>a</th></tr></thead><tbody><tr><td><ul><li>one</li></ul></td></tr></tbody></table>",
        );
        assert!(c.lossy, "the list was flattened: {:?}", c.warnings);
        assert!(c.markdown.contains("one"), "{}", c.markdown);
    }

    #[test]
    fn a_plain_table_is_not_reported_as_lossy() {
        let c = html_to_markdown(
            "<table><thead><tr><th>a</th><th>b</th></tr></thead><tbody><tr><td>1</td><td>2</td></tr></tbody></table>",
        );
        assert!(!c.lossy, "{:?}", c.warnings);
    }

    #[test]
    fn an_inline_and_list_body_is_not_reported_as_lossy() {
        let c = html_to_markdown("<p>plain <b>bold</b> and <i>italic</i></p><ul><li>one</li></ul>");
        assert!(!c.lossy, "{:?}", c.warnings);
    }
}
