//! In-memory recorder and canned-response store for `Http::fake()`.
//!
//! Activated by `Http::fake(|| async { ... }).await`: the closure runs
//! inside a `tokio::task_local!` scope where every `RequestBuilder::send`
//! is intercepted, captured into a recorded-requests vec, and matched
//! against canned responses queued via [`fake_response`].
//!
//! Each task gets its own isolated fake state (`Arc<Mutex<FakeState>>`),
//! so tests can run in parallel without serializing themselves on a
//! process-wide mutex.
//!
//! Note: `tokio::task_local!` is task-scoped. Work spawned via
//! `tokio::spawn` inside the scope runs on a fresh task and does NOT
//! inherit the fake by default - those requests escape to the real
//! network (or fail-closed when `Http::fail_on_real_calls()` is on).
//!
//! For the cases that actually want spawned tasks to share the parent's
//! fake (e.g. a queue worker that calls outbound HTTP from within a
//! spawned future), [`Http::spawn_with_fake_inheritance`] captures the
//! current fake state via the shared `Arc` and re-installs it in the
//! child's `tokio::task_local!` scope. Recorded requests and consumed
//! canned responses are shared with the parent through the same Arc -
//! `assert_sent` on the parent sees what the child sent.

use crate::lock;
use std::future::Future;
use std::sync::{Arc, Mutex};

use bytes::Bytes;

use super::{ClientResponse, GlobalConfiguration, Http, Prepared, RequestBuilder};
use crate::http::glob_match;

tokio::task_local! {
    /// Per-task fake state. Set by [`Http::fake`]. Inside the scope,
    /// `is_fake_active()` returns `true` and `intercept` reads / mutates
    /// the state. Outside, all of them return `false` / panic with a
    /// friendly error.
    ///
    /// `Arc<Mutex<...>>` (rather than `Mutex<...>` directly) so the
    /// state can be cloned cheaply and re-installed in spawned tasks
    /// via [`Http::spawn_with_fake_inheritance`]; recorded requests
    /// from the child remain visible to the parent through the same
    /// Arc.
    static FAKE_STATE: Arc<Mutex<FakeState>>;
}

#[derive(Default)]
pub(crate) struct FakeState {
    recorded: Vec<RecordedRequest>,
    canned: Vec<CannedResponse>,
    /// The stubs that stay: [`Http::fake_url`], [`Http::fake_using`] and
    /// [`Http::fake_sequence`], asked in the order they were registered.
    stubs: Vec<Stub>,
    /// The URL patterns of [`Http::allow_stray_requests`].
    allowed_stray: Vec<String>,
    /// The global middleware and options registered inside this fake: they
    /// belong to the fake, so parallel tests do not see each other's.
    pub(crate) global: GlobalConfiguration,
}

/// A recorded outbound request - used by [`assert_sent`] /
/// [`assert_not_sent`].
#[derive(Debug, Clone)]
pub struct RecordedRequest {
    /// HTTP method as a static string: `"GET"`, `"POST"`, etc.
    pub method: String,
    /// The URL the request is sent to: the URL passed to
    /// `Http::get`/`post`/etc., with its base URL, URL parameters and
    /// query applied.
    pub url: String,
    /// The headers the request is sent with, in order: the ones the
    /// request and the global middleware added, then the `Content-Type`
    /// its body sets and the user agent, unless the request set them.
    pub headers: Vec<(String, String)>,
    /// Raw body bytes (JSON serialized as JSON, form serialized as
    /// urlencoded, multipart as its encoded parts, raw passed through).
    pub body: Option<Vec<u8>>,
}

impl RecordedRequest {
    /// The first value of the header `name`, compared without regard to
    /// case, as Laravel's `Request::header` reads it.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// Whether the request was sent with the header `name` set to exactly
    /// `value`; the name is compared without regard to case (Laravel's
    /// `hasHeader($name, $value)`).
    pub fn has_header(&self, name: &str, value: &str) -> bool {
        self.headers
            .iter()
            .any(|(header, sent)| header.eq_ignore_ascii_case(name) && sent == value)
    }

    /// Whether the body is JSON: the `Content-Type` names `json`, as
    /// Laravel's `isJson` decides.
    pub fn is_json(&self) -> bool {
        self.header("content-type")
            .is_some_and(|content_type| content_type.contains("json"))
    }

    /// Whether the body is an URL-encoded form (Laravel's `isForm`).
    pub fn is_form(&self) -> bool {
        self.header("content-type").is_some_and(|content_type| {
            content_type.split(';').next().is_some_and(|media| {
                media
                    .trim()
                    .eq_ignore_ascii_case("application/x-www-form-urlencoded")
            })
        })
    }

