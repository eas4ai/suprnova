//! Deciding each error response the framework renders (PAR-062), Laravel's
//! `Inertia::handleExceptionsUsing` with its `ExceptionResponse`.
//!
//! [`Inertia::handle_exceptions_using`](crate::Inertia::handle_exceptions_using)
//! installs a callback on the active container's Inertia registry. The
//! error-response middleware ([`InertiaErrorPageMiddleware`], which
//! [`Inertia::install`](crate::Inertia::install) registers) hands it an
//! [`InertiaErrorResponse`] for every error response that passes through,
//! and sends what the callback decides. The server hands it every error
//! response no middleware decided, after the whole stack.
//! [`InertiaConfig::error_page`](crate::InertiaConfig::error_page) is the
//! same mechanism with a callback the framework supplies.
//!
//! [`InertiaErrorPageMiddleware`]: crate::InertiaErrorPageMiddleware

use std::sync::Arc;

use serde::Serialize;
use serde_json::{Map, Value};

use crate::error::ErrorReport;
use crate::http::{HttpResponse, Request};

use super::InertiaRequestExt;

/// The callback [`Inertia::handle_exceptions_using`](crate::Inertia::handle_exceptions_using)
/// installs.
pub(crate) type ExceptionHandler =
    Arc<dyn Fn(InertiaErrorResponse<'_>) -> Option<InertiaErrorResponse<'_>> + Send + Sync>;

/// One error response the framework is about to send, handed to the
/// callback [`Inertia::handle_exceptions_using`](crate::Inertia::handle_exceptions_using)
/// installed so the application decides what the client gets - Laravel's
/// `ExceptionResponse`.
///
/// It shows what went wrong ([`status`](Self::status),
/// [`error`](Self::error)), who asked ([`request`](Self::request),
/// [`method`](Self::method)) and what the framework would send
/// ([`response`](Self::response)). The callback returns it with a decision
/// to replace the response, or `None` to keep the response as it is:
///
/// - [`render`](Self::render) - an Inertia page with the original status;
/// - [`with_shared_data`](Self::with_shared_data) - include the shared
///   props in that page;
/// - [`respond_with`](Self::respond_with) - any other response.
///
/// Returned without a decision, it keeps the response too.
///
/// The lifetime ties the value to the one call it was made for: the
/// framework still owns the response it describes, which is what lets
/// `None` keep it.
pub struct InertiaErrorResponse<'a> {
    error: &'a ErrorReport,
    request: &'a CapturedRequest,
    response: &'a HttpResponse,
    decision: Decision,
    shared_data: bool,
}

/// What the callback chose.
pub(crate) enum Decision {
    /// Send the response the framework built.
    Keep,
    /// Render this page component with these props, at the original status.
    /// The props are an error when they did not serialize to an object.
    Render {
        component: String,
        props: Result<Map<String, Value>, String>,
    },
    /// Send this response instead.
    Respond(HttpResponse),
}

impl<'a> InertiaErrorResponse<'a> {
    /// Describe `response`, built from `error`, for the request `request`.
    pub(crate) fn new(
        error: &'a ErrorReport,
        request: &'a CapturedRequest,
        response: &'a HttpResponse,
    ) -> Self {
        Self {
            error,
            request,
            response,
            decision: Decision::Keep,
            shared_data: false,
        }
    }

    /// The decision, and whether the page includes the shared props.
    pub(crate) fn into_decision(self) -> (Decision, bool) {
        (self.decision, self.shared_data)
    }

    /// The status of the response the framework would send - Laravel's
    /// `statusCode()`. A page from [`render`](Self::render) keeps it.
    pub fn status(&self) -> u16 {
        self.response.status_code()
    }

