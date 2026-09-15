//! Askama AST walking and bounded branch expansion.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::path::PathBuf;
use std::sync::Arc;

use askama_parser::node::{Call, If, Macro, Node};
use askama_parser::{Ast, Expr, LetValueOrBlock, PathOrIdentifier, Span, Syntax};

use crate::identity::{ComponentName, ViewName};

use super::diagnostic::{DiagnosticCode, DiagnosticCollector, DiagnosticSeverity};
use super::limits::CheckerLimits;
use super::template::TemplateCatalog;

pub(crate) const DYNAMIC_MARKER: &str = "suprnova-checker-dynamic-7f3e";
pub(crate) const CHECKED_KEY_MARKER: &str = "suprnova-checker-key-7f3e";
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
        self.render_view(view, &Overrides::new(), &mut Vec::new())
    }

    fn render_view(
        &mut self,
        view: &ViewName,
        incoming_overrides: &Overrides,
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
                Ok(parent) => self.render_view(&parent, &overrides, stack),
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
        for node in nodes {
            if branches.is_empty() {
                break;
            }
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
                    if expression_uses_raw_safe(source, expression.span()) {
                        let (line, column) = span_location(source, expression.span());
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
                        if expression_uses_filter(source, expression.span(), "live_key") {
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
                    } else {
                        let mut choices: Vec<&[Box<Node<'_>>]> = node
                            .branches
                            .iter()
                            .map(|branch| branch.nodes.as_slice())
                            .collect();
                        if node.branches.iter().all(|branch| branch.cond.is_some()) {
                            choices.push(&[]);
                        }
                        self.expand_choices(
                            branches, &choices, overrides, view, source, stack, scope,
                        )
                    }
                }
                Node::Match(node) => {
                    let choices: Vec<&[Box<Node<'_>>]> =
                        node.arms.iter().map(|arm| arm.nodes.as_slice()).collect();
                    self.expand_choices(branches, &choices, overrides, view, source, stack, scope)
                }
                Node::Loop(node) => self.expand_loop(
                    branches,
                    &node.body,
                    &node.else_nodes,
                    overrides,
                    view,
                    source,
                    stack,
                    scope,
                ),
                Node::Include(include) => match ViewName::parse(include.path) {
                    Ok(include) => {
                        let fragments = self.render_view(&include, &Overrides::new(), stack);
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
                    if filter_is_safe(&block.filters) {
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
                    let caller = if call.nodes.is_empty() {
                        Vec::new()
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
                    if let LetValueOrBlock::Block { .. } = &node.val {
                        branches
                    } else {
                        branches
                    }
                }
                Node::Comment(_)
                | Node::Declare(_)
                | Node::Compound(_)
                | Node::Extends(_)
                | Node::Import(_)
                | Node::Break(_)
                | Node::Continue(_) => branches,
            };
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
        choices: &[&[Box<Node<'_>>]],
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
                let choice_branches =
                    self.expand_nodes(choice, vec![seed], overrides, view, source, stack, scope);
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
        scope: &Scope<'_, '_>,
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
                self.expand_nodes(body, body_seeds, overrides, view, source, stack, scope);
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
                scope,
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

fn expression_uses_raw_safe(source: &str, span: Span) -> bool {
    expression_uses_filter(source, span, "safe")
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

fn filter_is_safe(filter: &askama_parser::Filter<'_>) -> bool {
    match &filter.name {
        PathOrIdentifier::Identifier(name) => **name == "safe",
        PathOrIdentifier::Path(path) => path
            .last()
            .is_some_and(|component| *component.name == "safe"),
    }
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
