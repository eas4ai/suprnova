//! Markdown to sanitized HTML rendering.

use std::fmt;
use std::sync::Mutex;

use ammonia::Builder as SanitizerBuilder;
use comrak::adapters::{HeadingAdapter, HeadingMeta};
use comrak::html::dangerous_url;
use comrak::nodes::{AstNode, NodeValue};
use comrak::options::Plugins;
use comrak::plugins::syntect::SyntectAdapter;
use comrak::{Arena, Options, format_html_with_plugins, parse_document};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::headings::{Heading, HeadingIdGenerator, prefixed_heading_id};

/// Result type returned by content rendering and docs build helpers.
pub type ContentResult<T> = Result<T, ContentError>;

/// Errors produced while rendering content or writing documentation artifacts.
#[derive(Debug, Error)]
pub enum ContentError {
    /// Filesystem read or write failed.
    #[error("content filesystem error: {0}")]
    Io(#[from] std::io::Error),
    /// JSON serialization failed.
    #[error("content serialization error: {0}")]
    Json(#[from] serde_json::Error),
    /// Two different chapter files in a docs table of contents map to the
    /// same slug, so one chapter's `<slug>.json` would overwrite the
    /// other's while the catalog still listed both.
    #[error(
        "docs chapters `{first}` and `{second}` both map to the slug `{slug}`; rename one of the files"
    )]
    DuplicateChapterSlug {
        /// The slug both chapters map to.
        slug: String,
        /// The chapter listed first, as the table of contents names it.
        first: String,
        /// The later chapter, as the table of contents names it.
        second: String,
    },
    /// A chapter's slug is the name of an artifact the docs build writes
    /// itself (`catalog`), which would overwrite the chapter.
    #[error(
        "docs chapter `{path}` maps to the slug `{slug}`, which the docs build reserves for its own `{slug}.json`; rename the file"
    )]
    ReservedChapterSlug {
        /// The reserved slug.
        slug: String,
        /// The chapter, as the table of contents names it.
        path: String,
    },
}

/// Options controlling Markdown rendering.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MarkdownOptions {
    /// Render raw HTML without sanitizing it.
    pub unsafe_html: bool,
    /// Prefix applied to generated heading anchor IDs.
    pub heading_anchor_prefix: String,
    /// Enable Comrak's math extensions and stable `language-math` code markers.
    pub render_math: bool,
}

impl Default for MarkdownOptions {
    fn default() -> Self {
        Self {
            unsafe_html: false,
            heading_anchor_prefix: String::new(),
            render_math: true,
        }
    }
}

/// Rendered Markdown plus metadata extracted from the parsed document.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RenderedMarkdown {
    /// Sanitized HTML ready to send to the browser.
    pub html: String,
    /// Plain text extracted from the Markdown AST.
    pub plain_text: String,
    /// Short preview text derived from `plain_text`.
    pub excerpt: String,
    /// Ordered heading metadata for table-of-contents UIs.
    pub headings: Vec<Heading>,
}

/// How a renderer treats raw HTML written in the Markdown, as the
/// `html_input` option of Laravel's CommonMark converter does.
///
/// A renderer that never calls [`MarkdownRenderer::html_input`] keeps
/// the treatment it has always had: when
/// [`MarkdownOptions::unsafe_html`] is off, the parser drops raw HTML and
/// the sanitizer cleans the rest of the output; when it is on, raw HTML
/// passes through.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HtmlInput {
    /// Keep raw HTML, then pass the whole output through the sanitizer,
    /// which keeps safe tags and attributes and removes scripts, event
    /// handlers and URLs with a scheme it does not allow. Use it to let
    /// users write some HTML, as Laravel's documentation advises running
    /// the output through an HTML purifier.
    Sanitize,
    /// Remove raw HTML. An inline tag goes and the text between an
    /// opening and a closing tag stays; an HTML block goes whole.
    Strip,
    /// Show raw HTML as text, escaped.
    Escape,
    /// Pass raw HTML through as written. Use it only for Markdown you
    /// trust, because a `<script>` in the text reaches the page.
    Allow,
}

