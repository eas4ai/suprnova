//! `TestResponse` - a fluent wrapper around the response a request
//! driven through [`crate::handle_request`] returns (see
//! `manual/http-tests.md`), or around a `(status, headers, body)` triple
//! a harness already captured. Laravel's
//! `Illuminate\Testing\TestResponse` equivalent: assertions read the
//! same way and panic with an expected/actual excerpt on failure -
//! this is a *testing* surface, so panicking is the contract here, the
//! same way it is for [`crate::testing::Expect`]. Every assertion
//! returns `&Self`, so they chain.
//!
//! Built with [`TestResponse::from_response`], it also keeps the
//! response's [`ErrorReport`], and every failing assertion prints it.

use std::sync::Arc;

use bytes::Bytes;
use http_body_util::BodyExt;

use crate::{Cookie, CookiePrefix, ErrorReport, SessionStore, is_valid_session_id};

/// A captured HTTP response, wrapped for fluent assertions. Build one
/// with [`Self::from_response`] from what [`crate::handle_request`]
/// returns, or with [`Self::new`] from whatever a test harness already
/// produced.
pub struct TestResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Bytes,
    session: Option<(Arc<dyn SessionStore>, String)>,
    report: Option<ErrorReport>,
}

impl TestResponse {
    /// Build a `TestResponse` from a status code, response headers, and
    /// the collected body. `headers` accepts anything iterable as
    /// `(String, String)` pairs - a `HashMap<String, String>`, a
    /// `Vec<(String, String)>`, or `hyper::HeaderMap::iter()` mapped to
    /// owned strings - so no existing harness has to change how it
    /// drives a request. Header names are normalized to lowercase for
    /// case-insensitive lookup; multiple values for the same name (two
    /// `Set-Cookie` headers, most commonly) are preserved, not
    /// collapsed.
    pub fn new(
        status: u16,
        headers: impl IntoIterator<Item = (String, String)>,
        body: impl Into<Bytes>,
    ) -> Self {
        Self {
            status,
            headers: headers
                .into_iter()
                .map(|(name, value)| (name.to_lowercase(), value))
                .collect(),
            body: body.into(),
            session: None,
            report: None,
        }
    }

    /// Build a `TestResponse` from the response [`crate::handle_request`]
    /// returns, collecting its body.
    ///
    /// Unlike [`Self::new`], this keeps the response's [`ErrorReport`],
    /// which lives in the response's in-process extensions and so never
    /// survives a trip over a socket. When the framework built the
    /// response from an error or a panic, every failing assertion then
    /// prints what went wrong, not only the sanitized body the client
    /// sees.
    ///
    /// # Panics
    ///
    /// Panics if the body fails while it is collected. The framework's
    /// own bodies cannot fail; a test-supplied body can.
    pub async fn from_response<B>(response: hyper::Response<B>) -> Self
    where
        B: hyper::body::Body,
        B::Error: std::fmt::Display,
    {
        let (mut parts, body) = response.into_parts();
        let report = parts.extensions.remove::<ErrorReport>();
        let body = match body.collect().await {
            Ok(collected) => collected.to_bytes(),
            Err(e) => panic!("TestResponse::from_response(): collecting the body failed: {e}"),
        };
        let headers = parts.headers.iter().map(|(name, value)| {
            (
                name.as_str().to_string(),
                String::from_utf8_lossy(value.as_bytes()).into_owned(),
            )
        });
        let mut response = Self::new(parts.status.as_u16(), headers, body);
        response.report = report;
        response
    }

    /// The error report the response carries: the error and its source
    /// chain, or a caught panic's message and location. `None` when the
    /// response was not built from an error, and always `None` for a
    /// response built with [`Self::new`].
    pub fn error_report(&self) -> Option<&ErrorReport> {
        self.report.as_ref()
    }

    /// Fail an assertion with `message`, followed by the error report
    /// when the response carries one: the sanitized body alone rarely
    /// says why a request failed.
    fn fail(&self, message: String) -> ! {
        fail_with_report(message, self.report.as_ref())
    }

