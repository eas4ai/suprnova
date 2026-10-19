//! User tokens signed by an outside authorization server (AGENT-006).
//!
//! An application that signs its users in somewhere else names that
//! authorization server, and the MCP endpoint accepts the JWT access tokens
//! it signs: each is checked against the server's published keys for its
//! signature, issuer, audience, expiry and scope, and its user is found by
//! a configured claim. Such a token is only ever a `user` caller.
//!
//! The keys are fetched the way Magnetar fetches a client metadata
//! document: HTTPS only, the host resolved here and every address it names
//! checked with [`is_public_address`] before anything connects, the
//! connection made to the checked address, no redirect followed, and the
//! body and the whole exchange bounded. Magnetar's own fetch is private to
//! it, so this is the same fence built on the public parts it is made of,
//! its [`HostResolver`] and [`PinnedHttpClient`]. The signature is checked
//! with `jsonwebtoken`, the crate Magnetar verifies its own JWTs with.

use std::collections::HashSet;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::Engine;
use jsonwebtoken::jwk::{AlgorithmParameters, Jwk, JwkSet, PublicKeyUse};
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use magnetar::authorization_server::{
    HostResolver, PinnedHttpClient, ReqwestPinnedClient, SystemResolver, is_public_address,
};
use serde_json::{Map, Value};
use url::{Host, Url};

use super::{CallerResolver, ResolveContext, ResolveError, ToolCaller};
use crate::auth::{Auth, Authenticatable};
use crate::error::FrameworkError;

/// The longest token read; anything longer is not one of this issuer's.
const MAX_TOKEN_LENGTH: usize = 16 * 1024;

/// Accepts the user tokens an outside authorization server signs.
///
/// ```rust,ignore
/// use suprnova::agents::{AgentsConfig, OutsideIssuer};
///
/// // The issuer's users sign in with their email, which it verifies and
/// // says so in `email_verified`; they are this application's users by
/// // that address.
/// let issuer = OutsideIssuer::new(
///     "https://login.example.com",
///     "https://login.example.com/.well-known/jwks.json",
/// )?
/// .user_claim("email")
/// .match_user_on("email");
/// let config = AgentsConfig::new().resolver(Arc::new(issuer));
/// ```
///
/// # Finding the user
///
/// By default the user is the one whose identifier is the token's `sub`.
/// That fits only an issuer that mints this application's own user ids,
/// such as one the application runs itself: an outside identity provider's
/// `sub` is its own identifier, which means nothing here.
///
/// To link users by that identifier, keep `sub` and match it against the
/// field the application stores it in: `.match_user_on("<provider>_sub")`.
/// The issuer assigns `sub` and never lets a user choose it, so this needs
/// no verified flag; no issuer sends a `sub_verified`.
///
/// Matching on any claim other than `sub`, chosen with
/// [`user_claim`](Self::user_claim), by identifier or with
/// [`match_user_on`](Self::match_user_on), requires the token to say the
/// issuer verified that claim: a flag claim, `<claim>_verified` as
/// OpenID Connect's `email_verified` and `phone_number_verified` are, or the
/// one [`verified_by`](Self::verified_by) names, set to JSON `true`. A
/// missing, `false` or non-boolean flag refuses the token. Without it, an
/// issuer that lets users set their own address would let anyone claim
/// `victim@example.com` and be signed in as that user here. There is no way
/// to turn the check off.
///
/// # What is checked
///
/// A token whose `iss` is not this issuer's is left to the next resolver.
/// One that is this issuer's must verify against a key the issuer
/// publishes, with an asymmetric algorithm, a `typ` of `JWT`, `at+jwt` or
/// none, an `aud` naming the MCP endpoint (or the audience set with
/// [`audience`](Self::audience)), an `exp` not past and an `nbf` and `iat`
/// not ahead, each within the leeway, and the user claim naming a user the
/// default guard's provider knows; otherwise it is refused. Its `scope`
/// (space-separated) or `scp` (a list) gives the caller's scopes, and its
/// `client_id` or `azp` the caller's client.
pub struct OutsideIssuer {
    issuer: String,
    audience: Option<String>,
    user_claim: String,
    match_field: Option<String>,
    verified_flag: Option<String>,
    leeway: Duration,
    keys: KeySet,
}

