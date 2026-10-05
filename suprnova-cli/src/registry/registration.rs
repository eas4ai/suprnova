//! Reading and editing an application's Live registration (REG-005).
//!
//! `src/live/mod.rs` is application code, so it is parsed with `syn` and
//! edited only where the parse found the builder: every other byte,
//! comments and formatting included, stays as it was. The builder must be
//! the scaffold's: one `pub fn registry()` whose body is a
//! `LiveRegistry::builder()` chain of `.register::<T>()?` calls ending in
//! `.build()`, either alone or bound by one `let` and returned as
//! `Ok(<binding>)`. Anything else gets the lines to add reported, because a
//! form the parse does not recognize is a form an edit could break.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::{Path, PathBuf};

use syn::spanned::Spanned as _;

use super::install::RegistrationEdits;
use super::library::is_rust_keyword;
use super::{RegistryError, Result};

/// The application's Live module, from the project root.
pub const LIVE_MODULE: &str = "src/live/mod.rs";

/// Plans the module declarations and registrations for one install.
pub(crate) fn edits(
    project_root: &Path,
    namespace_module: &str,
    modules: &[String],
    register: &[String],
    unregister: &[String],
) -> Result<RegistrationEdits> {
    if !is_identifier(namespace_module) {
        return Err(RegistryError::Invalid(format!(
            "`{namespace_module}` is not a namespace module name"
        )));
    }
    for module in modules {
        if !is_identifier(module) {
            return Err(RegistryError::Invalid(format!(
                "`{module}` is not a module name a component may declare"
            )));
        }
    }
    let register = full_paths(namespace_module, register)?;
    let unregister = full_paths(namespace_module, unregister)?;
    if let Some(both) = register.iter().find(|path| unregister.contains(path)) {
        return Err(RegistryError::Invalid(format!(
            "`{both}` is both registered and unregistered"
        )));
    }
    let plan = Wanted {
        namespace_module,
        modules,
        register: &register,
        unregister: &unregister,
    };
    let live_path = project_root.join(LIVE_MODULE);
    let Some(live_source) = read_source(&live_path)? else {
        return Ok(plan.report(None, None));
    };
    let Ok(live_file) = syn::parse_file(&live_source) else {
        return Ok(plan.report(None, None));
    };
    let namespace_relative = PathBuf::from(format!("src/live/{namespace_module}/mod.rs"));
    let namespace_path = project_root.join(&namespace_relative);
    let namespace_source = read_source(&namespace_path)?;
    let namespace_file = match &namespace_source {
        Some(source) => match syn::parse_file(source) {
            Ok(file) => Some(file),
            Err(_) => return Ok(plan.report(Some(&live_file), None)),
        },
        None => None,
    };
    let declared_namespace = module_declaration(&live_file, namespace_module);
    let flat_module = project_root.join(format!("src/live/{namespace_module}.rs"));
    if matches!(declared_namespace, Declaration::Elsewhere)
        || std::fs::symlink_metadata(&flat_module).is_ok()
    {
        return Ok(plan.report(Some(&live_file), namespace_file.as_ref()));
    }
    if let Some(file) = &namespace_file
        && modules
            .iter()
            .any(|module| matches!(module_declaration(file, module), Declaration::Elsewhere))
    {
        return Ok(plan.report(Some(&live_file), namespace_file.as_ref()));
    }
    let Some(builder) = find_builder(&live_source, &live_file) else {
        return Ok(plan.report(Some(&live_file), namespace_file.as_ref()));
    };

    let mut live_edits: Vec<(Range<usize>, String)> = Vec::new();
    let needs_namespace = !modules.is_empty() || !register.is_empty();
    if needs_namespace && matches!(declared_namespace, Declaration::Missing) {
        live_edits.push(declare_module(
            &live_source,
            &live_file,
            namespace_module,
            builder.function_start,
        ));
    }
    live_edits.extend(builder.edits(&live_source, &register, &unregister));

    let mut files = Vec::new();
    if !live_edits.is_empty() {
        files.push((PathBuf::from(LIVE_MODULE), apply(&live_source, live_edits)?));
    }
    let missing_modules: Vec<&String> = modules
        .iter()
        .filter(|module| {
            namespace_file
                .as_ref()
                .is_none_or(|file| matches!(module_declaration(file, module), Declaration::Missing))
        })
        .collect();
    if !missing_modules.is_empty() {
        let source = match (&namespace_source, &namespace_file) {
            (Some(source), Some(file)) => {
                let mut edits = Vec::new();
                for module in &missing_modules {
                    edits.push(append_module(source, file, module));
                }
                apply(source, edits)?
            }
            _ => namespace_module_source(namespace_module, &missing_modules),
        };
        files.push((namespace_relative, source));
    }
    Ok(RegistrationEdits::Write(files))
}

