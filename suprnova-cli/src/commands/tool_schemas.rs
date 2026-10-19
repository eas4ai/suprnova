//! `frontend/src/types/tool-schemas.ts`: the JSON Schema of every form
//! request, for the kits' form tools (AGENT-015).
//!
//! A kit form tool registers its form's server request type as its input
//! schema, and that schema must be the one the server derives: what
//! `#[tool_input]` publishes through `schemars` (draft 2020-12, the
//! deserialize contract). The CLI cannot compile the project, so it reads
//! each request struct with `syn` and writes, for a closed set of
//! constructs, exactly the JSON `schemars` 1.2.1 writes, key order
//! included. A struct with anything outside that set gets no schema and a
//! refusal naming it and the reason: an approximate schema would let an
//! agent send input the server refuses, or keep it from input the server
//! takes.
//!
//! The supported set:
//! - named-field structs without generics;
//! - fields of `String`, `bool`, the integer and float types, and
//!   `Option<T>` and `Vec<T>` of supported types;
//! - serde's `rename` and `rename_all` (the deserialize names),
//!   `deny_unknown_fields`, `default` (a field's, or the struct's when it
//!   derives `Default`), `skip`, `skip_deserializing` and `alias`;
//! - doc comments;
//! - the `validator` rules, read as `schemars` reads them: `length`,
//!   `range`, `contains`, `email`, `url`, `ip` and `required` shape the
//!   schema, and `custom`, `must_match`, `credit_card`,
//!   `non_control_character`, `does_not_contain` and `nested` leave it as it
//!   is, since `schemars` writes nothing for them either.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use syn::ext::IdentExt;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::{
    Attribute, Expr, ExprLit, ExprUnary, Fields, GenericArgument, ItemImpl, ItemStruct, Lit, Meta,
    PathArguments, Token, Type, UnOp,
};
use walkdir::WalkDir;

use super::generate_types::{
    RenameRule, derive_list_names, direction_value, names_framework_item, ts_property_key,
    write_if_changed,
};

/// The draft the server's schemas declare, as `schemars` writes it.
const DRAFT_2020_12: &str = "https://json-schema.org/draft/2020-12/schema";

/// A JSON value whose objects keep the order their keys were written in,
/// so the generated file lists each schema's keys in the order `schemars`
/// does, whatever `serde_json` features the CLI is built with.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// An integer, as an integer literal or a type's bound gives it.
    Int(i128),
    /// A float literal's value.
    Float(f64),
    /// A string.
    Str(String),
    /// An array.
    Array(Vec<Json>),
    /// An object, its entries in insertion order.
    Object(Vec<(String, Json)>),
}

impl Json {
    fn str(value: &str) -> Self {
        Self::Str(value.to_string())
    }

    fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Self::Object(entries) => entries
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    /// Set `key`: in its place when the object has it, as `serde_json`'s
    /// order-preserving map does, else at the end.
    fn insert(&mut self, key: &str, value: Json) {
        if let Self::Object(entries) = self {
            match entries.iter_mut().find(|(name, _)| name == key) {
                Some((_, slot)) => *slot = value,
                None => entries.push((key.to_string(), value)),
            }
        }
    }

    /// Whether the schema's `type` is, or lists, `ty`.
    fn has_type(&self, ty: &str) -> bool {
        match self.get("type") {
            Some(Self::Str(name)) => name == ty,
            Some(Self::Array(names)) => names.iter().any(|name| *name == Self::str(ty)),
            _ => false,
        }
    }

    /// The value as `serde_json` holds it, for comparing with what the
    /// server derives.
    // Used by the integration tests (`tests/agents_browser/schemas.rs`),
    // which the binary target's dead-code lint cannot see.
    #[allow(dead_code)]
    pub fn to_value(&self) -> serde_json::Value {
        use serde_json::Value;
        match self {
            Self::Null => Value::Null,
            Self::Bool(flag) => Value::Bool(*flag),
            Self::Int(number) => i64::try_from(*number)
                .map(Value::from)
                .or_else(|_| u64::try_from(*number).map(Value::from))
                .unwrap_or(Value::Null),
            Self::Float(number) => serde_json::Number::from_f64(*number)
                .map(Value::Number)
                .unwrap_or(Value::Null),
            Self::Str(text) => Value::String(text.clone()),
            Self::Array(items) => Value::Array(items.iter().map(Self::to_value).collect()),
            Self::Object(entries) => Value::Object(
                entries
                    .iter()
                    .map(|(key, value)| (key.clone(), value.to_value()))
                    .collect(),
            ),
        }
    }

    fn is_scalar(&self) -> bool {
        !matches!(self, Self::Array(_) | Self::Object(_))
    }