    /// Whether the body is `multipart/form-data`, as
    /// [`RequestBuilder::attach`] sends it (Laravel's `isMultipart`).
    pub fn is_multipart(&self) -> bool {
        self.header("content-type")
            .is_some_and(|content_type| content_type.contains("multipart"))
    }
}

/// A response a stub answers with: [`Http::fake_url`],
/// [`Http::fake_using`] and [`Http::fake_sequence`] take it (Laravel's
/// `Http::response`).
#[derive(Debug, Clone)]
pub struct FakeResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Bytes,
}

impl FakeResponse {
    /// A response with `status`, no headers and an empty body.
    pub fn new(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Bytes::new(),
        }
    }

    /// A response with `status` and `body` as JSON, with
    /// `Content-Type: application/json`.
    pub fn json(status: u16, body: serde_json::Value) -> Self {
        Self::new(status)
            .header("content-type", "application/json")
            .body(body.to_string())
    }

    /// A response with `status` and `body` as it is, with
    /// `Content-Type: text/plain; charset=utf-8`.
    pub fn text(status: u16, body: impl Into<String>) -> Self {
        Self::new(status)
            .header("content-type", "text/plain; charset=utf-8")
            .body(body.into())
    }

    /// Add a header to the response.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Replace the body. The `Content-Type` is not changed.
    pub fn body(mut self, body: impl Into<Bytes>) -> Self {
        self.body = body.into();
        self
    }

    fn to_response(&self) -> ClientResponse {
        ClientResponse::fake(self.status, self.headers.clone(), self.body.clone())
    }
}

/// The responses of [`Http::fake_sequence`], answered in turn (Laravel's
/// `ResponseSequence`).
///
/// The fake holds the same sequence, so responses pushed after the call
/// are answered too. Once the responses run out, a request fails, unless
/// [`when_empty`](Self::when_empty) gives the response to answer then or
/// [`dont_fail_when_empty`](Self::dont_fail_when_empty) makes it an empty
/// `200`.
#[derive(Clone, Default)]
pub struct ResponseSequence {
    state: Arc<Mutex<SequenceState>>,
}

#[derive(Default)]
struct SequenceState {
    responses: std::collections::VecDeque<FakeResponse>,
    when_empty: Option<FakeResponse>,
}

impl ResponseSequence {
    fn state(&self) -> std::sync::MutexGuard<'_, SequenceState> {
        // The queue is whole after any panic, so a poisoned lock goes on
        // with it.
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Answer `response` after the ones before it.
    pub fn push(self, response: FakeResponse) -> Self {
        self.state().responses.push_back(response);
        self
    }

    /// Answer an empty response with `status` after the ones before it
    /// (Laravel's `pushStatus`).
    pub fn push_status(self, status: u16) -> Self {
        self.push(FakeResponse::new(status))
    }

    /// Answer `response` to every request once the sequence runs out.
    pub fn when_empty(self, response: FakeResponse) -> Self {
        self.state().when_empty = Some(response);
        self
    }

    /// Answer an empty `200` once the sequence runs out, instead of
    /// failing the request.
    pub fn dont_fail_when_empty(self) -> Self {
        self.when_empty(FakeResponse::new(200))
    }

    /// Whether every pushed response has been answered.
    pub fn is_empty(&self) -> bool {
        self.state().responses.is_empty()
    }

    fn next(&self, pattern: &str, url: &str) -> Result<ClientResponse, crate::FrameworkError> {
        let mut state = self.state();
        match state.responses.pop_front() {
            Some(response) => Ok(response.to_response()),
            None => match &state.when_empty {
                Some(response) => Ok(response.to_response()),
                None => Err(crate::FrameworkError::internal(format!(
                    "Http::fake_sequence(\"{pattern}\"): the response sequence is empty, and \
                     {url} asked it for another response. Push more responses, or give it \
                     when_empty(...) or dont_fail_when_empty()."
                ))),
            },
        }
    }
}

/// The function behind [`Http::fake_using`].
type StubCallback = dyn Fn(&RecordedRequest) -> Option<FakeResponse> + Send + Sync;

/// A stub that stays, asked in the order it was registered.
#[derive(Clone)]
enum Stub {
    Url {
        pattern: String,
        response: FakeResponse,
    },
    Callback(Arc<StubCallback>),
    Sequence {
        pattern: String,
        sequence: ResponseSequence,
    },
}

