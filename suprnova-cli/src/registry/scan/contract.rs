//! A Live component's contract read from its Rust syntax (REG-022). The
//! derive and the `#[live]` impl macro build `ComponentMetadata` from the
//! same syntax when the application compiles; this module mirrors them
//! without compiling anything, so `live:check`'s view checker can check the
//! component's views before a byte of it is installed. Each type-to-codec
//! rule matches the macro's in `suprnova-macros/src/live/`.

use std::collections::BTreeMap;

use syn::{Attribute, Fields, ImplItem, Item, LitInt, LitStr, Type};

use suprnova_live::action::{
    ActionArgumentField, ActionArgumentSchema, AuthorizationRequirement, TransactionPolicy,
};
use suprnova_live::async_updates::{
    BoundedEventNames, BoundedTargets, BoundedTopics, BrowserPayloadSchema, EventTarget,
    ReconnectPolicy, StreamName, SubscriptionMetadata, SubscriptionMode, SubscriptionModes,
    TopicName,
};
use suprnova_live::identity::{ActionName, ComponentName, ModelField, ViewName};
use suprnova_live::metadata::{
    ActionMetadata, ComponentMetadata, ContractVersions, EffectMetadata, EventMetadata,
    FieldMetadata,
};
use suprnova_live::snapshot::state::{FieldCategory, StateCodec};
use suprnova_live::state::{BindingTiming, ModelCodec, UrlBinding, UrlBindingMode};
use suprnova_live::upload::{UploadFieldPolicy, UploadReplacementPolicy, UploadScanPolicy};
use suprnova_live::validation::ValidationSelection;

use super::ComponentFiles;
use super::Finding;
use super::limits::{self, Language};

/// One component's contract, read from its struct and `#[live]` impls.
pub(crate) struct Contract {
    /// The metadata the macros would generate.
    pub metadata: ComponentMetadata,
}

/// Every contract a component's Rust declares, and why any could not be
/// read.
#[derive(Default)]
pub(crate) struct Contracts {
    /// The contracts read in full.
    pub contracts: Vec<Contract>,
    /// The contracts that could not be read, as refusals.
    pub findings: Vec<Finding>,
}

/// A payload contract an `impl EventPayloadMetadata` or
/// `impl EffectPayloadMetadata` declares with literal constants.
#[derive(Clone, Debug, Default)]
struct Payload {
    name: Option<String>,
    version: Option<u16>,
    schema: Option<BrowserPayloadSchema>,
    contract: Option<String>,
}

/// What the files declare, gathered before any contract is built.
#[derive(Default)]
struct Declarations<'a> {
    structs: Vec<(&'a syn::ItemStruct, String)>,
    live_impls: BTreeMap<String, Vec<(&'a syn::ItemImpl, String)>>,
    events: BTreeMap<String, Payload>,
    effects: BTreeMap<String, Payload>,
    functions: BTreeMap<String, &'a syn::ItemFn>,
}

/// Reads every `#[live]` component the component's Rust declares.
pub(crate) fn read(component: &ComponentFiles<'_>) -> Contracts {
    let mut parsed = Vec::new();
    for (name, bytes) in component.files {
        if !name.ends_with(".rs") {
            continue;
        }
        // The Rust scan reports a file it cannot parse or that nests past
        // its limits; such a file holds no contract this module can read.
        let Ok(text) = std::str::from_utf8(bytes) else {
            continue;
        };
        if limits::check(text, Language::Rust).is_err() {
            continue;
        }
        if let Ok(file) = syn::parse_file(text) {
            parsed.push((name.clone(), file));
        }
    }
    let mut declarations = Declarations::default();
    for (name, file) in &parsed {
        gather(&file.items, name, &mut declarations);
    }
    let mut contracts = Contracts::default();
    for (item, file) in &declarations.structs {
        let line = super::rust::line_of_span(item.ident.span());
        match build(item, &declarations) {
            Ok(metadata) => contracts.contracts.push(Contract { metadata }),
            Err(message) => contracts.findings.push(Finding {
                check: "view-contract",
                file: file.clone(),
                line,
                message: format!(
                    "the Live contract of `{}` cannot be read from its source: {message}",
                    item.ident
                ),
            }),
        }
    }
    contracts
}

fn gather<'a>(items: &'a [Item], file: &str, declarations: &mut Declarations<'a>) {
    for item in items {
        match item {
            Item::Struct(item) if live_args(&item.attrs).is_some() => {
                declarations.structs.push((item, file.to_string()));
            }
            Item::Impl(item) => {
                let Some(type_name) = type_name(&item.self_ty) else {
                    continue;
                };
                if let Some((_, trait_path, _)) = &item.trait_ {
                    let trait_name = trait_path
                        .segments
                        .last()
                        .map(|segment| segment.ident.to_string())
                        .unwrap_or_default();
                    match trait_name.as_str() {
                        "EventPayloadMetadata" => {
                            declarations.events.insert(type_name, payload(item));
                        }
                        "EffectPayloadMetadata" => {
                            declarations.effects.insert(type_name, payload(item));
                        }
                        _ => {}
                    }
                } else if item.attrs.iter().any(|attr| attr.path().is_ident("live")) {
                    declarations
                        .live_impls
                        .entry(type_name)
                        .or_default()
                        .push((item, file.to_string()));
                }
            }
            Item::Fn(function) => {
                declarations
                    .functions
                    .insert(function.sig.ident.to_string(), function);
            }
            Item::Mod(module) => {
                if let Some((_, content)) = &module.content {
                    gather(content, file, declarations);
                }
            }
            _ => {}
        }
    }
}

