//! Route generation for type-safe frontend integration
//!
//! Generates TypeScript route helpers compatible with Inertia.js v2+ UrlMethodPair interface.
//! This allows type-safe navigation with:
//! - `router.visit(controllers.user.show({ id: '123' }))`
//! - `form.submit(controllers.todo.store({ title: 'Task', completed: false }))`
//! - `<Link href={controllers.user.index()}>Users</Link>`

use proc_macro2::{Delimiter, TokenStream, TokenTree};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use syn::visit::Visit;
use syn::{Attribute, Fields, FnArg, ItemFn, ItemStruct, Type};
use walkdir::WalkDir;

use super::generate_types::{
    derive_list_names, names_framework_item, serde_input_key, ts_property_key,
};
use crate::ui;

/// HTTP methods for routes
#[derive(Debug, Clone, PartialEq)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl HttpMethod {
    fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "get" => Some(HttpMethod::Get),
            "post" => Some(HttpMethod::Post),
            "put" => Some(HttpMethod::Put),
            "patch" => Some(HttpMethod::Patch),
            "delete" => Some(HttpMethod::Delete),
            _ => None,
        }
    }

    fn to_ts_method(&self) -> &'static str {
        match self {
            HttpMethod::Get => "get",
            HttpMethod::Post => "post",
            HttpMethod::Put => "put",
            HttpMethod::Patch => "patch",
            HttpMethod::Delete => "delete",
        }
    }
}

/// A path parameter extracted from route patterns like /users/{id}
#[derive(Debug, Clone)]
pub struct PathParam {
    pub name: String,
    /// `{name?}`: the route matches without it, and the helper leaves it
    /// out the way the backend's `route()` does.
    pub optional: bool,
}

/// A parsed route definition from routes.rs
#[derive(Debug, Clone)]
pub struct RouteDefinition {
    pub method: HttpMethod,
    pub path: String,
    pub handler_module: String, // e.g., "controllers::user"
    pub handler_fn: String,     // e.g., "show"
    pub name: Option<String>,   // e.g., "users.show"
    pub path_params: Vec<PathParam>,
}

/// Information about a handler function
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct HandlerInfo {
    pub name: String,
    pub has_handler_attr: bool,
    pub request_type: Option<String>,
}

/// A form request struct definition
#[derive(Debug, Clone)]
pub struct FormRequestStruct {
    pub name: String,
    pub fields: Vec<FormRequestField>,
}

#[derive(Debug, Clone)]
pub struct FormRequestField {
    /// The key the request is deserialized from: serde's `rename` and
    /// `rename_all` applied, not the Rust field name.
    pub name: String,
    pub ty: RustType,
}

/// Rust type representation for TypeScript conversion
#[derive(Debug, Clone)]
pub enum RustType {
    String,
    Number,
    Bool,
    Option(Box<RustType>),
    Vec(Box<RustType>),
    Custom(String),
}

/// A complete route ready for TypeScript generation
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct GeneratedRoute {
    pub definition: RouteDefinition,
    pub handler_info: Option<HandlerInfo>,
    pub request_struct: Option<FormRequestStruct>,
}

/// The prefixes the `group!`s around a route add to it.
///
/// Mirrors `GroupDef::register_with_inherited` in the framework: paths join
/// outside in, name prefixes concatenate outside in, and a
/// `controller = ...` path applies only to the bare handler names of its own
/// group, not to a group nested inside it.
#[derive(Clone, Default)]
struct GroupScope {
    /// The joined path of every enclosing group; `None` outside any group.
    path_prefix: Option<String>,
    name_prefix: String,
    controller: Option<String>,
}

/// Parse routes.rs file content and extract route definitions.
///
/// The file is read as Rust tokens, not with a pattern over its text, so a
/// route inside `group!` gets the group's path prefix and name prefix the
/// way the backend registers it. A route the scan cannot read (a closure
/// handler, a computed path) is skipped.
pub fn parse_routes_file(content: &str) -> Vec<RouteDefinition> {
    let Ok(tokens) = content.parse::<TokenStream>() else {
        return Vec::new();
    };
    let mut routes = Vec::new();
    collect_routes(tokens, &GroupScope::default(), &mut routes);
    routes
}

/// Walk `tokens`, collecting every route macro and descending into every
/// group and every other token group with the scope it adds.
fn collect_routes(tokens: TokenStream, scope: &GroupScope, routes: &mut Vec<RouteDefinition>) {
    let trees: Vec<TokenTree> = tokens.into_iter().collect();
    let mut at = 0;
    while at < trees.len() {
        if let Some((name, body)) = macro_call_at(&trees, at) {
            let (chain, next) = method_chain(&trees, at + 3);
            match name.as_str() {
                "get" | "post" | "put" | "patch" | "delete" => {
                    if let Some(route) = route_definition(&name, body, &chain, scope) {
                        routes.push(route);
                    }
                }
                "group" => {
                    if let Some((items, inner)) = group_scope(body, &chain, scope) {
                        collect_routes(items, &inner, routes);
                    }
                }
                _ => collect_routes(body, scope, routes),
            }
            at = next;
            continue;
        }
        if let TokenTree::Group(group) = &trees[at] {
            collect_routes(group.stream(), scope, routes);
        }
        at += 1;
    }
}

/// `name ! ( ... )` at `at`: the macro's name and the tokens inside it.
fn macro_call_at(trees: &[TokenTree], at: usize) -> Option<(String, TokenStream)> {
    let TokenTree::Ident(name) = trees.get(at)? else {
        return None;
    };
    let TokenTree::Punct(bang) = trees.get(at + 1)? else {
        return None;
    };
    let TokenTree::Group(body) = trees.get(at + 2)? else {
        return None;
    };
    (bang.as_char() == '!').then(|| (name.to_string(), body.stream()))
}

