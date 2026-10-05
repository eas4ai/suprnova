//! The walker: visits every item, type, pattern and expression of the
//! component's Rust, resolves each path through the component's own modules
//! and `use` declarations, and classifies it against the allowlist. A path
//! it cannot resolve to something admitted is refused where it is written.

use std::collections::{BTreeMap, BTreeSet};

use syn::spanned::Spanned;

use super::modules::{
    Def, Module, ModuleTable, Path, TypeImpls, UseDecl, collect_use, fields_of, join, line_of,
    segments, unraw,
};
use super::ty::{Bound, PRIMITIVES, STD_TRAIT_METHODS, Ty, closure_params, std_method_result};
use crate::registry::Capability;
use crate::registry::scan::Finding;
use crate::registry::scan::allowlist::{Admission, Allowlist, STD_ALLOWED};

/// How deep the walker follows nested expressions, types and patterns
/// before it refuses the file rather than risk its own stack.
const MAX_DEPTH: usize = 160;

/// Container calls that resolve a service by its type: the type must be
/// named so its capability is known (REG-030).
const CONTAINER_RESOLVERS: &[&str] = &[
    "suprnova::App::make",
    "suprnova::App::get",
    "suprnova::App::resolve",
    "suprnova::App::resolve_make",
    "suprnova::App::has",
    "suprnova::App::bound",
    "suprnova::App::has_binding",
    "suprnova::App::bound_binding",
    "suprnova::Container::make",
    "suprnova::Container::get",
    "suprnova::Container::has",
    "suprnova::Container::has_binding",
];

/// The prelude names a component may write without importing them, and
/// the `std` path each one names.
const PRELUDE: &[(&str, &str)] = &[
    ("Option", "std::option::Option"),
    ("Some", "std::option::Option::Some"),
    ("None", "std::option::Option::None"),
    ("Result", "std::result::Result"),
    ("Ok", "std::result::Result::Ok"),
    ("Err", "std::result::Result::Err"),
    ("String", "std::string::String"),
    ("ToString", "std::string::ToString"),
    ("Vec", "std::vec::Vec"),
    ("Box", "std::boxed::Box"),
    ("ToOwned", "std::borrow::ToOwned"),
    ("Clone", "std::clone::Clone"),
    ("Copy", "std::marker::Copy"),
    ("Send", "std::marker::Send"),
    ("Sync", "std::marker::Sync"),
    ("Sized", "std::marker::Sized"),
    ("Unpin", "std::marker::Unpin"),
    ("Default", "std::default::Default"),
    ("Drop", "std::ops::Drop"),
    ("Fn", "std::ops::Fn"),
    ("FnMut", "std::ops::FnMut"),
    ("FnOnce", "std::ops::FnOnce"),
    ("AsRef", "std::convert::AsRef"),
    ("AsMut", "std::convert::AsMut"),
    ("From", "std::convert::From"),
    ("Into", "std::convert::Into"),
    ("TryFrom", "std::convert::TryFrom"),
    ("TryInto", "std::convert::TryInto"),
    ("Iterator", "std::iter::Iterator"),
    ("IntoIterator", "std::iter::IntoIterator"),
    ("Extend", "std::iter::Extend"),
    ("DoubleEndedIterator", "std::iter::DoubleEndedIterator"),
    ("ExactSizeIterator", "std::iter::ExactSizeIterator"),
    ("FromIterator", "std::iter::FromIterator"),
    ("PartialEq", "std::cmp::PartialEq"),
    ("Eq", "std::cmp::Eq"),
    ("PartialOrd", "std::cmp::PartialOrd"),
    ("Ord", "std::cmp::Ord"),
    ("drop", "std::mem::drop"),
    ("Future", "std::future::Future"),
    ("IntoFuture", "std::future::IntoFuture"),
];

/// The crate roots a path may start with.
const CRATE_ROOTS: &[&str] = &["suprnova", "std", "core", "alloc"];

/// One Live component the Rust defines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DefinedComponent {
    /// `<module>::<Type>`, relative to the namespace module.
    pub path: String,
    /// The file that defines it.
    pub file: String,
    /// The line of its struct.
    pub line: Option<u32>,
}

/// Which namespace a path is looked up in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Ns {
    /// A type, trait or module.
    Type,
    /// A value, function or constructor.
    Value,
}

/// What a written path resolved to.
#[derive(Debug, Clone)]
pub(super) enum Res {
    /// A local binding.
    Local(Ty),
    /// A generic parameter, by its bounds.
    Generic(Vec<Bound>),
    /// A full path.
    Full(Path),
    /// A primitive type and the segments after it.
    Prim(String),
    /// The path names nothing the component defines or may import; the
    /// written path is kept for the message.
    Unresolved(Path),
}

/// How a full path was classified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Class {
    /// One of the component's own items.
    Own,
    /// A module of a component it depends on.
    Dependency,
    /// A Suprnova item or a path under a re-export, by its path.
    Api {
        /// The path the allowlist admitted (canonical for an item).
        path: String,
        /// Whether the path sits under a re-exported crate or type.
        prefix: bool,
    },
    /// A Suprnova module.
    ApiModule,
    /// An effect-free `std` path, written with a `std::` root.
    Std(String),
    /// A `std` module that holds admitted paths.
    StdModule,
    /// A primitive type.
    Prim(String),
    /// Refused; a finding was recorded.
    Refused,
}

/// One block scope: its locals and the items and imports it declares.
#[derive(Clone, Default)]
struct Scope {
    locals: Vec<(String, Ty)>,
    module: Module,
    path: Path,
}

/// What the walker is visiting the attributes of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Site {
    /// A struct.
    Struct,
    /// A struct field.
    Field,
    /// An enum variant.
    Variant,
    /// A `#[live]` impl block.
    Impl,
    /// A method of a `#[live]` impl block.
    LiveMethod,
    /// Any other item, statement or expression.
    Other,
}

/// The walker's state for one component.
pub(crate) struct Walker<'a> {
    pub(super) allowlist: &'a Allowlist,
    pub(super) namespace: &'a str,
    pub(super) views: BTreeSet<String>,
    pub(super) dependency_modules: Vec<Path>,
    pub(super) table: &'a ModuleTable,
    pub(super) impls: BTreeMap<String, TypeImpls>,
    pub(super) file: String,
    pub(super) module: Path,
    scopes: Vec<Scope>,
    extra_modules: BTreeMap<Path, Module>,
    generics: Vec<BTreeMap<String, Vec<Bound>>>,
    pub(super) self_ty: Vec<Ty>,
    depth: usize,
    pub(super) silent: usize,
    block_counter: usize,
    pub(super) serde_derive: Vec<bool>,
    pub(crate) capabilities: BTreeSet<Capability>,
    /// The items whose bodies are being visited: a function or method, a
    /// static or a const, each keyed as [`Walker::item_capabilities`] is.
    item_keys: Vec<String>,
    /// The capabilities each of the component's own functions, methods,
    /// statics and consts reaches in its own body, keyed by its full path
    /// under `crate::live::` for a free item and by its bare name for a
    /// method, so the view scan can refuse a call that carries one
    /// (REG-031). Calls between them are joined by the view scan.
    pub(crate) item_capabilities: BTreeMap<String, BTreeSet<Capability>>,
    pub(crate) findings: Vec<Finding>,
    pub(crate) defined: Vec<DefinedComponent>,
}

impl<'a> Walker<'a> {
    /// A walker over a module table.
    pub(super) fn new(
        allowlist: &'a Allowlist,
        namespace: &'a str,
        views: BTreeSet<String>,
        dependency_modules: Vec<Path>,
        table: &'a ModuleTable,
    ) -> Self {
        Walker {
            allowlist,
            namespace,
            views,
            dependency_modules,
            table,
            impls: BTreeMap::new(),
            file: String::new(),
            module: Vec::new(),
            scopes: Vec::new(),
            extra_modules: BTreeMap::new(),
            generics: Vec::new(),
            self_ty: Vec::new(),
            depth: 0,
            silent: 0,
            block_counter: 0,
            serde_derive: Vec::new(),
            capabilities: BTreeSet::new(),
            item_keys: Vec::new(),
            item_capabilities: BTreeMap::new(),
            findings: Vec::new(),
            defined: Vec::new(),
        }
    }

    // ----- findings -------------------------------------------------------

    /// Records a refusal at a span, unless the walker is only typing.
    pub(super) fn refuse(&mut self, check: &'static str, span: proc_macro2::Span, message: String) {
        if self.silent > 0 {
            return;
        }
        let finding = Finding {
            check,
            file: self.file.clone(),
            line: line_of(span),
            message,
        };
        if !self.findings.contains(&finding) {
            self.findings.push(finding);
        }
    }

    fn grant(&mut self, capability: Option<Capability>) {
        if self.silent == 0
            && let Some(capability) = capability
        {
            self.capabilities.insert(capability);
            for key in &self.item_keys {
                self.item_capabilities
                    .entry(key.clone())
                    .or_default()
                    .insert(capability);
            }
        }
    }

    /// Visits an item's body with its key on the stack, so every
    /// capability granted inside is recorded against it.
    fn with_item_key(&mut self, key: String, visit: impl FnOnce(&mut Self)) {
        self.item_capabilities.entry(key.clone()).or_default();
        self.item_keys.push(key);
        visit(self);
        self.item_keys.pop();
    }

    /// The key of a free item named `name` in the current module.
    fn free_item_key(&self, name: &syn::Ident) -> String {
        format!("{}::{}", self.module.join("::"), unraw(name))
    }

    // ----- impl table -----------------------------------------------------

    /// Resolves every `impl` block's self type and records its methods and
    /// traits under the own type it extends.
    pub(super) fn gather_impls(&mut self) {
        for (self_ty, trait_path, items, module) in &self.table.impls {
            let syn::Type::Path(type_path) = self_ty else {
                continue;
            };
            if type_path.qself.is_some() {
                continue;
            }
            let saved = std::mem::replace(&mut self.module, module.clone());
            self.silent += 1;
            let resolved = self.resolve(&type_path.path, Ns::Type);
            self.silent -= 1;
            self.module = saved;
            let Res::Full(full) = resolved else {
                continue;
            };
            if !self.table.is_own(&full) {
                continue;
            }
            let entry = self.impls.entry(join(&full)).or_default();
            if let Some(trait_path) = trait_path {
                entry.traits.push((trait_path.clone(), module.clone()));
            }
            for item in items {
                if let syn::ImplItem::Fn(method) = item {
                    entry.methods.insert(
                        unraw(&method.sig.ident),
                        (method.sig.clone(), module.clone()),
                    );
                }
            }
        }
    }

    // ----- resolution -----------------------------------------------------

    fn module_at(&self, path: &[String]) -> Option<&Module> {
        self.table
            .modules
            .get(path)
            .or_else(|| self.extra_modules.get(path))
    }

    /// Resolves a `use` declaration made in `module`.
    pub(super) fn resolve_use(
        &self,
        module: &[String],
        decl: &UseDecl,
        guard: usize,
    ) -> Result<Path, Path> {
        let segments = &decl.segments;
        let Some(first) = segments.first() else {
            return Err(segments.clone());
        };
        if guard > 24 {
            return Err(segments.clone());
        }
        if decl.leading_colon {
            return if CRATE_ROOTS.contains(&first.as_str()) {
                Ok(segments.clone())
            } else {
                Err(segments.clone())
            };
        }
        match first.as_str() {
            "crate" => Ok(segments.clone()),
            "self" => {
                let mut full = module.to_vec();
                full.extend(segments[1..].iter().cloned());
                Ok(full)
            }
            "super" => {
                let mut full = module.to_vec();
                let mut rest = segments.as_slice();
                while rest.first().is_some_and(|segment| segment == "super") {
                    if full.len() <= 1 {
                        return Err(segments.clone());
                    }
                    full.pop();
                    rest = &rest[1..];
                }
                full.extend(rest.iter().cloned());
                Ok(full)
            }
            name => {
                if let Some(found) = self.lookup_in_module(module, name, guard + 1) {
                    let mut full = found;
                    full.extend(segments[1..].iter().cloned());
                    Ok(full)
                } else if CRATE_ROOTS.contains(&name) {
                    Ok(segments.clone())
                } else {
                    Err(segments.clone())
                }
            }
        }
    }

