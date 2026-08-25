//! Authoritative bounded content inspection and scanning contracts.

use std::fmt;
use std::sync::Arc;

use crate::host::TrustedLiveRequestContext;
use crate::identity::{ContentDigest, ModelField, UnixMillis};
use crate::limits::UploadLimits;

use super::{
    AuthoritativeUploadType, QuarantineBytes, ReadUpload, TransferGrantScope, TransitionOutcome,
    UploadChecksum, UploadControlKind, UploadError, UploadErrorKind, UploadFieldPolicy,
    UploadFuture, UploadHandle, UploadIdempotencyKey, UploadMediaType, UploadProvider,
    UploadRevision, UploadScanPolicy, UploadService, UploadState, UploadTransition,
    UploadTransitionRequest, VerifyTransfer,
};

const MAX_CLIENT_NAME_BYTES: usize = 1_024;
const MAX_CLAIMED_MEDIA_TYPE_BYTES: usize = 127;
const PNG_PREFIX_BYTES: usize = 32;
const GIF_PREFIX_BYTES: usize = 16;
const WEBP_PREFIX_BYTES: usize = 64;
const JPEG_PREFIX_BYTES: usize = 256 * 1024;

/// Normalized untrusted display metadata supplied by the selecting browser.
#[derive(Clone, Eq, PartialEq)]
pub struct ClientUploadMetadata {
    display_name: String,
    claimed_media_type: Option<String>,
}

impl ClientUploadMetadata {
    /// Normalizes bounded display metadata while rejecting path-control input.
    pub fn new(display_name: &str, claimed_media_type: Option<&str>) -> Result<Self, UploadError> {
        let display_name = display_name.trim();
        let valid_name = !display_name.is_empty()
            && display_name.len() <= MAX_CLIENT_NAME_BYTES
            && !matches!(display_name, "." | "..")
            && !display_name.contains(['/', '\\'])
            && !display_name.chars().any(char::is_control);
        if !valid_name {
            return Err(UploadError::new(UploadErrorKind::InvalidField));
        }
        let claimed_media_type = claimed_media_type
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| {
                let valid = value.len() <= MAX_CLAIMED_MEDIA_TYPE_BYTES
                    && value.bytes().all(|byte| {
                        byte.is_ascii_lowercase()
                            || byte.is_ascii_digit()
                            || matches!(byte, b'/' | b'+' | b'.' | b'-')
                    })
                    && value.split_once('/').is_some_and(|(kind, subtype)| {
                        !kind.is_empty() && !subtype.is_empty() && !subtype.contains('/')
                    });
                if valid {
                    Ok(value.to_owned())
                } else {
                    Err(UploadError::new(UploadErrorKind::InvalidField))
                }
            })
            .transpose()?;
        Ok(Self {
            display_name: display_name.to_owned(),
            claimed_media_type,
        })
    }

    /// Returns the normalized display-only original name.
    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// Returns the untrusted normalized browser MIME claim, when present.
    #[must_use]
    pub fn claimed_media_type(&self) -> Option<&str> {
        self.claimed_media_type.as_deref()
    }

    pub(crate) fn extension(&self) -> Option<&str> {
        self.display_name
            .rsplit_once('.')
            .and_then(|(stem, extension)| {
                (!stem.is_empty() && !extension.is_empty()).then_some(extension)
            })
    }
}

impl fmt::Debug for ClientUploadMetadata {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClientUploadMetadata")
            .field("display_name", &"<untrusted-display-name>")
            .field("claimed_media_type", &self.claimed_media_type)
            .finish()
    }
}

/// Content type determined from authoritative magic bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DetectedUploadType {
    /// Content is a GIF image.
    Gif,
    /// Content is a JPEG image.
    Jpeg,
    /// Content is a PNG image.
    Png,
    /// Content is a WebP image.
    Webp,
    /// The bounded built-in classifier does not recognize this content.
    Unknown,
}

impl DetectedUploadType {
    /// Returns the built-in media type when recognized.
    #[must_use]
    pub const fn media_type(self) -> Option<UploadMediaType> {
        match self {
            Self::Gif => Some(UploadMediaType::Gif),
            Self::Jpeg => Some(UploadMediaType::Jpeg),
            Self::Png => Some(UploadMediaType::Png),
            Self::Webp => Some(UploadMediaType::Webp),
            Self::Unknown => None,
        }
    }
}

/// Checked image dimensions without decoded pixel data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MediaDimensions {
    width: u32,
    height: u32,
    pixels: u64,
}

