//! Closed server-visible Live directive grammar.

use crate::canonical::CanonicalValue;
use crate::identity::{ComponentName, ModelField, SignalName};
use crate::limits::InputLimits;
use crate::metadata::{ActionMetadata, ComponentMetadata};
use crate::registry::ComponentRegistry;
use crate::snapshot::state::FieldCategory;
use crate::state::{BindingTiming, UrlBindingMode};

use super::DIRECTIVE_GRAMMAR_VERSION;
use super::branch::DYNAMIC_MARKER;
use super::diagnostic::{DiagnosticCode, DiagnosticCollector, DiagnosticSeverity};
use super::generated_directive_contract::{
    DirectiveContract, DirectiveValue, FRESHNESS_COMBINATIONS, directive_contract,
    valid_directive_scalar_value,
};

pub(crate) struct DirectiveContext<'checker, 'diagnostics> {
    pub(crate) registry: &'checker ComponentRegistry,
    pub(crate) owner: &'checker ComponentMetadata,
    pub(crate) ancestors: &'checker [ComponentName],
    pub(crate) morph_ancestors: &'checker [MorphControlKind],
    pub(crate) tag: &'checker str,
    pub(crate) attributes: &'checker [(String, String)],
    pub(crate) path: &'checker crate::identity::ViewName,
    /// Where the directive's attribute is written.
    pub(crate) line: u32,
    pub(crate) column: u32,
    pub(crate) diagnostics: &'diagnostics mut DiagnosticCollector,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MorphControlKind {
    Preserve,
    Ignore,
    Replace,
    Persist,
    Teleport,
}

const MORPH_CONTROLS: &[&str] = &["preserve", "ignore", "replace", "persist", "teleport"];

fn directive_name(name: &str) -> Option<&str> {
    name.strip_prefix("live:")
        .and_then(|suffix| suffix.split('.').next())
}

pub(crate) fn morph_control_kind(attributes: &[(String, String)]) -> Option<MorphControlKind> {
    attributes
        .iter()
        .find_map(|(name, _)| match directive_name(name) {
            Some("preserve") => Some(MorphControlKind::Preserve),
            Some("ignore") => Some(MorphControlKind::Ignore),
            Some("replace") => Some(MorphControlKind::Replace),
            Some("persist") => Some(MorphControlKind::Persist),
            Some("teleport") => Some(MorphControlKind::Teleport),
            _ => None,
        })
}

pub(crate) fn validate_directive(name: &str, value: &str, context: &mut DirectiveContext<'_, '_>) {
    let Some(suffix) = name.strip_prefix("live:") else {
        return;
    };
    let mut parts = suffix.split('.');
    let directive = parts.next().unwrap_or_default();
    let suffix_parts: Vec<&str> = parts.collect();
    if context.morph_ancestors.contains(&MorphControlKind::Ignore) {
        push_error(context, DiagnosticCode::OwnershipViolation);
        return;
    }
    if matches!(
        directive,
        "mount" | "hydrate" | "dehydrate" | "render" | "destroy" | "teardown"
    ) {
        push_error(context, DiagnosticCode::ForbiddenLifecycle);
        return;
    }
    let Some(contract) = directive_contract(directive) else {
        push_error(context, DiagnosticCode::UnknownDirective);
        return;
    };
    let role = suffix_parts
        .first()
        .is_some_and(|candidate| contract.roles.contains(candidate))
        .then(|| suffix_parts[0]);
    let raw_modifiers = if role.is_some() {
        &suffix_parts[1..]
    } else {
        &suffix_parts[..]
    };
    let Some(modifiers) = normalize_modifiers(contract, raw_modifiers) else {
        push_error(context, DiagnosticCode::InvalidModifier);
        return;
    };
    if modifiers
        .iter()
        .enumerate()
        .any(|(index, modifier)| modifiers[index + 1..].contains(modifier))
    {
        push_error(context, DiagnosticCode::InvalidModifier);
        return;
    }
    if contract.modifier_conflicts.iter().any(|group| {
        modifiers
            .iter()
            .filter(|modifier| group.contains(modifier))
            .count()
            > 1
    }) {
        push_error(context, DiagnosticCode::InvalidModifier);
        return;
    }
    if contract.value != DirectiveValue::Empty && value.contains(DYNAMIC_MARKER) {
        push(
            context,
            DiagnosticCode::DynamicStructureUnproved,
            DiagnosticSeverity::Unproved,
        );
        return;
    }
    if contract.capability.is_some()
        && context.owner.versions().checker_contract() < DIRECTIVE_GRAMMAR_VERSION
    {
        push_error(context, DiagnosticCode::InvalidModifier);
        return;
    }
    if has_conflict(contract, context.attributes) || !valid_contract_value(contract, value) {
        push_error(context, DiagnosticCode::InvalidModifier);
        return;
    }

    match directive {
        "click" | "submit" | "change" | "input" | "keydown" | "init" => {
            validate_action(directive, value, context);
        }
        "model" => validate_model(value, &modifiers, context),
        "error" => validate_error_target(value, context),
        "idle" | "dirty" | "queued" | "loading" | "validating" | "success" | "interrupted"
        | "offline" | "retrying" => {
            if !value.is_empty() {
                validate_feedback_target(value, context);
            }
        }
        "url" => validate_url(value, &modifiers, context),
        "effect" => validate_effect(value, context),
        "on" => validate_event(value, context),
        "upload" => validate_upload(value, role, context),
        "progress" => validate_progress(value, context),
        "stream" => validate_subscription(value, context),
        "preserve" | "ignore" | "replace" | "persist" | "teleport" => {
            validate_morph_control(directive, value, &modifiers, context);
        }
        "navigate" | "prefetch" => validate_navigation(context),
        _ => {}
    }
}

