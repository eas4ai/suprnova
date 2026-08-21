//! Trusted framework-adapter context for seed promotion.

use std::fmt;

use crate::identity::ScopeFingerprint;
use crate::snapshot::ExpectedSeedV1;

/// Assertion that the owning Suprnova adapter completed request trust checks.
///
/// Iteration 001 deliberately does not implement Suprnova session, CSRF,
/// authorization, or tenant middleware. Constructing this marker asserts that
/// the owning adapter completed the checks required by its route before calling
/// the promotion service.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PromotionAttestations {
    verified_by_framework_adapter: bool,
}

impl PromotionAttestations {
    /// Asserts that the owning framework adapter completed its required checks.
    #[must_use]
    pub const fn verified() -> Self {
        Self {
            verified_by_framework_adapter: true,
        }
    }

    pub(crate) const fn is_verified(self) -> bool {
        self.verified_by_framework_adapter
    }
}

/// Adapter-supplied current compatibility, binding, scope, and trust context.
///
/// The type's `Trusted` name describes its boundary: values must be derived from
/// the current Suprnova request and registered component metadata, never copied
/// from browser fields. Iteration 001 consumes these assertions but does not
/// implement sessions, CSRF, authorization, or tenant middleware itself.
#[derive(Clone)]
pub struct TrustedPromotionContext {
    pub(crate) expected_seed: ExpectedSeedV1,
    pub(crate) scope: ScopeFingerprint,
    pub(crate) attestations: PromotionAttestations,
}

impl TrustedPromotionContext {
    /// Creates context from current adapter-owned expectations and scope.
    #[must_use]
    pub const fn new(
        expected_seed: ExpectedSeedV1,
        scope: ScopeFingerprint,
        attestations: PromotionAttestations,
    ) -> Self {
        Self {
            expected_seed,
            scope,
            attestations,
        }
    }

    /// Returns the trusted current principal/session/tenant scope fingerprint.
    #[must_use]
    pub const fn scope(&self) -> &ScopeFingerprint {
        &self.scope
    }
}

impl fmt::Debug for TrustedPromotionContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<TrustedPromotionContext:redacted>")
    }
}