    /// Pretty JSON at `indent` spaces: objects one key per line, arrays of
    /// scalars on one line.
    fn render(&self, out: &mut String, indent: usize) {
        match self {
            Self::Null => out.push_str("null"),
            Self::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
            Self::Int(number) => out.push_str(&number.to_string()),
            // `serde_json`'s own float text, so `0.0` stays `0.0`.
            Self::Float(number) => {
                out.push_str(&serde_json::to_string(number).unwrap_or_else(|_| "null".into()))
            }
            Self::Str(text) => push_string(out, text),
            Self::Array(items) if items.is_empty() => out.push_str("[]"),
            Self::Array(items) if items.iter().all(Self::is_scalar) => {
                out.push('[');
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        out.push_str(", ");
                    }
                    item.render(out, indent);
                }
                out.push(']');
            }
            Self::Array(items) => {
                out.push_str("[\n");
                for (index, item) in items.iter().enumerate() {
                    out.push_str(&" ".repeat(indent + 2));
                    item.render(out, indent + 2);
                    out.push_str(if index + 1 < items.len() { ",\n" } else { "\n" });
                }
                out.push_str(&" ".repeat(indent));
                out.push(']');
            }
            Self::Object(entries) if entries.is_empty() => out.push_str("{}"),
            Self::Object(entries) => {
                out.push_str("{\n");
                for (index, (key, value)) in entries.iter().enumerate() {
                    out.push_str(&" ".repeat(indent + 2));
                    push_string(out, key);
                    out.push_str(": ");
                    value.render(out, indent + 2);
                    out.push_str(if index + 1 < entries.len() {
                        ",\n"
                    } else {
                        "\n"
                    });
                }
                out.push_str(&" ".repeat(indent));
                out.push('}');
            }
        }
    }
}

/// `text` as a JSON string, escaped as `serde_json` escapes it.
fn push_string(out: &mut String, text: &str) {
    out.push_str(&serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into()));
}

/// A form request the generator cannot describe exactly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// The struct's name.
    pub name: String,
    /// The file that declares it, relative to the project.
    pub file: PathBuf,
    /// Why it has no schema.
    pub reason: String,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "No tool schema for `{}` ({}): {}.",
            self.name,
            self.file.display(),
            self.reason
        )
    }
}

/// What one scan found: a schema per form request it can describe, keyed
/// by the struct's name and in name order, and a refusal per one it
/// cannot.
#[derive(Debug, Clone, Default)]
pub struct ToolSchemas {
    /// Each schema, keyed by its struct's name.
    pub schemas: Vec<(String, Json)>,
    /// Each form request that got no schema.
    pub refusals: Vec<Refusal>,
}

impl ToolSchemas {
    /// The generated module, `frontend/src/types/tool-schemas.ts`.
    pub fn render(&self) -> String {
        let mut out = String::from(
            "// This file is auto-generated by Suprnova. Do not edit manually.\n\
             // Run `suprnova generate-types` to regenerate.\n\
             //\n\
             // The JSON Schema (draft 2020-12) of each form request under `src/`,\n\
             // keyed by the struct's name, exactly as the server derives it. A kit\n\
             // form tool registers one as its input schema:\n\
             // `useFormTool(form, { schema: toolSchemas.StoreNoteRequest, ... })`.\n\n",
        );
        if self.schemas.is_empty() {
            out.push_str("export const toolSchemas = {} as const\n");
        } else {
            out.push_str("export const toolSchemas = {\n");
            for (name, schema) in &self.schemas {
                out.push_str("  ");
                out.push_str(&ts_property_key(name));
                out.push_str(": ");
                schema.render(&mut out, 2);
                out.push_str(",\n");
            }
            out.push_str("} as const\n");
        }
        out.push_str("\nexport type ToolSchemaName = keyof typeof toolSchemas\n");
        out
    }
}

/// Scan one source file, as `src/lib.rs` of a project.
// Used by the integration tests, as `Json::to_value` is.
#[allow(dead_code)]
pub fn scan_source(source: &str) -> Result<ToolSchemas, String> {
    let syntax = syn::parse_file(source).map_err(|error| error.to_string())?;
    let mut collector = Collector::new(Path::new("src/lib.rs"));
    collector.visit_file(&syntax);
    Ok(collector.finish())
}

/// Scan every `.rs` file under the project's `src/`, in name order. A file
/// that cannot be read or parsed fails the scan, since a partial scan would
/// drop schemas the project has.
pub fn scan_project(project_path: &Path) -> Result<ToolSchemas, String> {
    let src = project_path.join("src");
    let mut collector = Collector::new(Path::new("src"));
    let mut failures = Vec::new();
    for entry in WalkDir::new(&src).sort_by_file_name() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                failures.push(error.to_string());
                continue;
            }
        };
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "rs") {
            continue;
        }
        let shown = Path::new("src").join(path.strip_prefix(&src).unwrap_or(path));
        let parsed = fs::read_to_string(path)
            .map_err(|error| format!("{}: read failed: {error}", shown.display()))
            .and_then(|content| {
                syn::parse_file(&content)
                    .map_err(|error| format!("{}: parse failed: {error}", shown.display()))
            });
        match parsed {
            Ok(syntax) => {
                collector.file = shown;
                collector.visit_file(&syntax);
            }
            Err(failure) => failures.push(failure),
        }
    }
    if !failures.is_empty() {
        return Err(format!(
            "Rust source scan failed with {} error(s); tool schemas were left unchanged:\n- {}",
            failures.len(),
            failures.join("\n- ")
        ));
    }
    Ok(collector.finish())
}

