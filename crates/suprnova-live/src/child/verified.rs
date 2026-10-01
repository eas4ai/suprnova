//! Non-browser-constructible child and parent acceptance capabilities.

use std::fmt;

use super::{ChildParametersV1, ChildParametersV2};
use crate::canonical::CanonicalValue;
use crate::component::composition::ChildKey;
use crate::identity::{InstanceId, Revision, ScopeFingerprint};
use crate::ledger::AcceptedOutcomeMetadata;

/// Opaque proof sourced only from committed instance-ledger outcome metadata.
#[derive(Clone)]
pub struct AcceptedParentRevision {
    pub(crate) scope: ScopeFingerprint,
    pub(crate) instance: InstanceId,
    pub(crate) revision: Revision,
}

impl AcceptedParentRevision {
    /// Projects the parent authority retained by one committed ledger outcome.
    #[must_use]
    pub fn from_accepted_outcome(metadata: &AcceptedOutcomeMetadata) -> Self {
        Self {
            scope: metadata.scope.clone(),
            instance: metadata.instance_id.clone(),
            revision: metadata.successor_revision,
        }
    }
}

impl fmt::Debug for AcceptedParentRevision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<AcceptedParentRevision:redacted>")
    }
}

/// Verified child parameters that alone may enter `params_changed` dispatch.
#[derive(Clone)]
pub struct VerifiedChildParametersV1 {
    body: ChildParametersV1,
    child_key: ChildKey,
}

impl VerifiedChildParametersV1 {
    pub(crate) const fn new(body: ChildParametersV1, child_key: ChildKey) -> Self {
        Self { body, child_key }
    }

    /// Returns the registered, typed canonical parameter object.
    #[must_use]
    pub const fn parameters(&self) -> &CanonicalValue {
        self.body.parameters()
    }

    /// Returns the accepted parent revision for child-local ordering.
    #[must_use]
    pub const fn parent_revision(&self) -> Revision {
        self.body.parent_revision()
    }

    /// Returns the stable child key bound by the capability.
    #[must_use]
    pub const fn child_key(&self) -> &ChildKey {
        &self.child_key
    }
}

impl fmt::Debug for VerifiedChildParametersV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VerifiedChildParametersV1")
            .field("parent_revision", &self.parent_revision())
            .field("child_key", &self.child_key)
            .field("parameters", &"<redacted>")
            .finish()
    }
}

/// Verified exact-child-bound parameters awaiting server eligibility checks.
#[derive(Clone)]
pub struct VerifiedChildParametersV2 {
    pub(crate) body: ChildParametersV2,
    child_key: ChildKey,
}

impl VerifiedChildParametersV2 {
    pub(crate) const fn new(body: ChildParametersV2, child_key: ChildKey) -> Self {
        Self { body, child_key }
    }

    /// Returns the registered, typed canonical parameter object.
    #[must_use]
    pub const fn parameters(&self) -> &CanonicalValue {
        self.body.parameters()
    }

    /// Returns the accepted parent revision encoded by the issuer.
    #[must_use]
    pub const fn parent_revision(&self) -> Revision {
        self.body.parent_revision()
    }

    /// Returns the stable child key bound by the capability.
    #[must_use]
    pub const fn child_key(&self) -> &ChildKey {
        &self.child_key
    }

    /// Returns the exact child instance bound by the capability.
    #[must_use]
    pub const fn child_instance(&self) -> &InstanceId {
        self.body.child_instance()
    }
}

impl fmt::Debug for VerifiedChildParametersV2 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VerifiedChildParametersV2")
            .field("parent_revision", &self.parent_revision())
            .field("child_key", &self.child_key)
            .field("child_instance", &self.child_instance())
            .field("parameters", &"<redacted>")
            .finish()
    }
}