/// Shared Markdown renderer for application pages, docs, and articles.
///
/// The builder calls [`html_input`](Self::html_input),
/// [`allow_unsafe_links`](Self::allow_unsafe_links) and
/// [`autolink`](Self::autolink) hold the choices Laravel passes to
/// CommonMark as options. They are fields of the renderer rather than of
/// [`MarkdownOptions`], whose fields are public, so code that builds
/// `MarkdownOptions` as a struct literal keeps compiling.
#[derive(Clone, Debug)]
pub struct MarkdownRenderer {
    options: MarkdownOptions,
    html_input: Option<HtmlInput>,
    allow_unsafe_links: Option<bool>,
    autolink: bool,
}

impl MarkdownRenderer {
    /// Create a renderer from explicit options.
    pub fn new(options: MarkdownOptions) -> Self {
        Self {
            options,
            html_input: None,
            allow_unsafe_links: None,
            autolink: false,
        }
    }

    /// Return the options used by this renderer.
    pub fn options(&self) -> &MarkdownOptions {
        &self.options
    }

    /// Choose how raw HTML in the Markdown is treated; see [`HtmlInput`].
    /// Without this call the renderer follows
    /// [`MarkdownOptions::unsafe_html`].
    pub fn html_input(mut self, mode: HtmlInput) -> Self {
        self.html_input = Some(mode);
        self
    }

    /// Choose whether a Markdown link or image may point at a
    /// `javascript:`, `vbscript:`, `file:` or `data:` URL (a `data:`
    /// image in PNG, GIF, JPEG or WebP is always allowed). A refused URL
    /// is rendered as an empty `href` or `src`. Without this call unsafe
    /// URLs are allowed only when [`MarkdownOptions::unsafe_html`] is on.
    ///
    /// The sanitizer removes unsafe URLs whatever this says, so `true`
    /// takes effect only with [`HtmlInput::Strip`], [`HtmlInput::Escape`]
    /// or [`HtmlInput::Allow`], or with `unsafe_html` on. URLs inside raw
    /// HTML follow [`html_input`](Self::html_input), not this call.
    pub fn allow_unsafe_links(mut self, allow: bool) -> Self {
        self.allow_unsafe_links = Some(allow);
        self
    }

    /// Turn bare `http://`, `https://` and `www.` URLs and email
    /// addresses into links, as GitHub-flavored Markdown does. Off by
    /// default, as it has always been here.
    pub fn autolink(mut self, on: bool) -> Self {
        self.autolink = on;
        self
    }

    /// Render a Markdown document to HTML and extracted metadata.
    pub fn render(&self, markdown: &str) -> ContentResult<RenderedMarkdown> {
        let policy = self.policy();
        let options = comrak_options(&self.options, &policy, self.autolink, Scope::Document);
        let arena = Arena::new();
        let root = parse_document(&arena, markdown, &options);
        apply_policy(root, &policy);
        let (headings, plain_text) = extract_metadata(root, &self.options.heading_anchor_prefix);
        let excerpt = excerpt_from_plain_text(&plain_text);

        let syntax_highlighter = SyntectAdapter::new(None);
        let heading_adapter = AnchorHeadingAdapter::new(self.options.heading_anchor_prefix.clone());
        let mut plugins = Plugins::default();
        plugins.render.codefence_syntax_highlighter = Some(&syntax_highlighter);
        plugins.render.heading_adapter = Some(&heading_adapter);

        let rendered = format_root(root, &options, &plugins)?;
        let html = if policy.sanitize {
            sanitize_html(&rendered)
        } else {
            rendered
        };

        Ok(RenderedMarkdown {
            html,
            plain_text,
            excerpt,
            headings,
        })
    }