/// The last segment of a path type, which names the type within the
/// component.
fn type_name(ty: &Type) -> Option<String> {
    match ty {
        Type::Path(path) if path.qself.is_none() => path
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string()),
        _ => None,
    }
}

fn payload(item: &syn::ItemImpl) -> Payload {
    let mut payload = Payload::default();
    for member in &item.items {
        let ImplItem::Const(constant) = member else {
            continue;
        };
        let value = &constant.expr;
        match constant.ident.to_string().as_str() {
            "NAME" => payload.name = string_literal(value),
            "VERSION" => payload.version = int_literal(value),
            "PAYLOAD_CONTRACT" => payload.contract = string_literal(value),
            "SCHEMA" => payload.schema = schema(value),
            _ => {}
        }
    }
    payload
}

fn string_literal(expr: &syn::Expr) -> Option<String> {
    match expr {
        syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(text),
            ..
        }) => Some(text.value()),
        _ => None,
    }
}

fn int_literal<T: std::str::FromStr>(expr: &syn::Expr) -> Option<T>
where
    T::Err: std::fmt::Display,
{
    match expr {
        syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Int(int),
            ..
        }) => int.base10_parse().ok(),
        _ => None,
    }
}

fn schema(expr: &syn::Expr) -> Option<BrowserPayloadSchema> {
    let syn::Expr::Path(path) = expr else {
        return None;
    };
    let variant = path.path.segments.last()?.ident.to_string();
    Some(match variant.as_str() {
        "Json" => BrowserPayloadSchema::Json,
        "Null" => BrowserPayloadSchema::Null,
        "Boolean" => BrowserPayloadSchema::Boolean,
        "I64" => BrowserPayloadSchema::I64,
        "U64" => BrowserPayloadSchema::U64,
        "F64" => BrowserPayloadSchema::F64,
        "String" => BrowserPayloadSchema::String,
        _ => return None,
    })
}

/// The `#[live(...)]` attribute of a component struct.
fn live_args(attrs: &[Attribute]) -> Option<&Attribute> {
    attrs
        .iter()
        .find(|attr| attr.path().is_ident("live") && matches!(attr.meta, syn::Meta::List(_)))
}

/// The struct-level arguments, as the derive reads them.
#[derive(Default)]
struct ComponentArgs {
    name: Option<String>,
    view: Option<String>,
    component_version: Option<u16>,
    state_schema_version: Option<u16>,
    action_schema_version: Option<u16>,
    checker_contract_version: Option<u16>,
    minimum_protocol_version: Option<u16>,
    refresh_on_promote: bool,
    events: Vec<syn::Path>,
    effects: Vec<syn::Path>,
    streams: Vec<StreamArgs>,
}

