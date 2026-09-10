//! The closed reason set behind a declined lookup (`outcome="declined"`).
//!
//! Every branch in [`super::middleware`]'s `lead_render` that records
//! [`super::middleware::LookupOutcome::Declined`] names one of these
//! variants from a typed value computed at that branch - never a boolean
//! reconstructed afterwards from the response or the request. Adding a
//! decline branch without threading a reason through does not compile,
//! because the payload is not optional and [`LookupDeclineReason::as_str`]
//! is an exhaustive match.

use suprnova_live::render_cache::DeclineReason;

/// Why a lookup declined to store or serve. Closed at compile time; the
/// `reason` telemetry label is [`LookupDeclineReason::as_str`] of the
/// variant, and no value here derives from request data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LookupDeclineReason {
    // Eligibility (mirrors the engine's `policy::DeclineReason`).
    /// The route policy is uncacheable.
    PolicyUncacheable,
    /// Not GET or HEAD.
    Method,
    /// Not a 200 canonical representation.
    Status,
    /// The body streams.
    Streaming,
    /// The response sets a cookie.
    SetsCookie,
    /// A hop-by-hop, connection, or per-request header is present.
    UnsafeHeaderName,
    // Observation.
    /// The collector's report overflowed, or an observed identity could
    /// not fit the observation window.
    ObservationOverflowed,
    /// The in-transaction ledger read failed.
    LedgerReadFailed,
    /// A stitched route's handler never began, so its content bucket is
    /// empty.
    HandlerNotBegun,
    // Classification narrowed to Uncacheable.
    /// A session value was read (not merely a session id).
    SessionValueRead,
    /// Secret configuration or feature context was read.
    SecretContextRead,
    /// Request context outside the declared variance affected rendering.
    UndeclaredContext,
    // Live document facts.
    /// An identity-bound island was mounted on a route that did not
    /// declare stitching.
    IdentityBoundWithoutStitching,
    /// A stitched route's capture is not a trustworthy account of the
    /// request.
    InvalidStitchCapture,
    /// A rendered document declared `NoStore`.
    NoStoreIntent,
    /// A public-seed island's promotion deadline could not be resolved.
    UnresolvableSeedDeadline,
    // Invariants over the key.
    /// A `PrivateCached` classification carried no reason at all.
    UnreasonedPrivateClass,
    /// The render observed a principal the key declares no dimension for.
    PrincipalUndeclared,
    /// The render observed a principal that differs from the key's value.
    PrincipalDivergent,
    /// The render observed a tenant the key declares no dimension for.
    TenantUndeclared,
    /// The render observed a tenant that differs from the key's value.
    TenantDivergent,
    /// The render observed a locale the key declares no dimension for.
    LocaleUndeclared,
    /// The render observed a locale that differs from the key's value.
    LocaleDivergent,
    // Publication.
    /// The seed's own promotion deadline elapsed before publication.
    SeedDeadlineElapsed,
    /// A response header could not be safely replayed.
    UnsafeHeaderValue,
    /// A stitched capture was marked invalid.
    CompositeCaptureInvalid,
    /// A stitched capture's slot count did not match the mounted islands.
    CompositeSlotCountMismatch,
    /// A stitched capture exceeded the maximum slot count.
    CompositeTooManySlots,
    /// The response body did not match the captured document digest.
    CompositeDigestMismatch,
    /// A captured slot's markup was empty.
    CompositeEmptySlot,
    /// A captured slot's markup could not be located in the body.
    CompositeSlotNotFound,
    /// A captured slot's markup, or its placement, could not be
    /// unambiguously resolved in the body.
    CompositeSlotAmbiguous,
    /// A named nested segment's representation class is `PrivateCached`,
    /// which no reauthorization mechanism can ever authorize from a named
    /// reference, so it could never resolve.
    CompositeNestedUnauthorizable,
    /// A named nested segment's representation class is wider than the
    /// including document's.
    CompositeNestedWiderClass,
    /// A named nested segment's freshness window is longer than the
    /// including document's.
    CompositeNestedLongerFreshness,
    /// A named nested segment would exceed `MAX_NESTING_DEPTH` once resolved.
    CompositeNestedDepthExceeded,
    /// A named nested segment would include the publishing entry, directly
    /// or transitively.
    CompositeNestedCycle,
    /// A named nested segment could not be resolved from the store to
    /// prove the narrowing rule holds.
    CompositeNestedUnresolvable,
}

