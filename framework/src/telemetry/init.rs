//! `init_telemetry` - the unified entry point that wires `tracing` and
//! (optionally) the OpenTelemetry SDK pipelines into a single subscriber.
//!
//! See [`crate::telemetry`] for the high-level design.

use crate::logging::config::LogConfig;
use crate::logging::init::install_base_subscriber;
use std::env;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Environment-driven OpenTelemetry configuration.
///
/// This struct models only the handful of vars Suprnova reads itself to
/// decide *whether*, *where to* and *as whom* to export:
///
/// | Field              | Env var                               | Default                         |
/// |--------------------|---------------------------------------|---------------------------------|
/// | `endpoint`         | `OTEL_EXPORTER_OTLP_ENDPOINT`         | _unset_ → telemetry disabled    |
/// | `traces_endpoint`  | `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT`  | `/v1/traces` under `endpoint`   |
/// | `metrics_endpoint` | `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT` | `/v1/metrics` under `endpoint`  |
/// | `logs_endpoint`    | `OTEL_EXPORTER_OTLP_LOGS_ENDPOINT`    | `/v1/logs` under `endpoint`     |
/// | `service_name`     | `OTEL_SERVICE_NAME`                   | `"suprnova"`                    |
/// | `service_version`  | `OTEL_SERVICE_VERSION`                | `CARGO_PKG_VERSION` at compile  |
/// | `disabled`         | `OTEL_SDK_DISABLED` (case-insensitive `true` / `1`) | `false`           |
///
/// Telemetry is "enabled" when `endpoint` is set to more than blanks
/// **and** `disabled` is `false`. The endpoint is read once at process
/// start; runtime mutation is unsupported.
///
/// **The rest of the standard OTLP knobs are read by the SDK, not here.**
/// `OTEL_EXPORTER_OTLP_HEADERS` (collector auth), `_PROTOCOL`, `_TIMEOUT`,
/// and `_COMPRESSION` are consumed directly by the `opentelemetry-otlp`
/// exporter builders when `init_telemetry` calls `.build()` - so operators
/// get the standard behavior without Suprnova re-modeling each one. Of the
/// compressions, `gzip` is compiled in. `zstd` is not, and asking for it
/// leaves the signals out, with the reason in the log.
///
/// # Where each signal is sent
///
/// The endpoint is a base URL, and each signal is sent to its own path
/// under it: `/v1/traces`, `/v1/metrics`, `/v1/logs`. A signal has a
/// variable of its own, `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT`,
/// `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT` and
/// `OTEL_EXPORTER_OTLP_LOGS_ENDPOINT`, whose URL is used as it is
/// written, with no path added. That is the rule of the OTLP
/// specification, and it is how traces go to one collector and metrics
/// to another. The base endpoint is what turns telemetry on: with it not
/// set, the endpoint of a signal has no effect.
///
/// An endpoint is a URL with the scheme `http` or `https` and a host. A
/// signal whose endpoint is none is left out and reported in the log, and
/// the other signals are exported.
#[derive(Debug, Clone)]
pub struct OtelConfig {
    /// OTLP collector base URL (e.g. `http://localhost:4318`).
    pub endpoint: Option<String>,
    /// The URL the traces are sent to, used as it is written. `None`
    /// sends them to `/v1/traces` under [`Self::endpoint`].
    pub traces_endpoint: Option<String>,
    /// The URL the metrics are sent to, used as it is written. `None`
    /// sends them to `/v1/metrics` under [`Self::endpoint`].
    pub metrics_endpoint: Option<String>,
    /// The URL the logs are sent to, used as it is written. `None`
    /// sends them to `/v1/logs` under [`Self::endpoint`].
    pub logs_endpoint: Option<String>,
    /// `service.name` resource attribute reported on every span / metric / log.
    pub service_name: String,
    /// `service.version` resource attribute.
    pub service_version: String,
    /// Honors the standard `OTEL_SDK_DISABLED=true` kill switch.
    pub disabled: bool,
}

