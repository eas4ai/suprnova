//! Attributes, derives and macros (REG-030). Each one is admitted by name
//! only after its path is resolved, so an import that renames another
//! macro to an admitted name is refused, and every path an admitted
//! attribute carries in its arguments is classified like any other path.

use syn::parse::{ParseStream, Parser};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{Token, parenthesized};

use super::modules::{join, segments, unraw};
use super::ty::Ty;
use super::walk::{Class, Ns, Res, Site, Walker};
use crate::registry::Capability;

/// The derives a component may use from `std`, by name.
const STD_DERIVES: &[&str] = &[
    "Debug",
    "Clone",
    "Copy",
    "PartialEq",
    "Eq",
    "PartialOrd",
    "Ord",
    "Hash",
    "Default",
];

/// The `std` macros a component may invoke. None of them reads anything
/// at build time: their arguments are parsed and scanned like code.
pub(crate) const ADMITTED_MACROS: &[&str] = &[
    "format",
    "format_args",
    "write",
    "writeln",
    "vec",
    "matches",
    "assert",
    "assert_eq",
    "assert_ne",
    "debug_assert",
    "debug_assert_eq",
    "debug_assert_ne",
    "panic",
    "unreachable",
    "todo",
    "unimplemented",
    "concat",
    "stringify",
    "line",
    "column",
    "file",
    "module_path",
    "cfg",
    "compile_error",
];

/// The attributes a component may write that are not Live helpers: lint
/// levels, documentation, layout and conditional compilation flags. None of
/// them runs code.
pub(crate) const ADMITTED_ATTRIBUTES: &[&str] = &[
    "allow",
    "warn",
    "deny",
    "forbid",
    "expect",
    "must_use",
    "inline",
    "cold",
    "non_exhaustive",
    "repr",
    "track_caller",
    "cfg",
];

/// The `#[derive(LiveComponent)]` helpers on a field.
pub(crate) const FIELD_HELPERS: &[&str] = &[
    "public",
    "model",
    "locked",
    "server_only",
    "session",
    "secret",
    "transient",
    "url",
    "upload",
    "validate",
];

/// The `#[live]` helpers on a method of a `#[live]` impl.
pub(crate) const METHOD_HELPERS: &[&str] = &[
    "mount",
    "action",
    "computed",
    "validate",
    "hydrate",
    "rendering",
    "rendered",
    "dehydrate",
    "teardown",
    "params_changed",
    "lazy_complete",
];

/// Keys whose string value names a function or module path.
const PATH_STRING_KEYS: &[&str] = &[
    "function",
    "with",
    "serialize_with",
    "deserialize_with",
    "skip_serializing_if",
    "default",
    "path",
];

/// Keys whose string value names a type.
const TYPE_STRING_KEYS: &[&str] = &["from", "try_from", "into"];

/// The keys each argument-taking helper admits.
const LIVE_KEYS: &[&str] = &[
    "name",
    "view",
    "component_version",
    "state_schema_version",
    "action_schema_version",
    "checker_contract_version",
    "minimum_protocol_version",
    "refresh_on_promote",
    "events",
    "effects",
    "streams",
    "stream",
    "topics",
    "targets",
    "fanout",
    "modes",
    "reconnect",
    "resume_attempts",
];
const MODEL_KEYS: &[&str] = &[
    "immediate",
    "change",
    "blur",
    "submit",
    "transient",
    "debounce",
];
const URL_KEYS: &[&str] = &["key", "mode", "omit_default"];
const UPLOAD_KEYS: &[&str] = &["policy"];
const ACTION_KEYS: &[&str] = &["name", "version", "authorize", "validate", "transaction"];
const VALIDATE_METHOD_KEYS: &[&str] = &["action"];
const VALIDATE_FIELD_KEYS: &[&str] = &[
    "email",
    "url",
    "length",
    "range",
    "must_match",
    "contains",
    "does_not_contain",
    "custom",
    "regex",
    "credit_card",
    "non_control_character",
    "required",
    "nested",
    "ip",
    "ipv4",
    "ipv6",
    "min",
    "max",
    "equal",
    "exclusive_min",
    "exclusive_max",
    "code",
    "message",
    "other",
    "pattern",
    "function",
    "use_context",
    "path",
    "needle",
];
const SERDE_KEYS: &[&str] = &[
    "rename",
    "rename_all",
    "rename_all_fields",
    "serialize",
    "deserialize",
    "alias",
    "skip",
    "skip_serializing",
    "skip_deserializing",
    "default",
    "flatten",
    "deny_unknown_fields",
    "tag",
    "content",
    "untagged",
    "transparent",
    "borrow",
    "other",
    "skip_serializing_if",
    "serialize_with",
    "deserialize_with",
    "with",
    "from",
    "try_from",
    "into",
    "crate",
    "expecting",
    "variant_identifier",
    "field_identifier",
];

