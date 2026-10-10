//! Checked U4/16 benchmark evidence and runner wiring contract.

use std::fs;
use std::path::PathBuf;

use serde_json::Value;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn pointer<'a>(value: &'a Value, path: &str) -> &'a Value {
    value
        .pointer(path)
        .unwrap_or_else(|| panic!("missing upload budget field {path}"))
}

#[test]
fn checked_upload_budget_locks_workload_bounds_and_exclusions() {
    let bytes = fs::read(root().join("browser/benchmarks/baselines/upload-budget-v1.json"))
        .expect("checked upload budget evidence");
    let value: Value = serde_json::from_slice(&bytes).expect("upload budget JSON");
    let encoded = String::from_utf8(bytes).expect("UTF-8 upload budget JSON");

    assert!(!encoded.contains("\"handle\""));
    assert!(!encoded.contains("018f47c1-2af0-7cc4-"));
    assert!(!encoded.contains("benchmark-grant-"));

    assert_eq!(pointer(&value, "/schemaVersion"), 1);
    assert_eq!(pointer(&value, "/workload"), "U4/16");
    assert!(pointer(&value, "/qualifiedBaseline").is_null());
    for owner in ["browser", "server"] {
        let prefix = format!("/exploratoryReference/{owner}");
        assert_eq!(pointer(&value, &format!("{prefix}/workload/files")), 4);
        assert_eq!(
            pointer(&value, &format!("{prefix}/workload/fileBytes")),
            16 * 1024 * 1024
        );
        assert_eq!(
            pointer(&value, &format!("{prefix}/workload/chunkBytes")),
            256 * 1024
        );
        assert_eq!(
            pointer(&value, &format!("{prefix}/workload/activeTransfers")),
            4
        );
        assert!(
            pointer(&value, &format!("{prefix}/methodology/measuredSamples"))
                .as_u64()
                .is_some_and(|samples| samples >= 30)
        );
        assert!(
            pointer(&value, &format!("{prefix}/methodology/warmupIterations"))
                .as_u64()
                .is_some_and(|warmups| warmups >= 1)
        );
    }
    assert_eq!(
        pointer(
            &value,
            "/exploratoryReference/browser/bounds/maxChunksPerActiveTransfer"
        ),
        2
    );
    assert_eq!(
        pointer(
            &value,
            "/exploratoryReference/browser/bounds/maxManagerOwnedBytes"
        ),
        256 * 1024
    );
    assert_eq!(
        pointer(
            &value,
            "/exploratoryReference/server/bounds/maxManagerOwnedBytes"
        ),
        512 * 1024
    );
    assert_eq!(
        pointer(
            &value,
            "/exploratoryReference/server/bounds/maxControlP95Microseconds"
        ),
        2_000
    );
    for excluded in ["bodyIo", "provider", "scanner", "applicationValidation"] {
        assert_eq!(
            pointer(
                &value,
                &format!("/exploratoryReference/server/measurements/excludedCalls/{excluded}")
            ),
            0
        );
    }
    assert_eq!(
        pointer(
            &value,
            "/exploratoryReference/server/methodology/independentRuns"
        ),
        1
    );
    let server_runs = pointer(&value, "/exploratoryReference/server/runs")
        .as_array()
        .expect("server process runs");
    assert_eq!(server_runs.len(), 1);
    assert_eq!(
        pointer(&value, "/exploratoryReference/server/runs/0/artifactSha256"),
        pointer(&value, "/exploratoryReference/artifact/sha256")
    );
    assert_eq!(
        pointer(&value, "/exploratoryReference/server/runs/0/runIndex"),
        1
    );
    assert!(
        pointer(&value, "/exploratoryReference/server/runs/0/processId")
            .as_u64()
            .is_some_and(|process_id| process_id > 0)
    );
    let hash = pointer(&value, "/exploratoryReference/artifact/sha256")
        .as_str()
        .expect("artifact hash");
    assert_eq!(hash.len(), 64);
    assert!(hash.bytes().all(|byte| byte.is_ascii_hexdigit()));
}

#[test]
fn upload_budget_is_an_on_demand_tool_with_a_release_safe_runner() {
    let cargo = fs::read_to_string(root().join("Cargo.toml")).expect("Cargo manifest");
    let package = fs::read_to_string(root().join("browser/package.json")).expect("browser package");
    let gate = fs::read_to_string(root().join("scripts/gate.sh")).expect("project gate");
    let runner = fs::read_to_string(root().join("scripts/run-upload-budget.sh"))
        .expect("upload budget runner");

    assert!(cargo.contains("name = \"upload_framework_budget\""));
    assert!(package.contains("\"budget:upload\""));
    assert!(
        !gate.contains("run-upload-budget.sh"),
        "the upload budget is an on-demand tool, not a gate phase"
    );
    assert!(runner.contains("SUPRNOVA_LIVE_S1_DEDICATED"));
    assert!(runner.contains("SUPRNOVA_LIVE_B1_DEDICATED"));
    assert!(runner.contains("run-upload-server-processes.mjs"));
}
