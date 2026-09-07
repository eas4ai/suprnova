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

#[test]
fn render_cache_budget_result_holds_the_c64_allocation_and_copy_bounds() {
    let result = benchmark_result("render-cache-budget-v1.json");

    assert_eq!(result["schema_version"], 1);
    assert_eq!(result["profile"], "release");
    assert_eq!(result["environment"]["classification"], "local_exploratory");
    assert!(
        result["environment"]["cpu_model"]
            .as_str()
            .is_some_and(|value| value != "unavailable")
    );

    let c64 = &result["c64"];
    assert_eq!(c64["body_bytes"], 65_536);
    assert_eq!(c64["dependencies"], 12);
    assert!(c64["warmup_iterations"].as_u64().expect("number") >= 30);
    assert!(c64["measured_samples"].as_u64().expect("number") >= 30);
    assert!(c64["allocations_max"].as_u64().expect("number") <= 4);
    assert_eq!(c64["allocations_cap"], 4);
    assert_eq!(c64["body_shared"], true);
    assert_eq!(
        c64["seed_deadline"]["body_shared"], true,
        "the seeded fixture shares its stored body too, reported on its own"
    );
    assert_eq!(c64["p95_cap_microseconds"], 250.0);
    assert!(c64["p95_microseconds"].as_f64().is_some());
    assert!(
        c64["not_modified"]["allocations_max"]
            .as_u64()
            .expect("number")
            <= 4
    );
    assert!(
        c64["seed_deadline"]["allocations_max"]
            .as_u64()
            .expect("number")
            <= 4,
        "the one entry shape that forms a header value per request stays inside the cap"
    );

    let composite = &result["c64_plus_4"];
    assert_eq!(composite["shell_bytes"], 65_536);
    assert_eq!(composite["slots"], 4);
    assert_eq!(composite["slot_bytes"], 4_096);
    assert!(composite["copy_ratio_max"].as_f64().expect("number") <= 2.0);
    assert_eq!(composite["copy_ratio_cap"], 2.0);
    assert_eq!(composite["p95_cap_microseconds"], 2_000.0);
    assert!(composite["p95_microseconds"].as_f64().is_some());
}

#[test]
fn the_render_cache_budget_is_an_on_demand_tool_and_never_a_gate_step() {
    let live_gate =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/gate.sh"))
            .expect("the Live gate script exists");
    assert!(
        !live_gate.contains("run-render-cache-budget.sh"),
        "the render-cache budget is an on-demand tool, not a gate phase"
    );

    let steps = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/gate-steps.json"),
    )
    .expect("the repository gate steps exist");
    assert!(
        !steps.contains("render_cache_budget") && !steps.contains("render-cache-budget"),
        "the repository gate must not run the budget"
    );

    let manifest = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
        .expect("the Live manifest exists");
    assert!(
        manifest.contains("name = \"render_cache_budget\""),
        "the budget bench is a registered target"
    );

    let runner = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/run-render-cache-budget.sh"),
    )
    .expect("the runner exists");
    assert!(
        runner.contains("SUPRNOVA_LIVE_S1_CPUSET") && !runner.contains("-D warnings"),
        "the runner pins the S1 processor set and never denies warnings wholesale"
    );
}
