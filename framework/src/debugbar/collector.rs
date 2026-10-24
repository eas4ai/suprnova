//! Task-local collection: formatting and locks are skipped when no request is collecting.

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Value, json};

use super::WireResponse;
use crate::error::redaction::{REDACTED, header_value_is_secret, names_a_secret, redact_text};

tokio::task_local! { static CURRENT: Arc<Mutex<Collector>>; }

#[derive(Default, Serialize)]
pub(super) struct Record {
    pub(super) id: String,
    pub(super) endpoint: String,
    request: BTreeMap<String, Value>,
    status: u16,
    duration_ms: f64,
    peak_memory_bytes: Option<u64>,
    memory_measurement: &'static str,
    timings: Vec<Value>,
    panels: BTreeMap<String, Vec<Value>>,
    error: Option<String>,
}

impl Record {
    pub(super) fn json(&self) -> Value {
        // The record holds only JSON values, strings and finite elapsed times.
        json!(self)
    }
}

pub(super) struct Collector {
    record: Record,
    secrets: Vec<String>,
    started: Instant,
    rss_start: Option<u64>,
    rss_peak: Option<u64>,
    closed: bool,
}

impl Collector {
    pub(super) fn capture<B>(request: &hyper::Request<B>) -> Self {
        let mut secrets = Vec::new();
        let headers: Vec<_> = request
            .headers()
            .iter()
            .map(|(name, value)| {
                let value = String::from_utf8_lossy(value.as_bytes());
                let shown = if header_value_is_secret(name.as_str()) {
                    secrets.push(value.into_owned());
                    format!("{REDACTED:?}")
                } else {
                    format!("{value:?}")
                };
                (name.as_str().to_string(), shown)
            })
            .collect();
        let query: Vec<_> =
            url::form_urlencoded::parse(request.uri().query().unwrap_or("").as_bytes())
                .map(|(name, value)| {
                    let shown = if names_a_secret(&name) {
                        secrets.push(value.into_owned());
                        format!("{REDACTED:?}")
                    } else {
                        format!("{value:?}")
                    };
                    (name.into_owned(), shown)
                })
                .collect();
        #[cfg(feature = "localization")]
        let locale = json!(format!("{:?}", crate::localization::debugbar_locale().1));
        #[cfg(not(feature = "localization"))]
        let locale = Value::Null;
        let request = BTreeMap::from([
            ("method".into(), json!(format!("{:?}", request.method()))),
            ("path".into(), json!(format!("{:?}", request.uri().path()))),
            ("query".into(), json!(query)),
            ("headers".into(), json!(headers)),
            ("route_pattern".into(), Value::Null),
            ("parameters".into(), json!({})),
            ("request_id".into(), Value::Null),
            ("session_id".into(), Value::Null),
            ("principal".into(), Value::Null),
            ("locale".into(), locale),
        ]);
        let rss = rss();
        Self {
            record: Record {
                id: uuid::Uuid::new_v4().to_string(),
                endpoint: "/_debugbar/".into(),
                request,
                status: 0,
                duration_ms: 0.0,
                peak_memory_bytes: None,
                memory_measurement: "sampled process RSS growth",
                timings: Vec::new(),
                panels: [
                    "queries",
                    "transactions",
                    "cache",
                    "events",
                    "mail",
                    "notifications",
                    "queue",
                    "render_cache",
                    "live",
                ]
                .into_iter()
                .map(|name| (name.into(), Vec::new()))
                .collect(),
                error: None,
            },
            secrets,
            started: Instant::now(),
            rss_start: rss,
            rss_peak: rss,
            closed: false,
        }
    }

    fn sample(&mut self) {
        if let Some(current) = rss() {
            self.rss_peak = Some(self.rss_peak.unwrap_or(current).max(current));
        }
    }