impl LookupDeclineReason {
    /// The `reason` label: the variant name in `snake_case`.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::PolicyUncacheable => "policy_uncacheable",
            Self::Method => "method",
            Self::Status => "status",
            Self::Streaming => "streaming",
            Self::SetsCookie => "sets_cookie",
            Self::UnsafeHeaderName => "unsafe_header_name",
            Self::ObservationOverflowed => "observation_overflowed",
            Self::LedgerReadFailed => "ledger_read_failed",
            Self::HandlerNotBegun => "handler_not_begun",
            Self::SessionValueRead => "session_value_read",
            Self::SecretContextRead => "secret_context_read",
            Self::UndeclaredContext => "undeclared_context",
            Self::IdentityBoundWithoutStitching => "identity_bound_without_stitching",
            Self::InvalidStitchCapture => "invalid_stitch_capture",
            Self::NoStoreIntent => "no_store_intent",
            Self::UnresolvableSeedDeadline => "unresolvable_seed_deadline",
            Self::UnreasonedPrivateClass => "unreasoned_private_class",
            Self::PrincipalUndeclared => "principal_undeclared",
            Self::PrincipalDivergent => "principal_divergent",
            Self::TenantUndeclared => "tenant_undeclared",
            Self::TenantDivergent => "tenant_divergent",
            Self::LocaleUndeclared => "locale_undeclared",
            Self::LocaleDivergent => "locale_divergent",
            Self::SeedDeadlineElapsed => "seed_deadline_elapsed",
            Self::UnsafeHeaderValue => "unsafe_header_value",
            Self::CompositeCaptureInvalid => "composite_capture_invalid",
            Self::CompositeSlotCountMismatch => "composite_slot_count_mismatch",
            Self::CompositeTooManySlots => "composite_too_many_slots",
            Self::CompositeDigestMismatch => "composite_digest_mismatch",
            Self::CompositeEmptySlot => "composite_empty_slot",
            Self::CompositeSlotNotFound => "composite_slot_not_found",
            Self::CompositeSlotAmbiguous => "composite_slot_ambiguous",
            Self::CompositeNestedUnauthorizable => "composite_nested_unauthorizable",
            Self::CompositeNestedWiderClass => "composite_nested_wider_class",
            Self::CompositeNestedLongerFreshness => "composite_nested_longer_freshness",
            Self::CompositeNestedDepthExceeded => "composite_nested_depth_exceeded",
            Self::CompositeNestedCycle => "composite_nested_cycle",
            Self::CompositeNestedUnresolvable => "composite_nested_unresolvable",
        }
    }

    /// Every variant, for the documentation test.
    #[cfg(any(test, feature = "testing"))]
    pub(crate) const ALL: &'static [LookupDeclineReason] = &[
        Self::PolicyUncacheable,
        Self::Method,
        Self::Status,
        Self::Streaming,
        Self::SetsCookie,
        Self::UnsafeHeaderName,
        Self::ObservationOverflowed,
        Self::LedgerReadFailed,
        Self::HandlerNotBegun,
        Self::SessionValueRead,
        Self::SecretContextRead,
        Self::UndeclaredContext,
        Self::IdentityBoundWithoutStitching,
        Self::InvalidStitchCapture,
        Self::NoStoreIntent,
        Self::UnresolvableSeedDeadline,
        Self::UnreasonedPrivateClass,
        Self::PrincipalUndeclared,
        Self::PrincipalDivergent,
        Self::TenantUndeclared,
        Self::TenantDivergent,
        Self::LocaleUndeclared,
        Self::LocaleDivergent,
        Self::SeedDeadlineElapsed,
        Self::UnsafeHeaderValue,
        Self::CompositeCaptureInvalid,
        Self::CompositeSlotCountMismatch,
        Self::CompositeTooManySlots,
        Self::CompositeDigestMismatch,
        Self::CompositeEmptySlot,
        Self::CompositeSlotNotFound,
        Self::CompositeSlotAmbiguous,
        Self::CompositeNestedUnauthorizable,
        Self::CompositeNestedWiderClass,
        Self::CompositeNestedLongerFreshness,
        Self::CompositeNestedDepthExceeded,
        Self::CompositeNestedCycle,
        Self::CompositeNestedUnresolvable,
    ];
}

impl From<DeclineReason> for LookupDeclineReason {
    /// The eligibility check's six reasons map one to one; only the header
    /// name is spelled differently, to distinguish it from
    /// [`LookupDeclineReason::UnsafeHeaderValue`] (a replayable header whose
    /// *value* cannot be stored, decided later, at publication).
    fn from(reason: DeclineReason) -> Self {
        match reason {
            DeclineReason::PolicyUncacheable => Self::PolicyUncacheable,
            DeclineReason::Method => Self::Method,
            DeclineReason::Status => Self::Status,
            DeclineReason::Streaming => Self::Streaming,
            DeclineReason::SetsCookie => Self::SetsCookie,
            DeclineReason::UnsafeHeader => Self::UnsafeHeaderName,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::LookupDeclineReason;

    #[test]
    fn every_reason_has_a_distinct_snake_case_label() {
        let mut seen = std::collections::HashSet::new();
        for reason in LookupDeclineReason::ALL {
            assert!(
                seen.insert(reason.as_str()),
                "duplicate reason label: {}",
                reason.as_str()
            );
            assert_eq!(reason.as_str(), reason.as_str().to_ascii_lowercase());
            assert!(!reason.as_str().contains(' '));
        }
    }
}
