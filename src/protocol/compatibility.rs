//! Explicit protocol/runtime/snapshot compatibility windows.

/// Independently versioned control-contract triplet observed on one message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VersionSet {
    protocol: u16,
    runtime_contract: u16,
    snapshot_schema: u16,
}

impl VersionSet {
    /// Creates an observed version triplet.
    #[must_use]
    pub const fn new(protocol: u16, runtime_contract: u16, snapshot_schema: u16) -> Self {
        Self {
            protocol,
            runtime_contract,
            snapshot_schema,
        }
    }
}

/// Safe compatibility decision for a rolling-deployment version triplet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompatibilityDecision {
    /// All independently versioned contracts are within the supported window.
    Compatible,
    /// Obtain one current document and asset set instead of guessing compatibility.
    RefreshDocument,
}

/// Closed exact-version compatibility window implemented by iteration 001.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompatibilityWindow {
    protocol: u16,
    runtime_contract: u16,
    snapshot_schema: u16,
}

impl CompatibilityWindow {
    /// Returns the initial exact v1 compatibility window.
    #[must_use]
    pub const fn v1() -> Self {
        Self {
            protocol: 1,
            runtime_contract: 1,
            snapshot_schema: 1,
        }
    }

    /// Evaluates the full triplet without guessing across breaking versions.
    #[must_use]
    pub const fn evaluate(self, observed: VersionSet) -> CompatibilityDecision {
        if observed.protocol == self.protocol
            && observed.runtime_contract == self.runtime_contract
            && observed.snapshot_schema == self.snapshot_schema
        {
            CompatibilityDecision::Compatible
        } else {
            CompatibilityDecision::RefreshDocument
        }
    }
}