fn validate_upload(value: &str, role: Option<&str>, context: &mut DirectiveContext<'_, '_>) {
    validate_upload_field(value, context);
    if role.is_none()
        && (context.tag != "input"
            || !has_attribute_case_insensitive(context.attributes, "type", "file"))
    {
        push_error(context, DiagnosticCode::AccessibilityViolation);
    }
}

fn validate_progress(value: &str, context: &mut DirectiveContext<'_, '_>) {
    if value.parse::<i64>().is_err() {
        validate_upload_field(value, context);
    }
    let has_progress_semantics =
        context.tag == "progress" || has_attribute(context.attributes, "role", "progressbar");
    let has_accessible_name = has_nonempty_attribute(context.attributes, "aria-label")
        || has_nonempty_attribute(context.attributes, "aria-labelledby");
    if !has_progress_semantics || !has_accessible_name {
        push_error(context, DiagnosticCode::AccessibilityViolation);
    }
}

fn validate_upload_field(value: &str, context: &mut DirectiveContext<'_, '_>) {
    let Ok(field_name) = ModelField::parse(value) else {
        push_error(context, DiagnosticCode::UnknownModel);
        return;
    };
    if let Some(field) = context
        .owner
        .fields()
        .iter()
        .find(|field| field.name() == &field_name)
    {
        if field.upload_policy().is_none() {
            push_error(context, DiagnosticCode::ForbiddenModel);
        }
        return;
    }
    let belongs_to_ancestor = context.ancestors.iter().rev().any(|ancestor| {
        context
            .registry
            .resolve(ancestor)
            .ok()
            .is_some_and(|descriptor| {
                descriptor
                    .metadata()
                    .fields()
                    .iter()
                    .any(|field| field.name() == &field_name && field.upload_policy().is_some())
            })
    });
    push_error(
        context,
        if belongs_to_ancestor {
            DiagnosticCode::OwnershipViolation
        } else {
            DiagnosticCode::UnknownModel
        },
    );
}

fn validate_subscription(value: &str, context: &mut DirectiveContext<'_, '_>) {
    if context
        .owner
        .subscriptions()
        .iter()
        .any(|subscription| subscription.stream().as_str() == value)
    {
        return;
    }
    let belongs_to_ancestor = context.ancestors.iter().rev().any(|ancestor| {
        context
            .registry
            .resolve(ancestor)
            .ok()
            .is_some_and(|descriptor| {
                descriptor
                    .metadata()
                    .subscriptions()
                    .iter()
                    .any(|subscription| subscription.stream().as_str() == value)
            })
    });
    push_error(
        context,
        if belongs_to_ancestor {
            DiagnosticCode::OwnershipViolation
        } else {
            DiagnosticCode::InvalidModifier
        },
    );
}

pub(crate) fn valid_freshness_combination(poll: bool, stream: &str) -> bool {
    FRESHNESS_COMBINATIONS
        .iter()
        .find(|combination| combination.poll == poll && combination.stream == stream)
        .is_some_and(|combination| combination.result != "directive_conflict")
}