impl MediaDimensions {
    /// Creates nonzero dimensions and checks width-times-height arithmetic.
    pub fn new(width: u32, height: u32) -> Result<Self, UploadError> {
        if width == 0 || height == 0 {
            return Err(UploadError::new(UploadErrorKind::MediaHeaderUnproved));
        }
        let pixels = u64::from(width)
            .checked_mul(u64::from(height))
            .ok_or_else(|| UploadError::new(UploadErrorKind::MediaHeaderUnproved))?;
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    /// Returns the width in pixels.
    #[must_use]
    pub const fn width(self) -> u32 {
        self.width
    }

    /// Returns the height in pixels.
    #[must_use]
    pub const fn height(self) -> u32 {
        self.height
    }

    /// Returns the checked width-times-height value.
    #[must_use]
    pub const fn pixels(self) -> u64 {
        self.pixels
    }
}

/// Pure bounded media-header classifier and dimension probe.
pub struct MediaHeaderProbe;

impl MediaHeaderProbe {
    /// Classifies only the four enabled formats from authoritative magic bytes.
    #[must_use]
    pub fn classify(bytes: &[u8]) -> DetectedUploadType {
        if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            DetectedUploadType::Png
        } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
            DetectedUploadType::Gif
        } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
            DetectedUploadType::Jpeg
        } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
            DetectedUploadType::Webp
        } else {
            DetectedUploadType::Unknown
        }
    }

    /// Returns the maximum prefix the engine may supply to `imagesize`.
    #[must_use]
    pub const fn prefix_limit(kind: DetectedUploadType) -> Option<usize> {
        match kind {
            DetectedUploadType::Png => Some(PNG_PREFIX_BYTES),
            DetectedUploadType::Gif => Some(GIF_PREFIX_BYTES),
            DetectedUploadType::Webp => Some(WEBP_PREFIX_BYTES),
            DetectedUploadType::Jpeg => Some(JPEG_PREFIX_BYTES),
            DetectedUploadType::Unknown => None,
        }
    }

    /// Reads dimensions from at most the format-specific cap and never decodes pixels.
    pub fn probe(bytes: &[u8]) -> Result<Option<MediaDimensions>, UploadError> {
        let kind = Self::classify(bytes);
        let Some(limit) = Self::prefix_limit(kind) else {
            return Ok(None);
        };
        let bounded = &bytes[..bytes.len().min(limit)];
        let size = imagesize::blob_size(bounded)
            .map_err(|_| UploadError::new(UploadErrorKind::MediaHeaderUnproved))?;
        let width = u32::try_from(size.width)
            .map_err(|_| UploadError::new(UploadErrorKind::MediaHeaderUnproved))?;
        let height = u32::try_from(size.height)
            .map_err(|_| UploadError::new(UploadErrorKind::MediaHeaderUnproved))?;
        MediaDimensions::new(width, height).map(Some)
    }
}

/// Bounded safe scanner/application rejection code.
#[derive(Clone, Eq, PartialEq)]
pub struct ScanReason(String);

impl ScanReason {
    /// Parses a low-cardinality safe reason code.
    pub fn parse(value: &str) -> Result<Self, UploadError> {
        let valid = !value.is_empty()
            && value.len() <= 64
            && value.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'_' | b'-' | b'.')
            });
        if valid {
            Ok(Self(value.to_owned()))
        } else {
            Err(UploadError::new(UploadErrorKind::InvalidField))
        }
    }

    /// Returns the validated low-cardinality reason code.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ScanReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<ScanReason>")
    }
}

/// Explicit result from one bounded host scanner invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScanDisposition {
    /// The scanner found no prohibited content.
    Clean,
    /// The scanner authoritatively rejected the upload.
    Rejected(ScanReason),
    /// The scanner capability could not currently produce a result.
    Unavailable,
    /// The scanner's host-enforced deadline elapsed.
    TimedOut,
}

/// Explicit application validation result after built-in inspection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApplicationValidationDecision {
    /// Application policy accepts the inspected upload.
    Allow,
    /// A trusted application classifier accepts otherwise-unrecognized bytes as this type.
    ///
    /// The classifier must derive this result from authoritative quarantined content rather
    /// than the browser's MIME or filename claims.
    AllowAs(AuthoritativeUploadType),
    /// Application policy rejects the inspected upload.
    Reject(ScanReason),
}

/// Authoritative, bounded, non-secret upload inspection facts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UploadInspection {
    handle: UploadHandle,
    client: ClientUploadMetadata,
    detected_type: DetectedUploadType,
    authoritative_type: Option<AuthoritativeUploadType>,
    bytes: u64,
    checksum: UploadChecksum,
    dimensions: Option<MediaDimensions>,
    inspected_at: UnixMillis,
}

