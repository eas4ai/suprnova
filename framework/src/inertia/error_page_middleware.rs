//! Framework error responses: the application's error callback, or the
//! app's Inertia error page.
//!
//! The Inertia client treats a response without an `X-Inertia` header as
//! non-Inertia (`inertia-3.6.1/packages/core/src/response.ts:68,173-175`)
//! and hands it to `dialog.show(...)` - the full-screen "All Inertia
//! requests must receive a valid Inertia response, however a plain JSON
//! response was received" modal (`response.ts:168-169`). Every non-2xx
//! the framework produces takes that path: the `403` from
//! [`PermissionMiddleware`](crate::PermissionMiddleware) or
//! [`Gate::authorize`](crate::Gate), the `404` for an unrouted path, a
//! `429`, a `500`. A user with the wrong role clicks a nav link and gets
//! a crash screen instead of a page.
//!
//! Laravel + Inertia solve this in the exception handler:
//! `Inertia::handleExceptionsUsing` hands every rendered exception to a
//! callback, which renders an app-defined `Error` component with the
//! status as a prop, keeping the status code. The decision cannot live in
//! `From<FrameworkError> for HttpResponse` here, because that impl has no
//! request in scope and the answer depends entirely on who asked - an
//! Inertia visit wants a page object, a hard navigation wants the HTML
//! shell, and an API client wants the JSON it has always had. So it lives
//! in a middleware, the same way
//! [`InertiaValidationRedirectMiddleware`](crate::InertiaValidationRedirectMiddleware)
//! post-processes a `422`.
//!
//! The middleware hands each error response to the callback
//! [`Inertia::handle_exceptions_using`](crate::Inertia::handle_exceptions_using)
//! installed, or, without one, to the default callback for the component
//! [`InertiaConfig::error_page`](crate::InertiaConfig::error_page) names
//! (PAR-062). With neither, it hands the request on and does nothing else.
//! [`Inertia::install`](crate::Inertia::install) always registers one. An
//! app whose stack answers before the Inertia layer is reached - CSRF, a
//! rate limiter, an auth guard registered above `install` - registers the
//! middleware itself, further out; see [`InertiaErrorPageMiddleware`] for
//! where it may sit.

use async_trait::async_trait;
use serde_json::{Map, Value};

use crate::error::ErrorReport;
use crate::http::{HttpResponse, Request, Response};
use crate::middleware::{Middleware, MiddlewareFuture, Next};

use super::exceptions::{CapturedRequest, Decision, ExceptionHandler, InertiaErrorResponse};
use super::{InertiaRequestExt, InertiaResponse, Prop};

/// Decides each framework error response: the application's error callback
/// when one is installed, else the default callback that renders the error
/// page component, keeping the original status code.
///
/// See the module documentation for why this is a middleware and not a
/// branch inside the error-to-response conversion.
///
/// # What it hands the callback
///
/// Every response with a status from `400` to `599` that the framework
/// rendered from an error: one that carries an
/// [`ErrorReport`](crate::ErrorReport) (a handler's or a middleware's
/// `Err`, a panic), and the framework's own error bodies that carry none -
/// an empty body, a JSON object with a string `message`, the router's
/// `404 Not Found`. It runs the chain inside it under the panic boundary's
/// rule, so a handler that panics reaches the callback as a `500` with the
/// panic's report. Three kinds of response are never handed over: one a
/// handler built itself in some other shape, which is the handler's
/// answer; an Inertia protocol response (`X-Inertia`,
/// `X-Inertia-Location`, `X-Inertia-Redirect`), which is an instruction to
/// the client; and a validation result, a `422` whose JSON body carries an
/// `errors` object, which
/// [`InertiaValidationRedirectMiddleware`](crate::InertiaValidationRedirectMiddleware)
/// owns. Each response is decided once, so a second instance further out
/// leaves it alone.
///
/// A page the callback renders goes through the root template
/// [`InertiaConfig::root_template_with`](crate::InertiaConfig::root_template_with)
/// picks for the request as it arrived, so an error under `/admin` keeps
/// the admin shell. It carries no view data: the handler that would have
/// set it failed or never ran.
///
/// # Registering it yourself
///
/// [`Inertia::install`](crate::Inertia::install) registers this
/// **innermost** of the Inertia layer, which is the right place for almost
/// every app: the scaffold registers `CsrfMiddleware` and the rest of its
/// stack *after* that call, so their responses pass back out through it.
///
/// It is the wrong place for an app that registers a middleware which
/// answers **before** the Inertia layer is reached - a `CsrfMiddleware`
/// registered above `Inertia::install`, an outer rate limiter, an auth
/// guard - because a middleware that returns without calling `next` never
/// hands its response to anything registered inside it. A lapsed session
/// posting a form is the case that bites: `CsrfMiddleware` answers `419`
/// with `{"message":"CSRF token mismatch."}`, that response never reaches
/// the Inertia layer, and the client shows the crash modal this exists to
/// remove. Register the middleware yourself, outside the one whose
/// rejections it should cover:
///
/// ```rust,no_run
/// use suprnova::{
///     global_middleware, CsrfMiddleware, Inertia, InertiaConfig,
///     InertiaErrorPageMiddleware, SessionConfig, SessionMiddleware,
/// };
///
/// pub fn register_http_stack() -> Result<(), suprnova::FrameworkError> {
///     global_middleware!(SessionMiddleware::new(SessionConfig::from_env()));
///     // `LocaleMiddleware::from_env()?` belongs here too, if the app is
///     // localized - the error page reads the locale share.
///     //
///     // Outside CSRF, so it sees the 419 that never reaches the layer below.
///     global_middleware!(InertiaErrorPageMiddleware::new("Error"));
///     global_middleware!(CsrfMiddleware::new());
///     Inertia::install(&InertiaConfig::new().error_page("Error"))
/// }
/// ```
///
/// `Inertia::install` sees the registration and skips its own, so the
/// position you chose is the one that stands - and so does the component
/// you named here, since this instance is the one in the chain. The page
/// is therefore named **once**, at your own registration, and
/// [`InertiaConfig::error_page`](crate::InertiaConfig::error_page) becomes
/// optional. An error callback, when one is installed, decides in place of
/// the component this instance names.
///
/// **Where it must sit.** After
/// [`SessionMiddleware`](crate::SessionMiddleware) and `LocaleMiddleware`,
/// always. The page it renders can carry the app's shared props - `auth`,
/// the locale share, flash - and it renders on the way *out*, once every
/// middleware registered inside it has returned and popped whatever
/// request scope it opened. Registered above those two, every error page
/// loses the visitor's session and locale. Then: before the middleware
/// whose rejections it should cover, and nowhere further out than that.
pub struct InertiaErrorPageMiddleware {
    /// The component the default callback renders. `None` for the instance
    /// `Inertia::install` registers, which reads the installed config's
    /// `error_page` per request.
    component: Option<String>,
}

