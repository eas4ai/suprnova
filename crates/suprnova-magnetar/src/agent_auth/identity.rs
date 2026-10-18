//! The identity endpoint (`POST /agent/identity`) and the claim endpoint
//! (`POST /agent/identity/claim`), with the bodies and error codes of
//! auth.md v0.6.0.

use chrono::{DateTime, Utc};
use serde_json::{Map, Value, json};

use super::config::TrustedIssuer;
use super::jwt::{
    AssertionClaims, ID_JAG_TYPE, ProviderTokenError, audience_matches, numeric_date,
    sign_assertion, verify_provider_token,
};
use super::store::{
    ANONYMOUS, IDENTITY_ASSERTION, NewAttempt, RegistrationRecord, SERVICE_AUTH, STATUS_ACTIVE,
    STATUS_CLAIMED, STATUS_EXPIRED, STATUS_PENDING, STATUS_REVOKED, STATUS_UNCLAIMED,
};
use super::{
    AgentUser, Core, ID_JAG_ASSERTION_TYPE, NO_STORE, ProvidedIdentity, ProvisioningDecision,
    agent_error, digest, failure, later, new_user_code, normalize_email, random_hex, rate_limited,
    respond, timestamp,
};
use crate::authorization_server::AuthorizationServer;
use crate::plugin::{LifecycleEventKind, WireResponse};
use crate::{Error, Result};

/// The plaintext one-time values of a new claim ceremony, returned once.
pub(crate) struct Ceremony {
    pub(crate) claim_token: Option<String>,
    pub(crate) attempt_id: String,
    pub(crate) attempt_token: String,
    pub(crate) user_code: String,
    pub(crate) attempt: NewAttempt,
}

impl Core {
    /// A fresh attempt: a claim attempt token and a user code, from the
    /// operating system's random source, stored only as digests.
    pub(crate) fn new_attempt(&self, now: DateTime<Utc>) -> Result<Ceremony> {
        let attempt_token = format!("cat_{}", random_hex(32));
        let user_code = new_user_code();
        let attempt_id = format!("cla_{}", random_hex(16));
        Ok(Ceremony {
            claim_token: None,
            attempt: NewAttempt {
                attempt_id: attempt_id.clone(),
                attempt_token_hash: digest(&attempt_token),
                user_code_hash: digest(&user_code),
                user_code_expires_at: later(now, self.config.user_code_lifetime)?,
            },
            attempt_id,
            attempt_token,
            user_code,
        })
    }

    /// The ceremony block auth.md returns as `claim` or `claim_attempt`.
    pub(crate) fn ceremony_block(&self, ceremony: &Ceremony) -> Value {
        json!({
            "user_code": ceremony.user_code,
            "expires_in": self.config.user_code_lifetime.as_secs(),
            "verification_uri": self.verification_uri(&ceremony.attempt_token),
            "interval": self.config.poll_interval.as_secs(),
        })
    }

    /// Sign an identity assertion for a registration, at its current
    /// version, carrying the user's email once it is bound to one.
    pub(crate) fn assertion(
        &self,
        record: &RegistrationRecord,
        user: Option<&AgentUser>,
        now: DateTime<Utc>,
    ) -> Result<(String, DateTime<Utc>)> {
        let mut expires = later(now, self.config.assertion_lifetime)?;
        // An unclaimed anonymous registration ends with its window, and so
        // does its assertion.
        if record.status == STATUS_UNCLAIMED
            && let Some(window) = record.claim_expires_at
        {
            expires = expires.min(window);
        }
        let claims = AssertionClaims {
            iss: self.issuer.clone(),
            sub: record.id.clone(),
            aud: self.issuer.clone(),
            iat: now.timestamp(),
            exp: expires.timestamp(),
            jti: random_hex(16),
            registration_type: record.kind.clone(),
            ver: record.assertion_version,
            auth_epoch: user.and_then(|user| user.auth_epoch),
            email: user.map(|user| user.email.clone()),
            email_verified: user.map(|_| true),
        };
        let token = sign_assertion(&self.config.signing_key, &claims)?;
        Ok((token, expires))
    }

