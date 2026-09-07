//! Complete L0 hits without decode, hashing, or key allocations: a
//! [`HotEntry`] is decoded once at publication and keeps every header value
//! a hit can precompute; [`serve_hot`] forms the response with at most a
//! header-map allocation and an `Age` value (see its doc for the one
//! per-request value a public seed deadline adds). [`respond`] is the same
//! formation for bytes that are not a hot entry, so hosts have one builder.
//!
//! Both public functions end in the same private builder over the same
//! precomputed values, so a hot hit and a cold response can never drift
//! apart in status, header order, or body treatment; the only difference is
//! where the values came from.

use bytes::Bytes;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use http::{Method, Response, StatusCode};

use super::coherence::age_seconds;
use super::entry::{CompleteEntry, SafeHeaders, Validator};
use super::http::{ConditionalOutcome, cache_control_value, conditional_matches, vary_value};
use super::policy::{FreshnessPolicy, RepresentationClass, SharedCachePolicy};
use super::store::PublicationFence;
use super::variance::VarianceDescriptor;
use super::{RenderCacheError, RenderCacheErrorKind};

/// Content type served when a stored entry declares none of its own.
const CONTENT_TYPE_FALLBACK: &str = "application/octet-stream";
/// Headers formed at serve time on top of the stored ones: content type,
/// entity tag, cache control, vary, age, and warning.
const FORMED_HEADERS: usize = 6;

/// Every header value a response can precompute, formed once from one
/// [`ResponseParts`].
struct FormedValues {
    status: StatusCode,
    etag: HeaderValue,
    etag_text: String,
    content_type: HeaderValue,
    /// `None` only when the value depends on the request instant, which is
    /// the case for a body with an embedded public seed deadline.
    cache_control: Option<HeaderValue>,
    vary: Option<HeaderValue>,
    extra: Vec<(HeaderName, HeaderValue)>,
}

impl FormedValues {
    /// Pays, once at publication, the single promotion `bytes` performs the
    /// first time a vector-backed buffer is cloned. Every later clone is
    /// then a reference-count increment, so a hit allocates nothing for the
    /// values it replays; dropping the clone here leaves the original in
    /// the promoted representation. It is a cost move, never a correctness
    /// requirement: the values are identical either way.
    fn promote(&self) {
        drop(self.etag.clone());
        drop(self.content_type.clone());
        if let Some(value) = &self.cache_control {
            drop(value.clone());
        }
        if let Some(value) = &self.vary {
            drop(value.clone());
        }
        for (_, value) in &self.extra {
            drop(value.clone());
        }
    }
}

/// The precomputed values with the facts a request-time value needs,
/// borrowed. [`HotEntry`] owns them; [`respond`] builds them into locals.
struct Formed<'a> {
    values: &'a FormedValues,
    class: RepresentationClass,
    shared: SharedCachePolicy,
    freshness: &'a FreshnessPolicy,
    seed_deadline_ms: Option<u64>,
    published_at_ms: u64,
}

/// Splits stored safe headers into the content type (falling back to
/// [`CONTENT_TYPE_FALLBACK`]) and the remaining replayable pairs. The three
/// headers this module forms itself (content type, cache control, and
/// vary) never reach the remainder, so a stored copy can never be replayed
/// beside the formed one.
fn split_headers(
    headers: &SafeHeaders,
) -> Result<(HeaderValue, Vec<(HeaderName, HeaderValue)>), RenderCacheError> {
    let invalid = || RenderCacheError::new(RenderCacheErrorKind::EntryInvalid);
    let mut content_type = None;
    let mut extra = Vec::new();
    for (name, value) in headers.iter() {
        match name {
            "content-type" => {
                content_type = Some(HeaderValue::from_str(value).map_err(|_| invalid())?);
            }
            "cache-control" | "vary" => {}
            _ => extra.push((
                HeaderName::from_bytes(name.as_bytes()).map_err(|_| invalid())?,
                HeaderValue::from_str(value).map_err(|_| invalid())?,
            )),
        }
    }
    let content_type = match content_type {
        Some(value) => value,
        None => HeaderValue::from_static(CONTENT_TYPE_FALLBACK),
    };
    Ok((content_type, extra))
}