/// One argument of an attribute, in the `key`, `key = value`, `key(...)`
/// and literal forms Live, serde and validator helpers use.
#[derive(Clone)]
enum ArgItem {
    Flag(syn::Path),
    Value(syn::Path, Box<syn::Expr>),
    List(syn::Path, Vec<ArgItem>),
    Lit,
}

fn parse_args(input: ParseStream<'_>) -> syn::Result<Vec<ArgItem>> {
    let mut items = Vec::new();
    while !input.is_empty() {
        if input.peek(syn::Lit) {
            input.parse::<syn::Lit>()?;
            items.push(ArgItem::Lit);
        } else {
            let path = input.call(syn::Path::parse_mod_style)?;
            if input.peek(Token![=]) {
                input.parse::<Token![=]>()?;
                items.push(ArgItem::Value(path, Box::new(input.parse()?)));
            } else if input.peek(syn::token::Paren) {
                let content;
                parenthesized!(content in input);
                items.push(ArgItem::List(path, parse_args(&content)?));
            } else {
                items.push(ArgItem::Flag(path));
            }
        }
        if input.is_empty() {
            break;
        }
        input.parse::<Token![,]>()?;
    }
    Ok(items)
}

fn key_of(path: &syn::Path) -> String {
    join(&segments(path))
}