    /// Attach the session store [`Self::assert_session_has`] reads from, and
    /// the cookie name it names the session with. Pass the same
    /// `Arc<dyn SessionStore>` the test's `SessionMiddleware` was built
    /// with (`SessionMiddleware::store()`) and the
    /// `SessionConfig::cookie_name` it used (`"suprnova_session"` unless
    /// overridden). The string is the wire name (the `Set-Cookie` name); any
    /// `__Host-`/`__Secure-` prefix is stripped only for AEAD lookup.
    pub fn with_session_store(
        mut self,
        store: Arc<dyn SessionStore>,
        cookie_name: impl Into<String>,
    ) -> Self {
        self.session = Some((store, cookie_name.into()));
        self
    }

    /// The numeric status code.
    pub fn status(&self) -> u16 {
        self.status
    }

    /// The first header value matching `name`, case-insensitive.
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.as_str())
    }

    fn headers_named(&self, name: &str) -> Vec<&str> {
        let name = name.to_lowercase();
        self.headers
            .iter()
            .filter(|(k, _)| *k == name)
            .map(|(_, v)| v.as_str())
            .collect()
    }

    /// The value of the first `Set-Cookie` header naming `name`, if
    /// any. Percent-decoded, via the same [`crate::http::parse_cookies`]
    /// every inbound `Cookie` header goes through.
    pub fn cookie(&self, name: &str) -> Option<String> {
        self.headers_named("set-cookie")
            .into_iter()
            .find_map(|raw| crate::http::parse_cookies(raw).remove(name))
    }

    /// The response body decoded as UTF-8 (lossily - invalid sequences
    /// become U+FFFD).
    pub fn body_text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// Parse the body as JSON.
    ///
    /// # Panics
    ///
    /// Panics with the raw body attached if it isn't valid JSON - this
    /// is a test-assertion helper, so an invalid body IS the failure.
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|e| {
            self.fail(format!(
                "TestResponse::json(): body is not valid JSON: {e}\n  body: {}",
                self.body_text()
            ))
        })
    }

    /// Assert the exact status code.
    pub fn assert_status(&self, expected: u16) -> &Self {
        if self.status != expected {
            self.fail(format!(
                "assert_status({expected})\n  Expected: {expected}\n  Received: {}\n  body: {}",
                self.status,
                self.body_text()
            ));
        }
        self
    }

    /// `assert_status(200)`.
    pub fn assert_ok(&self) -> &Self {
        if self.status != 200 {
            self.fail(format!(
                "assert_ok()\n  Expected: 200\n  Received: {}\n  body: {}",
                self.status,
                self.body_text()
            ));
        }
        self
    }

    /// Assert the response is a redirect: a 3xx status carrying a
    /// `Location` header. When `target` is `Some`, also asserts
    /// `Location` equals it exactly.
    pub fn assert_redirect(&self, target: Option<&str>) -> &Self {
        let location = self.header("location");
        if !(300..400).contains(&self.status) || location.is_none() {
            self.fail(format!(
                "assert_redirect({target:?})\n  Expected: a 3xx status with a Location header\n  \
                 Received: status {}, location {location:?}\n  body: {}",
                self.status,
                self.body_text()
            ));
        }
        if let Some(expected) = target
            && location != Some(expected)
        {
            self.fail(format!(
                "assert_redirect(Some({expected:?}))\n  Expected Location: {expected:?}\n  \
                 Received Location: {location:?}"
            ));
        }
        self
    }

    /// Assert the JSON body is a superset of `expected`: every key in
    /// `expected` - recursively, through nested objects - is present in
    /// the body with an equal value. Extra keys in the body are
    /// ignored. Arrays compare element-by-element and must match in
    /// length; they are not treated as unordered sets.
    pub fn assert_json(&self, expected: serde_json::Value) -> &Self {
        let actual = self.json();
        if let Some(path) = json_subset_mismatch("$", &expected, &actual) {
            self.fail(format!(
                "assert_json(...)\n  mismatch at `{path}`\n  Expected (subset): {expected}\n  \
                 Received: {actual}"
            ));
        }
        self
    }

    /// Assert the value at a dot-separated `path` into the JSON body
    /// equals `expected`. A numeric segment indexes into a JSON array
    /// (`"items.0.id"`); every other segment looks up an object key.
    pub fn assert_json_path(&self, path: &str, expected: impl Into<serde_json::Value>) -> &Self {
        let root = self.json();
        let expected = expected.into();
        let found = json_path(&root, path);
        if found != Some(&expected) {
            self.fail(format!(
                "assert_json_path({path:?}, ...)\n  Expected: {expected}\n  Received: {}",
                found
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "<missing>".to_string())
            ));
        }
        self
    }

    /// Assert the length of a JSON array. `path` names the array with
    /// [`Self::assert_json_path`] dot notation; `None` means the body
    /// itself must be the array.
    pub fn assert_json_count(&self, path: Option<&str>, expected: usize) -> &Self {
        let root = self.json();
        let target = match path {
            Some(p) => json_path(&root, p).cloned(),
            None => Some(root.clone()),
        };
        let actual_len = match &target {
            Some(serde_json::Value::Array(items)) => Some(items.len()),
            _ => None,
        };
        if actual_len != Some(expected) {
            self.fail(format!(
                "assert_json_count({path:?}, {expected})\n  Expected: an array of length \
                 {expected}\n  Received: {}",
                target
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "<missing or not an array>".to_string())
            ));
        }
        self
    }

    /// Assert the body, decoded as UTF-8, contains `needle`.
    pub fn assert_see(&self, needle: &str) -> &Self {
        let body = self.body_text();
        if !body.contains(needle) {
            self.fail(format!(
                "assert_see({needle:?})\n  body did not contain the needle\n  body: {body}"
            ));
        }
        self
    }

    /// Assert a header's value matches exactly (case-insensitive name).
    pub fn assert_header(&self, name: &str, expected: &str) -> &Self {
        let actual = self.header(name);
        if actual != Some(expected) {
            self.fail(format!(
                "assert_header({name:?}, {expected:?})\n  Expected: {expected:?}\n  \
                 Received: {actual:?}"
            ));
        }
        self
    }

    /// Assert a cookie named `name` was set (any `Set-Cookie` header).
    pub fn assert_cookie(&self, name: &str) -> &Self {
        if self.cookie(name).is_none() {
            self.fail(format!(
                "assert_cookie({name:?})\n  no Set-Cookie header named {name:?}\n  Set-Cookie \
                 headers: {:?}",
                self.headers_named("set-cookie")
            ));
        }
        self
    }

    /// Assert the session named by this response's session cookie has
    /// `key` set to `expected`.
    ///
    /// Requires [`Self::with_session_store`] first. There is no honest
    /// way to read server-side session state from a wire-level response
    /// alone - the session lives in the store, keyed by the id inside
    /// the (encrypted) session cookie, not in the response body. This
    /// decrypts the cookie with the same [`crate::CryptPurpose::Cookie`]
    /// purpose [`crate::SessionMiddleware`] writes it under, extracts
    /// the session id, and reads that row from the attached store - the
    /// same lookup the middleware itself performs on the next request.
    ///
    /// # Panics
    ///
    /// Panics if no store was attached, no session cookie is present,
    /// the cookie fails to decrypt, the store has no row for the
    /// decrypted id, or `key` isn't set to `expected`.
    pub async fn assert_session_has(
        &self,
        key: &str,
        expected: impl Into<serde_json::Value>,
    ) -> &Self {
        let Some((store, cookie_name)) = self.session.as_ref() else {
            self.fail(format!(
                "assert_session_has({key:?}, ...) called without a session store - call \
                 .with_session_store(store, cookie_name) first"
            ));
        };
        let Some(raw) = self.cookie(cookie_name) else {
            self.fail(format!(
                "assert_session_has({key:?}, ...): no {cookie_name:?} cookie in the response"
            ));
        };
        let plaintext = Cookie::read_encrypted_for(CookiePrefix::strip(cookie_name), &raw)
            .unwrap_or_else(|e| {
                self.fail(format!(
                    "assert_session_has({key:?}, ...): session cookie failed to decrypt: {e}"
                ))
            });
        let Some(session_id) = plaintext
            .split('.')
            .next()
            .filter(|id| is_valid_session_id(id))
        else {
            self.fail(format!(
                "assert_session_has({key:?}, ...): decrypted cookie payload is not a valid \
                 session id: {plaintext:?}"
            ));
        };
        let stored = store.read(session_id).await.unwrap_or_else(|e| {
            self.fail(format!(
                "assert_session_has({key:?}, ...): store read failed: {e}"
            ))
        });
        let Some(session_data) = stored else {
            self.fail(format!(
                "assert_session_has({key:?}, ...): no session row for id {session_id}"
            ));
        };
        let expected = expected.into();
        let actual = session_data.data.get(key);
        if actual != Some(&expected) {
            self.fail(format!(
                "assert_session_has({key:?}, ...)\n  Expected: {expected}\n  Received: {}\n  \
                 session id: {session_id}",
                actual
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "<missing key>".to_string())
            ));
        }
        self
    }

    /// Build an [`AssertableInertia`](crate::testing::AssertableInertia)
    /// from this response's JSON body and assert it as an Inertia page
    /// object.
    ///
    /// Requires the response to actually be an Inertia visit response -
    /// the request that produced it must have sent `X-Inertia: true`, or
    /// there is no page object to parse (a hard navigation returns the
    /// HTML shell instead; use
    /// [`crate::testing::AssertableInertia::from_response`] directly on
    /// the `HttpResponse` for that case, which handles both shapes).
    ///
    /// # Panics
    ///
    /// Panics if the response has no `X-Inertia` header, or if the body
    /// isn't a valid Inertia page object.
    pub fn assert_inertia(&self) -> crate::testing::AssertableInertia {
        if self.header("x-inertia") != Some("true") {
            self.fail(format!(
                "assert_inertia(): expected an X-Inertia response (X-Inertia: true header), \
                 got X-Inertia = {:?}. A hard navigation returns the HTML shell instead of a \
                 page object - send `X-Inertia: true` with the request, or use \
                 AssertableInertia::from_response(&http_response) directly.",
                self.header("x-inertia")
            ));
        }
        crate::testing::AssertableInertia::from_page(self.json(), self.report.clone())
    }
}