impl InertiaErrorPageMiddleware {
    /// Build the middleware for a page component name (e.g. `"Error"`).
    ///
    /// Reach for this only to register the middleware at a position of
    /// your own choosing - see [the type's docs](Self) for when that is
    /// needed and where it may sit. Otherwise name the component with
    /// [`InertiaConfig::error_page`](crate::InertiaConfig::error_page) and
    /// let [`Inertia::install`](crate::Inertia::install) place it.
    pub fn new(component: impl Into<String>) -> Self {
        Self {
            component: Some(component.into()),
        }
    }

    /// The instance `Inertia::install` and the route-group stack place:
    /// the default callback renders `component`, or the installed config's
    /// `error_page` when that is `None`.
    pub(crate) fn with_component(component: Option<String>) -> Self {
        Self { component }
    }

    /// Who decides this request's error responses, if anyone does.
    ///
    /// Read per request from the active container, so a callback or an
    /// install under `TestContainer::fake()` decides only that test's
    /// requests.
    fn decider(&self) -> Option<Decider> {
        let registry = crate::App::inertia_registry();
        if let Some(callback) = registry.exception_handler() {
            return Some(Decider::Callback(callback));
        }
        self.component
            .clone()
            .or_else(|| registry.installed_error_page())
            .map(Decider::Page)
    }
}

/// Who decides a request's error responses.
enum Decider {
    /// The callback `Inertia::handle_exceptions_using` installed.
    Callback(ExceptionHandler),
    /// The default callback, rendering this error page component.
    Page(String),
}

#[async_trait]
impl Middleware for InertiaErrorPageMiddleware {
    async fn handle(&self, request: Request, next: Next) -> Response {
        let Some(decider) = self.decider() else {
            return next(request).await;
        };
        // The default callback renders only for an Inertia visit or a
        // browser navigation. For anyone else the response is exactly the
        // one the framework sends without this middleware, a panic's `500`
        // included, so the request is handed on untouched: an API client -
        // the common case for a service that also serves an SPA - pays a
        // registry read and two header lookups and nothing else.
        if matches!(decider, Decider::Page(_)) && audience(&request) == Audience::Neither {
            return next(request).await;
        }

        let captured = CapturedRequest::capture(&request);
        let response = run_catching_panics(&captured, next(request)).await;
        let was_ok = response.is_ok();
        let http = response.unwrap_or_else(|e| e);
        let restore = |http| if was_ok { Ok(http) } else { Err(http) };

        if http.is_error_decided() || !is_error_response(&http) {
            return restore(http);
        }
        // With debug on, the response the framework would send for a 5xx
        // a browser or an Inertia visit gets is the development error page
        // (PAR-012), so that is what the callback receives.
        let http = crate::error::debug_page::page_for(http);
        restore(decide(&decider, &captured, http).await.mark_error_decided())
    }
}

/// Run the rest of the chain under the panic boundary's rule: a panic
/// becomes the `500` the server's `execute_chain_safely` answers with, its
/// report included, so the callback sees it. The panic is caught inside the
/// request's `REQUEST_ID` scope, which `RequestIdMiddleware` keeps open
/// around this middleware, so the body and the log carry the request id.
async fn run_catching_panics(captured: &CapturedRequest, chain: MiddlewareFuture) -> Response {
    match crate::error::catch_panic(chain).await {
        Ok(response) => response,
        Err(panic) => {
            let request_id = crate::logging::current_request_id();
            Err(crate::server::panic_into_response(
                panic,
                &captured.method,
                captured.path(),
                request_id.as_ref().map_or("", |id| id.as_str()),
            ))
        }
    }
}

