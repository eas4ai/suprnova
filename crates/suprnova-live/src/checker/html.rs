//! Compositional HTML checking of a rendered view.
//!
//! The checker walks the rendered tree with a small set of path states. A
//! state summarizes one way through the template: its open elements, the
//! facts later checks read (keys, element ids, freshness, field declarations,
//! submit forms, teleports), and any text not yet tokenized. Each arm of a
//! choice continues every state, and afterwards the states that leave the
//! same element structure merge into one, so independent conditionals add to
//! the work instead of multiplying it. Merging unions the facts, which is
//! exact for the checks that read them: two arms of one choice never render
//! together, and arms of different choices can always render together.
//!
//! Text is tokenized when a state reaches a point where html5ever is between
//! tokens. Inside one tag, or one raw-text element such as `script`, there is
//! no such point, so arms that differ there stay separate states until that
//! tag or element ends: that is where combinations are enumerated. A teleport,
//! whose target must exist on its own path, keeps states that differ in their
//! teleports apart for the rest of the view.

use std::cell::{Cell, RefCell};
use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use html5ever::TokenizerResult;
use html5ever::tendril::SliceExt as _;
use html5ever::tokenizer::states::RawKind;
use html5ever::tokenizer::{
    BufferQueue, Tag, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
};

use crate::identity::{ComponentName, ModelField, ViewName};
use crate::metadata::ComponentMetadata;
use crate::registry::ComponentRegistry;
use crate::view::{MAX_KEY_BYTES, in_key_alphabet};

use super::branch::{
    CHECKED_DIGEST_MARKER, CHECKED_KEY_MARKER, DYNAMIC_MARKER, Fragment, LOOP_END_MARKER,
    LOOP_START_MARKER, Origin, Piece, RenderedView, SourceFile, location,
};
use super::diagnostic::{DiagnosticCode, DiagnosticCollector, DiagnosticSeverity};
use super::directive::{
    DirectiveContext, MorphControlKind, morph_control_kind, valid_freshness_combination,
    validate_directive,
};
use super::limits::CheckerLimits;
use super::template::TemplateCatalog;

/// The most model fields one `live:submit` form may bind: a Live request
/// carries 128 operations, one per proposed field plus the invoked action
/// (LIVE-029, matching the framework's `ProtocolLimits`).
const MAX_SUBMIT_FORM_FIELDS: usize = 127;

/// A start tag no template renders, fed after a state's pending text: when
/// html5ever emits it as a tag, the text ended between tokens.
const PROBE_TAG: &str = "suprnova-checker-probe-7f3e";
const PROBE: &str = "<suprnova-checker-probe-7f3e>";

/// The island every view renders into before any `live:component`.
const ROOT_ISLAND: usize = 0;

/// A place in a template the checker read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Position {
    file: u32,
    offset: u32,
}

/// The start of the root view, for a failure no element owns.
const VIEW_START: Position = Position { file: 0, offset: 0 };

pub(crate) fn check_rendered_view(
    rendered: &RenderedView<'_>,
    registry: &ComponentRegistry,
    catalog: &TemplateCatalog,
    root: &ComponentMetadata,
    limits: CheckerLimits,
    diagnostics: &mut DiagnosticCollector,
) {
    let mut check = ViewCheck {
        registry,
        catalog,
        root,
        limits,
        diagnostics,
        files: &rendered.files,
        branched: rendered.branched,
        next_island: ROOT_ISLAND + 1,
        next_form: 0,
        branch_limit_reported: false,
    };
    let start = PathState {
        pending: Vec::new(),
        failed_at: 0,
        facts: Rc::new(HtmlFacts::new(root.identity())),
    };
    let states = check.walk(&rendered.fragment, vec![start]);
    for mut state in states {
        check.settle(&mut state, true);
        check.finish(&state.facts);
    }
}

#[derive(Clone)]
struct ElementFrame {
    tag: String,
    owner: ComponentName,
    island: usize,
    morph_control: Option<MorphControlKind>,
    submit_form: Option<usize>,
}

impl ElementFrame {
    fn same_structure(&self, other: &Self) -> bool {
        self.tag == other.tag
            && self.owner == other.owner
            && self.island == other.island
            && self.morph_control == other.morph_control
            && self.submit_form == other.submit_form
    }
}

/// The distinct model fields under one `live:submit` form, and whether the
/// form has already been reported past the bound.
#[derive(Clone, Default)]
struct SubmitForm {
    fields: BTreeSet<String>,
    reported: bool,
}

#[derive(Clone)]
struct TeleportIntent {
    target: String,
    owner: ComponentName,
    position: Position,
}

/// How many elements carry one id on the paths a state stands for, and who
/// owns them. One path has one count; merged paths keep the range.
#[derive(Clone, Default)]
struct IdCount {
    fewest: usize,
    most: usize,
    owners: BTreeSet<ComponentName>,
}

