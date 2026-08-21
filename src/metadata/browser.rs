//! Registered browser event and effect payload contracts.

use crate::identity::BrowserOperationName;

use super::{MetadataError, MetadataErrorKind};

/// Versioned payload metadata implemented by every declared browser event type.
pub trait EventPayloadMetadata {
    /// Stable browser event identity.
    const NAME: &'static str;
    /// Independently evolving event payload version.
    const VERSION: u16;
}

/// Versioned payload metadata implemented by every declared browser effect type.
pub trait EffectPayloadMetadata {
    /// Stable browser effect identity.
    const NAME: &'static str;
    /// Independently evolving effect payload version.
    const VERSION: u16;
}

/// Canonical metadata for one declared browser event payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventMetadata {
    name: BrowserOperationName,
    version: u16,
}

impl EventMetadata {
    /// Builds metadata through a declared versioned payload type.
    pub fn from_payload<T: EventPayloadMetadata>() -> Result<Self, MetadataError> {
        Ok(Self {
            name: BrowserOperationName::parse(T::NAME)
                .map_err(|_| MetadataError::new(MetadataErrorKind::InvalidIdentity))?,
            version: valid_payload_version(T::VERSION)?,
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
}

/// Canonical metadata for one declared browser effect payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectMetadata {
    name: BrowserOperationName,
    version: u16,
}

impl EffectMetadata {
    /// Builds metadata through a declared versioned payload type.
    pub fn from_payload<T: EffectPayloadMetadata>() -> Result<Self, MetadataError> {
        Ok(Self {
            name: BrowserOperationName::parse(T::NAME)
                .map_err(|_| MetadataError::new(MetadataErrorKind::InvalidIdentity))?,
            version: valid_payload_version(T::VERSION)?,
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
}

fn valid_payload_version(version: u16) -> Result<u16, MetadataError> {
    if version == 0 {
        return Err(MetadataError::new(MetadataErrorKind::InvalidVersion));
    }
    Ok(version)
}
