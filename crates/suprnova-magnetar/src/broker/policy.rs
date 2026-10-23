//! Dossier-driven decisions the lease protocol needs but never branches on
//! a provider name to make: rotation detection and `invalid_grant`
//! handling, both derived purely from
//! [`crate::oauth::provider::RefreshPolicy`] and the actual provider
//! response (`docs/specs/suprnova-magnetar/11-token-broker.md`'s "Refresh
//! under rotation" section).

use std::time::Duration;

use crate::oauth::errors::OAuthProtocolError;
use crate::oauth::protocol::TokenSuccessResponse;
use crate::oauth::provider::InvalidGrantMeaning;

/// The RFC 6749 §5.2 error code this module treats as the `invalid_grant`
/// dossier signal.
const INVALID_GRANT: &str = "invalid_grant";

/// Whether a successful exchange rotated the refresh token.
///
/// [`crate::oauth::provider::RefreshPolicy`] (as Task 3's review settled
/// it) carries no static `rotates` flag, so rotation is operationalized
/// per-response instead of as a dossier fact: an exchange counts as a
/// rotation exactly when the provider issued a new `refresh_token` on this
/// response. The lease protocol only increments a record's generation, and
/// only stores a new encrypted refresh token, when this returns `true`;
/// otherwise it retains the previously stored refresh token untouched.
#[must_use]
pub(super) fn rotated(response: &TokenSuccessResponse) -> bool {
    response.refresh_token.is_some()
}

/// How the lease protocol should react to a failed provider exchange.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum FailureClass {
    /// The dossier says `invalid_grant` here is a reuse/exfiltration
    /// signal: revoke the family and fire the reuse hook.
    Reuse,
    /// The dossier says `invalid_grant` here is ordinary revocation,
    /// expiry, or an otherwise-invalid grant: mark the record revoked, no
    /// reuse hook.
    OrdinaryRevocation,
    /// A retriable upstream failure, with the provider's `Retry-After`
    /// when it supplied one. The claim is left to expire on its own bound
    /// rather than cleared early, so a genuinely transient failure does
    /// not race a concurrent reclaim.
    Retriable {
        /// The provider's stated retry delay, when present.
        retry_after: Option<Duration>,
    },
    /// A terminal, non-retriable failure unrelated to `invalid_grant`
    /// (malformed response, Magnetar-side provider misconfiguration,
    /// or an OAuth error code other than `invalid_grant`).
    Terminal,
}

/// Classify one provider exchange failure for the lease protocol, using
/// only [`InvalidGrantMeaning`] and the error's own typed shape -- never a
/// provider-name branch.
pub(super) fn classify(
    error: &OAuthProtocolError,
    invalid_grant_meaning: InvalidGrantMeaning,
) -> FailureClass {
    match error {
        OAuthProtocolError::ProviderReportedError { code, .. } if code == INVALID_GRANT => {
            match invalid_grant_meaning {
                InvalidGrantMeaning::ReuseOrExternalRevocation => FailureClass::Reuse,
                InvalidGrantMeaning::OrdinaryRevocation => FailureClass::OrdinaryRevocation,
            }
        }
        OAuthProtocolError::UpstreamUnavailable {
            retry_after_seconds,
            ..
        } => FailureClass::Retriable {
            retry_after: retry_after_seconds.map(Duration::from_secs),
        },
        _ => FailureClass::Terminal,
    }
}