fn validate_morph_control(
    directive: &str,
    value: &str,
    modifiers: &[&str],
    context: &mut DirectiveContext<'_, '_>,
) {
    let controls = context
        .attributes
        .iter()
        .filter(|(name, _)| directive_name(name).is_some_and(|name| MORPH_CONTROLS.contains(&name)))
        .count();
    let keys = context
        .attributes
        .iter()
        .filter(|(name, _)| name == "live:key")
        .count();
    if keys != 1 {
        push_error(context, DiagnosticCode::InvalidKey);
    }
    if controls != 1 {
        push_error(context, DiagnosticCode::InvalidModifier);
        return;
    }
    if context
        .attributes
        .iter()
        .any(|(name, _)| name == "live:component")
    {
        push_error(context, DiagnosticCode::OwnershipViolation);
        return;
    }
    if matches!(directive, "persist" | "teleport")
        && context.morph_ancestors.iter().any(|ancestor| {
            matches!(
                ancestor,
                MorphControlKind::Persist | MorphControlKind::Teleport
            )
        })
    {
        push_error(context, DiagnosticCode::OwnershipViolation);
        return;
    }
    let valid_mode = match directive {
        "preserve" => modifiers == ["self"],
        "ignore" => matches!(modifiers, ["children"] | ["subtree"]),
        "replace" => modifiers == ["subtree"],
        "persist" => modifiers.is_empty() && local_identifier(value),
        "teleport" => {
            modifiers.is_empty()
                && value.strip_prefix('#').is_some_and(local_identifier)
                && context
                    .attributes
                    .iter()
                    .find(|(name, _)| name == "id")
                    .is_none_or(|(_, id)| value != format!("#{id}"))
        }
        _ => false,
    };
    if !valid_mode {
        push_error(
            context,
            if directive == "teleport" {
                DiagnosticCode::AccessibilityViolation
            } else {
                DiagnosticCode::InvalidModifier
            },
        );
    }
}

fn normalize_modifiers(
    contract: &'static DirectiveContract,
    segments: &[&str],
) -> Option<Vec<&'static str>> {
    if segments.len() > 16 || segments.iter().any(|segment| segment.is_empty()) {
        return None;
    }
    let mut normalized = Vec::with_capacity(segments.len());
    let mut index = 0;
    while index < segments.len() {
        let maximum = usize::min(3, segments.len() - index);
        let mut matched = None;
        for width in (1..=maximum).rev() {
            let candidate = segments[index..index + width].join(".");
            if let Some(allowed) = contract
                .modifiers
                .iter()
                .copied()
                .find(|allowed| *allowed == candidate)
            {
                matched = Some((allowed, width));
                break;
            }
        }
        let (modifier, width) = matched?;
        normalized.push(modifier);
        index += width;
    }
    Some(normalized)
}

fn has_conflict(contract: &DirectiveContract, attributes: &[(String, String)]) -> bool {
    attributes.iter().any(|(name, _)| {
        name.strip_prefix("live:")
            .and_then(|suffix| suffix.split('.').next())
            .is_some_and(|name| contract.conflicts.contains(&name))
    })
}

fn valid_contract_value(contract: &DirectiveContract, value: &str) -> bool {
    if contract.capability.is_some()
        && let Some(valid) = valid_directive_scalar_value(contract.value, value)
    {
        return valid;
    }
    match contract.value {
        DirectiveValue::Empty => value.is_empty(),
        DirectiveValue::Identifier | DirectiveValue::Field => local_identifier(value),
        DirectiveValue::Action => parse_action_call(value).is_some(),
        DirectiveValue::Target => safe_contract_target(value),
        DirectiveValue::Mapping => valid_mapping(contract.name, value),
        DirectiveValue::Literal => {
            local_identifier(value)
                || matches!(value, "true" | "false" | "null")
                || value.parse::<i64>().is_ok()
        }
    }
}

fn validate_action(directive: &str, value: &str, context: &mut DirectiveContext<'_, '_>) {
    let Some(call) = parse_action_call(value) else {
        push_error(context, DiagnosticCode::InvalidModifier);
        return;
    };
    validate_action_identity(call.name, context);
    if let Some(action) = context
        .owner
        .actions()
        .iter()
        .find(|action| action.name().as_str() == call.name)
    {
        validate_action_arguments(action, &call.arguments, context);
    }
    let accessible = match directive {
        "submit" => context.tag == "form",
        "click" => {
            matches!(context.tag, "button" | "a" | "input" | "select")
                || has_attribute(context.attributes, "role", "button")
        }
        _ => true,
    };
    if !accessible {
        push_error(context, DiagnosticCode::AccessibilityViolation);
    }
}

