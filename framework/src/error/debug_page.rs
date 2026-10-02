//! The development error page (PAR-012 to PAR-015).
//!
//! With debug mode on, a response with status 500 or above that carries
//! an [`ErrorReport`], sent to an Inertia visit or to a request whose
//! `Accept` lists `text/html`, is replaced by one HTML document: the
//! error chain or the panic, the stack frames recorded where it began,
//! and the request.
//!
//! # Where it happens
//!
//! In the server, around everything a request runs, because the panic
//! boundary sits above every middleware: a middleware never sees the 500
//! a panic becomes, so it could not build this page. The server reads
//! `Config::is_debug()` once per request. With it off, none of this
//! module runs: the request is not captured, no frame is recorded, and
//! the response is the one the app sends in production. With it on, the
//! server captures the request before the middleware chain consumes it
//! ([`DebugRequest::capture`]), serves the request with frames recorded
//! ([`DebugRequest::serve`]), and replaces the response when it is one
//! of the responses above. Because it runs last, the page also takes the
//! place of the app's Inertia error page for those responses.
//!
//! # What it never shows
//!
//! The request body, environment variables and configuration values. The
//! values of `Authorization`, `Proxy-Authorization`, `Cookie` and
//! `Set-Cookie`, and of every header and query parameter whose name
//! contains `token`, `secret`, `password`, `key` or `signature` in any
//! letter case, are redacted. So is every such parameter wherever else it
//! appears in the text the page shows, such as the query of a `Referer` or
//! an `X-Original-URI` header, and the password of every URL with
//! credentials.
//!
//! # What it loads
//!
//! Nothing. It is one document with an inline style sheet, no script and
//! no external resource, so it renders when the frontend build, the Vite
//! manifest, Inertia or the view layer is what failed. Every value it
//! shows passes through one escaping function ([`Html::text`]); markup
//! can only come from `&'static str` ([`Html::raw`]). The response
//! carries `Cache-Control: no-store` and a `Content-Security-Policy` that
//! allows no script.

use std::borrow::Cow;
use std::convert::Infallible;
use std::future::Future;
use std::sync::{Arc, OnceLock};

use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::StatusCode;
use hyper::header::{self, HeaderName, HeaderValue};

use super::ErrorReport;
use super::frames::{self, Frame, Origin};

/// The body type the server sends.
type Body = BoxBody<Bytes, Infallible>;

/// The page's policy: nothing may load or run, and the inline style
/// sheet may apply. `default-src 'none'` covers scripts, so no
/// `script-src` is needed to forbid them.
const CONTENT_SECURITY_POLICY: &str =
    "default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'";

/// What a redacted value is shown as.
const REDACTED: &str = "[redacted]";

/// A header or query parameter whose name contains one of these, in any
/// letter case, has its value redacted.
const SECRET_NAME_PARTS: &[&str] = &["token", "secret", "password", "key", "signature"];

/// Headers whose values are always redacted.
const CREDENTIAL_HEADERS: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
];

tokio::task_local! {
    /// What the server learns about the request while serving it: the
    /// route pattern routing matched and the request id it resolved.
    static NOTES: Arc<Notes>;
}

#[derive(Debug, Default)]
struct Notes {
    route_pattern: OnceLock<String>,
    request_id: OnceLock<String>,
}

/// Note the route pattern routing matched, for the page. Does nothing
/// unless the request is served with debug on.
pub(crate) fn note_route_pattern(pattern: &str) {
    let _ = NOTES.try_with(|notes| {
        // The first match is the request's; a second one cannot happen.
        let _ = notes.route_pattern.set(pattern.to_string());
    });
}

/// Note the request id the server resolved, for the page. Does nothing
/// unless the request is served with debug on.
pub(crate) fn note_request_id(request_id: &str) {
    let _ = NOTES.try_with(|notes| {
        // Resolved once per request; a second note would be the same id.
        let _ = notes.request_id.set(request_id.to_string());
    });
}

/// The parts of a request the page shows, captured before the middleware
/// chain takes the request. Only built with debug mode on.
#[derive(Debug)]
pub(crate) struct DebugRequest {
    method: String,
    path: String,
    query: Option<String>,
    /// Every header, in order, values decoded lossily. Redacted when the
    /// page is rendered, not here, so the rule lives in one place.
    headers: Vec<(String, String)>,
    /// `X-Inertia: true`.
    is_inertia_visit: bool,
    /// The `Accept` header lists `text/html`.
    accepts_html: bool,
    notes: Arc<Notes>,
}