/// Forms every precomputable value for one response. Fails closed when a
/// status or a stored header name or value is not representable on the
/// wire, so an entry that could only be served malformed is never served.
fn form_values(parts: &ResponseParts<'_>) -> Result<FormedValues, RenderCacheError> {
    let invalid = || RenderCacheError::new(RenderCacheErrorKind::EntryInvalid);
    let status = StatusCode::from_u16(parts.status).map_err(|_| invalid())?;
    let etag_text = parts.validator.etag();
    let etag = HeaderValue::from_str(&etag_text).map_err(|_| invalid())?;
    let (content_type, extra) = split_headers(parts.headers)?;
    let cache_control = match (parts.cache_control_override, parts.seed_deadline_ms) {
        (Some(fixed), _) => Some(HeaderValue::from_str(fixed).map_err(|_| invalid())?),
        // A public seed's remaining lifetime shrinks with the request
        // instant, so that one value is formed per request instead.
        (None, Some(_)) => None,
        (None, None) => Some(
            HeaderValue::from_str(&cache_control_value(
                parts.class,
                parts.shared,
                parts.freshness,
                None,
            ))
            .map_err(|_| invalid())?,
        ),
    };
    let vary = match vary_value(parts.variance) {
        Some(value) => Some(HeaderValue::from_str(&value).map_err(|_| invalid())?),
        None => None,
    };
    Ok(FormedValues {
        status,
        etag,
        etag_text,
        content_type,
        cache_control,
        vary,
        extra,
    })
}

/// The one response builder: conditional evaluation, body treatment, header
/// order, and status for both public entry points.
fn build(
    formed: &Formed<'_>,
    body: &Bytes,
    request: HotRequest<'_>,
    warning: Option<&'static str>,
) -> Response<Bytes> {
    let values = formed.values;
    let not_modified = matches!(
        conditional_matches(request.if_none_match, &values.etag_text),
        ConditionalOutcome::NotModified
    );
    let body = if not_modified || request.method == Method::HEAD {
        Bytes::new()
    } else {
        body.clone()
    };
    let mut headers = HeaderMap::with_capacity(FORMED_HEADERS + values.extra.len());
    headers.insert(http::header::CONTENT_TYPE, values.content_type.clone());
    for (name, value) in &values.extra {
        headers.append(name.clone(), value.clone());
    }
    headers.insert(http::header::ETAG, values.etag.clone());
    match &values.cache_control {
        Some(value) => {
            headers.insert(http::header::CACHE_CONTROL, value.clone());
        }
        None => {
            let remaining = formed
                .seed_deadline_ms
                .map(|deadline| deadline.saturating_sub(request.now_ms));
            let value =
                cache_control_value(formed.class, formed.shared, formed.freshness, remaining);
            if let Ok(value) = HeaderValue::from_str(&value) {
                headers.insert(http::header::CACHE_CONTROL, value);
            }
        }
    }
    if let Some(vary) = &values.vary {
        headers.insert(http::header::VARY, vary.clone());
    }
    headers.insert(
        http::header::AGE,
        HeaderValue::from(age_seconds(formed.published_at_ms, request.now_ms)),
    );
    if let Some(warning) = warning {
        headers.insert(http::header::WARNING, HeaderValue::from_static(warning));
    }
    let mut response = Response::new(body);
    *response.status_mut() = if not_modified {
        StatusCode::NOT_MODIFIED
    } else {
        values.status
    };
    *response.headers_mut() = headers;
    response
}

/// A Complete entry prepared for hot service. See the module doc.
pub struct HotEntry {
    entry: CompleteEntry,
    fence: PublicationFence,
    published_at_ms: u64,
    shared: SharedCachePolicy,
    freshness: FreshnessPolicy,
    values: FormedValues,
}

/// Written by hand rather than derived: [`CompleteEntry`]'s own `Debug`
/// prints the body, and a prepared entry's debug form is a diagnostic, not
/// a place to spill stored bytes. Lengths and counts stand in for content.
impl std::fmt::Debug for HotEntry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HotEntry")
            .field("status", &self.values.status.as_u16())
            .field("class", &self.entry.header().class)
            .field("published_at_ms", &self.published_at_ms)
            .field("fence_epoch", &self.fence.epoch)
            .field("fence_token", &self.fence.token)
            .field("seed_deadline_ms", &self.entry.header().seed_deadline_ms)
            .field("body_bytes", &self.entry.body().len())
            .field("replayed_headers", &self.values.extra.len())
            .finish()
    }
}

impl HotEntry {
    /// Forms every precomputable header value. Fails closed when a stored
    /// header name or value is not representable as an HTTP header.
    pub fn prepare(
        entry: CompleteEntry,
        shared: SharedCachePolicy,
        freshness: &FreshnessPolicy,
        published_at_ms: u64,
        fence: PublicationFence,
    ) -> Result<Self, RenderCacheError> {
        let values = {
            let header = entry.header();
            form_values(&ResponseParts {
                status: header.status,
                class: header.class,
                shared,
                freshness,
                headers: &header.headers,
                variance: &header.variance,
                validator: entry.validator(),
                body: entry.body(),
                published_at_ms,
                seed_deadline_ms: header.seed_deadline_ms,
                cache_control_override: None,
            })?
        };
        values.promote();
        // The body is shared with every response this entry ever serves, so
        // its one-time promotion belongs here too. See `FormedValues::promote`.
        drop(entry.body().clone());
        Ok(Self {
            entry,
            fence,
            published_at_ms,
            shared,
            freshness: *freshness,
            values,
        })
    }