/// Checks a directive's literal arguments against the action's signature
/// the way the server decodes them: each literal binds the parameter at its
/// position, a trailing optional parameter may be left out, and each value
/// must pass the parameter's registered codec, `null` only where the
/// parameter is optional. One diagnostic names the first mismatch.
fn validate_action_arguments(
    action: &ActionMetadata,
    arguments: &[ActionLiteral],
    context: &mut DirectiveContext<'_, '_>,
) {
    let parameters: Vec<_> = action.arguments().declared().collect();
    let limits = InputLimits::default();
    let fits = arguments.len() <= parameters.len()
        && parameters
            .iter()
            .enumerate()
            .all(|(index, parameter)| match arguments.get(index) {
                None | Some(ActionLiteral::Null) => !parameter.required(),
                Some(literal) => literal
                    .canonical()
                    .is_some_and(|value| parameter.codec().validate(&value, &limits).is_ok()),
            });
    if !fits {
        push_error(context, DiagnosticCode::InvalidActionArguments);
    }
}

// A feedback directive scopes to an action, a bound model field, or the
// island (spec 11, targeted feedback states). A field target names one of
// the owner's model-bound fields; anything else resolves as an action, so
// an unknown name still reports unknown_action.
fn validate_feedback_target(value: &str, context: &mut DirectiveContext<'_, '_>) {
    if let Ok(field_name) = ModelField::parse(value)
        && context
            .owner
            .fields()
            .iter()
            .any(|field| field.name() == &field_name && field.model_codec().is_some())
    {
        return;
    }
    validate_action_identity(value, context);
}

/// Error feedback targets what the browser runtime resolves for it: a field
/// the component declares, or an action of the component or an ancestor, the
/// scope a validation summary names (LIVE-027). A declared field keeps the
/// field rules, so a secret or server-only field is still refused.
fn validate_error_target(value: &str, context: &mut DirectiveContext<'_, '_>) {
    let declares_field = ModelField::parse(value).is_ok_and(|field_name| {
        context
            .owner
            .fields()
            .iter()
            .any(|field| field.name() == &field_name)
    });
    if declares_field {
        validate_field(value, false, context);
        return;
    }
    validate_action_identity(value, context);
}

fn validate_action_identity(value: &str, context: &mut DirectiveContext<'_, '_>) {
    if has_action(context.owner, value) {
        return;
    }
    let belongs_to_ancestor = context.ancestors.iter().rev().any(|ancestor| {
        context
            .registry
            .resolve(ancestor)
            .ok()
            .is_some_and(|descriptor| has_action(descriptor.metadata(), value))
    });
    push_error(
        context,
        if belongs_to_ancestor {
            DiagnosticCode::OwnershipViolation
        } else {
            DiagnosticCode::UnknownAction
        },
    );
}

fn validate_model(value: &str, modifiers: &[&str], context: &mut DirectiveContext<'_, '_>) {
    let Ok(field_name) = ModelField::parse(value) else {
        push_error(context, DiagnosticCode::UnknownModel);
        return;
    };
    let Some(field) = context
        .owner
        .fields()
        .iter()
        .find(|field| field.name() == &field_name)
    else {
        push_error(context, DiagnosticCode::UnknownModel);
        return;
    };
    if !matches!(
        field.category(),
        FieldCategory::Model | FieldCategory::Transient
    ) || field.model_codec().is_none()
    {
        push_error(context, DiagnosticCode::ForbiddenModel);
        return;
    }
    if !matches!(context.tag, "input" | "select" | "textarea") {
        push_error(context, DiagnosticCode::AccessibilityViolation);
    }
    let timing_modifiers: Vec<_> = modifiers
        .iter()
        .copied()
        .filter(|modifier| !matches!(*modifier, "latest" | "serial" | "parallel"))
        .collect();
    if timing_modifiers.len() > 1 {
        push_error(context, DiagnosticCode::InvalidModifier);
        return;
    }
    let Some(modifier) = timing_modifiers.first().copied() else {
        return;
    };
    let direct_match = matches!(
        (modifier, field.binding_timing()),
        ("immediate", Some(BindingTiming::Immediate))
            | ("change", Some(BindingTiming::Change))
            | ("blur", Some(BindingTiming::Blur))
            | ("submit", Some(BindingTiming::Submit))
    );
    let debounce_match = modifier
        .strip_prefix("debounce.")
        .and_then(|value| value.strip_suffix("ms"))
        .and_then(|value| value.parse::<u32>().ok())
        .is_some_and(|millis| {
            field
                .binding_timing()
                .and_then(BindingTiming::debounce_millis)
                == Some(millis)
        });
    if !direct_match && !debounce_match && modifier != "action" {
        push_error(context, DiagnosticCode::InvalidModifier);
    }
}