/// The `.method(args)` calls chained after the token at `at - 1`, and the
/// index of the first token after them.
fn method_chain(trees: &[TokenTree], mut at: usize) -> (Vec<(String, TokenStream)>, usize) {
    let mut chain = Vec::new();
    while let (
        Some(TokenTree::Punct(dot)),
        Some(TokenTree::Ident(method)),
        Some(TokenTree::Group(args)),
    ) = (trees.get(at), trees.get(at + 1), trees.get(at + 2))
    {
        if dot.as_char() != '.' || args.delimiter() != Delimiter::Parenthesis {
            break;
        }
        chain.push((method.to_string(), args.stream()));
        at += 3;
    }
    (chain, at)
}

/// The string literal a token stream holds, when it holds exactly one.
fn string_literal(tokens: TokenStream) -> Option<String> {
    syn::parse2::<syn::LitStr>(tokens)
        .ok()
        .map(|literal| literal.value())
}

/// The string the last `.<method>("...")` in `chain` sets.
fn chained_string(chain: &[(String, TokenStream)], method: &str) -> Option<String> {
    chain
        .iter()
        .rev()
        .find(|(name, _)| name == method)
        .and_then(|(_, args)| string_literal(args.clone()))
}

/// Split a macro body at its top-level commas.
fn comma_separated(tokens: TokenStream) -> Vec<Vec<TokenTree>> {
    let mut parts = vec![Vec::new()];
    for tree in tokens {
        match &tree {
            TokenTree::Punct(punct) if punct.as_char() == ',' => parts.push(Vec::new()),
            _ => {
                if let Some(part) = parts.last_mut() {
                    part.push(tree);
                }
            }
        }
    }
    if parts.last().is_some_and(Vec::is_empty) {
        parts.pop();
    }
    parts
}

/// A Rust path written as tokens (`controllers::user::show`), or `None` for
/// anything else, such as a closure.
fn path_text(tokens: &[TokenTree]) -> Option<String> {
    let mut text = String::new();
    for tree in tokens {
        match tree {
            TokenTree::Ident(ident) => text.push_str(&ident.to_string()),
            TokenTree::Punct(punct) if punct.as_char() == ':' => text.push(':'),
            _ => return None,
        }
    }
    let text = text.trim_start_matches("::").to_string();
    (!text.is_empty()).then_some(text)
}

fn route_definition(
    method: &str,
    body: TokenStream,
    chain: &[(String, TokenStream)],
    scope: &GroupScope,
) -> Option<RouteDefinition> {
    let method = HttpMethod::from_str(method)?;
    let mut parts = comma_separated(body).into_iter();
    let path = string_literal(parts.next()?.into_iter().collect())?;
    let handler = path_text(&parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }

    let (handler_module, handler_fn) = match handler.rsplit_once("::") {
        Some((module, function)) => (module.to_string(), function.to_string()),
        None => (scope.controller.clone()?, handler),
    };

    let joined = match &scope.path_prefix {
        Some(prefix) => join_paths(prefix, &path),
        None => path,
    };
    let path = convert_route_params(&joined);
    let name = chained_string(chain, "name").map(|name| format!("{}{name}", scope.name_prefix));
    let path_params = path_param_names(&path)
        .into_iter()
        .map(|(name, optional)| PathParam { name, optional })
        .collect();

    Some(RouteDefinition {
        method,
        path,
        handler_module,
        handler_fn,
        name,
        path_params,
    })
}

/// The items of a `group!` and the scope they register in.
fn group_scope(
    body: TokenStream,
    chain: &[(String, TokenStream)],
    scope: &GroupScope,
) -> Option<(TokenStream, GroupScope)> {
    let mut parts = comma_separated(body).into_iter();
    let prefix = string_literal(parts.next()?.into_iter().collect())?;
    let mut controller = None;
    let mut items = None;
    for part in parts {
        match part.as_slice() {
            [TokenTree::Group(group)] if group.delimiter() == Delimiter::Brace => {
                items = Some(group.stream());
            }
            [TokenTree::Ident(key), TokenTree::Punct(eq), rest @ ..]
                if key == "controller" && eq.as_char() == '=' =>
            {
                controller = Some(path_text(rest)?);
            }
            _ => return None,
        }
    }
    let path_prefix = join_paths(scope.path_prefix.as_deref().unwrap_or(""), &prefix);
    let name_prefix = format!(
        "{}{}",
        scope.name_prefix,
        chained_string(chain, "name").unwrap_or_default()
    );
    Some((
        items?,
        GroupScope {
            path_prefix: Some(path_prefix),
            name_prefix,
            controller,
        },
    ))
}

/// The framework's `join_paths`: one `/` between prefix and child, and `/`
/// for two empty halves.
fn join_paths(prefix: &str, child: &str) -> String {
    let prefix = prefix.trim_end_matches('/');
    let child = child.trim_start_matches('/');
    match (prefix.is_empty(), child.is_empty()) {
        (true, true) => "/".to_string(),
        (false, true) => prefix.to_string(),
        (_, false) => format!("{prefix}/{child}"),
    }
}

