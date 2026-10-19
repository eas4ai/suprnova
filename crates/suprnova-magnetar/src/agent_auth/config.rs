//! The configuration of agent auth: which identity types it accepts, the
//! trust list, the scopes before and after a claim, the key that signs
//! identity assertions, and the lifetimes and budgets of the ceremony.

use std::fmt;
use std::time::Duration;

use base64::Engine;
use chrono::{DateTime, Utc};
use jsonwebtoken::{DecodingKey, EncodingKey};
use p256::elliptic_curve::sec1::ToEncodedPoint;
use p256::pkcs8::{DecodePrivateKey, EncodePrivateKey};
use url::Url;

use crate::abuse::AbusePolicy;
use crate::{Error, Result};

/// The issuer of ID-JAGs the application trusts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustedIssuer {
    /// The provider's issuer identifier, compared exactly with an ID-JAG's
    /// or a Security Event Token's `iss`.
    pub issuer: String,
    /// The `https` URL of the provider's JSON Web Key Set.
    pub jwks_uri: String,
    /// The name the claim page shows for the provider. The application
    /// writes it; a name the provider sends is never shown, because a
    /// malicious provider would pick its own.
    pub display_name: String,
    /// The `client_id` values an ID-JAG from this issuer may carry. Empty
    /// means the issuer's own identifier only.
    pub client_ids: Vec<String>,
}

impl TrustedIssuer {
    /// Trust `issuer`, whose keys are at `jwks_uri`, under the name
    /// `display_name`.
    #[must_use]
    pub fn new(
        issuer: impl Into<String>,
        jwks_uri: impl Into<String>,
        display_name: impl Into<String>,
    ) -> Self {
        Self {
            issuer: issuer.into(),
            jwks_uri: jwks_uri.into(),
            display_name: display_name.into(),
            client_ids: Vec::new(),
        }
    }

    /// Accept ID-JAGs that name `client_id`, such as the provider's client
    /// metadata document URL.
    #[must_use]
    pub fn with_client_id(mut self, client_id: impl Into<String>) -> Self {
        self.client_ids.push(client_id.into());
        self
    }

    /// Whether an ID-JAG's `client_id` names this provider.
    pub(crate) fn accepts_client_id(&self, client_id: &str) -> bool {
        if self.client_ids.is_empty() {
            client_id == self.issuer
        } else {
            self.client_ids.iter().any(|accepted| accepted == client_id)
        }
    }
}

/// A P-256 key that signs identity assertions (ES256), with the key
/// identifier assertions name it by.
///
/// The private key never leaves this value: `Debug` prints the identifier
/// only.
#[derive(Clone)]
pub struct SigningKey {
    kid: String,
    encoding: EncodingKey,
    decoding: DecodingKey,
}

impl fmt::Debug for SigningKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SigningKey")
            .field("kid", &self.kid)
            .finish_non_exhaustive()
    }
}

impl SigningKey {
    /// A key from a PEM private key, PKCS#8 (`PRIVATE KEY`) or SEC1
    /// (`EC PRIVATE KEY`), on the P-256 curve.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] for an empty key identifier or a PEM
    /// that is not a P-256 private key.
    pub fn from_pem(kid: impl Into<String>, pem: &str) -> Result<Self> {
        let kid = kid.into();
        if kid.trim().is_empty() {
            return Err(invalid(
                "signing_key",
                "the key identifier must not be empty",
            ));
        }
        let secret = p256::SecretKey::from_pkcs8_pem(pem)
            .or_else(|_| p256::SecretKey::from_sec1_pem(pem))
            .map_err(|_| invalid("signing_key", "not a P-256 private key in PEM form"))?;
        let der = secret
            .to_pkcs8_der()
            .map_err(|_| invalid("signing_key", "the key cannot be encoded as PKCS#8"))?;
        let point = secret.public_key().to_encoded_point(false);
        let (Some(x), Some(y)) = (point.x(), point.y()) else {
            return Err(invalid("signing_key", "the public key has no coordinates"));
        };
        let encode = |bytes: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        let decoding = DecodingKey::from_ec_components(&encode(x), &encode(y))
            .map_err(|_| invalid("signing_key", "the public key cannot be read back"))?;
        Ok(Self {
            kid,
            encoding: EncodingKey::from_ec_der(der.as_bytes()),
            decoding,
        })
    }

