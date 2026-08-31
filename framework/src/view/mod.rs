//! Checked server-rendered view contracts for routes and Live components.
//!
//! Askama is the normative checked substrate behind this facade. Application
//! handlers and components depend on Suprnova-owned contracts rather than the
//! template engine's implementation modules.

use std::fmt;

mod response;

pub use http::{HeaderName, HeaderValue, StatusCode};
pub use response::{DocumentResponseError, DocumentResponseErrorKind, document_response};
pub use suprnova_live::identity::ViewName;

pub use suprnova_live::view::{
    AssetSet, CanonicalDocumentConformance, CanonicalDocumentIntent, CanonicalDocumentRequest,
    ChildMount, DocumentCachePolicy, DocumentMediaType, DocumentRender, DocumentResponseIntent,
    DocumentValidator, IslandRender, MountMetadata, MountSnapshotKind, RegisteredSanitizer,
    RenderLimits, SanitizerFailure, SanitizerId, TrustedHtml, TrustedMarkupError,
    TrustedMarkupErrorKind, TrustedMarkupReason, ViewError, ViewErrorKind,
};

/// Result returned by a checked custom view filter.
pub type FilterResult<T> = askama::Result<T>;

/// Read-only template environment supplied to a checked custom view filter.
pub use askama::Values as FilterValues;

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
    pub trait Sealed {
        fn engine_template(&self) -> &dyn suprnova_live::view::ViewTemplate;
    }

    impl<T> Sealed for T
    where
        T: suprnova_live::view::ViewTemplate,
    {
        fn engine_template(&self) -> &dyn suprnova_live::view::ViewTemplate {
            self
        }
    }
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

/// Framework-owned checked renderer used by routes and Live components.
///
/// The wrapper keeps the normative template engine and host-neutral engine trait
/// out of application bounds while retaining the engine's bounded validation.
#[derive(Clone, Copy, Debug)]
pub struct ViewRenderer {
    engine: suprnova_live::view::ViewRenderer,
}

impl ViewRenderer {
    /// Creates a checked renderer from validated output limits.
    pub fn new(limits: RenderLimits) -> Result<Self, ViewError> {
        suprnova_live::view::ViewRenderer::new(limits).map(|engine| Self { engine })
    }

    /// Renders and validates one complete canonical document.
    pub fn render_document<T: ViewTemplate + ?Sized>(
        &self,
        view: ViewName,
        template: &T,
        response: DocumentResponseIntent,
        assets: AssetSet,
        mounts: Vec<MountMetadata>,
    ) -> Result<DocumentRender, ViewError> {
        self.engine.render_document(
            view,
            sealed::Sealed::engine_template(template),
            response,
            assets,
            mounts,
        )
    }

    /// Renders and validates one independently owned Live island.
    pub fn render_island<T: ViewTemplate + ?Sized>(
        &self,
        view: ViewName,
        template: &T,
        assets: AssetSet,
        children: Vec<ChildMount>,
    ) -> Result<IslandRender, ViewError> {
        self.engine.render_island(
            view,
            sealed::Sealed::engine_template(template),
            assets,
            children,
        )
    }

    /// Validates a component-owned fragment before engine wrapper allocation.
    pub fn validate_island_fragment(
        &self,
        view: ViewName,
        output: &IslandRender,
    ) -> Result<(), ViewError> {
        self.engine.validate_island_fragment(view, output)
    }

    /// Validates already assembled engine-owned island output before publication.
    pub fn validate_island_output(
        &self,
        view: ViewName,
        output: IslandRender,
    ) -> Result<IslandRender, ViewError> {
        self.engine.validate_island_output(view, output)
    }
}

/// Checked filters available to the normative template substrate.
pub mod filters {
    pub use suprnova_live::view::filters::trusted_html;
}