fn validate_field(value: &str, model_only: bool, context: &mut DirectiveContext<'_, '_>) {
    let Ok(field_name) = ModelField::parse(value) else {
        push_error(context, DiagnosticCode::UnknownModel);
        return;
    };
    let Some(field) = context
        .owner
        .fields()
        .iter()
        .find(|field| field.name() == &field_name)
    else {
        push_error(context, DiagnosticCode::UnknownModel);
        return;
    };
    if model_only && field.model_codec().is_none()
        || matches!(
            field.category(),
            FieldCategory::Secret | FieldCategory::ServerOnly | FieldCategory::Session
        )
    {
        push_error(context, DiagnosticCode::ForbiddenModel);
    }
}

fn validate_url(value: &str, modifiers: &[&str], context: &mut DirectiveContext<'_, '_>) {
    if modifiers.len() != 1 {
        push_error(context, DiagnosticCode::InvalidUrlBinding);
        return;
    }
    let Ok(field_name) = ModelField::parse(value) else {
        push_error(context, DiagnosticCode::InvalidUrlBinding);
        return;
    };
    let binding = context
        .owner
        .fields()
        .iter()
        .find(|field| field.name() == &field_name)
        .and_then(|field| field.url_binding());
    let matches = matches!(
        (modifiers[0], binding.map(|binding| binding.mode())),
        ("reflect", Some(UrlBindingMode::Reflect)) | ("navigate", Some(UrlBindingMode::Navigate))
    );
    if !matches {
        push_error(context, DiagnosticCode::InvalidUrlBinding);
    }
}

fn validate_event(value: &str, context: &mut DirectiveContext<'_, '_>) {
    if !context
        .owner
        .events()
        .iter()
        .any(|event| event.name().as_str() == value)
    {
        push_error(context, DiagnosticCode::UnknownEvent);
    }
}

fn validate_effect(value: &str, context: &mut DirectiveContext<'_, '_>) {
    if !context
        .owner
        .effects()
        .iter()
        .any(|effect| effect.name().as_str() == value)
    {
        push_error(context, DiagnosticCode::UnknownEffect);
    }
}

fn has_action(metadata: &ComponentMetadata, value: &str) -> bool {
    metadata
        .actions()
        .iter()
        .any(|action| action.name().as_str() == value)
}

fn has_attribute(attributes: &[(String, String)], name: &str, value: &str) -> bool {
    attributes
        .iter()
        .any(|(attribute, actual)| attribute == name && actual == value)
}

fn has_attribute_case_insensitive(
    attributes: &[(String, String)],
    name: &str,
    value: &str,
) -> bool {
    attributes
        .iter()
        .any(|(attribute, actual)| attribute == name && actual.eq_ignore_ascii_case(value))
}

fn has_nonempty_attribute(attributes: &[(String, String)], name: &str) -> bool {
    attributes
        .iter()
        .any(|(attribute, value)| attribute == name && !value.trim().is_empty())
}

fn safe_navigation_target(value: &str) -> bool {
    value.starts_with('/')
        && !value.starts_with("//")
        && !value.contains('\\')
        && !value.bytes().any(|byte| byte <= 31 || byte == 127)
}

fn safe_contract_target(value: &str) -> bool {
    local_identifier(value)
        || value.strip_prefix('#').is_some_and(local_identifier)
        || safe_navigation_target(value)
}

fn valid_mapping(directive: &str, value: &str) -> bool {
    let mut count = 0;
    for entry in value.split(',') {
        count += 1;
        if count > 16 {
            return false;
        }
        let Some((name, mapped)) = entry.split_once(':') else {
            return false;
        };
        let valid_name = match directive {
            "signal" => signal_name(name),
            "class" => safe_class_name(name),
            "attr" => safe_attribute_name(name),
            _ => false,
        };
        if mapped.contains(':')
            || !valid_name
            || !(signal_name(mapped) || (directive == "signal" && safe_signal_integer(mapped)))
        {
            return false;
        }
    }
    count > 0
}

fn signal_name(value: &str) -> bool {
    SignalName::parse(value).is_ok()
}

fn safe_attribute_token(value: &str) -> bool {
    let mut bytes = value.bytes();
    value.len() <= 128
        && bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
        && bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
}

