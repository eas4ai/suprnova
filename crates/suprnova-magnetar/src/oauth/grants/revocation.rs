//! RFC 7009 token revocation orchestration
//! (`docs/specs/suprnova-magnetar/09-oauth-engine.md`'s "Refresh and
//! revocation" section). The wire rendering and Body/Query placement live
//! in each provider's [`crate::oauth::provider::OAuthProvider::revoke`]
//! (Task 3, reviewed); this module is the caller-facing entry point (the
//! token broker, a later task, calls through here) plus input validation
//! and the "no revocation support" posture.

use crate::oauth::errors::{OAuthProtocolError, OAuthResult};
use crate::oauth::provider::{OAuthProvider, TokenHint};

/// Revoke one token through `provider`'s dossier-defined revocation
/// contract.
///
/// A provider without revocation support, or one that cannot fulfil this
/// specific request, surfaces that as an [`OAuthProtocolError`] from its own
/// [`OAuthProvider::revoke`] implementation -- this function never treats a
/// provider error as success, and never panics for an unsupported provider.
///
/// # Errors
///
/// Returns [`OAuthProtocolError::InvalidRequestShape`] for an empty `token`.
/// Otherwise propagates whatever [`OAuthProvider::revoke`] returns.
pub async fn execute(
    provider: &dyn OAuthProvider,
    token: &str,
    hint: TokenHint,
) -> OAuthResult<()> {
    if token.is_empty() {
        return Err(OAuthProtocolError::InvalidRequestShape {
            field: "token".to_owned(),
            message: "a revocation token must not be empty".to_owned(),
        });
    }
    provider.revoke(token, hint).await
}
