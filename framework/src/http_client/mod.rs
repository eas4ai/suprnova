//! Outbound HTTP client.
//!
//! `Http::get`, `Http::post`, etc. return a [`RequestBuilder`]; call
//! `.send().await` to execute and get a [`ClientResponse`]. The backing
//! `reqwest::Client` uses rustls for TLS, has a 30s default timeout, and
//! identifies itself with the `suprnova/<version>` user agent.
//!
//! For tests, [`Http::fake`] runs your async closure inside a
//! `tokio::task_local!` scope. Every outbound request from inside that
//! scope is captured into an in-memory recorder and answered with the
//! canned responses you've queued via `fake_response(...)`. Because the
//! state is task-local, tests can run in parallel without coordinating.
//!
//! Two additional helpers cover the corner where task-local isolation
//! is the wrong default:
//!
//! - [`Http::fail_on_real_calls`] flips a process-global guard so any
//!   outbound request that doesn't match an active fake errors out
//!   (with a `FrameworkError::internal` instead of hitting the
//!   network). This catches accidental escape from a fake scope -
//!   e.g. a `tokio::spawn` that doesn't inherit task-local state.
//!   Use [`FailOnRealCallsGuard`] for the RAII pattern that resets on
//!   drop, so a test forgetting to call `allow_real_calls` doesn't
//!   poison other tests.
//!
//! - [`Http::spawn_with_fake_inheritance`] is the explicit opt-in for
//!   spawning a task that inherits the parent's fake state. Recorded
//!   requests and consumed canned responses are shared with the
//!   parent - `assert_sent` on the parent sees what the child sent.

pub(crate) mod fake;
pub(crate) mod vendor;

use std::borrow::Borrow;
use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;

use bytes::Bytes;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Serialize;

use crate::FrameworkError;

pub use fake::{
    FakeResponse, RecordedRequest, ResponseSequence, assert_not_sent, assert_sent, fake_response,
};

/// Process-global install count raised by [`Http::fail_on_real_calls`].
/// While nonzero, [`RequestBuilder::send`] refuses to hit the real
/// network - every outbound call that isn't intercepted by an active
/// fake returns an error.
///
/// The count is process-global by design: the goal is to fail closed on
/// accidental network escape from spawned tasks that don't inherit the
/// caller's task-local fake. Tests that flip this should use
/// [`FailOnRealCallsGuard`] (or call [`Http::allow_real_calls`] in
/// teardown) so the flag doesn't leak between tests.
static FAIL_ON_REAL_CALLS: AtomicUsize = AtomicUsize::new(0);

static REQWEST_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
static REQWEST_CLIENT_NO_REDIRECT: OnceLock<reqwest::Client> = OnceLock::new();

/// The clients built for a request's own connect timeout, by the timeout
/// and whether they follow redirects. reqwest sets the connect timeout on
/// the client, not on the request, so each distinct timeout gets a client
/// of its own, built once and kept, as the two default clients are.
static CONNECT_TIMEOUT_CLIENTS: Mutex<Option<HashMap<(Duration, bool), reqwest::Client>>> =
    Mutex::new(None);

/// A function every request passes through before it is sent.
pub(crate) type RequestMiddleware = dyn Fn(RequestBuilder) -> RequestBuilder + Send + Sync;

/// A function every response passes through before the caller sees it.
pub(crate) type ResponseMiddleware = dyn Fn(ClientResponse) -> ClientResponse + Send + Sync;

/// The global middleware and options, Laravel's factory-level
/// `globalMiddleware` and `globalOptions`.
#[derive(Clone, Default)]
pub(crate) struct GlobalConfiguration {
    request: Vec<Arc<RequestMiddleware>>,
    response: Vec<Arc<ResponseMiddleware>>,
    options: Option<Arc<RequestMiddleware>>,
}

impl GlobalConfiguration {
    /// This configuration followed by `later`: the middleware of both, in
    /// order, and the options of `later` when it has any.
    fn then(mut self, later: GlobalConfiguration) -> Self {
        self.request.extend(later.request);
        self.response.extend(later.response);
        if later.options.is_some() {
            self.options = later.options;
        }
        self
    }
}

/// The process-wide global configuration. Registrations made inside an
/// [`Http::fake`] scope go to that scope instead.
static GLOBAL_CONFIGURATION: RwLock<GlobalConfiguration> = RwLock::new(GlobalConfiguration {
    request: Vec::new(),
    response: Vec::new(),
    options: None,
});

tokio::task_local! {
    /// Set by [`Http::without_global_configuration`]: requests created on
    /// this task while it is set take no global middleware or options.
    static WITHOUT_GLOBAL_CONFIGURATION: ();
}

/// Change the global configuration: the one of the [`Http::fake`] scope
/// active on this task, or else the process-wide one.
fn configure_global(configure: impl FnOnce(&mut GlobalConfiguration)) {
    let mut configure = Some(configure);
    let scoped = fake::configure_scoped(|global| {
        if let Some(configure) = configure.take() {
            configure(global);
        }
    });
    if !scoped && let Some(configure) = configure.take() {
        // The lists are whole after any panic, so a poisoned lock goes on
        // with them.
        let mut global = GLOBAL_CONFIGURATION
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        configure(&mut global);
    }
}

/// The global configuration a request created now takes: none inside
/// [`Http::without_global_configuration`], else the process-wide one
/// followed by the one of the active [`Http::fake`] scope.
fn current_global_configuration() -> GlobalConfiguration {
    if WITHOUT_GLOBAL_CONFIGURATION.try_with(|()| ()).is_ok() {
        return GlobalConfiguration::default();
    }
    let process = GLOBAL_CONFIGURATION
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    match fake::scoped_global() {
        Some(scoped) => process.then(scoped),
        None => process,
    }
}

/// Default cap on a buffered outbound response body (25 MiB). A slow or
/// malicious upstream can otherwise stream an unbounded body into memory
/// via `ClientResponse::json`/`text`/`bytes`. Override globally with
/// [`Http::set_max_response_bytes`] or per request with
/// [`RequestBuilder::max_response_bytes`].
pub(crate) const DEFAULT_MAX_RESPONSE_BODY_BYTES: usize = 25 * 1024 * 1024;

/// Process-global response-body cap. `0` means "unset" - readers fall
/// back to [`DEFAULT_MAX_RESPONSE_BODY_BYTES`].
static MAX_RESPONSE_BODY_BYTES: AtomicUsize = AtomicUsize::new(0);

/// Shared builder config for both clients: rustls TLS, a 30s overall
/// timeout, a 10s connect timeout, and the suprnova user-agent. The
/// redirect policy is layered on by each caller.
fn base_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .user_agent(USER_AGENT)
}

/// The client for a request: one of the two shared clients, or, for a
/// request with its own connect timeout, the client kept for that timeout.
fn client_for(
    no_redirects: bool,
    connect_timeout: Option<Duration>,
) -> Result<reqwest::Client, FrameworkError> {
    let Some(connect_timeout) = connect_timeout else {
        return Ok(if no_redirects {
            client_no_redirect().clone()
        } else {
            client().clone()
        });
    };
    // The map only grows by whole entries, so a poisoned lock goes on with
    // it.
    let mut clients = CONNECT_TIMEOUT_CLIENTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let clients = clients.get_or_insert_with(HashMap::new);
    if let Some(client) = clients.get(&(connect_timeout, no_redirects)) {
        return Ok(client.clone());
    }
    let mut builder = base_builder().connect_timeout(connect_timeout);
    if no_redirects {
        builder = builder.redirect(reqwest::redirect::Policy::none());
    }
    let client = builder.build().map_err(|e| {
        FrameworkError::internal(format!(
            "Http: building a client with a {connect_timeout:?} connect timeout failed: {e}"
        ))
    })?;
    clients.insert((connect_timeout, no_redirects), client.clone());
    Ok(client)
}

/// The default client. Follows redirects (reqwest's default cap of 10),
/// matching general-purpose HTTP-client convention. Callers passing a
/// user-influenced URL that want to avoid redirect-based SSRF should use
/// [`RequestBuilder::no_redirects`], which routes through
/// [`client_no_redirect`].
fn client() -> &'static reqwest::Client {
    REQWEST_CLIENT.get_or_init(|| {
        base_builder()
            .build()
            .expect("reqwest::Client::builder().build() - rustls available")
    })
}

