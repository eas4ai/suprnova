//! The component's own module tree, read from its syntax before anything
//! is classified: what each module defines and imports, and which methods
//! and traits each own type has. The walker resolves every path against it.

use std::collections::BTreeMap;

use syn::spanned::Spanned;

/// A full path, one segment per element.
pub(super) type Path = Vec<String>;

/// Joins a path with `::`.
pub(super) fn join(path: &[String]) -> String {
    path.join("::")
}

/// The written form of a `syn` path, raw identifiers unraw'd.
pub(super) fn segments(path: &syn::Path) -> Path {
    path.segments
        .iter()
        .map(|segment| unraw(&segment.ident))
        .collect()
}

/// An identifier without its `r#` prefix, so `r#std` and `std` are one
/// name.
pub(super) fn unraw(ident: &syn::Ident) -> String {
    let text = ident.to_string();
    match text.strip_prefix("r#") {
        Some(stripped) => stripped.to_string(),
        None => text,
    }
}

/// The 1-based line a span starts on, when the parser knows it.
pub(super) fn line_of(span: proc_macro2::Span) -> Option<u32> {
    let line = span.start().line;
    (line > 0).then(|| u32::try_from(line).unwrap_or(u32::MAX))
}

/// One `use` leaf: the written path it imports and where.
#[derive(Debug, Clone)]
pub(super) struct UseDecl {
    /// The imported path's segments, `self` leaves already folded into
    /// their parent.
    pub segments: Path,
    /// Whether the path starts with `::`.
    pub leading_colon: bool,
    /// The line of the leaf.
    pub line: Option<u32>,
}

/// What a module defines under one name.
#[derive(Clone)]
pub(super) enum Def {
    /// A struct, by its named or positional field types.
    Struct(BTreeMap<String, syn::Type>),
    /// An enum.
    Enum,
    /// A function, by its signature.
    Fn(Box<syn::Signature>),
    /// A trait, by its method signatures.
    Trait(BTreeMap<String, syn::Signature>),
    /// A type alias.
    Alias(Box<syn::Type>),
    /// A const or static, by its type.
    Value(Box<syn::Type>),
    /// An inline module.
    Mod,
    /// Something the walker refuses on its own (a union, for example);
    /// recorded so names still resolve.
    Other,
}

/// One module of the component.
#[derive(Clone, Default)]
pub(super) struct Module {
    /// What it defines.
    pub defs: BTreeMap<String, Def>,
    /// What it imports, by the name it binds.
    pub uses: BTreeMap<String, UseDecl>,
    /// Its glob imports.
    pub globs: Vec<UseDecl>,
}

/// The methods and traits of one own type, gathered from every `impl`.
#[derive(Clone, Default)]
pub(super) struct TypeImpls {
    /// Methods by name, with the module whose scope their signature names
    /// types in.
    pub methods: BTreeMap<String, (syn::Signature, Path)>,
    /// Each implemented trait's written path and the module it was written
    /// in.
    pub traits: Vec<(syn::Path, Path)>,
}

/// Every own module, by full path.
#[derive(Clone, Default)]
pub(super) struct ModuleTable {
    /// The modules.
    pub modules: BTreeMap<Path, Module>,
    /// The file modules, `crate::live::<namespace_module>::<stem>`.
    pub roots: Vec<Path>,
    /// Every `impl` block: its self type and trait as written, and the
    /// module it sits in; resolved by the walker once every name is known.
    pub impls: Vec<(syn::Type, Option<syn::Path>, Vec<syn::ImplItem>, Path)>,
}

impl ModuleTable {
    /// Records one file's items under its module path.
    pub fn add_file(&mut self, module: Path, items: &[syn::Item]) {
        self.roots.push(module.clone());
        self.add_module(module, items);
    }