/// Every type the application's registry builder registers, as a full path
/// from the crate root, or none when `src/live/mod.rs` is missing or not in
/// the scaffold's form. `live:registry check` reads a library's preview
/// application with this, so the preview and an install agree on what
/// "registered" means.
pub fn registered_components(project_root: &Path) -> Result<Option<BTreeSet<String>>> {
    let Some(source) = read_source(&project_root.join(LIVE_MODULE))? else {
        return Ok(None);
    };
    let Ok(file) = syn::parse_file(&source) else {
        return Ok(None);
    };
    Ok(find_builder(&source, &file).map(|builder| {
        builder
            .registrations
            .into_iter()
            .map(|registration| registration.path)
            .collect()
    }))
}

/// What one install asks for, validated.
struct Wanted<'a> {
    namespace_module: &'a str,
    modules: &'a [String],
    register: &'a [String],
    unregister: &'a [String],
}

impl Wanted<'_> {
    /// The lines the developer adds by hand when the application's form is
    /// not one an edit can trust: every declaration the parsed files do not
    /// already hold, every registration and every removal.
    fn report(&self, live: Option<&syn::File>, namespace: Option<&syn::File>) -> RegistrationEdits {
        let mut lines = Vec::new();
        let namespace_module = self.namespace_module;
        let declared = |file: Option<&syn::File>, module: &str| {
            file.is_some_and(|file| matches!(module_declaration(file, module), Declaration::Plain))
        };
        if !declared(live, namespace_module) {
            lines.push(format!("{LIVE_MODULE}: pub mod {namespace_module};"));
        }
        for module in self.modules {
            if !declared(namespace, module) {
                lines.push(format!(
                    "src/live/{namespace_module}/mod.rs: pub mod {module};"
                ));
            }
        }
        for path in self.register {
            lines.push(format!(
                "{LIVE_MODULE}, in registry(): .register::<{path}>()?"
            ));
        }
        for path in self.unregister {
            lines.push(format!(
                "{LIVE_MODULE}, in registry(): remove .register::<{path}>()?"
            ));
        }
        RegistrationEdits::Report(lines)
    }
}

/// Expands manifest entries to full paths, refusing anything that is not
/// `<module>::<Type>` under the namespace module. The names reach source
/// code, so each is checked to be a plain ASCII identifier.
fn full_paths(namespace_module: &str, entries: &[String]) -> Result<Vec<String>> {
    let mut paths: Vec<String> = Vec::new();
    for entry in entries {
        let segments: Vec<&str> = entry.split("::").collect();
        let (module, name) = match segments.as_slice() {
            ["crate", "live", namespace, module, name] if *namespace == namespace_module => {
                (*module, *name)
            }
            [module, name] => (*module, *name),
            _ => {
                return Err(RegistryError::Invalid(format!(
                    "`{entry}` is not `<module>::<Type>` under `crate::live::{namespace_module}`"
                )));
            }
        };
        if !is_identifier(module) || !is_identifier(name) {
            return Err(RegistryError::Invalid(format!(
                "`{entry}` is not `<module>::<Type>` with plain identifiers"
            )));
        }
        let path = format!("crate::live::{module}::{name}");
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    Ok(paths)
}

/// An ASCII identifier that is not a keyword, the only names written into
/// application source.
fn is_identifier(name: &str) -> bool {
    let mut bytes = name.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || (first == b'_' && name.len() > 1))
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        && !is_rust_keyword(name)
        && syn::parse_str::<syn::Ident>(name).is_ok()
}

fn read_source(path: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(source) => Ok(Some(source)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(RegistryError::Io(format!(
            "cannot read {}: {error}",
            path.display()
        ))),
    }
}

/// How a module is declared at the top level of a file.
enum Declaration {
    /// No item of that name.
    Missing,
    /// `mod <name>;`, with or without `pub`: the module's file is where an
    /// install writes it.
    Plain,
    /// Inline, under `#[path]`, or declared twice: its source is somewhere an
    /// install does not write.
    Elsewhere,
}

