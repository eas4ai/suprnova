//! Request and job diagnostics stay with their owner, never a global breadcrumb bag.

use super::{Reporter, ReportingConfig, ReportingEvent, ReportingKind};
use crate::{App, Request};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::SystemTime;

#[derive(Clone, Default)]
pub(crate) struct ReportScope(Arc<Mutex<State>>);

#[derive(Default)]
struct State {
    request: Option<Value>,
    name: Option<String>,
    request_id: Option<String>,
    user_id: Option<String>,
    ip: Option<String>,
    breadcrumbs: VecDeque<Value>,
    trace: Option<Trace>,
    failed: bool,
    job: bool,
}

struct Trace {
    id: String,
    started: SystemTime,
    op: &'static str,
    config: ReportingConfig,
    reporter: Arc<dyn Reporter>,
}

tokio::task_local! { static CURRENT: ReportScope; }

impl ReportScope {
    pub(crate) fn http<B>(request: &hyper::Request<B>, peer: Option<std::net::IpAddr>) -> Self {
        let scope = Self::default();
        let headers: serde_json::Map<_, _> = request
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.to_string(), json!(value)))
            })
            .collect();
        let mut state = scope.0.lock().unwrap_or_else(PoisonError::into_inner);
        let url = if request.uri().scheme().is_some() {
            request.uri().to_string()
        } else {
            let authority = request
                .headers()
                .get("host")
                .and_then(|value| value.to_str().ok())
                .unwrap_or("localhost");
            format!("http://{authority}{}", request.uri())
        };
        state.request =
            Some(json!({"method":request.method().as_str(),"url":url,"headers":headers}));
        state.request_id = request
            .headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
            .or_else(|| Some(crate::logging::RequestId::new().to_string()));
        state.ip = peer.map(|ip| ip.to_string());
        drop(state);
        scope.start("http.server", request.uri().path());
        scope
    }

    pub(crate) fn start(&self, op: &'static str, name: &str) {
        let config = App::get::<ReportingConfig>().unwrap_or_default();
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        state.name = Some(name.to_owned());
        state.job = op != "http.server";
        if state.trace.is_none()
            && config.traces_sample_rate > 0.0
            && rand::random::<f32>() < config.traces_sample_rate
            && let Ok(reporter) = App::resolve_make::<dyn Reporter>()
        {
            state.trace = Some(Trace {
                id: uuid::Uuid::new_v4().simple().to_string(),
                started: crate::clock::now().into(),
                op,
                config,
                reporter,
            });
        }
    }

    pub(crate) async fn run<F: Future>(&self, future: F) -> F::Output {
        CURRENT.scope(self.clone(), future).await
    }

    /// A response can fail without emitting an exception, while a rescued
    /// exception can fail work even when its final response succeeds.
    pub(crate) fn note_status(&self, status: u16) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).failed |= status >= 500;
    }
}

pub(crate) fn current() -> Option<ReportScope> {
    let context = crate::Context::current().map(|store| store.reporting.clone());
    if let Some(scope) = &context
        && scope.0.lock().unwrap_or_else(PoisonError::into_inner).job
    {
        return context;
    }
    CURRENT.try_with(Clone::clone).ok().or(context)
}

pub(crate) fn note_request(request: &Request, request_id: &str) {
    if let Some(scope) = current() {
        let mut state = scope.0.lock().unwrap_or_else(PoisonError::into_inner);
        let headers: serde_json::Map<_, _> = request
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.to_string(), json!(value)))
            })
            .collect();
        state.request = Some(
            json!({"method":request.method().as_str(),"url":request.full_url(),"headers":headers}),
        );
        state.name = Some(
            request
                .route_pattern()
                .unwrap_or_else(|| request.path())
                .to_owned(),
        );
        state.request_id = Some(request_id.to_owned());
        state.user_id = request.auth_user_id().map(str::to_owned);
        state.ip = request.ip();
    }
}

pub(crate) fn enrich(payload: &mut Value, send_pii: bool) {
    if let Some(scope) = current() {
        let mut state = scope.0.lock().unwrap_or_else(PoisonError::into_inner);
        state.failed = true;
        if let Some(request) = &state.request {
            payload["request"] = request.clone();
        }
        if let Some(name) = &state.name {
            payload["transaction"] = json!(name);
        }
        let id = crate::logging::current_request_id()
            .map(|id| id.to_string())
            .or_else(|| state.request_id.clone())
            .or_else(|| crate::Context::get::<String>("_request_id"));
        if let Some(id) = id {
            payload["tags"]["request_id"] = json!(id);
        }
        let user = state
            .request
            .as_ref()
            .and_then(|_| crate::auth::request_state::current_user_id())
            .or_else(|| state.user_id.clone());
        if let Some(user) = user {
            payload["user"]["id"] = json!(user);
        }
        if send_pii && let Some(ip) = &state.ip {
            payload["user"]["ip_address"] = json!(ip);
        }
        payload["breadcrumbs"] = json!({"values":state.breadcrumbs});
        if let Some(trace) = &state.trace {
            payload["contexts"]["trace"] =
                json!({"type":"trace","trace_id":trace.id,"span_id":&trace.id[..16],"op":trace.op});
        }
    }
}

pub(crate) fn record_log(
    level: &str,
    message: &str,
    context: &serde_json::Map<String, Value>,
) -> Option<String> {
    current().and_then(|scope| {
        let mut state = scope.0.lock().unwrap_or_else(PoisonError::into_inner);
        if state.breadcrumbs.len() == 32 { state.breadcrumbs.pop_front(); }
        state.breadcrumbs.push_back(json!({"timestamp":timestamp(crate::clock::now().into()),"level":level,"category":"log","message":message,"data":context}));
        state.trace.as_ref().map(|trace|trace.id.clone())
    })
}

pub(crate) fn timestamp(time: SystemTime) -> f64 {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

impl Drop for State {
    fn drop(&mut self) {
        let Some(trace) = self.trace.take() else {
            return;
        };
        let mut payload = super::event::base(&trace.config);
        payload["transaction"] = json!(self.name.as_deref().unwrap_or("job"));
        payload["start_timestamp"] = json!(timestamp(trace.started));
        payload["timestamp"] = json!(crate::clock::now().to_rfc3339());
        payload["spans"] = json!([]);
        payload["contexts"]["trace"] = json!({"type":"trace","trace_id":trace.id,"span_id":&trace.id[..16],"op":trace.op,"status":if self.failed {"internal_error"} else {"ok"}});
        crate::error::redaction::scrub(&mut payload, trace.config.send_pii);
        let event = ReportingEvent {
            kind: ReportingKind::Transaction,
            payload,
        };
        let _ = super::Reporting::send_using(&trace.reporter, event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn report_003_early_request_failures_have_an_id_before_middleware_runs() {
        let request = hyper::Request::new(());
        let scope = ReportScope::http(&request, None);
        scope
            .run(async {
                let mut payload = json!({});
                enrich(&mut payload, false);
                let id = payload["tags"]["request_id"].as_str().expect("request id");
                assert!(uuid::Uuid::parse_str(id).is_ok(), "generated request id");
                assert_eq!(payload["request"]["method"], "GET");
            })
            .await;
    }
}
