//! E100/1K server transport-owner proof over the production bounded session.

use std::error::Error;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::Serialize;
use suprnova_live_test_support::{AsyncBudgetOwnerEvidence, measure_async_budget_owner};

static TEMP_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ServerEvidence<'a> {
    artifact_sha256: &'a str,
    evidence: &'a AsyncBudgetOwnerEvidence,
    process_id: u32,
    schema_version: u8,
    suite: &'static str,
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("async budget output parent"))?;
    fs::create_dir_all(parent)?;
    let metadata = fs::symlink_metadata(path).ok();
    if metadata.as_ref().is_some_and(fs::Metadata::is_symlink) {
        return Err(std::io::Error::other("async budget output symlink").into());
    }
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = parent.join(format!(".async-server-v1.tmp-{}-{sequence}", process::id()));
    let result = (|| -> Result<(), Box<dyn Error>> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let output = std::env::var_os("SUPRNOVA_LIVE_ASYNC_SERVER_OUTPUT");
    let artifact_sha256 = std::env::var("SUPRNOVA_LIVE_ASYNC_ARTIFACT_SHA256").ok();
    let evidence = measure_async_budget_owner()
        .await
        .map_err(std::io::Error::other)?;
    if cfg!(debug_assertions) && output.is_none() && artifact_sha256.is_none() {
        println!(
            "E100/1K server owner debug contract check transports={} memberships={} dispatches={} queue={}/{} fairness={} mismatches={}; evidence write skipped",
            evidence.physical_document_transports,
            evidence.logical_memberships,
            evidence.dispatches,
            evidence.max_queued_events,
            evidence.max_queued_bytes,
            evidence.fairness_maximum_lead,
            evidence.sequence_mismatches,
        );
        return Ok(());
    }
    let output =
        PathBuf::from(output.ok_or_else(|| std::io::Error::other("async budget output missing"))?);
    let artifact_sha256 = artifact_sha256
        .ok_or_else(|| std::io::Error::other("async budget artifact hash missing"))?;
    if artifact_sha256.len() != 64
        || !artifact_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(std::io::Error::other("async budget artifact hash").into());
    }
    let result = ServerEvidence {
        artifact_sha256: &artifact_sha256,
        evidence: &evidence,
        process_id: process::id(),
        schema_version: 1,
        suite: "E100/1K",
    };
    let mut bytes = serde_json::to_vec_pretty(&result)?;
    bytes.push(b'\n');
    atomic_write(&output, &bytes)?;
    println!(
        "E100/1K server owner transports={} memberships={} dispatches={} queue={}/{} fairness={} mismatches={} output={}",
        evidence.physical_document_transports,
        evidence.logical_memberships,
        evidence.dispatches,
        evidence.max_queued_events,
        evidence.max_queued_bytes,
        evidence.fairness_maximum_lead,
        evidence.sequence_mismatches,
        output.display()
    );
    Ok(())
}
