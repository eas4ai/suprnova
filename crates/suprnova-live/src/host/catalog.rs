//! Immutable route/slot mount catalog bound to the component registry.

use std::collections::{HashMap, HashSet};
use std::fmt;

use crate::SUPPORTED_PROTOCOL_VERSIONS;
use crate::identity::{ComponentName, ContentDigest, IslandSlot, RouteIdentity};
use crate::registry::ComponentRegistry;
use crate::snapshot::ExpectedSeedV1;

use super::{HostContextError, HostContextErrorKind};

const MAX_MOUNTS: usize = 16_384;

/// Presence policy for one host identity dimension at a mount.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScopeRequirement {
    /// The current request must carry the dimension.
    Required,
    /// The current request may carry or omit the dimension.
    Optional,
    /// The current request must omit the dimension under explicit route policy.
    Absent,
}

/// Session, principal, and tenant presence policy for one registered mount.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MountScopeRequirements {
    session: ScopeRequirement,
    principal: ScopeRequirement,
    tenant: ScopeRequirement,
}

impl MountScopeRequirements {
    /// Creates explicit identity requirements for a registered mount.
    #[must_use]
    pub const fn new(
        session: ScopeRequirement,
        principal: ScopeRequirement,
        tenant: ScopeRequirement,
    ) -> Self {
        Self {
            session,
            principal,
            tenant,
        }
    }

    pub(crate) const fn session(self) -> ScopeRequirement {
        self.session
    }

    pub(crate) const fn principal(self) -> ScopeRequirement {
        self.principal
    }

    pub(crate) const fn tenant(self) -> ScopeRequirement {
        self.tenant
    }
}

/// Startup declaration of one route/slot mount and its seed expectations.
pub struct MountCatalogEntry {
    expected_seed: ExpectedSeedV1,
    requirements: MountScopeRequirements,
}

impl MountCatalogEntry {
    /// Declares one mount before registry validation.
    #[must_use]
    pub const fn new(expected_seed: ExpectedSeedV1, requirements: MountScopeRequirements) -> Self {
        Self {
            expected_seed,
            requirements,
        }
    }
}

impl fmt::Debug for MountCatalogEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<MountCatalogEntry:redacted>")
    }
}

/// Browser-selectable mount tuple checked against trusted catalog ownership.
#[derive(Clone)]
pub struct MountSelection {
    route: RouteIdentity,
    slot: IslandSlot,
    component: ComponentName,
    contract_digest: ContentDigest,
    protocol: u16,
}

impl MountSelection {
    /// Groups the selected route/slot/component contract and protocol.
    #[must_use]
    pub const fn new(
        route: RouteIdentity,
        slot: IslandSlot,
        component: ComponentName,
        contract_digest: ContentDigest,
        protocol: u16,
    ) -> Self {
        Self {
            route,
            slot,
            component,
            contract_digest,
            protocol,
        }
    }

    /// Returns the selected route identity.
    #[must_use]
    pub const fn route(&self) -> &RouteIdentity {
        &self.route
    }

    /// Returns the selected island slot.
    #[must_use]
    pub const fn slot(&self) -> &IslandSlot {
        &self.slot
    }

    /// Returns the selected component identity.
    #[must_use]
    pub const fn component(&self) -> &ComponentName {
        &self.component
    }

    /// Returns the selected generated contract digest.
    #[must_use]
    pub const fn contract_digest(&self) -> &ContentDigest {
        &self.contract_digest
    }
}

impl fmt::Debug for MountSelection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<MountSelection:redacted>")
    }
}

/// Registry-verified mount facts retained by trusted request context.
#[derive(Clone)]
pub struct VerifiedMountCatalogMatch {
    expected_seed: ExpectedSeedV1,
    contract_digest: ContentDigest,
    minimum_protocol: u16,
    protocol: u16,
    requirements: MountScopeRequirements,
}

impl VerifiedMountCatalogMatch {
    /// Returns the exact registered component.
    #[must_use]
    pub const fn component(&self) -> &ComponentName {
        self.expected_seed.component.name()
    }

    /// Returns the exact trusted route identity.
    #[must_use]
    pub const fn route(&self) -> &RouteIdentity {
        &self.expected_seed.route
    }

    /// Returns the exact trusted island slot.
    #[must_use]
    pub const fn slot(&self) -> &IslandSlot {
        &self.expected_seed.slot
    }

