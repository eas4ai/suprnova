//! Canonical metadata for one registered Live component.

use crate::identity::{ComponentName, ContentDigest, ViewName};

use super::digest::contract_digest;
use super::{ActionMetadata, ContractVersions, FieldMetadata, MetadataError, MetadataErrorKind};

const MAX_FIELDS: usize = 512;
const MAX_ACTIONS: usize = 256;

/// Complete canonical metadata used by macros, checking, and runtime lookup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComponentMetadata {
    identity: ComponentName,
    view: ViewName,
    versions: ContractVersions,
    fields: Vec<FieldMetadata>,
    actions: Vec<ActionMetadata>,
    contract_digest: ContentDigest,
}

impl ComponentMetadata {
    /// Creates bounded metadata, rejecting duplicate identities before serving.
    pub fn new(
        identity: ComponentName,
        view: ViewName,
        versions: ContractVersions,
        mut fields: Vec<FieldMetadata>,
        mut actions: Vec<ActionMetadata>,
    ) -> Result<Self, MetadataError> {
        if fields.len() > MAX_FIELDS {
            return Err(MetadataError::new(MetadataErrorKind::TooManyFields));
        }
        if actions.len() > MAX_ACTIONS {
            return Err(MetadataError::new(MetadataErrorKind::TooManyActions));
        }

        fields.sort_by(|left, right| left.name().cmp(right.name()));
        if fields
            .windows(2)
            .any(|pair| pair[0].name() == pair[1].name())
        {
            return Err(MetadataError::new(MetadataErrorKind::DuplicateField));
        }

        actions.sort_by(|left, right| left.name().cmp(right.name()));
        if actions
            .windows(2)
            .any(|pair| pair[0].name() == pair[1].name())
        {
            return Err(MetadataError::new(MetadataErrorKind::DuplicateAction));
        }

        let contract_digest = contract_digest(&identity, &view, versions, &fields, &actions)?;
        Ok(Self {
            identity,
            view,
            versions,
            fields,
            actions,
            contract_digest,
        })
    }

    /// Returns the stable registered component identity.
    #[must_use]
    pub const fn identity(&self) -> &ComponentName {
        &self.identity
    }

    /// Returns the checked external-template identity.
    #[must_use]
    pub const fn view(&self) -> &ViewName {
        &self.view
    }

    /// Returns the independent version set.
    #[must_use]
    pub const fn versions(&self) -> ContractVersions {
        self.versions
    }

    /// Returns fields in canonical identity order.
    #[must_use]
    pub fn fields(&self) -> &[FieldMetadata] {
        &self.fields
    }

    /// Returns actions in canonical identity order.
    #[must_use]
    pub fn actions(&self) -> &[ActionMetadata] {
        &self.actions
    }

    /// Returns the purpose-specific canonical semantic contract digest.
    #[must_use]
    pub const fn contract_digest(&self) -> &ContentDigest {
        &self.contract_digest
    }
}
