//! Askama AST walking into a bounded tree of rendered markup.
//!
//! The renderer does not enumerate control flow. Each `{% if %}`, `{% match %}`,
//! and `{% for %}` becomes one choice whose arms are rendered once, so the
//! rendered view grows with the template, not with the product of its
//! conditionals. The HTML checker walks the tree and decides where arms must be
//! told apart.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

use askama_parser::node::{If, Macro, Node};
use askama_parser::{
    Ast, Expr, Filter, LetValueOrBlock, PathOrIdentifier, Span, Syntax, Target, WithSpan,
};

use crate::identity::{ComponentName, ViewName};

use super::diagnostic::{DiagnosticCode, DiagnosticCollector, DiagnosticSeverity};
use super::limits::CheckerLimits;
use super::template::TemplateCatalog;

pub(crate) const DYNAMIC_MARKER: &str = "suprnova-checker-dynamic-7f3e";
pub(crate) const CHECKED_KEY_MARKER: &str = "suprnova-checker-key-7f3e";
/// Stands in for a `live_key_digest` key, at that key's exact length, so a
/// key built around one is measured as the runtime will measure it.
pub(crate) const CHECKED_DIGEST_MARKER: &str = "suprnova-checker-digest-7f3e-0000";
const _: () = assert!(CHECKED_DIGEST_MARKER.len() == crate::view::DIGEST_KEY_BYTES);

pub(crate) const LOOP_START_MARKER: &str = "suprnova-checker-loop-start-7f3e";
pub(crate) const LOOP_END_MARKER: &str = "suprnova-checker-loop-end-7f3e";

/// Where a piece of rendered text came from: a template the checker read and
/// a byte offset into it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Origin {
    pub(crate) file: u32,
    pub(crate) offset: u32,
    /// The text is the template's own bytes, so each rendered byte maps to
    /// the source byte at the same distance. Text the checker substitutes, a
    /// marker or an escaped literal, maps every byte to `offset`.
    pub(crate) literal: bool,
}

/// One template the checker read, so an [`Origin`] can be reported as a
/// file, line, and column.
pub(crate) struct SourceFile<'c> {
    pub(crate) view: ViewName,
    pub(crate) source: &'c str,
}

/// One step of a rendered view: text, or a choice between arms of which the
/// template renders exactly one.
#[derive(Clone)]
pub(crate) enum Piece<'c> {
    Text(Cow<'c, str>, Origin),
    Choice(ChoicePiece<'c>),
}

/// The arms of one conditional, match, or loop, and where its tag stands.
#[derive(Clone)]
pub(crate) struct ChoicePiece<'c> {
    pub(crate) arms: Vec<Fragment<'c>>,
    pub(crate) origin: Origin,
}

/// A sequence of rendered pieces.
#[derive(Clone, Default)]
pub(crate) struct Fragment<'c> {
    pub(crate) pieces: Vec<Piece<'c>>,
}

/// The complete rendered tree of one component view.
pub(crate) struct RenderedView<'c> {
    pub(crate) fragment: Fragment<'c>,
    pub(crate) files: Vec<SourceFile<'c>>,
    /// The view renders through a choice, an include, or an inherited block,
    /// so a stack error is reported as a branch mismatch rather than plain
    /// malformed HTML. Every path through the view shares this, because every
    /// path passes each choice of the top-level sequence.
    pub(crate) branched: bool,
}

/// One definition of a block along an `{% extends %}` chain: its body and
/// the template that defines it.
#[derive(Clone, Copy)]
struct BlockLink<'c, 'a> {
    nodes: &'c [Box<Node<'a>>],
    template: &'c TemplateEnv<'a>,
}

/// Every definition of each block name along an `{% extends %}` chain, the
/// most derived first. Askama writes the first where the root template
/// places the block, with the locals there, and the next one at each
/// `{{ super() }}` inside it.
type Chain<'c, 'a> = BTreeMap<&'a str, Vec<BlockLink<'c, 'a>>>;

/// The block definition being rendered, so `{{ super() }}` can write the
/// next one up the chain.
#[derive(Clone, Copy)]
struct BlockCursor<'c, 'a> {
    name: &'a str,
    links: &'c [BlockLink<'c, 'a>],
    index: usize,
}

/// What a macro argument is bound to during one expansion. A string, number,
/// or boolean literal at the call site is substituted into the macro body, so
/// `live:model="{{ name }}"` inside a macro called with `"query"` is checked
/// as the literal binding it becomes at compile time; anything else stays
/// dynamic, exactly like a template expression outside a macro.
#[derive(Clone, Debug)]
enum Binding {
    Literal(String),
    Dynamic,
}

type Bindings = BTreeMap<String, Binding>;

/// Names bound to a value Askama writes without HTML escaping: the result of
/// `safe`, of `escape` with an escaper that does not escape HTML, or of any
/// expression built from such a name. Writing one with `{{ }}` emits raw
/// markup, so the checker follows these names through `{% let %}`, loops,
/// `if let`, `match`, and macro arguments to the place they are written.
type RawNames = BTreeSet<String>;

/// The escapers Askama 0.16 maps to its HTML escaper by default; every other
/// name, `none`, `txt`, `md`, `yml`, and the empty string among them, is a
/// text escaper that writes markup unchanged.
const HTML_ESCAPERS: &[&str] = &[
    "askama", "html", "htm", "j2", "jinja", "jinja2", "rinja", "svg", "xml",
];

/// A parsed template with the templates it imports, so a macro body can call
/// the macros its own template can see, whichever template it was called from.
struct TemplateEnv<'a> {
    view: ViewName,
    file: u32,
    source: &'a str,
    ast: Ast<'a>,
    imports: Vec<(String, TemplateEnv<'a>)>,
}

impl<'a> TemplateEnv<'a> {
    fn find_macro(
        &self,
        scope: Option<&str>,
        name: &str,
    ) -> Option<(&Macro<'a>, &TemplateEnv<'a>)> {
        match scope {
            None => self
                .ast
                .nodes()
                .iter()
                .find_map(|node| match node.as_ref() {
                    Node::Macro(definition) if *definition.name == name => {
                        Some((&**definition, self))
                    }
                    _ => None,
                }),
            Some(scope) => self
                .imports
                .iter()
                .find(|(imported, _)| imported == scope)
                .and_then(|(_, env)| env.find_macro(None, name)),
        }
    }
}

/// The names that splice a macro's caller content: `caller` itself, and any
/// alias a `{% set c = caller %}` makes.
type CallerNames = BTreeSet<String>;

/// The expansion scope handed down the node walk: the template whose macros
/// are visible, the argument bindings of the macro being expanded, and the
/// caller content a `{{ caller() }}` splices in.
struct Scope<'s, 'a> {
    template: &'s TemplateEnv<'a>,
    bindings: &'s Bindings,
    raw: &'s RawNames,
    caller: Option<&'s CallerContent<'s, 'a>>,
    caller_names: &'s CallerNames,
    block: Option<BlockCursor<'s, 'a>>,
    macro_depth: usize,
}

/// The body of a `{% call %}` block, kept unrendered. Askama renders it at
/// each `caller()` inside the macro, in the macro's scope, so a macro
/// parameter or local shadows the call site's name; it resolves macros
/// through the call site's template.
struct CallerContent<'c, 'a> {
    nodes: &'c [Box<Node<'a>>],
    template: &'c TemplateEnv<'a>,
    place: Place<'c, 'a>,
    chain: &'c Chain<'c, 'a>,
}

/// One macro expansion: the definition, the template that defines it, the
/// arguments the call passes, the caller content, if any, and where the
/// call stands.
struct Invocation<'i, 'a> {
    definition: &'i Macro<'a>,
    template: &'i TemplateEnv<'a>,
    arguments: &'i [WithSpan<Box<Expr<'a>>>],
    caller: Option<&'i CallerContent<'i, 'a>>,
    span: Span,
}

/// The tag that brought a template in, `{% include %}`, `{% import %}`, or
/// `{% extends %}`, so a template that cannot be loaded is reported where
/// it was named.
#[derive(Clone, Copy)]
struct Site<'s> {
    view: &'s ViewName,
    line: u32,
    column: u32,
}

