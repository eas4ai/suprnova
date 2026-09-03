//! `suprnova live:check` - check every registered Live view with the
//! integrated checker running inside the application.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::commands::live_tool::{self, Operation, Outcome, Severity};
use crate::ui;

pub fn run(templates: Vec<PathBuf>, allow_unproved: bool, timeout_secs: u64) {
    if let Err(e) = run_inner(templates, allow_unproved, timeout_secs) {
        ui::error(&e);
        std::process::exit(1);
    }
}

/// Resolve template roots: explicit `--templates`, else `askama.toml`'s
/// `[general] dirs`, else Askama's default `templates/`.
pub(crate) fn template_roots(explicit: Vec<PathBuf>) -> Result<Vec<PathBuf>, String> {
    let roots = if !explicit.is_empty() {
        explicit
    } else if let Ok(config) = std::fs::read_to_string("askama.toml") {
        let table: toml::Table =
            toml::from_str(&config).map_err(|e| format!("askama.toml is not valid TOML: {e}"))?;
        let dirs: Vec<PathBuf> = table
            .get("general")
            .and_then(|general| general.get("dirs"))
            .and_then(|dirs| dirs.as_array())
            .map(|dirs| {
                dirs.iter()
                    .filter_map(|dir| dir.as_str().map(PathBuf::from))
                    .collect()
            })
            .unwrap_or_default();
        if dirs.is_empty() {
            vec![PathBuf::from("templates")]
        } else {
            dirs
        }
    } else {
        vec![PathBuf::from("templates")]
    };
    for root in &roots {
        if !root.is_dir() {
            return Err(format!(
                "Template root {} does not exist or is not a directory",
                root.display()
            ));
        }
    }
    Ok(roots)
}

pub(crate) fn require_project() -> Result<(), String> {
    if !Path::new("Cargo.toml").exists() {
        ui::hint("Make sure you're in a Suprnova project root directory.");
        return Err("No Cargo.toml found in the current directory".to_string());
    }
    Ok(())
}

pub(crate) fn explain_helper_failure(kind: &str) -> String {
    match kind {
        "live_tooling_registry_unavailable" => {
            ui::hint("Bind the registry during bootstrap so the helper can see your components:");
            ui::command(
                "suprnova::App::singleton(crate::live::registry().expect(\"Live registry\"));",
            );
            "No Live registry is bound in the application container".to_string()
        }
        other => format!(
            "The application helper failed: {}",
            super::live_tool::display_text(&other.to_string())
        ),
    }
}

fn run_inner(
    templates: Vec<PathBuf>,
    allow_unproved: bool,
    timeout_secs: u64,
) -> Result<(), String> {
    require_project()?;
    let roots = template_roots(templates)?;
    let listed: Vec<String> = roots
        .iter()
        .map(|root| root.display().to_string())
        .collect();
    ui::info(&format!("Checking Live views under {}", listed.join(", ")));
    ui::hint("Building and running the application's Live tooling helper...");
    let mut extra = Vec::with_capacity(roots.len() * 2);
    for root in &roots {
        extra.push("--templates".to_string());
        extra.push(root.display().to_string());
    }
    let session = live_tool::run(Operation::Check, &extra, Duration::from_secs(timeout_secs))
        .map_err(|e| e.to_string())?;
    if session.outcome == Outcome::Failed {
        return Err(explain_helper_failure(
            session.error.as_deref().unwrap_or("unknown failure"),
        ));
    }
    let summary = session
        .summary
        .ok_or_else(|| "The application helper reported no check summary".to_string())?;
    for diagnostic in &session.diagnostics {
        let location = format!(
            "{}:{}:{}",
            super::live_tool::display_text(diagnostic.view.as_deref().unwrap_or("<no view>")),
            diagnostic.line,
            diagnostic.column
        );
        let component = super::live_tool::display_text(
            diagnostic.component.as_deref().unwrap_or("<no component>"),
        );
        let message = format!(
            "{location}: {} [{}] in {component}",
            super::live_tool::display_text(&diagnostic.code),
            diagnostic.severity.as_str()
        );
        match diagnostic.severity {
            Severity::Error => ui::error(&message),
            Severity::Unproved => ui::warning(&message),
        }
    }
    ui::info(&format!(
        "Checked {} component(s) against {} template file(s): {} proved, {} error(s), {} unproved",
        summary.components,
        summary.template_files,
        summary.proved,
        summary.errors,
        summary.unproved
    ));
    if summary.components == 0 {
        ui::warning("No Live components are registered; nothing was checked");
        ui::hint("Run 'suprnova live:make <name>' to scaffold one.");
        return Ok(());
    }
    if summary.errors > 0 {
        return Err(format!(
            "Live check failed with {} error(s)",
            summary.errors
        ));
    }
    if summary.unproved > 0 && !allow_unproved {
        return Err(format!(
            "Live check found {} unproved dynamic structure(s); pass --allow-unproved to accept them",
            summary.unproved
        ));
    }
    if summary.unproved > 0 {
        ui::success("Every Live view checked; unproved dynamic structures were accepted");
    } else {
        ui::success("Every Live view is proved");
    }
    Ok(())
}