fn module_declaration(file: &syn::File, name: &str) -> Declaration {
    let found: Vec<&syn::ItemMod> = file
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Mod(module) if module.ident == name => Some(module),
            _ => None,
        })
        .collect();
    match found.as_slice() {
        [] => Declaration::Missing,
        [module]
            if module.content.is_none()
                && !module.attrs.iter().any(|attr| attr.path().is_ident("path")) =>
        {
            Declaration::Plain
        }
        _ => Declaration::Elsewhere,
    }
}

/// The source of a namespace module the install creates.
fn namespace_module_source(namespace_module: &str, modules: &[&String]) -> String {
    let mut source = format!(
        "//! The Live components of the `{namespace_module}` library, declared by\n//! `suprnova live:add`.\n\n"
    );
    for module in modules {
        source.push_str(&format!("pub mod {module};\n"));
    }
    source
}

/// The edit that declares `pub mod <name>;` in `src/live/mod.rs`: after the
/// last module declaration, else after the last `use`, else before the
/// registry function.
fn declare_module(
    source: &str,
    file: &syn::File,
    name: &str,
    function_start: usize,
) -> (Range<usize>, String) {
    let last_module = file
        .items
        .iter()
        .filter(|item| matches!(item, syn::Item::Mod(module) if module.content.is_none()))
        .map(|item| item.span().byte_range().end)
        .max();
    if let Some(end) = last_module {
        return insert_line_after(source, end, &format!("pub mod {name};"));
    }
    let last_use = file
        .items
        .iter()
        .filter(|item| matches!(item, syn::Item::Use(_)))
        .map(|item| item.span().byte_range().end)
        .max();
    if let Some(end) = last_use {
        let at = line_end(source, end);
        let line = if at == source.len() && !source.ends_with('\n') {
            format!("\n\npub mod {name};\n")
        } else {
            format!("\npub mod {name};\n")
        };
        return (at..at, line);
    }
    let at = line_start(source, function_start);
    (at..at, format!("pub mod {name};\n\n"))
}

/// The edit that declares `pub mod <name>;` in an existing namespace module,
/// after its last module declaration or at its end.
fn append_module(source: &str, file: &syn::File, name: &str) -> (Range<usize>, String) {
    let last_module = file
        .items
        .iter()
        .filter(|item| matches!(item, syn::Item::Mod(_)))
        .map(|item| item.span().byte_range().end)
        .max();
    match last_module {
        Some(end) => insert_line_after(source, end, &format!("pub mod {name};")),
        None => {
            let at = source.len();
            let separator = if source.is_empty() || source.ends_with('\n') {
                ""
            } else {
                "\n"
            };
            (at..at, format!("{separator}pub mod {name};\n"))
        }
    }
}

/// An insertion of `line` as a new line after the line holding `position`.
fn insert_line_after(source: &str, position: usize, line: &str) -> (Range<usize>, String) {
    let at = line_end(source, position);
    if at == source.len() && !source.ends_with('\n') {
        (at..at, format!("\n{line}\n"))
    } else {
        (at..at, format!("{line}\n"))
    }
}

/// The offset just past the newline that ends the line holding `position`,
/// or the end of the source.
fn line_end(source: &str, position: usize) -> usize {
    source[position..]
        .find('\n')
        .map_or(source.len(), |offset| position + offset + 1)
}

/// The offset where the line holding `position` starts.
fn line_start(source: &str, position: usize) -> usize {
    source[..position]
        .rfind('\n')
        .map_or(0, |offset| offset + 1)
}

/// The leading whitespace of the line holding `position`.
fn indentation(source: &str, position: usize) -> &str {
    let start = line_start(source, position);
    let line = &source[start..];
    let width = line
        .bytes()
        .take_while(|byte| *byte == b' ' || *byte == b'\t')
        .count();
    &line[..width]
}

/// Applies non-overlapping edits to the original source, last first, so
/// each range still names the bytes the parse saw.
fn apply(source: &str, mut edits: Vec<(Range<usize>, String)>) -> Result<String> {
    edits.sort_by(|a, b| b.0.start.cmp(&a.0.start).then(b.0.end.cmp(&a.0.end)));
    let mut out = source.to_owned();
    let mut floor = usize::MAX;
    for (range, text) in edits {
        if range.end > floor || range.end > source.len() || range.start > range.end {
            return Err(RegistryError::Invalid(
                "the registration edits overlap; nothing was written".to_owned(),
            ));
        }
        out.replace_range(range.clone(), &text);
        floor = range.start;
    }
    Ok(out)
}

