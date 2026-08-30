//! Island result authority and structural HTML boundary inspection.

use std::cell::RefCell;
use std::fmt;

use bytes::Bytes;
use html5ever::TokenizerResult;
use html5ever::tendril::SliceExt as _;
use html5ever::tokenizer::states::RawKind;
use html5ever::tokenizer::{BufferQueue, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer};

use crate::identity::IslandSlot;

use super::{AssetSet, ChildMount};

const ROOT_ATTRIBUTE: &str = "data-suprnova-live-root";

/// Successful island render with no route or transport response authority.
///
/// This type deliberately has no status, header, cookie, cache, redirect, or
/// media-type authority:
///
/// ```compile_fail
/// use suprnova_live::view::IslandRender;
/// fn forbidden_transport_access(render: &IslandRender) {
///     let _ = (
///         &render.status,
///         &render.headers,
///         &render.cookies,
///         &render.cache,
///         &render.media_type,
///     );
/// }
/// ```
#[derive(Clone)]
pub struct IslandRender {
    /// One complete server-rendered island boundary.
    pub body: Bytes,
    /// Deterministically ordered asset requirements.
    pub assets: AssetSet,
    /// Independently owned nested island declarations.
    pub children: Vec<ChildMount>,
}

impl fmt::Debug for IslandRender {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IslandRender")
            .field("body_bytes", &self.body.len())
            .field("assets", &self.assets)
            .field("child_count", &self.children.len())
            .finish()
    }
}

#[derive(Default)]
struct InspectionState {
    depth: usize,
    top_level_elements: usize,
    top_level_non_whitespace: bool,
    doctype_html: bool,
    html_element: bool,
    parse_error: bool,
    invalid_mount: bool,
    executable_mount: bool,
    roots: Vec<(IslandSlot, usize)>,
}

#[derive(Default)]
struct InspectionSink(RefCell<InspectionState>);

impl TokenSink for InspectionSink {
    type Handle = ();

    fn process_token(&self, token: Token, _: u64) -> TokenSinkResult<Self::Handle> {
        let mut state = self.0.borrow_mut();
        let mut result = TokenSinkResult::Continue;
        match token {
            Token::DoctypeToken(doctype) => {
                state.doctype_html = doctype
                    .name
                    .as_ref()
                    .is_some_and(|name| name.eq_ignore_ascii_case("html"));
            }
            Token::TagToken(tag) if tag.kind == TagKind::StartTag => {
                let depth = state.depth;
                if depth == 0 {
                    state.top_level_elements = state.top_level_elements.saturating_add(1);
                }
                if depth == 0 && tag.name.as_ref().eq_ignore_ascii_case("html") {
                    state.html_element = true;
                }
                let marker = tag
                    .attrs
                    .iter()
                    .find(|attribute| attribute.name.local.as_ref() == ROOT_ATTRIBUTE);
                if let Some(marker) = marker {
                    match IslandSlot::parse(marker.value.as_ref()) {
                        Ok(slot) => state.roots.push((slot, depth)),
                        Err(_) => state.invalid_mount = true,
                    }
                    if executable_element(tag.name.as_ref())
                        || tag.attrs.iter().any(|attribute| {
                            attribute
                                .name
                                .local
                                .as_ref()
                                .to_ascii_lowercase()
                                .starts_with("on")
                        })
                    {
                        state.executable_mount = true;
                    }
                }
                if !tag.self_closing && !void_element(tag.name.as_ref()) {
                    state.depth = state.depth.saturating_add(1);
                }
                result = raw_text_transition(tag.name.as_ref());
            }
            Token::TagToken(tag) if tag.kind == TagKind::EndTag => {
                if !void_element(tag.name.as_ref()) {
                    state.depth = state.depth.saturating_sub(1);
                }
            }
            Token::CharacterTokens(text) => {
                if state.depth == 0 && !text.chars().all(char::is_whitespace) {
                    state.top_level_non_whitespace = true;
                }
            }
            Token::NullCharacterToken | Token::ParseError(_) => state.parse_error = true,
            _ => {}
        }
        result
    }
}

pub(crate) struct HtmlInspection {
    pub(crate) top_level_elements: usize,
    pub(crate) top_level_non_whitespace: bool,
    pub(crate) complete_document: bool,
    pub(crate) parse_error: bool,
    pub(crate) invalid_mount: bool,
    pub(crate) executable_mount: bool,
    pub(crate) roots: Vec<(IslandSlot, usize)>,
}

pub(crate) fn inspect_html(body: &str) -> HtmlInspection {
    let input = BufferQueue::default();
    input.push_back(body.to_tendril());
    let tokenizer = Tokenizer::new(InspectionSink::default(), Default::default());
    while tokenizer.feed(&input) != TokenizerResult::Done {}
    tokenizer.end();
    let state = tokenizer.sink.0.into_inner();
    HtmlInspection {
        top_level_elements: state.top_level_elements,
        top_level_non_whitespace: state.top_level_non_whitespace,
        complete_document: state.doctype_html && state.html_element,
        parse_error: state.parse_error,
        invalid_mount: state.invalid_mount,
        executable_mount: state.executable_mount,
        roots: state.roots,
    }
}

fn executable_element(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "script" | "style" | "iframe" | "object" | "embed"
    )
}

fn raw_text_transition(name: &str) -> TokenSinkResult<()> {
    match name.to_ascii_lowercase().as_str() {
        "title" | "textarea" => TokenSinkResult::RawData(RawKind::Rcdata),
        "style" | "xmp" | "iframe" | "noembed" | "noframes" | "noscript" => {
            TokenSinkResult::RawData(RawKind::Rawtext)
        }
        "script" => TokenSinkResult::RawData(RawKind::ScriptData),
        "plaintext" => TokenSinkResult::Plaintext,
        _ => TokenSinkResult::Continue,
    }
}

fn void_element(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}