/// A client that never follows redirects - a 3xx is returned to the caller
/// as-is. Selected per request via [`RequestBuilder::no_redirects`]. Use it
/// when the request URL is influenced by untrusted input: a redirect to an
/// internal or cloud-metadata host (SSRF) surfaces as a 3xx response rather
/// than being silently followed.
fn client_no_redirect() -> &'static reqwest::Client {
    REQWEST_CLIENT_NO_REDIRECT.get_or_init(|| {
        base_builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("reqwest::Client::builder().build() - rustls available")
    })
}

/// Static facade for outbound HTTP requests. Closed for v1 - we do not
/// expose the underlying `reqwest::Client`. To grow the surface, add
/// methods here.
pub struct Http;

impl Http {
    /// Begin a GET request.
    pub fn get(url: impl Into<String>) -> RequestBuilder {
        RequestBuilder::new(Method::Get, url.into())
    }

    /// Begin a POST request.
    pub fn post(url: impl Into<String>) -> RequestBuilder {
        RequestBuilder::new(Method::Post, url.into())
    }

    /// Begin a PUT request.
    pub fn put(url: impl Into<String>) -> RequestBuilder {
        RequestBuilder::new(Method::Put, url.into())
    }

    /// Begin a PATCH request.
    pub fn patch(url: impl Into<String>) -> RequestBuilder {
        RequestBuilder::new(Method::Patch, url.into())
    }

    /// Begin a DELETE request.
    pub fn delete(url: impl Into<String>) -> RequestBuilder {
        RequestBuilder::new(Method::Delete, url.into())
    }

    /// Begin a HEAD request: the response has the status and headers of a
    /// GET and no body (Laravel's `head`).
    pub fn head(url: impl Into<String>) -> RequestBuilder {
        RequestBuilder::new(Method::Head, url.into())
    }

    /// Pass every request created after this call through `middleware`
    /// before it is sent (Laravel's `globalRequestMiddleware`).
    ///
    /// The middleware receives the request with its final URL and returns
    /// the request to send; it can add headers, a token, or a signature.
    /// It runs once per [`RequestBuilder::send`], before the first attempt,
    /// in the order the middleware was registered. The fake records the
    /// request as the middleware left it.
    ///
    /// Registered outside a fake it is process-wide: call it during boot.
    /// Registered inside an [`Http::fake`] scope it belongs to that scope,
    /// so tests running in parallel do not see each other's. The requests
    /// the framework's vendor drivers send (mail providers, Pinecone) do not
    /// go through the facade and stay outside it.
    pub fn global_request_middleware(
        middleware: impl Fn(RequestBuilder) -> RequestBuilder + Send + Sync + 'static,
    ) {
        let middleware: Arc<RequestMiddleware> = Arc::new(middleware);
        configure_global(|global| global.request.push(middleware));
    }

    /// Pass the response of every request created after this call through
    /// `middleware` before the caller sees it (Laravel's
    /// `globalResponseMiddleware`).
    ///
    /// It sees every response, a faked one included, and each attempt of
    /// a retried request; an attempt that got no response has nothing to
    /// pass. Where it is registered decides where it applies, as for
    /// [`Http::global_request_middleware`].
    pub fn global_response_middleware(
        middleware: impl Fn(ClientResponse) -> ClientResponse + Send + Sync + 'static,
    ) {
        let middleware: Arc<ResponseMiddleware> = Arc::new(middleware);
        configure_global(|global| global.response.push(middleware));
    }

    /// Set the options every request starts with (Laravel's
    /// `globalOptions`): `options` runs on each new request as it is
    /// created, so what the request sets itself comes after and wins where
    /// a setting replaces, such as a timeout. Headers are appended, so a
    /// header set both ways is sent twice, the global one first.
    ///
    /// Calling it again replaces the options, as Laravel's does. Where it
    /// is called decides where it applies, as for
    /// [`Http::global_request_middleware`].
    pub fn global_options(
        options: impl Fn(RequestBuilder) -> RequestBuilder + Send + Sync + 'static,
    ) {
        let options: Arc<RequestMiddleware> = Arc::new(options);
        configure_global(|global| global.options = Some(options));
    }

    /// Run `f` with the requests it creates taking no global middleware
    /// and no global options (Laravel's `withoutGlobalConfiguration`).
    ///
    /// Only the requests created on this task inside `f` are left out:
    /// the scope is a `tokio::task_local!`, so other requests of the
    /// process keep their middleware while `f` runs, and work `f` spawns
    /// with `tokio::spawn` does not inherit the scope.
    pub async fn without_global_configuration<F, Fut, T>(f: F) -> T
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = T>,
    {
        WITHOUT_GLOBAL_CONFIGURATION
            .scope((), async move { f().await })
            .await
    }

    /// Answer every request whose URL matches `pattern` with `response`,
    /// for as long as the fake lives (Laravel's `Http::fake([$url =>
    /// $response])`).
    ///
    /// `*` matches any run of characters, and a leading `*` is implied, so
    /// `"api.test/users/*"` matches `https://api.test/users/7`. Unlike
    /// [`fake_response`], the stub is not used up. A [`fake_response`]
    /// entry that matches answers first; the stubs of `fake_url`,
    /// [`Http::fake_using`] and [`Http::fake_sequence`] are then asked in
    /// the order they were registered.
    ///
    /// **Must be called inside a `Http::fake(|| async { ... })` scope.**
    /// Panics if no fake scope is active on the current task.
    pub fn fake_url(pattern: &str, response: FakeResponse) {
        fake::fake_url(pattern, response);
    }

    /// Ask `callback` for the response to each request: `Some` answers it,
    /// and `None` lets the stubs registered after it, and then the
    /// default, answer (Laravel's `Http::fake(fn ($request) => ...)`).
    ///
    /// The callback receives the request as it is recorded, so it can
    /// read the URL, the headers and the body.
    ///
    /// **Must be called inside a `Http::fake(|| async { ... })` scope.**
    /// Panics if no fake scope is active on the current task.
    pub fn fake_using(
        callback: impl Fn(&RecordedRequest) -> Option<FakeResponse> + Send + Sync + 'static,
    ) {
        fake::fake_using(Arc::new(callback));
    }

    /// Answer the requests whose URL matches `pattern`, as for
    /// [`Http::fake_url`], with the responses of the returned sequence in
    /// turn (Laravel's `fakeSequence`). Push the responses on the sequence
    /// it returns.
    ///
    /// **Must be called inside a `Http::fake(|| async { ... })` scope.**
    /// Panics if no fake scope is active on the current task.
    pub fn fake_sequence(pattern: &str) -> ResponseSequence {
        fake::fake_sequence(pattern)
    }

    /// Switch the refusal of stray requests on or off (Laravel's
    /// `preventStrayRequests`). While it is on, a request that no fake
    /// answers fails instead of reaching the network.
    ///
    /// It is the process-wide switch [`Self::fail_on_real_calls`] and
    /// [`Self::allow_real_calls`] move: `true` installs one hold on it, as
    /// `fail_on_real_calls` does, and `false` releases every hold, as
    /// `allow_real_calls` does.
    pub fn prevent_stray_requests(on: bool) {
        if on {
            Self::fail_on_real_calls();
        } else {
            Self::allow_real_calls();
        }
    }

    /// Whether stray requests are refused (Laravel's
    /// `preventingStrayRequests`); [`Self::is_guarded`] under its Laravel
    /// name.
    pub fn preventing_stray_requests() -> bool {
        Self::is_guarded()
    }

    /// Let a request inside this [`Http::fake`] scope that no stub
    /// answers reach the network when its URL matches one of `patterns`,
    /// even while stray requests are refused (Laravel's
    /// `allowStrayRequests`). `*` matches any run of characters; no
    /// leading `*` is implied. Calling it again replaces the patterns.
    ///
    /// Such a request is still recorded, and goes out through the real
    /// client with its global middleware. Without the refusal on, a URL on
    /// the list reaches the network too, where the fake would otherwise
    /// answer an empty `200`.
    ///
    /// **Must be called inside a `Http::fake(|| async { ... })` scope.**
    /// Panics if no fake scope is active on the current task.
    pub fn allow_stray_requests(patterns: &[&str]) {
        fake::allow_stray_requests(patterns);
    }