struct StreamArgs {
    name: String,
    topics: Vec<String>,
    events: Vec<syn::Path>,
    targets: Vec<String>,
    fanout: u16,
    modes: Vec<String>,
    reconnect: Option<String>,
    resume_attempts: Option<u8>,
}

fn parse_path_list(meta: &syn::meta::ParseNestedMeta<'_>) -> syn::Result<Vec<syn::Path>> {
    let content;
    syn::parenthesized!(content in meta.input);
    let paths = content.parse_terminated(
        |input: syn::parse::ParseStream<'_>| input.parse::<syn::Path>(),
        syn::Token![,],
    )?;
    Ok(paths.into_iter().collect())
}

fn parse_str_list(meta: &syn::meta::ParseNestedMeta<'_>) -> syn::Result<Vec<String>> {
    let content;
    syn::parenthesized!(content in meta.input);
    let values = content.parse_terminated(
        |input: syn::parse::ParseStream<'_>| input.parse::<LitStr>(),
        syn::Token![,],
    )?;
    Ok(values.into_iter().map(|value| value.value()).collect())
}

fn version(meta: &syn::meta::ParseNestedMeta<'_>) -> syn::Result<u16> {
    let literal: LitInt = meta.value()?.parse()?;
    literal.base10_parse()
}

fn parse_component_args(attr: &Attribute) -> syn::Result<ComponentArgs> {
    let mut args = ComponentArgs::default();
    attr.parse_nested_meta(|meta| {
        let key = meta
            .path
            .get_ident()
            .map(ToString::to_string)
            .unwrap_or_default();
        match key.as_str() {
            "name" => args.name = Some(meta.value()?.parse::<LitStr>()?.value()),
            "view" => args.view = Some(meta.value()?.parse::<LitStr>()?.value()),
            "component_version" => args.component_version = Some(version(&meta)?),
            "state_schema_version" => args.state_schema_version = Some(version(&meta)?),
            "action_schema_version" => args.action_schema_version = Some(version(&meta)?),
            "checker_contract_version" => args.checker_contract_version = Some(version(&meta)?),
            "minimum_protocol_version" => args.minimum_protocol_version = Some(version(&meta)?),
            "refresh_on_promote" => args.refresh_on_promote = true,
            "events" => args.events = parse_path_list(&meta)?,
            "effects" => args.effects = parse_path_list(&meta)?,
            "streams" => {
                meta.parse_nested_meta(|stream| {
                    if !stream.path.is_ident("stream") {
                        return Err(stream.error("expected `stream(...)`"));
                    }
                    args.streams.push(parse_stream(&stream)?);
                    Ok(())
                })?;
            }
            _ => return Err(meta.error("an unknown Live component argument")),
        }
        Ok(())
    })?;
    Ok(args)
}

fn parse_stream(meta: &syn::meta::ParseNestedMeta<'_>) -> syn::Result<StreamArgs> {
    let mut stream = StreamArgs {
        name: String::new(),
        topics: Vec::new(),
        events: Vec::new(),
        targets: vec!["self".to_string()],
        fanout: 1,
        modes: vec!["sse".to_string(), "websocket".to_string()],
        reconnect: None,
        resume_attempts: None,
    };
    meta.parse_nested_meta(|item| {
        let key = item
            .path
            .get_ident()
            .map(ToString::to_string)
            .unwrap_or_default();
        match key.as_str() {
            "name" => stream.name = item.value()?.parse::<LitStr>()?.value(),
            "topics" => stream.topics = parse_str_list(&item)?,
            "events" => stream.events = parse_path_list(&item)?,
            "targets" => stream.targets = parse_str_list(&item)?,
            "fanout" => stream.fanout = item.value()?.parse::<LitInt>()?.base10_parse()?,
            "modes" => stream.modes = parse_str_list(&item)?,
            "reconnect" => stream.reconnect = Some(item.value()?.parse::<LitStr>()?.value()),
            "resume_attempts" => {
                stream.resume_attempts = Some(item.value()?.parse::<LitInt>()?.base10_parse()?);
            }
            _ => return Err(item.error("an unknown stream argument")),
        }
        Ok(())
    })?;
    Ok(stream)
}