impl Walker<'_> {
    /// Whether a name is bound by an import or definition in scope, so a
    /// bare attribute or macro name means what that binding says.
    fn name_bound(&mut self, name: &str) -> bool {
        let written = vec![name.to_string()];
        self.silent += 1;
        let res = self.resolve_segments(&written, false, Ns::Type);
        self.silent -= 1;
        match res {
            Res::Full(full) => {
                !(full.len() == 1 && full[0] == name) && !is_prelude_path(&full, name)
            }
            _ => false,
        }
    }

    fn resolve_silently(&mut self, path: &syn::Path) -> (Res, Class) {
        self.silent += 1;
        let res = self.resolve(path, Ns::Type);
        let class = self.classify_res(&res, path.span());
        self.silent -= 1;
        (res, class)
    }

    /// Whether an attribute is the `#[live]` attribute macro.
    pub(super) fn is_live_attribute(&mut self, attr: &syn::Attribute) -> bool {
        let path = attr.path();
        let written = segments(path);
        if written.len() == 1
            && path.leading_colon.is_none()
            && written[0] == "live"
            && !self.name_bound("live")
        {
            return true;
        }
        matches!(self.resolve_silently(path), (_, Class::Api { ref path, .. }) if path == "suprnova::live")
    }

    /// Checks every attribute on an item, field, statement or expression;
    /// returns whether the attributes derive serde's traits.
    pub(super) fn check_attrs(&mut self, attrs: &[syn::Attribute], site: Site) -> bool {
        let mut serde = false;
        for attr in attrs {
            if let Some(derived) = self.check_attr(attr, site) {
                serde |= derived;
            }
        }
        serde
    }

    fn check_attr(&mut self, attr: &syn::Attribute, site: Site) -> Option<bool> {
        let path = attr.path();
        let written = segments(path);
        let name = written.last().cloned().unwrap_or_default();
        let bare = written.len() == 1 && path.leading_colon.is_none() && !self.name_bound(&name);
        if !bare {
            if self.is_live_attribute(attr) {
                self.check_live_attr(attr, site);
                return None;
            }
            self.refuse(
                "rust-attribute",
                attr.span(),
                format!(
                    "`#[{}]` invokes a macro that is not admitted",
                    join(&written)
                ),
            );
            return None;
        }
        match name.as_str() {
            "doc" => self.check_doc(attr),
            "allow" | "warn" | "deny" | "forbid" | "expect" | "repr" | "cfg" => {
                if !matches!(attr.meta, syn::Meta::List(_)) {
                    self.refuse(
                        "rust-attribute",
                        attr.span(),
                        format!("a malformed `#[{name}]`"),
                    );
                }
            }
            other if ADMITTED_ATTRIBUTES.contains(&other) => {
                if let syn::Meta::NameValue(value) = &attr.meta
                    && !matches!(value.value, syn::Expr::Lit(_))
                {
                    self.refuse(
                        "rust-attribute",
                        attr.span(),
                        format!("`#[{name}]` with a computed value"),
                    );
                }
            }
            "derive" => return Some(self.check_derive(attr)),
            "live" => self.check_live_attr(attr, site),
            "serde"
                if self.serde_derive.last().copied().unwrap_or(false) || site == Site::Struct =>
            {
                self.check_helper_args(attr, SERDE_KEYS, "serde");
            }
            helper if site == Site::Field && FIELD_HELPERS.contains(&helper) => {
                self.check_field_helper(attr, helper);
            }
            helper if site == Site::LiveMethod && METHOD_HELPERS.contains(&helper) => {
                self.check_method_helper(attr, helper);
            }
            _ => self.refuse(
                "rust-attribute",
                attr.span(),
                format!("`#[{name}]` is not an admitted attribute here"),
            ),
        }
        None
    }

    fn check_doc(&mut self, attr: &syn::Attribute) {
        match &attr.meta {
            syn::Meta::NameValue(value) => {
                if !matches!(&value.value, syn::Expr::Lit(lit) if matches!(lit.lit, syn::Lit::Str(_)))
                {
                    self.refuse(
                        "rust-attribute",
                        attr.span(),
                        "`#[doc = ...]` with a value that is not a string literal".to_string(),
                    );
                }
            }
            syn::Meta::List(_) => {
                let parsed = attr.parse_args_with(parse_args);
                let admitted = parsed.as_ref().is_ok_and(|items| {
                    items.iter().all(|item| match item {
                        ArgItem::Flag(path) => {
                            matches!(key_of(path).as_str(), "hidden" | "inline" | "no_inline")
                        }
                        ArgItem::Value(path, value) => {
                            key_of(path) == "alias" && matches!(**value, syn::Expr::Lit(_))
                        }
                        ArgItem::List(path, inner) => {
                            key_of(path) == "alias"
                                && inner.iter().all(|item| matches!(item, ArgItem::Lit))
                        }
                        _ => false,
                    })
                });
                if !admitted {
                    self.refuse(
                        "rust-attribute",
                        attr.span(),
                        "a `#[doc(...)]` the scan cannot read".to_string(),
                    );
                }
            }
            syn::Meta::Path(_) => {
                self.refuse("rust-attribute", attr.span(), "a bare `#[doc]`".to_string());
            }
        }
    }

    fn check_derive(&mut self, attr: &syn::Attribute) -> bool {
        let parsed = attr.parse_args_with(Punctuated::<syn::Path, Token![,]>::parse_terminated);
        let Ok(paths) = parsed else {
            self.refuse(
                "rust-derive",
                attr.span(),
                "a `#[derive(...)]` the scan cannot read".to_string(),
            );
            return false;
        };
        let mut serde = false;
        for path in &paths {
            let written = segments(path);
            let name = written.last().cloned().unwrap_or_default();
            let bare =
                written.len() == 1 && path.leading_colon.is_none() && !self.name_bound(&name);
            if bare {
                if !STD_DERIVES.contains(&name.as_str()) {
                    self.refuse(
                        "rust-derive",
                        path.span(),
                        format!("`#[derive({name})]` is not an admitted derive; import Suprnova's derives by name"),
                    );
                }
                continue;
            }
            let (res, class) = self.resolve_silently(path);
            let admitted = match (&res, &class) {
                (_, Class::Api { path, .. }) if path == "suprnova::LiveComponent" => true,
                (_, Class::Api { path, prefix: true })
                    if path.starts_with("suprnova::serde::")
                        && (path.ends_with("::Serialize") || path.ends_with("::Deserialize")) =>
                {
                    serde = true;
                    true
                }
                (Res::Full(full), Class::Std(_) | Class::StdModule) => {
                    STD_DERIVES.contains(&full.last().map(String::as_str).unwrap_or_default())
                }
                (Res::Full(full), Class::Refused)
                    if matches!(
                        full.first().map(String::as_str),
                        Some("std" | "core" | "alloc")
                    ) =>
                {
                    STD_DERIVES.contains(&full.last().map(String::as_str).unwrap_or_default())
                }
                _ => false,
            };
            if !admitted {
                self.refuse(
                    "rust-derive",
                    path.span(),
                    format!("`#[derive({})]` is not an admitted derive", join(&written)),
                );
            }
        }
        serde
    }

    fn check_live_attr(&mut self, attr: &syn::Attribute, site: Site) {
        match (site, &attr.meta) {
            (Site::Struct, syn::Meta::List(_)) => self.check_helper_args(attr, LIVE_KEYS, "live"),
            (Site::Impl, syn::Meta::Path(_)) => {}
            _ => self.refuse(
                "rust-attribute",
                attr.span(),
                "`#[live]` belongs on a component struct (with its arguments) or its impl block"
                    .to_string(),
            ),
        }
    }

    fn check_field_helper(&mut self, attr: &syn::Attribute, helper: &str) {
        match helper {
            "session" => self.grant_capability(Capability::Session),
            "upload" => self.grant_capability(Capability::Files),
            _ => {}
        }
        match (helper, &attr.meta) {
            (
                "public" | "locked" | "server_only" | "session" | "secret" | "transient",
                syn::Meta::Path(_),
            ) => {}
            ("model" | "url" | "upload", syn::Meta::Path(_)) => {}
            ("model", syn::Meta::List(_)) => self.check_helper_args(attr, MODEL_KEYS, helper),
            ("url", syn::Meta::List(_)) => self.check_helper_args(attr, URL_KEYS, helper),
            ("upload", syn::Meta::List(_)) => self.check_helper_args(attr, UPLOAD_KEYS, helper),
            ("validate", syn::Meta::List(_)) => {
                self.check_helper_args(attr, VALIDATE_FIELD_KEYS, helper)
            }
            _ => self.refuse(
                "rust-attribute",
                attr.span(),
                format!("a malformed `#[{helper}]`"),
            ),
        }
    }

    fn check_method_helper(&mut self, attr: &syn::Attribute, helper: &str) {
        match (helper, &attr.meta) {
            ("action", syn::Meta::List(_)) => {
                self.check_helper_args(attr, ACTION_KEYS, helper);
                if let Ok(items) = attr.parse_args_with(parse_args) {
                    let transaction = items.iter().any(|item| match item {
                        ArgItem::Value(path, value) if key_of(path) == "transaction" => matches!(
                            &**value,
                            syn::Expr::Lit(lit)
                                if matches!(&lit.lit, syn::Lit::Str(text) if text.value() == "required")
                        ),
                        _ => false,
                    });
                    if transaction {
                        self.grant_capability(Capability::Database);
                    }
                }
            }
            ("validate", syn::Meta::List(_)) => {
                self.check_helper_args(attr, VALIDATE_METHOD_KEYS, helper)
            }
            (_, syn::Meta::Path(_)) => {}
            _ => self.refuse(
                "rust-attribute",
                attr.span(),
                format!("a malformed `#[{helper}]`"),
            ),
        }
    }

    fn grant_capability(&mut self, capability: Capability) {
        if self.silent == 0 {
            self.capabilities.insert(capability);
        }
    }

    /// Checks an argument list against the keys a helper admits and
    /// classifies every path it carries.
    fn check_helper_args(&mut self, attr: &syn::Attribute, keys: &[&str], helper: &str) {
        match attr.parse_args_with(parse_args) {
            Ok(items) => {
                if helper == "live"
                    && items.iter().any(
                        |item| matches!(item, ArgItem::List(path, _) if key_of(path) == "streams"),
                    )
                {
                    self.grant_capability(Capability::Network);
                }
                self.check_items(&items, keys, helper, None);
            }
            Err(error) => self.refuse(
                "rust-attribute",
                error.span(),
                format!("`#[{helper}(...)]` arguments the scan cannot read: {error}"),
            ),
        }
    }

    fn check_items(
        &mut self,
        items: &[ArgItem],
        keys: &[&str],
        helper: &str,
        list_key: Option<&str>,
    ) {
        for item in items {
            match item {
                ArgItem::Flag(path) => {
                    if matches!(list_key, Some("events" | "effects")) {
                        self.check_path_args(path);
                        self.check_path(path, Ns::Type);
                    } else if !keys.contains(&key_of(path).as_str()) {
                        self.refuse(
                            "rust-attribute",
                            path.span(),
                            format!(
                                "`{}` is not an admitted `#[{helper}]` argument",
                                key_of(path)
                            ),
                        );
                    }
                }
                ArgItem::Lit => {}
                ArgItem::Value(path, value) => {
                    let key = key_of(path);
                    if !keys.contains(&key.as_str()) {
                        self.refuse(
                            "rust-attribute",
                            path.span(),
                            format!("`{key}` is not an admitted `#[{helper}]` argument"),
                        );
                        continue;
                    }
                    self.check_value(&key, value, helper);
                }
                ArgItem::List(path, inner) => {
                    let key = key_of(path);
                    if !keys.contains(&key.as_str()) {
                        self.refuse(
                            "rust-attribute",
                            path.span(),
                            format!("`{key}` is not an admitted `#[{helper}]` argument"),
                        );
                        continue;
                    }
                    self.check_items(inner, keys, helper, Some(key.as_str()));
                }
            }
        }
    }

    fn check_value(&mut self, key: &str, value: &syn::Expr, helper: &str) {
        match value {
            syn::Expr::Lit(lit) => {
                let syn::Lit::Str(text) = &lit.lit else {
                    return;
                };
                if key == "crate" {
                    if text.value() != "suprnova::serde" {
                        self.refuse(
                            "rust-attribute",
                            text.span(),
                            format!(
                                "`#[serde(crate = {:?})]` points serde at another crate",
                                text.value()
                            ),
                        );
                    }
                } else if PATH_STRING_KEYS.contains(&key)
                    && !(helper == "serde" && key == "default" && text.value().is_empty())
                {
                    match syn::parse_str::<syn::Path>(&text.value()) {
                        Ok(mut path) => {
                            respan(&mut path, text.span());
                            self.check_path_args(&path);
                            if key == "with" {
                                for member in ["serialize", "deserialize"] {
                                    let mut full = path.clone();
                                    full.segments.push(syn::PathSegment::from(syn::Ident::new(
                                        member,
                                        text.span(),
                                    )));
                                    self.check_path(&full, Ns::Value);
                                }
                            } else {
                                self.check_path(&path, Ns::Value);
                            }
                        }
                        Err(_) => self.refuse(
                            "rust-attribute",
                            text.span(),
                            format!("`{key} = {:?}` is not a path", text.value()),
                        ),
                    }
                } else if TYPE_STRING_KEYS.contains(&key) && helper == "serde" {
                    match syn::parse_str::<syn::Type>(&text.value()) {
                        Ok(ty) => {
                            let saved = self.findings.len();
                            self.visit_type(&ty);
                            for finding in &mut self.findings[saved..] {
                                finding.line = super::modules::line_of(text.span());
                            }
                        }
                        Err(_) => self.refuse(
                            "rust-attribute",
                            text.span(),
                            format!("`{key} = {:?}` is not a type", text.value()),
                        ),
                    }
                }
            }
            syn::Expr::Path(path) if path.qself.is_none() => {
                self.check_path_args(&path.path);
                self.check_path(&path.path, Ns::Value);
            }
            syn::Expr::Unary(unary)
                if matches!(unary.op, syn::UnOp::Neg(_))
                    && matches!(&*unary.expr, syn::Expr::Lit(_)) => {}
            other => self.refuse(
                "rust-attribute",
                other.span(),
                format!("`{key}` takes a literal or a path, not a computed value"),
            ),
        }
    }

    /// Checks the struct `#[live(...)]` attribute's name and view and
    /// records the component it defines (REG-004, REG-030).
    pub(super) fn check_live_struct(&mut self, item: &syn::ItemStruct) {
        let Some(attr) = item
            .attrs
            .iter()
            .find(|attr| matches!(attr.meta, syn::Meta::List(_)) && attr.path().is_ident("live"))
        else {
            return;
        };
        let items = match attr.parse_args_with(parse_args) {
            Ok(items) => items,
            Err(_) => return,
        };
        let literal = |key: &str| {
            items.iter().find_map(|item| match item {
                ArgItem::Value(path, value) if key_of(path) == key => match &**value {
                    syn::Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Str(text),
                        ..
                    }) => Some(text.value()),
                    _ => None,
                },
                _ => None,
            })
        };
        let span = attr.span();
        let ident = unraw(&item.ident);
        match literal("name") {
            Some(name) if name.starts_with(&format!("{}.", self.namespace)) => {}
            Some(name) => self.refuse(
                "rust-component",
                span,
                format!(
                    "the Live component name `{name}` does not start with `{}.`",
                    self.namespace
                ),
            ),
            None => self.refuse(
                "rust-component",
                span,
                format!("`{ident}` has no literal Live component name"),
            ),
        }
        match literal("view") {
            Some(view) if self.views.contains(&view) => {}
            Some(view) => self.refuse(
                "rust-component",
                span,
                format!("the view `{view}` is not one of the component's views nor a view it may include"),
            ),
            None => self.refuse("rust-component", span, format!("`{ident}` has no literal view")),
        }
        let relative: Vec<String> = self
            .module
            .iter()
            .skip(3)
            .cloned()
            .chain(std::iter::once(ident))
            .collect();
        self.defined.push(super::walk::DefinedComponent {
            path: join(&relative),
            file: self.file.clone(),
            line: super::modules::line_of(item.ident.span()),
        });
    }

    /// Visits a macro invocation: admitted `std` macros have their
    /// arguments scanned as code; everything else is refused.
    pub(super) fn visit_macro(&mut self, mac: &syn::Macro, span: proc_macro2::Span) -> Ty {
        let written = segments(&mac.path);
        let last = written.last().cloned().unwrap_or_default();
        let bare =
            written.len() == 1 && mac.path.leading_colon.is_none() && !self.name_bound(&last);
        let name = if bare {
            Some(last.clone())
        } else {
            let (res, _) = self.resolve_silently(&mac.path);
            match res {
                Res::Full(full)
                    if full.len() == 2 && matches!(full[0].as_str(), "std" | "core" | "alloc") =>
                {
                    Some(full[1].clone())
                }
                _ => None,
            }
        };
        let Some(name) = name else {
            self.refuse(
                "rust-macro",
                span,
                format!("`{}!` is not an admitted macro", join(&written)),
            );
            return Ty::Unknown;
        };
        match name.as_str() {
            "include" | "include_str" | "include_bytes" => {
                self.refuse(
                    "rust-macro",
                    span,
                    format!("`{name}!` reads a file at build time"),
                );
                Ty::Unknown
            }
            "env" | "option_env" => {
                self.refuse(
                    "rust-macro",
                    span,
                    format!("`{name}!` reads the build environment"),
                );
                Ty::Unknown
            }
            "println" | "print" | "eprintln" | "eprint" | "dbg" => {
                self.refuse(
                    "rust-macro",
                    span,
                    format!("`{name}!` writes to the process's output"),
                );
                Ty::Unknown
            }
            "stringify" => Ty::prim("str"),
            "line" | "column" => Ty::prim("u32"),
            "file" | "module_path" => Ty::prim("str"),
            "cfg" => Ty::prim("bool"),
            "matches" => {
                self.visit_matches(mac, span);
                Ty::prim("bool")
            }
            "vec" => self.visit_vec(mac, span),
            "write" | "writeln" => {
                let args = self.macro_args(mac, &name, span);
                if let Some(first) = args.first() {
                    let destination = self.visit_expr(first, None);
                    let writable = matches!(&destination, Ty::Std(kind, _) if kind == "String" || kind == "Formatter");
                    if !writable {
                        self.refuse(
                            "rust-method",
                            first.span(),
                            format!("`{name}!` writes to a destination the scan cannot show is a `String` or a `Formatter`"),
                        );
                    }
                }
                for arg in args.iter().skip(1) {
                    self.visit_format_arg(arg);
                }
                Ty::Unknown
            }
            admitted if ADMITTED_MACROS.contains(&admitted) => {
                for arg in self.macro_args(mac, &name, span) {
                    self.visit_format_arg(&arg);
                }
                match admitted {
                    "format" => Ty::string(),
                    "concat" => Ty::prim("str"),
                    _ => Ty::Unknown,
                }
            }
            other => {
                self.refuse(
                    "rust-macro",
                    span,
                    format!("`{other}!` is not an admitted macro"),
                );
                Ty::Unknown
            }
        }
    }

    fn macro_args(
        &mut self,
        mac: &syn::Macro,
        name: &str,
        span: proc_macro2::Span,
    ) -> Vec<syn::Expr> {
        if mac.tokens.is_empty() {
            return Vec::new();
        }
        match mac.parse_body_with(Punctuated::<syn::Expr, Token![,]>::parse_terminated) {
            Ok(args) => args.into_iter().collect(),
            Err(_) => {
                self.refuse(
                    "rust-macro",
                    span,
                    format!("the arguments of `{name}!` are not expressions the scan can read"),
                );
                Vec::new()
            }
        }
    }

    fn visit_format_arg(&mut self, arg: &syn::Expr) {
        match arg {
            syn::Expr::Assign(assign) if matches!(&*assign.left, syn::Expr::Path(path) if path.path.get_ident().is_some()) =>
            {
                self.visit_expr(&assign.right, None);
            }
            other => {
                self.visit_expr(other, None);
            }
        }
    }

    fn visit_vec(&mut self, mac: &syn::Macro, span: proc_macro2::Span) -> Ty {
        let repeat = |input: ParseStream<'_>| -> syn::Result<(syn::Expr, syn::Expr)> {
            let element: syn::Expr = input.parse()?;
            input.parse::<Token![;]>()?;
            let count: syn::Expr = input.parse()?;
            Ok((element, count))
        };
        if let Ok((element, count)) = mac.parse_body_with(repeat) {
            let ty = self.visit_expr(&element, None);
            self.visit_expr(&count, None);
            return Ty::std("Vec", vec![ty]);
        }
        let args = self.macro_args(mac, "vec", span);
        let mut first = Ty::Unknown;
        for (index, arg) in args.iter().enumerate() {
            let ty = self.visit_expr(arg, None);
            if index == 0 {
                first = ty;
            }
        }
        Ty::std("Vec", vec![first])
    }

    fn visit_matches(&mut self, mac: &syn::Macro, span: proc_macro2::Span) {
        let parser =
            |input: ParseStream<'_>| -> syn::Result<(syn::Expr, syn::Pat, Option<syn::Expr>)> {
                let scrutinee: syn::Expr = input.parse()?;
                input.parse::<Token![,]>()?;
                let pattern = syn::Pat::parse_multi_with_leading_vert(input)?;
                let guard = if input.peek(Token![if]) {
                    input.parse::<Token![if]>()?;
                    Some(input.parse()?)
                } else {
                    None
                };
                if input.peek(Token![,]) {
                    input.parse::<Token![,]>()?;
                }
                Ok((scrutinee, pattern, guard))
            };
        match Parser::parse2(parser, mac.tokens.clone()) {
            Ok((scrutinee, pattern, guard)) => {
                let ty = self.visit_expr(&scrutinee, None);
                self.visit_pat(&pattern, &ty);
                if let Some(guard) = guard {
                    self.visit_expr(&guard, None);
                }
            }
            Err(_) => self.refuse(
                "rust-macro",
                span,
                "the arguments of `matches!` are not ones the scan can read".to_string(),
            ),
        }
    }
}

/// Whether a resolved path is the prelude's binding for `name`.
fn is_prelude_path(full: &[String], name: &str) -> bool {
    full.first().is_some_and(|first| first == "std") && full.last().is_some_and(|last| last == name)
}

/// Gives every segment of a path parsed from a string the string's span,
/// so a refusal names the attribute's line.
fn respan(path: &mut syn::Path, span: proc_macro2::Span) {
    for segment in &mut path.segments {
        segment.ident.set_span(span);
    }
}