impl std::fmt::Debug for OutsideIssuer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OutsideIssuer")
            .field("issuer", &self.issuer)
            .field("jwks_uri", &self.keys.url.as_str())
            .field("audience", &self.audience)
            .field("user_claim", &self.user_claim)
            .finish_non_exhaustive()
    }
}

impl OutsideIssuer {
    /// Accept tokens whose `iss` is exactly `issuer`, verified against the
    /// key set (RFC 7517) at `jwks_uri`. The user is the one whose
    /// identifier is the `sub` claim, which fits only an issuer that mints
    /// this application's user ids (see "Finding the user"); the audience is
    /// the MCP endpoint's canonical URL; the leeway on times is one minute;
    /// keys are fetched through the system resolver and Magnetar's pinned
    /// HTTPS client within 64 KiB and 5 seconds, kept fresh 10 minutes with
    /// 10 more of grace while a refetch fails, and fetched at most once
    /// every 30 seconds.
    ///
    /// # Errors
    ///
    /// When `issuer` is empty, or `jwks_uri` is not an `https` URL without
    /// credentials or a fragment.
    pub fn new(issuer: impl Into<String>, jwks_uri: &str) -> Result<Self, FrameworkError> {
        let issuer = issuer.into();
        if issuer.trim().is_empty() {
            return Err(FrameworkError::internal(
                "an outside issuer needs its issuer identifier",
            ));
        }
        let url = Url::parse(jwks_uri).map_err(|error| {
            FrameworkError::internal(format!("the outside issuer's key set URL: {error}"))
        })?;
        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || url.host().is_none()
        {
            return Err(FrameworkError::internal(
                "the outside issuer's key set URL must be an https URL without credentials or a \
                 fragment",
            ));
        }
        let timeout = Duration::from_secs(5);
        Ok(Self {
            issuer,
            audience: None,
            user_claim: "sub".to_owned(),
            match_field: None,
            verified_flag: None,
            leeway: Duration::from_secs(60),
            keys: KeySet {
                url,
                resolver: Arc::new(SystemResolver),
                client: Arc::new(ReqwestPinnedClient::new(timeout)),
                max_bytes: 64 * 1024,
                timeout,
                max_age: Duration::from_secs(600),
                refetch_after: Duration::from_secs(30),
                state: Mutex::new(KeyState::default()),
                fetching: tokio::sync::Mutex::new(()),
            },
        })
    }

    /// Require `aud` to name `audience` instead of the MCP endpoint's
    /// canonical URL, for an issuer that names its resources otherwise.
    pub fn audience(mut self, audience: impl Into<String>) -> Self {
        self.audience = Some(audience.into());
        self
    }

    /// Find the user by the claim `claim` instead of `sub`: its value is
    /// the user's identifier, as the default guard's provider looks a user
    /// up by id, unless [`match_user_on`](Self::match_user_on) names a
    /// field. A claim other than `sub` must come with its verified flag
    /// (see "Finding the user").
    pub fn user_claim(mut self, claim: impl Into<String>) -> Self {
        self.user_claim = claim.into();
        self
    }

    /// Find the user whose `field` holds the user claim's value, through
    /// the provider's credential lookup with no password, rather than by
    /// identifier: `.match_user_on("oidc_sub")` links by the issuer's own
    /// `sub` and needs no flag, while `.user_claim("email").match_user_on("email")`
    /// needs the token's `email_verified` (see "Finding the user").
    pub fn match_user_on(mut self, field: impl Into<String>) -> Self {
        self.match_field = Some(field.into());
        self
    }