/// The framework's `convert_route_params`: a `:name` segment becomes
/// `{name}`. A colon inside a segment stays literal.
fn convert_route_params(path: &str) -> String {
    path.split('/')
        .map(|segment| match segment.strip_prefix(':') {
            Some(name) => format!("{{{name}}}"),
            None => segment.to_string(),
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// What one `{...}` placeholder of a route path is.
enum Placeholder<'a> {
    /// `{name}`: one path segment.
    Segment(&'a str),
    /// `{*name}`: the rest of the path, slashes included. The backend
    /// captures it under `name`, without the `*`.
    CatchAll(&'a str),
    /// `{name?}`: one path segment the route matches without.
    Optional(&'a str),
}

/// Read the text between `{` and `}`. Anything other than a plain, a
/// catch-all or an optional name is not a parameter the helpers fill in.
fn placeholder(inner: &str) -> Option<Placeholder<'_>> {
    let is_name =
        |name: &str| !name.is_empty() && name.chars().all(|ch| ch.is_alphanumeric() || ch == '_');
    if let Some(name) = inner.strip_suffix('?') {
        return is_name(name).then_some(Placeholder::Optional(name));
    }
    match inner.strip_prefix('*') {
        Some(name) if is_name(name) => Some(Placeholder::CatchAll(name)),
        None if is_name(inner) => Some(Placeholder::Segment(inner)),
        _ => None,
    }
}

/// Split `path` into literal text and placeholders, in order.
fn path_pieces(path: &str) -> Vec<Result<Placeholder<'_>, &str>> {
    let mut pieces = Vec::new();
    let mut rest = path;
    while let Some(open) = rest.find('{') {
        let Some(length) = rest[open..].find('}') else {
            break;
        };
        let close = open + length;
        if open > 0 {
            pieces.push(Err(&rest[..open]));
        }
        match placeholder(&rest[open + 1..close]) {
            Some(found) => pieces.push(Ok(found)),
            None => pieces.push(Err(&rest[open..=close])),
        }
        rest = &rest[close + 1..];
    }
    if !rest.is_empty() {
        pieces.push(Err(rest));
    }
    pieces
}

/// The parameter names a path's placeholders capture, in order, and
/// whether each is optional.
fn path_param_names(path: &str) -> Vec<(String, bool)> {
    path_pieces(path)
        .into_iter()
        .filter_map(|piece| match piece {
            Ok(Placeholder::Segment(name) | Placeholder::CatchAll(name)) => {
                Some((name.to_string(), false))
            }
            Ok(Placeholder::Optional(name)) => Some((name.to_string(), true)),
            Err(_) => None,
        })
        .collect()
}

/// Whether a path has an optional placeholder, whose URL the generated
/// `routeUrl` helper builds.
fn has_optional_placeholder(path: &str) -> bool {
    path_pieces(path)
        .iter()
        .any(|piece| matches!(piece, Ok(Placeholder::Optional(_))))
}

/// Visitor that collects handler functions with #[handler] attribute
struct HandlerVisitor {
    handlers: Vec<HandlerInfo>,
}

impl HandlerVisitor {
    fn new() -> Self {
        Self {
            handlers: Vec::new(),
        }
    }

    fn has_handler_attr(&self, attrs: &[Attribute]) -> bool {
        attrs.iter().any(|attr| attr.path().is_ident("handler"))
    }

    fn extract_request_type(&self, func: &ItemFn) -> Option<String> {
        // Get the first parameter's type
        if let Some(FnArg::Typed(pat_type)) = func.sig.inputs.first() {
            return self.type_to_string(&pat_type.ty);
        }
        None
    }

    fn type_to_string(&self, ty: &Type) -> Option<String> {
        match ty {
            Type::Path(type_path) => {
                let segments: Vec<String> = type_path
                    .path
                    .segments
                    .iter()
                    .map(|s| s.ident.to_string())
                    .collect();

                let type_name = segments.last()?.clone();

                // Skip if it's Request type (not a form request)
                if type_name == "Request" {
                    return None;
                }

                Some(type_name)
            }
            _ => None,
        }
    }
}

impl<'ast> Visit<'ast> for HandlerVisitor {
    fn visit_item_fn(&mut self, node: &'ast ItemFn) {
        let has_handler = self.has_handler_attr(&node.attrs);
        let request_type = if has_handler {
            self.extract_request_type(node)
        } else {
            None
        };

        self.handlers.push(HandlerInfo {
            name: node.sig.ident.to_string(),
            has_handler_attr: has_handler,
            request_type,
        });

        syn::visit::visit_item_fn(self, node);
    }
}

/// Visitor that collects the form request structs
struct FormRequestVisitor {
    structs: Vec<FormRequestStruct>,
}

impl FormRequestVisitor {
    fn new() -> Self {
        Self {
            structs: Vec::new(),
        }
    }

    /// Whether the attributes of a struct make it a form request.
    ///
    /// A form request is declared in one of three ways, and the scan reads
    /// the source, so it has to know each by its name: the attribute
    /// `#[request]`, the derive that the crate root exports as
    /// `FormRequestDerive` (`FormRequest` is the name of the trait there,
    /// and the name of the derive in the macro crate), and the helper
    /// attribute `#[form_request(..)]` that only the derive accepts.
    fn has_form_request_attr(&self, attrs: &[Attribute]) -> bool {
        attrs.iter().any(|attr| {
            let path = attr.path();
            path.is_ident("form_request") || names_framework_item(path, "request")
        }) || derive_list_names(attrs, "FormRequestDerive")
            || derive_list_names(attrs, "FormRequest")
    }

    fn parse_type(ty: &Type) -> RustType {
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
                        if let syn::PathArguments::AngleBracketed(args) = &segment.arguments
                            && let Some(syn::GenericArgument::Type(inner_ty)) = args.args.first()
                        {
                            return RustType::Option(Box::new(Self::parse_type(inner_ty)));
                        }
                        RustType::Option(Box::new(RustType::Custom("unknown".to_string())))
                    }
                    "Vec" => {
                        if let syn::PathArguments::AngleBracketed(args) = &segment.arguments
                            && let Some(syn::GenericArgument::Type(inner_ty)) = args.args.first()
                        {
                            return RustType::Vec(Box::new(Self::parse_type(inner_ty)));
                        }
                        RustType::Vec(Box::new(RustType::Custom("unknown".to_string())))
                    }
                    other => RustType::Custom(other.to_string()),
                }
            }
            Type::Reference(type_ref) => {
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
                Self::parse_type(&type_ref.elem)
            }
            _ => RustType::Custom("unknown".to_string()),
        }
    }
}

