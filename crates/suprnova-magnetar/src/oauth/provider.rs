//! The `OAuthProvider` abstraction and provider registry
//! (`docs/specs/suprnova-magnetar/09-oauth-engine.md`'s "Provider model:
//! config + quirk handlers"; `docs/specs/suprnova-magnetar/10-providers.md`'s
//! dossier discipline).
//!
//! A provider is data (its [`AuthorizationRequestShape`]/[`TokenRequestShape`],
//! per Task 1) plus the two operations that cannot be reduced to data:
//! turning an already-fetched provider response into a
//! [`VerifiedProviderIdentity`], and revoking a token. Neither operation
//! performs I/O itself. `resolve_identity` receives a [`ProviderResponse`]
//! the *host* already fetched (a userinfo/graph-API body, or Apple's signed
//! ID token) and only parses/verifies it; `revoke` renders a
//! [`RevocationRequest`] and hands it to a host-supplied
//! [`RevocationTransport`] rather than calling out over the network itself.
//! This is the seam `docs/specs/suprnova-magnetar/10-providers.md`'s Apple
//! and TikTok sections describe as "any network seam is a trait the host
//! implements" -- offline test suites supply fakes for both traits.
//!
//! `resolve_identity`'s ID-token/userinfo verification IS provider-specific
//! code (09's "quirk handler for identity mapping"), unlike the declarative
//! [`AuthorizationRequestShape`]/[`TokenRequestShape`] surface: every
//! provider must implement it, but no engine function branches on which
//! provider produced the [`ProviderResponse`] it consumes.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use secrecy::SecretString;

use super::errors::{OAuthProtocolError, OAuthResult};
use super::identity::VerifiedProviderIdentity;
use super::request_shape::{AuthorizationRequestShape, TokenRequestShape};

/// The identity an [`OAuthProvider::resolve_identity`] call yields.
///
/// This is exactly [`VerifiedProviderIdentity`] -- Task 2 defined that seam
/// in anticipation of this trait ("already authenticated by an
/// `OAuthProvider` plugin (Task 3)"; `src/oauth/identity.rs`'s module doc).
/// Task 3 does not duplicate it under a competing shape.
pub type ProviderIdentity = VerifiedProviderIdentity;

/// Per-provider endpoint URL overrides
/// (`docs/specs/suprnova-magnetar/09-oauth-engine.md` line 84:
/// "`EndpointOverrides` reaches every endpoint the engine calls -- authorize,
/// token, userinfo, revocation, device endpoints").
///
/// Every field defaults to the provider's real dossier URL when `None`; a
/// host constructs a provider with [`EndpointOverrides::default`] for real
/// traffic, or with individual fields populated to redirect a provider's
/// accessors at a fake endpoint for offline mock-driven suites. Pure
/// config: no [`OAuthProvider`] implementation branches on whether a field
/// is set, it only substitutes what an endpoint accessor returns.
///
/// `device_authorization_endpoint`/`device_token_endpoint` exist for a
/// first-party provider that itself exposes an RFC 8628 device-authorization
/// grant this engine would call into as a *client*. None of this crate's
/// five dossiers do. Magnetar's own device-authorization endpoints -- this
/// crate acting as the authorization *server*, per
/// [`crate::oauth::device`] -- are configured through
/// [`crate::oauth::device::DeviceAuthorizationConfig`] instead, since the
/// engine serves those rather than calling them.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EndpointOverrides {
    /// Overrides [`OAuthProvider::authorization_endpoint`].
    pub authorization_endpoint: Option<String>,
    /// Overrides [`OAuthProvider::token_endpoint`].
    pub token_endpoint: Option<String>,
    /// Overrides [`OAuthProvider::userinfo_endpoint`].
    pub userinfo_endpoint: Option<String>,
    /// Overrides the endpoint [`OAuthProvider::revoke`] renders a
    /// [`RevocationRequest`] against.
    pub revocation_endpoint: Option<String>,
    /// Reserved for a future device-authorization-capable provider (see
    /// this type's doc); unused by any of the five current dossiers.
    pub device_authorization_endpoint: Option<String>,
    /// Reserved for a future device-authorization-capable provider (see
    /// this type's doc); unused by any of the five current dossiers.
    pub device_token_endpoint: Option<String>,
}

/// Which token a [`OAuthProvider::revoke`] call targets.
///
/// Mirrors RFC 7009 §2.1's `token_type_hint`; [`Self::wire_value`] renders
/// the wire string a provider's revocation request expects.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TokenHint {
    /// The token being revoked is an access token.
    Access,
    /// The token being revoked is a refresh token.
    Refresh,
}

impl TokenHint {
    /// The RFC 7009 §2.1 `token_type_hint` wire value.
    #[must_use]
    pub fn wire_value(self) -> &'static str {
        match self {
            Self::Access => "access_token",
            Self::Refresh => "refresh_token",
        }
    }
}

