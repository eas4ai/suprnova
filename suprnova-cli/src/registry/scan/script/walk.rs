//! The script walker. It runs twice over the same syntax tree with the
//! same scopes, so every binding gets the same identity both times: the
//! first pass records what each binding is assigned and where each
//! function is called; the second checks every call, member access and
//! assignment against those facts and the lists in `lists.rs`.

use std::collections::{BTreeSet, HashMap};

use oxc_ast::ast::*;
use oxc_span::{GetSpan, Span};

use super::super::Finding;
use super::super::url::{check_constant, srcset_urls};
use super::lists::{
    ADMITTED_CONSTRUCTORS, ADMITTED_GLOBALS, ADMITTED_METHODS, GLOBAL_OBJECTS, IMPLICITLY_CALLED,
    INHERITED_METHODS, PAGE_METHODS, PAGE_OBJECTS, READ_ONLY_PROPERTIES, REFUSED_ELEMENTS,
    REFUSED_PROPERTIES, Rule, URL_ATTRIBUTES, URL_CSS_PROPERTIES, URL_PROPERTIES,
    constructor_rule_for, rule_for,
};

/// How many constant values a traced expression may stand for before the
/// scan stops following it.
const MAX_VALUES: usize = 64;

/// How deep tracing follows bindings and call sites.
const MAX_TRACE_DEPTH: usize = 12;

/// How deep the walker follows nested expressions.
const MAX_DEPTH: usize = 200;

/// Why destructuring `prototype` out of an object is refused (REG-032).
const DESTRUCTURED_PROTOTYPE: &str =
    "destructuring `prototype` puts a prototype in a name the scan does not follow";

pub(super) type Bid = usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Var,
    Let,
    Const,
    Function,
    Class,
    Param,
    Import,
    Catch,
    Implicit,
}

pub(super) struct Binding<'a> {
    pub kind: Kind,
    pub init: Option<&'a Expression<'a>>,
    pub function: bool,
    pub class: Option<&'a Class<'a>>,
    pub param_of: Option<Bid>,
    pub param_index: usize,
    pub param_default: Option<&'a Expression<'a>>,
    pub assignments: Vec<&'a Expression<'a>>,
    pub opaque: bool,
    pub numeric_updates: bool,
    pub calls: Vec<&'a oxc_allocator::Vec<'a, Argument<'a>>>,
    pub escapes: bool,
    /// Whether another script can import the binding, so what it holds
    /// leaves the file the scan reads (REG-032).
    pub exported: bool,
    /// The function a function declaration binds, whose parameters a call
    /// by name hands its arguments to.
    pub declaration: Option<&'a Function<'a>>,
}

#[derive(Default)]
pub(super) struct Facts<'a> {
    pub bindings: Vec<Binding<'a>>,
    /// The methods each class the script defines carries, by the offset
    /// of the class: its body's methods and function-valued fields, and the
    /// functions its methods assign to `this`.
    pub instance_methods: HashMap<u32, BTreeSet<&'a str>>,
    pub private_methods: BTreeSet<&'a str>,
    pub tainted: BTreeSet<&'a str>,
    /// The globals of the script's own: names it writes on the global
    /// object that no browser property may hold, which it may read back.
    pub written_globals: BTreeSet<String>,
    /// The binding each name the walk resolved had where it is written, by
    /// the name's offset, or `None` for a global: a value followed from
    /// elsewhere names these bindings, not the ones the same names have
    /// where the value is used (REG-032).
    pub resolved: HashMap<u32, Option<Bid>>,
    /// The class each `this` in an instance context stands for, by the
    /// offset of the `this` and of the class.
    pub this_class: HashMap<u32, u32>,
    /// Every member name a class the script defines gives its instances or
    /// itself, by the offset of the class: its methods, fields and
    /// accessors, and each name its code writes on `this`.
    pub class_members: HashMap<u32, BTreeSet<String>>,
    /// Every property name the script defines on an object of its own: an
    /// object literal's key, a class member, a name it writes on a member.
    /// A method of that name may be the script's, not a browser API's.
    pub defined_names: BTreeSet<String>,
    /// The `arguments` binding of each function, by the function's offset:
    /// a function that reads it receives arguments its parameters do not
    /// name.
    pub arguments_of: HashMap<u32, Bid>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Phase {
    Collect,
    Check,
}

/// Where an expression stands, for the rules that depend on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pos {
    /// A value the script stores or hands on where the scan stops
    /// following it: an argument, an element, a member, a `return`.
    Value,
    /// A value kept in a name the scan follows to where it is used: the
    /// initializer or assignment of a variable no other script imports, a
    /// parameter's default, the argument of a function the script calls
    /// by name (REG-032).
    Kept,
    /// A value used up where it stands: an operand, a test, a key, a
    /// statement's expression, a callee, a callback a browser API calls.
    Operand,
    Object,
    Typeof,
    Comparison,
}

impl Pos {
    /// Whether the expression is a value at all, which a prototype may not
    /// be (REG-032).
    fn value(self) -> bool {
        matches!(self, Pos::Value | Pos::Kept | Pos::Operand)
    }

    /// Where a branch of a conditional or logical expression stands: the
    /// value it yields stands where the whole expression does, but the
    /// global object never reaches a receiver, a `typeof` or a comparison
    /// through a branch.
    fn branch(self) -> Pos {
        match self {
            Pos::Value => Pos::Value,
            Pos::Kept => Pos::Kept,
            _ => Pos::Operand,
        }
    }
}

/// What a value is traced back to (REG-032).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Root {
    /// A built-in object or function, or a member one hands over:
    /// `Object`, `window.JSON`, `Object.keys`, `Array.prototype.slice`.
    BuiltIn,
    /// A built-in, or a method a value the script did not make inherits
    /// (`[].slice`, an element's `addEventListener`). Only a write is
    /// checked against this: the same names are data on the objects a
    /// script passes around (`result.error`, `page.next`), so reading one
    /// stays admitted.
    Method,
    /// `document`, `location` or `history`: the page itself.
    Page,
}

/// How far from a prototype an expression may stand and still count as
/// one (REG-032).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reach {
    /// The prototype itself: what a script may not keep or pass on.
    Itself,
    /// The prototype or a value read from it: what a write may not change.
    Through,
}

/// The facts a component gives the script scan.
pub(super) struct Context<'c> {
    pub file: String,
    pub line_starts: Vec<usize>,
    pub element_prefix: String,
    pub elements: Option<&'c [String]>,
    pub own_scripts: BTreeSet<String>,
    pub own_directory: String,
    pub importable_scripts: BTreeSet<String>,
}

pub(super) struct Walker<'a, 'c> {
    pub phase: Phase,
    pub facts: Facts<'a>,
    pub findings: Vec<Finding>,
    context: &'c Context<'c>,
    scopes: Vec<HashMap<&'a str, Bid>>,
    next_id: usize,
    this_is_instance: Vec<bool>,
    classes: Vec<&'a Class<'a>>,
    depth: usize,
}

fn unparen<'b, 'a>(expr: &'b Expression<'a>) -> &'b Expression<'a> {
    match expr {
        Expression::ParenthesizedExpression(inner) => unparen(&inner.expression),
        other => other,
    }
}

impl<'a, 'c> Walker<'a, 'c> {
    pub(super) fn new(phase: Phase, facts: Facts<'a>, context: &'c Context<'c>) -> Self {
        Walker {
            phase,
            facts,
            findings: Vec::new(),
            context,
            scopes: Vec::new(),
            next_id: 0,
            this_is_instance: vec![false],
            classes: Vec::new(),
            depth: 0,
        }
    }

    fn check(&self) -> bool {
        self.phase == Phase::Check
    }

    fn line(&self, span: Span) -> u32 {
        let offset = span.start as usize;
        let line = self
            .context
            .line_starts
            .partition_point(|start| *start <= offset);
        u32::try_from(line.max(1)).unwrap_or(u32::MAX)
    }

    fn refuse(&mut self, check: &'static str, span: Span, message: String) {
        if !self.check() {
            return;
        }
        let finding = Finding {
            check,
            file: self.context.file.clone(),
            line: Some(self.line(span)),
            message,
        };
        if !self.findings.contains(&finding) {
            self.findings.push(finding);
        }
    }

    // ----- scopes -----------------------------------------------------

