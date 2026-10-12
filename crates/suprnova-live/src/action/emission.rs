//! Registered typed browser event and effect payloads.

use std::any::TypeId;
use std::fmt;

use serde::Serialize;

use crate::canonical::{CanonicalValue, to_canonical_bytes};
use crate::identity::BrowserOperationName;
use crate::limits::InputLimits;
use crate::metadata::{EffectPayloadMetadata, EventPayloadMetadata};
use crate::registry::ComponentDescriptor;

use super::{OutcomeError, OutcomeErrorKind};

/// Typed event payload contract implemented explicitly by application event types.
pub trait LiveEventPayload: EventPayloadMetadata + Serialize + 'static {}

/// Typed browser-effect payload contract implemented explicitly by permitted effect types.
pub trait LiveEffectPayload: EffectPayloadMetadata + Serialize + 'static {}

/// Closed browser emission category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EmissionKind {
    /// A declared application-facing Live event.
    Event,
    /// A declared browser effect whose implementation belongs to the browser runtime.
    Effect,
}

/// Bounded canonical payload proven to occur in the current component descriptor.
#[derive(Clone, PartialEq)]
pub struct RegisteredEmission {
    kind: EmissionKind,
    name: BrowserOperationName,
    version: u16,
    payload_type: TypeId,
    payload: CanonicalValue,
}

impl RegisteredEmission {
    /// Encodes one descriptor-registered event payload.
    pub fn event<T: LiveEventPayload>(
        descriptor: &ComponentDescriptor,
        payload: &T,
        limits: &InputLimits,
    ) -> Result<Self, OutcomeError> {
        let registered = descriptor
            .metadata()
            .events()
            .iter()
            .any(|entry| entry.matches_payload::<T>());
        Self::encode(
            registered,
            EmissionKind::Event,
            T::NAME,
            T::VERSION,
            TypeId::of::<T>(),
            payload,
            limits,
        )
    }

    /// Encodes one descriptor-registered effect payload.
    pub fn effect<T: LiveEffectPayload>(
        descriptor: &ComponentDescriptor,
        payload: &T,
        limits: &InputLimits,
    ) -> Result<Self, OutcomeError> {
        let registered = descriptor
            .metadata()
            .effects()
            .iter()
            .any(|entry| entry.matches_payload::<T>());
        Self::encode(
            registered,
            EmissionKind::Effect,
            T::NAME,
            T::VERSION,
            TypeId::of::<T>(),
            payload,
            limits,
        )
    }

    fn encode<T: Serialize>(
        registered: bool,
        kind: EmissionKind,
        name: &str,
        version: u16,
        payload_type: TypeId,
        payload: &T,
        limits: &InputLimits,
    ) -> Result<Self, OutcomeError> {
        let name = BrowserOperationName::parse(name)
            .map_err(|_| OutcomeError::new(OutcomeErrorKind::InvalidPayload))?;
        if !registered {
            return Err(OutcomeError::new(OutcomeErrorKind::UnregisteredEmission));
        }
        let payload = serde_json::to_value(payload)
            .map_err(|_| OutcomeError::new(OutcomeErrorKind::InvalidPayload))?;
        let payload = CanonicalValue::from_serde_value(payload)
            .map_err(|_| OutcomeError::new(OutcomeErrorKind::InvalidPayload))?;
        to_canonical_bytes(&payload, limits)
            .map_err(|_| OutcomeError::new(OutcomeErrorKind::InvalidPayload))?;
        Ok(Self {
            kind,
            name,
            version,
            payload_type,
            payload,
        })
    }

    /// Returns the event/effect category.
    #[must_use]
    pub const fn kind(&self) -> EmissionKind {
        self.kind
    }

    /// Returns the registered stable browser operation identity.
    #[must_use]
    pub const fn name(&self) -> &BrowserOperationName {
        &self.name
    }

    /// Returns the independently versioned payload schema.
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }

    /// Returns the bounded canonical payload for protocol encoding.
    #[must_use]
    pub const fn payload(&self) -> &CanonicalValue {
        &self.payload
    }

    pub(crate) fn is_registered(&self, descriptor: &ComponentDescriptor) -> bool {
        match self.kind {
            EmissionKind::Event => descriptor.metadata().events().iter().any(|entry| {
                entry.name() == &self.name
                    && entry.version() == self.version
                    && entry.payload_type() == self.payload_type
            }),
            EmissionKind::Effect => descriptor.metadata().effects().iter().any(|entry| {
                entry.name() == &self.name
                    && entry.version() == self.version
                    && entry.payload_type() == self.payload_type
            }),
        }
    }
}

impl fmt::Debug for RegisteredEmission {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RegisteredEmission")
            .field("kind", &self.kind)
            .field("name", &self.name.as_str())
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}
