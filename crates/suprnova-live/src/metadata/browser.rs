//! Registered browser event and effect payload contracts.

use std::any::TypeId;
use std::fmt;
use std::num::NonZeroU16;

use crate::async_updates::{
    BoundedTargets, BrowserPayloadSchema, EventCyclePolicy, EventOrder, EventSource, EventTarget,
    MAX_EVENT_FANOUT,
};
use crate::identity::{BrowserOperationName, IdentityError};

use super::{MetadataError, MetadataErrorKind};

/// Versioned payload metadata implemented by every declared browser event type.
pub trait EventPayloadMetadata {
    /// Stable browser event identity.
    const NAME: &'static str;
    /// Independently evolving event payload version.
    const VERSION: u16;
    /// Browser-visible root payload schema.
    const SCHEMA: BrowserPayloadSchema = BrowserPayloadSchema::Json;
    /// Stable payload contract identity included in the component contract digest.
    const PAYLOAD_CONTRACT: &'static str = Self::NAME;
}

/// Versioned payload metadata implemented by every declared browser effect type.
pub trait EffectPayloadMetadata {
    /// Stable browser effect identity.
    const NAME: &'static str;
    /// Independently evolving effect payload version.
    const VERSION: u16;
}

/// Stable validated identity of one browser event payload contract.
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PayloadContractIdentity(BrowserOperationName);

impl PayloadContractIdentity {
    /// Parses the same bounded ASCII identity grammar used by registered browser operations.
    pub fn parse(value: &str) -> Result<Self, IdentityError> {
        BrowserOperationName::parse(value).map(Self)
    }

    /// Returns the stable payload contract identity text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for PayloadContractIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<PayloadContractIdentity>")
    }
}

/// Canonical metadata for one declared browser event payload.
#[derive(Clone, Eq, PartialEq)]
pub struct EventMetadata {
    name: BrowserOperationName,
    version: u16,
    payload_type: TypeId,
    payload_contract: PayloadContractIdentity,
    schema: BrowserPayloadSchema,
    source: EventSource,
    targets: BoundedTargets,
    order: EventOrder,
    cycle: EventCyclePolicy,
    maximum_fanout: NonZeroU16,
}

impl fmt::Debug for EventMetadata {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EventMetadata")
            .field("name", &self.name.as_str())
            .field("version", &self.version)
            .field("schema", &self.schema)
            .field("source", &self.source)
            .field("targets", &self.targets)
            .field("order", &self.order)
            .field("cycle", &self.cycle)
            .field("maximum_fanout", &self.maximum_fanout)
            .finish()
    }
}

impl EventMetadata {
    /// Builds metadata through a declared versioned payload type.
    pub fn from_payload<T: EventPayloadMetadata + 'static>() -> Result<Self, MetadataError> {
        Self::from_payload_with_contract::<T>(
            EventSource::Component,
            BoundedTargets::new(vec![EventTarget::SelfIsland])?,
            EventOrder::PerSourceSequence,
            EventCyclePolicy::ForbidRepeatedIsland,
            1,
        )
    }

    /// Builds metadata with an explicit source, scope, ordering, cycle, and fanout contract.
    pub fn from_payload_with_contract<T: EventPayloadMetadata + 'static>(
        source: EventSource,
        targets: BoundedTargets,
        order: EventOrder,
        cycle: EventCyclePolicy,
        maximum_fanout: u16,
    ) -> Result<Self, MetadataError> {
        let maximum_fanout = NonZeroU16::new(maximum_fanout)
            .filter(|fanout| fanout.get() <= MAX_EVENT_FANOUT)
            .ok_or_else(|| MetadataError::new(MetadataErrorKind::InvalidEventFanout))?;
        if usize::from(maximum_fanout.get()) < targets.as_slice().len() {
            return Err(MetadataError::new(MetadataErrorKind::InvalidEventFanout));
        }
        Ok(Self {
            name: BrowserOperationName::parse(T::NAME)
                .map_err(|_| MetadataError::new(MetadataErrorKind::InvalidIdentity))?,
            version: valid_payload_version(T::VERSION)?,
            payload_type: TypeId::of::<T>(),
            payload_contract: PayloadContractIdentity::parse(T::PAYLOAD_CONTRACT)
                .map_err(|_| MetadataError::new(MetadataErrorKind::InvalidIdentity))?,
            schema: T::SCHEMA,
            source,
            targets,
            order,
            cycle,
            maximum_fanout,
        })
    }

    /// Returns the stable browser event identity.
    #[must_use]
    pub const fn name(&self) -> &BrowserOperationName {
        &self.name
    }

    /// Returns the event payload version.
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }

    /// Returns the stable payload contract identity.
    #[must_use]
    pub const fn payload_contract(&self) -> &PayloadContractIdentity {
        &self.payload_contract
    }

    /// Returns the browser-visible root payload schema.
    #[must_use]
    pub const fn schema(&self) -> BrowserPayloadSchema {
        self.schema
    }

    /// Returns the trusted event source kind.
    #[must_use]
    pub const fn source(&self) -> EventSource {
        self.source
    }

    /// Returns target-specific propagation scopes in canonical order.
    #[must_use]
    pub const fn targets(&self) -> &BoundedTargets {
        &self.targets
    }

    /// Returns the delivery ordering contract.
    #[must_use]
    pub const fn order(&self) -> EventOrder {
        self.order
    }

    /// Returns the event cycle-prevention contract.
    #[must_use]
    pub const fn cycle(&self) -> EventCyclePolicy {
        self.cycle
    }

    /// Returns the maximum delivery fanout.
    #[must_use]
    pub const fn maximum_fanout(&self) -> NonZeroU16 {
        self.maximum_fanout
    }

    pub(crate) fn matches_payload<T: EventPayloadMetadata + 'static>(&self) -> bool {
        self.payload_type == TypeId::of::<T>()
            && self.payload_contract.as_str() == T::PAYLOAD_CONTRACT
            && self.name.as_str() == T::NAME
            && self.version == T::VERSION
            && self.schema == T::SCHEMA
    }

    pub(crate) const fn payload_type(&self) -> TypeId {
        self.payload_type
    }
}

