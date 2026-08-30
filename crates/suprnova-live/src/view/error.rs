//! Redacted rendering failures with bounded source identity.

use std::error::Error;
use std::fmt;

use crate::identity::ViewName;

/// Closed reason a view did not produce an accepted render result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ViewErrorKind {
    /// A renderer or metadata limit was zero or above its hard ceiling.
    InvalidLimits,
    /// Askama failed after rendering began.
    TemplateRenderFailed,
    /// A required runtime view value was absent.
    MissingViewData,
    /// A runtime view value had the wrong registered type.
    InvalidViewData,
    /// Rendered HTML exceeded its configured byte limit.
    BodyTooLarge,
    /// Declared asset metadata exceeded its configured count.
    TooManyAssets,
    /// Declared document mount metadata exceeded its configured count.
    TooManyMounts,
    /// Declared child mount metadata exceeded its configured count.
    TooManyChildren,
    /// A snapshot or child metadata field was empty or exceeded its bound.
    InvalidMountMetadata,
    /// Rendered document markup was not complete canonical HTML.
    InvalidDocument,
    /// Document mount markers and typed metadata disagreed.
    MountMetadataMismatch,
    /// An island did not expose one explicit root boundary.
    MissingIslandRoot,
    /// An island exposed more than one top-level root boundary.
    MultipleIslandRoots,
    /// Live mount metadata appeared on executable or event-bearing markup.
    ExecutableMountMetadata,
    /// Typed response metadata attempted to own a forbidden transport field.
    ForbiddenResponseIntent,
}

impl ViewErrorKind {
    /// Returns a stable safe machine value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidLimits => "invalid_view_limits",
            Self::TemplateRenderFailed => "template_render_failed",
            Self::MissingViewData => "missing_view_data",
            Self::InvalidViewData => "invalid_view_data",
            Self::BodyTooLarge => "view_body_too_large",
            Self::TooManyAssets => "too_many_view_assets",
            Self::TooManyMounts => "too_many_document_mounts",
            Self::TooManyChildren => "too_many_child_mounts",
            Self::InvalidMountMetadata => "invalid_mount_metadata",
            Self::InvalidDocument => "invalid_canonical_document",
            Self::MountMetadataMismatch => "mount_metadata_mismatch",
            Self::MissingIslandRoot => "missing_island_root",
            Self::MultipleIslandRoots => "multiple_island_roots",
            Self::ExecutableMountMetadata => "executable_mount_metadata",
            Self::ForbiddenResponseIntent => "forbidden_document_response_intent",
        }
    }
}

/// Redacted rendering error that retains only a validated source identity.
#[derive(Clone, Eq, PartialEq)]
pub struct ViewError {
    kind: ViewErrorKind,
    view: Option<ViewName>,
}

impl ViewError {
    pub(crate) const fn new(kind: ViewErrorKind) -> Self {
        Self { kind, view: None }
    }

    pub(crate) fn at(kind: ViewErrorKind, view: &ViewName) -> Self {
        Self {
            kind,
            view: Some(view.clone()),
        }
    }

    /// Returns the closed failure class.
    #[must_use]
    pub const fn kind(&self) -> ViewErrorKind {
        self.kind
    }
}

impl fmt::Display for ViewError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.as_str())?;
        if let Some(view) = &self.view {
            formatter.write_str(":")?;
            formatter.write_str(view.as_str())?;
        }
        Ok(())
    }
}

impl fmt::Debug for ViewError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for ViewError {}