    /// Run an async test body inside a fake-HTTP scope.
    ///
    /// Every `RequestBuilder::send` invoked from inside `f` is
    /// intercepted: the request is captured, and a canned response
    /// queued via [`fake_response`] is returned. Tests in different
    /// tasks see different fake states, so parallel test execution is
    /// safe.
    ///
    /// **Caveat:** the scope is `tokio::task_local!`, which is scoped
    /// to the *current* task only. Work spawned via `tokio::spawn`
    /// inside `f` runs on a different task and will NOT see the fake -
    /// those requests will hit the real network. If you need a
    /// spawned task to use the fake, capture the work into a closure
    /// and `await` it directly, or pass the fake scope's data through
    /// explicit channels.
    ///
    /// Returns whatever the closure returns.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use suprnova::{Http, fake_response, assert_sent};
    ///
    /// # async fn ex() {
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
    /// # }
    /// ```
    pub async fn fake<F, Fut, T>(f: F) -> T
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = T>,
    {
        fake::install_fake_scope(f).await
    }

    /// Queue a canned response whose body is sent back verbatim as text,
    /// without JSON-encoding - the raw-body sibling of [`fake_response`]
    /// for upstream APIs that speak `text/plain` instead of JSON. `Password`'s
    /// `uncompromised()` check is the motivating case: HIBP's k-anonymity
    /// range endpoint answers `SUFFIX:COUNT` lines, not a JSON document, so
    /// [`fake_response`] (which always JSON-encodes its `body` argument)
    /// can't stand in for it in a test.
    ///
    /// Same method/URL-substring matching and consume-on-match semantics as
    /// [`fake_response`]: the first request whose method matches
    /// (case-insensitive, or `"*"` for any) and whose URL contains
    /// `url_substring` gets this response, and the canned entry is
    /// consumed. The response's `content-type` header is
    /// `text/plain; charset=utf-8`.
    ///
    /// **Must be called inside a `Http::fake(|| async { ... })` scope.**
    /// Panics if no fake scope is active on the current task.
    ///
    /// ```rust,no_run
    /// # use suprnova::Http;
    /// # async fn ex() {
    /// Http::fake(|| async {
    ///     Http::fake_response_text("GET", "/range/5BAA6", 200, "1E4C9...:3730471\r\n");
    ///     let resp = Http::get("https://api.pwnedpasswords.com/range/5BAA6")
    ///         .send()
    ///         .await
    ///         .unwrap();
    ///     assert_eq!(resp.text().await.unwrap(), "1E4C9...:3730471\r\n");
    /// })
    /// .await;
    /// # }
    /// ```
    pub fn fake_response_text(method: &str, url_substring: &str, status: u16, body: &str) {
        fake::fake_response_text(method, url_substring, status, body);
    }

    /// Enable test-guard mode: any outbound HTTP call that doesn't
    /// match an active fake returns
    /// `Err(FrameworkError::internal(...))` instead of hitting the
    /// real network.
    ///
    /// This is intentionally process-global so it catches the case it
    /// was built for - `tokio::spawn`-ed work escaping the caller's
    /// task-local [`Http::fake`] scope and silently calling real
    /// services. Pair with [`Self::allow_real_calls`] in teardown, or
    /// prefer [`FailOnRealCallsGuard`] which resets on drop:
    ///
    /// ```rust,no_run
    /// let _guard = suprnova::FailOnRealCallsGuard::install();
    /// // Inside this scope, any unfaked outbound call fails closed.
    /// ```
    pub fn fail_on_real_calls() {
        FAIL_ON_REAL_CALLS.fetch_add(1, Ordering::SeqCst);
    }

    /// Disable the test-guard mode. After this returns, unfaked
    /// outbound calls proceed to the real network as usual. Default
    /// state at process start is "real calls allowed". Resets the
    /// install count to zero, releasing every outstanding guard.
    pub fn allow_real_calls() {
        FAIL_ON_REAL_CALLS.store(0, Ordering::SeqCst);
    }

    /// `true` when [`Self::fail_on_real_calls`] is active.
    pub fn is_guarded() -> bool {
        FAIL_ON_REAL_CALLS.load(Ordering::SeqCst) > 0
    }

    /// Set the process-global cap on a buffered outbound response body
    /// (`ClientResponse::json`/`text`/`bytes`). Bounds memory pressure
    /// from a slow or malicious upstream streaming a very large body.
    /// Set once at boot; per-request overrides via
    /// [`RequestBuilder::max_response_bytes`].
    pub fn set_max_response_bytes(limit: usize) {
        MAX_RESPONSE_BODY_BYTES.store(limit, Ordering::SeqCst);
    }

    /// The effective process-global response-body cap - the value set by
    /// [`Self::set_max_response_bytes`], or
    /// `DEFAULT_MAX_RESPONSE_BODY_BYTES` (25 MiB) if unset.
    pub fn max_response_bytes() -> usize {
        match MAX_RESPONSE_BODY_BYTES.load(Ordering::SeqCst) {
            0 => DEFAULT_MAX_RESPONSE_BODY_BYTES,
            n => n,
        }
    }

    /// Spawn a task that inherits the calling task's fake state.
    ///
    /// `tokio::spawn` does NOT carry `tokio::task_local!` values into
    /// the spawned future, so a fake registered in the outer scope
    /// doesn't apply to the spawned task. This helper captures the
    /// current task's fake state (an `Arc<Mutex<FakeState>>`) and
    /// re-installs it in the child's task-local scope. Recorded
    /// requests from the child are visible to the parent through the
    /// same Arc - `assert_sent` after the child completes sees what
    /// the child sent.
    ///
    /// Most production code shouldn't need this - it's a test-time
    /// helper for code-under-test that itself spawns tasks (e.g. a
    /// queue worker that makes outbound HTTP from a spawned future).
    ///
    /// If no fake scope is active on the calling task, this is
    /// equivalent to `tokio::spawn(future)` - the child runs without
    /// any fake context and outbound calls take the normal real
    /// network path (or fail closed when
    /// [`Self::fail_on_real_calls`] is on).
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use suprnova::{Http, fake_response};
    /// # async fn ex() {
    /// Http::fake(|| async {
    ///     fake_response("GET", "/child", 204, serde_json::json!({}));
    ///     let handle = Http::spawn_with_fake_inheritance(async {
    ///         Http::get("https://child.test").send().await
    ///     });
    ///     let response = handle.await.unwrap().unwrap();
    ///     assert_eq!(response.status(), 204);
    /// })
    /// .await;
    /// # }
    /// ```
    pub fn spawn_with_fake_inheritance<F, T>(future: F) -> tokio::task::JoinHandle<T>
    where
        F: Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        match fake::snapshot_current_fake_state() {
            Some(state) => {
                tokio::spawn(async move { fake::install_inherited_scope(state, future).await })
            }
            None => tokio::spawn(future),
        }
    }
}

/// RAII guard for [`Http::fail_on_real_calls`]. `install()` flips the
/// flag on; the guard's `Drop` impl flips it back off, even if the
/// test panics. Use this in test setup to avoid poisoning sibling
/// tests when the body exits early:
///
/// ```rust,no_run
/// #[tokio::test]
/// async fn my_test() {
///     let _guard = suprnova::FailOnRealCallsGuard::install();
///     // Any unfaked outbound HTTP call in this scope errors out.
/// }
/// ```
///
/// `Drop` releases one install, so nested guards compose correctly:
/// dropping an inner guard returns the count to whatever the outer
/// scope holds, not unconditionally to "allowed".
///
/// The install count is process-global by design: this catches the
/// exact failure it was built for (work `tokio::spawn`-ed out of a
/// [`Http::fake`] scope hitting the real network). Parallel tasks
/// that each install their own guard compose safely - the flag stays
/// armed until the last outstanding guard drops. For parallel test
/// isolation, still prefer per-task fake scopes via [`Http::fake`] +
/// [`Http::spawn_with_fake_inheritance`] instead of relying on the
/// guard alone.
#[must_use = "FailOnRealCallsGuard releases the guard on drop - bind it to a name"]
pub struct FailOnRealCallsGuard;

impl FailOnRealCallsGuard {
    /// Arm [`Http::fail_on_real_calls`] and return a guard whose
    /// `Drop` impl releases one install, making nested and parallel
    /// installs safe.
    pub fn install() -> Self {
        Http::fail_on_real_calls();
        Self
    }
}

impl Drop for FailOnRealCallsGuard {
    fn drop(&mut self) {
        // The closure never returns `None`, so this cannot fail; the
        // `let _` only satisfies `unused_must_use`.
        let _ = FAIL_ON_REAL_CALLS.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
            Some(count.saturating_sub(1))
        });
    }
}

/// HTTP method, kept as a small internal enum so we don't leak
/// `reqwest::Method` through our API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
    Head,
}