    /// The full path `name` binds in a module, through its definitions,
    /// imports and admitted glob imports.
    fn lookup_in_module(&self, module: &[String], name: &str, guard: usize) -> Option<Path> {
        let entry = self.module_at(module)?;
        if entry.defs.contains_key(name) {
            let mut full = module.to_vec();
            full.push(name.to_string());
            return Some(full);
        }
        if let Some(decl) = entry.uses.get(name) {
            return self.resolve_use(module, decl, guard).ok();
        }
        for glob in &entry.globs {
            let Ok(target) = self.resolve_use(module, glob, guard) else {
                continue;
            };
            if self.table.is_own(&target) {
                if let Some(found) = self.lookup_in_module(&target, name, guard + 1) {
                    return Some(found);
                }
                continue;
            }
            if self.glob_is_std(&target) || self.is_dependency(&target) {
                let mut full = target;
                full.push(name.to_string());
                return Some(full);
            }
        }
        None
    }

    fn glob_is_std(&self, target: &[String]) -> bool {
        match target.first().map(String::as_str) {
            Some("std" | "core" | "alloc") => {
                let normalized = std_normalized(target);
                STD_ALLOWED.iter().any(|entry| {
                    normalized == *entry || normalized.starts_with(&format!("{entry}::"))
                })
            }
            _ => false,
        }
    }

    fn is_dependency(&self, path: &[String]) -> bool {
        self.dependency_modules
            .iter()
            .any(|module| path.len() >= module.len() && path[..module.len()] == module[..])
    }

    /// Resolves a written path to what it names.
    pub(super) fn resolve(&mut self, path: &syn::Path, ns: Ns) -> Res {
        let written = segments(path);
        self.resolve_segments(&written, path.leading_colon.is_some(), ns)
    }

    pub(super) fn resolve_segments(
        &mut self,
        written: &[String],
        leading_colon: bool,
        ns: Ns,
    ) -> Res {
        let Some(first) = written.first() else {
            return Res::Unresolved(written.to_vec());
        };
        let rest = &written[1..];
        let with_rest = |mut base: Path| {
            base.extend(rest.iter().cloned());
            base
        };
        if leading_colon {
            return if CRATE_ROOTS.contains(&first.as_str()) {
                Res::Full(self.expand(written.to_vec()))
            } else {
                Res::Unresolved(written.to_vec())
            };
        }
        match first.as_str() {
            "crate" => return Res::Full(self.expand(written.to_vec())),
            "self" if written.len() > 1 || ns == Ns::Type => {
                return Res::Full(self.expand(with_rest(self.module.clone())));
            }
            "super" => {
                let decl = UseDecl {
                    segments: written.to_vec(),
                    leading_colon: false,
                    line: None,
                };
                return match self.resolve_use(&self.module.clone(), &decl, 0) {
                    Ok(full) => Res::Full(self.expand(full)),
                    Err(written) => Res::Unresolved(written),
                };
            }
            "Self" => {
                let Some(self_ty) = self.self_ty.last().cloned() else {
                    return Res::Unresolved(written.to_vec());
                };
                return match self_ty {
                    Ty::Own(path) | Ty::Api(path) => {
                        let base: Path = path.split("::").map(str::to_string).collect();
                        Res::Full(self.expand(with_rest(base)))
                    }
                    Ty::Prim(name) => Res::Prim(name),
                    Ty::Std(name, _) => match std_path_of(&name) {
                        Some(path) => Res::Full(with_rest(path)),
                        None => Res::Unresolved(written.to_vec()),
                    },
                    _ => Res::Unresolved(written.to_vec()),
                };
            }
            _ => {}
        }
        if ns == Ns::Value && written.len() == 1 {
            for scope in self.scopes.iter().rev() {
                if let Some((_, ty)) = scope.locals.iter().rev().find(|(name, _)| name == first) {
                    return Res::Local(ty.clone());
                }
            }
            if first == "self" {
                return Res::Unresolved(written.to_vec());
            }
        }
        for frame in self.generics.iter().rev() {
            if let Some(bounds) = frame.get(first) {
                return Res::Generic(bounds.clone());
            }
        }
        let scope_paths: Vec<Path> = self
            .scopes
            .iter()
            .rev()
            .map(|scope| scope.path.clone())
            .collect();
        for scope_path in scope_paths {
            if let Some(found) = self.lookup_in_module(&scope_path, first, 0) {
                return Res::Full(self.expand(with_rest(found)));
            }
        }
        if let Some(found) = self.lookup_in_module(&self.module.clone(), first, 0) {
            return Res::Full(self.expand(with_rest(found)));
        }
        if let Some((_, std_path)) = PRELUDE.iter().find(|(name, _)| name == first) {
            let base: Path = std_path.split("::").map(str::to_string).collect();
            return Res::Full(with_rest(base));
        }
        if PRIMITIVES.contains(&first.as_str()) {
            return Res::Prim(first.clone());
        }
        if CRATE_ROOTS.contains(&first.as_str()) {
            return Res::Full(written.to_vec());
        }
        Res::Unresolved(written.to_vec())
    }

    /// Follows the component's own `use` re-exports, so an alias of a
    /// refused path is classified as that path.
    pub(super) fn expand(&self, mut full: Path) -> Path {
        for _ in 0..32 {
            let Some(module) = self.table.owning_module(&full).cloned() else {
                return full;
            };
            let name = full[module.len()].clone();
            let Some(entry) = self.module_at(&module) else {
                return full;
            };
            if let Some(def) = entry.defs.get(&name) {
                // `Alias::item` names an item of the aliased type: follow
                // the alias, so `type H = TrustedHtml; H::framework_static`
                // is classified as the constructor it is.
                let target = match def {
                    Def::Alias(ty) if full.len() > module.len() + 1 => match &**ty {
                        syn::Type::Path(type_path) if type_path.qself.is_none() => {
                            let decl = UseDecl {
                                segments: segments(&type_path.path),
                                leading_colon: type_path.path.leading_colon.is_some(),
                                line: None,
                            };
                            self.resolve_use(&module, &decl, 0).ok()
                        }
                        _ => None,
                    },
                    _ => None,
                };
                let Some(target) = target else {
                    return full;
                };
                let rest: Path = full[module.len() + 1..].to_vec();
                full = target;
                full.extend(rest);
                continue;
            }
            let replacement = if let Some(decl) = entry.uses.get(&name) {
                self.resolve_use(&module, decl, 0).ok()
            } else {
                self.lookup_in_module(&module, &name, 0)
            };
            match replacement {
                Some(target) if target != full[..=module.len()] => {
                    let rest: Path = full[module.len() + 1..].to_vec();
                    full = target;
                    full.extend(rest);
                }
                _ => return full,
            }
        }
        full
    }

    // ----- classification -------------------------------------------------

    /// Classifies a full path, recording its capability or refusal.
    pub(super) fn classify(&mut self, full: &[String], span: proc_macro2::Span) -> Class {
        let text = join(full);
        if let Some(index) = full.iter().position(|segment| segment == "TrustedHtml")
            && index + 1 < full.len()
        {
            self.refuse(
                "rust-trusted-html",
                span,
                format!(
                    "`{text}` constructs a TrustedHtml; only the framework builds trusted markup"
                ),
            );
            return Class::Refused;
        }
        match full.first().map(String::as_str) {
            Some("crate") => {
                if self.table.is_own(full) {
                    Class::Own
                } else if self.is_dependency(full)
                    || self.extra_modules.keys().any(|m| full.starts_with(m))
                {
                    if self.is_dependency(full) {
                        Class::Dependency
                    } else {
                        Class::Own
                    }
                } else {
                    self.refuse(
                        "rust-path",
                        span,
                        format!("`{text}` names application code outside the component and the components it depends on"),
                    );
                    Class::Refused
                }
            }
            Some("suprnova") => {
                match self.allowlist.admit(&text) {
                    Some(Admission::Item { canonical, item }) => {
                        if item.hidden {
                            self.refuse(
                                "rust-hidden",
                                span,
                                format!("`{text}` is hidden (`#[doc(hidden)]`), not Suprnova API"),
                            );
                            return Class::Refused;
                        }
                        let canonical = canonical.to_string();
                        self.grant(item.capability);
                        Class::Api {
                            path: canonical,
                            prefix: false,
                        }
                    }
                    Some(Admission::Prefix { root, item }) => {
                        if item.hidden {
                            self.refuse("rust-hidden", span, format!("`{text}` is under `{root}`, a hidden re-export, not Suprnova API"));
                            return Class::Refused;
                        }
                        self.grant(item.capability);
                        Class::Api {
                            path: text,
                            prefix: true,
                        }
                    }
                    Some(Admission::Module) => Class::ApiModule,
                    Some(Admission::Refused { canonical }) => {
                        let canonical = canonical.to_string();
                        self.refuse(
                        "rust-path",
                        span,
                        format!("`{canonical}` is Suprnova API the registry does not admit: its capability is undecided or it changes the application"),
                    );
                        Class::Refused
                    }
                    None => {
                        self.refuse(
                            "rust-path",
                            span,
                            format!(
                                "`{text}` is not on the allowlist of Suprnova's documented API"
                            ),
                        );
                        Class::Refused
                    }
                }
            }
            Some("std" | "core" | "alloc") => {
                let normalized = std_normalized(full);
                if STD_ALLOWED.iter().any(|entry| {
                    normalized == *entry || normalized.starts_with(&format!("{entry}::"))
                }) {
                    Class::Std(normalized)
                } else if STD_ALLOWED
                    .iter()
                    .any(|entry| entry.starts_with(&format!("{normalized}::")))
                {
                    Class::StdModule
                } else {
                    self.refuse(
                        "rust-path",
                        span,
                        format!(
                            "`{text}` is outside the effect-free part of std a component may name"
                        ),
                    );
                    Class::Refused
                }
            }
            _ => {
                self.refuse(
                    "rust-path",
                    span,
                    format!("`{text}` names a crate Suprnova does not re-export"),
                );
                Class::Refused
            }
        }
    }

    /// Resolves and classifies a written path in one step.
    pub(super) fn check_path(&mut self, path: &syn::Path, ns: Ns) -> (Res, Class) {
        let res = self.resolve(path, ns);
        let class = self.classify_res(&res, path.span());
        (res, class)
    }

    pub(super) fn classify_res(&mut self, res: &Res, span: proc_macro2::Span) -> Class {
        match res {
            Res::Local(_) | Res::Generic(_) => Class::Own,
            Res::Prim(name) => Class::Prim(name.clone()),
            Res::Full(full) => self.classify(&full.clone(), span),
            Res::Unresolved(written) => {
                let text = join(written);
                self.refuse(
                    "rust-path",
                    span,
                    format!("`{text}` names nothing the component defines or imports from Suprnova or the effect-free part of std"),
                );
                Class::Refused
            }
        }
    }

    // ----- types ----------------------------------------------------------

    /// The type a resolved type path names.
    fn ty_of_path(&mut self, res: &Res, class: &Class, args: Vec<Ty>) -> Ty {
        match (res, class) {
            (Res::Generic(bounds), _) => Ty::Bounded(bounds.clone()),
            (Res::Prim(name), _) => Ty::Prim(name.clone()),
            (_, Class::Prim(name)) => Ty::Prim(name.clone()),
            (Res::Full(full), Class::Std(_)) => {
                let name = full.last().cloned().unwrap_or_default();
                if name == "Duration" || name == "Ordering" {
                    Ty::Std(name, Vec::new())
                } else {
                    Ty::Std(name, args)
                }
            }
            (Res::Full(full), Class::Own) => self.own_type(full),
            (Res::Full(full), Class::Dependency) => Ty::Api(format!("dependency:{}", join(full))),
            (_, Class::Api { path, .. }) => Ty::Api(path.clone()),
            _ => Ty::Unknown,
        }
    }