/// What writing the module did.
pub struct ToolSchemasOutcome {
    /// How many schemas the module holds.
    pub count: usize,
    /// The form requests that got no schema.
    pub refusals: Vec<Refusal>,
    /// Whether the file on disk changed.
    pub wrote: bool,
}

/// Scan the project and write the module to `output_path`, only when its
/// bytes change, as `inertia-props.ts` is written.
pub fn generate_tool_schemas_to_file(
    project_path: &Path,
    output_path: &Path,
) -> Result<ToolSchemasOutcome, String> {
    let scan = scan_project(project_path)?;
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create output directory: {error}"))?;
    }
    let wrote = write_if_changed(output_path, &scan.render())?;
    Ok(ToolSchemasOutcome {
        count: scan.schemas.len(),
        refusals: scan.refusals,
        wrote,
    })
}

/// A struct the scan found, and where.
struct Found {
    file: PathBuf,
    item: ItemStruct,
    declared: bool,
}

/// Collects the structs of every file and the names a `FormRequest` impl
/// is written for, since a request type is declared either way.
struct Collector {
    file: PathBuf,
    structs: Vec<Found>,
    impls: BTreeSet<String>,
}

impl Collector {
    fn new(file: &Path) -> Self {
        Self {
            file: file.to_path_buf(),
            structs: Vec::new(),
            impls: BTreeSet::new(),
        }
    }

    /// Each form request's schema or refusal. Two requests of one name
    /// cannot share a key of `toolSchemas`, so both are refused.
    fn finish(self) -> ToolSchemas {
        let Self { structs, impls, .. } = self;
        let mut by_name: BTreeMap<String, Vec<Found>> = BTreeMap::new();
        for found in structs {
            let name = found.item.ident.unraw().to_string();
            if found.declared || impls.contains(&name) {
                by_name.entry(name).or_default().push(found);
            }
        }
        let mut scan = ToolSchemas::default();
        for (name, found) in by_name {
            if found.len() > 1 {
                let files: Vec<String> = found
                    .iter()
                    .map(|found| found.file.display().to_string())
                    .collect();
                for each in &found {
                    scan.refusals.push(Refusal {
                        name: name.clone(),
                        file: each.file.clone(),
                        reason: format!(
                            "another form request has the same name ({}), and `toolSchemas` \
                             keys each schema by its struct's name",
                            files.join(", ")
                        ),
                    });
                }
                continue;
            }
            let Some(found) = found.into_iter().next() else {
                continue;
            };
            match schema_of(&found.item) {
                Ok(schema) => scan.schemas.push((name, schema)),
                Err(reason) => scan.refusals.push(Refusal {
                    name,
                    file: found.file,
                    reason,
                }),
            }
        }
        scan
    }
}

impl<'ast> Visit<'ast> for Collector {
    fn visit_item_struct(&mut self, node: &'ast ItemStruct) {
        self.structs.push(Found {
            file: self.file.clone(),
            item: node.clone(),
            declared: declares_form_request(&node.attrs),
        });
        syn::visit::visit_item_struct(self, node);
    }

    fn visit_item_impl(&mut self, node: &'ast ItemImpl) {
        if let (Some((None, trait_path, _)), Type::Path(self_ty)) = (&node.trait_, &*node.self_ty)
            && names_framework_item(trait_path, "FormRequest")
            && self_ty.qself.is_none()
            && let Some(segment) = self_ty.path.segments.last()
        {
            self.impls.insert(segment.ident.unraw().to_string());
        }
        syn::visit::visit_item_impl(self, node);
    }
}

/// Whether the attributes declare a form request: `#[request]`,
/// `#[tool_input]`, the `FormRequest` derive (the crate root names it
/// `FormRequestDerive`) or its `#[form_request(..)]` helper.
fn declares_form_request(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        let path = attr.path();
        path.is_ident("form_request")
            || names_framework_item(path, "request")
            || names_framework_item(path, "tool_input")
    }) || derive_list_names(attrs, "FormRequest")
        || derive_list_names(attrs, "FormRequestDerive")
}

/// What serde's container attributes say about the schema.
#[derive(Default)]
struct ContainerSerde {
    rename: Option<String>,
    rename_all: Option<RenameRule>,
    deny_unknown_fields: bool,
    default: bool,
}

