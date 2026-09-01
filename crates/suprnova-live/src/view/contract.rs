//! Host-neutral view metadata and the checked template substrate contract.

use std::collections::BTreeSet;
use std::fmt;

use askama::Template;
use bytes::Bytes;

use crate::component::composition::{ChildHandle, PendingChildParameters};
use crate::identity::{ComponentName, IslandSlot, ViewName};

use super::{ViewError, ViewErrorKind};

const HARD_MAX_BODY_BYTES: usize = 16 * 1024 * 1024;
const HARD_MAX_ASSETS: usize = 1_024;
const HARD_MAX_MOUNTS: usize = 1_024;
const HARD_MAX_CHILDREN: usize = 1_024;
const HARD_MAX_SNAPSHOT_BYTES: usize = 2 * 1024 * 1024;

/// Validated bounds applied before a render result becomes authoritative.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RenderLimits {
    max_body_bytes: usize,
    max_assets: usize,
    max_mounts: usize,
    max_children: usize,
    max_snapshot_bytes: usize,
}

impl RenderLimits {
    /// Creates a bounded rendering policy below engine hard ceilings.
    pub fn new(
        max_body_bytes: usize,
        max_assets: usize,
        max_mounts: usize,
        max_children: usize,
        max_snapshot_bytes: usize,
    ) -> Result<Self, ViewError> {
        let valid = (1..=HARD_MAX_BODY_BYTES).contains(&max_body_bytes)
            && (1..=HARD_MAX_ASSETS).contains(&max_assets)
            && (1..=HARD_MAX_MOUNTS).contains(&max_mounts)
            && (1..=HARD_MAX_CHILDREN).contains(&max_children)
            && (1..=HARD_MAX_SNAPSHOT_BYTES).contains(&max_snapshot_bytes);
        if !valid {
            return Err(ViewError::new(ViewErrorKind::InvalidLimits));
        }
        Ok(Self {
            max_body_bytes,
            max_assets,
            max_mounts,
            max_children,
            max_snapshot_bytes,
        })
    }

    /// Returns conservative standalone defaults for ordinary views.
    #[must_use]
    pub const fn standard() -> Self {
        Self {
            max_body_bytes: 2 * 1024 * 1024,
            max_assets: 128,
            max_mounts: 128,
            max_children: 128,
            max_snapshot_bytes: 512 * 1024,
        }
    }

    pub(crate) const fn max_body_bytes(self) -> usize {
        self.max_body_bytes
    }

    pub(crate) const fn max_assets(self) -> usize {
        self.max_assets
    }

    pub(crate) const fn max_mounts(self) -> usize {
        self.max_mounts
    }

    pub(crate) const fn max_children(self) -> usize {
        self.max_children
    }

    pub(crate) const fn max_snapshot_bytes(self) -> usize {
        self.max_snapshot_bytes
    }
}

/// Internal failure classification returned by a checked template substrate.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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

    impl<T> Sealed for T where T: askama::Template {}
}

/// Framework-owned template contract used by document and island renderers.
///
/// Askama implements this contract normatively without appearing in handler or
/// component render method signatures. Application code cannot install an
/// unchecked generic renderer:
///
/// ```compile_fail
/// # use suprnova_live as suprnova;
/// use std::fmt;
/// use suprnova::view::{TemplateFailure, ViewTemplate};
/// struct UncheckedRenderer;
/// impl ViewTemplate for UncheckedRenderer {
///     fn render_view(&self, _: &mut dyn fmt::Write) -> Result<(), TemplateFailure> {
///         Ok(())
///     }
/// }
/// ```
pub trait ViewTemplate: sealed::Sealed {
    /// Writes template output into a framework-owned bounded buffer.
    fn render_view(&self, output: &mut dyn fmt::Write) -> Result<(), TemplateFailure>;
}

impl<T> ViewTemplate for T
where
    T: Template,
{
    fn render_view(&self, output: &mut dyn fmt::Write) -> Result<(), TemplateFailure> {
        self.render_into(output).map_err(|error| match error {
            askama::Error::ValueMissing => TemplateFailure::MissingData,
            askama::Error::ValueType => TemplateFailure::InvalidData,
            _ => TemplateFailure::Failed,
        })
    }
}

