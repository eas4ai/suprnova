//! Canonical document result and host-neutral HTTP conformance projection.

use bytes::Bytes;
use http::header::{
    CACHE_CONTROL, CONNECTION, CONTENT_LENGTH, CONTENT_TYPE, SET_COOKIE, TE, TRAILER,
    TRANSFER_ENCODING, UPGRADE,
};
use http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use sha2::{Digest as _, Sha256};
use std::fmt;

use super::{AssetSet, MountMetadata, ViewError, ViewErrorKind};

const MAX_HEADERS: usize = 64;
const MAX_HEADER_BYTES: usize = 16 * 1024;

/// Typed document cache intent retained for the host adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DocumentCachePolicy {
    /// Shared caches must not store the response.
    Private,
    /// No cache may store the response.
    NoStore,
    /// Cacheability is delegated to a later bounded host policy.
    Public,
}

/// Media type of a canonical document representation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DocumentMediaType {
    /// UTF-8 HTML document bytes.
    HtmlUtf8,
}

/// Document-only response intent; islands cannot construct or carry this type.
#[derive(Clone)]
pub struct DocumentResponseIntent {
    status: StatusCode,
    headers: HeaderMap,
    cache: DocumentCachePolicy,
    media_type: DocumentMediaType,
}

impl fmt::Debug for DocumentResponseIntent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DocumentResponseIntent")
            .field("status", &self.status)
            .field("header_count", &self.headers.len())
            .field("cache", &self.cache)
            .field("media_type", &self.media_type)
            .finish()
    }
}

impl DocumentResponseIntent {
    /// Creates HTML response intent for a body-bearing non-informational status.
    pub fn html(status: StatusCode) -> Result<Self, ViewError> {
        if status.is_informational()
            || matches!(status, StatusCode::NO_CONTENT | StatusCode::NOT_MODIFIED)
        {
            return Err(ViewError::new(ViewErrorKind::ForbiddenResponseIntent));
        }
        Ok(Self {
            status,
            headers: HeaderMap::new(),
            cache: DocumentCachePolicy::Private,
            media_type: DocumentMediaType::HtmlUtf8,
        })
    }

    /// Adds one bounded end-to-end header outside host-owned transport fields.
    pub fn with_header(mut self, name: HeaderName, value: HeaderValue) -> Result<Self, ViewError> {
        if forbidden_header(&name)
            || self.headers.len() >= MAX_HEADERS
            || header_bytes(&self.headers)
                .saturating_add(name.as_str().len())
                .saturating_add(value.as_bytes().len())
                > MAX_HEADER_BYTES
        {
            return Err(ViewError::new(ViewErrorKind::ForbiddenResponseIntent));
        }
        self.headers.insert(name, value);
        Ok(self)
    }

    /// Replaces the typed document cache intent.
    #[must_use]
    pub const fn with_cache(mut self, cache: DocumentCachePolicy) -> Self {
        self.cache = cache;
        self
    }

    /// Returns the desired document status.
    #[must_use]
    pub const fn status(&self) -> StatusCode {
        self.status
    }

    /// Returns bounded end-to-end document headers.
    #[must_use]
    pub const fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Returns the typed cache intent.
    #[must_use]
    pub const fn cache(&self) -> DocumentCachePolicy {
        self.cache
    }

    /// Returns the canonical document media type.
    #[must_use]
    pub const fn media_type(&self) -> DocumentMediaType {
        self.media_type
    }

    const fn with_projected_status(mut self, status: StatusCode) -> Self {
        self.status = status;
        self
    }
}

fn forbidden_header(name: &HeaderName) -> bool {
    matches!(
        name,
        &CONNECTION
            | &CONTENT_LENGTH
            | &CONTENT_TYPE
            | &SET_COOKIE
            | &TE
            | &TRAILER
            | &TRANSFER_ENCODING
            | &UPGRADE
            | &CACHE_CONTROL
    )
}

fn header_bytes(headers: &HeaderMap) -> usize {
    headers
        .iter()
        .map(|(name, value)| name.as_str().len().saturating_add(value.as_bytes().len()))
        .sum()
}

