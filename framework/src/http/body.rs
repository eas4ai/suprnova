//! Body parsing utilities for HTTP requests
//!
//! Provides async body collection and parsing for JSON and form-urlencoded data.
//!
//! Body collection is capped to bound process memory under load. The cap
//! is layered in three places - see [`DEFAULT_MAX_REQUEST_BODY_BYTES`],
//! [`set_global_max_request_body_bytes`], and
//! [`crate::http::FormRequest::max_body_bytes`].

use crate::error::FrameworkError;
use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::body::Incoming;
use serde::de::DeserializeOwned;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Default cap on generic (JSON / form-urlencoded / raw) request body size,
/// in bytes.
///
/// 8 MiB - large enough for typical JSON payloads (including base64-encoded
/// images embedded in JSON), small enough that an unauthenticated client
/// can't trivially exhaust process memory with a single request. Set at
/// compile time; can be overridden at boot via
/// [`set_global_max_request_body_bytes`] or per FormRequest struct via
/// [`crate::http::FormRequest::max_body_bytes`].
///
/// Multipart uploads use a separate, larger cap
/// (`DEFAULT_MAX_MULTIPART_BODY_BYTES`); they're expected to carry binary
/// payloads.
pub const DEFAULT_MAX_REQUEST_BODY_BYTES: usize = 8 * 1024 * 1024;

static GLOBAL_MAX_REQUEST_BODY: AtomicUsize = AtomicUsize::new(0);

/// Set the process-global cap on generic request body size, in bytes.
///
/// Called at boot - typically from `bootstrap.rs` - to override the
/// compile-time [`DEFAULT_MAX_REQUEST_BODY_BYTES`]. Setting `0` is special:
/// it means "use the default". Setting `usize::MAX` disables the cap
/// entirely (not recommended for public-facing endpoints).
///
/// Per-FormRequest overrides via
/// [`crate::http::FormRequest::max_body_bytes`] still take precedence.
///
/// Thread-safe; can be called multiple times. The most recent value wins
/// for any subsequent request.
pub fn set_global_max_request_body_bytes(bytes: usize) {
    GLOBAL_MAX_REQUEST_BODY.store(bytes, Ordering::SeqCst);
}

/// Read the currently-configured global cap. Returns the default
/// ([`DEFAULT_MAX_REQUEST_BODY_BYTES`]) if
/// [`set_global_max_request_body_bytes`] has never been called or was last
/// called with `0`.
pub fn global_max_request_body_bytes() -> usize {
    let stored = GLOBAL_MAX_REQUEST_BODY.load(Ordering::SeqCst);
    if stored == 0 {
        DEFAULT_MAX_REQUEST_BODY_BYTES
    } else {
        stored
    }
}

/// Collect the full body from an [`Incoming`] stream into [`Bytes`],
/// enforcing a cap on total size.
///
/// Enforcement is two-layered:
///
/// 1. **Pre-check**: when `content_length` is `Some(n)` and `n > max_bytes`,
///    rejects with HTTP 413 **before reading any body bytes**. This is the
///    cheap path for the common case where a client declares an honest
///    `Content-Length` header.
///
/// 2. **Progressive**: every frame is added to a running total; the
///    function rejects with HTTP 413 as soon as the accumulated total
///    exceeds `max_bytes`. This catches:
///    - clients that lie about `Content-Length` (declare small, send big)
///    - chunked transfers with no `Content-Length`
///
/// Overflow always returns
/// `Err(FrameworkError::Domain { status_code: 413, .. })` so the framework's
/// standard error → response mapping renders the right status.
pub async fn collect_body_with_cap(
    body: Incoming,
    content_length: Option<u64>,
    max_bytes: usize,
) -> Result<Bytes, FrameworkError> {
    // Pre-reject when Content-Length is declared and exceeds the cap. This
    // avoids buffering even a single frame for an attacker's giant POST.
    if let Some(len) = content_length
        && len > max_bytes as u64
    {
        return Err(over_limit(max_bytes));
    }

    // `Incoming: Unpin`, so `body.frame()` is callable on `&mut body` without
    // pinning.
    let mut body = body;
    // Each frame is copied as it arrives and dropped, so what the body
    // holds tracks its bytes, whatever number of frames the client chose to
    // send them in; a frame also shares the connection's read buffer, which
    // keeping it would keep. The buffer grows only with bytes that arrived,
    // never with a declared length a client need not send, and the room
    // left over is released once, so the body is held at its length for as
    // long as the handler keeps it.
    let mut buf: Vec<u8> = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame = frame
            .map_err(|e| FrameworkError::internal(format!("Failed to read request body: {e}")))?;
        // Frames may carry data OR trailers; we only count + buffer data.
        // `into_data` returns `Ok(Bytes)` for data frames and `Err(Frame)`
        // for trailer frames (which we ignore).
        if let Ok(data) = frame.into_data() {
            if data.len() > max_bytes - buf.len() {
                return Err(over_limit(max_bytes));
            }
            buf.extend_from_slice(&data);
        }
    }
    if buf.capacity() > buf.len() {
        buf.shrink_to_fit();
    }
    Ok(Bytes::from(buf))
}

