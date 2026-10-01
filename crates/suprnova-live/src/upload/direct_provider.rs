//! Constrained capabilities for provider-neutral direct-to-storage transfer.

use std::{collections::HashSet, fmt};

use http::{HeaderValue, Uri, header::HeaderName};
use zeroize::Zeroizing;

use crate::identity::UnixMillis;

use super::{UploadError, UploadErrorKind, UploadHandle};

const MAX_PROVIDER_URL_BYTES: usize = 2_048;
const MAX_DIRECT_HEADERS: usize = 16;
const MAX_DIRECT_HEADER_NAME_BYTES: usize = 64;
const MAX_DIRECT_HEADER_VALUE_BYTES: usize = 1_024;
const MAX_DIRECT_HEADER_BYTES: usize = 8 * 1_024;
const MAX_DIRECT_INSTRUCTION_LIFETIME_MS: u64 = 15 * 60 * 1_000;
const DIRECT_PART_REFERENCE_BYTES: usize = 32;

/// Preconfigured HTTPS origin allowed to receive direct-upload bytes.
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct TrustedProviderOrigin {
    encoded: String,
}

impl TrustedProviderOrigin {
    /// Parses an HTTPS origin without credentials, path, query, or fragment.
    pub fn parse(value: &str) -> Result<Self, UploadError> {
        if value.is_empty()
            || value.len() > MAX_PROVIDER_URL_BYTES
            || value.contains(['#', '?', '@', '\\'])
        {
            return Err(invalid_field());
        }
        let uri = value.parse::<Uri>().map_err(|_| invalid_field())?;
        let scheme = uri.scheme_str().ok_or_else(invalid_field)?;
        let authority = uri.authority().ok_or_else(invalid_field)?;
        let path = uri.path_and_query().map(|value| value.as_str());
        if scheme != "https" || authority.host().is_empty() || !matches!(path, None | Some("/")) {
            return Err(invalid_field());
        }
        Ok(Self {
            encoded: format!("https://{authority}"),
        })
    }

    fn permits(&self, uri: &Uri) -> bool {
        let Some(authority) = uri.authority() else {
            return false;
        };
        uri.scheme_str() == Some("https")
            && self.encoded == format!("https://{authority}")
            && !authority.as_str().contains('@')
    }

    /// Returns the explicitly configured origin for host-side serialization.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.encoded
    }
}

impl fmt::Debug for TrustedProviderOrigin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<TrustedProviderOrigin:redacted>")
    }
}

/// HTTPS direct-transfer endpoint proven to match one configured provider origin.
#[derive(Clone, Eq, PartialEq)]
pub struct TrustedProviderUrl {
    encoded: Zeroizing<String>,
}

impl TrustedProviderUrl {
    /// Parses and origin-binds a bounded absolute HTTPS endpoint.
    pub fn parse(value: &str, origin: &TrustedProviderOrigin) -> Result<Self, UploadError> {
        if value.is_empty() || value.len() > MAX_PROVIDER_URL_BYTES || value.contains(['#', '\\']) {
            return Err(invalid_field());
        }
        let uri = value.parse::<Uri>().map_err(|_| invalid_field())?;
        if !origin.permits(&uri) || uri.path_and_query().is_none() || !uri.path().starts_with('/') {
            return Err(invalid_field());
        }
        Ok(Self {
            encoded: Zeroizing::new(value.to_owned()),
        })
    }

    /// Returns the explicitly authorized endpoint for transfer serialization.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.encoded.as_str()
    }
}

impl fmt::Debug for TrustedProviderUrl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<TrustedProviderUrl:redacted>")
    }
}

/// Closed direct-transfer HTTP method vocabulary.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TransferMethod {
    /// Upload exactly one part with HTTP `PUT`.
    Put,
}

impl TransferMethod {
    /// Returns the canonical HTTP method token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Put => "PUT",
        }
    }
}

/// Bounded exact headers required by one direct-transfer instruction.
#[derive(Clone, Eq, PartialEq)]
pub struct BoundedHeaders {
    values: Vec<(HeaderName, Zeroizing<String>)>,
}