impl<'ast> Visit<'ast> for FormRequestVisitor {
    fn visit_item_struct(&mut self, node: &'ast ItemStruct) {
        if self.has_form_request_attr(&node.attrs) {
            let name = node.ident.to_string();

            // The interface describes what the request deserializes, so a
            // field carries the key serde reads it from, and a field serde
            // skips on input is left out.
            let fields = match &node.fields {
                Fields::Named(named) => named
                    .named
                    .iter()
                    .filter_map(|f| {
                        serde_input_key(&node.attrs, f).map(|name| FormRequestField {
                            name,
                            ty: Self::parse_type(&f.ty),
                        })
                    })
                    .collect(),
                _ => Vec::new(),
            };

            self.structs.push(FormRequestStruct { name, fields });
        }

        syn::visit::visit_item_struct(self, node);
    }
}

/// Scan a controller file for handler functions
fn scan_controller_handlers(content: &str) -> Vec<HandlerInfo> {
    if let Ok(syntax) = syn::parse_file(content) {
        let mut visitor = HandlerVisitor::new();
        visitor.visit_file(&syntax);
        return visitor.handlers;
    }
    Vec::new()
}

/// Scan all Rust files for FormRequest structs
fn scan_form_requests(project_path: &Path) -> HashMap<String, FormRequestStruct> {
    let src_path = project_path.join("src");
    let mut form_requests = HashMap::new();

    for entry in WalkDir::new(&src_path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|ext| ext == "rs").unwrap_or(false))
    {
        if let Ok(content) = fs::read_to_string(entry.path())
            && let Ok(syntax) = syn::parse_file(&content)
        {
            let mut visitor = FormRequestVisitor::new();
            visitor.visit_file(&syntax);
            for s in visitor.structs {
                form_requests.insert(s.name.clone(), s);
            }
        }
    }

    form_requests
}

/// Resolve handler module to file path
/// e.g., "controllers::user" -> "src/controllers/user.rs"
fn resolve_module_to_file(project_path: &Path, module_path: &str) -> Option<std::path::PathBuf> {
    let parts: Vec<&str> = module_path.split("::").collect();
    if parts.is_empty() {
        return None;
    }

    // Try as a file directly: src/controllers/user.rs
    let file_path = project_path
        .join("src")
        .join(parts.join("/"))
        .with_extension("rs");
    if file_path.exists() {
        return Some(file_path);
    }

    // Try as module folder: src/controllers/user/mod.rs
    let mod_path = project_path
        .join("src")
        .join(parts.join("/"))
        .join("mod.rs");
    if mod_path.exists() {
        return Some(mod_path);
    }

    None
}

/// Scan routes and handlers to build GeneratedRoute list
pub fn scan_routes(project_path: &Path) -> Result<Vec<GeneratedRoute>, String> {
    // Read routes.rs
    let routes_file = project_path.join("src/routes.rs");
    if !routes_file.exists() {
        return Err("src/routes.rs not found".to_string());
    }

    let routes_content =
        fs::read_to_string(&routes_file).map_err(|e| format!("Failed to read routes.rs: {}", e))?;

    let route_definitions = parse_routes_file(&routes_content);

    // Scan all form requests
    let form_requests = scan_form_requests(project_path);

    // Process each route
    let mut generated_routes = Vec::new();

    for def in route_definitions {
        // Try to find the handler
        let handler_info = if let Some(controller_file) =
            resolve_module_to_file(project_path, &def.handler_module)
        {
            if let Ok(content) = fs::read_to_string(&controller_file) {
                let handlers = scan_controller_handlers(&content);
                handlers.into_iter().find(|h| h.name == def.handler_fn)
            } else {
                None
            }
        } else {
            None
        };

        // Find the form request struct if the handler has one
        let request_struct = handler_info
            .as_ref()
            .and_then(|h| h.request_type.as_ref())
            .and_then(|type_name| form_requests.get(type_name).cloned());

        generated_routes.push(GeneratedRoute {
            definition: def,
            handler_info,
            request_struct,
        });
    }

    Ok(generated_routes)
}

/// Convert RustType to TypeScript type string
fn rust_type_to_ts(ty: &RustType) -> String {
    match ty {
        RustType::String => "string".to_string(),
        RustType::Number => "number".to_string(),
        RustType::Bool => "boolean".to_string(),
        RustType::Option(inner) => format!("{} | null", rust_type_to_ts(inner)),
        RustType::Vec(inner) => format!("{}[]", rust_type_to_ts(inner)),
        RustType::Custom(name) => name.clone(),
    }
}