/// An island's possible freshness directives: each `(poll, stream)` pair a
/// path can reach, and where it was last set.
#[derive(Clone)]
struct IslandFreshness {
    owner: ComponentName,
    states: BTreeMap<(bool, &'static str), Position>,
}

impl IslandFreshness {
    fn new(owner: ComponentName, position: Position) -> Self {
        Self {
            owner,
            states: BTreeMap::from([((false, "absent"), position)]),
        }
    }
}

#[derive(Clone, Default)]
struct IslandFieldDeclarations {
    uploads: BTreeSet<ModelField>,
    models: BTreeSet<ModelField>,
}

/// Everything later checks read about the markup one state has seen.
#[derive(Clone)]
struct HtmlFacts {
    stack: Vec<ElementFrame>,
    keys: BTreeSet<String>,
    element_ids: BTreeSet<(usize, String)>,
    ids: BTreeMap<String, IdCount>,
    teleports: Vec<TeleportIntent>,
    freshness: BTreeMap<usize, IslandFreshness>,
    field_declarations: BTreeMap<usize, IslandFieldDeclarations>,
    submit_forms: BTreeMap<usize, SubmitForm>,
    tokens: usize,
    attributes: usize,
    loop_depth: usize,
    stopped: bool,
}

impl HtmlFacts {
    fn new(root: &ComponentName) -> Self {
        Self {
            stack: Vec::new(),
            keys: BTreeSet::new(),
            element_ids: BTreeSet::new(),
            ids: BTreeMap::new(),
            teleports: Vec::new(),
            freshness: BTreeMap::from([(
                ROOT_ISLAND,
                IslandFreshness::new(root.clone(), VIEW_START),
            )]),
            field_declarations: BTreeMap::from([(ROOT_ISLAND, IslandFieldDeclarations::default())]),
            submit_forms: BTreeMap::new(),
            tokens: 0,
            attributes: 0,
            loop_depth: 0,
            stopped: false,
        }
    }

    /// Whether the rest of the view checks the same on both: the same open
    /// elements, loop depth, and teleports. Everything else merges.
    fn same_structure(&self, other: &Self) -> bool {
        self.stopped == other.stopped
            && self.loop_depth == other.loop_depth
            && self.stack.len() == other.stack.len()
            && self
                .stack
                .iter()
                .zip(&other.stack)
                .all(|(left, right)| left.same_structure(right))
            && self.teleports.len() == other.teleports.len()
            && self
                .teleports
                .iter()
                .zip(&other.teleports)
                .all(|(left, right)| left.target == right.target && left.owner == right.owner)
    }

    /// Folds another path's facts into these. A key or id either path
    /// holds is one a later element can repeat on some path; counts keep
    /// their range so a teleport target missing on one path is still seen.
    fn merge(&mut self, other: &Self) {
        self.keys.extend(other.keys.iter().cloned());
        self.element_ids.extend(other.element_ids.iter().cloned());
        let names: BTreeSet<String> = self.ids.keys().chain(other.ids.keys()).cloned().collect();
        for name in names {
            let left = self.ids.get(&name).cloned().unwrap_or_default();
            let right = other.ids.get(&name).cloned().unwrap_or_default();
            self.ids.insert(
                name,
                IdCount {
                    fewest: left.fewest.min(right.fewest),
                    most: left.most.max(right.most),
                    owners: left.owners.union(&right.owners).cloned().collect(),
                },
            );
        }
        for (island, freshness) in &other.freshness {
            match self.freshness.entry(*island) {
                Entry::Vacant(entry) => {
                    entry.insert(freshness.clone());
                }
                Entry::Occupied(mut entry) => {
                    for (state, position) in &freshness.states {
                        entry.get_mut().states.entry(*state).or_insert(*position);
                    }
                }
            }
        }
        for (island, declarations) in &other.field_declarations {
            let merged = self.field_declarations.entry(*island).or_default();
            merged.uploads.extend(declarations.uploads.iter().cloned());
            merged.models.extend(declarations.models.iter().cloned());
        }
        for (form, fields) in &other.submit_forms {
            let merged = self.submit_forms.entry(*form).or_default();
            merged.fields.extend(fields.fields.iter().cloned());
            merged.reported |= fields.reported;
        }
        self.tokens = self.tokens.max(other.tokens);
        self.attributes = self.attributes.max(other.attributes);
    }

    fn current_owner(&self, root: &ComponentName) -> ComponentName {
        self.stack
            .last()
            .map_or_else(|| root.clone(), |frame| frame.owner.clone())
    }

