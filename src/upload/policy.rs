//! Digest-significant upload field policy.

use sha2::{Digest, Sha256};

use crate::identity::{ActionName, ContentDigest};

use super::{UploadError, UploadErrorKind};

const MAX_ACCEPTED_MEDIA_TYPES: usize = 16;
const MAX_ACCEPTED_EXTENSIONS: usize = 16;
const MAX_MEDIA_TYPE_BYTES: usize = 127;
const MAX_EXTENSION_BYTES: usize = 32;

/// Built-in content types that the bounded engine can identify authoritatively.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum UploadMediaType {
    /// Graphics Interchange Format.
    Gif,
    /// Joint Photographic Experts Group image.
    Jpeg,
    /// Portable Network Graphics image.
    Png,
    /// WebP image.
    Webp,
}

impl UploadMediaType {
    /// Returns the canonical media type.
    #[must_use]
    pub const fn media_type(self) -> &'static str {
        match self {
            Self::Gif => "image/gif",
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
            Self::Webp => "image/webp",
        }
    }
}

/// Content type proved by the engine classifier or a trusted application classifier.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct AuthoritativeUploadType(String);

impl AuthoritativeUploadType {
    /// Creates an application-classified canonical MIME type.
    pub fn application(media_type: &str) -> Result<Self, UploadError> {
        if valid_media_type(media_type) {
            Ok(Self(media_type.to_owned()))
        } else {
            Err(UploadError::new(UploadErrorKind::InvalidField))
        }
    }

    /// Returns the canonical authoritative MIME type.
    #[must_use]
    pub fn media_type(&self) -> &str {
        &self.0
    }
}

impl From<UploadMediaType> for AuthoritativeUploadType {
    fn from(value: UploadMediaType) -> Self {
        Self(value.media_type().to_owned())
    }
}

/// Canonical accepted or application-classified content type.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct AcceptedUploadType {
    media_type: String,
    extensions: Vec<String>,
}

impl AcceptedUploadType {
    /// Creates an application content contract from a canonical MIME type and extensions.
    pub fn application(media_type: &str, extensions: &[&str]) -> Result<Self, UploadError> {
        if !valid_media_type(media_type)
            || extensions.is_empty()
            || extensions.len() > MAX_ACCEPTED_EXTENSIONS
        {
            return Err(UploadError::new(UploadErrorKind::InvalidField));
        }
        let mut extensions = extensions
            .iter()
            .map(|extension| extension.trim_start_matches('.'))
            .map(|extension| {
                let valid = !extension.is_empty()
                    && extension.len() <= MAX_EXTENSION_BYTES
                    && extension
                        .bytes()
                        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit());
                if valid {
                    Ok(extension.to_owned())
                } else {
                    Err(UploadError::new(UploadErrorKind::InvalidField))
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        extensions.sort_unstable();
        extensions.dedup();
        Ok(Self {
            media_type: media_type.to_owned(),
            extensions,
        })
    }

    /// Returns the canonical authoritative media type.
    #[must_use]
    pub fn media_type(&self) -> &str {
        &self.media_type
    }

    /// Returns canonical extensions without leading dots.
    #[must_use]
    pub fn extensions(&self) -> &[String] {
        &self.extensions
    }

    pub(crate) fn accepts_extension(&self, extension: &str) -> bool {
        self.extensions
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(extension))
    }
}

impl From<UploadMediaType> for AcceptedUploadType {
    fn from(value: UploadMediaType) -> Self {
        let extensions = match value {
            UploadMediaType::Gif => vec!["gif".to_owned()],
            UploadMediaType::Jpeg => vec!["jpeg".to_owned(), "jpg".to_owned()],
            UploadMediaType::Png => vec!["png".to_owned()],
            UploadMediaType::Webp => vec!["webp".to_owned()],
        };
        Self {
            media_type: value.media_type().to_owned(),
            extensions,
        }
    }
}

/// What selecting a replacement file does to the previous temporary upload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UploadReplacementPolicy {
    /// Retire the previous temporary upload when replacement succeeds.
    RetirePrevious,
    /// Keep the previous temporary upload until the form explicitly removes it.
    PreservePrevious,
}

impl UploadReplacementPolicy {
    pub(crate) const fn code(self) -> u8 {
        match self {
            Self::RetirePrevious => 1,
            Self::PreservePrevious => 2,
        }
    }
}

