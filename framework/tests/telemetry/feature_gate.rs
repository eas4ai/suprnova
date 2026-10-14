//! Compile-time verification that the public telemetry API is identical
//! across both feature configurations.
//!
//! These tests don't assert runtime behavior (the Metrics facade is a
//! no-op until `init_telemetry` installs providers). They exist so that
//! a future refactor cannot accidentally break the no-feature default
//! build, which is what the vast majority of consumers will use.

use suprnova::{Metrics, OtelConfig};

#[test]
fn metrics_api_compiles_and_noops_before_init() {
    let c = Metrics::counter("gate.test.requests.total");
    c.inc();
    c.inc_by(10);
    c.inc_with(&[("env", "test"), ("region", "us-east-1")]);

    let h = Metrics::histogram("gate.test.request.duration");
    h.record(1.5);
    h.record_with(2.5, &[("route", "/health")]);

    let g = Metrics::gauge("gate.test.queue.depth");
    g.set(0.0);
    g.set_with(7.0, &[("queue", "default")]);
}

/// Attribute values with a type compile with the feature and without
/// it, as the values that are a text do.
#[test]
fn typed_attribute_values_compile_and_noop_before_init() {
    use suprnova::AttrValue;

    let c = Metrics::counter("gate.test.typed.requests");
    c.inc_with(&[("http.response.status_code", 404)]);
    c.inc_with(&[("error", true)]);
    c.inc_with(&[
        ("route", AttrValue::from("/health")),
        ("http.response.status_code", AttrValue::from(200_u16)),
        ("error", AttrValue::from(false)),
    ]);

    let route = String::from("/posts/{id}");
    let h = Metrics::histogram("gate.test.typed.duration");
    h.record_with(2.5, &[("route", &route)]);

    let g = Metrics::gauge("gate.test.typed.depth");
    g.set_with(7.0, &[("ratio", 0.5)]);
}

#[test]
fn otel_config_api_compiles() {
    let cfg = OtelConfig::disabled();
    assert!(!cfg.is_enabled());

    // `from_env` is safe to call regardless of what's in the env; it
    // never panics, and `is_enabled` reflects whatever it found.
    let env_cfg = OtelConfig::from_env();
    // No assertion on `is_enabled()` here - the test runner may or may
    // not have OTEL_EXPORTER_OTLP_ENDPOINT set.
    let _ = env_cfg.service_name;
    let _ = env_cfg.service_version;
}