    pub(crate) async fn identity(
        &self,
        server: &AuthorizationServer,
        body: &Value,
        client_address: Option<&str>,
    ) -> WireResponse {
        let Some(fields) = body.as_object() else {
            return agent_error(400, "invalid_request", "the body must be a JSON object");
        };
        let result = match fields.get("type").and_then(Value::as_str) {
            Some("anonymous") => self.register_anonymous(client_address).await,
            Some("service_auth") => self.register_service_auth(fields, client_address).await,
            Some("identity_assertion") => {
                self.register_identity_assertion(server, fields, client_address)
                    .await
            }
            Some(_) => Ok(agent_error(
                400,
                "invalid_request",
                "type must be anonymous, service_auth or identity_assertion",
            )),
            None => Ok(agent_error(400, "invalid_request", "type is required")),
        };
        result.unwrap_or_else(|error| failure(&error))
    }

    async fn register_anonymous(&self, client_address: Option<&str>) -> Result<WireResponse> {
        if !self.config.identity_types.anonymous {
            return Ok(agent_error(
                400,
                "anonymous_not_enabled",
                "this service does not accept anonymous registration",
            ));
        }
        if let Some(retry_after) = self
            .admit(self.config.limits.anonymous, ANONYMOUS, client_address)
            .await?
        {
            return Ok(rate_limited(retry_after));
        }
        let now = Utc::now();
        let claim_token = format!("clm_{}", random_hex(32));
        let window = later(now, self.config.registration_window)?;
        let record = RegistrationRecord {
            id: format!("reg_{}", random_hex(16)),
            kind: ANONYMOUS.to_owned(),
            status: STATUS_UNCLAIMED.to_owned(),
            user_id: None,
            claim_email: None,
            claim_user_id: None,
            claim_token_hash: Some(digest(&claim_token)),
            claim_expires_at: Some(window),
            attempt_id: None,
            attempt_token_hash: None,
            user_code_hash: None,
            user_code_expires_at: None,
            user_code_failures: 0,
            last_polled_at: None,
            delegation_key: None,
            issuer: None,
            subject: None,
            scopes: self.config.post_claim_scopes.clone(),
            assertion_version: 1,
            auth_epoch: None,
            client_address: client_address.map(str::to_owned),
            grant_id: None,
            cleanup_pending: false,
            created_at: now,
            claimed_at: None,
            revoked_at: None,
        };
        self.store.create_registration(record.clone()).await?;
        self.emit_created(&record, client_address).await;
        let (assertion, expires) = self.assertion(&record, None, now)?;
        self.emit(
            LifecycleEventKind::AgentAssertionIssued,
            None,
            &[("registration_id", record.id.clone())],
            client_address,
        )
        .await;
        Ok(respond(
            200,
            Some(json!({
                "registration_id": record.id,
                "registration_type": ANONYMOUS,
                "identity_assertion": assertion,
                "assertion_expires": timestamp(expires),
                "pre_claim_scopes": self.config.pre_claim_scopes,
                "claim_url": self.endpoint("identity/claim"),
                "claim_token": claim_token,
                "claim_token_expires": timestamp(window),
                "post_claim_scopes": self.config.post_claim_scopes,
            })),
            &NO_STORE,
        ))
    }