/// Whether `response` is an error response the framework rendered, the
/// ones the callback is handed. See [`InertiaErrorPageMiddleware`].
fn is_error_response(response: &HttpResponse) -> bool {
    if !(400..=599).contains(&response.status_code())
        || is_protocol_response(response)
        || is_validation_result(response)
    {
        return false;
    }
    response.error_report().is_some()
        || (!response.is_streaming()
            && replaceable_body(response.header_value("Content-Type"), response.body()).is_some())
}

/// Whether `response` is an Inertia protocol response: a page, or an
/// instruction the client acts on by its header rather than its body.
fn is_protocol_response(response: &HttpResponse) -> bool {
    response
        .header_values("X-Inertia")
        .any(|v| v.eq_ignore_ascii_case("true"))
        || response.header_value("X-Inertia-Location").is_some()
        || response.header_value("X-Inertia-Redirect").is_some()
}

/// Whether `response` is a validation result: a `422` whose JSON body
/// carries an `errors` object, the framework's
/// `{"message": .., "errors": {..}}`. The validation redirect owns it: it
/// turns an Inertia visit's `422` into the redirect back to the form with
/// the errors flashed, and an API client or a Precognition dry run reads
/// the errors off it. A callback that rendered it would break every form.
fn is_validation_result(response: &HttpResponse) -> bool {
    response.status_code() == 422
        && !response.is_streaming()
        && serde_json::from_slice::<Value>(response.body())
            .ok()
            .is_some_and(|body| body.get("errors").is_some_and(Value::is_object))
}

/// Hand `http` to the decider and build what it chose.
async fn decide(decider: &Decider, captured: &CapturedRequest, http: HttpResponse) -> HttpResponse {
    let (decision, shared_data) = {
        // A response the framework built without an error behind it carries
        // no report; the callback reads one made from its message.
        let made;
        let error = match http.error_report() {
            Some(report) => report,
            None => {
                made = ErrorReport::from_message(report_message(&http));
                &made
            }
        };
        let response = InertiaErrorResponse::new(error, captured, &http);
        let decided = match decider {
            Decider::Callback(callback) => callback(response),
            Decider::Page(component) => default_callback(component, response),
        };
        decided.map_or((Decision::Keep, false), InertiaErrorResponse::into_decision)
    };
    match decision {
        Decision::Keep => http,
        // The replacement answers the same failure, so the report stays.
        Decision::Respond(replacement) => replacement.with_error_report_of(http),
        Decision::Render { component, props } => match props {
            Ok(props) => render_page(captured, http, component, props, shared_data).await,
            Err(problem) => {
                tracing::warn!(
                    %component,
                    status = http.status_code(),
                    request_id = ?crate::logging::current_request_id(),
                    %problem,
                    "Inertia error page props are not an object; returning the original error response"
                );
                http
            }
        },
    }
}

/// The message a response without a report is reported with: its error
/// body's `message`, or its status's reason phrase.
fn report_message(response: &HttpResponse) -> String {
    let message = match replaceable_body(response.header_value("Content-Type"), response.body()) {
        Some(ReplaceableBody::FrameworkError(fields)) => fields
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string),
        _ => None,
    };
    message.unwrap_or_else(|| reason_phrase(response.status_code()))
}

/// The callback [`InertiaConfig::error_page`](crate::InertiaConfig::error_page)
/// installs: the rule [`decide_page`] states, rendering `component` with
/// the shared props for the responses it covers and keeping every other
/// response.
///
/// With debug on, a `5xx` it receives for a browser or an Inertia visit is
/// the development error page, an HTML body, which the rule leaves alone.
fn default_callback<'a>(
    component: &str,
    error: InertiaErrorResponse<'a>,
) -> Option<InertiaErrorResponse<'a>> {
    let request = error.request();
    let response = error.response();
    let facts = ErrorResponseFacts {
        status: response.status_code(),
        is_inertia_request: request.is_inertia(),
        accept: request.header("Accept"),
        response_is_inertia_page: response
            .header_values("X-Inertia")
            .any(|v| v.eq_ignore_ascii_case("true")),
        has_inertia_location: response.header_value("X-Inertia-Location").is_some(),
        is_streaming: response.is_streaming(),
        content_type: response.header_value("Content-Type"),
        body: response.body(),
    };
    let ErrorPageDecision::Render(props) = decide_page(&facts) else {
        return None;
    };
    let mut page = Map::new();
    page.insert("status".to_string(), props.status.into());
    page.insert("message".to_string(), props.message.into());
    if let Some(request_id) = props.request_id {
        page.insert("request_id".to_string(), request_id.into());
    }
    Some(
        error
            .render(component, Value::Object(page))
            .with_shared_data(),
    )
}

