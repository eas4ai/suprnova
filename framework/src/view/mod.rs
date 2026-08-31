//! Checked server-rendered view contracts for routes and Live components.
//!
//! Askama is the normative checked substrate behind this facade. Application
//! handlers and components depend on Suprnova-owned contracts rather than the
//! template engine's implementation modules.

use std::fmt;

pub use suprnova_live::view::{
    AssetSet, CanonicalDocumentConformance, CanonicalDocumentIntent, CanonicalDocumentRequest,
    ChildMount, DocumentCachePolicy, DocumentMediaType, DocumentRender, DocumentResponseIntent,
    DocumentValidator, IslandRender, MountMetadata, MountSnapshotKind, RegisteredSanitizer,
    RenderLimits, SanitizerFailure, SanitizerId, TrustedHtml, TrustedMarkupError,
    TrustedMarkupErrorKind, TrustedMarkupReason, ViewError, ViewErrorKind,
};

/// Redacted failure returned by the normative checked template substrate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum TemplateFailure {
    /// Required dynamic render data was absent.
    MissingData,
    /// Dynamic render data had a different type than the checked template expected.
    InvalidData,
    /// Rendering failed for another redacted reason.
    Failed,
}

mod sealed {
    pub trait Sealed {}

    impl<T> Sealed for T where T: suprnova_live::view::ViewTemplate {}
}

/// Framework-owned contract implemented by the normative checked template substrate.
#[allow(
    private_bounds,
    reason = "the private supertrait prevents unchecked renderer implementations"
)]
pub trait ViewTemplate: sealed::Sealed {
    /// Writes checked template output into a framework-owned bounded buffer.
    fn render_view(&self, output: &mut dyn fmt::Write) -> Result<(), TemplateFailure>;
}

#[doc(hidden)]
impl<T> ViewTemplate for T
where
    T: suprnova_live::view::ViewTemplate,
{
    fn render_view(&self, output: &mut dyn fmt::Write) -> Result<(), TemplateFailure> {
        suprnova_live::view::ViewTemplate::render_view(self, output).map_err(
            |failure| match failure {
                suprnova_live::view::TemplateFailure::MissingData => TemplateFailure::MissingData,
                suprnova_live::view::TemplateFailure::InvalidData => TemplateFailure::InvalidData,
                suprnova_live::view::TemplateFailure::Failed => TemplateFailure::Failed,
            },
        )
    }
}

/// Checked filters available to the normative template substrate.
pub mod filters {
    pub use suprnova_live::view::filters::trusted_html;
}
