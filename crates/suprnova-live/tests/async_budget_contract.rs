//! E100/1K and R100 benchmark wiring and server-owner contract.

use std::fs;
use std::path::Path;

use suprnova_live_test_support::measure_async_budget_owner;

fn repository_file(path: &str) -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(path))
        .unwrap_or_else(|error| panic!("read {path}: {error}"))
}

#[tokio::test]
async fn server_owner_proves_exact_multiplexed_workload() {
    let evidence = measure_async_budget_owner()
        .await
        .expect("measure production bounded document owner");
    assert_eq!(evidence.provider_path, "BoundedDocumentTransportSession");
    assert_eq!(evidence.physical_document_transports, 1);
    assert_eq!(evidence.logical_memberships, 100);
    assert_eq!(evidence.dispatches, 1_100);
    assert_eq!(evidence.final_current_subscriptions, 100);
    assert_eq!(evidence.sequence_mismatches, 0);
    assert!(evidence.fairness_maximum_lead <= 1);
    assert!(evidence.max_queued_events <= 64);
    assert!(evidence.max_queued_bytes <= 256 * 1_024);
}

#[test]
fn async_benchmark_is_on_demand_and_gate_never_blanket_denies_warnings() {
    let cargo = repository_file("Cargo.toml");
    let package = repository_file("browser/package.json");
    let gate = repository_file("scripts/gate.sh");
    let runner = repository_file("scripts/run-async-budget.sh");
    assert!(cargo.contains("name = \"async_framework_budget\""));
    assert!(package.contains("\"budget:async\": \"node scripts/run-async-budget.mjs\""));
    assert!(
        !gate.contains("run-async-budget.sh"),
        "the async budget is an on-demand tool, not a gate phase"
    );
    assert!(runner.contains("SUPRNOVA_LIVE_B1_DEDICATED"));
    assert!(!gate.contains("-D warnings"));
    assert!(!runner.contains("-D warnings"));
}
