//! Provider-neutral primary authentication and factor-gating contracts.
//!
//! Providers verify their own credentials and hand the host a private
//! [`VerifiedPrincipal`](crate::auth::VerifiedPrincipal). Session issuance remains entirely inside the factor
//! gate; provider implementations never receive a session-issuance witness.

mod factor_gate;
mod primary;
pub mod reauth;

pub use factor_gate::{
    FactorGate, FactorVerifier, OpaqueFactorGate, PreparedFactorProof, SignInDecision,
    TWO_FACTOR_CHALLENGE_KIND,
};
pub(crate) use primary::FactorGateApproval;
pub use primary::{
    AuthenticationContext, PrimaryAuth, PrimaryCredential, SignInMethod, VerifiedPrincipal,
};
