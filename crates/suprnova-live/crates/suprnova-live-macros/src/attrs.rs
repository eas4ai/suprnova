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
    pub(crate) timing: Option<ModelTimingArgs>,
    pub(crate) url: Option<UrlArgs>,
}

#[derive(Clone, Copy)]
pub(crate) enum ModelTimingArgs {
    Immediate,
    Change,
    Blur,
    Submit,
    Debounce(u32),
}

#[derive(Clone, Copy)]
pub(crate) enum UrlModeArgs {
    Reflect,
    Navigate,
}

pub(crate) struct UrlArgs {
    pub(crate) key: Option<LitStr>,
    pub(crate) mode: UrlModeArgs,
    pub(crate) omit_default: bool,
    pub(crate) span: Span,
}

pub(crate) struct ActionArgs {
    pub(crate) name: Option<LitStr>,
    pub(crate) version: u16,
    pub(crate) authorization: ActionAuthorizationArgs,
    pub(crate) validation: ActionValidationArgs,
    pub(crate) transaction: ActionTransactionArgs,
}

#[derive(Clone, Copy)]
pub(crate) enum ActionAuthorizationArgs {
    Public,
    Current,
}

#[derive(Clone, Copy)]
pub(crate) enum ActionValidationArgs {
    None,
    Whole,
    Arguments,
    All,
}