/// Whether `url` matches a stub `pattern`: `*` matches any run of
/// characters, and a leading `*` is implied, as Laravel's `stubUrl` puts
/// one in front of the pattern before it calls `Str::is`.
fn stub_matches(pattern: &str, url: &str) -> bool {
    if pattern.starts_with('*') {
        glob_match(pattern, url)
    } else {
        glob_match(&format!("*{pattern}"), url)
    }
}

struct CannedResponse {
    method: String,
    url_substring: String,
    status: u16,
    body: Bytes,
    /// `content-type` header value the intercepted response answers with.
    /// `"application/json"` for [`fake_response`], `"text/plain; charset=utf-8"`
    /// for [`fake_response_text`] - the two entry points differ only in how
    /// the body is *produced* (JSON-encoded vs. verbatim bytes); this field
    /// keeps the header truthful either way.
    content_type: String,
}

/// Queue a canned response. The first request whose method matches
/// (case-insensitive) and whose URL contains `url_substring` returns
/// this response - and the canned entry is consumed.
///
/// Method `"*"` matches any method.
///
/// Subsequent matching requests fall through to the next canned entry,
/// then to the stubs of [`Http::fake_url`], [`Http::fake_using`] and
/// [`Http::fake_sequence`], or - if none match - return an empty `200 {}`.
///
/// **Must be called inside a `Http::fake(|| async { ... })` scope.**
/// Panics if no fake scope is active on the current task.
pub fn fake_response(method: &str, url_substring: &str, status: u16, body: serde_json::Value) {
    let bytes =
        serde_json::to_vec(&body).expect("fake_response body (a serde_json::Value) must serialize");
    with_state(|s| {
        s.canned.push(CannedResponse {
            method: method.to_string(),
            url_substring: url_substring.to_string(),
            status,
            body: Bytes::from(bytes),
            content_type: "application/json".to_string(),
        });
    });
}

/// Queue a canned response whose body is sent back to the caller verbatim
/// as text, without JSON-encoding - the raw-body sibling of
/// [`fake_response`] for upstream APIs that speak `text/plain` rather than
/// JSON (the HIBP k-anonymity range endpoint [`crate::HibpVerifier`] calls
/// is the motivating case). Same method/URL-substring matching and
/// consume-on-match semantics as `fake_response`; the response's
/// `content-type` header is `text/plain; charset=utf-8` instead of
/// `application/json`.
///
/// Reached through [`crate::http_client::Http::fake_response_text`], the
/// public entry point - kept `pub(crate)` here because there is no reason
/// for a caller to reach the `fake` module directly.
///
/// **Must be called inside a `Http::fake(|| async { ... })` scope.**
/// Panics if no fake scope is active on the current task.
pub(crate) fn fake_response_text(method: &str, url_substring: &str, status: u16, body: &str) {
    with_state(|s| {
        s.canned.push(CannedResponse {
            method: method.to_string(),
            url_substring: url_substring.to_string(),
            status,
            body: Bytes::from(body.as_bytes().to_vec()),
            content_type: "text/plain; charset=utf-8".to_string(),
        });
    });
}

/// Assert that at least one recorded request satisfies the predicate.
/// Panics with a list of recorded requests on failure.
///
/// Must be called inside a `Http::fake(...)` scope.
pub fn assert_sent(predicate: impl Fn(&RecordedRequest) -> bool) {
    with_state(|s| {
        if !s.recorded.iter().any(&predicate) {
            panic!(
                "assert_sent: no recorded request matched the predicate. \
                 Recorded (header values and bodies redacted):{}",
                redacted(&s.recorded)
            );
        }
    });
}

/// Assert that no recorded request satisfies the predicate. Panics
/// with the offending request on failure.
///
/// Must be called inside a `Http::fake(...)` scope.
pub fn assert_not_sent(predicate: impl Fn(&RecordedRequest) -> bool) {
    with_state(|s| {
        if let Some(hit) = s.recorded.iter().find(|r| predicate(r)) {
            panic!(
                "assert_not_sent: forbidden request was sent (header values and \
                 bodies redacted):{}",
                redacted(std::slice::from_ref(hit))
            );
        }
    });
}

/// Run `f` inside a task-local fake scope. While `f` is awaiting, every
/// outbound HTTP call on the same task is intercepted instead of
/// hitting the network.
///
/// Returns whatever the closure returns.
///
/// # Example
///
/// ```rust,no_run
/// # use suprnova::{Http, fake_response, assert_sent};
/// # use suprnova::serde_json;
/// # async fn ex() -> Result<(), Box<dyn std::error::Error>> {
/// Http::fake(|| async {
///     fake_response("POST", "/api/users", 201, serde_json::json!({"id": 1}));
///     let resp = Http::post("https://example.com/api/users")
///         .json(&serde_json::json!({"name": "Ada"}))
///         .send()
///         .await
///         .unwrap();
///     assert_eq!(resp.status(), 201);
///     assert_sent(|r| r.method == "POST" && r.url.contains("/api/users"));
/// })
/// .await;
/// # Ok(()) }
/// ```
pub async fn install_fake_scope<F, Fut, T>(f: F) -> T
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = T>,
{
    FAKE_STATE
        .scope(Arc::new(Mutex::new(FakeState::default())), f())
        .await
}