    fn current_island(&self) -> usize {
        self.stack.last().map_or(ROOT_ISLAND, |frame| frame.island)
    }
}

/// One way through the view: the text not yet tokenized, and the facts of
/// everything before it. Facts are shared until a state writes them, so
/// states that only differ in pending text cost little.
#[derive(Clone)]
struct PathState<'t> {
    pending: Vec<(&'t str, Origin)>,
    /// The pending length when tokenizing it last failed to end between
    /// tokens, or zero. Text without a `>` cannot end a tag, comment, or
    /// raw-text element, so a retry waits for one.
    failed_at: usize,
    facts: Rc<HtmlFacts>,
}

impl PathState<'_> {
    fn same_structure(&self, other: &Self) -> bool {
        pending_bytes(&self.pending).eq(pending_bytes(&other.pending))
            && self.facts.same_structure(&other.facts)
    }

    fn absorb(&mut self, other: &Self) {
        if !Rc::ptr_eq(&self.facts, &other.facts) {
            Rc::make_mut(&mut self.facts).merge(&other.facts);
        }
        self.failed_at = self.failed_at.min(other.failed_at);
    }
}

fn pending_bytes<'p>(pending: &'p [(&str, Origin)]) -> impl Iterator<Item = u8> + 'p {
    pending.iter().flat_map(|(text, _)| text.bytes())
}

struct ViewCheck<'v, 'checker, 'diagnostics> {
    registry: &'checker ComponentRegistry,
    catalog: &'checker TemplateCatalog,
    root: &'checker ComponentMetadata,
    limits: CheckerLimits,
    diagnostics: &'diagnostics mut DiagnosticCollector,
    files: &'v [SourceFile<'v>],
    branched: bool,
    next_island: usize,
    next_form: usize,
    branch_limit_reported: bool,
}