/// The schema of a form request struct, as `schemars` derives it, or why
/// the generator cannot write it exactly.
fn schema_of(item: &ItemStruct) -> Result<Json, String> {
    if !item.generics.params.is_empty() {
        return Err("it is generic".into());
    }
    check_attributes(&item.attrs, "it has")?;
    let serde = container_serde(&item.attrs)?;
    if serde.default && !derive_list_names(&item.attrs, "Default") {
        return Err(
            "`#[serde(default)]` on the struct reads its `Default` implementation, \
             which generate-types can only read when the struct derives `Default`"
                .into(),
        );
    }
    let Fields::Named(named) = &item.fields else {
        return Err("it has no named fields".into());
    };
    let doc = doc_text(&item.attrs).map_err(|reason| format!("it has {reason}"))?;

    let mut schema = Json::Object(vec![("type".into(), Json::str("object"))]);
    if serde.deny_unknown_fields {
        schema.insert("additionalProperties", Json::Bool(false));
    }
    let mut properties: Vec<(String, Json)> = Vec::new();
    let mut required: Vec<Json> = Vec::new();
    for field in &named.named {
        let Some(property) = property_of(field, &serde)? else {
            continue;
        };
        if properties.iter().any(|(name, _)| *name == property.name) {
            return Err(format!(
                "two fields read the key `{}`, which serde refuses",
                property.name
            ));
        }
        if property.required {
            required.push(Json::str(&property.name));
        }
        properties.push((property.name, property.schema));
    }
    if !properties.is_empty() {
        schema.insert("properties", Json::Object(properties));
    }
    if !required.is_empty() {
        schema.insert("required", Json::Array(required));
    }
    if let Some(doc) = doc {
        insert_doc(&mut schema, &doc);
    }
    if schema.get("title").is_none() {
        let name = serde
            .rename
            .unwrap_or_else(|| item.ident.unraw().to_string());
        schema.insert("title", Json::Str(name));
    }
    schema.insert("$schema", Json::str(DRAFT_2020_12));
    Ok(schema)
}

/// Refuse an attribute that changes the schema in a way the generator
/// does not follow. `subject` starts the reason: "it has" for the struct,
/// "field `x` has" for a field.
fn check_attributes(attrs: &[Attribute], subject: &str) -> Result<(), String> {
    for attr in attrs {
        let path = attr.path();
        if path.is_ident("schemars") || path.is_ident("garde") {
            let name = path
                .get_ident()
                .map(ToString::to_string)
                .unwrap_or_default();
            return Err(format!(
                "{subject} a `{name}` attribute, which generate-types does not read"
            ));
        }
        if path.is_ident("cfg") || path.is_ident("cfg_attr") {
            return Err(format!(
                "{subject} the attribute `cfg`, so its schema depends on the build"
            ));
        }
        if path.is_ident("deprecated") {
            return Err(format!(
                "{subject} the attribute `deprecated`, which generate-types does not support"
            ));
        }
    }
    Ok(())
}

/// The arguments of every `attr_name` attribute, or the reason one does
/// not parse.
fn attribute_args(
    attrs: &[Attribute],
    attr_name: &str,
    subject: &str,
) -> Result<Vec<Meta>, String> {
    let mut metas = Vec::new();
    for attr in attrs.iter().filter(|attr| attr.path().is_ident(attr_name)) {
        let parsed = attr
            .parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
            .map_err(|_| {
                format!("{subject} a `{attr_name}` attribute generate-types cannot read")
            })?;
        metas.extend(parsed);
    }
    Ok(metas)
}

/// `meta` as the source writes it: `flatten`, `default = "ten"`.
fn meta_text(meta: &Meta) -> String {
    source_text(meta.span()).unwrap_or_else(|| path_text(meta.path()))
}

/// The source text a span covers. The CLI parses files itself, so
/// `proc-macro2`'s `span-locations` holds every span's text.
fn source_text(span: proc_macro2::Span) -> Option<String> {
    span.source_text()
}

fn path_text(path: &syn::Path) -> String {
    path.segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}

fn container_serde(attrs: &[Attribute]) -> Result<ContainerSerde, String> {
    let mut serde = ContainerSerde::default();
    for meta in attribute_args(attrs, "serde", "it has")? {
        let unsupported = || {
            format!(
                "it has the serde attribute `{}`, which generate-types does not support",
                meta_text(&meta)
            )
        };
        let path = meta.path();
        if path.is_ident("rename") {
            serde.rename = deserialize_name(&meta).ok_or_else(unsupported)?;
        } else if path.is_ident("rename_all") {
            serde.rename_all = match deserialize_name(&meta).ok_or_else(unsupported)? {
                Some(rule) => Some(RenameRule::parse(&rule).ok_or_else(unsupported)?),
                None => None,
            };
        } else if path.is_ident("deny_unknown_fields") && matches!(meta, Meta::Path(_)) {
            serde.deny_unknown_fields = true;
        } else if path.is_ident("default") && matches!(meta, Meta::Path(_)) {
            serde.default = true;
        } else {
            return Err(unsupported());
        }
    }
    Ok(serde)
}

