//! OAuth state ceremonies: begin/callback lifecycle, RFC 7636 PKCE, and the
//! session-binding modes (`docs/specs/suprnova-magnetar/09-oauth-engine.md`).
//!
//! State IS the ceremony selector (adapted in behavior, not code, from
//! `torii_integration/oauth.rs`'s `begin`/`verify_and_consume_ceremony`):
//! `begin` mints `(state, verifier, provider, intent, binding)` into 02's
//! [`CeremonyStore`] under [`OAUTH_AUTHORIZATION_KIND`]; `complete` consumes
//! it atomically by selector, giving exactly one callback winner. Unlike
//! Suprnova's framework-session lookup, the session-binding check here
//! compares an opaque 32-byte digest supplied by the caller at both ends
//! ([`CeremonyBinding::HostSessionDigest`]) -- Magnetar has no ambient
//! session of its own to consult. A web adapter supplies its initiating
//! data session's digest at `begin`; an API/standalone adapter opts out
//! explicitly with [`CeremonyBinding::StateOnly`]. The digest check runs
//! *before* the atomic consume, so a mismatch never mutates ceremony state.
//! Code exchange (Task 4) and identity resolution ([`super::identity`]) are
//! deliberately not composed here -- `complete` returns the consumed
//! [`OAuthCeremony`] and stops.

use std::sync::Arc;
use std::time::Duration as StdDuration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::abuse::{AbuseLimiter, AbusePolicy, Permit};
use crate::crypto::{CryptoPurpose, Encryptor};
use crate::oauth::request_shape::PkcePosture;
use crate::storage::{CeremonyStore, CredentialActor, NewCeremony};
use crate::{Error, Result};

/// Ceremony kind namespace for OAuth authorization ceremonies.
pub const OAUTH_AUTHORIZATION_KIND: &str = "oauth.authorization";

/// Grounded default ceremony lifetime (Suprnova's 10 minutes: "generous for
/// slow networks while keeping unused ceremonies pruned").
pub const OAUTH_STATE_TTL: StdDuration = StdDuration::from_secs(10 * 60);

/// Purpose namespace for the begin-time abuse limiter.
pub const OAUTH_BEGIN_PURPOSE: &str = "oauth-begin";

/// How an OAuth ceremony is bound to the caller that began it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CeremonyBinding {
    /// Bound to the initiating data session's token digest (today's
    /// login-CSRF protection, parity). The callback must present the same
    /// digest or the ceremony is rejected without being consumed.
    HostSessionDigest([u8; 32]),
    /// Explicitly session-unbound. Only API/standalone hosts may select
    /// this; the mode is explicit host configuration, never inferred.
    StateOnly,
}

/// Caller intent for a begun ceremony.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum OAuthIntent {
    /// A primary sign-in attempt; no actor is bound at begin time.
    SignIn,
    /// Linking a provider identity to an already-authenticated actor.
    Link {
        /// The begin-time authenticated actor. The callback's ambient
        /// session is never the link target -- only the value stored here.
        actor_user_id: String,
    },
}

/// Input to [`OAuthAuthorizationService::begin`].
pub struct OAuthBeginInput {
    /// Provider key (the `{provider}` route segment).
    pub provider: String,
    /// Caller intent: sign-in or link.
    pub intent: OAuthIntent,
    /// Trusted begin-time actor for a link intent. Sign-in must omit it.
    pub actor: Option<CredentialActor>,
    /// How the minted ceremony is bound to the caller.
    pub binding: CeremonyBinding,
}

/// Input to [`OAuthAuthorizationService::complete`].
pub struct OAuthCallbackInput {
    /// The `state` value echoed by the provider on the callback.
    pub state: String,
    /// The provider the callback claims to be from (the `{provider}` route
    /// segment the callback landed on).
    pub provider: String,
    /// The caller's current data-session digest, present only when the host
    /// adapter tracks one. Required and compared when the ceremony used
    /// [`CeremonyBinding::HostSessionDigest`]; ignored for
    /// [`CeremonyBinding::StateOnly`].
    pub host_session_digest: Option<[u8; 32]>,
}

/// A consumed authorization ceremony, ready for token exchange (Task 4).
pub struct OAuthCeremony {
    /// The ceremony selector (equal to [`OAuthCallbackInput::state`]).
    pub selector: String,
    /// The provider the ceremony was minted for.
    pub provider: String,
    /// The RFC 7636 PKCE code verifier, present when [`begin`](OAuthAuthorizationService::begin)
    /// minted one.
    pub verifier: Option<SecretString>,
    /// The OIDC nonce, present when `begin` minted one
    /// ([`crate::oauth::request_shape::AuthorizationRequestShape::requires_nonce`]).
    /// Hand this to [`crate::oauth::provider::ProviderResponse::AppleIdToken`]'s
    /// `nonce` field (or the equivalent seam for any other nonce-requiring
    /// provider) so `resolve_identity` can check it against the ID token's
    /// `nonce` claim.
    pub nonce: Option<String>,
    /// The begin-time caller intent.
    pub intent: OAuthIntent,
    /// The exact trusted actor captured at begin time, when linking.
    pub actor: Option<CredentialActor>,
    /// The ceremony's binding mode.
    pub binding: CeremonyBinding,
}