impl OtelConfig {
    /// Read configuration from the environment. Never panics; missing
    /// vars fall back to defaults and the caller can inspect
    /// [`Self::is_enabled`] to decide whether to install exporters.
    pub fn from_env() -> Self {
        let url = |name: &str| {
            env::var(name)
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };
        let endpoint = url("OTEL_EXPORTER_OTLP_ENDPOINT");
        let service_name = env::var("OTEL_SERVICE_NAME").unwrap_or_else(|_| "suprnova".to_string());
        let service_version = env::var("OTEL_SERVICE_VERSION")
            .unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_string());
        let disabled = parse_sdk_disabled(env::var("OTEL_SDK_DISABLED").ok().as_deref());
        Self {
            endpoint,
            traces_endpoint: url("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT"),
            metrics_endpoint: url("OTEL_EXPORTER_OTLP_METRICS_ENDPOINT"),
            logs_endpoint: url("OTEL_EXPORTER_OTLP_LOGS_ENDPOINT"),
            service_name,
            service_version,
            disabled,
        }
    }

    /// Sentinel value: telemetry is explicitly off. Used by
    /// [`crate::logging::init_subscriber`] for the legacy non-OTel path.
    pub fn disabled() -> Self {
        Self {
            endpoint: None,
            traces_endpoint: None,
            metrics_endpoint: None,
            logs_endpoint: None,
            service_name: "suprnova".to_string(),
            service_version: env!("CARGO_PKG_VERSION").to_string(),
            disabled: true,
        }
    }

    /// Telemetry is enabled iff an endpoint is configured **and** the
    /// `OTEL_SDK_DISABLED` kill switch is not set. An endpoint of blanks
    /// is no endpoint.
    pub fn is_enabled(&self) -> bool {
        self.endpoint
            .as_deref()
            .is_some_and(|endpoint| !endpoint.trim().is_empty())
            && !self.disabled
    }
}

/// The URL each signal is sent to. See [`OtelConfig`].
///
/// The exporter uses an endpoint it is given in code as it is written,
/// and adds the path of the signal to one it reads from the environment
/// itself. So the path is added here. An exporter that is given the
/// base URL alone posts every signal to the root of the collector, which
/// has nothing there.
#[cfg(any(test, feature = "otel"))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct SignalEndpoints {
    traces: String,
    metrics: String,
    logs: String,
}

#[cfg(any(test, feature = "otel"))]
impl SignalEndpoints {
    fn of(config: &OtelConfig) -> Self {
        let base = config.endpoint.as_deref().unwrap_or_default().trim();
        let base = base.trim_end_matches('/');
        let of = |own: &Option<String>, path: &str| {
            own.as_deref()
                .map(str::trim)
                .filter(|url| !url.is_empty())
                .map_or_else(|| format!("{base}{path}"), str::to_owned)
        };
        Self {
            traces: of(&config.traces_endpoint, "/v1/traces"),
            metrics: of(&config.metrics_endpoint, "/v1/metrics"),
            logs: of(&config.logs_endpoint, "/v1/logs"),
        }
    }
}

/// Why nothing can be sent to `url`, or `None` when it is the URL of a
/// collector: `http` or `https`, and a host.
///
/// The exporter takes a path alone, `/v1/traces`, as its endpoint and is
/// built with it. Every request then fails where the HTTP client reads the
/// URL, and the processors of the SDK drop the result of an export, so a
/// signal with such an endpoint sends nothing and says nothing. It is
/// found here, before the exporter is built. The reason never has the URL
/// in it, which can carry a password or a token.
#[cfg(any(test, feature = "otel"))]
fn why_no_collector(url: &str) -> Option<&'static str> {
    match url::Url::parse(url) {
        Err(_) => Some("the endpoint is not a URL with a scheme and a host"),
        Ok(parsed) if !matches!(parsed.scheme(), "http" | "https") => {
            Some("the scheme of the endpoint is not http or https")
        }
        Ok(parsed) if parsed.host_str().is_none_or(str::is_empty) => {
            Some("the endpoint has no host")
        }
        Ok(_) => None,
    }
}