fn error(what: impl std::fmt::Display) -> String {
    what.to_string()
}

/// Builds the metadata the macros would generate for one component.
fn build(
    item: &syn::ItemStruct,
    declarations: &Declarations<'_>,
) -> Result<ComponentMetadata, String> {
    let attr = live_args(&item.attrs).ok_or("no `#[live(...)]` attribute")?;
    let args = parse_component_args(attr).map_err(error)?;
    let name = args.name.as_deref().ok_or("no literal `name`")?;
    let view = args.view.as_deref().ok_or("no literal `view`")?;
    let identity =
        ComponentName::parse(name).map_err(|_| format!("`{name}` is not a Live component name"))?;
    let view = ViewName::parse(view).map_err(|_| format!("`{view}` is not a view name"))?;
    let versions = ContractVersions::new(
        args.component_version.unwrap_or(1),
        args.state_schema_version.unwrap_or(1),
        args.action_schema_version.unwrap_or(1),
        args.checker_contract_version.unwrap_or(1),
        args.minimum_protocol_version.unwrap_or(1),
    )
    .map_err(error)?;
    let type_name = item.ident.to_string();
    let actions = actions(declarations.live_impls.get(&type_name))?;
    let fields = fields(item, &actions, declarations)?;
    let mut events = args
        .events
        .iter()
        .map(|path| event_metadata(path, declarations))
        .collect::<Result<Vec<_>, String>>()?;
    let effects = args
        .effects
        .iter()
        .map(|path| effect(path, declarations))
        .collect::<Result<Vec<_>, String>>()?;
    let mut subscriptions = Vec::with_capacity(args.streams.len());
    for stream in &args.streams {
        let (subscription, stream_events) = subscription(stream, declarations)?;
        subscriptions.push(subscription);
        events.extend(stream_events);
    }
    ComponentMetadata::new_with_async_contracts(
        identity,
        view,
        versions,
        fields,
        actions,
        events,
        effects,
        subscriptions,
        args.refresh_on_promote,
    )
    .map_err(error)
}

fn payload_of<'d>(
    path: &syn::Path,
    table: &'d BTreeMap<String, Payload>,
    what: &str,
) -> Result<&'d Payload, String> {
    let name = path
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
        .unwrap_or_default();
    table.get(&name).ok_or_else(|| {
        format!("the {what} payload `{name}` has no `impl {what}PayloadMetadata` in the component with literal constants")
    })
}

fn event_metadata(
    path: &syn::Path,
    declarations: &Declarations<'_>,
) -> Result<EventMetadata, String> {
    let payload = payload_of(path, &declarations.events, "Event")?;
    let name = payload
        .name
        .as_deref()
        .ok_or("an event payload without a literal `NAME`")?;
    let version = payload
        .version
        .ok_or("an event payload without a literal `VERSION`")?;
    EventMetadata::from_source_contract(
        name,
        version,
        payload.contract.as_deref().unwrap_or(name),
        payload.schema.unwrap_or(BrowserPayloadSchema::Json),
    )
    .map_err(error)
}

fn effect(path: &syn::Path, declarations: &Declarations<'_>) -> Result<EffectMetadata, String> {
    let payload = payload_of(path, &declarations.effects, "Effect")?;
    let name = payload
        .name
        .as_deref()
        .ok_or("an effect payload without a literal `NAME`")?;
    let version = payload
        .version
        .ok_or("an effect payload without a literal `VERSION`")?;
    EffectMetadata::from_source_contract(name, version).map_err(error)
}