/// The result of [`OAuthAuthorizationService::begin`].
pub struct OAuthBegun {
    /// The minted ceremony selector; embed this as the `state` query
    /// parameter on the provider's authorization URL.
    pub selector: String,
    /// The RFC 7636 S256 code challenge, present when `pkce` was
    /// [`PkcePosture::Required`]; embed as `code_challenge` with
    /// `code_challenge_method=S256`.
    pub code_challenge: Option<String>,
    /// The minted OIDC nonce, present when `requires_nonce` was `true`;
    /// embed as `nonce` on the provider's authorization URL.
    pub nonce: Option<String>,
}

/// Route/provider-scoped configuration for [`OAuthAuthorizationService`].
#[derive(Clone, Copy, Debug)]
pub struct OAuthAuthorizationConfig {
    /// Abuse budget consulted before every `begin`.
    pub begin_policy: AbusePolicy,
}

impl Default for OAuthAuthorizationConfig {
    fn default() -> Self {
        Self {
            begin_policy: AbusePolicy {
                max_requests: 20,
                window: StdDuration::from_secs(60),
            },
        }
    }
}

#[derive(Serialize, Deserialize)]
struct CredentialActorSnapshot {
    user_id: String,
    issuance_epoch: u64,
    opaque_session_id: Option<String>,
    expires_at: Option<DateTime<Utc>>,
}

impl CredentialActorSnapshot {
    fn capture(actor: &CredentialActor) -> Self {
        Self {
            user_id: actor.user_id().to_owned(),
            issuance_epoch: actor.issuance_epoch(),
            opaque_session_id: actor.opaque_session_id().map(str::to_owned),
            expires_at: actor.expires_at(),
        }
    }

    fn into_actor(self) -> CredentialActor {
        CredentialActor::from_snapshot(
            self.user_id,
            self.issuance_epoch,
            self.opaque_session_id,
            self.expires_at,
        )
    }
}

/// Serialized, encrypted ceremony payload. `selector` is deliberately
/// excluded -- it already lives on the [`crate::storage::CeremonyRecord`]
/// row and is redundant to duplicate inside the encrypted blob.
#[derive(Serialize, Deserialize)]
struct CeremonyPayload {
    provider: String,
    /// Plaintext PKCE verifier. `secrecy::SecretString` does not implement
    /// `Serialize`/`Deserialize` without opting the wrapped type into
    /// `SerializableSecret` (deliberately not done upstream for `str`), so
    /// the encrypted-at-rest payload carries a plain `String` and the
    /// public API wraps/unwraps `SecretString` at the boundary.
    verifier: Option<String>,
    nonce: Option<String>,
    intent: OAuthIntent,
    actor: Option<CredentialActorSnapshot>,
    binding: CeremonyBinding,
}

/// OAuth state-ceremony lifecycle: begin and complete.
pub struct OAuthAuthorizationService {
    ceremonies: Arc<dyn CeremonyStore>,
    encryptor: Arc<dyn Encryptor>,
    limiter: Arc<dyn AbuseLimiter>,
    config: OAuthAuthorizationConfig,
}

impl OAuthAuthorizationService {
    /// Bind the service to ceremony storage, ceremony-state encryption, and
    /// the abuse limiter.
    pub fn new(
        ceremonies: Arc<dyn CeremonyStore>,
        encryptor: Arc<dyn Encryptor>,
        limiter: Arc<dyn AbuseLimiter>,
        config: OAuthAuthorizationConfig,
    ) -> Self {
        Self {
            ceremonies,
            encryptor,
            limiter,
            config,
        }
    }

