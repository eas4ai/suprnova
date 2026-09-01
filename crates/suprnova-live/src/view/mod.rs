//! Checked deterministic rendering for canonical documents and Live islands.

mod contract;
mod document;
mod error;
mod island;
mod root;
mod trusted_html;

use std::collections::BTreeSet;
use std::fmt;

use bytes::Bytes;

use crate::identity::{IslandSlot, ViewName};

pub use contract::{
    AssetSet, ChildMount, MountMetadata, MountSnapshotKind, RenderLimits, TemplateFailure,
    ViewTemplate,
};
pub use document::{
    CanonicalDocumentConformance, CanonicalDocumentIntent, CanonicalDocumentRequest,
    DocumentCachePolicy, DocumentMediaType, DocumentRender, DocumentResponseIntent,
    DocumentValidator,
};
pub use error::{ViewError, ViewErrorKind};
pub use island::IslandRender;
pub(crate) use root::{
    IslandRootFlag, IslandRootInput, IslandSnapshotForm, MAX_SUCCESSOR_METADATA_BYTES,
    assemble_island_root,
};
pub use trusted_html::{
    RegisteredSanitizer, SanitizerFailure, SanitizerId, TrustedHtml, TrustedMarkupError,
    TrustedMarkupErrorKind, TrustedMarkupReason,
};

/// Suprnova-owned checked filters made available to Askama view adapters.
pub mod filters {
    pub use super::trusted_html::filters::trusted_html;
}

/// Bounded renderer that publishes results only after complete validation.
#[derive(Clone, Copy, Debug)]
pub struct ViewRenderer {
    limits: RenderLimits,
}

impl ViewRenderer {
    /// Creates a renderer from already validated bounds.
    pub fn new(limits: RenderLimits) -> Result<Self, ViewError> {
        RenderLimits::new(
            limits.max_body_bytes(),
            limits.max_assets(),
            limits.max_mounts(),
            limits.max_children(),
            limits.max_snapshot_bytes(),
        )?;
        Ok(Self { limits })
    }

    /// Renders one complete document and validates all typed response metadata.
    pub fn render_document<T: ViewTemplate + ?Sized>(
        &self,
        view: ViewName,
        template: &T,
        response: DocumentResponseIntent,
        assets: AssetSet,
        mounts: Vec<MountMetadata>,
    ) -> Result<DocumentRender, ViewError> {
        self.validate_common_metadata(&view, &assets)?;
        if mounts.len() > self.limits.max_mounts() {
            return Err(ViewError::at(ViewErrorKind::TooManyMounts, &view));
        }
        if mounts
            .iter()
            .any(|mount| mount.signed_snapshot().len() > self.limits.max_snapshot_bytes())
        {
            return Err(ViewError::at(ViewErrorKind::InvalidMountMetadata, &view));
        }
        let body = self.render_body(&view, template)?;
        let text = std::str::from_utf8(&body)
            .map_err(|_| ViewError::at(ViewErrorKind::TemplateRenderFailed, &view))?;
        let inspection = island::inspect_html(text);
        if inspection.parse_error || !inspection.complete_document {
            return Err(ViewError::at(ViewErrorKind::InvalidDocument, &view));
        }
        validate_mounts(&view, &inspection, mounts.iter().map(MountMetadata::slot))?;
        Ok(DocumentRender {
            body,
            response,
            assets,
            mounts,
        })
    }

    /// Renders one island without granting document or endpoint response authority.
    pub fn render_island<T: ViewTemplate + ?Sized>(
        &self,
        view: ViewName,
        template: &T,
        assets: AssetSet,
        children: Vec<ChildMount>,
    ) -> Result<IslandRender, ViewError> {
        let body = self.render_body(&view, template)?;
        self.validate_island_output(
            view,
            IslandRender {
                body,
                assets,
                children,
            },
        )
    }

    /// Renders checked component-owned markup before the engine adds its Live root.
    pub fn render_component_fragment<T: ViewTemplate + ?Sized>(
        &self,
        view: ViewName,
        template: &T,
        assets: AssetSet,
        children: Vec<ChildMount>,
    ) -> Result<IslandRender, ViewError> {
        let body = self.render_body(&view, template)?;
        let output = IslandRender {
            body,
            assets,
            children,
        };
        self.validate_island_fragment(view, &output)?;
        Ok(output)
    }

    /// Validates component-owned fragment bounds before engine wrapper allocation.
    pub fn validate_island_fragment(
        &self,
        view: ViewName,
        output: &IslandRender,
    ) -> Result<(), ViewError> {
        self.validate_common_metadata(&view, &output.assets)?;
        if output.children.len() > self.limits.max_children() {
            return Err(ViewError::at(ViewErrorKind::TooManyChildren, &view));
        }
        if output.body.len() > self.limits.max_body_bytes() {
            return Err(ViewError::at(ViewErrorKind::BodyTooLarge, &view));
        }
        let text = std::str::from_utf8(&output.body)
            .map_err(|_| ViewError::at(ViewErrorKind::TemplateRenderFailed, &view))?;
        let inspection = island::inspect_html(text);
        if inspection.executable_mount {
            return Err(ViewError::at(ViewErrorKind::ExecutableMountMetadata, &view));
        }
        if inspection.invalid_mount || inspection.parse_error || inspection.complete_document {
            return Err(ViewError::at(ViewErrorKind::MissingIslandRoot, &view));
        }
        validate_slot_set(
            &view,
            inspection.roots.iter().map(|(slot, _)| slot),
            output.children.iter().map(ChildMount::slot),
        )
    }