impl UploadInspection {
    /// Reconstructs immutable evidence loaded by a trusted persistence adapter.
    #[allow(
        clippy::too_many_arguments,
        reason = "persistence reconstructs the complete immutable inspection tuple"
    )]
    pub fn from_store(
        handle: UploadHandle,
        client: ClientUploadMetadata,
        detected_type: DetectedUploadType,
        authoritative_type: Option<AuthoritativeUploadType>,
        bytes: u64,
        checksum: UploadChecksum,
        dimensions: Option<MediaDimensions>,
        inspected_at: UnixMillis,
    ) -> Result<Self, UploadError> {
        let built_in = detected_type
            .media_type()
            .map(AuthoritativeUploadType::from);
        if built_in.as_ref() != authoritative_type.as_ref()
            && detected_type != DetectedUploadType::Unknown
        {
            return Err(UploadError::new(UploadErrorKind::InvalidField));
        }
        if (detected_type == DetectedUploadType::Unknown) != dimensions.is_none() {
            return Err(UploadError::new(UploadErrorKind::InvalidField));
        }
        Ok(Self {
            handle,
            client,
            detected_type,
            authoritative_type,
            bytes,
            checksum,
            dimensions,
            inspected_at,
        })
    }

    /// Returns the opaque temporary upload identity.
    #[must_use]
    pub const fn handle(&self) -> &UploadHandle {
        &self.handle
    }

    /// Returns normalized display-only client metadata.
    #[must_use]
    pub const fn client(&self) -> &ClientUploadMetadata {
        &self.client
    }

    /// Returns the type determined from authoritative magic bytes.
    #[must_use]
    pub const fn detected_type(&self) -> DetectedUploadType {
        self.detected_type
    }

    /// Returns the authoritative built-in or application-classified MIME type.
    #[must_use]
    pub const fn authoritative_type(&self) -> Option<&AuthoritativeUploadType> {
        self.authoritative_type.as_ref()
    }

    /// Returns the authoritative verified byte count.
    #[must_use]
    pub const fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Returns the authoritative verified SHA-256 checksum.
    #[must_use]
    pub const fn checksum(&self) -> &UploadChecksum {
        &self.checksum
    }

    /// Returns bounded image dimensions when recognized.
    #[must_use]
    pub const fn dimensions(&self) -> Option<MediaDimensions> {
        self.dimensions
    }

    /// Returns the authoritative inspection instant.
    #[must_use]
    pub const fn inspected_at(&self) -> UnixMillis {
        self.inspected_at
    }
}

/// Read-only bounded view of authoritative quarantined content.
#[derive(Clone, Copy)]
pub struct UploadContent<'a> {
    provider: &'a dyn UploadProvider,
    handle: &'a UploadHandle,
    total_bytes: u64,
    maximum_read_bytes: usize,
    deadline: UnixMillis,
}

impl<'a> UploadContent<'a> {
    const fn new(
        provider: &'a dyn UploadProvider,
        handle: &'a UploadHandle,
        total_bytes: u64,
        maximum_read_bytes: usize,
        deadline: UnixMillis,
    ) -> Self {
        Self {
            provider,
            handle,
            total_bytes,
            maximum_read_bytes,
            deadline,
        }
    }

    /// Reads at most one configured chunk without exposing provider mutation authority.
    pub fn read(
        &self,
        offset: u64,
        maximum_bytes: usize,
    ) -> UploadFuture<'_, Result<QuarantineBytes, UploadError>> {
        Box::pin(async move {
            if maximum_bytes == 0
                || maximum_bytes > self.maximum_read_bytes
                || offset > self.total_bytes
            {
                return Err(UploadError::new(UploadErrorKind::InputTooLarge));
            }
            let remaining = self.total_bytes - offset;
            let bounded = usize::try_from(remaining.min(maximum_bytes as u64))
                .map_err(|_| UploadError::new(UploadErrorKind::InputTooLarge))?;
            if bounded == 0 {
                return Ok(QuarantineBytes::new());
            }
            let bytes = self
                .provider
                .read(ReadUpload::new(self.handle, offset, bounded))
                .await?;
            if bytes.is_empty() || bytes.len() > bounded {
                return Err(UploadError::new(UploadErrorKind::IncompleteTransfer));
            }
            Ok(bytes)
        })
    }

    /// Returns the exact verified whole-upload size.
    #[must_use]
    pub const fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    /// Returns the per-read response ceiling.
    #[must_use]
    pub const fn maximum_read_bytes(&self) -> usize {
        self.maximum_read_bytes
    }

    /// Returns the absolute host-enforced validation deadline.
    #[must_use]
    pub const fn deadline(&self) -> UnixMillis {
        self.deadline
    }
}