    fn add_module(&mut self, module: Path, items: &[syn::Item]) {
        let mut entry = Module::default();
        for item in items {
            match item {
                syn::Item::Struct(item) => {
                    entry
                        .defs
                        .insert(unraw(&item.ident), Def::Struct(fields_of(&item.fields)));
                }
                syn::Item::Enum(item) => {
                    entry.defs.insert(unraw(&item.ident), Def::Enum);
                }
                syn::Item::Union(item) => {
                    entry.defs.insert(unraw(&item.ident), Def::Other);
                }
                syn::Item::Fn(item) => {
                    entry
                        .defs
                        .insert(unraw(&item.sig.ident), Def::Fn(Box::new(item.sig.clone())));
                }
                syn::Item::Trait(item) => {
                    let methods = item
                        .items
                        .iter()
                        .filter_map(|member| match member {
                            syn::TraitItem::Fn(method) => {
                                Some((unraw(&method.sig.ident), method.sig.clone()))
                            }
                            _ => None,
                        })
                        .collect();
                    entry.defs.insert(unraw(&item.ident), Def::Trait(methods));
                }
                syn::Item::Type(item) => {
                    entry
                        .defs
                        .insert(unraw(&item.ident), Def::Alias(item.ty.clone()));
                }
                syn::Item::Const(item) => {
                    entry
                        .defs
                        .insert(unraw(&item.ident), Def::Value(item.ty.clone()));
                }
                syn::Item::Static(item) => {
                    entry
                        .defs
                        .insert(unraw(&item.ident), Def::Value(item.ty.clone()));
                }
                syn::Item::Mod(item) => {
                    entry.defs.insert(unraw(&item.ident), Def::Mod);
                    if let Some((_, content)) = &item.content {
                        let mut child = module.clone();
                        child.push(unraw(&item.ident));
                        self.add_module(child, content);
                    }
                }
                syn::Item::Use(item) => {
                    collect_use(
                        &item.tree,
                        &mut Vec::new(),
                        item.leading_colon.is_some(),
                        &mut entry,
                    );
                }
                syn::Item::Impl(item) => {
                    self.impls.push((
                        (*item.self_ty).clone(),
                        item.trait_.as_ref().map(|(_, path, _)| path.clone()),
                        item.items.clone(),
                        module.clone(),
                    ));
                }
                syn::Item::Macro(item) => {
                    if let Some(ident) = &item.ident {
                        entry.defs.insert(unraw(ident), Def::Other);
                    }
                }
                _ => {}
            }
        }
        self.modules.insert(module, entry);
    }

    /// The longest own module path that is a proper prefix of `path`.
    pub fn owning_module(&self, path: &[String]) -> Option<&Path> {
        (1..path.len())
            .rev()
            .map(|end| &path[..end])
            .find_map(|prefix| self.modules.get_key_value(prefix).map(|(key, _)| key))
    }

    /// Whether `path` is one of the component's modules or sits under one.
    pub fn is_own(&self, path: &[String]) -> bool {
        self.roots
            .iter()
            .any(|root| path.len() >= root.len() && path[..root.len()] == root[..])
    }
}

/// The named or positional fields of a struct with their types.
pub(super) fn fields_of(fields: &syn::Fields) -> BTreeMap<String, syn::Type> {
    match fields {
        syn::Fields::Named(named) => named
            .named
            .iter()
            .filter_map(|field| {
                field
                    .ident
                    .as_ref()
                    .map(|ident| (unraw(ident), field.ty.clone()))
            })
            .collect(),
        syn::Fields::Unnamed(unnamed) => unnamed
            .unnamed
            .iter()
            .enumerate()
            .map(|(index, field)| (index.to_string(), field.ty.clone()))
            .collect(),
        syn::Fields::Unit => BTreeMap::new(),
    }
}

/// Flattens a `use` tree into its leaves.
pub(super) fn collect_use(
    tree: &syn::UseTree,
    prefix: &mut Path,
    leading_colon: bool,
    module: &mut Module,
) {
    match tree {
        syn::UseTree::Path(path) => {
            prefix.push(unraw(&path.ident));
            collect_use(&path.tree, prefix, leading_colon, module);
            prefix.pop();
        }
        syn::UseTree::Name(name) => {
            let ident = unraw(&name.ident);
            let (segments, bound) = if ident == "self" {
                (prefix.clone(), prefix.last().cloned().unwrap_or_default())
            } else {
                let mut segments = prefix.clone();
                segments.push(ident.clone());
                (segments, ident)
            };
            module.uses.insert(
                bound,
                UseDecl {
                    segments,
                    leading_colon,
                    line: line_of(name.ident.span()),
                },
            );
        }
        syn::UseTree::Rename(rename) => {
            let ident = unraw(&rename.ident);
            let mut segments = prefix.clone();
            if ident != "self" {
                segments.push(ident);
            }
            // `use path as _;` binds no name but still imports a trait's
            // methods; it is kept under a key no identifier can spell.
            let mut bound = unraw(&rename.rename);
            if bound == "_" {
                bound = format!("_#{}", module.uses.len());
            }
            module.uses.insert(
                bound,
                UseDecl {
                    segments,
                    leading_colon,
                    line: line_of(rename.ident.span()),
                },
            );
        }
        syn::UseTree::Glob(glob) => {
            module.globs.push(UseDecl {
                segments: prefix.clone(),
                leading_colon,
                line: line_of(glob.span()),
            });
        }
        syn::UseTree::Group(group) => {
            for item in &group.items {
                collect_use(item, prefix, leading_colon, module);
            }
        }
    }
}