impl BoundedHeaders {
    /// Parses a small duplicate-free set while rejecting ambient or hop-by-hop headers.
    pub fn parse(values: &[(&str, &str)]) -> Result<Self, UploadError> {
        if values.len() > MAX_DIRECT_HEADERS {
            return Err(invalid_field());
        }
        let mut total = 0_usize;
        let mut names = HashSet::with_capacity(values.len());
        let mut parsed = Vec::with_capacity(values.len());
        for (name, value) in values {
            if name.len() > MAX_DIRECT_HEADER_NAME_BYTES
                || value.len() > MAX_DIRECT_HEADER_VALUE_BYTES
            {
                return Err(invalid_field());
            }
            let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| invalid_field())?;
            if forbidden_header(&name) || !names.insert(name.clone()) {
                return Err(invalid_field());
            }
            let header_value = HeaderValue::from_str(value).map_err(|_| invalid_field())?;
            total = total
                .checked_add(name.as_str().len())
                .and_then(|total| total.checked_add(header_value.as_bytes().len()))
                .ok_or_else(invalid_field)?;
            if total > MAX_DIRECT_HEADER_BYTES {
                return Err(invalid_field());
            }
            parsed.push((name, Zeroizing::new((*value).to_owned())));
        }
        Ok(Self { values: parsed })
    }

    /// Iterates the exact host-validated headers for controlled serialization.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&HeaderName, &str)> {
        self.values
            .iter()
            .map(|(name, value)| (name, value.as_str()))
    }
}

impl fmt::Debug for BoundedHeaders {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BoundedHeaders")
            .field("count", &self.values.len())
            .finish_non_exhaustive()
    }
}

/// One nonempty, nonoverflowing byte range in a direct transfer.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct UploadPart {
    index: u32,
    offset: u64,
    bytes: u64,
}

impl UploadPart {
    /// Creates a nonempty part with a representable exclusive end offset.
    pub fn new(index: u32, offset: u64, bytes: u64) -> Result<Self, UploadError> {
        if bytes == 0 || offset.checked_add(bytes).is_none() {
            return Err(invalid_field());
        }
        Ok(Self {
            index,
            offset,
            bytes,
        })
    }

    /// Returns the zero-based part index.
    #[must_use]
    pub const fn index(&self) -> u32 {
        self.index
    }

    /// Returns the authoritative byte offset.
    #[must_use]
    pub const fn offset(&self) -> u64 {
        self.offset
    }

    /// Returns the exact part byte count.
    #[must_use]
    pub const fn bytes(&self) -> u64 {
        self.bytes
    }
}

/// Opaque non-authoritative identity binding a reported provider part to its upload.
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct DirectPartReference(String);

impl DirectPartReference {
    /// Parses exactly 128 bits of canonical lowercase hexadecimal identity.
    pub fn parse(value: &str) -> Result<Self, UploadError> {
        let valid = value.len() == DIRECT_PART_REFERENCE_BYTES
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'));
        if !valid {
            return Err(invalid_field());
        }
        Ok(Self(value.to_owned()))
    }

    /// Returns the provider-part identity for controlled protocol serialization.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for DirectPartReference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<DirectPartReference:redacted>")
    }
}

/// One short-lived, provider-origin-bound, method-bound, byte-bound instruction.
#[derive(Clone, Eq, PartialEq)]
pub struct DirectTransferInstruction {
    method: TransferMethod,
    endpoint: TrustedProviderUrl,
    required_headers: BoundedHeaders,
    part: UploadPart,
    reference: DirectPartReference,
    expires_at: UnixMillis,
    maximum_bytes: usize,
}

impl DirectTransferInstruction {
    /// Constructs an instruction whose lifetime and byte authority are strictly bounded.
    #[allow(
        clippy::too_many_arguments,
        reason = "all authority dimensions are explicit"
    )]
    pub fn new(
        method: TransferMethod,
        endpoint: TrustedProviderUrl,
        required_headers: BoundedHeaders,
        part: UploadPart,
        reference: DirectPartReference,
        issued_at: UnixMillis,
        expires_at: UnixMillis,
        maximum_bytes: usize,
    ) -> Result<Self, UploadError> {
        let lifetime = expires_at
            .get()
            .checked_sub(issued_at.get())
            .ok_or_else(invalid_field)?;
        let part_bytes = usize::try_from(part.bytes()).map_err(|_| invalid_field())?;
        if lifetime == 0
            || lifetime > MAX_DIRECT_INSTRUCTION_LIFETIME_MS
            || maximum_bytes == 0
            || maximum_bytes != part_bytes
        {
            return Err(invalid_field());
        }
        Ok(Self {
            method,
            endpoint,
            required_headers,
            part,
            reference,
            expires_at,
            maximum_bytes,
        })
    }

    /// Returns the exact allowed HTTP method.
    #[must_use]
    pub const fn method(&self) -> TransferMethod {
        self.method
    }

    /// Returns the origin-bound endpoint for controlled transfer serialization.
    #[must_use]
    pub const fn endpoint(&self) -> &TrustedProviderUrl {
        &self.endpoint
    }

    /// Returns the exact bounded header set.
    #[must_use]
    pub const fn required_headers(&self) -> &BoundedHeaders {
        &self.required_headers
    }

    /// Returns the exact part range.
    #[must_use]
    pub const fn part(&self) -> &UploadPart {
        &self.part
    }

    /// Returns the non-authoritative provider-part reference.
    #[must_use]
    pub const fn reference(&self) -> &DirectPartReference {
        &self.reference
    }

    /// Returns the exclusive instruction expiry instant.
    #[must_use]
    pub const fn expires_at(&self) -> UnixMillis {
        self.expires_at
    }

    /// Returns the exact body byte ceiling.
    #[must_use]
    pub const fn maximum_bytes(&self) -> usize {
        self.maximum_bytes
    }

    /// Returns whether the instruction remains current at the supplied instant.
    #[must_use]
    pub fn is_current(&self, now: UnixMillis) -> bool {
        now < self.expires_at
    }

    /// Confirms every authority dimension was established by the checked constructor.
    #[must_use]
    pub const fn is_constrained(&self) -> bool {
        true
    }
}

