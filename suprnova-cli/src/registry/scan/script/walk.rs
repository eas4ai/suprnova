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
    READ_ONLY_PROPERTIES, REFUSED_ELEMENTS, REFUSED_PROPERTIES, Rule, URL_ATTRIBUTES,
    URL_CSS_PROPERTIES, URL_PROPERTIES, constructor_rule_for, rule_for,
};

/// How many constant values a traced expression may stand for before the
/// scan stops following it.
const MAX_VALUES: usize = 64;

/// How deep tracing follows bindings and call sites.
const MAX_TRACE_DEPTH: usize = 12;

/// How deep the walker follows nested expressions.
const MAX_DEPTH: usize = 200;

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
    pub written_globals: BTreeSet<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Phase {
    Collect,
    Check,
}

/// Where an expression stands, for the rules that depend on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pos {
    Value,
    Object,
    Typeof,
    Comparison,
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
                            self.declare(id.name.as_str(), Kind::Function);
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
                    self.declare(id.name.as_str(), Kind::Function);
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
                self.expr(&statement.expression, Pos::Value);
            }
            Statement::DoWhileStatement(statement) => {
                self.statement(&statement.body);
                self.expr(&statement.test, Pos::Value);
            }
            Statement::WhileStatement(statement) => {
                self.expr(&statement.test, Pos::Value);
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
                            self.expr(expression, Pos::Value);
                        }
                    }
                    None => {}
                }
                if let Some(test) = &statement.test {
                    self.expr(test, Pos::Value);
                }
                if let Some(update) = &statement.update {
                    self.expr(update, Pos::Value);
                }
                self.statement(&statement.body);
                self.scopes.pop();
            }
            Statement::ForInStatement(statement) => {
                self.expr(&statement.right, Pos::Value);
                self.scopes.push(HashMap::new());
                self.for_left(&statement.left);
                self.statement(&statement.body);
                self.scopes.pop();
            }
            Statement::ForOfStatement(statement) => {
                self.expr(&statement.right, Pos::Value);
                self.scopes.push(HashMap::new());
                self.for_left(&statement.left);
                self.statement(&statement.body);
                self.scopes.pop();
            }
            Statement::IfStatement(statement) => {
                self.expr(&statement.test, Pos::Value);
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
                self.expr(&statement.discriminant, Pos::Value);
                self.scopes.push(HashMap::new());
                for case in &statement.cases {
                    self.hoist_lexical(&case.consequent);
                }
                for case in &statement.cases {
                    if let Some(test) = &case.test {
                        self.expr(test, Pos::Value);
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
                self.value_with_owner(init, owner);
            }
        }
    }

    /// Walks an initializer; a function or arrow assigned to a binding has
    /// that binding as its owner, so its parameters can be traced to its
    /// call sites.
    fn value_with_owner(&mut self, init: &'a Expression<'a>, owner: Option<Bid>) {
        match unparen(init) {
            Expression::FunctionExpression(function) => self.function(function, owner, false),
            Expression::ArrowFunctionExpression(arrow) => self.arrow(arrow, owner),
            _ => self.expr(init, Pos::Value),
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
                self.expr(initializer, Pos::Value);
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
        self.declare("arguments", Kind::Implicit);
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
        for statement in &arrow.body.statements {
            self.statement(statement);
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
                self.expr(expression, Pos::Value);
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
        match self.lookup(name) {
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
                    self.expr(expression, Pos::Value);
                }
            }
            Expression::Identifier(reference) => self.identifier(reference, pos),
            Expression::MetaProperty(_) => {}
            Expression::ThisExpression(this) => {
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
                    Pos::Value
                };
                self.expr(&binary.left, side);
                self.expr(&binary.right, side);
            }
            Expression::PrivateInExpression(private) => self.expr(&private.right, Pos::Comparison),
            Expression::LogicalExpression(logical) => {
                self.expr(&logical.left, Pos::Value);
                self.expr(&logical.right, Pos::Value);
            }
            Expression::ConditionalExpression(conditional) => {
                self.expr(&conditional.test, Pos::Value);
                self.expr(&conditional.consequent, Pos::Value);
                self.expr(&conditional.alternate, Pos::Value);
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
                    self.expr(expression, if index == last { pos } else { Pos::Value });
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
                let inner = if unary.operator == UnaryOperator::Typeof {
                    Pos::Typeof
                } else {
                    Pos::Value
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
                    if property.computed {
                        if let Some(key) = property.key.as_expression() {
                            self.expr(key, Pos::Value);
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

    fn is_global_object(&self, expr: &Expression<'a>) -> bool {
        match unparen(expr) {
            Expression::Identifier(reference) => {
                GLOBAL_OBJECTS.contains(&reference.name.as_str())
                    && self.lookup(reference.name.as_str()).is_none()
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
        self.expr(key, Pos::Value);
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
                            self.expr(&computed.expression, Pos::Value);
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
                            self.expr(&computed.expression, Pos::Value);
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
        let owner = match (assignment.left.as_simple_assignment_target(), simple) {
            (Some(SimpleAssignmentTarget::AssignmentTargetIdentifier(identifier)), true) => {
                self.lookup(identifier.name.as_str())
            }
            _ => None,
        };
        self.value_with_owner(&assignment.right, owner);
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
        for simple in targets {
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
        if let AssignmentTarget::ObjectAssignmentTarget(object) = target {
            for property in &object.properties {
                if let AssignmentTargetProperty::AssignmentTargetPropertyProperty(property) =
                    property
                {
                    if property.computed
                        && let Some(key) = property.name.as_expression()
                    {
                        self.computed_key(key, property.span);
                    } else if let Some(name) = property.name.static_name() {
                        self.property_name(&name, property.span, false);
                    }
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
                        Some(value) if !opaque => {
                            self.url_value(value, identifier.span, "`location`")
                        }
                        _ => self.refuse(
                            "script-url",
                            identifier.span,
                            "`location` is assigned a value the scan cannot check".to_string(),
                        ),
                    },
                    None => self.refuse(
                        "script-global",
                        identifier.span,
                        format!(
                            "assigning `{name}`, which the script does not declare, writes a global"
                        ),
                    ),
                }
            }
            other => {
                if let Some(member) = other.as_member_expression() {
                    self.member_access(member, true);
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

    fn record_write(
        &mut self,
        member: &'a MemberExpression<'a>,
        name: &str,
        value: Option<&'a Expression<'a>>,
    ) {
        if self.is_global_object(member.object()) {
            self.facts.written_globals.insert(name.to_string());
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
        if prototype_chain(member.object()) {
            self.refuse(
                "script-prototype",
                span,
                "assigning a member of a prototype changes it".to_string(),
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
        match unparen(callee) {
            Expression::Identifier(reference) => {
                if self.lookup(reference.name.as_str()).is_none() {
                    self.global(reference.name.as_str(), reference.span, Pos::Value);
                }
            }
            Expression::FunctionExpression(function) => self.function(function, None, false),
            Expression::ArrowFunctionExpression(arrow) => self.arrow(arrow, None),
            other => {
                if let Some(member) = other.as_member_expression() {
                    self.member_access(member, false);
                } else {
                    self.expr(other, Pos::Value);
                }
            }
        }
    }

    fn call(&mut self, call: &'a CallExpression<'a>) {
        let callee = unparen(&call.callee);
        if let Expression::Identifier(reference) = callee
            && let Some(id) = self.lookup(reference.name.as_str())
            && self.phase == Phase::Collect
            && let Some(binding) = self.facts.bindings.get_mut(id)
        {
            binding.calls.push(&call.arguments);
        }
        self.callee_walk(&call.callee);
        for argument in &call.arguments {
            match argument {
                Argument::SpreadElement(spread) => self.expr(&spread.argument, Pos::Value),
                other => {
                    if let Some(expression) = other.as_expression() {
                        self.expr(expression, Pos::Value);
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
            Expression::Identifier(reference) if self.lookup(reference.name.as_str()).is_none() => {
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
                let Some(id) = self.lookup(reference.name.as_str()) else {
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
                            .lookup(class.name.as_str())
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
                match self.lookup(name) {
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
                if reference.name == "undefined" && self.lookup("undefined").is_none() =>
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
        for argument in &new.arguments {
            match argument {
                Argument::SpreadElement(spread) => self.expr(&spread.argument, Pos::Value),
                other => {
                    if let Some(expression) = other.as_expression() {
                        self.expr(expression, Pos::Value);
                    }
                }
            }
        }
        if !self.check() {
            return;
        }
        let name = match callee {
            Expression::Identifier(reference) if self.lookup(reference.name.as_str()).is_none() => {
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
                match self.lookup(name) {
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
                self.expr(other, Pos::Value);
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
        let Some(binding) = self
            .lookup(reference.name.as_str())
            .and_then(|id| self.binding(id))
        else {
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
            let mut segments: Vec<&str> = base.trim_matches('/').split('/').collect();
            segments.pop();
            while let Some(next) = rest.strip_prefix("../") {
                segments.pop();
                rest = next;
            }
            Some(format!("/{}/{rest}", segments.join("/")))
        } else if specifier.starts_with('/') && !specifier.starts_with("//") {
            Some(specifier.to_string())
        } else {
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
                format!("`import \"{specifier}\"` loads a module from outside the component and the components it depends on"),
            );
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
                    if callee.name == "String" && self.lookup("String").is_none()) =>
            {
                match call.arguments.first() {
                    Some(argument) => self.trace(argument.as_expression()?, depth + 1)?,
                    None => vec![String::new()],
                }
            }
            Expression::Identifier(reference) => {
                let name = reference.name.as_str();
                match self.lookup(name) {
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
                let Some(binding) = self
                    .lookup(reference.name.as_str())
                    .and_then(|id| self.binding(id))
                else {
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

fn is_function_expression(expr: &Expression<'_>) -> bool {
    matches!(
        unparen(expr),
        Expression::FunctionExpression(_)
            | Expression::ArrowFunctionExpression(_)
            | Expression::ClassExpression(_)
    )
}

/// Whether a member write's object is reached through a `prototype`.
fn prototype_chain(object: &Expression<'_>) -> bool {
    match unparen(object) {
        Expression::StaticMemberExpression(member) => {
            member.property.name == "prototype"
                || member.property.name == "__proto__"
                || prototype_chain(&member.object)
        }
        Expression::ComputedMemberExpression(member) => prototype_chain(&member.object),
        _ => false,
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

fn collect_targets<'a>(
    target: &'a AssignmentTarget<'a>,
    out: &mut Vec<&'a SimpleAssignmentTarget<'a>>,
) {
    if let Some(simple) = target.as_simple_assignment_target() {
        out.push(simple);
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
                    AssignmentTargetProperty::AssignmentTargetPropertyIdentifier(identifier) => {
                        let _ = identifier;
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
    out: &mut Vec<&'a SimpleAssignmentTarget<'a>>,
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