impl Method {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
            Self::Head => "HEAD",
        }
    }

    fn into_reqwest(self) -> reqwest::Method {
        match self {
            Self::Get => reqwest::Method::GET,
            Self::Post => reqwest::Method::POST,
            Self::Put => reqwest::Method::PUT,
            Self::Patch => reqwest::Method::PATCH,
            Self::Delete => reqwest::Method::DELETE,
            Self::Head => reqwest::Method::HEAD,
        }
    }

    /// Whether this method is idempotent per RFC 7231 §4.2.2 - sending
    /// the request more than once has the same effect as sending it once.
    /// Retries are only safe (no duplicated side effect) for idempotent
    /// methods. GET/HEAD/PUT/DELETE are idempotent; POST and PATCH are not.
    pub(crate) fn is_idempotent(self) -> bool {
        matches!(self, Self::Get | Self::Head | Self::Put | Self::Delete)
    }
}

#[derive(Debug, Clone)]
pub(crate) enum Body {
    Json(serde_json::Value),
    Form(serde_json::Value),
    Raw(Bytes),
    /// The parts [`RequestBuilder::attach`] added, encoded as
    /// `multipart/form-data` when the request is prepared.
    Multipart(Vec<MultipartPart>),
}

/// One part of a `multipart/form-data` body.
#[derive(Debug, Clone)]
pub(crate) struct MultipartPart {
    name: String,
    contents: Bytes,
    filename: Option<String>,
}

/// What a request sends besides its method and URL, computed once: the
/// wire and the fake's record take it from the same place, so the fake
/// records the headers the request is sent with.
#[derive(Debug, Clone)]
pub(crate) struct Prepared {
    /// The request's headers, then the `Content-Type` its body sets and the
    /// user agent, each unless the request set it.
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Option<Bytes>,
}

/// The user agent every request of the facade sends.
const USER_AGENT: &str = concat!("suprnova/", env!("CARGO_PKG_VERSION"));

/// What is written as `%XX` in a URL parameter: everything but the
/// unreserved characters of RFC 3986, so a value cannot add a path
/// segment, a query, a fragment or a host, as RFC 6570's simple string
/// expansion, which Laravel's `withUrlParameters` uses, encodes it.
const URL_PARAMETER: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// A name or file name inside a `Content-Disposition` header, with the
/// three characters that would end or break the quoted string written as
/// `%XX`, as browsers write a form's field names.
fn disposition_value(value: &str) -> String {
    value
        .replace('"', "%22")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

impl Prepared {
    /// Prepare `builder`: encode its body and set the headers that body
    /// needs.
    fn of(builder: &RequestBuilder) -> Result<Self, FrameworkError> {
        let mut headers = builder.headers.clone();
        let has = |headers: &[(String, String)], name: &str| {
            headers
                .iter()
                .any(|(header, _)| header.eq_ignore_ascii_case(name))
        };
        let (content_type, body) = match &builder.body {
            Some(Body::Json(value)) => (
                Some("application/json".to_string()),
                Some(Bytes::from(value.to_string())),
            ),
            Some(Body::Form(value)) => {
                let encoded = serde_urlencoded::to_string(value).map_err(|e| {
                    FrameworkError::internal(format!("Http::form body serialization failed: {e}"))
                })?;
                (
                    Some("application/x-www-form-urlencoded".to_string()),
                    Some(Bytes::from(encoded)),
                )
            }
            Some(Body::Raw(bytes)) => (None, Some(bytes.clone())),
            Some(Body::Multipart(parts)) => {
                let boundary = format!("suprnova-{}", uuid::Uuid::new_v4().simple());
                let mut encoded = Vec::new();
                for part in parts {
                    encoded.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
                    encoded.extend_from_slice(
                        format!(
                            "Content-Disposition: form-data; name=\"{}\"",
                            disposition_value(&part.name)
                        )
                        .as_bytes(),
                    );
                    if let Some(filename) = &part.filename {
                        encoded.extend_from_slice(
                            format!("; filename=\"{}\"", disposition_value(filename)).as_bytes(),
                        );
                    }
                    encoded.extend_from_slice(b"\r\n\r\n");
                    encoded.extend_from_slice(&part.contents);
                    encoded.extend_from_slice(b"\r\n");
                }
                encoded.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
                (
                    Some(format!("multipart/form-data; boundary={boundary}")),
                    Some(Bytes::from(encoded)),
                )
            }
            None => (None, None),
        };
        if let Some(content_type) = content_type
            && !has(&headers, "content-type")
        {
            headers.push(("content-type".to_string(), content_type));
        }
        if !has(&headers, "user-agent") {
            headers.push(("user-agent".to_string(), USER_AGENT.to_string()));
        }
        Ok(Self { headers, body })
    }
}

/// Retry policy attached to a [`RequestBuilder`].
///
/// Created with [`RequestBuilder::retry`] (idempotent methods only) or
/// [`RequestBuilder::retry_non_idempotent`] (all methods). Retries on
/// transient failures (connect/timeout, HTTP 5xx). The delay before
/// attempt `n+1` is a random duration in `[0, base_backoff * 2^(n-1)]`
/// (full jitter), capped at 30s. For HTTP 503 the wait is the larger of
/// that backoff and any `Retry-After` header, still capped at 30s.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RetryPolicy {
    pub(crate) max_attempts: u32,
    pub(crate) base_backoff: Duration,
    /// When `false` (the default, set by [`RequestBuilder::retry`]),
    /// retries are limited to idempotent methods. When `true` (set by
    /// [`RequestBuilder::retry_non_idempotent`]), POST/PATCH are retried
    /// too - only safe when the upstream is protected by an idempotency
    /// key or is otherwise safe to call more than once.
    pub(crate) retry_non_idempotent: bool,
}

/// What failed on the attempt a [`RequestBuilder::retry_when`] predicate
/// is being asked about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryOutcome {
    /// The attempt never got a response - a connect, DNS, or timeout
    /// failure before any bytes came back.
    TransportError,
    /// The attempt got a response with this status code - always
    /// `500..600`, the only status the built-in policy ever retries.
    Status(u16),
}

/// Passed to the predicate registered via [`RequestBuilder::retry_when`]
/// on every attempt the built-in [retry policy](RequestBuilder::retry)
/// has already decided is retry-eligible.
#[derive(Debug, Clone)]
pub struct RetryContext {
    /// The attempt that just produced `outcome`, 1-based. The predicate
    /// is consulted before attempt `attempt + 1` would be made.
    pub attempt: u32,
    /// The request's HTTP method: `"GET"`, `"POST"`, etc.
    pub method: &'static str,
    /// The request URL, exactly as passed to `Http::get`/`post`/etc.
    pub url: String,
    /// What failed on this attempt.
    pub outcome: RetryOutcome,
}

/// Boxed [`RequestBuilder::retry_when`] predicate shape - extracted into a
/// type alias so the field below reads clean and clippy's
/// `type_complexity` lint is satisfied.
pub(crate) type RetryPredicate = Box<dyn Fn(&RetryContext) -> bool + Send + Sync>;

/// Builder for an outbound HTTP request. Created via the [`Http`] facade.
pub struct RequestBuilder {
    pub(crate) method: Method,
    pub(crate) url: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Option<Body>,
    /// Set when [`RequestBuilder::json`]/[`RequestBuilder::form`] fail to
    /// serialize the value. [`RequestBuilder::send`] surfaces it as an
    /// error instead of sending a body that silently degraded to `null`.
    pub(crate) body_error: Option<String>,
    pub(crate) timeout: Option<Duration>,
    pub(crate) retry: Option<RetryPolicy>,
    /// Optional predicate consulted before every retry the built-in
    /// policy (`retry`) would otherwise make. Set via
    /// [`RequestBuilder::retry_when`]. Returning `false` vetoes that one
    /// retry; the predicate can never expand the policy - it isn't
    /// consulted for an attempt the policy wasn't already going to
    /// retry, and it can't push the attempt count past `max_attempts`.
    pub(crate) retry_when: Option<RetryPredicate>,
    /// Per-request response-body cap; falls back to the process-global
    /// default ([`Http::max_response_bytes`]) when `None`.
    pub(crate) max_response_bytes: Option<usize>,
    /// When `true` ([`RequestBuilder::no_redirects`]), this request routes
    /// through the non-following client so a 3xx is returned as-is rather
    /// than followed - an SSRF guard for user-influenced URLs.
    pub(crate) no_redirects: bool,
    /// Put in front of a URL without a scheme when the request is sent.
    pub(crate) base_url: Option<String>,
    /// Merged into the URL's query when the request is sent.
    pub(crate) query: Vec<(String, String)>,
    /// Expanded into the URL's `{name}` placeholders when the request is
    /// sent.
    pub(crate) url_parameters: Vec<(String, String)>,
    /// This request's own connect timeout; `None` keeps the shared
    /// client's 10 seconds.
    pub(crate) connect_timeout: Option<Duration>,
    /// The global request middleware the request took when it was created.
    pub(crate) request_middleware: Vec<Arc<RequestMiddleware>>,
    /// The global response middleware the request took when it was
    /// created.
    pub(crate) response_middleware: Vec<Arc<ResponseMiddleware>>,
}