impl fmt::Debug for UploadContent<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<UploadContent:redacted>")
    }
}

/// Scanner input with a host-enforced absolute deadline.
#[derive(Clone, Copy)]
pub struct ScanInput<'a> {
    upload: &'a UploadInspection,
    content: UploadContent<'a>,
    started_at: UnixMillis,
    deadline: UnixMillis,
}

impl<'a> ScanInput<'a> {
    const fn new(
        upload: &'a UploadInspection,
        content: UploadContent<'a>,
        started_at: UnixMillis,
        deadline: UnixMillis,
    ) -> Self {
        Self {
            upload,
            content,
            started_at,
            deadline,
        }
    }

    /// Returns the upload inspection without raw byte material.
    #[must_use]
    pub const fn upload(&self) -> &UploadInspection {
        self.upload
    }

    /// Returns the bounded read-only authoritative content view.
    #[must_use]
    pub const fn content(&self) -> &UploadContent<'a> {
        &self.content
    }

    /// Returns when scan admission began.
    #[must_use]
    pub const fn started_at(&self) -> UnixMillis {
        self.started_at
    }

    /// Returns the absolute deadline the host scanner must enforce.
    #[must_use]
    pub const fn deadline(&self) -> UnixMillis {
        self.deadline
    }
}

impl fmt::Debug for ScanInput<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<ScanInput:redacted>")
    }
}

/// Trusted application-validation input with bounded authoritative content access.
#[derive(Clone, Copy)]
pub struct ApplicationValidationInput<'a> {
    upload: &'a UploadInspection,
    content: UploadContent<'a>,
    started_at: UnixMillis,
    deadline: UnixMillis,
}

impl<'a> ApplicationValidationInput<'a> {
    const fn new(
        upload: &'a UploadInspection,
        content: UploadContent<'a>,
        started_at: UnixMillis,
        deadline: UnixMillis,
    ) -> Self {
        Self {
            upload,
            content,
            started_at,
            deadline,
        }
    }

    /// Returns authoritative inspection facts and untrusted display metadata.
    #[must_use]
    pub const fn upload(&self) -> &UploadInspection {
        self.upload
    }

    /// Returns the bounded read-only authoritative content view.
    #[must_use]
    pub const fn content(&self) -> &UploadContent<'a> {
        &self.content
    }

    /// Returns when validation admission began.
    #[must_use]
    pub const fn started_at(&self) -> UnixMillis {
        self.started_at
    }

    /// Returns the absolute deadline the host validator must enforce.
    #[must_use]
    pub const fn deadline(&self) -> UnixMillis {
        self.deadline
    }
}

impl fmt::Debug for ApplicationValidationInput<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<ApplicationValidationInput:redacted>")
    }
}

/// Host-owned bounded content scanner.
pub trait UploadScanner: Send + Sync {
    /// Scans one authoritative upload without blocking the engine executor.
    fn scan<'a>(
        &'a self,
        input: ScanInput<'a>,
    ) -> UploadFuture<'a, Result<ScanDisposition, UploadError>>;
}

/// Application-specific validation after built-in authoritative inspection.
pub trait UploadApplicationValidator: Send + Sync {
    /// Applies bounded content classification and domain rules under the supplied deadline.
    fn validate<'a>(
        &'a self,
        input: ApplicationValidationInput<'a>,
    ) -> UploadFuture<'a, Result<ApplicationValidationDecision, UploadError>>;
}

/// Stable user-correctable reason an upload did not become ready.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UploadRejectionReason {
    /// Actual bytes did not match the expected or permitted size.
    SizeMismatch,
    /// Actual SHA-256 integrity did not match the completed transfer.
    IntegrityMismatch,
    /// Authoritative type disagreed with field policy or display claims.
    TypeMismatch,
    /// A recognized media header could not prove safe dimensions within its cap.
    MediaHeaderUnproved,
    /// Image width or height exceeded field policy.
    DimensionsExceeded,
    /// Checked width-times-height exceeded field policy.
    PixelsExceeded,
    /// The scanner authoritatively rejected the content.
    ScanRejected,
    /// Scanner timeout policy rejected the content.
    ScanTimedOut,
    /// Scanner-unavailable policy rejected the content.
    ScanUnavailable,
    /// Application-specific authoritative validation rejected the content.
    ApplicationRejected,
}