/// The template a node list belongs to: its registered name, its index in
/// the rendered view's file table, and its source text.
#[derive(Clone, Copy)]
struct Place<'p, 'a> {
    view: &'p ViewName,
    file: u32,
    source: &'a str,
}

pub(crate) struct BranchRenderer<'checker, 'diagnostics> {
    catalog: &'checker TemplateCatalog,
    limits: CheckerLimits,
    component: &'checker ComponentName,
    diagnostics: &'diagnostics mut DiagnosticCollector,
    files: Vec<SourceFile<'checker>>,
    node_count: usize,
    expanded_bytes: usize,
    branched: bool,
    source_limit_reported: bool,
}

impl<'checker, 'diagnostics> BranchRenderer<'checker, 'diagnostics> {
    pub(crate) fn new(
        catalog: &'checker TemplateCatalog,
        limits: CheckerLimits,
        component: &'checker ComponentName,
        diagnostics: &'diagnostics mut DiagnosticCollector,
    ) -> Self {
        Self {
            catalog,
            limits,
            component,
            diagnostics,
            files: Vec::new(),
            node_count: 0,
            expanded_bytes: 0,
            branched: false,
            source_limit_reported: false,
        }
    }

    /// Renders the view into one tree, or `None` when a failure left nothing
    /// checkable; that failure has reported itself.
    pub(crate) fn render(mut self, view: &ViewName) -> Option<RenderedView<'checker>> {
        let fragment =
            self.render_view(view, &Chain::new(), &RawNames::new(), &mut Vec::new(), None)?;
        Some(RenderedView {
            fragment,
            files: self.files,
            branched: self.branched,
        })
    }

    fn file_index(&mut self, view: &ViewName, source: &'checker str) -> u32 {
        if let Some(index) = self.files.iter().position(|file| &file.view == view) {
            return u32::try_from(index).unwrap_or(u32::MAX);
        }
        self.files.push(SourceFile {
            view: view.clone(),
            source,
        });
        u32::try_from(self.files.len() - 1).unwrap_or(u32::MAX)
    }

    /// Renders one template. `raw` holds the raw names of the template that
    /// includes this one: an included template is expanded in its includer's
    /// scope, so it sees the includer's locals. `site` is the tag that named
    /// the template, absent for the component's own view.
    fn render_view(
        &mut self,
        view: &ViewName,
        chain: &Chain<'_, 'checker>,
        raw: &RawNames,
        stack: &mut Vec<ViewName>,
        site: Option<Site<'_>>,
    ) -> Option<Fragment<'checker>> {
        let (named_in, named_line, named_column) =
            site.map_or((view, 1, 1), |site| (site.view, site.line, site.column));
        if stack.len() >= self.limits.max_include_depth() || stack.contains(view) {
            self.push(
                DiagnosticCode::IncludeDepthLimit,
                DiagnosticSeverity::Error,
                named_in,
                named_line,
                named_column,
            );
            return None;
        }
        let Some(source) = self.catalog.source(view) else {
            self.push(
                DiagnosticCode::MissingTemplate,
                DiagnosticSeverity::Error,
                named_in,
                named_line,
                named_column,
            );
            return None;
        };
        if source.len() > self.limits.max_source_bytes() {
            self.push(
                DiagnosticCode::SourceLimit,
                DiagnosticSeverity::Error,
                view,
                1,
                1,
            );
            return None;
        }

        let path: Arc<std::path::Path> = Arc::from(PathBuf::from(view.as_str()));
        let ast = match Ast::from_str(source, Some(path), &Syntax::default()) {
            Ok(ast) => ast,
            Err(error) => {
                let (line, column) = location(source, error.offset);
                self.push(
                    DiagnosticCode::AskamaSyntax,
                    DiagnosticSeverity::Error,
                    view,
                    line,
                    column,
                );
                return None;
            }
        };
        let count = count_nodes(ast.nodes());
        self.node_count = self.node_count.saturating_add(count);
        if self.node_count > self.limits.max_template_nodes() {
            self.push(
                DiagnosticCode::NodeLimit,
                DiagnosticSeverity::Error,
                view,
                1,
                1,
            );
            return None;
        }

        stack.push(view.clone());
        let file = self.file_index(view, source);
        let imports = self.load_imports(&ast, view, source, stack);
        let env = TemplateEnv {
            view: view.clone(),
            file,
            source,
            ast,
            imports,
        };
        let root_bindings = Bindings::new();
        let no_caller = CallerNames::new();
        let scope = Scope {
            template: &env,
            bindings: &root_bindings,
            raw,
            caller: None,
            caller_names: &no_caller,
            block: None,
            macro_depth: 0,
        };
        let place = Place { view, file, source };
        // This template's block definitions join the chain behind the more
        // derived ones already in it.
        let mut extended: Chain<'_, 'checker> = chain.clone();
        collect_blocks(env.ast.nodes(), &env, &mut extended);
        let parent = env.ast.nodes().iter().find_map(|node| match node.as_ref() {
            Node::Extends(parent) => Some((parent.path, tag_location(source, node.span()))),
            _ => None,
        });
        let rendered = if let Some((parent, (line, column))) = parent {
            let site = Site { view, line, column };
            match ViewName::parse(parent) {
                Ok(parent) => self.render_view(&parent, &extended, raw, stack, Some(site)),
                Err(_) => {
                    self.push(
                        DiagnosticCode::MissingTemplate,
                        DiagnosticSeverity::Error,
                        view,
                        line,
                        column,
                    );
                    None
                }
            }
        } else {
            let mut fragment = Fragment::default();
            self.expand_nodes(
                env.ast.nodes(),
                &mut fragment,
                &extended,
                place,
                stack,
                &scope,
            )
            .map(|()| fragment)
        };
        stack.pop();
        rendered
    }

    /// Parses every template a `{% import %}` names so its macros can be
    /// called, under the same depth and cycle limits as includes. An import
    /// that names no template in the catalog is reported where the import
    /// stands and yields no macros.
    fn load_imports(
        &mut self,
        ast: &Ast<'checker>,
        view: &ViewName,
        source: &str,
        stack: &mut Vec<ViewName>,
    ) -> Vec<(String, TemplateEnv<'checker>)> {
        let mut imports = Vec::new();
        for node in ast.nodes() {
            let Node::Import(import) = node.as_ref() else {
                continue;
            };
            let (line, column) = tag_location(source, node.span());
            let site = Site { view, line, column };
            let Some(env) = self.load_template(import.path, site, stack) else {
                continue;
            };
            imports.push((import.scope.to_owned(), env));
        }
        imports
    }

    fn load_template(
        &mut self,
        path: &str,
        site: Site<'_>,
        stack: &mut Vec<ViewName>,
    ) -> Option<TemplateEnv<'checker>> {
        let Ok(imported) = ViewName::parse(path) else {
            self.push(
                DiagnosticCode::MissingTemplate,
                DiagnosticSeverity::Error,
                site.view,
                site.line,
                site.column,
            );
            return None;
        };
        if stack.len() >= self.limits.max_include_depth() || stack.contains(&imported) {
            self.push(
                DiagnosticCode::IncludeDepthLimit,
                DiagnosticSeverity::Error,
                site.view,
                site.line,
                site.column,
            );
            return None;
        }
        let Some(source) = self.catalog.source(&imported) else {
            self.push(
                DiagnosticCode::MissingTemplate,
                DiagnosticSeverity::Error,
                site.view,
                site.line,
                site.column,
            );
            return None;
        };
        if source.len() > self.limits.max_source_bytes() {
            self.report_source_limit(&imported, 1, 1);
            return None;
        }
        let path: Arc<std::path::Path> = Arc::from(PathBuf::from(imported.as_str()));
        let ast = match Ast::from_str(source, Some(path), &Syntax::default()) {
            Ok(ast) => ast,
            Err(error) => {
                let (line, column) = location(source, error.offset);
                self.push(
                    DiagnosticCode::AskamaSyntax,
                    DiagnosticSeverity::Error,
                    &imported,
                    line,
                    column,
                );
                return None;
            }
        };
        self.node_count = self.node_count.saturating_add(count_nodes(ast.nodes()));
        if self.node_count > self.limits.max_template_nodes() {
            self.push(
                DiagnosticCode::NodeLimit,
                DiagnosticSeverity::Error,
                &imported,
                1,
                1,
            );
            return None;
        }
        stack.push(imported.clone());
        let file = self.file_index(&imported, source);
        let imports = self.load_imports(&ast, &imported, source, stack);
        stack.pop();
        Some(TemplateEnv {
            view: imported,
            file,
            source,
            ast,
            imports,
        })
    }

    /// Renders `nodes` onto the end of `out`. `None` means this path renders
    /// nothing checkable, after the failure has reported itself, exactly as
    /// a missing include or a depth limit stops the template there.
    fn expand_nodes(
        &mut self,
        nodes: &[Box<Node<'checker>>],
        out: &mut Fragment<'checker>,
        chain: &Chain<'_, 'checker>,
        place: Place<'_, 'checker>,
        stack: &mut Vec<ViewName>,
        scope: &Scope<'_, 'checker>,
    ) -> Option<()> {
        let Place { view, source, .. } = place;
        // A `{% let %}` rebinds a name for the nodes after it in this block:
        // it shadows a macro argument's literal and makes the name raw or
        // not, by its value.
        let mut environment: Option<(Bindings, RawNames, CallerNames)> = None;
        for (index, node) in nodes.iter().enumerate() {
            let (bindings, raw, caller_names) = match &environment {
                Some((bindings, raw, caller_names)) => (bindings, raw, caller_names),
                None => (scope.bindings, scope.raw, scope.caller_names),
            };
            let scope = &Scope {
                template: scope.template,
                bindings,
                raw,
                caller: scope.caller,
                caller_names,
                block: scope.block,
                macro_depth: scope.macro_depth,
            };
            let mut rebound = None;
            match node.as_ref() {
                Node::Lit(lit) => {
                    for text in [*lit.lws, *lit.val, *lit.rws] {
                        self.push_literal(out, place, text, node.span())?;
                    }
                }
                Node::Raw(raw) => {
                    for text in [*raw.lit.lws, *raw.lit.val, *raw.lit.rws] {
                        self.push_literal(out, place, text, node.span())?;
                    }
                }
                Node::Expr(_, expression) => {
                    let origin = Origin {
                        file: place.file,
                        offset: offset_u32(expression_start(expression).unwrap_or(0)),
                        literal: false,
                    };
                    // Askama writes `{{ show(x) }}` and `{{ ui::show(x) }}` as
                    // macro calls when the name resolves to a macro, so the
                    // body is expanded and checked where the call stands.
                    // `{{ super() }}` writes the next definition of the block
                    // being rendered, with the locals here.
                    if let Expr::Call(call) = strip_groups(expression)
                        && call.args.is_empty()
                        && matches!(&**call.path, Expr::Var("super"))
                    {
                        match scope.block {
                            Some(cursor) if cursor.index + 1 < cursor.links.len() => {
                                self.branched = true;
                                let next = BlockCursor {
                                    index: cursor.index + 1,
                                    ..cursor
                                };
                                self.expand_block(
                                    out,
                                    next,
                                    chain,
                                    stack,
                                    scope,
                                    expression.span(),
                                )?;
                            }
                            _ => {
                                let (line, column) = expression_location(source, expression);
                                self.push(
                                    DiagnosticCode::DynamicStructureUnproved,
                                    DiagnosticSeverity::Unproved,
                                    view,
                                    line,
                                    column,
                                );
                            }
                        }
                        continue;
                    }
                    let splices_caller = is_caller_call(expression, scope.caller_names);
                    if !splices_caller
                        && let Expr::Call(call) = strip_groups(expression)
                        && let Some((definition, template)) = expression_macro(scope, call)
                    {
                        let invocation = Invocation {
                            definition,
                            template,
                            arguments: &call.args,
                            caller: None,
                            span: expression.span(),
                        };
                        self.expand_macro(out, invocation, place, stack, scope)?;
                        continue;
                    }
                    if splices_caller || is_caller_call(expression, &bare_caller()) {
                        let Some(caller) = scope.caller.filter(|_| splices_caller) else {
                            let (line, column) = expression_location(source, expression);
                            self.push(
                                DiagnosticCode::DynamicStructureUnproved,
                                DiagnosticSeverity::Unproved,
                                view,
                                line,
                                column,
                            );
                            continue;
                        };
                        // The macro's bindings and raw names here already lie
                        // over the call site's; `caller` itself is not visible
                        // inside its own content.
                        let no_caller = CallerNames::new();
                        let caller_scope = Scope {
                            template: caller.template,
                            bindings: scope.bindings,
                            raw: scope.raw,
                            caller: None,
                            caller_names: &no_caller,
                            block: None,
                            macro_depth: scope.macro_depth,
                        };
                        self.expand_nodes(
                            caller.nodes,
                            out,
                            caller.chain,
                            caller.place,
                            stack,
                            &caller_scope,
                        )?;
                        continue;
                    }
                    if let Some(Binding::Literal(literal)) =
                        bound_variable(expression, scope.bindings)
                    {
                        self.push_text(out, Cow::Owned(escape_html(literal)), origin, view)?;
                        continue;
                    }
                    if expression_is_raw(expression, scope.raw) {
                        let (line, column) = expression_location(source, expression);
                        self.push(
                            DiagnosticCode::RawSafe,
                            DiagnosticSeverity::Error,
                            view,
                            line,
                            column,
                        );
                    }
                    let marker =
                        if expression_uses_filter(source, expression.span(), "live_key_digest") {
                            CHECKED_DIGEST_MARKER
                        } else if expression_uses_filter(source, expression.span(), "live_key") {
                            CHECKED_KEY_MARKER
                        } else {
                            DYNAMIC_MARKER
                        };
                    self.push_text(out, Cow::Borrowed(marker), origin, view)?;
                }
                Node::If(node) => {
                    // A condition the macro's literal arguments decide is not
                    // a branch: only the arm they select is rendered, so a
                    // library macro called many times does not add a choice
                    // for every `{% if %}` it carries.
                    if let Some(decided) = decided_branch(node, scope.bindings) {
                        if let Some(nodes) = decided {
                            self.expand_nodes(nodes, out, chain, place, stack, scope)?;
                        }
                    } else {
                        // A name an `if let` binds shadows a macro argument
                        // only inside the arm that binds it.
                        let mut choices: Vec<Choice<'_, '_>> = node
                            .branches
                            .iter()
                            .map(|branch| {
                                let (names, raw) = branch.cond.as_ref().map_or_else(
                                    || (Vec::new(), None),
                                    |cond| bind_let_chain(cond, scope.raw),
                                );
                                Choice {
                                    nodes: branch.nodes.as_slice(),
                                    shadowed: shadowed_bindings(scope.bindings, &names),
                                    raw,
                                }
                            })
                            .collect();
                        if node.branches.iter().all(|branch| branch.cond.is_some()) {
                            choices.push(Choice {
                                nodes: &[],
                                shadowed: None,
                                raw: None,
                            });
                        }
                        let origin = tag_origin(place, node.span());
                        self.expand_choices(out, &choices, origin, chain, place, stack, scope)?;
                    }
                }
                Node::Match(node) => {
                    let value_is_raw = expression_is_raw(&node.expr, scope.raw);
                    let choices: Vec<Choice<'_, '_>> = node
                        .arms
                        .iter()
                        .map(|arm| {
                            let mut names = Vec::new();
                            for target in &arm.target {
                                bound_names(target, &mut names);
                            }
                            Choice {
                                nodes: arm.nodes.as_slice(),
                                shadowed: shadowed_bindings(scope.bindings, &names),
                                raw: rebind_raw(scope.raw, &names, value_is_raw),
                            }
                        })
                        .collect();
                    let origin = tag_origin(place, node.span());
                    self.expand_choices(out, &choices, origin, chain, place, stack, scope)?;
                }
                Node::Loop(node) => {
                    let mut names = vec!["loop"];
                    bound_names(&node.var, &mut names);
                    let shadowed = shadowed_bindings(scope.bindings, &names);
                    // An item of a raw sequence is raw; `loop` never is.
                    let mut loop_raw = rebind_raw(
                        scope.raw,
                        &names[1..],
                        expression_is_raw(&node.iter, scope.raw),
                    );
                    if loop_raw.as_ref().unwrap_or(scope.raw).contains("loop") {
                        loop_raw
                            .get_or_insert_with(|| scope.raw.clone())
                            .remove("loop");
                    }
                    let loop_scope = Scope {
                        template: scope.template,
                        bindings: shadowed.as_ref().unwrap_or(scope.bindings),
                        raw: loop_raw.as_ref().unwrap_or(scope.raw),
                        caller: scope.caller,
                        caller_names: scope.caller_names,
                        block: scope.block,
                        macro_depth: scope.macro_depth,
                    };
                    // The loop's own names are bound in its body, not in
                    // the `{% else %}` rendered when it has no items.
                    let origin = tag_origin(place, node.span());
                    self.expand_loop(
                        out,
                        (&node.body, &node.else_nodes),
                        origin,
                        chain,
                        place,
                        stack,
                        (&loop_scope, scope),
                    )?;
                }
                Node::Include(include) => {
                    let (line, column) = tag_location(source, node.span());
                    match ViewName::parse(include.path) {
                        Ok(include) => {
                            let site = Site { view, line, column };
                            let fragment = self.render_view(
                                &include,
                                &Chain::new(),
                                scope.raw,
                                stack,
                                Some(site),
                            )?;
                            self.branched = true;
                            out.pieces.extend(fragment.pieces);
                        }
                        Err(_) => {
                            self.push(
                                DiagnosticCode::MissingTemplate,
                                DiagnosticSeverity::Error,
                                view,
                                line,
                                column,
                            );
                            return None;
                        }
                    }
                }
                // Askama writes the most derived definition here, with the
                // locals at this site; a block nested in itself does not
                // compile and is not expanded again.
                Node::BlockDef(block) => {
                    if scope.block.is_some_and(|cursor| cursor.name == *block.name) {
                        let (line, column) = tag_location(source, node.span());
                        self.push(
                            DiagnosticCode::DynamicStructureUnproved,
                            DiagnosticSeverity::Unproved,
                            view,
                            line,
                            column,
                        );
                        continue;
                    }
                    match chain.get(*block.name).filter(|links| !links.is_empty()) {
                        Some(links) => {
                            if links.len() > 1 {
                                self.branched = true;
                            }
                            let cursor = BlockCursor {
                                name: *block.name,
                                links,
                                index: 0,
                            };
                            self.expand_block(out, cursor, chain, stack, scope, node.span())?;
                        }
                        None => {
                            self.expand_nodes(&block.nodes, out, chain, place, stack, scope)?;
                        }
                    }
                }
                Node::FilterBlock(block) => {
                    let (line, column) = tag_location(source, node.span());
                    self.push(
                        DiagnosticCode::DynamicStructureUnproved,
                        DiagnosticSeverity::Unproved,
                        view,
                        line,
                        column,
                    );
                    if filter_writes_raw(&block.filters, scope.raw) {
                        self.push(
                            DiagnosticCode::RawSafe,
                            DiagnosticSeverity::Error,
                            view,
                            line,
                            column,
                        );
                    }
                    self.expand_nodes(&block.nodes, out, chain, place, stack, scope)?;
                }
                // A macro call: the body is walked with the call's literal
                // arguments bound, the caller content spliced at each
                // `{{ caller() }}`, and the defining template's macros in
                // scope. A call the checker cannot resolve, or one passing
                // caller arguments, stays an explicit unproved result.
                Node::Call(call) => {
                    let scope_name = call.scope.as_ref().map(|scope| **scope);
                    let resolved = scope.template.find_macro(scope_name, *call.name);
                    let Some((definition, template)) = resolved else {
                        let (line, column) = tag_location(source, node.span());
                        self.push(
                            DiagnosticCode::DynamicStructureUnproved,
                            DiagnosticSeverity::Unproved,
                            view,
                            line,
                            column,
                        );
                        continue;
                    };
                    if !call.caller_args.is_empty() {
                        let (line, column) = tag_location(source, node.span());
                        self.push(
                            DiagnosticCode::DynamicStructureUnproved,
                            DiagnosticSeverity::Unproved,
                            view,
                            line,
                            column,
                        );
                        continue;
                    }
                    let caller = CallerContent {
                        nodes: &call.nodes,
                        template: scope.template,
                        place,
                        chain,
                    };
                    let invocation = Invocation {
                        definition,
                        template,
                        arguments: call.args.as_deref().unwrap_or(&[]),
                        caller: Some(&caller),
                        span: node.span(),
                    };
                    self.expand_macro(out, invocation, place, stack, scope)?;
                }
                // A definition renders nothing where it stands; its body is
                // walked at each call.
                Node::Macro(_) => {}
                Node::Let(node) => {
                    let mut names = Vec::new();
                    bound_names(&node.var, &mut names);
                    // A `let mut` name can be reassigned inside a nested
                    // block and keep that value after it.
                    let reassigned_raw = node.is_mutable && {
                        let later = possibly_raw_names(&nodes[index + 1..], scope.raw);
                        names.iter().any(|name| later.contains(*name))
                    };
                    let value_is_raw = reassigned_raw
                        || match &node.val {
                            LetValueOrBlock::Value(value) => expression_is_raw(value, scope.raw),
                            // Askama renders a `{% set %}` block into a
                            // string and escapes that string where it is
                            // written, so the name holds escaped text. A raw
                            // write inside the block is still reported where
                            // it stands; the block's own markup renders
                            // nothing here.
                            LetValueOrBlock::Block { nodes: block, .. } => {
                                let mut discarded = Fragment::default();
                                let _ = self.expand_nodes(
                                    block,
                                    &mut discarded,
                                    chain,
                                    place,
                                    stack,
                                    scope,
                                );
                                false
                            }
                        };
                    let mut bindings = scope.bindings.clone();
                    for name in &names {
                        bindings.remove(*name);
                    }
                    let raw = rebind_raw(scope.raw, &names, value_is_raw)
                        .unwrap_or_else(|| scope.raw.clone());
                    // `{% set c = caller %}` makes `c()` splice the caller
                    // content; any other value shadows a caller alias.
                    let mut caller_names = scope.caller_names.clone();
                    let aliases_caller = matches!(
                        (&node.var, &node.val),
                        (Target::Name(_), LetValueOrBlock::Value(value))
                            if matches!(&***value, Expr::Var(name) if scope.caller_names.contains(*name))
                    );
                    for name in &names {
                        if aliases_caller {
                            caller_names.insert((*name).to_owned());
                        } else {
                            caller_names.remove(*name);
                        }
                    }
                    rebound = Some((bindings, raw, caller_names));
                }
                // A name declared without a value is assigned later, possibly
                // inside a nested block whose value it keeps after the block,
                // so it is raw when any later assignment to it can be.
                Node::Declare(declare) => {
                    let name = *declare.var_name;
                    let later = possibly_raw_names(&nodes[index + 1..], scope.raw);
                    if let Some(raw) = rebind_raw(scope.raw, &[name], later.contains(name)) {
                        rebound = Some((scope.bindings.clone(), raw, scope.caller_names.clone()));
                    }
                }
                // `{% mut x = value %}` and the compound forms leave `x` raw
                // when the assigned value is raw; a raw `x` stays raw.
                Node::Compound(compound) => {
                    if expression_is_raw(&compound.op.rhs, scope.raw)
                        && let Some(name) = assigned_name(&compound.op.lhs)
                        && let Some(raw) = rebind_raw(scope.raw, &[name], true)
                    {
                        rebound = Some((scope.bindings.clone(), raw, scope.caller_names.clone()));
                    }
                }
                Node::Comment(_)
                | Node::Extends(_)
                | Node::Import(_)
                | Node::Break(_)
                | Node::Continue(_) => {}
            }
            if let Some(next) = rebound {
                environment = Some(next);
            }
        }
        Some(())
    }

    /// Expands one definition of a block where the block, or the
    /// `{{ super() }}` that asks for it, stands: in the defining template,
    /// with the bindings and raw names of the site. Block nesting counts
    /// against the include depth limit.
    fn expand_block(
        &mut self,
        out: &mut Fragment<'checker>,
        cursor: BlockCursor<'_, 'checker>,
        chain: &Chain<'_, 'checker>,
        stack: &mut Vec<ViewName>,
        scope: &Scope<'_, 'checker>,
        span: Span,
    ) -> Option<()> {
        let link = cursor.links.get(cursor.index)?;
        if scope.macro_depth >= self.limits.max_include_depth() {
            let (line, column) = tag_location(link.template.source, span);
            let view = link.template.view.clone();
            self.push(
                DiagnosticCode::IncludeDepthLimit,
                DiagnosticSeverity::Error,
                &view,
                line,
                column,
            );
            return None;
        }
        let place = Place {
            view: &link.template.view,
            file: link.template.file,
            source: link.template.source,
        };
        let no_caller = CallerNames::new();
        let block_scope = Scope {
            template: link.template,
            bindings: scope.bindings,
            raw: scope.raw,
            caller: None,
            caller_names: &no_caller,
            block: Some(cursor),
            macro_depth: scope.macro_depth + 1,
        };
        self.expand_nodes(link.nodes, out, chain, place, stack, &block_scope)
    }

    /// Expands a macro body where the call stands, with the call's arguments
    /// bound and the defining template's macros in scope, under the include
    /// depth limit, which also bounds macro recursion.
    fn expand_macro(
        &mut self,
        out: &mut Fragment<'checker>,
        invocation: Invocation<'_, 'checker>,
        place: Place<'_, 'checker>,
        stack: &mut Vec<ViewName>,
        scope: &Scope<'_, 'checker>,
    ) -> Option<()> {
        let Invocation {
            definition,
            template,
            arguments,
            caller,
            span,
        } = invocation;
        if scope.macro_depth >= self.limits.max_include_depth() {
            let (line, column) = tag_location(place.source, span);
            self.push(
                DiagnosticCode::IncludeDepthLimit,
                DiagnosticSeverity::Error,
                place.view,
                line,
                column,
            );
            return None;
        }
        let bindings = bind_arguments(definition, arguments, scope.bindings);
        let raw = bind_raw_arguments(definition, arguments, scope.raw);
        let caller_names = if caller.is_some() {
            bare_caller()
        } else {
            CallerNames::new()
        };
        let inner = Scope {
            template,
            bindings: &bindings,
            raw: &raw,
            caller,
            caller_names: &caller_names,
            block: None,
            macro_depth: scope.macro_depth + 1,
        };
        let body = Place {
            view: &template.view,
            file: template.file,
            source: template.source,
        };
        self.expand_nodes(&definition.nodes, out, &Chain::new(), body, stack, &inner)
    }

    /// Renders each arm once into one choice. An arm whose rendering fails
    /// is not a way through the template; when none is left, the path stops.
    #[allow(
        clippy::too_many_arguments,
        reason = "choice expansion keeps its authority inputs explicit"
    )]
    fn expand_choices(
        &mut self,
        out: &mut Fragment<'checker>,
        choices: &[Choice<'_, 'checker>],
        origin: Origin,
        chain: &Chain<'_, 'checker>,
        place: Place<'_, 'checker>,
        stack: &mut Vec<ViewName>,
        scope: &Scope<'_, 'checker>,
    ) -> Option<()> {
        let mut arms = Vec::with_capacity(choices.len());
        for choice in choices {
            let choice_scope = Scope {
                template: scope.template,
                bindings: choice.shadowed.as_ref().unwrap_or(scope.bindings),
                raw: choice.raw.as_ref().unwrap_or(scope.raw),
                caller: scope.caller,
                caller_names: scope.caller_names,
                block: scope.block,
                macro_depth: scope.macro_depth,
            };
            let mut arm = Fragment::default();
            if self
                .expand_nodes(choice.nodes, &mut arm, chain, place, stack, &choice_scope)
                .is_some()
            {
                arms.push(arm);
            } else if self.source_limit_reported {
                return None;
            }
        }
        if arms.is_empty() {
            return None;
        }
        self.branched = true;
        out.pieces.push(Piece::Choice(ChoicePiece { arms, origin }));
        Some(())
    }

    /// A loop is a choice between its body, between the loop markers that
    /// let the HTML check see it repeats, and its `{% else %}`.
    #[allow(
        clippy::too_many_arguments,
        reason = "loop expansion keeps its authority inputs explicit"
    )]
    fn expand_loop(
        &mut self,
        out: &mut Fragment<'checker>,
        (body, else_nodes): (&[Box<Node<'checker>>], &[Box<Node<'checker>>]),
        origin: Origin,
        chain: &Chain<'_, 'checker>,
        place: Place<'_, 'checker>,
        stack: &mut Vec<ViewName>,
        (body_scope, else_scope): (&Scope<'_, 'checker>, &Scope<'_, 'checker>),
    ) -> Option<()> {
        let view = place.view;
        let mut arms = Vec::with_capacity(2);
        let mut repeated = Fragment::default();
        let rendered = self
            .push_text(
                &mut repeated,
                Cow::Borrowed("<!--suprnova-checker-loop-start-7f3e-->"),
                origin,
                view,
            )
            .and_then(|()| self.expand_nodes(body, &mut repeated, chain, place, stack, body_scope))
            .and_then(|()| {
                self.push_text(
                    &mut repeated,
                    Cow::Borrowed("<!--suprnova-checker-loop-end-7f3e-->"),
                    origin,
                    view,
                )
            });
        if rendered.is_some() {
            arms.push(repeated);
        } else if self.source_limit_reported {
            return None;
        }
        let mut empty = Fragment::default();
        if self
            .expand_nodes(else_nodes, &mut empty, chain, place, stack, else_scope)
            .is_some()
        {
            arms.push(empty);
        } else if self.source_limit_reported {
            return None;
        }
        if arms.is_empty() {
            return None;
        }
        self.branched = true;
        out.pieces.push(Piece::Choice(ChoicePiece { arms, origin }));
        Some(())
    }

    /// Appends template text, mapped byte for byte to its source.
    fn push_literal(
        &mut self,
        out: &mut Fragment<'checker>,
        place: Place<'_, 'checker>,
        text: &'checker str,
        node: Span,
    ) -> Option<()> {
        if text.is_empty() {
            return Some(());
        }
        let origin = match offset_in(place.source, text) {
            Some(offset) => Origin {
                file: place.file,
                offset: offset_u32(offset),
                literal: true,
            },
            None => Origin {
                file: place.file,
                offset: offset_u32(node.byte_range().map_or(0, |range| range.start)),
                literal: false,
            },
        };
        self.push_text(out, Cow::Borrowed(text), origin, place.view)
    }

    fn push_text(
        &mut self,
        out: &mut Fragment<'checker>,
        text: Cow<'checker, str>,
        origin: Origin,
        view: &ViewName,
    ) -> Option<()> {
        self.charge(text.len(), origin, view)?;
        out.pieces.push(Piece::Text(text, origin));
        Some(())
    }

    /// Counts rendered bytes against the source ceiling. The whole expanded
    /// view, every arm included, must fit, which also bounds every single
    /// path through it; the text that crosses the ceiling is where it is
    /// reported.
    fn charge(&mut self, bytes: usize, origin: Origin, view: &ViewName) -> Option<()> {
        self.expanded_bytes = self.expanded_bytes.saturating_add(bytes);
        if self.expanded_bytes > self.limits.max_source_bytes() {
            // `origin` names a file of `view`'s render; `view` itself is the
            // place should the table ever lack it.
            let located = self
                .files
                .get(usize::try_from(origin.file).unwrap_or(usize::MAX))
                .map(|file| {
                    let (line, column) =
                        location(file.source, usize::try_from(origin.offset).unwrap_or(0));
                    (file.view.clone(), line, column)
                });
            let (view, line, column) = located.unwrap_or_else(|| (view.clone(), 1, 1));
            self.report_source_limit(&view, line, column);
            return None;
        }
        Some(())
    }

    fn report_source_limit(&mut self, view: &ViewName, line: u32, column: u32) {
        if self.source_limit_reported {
            return;
        }
        self.source_limit_reported = true;
        self.push(
            DiagnosticCode::SourceLimit,
            DiagnosticSeverity::Error,
            view,
            line,
            column,
        );
    }

    fn push(
        &mut self,
        code: DiagnosticCode,
        severity: DiagnosticSeverity,
        view: &ViewName,
        line: u32,
        column: u32,
    ) {
        self.diagnostics.push(
            code,
            severity,
            Some(view),
            line,
            column,
            Some(self.component),
        );
    }
}