/// One `.register::<T>()?` call in the builder chain.
struct Registration {
    /// The registered type as a full path from the crate root.
    path: String,
    /// The call's bytes from the end of its receiver to its `?`, leading
    /// whitespace included: removing them leaves the chain whole.
    range: Range<usize>,
}

/// The scaffold's builder, as the parse found it.
struct Builder {
    /// Every registration, in chain order.
    registrations: Vec<Registration>,
    /// Where `.build()`'s receiver ends: new calls are inserted here.
    receiver_end: usize,
    /// Where the `.` of `.build()` is.
    build_dot: usize,
    /// Where the chain's statement starts, for the indentation of a split.
    chain_start: usize,
    /// Where the registry function starts, its attributes included.
    function_start: usize,
}

impl Builder {
    /// The edits that leave each wanted type registered once and each
    /// dropped type not at all.
    fn edits(
        &self,
        source: &str,
        register: &[String],
        unregister: &[String],
    ) -> Vec<(Range<usize>, String)> {
        let mut edits = Vec::new();
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for registration in &self.registrations {
            let path = registration.path.as_str();
            let dropped = unregister.iter().any(|wanted| wanted == path);
            let repeated = register.iter().any(|wanted| wanted == path) && !seen.insert(path);
            if dropped || repeated {
                edits.push((registration.range.clone(), String::new()));
            }
        }
        let added: Vec<&String> = register
            .iter()
            .filter(|wanted| {
                !self
                    .registrations
                    .iter()
                    .any(|registration| &registration.path == *wanted)
            })
            .collect();
        if added.is_empty() {
            return edits;
        }
        let gap = &source[self.receiver_end..self.build_dot];
        if gap.contains('\n') {
            let indent = indentation(source, self.build_dot);
            let text: String = added
                .iter()
                .map(|path| format!("\n{indent}.register::<{path}>()?"))
                .collect();
            edits.push((self.receiver_end..self.receiver_end, text));
        } else {
            let indent = format!("{}    ", indentation(source, self.chain_start));
            let mut text: String = added
                .iter()
                .map(|path| format!("\n{indent}.register::<{path}>()?"))
                .collect();
            text.push('\n');
            text.push_str(&indent);
            edits.push((self.receiver_end..self.build_dot, text));
        }
        edits
    }
}

/// Finds the one `pub fn registry()` in the scaffold's form.
fn find_builder(source: &str, file: &syn::File) -> Option<Builder> {
    let functions: Vec<&syn::ItemFn> = file
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Fn(function) if function.sig.ident == "registry" => Some(function),
            _ => None,
        })
        .collect();
    let [function] = functions.as_slice() else {
        return None;
    };
    if !matches!(function.vis, syn::Visibility::Public(_)) || !function.sig.inputs.is_empty() {
        return None;
    }
    let chain = match function.block.stmts.as_slice() {
        [syn::Stmt::Expr(chain, None)] => chain,
        [syn::Stmt::Local(local), syn::Stmt::Expr(returned, None)] => {
            let syn::Pat::Ident(binding) = &local.pat else {
                return None;
            };
            if binding.by_ref.is_some() || binding.subpat.is_some() {
                return None;
            }
            let init = local.init.as_ref()?;
            if init.diverge.is_some() || !returns_binding(returned, &binding.ident) {
                return None;
            }
            &*init.expr
        }
        _ => return None,
    };
    let syn::Expr::MethodCall(build) = chain else {
        return None;
    };
    if build.method != "build" || build.turbofish.is_some() || !build.args.is_empty() {
        return None;
    }
    let resolver = Resolver::new(source, file);
    let mut registrations = Vec::new();
    let mut link = &*build.receiver;
    loop {
        match link {
            syn::Expr::Try(attempt) => {
                let syn::Expr::MethodCall(call) = &*attempt.expr else {
                    return None;
                };
                if call.method != "register" || !call.args.is_empty() {
                    return None;
                }
                let turbofish = call.turbofish.as_ref()?;
                let [syn::GenericArgument::Type(ty)] =
                    turbofish.args.iter().collect::<Vec<_>>().as_slice()
                else {
                    return None;
                };
                let path = resolver.type_path(ty);
                let start = call.receiver.span().byte_range().end;
                let end = attempt.span().byte_range().end;
                registrations.push(Registration {
                    path,
                    range: start..end,
                });
                link = &*call.receiver;
            }
            syn::Expr::Call(call) if call.args.is_empty() && is_builder_path(&call.func) => {
                break;
            }
            _ => return None,
        }
    }
    registrations.reverse();
    let function_start = function.span().byte_range().start;
    Some(Builder {
        registrations,
        receiver_end: build.receiver.span().byte_range().end,
        build_dot: build.dot_token.span.byte_range().start,
        chain_start: function
            .block
            .stmts
            .first()
            .map_or(function_start, |statement| {
                statement.span().byte_range().start
            }),
        function_start,
    })
}