    /// A new P-256 private key from the operating system's random source,
    /// in PKCS#8 PEM form, for an operator to store as the configured
    /// signing key. It is returned as a secret, so it prints only where
    /// the caller exposes it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Internal`] when the key cannot be encoded.
    pub fn generate_pem() -> Result<secrecy::SecretString> {
        let secret = p256::SecretKey::random(&mut rand::rngs::OsRng);
        let pem = secret
            .to_pkcs8_pem(p256::pkcs8::LineEnding::LF)
            .map_err(|_| Error::Internal {
                message: "a new signing key could not be encoded as PKCS#8".to_owned(),
            })?;
        Ok(secrecy::SecretString::from(pem.as_str()))
    }

    /// The key identifier.
    #[must_use]
    pub fn kid(&self) -> &str {
        &self.kid
    }

    pub(crate) fn encoding(&self) -> &EncodingKey {
        &self.encoding
    }

    pub(crate) fn decoding(&self) -> &DecodingKey {
        &self.decoding
    }
}

/// A signing key that was rotated out. It keeps verifying the assertions
/// it signed until it retires: by default when it was rotated out plus the
/// longest assertion lifetime, after which no assertion it signed can still
/// be unexpired.
#[derive(Clone, Debug)]
pub struct PreviousSigningKey {
    /// The key.
    pub key: SigningKey,
    /// When it stopped signing new assertions.
    pub rotated_out_at: DateTime<Utc>,
    /// An explicit retirement, overriding the default one.
    pub verify_until: Option<DateTime<Utc>>,
}

impl PreviousSigningKey {
    /// `key`, rotated out at `rotated_out_at`.
    #[must_use]
    pub fn new(key: SigningKey, rotated_out_at: DateTime<Utc>) -> Self {
        Self {
            key,
            rotated_out_at,
            verify_until: None,
        }
    }

    /// Retire the key at `at` instead of the default.
    #[must_use]
    pub fn verifying_until(mut self, at: DateTime<Utc>) -> Self {
        self.verify_until = Some(at);
        self
    }

    /// When the key stops verifying: the explicit retirement, or the
    /// rotation plus the longest assertion lifetime. `None` when that sum
    /// overflows a timestamp, which retires the key.
    #[must_use]
    pub fn retires_at(&self, assertion_lifetime: Duration) -> Option<DateTime<Utc>> {
        match self.verify_until {
            Some(at) => Some(at),
            None => chrono::Duration::from_std(assertion_lifetime)
                .ok()
                .and_then(|lifetime| self.rotated_out_at.checked_add_signed(lifetime)),
        }
    }
}

/// The two registration budgets of one identity type: per client address,
/// checked first, and for the whole application.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegistrationLimit {
    /// The budget of one client address.
    pub per_address: AbusePolicy,
    /// The budget of the whole application.
    pub per_application: AbusePolicy,
}

impl RegistrationLimit {
    /// `per_address` and `per_application` registrations an hour.
    #[must_use]
    pub fn per_hour(per_address: u32, per_application: u32) -> Self {
        let hour = Duration::from_secs(3600);
        Self {
            per_address: AbusePolicy {
                max_requests: per_address,
                window: hour,
            },
            per_application: AbusePolicy {
                max_requests: per_application,
                window: hour,
            },
        }
    }
}