/// Adds every block a template defines, at any depth Askama looks, to the
/// chain behind the definitions already there.
fn collect_blocks<'c, 'a>(
    nodes: &'c [Box<Node<'a>>],
    template: &'c TemplateEnv<'a>,
    chain: &mut Chain<'c, 'a>,
) {
    for node in nodes {
        match node.as_ref() {
            Node::BlockDef(block) => {
                chain.entry(*block.name).or_default().push(BlockLink {
                    nodes: &block.nodes,
                    template,
                });
                collect_blocks(&block.nodes, template, chain);
            }
            Node::If(node) => {
                for branch in &node.branches {
                    collect_blocks(&branch.nodes, template, chain);
                }
            }
            Node::Loop(node) => {
                collect_blocks(&node.body, template, chain);
                collect_blocks(&node.else_nodes, template, chain);
            }
            Node::Match(node) => {
                for arm in &node.arms {
                    collect_blocks(&arm.nodes, template, chain);
                }
            }
            _ => {}
        }
    }
}

/// The byte offset of `slice` inside `source`, when it is a subslice of it.
/// The Askama parser borrows every literal from the template text, so this
/// recovers each literal's place without a second parse.
fn offset_in(source: &str, slice: &str) -> Option<usize> {
    let start = slice.as_ptr().addr().checked_sub(source.as_ptr().addr())?;
    (start.checked_add(slice.len())? <= source.len()).then_some(start)
}