/// The already-fetched provider response [`OAuthProvider::resolve_identity`]
/// parses and verifies.
///
/// Every field here was produced by the *host*, not the provider plugin --
/// providers never perform I/O
/// (`docs/specs/suprnova-magnetar/10-providers.md`'s per-provider identity
/// sections). Which variant a provider expects is itself a dossier fact:
/// Apple has no userinfo endpoint (identity lives in the signed ID token
/// returned by the token exchange); every other first-party provider here
/// resolves identity from a userinfo/graph-API GET the host performs against
/// the endpoint the provider's dossier names.
#[derive(Clone)]
pub enum ProviderResponse {
    /// A userinfo/graph-API JSON response body, already fetched by the host
    /// from the endpoint the provider's dossier names (Google, Facebook, X,
    /// TikTok).
    UserInfo {
        /// The raw, unparsed JSON response body.
        body: String,
    },
    /// Apple's signed ID token (JWT compact serialization) from the
    /// token-endpoint response, plus the nonce the authorization request
    /// carried (when one was minted), for the provider's JWKS-verification
    /// quirk handler to check against the token's `nonce` claim.
    AppleIdToken {
        /// The raw, unverified ID token. `resolve_identity` MUST verify its
        /// signature, issuer, audience, and expiry before trusting any
        /// claim inside it.
        id_token: SecretString,
        /// The nonce sent on the authorization request, when the caller
        /// minted one.
        nonce: Option<String>,
        /// The raw `user` JSON parameter from Apple's form_post callback
        /// body (`{"name":{"firstName":"...","lastName":"..."}}`),
        /// present only on the account's first authorization -- Apple
        /// never resends it, and it is never part of the ID token.
        form_post_user: Option<String>,
    },
}

/// How a provider authenticates the client on its token/revocation
/// endpoints -- a dossier fact Task 4's grant engine and Task 5's lease
/// broker consult when they render the actual HTTP request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClientAuthentication {
    /// `client_id`/`client_secret` (or the provider's equivalent parameter
    /// names, per [`TokenRequestShape::client_id_param`]) in the POST body.
    RequestBody,
    /// HTTP `Authorization: Basic` header (RFC 6749 §2.3.1).
    HttpBasic,
    /// A signed JWT client assertion minted per-request (Apple's ES256
    /// client secret).
    SignedJwt,
}

/// Whether a provider's `invalid_grant` response signals genuine external
/// spend/reuse of a rotated token (Task 5's broker must revoke the whole
/// token family and fire the reuse hook) or ordinary provider-side
/// revocation/expiry (mark the local record revoked without blaming the
/// caller).
///
/// Four of the five first-party providers document long-lived,
/// non-rotating refresh tokens: `invalid_grant` there means the token was
/// revoked, expired, or is otherwise no longer valid, never "someone else
/// already used the single-use token I'm holding" --
/// [`InvalidGrantMeaning::OrdinaryRevocation`]. TikTok is the exception:
/// its refresh-token-grant response documents that "the returned
/// `refresh_token` may be different than the one passed in the payload,"
/// i.e. TikTok rotates refresh tokens on every use
/// (`developers.tiktok.com/doc/oauth-user-access-token-management`,
/// verified live 2026-08-19), so an `invalid_grant` there can mean a
/// stale/already-rotated-out token was presented --
/// [`InvalidGrantMeaning::ReuseOrExternalRevocation`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidGrantMeaning {
    /// `invalid_grant` indicates the presented token was already consumed
    /// by another party -- a reuse/exfiltration signal.
    ReuseOrExternalRevocation,
    /// `invalid_grant` indicates ordinary revocation, expiry, or an
    /// otherwise-invalid grant with no reuse implication.
    OrdinaryRevocation,
}

/// Per-provider refresh/revocation dossier facts
/// (`docs/specs/suprnova-magnetar/10-providers.md`'s "refresh semantics,
/// revocation support" dossier line). Declarative, provider-agnostic data
/// consumed by Task 4's grant engine and Task 5's lease broker -- neither
/// this struct nor its consumers branch on a provider name to interpret it.
#[derive(Clone, Debug)]
pub struct RefreshPolicy {
    /// Whether this provider issues a refresh token from the
    /// authorization-code grant at all.
    pub supported: bool,
    /// How this provider authenticates the client on its token and
    /// revocation endpoints.
    pub token_client_authentication: ClientAuthentication,
    /// Extra authorization-request parameters a grant engine must add,
    /// beyond [`AuthorizationRequestShape`]'s wire pairs, for a refresh
    /// token to be issued (Google: `access_type=offline`).
    pub extra_authorization_params: Vec<(String, String)>,
    /// Extra scopes this provider requires for its refresh flow to succeed
    /// (X: `offline.access`).
    pub required_scopes: Vec<String>,
    /// Whether forcing re-consent (Google: `prompt=consent`) is needed to
    /// guarantee reissue of a refresh token on a repeat authorization.
    pub requires_reconsent_for_reissue: bool,
    /// How this provider's `invalid_grant` response should be interpreted.
    pub invalid_grant_meaning: InvalidGrantMeaning,
}