/// Whether an event or a span is to be exported: everything but what the
/// exporters cause themselves.
///
/// An export is an HTTP request, and the HTTP client logs its requests at
/// debug level from a thread of its own. Exported, those lines are the
/// content of the next export, whose request logs again, and an idle
/// process sends lines about its own sending without end. They are still
/// printed: the filter is on the two layers that export.
#[cfg(feature = "otel")]
fn not_of_the_exporters(metadata: &tracing::Metadata<'_>) -> bool {
    !of_an_export(metadata.target())
}

/// Whether `target` is of a crate that an export runs through.
#[cfg(any(test, feature = "otel"))]
fn of_an_export(target: &str) -> bool {
    const CRATES_OF_AN_EXPORT: [&str; 10] = [
        "hyper",
        "hyper_util",
        "h2",
        "reqwest",
        "rustls",
        "tower",
        "opentelemetry",
        "opentelemetry_sdk",
        "opentelemetry_otlp",
        "opentelemetry_http",
    ];
    CRATES_OF_AN_EXPORT.iter().any(|name| {
        target
            .strip_prefix(name)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with("::"))
    })
}

/// Parse the `OTEL_SDK_DISABLED` value into a boolean.
///
/// The OTel spec treats it as a case-insensitive boolean ("true"/"false").
/// We accept any case of `true`, plus the common `1` convention; everything
/// else (including `false`, `0`, empty, and unset) leaves telemetry enabled.
/// Pulled out as a pure function so the parsing contract is unit-testable
/// without mutating process-global environment state.
fn parse_sdk_disabled(value: Option<&str>) -> bool {
    value
        .map(|v| {
            let v = v.trim();
            v.eq_ignore_ascii_case("true") || v == "1"
        })
        .unwrap_or(false)
}

/// RAII handle returned from [`init_telemetry`]. Owns the SDK provider
/// instances so they can be flushed deterministically on shutdown.
///
/// Call [`shutdown`](Self::shutdown) before the process exits. Dropping
/// the guard without calling `shutdown` emits a warning via `tracing`
/// because batch processors buffer span/metric/log payloads in memory -
/// silently dropping the guard would silently drop telemetry.
///
/// The guard is `Send + Sync` so it can be moved into spawned tasks if
/// needed (e.g. the server keeps it on the main task and flushes on
/// signal).
pub struct TelemetryGuard {
    shutdown_called: Arc<AtomicBool>,
    #[cfg(feature = "otel")]
    tracer_provider: Option<opentelemetry_sdk::trace::SdkTracerProvider>,
    #[cfg(feature = "otel")]
    meter_provider: Option<opentelemetry_sdk::metrics::SdkMeterProvider>,
    #[cfg(feature = "otel")]
    logger_provider: Option<opentelemetry_sdk::logs::SdkLoggerProvider>,
}

impl TelemetryGuard {
    /// `true` when this guard owns at least one live SDK provider that
    /// still needs an explicit flush. The Drop warning is gated on this -
    /// a guard with no providers (the disabled path, the legacy
    /// `init_subscriber` path, or any non-`otel` build) has nothing to
    /// lose on drop and must stay silent.
    #[cfg(feature = "otel")]
    fn owns_providers(&self) -> bool {
        self.tracer_provider.is_some()
            || self.meter_provider.is_some()
            || self.logger_provider.is_some()
    }

    /// Without the `otel` feature there are no providers to own.
    #[cfg(not(feature = "otel"))]
    fn owns_providers(&self) -> bool {
        false
    }

    /// Mark this guard as "shutdown" without invoking provider flush -
    /// used by the legacy `init_subscriber` path. That path holds no
    /// providers, so [`Self::owns_providers`] already keeps Drop silent;
    /// this additionally records the shutdown so the state is unambiguous.
    pub(crate) fn mark_shutdown_for_legacy(self) {
        self.shutdown_called.store(true, Ordering::SeqCst);
    }

