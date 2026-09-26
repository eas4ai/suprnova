# Feature map: `manual/observability.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 26 checked.

## Rust API: suprnova

### Re-exported from other crates

- [ ] struct `suprnova::telemetry::propagation::HeaderExtractor` re-exports `opentelemetry_http::HeaderExtractor` (feature: `otel`, off by default)
- [ ] struct `suprnova::telemetry::propagation::HeaderInjector` re-exports `opentelemetry_http::HeaderInjector` (feature: `otel`, off by default)

### `suprnova::telemetry::init`

- [ ] fn `suprnova::init_telemetry` · framework/src/telemetry/init.rs:239 (also `suprnova::telemetry::init::init_telemetry`, `suprnova::telemetry::init_telemetry`)
- [ ] struct `suprnova::OtelConfig` · framework/src/telemetry/init.rs:38 (also `suprnova::telemetry::OtelConfig`, `suprnova::telemetry::init::OtelConfig`)
  - Public fields: `endpoint`, `service_name`, `service_version`, `disabled`
  - [ ] fn `suprnova::OtelConfig::from_env` · framework/src/telemetry/init.rs:53
  - [ ] fn `suprnova::OtelConfig::disabled` · framework/src/telemetry/init.rs:76
  - [ ] fn `suprnova::OtelConfig::is_enabled` · framework/src/telemetry/init.rs:87
- [ ] struct `suprnova::TelemetryGuard` · framework/src/telemetry/init.rs:119 (also `suprnova::telemetry::TelemetryGuard`, `suprnova::telemetry::init::TelemetryGuard`)
  - [ ] fn `suprnova::TelemetryGuard::shutdown` · framework/src/telemetry/init.rs:161

### `suprnova::telemetry::metrics::real` (private module; items are public through re-exports)

- [ ] struct `suprnova::CounterHandle` · framework/src/telemetry/metrics.rs:71 (also `suprnova::telemetry::CounterHandle`, `suprnova::telemetry::metrics::CounterHandle`)
  - [ ] fn `suprnova::CounterHandle::inc` · framework/src/telemetry/metrics.rs:75
  - [ ] fn `suprnova::CounterHandle::inc_by` · framework/src/telemetry/metrics.rs:79
  - [ ] fn `suprnova::CounterHandle::inc_with` · framework/src/telemetry/metrics.rs:83
- [ ] struct `suprnova::GaugeHandle` · framework/src/telemetry/metrics.rs:105 (also `suprnova::telemetry::GaugeHandle`, `suprnova::telemetry::metrics::GaugeHandle`)
  - [ ] fn `suprnova::GaugeHandle::set` · framework/src/telemetry/metrics.rs:110
  - [ ] fn `suprnova::GaugeHandle::set_with` · framework/src/telemetry/metrics.rs:114
- [ ] struct `suprnova::HistogramHandle` · framework/src/telemetry/metrics.rs:90 (also `suprnova::telemetry::HistogramHandle`, `suprnova::telemetry::metrics::HistogramHandle`)
  - [ ] fn `suprnova::HistogramHandle::record` · framework/src/telemetry/metrics.rs:94
  - [ ] fn `suprnova::HistogramHandle::record_with` · framework/src/telemetry/metrics.rs:98
- [ ] struct `suprnova::Metrics` · framework/src/telemetry/metrics.rs:47 (also `suprnova::telemetry::Metrics`, `suprnova::telemetry::metrics::Metrics`)
  - [ ] fn `suprnova::Metrics::counter` · framework/src/telemetry/metrics.rs:51
  - [ ] fn `suprnova::Metrics::histogram` · framework/src/telemetry/metrics.rs:57
  - [ ] fn `suprnova::Metrics::gauge` · framework/src/telemetry/metrics.rs:63

### `suprnova::telemetry::propagation`

- [ ] fn `suprnova::telemetry::propagation::extract_w3c_trace_context` · framework/src/telemetry/propagation.rs:57 (feature: `otel`, off by default)
- [ ] fn `suprnova::telemetry::propagation::install_trace_context_propagator` · framework/src/telemetry/propagation.rs:24 (feature: `otel`, off by default; a stub with the same name exists when the feature is off)
- [ ] fn `suprnova::telemetry::propagation::join_upstream_trace` · framework/src/telemetry/propagation.rs:81 (feature: `otel`, off by default; a stub with the same name exists when the feature is off)