/// Render `component` with `props` in place of `http`, keeping its status.
///
/// When the page fails to render, `http` is sent as it is.
async fn render_page(
    captured: &CapturedRequest,
    http: HttpResponse,
    component: String,
    props: Map<String, Value>,
    shared_data: bool,
) -> HttpResponse {
    // The replaced response's headers come along except the ones that only
    // described the body being replaced, or how it could be stored - see
    // `header_survives_rewrite` for the rule and why it is phrased as a
    // drop list. This is what keeps `Retry-After` on a `429` and
    // `WWW-Authenticate` on a `401` true after the body becomes a page.
    let carried: Vec<(String, String)> = http
        .headers()
        .filter(|(name, _)| header_survives_rewrite(name))
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect();

    let status = http.status_code();
    let mut page = InertiaResponse::new(component.clone());
    for (key, value) in props {
        page = page.prop(key, Prop::eager(value));
    }
    if !shared_data {
        page = page.without_shared_data();
    }

    // Rendered under the root captured before the handler ran, so the
    // page's URL, its Vite tags and its shared props name the root the
    // request arrived under (PFX-002). The root template is the one the
    // chooser picks for the captured request, and the page has no view
    // data of its own (RDOC-006).
    let root = std::sync::Arc::clone(&captured.root);
    match crate::routing::root::scope(root, page.resolve(captured)).await {
        Ok(rendered) => rendered
            .status(status)
            .with_headers(carried)
            // Last, and unconditional: the page is per-viewer where the
            // body it replaced was not.
            .header("Cache-Control", ERROR_PAGE_CACHE_CONTROL)
            // The page replaces the body, not what went wrong.
            .with_error_report_of(http),
        Err(e) => {
            // The error page failing is not a reason to lose the error.
            // Returning the original response means the user sees the modal
            // again - bad, but recoverable and truthful - rather than a
            // second failure masking the first.
            tracing::warn!(
                %component,
                status,
                request_id = ?crate::logging::current_request_id(),
                error = %e,
                "Inertia error page failed to render; returning the original error response"
            );
            http
        }
    }
}

/// Who the response is for. Only these two audiences can be handed a
/// page instead of the body they would otherwise get.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Audience {
    /// An Inertia XHR visit (`X-Inertia: true`).
    InertiaVisit,
    /// A hard browser navigation: no `X-Inertia`, and an `Accept` that
    /// prefers HTML over JSON.
    BrowserNavigation,
    /// Everything else - an API client, a webhook, a health probe.
    Neither,
}

fn audience<R: InertiaRequestExt + ?Sized>(request: &R) -> Audience {
    if request.is_inertia() {
        Audience::InertiaVisit
    } else if prefers_html_over_json(request.header("Accept")) {
        Audience::BrowserNavigation
    } else {
        Audience::Neither
    }
}

/// Props the error component receives.
#[derive(Debug, PartialEq, Eq)]
struct ErrorPageProps {
    status: u16,
    message: String,
    request_id: Option<String>,
}

/// Outcome of the one decision this middleware makes.
#[derive(Debug, PartialEq, Eq)]
enum ErrorPageDecision {
    /// Leave the response exactly as the rest of the framework built it.
    PassThrough,
    /// Replace it with the error page carrying these props.
    Render(ErrorPageProps),
}

/// Everything the decision reads, lifted off `Request` and
/// `HttpResponse` so the rule set is a pure function.
struct ErrorResponseFacts<'a> {
    status: u16,
    is_inertia_request: bool,
    accept: Option<&'a str>,
    response_is_inertia_page: bool,
    has_inertia_location: bool,
    is_streaming: bool,
    content_type: Option<&'a str>,
    body: &'a [u8],
}

/// The default callback's whole rule set, in one place.
///
/// Every arm that returns [`ErrorPageDecision::PassThrough`] is a
/// contract somebody else already owns; taking the response from them
/// would break the thing they exist to do.
fn decide_page(facts: &ErrorResponseFacts<'_>) -> ErrorPageDecision {
    // Only client and server errors. This is also what leaves `302`
    // and every other redirect alone.
    if !(400..=599).contains(&facts.status) {
        return ErrorPageDecision::PassThrough;
    }
    // A streaming body reports an empty buffered slice because nothing
    // has been produced yet - there is no body here to judge, and
    // replacing a stream mid-flight would truncate it.
    if facts.is_streaming {
        return ErrorPageDecision::PassThrough;
    }
    // Already a valid Inertia response: a handler that rendered its own
    // page and gave it an error status.
    if facts.response_is_inertia_page {
        return ErrorPageDecision::PassThrough;
    }
    // `X-Inertia-Location` is a client instruction to do a full-page
    // visit - the version middleware's `409`, and the RBAC middlewares'
    // `redirect_to` denial. The client acts on the header, not the body.
    if facts.has_inertia_location {
        return ErrorPageDecision::PassThrough;
    }
    // `422` belongs to `InertiaValidationRedirectMiddleware`, which
    // bounces it back to the form with the errors flashed. A `422` that
    // survives that middleware did so deliberately (no `errors` object,
    // or a Precognition dry-run whose contract is that the client reads
    // the errors off this very response).
    if facts.status == 422 {
        return ErrorPageDecision::PassThrough;
    }
    // An API client keeps the JSON contract it has always had.
    if !facts.is_inertia_request && !prefers_html_over_json(facts.accept) {
        return ErrorPageDecision::PassThrough;
    }

    let Some(body) = replaceable_body(facts.content_type, facts.body) else {
        return ErrorPageDecision::PassThrough;
    };

    let (message, request_id) = match body {
        // The framework's own error bodies are `{ message, request_id }`,
        // with `message` already sanitized for `5xx` - the page must show
        // the same string the JSON path would have.
        ReplaceableBody::FrameworkError(fields) => (
            fields
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string),
            fields
                .get("request_id")
                .and_then(Value::as_str)
                .map(str::to_string),
        ),
        ReplaceableBody::Empty | ReplaceableBody::RouterNotFound => (None, None),
    };

    ErrorPageDecision::Render(ErrorPageProps {
        status: facts.status,
        message: message.unwrap_or_else(|| reason_phrase(facts.status)),
        request_id,
    })
}