fn offset_u32(offset: usize) -> u32 {
    u32::try_from(offset).unwrap_or(u32::MAX)
}

/// Where a block tag such as `{% if %}` opens. Askama spans a block node
/// from its keyword, so the start steps back over the whitespace and
/// whitespace-control mark to the `{%` that opens the tag.
fn tag_start(source: &str, span: Span) -> usize {
    let keyword = span.byte_range().map_or(0, |range| range.start);
    let before = source.get(..keyword).unwrap_or_default();
    let trimmed = before.trim_end_matches(|character: char| {
        character.is_whitespace() || matches!(character, '-' | '+' | '~')
    });
    trimmed
        .strip_suffix("{%")
        .map_or(keyword, |opening| opening.len())
}

fn tag_origin(place: Place<'_, '_>, span: Span) -> Origin {
    Origin {
        file: place.file,
        offset: offset_u32(tag_start(place.source, span)),
        literal: false,
    }
}

fn tag_location(source: &str, span: Span) -> (u32, u32) {
    location(source, tag_start(source, span))
}

fn expression_uses_filter(source: &str, span: Span, expected: &str) -> bool {
    let Some(expression) = span.as_infix_of(source) else {
        return false;
    };
    let compact: String = expression
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect();
    compact.split('|').skip(1).any(|filter| {
        filter
            .strip_prefix(expected)
            .is_some_and(|suffix| suffix.is_empty() || suffix.starts_with('('))
    })
}