/// The deserialize name `rename` or `rename_all` gives: `Some(None)` for a
/// form that renames the serialize direction only, `None` for a form serde
/// does not take.
fn deserialize_name(meta: &Meta) -> Option<Option<String>> {
    match meta {
        Meta::NameValue(_) => direction_value(meta, "deserialize").map(Some),
        Meta::List(_) => Some(direction_value(meta, "deserialize")),
        Meta::Path(_) => None,
    }
}

/// One property of the struct's schema.
struct Property {
    name: String,
    schema: Json,
    required: bool,
}

/// The property `field` adds, `None` when serde never reads the field, or
/// why the generator cannot write it.
fn property_of(field: &syn::Field, container: &ContainerSerde) -> Result<Option<Property>, String> {
    let Some(ident) = &field.ident else {
        return Err("it has no named fields".into());
    };
    let label = ident.unraw().to_string();
    let subject = format!("field `{label}` has");
    check_attributes(&field.attrs, &subject)?;

    let mut rename = None;
    let mut default = false;
    let mut skipped = false;
    for meta in attribute_args(&field.attrs, "serde", &subject)? {
        let unsupported = || {
            format!(
                "{subject} the serde attribute `{}`, which generate-types does not support",
                meta_text(&meta)
            )
        };
        let path = meta.path();
        if path.is_ident("rename") {
            if let Some(name) = deserialize_name(&meta).ok_or_else(unsupported)? {
                rename = Some(name);
            }
        } else if path.is_ident("alias") && matches!(meta, Meta::NameValue(_)) {
            // An alias is another name serde reads; the schema names the
            // field by its own name only, as `schemars` does.
        } else if path.is_ident("default") && matches!(meta, Meta::Path(_)) {
            default = true;
        } else if (path.is_ident("skip") || path.is_ident("skip_deserializing"))
            && matches!(meta, Meta::Path(_))
        {
            skipped = true;
        } else {
            return Err(unsupported());
        }
    }
    if skipped {
        return Ok(None);
    }

    let rules = rules_of(&field.attrs, &label)?;
    let doc = doc_text(&field.attrs).map_err(|reason| format!("{subject} {reason}"))?;
    let shape = Shape::of(&field.ty).ok_or_else(|| {
        format!(
            "{subject} the type `{}`, which generate-types cannot describe exactly; it \
             describes strings, booleans, numbers, and `Option` and `Vec` of them",
            source_text(field.ty.span()).unwrap_or_else(|| "its type".into())
        )
    })?;

    // `schemars` builds a field's schema in this order: the type's own,
    // then the doc comment's title and description, then the validation
    // rules, then the default.
    let mut schema = if rules.required {
        shape.non_optional_schema()
    } else {
        shape.schema()
    };
    if let Some(doc) = doc {
        insert_doc(&mut schema, &doc);
    }
    rules.apply(&mut schema);
    let has_default = default || container.default;
    if has_default {
        schema.insert("default", shape.default_value());
    }
    let name = rename.unwrap_or_else(|| match container.rename_all {
        Some(rule) => rule.apply(&label),
        None => label.clone(),
    });
    let optional = has_default || (!rules.required && shape.is_option());
    Ok(Some(Property {
        name,
        schema,
        required: !optional,
    }))
}

