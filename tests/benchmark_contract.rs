//! Contract checks for checked-in A8/16 and macro-expansion evidence.

use std::fs;
use std::path::Path;

fn benchmark_result(name: &str) -> serde_json::Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("benchmarks")
        .join(name);
    let bytes = fs::read(path).expect("checked-in benchmark result exists");
    serde_json::from_slice(&bytes).expect("benchmark result is valid JSON")
}

#[test]
fn named_a8_16_fixture_and_result_stay_inside_iteration_001_budgets() {
    let result = benchmark_result("snapshot-budget-v1.json");

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

#[test]
fn action_framework_result_covers_the_a8_16_pipeline_under_two_milliseconds() {
    let result = benchmark_result("action-budget-v1.json");

    assert_eq!(result["schema_version"], 1);
    assert_eq!(result["workload"], "A8/16-action-framework");
    assert_eq!(result["state_bytes"], 8 * 1024);
    assert_eq!(result["html_bytes"], 16 * 1024);
    assert!(result["warmup_iterations"].as_u64().expect("number") >= 30);
    assert!(result["measured_samples"].as_u64().expect("number") >= 30);
    assert!(result["p95_microseconds"].as_f64().expect("number") < 2_000.0);
    assert_eq!(
        result["stages"],
        serde_json::json!([
            "parse",
            "verify",
            "claim",
            "hydrate",
            "bind",
            "dispatch",
            "successor_classify"
        ])
    );
    assert_eq!(
        result["excluded"],
        serde_json::json!(["application_action_body", "provider_io", "askama_render"])
    );
    assert_eq!(result["profile"], "release");
    assert_eq!(result["environment"]["classification"], "local_exploratory");
    assert!(
        result["environment"]["cpu_model"]
            .as_str()
            .is_some_and(|value| value != "unavailable")
    );
    assert_eq!(
        result["fixture_sha256"]
            .as_str()
            .expect("fixture digest is text")
            .len(),
        64
    );
}

#[test]
fn macro_expansion_evidence_is_fixed_and_does_not_grow_superlinearly() {
    let result = benchmark_result("expansion-budget-v1.json");
    let fixtures = result["fixtures"].as_array().expect("fixture list");

    assert_eq!(result["schema_version"], 1);
    assert_eq!(result["workload"], "component-expansion");
    assert_eq!(result["environment"]["classification"], "local_exploratory");
    assert_eq!(fixtures.len(), 3);

    let expected_counts = [1_u64, 10, 100];
    for (fixture, expected_count) in fixtures.iter().zip(expected_counts) {
        assert_eq!(fixture["component_count"], expected_count);
        assert!(fixture["expanded_tokens"].as_u64().expect("token count") > 0);
        assert!(fixture["expanded_bytes"].as_u64().expect("byte count") > 0);
        assert!(fixture["cargo_check_milliseconds"].as_u64().is_some());
        assert_eq!(
            fixture["fixture_sha256"]
                .as_str()
                .expect("fixture digest")
                .len(),
            64
        );
    }

    for metric in ["expanded_tokens", "expanded_bytes"] {
        let one = fixtures[0][metric].as_f64().expect("one-component metric");
        let ten = fixtures[1][metric].as_f64().expect("ten-component metric");
        let hundred = fixtures[2][metric]
            .as_f64()
            .expect("hundred-component metric");
        assert!(ten / one <= 12.0, "{metric} grew superlinearly at 10");
        assert!(hundred / ten <= 12.0, "{metric} grew superlinearly at 100");
    }
}
