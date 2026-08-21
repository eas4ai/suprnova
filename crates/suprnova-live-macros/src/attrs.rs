use proc_macro2::Span;
use syn::ext::IdentExt as _;
use syn::parse::Parser as _;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned as _;
use syn::{Attribute, LitInt, LitStr, Path, Token, Type, parenthesized};

pub(crate) struct ComponentArgs {
    pub(crate) name: LitStr,
    pub(crate) view: LitStr,
    pub(crate) component_version: u16,
    pub(crate) state_schema_version: u16,
    pub(crate) action_schema_version: u16,
    pub(crate) checker_contract_version: u16,
    pub(crate) minimum_protocol_version: u16,
    pub(crate) refresh_on_promote: bool,
    pub(crate) events: Vec<Path>,
    pub(crate) effects: Vec<Path>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum FieldKind {
    State,
    Public,
    Model,
    Locked,
    ServerOnly,
    Session,
    Transient,
    Secret,
}

pub(crate) struct FieldArgs {
    pub(crate) kind: FieldKind,
    pub(crate) url: bool,
}

pub(crate) struct ActionArgs {
    pub(crate) name: Option<LitStr>,
    pub(crate) version: u16,
}

pub(crate) fn parse_component_args(attributes: &[Attribute]) -> syn::Result<ComponentArgs> {
    let mut live_attributes = attributes.iter().filter(|attribute| {
        attribute.path().is_ident("live") || attribute.path().is_ident("__suprnova_live")
    });
    let attribute = live_attributes.next().ok_or_else(|| {
        syn::Error::new(
            Span::call_site(),
            "LiveComponent requires #[live(name = \"...\", view = \"...\")]",
        )
    })?;
    if let Some(duplicate) = live_attributes.next() {
        return Err(syn::Error::new(
            duplicate.span(),
            "duplicate Live component helper",
        ));
    }

    let mut name = None;
    let mut view = None;
    let mut component_version = None;
    let mut state_schema_version = None;
    let mut action_schema_version = None;
    let mut checker_contract_version = None;
    let mut minimum_protocol_version = None;
    let mut refresh_on_promote = None;
    let mut events = None;
    let mut effects = None;

    attribute.parse_nested_meta(|meta| {
        if meta.path.is_ident("name") {
            return assign_once(&mut name, meta.value()?.parse()?, meta.path.span(), "name");
        }
        if meta.path.is_ident("view") {
            return assign_once(&mut view, meta.value()?.parse()?, meta.path.span(), "view");
        }
        if meta.path.is_ident("component_version") {
            return parse_version(&meta, &mut component_version, "component_version");
        }
        if meta.path.is_ident("state_schema_version") {
            return parse_version(&meta, &mut state_schema_version, "state_schema_version");
        }
        if meta.path.is_ident("action_schema_version") {
            return parse_version(&meta, &mut action_schema_version, "action_schema_version");
        }
        if meta.path.is_ident("checker_contract_version") {
            return parse_version(
                &meta,
                &mut checker_contract_version,
                "checker_contract_version",
            );
        }
        if meta.path.is_ident("minimum_protocol_version") {
            return parse_version(
                &meta,
                &mut minimum_protocol_version,
                "minimum_protocol_version",
            );
        }
        if meta.path.is_ident("refresh_on_promote") {
            return assign_once(
                &mut refresh_on_promote,
                true,
                meta.path.span(),
                "refresh_on_promote",
            );
        }
        if meta.path.is_ident("events") {
            let parsed = parse_path_list(&meta)?;
            return assign_once(&mut events, parsed, meta.path.span(), "events");
        }
        if meta.path.is_ident("effects") {
            let parsed = parse_path_list(&meta)?;
            return assign_once(&mut effects, parsed, meta.path.span(), "effects");
        }
        Err(meta.error("unknown Live component helper"))
    })?;

    let name = name.ok_or_else(|| syn::Error::new(attribute.span(), "missing `name` helper"))?;
    validate_registered_name(&name)?;
    let view = view.ok_or_else(|| syn::Error::new(attribute.span(), "missing `view` helper"))?;
    validate_view_name(&view)?;

    let component_version = component_version.unwrap_or(1);
    let state_schema_version = state_schema_version.unwrap_or(1);
    let action_schema_version = action_schema_version.unwrap_or(1);
    let checker_contract_version = checker_contract_version.unwrap_or(1);
    let minimum_protocol_version = minimum_protocol_version.unwrap_or(1);
    if !matches!(minimum_protocol_version, 1 | 2) {
        return Err(syn::Error::new(
            attribute.span(),
            "minimum_protocol_version is not supported by this Live macro release",
        ));
    }
    let refresh_on_promote = refresh_on_promote.unwrap_or(false);
    if refresh_on_promote && minimum_protocol_version < 2 {
        return Err(syn::Error::new(
            attribute.span(),
            "refresh_on_promote requires minimum_protocol_version = 2 or newer",
        ));
    }

    Ok(ComponentArgs {
        name,
        view,
        component_version,
        state_schema_version,
        action_schema_version,
        checker_contract_version,
        minimum_protocol_version,
        refresh_on_promote,
        events: events.unwrap_or_default(),
        effects: effects.unwrap_or_default(),
    })
}

pub(crate) fn parse_field_args(attributes: &[Attribute]) -> syn::Result<FieldArgs> {
    let mut kind = None;
    let mut url_span = None;

    for attribute in attributes {
        let Some(name) = attribute
            .path()
            .get_ident()
            .map(|ident| ident.unraw().to_string())
        else {
            continue;
        };
        let parsed_kind = match name.as_str() {
            "public" => Some(FieldKind::Public),
            "locked" => Some(FieldKind::Locked),
            "server_only" => Some(FieldKind::ServerOnly),
            "session" => Some(FieldKind::Session),
            "secret" => Some(FieldKind::Secret),
            "model" => Some(parse_model_kind(attribute)?),
            "transient" => {
                return Err(syn::Error::new(
                    attribute.span(),
                    "transient state must be declared with #[model(transient)]",
                ));
            }
            "url" => {
                ensure_path_attribute(attribute, "url")?;
                if url_span.replace(attribute.span()).is_some() {
                    return Err(syn::Error::new(attribute.span(), "duplicate #[url] helper"));
                }
                None
            }
            name if is_method_helper(name) => {
                return Err(syn::Error::new(
                    attribute.span(),
                    "method helper cannot be placed on component state",
                ));
            }
            "live" | "__suprnova_live" => {
                return Err(syn::Error::new(
                    attribute.span(),
                    "component helper cannot be placed on a field",
                ));
            }
            _ => None,
        };

        if let Some(parsed_kind) = parsed_kind
            && kind.replace((parsed_kind, attribute.span())).is_some()
        {
            return Err(syn::Error::new(
                attribute.span(),
                "a component field may declare only one state category",
            ));
        }
    }

    let kind = kind.map_or(FieldKind::State, |(kind, _)| kind);
    if let Some(span) = url_span
        && matches!(
            kind,
            FieldKind::Transient | FieldKind::Secret | FieldKind::Session
        )
    {
        return Err(syn::Error::new(
            span,
            "transient, secret, and session state cannot be URL-exposed",
        ));
    }
    Ok(FieldArgs {
        kind,
        url: url_span.is_some(),
    })
}

pub(crate) fn parse_action_args(attribute: &Attribute) -> syn::Result<ActionArgs> {
    let mut name = None;
    let mut version = None;
    match &attribute.meta {
        syn::Meta::Path(_) => {}
        syn::Meta::List(_) => attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident("name") {
                return assign_once(&mut name, meta.value()?.parse()?, meta.path.span(), "name");
            }
            if meta.path.is_ident("version") {
                return parse_version(&meta, &mut version, "version");
            }
            Err(meta.error("unknown action helper"))
        })?,
        syn::Meta::NameValue(_) => {
            return Err(syn::Error::new(
                attribute.span(),
                "expected #[action] or #[action(name = \"...\", version = N)]",
            ));
        }
    }
    if let Some(name) = &name {
        validate_registered_name(name)?;
    }
    Ok(ActionArgs {
        name,
        version: version.unwrap_or(1),
    })
}