    fn own_type(&mut self, full: &[String]) -> Ty {
        let Some((name, module)) = full.split_last() else {
            return Ty::Unknown;
        };
        match self
            .module_at(module)
            .and_then(|entry| entry.defs.get(name))
            .cloned()
        {
            Some(Def::Alias(target)) => self.type_in_module(&target, module, None),
            Some(Def::Struct(_) | Def::Enum | Def::Other) => Ty::Own(join(full)),
            Some(Def::Trait(_)) => Ty::Bounded(vec![Bound::Own(join(full))]),
            _ => Ty::Own(join(full)),
        }
    }

    /// The type a `syn::Type` names, written in another module, without
    /// recording findings (they are recorded where the type is written).
    pub(super) fn type_in_module(
        &mut self,
        ty: &syn::Type,
        module: &[String],
        self_ty: Option<Ty>,
    ) -> Ty {
        let saved_module = std::mem::replace(&mut self.module, module.to_vec());
        let saved_scopes = std::mem::take(&mut self.scopes);
        let pushed = self_ty.is_some();
        if let Some(self_ty) = self_ty {
            self.self_ty.push(self_ty);
        }
        self.silent += 1;
        let result = self.visit_type(ty);
        self.silent -= 1;
        if pushed {
            self.self_ty.pop();
        }
        self.scopes = saved_scopes;
        self.module = saved_module;
        result
    }

    /// Visits a type: classifies every path in it and returns what it
    /// names.
    pub(super) fn visit_type(&mut self, ty: &syn::Type) -> Ty {
        if !self.enter(ty.span()) {
            return Ty::Unknown;
        }
        let result = match ty {
            syn::Type::Path(type_path) => {
                if let Some(qself) = &type_path.qself {
                    self.visit_type(&qself.ty);
                    self.check_path_args(&type_path.path);
                    self.check_path(&type_path.path, Ns::Type);
                    Ty::Unknown
                } else {
                    let args = self.check_path_args(&type_path.path);
                    let (res, class) = self.check_path(&type_path.path, Ns::Type);
                    self.ty_of_path(&res, &class, args)
                }
            }
            syn::Type::Reference(reference) => self.visit_type(&reference.elem),
            syn::Type::Slice(slice) => Ty::Slice(Box::new(self.visit_type(&slice.elem))),
            syn::Type::Array(array) => {
                self.visit_expr(&array.len, None);
                Ty::Slice(Box::new(self.visit_type(&array.elem)))
            }
            syn::Type::Tuple(tuple) => {
                if tuple.elems.is_empty() {
                    Ty::Unit
                } else {
                    Ty::Tuple(
                        tuple
                            .elems
                            .iter()
                            .map(|elem| self.visit_type(elem))
                            .collect(),
                    )
                }
            }
            syn::Type::Paren(paren) => self.visit_type(&paren.elem),
            syn::Type::Group(group) => self.visit_type(&group.elem),
            syn::Type::ImplTrait(impl_trait) => {
                Ty::Bounded(self.visit_bounds(impl_trait.bounds.iter()))
            }
            syn::Type::TraitObject(object) => Ty::Bounded(self.visit_bounds(object.bounds.iter())),
            syn::Type::BareFn(bare) => {
                if bare.unsafety.is_some() {
                    self.refuse(
                        "rust-unsafe",
                        bare.span(),
                        "an `unsafe fn` pointer type".to_string(),
                    );
                }
                if bare.abi.is_some() {
                    self.refuse(
                        "rust-extern",
                        bare.span(),
                        "an `extern` function pointer type".to_string(),
                    );
                }
                for input in &bare.inputs {
                    self.visit_type(&input.ty);
                }
                match &bare.output {
                    syn::ReturnType::Type(_, output) => {
                        Ty::Closure(Box::new(self.visit_type(output)))
                    }
                    syn::ReturnType::Default => Ty::Closure(Box::new(Ty::Unit)),
                }
            }
            syn::Type::Never(_) | syn::Type::Infer(_) => Ty::Unknown,
            syn::Type::Ptr(pointer) => {
                self.refuse(
                    "rust-construct",
                    pointer.span(),
                    "a raw pointer type".to_string(),
                );
                Ty::Unknown
            }
            syn::Type::Macro(mac) => {
                self.refuse(
                    "rust-macro",
                    mac.span(),
                    format!(
                        "the macro `{}!` in a type position",
                        join(&segments(&mac.mac.path))
                    ),
                );
                Ty::Unknown
            }
            _ => {
                self.refuse(
                    "rust-construct",
                    ty.span(),
                    "a type the scan cannot classify".to_string(),
                );
                Ty::Unknown
            }
        };
        self.leave();
        result
    }

    /// Classifies the generic arguments of every segment of a path and
    /// returns the last segment's type arguments.
    pub(super) fn check_path_args(&mut self, path: &syn::Path) -> Vec<Ty> {
        let mut last = Vec::new();
        for segment in &path.segments {
            last.clear();
            match &segment.arguments {
                syn::PathArguments::None => {}
                syn::PathArguments::AngleBracketed(angle) => {
                    for arg in &angle.args {
                        match arg {
                            syn::GenericArgument::Type(ty) => last.push(self.visit_type(ty)),
                            syn::GenericArgument::Const(expr) => {
                                self.visit_expr(expr, None);
                            }
                            syn::GenericArgument::AssocType(assoc) => {
                                self.visit_type(&assoc.ty);
                            }
                            syn::GenericArgument::AssocConst(assoc) => {
                                self.visit_expr(&assoc.value, None);
                            }
                            syn::GenericArgument::Constraint(constraint) => {
                                self.visit_bounds(constraint.bounds.iter());
                            }
                            syn::GenericArgument::Lifetime(_) => {}
                            _ => self.refuse(
                                "rust-construct",
                                arg.span(),
                                "a generic argument the scan cannot classify".to_string(),
                            ),
                        }
                    }
                }
                syn::PathArguments::Parenthesized(parenthesized) => {
                    for input in &parenthesized.inputs {
                        self.visit_type(input);
                    }
                    if let syn::ReturnType::Type(_, output) = &parenthesized.output {
                        self.visit_type(output);
                    }
                }
            }
        }
        last
    }