/// `Cache-Control` the error page always sets for itself, replacing
/// whatever the response it stands in for carried.
///
/// The body being replaced said nothing about any particular viewer - the
/// framework's JSON envelope, or a fixed `404 Not Found`. The page that
/// replaces it carries the app's shared props: `auth.user`, flash, the
/// locale share. That is per-viewer content, and it must never be stored
/// by a shared cache and handed to somebody else, whatever the original
/// response permitted. Symfony and Laravel default a session-bearing
/// response to exactly this pair, and so does this.
const ERROR_PAGE_CACHE_CONTROL: &str = "no-cache, private";

/// Whether a header on the replaced response carries over onto the page.
///
/// [`InertiaHeadersMiddleware`](crate::InertiaHeadersMiddleware) applies the
/// same rule when it turns an empty Inertia response into a redirect back.
///
/// One rule, stated as what is **dropped** rather than what is kept, so a
/// header nobody here thought of survives instead of silently
/// disappearing. A field is dropped in exactly three cases.
///
/// **It described the representation being replaced, or how that
/// representation was framed on the wire.**
///
/// - **Every `Content-*` field.** `Content-Length: 94` on a four-kilobyte
///   HTML shell is a framing bug, `Content-Type: application/json` on a
///   page object is a lie, and `Content-Encoding: gzip` claims a
///   compression that was never applied to the new body. The test is a
///   prefix rather than an enumeration on purpose: a representation field
///   added to HTTP after this was written is dropped by default, which is
///   the safe direction for metadata that describes bytes we threw away.
/// - **`Transfer-Encoding`**, the framing counterpart of
///   `Content-Length`, which shares none of that prefix.
///
/// **It described how the replaced representation could be stored or
/// revalidated.** `Cache-Control`, `Expires`, `Age`, `ETag`,
/// `Last-Modified`. The page is per-viewer where the body it replaced was
/// not, so inheriting the old response's storage permission is how one
/// viewer's `auth.user` ends up served to another out of a shared cache,
/// and inheriting its validators is how a cache revalidates the page
/// against an entity that no longer exists. The page sets its own
/// [`ERROR_PAGE_CACHE_CONTROL`] instead of carrying one.
///
/// **The page response is the authority on it.** `X-Inertia`.
/// Unreachable in practice - a response already carrying it never reaches
/// the rewrite, see [`is_error_response`] - and stated anyway so the
/// rewrite cannot inherit a contradictory claim.
///
/// `Content-Security-Policy` and `Content-Security-Policy-Report-Only`
/// are the one carve-out from the `Content-` prefix. They share it and
/// are not representation metadata at all - they are response policy -
/// and dropping a CSP from an error page would be a security regression.
///
/// Everything else describes the request, the connection, or what the
/// client should do next: `Retry-After` on a `429`, `WWW-Authenticate` on
/// a `401`, `Vary`, `Set-Cookie`, `X-Request-Id`. None of that stopped
/// being true because the body changed.
pub(crate) fn header_survives_rewrite(name: &str) -> bool {
    const CONTENT: &[u8] = b"Content-";
    let bytes = name.as_bytes();
    let content_prefixed =
        bytes.len() >= CONTENT.len() && bytes[..CONTENT.len()].eq_ignore_ascii_case(CONTENT);
    let security_policy = name.eq_ignore_ascii_case("Content-Security-Policy")
        || name.eq_ignore_ascii_case("Content-Security-Policy-Report-Only");
    let storage_metadata = ["Cache-Control", "Expires", "Age", "ETag", "Last-Modified"]
        .iter()
        .any(|field| name.eq_ignore_ascii_case(field));

    !((content_prefixed && !security_policy)
        || storage_metadata
        || name.eq_ignore_ascii_case("Transfer-Encoding")
        || name.eq_ignore_ascii_case("X-Inertia"))
}

/// Body shapes an error page may stand in for.
enum ReplaceableBody {
    /// No body at all.
    Empty,
    /// The framework's standard error envelope.
    FrameworkError(serde_json::Map<String, Value>),
    /// The fixed `404` the router and the static-file handler emit when
    /// nothing matched.
    RouterNotFound,
}

/// Classify the body, or refuse to touch it.
///
/// The gate is the **shape** of the body, not its author. It cannot be
/// otherwise: the brief's own 401 case is a body some middleware wrote by
/// hand, and it has to become a page for the same reason the framework's
/// own does - the client would modal it either way. So an
/// `application/json` object carrying a string `message` is replaceable
/// whoever built it, and only `message` and `request_id` survive into the
/// props.
///
/// What that leaves as the opt-out, and what the manual documents: a body
/// in any other shape - an app's own HTML error page, plain text that is
/// not the router's own `404`, a JSON envelope keyed differently - passes
/// through untouched, as does a response already marked `X-Inertia`.
fn replaceable_body(content_type: Option<&str>, body: &[u8]) -> Option<ReplaceableBody> {
    if body.is_empty() {
        return Some(ReplaceableBody::Empty);
    }
    let content_type = content_type.unwrap_or("");
    if content_type.starts_with("application/json") {
        let Ok(Value::Object(fields)) = serde_json::from_slice::<Value>(body) else {
            return None;
        };
        // `message` is what makes it the framework's envelope rather
        // than some other JSON that happens to carry an error status.
        if !fields.get("message").is_some_and(Value::is_string) {
            return None;
        }
        return Some(ReplaceableBody::FrameworkError(fields));
    }
    if content_type.starts_with("text/plain") && body == crate::http::NOT_FOUND_BODY.as_bytes() {
        return Some(ReplaceableBody::RouterNotFound);
    }
    None
}

