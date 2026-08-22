//! Validated endpoint byte and protocol policy.

use crate::protocol::ProtocolLimits;
use crate::snapshot::SnapshotLimits;

use super::{EndpointError, EndpointErrorKind};

/// Bounded protocol and snapshot policy owned by one Live endpoint.
#[derive(Clone, Debug)]
pub struct LiveEndpointConfig {
    protocol: ProtocolLimits,
    snapshot: SnapshotLimits,
    max_request_bytes: usize,
    max_response_bytes: usize,
}

impl LiveEndpointConfig {
    /// Creates an endpoint whose whole request and response bounds match the protocol ceiling.
    pub fn new(protocol: ProtocolLimits, snapshot: SnapshotLimits) -> Result<Self, EndpointError> {
        let max_request_bytes = protocol.input().max_bytes();
        if max_request_bytes == 0 {
            return Err(EndpointError::new(EndpointErrorKind::InvalidConfiguration));
        }
        Ok(Self {
            protocol,
            snapshot,
            max_request_bytes,
            max_response_bytes: max_request_bytes,
        })
    }

    /// Returns the whole request-body ceiling.
    #[must_use]
    pub const fn max_request_bytes(&self) -> usize {
        self.max_request_bytes
    }

    /// Returns the complete encoded response-body ceiling.
    #[must_use]
    pub const fn max_response_bytes(&self) -> usize {
        self.max_response_bytes
    }

    /// Applies a stricter complete-response ceiling without exceeding protocol input bounds.
    pub fn with_max_response_bytes(mut self, max: usize) -> Result<Self, EndpointError> {
        if max == 0 || max > self.protocol.input().max_bytes() {
            return Err(EndpointError::new(EndpointErrorKind::InvalidConfiguration));
        }
        self.max_response_bytes = max;
        Ok(self)
    }

    pub(crate) const fn protocol(&self) -> &ProtocolLimits {
        &self.protocol
    }

    pub(crate) const fn snapshot(&self) -> &SnapshotLimits {
        &self.snapshot
    }
}