fn safe_class_name(value: &str) -> bool {
    let mut bytes = value.bytes();
    value.len() <= 128
        && bytes.next().is_some_and(|byte| byte.is_ascii_alphabetic())
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn safe_attribute_name(value: &str) -> bool {
    let normalized = value.to_ascii_lowercase();
    let module_data_attribute = matches!(normalized.as_str(), "data-action" | "data-controller")
        || normalized.strip_prefix("data-").is_some_and(|suffix| {
            matches!(
                suffix.rsplit_once('-').map(|(_, role)| role),
                Some("class" | "outlet" | "target" | "value")
            )
        });
    safe_attribute_token(value)
        && !normalized.starts_with("on")
        && !normalized.starts_with("data-suprnova-live-")
        && !module_data_attribute
        && !matches!(
            normalized.as_str(),
            "action"
                | "background"
                | "cite"
                | "crossorigin"
                | "data"
                | "formaction"
                | "formenctype"
                | "formmethod"
                | "formtarget"
                | "href"
                | "integrity"
                | "is"
                | "manifest"
                | "method"
                | "nonce"
                | "ping"
                | "poster"
                | "profile"
                | "referrerpolicy"
                | "rel"
                | "src"
                | "srcdoc"
                | "srcset"
                | "style"
                | "target"
                | "type"
                | "usemap"
                | "xlink-href"
        )
}

fn safe_signal_integer(value: &str) -> bool {
    let unsigned = value.strip_prefix('-').unwrap_or(value);
    if unsigned.is_empty()
        || (unsigned.len() > 1 && unsigned.starts_with('0'))
        || unsigned.len() > 16
        || !unsigned.bytes().all(|byte| byte.is_ascii_digit())
    {
        return false;
    }
    value
        .parse::<i64>()
        .is_ok_and(|integer| integer.unsigned_abs() <= 9_007_199_254_740_991)
}

fn validate_navigation(context: &mut DirectiveContext<'_, '_>) {
    let target = context
        .attributes
        .iter()
        .find(|(name, _)| name == "href")
        .map(|(_, value)| value.as_str());
    if context.tag != "a" || target.is_none_or(|target| !safe_navigation_target(target)) {
        push_error(context, DiagnosticCode::AccessibilityViolation);
    }
}

/// The most literal arguments one action directive may carry, the bound
/// the server places on one action's argument schema.
const MAX_ACTION_ARGUMENTS: usize = 128;

/// The longest action directive value, in UTF-16 units: the bound the
/// browser runtime places on every directive value it reads.
const MAX_ACTION_VALUE_UNITS: usize = 2_048;

/// An action directive's value: the action's name and the literal arguments
/// written after it, empty when the value is the bare name.
#[derive(Debug, PartialEq)]
struct ActionCall<'v> {
    name: &'v str,
    arguments: Vec<ActionLiteral>,
}

/// One JSON-style literal argument.
#[derive(Clone, Debug, PartialEq)]
enum ActionLiteral {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
}

impl ActionLiteral {
    /// The value the browser sends for this literal.
    fn canonical(&self) -> Option<CanonicalValue> {
        match self {
            Self::Null => Some(CanonicalValue::Null),
            Self::Bool(value) => Some(CanonicalValue::Bool(*value)),
            Self::Number(value) => CanonicalValue::number(*value).ok(),
            Self::String(value) => Some(CanonicalValue::String(value.clone())),
        }
    }
}

/// Parses `name` or `name(literal, ...)`, at most 2,048 UTF-16 units. The
/// grammar is closed and shared
/// with the browser runtime: literals are JSON numbers, strings in single or
/// double quotes with JSON escapes (plus `\'`), `true`, `false`, and `null`,
/// separated by commas, with JSON whitespace around them. Nothing is
/// evaluated, so the same text always yields the same arguments.
fn parse_action_call(value: &str) -> Option<ActionCall<'_>> {
    if value.encode_utf16().count() > MAX_ACTION_VALUE_UNITS {
        return None;
    }
    let Some(open) = value.find('(') else {
        return action_name(value).then(|| ActionCall {
            name: value,
            arguments: Vec::new(),
        });
    };
    let name = value.get(..open)?;
    let body = value.get(open + 1..)?.strip_suffix(')')?;
    if !action_name(name) {
        return None;
    }
    let mut scanner = LiteralScanner { rest: body };
    let mut arguments = Vec::new();
    scanner.skip_space();
    if scanner.rest.is_empty() {
        return Some(ActionCall { name, arguments });
    }
    loop {
        arguments.push(scanner.literal()?);
        if arguments.len() > MAX_ACTION_ARGUMENTS {
            return None;
        }
        scanner.skip_space();
        if scanner.rest.is_empty() {
            return Some(ActionCall { name, arguments });
        }
        scanner.rest = scanner.rest.strip_prefix(',')?;
        scanner.skip_space();
    }
}