    /// Flush and shut down all installed OpenTelemetry providers.
    ///
    /// This is async because the batch processors flush buffered data
    /// to the collector over HTTP. It is safe to call exactly once;
    /// subsequent calls are no-ops.
    pub async fn shutdown(self) {
        // Mark shutdown so the `Drop` impl doesn't warn about a lost flush.
        // `shutdown` takes `self` by value, so it runs at most once.
        self.shutdown_called.store(true, Ordering::SeqCst);
        // The file log channels buffer; nothing written before a clean
        // exit may be lost.
        crate::logging::Log::flush();
        #[cfg(feature = "otel")]
        {
            if let Some(provider) = &self.tracer_provider
                && let Err(err) = provider.shutdown()
            {
                tracing::warn!(?err, "OTel tracer provider shutdown error");
            }
            if let Some(provider) = &self.meter_provider
                && let Err(err) = provider.shutdown()
            {
                tracing::warn!(?err, "OTel meter provider shutdown error");
            }
            if let Some(provider) = &self.logger_provider
                && let Err(err) = provider.shutdown()
            {
                tracing::warn!(?err, "OTel logger provider shutdown error");
            }
        }
    }
}

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        crate::logging::Log::flush();
        // Warn only when we hold providers that were never flushed.
        // Guards with no providers (disabled path, legacy subscriber path,
        // non-`otel` builds) have nothing buffered, so a silent drop is
        // correct - warning there would be pure noise on every process that
        // runs without a collector configured.
        if self.owns_providers() && !self.shutdown_called.load(Ordering::SeqCst) {
            tracing::warn!(
                "TelemetryGuard dropped without shutdown() - buffered \
                 telemetry may be lost. Call guard.shutdown().await before \
                 exiting."
            );
        }
    }
}

/// Build a [`TelemetryGuard`] with no provider handles. Used by the
/// disabled / no-feature paths. Holds no providers, so its Drop is silent.
fn empty_guard() -> TelemetryGuard {
    TelemetryGuard {
        shutdown_called: Arc::new(AtomicBool::new(false)),
        #[cfg(feature = "otel")]
        tracer_provider: None,
        #[cfg(feature = "otel")]
        meter_provider: None,
        #[cfg(feature = "otel")]
        logger_provider: None,
    }
}

/// Install the global `tracing` subscriber and (when applicable) the
/// OpenTelemetry SDK pipelines.
///
/// Behavior:
///
/// 1. Always installs the standard fmt layer driven by [`LogConfig`].
/// 2. When compiled with `feature = "otel"` **and** `otel_config.is_enabled()`,
///    additionally:
///    - builds OTLP HTTP-proto exporters for traces, metrics, and logs;
///    - wraps each in an SDK provider with the configured service-name
///      resource;
///    - installs the providers globally so any code can call
///      `opentelemetry::global::tracer(...)` / `meter(...)`;
///    - installs a `TraceContextPropagator` (from `opentelemetry_sdk::propagation`)
///      for W3C trace-context propagation;
///    - registers a `tracing-opentelemetry` layer so every `tracing::span`
///      becomes an OTel span automatically;
///    - registers the `opentelemetry-appender-tracing` bridge so every
///      `tracing::event` is forwarded to the OTel log pipeline as well.
///
/// Idempotent: a second call is a no-op (the subscriber install returns
/// an error which we silently absorb so tests can call this repeatedly).
pub fn init_telemetry(log_config: LogConfig, otel_config: OtelConfig) -> TelemetryGuard {
    #[cfg(feature = "otel")]
    {
        if otel_config.is_enabled() {
            return init_telemetry_with_otel(log_config, otel_config);
        }
    }
    let _ = otel_config; // silence unused warning when feature is off
    install_base_subscriber(&log_config);
    empty_guard()
}

