//! Closed and redacted seed-promotion failures.

use std::error::Error;
use std::fmt;

/// Closed reason seed promotion did not produce instance authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PromotionErrorKind {
    /// A configured byte, duration, count, or cardinality bound was invalid.
    InvalidConfiguration,
    /// Encoded seed input exceeded the service preflight limit.
    InputTooLarge,
    /// Seed integrity, compatibility, binding, or validity verification failed.
    SnapshotRejected,
    /// Trusted request authority expired before promotion began.
    ContextRejected,
    /// A browser nonce was reused with different signed input.
    NonceConflict,
    /// An exact promotion for this nonce is currently pending.
    InProgress,
    /// A failed or cancelled reservation remains retained against immediate reuse.
    AbandonedRetention,
    /// The scoped fixed-window promotion rate was exceeded.
    RateLimited,
    /// The scope reached its live pending/accepted instance limit.
    OutstandingLimit,
    /// The route/component pair reached its live pending/accepted instance limit.
    RouteComponentLimit,
    /// Bounded reservation or rate-bucket storage reached capacity.
    StorageLimit,
    /// Server-controlled instance identity generation failed.
    RandomUnavailable,
    /// The configured instance ledger rejected or failed promotion.
    LedgerUnavailable,
    /// A provider returned metadata inconsistent with the promotion contract.
    ProviderInvariant,
}

impl PromotionErrorKind {
    /// Returns the stable safe machine value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidConfiguration => "invalid_promotion_configuration",
            Self::InputTooLarge => "promotion_input_too_large",
            Self::SnapshotRejected => "promotion_snapshot_rejected",
            Self::ContextRejected => "promotion_context_rejected",
            Self::NonceConflict => "promotion_nonce_conflict",
            Self::InProgress => "promotion_in_progress",
            Self::AbandonedRetention => "promotion_abandoned_retention",
            Self::RateLimited => "promotion_rate_limited",
            Self::OutstandingLimit => "promotion_outstanding_limit",
            Self::RouteComponentLimit => "promotion_route_component_limit",
            Self::StorageLimit => "promotion_storage_limit",
            Self::RandomUnavailable => "promotion_random_unavailable",
            Self::LedgerUnavailable => "promotion_ledger_unavailable",
            Self::ProviderInvariant => "promotion_provider_invariant",
        }
    }
}

/// Redacted promotion error that never includes seed, nonce, scope, or state bytes.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct PromotionError {
    kind: PromotionErrorKind,
}

impl PromotionError {
    pub(crate) const fn new(kind: PromotionErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the closed rejection reason.
    #[must_use]
    pub const fn kind(self) -> PromotionErrorKind {
        self.kind
    }
}

impl fmt::Display for PromotionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.as_str())
    }
}

impl fmt::Debug for PromotionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl Error for PromotionError {}