/// Panic with an assertion's `message`, followed by `report` when there
/// is one. [`TestResponse`] and
/// [`AssertableInertia`](crate::testing::AssertableInertia) share it, so a
/// failure on either reads the same way.
pub(crate) fn fail_with_report(message: String, report: Option<&ErrorReport>) -> ! {
    match report {
        Some(report) => {
            let report = report.to_string().replace('\n', "\n    ");
            panic!("{message}\n  error report:\n    {report}")
        }
        None => panic!("{message}"),
    }
}

/// Returns `Some(path)` naming the first point of mismatch between
/// `expected` and `actual`, or `None` if `actual` is a superset of
/// `expected` - see [`TestResponse::assert_json`].
fn json_subset_mismatch(
    path: &str,
    expected: &serde_json::Value,
    actual: &serde_json::Value,
) -> Option<String> {
    use serde_json::Value;
    match (expected, actual) {
        (Value::Object(expected_map), Value::Object(actual_map)) => {
            expected_map.iter().find_map(|(k, v)| {
                let child = format!("{path}.{k}");
                match actual_map.get(k) {
                    Some(actual_v) => json_subset_mismatch(&child, v, actual_v),
                    None => Some(child),
                }
            })
        }
        (Value::Array(expected_items), Value::Array(actual_items)) => {
            if expected_items.len() != actual_items.len() {
                return Some(path.to_string());
            }
            expected_items
                .iter()
                .zip(actual_items)
                .enumerate()
                .find_map(|(i, (e, a))| json_subset_mismatch(&format!("{path}[{i}]"), e, a))
        }
        _ if expected == actual => None,
        _ => Some(path.to_string()),
    }
}

/// Resolve a dot-separated path (`"data.items.1.id"`) against `root` -
/// see [`TestResponse::assert_json_path`].
fn json_path<'a>(root: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut current = root;
    for segment in path.split('.') {
        current = match current {
            serde_json::Value::Object(map) => map.get(segment)?,
            serde_json::Value::Array(items) => items.get(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}
