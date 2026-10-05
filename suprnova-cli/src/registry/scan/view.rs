//! The view scan (REG-031): Askama's parser for the template expressions,
//! a markup walk for the elements and attributes, and `cssparser` for the
//! stylesheets, `style` elements and `style` attributes.

mod css;
mod markup;

use std::collections::{BTreeSet, HashMap};
use std::panic::{AssertUnwindSafe, catch_unwind};

use askama_parser::node::Lit;
use askama_parser::{
    Ast, Expr, Filter, LetValueOrBlock, Node, Target, TyGenerics, TyGenericsKind, WithSpan,
};

use super::allowlist::{Admission, Allowlist, STD_ALLOWED};
use super::url::{UrlRefusal, check_constant, names_another_origin};
use super::{ComponentFiles, ScanReport};
use crate::registry::Result;
use markup::{Dynamic, Markup, Sink};

pub use markup::{REFUSED_ATTRIBUTES, REFUSED_ELEMENTS, URL_ATTRIBUTES};

/// Askama's own filters (askama 0.16), which a view may use. `safe` is
/// listed so it can be refused by name, and `escape` is admitted only with
/// the `html` escaper.
pub const ASKAMA_FILTERS: &[&str] = &[
    "assigned_or",
    "center",
    "default",
    "defined_or",
    "deref",
    "escape",
    "e",
    "filesizeformat",
    "fmt",
    "format",
    "indent",
    "join",
    "json",
    "tojson",
    "linebreaks",
    "linebreaksbr",
    "paragraphbreaks",
    "pluralize",
    "ref",
    "reject",
    "truncate",
    "urlencode",
    "urlencode_strict",
    "value",
    "wordcount",
    "capitalize",
    "lower",
    "lowercase",
    "title",
    "titlecase",
    "trim",
    "upper",
    "uppercase",
    "unique",
];

/// The framework's view filters, as `suprnova::view::filters` re-exports
/// them. `trusted_html` takes only a `TrustedHtml`, which only the
/// framework can construct (REG-030).
pub const FRAMEWORK_FILTERS: &[&str] = &["live_key", "live_key_digest", "trusted_html"];

/// Methods a view may call on a value: effect-free inspection methods of
/// `std` types.
pub const VIEW_METHODS: &[&str] = &[
    "is_empty",
    "len",
    "contains",
    "starts_with",
    "ends_with",
    "as_bytes",
    "as_str",
    "is_some",
    "is_none",
    "is_ok",
    "is_err",
    "first",
    "last",
    "get",
    "iter",
    "to_string",
    "to_lowercase",
    "to_uppercase",
    "trim",
    "unwrap_or_default",
];

/// How many different markup states the walker follows at once.
const MAX_PATHS: usize = 64;

/// How many passes over a loop body the walker makes before it gives up
/// waiting for the body's markup states to settle.
const MAX_LOOP_PASSES: usize = 8;

/// What an expression is, as far as the markup it lands in cares.
#[derive(Debug, Clone, Default)]
struct Info {
    /// A state read, a value the application passes in, or a call to a
    /// capability-free allowlist helper whose constant arguments stay on
    /// the application's origin (REG-031).
    url_source: bool,
    /// Its output is written unescaped: `caller()`, `json` and
    /// `trusted_html`.
    raw: bool,
    /// Its value, when it is a constant string.
    constant: Option<String>,
    /// It holds a string literal anywhere.
    literal: bool,
    /// The template variable it reads, for a bare read or a field of one.
    var: Option<String>,
}

/// One `{% call %}`, checked once the whole template is walked, when the
/// role of each macro parameter is known.
struct CallSite {
    scope: Option<String>,
    name: String,
    args: Vec<(Option<String>, Info)>,
    span: askama_parser::Span,
}

/// A macro the template defines: its parameters, their defaults, and the
/// positions of the parameters it writes into a URL attribute.
type MacroShape = (Vec<String>, Vec<Option<Info>>, BTreeSet<usize>);

/// Askama filters whose output is written unescaped.
const RAW_FILTERS: &[&str] = &["json", "tojson", "trusted_html", "safe"];

/// What the view scan knows about the component.
struct Context<'a> {
    allowlist: &'a Allowlist,
    namespace_module: String,
    views: BTreeSet<String>,
    rust_modules: BTreeSet<String>,
    own_methods: BTreeSet<String>,
    dependency_modules: Vec<String>,
}

