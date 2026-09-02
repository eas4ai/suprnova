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
    pub(crate) streams: Vec<StreamArgs>,
}

/// One declared asynchronous stream subscription on a Live component.
pub(crate) struct StreamArgs {
    pub(crate) name: LitStr,
    pub(crate) topics: Vec<LitStr>,
    pub(crate) events: Vec<Path>,
    pub(crate) targets: Vec<StreamTargetArgs>,
    pub(crate) fanout: u16,
    pub(crate) modes: Vec<StreamModeArgs>,
    pub(crate) reconnect: StreamReconnectArgs,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum StreamTargetArgs {
    SelfIsland,
    Parent,
    Child,
    Document,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum StreamModeArgs {
    ServerSentEvents,
    WebSocket,
}

#[derive(Clone, Copy)]
pub(crate) enum StreamReconnectArgs {
    RefreshOnReconnect,
    ResumeOrRefresh { maximum_attempts: u8 },
}

const MAX_STREAM_TOPIC_BYTES: usize = 256;
const MAX_STREAM_FANOUT: u16 = 1_024;
const MAX_STREAM_RESUME_ATTEMPTS: u8 = 16;

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
    pub(crate) upload: Option<UploadArgs>,
}

pub(crate) struct UploadArgs {
    pub(crate) policy: Path,
    pub(crate) span: Span,
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

pub(crate) struct ValidationHookArgs {
    pub(crate) action: Option<LitStr>,
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
    let mut streams = None;

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
        if meta.path.is_ident("streams") {
            let span = meta.path.span();
            let parsed = parse_stream_list(&meta)?;
            return assign_once(&mut streams, (parsed, span), span, "streams");
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
    let streams = match streams {
        Some((streams, span)) => {
            if minimum_protocol_version < 2 {
                return Err(syn::Error::new(
                    span,
                    "streams require minimum_protocol_version = 2 or newer",
                ));
            }
            streams
        }
        None => Vec::new(),
    };

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
        streams,
    })
}

fn parse_stream_list(meta: &syn::meta::ParseNestedMeta<'_>) -> syn::Result<Vec<StreamArgs>> {
    let mut streams: Vec<StreamArgs> = Vec::new();
    meta.parse_nested_meta(|stream| {
        if !stream.path.is_ident("stream") {
            return Err(stream.error("expected `stream(name = \"...\", ...)`"));
        }
        let parsed = parse_stream_args(&stream)?;
        if streams
            .iter()
            .any(|existing| existing.name.value() == parsed.name.value())
        {
            return Err(syn::Error::new(parsed.name.span(), "duplicate stream name"));
        }
        for event in &parsed.events {
            if streams
                .iter()
                .any(|existing| existing.events.iter().any(|known| same_path(known, event)))
            {
                return Err(syn::Error::new(
                    event.span(),
                    "an event type may be delivered by only one stream",
                ));
            }
        }
        streams.push(parsed);
        Ok(())
    })?;
    if streams.is_empty() {
        return Err(meta.error("streams cannot be empty"));
    }
    Ok(streams)
}

fn parse_stream_args(meta: &syn::meta::ParseNestedMeta<'_>) -> syn::Result<StreamArgs> {
    let mut name = None;
    let mut topics = None;
    let mut events = None;
    let mut targets = None;
    let mut fanout = None;
    let mut modes = None;
    let mut reconnect = None;
    let mut resume_attempts = None;
    meta.parse_nested_meta(|item| {
        if item.path.is_ident("name") {
            return assign_once(&mut name, item.value()?.parse()?, item.path.span(), "name");
        }
        if item.path.is_ident("topics") {
            let parsed = parse_str_list(&item, "topics")?;
            return assign_once(&mut topics, parsed, item.path.span(), "topics");
        }
        if item.path.is_ident("events") {
            let parsed = parse_path_list(&item)?;
            return assign_once(&mut events, parsed, item.path.span(), "events");
        }
        if item.path.is_ident("targets") {
            let parsed = parse_str_list(&item, "targets")?
                .iter()
                .map(parse_stream_target)
                .collect::<syn::Result<Vec<_>>>()?;
            return assign_once(&mut targets, parsed, item.path.span(), "targets");
        }
        if item.path.is_ident("fanout") {
            let literal: LitInt = item.value()?.parse()?;
            let value = literal.base10_parse::<u16>()?;
            if value == 0 || value > MAX_STREAM_FANOUT {
                return Err(syn::Error::new(
                    literal.span(),
                    "fanout must be between 1 and 1024",
                ));
            }
            return assign_once(&mut fanout, value, item.path.span(), "fanout");
        }
        if item.path.is_ident("modes") {
            let parsed = parse_str_list(&item, "modes")?
                .iter()
                .map(parse_stream_mode)
                .collect::<syn::Result<Vec<_>>>()?;
            return assign_once(&mut modes, parsed, item.path.span(), "modes");
        }
        if item.path.is_ident("reconnect") {
            let value: LitStr = item.value()?.parse()?;
            return assign_once(&mut reconnect, value, item.path.span(), "reconnect");
        }
        if item.path.is_ident("resume_attempts") {
            let literal: LitInt = item.value()?.parse()?;
            let value = literal.base10_parse::<u8>()?;
            if value == 0 || value > MAX_STREAM_RESUME_ATTEMPTS {
                return Err(syn::Error::new(
                    literal.span(),
                    "resume_attempts must be between 1 and 16",
                ));
            }
            return assign_once(
                &mut resume_attempts,
                value,
                item.path.span(),
                "resume_attempts",
            );
        }
        Err(item.error("unknown stream helper"))
    })?;

    let name = name.ok_or_else(|| meta.error("missing stream `name` helper"))?;
    validate_registered_name(&name)?;
    let topics = topics.ok_or_else(|| meta.error("missing stream `topics` helper"))?;
    for topic in &topics {
        validate_topic_template(topic)?;
    }
    if has_duplicate_strings(&topics) {
        return Err(meta.error("duplicate stream topic"));
    }
    let events = events.ok_or_else(|| meta.error("missing stream `events` helper"))?;
    for (index, event) in events.iter().enumerate() {
        if events[..index].iter().any(|known| same_path(known, event)) {
            return Err(syn::Error::new(event.span(), "duplicate stream event"));
        }
    }
    let targets = targets.unwrap_or_else(|| vec![StreamTargetArgs::SelfIsland]);
    if (1..targets.len()).any(|index| targets[..index].contains(&targets[index])) {
        return Err(meta.error("duplicate stream target"));
    }
    let fanout = fanout.unwrap_or(1);
    if usize::from(fanout) < targets.len() {
        return Err(meta.error("fanout must cover every declared target"));
    }
    let modes =
        modes.unwrap_or_else(|| vec![StreamModeArgs::ServerSentEvents, StreamModeArgs::WebSocket]);
    if (1..modes.len()).any(|index| modes[..index].contains(&modes[index])) {
        return Err(meta.error("duplicate stream mode"));
    }
    let reconnect = match reconnect.as_ref().map(LitStr::value).as_deref() {
        None | Some("resume_or_refresh") => StreamReconnectArgs::ResumeOrRefresh {
            maximum_attempts: resume_attempts.unwrap_or(4),
        },
        Some("refresh_on_reconnect") => {
            if resume_attempts.is_some() {
                return Err(
                    meta.error("resume_attempts requires reconnect = \"resume_or_refresh\"")
                );
            }
            StreamReconnectArgs::RefreshOnReconnect
        }
        Some(_) => {
            return Err(syn::Error::new(
                reconnect.expect("reconnect literal present").span(),
                "reconnect must be `resume_or_refresh` or `refresh_on_reconnect`",
            ));
        }
    };
    Ok(StreamArgs {
        name,
        topics,
        events,
        targets,
        fanout,
        modes,
        reconnect,
    })
}

fn parse_stream_target(value: &LitStr) -> syn::Result<StreamTargetArgs> {
    match value.value().as_str() {
        "self" => Ok(StreamTargetArgs::SelfIsland),
        "parent" => Ok(StreamTargetArgs::Parent),
        "child" => Ok(StreamTargetArgs::Child),
        "document" => Ok(StreamTargetArgs::Document),
        _ => Err(syn::Error::new(
            value.span(),
            "stream targets must be `self`, `parent`, `child`, or `document`",
        )),
    }
}

fn parse_stream_mode(value: &LitStr) -> syn::Result<StreamModeArgs> {
    match value.value().as_str() {
        "sse" => Ok(StreamModeArgs::ServerSentEvents),
        "websocket" => Ok(StreamModeArgs::WebSocket),
        _ => Err(syn::Error::new(
            value.span(),
            "stream modes must be `sse` or `websocket`",
        )),
    }
}

fn validate_topic_template(value: &LitStr) -> syn::Result<()> {
    let text = value.value();
    let valid = !text.is_empty()
        && text.len() <= MAX_STREAM_TOPIC_BYTES
        && text.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'/' | b'-')
        })
        && text.split('/').all(|segment| {
            let parameter = segment.strip_prefix(':').unwrap_or(segment);
            !parameter.is_empty() && !matches!(parameter, "." | "..") && !parameter.contains(':')
        });
    if !valid {
        return Err(syn::Error::new(
            value.span(),
            "stream topics must use bounded `segment/:parameter` paths",
        ));
    }
    Ok(())
}

