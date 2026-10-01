//! Standalone launcher for the deterministic Iteration 004 reference host.

use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;

use suprnova_live_test_support::{ReferenceFaultSchedule, ReferenceHost, ReferenceHostConfig};

const DEFAULT_PORT: u16 = 4_174;

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("suprnova-live reference host: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let artifact_root = environment_path("SUPRNOVA_LIVE_ARTIFACT_ROOT")
        .unwrap_or_else(|| crate_root.join("../../browser/dist"));
    let quarantine_root = environment_path("SUPRNOVA_LIVE_QUARANTINE_ROOT")
        .unwrap_or_else(|| crate_root.join("../../target/reference-host-quarantine"));
    let port = std::env::var("SUPRNOVA_LIVE_REFERENCE_PORT")
        .ok()
        .map(|value| value.parse::<u16>())
        .transpose()
        .map_err(|_| "SUPRNOVA_LIVE_REFERENCE_PORT must be a decimal u16".to_owned())?
        .unwrap_or(DEFAULT_PORT);
    let configured_fault = std::env::var("SUPRNOVA_LIVE_REFERENCE_FAULT").ok();
    let fault_schedule = parse_fault_schedule(configured_fault.as_deref())?;
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let mut config = ReferenceHostConfig::new(address, artifact_root, quarantine_root)
        .with_fault_schedule(fault_schedule);
    if let Ok(origin) = std::env::var("SUPRNOVA_LIVE_REFERENCE_STATIC_ORIGIN")
        && !origin.is_empty()
    {
        config = config.with_static_scenario_origin(origin);
    }
    let host = ReferenceHost::start(config).await?;
    println!("{{\"origin\":\"{}\",\"ready\":true}}", host.origin());

    tokio::task::spawn_blocking(|| {
        let mut line = String::new();
        io::stdin().read_line(&mut line)
    })
    .await
    .map_err(|error| format!("shutdown reader task: {error}"))?
    .map_err(|error| format!("shutdown reader: {error}"))?;
    host.shutdown().await
}

fn environment_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn parse_fault_schedule(value: Option<&str>) -> Result<ReferenceFaultSchedule, String> {
    match value {
        None | Some("") | Some("none") => Ok(ReferenceFaultSchedule::None),
        Some("sequence-gap-once") => Ok(ReferenceFaultSchedule::SequenceGapOnce),
        Some("upload-body-interrupted-once") => {
            Ok(ReferenceFaultSchedule::UploadBodyInterruptedOnce)
        }
        Some(_) => {
            Err("SUPRNOVA_LIVE_REFERENCE_FAULT must name a compiled fault schedule".to_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ReferenceFaultSchedule, parse_fault_schedule};

    #[test]
    fn launcher_accepts_only_closed_compiled_fault_names() {
        assert_eq!(
            parse_fault_schedule(None).expect("default schedule"),
            ReferenceFaultSchedule::None
        );
        assert_eq!(
            parse_fault_schedule(Some("sequence-gap-once")).expect("sequence schedule"),
            ReferenceFaultSchedule::SequenceGapOnce
        );
        assert_eq!(
            parse_fault_schedule(Some("upload-body-interrupted-once")).expect("upload schedule"),
            ReferenceFaultSchedule::UploadBodyInterruptedOnce
        );
        assert!(parse_fault_schedule(Some("../../browser-selected")).is_err());
    }
}