/// Accepted evidence bound to one exact authority, policy, and Ready revision.
#[derive(Clone, Eq, PartialEq)]
pub struct ValidatedUpload {
    authority: TransferGrantScope,
    ready_revision: UploadRevision,
    policy_digest: ContentDigest,
    inspection: UploadInspection,
}

impl ValidatedUpload {
    /// Reconstructs checked evidence loaded by a trusted persistence adapter.
    pub fn from_store(
        authority: TransferGrantScope,
        ready_revision: UploadRevision,
        policy_digest: ContentDigest,
        inspection: UploadInspection,
    ) -> Result<Self, UploadError> {
        if ready_revision.get() == 0 || authority.handle() != inspection.handle() {
            return Err(UploadError::new(UploadErrorKind::InvalidField));
        }
        Ok(Self {
            authority,
            ready_revision,
            policy_digest,
            inspection,
        })
    }

    /// Returns the temporary upload identity.
    #[must_use]
    pub const fn handle(&self) -> &UploadHandle {
        self.authority.handle()
    }

    /// Returns the complete principal/session/tenant/component/field binding.
    #[must_use]
    pub const fn authority(&self) -> &TransferGrantScope {
        &self.authority
    }

    /// Returns the exact lifecycle revision at which content became Ready.
    #[must_use]
    pub const fn ready_revision(&self) -> UploadRevision {
        self.ready_revision
    }

    /// Returns the semantic field-policy digest used for acceptance.
    #[must_use]
    pub const fn policy_digest(&self) -> &ContentDigest {
        &self.policy_digest
    }

    /// Returns authoritative inspection facts.
    #[must_use]
    pub const fn inspection(&self) -> &UploadInspection {
        &self.inspection
    }
}

impl fmt::Debug for ValidatedUpload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ValidatedUpload")
            .field("authority", &"<redacted>")
            .field("ready_revision", &self.ready_revision)
            .field("policy_digest", &"<digest>")
            .field("inspection", &self.inspection)
            .finish()
    }
}

/// Whether validation evidence was newly persisted or exactly replayed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValidationStoreDisposition {
    /// New immutable validation evidence was stored.
    Stored,
    /// The exact immutable evidence already existed.
    ExistingOutcome,
}

/// Host-owned persistent validation evidence, normally colocated with upload authority.
pub trait UploadValidationStore: Send + Sync {
    /// Persists evidence before the Ready transition; conflicts fail closed.
    fn put<'a>(
        &'a self,
        evidence: ValidatedUpload,
    ) -> UploadFuture<'a, Result<ValidationStoreDisposition, UploadError>>;

    /// Loads immutable evidence for finalization or retry.
    fn load<'a>(
        &'a self,
        handle: &'a UploadHandle,
    ) -> UploadFuture<'a, Result<Option<ValidatedUpload>, UploadError>>;

    /// Removes stale or terminal evidence idempotently.
    fn remove<'a>(&'a self, handle: &'a UploadHandle) -> UploadFuture<'a, Result<(), UploadError>>;
}

/// Trusted request to validate one completed transfer and conditionally accept it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UploadValidationRequest {
    handle: UploadHandle,
    field: ModelField,
    expected_revision: UploadRevision,
    idempotency_key: UploadIdempotencyKey,
    client: ClientUploadMetadata,
    expected_bytes: u64,
    checksum: UploadChecksum,
    policy: UploadFieldPolicy,
}

impl UploadValidationRequest {
    /// Groups trusted lifecycle identity with untrusted display claims and field policy.
    #[allow(
        clippy::too_many_arguments,
        reason = "the validation request is one complete bounded admission tuple"
    )]
    #[must_use]
    pub const fn new(
        handle: UploadHandle,
        field: ModelField,
        expected_revision: UploadRevision,
        idempotency_key: UploadIdempotencyKey,
        client: ClientUploadMetadata,
        expected_bytes: u64,
        checksum: UploadChecksum,
        policy: UploadFieldPolicy,
    ) -> Self {
        Self {
            handle,
            field,
            expected_revision,
            idempotency_key,
            client,
            expected_bytes,
            checksum,
            policy,
        }
    }
}

/// High-level lifecycle result of one validation attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UploadValidationDisposition {
    /// Evidence was persisted and the upload is Ready.
    Ready,
    /// The upload was authoritatively rejected.
    Rejected,
    /// The upload remains Verifying and may be retried.
    Retry,
}