/// An action name in the directive token grammar of the reviewed fixture,
/// the grammar the browser runtime applies: a lowercase letter first, then
/// lowercase letters, digits, `_`, `.`, `:`, or `-`, at most 64 bytes.
fn action_name(name: &str) -> bool {
    valid_directive_scalar_value(DirectiveValue::Action, name) == Some(true)
}

struct LiteralScanner<'t> {
    rest: &'t str,
}

impl LiteralScanner<'_> {
    fn skip_space(&mut self) {
        self.rest = self.rest.trim_start_matches([' ', '\t', '\n', '\r']);
    }

    fn literal(&mut self) -> Option<ActionLiteral> {
        for (keyword, literal) in [
            ("true", ActionLiteral::Bool(true)),
            ("false", ActionLiteral::Bool(false)),
            ("null", ActionLiteral::Null),
        ] {
            if let Some(rest) = self.rest.strip_prefix(keyword) {
                self.rest = rest;
                return Some(literal);
            }
        }
        match self.rest.chars().next()? {
            quote @ ('\'' | '"') => self.string(quote),
            _ => self.number(),
        }
    }

    fn number(&mut self) -> Option<ActionLiteral> {
        let bytes = self.rest.as_bytes();
        let digits = |from: usize| {
            bytes.get(from..).map_or(0, |tail| {
                tail.iter().take_while(|byte| byte.is_ascii_digit()).count()
            })
        };
        let mut end = usize::from(bytes.first() == Some(&b'-'));
        match bytes.get(end) {
            Some(b'0') => end += 1,
            Some(b'1'..=b'9') => end += digits(end),
            _ => return None,
        }
        if bytes.get(end) == Some(&b'.') {
            let fraction = digits(end + 1);
            if fraction == 0 {
                return None;
            }
            end += 1 + fraction;
        }
        if matches!(bytes.get(end), Some(b'e' | b'E')) {
            end += 1;
            if matches!(bytes.get(end), Some(b'+' | b'-')) {
                end += 1;
            }
            let exponent = digits(end);
            if exponent == 0 {
                return None;
            }
            end += exponent;
        }
        let number: f64 = self.rest.get(..end)?.parse().ok()?;
        self.rest = self.rest.get(end..)?;
        number.is_finite().then_some(ActionLiteral::Number(number))
    }

    fn string(&mut self, quote: char) -> Option<ActionLiteral> {
        let mut text = String::new();
        let mut characters = self.rest.char_indices().skip(1);
        while let Some((index, character)) = characters.next() {
            match character {
                _ if character == quote => {
                    self.rest = self.rest.get(index + character.len_utf8()..)?;
                    return Some(ActionLiteral::String(text));
                }
                '\\' => {
                    let (_, escaped) = characters.next()?;
                    text.push(match escaped {
                        '"' | '\'' | '\\' | '/' => escaped,
                        'b' => '\u{8}',
                        'f' => '\u{c}',
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        'u' => {
                            let unit = hex_unit(&mut characters)?;
                            if (0xD800..0xDC00).contains(&unit) {
                                let (_, slash) = characters.next()?;
                                let (_, marker) = characters.next()?;
                                let low = hex_unit(&mut characters)?;
                                if slash != '\\'
                                    || marker != 'u'
                                    || !(0xDC00..0xE000).contains(&low)
                                {
                                    return None;
                                }
                                char::from_u32(0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00))?
                            } else {
                                char::from_u32(unit)?
                            }
                        }
                        _ => return None,
                    });
                }
                _ if u32::from(character) < 0x20 => return None,
                _ => text.push(character),
            }
        }
        None
    }
}

/// Four hexadecimal digits of a `\u` escape as one UTF-16 code unit.
fn hex_unit(characters: &mut impl Iterator<Item = (usize, char)>) -> Option<u32> {
    let mut unit = 0;
    for _ in 0..4 {
        let (_, digit) = characters.next()?;
        unit = unit * 16 + digit.to_digit(16)?;
    }
    Some(unit)
}

fn local_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'_' | b'-' | b'.' | b':')
        })
}

fn push_error(context: &mut DirectiveContext<'_, '_>, code: DiagnosticCode) {
    push(context, code, DiagnosticSeverity::Error);
}

