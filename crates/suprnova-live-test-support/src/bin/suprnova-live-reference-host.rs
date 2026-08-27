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
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let host = ReferenceHost::start(
        ReferenceHostConfig::new(address, artifact_root, quarantine_root)
            .with_fault_schedule(ReferenceFaultSchedule::None),
    )
    .await?;
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