/// Deterministically ordered registered assets required by one render.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AssetSet {
    assets: BTreeSet<ViewName>,
}

impl AssetSet {
    /// Creates an empty asset declaration.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            assets: BTreeSet::new(),
        }
    }

    /// Creates a deterministically ordered set of validated asset identities.
    #[must_use]
    pub fn new(assets: impl IntoIterator<Item = ViewName>) -> Self {
        Self {
            assets: assets.into_iter().collect(),
        }
    }

    /// Returns whether the render declared no assets.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.assets.is_empty()
    }

    /// Returns the unique asset count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.assets.len()
    }
}

/// Signed snapshot form associated with one initial island boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MountSnapshotKind {
    /// Public reusable seed promoted on first interaction.
    PublicSeed,
    /// Principal/session/tenant-scoped instance snapshot.
    Instance,
}

/// Inert typed browser mount metadata kept separate from executable markup.
#[derive(Clone)]
pub struct MountMetadata {
    slot: IslandSlot,
    component: ComponentName,
    snapshot_kind: MountSnapshotKind,
    signed_snapshot: Bytes,
}

impl MountMetadata {
    /// Creates non-executable metadata for one document-local island slot.
    pub fn new(
        slot: IslandSlot,
        component: ComponentName,
        snapshot_kind: MountSnapshotKind,
        signed_snapshot: Bytes,
    ) -> Result<Self, ViewError> {
        if signed_snapshot.is_empty() || signed_snapshot.len() > HARD_MAX_SNAPSHOT_BYTES {
            return Err(ViewError::new(ViewErrorKind::InvalidMountMetadata));
        }
        Ok(Self {
            slot,
            component,
            snapshot_kind,
            signed_snapshot,
        })
    }

    /// Returns the stable document-local slot.
    #[must_use]
    pub const fn slot(&self) -> &IslandSlot {
        &self.slot
    }

    /// Returns the registered component identity.
    #[must_use]
    pub const fn component(&self) -> &ComponentName {
        &self.component
    }

    /// Returns whether metadata carries a public seed or instance snapshot.
    #[must_use]
    pub const fn snapshot_kind(&self) -> MountSnapshotKind {
        self.snapshot_kind
    }

    /// Returns the signed, non-executable snapshot transport bytes.
    #[must_use]
    pub fn signed_snapshot(&self) -> &[u8] {
        &self.signed_snapshot
    }
}

impl fmt::Debug for MountMetadata {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<MountMetadata:redacted>")
    }
}

/// Typed metadata for one independently owned nested island.
#[derive(Clone)]
pub struct ChildMount {
    slot: IslandSlot,
    component: ComponentName,
    transition: Option<ChildMountTransition>,
}

#[derive(Clone)]
pub(crate) enum ChildMountTransition {
    Surviving(ChildHandle),
    Pending(PendingChildParameters),
}

impl ChildMount {
    /// Declares a child boundary without granting parent ownership of child state.
    #[must_use]
    pub const fn new(slot: IslandSlot, component: ComponentName) -> Self {
        Self {
            slot,
            component,
            transition: None,
        }
    }

    /// Declares an unchanged surviving child with exact signed-lineage identity.
    #[must_use]
    pub fn surviving(slot: IslandSlot, child: ChildHandle) -> Self {
        Self {
            slot,
            component: child.component().clone(),
            transition: Some(ChildMountTransition::Surviving(child)),
        }
    }

    /// Declares a surviving child whose validated parameter value changed.
    #[must_use]
    pub fn pending_parameters(slot: IslandSlot, pending: PendingChildParameters) -> Self {
        Self {
            slot,
            component: pending.child().component().clone(),
            transition: Some(ChildMountTransition::Pending(pending)),
        }
    }

    /// Returns the child slot local to its parent render.
    #[must_use]
    pub const fn slot(&self) -> &IslandSlot {
        &self.slot
    }

    /// Returns the registered child component identity.
    #[must_use]
    pub const fn component(&self) -> &ComponentName {
        &self.component
    }

    pub(crate) const fn transition(&self) -> Option<&ChildMountTransition> {
        self.transition.as_ref()
    }
}

impl fmt::Debug for ChildMount {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<ChildMount:redacted>")
    }
}