impl ViewCheck<'_, '_, '_> {
    /// Continues every state through a fragment. A choice settles each
    /// state, walks every arm from it, and admits what comes out, merging
    /// states that check the same from there on.
    fn walk<'t>(
        &mut self,
        fragment: &'t Fragment<'_>,
        mut states: Vec<PathState<'t>>,
    ) -> Vec<PathState<'t>> {
        for piece in &fragment.pieces {
            if states.is_empty() {
                break;
            }
            match piece {
                Piece::Text(text, origin) => {
                    for state in &mut states {
                        state.pending.push((text.as_ref(), *origin));
                    }
                }
                Piece::Choice(choice) => {
                    let mut next = Vec::new();
                    for mut state in states {
                        self.settle(&mut state, false);
                        for arm in &choice.arms {
                            for result in self.walk(arm, vec![state.clone()]) {
                                self.admit(&mut next, result, choice.origin);
                            }
                        }
                    }
                    states = next;
                }
            }
        }
        states
    }

    fn admit<'t>(
        &mut self,
        states: &mut Vec<PathState<'t>>,
        mut state: PathState<'t>,
        origin: Origin,
    ) {
        self.settle(&mut state, false);
        if let Some(existing) = states
            .iter_mut()
            .find(|existing| existing.same_structure(&state))
        {
            existing.absorb(&state);
            return;
        }
        if states.len() >= self.limits.max_branch_states() {
            if !self.branch_limit_reported {
                self.branch_limit_reported = true;
                let position = Position {
                    file: origin.file,
                    offset: origin.offset,
                };
                let root = self.root.identity().clone();
                self.push_located(
                    DiagnosticCode::BranchLimit,
                    DiagnosticSeverity::Error,
                    position,
                    &root,
                );
            }
            return;
        }
        states.push(state);
    }

    /// Tokenizes and checks as much of a state's pending text as ends
    /// between tokens: all of it at the end of the view or when it ends
    /// there, or else everything before the tag it ends inside. A choice
    /// usually falls inside a tag, between one control's attributes and the
    /// next's, so checking up to that tag's start is what lets states that
    /// differ only in an earlier tag merge.
    fn settle(&mut self, state: &mut PathState<'_>, at_end: bool) {
        if state.facts.stopped {
            state.pending.clear();
            state.failed_at = 0;
            return;
        }
        if state.pending.is_empty() {
            return;
        }
        let text: String = state.pending.iter().map(|(text, _)| *text).collect();
        if !at_end
            && state.failed_at > 0
            && !text
                .get(state.failed_at..)
                .is_some_and(|tail| tail.contains('>'))
        {
            return;
        }
        if let Some(tokens) = tokenize(&text, at_end) {
            self.commit(state, tokens, text.len());
            return;
        }
        // The text ends inside a tag, a comment, or a raw-text element. The
        // incomplete part starts at a `<`; the latest one or two are tried,
        // each proved by the probe, so raw text with many `<` stays cheap.
        let settled = chunk_starts(&text)
            .into_iter()
            .rev()
            .filter(|start| *start > 0)
            .take(2)
            .find_map(|start| {
                tokenize(text.get(..start).unwrap_or_default(), false).map(|tokens| (start, tokens))
            });
        if let Some((start, tokens)) = settled {
            self.commit(state, tokens, start);
        }
        state.failed_at = pending_bytes(&state.pending).count();
    }

    /// Checks the tokens of the first `length` bytes of pending text and
    /// keeps the rest pending.
    fn commit(&mut self, state: &mut PathState<'_>, tokens: Vec<(Token, usize)>, length: usize) {
        let map = SourceMap::new(&state.pending);
        let facts = Rc::make_mut(&mut state.facts);
        for (token, offset) in tokens {
            self.process(facts, token, map.position(offset));
        }
        state.pending = split_pending(&state.pending, length);
        state.failed_at = 0;
    }

    fn process(&mut self, facts: &mut HtmlFacts, token: Token, position: Position) {
        if facts.stopped {
            return;
        }
        facts.tokens = facts.tokens.saturating_add(1);
        if facts.tokens > self.limits.max_html_tokens() {
            let root = self.root.identity().clone();
            self.push(
                DiagnosticCode::HtmlTokenLimit,
                DiagnosticSeverity::Error,
                position,
                &root,
            );
            facts.stopped = true;
            return;
        }
        match token {
            Token::TagToken(tag) if tag.kind == TagKind::StartTag => {
                self.start_tag(facts, &tag, position);
            }
            Token::TagToken(tag) if tag.kind == TagKind::EndTag => {
                let name = tag.name.as_ref().to_ascii_lowercase();
                if void_element(&name) || facts.stack.last().is_none_or(|frame| frame.tag != name) {
                    self.push_stack_error(facts, position);
                } else {
                    facts.stack.pop();
                }
            }
            Token::CommentToken(comment) if comment.as_ref() == LOOP_START_MARKER => {
                facts.loop_depth = facts.loop_depth.saturating_add(1);
            }
            Token::CommentToken(comment) if comment.as_ref() == LOOP_END_MARKER => {
                if facts.loop_depth == 0 {
                    let owner = facts.current_owner(self.root.identity());
                    self.push(
                        DiagnosticCode::HtmlSyntax,
                        DiagnosticSeverity::Error,
                        position,
                        &owner,
                    );
                } else {
                    facts.loop_depth -= 1;
                }
            }
            Token::NullCharacterToken | Token::ParseError(_) => {
                let owner = facts.current_owner(self.root.identity());
                self.push(
                    DiagnosticCode::HtmlSyntax,
                    DiagnosticSeverity::Error,
                    position,
                    &owner,
                );
            }
            _ => {}
        }
    }

    fn start_tag(&mut self, facts: &mut HtmlFacts, tag: &Tag, position: Position) {
        facts.attributes = facts.attributes.saturating_add(tag.attrs.len());
        if facts.attributes > self.limits.max_attributes() {
            let owner = facts.current_owner(self.root.identity());
            self.push(
                DiagnosticCode::AttributeLimit,
                DiagnosticSeverity::Error,
                position,
                &owner,
            );
            facts.stopped = true;
            return;
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
            let owner = facts.current_owner(self.root.identity());
            self.push(
                DiagnosticCode::DynamicStructureUnproved,
                DiagnosticSeverity::Unproved,
                position,
                &owner,
            );
        }

        let prior_owner = facts.current_owner(self.root.identity());
        let mut owner = prior_owner.clone();
        let mut island = facts.current_island();
        if let Some((_, component)) = attributes.iter().find(|(name, _)| name == "live:component") {
            owner = self.resolve_component(component, &attributes, position, &prior_owner);
            island = self.next_island;
            self.next_island += 1;
            facts
                .freshness
                .insert(island, IslandFreshness::new(owner.clone(), position));
            facts
                .field_declarations
                .insert(island, IslandFieldDeclarations::default());
        }
        if let Some((_, id)) = attributes.iter().find(|(name, _)| name == "id") {
            let count = facts.ids.entry(id.clone()).or_default();
            count.fewest = count.fewest.saturating_add(1);
            count.most = count.most.saturating_add(1);
            count.owners.insert(owner.clone());
        }
        if let Some((_, target)) = attributes.iter().find(|(name, _)| {
            name.strip_prefix("live:")
                .is_some_and(|name| name.split('.').next() == Some("teleport"))
        }) && let Some(target) = target.strip_prefix('#')
        {
            facts.teleports.push(TeleportIntent {
                target: target.to_owned(),
                owner: owner.clone(),
                position,
            });
        }
        observe_freshness(facts, island, &attributes, position);
        self.observe_upload_model_exclusivity(facts, island, &attributes, position, &owner);
        let submit_form = self.observe_submit_form(facts, &attributes, position, &owner);
        self.validate_keys(facts, &attributes, position, &owner);
        self.validate_element_id(facts, &attributes, island, position, &owner);
        let ancestors: Vec<ComponentName> = std::iter::once(self.root.identity().clone())
            .chain(facts.stack.iter().map(|frame| frame.owner.clone()))
            .collect();
        let registry = self.registry;
        let owner_metadata = registry
            .resolve(&owner)
            .ok()
            .map_or(self.root, |descriptor| descriptor.metadata());
        let morph_ancestors: Vec<MorphControlKind> = facts
            .stack
            .iter()
            .filter_map(|frame| frame.morph_control)
            .collect();
        let (view, line, _) = resolve(self.files, self.root.view(), position);
        for (name, value) in &attributes {
            if name.starts_with("live:") && !matches!(name.as_str(), "live:component" | "live:key")
            {
                let mut context = DirectiveContext {
                    registry,
                    owner: owner_metadata,
                    ancestors: &ancestors,
                    morph_ancestors: &morph_ancestors,
                    tag: &tag_name,
                    attributes: &attributes,
                    path: view,
                    line,
                    diagnostics: &mut *self.diagnostics,
                };
                validate_directive(name, value, &mut context);
            }
        }
        if !tag.self_closing && !void_element(&tag_name) {
            if facts.stack.len() >= self.limits.max_stack_depth() {
                self.push(
                    DiagnosticCode::StackDepthLimit,
                    DiagnosticSeverity::Error,
                    position,
                    &owner,
                );
                facts.stopped = true;
            } else {
                facts.stack.push(ElementFrame {
                    morph_control: morph_control_kind(&attributes),
                    tag: tag_name,
                    owner,
                    island,
                    submit_form,
                });
            }
        }
    }

    /// Counts the distinct model fields under the enclosing `live:submit`
    /// form and reports the form once past what one request carries. A
    /// submit proposes every model control of its form, so a larger form
    /// has no working Live submit (LIVE-029). Merged paths count the fields
    /// any of them binds, so a form is never proved under the bound when one
    /// path exceeds it. Returns the form the element's descendants inherit.
    fn observe_submit_form(
        &mut self,
        facts: &mut HtmlFacts,
        attributes: &[(String, String)],
        position: Position,
        owner: &ComponentName,
    ) -> Option<usize> {
        let opens_form = attributes
            .iter()
            .any(|(name, _)| name == "live:submit" || name.starts_with("live:submit."));
        let form = if opens_form {
            let form = self.next_form;
            self.next_form += 1;
            facts.submit_forms.insert(form, SubmitForm::default());
            Some(form)
        } else {
            facts.stack.last().and_then(|frame| frame.submit_form)
        };
        let index = form?;
        let entry = facts.submit_forms.entry(index).or_default();
        for (name, value) in attributes {
            if name == "live:model" || name.starts_with("live:model.") {
                entry.fields.insert(value.clone());
            }
        }
        if entry.fields.len() > MAX_SUBMIT_FORM_FIELDS && !entry.reported {
            entry.reported = true;
            self.push(
                DiagnosticCode::SubmitProposalLimit,
                DiagnosticSeverity::Error,
                position,
                owner,
            );
        }
        form
    }

    fn observe_upload_model_exclusivity(
        &mut self,
        facts: &mut HtmlFacts,
        island: usize,
        attributes: &[(String, String)],
        position: Position,
        owner: &ComponentName,
    ) {
        let fields = |directive: &str| {
            attributes
                .iter()
                .filter_map(|(name, value)| {
                    (name
                        .strip_prefix("live:")
                        .and_then(|suffix| suffix.split('.').next())
                        == Some(directive)
                        && !value.contains(DYNAMIC_MARKER))
                    .then(|| ModelField::parse(value).ok())
                    .flatten()
                })
                .collect::<BTreeSet<_>>()
        };
        let uploads = fields("upload");
        let models = fields("model");
        let declarations = facts.field_declarations.entry(island).or_default();
        let conflicts = uploads
            .iter()
            .any(|field| declarations.models.contains(field))
            || models
                .iter()
                .any(|field| declarations.uploads.contains(field));
        declarations.uploads.extend(uploads);
        declarations.models.extend(models);
        if conflicts {
            self.push(
                DiagnosticCode::InvalidModifier,
                DiagnosticSeverity::Error,
                position,
                owner,
            );
        }
    }

    fn resolve_component(
        &mut self,
        value: &str,
        attributes: &[(String, String)],
        position: Position,
        fallback: &ComponentName,
    ) -> ComponentName {
        let Ok(component) = ComponentName::parse(value) else {
            self.push(
                DiagnosticCode::UnknownComponent,
                DiagnosticSeverity::Error,
                position,
                fallback,
            );
            return fallback.clone();
        };
        let Ok(descriptor) = self.registry.resolve(&component) else {
            self.push(
                DiagnosticCode::UnknownComponent,
                DiagnosticSeverity::Error,
                position,
                fallback,
            );
            return fallback.clone();
        };
        let metadata = descriptor.metadata();
        if !self.catalog.contains(metadata.view()) {
            self.push(
                DiagnosticCode::MissingView,
                DiagnosticSeverity::Error,
                position,
                metadata.identity(),
            );
        }
        if !attributes.iter().any(|(name, _)| name == "live:key") {
            self.push(
                DiagnosticCode::InvalidKey,
                DiagnosticSeverity::Error,
                position,
                metadata.identity(),
            );
        }
        metadata.identity().clone()
    }

    fn validate_keys(
        &mut self,
        facts: &mut HtmlFacts,
        attributes: &[(String, String)],
        position: Position,
        owner: &ComponentName,
    ) {
        for (_, key) in attributes.iter().filter(|(name, _)| name == "live:key") {
            let checked = (key.contains(CHECKED_KEY_MARKER) || key.contains(CHECKED_DIGEST_MARKER))
                && !key.contains(DYNAMIC_MARKER);
            let valid = key.len() <= MAX_KEY_BYTES
                && !key.contains(DYNAMIC_MARKER)
                && (facts.loop_depth == 0 || checked)
                && in_key_alphabet(key);
            if !valid {
                self.push(
                    DiagnosticCode::InvalidKey,
                    DiagnosticSeverity::Error,
                    position,
                    owner,
                );
            } else if !checked && !facts.keys.insert(key.clone()) {
                self.push(
                    DiagnosticCode::DuplicateKey,
                    DiagnosticSeverity::Error,
                    position,
                    owner,
                );
            }
        }
    }

    /// The runtime checks every element id inside an island with the
    /// stable-key rule and refuses a repeated one at the first morph
    /// (LIVE-034), so ids are held to the same rule here. A dynamic part of an
    /// id is data the checker cannot see; the literal bytes around it, the
    /// first byte included, are judged, and only a fully literal id can be
    /// known to repeat. Each island's ids are its own, as the runtime scans
    /// each island apart, and the content of a `template` element is inert
    /// markup the runtime does not scan. The checker cannot tell that an
    /// element inside a loop renders only once, so a literal id there is a
    /// repeat.
    fn validate_element_id(
        &mut self,
        facts: &mut HtmlFacts,
        attributes: &[(String, String)],
        island: usize,
        position: Position,
        owner: &ComponentName,
    ) {
        let Some((_, id)) = attributes.iter().find(|(name, _)| name == "id") else {
            return;
        };
        if facts.stack.iter().any(|frame| frame.tag == "template") {
            return;
        }
        let dynamic = id.contains(DYNAMIC_MARKER)
            || id.contains(CHECKED_KEY_MARKER)
            || id.contains(CHECKED_DIGEST_MARKER);
        let spelled = id
            .replace(DYNAMIC_MARKER, "d")
            .replace(CHECKED_KEY_MARKER, "k")
            .replace(CHECKED_DIGEST_MARKER, "k");
        if !in_key_alphabet(&spelled) || (!dynamic && id.len() > MAX_KEY_BYTES) {
            self.push(
                DiagnosticCode::InvalidElementId,
                DiagnosticSeverity::Error,
                position,
                owner,
            );
        } else if !dynamic
            && (facts.loop_depth > 0 || !facts.element_ids.insert((island, id.clone())))
        {
            self.push(
                DiagnosticCode::DuplicateElementId,
                DiagnosticSeverity::Error,
                position,
                owner,
            );
        }
    }

    fn push_stack_error(&mut self, facts: &HtmlFacts, position: Position) {
        let owner = facts.current_owner(self.root.identity());
        self.push(
            if self.branched {
                DiagnosticCode::BranchStackMismatch
            } else {
                DiagnosticCode::HtmlSyntax
            },
            DiagnosticSeverity::Error,
            position,
            &owner,
        );
    }

    fn finish(&mut self, facts: &HtmlFacts) {
        if facts.stopped {
            return;
        }
        if !facts.stack.is_empty() || facts.loop_depth != 0 {
            self.push_stack_error(facts, VIEW_START);
            return;
        }
        for freshness in facts.freshness.values() {
            for ((poll, stream), position) in &freshness.states {
                if !valid_freshness_combination(*poll, stream) {
                    self.push(
                        DiagnosticCode::InvalidModifier,
                        DiagnosticSeverity::Error,
                        *position,
                        &freshness.owner,
                    );
                }
            }
        }
        for intent in &facts.teleports {
            match facts.ids.get(&intent.target) {
                Some(count) if count.fewest == 1 && count.most == 1 => {
                    if count.owners.iter().any(|owner| owner != &intent.owner) {
                        self.push(
                            DiagnosticCode::OwnershipViolation,
                            DiagnosticSeverity::Error,
                            intent.position,
                            &intent.owner,
                        );
                    }
                }
                _ => self.push(
                    DiagnosticCode::AccessibilityViolation,
                    DiagnosticSeverity::Error,
                    intent.position,
                    &intent.owner,
                ),
            }
        }
    }

    fn push(
        &mut self,
        code: DiagnosticCode,
        severity: DiagnosticSeverity,
        position: Position,
        component: &ComponentName,
    ) {
        let (view, line, _) = resolve(self.files, self.root.view(), position);
        self.diagnostics
            .push(code, severity, Some(view), line, 1, Some(component));
    }

    fn push_located(
        &mut self,
        code: DiagnosticCode,
        severity: DiagnosticSeverity,
        position: Position,
        component: &ComponentName,
    ) {
        let (view, line, column) = resolve(self.files, self.root.view(), position);
        self.diagnostics
            .push(code, severity, Some(view), line, column, Some(component));
    }
}

