//! Body parsing utilities for HTTP requests
//!
//! Provides async body collection and parsing for JSON, form-urlencoded and
//! multipart data.
//!
//! Body collection is capped to bound process memory under load. The cap
//! is layered in three places - see [`DEFAULT_MAX_REQUEST_BODY_BYTES`],
//! [`set_global_max_request_body_bytes`], and
//! [`crate::http::FormRequest::max_body_bytes`].
//!
//! Body collection is also bounded in time: every read waits under the
//! request's body deadlines, `SERVER_BODY_READ_TIMEOUT` for each frame and
//! `SERVER_BODY_TOTAL_TIMEOUT` for the whole body (see
//! [`crate::config::ServerConfig::body_read_timeout`]).

use crate::config::providers::{DEFAULT_BODY_READ_TIMEOUT_SECS, DEFAULT_BODY_TOTAL_TIMEOUT_SECS};
use crate::error::FrameworkError;
use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::body::Incoming;
use serde::de::DeserializeOwned;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::time::Instant;

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

/// How long a request body may take to arrive (HRD-001): the longest
/// wait for its next frame, and the instant by which all of it must have
/// arrived.
///
/// The header-read deadline ends once the request head is complete, and
/// hyper bounds nothing after it, so without this a client could send
/// complete headers and then hold the connection by sending the body a
/// byte at a time, or not at all. `Server::run` starts one for each
/// request as the head arrives and hands it to the request, and every read
/// of the body the framework performs waits under it: [`collect_after`],
/// the request's partial read, and the multipart parsers. A read that
/// misses it fails with [`timed_out`] and drops the stream; hyper then
/// stops reading the connection and closes it after the response.
#[derive(Clone, Copy, Debug)]
pub(crate) struct BodyDeadline {
    /// The longest wait for the next frame.
    frame: Duration,
    /// When the whole body must have arrived.
    whole: Instant,
}

impl BodyDeadline {
    /// Deadlines that start now: at most `frame` between frames, and all of
    /// the body within `total`.
    pub(crate) fn start(frame: Duration, total: Duration) -> Self {
        Self {
            frame,
            whole: after(Instant::now(), total),
        }
    }

    /// Deadlines that start now, from the registered
    /// [`ServerConfig`](crate::config::ServerConfig), or the defaults when
    /// none is registered. For a request no `Server::run` served: an
    /// embedder's own hyper loop, or a request a test built.
    pub(crate) fn configured() -> Self {
        match crate::config::Config::get::<crate::config::ServerConfig>() {
            Some(config) => Self::start(config.body_read_timeout, config.body_total_timeout),
            None => Self::start(
                Duration::from_secs(DEFAULT_BODY_READ_TIMEOUT_SECS),
                Duration::from_secs(DEFAULT_BODY_TOTAL_TIMEOUT_SECS),
            ),
        }
    }

    /// The instant the next frame must arrive by, if it is waited for from
    /// now: the per-frame deadline, or the whole-body one if that is sooner.
    pub(crate) fn next_frame(&self) -> Instant {
        after(Instant::now(), self.frame).min(self.whole)
    }

    /// The next frame of `body`, or [`timed_out`] when it does not arrive by
    /// [`Self::next_frame`].
    pub(crate) async fn frame(
        &self,
        body: &mut Incoming,
    ) -> Result<Option<Result<hyper::body::Frame<Bytes>, hyper::Error>>, FrameworkError> {
        tokio::time::timeout_at(self.next_frame(), body.frame())
            .await
            .map_err(|_| timed_out())
    }
}

/// `wait` after `now`, or thirty years after it when `now + wait` does not
/// fit an [`Instant`]: a configured deadline of `u64::MAX` seconds is a
/// very long deadline, not an overflow.
fn after(now: Instant, wait: Duration) -> Instant {
    now.checked_add(wait)
        .unwrap_or_else(|| now + Duration::from_secs(30 * 365 * 24 * 60 * 60))
}