/// Safe validation result with optional accepted evidence or rejection reason.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UploadValidationOutcome {
    disposition: UploadValidationDisposition,
    evidence: Option<ValidatedUpload>,
    reason: Option<UploadRejectionReason>,
    transition: Option<TransitionOutcome>,
}

impl UploadValidationOutcome {
    /// Returns whether validation accepted, rejected, or requested retry.
    #[must_use]
    pub const fn disposition(&self) -> UploadValidationDisposition {
        self.disposition
    }

    /// Returns accepted evidence only for Ready.
    #[must_use]
    pub const fn evidence(&self) -> Option<&ValidatedUpload> {
        self.evidence.as_ref()
    }

    /// Returns the typed reason for rejection or retry.
    #[must_use]
    pub const fn reason(&self) -> Option<UploadRejectionReason> {
        self.reason
    }

    /// Returns the committed lifecycle mutation for Ready or Rejected.
    #[must_use]
    pub const fn transition(&self) -> Option<TransitionOutcome> {
        self.transition
    }
}

/// Authoritative validation coordinator over provider bytes and persistent evidence.
pub struct UploadValidationService {
    authority: Arc<UploadService>,
    provider: Arc<dyn UploadProvider>,
    evidence: Arc<dyn UploadValidationStore>,
    scanner: Option<Arc<dyn UploadScanner>>,
    application: Option<Arc<dyn UploadApplicationValidator>>,
    limits: UploadLimits,
}

impl UploadValidationService {
    /// Creates a finite executor-neutral validation service.
    pub fn new(
        authority: Arc<UploadService>,
        provider: Arc<dyn UploadProvider>,
        evidence: Arc<dyn UploadValidationStore>,
        scanner: Option<Arc<dyn UploadScanner>>,
        application: Option<Arc<dyn UploadApplicationValidator>>,
        limits: UploadLimits,
    ) -> Result<Self, UploadError> {
        if limits.max_validation_ms() == 0
            || limits.max_scan_ms() == 0
            || limits.max_chunk_bytes() == 0
        {
            return Err(UploadError::new(UploadErrorKind::InvalidField));
        }
        Ok(Self {
            authority,
            provider,
            evidence,
            scanner,
            application,
            limits,
        })
    }