fn observe_freshness(
    facts: &mut HtmlFacts,
    island: usize,
    attributes: &[(String, String)],
    position: Position,
) {
    let Some(freshness) = facts.freshness.get_mut(&island) else {
        return;
    };
    let polls = attributes.iter().any(|(name, _)| {
        name.strip_prefix("live:")
            .is_some_and(|suffix| suffix.split('.').next() == Some("poll"))
    });
    let stream = attributes
        .iter()
        .find(|(name, _)| {
            name.strip_prefix("live:")
                .is_some_and(|suffix| suffix.split('.').next() == Some("stream"))
        })
        .map(|(name, _)| {
            if name
                .split('.')
                .skip(1)
                .any(|modifier| modifier == "push-only")
            {
                "push-only"
            } else if name.split('.').skip(1).any(|modifier| modifier == "hybrid") {
                "hybrid"
            } else {
                "default"
            }
        });
    if !polls && stream.is_none() {
        return;
    }
    let states = std::mem::take(&mut freshness.states);
    for (poll, current) in states.into_keys() {
        let next = match stream {
            Some(candidate) if current == "absent" => candidate,
            Some(_) => "invalid",
            None => current,
        };
        freshness
            .states
            .entry((poll || polls, next))
            .or_insert(position);
    }
}

/// The pending text from byte `length` on, with each origin moved to
/// where its remaining text starts.
fn split_pending<'t>(pending: &[(&'t str, Origin)], length: usize) -> Vec<(&'t str, Origin)> {
    let mut start = 0usize;
    let mut rest = Vec::new();
    for (text, origin) in pending {
        let end = start.saturating_add(text.len());
        if end > length {
            let skip = length.saturating_sub(start);
            let mut origin = *origin;
            if origin.literal {
                origin.offset = origin
                    .offset
                    .saturating_add(u32::try_from(skip).unwrap_or(u32::MAX));
            }
            rest.push((text.get(skip..).unwrap_or_default(), origin));
        }
        start = end;
    }
    rest
}