    fn declare(&mut self, name: &'a str, kind: Kind) -> Bid {
        if let Some(scope) = self.scopes.last()
            && let Some(existing) = scope.get(name)
        {
            return *existing;
        }
        let id = self.next_id;
        self.next_id += 1;
        if self.phase == Phase::Collect {
            self.facts.bindings.push(Binding {
                kind,
                init: None,
                function: matches!(kind, Kind::Function),
                class: None,
                param_of: None,
                param_index: 0,
                param_default: None,
                assignments: Vec::new(),
                opaque: false,
                numeric_updates: false,
                calls: Vec::new(),
                escapes: false,
                exported: false,
                declaration: None,
            });
        }
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name, id);
        }
        id
    }

    fn binding_mut(&mut self, id: Bid) -> Option<&mut Binding<'a>> {
        if self.phase == Phase::Collect {
            self.facts.bindings.get_mut(id)
        } else {
            None
        }
    }

    fn binding(&self, id: Bid) -> Option<&Binding<'a>> {
        self.facts.bindings.get(id)
    }

    fn lookup(&self, name: &str) -> Option<Bid> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
    }

    /// Resolves a name where the walk stands, and records the binding it
    /// names there for [`Self::bound`].
    fn resolve(&mut self, reference: &IdentifierReference<'a>) -> Option<Bid> {
        let id = self.lookup(reference.name.as_str());
        if self.phase == Phase::Collect {
            self.facts.resolved.insert(reference.span.start, id);
        }
        id
    }

    /// The binding a name had where it is written, which is what a value
    /// followed from an initializer, an assignment or a call's argument
    /// must use: the name may be shadowed where the value is used. A name
    /// the walk has not reached yet, which only the first phase meets,
    /// resolves where the walk stands.
    fn bound(&self, reference: &IdentifierReference<'a>) -> Option<Bid> {
        match self.facts.resolved.get(&reference.span.start) {
            Some(id) => *id,
            None => self.lookup(reference.name.as_str()),
        }
    }

    fn declare_pattern(
        &mut self,
        pattern: &'a BindingPattern<'a>,
        kind: Kind,
        init: Option<&'a Expression<'a>>,
    ) {
        match pattern {
            BindingPattern::BindingIdentifier(identifier) => {
                let id = self.declare(identifier.name.as_str(), kind);
                if let Some(binding) = self.binding_mut(id) {
                    match (binding.init, init) {
                        (None, Some(init)) if binding.assignments.is_empty() => {
                            binding.init = Some(init)
                        }
                        (_, Some(init)) => binding.assignments.push(init),
                        _ => {}
                    }
                    if let Some(init) = init {
                        let init = unparen(init);
                        if matches!(
                            init,
                            Expression::FunctionExpression(_)
                                | Expression::ArrowFunctionExpression(_)
                        ) {
                            binding.function = true;
                        }
                        if let Expression::ClassExpression(class) = init {
                            binding.class = Some(class);
                        }
                    }
                }
            }
            BindingPattern::ObjectPattern(object) => {
                for property in &object.properties {
                    self.declare_opaque(&property.value, kind);
                }
                if let Some(rest) = &object.rest {
                    self.declare_opaque(&rest.argument, kind);
                }
            }
            BindingPattern::ArrayPattern(array) => {
                for element in array.elements.iter().flatten() {
                    self.declare_opaque(element, kind);
                }
                if let Some(rest) = &array.rest {
                    self.declare_opaque(&rest.argument, kind);
                }
            }
            BindingPattern::AssignmentPattern(assignment) => {
                self.declare_opaque(&assignment.left, kind)
            }
        }
    }

    fn declare_opaque(&mut self, pattern: &'a BindingPattern<'a>, kind: Kind) {
        let mut names = Vec::new();
        pattern_names(pattern, &mut names);
        for name in names {
            let id = self.declare(name, kind);
            if let Some(binding) = self.binding_mut(id) {
                binding.opaque = true;
            }
        }
    }

    /// Declares the `var` bindings of a function body, which belong to the
    /// function wherever they are written.
    fn hoist_vars(&mut self, statements: &'a [Statement<'a>]) {
        for statement in statements {
            self.hoist_vars_in(statement);
        }
    }

    fn hoist_vars_in(&mut self, statement: &'a Statement<'a>) {
        match statement {
            Statement::VariableDeclaration(declaration)
                if declaration.kind == VariableDeclarationKind::Var =>
            {
                for declarator in &declaration.declarations {
                    self.declare_pattern(&declarator.id, Kind::Var, declarator.init.as_ref());
                }
            }
            Statement::BlockStatement(block) => self.hoist_vars(&block.body),
            Statement::IfStatement(statement) => {
                self.hoist_vars_in(&statement.consequent);
                if let Some(alternate) = &statement.alternate {
                    self.hoist_vars_in(alternate);
                }
            }
            Statement::ForStatement(statement) => {
                if let Some(ForStatementInit::VariableDeclaration(declaration)) = &statement.init
                    && declaration.kind == VariableDeclarationKind::Var
                {
                    for declarator in &declaration.declarations {
                        self.declare_pattern(&declarator.id, Kind::Var, declarator.init.as_ref());
                    }
                }
                self.hoist_vars_in(&statement.body);
            }
            Statement::ForInStatement(statement) => {
                if let ForStatementLeft::VariableDeclaration(declaration) = &statement.left
                    && declaration.kind == VariableDeclarationKind::Var
                {
                    for declarator in &declaration.declarations {
                        self.declare_opaque(&declarator.id, Kind::Var);
                    }
                }
                self.hoist_vars_in(&statement.body);
            }
            Statement::ForOfStatement(statement) => {
                if let ForStatementLeft::VariableDeclaration(declaration) = &statement.left
                    && declaration.kind == VariableDeclarationKind::Var
                {
                    for declarator in &declaration.declarations {
                        self.declare_opaque(&declarator.id, Kind::Var);
                    }
                }
                self.hoist_vars_in(&statement.body);
            }
            Statement::WhileStatement(statement) => self.hoist_vars_in(&statement.body),
            Statement::DoWhileStatement(statement) => self.hoist_vars_in(&statement.body),
            Statement::LabeledStatement(statement) => self.hoist_vars_in(&statement.body),
            Statement::TryStatement(statement) => {
                self.hoist_vars(&statement.block.body);
                if let Some(handler) = &statement.handler {
                    self.hoist_vars(&handler.body.body);
                }
                if let Some(finalizer) = &statement.finalizer {
                    self.hoist_vars(&finalizer.body);
                }
            }
            Statement::SwitchStatement(statement) => {
                for case in &statement.cases {
                    self.hoist_vars(&case.consequent);
                }
            }
            Statement::ExportNamedDeclaration(export) => {
                if let Some(Declaration::VariableDeclaration(declaration)) = &export.declaration
                    && declaration.kind == VariableDeclarationKind::Var
                {
                    for declarator in &declaration.declarations {
                        self.declare_pattern(&declarator.id, Kind::Var, declarator.init.as_ref());
                    }
                }
            }
            _ => {}
        }
    }

    /// Declares the lexical bindings and function declarations a block
    /// holds directly.
    fn hoist_lexical(&mut self, statements: &'a [Statement<'a>]) {
        for statement in statements {
            let declaration = match statement {
                Statement::ExportNamedDeclaration(export) => export.declaration.as_ref(),
                _ => statement.as_declaration(),
            };
            if let Some(declaration) = declaration {
                self.hoist_declaration(declaration);
            }
            if let Statement::ExportDefaultDeclaration(export) = statement {
                match &export.declaration {
                    ExportDefaultDeclarationKind::FunctionDeclaration(function) => {
                        if let Some(id) = &function.id {
                            let bid = self.declare(id.name.as_str(), Kind::Function);
                            if let Some(binding) = self.binding_mut(bid) {
                                binding.declaration = Some(function);
                            }
                        }
                    }
                    ExportDefaultDeclarationKind::ClassDeclaration(class) => {
                        if let Some(id) = &class.id {
                            let bid = self.declare(id.name.as_str(), Kind::Class);
                            if let Some(binding) = self.binding_mut(bid) {
                                binding.class = Some(class);
                            }
                        }
                    }
                    _ => {}
                }
            }
            if let Statement::ImportDeclaration(import) = statement {
                for specifier in import.specifiers.iter().flatten() {
                    let local = match specifier {
                        ImportDeclarationSpecifier::ImportSpecifier(specifier) => &specifier.local,
                        ImportDeclarationSpecifier::ImportDefaultSpecifier(specifier) => {
                            &specifier.local
                        }
                        ImportDeclarationSpecifier::ImportNamespaceSpecifier(specifier) => {
                            &specifier.local
                        }
                    };
                    self.declare(local.name.as_str(), Kind::Import);
                }
            }
        }
    }

    fn hoist_declaration(&mut self, declaration: &'a Declaration<'a>) {
        match declaration {
            Declaration::VariableDeclaration(variables)
                if variables.kind != VariableDeclarationKind::Var =>
            {
                let kind = if variables.kind == VariableDeclarationKind::Const {
                    Kind::Const
                } else {
                    Kind::Let
                };
                for declarator in &variables.declarations {
                    self.declare_pattern(&declarator.id, kind, declarator.init.as_ref());
                }
            }
            Declaration::FunctionDeclaration(function) => {
                if let Some(id) = &function.id {
                    let bid = self.declare(id.name.as_str(), Kind::Function);
                    if let Some(binding) = self.binding_mut(bid) {
                        binding.declaration = Some(function);
                    }
                }
            }
            Declaration::ClassDeclaration(class) => {
                if let Some(id) = &class.id {
                    let bid = self.declare(id.name.as_str(), Kind::Class);
                    if let Some(binding) = self.binding_mut(bid) {
                        binding.class = Some(class);
                    }
                }
            }
            _ => {}
        }
    }

    // ----- program and statements --------------------------------------

    pub(super) fn program(&mut self, program: &'a Program<'a>) {
        self.scopes.push(HashMap::new());
        self.hoist_vars(&program.body);
        self.hoist_lexical(&program.body);
        for statement in &program.body {
            self.statement(statement);
        }
        self.scopes.pop();
    }

    fn block(&mut self, statements: &'a [Statement<'a>]) {
        self.scopes.push(HashMap::new());
        self.hoist_lexical(statements);
        for statement in statements {
            self.statement(statement);
        }
        self.scopes.pop();
    }

    fn statement(&mut self, statement: &'a Statement<'a>) {
        if !self.enter(statement.span()) {
            return;
        }
        match statement {
            Statement::BlockStatement(block) => self.block(&block.body),
            Statement::BreakStatement(_)
            | Statement::ContinueStatement(_)
            | Statement::DebuggerStatement(_)
            | Statement::EmptyStatement(_) => {}
            Statement::ExpressionStatement(statement) => {
                self.expr(&statement.expression, Pos::Operand);
            }
            Statement::DoWhileStatement(statement) => {
                self.statement(&statement.body);
                self.expr(&statement.test, Pos::Operand);
            }
            Statement::WhileStatement(statement) => {
                self.expr(&statement.test, Pos::Operand);
                self.statement(&statement.body);
            }
            Statement::ForStatement(statement) => {
                self.scopes.push(HashMap::new());
                match &statement.init {
                    Some(ForStatementInit::VariableDeclaration(declaration)) => {
                        if declaration.kind != VariableDeclarationKind::Var {
                            let kind = if declaration.kind == VariableDeclarationKind::Const {
                                Kind::Const
                            } else {
                                Kind::Let
                            };
                            for declarator in &declaration.declarations {
                                self.declare_pattern(
                                    &declarator.id,
                                    kind,
                                    declarator.init.as_ref(),
                                );
                            }
                        }
                        self.variable_declaration(declaration);
                    }
                    Some(init) => {
                        if let Some(expression) = init.as_expression() {
                            self.expr(expression, Pos::Operand);
                        }
                    }
                    None => {}
                }
                if let Some(test) = &statement.test {
                    self.expr(test, Pos::Operand);
                }
                if let Some(update) = &statement.update {
                    self.expr(update, Pos::Operand);
                }
                self.statement(&statement.body);
                self.scopes.pop();
            }
            Statement::ForInStatement(statement) => {
                self.expr(&statement.right, Pos::Operand);
                self.scopes.push(HashMap::new());
                self.for_left(&statement.left);
                self.statement(&statement.body);
                self.scopes.pop();
            }
            Statement::ForOfStatement(statement) => {
                self.expr(&statement.right, Pos::Operand);
                self.scopes.push(HashMap::new());
                self.for_left(&statement.left);
                self.statement(&statement.body);
                self.scopes.pop();
            }
            Statement::IfStatement(statement) => {
                self.expr(&statement.test, Pos::Operand);
                self.statement(&statement.consequent);
                if let Some(alternate) = &statement.alternate {
                    self.statement(alternate);
                }
            }
            Statement::LabeledStatement(statement) => self.statement(&statement.body),
            Statement::ReturnStatement(statement) => {
                if let Some(argument) = &statement.argument {
                    self.expr(argument, Pos::Value);
                }
            }
            Statement::SwitchStatement(statement) => {
                self.expr(&statement.discriminant, Pos::Operand);
                self.scopes.push(HashMap::new());
                for case in &statement.cases {
                    self.hoist_lexical(&case.consequent);
                }
                for case in &statement.cases {
                    if let Some(test) = &case.test {
                        self.expr(test, Pos::Operand);
                    }
                    for statement in &case.consequent {
                        self.statement(statement);
                    }
                }
                self.scopes.pop();
            }
            Statement::ThrowStatement(statement) => self.expr(&statement.argument, Pos::Value),
            Statement::TryStatement(statement) => {
                self.block(&statement.block.body);
                if let Some(handler) = &statement.handler {
                    self.scopes.push(HashMap::new());
                    if let Some(param) = &handler.param {
                        self.declare_opaque(&param.pattern, Kind::Catch);
                        self.pattern_defaults(&param.pattern);
                    }
                    self.block(&handler.body.body);
                    self.scopes.pop();
                }
                if let Some(finalizer) = &statement.finalizer {
                    self.block(&finalizer.body);
                }
            }
            Statement::WithStatement(statement) => {
                self.refuse(
                    "script-construct",
                    statement.span,
                    "a `with` statement".to_string(),
                );
            }
            Statement::VariableDeclaration(declaration) => self.variable_declaration(declaration),
            Statement::FunctionDeclaration(function) => {
                let owner = function
                    .id
                    .as_ref()
                    .and_then(|id| self.lookup(id.name.as_str()));
                self.function(function, owner, false);
            }
            Statement::ClassDeclaration(class) => self.class(class),
            Statement::ImportDeclaration(import) => self.import_source(&import.source),
            Statement::ExportAllDeclaration(export) => self.import_source(&export.source),
            Statement::ExportNamedDeclaration(export) => {
                if let Some(source) = &export.source {
                    self.import_source(source);
                }
                match &export.declaration {
                    Some(Declaration::VariableDeclaration(declaration)) => {
                        for declarator in &declaration.declarations {
                            let mut names = Vec::new();
                            pattern_names(&declarator.id, &mut names);
                            for name in names {
                                self.mark_exported(name);
                            }
                        }
                        self.variable_declaration(declaration)
                    }
                    Some(Declaration::FunctionDeclaration(function)) => {
                        let owner = function
                            .id
                            .as_ref()
                            .and_then(|id| self.lookup(id.name.as_str()));
                        self.function(function, owner, false);
                    }
                    Some(Declaration::ClassDeclaration(class)) => self.class(class),
                    Some(_) => self.refuse(
                        "script-construct",
                        export.span,
                        "a TypeScript declaration".to_string(),
                    ),
                    None => {
                        for specifier in &export.specifiers {
                            if let ModuleExportName::IdentifierReference(reference) =
                                &specifier.local
                            {
                                if export.source.is_none() {
                                    self.mark_exported(reference.name.as_str());
                                }
                                self.identifier(reference, Pos::Value);
                            }
                        }
                    }
                }
            }
            Statement::ExportDefaultDeclaration(export) => match &export.declaration {
                ExportDefaultDeclarationKind::FunctionDeclaration(function) => {
                    let owner = function
                        .id
                        .as_ref()
                        .and_then(|id| self.lookup(id.name.as_str()));
                    self.function(function, owner, false);
                }
                ExportDefaultDeclarationKind::ClassDeclaration(class) => self.class(class),
                other => match other.as_expression() {
                    Some(expression) => self.expr(expression, Pos::Value),
                    None => self.refuse(
                        "script-construct",
                        export.span,
                        "a TypeScript declaration".to_string(),
                    ),
                },
            },
            _ => self.refuse(
                "script-construct",
                statement.span(),
                "a statement the scan cannot classify".to_string(),
            ),
        }
        self.leave();
    }

    fn for_left(&mut self, left: &'a ForStatementLeft<'a>) {
        match left {
            ForStatementLeft::VariableDeclaration(declaration) => {
                if declaration.kind != VariableDeclarationKind::Var {
                    let kind = if declaration.kind == VariableDeclarationKind::Const {
                        Kind::Const
                    } else {
                        Kind::Let
                    };
                    for declarator in &declaration.declarations {
                        self.declare_opaque(&declarator.id, kind);
                    }
                }
                for declarator in &declaration.declarations {
                    self.pattern_defaults(&declarator.id);
                }
            }
            other => {
                if let Some(target) = other.as_assignment_target() {
                    self.assignment_target(target, None, true);
                }
            }
        }
    }

    fn variable_declaration(&mut self, declaration: &'a VariableDeclaration<'a>) {
        for declarator in &declaration.declarations {
            self.pattern_defaults(&declarator.id);
            if let Some(init) = &declarator.init {
                let owner = match &declarator.id {
                    BindingPattern::BindingIdentifier(identifier) => {
                        self.lookup(identifier.name.as_str())
                    }
                    _ => None,
                };
                let pos = self.kept_in(owner);
                self.value_with_owner(init, owner, pos);
            }
        }
    }

    /// Where a value written to a name stands: kept, when the name is a
    /// binding of the script's that no other script imports, so the scan
    /// follows the value to where the name is used; a value, otherwise
    /// (REG-032).
    fn kept_in(&self, owner: Option<Bid>) -> Pos {
        match owner.and_then(|id| self.binding(id)) {
            Some(binding) if !binding.exported => Pos::Kept,
            _ => Pos::Value,
        }
    }

    /// Walks an initializer; a function or arrow assigned to a binding has
    /// that binding as its owner, so its parameters can be traced to its
    /// call sites.
    fn value_with_owner(&mut self, init: &'a Expression<'a>, owner: Option<Bid>, pos: Pos) {
        match unparen(init) {
            Expression::FunctionExpression(function) => self.function(function, owner, false),
            Expression::ArrowFunctionExpression(arrow) => self.arrow(arrow, owner),
            _ => self.expr(init, pos),
        }
    }

    fn pattern_defaults(&mut self, pattern: &'a BindingPattern<'a>) {
        match pattern {
            BindingPattern::BindingIdentifier(_) => {}
            BindingPattern::ObjectPattern(object) => {
                for property in &object.properties {
                    if property.computed
                        && let Some(key) = property.key.as_expression()
                    {
                        self.computed_key(key, property.span);
                    } else if let Some(name) = property.key.static_name() {
                        self.property_name(&name, property.span, false);
                    }
                    self.destructured_prototype(&property.key, property.computed, property.span);
                    self.pattern_defaults(&property.value);
                }
                if let Some(rest) = &object.rest {
                    self.pattern_defaults(&rest.argument);
                }
            }
            BindingPattern::ArrayPattern(array) => {
                for element in array.elements.iter().flatten() {
                    self.pattern_defaults(element);
                }
                if let Some(rest) = &array.rest {
                    self.pattern_defaults(&rest.argument);
                }
            }
            BindingPattern::AssignmentPattern(assignment) => {
                self.pattern_defaults(&assignment.left);
                self.expr(&assignment.right, Pos::Value);
            }
        }
    }

    // ----- functions and classes ----------------------------------------

    fn params(&mut self, params: &'a FormalParameters<'a>, owner: Option<Bid>) {
        for (index, param) in params.items.iter().enumerate() {
            match &param.pattern {
                BindingPattern::BindingIdentifier(identifier) => {
                    let id = self.declare(identifier.name.as_str(), Kind::Param);
                    let default = param.initializer.as_deref();
                    if let Some(binding) = self.binding_mut(id) {
                        binding.param_of = owner;
                        binding.param_index = index;
                        binding.param_default = default;
                    }
                }
                other => self.declare_opaque(other, Kind::Param),
            }
            self.pattern_defaults(&param.pattern);
            if let Some(initializer) = &param.initializer {
                let pos = if matches!(param.pattern, BindingPattern::BindingIdentifier(_)) {
                    Pos::Kept
                } else {
                    Pos::Value
                };
                self.expr(initializer, pos);
            }
        }
        if let Some(rest) = &params.rest {
            self.declare_opaque(&rest.rest.argument, Kind::Param);
            self.pattern_defaults(&rest.rest.argument);
        }
    }

    fn function(&mut self, function: &'a Function<'a>, owner: Option<Bid>, method: bool) {
        self.this_is_instance.push(method);
        self.scopes.push(HashMap::new());
        if function.r#type == FunctionType::FunctionExpression
            && let Some(id) = &function.id
        {
            let bid = self.declare(id.name.as_str(), Kind::Function);
            if let Some(binding) = self.binding_mut(bid) {
                binding.function = true;
            }
        }
        let arguments = self.declare("arguments", Kind::Implicit);
        if self.phase == Phase::Collect {
            self.facts
                .arguments_of
                .insert(function.span.start, arguments);
        }
        self.params(&function.params, owner);
        if let Some(body) = &function.body {
            self.hoist_vars(&body.statements);
            self.hoist_lexical(&body.statements);
            for statement in &body.statements {
                self.statement(statement);
            }
        }
        self.scopes.pop();
        self.this_is_instance.pop();
    }

    fn arrow(&mut self, arrow: &'a ArrowFunctionExpression<'a>, owner: Option<Bid>) {
        let inherited = self.this_is_instance.last().copied().unwrap_or(false);
        self.this_is_instance.push(inherited);
        self.scopes.push(HashMap::new());
        self.params(&arrow.params, owner);
        self.hoist_vars(&arrow.body.statements);
        self.hoist_lexical(&arrow.body.statements);
        match arrow.body.statements.first() {
            // `() => value` returns the value, which leaves as a `return`'s
            // does.
            Some(Statement::ExpressionStatement(statement)) if arrow.expression => {
                self.expr(&statement.expression, Pos::Value);
            }
            _ => {
                for statement in &arrow.body.statements {
                    self.statement(statement);
                }
            }
        }
        self.scopes.pop();
        self.this_is_instance.pop();
    }

    fn class(&mut self, class: &'a Class<'a>) {
        self.classes.push(class);
        self.class_body(class);
        self.classes.pop();
    }

    fn class_body(&mut self, class: &'a Class<'a>) {
        if !class.decorators.is_empty() {
            self.refuse(
                "script-construct",
                class.span,
                "a class decorator".to_string(),
            );
        }
        if let Some(super_class) = &class.super_class {
            self.constructor_target(super_class, class.span);
        }
        self.scopes.push(HashMap::new());
        if class.r#type == ClassType::ClassExpression
            && let Some(id) = &class.id
        {
            let bid = self.declare(id.name.as_str(), Kind::Class);
            if let Some(binding) = self.binding_mut(bid) {
                binding.class = Some(class);
            }
        }
        if self.phase == Phase::Collect {
            for element in &class.body.body {
                if let Some(name) = element.property_key().and_then(PropertyKey::static_name) {
                    self.class_member(class, name.to_string());
                }
            }
        }
        for element in &class.body.body {
            match element {
                ClassElement::MethodDefinition(method) => {
                    self.class_key(&method.key, method.computed, method.span);
                    if self.phase == Phase::Collect {
                        let callable = matches!(method.kind, MethodDefinitionKind::Method);
                        self.record_member_name(&method.key, callable);
                    }
                    self.function(&method.value, None, true);
                }
                ClassElement::PropertyDefinition(property) => {
                    self.class_key(&property.key, property.computed, property.span);
                    if let Some(value) = &property.value {
                        if self.phase == Phase::Collect {
                            self.record_member_name(&property.key, is_function_expression(value));
                        }
                        self.implicitly_called(&property.key, value, property.span);
                        self.this_is_instance.push(true);
                        self.expr(value, Pos::Value);
                        self.this_is_instance.pop();
                    }
                }
                ClassElement::AccessorProperty(accessor) => {
                    self.class_key(&accessor.key, accessor.computed, accessor.span);
                    if self.phase == Phase::Collect {
                        self.record_member_name(&accessor.key, false);
                    }
                    if let Some(value) = &accessor.value {
                        self.this_is_instance.push(true);
                        self.expr(value, Pos::Value);
                        self.this_is_instance.pop();
                    }
                }
                ClassElement::StaticBlock(block) => {
                    self.this_is_instance.push(true);
                    self.block(&block.body);
                    self.this_is_instance.pop();
                }
                ClassElement::TSIndexSignature(signature) => {
                    self.refuse(
                        "script-construct",
                        signature.span,
                        "a TypeScript index signature".to_string(),
                    );
                }
            }
        }
        self.scopes.pop();
    }

    fn class_key(&mut self, key: &'a PropertyKey<'a>, computed: bool, span: Span) {
        if computed {
            if let Some(expression) = key.as_expression() {
                self.expr(expression, Pos::Operand);
            }
        } else if let Some(name) = key.static_name()
            && name == "__proto__"
        {
            self.refuse(
                "script-property",
                span,
                "`__proto__` changes an object's prototype".to_string(),
            );
        }
    }

    fn record_member_name(&mut self, key: &'a PropertyKey<'a>, callable: bool) {
        match key {
            PropertyKey::PrivateIdentifier(private) => {
                if callable {
                    self.facts.private_methods.insert(private.name.as_str());
                } else {
                    self.facts.tainted.insert(private.name.as_str());
                }
            }
            PropertyKey::StaticIdentifier(identifier) => {
                if callable {
                    self.instance_method(identifier.name.as_str());
                } else {
                    self.facts.tainted.insert(identifier.name.as_str());
                }
            }
            _ => {}
        }
    }

    /// Records a member name a class gives its instances or itself, which
    /// is also a name the script defines.
    fn class_member(&mut self, class: &Class<'a>, name: String) {
        self.facts.defined_names.insert(name.clone());
        self.facts
            .class_members
            .entry(class.span.start)
            .or_default()
            .insert(name);
    }

    /// Records that another script can import the binding a name has here.
    fn mark_exported(&mut self, name: &str) {
        if self.phase != Phase::Collect {
            return;
        }
        if let Some(id) = self.lookup(name)
            && let Some(binding) = self.facts.bindings.get_mut(id)
        {
            binding.exported = true;
        }
    }

    /// Records a method of the class being walked.
    fn instance_method(&mut self, name: &'a str) {
        if let Some(class) = self.classes.last() {
            self.facts
                .instance_methods
                .entry(class.span.start)
                .or_default()
                .insert(name);
        }
    }

    /// Whether a class the script defines carries a method.
    fn class_defines(&self, class: &Class<'a>, name: &str) -> bool {
        self.facts
            .instance_methods
            .get(&class.span.start)
            .is_some_and(|methods| methods.contains(name))
    }

    /// A value stored under a name the browser calls on its own must be a
    /// function the script defines.
    fn implicitly_called(
        &mut self,
        key: &'a PropertyKey<'a>,
        value: &'a Expression<'a>,
        span: Span,
    ) {
        let Some(name) = key.static_name() else {
            return;
        };
        if IMPLICITLY_CALLED.contains(&name.as_ref()) && !self.callback_safe(value, 0) {
            self.refuse(
                "script-call",
                span,
                format!("`{name}` is called by the browser itself, so its value must be a function the script defines"),
            );
        }
    }

    // ----- expressions -------------------------------------------------

    fn enter(&mut self, span: Span) -> bool {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            self.depth -= 1;
            self.refuse(
                "script-limit",
                span,
                format!("nesting deeper than {MAX_DEPTH} levels"),
            );
            return false;
        }
        true
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    fn identifier(&mut self, reference: &'a IdentifierReference<'a>, pos: Pos) {
        let name = reference.name.as_str();
        match self.resolve(reference) {
            Some(id) => {
                if self.phase == Phase::Collect
                    && let Some(binding) = self.facts.bindings.get_mut(id)
                {
                    binding.escapes = true;
                }
            }
            None => self.global(name, reference.span, pos),
        }
    }

    fn global(&mut self, name: &str, span: Span, pos: Pos) {
        if GLOBAL_OBJECTS.contains(&name) {
            if !matches!(pos, Pos::Object | Pos::Typeof | Pos::Comparison) {
                self.refuse(
                    "script-global",
                    span,
                    format!("`{name}` is the global object; a script may only read an admitted global from it"),
                );
            }
            return;
        }
        if name == "eval" || name == "Function" {
            self.refuse(
                "script-eval",
                span,
                format!("`{name}` compiles a string into code"),
            );
            return;
        }
        if !ADMITTED_GLOBALS.contains(&name) {
            self.refuse(
                "script-global",
                span,
                format!("`{name}` is not a standard browser API the scan admits"),
            );
        }
    }

    fn expr(&mut self, expr: &'a Expression<'a>, pos: Pos) {
        if !self.enter(expr.span()) {
            return;
        }
        if pos.value() {
            self.held_prototype(expr);
        }
        if pos == Pos::Value {
            self.held_builtin(expr);
        }
        self.expr_inner(expr, pos);
        self.leave();
    }

    fn expr_inner(&mut self, expr: &'a Expression<'a>, pos: Pos) {
        match expr {
            Expression::BooleanLiteral(_)
            | Expression::NullLiteral(_)
            | Expression::NumericLiteral(_)
            | Expression::BigIntLiteral(_)
            | Expression::RegExpLiteral(_)
            | Expression::StringLiteral(_)
            | Expression::Super(_) => {}
            Expression::TemplateLiteral(template) => {
                for expression in &template.expressions {
                    self.expr(expression, Pos::Operand);
                }
            }
            Expression::Identifier(reference) => self.identifier(reference, pos),
            Expression::MetaProperty(_) => {}
            Expression::ThisExpression(this) => {
                if self.phase == Phase::Collect
                    && self.this_is_instance.last().copied().unwrap_or(false)
                    && let Some(class) = self.classes.last()
                {
                    self.facts
                        .this_class
                        .insert(this.span.start, class.span.start);
                }
                if !self.this_is_instance.last().copied().unwrap_or(false) {
                    self.refuse(
                        "script-this",
                        this.span,
                        "`this` outside a class is the global object when the browser runs the file as a classic script".to_string(),
                    );
                }
            }
            Expression::ArrayExpression(array) => {
                for element in &array.elements {
                    match element {
                        ArrayExpressionElement::SpreadElement(spread) => {
                            self.expr(&spread.argument, Pos::Value)
                        }
                        ArrayExpressionElement::Elision(_) => {}
                        other => {
                            if let Some(expression) = other.as_expression() {
                                self.expr(expression, Pos::Value);
                            }
                        }
                    }
                }
            }
            Expression::ArrowFunctionExpression(arrow) => self.arrow(arrow, None),
            Expression::FunctionExpression(function) => self.function(function, None, false),
            Expression::ClassExpression(class) => self.class(class),
            Expression::AssignmentExpression(assignment) => self.assignment(assignment),
            Expression::AwaitExpression(await_expr) => self.expr(&await_expr.argument, Pos::Value),
            Expression::BinaryExpression(binary) => {
                let comparison = matches!(
                    binary.operator,
                    BinaryOperator::Equality
                        | BinaryOperator::Inequality
                        | BinaryOperator::StrictEquality
                        | BinaryOperator::StrictInequality
                        | BinaryOperator::Instanceof
                        | BinaryOperator::In
                );
                let side = if comparison {
                    Pos::Comparison
                } else {
                    Pos::Operand
                };
                self.expr(&binary.left, side);
                self.expr(&binary.right, side);
            }
            Expression::PrivateInExpression(private) => self.expr(&private.right, Pos::Comparison),
            Expression::LogicalExpression(logical) => {
                self.expr(&logical.left, pos.branch());
                self.expr(&logical.right, pos.branch());
            }
            Expression::ConditionalExpression(conditional) => {
                self.expr(&conditional.test, Pos::Operand);
                self.expr(&conditional.consequent, pos.branch());
                self.expr(&conditional.alternate, pos.branch());
            }
            Expression::CallExpression(call) => self.call(call),
            Expression::ChainExpression(chain) => match &chain.expression {
                ChainElement::CallExpression(call) => self.call(call),
                ChainElement::TSNonNullExpression(non_null) => {
                    self.refuse(
                        "script-construct",
                        non_null.span,
                        "TypeScript syntax".to_string(),
                    );
                }
                other => {
                    if let Some(member) = other.as_member_expression() {
                        self.member(member, pos);
                    }
                }
            },
            Expression::ImportExpression(import) => {
                self.refuse(
                    "script-import",
                    import.span,
                    "a dynamic `import()` loads code the scan cannot read".to_string(),
                );
            }
            Expression::NewExpression(new) => self.new_expression(new),
            Expression::ObjectExpression(object) => self.object(object),
            Expression::ParenthesizedExpression(inner) => self.expr(&inner.expression, pos),
            Expression::SequenceExpression(sequence) => {
                let last = sequence.expressions.len().saturating_sub(1);
                for (index, expression) in sequence.expressions.iter().enumerate() {
                    self.expr(expression, if index == last { pos } else { Pos::Operand });
                }
            }
            Expression::TaggedTemplateExpression(tagged) => {
                if !self.callback_safe(&tagged.tag, 0) {
                    self.refuse(
                        "script-call",
                        tagged.span,
                        "a tagged template calls its tag; the tag must be a function the script defines".to_string(),
                    );
                }
                self.callee_walk(&tagged.tag);
                for expression in &tagged.quasi.expressions {
                    self.expr(expression, Pos::Value);
                }
            }
            Expression::UnaryExpression(unary) => {
                if unary.operator == UnaryOperator::Delete {
                    self.member_delete(unary);
                }
                let inner = if unary.operator == UnaryOperator::Typeof {
                    Pos::Typeof
                } else {
                    Pos::Operand
                };
                self.expr(&unary.argument, inner);
            }
            Expression::UpdateExpression(update) => {
                self.simple_target(&update.argument, None, false, true);
            }
            Expression::YieldExpression(yield_expr) => {
                if let Some(argument) = &yield_expr.argument {
                    self.expr(argument, Pos::Value);
                }
            }
            Expression::ComputedMemberExpression(_)
            | Expression::StaticMemberExpression(_)
            | Expression::PrivateFieldExpression(_) => {
                if let Some(member) = expr.as_member_expression() {
                    self.member(member, pos);
                }
            }
            _ => self.refuse(
                "script-construct",
                expr.span(),
                "an expression the scan cannot classify".to_string(),
            ),
        }
    }

    fn object(&mut self, object: &'a ObjectExpression<'a>) {
        for property in &object.properties {
            match property {
                ObjectPropertyKind::ObjectProperty(property) => {
                    if self.phase == Phase::Collect
                        && !property.computed
                        && let Some(name) = property.key.static_name()
                    {
                        self.facts.defined_names.insert(name.to_string());
                    }
                    if property.computed {
                        if let Some(key) = property.key.as_expression() {
                            self.expr(key, Pos::Operand);
                        }
                    } else if let Some(name) = property.key.static_name()
                        && name == "__proto__"
                        && !property.shorthand
                        && !property.method
                    {
                        self.refuse(
                            "script-property",
                            property.span,
                            "`__proto__` changes an object's prototype".to_string(),
                        );
                    }
                    if property.kind == PropertyKind::Init {
                        self.implicitly_called(&property.key, &property.value, property.span);
                    }
                    match &property.value {
                        Expression::FunctionExpression(function) => {
                            self.function(function, None, false);
                        }
                        value => self.expr(value, Pos::Value),
                    }
                }
                ObjectPropertyKind::SpreadProperty(spread) => {
                    self.expr(&spread.argument, Pos::Value)
                }
            }
        }
    }

    // ----- members ----------------------------------------------------

    /// The name a member expression reads, when the scan can know it: a
    /// static name, or a computed key that traces to one constant.
    fn member_name(&mut self, member: &'a MemberExpression<'a>) -> Option<String> {
        match member {
            MemberExpression::StaticMemberExpression(member) => {
                Some(member.property.name.to_string())
            }
            MemberExpression::PrivateFieldExpression(member) => {
                Some(format!("#{}", member.field.name))
            }
            MemberExpression::ComputedMemberExpression(member) => {
                let values = self.trace(&member.expression, 0)?;
                if values.len() == 1 {
                    values.into_iter().next()
                } else {
                    None
                }
            }
        }
    }

    /// Whether an expression may evaluate to the global object: one of its
    /// names the script does not declare, or a sequence, conditional,
    /// logical or assignment expression that may yield one, as `(0,
    /// window)` does. Every rule about a member of the global object reads
    /// through these, so `(0, window).localStorage` is checked as
    /// `window.localStorage` is (REG-032). Past [`MAX_TRACE_DEPTH`] the
    /// answer is yes, which only adds checks.
    fn is_global_object(&self, expr: &Expression<'a>) -> bool {
        self.global_object_within(expr, 0)
    }

    /// [`Self::is_global_object`] at a nesting depth.
    fn global_object_within(&self, expr: &Expression<'a>, depth: usize) -> bool {
        if depth > MAX_TRACE_DEPTH {
            return true;
        }
        match unparen(expr) {
            Expression::SequenceExpression(sequence) => sequence
                .expressions
                .last()
                .is_some_and(|last| self.global_object_within(last, depth + 1)),
            Expression::ConditionalExpression(conditional) => {
                self.global_object_within(&conditional.consequent, depth + 1)
                    || self.global_object_within(&conditional.alternate, depth + 1)
            }
            Expression::LogicalExpression(logical) => {
                self.global_object_within(&logical.left, depth + 1)
                    || self.global_object_within(&logical.right, depth + 1)
            }
            Expression::AssignmentExpression(assignment) => {
                self.global_object_within(&assignment.right, depth + 1)
            }
            other => self.names_global_object(other),
        }
    }

    /// Whether an expression is one of the global object's names the
    /// script does not declare, as written.
    fn names_global_object(&self, expr: &Expression<'a>) -> bool {
        match unparen(expr) {
            Expression::Identifier(reference) => {
                GLOBAL_OBJECTS.contains(&reference.name.as_str()) && self.bound(reference).is_none()
            }
            _ => false,
        }
    }

    fn property_name(&mut self, name: &str, span: Span, write: bool) {
        if REFUSED_PROPERTIES.contains(&name) {
            let check = if name == "eval" || name == "Function" || name == "constructor" {
                "script-eval"
            } else {
                "script-property"
            };
            self.refuse(check, span, format!("the property `{name}` is refused"));
        } else if write && READ_ONLY_PROPERTIES.contains(&name) {
            self.refuse(
                "script-property",
                span,
                format!("writing `{name}` is refused"),
            );
        }
    }

    fn computed_key(&mut self, key: &'a Expression<'a>, span: Span) {
        self.expr(key, Pos::Operand);
        match self.trace(key, 0) {
            Some(values) => {
                for value in values {
                    self.property_name(&value, span, false);
                }
            }
            None if self.numeric(key, 0) => {}
            None => self.refuse(
                "script-computed",
                span,
                "a computed property whose key is neither a constant nor a number".to_string(),
            ),
        }
    }

    fn member(&mut self, member: &'a MemberExpression<'a>, _pos: Pos) {
        self.member_access(member, false);
    }

    fn member_access(&mut self, member: &'a MemberExpression<'a>, write: bool) {
        let object = member.object();
        self.expr(object, Pos::Object);
        match member {
            MemberExpression::StaticMemberExpression(static_member) => {
                let name = static_member.property.name.as_str();
                self.property_name(name, static_member.span, write);
                if self.is_global_object(object) && !write && !self.global_admitted(name) {
                    self.refuse(
                        "script-global",
                        static_member.span,
                        format!("`{name}` is not a standard browser API the scan admits"),
                    );
                }
            }
            MemberExpression::ComputedMemberExpression(computed) => {
                if self.is_global_object(object) {
                    match self.trace(&computed.expression, 0) {
                        Some(values) => {
                            self.expr(&computed.expression, Pos::Operand);
                            for value in values {
                                self.property_name(&value, computed.span, write);
                                if !write && !self.global_admitted(&value) {
                                    self.refuse(
                                        "script-global",
                                        computed.span,
                                        format!("`{value}` is not a standard browser API the scan admits"),
                                    );
                                }
                            }
                        }
                        None => {
                            self.expr(&computed.expression, Pos::Operand);
                            self.refuse(
                                "script-computed",
                                computed.span,
                                "a computed property of the global object whose key is not a constant".to_string(),
                            );
                        }
                    }
                } else {
                    self.computed_key(&computed.expression, computed.span);
                    if write && let Some(values) = self.trace(&computed.expression, 0) {
                        for value in values {
                            self.property_name(&value, computed.span, true);
                        }
                    }
                }
            }
            MemberExpression::PrivateFieldExpression(_) => {}
        }
    }

    fn global_admitted(&self, name: &str) -> bool {
        (ADMITTED_GLOBALS.contains(&name) && !GLOBAL_OBJECTS.contains(&name))
            || self.facts.written_globals.contains(name)
    }

    // ----- assignments -------------------------------------------------

    fn assignment(&mut self, assignment: &'a AssignmentExpression<'a>) {
        let simple = assignment.operator == AssignmentOperator::Assign;
        let value = if simple {
            Some(&assignment.right)
        } else {
            None
        };
        match assignment.left.as_simple_assignment_target() {
            Some(target) => {
                self.simple_target(target, value, !simple, false);
                if let Some(member) = target.as_member_expression()
                    && simple
                {
                    self.write_value(member, &assignment.right);
                }
            }
            None => self.assignment_target(&assignment.left, value, !simple),
        }
        // `a ||= b` may leave `b` in `a`. The name stays opaque to constant
        // tracing, but what it may hold is followed like an assignment's
        // value (REG-032).
        if self.phase == Phase::Collect
            && matches!(
                assignment.operator,
                AssignmentOperator::LogicalOr
                    | AssignmentOperator::LogicalAnd
                    | AssignmentOperator::LogicalNullish
            )
            && let Some(SimpleAssignmentTarget::AssignmentTargetIdentifier(identifier)) =
                assignment.left.as_simple_assignment_target()
            && let Some(id) = self.lookup(identifier.name.as_str())
            && let Some(binding) = self.facts.bindings.get_mut(id)
        {
            binding.assignments.push(&assignment.right);
        }
        let owner = match (assignment.left.as_simple_assignment_target(), simple) {
            (Some(SimpleAssignmentTarget::AssignmentTargetIdentifier(identifier)), true) => {
                self.lookup(identifier.name.as_str())
            }
            _ => None,
        };
        let pos = self.kept_in(owner);
        self.value_with_owner(&assignment.right, owner, pos);
    }

    fn assignment_target(
        &mut self,
        target: &'a AssignmentTarget<'a>,
        value: Option<&'a Expression<'a>>,
        opaque: bool,
    ) {
        if let Some(simple) = target.as_simple_assignment_target() {
            self.simple_target(simple, value, opaque, false);
            return;
        }
        // A destructuring assignment: each target receives a value the scan
        // does not follow.
        let mut targets = Vec::new();
        collect_targets(target, &mut targets);
        for target in targets {
            let simple = match target {
                Target::Simple(simple) => simple,
                Target::Shorthand(identifier) => {
                    self.identifier_target(identifier, None, true, false);
                    continue;
                }
            };
            self.simple_target(simple, None, true, false);
            if let Some(member) = simple.as_member_expression()
                && let Some(name) = self.member_name(member)
                && checked_write(&name)
            {
                self.refuse(
                    "script-property",
                    member.span(),
                    format!("a destructuring assignment writes `{name}` with a value the scan cannot check"),
                );
            }
        }
        self.destructuring_target(target);
    }

    /// Walks what a destructuring assignment's target evaluates itself, at
    /// every depth: each key gets the property checks, a `prototype` key is
    /// refused (REG-032), and each default is walked as a value. Left
    /// unwalked, a default or a nested key could hold `eval` or a
    /// prototype that no rule sees.
    fn destructuring_target(&mut self, target: &'a AssignmentTarget<'a>) {
        match target {
            AssignmentTarget::ObjectAssignmentTarget(object) => {
                for property in &object.properties {
                    match property {
                        AssignmentTargetProperty::AssignmentTargetPropertyIdentifier(shorthand) => {
                            let name = shorthand.binding.name.as_str();
                            self.property_name(name, shorthand.span, false);
                            if self.check() && prototype_name(name) {
                                self.refuse(
                                    "script-prototype",
                                    shorthand.span,
                                    DESTRUCTURED_PROTOTYPE.to_string(),
                                );
                            }
                            if let Some(init) = &shorthand.init {
                                self.expr(init, Pos::Value);
                            }
                        }
                        AssignmentTargetProperty::AssignmentTargetPropertyProperty(property) => {
                            if property.computed
                                && let Some(key) = property.name.as_expression()
                            {
                                self.computed_key(key, property.span);
                            } else if let Some(name) = property.name.static_name() {
                                self.property_name(&name, property.span, false);
                            }
                            self.destructured_prototype(
                                &property.name,
                                property.computed,
                                property.span,
                            );
                            self.destructuring_default(&property.binding);
                        }
                    }
                }
                if let Some(rest) = &object.rest {
                    self.destructuring_target(&rest.target);
                }
            }
            AssignmentTarget::ArrayAssignmentTarget(array) => {
                for element in array.elements.iter().flatten() {
                    self.destructuring_default(element);
                }
                if let Some(rest) = &array.rest {
                    self.destructuring_target(&rest.target);
                }
            }
            _ => {}
        }
    }

    /// [`Self::destructuring_target`] for a target that may carry a
    /// default.
    fn destructuring_default(&mut self, target: &'a AssignmentTargetMaybeDefault<'a>) {
        match target {
            AssignmentTargetMaybeDefault::AssignmentTargetWithDefault(with_default) => {
                self.destructuring_target(&with_default.binding);
                self.expr(&with_default.init, Pos::Value);
            }
            other => {
                if let Some(target) = other.as_assignment_target() {
                    self.destructuring_target(target);
                }
            }
        }
    }

    fn simple_target(
        &mut self,
        target: &'a SimpleAssignmentTarget<'a>,
        value: Option<&'a Expression<'a>>,
        opaque: bool,
        update: bool,
    ) {
        match target {
            SimpleAssignmentTarget::AssignmentTargetIdentifier(identifier) => {
                self.identifier_target(identifier, value, opaque, update);
            }
            other => {
                if let Some(member) = other.as_member_expression() {
                    self.member_access(member, true);
                    // Every member write reaches this arm: an assignment of
                    // any operator, `++` or `--`, a destructuring target and
                    // a `for` loop's target (REG-032).
                    if self.check() {
                        if self.prototype(member.object(), Reach::Through, 0, &mut BTreeSet::new())
                        {
                            self.refuse(
                                "script-prototype",
                                member.span(),
                                "assigning a member of a prototype changes it".to_string(),
                            );
                        } else {
                            self.builtin_write(member, member.span(), "assigning");
                        }
                    }
                    if let Some(name) = self.member_name(member) {
                        if opaque && checked_write(&name) {
                            self.refuse(
                                "script-property",
                                member.span(),
                                format!("`{name}` is written with a value the scan cannot check"),
                            );
                        }
                        if self.phase == Phase::Collect {
                            self.record_write(member, &name, value);
                        }
                    }
                } else {
                    self.refuse(
                        "script-construct",
                        other.span(),
                        "TypeScript syntax".to_string(),
                    );
                }
            }
        }
    }

    /// A write to a name: a binding the script declares records the value
    /// it receives (or that it receives one the scan does not follow), and
    /// a name it does not declare is a global, which only `location` may be
    /// assigned, and then only a URL the scan admits.
    fn identifier_target(
        &mut self,
        identifier: &'a IdentifierReference<'a>,
        value: Option<&'a Expression<'a>>,
        opaque: bool,
        update: bool,
    ) {
        let name = identifier.name.as_str();
        match self.lookup(name) {
            Some(id) => {
                if self.phase == Phase::Collect
                    && let Some(binding) = self.facts.bindings.get_mut(id)
                {
                    match (value, update) {
                        (_, true) => binding.numeric_updates = true,
                        (Some(value), false) if !opaque => binding.assignments.push(value),
                        _ => binding.opaque = true,
                    }
                }
            }
            None if name == "location" => match value {
                Some(value) if !opaque => self.url_value(value, identifier.span, "`location`"),
                _ => self.refuse(
                    "script-url",
                    identifier.span,
                    "`location` is assigned a value the scan cannot check".to_string(),
                ),
            },
            None => self.refuse(
                "script-global",
                identifier.span,
                format!("assigning `{name}`, which the script does not declare, writes a global"),
            ),
        }
    }

    fn record_write(
        &mut self,
        member: &'a MemberExpression<'a>,
        name: &str,
        value: Option<&'a Expression<'a>>,
    ) {
        // Only a write on the global object's name makes a global of the
        // script's own, which later reads may name; a write through an
        // expression that may yield it (`(c ? window : o).x = v`) may
        // land on another object. A name the browser may already define
        // there is never the script's own (REG-032).
        if self.names_global_object(member.object()) && claimable_global(name) {
            self.facts.written_globals.insert(name.to_string());
        }
        self.facts.defined_names.insert(name.to_string());
        if matches!(unparen(member.object()), Expression::ThisExpression(_))
            && let Some(class) = self.classes.last().copied()
        {
            self.class_member(class, name.to_string());
        }
        let callable = value.is_some_and(is_function_expression);
        if let MemberExpression::PrivateFieldExpression(private) = member {
            if callable {
                self.facts
                    .private_methods
                    .insert(private.field.name.as_str());
            } else {
                self.facts.tainted.insert(private.field.name.as_str());
            }
            return;
        }
        if let MemberExpression::StaticMemberExpression(static_member) = member {
            let name = static_member.property.name.as_str();
            if callable && matches!(unparen(member.object()), Expression::ThisExpression(_)) {
                self.instance_method(name);
            } else if !callable {
                self.facts.tainted.insert(name);
            }
        }
    }

    /// Checks the value written to a member by name.
    fn write_value(&mut self, member: &'a MemberExpression<'a>, value: &'a Expression<'a>) {
        let Some(name) = self.member_name(member) else {
            return;
        };
        let span = member.span();
        if self.is_global_object(member.object()) && ADMITTED_GLOBALS.contains(&name.as_str()) {
            self.refuse(
                "script-global",
                span,
                format!("assigning `{name}` on the global object replaces a browser API"),
            );
        }
        if IMPLICITLY_CALLED.contains(&name.as_str()) && !self.callback_safe(value, 0) {
            self.refuse(
                "script-call",
                span,
                format!("`{name}` is called by the browser itself, so its value must be a function the script defines"),
            );
        }
        if URL_PROPERTIES.contains(&name.as_str()) {
            let list = matches!(name.as_str(), "srcset" | "imageSrcset" | "ping");
            self.url_value_in(value, span, &format!("`{name}`"), list);
        } else if name.len() > 2 && name.starts_with("on") {
            if !matches!(unparen(value), Expression::NullLiteral(_))
                && !self.callback_safe(value, 0)
            {
                self.refuse(
                    "script-handler",
                    span,
                    format!("`{name}` is an event handler; its value must be a function the script defines"),
                );
            }
        } else if name == "cssText" || name == "style" {
            self.css_value(value, span, None);
        } else if let MemberExpression::StaticMemberExpression(static_member) = member
            && let Some(style) = unparen(&static_member.object).as_member_expression()
            && style.static_property_name() == Some("style")
        {
            let property = kebab(&name);
            if URL_CSS_PROPERTIES.contains(&property.as_str()) {
                self.css_value(value, span, Some(&property));
            }
        }
    }

    // ----- calls -------------------------------------------------------

    /// Walks a callee's parts without treating the callee itself as a
    /// value that escapes.
    fn callee_walk(&mut self, callee: &'a Expression<'a>) {
        self.prototype_receiver(callee);
        match unparen(callee) {
            Expression::Identifier(reference) => {
                if self.resolve(reference).is_none() {
                    self.global(reference.name.as_str(), reference.span, Pos::Value);
                }
            }
            Expression::FunctionExpression(function) => self.function(function, None, false),
            Expression::ArrowFunctionExpression(arrow) => self.arrow(arrow, None),
            other => {
                if let Some(member) = other.as_member_expression() {
                    self.member_access(member, false);
                } else {
                    self.expr(other, Pos::Operand);
                }
            }
        }
    }

    fn call(&mut self, call: &'a CallExpression<'a>) {
        let callee = unparen(&call.callee);
        if let Expression::Identifier(reference) = callee
            && let Some(id) = self.resolve(reference)
            && self.phase == Phase::Collect
            && let Some(binding) = self.facts.bindings.get_mut(id)
        {
            binding.calls.push(&call.arguments);
        }
        self.callee_walk(&call.callee);
        for (index, argument) in call.arguments.iter().enumerate() {
            match argument {
                Argument::SpreadElement(spread) => self.expr(&spread.argument, Pos::Value),
                other => {
                    if let Some(expression) = other.as_expression() {
                        let pos = self.argument_pos(callee, &call.arguments, index);
                        self.expr(expression, pos);
                    }
                }
            }
        }
        if !self.check() {
            return;
        }
        // `f.call(this, ...)`, `f.apply(this, [...])` and `f.bind(...)`
        // invoke `f`: resolve it with its own rules on the shifted
        // arguments.
        if let Some(member) = callee.as_member_expression()
            && let Some(name) = member.static_property_name()
            && matches!(name, "call" | "apply" | "bind")
        {
            let target = member.object();
            if !self.resolves_callee(target, 0) {
                self.refuse(
                    "script-call",
                    call.span,
                    format!("`.{name}()` invokes a function the scan cannot resolve"),
                );
                return;
            }
            for rule in self.callee_rules(target) {
                match name {
                    "call" => {
                        let shifted: Vec<&'a Argument<'a>> =
                            call.arguments.iter().skip(1).collect();
                        self.apply_rule(rule, target, &shifted, call.span);
                    }
                    _ => self.refuse(
                        "script-call",
                        call.span,
                        format!("`.{name}()` on a function whose arguments the scan checks"),
                    ),
                }
            }
            return;
        }
        if !self.resolves_callee(callee, 0) {
            self.refuse(
                "script-call",
                call.span,
                "the call resolves to neither a function the script defines nor a standard browser API".to_string(),
            );
            return;
        }
        let arguments: Vec<&'a Argument<'a>> = call.arguments.iter().collect();
        for rule in self.callee_rules(callee) {
            self.apply_rule(rule, callee, &arguments, call.span);
        }
    }

    /// The argument rule of an admitted callee, when it has one.
    fn callee_rule(&self, callee: &Expression<'a>) -> Option<Rule> {
        self.callee_rules(callee).into_iter().next()
    }

    /// The argument rules of every admitted function a callee may evaluate
    /// to: a sequence's last expression, both sides of a conditional or a
    /// logical expression, a bound function's target.
    fn callee_rules(&self, callee: &Expression<'a>) -> Vec<Rule> {
        match unparen(callee) {
            Expression::Identifier(reference) if self.bound(reference).is_none() => {
                rule_for(reference.name.as_str()).into_iter().collect()
            }
            Expression::SequenceExpression(sequence) => sequence
                .expressions
                .last()
                .map(|last| self.callee_rules(last))
                .unwrap_or_default(),
            Expression::ConditionalExpression(conditional) => {
                let mut rules = self.callee_rules(&conditional.consequent);
                rules.extend(self.callee_rules(&conditional.alternate));
                rules
            }
            Expression::LogicalExpression(logical) => {
                let mut rules = self.callee_rules(&logical.left);
                rules.extend(self.callee_rules(&logical.right));
                rules
            }
            Expression::ChainExpression(chain) => match &chain.expression {
                ChainElement::CallExpression(call) => self.bound_rules(call),
                other => other
                    .as_member_expression()
                    .map(|member| self.member_rules(member))
                    .unwrap_or_default(),
            },
            Expression::CallExpression(call) => self.bound_rules(call),
            other => other
                .as_member_expression()
                .map(|member| self.member_rules(member))
                .unwrap_or_default(),
        }
    }

    fn bound_rules(&self, call: &CallExpression<'a>) -> Vec<Rule> {
        match unparen(&call.callee).as_member_expression() {
            Some(member) if member.static_property_name() == Some("bind") => {
                self.callee_rules(member.object())
            }
            _ => Vec::new(),
        }
    }

    fn member_rules(&self, member: &MemberExpression<'a>) -> Vec<Rule> {
        let Some(name) = member.static_property_name() else {
            return Vec::new();
        };
        if self.script_method(member, name) {
            return Vec::new();
        }
        rule_for(name).into_iter().collect()
    }

    /// Whether a member call names a method the script defines on an object
    /// the script made, rather than a browser API of the same name.
    fn script_method(&self, member: &MemberExpression<'a>, name: &str) -> bool {
        if self.facts.tainted.contains(name) {
            return false;
        }
        match unparen(member.object()) {
            // `super.name()` calls the parent class, a browser API for an
            // element, so it keeps the API's rules.
            Expression::Super(_) => false,
            Expression::ThisExpression(_) => self
                .classes
                .last()
                .is_some_and(|class| self.class_defines(class, name)),
            Expression::Identifier(reference) => {
                let Some(id) = self.bound(reference) else {
                    return false;
                };
                let Some(binding) = self.binding(id) else {
                    return false;
                };
                if binding.kind != Kind::Const {
                    return false;
                }
                match binding.init.map(unparen) {
                    Some(Expression::NewExpression(new)) => match unparen(&new.callee) {
                        Expression::Identifier(class) => self
                            .bound(class)
                            .and_then(|class_id| self.binding(class_id))
                            .and_then(|class_binding| class_binding.class)
                            .is_some_and(|class| self.class_defines(class, name)),
                        _ => false,
                    },
                    Some(Expression::ObjectExpression(object)) => {
                        object.properties.iter().any(|property| {
                            matches!(property, ObjectPropertyKind::ObjectProperty(property)
                            if property.kind == PropertyKind::Init
                                && property.key.static_name().is_some_and(|key| key == name)
                                && is_function_expression(&property.value))
                        })
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }

    /// Whether a callee resolves to a function the script defines or a
    /// standard browser API (REG-032).
    fn resolves_callee(&self, callee: &Expression<'a>, depth: usize) -> bool {
        if depth > MAX_TRACE_DEPTH {
            return false;
        }
        match unparen(callee) {
            Expression::Identifier(reference) => {
                let name = reference.name.as_str();
                match self.bound(reference) {
                    Some(id) => self.binding_callable(id, depth + 1),
                    None => {
                        ADMITTED_GLOBALS.contains(&name)
                            && !GLOBAL_OBJECTS.contains(&name)
                            && name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
                            || matches!(
                                name,
                                "String"
                                    | "Number"
                                    | "Boolean"
                                    | "Array"
                                    | "Object"
                                    | "Symbol"
                                    | "Date"
                                    | "Error"
                                    | "URL"
                            )
                    }
                }
            }
            Expression::FunctionExpression(_)
            | Expression::ArrowFunctionExpression(_)
            | Expression::ClassExpression(_) => true,
            Expression::SequenceExpression(sequence) => sequence
                .expressions
                .last()
                .is_some_and(|last| self.resolves_callee(last, depth + 1)),
            Expression::ConditionalExpression(conditional) => {
                self.resolves_callee(&conditional.consequent, depth + 1)
                    && self.resolves_callee(&conditional.alternate, depth + 1)
            }
            Expression::LogicalExpression(logical) => {
                self.resolves_callee(&logical.left, depth + 1)
                    && self.resolves_callee(&logical.right, depth + 1)
            }
            Expression::ChainExpression(chain) => match &chain.expression {
                ChainElement::CallExpression(call) => self.bound_function(call, depth),
                other => other
                    .as_member_expression()
                    .is_some_and(|member| self.member_resolves(member)),
            },
            Expression::CallExpression(call) => self.bound_function(call, depth),
            other => other
                .as_member_expression()
                .is_some_and(|member| self.member_resolves(member)),
        }
    }

    /// `f.bind(...)` returns `f` bound: it resolves when `f` does.
    fn bound_function(&self, call: &CallExpression<'a>, depth: usize) -> bool {
        let Some(member) = unparen(&call.callee).as_member_expression() else {
            return false;
        };
        member.static_property_name() == Some("bind")
            && self.resolves_callee(member.object(), depth + 1)
            && self.callee_rule(member.object()).is_none()
    }

    fn member_resolves(&self, member: &MemberExpression<'a>) -> bool {
        match member {
            MemberExpression::PrivateFieldExpression(private) => {
                let name = private.field.name.as_str();
                self.facts.private_methods.contains(name) && !self.facts.tainted.contains(name)
            }
            MemberExpression::StaticMemberExpression(static_member) => {
                let name = static_member.property.name.as_str();
                if REFUSED_PROPERTIES.contains(&name) {
                    return false;
                }
                if self.script_method(member, name) {
                    return true;
                }
                ADMITTED_METHODS.contains(&name) && !self.facts.tainted.contains(name)
            }
            MemberExpression::ComputedMemberExpression(_) => false,
        }
    }

    fn binding_callable(&self, id: Bid, depth: usize) -> bool {
        let Some(binding) = self.binding(id) else {
            return false;
        };
        match binding.kind {
            Kind::Function | Kind::Class | Kind::Import => {
                !binding.opaque && binding.assignments.is_empty()
            }
            Kind::Const | Kind::Let | Kind::Var => {
                if binding.opaque || binding.numeric_updates {
                    return false;
                }
                let values = binding
                    .init
                    .into_iter()
                    .chain(binding.assignments.iter().copied());
                let mut any = false;
                for value in values {
                    any = true;
                    if !self.callback_safe(value, depth + 1) {
                        return false;
                    }
                }
                any
            }
            Kind::Param => {
                let Some(owner) = binding.param_of else {
                    return false;
                };
                let Some(function) = self.binding(owner) else {
                    return false;
                };
                if function.escapes || function.calls.is_empty() {
                    return false;
                }
                if let Some(default) = binding.param_default
                    && !self.callback_safe(default, depth + 1)
                {
                    return false;
                }
                function
                    .calls
                    .iter()
                    .all(|arguments| match arguments.get(binding.param_index) {
                        None => true,
                        Some(Argument::SpreadElement(_)) => false,
                        Some(argument) => argument
                            .as_expression()
                            .is_some_and(|expression| self.callback_safe(expression, depth + 1)),
                    })
            }
            Kind::Catch | Kind::Implicit => false,
        }
    }

    /// Whether a timer's handler is a function: [`Self::callback_safe`]
    /// without the `null` and `undefined` it allows for an optional
    /// callback, because REG-032 refuses a timer given anything but a
    /// function, and a `null` handler is the shape a string handler takes
    /// once the scan cannot read it.
    fn timer_handler_safe(&self, value: &Expression<'a>, depth: usize) -> bool {
        if depth > MAX_TRACE_DEPTH {
            return false;
        }
        match unparen(value) {
            Expression::NullLiteral(_) => false,
            Expression::Identifier(reference) if reference.name == "undefined" => false,
            Expression::ConditionalExpression(conditional) => {
                self.timer_handler_safe(&conditional.consequent, depth + 1)
                    && self.timer_handler_safe(&conditional.alternate, depth + 1)
            }
            Expression::LogicalExpression(logical) => {
                self.timer_handler_safe(&logical.left, depth + 1)
                    && self.timer_handler_safe(&logical.right, depth + 1)
            }
            other => self.callback_safe(other, depth),
        }
    }

    /// Whether a value passed where it will be called is a function the
    /// script defines or a standard browser function whose arguments need
    /// no checking.
    fn callback_safe(&self, value: &Expression<'a>, depth: usize) -> bool {
        if depth > MAX_TRACE_DEPTH {
            return false;
        }
        match unparen(value) {
            Expression::FunctionExpression(_)
            | Expression::ArrowFunctionExpression(_)
            | Expression::ClassExpression(_) => true,
            Expression::NullLiteral(_) => true,
            Expression::Identifier(reference)
                if reference.name == "undefined" && self.bound(reference).is_none() =>
            {
                true
            }
            Expression::ConditionalExpression(conditional) => {
                self.callback_safe(&conditional.consequent, depth + 1)
                    && self.callback_safe(&conditional.alternate, depth + 1)
            }
            Expression::LogicalExpression(logical) => {
                self.callback_safe(&logical.left, depth + 1)
                    && self.callback_safe(&logical.right, depth + 1)
            }
            other => self.resolves_callee(other, depth + 1) && self.callee_rule(other).is_none(),
        }
    }

    fn new_expression(&mut self, new: &'a NewExpression<'a>) {
        let callee = unparen(&new.callee);
        self.constructor_target(callee, new.span);
        let rule = match callee {
            Expression::Identifier(reference)
                if self.check() && self.bound(reference).is_none() =>
            {
                constructor_rule_for(reference.name.as_str())
            }
            _ => None,
        };
        for (index, argument) in new.arguments.iter().enumerate() {
            match argument {
                Argument::SpreadElement(spread) => self.expr(&spread.argument, Pos::Value),
                other => {
                    if let Some(expression) = other.as_expression() {
                        let pos = match rule {
                            Some(Rule::Callbacks(indices)) if indices.contains(&index) => {
                                Pos::Operand
                            }
                            _ => Pos::Value,
                        };
                        self.expr(expression, pos);
                    }
                }
            }
        }
        if !self.check() {
            return;
        }
        let name = match callee {
            Expression::Identifier(reference) if self.bound(reference).is_none() => {
                Some(reference.name.as_str())
            }
            other => other
                .as_member_expression()
                .filter(|member| self.is_global_object(member.object()))
                .and_then(|member| member.static_property_name()),
        };
        if let Some(name) = name
            && let Some(rule) = constructor_rule_for(name)
        {
            let arguments: Vec<&'a Argument<'a>> = new.arguments.iter().collect();
            self.apply_rule(rule, callee, &arguments, new.span);
        }
    }

    /// Checks what `new` or `extends` instantiates: a class or function the
    /// script defines, or an admitted constructor.
    fn constructor_target(&mut self, callee: &'a Expression<'a>, span: Span) {
        let callee = unparen(callee);
        match callee {
            Expression::Identifier(reference) => {
                let name = reference.name.as_str();
                match self.resolve(reference) {
                    Some(id) => {
                        if self.check() && !self.binding_callable(id, 0) {
                            self.refuse(
                                "script-call",
                                span,
                                format!("`new {name}` instantiates a value the scan cannot trace to a class"),
                            );
                        }
                    }
                    None => {
                        if name == "Function" || name == "eval" {
                            self.refuse(
                                "script-eval",
                                span,
                                format!("`{name}` compiles a string into code"),
                            );
                        } else if !ADMITTED_CONSTRUCTORS.contains(&name)
                            && !(name.starts_with("HTML") && ADMITTED_GLOBALS.contains(&name))
                        {
                            self.refuse(
                                "script-global",
                                span,
                                format!("`{name}` is not a constructor the scan admits"),
                            );
                        }
                    }
                }
            }
            Expression::ClassExpression(class) => self.class(class),
            other => {
                self.expr(other, Pos::Operand);
                let admitted = other.as_member_expression().is_some_and(|member| {
                    self.is_global_object(member.object())
                        && member
                            .static_property_name()
                            .is_some_and(|name| ADMITTED_CONSTRUCTORS.contains(&name))
                });
                if !admitted {
                    self.refuse(
                        "script-call",
                        span,
                        "`new` on a value the scan cannot trace to a class".to_string(),
                    );
                }
            }
        }
    }

    fn argument_expression(
        arguments: &[&'a Argument<'a>],
        index: usize,
    ) -> Option<&'a Expression<'a>> {
        arguments
            .get(index)
            .and_then(|argument| argument.as_expression())
    }

    fn apply_rule(
        &mut self,
        rule: Rule,
        callee: &'a Expression<'a>,
        arguments: &[&'a Argument<'a>],
        span: Span,
    ) {
        if arguments
            .iter()
            .any(|argument| matches!(argument, Argument::SpreadElement(_)))
        {
            self.refuse(
                "script-call",
                span,
                "spread arguments to a call whose arguments the scan checks".to_string(),
            );
            return;
        }
        match rule {
            Rule::Callbacks(indices) => {
                for index in indices {
                    if let Some(argument) = Self::argument_expression(arguments, *index)
                        && !self.callback_safe(argument, 0)
                    {
                        self.refuse(
                            "script-call",
                            argument.span(),
                            "a callback that resolves to neither a function the script defines nor a standard browser function".to_string(),
                        );
                    }
                }
            }
            Rule::Timer => match Self::argument_expression(arguments, 0) {
                Some(argument) if self.timer_handler_safe(argument, 0) => {}
                Some(argument) => self.refuse(
                    "script-timer",
                    argument.span(),
                    "a timer given anything but a function compiles a string into code".to_string(),
                ),
                None => {}
            },
            Rule::Attribute(name_index, value_index) => {
                let Some(name) = Self::argument_expression(arguments, name_index) else {
                    return;
                };
                let Some(names) = self.trace(name, 0) else {
                    self.refuse(
                        "script-attribute",
                        name.span(),
                        "an attribute name that does not trace to constants".to_string(),
                    );
                    return;
                };
                for attribute in names {
                    let lower = attribute.to_ascii_lowercase();
                    let local = lower.rsplit(':').next().unwrap_or(&lower).to_string();
                    if local.starts_with("on")
                        || matches!(
                            local.as_str(),
                            "srcdoc" | "style" | "http-equiv" | "content"
                        )
                        || lower == "xml:base"
                    {
                        self.refuse(
                            "script-attribute",
                            name.span(),
                            format!("setting the attribute `{attribute}` is refused"),
                        );
                    } else if (URL_ATTRIBUTES.contains(&lower.as_str())
                        || URL_ATTRIBUTES.contains(&local.as_str()))
                        && let Some(value) = value_index
                            .and_then(|index| Self::argument_expression(arguments, index))
                    {
                        let list = matches!(
                            local.as_str(),
                            "srcset" | "imagesrcset" | "ping" | "archive"
                        );
                        self.url_value_in(
                            value,
                            value.span(),
                            &format!("the `{attribute}` attribute"),
                            list,
                        );
                    }
                }
            }
            Rule::Element(index) => {
                let Some(name) = Self::argument_expression(arguments, index) else {
                    return;
                };
                match self.trace(name, 0) {
                    Some(names) => {
                        for element in names {
                            let lower = element.to_ascii_lowercase();
                            let local = lower.rsplit(':').next().unwrap_or(&lower);
                            if REFUSED_ELEMENTS.contains(&local) {
                                self.refuse(
                                    "script-element",
                                    name.span(),
                                    format!("creating a `<{element}>` element is refused"),
                                );
                            }
                        }
                    }
                    None => self.refuse(
                        "script-element",
                        name.span(),
                        "creating an element whose name does not trace to a constant".to_string(),
                    ),
                }
            }
            Rule::Url(index) => {
                if let Some(value) = Self::argument_expression(arguments, index) {
                    if let Expression::NewExpression(new) = unparen(value)
                        && matches!(unparen(&new.callee), Expression::Identifier(reference) if reference.name == "Request")
                    {
                        return;
                    }
                    self.url_value(value, value.span(), "the URL");
                }
            }
            Rule::Open => {
                let receiver = callee.as_member_expression().map(MemberExpression::object);
                let window = match receiver {
                    None => matches!(unparen(callee), Expression::Identifier(_)),
                    Some(object) => self.is_global_object(object),
                };
                let request = receiver.is_some_and(|object| self.is_request(object));
                if window {
                    if let Some(value) = Self::argument_expression(arguments, 0) {
                        self.url_value(value, value.span(), "the window URL");
                    }
                } else if request {
                    if let Some(value) = Self::argument_expression(arguments, 1) {
                        self.url_value(value, value.span(), "the request URL");
                    }
                } else {
                    self.refuse(
                        "script-call",
                        span,
                        "`.open()` on an object the scan cannot show is a window or an `XMLHttpRequest`, so it may be `document.open`".to_string(),
                    );
                }
            }
            Rule::Navigate => {
                let Some(value) = Self::argument_expression(arguments, 0) else {
                    return;
                };
                if matches!(unparen(value), Expression::RegExpLiteral(_)) {
                    return;
                }
                if self.trace(value, 0).is_some() {
                    self.url_value(value, value.span(), "the URL");
                } else {
                    self.refuse(
                        "script-url",
                        value.span(),
                        "`assign` and `replace` may navigate `location`, so their first argument must be a constant URL or a regular expression".to_string(),
                    );
                }
                if let Some(replacement) = Self::argument_expression(arguments, 1)
                    && self.trace(replacement, 0).is_none()
                    && !self.callback_safe(replacement, 0)
                {
                    self.refuse(
                        "script-call",
                        replacement.span(),
                        "a replacement that is neither a constant nor a function the script defines".to_string(),
                    );
                }
            }
            Rule::Define => {
                if let Some(name) = Self::argument_expression(arguments, 0) {
                    match self.trace(name, 0) {
                        Some(names) => {
                            for element in names {
                                self.defined_element(&element, name.span());
                            }
                        }
                        None => self.refuse(
                            "script-element",
                            name.span(),
                            "`customElements.define` with a name that is not a constant"
                                .to_string(),
                        ),
                    }
                }
                if let Some(class) = Self::argument_expression(arguments, 1)
                    && !self.callback_safe(class, 0)
                {
                    self.refuse(
                        "script-call",
                        class.span(),
                        "`customElements.define` with a class the script does not define"
                            .to_string(),
                    );
                }
            }
            Rule::Keyframes => {
                if let Some(keyframes) = Self::argument_expression(arguments, 0) {
                    self.keyframes(keyframes, span);
                }
            }
            Rule::StyleProperty => {
                let Some(name) = Self::argument_expression(arguments, 0) else {
                    return;
                };
                match self.trace(name, 0) {
                    Some(names) => {
                        for property in names {
                            let property = property.to_ascii_lowercase();
                            if (property.starts_with("--")
                                || URL_CSS_PROPERTIES.contains(&property.as_str()))
                                && let Some(value) = Self::argument_expression(arguments, 1)
                            {
                                self.css_value(value, value.span(), Some(&property));
                            }
                        }
                    }
                    None => self.refuse(
                        "script-css",
                        name.span(),
                        "`setProperty` with a property name that is not a constant".to_string(),
                    ),
                }
            }
        }
    }

    /// Checks the keyframes `animate` is given: a literal object, or a
    /// literal array of them, whose properties that name a resource hold
    /// constants the CSS scan admits.
    fn keyframes(&mut self, keyframes: &'a Expression<'a>, span: Span) {
        let frames: Vec<&'a ObjectExpression<'a>> = match unparen(keyframes) {
            Expression::ObjectExpression(object) => vec![&**object],
            Expression::ArrayExpression(array) => {
                let mut frames = Vec::new();
                for element in &array.elements {
                    match element.as_expression().map(unparen) {
                        Some(Expression::ObjectExpression(object)) => frames.push(&**object),
                        _ => {
                            self.refuse(
                                "script-css",
                                span,
                                "`animate` keyframes that are not literal objects".to_string(),
                            );
                            return;
                        }
                    }
                }
                frames
            }
            _ => {
                self.refuse(
                    "script-css",
                    span,
                    "`animate` keyframes that are not literal objects".to_string(),
                );
                return;
            }
        };
        for frame in frames {
            for property in &frame.properties {
                let ObjectPropertyKind::ObjectProperty(property) = property else {
                    self.refuse(
                        "script-css",
                        span,
                        "a spread in `animate` keyframes".to_string(),
                    );
                    continue;
                };
                let Some(name) = property.key.static_name().filter(|_| !property.computed) else {
                    self.refuse(
                        "script-css",
                        property.span,
                        "a computed property in `animate` keyframes".to_string(),
                    );
                    continue;
                };
                let css = kebab(&name);
                if !URL_CSS_PROPERTIES.contains(&css.as_str()) && !css.starts_with("--") {
                    continue;
                }
                match unparen(&property.value) {
                    Expression::ArrayExpression(values) => {
                        for value in &values.elements {
                            match value.as_expression() {
                                Some(value) => self.css_value(value, property.span, Some(&css)),
                                None => self.refuse(
                                    "script-css",
                                    property.span,
                                    "a keyframe value the scan cannot read".to_string(),
                                ),
                            }
                        }
                    }
                    value => self.css_value(value, property.span, Some(&css)),
                }
            }
        }
    }

    fn defined_element(&mut self, element: &str, span: Span) {
        let prefix = &self.context.element_prefix;
        if !element.starts_with(prefix.as_str()) {
            self.refuse(
                "script-element",
                span,
                format!("`{element}` does not start with the namespace prefix `{prefix}`"),
            );
            return;
        }
        if let Some(elements) = self.context.elements
            && !elements.iter().any(|declared| declared == element)
        {
            self.refuse(
                "script-element",
                span,
                format!("`{element}` is defined but the manifest does not declare it"),
            );
        }
    }

    fn is_request(&self, object: &Expression<'a>) -> bool {
        let Expression::Identifier(reference) = unparen(object) else {
            return false;
        };
        let Some(binding) = self.bound(reference).and_then(|id| self.binding(id)) else {
            return false;
        };
        binding.kind == Kind::Const
            && matches!(binding.init.map(unparen), Some(Expression::NewExpression(new))
                if matches!(unparen(&new.callee), Expression::Identifier(class) if class.name == "XMLHttpRequest"))
    }

    fn url_value(&mut self, value: &'a Expression<'a>, span: Span, what: &str) {
        self.url_value_in(value, span, what, false);
    }

    /// Checks a value written where the browser loads or navigates to a URL;
    /// a list-valued one (`srcset`, `imagesrcset`, `ping`, `archive`) has
    /// every URL in it checked, not only the first.
    fn url_value_in(&mut self, value: &'a Expression<'a>, span: Span, what: &str, list: bool) {
        match self.trace(value, 0) {
            Some(texts) => {
                for text in texts {
                    let urls: Vec<&str> = if list {
                        srcset_urls(&text)
                    } else {
                        vec![text.as_str()]
                    };
                    for url in urls {
                        if let Err(refusal) = check_constant(url) {
                            self.refuse(
                                "script-url",
                                span,
                                format!("{what}: {}", refusal.describe()),
                            );
                            return;
                        }
                    }
                }
            }
            None => self.refuse(
                "script-url",
                span,
                format!("{what} must be a constant that stays on the application's origin"),
            ),
        }
    }

    fn css_value(&mut self, value: &'a Expression<'a>, span: Span, property: Option<&str>) {
        match self.trace(value, 0) {
            Some(values) => {
                for text in values {
                    let declaration = match property {
                        Some(property) => format!("{property}: {text}"),
                        None => text,
                    };
                    let line = self.line(span);
                    for finding in
                        super::super::view::check_css_text(&self.context.file, &declaration, line)
                    {
                        if !self.findings.contains(&finding) {
                            self.findings.push(finding);
                        }
                    }
                }
            }
            None => self.refuse(
                "script-css",
                span,
                "a style value that may name a resource must be a constant the CSS scan admits"
                    .to_string(),
            ),
        }
    }

    fn import_source(&mut self, source: &'a StringLiteral<'a>) {
        let specifier = source.value.as_str();
        let base = format!("/{}/", self.context.own_directory);
        let resolved = if let Some(relative) = specifier.strip_prefix("./") {
            Some(format!("{base}{relative}"))
        } else if let Some(mut rest) = specifier.strip_prefix("../") {
            // Each `../` leaves one directory. One that leaves the
            // components' root would resolve, under a path prefix
            // (PFX-006), to a URL outside the prefix that no component
            // owns, so it is refused; the directories it did leave are
            // not compared against the admitted scripts.
            let mut segments: Vec<&str> = base.trim_matches('/').split('/').collect();
            let mut inside = segments.pop().is_some();
            while let Some(next) = rest.strip_prefix("../") {
                inside &= segments.pop().is_some();
                rest = next;
            }
            if inside {
                let directory = segments.join("/");
                Some(if directory.is_empty() {
                    format!("/{rest}")
                } else {
                    format!("/{directory}/{rest}")
                })
            } else {
                None
            }
        } else {
            // An absolute path names the asset route with no path prefix
            // (PFX-006), so it breaks under one, even to an own script.
            None
        };
        let admitted = resolved.as_deref().is_some_and(|path| {
            if path.contains("/./")
                || path.contains("/../")
                || path.contains('\\')
                || path.contains('?')
                || path.contains('#')
            {
                return false;
            }
            let Some((directory, file)) = path.trim_start_matches('/').rsplit_once('/') else {
                return false;
            };
            if !file.ends_with(".js") {
                return false;
            }
            if directory == self.context.own_directory {
                return self.context.own_scripts.contains(file);
            }
            self.context
                .importable_scripts
                .contains(&format!("{directory}/{file}"))
        });
        if !admitted {
            self.refuse(
                "script-import",
                source.span,
                format!("`import \"{specifier}\"`: the specifier must be a relative path (`./` or `../`) to a script of the component or of a component it depends on"),
            );
        }
    }

    // ----- prototypes ----------------------------------------------------

    /// Refuses a prototype used as a value (REG-032). A script may read a
    /// prototype's members, but once it keeps the prototype in a name or
    /// passes it on, the scan cannot follow it to where it is changed, so
    /// it is stopped where it would leave. A parenthesized, sequence,
    /// conditional, logical or assignment expression is not refused
    /// itself: the walk visits the part that yields the prototype as a
    /// value of its own and refuses it there.
    fn held_prototype(&mut self, expr: &'a Expression<'a>) {
        if !self.check()
            || matches!(
                expr,
                Expression::ParenthesizedExpression(_)
                    | Expression::SequenceExpression(_)
                    | Expression::ConditionalExpression(_)
                    | Expression::LogicalExpression(_)
                    | Expression::AssignmentExpression(_)
            )
            || !self.prototype(expr, Reach::Itself, 0, &mut BTreeSet::new())
        {
            return;
        }
        let what = prototype_text(expr);
        self.refuse(
            "script-prototype",
            expr.span(),
            format!("{what} is used as a value; a script may read a prototype's members, as `Array.prototype.slice` does, but may not keep or pass on the prototype, because the scan cannot follow it to where it is changed"),
        );
    }

    /// Refuses a method called on a prototype itself (REG-032): it runs
    /// with the prototype as `this`, and `Array.prototype` is an array, so
    /// `push`, `fill` or `splice` changes it. A method borrowed with `call`
    /// runs on the value it is given instead, and stays admitted.
    fn prototype_receiver(&mut self, callee: &'a Expression<'a>) {
        if !self.check() {
            return;
        }
        let member = match unparen(callee) {
            Expression::ChainExpression(chain) => chain.expression.as_member_expression(),
            other => other.as_member_expression(),
        };
        let Some(member) = member else {
            return;
        };
        if !self.prototype(member.object(), Reach::Itself, 0, &mut BTreeSet::new()) {
            return;
        }
        let method = member
            .static_property_name()
            .map_or_else(|| "a method".to_string(), |name| format!("`{name}`"));
        let what = prototype_text(member.object());
        self.refuse(
            "script-prototype",
            callee.span(),
            format!("{method} is called on {what} itself, so it runs with the prototype as `this` and can change it, as `push`, `fill` and `splice` do; borrow the method with `call` instead"),
        );
    }

    /// Refuses destructuring `prototype` out of an object (REG-032): the
    /// target receives the prototype, and a destructured name is one the
    /// scan does not follow.
    fn destructured_prototype(&mut self, key: &PropertyKey<'a>, computed: bool, span: Span) {
        if !self.check() {
            return;
        }
        let named = if computed {
            key.as_expression()
                .and_then(|key| self.trace(key, 0))
                .is_some_and(|keys| keys.iter().any(|key| prototype_name(key)))
        } else {
            key.static_name().is_some_and(|name| prototype_name(&name))
        };
        if named {
            self.refuse("script-prototype", span, DESTRUCTURED_PROTOTYPE.to_string());
        }
    }

    /// Refuses `delete` of a member of a prototype or of a value read from
    /// one, of a built-in, or of a page object's method, which changes it
    /// as a write does (REG-032).
    fn member_delete(&mut self, unary: &'a UnaryExpression<'a>) {
        if !self.check() {
            return;
        }
        let member = match unparen(&unary.argument) {
            Expression::ChainExpression(chain) => chain.expression.as_member_expression(),
            other => other.as_member_expression(),
        };
        let Some(member) = member else {
            return;
        };
        if self.prototype(member.object(), Reach::Through, 0, &mut BTreeSet::new()) {
            self.refuse(
                "script-prototype",
                unary.span,
                "deleting a member of a prototype changes it".to_string(),
            );
        } else {
            self.builtin_write(member, unary.span, "deleting");
        }
    }

    /// Whether an expression may evaluate to a prototype, or, with
    /// [`Reach::Through`], to a prototype or a value read from one: a
    /// member named `prototype` or `__proto__`, by a static name or a
    /// computed key that traces to one; what `getPrototypeOf` returns; a
    /// sequence, conditional, logical or assignment expression that may
    /// yield one; a binding the script initializes, defaults or assigns
    /// from one; and, with [`Reach::Through`], a parameter whose function
    /// a call by name hands one, as `f(Array.prototype.slice)` hands `slice`.
    /// A destructured name is not followed: [`Self::held_prototype`]
    /// refuses a prototype where it would enter one, and
    /// [`Self::held_builtin`] a value read from one.
    fn prototype(
        &self,
        expr: &Expression<'a>,
        reach: Reach,
        depth: usize,
        seen: &mut BTreeSet<Bid>,
    ) -> bool {
        if depth > MAX_TRACE_DEPTH {
            return false;
        }
        match unparen(expr) {
            Expression::CallExpression(call) => self.returns_prototype(call),
            Expression::ChainExpression(chain) => match &chain.expression {
                ChainElement::CallExpression(call) => self.returns_prototype(call),
                other => other
                    .as_member_expression()
                    .is_some_and(|member| self.prototype_member(member, reach, depth, seen)),
            },
            Expression::SequenceExpression(sequence) => sequence
                .expressions
                .last()
                .is_some_and(|last| self.prototype(last, reach, depth + 1, seen)),
            Expression::ConditionalExpression(conditional) => {
                self.prototype(&conditional.consequent, reach, depth + 1, seen)
                    || self.prototype(&conditional.alternate, reach, depth + 1, seen)
            }
            Expression::LogicalExpression(logical) => {
                self.prototype(&logical.left, reach, depth + 1, seen)
                    || self.prototype(&logical.right, reach, depth + 1, seen)
            }
            Expression::AssignmentExpression(assignment) => {
                assignment.operator == AssignmentOperator::Assign
                    && self.prototype(&assignment.right, reach, depth + 1, seen)
            }
            Expression::Identifier(reference) => {
                let Some(id) = self.bound(reference) else {
                    return false;
                };
                if !seen.insert(id) {
                    return false;
                }
                let Some(binding) = self.binding(id) else {
                    return false;
                };
                if binding
                    .init
                    .into_iter()
                    .chain(binding.param_default)
                    .chain(binding.assignments.iter().copied())
                    .any(|value| self.prototype(value, reach, depth + 1, seen))
                {
                    return true;
                }
                reach == Reach::Through
                    && self
                        .passed_arguments(binding)
                        .into_iter()
                        .any(|argument| self.prototype(argument, reach, depth + 1, seen))
            }
            other => other
                .as_member_expression()
                .is_some_and(|member| self.prototype_member(member, reach, depth, seen)),
        }
    }

    /// [`Self::prototype`] for a member expression.
    fn prototype_member(
        &self,
        member: &MemberExpression<'a>,
        reach: Reach,
        depth: usize,
        seen: &mut BTreeSet<Bid>,
    ) -> bool {
        let named = match member {
            MemberExpression::StaticMemberExpression(member) => {
                prototype_name(member.property.name.as_str())
            }
            MemberExpression::ComputedMemberExpression(member) => self
                .trace(&member.expression, 0)
                .is_some_and(|keys| keys.iter().any(|key| prototype_name(key))),
            MemberExpression::PrivateFieldExpression(_) => false,
        };
        named
            || (reach == Reach::Through && self.prototype(member.object(), reach, depth + 1, seen))
    }

    /// Whether a call is `getPrototypeOf` on any receiver but an object
    /// whose own method of that name the script defines: the scan cannot
    /// tell `Object` or `Reflect` from a name that holds one.
    fn returns_prototype(&self, call: &CallExpression<'a>) -> bool {
        let Some(member) = unparen(&call.callee).as_member_expression() else {
            return false;
        };
        let named = match member {
            MemberExpression::StaticMemberExpression(member) => {
                member.property.name == "getPrototypeOf"
            }
            MemberExpression::ComputedMemberExpression(member) => self
                .trace(&member.expression, 0)
                .is_some_and(|names| names.iter().any(|name| name == "getPrototypeOf")),
            MemberExpression::PrivateFieldExpression(_) => false,
        };
        named && !self.script_method(member, "getPrototypeOf")
    }

    // ----- built-ins -----------------------------------------------------

    /// Refuses a built-in used as a value where the scan stops following
    /// it (REG-032): an argument to anything but a function the script
    /// calls by name or a browser API that only calls it back, an element
    /// or property of an object, a member it is written to, a destructured
    /// name, a `return`, a `yield`, a `throw`, and a binding another script
    /// imports. There it would reach code that changes a member of it out
    /// of the scan's sight: `Object.defineProperty(Object, "keys", ...)`,
    /// `Promise.resolve(Math).then((m) => { m.random = f; })`. A prototype
    /// itself is the prototype rule's to refuse.
    fn held_builtin(&mut self, expr: &'a Expression<'a>) {
        if !self.check()
            || matches!(
                expr,
                Expression::ParenthesizedExpression(_)
                    | Expression::SequenceExpression(_)
                    | Expression::ConditionalExpression(_)
                    | Expression::LogicalExpression(_)
                    | Expression::AwaitExpression(_)
            )
            || self.prototype(expr, Reach::Itself, 0, &mut BTreeSet::new())
            || !self.reaches(Root::BuiltIn, expr, 0, &mut BTreeSet::new())
        {
            return;
        }
        let what = self.builtin_text(expr);
        self.refuse(
            "script-builtin",
            expr.span(),
            format!("{what} is a built-in, passed on here where the scan stops following it; a script may call a built-in, read from it, compare it or keep it in a name, but may not pass it on, because the scan cannot follow it to where a member of it is changed"),
        );
    }

    /// Refuses a write or a `delete` of a member of a built-in, or of a
    /// method of the page (REG-032). Either changes, for every script on the
    /// page, a function or object the browser provides: `Object.keys = f`,
    /// `p.call = g` where `p` holds `Object.keys`, `document.createElement
    /// = f`. Two kinds of name are refused on every value the script did not
    /// make, because the scan cannot follow every path to what they change:
    /// a method of the page, which `getRootNode()`, a `parentNode` or an
    /// event's `currentTarget` may hand over as `document`, and `call`,
    /// `apply` and `bind`, through which every script borrows a built-in
    /// method that any value may hold (`Math.random().toPrecision`).
    fn builtin_write(&mut self, member: &'a MemberExpression<'a>, span: Span, verb: &str) {
        let object = member.object();
        if self.reaches(Root::Method, object, 0, &mut BTreeSet::new()) {
            let what = self.builtin_text(object);
            self.refuse(
                "script-builtin",
                span,
                format!(
                    "{verb} a member of {what} changes a built-in for every script on the page"
                ),
            );
            return;
        }
        let Some(names) = self.member_names(member) else {
            return;
        };
        if let Some(name) = names.iter().find(|name| inherited_method(name))
            && self.reaches(Root::Page, object, 0, &mut BTreeSet::new())
        {
            let page = match path_text(object) {
                Some(path) if PAGE_OBJECTS.contains(&path.as_str()) => format!("`{path}`"),
                _ => "the page".to_string(),
            };
            self.refuse(
                "script-builtin",
                span,
                format!("{verb} `{name}` on {page} replaces a built-in function every script on the page calls"),
            );
            return;
        }
        let page = names
            .iter()
            .find(|name| PAGE_METHODS.contains(&name.as_str()));
        let borrowing = names.iter().find(|name| borrowing_method(name));
        if (page.is_none() && borrowing.is_none())
            || self.made_by_script(object, 0, &mut BTreeSet::new())
        {
            return;
        }
        if let Some(name) = page {
            self.refuse(
                "script-builtin",
                span,
                format!("{verb} `{name}` on a value the script did not make may replace the page's own `{name}`: the page reaches a script through calls, elements and events the scan does not follow (`getRootNode()` returns `document`), so only an object the script made may take a member of that name"),
            );
            return;
        }
        if let Some(name) = borrowing
            && !self.instance_this(object)
        {
            self.refuse(
                "script-builtin",
                span,
                format!("{verb} `{name}` on a value the script did not make may change the `{name}` every script borrows a built-in method with, and any value may hold one (`Math.random().toPrecision`), so only an object or function the script made may take a member of that name"),
            );
        }
    }

    /// Whether an expression is `this` in a class, its own instance, whose
    /// members are its own to write (REG-032).
    fn instance_this(&self, expr: &Expression<'a>) -> bool {
        matches!(unparen(expr), Expression::ThisExpression(this)
            if self.facts.this_class.contains_key(&this.span.start))
    }

    /// Whether an expression always evaluates to a value the script made,
    /// never one the browser hands it (REG-032): an object, array, function
    /// or class literal, a function or class it declares, a new instance of
    /// a standard constructor, or a name every value of which is one, a
    /// parameter included when every call of its function by name passes
    /// one. An instance of a class of the script's own does not count: its
    /// constructor may return any object.
    fn made_by_script(
        &self,
        expr: &Expression<'a>,
        depth: usize,
        seen: &mut BTreeSet<Bid>,
    ) -> bool {
        if depth > MAX_TRACE_DEPTH {
            return false;
        }
        match unparen(expr) {
            Expression::ObjectExpression(_)
            | Expression::ArrayExpression(_)
            | Expression::FunctionExpression(_)
            | Expression::ArrowFunctionExpression(_)
            | Expression::ClassExpression(_) => true,
            Expression::NewExpression(new) => matches!(unparen(&new.callee),
                Expression::Identifier(callee)
                    if self.bound(callee).is_none()
                        && ADMITTED_CONSTRUCTORS.contains(&callee.name.as_str())),
            Expression::Identifier(reference) => self
                .bound(reference)
                .is_some_and(|id| self.binding_made(id, depth + 1, seen)),
            Expression::SequenceExpression(sequence) => sequence
                .expressions
                .last()
                .is_some_and(|last| self.made_by_script(last, depth + 1, seen)),
            Expression::ConditionalExpression(conditional) => {
                self.made_by_script(&conditional.consequent, depth + 1, seen)
                    && self.made_by_script(&conditional.alternate, depth + 1, seen)
            }
            Expression::LogicalExpression(logical) => {
                self.made_by_script(&logical.left, depth + 1, seen)
                    && self.made_by_script(&logical.right, depth + 1, seen)
            }
            Expression::AssignmentExpression(assignment) => {
                assignment.operator == AssignmentOperator::Assign
                    && self.made_by_script(&assignment.right, depth + 1, seen)
            }
            _ => false,
        }
    }

    /// [`Self::made_by_script`] for a binding: a function or class the
    /// script declares and never reassigns, a variable every value of which
    /// is made, or a parameter every call of whose function by name passes
    /// one made. A binding met again inside its own values adds none.
    fn binding_made(&self, id: Bid, depth: usize, seen: &mut BTreeSet<Bid>) -> bool {
        if !seen.insert(id) {
            return true;
        }
        let Some(binding) = self.binding(id) else {
            return false;
        };
        if binding.opaque || binding.numeric_updates {
            return false;
        }
        match binding.kind {
            Kind::Function | Kind::Class => binding.assignments.is_empty(),
            Kind::Const | Kind::Let | Kind::Var => {
                let mut values = binding
                    .init
                    .into_iter()
                    .chain(binding.assignments.iter().copied())
                    .peekable();
                values.peek().is_some()
                    && values.all(|value| self.made_by_script(value, depth, seen))
            }
            Kind::Param => {
                let Some(function) = binding.param_of.and_then(|owner| self.binding(owner)) else {
                    return false;
                };
                if function.escapes || function.calls.is_empty() {
                    return false;
                }
                if let Some(default) = binding.param_default
                    && !self.made_by_script(default, depth, seen)
                {
                    return false;
                }
                function
                    .calls
                    .iter()
                    // An omitted argument is the default, checked above, or
                    // `undefined`, which has no member to write.
                    .all(|arguments| match arguments.get(binding.param_index) {
                        None => true,
                        Some(Argument::SpreadElement(_)) => false,
                        Some(argument) => argument
                            .as_expression()
                            .is_some_and(|value| self.made_by_script(value, depth, seen)),
                    })
            }
            Kind::Import | Kind::Catch | Kind::Implicit => false,
        }
    }

    /// Whether an expression may evaluate to what `root` names: with
    /// [`Root::BuiltIn`], a built-in object or function or a member one
    /// hands over (`Object`, `window.JSON`, `Object.keys`, what `super`
    /// reads); with [`Root::Method`], also a method any value inherits
    /// (`[].slice`); with [`Root::Page`], `document`, `location` or
    /// `history`. A name is followed to its initializer, default and
    /// assignments, and a parameter to the arguments each call of its
    /// function by name passes; a value the scan does not follow (a
    /// destructured name, an import, what most calls return) is not,
    /// because [`Self::held_builtin`] stops a built-in before it enters one.
    /// Two calls are followed ([`Self::call_reaches`]): `Object(value)`
    /// returns the value, and `getRootNode()` the page. Past
    /// [`MAX_TRACE_DEPTH`] the answer is yes, so a chain too long to follow
    /// is refused rather than admitted.
    fn reaches(
        &self,
        root: Root,
        expr: &Expression<'a>,
        depth: usize,
        seen: &mut BTreeSet<Bid>,
    ) -> bool {
        if depth > MAX_TRACE_DEPTH {
            return true;
        }
        match unparen(expr) {
            Expression::Identifier(reference) => match self.bound(reference) {
                None => {
                    let name = reference.name.as_str();
                    match root {
                        Root::BuiltIn | Root::Method => builtin_global(name),
                        Root::Page => PAGE_OBJECTS.contains(&name),
                    }
                }
                Some(id) => self.binding_reaches(root, id, depth + 1, seen),
            },
            // `super` reads the parent class, a built-in such as
            // `HTMLElement` for a custom element.
            Expression::Super(_) => root != Root::Page,
            Expression::SequenceExpression(sequence) => sequence
                .expressions
                .last()
                .is_some_and(|last| self.reaches(root, last, depth + 1, seen)),
            Expression::ConditionalExpression(conditional) => {
                self.reaches(root, &conditional.consequent, depth + 1, seen)
                    || self.reaches(root, &conditional.alternate, depth + 1, seen)
            }
            Expression::LogicalExpression(logical) => {
                self.reaches(root, &logical.left, depth + 1, seen)
                    || self.reaches(root, &logical.right, depth + 1, seen)
            }
            Expression::AssignmentExpression(assignment) => match assignment.operator {
                AssignmentOperator::Assign => {
                    self.reaches(root, &assignment.right, depth + 1, seen)
                }
                // `a ||= b` yields `a` or `b`.
                AssignmentOperator::LogicalOr
                | AssignmentOperator::LogicalAnd
                | AssignmentOperator::LogicalNullish => {
                    self.reaches(root, &assignment.right, depth + 1, seen)
                        || self.target_reaches(root, &assignment.left, depth + 1, seen)
                }
                _ => false,
            },
            Expression::AwaitExpression(await_expr) => {
                self.reaches(root, &await_expr.argument, depth + 1, seen)
            }
            Expression::CallExpression(call) => self.call_reaches(root, call, depth, seen),
            Expression::ChainExpression(chain) => match &chain.expression {
                ChainElement::CallExpression(call) => self.call_reaches(root, call, depth, seen),
                other => other
                    .as_member_expression()
                    .is_some_and(|member| self.member_reaches(root, member, depth, seen)),
            },
            other => other
                .as_member_expression()
                .is_some_and(|member| self.member_reaches(root, member, depth, seen)),
        }
    }

    /// [`Self::reaches`] for what a call returns (REG-032). `Object(value)`
    /// returns the value itself, so it reaches what the value reaches.
    /// `getRootNode()` returns the document for any node in it, as
    /// `ownerDocument` does, whatever node it is called on, so it is the
    /// page: `document.getRootNode()` is `document`. Any other call returns
    /// a value the scan does not follow.
    fn call_reaches(
        &self,
        root: Root,
        call: &CallExpression<'a>,
        depth: usize,
        seen: &mut BTreeSet<Bid>,
    ) -> bool {
        if let Expression::Identifier(callee) = unparen(&call.callee)
            && callee.name == "Object"
            && self.bound(callee).is_none()
        {
            return call
                .arguments
                .first()
                .and_then(Argument::as_expression)
                .is_some_and(|value| self.reaches(root, value, depth + 1, seen));
        }
        root == Root::Page
            && self.invokes(
                &call.callee,
                "getRootNode",
                &|_| true,
                depth + 1,
                &mut BTreeSet::new(),
            )
    }

    /// Whether a callee may invoke the method `name` read off a receiver
    /// `receiver` admits, or the global function of that name: the method
    /// itself, borrowed with `call` or `apply`, bound with `bind`, or held
    /// in a name (REG-032). Past [`MAX_TRACE_DEPTH`] the answer is yes.
    fn invokes(
        &self,
        callee: &Expression<'a>,
        name: &str,
        receiver: &dyn Fn(&Expression<'a>) -> bool,
        depth: usize,
        seen: &mut BTreeSet<Bid>,
    ) -> bool {
        if depth > MAX_TRACE_DEPTH {
            return true;
        }
        let member = match unparen(callee) {
            Expression::Identifier(reference) => {
                return match self.bound(reference) {
                    None => reference.name == name && ADMITTED_GLOBALS.contains(&name),
                    Some(id) => {
                        if !seen.insert(id) {
                            return false;
                        }
                        let Some(binding) = self.binding(id) else {
                            return false;
                        };
                        binding
                            .init
                            .into_iter()
                            .chain(binding.param_default)
                            .chain(binding.assignments.iter().copied())
                            .chain(self.passed_arguments(binding))
                            .any(|value| self.invokes(value, name, receiver, depth + 1, seen))
                    }
                };
            }
            Expression::SequenceExpression(sequence) => {
                return sequence
                    .expressions
                    .last()
                    .is_some_and(|last| self.invokes(last, name, receiver, depth + 1, seen));
            }
            Expression::ConditionalExpression(conditional) => {
                return self.invokes(&conditional.consequent, name, receiver, depth + 1, seen)
                    || self.invokes(&conditional.alternate, name, receiver, depth + 1, seen);
            }
            Expression::LogicalExpression(logical) => {
                return self.invokes(&logical.left, name, receiver, depth + 1, seen)
                    || self.invokes(&logical.right, name, receiver, depth + 1, seen);
            }
            // `f.bind(...)` returns `f` bound.
            Expression::CallExpression(call) => {
                return unparen(&call.callee)
                    .as_member_expression()
                    .filter(|member| member.static_property_name() == Some("bind"))
                    .is_some_and(|member| {
                        self.invokes(member.object(), name, receiver, depth + 1, seen)
                    });
            }
            Expression::ChainExpression(chain) => match chain.expression.as_member_expression() {
                Some(member) => member,
                None => return false,
            },
            other => match other.as_member_expression() {
                Some(member) => member,
                None => return false,
            },
        };
        let named = self
            .member_names(member)
            .is_some_and(|names| names.iter().any(|candidate| candidate == name));
        if named && receiver(member.object()) {
            return true;
        }
        // `f.call(...)` and `f.apply(...)` invoke `f`.
        matches!(member.static_property_name(), Some("call" | "apply"))
            && self.invokes(member.object(), name, receiver, depth + 1, seen)
    }

    /// [`Self::reaches`] for the old value of a logical assignment's target.
    fn target_reaches(
        &self,
        root: Root,
        target: &AssignmentTarget<'a>,
        depth: usize,
        seen: &mut BTreeSet<Bid>,
    ) -> bool {
        match target.as_simple_assignment_target() {
            Some(SimpleAssignmentTarget::AssignmentTargetIdentifier(identifier)) => self
                .bound(identifier)
                .is_some_and(|id| self.binding_reaches(root, id, depth, seen)),
            Some(other) => other
                .as_member_expression()
                .is_some_and(|member| self.member_reaches(root, member, depth, seen)),
            None => false,
        }
    }

    /// [`Self::reaches`] for a binding: its initializer, default and
    /// assignments, and, for a parameter, each argument a call of its
    /// function by name passes in its place.
    fn binding_reaches(&self, root: Root, id: Bid, depth: usize, seen: &mut BTreeSet<Bid>) -> bool {
        if !seen.insert(id) {
            return false;
        }
        let Some(binding) = self.binding(id) else {
            return false;
        };
        binding
            .init
            .into_iter()
            .chain(binding.param_default)
            .chain(binding.assignments.iter().copied())
            .chain(self.passed_arguments(binding))
            .any(|value| self.reaches(root, value, depth, seen))
    }

    /// [`Self::reaches`] for a member expression.
    fn member_reaches(
        &self,
        root: Root,
        member: &MemberExpression<'a>,
        depth: usize,
        seen: &mut BTreeSet<Bid>,
    ) -> bool {
        let object = member.object();
        let names = self.member_names(member);
        let named = |test: &dyn Fn(&str) -> bool| {
            names
                .as_ref()
                .is_some_and(|names| names.iter().any(|name| test(name)))
        };
        if self.is_global_object(object) {
            return match root {
                Root::BuiltIn | Root::Method => named(&builtin_global),
                Root::Page => named(&|name| PAGE_OBJECTS.contains(&name)),
            };
        }
        match root {
            Root::BuiltIn | Root::Method => {
                if let MemberExpression::PrivateFieldExpression(_) = member {
                    // A private field holds only what the script stores in
                    // it, and a built-in is never stored in a member.
                    return false;
                }
                let constant = names.as_ref().is_some_and(|names| {
                    !names.is_empty() && names.iter().all(|name| constant_name(name))
                });
                (!constant && self.reaches(root, object, depth + 1, seen))
                    || (root == Root::Method
                        && names.as_ref().is_some_and(|names| {
                            names.iter().any(|name| {
                                inherited_method(name) && !self.own_member(object, name)
                            })
                        }))
                    || names.as_ref().is_some_and(|names| {
                        self.literal_inherits(object, names, depth + 1, &mut BTreeSet::new())
                    })
            }
            Root::Page => {
                named(&|name| name == "ownerDocument")
                    || (named(&|name| name == "location")
                        && self.reaches(root, object, depth + 1, seen))
            }
        }
    }

    /// The names a member expression reads, when the scan can list them: a
    /// static name, or the constants a computed key traces to.
    fn member_names(&self, member: &MemberExpression<'a>) -> Option<Vec<String>> {
        match member {
            MemberExpression::StaticMemberExpression(member) => {
                Some(vec![member.property.name.to_string()])
            }
            MemberExpression::ComputedMemberExpression(member) => self.trace(&member.expression, 0),
            MemberExpression::PrivateFieldExpression(member) => {
                Some(vec![format!("#{}", member.field.name)])
            }
        }
    }

    /// Whether a member read off `object` under one of `names` may be a
    /// method the value inherits from a built-in because the value is a
    /// literal or an operator's result, whose kind the scan knows
    /// (REG-032). Whatever the name, a member that a primitive, an array or
    /// a regular expression does not hold itself is its prototype's, a
    /// built-in every script shares: `(0).toPrecision`, `(-1).toPrecision`,
    /// `"".anchor`, `[].copyWithin`, `/x/.compile`. What each holds itself
    /// is data: a string's or an array's `length` and indices, a regular
    /// expression's `lastIndex`, `source` and flags. A class inherits the
    /// statics of the class it extends, so one it does not declare is its
    /// parent's, and a built-in's at the root (`class A extends Promise {}`
    /// hands over `Promise.withResolvers`). A name counts only when it can
    /// hold nothing else: a constant initialized with the literal, or a
    /// class never reassigned. A parameter or a variable is not followed,
    /// because the scan reads no `typeof` test: `typeof o === "string" ? o :
    /// o.label` reads `label` only off an object, though a call passes `o` a
    /// string. An object or function literal inherits only from
    /// `Object.prototype` and `Function.prototype`, whose methods' names the
    /// lists already hold. Past [`MAX_TRACE_DEPTH`] the answer is yes.
    fn literal_inherits(
        &self,
        object: &Expression<'a>,
        names: &[String],
        depth: usize,
        seen: &mut BTreeSet<Bid>,
    ) -> bool {
        if depth > MAX_TRACE_DEPTH {
            return true;
        }
        let inherited =
            |own: &dyn Fn(&str) -> bool| names.iter().any(|name| !index_name(name) && !own(name));
        match unparen(object) {
            Expression::ArrayExpression(_) => inherited(&|name| name == "length"),
            Expression::RegExpLiteral(_) => inherited(&regexp_data),
            Expression::ClassExpression(class) => self.class_inherits(class, names, depth, seen),
            Expression::Identifier(reference) => self
                .bound(reference)
                .is_some_and(|id| self.binding_literal_inherits(id, names, depth + 1, seen)),
            Expression::SequenceExpression(sequence) => sequence
                .expressions
                .last()
                .is_some_and(|last| self.literal_inherits(last, names, depth + 1, seen)),
            Expression::ConditionalExpression(conditional) => {
                self.literal_inherits(&conditional.consequent, names, depth + 1, seen)
                    || self.literal_inherits(&conditional.alternate, names, depth + 1, seen)
            }
            Expression::LogicalExpression(logical) => {
                self.literal_inherits(&logical.left, names, depth + 1, seen)
                    || self.literal_inherits(&logical.right, names, depth + 1, seen)
            }
            Expression::AssignmentExpression(assignment) => match assignment.operator {
                AssignmentOperator::Assign => {
                    self.literal_inherits(&assignment.right, names, depth + 1, seen)
                }
                // `a ||= b` yields `a`, which is not followed, or `b`.
                AssignmentOperator::LogicalOr
                | AssignmentOperator::LogicalAnd
                | AssignmentOperator::LogicalNullish => {
                    self.literal_inherits(&assignment.right, names, depth + 1, seen)
                }
                // `a += b` and the other arithmetic forms yield a primitive.
                _ => inherited(&|name| name == "length"),
            },
            Expression::AwaitExpression(await_expr) => {
                self.literal_inherits(&await_expr.argument, names, depth + 1, seen)
            }
            other if primitive_result(other) => inherited(&|name| name == "length"),
            _ => false,
        }
    }

    /// [`Self::literal_inherits`] for a name that can hold only one value:
    /// a class the script declares and never reassigns, or a constant
    /// initialized with a literal, an operator's result or a class.
    fn binding_literal_inherits(
        &self,
        id: Bid,
        names: &[String],
        depth: usize,
        seen: &mut BTreeSet<Bid>,
    ) -> bool {
        if !seen.insert(id) {
            return false;
        }
        let Some(binding) = self.binding(id) else {
            return false;
        };
        if binding.opaque || !binding.assignments.is_empty() {
            return false;
        }
        match binding.kind {
            Kind::Class => binding
                .class
                .is_some_and(|class| self.class_inherits(class, names, depth, seen)),
            Kind::Const => binding.init.is_some_and(|init| {
                let value = unparen(init);
                (primitive_result(value)
                    || matches!(
                        value,
                        Expression::ArrayExpression(_)
                            | Expression::RegExpLiteral(_)
                            | Expression::ClassExpression(_)
                    ))
                    && self.literal_inherits(value, names, depth, seen)
            }),
            _ => false,
        }
    }

    /// [`Self::literal_inherits`] for a class: a static it does not declare
    /// is the one the class it extends has, which is inherited when that
    /// class is a built-in or inherits it in turn. A class's `prototype`,
    /// `length` and `name` are its own, and a constant's name
    /// (`ELEMENT_NODE`) is a number.
    fn class_inherits(
        &self,
        class: &Class<'a>,
        names: &[String],
        depth: usize,
        seen: &mut BTreeSet<Bid>,
    ) -> bool {
        let undeclared: Vec<String> = names
            .iter()
            .filter(|name| {
                !index_name(name)
                    && !constant_name(name)
                    && !matches!(name.as_str(), "prototype" | "length" | "name")
                    && !declares_static(class, name)
            })
            .cloned()
            .collect();
        let Some(parent) = &class.super_class else {
            return false;
        };
        !undeclared.is_empty()
            && (self.reaches(Root::BuiltIn, parent, depth + 1, &mut BTreeSet::new())
                || self.literal_inherits(parent, &undeclared, depth + 1, seen))
    }

    /// Whether a member is one the script gave the object itself, so a
    /// method of that name is the script's own, not one the object
    /// inherits: a member a class it defines declares or writes on `this`,
    /// read from `this` in that class or from a constant it built with
    /// `new`, or a key of an object literal, read from the literal or a
    /// constant it initializes.
    fn own_member(&self, object: &Expression<'a>, name: &str) -> bool {
        let class_has = |class: u32| {
            self.facts
                .class_members
                .get(&class)
                .is_some_and(|members| members.contains(name))
        };
        match unparen(object) {
            Expression::ThisExpression(this) => self
                .facts
                .this_class
                .get(&this.span.start)
                .is_some_and(|class| class_has(*class)),
            Expression::ObjectExpression(object) => object_defines(object, name),
            Expression::Identifier(reference) => {
                let Some(binding) = self.bound(reference).and_then(|id| self.binding(id)) else {
                    return false;
                };
                if binding.kind != Kind::Const {
                    return false;
                }
                match binding.init.map(unparen) {
                    Some(Expression::ObjectExpression(object)) => object_defines(object, name),
                    Some(Expression::NewExpression(new)) => match unparen(&new.callee) {
                        Expression::Identifier(class) => self
                            .bound(class)
                            .and_then(|id| self.binding(id))
                            .and_then(|binding| binding.class)
                            .is_some_and(|class| class_has(class.span.start)),
                        _ => false,
                    },
                    _ => false,
                }
            }
            _ => false,
        }
    }

    /// The arguments each call of a parameter's function by name passes in
    /// the parameter's place. A spread argument is left out: its elements
    /// are an array's, where a built-in never goes.
    fn passed_arguments(&self, binding: &Binding<'a>) -> Vec<&'a Expression<'a>> {
        if binding.kind != Kind::Param {
            return Vec::new();
        }
        let Some(function) = binding.param_of.and_then(|owner| self.binding(owner)) else {
            return Vec::new();
        };
        function
            .calls
            .iter()
            .filter_map(|arguments| arguments.get(binding.param_index))
            .filter_map(Argument::as_expression)
            .collect()
    }

    /// Where an argument of a call stands (REG-032): kept, when it is
    /// handed to a parameter the scan follows; used up, when a browser API
    /// only calls it back; a value, otherwise.
    fn argument_pos(
        &self,
        callee: &Expression<'a>,
        arguments: &[Argument<'a>],
        index: usize,
    ) -> Pos {
        if !self.check() {
            return Pos::Value;
        }
        // A spread shifts the arguments after it to parameters the scan
        // cannot name.
        if arguments
            .iter()
            .any(|argument| matches!(argument, Argument::SpreadElement(_)))
        {
            return Pos::Value;
        }
        if let Expression::Identifier(reference) = unparen(callee)
            && let Some(id) = self.bound(reference)
        {
            return if self.traced_parameter(id, index) {
                Pos::Kept
            } else {
                Pos::Value
            };
        }
        if self.browser_callback(callee, index) {
            Pos::Operand
        } else {
            Pos::Value
        }
    }

    /// Whether every function a binding holds takes the argument at
    /// `index` in a plain parameter the scan follows to its uses, and none
    /// reads `arguments`, through which the argument would arrive
    /// unfollowed.
    fn traced_parameter(&self, id: Bid, index: usize) -> bool {
        let Some(binding) = self.binding(id) else {
            return false;
        };
        if binding.opaque || binding.numeric_updates {
            return false;
        }
        match binding.kind {
            Kind::Function => {
                binding.assignments.is_empty()
                    && binding.declaration.is_some_and(|function| {
                        plain_parameter(&function.params, index) && !self.reads_arguments(function)
                    })
            }
            Kind::Const | Kind::Let | Kind::Var => {
                let mut any = false;
                for value in binding
                    .init
                    .into_iter()
                    .chain(binding.assignments.iter().copied())
                {
                    any = true;
                    let traced = match unparen(value) {
                        Expression::FunctionExpression(function) => {
                            plain_parameter(&function.params, index)
                                && !self.reads_arguments(function)
                        }
                        Expression::ArrowFunctionExpression(arrow) => {
                            plain_parameter(&arrow.params, index)
                        }
                        _ => false,
                    };
                    if !traced {
                        return false;
                    }
                }
                any
            }
            _ => false,
        }
    }

    /// Whether a function reads its `arguments`.
    fn reads_arguments(&self, function: &Function<'a>) -> bool {
        self.facts
            .arguments_of
            .get(&function.span.start)
            .and_then(|id| self.binding(*id))
            .is_none_or(|binding| binding.escapes)
    }

    /// Whether a call is to a browser API that takes the argument at
    /// `index` only to call it back (`map(Number)`, `then(console.log)`): a
    /// global function, or a method whose name the script does not define
    /// on any object of its own, which would make the receiver possibly the
    /// script's own object rather than the API.
    fn browser_callback(&self, callee: &Expression<'a>, index: usize) -> bool {
        let callee = match unparen(callee) {
            Expression::ChainExpression(chain) => match chain.expression.as_member_expression() {
                Some(member) => member,
                None => return false,
            },
            other => match other.as_member_expression() {
                Some(member) => member,
                None => {
                    return matches!(other, Expression::Identifier(reference)
                        if self.bound(reference).is_none()
                            && is_callback(rule_for(reference.name.as_str()), index));
                }
            },
        };
        let Some(name) = callee.static_property_name() else {
            return false;
        };
        !self.facts.defined_names.contains(name)
            && is_callback(self.member_rules(callee).into_iter().next(), index)
    }

    /// How a refusal names a built-in: by its path, or, read off a literal
    /// or what a call returns, by the member read (`(0).toPrecision` is
    /// `.toPrecision`).
    fn builtin_text(&self, expr: &Expression<'a>) -> String {
        match (unparen(expr), path_text(expr)) {
            (Expression::Identifier(reference), Some(path)) if self.bound(reference).is_some() => {
                format!("`{path}`, which holds a built-in,")
            }
            (_, Some(path)) => format!("`{path}`"),
            (Expression::StaticMemberExpression(member), None) => {
                format!("`.{}`", member.property.name)
            }
            (_, None) => "a built-in".to_string(),
        }
    }

    // ----- tracing -------------------------------------------------------

    /// The constant strings an expression can evaluate to, when the scan
    /// can list them.
    fn trace(&self, expr: &Expression<'a>, depth: usize) -> Option<Vec<String>> {
        if depth > MAX_TRACE_DEPTH {
            return None;
        }
        let result = match unparen(expr) {
            Expression::StringLiteral(literal) => vec![literal.value.to_string()],
            Expression::NumericLiteral(literal) => vec![number_text(literal.value)],
            Expression::BooleanLiteral(literal) => vec![literal.value.to_string()],
            Expression::NullLiteral(_) => vec!["null".to_string()],
            Expression::TemplateLiteral(template) => {
                let mut values = vec![String::new()];
                for (index, quasi) in template.quasis.iter().enumerate() {
                    let cooked = quasi.value.cooked.as_ref()?.to_string();
                    values = values.into_iter().map(|value| value + &cooked).collect();
                    if let Some(expression) = template.expressions.get(index) {
                        let parts = self.trace(expression, depth + 1)?;
                        values = product(&values, &parts)?;
                    }
                }
                values
            }
            Expression::BinaryExpression(binary) if binary.operator == BinaryOperator::Addition => {
                let left = self.trace(&binary.left, depth + 1)?;
                let right = self.trace(&binary.right, depth + 1)?;
                product(&left, &right)?
            }
            Expression::ConditionalExpression(conditional) => {
                let mut values = self.trace(&conditional.consequent, depth + 1)?;
                values.extend(self.trace(&conditional.alternate, depth + 1)?);
                values
            }
            Expression::LogicalExpression(logical) => {
                let mut values = self.trace(&logical.left, depth + 1)?;
                values.extend(self.trace(&logical.right, depth + 1)?);
                values
            }
            Expression::SequenceExpression(sequence) => {
                self.trace(sequence.expressions.last()?, depth + 1)?
            }
            Expression::CallExpression(call)
                if matches!(unparen(&call.callee), Expression::Identifier(callee)
                    if callee.name == "String" && self.bound(callee).is_none()) =>
            {
                match call.arguments.first() {
                    Some(argument) => self.trace(argument.as_expression()?, depth + 1)?,
                    None => vec![String::new()],
                }
            }
            Expression::Identifier(reference) => {
                let name = reference.name.as_str();
                match self.bound(reference) {
                    None if name == "undefined" => vec!["undefined".to_string()],
                    None => return None,
                    Some(id) => self.trace_binding(id, depth + 1)?,
                }
            }
            _ => return None,
        };
        let mut unique: Vec<String> = Vec::new();
        for value in result {
            if !unique.contains(&value) {
                unique.push(value);
            }
        }
        (unique.len() <= MAX_VALUES).then_some(unique)
    }

    fn trace_binding(&self, id: Bid, depth: usize) -> Option<Vec<String>> {
        let binding = self.binding(id)?;
        if binding.opaque || binding.numeric_updates {
            return None;
        }
        match binding.kind {
            Kind::Const | Kind::Let | Kind::Var => {
                let mut values = Vec::new();
                match binding.init {
                    Some(init) => values.extend(self.trace(init, depth)?),
                    None if binding.kind != Kind::Const => values.push("undefined".to_string()),
                    None => return None,
                }
                for assignment in &binding.assignments {
                    values.extend(self.trace(assignment, depth)?);
                }
                Some(values)
            }
            Kind::Param => {
                let function = self.binding(binding.param_of?)?;
                if function.escapes || function.calls.is_empty() {
                    return None;
                }
                let mut values = Vec::new();
                for arguments in &function.calls {
                    match arguments.get(binding.param_index) {
                        None => match binding.param_default {
                            Some(default) => values.extend(self.trace(default, depth)?),
                            None => values.push("undefined".to_string()),
                        },
                        Some(Argument::SpreadElement(_)) => return None,
                        Some(argument) => {
                            values.extend(self.trace(argument.as_expression()?, depth)?)
                        }
                    }
                }
                Some(values)
            }
            _ => None,
        }
    }

    /// Whether an expression always evaluates to a number, so a computed
    /// key built from it cannot name a property such as `eval`.
    fn numeric(&self, expr: &Expression<'a>, depth: usize) -> bool {
        if depth > MAX_TRACE_DEPTH {
            return false;
        }
        match unparen(expr) {
            Expression::NumericLiteral(_) => true,
            Expression::UnaryExpression(unary) => matches!(
                unary.operator,
                UnaryOperator::UnaryNegation | UnaryOperator::UnaryPlus | UnaryOperator::BitwiseNot
            ),
            Expression::UpdateExpression(_) => true,
            Expression::BinaryExpression(binary) => match binary.operator {
                BinaryOperator::Addition => {
                    self.numeric(&binary.left, depth + 1) && self.numeric(&binary.right, depth + 1)
                }
                BinaryOperator::Subtraction
                | BinaryOperator::Multiplication
                | BinaryOperator::Division
                | BinaryOperator::Remainder
                | BinaryOperator::Exponential
                | BinaryOperator::ShiftLeft
                | BinaryOperator::ShiftRight
                | BinaryOperator::ShiftRightZeroFill
                | BinaryOperator::BitwiseOR
                | BinaryOperator::BitwiseXOR
                | BinaryOperator::BitwiseAnd => true,
                _ => false,
            },
            Expression::ConditionalExpression(conditional) => {
                self.numeric(&conditional.consequent, depth + 1)
                    && self.numeric(&conditional.alternate, depth + 1)
            }
            Expression::SequenceExpression(sequence) => sequence
                .expressions
                .last()
                .is_some_and(|last| self.numeric(last, depth + 1)),
            Expression::Identifier(reference) => {
                let Some(binding) = self.bound(reference).and_then(|id| self.binding(id)) else {
                    return false;
                };
                if binding.opaque || !matches!(binding.kind, Kind::Const | Kind::Let | Kind::Var) {
                    return false;
                }
                let Some(init) = binding.init else {
                    return false;
                };
                self.numeric(init, depth + 1)
                    && binding
                        .assignments
                        .iter()
                        .all(|value| self.numeric(value, depth + 1))
            }
            _ => false,
        }
    }
}

/// Whether a script may claim a name it writes on the global object as a
/// global of its own, which it may then read back (REG-032). The browser's
/// own properties of the global object whose writes fail, so that a
/// `try` around the write leaves the browser's value to read
/// (`localStorage`, `indexedDB`, `navigator` and every property added
/// later), are attributes, and attributes are named in lower camel case.
/// A name that starts with a capital is an interface or a namespace,
/// which a write replaces, so the script reads back its own value. A name
/// the scan's lists already hold is never the script's: it is admitted,
/// refused or read-only on its own terms.
fn claimable_global(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_uppercase())
        && !ADMITTED_GLOBALS.contains(&name)
        && !REFUSED_PROPERTIES.contains(&name)
        && !READ_ONLY_PROPERTIES.contains(&name)
}

/// Whether a global the script does not declare names a built-in object or
/// function: an admitted global that is neither the global object, nor a
/// primitive value, nor the page (REG-032).
fn builtin_global(name: &str) -> bool {
    ADMITTED_GLOBALS.contains(&name)
        && !GLOBAL_OBJECTS.contains(&name)
        && !PAGE_OBJECTS.contains(&name)
        && !matches!(name, "undefined" | "NaN" | "Infinity")
}

/// Whether a member name is a constant's: all capitals, digits and
/// underscores, as the language's (`Math.PI`, `Number.MAX_SAFE_INTEGER`)
/// and the DOM's (`Node.ELEMENT_NODE`) are. Each is a number, never an
/// object or a function, so reading one hands over no built-in.
fn constant_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_uppercase())
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// Whether a member name is a built-in method's, which a value the script
/// did not make inherits.
fn inherited_method(name: &str) -> bool {
    ADMITTED_METHODS.contains(&name) || INHERITED_METHODS.contains(&name)
}

/// Whether a member name is one of the three through which every script
/// borrows a built-in method: `Array.prototype.slice.call(list)` runs the
/// `call` that `slice` holds (REG-032).
fn borrowing_method(name: &str) -> bool {
    matches!(name, "call" | "apply" | "bind")
}

/// Whether a member name is an index, which names an element or a
/// character, never a method.
fn index_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_digit())
}

/// Whether a member name is data a regular expression holds itself or
/// reads from its flags, rather than a method it inherits.
fn regexp_data(name: &str) -> bool {
    matches!(
        name,
        "lastIndex"
            | "source"
            | "flags"
            | "global"
            | "ignoreCase"
            | "multiline"
            | "dotAll"
            | "unicode"
            | "unicodeSets"
            | "sticky"
            | "hasIndices"
    )
}

/// Whether an expression always yields a primitive: a literal other than a
/// regular expression, or what an operator returns. Its members are the
/// methods its type's prototype holds.
fn primitive_result(expr: &Expression<'_>) -> bool {
    matches!(
        expr,
        Expression::NumericLiteral(_)
            | Expression::BigIntLiteral(_)
            | Expression::BooleanLiteral(_)
            | Expression::StringLiteral(_)
            | Expression::TemplateLiteral(_)
            | Expression::UnaryExpression(_)
            | Expression::BinaryExpression(_)
            | Expression::UpdateExpression(_)
            | Expression::PrivateInExpression(_)
    )
}

/// Whether a class declares a static member of this name.
fn declares_static(class: &Class<'_>, name: &str) -> bool {
    class
        .body
        .body
        .iter()
        .any(|element| element.r#static() && element.static_name().is_some_and(|key| key == name))
}

/// Whether the parameter at `index` is a plain name, which the scan
/// follows, rather than a pattern, a rest parameter or none at all.
fn plain_parameter(params: &FormalParameters<'_>, index: usize) -> bool {
    params
        .items
        .get(index)
        .is_some_and(|param| matches!(param.pattern, BindingPattern::BindingIdentifier(_)))
}

/// Whether an argument rule calls back the argument at `index`.
fn is_callback(rule: Option<Rule>, index: usize) -> bool {
    match rule {
        Some(Rule::Callbacks(indices)) => indices.contains(&index),
        Some(Rule::Timer) => index == 0,
        _ => false,
    }
}

/// Whether an object literal gives itself a member of this name.
fn object_defines(object: &ObjectExpression<'_>, name: &str) -> bool {
    object.properties.iter().any(|property| {
        matches!(property, ObjectPropertyKind::ObjectProperty(property)
            if !property.computed
                && property.key.static_name().is_some_and(|key| key == name))
    })
}

/// A static member path from a name, `this` or `super`, as a refusal
/// quotes it: `Object.keys`, `this.ownerDocument`.
fn path_text(expr: &Expression<'_>) -> Option<String> {
    match unparen(expr) {
        Expression::Identifier(reference) => Some(reference.name.to_string()),
        Expression::ThisExpression(_) => Some("this".to_string()),
        Expression::Super(_) => Some("super".to_string()),
        Expression::StaticMemberExpression(member) => Some(format!(
            "{}.{}",
            path_text(&member.object)?,
            member.property.name
        )),
        _ => None,
    }
}

fn is_function_expression(expr: &Expression<'_>) -> bool {
    matches!(
        unparen(expr),
        Expression::FunctionExpression(_)
            | Expression::ArrowFunctionExpression(_)
            | Expression::ClassExpression(_)
    )
}

/// Whether a property name reads an object's prototype.
fn prototype_name(name: &str) -> bool {
    name == "prototype" || name == "__proto__"
}

/// How a refusal names an expression that yields a prototype.
fn prototype_text(expr: &Expression<'_>) -> String {
    let member = match unparen(expr) {
        Expression::Identifier(reference) => {
            return format!("`{}`, which holds a prototype,", reference.name);
        }
        Expression::CallExpression(_) => {
            return "the prototype `getPrototypeOf` returns".to_string();
        }
        Expression::ChainExpression(chain) => match &chain.expression {
            ChainElement::CallExpression(_) => {
                return "the prototype `getPrototypeOf` returns".to_string();
            }
            other => other.as_member_expression(),
        },
        other => other.as_member_expression(),
    };
    match member.map(|member| unparen(member.object())) {
        Some(Expression::Identifier(owner)) => format!("the prototype of `{}`", owner.name),
        _ => "a prototype".to_string(),
    }
}

/// Writes whose value the scan must see.
fn checked_write(name: &str) -> bool {
    URL_PROPERTIES.contains(&name)
        || (name.len() > 2 && name.starts_with("on"))
        || matches!(name, "cssText" | "style")
}

fn pattern_names<'a>(pattern: &'a BindingPattern<'a>, out: &mut Vec<&'a str>) {
    match pattern {
        BindingPattern::BindingIdentifier(identifier) => out.push(identifier.name.as_str()),
        BindingPattern::ObjectPattern(object) => {
            for property in &object.properties {
                pattern_names(&property.value, out);
            }
            if let Some(rest) = &object.rest {
                pattern_names(&rest.argument, out);
            }
        }
        BindingPattern::ArrayPattern(array) => {
            for element in array.elements.iter().flatten() {
                pattern_names(element, out);
            }
            if let Some(rest) = &array.rest {
                pattern_names(&rest.argument, out);
            }
        }
        BindingPattern::AssignmentPattern(assignment) => pattern_names(&assignment.left, out),
    }
}

/// One target a destructuring assignment writes.
enum Target<'a> {
    /// A name, a member or TypeScript syntax.
    Simple(&'a SimpleAssignmentTarget<'a>),
    /// A shorthand property's name (`u` in `({ u } = o)`), which is both
    /// the key read and the name written.
    Shorthand(&'a IdentifierReference<'a>),
}