/// Policy applied when a required scan cannot produce a clean/rejected result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScanFailurePolicy {
    /// Leave the upload verifying and permit a bounded retry.
    Retry,
    /// Reject the upload rather than treating scanner silence as success.
    Reject,
}

impl ScanFailurePolicy {
    pub(crate) const fn code(self) -> u8 {
        match self {
            Self::Retry => 1,
            Self::Reject => 2,
        }
    }
}

/// Whether authoritative upload acceptance requires a content scan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UploadScanPolicy {
    /// No scanner is required for this field.
    Disabled,
    /// A scanner is required and both failure modes have explicit policy.
    Required {
        /// Disposition when the scanner reports its deadline elapsed.
        on_timeout: ScanFailurePolicy,
        /// Disposition when the scanner capability is unavailable.
        on_unavailable: ScanFailurePolicy,
    },
}

/// Finite authoritative image dimension limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UploadDimensionLimits {
    maximum_width: u32,
    maximum_height: u32,
    maximum_pixels: u64,
}

impl UploadDimensionLimits {
    /// Creates nonzero width, height, and pixel ceilings.
    pub fn new(
        maximum_width: u32,
        maximum_height: u32,
        maximum_pixels: u64,
    ) -> Result<Self, UploadError> {
        if maximum_width == 0 || maximum_height == 0 || maximum_pixels == 0 {
            return Err(UploadError::new(UploadErrorKind::InvalidField));
        }
        Ok(Self {
            maximum_width,
            maximum_height,
            maximum_pixels,
        })
    }

    /// Returns the maximum accepted width.
    #[must_use]
    pub const fn maximum_width(self) -> u32 {
        self.maximum_width
    }

    /// Returns the maximum accepted height.
    #[must_use]
    pub const fn maximum_height(self) -> u32 {
        self.maximum_height
    }

    /// Returns the maximum accepted width-times-height value.
    #[must_use]
    pub const fn maximum_pixels(self) -> u64 {
        self.maximum_pixels
    }
}

/// Canonical component-field upload contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UploadFieldPolicy {
    maximum_files: usize,
    maximum_file_bytes: u64,
    replacement: UploadReplacementPolicy,
    accepted_types: Vec<AcceptedUploadType>,
    dimensions: Option<UploadDimensionLimits>,
    scan: UploadScanPolicy,
    finalize_action: ActionName,
    contract_digest: ContentDigest,
}

impl UploadFieldPolicy {
    /// Creates one finite, canonical, digest-significant upload policy.
    #[allow(
        clippy::too_many_arguments,
        reason = "the field policy is one closed generated contract tuple"
    )]
    pub fn new(
        maximum_files: usize,
        maximum_file_bytes: u64,
        replacement: UploadReplacementPolicy,
        accepted_media_types: Vec<UploadMediaType>,
        dimensions: Option<UploadDimensionLimits>,
        scan: UploadScanPolicy,
        finalize_action: ActionName,
    ) -> Result<Self, UploadError> {
        Self::new_with_accepted_types(
            maximum_files,
            maximum_file_bytes,
            replacement,
            accepted_media_types.into_iter().map(Into::into).collect(),
            dimensions,
            scan,
            finalize_action,
        )
    }

    /// Creates a policy including application-classified content contracts.
    #[allow(
        clippy::too_many_arguments,
        reason = "the field policy is one closed generated contract tuple"
    )]
    pub fn new_with_accepted_types(
        maximum_files: usize,
        maximum_file_bytes: u64,
        replacement: UploadReplacementPolicy,
        mut accepted_types: Vec<AcceptedUploadType>,
        dimensions: Option<UploadDimensionLimits>,
        scan: UploadScanPolicy,
        finalize_action: ActionName,
    ) -> Result<Self, UploadError> {
        accepted_types.sort_unstable();
        accepted_types.dedup();
        if maximum_files == 0
            || maximum_file_bytes == 0
            || accepted_types.len() > MAX_ACCEPTED_MEDIA_TYPES
            || accepted_types
                .windows(2)
                .any(|pair| pair[0].media_type() == pair[1].media_type())
        {
            return Err(UploadError::new(UploadErrorKind::InvalidField));
        }
        let contract_digest = policy_digest(
            maximum_files,
            maximum_file_bytes,
            replacement,
            &accepted_types,
            dimensions,
            scan,
            &finalize_action,
        )?;
        Ok(Self {
            maximum_files,
            maximum_file_bytes,
            replacement,
            accepted_types,
            dimensions,
            scan,
            finalize_action,
            contract_digest,
        })
    }

    /// Returns the per-field file count ceiling.
    #[must_use]
    pub const fn maximum_files(&self) -> usize {
        self.maximum_files
    }

    /// Returns the per-file byte ceiling.
    #[must_use]
    pub const fn maximum_file_bytes(&self) -> u64 {
        self.maximum_file_bytes
    }

    /// Returns the field's replacement behavior.
    #[must_use]
    pub const fn replacement(&self) -> UploadReplacementPolicy {
        self.replacement
    }

    /// Returns accepted built-in and application-classified types in canonical order.
    #[must_use]
    pub fn accepted_types(&self) -> &[AcceptedUploadType] {
        &self.accepted_types
    }

    /// Returns optional image dimension limits.
    #[must_use]
    pub const fn dimensions(&self) -> Option<UploadDimensionLimits> {
        self.dimensions
    }

    /// Returns the scanner policy.
    #[must_use]
    pub const fn scan(&self) -> UploadScanPolicy {
        self.scan
    }

    /// Returns the only registered action permitted to finalize this field.
    #[must_use]
    pub const fn finalize_action(&self) -> &ActionName {
        &self.finalize_action
    }

    /// Returns the canonical semantic policy digest.
    #[must_use]
    pub const fn contract_digest(&self) -> &ContentDigest {
        &self.contract_digest
    }
}

