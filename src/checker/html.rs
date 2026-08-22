//! Bounded html5ever tokenization and strict branch-state validation.

use std::cell::RefCell;
use std::collections::BTreeSet;

use html5ever::TokenizerResult;
use html5ever::tendril::SliceExt as _;
use html5ever::tokenizer::states::RawKind;
use html5ever::tokenizer::{BufferQueue, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer};

use crate::identity::ComponentName;
use crate::metadata::ComponentMetadata;
use crate::registry::ComponentRegistry;

use super::branch::{
    CHECKED_KEY_MARKER, DYNAMIC_MARKER, LOOP_END_MARKER, LOOP_START_MARKER, RenderedBranch,
};
use super::diagnostic::{DiagnosticCode, DiagnosticCollector, DiagnosticSeverity};
use super::directive::{DirectiveContext, validate_directive};
use super::limits::CheckerLimits;
use super::template::TemplateCatalog;

pub(crate) fn check_html_branches(
    branches: &[RenderedBranch],
    registry: &ComponentRegistry,
    catalog: &TemplateCatalog,
    root: &ComponentMetadata,
    limits: CheckerLimits,
    diagnostics: &mut DiagnosticCollector,
) {
    for branch in branches {
        let input = BufferQueue::default();
        input.push_back(branch.html.to_tendril());
        let sink = CheckerSink {
            state: RefCell::new(HtmlState::new(
                registry,
                catalog,
                root,
                branch,
                limits,
                diagnostics,
            )),
        };
        let tokenizer = Tokenizer::new(sink, Default::default());
        while tokenizer.feed(&input) != TokenizerResult::Done {}
        tokenizer.end();
        tokenizer.sink.state.into_inner().finish();
    }
}

struct ElementFrame {
    tag: String,
    owner: ComponentName,
}

struct HtmlState<'checker, 'diagnostics> {
    registry: &'checker ComponentRegistry,
    catalog: &'checker TemplateCatalog,
    root: &'checker ComponentMetadata,
    branch: &'checker RenderedBranch,
    limits: CheckerLimits,
    diagnostics: &'diagnostics mut DiagnosticCollector,
    stack: Vec<ElementFrame>,
    keys: BTreeSet<String>,
    tokens: usize,
    attributes: usize,
    loop_depth: usize,
    stopped: bool,
}

impl<'checker, 'diagnostics> HtmlState<'checker, 'diagnostics> {
    fn new(
        registry: &'checker ComponentRegistry,
        catalog: &'checker TemplateCatalog,
        root: &'checker ComponentMetadata,
        branch: &'checker RenderedBranch,
        limits: CheckerLimits,
        diagnostics: &'diagnostics mut DiagnosticCollector,
    ) -> Self {
        Self {
            registry,
            catalog,
            root,
            branch,
            limits,
            diagnostics,
            stack: Vec::new(),
            keys: BTreeSet::new(),
            tokens: 0,
            attributes: 0,
            loop_depth: 0,
            stopped: false,
        }
    }

    fn process(&mut self, token: Token, line: u64) -> TokenSinkResult<()> {
        if self.stopped {
            return TokenSinkResult::Continue;
        }
        self.tokens = self.tokens.saturating_add(1);
        if self.tokens > self.limits.max_html_tokens() {
            self.push(
                DiagnosticCode::HtmlTokenLimit,
                DiagnosticSeverity::Error,
                line,
                self.root.identity(),
            );
            self.stopped = true;
            return TokenSinkResult::Continue;
        }
        match token {
            Token::TagToken(tag) if tag.kind == TagKind::StartTag => {
                self.attributes = self.attributes.saturating_add(tag.attrs.len());
                if self.attributes > self.limits.max_attributes() {
                    let owner = self.current_owner_name();
                    self.push(
                        DiagnosticCode::AttributeLimit,
                        DiagnosticSeverity::Error,
                        line,
                        &owner,
                    );
                    self.stopped = true;
                    return TokenSinkResult::Continue;
                }
                let tag_name = tag.name.as_ref().to_ascii_lowercase();
                let attributes: Vec<(String, String)> = tag
                    .attrs
                    .iter()
                    .map(|attribute| {
                        (
                            attribute.name.local.as_ref().to_ascii_lowercase(),
                            attribute.value.as_ref().to_owned(),
                        )
                    })
                    .collect();
                if tag_name.contains(DYNAMIC_MARKER)
                    || attributes
                        .iter()
                        .any(|(name, _)| name.contains(DYNAMIC_MARKER))
                {
                    let owner = self.current_owner_name();
                    self.push(
                        DiagnosticCode::DynamicStructureUnproved,
                        DiagnosticSeverity::Unproved,
                        line,
                        &owner,
                    );
                }

                let prior_owner = self.current_owner_name();
                let mut owner = prior_owner.clone();
                if let Some((_, component)) =
                    attributes.iter().find(|(name, _)| name == "live:component")
                {
                    owner = self.resolve_component(component, &attributes, line, &prior_owner);
                }
                self.validate_keys(&attributes, line, &owner);
                let ancestors: Vec<ComponentName> = std::iter::once(self.root.identity().clone())
                    .chain(self.stack.iter().map(|frame| frame.owner.clone()))
                    .collect();
                let registry = self.registry;
                let owner_metadata = registry
                    .resolve(&owner)
                    .ok()
                    .map_or(self.root, |descriptor| descriptor.metadata());
                for (name, value) in &attributes {
                    if name.starts_with("live:")
                        && !matches!(name.as_str(), "live:component" | "live:key")
                    {
                        let mut context = DirectiveContext {
                            registry,
                            owner: owner_metadata,
                            ancestors: &ancestors,
                            tag: &tag_name,
                            attributes: &attributes,
                            path: &self.branch.path,
                            line: line_number(line),
                            diagnostics: &mut *self.diagnostics,
                        };
                        validate_directive(name, value, &mut context);
                    }
                }
                if !tag.self_closing && !void_element(&tag_name) {
                    if self.stack.len() >= self.limits.max_stack_depth() {
                        self.push(
                            DiagnosticCode::StackDepthLimit,
                            DiagnosticSeverity::Error,
                            line,
                            &owner,
                        );
                        self.stopped = true;
                    } else {
                        self.stack.push(ElementFrame {
                            tag: tag_name.clone(),
                            owner,
                        });
                    }
                }
                raw_text_transition(&tag_name)
            }
            Token::TagToken(tag) if tag.kind == TagKind::EndTag => {
                let name = tag.name.as_ref().to_ascii_lowercase();
                if void_element(&name) || self.stack.last().is_none_or(|frame| frame.tag != name) {
                    self.push_stack_error(line);
                } else {
                    self.stack.pop();
                }
                TokenSinkResult::Continue
            }
            Token::CommentToken(comment) if comment.as_ref() == LOOP_START_MARKER => {
                self.loop_depth = self.loop_depth.saturating_add(1);
                TokenSinkResult::Continue
            }
            Token::CommentToken(comment) if comment.as_ref() == LOOP_END_MARKER => {
                if self.loop_depth == 0 {
                    let owner = self.current_owner_name();
                    self.push(
                        DiagnosticCode::HtmlSyntax,
                        DiagnosticSeverity::Error,
                        line,
                        &owner,
                    );
                } else {
                    self.loop_depth -= 1;
                }
                TokenSinkResult::Continue
            }
            Token::NullCharacterToken | Token::ParseError(_) => {
                let owner = self.current_owner_name();
                self.push(
                    DiagnosticCode::HtmlSyntax,
                    DiagnosticSeverity::Error,
                    line,
                    &owner,
                );
                TokenSinkResult::Continue
            }
            _ => TokenSinkResult::Continue,
        }
    }