impl RequestBuilder {
    /// A request taking the global configuration in force now, as
    /// Laravel's factory hands it to each new pending request: the global
    /// options run on it at once, so what the caller sets afterwards comes
    /// after them, and the global middleware is kept for `send`.
    pub(crate) fn new(method: Method, url: String) -> Self {
        let global = current_global_configuration();
        let builder = Self {
            method,
            url,
            headers: Vec::new(),
            body: None,
            body_error: None,
            timeout: None,
            retry: None,
            retry_when: None,
            max_response_bytes: None,
            no_redirects: false,
            base_url: None,
            query: Vec::new(),
            url_parameters: Vec::new(),
            connect_timeout: None,
            request_middleware: global.request,
            response_middleware: global.response,
        };
        match global.options {
            Some(options) => options(builder),
            None => builder,
        }
    }

    /// The HTTP method: `"GET"`, `"POST"`, and so on. Lets a middleware
    /// decide by the method.
    pub fn method(&self) -> &'static str {
        self.method.as_str()
    }

    /// The URL. Inside a global request middleware it is the URL the
    /// request is sent to, its base URL, URL parameters and query applied;
    /// before `send` it is the URL as given.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Put `url` in front of the request's URL when that URL does not
    /// start with `http://` or `https://` (Laravel's `baseUrl`), with one
    /// slash between them: `Http::get("users").base_url("https://api.test/v1")`
    /// sends to `https://api.test/v1/users`. An absolute URL is sent as it
    /// is. Calling it again replaces the base URL.
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = Some(url.into());
        self
    }

    /// Merge `params` into the URL's query (Laravel's
    /// `withQueryParameters`). A name the URL already has takes the value
    /// given here; the other names of the URL stay. The values are encoded
    /// as a form encodes them. Calling it again adds more.
    ///
    /// The URL must be absolute once its base URL is applied, or `send`
    /// fails.
    pub fn query<K, V>(mut self, params: &[(K, V)]) -> Self
    where
        K: AsRef<str>,
        V: AsRef<str>,
    {
        for (name, value) in params {
            self.query
                .push((name.as_ref().to_string(), value.as_ref().to_string()));
        }
        self
    }

    /// Expand the `{name}` placeholders of the URL with `params` (Laravel's
    /// `withUrlParameters`): `https://a.test/users/{id}` with `id` set to
    /// `a/b` sends to `https://a.test/users/a%2Fb`.
    ///
    /// Every character of a value but the letters, the digits and `-`, `.`,
    /// `_`, `~` is percent-encoded, so a value cannot add a path segment, a
    /// query, a fragment or a host. A placeholder no value names is left as
    /// it is. Only the simple `{name}` form is expanded. Calling it again
    /// adds more.
    pub fn url_parameters<P, K, V>(mut self, params: impl IntoIterator<Item = P>) -> Self
    where
        P: Borrow<(K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        for pair in params {
            let (name, value) = pair.borrow();
            self.url_parameters
                .push((name.as_ref().to_string(), value.as_ref().to_string()));
        }
        self
    }

    /// Add a file part to a `multipart/form-data` body (Laravel's
    /// `attach`): `contents` under the field `name`, with `filename` when
    /// the part is a file. Calling it again adds another part. It replaces
    /// a JSON, form or raw body set before it, and [`Self::json`],
    /// [`Self::form`] and [`Self::body`] replace the parts.
    ///
    /// The body is encoded when the request is sent, with a random
    /// boundary in its `Content-Type`, and the fake records it encoded.
    pub fn attach(
        mut self,
        name: impl Into<String>,
        contents: impl AsRef<[u8]>,
        filename: Option<&str>,
    ) -> Self {
        let part = MultipartPart {
            name: name.into(),
            contents: Bytes::copy_from_slice(contents.as_ref()),
            filename: filename.map(str::to_string),
        };
        match &mut self.body {
            Some(Body::Multipart(parts)) => parts.push(part),
            _ => self.body = Some(Body::Multipart(vec![part])),
        }
        self
    }

    /// How long this request may take to connect (Laravel's
    /// `connectTimeout`). The shared client allows 10 seconds.
    ///
    /// reqwest fixes the connect timeout when it builds a client, so each
    /// distinct timeout gets a client of its own, built on first use and
    /// kept; use a few fixed values rather than one computed per request.
    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = Some(timeout);
        self
    }

    /// Apply the base URL, the URL parameters and the query to the URL,
    /// and clear them, so applying again changes nothing.
    fn resolve_url(&mut self) -> Result<(), FrameworkError> {
        if let Some(base) = self.base_url.take() {
            let absolute = ["http://", "https://"].iter().any(|scheme| {
                self.url
                    .get(..scheme.len())
                    .is_some_and(|start| start.eq_ignore_ascii_case(scheme))
            });
            if !absolute {
                self.url = format!(
                    "{}/{}",
                    base.trim_end_matches('/'),
                    self.url.trim_start_matches('/')
                );
            }
        }
        for (name, value) in std::mem::take(&mut self.url_parameters) {
            let placeholder = format!("{{{name}}}");
            if self.url.contains(&placeholder) {
                let encoded = utf8_percent_encode(&value, URL_PARAMETER).to_string();
                self.url = self.url.replace(&placeholder, &encoded);
            }
        }
        let query = std::mem::take(&mut self.query);
        if !query.is_empty() {
            let mut url = url::Url::parse(&self.url).map_err(|e| {
                FrameworkError::internal(format!(
                    "Http: query parameters need an absolute URL, and `{}` is none ({e}); \
                     give the request a base_url",
                    self.url
                ))
            })?;
            let kept: Vec<(String, String)> = url
                .query_pairs()
                .into_owned()
                .filter(|(name, _)| !query.iter().any(|(given, _)| given == name))
                .collect();
            url.query_pairs_mut()
                .clear()
                .extend_pairs(kept)
                .extend_pairs(query);
            self.url = url.into();
        }
        Ok(())
    }

    /// Do not follow HTTP redirects for this request: a 3xx response is
    /// returned to the caller as-is instead of being followed.
    ///
    /// The default client follows redirects (up to reqwest's cap of 10),
    /// which is the right behavior for a general-purpose client calling
    /// trusted endpoints. Reach for this when the request URL is derived
    /// from untrusted input - it closes a redirect-based SSRF vector where
    /// a hostile endpoint answers with a 3xx pointing at an internal or
    /// cloud-metadata address. With redirects disabled, that 3xx surfaces
    /// as a normal response your code can inspect and reject.
    pub fn no_redirects(mut self) -> Self {
        self.no_redirects = true;
        self
    }

    /// Append a header. Repeats are kept; reqwest will join them per
    /// HTTP semantics.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Send the body as JSON. Replaces any previously-set body. Sets
    /// `Content-Type: application/json` automatically on the wire.
    pub fn json<T: Serialize>(mut self, value: &T) -> Self {
        match serde_json::to_value(value) {
            Ok(v) => self.body = Some(Body::Json(v)),
            // Record the failure rather than silently sending `null`;
            // `send` turns this into an error before any request goes out.
            Err(e) => self.body_error = Some(format!("Http::json body serialization failed: {e}")),
        }
        self
    }

    /// Send the body as `application/x-www-form-urlencoded`. The value
    /// must serialize to a JSON object - keys become form fields.
    pub fn form<T: Serialize>(mut self, value: &T) -> Self {
        match serde_json::to_value(value) {
            Ok(v) => self.body = Some(Body::Form(v)),
            Err(e) => self.body_error = Some(format!("Http::form body serialization failed: {e}")),
        }
        self
    }

    /// Send a raw byte body. The caller is responsible for setting
    /// `Content-Type`.
    pub fn body(mut self, bytes: impl Into<Bytes>) -> Self {
        self.body = Some(Body::Raw(bytes.into()));
        self
    }

    /// Override the request timeout. Defaults to 30 seconds from the
    /// shared client.
    pub fn timeout(mut self, dur: Duration) -> Self {
        self.timeout = Some(dur);
        self
    }

    /// Cap the response body this request will buffer (via
    /// `ClientResponse::json`/`text`/`bytes`), overriding the
    /// process-global [`Http::max_response_bytes`] for this one request.
    pub fn max_response_bytes(mut self, limit: usize) -> Self {
        self.max_response_bytes = Some(limit);
        self
    }

    /// Attach a Bearer token via the `Authorization` header.
    pub fn bearer_token(self, token: impl AsRef<str>) -> Self {
        self.header("Authorization", format!("Bearer {}", token.as_ref()))
    }

    /// Attach HTTP Basic auth. `password` is optional - `None` produces
    /// `user:`.
    pub fn basic_auth(self, user: impl AsRef<str>, password: Option<&str>) -> Self {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        let credential = format!("{}:{}", user.as_ref(), password.unwrap_or(""));
        let encoded = STANDARD.encode(credential);
        self.header("Authorization", format!("Basic {}", encoded))
    }

    /// Configure transient-failure retries for IDEMPOTENT methods.
    ///
    /// `max_attempts` is the total number of attempts including the
    /// first try (so `max_attempts=4` retries up to three times).
    /// `base_backoff` seeds the delay; the wait before attempt `n+1` is a
    /// random duration in `[0, base_backoff * 2^(n-1)]` (full jitter, so
    /// many workers retrying the same outage don't synchronize into a
    /// thundering herd), capped at 30s.
    ///
    /// A request is eligible for retry only if its method is idempotent
    /// (GET/PUT/DELETE) - see [`Self::retry_non_idempotent`] to opt POST/
    /// PATCH in. An eligible request is retried when:
    /// - The send fails before we have a response (connect / timeout /
    ///   DNS errors), or
    /// - The response status is 5xx.
    ///
    /// 4xx and 2xx/3xx are returned as-is. After exhausting retries the
    /// last response (or the last error) is returned. For 503 the wait is
    /// the larger of the jittered backoff and the `Retry-After` header
    /// (delta-seconds or HTTP-date), still capped at 30s.
    ///
    /// Calling `.retry()` again replaces the previous policy.
    pub fn retry(mut self, max_attempts: u32, base_backoff: Duration) -> Self {
        let attempts = max_attempts.max(1);
        self.retry = Some(RetryPolicy {
            max_attempts: attempts,
            base_backoff,
            retry_non_idempotent: false,
        });
        self
    }

    /// Like [`Self::retry`], but ALSO retries non-idempotent methods
    /// (`POST`, `PATCH`).
    ///
    /// [`Self::retry`] deliberately skips POST/PATCH: if the upstream
    /// already performed the write but the response was lost (or it
    /// returned 5xx *after* committing), a blind retry duplicates the
    /// side effect. Only reach for this when the request is safe to send
    /// more than once - e.g. it carries an idempotency key the server
    /// honors, or the operation is naturally safe to repeat. Idempotent
    /// methods (GET/PUT/DELETE) are retried by both this and
    /// [`Self::retry`]; calling either again replaces the previous policy.
    pub fn retry_non_idempotent(mut self, max_attempts: u32, base_backoff: Duration) -> Self {
        self.retry = Some(RetryPolicy {
            max_attempts: max_attempts.max(1),
            base_backoff,
            retry_non_idempotent: true,
        });
        self
    }

    /// Register a predicate consulted before every retry the built-in
    /// policy ([`Self::retry`] / [`Self::retry_non_idempotent`]) would
    /// otherwise make. It only narrows that policy: `false` vetoes an
    /// already-eligible retry; it's never consulted for an attempt the
    /// policy wouldn't retry anyway (a 4xx status, or a non-idempotent
    /// method without [`Self::retry_non_idempotent`]), and it can never
    /// push the attempt count past `max_attempts`.
    ///
    /// Without a policy from [`Self::retry`] or
    /// [`Self::retry_non_idempotent`], no attempt is ever retry-eligible,
    /// so a predicate registered alone has no effect. Calling this again
    /// replaces the previous predicate.
    ///
    /// ```rust,no_run
    /// # use suprnova::Http;
    /// # use std::time::Duration;
    /// # async fn ex() {
    /// let response = Http::get("https://example.com/users")
    ///     .retry(3, Duration::from_millis(100))
    ///     .retry_when(|ctx| ctx.method != "GET")
    ///     .send()
    ///     .await;
    /// # let _ = response;
    /// # }
    /// ```
    pub fn retry_when(
        mut self,
        predicate: impl Fn(&RetryContext) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.retry_when = Some(Box::new(predicate));
        self
    }

    /// Execute the request. When [`Http::fake`] is active, returns the
    /// matched canned response instead of hitting the network. If a
    /// retry policy is configured via [`Self::retry`], transient
    /// failures and 5xx responses are retried with exponential
    /// backoff (see [`Self::retry`] for the rules).
    pub async fn send(mut self) -> Result<ClientResponse, FrameworkError> {
        // Surface a json()/form() serialization failure recorded on the
        // builder instead of sending a body that silently degraded to null.
        if let Some(err) = &self.body_error {
            return Err(FrameworkError::internal(err.clone()));
        }
        // The global request middleware sees the URL the request goes to,
        // and may change the request; whatever URL parts it adds are
        // applied after it.
        self.resolve_url()?;
        for middleware in std::mem::take(&mut self.request_middleware) {
            self = middleware(self);
        }
        if let Some(err) = &self.body_error {
            return Err(FrameworkError::internal(err.clone()));
        }
        self.resolve_url()?;
        let prepared = Prepared::of(&self)?;
        let response_middleware = std::mem::take(&mut self.response_middleware);

        // Cap applied to whatever body the returned response buffers.
        let effective_max = self
            .max_response_bytes
            .unwrap_or_else(Http::max_response_bytes);

        let max_attempts = self.retry.map(|p| p.max_attempts).unwrap_or(1);

        let mut last_err: Option<FrameworkError> = None;
        for attempt in 1..=max_attempts {
            let outcome = if fake::is_fake_active() {
                match fake::intercept(&self, &prepared) {
                    fake::Interception::Answered(answer) => answer,
                    fake::Interception::Network => build_and_send(&self, &prepared).await,
                }
            } else if Http::is_guarded() {
                // Process-global fail-closed mode: outbound calls
                // that don't match an active fake error out instead
                // of hitting the real network. Mirrors Laravel's
                // `Http::preventStrayRequests()`. The error is
                // `FrameworkError::internal` - the request URL is
                // included so the user can identify where the
                // unmatched call originated, but no headers/body
                // detail leaks.
                Err::<ClientResponse, FrameworkError>(FrameworkError::internal(format!(
                    "Http::fail_on_real_calls is active and no fake matched outbound \
                     request to {}. Register a matching fake via fake_response(...), \
                     or release the guard via FailOnRealCallsGuard / \
                     Http::allow_real_calls() to allow real network access.",
                    self.url,
                )))
            } else {
                build_and_send(&self, &prepared).await
            };
            let outcome = outcome.map(|response| {
                response_middleware
                    .iter()
                    .fold(response, |response, middleware| middleware(response))
            });

            match outcome {
                Ok(resp) => {
                    let status = resp.status();
                    if (500..600).contains(&status)
                        && let Some(backoff) =
                            self.backoff_before_retry(attempt, RetryOutcome::Status(status))
                    {
                        // A 503 can say how long to wait. It is obeyed
                        // when it asks for more than the backoff, up to
                        // the cap.
                        let wait = if status == 503 {
                            std::cmp::min(
                                std::cmp::max(backoff, retry_after_from(&resp)),
                                MAX_RETRY_WAIT,
                            )
                        } else {
                            backoff
                        };
                        tokio::time::sleep(wait).await;
                        continue;
                    }
                    return Ok(resp.with_max_bytes(effective_max));
                }
                Err(e) => {
                    if let Some(backoff) =
                        self.backoff_before_retry(attempt, RetryOutcome::TransportError)
                    {
                        last_err = Some(e);
                        tokio::time::sleep(backoff).await;
                        continue;
                    }
                    return Err(e);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| {
            FrameworkError::internal("Http::send retries exhausted without a response")
        }))
    }

    /// Whether the attempt that ended in `outcome` is followed by another
    /// one, and how long to wait before it. `None` ends the request with
    /// what the attempt gave.
    ///
    /// This is the one place that decides. A response with a status of
    /// the server and an attempt that got no response are retried by the
    /// same rule:
    ///
    /// - a retry policy is set, and this was not its last attempt;
    /// - the method is idempotent, or the caller asked for
    ///   `retry_non_idempotent`: a POST or a PATCH whose first attempt
    ///   may have taken effect is not sent again unasked;
    /// - the predicate of `retry_when`, when there is one, allows it.
    fn backoff_before_retry(&self, attempt: u32, outcome: RetryOutcome) -> Option<Duration> {
        let policy = self.retry?;
        let method_retryable = policy.retry_non_idempotent || self.method.is_idempotent();
        if !method_retryable || attempt >= policy.max_attempts {
            return None;
        }
        let allowed = self.retry_when.as_ref().is_none_or(|predicate| {
            predicate(&RetryContext {
                attempt,
                method: self.method.as_str(),
                url: self.url.clone(),
                outcome,
            })
        });
        allowed.then(|| backoff_for(attempt, policy.base_backoff))
    }
}