/// Scans the component's views and stylesheets.
pub fn scan(component: &ComponentFiles<'_>, allowlist: &Allowlist) -> Result<ScanReport> {
    let mut views: BTreeSet<String> = component
        .files
        .iter()
        .filter(|(name, _)| name.ends_with(".html"))
        .map(|(name, _)| format!("{}-ui/{}/{name}", component.namespace, component.directory))
        .collect();
    views.extend(component.importable_views.iter().cloned());
    let rust_modules = component
        .files
        .iter()
        .filter_map(|(name, _)| name.strip_suffix(".rs").map(str::to_string))
        .collect();
    let context = Context {
        allowlist,
        namespace_module: component.namespace.replace('-', "_"),
        views,
        rust_modules,
        own_methods: own_methods(component),
        dependency_modules: component.dependency_modules.to_vec(),
    };
    let mut report = ScanReport::default();
    for (name, bytes) in component.files {
        let is_view = name.ends_with(".html");
        if !is_view && !name.ends_with(".css") {
            continue;
        }
        let mut sink = Sink {
            file: name.clone(),
            findings: Vec::new(),
            url_vars: Vec::new(),
        };
        match std::str::from_utf8(bytes) {
            Ok(text) if is_view => scan_view(text, &context, &mut sink),
            Ok(text) => css::check(text, 1, &mut sink),
            Err(_) => sink.refuse("view-parse", 1, "the file is not UTF-8".to_string()),
        }
        report.findings.extend(sink.findings);
    }
    Ok(report)
}

/// The refusals the CSS scan makes of a CSS text written somewhere other
/// than a view or stylesheet, such as a value a script assigns to a style.
pub(crate) fn check_css_text(file: &str, text: &str, line: u32) -> Vec<super::Finding> {
    let mut sink = Sink {
        file: file.to_string(),
        findings: Vec::new(),
        url_vars: Vec::new(),
    };
    css::check(text, line, &mut sink);
    sink.findings
}

/// The method names the component's own Rust defines, which its views may
/// call on the component's state.
fn own_methods(component: &ComponentFiles<'_>) -> BTreeSet<String> {
    let mut methods = BTreeSet::new();
    for (name, bytes) in component.files {
        if !name.ends_with(".rs") {
            continue;
        }
        let Ok(text) = std::str::from_utf8(bytes) else {
            continue;
        };
        // The Rust scan refuses a file built to exhaust the parser's stack;
        // it is not parsed here either.
        if super::limits::check(text, super::limits::Language::Rust).is_err() {
            continue;
        }
        let Ok(file) = syn::parse_file(text) else {
            continue;
        };
        for item in &file.items {
            if let syn::Item::Impl(item) = item {
                for member in &item.items {
                    if let syn::ImplItem::Fn(method) = member {
                        methods.insert(method.sig.ident.to_string());
                    }
                }
            }
        }
    }
    methods
}

fn scan_view(text: &str, context: &Context<'_>, sink: &mut Sink) {
    let syntax = askama_parser::Syntax::default();
    let parsed = catch_unwind(AssertUnwindSafe(|| Ast::from_str(text, None, &syntax)));
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(index, _)| index + 1))
        .collect();
    let ast = match parsed {
        Ok(Ok(ast)) => ast,
        Ok(Err(error)) => {
            let line = line_at(&line_starts, error.offset);
            let message = error
                .message
                .as_deref()
                .unwrap_or("the template does not parse")
                .to_string();
            sink.refuse(
                "view-parse",
                line,
                format!("the template does not parse: {message}"),
            );
            return;
        }
        Err(_) => {
            sink.refuse(
                "view-parse",
                1,
                "the template parser failed on this file".to_string(),
            );
            return;
        }
    };
    // An application's Askama configuration may make whitespace beside a
    // template tag preserved or suppressed by default; the markup is read
    // both ways, so whitespace removal cannot join two tokens unseen.
    for suppress_by_default in [false, true] {
        let mut walker = Walker {
            source: text,
            line_starts: &line_starts,
            context,
            sink,
            suppress_by_default,
            check_expressions: !suppress_by_default,
            imports: BTreeSet::new(),
            locals: HashMap::new(),
            macros: HashMap::new(),
            calls: Vec::new(),
        };
        let mut paths = vec![Markup::new()];
        walker.nodes(ast.nodes(), &mut paths);
        for markup in &mut paths {
            markup.finish(walker.sink);
        }
        walker.check_calls();
    }
}

fn line_at(line_starts: &[usize], offset: usize) -> u32 {
    let line = line_starts.partition_point(|start| *start <= offset);
    u32::try_from(line.max(1)).unwrap_or(u32::MAX)
}

struct Walker<'a, 'b> {
    source: &'a str,
    line_starts: &'a [usize],
    context: &'a Context<'a>,
    sink: &'b mut Sink,
    suppress_by_default: bool,
    check_expressions: bool,
    imports: BTreeSet<String>,
    /// Template-local names and whether each holds a URL source.
    locals: HashMap<String, bool>,
    /// Each macro this template defines: its parameters, their defaults,
    /// and the parameters it writes into a URL attribute.
    macros: HashMap<String, MacroShape>,
    calls: Vec<CallSite>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Leading,
    Trailing,
}