/// Where a [`RevocationRequest`]'s `params` are sent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParamPlacement {
    /// Form-encoded (`application/x-www-form-urlencoded`) request body --
    /// every RFC 7009-shaped provider here (Apple, Google, X, TikTok).
    Body,
    /// URL query string -- Facebook's `DELETE .../me/permissions`, whose
    /// Graph API takes `access_token` as a query parameter, not a body
    /// field (`DELETE` requests conventionally carry no body at all).
    Query,
}

/// A rendered revocation request, ready for a [`RevocationTransport`] to
/// send. Deliberately carries no [`Debug`]/[`Clone`] impl: `params`/
/// `headers` may include a raw token or client secret, and this type exists
/// only to cross the provider -> transport boundary once.
pub struct RevocationRequest {
    /// The HTTP method the provider's revocation endpoint expects
    /// (`"POST"` for every first-party provider here; Facebook's Graph API
    /// de-authorization is `"DELETE"`).
    pub method: &'static str,
    /// The absolute revocation endpoint URL.
    pub endpoint: String,
    /// Where `params` belong on the wire -- the struct alone cannot say
    /// this ("form-encoded body parameters" was wrong for Facebook, whose
    /// dossier documents a query parameter on a body-less `DELETE`); a
    /// transport MUST honor [`Self::placement`] rather than assume body.
    pub placement: ParamPlacement,
    /// Parameters, in the order the provider expects them. May contain the
    /// token being revoked and/or a client secret; never logged verbatim.
    pub params: Vec<(String, String)>,
    /// Additional request headers (X's HTTP Basic client authentication).
    pub headers: Vec<(String, String)>,
}

/// Client-authentication material a provider contributes to a token or
/// revocation request, rendered from its declared
/// [`RefreshPolicy::token_client_authentication`] posture.
///
/// Uniform across postures on purpose: a caller (Task 4's grant engine,
/// and this crate's own `revoke` implementations) merges `params` into the
/// outgoing request body/query and `headers` into the outgoing request
/// headers, without ever matching on [`ClientAuthentication`] itself --
/// [`ClientAuthentication::RequestBody`] and
/// [`ClientAuthentication::SignedJwt`] both resolve to a `client_secret`
/// body parameter (Apple's is a minted JWT instead of a static secret);
/// [`ClientAuthentication::HttpBasic`] resolves to an `Authorization`
/// header and no parameters. `client_id` is never included here -- it is
/// already part of [`crate::oauth::request_shape::TokenRequestParams`] and
/// [`AuthorizationRequestShape::client_id_param`]'s rendering; this seam
/// exists only for the credential material Task 1 deliberately left out of
/// the declarative params.
#[derive(Default)]
pub struct ClientAuthenticationMaterial {
    /// Parameters to add to the request body (or query, for a placement
    /// that uses one).
    pub params: Vec<(String, String)>,
    /// Headers to add to the request.
    pub headers: Vec<(String, String)>,
}

/// The revocation network seam a host implements. Providers never perform
/// I/O themselves (`docs/specs/suprnova-magnetar/10-providers.md`): a
/// provider's [`OAuthProvider::revoke`] renders a [`RevocationRequest`] and
/// hands it here. Offline test suites supply a recording fake.
#[async_trait]
pub trait RevocationTransport: Send + Sync {
    /// Send one revocation request. `Ok(())` only when the provider
    /// confirmed revocation (an RFC 7009 2xx, or the provider's documented
    /// success shape); a non-2xx or transport failure must return `Err`.
    async fn send(&self, request: RevocationRequest) -> OAuthResult<()>;
}

