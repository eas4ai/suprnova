//! A tolerant HTML tokenizer for Askama views (REG-031). It follows the
//! HTML parser's states where they decide what is a tag, an attribute or a
//! comment, so the scan sees the markup a browser builds. Template output
//! reaches it as dynamic pieces: an expression may stand for a text node or
//! an attribute value, never for markup inside a tag, and a template branch
//! must leave the tokenizer in the same state on every path.

use super::super::Finding;
use super::super::url::{UrlRefusal, check_constant, prefix_commits_to_origin, srcset_urls};
use super::css;

/// Elements a view may not hold (REG-031): each one loads or runs
/// something the scan cannot read, or changes how the page resolves URLs.
pub const REFUSED_ELEMENTS: &[&str] = &[
    "script",
    "iframe",
    "frame",
    "frameset",
    "object",
    "embed",
    "applet",
    "portal",
    "base",
    "link",
    "meta",
    "animate",
    "animatemotion",
    "animatetransform",
    "set",
    "foreignobject",
    "handler",
    "listener",
];

/// Attributes that carry a URL the browser loads or navigates to.
pub const URL_ATTRIBUTES: &[&str] = &[
    "href",
    "src",
    "srcset",
    "action",
    "formaction",
    "poster",
    "data",
    "xlink:href",
    "ping",
    "background",
    "cite",
    "longdesc",
    "lowsrc",
    "dynsrc",
    "codebase",
    "manifest",
    "icon",
    "imagesrcset",
    "usemap",
    "archive",
    "classid",
];

/// Attributes a view may not write at all.
pub const REFUSED_ATTRIBUTES: &[&str] = &["srcdoc", "xml:base"];

/// Elements whose content the HTML parser reads as text, not markup.
const RAW_TEXT_ELEMENTS: &[&str] = &[
    "script",
    "style",
    "xmp",
    "iframe",
    "noembed",
    "noframes",
    "textarea",
    "title",
    "plaintext",
];

/// Where a refusal goes.
pub(crate) struct Sink {
    /// The file being scanned.
    pub file: String,
    /// The refusals so far.
    pub findings: Vec<Finding>,
    /// The template variables that supplied a URL attribute, so a macro
    /// parameter used as a URL is checked at each call site.
    pub url_vars: Vec<String>,
}

impl Sink {
    pub(crate) fn refuse(&mut self, check: &'static str, line: u32, message: String) {
        let finding = Finding {
            check,
            file: self.file.clone(),
            line: Some(line),
            message,
        };
        if !self.findings.contains(&finding) {
            self.findings.push(finding);
        }
    }
}