/// Whether `expression` is `Ok(<binding>)`.
fn returns_binding(expression: &syn::Expr, binding: &syn::Ident) -> bool {
    let syn::Expr::Call(call) = expression else {
        return false;
    };
    let syn::Expr::Path(function) = &*call.func else {
        return false;
    };
    if !function.path.is_ident("Ok") {
        return false;
    }
    matches!(
        call.args.iter().collect::<Vec<_>>().as_slice(),
        [syn::Expr::Path(argument)] if argument.path.is_ident(binding)
    )
}

/// Whether `function` names `LiveRegistry::builder`, by any path ending in
/// those two segments.
fn is_builder_path(function: &syn::Expr) -> bool {
    let syn::Expr::Path(path) = function else {
        return false;
    };
    let segments: Vec<&syn::PathSegment> = path.path.segments.iter().collect();
    matches!(
        segments.as_slice(),
        [.., owner, method]
            if owner.ident == "LiveRegistry"
                && method.ident == "builder"
                && owner.arguments.is_none()
                && method.arguments.is_none()
    )
}

/// Resolves a type named in `src/live/mod.rs`, which is `crate::live`, to a
/// full path from the crate root, through the file's `use` items, so two
/// spellings of one type are recognized as one registration.
struct Resolver<'a> {
    source: &'a str,
    /// Names the file brings in with `use`, by the name they bind.
    imports: BTreeMap<String, Vec<String>>,
    /// Modules and items the file declares, which a relative path starts at.
    local: BTreeSet<String>,
}

impl<'a> Resolver<'a> {
    fn new(source: &'a str, file: &syn::File) -> Self {
        let mut resolver = Resolver {
            source,
            imports: BTreeMap::new(),
            local: BTreeSet::new(),
        };
        for item in &file.items {
            let name = match item {
                syn::Item::Mod(item) => Some(&item.ident),
                syn::Item::Struct(item) => Some(&item.ident),
                syn::Item::Enum(item) => Some(&item.ident),
                syn::Item::Type(item) => Some(&item.ident),
                syn::Item::Union(item) => Some(&item.ident),
                _ => None,
            };
            if let Some(name) = name {
                resolver.local.insert(name.to_string());
            }
        }
        for item in &file.items {
            if let syn::Item::Use(item) = item {
                let prefix = if item.leading_colon.is_some() {
                    vec![String::new()]
                } else {
                    Vec::new()
                };
                resolver.collect_use(&item.tree, prefix);
            }
        }
        resolver
    }

    fn collect_use(&mut self, tree: &syn::UseTree, prefix: Vec<String>) {
        match tree {
            syn::UseTree::Path(path) => {
                let mut prefix = prefix;
                prefix.push(path.ident.to_string());
                self.collect_use(&path.tree, prefix);
            }
            syn::UseTree::Name(name) => {
                let mut path = prefix;
                path.push(name.ident.to_string());
                self.imports.insert(name.ident.to_string(), path);
            }
            syn::UseTree::Rename(rename) => {
                let mut path = prefix;
                path.push(rename.ident.to_string());
                self.imports.insert(rename.rename.to_string(), path);
            }
            syn::UseTree::Group(group) => {
                for tree in &group.items {
                    self.collect_use(tree, prefix.clone());
                }
            }
            syn::UseTree::Glob(_) => {}
        }
    }

    /// The full path a registered type names, or its source text when it is
    /// not a plain path, which then matches nothing an install manages.
    fn type_path(&self, ty: &syn::Type) -> String {
        let text = || self.source[ty.span().byte_range()].to_owned();
        let syn::Type::Path(path) = ty else {
            return text();
        };
        if path.qself.is_some()
            || path
                .path
                .segments
                .iter()
                .any(|segment| !segment.arguments.is_none())
        {
            return text();
        }
        let mut segments: Vec<String> = path
            .path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect();
        if path.path.leading_colon.is_some() {
            segments.insert(0, String::new());
            return segments.join("::");
        }
        match self.resolve(segments.clone(), 0) {
            Some(resolved) => resolved.join("::"),
            None => {
                segments.insert(0, "?".to_owned());
                segments.join("::")
            }
        }
    }