    /// Name the flag claim that says the issuer verified the user claim,
    /// for an issuer that does not call it `<claim>_verified`: for example
    /// `.user_claim("upn").verified_by("upn_checked")`. The token must carry
    /// it set to JSON `true` whenever the user claim is not `sub`. With the
    /// user claim `sub`, matched by identifier or against a field, no flag
    /// is read, since the issuer assigns `sub` itself.
    pub fn verified_by(mut self, flag: impl Into<String>) -> Self {
        self.verified_flag = Some(flag.into());
        self
    }

    /// How far a token's times may be off the server's clock: one minute
    /// unless set.
    pub fn leeway(mut self, leeway: Duration) -> Self {
        self.leeway = leeway;
        self
    }

    /// Fetch the key set through `resolver` and `client`. A test gives
    /// fakes of its own, so no test reaches the network; every address the
    /// resolver answers is still checked before the client is asked.
    pub fn key_fetch(
        mut self,
        resolver: Arc<dyn HostResolver>,
        client: Arc<dyn PinnedHttpClient>,
    ) -> Self {
        self.keys.resolver = resolver;
        self.keys.client = client;
        self
    }

    /// Keep a fetched key set fresh for `max_age`, after which the next
    /// request fetches it again. `refetch_after` bounds only fetching again
    /// for a key id the fresh set lacks, and fetching again after a failed
    /// fetch, so neither made-up key ids nor tokens arriving while the
    /// issuer is down can make the server fetch on every request. After a
    /// failed fetch, the last good set still answers for the keys it holds
    /// until it is twice `max_age` old; a key id it lacks, and every token
    /// after that, gets 503.
    pub fn key_cache(mut self, max_age: Duration, refetch_after: Duration) -> Self {
        self.keys.max_age = max_age;
        self.keys.refetch_after = refetch_after;
        self
    }

    /// The user the claim's value names, through the default guard's
    /// provider.
    async fn find_user(&self, value: &str) -> Result<Arc<dyn Authenticatable>, ResolveError> {
        let guard_name = Auth::default_guard_name();
        let Some(provider) = crate::auth::epoch::guard_provider(&guard_name) else {
            return Err(ResolveError::Unavailable(FrameworkError::internal(
                format!(
                    "the default guard '{guard_name}' has no user provider to load a token's user"
                ),
            )));
        };
        let found = match &self.match_field {
            None => provider.retrieve_by_id(value).await,
            Some(field) => {
                let mut credentials = Map::new();
                credentials.insert(field.clone(), Value::String(value.to_owned()));
                provider
                    .retrieve_by_credentials(&Value::Object(credentials))
                    .await
            }
        };
        match found {
            Ok(Some(user)) => Ok(user),
            Ok(None) => Err(ResolveError::Invalid("The token's user is not known here.")),
            Err(error) => Err(ResolveError::Unavailable(error)),
        }
    }
}

