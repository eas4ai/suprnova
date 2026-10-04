//! Declarative OAuth request shapes
//! (`docs/specs/suprnova-magnetar/09-oauth-engine.md`'s request-shape
//! surface).
//!
//! A provider's deviations from the RFC 6749 defaults -- parameter naming,
//! scope encoding, PKCE posture, and response mode -- are *data*
//! ([`AuthorizationRequestShape`], [`TokenRequestShape`]), not code. Every
//! provider-specific request renders through the single
//! [`render_authorization_request`]/[`render_token_request`] pair below; no
//! branch anywhere in this module inspects a provider name. 10's Apple and
//! TikTok plugins (and any community plugin) supply a shape value; the
//! engine takes no changes.
//!
//! Adapted in pattern (not in code, which is Magnetar's own) from
//! `arctic-oauth`'s per-provider request/PKCE/state helpers (MIT; see
//! `THIRD_PARTY_NOTICES.md` and `docs/provenance/source-ledger.md`) and the
//! `docs/specs/suprnova-magnetar/09-oauth-engine.md`/`10-providers.md`
//! dossier facts for Apple and TikTok.

use secrecy::{ExposeSecret, SecretString};

use crate::oauth::errors::{OAuthProtocolError, OAuthResult};

/// Whether an authorization-code flow attaches RFC 7636 PKCE parameters.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PkcePosture {
    /// `code_challenge`/`code_challenge_method` accompany the authorization
    /// request; `code_verifier` accompanies the token exchange. The 09
    /// engine default for every provider unless 10 records dossier
    /// evidence of rejection (Apple).
    Required,
    /// The provider rejects PKCE parameters on this flow (Apple: Apple
    /// rejects `code_challenge` on its authorization endpoint).
    Disabled,
}

/// The declarative shape of a provider's authorization request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorizationRequestShape {
    /// The query parameter name carrying the client identifier (`client_id`
    /// for RFC-compliant providers; TikTok uses `client_key`).
    pub client_id_param: String,
    /// The delimiter joining multiple scope tokens (space for RFC-compliant
    /// providers; TikTok uses a comma).
    pub scope_delimiter: String,
    /// Whether `scope` is sent even when the caller requested no scopes
    /// (TikTok always emits `scope`).
    pub always_send_scope: bool,
    /// Whether this flow attaches PKCE parameters.
    pub pkce: PkcePosture,
    /// An explicit `response_mode` override, when the provider requires one
    /// (Apple requires `form_post`).
    pub response_mode: Option<String>,
    /// Whether this flow attaches an OIDC `nonce` parameter, bound into the
    /// returned ID token and checked at identity-resolution time (Apple:
    /// PKCE is disabled for this provider, so `nonce` is its only
    /// authorization-request replay/injection defense -- 09's engine mints
    /// one at [`crate::oauth::authorization::OAuthAuthorizationService::begin`]
    /// whenever a provider's shape sets this).
    pub requires_nonce: bool,
}

impl Default for AuthorizationRequestShape {
    /// The RFC 6749 spec-compliant default: `client_id`, space-delimited
    /// scopes sent only when requested, PKCE required, no response-mode
    /// override, no nonce.
    fn default() -> Self {
        Self {
            client_id_param: "client_id".to_owned(),
            scope_delimiter: " ".to_owned(),
            always_send_scope: false,
            pkce: PkcePosture::Required,
            response_mode: None,
            requires_nonce: false,
        }
    }
}

/// The declarative shape of a provider's token request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenRequestShape {
    /// The form parameter name carrying the client identifier (`client_id`
    /// for RFC-compliant providers; TikTok uses `client_key`).
    pub client_id_param: String,
    /// The delimiter joining multiple scope tokens (space for RFC-compliant
    /// providers; TikTok uses a comma).
    pub scope_delimiter: String,
    /// Whether `scope` is sent even when the caller requested no scopes
    /// (TikTok always emits `scope`).
    pub always_send_scope: bool,
    /// Whether this provider may encode an OAuth error in an HTTP-success
    /// response body (TikTok's token endpoint returns errors with HTTP
    /// 200; a caller with this set must probe the body via
    /// [`crate::oauth::protocol::parse_token_response_body`] rather than
    /// trusting the status code).
    pub accept_http_success_error_body: bool,
}

impl Default for TokenRequestShape {
    /// The RFC 6749 spec-compliant default: `client_id`, space-delimited
    /// scopes sent only when requested, errors always distinguished by
    /// HTTP status.
    fn default() -> Self {
        Self {
            client_id_param: "client_id".to_owned(),
            scope_delimiter: " ".to_owned(),
            always_send_scope: false,
            accept_http_success_error_body: false,
        }
    }
}

/// Caller-supplied values for one authorization request.
///
/// Distinct from [`AuthorizationRequestShape`], which is fixed per
/// provider: these values vary per request (the requested scopes, the
/// generated CSRF state, the minted PKCE challenge).
#[derive(Clone, Debug, Default)]
pub struct AuthorizationRequestParams {
    /// The registered client identifier.
    pub client_id: String,
    /// The callback URI, when the client sends one explicitly.
    pub redirect_uri: Option<String>,
    /// The requested scopes, unencoded.
    pub scopes: Vec<String>,
    /// The CSRF state value to echo back on the callback.
    pub state: Option<String>,
    /// The RFC 7636 PKCE code challenge, when the caller minted one.
    pub code_challenge: Option<String>,
    /// The OIDC `nonce`, when the caller minted one
    /// ([`AuthorizationRequestShape::requires_nonce`]).
    pub nonce: Option<String>,
}