fn subscription(
    stream: &StreamArgs,
    declarations: &Declarations<'_>,
) -> Result<(SubscriptionMetadata, Vec<EventMetadata>), String> {
    let targets = stream
        .targets
        .iter()
        .map(|target| match target.as_str() {
            "self" => Ok(EventTarget::SelfIsland),
            "parent" => Ok(EventTarget::Parent),
            "child" => Ok(EventTarget::Child),
            "document" => Ok(EventTarget::Document),
            other => Err(format!("`{other}` is not a stream target")),
        })
        .collect::<Result<Vec<_>, String>>()?;
    let events = stream
        .events
        .iter()
        .map(|path| {
            event_metadata(path, declarations)?
                .into_stream_contract(
                    BoundedTargets::new(targets.clone()).map_err(error)?,
                    stream.fanout,
                )
                .map_err(error)
        })
        .collect::<Result<Vec<_>, String>>()?;
    let topics = stream
        .topics
        .iter()
        .map(|topic| TopicName::parse(topic).map_err(error))
        .collect::<Result<Vec<_>, String>>()?;
    let modes = stream
        .modes
        .iter()
        .map(|mode| match mode.as_str() {
            "sse" => Ok(SubscriptionMode::ServerSentEvents),
            "websocket" => Ok(SubscriptionMode::WebSocket),
            other => Err(format!("`{other}` is not a stream mode")),
        })
        .collect::<Result<Vec<_>, String>>()?;
    let reconnect = match stream.reconnect.as_deref() {
        None | Some("resume_or_refresh") => ReconnectPolicy::ResumeOrRefresh {
            maximum_attempts: std::num::NonZeroU8::new(stream.resume_attempts.unwrap_or(4))
                .ok_or("`resume_attempts` must be at least 1")?,
        },
        Some("refresh_on_reconnect") => ReconnectPolicy::RefreshOnReconnect,
        Some(other) => return Err(format!("`{other}` is not a reconnect policy")),
    };
    let subscription = SubscriptionMetadata::new(
        StreamName::parse(&stream.name).map_err(error)?,
        BoundedTopics::new(topics).map_err(error)?,
        BoundedEventNames::new(events.iter().map(|event| event.name().clone()).collect())
            .map_err(error)?,
        SubscriptionModes::new(modes).map_err(error)?,
        reconnect,
    );
    Ok((subscription, events))
}

/// The state category a field's helpers give it, and its model timing.
fn field_kind(field: &syn::Field) -> Result<(FieldCategory, Option<BindingTiming>), String> {
    let mut category = None;
    let mut timing = None;
    for attr in &field.attrs {
        let Some(name) = attr.path().get_ident().map(ToString::to_string) else {
            continue;
        };
        let parsed = match name.as_str() {
            "public" => FieldCategory::Public,
            "locked" => FieldCategory::Locked,
            "server_only" => FieldCategory::ServerOnly,
            "session" => FieldCategory::Session,
            "secret" => FieldCategory::Secret,
            "model" => {
                let (transient, model_timing) = model_args(attr)?;
                timing = Some(model_timing);
                if transient {
                    FieldCategory::Transient
                } else {
                    FieldCategory::Model
                }
            }
            _ => continue,
        };
        if category.replace(parsed).is_some() {
            return Err("a field declares two state categories".to_string());
        }
    }
    Ok((category.unwrap_or(FieldCategory::State), timing))
}

fn model_args(attr: &Attribute) -> Result<(bool, BindingTiming), String> {
    let mut transient = false;
    let mut timing = BindingTiming::Submit;
    if let syn::Meta::List(_) = attr.meta {
        attr.parse_nested_meta(|meta| {
            let key = meta
                .path
                .get_ident()
                .map(ToString::to_string)
                .unwrap_or_default();
            match key.as_str() {
                "transient" => transient = true,
                "immediate" => timing = BindingTiming::Immediate,
                "change" => timing = BindingTiming::Change,
                "blur" => timing = BindingTiming::Blur,
                "submit" => timing = BindingTiming::Submit,
                "debounce" => {
                    let milliseconds: u32 = meta.value()?.parse::<LitInt>()?.base10_parse()?;
                    timing = BindingTiming::debounce(milliseconds)
                        .map_err(|_| meta.error("an unsupported debounce"))?;
                }
                _ => return Err(meta.error("an unknown model helper")),
            }
            Ok(())
        })
        .map_err(error)?;
    }
    Ok((transient, timing))
}

struct UrlArgs {
    key: Option<String>,
    navigate: bool,
    omit_default: bool,
}