/// Install a previously-captured `Arc<Mutex<FakeState>>` for the
/// duration of `fut`, so a spawned task can share the parent's
/// recorded-requests / canned-responses store. Used by
/// [`Http::spawn_with_fake_inheritance`].
pub(crate) async fn install_inherited_scope<F, T>(state: Arc<Mutex<FakeState>>, fut: F) -> T
where
    F: Future<Output = T>,
{
    FAKE_STATE.scope(state, fut).await
}

/// Capture the current task's fake state for re-installation in a
/// spawned task. Returns `None` when no fake scope is active on the
/// current task - in which case the caller should fall through to a
/// regular `tokio::spawn` rather than asserting inheritance.
pub(crate) fn snapshot_current_fake_state() -> Option<Arc<Mutex<FakeState>>> {
    FAKE_STATE.try_with(|state| state.clone()).ok()
}

/// `true` if a fake scope is active on the current task. Used by
/// `RequestBuilder::send` to decide whether to short-circuit.
pub(crate) fn is_fake_active() -> bool {
    // `try_with` returns Ok if the task_local is in scope.
    FAKE_STATE.try_with(|_| ()).is_ok()
}

/// What the fake does with a request.
pub(crate) enum Interception {
    /// The fake answered: a stub's response, the default `200 {}`, or the
    /// refusal of a stray request.
    Answered(Result<ClientResponse, crate::FrameworkError>),
    /// No stub answered and the URL is one
    /// [`Http::allow_stray_requests`] allows: the request goes to the
    /// network.
    Network,
}

/// Record `req` as `prepared` describes it and decide what answers it:
/// a [`fake_response`] entry first, used up as it answers, then the
/// stubs that stay, in the order they were registered, then the
/// allowlist of stray requests.
pub(crate) fn intercept(req: &RequestBuilder, prepared: &Prepared) -> Interception {
    let request = RecordedRequest {
        method: req.method.as_str().to_string(),
        url: req.url.clone(),
        headers: prepared.headers.clone(),
        body: prepared.body.as_ref().map(|body| body.to_vec()),
    };
    let method_str = req.method.as_str();

    let (canned, stubs, allowed) = with_state(|s| {
        s.recorded.push(request.clone());
        let idx = s.canned.iter().position(|c| {
            let m_ok = c.method == "*" || c.method.eq_ignore_ascii_case(method_str);
            m_ok && req.url.contains(&c.url_substring)
        });
        let canned = idx.map(|i| s.canned.remove(i));
        let allowed = s
            .allowed_stray
            .iter()
            .any(|pattern| glob_match(pattern, &req.url));
        (canned, s.stubs.clone(), allowed)
    });

    if let Some(c) = canned {
        return Interception::Answered(Ok(ClientResponse::fake(
            c.status,
            vec![("content-type".to_string(), c.content_type.clone())],
            c.body,
        )));
    }

    // The state is not locked while a stub runs: a `fake_using` callback
    // may call the fake's own helpers.
    for stub in &stubs {
        let answer = match stub {
            Stub::Url { pattern, response } => {
                stub_matches(pattern, &request.url).then(|| Ok(response.to_response()))
            }
            Stub::Callback(callback) => {
                callback(&request).map(|response| Ok(response.to_response()))
            }
            Stub::Sequence { pattern, sequence } => {
                stub_matches(pattern, &request.url).then(|| sequence.next(pattern, &request.url))
            }
        };
        if let Some(answer) = answer {
            return Interception::Answered(answer);
        }
    }

    if allowed {
        return Interception::Network;
    }
    if Http::is_guarded() {
        // No stub matched. With the fail-closed guard active, a drifted
        // URL/method must fail loudly rather than silently returning an
        // empty 200 that masks the mismatch.
        return Interception::Answered(Err(crate::FrameworkError::internal(format!(
            "Http::fake: no canned response matched {} {} while \
             Http::fail_on_real_calls is active. Register a matching \
             fake_response(...), allow the URL with Http::allow_stray_requests, \
             or release the guard to allow the default empty 200 response.",
            method_str, req.url
        ))));
    }
    Interception::Answered(Ok(ClientResponse::fake(
        200,
        vec![("content-type".to_string(), "application/json".to_string())],
        Bytes::from_static(b"{}"),
    )))
}

