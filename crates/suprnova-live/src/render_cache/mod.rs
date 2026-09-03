//! RenderCache: typed Complete representations of canonical documents with
//! the policy, variance, key, entry, storage, generation, coherence, and HTTP
//! contracts needed to prove safe reuse. The host adapts these contracts;
//! Live is complete without them.

/// Route and group policy, deterministic patches, and concrete-response
/// eligibility.
pub mod policy;

/// Variance dimensions, opaque private key material, and the classification
/// that only preserves or reduces sharing.
pub mod variance;

/// Canonical, bounded, versioned lookup keys derived from representation
/// identity.
pub mod key;

pub use key::{RenderKey, RenderKeyDimensions, RenderKeyInput};
pub use policy::{
    CoherenceMode, DeclineReason, Eligibility, FailurePolicy, FreshnessPolicy, PolicyPatch,
    QueryPolicy, QueryUnknown, RenderCachePolicy, RenderCachePolicyBuilder, RepresentationClass,
    ResponseSignals, SharedCachePolicy, StorageLayers,
};
pub use variance::{
    ClassificationOutcome, ClassificationReason, DimensionValue, ObservedContext, PrivateMaterial,
    VarianceDescriptor, VarianceDimension, classify,
};

/// Closed failure categories for RenderCache contracts; messages never carry
/// keys, bodies, or identity material.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum RenderCacheErrorKind {
    /// A policy or patch violated a bound or tried to widen sharing.
    PolicyInvalid,
    /// A variance dimension value exceeded its bound or was undeclared.
    VarianceInvalid,
    /// Key input exceeded a bound.
    KeyInvalid,
    /// A stored entry failed decoding, integrity, or a bound.
    EntryInvalid,
    /// The entry format is known but not supported by this build.
    EntryUnsupported,
    /// A store, ledger, or coordinator provider failed.
    ProviderUnavailable,
    /// A publication lost its fence.
    PublicationFenced,
}

/// A RenderCache contract violation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RenderCacheError {
    kind: RenderCacheErrorKind,
}

impl RenderCacheError {
    /// Creates an error of one closed kind.
    #[must_use]
    pub const fn new(kind: RenderCacheErrorKind) -> Self {
        Self { kind }
    }

    /// The closed kind.
    #[must_use]
    pub const fn kind(&self) -> RenderCacheErrorKind {
        self.kind
    }
}

impl std::fmt::Display for RenderCacheError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self.kind {
            RenderCacheErrorKind::PolicyInvalid => "render_cache_policy_invalid",
            RenderCacheErrorKind::VarianceInvalid => "render_cache_variance_invalid",
            RenderCacheErrorKind::KeyInvalid => "render_cache_key_invalid",
            RenderCacheErrorKind::EntryInvalid => "render_cache_entry_invalid",
            RenderCacheErrorKind::EntryUnsupported => "render_cache_entry_unsupported",
            RenderCacheErrorKind::ProviderUnavailable => "render_cache_provider_unavailable",
            RenderCacheErrorKind::PublicationFenced => "render_cache_publication_fenced",
        })
    }
}

impl std::error::Error for RenderCacheError {}
