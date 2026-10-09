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
    latest_protocol: u16,
}

impl CompatibilityWindow {
    /// Returns the initial exact v1 compatibility window.
    #[must_use]
    pub const fn v1() -> Self {
        Self { latest_protocol: 1 }
    }

    /// Returns the v2 rolling window, accepting only whole v1 or whole v2 contracts.
    #[must_use]
    pub const fn v2() -> Self {
        Self { latest_protocol: 2 }
    }

    /// Evaluates the full triplet without guessing across breaking versions.
    #[must_use]
    pub const fn evaluate(self, observed: VersionSet) -> CompatibilityDecision {
        let whole_contract = observed.snapshot_schema == 1
            && observed.protocol == observed.runtime_contract
            && observed.protocol >= 1
            && observed.protocol <= self.latest_protocol;
        if whole_contract {
            CompatibilityDecision::Compatible
        } else {
            CompatibilityDecision::RefreshDocument
        }
    }
}