/// A template value written into the markup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Dynamic {
    /// Whether the value may stand as a URL: a bare state read or a call
    /// to a capability-free allowlist helper.
    pub url_source: bool,
    /// The template variable the value reads, when it reads one.
    pub var: Option<String>,
    /// The line of the expression.
    pub line: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Piece {
    Text(String),
    Dynamic(Dynamic),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Attribute {
    name: String,
    pieces: Vec<Piece>,
    line: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Tag {
    name: String,
    end: bool,
    self_closing: bool,
    attributes: Vec<Attribute>,
    line: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Data,
    TagOpen,
    EndTagOpen,
    TagName,
    BeforeAttributeName,
    AttributeName,
    AfterAttributeName,
    BeforeAttributeValue,
    DoubleQuoted,
    SingleQuoted,
    Unquoted,
    AfterQuoted,
    SelfClosing,
    MarkupDeclaration,
    Comment,
    Bogus,
    RawText,
}

/// The tokenizer's whole state. A template branch forks it, and the view
/// walker carries every distinct state forward, so each way the template
/// can render is tokenized as the browser would.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Markup {
    mode: Mode,
    tag: Option<Tag>,
    declaration: String,
    comment_tail: String,
    raw_element: String,
    raw_text: String,
    raw_line: u32,
    raw_dynamic: bool,
    foreign_depth: usize,
    line: u32,
}

impl Markup {
    /// A tokenizer at the start of a document or fragment.
    pub(crate) fn new() -> Self {
        Markup {
            mode: Mode::Data,
            tag: None,
            declaration: String::new(),
            comment_tail: String::new(),
            raw_element: String::new(),
            raw_text: String::new(),
            raw_line: 1,
            raw_dynamic: false,
            foreign_depth: 0,
            line: 1,
        }
    }

    /// Whether the tokenizer is inside an `svg` or `math` element, where the
    /// browser parses some elements differently.
    pub(crate) fn in_foreign_content(&self) -> bool {
        self.foreign_depth > 0
    }

    /// Whether the tokenizer is between nodes, where template output is
    /// text and an included fragment starts.
    pub(crate) fn in_data(&self) -> bool {
        self.mode == Mode::Data
    }

    /// Feeds literal template text, starting on `line`.
    pub(crate) fn text(&mut self, text: &str, line: u32, sink: &mut Sink) {
        self.line = line;
        let chars: Vec<char> = text.chars().collect();
        let mut index = 0;
        while index < chars.len() {
            let c = chars[index];
            index += 1;
            self.step(c, sink);
            if c == '\n' {
                self.line = self.line.saturating_add(1);
            }
        }
    }

    /// Feeds one template value.
    pub(crate) fn dynamic(&mut self, value: Dynamic, sink: &mut Sink) {
        match self.mode {
            Mode::Data | Mode::Comment | Mode::Bogus => {}
            Mode::DoubleQuoted | Mode::SingleQuoted => {
                if let Some(attribute) = self.current_attribute() {
                    attribute.pieces.push(Piece::Dynamic(value));
                }
            }
            Mode::Unquoted | Mode::BeforeAttributeValue => sink.refuse(
                "view-markup",
                value.line,
                "an expression in an unquoted attribute value can add attributes; quote the value".to_string(),
            ),
            Mode::RawText => {
                self.raw_dynamic = true;
                if self.raw_element == "style" {
                    sink.refuse(
                        "view-css",
                        value.line,
                        "an expression inside a `style` element writes CSS the scan cannot read".to_string(),
                    );
                }
            }
            _ => sink.refuse(
                "view-markup",
                value.line,
                "an expression inside a tag, outside an attribute value, writes markup the scan cannot read".to_string(),
            ),
        }
    }

    /// Reports a view that ends inside a tag, comment or raw-text element.
    pub(crate) fn finish(&mut self, sink: &mut Sink) {
        if self.mode == Mode::RawText {
            self.close_raw(sink);
            return;
        }
        if !matches!(self.mode, Mode::Data | Mode::Comment | Mode::Bogus) {
            sink.refuse(
                "view-markup",
                self.line,
                "the template ends inside a tag".to_string(),
            );
        }
    }

    fn current_attribute(&mut self) -> Option<&mut Attribute> {
        self.tag.as_mut().and_then(|tag| tag.attributes.last_mut())
    }

    fn start_attribute(&mut self, first: Option<char>) {
        let line = self.line;
        if let Some(tag) = self.tag.as_mut() {
            tag.attributes.push(Attribute {
                name: first
                    .map(|c| c.to_ascii_lowercase().to_string())
                    .unwrap_or_default(),
                pieces: Vec::new(),
                line,
            });
        }
    }

    fn push_value(&mut self, c: char) {
        if let Some(attribute) = self.current_attribute() {
            match attribute.pieces.last_mut() {
                Some(Piece::Text(text)) => text.push(c),
                _ => attribute.pieces.push(Piece::Text(c.to_string())),
            }
        }
    }

    fn step(&mut self, c: char, sink: &mut Sink) {
        let whitespace = matches!(c, '\t' | '\n' | '\u{c}' | ' ' | '\r');
        match self.mode {
            Mode::Data => {
                if c == '<' {
                    self.mode = Mode::TagOpen;
                }
            }
            Mode::TagOpen => match c {
                '!' => {
                    self.mode = Mode::MarkupDeclaration;
                    self.declaration.clear();
                }
                '/' => self.mode = Mode::EndTagOpen,
                '?' => self.mode = Mode::Bogus,
                c if c.is_ascii_alphabetic() => {
                    self.tag = Some(Tag {
                        name: c.to_ascii_lowercase().to_string(),
                        end: false,
                        self_closing: false,
                        attributes: Vec::new(),
                        line: self.line,
                    });
                    self.mode = Mode::TagName;
                }
                '<' => {}
                _ => self.mode = Mode::Data,
            },
            Mode::EndTagOpen => match c {
                c if c.is_ascii_alphabetic() => {
                    self.tag = Some(Tag {
                        name: c.to_ascii_lowercase().to_string(),
                        end: true,
                        self_closing: false,
                        attributes: Vec::new(),
                        line: self.line,
                    });
                    self.mode = Mode::TagName;
                }
                '>' => self.mode = Mode::Data,
                _ => self.mode = Mode::Bogus,
            },
            Mode::TagName => {
                if whitespace {
                    self.mode = Mode::BeforeAttributeName;
                } else if c == '/' {
                    self.mode = Mode::SelfClosing;
                } else if c == '>' {
                    self.emit(sink);
                } else if let Some(tag) = self.tag.as_mut() {
                    tag.name.push(c.to_ascii_lowercase());
                }
            }
            Mode::BeforeAttributeName => {
                if whitespace {
                } else if c == '/' {
                    self.mode = Mode::SelfClosing;
                } else if c == '>' {
                    self.emit(sink);
                } else {
                    self.start_attribute(Some(c));
                    self.mode = Mode::AttributeName;
                }
            }
            Mode::AttributeName => {
                if whitespace {
                    self.mode = Mode::AfterAttributeName;
                } else if c == '/' {
                    self.mode = Mode::SelfClosing;
                } else if c == '>' {
                    self.emit(sink);
                } else if c == '=' {
                    self.mode = Mode::BeforeAttributeValue;
                } else if let Some(attribute) = self.current_attribute() {
                    attribute.name.push(c.to_ascii_lowercase());
                }
            }
            Mode::AfterAttributeName => {
                if whitespace {
                } else if c == '/' {
                    self.mode = Mode::SelfClosing;
                } else if c == '=' {
                    self.mode = Mode::BeforeAttributeValue;
                } else if c == '>' {
                    self.emit(sink);
                } else {
                    self.start_attribute(Some(c));
                    self.mode = Mode::AttributeName;
                }
            }
            Mode::BeforeAttributeValue => {
                if whitespace {
                } else if c == '"' {
                    self.mode = Mode::DoubleQuoted;
                } else if c == '\'' {
                    self.mode = Mode::SingleQuoted;
                } else if c == '>' {
                    self.emit(sink);
                } else {
                    self.mode = Mode::Unquoted;
                    self.push_value(c);
                }
            }
            Mode::DoubleQuoted => {
                if c == '"' {
                    self.mode = Mode::AfterQuoted;
                } else {
                    self.push_value(c);
                }
            }
            Mode::SingleQuoted => {
                if c == '\'' {
                    self.mode = Mode::AfterQuoted;
                } else {
                    self.push_value(c);
                }
            }
            Mode::Unquoted => {
                if whitespace {
                    self.mode = Mode::BeforeAttributeName;
                } else if c == '>' {
                    self.emit(sink);
                } else {
                    self.push_value(c);
                }
            }
            Mode::AfterQuoted => {
                if whitespace {
                    self.mode = Mode::BeforeAttributeName;
                } else if c == '/' {
                    self.mode = Mode::SelfClosing;
                } else if c == '>' {
                    self.emit(sink);
                } else {
                    self.start_attribute(Some(c));
                    self.mode = Mode::AttributeName;
                }
            }
            Mode::SelfClosing => {
                if c == '>' {
                    if let Some(tag) = self.tag.as_mut() {
                        tag.self_closing = true;
                    }
                    self.emit(sink);
                } else {
                    self.mode = Mode::BeforeAttributeName;
                    self.step(c, sink);
                }
            }
            Mode::MarkupDeclaration => {
                self.declaration.push(c);
                if self.declaration == "--" {
                    self.mode = Mode::Comment;
                    self.comment_tail.clear();
                } else if !"--".starts_with(self.declaration.as_str()) {
                    self.mode = if c == '>' { Mode::Data } else { Mode::Bogus };
                }
            }
            Mode::Comment => {
                self.comment_tail.push(c);
                if self.comment_tail.len() > 4 {
                    let cut = self.comment_tail.len() - 4;
                    self.comment_tail.drain(..cut);
                }
                // `-->`, `--!>`, and the abrupt `<!-->` and `<!--->` all
                // close a comment.
                if self.comment_tail.ends_with("-->")
                    || self.comment_tail.ends_with("--!>")
                    || self.comment_tail == ">"
                    || self.comment_tail == "->"
                {
                    self.mode = Mode::Data;
                }
            }
            Mode::Bogus => {
                if c == '>' {
                    self.mode = Mode::Data;
                }
            }
            Mode::RawText => {
                self.raw_text.push(c);
                let closing = format!("</{}", self.raw_element);
                if c == '>' || c == '/' || whitespace {
                    let before = &self.raw_text[..self.raw_text.len() - c.len_utf8()];
                    if before.to_ascii_lowercase().ends_with(&closing) {
                        let content_len = before.len() - closing.len();
                        self.raw_text.truncate(content_len);
                        self.close_raw(sink);
                        if c == '>' {
                            self.mode = Mode::Data;
                        } else {
                            self.tag = Some(Tag {
                                name: self.raw_element.clone(),
                                end: true,
                                self_closing: false,
                                attributes: Vec::new(),
                                line: self.line,
                            });
                            self.mode = if c == '/' {
                                Mode::SelfClosing
                            } else {
                                Mode::BeforeAttributeName
                            };
                        }
                    }
                }
            }
        }
    }

    fn close_raw(&mut self, sink: &mut Sink) {
        if self.raw_element == "style" && !self.raw_dynamic {
            css::check(&self.raw_text, self.raw_line, sink);
        }
        self.raw_text.clear();
        self.raw_dynamic = false;
        self.mode = Mode::Data;
    }

    fn emit(&mut self, sink: &mut Sink) {
        self.mode = Mode::Data;
        let Some(tag) = self.tag.take() else {
            return;
        };
        let foreign = matches!(tag.name.as_str(), "svg" | "math");
        if tag.end {
            if foreign {
                self.foreign_depth = self.foreign_depth.saturating_sub(1);
            }
            return;
        }
        check_tag(&tag, sink);
        if foreign && !tag.self_closing {
            self.foreign_depth += 1;
            return;
        }
        if self.foreign_depth > 0 {
            // Inside SVG or MathML a `style` or `title` element holds
            // markup, and an HTML element inside it breaks back out; the
            // scan would read it as text, so a `style` there is refused and
            // the others are read as markup.
            if tag.name == "style" {
                sink.refuse(
                    "view-element",
                    tag.line,
                    "a `<style>` inside `<svg>` or `<math>` is parsed as markup by the browser"
                        .to_string(),
                );
            }
            return;
        }
        if RAW_TEXT_ELEMENTS.contains(&tag.name.as_str()) {
            self.mode = Mode::RawText;
            self.raw_element = tag.name.clone();
            self.raw_text.clear();
            self.raw_dynamic = false;
            self.raw_line = self.line;
        }
    }
}

fn check_tag(tag: &Tag, sink: &mut Sink) {
    let local = tag.name.rsplit(':').next().unwrap_or(&tag.name);
    if REFUSED_ELEMENTS.contains(&tag.name.as_str()) || REFUSED_ELEMENTS.contains(&local) {
        sink.refuse(
            "view-element",
            tag.line,
            format!(
                "a `<{}>` element loads or runs what the scan cannot read",
                tag.name
            ),
        );
    }
    for attribute in &tag.attributes {
        check_attribute(tag, attribute, sink);
    }
}

fn check_attribute(tag: &Tag, attribute: &Attribute, sink: &mut Sink) {
    let name = attribute.name.as_str();
    if name.starts_with("on") {
        sink.refuse(
            "view-attribute",
            attribute.line,
            format!("`{name}` on `<{}>` is an event handler attribute", tag.name),
        );
        return;
    }
    if REFUSED_ATTRIBUTES.contains(&name) {
        sink.refuse(
            "view-attribute",
            attribute.line,
            format!("`{name}` on `<{}>` is refused", tag.name),
        );
        return;
    }
    if name == "style" {
        check_style_attribute(attribute, sink);
        return;
    }
    if URL_ATTRIBUTES.contains(&name) {
        check_url_attribute(tag, attribute, sink);
    }
}

fn texts_only(pieces: &[Piece]) -> Option<String> {
    let mut text = String::new();
    for piece in pieces {
        match piece {
            Piece::Text(part) => text.push_str(part),
            Piece::Dynamic(_) => return None,
        }
    }
    Some(text)
}

fn check_style_attribute(attribute: &Attribute, sink: &mut Sink) {
    match texts_only(&attribute.pieces) {
        Some(text) => match decode_references(&text) {
            Ok(decoded) => css::check(&decoded, attribute.line, sink),
            Err(reference) => sink.refuse(
                "view-css",
                attribute.line,
                format!("the character reference `&{reference};` in a `style` attribute is one the scan does not decode"),
            ),
        },
        None => sink.refuse(
            "view-css",
            attribute.line,
            "an expression inside a `style` attribute writes CSS the scan cannot read".to_string(),
        ),
    }
}

fn check_url_attribute(tag: &Tag, attribute: &Attribute, sink: &mut Sink) {
    let name = attribute.name.as_str();
    let refuse =
        |sink: &mut Sink, message: String| sink.refuse("view-url", attribute.line, message);
    let list = matches!(name, "srcset" | "imagesrcset" | "ping" | "archive");
    if let Some(text) = texts_only(&attribute.pieces) {
        let decoded = match decode_references(&text) {
            Ok(decoded) => decoded,
            Err(reference) => {
                refuse(
                    sink,
                    format!(
                        "`{name}` on `<{}>` holds the character reference `&{reference};`, which the scan does not decode",
                        tag.name
                    ),
                );
                return;
            }
        };
        let urls: Vec<&str> = if list {
            srcset_urls(&decoded)
        } else {
            vec![decoded.as_str()]
        };
        for url in urls {
            if let Err(refusal) = check_constant(url) {
                refuse(sink, url_message(name, &tag.name, refusal));
                return;
            }
        }
        return;
    }
    let mut prefix = String::new();
    let mut seen_dynamic = false;
    for piece in &attribute.pieces {
        match piece {
            Piece::Text(text) if !seen_dynamic => prefix.push_str(text),
            Piece::Text(_) => {}
            Piece::Dynamic(dynamic) => {
                seen_dynamic = true;
                if let Some(var) = &dynamic.var {
                    sink.url_vars.push(var.clone());
                }
                if !dynamic.url_source {
                    refuse(
                        sink,
                        format!(
                            "`{name}` on `<{}>` takes a value the component computes; a URL may come only from state or from a capability-free URL helper such as `route` or `url::to`",
                            tag.name
                        ),
                    );
                    return;
                }
            }
        }
    }
    let dynamics = attribute
        .pieces
        .iter()
        .filter(|piece| matches!(piece, Piece::Dynamic(_)))
        .count();
    let bare = prefix.trim().is_empty()
        && dynamics == 1
        && attribute.pieces.iter().all(|piece| {
            matches!(piece, Piece::Dynamic(_))
                || matches!(piece, Piece::Text(text) if text.trim().is_empty())
        });
    if bare {
        return;
    }
    let decoded = decode_references(&prefix).unwrap_or_default();
    if list || !prefix_commits_to_origin(&decoded) {
        refuse(
            sink,
            format!(
                "`{name}` on `<{}>` mixes constant text and values in a way that can leave the application's origin",
                tag.name
            ),
        );
    }
}

fn url_message(attribute: &str, element: &str, refusal: UrlRefusal) -> String {
    format!("`{attribute}` on `<{element}>`: {}", refusal.describe())
}

/// Decodes the character references an attribute value holds, as the HTML
/// parser does before the value reaches the URL parser. Numeric references
/// and the five basic named ones are decoded; any other named reference
/// ending in `;` is reported so the caller can refuse it, since the scan
/// would otherwise check a different string than the browser loads.
pub(crate) fn decode_references(text: &str) -> Result<String, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        if c != '&' {
            out.push(c);
            index += 1;
            continue;
        }
        let rest = &chars[index + 1..];
        if rest.first() == Some(&'#') {
            let hex = matches!(rest.get(1), Some('x' | 'X'));
            let start = if hex { 2 } else { 1 };
            let digits: String = rest[start..]
                .iter()
                .take_while(|d| {
                    if hex {
                        d.is_ascii_hexdigit()
                    } else {
                        d.is_ascii_digit()
                    }
                })
                .collect();
            if digits.is_empty() {
                out.push('&');
                index += 1;
                continue;
            }
            let value = u32::from_str_radix(&digits, if hex { 16 } else { 10 }).unwrap_or(0xFFFD);
            out.push(
                char::from_u32(value)
                    .filter(|c| *c != '\0')
                    .unwrap_or('\u{FFFD}'),
            );
            index += 1 + start + digits.len();
            if chars.get(index) == Some(&';') {
                index += 1;
            }
            continue;
        }
        let name: String = rest
            .iter()
            .take_while(|n| n.is_ascii_alphanumeric())
            .collect();
        if !name.is_empty() && rest.get(name.len()) == Some(&';') {
            let decoded = match name.as_str() {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                _ => return Err(name),
            };
            out.push(decoded);
            index += name.len() + 2;
            continue;
        }
        out.push('&');
        index += 1;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::decode_references;

    #[test]
    fn numeric_and_basic_references_decode() {
        assert_eq!(
            decode_references("&#106;avascript&#x3a;").as_deref(),
            Ok("javascript:")
        );
        assert_eq!(
            decode_references("&#106avascript:").as_deref(),
            Ok("javascript:")
        );
        assert_eq!(decode_references("a&amp;b=1&c").as_deref(), Ok("a&b=1&c"));
        assert_eq!(
            decode_references("javascript&colon;x"),
            Err("colon".to_string())
        );
    }
}
