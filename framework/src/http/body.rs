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
//! A request body is also bounded in time (HRD-001). The body a
//! [`crate::http::Request`] holds is a [`TimedBody`], which waits for each
//! frame under the request's body deadlines, `SERVER_BODY_READ_TIMEOUT` for
//! each frame and `SERVER_BODY_TOTAL_TIMEOUT` for the whole body (see
//! [`crate::config::ServerConfig::body_read_timeout`]), whoever reads it.
//! The server also watches each connection, so a body nothing reads is
//! abandoned with its connection at the total deadline.

use crate::config::providers::{DEFAULT_BODY_READ_TIMEOUT_SECS, DEFAULT_BODY_TOTAL_TIMEOUT_SECS};
use crate::error::FrameworkError;
use bytes::Bytes;
use futures::task::AtomicWaker;
use http_body_util::BodyExt;
use hyper::body::{Body, Frame, Incoming, SizeHint};
use serde::de::DeserializeOwned;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::task::{Context, Poll, Waker};
use std::time::Duration;
use tokio::task::AbortHandle;
use tokio::time::{Instant, Sleep};

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
/// request as the head arrives and hands it to the request, which reads
/// its body through a [`TimedBody`] that waits under it on every poll,
/// whoever polls. A wait that misses it fails with [`timed_out`] and drops
/// the stream; hyper then stops reading the connection and closes it after
/// the response. The connection's [`ConnectionWatch`] holds the whole-body
/// instant too, for the body nothing polls.
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

    /// Whether the instant the whole body had to arrive by has passed.
    pub(crate) fn passed(&self) -> bool {
        Instant::now() >= self.whole
    }
}

/// A request body that waits for each frame under the request's body
/// deadlines (HRD-001), whoever reads it.
///
/// [`BodyState::Streaming`](crate::http::BodyState::Streaming) holds one,
/// so a handler that takes the body with
/// [`Request::into_parts`](crate::http::Request::into_parts) and polls it
/// itself reads under the deadlines that `body_bytes`, `json` and the
/// multipart parsers read under: at most `SERVER_BODY_READ_TIMEOUT` for each
/// frame, and the whole body by `SERVER_BODY_TOTAL_TIMEOUT` after the server
/// read the request head. A frame that misses either ends the body with the
/// `408 Request Timeout` error, after which the body yields nothing more.
/// The stream underneath is dropped with it, so the server stops reading
/// the connection and closes it after the response.
///
/// It is a [`hyper::body::Body`] whose error is a [`FrameworkError`], so
/// `http_body_util::BodyExt` reads it as it reads any body, and `?` returns
/// a failed frame from a handler: the `408` for a late one, or, for a
/// stream that fails, the error [`Request::body_bytes`](crate::http::Request::body_bytes)
/// returns. A body that `Server::run` serves is watched from its connection
/// as well: one that has not arrived whole by the total deadline, read or
/// not, loses its connection.
pub struct TimedBody {
    /// The stream, until it ends, fails or misses a deadline.
    stream: Option<Incoming>,
    /// The deadlines each wait for a frame runs under.
    deadline: BodyDeadline,
    /// The timer of the wait for a frame, kept between waits so that a
    /// body of many frames allocates it once.
    wait: Option<Pin<Box<Sleep>>>,
    /// Whether `wait` is set for the frame being waited for.
    waiting: bool,
    /// The connection's watch on this body, told when the body arrives.
    arrival: Option<BodyArrival>,
}

impl TimedBody {
    /// `stream` under `deadline`, with `arrival` to tell the connection's
    /// watch when it has arrived whole.
    pub(crate) fn new(
        stream: Incoming,
        deadline: BodyDeadline,
        arrival: Option<BodyArrival>,
    ) -> Self {
        let mut body = Self {
            stream: Some(stream),
            deadline,
            wait: None,
            waiting: false,
            arrival,
        };
        // A body with nothing to send, as most requests have, has arrived.
        if body.stream.as_ref().is_some_and(Incoming::is_end_stream) {
            body.arrived();
        }
        body
    }