/// Whether Askama writes this expression's value without HTML escaping. It
/// reads the parsed expression, so `x|safe|lower`, a filter argument, and a
/// name bound to a raw value are all seen. A value built from a raw name is
/// raw: Askama's `escape` passes a value already marked safe through
/// unchanged, and the checker does not try to prove what any other filter or
/// method makes of raw markup.
fn expression_is_raw(expression: &Expr<'_>, raw: &RawNames) -> bool {
    match expression {
        Expr::Var(name) => raw.contains(*name),
        Expr::Filter(filter) => filter_writes_raw(filter, raw),
        other => sub_expressions(other)
            .into_iter()
            .any(|inner| expression_is_raw(inner, raw)),
    }
}

/// Whether a filter, or anything it filters, writes raw markup.
fn filter_writes_raw(filter: &Filter<'_>, raw: &RawNames) -> bool {
    filter_is_raw(filter)
        || filter
            .arguments
            .iter()
            .any(|argument| expression_is_raw(argument, raw))
}

/// `safe`, and `escape` or `e` with an escaper that does not escape HTML.
/// An escaper Askama cannot read as a string literal is refused when the
/// template compiles; the checker counts it raw rather than prove it.
fn filter_is_raw(filter: &Filter<'_>) -> bool {
    match filter_name(filter) {
        "safe" => true,
        "escape" | "e" => escaper_argument(filter).is_some_and(|escaper| match escaper {
            Expr::StrLit(literal) => !HTML_ESCAPERS.contains(&literal.content),
            _ => true,
        }),
        _ => false,
    }
}

