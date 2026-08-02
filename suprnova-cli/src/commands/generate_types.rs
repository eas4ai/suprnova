use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use syn::visit::Visit;
use syn::{Attribute, Fields, GenericArgument, ItemStruct, PathArguments, Type};
use walkdir::WalkDir;

use crate::ui;

/// Represents a parsed InertiaProps/Data struct
#[derive(Debug, Clone)]
pub struct InertiaPropsStruct {
    pub name: String,
    /// Generic type parameter names (e.g. `["T"]` for `struct Foo<T>`).
    pub type_params: Vec<String>,
    pub fields: Vec<StructField>,
}

/// Flags derived from `#[data(...)]` field attributes.
#[derive(Debug, Clone, Default)]
pub struct DataFieldFlags {
    /// Field is only sent from client → server (excluded from output type)
    pub input_only: bool,
    /// Field is only sent from server → client (excluded from input type)
    pub output_only: bool,
    /// Runtime-only opt-in for sparse fieldsets; no TS effect
    pub allow_include: bool,
    /// Lazily-loaded prop; treated as output-only for TS purposes
    pub lazy: bool,
}

#[derive(Debug, Clone)]
pub struct StructField {
    pub name: String,
    pub ty: RustType,
    pub data_flags: DataFieldFlags,
}

#[derive(Debug, Clone)]
pub enum RustType {
    String,
    Number,
    Bool,
    Option(Box<RustType>),
    Vec(Box<RustType>),
    HashMap(Box<RustType>, Box<RustType>),
    /// `Field<T>` — serialises as `T | null`; optional on the wire
    Field(Box<RustType>),
    /// `Prop<T>` — deferred/lazy prop; optional, never null
    Prop(Box<RustType>),
    Custom(String),
}

/// Visitor that collects structs with #[derive(InertiaProps)] or #[derive(Data)]
/// into `structs`, and every other named-field struct into `plain_structs` so
/// prop fields can resolve nested DTOs that never derived anything.
struct InertiaPropsVisitor {
    structs: Vec<InertiaPropsStruct>,
    plain_structs: Vec<InertiaPropsStruct>,
}

impl InertiaPropsVisitor {
    fn new() -> Self {
        Self {
            structs: Vec::new(),
            plain_structs: Vec::new(),
        }
    }

