//! Typed non-authoritative browser facts admitted only after request validation.

use std::fmt;

use crate::canonical::CanonicalValue;
use crate::mount::DocumentMountKey;

use super::{ProtocolError, ProtocolErrorKind, VersionedUpdateRequest};

/// Reserved semantic extension carrying one server-emitted document-local root key.
pub const DOCUMENT_KEY_EXTENSION_V1: &str = "x_suprnova_live_document_key_v1";

/// Inert browser presentation context that can never select server authority.
#[derive(Clone, Eq, PartialEq)]
pub struct BrowserRenderContext {
    document_key: DocumentMountKey,
}

impl BrowserRenderContext {
    /// Parses the reserved extension and binds it to the independently expected island key.
    pub fn from_request(
        request: &VersionedUpdateRequest,
        expected: &DocumentMountKey,
    ) -> Result<Self, ProtocolError> {
        let extensions = match request {
            VersionedUpdateRequest::V1(request) => request.extensions(),
            VersionedUpdateRequest::V2(request) => request.extensions(),
        };
        let Some(CanonicalValue::String(candidate)) = extensions.get(DOCUMENT_KEY_EXTENSION_V1)
        else {
            return Err(ProtocolError::new(ProtocolErrorKind::InvalidExtension));
        };
        Self::checked(candidate, expected)
    }

    /// Checks an untrusted key against independently selected document-local presentation facts.
    pub fn checked(candidate: &str, expected: &DocumentMountKey) -> Result<Self, ProtocolError> {
        let document_key = DocumentMountKey::parse(candidate)
            .map_err(|_| ProtocolError::new(ProtocolErrorKind::InvalidExtension))?;
        if &document_key != expected {
            return Err(ProtocolError::new(ProtocolErrorKind::InvalidExtension));
        }
        Ok(Self { document_key })
    }

    /// Returns the validated inert key suitable only for successor-root presentation.
    #[must_use]
    pub const fn document_key(&self) -> &DocumentMountKey {
        &self.document_key
    }
}

impl fmt::Debug for BrowserRenderContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<BrowserRenderContext:inert>")
    }
}