    /// The body has arrived whole: the connection's watch stands down.
    fn arrived(&mut self) {
        if let Some(arrival) = self.arrival.take() {
            arrival.stand_down();
        }
    }

    /// Stop reading: drop the stream, after which the body yields nothing.
    fn abandon(&mut self) {
        self.stream = None;
        self.waiting = false;
    }
}

impl From<Incoming> for TimedBody {
    /// The body of a request no `Server::run` served, such as one an
    /// embedder's own hyper loop hands over, under the deadlines the
    /// registered [`ServerConfig`](crate::config::ServerConfig) sets, or the
    /// defaults, starting now.
    fn from(stream: Incoming) -> Self {
        Self::new(stream, BodyDeadline::configured(), None)
    }
}

impl Body for TimedBody {
    type Data = Bytes;
    type Error = FrameworkError;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, FrameworkError>>> {
        let this = self.get_mut();
        let Some(stream) = this.stream.as_mut() else {
            return Poll::Ready(None);
        };
        // The stream first: a frame that has arrived is read, however late
        // the reader is to ask for it.
        match Pin::new(&mut *stream).poll_frame(cx) {
            Poll::Ready(Some(Ok(frame))) => {
                this.waiting = false;
                // A body of declared length has arrived with its last byte,
                // whether or not the reader asks for the end after it.
                if stream.is_end_stream() {
                    this.arrived();
                }
                Poll::Ready(Some(Ok(frame)))
            }
            Poll::Ready(None) => {
                this.abandon();
                this.arrived();
                Poll::Ready(None)
            }
            Poll::Ready(Some(Err(error))) => {
                this.abandon();
                // The connection's watch cuts the stream off at the total
                // deadline, so a stream that fails past it failed because
                // it was late.
                let error = if this.deadline.passed() {
                    timed_out()
                } else {
                    read_failure(error)
                };
                Poll::Ready(Some(Err(error)))
            }
            Poll::Pending => {
                let deadline = this.deadline;
                let wait = this.wait.get_or_insert_with(|| {
                    Box::pin(tokio::time::sleep_until(deadline.next_frame()))
                });
                // The wait for this frame starts with the first poll that
                // finds it missing.
                if !this.waiting {
                    wait.as_mut().reset(deadline.next_frame());
                    this.waiting = true;
                }
                if wait.as_mut().poll(cx).is_ready() {
                    this.abandon();
                    return Poll::Ready(Some(Err(timed_out())));
                }
                Poll::Pending
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        self.stream.as_ref().is_none_or(Incoming::is_end_stream)
    }

    fn size_hint(&self) -> SizeHint {
        self.stream
            .as_ref()
            .map_or_else(|| SizeHint::with_exact(0), Incoming::size_hint)
    }
}

/// The watch a connection [`Server::run`](crate::Server::run) serves keeps
/// on its request bodies (HRD-001): the timer that abandons the connection
/// when a body has not arrived whole by its total deadline, whether or not
/// anything reads it.
///
/// The deadlines of a [`TimedBody`] act only while something polls it. A
/// handler that keeps the request and never reads the body, or reads it
/// only after other work, polls neither, and hyper reads the connection
/// only for a body that is polled. So each request head arms a timer for
/// that request's whole-body instant, and a body that arrives whole stands
/// it down. A timer that fires marks the connection abandoned and wakes
/// it; the connection's IO then fails its reads and flushes, which ends
/// hyper's connection and closes the socket.
#[derive(Default)]
pub(crate) struct ConnectionWatch {
    /// Set when a body missed its total deadline; never cleared.
    abandoned: AtomicBool,
    /// The task driving the connection, woken when it is abandoned.
    waker: AtomicWaker,
    /// The timer of the request whose body the connection is reading.
    timer: std::sync::Mutex<Option<AbortHandle>>,
}

impl ConnectionWatch {
    /// A request head arrived. The previous request's timer stands down,
    /// since hyper reads a head only once the body before it is read or
    /// abandoned, and unless the new body is empty a timer starts that
    /// abandons the connection at `deadline`'s whole-body instant. The
    /// returned [`BodyArrival`] stands that timer down once the body has
    /// arrived.
    pub(crate) fn arm(
        self: &Arc<Self>,
        deadline: BodyDeadline,
        empty: bool,
    ) -> Option<BodyArrival> {
        let mut timer = crate::lock::recover(&self.timer);
        if let Some(previous) = timer.take() {
            previous.abort();
        }
        if empty {
            return None;
        }
        // Weak, so a timer never keeps a closed connection's watch alive.
        let watch = Arc::downgrade(self);
        let task = tokio::spawn(async move {
            tokio::time::sleep_until(deadline.whole).await;
            if let Some(watch) = watch.upgrade() {
                watch.abandon();
            }
        });
        *timer = Some(task.abort_handle());
        Some(BodyArrival(Arc::new(task.abort_handle())))
    }

    /// Mark the connection abandoned and wake the task that drives it, for
    /// its IO to fail.
    fn abandon(&self) {
        self.abandoned.store(true, Ordering::Release);
        self.waker.wake();
    }

    /// Whether a body on this connection missed its total deadline.
    pub(crate) fn is_abandoned(&self) -> bool {
        self.abandoned.load(Ordering::Acquire)
    }

    /// Remember the task polling the connection's IO, to wake when the
    /// connection is abandoned.
    pub(crate) fn register(&self, waker: &Waker) {
        self.waker.register(waker);
    }
}

impl Drop for ConnectionWatch {
    /// A closed connection's timer stands down rather than sleep out the
    /// total deadline, an hour by default.
    fn drop(&mut self) {
        let timer = match self.timer.get_mut() {
            Ok(timer) => timer.take(),
            Err(poisoned) => poisoned.into_inner().take(),
        };
        if let Some(timer) = timer {
            timer.abort();
        }
    }
}

/// What a request's [`TimedBody`] tells its connection's
/// [`ConnectionWatch`]: that the body arrived whole, so the timer that
/// would abandon the connection stands down. `Server::run` puts it in the
/// request's extensions for [`crate::http::Request::new`] to hand to the
/// body.
#[derive(Clone)]
pub(crate) struct BodyArrival(Arc<AbortHandle>);

impl BodyArrival {
    /// Stand the request's timer down.
    fn stand_down(self) {
        self.0.abort();
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

/// Whether `error` is the one a [`TimedBody`] fails with when a frame
/// misses its deadline, for a reader that passes the body's errors through
/// a layer that hides them, as multer does.
pub(crate) fn is_timed_out(error: &FrameworkError) -> bool {
    error.status_code() == 408
}

/// Collect the full body from a request body stream into [`Bytes`],
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
/// The read waits under the body deadlines; a body that misses one returns
/// `Err(FrameworkError::Domain { status_code: 408, .. })`. A [`TimedBody`],
/// the body `Request::into_parts` hands over, keeps the deadlines the
/// server started as it read the request head. A hyper `Incoming` starts
/// the configured ones when this is called.
pub async fn collect_body_with_cap(
    body: impl Into<TimedBody>,
    content_length: Option<u64>,
    max_bytes: usize,
) -> Result<Bytes, FrameworkError> {
    collect_body_until(body.into(), content_length, max_bytes).await
}

/// [`collect_body_with_cap`] for the body a request holds, whose deadlines
/// count from when the server read the request head.
pub(crate) async fn collect_body_until(
    body: TimedBody,
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
    collect_after(Bytes::new(), body, max_bytes).await
}

/// Collect `rest`, the stream after the bytes `read` a middleware already
/// read, onto them, the whole capped at `max_bytes` as
/// [`collect_body_with_cap`] caps it, each frame waited for under the
/// body's deadlines. For a body a middleware stopped reading at its own
/// limit ([`BodyState::Partial`](crate::http::BodyState::Partial)), and the
/// read behind every other collection of a body.
pub(crate) async fn collect_after(
    read: Bytes,
    rest: TimedBody,
    max_bytes: usize,
) -> Result<Bytes, FrameworkError> {
    if read.len() > max_bytes {
        return Err(over_limit(max_bytes));
    }
    // `TimedBody: Unpin`, so `body.frame()` is callable on `&mut body`
    // without pinning.
    let mut body = rest;
    // Each frame is copied as it arrives and dropped, so what the body
    // holds tracks its bytes, whatever number of frames the client chose to
    // send them in; a frame also shares the connection's read buffer, which
    // keeping it would keep. The buffer grows only with bytes that arrived,
    // never with a declared length a client need not send, and the room
    // left over is released once, so the body is held at its length for as
    // long as the handler keeps it.
    let mut buf: Vec<u8> = read.to_vec();
    while let Some(frame) = body.frame().await {
        // A late frame fails with the `408`, a failed stream with the
        // error `read_failure` words.
        let frame = frame?;
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

/// Collect the full body from a request body stream, capped at the
/// process-global request-body limit (see
/// [`global_max_request_body_bytes`]), under the body deadlines as
/// [`collect_body_with_cap`] reads. For callers that don't have the
/// `Content-Length` header handy, no pre-check is performed; the
/// progressive cap still enforces during read.
///
/// New callers that have access to the `Content-Length` header (e.g.
/// `Request::body_bytes`) should prefer [`collect_body_with_cap`] directly
/// and pass the parsed length so oversized requests are rejected before
/// any read.
pub async fn collect_body(body: impl Into<TimedBody>) -> Result<Bytes, FrameworkError> {
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
        assert!(is_timed_out(&timed_out()));
        assert!(!is_timed_out(&FrameworkError::internal(
            "Failed to read request body: connection reset"
        )));
    }

    /// A body that has not arrived by its whole-body instant abandons the
    /// connection, whether or not anything reads it.
    #[tokio::test(start_paused = true)]
    async fn hardening_an_armed_watch_abandons_the_connection_at_the_total_deadline() {
        let watch = Arc::new(ConnectionWatch::default());
        let deadline = BodyDeadline::start(Duration::from_secs(30), Duration::from_secs(60));
        let _arrival = watch.arm(deadline, false).expect("a body to watch");

        tokio::time::sleep(Duration::from_secs(59)).await;
        assert!(!watch.is_abandoned(), "not before the total deadline");
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert!(watch.is_abandoned(), "at the total deadline");
    }

    /// A body that arrives whole stands its connection's watch down.
    #[tokio::test(start_paused = true)]
    async fn hardening_a_body_that_arrives_stands_the_watch_down() {
        let watch = Arc::new(ConnectionWatch::default());
        let deadline = BodyDeadline::start(Duration::from_secs(30), Duration::from_secs(60));
        let arrival = watch.arm(deadline, false).expect("a body to watch");

        arrival.stand_down();
        tokio::time::sleep(Duration::from_secs(120)).await;
        assert!(!watch.is_abandoned());
    }

    /// The next request's head stands the previous request's watch down:
    /// hyper reads a head only once the body before it is done with, so
    /// a body a handler dropped unread, which hyper drained, does not
    /// close the keep-alive connection under the next request. A request
    /// with no body, as a `GET` has, arms nothing.
    #[tokio::test(start_paused = true)]
    async fn hardening_the_next_request_stands_the_previous_watch_down() {
        let watch = Arc::new(ConnectionWatch::default());
        let first = BodyDeadline::start(Duration::from_secs(30), Duration::from_secs(60));
        let _unread = watch.arm(first, false).expect("a body to watch");

        let next = BodyDeadline::start(Duration::from_secs(30), Duration::from_secs(60));
        assert!(
            watch.arm(next, true).is_none(),
            "an empty body arms nothing"
        );
        tokio::time::sleep(Duration::from_secs(120)).await;
        assert!(!watch.is_abandoned());
    }

    /// A closed connection's timer does not outlive it, and a timer that
    /// fires after the watch is gone does nothing.
    #[tokio::test(start_paused = true)]
    async fn hardening_a_closed_connection_takes_its_timer_with_it() {
        let watch = Arc::new(ConnectionWatch::default());
        let deadline = BodyDeadline::start(Duration::from_secs(30), Duration::from_secs(60));
        let arrival = watch.arm(deadline, false).expect("a body to watch");

        drop(watch);
        tokio::time::sleep(Duration::from_millis(1)).await;
        assert!(
            arrival.0.is_finished(),
            "dropping the connection's watch aborts its timer"
        );
    }
}