    /// Reauthorizes, inspects, scans, persists evidence, and conditionally accepts content.
    pub async fn validate(
        &self,
        context: &TrustedLiveRequestContext,
        request: UploadValidationRequest,
        now: UnixMillis,
    ) -> Result<UploadValidationOutcome, UploadError> {
        let validation_deadline = deadline_after(now, self.limits.max_validation_ms())?;
        let record = self
            .authority
            .trusted_status(
                context,
                request.field.clone(),
                request.handle.clone(),
                UploadControlKind::Status,
                now,
            )
            .await?;
        if record.state() != UploadState::Verifying
            || record.revision() != request.expected_revision
        {
            return Err(UploadError::new(UploadErrorKind::UploadConflict));
        }

        let integrity = match self
            .provider
            .verify(VerifyTransfer::new(&request.handle, &request.checksum))
            .await
        {
            Ok(integrity) => integrity,
            Err(error) if error.kind() == UploadErrorKind::ChecksumMismatch => {
                return self
                    .reject(
                        context,
                        &request,
                        UploadRejectionReason::IntegrityMismatch,
                        now,
                    )
                    .await;
            }
            Err(error) if error.kind() == UploadErrorKind::IncompleteTransfer => {
                return self
                    .reject(context, &request, UploadRejectionReason::SizeMismatch, now)
                    .await;
            }
            Err(error) => return Err(error),
        };
        if integrity.bytes() != request.expected_bytes
            || integrity.bytes() > request.policy.maximum_file_bytes()
            || integrity.bytes() > self.limits.max_file_bytes()
        {
            return self
                .reject(context, &request, UploadRejectionReason::SizeMismatch, now)
                .await;
        }

        let initial = self
            .read_prefix(&request.handle, integrity.bytes(), 12)
            .await?;
        let detected_type = MediaHeaderProbe::classify(&initial);
        let dimensions = if let Some(prefix_limit) = MediaHeaderProbe::prefix_limit(detected_type) {
            let prefix = self
                .read_prefix(&request.handle, integrity.bytes(), prefix_limit)
                .await?;
            match MediaHeaderProbe::probe(&prefix) {
                Ok(dimensions) => dimensions,
                Err(error) if error.kind() == UploadErrorKind::MediaHeaderUnproved => {
                    return self
                        .reject(
                            context,
                            &request,
                            UploadRejectionReason::MediaHeaderUnproved,
                            now,
                        )
                        .await;
                }
                Err(error) => return Err(error),
            }
        } else {
            None
        };
        if let Some(reason) = dimension_rejection(request.policy.dimensions(), dimensions) {
            return self.reject(context, &request, reason, now).await;
        }

        let mut inspection = UploadInspection {
            handle: request.handle.clone(),
            client: request.client.clone(),
            detected_type,
            authoritative_type: detected_type
                .media_type()
                .map(AuthoritativeUploadType::from),
            bytes: integrity.bytes(),
            checksum: integrity.checksum().clone(),
            dimensions,
            inspected_at: now,
        };
        if let Some(application) = &self.application {
            let content = UploadContent::new(
                self.provider.as_ref(),
                inspection.handle(),
                inspection.bytes(),
                self.limits.max_chunk_bytes(),
                validation_deadline,
            );
            match application
                .validate(ApplicationValidationInput::new(
                    &inspection,
                    content,
                    now,
                    validation_deadline,
                ))
                .await?
            {
                ApplicationValidationDecision::Allow => {}
                ApplicationValidationDecision::AllowAs(classified) => {
                    if inspection
                        .authoritative_type
                        .as_ref()
                        .is_some_and(|built_in| built_in != &classified)
                    {
                        return self
                            .reject(context, &request, UploadRejectionReason::TypeMismatch, now)
                            .await;
                    }
                    inspection.authoritative_type = Some(classified);
                }
                ApplicationValidationDecision::Reject(_) => {
                    return self
                        .reject(
                            context,
                            &request,
                            UploadRejectionReason::ApplicationRejected,
                            now,
                        )
                        .await;
                }
            }
        }
        if !type_matches(&request, &inspection) {
            return self
                .reject(context, &request, UploadRejectionReason::TypeMismatch, now)
                .await;
        }
        if let Some(outcome) = self
            .scan(&request, &inspection, now, validation_deadline)
            .await?
        {
            return match outcome {
                ScanPolicyOutcome::Reject(reason) => {
                    self.reject(context, &request, reason, now).await
                }
                ScanPolicyOutcome::Retry(reason) => Ok(UploadValidationOutcome {
                    disposition: UploadValidationDisposition::Retry,
                    evidence: None,
                    reason: Some(reason),
                    transition: None,
                }),
            };
        }

        let ready_revision = request.expected_revision.checked_next()?;
        let evidence = ValidatedUpload {
            authority: record.authority().clone(),
            ready_revision,
            policy_digest: request.policy.contract_digest().clone(),
            inspection,
        };
        self.evidence.put(evidence.clone()).await?;
        let transition = self
            .authority
            .trusted_transition(
                context,
                request.field,
                UploadTransitionRequest::new(
                    request.handle,
                    request.expected_revision,
                    request.idempotency_key,
                    UploadTransition::Accept,
                ),
                now,
            )
            .await?;
        if transition.state() != UploadState::Ready || transition.revision() != ready_revision {
            return Err(UploadError::new(UploadErrorKind::UploadConflict));
        }
        Ok(UploadValidationOutcome {
            disposition: UploadValidationDisposition::Ready,
            evidence: Some(evidence),
            reason: None,
            transition: Some(transition),
        })
    }

    async fn read_prefix(
        &self,
        handle: &UploadHandle,
        total_bytes: u64,
        prefix_limit: usize,
    ) -> Result<Vec<u8>, UploadError> {
        let target = usize::try_from(total_bytes.min(prefix_limit as u64))
            .map_err(|_| UploadError::new(UploadErrorKind::InputTooLarge))?;
        let mut prefix = Vec::with_capacity(target);
        while prefix.len() < target {
            let maximum = (target - prefix.len()).min(self.limits.max_chunk_bytes());
            let offset = u64::try_from(prefix.len())
                .map_err(|_| UploadError::new(UploadErrorKind::InputTooLarge))?;
            let bytes = self
                .provider
                .read(ReadUpload::new(handle, offset, maximum))
                .await?;
            if bytes.is_empty() || bytes.len() > maximum {
                return Err(UploadError::new(UploadErrorKind::IncompleteTransfer));
            }
            prefix.extend_from_slice(&bytes);
        }
        Ok(prefix)
    }