impl Walker<'_, '_> {
    fn line(&self, span: askama_parser::Span) -> u32 {
        span.byte_range()
            .map(|range| line_at(self.line_starts, range.start))
            .unwrap_or(1)
    }

    fn refuse(&mut self, check: &'static str, span: askama_parser::Span, message: String) {
        if self.check_expressions {
            let line = self.line(span);
            self.sink.refuse(check, line, message);
        }
    }

    fn refuse_markup(&mut self, check: &'static str, span: askama_parser::Span, message: String) {
        let line = self.line(span);
        self.sink.refuse(check, line, message);
    }

    // ----- markup -----------------------------------------------------

    /// Whether whitespace beside a template tag is removed: `-` on the tag
    /// always removes it, `+` and `~` keep (some of) it, and an unmarked
    /// tag follows the configuration's default.
    fn removed(&self, span: askama_parser::Span, side: Side) -> bool {
        let Some(range) = span.byte_range() else {
            return false;
        };
        let marker = match side {
            Side::Trailing => {
                let after = &self.source[range.end..];
                if ["{%", "{{", "{#"]
                    .iter()
                    .any(|open| after.starts_with(open))
                {
                    Some(after[2..].chars().next())
                } else {
                    None
                }
            }
            Side::Leading => {
                let before = &self.source[..range.start];
                if ["%}", "}}", "#}"]
                    .iter()
                    .any(|close| before.ends_with(close))
                {
                    Some(before[..before.len() - 2].chars().last())
                } else {
                    None
                }
            }
        };
        match marker {
            None => false,
            Some(Some('-')) => true,
            Some(Some('+' | '~')) => false,
            Some(_) => self.suppress_by_default,
        }
    }

    fn lit(&mut self, lit: &Lit<'_>, paths: &mut [Markup]) {
        let parts = [
            (lit.lws, Some(Side::Leading)),
            (lit.val, None),
            (lit.rws, Some(Side::Trailing)),
        ];
        for (part, side) in parts {
            if part.is_empty() {
                continue;
            }
            if let Some(side) = side
                && self.removed(part.span(), side)
            {
                continue;
            }
            let line = self.line(part.span());
            for markup in paths.iter_mut() {
                markup.text(&part, line, self.sink);
            }
        }
    }