#[cfg(feature = "otel")]
fn init_telemetry_with_otel(log_config: LogConfig, otel_config: OtelConfig) -> TelemetryGuard {
    use crate::logging::init::build_env_filter;
    use opentelemetry::KeyValue;
    use opentelemetry::global;
    use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
    use opentelemetry_otlp::WithExportConfig;
    use opentelemetry_sdk::Resource;
    use opentelemetry_sdk::logs::SdkLoggerProvider;
    use opentelemetry_sdk::metrics::SdkMeterProvider;
    use opentelemetry_sdk::trace::SdkTracerProvider;
    use opentelemetry_semantic_conventions::resource as semconv;
    use tracing_subscriber::Layer;
    use tracing_subscriber::filter::filter_fn;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let endpoints = SignalEndpoints::of(&otel_config);

    // Resource is shared across all three signals.
    let resource = Resource::builder()
        .with_attributes(vec![
            KeyValue::new(semconv::SERVICE_NAME, otel_config.service_name.clone()),
            KeyValue::new(
                semconv::SERVICE_VERSION,
                otel_config.service_version.clone(),
            ),
        ])
        .build();

    // Each signal is built by itself. One that cannot be built, which is
    // what a URL with a mistake in it does, is left out and reported, and
    // the other two are exported. The report waits until the subscriber
    // is installed: an error that is logged before there is one is seen
    // by nobody.
    let mut not_built: Vec<(&'static str, String)> = Vec::new();

    // --- Traces ---
    let tracer_provider = match why_no_collector(&endpoints.traces) {
        Some(reason) => {
            not_built.push(("traces", reason.to_owned()));
            None
        }
        None => match opentelemetry_otlp::SpanExporter::builder()
            .with_http()
            .with_endpoint(&endpoints.traces)
            .build()
        {
            Ok(exporter) => {
                let provider = SdkTracerProvider::builder()
                    .with_batch_exporter(exporter)
                    .with_resource(resource.clone())
                    .build();
                global::set_tracer_provider(provider.clone());
                Some(provider)
            }
            Err(err) => {
                not_built.push(("traces", why_not_built(&err)));
                None
            }
        },
    };

    // --- Metrics ---
    let meter_provider = match why_no_collector(&endpoints.metrics) {
        Some(reason) => {
            not_built.push(("metrics", reason.to_owned()));
            None
        }
        None => match opentelemetry_otlp::MetricExporter::builder()
            .with_http()
            .with_endpoint(&endpoints.metrics)
            .build()
        {
            Ok(exporter) => {
                let provider = SdkMeterProvider::builder()
                    .with_periodic_exporter(exporter)
                    .with_resource(resource.clone())
                    .build();
                global::set_meter_provider(provider.clone());
                Some(provider)
            }
            Err(err) => {
                not_built.push(("metrics", why_not_built(&err)));
                None
            }
        },
    };

    // --- Logs ---
    let logger_provider = match why_no_collector(&endpoints.logs) {
        Some(reason) => {
            not_built.push(("logs", reason.to_owned()));
            None
        }
        None => match opentelemetry_otlp::LogExporter::builder()
            .with_http()
            .with_endpoint(&endpoints.logs)
            .build()
        {
            Ok(exporter) => Some(
                SdkLoggerProvider::builder()
                    .with_batch_exporter(exporter)
                    .with_resource(resource)
                    .build(),
            ),
            Err(err) => {
                not_built.push(("logs", why_not_built(&err)));
                None
            }
        },
    };

    // --- Propagation ---
    if tracer_provider.is_some() {
        crate::telemetry::propagation::install_trace_context_propagator();
    }

    // --- Wire layers into the global subscriber ---
    //
    // The output layers are boxed, so the subscriber has one type whatever
    // the format, and the OpenTelemetry layers are built once on top of it.
    // A signal that was not built has no layer: `None` is a layer that does
    // nothing.
    let env_filter = build_env_filter(&log_config.level);

    // try_init() returns Err if a global default is already set (e.g.
    // tests). The existing subscriber wins and we still hand back a guard
    // for orderly shutdown of the providers we built. It also forwards
    // the records of the `log` crate, as the base subscriber does.
    let installed = tracing_subscriber::registry()
        .with(env_filter)
        .with(crate::logging::layer::output_layers(&log_config))
        .with(tracer_provider.as_ref().map(|_| {
            tracing_opentelemetry::layer()
                .with_tracer(global::tracer("suprnova"))
                .with_filter(filter_fn(not_of_the_exporters))
        }))
        .with(logger_provider.as_ref().map(|provider| {
            OpenTelemetryTracingBridge::new(provider).with_filter(filter_fn(not_of_the_exporters))
        }))
        .try_init()
        .is_ok();

    if installed {
        // Only once installed, so a refused install leaves the live
        // subscriber's channels and format as they were.
        crate::logging::layer::adopt_installed(&log_config);
    } else {
        tracing::warn!(
            "tracing subscriber already installed; keeping the existing one (this LogConfig \
             was not applied, and the spans and the log lines are not exported)"
        );
    }
    for (signal, reason) in not_built {
        tracing::error!(
            signal,
            reason,
            "failed to build the OTLP exporter of this signal; continuing without it"
        );
    }

    TelemetryGuard {
        shutdown_called: Arc::new(AtomicBool::new(false)),
        tracer_provider,
        meter_provider,
        logger_provider,
    }
}

/// Why an exporter was not built, in words that are safe in a log.
///
/// The error of an endpoint that is no URL carries the endpoint as it was
/// written, and an endpoint can have a user and a password or a token in
/// it. So this says what kind of error it was, and for an endpoint the
/// reason the parser gave, and never the endpoint.
#[cfg(feature = "otel")]
fn why_not_built(error: &opentelemetry_otlp::ExporterBuildError) -> String {
    use opentelemetry_otlp::ExporterBuildError;

    match error {
        ExporterBuildError::InvalidUri(_, reason) => {
            format!("the endpoint is not a URL the exporter can use: {reason}")
        }
        ExporterBuildError::NoHttpClient => "no HTTP client is compiled in".to_owned(),
        ExporterBuildError::ThreadSpawnFailed => {
            "the thread of the HTTP client could not be started".to_owned()
        }
        ExporterBuildError::UnsupportedCompressionAlgorithm(what) => {
            format!("the compression that was asked for cannot be used: {what}")
        }
        _ => "the exporter refused its configuration".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Same env-serialization pattern as `crate::logging::config` -
    // tests in this module touch global env so they must run sequentially.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn clear_env() {
        // SAFETY: ENV_LOCK guards concurrent env mutation within this module.
        unsafe {
            std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
            std::env::remove_var("OTEL_SERVICE_NAME");
            std::env::remove_var("OTEL_SERVICE_VERSION");
            std::env::remove_var("OTEL_SDK_DISABLED");
        }
    }

    fn config(base: &str) -> OtelConfig {
        OtelConfig {
            endpoint: Some(base.to_owned()),
            disabled: false,
            ..OtelConfig::disabled()
        }
    }

    #[test]
    fn each_signal_has_its_path_under_the_base() {
        for base in [
            "http://localhost:4318",
            "http://localhost:4318/",
            " http://localhost:4318 ",
        ] {
            assert_eq!(
                SignalEndpoints::of(&config(base)),
                SignalEndpoints {
                    traces: "http://localhost:4318/v1/traces".to_owned(),
                    metrics: "http://localhost:4318/v1/metrics".to_owned(),
                    logs: "http://localhost:4318/v1/logs".to_owned(),
                },
                "{base:?}"
            );
        }
        assert_eq!(
            SignalEndpoints::of(&config("https://otel.example.com/tenant-7")).traces,
            "https://otel.example.com/tenant-7/v1/traces",
            "a base with a path keeps it"
        );
    }

    #[test]
    fn the_endpoint_of_a_signal_is_used_as_it_is_written() {
        let endpoints = SignalEndpoints::of(&OtelConfig {
            traces_endpoint: Some("http://tempo:4318/v1/traces".to_owned()),
            metrics_endpoint: Some(" http://mimir:9009/otlp/v1/metrics ".to_owned()),
            ..config("http://localhost:4318")
        });

        assert_eq!(endpoints.traces, "http://tempo:4318/v1/traces");
        assert_eq!(
            endpoints.metrics, "http://mimir:9009/otlp/v1/metrics",
            "no path is added to it"
        );
        assert_eq!(
            endpoints.logs, "http://localhost:4318/v1/logs",
            "a signal with no endpoint of its own goes to the base"
        );
    }

    #[cfg(feature = "otel")]
    #[test]
    fn the_reason_an_exporter_was_not_built_never_has_the_endpoint() {
        use opentelemetry_otlp::ExporterBuildError;

        let said = why_not_built(&ExporterBuildError::InvalidUri(
            "http://operator:hunter2@collector.internal.test:4318/v1/traces?token=abc".to_owned(),
            "invalid format".to_owned(),
        ));
        assert!(said.contains("invalid format"), "{said}");
        for secret in [
            "hunter2",
            "operator",
            "collector.internal.test",
            "token=abc",
        ] {
            assert!(
                !said.contains(secret),
                "the reason shows `{secret}`: {said}"
            );
        }

        let said = why_not_built(&ExporterBuildError::InternalFailure(
            "http://operator:hunter2@collector.internal.test".to_owned(),
        ));
        assert!(!said.contains("hunter2"), "{said}");
        assert_eq!(
            why_not_built(&ExporterBuildError::NoHttpClient),
            "no HTTP client is compiled in"
        );
    }

    #[test]
    fn a_blank_base_does_not_turn_telemetry_on() {
        assert!(config("http://localhost:4318").is_enabled());
        assert!(!config("").is_enabled());
        assert!(!config("   ").is_enabled());
        let off = OtelConfig {
            disabled: true,
            ..config("http://localhost:4318")
        };
        assert!(!off.is_enabled());
    }

    #[test]
    fn an_endpoint_is_a_url_with_a_scheme_and_a_host() {
        for url in [
            "http://localhost:4318/v1/traces",
            "https://otel.example.com/tenant-7/v1/logs",
            "http://[::1]:4318/v1/metrics",
        ] {
            assert_eq!(why_no_collector(url), None, "{url}");
        }
        for url in [
            "/v1/traces",
            "localhost:4318/v1/traces",
            "http://[not a url",
            "ftp://collector.internal.test/v1/traces",
            "",
        ] {
            let reason = why_no_collector(url);
            assert!(reason.is_some(), "{url:?} is no endpoint");
            assert!(
                !reason.is_some_and(|reason| reason.contains("collector.internal.test")),
                "the reason has the endpoint in it"
            );
        }
    }

    #[test]
    fn what_the_exporters_cause_themselves_is_not_exported() {
        for target in [
            "hyper_util::client::legacy::pool",
            "hyper_util::client::legacy::connect::http",
            "hyper::proto::h1::io",
            "reqwest::connect",
            "h2::codec::framed_write",
            "rustls::client::hs",
            "opentelemetry_sdk",
            "tower::buffer::worker",
        ] {
            assert!(of_an_export(target), "{target} is of an export");
        }
        for target in [
            "app::http::controllers",
            "suprnova::telemetry::init",
            "hyperion::engine",
            "h2o::client",
            "towering::inferno",
        ] {
            assert!(!of_an_export(target), "{target} is of the application");
        }
    }

    #[test]
    fn a_blank_endpoint_of_a_signal_is_no_endpoint() {
        let endpoints = SignalEndpoints::of(&OtelConfig {
            logs_endpoint: Some("   ".to_owned()),
            ..config("http://localhost:4318")
        });
        assert_eq!(endpoints.logs, "http://localhost:4318/v1/logs");
    }

    #[test]
    fn otel_config_disabled_sentinel() {
        let cfg = OtelConfig::disabled();
        assert!(!cfg.is_enabled());
        assert!(cfg.disabled);
        assert!(cfg.endpoint.is_none());
    }

    #[test]
    fn otel_config_from_env_no_endpoint() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_env();
        let cfg = OtelConfig::from_env();
        assert!(!cfg.is_enabled());
        assert_eq!(cfg.service_name, "suprnova");
        assert_eq!(cfg.service_version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn otel_config_from_env_with_endpoint() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_env();
        // SAFETY: ENV_LOCK serializes env access.
        unsafe {
            std::env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", "http://localhost:4318");
            std::env::set_var("OTEL_SERVICE_NAME", "test-service");
        }
        let cfg = OtelConfig::from_env();
        assert!(cfg.is_enabled());
        assert_eq!(cfg.endpoint.as_deref(), Some("http://localhost:4318"));
        assert_eq!(cfg.service_name, "test-service");
        clear_env();
    }

    #[test]
    fn otel_config_sdk_disabled_flag() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_env();
        // SAFETY: ENV_LOCK serializes env access.
        unsafe {
            std::env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", "http://localhost:4318");
            std::env::set_var("OTEL_SDK_DISABLED", "true");
        }
        let cfg = OtelConfig::from_env();
        // Endpoint set but kill switch wins.
        assert!(!cfg.is_enabled());
        assert!(cfg.disabled);
        clear_env();
    }

    #[cfg(feature = "otel")]
    #[test]
    fn init_telemetry_no_endpoint_stays_noop() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_env();
        let guard = init_telemetry(LogConfig::default(), OtelConfig::from_env());
        assert!(guard.tracer_provider.is_none());
        assert!(guard.meter_provider.is_none());
        assert!(guard.logger_provider.is_none());
        // Acknowledge the guard so Drop doesn't warn.
        guard.mark_shutdown_for_legacy();
    }

    // ---- OTEL_SDK_DISABLED parse (no env mutation needed) -------------

    #[test]
    fn sdk_disabled_accepts_case_insensitive_true_and_one() {
        for v in ["true", "True", "TRUE", "tRuE", "1"] {
            assert!(parse_sdk_disabled(Some(v)), "{v:?} should disable the SDK",);
        }
    }

    #[test]
    fn sdk_disabled_trims_surrounding_whitespace() {
        assert!(parse_sdk_disabled(Some("  true  ")));
        assert!(parse_sdk_disabled(Some(" 1 ")));
    }

    #[test]
    fn sdk_disabled_leaves_telemetry_enabled_for_other_values() {
        // Unset, explicit false, zero, and arbitrary text all mean "enabled".
        for v in [
            None,
            Some("false"),
            Some("FALSE"),
            Some("0"),
            Some("yes"),
            Some(""),
        ] {
            assert!(!parse_sdk_disabled(v), "{v:?} must NOT disable the SDK",);
        }
    }

    // ---- empty / disabled guard drop is silent ------------------------

    #[test]
    fn empty_guard_owns_no_providers_so_drop_is_silent() {
        // The disabled path returns `empty_guard()`. It holds no providers,
        // so `owns_providers()` is false and Drop must not warn about lost
        // telemetry - there is nothing buffered. Regression guard for the
        // spurious "buffered telemetry may be lost" warning that fired on
        // every collector-less process before this fix.
        let guard = empty_guard();
        assert!(
            !guard.owns_providers(),
            "a guard with no providers must report owns_providers() == false",
        );
        // Drop runs here with shutdown_called still false; the assertion
        // above pins the invariant the Drop warning is gated on. (A
        // subscriber-capture assertion would need global subscriber state,
        // which collides with parallel tests - the owns_providers gate is
        // the deterministic core.)
        drop(guard);
    }
}