/// Generate TypeScript routes file
pub fn generate_typescript(routes: &[GeneratedRoute]) -> String {
    let mut output = String::new();

    output.push_str("// This file is auto-generated by Suprnova. Do not edit manually.\n");
    output.push_str("// Run `suprnova generate-types` to regenerate.\n");
    output.push_str("// Compatible with Inertia.js v2+ UrlMethodPair interface\n\n");

    output.push_str("import type { Method } from '@inertiajs/core';\n\n");

    // RouteConfig interface
    output.push_str("// Route configuration - compatible with Inertia's UrlMethodPair\n");
    output.push_str("export interface RouteConfig<TData = void> {\n");
    output.push_str("  url: string;\n");
    output.push_str("  method: Method;  // 'get' | 'post' | 'put' | 'patch' | 'delete'\n");
    output.push_str("  data?: TData;\n");
    output.push_str("}\n\n");

    if routes
        .iter()
        .any(|route| has_optional_placeholder(&route.definition.path))
    {
        output.push_str(ROUTE_URL_HELPER);
    }

    // Collect all unique form request types
    let mut form_request_types: Vec<&FormRequestStruct> = routes
        .iter()
        .filter_map(|r| r.request_struct.as_ref())
        .collect();
    form_request_types.sort_by(|a, b| a.name.cmp(&b.name));
    form_request_types.dedup_by(|a, b| a.name == b.name);

    // Generate request type interfaces
    if !form_request_types.is_empty() {
        output.push_str("// Request types (from #[form_request] structs)\n");
        for form_req in &form_request_types {
            output.push_str(&format!("export interface {} {{\n", form_req.name));
            for field in &form_req.fields {
                let ts_type = rust_type_to_ts(&field.ty);
                output.push_str(&format!(
                    "  {}: {};\n",
                    ts_property_key(&field.name),
                    ts_type
                ));
            }
            output.push_str("}\n\n");
        }
    }

    // One helper key per route, shared by the controllers object, the
    // params interface and the named-routes lookup.
    let helper_keys = helper_keys(routes);

    // Collect all path param types
    let routes_with_params: Vec<(&GeneratedRoute, &String)> = routes
        .iter()
        .zip(&helper_keys)
        .filter(|(r, _)| !r.definition.path_params.is_empty())
        .collect();

    if !routes_with_params.is_empty() {
        output.push_str("// Path parameter types\n");
        for (route, key) in &routes_with_params {
            let interface_name = generate_params_interface_name(route, key);
            output.push_str(&format!("export interface {} {{\n", interface_name));
            for param in &route.definition.path_params {
                let marker = if param.optional { "?" } else { "" };
                output.push_str(&format!("  {}{marker}: string;\n", param.name));
            }
            output.push_str("}\n\n");
        }
    }

    // Group routes by module (first part of handler_module after "controllers::")
    let mut modules: HashMap<String, Vec<(&GeneratedRoute, &String)>> = HashMap::new();
    for (route, key) in routes.iter().zip(&helper_keys) {
        let module_name = extract_controller_name(&route.definition.handler_module);
        modules.entry(module_name).or_default().push((route, key));
    }

    // Generate controllers object
    output.push_str("// Controller namespace - mirrors backend structure\n");
    output.push_str("export const controllers = {\n");

    let mut module_names: Vec<&String> = modules.keys().collect();
    module_names.sort();

    for (i, module_name) in module_names.iter().enumerate() {
        let module_routes = modules.get(*module_name).unwrap();
        output.push_str(&format!("  {}: {{\n", module_name));

        for (j, (route, fn_name)) in module_routes.iter().enumerate() {
            let method = route.definition.method.to_ts_method();
            let has_params = !route.definition.path_params.is_empty();
            let has_data = route.request_struct.is_some();

            // Determine function signature
            let (params_signature, return_type) = if has_params && has_data {
                let params_type = generate_params_interface_name(route, fn_name);
                let data_type = route.request_struct.as_ref().unwrap().name.clone();
                (
                    format!("params: {}, data: {}", params_type, data_type),
                    format!("RouteConfig<{}>", data_type),
                )
            } else if has_params {
                let params_type = generate_params_interface_name(route, fn_name);
                // Every parameter optional: the helper can be called bare.
                let default = if route.definition.path_params.iter().all(|p| p.optional) {
                    " = {}"
                } else {
                    ""
                };
                (
                    format!("params: {params_type}{default}"),
                    "RouteConfig".to_string(),
                )
            } else if has_data {
                let data_type = route.request_struct.as_ref().unwrap().name.clone();
                (
                    format!("data: {}", data_type),
                    format!("RouteConfig<{}>", data_type),
                )
            } else {
                (String::new(), "RouteConfig".to_string())
            };

            // Generate URL with params interpolation
            let url = if has_optional_placeholder(&route.definition.path) {
                generate_route_url_call(&route.definition.path)
            } else if has_params {
                generate_url_with_params(&route.definition.path)
            } else {
                format!("'{}'", route.definition.path)
            };

            // Generate the function body
            let data_prop = if has_data { ", data" } else { "" };

            let comma = if j < module_routes.len() - 1 { "," } else { "" };
            output.push_str(&format!(
                "    {}: ({}): {} => ({{ url: {}, method: '{}'{} }}){}\n",
                fn_name, params_signature, return_type, url, method, data_prop, comma
            ));
        }

        let comma = if i < module_names.len() - 1 { "," } else { "" };
        output.push_str(&format!("  }}{}\n", comma));
    }

    output.push_str("} as const;\n\n");

    // Generate named routes lookup
    let named_routes: Vec<(&String, &GeneratedRoute, &String)> = routes
        .iter()
        .zip(&helper_keys)
        .filter_map(|(r, key)| r.definition.name.as_ref().map(|name| (name, r, key)))
        .collect();

    if !named_routes.is_empty() {
        output.push_str("// Named routes lookup\n");
        output.push_str("export const routes = {\n");

        for (i, (name, route, fn_name)) in named_routes.iter().enumerate() {
            let module = extract_controller_name(&route.definition.handler_module);
            let comma = if i < named_routes.len() - 1 { "," } else { "" };
            output.push_str(&format!(
                "  '{}': controllers.{}.{}{}\n",
                name, module, fn_name, comma
            ));
        }

        output.push_str("} as const;\n");
    }

    output
}