/// The registration budgets of each identity type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegistrationLimits {
    /// Anonymous registrations: 5 an hour per address, 100 per application.
    pub anonymous: RegistrationLimit,
    /// `service_auth` registrations, which mint a user code without
    /// authentication like anonymous ones: 5 and 100 an hour.
    pub service_auth: RegistrationLimit,
    /// ID-JAG registrations: 60 an hour per address, 1000 per application.
    pub identity_assertion: RegistrationLimit,
    /// Assertion exchanges at the token endpoint, per registration: 60 an
    /// hour, one a minute on average, ample for a one-hour access token.
    pub exchange: AbusePolicy,
    /// Claim attempts per claim token: 10 an hour. Each attempt mints a
    /// user code, so a leaked claim token cannot mint codes without end.
    pub claim_per_token: AbusePolicy,
    /// Claim attempts per client address: 60 an hour.
    pub claim_per_address: AbusePolicy,
    /// Security Event Tokens per client address: 120 a minute. Each one
    /// that names an unseen key can cost a key set fetch.
    pub events_per_address: AbusePolicy,
}

impl Default for RegistrationLimits {
    fn default() -> Self {
        Self {
            anonymous: RegistrationLimit::per_hour(5, 100),
            service_auth: RegistrationLimit::per_hour(5, 100),
            identity_assertion: RegistrationLimit::per_hour(60, 1000),
            exchange: AbusePolicy {
                max_requests: 60,
                window: Duration::from_secs(3600),
            },
            claim_per_token: AbusePolicy {
                max_requests: 10,
                window: Duration::from_secs(3600),
            },
            claim_per_address: AbusePolicy {
                max_requests: 60,
                window: Duration::from_secs(3600),
            },
            events_per_address: AbusePolicy {
                max_requests: 120,
                window: Duration::from_secs(60),
            },
        }
    }
}

/// Which registration types the identity endpoint accepts. A type that is
/// off answers its `*_not_enabled` code and is not advertised.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdentityTypes {
    /// `anonymous`: an agent with no user identity registers itself.
    pub anonymous: bool,
    /// `service_auth`: an agent names the user's email and the user
    /// confirms with a code.
    pub service_auth: bool,
    /// `identity_assertion`: an agent presents an ID-JAG from a trusted
    /// provider. Needs at least one [`TrustedIssuer`].
    pub identity_assertion: bool,
}

impl Default for IdentityTypes {
    fn default() -> Self {
        Self {
            anonymous: true,
            service_auth: true,
            identity_assertion: false,
        }
    }
}