pub(crate) fn validate_registered_name(value: &LitStr) -> syn::Result<()> {
    let value_text = value.value();
    let valid = !value_text.is_empty()
        && value_text.len() <= 128
        && value_text.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'/' | b'-')
        });
    if !valid {
        return Err(syn::Error::new(
            value.span(),
            "registered names must use the bounded ASCII component identity grammar",
        ));
    }
    Ok(())
}

pub(crate) fn contains_reference(ty: &Type) -> bool {
    match ty {
        Type::Reference(_) | Type::Ptr(_) => true,
        Type::Array(value) => contains_reference(&value.elem),
        Type::Slice(value) => contains_reference(&value.elem),
        Type::Tuple(value) => value.elems.iter().any(contains_reference),
        Type::Paren(value) => contains_reference(&value.elem),
        Type::Group(value) => contains_reference(&value.elem),
        Type::Path(value) => {
            value
                .qself
                .as_ref()
                .is_some_and(|qself| contains_reference(&qself.ty))
                || value
                    .path
                    .segments
                    .iter()
                    .any(|segment| match &segment.arguments {
                        syn::PathArguments::AngleBracketed(arguments) => {
                            arguments.args.iter().any(|argument| match argument {
                                syn::GenericArgument::Type(ty) => contains_reference(ty),
                                syn::GenericArgument::AssocType(assoc) => {
                                    contains_reference(&assoc.ty)
                                }
                                _ => false,
                            })
                        }
                        syn::PathArguments::Parenthesized(arguments) => {
                            arguments.inputs.iter().any(contains_reference)
                                || match &arguments.output {
                                    syn::ReturnType::Default => false,
                                    syn::ReturnType::Type(_, ty) => contains_reference(ty),
                                }
                        }
                        syn::PathArguments::None => false,
                    })
        }
        _ => false,
    }
}