/// Maps a byte of a state's pending text back to the template that wrote
/// it.
struct SourceMap {
    segments: Vec<(usize, Origin, usize)>,
}

impl SourceMap {
    fn new(pending: &[(&str, Origin)]) -> Self {
        let mut start = 0usize;
        let segments = pending
            .iter()
            .map(|(text, origin)| {
                let segment = (start, *origin, text.len());
                start = start.saturating_add(text.len());
                segment
            })
            .collect();
        Self { segments }
    }

    fn position(&self, offset: usize) -> Position {
        let index = self
            .segments
            .partition_point(|(start, _, _)| *start <= offset)
            .saturating_sub(1);
        let Some((start, origin, len)) = self.segments.get(index) else {
            return VIEW_START;
        };
        let within = offset.saturating_sub(*start).min(*len);
        Position {
            file: origin.file,
            offset: if origin.literal {
                origin
                    .offset
                    .saturating_add(u32::try_from(within).unwrap_or(u32::MAX))
            } else {
                origin.offset
            },
        }
    }
}

/// The file, line, and column of a position. A position always names a
/// file the renderer recorded; `fallback`, the component's own view, only
/// stands in should that ever not hold.
fn resolve<'f>(
    files: &'f [SourceFile<'_>],
    fallback: &'f ViewName,
    position: Position,
) -> (&'f ViewName, u32, u32) {
    let Some(file) = usize::try_from(position.file)
        .ok()
        .and_then(|index| files.get(index))
    else {
        return (fallback, 1, 1);
    };
    let offset = usize::try_from(position.offset).unwrap_or(usize::MAX);
    let (line, column) = location(file.source, offset);
    (&file.view, line, column)
}

