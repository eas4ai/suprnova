//! Closed server-visible Live directive grammar.

use crate::identity::{ComponentName, ModelField, SignalName};
use crate::metadata::ComponentMetadata;
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
    pub(crate) line: u32,
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

#[cfg(test)]
mod tests {
    use super::{directive_contract, valid_contract_value};

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