fn has_duplicate_strings(values: &[LitStr]) -> bool {
    (1..values.len()).any(|index| {
        values[..index]
            .iter()
            .any(|known| known.value() == values[index].value())
    })
}

fn same_path(left: &Path, right: &Path) -> bool {
    use quote::ToTokens as _;
    left.to_token_stream().to_string() == right.to_token_stream().to_string()
}

fn parse_str_list(meta: &syn::meta::ParseNestedMeta<'_>, label: &str) -> syn::Result<Vec<LitStr>> {
    let content;
    parenthesized!(content in meta.input);
    let values = Punctuated::<LitStr, Token![,]>::parse_terminated.parse2(content.parse()?)?;
    if values.is_empty() {
        return Err(meta.error(format!("{label} cannot be empty")));
    }
    Ok(values.into_iter().collect())
}

pub(crate) fn parse_field_args(attributes: &[Attribute]) -> syn::Result<FieldArgs> {
    let mut kind = None;
    let mut timing = None;
    let mut url = None;
    let mut upload = None;

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
            "upload" => {
                if upload.replace(parse_upload_args(attribute)?).is_some() {
                    return Err(syn::Error::new(
                        attribute.span(),
                        "duplicate #[upload] helper",
                    ));
                }
                None
            }
            "validate" if matches!(attribute.meta, syn::Meta::List(_)) => {
                // `validator::Validate` owns field-level list-form rules. Bare
                // `#[validate]` remains the Live method helper and is rejected
                // on state below.
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
    if let Some(upload) = &upload
        && !matches!(kind, FieldKind::Model | FieldKind::Transient)
    {
        return Err(syn::Error::new(
            upload.span,
            "only model-backed state can declare an upload policy",
        ));
    }
    Ok(FieldArgs {
        kind,
        timing,
        url,
        upload,
    })
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

pub(crate) fn parse_validation_hook_args(attribute: &Attribute) -> syn::Result<ValidationHookArgs> {
    let mut action = None;
    match &attribute.meta {
        syn::Meta::Path(_) => {}
        syn::Meta::List(_) => attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident("action") {
                return assign_once(
                    &mut action,
                    meta.value()?.parse()?,
                    meta.path.span(),
                    "action",
                );
            }
            Err(meta.error("unknown validation helper"))
        })?,
        syn::Meta::NameValue(_) => {
            return Err(syn::Error::new(
                attribute.span(),
                "expected #[validate] or #[validate(action = \"...\")]",
            ));
        }
    }
    if let Some(action) = &action {
        validate_registered_name(action)?;
    }
    Ok(ValidationHookArgs { action })
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
        "public"
            | "model"
            | "locked"
            | "server_only"
            | "session"
            | "secret"
            | "transient"
            | "url"
            | "upload"
    )
}

fn parse_upload_args(attribute: &Attribute) -> syn::Result<UploadArgs> {
    let mut policy = None;
    match &attribute.meta {
        syn::Meta::List(list) => list.parse_nested_meta(|meta| {
            if meta.path.is_ident("policy") {
                let parsed: Path = meta.value()?.parse()?;
                return assign_once(&mut policy, parsed, meta.path.span(), "policy");
            }
            Err(meta.error("unknown upload helper"))
        })?,
        _ => {
            return Err(syn::Error::new(
                attribute.span(),
                "expected #[upload(policy = application::policy)]",
            ));
        }
    }
    let policy = policy.ok_or_else(|| {
        syn::Error::new(
            attribute.span(),
            "upload policy helper requires `policy = path`",
        )
    })?;
    Ok(UploadArgs {
        policy,
        span: attribute.span(),
    })
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