fn url_args(field: &syn::Field) -> Result<Option<UrlArgs>, String> {
    let Some(attr) = field.attrs.iter().find(|attr| attr.path().is_ident("url")) else {
        return Ok(None);
    };
    let mut args = UrlArgs {
        key: None,
        navigate: false,
        omit_default: false,
    };
    if let syn::Meta::List(_) = attr.meta {
        attr.parse_nested_meta(|meta| {
            let key = meta
                .path
                .get_ident()
                .map(ToString::to_string)
                .unwrap_or_default();
            match key.as_str() {
                "key" => args.key = Some(meta.value()?.parse::<LitStr>()?.value()),
                "mode" => args.navigate = meta.value()?.parse::<LitStr>()?.value() == "navigate",
                "omit_default" => args.omit_default = true,
                _ => return Err(meta.error("an unknown URL helper")),
            }
            Ok(())
        })
        .map_err(error)?;
    }
    Ok(Some(args))
}

/// The upload policy function a field names.
fn upload_policy(field: &syn::Field) -> Result<Option<syn::Path>, String> {
    let Some(attr) = field
        .attrs
        .iter()
        .find(|attr| attr.path().is_ident("upload"))
    else {
        return Ok(None);
    };
    let mut policy = None;
    attr.parse_nested_meta(|meta| {
        if meta.path.is_ident("policy") {
            policy = Some(meta.value()?.parse::<syn::Path>()?);
            return Ok(());
        }
        Err(meta.error("an unknown upload helper"))
    })
    .map_err(error)?;
    policy
        .map(Some)
        .ok_or_else(|| "`#[upload]` without a policy".to_string())
}

/// The finalize action an upload policy function names with a literal, the
/// only part of the policy the view checker reads.
fn finalize_action(policy: &syn::Path, declarations: &Declarations<'_>) -> Result<String, String> {
    let name = policy
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
        .unwrap_or_default();
    let function = declarations.functions.get(&name).ok_or_else(|| {
        format!("the upload policy `{name}` is not a function the component defines")
    })?;
    let mut finder = FinalizeFinder::default();
    syn::visit::visit_item_fn(&mut finder, function);
    finder
        .action
        .ok_or_else(|| format!("the upload policy `{name}` names no literal `finalize_action`"))
}

#[derive(Default)]
struct FinalizeFinder {
    action: Option<String>,
}

impl<'ast> syn::visit::Visit<'ast> for FinalizeFinder {
    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if call.method == "finalize_action"
            && let Some(syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(text),
                ..
            })) = call.args.first()
        {
            self.action = Some(text.value());
        }
        syn::visit::visit_expr_method_call(self, call);
    }
}

fn fields(
    item: &syn::ItemStruct,
    actions: &[ActionMetadata],
    declarations: &Declarations<'_>,
) -> Result<Vec<FieldMetadata>, String> {
    let Fields::Named(named) = &item.fields else {
        return Err("Live component state must use named fields".to_string());
    };
    let mut out = Vec::with_capacity(named.named.len());
    for field in &named.named {
        let ident = field.ident.as_ref().ok_or("a field without a name")?;
        let name = ident.to_string();
        let name = name.strip_prefix("r#").unwrap_or(&name).to_string();
        let (category, timing) = field_kind(field)?;
        let mut metadata = FieldMetadata::new(
            ModelField::parse(&name).map_err(|_| format!("`{name}` is not a Live field name"))?,
            category,
            state_codec(&field.ty),
            true,
        );
        let model = model_codec(&field.ty);
        if let Some(timing) = timing {
            metadata = metadata
                .with_model_binding(model.clone(), timing)
                .map_err(error)?;
        }
        if category == FieldCategory::Session {
            metadata = metadata
                .with_session_binding(model.clone())
                .map_err(error)?;
        }
        if let Some(url) = url_args(field)? {
            let mode = if url.navigate {
                UrlBindingMode::Navigate
            } else {
                UrlBindingMode::Reflect
            };
            let binding = UrlBinding::new(
                url.key.as_deref().unwrap_or(&name),
                category,
                model.clone(),
                mode,
                url.omit_default,
            )
            .map_err(|_| format!("the URL binding of `{name}` is invalid"))?;
            metadata = metadata.with_url_binding(binding).map_err(error)?;
        }
        if let Some(policy) = upload_policy(field)? {
            let action = finalize_action(&policy, declarations)?;
            let action = ActionName::parse(&action)
                .map_err(|_| format!("`{action}` is not an action name"))?;
            if !actions.iter().any(|known| known.name() == &action) {
                return Err(format!(
                    "the upload policy of `{name}` finalizes with `{}`, which the component does not define",
                    action.as_str()
                ));
            }
            // The view checker reads only that the field takes uploads; the
            // policy's limits and types are the application's business at
            // run time.
            let policy = UploadFieldPolicy::new(
                1,
                1,
                UploadReplacementPolicy::RetirePrevious,
                Vec::new(),
                None,
                UploadScanPolicy::Disabled,
                action,
            )
            .map_err(error)?;
            metadata = metadata.with_upload_policy(policy).map_err(error)?;
        }
        out.push(metadata);
    }
    Ok(out)
}