    async fn register_service_auth(
        &self,
        fields: &Map<String, Value>,
        client_address: Option<&str>,
    ) -> Result<WireResponse> {
        if !self.config.identity_types.service_auth {
            return Ok(agent_error(
                400,
                "service_auth_not_enabled",
                "this service does not accept service_auth registration",
            ));
        }
        let Some(hint) = fields.get("login_hint").and_then(Value::as_str) else {
            return Ok(agent_error(
                400,
                "invalid_request",
                "login_hint is required",
            ));
        };
        let Some(email) = normalize_email(hint) else {
            return Ok(agent_error(
                400,
                "invalid_request",
                "login_hint must be an email address",
            ));
        };
        if let Some(retry_after) = self
            .admit(
                self.config.limits.service_auth,
                SERVICE_AUTH,
                client_address,
            )
            .await?
        {
            return Ok(rate_limited(retry_after));
        }
        let now = Utc::now();
        let claim_token = format!("clm_{}", random_hex(32));
        let window = later(now, self.config.registration_window)?;
        let ceremony = self.new_attempt(now)?;
        let record = RegistrationRecord {
            id: format!("reg_{}", random_hex(16)),
            kind: SERVICE_AUTH.to_owned(),
            status: STATUS_PENDING.to_owned(),
            user_id: None,
            claim_email: Some(email.clone()),
            claim_user_id: None,
            claim_token_hash: Some(digest(&claim_token)),
            claim_expires_at: Some(window),
            attempt_id: Some(ceremony.attempt.attempt_id.clone()),
            attempt_token_hash: Some(ceremony.attempt.attempt_token_hash.clone()),
            user_code_hash: Some(ceremony.attempt.user_code_hash.clone()),
            user_code_expires_at: Some(ceremony.attempt.user_code_expires_at),
            user_code_failures: 0,
            last_polled_at: None,
            delegation_key: None,
            issuer: None,
            subject: None,
            scopes: self.config.post_claim_scopes.clone(),
            assertion_version: 1,
            auth_epoch: None,
            client_address: client_address.map(str::to_owned),
            grant_id: None,
            cleanup_pending: false,
            created_at: now,
            claimed_at: None,
            revoked_at: None,
        };
        self.store.create_registration(record.clone()).await?;
        self.emit_created(&record, client_address).await;
        self.emit_ceremony(&record.id, &email, client_address).await;
        Ok(respond(
            200,
            Some(json!({
                "registration_id": record.id,
                "registration_type": SERVICE_AUTH,
                "claim_url": self.endpoint("identity/claim"),
                "claim_token": claim_token,
                "claim_token_expires": timestamp(window),
                "post_claim_scopes": self.config.post_claim_scopes,
                "claim": self.ceremony_block(&ceremony),
            })),
            &NO_STORE,
        ))
    }

    async fn register_identity_assertion(
        &self,
        server: &AuthorizationServer,
        fields: &Map<String, Value>,
        client_address: Option<&str>,
    ) -> Result<WireResponse> {
        if !self.config.identity_types.identity_assertion {
            return Ok(agent_error(
                400,
                "issuer_not_enabled",
                "this service does not accept identity assertions",
            ));
        }
        if fields.get("assertion_type").and_then(Value::as_str) != Some(ID_JAG_ASSERTION_TYPE) {
            return Ok(agent_error(
                400,
                "invalid_request",
                "assertion_type must be urn:ietf:params:oauth:token-type:id-jag",
            ));
        }
        let Some(assertion) = fields.get("assertion").and_then(Value::as_str) else {
            return Ok(agent_error(400, "invalid_request", "assertion is required"));
        };
        if let Some(retry_after) = self
            .admit(
                self.config.limits.identity_assertion,
                IDENTITY_ASSERTION,
                client_address,
            )
            .await?
        {
            return Ok(rate_limited(retry_after));
        }
        let verified = match verify_provider_token(
            assertion,
            ID_JAG_TYPE,
            &self.config.trusted_issuers,
            &self.key_sets,
        )
        .await
        {
            Ok(verified) => verified,
            Err(ProviderTokenError::Malformed(reason)) => {
                return Ok(agent_error(400, "invalid_request", reason));
            }
            Err(ProviderTokenError::UntrustedIssuer) => {
                return Ok(agent_error(
                    400,
                    "invalid_issuer",
                    "the issuer is not on this service's trust list",
                ));
            }
            Err(ProviderTokenError::KeysUnavailable(reason)) => {
                tracing::warn!(reason, "a trusted provider's key set could not be fetched");
                return Ok(agent_error(
                    400,
                    "invalid_signature",
                    "the provider's key set could not be fetched",
                ));
            }
            Err(ProviderTokenError::UnknownKey) => {
                return Ok(agent_error(
                    400,
                    "invalid_signature",
                    "the provider's key set has no key with this key id",
                ));
            }
            Err(ProviderTokenError::BadSignature) => {
                return Ok(agent_error(
                    400,
                    "invalid_signature",
                    "the signature does not verify against the provider's key",
                ));
            }
        };
        let now = Utc::now();
        let claims = &verified.claims;
        let checked = match self.check_id_jag(&verified.trusted, claims, now) {
            Ok(checked) => checked,
            Err(response) => return Ok(response),
        };
        // The identifier is spent last, after every other check passed, so
        // a refused token does not need a fresh one to be corrected.
        let replay_key = digest(&format!(
            "id-jag\n{}\n{}",
            verified.trusted.issuer, checked.jti
        ));
        let replay_until = DateTime::from_timestamp(checked.exp, 0)
            .and_then(|exp| later(exp, self.config.clock_skew).ok())
            .unwrap_or(now);
        if !self.store.record_replay(&replay_key, replay_until).await? {
            return Ok(agent_error(
                400,
                "replay_detected",
                "this ID-JAG's jti was already used",
            ));
        }
        self.match_delegation(server, &verified.trusted, checked, client_address, now)
            .await
    }