impl DebugRequest {
    /// Capture what the page shows about `request`.
    pub(crate) fn capture<B>(request: &hyper::Request<B>) -> Self {
        let headers = request.headers();
        Self {
            method: request.method().as_str().to_string(),
            path: request.uri().path().to_string(),
            query: request.uri().query().map(str::to_string),
            headers: headers
                .iter()
                .map(|(name, value)| {
                    (
                        name.as_str().to_string(),
                        String::from_utf8_lossy(value.as_bytes()).into_owned(),
                    )
                })
                .collect(),
            is_inertia_visit: headers
                .get("x-inertia")
                .is_some_and(|value| value.as_bytes() == b"true"),
            accepts_html: accept_lists_html(
                headers
                    .get_all(header::ACCEPT)
                    .iter()
                    .filter_map(|value| value.to_str().ok()),
            ),
            notes: Arc::default(),
        }
    }

    /// Serve the request with `route`, recording frames, then replace the
    /// response with the page when PAR-012 asks for it.
    pub(crate) async fn serve<F>(self, route: F) -> hyper::Response<Body>
    where
        F: Future<Output = hyper::Response<Body>>,
    {
        // Boxed: the request future is large, and the two scopes would
        // otherwise hold it inline on the connection task's stack.
        let routed = NOTES.scope(Arc::clone(&self.notes), Box::pin(route));
        let response = frames::record_frames(true, routed).await;
        self.replace(response)
    }

    /// The page in place of `response`, or `response` as it is.
    fn replace(&self, response: hyper::Response<Body>) -> hyper::Response<Body> {
        let status = response.status();
        if status.as_u16() < 500 || !(self.accepts_html || self.is_inertia_visit) {
            return response;
        }
        let Some(report) = response.extensions().get::<ErrorReport>() else {
            return response;
        };
        let request_id = self.notes.request_id.get().cloned().or_else(|| {
            response
                .headers()
                .get("x-request-id")
                .and_then(|value| value.to_str().ok())
                .map(str::to_string)
        });
        let page = self.render(status, report, request_id.as_deref());

        // The status and the extensions stay, the error report among
        // them, so a test still reads it. So do the headers that say
        // something about the request or the client's next step:
        // `Set-Cookie`, `X-Request-Id`, `Vary`, `Retry-After`.
        let (mut parts, _replaced_body) = response.into_parts();
        let dropped: Vec<HeaderName> = parts
            .headers
            .keys()
            .filter(|name| !header_survives_page(name.as_str()))
            .cloned()
            .collect();
        for name in &dropped {
            parts.headers.remove(name);
        }
        parts.headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        );
        parts
            .headers
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        parts.headers.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(CONTENT_SECURITY_POLICY),
        );
        let body = Full::new(Bytes::from(page))
            .map_err(|never| match never {})
            .boxed();
        hyper::Response::from_parts(parts, body)
    }

    /// The page, as one HTML document.
    fn render(&self, status: StatusCode, report: &ErrorReport, request_id: Option<&str>) -> String {
        let headline = redact_text(report.chain().first().map_or("", String::as_str)).into_owned();
        let status_line = format!(
            "{} {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or("Server Error")
        );

        let mut html = Html::default();
        html.raw("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n")
            .raw("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n")
            .raw("<meta name=\"robots\" content=\"noindex\">\n<title>")
            .text(&status_line)
            .raw(" - ")
            .text(&headline)
            .raw("</title>\n<style>")
            .raw(STYLE)
            .raw("</style>\n</head>\n<body>\n<main>\n<header>\n<p class=\"status\">")
            .text(&status_line)
            .raw("</p>\n<h1>")
            .text(&headline)
            .raw("</h1>\n<p class=\"note\">Suprnova's development error page, shown because debug mode is on. With debug off, this request gets the response the app sends in production.</p>\n</header>\n");

        render_failure(&mut html, report);
        render_frames(&mut html, report);
        self.render_request(&mut html, request_id);

        html.raw("</main>\n</body>\n</html>\n");
        html.0
    }

    fn render_request(&self, html: &mut Html, request_id: Option<&str>) {
        html.raw("<h2>Request</h2>\n<table>\n<tr><th>Method</th><td><code>")
            .text(&self.method)
            .raw("</code></td></tr>\n<tr><th>Path</th><td><code>")
            .text(&redact_text(&self.path))
            .raw("</code></td></tr>\n<tr><th>Route</th><td>");
        match self.notes.route_pattern.get() {
            Some(pattern) => html.raw("<code>").text(pattern).raw("</code>"),
            None => html.raw("<span class=\"none\">No route matched.</span>"),
        };
        html.raw("</td></tr>\n<tr><th>Request id</th><td>");
        match request_id {
            Some(id) => html.raw("<code>").text(id).raw("</code>"),
            None => html.raw("<span class=\"none\">None was assigned.</span>"),
        };
        html.raw("</td></tr>\n</table>\n<h3>Query</h3>\n");

        let query: Vec<(String, String)> = self
            .query
            .as_deref()
            .map(|query| {
                url::form_urlencoded::parse(query.as_bytes())
                    .map(|(name, value)| (name.into_owned(), value.into_owned()))
                    .collect()
            })
            .unwrap_or_default();
        if query.is_empty() {
            html.raw("<p class=\"none\">No query parameters.</p>\n");
        } else {
            html.raw("<table>\n");
            for (name, value) in &query {
                let secret = names_a_secret(name);
                render_pair(html, name, value, secret);
            }
            html.raw("</table>\n");
        }

        html.raw("<h3>Headers</h3>\n<table>\n");
        for (name, value) in &self.headers {
            let secret = header_value_is_secret(name);
            render_pair(html, name, value, secret);
        }
        html.raw("</table>\n<p class=\"note\">The values of credential and cookie headers, and of every header and parameter whose name contains token, secret, password, key or signature, are redacted, wherever the parameter appears; so are URL passwords. This page never shows the request body, environment variables or configuration values.</p>\n");
    }
}

