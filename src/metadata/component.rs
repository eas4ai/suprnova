//! Canonical metadata for one registered Live component.

use crate::identity::{ComponentName, ContentDigest, ViewName};

use super::digest::contract_digest;
use super::{
    ActionMetadata, ContractVersions, EffectMetadata, EventMetadata, FieldMetadata, MetadataError,
    MetadataErrorKind,
};

const MAX_FIELDS: usize = 512;
const MAX_ACTIONS: usize = 256;
const MAX_EVENTS: usize = 256;
const MAX_EFFECTS: usize = 256;

/// Complete canonical metadata used by macros, checking, and runtime lookup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComponentMetadata {
    identity: ComponentName,
    view: ViewName,
    versions: ContractVersions,
    fields: Vec<FieldMetadata>,
    actions: Vec<ActionMetadata>,
    events: Vec<EventMetadata>,
    effects: Vec<EffectMetadata>,
    refresh_on_promote: bool,
    contract_digest: ContentDigest,
}

impl ComponentMetadata {
    /// Creates bounded metadata, rejecting duplicate identities before serving.
    pub fn new(
        identity: ComponentName,
        view: ViewName,
        versions: ContractVersions,
        fields: Vec<FieldMetadata>,
        actions: Vec<ActionMetadata>,
    ) -> Result<Self, MetadataError> {
        Self::new_with_browser_contracts(
            identity,
            view,
            versions,
            fields,
            actions,
            Vec::new(),
            Vec::new(),
            false,
        )
    }

    /// Creates complete metadata including declared browser payload contracts.
    #[allow(
        clippy::too_many_arguments,
        reason = "the macro supplies one closed contract tuple"
    )]
    pub fn new_with_browser_contracts(
        identity: ComponentName,
        view: ViewName,
        versions: ContractVersions,
        mut fields: Vec<FieldMetadata>,
        mut actions: Vec<ActionMetadata>,
        mut events: Vec<EventMetadata>,
        mut effects: Vec<EffectMetadata>,
        refresh_on_promote: bool,
    ) -> Result<Self, MetadataError> {
        if refresh_on_promote && versions.minimum_protocol() < 2 {
            return Err(MetadataError::new(MetadataErrorKind::UnsupportedProtocol));
        }
        if fields.len() > MAX_FIELDS {
            return Err(MetadataError::new(MetadataErrorKind::TooManyFields));
        }
        if actions.len() > MAX_ACTIONS {
            return Err(MetadataError::new(MetadataErrorKind::TooManyActions));
        }
        if events.len() > MAX_EVENTS {
            return Err(MetadataError::new(MetadataErrorKind::TooManyEvents));
        }
        if effects.len() > MAX_EFFECTS {
            return Err(MetadataError::new(MetadataErrorKind::TooManyEffects));
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

        events.sort_by(|left, right| left.name().cmp(right.name()));
        if events
            .windows(2)
            .any(|pair| pair[0].name() == pair[1].name())
        {
            return Err(MetadataError::new(MetadataErrorKind::DuplicateEvent));
        }

        effects.sort_by(|left, right| left.name().cmp(right.name()));
        if effects
            .windows(2)
            .any(|pair| pair[0].name() == pair[1].name())
        {
            return Err(MetadataError::new(MetadataErrorKind::DuplicateEffect));
        }

        let contract_digest = contract_digest(
            &identity,
            &view,
            versions,
            &fields,
            &actions,
            &events,
            &effects,
            refresh_on_promote,
        )?;
        Ok(Self {
            identity,
            view,
            versions,
            fields,
            actions,
            events,
            effects,
            refresh_on_promote,
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

    /// Returns declared browser events in canonical identity order.
    #[must_use]
    pub fn events(&self) -> &[EventMetadata] {
        &self.events
    }

    /// Returns declared browser effects in canonical identity order.
    #[must_use]
    pub fn effects(&self) -> &[EffectMetadata] {
        &self.effects
    }

    /// Returns whether seed promotion must resolve as a protocol-v2 fresh render.
    #[must_use]
    pub const fn refresh_on_promote(&self) -> bool {
        self.refresh_on_promote
    }

    /// Returns the purpose-specific canonical semantic contract digest.
    #[must_use]
    pub const fn contract_digest(&self) -> &ContentDigest {
        &self.contract_digest
    }
}