/// Single attempt at the request. No retry logic, no fake interception.
/// The headers and the body come from `prepared`, the computation the
/// fake records from too.
async fn build_and_send(
    builder: &RequestBuilder,
    prepared: &Prepared,
) -> Result<ClientResponse, FrameworkError> {
    // User-influenced URLs can opt out of redirect-following to close the
    // redirect-based SSRF vector; everything else uses the default
    // redirect-following client.
    let http = client_for(builder.no_redirects, builder.connect_timeout)?;
    let mut req = http.request(builder.method.into_reqwest(), &builder.url);

    for (k, v) in &prepared.headers {
        req = req.header(k.as_str(), v.as_str());
    }
    if let Some(t) = builder.timeout {
        req = req.timeout(t);
    }
    if let Some(body) = &prepared.body {
        req = req.body(body.clone());
    }

    // Build the request so we can mutate its header map to inject
    // W3C trace context (otel feature only - no-op otherwise).
    let mut request = req
        .build()
        .map_err(|e| FrameworkError::internal(format!("Http::send failed: {e}")))?;

    inject_w3c_trace_context(&mut request);

    let resp = http
        .execute(request)
        .await
        .map_err(|e| FrameworkError::internal(format!("Http::send failed: {e}")))?;
    Ok(ClientResponse::real(resp))
}