    fn has_inertia_props_derive(&self, attrs: &[Attribute]) -> bool {
        for attr in attrs {
            if attr.path().is_ident("derive")
                && let Ok(nested) = attr.parse_args_with(
                    syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated,
                )
            {
                for path in nested {
                    if path.is_ident("InertiaProps") {
                        return true;
                    }
                    // Also check for suprnova::InertiaProps
                    if path.segments.len() == 2 {
                        let first = &path.segments[0].ident;
                        let second = &path.segments[1].ident;
                        if first == "suprnova" && second == "InertiaProps" {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    fn has_data_derive(&self, attrs: &[Attribute]) -> bool {
        for attr in attrs {
            if attr.path().is_ident("derive")
                && let Ok(nested) = attr.parse_args_with(
                    syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated,
                )
            {
                for path in nested {
                    if path.is_ident("Data") {
                        return true;
                    }
                    // Also check for suprnova::Data
                    if path.segments.len() == 2 {
                        let first = &path.segments[0].ident;
                        let second = &path.segments[1].ident;
                        if first == "suprnova" && second == "Data" {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    fn parse_type(&self, ty: &Type) -> RustType {
        match ty {
            Type::Path(type_path) => {
                let segment = type_path.path.segments.last().unwrap();
                let ident = segment.ident.to_string();

                match ident.as_str() {
                    "String" | "str" => RustType::String,
                    "i8" | "i16" | "i32" | "i64" | "i128" | "isize" | "u8" | "u16" | "u32"
                    | "u64" | "u128" | "usize" | "f32" | "f64" => RustType::Number,
                    "bool" => RustType::Bool,
                    "Option" => {
                        if let PathArguments::AngleBracketed(args) = &segment.arguments
                            && let Some(GenericArgument::Type(inner_ty)) = args.args.first()
                        {
                            return RustType::Option(Box::new(self.parse_type(inner_ty)));
                        }
                        RustType::Option(Box::new(RustType::Custom("unknown".to_string())))
                    }
                    "Vec" => {
                        if let PathArguments::AngleBracketed(args) = &segment.arguments
                            && let Some(GenericArgument::Type(inner_ty)) = args.args.first()
                        {
                            return RustType::Vec(Box::new(self.parse_type(inner_ty)));
                        }
                        RustType::Vec(Box::new(RustType::Custom("unknown".to_string())))
                    }
                    "HashMap" | "BTreeMap" => {
                        if let PathArguments::AngleBracketed(args) = &segment.arguments {
                            let mut iter = args.args.iter();
                            if let (
                                Some(GenericArgument::Type(key_ty)),
                                Some(GenericArgument::Type(val_ty)),
                            ) = (iter.next(), iter.next())
                            {
                                return RustType::HashMap(
                                    Box::new(self.parse_type(key_ty)),
                                    Box::new(self.parse_type(val_ty)),
                                );
                            }
                        }
                        RustType::HashMap(
                            Box::new(RustType::String),
                            Box::new(RustType::Custom("unknown".to_string())),
                        )
                    }
                    "Field" => {
                        if let PathArguments::AngleBracketed(args) = &segment.arguments
                            && let Some(GenericArgument::Type(inner_ty)) = args.args.first()
                        {
                            return RustType::Field(Box::new(self.parse_type(inner_ty)));
                        }
                        RustType::Field(Box::new(RustType::Custom("unknown".to_string())))
                    }
                    "Prop" => {
                        if let PathArguments::AngleBracketed(args) = &segment.arguments
                            && let Some(GenericArgument::Type(inner_ty)) = args.args.first()
                        {
                            return RustType::Prop(Box::new(self.parse_type(inner_ty)));
                        }
                        RustType::Prop(Box::new(RustType::Custom("unknown".to_string())))
                    }
                    other => RustType::Custom(other.to_string()),
                }
            }
            Type::Reference(type_ref) => {
                // Handle &str as String
                if let Type::Path(inner) = &*type_ref.elem
                    && inner
                        .path
                        .segments
                        .last()
                        .map(|s| s.ident == "str")
                        .unwrap_or(false)
                {
                    return RustType::String;
                }
                self.parse_type(&type_ref.elem)
            }
            _ => RustType::Custom("unknown".to_string()),
        }
    }
}

/// Parse `#[data(...)]` attributes on a field into `DataFieldFlags`.
fn parse_data_flags(attrs: &[Attribute]) -> DataFieldFlags {
    let mut flags = DataFieldFlags::default();
    for attr in attrs {
        if !attr.path().is_ident("data") {
            continue;
        }
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("input_only") {
                flags.input_only = true;
            } else if meta.path.is_ident("output_only") {
                flags.output_only = true;
            } else if meta.path.is_ident("allow_include") {
                flags.allow_include = true;
            } else if meta.path.is_ident("lazy") {
                flags.lazy = true;
            }
            Ok(())
        });
    }
    flags
}

impl<'ast> Visit<'ast> for InertiaPropsVisitor {
    fn visit_item_struct(&mut self, node: &'ast ItemStruct) {
        let derived =
            self.has_inertia_props_derive(&node.attrs) || self.has_data_derive(&node.attrs);

        let fields: Vec<StructField> = match &node.fields {
            Fields::Named(named) => named
                .named
                .iter()
                .filter_map(|f| {
                    f.ident.as_ref().map(|ident| StructField {
                        name: ident.to_string(),
                        ty: self.parse_type(&f.ty),
                        data_flags: parse_data_flags(&f.attrs),
                    })
                })
                .collect(),
            _ => Vec::new(),
        };

        if derived || !fields.is_empty() {
            let parsed = InertiaPropsStruct {
                name: node.ident.to_string(),
                type_params: node
                    .generics
                    .type_params()
                    .map(|tp| tp.ident.to_string())
                    .collect(),
                fields,
            };
            if derived {
                self.structs.push(parsed);
            } else {
                // Tuple/unit structs stay out: promoting one would emit an
                // empty interface that hides a shape the generator can't
                // express, whereas `unknown` + a warning is honest.
                self.plain_structs.push(parsed);
            }
        }

        // Continue visiting nested items
        syn::visit::visit_item_struct(self, node);
    }
}

/// Scan all Rust files in the src directory for InertiaProps/Data structs,
/// with plain-struct definitions resolved transitively (see
/// [`resolve_reachable`]).
pub fn scan_inertia_props(project_path: &Path) -> Vec<InertiaPropsStruct> {
    let src_path = project_path.join("src");
    let mut derived = Vec::new();
    let mut plain = Vec::new();
    visit_path_into(&src_path, &mut derived, &mut plain);
    resolve_reachable(derived, plain)
}

/// Walk a directory tree, collecting derived structs into `derived` and every
/// other named-field struct into `plain`.
fn visit_path_into(
    root: &Path,
    derived: &mut Vec<InertiaPropsStruct>,
    plain: &mut Vec<InertiaPropsStruct>,
) {
    // `sort_by_file_name` is not cosmetic. The output file is checked in,
    // so the walk order becomes the declaration order in a tracked
    // artifact — and an unsorted `WalkDir` yields whatever order the
    // filesystem hands back, which differs between machines and after any
    // directory rewrite. Without it, two developers running the documented
    // command get two different files.
    for entry in WalkDir::new(root)
        .sort_by_file_name()
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|ext| ext == "rs").unwrap_or(false))
    {
        if let Ok(content) = fs::read_to_string(entry.path())
            && let Ok(syntax) = syn::parse_file(&content)
        {
            let mut visitor = InertiaPropsVisitor::new();
            visitor.visit_file(&syntax);
            derived.extend(visitor.structs);
            plain.extend(visitor.plain_structs);
        }
    }
}

/// Promote plain (underived) structs reachable from the derived roots' fields
/// into the emitted set, transitively.
///
/// A prop field naming a DTO that never derived `InertiaProps`/`Data` used to
/// degrade to `unknown`, silently weakening a committed types file the moment
/// anything reran the generator. The struct's definition is right there in
/// `src/`, so emit its real interface instead; `unknown` (with a warning) is
/// reserved for types the project genuinely doesn't define — external crate
/// types, enums, tuple structs. On duplicate names the first definition wins,
/// matching how emission builds its `known` set.
fn resolve_reachable(
    mut derived: Vec<InertiaPropsStruct>,
    plain: Vec<InertiaPropsStruct>,
) -> Vec<InertiaPropsStruct> {
    let mut plain_by_name: HashMap<String, InertiaPropsStruct> = HashMap::new();
    for s in plain {
        plain_by_name.entry(s.name.clone()).or_insert(s);
    }

    let mut emitted: HashSet<String> = derived.iter().map(|s| s.name.clone()).collect();
    let mut queue: Vec<String> = Vec::new();
    for s in &derived {
        for f in &s.fields {
            collect_custom_names(&f.ty, &mut queue);
        }
    }

    while let Some(name) = queue.pop() {
        if emitted.contains(&name) {
            continue;
        }
        if let Some(s) = plain_by_name.remove(&name) {
            emitted.insert(name);
            for f in &s.fields {
                collect_custom_names(&f.ty, &mut queue);
            }
            derived.push(s);
        }
    }

    derived
}

/// Convert a RustType to a TypeScript type string.
///
/// A `Custom(name)` keeps its bare name only when the name resolves to a type
/// the generator actually emits — another InertiaProps/Data struct (`known`) or
/// one of the current struct's generic parameters (`generics`). Anything else (a
/// DTO that forgot to derive InertiaProps/Data, an external type the generator
/// can't see) degrades to `unknown`, so the emitted `.ts` never references an
/// undeclared identifier that would fail `tsc`/`svelte-check`. `is_resolved_custom`
/// is the single source of truth shared with the unresolved-ref diagnostic.
fn rust_type_to_ts(ty: &RustType, known: &HashSet<String>, generics: &[String]) -> String {
    match ty {
        RustType::String => "string".to_string(),
        RustType::Number => "number".to_string(),
        RustType::Bool => "boolean".to_string(),
        RustType::Option(inner) => format!("{} | null", rust_type_to_ts(inner, known, generics)),
        RustType::Vec(inner) => format!("Array<{}>", rust_type_to_ts(inner, known, generics)),
        RustType::HashMap(key, val) => format!(
            "Record<{}, {}>",
            rust_type_to_ts(key, known, generics),
            rust_type_to_ts(val, known, generics)
        ),
        RustType::Field(inner) => format!("{} | null", rust_type_to_ts(inner, known, generics)),
        RustType::Prop(inner) => rust_type_to_ts(inner, known, generics),
        RustType::Custom(name) => {
            if is_resolved_custom(name, known, generics) {
                name.clone()
            } else {
                "unknown".to_string()
            }
        }
    }
}

/// Whether a `Custom` type name resolves to something the generated `.ts`
/// declares: a generated struct, a generic parameter in scope, or the
/// already-degraded `unknown` placeholder the parser emits for unparseable
/// types. Everything else is an undeclared reference.
fn is_resolved_custom(name: &str, known: &HashSet<String>, generics: &[String]) -> bool {
    name == "unknown" || known.contains(name) || generics.iter().any(|g| g == name)
}

/// Return the optional marker for a field's TS declaration.
/// `Field<T>` and `Prop<T>` are optional (may be absent on the wire).
fn optional_marker(ty: &RustType) -> &'static str {
    match ty {
        RustType::Field(_) | RustType::Prop(_) => "?",
        _ => "",
    }
}

/// Order structs by dependency, dependents first.
///
/// The in-degree here counts how many structs *reference* a given one, so
/// the queue seeds with the structs nobody references and walks inward —
/// the reverse of the "dependencies first" this was once documented as
/// producing. That is fine and deliberate to leave alone: a TypeScript
/// interface may reference another declared later in the same file, so
/// declaration order carries no meaning for the consumer. Flipping it now
/// would reorder a checked-in file to no one's benefit.
///
/// What *does* matter is that the order is a pure function of the input —
/// see `tests/generate_types_deterministic.rs`.
fn topological_sort(structs: &[InertiaPropsStruct]) -> Vec<&InertiaPropsStruct> {
    let struct_map: HashMap<_, _> = structs.iter().map(|s| (s.name.clone(), s)).collect();
    let struct_names: HashSet<_> = structs.iter().map(|s| s.name.clone()).collect();

    // Build dependency graph
    let mut deps: HashMap<String, HashSet<String>> = HashMap::new();
    for s in structs {
        let mut s_deps = HashSet::new();
        for field in &s.fields {
            collect_type_deps(&field.ty, &mut s_deps, &struct_names);
        }
        // A struct that references itself (e.g. a tree node with
        // `children: Vec<Self>`) is not an ordering dependency — a TS interface
        // can name itself. Dropping the self-edge keeps Kahn's algorithm from
        // pinning the node's in-degree above zero forever, which silently
        // omitted every self-referencing struct from the output.
        s_deps.remove(&s.name);
        deps.insert(s.name.clone(), s_deps);
    }

    // Kahn's algorithm for topological sort
    let mut in_degree: HashMap<String, usize> =
        struct_names.iter().map(|n| (n.clone(), 0)).collect();
    for s_deps in deps.values() {
        for dep in s_deps {
            if let Some(count) = in_degree.get_mut(dep) {
                *count += 1;
            }
        }
    }

    // Both seeds and successors are drawn from hash containers, whose
    // iteration order Rust randomises per process. Left unsorted, this
    // emitted the same interfaces in a different order on every single
    // run — so the checked-in output showed a diff each time anyone ran
    // the documented command, and a generated file that churns for no
    // reason is a generated file people stop regenerating.
    let mut queue: Vec<_> = in_degree
        .iter()
        .filter(|&(_, &count)| count == 0)
        .map(|(name, _)| name.clone())
        .collect();
    // Descending, because the loop below pops from the back.
    queue.sort_by(|a, b| b.cmp(a));
    let mut result = Vec::new();

    while let Some(name) = queue.pop() {
        if let Some(s) = struct_map.get(&name) {
            result.push(*s);
        }
        if let Some(s_deps) = deps.get(&name) {
            let mut successors: Vec<_> = s_deps.iter().cloned().collect();
            successors.sort();
            for dep in successors {
                if let Some(count) = in_degree.get_mut(&dep) {
                    *count = count.saturating_sub(1);
                    if *count == 0 {
                        queue.push(dep);
                    }
                }
            }
        }
    }

    // Any struct still missing was trapped in a reference cycle (mutual
    // recursion, e.g. A -> B -> A). TS interfaces may reference each other
    // regardless of declaration order, so emit the leftovers in arbitrary
    // order rather than dropping them.
    if result.len() < structs.len() {
        let emitted: HashSet<_> = result.iter().map(|s| s.name.clone()).collect();
        for s in structs {
            if !emitted.contains(&s.name) {
                result.push(s);
            }
        }
    }

    result
}

fn collect_type_deps(ty: &RustType, deps: &mut HashSet<String>, known: &HashSet<String>) {
    match ty {
        RustType::Custom(name) if known.contains(name) => {
            deps.insert(name.clone());
        }
        RustType::Option(inner) | RustType::Vec(inner) => {
            collect_type_deps(inner, deps, known);
        }
        RustType::Field(inner) | RustType::Prop(inner) => {
            collect_type_deps(inner, deps, known);
        }
        RustType::HashMap(key, val) => {
            collect_type_deps(key, deps, known);
            collect_type_deps(val, deps, known);
        }
        _ => {}
    }
}

/// A field whose type references something the generator can't emit — not an
/// InertiaProps/Data struct, not a generic parameter. Reported as a warning; the
/// field itself is emitted as `unknown` (see `rust_type_to_ts`).
struct UnresolvedRef {
    struct_name: String,
    field_name: String,
    type_name: String,
}

/// Walk every generated struct's fields and collect references to custom types
/// that aren't generated (and aren't generic parameters). Uses the same
/// `is_resolved_custom` predicate as emission, so the diagnostic and the emitted
/// `unknown` can never disagree.
fn collect_unresolved_refs(structs: &[InertiaPropsStruct]) -> Vec<UnresolvedRef> {
    let known: HashSet<String> = structs.iter().map(|s| s.name.clone()).collect();
    let mut refs = Vec::new();
    for s in structs {
        for f in &s.fields {
            let mut names = Vec::new();
            collect_custom_names(&f.ty, &mut names);
            for name in names {
                if !is_resolved_custom(&name, &known, &s.type_params) {
                    refs.push(UnresolvedRef {
                        struct_name: s.name.clone(),
                        field_name: f.name.clone(),
                        type_name: name,
                    });
                }
            }
        }
    }
    refs
}

/// Gather every `Custom` type name nested anywhere inside a `RustType`.
fn collect_custom_names(ty: &RustType, out: &mut Vec<String>) {
    match ty {
        RustType::Custom(name) => out.push(name.clone()),
        RustType::Option(inner)
        | RustType::Vec(inner)
        | RustType::Field(inner)
        | RustType::Prop(inner) => collect_custom_names(inner, out),
        RustType::HashMap(key, val) => {
            collect_custom_names(key, out);
            collect_custom_names(val, out);
        }
        _ => {}
    }
}

/// Print one warning per distinct unresolved prop type, so a type the
/// generator can't see surfaces at generation time instead of as a later
/// `tsc`/`svelte-check` failure on the now-`unknown` field. Structs defined
/// anywhere in `src/` resolve even without derives (see `resolve_reachable`),
/// so this fires only for external types, enums, and tuple structs.
fn warn_unresolved_refs(structs: &[InertiaPropsStruct]) {
    let mut seen = HashSet::new();
    for r in collect_unresolved_refs(structs) {
        if seen.insert(r.type_name.clone()) {
            ui::warning(&format!(
                "Prop type `{}` (referenced by `{}.{}`) isn't a struct this project defines — \
                 emitting `unknown`. Mirror it as a local struct (or declare it in an ambient \
                 .d.ts) for a precise type.",
                r.type_name, r.struct_name, r.field_name
            ));
        }
    }
}

/// Emit paired output + (optionally) input TypeScript interfaces for one struct.
///
/// A paired `<Name>Input` interface is emitted whenever any field carries an
/// `input_only`, `output_only`, or `lazy` flag — i.e. whenever the input and
/// output shapes differ.
fn emit_ts_for_struct(s: &InertiaPropsStruct, known: &HashSet<String>) -> String {
    let has_flags = s
        .fields
        .iter()
        .any(|f| f.data_flags.input_only || f.data_flags.output_only || f.data_flags.lazy);

    // Build generic type parameter suffix, e.g. "<T>" or "<A, B>" or "".
    let generics = if s.type_params.is_empty() {
        String::new()
    } else {
        format!("<{}>", s.type_params.join(", "))
    };

    let mut out = String::new();

    // Output interface — what the frontend RECEIVES
    out.push_str(&format!("export interface {}{} {{\n", s.name, generics));
    for f in s.fields.iter().filter(|f| !f.data_flags.input_only) {
        out.push_str(&format!(
            "  {}{}: {};\n",
            f.name,
            optional_marker(&f.ty),
            rust_type_to_ts(&f.ty, known, &s.type_params)
        ));
    }
    out.push_str("}\n\n");

    // Input interface — what the frontend SENDS (only when shapes differ)
    if has_flags {
        out.push_str(&format!(
            "export interface {}Input{} {{\n",
            s.name, generics
        ));
        // Exclude output_only AND lazy fields (lazy props are output-only in nature)
        for f in s
            .fields
            .iter()
            .filter(|f| !f.data_flags.output_only && !f.data_flags.lazy)
        {
            out.push_str(&format!(
                "  {}{}: {};\n",
                f.name,
                optional_marker(&f.ty),
                rust_type_to_ts(&f.ty, known, &s.type_params)
            ));
        }
        out.push_str("}\n\n");
    }

    out
}

/// Generate TypeScript interfaces from the structs.
///
/// This is the canonical emission path; both the file-write entry point and
/// the in-memory `generate_types_string` helper call through here.
pub fn generate_typescript(structs: &[InertiaPropsStruct]) -> String {
    let sorted = topological_sort(structs);
    let known: HashSet<String> = structs.iter().map(|s| s.name.clone()).collect();

    let mut output = String::new();
    output.push_str("// This file is auto-generated by Suprnova. Do not edit manually.\n");
    output.push_str("// Run `suprnova generate-types` to regenerate.\n\n");

    for s in sorted {
        output.push_str(&emit_ts_for_struct(s, &known));
    }

    output
}

/// Input source for `generate_types_string`.
// Used exclusively from integration tests (suprnova-cli/tests/), which are
// separate compilation units invisible to the dead_code lint on the binary target.
#[allow(dead_code)]
pub enum ScanInput {
    /// Parse a single Rust source string (for testing without a filesystem walk).
    Source(&'static str),
    /// Walk a directory tree (production code path).
    Walk(std::path::PathBuf),
}

/// Generate TypeScript type declarations from a given source, returning the
/// result as a `String` without writing to disk.
///
/// Both the test harness and `generate_types_to_file` delegate here so that
/// a single emission path is always exercised.
// Used exclusively from integration tests (suprnova-cli/tests/), which are
// separate compilation units invisible to the dead_code lint on the binary target.
#[allow(dead_code)]
pub fn generate_types_string(input: ScanInput) -> String {
    let structs: Vec<InertiaPropsStruct> = match input {
        ScanInput::Source(src) => {
            let syntax = syn::parse_file(src).expect("ScanInput::Source: invalid Rust");
            let mut visitor = InertiaPropsVisitor::new();
            visitor.visit_file(&syntax);
            resolve_reachable(visitor.structs, visitor.plain_structs)
        }
        ScanInput::Walk(root) => {
            let mut derived = Vec::new();
            let mut plain = Vec::new();
            visit_path_into(&root, &mut derived, &mut plain);
            resolve_reachable(derived, plain)
        }
    };

    // Emit without the file-level header comment so tests get clean output.
    let sorted = topological_sort(&structs);
    let known: HashSet<String> = structs.iter().map(|s| s.name.clone()).collect();
    let mut output = String::new();
    for s in sorted {
        output.push_str(&emit_ts_for_struct(s, &known));
    }
    output
}

/// Generate types and write to the output file
pub fn generate_types_to_file(project_path: &Path, output_path: &Path) -> Result<usize, String> {
    let structs = scan_inertia_props(project_path);

    if structs.is_empty() {
        return Ok(0);
    }

    // Surface prop fields that reference un-generatable types (degraded to
    // `unknown` in the output) so the missing derive is fixed at the source.
    warn_unresolved_refs(&structs);

    // Ensure output directory exists
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create output directory: {}", e))?;
    }

    let typescript = generate_typescript(&structs);
    fs::write(output_path, typescript)
        .map_err(|e| format!("Failed to write TypeScript file: {}", e))?;

    Ok(structs.len())
}

// ---------------------------------------------------------------------
// lang-keys.ts: a `MessageKey` string-union generated from Fluent (.ftl)
// message catalogs, so the frontend gets a compile error the moment it
// references a translation key that doesn't exist in `lang/`.
// ---------------------------------------------------------------------

/// Parse a single Fluent (`.ftl`) source string, returning the sorted,
/// deduped `Entry::Message` ids it declares.
///
/// `Entry::Term` ids (the leading `-` marks a private, non-exposed
/// translation unit — e.g. `-brand-name`) and comments are not messages
/// and are excluded; only ids a frontend could actually pass to a
/// translation call belong in `MessageKey`.
///
/// A source with Fluent syntax errors yields an empty vec rather than a
/// partial result. `fluent_syntax::parser::parse` recovers entry-by-entry
/// and can hand back a resource with most messages intact even when one
/// entry is malformed, but a `MessageKey` union that quietly dropped an
/// id (because it happened to sit next to a typo) is a worse failure mode
/// than one that's visibly missing everything — the file-scanning caller
/// (`extract_message_ids_from_file`) applies this same all-or-nothing
/// policy and is the one that actually reaches disk and warns.
pub fn extract_message_ids(ftl: &str) -> Vec<String> {
    let mut ids = match fluent_syntax::parser::parse(ftl) {
        Ok(resource) => message_ids_from_body(resource.body),
        Err(_) => Vec::new(),
    };
    ids.sort();
    ids.dedup();
    ids
}

/// Pull the `Entry::Message` ids out of a parsed resource's body, in
/// declaration order (callers sort/dedup as needed — single-file callers
/// via `extract_message_ids`, multi-file aggregation via
/// `collect_lang_message_ids`).
fn message_ids_from_body(body: Vec<fluent_syntax::ast::Entry<&str>>) -> Vec<String> {
    body.into_iter()
        .filter_map(|entry| match entry {
            fluent_syntax::ast::Entry::Message(message) => Some(message.id.name.to_string()),
            _ => None,
        })
        .collect()
}

/// Render the `export type MessageKey = ...` union body from a sorted
/// list of message ids.
///
/// No escaping is needed: Fluent identifiers are grammatically restricted
/// to `[a-zA-Z][a-zA-Z0-9_-]*` (see `fluent_syntax`'s
/// `get_identifier`/`get_identifier_unchecked`), so an id can never
/// contain a `"`, a backslash, or a newline — every id that reaches here
/// already passed through the parser's identifier grammar.
///
/// Deliberately takes only `ids`, with no locale/header context, so its
/// unit test doesn't need a filesystem or a resolved locale. The
/// generated-file header (which does name the locale) is layered on by
/// `render_lang_keys_file`.
pub fn render_lang_keys(ids: &[String]) -> String {
    if ids.is_empty() {
        // Not reachable from `generate_lang_keys_to_file` (zero ids means
        // the file isn't written at all — see its doc comment), but this
        // function is public and callable directly, and `never` is the
        // honest TypeScript spelling of "a union of nothing" rather than
        // a panic on the `ids.len() - 1` below.
        return "export type MessageKey = never;\n".to_string();
    }

    let mut out = String::from("export type MessageKey =\n");
    let last = ids.len() - 1;
    for (i, id) in ids.iter().enumerate() {
        let terminator = if i == last { ";" } else { "" };
        out.push_str(&format!("  | \"{}\"{}\n", id, terminator));
    }
    out
}

/// Render the full `lang-keys.ts` file contents: the generated-file
/// header (naming the locale chain actually scanned — just the default
/// locale when it has no configured parents, or the full
/// `child -> parent -> ...` walk when `APP_LOCALE_PARENTS` gives it one)
/// plus the `MessageKey` union body from `render_lang_keys`.
///
/// `chain` is `locale_chain`'s output and is never empty when this is
/// called: `generate_lang_keys_to_file` only reaches here after `ids` is
/// non-empty, and `ids` is built by scanning `chain`'s members, so at
/// least one member (the default locale itself, always `chain[0]`) must
/// exist.
fn render_lang_keys_file(ids: &[String], chain: &[String]) -> String {
    let scanned = match chain {
        [only] => format!("lang/{only}/*.ftl"),
        _ => format!("lang/{}/*.ftl (chain: {})", chain[0], chain.join(" -> ")),
    };
    format!(
        "// Generated by `suprnova generate-types` — do not edit.\n// Message ids from {}.\n{}",
        scanned,
        render_lang_keys(ids)
    )
}

/// Resolve the default locale used to select which `lang/<locale>/*.ftl`
/// catalogs get scanned: the project's `.env` `APP_LOCALE`, or `en`.
///
/// Reads the `.env` file directly with `dotenvy::from_path_iter` rather
/// than loading it into the process environment — this runs from the
/// watcher on every debounced regeneration, and mutating global env vars
/// from a background thread on every file save is the kind of thing that
/// only bites much later. A missing `.env`, an unreadable one, or one
/// without `APP_LOCALE` all resolve to `en`, matching the framework's own
/// `LocalizationConfig::from_env` default.
pub fn resolve_default_locale(project_path: &Path) -> String {
    let env_path = project_path.join(".env");
    dotenvy::from_path_iter(&env_path)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .find(|(key, _)| key == "APP_LOCALE")
        .map(|(_, value)| value)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "en".to_string())
}

/// Resolve `APP_LOCALE_PARENTS` the same way `resolve_default_locale`
/// resolves `APP_LOCALE`: read straight from `.env` with
/// `dotenvy::from_path_iter` rather than the process environment (same
/// rationale — this runs from the watcher on every debounced
/// regeneration).
///
/// Parses comma-separated `child=parent` pairs (`pt-PT=pt-BR,en-AU=en-GB`),
/// trimming whitespace around both the segment and each side of `=`, same
/// shape as the framework's `parse_parents`
/// (`framework/src/localization/config.rs`) — but this crate doesn't
/// depend on the framework at runtime (see the Cargo.toml comment by the
/// `fluent-syntax`/base64 deps), so this is a small local re-parse rather
/// than a shared call, and it does not validate each side as a BCP-47
/// `Locale` the way the framework parser does.
///
/// Deliberately mirrors `resolve_default_locale`'s tolerant posture, not
/// the framework's fail-loud one: a malformed segment (no `=`, or an
/// empty child/parent) is skipped rather than erroring the whole
/// `generate-types` run, exactly how `resolve_default_locale` silently
/// drops an unparseable `.env` line via `dotenvy`'s per-line `Result`
/// instead of failing. This function performs no cycle rejection either —
/// a cycle here is left in the map and instead made safe where it's
/// walked, by `locale_chain`'s visited-set guard.
fn resolve_locale_parents(project_path: &Path) -> HashMap<String, String> {
    let env_path = project_path.join(".env");
    let raw = dotenvy::from_path_iter(&env_path)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .find(|(key, _)| key == "APP_LOCALE_PARENTS")
        .map(|(_, value)| value)
        .unwrap_or_default();
    parse_locale_parents(&raw)
}

/// Parse an `APP_LOCALE_PARENTS`-shaped string into a `child -> parent`
/// map. Pure and file-independent (unlike `resolve_locale_parents`, which
/// wraps it) so its whitespace-tolerance and malformed-segment handling
/// are directly testable without fighting `.env` file-syntax quirks —
/// dotenv's own unquoted-value grammar rejects a value that mixes
/// whitespace with more than one bare `=` (confirmed empirically: only a
/// quoted `.env` value or one with no internal whitespace at all survives
/// `dotenvy::from_path_iter` intact), which has nothing to do with this
/// function's own tolerance for the *parsed* value's whitespace.
///
/// Segments are comma-separated `child=parent` pairs; both the segment and
/// each side of `=` are trimmed, and a blank segment (a stray comma, or an
/// empty string overall) is skipped — same shape as the framework's
/// `parse_parents`. Unlike that framework parser, a malformed segment (no
/// `=`, or an empty child/parent) is skipped rather than erroring the
/// whole value, and a repeated child is last-wins rather than rejected as
/// ambiguous — see `resolve_locale_parents`'s doc for why this mirrors
/// `resolve_default_locale`'s tolerant posture instead.
fn parse_locale_parents(raw: &str) -> HashMap<String, String> {
    let mut parents = HashMap::new();
    for segment in raw.split(',') {
        let segment = segment.trim();
        if segment.is_empty() {
            continue;
        }
        let Some((child, parent)) = segment.split_once('=') else {
            continue;
        };
        let child = child.trim();
        let parent = parent.trim();
        if child.is_empty() || parent.is_empty() {
            continue;
        }
        parents.insert(child.to_string(), parent.to_string());
    }
    parents
}

/// Walk `default_locale`'s fallback chain through `parents`: the locale
/// itself, then its configured parent, then that locale's configured
/// parent, transitively — the same order `Lang::get`/`try_get` walk in the
/// framework (`framework/src/localization/mod.rs`'s `fallback_chain`), and
/// the same walk `FluentTranslator` flattens into each served catalog
/// (`framework/src/localization/fluent.rs`'s `catalog_ast`).
///
/// Deliberately stops at the configured-parents boundary: the global
/// `APP_FALLBACK_LOCALE` is a `Lang`-facade-level, request-time fallback
/// applied *on top of* a resolved catalog, not something the served
/// catalog (or this key union) flattens in — mirroring the framework's own
/// "flattening covers configured parents only" split between
/// `catalog_ast` and `fallback_chain`.
///
/// Guarded against cycles with a visited set (same shape as the
/// framework's `fallback_chain` guard): a locale revisited within the walk
/// stops it there rather than looping forever. `resolve_locale_parents`
/// performs no cycle rejection at parse time, so this is the only guard
/// standing between a cyclic `APP_LOCALE_PARENTS` and a hang.
fn locale_chain(default_locale: &str, parents: &HashMap<String, String>) -> Vec<String> {
    let mut chain = Vec::new();
    let mut visited = HashSet::new();
    let mut current = default_locale.to_string();
    loop {
        if !visited.insert(current.clone()) {
            break;
        }
        chain.push(current.clone());
        match parents.get(&current) {
            Some(parent) => current = parent.clone(),
            None => break,
        }
    }
    chain
}

/// Read and parse one `.ftl` catalog file into its message ids.
///
/// Neither a missing file nor a Fluent syntax error is fatal to
/// `generate-types` as a whole: the offending file is skipped in full and
/// reported with a warning naming it, and every other catalog still
/// contributes its ids. See `extract_message_ids` for why a malformed
/// file contributes nothing rather than a partial result.
fn extract_message_ids_from_file(path: &Path) -> Vec<String> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            ui::warning(&format!(
                "Failed to read {}: {} (skipping this catalog)",
                path.display(),
                e
            ));
            return Vec::new();
        }
    };

    // Parsed once here purely to detect a syntax error worth warning
    // about (which needs the `ParserError`s `extract_message_ids` doesn't
    // expose), and — only on success — a second time inside
    // `extract_message_ids`, which is the single source of truth for
    // "how does a parsed resource become an id list" shared with its
    // direct unit tests. Two parses of a small `.ftl` file is a
    // non-issue for a dev-time code generator.
    if let Err((_, errors)) = fluent_syntax::parser::parse(content.as_str()) {
        ui::warning(&format!(
            "{} has {} Fluent syntax error(s) — skipping this catalog",
            path.display(),
            errors.len()
        ));
        return Vec::new();
    }

    extract_message_ids(&content)
}

/// Collect every distinct `Entry::Message` id declared under
/// `lang/<locale>/*.ftl`, sorted and deduped across files.
///
/// Returns an empty vec when the locale directory doesn't exist, matching
/// `generate_lang_keys_to_file`'s policy of treating "no ids" and "no
/// lang/ dir" identically.
fn collect_lang_message_ids(project_path: &Path, locale: &str) -> Vec<String> {
    let locale_dir = project_path.join("lang").join(locale);
    if !locale_dir.is_dir() {
        return Vec::new();
    }

    let Ok(entries) = fs::read_dir(&locale_dir) else {
        return Vec::new();
    };

    // Sorted for the same reason `visit_path_into` sorts its WalkDir: the
    // output is a checked-in artifact, so an unsorted `read_dir` (whose
    // order differs by filesystem and machine) would make every run a
    // spurious diff even when no `.ftl` file actually changed. Sorting by
    // ids at the end already makes the *union* deterministic; sorting the
    // file list too keeps which-file-warned-about-what deterministic as
    // well.
    let mut ftl_paths: Vec<_> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|ext| ext == "ftl").unwrap_or(false))
        .collect();
    ftl_paths.sort();

    let mut ids: Vec<String> = ftl_paths
        .iter()
        .flat_map(|path| extract_message_ids_from_file(path))
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

/// Generate `lang-keys.ts`: parse `lang/<locale>/*.ftl` for
/// `Entry::Message` ids across the default locale's whole fallback
/// chain — the default locale itself plus every `APP_LOCALE_PARENTS`
/// ancestor (`locale_chain`) — and emit the union as a `MessageKey`
/// string-union.
///
/// Scanning only the default locale's own directory would be wrong the
/// moment `APP_LOCALE_PARENTS` is in play: a delta-style catalog (e.g.
/// `lang/pt-PT/` declaring only the handful of strings that differ from
/// `pt-BR`) is the normal, intended shape once a locale has a configured
/// parent, and the *served* catalog (`FluentTranslator`'s chain-flattened
/// `/_suprnova/lang/<locale>.ftl`) already resolves every inherited key
/// fine — so the generated `MessageKey` type must union across the same
/// chain, not just scan the child's directory, or it rejects keys the
/// running app actually serves. The global `APP_FALLBACK_LOCALE` is
/// deliberately excluded from the union (see `locale_chain`'s doc) — it's
/// a request-time `Lang`-facade fallback, not part of what any one
/// locale's served catalog flattens.
///
/// A project with no `lang/` directory for *any* chain member, or whose
/// chain-wide catalogs declare zero messages between them, isn't
/// localized in any way this can see — writing an empty union would be
/// worse than useless (an uninhabited type nothing can ever satisfy), so
/// the file is not written at all. This also covers a child locale with no
/// directory of its own at all (nothing to override yet) as long as some
/// ancestor in the chain has keys: the union is non-empty, so the file is
/// still written from the ancestor's ids alone, not deleted. Any stale
/// `lang-keys.ts` from a prior run (every chain member's `lang/` dir was
/// since removed, or every message deleted) is cleaned up so the frontend
/// doesn't keep type-checking against ids that no longer exist.
///
/// Returns the number of ids written (0 when nothing was written).
pub fn generate_lang_keys_to_file(
    project_path: &Path,
    output_path: &Path,
) -> Result<usize, String> {
    let locale = resolve_default_locale(project_path);
    let parents = resolve_locale_parents(project_path);
    let chain = locale_chain(&locale, &parents);

    let mut ids: Vec<String> = chain
        .iter()
        .flat_map(|member| collect_lang_message_ids(project_path, member))
        .collect();
    ids.sort();
    ids.dedup();

    if ids.is_empty() {
        if output_path.exists() {
            fs::remove_file(output_path)
                .map_err(|e| format!("Failed to remove stale {}: {}", output_path.display(), e))?;
        }
        return Ok(0);
    }

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create output directory: {}", e))?;
    }

    let contents = render_lang_keys_file(&ids, &chain);
    fs::write(output_path, contents)
        .map_err(|e| format!("Failed to write {}: {}", output_path.display(), e))?;

    Ok(ids.len())
}

/// Main entry point for the generate-types command
pub fn run(output: Option<String>, watch: bool, routes: bool) {
    let project_path = Path::new(".");

    // Validate Suprnova project
    let cargo_toml = project_path.join("Cargo.toml");
    if !cargo_toml.exists() {
        ui::error("Not a Suprnova project (no Cargo.toml found)");
        std::process::exit(1);
    }

    let output_path = output
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| project_path.join("frontend/src/types/inertia-props.ts"));