#[derive(Clone, Copy)]
pub(crate) enum ActionTransactionArgs {
    None,
    Required,
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
    let mut timing = None;
    let mut url = None;

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
            "model" => {
                let (model_kind, model_timing) = parse_model_args(attribute)?;
                timing = Some(model_timing);
                Some(model_kind)
            }
            "transient" => {
                return Err(syn::Error::new(
                    attribute.span(),
                    "transient state must be declared with #[model(transient)]",
                ));
            }
            "url" => {
                if url.replace(parse_url_args(attribute)?).is_some() {
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
    if let Some(url) = &url
        && !matches!(
            kind,
            FieldKind::State | FieldKind::Public | FieldKind::Model
        )
    {
        return Err(syn::Error::new(
            url.span,
            "only ordinary, public, or model state can be URL-exposed",
        ));
    }
    Ok(FieldArgs { kind, timing, url })
}

pub(crate) fn parse_action_args(attribute: &Attribute) -> syn::Result<ActionArgs> {
    let mut name = None;
    let mut version = None;
    let mut authorization = None;
    let mut validation = None;
    let mut transaction = None;
    match &attribute.meta {
        syn::Meta::Path(_) => {}
        syn::Meta::List(_) => attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident("name") {
                return assign_once(&mut name, meta.value()?.parse()?, meta.path.span(), "name");
            }
            if meta.path.is_ident("version") {
                return parse_version(&meta, &mut version, "version");
            }
            if meta.path.is_ident("authorize") {
                let value: LitStr = meta.value()?.parse()?;
                let parsed = match value.value().as_str() {
                    "public" => ActionAuthorizationArgs::Public,
                    "current" => ActionAuthorizationArgs::Current,
                    _ => {
                        return Err(syn::Error::new(
                            value.span(),
                            "authorize must be `public` or `current`",
                        ));
                    }
                };
                return assign_once(&mut authorization, parsed, meta.path.span(), "authorize");
            }
            if meta.path.is_ident("validate") {
                let value: LitStr = meta.value()?.parse()?;
                let parsed = match value.value().as_str() {
                    "none" => ActionValidationArgs::None,
                    "whole" => ActionValidationArgs::Whole,
                    "arguments" => ActionValidationArgs::Arguments,
                    "all" => ActionValidationArgs::All,
                    _ => {
                        return Err(syn::Error::new(
                            value.span(),
                            "validate must be `none`, `whole`, `arguments`, or `all`",
                        ));
                    }
                };
                return assign_once(&mut validation, parsed, meta.path.span(), "validate");
            }
            if meta.path.is_ident("transaction") {
                let value: LitStr = meta.value()?.parse()?;
                let parsed = match value.value().as_str() {
                    "none" => ActionTransactionArgs::None,
                    "required" => ActionTransactionArgs::Required,
                    _ => {
                        return Err(syn::Error::new(
                            value.span(),
                            "transaction must be `none` or `required`",
                        ));
                    }
                };
                return assign_once(&mut transaction, parsed, meta.path.span(), "transaction");
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
        authorization: authorization.unwrap_or(ActionAuthorizationArgs::Public),
        validation: validation.unwrap_or(ActionValidationArgs::None),
        transaction: transaction.unwrap_or(ActionTransactionArgs::None),
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

fn parse_model_args(attribute: &Attribute) -> syn::Result<(FieldKind, ModelTimingArgs)> {
    match &attribute.meta {
        syn::Meta::Path(_) => Ok((FieldKind::Model, ModelTimingArgs::Submit)),
        syn::Meta::List(list) => {
            let mut transient = false;
            let mut timing = None;
            list.parse_nested_meta(|meta| {
                if meta.path.is_ident("transient") {
                    if transient {
                        return Err(meta.error("duplicate transient model helper"));
                    }
                    transient = true;
                    return Ok(());
                }
                let parsed_timing = if meta.path.is_ident("immediate") {
                    ModelTimingArgs::Immediate
                } else if meta.path.is_ident("change") {
                    ModelTimingArgs::Change
                } else if meta.path.is_ident("blur") {
                    ModelTimingArgs::Blur
                } else if meta.path.is_ident("submit") {
                    ModelTimingArgs::Submit
                } else if meta.path.is_ident("debounce") {
                    let literal: LitInt = meta.value()?.parse()?;
                    let milliseconds = literal.base10_parse::<u32>()?;
                    if milliseconds == 0 || milliseconds > 60_000 {
                        return Err(syn::Error::new(
                            literal.span(),
                            "model debounce must be between 1 and 60000 milliseconds",
                        ));
                    }
                    ModelTimingArgs::Debounce(milliseconds)
                } else {
                    return Err(meta.error("unknown model helper"));
                };
                if timing.replace(parsed_timing).is_some() {
                    return Err(meta.error("conflicting model timing helpers"));
                }
                Ok(())
            })?;
            Ok((
                if transient {
                    FieldKind::Transient
                } else {
                    FieldKind::Model
                },
                timing.unwrap_or(ModelTimingArgs::Submit),
            ))
        }
        syn::Meta::NameValue(_) => Err(syn::Error::new(
            attribute.span(),
            "expected #[model] or #[model(transient)]",
        )),
    }
}

fn parse_url_args(attribute: &Attribute) -> syn::Result<UrlArgs> {
    let mut key = None;
    let mut mode = None;
    let mut omit_default = false;
    match &attribute.meta {
        syn::Meta::Path(_) => {}
        syn::Meta::List(list) => list.parse_nested_meta(|meta| {
            if meta.path.is_ident("key") {
                let parsed: LitStr = meta.value()?.parse()?;
                validate_query_key(&parsed)?;
                return assign_once(&mut key, parsed, meta.path.span(), "key");
            }
            if meta.path.is_ident("mode") {
                let literal: LitStr = meta.value()?.parse()?;
                let parsed = match literal.value().as_str() {
                    "reflect" => UrlModeArgs::Reflect,
                    "navigate" => UrlModeArgs::Navigate,
                    _ => {
                        return Err(syn::Error::new(
                            literal.span(),
                            "URL mode must be `reflect` or `navigate`",
                        ));
                    }
                };
                return assign_once(&mut mode, parsed, meta.path.span(), "mode");
            }
            if meta.path.is_ident("omit_default") {
                if omit_default {
                    return Err(meta.error("duplicate omit_default URL helper"));
                }
                omit_default = true;
                return Ok(());
            }
            Err(meta.error("unknown URL helper"))
        })?,
        syn::Meta::NameValue(_) => {
            return Err(syn::Error::new(
                attribute.span(),
                "expected #[url] or #[url(key = \"...\", mode = \"reflect\")]",
            ));
        }
    }
    Ok(UrlArgs {
        key,
        mode: mode.unwrap_or(UrlModeArgs::Reflect),
        omit_default,
        span: attribute.span(),
    })
}

fn validate_query_key(value: &LitStr) -> syn::Result<()> {
    let text = value.value();
    let valid = !text.is_empty()
        && text.len() <= 64
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'));
    if !valid {
        return Err(syn::Error::new(
            value.span(),
            "URL query keys use a bounded ASCII key grammar",
        ));
    }
    Ok(())
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