/// Configuration of agent auth.
#[derive(Clone, Debug)]
pub struct AgentAuthConfig {
    /// The path prefix of the agent routes, `/agent` by default, so the
    /// identity endpoint is `/agent/identity` as auth.md names it.
    pub route_prefix: String,
    /// Where the plugin serves the instructions an agent reads,
    /// `/auth.md` by default.
    pub skill_path: String,
    /// The host's claim page. It receives the claim attempt in the
    /// `claim_attempt_token` query parameter after the user signs in.
    pub claim_page_url: String,
    /// Send the user straight to the claim page: the `verification_uri`
    /// is the claim page's absolute URL, the issuer followed by
    /// [`claim_page_url`](Self::claim_page_url), with the attempt token.
    /// Off by default, which sends the user through the sign-in page with
    /// the claim page as its return address.
    ///
    /// A sign-in page that sends an already signed-in user home loses the
    /// claim on the default route. With this on, the host's claim page must
    /// itself send a guest to sign in and back, with its own address and
    /// query as the return address.
    pub direct_claim_page: bool,
    /// The name `/auth.md` gives the service.
    pub service_name: String,
    /// Which identity types are on.
    pub identity_types: IdentityTypes,
    /// The providers whose ID-JAGs and revocation events are accepted.
    pub trusted_issuers: Vec<TrustedIssuer>,
    /// The scopes an anonymous registration holds until a user claims it.
    pub pre_claim_scopes: Vec<String>,
    /// The scopes a registration bound to a user holds.
    pub post_claim_scopes: Vec<String>,
    /// The key that signs new identity assertions.
    pub signing_key: SigningKey,
    /// Keys that signed assertions before a rotation. Each verifies the
    /// assertions it signed until it retires, and signs nothing new.
    pub previous_signing_keys: Vec<PreviousSigningKey>,
    /// How long an identity assertion can be exchanged, 30 days by default.
    /// An unclaimed anonymous registration's assertion ends with its
    /// registration window.
    pub assertion_lifetime: Duration,
    /// How long a registration waits for a claim, 24 hours by default.
    pub registration_window: Duration,
    /// How long a user code works, ten minutes by default.
    pub user_code_lifetime: Duration,
    /// How many wrong user codes one claim attempt survives, 5 by default.
    /// The agent starts a new attempt after that.
    pub max_user_code_failures: u32,
    /// The polling interval the ceremony asks for, five seconds by default;
    /// a faster poll answers `slow_down`.
    pub poll_interval: Duration,
    /// The oldest user authentication an ID-JAG may carry in `auth_time`,
    /// one hour by default.
    pub id_jag_max_auth_age: Duration,
    /// How far ahead of this server's clock an `iat` or `auth_time` may be,
    /// two minutes by default.
    pub clock_skew: Duration,
    /// How far ahead of this server's clock an ID-JAG's or a Security Event
    /// Token's `exp` may lie, one hour by default: auth.md expects a
    /// near-term `exp`, and a replay record lives until it.
    pub max_provider_token_lifetime: Duration,
    /// The oldest Security Event Token the events endpoint accepts, by its
    /// `iat`, 24 hours by default. The replay record of an accepted one
    /// lives as long.
    pub event_max_age: Duration,
    /// The audiences an ID-JAG or a Security Event Token may name. Empty
    /// means the authorization server's issuer and its resources, which
    /// covers both readings auth.md gives.
    pub audiences: Vec<String>,
    /// The resource agent tokens are bound to when the token request names
    /// none. `None` means the authorization server's first resource.
    pub default_resource: Option<String>,
    /// The largest request body a route reads.
    pub max_request_bytes: usize,
    /// The registration budgets.
    pub limits: RegistrationLimits,
}

impl AgentAuthConfig {
    /// A configuration signing with `signing_key`, with every default:
    /// anonymous and `service_auth` on, ID-JAGs off until a trust list is
    /// given, no scopes, a 16 KiB request bound and the auth.md budgets.
    #[must_use]
    pub fn new(signing_key: SigningKey) -> Self {
        Self {
            route_prefix: "/agent".to_owned(),
            skill_path: "/auth.md".to_owned(),
            claim_page_url: "/agent/claim".to_owned(),
            direct_claim_page: false,
            service_name: "this service".to_owned(),
            identity_types: IdentityTypes::default(),
            trusted_issuers: Vec::new(),
            pre_claim_scopes: Vec::new(),
            post_claim_scopes: Vec::new(),
            signing_key,
            previous_signing_keys: Vec::new(),
            assertion_lifetime: Duration::from_secs(30 * 24 * 3600),
            registration_window: Duration::from_secs(24 * 3600),
            user_code_lifetime: Duration::from_secs(600),
            max_user_code_failures: 5,
            poll_interval: Duration::from_secs(5),
            id_jag_max_auth_age: Duration::from_secs(3600),
            clock_skew: Duration::from_secs(120),
            max_provider_token_lifetime: Duration::from_secs(3600),
            event_max_age: Duration::from_secs(24 * 3600),
            audiences: Vec::new(),
            default_resource: None,
            max_request_bytes: 16 * 1024,
            limits: RegistrationLimits::default(),
        }
    }