/// One `name: value` row, the value redacted when `secret`.
fn render_pair(html: &mut Html, name: &str, value: &str, secret: bool) {
    html.raw("<tr><th>").text(name).raw("</th><td>");
    if secret {
        html.raw("<span class=\"none\">")
            .text(REDACTED)
            .raw("</span>");
    } else {
        html.raw("<code>").text(&redact_text(value)).raw("</code>");
    }
    html.raw("</td></tr>\n");
}

/// The error chain, one link per line, or the panic and its location.
fn render_failure(html: &mut Html, report: &ErrorReport) {
    if report.is_panic() {
        html.raw("<h2>Panic</h2>\n<p>");
        match report.panic_location() {
            Some(location) => html
                .raw("The request panicked at <code>")
                .text(location)
                .raw("</code>:</p>\n"),
            None => html.raw("The request panicked. Its location was not recorded:</p>\n"),
        };
        html.raw("<pre>")
            .text(&redact_text(
                report.chain().first().map_or("", String::as_str),
            ))
            .raw("</pre>\n");
        return;
    }
    html.raw("<h2>Error</h2>\n<ol class=\"chain\">\n");
    for (index, link) in report.chain().iter().enumerate() {
        html.raw("<li>");
        if index > 0 {
            html.raw("<span class=\"cause\">caused by</span> ");
        }
        html.raw("<code>")
            .text(&redact_text(link))
            .raw("</code></li>\n");
    }
    html.raw("</ol>\n");
}

/// The recorded frames: the application's shown, every run of the
/// others collapsed into a `<details>` whose summary counts them.
fn render_frames(html: &mut Html, report: &ErrorReport) {
    html.raw("<h2>Stack frames</h2>\n");
    let Some(recorded) = report.frames() else {
        html.raw("<p class=\"note\">No stack frames were recorded for this error. Suprnova records them on the request's own task, where a FrameworkError or AppError constructor or conversion creates the error, or where a panic is raised. This error was created somewhere else: on another task, before the request began, or as a struct literal.</p>\n");
        return;
    };
    if report.is_panic() {
        html.raw("<p>Recorded where the panic was raised, at <code>");
    } else {
        html.raw("<p>Recorded where the error was created, at <code>");
    }
    html.text(recorded.site()).raw("</code>.</p>\n");
    let resolved = recorded.resolve();
    if resolved.frames.is_empty() {
        html.raw("<p class=\"note\">This platform could not capture the stack.</p>\n");
        return;
    }
    html.raw("<p class=\"note\">The innermost frame is first. The application's frames are shown; the frames of the framework, the async runtime, other dependencies and the standard library are collapsed.</p>\n<ol class=\"frames\">\n");
    let mut rest = resolved.frames.as_slice();
    while let Some(first) = rest.first() {
        if first.origin == Origin::App {
            html.raw("<li class=\"app\">");
            render_frame(html, first);
            html.raw("</li>\n");
            rest = &rest[1..];
            continue;
        }
        let run = rest
            .iter()
            .position(|frame| frame.origin == Origin::App)
            .unwrap_or(rest.len());
        let (collapsed, after) = rest.split_at(run);
        html.raw("<li><details>\n<summary>")
            .text(&collapsed_summary(collapsed))
            .raw("</summary>\n<ol>\n");
        for frame in collapsed {
            html.raw("<li>");
            render_frame(html, frame);
            html.raw("</li>\n");
        }
        html.raw("</ol>\n</details></li>\n");
        rest = after;
    }
    html.raw("</ol>\n");
    if resolved.omitted > 0 {
        html.raw("<p class=\"note\">")
            .text(&resolved.omitted.to_string())
            .raw(" more frames are not listed.</p>\n");
    }
}