    ui::info("Scanning for InertiaProps structs...");

    match generate_types_to_file(project_path, &output_path) {
        Ok(0) => {
            ui::warning("No InertiaProps structs found.");
        }
        Ok(count) => {
            ui::info(&format!("Found {} InertiaProps struct(s)", count));
            ui::success(&format!("Generated {}", output_path.display()));
        }
        Err(e) => {
            ui::error(&e);
            std::process::exit(1);
        }
    }

    // lang-keys.ts is opt-in the same way: silent when the project has no
    // `lang/` (Ok(0)) rather than the "found 0" ceremony InertiaProps gets
    // above, because the overwhelming majority of projects aren't
    // localized at all and would otherwise see this warning on every
    // single run forever.
    let lang_keys_output = project_path.join("frontend/src/types/lang-keys.ts");
    match generate_lang_keys_to_file(project_path, &lang_keys_output) {
        Ok(0) => {}
        Ok(count) => {
            ui::info(&format!("Found {} message id(s) in lang/", count));
            ui::success(&format!("Generated {}", lang_keys_output.display()));
        }
        Err(e) => {
            ui::error(&e);
            std::process::exit(1);
        }
    }

    // Route types are opt-in: dropping an unconsumed routes.ts into every
    // project that runs generate-types is churn, not a feature.
    if routes {
        generate_route_types(project_path);
    }