    /// Render Markdown's inline syntax only (emphasis, code spans,
    /// links, images, raw inline HTML, line breaks), with no paragraph
    /// around it, as Laravel's `Str::inlineMarkdown` does through
    /// CommonMark's inlines-only extension.
    ///
    /// Comrak has no inlines-only mode, so this makes one. Leading spaces
    /// and tabs are removed from each line, as CommonMark removes them
    /// from a paragraph's lines, and each line then starts with a marker:
    /// a Unicode space the text does not hold, chosen from U+1680,
    /// U+2000 to U+200A, U+202F, U+205F and U+3000. A line that starts
    /// with a marker starts no block and is never blank, so the whole text
    /// parses as one paragraph and every block marker (`#`, `>`, `-`,
    /// `1.`, a fence, an indent, a setext underline) stays text. Because
    /// the marker is whitespace, emphasis at the start of a line parses
    /// as it does at a line start. The paragraph's children are rendered
    /// without the paragraph, and the markers are removed from the
    /// output. Tables, task lists, footnotes and front matter, which are
    /// blocks, are off. Lines stay separated by a line break, and a blank
    /// line between two lines stays in the output. In the unlikely case
    /// that the text holds every one of those spaces, it is rendered as
    /// escaped text with no Markdown.
    ///
    /// Raw HTML, unsafe links, autolinks and the sanitizer follow the
    /// same choices as [`render`](Self::render).
    pub fn render_inline(&self, markdown: &str) -> ContentResult<String> {
        let Some(marker) = INLINE_LINE_MARKERS
            .iter()
            .copied()
            .find(|candidate| !markdown.contains(*candidate))
        else {
            return Ok(escape_text(markdown));
        };

        let policy = self.policy();
        let options = comrak_options(&self.options, &policy, self.autolink, Scope::Inline);
        let arena = Arena::new();
        let marked = mark_lines(markdown, marker);
        let root = parse_document(&arena, &marked, &options);
        apply_policy(root, &policy);
        unwrap_paragraphs(root);

        let rendered = format_root(root, &options, &Plugins::default())?;
        let unmarked: String = rendered.chars().filter(|c| *c != marker).collect();
        Ok(if policy.sanitize {
            sanitize_html(&unmarked)
        } else {
            unmarked
        })
    }

    /// Resolve the options and the builder calls into what one render
    /// does.
    fn policy(&self) -> RenderPolicy {
        let unsafe_links = self.allow_unsafe_links.unwrap_or(self.options.unsafe_html);
        match self.html_input {
            // The output this renderer has always produced: the parser
            // drops raw HTML and refuses unsafe URLs, and the sanitizer
            // cleans what is left.
            None if !self.options.unsafe_html => RenderPolicy {
                parser_drops_raw_html: true,
                escape_raw_html: false,
                tagfilter: true,
                strip_raw_html: false,
                refuse_unsafe_links: false,
                sanitize: true,
            },
            None => RenderPolicy {
                parser_drops_raw_html: false,
                escape_raw_html: false,
                tagfilter: false,
                strip_raw_html: false,
                refuse_unsafe_links: !unsafe_links,
                sanitize: false,
            },
            Some(mode) => RenderPolicy {
                parser_drops_raw_html: false,
                escape_raw_html: mode == HtmlInput::Escape,
                tagfilter: mode == HtmlInput::Sanitize,
                strip_raw_html: mode == HtmlInput::Strip,
                refuse_unsafe_links: !unsafe_links,
                sanitize: mode == HtmlInput::Sanitize,
            },
        }
    }
}

impl Default for MarkdownRenderer {
    fn default() -> Self {
        Self::new(MarkdownOptions::default())
    }
}

/// The spaces [`MarkdownRenderer::render_inline`] can mark lines with.
/// Each is Unicode whitespace, so emphasis next to it parses as it does
/// at a line start, and each is more than one byte, so the block parser,
/// which reads only ASCII spaces and tabs as indentation, sees text.
const INLINE_LINE_MARKERS: [char; 15] = [
    '\u{3000}', '\u{2000}', '\u{2001}', '\u{2002}', '\u{2003}', '\u{2004}', '\u{2005}', '\u{2006}',
    '\u{2007}', '\u{2008}', '\u{2009}', '\u{200A}', '\u{202F}', '\u{205F}', '\u{1680}',
];