fn render_frame(html: &mut Html, frame: &Frame) {
    html.raw("<code class=\"fn\">")
        .text(&frame.function)
        .raw("</code>");
    if frame.origin != Origin::App {
        html.raw(" <span class=\"origin\">")
            .text(frame.origin.label())
            .raw("</span>");
    }
    if let Some(location) = &frame.location {
        html.raw("<span class=\"at\">")
            .text(location)
            .raw("</span>");
    }
}

/// `"14 frames: framework 6, async runtime 3, standard library 5"`.
fn collapsed_summary(frames: &[Frame]) -> String {
    let counts: Vec<String> = [
        Origin::Framework,
        Origin::Runtime,
        Origin::Dependency,
        Origin::Std,
    ]
    .into_iter()
    .filter_map(|origin| {
        let count = frames.iter().filter(|frame| frame.origin == origin).count();
        (count > 0).then(|| format!("{} {count}", origin.label()))
    })
    .collect();
    let noun = if frames.len() == 1 { "frame" } else { "frames" };
    format!("{} {noun}: {}", frames.len(), counts.join(", "))
}

/// An HTML document under construction.
///
/// Markup only comes in as `&'static str`, through [`Self::raw`]; every
/// value the page shows goes through [`Self::text`], which escapes it.
/// So no value from the request or the error can become markup.
#[derive(Default)]
struct Html(String);

impl Html {
    /// Append fixed markup.
    fn raw(&mut self, markup: &'static str) -> &mut Self {
        self.0.push_str(markup);
        self
    }

    /// Append `value` as text, escaped.
    fn text(&mut self, value: &str) -> &mut Self {
        for ch in value.chars() {
            match ch {
                '&' => self.0.push_str("&amp;"),
                '<' => self.0.push_str("&lt;"),
                '>' => self.0.push_str("&gt;"),
                '"' => self.0.push_str("&quot;"),
                '\'' => self.0.push_str("&#39;"),
                _ => self.0.push(ch),
            }
        }
        self
    }
}

/// Whether a header's value is redacted on the page.
fn header_value_is_secret(name: &str) -> bool {
    CREDENTIAL_HEADERS
        .iter()
        .any(|credential| name.eq_ignore_ascii_case(credential))
        || names_a_secret(name)
}

/// Whether `name` contains `token`, `secret`, `password`, `key` or
/// `signature`, in any letter case.
fn names_a_secret(name: &str) -> bool {
    let name = name.to_lowercase();
    SECRET_NAME_PARTS.iter().any(|part| name.contains(part))
}

/// `text` with every secret the page must not show replaced: the value of
/// each parameter whose name names a secret
/// ([`redact_secret_parameters`]), and the password of each URL
/// ([`redact_url_passwords`]).
///
/// Every value the page shows from the request or the error goes through
/// this, not only the query: a `Referer`, an `X-Original-URI` or an
/// error message can carry the same URL.
fn redact_text(text: &str) -> Cow<'_, str> {
    match redact_secret_parameters(text) {
        Cow::Borrowed(text) => redact_url_passwords(text),
        Cow::Owned(text) => Cow::Owned(redact_url_passwords(&text).into_owned()),
    }
}

/// Whether `c` ends a parameter's name, looking back from its `=`.
fn ends_parameter_name(c: char) -> bool {
    c.is_ascii_whitespace()
        || matches!(
            c,
            '?' | '&' | ';' | ',' | '#' | '/' | '"' | '\'' | '<' | '>' | '`'
        )
}