pub(crate) fn is_method_helper(name: &str) -> bool {
    matches!(
        name,
        "mount"
            | "action"
            | "computed"
            | "validate"
            | "hydrate"
            | "rendering"
            | "rendered"
            | "dehydrate"
            | "teardown"
            | "params_changed"
            | "lazy_complete"
    )
}

pub(crate) fn is_field_helper(name: &str) -> bool {
    matches!(
        name,
        "public" | "model" | "locked" | "server_only" | "session" | "secret" | "transient" | "url"
    )
}

fn parse_model_kind(attribute: &Attribute) -> syn::Result<FieldKind> {
    match &attribute.meta {
        syn::Meta::Path(_) => Ok(FieldKind::Model),
        syn::Meta::List(list) => {
            let mut transient = false;
            list.parse_nested_meta(|meta| {
                if !meta.path.is_ident("transient") {
                    return Err(meta.error("unknown model helper"));
                }
                if transient {
                    return Err(meta.error("duplicate transient model helper"));
                }
                transient = true;
                Ok(())
            })?;
            if !transient {
                return Err(syn::Error::new(
                    attribute.span(),
                    "model helper list must contain `transient`",
                ));
            }
            Ok(FieldKind::Transient)
        }
        syn::Meta::NameValue(_) => Err(syn::Error::new(
            attribute.span(),
            "expected #[model] or #[model(transient)]",
        )),
    }
}

fn ensure_path_attribute(attribute: &Attribute, name: &str) -> syn::Result<()> {
    if matches!(attribute.meta, syn::Meta::Path(_)) {
        Ok(())
    } else {
        Err(syn::Error::new(
            attribute.span(),
            format!("#[{name}] does not accept arguments"),
        ))
    }
}

fn validate_view_name(value: &LitStr) -> syn::Result<()> {
    let text = value.value();
    let valid = !text.is_empty()
        && text.len() <= 256
        && !text.starts_with('/')
        && !text.ends_with('/')
        && !text.contains(':')
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'/' | b'-'))
        && text
            .split('/')
            .all(|segment| !segment.is_empty() && !matches!(segment, "." | ".."));
    if !valid {
        return Err(syn::Error::new(
            value.span(),
            "view must be a bounded relative template identity without traversal",
        ));
    }
    Ok(())
}

fn parse_version(
    meta: &syn::meta::ParseNestedMeta<'_>,
    slot: &mut Option<u16>,
    label: &str,
) -> syn::Result<()> {
    let literal: LitInt = meta.value()?.parse()?;
    let version = literal.base10_parse::<u16>()?;
    if version == 0 {
        return Err(syn::Error::new(
            literal.span(),
            format!("{label} must be a non-zero u16"),
        ));
    }
    assign_once(slot, version, meta.path.span(), label)
}

fn parse_path_list(meta: &syn::meta::ParseNestedMeta<'_>) -> syn::Result<Vec<Path>> {
    let content;
    parenthesized!(content in meta.input);
    let paths = Punctuated::<Path, Token![,]>::parse_terminated.parse2(content.parse()?)?;
    if paths.is_empty() {
        return Err(meta.error("payload list cannot be empty"));
    }
    Ok(paths.into_iter().collect())
}

fn assign_once<T>(slot: &mut Option<T>, value: T, span: Span, label: &str) -> syn::Result<()> {
    if slot.replace(value).is_some() {
        return Err(syn::Error::new(span, format!("duplicate `{label}` helper")));
    }
    Ok(())
}
