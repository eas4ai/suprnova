//! Contract checks for the checked-in A8/16 snapshot-processing result.

use std::fs;
use std::path::Path;

#[test]
fn named_a8_16_fixture_and_result_stay_inside_iteration_001_budgets() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("benchmarks")
        .join("snapshot-budget-v1.json");
    let bytes = fs::read(path).expect("checked-in benchmark result exists");
    let result: serde_json::Value =
        serde_json::from_slice(&bytes).expect("benchmark result is valid JSON");

    assert_eq!(result["schema_version"], 1);
    assert_eq!(result["workload"], "A8/16");
    assert_eq!(result["state_bytes"], 8 * 1024);
    assert_eq!(result["html_bytes"], 16 * 1024);
    assert!(result["control_overhead_bytes"].as_u64().expect("number") <= 1_024);
    assert!(result["snapshot_overhead_bytes"].as_u64().expect("number") <= 768);
    assert!(result["measured_samples"].as_u64().expect("number") >= 30);
    assert!(result["p95_microseconds"].as_f64().expect("number") <= 500.0);
    assert_eq!(
        result["stages"],
        serde_json::json!(["verify", "hydrate", "dehydrate", "canonicalize", "sign"])
    );
    assert_eq!(result["profile"], "release");
    assert!(
        result["environment"]["cpu_model"]
            .as_str()
            .is_some_and(|value| value != "unavailable")
    );
    assert!(result["environment"]["database"].is_string());
    assert!(result["environment"]["provider_versions"].is_object());
    assert_eq!(
        result["fixture_sha256"]
            .as_str()
            .expect("fixture digest is text")
            .len(),
        64
    );
}