    /// Begin an OAuth ceremony: acquire the begin-time abuse budget, mint an
    /// RFC 7636 PKCE verifier/challenge pair per `pkce` and (when
    /// `requires_nonce` is set by the provider's
    /// [`crate::oauth::request_shape::AuthorizationRequestShape`]) an OIDC
    /// nonce, and store
    /// `(state, verifier, nonce, provider, intent, binding)` under
    /// [`OAUTH_AUTHORIZATION_KIND`].
    ///
    /// `limiter_identity` is caller-supplied material to key the abuse
    /// budget by (a client IP for sign-in, the actor id for link) -- the
    /// same acquisition runs regardless of what that material resolves to,
    /// so a limiter observation alone never reveals account existence.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] for an empty provider or an empty
    /// link actor id, [`Error::Conflict`] when the abuse budget is
    /// exhausted, and [`Error::DependencyUnavailable`] when the limiter
    /// backend fails (fail closed: no ceremony is minted).
    pub async fn begin(
        &self,
        input: OAuthBeginInput,
        pkce: PkcePosture,
        requires_nonce: bool,
        limiter_identity: &str,
    ) -> Result<OAuthBegun> {
        if input.provider.is_empty() {
            return Err(invalid("provider", "must not be empty"));
        }
        match (&input.intent, input.actor.as_ref()) {
            (OAuthIntent::SignIn, None) => {}
            (OAuthIntent::SignIn, Some(_)) => {
                return Err(invalid("actor", "must be omitted for a sign-in intent"));
            }
            (OAuthIntent::Link { actor_user_id }, Some(actor))
                if !actor_user_id.is_empty() && actor.user_id() == actor_user_id => {}
            (OAuthIntent::Link { actor_user_id }, _) if actor_user_id.is_empty() => {
                return Err(invalid(
                    "actor_user_id",
                    "must not be empty for a link intent",
                ));
            }
            (OAuthIntent::Link { .. }, _) => {
                return Err(invalid(
                    "actor",
                    "must be present and match the link intent actor",
                ));
            }
        }

        self.acquire_begin_budget(&input.provider, limiter_identity)
            .await?;

        let selector = new_selector("oauth-state");
        let (verifier, code_challenge) = match pkce {
            PkcePosture::Required => {
                let verifier = random_verifier();
                let challenge = s256_challenge(verifier.expose_secret());
                (Some(verifier.expose_secret().to_owned()), Some(challenge))
            }
            PkcePosture::Disabled => (None, None),
        };
        let nonce = requires_nonce.then(random_nonce);

        let payload = CeremonyPayload {
            provider: input.provider,
            verifier,
            nonce: nonce.clone(),
            intent: input.intent,
            actor: input.actor.as_ref().map(CredentialActorSnapshot::capture),
            binding: input.binding,
        };
        let ciphertext = encrypt(self.encryptor.as_ref(), &payload)?;

        self.ceremonies
            .create(NewCeremony {
                selector: selector.clone(),
                kind: OAUTH_AUTHORIZATION_KIND.to_owned(),
                state: "pending".to_owned(),
                payload: ciphertext,
                expires_at: Utc::now()
                    + ChronoDuration::from_std(OAUTH_STATE_TTL)
                        .expect("OAUTH_STATE_TTL fits in chrono::Duration"),
            })
            .await?;

        Ok(OAuthBegun {
            selector,
            code_challenge,
            nonce,
        })
    }

    /// Complete an OAuth callback: validate the binding, atomically consume
    /// the ceremony, and confirm the provider matches.
    ///
    /// The binding check runs against a non-consuming [`CeremonyStore::peek`]
    /// *before* the atomic consume, so a missing/mismatched
    /// [`OAuthCallbackInput::host_session_digest`] never mutates ceremony
    /// state -- the ceremony remains available for the legitimate session to
    /// complete. A wrong-provider callback is checked only after the atomic
    /// consume (defense in depth, matching Suprnova's `verify_and_consume_ceremony`
    /// precedent): it is a caller/attacker error, and the tradeoff of
    /// burning the ceremony is accepted rather than adding a second
    /// non-atomic lookup.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] for an empty state/provider, a
    /// missing/expired/already-consumed ceremony, a binding mismatch (no
    /// mutation), or a provider mismatch (ceremony consumed).
    pub async fn complete(&self, input: OAuthCallbackInput) -> Result<OAuthCeremony> {
        if input.state.is_empty() {
            return Err(invalid("state", "must not be empty"));
        }
        if input.provider.is_empty() {
            return Err(invalid("provider", "must not be empty"));
        }

        let peeked = self
            .ceremonies
            .peek(&input.state, OAUTH_AUTHORIZATION_KIND)
            .await?
            .ok_or_else(ceremony_not_found)?;
        let payload: CeremonyPayload = decrypt(self.encryptor.as_ref(), &peeked.payload)?;
        check_binding(&payload.binding, input.host_session_digest)?;

        let consumed = self
            .ceremonies
            .consume(&input.state, OAUTH_AUTHORIZATION_KIND)
            .await?
            .ok_or_else(ceremony_not_found)?;
        let payload: CeremonyPayload = decrypt(self.encryptor.as_ref(), &consumed.payload)?;
        if payload.provider != input.provider {
            return Err(provider_mismatch());
        }

        Ok(OAuthCeremony {
            selector: input.state,
            provider: payload.provider,
            verifier: payload.verifier.map(SecretString::from),
            nonce: payload.nonce,
            intent: payload.intent,
            actor: payload.actor.map(CredentialActorSnapshot::into_actor),
            binding: payload.binding,
        })
    }