    /// What went wrong: the error and its source chain, or a caught panic's
    /// message and location (PAR-010).
    ///
    /// A response the framework builds without an error behind it - the
    /// router's `404` for a path no route matches, a middleware's own
    /// `{"message": ...}` answer - carries no report of its own, so this
    /// reports its message, or the status's reason phrase.
    pub fn error(&self) -> &'a ErrorReport {
        self.error
    }

    /// The request, as it arrived, before the handler ran: its path, query
    /// and headers.
    pub fn request(&self) -> &'a dyn InertiaRequestExt {
        self.request
    }

    /// The request's method, such as `GET`.
    pub fn method(&self) -> &'a str {
        &self.request.method
    }

    /// The response the framework would send. With debug mode on, a `5xx`
    /// that carries an error report, sent to a browser or an Inertia visit,
    /// is the development error page here (PAR-012).
    pub fn response(&self) -> &'a HttpResponse {
        self.response
    }

    /// Render the Inertia page `component` with `props` in place of the
    /// response, keeping its status - Laravel's `render($component, $props)`.
    ///
    /// `props` is anything that serializes to a JSON object, such as
    /// `serde_json::json!({...})` or a `#[derive(Serialize)]` struct; `()`
    /// and `None` mean no props. The page renders the way any page does,
    /// through the root template the request's chooser picks, with
    /// `Cache-Control: no-cache, private`, the original response's headers
    /// except the ones that described its body or how it could be stored,
    /// and none of the shared props unless
    /// [`with_shared_data`](Self::with_shared_data) is called.
    ///
    /// When the props do not serialize to an object, or the page fails to
    /// render, the framework logs a warning and sends the original
    /// response.
    pub fn render(mut self, component: impl Into<String>, props: impl Serialize) -> Self {
        self.decision = Decision::Render {
            component: component.into(),
            props: props_object(props),
        };
        self
    }

    /// Include the shared props in the page [`render`](Self::render)
    /// builds - Laravel's `withSharedData()`: the shared registry
    /// (`Inertia::share`, `App::inertia_share`, the shared providers and the
    /// [`InertiaSharedData`](crate::InertiaSharedData) provider) and the
    /// middleware hooks' `share` and `share_once`. Without it the page
    /// carries only its own props.
    ///
    /// For a response the server decides after the whole stack, such as
    /// the answer of a middleware registered before
    /// [`Inertia::install`](crate::Inertia::install), the shared registry
    /// and providers are included, but every request scope a middleware
    /// opened has closed: no session data, no detected locale, and none of
    /// the middleware hooks' shares.
    pub fn with_shared_data(mut self) -> Self {
        self.shared_data = true;
        self
    }

    /// Send `response` in place of the error response. The original's
    /// error report stays on it, in process, for tests and middleware to
    /// read.
    pub fn respond_with(mut self, response: HttpResponse) -> Self {
        self.decision = Decision::Respond(response);
        self
    }
}

/// `props` as a page's prop object.
fn props_object(props: impl Serialize) -> Result<Map<String, Value>, String> {
    match serde_json::to_value(props) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(Value::Null) => Ok(Map::new()),
        Ok(other) => Err(format!(
            "the props must serialize to a JSON object, not {}",
            json_kind(&other)
        )),
        Err(e) => Err(format!("the props failed to serialize: {e}")),
    }
}

/// The name of a JSON value's kind, for an error message.
fn json_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// The parts of the request an error response is decided and rendered
/// with, captured before the handler takes the request.
///
/// The whole header map comes along rather than a hand-picked subset: an
/// app's [`InertiaSharedData`](crate::InertiaSharedData) provider is
/// handed this value when the page includes the shared props and may read
/// any header it likes, and a shared prop that silently sees fewer headers
/// on the error page than on every other page would be a trap. Cloning a
/// `HeaderMap` is one table allocation plus refcount bumps on
/// `Bytes`-backed values.
pub(crate) struct CapturedRequest {
    pub(crate) method: String,
    path: String,
    path_and_query: String,
    full_url: String,
    headers: hyper::HeaderMap,
    /// The public root of the request, taken with the rest before the
    /// handler runs (PFX-002).
    pub(crate) root: Arc<str>,
}

impl CapturedRequest {
    /// Capture `request` before it is handed on.
    pub(crate) fn capture(request: &Request) -> Self {
        Self {
            method: request.method().as_str().to_string(),
            path: Request::path(request).to_string(),
            path_and_query: InertiaRequestExt::path_and_query(request),
            full_url: InertiaRequestExt::full_url(request),
            headers: request.headers().clone(),
            root: request.public_root(),
        }
    }
}

impl InertiaRequestExt for CapturedRequest {
    fn path(&self) -> &str {
        &self.path
    }

    fn path_and_query(&self) -> String {
        self.path_and_query.clone()
    }

    fn full_url(&self) -> String {
        self.full_url.clone()
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn props_must_be_an_object_or_nothing() {
        assert_eq!(
            props_object(serde_json::json!({"status": 404})).map(|m| m.len()),
            Ok(1)
        );
        assert_eq!(props_object(()).map(|m| m.len()), Ok(0));
        assert_eq!(props_object(None::<u8>).map(|m| m.len()), Ok(0));
        assert_eq!(
            props_object(vec![1, 2]),
            Err("the props must serialize to a JSON object, not an array".to_string())
        );
        assert_eq!(
            props_object("Error"),
            Err("the props must serialize to a JSON object, not a string".to_string())
        );
    }
}
