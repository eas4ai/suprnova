//! Closed server-visible Live directive grammar.

use crate::identity::{ComponentName, ModelField};
use crate::metadata::ComponentMetadata;
use crate::registry::ComponentRegistry;
use crate::snapshot::state::FieldCategory;
use crate::state::{BindingTiming, UrlBindingMode};

use super::branch::DYNAMIC_MARKER;
use super::diagnostic::{DiagnosticCode, DiagnosticCollector, DiagnosticSeverity};
use super::generated_directive_contract::{DirectiveContract, DirectiveValue, directive_contract};

pub(crate) struct DirectiveContext<'checker, 'diagnostics> {
    pub(crate) registry: &'checker ComponentRegistry,
    pub(crate) owner: &'checker ComponentMetadata,
    pub(crate) ancestors: &'checker [ComponentName],
    pub(crate) tag: &'checker str,
    pub(crate) attributes: &'checker [(String, String)],
    pub(crate) path: &'checker crate::identity::ViewName,
    pub(crate) line: u32,
    pub(crate) diagnostics: &'diagnostics mut DiagnosticCollector,
}

pub(crate) fn validate_directive(name: &str, value: &str, context: &mut DirectiveContext<'_, '_>) {
    let Some(suffix) = name.strip_prefix("live:") else {
        return;
    };
    let mut parts = suffix.split('.');
    let directive = parts.next().unwrap_or_default();
    let raw_modifiers: Vec<&str> = parts.collect();
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
    let Some(modifiers) = normalize_modifiers(contract, &raw_modifiers) else {
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
    if contract.value != DirectiveValue::Empty && value.contains(DYNAMIC_MARKER) {
        push(
            context,
            DiagnosticCode::DynamicStructureUnproved,
            DiagnosticSeverity::Unproved,
        );
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
        "error" => validate_field(value, false, context),
        "idle" | "dirty" | "queued" | "loading" | "validating" | "success" | "interrupted"
        | "offline" | "retrying" => {
            if !value.is_empty() {
                validate_action_identity(value, context);
            }
        }
        "url" => validate_url(value, &modifiers, context),
        "effect" => validate_effect(value, context),
        "on" => validate_event(value, context),
        "navigate" | "prefetch" => validate_navigation(context),
        _ => {}
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
    match contract.value {
        DirectiveValue::Empty => value.is_empty(),
        DirectiveValue::Identifier | DirectiveValue::Field | DirectiveValue::Action => {
            local_identifier(value)
        }
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
    validate_action_identity(value, context);
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
    let mut bytes = value.bytes();
    value.len() <= 128
        && bytes.next().is_some_and(|byte| byte.is_ascii_alphabetic())
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn safe_class_name(value: &str) -> bool {
    signal_name(value)
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
    signal_name(value)
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
        1,
        Some(context.owner.identity()),
    );
}