/// Records every token with the chunk of input it was emitted in. Raw-text
/// elements switch the tokenizer the way a tree builder would, so `script`
/// and `textarea` content is not read as markup.
#[derive(Default)]
struct RecordingSink {
    recorded: RefCell<Vec<(Token, usize)>>,
    chunk: Cell<usize>,
}

impl TokenSink for RecordingSink {
    type Handle = ();

    fn process_token(&self, token: Token, _line: u64) -> TokenSinkResult<Self::Handle> {
        let transition = match &token {
            Token::TagToken(tag) if tag.kind == TagKind::StartTag => {
                raw_text_transition(&tag.name.as_ref().to_ascii_lowercase())
            }
            _ => TokenSinkResult::Continue,
        };
        self.recorded.borrow_mut().push((token, self.chunk.get()));
        transition
    }
}

/// Tokenizes `text` from html5ever's initial state and returns each token
/// with the byte offset where it starts. Unless `at_end`, the text must end
/// between tokens, which the probe tag proves: when html5ever reads it as a
/// tag of its own, nothing in `text` was left open, and a fresh tokenizer can
/// take the text that follows. `None` means the text ended inside a tag,
/// comment, or raw-text element.
fn tokenize(text: &str, at_end: bool) -> Option<Vec<(Token, usize)>> {
    let starts = chunk_starts(text);
    let tokenizer = Tokenizer::new(
        RecordingSink::default(),
        TokenizerOpts {
            discard_bom: false,
            ..TokenizerOpts::default()
        },
    );
    let queue = BufferQueue::default();
    for (index, start) in starts.iter().copied().enumerate() {
        let end = starts.get(index + 1).copied().unwrap_or(text.len());
        tokenizer.sink.chunk.set(index);
        queue.push_back(text.get(start..end).unwrap_or_default().to_tendril());
        while tokenizer.feed(&queue) != TokenizerResult::Done {}
    }
    let settled = if at_end {
        tokenizer.end();
        usize::MAX
    } else {
        let settled = tokenizer.sink.recorded.borrow().len();
        queue.push_back(PROBE.to_tendril());
        while tokenizer.feed(&queue) != TokenizerResult::Done {}
        if !probe_reached(&tokenizer.sink.recorded.borrow()[settled..]) {
            return None;
        }
        settled
    };
    let mut recorded = tokenizer.sink.recorded.into_inner();
    recorded.truncate(settled);
    Some(
        recorded
            .into_iter()
            .map(|(token, chunk)| {
                let offset = token_start(text, &starts, chunk, &token);
                (token, offset)
            })
            .collect(),
    )
}