    /// Validates already assembled engine-owned island output before publication.
    pub fn validate_island_output(
        &self,
        view: ViewName,
        output: IslandRender,
    ) -> Result<IslandRender, ViewError> {
        self.validate_common_metadata(&view, &output.assets)?;
        if output.children.len() > self.limits.max_children() {
            return Err(ViewError::at(ViewErrorKind::TooManyChildren, &view));
        }
        if output.body.len() > self.limits.max_body_bytes() {
            return Err(ViewError::at(ViewErrorKind::BodyTooLarge, &view));
        }
        let body = output.body;
        let text = std::str::from_utf8(&body)
            .map_err(|_| ViewError::at(ViewErrorKind::TemplateRenderFailed, &view))?;
        let inspection = island::inspect_html(text);
        if inspection.executable_mount {
            return Err(ViewError::at(ViewErrorKind::ExecutableMountMetadata, &view));
        }
        if inspection.invalid_mount || inspection.parse_error {
            return Err(ViewError::at(ViewErrorKind::MissingIslandRoot, &view));
        }
        let top_roots = inspection
            .roots
            .iter()
            .filter(|(_, depth)| *depth == 0)
            .count();
        if inspection.top_level_elements > 1 || top_roots > 1 {
            return Err(ViewError::at(ViewErrorKind::MultipleIslandRoots, &view));
        }
        if inspection.top_level_elements != 1
            || inspection.top_level_non_whitespace
            || top_roots != 1
        {
            return Err(ViewError::at(ViewErrorKind::MissingIslandRoot, &view));
        }
        let nested_slots = inspection
            .roots
            .iter()
            .filter_map(|(slot, depth)| (*depth > 0).then_some(slot));
        validate_slot_set(
            &view,
            nested_slots,
            output.children.iter().map(ChildMount::slot),
        )?;
        Ok(IslandRender {
            body,
            assets: output.assets,
            children: output.children,
        })
    }

    fn validate_common_metadata(
        &self,
        view: &ViewName,
        assets: &AssetSet,
    ) -> Result<(), ViewError> {
        if assets.len() > self.limits.max_assets() {
            return Err(ViewError::at(ViewErrorKind::TooManyAssets, view));
        }
        Ok(())
    }

    fn render_body<T: ViewTemplate + ?Sized>(
        &self,
        view: &ViewName,
        template: &T,
    ) -> Result<Bytes, ViewError> {
        let mut output = BoundedOutput::new(self.limits.max_body_bytes());
        if let Err(failure) = template.render_view(&mut output) {
            let kind = if output.overflowed {
                ViewErrorKind::BodyTooLarge
            } else {
                match failure {
                    TemplateFailure::MissingData => ViewErrorKind::MissingViewData,
                    TemplateFailure::InvalidData => ViewErrorKind::InvalidViewData,
                    TemplateFailure::Failed => ViewErrorKind::TemplateRenderFailed,
                }
            };
            return Err(ViewError::at(kind, view));
        }
        if output.overflowed {
            return Err(ViewError::at(ViewErrorKind::BodyTooLarge, view));
        }
        Ok(Bytes::from(output.body))
    }
}

struct BoundedOutput {
    body: String,
    max_bytes: usize,
    overflowed: bool,
}

impl BoundedOutput {
    fn new(max_bytes: usize) -> Self {
        Self {
            body: String::new(),
            max_bytes,
            overflowed: false,
        }
    }
}

impl fmt::Write for BoundedOutput {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        if self.body.len().saturating_add(value.len()) > self.max_bytes {
            self.overflowed = true;
            return Err(fmt::Error);
        }
        self.body.push_str(value);
        Ok(())
    }
}

fn validate_mounts<'a, 'b>(
    view: &ViewName,
    inspection: &'a island::HtmlInspection,
    expected: impl Iterator<Item = &'b IslandSlot>,
) -> Result<(), ViewError> {
    if inspection.executable_mount {
        return Err(ViewError::at(ViewErrorKind::ExecutableMountMetadata, view));
    }
    if inspection.invalid_mount {
        return Err(ViewError::at(ViewErrorKind::MountMetadataMismatch, view));
    }
    validate_slot_set(
        view,
        inspection.roots.iter().map(|(slot, _)| slot),
        expected,
    )
}

fn validate_slot_set<'a, 'b>(
    view: &ViewName,
    observed: impl Iterator<Item = &'a IslandSlot>,
    expected: impl Iterator<Item = &'b IslandSlot>,
) -> Result<(), ViewError> {
    let observed: Vec<_> = observed.cloned().collect();
    let expected: Vec<_> = expected.cloned().collect();
    let observed_set: BTreeSet<_> = observed.iter().cloned().collect();
    let expected_set: BTreeSet<_> = expected.iter().cloned().collect();
    if observed.len() != observed_set.len()
        || expected.len() != expected_set.len()
        || observed_set != expected_set
    {
        return Err(ViewError::at(ViewErrorKind::MountMetadataMismatch, view));
    }
    Ok(())
}