    /// The decoded entry.
    #[must_use]
    pub fn entry(&self) -> &CompleteEntry {
        &self.entry
    }

    /// The fence it was published under.
    #[must_use]
    pub const fn fence(&self) -> PublicationFence {
        self.fence
    }

    /// Its publication instant.
    #[must_use]
    pub const fn published_at_ms(&self) -> u64 {
        self.published_at_ms
    }

    /// The borrowed view [`build`] works from. Class and seed deadline are
    /// read back off the entry's own header rather than copied into this
    /// struct, so nothing here can drift from the entry it prepared.
    fn formed(&self) -> Formed<'_> {
        Formed {
            values: &self.values,
            class: self.entry.header().class,
            shared: self.shared,
            freshness: &self.freshness,
            seed_deadline_ms: self.entry.header().seed_deadline_ms,
            published_at_ms: self.published_at_ms,
        }
    }
}

/// What a hit needs from the request.
#[derive(Clone, Copy, Debug)]
pub struct HotRequest<'a> {
    /// The request method; `HEAD` is served body-free.
    pub method: &'a Method,
    /// The raw `If-None-Match` value, if sent.
    pub if_none_match: Option<&'a str>,
    /// The instant freshness is evaluated at.
    pub now_ms: u64,
}

/// Forms the response for a hot hit. Allocates at most the header map and
/// the `Age` value; the body is the stored `Bytes`, shared, and every other
/// header value was formed at publication. The one exception is an entry
/// whose body embeds a public seed deadline: its `Cache-Control` shrinks
/// with the clock, so that single value is formed per request.
///
/// `warning` is an engine constant, such as the one
/// [`super::coherence::warning_header`] returns, and must be a valid header
/// value.
#[must_use]
pub fn serve_hot(
    entry: &HotEntry,
    request: HotRequest<'_>,
    warning: Option<&'static str>,
) -> Response<Bytes> {
    build(&entry.formed(), entry.entry.body(), request, warning)
}

/// Everything [`respond`] needs for bytes that are not a hot entry.
pub struct ResponseParts<'a> {
    /// Final status.
    pub status: u16,
    /// Effective class.
    pub class: RepresentationClass,
    /// The route's shared-cache policy.
    pub shared: SharedCachePolicy,
    /// The route's freshness policy.
    pub freshness: &'a FreshnessPolicy,
    /// Replayable safe headers.
    pub headers: &'a SafeHeaders,
    /// Declared variance.
    pub variance: &'a VarianceDescriptor,
    /// The represented-byte validator.
    pub validator: &'a Validator,
    /// The body.
    pub body: &'a Bytes,
    /// Publication instant.
    pub published_at_ms: u64,
    /// Seed deadline, if the body embeds a public seed.
    pub seed_deadline_ms: Option<u64>,
    /// A fixed `Cache-Control` that replaces the computed one (a slotted
    /// Composite assembly is `private, no-store`).
    pub cache_control_override: Option<&'static str>,
}

/// Written by hand rather than derived, for the reason [`HotEntry`]'s own
/// `Debug` gives: the body is a length here, never bytes.
impl std::fmt::Debug for ResponseParts<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResponseParts")
            .field("status", &self.status)
            .field("class", &self.class)
            .field("shared", &self.shared)
            .field("published_at_ms", &self.published_at_ms)
            .field("seed_deadline_ms", &self.seed_deadline_ms)
            .field("cache_control_override", &self.cache_control_override)
            .field("body_bytes", &self.body.len())
            .finish()
    }
}

/// Forms the response for `parts` with the same rules as [`serve_hot`],
/// through the same builder. Allocates per header; use [`serve_hot`] on the
/// hot path.
///
/// `warning` carries the same requirement [`serve_hot`] states.
pub fn respond(
    parts: ResponseParts<'_>,
    request: HotRequest<'_>,
    warning: Option<&'static str>,
) -> Result<Response<Bytes>, RenderCacheError> {
    let values = form_values(&parts)?;
    let formed = Formed {
        values: &values,
        class: parts.class,
        shared: parts.shared,
        freshness: parts.freshness,
        seed_deadline_ms: parts.seed_deadline_ms,
        published_at_ms: parts.published_at_ms,
    };
    Ok(build(&formed, parts.body, request, warning))
}