#[async_trait]
impl CallerResolver for OutsideIssuer {
    async fn resolve(
        &self,
        token: &str,
        ctx: &ResolveContext<'_>,
    ) -> Result<ToolCaller, ResolveError> {
        // Whether the token is this issuer's at all is read before its
        // signature: a token that is not a JWT, or names another issuer,
        // is left to the next resolver. Nothing read here is trusted until
        // the signature verifies.
        if token.len() > MAX_TOKEN_LENGTH {
            return Err(ResolveError::NotMine);
        }
        let Ok(header) = jsonwebtoken::decode_header(token) else {
            return Err(ResolveError::NotMine);
        };
        let Some(unverified) = unverified_claims(token) else {
            return Err(ResolveError::NotMine);
        };
        if unverified.get("iss").and_then(Value::as_str) != Some(self.issuer.as_str()) {
            return Err(ResolveError::NotMine);
        }
        if !is_access_token_type(header.typ.as_deref()) {
            return Err(ResolveError::Invalid("The token is not an access token."));
        }
        if !is_asymmetric(header.alg) {
            return Err(ResolveError::Invalid(
                "The token's signing algorithm is not accepted.",
            ));
        }
        let Some(kid) = header.kid.as_deref() else {
            return Err(ResolveError::Invalid("The token names no signing key."));
        };
        let jwk = match self.keys.key(kid).await {
            Ok(jwk) => jwk,
            Err(KeyError::Unknown) => {
                return Err(ResolveError::Invalid(
                    "The token's signing key is not one its issuer publishes.",
                ));
            }
            Err(KeyError::Unavailable(reason)) => {
                return Err(ResolveError::Unavailable(FrameworkError::internal(
                    format!("the outside issuer's key set could not be fetched: {reason}"),
                )));
            }
        };
        let Some(key) = decoding_key(&jwk, header.alg) else {
            return Err(ResolveError::Invalid(
                "The token's signing key does not fit its algorithm.",
            ));
        };
        let Ok(verified) =
            jsonwebtoken::decode::<Map<String, Value>>(token, &key, &bare_validation(header.alg))
        else {
            return Err(ResolveError::Invalid(
                "The token's signature does not verify.",
            ));
        };
        let claims = verified.claims;
        if claims.get("iss").and_then(Value::as_str) != Some(self.issuer.as_str()) {
            return Err(ResolveError::Invalid(
                "The token was issued by another server.",
            ));
        }
        let now = chrono::Utc::now().timestamp();
        let leeway = i64::try_from(self.leeway.as_secs()).unwrap_or(i64::MAX);
        let Some(expires) = numeric_date(&claims, "exp") else {
            return Err(ResolveError::Invalid("The token has no expiry."));
        };
        if expires.saturating_add(leeway) <= now {
            return Err(ResolveError::Invalid("The token expired."));
        }
        if numeric_date(&claims, "nbf")
            .is_some_and(|not_before| not_before.saturating_sub(leeway) > now)
        {
            return Err(ResolveError::Invalid("The token is not valid yet."));
        }
        if numeric_date(&claims, "iat").is_some_and(|issued| issued.saturating_sub(leeway) > now) {
            return Err(ResolveError::Invalid("The token was issued in the future."));
        }
        let audience = self.audience.as_deref().unwrap_or(ctx.resource);
        if !audience_names(&claims, audience) {
            return Err(ResolveError::Invalid(
                "The token was issued for another resource.",
            ));
        }
        let Some(subject) = claims
            .get(&self.user_claim)
            .and_then(Value::as_str)
            .filter(|subject| !subject.is_empty())
        else {
            return Err(ResolveError::Invalid("The token does not name its user."));
        };
        if self.user_claim != "sub" {
            let flag = self
                .verified_flag
                .clone()
                .unwrap_or_else(|| format!("{}_verified", self.user_claim));
            if claims.get(&flag) != Some(&Value::Bool(true)) {
                return Err(ResolveError::Invalid(
                    "The claim the user is matched on is not verified.",
                ));
            }
        }
        let user = self.find_user(subject).await?;
        let client = claims
            .get("client_id")
            .or_else(|| claims.get("azp"))
            .and_then(Value::as_str)
            .unwrap_or(&self.issuer)
            .to_owned();
        Ok(ToolCaller::user(user)
            .scopes(scopes(&claims))
            .client(client))
    }

    fn authorization_servers(&self) -> Vec<String> {
        vec![self.issuer.clone()]
    }
}

/// Whether a `typ` header marks an access token: none, `JWT`, or `at+jwt`
/// (RFC 9068), without case and with or without `application/`. An
/// identity assertion (`oauth-id-jag+jwt`) or any other typed JWT is not
/// one.
fn is_access_token_type(typ: Option<&str>) -> bool {
    let Some(typ) = typ else {
        return true;
    };
    let typ = typ.to_ascii_lowercase();
    let typ = typ.strip_prefix("application/").unwrap_or(&typ);
    typ == "jwt" || typ == "at+jwt"
}