    fn current_owner_name(&self) -> ComponentName {
        self.stack
            .last()
            .map_or_else(|| self.root.identity().clone(), |frame| frame.owner.clone())
    }

    fn resolve_component(
        &mut self,
        value: &str,
        attributes: &[(String, String)],
        line: u64,
        fallback: &ComponentName,
    ) -> ComponentName {
        let Ok(component) = ComponentName::parse(value) else {
            self.push(
                DiagnosticCode::UnknownComponent,
                DiagnosticSeverity::Error,
                line,
                fallback,
            );
            return fallback.clone();
        };
        let Ok(descriptor) = self.registry.resolve(&component) else {
            self.push(
                DiagnosticCode::UnknownComponent,
                DiagnosticSeverity::Error,
                line,
                fallback,
            );
            return fallback.clone();
        };
        let metadata = descriptor.metadata();
        if !self.catalog.contains(metadata.view()) {
            self.push(
                DiagnosticCode::MissingView,
                DiagnosticSeverity::Error,
                line,
                metadata.identity(),
            );
        }
        if !attributes.iter().any(|(name, _)| name == "live:key") {
            self.push(
                DiagnosticCode::InvalidKey,
                DiagnosticSeverity::Error,
                line,
                metadata.identity(),
            );
        }
        metadata.identity().clone()
    }

    fn validate_keys(&mut self, attributes: &[(String, String)], line: u64, owner: &ComponentName) {
        for (_, key) in attributes.iter().filter(|(name, _)| name == "live:key") {
            let checked = key.contains(CHECKED_KEY_MARKER) && !key.contains(DYNAMIC_MARKER);
            let valid = !key.is_empty()
                && key.len() <= 128
                && !key.contains(DYNAMIC_MARKER)
                && (self.loop_depth == 0 || checked)
                && key.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':')
                });
            if !valid {
                self.push(
                    DiagnosticCode::InvalidKey,
                    DiagnosticSeverity::Error,
                    line,
                    owner,
                );
            } else if !checked && !self.keys.insert(key.clone()) {
                self.push(
                    DiagnosticCode::DuplicateKey,
                    DiagnosticSeverity::Error,
                    line,
                    owner,
                );
            }
        }
    }

    fn push_stack_error(&mut self, line: u64) {
        let owner = self.current_owner_name();
        self.push(
            if self.branch.branched {
                DiagnosticCode::BranchStackMismatch
            } else {
                DiagnosticCode::HtmlSyntax
            },
            DiagnosticSeverity::Error,
            line,
            &owner,
        );
    }

    fn finish(mut self) {
        if !self.stopped && (!self.stack.is_empty() || self.loop_depth != 0) {
            self.push_stack_error(1);
        }
    }

    fn push(
        &mut self,
        code: DiagnosticCode,
        severity: DiagnosticSeverity,
        line: u64,
        component: &ComponentName,
    ) {
        self.diagnostics.push(
            code,
            severity,
            Some(&self.branch.path),
            line_number(line),
            1,
            Some(component),
        );
    }
}

struct CheckerSink<'checker, 'diagnostics> {
    state: RefCell<HtmlState<'checker, 'diagnostics>>,
}

impl TokenSink for CheckerSink<'_, '_> {
    type Handle = ();

    fn process_token(&self, token: Token, line: u64) -> TokenSinkResult<Self::Handle> {
        self.state.borrow_mut().process(token, line)
    }
}

fn line_number(line: u64) -> u32 {
    u32::try_from(line.max(1)).unwrap_or(u32::MAX)
}

fn raw_text_transition(name: &str) -> TokenSinkResult<()> {
    match name {
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
        name,
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
