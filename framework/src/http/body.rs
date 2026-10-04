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
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
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
/// Deserialization errors map to 422 Unprocessable Entity - the client
/// supplied invalid input (wrong shape, rejected fields, bad types).
pub fn parse_json<T: DeserializeOwned>(bytes: &Bytes) -> Result<T, FrameworkError> {
    serde_json::from_slice(bytes)
        .map_err(|e| FrameworkError::domain(format!("Failed to parse JSON body: {}", e), 422))
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
/// last value, as PHP does, unless it ends in `[]`, PHP's mark for a list.
/// The multipart extractor reads a form by the same two rules, so a form
/// gives a handler the same values whichever way it is posted.
///
/// Deserialization errors map to 422 Unprocessable Entity - the client
/// supplied invalid input.
pub fn parse_form<T: DeserializeOwned>(bytes: &Bytes) -> Result<T, FrameworkError> {
    serde_urlencoded::from_str(&form_as_laravel_reads_it(bytes, struct_field_names::<T>()))
        .map_err(|e| FrameworkError::domain(format!("Failed to parse form body: {}", e), 422))
}

/// `bytes` re-encoded with only the pairs Laravel would read: no empty
/// value, and for a name that is not a list only its last pair.
///
/// `fields` names the only pairs a repeat can matter for, a struct's
/// fields: any other name a struct ignores, so only these are tracked, and
/// the lookup holds one entry per field however many names the client
/// sends. With `None` the target can hold any name, as a map does, and
/// every name is tracked.
fn form_as_laravel_reads_it(bytes: &[u8], fields: Option<&[&str]>) -> String {
    let fields: Option<HashSet<&str>> = fields.map(|names| names.iter().copied().collect());
    // The position of the last pair of each tracked name.
    let mut last: HashMap<Cow<'_, str>, usize> = HashMap::new();
    for (at, (name, _)) in url::form_urlencoded::parse(bytes).enumerate() {
        let tracked = !name.ends_with("[]")
            && fields
                .as_ref()
                .is_none_or(|fields| fields.contains(name.as_ref()));
        if tracked {
            last.insert(name, at);
        }
    }

    let mut kept = url::form_urlencoded::Serializer::new(String::with_capacity(bytes.len()));
    for (at, (name, value)) in url::form_urlencoded::parse(bytes).enumerate() {
        let replaced = last
            .get(name.as_ref())
            .is_some_and(|&last_at| last_at != at);
        if !value.is_empty() && !replaced {
            kept.append_pair(&name, &value);
        }
    }
    kept.finish()
}

/// The names of `T`'s fields when `T` deserializes as a struct; `None`
/// when it deserializes as anything else, a map or a struct with a
/// flattened field among them, which can take any name.
///
/// A derived `Deserialize` hands its field names to the deserializer
/// before it reads a byte, so a deserializer that keeps the names and
/// stops there reads them without any input.
fn struct_field_names<T: DeserializeOwned>() -> Option<&'static [&'static str]> {
    match T::deserialize(FieldNames) {
        Err(FieldNamesRead(fields)) => fields,
        Ok(_) => None,
    }
}

/// The deserializer [`struct_field_names`] runs: it fails at once,
/// carrying the field names when it was asked for a struct.
struct FieldNames;

/// The "error" that carries what [`FieldNames`] read.
#[derive(Debug)]
struct FieldNamesRead(Option<&'static [&'static str]>);

impl std::fmt::Display for FieldNamesRead {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("read the field names of a struct, not a value")
    }
}

impl std::error::Error for FieldNamesRead {}

impl serde::de::Error for FieldNamesRead {
    fn custom<M: std::fmt::Display>(_message: M) -> Self {
        Self(None)
    }
}

impl<'de> serde::Deserializer<'de> for FieldNames {
    type Error = FieldNamesRead;

    fn deserialize_any<V: serde::de::Visitor<'de>>(self, _: V) -> Result<V::Value, Self::Error> {
        Err(FieldNamesRead(None))
    }

    fn deserialize_struct<V: serde::de::Visitor<'de>>(
        self,
        _name: &'static str,
        fields: &'static [&'static str],
        _visitor: V,
    ) -> Result<V::Value, Self::Error> {
        Err(FieldNamesRead(Some(fields)))
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map enum identifier ignored_any
    }
}

#[cfg(test)]
mod form_tests {
    use super::*;
    use serde::Deserialize;
    use std::collections::BTreeMap;

    #[derive(Debug, Deserialize)]
    struct Profile {
        name: String,
        #[serde(rename = "about")]
        bio: Option<String>,
    }

    #[derive(Debug, Deserialize)]
    struct WithExtra {
        name: String,
        #[serde(flatten)]
        extra: BTreeMap<String, String>,
    }

    #[test]
    fn a_struct_hands_over_its_wire_names_and_a_map_none() {
        assert_eq!(
            struct_field_names::<Profile>(),
            Some(&["name", "about"][..])
        );
        assert_eq!(struct_field_names::<BTreeMap<String, String>>(), None);
        assert_eq!(struct_field_names::<WithExtra>(), None);
    }

    #[test]
    fn a_struct_reads_a_renamed_field_by_its_wire_name() {
        let form: Profile =
            parse_form(&Bytes::from_static(b"name=&name=Ada&about=x&about=Hi")).expect("a struct");
        assert_eq!(form.name, "Ada");
        assert_eq!(form.bio.as_deref(), Some("Hi"));

        let form: Profile = parse_form(&Bytes::from_static(b"name=Ada&about=")).expect("a struct");
        assert_eq!(form.bio, None);
    }

    #[test]
    fn a_list_keeps_every_value_and_no_name_keeps_an_empty_one() {
        assert_eq!(
            form_as_laravel_reads_it(b"tags[]=a&tags[]=&tags[]=b&name=x&name=y&bio=", None),
            "tags%5B%5D=a&tags%5B%5D=b&name=y"
        );
        // `name=` last is `null`, whatever came before it.
        assert_eq!(form_as_laravel_reads_it(b"name=x&name=", None), "");
    }

    #[test]
    fn only_a_structs_own_names_are_tracked() {
        // `other` is no field of the struct, so its repeats pass through
        // for the struct to ignore.
        assert_eq!(
            form_as_laravel_reads_it(b"other=1&name=x&other=2&name=y", Some(&["name"])),
            "other=1&other=2&name=y"
        );
    }

    #[test]
    fn a_map_and_a_flattened_struct_read_the_last_value_of_a_name() {
        let map: BTreeMap<String, String> =
            parse_form(&Bytes::from_static(b"a=1&a=2&b=&c=3")).expect("a map");
        assert_eq!(
            map,
            BTreeMap::from([("a".into(), "2".into()), ("c".into(), "3".into())])
        );

        let form: WithExtra =
            parse_form(&Bytes::from_static(b"name=x&name=y&x=1&x=2&blank=")).expect("a struct");
        assert_eq!(form.name, "y");
        assert_eq!(form.extra, BTreeMap::from([("x".into(), "2".into())]));
    }

    #[test]
    fn an_encoded_value_survives_the_round_trip() {
        let map: BTreeMap<String, String> =
            parse_form(&Bytes::from_static(b"q=a+b%26c%3Dd&q2=%C3%A9")).expect("a map");
        assert_eq!(map["q"], "a b&c=d");
        assert_eq!(map["q2"], "\u{e9}");
    }
}
