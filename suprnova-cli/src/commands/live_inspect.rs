//! `suprnova live:inspect` - report the application's safe Live runtime,
//! registry, provider, and artifact state.

use std::time::Duration;

use serde::Serialize;

use crate::commands::live_check::{explain_helper_failure, require_project};
use crate::commands::live_tool::{self, ComponentReport, Operation, Outcome, RuntimeReport};
use crate::ui;

#[derive(Serialize)]
struct InspectDocument<'a> {
    framework: &'a str,
    assets: Option<&'a str>,
    runtime: Option<&'a RuntimeReport>,
    components: &'a [ComponentReport],
}

pub fn run(json: bool, timeout_secs: u64) {
    if let Err(e) = run_inner(json, timeout_secs) {
        ui::error(&e);
        std::process::exit(1);
    }
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn run_inner(json: bool, timeout_secs: u64) -> Result<(), String> {
    require_project()?;
    if !json {
        ui::hint("Building and running the application's Live tooling helper...");
    }
    let session = live_tool::run(Operation::Inspect, &[], Duration::from_secs(timeout_secs))
        .map_err(|e| e.to_string())?;
    if session.outcome == Outcome::Failed {
        return Err(explain_helper_failure(
            session.error.as_deref().unwrap_or("unknown failure"),
        ));
    }
    let runtime = session
        .runtime
        .as_ref()
        .ok_or_else(|| "The application helper reported no runtime state".to_string())?;
    if json {
        let document = InspectDocument {
            framework: &session.framework,
            assets: session.assets.as_deref(),
            runtime: Some(runtime),
            components: &session.components,
        };
        let text = serde_json::to_string_pretty(&document)
            .map_err(|e| format!("Failed to encode the report: {e}"))?;
        println!("{text}");
        return Ok(());
    }
    ui::header("Live runtime");
    ui::label_value("Framework", &session.framework);
    ui::label_value(
        "Asset identity",
        session.assets.as_deref().unwrap_or("unavailable"),
    );
    ui::label_value("Browser runtime", &runtime.browser_runtime_version);
    let protocols: Vec<String> = runtime
        .protocol_versions
        .iter()
        .map(u16::to_string)
        .collect();
    ui::label_value("Protocol versions", &protocols.join(", "));
    ui::label_value("Registry bound", yes_no(runtime.registry_bound));
    ui::label_value("Runtime bound", yes_no(runtime.runtime_bound));
    ui::label_value("Components", &runtime.components.to_string());
    ui::label_value(
        "Max request bytes",
        &runtime.config.max_request_bytes.to_string(),
    );
    ui::label_value(
        "Max response bytes",
        &runtime.config.max_response_bytes.to_string(),
    );
    ui::label_value(
        "Max context lifetime (ms)",
        &runtime.config.max_context_lifetime_ms.to_string(),
    );
    let host = &runtime.upload_host;
    ui::label_value(
        "Upload host",
        &if host.installed {
            format!(
                "installed (finalizer: {}, direct provider: {}, scanner: {}, validator: {})",
                yes_no(host.finalizer),
                yes_no(host.direct_provider),
                yes_no(host.scanner),
                yes_no(host.application_validator)
            )
        } else {
            "not installed".to_string()
        },
    );
    if let Some(ready) = &runtime.readiness {
        let missing: Vec<&str> = [
            ("clock", ready.clock),
            ("random", ready.random),
            ("key ring", ready.key_ring),
            ("ledger", ready.ledger),
            ("promotion", ready.promotion),
            ("execution", ready.execution),
            ("context validator", ready.context_validator),
            ("host ports", ready.host_ports),
            ("upload ports", ready.upload_ports),
            ("upload services", ready.upload_services),
            ("mount catalog", ready.mount_catalog),
            ("response and cancellation", ready.response_and_cancellation),
            ("subscription ports", ready.subscription_ports),
            ("async state", ready.async_state),
        ]
        .into_iter()
        .filter(|(_, present)| !*present)
        .map(|(name, _)| name)
        .collect();
        ui::label_value(
            "Runtime services",
            &if missing.is_empty() {
                "all assembled".to_string()
            } else {
                format!("missing: {}", missing.join(", "))
            },
        );
    }
    if !session.components.is_empty() {
        ui::header("Components");
        for component in &session.components {
            ui::label_value(&component.name, &component.view);
            ui::hint(&format!(
                "fields {} (uploads {}), actions {}, events {}, effects {}, subscriptions {}, min protocol {}, digest {}",
                component.fields,
                component.upload_fields,
                component.actions,
                component.events,
                component.effects,
                component.subscriptions,
                component.minimum_protocol,
                component.contract_digest
            ));
        }
    }
    ui::br();
    Ok(())
}