/// Whether `c` ends a parameter's value. A value may hold `=`, `,`, `/`
/// and `?`, so none of them ends it.
fn ends_parameter_value(c: char) -> bool {
    c.is_ascii_whitespace() || matches!(c, '&' | ';' | '#' | '"' | '\'' | '<' | '>' | '`')
}

/// `text` with the value of every `name=value` parameter whose name
/// contains `token`, `secret`, `password`, `key` or `signature` replaced.
///
/// It reads parameters wherever they stand: in a URL's query, a bare
/// query string, matrix parameters after `;`, a fragment such as
/// `#access_token=...`, or a `key=value` list. Names are
/// percent-decoded before they are checked, so `api%5Fkey` counts. A
/// parameter that only looks like one is redacted too: hiding a value
/// that was not secret costs less than showing one that was.
fn redact_secret_parameters(text: &str) -> Cow<'_, str> {
    if !text.contains('=') {
        return Cow::Borrowed(text);
    }
    let mut redacted = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(equals) = rest.find('=') {
        let (before, from_equals) = rest.split_at(equals);
        let value_and_rest = &from_equals[1..];
        // Every delimiter is one ASCII byte, so `at + 1` is a boundary.
        let name_start = before.rfind(ends_parameter_name).map_or(0, |at| at + 1);
        let name = decoded_parameter_name(&before[name_start..]);
        let value_end = value_and_rest
            .find(ends_parameter_value)
            .unwrap_or(value_and_rest.len());
        redacted.push_str(before);
        redacted.push('=');
        if value_end > 0 && names_a_secret(&name) {
            redacted.push_str(REDACTED);
        } else {
            redacted.push_str(&value_and_rest[..value_end]);
        }
        rest = &value_and_rest[value_end..];
    }
    redacted.push_str(rest);
    Cow::Owned(redacted)
}

/// A parameter name, percent-decoded the way a query string is.
fn decoded_parameter_name(raw: &str) -> String {
    url::form_urlencoded::parse(raw.as_bytes())
        .next()
        .map(|(name, _)| name.into_owned())
        .unwrap_or_default()
}

/// `text` with the password of every `scheme://user:password@host` URL
/// replaced, and the rest of the URL kept readable.
///
/// A URL runs from `://` to the next whitespace or quote. Its user info
/// ends at the last `@` in it and the password starts after the first `:`
/// in the user info, so a password holding an unencoded `/`, `?` or `#`,
/// as generated secrets often do, is still found. A URL whose path holds
/// both `:` and `@` loses more than its password; that over-redaction is
/// the safe side.
fn redact_url_passwords(text: &str) -> Cow<'_, str> {
    if !text.contains("://") {
        return Cow::Borrowed(text);
    }
    let mut redacted = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("://") {
        let (head, tail) = rest.split_at(at + 3);
        redacted.push_str(head);
        let end = tail
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '`'))
            .unwrap_or(tail.len());
        let url = &tail[..end];
        let credentials = url
            .rfind('@')
            .and_then(|at_sign| url[..at_sign].find(':').map(|colon| (colon, at_sign)));
        match credentials {
            Some((colon, at_sign)) => {
                redacted.push_str(&url[..=colon]);
                redacted.push_str(REDACTED);
                redacted.push_str(&url[at_sign..]);
            }
            None => redacted.push_str(url),
        }
        rest = &tail[end..];
    }
    redacted.push_str(rest);
    Cow::Owned(redacted)
}

/// Whether one of the `Accept` values lists `text/html`.
///
/// PAR-012's rule is plainer than content negotiation: listing HTML is
/// enough, whatever else the client accepts. A range with `q=0` says the
/// client does not accept HTML (RFC 9110), so it does not count.
fn accept_lists_html<'a>(values: impl Iterator<Item = &'a str>) -> bool {
    values.flat_map(|value| value.split(',')).any(|range| {
        let mut parts = range.split(';');
        let media_type = parts.next().unwrap_or("").trim();
        media_type.eq_ignore_ascii_case("text/html")
            && !parts.any(|parameter| {
                parameter.split_once('=').is_some_and(|(name, value)| {
                    name.trim().eq_ignore_ascii_case("q")
                        && value.trim().parse::<f32>().is_ok_and(|q| q == 0.0)
                })
            })
    })
}

