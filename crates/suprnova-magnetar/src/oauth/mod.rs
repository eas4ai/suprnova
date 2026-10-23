//! OAuth protocol engine (`docs/specs/suprnova-magnetar/09-oauth-engine.md`).
//!
//! This module exposes the I/O-free foundation (RFC 6749 §5 token-endpoint
//! wire types and declarative request-shape rendering), OAuth state
//! ceremonies with PKCE and session-binding modes, identity resolution and
//! account linking, email completion, the `OAuthProvider` trait + registry,
//! single-shot grant executors and RFC 7009 revocation, and (behind
//! `device-authorization`) RFC 8628 device authorization. It intentionally
//! excludes the token broker -- lease management and persisted/rotated
//! token records are a later iteration-003 task's.

/// OAuth state ceremonies: begin/callback lifecycle, PKCE, and binding
/// modes.
pub mod authorization;
/// Device authorization (RFC 8628): user-code/device-code ceremonies,
/// approve/deny, and polling.
#[cfg(feature = "device-authorization")]
pub mod device;
/// Mailed proof-of-ownership completion for no-trusted-email identities.
pub mod email_completion;
/// Errors raised by the protocol and request-shape types.
pub mod errors;
/// Single-shot grant executors (authorization_code, client_credentials,
/// jwt_bearer, refresh_token) and RFC 7009 revocation.
pub mod grants;
/// Identity resolution and account linking.
pub mod identity;
/// RFC 6749 §5 token-endpoint wire types.
pub mod protocol;
/// The `OAuthProvider` trait and provider registry.
pub mod provider;
/// Declarative, provider-agnostic OAuth request-shape rendering.
pub mod request_shape;

pub use authorization::{
    CeremonyBinding, OAUTH_AUTHORIZATION_KIND, OAUTH_BEGIN_PURPOSE, OAUTH_STATE_TTL,
    OAuthAuthorizationConfig, OAuthAuthorizationService, OAuthBeginInput, OAuthBegun,
    OAuthCallbackInput, OAuthCeremony, OAuthIntent,
};
pub use email_completion::{
    EmailCompletionConfig, EmailCompletionService, OAUTH_EMAIL_COMPLETION_PURPOSE,
    OAUTH_EMAIL_COMPLETION_TTL,
};
pub use errors::{OAuthErrorClass, OAuthErrorTraceContext, OAuthProtocolError, OAuthResult};
pub use identity::{
    AutoLinkPolicy, IdentityOutcome, IdentityResolver, OAUTH_PENDING_IDENTITY_KIND,
    VerifiedProviderIdentity,
};
pub use protocol::{
    OAuthErrorCode, TokenErrorResponse, TokenResponseBody, TokenSuccessResponse,
    parse_token_response_body,
};
pub use provider::{
    ClientAuthentication, ClientAuthenticationMaterial, EndpointOverrides, InvalidGrantMeaning,
    OAuthProvider, OAuthProviderRegistry, ParamPlacement, ProviderIdentity, ProviderResponse,
    RefreshPolicy, RevocationRequest, RevocationTransport, TokenHint,
};
pub use request_shape::{
    AuthorizationRequestParams, AuthorizationRequestShape, PkcePosture, TokenRequestParams,
    TokenRequestShape, render_authorization_request, render_token_request,
};