/// What one render does with raw HTML, links and the sanitizer.
struct RenderPolicy {
    /// Comrak's own safe mode: raw HTML becomes a comment the sanitizer
    /// removes, and unsafe URLs become empty.
    parser_drops_raw_html: bool,
    /// Comrak writes raw HTML escaped.
    escape_raw_html: bool,
    /// GitHub's tag filter, which disables a few dangerous tags.
    tagfilter: bool,
    /// Raw HTML nodes are removed from the tree before rendering.
    strip_raw_html: bool,
    /// Unsafe link and image URLs are emptied in the tree.
    refuse_unsafe_links: bool,
    /// The output passes through the sanitizer.
    sanitize: bool,
}

/// Whether the parser reads a whole document or the inline text of
/// [`MarkdownRenderer::render_inline`].
#[derive(Clone, Copy, Eq, PartialEq)]
enum Scope {
    Document,
    Inline,
}

fn comrak_options(
    render_options: &MarkdownOptions,
    policy: &RenderPolicy,
    autolink: bool,
    scope: Scope,
) -> Options<'static> {
    let mut options = Options::default();
    let document = scope == Scope::Document;
    options.extension.strikethrough = true;
    options.extension.table = document;
    options.extension.tasklist = document;
    options.extension.footnotes = document;
    options.extension.front_matter_delimiter = document.then(|| "---".to_owned());
    options.extension.autolink = autolink;
    options.extension.tagfilter = policy.tagfilter;
    options.extension.math_code = render_options.render_math;
    options.extension.math_dollars = render_options.render_math;
    options.render.r#unsafe = !policy.parser_drops_raw_html;
    options.render.escape = policy.escape_raw_html;
    options
}

/// Remove raw HTML and empty unsafe URLs in the parsed tree, as the
/// policy asks.
fn apply_policy<'a>(root: &'a AstNode<'a>, policy: &RenderPolicy) {
    if !policy.strip_raw_html && !policy.refuse_unsafe_links {
        return;
    }
    let nodes: Vec<&'a AstNode<'a>> = root.descendants().collect();
    for node in nodes {
        let is_raw_html = matches!(
            node.data.borrow().value,
            NodeValue::HtmlBlock(_) | NodeValue::HtmlInline(_)
        );
        if is_raw_html {
            if policy.strip_raw_html {
                node.detach();
            }
            continue;
        }
        if policy.refuse_unsafe_links
            && let NodeValue::Link(link) | NodeValue::Image(link) =
                &mut node.data.borrow_mut().value
            && dangerous_url(&link.url)
        {
            link.url.clear();
        }
    }
}

/// Move each paragraph's children up to the document and remove the
/// paragraph, so inline content renders with no `<p>` around it. A line
/// break stands between two paragraphs, though marked lines make one.
fn unwrap_paragraphs<'a>(root: &'a AstNode<'a>) {
    let blocks: Vec<&'a AstNode<'a>> = root.children().collect();
    for block in blocks {
        if !matches!(block.data.borrow().value, NodeValue::Paragraph) {
            continue;
        }
        let inlines: Vec<&'a AstNode<'a>> = block.children().collect();
        for inline in inlines {
            block.insert_before(inline);
        }
        block.detach();
    }
}

/// Start each line of `markdown` with `marker`, after removing the
/// line's leading spaces and tabs. A line ends at `\n`, `\r\n` or `\r`,
/// as CommonMark's lines do, and a line ending at the very end of the
/// text starts no new line.
fn mark_lines(markdown: &str, marker: char) -> String {
    let mut marked = String::with_capacity(markdown.len() + 16);
    let mut rest = markdown;
    loop {
        let line = rest.trim_start_matches([' ', '\t']);
        marked.push(marker);
        let Some(at) = line.find(['\n', '\r']) else {
            marked.push_str(line);
            return marked;
        };
        let ending = if line[at..].starts_with("\r\n") { 2 } else { 1 };
        marked.push_str(&line[..at + ending]);
        rest = &line[at + ending..];
        if rest.is_empty() {
            return marked;
        }
    }
}

/// `text` with the characters HTML gives meaning to escaped.
fn escape_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Write the parsed tree as HTML. A write into a `String` fails only when
/// a render plugin fails.
fn format_root<'a>(
    root: &'a AstNode<'a>,
    options: &Options<'_>,
    plugins: &Plugins<'_>,
) -> ContentResult<String> {
    let mut out = String::new();
    format_html_with_plugins(root, options, &mut out, plugins).map_err(|_| {
        ContentError::Io(std::io::Error::other(
            "writing the rendered Markdown as HTML failed",
        ))
    })?;
    Ok(out)
}