    async fn acquire_begin_budget(&self, provider: &str, limiter_identity: &str) -> Result<()> {
        let key = abuse_key(OAUTH_BEGIN_PURPOSE, provider, limiter_identity);
        match self.limiter.acquire(&key, self.config.begin_policy).await {
            Ok(Permit::Allowed { .. }) => Ok(()),
            Ok(Permit::Rejected { .. }) => Err(Error::Conflict {
                resource: "oauth.begin".to_owned(),
                message: "too many OAuth begin attempts, retry later".to_owned(),
            }),
            Err(error) => {
                tracing::error!(
                    error = %error,
                    provider,
                    "oauth begin abuse limiter unavailable; failing closed"
                );
                Err(Error::DependencyUnavailable {
                    dependency: "abuse-limiter".to_owned(),
                    message: error.to_string(),
                })
            }
        }
    }
}

fn check_binding(binding: &CeremonyBinding, presented: Option<[u8; 32]>) -> Result<()> {
    match binding {
        CeremonyBinding::StateOnly => Ok(()),
        CeremonyBinding::HostSessionDigest(expected) => match presented {
            Some(digest) if bool::from(expected.ct_eq(&digest)) => Ok(()),
            _ => Err(binding_mismatch()),
        },
    }
}

/// Build a purpose/provider-scoped abuse-limiter key from caller-supplied
/// identity material. The identity is digested so raw IPs/emails/actor ids
/// never reach the limiter backend.
fn abuse_key(purpose: &str, provider: &str, identity: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(identity.as_bytes());
    let digest = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{purpose}:{provider}:{digest}")
}

/// Generate a selector/token-shaped random identifier with a debugging
/// prefix. 128 bits of CSPRNG entropy, matching the shared factor gate's
/// challenge-selector precedent (`src/auth/factor_gate.rs`).
pub(crate) fn new_selector(prefix: &str) -> String {
    format!("{prefix}-{:032x}", rand::random::<u128>())
}

/// Mint an RFC 7636 code verifier: 32 bytes of CSPRNG entropy, base64
/// URL-safe (no padding) encoded -- a 43-character string inside the RFC's
/// 43-128 character allowed range.
fn random_verifier() -> SecretString {
    let mut bytes = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut bytes);
    SecretString::from(URL_SAFE_NO_PAD.encode(bytes))
}

/// RFC 7636 S256: `BASE64URL-ENCODE(SHA256(verifier))`.
fn s256_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Mint an OIDC `nonce`: 32 bytes of CSPRNG entropy, base64 URL-safe (no
/// padding) encoded. Not a credential (it is echoed back inside a public
/// ID token and never grants access on its own), so it is a plain
/// [`String`], not [`SecretString`].
fn random_nonce() -> String {
    let mut bytes = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

pub(crate) fn encrypt<T: Serialize>(encryptor: &dyn Encryptor, value: &T) -> Result<Vec<u8>> {
    let plaintext = serde_json::to_vec(value).map_err(|error| Error::Internal {
        message: format!("oauth ceremony payload serialization failed: {error}"),
    })?;
    encryptor.encrypt(CryptoPurpose::CeremonyState, &plaintext)
}

pub(crate) fn decrypt<T: for<'de> Deserialize<'de>>(
    encryptor: &dyn Encryptor,
    ciphertext: &[u8],
) -> Result<T> {
    let plaintext = encryptor.decrypt(CryptoPurpose::CeremonyState, ciphertext)?;
    serde_json::from_slice(&plaintext).map_err(|error| Error::InvalidInput {
        field: "ceremony".to_owned(),
        message: format!("invalid ceremony state: {error}"),
    })
}

fn invalid(field: &str, message: &str) -> Error {
    Error::InvalidInput {
        field: field.to_owned(),
        message: message.to_owned(),
    }
}

fn ceremony_not_found() -> Error {
    Error::InvalidInput {
        field: "state".to_owned(),
        message: "oauth state is missing, expired, or already consumed".to_owned(),
    }
}

fn binding_mismatch() -> Error {
    Error::InvalidInput {
        field: "host_session_digest".to_owned(),
        message: "oauth ceremony session binding mismatch".to_owned(),
    }
}

fn provider_mismatch() -> Error {
    Error::InvalidInput {
        field: "provider".to_owned(),
        message: "oauth state was issued for a different provider".to_owned(),
    }
}