/// Canonical metadata for one declared browser effect payload.
#[derive(Clone, Eq, PartialEq)]
pub struct EffectMetadata {
    name: BrowserOperationName,
    version: u16,
    payload_type: TypeId,
}

impl fmt::Debug for EffectMetadata {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EffectMetadata")
            .field("name", &self.name.as_str())
            .field("version", &self.version)
            .finish()
    }
}

impl EffectMetadata {
    /// Builds metadata through a declared versioned payload type.
    pub fn from_payload<T: EffectPayloadMetadata + 'static>() -> Result<Self, MetadataError> {
        Ok(Self {
            name: BrowserOperationName::parse(T::NAME)
                .map_err(|_| MetadataError::new(MetadataErrorKind::InvalidIdentity))?,
            version: valid_payload_version(T::VERSION)?,
            payload_type: TypeId::of::<T>(),
        })
    }

    /// Returns the stable browser effect identity.
    #[must_use]
    pub const fn name(&self) -> &BrowserOperationName {
        &self.name
    }

    /// Returns the effect payload version.
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }

    pub(crate) fn matches_payload<T: EffectPayloadMetadata + 'static>(&self) -> bool {
        self.payload_type == TypeId::of::<T>()
            && self.name.as_str() == T::NAME
            && self.version == T::VERSION
    }

    pub(crate) const fn payload_type(&self) -> TypeId {
        self.payload_type
    }
}

fn valid_payload_version(version: u16) -> Result<u16, MetadataError> {
    if version == 0 {
        return Err(MetadataError::new(MetadataErrorKind::InvalidVersion));
    }
    Ok(version)
}

#[cfg(test)]
mod tests {
    use crate::async_updates::BrowserPayloadSchema;

    use super::*;

    struct DefaultPayloadContract;

    impl EventPayloadMetadata for DefaultPayloadContract {
        const NAME: &'static str = "default_payload_contract";
        const VERSION: u16 = 1;
    }

    struct InvalidPayloadContract;

    impl EventPayloadMetadata for InvalidPayloadContract {
        const NAME: &'static str = "invalid_payload_contract";
        const VERSION: u16 = 1;
        const PAYLOAD_CONTRACT: &'static str = "";
    }

    struct UnboundedPayloadContract;

    impl EventPayloadMetadata for UnboundedPayloadContract {
        const NAME: &'static str = "unbounded_payload_contract";
        const VERSION: u16 = 1;
        const PAYLOAD_CONTRACT: &'static str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    }

    struct RegisteredPayload;

    impl EventPayloadMetadata for RegisteredPayload {
        const NAME: &'static str = "registered_payload";
        const VERSION: u16 = 1;
        const SCHEMA: BrowserPayloadSchema = BrowserPayloadSchema::Json;
        const PAYLOAD_CONTRACT: &'static str = "shared_stable_label";
    }

    struct SpoofedPayload;

    impl EventPayloadMetadata for SpoofedPayload {
        const NAME: &'static str = "registered_payload";
        const VERSION: u16 = 1;
        const SCHEMA: BrowserPayloadSchema = BrowserPayloadSchema::Json;
        const PAYLOAD_CONTRACT: &'static str = "shared_stable_label";
    }

    const DEBUG_PAYLOAD_SENTINEL: &str = "debug_payload_identity_sentinel";

    struct DebugSentinelPayload;

    impl EventPayloadMetadata for DebugSentinelPayload {
        const NAME: &'static str = "debug_sentinel_event";
        const VERSION: u16 = 1;
        const PAYLOAD_CONTRACT: &'static str = DEBUG_PAYLOAD_SENTINEL;
    }

    #[test]
    fn payload_contract_identity_defaults_to_the_registered_event_identity() {
        let metadata = EventMetadata::from_payload::<DefaultPayloadContract>()
            .expect("default payload contract");

        assert_eq!(
            metadata.payload_contract().as_str(),
            DefaultPayloadContract::NAME
        );
    }

    #[test]
    fn payload_contract_identity_is_validated_and_bounded() {
        let invalid = EventMetadata::from_payload::<InvalidPayloadContract>()
            .expect_err("empty payload contract");
        assert_eq!(invalid.kind(), MetadataErrorKind::InvalidIdentity);

        let unbounded = EventMetadata::from_payload::<UnboundedPayloadContract>()
            .expect_err("unbounded payload contract");
        assert_eq!(unbounded.kind(), MetadataErrorKind::InvalidIdentity);
    }

    #[test]
    fn stable_payload_label_cannot_spoof_the_runtime_payload_type() {
        let metadata =
            EventMetadata::from_payload::<RegisteredPayload>().expect("registered payload");

        assert!(metadata.matches_payload::<RegisteredPayload>());
        assert!(!metadata.matches_payload::<SpoofedPayload>());
    }

    #[test]
    fn payload_contract_debug_output_does_not_reveal_the_stable_identity() {
        let identity =
            PayloadContractIdentity::parse(DEBUG_PAYLOAD_SENTINEL).expect("payload identity");
        let metadata =
            EventMetadata::from_payload::<DebugSentinelPayload>().expect("event metadata");

        assert!(!format!("{identity:?}").contains(DEBUG_PAYLOAD_SENTINEL));
        assert!(!format!("{metadata:?}").contains(DEBUG_PAYLOAD_SENTINEL));
    }
}