/// The derive's state codec: by the written type, spaces removed.
fn state_codec(ty: &Type) -> StateCodec {
    match written(ty).as_str() {
        "i64" => StateCodec::I64Decimal,
        "u64" => StateCodec::U64Decimal,
        "Vec<u8>" | "std::vec::Vec<u8>" | "::std::vec::Vec<u8>" => StateCodec::BytesBase64Url,
        _ => StateCodec::Json,
    }
}

/// A path type as the derive compares it: its tokens with no spaces. Only
/// path types matter; any other type gets the default codec.
fn written(ty: &Type) -> String {
    let Type::Path(path) = ty else {
        return String::new();
    };
    if path.qself.is_some() {
        return String::new();
    }
    let mut out = String::new();
    if path.path.leading_colon.is_some() {
        out.push_str("::");
    }
    for (index, segment) in path.path.segments.iter().enumerate() {
        if index > 0 {
            out.push_str("::");
        }
        out.push_str(&segment.ident.to_string());
        if let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments {
            let inner: Vec<String> = arguments
                .args
                .iter()
                .map(|argument| match argument {
                    syn::GenericArgument::Type(ty) => written(ty),
                    _ => "?".to_string(),
                })
                .collect();
            out.push('<');
            out.push_str(&inner.join(","));
            out.push('>');
        }
    }
    out
}

/// The macros' model codec: by the type's last segment and its arguments.
fn model_codec(ty: &Type) -> ModelCodec {
    let Type::Path(path) = ty else {
        return ModelCodec::Json;
    };
    let Some(segment) = path.path.segments.last() else {
        return ModelCodec::Json;
    };
    let ident = segment.ident.to_string();
    let ident = ident.strip_prefix("r#").unwrap_or(&ident);
    match ident {
        "String" => return ModelCodec::String,
        "bool" => return ModelCodec::Boolean,
        "i64" => return ModelCodec::I64,
        "u64" => return ModelCodec::U64,
        "f32" | "f64" => return ModelCodec::F64,
        "Date" => return ModelCodec::Date,
        "OffsetDateTime" => return ModelCodec::DateTime,
        "Uuid" => return ModelCodec::Uuid,
        _ => {}
    }
    let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return ModelCodec::Json;
    };
    let types: Vec<&Type> = arguments
        .args
        .iter()
        .filter_map(|argument| match argument {
            syn::GenericArgument::Type(ty) => Some(ty),
            _ => None,
        })
        .collect();
    match (ident, types.as_slice()) {
        ("Option", [inner]) => model_codec(inner),
        ("Vec", [inner]) => ModelCodec::list(model_codec(inner)),
        ("BTreeMap" | "HashMap", [_key, value]) => ModelCodec::map(model_codec(value)),
        _ => ModelCodec::Json,
    }
}

fn is_option(ty: &Type) -> bool {
    matches!(ty, Type::Path(path) if path.path.segments.last().is_some_and(|segment| segment.ident == "Option"))
}

fn is_authorized_action_reference(ty: &Type) -> bool {
    let Type::Reference(reference) = ty else {
        return false;
    };
    reference.mutability.is_none()
        && matches!(&*reference.elem, Type::Path(path)
            if path.path.segments.last().is_some_and(|segment| segment.ident == "AuthorizedAction"))
}

