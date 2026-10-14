//! Independent component contract versions.

use super::{MetadataError, MetadataErrorKind};
use crate::SUPPORTED_PROTOCOL_VERSIONS;

/// Independently evolving versions bound by one component contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContractVersions {
    component: u16,
    state_schema: u16,
    action_schema: u16,
    checker_contract: u16,
    minimum_protocol: u16,
}

impl ContractVersions {
    /// Creates a version set whose independent identities are all nonzero.
    pub fn new(
        component: u16,
        state_schema: u16,
        action_schema: u16,
        checker_contract: u16,
        minimum_protocol: u16,
    ) -> Result<Self, MetadataError> {
        if [
            component,
            state_schema,
            action_schema,
            checker_contract,
            minimum_protocol,
        ]
        .contains(&0)
        {
            return Err(MetadataError::new(MetadataErrorKind::InvalidVersion));
        }
        if !SUPPORTED_PROTOCOL_VERSIONS.contains(&minimum_protocol) {
            return Err(MetadataError::new(MetadataErrorKind::UnsupportedProtocol));
        }
        Ok(Self {
            component,
            state_schema,
            action_schema,
            checker_contract,
            minimum_protocol,
        })
    }

    /// Returns the component behavior contract version.
    #[must_use]
    pub const fn component(self) -> u16 {
        self.component
    }

    /// Returns the component state-schema version.
    #[must_use]
    pub const fn state_schema(self) -> u16 {
        self.state_schema
    }

    /// Returns the registered-action schema version.
    #[must_use]
    pub const fn action_schema(self) -> u16 {
        self.action_schema
    }

    /// Returns the checked-template contract version.
    #[must_use]
    pub const fn checker_contract(self) -> u16 {
        self.checker_contract
    }

    /// Returns the minimum Live protocol understood by the component.
    #[must_use]
    pub const fn minimum_protocol(self) -> u16 {
        self.minimum_protocol
    }
}
