//! Suprnova-owned application authoring contract for Live upload fields.

use std::fmt;

/// Built-in content type the Live engine can classify authoritatively.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UploadType {
    /// Graphics Interchange Format.
    Gif,
    /// Joint Photographic Experts Group image.
    Jpeg,
    /// Portable Network Graphics image.
    Png,
    /// WebP image.
    Webp,
}

/// What selecting a replacement file does to the previous temporary upload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UploadReplacement {
    /// Retire the previous temporary upload when replacement succeeds.
    RetirePrevious,
    /// Keep the previous temporary upload until explicitly removed.
    PreservePrevious,
}

/// Closed scanner failure disposition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UploadScanFailure {
    /// Leave verification pending for a bounded retry.
    Retry,
    /// Reject rather than treating scanner silence as success.
    Reject,
}

/// Whether authoritative acceptance requires a content scan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UploadScan {
    /// No scanner is required for this field.
    Disabled,
    /// A scanner is required with explicit fail-closed dispositions.
    Required {
        /// Disposition when the scanner times out.
        on_timeout: UploadScanFailure,
        /// Disposition when no scanner capability is available.
        on_unavailable: UploadScanFailure,
    },
}

#[derive(Clone)]
enum AcceptedType {
    BuiltIn(UploadType),
    Application {
        media_type: String,
        extensions: Vec<String>,
    },
}

#[derive(Clone, Copy)]
struct Dimensions {
    maximum_width: u32,
    maximum_height: u32,
    maximum_pixels: u64,
}

/// Which rule an [`UploadPolicy`] breaks.
///
/// The engine refuses a policy with one closed error for every rule, which
/// is the right answer for a browser and tells the developer who declared
/// the field nothing. This is the answer for the developer:
/// [`UploadPolicy::validate`] returns it, and the registration of a
/// component logs it before it fails.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum UploadPolicyError {
    /// `maximum_files` was not given, or is `0`.
    MaximumFiles,
    /// `maximum_file_bytes` was not given, or is `0`.
    MaximumFileBytes,
    /// A media type of `accept_application` is not in its canonical form,
    /// or its extensions are not: a media type has the form
    /// `type/subtype`, of lower case letters, digits and `+ . -`, and is
    /// at most 127 bytes, and it has between 1 and 16 extensions of lower
    /// case letters and digits, each at most 32 bytes.
    AcceptedType {
        /// The media type as it was declared.
        media_type: String,
    },
    /// More than 16 accepted types, or one media type declared twice.
    AcceptedTypes,
    /// A limit of `dimensions` is `0`.
    Dimensions,
    /// `finalize_action` was not given.
    MissingFinalizeAction,
    /// The name of `finalize_action` is no name of an action.
    FinalizeAction {
        /// The name as it was declared.
        name: String,
    },
}

impl fmt::Display for UploadPolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MaximumFiles => {
                formatter.write_str("upload policy: maximum_files is not set, or is 0")
            }
            Self::MaximumFileBytes => {
                formatter.write_str("upload policy: maximum_file_bytes is not set, or is 0")
            }
            Self::AcceptedType { media_type } => write!(
                formatter,
                "upload policy: the accepted type `{media_type}` is not in canonical form: \
                 a media type has the form type/subtype, of lower case letters, digits and \
                 + . -, and is at most 127 bytes, and has between 1 and 16 extensions of \
                 lower case letters and digits, each at most 32 bytes"
            ),
            Self::AcceptedTypes => formatter.write_str(
                "upload policy: more than 16 accepted types, or one media type declared twice",
            ),
            Self::Dimensions => formatter.write_str(
                "upload policy: a limit of dimensions is 0; width, height and pixels are \
                 each at least 1",
            ),
            Self::MissingFinalizeAction => {
                formatter.write_str("upload policy: finalize_action is not set")
            }
            Self::FinalizeAction { name } => write!(
                formatter,
                "upload policy: the finalize_action `{name}` is no name of an action"
            ),
        }
    }
}

impl std::error::Error for UploadPolicyError {}

/// Opaque validated-at-registration upload policy returned by a field helper.
#[derive(Clone)]
pub struct UploadPolicy {
    maximum_files: Option<usize>,
    maximum_file_bytes: Option<u64>,
    replacement: UploadReplacement,
    accepted: Vec<AcceptedType>,
    dimensions: Option<Dimensions>,
    scan: UploadScan,
    finalize_action: Option<String>,
}

impl UploadPolicy {
    /// Starts an explicit upload policy declaration.
    #[must_use]
    pub const fn builder() -> UploadPolicyBuilder {
        UploadPolicyBuilder {
            policy: Self {
                maximum_files: None,
                maximum_file_bytes: None,
                replacement: UploadReplacement::RetirePrevious,
                accepted: Vec::new(),
                dimensions: None,
                scan: UploadScan::Disabled,
                finalize_action: None,
            },
        }
    }

    /// Check the policy against the rules of the engine, and say which
    /// one it breaks.
    ///
    /// The registration of the component makes the same check. This is for
    /// a test of the policy by itself, and for the developer who wants the
    /// reason where the registration gives the closed error.
    ///
    /// Whether `finalize_action` names an action of the component is not
    /// checked here: the policy does not know its component. The
    /// registration checks it.
    ///
    /// # Errors
    ///
    /// The first rule the policy breaks. The rules of one value are
    /// checked in the order of [`UploadPolicyError`], and the rule of the
    /// list of the accepted types,
    /// [`AcceptedTypes`](UploadPolicyError::AcceptedTypes), is checked last.
    pub fn validate(&self) -> Result<(), UploadPolicyError> {
        self.clone().into_engine().map(|_| ())
    }