    async fn scan(
        &self,
        request: &UploadValidationRequest,
        inspection: &UploadInspection,
        now: UnixMillis,
        validation_deadline: UnixMillis,
    ) -> Result<Option<ScanPolicyOutcome>, UploadError> {
        let UploadScanPolicy::Required {
            on_timeout,
            on_unavailable,
        } = request.policy.scan()
        else {
            return Ok(None);
        };
        let scan_deadline = deadline_after(now, self.limits.max_scan_ms())?;
        let deadline = UnixMillis::new(scan_deadline.get().min(validation_deadline.get()));
        let content = UploadContent::new(
            self.provider.as_ref(),
            inspection.handle(),
            inspection.bytes(),
            self.limits.max_chunk_bytes(),
            deadline,
        );
        let disposition = match &self.scanner {
            Some(scanner) => {
                scanner
                    .scan(ScanInput::new(inspection, content, now, deadline))
                    .await?
            }
            None => ScanDisposition::Unavailable,
        };
        let outcome = match disposition {
            ScanDisposition::Clean => return Ok(None),
            ScanDisposition::Rejected(_) => {
                ScanPolicyOutcome::Reject(UploadRejectionReason::ScanRejected)
            }
            ScanDisposition::TimedOut => match on_timeout {
                super::ScanFailurePolicy::Retry => {
                    ScanPolicyOutcome::Retry(UploadRejectionReason::ScanTimedOut)
                }
                super::ScanFailurePolicy::Reject => {
                    ScanPolicyOutcome::Reject(UploadRejectionReason::ScanTimedOut)
                }
            },
            ScanDisposition::Unavailable => match on_unavailable {
                super::ScanFailurePolicy::Retry => {
                    ScanPolicyOutcome::Retry(UploadRejectionReason::ScanUnavailable)
                }
                super::ScanFailurePolicy::Reject => {
                    ScanPolicyOutcome::Reject(UploadRejectionReason::ScanUnavailable)
                }
            },
        };
        Ok(Some(outcome))
    }

    async fn reject(
        &self,
        context: &TrustedLiveRequestContext,
        request: &UploadValidationRequest,
        reason: UploadRejectionReason,
        now: UnixMillis,
    ) -> Result<UploadValidationOutcome, UploadError> {
        let transition = self
            .authority
            .trusted_transition(
                context,
                request.field.clone(),
                UploadTransitionRequest::new(
                    request.handle.clone(),
                    request.expected_revision,
                    request.idempotency_key.clone(),
                    UploadTransition::Reject,
                ),
                now,
            )
            .await?;
        Ok(UploadValidationOutcome {
            disposition: UploadValidationDisposition::Rejected,
            evidence: None,
            reason: Some(reason),
            transition: Some(transition),
        })
    }
}

impl fmt::Debug for UploadValidationService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UploadValidationService")
            .field("scanner", &self.scanner.is_some())
            .field("application", &self.application.is_some())
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

enum ScanPolicyOutcome {
    Reject(UploadRejectionReason),
    Retry(UploadRejectionReason),
}

fn deadline_after(started_at: UnixMillis, maximum_ms: u64) -> Result<UnixMillis, UploadError> {
    started_at
        .get()
        .checked_add(maximum_ms)
        .map(UnixMillis::new)
        .ok_or_else(|| UploadError::new(UploadErrorKind::InvalidField))
}

fn type_matches(request: &UploadValidationRequest, inspection: &UploadInspection) -> bool {
    let Some(actual) = inspection.authoritative_type() else {
        return request.policy.accepted_types().is_empty();
    };
    let accepted = request.policy.accepted_types();
    let accepted_contract = accepted
        .iter()
        .find(|candidate| candidate.media_type() == actual.media_type());
    let allowed = accepted.is_empty() || accepted_contract.is_some();
    let mime_matches = request
        .client
        .claimed_media_type()
        .is_none_or(|claimed| claimed == actual.media_type());
    let extension_matches = accepted_contract.is_none_or(|contract| {
        request
            .client
            .extension()
            .is_none_or(|extension| contract.accepts_extension(extension))
    });
    allowed && mime_matches && extension_matches
}

fn dimension_rejection(
    limits: Option<super::UploadDimensionLimits>,
    dimensions: Option<MediaDimensions>,
) -> Option<UploadRejectionReason> {
    let limits = limits?;
    let Some(dimensions) = dimensions else {
        return Some(UploadRejectionReason::MediaHeaderUnproved);
    };
    if dimensions.width() > limits.maximum_width() || dimensions.height() > limits.maximum_height()
    {
        Some(UploadRejectionReason::DimensionsExceeded)
    } else if dimensions.pixels() > limits.maximum_pixels() {
        Some(UploadRejectionReason::PixelsExceeded)
    } else {
        None
    }
}