/// Whether the caller would rather have a page than a JSON document.
///
/// A hard navigation from a browser sends
/// `text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8`, so
/// HTML outranks JSON. `curl` sends `*/*`, which ranks them equally - and
/// equal is not "prefers", so an unopinionated client keeps its JSON.
/// An explicit `Accept: application/json` never reaches here at all.
fn prefers_html_over_json(accept: Option<&str>) -> bool {
    let Some(accept) = accept else {
        return false;
    };
    quality_for(accept, "text", "html") > quality_for(accept, "application", "json")
}

/// Quality assigned to one media type by an `Accept` header, as
/// thousandths so the result is an integer and orders exactly. RFC 9110
/// qvalues carry up to three decimals; hundredths tied `0.502` with
/// `0.501` and read `0.004` as a refusal.
///
/// RFC 9110 §12.5.1 resolves a type against the **most specific**
/// matching range, not the highest-scoring one: given
/// `*/*;q=0.8, text/html;q=0.1`, `text/html` is 0.1. Returns `0` when
/// nothing matches.
fn quality_for(accept: &str, media_type: &str, subtype: &str) -> u16 {
    // Higher is more specific: exact type/subtype, then type/*, then */*.
    let mut best_specificity = -1i8;
    let mut best_quality = 0u16;

    for range in accept.split(',') {
        let mut parts = range.split(';');
        // `split` always yields at least one item, so this names the
        // whole range when it carries no parameters.
        let name = parts.next().unwrap_or(range).trim();
        let (range_type, range_subtype) = match name.split_once('/') {
            Some(pair) => pair,
            None => continue,
        };
        let specificity = if range_type.eq_ignore_ascii_case(media_type)
            && range_subtype.eq_ignore_ascii_case(subtype)
        {
            2
        } else if range_type.eq_ignore_ascii_case(media_type) && range_subtype == "*" {
            1
        } else if range_type == "*" && range_subtype == "*" {
            0
        } else {
            continue;
        };
        if specificity < best_specificity {
            continue;
        }
        let quality = parts
            .filter_map(|param| {
                let (key, value) = param.split_once('=')?;
                key.trim().eq_ignore_ascii_case("q").then_some(value.trim())
            })
            .next()
            .map_or(1000, parse_quality);
        // A later range at the same specificity wins ties, which only
        // matters for a malformed header listing the same range twice.
        best_specificity = specificity;
        best_quality = quality;
    }

    best_quality
}

/// Parse an RFC 9110 qvalue (`0`..`1` with up to three decimals) into
/// thousandths. Anything unparseable reads as `0`, matching the spec's
/// "a sender that does not want the type" default for a broken value.
fn parse_quality(raw: &str) -> u16 {
    raw.parse::<f32>()
        .ok()
        .filter(|q| (0.0..=1.0).contains(q))
        .map_or(0, |q| (q * 1000.0).round() as u16)
}