    pub(crate) fn into_engine(
        self,
    ) -> Result<suprnova_live::upload::UploadFieldPolicy, UploadPolicyError> {
        use suprnova_live::upload::{
            AcceptedUploadType, ScanFailurePolicy, UploadDimensionLimits, UploadMediaType,
            UploadReplacementPolicy, UploadScanPolicy,
        };

        let maximum_files = self
            .maximum_files
            .filter(|maximum| *maximum > 0)
            .ok_or(UploadPolicyError::MaximumFiles)?;
        let maximum_file_bytes = self
            .maximum_file_bytes
            .filter(|maximum| *maximum > 0)
            .ok_or(UploadPolicyError::MaximumFileBytes)?;
        let accepted = self
            .accepted
            .into_iter()
            .map(|accepted| match accepted {
                AcceptedType::BuiltIn(kind) => Ok(AcceptedUploadType::from(match kind {
                    UploadType::Gif => UploadMediaType::Gif,
                    UploadType::Jpeg => UploadMediaType::Jpeg,
                    UploadType::Png => UploadMediaType::Png,
                    UploadType::Webp => UploadMediaType::Webp,
                })),
                AcceptedType::Application {
                    media_type,
                    extensions,
                } => {
                    let names = extensions.iter().map(String::as_str).collect::<Vec<_>>();
                    AcceptedUploadType::application(&media_type, &names)
                        .map_err(|_| UploadPolicyError::AcceptedType { media_type })
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        let dimensions = self
            .dimensions
            .map(|limits| {
                UploadDimensionLimits::new(
                    limits.maximum_width,
                    limits.maximum_height,
                    limits.maximum_pixels,
                )
                .map_err(|_| UploadPolicyError::Dimensions)
            })
            .transpose()?;
        let failure = |value| match value {
            UploadScanFailure::Retry => ScanFailurePolicy::Retry,
            UploadScanFailure::Reject => ScanFailurePolicy::Reject,
        };
        let scan = match self.scan {
            UploadScan::Disabled => UploadScanPolicy::Disabled,
            UploadScan::Required {
                on_timeout,
                on_unavailable,
            } => UploadScanPolicy::Required {
                on_timeout: failure(on_timeout),
                on_unavailable: failure(on_unavailable),
            },
        };
        let replacement = match self.replacement {
            UploadReplacement::RetirePrevious => UploadReplacementPolicy::RetirePrevious,
            UploadReplacement::PreservePrevious => UploadReplacementPolicy::PreservePrevious,
        };
        let name = self
            .finalize_action
            .ok_or(UploadPolicyError::MissingFinalizeAction)?;
        let action = suprnova_live::identity::ActionName::parse(&name)
            .map_err(|_| UploadPolicyError::FinalizeAction { name })?;
        // What is left for the engine to refuse is the list of the accepted
        // types as a whole: every rule of one value was checked above.
        suprnova_live::upload::UploadFieldPolicy::new_with_accepted_types(
            maximum_files,
            maximum_file_bytes,
            replacement,
            accepted,
            dimensions,
            scan,
            action,
        )
        .map_err(|_| UploadPolicyError::AcceptedTypes)
    }
}

impl fmt::Debug for UploadPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<UploadPolicy:redacted>")
    }
}

/// Builder covering the complete current Live upload-field contract.
pub struct UploadPolicyBuilder {
    policy: UploadPolicy,
}

impl UploadPolicyBuilder {
    /// Sets the nonzero per-field file-count ceiling.
    #[must_use]
    pub fn maximum_files(mut self, maximum: usize) -> Self {
        self.policy.maximum_files = Some(maximum);
        self
    }

    /// Sets the nonzero per-file byte ceiling.
    #[must_use]
    pub fn maximum_file_bytes(mut self, maximum: u64) -> Self {
        self.policy.maximum_file_bytes = Some(maximum);
        self
    }

    /// Sets replacement behavior.
    #[must_use]
    pub fn replacement(mut self, replacement: UploadReplacement) -> Self {
        self.policy.replacement = replacement;
        self
    }

    /// Adds one built-in authoritative content type.
    #[must_use]
    pub fn accept(mut self, accepted: UploadType) -> Self {
        self.policy.accepted.push(AcceptedType::BuiltIn(accepted));
        self
    }

    /// Adds an application-classified canonical media type and extensions.
    #[must_use]
    pub fn accept_application(mut self, media_type: &str, extensions: &[&str]) -> Self {
        self.policy.accepted.push(AcceptedType::Application {
            media_type: media_type.to_owned(),
            extensions: extensions
                .iter()
                .map(|extension| (*extension).to_owned())
                .collect(),
        });
        self
    }

    /// Sets finite image width, height, and pixel ceilings.
    #[must_use]
    pub fn dimensions(
        mut self,
        maximum_width: u32,
        maximum_height: u32,
        maximum_pixels: u64,
    ) -> Self {
        self.policy.dimensions = Some(Dimensions {
            maximum_width,
            maximum_height,
            maximum_pixels,
        });
        self
    }

    /// Sets the scanner requirement and failure policy.
    #[must_use]
    pub fn scan(mut self, scan: UploadScan) -> Self {
        self.policy.scan = scan;
        self
    }

    /// Binds the only registered action permitted to finalize this field.
    #[must_use]
    pub fn finalize_action(mut self, action: &str) -> Self {
        self.policy.finalize_action = Some(action.to_owned());
        self
    }

    /// Finishes the declaration; registry construction performs closed validation.
    #[must_use]
    pub fn build(self) -> UploadPolicy {
        self.policy
    }
}

impl fmt::Debug for UploadPolicyBuilder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<UploadPolicyBuilder:redacted>")
    }
}