    fn finish(&mut self, response: &WireResponse) -> Record {
        self.sample();
        self.record.status = response.status().as_u16();
        self.record.duration_ms = self.started.elapsed().as_secs_f64() * 1000.0;
        self.record.peak_memory_bytes = self
            .rss_start
            .zip(self.rss_peak)
            .map(|(start, peak)| peak.saturating_sub(start));
        if let Some(id) = response
            .headers()
            .get("x-request-id")
            .and_then(|id| id.to_str().ok())
        {
            self.record
                .request
                .insert("request_id".into(), json!(format!("{id:?}")));
        }
        self.record.error = response
            .extensions()
            .get::<crate::ErrorReport>()
            .map(|error| format!("{error:?}"));
        for value in self.record.request.values_mut() {
            sanitize(value, &self.secrets);
        }
        for values in self.record.panels.values_mut() {
            for value in values {
                sanitize(value, &self.secrets);
            }
        }
        for value in &mut self.record.timings {
            sanitize(value, &self.secrets);
        }
        if let Some(error) = self.record.error.take() {
            let mut value = Value::String(error);
            sanitize(&mut value, &self.secrets);
            self.record.error = value.as_str().map(str::to_owned);
        }
        self.closed = true;
        self.secrets.clear();
        std::mem::take(&mut self.record)
    }
}

pub(super) async fn serve<F: Future<Output = WireResponse>>(
    collector: Collector,
    response: F,
) -> (WireResponse, Record) {
    let current = Arc::new(Mutex::new(collector));
    let response = CURRENT.scope(Arc::clone(&current), async {
        tokio::pin!(response);
        let mut interval = tokio::time::interval(Duration::from_millis(10));
        loop {
            tokio::select! {
                response = &mut response => break response,
                _ = interval.tick() => current.lock().unwrap_or_else(|p| p.into_inner()).sample(),
            }
        }
    }).await;
    let record = current
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .finish(&response);
    (response, record)
}

pub(crate) fn active() -> bool {
    CURRENT.try_with(|_| ()).is_ok()
}

pub(crate) fn record(panel: &str, name: &str, value: &impl Debug) {
    let _ = CURRENT.try_with(|current| {
        let mut current = current.lock().unwrap_or_else(|p| p.into_inner());
        if current.closed {
            return;
        }
        if let Some(entries) = current.record.panels.get_mut(panel) {
            entries.push(json!({"name": name, "value": format!("{value:?}")}));
        }
    });
}

pub(crate) fn event<E: crate::events::Event>(event: &E, listeners: Vec<&str>) {
    if !active() {
        return;
    }
    let any = event as &dyn std::any::Any;
    if let Some(query) = any.downcast_ref::<crate::database::events::QueryExecuted>() {
        let mut shown = query.clone();
        // A positional binding has no field name of its own. If the SQL names
        // a secret, hide the statement and its bindings rather than guess which
        // position held that value. Listeners still receive the original event.
        if shown
            .sql
            .split(|character: char| !character.is_alphanumeric() && character != '_')
            .any(names_a_secret)
        {
            shown.sql = REDACTED.into();
            shown.bindings.fill(REDACTED.into());
            if shown.result.is_err() {
                shown.result = Err(REDACTED.into());
            }
        }
        record("events", E::event_name(), &(&shown, listeners));
        record("queries", E::event_name(), &shown);
        return;
    }
    record("events", E::event_name(), &(event, listeners));
    if any.is::<crate::database::events::TransactionBeginning>()
        || any.is::<crate::database::events::TransactionCommitted>()
        || any.is::<crate::database::events::TransactionRolledBack>()
    {
        record("transactions", E::event_name(), event);
    }
}

/// Read observational snapshots rather than the cache-classifying facades: a
/// diagnostic read must not change whether an application's render is cacheable.
pub(crate) fn note_request(request: &crate::Request) {
    note_context();
    let _ = CURRENT.try_with(|current| {
        let mut current = current.lock().unwrap_or_else(|p| p.into_inner());
        if current.closed {
            return;
        }
        current.record.endpoint = format!("{}/_debugbar/", request.public_root());
        let parameters: BTreeMap<_, _> = request
            .params()
            .iter()
            .map(|(name, value)| {
                let shown = if names_a_secret(name) {
                    current.secrets.push(value.clone());
                    format!("{REDACTED:?}")
                } else {
                    format!("{value:?}")
                };
                (name, shown)
            })
            .collect();
        current
            .record
            .request
            .insert("parameters".into(), json!(parameters));
        current.record.request.insert(
            "route_pattern".into(),
            json!(request.route_pattern().map(|value| format!("{value:?}"))),
        );
    });
}