fn valid_media_type(media_type: &str) -> bool {
    media_type.len() <= MAX_MEDIA_TYPE_BYTES
        && media_type.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'/' | b'+' | b'.' | b'-')
        })
        && media_type.split_once('/').is_some_and(|(kind, subtype)| {
            !kind.is_empty() && !subtype.is_empty() && !subtype.contains('/')
        })
}

#[allow(
    clippy::too_many_arguments,
    reason = "the digest covers the complete closed policy tuple"
)]
fn policy_digest(
    maximum_files: usize,
    maximum_file_bytes: u64,
    replacement: UploadReplacementPolicy,
    accepted_types: &[AcceptedUploadType],
    dimensions: Option<UploadDimensionLimits>,
    scan: UploadScanPolicy,
    finalize_action: &ActionName,
) -> Result<ContentDigest, UploadError> {
    let maximum_files = u64::try_from(maximum_files)
        .map_err(|_| UploadError::new(UploadErrorKind::InvalidField))?;
    let mut hasher = Sha256::new();
    hasher.update(b"suprnova-live:upload-field-policy:v1\0");
    hasher.update(maximum_files.to_be_bytes());
    hasher.update(maximum_file_bytes.to_be_bytes());
    hasher.update([replacement.code()]);
    hasher.update([accepted_types.len() as u8]);
    for accepted in accepted_types {
        let media_type = accepted.media_type().as_bytes();
        hasher.update([media_type.len() as u8]);
        hasher.update(media_type);
        hasher.update([accepted.extensions().len() as u8]);
        for extension in accepted.extensions() {
            hasher.update([extension.len() as u8]);
            hasher.update(extension.as_bytes());
        }
    }
    match dimensions {
        Some(dimensions) => {
            hasher.update([1]);
            hasher.update(dimensions.maximum_width.to_be_bytes());
            hasher.update(dimensions.maximum_height.to_be_bytes());
            hasher.update(dimensions.maximum_pixels.to_be_bytes());
        }
        None => hasher.update([0]),
    }
    match scan {
        UploadScanPolicy::Disabled => hasher.update([0]),
        UploadScanPolicy::Required {
            on_timeout,
            on_unavailable,
        } => hasher.update([1, on_timeout.code(), on_unavailable.code()]),
    }
    let action = finalize_action.as_str().as_bytes();
    let action_len =
        u16::try_from(action.len()).map_err(|_| UploadError::new(UploadErrorKind::InvalidField))?;
    hasher.update(action_len.to_be_bytes());
    hasher.update(action);
    ContentDigest::from_bytes(&hasher.finalize())
        .map_err(|_| UploadError::new(UploadErrorKind::InvalidField))
}