/// The provider contract 10's five first-party plugins (and any community
/// provider) implement. `docs/specs/suprnova-magnetar/09-oauth-engine.md`'s
/// provider model: config plus the identity/refresh escape hatches, never a
/// branch in the engine on provider name.
#[async_trait]
pub trait OAuthProvider: Send + Sync {
    /// Stable provider key -- the `{provider}` route segment and
    /// [`OAuthProviderRegistry`] lookup key.
    fn name(&self) -> &'static str;
    /// This provider's declarative authorization-request shape (Task 1's
    /// [`render_authorization_request`](super::request_shape::render_authorization_request)
    /// renders it).
    fn authorization_shape(&self) -> AuthorizationRequestShape;
    /// This provider's declarative token-request shape (Task 1's
    /// [`render_token_request`](super::request_shape::render_token_request)
    /// renders it).
    fn token_shape(&self) -> TokenRequestShape;
    /// Parse and verify an already-fetched provider response into a
    /// [`VerifiedProviderIdentity`]. Never performs I/O.
    async fn resolve_identity(&self, response: ProviderResponse) -> OAuthResult<ProviderIdentity>;
    /// Revoke a token via the provider's dossier-defined revocation
    /// request, sent through the injected [`RevocationTransport`].
    async fn revoke(&self, token: &str, hint: TokenHint) -> OAuthResult<()>;
    /// This provider's client identifier, as it belongs under
    /// [`AuthorizationRequestShape::client_id_param`]/
    /// [`TokenRequestShape::client_id_param`] (TikTok's `client_key` value,
    /// not a literal parameter name). Lets a caller (Task 4's grant
    /// engine) render a full token request -- `client_id` plus
    /// [`OAuthProvider::client_authentication`]'s secret material -- without
    /// reaching into provider-private config; configs themselves stay
    /// private.
    fn client_id(&self) -> &str;
    /// This provider's token-endpoint URL, as an owned `String` (Facebook's
    /// is versioned via its configured Graph API version, so it cannot be
    /// a static `&str`). Task 4's grant executors POST here directly
    /// through the host transport seam; providers keep the URL private
    /// otherwise, exactly as with [`OAuthProvider::client_id`].
    fn token_endpoint(&self) -> String;
    /// This provider's authorization-endpoint URL, as an owned `String`.
    /// A caller combines this with
    /// [`crate::oauth::request_shape::render_authorization_request`]'s
    /// wire parameters to build the full redirect URL; Task 3 left it
    /// provider-private the same way it originally left
    /// [`OAuthProvider::token_endpoint`] private, closed by the same
    /// [`EndpointOverrides`] seam.
    fn authorization_endpoint(&self) -> String;
    /// This provider's userinfo-endpoint URL, when it has one. `None` for
    /// a provider whose identity comes entirely from a signed token (Apple:
    /// "Apple has no userinfo endpoint", its dossier doc). The host
    /// performs the actual fetch (`ProviderResponse::UserInfo` is already
    /// host-produced input to [`OAuthProvider::resolve_identity`]); this
    /// accessor only tells the host *where*, so the URL lives in one
    /// override-aware place instead of being duplicated in host
    /// configuration.
    fn userinfo_endpoint(&self) -> Option<String>;
    /// This provider's refresh/revocation dossier facts.
    fn refresh_policy(&self) -> RefreshPolicy;
    /// Render this provider's client-authentication material for a token
    /// or revocation request (see [`ClientAuthenticationMaterial`]'s doc).
    /// Every provider implements this regardless of its declared
    /// [`ClientAuthentication`] posture, so a caller never needs to reach
    /// into provider-private config or match on the posture itself to
    /// authenticate a request -- including Apple's [`ClientAuthentication::SignedJwt`],
    /// which mints a fresh client secret on every call.
    async fn client_authentication(&self) -> OAuthResult<ClientAuthenticationMaterial>;
}

/// Providers keyed by [`OAuthProvider::name`] -- the `{provider}` route
/// segment (`docs/specs/suprnova-magnetar/09-oauth-engine.md`: "The registry
/// keys providers by name"). Task 5's broker and route layer look providers
/// up here; this crate performs no registration on its own -- the host
/// composes a registry from whichever first-party/community providers it
/// enables.
#[derive(Default)]
pub struct OAuthProviderRegistry {
    providers: HashMap<&'static str, Arc<dyn OAuthProvider>>,
}

impl OAuthProviderRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a provider under its own [`OAuthProvider::name`].
    ///
    /// # Errors
    ///
    /// Returns [`OAuthProtocolError::ProviderConfiguration`] when a
    /// provider is already registered under the same name -- matching
    /// [`crate::plugin::registry::PluginRegistryBuilder`]'s "duplicate
    /// plugin name" convention rather than silently clobbering the first
    /// registration.
    pub fn register(&mut self, provider: Arc<dyn OAuthProvider>) -> OAuthResult<&mut Self> {
        let name = provider.name();
        if self.providers.contains_key(name) {
            return Err(OAuthProtocolError::ProviderConfiguration {
                provider: name,
                message: "a provider is already registered under this name".to_owned(),
            });
        }
        self.providers.insert(name, provider);
        Ok(self)
    }

    /// Look up a provider by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<Arc<dyn OAuthProvider>> {
        self.providers.get(name).cloned()
    }

    /// Every registered provider name, sorted for deterministic iteration.
    #[must_use]
    pub fn names(&self) -> Vec<&'static str> {
        let mut names: Vec<&'static str> = self.providers.keys().copied().collect();
        names.sort_unstable();
        names
    }
}