/// Successful complete-document render produced only after all validation.
#[derive(Clone)]
pub struct DocumentRender {
    /// Complete canonical HTML bytes.
    pub body: Bytes,
    /// Typed route-owned response metadata.
    pub response: DocumentResponseIntent,
    /// Deterministically ordered asset requirements.
    pub assets: AssetSet,
    /// Inert metadata for initial independently owned islands.
    pub mounts: Vec<MountMetadata>,
}

impl fmt::Debug for DocumentRender {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DocumentRender")
            .field("body_bytes", &self.body.len())
            .field("response", &self.response)
            .field("assets", &self.assets)
            .field("mount_count", &self.mounts.len())
            .finish()
    }
}

/// Content-derived validator used only by the standalone conformance adapter.
#[derive(Clone, Eq, PartialEq)]
pub struct DocumentValidator([u8; 32]);

impl fmt::Debug for DocumentValidator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<DocumentValidator>")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ConformanceMethod {
    Get,
    Head,
}

/// Normalized request facts accepted by the standalone document conformance adapter.
#[derive(Clone, Debug)]
pub struct CanonicalDocumentRequest {
    method: ConformanceMethod,
    if_none_match: Option<DocumentValidator>,
}

impl CanonicalDocumentRequest {
    /// Creates a canonical GET projection request.
    #[must_use]
    pub const fn get(if_none_match: Option<DocumentValidator>) -> Self {
        Self {
            method: ConformanceMethod::Get,
            if_none_match,
        }
    }

    /// Creates a canonical HEAD projection request.
    #[must_use]
    pub const fn head(if_none_match: Option<DocumentValidator>) -> Self {
        Self {
            method: ConformanceMethod::Head,
            if_none_match,
        }
    }
}

/// Host-neutral projection proving canonical document response semantics.
#[derive(Clone)]
pub struct CanonicalDocumentIntent {
    response: DocumentResponseIntent,
    body: Bytes,
    representation_length: usize,
    validator: DocumentValidator,
}

impl fmt::Debug for CanonicalDocumentIntent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CanonicalDocumentIntent")
            .field("response", &self.response)
            .field("body_bytes", &self.body.len())
            .field("representation_length", &self.representation_length)
            .field("validator", &self.validator)
            .finish()
    }
}

impl CanonicalDocumentIntent {
    /// Returns the projected status.
    #[must_use]
    pub const fn status(&self) -> StatusCode {
        self.response.status()
    }

    /// Returns the projected document response metadata.
    #[must_use]
    pub const fn response(&self) -> &DocumentResponseIntent {
        &self.response
    }

    /// Returns GET bytes or an empty body for HEAD/not-modified intent.
    #[must_use]
    pub const fn body(&self) -> &Bytes {
        &self.body
    }

    /// Returns the complete representation length even when the body is suppressed.
    #[must_use]
    pub const fn representation_length(&self) -> usize {
        self.representation_length
    }

    /// Returns the deterministic content validator.
    #[must_use]
    pub const fn validator(&self) -> &DocumentValidator {
        &self.validator
    }
}

/// Standalone conformance adapter; this is deliberately not a Suprnova route.
#[derive(Clone, Copy, Debug, Default)]
pub struct CanonicalDocumentConformance;

impl CanonicalDocumentConformance {
    /// Projects GET, HEAD, and conditional response intent from a validated render.
    #[must_use]
    pub fn project(
        render: &DocumentRender,
        request: &CanonicalDocumentRequest,
    ) -> CanonicalDocumentIntent {
        let validator = DocumentValidator(Sha256::digest(&render.body).into());
        let not_modified = request.if_none_match.as_ref() == Some(&validator);
        let body = if not_modified || request.method == ConformanceMethod::Head {
            Bytes::new()
        } else {
            render.body.clone()
        };
        let status = if not_modified {
            StatusCode::NOT_MODIFIED
        } else {
            render.response.status()
        };
        CanonicalDocumentIntent {
            response: render.response.clone().with_projected_status(status),
            body,
            representation_length: render.body.len(),
            validator,
        }
    }
}