/// Inject the current OpenTelemetry context into outbound request
/// headers using the globally-registered text-map propagator
/// (`TraceContextPropagator` is installed by `init_telemetry` when the
/// `otel` feature is enabled). Produces `traceparent` / `tracestate`
/// headers that downstream services parse to continue the trace.
///
/// If no OTel context is active (i.e. `Context::current()` is empty),
/// the propagator emits nothing and headers are left untouched. This
/// keeps the code path safe to run unconditionally on every request.
#[cfg(feature = "otel")]
fn inject_w3c_trace_context(request: &mut reqwest::Request) {
    use crate::telemetry::propagation::HeaderInjector;
    use opentelemetry::global;

    let cx = opentelemetry::Context::current();
    let mut injector = HeaderInjector(request.headers_mut());
    global::get_text_map_propagator(|propagator| propagator.inject_context(&cx, &mut injector));
}

/// No-op stub when the `otel` feature is disabled - header injection
/// has nothing to do because no propagator is installed.
#[cfg(not(feature = "otel"))]
fn inject_w3c_trace_context(_request: &mut reqwest::Request) {}

/// Maximum wait between two retry attempts. Bounds both the exponential
/// backoff and a hostile `Retry-After` (e.g. `Retry-After: 86400`) so a
/// single retry can never park a task for more than 30 seconds.
const MAX_RETRY_WAIT: Duration = Duration::from_secs(30);

/// Exponential backoff with full jitter. The ceiling is
/// `base_backoff * 2^(attempt-1)` (saturating, capped at
/// [`MAX_RETRY_WAIT`]); the returned wait is a uniform random duration in
/// `[0, ceiling]`. Full jitter (AWS's published recipe) keeps many
/// workers retrying the same outage from synchronizing into a thundering
/// herd.
fn backoff_for(attempt: u32, base_backoff: Duration) -> Duration {
    use rand::RngExt;

    // `Duration::saturating_mul` takes u32; cap the exponent at 31 so
    // `1u32 << exp` is well-defined.
    let exp = attempt.saturating_sub(1).min(31);
    let factor: u32 = 1u32 << exp;
    let ceiling = base_backoff.saturating_mul(factor).min(MAX_RETRY_WAIT);
    let ceiling_ms = ceiling.as_millis() as u64;
    if ceiling_ms == 0 {
        return Duration::ZERO;
    }
    // Uniform in `[0, ceiling_ms]`; millisecond precision is plenty for
    // backoff scheduling.
    let jittered = rand::rng().random_range(0..=ceiling_ms);
    Duration::from_millis(jittered)
}

/// Parse a `Retry-After` header in either RFC 7231 form: integer
/// delta-seconds, or an HTTP-date. For an HTTP-date the wait is the time
/// from now until that instant (a date already in the past yields
/// `Duration::ZERO`). Returns `Duration::ZERO` if the header is missing
/// or unparseable.
fn retry_after_from(resp: &ClientResponse) -> Duration {
    let Some(raw) = resp.header("Retry-After") else {
        return Duration::ZERO;
    };
    let raw = raw.trim();
    // Delta-seconds form (the common case).
    if let Ok(secs) = raw.parse::<u64>() {
        return Duration::from_secs(secs);
    }
    // HTTP-date form: wait until that instant, clamped at zero if it is
    // already in the past.
    match httpdate::parse_http_date(raw) {
        Ok(when) => when
            .duration_since(std::time::SystemTime::now())
            .unwrap_or(Duration::ZERO),
        Err(_) => Duration::ZERO,
    }
}

/// Outbound response. Wraps `reqwest::Response` (real) or in-memory
/// bytes (fake). `max_bytes` caps how much body `json`/`text`/`bytes`
/// will buffer.
pub struct ClientResponse {
    inner: ClientResponseInner,
    max_bytes: usize,
}

enum ClientResponseInner {
    Real(reqwest::Response),
    Fake {
        status: u16,
        headers: Vec<(String, String)>,
        body: Bytes,
    },
}

impl ClientResponse {
    pub(crate) fn real(resp: reqwest::Response) -> Self {
        Self {
            inner: ClientResponseInner::Real(resp),
            max_bytes: DEFAULT_MAX_RESPONSE_BODY_BYTES,
        }
    }

    pub(crate) fn fake(status: u16, headers: Vec<(String, String)>, body: Bytes) -> Self {
        Self {
            inner: ClientResponseInner::Fake {
                status,
                headers,
                body,
            },
            max_bytes: DEFAULT_MAX_RESPONSE_BODY_BYTES,
        }
    }

    /// Set the response-body cap. Called by [`RequestBuilder::send`] with
    /// the request's effective limit.
    pub(crate) fn with_max_bytes(mut self, max: usize) -> Self {
        self.max_bytes = max;
        self
    }

    /// Response status code.
    pub fn status(&self) -> u16 {
        match &self.inner {
            ClientResponseInner::Real(r) => r.status().as_u16(),
            ClientResponseInner::Fake { status, .. } => *status,
        }
    }