    /// Resolves a path relative to `crate::live`. `depth` bounds the walk
    /// through `use` items that name each other.
    fn resolve(&self, segments: Vec<String>, depth: usize) -> Option<Vec<String>> {
        if depth > 8 {
            return None;
        }
        let first = segments.first()?.as_str();
        let rest = &segments[1..];
        let joined = |head: &[&str]| -> Vec<String> {
            head.iter()
                .map(|segment| (*segment).to_owned())
                .chain(rest.iter().cloned())
                .collect()
        };
        match first {
            "" => Some(segments.clone()),
            "crate" => Some(segments.clone()),
            "self" => Some(joined(&["crate", "live"])),
            "super" => {
                if rest.first().is_some_and(|segment| segment == "super") {
                    None
                } else {
                    Some(joined(&["crate"]))
                }
            }
            name if self.imports.contains_key(name) => {
                let mut target = self.imports.get(name)?.clone();
                target.extend(rest.iter().cloned());
                self.resolve(target, depth + 1)
            }
            name if self.local.contains(name) || rest.is_empty() => Some(
                ["crate", "live"]
                    .iter()
                    .map(|segment| (*segment).to_owned())
                    .chain(segments.iter().cloned())
                    .collect(),
            ),
            _ => Some(segments.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::registered_components;
    use crate::registry::RegistryError;
    use crate::registry::install::{RegistrationEdits, registration_edits};

    const SCAFFOLD: &str = include_str!("../templates/files/backend/live/mod.rs.tpl");
    const COUNTER: &str = "crate::live::acme::counter::Counter";

    fn project(live_mod: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("src/live")).expect("src/live");
        std::fs::write(dir.path().join("src/live/mod.rs"), live_mod).expect("mod.rs");
        dir
    }

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    fn write(dir: &Path, edits: &RegistrationEdits) {
        let RegistrationEdits::Write(files) = edits else {
            panic!("expected edits to write, got {edits:?}");
        };
        for (path, source) in files {
            assert!(path.is_relative(), "{} is not relative", path.display());
            let target = dir.join(path);
            std::fs::create_dir_all(target.parent().expect("parent")).expect("dir");
            std::fs::write(target, source).expect("write");
        }
    }

    fn written(edits: &RegistrationEdits, path: &str) -> String {
        let RegistrationEdits::Write(files) = edits else {
            panic!("expected edits to write, got {edits:?}");
        };
        files
            .iter()
            .find(|(file, _)| file == &PathBuf::from(path))
            .map(|(_, source)| source.clone())
            .unwrap_or_else(|| panic!("{path} is not written: {files:?}"))
    }

    fn install(dir: &Path) -> RegistrationEdits {
        registration_edits(
            dir,
            "acme",
            &strings(&["counter"]),
            &strings(&["counter::Counter"]),
            &[],
        )
        .expect("edits")
    }

    #[test]
    fn reg_005_a_scaffolded_application_gets_the_module_and_the_full_registration_path() {
        let dir = project(SCAFFOLD);
        let edits = install(dir.path());
        let live = written(&edits, "src/live/mod.rs");
        assert!(
            live.contains(
                "    let registry = LiveRegistry::builder()\n        .register::<crate::live::acme::counter::Counter>()?\n        .build();"
            ),
            "{live}"
        );
        assert_eq!(live.matches("pub mod acme;").count(), 1, "{live}");
        assert!(
            live.find("pub mod acme;") < live.find("/// Builds the registry"),
            "{live}"
        );
        syn::parse_file(&live).expect("the edited module still parses");
        let namespace = written(&edits, "src/live/acme/mod.rs");
        assert!(namespace.contains("\npub mod counter;\n"), "{namespace}");
        syn::parse_file(&namespace).expect("the namespace module parses");
        // Every byte outside the two insertions is kept.
        let without = live.replace("pub mod acme;\n\n", "").replace(
            "\n        .register::<crate::live::acme::counter::Counter>()?",
            "",
        );
        assert_eq!(without, SCAFFOLD);
    }

    #[test]
    fn reg_005_installing_again_adds_no_second_declaration_or_registration() {
        let dir = project(SCAFFOLD);
        write(dir.path(), &install(dir.path()));
        let again = install(dir.path());
        assert_eq!(again, RegistrationEdits::Write(Vec::new()));
        let registered = registered_components(dir.path())
            .expect("reads")
            .expect("the scaffold's form");
        assert_eq!(registered.into_iter().collect::<Vec<_>>(), vec![COUNTER]);
    }

    #[test]
    fn reg_005_a_dropped_type_loses_its_registration_and_a_new_one_gains_one() {
        let dir = project(SCAFFOLD);
        write(dir.path(), &install(dir.path()));
        let edits = registration_edits(
            dir.path(),
            "acme",
            &strings(&["counter"]),
            &strings(&["counter::Tally"]),
            &strings(&["counter::Counter"]),
        )
        .expect("edits");
        let live = written(&edits, "src/live/mod.rs");
        assert!(!live.contains("Counter>"), "{live}");
        assert_eq!(
            live.matches(".register::<crate::live::acme::counter::Tally>()?")
                .count(),
            1,
            "{live}"
        );
        assert_eq!(live.matches("pub mod acme;").count(), 1, "{live}");
        syn::parse_file(&live).expect("parses");
    }

    #[test]
    fn reg_005_a_registration_written_another_way_counts_as_the_same_type() {
        let source = SCAFFOLD.replace(
            "    let registry = LiveRegistry::builder()\n        .build();",
            "    let registry = LiveRegistry::builder()\n        // The acme counter.\n        .register::<acme::counter::Counter>()?\n        .register::<self::acme::counter::Counter>()?\n        .build();",
        );
        let source = source.replace(
            "/// Builds the registry",
            "pub mod acme;\n\n/// Builds the registry",
        );
        let dir = project(&source);
        std::fs::create_dir_all(dir.path().join("src/live/acme")).expect("dir");
        std::fs::write(
            dir.path().join("src/live/acme/mod.rs"),
            "pub mod counter;\n",
        )
        .expect("namespace module");
        let edits = install(dir.path());
        let live = written(&edits, "src/live/mod.rs");
        assert!(live.contains("// The acme counter."), "{live}");
        assert_eq!(live.matches(".register::<").count(), 1, "{live}");
        assert!(
            live.contains(".register::<acme::counter::Counter>()?"),
            "{live}"
        );
    }

    #[test]
    fn reg_005_a_one_line_builder_is_split_before_build() {
        let source = SCAFFOLD.replace(
            "LiveRegistry::builder()\n        .build();",
            "LiveRegistry::builder().build();",
        );
        let dir = project(&source);
        let live = written(&install(dir.path()), "src/live/mod.rs");
        assert!(
            live.contains(
                "    let registry = LiveRegistry::builder()\n        .register::<crate::live::acme::counter::Counter>()?\n        .build();"
            ),
            "{live}"
        );
    }

    #[test]
    fn reg_005_a_lone_builder_chain_is_the_scaffold_s_other_form() {
        let source = "use suprnova::live::{LiveRegistry, RegistryError};\n\n/// Builds the registry.\npub fn registry() -> Result<LiveRegistry, RegistryError> {\n    LiveRegistry::builder()\n        .build()\n}\n";
        let dir = project(source);
        let live = written(&install(dir.path()), "src/live/mod.rs");
        assert_eq!(
            live,
            "use suprnova::live::{LiveRegistry, RegistryError};\n\npub mod acme;\n\n/// Builds the registry.\npub fn registry() -> Result<LiveRegistry, RegistryError> {\n    LiveRegistry::builder()\n        .register::<crate::live::acme::counter::Counter>()?\n        .build()\n}\n"
        );
    }

    #[test]
    fn reg_005_an_unrecognized_builder_gets_lines_reported_and_nothing_written() {
        let chain = "    let registry = LiveRegistry::builder()\n        .build();";
        for (label, source) in [
            (
                "a second registry()",
                format!("{SCAFFOLD}\npub fn registry() -> u8 {{ 0 }}\n"),
            ),
            (
                "a statement before the chain",
                SCAFFOLD.replace(
                    chain,
                    &format!("    let extra = 1;\n    let _ = extra;\n{chain}"),
                ),
            ),
            (
                "another builder method",
                SCAFFOLD.replace(".build();", ".with_limit(2)\n        .build();"),
            ),
            (
                "another constructor",
                SCAFFOLD.replace("LiveRegistry::builder()", "LiveRegistry::new()"),
            ),
            (
                "a private registry()",
                SCAFFOLD.replace("pub fn registry()", "fn registry()"),
            ),
            (
                "a registration without `?`",
                SCAFFOLD.replace(".build();", ".register::<x::X>()\n        .build();"),
            ),
            (
                "an inline namespace module",
                format!("{SCAFFOLD}\nmod acme {{}}\n"),
            ),
            (
                "a namespace module somewhere else",
                format!("{SCAFFOLD}\n#[path = \"elsewhere.rs\"]\npub mod acme;\n"),
            ),
            (
                "source that does not parse",
                "pub fn registry( {".to_owned(),
            ),
        ] {
            let dir = project(&source);
            let edits = install(dir.path());
            let RegistrationEdits::Report(lines) = edits else {
                panic!("{label}: expected a report, got {edits:?}");
            };
            assert!(
                lines.iter().any(
                    |line| line.contains(".register::<crate::live::acme::counter::Counter>()?")
                ),
                "{label}: {lines:?}"
            );
            assert!(
                lines.iter().any(|line| line.contains("pub mod counter;")),
                "{label}: {lines:?}"
            );
            if !label.contains("namespace module") {
                assert_eq!(
                    registered_components(dir.path()).expect("reads"),
                    None,
                    "{label}"
                );
            }
        }
    }

    #[test]
    fn reg_005_a_missing_live_module_or_a_conflicting_namespace_file_is_reported() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(matches!(install(dir.path()), RegistrationEdits::Report(_)));
        assert_eq!(registered_components(dir.path()).expect("reads"), None);

        let dir = project(SCAFFOLD);
        std::fs::write(dir.path().join("src/live/acme.rs"), "").expect("acme.rs");
        assert!(matches!(install(dir.path()), RegistrationEdits::Report(_)));

        let dir = project(SCAFFOLD);
        std::fs::create_dir_all(dir.path().join("src/live/acme")).expect("dir");
        std::fs::write(
            dir.path().join("src/live/acme/mod.rs"),
            "#[path = \"../../../../elsewhere.rs\"]\npub mod counter;\n",
        )
        .expect("namespace module");
        assert!(matches!(install(dir.path()), RegistrationEdits::Report(_)));
    }

    #[test]
    fn reg_005_an_existing_namespace_module_keeps_its_bytes_and_gains_the_module() {
        let dir = project(SCAFFOLD);
        std::fs::create_dir_all(dir.path().join("src/live/acme")).expect("dir");
        let existing = "//! Acme components.\n\npub mod badge;\n\n// Keep this note.\n";
        std::fs::write(dir.path().join("src/live/acme/mod.rs"), existing).expect("module");
        let namespace = written(&install(dir.path()), "src/live/acme/mod.rs");
        assert_eq!(
            namespace,
            "//! Acme components.\n\npub mod badge;\npub mod counter;\n\n// Keep this note.\n"
        );
    }

    #[test]
    fn reg_005_hostile_names_are_refused_before_anything_is_planned() {
        let dir = project(SCAFFOLD);
        for (modules, register) in [
            (
                vec!["counter"],
                vec!["counter::Counter>()?; std::process::exit(1); //"],
            ),
            (
                vec!["counter"],
                vec!["crate::live::other::counter::Counter"],
            ),
            (vec!["counter"], vec!["a::b::c::Counter"]),
            (vec!["mod"], vec!["counter::Counter"]),
            (vec!["../x"], vec!["counter::Counter"]),
            (vec!["counter"], vec!["counter::fn"]),
            (vec!["counter"], vec!["counter::Cöunter"]),
        ] {
            let result = registration_edits(
                dir.path(),
                "acme",
                &strings(&modules),
                &strings(&register),
                &[],
            );
            assert!(
                matches!(result, Err(RegistryError::Invalid(_))),
                "{modules:?} {register:?}: {result:?}"
            );
        }
        assert!(matches!(
            registration_edits(dir.path(), "self", &[], &strings(&["a::B"]), &[]),
            Err(RegistryError::Invalid(_))
        ));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("src/live/mod.rs")).expect("read"),
            SCAFFOLD
        );
    }

    #[test]
    fn reg_005_registrations_resolve_through_use_items() {
        let source = SCAFFOLD
            .replace(
                "use suprnova::live::{",
                "pub mod acme;\n\nuse acme::counter::Counter as Tally;\nuse suprnova::live::{",
            )
            .replace(".build();", ".register::<Tally>()?\n        .build();");
        let dir = project(&source);
        let registered = registered_components(dir.path())
            .expect("reads")
            .expect("form");
        assert_eq!(registered.into_iter().collect::<Vec<_>>(), vec![COUNTER]);
    }
}