    /// The claim checks of an ID-JAG, in the order auth.md lists them, each
    /// with its error code.
    fn check_id_jag(
        &self,
        trusted: &TrustedIssuer,
        claims: &Map<String, Value>,
        now: DateTime<Utc>,
    ) -> std::result::Result<CheckedIdJag, WireResponse> {
        let skew = i64::try_from(self.config.clock_skew.as_secs()).unwrap_or(i64::MAX);
        let max_age = self.config.id_jag_max_auth_age.as_secs();
        let Some(exp) = numeric_date(claims, "exp") else {
            return Err(agent_error(400, "invalid_request", "exp is required"));
        };
        if exp <= now.timestamp() {
            return Err(agent_error(400, "expired", "the ID-JAG expired"));
        }
        if !audience_matches(claims, &self.audiences) {
            return Err(agent_error(
                400,
                "invalid_audience",
                "aud does not name this service",
            ));
        }
        let Some(iat) = numeric_date(claims, "iat") else {
            return Err(agent_error(400, "invalid_request", "iat is required"));
        };
        if iat > now.timestamp().saturating_add(skew) {
            return Err(agent_error(
                400,
                "invalid_request",
                "iat is further in the future than the allowed clock skew",
            ));
        }
        let subject = match claims.get("sub").and_then(Value::as_str) {
            Some(subject) if !subject.is_empty() && subject.len() <= 1024 => subject.to_owned(),
            _ => return Err(agent_error(400, "invalid_request", "sub is required")),
        };
        let jti = match claims.get("jti").and_then(Value::as_str) {
            Some(jti) if !jti.is_empty() && jti.len() <= 512 => jti.to_owned(),
            _ => return Err(agent_error(400, "invalid_request", "jti is required")),
        };
        match claims.get("client_id").and_then(Value::as_str) {
            Some(client_id) if trusted.accepts_client_id(client_id) => {}
            _ => {
                return Err(agent_error(
                    400,
                    "invalid_client_id",
                    "client_id does not name this provider",
                ));
            }
        }
        let verified = |flag: &str, value: &str| {
            claims.get(flag).and_then(Value::as_bool) == Some(true)
                && claims
                    .get(value)
                    .and_then(Value::as_str)
                    .is_some_and(|value| !value.trim().is_empty())
        };
        let verified_email = verified("email_verified", "email")
            .then(|| claims.get("email").and_then(Value::as_str))
            .flatten()
            .and_then(normalize_email);
        let verified_phone = verified("phone_number_verified", "phone_number")
            .then(|| claims.get("phone_number").and_then(Value::as_str))
            .flatten()
            .map(|phone| phone.trim().to_owned());
        if verified_email.is_none() && verified_phone.is_none() {
            return Err(agent_error(
                400,
                "missing_verified_email",
                "the ID-JAG carries neither a verified email nor a verified phone number",
            ));
        }
        let login_required = |description: String| {
            let header = format!(
                "AgentAuth error=\"login_required\", max_age=\"{max_age}\", error_description=\"{description}\""
            );
            respond(
                401,
                Some(json!({
                    "error": "login_required",
                    "error_description": description,
                    "message": description,
                    "max_age": max_age,
                })),
                &[
                    NO_STORE[0],
                    NO_STORE[1],
                    ("www-authenticate", header.as_str()),
                ],
            )
        };
        let Some(auth_time) = numeric_date(claims, "auth_time") else {
            return Err(login_required(
                "auth_time is missing. Re-authenticate at the provider and request a fresh ID-JAG."
                    .to_owned(),
            ));
        };
        if auth_time > now.timestamp().saturating_add(skew) {
            return Err(login_required(
                "auth_time is in the future. Re-authenticate at the provider and request a fresh ID-JAG."
                    .to_owned(),
            ));
        }
        let age = now.timestamp().saturating_sub(auth_time);
        if u64::try_from(age).unwrap_or(0) > max_age {
            return Err(login_required(format!(
                "auth_time is {age}s old; max allowed is {max_age}s. Re-authenticate at the provider and request a fresh ID-JAG."
            )));
        }
        Ok(CheckedIdJag {
            subject,
            jti,
            exp,
            verified_email,
            verified_phone,
            name: claims
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_owned),
            agent_platform: claims
                .get("agent_platform")
                .and_then(Value::as_str)
                .map(str::to_owned),
            agent_context_id: claims
                .get("agent_context_id")
                .and_then(Value::as_str)
                .map(str::to_owned),
        })
    }

    /// Resolve the ID-JAG's user: an existing delegation first, then a
    /// verified email or phone (which steps up and never binds silently),
    /// then the provisioning policy.
    async fn match_delegation(
        &self,
        server: &AuthorizationServer,
        trusted: &TrustedIssuer,
        checked: CheckedIdJag,
        client_address: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<WireResponse> {
        let delegation_key = digest(&format!("{}\n{}", trusted.issuer, checked.subject));
        let correlation = checked.correlation(&trusted.issuer);
        for _ in 0..2 {
            let outcome = match self.store.find_by_delegation(&delegation_key).await? {
                Some(existing) => {
                    self.answer_existing_delegation(
                        server,
                        existing,
                        &correlation,
                        client_address,
                        now,
                    )
                    .await
                }
                None => {
                    self.answer_new_delegation(
                        trusted,
                        &checked,
                        &delegation_key,
                        client_address,
                        now,
                    )
                    .await
                }
            };
            match outcome {
                // A concurrent presentation of the same delegation created
                // or changed the row first: answer from it.
                Err(Error::Conflict { .. }) => continue,
                other => return other,
            }
        }
        Err(Error::Conflict {
            resource: "agent registration".to_owned(),
            message: "the delegation kept changing while it was matched".to_owned(),
        })
    }

    async fn answer_new_delegation(
        &self,
        trusted: &TrustedIssuer,
        checked: &CheckedIdJag,
        delegation_key: &str,
        client_address: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<WireResponse> {
        let matched = match &checked.verified_email {
            Some(email) => self.users.find_by_verified_email(email).await?,
            None => None,
        };
        let matched = match (matched, &checked.verified_phone) {
            (Some(user), _) => Some(user),
            (None, Some(phone)) => self.users.find_by_verified_phone(phone).await?,
            (None, None) => None,
        };
        if let Some(user) = matched {
            return self
                .start_step_up(trusted, checked, delegation_key, user, client_address, now)
                .await;
        }
        let identity = ProvidedIdentity {
            issuer: trusted.issuer.clone(),
            subject: checked.subject.clone(),
            provider_name: trusted.display_name.clone(),
            verified_email: checked.verified_email.clone(),
            verified_phone: checked.verified_phone.clone(),
            name: checked.name.clone(),
        };
        match self.provisioning.provision(&identity).await? {
            ProvisioningDecision::Provisioned(user) => {
                self.bind_provisioned(trusted, checked, delegation_key, user, client_address, now)
                    .await
            }
            ProvisioningDecision::Refused => Ok(agent_error(
                400,
                "invalid_request",
                "this service does not create an account for this identity; sign up at the \
                 service first or use another registration type",
            )),
        }
    }

    async fn answer_existing_delegation(
        &self,
        server: &AuthorizationServer,
        existing: RegistrationRecord,
        correlation: &[(&'static str, String)],
        client_address: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<WireResponse> {
        if Core::is_bound(&existing) {
            let user_id = existing.user_id.clone().unwrap_or_default();
            let Some(user) = self.users.find_by_id(&user_id).await? else {
                return Ok(agent_error(
                    400,
                    "invalid_request",
                    "the account this delegation was bound to no longer exists",
                ));
            };
            if Core::epoch_moved(&existing, &user) {
                // The user signed out everywhere or changed their password
                // since the link: it ends, and this presentation starts
                // over as a new delegation, which steps up again.
                self.end_registration(
                    &server.token_issuer(),
                    &existing,
                    client_address,
                    &[
                        ("reason", "epoch_changed".to_owned()),
                        ("actor", "system".to_owned()),
                    ],
                )
                .await?;
                return Err(Error::Conflict {
                    resource: "agent registration".to_owned(),
                    message: "the delegation ended with the user's epoch".to_owned(),
                });
            }
            let (assertion, expires) = self.assertion(&existing, Some(&user), now)?;
            let mut attributes = vec![("registration_id", existing.id.clone())];
            attributes.extend(correlation.iter().cloned());
            self.emit(
                LifecycleEventKind::AgentAssertionIssued,
                Some(&user.user_id),
                &attributes,
                client_address,
            )
            .await;
            return Ok(respond(
                200,
                Some(json!({
                    "registration_id": existing.id,
                    "registration_type": IDENTITY_ASSERTION,
                    "identity_assertion": assertion,
                    "assertion_expires": timestamp(expires),
                    "scopes": existing.scopes,
                })),
                &NO_STORE,
            ));
        }
        if existing.status == STATUS_PENDING || existing.status == STATUS_EXPIRED {
            // The same delegation keys one row whether pending or bound, so
            // a repeated presentation during step-up reuses it with a fresh
            // ceremony.
            let claim_token = format!("clm_{}", random_hex(32));
            let window = later(now, self.config.registration_window)?;
            let mut ceremony = self.new_attempt(now)?;
            if !self
                .store
                .restart_step_up(
                    &existing.id,
                    &digest(&claim_token),
                    window,
                    ceremony.attempt.clone(),
                )
                .await?
            {
                return Err(Error::Conflict {
                    resource: "agent registration".to_owned(),
                    message: "the step-up changed while it was restarted".to_owned(),
                });
            }
            ceremony.claim_token = Some(claim_token);
            let email = existing.claim_email.clone().unwrap_or_default();
            self.emit_ceremony(&existing.id, &email, client_address)
                .await;
            return Ok(self.interaction_required(&existing.id, &ceremony, window));
        }
        if existing.status == STATUS_REVOKED {
            // A revocation whose clean-up failed still names the
            // delegation; finish it, and this presentation starts over.
            self.end_registration(&server.token_issuer(), &existing, client_address, &[])
                .await?;
            return Err(Error::Conflict {
                resource: "agent registration".to_owned(),
                message: "the delegation was being revoked".to_owned(),
            });
        }
        Ok(agent_error(
            400,
            "invalid_request",
            "this delegation cannot register now",
        ))
    }

    async fn start_step_up(
        &self,
        trusted: &TrustedIssuer,
        checked: &CheckedIdJag,
        delegation_key: &str,
        user: AgentUser,
        client_address: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<WireResponse> {
        let claim_token = format!("clm_{}", random_hex(32));
        let window = later(now, self.config.registration_window)?;
        let mut ceremony = self.new_attempt(now)?;
        let record = RegistrationRecord {
            id: format!("reg_{}", random_hex(16)),
            kind: IDENTITY_ASSERTION.to_owned(),
            status: STATUS_PENDING.to_owned(),
            user_id: None,
            claim_email: Some(crate::password::normalize_email(&user.email)),
            claim_user_id: Some(user.user_id.clone()),
            claim_token_hash: Some(digest(&claim_token)),
            claim_expires_at: Some(window),
            attempt_id: Some(ceremony.attempt.attempt_id.clone()),
            attempt_token_hash: Some(ceremony.attempt.attempt_token_hash.clone()),
            user_code_hash: Some(ceremony.attempt.user_code_hash.clone()),
            user_code_expires_at: Some(ceremony.attempt.user_code_expires_at),
            user_code_failures: 0,
            last_polled_at: None,
            delegation_key: Some(delegation_key.to_owned()),
            issuer: Some(trusted.issuer.clone()),
            subject: Some(checked.subject.clone()),
            scopes: self.config.post_claim_scopes.clone(),
            assertion_version: 1,
            auth_epoch: None,
            client_address: client_address.map(str::to_owned),
            grant_id: None,
            cleanup_pending: false,
            created_at: now,
            claimed_at: None,
            revoked_at: None,
        };
        self.store.create_registration(record.clone()).await?;
        ceremony.claim_token = Some(claim_token);
        self.emit_created(&record, client_address).await;
        self.emit_ceremony(&record.id, &user.email, client_address)
            .await;
        Ok(self.interaction_required(&record.id, &ceremony, window))
    }

    async fn bind_provisioned(
        &self,
        trusted: &TrustedIssuer,
        checked: &CheckedIdJag,
        delegation_key: &str,
        user: AgentUser,
        client_address: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<WireResponse> {
        let record = RegistrationRecord {
            id: format!("reg_{}", random_hex(16)),
            kind: IDENTITY_ASSERTION.to_owned(),
            status: STATUS_ACTIVE.to_owned(),
            user_id: Some(user.user_id.clone()),
            claim_email: None,
            claim_user_id: None,
            claim_token_hash: None,
            claim_expires_at: None,
            attempt_id: None,
            attempt_token_hash: None,
            user_code_hash: None,
            user_code_expires_at: None,
            user_code_failures: 0,
            last_polled_at: None,
            delegation_key: Some(delegation_key.to_owned()),
            issuer: Some(trusted.issuer.clone()),
            subject: Some(checked.subject.clone()),
            scopes: self.config.post_claim_scopes.clone(),
            assertion_version: 1,
            auth_epoch: user.auth_epoch,
            client_address: client_address.map(str::to_owned),
            grant_id: None,
            cleanup_pending: false,
            created_at: now,
            claimed_at: None,
            revoked_at: None,
        };
        self.store.create_registration(record.clone()).await?;
        self.emit_created(&record, client_address).await;
        let (assertion, expires) = self.assertion(&record, Some(&user), now)?;
        let mut attributes = vec![("registration_id", record.id.clone())];
        attributes.extend(checked.correlation(&trusted.issuer));
        self.emit(
            LifecycleEventKind::AgentAssertionIssued,
            Some(&user.user_id),
            &attributes,
            client_address,
        )
        .await;
        Ok(respond(
            200,
            Some(json!({
                "registration_id": record.id,
                "registration_type": IDENTITY_ASSERTION,
                "identity_assertion": assertion,
                "assertion_expires": timestamp(expires),
                "scopes": record.scopes,
            })),
            &NO_STORE,
        ))
    }

    /// The `401 interaction_required` answer of an ID-JAG step-up.
    fn interaction_required(
        &self,
        registration_id: &str,
        ceremony: &Ceremony,
        window: DateTime<Utc>,
    ) -> WireResponse {
        let description = "This account exists at the service; the user must sign in and confirm \
                           linking the provider.";
        let header = format!(
            "AgentAuth error=\"interaction_required\", error_description=\"{description}\""
        );
        respond(
            401,
            Some(json!({
                "error": "interaction_required",
                "error_description": description,
                "message": description,
                "registration_id": registration_id,
                "registration_type": IDENTITY_ASSERTION,
                "claim_url": self.endpoint("identity/claim"),
                "claim_token": ceremony.claim_token,
                "claim_token_expires": timestamp(window),
                "post_claim_scopes": self.config.post_claim_scopes,
                "claim": self.ceremony_block(ceremony),
            })),
            &[
                NO_STORE[0],
                NO_STORE[1],
                ("www-authenticate", header.as_str()),
            ],
        )
    }

    async fn emit_created(&self, record: &RegistrationRecord, client_address: Option<&str>) {
        let mut attributes = vec![
            ("registration_id", record.id.clone()),
            ("registration_type", record.kind.clone()),
        ];
        if let Some(issuer) = &record.issuer {
            attributes.push(("iss", issuer.clone()));
        }
        if let Some(subject) = &record.subject {
            attributes.push(("sub", subject.clone()));
        }
        self.emit(
            LifecycleEventKind::AgentRegistrationCreated,
            record.user_id.as_deref(),
            &attributes,
            client_address,
        )
        .await;
    }

    /// Record `claim.requested` and `user_code.minted` for a new attempt.
    pub(crate) async fn emit_ceremony(
        &self,
        registration_id: &str,
        email: &str,
        client_address: Option<&str>,
    ) {
        self.emit(
            LifecycleEventKind::AgentClaimRequested,
            None,
            &[
                ("registration_id", registration_id.to_owned()),
                ("email", email.to_owned()),
            ],
            client_address,
        )
        .await;
        self.emit(
            LifecycleEventKind::AgentUserCodeMinted,
            None,
            &[("registration_id", registration_id.to_owned())],
            client_address,
        )
        .await;
    }

    pub(crate) async fn initiate_claim(
        &self,
        body: &Value,
        client_address: Option<&str>,
    ) -> WireResponse {
        self.initiate_claim_checked(body, client_address)
            .await
            .unwrap_or_else(|error| failure(&error))
    }

    async fn initiate_claim_checked(
        &self,
        body: &Value,
        client_address: Option<&str>,
    ) -> Result<WireResponse> {
        let Some(fields) = body.as_object() else {
            return Ok(agent_error(
                400,
                "invalid_request",
                "the body must be a JSON object",
            ));
        };
        let Some(claim_token) = fields.get("claim_token").and_then(Value::as_str) else {
            return Ok(agent_error(
                400,
                "invalid_request",
                "claim_token is required",
            ));
        };
        let Some(email) = fields
            .get("email")
            .and_then(Value::as_str)
            .and_then(normalize_email)
        else {
            return Ok(agent_error(
                400,
                "invalid_request",
                "email is required and must be an email address",
            ));
        };
        let token_hash = digest(claim_token);
        let Some(record) = self.store.find_by_claim_token(&token_hash).await? else {
            return Ok(agent_error(
                400,
                "invalid_claim_token",
                "the claim token is wrong or expired",
            ));
        };
        let now = Utc::now();
        if let Some(response) = Self::unclaimable(&record, now) {
            return Ok(response);
        }
        if record.kind != ANONYMOUS
            && record
                .claim_email
                .as_deref()
                .is_some_and(|bound| !super::same_email(bound, &email))
        {
            return Ok(agent_error(
                400,
                "invalid_request",
                "email must be the email this registration is bound to",
            ));
        }
        let ceremony = self.new_attempt(now)?;
        if !self
            .store
            .start_attempt(
                &record.id,
                &token_hash,
                &email,
                ceremony.attempt.clone(),
                now,
            )
            .await?
        {
            // The registration changed between the read and the write.
            let current = self.store.find_registration(&record.id).await?;
            return Ok(current
                .and_then(|current| Self::unclaimable(&current, Utc::now()))
                .unwrap_or_else(|| {
                    agent_error(
                        400,
                        "claimed_or_in_flight",
                        "the registration changed; read the registration response again",
                    )
                }));
        }
        self.emit_ceremony(&record.id, &email, client_address).await;
        Ok(respond(
            200,
            Some(json!({
                "registration_id": record.id,
                "claim_attempt_id": ceremony.attempt_id,
                "status": "initiated",
                "expires_at": timestamp(ceremony.attempt.user_code_expires_at),
                "claim_attempt": self.ceremony_block(&ceremony),
            })),
            &NO_STORE,
        ))
    }

    /// Why a registration cannot start a claim attempt, as the claim
    /// endpoint answers it, or `None` when it can.
    fn unclaimable(record: &RegistrationRecord, now: DateTime<Utc>) -> Option<WireResponse> {
        match record.status.as_str() {
            STATUS_REVOKED => Some(agent_error(
                400,
                "invalid_claim_token",
                "the claim token is wrong or expired",
            )),
            STATUS_CLAIMED | STATUS_ACTIVE => Some(agent_error(
                400,
                "claimed_or_in_flight",
                "the registration is already claimed",
            )),
            STATUS_EXPIRED => Some(claim_expired()),
            _ if record.claim_expires_at.is_none_or(|window| window <= now) => {
                Some(claim_expired())
            }
            _ => None,
        }
    }
}

fn claim_expired() -> WireResponse {
    agent_error(
        410,
        "claim_expired",
        "the registration expired before it was claimed; register again",
    )
}

/// The claims of an ID-JAG that passed every check.
pub(crate) struct CheckedIdJag {
    subject: String,
    jti: String,
    exp: i64,
    verified_email: Option<String>,
    verified_phone: Option<String>,
    name: Option<String>,
    agent_platform: Option<String>,
    agent_context_id: Option<String>,
}

impl CheckedIdJag {
    /// The fields auth.md asks ID-JAG audit events to carry, so operators
    /// can correlate with the provider's logs.
    fn correlation(&self, issuer: &str) -> Vec<(&'static str, String)> {
        let mut fields = vec![("iss", issuer.to_owned()), ("sub", self.subject.clone())];
        if let Some(platform) = &self.agent_platform {
            fields.push(("agent_platform", platform.clone()));
        }
        if let Some(context) = &self.agent_context_id {
            fields.push(("agent_context_id", context.clone()));
        }
        fields
    }
}