/// The key each route's helper gets in its module of the `controllers`
/// object, in the order of `routes`.
///
/// The first route of a handler keeps the handler's name. A later route of
/// the same handler in the same module gets a key from its route name or its
/// path, so two routes never share one helper. The named-routes lookup and
/// the params interface use these keys too, so an alias reaches its own
/// route rather than the first route of its handler.
fn helper_keys(routes: &[GeneratedRoute]) -> Vec<String> {
    let mut used_names: HashMap<(String, String), usize> = HashMap::new();
    routes
        .iter()
        .map(|route| {
            let module = extract_controller_name(&route.definition.handler_module);
            let base_fn_name = &route.definition.handler_fn;
            let seen = used_names
                .entry((module, base_fn_name.clone()))
                .or_insert(0);
            let key = if *seen == 0 {
                base_fn_name.clone()
            } else {
                // Duplicate handler in this module - derive a unique key from the
                // route name or path, then sanitize it to a valid TS identifier.
                let raw = if let Some(name) = &route.definition.name {
                    // Use the last part of the route name: "home" from "home", "protected" from name
                    name.split('.')
                        .next_back()
                        .unwrap_or(base_fn_name)
                        .to_string()
                } else {
                    // Use path to create unique name
                    route.definition.path.trim_start_matches('/').to_string()
                };
                let key = sanitize_route_key(&raw);
                if key.is_empty() {
                    format!("{}_{}", base_fn_name, *seen + 1)
                } else {
                    key
                }
            };
            *seen += 1;
            key
        })
        .collect()
}

/// Generate params interface name from route
fn generate_params_interface_name(route: &GeneratedRoute, helper_key: &str) -> String {
    // Convert handler to PascalCase: user::show -> UserShowParams. A second
    // route of the same handler is named after its own helper key, so the
    // two interfaces do not merge into one that requires both routes' params.
    let module = extract_controller_name(&route.definition.handler_module);
    format!(
        "{}{}Params",
        to_pascal_case(&module),
        to_pascal_case(helper_key)
    )
}

/// Sanitize an arbitrary string (a route path or route-name segment) into a
/// valid, unquoted TypeScript identifier for use as an object-property key.
///
/// Every character that isn't ASCII-alphanumeric or `_` becomes `_`, and a
/// leading digit is prefixed with `_`. Without this, a file extension
/// (`/favicon-16x16.png` -> `favicon_16x16.png`) or a leading digit
/// (`/2fa` -> `2fa`) would leak into the generated `controllers` object as an
/// illegal identifier, producing TypeScript that fails `tsc`/`svelte-check`.
fn sanitize_route_key(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

/// Convert snake_case to PascalCase
fn to_pascal_case(s: &str) -> String {
    s.split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().chain(chars).collect(),
            }
        })
        .collect()
}

/// Extract controller name from module path
/// e.g., "controllers::user" -> "user"
fn extract_controller_name(module_path: &str) -> String {
    module_path
        .split("::")
        .last()
        .unwrap_or("unknown")
        .to_string()
}

/// Generate the template literal that builds a route's URL from `params`.
///
/// Each value is percent-encoded the way the backend's `route()` helper
/// encodes it, so a slug of `a/b` reaches the handler as `a/b` instead of
/// adding a path segment, and `?` or `#` in a value cannot start a query or
/// a fragment. A catch-all value spans segments: each segment is encoded and
/// the slashes between them stay.
fn generate_url_with_params(path: &str) -> String {
    let mut template = String::from("`");
    for piece in path_pieces(path) {
        match piece {
            Ok(Placeholder::Segment(name)) => {
                template.push_str(&format!("${{encodeURIComponent(String(params.{name}))}}"));
            }
            Ok(Placeholder::CatchAll(name)) => template.push_str(&format!(
                "${{String(params.{name}).split('/').map(encodeURIComponent).join('/')}}"
            )),
            // A path with an optional placeholder goes through
            // `generate_route_url_call`; kept literal here for completeness.
            Ok(Placeholder::Optional(name)) => template.push_str(&format!("{{{name}?}}")),
            Err(text) => {
                for ch in text.chars() {
                    if matches!(ch, '`' | '\\' | '$') {
                        template.push('\\');
                    }
                    template.push(ch);
                }
            }
        }
    }
    template.push('`');
    template
}

/// The TypeScript helper that builds a URL with optional placeholders the
/// way the backend's `route()` fills them: a value is encoded as one segment
/// (a catch-all keeps its slashes), an optional parameter with no value is
/// left out with its segment, and one with no value before a later given
/// value stays as its placeholder, since leaving it out would move the later
/// value into its place. An empty string is no value.
const ROUTE_URL_HELPER: &str = r#"// Builds a URL the way the backend's route() does: each value is encoded as
// one segment (a catch-all keeps its slashes), an optional parameter without
// a value is left out with its segment, and one without a value before a
// later given value stays as its placeholder.
type UrlPart = string | { name: string; value: unknown; optional?: boolean; catchAll?: boolean };
function routeUrl(parts: UrlPart[]): string {
  const given = (value: unknown): boolean =>
    value !== undefined && value !== null && String(value) !== '';
  let lastGiven = -1;
  parts.forEach((part, index) => {
    if (typeof part !== 'string' && part.optional && given(part.value)) lastGiven = index;
  });
  let url = '';
  parts.forEach((part, index) => {
    if (typeof part === 'string') {
      url += part;
    } else if (!part.optional || given(part.value)) {
      const text = String(part.value);
      url += part.catchAll ? text.split('/').map(encodeURIComponent).join('/') : encodeURIComponent(text);
    } else if (index < lastGiven) {
      url += `{${part.name}}`;
    } else if (url.length > 1 && url.endsWith('/')) {
      url = url.slice(0, -1);
    }
  });
  return url;
}

"#;