/// Where each chunk of input starts: the text is fed in pieces that each
/// begin at a `<`, so a tag's start can be found from the chunk it ended in.
fn chunk_starts(text: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(
            text.match_indices('<')
                .map(|(index, _)| index)
                .filter(|index| *index > 0),
        )
        .collect()
}

/// The probe was read as a start tag after nothing but character data.
fn probe_reached(tail: &[(Token, usize)]) -> bool {
    let Some(((last, _), before)) = tail.split_last() else {
        return false;
    };
    before
        .iter()
        .all(|(token, _)| matches!(token, Token::CharacterTokens(_)))
        && matches!(last, Token::TagToken(tag)
            if tag.kind == TagKind::StartTag
                && tag.name.as_ref() == PROBE_TAG
                && tag.attrs.is_empty()
                && !tag.self_closing)
}

/// The byte offset where a token starts. A tag is emitted at its `>`, in the
/// chunk that holds it; it starts at the latest chunk boundary at or before
/// that which opens a tag of its name, which is that chunk itself unless an
/// attribute value held a `<`.
fn token_start(text: &str, starts: &[usize], chunk: usize, token: &Token) -> usize {
    let chunk = chunk.min(starts.len().saturating_sub(1));
    let fallback = starts.get(chunk).copied().unwrap_or(0);
    let Token::TagToken(tag) = token else {
        return fallback;
    };
    starts[..=chunk]
        .iter()
        .rev()
        .copied()
        .find(|start| opens_tag(text, *start, tag))
        .unwrap_or(fallback)
}

fn opens_tag(text: &str, start: usize, tag: &Tag) -> bool {
    let Some(rest) = text.get(start..).and_then(|rest| rest.strip_prefix('<')) else {
        return false;
    };
    let rest = if tag.kind == TagKind::EndTag {
        match rest.strip_prefix('/') {
            Some(rest) => rest,
            None => return false,
        }
    } else {
        rest
    };
    let name = tag.name.as_ref();
    rest.get(..name.len())
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(name))
        && rest[name.len()..]
            .chars()
            .next()
            .is_none_or(|next| next.is_ascii_whitespace() || matches!(next, '/' | '>'))
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