fn filter_name<'f>(filter: &'f Filter<'_>) -> &'f str {
    match &filter.name {
        PathOrIdentifier::Identifier(name) => name,
        PathOrIdentifier::Path(path) => path.last().map_or("", |component| &component.name),
    }
}

/// The escaper `escape` was given, positionally after the filtered value or
/// as the named argument `escaper`; `None` selects the template's own HTML
/// escaper.
fn escaper_argument<'f, 'a>(filter: &'f Filter<'a>) -> Option<&'f Expr<'a>> {
    filter
        .arguments
        .iter()
        .skip(1)
        .find_map(|argument| match &***argument {
            Expr::NamedArgument(name, value) => (**name == "escaper").then_some(&***value),
            positional => Some(positional),
        })
}

/// The direct sub-expressions of an expression, so a check can look inside
/// every operand, argument, and element.
fn sub_expressions<'e, 'a>(expression: &'e Expr<'a>) -> Vec<&'e WithSpan<Box<Expr<'a>>>> {
    match expression {
        Expr::Array(items) | Expr::Tuple(items) | Expr::Concat(items) => items.iter().collect(),
        Expr::ArrayRepeat(first, second) | Expr::Index(first, second) => vec![first, second],
        Expr::AssociatedItem(inner, _)
        | Expr::As(inner, _)
        | Expr::NamedArgument(_, inner)
        | Expr::Unary(_, inner)
        | Expr::Group(inner)
        | Expr::Try(inner) => vec![inner],
        Expr::Filter(filter) => filter.arguments.iter().collect(),
        Expr::BinOp(binary) => vec![&binary.lhs, &binary.rhs],
        Expr::Range(range) => range.lhs.iter().chain(range.rhs.iter()).collect(),
        Expr::Call(call) => std::iter::once(&call.path)
            .chain(call.args.iter())
            .collect(),
        Expr::Struct(structure) => std::iter::once(&structure.path)
            .chain(
                structure
                    .fields
                    .iter()
                    .filter_map(|field| field.value.as_ref()),
            )
            .chain(structure.base.iter())
            .collect(),
        Expr::LetCond(cond) => vec![&cond.expr],
        Expr::BoolLit(_)
        | Expr::NumLit(..)
        | Expr::StrLit(_)
        | Expr::CharLit(_)
        | Expr::Var(_)
        | Expr::Path(_)
        | Expr::RustMacro(..)
        | Expr::FilterSource
        | Expr::IsDefined(_)
        | Expr::IsNotDefined(_)
        | Expr::ArgumentPlaceholder => Vec::new(),
    }
}

/// The source location where an expression starts. Askama gives a filtered
/// expression the span of its last filter, so the start is the earliest
/// span anywhere in the expression, the first character inside `{{ }}`.
fn expression_location(source: &str, expression: &WithSpan<Box<Expr<'_>>>) -> (u32, u32) {
    expression_start(expression).map_or((1, 1), |offset| location(source, offset))
}

fn expression_start(expression: &WithSpan<Box<Expr<'_>>>) -> Option<usize> {
    let own = expression.span().byte_range().map(|range| range.start);
    sub_expressions(expression)
        .into_iter()
        .filter_map(expression_start)
        .chain(own)
        .min()
}

