//! Closed server-visible Live directive grammar.

use crate::identity::{ComponentName, ModelField};
use crate::metadata::ComponentMetadata;
use crate::registry::ComponentRegistry;
use crate::snapshot::state::FieldCategory;
use crate::state::{BindingTiming, UrlBindingMode};

use super::branch::DYNAMIC_MARKER;
use super::diagnostic::{DiagnosticCode, DiagnosticCollector, DiagnosticSeverity};

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
    let modifiers: Vec<&str> = parts.collect();
    if modifiers
        .iter()
        .enumerate()
        .any(|(index, modifier)| modifiers[index + 1..].contains(modifier))
    {
        push_error(context, DiagnosticCode::InvalidModifier);
        return;
    }
    if identity_value(directive) && value.contains(DYNAMIC_MARKER) {
        push(
            context,
            DiagnosticCode::DynamicStructureUnproved,
            DiagnosticSeverity::Unproved,
        );
        return;
    }

    match directive {
        "click" | "submit" | "change" | "input" | "keydown" | "init" => {
            validate_action(directive, value, &modifiers, context);
        }
        "model" => validate_model(value, &modifiers, context),
        "error" => {
            if modifiers.is_empty() {
                validate_field(value, false, context);
            } else {
                push_error(context, DiagnosticCode::InvalidModifier);
            }
        }
        "loading" => {
            if !modifiers.is_empty() {
                push_error(context, DiagnosticCode::InvalidModifier);
            } else if !value.is_empty() {
                validate_action_identity(value, context);
            }
        }
        "url" => validate_url(value, &modifiers, context),
        "effect" => validate_effect(value, &modifiers, context),
        "on" | "stream" => validate_event(value, &modifiers, context),
        "preserve" | "lazy" => {
            if !modifiers.is_empty() || !value.is_empty() {
                push_error(context, DiagnosticCode::InvalidModifier);
            }
        }
        "navigate" => {
            if !modifiers.is_empty() || !safe_target(value) {
                push_error(context, DiagnosticCode::InvalidModifier);
            }
        }
        "signal" | "show" | "toggle" | "class" => {
            if !modifiers.is_empty() || !local_identifier(value) {
                push_error(context, DiagnosticCode::InvalidModifier);
            }
        }
        "poll" => {
            if !modifiers.is_empty()
                || value
                    .parse::<u32>()
                    .ok()
                    .is_none_or(|interval| !(100..=3_600_000).contains(&interval))
            {
                push_error(context, DiagnosticCode::InvalidModifier);
            }
        }
        "component" | "key" => {
            if !modifiers.is_empty() {
                push_error(context, DiagnosticCode::InvalidModifier);
            }
        }
        "mount" | "hydrate" | "dehydrate" | "render" | "destroy" | "teardown" => {
            push_error(context, DiagnosticCode::ForbiddenLifecycle);
        }
        _ => push_error(context, DiagnosticCode::UnknownDirective),
    }
}

fn validate_action(
    directive: &str,
    value: &str,
    modifiers: &[&str],
    context: &mut DirectiveContext<'_, '_>,
) {
    let valid_modifiers = match directive {
        "click" => modifiers
            .iter()
            .all(|modifier| matches!(*modifier, "prevent" | "stop" | "once")),
        "submit" => modifiers.iter().all(|modifier| *modifier == "prevent"),
        "keydown" => modifiers
            .iter()
            .all(|modifier| matches!(*modifier, "enter" | "escape" | "prevent" | "stop")),
        _ => modifiers.is_empty(),
    };
    if !valid_modifiers {
        push_error(context, DiagnosticCode::InvalidModifier);
        return;
    }
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
    if modifiers.len() > 1 {
        push_error(context, DiagnosticCode::InvalidModifier);
        return;
    }
    let Some(modifier) = modifiers.first().copied() else {
        return;
    };
    let matches = matches!(
        (modifier, field.binding_timing()),
        ("immediate", Some(BindingTiming::Immediate))
            | ("change", Some(BindingTiming::Change))
            | ("blur", Some(BindingTiming::Blur))
            | ("submit", Some(BindingTiming::Submit))
            | ("debounce", Some(BindingTiming::Debounce(_)))
    );
    if !matches {
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

fn validate_event(value: &str, modifiers: &[&str], context: &mut DirectiveContext<'_, '_>) {
    if !modifiers.is_empty()
        || !context
            .owner
            .events()
            .iter()
            .any(|event| event.name().as_str() == value)
    {
        push_error(context, DiagnosticCode::UnknownEvent);
    }
}

fn validate_effect(value: &str, modifiers: &[&str], context: &mut DirectiveContext<'_, '_>) {
    if !modifiers.is_empty()
        || !context
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

fn identity_value(directive: &str) -> bool {
    matches!(
        directive,
        "click"
            | "submit"
            | "change"
            | "input"
            | "keydown"
            | "init"
            | "model"
            | "error"
            | "loading"
            | "url"
            | "effect"
            | "on"
            | "stream"
            | "component"
            | "key"
    )
}

fn safe_target(value: &str) -> bool {
    value.starts_with('/') && !value.starts_with("//") && !value.contains(['\r', '\n'])
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
