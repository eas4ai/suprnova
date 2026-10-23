//! Least-privilege projection of validated host request authority.

use std::fmt;

use crate::host::TrustedLiveRequestContext;
use crate::identity::{ScopeFingerprint, UnixMillis};
use crate::snapshot::ExpectedSeedV1;

/// Seed-promotion authority projected only from a trusted Live request context.
#[derive(Clone)]
pub struct TrustedPromotionContext {
    pub(crate) expected_seed: ExpectedSeedV1,
    pub(crate) scope: ScopeFingerprint,
    expires_at: UnixMillis,
}

impl TrustedPromotionContext {
    pub(crate) fn from_request(request: &TrustedLiveRequestContext) -> Self {
        Self {
            expected_seed: request.mount().expected_seed().clone(),
            scope: request.scope().clone(),
            expires_at: request.expires_at(),
        }
    }

    /// Returns the trusted current principal/session/tenant scope fingerprint.
    #[must_use]
    pub const fn scope(&self) -> &ScopeFingerprint {
        &self.scope
    }

    pub(crate) fn ensure_current(&self, now: UnixMillis) -> bool {
        now < self.expires_at
    }
}

impl fmt::Debug for TrustedPromotionContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<TrustedPromotionContext:redacted>")
    }
}