/// The error a read of the body answers when the body misses its deadline:
/// `408 Request Timeout`, the status a server sends when it stops waiting
/// for a request (RFC 9110 15.5.9). The server marks every `408` response
/// `Connection: close`.
pub(crate) fn timed_out() -> FrameworkError {
    FrameworkError::domain("the request body did not arrive in time", 408)
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
///
/// The read waits under the configured body deadlines, started when this
/// is called; a body that misses one returns
/// `Err(FrameworkError::Domain { status_code: 408, .. })`. A
/// [`Request`](crate::http::Request) read through its own methods counts
/// the whole-body deadline from when the server read the request head.
pub async fn collect_body_with_cap(
    body: Incoming,
    content_length: Option<u64>,
    max_bytes: usize,
) -> Result<Bytes, FrameworkError> {
    collect_body_until(body, content_length, max_bytes, BodyDeadline::configured()).await
}

/// [`collect_body_with_cap`] under the deadlines `deadline` sets: the form
/// a request's own reads use, so the whole-body deadline counts from when
/// the server read the request head.
pub(crate) async fn collect_body_until(
    body: Incoming,
    content_length: Option<u64>,
    max_bytes: usize,
    deadline: BodyDeadline,
) -> Result<Bytes, FrameworkError> {
    // Pre-reject when Content-Length is declared and exceeds the cap. This
    // avoids buffering even a single frame for an attacker's giant POST.
    if let Some(len) = content_length
        && len > max_bytes as u64
    {
        return Err(over_limit(max_bytes));
    }
    collect_after(Bytes::new(), body, max_bytes, deadline).await
}

/// Collect `rest`, the stream after the bytes `read` a middleware already
/// read, onto them, the whole capped at `max_bytes` as
/// [`collect_body_with_cap`] caps it and each frame waited for under
/// `deadline`. For a body a middleware stopped reading at its own limit
/// ([`BodyState::Partial`](crate::http::BodyState::Partial)), and the read
/// behind every other collection of a body.
pub(crate) async fn collect_after(
    read: Bytes,
    rest: Incoming,
    max_bytes: usize,
    deadline: BodyDeadline,
) -> Result<Bytes, FrameworkError> {
    if read.len() > max_bytes {
        return Err(over_limit(max_bytes));
    }
    // `Incoming: Unpin`, so `body.frame()` is callable on `&mut body` without
    // pinning.
    let mut body = rest;
    // Each frame is copied as it arrives and dropped, so what the body
    // holds tracks its bytes, whatever number of frames the client chose to
    // send them in; a frame also shares the connection's read buffer, which
    // keeping it would keep. The buffer grows only with bytes that arrived,
    // never with a declared length a client need not send, and the room
    // left over is released once, so the body is held at its length for as
    // long as the handler keeps it.
    let mut buf: Vec<u8> = read.to_vec();
    while let Some(frame) = deadline.frame(&mut body).await? {
        let frame = frame.map_err(read_failure)?;
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

/// The error a read of the body answers when the stream fails: the one
/// [`Request::body_bytes`](crate::http::Request::body_bytes) returns, and
/// the one a middleware that read the body first keeps for the handler.
pub(crate) fn read_failure(error: hyper::Error) -> FrameworkError {
    FrameworkError::internal(format!("Failed to read request body: {error}"))
}

#[inline]
pub(crate) fn over_limit(max_bytes: usize) -> FrameworkError {
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
/// last value, as PHP does. A name with brackets is nested data, as PHP's
/// `parse_str` reads it: `user[name]` is the member `name` of `user`,
/// `tags[]` appends to the list `tags`, and `photos[1]` is an element of
/// the list `photos`, read in index order, with empty elements `null` in
/// their places.
///
/// A field that is missing or does not parse answers as a validation
/// failure, a 422 whose `errors` names every such field under its input
/// name with a catalog message, as the multipart extractor reports one.
/// Any other failure, such as a name a struct denies, is a 422 that words
/// it.
pub fn parse_form<T: DeserializeOwned>(bytes: &Bytes) -> Result<T, FrameworkError> {
    crate::http::input::parse_form_input(bytes)
        .map_err(|error| error.into_framework_error("Failed to parse form body"))
}

/// Whether a `Content-Type` value names `multipart/form-data`, compared as
/// [`is_form_urlencoded`] compares, so a boundary parameter or a capital
/// letter does not change what the body is.
pub(crate) fn is_multipart_form_data(content_type: &str) -> bool {
    content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .eq_ignore_ascii_case("multipart/form-data")
}

/// Read a `multipart/form-data` body into `T` as a form body is read, with
/// bracketed and indexed names nested (see [`parse_form`]), and each part
/// that carries a file handed to an `UploadedFile` field.
///
/// This is the body the Inertia client sends for a form with a file. The
/// body is capped at `max_body_bytes`, the cap the caller applies to any
/// body it reads, so a multipart body is never larger than a url-encoded
/// one may be. The part ceiling and the in-memory limit of each part come
/// from the upload settings, as for `#[derive(MultipartRequest)]`
/// ([`crate::http::upload::global_max_multipart_parts`],
/// [`crate::http::upload::global_upload_spill_threshold`]): a body over a
/// limit answers 413, and a file part over the in-memory limit is written
/// to a temp file.
pub(crate) async fn parse_multipart<T: DeserializeOwned>(
    req: crate::http::Request,
    max_body_bytes: usize,
) -> Result<T, FrameworkError> {
    parse_multipart_with_route_inputs(req, max_body_bytes, serde_json::Map::new()).await
}

/// Keep multipart limits and file handling shared when a form also has route inputs.
pub(crate) async fn parse_multipart_with_route_inputs<T: DeserializeOwned>(
    req: crate::http::Request,
    max_body_bytes: usize,
    route_inputs: serde_json::Map<String, serde_json::Value>,
) -> Result<T, FrameworkError> {
    let payload = crate::http::upload::parse_multipart_streaming_with_limits(
        req,
        crate::http::upload::MultipartLimits {
            max_body_bytes,
            max_parts: crate::http::upload::global_max_multipart_parts(),
            spill_threshold: crate::http::upload::global_upload_spill_threshold(),
            per_field_max_counts: &[],
        },
        |_, _, _| Ok(()),
    )
    .await?;
    let parsed = if route_inputs.is_empty() {
        crate::http::input::parse_multipart_input(payload)
    } else {
        crate::http::input::parse_multipart_with_route_inputs(payload, route_inputs)
    };
    parsed.map_err(|error| error.into_framework_error("Failed to parse multipart body"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// HRD-001: the per-frame wait never runs past the whole-body deadline,
    /// so a body that keeps arriving inside the per-frame deadline still
    /// ends at the total one.
    #[tokio::test]
    async fn hardening_the_next_frame_waits_no_later_than_the_whole_body() {
        let deadline = BodyDeadline::start(Duration::from_secs(30), Duration::from_millis(50));
        assert_eq!(deadline.next_frame(), deadline.whole);

        let deadline = BodyDeadline::start(Duration::from_millis(50), Duration::from_secs(30));
        assert!(deadline.next_frame() < deadline.whole);
    }

    /// A configured deadline too long to add to the clock is a very long
    /// deadline, not a panic.
    #[tokio::test]
    async fn hardening_a_deadline_past_the_clock_does_not_overflow() {
        let deadline = BodyDeadline::start(Duration::MAX, Duration::MAX);
        let now = Instant::now();
        assert!(deadline.whole > now);
        assert!(deadline.next_frame() > now);
    }

    /// The error a late body answers is a `408`.
    #[test]
    fn hardening_a_late_body_answers_408() {
        assert_eq!(timed_out().status_code(), 408);
    }
}