/// Every target a destructuring assignment writes, at any depth.
fn collect_targets<'a>(target: &'a AssignmentTarget<'a>, out: &mut Vec<Target<'a>>) {
    if let Some(simple) = target.as_simple_assignment_target() {
        out.push(Target::Simple(simple));
        return;
    }
    match target {
        AssignmentTarget::ArrayAssignmentTarget(array) => {
            for element in array.elements.iter().flatten() {
                collect_maybe_default(element, out);
            }
            if let Some(rest) = &array.rest {
                collect_targets(&rest.target, out);
            }
        }
        AssignmentTarget::ObjectAssignmentTarget(object) => {
            for property in &object.properties {
                match property {
                    AssignmentTargetProperty::AssignmentTargetPropertyIdentifier(shorthand) => {
                        out.push(Target::Shorthand(&shorthand.binding));
                    }
                    AssignmentTargetProperty::AssignmentTargetPropertyProperty(property) => {
                        collect_maybe_default(&property.binding, out);
                    }
                }
            }
            if let Some(rest) = &object.rest {
                collect_targets(&rest.target, out);
            }
        }
        _ => {}
    }
}

fn collect_maybe_default<'a>(
    target: &'a AssignmentTargetMaybeDefault<'a>,
    out: &mut Vec<Target<'a>>,
) {
    match target {
        AssignmentTargetMaybeDefault::AssignmentTargetWithDefault(with_default) => {
            collect_targets(&with_default.binding, out);
        }
        other => {
            if let Some(target) = other.as_assignment_target() {
                collect_targets(target, out);
            }
        }
    }
}

fn product(left: &[String], right: &[String]) -> Option<Vec<String>> {
    if left.len().saturating_mul(right.len()) > MAX_VALUES {
        return None;
    }
    let mut out = Vec::with_capacity(left.len() * right.len());
    for a in left {
        for b in right {
            out.push(format!("{a}{b}"));
        }
    }
    Some(out)
}

fn number_text(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

/// `backgroundImage` as `background-image`.
fn kebab(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for c in name.chars() {
        if c.is_ascii_uppercase() {
            out.push('-');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}