/// Every name the nodes could bind to a raw value at any depth, ignoring
/// block scope. A name declared with `{% decl %}` or bound with `let mut` can
/// be assigned inside a nested block and keep that value after it; reading
/// this set at the declaration counts such a name raw when any assignment to
/// it, or to a name of the same spelling, could be raw. It grows to a fixed
/// point, so an assignment from a name made raw further on is seen too.
fn possibly_raw_names(nodes: &[Box<Node<'_>>], base: &RawNames) -> RawNames {
    let mut raw = base.clone();
    loop {
        let before = raw.len();
        collect_raw_assignments(nodes, &mut raw);
        if raw.len() == before {
            return raw;
        }
    }
}

fn collect_raw_assignments(nodes: &[Box<Node<'_>>], raw: &mut RawNames) {
    fn insert_all(raw: &mut RawNames, names: &[&str]) {
        raw.extend(names.iter().map(|name| (*name).to_owned()));
    }
    for node in nodes {
        let mut names = Vec::new();
        match node.as_ref() {
            Node::Let(node) => match &node.val {
                LetValueOrBlock::Value(value) => {
                    if expression_is_raw(value, raw) {
                        bound_names(&node.var, &mut names);
                        insert_all(raw, &names);
                    }
                }
                LetValueOrBlock::Block { nodes, .. } => collect_raw_assignments(nodes, raw),
            },
            Node::Compound(compound) => {
                if expression_is_raw(&compound.op.rhs, raw)
                    && let Some(name) = assigned_name(&compound.op.lhs)
                {
                    raw.insert(name.to_owned());
                }
            }
            Node::Loop(node) => {
                if expression_is_raw(&node.iter, raw) {
                    bound_names(&node.var, &mut names);
                    insert_all(raw, &names);
                }
                collect_raw_assignments(&node.body, raw);
                collect_raw_assignments(&node.else_nodes, raw);
            }
            Node::If(node) => {
                for branch in &node.branches {
                    if let Some(cond) = &branch.cond
                        && let (_, Some(bound)) = bind_let_chain(cond, raw)
                    {
                        raw.extend(bound);
                    }
                    collect_raw_assignments(&branch.nodes, raw);
                }
            }
            Node::Match(node) => {
                let value_is_raw = expression_is_raw(&node.expr, raw);
                for arm in &node.arms {
                    if value_is_raw {
                        for target in &arm.target {
                            bound_names(target, &mut names);
                        }
                    }
                    collect_raw_assignments(&arm.nodes, raw);
                }
                insert_all(raw, &names);
            }
            Node::BlockDef(block) => collect_raw_assignments(&block.nodes, raw),
            Node::Call(call) => collect_raw_assignments(&call.nodes, raw),
            Node::FilterBlock(block) => collect_raw_assignments(&block.nodes, raw),
            Node::Macro(definition) => collect_raw_assignments(&definition.nodes, raw),
            Node::Lit(_)
            | Node::Comment(_)
            | Node::Expr(..)
            | Node::Declare(_)
            | Node::Extends(_)
            | Node::Include(_)
            | Node::Import(_)
            | Node::Raw(_)
            | Node::Break(_)
            | Node::Continue(_) => {}
        }
    }
}

/// The names an `if` condition binds and the raw names after it. A leading
/// `if let` binds from the condition, and each `let` after `&&` binds from
/// its own expression, left to right, so a later binding can read an earlier
/// one. `None` when the raw names do not change.
fn bind_let_chain<'c, 'a>(
    cond: &'c askama_parser::node::CondTest<'a>,
    raw: &RawNames,
) -> (Vec<&'a str>, Option<RawNames>) {
    let mut lets = Vec::new();
    collect_lets(cond, &mut lets);
    let mut names = Vec::new();
    let mut after = raw.clone();
    for (target, value) in lets {
        let mut bound = Vec::new();
        bound_names(target, &mut bound);
        let value_is_raw = expression_is_raw(value, &after);
        if let Some(rebound) = rebind_raw(&after, &bound, value_is_raw) {
            after = rebound;
        }
        names.extend(bound);
    }
    let changed = after != *raw;
    (names, changed.then_some(after))
}

/// Each `let` of a condition with the expression it binds from, in order.
fn collect_lets<'c, 'a>(
    cond: &'c askama_parser::node::CondTest<'a>,
    lets: &mut Vec<(&'c Target<'a>, &'c Expr<'a>)>,
) {
    if let Some(target) = &cond.target {
        lets.push((target, &cond.expr));
    }
    collect_let_conditions(&cond.expr, lets);
}

fn collect_let_conditions<'c, 'a>(
    expression: &'c Expr<'a>,
    lets: &mut Vec<(&'c Target<'a>, &'c Expr<'a>)>,
) {
    match expression {
        Expr::LetCond(cond) => collect_lets(cond, lets),
        other => {
            for inner in sub_expressions(other) {
                collect_let_conditions(inner, lets);
            }
        }
    }
}

/// The variable an assignment target writes: `x` in `x`, `x.field`, or
/// `x[0]`.
fn assigned_name<'e>(target: &'e Expr<'_>) -> Option<&'e str> {
    match target {
        Expr::Var(name) => Some(name),
        Expr::AssociatedItem(inner, _)
        | Expr::Index(inner, _)
        | Expr::Group(inner)
        | Expr::Unary(_, inner) => assigned_name(inner),
        _ => None,
    }
}

/// The raw names after `names` are bound to a value that is raw or not, or
/// `None` when that changes nothing.
fn rebind_raw(raw: &RawNames, names: &[&str], value_is_raw: bool) -> Option<RawNames> {
    if !value_is_raw && !names.iter().any(|name| raw.contains(*name)) {
        return None;
    }
    let mut rebound = raw.clone();
    for name in names {
        if value_is_raw {
            rebound.insert((*name).to_owned());
        } else {
            rebound.remove(*name);
        }
    }
    Some(rebound)
}

/// Binds a macro's parameters for one call: positional arguments first, then
/// named ones, then each parameter's default. A literal binds as its text; a
/// variable that is itself bound in the calling scope carries that binding
/// through; anything else is dynamic.
fn bind_arguments(
    definition: &Macro<'_>,
    supplied: &[WithSpan<Box<Expr<'_>>>],
    outer: &Bindings,
) -> Bindings {
    // Askama expands a macro in the calling scope, so the caller's literal
    // bindings stay visible under the parameters.
    let mut bindings = outer.clone();
    let mut positional = supplied
        .iter()
        .filter(|argument| !matches!(&****argument, Expr::NamedArgument(_, _)));
    for parameter in &definition.args {
        let name = (*parameter.name).to_owned();
        let named = supplied.iter().find_map(|argument| match &***argument {
            Expr::NamedArgument(argument_name, value) if **argument_name == *parameter.name => {
                Some(&***value)
            }
            _ => None,
        });
        let value = named.or_else(|| positional.next().map(|argument| &***argument));
        let binding = match value {
            Some(expression) => binding_for(expression, outer),
            None => parameter
                .default
                .as_ref()
                .map_or(Binding::Dynamic, |default| binding_for(default, outer)),
        };
        bindings.insert(name, binding);
    }
    bindings
}

/// The raw names inside a macro body for one call. Askama expands a macro
/// in the calling scope, so the caller's raw names stay visible unless a
/// parameter shadows one; a parameter is raw when the argument it receives,
/// or its default, is raw in the calling scope.
fn bind_raw_arguments(
    definition: &Macro<'_>,
    supplied: &[WithSpan<Box<Expr<'_>>>],
    outer: &RawNames,
) -> RawNames {
    let mut raw = outer.clone();
    let mut positional = supplied
        .iter()
        .filter(|argument| !matches!(&****argument, Expr::NamedArgument(_, _)));
    for parameter in &definition.args {
        let named = supplied.iter().find_map(|argument| match &***argument {
            Expr::NamedArgument(argument_name, value) if **argument_name == *parameter.name => {
                Some(&***value)
            }
            _ => None,
        });
        let value = named.or_else(|| positional.next().map(|argument| &***argument));
        let value_is_raw = match value {
            Some(expression) => expression_is_raw(expression, outer),
            None => parameter
                .default
                .as_ref()
                .is_some_and(|default| expression_is_raw(default, outer)),
        };
        if value_is_raw {
            raw.insert((*parameter.name).to_owned());
        } else {
            raw.remove(*parameter.name);
        }
    }
    raw
}

fn binding_for(expression: &Expr<'_>, outer: &Bindings) -> Binding {
    match expression {
        Expr::StrLit(literal) => Binding::Literal(literal.content.to_owned()),
        Expr::NumLit(text, _) => Binding::Literal((*text).to_owned()),
        Expr::BoolLit(value) => Binding::Literal(value.to_string()),
        Expr::Var(name) => outer.get(*name).cloned().unwrap_or(Binding::Dynamic),
        Expr::Group(inner) => binding_for(inner, outer),
        _ => Binding::Dynamic,
    }
}

/// One arm of an `{% if %}` or a `{% match %}`: its nodes, and the argument
/// bindings with the names the arm itself binds removed, when it binds any.
struct Choice<'n, 'a> {
    nodes: &'n [Box<Node<'a>>],
    shadowed: Option<Bindings>,
    raw: Option<RawNames>,
}

/// Collects the names a `for` target, a `match` arm, or an `if let` binds.
fn bound_names<'a>(target: &Target<'a>, names: &mut Vec<&'a str>) {
    match target {
        Target::Name(name) => names.push(**name),
        Target::Tuple(tuple) => {
            for inner in &tuple.1 {
                bound_names(inner, names);
            }
        }
        Target::Array(array) => {
            for inner in array.iter() {
                bound_names(inner, names);
            }
        }
        Target::Struct(structure) => {
            for named in &structure.1 {
                bound_names(&named.dest, names);
            }
        }
        Target::OrChain(chain) => {
            for inner in chain.iter() {
                bound_names(inner, names);
            }
        }
        Target::Rest(rest) => {
            if let Some(name) = &**rest {
                names.push(**name);
            }
        }
        Target::NumLit(..)
        | Target::StrLit(_)
        | Target::CharLit(_)
        | Target::BoolLit(_)
        | Target::Path(_)
        | Target::Placeholder(_) => {}
    }
}

/// The bindings with every name a loop, a match arm, or an `if let` binds
/// removed, or `None` when it binds none of them. Inside that body the name is
/// the new binding, whatever literal a macro argument of the same name holds,
/// so the checker must not prove it from the argument (LIVE-036).
fn shadowed_bindings(bindings: &Bindings, names: &[&str]) -> Option<Bindings> {
    if !names.iter().any(|name| bindings.contains_key(*name)) {
        return None;
    }
    let mut inner = bindings.clone();
    for name in names {
        inner.remove(*name);
    }
    Some(inner)
}

/// The arm an `{% if %}` takes when every condition before it is decided by
/// the bindings: `Some(Some(nodes))` for the selected arm, `Some(None)` when
/// every condition is false and there is no `{% else %}`, and `None` when a
/// condition depends on something the bindings do not hold, which leaves the
/// node a branch.
fn decided_branch<'n, 'a>(
    node: &'n If<'a>,
    bindings: &Bindings,
) -> Option<Option<&'n [Box<Node<'a>>]>> {
    for branch in &node.branches {
        let Some(cond) = &branch.cond else {
            return Some(Some(branch.nodes.as_slice()));
        };
        if cond.target.is_some() {
            return None;
        }
        if literal_truth(&cond.expr, bindings)? {
            return Some(Some(branch.nodes.as_slice()));
        }
    }
    Some(None)
}

/// The truth of an expression the bindings decide: a bound boolean, its
/// negation, `==` and `!=` between literals, and `&&` and `||` of those. A
/// string or number is compared by its literal text; anything else is
/// undecided and stays a branch.
fn literal_truth(expression: &Expr<'_>, bindings: &Bindings) -> Option<bool> {
    match expression {
        Expr::Group(inner) => literal_truth(inner, bindings),
        Expr::Unary("!", inner) => literal_truth(inner, bindings).map(|value| !value),
        Expr::BinOp(binary) if matches!(binary.op, "==" | "!=") => {
            let (Binding::Literal(lhs), Binding::Literal(rhs)) = (
                binding_for(&binary.lhs, bindings),
                binding_for(&binary.rhs, bindings),
            ) else {
                return None;
            };
            Some((lhs == rhs) == (binary.op == "=="))
        }
        Expr::BinOp(binary) if matches!(binary.op, "&&" | "||") => {
            let lhs = literal_truth(&binary.lhs, bindings)?;
            let rhs = literal_truth(&binary.rhs, bindings)?;
            Some(if binary.op == "&&" {
                lhs && rhs
            } else {
                lhs || rhs
            })
        }
        _ => match binding_for(expression, bindings) {
            Binding::Literal(value) if value == "true" => Some(true),
            Binding::Literal(value) if value == "false" => Some(false),
            _ => None,
        },
    }
}

/// The binding of a bare variable expression, when the expression is one.
fn bound_variable<'b>(expression: &Expr<'_>, bindings: &'b Bindings) -> Option<&'b Binding> {
    match expression {
        Expr::Var(name) => bindings.get(*name),
        Expr::Group(inner) => bound_variable(inner, bindings),
        _ => None,
    }
}

/// The expression inside any parentheses around it.
fn strip_groups<'e, 'a>(expression: &'e Expr<'a>) -> &'e Expr<'a> {
    match expression {
        Expr::Group(inner) => strip_groups(inner),
        other => other,
    }
}

/// The macro a call expression names, bare or as `scope::name`, when the
/// template can see one: Askama writes such a call as the macro's body.
fn expression_macro<'s, 'a>(
    scope: &Scope<'s, 'a>,
    call: &askama_parser::expr::Call<'a>,
) -> Option<(&'s Macro<'a>, &'s TemplateEnv<'a>)> {
    match &**call.path {
        Expr::Var(name) => scope.template.find_macro(None, name),
        Expr::Path(path) => match path.as_slice() {
            [module, name] if module.generics.is_none() && name.generics.is_none() => {
                scope.template.find_macro(Some(&module.name), &name.name)
            }
            _ => None,
        },
        _ => None,
    }
}

/// Whether the expression, inside any parentheses, calls one of `names` with
/// no arguments: a splice of the caller content.
fn is_caller_call(expression: &Expr<'_>, names: &CallerNames) -> bool {
    match strip_groups(expression) {
        Expr::Call(call) => {
            call.args.is_empty() && matches!(&**call.path, Expr::Var(name) if names.contains(*name))
        }
        _ => false,
    }
}

fn bare_caller() -> CallerNames {
    CallerNames::from(["caller".to_owned()])
}

/// Escapes a substituted literal the way Askama escapes `{{ }}` output, so
/// the checked HTML is the HTML the browser receives.
fn escape_html(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#x27;"),
            other => escaped.push(other),
        }
    }
    escaped
}

