//! Askama AST walking and bounded branch expansion.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::path::PathBuf;
use std::sync::Arc;

use askama_parser::node::Node;
use askama_parser::{Ast, LetValueOrBlock, PathOrIdentifier, Span, Syntax};

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
        let parent = ast.nodes().iter().find_map(|node| match node.as_ref() {
            Node::Extends(parent) => Some(parent.path),
            _ => None,
        });
        let rendered = if let Some(parent) = parent {
            let mut overrides = incoming_overrides.clone();
            for node in ast.nodes() {
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
                ast.nodes(),
                vec![RenderedBranch::empty(view)],
                incoming_overrides,
                view,
                source,
                stack,
            )
        };
        stack.pop();
        rendered
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
                    let mut choices: Vec<&[Box<Node<'_>>]> = node
                        .branches
                        .iter()
                        .map(|branch| branch.nodes.as_slice())
                        .collect();
                    if node.branches.iter().all(|branch| branch.cond.is_some()) {
                        choices.push(&[]);
                    }
                    self.expand_choices(branches, &choices, overrides, view, source, stack)
                }
                Node::Match(node) => {
                    let choices: Vec<&[Box<Node<'_>>]> =
                        node.arms.iter().map(|arm| arm.nodes.as_slice()).collect();
                    self.expand_choices(branches, &choices, overrides, view, source, stack)
                }
                Node::Loop(node) => self.expand_loop(
                    branches,
                    &node.body,
                    &node.else_nodes,
                    overrides,
                    view,
                    source,
                    stack,
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
                        self.expand_nodes(&block.nodes, branches, overrides, view, source, stack)
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
                    self.expand_nodes(&block.nodes, branches, overrides, view, source, stack)
                }
                Node::Call(_) | Node::Macro(_) => {
                    let (line, column) = span_location(source, node.span());
                    self.push(
                        DiagnosticCode::DynamicStructureUnproved,
                        DiagnosticSeverity::Unproved,
                        view,
                        line,
                        column,
                    );
                    branches
                }
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
    ) -> Vec<RenderedBranch> {
        let mut expanded = Vec::new();
        for branch in branches {
            for choice in choices {
                let mut seed = branch.clone();
                seed.branched = true;
                let choice_branches =
                    self.expand_nodes(choice, vec![seed], overrides, view, source, stack);
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
            let body_branches = self.expand_nodes(body, body_seeds, overrides, view, source, stack);
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
            let empty_branches =
                self.expand_nodes(else_nodes, vec![empty_seed], overrides, view, source, stack);
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