/// Whether a header of the replaced response stays on the page.
///
/// Dropped: every `Content-*` field, which described the body the page
/// replaces (the app's own `Content-Security-Policy` among them: the page
/// sets its own); `Transfer-Encoding`; the fields that said how the old
/// body could be stored or revalidated; and every `X-Inertia*` field,
/// since the page is not an Inertia response.
fn header_survives_page(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    !(name.starts_with("content-")
        || name.starts_with("x-inertia")
        || matches!(
            name.as_str(),
            "transfer-encoding" | "cache-control" | "expires" | "age" | "etag" | "last-modified"
        ))
}

/// The page's style sheet: plain, compact, light and dark.
const STYLE: &str = "
:root{color-scheme:light dark;--bg:#f6f8fa;--panel:#ffffff;--fg:#1f2328;--muted:#59636e;--line:#d1d9e0;--accent:#b42318}
@media (prefers-color-scheme:dark){:root{--bg:#0d1117;--panel:#161b22;--fg:#e6edf3;--muted:#9198a1;--line:#30363d;--accent:#ff7b72}}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--fg);font:14px/1.5 system-ui,-apple-system,'Segoe UI',sans-serif}
main{max-width:1100px;margin:0 auto;padding:24px 16px 48px}
header{border-left:4px solid var(--accent);padding:2px 0 2px 14px}
h1{margin:4px 0 8px;font-size:20px;line-height:1.35;overflow-wrap:anywhere}
h2{margin:28px 0 8px;padding-bottom:4px;font-size:15px;border-bottom:1px solid var(--line)}
h3{margin:16px 0 6px;font-size:13px;color:var(--muted)}
p{margin:0 0 8px}
code,pre{font:12.5px/1.5 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;overflow-wrap:anywhere}
pre{margin:0;padding:10px 12px;white-space:pre-wrap;background:var(--panel);border:1px solid var(--line);border-radius:6px}
.status{margin:0;font-weight:600;color:var(--accent)}
.note,.none,.cause,.origin,.at,summary{color:var(--muted)}
.note{font-size:13px}
ol.chain{margin:0;padding-left:22px}
ol.chain li{margin:2px 0}
ol.frames{margin:8px 0;padding:0;list-style:none;background:var(--panel);border:1px solid var(--line);border-radius:6px}
ol.frames>li{padding:6px 12px;border-top:1px solid var(--line)}
ol.frames>li:first-child{border-top:0}
ol.frames .app .fn{font-weight:600}
details ol{margin:6px 0 2px;padding:0 0 0 12px;list-style:none;border-left:2px solid var(--line)}
details li{padding:2px 0}
summary{cursor:pointer}
.origin{font-size:11px}
.at{display:block;font-size:12px;overflow-wrap:anywhere}
table{width:100%;margin-bottom:8px;border-collapse:collapse;background:var(--panel);border:1px solid var(--line)}
th,td{padding:4px 10px;text-align:left;vertical-align:top;border-top:1px solid var(--line);overflow-wrap:anywhere}
th{width:30%;font-weight:600;color:var(--muted)}
";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::FrameworkError;

    fn request(target: &str, headers: &[(&str, &str)]) -> DebugRequest {
        let mut builder = hyper::Request::builder().method("GET").uri(target);
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let request = builder.body(()).expect("a well-formed test request");
        DebugRequest::capture(&request)
    }

    fn response(status: u16, report: Option<ErrorReport>) -> hyper::Response<Body> {
        let mut response = hyper::Response::builder()
            .status(status)
            .header("Content-Type", "application/json")
            .header("Content-Length", "34")
            .header("Content-Security-Policy", "script-src 'self'")
            .header("Cache-Control", "public, max-age=60")
            .header("X-Inertia", "true")
            .header("Set-Cookie", "session=abc")
            .header("X-Request-Id", "rid-1")
            .body(
                Full::new(Bytes::from_static(
                    br#"{"message":"Internal Server Error"}"#,
                ))
                .map_err(|never| match never {})
                .boxed(),
            )
            .expect("a well-formed test response");
        if let Some(report) = report {
            response.extensions_mut().insert(report);
        }
        response
    }

    fn report() -> ErrorReport {
        ErrorReport::from_error(&FrameworkError::internal("the ledger closed"))
    }

    async fn body_of(response: hyper::Response<Body>) -> String {
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("an infallible body")
            .to_bytes();
        String::from_utf8(bytes.to_vec()).expect("the page is UTF-8")
    }

    #[tokio::test]
    async fn a_reported_5xx_for_a_browser_becomes_the_page_with_its_own_headers() {
        let browser = request("/ledger", &[("Accept", "text/html")]);

        let page = browser.replace(response(503, Some(report())));

        assert_eq!(page.status(), 503);
        assert!(
            page.extensions().get::<ErrorReport>().is_some(),
            "the page keeps the report"
        );
        let headers = page.headers();
        assert_eq!(headers["content-type"], "text/html; charset=utf-8");
        assert_eq!(headers["cache-control"], "no-store");
        assert_eq!(headers["content-security-policy"], CONTENT_SECURITY_POLICY);
        assert_eq!(headers.get_all("content-security-policy").iter().count(), 1);
        assert!(headers.get("content-length").is_none());
        assert!(headers.get("x-inertia").is_none());
        assert_eq!(headers["set-cookie"], "session=abc");
        assert_eq!(headers["x-request-id"], "rid-1");
        let body = body_of(page).await;
        assert!(body.starts_with("<!doctype html>"), "{body}");
        assert!(body.contains("the ledger closed"), "{body}");
        assert!(body.contains("rid-1"), "the response's request id: {body}");
    }

    #[test]
    fn everything_else_is_left_as_it_is() {
        let browser = request("/ledger", &[("Accept", "text/html")]);
        let api = request("/ledger", &[("Accept", "application/json")]);

        let cases = [
            ("a 4xx", browser.replace(response(404, Some(report())))),
            ("a 5xx with no report", browser.replace(response(500, None))),
            ("a JSON client", api.replace(response(500, Some(report())))),
        ];
        for (case, kept) in cases {
            assert_eq!(
                kept.headers()["content-type"],
                "application/json",
                "{case} must be left as it is"
            );
        }
    }

    #[test]
    fn an_inertia_visit_gets_the_page_without_listing_html() {
        let visit = request("/ledger", &[("X-Inertia", "true")]);

        let page = visit.replace(response(500, Some(report())));

        assert_eq!(page.headers()["content-type"], "text/html; charset=utf-8");
    }

    #[test]
    fn accept_must_list_text_html_with_a_nonzero_quality() {
        let lists = |accept: &str| accept_lists_html(std::iter::once(accept));

        assert!(lists("text/html"));
        assert!(lists("application/json, TEXT/HTML;q=0.1"));
        assert!(lists(
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"
        ));
        assert!(!lists("*/*"));
        assert!(!lists("text/*"));
        assert!(!lists("application/json"));
        assert!(!lists("text/html;q=0"));
        assert!(!lists("text/htmlx"));
        assert!(!accept_lists_html(std::iter::empty()));
    }

    #[test]
    fn credential_and_secret_named_values_are_redacted() {
        for name in [
            "Authorization",
            "proxy-authorization",
            "COOKIE",
            "set-cookie",
            "x-api-key",
            "X-CSRF-TOKEN",
            "x-client-secret",
            "X-Password-Reset",
            "x-webhook-signature",
        ] {
            assert!(header_value_is_secret(name), "{name}");
        }
        for name in ["accept", "x-request-id", "user-agent", "keep-alive"] {
            assert!(!header_value_is_secret(name), "{name}");
        }
        for name in [
            "token",
            "api_KEY",
            "Signature",
            "client_secret",
            "PassWord",
            "user[password]",
        ] {
            assert!(names_a_secret(name), "{name}");
        }
        assert!(!names_a_secret("view"));
    }

    #[test]
    fn a_url_password_is_redacted_and_the_rest_kept() {
        let cases = [
            (
                "connecting to postgres://app:hunter2@db/app failed",
                "connecting to postgres://app:[redacted]@db/app failed",
            ),
            (
                "redis://:s3cret@cache:6379/0 and mysql://root:p@ss:w@db?x=1",
                "redis://:[redacted]@cache:6379/0 and mysql://root:[redacted]@db?x=1",
            ),
            (
                "https://user@example.com/a and https://example.com:8443/b",
                "https://user@example.com/a and https://example.com:8443/b",
            ),
            (
                "postgres://app:pa/ss@db/app and redis://:p?ss@cache:6379 and amqp://u:p#ss@mq/vhost",
                "postgres://app:[redacted]@db/app and redis://:[redacted]@cache:6379 and amqp://u:[redacted]@mq/vhost",
            ),
            ("no url here", "no url here"),
            ("dangling ://", "dangling ://"),
        ];
        for (text, expected) in cases {
            assert_eq!(redact_url_passwords(text), expected);
        }
    }

    #[test]
    fn every_value_is_escaped() {
        let mut html = Html::default();
        html.raw("<p>")
            .text("<script>alert('x') & \"y\"</script>")
            .raw("</p>");

        assert_eq!(
            html.0,
            "<p>&lt;script&gt;alert(&#39;x&#39;) &amp; &quot;y&quot;&lt;/script&gt;</p>"
        );
    }

    #[test]
    fn the_page_shows_the_request_and_redacts_its_secrets() {
        let captured = request(
            "/ledger/7?view=ledger-overview&api_token=s3cr3t",
            &[
                ("Accept", "text/html"),
                ("Authorization", "Bearer s3cr3t"),
                ("X-Trace", "<b>trace</b>"),
            ],
        );
        captured.notes.route_pattern.set("/ledger/{id}".into()).ok();

        let page = captured.render(StatusCode::INTERNAL_SERVER_ERROR, &report(), Some("rid-9"));

        for shown in [
            "GET",
            "/ledger/7",
            "/ledger/{id}",
            "rid-9",
            "ledger-overview",
            "x-trace",
            "&lt;b&gt;trace&lt;/b&gt;",
        ] {
            assert!(
                page.contains(shown),
                "the page must show {shown:?}:\n{page}"
            );
        }
        assert!(!page.contains("s3cr3t"), "{page}");
        assert!(!page.contains("<b>"), "{page}");
        assert!(!page.to_ascii_lowercase().contains("<script"), "{page}");
    }

    #[test]
    fn secret_parameters_in_any_header_value_are_redacted() {
        let captured = request(
            "/ledger",
            &[
                ("Accept", "text/html"),
                (
                    "Referer",
                    "https://app.test/reset?token=s3cr3t-1&view=summary&Api%5FKey=s3cr3t-2",
                ),
                ("X-Original-URI", "/reset?client_secret=s3cr3t-3&page=7"),
                (
                    "X-Rewrite-URL",
                    "/a;password=s3cr3t-4#access_token=s3cr3t-5",
                ),
                ("X-Forwarded-Query", "SIGNATURE=s3cr3t-6&sort=asc"),
                ("Forwarded", "for=192.0.2.60;proto=https;by=203.0.113.43"),
            ],
        );

        let page = captured.render(StatusCode::INTERNAL_SERVER_ERROR, &report(), None);

        assert!(!page.contains("s3cr3t"), "{page}");
        for shown in [
            "https://app.test/reset?token=",
            "view=summary",
            "page=7",
            "/a;password=",
            "sort=asc",
            "for=192.0.2.60;proto=https;by=203.0.113.43",
        ] {
            assert!(
                page.contains(shown),
                "the page must show {shown:?}:\n{page}"
            );
        }
    }

    #[test]
    fn a_report_without_frames_says_so() {
        let captured = request("/ledger", &[("Accept", "text/html")]);

        let page = captured.render(StatusCode::INTERNAL_SERVER_ERROR, &report(), None);

        assert!(page.contains("No stack frames were recorded"), "{page}");
        assert!(page.contains("None was assigned."), "{page}");
        assert!(page.contains("No route matched."), "{page}");
    }

    #[test]
    fn a_collapsed_run_states_its_count_by_origin() {
        let frame = |origin| Frame {
            function: "f".into(),
            location: None,
            origin,
        };

        assert_eq!(
            collapsed_summary(&[
                frame(Origin::Std),
                frame(Origin::Framework),
                frame(Origin::Std)
            ]),
            "3 frames: framework 1, standard library 2"
        );
        assert_eq!(
            collapsed_summary(&[frame(Origin::Runtime)]),
            "1 frame: async runtime 1"
        );
    }

    #[test]
    fn only_headers_about_the_request_or_the_next_step_survive() {
        for kept in [
            "set-cookie",
            "x-request-id",
            "vary",
            "retry-after",
            "www-authenticate",
        ] {
            assert!(header_survives_page(kept), "{kept}");
        }
        for dropped in [
            "content-type",
            "Content-Length",
            "content-encoding",
            "content-security-policy",
            "content-security-policy-report-only",
            "transfer-encoding",
            "cache-control",
            "etag",
            "x-inertia",
            "x-inertia-location",
        ] {
            assert!(!header_survives_page(dropped), "{dropped}");
        }
    }
}