    /// Classifies trait bounds and returns them.
    pub(super) fn visit_bounds<'b>(
        &mut self,
        bounds: impl Iterator<Item = &'b syn::TypeParamBound>,
    ) -> Vec<Bound> {
        let mut out = Vec::new();
        for bound in bounds {
            match bound {
                syn::TypeParamBound::Trait(trait_bound) => {
                    self.check_path_args(&trait_bound.path);
                    let (res, class) = self.check_path(&trait_bound.path, Ns::Type);
                    match (res, class) {
                        (Res::Full(full), Class::Own) => out.push(Bound::Own(join(&full))),
                        (_, Class::Api { path, .. }) => out.push(Bound::Api(path)),
                        (_, Class::Std(path)) => out.push(Bound::Std(path)),
                        _ => {}
                    }
                }
                syn::TypeParamBound::Lifetime(_) | syn::TypeParamBound::PreciseCapture(_) => {}
                _ => self.refuse(
                    "rust-construct",
                    bound.span(),
                    "a bound the scan cannot classify".to_string(),
                ),
            }
        }
        out
    }

    // ----- depth ----------------------------------------------------------

    fn enter(&mut self, span: proc_macro2::Span) -> bool {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            self.depth -= 1;
            self.refuse(
                "rust-limit",
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

    // ----- scopes ---------------------------------------------------------

    fn push_scope(&mut self) {
        self.block_counter += 1;
        let mut path = self
            .scopes
            .last()
            .map(|scope| scope.path.clone())
            .unwrap_or_else(|| self.module.clone());
        path.push(format!("{{block {}}}", self.block_counter));
        self.scopes.push(Scope {
            locals: Vec::new(),
            module: Module::default(),
            path,
        });
    }

    fn pop_scope(&mut self) {
        if let Some(scope) = self.scopes.pop() {
            self.extra_modules.insert(scope.path, scope.module);
        }
    }

    pub(super) fn bind(&mut self, name: String, ty: Ty) {
        if self.scopes.is_empty() {
            self.push_scope();
        }
        if let Some(scope) = self.scopes.last_mut() {
            scope.locals.push((name, ty));
        }
    }

    /// Registers the items and imports a block declares, so names resolve
    /// to them anywhere in the block.
    fn declare_block_items(&mut self, stmts: &[syn::Stmt]) {
        let Some(scope) = self.scopes.last_mut() else {
            return;
        };
        for stmt in stmts {
            if let syn::Stmt::Item(item) = stmt {
                match item {
                    syn::Item::Use(item_use) => collect_use(
                        &item_use.tree,
                        &mut Vec::new(),
                        item_use.leading_colon.is_some(),
                        &mut scope.module,
                    ),
                    syn::Item::Struct(item) => {
                        scope
                            .module
                            .defs
                            .insert(unraw(&item.ident), Def::Struct(fields_of(&item.fields)));
                    }
                    syn::Item::Enum(item) => {
                        scope.module.defs.insert(unraw(&item.ident), Def::Enum);
                    }
                    syn::Item::Fn(item) => {
                        scope
                            .module
                            .defs
                            .insert(unraw(&item.sig.ident), Def::Fn(Box::new(item.sig.clone())));
                    }
                    syn::Item::Const(item) => {
                        scope
                            .module
                            .defs
                            .insert(unraw(&item.ident), Def::Value(item.ty.clone()));
                    }
                    syn::Item::Static(item) => {
                        scope
                            .module
                            .defs
                            .insert(unraw(&item.ident), Def::Value(item.ty.clone()));
                    }
                    syn::Item::Type(item) => {
                        scope
                            .module
                            .defs
                            .insert(unraw(&item.ident), Def::Alias(item.ty.clone()));
                    }
                    syn::Item::Trait(item) => {
                        scope
                            .module
                            .defs
                            .insert(unraw(&item.ident), Def::Trait(BTreeMap::new()));
                    }
                    _ => {}
                }
            }
        }
        let path = scope.path.clone();
        let module = scope.module.clone();
        self.extra_modules.insert(path, module);
    }

    // ----- items ----------------------------------------------------------

    /// Visits the items of one module.
    pub(super) fn visit_items(&mut self, items: &[syn::Item]) {
        for item in items {
            self.visit_item(item);
        }
    }

    pub(super) fn visit_item(&mut self, item: &syn::Item) {
        match item {
            syn::Item::Use(item_use) => {
                self.check_attrs(&item_use.attrs, Site::Other);
                self.visit_use(item_use);
            }
            syn::Item::Struct(item) => {
                self.visit_struct(item);
            }
            syn::Item::Enum(item) => {
                let serde = self.check_attrs(&item.attrs, Site::Struct);
                self.serde_derive.push(serde);
                self.push_generics(&item.generics);
                for variant in &item.variants {
                    self.check_attrs(&variant.attrs, Site::Variant);
                    for field in variant.fields.iter() {
                        self.check_attrs(&field.attrs, Site::Field);
                        self.visit_type(&field.ty);
                    }
                    if let Some((_, discriminant)) = &variant.discriminant {
                        self.visit_expr(discriminant, None);
                    }
                }
                self.generics.pop();
                self.serde_derive.pop();
            }
            syn::Item::Union(item) => {
                self.refuse(
                    "rust-construct",
                    item.ident.span(),
                    format!("the union `{}`", item.ident),
                );
            }
            syn::Item::Fn(item) => {
                self.check_attrs(&item.attrs, Site::Other);
                let key = self.free_item_key(&item.sig.ident);
                self.with_item_key(key, |walker| {
                    walker.visit_fn(&item.sig, &item.block, None);
                });
            }
            syn::Item::Trait(item) => {
                self.check_attrs(&item.attrs, Site::Other);
                if item.unsafety.is_some() {
                    self.refuse(
                        "rust-unsafe",
                        item.ident.span(),
                        format!("the unsafe trait `{}`", item.ident),
                    );
                }
                if item.auto_token.is_some() {
                    self.refuse(
                        "rust-construct",
                        item.ident.span(),
                        format!("the auto trait `{}`", item.ident),
                    );
                }
                self.push_generics(&item.generics);
                self.visit_bounds(item.supertraits.iter());
                let mut own = self.module.clone();
                own.push(unraw(&item.ident));
                let own = join(&own);
                self.generics
                    .last_mut()
                    .map(|frame| frame.insert("Self".to_string(), vec![Bound::Own(own.clone())]));
                self.self_ty.push(Ty::Bounded(vec![Bound::Own(own)]));
                for member in &item.items {
                    match member {
                        syn::TraitItem::Fn(method) => {
                            self.check_attrs(&method.attrs, Site::Other);
                            match &method.default {
                                Some(block) => {
                                    let key = unraw(&method.sig.ident);
                                    self.with_item_key(key, |walker| {
                                        walker.visit_fn(&method.sig, block, None);
                                    });
                                }
                                None => self.visit_signature_only(&method.sig),
                            }
                        }
                        syn::TraitItem::Const(constant) => {
                            self.check_attrs(&constant.attrs, Site::Other);
                            self.visit_type(&constant.ty);
                            if let Some((_, expr)) = &constant.default {
                                self.visit_expr(expr, None);
                            }
                        }
                        syn::TraitItem::Type(assoc) => {
                            self.check_attrs(&assoc.attrs, Site::Other);
                            self.visit_bounds(assoc.bounds.iter());
                            if let Some((_, ty)) = &assoc.default {
                                self.visit_type(ty);
                            }
                        }
                        syn::TraitItem::Macro(mac) => {
                            self.refuse(
                                "rust-macro",
                                mac.span(),
                                format!(
                                    "the macro `{}!` in a trait",
                                    join(&segments(&mac.mac.path))
                                ),
                            );
                        }
                        _ => self.refuse(
                            "rust-construct",
                            member.span(),
                            "a trait item the scan cannot classify".to_string(),
                        ),
                    }
                }
                self.self_ty.pop();
                self.generics.pop();
            }
            syn::Item::Impl(item) => self.visit_impl(item),
            syn::Item::Const(item) => {
                self.check_attrs(&item.attrs, Site::Other);
                let ty = self.visit_type(&item.ty);
                let key = self.free_item_key(&item.ident);
                self.with_item_key(key, |walker| {
                    walker.visit_expr(&item.expr, Some(&ty));
                });
            }
            syn::Item::Static(item) => {
                self.check_attrs(&item.attrs, Site::Other);
                if matches!(item.mutability, syn::StaticMutability::Mut(_)) {
                    self.refuse(
                        "rust-construct",
                        item.ident.span(),
                        format!("the mutable static `{}`", item.ident),
                    );
                }
                let ty = self.visit_type(&item.ty);
                let key = self.free_item_key(&item.ident);
                self.with_item_key(key, |walker| {
                    walker.visit_expr(&item.expr, Some(&ty));
                });
            }
            syn::Item::Type(item) => {
                self.check_attrs(&item.attrs, Site::Other);
                self.push_generics(&item.generics);
                self.visit_type(&item.ty);
                self.generics.pop();
            }
            syn::Item::Mod(item) => {
                self.check_attrs(&item.attrs, Site::Other);
                if item.unsafety.is_some() {
                    self.refuse(
                        "rust-unsafe",
                        item.ident.span(),
                        format!("the unsafe module `{}`", item.ident),
                    );
                }
                match &item.content {
                    Some((_, content)) => {
                        let mut child = self.module.clone();
                        child.push(unraw(&item.ident));
                        let saved = std::mem::replace(&mut self.module, child);
                        let saved_scopes = std::mem::take(&mut self.scopes);
                        self.visit_items(content);
                        self.scopes = saved_scopes;
                        self.module = saved;
                    }
                    None => self.refuse(
                        "rust-module",
                        item.ident.span(),
                        format!(
                            "`mod {};` reads a file the manifest does not name",
                            item.ident
                        ),
                    ),
                }
            }
            syn::Item::ExternCrate(item) => {
                self.refuse(
                    "rust-extern",
                    item.ident.span(),
                    format!("`extern crate {}`", item.ident),
                );
            }
            syn::Item::ForeignMod(item) => {
                self.refuse(
                    "rust-extern",
                    item.abi.span(),
                    "an `extern` block".to_string(),
                );
            }
            syn::Item::Macro(item) => {
                let name = join(&segments(&item.mac.path));
                let message = if name == "macro_rules" {
                    "`macro_rules!` defines a macro the scan cannot read".to_string()
                } else {
                    format!("the item macro `{name}!`")
                };
                self.refuse("rust-macro", item.mac.path.span(), message);
            }
            syn::Item::TraitAlias(item) => {
                self.refuse(
                    "rust-construct",
                    item.ident.span(),
                    format!("the trait alias `{}`", item.ident),
                );
            }
            _ => self.refuse(
                "rust-construct",
                item.span(),
                "an item the scan cannot classify".to_string(),
            ),
        }
    }

    fn visit_use(&mut self, item_use: &syn::ItemUse) {
        let mut module = Module::default();
        collect_use(
            &item_use.tree,
            &mut Vec::new(),
            item_use.leading_colon.is_some(),
            &mut module,
        );
        let context = self
            .scopes
            .last()
            .map(|scope| scope.path.clone())
            .unwrap_or_else(|| self.module.clone());
        let span = item_use.span();
        let mut leaves: Vec<(UseDecl, bool)> = module
            .uses
            .into_values()
            .map(|decl| (decl, false))
            .collect();
        leaves.extend(module.globs.into_iter().map(|decl| (decl, true)));
        for (decl, glob) in leaves {
            let line_span = span;
            let resolved = self.resolve_use(&context, &decl, 0);
            let full = match resolved {
                Ok(full) => self.expand(full),
                Err(written) => {
                    self.refuse_at(
                        "rust-path",
                        decl.line,
                        line_span,
                        format!("`{}` names a crate Suprnova does not re-export or nothing the component defines", join(&written)),
                    );
                    continue;
                }
            };
            if glob {
                self.check_glob(&full, decl.line, line_span);
                continue;
            }
            let saved = self.findings.len();
            self.classify(&full, line_span);
            for finding in &mut self.findings[saved..] {
                if decl.line.is_some() {
                    finding.line = decl.line;
                }
            }
        }
    }

    fn refuse_at(
        &mut self,
        check: &'static str,
        line: Option<u32>,
        span: proc_macro2::Span,
        message: String,
    ) {
        let before = self.findings.len();
        self.refuse(check, span, message);
        if let Some(line) = line {
            for finding in &mut self.findings[before..] {
                finding.line = Some(line);
            }
        }
    }

    fn check_glob(&mut self, full: &[String], line: Option<u32>, span: proc_macro2::Span) {
        if self.table.is_own(full) || self.is_dependency(full) || self.glob_is_std(full) {
            return;
        }
        let text = join(full);
        let message = if full.first().is_some_and(|first| first == "suprnova") {
            format!(
                "`use {text}::*` imports names the scan cannot list; import each Suprnova item by name"
            )
        } else {
            format!(
                "`use {text}::*` imports from outside the component, its dependencies and the effect-free part of std"
            )
        };
        self.refuse_at("rust-glob", line, span, message);
    }

    fn visit_struct(&mut self, item: &syn::ItemStruct) {
        let serde = self.check_attrs(&item.attrs, Site::Struct);
        self.check_live_struct(item);
        self.serde_derive.push(serde);
        self.push_generics(&item.generics);
        for field in item.fields.iter() {
            self.check_attrs(&field.attrs, Site::Field);
            self.visit_type(&field.ty);
        }
        self.generics.pop();
        self.serde_derive.pop();
    }

    fn visit_impl(&mut self, item: &syn::ItemImpl) {
        let live = item.attrs.iter().any(|attr| self.is_live_attribute(attr));
        self.check_attrs(&item.attrs, if live { Site::Impl } else { Site::Other });
        if item.unsafety.is_some() {
            self.refuse(
                "rust-unsafe",
                item.impl_token.span(),
                "an `unsafe impl`".to_string(),
            );
        }
        if let Some((negative, _, _)) = &item.trait_
            && negative.is_some()
        {
            self.refuse(
                "rust-construct",
                item.impl_token.span(),
                "a negative impl".to_string(),
            );
        }
        self.push_generics(&item.generics);
        let self_ty = self.visit_type(&item.self_ty);
        if let Some((_, path, _)) = &item.trait_ {
            self.check_path_args(path);
            self.check_path(path, Ns::Type);
        }
        self.self_ty.push(self_ty.clone());
        for member in &item.items {
            match member {
                syn::ImplItem::Fn(method) => {
                    self.check_attrs(
                        &method.attrs,
                        if live { Site::LiveMethod } else { Site::Other },
                    );
                    let key = unraw(&method.sig.ident);
                    let method_ty = self_ty.clone();
                    self.with_item_key(key, |walker| {
                        walker.visit_fn(&method.sig, &method.block, Some(method_ty));
                    });
                }
                syn::ImplItem::Const(constant) => {
                    self.check_attrs(&constant.attrs, Site::Other);
                    let ty = self.visit_type(&constant.ty);
                    self.visit_expr(&constant.expr, Some(&ty));
                }
                syn::ImplItem::Type(assoc) => {
                    self.check_attrs(&assoc.attrs, Site::Other);
                    self.visit_type(&assoc.ty);
                }
                syn::ImplItem::Macro(mac) => {
                    self.refuse(
                        "rust-macro",
                        mac.span(),
                        format!("the macro `{}!` in an impl", join(&segments(&mac.mac.path))),
                    );
                }
                _ => self.refuse(
                    "rust-construct",
                    member.span(),
                    "an impl item the scan cannot classify".to_string(),
                ),
            }
        }
        self.self_ty.pop();
        self.generics.pop();
    }

    fn push_generics(&mut self, generics: &syn::Generics) {
        let mut frame: BTreeMap<String, Vec<Bound>> = BTreeMap::new();
        for param in &generics.params {
            if let syn::GenericParam::Type(type_param) = param {
                frame.insert(unraw(&type_param.ident), Vec::new());
            }
        }
        self.generics.push(frame);
        for param in &generics.params {
            match param {
                syn::GenericParam::Type(type_param) => {
                    self.check_attrs(&type_param.attrs, Site::Other);
                    let bounds = self.visit_bounds(type_param.bounds.iter());
                    if let Some(default) = &type_param.default {
                        self.visit_type(default);
                    }
                    if let Some(frame) = self.generics.last_mut() {
                        frame
                            .entry(unraw(&type_param.ident))
                            .or_default()
                            .extend(bounds);
                    }
                }
                syn::GenericParam::Const(const_param) => {
                    self.visit_type(&const_param.ty);
                    if let Some(default) = &const_param.default {
                        self.visit_expr(default, None);
                    }
                }
                syn::GenericParam::Lifetime(_) => {}
            }
        }
        if let Some(where_clause) = &generics.where_clause {
            for predicate in &where_clause.predicates {
                match predicate {
                    syn::WherePredicate::Type(predicate) => {
                        let bounded = self.visit_type(&predicate.bounded_ty);
                        let bounds = self.visit_bounds(predicate.bounds.iter());
                        if let syn::Type::Path(path) = &predicate.bounded_ty
                            && path.qself.is_none()
                            && path.path.segments.len() == 1
                            && let Ty::Bounded(_) = bounded
                        {
                            let name = unraw(&path.path.segments[0].ident);
                            if let Some(frame) = self.generics.last_mut()
                                && let Some(existing) = frame.get_mut(&name)
                            {
                                existing.extend(bounds);
                            }
                        }
                    }
                    syn::WherePredicate::Lifetime(_) => {}
                    _ => self.refuse(
                        "rust-construct",
                        predicate.span(),
                        "a where clause the scan cannot classify".to_string(),
                    ),
                }
            }
        }
    }

    fn visit_signature_only(&mut self, sig: &syn::Signature) {
        self.push_generics(&sig.generics);
        self.check_signature_flags(sig);
        for input in &sig.inputs {
            if let syn::FnArg::Typed(typed) = input {
                self.check_attrs(&typed.attrs, Site::Other);
                self.visit_type(&typed.ty);
            }
        }
        if let syn::ReturnType::Type(_, output) = &sig.output {
            self.visit_type(output);
        }
        self.generics.pop();
    }

    fn check_signature_flags(&mut self, sig: &syn::Signature) {
        if sig.unsafety.is_some() {
            self.refuse(
                "rust-unsafe",
                sig.ident.span(),
                format!("the unsafe function `{}`", sig.ident),
            );
        }
        if sig.abi.is_some() {
            self.refuse(
                "rust-extern",
                sig.ident.span(),
                format!("the `extern` function `{}`", sig.ident),
            );
        }
        if let Some(variadic) = &sig.variadic {
            self.refuse(
                "rust-extern",
                variadic.span(),
                "a variadic function".to_string(),
            );
        }
    }

    fn visit_fn(&mut self, sig: &syn::Signature, block: &syn::Block, self_ty: Option<Ty>) {
        self.push_generics(&sig.generics);
        self.check_signature_flags(sig);
        self.push_scope();
        for input in &sig.inputs {
            match input {
                syn::FnArg::Receiver(receiver) => {
                    self.check_attrs(&receiver.attrs, Site::Other);
                    self.visit_type(&receiver.ty);
                    let ty = self_ty
                        .clone()
                        .or_else(|| self.self_ty.last().cloned())
                        .unwrap_or(Ty::Unknown);
                    self.bind("self".to_string(), ty);
                }
                syn::FnArg::Typed(typed) => {
                    self.check_attrs(&typed.attrs, Site::Other);
                    let ty = self.visit_type(&typed.ty);
                    self.visit_pat(&typed.pat, &ty);
                }
            }
        }
        let expected = match &sig.output {
            syn::ReturnType::Type(_, output) => Some(self.visit_type(output)),
            syn::ReturnType::Default => None,
        };
        self.visit_block_inner(block, expected.as_ref());
        self.pop_scope();
        self.generics.pop();
    }

    // ----- blocks and statements -----------------------------------------

    pub(super) fn visit_block(&mut self, block: &syn::Block, expected: Option<&Ty>) -> Ty {
        self.push_scope();
        let ty = self.visit_block_inner(block, expected);
        self.pop_scope();
        ty
    }

    fn visit_block_inner(&mut self, block: &syn::Block, expected: Option<&Ty>) -> Ty {
        self.declare_block_items(&block.stmts);
        let mut last = Ty::Unit;
        let count = block.stmts.len();
        for (index, stmt) in block.stmts.iter().enumerate() {
            let tail = index + 1 == count;
            last = match stmt {
                syn::Stmt::Local(local) => {
                    self.visit_local(local);
                    Ty::Unit
                }
                syn::Stmt::Item(item) => {
                    let saved_module = self.module.clone();
                    self.visit_item(item);
                    self.module = saved_module;
                    Ty::Unit
                }
                syn::Stmt::Expr(expr, semi) => {
                    let ty = self.visit_expr(
                        expr,
                        if tail && semi.is_none() {
                            expected
                        } else {
                            None
                        },
                    );
                    if semi.is_some() { Ty::Unit } else { ty }
                }
                syn::Stmt::Macro(mac) => {
                    self.check_attrs(&mac.attrs, Site::Other);
                    self.visit_macro(&mac.mac, mac.span());
                    Ty::Unit
                }
            };
        }
        last
    }

    fn visit_local(&mut self, local: &syn::Local) {
        self.check_attrs(&local.attrs, Site::Other);
        let (pat, annotated) = match &local.pat {
            syn::Pat::Type(typed) => {
                self.check_attrs(&typed.attrs, Site::Other);
                let ty = self.visit_type(&typed.ty);
                (&*typed.pat, Some(ty))
            }
            other => (other, None),
        };
        let init_ty = match &local.init {
            Some(init) => {
                let ty = self.visit_expr(&init.expr, annotated.as_ref());
                if let Some((_, diverge)) = &init.diverge {
                    self.visit_expr(diverge, None);
                }
                ty
            }
            None => Ty::Unknown,
        };
        let ty = annotated.unwrap_or(init_ty);
        self.visit_pat(pat, &ty);
    }

    // ----- patterns -------------------------------------------------------

    pub(super) fn visit_pat(&mut self, pat: &syn::Pat, ty: &Ty) {
        if !self.enter(pat.span()) {
            return;
        }
        match pat {
            syn::Pat::Ident(ident) => {
                self.check_attrs(&ident.attrs, Site::Other);
                if let Some((_, sub)) = &ident.subpat {
                    self.visit_pat(sub, ty);
                }
                self.bind(unraw(&ident.ident), ty.clone());
            }
            syn::Pat::Type(typed) => {
                let annotated = self.visit_type(&typed.ty);
                self.visit_pat(&typed.pat, &annotated);
            }
            syn::Pat::Path(path) => {
                if let Some(qself) = &path.qself {
                    self.visit_type(&qself.ty);
                }
                self.check_path_args(&path.path);
                self.check_path(&path.path, Ns::Value);
            }
            syn::Pat::TupleStruct(tuple) => {
                self.check_path_args(&tuple.path);
                let (res, _) = self.check_path(&tuple.path, Ns::Value);
                let element_types = self.variant_fields(&res, ty, tuple.elems.len());
                for (index, elem) in tuple.elems.iter().enumerate() {
                    let field_ty = element_types.get(index).cloned().unwrap_or(Ty::Unknown);
                    self.visit_pat(elem, &field_ty);
                }
            }
            syn::Pat::Struct(structure) => {
                self.check_path_args(&structure.path);
                let (res, class) = self.check_path(&structure.path, Ns::Type);
                let fields = self.struct_fields(&res, &class);
                for field in &structure.fields {
                    let name = match &field.member {
                        syn::Member::Named(ident) => unraw(ident),
                        syn::Member::Unnamed(index) => index.index.to_string(),
                    };
                    let field_ty = fields.get(&name).cloned().unwrap_or(Ty::Unknown);
                    self.visit_pat(&field.pat, &field_ty);
                }
            }
            syn::Pat::Tuple(tuple) => {
                for (index, elem) in tuple.elems.iter().enumerate() {
                    let elem_ty = match ty {
                        Ty::Tuple(parts) => parts.get(index).cloned().unwrap_or(Ty::Unknown),
                        _ => Ty::Unknown,
                    };
                    self.visit_pat(elem, &elem_ty);
                }
            }
            syn::Pat::Slice(slice) => {
                let element = ty.element();
                for elem in &slice.elems {
                    self.visit_pat(elem, &element);
                }
            }
            syn::Pat::Reference(reference) => self.visit_pat(&reference.pat, ty),
            syn::Pat::Paren(paren) => self.visit_pat(&paren.pat, ty),
            syn::Pat::Or(or) => {
                for case in &or.cases {
                    self.visit_pat(case, ty);
                }
            }
            syn::Pat::Lit(lit) => {
                self.visit_expr(&syn::Expr::Lit(lit.clone()), None);
            }
            syn::Pat::Range(range) => {
                if let Some(start) = &range.start {
                    self.visit_expr(start, None);
                }
                if let Some(end) = &range.end {
                    self.visit_expr(end, None);
                }
            }
            syn::Pat::Const(constant) => {
                self.visit_block(&constant.block, None);
            }
            syn::Pat::Wild(_) | syn::Pat::Rest(_) => {}
            syn::Pat::Macro(mac) => {
                self.refuse(
                    "rust-macro",
                    mac.span(),
                    format!(
                        "the macro `{}!` in a pattern",
                        join(&segments(&mac.mac.path))
                    ),
                );
            }
            _ => self.refuse(
                "rust-construct",
                pat.span(),
                "a pattern the scan cannot classify".to_string(),
            ),
        }
        self.leave();
    }

    /// The field types a tuple-struct or variant pattern binds.
    fn variant_fields(&mut self, res: &Res, scrutinee: &Ty, count: usize) -> Vec<Ty> {
        let Res::Full(full) = res else {
            return vec![Ty::Unknown; count];
        };
        let last = full.last().map(String::as_str).unwrap_or_default();
        match (last, scrutinee) {
            ("Some", Ty::Std(name, args)) if name == "Option" => args.clone(),
            ("Ok", Ty::Std(name, args)) if name == "Result" => {
                args.first().cloned().into_iter().collect()
            }
            ("Err", Ty::Std(name, args)) if name == "Result" => {
                args.get(1).cloned().into_iter().collect()
            }
            _ => {
                let class = self.silent_class(full);
                if let Class::Own = class
                    && let Some((name, module)) = full.split_last()
                    && let Some(Def::Struct(fields)) = self
                        .module_at(module)
                        .and_then(|m| m.defs.get(name))
                        .cloned()
                {
                    return (0..count)
                        .map(|index| {
                            fields
                                .get(&index.to_string())
                                .map(|ty| self.type_in_module(ty, module, None))
                                .unwrap_or(Ty::Unknown)
                        })
                        .collect();
                }
                vec![Ty::Unknown; count]
            }
        }
    }

    fn silent_class(&mut self, full: &[String]) -> Class {
        self.silent += 1;
        let class = self.classify(full, proc_macro2::Span::call_site());
        self.silent -= 1;
        class
    }

    /// The field types of an own struct.
    pub(super) fn struct_fields(&mut self, res: &Res, class: &Class) -> BTreeMap<String, Ty> {
        let Res::Full(full) = res else {
            return BTreeMap::new();
        };
        if *class != Class::Own {
            return BTreeMap::new();
        }
        let Some((name, module)) = full.split_last() else {
            return BTreeMap::new();
        };
        let module = module.to_vec();
        let Some(Def::Struct(fields)) = self
            .module_at(&module)
            .and_then(|m| m.defs.get(name))
            .cloned()
        else {
            return BTreeMap::new();
        };
        let self_ty = Ty::Own(join(full));
        fields
            .iter()
            .map(|(field, ty)| {
                (
                    field.clone(),
                    self.type_in_module(ty, &module, Some(self_ty.clone())),
                )
            })
            .collect()
    }

    /// The field types of an own type, by its type.
    pub(super) fn fields_of_type(&mut self, ty: &Ty) -> BTreeMap<String, Ty> {
        match ty {
            Ty::Own(path) => {
                let full: Path = path.split("::").map(str::to_string).collect();
                self.struct_fields(&Res::Full(full), &Class::Own)
            }
            _ => BTreeMap::new(),
        }
    }

    // ----- expressions ----------------------------------------------------

    /// Visits an expression and returns its type as far as the scan knows.
    pub(super) fn visit_expr(&mut self, expr: &syn::Expr, expected: Option<&Ty>) -> Ty {
        if !self.enter(expr.span()) {
            return Ty::Unknown;
        }
        self.check_attrs(expr_attrs(expr), Site::Other);
        let result = self.visit_expr_inner(expr, expected);
        self.leave();
        result
    }

    fn visit_expr_inner(&mut self, expr: &syn::Expr, expected: Option<&Ty>) -> Ty {
        match expr {
            syn::Expr::Lit(lit) => lit_type(&lit.lit),
            syn::Expr::Path(path) => {
                if let Some(qself) = &path.qself {
                    self.visit_type(&qself.ty);
                    self.check_path_args(&path.path);
                    self.check_path(&path.path, Ns::Type);
                    return Ty::Unknown;
                }
                self.check_path_args(&path.path);
                let (res, class) = self.check_path(&path.path, Ns::Value);
                match res {
                    Res::Local(ty) => ty,
                    Res::Full(full) => self.value_type(&full, &class),
                    _ => Ty::Unknown,
                }
            }
            syn::Expr::Call(call) => self.visit_call(call, expected),
            syn::Expr::MethodCall(call) => self.visit_method_call(call, expected),
            syn::Expr::Field(field) => {
                let base = self.visit_expr(&field.base, None);
                match &field.member {
                    syn::Member::Named(ident) => self
                        .fields_of_type(&base)
                        .get(&unraw(ident))
                        .cloned()
                        .unwrap_or(Ty::Unknown),
                    syn::Member::Unnamed(index) => match &base {
                        Ty::Tuple(parts) => parts
                            .get(index.index as usize)
                            .cloned()
                            .unwrap_or(Ty::Unknown),
                        _ => self
                            .fields_of_type(&base)
                            .get(&index.index.to_string())
                            .cloned()
                            .unwrap_or(Ty::Unknown),
                    },
                }
            }
            syn::Expr::Binary(binary) => {
                let left = self.visit_expr(&binary.left, None);
                let right = self.visit_expr(&binary.right, None);
                use syn::BinOp::*;
                match binary.op {
                    Eq(_) | Lt(_) | Le(_) | Ne(_) | Ge(_) | Gt(_) | And(_) | Or(_) => {
                        Ty::prim("bool")
                    }
                    AddAssign(_) | SubAssign(_) | MulAssign(_) | DivAssign(_) | RemAssign(_)
                    | BitXorAssign(_) | BitAndAssign(_) | BitOrAssign(_) | ShlAssign(_)
                    | ShrAssign(_) => Ty::Unit,
                    _ => match (&left, &right) {
                        (Ty::Prim(_) | Ty::Std(..), _) => left,
                        (Ty::Unknown, Ty::Prim(_)) => right,
                        _ => Ty::Unknown,
                    },
                }
            }
            syn::Expr::Unary(unary) => {
                let inner = self.visit_expr(&unary.expr, None);
                match unary.op {
                    syn::UnOp::Deref(_) => inner.deref_target().unwrap_or(inner),
                    _ => inner,
                }
            }
            syn::Expr::Assign(assign) => {
                let target = self.visit_expr(&assign.left, None);
                let hint = (target != Ty::Unknown).then_some(target);
                self.visit_expr(&assign.right, hint.as_ref());
                Ty::Unit
            }
            syn::Expr::Block(block) => self.visit_block(&block.block, expected),
            syn::Expr::Const(constant) => self.visit_block(&constant.block, expected),
            syn::Expr::Unsafe(unsafe_block) => {
                self.refuse(
                    "rust-unsafe",
                    unsafe_block.span(),
                    "an `unsafe` block".to_string(),
                );
                self.visit_block(&unsafe_block.block, expected)
            }
            syn::Expr::If(if_expr) => {
                self.push_scope();
                self.visit_expr(&if_expr.cond, None);
                let then = self.visit_block(&if_expr.then_branch, expected);
                self.pop_scope();
                if let Some((_, else_branch)) = &if_expr.else_branch {
                    self.visit_expr(else_branch, expected);
                }
                then
            }
            syn::Expr::Let(let_expr) => {
                let ty = self.visit_expr(&let_expr.expr, None);
                self.visit_pat(&let_expr.pat, &ty);
                Ty::prim("bool")
            }
            syn::Expr::While(while_expr) => {
                self.push_scope();
                self.visit_expr(&while_expr.cond, None);
                self.visit_block(&while_expr.body, None);
                self.pop_scope();
                Ty::Unit
            }
            syn::Expr::ForLoop(for_loop) => {
                let iterable = self.visit_expr(&for_loop.expr, None);
                let element = match &iterable {
                    Ty::Std(name, args) if name == "Iter" => {
                        args.first().cloned().unwrap_or(Ty::Unknown)
                    }
                    other => other.element(),
                };
                self.push_scope();
                self.visit_pat(&for_loop.pat, &element);
                self.visit_block(&for_loop.body, None);
                self.pop_scope();
                Ty::Unit
            }
            syn::Expr::Loop(loop_expr) => {
                self.visit_block(&loop_expr.body, None);
                Ty::Unknown
            }
            syn::Expr::Match(match_expr) => {
                let scrutinee = self.visit_expr(&match_expr.expr, None);
                let mut first = None;
                for arm in &match_expr.arms {
                    self.check_attrs(&arm.attrs, Site::Other);
                    self.push_scope();
                    self.visit_pat(&arm.pat, &scrutinee);
                    if let Some((_, guard)) = &arm.guard {
                        self.visit_expr(guard, None);
                    }
                    let body = self.visit_expr(&arm.body, expected);
                    self.pop_scope();
                    if first.is_none() {
                        first = Some(body);
                    }
                }
                first.unwrap_or(Ty::Unknown)
            }
            syn::Expr::Closure(closure) => self.visit_closure(closure, &[]),
            syn::Expr::Async(async_block) => {
                self.visit_block(&async_block.block, None);
                Ty::Unknown
            }
            syn::Expr::Await(await_expr) => {
                self.visit_expr(&await_expr.base, None);
                Ty::Unknown
            }
            syn::Expr::Try(try_expr) => {
                let inner = self.visit_expr(&try_expr.expr, None);
                match &inner {
                    Ty::Std(name, args) if name == "Option" || name == "Result" => {
                        args.first().cloned().unwrap_or(Ty::Unknown)
                    }
                    _ => Ty::Unknown,
                }
            }
            syn::Expr::TryBlock(try_block) => {
                self.visit_block(&try_block.block, None);
                Ty::Unknown
            }
            syn::Expr::Cast(cast) => {
                self.visit_expr(&cast.expr, None);
                self.visit_type(&cast.ty)
            }
            syn::Expr::Reference(reference) => self.visit_expr(&reference.expr, expected),
            syn::Expr::RawAddr(raw) => {
                self.refuse(
                    "rust-construct",
                    raw.span(),
                    "a raw address (`&raw`)".to_string(),
                );
                Ty::Unknown
            }
            syn::Expr::Array(array) => {
                let element_hint = expected.map(Ty::element);
                let mut first = Ty::Unknown;
                for (index, elem) in array.elems.iter().enumerate() {
                    let ty = self.visit_expr(elem, element_hint.as_ref());
                    if index == 0 {
                        first = ty;
                    }
                }
                Ty::Slice(Box::new(first))
            }
            syn::Expr::Repeat(repeat) => {
                let element = self.visit_expr(&repeat.expr, None);
                self.visit_expr(&repeat.len, None);
                Ty::Slice(Box::new(element))
            }
            syn::Expr::Tuple(tuple) => {
                if tuple.elems.is_empty() {
                    return Ty::Unit;
                }
                let hints: Vec<Ty> = match expected {
                    Some(Ty::Tuple(parts)) => parts.clone(),
                    _ => Vec::new(),
                };
                Ty::Tuple(
                    tuple
                        .elems
                        .iter()
                        .enumerate()
                        .map(|(index, elem)| self.visit_expr(elem, hints.get(index)))
                        .collect(),
                )
            }
            syn::Expr::Range(range) => {
                let start = range
                    .start
                    .as_ref()
                    .map(|start| self.visit_expr(start, None));
                let end = range.end.as_ref().map(|end| self.visit_expr(end, None));
                Ty::std("Iter", vec![start.or(end).unwrap_or(Ty::Unknown)])
            }
            syn::Expr::Index(index) => {
                let base = self.visit_expr(&index.expr, None);
                let key = self.visit_expr(&index.index, None);
                match &base {
                    Ty::Std(name, args) if matches!(name.as_str(), "HashMap" | "BTreeMap") => {
                        args.get(1).cloned().unwrap_or(Ty::Unknown)
                    }
                    Ty::Std(name, _) if name == "String" => Ty::prim("str"),
                    Ty::Prim(name) if name == "str" => Ty::prim("str"),
                    _ if matches!(key, Ty::Std(ref name, _) if name == "Iter") => base.clone(),
                    _ => base.element(),
                }
            }
            syn::Expr::Struct(structure) => self.visit_struct_expr(structure),
            syn::Expr::Return(ret) => {
                if let Some(value) = &ret.expr {
                    self.visit_expr(value, None);
                }
                Ty::Unknown
            }
            syn::Expr::Break(brk) => {
                if let Some(value) = &brk.expr {
                    self.visit_expr(value, None);
                }
                Ty::Unknown
            }
            syn::Expr::Continue(_) | syn::Expr::Infer(_) => Ty::Unknown,
            syn::Expr::Paren(paren) => self.visit_expr(&paren.expr, expected),
            syn::Expr::Group(group) => self.visit_expr(&group.expr, expected),
            syn::Expr::Macro(mac) => self.visit_macro(&mac.mac, mac.span()),
            syn::Expr::Yield(yield_expr) => {
                self.refuse(
                    "rust-construct",
                    yield_expr.span(),
                    "a `yield` expression".to_string(),
                );
                Ty::Unknown
            }
            _ => {
                self.refuse(
                    "rust-construct",
                    expr.span(),
                    "an expression the scan cannot classify".to_string(),
                );
                Ty::Unknown
            }
        }
    }

    /// The type a value path evaluates to.
    fn value_type(&mut self, full: &[String], class: &Class) -> Ty {
        let last = full.last().map(String::as_str).unwrap_or_default();
        match class {
            Class::Std(_) => match last {
                "None" => Ty::std("Option", vec![Ty::Unknown]),
                _ => Ty::Unknown,
            },
            Class::Own => {
                let Some((name, module)) = full.split_last() else {
                    return Ty::Unknown;
                };
                match self
                    .module_at(module)
                    .and_then(|m| m.defs.get(name))
                    .cloned()
                {
                    Some(Def::Value(ty)) => self.type_in_module(&ty, module, None),
                    Some(Def::Struct(_)) => Ty::Own(join(full)),
                    Some(Def::Fn(sig)) => {
                        Ty::Closure(Box::new(self.return_type(&sig, module, None)))
                    }
                    _ => {
                        // An enum variant: its value is the enum.
                        if let Some((_, enum_module)) = module.split_last()
                            && let Some(Def::Enum) = self
                                .module_at(enum_module)
                                .and_then(|m| m.defs.get(&module[module.len() - 1]))
                        {
                            return Ty::Own(join(module));
                        }
                        Ty::Unknown
                    }
                }
            }
            _ => Ty::Unknown,
        }
    }

    fn return_type(&mut self, sig: &syn::Signature, module: &[String], self_ty: Option<Ty>) -> Ty {
        match &sig.output {
            syn::ReturnType::Type(_, output) => self.type_in_module(output, module, self_ty),
            syn::ReturnType::Default => Ty::Unit,
        }
    }

    fn visit_call(&mut self, call: &syn::ExprCall, expected: Option<&Ty>) -> Ty {
        let syn::Expr::Path(callee) = &*call.func else {
            let callee = self.visit_expr(&call.func, None);
            for arg in &call.args {
                self.visit_expr(arg, None);
            }
            return match callee {
                Ty::Closure(ret) => *ret,
                _ => Ty::Unknown,
            };
        };
        self.check_attrs(&callee.attrs, Site::Other);
        if let Some(qself) = &callee.qself {
            self.visit_type(&qself.ty);
            self.check_path_args(&callee.path);
            self.check_path(&callee.path, Ns::Type);
            for arg in &call.args {
                self.visit_expr(arg, None);
            }
            return Ty::Unknown;
        }
        let turbofish = self.check_path_args(&callee.path);
        let (res, class) = self.check_path(&callee.path, Ns::Value);
        let arg_types: Vec<Ty> = call
            .args
            .iter()
            .map(|arg| self.visit_expr(arg, None))
            .collect();
        match (&res, &class) {
            (Res::Local(Ty::Closure(ret)), _) => (**ret).clone(),
            (Res::Generic(bounds), _) => {
                let name = callee
                    .path
                    .segments
                    .last()
                    .map(|segment| unraw(&segment.ident))
                    .unwrap_or_default();
                self.check_bounded_member(bounds, &name, callee.span());
                Ty::Unknown
            }
            (Res::Full(full), Class::Api { path, .. }) => {
                if CONTAINER_RESOLVERS.contains(&path.as_str()) {
                    return self.container_resolution(path, &callee.path, expected, callee.span());
                }
                let last = full.last().map(String::as_str).unwrap_or_default();
                let owner: Path = full[..full.len().saturating_sub(1)].to_vec();
                if last == "default" && !owner.is_empty() {
                    let owner_text = join(&owner);
                    if self.allowlist.lookup(&owner_text).is_some() {
                        return Ty::Api(owner_text);
                    }
                }
                if let Some(returned) = self.allowlist.returns(path) {
                    let returned = returned.to_string();
                    return self.returned_type(&returned);
                }
                match self.allowlist.kind(path) {
                    Some("struct" | "enum") => Ty::Api(path.clone()),
                    _ => Ty::Unknown,
                }
            }
            (Res::Full(full), Class::Std(_)) => {
                std_constructor(full, &arg_types, &turbofish, expected)
            }
            (Res::Full(full), Class::Own) => {
                let Some((name, module)) = full.split_last() else {
                    return Ty::Unknown;
                };
                let module = module.to_vec();
                match self
                    .module_at(&module)
                    .and_then(|m| m.defs.get(name))
                    .cloned()
                {
                    Some(Def::Fn(sig)) => self.return_type(&sig, &module, None),
                    Some(Def::Struct(_)) => Ty::Own(join(full)),
                    _ => {
                        // `Type::method(...)` or an enum variant constructor.
                        let owner = join(&module);
                        if let Some(impls) = self.impls.get(&owner).cloned()
                            && let Some((sig, sig_module)) = impls.methods.get(name)
                        {
                            return self.return_type(sig, sig_module, Some(Ty::Own(owner)));
                        }
                        if let Some((enum_name, enum_module)) = module.split_last()
                            && let Some(Def::Enum) = self
                                .module_at(enum_module)
                                .and_then(|m| m.defs.get(enum_name))
                        {
                            return Ty::Own(owner);
                        }
                        let _ = expected;
                        Ty::Unknown
                    }
                }
            }
            (Res::Prim(_), _) => Ty::Unknown,
            _ => Ty::Unknown,
        }
    }

    fn container_resolution(
        &mut self,
        resolver: &str,
        path: &syn::Path,
        expected: Option<&Ty>,
        span: proc_macro2::Span,
    ) -> Ty {
        let named = path
            .segments
            .last()
            .and_then(|segment| match &segment.arguments {
                syn::PathArguments::AngleBracketed(angle) => {
                    angle.args.iter().find_map(|arg| match arg {
                        syn::GenericArgument::Type(ty) => Some(ty.clone()),
                        _ => None,
                    })
                }
                _ => None,
            });
        match (named, expected) {
            (Some(ty), _) => {
                // The turbofish was classified with the path's arguments;
                // naming it again only computes its type.
                self.silent += 1;
                let resolved = self.visit_type(&ty);
                self.silent -= 1;
                resolved
            }
            (None, Some(expected)) if *expected != Ty::Unknown => expected.clone(),
            _ => {
                self.refuse(
                    "rust-container",
                    span,
                    format!("`{resolver}` resolves a service whose type the scan cannot name; write `{resolver}::<Type>()`"),
                );
                Ty::Unknown
            }
        }
    }

    fn visit_method_call(&mut self, call: &syn::ExprMethodCall, expected: Option<&Ty>) -> Ty {
        let receiver = self.visit_expr(&call.receiver, None);
        let name = unraw(&call.method);
        let turbofish: Vec<Ty> = call
            .turbofish
            .as_ref()
            .map(|turbofish| {
                turbofish
                    .args
                    .iter()
                    .map(|arg| match arg {
                        syn::GenericArgument::Type(ty) => self.visit_type(ty),
                        syn::GenericArgument::Const(expr) => {
                            self.visit_expr(expr, None);
                            Ty::Unknown
                        }
                        _ => Ty::Unknown,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut arg_types = Vec::with_capacity(call.args.len());
        for (index, arg) in call.args.iter().enumerate() {
            let first = arg_types.first().cloned().unwrap_or(Ty::Unknown);
            let ty = match arg {
                syn::Expr::Closure(closure) => {
                    let params = closure_params(&receiver.clone(), &name, index, &first);
                    self.visit_closure(closure, &params)
                }
                other => self.visit_expr(other, None),
            };
            arg_types.push(ty);
        }
        let span = call.method.span();
        let result = self.classify_method(
            &receiver,
            &name,
            &arg_types,
            turbofish.first(),
            span,
            expected,
        );
        match (result, expected) {
            (Ty::Unknown, Some(expected))
                if matches!(
                    name.as_str(),
                    "into" | "collect" | "parse" | "try_into" | "sum" | "product"
                ) =>
            {
                expected.clone()
            }
            (result, _) => result,
        }
    }

    fn visit_closure(&mut self, closure: &syn::ExprClosure, params: &[Ty]) -> Ty {
        self.push_scope();
        for (index, input) in closure.inputs.iter().enumerate() {
            let ty = params.get(index).cloned().unwrap_or(Ty::Unknown);
            self.visit_pat(input, &ty);
        }
        let annotated = match &closure.output {
            syn::ReturnType::Type(_, output) => Some(self.visit_type(output)),
            syn::ReturnType::Default => None,
        };
        let body = self.visit_expr(&closure.body, annotated.as_ref());
        self.pop_scope();
        Ty::Closure(Box::new(annotated.unwrap_or(body)))
    }

    fn visit_struct_expr(&mut self, structure: &syn::ExprStruct) -> Ty {
        if let Some(qself) = &structure.qself {
            self.visit_type(&qself.ty);
        }
        self.check_path_args(&structure.path);
        let (res, class) = self.check_path(&structure.path, Ns::Type);
        if structure
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "TrustedHtml")
        {
            self.refuse(
                "rust-trusted-html",
                structure.span(),
                "a TrustedHtml struct literal; only the framework builds trusted markup"
                    .to_string(),
            );
        }
        let fields = self.struct_fields(&res, &class);
        for field in &structure.fields {
            self.check_attrs(&field.attrs, Site::Other);
            let name = match &field.member {
                syn::Member::Named(ident) => unraw(ident),
                syn::Member::Unnamed(index) => index.index.to_string(),
            };
            let hint = fields.get(&name).cloned();
            self.visit_expr(&field.expr, hint.as_ref());
        }
        if let Some(rest) = &structure.rest {
            self.visit_expr(rest, None);
        }
        match (res, class) {
            (Res::Full(full), Class::Own) => Ty::Own(join(&full)),
            (_, Class::Api { path, .. }) => Ty::Api(path),
            _ => Ty::Unknown,
        }
    }

    // ----- methods --------------------------------------------------------

    /// Classifies a method call by the method it resolves to on its
    /// receiver's type, and returns what it evaluates to.
    fn classify_method(
        &mut self,
        receiver: &Ty,
        name: &str,
        args: &[Ty],
        turbofish: Option<&Ty>,
        span: proc_macro2::Span,
        expected: Option<&Ty>,
    ) -> Ty {
        let receiver = receiver.clone();
        // A Suprnova trait imported into scope may supply the method on
        // any receiver, `std` types included.
        self.check_scope_traits(name, span);
        match &receiver {
            Ty::Unknown => {
                self.refuse(
                    "rust-method",
                    span,
                    format!("`.{name}()` is called on a value whose type the scan cannot name; annotate the binding's type"),
                );
                Ty::Unknown
            }
            Ty::Own(path) => self.own_method(path, name, args, turbofish, span),
            Ty::Api(path) if path.starts_with("dependency:") => Ty::Unknown,
            Ty::Api(path) => {
                let expected = turbofish.or(expected);
                self.api_method(path, name, span, expected)
            }
            Ty::Bounded(bounds) => {
                self.check_bounded_member(bounds, name, span);
                Ty::Unknown
            }
            other => {
                if let Some(target) = other.deref_target()
                    && !target.is_std()
                    && target != Ty::Unknown
                    && !is_pointer_method(name)
                {
                    return self.classify_method(&target, name, args, turbofish, span, expected);
                }
                std_method_result(other, name, args, turbofish)
            }
        }
    }

    fn own_method(
        &mut self,
        path: &str,
        name: &str,
        args: &[Ty],
        turbofish: Option<&Ty>,
        span: proc_macro2::Span,
    ) -> Ty {
        let _ = (args, turbofish);
        if let Some(impls) = self.impls.get(path).cloned() {
            if let Some((sig, module)) = impls.methods.get(name) {
                return self.return_type(sig, module, Some(Ty::Own(path.to_string())));
            }
            for (trait_path, module) in &impls.traits {
                let saved = std::mem::replace(&mut self.module, module.clone());
                self.silent += 1;
                let (res, class) = self.check_path(trait_path, Ns::Type);
                self.silent -= 1;
                self.module = saved;
                match (res, class) {
                    (Res::Full(full), Class::Own) => {
                        if let Some((trait_name, trait_module)) = full.split_last()
                            && let Some(Def::Trait(methods)) = self
                                .module_at(trait_module)
                                .and_then(|m| m.defs.get(trait_name))
                                .cloned()
                            && let Some(sig) = methods.get(name)
                        {
                            return self.return_type(
                                sig,
                                trait_module,
                                Some(Ty::Own(path.to_string())),
                            );
                        }
                    }
                    (
                        _,
                        Class::Api {
                            path: trait_api,
                            prefix,
                        },
                    ) => {
                        if let Some(found) = self.allowlist.member(&trait_api, name) {
                            self.admit_member(found, &format!("{trait_api}::{name}"), span);
                            return Ty::Unknown;
                        }
                        if prefix {
                            return Ty::Unknown;
                        }
                    }
                    _ => {}
                }
            }
        }
        if STD_TRAIT_METHODS.contains(&name) || name == "default" {
            return std_method_result(&Ty::Own(path.to_string()), name, &[], None);
        }
        self.refuse(
            "rust-method",
            span,
            format!(
                "`.{name}()` is not a method the component defines on `{path}` nor one of `std`'s"
            ),
        );
        Ty::Unknown
    }

    fn api_method(
        &mut self,
        path: &str,
        name: &str,
        span: proc_macro2::Span,
        expected: Option<&Ty>,
    ) -> Ty {
        if let Some(found) = self.allowlist.member(path, name) {
            let member = format!("{path}::{name}");
            let canonical = match found {
                Admission::Item { canonical, .. } => Some(canonical.to_string()),
                _ => None,
            };
            self.admit_member(found, &member, span);
            if CONTAINER_RESOLVERS.contains(&member.as_str()) {
                return match expected {
                    Some(expected) if *expected != Ty::Unknown => expected.clone(),
                    _ => {
                        self.refuse(
                            "rust-container",
                            span,
                            format!(
                                "`.{name}()` resolves a service whose type the scan cannot name"
                            ),
                        );
                        Ty::Unknown
                    }
                };
            }
            if let Some(returned) = canonical
                .as_deref()
                .and_then(|canonical| self.allowlist.returns(canonical))
            {
                let returned = returned.to_string();
                return self.returned_type(&returned);
            }
            return Ty::Unknown;
        }
        if STD_TRAIT_METHODS.contains(&name) {
            if let Some((_, item)) = self.allowlist.lookup(path) {
                let capability = item.capability;
                self.grant(capability);
            }
            return std_method_result(&Ty::Api(path.to_string()), name, &[], None);
        }
        self.refuse(
            "rust-method",
            span,
            format!("`.{name}()` is not on the allowlist for `{path}`"),
        );
        Ty::Unknown
    }

    /// The type a Suprnova function returns, from the type the feature map
    /// recorded for it: `std` types by name, Suprnova types by the path the
    /// allowlist admits, and anything else unknown, so a method called on
    /// it is still refused.
    fn returned_type(&self, text: &str) -> Ty {
        let mut parser = TypeText {
            text: text.as_bytes(),
            index: 0,
        };
        let ty = parser.parse(self.allowlist, 0);
        if parser.index == parser.text.len() {
            ty
        } else {
            Ty::Unknown
        }
    }

    fn admit_member(&mut self, found: Admission<'_>, member: &str, span: proc_macro2::Span) {
        match found {
            Admission::Item { item, canonical } => {
                if item.hidden {
                    self.refuse(
                        "rust-hidden",
                        span,
                        format!("`{canonical}` is hidden, not Suprnova API"),
                    );
                } else {
                    self.grant(item.capability);
                }
            }
            Admission::Prefix { item, root } => {
                if item.hidden {
                    self.refuse(
                        "rust-hidden",
                        span,
                        format!("`{member}` is under `{root}`, a hidden re-export"),
                    );
                } else {
                    self.grant(item.capability);
                }
            }
            Admission::Refused { canonical } => {
                let canonical = canonical.to_string();
                self.refuse(
                    "rust-method",
                    span,
                    format!("`{canonical}` is Suprnova API the registry does not admit"),
                );
            }
            Admission::Module => {}
        }
    }

    fn check_bounded_member(&mut self, bounds: &[Bound], name: &str, span: proc_macro2::Span) {
        let mut api_bound = false;
        for bound in bounds {
            if let Bound::Api(path) = bound {
                api_bound = true;
                if let Some(found) = self.allowlist.member(path, name) {
                    self.admit_member(found, &format!("{path}::{name}"), span);
                    return;
                }
            }
        }
        if bounds.is_empty() {
            if STD_TRAIT_METHODS.contains(&name) {
                return;
            }
            self.refuse(
                "rust-method",
                span,
                format!(
                    "`.{name}()` is called on a generic value with no bound the scan can follow"
                ),
            );
            return;
        }
        if api_bound
            && !STD_TRAIT_METHODS.contains(&name)
            && !bounds
                .iter()
                .any(|bound| matches!(bound, Bound::Own(_) | Bound::Std(_)))
        {
            self.refuse(
                "rust-method",
                span,
                format!("`.{name}()` is not on the allowlist for the value's Suprnova bounds"),
            );
        }
    }

    /// Classifies a method name against every Suprnova trait the current
    /// scope imports by name.
    fn check_scope_traits(&mut self, name: &str, span: proc_macro2::Span) {
        let mut decls: Vec<(Path, UseDecl)> = Vec::new();
        for scope in &self.scopes {
            for decl in scope.module.uses.values() {
                decls.push((scope.path.clone(), decl.clone()));
            }
        }
        if let Some(module) = self.module_at(&self.module) {
            for decl in module.uses.values() {
                decls.push((self.module.clone(), decl.clone()));
            }
        }
        for (context, decl) in decls {
            let Ok(full) = self.resolve_use(&context, &decl, 0) else {
                continue;
            };
            let full = self.expand(full);
            if full.first().is_none_or(|first| first != "suprnova") {
                continue;
            }
            let text = join(&full);
            let Some((canonical, _)) = self.allowlist.lookup(&text) else {
                continue;
            };
            if self.allowlist.kind(canonical) != Some("trait") {
                continue;
            }
            let canonical = canonical.to_string();
            if let Some(found) = self.allowlist.admit(&format!("{canonical}::{name}"))
                && !matches!(found, Admission::Module)
            {
                self.admit_member(found, &format!("{canonical}::{name}"), span);
            }
        }
    }
}

/// The attributes an expression carries.
pub(super) fn expr_attrs(expr: &syn::Expr) -> &[syn::Attribute] {
    match expr {
        syn::Expr::Array(e) => &e.attrs,
        syn::Expr::Assign(e) => &e.attrs,
        syn::Expr::Async(e) => &e.attrs,
        syn::Expr::Await(e) => &e.attrs,
        syn::Expr::Binary(e) => &e.attrs,
        syn::Expr::Block(e) => &e.attrs,
        syn::Expr::Break(e) => &e.attrs,
        syn::Expr::Call(e) => &e.attrs,
        syn::Expr::Cast(e) => &e.attrs,
        syn::Expr::Closure(e) => &e.attrs,
        syn::Expr::Const(e) => &e.attrs,
        syn::Expr::Continue(e) => &e.attrs,
        syn::Expr::Field(e) => &e.attrs,
        syn::Expr::ForLoop(e) => &e.attrs,
        syn::Expr::Group(e) => &e.attrs,
        syn::Expr::If(e) => &e.attrs,
        syn::Expr::Index(e) => &e.attrs,
        syn::Expr::Infer(e) => &e.attrs,
        syn::Expr::Let(e) => &e.attrs,
        syn::Expr::Lit(e) => &e.attrs,
        syn::Expr::Loop(e) => &e.attrs,
        syn::Expr::Macro(e) => &e.attrs,
        syn::Expr::Match(e) => &e.attrs,
        syn::Expr::MethodCall(e) => &e.attrs,
        syn::Expr::Paren(e) => &e.attrs,
        syn::Expr::Path(e) => &e.attrs,
        syn::Expr::Range(e) => &e.attrs,
        syn::Expr::RawAddr(e) => &e.attrs,
        syn::Expr::Reference(e) => &e.attrs,
        syn::Expr::Repeat(e) => &e.attrs,
        syn::Expr::Return(e) => &e.attrs,
        syn::Expr::Struct(e) => &e.attrs,
        syn::Expr::Try(e) => &e.attrs,
        syn::Expr::TryBlock(e) => &e.attrs,
        syn::Expr::Tuple(e) => &e.attrs,
        syn::Expr::Unary(e) => &e.attrs,
        syn::Expr::Unsafe(e) => &e.attrs,
        syn::Expr::While(e) => &e.attrs,
        syn::Expr::Yield(e) => &e.attrs,
        _ => &[],
    }
}

/// Methods a smart pointer answers itself rather than its target.
fn is_pointer_method(name: &str) -> bool {
    matches!(name, "clone" | "as_ref" | "borrow" | "to_owned")
}

/// `std::...` for a path written with `core::` or `alloc::`.
pub(super) fn std_normalized(full: &[String]) -> String {
    let mut parts: Vec<&str> = full.iter().map(String::as_str).collect();
    if let Some(first) = parts.first_mut()
        && (*first == "core" || *first == "alloc")
    {
        *first = "std";
    }
    parts.join("::")
}

/// The `std` path of a `std` type the scan tracks by name.
fn std_path_of(name: &str) -> Option<Path> {
    let path = match name {
        "String" => "std::string::String",
        "Vec" => "std::vec::Vec",
        "Option" => "std::option::Option",
        "Result" => "std::result::Result",
        "Box" => "std::boxed::Box",
        "HashMap" => "std::collections::HashMap",
        "BTreeMap" => "std::collections::BTreeMap",
        "HashSet" => "std::collections::HashSet",
        "BTreeSet" => "std::collections::BTreeSet",
        "VecDeque" => "std::collections::VecDeque",
        _ => return None,
    };
    Some(path.split("::").map(str::to_string).collect())
}

/// The type of a literal.
fn lit_type(lit: &syn::Lit) -> Ty {
    match lit {
        syn::Lit::Str(_) => Ty::prim("str"),
        syn::Lit::ByteStr(_) | syn::Lit::CStr(_) => Ty::Slice(Box::new(Ty::prim("u8"))),
        syn::Lit::Byte(_) => Ty::prim("u8"),
        syn::Lit::Char(_) => Ty::prim("char"),
        syn::Lit::Int(int) => Ty::prim(if int.suffix().is_empty() {
            "i64"
        } else {
            int.suffix()
        }),
        syn::Lit::Float(float) => Ty::prim(if float.suffix().is_empty() {
            "f64"
        } else {
            float.suffix()
        }),
        syn::Lit::Bool(_) => Ty::prim("bool"),
        _ => Ty::Unknown,
    }
}

/// The type a call to a `std` constructor or function returns.
fn std_constructor(full: &[String], args: &[Ty], turbofish: &[Ty], expected: Option<&Ty>) -> Ty {
    let last = full.last().map(String::as_str).unwrap_or_default();
    let owner = full
        .len()
        .checked_sub(2)
        .and_then(|index| full.get(index))
        .map(String::as_str);
    let first_arg = args.first().cloned().unwrap_or(Ty::Unknown);
    match (owner, last) {
        (_, "Some") => Ty::std("Option", vec![first_arg]),
        (_, "Ok") => Ty::std("Result", vec![first_arg, Ty::Unknown]),
        (_, "Err") => Ty::std("Result", vec![Ty::Unknown, first_arg]),
        (_, "take" | "replace") if owner == Some("mem") => first_arg,
        (Some("String"), _) => Ty::string(),
        (Some(name @ ("Box" | "Rc" | "Arc")), "new") => Ty::std(name, vec![first_arg]),
        (
            Some(name @ ("Vec" | "VecDeque" | "HashMap" | "BTreeMap" | "HashSet" | "BTreeSet")),
            _,
        ) => match expected {
            Some(Ty::Std(expected_name, args)) if expected_name == name => {
                Ty::Std(name.to_string(), args.clone())
            }
            _ if !turbofish.is_empty() => Ty::Std(name.to_string(), turbofish.to_vec()),
            _ => Ty::Std(name.to_string(), vec![Ty::Unknown, Ty::Unknown]),
        },
        (Some("Duration"), _) => Ty::std("Duration", Vec::new()),
        _ => Ty::Unknown,
    }
}

/// A parser for the return types the feature map records: full paths with
/// angle-bracketed arguments, `&`, tuples, slices, primitives and `_`.
struct TypeText<'t> {
    text: &'t [u8],
    index: usize,
}

impl TypeText<'_> {
    fn eat(&mut self, byte: u8) -> bool {
        if self.text.get(self.index) == Some(&byte) {
            self.index += 1;
            true
        } else {
            false
        }
    }

    fn list(&mut self, allowlist: &Allowlist, depth: usize, close: u8) -> Vec<Ty> {
        let mut items = Vec::new();
        if self.eat(close) {
            return items;
        }
        loop {
            items.push(self.parse(allowlist, depth + 1));
            if self.eat(b',') {
                continue;
            }
            if !self.eat(close) {
                self.index = self.text.len() + 1;
            }
            return items;
        }
    }

    fn parse(&mut self, allowlist: &Allowlist, depth: usize) -> Ty {
        if depth > 16 {
            self.index = self.text.len() + 1;
            return Ty::Unknown;
        }
        if self.eat(b'&') {
            return self.parse(allowlist, depth + 1);
        }
        if self.eat(b'(') {
            let parts = self.list(allowlist, depth, b')');
            return if parts.is_empty() {
                Ty::Unit
            } else {
                Ty::Tuple(parts)
            };
        }
        if self.eat(b'[') {
            let parts = self.list(allowlist, depth, b']');
            return Ty::Slice(Box::new(parts.into_iter().next().unwrap_or(Ty::Unknown)));
        }
        if self.text.get(self.index) == Some(&b'_')
            && !self
                .text
                .get(self.index + 1)
                .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b':'))
        {
            self.index += 1;
            return Ty::Unknown;
        }
        let start = self.index;
        while self
            .text
            .get(self.index)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b':'))
        {
            self.index += 1;
        }
        let path = String::from_utf8_lossy(&self.text[start..self.index]).into_owned();
        let args = if self.eat(b'<') {
            self.list(allowlist, depth, b'>')
        } else {
            Vec::new()
        };
        if path.is_empty() {
            self.index = self.text.len() + 1;
            return Ty::Unknown;
        }
        if PRIMITIVES.contains(&path.as_str()) {
            return Ty::Prim(path);
        }
        let first = path.split("::").next().unwrap_or_default();
        let last = path.rsplit("::").next().unwrap_or_default().to_string();
        match first {
            "core" | "alloc" | "std" => match last.as_str() {
                "String" | "Vec" | "Option" | "Result" | "HashMap" | "BTreeMap" | "HashSet"
                | "BTreeSet" | "VecDeque" | "Box" | "Rc" | "Arc" | "Cow" | "Duration"
                | "Ordering" => Ty::Std(last, args),
                _ => Ty::Unknown,
            },
            "suprnova" => match allowlist.lookup(&path) {
                Some((canonical, item)) if !item.hidden => Ty::Api(canonical.to_string()),
                _ => Ty::Unknown,
            },
            _ => Ty::Unknown,
        }
    }
}