fn push(
    context: &mut DirectiveContext<'_, '_>,
    code: DiagnosticCode,
    severity: DiagnosticSeverity,
) {
    context.diagnostics.push(
        code,
        severity,
        Some(context.path),
        context.line,
        context.column,
        Some(context.owner.identity()),
    );
}

#[cfg(test)]
mod tests {
    use super::{
        ActionLiteral, MAX_ACTION_ARGUMENTS, directive_contract, parse_action_call,
        valid_contract_value,
    };

    fn literal(value: &serde_json::Value) -> ActionLiteral {
        match value {
            serde_json::Value::Null => ActionLiteral::Null,
            serde_json::Value::Bool(value) => ActionLiteral::Bool(*value),
            serde_json::Value::Number(value) => {
                ActionLiteral::Number(value.as_f64().expect("finite vector number"))
            }
            serde_json::Value::String(value) => ActionLiteral::String(value.clone()),
            other => panic!("vector argument is not a literal: {other}"),
        }
    }

    /// The browser parser runs the same vectors, so the checker and the
    /// runtime read every action directive value alike.
    #[test]
    fn action_calls_parse_as_the_shared_vectors_say() {
        let vectors: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/checker/action-call-grammar.json"
        ))
        .expect("action call vectors");
        for case in vectors["valid"].as_array().expect("valid vectors") {
            let value = case["value"].as_str().expect("vector value");
            let call = parse_action_call(value).unwrap_or_else(|| panic!("rejected {value:?}"));
            assert_eq!(Some(call.name), case["name"].as_str(), "{value:?}");
            let expected: Vec<ActionLiteral> = case["arguments"]
                .as_array()
                .map_or_else(Vec::new, |arguments| {
                    arguments.iter().map(literal).collect()
                });
            assert_eq!(call.arguments, expected, "{value:?}");
        }
        for value in vectors["invalid"].as_array().expect("invalid vectors") {
            let value = value.as_str().expect("vector value");
            assert_eq!(parse_action_call(value), None, "accepted {value:?}");
        }
        let limits = &vectors["limits"];
        assert_eq!(
            limits["maximum_arguments"].as_u64(),
            u64::try_from(MAX_ACTION_ARGUMENTS).ok()
        );
        let at_bound = format!("save({})", vec!["1"; MAX_ACTION_ARGUMENTS].join(","));
        assert!(parse_action_call(&at_bound).is_some());
        let past_bound = format!("save({})", vec!["1"; MAX_ACTION_ARGUMENTS + 1].join(","));
        assert_eq!(parse_action_call(&past_bound), None);

        let name_bytes = limits["name"]["maximum_bytes"]
            .as_u64()
            .and_then(|bytes| usize::try_from(bytes).ok())
            .expect("name bound");
        assert!(parse_action_call(&format!("{}(1)", "a".repeat(name_bytes))).is_some());
        assert_eq!(
            parse_action_call(&format!("{}(1)", "a".repeat(name_bytes + 1))),
            None
        );
        // The bound counts UTF-16 units, as the browser measures an attribute
        // value: each `é` is one unit in two bytes.
        let units = limits["value_maximum_utf16_units"]
            .as_u64()
            .and_then(|units| usize::try_from(units).ok())
            .expect("value bound");
        let filler = |units: usize| format!("say('{}')", "é".repeat(units - "say('')".len()));
        assert!(parse_action_call(&filler(units)).is_some());
        assert_eq!(parse_action_call(&filler(units + 1)), None);
    }

    #[test]
    fn promoted_scalar_values_share_one_bounded_lexical_grammar() {
        let oversized = "9".repeat(65);
        let invalid = ["-", "123abc", "Refresh", oversized.as_str()];
        for name in ["upload", "progress", "stream"] {
            let contract = directive_contract(name).expect("promoted directive contract");
            for value in &invalid {
                assert!(
                    !valid_contract_value(contract, value),
                    "{name} accepted {value}"
                );
            }
            assert!(valid_contract_value(contract, "registered_name"));
        }

        let poll = directive_contract("poll").expect("poll contract");
        assert!(valid_contract_value(poll, ""));
        assert!(!valid_contract_value(poll, "registered_name"));

        let progress = directive_contract("progress").expect("progress contract");
        for value in ["0", "-1", "9007199254740991"] {
            assert!(
                valid_contract_value(progress, value),
                "progress rejected {value}"
            );
        }
        for value in ["01", "-0", "9007199254740992"] {
            assert!(
                !valid_contract_value(progress, value),
                "progress accepted {value}"
            );
        }
    }
}
