//! The middleware that records requests for the extension and answers its
//! entry endpoints: Laravel's `RequestRecorder`, `EntryStore::flush` and
//! the routes `DevToolsServiceProvider` registers.

use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use futures::FutureExt;
use serde_json::Value;

use super::config::DevToolsConfig;
use super::endpoints;
use super::entry::{self, RequestFacts, Stamp};
use super::recorder::{self, Recorder, RenderPayload};
use super::redact::Redactor;
use super::store::{self, EntriesRepository};
use super::ulid;
use crate::http::{HttpResponse, Request, Response};
use crate::middleware::{Middleware, Next};

/// The response header carrying the entry's id.
pub(crate) const ID_HEADER: &str = "X-Inertia-Devtools-Id";
/// The response header carrying the entry the client's next request in
/// the same batch belongs to.
pub(crate) const PARENT_OUT_HEADER: &str = "X-Inertia-Devtools-Parent-Out";
/// The response header carrying the path the application is served
/// under, when it is not the host's root.
pub(crate) const BASE_PATH_HEADER: &str = "X-Inertia-Devtools-Base-Path";

/// Records each request for the Inertia DevTools browser extension and
/// answers the extension's `GET /_inertia/devtools/entries` and
/// `GET /_inertia/devtools/entries/{id}`.
///
/// [`Inertia::install`](crate::Inertia::install) and
/// [`Inertia::middleware`](crate::Inertia::middleware) put it outermost in
/// the Inertia stack when [`DevToolsConfig::is_enabled`] says so, inside
/// the session, so an entry sees the response the rest of the stack
/// produced and the endpoints can reflash the session and ask the gate
/// about the signed-in user.
///
/// Recording never changes the response beyond the DevTools headers and,
/// on the first visit of a page, the id tag: a failure while recording
/// drops the entry, and a failure to store one is logged once and stops
/// recording for 30 seconds.
#[derive(Clone)]
pub struct DevToolsMiddleware {
    config: Arc<DevToolsConfig>,
    repository: Arc<EntriesRepository>,
    redactor: Arc<Redactor>,
    /// Whether this instance records requests, or only answers the
    /// endpoints.
    records: bool,
}

impl DevToolsMiddleware {
    /// The middleware for `config`, whether or not it is enabled: the
    /// stack decides whether to carry it.
    pub fn new(config: DevToolsConfig) -> Self {
        let redactor = Redactor::new(&config.redact_keys, &config.redact_headers);
        let repository = EntriesRepository::new(config.storage_path.clone());
        Self {
            config: Arc::new(config),
            repository: Arc::new(repository),
            redactor: Arc::new(redactor),
            records: true,
        }
    }

    /// The middleware that answers the entry endpoints and records
    /// nothing: the global one [`Inertia::install`](crate::Inertia::install)
    /// registers when the Inertia stack goes on route groups, which record
    /// their own routes.
    pub(crate) fn endpoints_only(config: DevToolsConfig) -> Self {
        Self {
            records: false,
            ..Self::new(config)
        }
    }

    /// Whether `request` is left alone: its path matches an `except`
    /// pattern, Laravel's `DevTools::enabledForRequest`.
    fn is_excepted(&self, request: &Request) -> bool {
        let patterns: Vec<&str> = self.config.except.iter().map(String::as_str).collect();
        !patterns.is_empty() && request.is(&patterns)
    }

    /// Record `facts` answered by `response`, returning the response with
    /// the DevTools headers and, on a first visit, the id tag.
    async fn record(
        &self,
        facts: RequestFacts,
        payload: Option<RenderPayload>,
        response: HttpResponse,
    ) -> HttpResponse {
        self.record_with(facts, payload, response, |entry| {
            self.redactor.redact_entry(entry);
        })
        .await
    }

    /// [`record`](Self::record) with `redact` as the storage pass, so a
    /// test can hand it one that fails.
    async fn record_with(
        &self,
        facts: RequestFacts,
        payload: Option<RenderPayload>,
        response: HttpResponse,
        redact: impl FnOnce(&mut Value),
    ) -> HttpResponse {
        let now = crate::clock::now();
        let now_ms = now.timestamp_millis();
        let id = ulid::new_id(u64::try_from(now_ms).unwrap_or_default());
        let parent_out = if facts.prefetch {
            id.clone()
        } else {
            facts.batch_id().unwrap_or(&id).to_string()
        };
        let mut response = response
            .replace_header(ID_HEADER, id.clone())
            .replace_header(PARENT_OUT_HEADER, parent_out);
        if !facts.base_path.is_empty() {
            response = response.replace_header(BASE_PATH_HEADER, facts.base_path.clone());
        }
        if let Some(tagged) = tag_first_visit(&facts, payload.as_ref(), &response, &id) {
            response = response.with_static_body(tagged);
        }
        let tab = facts.tab().map(str::to_string);
        // Building the entry and redacting it run under one guard: a panic
        // in either drops the entry and leaves the response as it is.
        let built = AssertUnwindSafe(async {
            let mut entry =
                entry::build(facts, payload, &response, Stamp { id: &id, at: now }).await;
            redact(&mut entry);
            entry
        })
        .catch_unwind()
        .await;
        let Ok(entry) = built else {
            tracing::debug!(
                "Inertia DevTools: building or redacting an entry panicked; the entry is dropped"
            );
            return response;
        };
        self.store(id, entry, tab, now).await;
        response
    }