    fn nodes(&mut self, nodes: &[Box<Node<'_>>], paths: &mut Vec<Markup>) {
        for node in nodes {
            self.node(node, paths);
        }
    }

    fn fragment(&mut self, nodes: &[Box<Node<'_>>]) {
        let mut fresh = vec![Markup::new()];
        self.nodes(nodes, &mut fresh);
        for markup in &mut fresh {
            markup.finish(self.sink);
        }
    }

    fn require_data(&mut self, paths: &[Markup], span: askama_parser::Span, what: &str) {
        if paths
            .iter()
            .any(|markup| !markup.in_data() || markup.in_foreign_content())
        {
            self.refuse_markup(
                "view-markup",
                span,
                format!("{what} inside a tag, a raw-text element or SVG writes markup the scan cannot follow"),
            );
        }
    }

    /// Keeps each distinct tokenizer state once, and refuses a template
    /// whose branches multiply the states past what the scan follows.
    fn settle(&mut self, paths: &mut Vec<Markup>, span: askama_parser::Span) {
        let mut distinct: Vec<Markup> = Vec::with_capacity(paths.len());
        for markup in paths.drain(..) {
            if !distinct.contains(&markup) {
                distinct.push(markup);
            }
        }
        if distinct.len() > MAX_PATHS {
            self.refuse_markup(
                "view-markup",
                span,
                format!("the template's branches make more than {MAX_PATHS} different markup states at once"),
            );
            distinct.truncate(MAX_PATHS);
        }
        *paths = distinct;
    }

    fn branches(
        &mut self,
        paths: &mut Vec<Markup>,
        bodies: &[&[Box<Node<'_>>]],
        exhaustive: bool,
        span: askama_parser::Span,
    ) {
        let start = paths.clone();
        let mut ends = Vec::new();
        for body in bodies {
            let mut branch = start.clone();
            self.nodes(body, &mut branch);
            ends.extend(branch);
        }
        if !exhaustive || bodies.is_empty() {
            ends.extend(start);
        }
        *paths = ends;
        self.settle(paths, span);
    }

    /// A loop body runs any number of times: its states are followed until
    /// another pass adds none.
    fn repeat(
        &mut self,
        paths: &mut Vec<Markup>,
        body: &[Box<Node<'_>>],
        span: askama_parser::Span,
    ) {
        for _ in 0..MAX_LOOP_PASSES {
            let mut next = paths.clone();
            self.nodes(body, &mut next);
            let before = paths.len();
            paths.extend(next);
            self.settle(paths, span);
            if paths.len() == before {
                return;
            }
        }
        self.refuse_markup(
            "view-markup",
            span,
            "a loop body keeps changing the markup state from one pass to the next".to_string(),
        );
    }

    fn node(&mut self, node: &Node<'_>, paths: &mut Vec<Markup>) {
        match node {
            Node::Lit(lit) => self.lit(lit, paths),
            Node::Comment(_) | Node::Declare(_) | Node::Break(_) | Node::Continue(_) => {}
            Node::Expr(_, expr) => {
                let info = self.expr(expr);
                let line = self.line(expr.span());
                if info.raw {
                    self.require_data(
                        paths,
                        expr.span(),
                        "unescaped output (`caller()`, `json` or `trusted_html`)",
                    );
                }
                for markup in paths.iter_mut() {
                    markup.dynamic(
                        Dynamic {
                            url_source: info.url_source,
                            var: info.var.clone(),
                            line,
                        },
                        self.sink,
                    );
                }
            }
            Node::Call(call) => {
                self.require_data(paths, call.span(), "a macro call");
                if let Some(scope) = &call.scope
                    && !self.imports.contains(**scope)
                {
                    self.refuse(
                        "view-include",
                        scope.span(),
                        format!(
                            "`{}::{}` calls a macro from a scope this view did not import",
                            **scope, *call.name
                        ),
                    );
                }
                let mut args = Vec::new();
                for arg in call.args.iter().flatten() {
                    let named = match &***arg {
                        Expr::NamedArgument(name, _) => Some(name.to_string()),
                        _ => None,
                    };
                    args.push((named, self.expr(arg)));
                }
                self.calls.push(CallSite {
                    scope: call.scope.as_ref().map(|scope| scope.to_string()),
                    name: call.name.to_string(),
                    args,
                    span: call.span(),
                });
                self.fragment(&call.nodes);
            }
            Node::Let(let_node) => {
                self.target(&let_node.var);
                let source = match &let_node.val {
                    LetValueOrBlock::Value(value) => self.expr(value).url_source,
                    LetValueOrBlock::Block { nodes, .. } => {
                        self.fragment(nodes);
                        false
                    }
                };
                self.bind_target(&let_node.var, source);
            }
            Node::Compound(compound) => {
                if let Expr::Var(name) = &**compound.op.lhs {
                    self.locals.insert((*name).to_string(), false);
                }
                self.expr(&compound.op.lhs);
                self.expr(&compound.op.rhs);
            }
            Node::If(if_node) => {
                let mut bodies: Vec<&[Box<Node<'_>>]> = Vec::new();
                let mut exhaustive = false;
                for branch in &if_node.branches {
                    match &branch.cond {
                        Some(test) => {
                            if let Some(target) = &test.target {
                                self.target(target);
                            }
                            let source = self.expr(&test.expr).url_source;
                            if let Some(target) = &test.target {
                                self.bind_target(target, source);
                            }
                        }
                        None => exhaustive = true,
                    }
                    bodies.push(&branch.nodes);
                }
                self.branches(paths, &bodies, exhaustive, if_node.span());
            }
            Node::Match(match_node) => {
                let source = self.expr(&match_node.expr).url_source;
                let mut bodies: Vec<&[Box<Node<'_>>]> = Vec::new();
                for arm in &match_node.arms {
                    for target in &arm.target {
                        self.target(target);
                        self.bind_target(target, source);
                    }
                    bodies.push(&arm.nodes);
                }
                self.branches(paths, &bodies, true, match_node.span());
            }
            Node::Loop(loop_node) => {
                self.target(&loop_node.var);
                let source = self.expr(&loop_node.iter).url_source;
                self.bind_target(&loop_node.var, source);
                if let Some(cond) = &loop_node.cond {
                    self.expr(cond);
                }
                // Zero passes run the `else` nodes; one or more run the body.
                let start = paths.clone();
                let mut looped = start.clone();
                self.nodes(&loop_node.body, &mut looped);
                self.repeat(&mut looped, &loop_node.body, loop_node.span());
                let mut otherwise = start;
                self.nodes(&loop_node.else_nodes, &mut otherwise);
                *paths = looped;
                paths.extend(otherwise);
                self.settle(paths, loop_node.span());
            }
            Node::Extends(extends) => self.template_target("extends", extends.path, extends.span()),
            Node::BlockDef(block) => self.nodes(&block.nodes, paths),
            Node::Include(include) => {
                self.require_data(paths, include.span(), "an `include`");
                self.template_target("include", include.path, include.span());
            }
            Node::Import(import) => {
                self.template_target("import", import.path, import.span());
                self.imports.insert(import.scope.to_string());
            }
            Node::Macro(macro_node) => {
                let params: Vec<String> = macro_node
                    .args
                    .iter()
                    .map(|arg| arg.name.to_string())
                    .collect();
                let defaults: Vec<Option<Info>> = macro_node
                    .args
                    .iter()
                    .map(|arg| arg.default.as_ref().map(|default| self.expr(default)))
                    .collect();
                let outer = std::mem::take(&mut self.sink.url_vars);
                let shadowed: Vec<(String, Option<bool>)> = params
                    .iter()
                    .map(|param| (param.clone(), self.locals.remove(param)))
                    .collect();
                self.fragment(&macro_node.nodes);
                for (param, previous) in shadowed {
                    if let Some(previous) = previous {
                        self.locals.insert(param, previous);
                    }
                }
                let used = std::mem::replace(&mut self.sink.url_vars, outer);
                let roles: BTreeSet<usize> = used
                    .iter()
                    .filter_map(|var| params.iter().position(|param| param == var))
                    .collect();
                self.macros
                    .insert(macro_node.name.to_string(), (params, defaults, roles));
            }
            Node::Raw(raw) => self.lit(&raw.lit, paths),
            Node::FilterBlock(block) => {
                if self.filter(&block.filters, block.span()).raw {
                    self.require_data(
                        paths,
                        block.span(),
                        "a filter block whose output is unescaped",
                    );
                }
                self.nodes(&block.nodes, paths);
            }
        }
    }

    fn template_target(&mut self, kind: &str, path: &str, span: askama_parser::Span) {
        if !self.context.views.contains(path) {
            self.refuse(
                "view-include",
                span,
                format!("`{kind} \"{path}\"` names a template that is neither one of the component's views, one its dependencies carry, nor a shipped view"),
            );
        }
    }

    /// Records each name a target binds and whether it holds a URL source;
    /// a name bound twice keeps the stricter reading.
    fn bind_target(&mut self, target: &Target<'_>, source: bool) {
        let mut names = Vec::new();
        target_names(target, &mut names);
        for name in names {
            let entry = self.locals.entry(name).or_insert(source);
            *entry = *entry && source;
        }
    }

    /// Checks every `{% call %}` once each macro's URL parameters are known:
    /// a value a local macro writes into a URL attribute must be a URL
    /// source or a constant on the application's origin, and a string
    /// passed to an imported macro, whose parameters the scan cannot see,
    /// must not name another origin.
    fn check_calls(&mut self) {
        let calls = std::mem::take(&mut self.calls);
        for call in calls {
            match &call.scope {
                None => {
                    let Some((params, defaults, roles)) = self.macros.get(&call.name).cloned()
                    else {
                        continue;
                    };
                    for role in roles {
                        let param = params.get(role).cloned().unwrap_or_default();
                        let arg = call
                            .args
                            .iter()
                            .find(|(named, _)| named.as_deref() == Some(param.as_str()))
                            .or_else(|| call.args.get(role).filter(|(named, _)| named.is_none()))
                            .map(|(_, info)| info.clone())
                            .or_else(|| defaults.get(role).cloned().flatten());
                        let Some(arg) = arg else {
                            continue;
                        };
                        let refused = match &arg.constant {
                            Some(constant) => {
                                check_constant(constant).err().map(UrlRefusal::describe)
                            }
                            None if arg.url_source => None,
                            None => Some("the value is one the component computes"),
                        };
                        if let Some(reason) = refused {
                            self.refuse_markup(
                                "view-url",
                                call.span,
                                format!("`{}` writes its `{param}` argument into a URL attribute: {reason}", call.name),
                            );
                        }
                    }
                }
                Some(scope) => {
                    for (_, arg) in &call.args {
                        let refused = match &arg.constant {
                            Some(constant) => names_another_origin(constant),
                            None => arg.literal && !arg.url_source,
                        };
                        if refused {
                            self.refuse_markup(
                                "view-url",
                                call.span,
                                format!(
                                    "`{scope}::{}` is passed a string that may name another origin or a computed one; a macro the scan cannot see may write it into a URL",
                                    call.name
                                ),
                            );
                        }
                    }
                }
            }
        }
    }

    // ----- expressions ------------------------------------------------

    fn target(&mut self, target: &Target<'_>) {
        match target {
            Target::Tuple(tuple) => {
                let (path, parts) = &**tuple;
                if path.len() > 1 {
                    let names: Vec<&str> = path.iter().map(|component| *component.name).collect();
                    self.value_path(&names, tuple.span(), false);
                }
                for part in parts {
                    self.target(part);
                }
            }
            Target::Struct(structure) => {
                let (path, fields) = &**structure;
                if path.len() > 1 {
                    let names: Vec<&str> = path.iter().map(|component| *component.name).collect();
                    self.value_path(&names, structure.span(), false);
                }
                for field in fields {
                    self.target(&field.dest);
                }
            }
            Target::Array(items) | Target::OrChain(items) => {
                for item in items.iter() {
                    self.target(item);
                }
            }
            Target::Path(path) if path.len() > 1 => {
                let names: Vec<&str> = path.iter().map(|component| *component.name).collect();
                self.value_path(&names, path.span(), false);
            }
            _ => {}
        }
    }

    /// Classifies a path a view names: an item of Suprnova's API that
    /// carries no capability, the component's own Rust, a dependency's
    /// module, or the effect-free part of `std`.
    fn value_path(&mut self, names: &[&str], span: askama_parser::Span, call: bool) -> Info {
        let text = names.join("::");
        let refused = |walker: &mut Self, message: String| {
            walker.refuse(if call { "view-call" } else { "view-path" }, span, message);
            Info::default()
        };
        match names.first().copied() {
            Some("suprnova") => match self.context.allowlist.admit(&text) {
                Some(Admission::Item { item, canonical }) => {
                    if item.hidden {
                        refused(self, format!("`{canonical}` is hidden, not Suprnova API"))
                    } else if let Some(capability) = item.capability {
                        refused(
                            self,
                            format!(
                                "`{canonical}` carries the {capability} capability; a view may call only what carries none"
                            ),
                        )
                    } else {
                        Info {
                            url_source: call,
                            ..Info::default()
                        }
                    }
                }
                Some(Admission::Prefix { item, root }) => {
                    if item.hidden {
                        refused(
                            self,
                            format!("`{text}` is under `{root}`, a hidden re-export"),
                        )
                    } else if let Some(capability) = item.capability {
                        refused(
                            self,
                            format!(
                                "`{text}` carries the {capability} capability; a view may call only what carries none"
                            ),
                        )
                    } else {
                        Info {
                            url_source: call,
                            ..Info::default()
                        }
                    }
                }
                Some(Admission::Module) => Info::default(),
                Some(Admission::Refused { canonical }) => {
                    let canonical = canonical.to_string();
                    refused(
                        self,
                        format!("`{canonical}` is Suprnova API the registry does not admit"),
                    )
                }
                None => refused(
                    self,
                    format!("`{text}` is not on the allowlist of Suprnova's documented API"),
                ),
            },
            Some("crate") => {
                let own = names.len() > 3
                    && names[1] == "live"
                    && names[2] == self.context.namespace_module
                    && self.context.rust_modules.contains(names[3]);
                let dependency = self.context.dependency_modules.iter().any(|module| {
                    let prefix = format!("crate::live::{module}");
                    text == prefix || text.starts_with(&format!("{prefix}::"))
                });
                if own || dependency {
                    Info::default()
                } else {
                    refused(
                        self,
                        format!("`{text}` names application code outside the component"),
                    )
                }
            }
            Some("std" | "core" | "alloc") => {
                let normalized = format!("std::{}", names[1..].join("::"));
                if STD_ALLOWED.iter().any(|entry| {
                    normalized == *entry || normalized.starts_with(&format!("{entry}::"))
                }) {
                    Info::default()
                } else {
                    refused(
                        self,
                        format!("`{text}` is outside the effect-free part of std"),
                    )
                }
            }
            Some("Some" | "None" | "Ok" | "Err") if names.len() == 1 => Info::default(),
            _ => refused(
                self,
                format!(
                    "`{text}` resolves against the module that includes the view, which the scan cannot see; name Suprnova's API by its full path"
                ),
            ),
        }
    }

    fn generics(&mut self, generics: &[WithSpan<TyGenerics<'_>>]) {
        for generic in generics {
            match &*generic.kind {
                TyGenericsKind::Path { path, args } => {
                    if path.len() > 1 {
                        let names: Vec<&str> = path.iter().map(|segment| **segment).collect();
                        self.value_path(&names, generic.span(), false);
                    }
                    if let Some(args) = args {
                        self.generics(args);
                    }
                }
                TyGenericsKind::Tuple(items) => self.generics(items),
                TyGenericsKind::Array { ty, .. } => self.generics(std::slice::from_ref(&**ty)),
            }
        }
    }

    /// Checks a filter, its name and its arguments, and reports whether its
    /// output is unescaped.
    fn filter(&mut self, filter: &Filter<'_>, span: askama_parser::Span) -> Info {
        let names: Vec<&str> = match &filter.name {
            askama_parser::PathOrIdentifier::Identifier(name) => vec![**name],
            askama_parser::PathOrIdentifier::Path(path) => {
                path.iter().map(|component| *component.name).collect()
            }
        };
        let name = match names.as_slice() {
            [single] => *single,
            ["suprnova", "view", "filters", name] if FRAMEWORK_FILTERS.contains(name) => name,
            _ => {
                self.refuse(
                    "view-filter",
                    span,
                    format!(
                        "the filter `{}` is neither Askama's own nor the framework's",
                        names.join("::")
                    ),
                );
                ""
            }
        };
        if name == "safe" {
            self.refuse(
                "view-escape",
                span,
                "the `safe` filter writes a value unescaped; only `trusted_html` may".to_string(),
            );
        } else if name == "escape" || name == "e" {
            let escaper = filter.arguments.get(1).map(|argument| match &***argument {
                Expr::StrLit(literal) => literal.content == "html",
                _ => false,
            });
            if escaper == Some(false) {
                self.refuse(
                    "view-escape",
                    span,
                    format!("`{name}` with an escaper other than `html` writes a value unescaped"),
                );
            }
        } else if !name.is_empty()
            && !ASKAMA_FILTERS.contains(&name)
            && !FRAMEWORK_FILTERS.contains(&name)
        {
            self.refuse(
                "view-filter",
                span,
                format!("the filter `{name}` is neither Askama's own nor the framework's"),
            );
        }
        let mut literal = false;
        for argument in &filter.arguments {
            literal |= self.expr(argument).literal;
        }
        Info {
            raw: RAW_FILTERS.contains(&name),
            literal,
            ..Info::default()
        }
    }

    fn expr(&mut self, expr: &WithSpan<Box<Expr<'_>>>) -> Info {
        let span = expr.span();
        match &***expr {
            Expr::StrLit(literal) => Info {
                constant: Some(unescape(literal.content)),
                literal: true,
                ..Info::default()
            },
            Expr::BoolLit(_) | Expr::NumLit(..) | Expr::CharLit(_) => Info::default(),
            Expr::Var(name) => Info {
                url_source: self.locals.get(*name).copied().unwrap_or(true),
                var: Some((*name).to_string()),
                ..Info::default()
            },
            Expr::Path(path) => {
                for component in path {
                    if let Some(generics) = &component.generics {
                        self.generics(generics);
                    }
                }
                let names: Vec<&str> = path.iter().map(|component| *component.name).collect();
                self.value_path(&names, span, false)
            }
            Expr::Concat(items) => {
                let mut constant = Some(String::new());
                let mut literal = false;
                for item in items {
                    let info = self.expr(item);
                    literal |= info.literal;
                    constant = match (constant, info.constant) {
                        (Some(mut text), Some(part)) => {
                            text.push_str(&part);
                            Some(text)
                        }
                        _ => None,
                    };
                }
                Info {
                    constant,
                    literal,
                    ..Info::default()
                }
            }
            Expr::Array(items) | Expr::Tuple(items) => {
                let mut literal = false;
                for item in items {
                    literal |= self.expr(item).literal;
                }
                Info {
                    literal,
                    ..Info::default()
                }
            }
            Expr::ArrayRepeat(item, count) => {
                let literal = self.expr(item).literal | self.expr(count).literal;
                Info {
                    literal,
                    ..Info::default()
                }
            }
            Expr::AssociatedItem(base, item) => {
                let base = self.expr(base);
                if let Some(generics) = &item.generics {
                    self.generics(generics);
                }
                Info {
                    url_source: base.url_source,
                    var: base.var,
                    literal: base.literal,
                    ..Info::default()
                }
            }
            Expr::Index(base, index) => {
                let literal = self.expr(base).literal | self.expr(index).literal;
                Info {
                    literal,
                    ..Info::default()
                }
            }
            Expr::Filter(filter) => self.filter(filter, span),
            Expr::As(inner, _) | Expr::Unary(_, inner) | Expr::Try(inner) => {
                let literal = self.expr(inner).literal;
                Info {
                    literal,
                    ..Info::default()
                }
            }
            Expr::Group(inner) => self.expr(inner),
            Expr::NamedArgument(_, value) => self.expr(value),
            Expr::BinOp(binary) => {
                let literal = self.expr(&binary.lhs).literal | self.expr(&binary.rhs).literal;
                Info {
                    literal,
                    ..Info::default()
                }
            }
            Expr::Range(range) => {
                let mut literal = false;
                if let Some(lhs) = &range.lhs {
                    literal |= self.expr(lhs).literal;
                }
                if let Some(rhs) = &range.rhs {
                    literal |= self.expr(rhs).literal;
                }
                Info {
                    literal,
                    ..Info::default()
                }
            }
            Expr::Call(call) => {
                if let Some(generics) = &call.generics {
                    self.generics(generics);
                }
                let args: Vec<Info> = call
                    .args
                    .iter()
                    .map(|argument| self.expr(argument))
                    .collect();
                let literal = args.iter().any(|arg| arg.literal);
                // A URL helper's constant arguments must stay on the
                // application's origin: `url::to` returns an absolute URL
                // it is given unchanged.
                let constants_stay = args.iter().all(|arg| {
                    arg.constant
                        .as_deref()
                        .is_none_or(|constant| check_constant(constant).is_ok())
                });
                match &**call.path {
                    Expr::Var("caller") => Info {
                        raw: true,
                        ..Info::default()
                    },
                    Expr::Var(name) => {
                        if !self.context.own_methods.contains(*name as &str) {
                            self.refuse(
                                "view-call",
                                span,
                                format!(
                                    "`{name}()` calls a method the component's Rust does not define"
                                ),
                            );
                        }
                        Info {
                            literal,
                            ..Info::default()
                        }
                    }
                    Expr::Path(path) => {
                        for component in path {
                            if let Some(generics) = &component.generics {
                                self.generics(generics);
                            }
                        }
                        let names: Vec<&str> =
                            path.iter().map(|component| *component.name).collect();
                        let info = self.value_path(&names, span, true);
                        Info {
                            url_source: info.url_source && constants_stay,
                            literal,
                            ..Info::default()
                        }
                    }
                    Expr::AssociatedItem(receiver, method) => {
                        self.expr(receiver);
                        if let Some(generics) = &method.generics {
                            self.generics(generics);
                        }
                        if !VIEW_METHODS.contains(&*method.name)
                            && !self.context.own_methods.contains(*method.name)
                        {
                            self.refuse(
                                "view-call",
                                span,
                                format!("`.{}()` is not a method a view may call", *method.name),
                            );
                        }
                        Info {
                            literal,
                            ..Info::default()
                        }
                    }
                    _ => {
                        self.refuse(
                            "view-call",
                            span,
                            "a call the scan cannot classify".to_string(),
                        );
                        Info::default()
                    }
                }
            }
            Expr::RustMacro(path, _) => {
                let name: Vec<&str> = path.iter().map(|segment| **segment).collect();
                self.refuse(
                    "view-macro",
                    span,
                    format!(
                        "the Rust macro `{}!` in a view runs at build time",
                        name.join("::")
                    ),
                );
                Info::default()
            }
            Expr::Struct(structure) => {
                let mut literal = self.expr(&structure.path).literal;
                for field in &structure.fields {
                    if let Some(value) = &field.value {
                        literal |= self.expr(value).literal;
                    }
                }
                if let Some(base) = &structure.base {
                    literal |= self.expr(base).literal;
                }
                Info {
                    literal,
                    ..Info::default()
                }
            }
            Expr::LetCond(cond) => {
                if let Some(target) = &cond.target {
                    self.target(target);
                }
                let info = self.expr(&cond.expr);
                if let Some(target) = &cond.target {
                    self.bind_target(target, info.url_source);
                }
                Info {
                    literal: info.literal,
                    ..Info::default()
                }
            }
            Expr::IsDefined(_)
            | Expr::IsNotDefined(_)
            | Expr::FilterSource
            | Expr::ArgumentPlaceholder => Info::default(),
        }
    }
}

/// The names a template target binds.
fn target_names(target: &Target<'_>, out: &mut Vec<String>) {
    match target {
        Target::Name(name) => out.push((**name).to_string()),
        Target::Tuple(tuple) => {
            for part in &tuple.1 {
                target_names(part, out);
            }
        }
        Target::Struct(structure) => {
            for field in &structure.1 {
                target_names(&field.dest, out);
            }
        }
        Target::Array(items) | Target::OrChain(items) => {
            for item in items.iter() {
                target_names(item, out);
            }
        }
        _ => {}
    }
}

/// A Rust string literal's content with its escapes resolved, as the
/// compiled template will hold it.
fn unescape(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut chars = content.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('0') => out.push('\0'),
            Some('x') => {
                let hex: String = chars.by_ref().take(2).collect();
                if let Ok(value) = u8::from_str_radix(&hex, 16) {
                    out.push(char::from(value));
                }
            }
            Some('u') => {
                let mut hex = String::new();
                if chars.peek() == Some(&'{') {
                    chars.next();
                    for c in chars.by_ref() {
                        if c == '}' {
                            break;
                        }
                        hex.push(c);
                    }
                }
                if let Some(c) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    out.push(c);
                }
            }
            Some('\n') => {
                while chars.peek().is_some_and(|c| c.is_whitespace()) {
                    chars.next();
                }
            }
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}