    /// Returns the generated contract digest checked against the registry.
    #[must_use]
    pub const fn contract_digest(&self) -> &ContentDigest {
        &self.contract_digest
    }

    /// Returns the component's generated minimum protocol.
    #[must_use]
    pub const fn minimum_protocol(&self) -> u16 {
        self.minimum_protocol
    }

    /// Returns the selected supported protocol for this request.
    #[must_use]
    pub const fn protocol(&self) -> u16 {
        self.protocol
    }

    pub(crate) const fn expected_seed(&self) -> &ExpectedSeedV1 {
        &self.expected_seed
    }

    pub(crate) const fn requirements(&self) -> MountScopeRequirements {
        self.requirements
    }
}

impl fmt::Debug for VerifiedMountCatalogMatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<VerifiedMountCatalogMatch:redacted>")
    }
}

/// Startup-only builder that proves catalog entries against immutable registry metadata.
#[derive(Debug, Default)]
pub struct MountCatalogBuilder {
    entries: HashMap<(RouteIdentity, IslandSlot), VerifiedMountCatalogMatch>,
    routes: HashSet<RouteIdentity>,
}

impl MountCatalogBuilder {
    /// Creates an empty non-authoritative catalog builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers one route/slot only after its component contract matches the registry.
    pub fn register(
        mut self,
        registry: &ComponentRegistry,
        entry: MountCatalogEntry,
    ) -> Result<Self, HostContextError> {
        if self.entries.len() >= MAX_MOUNTS {
            return Err(HostContextError::new(HostContextErrorKind::CatalogConflict));
        }
        let component = entry.expected_seed.component.name();
        let descriptor = registry
            .resolve(component)
            .map_err(|_| HostContextError::new(HostContextErrorKind::ComponentMismatch))?;
        if descriptor.contract_digest() != entry.expected_seed.component.contract_digest() {
            return Err(HostContextError::new(
                HostContextErrorKind::ContractMismatch,
            ));
        }
        if !entry
            .expected_seed
            .component
            .matches_schemas(&entry.expected_seed.schemas)
        {
            return Err(HostContextError::new(
                HostContextErrorKind::ContractMismatch,
            ));
        }
        let route = entry.expected_seed.route.clone();
        let slot = entry.expected_seed.slot.clone();
        let key = (route.clone(), slot);
        let verified = VerifiedMountCatalogMatch {
            expected_seed: entry.expected_seed,
            contract_digest: descriptor.contract_digest().clone(),
            minimum_protocol: descriptor.metadata().versions().minimum_protocol(),
            protocol: descriptor.metadata().versions().minimum_protocol(),
            requirements: entry.requirements,
        };
        if self.entries.insert(key, verified).is_some() {
            return Err(HostContextError::new(HostContextErrorKind::CatalogConflict));
        }
        self.routes.insert(route);
        Ok(self)
    }

    /// Consumes startup state into an immutable mount catalog.
    #[must_use]
    pub fn build(self) -> MountCatalog {
        MountCatalog {
            entries: self.entries,
            routes: self.routes,
        }
    }
}

/// Immutable trusted route/slot catalog.
#[derive(Debug)]
pub struct MountCatalog {
    entries: HashMap<(RouteIdentity, IslandSlot), VerifiedMountCatalogMatch>,
    routes: HashSet<RouteIdentity>,
}

impl MountCatalog {
    pub(crate) fn resolve(
        &self,
        selection: &MountSelection,
    ) -> Result<VerifiedMountCatalogMatch, HostContextError> {
        if !self.routes.contains(&selection.route) {
            return Err(HostContextError::new(HostContextErrorKind::RouteMismatch));
        }
        let mount = self
            .entries
            .get(&(selection.route.clone(), selection.slot.clone()))
            .ok_or_else(|| HostContextError::new(HostContextErrorKind::SlotMismatch))?;
        if mount.component() != &selection.component {
            return Err(HostContextError::new(
                HostContextErrorKind::ComponentMismatch,
            ));
        }
        if mount.contract_digest != selection.contract_digest {
            return Err(HostContextError::new(
                HostContextErrorKind::ContractMismatch,
            ));
        }
        if selection.protocol < mount.minimum_protocol
            || !SUPPORTED_PROTOCOL_VERSIONS.contains(&selection.protocol)
        {
            return Err(HostContextError::new(
                HostContextErrorKind::ProtocolMismatch,
            ));
        }
        let mut verified = mount.clone();
        verified.protocol = selection.protocol;
        Ok(verified)
    }
}
