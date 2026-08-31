//! Projection of a checked document render into Suprnova's HTTP response type.

use std::error::Error;
use std::fmt;

use super::{DocumentCachePolicy, DocumentMediaType, DocumentRender};
use crate::HttpResponse;

/// Closed failure classes produced while adapting checked document metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DocumentResponseErrorKind {
    /// A typed HTTP header could not be represented by the framework's text header API.
    NonTextHeaderValue,
}

/// Redacted failure produced while adapting a checked document response.
#[derive(Debug)]
pub struct DocumentResponseError {
    kind: DocumentResponseErrorKind,
    source: http::header::ToStrError,
}

impl DocumentResponseError {
    /// Returns the closed adapter failure class.
    #[must_use]
    pub const fn kind(&self) -> DocumentResponseErrorKind {
        self.kind
    }
}

impl fmt::Display for DocumentResponseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            DocumentResponseErrorKind::NonTextHeaderValue => {
                formatter.write_str("document_header_value_is_not_text")
            }
        }
    }
}

impl Error for DocumentResponseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

/// Converts a completely validated document render into Suprnova's HTTP response.
///
/// Public cache intent remains private at this boundary until RenderCache proves a
/// concrete response safe for bounded shared caching. `NoStore` is preserved
/// immediately, while all checked end-to-end headers are copied without granting
/// templates authority over host-owned transport headers.
pub fn document_response(document: DocumentRender) -> Result<HttpResponse, DocumentResponseError> {
    let content_type = match document.response.media_type() {
        DocumentMediaType::HtmlUtf8 => "text/html; charset=utf-8",
    };
    let cache_control = match document.response.cache() {
        DocumentCachePolicy::NoStore => "no-store",
        DocumentCachePolicy::Private | DocumentCachePolicy::Public => "private",
    };
    let mut response = HttpResponse::bytes_body(document.body, content_type)
        .status(document.response.status().as_u16())
        .replace_header("Cache-Control", cache_control);

    for (name, value) in document.response.headers() {
        let value = value.to_str().map_err(|source| DocumentResponseError {
            kind: DocumentResponseErrorKind::NonTextHeaderValue,
            source,
        })?;
        response = response.header(name.as_str(), value);
    }

    Ok(response)
}