#[inline]
fn over_limit(max_bytes: usize) -> FrameworkError {
    FrameworkError::Domain {
        message: format!("request body exceeds {max_bytes} bytes (cap)"),
        status_code: 413,
    }
}

/// Parse the `Content-Length` header into a byte count, if present and
/// well-formed. Returns `None` for an absent, non-ASCII, or unparseable
/// value. Shared by the generic body path ([`Request::body_bytes_with_cap`])
/// and the multipart parser so both pre-reject an honestly-declared
/// oversized body the same way.
pub(crate) fn parse_content_length(headers: &hyper::http::HeaderMap) -> Option<u64> {
    headers
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
}

/// Collect the full body from an [`Incoming`] stream, capped at the
/// process-global request-body limit (see
/// [`global_max_request_body_bytes`]). For callers that don't have the
/// `Content-Length` header handy, no pre-check is performed; the
/// progressive cap still enforces during read.
///
/// New callers that have access to the `Content-Length` header (e.g.
/// `Request::body_bytes`) should prefer [`collect_body_with_cap`] directly
/// and pass the parsed length so oversized requests are rejected before
/// any read.
pub async fn collect_body(body: Incoming) -> Result<Bytes, FrameworkError> {
    collect_body_with_cap(body, None, global_max_request_body_bytes()).await
}

/// Parse bytes as JSON into the target type
///
/// A body that is JSON but does not fit a struct `T` answers as a
/// validation failure, a 422 whose `errors` names every field that failed
/// under its input name (`address.street`, `items.1`) with a catalog
/// message: `validation-required` for a missing field or a `null` one, and
/// `validation-integer`, `validation-numeric`, `validation-boolean`,
/// `validation-string` or `validation-format` for a value of the wrong
/// kind. That is what lets the Inertia validation redirect show each one
/// under its input. Any other failure, a body that is not JSON among them,
/// is a 422 that words it.
pub fn parse_json<T: DeserializeOwned>(bytes: &Bytes) -> Result<T, FrameworkError> {
    serde_json::from_slice(bytes).map_err(|e| {
        match crate::http::input::json_field_failures::<T>(bytes) {
            Some(errors) => FrameworkError::Validation(errors),
            None => FrameworkError::domain(format!("Failed to parse JSON body: {}", e), 422),
        }
    })
}

/// Whether a `Content-Type` value names `application/x-www-form-urlencoded`.
///
/// Media types are case-insensitive and may carry parameters (RFC 9110
/// 8.3.1), so `Application/X-WWW-Form-Urlencoded; charset=UTF-8` is a form
/// body. `FormRequest` always parsed it as one; every other place that
/// decides whether to read a body as a form goes through here, so the
/// middleware that reads a field (CSRF's `_token`, a rate-limit identity,
/// Pusher's `socket_id`) and the handler that parses the body cannot
/// disagree about what the body is.
pub(crate) fn is_form_urlencoded(content_type: &str) -> bool {
    content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .eq_ignore_ascii_case("application/x-www-form-urlencoded")
}

/// Parse bytes as form-urlencoded into the target type, reading the form
/// as Laravel's request does.
///
/// A form cannot send `null`: an HTML form sends an empty input as `name=`,
/// and Laravel's default `ConvertEmptyStringsToNull` middleware reads it as
/// `null` before any rule runs. So an empty value is left out, which is how
/// `null` reaches a typed field: an `Option` is `None` and a required field
/// is missing, a `String` included. A name sent more than once keeps its
/// last value, as PHP does. A name that ends in `[]`, PHP's mark for a
/// list, is read as a list under the name without the brackets, with its
/// empty elements left out. The multipart extractor reads a form by the
/// same rules, so a form gives a handler the same values whichever way it
/// is posted.
///
/// A field that is missing or does not parse answers as a validation
/// failure, a 422 whose `errors` names every such field under its input
/// name with a catalog message, as the multipart extractor reports one.
/// Any other failure, such as a name a struct denies, is a 422 that words
/// it.
pub fn parse_form<T: DeserializeOwned>(bytes: &Bytes) -> Result<T, FrameworkError> {
    crate::http::input::parse_form_input(bytes).map_err(|error| match error {
        crate::http::input::InputError::Fields(errors) => FrameworkError::Validation(errors),
        crate::http::input::InputError::Other(message) => {
            FrameworkError::domain(format!("Failed to parse form body: {message}"), 422)
        }
    })
}