fn sanitize_html(html: &str) -> String {
    let mut sanitizer = SanitizerBuilder::default();
    sanitizer.add_generic_attributes(&[
        "aria-hidden",
        "aria-label",
        "checked",
        "class",
        "data-footnote-backref",
        "data-footnote-backref-idx",
        "data-footnote-ref",
        "data-footnotes",
        "data-math-style",
        "disabled",
        "id",
        "type",
    ]);
    sanitizer.clean(html).to_string()
}

fn extract_metadata<'a>(root: &'a AstNode<'a>, prefix: &str) -> (Vec<Heading>, String) {
    let mut headings = Vec::new();
    let mut heading_ids = HeadingIdGenerator::new();
    let mut plain_text = String::new();
    walk_metadata(
        root,
        prefix,
        &mut heading_ids,
        &mut headings,
        &mut plain_text,
    );
    (headings, normalize_whitespace(&plain_text))
}

fn walk_metadata<'a>(
    node: &'a AstNode<'a>,
    prefix: &str,
    heading_ids: &mut HeadingIdGenerator,
    headings: &mut Vec<Heading>,
    plain_text: &mut String,
) {
    {
        let data = node.data.borrow();
        match &data.value {
            NodeValue::Heading(heading) => {
                let title = normalize_whitespace(&collect_visible_text(node));
                let id = prefixed_heading_id(prefix, heading_ids, &title);
                headings.push(Heading {
                    level: heading.level,
                    id,
                    title,
                });
            }
            NodeValue::Text(text) => append_plain_text(plain_text, text),
            NodeValue::Code(code) => append_plain_text(plain_text, &code.literal),
            NodeValue::CodeBlock(code) => append_plain_text(plain_text, &code.literal),
            NodeValue::SoftBreak | NodeValue::LineBreak => append_space(plain_text),
            _ => {}
        }
    }

    for child in node.children() {
        walk_metadata(child, prefix, heading_ids, headings, plain_text);
    }
}

fn collect_visible_text<'a>(node: &'a AstNode<'a>) -> String {
    let mut out = String::new();
    collect_visible_text_into(node, &mut out);
    out
}

fn collect_visible_text_into<'a>(node: &'a AstNode<'a>, out: &mut String) {
    {
        let data = node.data.borrow();
        match &data.value {
            NodeValue::Text(text) => append_plain_text(out, text),
            NodeValue::Code(code) => append_plain_text(out, &code.literal),
            NodeValue::SoftBreak | NodeValue::LineBreak => append_space(out),
            _ => {}
        }
    }

    for child in node.children() {
        collect_visible_text_into(child, out);
    }
}

fn append_plain_text(out: &mut String, text: &str) {
    if text.is_empty() {
        return;
    }

    // A SoftBreak/LineBreak before a node whose text opens with closing
    // punctuation (`.`, `,`, etc.) leaves a single break-introduced space
    // sitting in front of that punctuation - e.g. `Hello\n. World` walks to
    // `"Hello . World"`. Drop only that spurious break space; intentional
    // spaced punctuation inside a single Text run (French `Bonjour : ...`)
    // is left untouched because it never crosses a break boundary.
    if starts_with_closing_punctuation(text) && out.ends_with(' ') {
        out.pop();
    }

    if should_insert_space(out, text) {
        out.push(' ');
    }
    out.push_str(text);
}

/// Closing punctuation that should hug the preceding word rather than be
/// pushed onto a new line by a Markdown SoftBreak/LineBreak.
fn starts_with_closing_punctuation(text: &str) -> bool {
    text.chars()
        .next()
        .is_some_and(|ch| matches!(ch, '.' | ',' | ':' | ';' | '!' | '?' | ')' | ']'))
}

fn should_insert_space(out: &str, text: &str) -> bool {
    if out.is_empty() || out.ends_with(char::is_whitespace) {
        return false;
    }

    !starts_with_closing_punctuation(text)
}