/// Answer every request whose URL matches `pattern` with `response`.
pub(crate) fn fake_url(pattern: &str, response: FakeResponse) {
    with_state(|s| {
        s.stubs.push(Stub::Url {
            pattern: pattern.to_string(),
            response,
        });
    });
}

/// Ask `callback` for the response to each request.
pub(crate) fn fake_using(callback: Arc<StubCallback>) {
    with_state(|s| s.stubs.push(Stub::Callback(callback)));
}

/// Answer the requests whose URL matches `pattern` from a new sequence.
pub(crate) fn fake_sequence(pattern: &str) -> ResponseSequence {
    let sequence = ResponseSequence::default();
    with_state(|s| {
        s.stubs.push(Stub::Sequence {
            pattern: pattern.to_string(),
            sequence: sequence.clone(),
        });
    });
    sequence
}

/// Let the stray requests whose URL matches one of `patterns` reach the
/// network, replacing the patterns before.
pub(crate) fn allow_stray_requests(patterns: &[&str]) {
    with_state(|s| {
        s.allowed_stray = patterns.iter().map(|pattern| pattern.to_string()).collect();
    });
}

/// Run `configure` on the global configuration of the fake active on this
/// task. `false`, without running it, when no fake is active.
pub(crate) fn configure_scoped(configure: impl FnOnce(&mut GlobalConfiguration)) -> bool {
    if !is_fake_active() {
        return false;
    }
    with_state(|s| configure(&mut s.global));
    true
}

/// The global configuration registered inside the fake active on this
/// task, when one is.
pub(crate) fn scoped_global() -> Option<GlobalConfiguration> {
    FAKE_STATE
        .try_with(|state| {
            lock::lock(state, "http fake state")
                .map(|guard| guard.global.clone())
                .ok()
        })
        .ok()
        .flatten()
}

/// Access the per-task `FakeState`. Panics if no scope is active.
fn with_state<R>(f: impl FnOnce(&mut FakeState) -> R) -> R {
    FAKE_STATE
        .try_with(|state| {
            let mut guard = lock::lock(state, "http fake state").expect("FakeState mutex poisoned");
            f(&mut guard)
        })
        .unwrap_or_else(|_| {
            panic!(
                "Http fake helpers called outside an Http::fake(...) scope. \
                 Wrap the test body in Http::fake(|| async {{ ... }}).await."
            )
        })
}

/// Format recorded requests for assertion-failure messages WITHOUT
/// leaking secrets. Header values and body bytes routinely carry bearer
/// tokens, API keys, and webhook payloads, so only the method, URL, a
/// small allowlist of non-sensitive header names, and a body byte count
/// are shown; every other header value and the body itself are redacted.
fn redacted(reqs: &[RecordedRequest]) -> String {
    const SAFE_HEADERS: &[&str] = &["content-type", "accept", "user-agent"];
    let mut out = String::new();
    for r in reqs {
        out.push_str(&format!("\n  {} {}", r.method, r.url));
        for (name, value) in &r.headers {
            if SAFE_HEADERS.iter().any(|h| h.eq_ignore_ascii_case(name)) {
                out.push_str(&format!("\n    {name}: {value}"));
            } else {
                out.push_str(&format!("\n    {name}: <redacted>"));
            }
        }
        match &r.body {
            Some(b) => out.push_str(&format!("\n    body: <{} bytes>", b.len())),
            None => out.push_str("\n    body: <none>"),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacted_hides_header_values_and_body_bytes() {
        let reqs = vec![RecordedRequest {
            method: "POST".to_string(),
            url: "https://api.test/charge".to_string(),
            headers: vec![
                (
                    "Authorization".to_string(),
                    "Bearer super-secret-token".to_string(),
                ),
                ("Content-Type".to_string(), "application/json".to_string()),
            ],
            body: Some(br#"{"card":"4242424242424242"}"#.to_vec()),
        }];
        let out = redacted(&reqs);
        // Method, URL, and allowlisted header values are shown.
        assert!(out.contains("POST https://api.test/charge"), "{out}");
        assert!(out.contains("Content-Type: application/json"), "{out}");
        // Sensitive header value and body bytes are NOT shown.
        assert!(
            !out.contains("super-secret-token"),
            "auth value leaked: {out}"
        );
        assert!(out.contains("Authorization: <redacted>"), "{out}");
        assert!(!out.contains("4242424242424242"), "body leaked: {out}");
        assert!(out.contains("body: <"), "{out}");
    }
}