    /// Look up a response header by name. Case-insensitive.
    pub fn header(&self, name: &str) -> Option<String> {
        match &self.inner {
            ClientResponseInner::Real(r) => r
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok().map(|s| s.to_string())),
            ClientResponseInner::Fake { headers, .. } => headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.clone()),
        }
    }

    /// Read the full body and parse as JSON, enforcing the response-body
    /// cap (see [`Http::set_max_response_bytes`] /
    /// [`RequestBuilder::max_response_bytes`]).
    pub async fn json<T: serde::de::DeserializeOwned>(self) -> Result<T, FrameworkError> {
        let max = self.max_bytes;
        let bytes = match self.inner {
            ClientResponseInner::Real(r) => read_capped(r, max).await?,
            ClientResponseInner::Fake { body, .. } => check_fake_within_cap(body, max)?,
        };
        serde_json::from_slice(&bytes)
            .map_err(|e| FrameworkError::internal(format!("Http json decode failed: {e}")))
    }

    /// Read the full body as UTF-8 text, enforcing the response-body cap.
    pub async fn text(self) -> Result<String, FrameworkError> {
        let max = self.max_bytes;
        let bytes = match self.inner {
            ClientResponseInner::Real(r) => read_capped(r, max).await?,
            ClientResponseInner::Fake { body, .. } => check_fake_within_cap(body, max)?,
        };
        String::from_utf8(bytes.to_vec())
            .map_err(|e| FrameworkError::internal(format!("Http body not UTF-8: {e}")))
    }

    /// Read the full body as bytes, enforcing the response-body cap.
    pub async fn bytes(self) -> Result<Bytes, FrameworkError> {
        let max = self.max_bytes;
        match self.inner {
            ClientResponseInner::Real(r) => read_capped(r, max).await,
            ClientResponseInner::Fake { body, .. } => check_fake_within_cap(body, max),
        }
    }

    /// Unwrap to the underlying `reqwest::Response`. This is an
    /// escape hatch for callers that need to reach for a `reqwest`
    /// API we don't expose (streaming bodies, redirect policy
    /// inspection, etc.). The response-body cap does NOT apply once you
    /// take the raw response - you own the read from there.
    ///
    /// Returns `Err(FrameworkError::internal(...))` if the response
    /// was produced by [`Http::fake`] - there is no underlying
    /// `reqwest::Response` in that case. Real responses are returned
    /// via `Ok`.
    pub fn into_inner(self) -> Result<reqwest::Response, FrameworkError> {
        match self.inner {
            ClientResponseInner::Real(r) => Ok(r),
            ClientResponseInner::Fake { .. } => Err(FrameworkError::internal(
                "into_inner is not available on fake responses",
            )),
        }
    }
}

/// Buffer a reqwest response body, rejecting it once it exceeds `max`
/// bytes. A declared `Content-Length` over the cap is rejected before any
/// body is read; the streaming loop then enforces the cap against the
/// actual bytes (Content-Length can be absent or lie).
async fn read_capped(resp: reqwest::Response, max: usize) -> Result<Bytes, FrameworkError> {
    if let Some(len) = resp.content_length()
        && len > max as u64
    {
        return Err(FrameworkError::internal(format!(
            "Http response body exceeds the {max}-byte cap (Content-Length {len})"
        )));
    }
    let mut resp = resp;
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| FrameworkError::internal(format!("Http body read failed: {e}")))?
    {
        if buf.len() + chunk.len() > max {
            return Err(FrameworkError::internal(format!(
                "Http response body exceeds the {max}-byte cap"
            )));
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(Bytes::from(buf))
}

/// Enforce the response-body cap on an in-memory fake body.
fn check_fake_within_cap(body: Bytes, max: usize) -> Result<Bytes, FrameworkError> {
    if body.len() > max {
        return Err(FrameworkError::internal(format!(
            "Http response body exceeds the {max}-byte cap"
        )));
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The contexts the predicate of a request was asked about.
    fn asked(
        request: RequestBuilder,
        allow: bool,
    ) -> (
        RequestBuilder,
        std::sync::Arc<std::sync::Mutex<Vec<RetryContext>>>,
    ) {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let record = std::sync::Arc::clone(&seen);
        let request = request.retry_when(move |context| {
            record
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(context.clone());
            allow
        });
        (request, seen)
    }

    const BOTH_OUTCOMES: [RetryOutcome; 2] =
        [RetryOutcome::Status(502), RetryOutcome::TransportError];

    #[test]
    fn a_request_without_a_policy_is_not_retried() {
        let request = Http::get("https://api.example.test/things");
        for outcome in BOTH_OUTCOMES {
            assert_eq!(request.backoff_before_retry(1, outcome), None);
        }
    }

    #[test]
    fn the_two_outcomes_are_retried_by_one_rule() {
        let base = Duration::from_millis(40);
        let request = Http::get("https://api.example.test/things").retry(3, base);
        for outcome in BOTH_OUTCOMES {
            for attempt in [1, 2] {
                let backoff = request
                    .backoff_before_retry(attempt, outcome)
                    .unwrap_or_else(|| panic!("attempt {attempt} of 3 is followed by another"));
                assert!(
                    backoff <= base.saturating_mul(1 << (attempt - 1)),
                    "the backoff of attempt {attempt} is over its ceiling: {backoff:?}"
                );
            }
            assert_eq!(
                request.backoff_before_retry(3, outcome),
                None,
                "the last attempt is followed by none"
            );
        }
    }

    #[test]
    fn a_post_is_retried_when_the_caller_asked_for_it() {
        let base = Duration::from_millis(10);
        let unasked = Http::post("https://api.example.test/things").retry(3, base);
        let asked_for = Http::post("https://api.example.test/things").retry_non_idempotent(3, base);
        for outcome in BOTH_OUTCOMES {
            assert_eq!(unasked.backoff_before_retry(1, outcome), None);
            assert!(asked_for.backoff_before_retry(1, outcome).is_some());
        }
    }

    #[test]
    fn the_predicate_is_asked_with_the_outcome_and_can_refuse() {
        let base = Duration::from_millis(10);
        let (refusing, seen) = asked(
            Http::get("https://api.example.test/things").retry(3, base),
            false,
        );
        for outcome in BOTH_OUTCOMES {
            assert_eq!(refusing.backoff_before_retry(2, outcome), None);
        }
        let seen = seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        assert_eq!(
            seen.iter()
                .map(|context| context.outcome)
                .collect::<Vec<_>>(),
            BOTH_OUTCOMES
        );
        assert!(seen.iter().all(|context| context.attempt == 2
            && context.method == "GET"
            && context.url == "https://api.example.test/things"));

        let (allowing, seen) = asked(
            Http::get("https://api.example.test/things").retry(3, base),
            true,
        );
        assert!(
            allowing
                .backoff_before_retry(1, RetryOutcome::TransportError)
                .is_some()
        );
        // The rule of the policy comes first: the predicate is not asked
        // about an attempt that the policy ends.
        assert_eq!(
            allowing.backoff_before_retry(3, RetryOutcome::TransportError),
            None
        );
        assert_eq!(
            seen.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len(),
            1
        );
    }

    #[test]
    fn idempotent_methods_are_get_head_put_delete() {
        assert!(Method::Head.is_idempotent());
        assert!(Method::Get.is_idempotent());
        assert!(Method::Put.is_idempotent());
        assert!(Method::Delete.is_idempotent());
        assert!(!Method::Post.is_idempotent());
        assert!(!Method::Patch.is_idempotent());
    }

    #[test]
    fn backoff_stays_within_ceiling_and_is_capped() {
        // Full jitter: every sample sits in [0, ceiling]. With a 100ms
        // base, attempt 3's ceiling is 100ms * 2^2 = 400ms.
        let base = Duration::from_millis(100);
        for _ in 0..256 {
            assert!(
                backoff_for(3, base) <= Duration::from_millis(400),
                "jittered backoff exceeded its ceiling"
            );
        }
        // A pathologically large attempt / base is bounded by the 30s cap
        // rather than overflowing or parking for longer.
        for _ in 0..256 {
            assert!(
                backoff_for(40, Duration::from_secs(10)) <= MAX_RETRY_WAIT,
                "backoff exceeded the 30s cap"
            );
        }
    }

    #[test]
    fn retry_after_parses_delta_seconds_and_http_date() {
        let with_header = |value: String| {
            ClientResponse::fake(503, vec![("Retry-After".to_string(), value)], Bytes::new())
        };

        // Delta-seconds form.
        assert_eq!(
            retry_after_from(&with_header("5".to_string())),
            Duration::from_secs(5)
        );

        // HTTP-date ~3s in the future parses to roughly 3s (HTTP-date has
        // whole-second granularity, so allow generous slack).
        let future = std::time::SystemTime::now() + Duration::from_secs(3);
        let d = retry_after_from(&with_header(httpdate::fmt_http_date(future)));
        assert!(
            d >= Duration::from_secs(1) && d <= Duration::from_secs(3),
            "http-date Retry-After should be ~3s, got {d:?}"
        );

        // A past HTTP-date clamps to zero.
        let past = std::time::SystemTime::now() - Duration::from_secs(120);
        assert_eq!(
            retry_after_from(&with_header(httpdate::fmt_http_date(past))),
            Duration::ZERO
        );

        // Unparseable header → zero.
        assert_eq!(
            retry_after_from(&with_header("soon".to_string())),
            Duration::ZERO
        );
    }
}