/// Generate a `routeUrl([...])` call for a path with an optional
/// placeholder. Literal text is a JSON string, which is a valid TypeScript
/// string literal.
fn generate_route_url_call(path: &str) -> String {
    let quote = |text: &str| serde_json::Value::String(text.to_owned()).to_string();
    let parts: Vec<String> = path_pieces(path)
        .into_iter()
        .map(|piece| match piece {
            Ok(Placeholder::Segment(name)) => {
                format!("{{ name: {}, value: params.{name} }}", quote(name))
            }
            Ok(Placeholder::CatchAll(name)) => format!(
                "{{ name: {}, value: params.{name}, catchAll: true }}",
                quote(name)
            ),
            Ok(Placeholder::Optional(name)) => format!(
                "{{ name: {}, value: params.{name}, optional: true }}",
                quote(name)
            ),
            Err(text) => quote(text),
        })
        .collect();
    format!("routeUrl([{}])", parts.join(", "))
}

/// Generate routes and write to the output file
pub fn generate_routes_to_file(project_path: &Path, output_path: &Path) -> Result<usize, String> {
    let routes = scan_routes(project_path)?;

    if routes.is_empty() {
        return Ok(0);
    }

    // Ensure output directory exists
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create output directory: {}", e))?;
    }

    let typescript = generate_typescript(&routes);
    fs::write(output_path, typescript)
        .map_err(|e| format!("Failed to write TypeScript file: {}", e))?;

    Ok(routes.len())
}