/// A field type the generator describes.
enum Shape {
    Base(&'static str),
    Option(Box<Shape>),
    Vec(Box<Shape>),
}

/// A number type and the schema `schemars` writes for it.
#[derive(Clone, Copy)]
struct Number {
    /// The Rust type's name.
    rust: &'static str,
    /// The schema's `type`: `integer` or `number`.
    json: &'static str,
    format: &'static str,
    /// The `minimum` and `maximum` written for the types narrower than 32
    /// bits.
    bounds: Option<(i128, i128)>,
}

const fn number(
    rust: &'static str,
    json: &'static str,
    format: &'static str,
    bounds: Option<(i128, i128)>,
) -> Number {
    Number {
        rust,
        json,
        format,
        bounds,
    }
}

/// The integer and float types.
const NUMBERS: &[Number] = &[
    number(
        "i8",
        "integer",
        "int8",
        Some((i8::MIN as i128, i8::MAX as i128)),
    ),
    number(
        "i16",
        "integer",
        "int16",
        Some((i16::MIN as i128, i16::MAX as i128)),
    ),
    number("i32", "integer", "int32", None),
    number("i64", "integer", "int64", None),
    number("i128", "integer", "int128", None),
    number("isize", "integer", "int", None),
    number("u8", "integer", "uint8", Some((0, u8::MAX as i128))),
    number("u16", "integer", "uint16", Some((0, u16::MAX as i128))),
    number("u32", "integer", "uint32", None),
    number("u64", "integer", "uint64", None),
    number("u128", "integer", "uint128", None),
    number("usize", "integer", "uint", None),
    number("f32", "number", "float", None),
    number("f64", "number", "double", None),
];

impl Shape {
    /// The shape of `ty`, written as a bare name: `String`, not
    /// `std::string::String`, so a project type of another meaning that
    /// shares a path's last name is never taken for it.
    fn of(ty: &Type) -> Option<Self> {
        let path = match ty {
            Type::Path(path) if path.qself.is_none() && path.path.leading_colon.is_none() => {
                &path.path
            }
            Type::Group(group) => return Self::of(&group.elem),
            _ => return None,
        };
        let [segment] = path.segments.iter().collect::<Vec<_>>()[..] else {
            return None;
        };
        let name = segment.ident.to_string();
        match &segment.arguments {
            PathArguments::None => ["String", "bool"]
                .into_iter()
                .chain(NUMBERS.iter().map(|number| number.rust))
                .find(|known| *known == name)
                .map(Self::Base),
            PathArguments::AngleBracketed(arguments) if arguments.args.len() == 1 => {
                let Some(GenericArgument::Type(inner)) = arguments.args.first() else {
                    return None;
                };
                let inner = Box::new(Self::of(inner)?);
                match name.as_str() {
                    "Option" => Some(Self::Option(inner)),
                    "Vec" => Some(Self::Vec(inner)),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn is_option(&self) -> bool {
        matches!(self, Self::Option(_))
    }

    /// The type's schema, as its `JsonSchema` impl writes it.
    fn schema(&self) -> Json {
        match self {
            Self::Base("String") => Json::Object(vec![("type".into(), Json::str("string"))]),
            Self::Base("bool") => Json::Object(vec![("type".into(), Json::str("boolean"))]),
            Self::Base(name) => {
                // `Shape::of` makes a `Base` only of `String`, `bool` and a
                // name in `NUMBERS`, so the fallback is never reached.
                let number = NUMBERS
                    .iter()
                    .find(|number| number.rust == *name)
                    .copied()
                    .unwrap_or(number("f64", "number", "double", None));
                let mut schema = Json::Object(vec![
                    ("type".into(), Json::str(number.json)),
                    ("format".into(), Json::str(number.format)),
                ]);
                match number.bounds {
                    Some((min, max)) => {
                        schema.insert("minimum", Json::Int(min));
                        schema.insert("maximum", Json::Int(max));
                    }
                    None if number.format.starts_with("uint") => {
                        schema.insert("minimum", Json::Int(0))
                    }
                    None => {}
                }
                schema
            }
            Self::Option(inner) => {
                let mut schema = inner.schema();
                allow_null(&mut schema);
                schema
            }
            Self::Vec(inner) => Json::Object(vec![
                ("type".into(), Json::str("array")),
                ("items".into(), inner.schema()),
            ]),
        }
    }

    /// The schema without the null an `Option` adds, which a field marked
    /// `#[validate(required)]` gets.
    fn non_optional_schema(&self) -> Json {
        match self {
            Self::Option(inner) => inner.non_optional_schema(),
            other => other.schema(),
        }
    }

    /// The type's `Default::default()`, as JSON.
    fn default_value(&self) -> Json {
        match self {
            Self::Base("String") => Json::str(""),
            Self::Base("bool") => Json::Bool(false),
            Self::Base(name) if name.starts_with('f') => Json::Float(0.0),
            Self::Base(_) => Json::Int(0),
            Self::Option(_) => Json::Null,
            Self::Vec(_) => Json::Array(Vec::new()),
        }
    }
}

/// Add `null` to the schema's `type`, as `schemars` does for an `Option`.
fn allow_null(schema: &mut Json) {
    let Some(ty) = schema.get("type").cloned() else {
        return;
    };
    let null = Json::str("null");
    match ty {
        Json::Str(name) if name != "null" => {
            schema.insert("type", Json::Array(vec![Json::Str(name), null]));
        }
        Json::Array(mut names) if !names.contains(&null) => {
            names.push(null);
            schema.insert("type", Json::Array(names));
        }
        _ => {}
    }
}

/// A `length` or `range` rule's bounds: `min`, else `equal`, and `max`,
/// else `equal`.
#[derive(Default)]
struct Bounds {
    min: Option<Json>,
    max: Option<Json>,
    equal: Option<Json>,
}

impl Bounds {
    fn lower(&self) -> Option<&Json> {
        self.min.as_ref().or(self.equal.as_ref())
    }

    fn upper(&self) -> Option<&Json> {
        self.max.as_ref().or(self.equal.as_ref())
    }
}

/// The validation rules of a field that shape its schema.
#[derive(Default)]
struct Rules {
    length: Option<Bounds>,
    range: Option<Bounds>,
    contains: Option<String>,
    format: Option<&'static str>,
    required: bool,
}

/// The validator rules `schemars` reads nothing from.
const RULES_WITHOUT_SCHEMA: &[&str] = &[
    "credit_card",
    "custom",
    "does_not_contain",
    "must_match",
    "nested",
    "non_control_character",
];

/// The validation rules of the field `label`, read as `schemars` reads
/// them: the first of a repeated rule or bound counts, and `min` or `max`
/// beside `equal` is dropped in the order written.
fn rules_of(attrs: &[Attribute], label: &str) -> Result<Rules, String> {
    let subject = format!("field `{label}` has");
    let mut rules = Rules::default();
    for meta in attribute_args(attrs, "validate", &subject)? {
        let rule = meta
            .path()
            .get_ident()
            .map(ToString::to_string)
            .unwrap_or_else(|| path_text(meta.path()));
        match rule.as_str() {
            "email" | "url" | "ip" => {
                rules.format.get_or_insert(match rule.as_str() {
                    "email" => "email",
                    "url" => "uri",
                    _ => "ip",
                });
            }
            "length" | "range" => {
                let Meta::List(list) = &meta else {
                    return Err(format!(
                        "{subject} the validation rule `{rule}` without bounds, which the \
                         `JsonSchema` derive refuses"
                    ));
                };
                let nested = list
                    .parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
                    .map_err(|_| format!("{subject} a `{rule}` rule generate-types cannot read"))?;
                let bounds = bounds_of(nested, &subject, &rule)?;
                let slot = if rule == "length" {
                    &mut rules.length
                } else {
                    &mut rules.range
                };
                slot.get_or_insert(bounds);
            }
            "contains" => {
                if rules.contains.is_none() {
                    rules.contains = Some(contains_pattern(&meta, &subject)?);
                }
            }
            "required" => {
                if !matches!(meta, Meta::Path(_)) {
                    return Err(format!(
                        "{subject} the validation rule `required` with arguments, which the \
                         `JsonSchema` derive refuses"
                    ));
                }
                rules.required = true;
            }
            "regex" => {
                return Err(format!(
                    "{subject} the validation rule `regex`, whose pattern is only known when \
                     the program runs"
                ));
            }
            known if RULES_WITHOUT_SCHEMA.contains(&known) => {}
            unknown => {
                return Err(format!(
                    "{subject} the validation rule `{unknown}`, which generate-types does not know"
                ));
            }
        }
    }
    Ok(rules)
}

fn bounds_of(
    nested: Punctuated<Meta, Token![,]>,
    subject: &str,
    rule: &str,
) -> Result<Bounds, String> {
    let mut bounds = Bounds::default();
    for meta in nested {
        let Meta::NameValue(pair) = &meta else {
            continue;
        };
        let key = path_text(&pair.path);
        if !matches!(key.as_str(), "min" | "max" | "equal") {
            // `message`, `code` and `exclusive_min` say nothing to `schemars`.
            continue;
        }
        let value = bound_value(&pair.value).ok_or_else(|| {
            format!(
                "{subject} the bound `{}` in `{rule}`, a value generate-types cannot read \
                 from the source",
                source_text(pair.value.span()).unwrap_or_else(|| "?".into())
            )
        })?;
        match key.as_str() {
            "min" if bounds.min.is_none() && bounds.equal.is_none() => bounds.min = Some(value),
            "max" if bounds.max.is_none() && bounds.equal.is_none() => bounds.max = Some(value),
            "equal" if bounds.min.is_none() && bounds.max.is_none() && bounds.equal.is_none() => {
                bounds.equal = Some(value)
            }
            _ => {}
        }
    }
    Ok(bounds)
}

/// A bound written as a number literal, negative or in a string, as
/// `schemars` takes it; `None` for anything only the compiler can work out.
fn bound_value(expr: &Expr) -> Option<Json> {
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Str(text),
            ..
        }) => bound_value(&syn::parse_str::<Expr>(&text.value()).ok()?),
        Expr::Lit(ExprLit {
            lit: Lit::Int(number),
            ..
        }) => match number.suffix() {
            "f64" => number.base10_parse::<f64>().ok().map(Json::Float),
            "f32" => None,
            _ => {
                let value = number.base10_parse::<i128>().ok()?;
                (i64::try_from(value).is_ok() || u64::try_from(value).is_ok())
                    .then_some(Json::Int(value))
            }
        },
        Expr::Lit(ExprLit {
            lit: Lit::Float(number),
            ..
        }) if matches!(number.suffix(), "" | "f64") => {
            number.base10_parse::<f64>().ok().map(Json::Float)
        }
        Expr::Unary(ExprUnary {
            op: UnOp::Neg(_),
            expr,
            ..
        }) => match bound_value(expr)? {
            Json::Int(value) => Some(Json::Int(-value)),
            Json::Float(value) => Some(Json::Float(-value)),
            _ => None,
        },
        _ => None,
    }
}

/// The literal `pattern` of a `contains` rule.
fn contains_pattern(meta: &Meta, subject: &str) -> Result<String, String> {
    let unreadable = || {
        format!(
            "{subject} a `contains` rule without a string `pattern`, which generate-types \
             cannot read"
        )
    };
    let Meta::List(list) = meta else {
        return Err(unreadable());
    };
    let nested = list
        .parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
        .map_err(|_| unreadable())?;
    nested
        .iter()
        .find_map(|meta| match meta {
            Meta::NameValue(pair) if pair.path.is_ident("pattern") => match &pair.value {
                Expr::Lit(ExprLit {
                    lit: Lit::Str(text),
                    ..
                }) => Some(text.value()),
                _ => None,
            },
            _ => None,
        })
        .ok_or_else(unreadable)
}

impl Rules {
    /// Write the rules into `schema` in `schemars`' order: `length` (on a
    /// string, then an array), `range`, the `contains` pattern, the format.
    fn apply(&self, schema: &mut Json) {
        if let Some(length) = &self.length {
            insert_bound(schema, "string", "minLength", length.lower());
            insert_bound(schema, "string", "maxLength", length.upper());
            insert_bound(schema, "array", "minItems", length.lower());
            insert_bound(schema, "array", "maxItems", length.upper());
        }
        if let Some(range) = &self.range {
            insert_bound(schema, "number", "minimum", range.lower());
            insert_bound(schema, "number", "maximum", range.upper());
        }
        if let Some(pattern) = &self.contains {
            let escaped = Json::Str(escape_regex(pattern));
            insert_bound(schema, "string", "pattern", Some(&escaped));
        }
        if let Some(format) = self.format {
            schema.insert("format", Json::str(format));
        }
    }
}

/// Insert a rule's `value` when the schema's type is `ty`; `number` takes
/// an `integer` too.
fn insert_bound(schema: &mut Json, ty: &str, key: &str, value: Option<&Json>) {
    let Some(value) = value else {
        return;
    };
    if schema.has_type(ty) || (ty == "number" && schema.has_type("integer")) {
        schema.insert(key, value.clone());
    }
}

/// `text` with every regex meta character escaped, as `schemars` escapes a
/// `contains` pattern.
fn escape_regex(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(
            c,
            '\\' | '.'
                | '+'
                | '*'
                | '?'
                | '('
                | ')'
                | '|'
                | '['
                | ']'
                | '{'
                | '}'
                | '^'
                | '$'
                | '#'
                | '&'
                | '-'
                | '~'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The doc comment, its lines joined as the derive joins them: one leading
/// space dropped from each, a line break between. `Err` names a doc
/// attribute that is not a string literal, whose text only the compiler
/// knows.
fn doc_text(attrs: &[Attribute]) -> Result<Option<String>, String> {
    let mut lines = Vec::new();
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("doc")) {
        let Meta::NameValue(pair) = &attr.meta else {
            continue;
        };
        let Expr::Lit(ExprLit {
            lit: Lit::Str(text),
            ..
        }) = &pair.value
        else {
            return Err("a doc comment that is not a string literal".into());
        };
        let text = text.value();
        lines.push(text.strip_prefix(' ').map(str::to_string).unwrap_or(text));
    }
    Ok((!lines.is_empty()).then(|| lines.join("\n")))
}

/// Add the doc comment's title and description, as `schemars` splits
/// them: a doc that opens with `#` titles the schema with that line.
fn insert_doc(schema: &mut Json, doc: &str) {
    let (title, description) = title_and_description(doc);
    if !title.is_empty() {
        schema.insert("title", Json::str(title));
    }
    if !description.is_empty() {
        schema.insert("description", Json::str(description));
    }
}

fn title_and_description(doc: &str) -> (&str, &str) {
    let doc = trim_ascii_whitespace(doc);
    if !doc.starts_with('#') {
        return ("", doc);
    }
    let end = doc.find('\n').unwrap_or(doc.len());
    (
        trim_ascii_whitespace(doc[..end].trim_start_matches('#')),
        trim_ascii_whitespace(&doc[end..]),
    )
}

/// `text` without the ASCII whitespace `u8::is_ascii_whitespace` names at
/// either end, as `schemars` trims a doc comment.
fn trim_ascii_whitespace(text: &str) -> &str {
    text.trim_matches(|c: char| c.is_ascii() && (c as u8).is_ascii_whitespace())
}