pub(crate) fn note_context() {
    let _ = CURRENT.try_with(|current| {
        let mut current = current.lock().unwrap_or_else(|p| p.into_inner());
        if current.closed {
            return;
        }
        let (session, session_principal) = crate::session::middleware::debugbar_identity();
        let session_active = session.is_some();
        if let Some(session) = session {
            current
                .record
                .request
                .insert("session_id".into(), json!(format!("{session:?}")));
        }
        let principal = crate::auth::request_state::debugbar_principal().or(session_principal);
        if principal.is_some() || session_active {
            current.record.request.insert(
                "principal".into(),
                json!(principal.map(|value| format!("{value:?}"))),
            );
        }
        #[cfg(feature = "localization")]
        {
            let (scoped, locale) = crate::localization::debugbar_locale();
            if scoped
                || current
                    .record
                    .request
                    .get("locale")
                    .is_none_or(Value::is_null)
            {
                current
                    .record
                    .request
                    .insert("locale".into(), json!(format!("{locale:?}")));
            }
        }
    });
}

/// A drop guard keeps elapsed spans when application code unwinds.
pub(crate) struct Span {
    name: String,
    started: Instant,
    children: Option<Arc<Mutex<Duration>>>,
}

pub(crate) fn span(name: &str, children: Option<Arc<Mutex<Duration>>>) -> Option<Span> {
    active().then(|| Span {
        name: name.into(),
        started: Instant::now(),
        children,
    })
}

impl Drop for Span {
    fn drop(&mut self) {
        let elapsed = self.started.elapsed();
        let children = self.children.as_ref().map_or(Duration::ZERO, |time| {
            *time.lock().unwrap_or_else(|p| p.into_inner())
        });
        let _ = CURRENT.try_with(|current| {
            current
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .record
                .timings
                .push(json!({
                    "name": self.name, "inclusive_ms": elapsed.as_secs_f64() * 1000.0,
                    "duration_ms": elapsed.saturating_sub(children).as_secs_f64() * 1000.0,
                }))
        });
    }
}

fn sanitize(value: &mut Value, secrets: &[String]) {
    match value {
        Value::String(text) => {
            *text = redact_text(text).into_owned();
            for secret in secrets.iter().filter(|value| !value.is_empty()) {
                *text = text.replace(secret, REDACTED);
                let debug = format!("{secret:?}");
                *text = text.replace(debug.trim_matches('"'), REDACTED);
            }
        }
        Value::Array(values) => values.iter_mut().for_each(|value| sanitize(value, secrets)),
        Value::Object(fields) => fields
            .values_mut()
            .for_each(|value| sanitize(value, secrets)),
        _ => {}
    }
}

fn rss() -> Option<u64> {
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find_map(|line| {
            line.strip_prefix("VmRSS:")
                .and_then(|value| value.split_whitespace().next())
                .and_then(|value| value.parse::<u64>().ok())
                .map(|kib| kib * 1024)
        })
}

/// Framework-owned child tasks keep the originating collector; unrelated spawned
/// application work has its own lifetime and does not acquire a request record.
pub(crate) fn in_current_scope<F: Future>(future: F) -> impl Future<Output = F::Output> {
    let current = CURRENT.try_with(Arc::clone).ok();
    async move {
        match current {
            Some(current) => CURRENT.scope(current, future).await,
            None => future.await,
        }
    }
}

/// Count time inside Next even when a timeout or panic drops its future.
pub(crate) struct NextSpan {
    time: Arc<Mutex<Duration>>,
    started: Instant,
}

pub(crate) fn next_span(time: Arc<Mutex<Duration>>) -> NextSpan {
    NextSpan {
        time,
        started: Instant::now(),
    }
}

impl Drop for NextSpan {
    fn drop(&mut self) {
        *self.time.lock().unwrap_or_else(|p| p.into_inner()) += self.started.elapsed();
    }
}

/// Built-in lookups have a route even though they bypass the application router.
pub(super) fn note_lookup(id: &str, root: &str) {
    let _ = CURRENT.try_with(|current| {
        let mut current = current.lock().unwrap_or_else(|p| p.into_inner());
        current.record.endpoint = format!("{root}/_debugbar/");
        current.record.request.insert(
            "route_pattern".into(),
            json!(format!("{:?}", "/_debugbar/{id}")),
        );
        current
            .record
            .request
            .insert("parameters".into(), json!({"id": format!("{id:?}")}));
    });
}
