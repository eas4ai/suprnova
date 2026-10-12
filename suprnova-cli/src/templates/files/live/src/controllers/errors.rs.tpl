//! The error page.
//!
//! [`ErrorPage`] renders every framework error response (a `404` for an
//! unknown path or another user's note, a `403`, the CSRF check's `419`, a
//! `429`, a `500` outside debug mode) as `templates/live/pages/error.html`
//! in the application layout, keeping the status. In debug mode a `5xx`
//! keeps the framework's development error page, which shows the failure.
//! A client that expects JSON, by Laravel's `expectsJson` rule, keeps the
//! JSON body it always had, and the Live runtime's own requests under
//! `/__live/` keep their protocol responses.
//!
//! The page replaces the body alone. The response's other headers come
//! along (`Allow` on a `405`, `Retry-After` on a `429`), except the ones
//! that described the body being replaced or how it could be stored, and
//! the error report the framework attached stays on the page's response,
//! so the log and the exception handler still see what went wrong.
//!
//! The page is not a Live document: an error can come from a path no route
//! matched, so there is no document route to mount on, and the page has
//! nothing to make live.

use suprnova::serde_json::Value;
use suprnova::view::ViewTemplate;
use suprnova::{
    __, Config, FrameworkError, HttpResponse, Middleware, Next, Request, Response, async_trait,
};

use crate::views::{ErrorDocument, Frame, Nav};

/// Renders error responses as the error page. Registered in
/// `bootstrap::register_http_stack`.
pub struct ErrorPage;

#[async_trait]
impl Middleware for ErrorPage {
    async fn handle(&self, request: Request, next: Next) -> Response {
        let wants_page = !request.expects_json() && !request.path().starts_with("/__live/");
        let response = next(request).await;
        if !wants_page {
            return response;
        }
        let (response, failed) = match response {
            Ok(response) => (response, false),
            Err(response) => (response, true),
        };
        let page = if renders(&response) {
            match page(&response).await {
                Ok(html) => {
                    let status = response.status_code();
                    let carried: Vec<(String, String)> = response
                        .headers()
                        .filter(|(name, _)| header_survives(name))
                        .map(|(name, value)| (name.to_owned(), value.to_owned()))
                        .collect();
                    HttpResponse::html(html)
                        .status(status)
                        .with_headers(carried)
                        // The page is per viewer where the body it replaced
                        // may not have been.
                        .header("Cache-Control", "no-cache, private")
                        // The page replaces the body, not what went wrong.
                        .with_error_report_of(response)
                }
                Err(error) => {
                    // The original error response still answers the request.
                    tracing::error!(error = %error, "rendering the error page failed");
                    response
                }
            }
        } else {
            response
        };
        if failed { Err(page) } else { Ok(page) }
    }
}

/// Whether a header of the replaced response belongs on the page too: every
/// header except the ones that described the body being replaced
/// (`Content-*`, other than the security policy) or how it could be stored.
fn header_survives(name: &str) -> bool {
    let content = name.len() >= 8 && name[..8].eq_ignore_ascii_case("Content-");
    let security_policy = name.eq_ignore_ascii_case("Content-Security-Policy")
        || name.eq_ignore_ascii_case("Content-Security-Policy-Report-Only");
    let storage = ["Cache-Control", "Expires", "Age", "ETag", "Last-Modified"]
        .iter()
        .any(|field| name.eq_ignore_ascii_case(field));
    !((content && !security_policy) || storage || name.eq_ignore_ascii_case("Transfer-Encoding"))
}

/// Whether `response` is a framework error response the page replaces: an
/// error status, a body that is not already HTML, and not a `5xx` that the
/// development error page shows in debug mode.
fn renders(response: &HttpResponse) -> bool {
    let status = response.status_code();
    let html = response
        .header_value("Content-Type")
        .is_some_and(|value| value.starts_with("text/html"));
    (400..=599).contains(&status)
        && !html
        && !response.is_streaming()
        && !(status >= 500 && Config::is_debug())
}

/// The error page's HTML for `response`. The message is the error body's
/// own `message`, except for a `404`, a `419` and a `5xx`.
async fn page(response: &HttpResponse) -> Result<String, FrameworkError> {
    let status = response.status_code();
    let body: Option<Value> = suprnova::serde_json::from_slice(response.body()).ok();
    let field = |name: &str| {
        body.as_ref()
            .and_then(|body| body.get(name))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    // A `404` names a model or a route nobody asked about, and a `419`
    // names the CSRF token, so both get the page's own words; so does a
    // `5xx`, whose detail belongs in the log.
    let message = match field("message") {
        Some(message) if !matches!(status, 404 | 419) && status < 500 => message,
        _ => default_message(status).to_owned(),
    };
    let view = ErrorDocument {
        frame: Frame::plain(&status.to_string(), Nav::None).await?,
        status,
        message,
        request_id: field("request_id"),
        // `lang/en/app.ftl` holds both, so a new locale translates them.
        reference: __!("error-reference"),
        go_home: __!("error-go-home"),
    };
    let mut html = String::new();
    view.render_view(&mut html)
        .map_err(|error| FrameworkError::internal(format!("the error page: {error:?}")))?;
    Ok(html)
}

/// What the page says for `status` when the response named nothing.
fn default_message(status: u16) -> &'static str {
    match status {
        401 => "Please sign in.",
        403 => "You are not allowed to do that.",
        404 => "That page does not exist.",
        419 => "The page expired. Go back, reload it and try again.",
        429 => "Too many requests. Wait a moment and try again.",
        500..=599 => "Something went wrong on our side.",
        _ => "The request could not be completed.",
    }
}
