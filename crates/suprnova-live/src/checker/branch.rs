//! Askama AST walking and bounded branch expansion.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

use askama_parser::node::{Call, If, Macro, Node};
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

/// Attributes no check reads: a control's checked or selected state and the
/// runtime's server-correction marker. An `{% if %}` whose every arm renders
/// only these, as a form renders each control's state from the island
/// (FORM-009), is expanded once without them instead of doubling the branch
/// states, since no check can see the difference. A check that starts reading
/// one of them must remove it from this list.
const UNCHECKED_STATE_ATTRIBUTES: &[&str] =
    &["checked", "selected", "data-suprnova-live-authoritative"];
pub(crate) const LOOP_START_MARKER: &str = "suprnova-checker-loop-start-7f3e";
pub(crate) const LOOP_END_MARKER: &str = "suprnova-checker-loop-end-7f3e";

#[derive(Clone)]
pub(crate) struct RenderedBranch {
    pub(crate) html: String,
    pub(crate) path: ViewName,
    pub(crate) branched: bool,
}

impl RenderedBranch {
    fn empty(path: &ViewName) -> Self {
        Self {
            html: String::new(),
            path: path.clone(),
            branched: false,
        }
    }
}

type Overrides = BTreeMap<String, Vec<RenderedBranch>>;

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

/// The expansion scope handed down the node walk: the template whose macros
/// are visible, the argument bindings of the macro being expanded, and the
/// caller content a `{{ caller() }}` splices in.
struct Scope<'s, 'a> {
    template: &'s TemplateEnv<'a>,
    bindings: &'s Bindings,
    raw: &'s RawNames,
    caller: Option<&'s [RenderedBranch]>,
    macro_depth: usize,
}