/// The action metadata every `#[action]` method of the component's
/// `#[live]` impls generates, ordered by name as the macro orders them.
fn actions(impls: Option<&Vec<(&syn::ItemImpl, String)>>) -> Result<Vec<ActionMetadata>, String> {
    let mut actions: BTreeMap<String, ActionMetadata> = BTreeMap::new();
    for (item, _) in impls.into_iter().flatten() {
        for member in &item.items {
            let ImplItem::Fn(method) = member else {
                continue;
            };
            let Some(attr) = method
                .attrs
                .iter()
                .find(|attr| attr.path().is_ident("action"))
            else {
                continue;
            };
            let args = action_args(attr)?;
            let name = args.name.unwrap_or_else(|| {
                let ident = method.sig.ident.to_string();
                ident.strip_prefix("r#").unwrap_or(&ident).to_string()
            });
            let mut fields = Vec::new();
            for (index, input) in method.sig.inputs.iter().skip(1).enumerate() {
                let syn::FnArg::Typed(typed) = input else {
                    return Err(format!("`{name}` takes an argument the macro refuses"));
                };
                if index == 0 && is_authorized_action_reference(&typed.ty) {
                    continue;
                }
                let syn::Pat::Ident(pattern) = &*typed.pat else {
                    return Err(format!("`{name}` takes an argument without a simple name"));
                };
                let wire = pattern.ident.to_string();
                let wire = wire.strip_prefix("r#").unwrap_or(&wire).to_string();
                fields.push(
                    ActionArgumentField::new(
                        ModelField::parse(&wire)
                            .map_err(|_| format!("`{wire}` is not an argument name"))?,
                        model_codec(&typed.ty),
                        !is_option(&typed.ty),
                    )
                    .map_err(error)?,
                );
            }
            let metadata = ActionMetadata::new_with_contract(
                ActionName::parse(&name).map_err(|_| format!("`{name}` is not an action name"))?,
                args.version,
                ActionArgumentSchema::new(fields).map_err(error)?,
                args.authorization,
                args.validation,
                args.transaction,
            )
            .map_err(error)?;
            if actions.insert(name.clone(), metadata).is_some() {
                return Err(format!("`{name}` is registered as an action twice"));
            }
        }
    }
    Ok(actions.into_values().collect())
}

struct ActionArgs {
    name: Option<String>,
    version: u16,
    authorization: AuthorizationRequirement,
    validation: ValidationSelection,
    transaction: TransactionPolicy,
}

fn action_args(attr: &Attribute) -> Result<ActionArgs, String> {
    let mut args = ActionArgs {
        name: None,
        version: 1,
        authorization: AuthorizationRequirement::Public,
        validation: ValidationSelection::None,
        transaction: TransactionPolicy::None,
    };
    if let syn::Meta::List(_) = attr.meta {
        attr.parse_nested_meta(|meta| {
            let key = meta
                .path
                .get_ident()
                .map(ToString::to_string)
                .unwrap_or_default();
            match key.as_str() {
                "name" => args.name = Some(meta.value()?.parse::<LitStr>()?.value()),
                "version" => args.version = version(&meta)?,
                "authorize" => {
                    args.authorization = match meta.value()?.parse::<LitStr>()?.value().as_str() {
                        "current" => AuthorizationRequirement::Current,
                        _ => AuthorizationRequirement::Public,
                    };
                }
                "validate" => {
                    args.validation = match meta.value()?.parse::<LitStr>()?.value().as_str() {
                        "whole" => ValidationSelection::WholeComponent,
                        "arguments" => ValidationSelection::ActionArguments,
                        "all" => ValidationSelection::ComponentAndArguments,
                        _ => ValidationSelection::None,
                    };
                }
                "transaction" => {
                    args.transaction = match meta.value()?.parse::<LitStr>()?.value().as_str() {
                        "required" => TransactionPolicy::Required,
                        _ => TransactionPolicy::None,
                    };
                }
                _ => return Err(meta.error("an unknown action helper")),
            }
            Ok(())
        })
        .map_err(error)?;
    }
    Ok(args)
}