/// The asymmetric algorithms an issuer may sign with. HMAC is never
/// accepted: its key would be a secret this application shares with the
/// issuer, and a published key set holds none.
fn is_asymmetric(algorithm: Algorithm) -> bool {
    matches!(
        algorithm,
        Algorithm::RS256
            | Algorithm::RS384
            | Algorithm::RS512
            | Algorithm::PS256
            | Algorithm::PS384
            | Algorithm::PS512
            | Algorithm::ES256
            | Algorithm::ES384
            | Algorithm::EdDSA
    )
}

/// The verification key a JWK gives for `algorithm`, or `None` for a
/// symmetric key, an encryption key, or a key pinned to another algorithm.
/// `jsonwebtoken` refuses a key of another family than the algorithm's.
fn decoding_key(jwk: &Jwk, algorithm: Algorithm) -> Option<DecodingKey> {
    if matches!(jwk.algorithm, AlgorithmParameters::OctetKey(_)) {
        return None;
    }
    if matches!(jwk.common.public_key_use, Some(PublicKeyUse::Encryption)) {
        return None;
    }
    if let Some(pinned) = &jwk.common.key_algorithm
        && format!("{pinned:?}") != format!("{algorithm:?}")
    {
        return None;
    }
    DecodingKey::from_jwk(jwk).ok()
}

/// A validation that checks the signature and the algorithm only; the
/// claims are checked by the resolver, which knows what each failure says.
fn bare_validation(algorithm: Algorithm) -> Validation {
    let mut validation = Validation::new(algorithm);
    validation.validate_exp = false;
    validation.validate_nbf = false;
    validation.validate_aud = false;
    validation.required_spec_claims = HashSet::new();
    validation
}

/// The payload of a JWT before its signature is checked, read only to see
/// whose token it is.
fn unverified_claims(token: &str) -> Option<Map<String, Value>> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(Value::Object(claims)) => Some(claims),
        _ => None,
    }
}

/// A numeric date claim (RFC 7519 section 2), whole seconds.
fn numeric_date(claims: &Map<String, Value>, name: &str) -> Option<i64> {
    let value = claims.get(name)?;
    value.as_i64().or_else(|| {
        value
            .as_f64()
            .filter(|seconds| seconds.is_finite())
            .map(|seconds| seconds as i64)
    })
}

/// Whether `aud`, a string or a list of strings, names `audience`.
fn audience_names(claims: &Map<String, Value>, audience: &str) -> bool {
    match claims.get("aud") {
        Some(Value::String(named)) => named == audience,
        Some(Value::Array(named)) => named
            .iter()
            .filter_map(Value::as_str)
            .any(|named| named == audience),
        _ => false,
    }
}

