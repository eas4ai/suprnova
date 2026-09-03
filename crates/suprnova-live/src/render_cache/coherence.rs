//! Fresh, stale-servable, stale-on-error, and dead intervals; bounded
//! validation leases; age and warning metadata.

use super::policy::{FreshnessPolicy, RepresentationClass};

/// Where a representation stands relative to its intervals.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FreshnessState {
    /// Within the fresh interval.
    Fresh,
    /// Past fresh, within stale-servable.
    StaleServable,
    /// Past stale-servable, within stale-on-error.
    StaleOnError,
    /// Past every interval.
    Dead,
}

/// Evaluates freshness at `now_ms`; private representations are never stale
/// served, so past fresh they are dead.
#[must_use]
pub fn evaluate_freshness(
    policy: &FreshnessPolicy,
    class: RepresentationClass,
    published_at_ms: u64,
    now_ms: u64,
) -> FreshnessState {
    let age = now_ms.saturating_sub(published_at_ms);
    if age < policy.fresh_ms() {
        return FreshnessState::Fresh;
    }
    if class == RepresentationClass::PrivateCached {
        return FreshnessState::Dead;
    }
    let past_fresh = age - policy.fresh_ms();
    if past_fresh < policy.stale_servable_ms() {
        return FreshnessState::StaleServable;
    }
    if past_fresh < policy.stale_on_error_ms() {
        return FreshnessState::StaleOnError;
    }
    FreshnessState::Dead
}

/// Whole seconds since publication, for the `Age` header.
#[must_use]
pub fn age_seconds(published_at_ms: u64, now_ms: u64) -> u64 {
    now_ms.saturating_sub(published_at_ms) / 1_000
}

/// The `Warning` header value for a state, if any.
#[must_use]
pub fn warning_header(state: FreshnessState) -> Option<&'static str> {
    match state {
        FreshnessState::StaleServable | FreshnessState::StaleOnError => {
            Some("110 - \"Response is Stale\"")
        }
        FreshnessState::Fresh | FreshnessState::Dead => None,
    }
}

/// A bounded local proof that the authority was reread recently.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ValidationLease {
    granted_at_ms: u64,
    expires_at_ms: u64,
}

impl ValidationLease {
    /// Grants a lease of `max_age_ms` from `now_ms`.
    #[must_use]
    pub fn grant(now_ms: u64, max_age_ms: u64) -> Self {
        Self {
            granted_at_ms: now_ms,
            expires_at_ms: now_ms.saturating_add(max_age_ms),
        }
    }

    /// Valid only within the monotonic window; a clock before the grant fails
    /// closed.
    #[must_use]
    pub fn valid_at(&self, now_ms: u64) -> bool {
        now_ms >= self.granted_at_ms && now_ms < self.expires_at_ms
    }

    /// A hint may only shorten the lease.
    pub fn hint_invalidate(&mut self, at_ms: u64) {
        self.expires_at_ms = self.expires_at_ms.min(at_ms);
    }
}