fn append_space(out: &mut String) {
    if !out.is_empty() && !out.ends_with(char::is_whitespace) {
        out.push(' ');
    }
}

fn normalize_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn excerpt_from_plain_text(plain_text: &str) -> String {
    const MAX_EXCERPT_CHARS: usize = 240;

    if plain_text.chars().count() <= MAX_EXCERPT_CHARS {
        return plain_text.to_owned();
    }

    let mut excerpt = plain_text
        .chars()
        .take(MAX_EXCERPT_CHARS)
        .collect::<String>()
        .trim_end()
        .to_owned();
    excerpt.push_str("...");
    excerpt
}

struct AnchorHeadingAdapter {
    prefix: String,
    heading_ids: Mutex<HeadingIdGenerator>,
}

impl AnchorHeadingAdapter {
    fn new(prefix: String) -> Self {
        Self {
            prefix,
            heading_ids: Mutex::new(HeadingIdGenerator::new()),
        }
    }
}

impl HeadingAdapter for AnchorHeadingAdapter {
    fn enter(
        &self,
        output: &mut dyn fmt::Write,
        heading: &HeadingMeta,
        _sourcepos: Option<comrak::nodes::Sourcepos>,
    ) -> fmt::Result {
        let mut heading_ids = self.heading_ids.lock().map_err(|_| fmt::Error)?;
        let id = prefixed_heading_id(&self.prefix, &mut heading_ids, &heading.content);
        write!(
            output,
            "<h{} id=\"{}\">",
            heading.level,
            escape_attribute(&id)
        )
    }

    fn exit(&self, output: &mut dyn fmt::Write, heading: &HeadingMeta) -> fmt::Result {
        write!(output, "</h{}>", heading.level)
    }
}

fn escape_attribute(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn break_before_punctuation_drops_only_the_spurious_space() {
        // Simulate the AST walk for `Hello\n. World`: Text, SoftBreak, Text.
        // The break introduces a space that would otherwise sit in front of
        // the leading `.`.
        let mut out = String::new();
        append_plain_text(&mut out, "Hello");
        append_space(&mut out); // SoftBreak / LineBreak
        append_plain_text(&mut out, ". World");
        assert_eq!(out, "Hello. World");
    }

    #[test]
    fn intentional_spaced_punctuation_in_a_single_run_is_preserved() {
        // French typography keeps a space before `:` and `?`. When the text
        // arrives as a single run (no break boundary), the spacing must
        // survive - the old global ` :` -> `:` replace corrupted it.
        let mut out = String::new();
        append_plain_text(&mut out, "Bonjour : comment ça va ?");
        assert_eq!(out, "Bonjour : comment ça va ?");
        // normalize_whitespace only collapses runs; it must not touch the
        // intentional single spaces around the punctuation.
        assert_eq!(
            normalize_whitespace(&out),
            "Bonjour : comment ça va ?",
            "spaced punctuation in body text must not be glued to its word"
        );
    }

    #[test]
    fn spaced_punctuation_survives_full_render_pipeline() {
        // End-to-end through the renderer: intentional spaced punctuation in
        // the body must reach plain_text and excerpt untouched.
        let out = MarkdownRenderer::default()
            .render("Bonjour : comment ça va ?")
            .expect("render");
        assert_eq!(out.plain_text, "Bonjour : comment ça va ?");
        assert_eq!(out.excerpt, "Bonjour : comment ça va ?");
    }

    #[test]
    fn closing_brackets_after_break_also_hug_the_word() {
        // `)` and `]` are in the closing set too, so a break before them is
        // collapsed the same way.
        let mut out = String::new();
        append_plain_text(&mut out, "see footnote");
        append_space(&mut out);
        append_plain_text(&mut out, ") done");
        assert_eq!(out, "see footnote) done");
    }

    #[test]
    fn normalize_whitespace_only_collapses_runs() {
        assert_eq!(
            normalize_whitespace("  a \t b\n c  "),
            "a b c",
            "whitespace runs collapse to single spaces; nothing else changes"
        );
    }
}