/// The token's scopes: `scope`, space-separated (RFC 9068), or else `scp`,
/// a list or a space-separated string. A `scope` that is not a string says
/// nothing, so `scp` is read.
fn scopes(claims: &Map<String, Value>) -> Vec<String> {
    let split = |text: &str| text.split_whitespace().map(str::to_owned).collect();
    if let Some(Value::String(text)) = claims.get("scope") {
        return split(text);
    }
    match claims.get("scp") {
        Some(Value::String(text)) => split(text),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

/// Why a key was not found.
enum KeyError {
    /// The issuer publishes no key with that id.
    Unknown,
    /// The key set could not be fetched.
    Unavailable(String),
}

/// The issuer's key set, fetched behind the fence and cached.
///
/// A fetched set is fresh for `max_age`; a set past it is fetched again on
/// the next request. `refetch_after` bounds two things only: fetching again
/// for a key id the fresh set lacks, so made-up key ids cannot make the
/// server fetch on every request, and fetching again after a failed fetch,
/// so tokens arriving while the issuer is down cannot either.
///
/// After a failed fetch, the last good set still answers for the keys it
/// holds until it is twice `max_age` old (a grace period of one more
/// `max_age`), so a short outage at the issuer does not refuse tokens
/// signed by a key it already published. A key id the set lacks gets 503,
/// not 401: the server cannot tell a key it failed to fetch from a made-up
/// one. Past the grace period every token gets 503. A key id missing from
/// a set fetched successfully is unknown, 401.
struct KeySet {
    url: Url,
    resolver: Arc<dyn HostResolver>,
    client: Arc<dyn PinnedHttpClient>,
    max_bytes: usize,
    timeout: Duration,
    max_age: Duration,
    refetch_after: Duration,
    state: Mutex<KeyState>,
    /// One fetch at a time: requests that miss together wait for one
    /// answer rather than each fetching.
    fetching: tokio::sync::Mutex<()>,
}

/// What the cache knows: the last good set, when it last fetched, and
/// whether that fetch failed.
#[derive(Clone, Default)]
struct KeyState {
    keys: Option<CachedKeys>,
    last_attempt: Option<Instant>,
    last_failed: bool,
}

#[derive(Clone)]
struct CachedKeys {
    keys: Arc<JwkSet>,
    fetched_at: Instant,
}

impl KeySet {
    /// The key `kid`, from the cache or a fetch (see the type's
    /// documentation for when each answers).
    async fn key(&self, kid: &str) -> Result<Jwk, KeyError> {
        let seen = self.state();
        if let Some(answer) = self.answer(&seen, kid) {
            return answer;
        }
        let _fetching = self.fetching.lock().await;
        // A fetch that finished while this request waited answers it, as
        // the one refetch it gets.
        let latest = self.state();
        if latest.last_attempt != seen.last_attempt
            && let Some(answer) = self.answer(&latest, kid)
        {
            return answer;
        }
        let fetched = match tokio::time::timeout(self.timeout, self.fetch()).await {
            Ok(fetched) => fetched,
            Err(_) => Err("the fetch timed out".to_owned()),
        };
        let now = Instant::now();
        let mut state = self.state();
        state.last_attempt = Some(now);
        state.last_failed = fetched.is_err();
        let answer = match fetched {
            Ok(keys) => {
                let found = keys.find(kid).cloned();
                state.keys = Some(CachedKeys {
                    keys: Arc::new(keys),
                    fetched_at: now,
                });
                found.ok_or(KeyError::Unknown)
            }
            Err(reason) => self.after_failure(state.keys.as_ref(), kid, reason),
        };
        if let Ok(mut held) = self.state.lock() {
            *held = state;
        }
        answer
    }

    /// The cache's state.
    fn state(&self) -> KeyState {
        self.state
            .lock()
            .map(|state| state.clone())
            .unwrap_or_default()
    }

    /// The last good set while it is fresh or in its grace period.
    fn within_grace<'a>(&self, keys: Option<&'a CachedKeys>) -> Option<&'a CachedKeys> {
        keys.filter(|keys| keys.fetched_at.elapsed() < self.max_age.saturating_mul(2))
    }

    /// The answer for `kid` after a failed fetch: the last good set's key
    /// through its grace period, and otherwise [`KeyError::Unavailable`],
    /// for a key id it lacks as for a set past its grace.
    fn after_failure(
        &self,
        keys: Option<&CachedKeys>,
        kid: &str,
        reason: String,
    ) -> Result<Jwk, KeyError> {
        self.within_grace(keys)
            .and_then(|keys| keys.keys.find(kid).cloned())
            .ok_or(KeyError::Unavailable(reason))
    }

    /// What `state` says about `kid` without fetching: the key,
    /// [`KeyError::Unknown`], or [`KeyError::Unavailable`]. `None` means
    /// fetch now.
    fn answer(&self, state: &KeyState, kid: &str) -> Option<Result<Jwk, KeyError>> {
        let recent = state
            .last_attempt
            .is_some_and(|attempt| attempt.elapsed() < self.refetch_after);
        let fresh = state
            .keys
            .as_ref()
            .filter(|keys| keys.fetched_at.elapsed() < self.max_age);
        if let Some(jwk) = fresh.and_then(|keys| keys.keys.find(kid)) {
            return Some(Ok(jwk.clone()));
        }
        if !recent {
            // A set past its age, a key id the set lacks, or a failure
            // older than `refetch_after`: fetch.
            return None;
        }
        if state.last_failed {
            // A failed fetch inside `refetch_after`: no new attempt yet.
            return Some(self.after_failure(
                state.keys.as_ref(),
                kid,
                "the key set could not be fetched".to_owned(),
            ));
        }
        // A fresh set fetched successfully lacks the key, and it is too soon
        // to fetch again for it: unknown. A good fetch whose set aged out is
        // fetched again.
        fresh.map(|_| Err(KeyError::Unknown))
    }

    /// One fenced GET of the key set.
    async fn fetch(&self) -> Result<JwkSet, String> {
        let port = self.url.port_or_known_default().unwrap_or(443);
        let addresses: Vec<IpAddr> = match self.url.host() {
            Some(Host::Ipv4(address)) => vec![IpAddr::V4(address)],
            Some(Host::Ipv6(address)) => vec![IpAddr::V6(address)],
            Some(Host::Domain(domain)) => self
                .resolver
                .resolve(domain, port)
                .await
                .map_err(|error| format!("the host does not resolve: {error}"))?,
            None => return Err("the URL has no host".to_owned()),
        };
        let Some(first) = addresses.first().copied() else {
            return Err("the host resolved to no address".to_owned());
        };
        // Every address is checked, not only the one used: a resolver that
        // mixes a public and a private answer is refused outright.
        if addresses.iter().any(|address| !is_public_address(*address)) {
            return Err("the host resolves to a non-public address".to_owned());
        }
        let mut response = self
            .client
            .get(&self.url, SocketAddr::new(first, port))
            .await
            .map_err(|error| error.to_string())?;
        if (300..400).contains(&response.status) {
            return Err(format!(
                "the key set answered with a redirect ({})",
                response.status
            ));
        }
        if response.status != 200 {
            return Err(format!(
                "the key set answered with status {}",
                response.status
            ));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .body
            .next_chunk()
            .await
            .map_err(|error| error.to_string())?
        {
            if body.len().saturating_add(chunk.len()) > self.max_bytes {
                return Err("the key set is too large".to_owned());
            }
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&body)
            .map_err(|error| format!("the key set does not parse: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_006_only_access_token_types_are_accepted() {
        assert!(is_access_token_type(None));
        assert!(is_access_token_type(Some("JWT")));
        assert!(is_access_token_type(Some("application/at+jwt")));
        assert!(!is_access_token_type(Some("oauth-id-jag+jwt")));
        assert!(!is_access_token_type(Some("secevent+jwt")));
    }

    #[test]
    fn agent_006_hmac_is_never_accepted() {
        assert!(!is_asymmetric(Algorithm::HS256));
        assert!(is_asymmetric(Algorithm::ES256));
    }

    #[test]
    fn agent_006_scopes_read_scope_or_scp() {
        let claims = |value: Value| match value {
            Value::Object(map) => map,
            _ => Map::new(),
        };
        assert_eq!(
            scopes(&claims(serde_json::json!({"scope": "a b"}))),
            vec!["a", "b"]
        );
        assert_eq!(
            scopes(&claims(serde_json::json!({"scp": ["a", "b"]}))),
            vec!["a", "b"]
        );
        assert_eq!(
            scopes(&claims(serde_json::json!({"scope": null, "scp": "a"}))),
            vec!["a"]
        );
        assert!(scopes(&claims(serde_json::json!({}))).is_empty());
    }

    #[test]
    fn agent_006_key_set_url_must_be_https() {
        assert!(OutsideIssuer::new("https://login.test", "http://login.test/jwks").is_err());
        assert!(OutsideIssuer::new("https://login.test", "https://u:p@login.test/jwks").is_err());
        assert!(OutsideIssuer::new("", "https://login.test/jwks").is_err());
        assert!(OutsideIssuer::new("https://login.test", "https://login.test/jwks").is_ok());
    }
}