/// Caller-supplied values for one authorization-code token exchange.
///
/// Distinct from [`TokenRequestShape`], which is fixed per provider.
/// `code` and `code_verifier` are single-use credentials -- together they
/// are the complete token-exchange secret -- so both are wrapped in
/// [`SecretString`] to keep them out of derived `Debug` output, matching
/// [`crate::oauth::protocol::TokenSuccessResponse`].
#[derive(Clone, Debug, Default)]
pub struct TokenRequestParams {
    /// The registered client identifier.
    pub client_id: String,
    /// The authorization code returned by the callback.
    pub code: SecretString,
    /// The callback URI, when the original authorization request sent one.
    pub redirect_uri: Option<String>,
    /// The RFC 7636 PKCE code verifier, when the flow used PKCE.
    pub code_verifier: Option<SecretString>,
    /// The requested scopes, unencoded.
    pub scopes: Vec<String>,
}

/// Renders one provider's authorization request into ordered wire
/// parameters.
///
/// `response_type=code` is fixed by RFC 6749 §4.1.1; the PKCE challenge
/// method is fixed to `S256`, the only posture 09's engine allows (PKCE
/// S256 is the default for authorization-code flows). Every other
/// parameter -- the client-id parameter name, scope encoding, whether PKCE
/// parameters appear at all, and the response mode -- is driven entirely by
/// `shape`. No branch here inspects which provider `shape` came from.
///
/// The returned keys and values are **raw** (unencoded): the caller must
/// `application/x-www-form-urlencoded`-encode both before sending them,
/// whether as a query string or a request body.
///
/// # Errors
///
/// Returns [`OAuthProtocolError::InvalidRequestShape`] when `shape.pkce` is
/// [`PkcePosture::Required`] and `params.code_challenge` is `None`, or when
/// `shape.requires_nonce` is `true` and `params.nonce` is `None`.
pub fn render_authorization_request(
    shape: &AuthorizationRequestShape,
    params: &AuthorizationRequestParams,
) -> OAuthResult<Vec<(String, String)>> {
    let mut wire = Vec::new();
    wire.push((shape.client_id_param.clone(), params.client_id.clone()));
    wire.push(("response_type".to_owned(), "code".to_owned()));

    if let Some(redirect_uri) = &params.redirect_uri {
        wire.push(("redirect_uri".to_owned(), redirect_uri.clone()));
    }

    if shape.always_send_scope || !params.scopes.is_empty() {
        wire.push((
            "scope".to_owned(),
            params.scopes.join(&shape.scope_delimiter),
        ));
    }

    if let Some(state) = &params.state {
        wire.push(("state".to_owned(), state.clone()));
    }

    match shape.pkce {
        PkcePosture::Required => {
            let challenge = params.code_challenge.as_ref().ok_or_else(|| {
                OAuthProtocolError::InvalidRequestShape {
                    field: "code_challenge".to_owned(),
                    message:
                        "PKCE is required by this request shape but no code challenge was supplied"
                            .to_owned(),
                }
            })?;
            wire.push(("code_challenge".to_owned(), challenge.clone()));
            wire.push(("code_challenge_method".to_owned(), "S256".to_owned()));
        }
        // A disabled posture drops any caller-supplied challenge rather
        // than send it: providers that disable PKCE (Apple) reject the
        // parameter outright.
        PkcePosture::Disabled => {}
    }

    if let Some(response_mode) = &shape.response_mode {
        wire.push(("response_mode".to_owned(), response_mode.clone()));
    }

    if shape.requires_nonce {
        let nonce =
            params
                .nonce
                .as_ref()
                .ok_or_else(|| OAuthProtocolError::InvalidRequestShape {
                    field: "nonce".to_owned(),
                    message: "a nonce is required by this request shape but none was supplied"
                        .to_owned(),
                })?;
        wire.push(("nonce".to_owned(), nonce.clone()));
    }

    Ok(wire)
}

/// Renders one provider's authorization-code token exchange into ordered
/// wire parameters.
///
/// `grant_type=authorization_code` is fixed by RFC 6749 §4.1.3; every other
/// parameter is driven entirely by `shape`. No branch here inspects which
/// provider `shape` came from.
///
/// The returned keys and values are **raw** (unencoded): the caller must
/// `application/x-www-form-urlencoded`-encode both before sending them as
/// the token-request body.
pub fn render_token_request(
    shape: &TokenRequestShape,
    params: &TokenRequestParams,
) -> Vec<(String, String)> {
    let mut wire = Vec::new();
    wire.push((shape.client_id_param.clone(), params.client_id.clone()));
    wire.push(("grant_type".to_owned(), "authorization_code".to_owned()));
    wire.push(("code".to_owned(), params.code.expose_secret().to_owned()));

    if let Some(redirect_uri) = &params.redirect_uri {
        wire.push(("redirect_uri".to_owned(), redirect_uri.clone()));
    }

    if let Some(verifier) = &params.code_verifier {
        wire.push((
            "code_verifier".to_owned(),
            verifier.expose_secret().to_owned(),
        ));
    }

    if shape.always_send_scope || !params.scopes.is_empty() {
        wire.push((
            "scope".to_owned(),
            params.scopes.join(&shape.scope_delimiter),
        ));
    }

    wire
}
