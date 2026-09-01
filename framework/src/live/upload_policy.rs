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

    pub(crate) fn into_engine(self) -> Result<suprnova_live::upload::UploadFieldPolicy, ()> {
        use suprnova_live::upload::{
            AcceptedUploadType, ScanFailurePolicy, UploadDimensionLimits, UploadMediaType,
            UploadReplacementPolicy, UploadScanPolicy,
        };

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
                    let extensions = extensions.iter().map(String::as_str).collect::<Vec<_>>();
                    AcceptedUploadType::application(&media_type, &extensions).map_err(|_| ())
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
                .map_err(|_| ())
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
        let action =
            suprnova_live::identity::ActionName::parse(self.finalize_action.as_deref().ok_or(())?)
                .map_err(|_| ())?;
        suprnova_live::upload::UploadFieldPolicy::new_with_accepted_types(
            self.maximum_files.ok_or(())?,
            self.maximum_file_bytes.ok_or(())?,
            replacement,
            accepted,
            dimensions,
            scan,
            action,
        )
        .map_err(|_| ())
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