    if watch {
        ui::hint("Watching for changes...");
        if let Err(e) = start_watcher(project_path, &output_path, &lang_keys_output) {
            ui::error(&format!("Failed to start watcher: {}", e));
            std::process::exit(1);
        }
    }
}

/// Generate route types
fn generate_route_types(project_path: &Path) {
    let routes_output = project_path.join("frontend/src/types/routes.ts");

    ui::info("Scanning routes for type-safe generation...");

    match super::generate_routes::generate_routes_to_file(project_path, &routes_output) {
        Ok(0) => {
            ui::warning("No routes found in src/routes.rs");
        }
        Ok(count) => {
            ui::info(&format!("Found {} route(s)", count));
            ui::success(&format!("Generated {}", routes_output.display()));
        }
        Err(e) => {
            ui::warning(&format!("Route generation error: {}", e));
        }
    }
}

/// Start file watcher for automatic type regeneration
///
/// Watches `src/` for `.rs` changes (regenerating `inertia-props.ts`, at
/// `output_path`) and, when the project has a `lang/` directory, watches
/// it too for `.ftl` changes (regenerating `lang-keys.ts`, at
/// `lang_keys_output`). A project without `lang/` at watcher-start simply
/// doesn't get that second watch — `notify` can't watch a path that
/// doesn't exist yet, and a project growing a `lang/` dir mid-`serve` is
/// outside what this needs to handle; rerunning `generate-types --watch`
/// picks it up.
fn start_watcher(
    project_path: &Path,
    output_path: &Path,
    lang_keys_output: &Path,
) -> Result<(), String> {
    use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
    use std::sync::mpsc::channel;
    use std::time::Duration;

    let (tx, rx) = channel();
    let src_path = project_path.join("src");
    let lang_path = project_path.join("lang");

    let mut watcher = RecommendedWatcher::new(
        move |res| {
            if let Ok(event) = res {
                let _ = tx.send(event);
            }
        },
        Config::default().with_poll_interval(Duration::from_secs(1)),
    )
    .map_err(|e| format!("Failed to create watcher: {}", e))?;

    watcher
        .watch(&src_path, RecursiveMode::Recursive)
        .map_err(|e| format!("Failed to watch directory: {}", e))?;

    ui::hint(&format!("Watching {} for changes", src_path.display()));

    if lang_path.is_dir() {
        watcher
            .watch(&lang_path, RecursiveMode::Recursive)
            .map_err(|e| format!("Failed to watch lang directory: {}", e))?;
        ui::hint(&format!("Watching {} for changes", lang_path.display()));
    }

    let output_path = output_path.to_path_buf();
    let lang_keys_output = lang_keys_output.to_path_buf();
    let project_path = project_path.to_path_buf();

    loop {
        match rx.recv() {
            Ok(event) => {
                let is_rust_change = event
                    .paths
                    .iter()
                    .any(|p| p.extension().map(|e| e == "rs").unwrap_or(false));
                let is_ftl_change = event
                    .paths
                    .iter()
                    .any(|p| p.extension().map(|e| e == "ftl").unwrap_or(false));

                if is_rust_change {
                    ui::hint("Detected changes, regenerating types...");
                    match generate_types_to_file(&project_path, &output_path) {
                        Ok(count) => {
                            ui::success(&format!("Regenerated {} type(s)", count));
                        }
                        Err(e) => {
                            ui::error(&format!("Failed to regenerate: {}", e));
                        }
                    }
                }

                if is_ftl_change {
                    ui::hint("Detected lang/ changes, regenerating lang-keys...");
                    match generate_lang_keys_to_file(&project_path, &lang_keys_output) {
                        Ok(0) => {
                            ui::hint("lang-keys.ts removed (no message ids)");
                        }
                        Ok(count) => {
                            ui::success(&format!("Regenerated {} message id(s)", count));
                        }
                        Err(e) => {
                            ui::error(&format!("Failed to regenerate lang-keys: {}", e));
                        }
                    }
                }
            }
            Err(e) => {
                return Err(format!("Watch error: {}", e));
            }
        }
    }
}