/// The status's reason phrase, for a body that carried no message.
fn reason_phrase(status: u16) -> String {
    hyper::StatusCode::from_u16(status)
        .ok()
        .and_then(|s| s.canonical_reason())
        .unwrap_or("Error")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts<'a>(status: u16, content_type: &'a str, body: &'a [u8]) -> ErrorResponseFacts<'a> {
        ErrorResponseFacts {
            status,
            is_inertia_request: true,
            accept: None,
            response_is_inertia_page: false,
            has_inertia_location: false,
            is_streaming: false,
            content_type: Some(content_type),
            body,
        }
    }

    const UNAUTHORIZED: &[u8] =
        br#"{"message":"This action is unauthorized.","request_id":"b4020e5d"}"#;

    #[test]
    fn a_framework_denial_becomes_the_page_with_its_message_and_request_id() {
        assert_eq!(
            decide_page(&facts(403, "application/json", UNAUTHORIZED)),
            ErrorPageDecision::Render(ErrorPageProps {
                status: 403,
                message: "This action is unauthorized.".to_string(),
                request_id: Some("b4020e5d".to_string()),
            })
        );
    }

    #[test]
    fn a_body_without_a_message_falls_back_to_the_reason_phrase() {
        // The router's own unrouted 404, and a body-less error.
        assert_eq!(
            decide_page(&facts(404, "text/plain", b"404 Not Found")),
            ErrorPageDecision::Render(ErrorPageProps {
                status: 404,
                message: "Not Found".to_string(),
                request_id: None,
            })
        );
        assert_eq!(
            decide_page(&facts(429, "", b"")),
            ErrorPageDecision::Render(ErrorPageProps {
                status: 429,
                message: "Too Many Requests".to_string(),
                request_id: None,
            })
        );
    }

    #[test]
    fn a_null_request_id_does_not_become_the_string_null() {
        // Outside a request scope the framework serializes `request_id`
        // as JSON null. The prop must be absent, not `"null"`.
        assert_eq!(
            decide_page(&facts(
                500,
                "application/json",
                br#"{"message":"Internal Server Error","request_id":null}"#
            )),
            ErrorPageDecision::Render(ErrorPageProps {
                status: 500,
                message: "Internal Server Error".to_string(),
                request_id: None,
            })
        );
    }

    #[test]
    fn everything_another_contract_owns_passes_through() {
        // Success and redirects.
        assert_eq!(
            decide_page(&facts(200, "text/html", b"ok")),
            ErrorPageDecision::PassThrough
        );
        assert_eq!(
            decide_page(&facts(302, "", b"")),
            ErrorPageDecision::PassThrough
        );
        // Validation belongs to the redirect-back middleware.
        assert_eq!(
            decide_page(&facts(
                422,
                "application/json",
                br#"{"message":"The given data was invalid.","errors":{"email":["r"]}}"#
            )),
            ErrorPageDecision::PassThrough
        );

        // A response that already is an Inertia page.
        let mut already = facts(410, "application/json", br#"{"component":"Gone"}"#);
        already.response_is_inertia_page = true;
        assert_eq!(decide_page(&already), ErrorPageDecision::PassThrough);

        // A version-mismatch bounce, or an RBAC `redirect_to` denial.
        let mut bounce = facts(409, "", b"");
        bounce.has_inertia_location = true;
        assert_eq!(decide_page(&bounce), ErrorPageDecision::PassThrough);

        // A streaming body has produced nothing yet to inspect.
        let mut streaming = facts(500, "text/event-stream", b"");
        streaming.is_streaming = true;
        assert_eq!(decide_page(&streaming), ErrorPageDecision::PassThrough);
    }

    #[test]
    fn a_body_in_any_other_shape_is_left_alone() {
        // The gate is the body's shape, not its author - so what is left
        // alone is everything that is not an error envelope.
        //
        // A handler's own HTML error page.
        assert_eq!(
            decide_page(&facts(404, "text/html; charset=utf-8", b"<h1>gone</h1>")),
            ErrorPageDecision::PassThrough
        );
        // A handler's own JSON envelope, in some other shape.
        assert_eq!(
            decide_page(&facts(
                402,
                "application/json",
                br#"{"error":"payment_required"}"#
            )),
            ErrorPageDecision::PassThrough
        );
        // A plain-text body that is not the router's fixed 404.
        assert_eq!(
            decide_page(&facts(404, "text/plain", b"no such widget")),
            ErrorPageDecision::PassThrough
        );
        // JSON that does not parse.
        assert_eq!(
            decide_page(&facts(500, "application/json", b"{not json")),
            ErrorPageDecision::PassThrough
        );
    }

    #[test]
    fn an_api_client_keeps_its_json_and_a_browser_gets_the_page() {
        let json_client = ErrorResponseFacts {
            is_inertia_request: false,
            accept: Some("application/json"),
            ..facts(403, "application/json", UNAUTHORIZED)
        };
        assert_eq!(decide_page(&json_client), ErrorPageDecision::PassThrough);

        let browser = ErrorResponseFacts {
            is_inertia_request: false,
            accept: Some("text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"),
            ..facts(403, "application/json", UNAUTHORIZED)
        };
        assert!(matches!(
            decide_page(&browser),
            ErrorPageDecision::Render(_)
        ));

        // No `Accept` at all is not a browser navigation.
        let bare = ErrorResponseFacts {
            is_inertia_request: false,
            accept: None,
            ..facts(403, "application/json", UNAUTHORIZED)
        };
        assert_eq!(decide_page(&bare), ErrorPageDecision::PassThrough);
    }

    #[test]
    fn content_negotiation_follows_the_most_specific_range() {
        // A real browser navigation.
        assert!(prefers_html_over_json(Some(
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"
        )));
        assert!(prefers_html_over_json(Some("text/html")));
        assert!(prefers_html_over_json(Some("text/*")));
        // `curl` and every other unopinionated client: equal, not
        // preferred.
        assert!(!prefers_html_over_json(Some("*/*")));
        assert!(!prefers_html_over_json(Some("application/json")));
        assert!(!prefers_html_over_json(Some("application/json, */*;q=0.1")));
        assert!(!prefers_html_over_json(None));
        // The specific range wins over the wildcard even when the
        // wildcard scores higher - RFC 9110 12.5.1.
        assert!(!prefers_html_over_json(Some("*/*;q=0.8, text/html;q=0.1")));
        // The Inertia client's own `Accept` prefers HTML, which is why
        // Laravel's `expectsJson()` is false for an Inertia visit.
        assert!(prefers_html_over_json(Some(
            "text/html, application/xhtml+xml"
        )));
        // Junk must not panic or accidentally prefer anything.
        assert!(!prefers_html_over_json(Some("")));
        assert!(!prefers_html_over_json(Some("garbage")));
        assert!(!prefers_html_over_json(Some("text/html;q=nope")));
    }

    /// RFC 9110 qvalues carry up to three decimals. Rounding them to
    /// hundredths tied `0.502` with `0.501` and read `0.004` as a refusal.
    #[test]
    fn content_negotiation_keeps_three_decimal_qvalues() {
        assert!(prefers_html_over_json(Some(
            "text/html;q=0.502, application/json;q=0.501"
        )));
        assert!(!prefers_html_over_json(Some(
            "text/html;q=0.501, application/json;q=0.502"
        )));
        // A small positive weight is still acceptable, so it outranks an
        // explicit refusal.
        assert!(prefers_html_over_json(Some(
            "text/html;q=0.004, application/json;q=0"
        )));
        assert_eq!(quality_for("text/html;q=0.001", "text", "html"), 1);
        assert_eq!(quality_for("text/html", "text", "html"), 1000);
    }

    #[test]
    fn only_what_described_the_replaced_body_is_dropped() {
        // Framing and representation metadata for a body that no longer
        // exists.
        for dropped in [
            "Content-Type",
            "content-length",
            "Content-Encoding",
            "Content-Language",
            "Content-Location",
            "Content-Range",
            "Content-Disposition",
            "Transfer-Encoding",
            "X-Inertia",
        ] {
            assert!(
                !header_survives_rewrite(dropped),
                "{dropped} describes the body being replaced and must not carry over"
            );
        }

        // Anything about the request, the connection, or what the client
        // does next is still true after the body changes.
        for kept in [
            "Retry-After",
            "WWW-Authenticate",
            "Vary",
            "Set-Cookie",
            "X-Request-Id",
            "Access-Control-Allow-Origin",
            "Strict-Transport-Security",
            "Precognition",
            "X-Inertia-Location",
        ] {
            assert!(
                header_survives_rewrite(kept),
                "{kept} says nothing about the body and must carry over"
            );
        }
    }

    #[test]
    fn nothing_that_let_the_old_body_be_stored_or_revalidated_carries_over() {
        // The body being replaced was the same for everyone. The page is
        // not - it carries `auth.user`, flash, the locale share. Letting
        // it inherit the old response's storage permission is how one
        // viewer's page is served to another out of a shared cache, and
        // letting it inherit the old validators is how a cache
        // revalidates the page against an entity that no longer exists.
        for dropped in [
            "Cache-Control",
            "cache-control",
            "Expires",
            "Age",
            "ETag",
            "Last-Modified",
        ] {
            assert!(
                !header_survives_rewrite(dropped),
                "{dropped} governed the storage of a body that no longer exists"
            );
        }
        // The page names its own policy rather than carrying one.
        assert_eq!(ERROR_PAGE_CACHE_CONTROL, "no-cache, private");
    }

    #[test]
    fn the_security_policy_headers_are_not_content_headers() {
        // They share the prefix by historical accident. Dropping a CSP
        // from the error page would be a security regression, so the
        // prefix rule carves them out by name.
        assert!(header_survives_rewrite("Content-Security-Policy"));
        assert!(header_survives_rewrite(
            "content-security-policy-report-only"
        ));
        // A short name that merely starts with "Content" is not prefixed
        // by "Content-" and must not be swept up.
        assert!(header_survives_rewrite("Content"));
    }

    #[test]
    fn only_responses_the_framework_rendered_from_an_error_reach_the_callback() {
        use crate::error::FrameworkError;

        // Built from an error: it carries a report, whatever its body.
        assert!(is_error_response(&HttpResponse::from(
            FrameworkError::domain("denied", 403)
        )));
        // The framework's own error bodies that carry no report.
        assert!(is_error_response(
            &HttpResponse::text(crate::http::NOT_FOUND_BODY).status(404)
        ));
        assert!(is_error_response(
            &HttpResponse::json(serde_json::json!({ "message": "CSRF token mismatch." }))
                .status(419)
        ));
        assert!(is_error_response(&HttpResponse::new().status(429)));

        // A handler's own answer in another shape, and anything that is not
        // an error status.
        assert!(!is_error_response(
            &HttpResponse::html("<h1>No such widget</h1>").status(404)
        ));
        assert!(!is_error_response(&HttpResponse::new().status(302)));
        assert!(!is_error_response(&HttpResponse::from(
            FrameworkError::domain("not an error status", 302)
        )));

        // Inertia protocol responses are instructions to the client, a
        // report or not.
        for header in ["X-Inertia-Location", "X-Inertia-Redirect"] {
            assert!(
                !is_error_response(&HttpResponse::new().status(409).header(header, "/next")),
                "{header}"
            );
        }
        assert!(!is_error_response(
            &HttpResponse::from(FrameworkError::domain("gone", 410)).header("X-Inertia", "true")
        ));

        // A validation result belongs to the validation redirect, though it
        // carries a report. A 422 in any other shape is an error like any
        // other.
        let mut errors = crate::ValidationErrors::new();
        errors.add("email", "The email field is required.");
        let validation = HttpResponse::from(FrameworkError::validation_errors(errors));
        assert_eq!(validation.status_code(), 422);
        assert!(validation.error_report().is_some());
        assert!(!is_error_response(&validation));
        assert!(is_error_response(&HttpResponse::from(
            FrameworkError::domain("unprocessable", 422)
        )));
    }

    #[test]
    fn a_response_without_a_report_is_reported_by_its_message() {
        assert_eq!(
            report_message(
                &HttpResponse::json(serde_json::json!({ "message": "CSRF token mismatch." }))
                    .status(419)
            ),
            "CSRF token mismatch."
        );
        assert_eq!(
            report_message(&HttpResponse::text(crate::http::NOT_FOUND_BODY).status(404)),
            "Not Found"
        );
    }

    #[test]
    fn an_unassigned_status_still_names_something() {
        assert_eq!(reason_phrase(403), "Forbidden");
        assert_eq!(reason_phrase(499), "Error");
    }
}