    /// Check the configuration against the scopes the authorization server
    /// declares.
    pub(crate) fn validate(&self, declared_scopes: &[String]) -> Result<()> {
        let types = self.identity_types;
        if !types.anonymous && !types.service_auth && !types.identity_assertion {
            return Err(invalid("identity_types", "turn at least one type on"));
        }
        if types.identity_assertion && self.trusted_issuers.is_empty() {
            return Err(invalid(
                "trusted_issuers",
                "identity_assertion needs at least one trusted issuer",
            ));
        }
        for entry in &self.trusted_issuers {
            if entry.issuer.trim().is_empty() || entry.display_name.trim().is_empty() {
                return Err(invalid(
                    "trusted_issuers",
                    "every entry needs an issuer and a display name",
                ));
            }
            let https = Url::parse(&entry.jwks_uri)
                .is_ok_and(|url| url.scheme() == "https" && url.host_str().is_some());
            if !https {
                return Err(invalid(
                    "trusted_issuers",
                    format!("the key set of {:?} must be an https URL", entry.issuer),
                ));
            }
        }
        let mut issuers = std::collections::HashSet::new();
        if !self
            .trusted_issuers
            .iter()
            .all(|entry| issuers.insert(entry.issuer.as_str()))
        {
            return Err(invalid("trusted_issuers", "an issuer is listed twice"));
        }
        for (field, scopes) in [
            ("pre_claim_scopes", &self.pre_claim_scopes),
            ("post_claim_scopes", &self.post_claim_scopes),
        ] {
            if let Some(unknown) = scopes.iter().find(|scope| !declared_scopes.contains(scope)) {
                return Err(invalid(
                    field,
                    format!("scope {unknown:?} is not declared by the authorization server"),
                ));
            }
        }
        if let Some(outside) = self
            .pre_claim_scopes
            .iter()
            .find(|scope| !self.post_claim_scopes.contains(scope))
        {
            return Err(invalid(
                "pre_claim_scopes",
                format!("scope {outside:?} is not among the post-claim scopes"),
            ));
        }
        for (field, value) in [
            ("assertion_lifetime", self.assertion_lifetime),
            ("registration_window", self.registration_window),
            ("user_code_lifetime", self.user_code_lifetime),
            ("poll_interval", self.poll_interval),
            ("id_jag_max_auth_age", self.id_jag_max_auth_age),
            ("event_max_age", self.event_max_age),
        ] {
            if value.as_secs() == 0 || chrono::Duration::from_std(value).is_err() {
                return Err(invalid(field, "must be at least one second and finite"));
            }
        }
        if chrono::Duration::from_std(self.clock_skew).is_err() {
            return Err(invalid("clock_skew", "must be finite"));
        }
        if self.max_user_code_failures == 0 {
            return Err(invalid("max_user_code_failures", "must be at least one"));
        }
        if self.max_request_bytes == 0 {
            return Err(invalid("max_request_bytes", "must be positive"));
        }
        for (field, path) in [
            ("route_prefix", &self.route_prefix),
            ("skill_path", &self.skill_path),
        ] {
            if !path.starts_with('/') {
                return Err(invalid(field, "must start with /"));
            }
        }
        if !is_local_path(&self.claim_page_url) {
            return Err(invalid(
                "claim_page_url",
                "must be a path on this application, starting with one /",
            ));
        }
        let mut kids = std::collections::HashSet::new();
        if !std::iter::once(&self.signing_key)
            .chain(
                self.previous_signing_keys
                    .iter()
                    .map(|previous| &previous.key),
            )
            .all(|key| kids.insert(key.kid()))
        {
            return Err(invalid(
                "previous_signing_keys",
                "a key identifier is used twice",
            ));
        }
        for limit in [
            self.limits.anonymous,
            self.limits.service_auth,
            self.limits.identity_assertion,
        ] {
            limit.per_address.validate()?;
            limit.per_application.validate()?;
        }
        Ok(())
    }

    /// The trust list entry for `issuer`.
    pub(crate) fn trusted(&self, issuer: &str) -> Option<&TrustedIssuer> {
        self.trusted_issuers
            .iter()
            .find(|entry| entry.issuer == issuer)
    }
}

/// A path on this application: one leading `/`, and neither `//` nor `/\`,
/// which a browser reads as another host.
pub(crate) fn is_local_path(value: &str) -> bool {
    value.starts_with('/') && !value.starts_with("//") && !value.starts_with("/\\")
}

fn invalid(field: &str, message: impl Into<String>) -> Error {
    Error::InvalidInput {
        field: field.to_owned(),
        message: message.into(),
    }
}