/// Main entry point for route generation (standalone use)
#[allow(dead_code)]
pub fn run(output: Option<String>) {
    let project_path = Path::new(".");

    // Validate Suprnova project
    let cargo_toml = project_path.join("Cargo.toml");
    if !cargo_toml.exists() {
        ui::error("Not a Suprnova project (no Cargo.toml found)");
        std::process::exit(1);
    }

    let output_path = output
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| project_path.join("frontend/src/types/routes.ts"));

    ui::info("Scanning routes for type-safe generation...");

    match generate_routes_to_file(project_path, &output_path) {
        Ok(0) => {
            ui::warning("No routes found in src/routes.rs");
        }
        Ok(count) => {
            ui::info(&format!("Found {} route(s)", count));
            ui::success(&format!("Generated {}", output_path.display()));
        }
        Err(e) => {
            ui::error(&e);
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod form_request_scan_tests {
    use super::*;

    fn scan(source: &str) -> HashMap<String, FormRequestStruct> {
        let dir = tempfile::tempdir().expect("tempdir");
        let src = dir.path().join("src");
        fs::create_dir_all(&src).expect("mkdir src");
        fs::write(src.join("requests.rs"), source).expect("write source");
        scan_form_requests(dir.path())
    }

    #[test]
    fn scan_sees_a_bare_derive() {
        let found = scan("#[derive(FormRequest)] pub struct Store { pub title: String }");
        assert!(found.contains_key("Store"));
    }

    #[test]
    fn scan_sees_a_qualified_derive() {
        let found = scan("#[derive(suprnova::FormRequest)] pub struct Store { pub title: String }");
        let store = found.get("Store").expect("qualified derive recognised");
        assert_eq!(store.fields.len(), 1);
    }

    #[test]
    fn scan_sees_the_attribute_form() {
        let found = scan("#[form_request] pub struct Store { pub title: String }");
        assert!(found.contains_key("Store"));
    }

    /// The names an application writes: the derive as the crate root exports
    /// it, and the attribute macro.
    #[test]
    fn scan_sees_the_names_of_the_crate_root() {
        for declaration in [
            "#[derive(FormRequestDerive)]",
            "#[derive(Deserialize, Validate, suprnova::FormRequestDerive)]",
            "#[request]",
            "#[suprnova::request]",
        ] {
            let found = scan(&format!(
                "{declaration} pub struct Store {{ pub title: String }}"
            ));
            assert!(found.contains_key("Store"), "{declaration}");
        }
    }

    #[test]
    fn scan_skips_the_attribute_of_another_crate() {
        let found = scan("#[other::request] pub struct Store { pub title: String }");
        assert!(found.is_empty());
    }

    #[test]
    fn scan_skips_a_struct_with_another_crates_derive() {
        let found = scan("#[derive(other::FormRequest)] pub struct Store { pub title: String }");
        assert!(found.is_empty());
    }
}

#[cfg(test)]
mod route_contract_tests {
    use super::*;

    fn generated(definitions: Vec<RouteDefinition>) -> Vec<GeneratedRoute> {
        definitions
            .into_iter()
            .map(|definition| GeneratedRoute {
                definition,
                handler_info: None,
                request_struct: None,
            })
            .collect()
    }

    /// `(method, path, name, module, handler)` for each parsed route.
    fn summary(definitions: &[RouteDefinition]) -> Vec<(String, String, Option<String>, String)> {
        definitions
            .iter()
            .map(|route| {
                (
                    route.method.to_ts_method().to_string(),
                    route.path.clone(),
                    route.name.clone(),
                    format!("{}::{}", route.handler_module, route.handler_fn),
                )
            })
            .collect()
    }

    #[test]
    fn group_prefixes_and_name_prefixes_compose_like_the_backend() {
        let definitions = parse_routes_file(
            r#"
routes! {
    get!("/", controllers::home::index).name("home"),
    group!("/users", {
        get!("/{id}", controllers::user::show).name("show"),
        group!("/{user}/posts", {
            get!("/:post", controllers::post::show).name("show"),
        }).name("posts."),
    }).middleware(AuthMiddleware::new()).name("users."),
    group!("/teams", controller = controllers::team, {
        post!("/", store).name("teams.store"),
    }),
}
"#,
        );

        assert_eq!(
            summary(&definitions),
            vec![
                (
                    "get".to_string(),
                    "/".to_string(),
                    Some("home".to_string()),
                    "controllers::home::index".to_string(),
                ),
                (
                    "get".to_string(),
                    "/users/{id}".to_string(),
                    Some("users.show".to_string()),
                    "controllers::user::show".to_string(),
                ),
                (
                    "get".to_string(),
                    "/users/{user}/posts/{post}".to_string(),
                    Some("users.posts.show".to_string()),
                    "controllers::post::show".to_string(),
                ),
                (
                    "post".to_string(),
                    "/teams".to_string(),
                    Some("teams.store".to_string()),
                    "controllers::team::store".to_string(),
                ),
            ]
        );
        let params: Vec<&str> = definitions[2]
            .path_params
            .iter()
            .map(|param| param.name.as_str())
            .collect();
        assert_eq!(params, ["user", "post"]);

        let ts = generate_typescript(&generated(definitions));
        assert!(
            ts.contains("url: `/users/${encodeURIComponent(String(params.id))}`"),
            "the helper must target the grouped path; got:\n{ts}"
        );
    }

    #[test]
    fn a_named_alias_of_a_repeated_handler_points_at_its_own_helper() {
        let definitions = parse_routes_file(
            r#"
routes! {
    get!("/first", controllers::home::index).name("home.first"),
    post!("/second", controllers::home::index).name("home.second"),
}
"#,
        );
        let ts = generate_typescript(&generated(definitions));

        assert!(
            ts.contains("second: (): RouteConfig => ({ url: '/second', method: 'post' })"),
            "the second route gets its own helper; got:\n{ts}"
        );
        assert!(
            ts.contains("'home.first': controllers.home.index"),
            "got:\n{ts}"
        );
        assert!(
            ts.contains("'home.second': controllers.home.second"),
            "the second alias must reference the second helper, not the first; got:\n{ts}"
        );
    }

    #[test]
    fn path_parameters_are_percent_encoded_like_the_backend_route_helper() {
        // `route()` encodes a value as one path segment, so `a/b` reaches
        // the handler as `a/b`. Interpolating it raw would change the path,
        // the query or the fragment instead.
        assert_eq!(
            generate_url_with_params("/posts/{slug}/edit"),
            "`/posts/${encodeURIComponent(String(params.slug))}/edit`"
        );
        // A catch-all spans segments: each segment is encoded and the
        // slashes between them stay.
        assert_eq!(
            generate_url_with_params("/files/{*rest}"),
            "`/files/${String(params.rest).split('/').map(encodeURIComponent).join('/')}`"
        );
        let definitions = parse_routes_file(r#"get!("/files/{*rest}", controllers::file::show)"#);
        let params: Vec<&str> = definitions[0]
            .path_params
            .iter()
            .map(|param| param.name.as_str())
            .collect();
        assert_eq!(params, ["rest"]);
    }

    #[test]
    fn optional_placeholders_follow_the_backend_route_helper() {
        let definitions = parse_routes_file(
            r#"
routes! {
    get!("/archive/{year?}/{month?}", controllers::archive::show).name("archive"),
}
"#,
        );
        let params: Vec<(&str, bool)> = definitions[0]
            .path_params
            .iter()
            .map(|param| (param.name.as_str(), param.optional))
            .collect();
        assert_eq!(params, [("year", true), ("month", true)]);

        let ts = generate_typescript(&generated(definitions));
        assert!(
            ts.contains("  year?: string;\n  month?: string;\n"),
            "optional parameters are optional keys; got:\n{ts}"
        );
        assert!(
            ts.contains("show: (params: ArchiveShowParams = {}): RouteConfig =>"),
            "a route whose parameters are all optional can be called without them; got:\n{ts}"
        );
        assert!(
            ts.contains(
                r#"url: routeUrl(["/archive/", { name: "year", value: params.year, optional: true }, "/", { name: "month", value: params.month, optional: true }])"#
            ),
            "the URL is built by the shared helper; got:\n{ts}"
        );
        assert!(
            ts.contains("function routeUrl(parts: UrlPart[]): string {"),
            "the helper is emitted once; got:\n{ts}"
        );
    }

    #[test]
    fn routes_without_optional_placeholders_emit_no_url_helper() {
        let definitions = parse_routes_file(r#"get!("/posts/{slug}", controllers::post::show)"#);
        let ts = generate_typescript(&generated(definitions));
        assert!(!ts.contains("routeUrl"), "got:\n{ts}");
    }

    #[test]
    fn request_interfaces_use_the_names_serde_deserializes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src = dir.path().join("src");
        fs::create_dir_all(&src).expect("mkdir src");
        fs::write(
            src.join("requests.rs"),
            r#"
#[derive(Deserialize, FormRequest)]
#[serde(rename_all = "camelCase")]
pub struct Store {
    pub display_name: String,
    #[serde(rename = "e-mail")]
    pub email: String,
    #[serde(rename(serialize = "out", deserialize = "given"))]
    pub both: String,
    #[serde(skip_deserializing)]
    pub server_only: String,
    #[serde(skip)]
    pub hidden: String,
    pub r#type: String,
}
"#,
        )
        .expect("write source");
        let store = scan_form_requests(dir.path())
            .remove("Store")
            .expect("Store is a form request");
        let mut route = generated(parse_routes_file(
            r#"post!("/store", controllers::thing::store)"#,
        ));
        route[0].request_struct = Some(store);
        let ts = generate_typescript(&route);

        let interface = ts
            .split("export interface Store {")
            .nth(1)
            .and_then(|rest| rest.split('}').next())
            .expect("Store interface emitted");
        let keys: Vec<&str> = interface
            .lines()
            .filter_map(|line| line.trim().split_once(':').map(|(key, _)| key))
            .collect();
        assert_eq!(keys, ["displayName", "\"e-mail\"", "given", "type"]);
    }
}