pub(crate) struct BranchRenderer<'checker, 'diagnostics> {
    catalog: &'checker TemplateCatalog,
    limits: CheckerLimits,
    component: &'checker ComponentName,
    diagnostics: &'diagnostics mut DiagnosticCollector,
    node_count: usize,
    branch_limit_reported: bool,
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
            node_count: 0,
            branch_limit_reported: false,
            source_limit_reported: false,
        }
    }

    pub(crate) fn render(&mut self, view: &ViewName) -> Vec<RenderedBranch> {
        self.render_view(view, &Overrides::new(), &RawNames::new(), &mut Vec::new())
    }

    /// Renders one template. `raw` holds the raw names of the template that
    /// includes this one: an included template is expanded in its includer's
    /// scope, so it sees the includer's locals.
    fn render_view(
        &mut self,
        view: &ViewName,
        incoming_overrides: &Overrides,
        raw: &RawNames,
        stack: &mut Vec<ViewName>,
    ) -> Vec<RenderedBranch> {
        if stack.len() >= self.limits.max_include_depth() || stack.contains(view) {
            self.push(
                DiagnosticCode::IncludeDepthLimit,
                DiagnosticSeverity::Error,
                view,
                1,
                1,
            );
            return Vec::new();
        }
        let Some(source) = self.catalog.source(view) else {
            self.push(
                DiagnosticCode::MissingTemplate,
                DiagnosticSeverity::Error,
                view,
                1,
                1,
            );
            return Vec::new();
        };
        if source.len() > self.limits.max_source_bytes() {
            self.push(
                DiagnosticCode::SourceLimit,
                DiagnosticSeverity::Error,
                view,
                1,
                1,
            );
            return Vec::new();
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
                return Vec::new();
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
            return Vec::new();
        }

        stack.push(view.clone());
        let imports = self.load_imports(&ast, view, stack);
        let env = TemplateEnv {
            view: view.clone(),
            source,
            ast,
            imports,
        };
        let root_bindings = Bindings::new();
        let scope = Scope {
            template: &env,
            bindings: &root_bindings,
            raw,
            caller: None,
            macro_depth: 0,
        };
        let parent = env.ast.nodes().iter().find_map(|node| match node.as_ref() {
            Node::Extends(parent) => Some(parent.path),
            _ => None,
        });
        let rendered = if let Some(parent) = parent {
            let mut overrides = incoming_overrides.clone();
            for node in env.ast.nodes() {
                if let Node::BlockDef(block) = node.as_ref() {
                    let name = (*block.name).to_owned();
                    if let Entry::Vacant(entry) = overrides.entry(name) {
                        let branches = self.expand_nodes(
                            &block.nodes,
                            vec![RenderedBranch::empty(view)],
                            incoming_overrides,
                            view,
                            source,
                            stack,
                            &scope,
                        );
                        entry.insert(branches);
                    }
                }
            }
            match ViewName::parse(parent) {
                Ok(parent) => self.render_view(&parent, &overrides, &RawNames::new(), stack),
                Err(_) => {
                    self.push(
                        DiagnosticCode::MissingTemplate,
                        DiagnosticSeverity::Error,
                        view,
                        1,
                        1,
                    );
                    Vec::new()
                }
            }
        } else {
            self.expand_nodes(
                env.ast.nodes(),
                vec![RenderedBranch::empty(view)],
                incoming_overrides,
                view,
                source,
                stack,
                &scope,
            )
        };
        stack.pop();
        rendered
    }

    /// Parses every template a `{% import %}` names so its macros can be
    /// called, under the same depth and cycle limits as includes. An import
    /// that names no template in the catalog is reported where the import
    /// stands and yields no macros.
    fn load_imports<'a>(
        &mut self,
        ast: &Ast<'a>,
        view: &ViewName,
        stack: &mut Vec<ViewName>,
    ) -> Vec<(String, TemplateEnv<'a>)>
    where
        'checker: 'a,
    {
        let mut imports = Vec::new();
        for node in ast.nodes() {
            let Node::Import(import) = node.as_ref() else {
                continue;
            };
            let Some(env) = self.load_template(import.path, view, stack) else {
                continue;
            };
            imports.push((import.scope.to_owned(), env));
        }
        imports
    }

    fn load_template<'a>(
        &mut self,
        path: &str,
        importer: &ViewName,
        stack: &mut Vec<ViewName>,
    ) -> Option<TemplateEnv<'a>>
    where
        'checker: 'a,
    {
        let Ok(imported) = ViewName::parse(path) else {
            self.push(
                DiagnosticCode::MissingTemplate,
                DiagnosticSeverity::Error,
                importer,
                1,
                1,
            );
            return None;
        };
        if stack.len() >= self.limits.max_include_depth() || stack.contains(&imported) {
            self.push(
                DiagnosticCode::IncludeDepthLimit,
                DiagnosticSeverity::Error,
                importer,
                1,
                1,
            );
            return None;
        }
        let Some(source) = self.catalog.source(&imported) else {
            self.push(
                DiagnosticCode::MissingTemplate,
                DiagnosticSeverity::Error,
                importer,
                1,
                1,
            );
            return None;
        };
        if source.len() > self.limits.max_source_bytes() {
            self.report_source_limit(&imported);
            return None;
        }
        let file: Arc<std::path::Path> = Arc::from(PathBuf::from(imported.as_str()));
        let ast = match Ast::from_str(source, Some(file), &Syntax::default()) {
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
        let imports = self.load_imports(&ast, &imported, stack);
        stack.pop();
        Some(TemplateEnv {
            view: imported,
            source,
            ast,
            imports,
        })
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "branch expansion keeps its authority inputs explicit"
    )]
    fn expand_nodes(
        &mut self,
        nodes: &[Box<Node<'_>>],
        mut branches: Vec<RenderedBranch>,
        overrides: &Overrides,
        view: &ViewName,
        source: &str,
        stack: &mut Vec<ViewName>,
        scope: &Scope<'_, '_>,
    ) -> Vec<RenderedBranch> {
        // A `{% let %}` rebinds a name for the nodes after it in this block:
        // it shadows a macro argument's literal and makes the name raw or
        // not, by its value.
        let mut environment: Option<(Bindings, RawNames)> = None;
        for (index, node) in nodes.iter().enumerate() {
            if branches.is_empty() {
                break;
            }
            let (bindings, raw) = match &environment {
                Some((bindings, raw)) => (bindings, raw),
                None => (scope.bindings, scope.raw),
            };
            let scope = &Scope {
                template: scope.template,
                bindings,
                raw,
                caller: scope.caller,
                macro_depth: scope.macro_depth,
            };
            let mut rebound = None;
            branches = match node.as_ref() {
                Node::Lit(lit) => {
                    let branches = self.append_text(branches, *lit.lws, view);
                    let branches = self.append_text(branches, *lit.val, view);
                    self.append_text(branches, *lit.rws, view)
                }
                Node::Raw(raw) => {
                    let branches = self.append_text(branches, *raw.lit.lws, view);
                    let branches = self.append_text(branches, *raw.lit.val, view);
                    self.append_text(branches, *raw.lit.rws, view)
                }
                Node::Expr(_, expression) => {
                    if is_caller_call(expression) {
                        let Some(caller) = scope.caller else {
                            let (line, column) = span_location(source, expression.span());
                            self.push(
                                DiagnosticCode::DynamicStructureUnproved,
                                DiagnosticSeverity::Unproved,
                                view,
                                line,
                                column,
                            );
                            continue;
                        };
                        branches = self.combine(branches, caller, false, view);
                        continue;
                    }
                    if let Some(Binding::Literal(literal)) =
                        bound_variable(expression, scope.bindings)
                    {
                        branches = self.append_text(branches, &escape_html(literal), view);
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
                    self.append_text(
                        branches,
                        if expression_uses_filter(source, expression.span(), "live_key_digest") {
                            CHECKED_DIGEST_MARKER
                        } else if expression_uses_filter(source, expression.span(), "live_key") {
                            CHECKED_KEY_MARKER
                        } else {
                            DYNAMIC_MARKER
                        },
                        view,
                    )
                }
                Node::If(node) => {
                    // A condition the macro's literal arguments decide is not
                    // a branch: only the arm they select is rendered, so a
                    // library macro called many times does not multiply the
                    // branch states by every `{% if %}` it carries.
                    if let Some(decided) = decided_branch(node, scope.bindings) {
                        match decided {
                            Some(nodes) => self.expand_nodes(
                                nodes, branches, overrides, view, source, stack, scope,
                            ),
                            None => branches,
                        }
                    } else if renders_only_unchecked_state(node, scope.raw) {
                        branches
                    } else {
                        // A name an `if let` binds shadows a macro argument
                        // only inside the arm that binds it.
                        let mut choices: Vec<Choice<'_, '_>> = node
                            .branches
                            .iter()
                            .map(|branch| {
                                let mut names = Vec::new();
                                let mut value_is_raw = false;
                                if let Some(cond) = branch.cond.as_ref()
                                    && let Some(target) = cond.target.as_ref()
                                {
                                    bound_names(target, &mut names);
                                    value_is_raw = expression_is_raw(&cond.expr, scope.raw);
                                }
                                Choice {
                                    nodes: branch.nodes.as_slice(),
                                    shadowed: shadowed_bindings(scope.bindings, &names),
                                    raw: rebind_raw(scope.raw, &names, value_is_raw),
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
                        self.expand_choices(
                            branches, &choices, overrides, view, source, stack, scope,
                        )
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
                    self.expand_choices(branches, &choices, overrides, view, source, stack, scope)
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
                        macro_depth: scope.macro_depth,
                    };
                    // The loop's own names are bound in its body, not in
                    // the `{% else %}` rendered when it has no items.
                    self.expand_loop(
                        branches,
                        &node.body,
                        &node.else_nodes,
                        overrides,
                        view,
                        source,
                        stack,
                        (&loop_scope, scope),
                    )
                }
                Node::Include(include) => match ViewName::parse(include.path) {
                    Ok(include) => {
                        let fragments =
                            self.render_view(&include, &Overrides::new(), scope.raw, stack);
                        self.combine(branches, &fragments, true, view)
                    }
                    Err(_) => {
                        self.push(
                            DiagnosticCode::MissingTemplate,
                            DiagnosticSeverity::Error,
                            view,
                            1,
                            1,
                        );
                        Vec::new()
                    }
                },
                Node::BlockDef(block) => {
                    if let Some(fragments) = overrides.get(*block.name) {
                        self.combine(branches, fragments, true, view)
                    } else {
                        self.expand_nodes(
                            &block.nodes,
                            branches,
                            overrides,
                            view,
                            source,
                            stack,
                            scope,
                        )
                    }
                }
                Node::FilterBlock(block) => {
                    let (line, column) = span_location(source, node.span());
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
                    self.expand_nodes(
                        &block.nodes,
                        branches,
                        overrides,
                        view,
                        source,
                        stack,
                        scope,
                    )
                }
                // A macro call: the body is walked with the call's literal
                // arguments bound, the caller content rendered first for
                // `{{ caller() }}`, and the defining template's macros in
                // scope. A call the checker cannot resolve, or one passing
                // caller arguments, stays an explicit unproved result.
                Node::Call(call) => {
                    let scope_name = call.scope.as_ref().map(|scope| **scope);
                    let resolved = scope.template.find_macro(scope_name, *call.name);
                    let Some((definition, template)) = resolved else {
                        let (line, column) = span_location(source, node.span());
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
                        let (line, column) = span_location(source, node.span());
                        self.push(
                            DiagnosticCode::DynamicStructureUnproved,
                            DiagnosticSeverity::Unproved,
                            view,
                            line,
                            column,
                        );
                        continue;
                    }
                    if scope.macro_depth >= self.limits.max_include_depth() {
                        let (line, column) = span_location(source, node.span());
                        self.push(
                            DiagnosticCode::IncludeDepthLimit,
                            DiagnosticSeverity::Error,
                            view,
                            line,
                            column,
                        );
                        return Vec::new();
                    }
                    let bindings = bind_arguments(definition, call, scope.bindings);
                    let raw = bind_raw_arguments(definition, call, scope.raw);
                    // An empty call block is empty caller content: one empty
                    // branch. Zero branches would multiply every branch after
                    // the call away and leave the rest of the view unchecked
                    // (LIVE-025).
                    let caller = if call.nodes.is_empty() {
                        vec![RenderedBranch::empty(view)]
                    } else {
                        self.expand_nodes(
                            &call.nodes,
                            vec![RenderedBranch::empty(view)],
                            overrides,
                            view,
                            source,
                            stack,
                            scope,
                        )
                    };
                    let inner = Scope {
                        template,
                        bindings: &bindings,
                        raw: &raw,
                        caller: Some(&caller),
                        macro_depth: scope.macro_depth + 1,
                    };
                    let body_view = template.view.clone();
                    let fragments = self.expand_nodes(
                        &definition.nodes,
                        vec![RenderedBranch::empty(&body_view)],
                        &Overrides::new(),
                        &body_view,
                        template.source,
                        stack,
                        &inner,
                    );
                    self.combine(branches, &fragments, false, view)
                }
                // A definition renders nothing where it stands; its body is
                // walked at each call.
                Node::Macro(_) => branches,
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
                            // Askama renders a `{% set %}` block into a string
                            // and escapes that string where it is written, so the
                            // name holds escaped text. A raw write inside the
                            // block is still reported where it stands; the
                            // block's own markup renders nothing here.
                            LetValueOrBlock::Block { nodes: block, .. } => {
                                let _ = self.expand_nodes(
                                    block,
                                    vec![RenderedBranch::empty(view)],
                                    overrides,
                                    view,
                                    source,
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
                    rebound = Some((bindings, raw));
                    branches
                }
                // A name declared without a value is assigned later, possibly
                // inside a nested block whose value it keeps after the block,
                // so it is raw when any later assignment to it can be.
                Node::Declare(declare) => {
                    let name = *declare.var_name;
                    let later = possibly_raw_names(&nodes[index + 1..], scope.raw);
                    if let Some(raw) = rebind_raw(scope.raw, &[name], later.contains(name)) {
                        rebound = Some((scope.bindings.clone(), raw));
                    }
                    branches
                }
                // `{% mut x = value %}` and the compound forms leave `x` raw
                // when the assigned value is raw; a raw `x` stays raw.
                Node::Compound(compound) => {
                    if expression_is_raw(&compound.op.rhs, scope.raw)
                        && let Some(name) = assigned_name(&compound.op.lhs)
                        && let Some(raw) = rebind_raw(scope.raw, &[name], true)
                    {
                        rebound = Some((scope.bindings.clone(), raw));
                    }
                    branches
                }
                Node::Comment(_)
                | Node::Extends(_)
                | Node::Import(_)
                | Node::Break(_)
                | Node::Continue(_) => branches,
            };
            if let Some(next) = rebound {
                environment = Some(next);
            }
        }
        branches
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "branch expansion keeps its authority inputs explicit"
    )]
    fn expand_choices(
        &mut self,
        branches: Vec<RenderedBranch>,
        choices: &[Choice<'_, '_>],
        overrides: &Overrides,
        view: &ViewName,
        source: &str,
        stack: &mut Vec<ViewName>,
        scope: &Scope<'_, '_>,
    ) -> Vec<RenderedBranch> {
        let mut expanded = Vec::new();
        for branch in branches {
            for choice in choices {
                let mut seed = branch.clone();
                seed.branched = true;
                let choice_scope = Scope {
                    template: scope.template,
                    bindings: choice.shadowed.as_ref().unwrap_or(scope.bindings),
                    raw: choice.raw.as_ref().unwrap_or(scope.raw),
                    caller: scope.caller,
                    macro_depth: scope.macro_depth,
                };
                let choice_branches = self.expand_nodes(
                    choice.nodes,
                    vec![seed],
                    overrides,
                    view,
                    source,
                    stack,
                    &choice_scope,
                );
                for choice_branch in choice_branches {
                    if !self.admit_branch(&mut expanded, choice_branch, view) {
                        return expanded;
                    }
                }
            }
        }
        expanded
    }

    fn combine(
        &mut self,
        branches: Vec<RenderedBranch>,
        fragments: &[RenderedBranch],
        branched: bool,
        view: &ViewName,
    ) -> Vec<RenderedBranch> {
        let mut combined = Vec::new();
        for branch in branches {
            for fragment in fragments {
                let mut next = branch.clone();
                let Some(next_len) = next.html.len().checked_add(fragment.html.len()) else {
                    self.report_source_limit(view);
                    return combined;
                };
                if next_len > self.limits.max_source_bytes() {
                    self.report_source_limit(view);
                    return combined;
                }
                next.html.push_str(&fragment.html);
                next.branched |= branched || fragment.branched;
                if !self.admit_branch(&mut combined, next, view) {
                    return combined;
                }
            }
        }
        combined
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "loop expansion keeps its authority inputs explicit"
    )]
    fn expand_loop(
        &mut self,
        branches: Vec<RenderedBranch>,
        body: &[Box<Node<'_>>],
        else_nodes: &[Box<Node<'_>>],
        overrides: &Overrides,
        view: &ViewName,
        source: &str,
        stack: &mut Vec<ViewName>,
        (body_scope, else_scope): (&Scope<'_, '_>, &Scope<'_, '_>),
    ) -> Vec<RenderedBranch> {
        let mut expanded = Vec::new();
        for branch in branches {
            let mut body_seed = branch.clone();
            body_seed.branched = true;
            let body_seeds = self.append_text(
                vec![body_seed],
                "<!--suprnova-checker-loop-start-7f3e-->",
                view,
            );
            let body_branches =
                self.expand_nodes(body, body_seeds, overrides, view, source, stack, body_scope);
            for body_branch in body_branches {
                let completed = self.append_text(
                    vec![body_branch],
                    "<!--suprnova-checker-loop-end-7f3e-->",
                    view,
                );
                for body_branch in completed {
                    if !self.admit_branch(&mut expanded, body_branch, view) {
                        return expanded;
                    }
                }
            }

            let mut empty_seed = branch;
            empty_seed.branched = true;
            let empty_branches = self.expand_nodes(
                else_nodes,
                vec![empty_seed],
                overrides,
                view,
                source,
                stack,
                else_scope,
            );
            for empty_branch in empty_branches {
                if !self.admit_branch(&mut expanded, empty_branch, view) {
                    return expanded;
                }
            }
        }
        expanded
    }

    fn append_text(
        &mut self,
        branches: Vec<RenderedBranch>,
        text: &str,
        view: &ViewName,
    ) -> Vec<RenderedBranch> {
        let mut appended = Vec::with_capacity(branches.len());
        for mut branch in branches {
            let Some(next_len) = branch.html.len().checked_add(text.len()) else {
                self.report_source_limit(view);
                continue;
            };
            if next_len > self.limits.max_source_bytes() {
                self.report_source_limit(view);
                continue;
            }
            branch.html.push_str(text);
            appended.push(branch);
        }
        appended
    }

    fn report_source_limit(&mut self, view: &ViewName) {
        if self.source_limit_reported {
            return;
        }
        self.source_limit_reported = true;
        self.push(
            DiagnosticCode::SourceLimit,
            DiagnosticSeverity::Error,
            view,
            1,
            1,
        );
    }

    fn admit_branch(
        &mut self,
        branches: &mut Vec<RenderedBranch>,
        branch: RenderedBranch,
        view: &ViewName,
    ) -> bool {
        if branches.len() >= self.limits.max_branch_states() {
            if !self.branch_limit_reported {
                self.branch_limit_reported = true;
                self.push(
                    DiagnosticCode::BranchLimit,
                    DiagnosticSeverity::Error,
                    view,
                    1,
                    1,
                );
            }
            return false;
        }
        branches.push(branch);
        true
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
                        && let Some(target) = &cond.target
                        && expression_is_raw(&cond.expr, raw)
                    {
                        bound_names(target, &mut names);
                    }
                    collect_raw_assignments(&branch.nodes, raw);
                }
                insert_all(raw, &names);
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
fn bind_arguments(definition: &Macro<'_>, call: &Call<'_>, outer: &Bindings) -> Bindings {
    let mut bindings = Bindings::new();
    let supplied: &[_] = call.args.as_deref().unwrap_or(&[]);
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
fn bind_raw_arguments(definition: &Macro<'_>, call: &Call<'_>, outer: &RawNames) -> RawNames {
    let mut raw = outer.clone();
    let supplied: &[_] = call.args.as_deref().unwrap_or(&[]);
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

/// Whether every arm of `node` renders only attributes no check reads, see
/// [`UNCHECKED_STATE_ATTRIBUTES`]. An arm qualifies when it is literal text
/// and expressions, the expressions sit inside quoted attribute values, none
/// is a raw `safe` output, and the text is whitespace-separated attributes
/// from that list. An `if let` never qualifies, because it binds names.
fn renders_only_unchecked_state(node: &If<'_>, raw: &RawNames) -> bool {
    node.branches.iter().all(|branch| {
        branch
            .cond
            .as_ref()
            .is_none_or(|cond| cond.target.is_none())
            && unchecked_state_only(&branch.nodes, raw)
    })
}

fn unchecked_state_only(nodes: &[Box<Node<'_>>], raw: &RawNames) -> bool {
    // An expression is spelled as a NUL, which no attribute name admits, so
    // one outside a quoted value disqualifies the arm.
    let mut text = String::new();
    for node in nodes {
        match node.as_ref() {
            Node::Lit(lit) => {
                text.push_str(*lit.lws);
                text.push_str(*lit.val);
                text.push_str(*lit.rws);
            }
            Node::Expr(_, expression) => {
                if expression_is_raw(expression, raw) {
                    return false;
                }
                text.push('\0');
            }
            Node::Comment(_) => {}
            _ => return false,
        }
    }
    let mut rest = text.as_str();
    loop {
        let trimmed = rest.trim_start_matches([' ', '\t', '\n', '\r']);
        if trimmed.is_empty() {
            return true;
        }
        if trimmed.len() == rest.len() {
            return false;
        }
        let name_len = trimmed
            .bytes()
            .take_while(|byte| byte.is_ascii_lowercase() || *byte == b'-')
            .count();
        let (name, after) = trimmed.split_at(name_len);
        if !UNCHECKED_STATE_ATTRIBUTES.contains(&name) {
            return false;
        }
        rest = match after.strip_prefix("=\"") {
            Some(value) => match value.find('"') {
                Some(end) => &value[end + 1..],
                None => return false,
            },
            None => after,
        };
    }
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
fn decided_branch<'n>(
    node: &'n If<'_>,
    bindings: &Bindings,
) -> Option<Option<&'n [Box<Node<'n>>]>> {
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

fn is_caller_call(expression: &Expr<'_>) -> bool {
    match expression {
        Expr::Call(call) => call.args.is_empty() && matches!(&**call.path, Expr::Var("caller")),
        _ => false,
    }
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

fn span_location(source: &str, span: Span) -> (u32, u32) {
    span.byte_range()
        .map_or((1, 1), |range| location(source, range.start))
}

fn location(source: &str, offset: usize) -> (u32, u32) {
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