    /// Store `entry`, enforce its tab's limit and prune when a prune is
    /// due, on the blocking pool, unless a recent write failure holds the
    /// breaker open. A failure is logged at `warn` once, until a write
    /// succeeds again.
    async fn store(
        &self,
        id: String,
        entry: Value,
        tab: Option<String>,
        now: chrono::DateTime<chrono::Utc>,
    ) {
        let path = self.repository.path().to_path_buf();
        let now_ms = now.timestamp_millis();
        if store::is_suppressed(&path, now_ms) {
            return;
        }
        let repository = Arc::clone(&self.repository);
        let config = Arc::clone(&self.config);
        let now_secs = now.timestamp();
        let written = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            repository.save(&id, &entry)?;
            if let Some(tab) = tab.as_deref() {
                repository.enforce_tab_limit(tab, config.limit)?;
            }
            repository.prune_if_due(now_secs, config.prune_interval_secs, config.ttl_hours)
        })
        .await;
        match written {
            Ok(Ok(())) => store::note_success(&path),
            Ok(Err(error)) => {
                if store::note_failure(&path, now_ms) {
                    tracing::warn!(
                        path = %path.display(),
                        error = %error,
                        "Inertia DevTools: failed to persist entry; recording is paused for 30 seconds"
                    );
                }
            }
            Err(join) => {
                if store::note_failure(&path, now_ms) {
                    tracing::warn!(
                        path = %path.display(),
                        error = %join,
                        "Inertia DevTools: failed to persist entry; recording is paused for 30 seconds"
                    );
                }
            }
        }
    }
}

/// The body of `response` with the entry id tag before its last `</body>`,
/// when the response is the first visit of a page: a `200` HTML document
/// answering a request that is not an Inertia visit, for which a page
/// rendered. `None` leaves the body as it is.
///
/// The panel of an extension that attaches after the page loaded has only
/// the document to read the id from. A plain HTML page gets no tag: the
/// extension reads one as DevTools being on for that page.
fn tag_first_visit(
    facts: &RequestFacts,
    payload: Option<&RenderPayload>,
    response: &HttpResponse,
    id: &str,
) -> Option<Bytes> {
    if facts.is_inertia
        || response.status_code() != 200
        || response.is_streaming()
        || payload.is_none_or(|payload| payload.component.is_empty())
    {
        return None;
    }
    let content_type = response.header_value("Content-Type")?.to_ascii_lowercase();
    if !content_type.contains("text/html") {
        return None;
    }
    let body = std::str::from_utf8(response.body()).ok()?;
    let at = body.rfind("</body>")?;
    let base = if facts.base_path.is_empty() {
        String::new()
    } else {
        format!(
            " data-inertia-devtools-base-path=\"{}\"",
            crate::inertia::escape_html_attr(&facts.base_path)
        )
    };
    let tag = format!(
        "<script data-inertia-devtools-id{base} type=\"application/json\">{}</script>",
        Value::String(id.to_string())
    );
    let mut tagged = String::with_capacity(body.len() + tag.len());
    tagged.push_str(&body[..at]);
    tagged.push_str(&tag);
    tagged.push_str(&body[at..]);
    Some(Bytes::from(tagged))
}

#[async_trait]
impl Middleware for DevToolsMiddleware {
    async fn handle(&self, request: Request, next: Next) -> Response {
        if let Some(endpoint) = endpoints::Endpoint::of(&request) {
            return endpoints::answer(&self.config, &self.repository, endpoint, &request).await;
        }
        if !self.records || self.is_excepted(&request) {
            return next(request).await;
        }
        let (request, facts) = match RequestFacts::capture(request).await {
            Ok(captured) => captured,
            Err(response) => return Err(response),
        };
        let recorder = Arc::new(Recorder::default());
        let response = recorder::scope(Arc::clone(&recorder), next(request)).await;
        let was_ok = response.is_ok();
        let http = response.unwrap_or_else(|response| response);
        let http = self.record(facts, recorder.take(), http).await;
        if was_ok { Ok(http) } else { Err(http) }
    }
}

#[cfg(all(test, feature = "testing"))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn indt_a_redaction_that_panics_leaves_the_response_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let middleware =
            DevToolsMiddleware::new(DevToolsConfig::new().enabled(true).storage_path(dir.path()));
        let Ok((_, facts)) = RequestFacts::capture(Request::for_test("GET", "/report")).await
        else {
            panic!("a GET with no body is read");
        };
        let response = HttpResponse::text("the handler's body")
            .status(201)
            .header("X-Handler", "yes");

        let recorded = middleware
            .record_with(facts, None, response, |_| panic!("the redaction failed"))
            .await;

        assert_eq!(recorded.status_code(), 201);
        assert_eq!(recorded.body(), b"the handler's body");
        assert_eq!(recorded.header_value("X-Handler"), Some("yes"));
        assert!(
            recorded.header_value(ID_HEADER).is_some(),
            "the headers still go out"
        );
        let stored = std::fs::read_dir(dir.path())
            .map(|entries| entries.count())
            .unwrap_or(0);
        assert_eq!(stored, 0, "the entry is dropped, nothing is stored");
    }
}