impl fmt::Debug for DirectTransferInstruction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DirectTransferInstruction")
            .field("method", &self.method)
            .field("part", &self.part)
            .field("expires_at", &self.expires_at)
            .field("maximum_bytes", &self.maximum_bytes)
            .finish_non_exhaustive()
    }
}

/// Provider-neutral instruction emitted by one transfer preparation or part report.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransferInstruction {
    /// Send a bounded request body through the authenticated Live upload route.
    ReverseProxy {
        /// Exact server-enforced body ceiling.
        maximum_bytes: usize,
    },
    /// Send bytes directly to the constrained provider capability.
    Direct(DirectTransferInstruction),
}

impl TransferInstruction {
    pub(crate) fn reverse_proxy(maximum_bytes: usize) -> Self {
        Self::ReverseProxy { maximum_bytes }
    }

    /// Returns whether every instruction authority dimension is bounded.
    #[must_use]
    pub const fn is_constrained(&self) -> bool {
        match self {
            Self::ReverseProxy { maximum_bytes } => *maximum_bytes > 0,
            Self::Direct(instruction) => instruction.is_constrained(),
        }
    }

    /// Borrows the direct capability when this is a direct-provider instruction.
    #[must_use]
    pub const fn as_direct(&self) -> Option<&DirectTransferInstruction> {
        match self {
            Self::ReverseProxy { .. } => None,
            Self::Direct(instruction) => Some(instruction),
        }
    }
}

/// Trusted report that asks a provider adapter to import one stored part outcome.
#[derive(Clone)]
pub struct ReportDirectPart<'a> {
    handle: &'a UploadHandle,
    part: UploadPart,
    reference: DirectPartReference,
    observed_at: UnixMillis,
}

impl<'a> ReportDirectPart<'a> {
    /// Binds provider evidence to its upload, part range, reference, and observation time.
    #[must_use]
    pub const fn new(
        handle: &'a UploadHandle,
        part: UploadPart,
        reference: DirectPartReference,
        observed_at: UnixMillis,
    ) -> Self {
        Self {
            handle,
            part,
            reference,
            observed_at,
        }
    }

    /// Returns the upload whose provider evidence is being imported.
    #[must_use]
    pub const fn handle(&self) -> &UploadHandle {
        self.handle
    }

    /// Returns the exact reported part.
    #[must_use]
    pub const fn part(&self) -> &UploadPart {
        &self.part
    }

    /// Returns the opaque provider-part reference.
    #[must_use]
    pub const fn reference(&self) -> &DirectPartReference {
        &self.reference
    }

    /// Returns the trusted provider observation instant.
    #[must_use]
    pub const fn observed_at(&self) -> UnixMillis {
        self.observed_at
    }
}

impl fmt::Debug for ReportDirectPart<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<ReportDirectPart:redacted>")
    }
}

fn forbidden_header(name: &HeaderName) -> bool {
    name.as_str().starts_with("proxy-")
        || name.as_str().starts_with("sec-")
        || matches!(
            name.as_str(),
            "accept-charset"
                | "accept-encoding"
                | "access-control-request-headers"
                | "access-control-request-method"
                | "connection"
                | "content-length"
                | "cookie"
                | "cookie2"
                | "date"
                | "dnt"
                | "expect"
                | "host"
                | "keep-alive"
                | "origin"
                | "permissions-policy"
                | "referer"
                | "te"
                | "trailer"
                | "transfer-encoding"
                | "upgrade"
                | "user-agent"
                | "via"
                | "x-http-method"
                | "x-http-method-override"
                | "x-method-override"
        )
}

fn invalid_field() -> UploadError {
    UploadError::new(UploadErrorKind::InvalidField)
}