fn count_nodes(nodes: &[Box<Node<'_>>]) -> usize {
    nodes.iter().fold(0usize, |count, node| {
        let nested = match node.as_ref() {
            Node::If(node) => node
                .branches
                .iter()
                .map(|branch| count_nodes(&branch.nodes))
                .sum(),
            Node::Match(node) => node.arms.iter().map(|arm| count_nodes(&arm.nodes)).sum(),
            Node::Loop(node) => count_nodes(&node.body) + count_nodes(&node.else_nodes),
            Node::BlockDef(node) => count_nodes(&node.nodes),
            Node::Macro(node) => count_nodes(&node.nodes),
            Node::Call(node) => count_nodes(&node.nodes),
            Node::FilterBlock(node) => count_nodes(&node.nodes),
            Node::Let(node) => match &node.val {
                LetValueOrBlock::Block { nodes, .. } => count_nodes(nodes),
                LetValueOrBlock::Value(_) => 0,
            },
            _ => 0,
        };
        count.saturating_add(1).saturating_add(nested)
    })
}

pub(crate) fn location(source: &str, offset: usize) -> (u32, u32) {
    let prefix = source.get(..offset).unwrap_or(source);
    let line = prefix
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        .saturating_add(1);
    let column = prefix
        .rsplit_once('\n')
        .map_or(prefix.len(), |(_, tail)| tail.len())
        .saturating_add(1);
    (
        u32::try_from(line).unwrap_or(u32::MAX),
        u32::try_from(column).unwrap_or(u32::MAX),
    )
}