#[cfg(test)]
mod lang_keys_tests {
    use super::*;

    #[test]
    fn extracts_sorted_message_ids_from_ftl() {
        let ftl = "zeta = Z\nalpha = A\n# comment\n-term = private\n";
        let ids = extract_message_ids(ftl);
        assert_eq!(ids, vec!["alpha", "zeta"]); // terms (-term) excluded
    }

    #[test]
    fn lang_keys_module_renders_union() {
        let out = render_lang_keys(&["alpha".into(), "zeta".into()]);
        assert!(out.contains(r#"| "alpha""#) && out.contains("export type MessageKey"));
    }

    #[test]
    fn extract_message_ids_dedupes_repeated_ids() {
        // Two files (or a copy/paste within one) can redeclare a key;
        // the union must not repeat the literal.
        let ftl = "dup = A\ndup = A again\nsolo = B\n";
        assert_eq!(extract_message_ids(ftl), vec!["dup", "solo"]);
    }

    #[test]
    fn extract_message_ids_on_malformed_source_is_empty_not_partial() {
        // `zeta` parses fine on its own, but the garbage line after it
        // makes the overall parse `Err`, and the all-or-nothing policy
        // means `zeta` must not leak through as a "partial" result.
        let ftl = "zeta = Z\n@@@ not a valid entry @@@\n";
        assert_eq!(extract_message_ids(ftl), Vec::<String>::new());
    }

    #[test]
    fn render_lang_keys_terminates_the_last_member_with_a_semicolon() {
        let out = render_lang_keys(&["alpha".into(), "zeta".into()]);
        assert_eq!(
            out,
            "export type MessageKey =\n  | \"alpha\"\n  | \"zeta\";\n"
        );
    }

    #[test]
    fn render_lang_keys_of_empty_ids_does_not_panic() {
        // `render_lang_keys` is `pub` and callable directly (not only
        // through `generate_lang_keys_to_file`, which never calls it with
        // an empty slice), so it must not underflow on `ids.len() - 1`.
        assert_eq!(render_lang_keys(&[]), "export type MessageKey = never;\n");
    }

    #[test]
    fn render_lang_keys_file_matches_the_documented_generated_header() {
        let out = render_lang_keys_file(
            &["welcome".into(), "validation-min".into()],
            &["en".to_string()],
        );
        assert_eq!(
            out,
            "// Generated by `suprnova generate-types` — do not edit.\n\
             // Message ids from lang/en/*.ftl.\n\
             export type MessageKey =\n\
             \u{20}\u{20}| \"welcome\"\n\
             \u{20}\u{20}| \"validation-min\";\n"
        );
    }

    #[test]
    fn render_lang_keys_file_names_the_full_chain_when_there_is_one() {
        let out = render_lang_keys_file(
            &["welcome".into()],
            &["pt-PT".to_string(), "pt-BR".to_string()],
        );
        assert_eq!(
            out,
            "// Generated by `suprnova generate-types` — do not edit.\n\
             // Message ids from lang/pt-PT/*.ftl (chain: pt-PT -> pt-BR).\n\
             export type MessageKey =\n\
             \u{20}\u{20}| \"welcome\";\n"
        );
    }

    #[test]
    fn resolve_default_locale_defaults_to_en_without_a_dot_env() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(resolve_default_locale(dir.path()), "en");
    }

    #[test]
    fn resolve_default_locale_reads_app_locale_from_dot_env() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join(".env"), "APP_LOCALE=fr\nOTHER=1\n").expect("write .env");
        assert_eq!(resolve_default_locale(dir.path()), "fr");
    }

    #[test]
    fn resolve_default_locale_falls_back_to_en_when_app_locale_is_blank() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join(".env"), "APP_LOCALE=\n").expect("write .env");
        assert_eq!(resolve_default_locale(dir.path()), "en");
    }

    #[test]
    fn resolve_locale_parents_is_empty_without_app_locale_parents() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(resolve_locale_parents(dir.path()), HashMap::new());
    }

    #[test]
    fn resolve_locale_parents_reads_pairs_from_dot_env() {
        // No internal whitespace, matching the manual's
        // `APP_LOCALE_PARENTS=pt-PT=pt-BR` shape — `.env` file syntax
        // itself (not this function) is what constrains what's writable
        // here; see `parse_locale_parents_*` below for whitespace
        // tolerance in the parsed value, tested directly against the pure
        // parser rather than fighting `dotenvy`'s line grammar.
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(
            dir.path().join(".env"),
            "APP_LOCALE_PARENTS=pt-PT=pt-BR,en-AU=en-GB\n",
        )
        .expect("write .env");
        let parents = resolve_locale_parents(dir.path());
        assert_eq!(parents.len(), 2);
        assert_eq!(parents.get("pt-PT"), Some(&"pt-BR".to_string()));
        assert_eq!(parents.get("en-AU"), Some(&"en-GB".to_string()));
    }

    #[test]
    fn parse_locale_parents_trims_whitespace_around_segments_and_sides() {
        let parents = parse_locale_parents(" pt-PT = pt-BR , en-AU=en-GB ");
        assert_eq!(parents.len(), 2);
        assert_eq!(parents.get("pt-PT"), Some(&"pt-BR".to_string()));
        assert_eq!(parents.get("en-AU"), Some(&"en-GB".to_string()));

        assert_eq!(parse_locale_parents(""), HashMap::new());
    }

    #[test]
    fn parse_locale_parents_skips_malformed_segments_without_erroring() {
        // Same tolerant posture as `resolve_default_locale`: a bad
        // segment is dropped, not fatal, and doesn't take the rest of the
        // value down with it. `no-equals` (missing `=`), the empty child
        // in `=orphan`, and the empty parent in `pt-PT=` are all
        // individually unusable — skipped, leaving only the one valid
        // pair.
        let parents = parse_locale_parents("no-equals,=orphan,pt-PT=,en-AU=en-GB");
        assert_eq!(parents.len(), 1);
        assert_eq!(parents.get("en-AU"), Some(&"en-GB".to_string()));
    }

    #[test]
    fn locale_chain_is_just_the_locale_without_parents() {
        assert_eq!(locale_chain("en", &HashMap::new()), vec!["en".to_string()]);
    }

    #[test]
    fn locale_chain_walks_multiple_hops() {
        let mut parents = HashMap::new();
        parents.insert("pt-PT".to_string(), "pt-BR".to_string());
        parents.insert("pt-BR".to_string(), "pt".to_string());
        assert_eq!(
            locale_chain("pt-PT", &parents),
            vec!["pt-PT".to_string(), "pt-BR".to_string(), "pt".to_string()]
        );
    }

    #[test]
    fn locale_chain_terminates_on_a_cycle_instead_of_hanging() {
        // `a -> b -> a`: without the visited-set guard this loops forever.
        let mut parents = HashMap::new();
        parents.insert("a".to_string(), "b".to_string());
        parents.insert("b".to_string(), "a".to_string());
        assert_eq!(
            locale_chain("a", &parents),
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn locale_chain_terminates_on_a_self_cycle() {
        let mut parents = HashMap::new();
        parents.insert("es".to_string(), "es".to_string());
        assert_eq!(locale_chain("es", &parents), vec!["es".to_string()]);
    }

    #[test]
    fn collect_lang_message_ids_is_empty_without_a_lang_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(
            collect_lang_message_ids(dir.path(), "en"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn collect_lang_message_ids_aggregates_and_dedupes_across_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let locale_dir = dir.path().join("lang/en");
        fs::create_dir_all(&locale_dir).expect("mkdir lang/en");
        fs::write(locale_dir.join("a.ftl"), "zeta = Z\nshared = one\n").expect("write a.ftl");
        fs::write(locale_dir.join("b.ftl"), "alpha = A\nshared = two\n").expect("write b.ftl");
        // Not `.ftl` — must be ignored.
        fs::write(locale_dir.join("notes.txt"), "zzz = should not appear\n")
            .expect("write notes.txt");

        assert_eq!(
            collect_lang_message_ids(dir.path(), "en"),
            vec!["alpha", "shared", "zeta"]
        );
    }

    #[test]
    fn collect_lang_message_ids_skips_a_malformed_file_without_panicking() {
        let dir = tempfile::tempdir().expect("tempdir");
        let locale_dir = dir.path().join("lang/en");
        fs::create_dir_all(&locale_dir).expect("mkdir lang/en");
        fs::write(locale_dir.join("good.ftl"), "welcome = Hi\n").expect("write good.ftl");
        fs::write(
            locale_dir.join("bad.ftl"),
            "also-good = Hi\n@@@ not valid fluent @@@\n",
        )
        .expect("write bad.ftl");

        // Must not panic (that alone is most of the assertion), and the
        // malformed file contributes nothing — not even `also-good`,
        // which would otherwise have parsed fine on its own.
        let ids = collect_lang_message_ids(dir.path(), "en");
        assert_eq!(ids, vec!["welcome"]);
    }

    #[test]
    fn generate_lang_keys_to_file_writes_sorted_union_when_ids_present() {
        let dir = tempfile::tempdir().expect("tempdir");
        let locale_dir = dir.path().join("lang/en");
        fs::create_dir_all(&locale_dir).expect("mkdir lang/en");
        fs::write(
            locale_dir.join("messages.ftl"),
            "welcome = Hi\nvalidation-min = Too short\n",
        )
        .expect("write messages.ftl");

        let output_path = dir.path().join("frontend/src/types/lang-keys.ts");
        let count =
            generate_lang_keys_to_file(dir.path(), &output_path).expect("generation must succeed");
        assert_eq!(count, 2);

        let written = fs::read_to_string(&output_path).expect("read generated file");
        assert_eq!(
            written,
            "// Generated by `suprnova generate-types` — do not edit.\n\
             // Message ids from lang/en/*.ftl.\n\
             export type MessageKey =\n\
             \u{20}\u{20}| \"validation-min\"\n\
             \u{20}\u{20}| \"welcome\";\n"
        );
    }

    #[test]
    fn generate_lang_keys_to_file_does_not_write_without_a_lang_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let output_path = dir.path().join("frontend/src/types/lang-keys.ts");

        let count = generate_lang_keys_to_file(dir.path(), &output_path)
            .expect("a non-localized project is not an error");
        assert_eq!(count, 0);
        assert!(
            !output_path.exists(),
            "non-localized projects must see no new artifact"
        );
    }

    #[test]
    fn generate_lang_keys_to_file_removes_a_stale_file_when_ids_disappear() {
        let dir = tempfile::tempdir().expect("tempdir");
        let output_path = dir.path().join("frontend/src/types/lang-keys.ts");
        fs::create_dir_all(output_path.parent().unwrap()).expect("mkdir output dir");
        fs::write(&output_path, "export type MessageKey =\n  | \"stale\";\n")
            .expect("seed a stale lang-keys.ts");

        // No lang/ dir this run (e.g. it was deleted, or every catalog now
        // declares zero messages) — the stale file must be cleaned up, not
        // left around asserting keys that no longer exist.
        let count = generate_lang_keys_to_file(dir.path(), &output_path)
            .expect("removing a stale file is not an error");
        assert_eq!(count, 0);
        assert!(!output_path.exists(), "the stale file must be removed");
    }

    #[test]
    fn generate_lang_keys_to_file_unions_a_delta_child_with_its_parent() {
        // `pt-PT` only overrides one key; the rest lives in `pt-BR`, its
        // configured parent. The generated union must cover both — a
        // frontend referencing a `pt-BR`-only key is valid, because the
        // served, chain-flattened catalog resolves it.
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(
            dir.path().join(".env"),
            "APP_LOCALE=pt-PT\nAPP_LOCALE_PARENTS=pt-PT=pt-BR\n",
        )
        .expect("write .env");

        let pt_pt = dir.path().join("lang/pt-PT");
        fs::create_dir_all(&pt_pt).expect("mkdir lang/pt-PT");
        fs::write(pt_pt.join("messages.ftl"), "custom-pt = So em PT\n")
            .expect("write pt-PT messages.ftl");

        let pt_br = dir.path().join("lang/pt-BR");
        fs::create_dir_all(&pt_br).expect("mkdir lang/pt-BR");
        fs::write(
            pt_br.join("messages.ftl"),
            "welcome = Bem-vindo\nvalidation-min = Curto\n",
        )
        .expect("write pt-BR messages.ftl");

        let output_path = dir.path().join("frontend/src/types/lang-keys.ts");
        let count =
            generate_lang_keys_to_file(dir.path(), &output_path).expect("generation must succeed");
        assert_eq!(count, 3);

        let written = fs::read_to_string(&output_path).expect("read generated file");
        assert!(written.contains(r#"| "custom-pt""#));
        assert!(written.contains(r#"| "welcome""#));
        assert!(written.contains(r#"| "validation-min""#));
    }

    #[test]
    fn generate_lang_keys_to_file_uses_the_parent_when_the_child_has_no_directory() {
        // `pt-PT` has no `lang/pt-PT/` at all yet (nothing overridden), but
        // its configured parent `pt-BR` has keys. This must resolve to the
        // parent's keys, not the empty-union delete branch — the served
        // catalog for `pt-PT` still resolves every one of them via the
        // fallback chain, so the type must not be deleted out from under a
        // working app.
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(
            dir.path().join(".env"),
            "APP_LOCALE=pt-PT\nAPP_LOCALE_PARENTS=pt-PT=pt-BR\n",
        )
        .expect("write .env");

        let pt_br = dir.path().join("lang/pt-BR");
        fs::create_dir_all(&pt_br).expect("mkdir lang/pt-BR");
        fs::write(pt_br.join("messages.ftl"), "welcome = Bem-vindo\n")
            .expect("write pt-BR messages.ftl");

        let output_path = dir.path().join("frontend/src/types/lang-keys.ts");
        // Seed a stale file to prove the delete branch is *not* taken —
        // it should be overwritten with the parent's keys, not removed.
        fs::create_dir_all(output_path.parent().unwrap()).expect("mkdir output dir");
        fs::write(&output_path, "export type MessageKey =\n  | \"stale\";\n")
            .expect("seed a stale lang-keys.ts");

        let count =
            generate_lang_keys_to_file(dir.path(), &output_path).expect("generation must succeed");
        assert_eq!(count, 1);
        assert!(
            output_path.exists(),
            "a parent with keys must not be treated as an empty union"
        );
        let written = fs::read_to_string(&output_path).expect("read generated file");
        assert!(written.contains(r#"| "welcome""#));
        assert!(!written.contains("stale"));
    }

    #[test]
    fn generate_lang_keys_to_file_unions_a_three_level_chain() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(
            dir.path().join(".env"),
            "APP_LOCALE=pt-PT\nAPP_LOCALE_PARENTS=pt-PT=pt-BR,pt-BR=pt\n",
        )
        .expect("write .env");

        for (locale, ftl) in [
            ("pt-PT", "only-pt-pt = A\n"),
            ("pt-BR", "only-pt-br = B\n"),
            ("pt", "only-pt = C\n"),
        ] {
            let locale_dir = dir.path().join("lang").join(locale);
            fs::create_dir_all(&locale_dir).expect("mkdir lang/<locale>");
            fs::write(locale_dir.join("messages.ftl"), ftl).expect("write messages.ftl");
        }

        let output_path = dir.path().join("frontend/src/types/lang-keys.ts");
        let count =
            generate_lang_keys_to_file(dir.path(), &output_path).expect("generation must succeed");
        assert_eq!(count, 3);

        let written = fs::read_to_string(&output_path).expect("read generated file");
        assert!(written.contains(r#"| "only-pt-pt""#));
        assert!(written.contains(r#"| "only-pt-br""#));
        assert!(written.contains(r#"| "only-pt""#));
    }

    #[test]
    fn generate_lang_keys_to_file_terminates_on_a_cyclic_app_locale_parents() {
        // `a=b,b=a`: a hand-authored cycle. `resolve_locale_parents`
        // performs no cycle rejection (unlike the framework's
        // `parse_parents`), so this must not hang — `locale_chain`'s
        // visited-set guard is what stands between this and an infinite
        // loop, and this test is the end-to-end proof it actually holds.
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(
            dir.path().join(".env"),
            "APP_LOCALE=a\nAPP_LOCALE_PARENTS=a=b,b=a\n",
        )
        .expect("write .env");

        let a_dir = dir.path().join("lang/a");
        fs::create_dir_all(&a_dir).expect("mkdir lang/a");
        fs::write(a_dir.join("messages.ftl"), "only-a = A\n").expect("write lang/a/messages.ftl");

        let b_dir = dir.path().join("lang/b");
        fs::create_dir_all(&b_dir).expect("mkdir lang/b");
        fs::write(b_dir.join("messages.ftl"), "only-b = B\n").expect("write lang/b/messages.ftl");

        let output_path = dir.path().join("frontend/src/types/lang-keys.ts");
        let count = generate_lang_keys_to_file(dir.path(), &output_path)
            .expect("a cycle must not be a hard error");
        assert_eq!(count, 2, "both cycle members are walked exactly once");

        let written = fs::read_to_string(&output_path).expect("read generated file");
        assert!(written.contains(r#"| "only-a""#));
        assert!(written.contains(r#"| "only-b""#));
    }
}
