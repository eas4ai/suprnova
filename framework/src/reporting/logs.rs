use super::{Reporting, ReportingConfig, ReportingEvent, ReportingKind};
use crate::{App, Log, LogLevel, MessageLogged};
use serde_json::{Value, json};
use std::sync::Once;

static INSTALL: Once = Once::new();

pub(crate) fn install() {
    INSTALL.call_once(|| Log::listen(record));
}

fn record(record: &MessageLogged) {
    let (breadcrumb_level, log_level) = match record.level {
        LogLevel::Emergency | LogLevel::Alert | LogLevel::Critical => ("fatal", "fatal"),
        LogLevel::Error => ("error", "error"),
        LogLevel::Warning => ("warning", "warn"),
        LogLevel::Notice | LogLevel::Info => ("info", "info"),
        LogLevel::Debug => ("debug", "debug"),
    };
    let trace_id = super::scope::record_log(breadcrumb_level, &record.message, &record.context);
    let config = App::get::<ReportingConfig>().unwrap_or_default();
    if !config.logs.is_some_and(|minimum| record.level <= minimum) {
        return;
    }
    let mut context = Value::Object(record.context.clone());
    crate::error::redaction::scrub(&mut context, config.send_pii);
    let attributes: serde_json::Map<_, _> = context
        .as_object()
        .into_iter()
        .flatten()
        .map(|(name, value)| {
            let (kind, value) = match value {
                Value::Bool(_) => ("boolean", value.clone()),
                Value::Number(number) if number.is_i64() => ("integer", value.clone()),
                Value::Number(_) => ("double", value.clone()),
                Value::String(_) => ("string", value.clone()),
                _ => ("string", json!(value.to_string())),
            };
            (name.clone(), json!({"type":kind,"value":value}))
        })
        .collect();
    let mut payload = json!({"level":log_level,"body":record.message,"timestamp":super::scope::timestamp(crate::clock::now().into()),"attributes":attributes});
    if let Some(id) = trace_id {
        payload["trace_id"] = json!(id);
    }
    crate::error::redaction::scrub(&mut payload, config.send_pii);
    let _ = Reporting::send(ReportingEvent {
        kind: ReportingKind::Log,
        payload,
    });
}
