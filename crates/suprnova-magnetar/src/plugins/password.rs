//! The `PasswordAuthProvider` plugin: registration, authentication, and the
//! password credential surface.
//!
//! Behavior is ported from torii's password service and the deployed
//! Suprnova flows: equal-cost idempotent registration that never updates an
//! existing credential, dual-format fixed-cost verification on every login
//! outcome (including locked accounts), and every success routed through the
//! shared factor gate - this plugin never mints a session itself.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use secrecy::{ExposeSecret, SecretString};
use serde_json::json;

use crate::abuse::AbusePolicy;
use crate::auth::{AuthenticationContext, SignInDecision, SignInMethod, VerifiedPrincipal};
use crate::password::{
    AttemptVerdict, LockoutService, PasswordVerifier, RehashOutcome, normalize_email,
    validate_password,
};
use crate::plugin::{
    Effect, EffectResponse, Method, Plugin, PluginResult, RequestContext, RouteDescriptor,
    WireBody, WireRequest, WireResponse,
};
use crate::schema::AuthSchema;
use crate::sessions::RememberFacade;
use crate::storage::{CredentialActor, MethodStore, NewUser, UserRecord, UserStore};
use crate::{Error, Result};

use super::{Gate, acquire, bad_request, body_string, generic_ok, request_metadata, unavailable};

/// Registration input.
pub struct RegisterInput {
    /// Candidate email address; normalized inside the provider.
    pub email: String,
    /// Candidate plaintext password.
    pub password: SecretString,
}

/// Registration result. The route response stays generic for both variants;
/// the internal `created` distinction only drives the configured
/// verification/session flow for genuinely new users.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegistrationOutcome {
    /// A new user row was created.
    Created {
        /// New user identifier.
        user_id: String,
        /// Normalized email stored on the row.
        email: String,
    },
    /// The email already belonged to a user; nothing changed.
    Existing {
        /// Existing user identifier.
        user_id: String,
    },
}

/// One password sign-in attempt.
pub struct PasswordAttempt {
    /// Presented email address.
    pub email: String,
    /// Presented plaintext password.
    pub password: SecretString,
    /// Host-supplied session metadata.
    pub metadata: crate::sessions::SessionMetadata,
}

/// Post-login report attached to a successful authentication.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RehashReport {
    /// The stored hash already meets the pinned target.
    NotNeeded,
    /// The credential was upgraded to the Argon2id target and persisted.
    Upgraded,
    /// The upgrade failed after a successful login; the credential is
    /// unchanged and authentication still succeeded.
    Failed {
        /// Failure detail.
        message: String,
    },
}

/// The password-domain provider surface (torii's service surface, kept).
#[async_trait]
pub trait PasswordAuthProvider: Send + Sync {
    /// Idempotent registration: an existing email returns the existing user
    /// unchanged and never updates its password.
    async fn register(&self, input: RegisterInput) -> Result<RegistrationOutcome>;
    /// Verify a primary credential with fixed-format hash work.
    async fn authenticate(&self, input: PasswordAttempt) -> Result<VerifiedPrincipal>;
    /// [`PasswordAuthProvider::authenticate`] plus the post-login rehash
    /// report.
    async fn authenticate_with_outcome(
        &self,
        input: PasswordAttempt,
    ) -> Result<(VerifiedPrincipal, RehashReport)>;
    /// Execute the same credential lookup and fixed-format verifier work as
    /// authentication without rehashing, creating a principal, or producing
    /// any session side effect.
    async fn perform_authentication_work(&self, email: &str, password: &SecretString)
    -> Result<()>;
    /// Change a password after verifying the current one.
    async fn change_password(
        &self,
        user_id: &str,
        current_password: SecretString,
        new_password: SecretString,
    ) -> Result<()>;
    /// Set a password without a current-password check (administrative and
    /// OAuth-onboarding path) while the authenticated actor remains live.
    async fn set_password(&self, actor: &CredentialActor, new_password: SecretString)
    -> Result<()>;
    /// Remove the password only when another sign-in method remains and the
    /// authenticated actor remains live. Returns whether removal happened.
    async fn remove_password(&self, actor: &CredentialActor) -> Result<bool>;
    /// Whether the user has a password credential (census input).
    async fn has_password(&self, user_id: &str) -> Result<bool>;
}

/// Concrete provider over the generic stores and the dual-format verifier.
pub struct PasswordAuthService {
    users: Arc<dyn UserStore>,
    methods: Arc<dyn MethodStore>,
    verifier: Arc<PasswordVerifier>,
}

impl PasswordAuthService {
    /// Bind the provider to user storage, the method census, and the
    /// installed verifier.
    pub fn new(
        users: Arc<dyn UserStore>,
        methods: Arc<dyn MethodStore>,
        verifier: Arc<PasswordVerifier>,
    ) -> Self {
        Self {
            users,
            methods,
            verifier,
        }
    }

    async fn verify_credentials(
        &self,
        email: &str,
        password: &SecretString,
    ) -> Result<(Option<UserRecord>, AttemptVerdict)> {
        let email = normalize_email(email);
        let user = self.users.find_by_email(&email).await?;
        let stored_hash = user.as_ref().and_then(|user| user.password_hash.clone());
        // Fixed-format work runs for every branch: unknown email,
        // passwordless account, wrong password, and success all cost one
        // bcrypt-format and one Argon2-format driver call.
        let verdict = self
            .verifier
            .verify_attempt(stored_hash.as_deref(), password)?;
        Ok((user, verdict))
    }
}

/// The canonical indistinguishable credential failure.
pub(crate) fn invalid_credentials() -> Error {
    Error::InvalidInput {
        field: "credentials".to_owned(),
        message: "invalid credentials".to_owned(),
    }
}

/// Whether an error is the indistinguishable credential failure.
pub(crate) fn is_invalid_credentials(error: &Error) -> bool {
    matches!(error, Error::InvalidInput { field, .. } if field == "credentials")
}

#[async_trait]
impl PasswordAuthProvider for PasswordAuthService {
    async fn register(&self, input: RegisterInput) -> Result<RegistrationOutcome> {
        validate_password(input.password.expose_secret())?;
        let email = normalize_email(&input.email);
        if email.is_empty() {
            return Err(Error::InvalidInput {
                field: "email".to_owned(),
                message: "must not be empty".to_owned(),
            });
        }
        let hash = self.verifier.mint_target(&input.password)?;
        if let Some(existing) = self.users.find_by_email(&email).await? {
            // Anti-enumeration and takeover protection: the equal-cost target
            // hash is discarded; the stored password is never touched.
            return Ok(RegistrationOutcome::Existing {
                user_id: existing.user_id,
            });
        }
        let created = self
            .users
            .create_user(NewUser {
                email: email.clone(),
                password_hash: Some(hash),
            })
            .await?;
        Ok(RegistrationOutcome::Created {
            user_id: created.user_id,
            email,
        })
    }

    async fn authenticate(&self, input: PasswordAttempt) -> Result<VerifiedPrincipal> {
        self.authenticate_with_outcome(input)
            .await
            .map(|(principal, _)| principal)
    }

    async fn perform_authentication_work(
        &self,
        email: &str,
        password: &SecretString,
    ) -> Result<()> {
        let email = normalize_email(email);
        let user = self.users.find_by_email(&email).await?;
        let stored_hash = user.as_ref().and_then(|user| user.password_hash.clone());
        self.verifier
            .verify_work_only(stored_hash.as_deref(), password)
    }

    async fn authenticate_with_outcome(
        &self,
        input: PasswordAttempt,
    ) -> Result<(VerifiedPrincipal, RehashReport)> {
        let (user, verdict) = self
            .verify_credentials(&input.email, &input.password)
            .await?;
        let Some(user) = user else {
            return Err(invalid_credentials());
        };
        if !verdict.valid {
            return Err(invalid_credentials());
        }
        let principal = VerifiedPrincipal::new(
            user.user_id.clone(),
            SignInMethod::Password,
            AuthenticationContext::new(input.metadata, user.auth_epoch, Utc::now()),
        )?;
        let actor = CredentialActor::from_verified_primary(&principal);
        let report = match verdict.rehash {
            RehashOutcome::NotNeeded => RehashReport::NotNeeded,
            RehashOutcome::Upgraded(upgraded) => {
                // Upgrade-only rehash: persistence failure is a post-login
                // outcome, never an authentication failure.
                match self.users.set_password_hash(&actor, &upgraded).await {
                    Ok(()) => RehashReport::Upgraded,
                    Err(error) => {
                        tracing::warn!(
                            user_id = %user.user_id,
                            error = %error,
                            "post-login credential upgrade failed"
                        );
                        RehashReport::Failed {
                            message: error.to_string(),
                        }
                    }
                }
            }
            RehashOutcome::Failed { message } => {
                tracing::warn!(
                    user_id = %user.user_id,
                    error = %message,
                    "post-login credential upgrade failed"
                );
                RehashReport::Failed { message }
            }
        };
        Ok((principal, report))
    }

    async fn change_password(
        &self,
        user_id: &str,
        current_password: SecretString,
        new_password: SecretString,
    ) -> Result<()> {
        validate_password(new_password.expose_secret())?;
        let Some(user) = self.users.find_by_id(user_id).await? else {
            let _ = self.verifier.verify_attempt(None, &current_password)?;
            return Err(invalid_credentials());
        };
        let verdict = self
            .verifier
            .verify_attempt(user.password_hash.as_deref(), &current_password)?;
        if !verdict.valid {
            return Err(invalid_credentials());
        }
        let actor = CredentialActor::verified_primary(&user.user_id, user.auth_epoch);
        let hash = self.verifier.mint_target(&new_password)?;
        self.users.set_password_hash(&actor, &hash).await
    }

    async fn set_password(
        &self,
        actor: &CredentialActor,
        new_password: SecretString,
    ) -> Result<()> {
        validate_password(new_password.expose_secret())?;
        let hash = self.verifier.mint_target(&new_password)?;
        self.users.set_password_hash(actor, &hash).await
    }

    async fn remove_password(&self, actor: &CredentialActor) -> Result<bool> {
        // Password census and removal run inside the actor fence transaction;
        // passkey and linked-account removal retain their existing path.
        self.methods.remove_password_if_not_last(actor).await
    }

    async fn has_password(&self, user_id: &str) -> Result<bool> {
        Ok(self
            .users
            .find_by_id(user_id)
            .await?
            .and_then(|user| user.password_hash)
            .is_some())
    }
}

/// Verification hand-off used by registration when the email-verification
/// plugin is composed. Implemented by that plugin's service; kept as a trait
/// so the password feature never depends on the verification feature.
#[async_trait]
pub trait RegistrationVerification: Send + Sync {
    /// Send the initial verification link for a newly created user.
    async fn send_for_new_user(&self, user_id: &str, email: &str) -> Result<()>;
}

/// Route-level configuration for the password plugin.
#[derive(Clone, Copy, Debug)]
pub struct PasswordPluginConfig {
    /// Send the initial verification mail for newly created users when a
    /// [`RegistrationVerification`] boundary is installed.
    pub send_verification_on_register: bool,
    /// Pass newly created users through the factor gate and establish a
    /// session directly from `register`.
    pub establish_session_on_register: bool,
    /// Abuse budget for `register`.
    pub register_policy: AbusePolicy,
    /// Abuse budget for `login`.
    pub login_policy: AbusePolicy,
}

impl Default for PasswordPluginConfig {
    fn default() -> Self {
        Self {
            send_verification_on_register: true,
            establish_session_on_register: false,
            register_policy: AbusePolicy {
                max_requests: 10,
                window: std::time::Duration::from_secs(3600),
            },
            login_policy: AbusePolicy {
                max_requests: 10,
                window: std::time::Duration::from_secs(60),
            },
        }
    }
}

/// The password route plugin: `register`, `login`, `logout`.
pub struct PasswordPlugin {
    provider: Arc<dyn PasswordAuthProvider>,
    lockout: Arc<LockoutService>,
    verification: Option<Arc<dyn RegistrationVerification>>,
    remember: Option<Arc<dyn RememberFacade>>,
    config: PasswordPluginConfig,
}

impl PasswordPlugin {
    /// Compose the plugin from its domain services.
    pub fn new(
        provider: Arc<dyn PasswordAuthProvider>,
        lockout: Arc<LockoutService>,
        verification: Option<Arc<dyn RegistrationVerification>>,
        remember: Option<Arc<dyn RememberFacade>>,
        config: PasswordPluginConfig,
    ) -> Self {
        Self {
            provider,
            lockout,
            verification,
            remember,
            config,
        }
    }

    async fn handle_register<S: AuthSchema>(
        &self,
        context: &RequestContext<'_, S>,
    ) -> PluginResult<WireResponse> {
        let Some(email) = body_string(context.request, "email") else {
            return Ok(bad_request("email is required"));
        };
        let Some(password) = body_string(context.request, "password") else {
            return Ok(bad_request("password is required"));
        };
        let identity = normalize_email(&email);
        match acquire(
            context,
            "password.register",
            &identity,
            self.config.register_policy,
        )
        .await
        {
            Gate::Proceed => {}
            Gate::Respond(response) => return Ok(response),
        }
        if let Err(error) = validate_password(&password) {
            return Ok(bad_request(&error.to_string()));
        }
        let outcome = self
            .provider
            .register(RegisterInput {
                email,
                password: SecretString::from(password.clone()),
            })
            .await?;
        let mut response = EffectResponse::json(generic_ok());
        if let RegistrationOutcome::Created { user_id, email } = outcome {
            if self.config.send_verification_on_register
                && let Some(verification) = &self.verification
            {
                verification.send_for_new_user(&user_id, &email).await?;
            }
            if self.config.establish_session_on_register {
                // The configured session flow authenticates the just-created
                // credential and routes it through the shared factor gate; a
                // brand-new user has no enrollment, so the gate issues
                // directly.
                let (principal, _) = self
                    .provider
                    .authenticate_with_outcome(PasswordAttempt {
                        email: email.clone(),
                        password: SecretString::from(password),
                        metadata: request_metadata(context.request),
                    })
                    .await?;
                let auth_context = principal.context().clone();
                match context
                    .plugin
                    .factor_gate()
                    .complete_sign_in(principal, auth_context)
                    .await
                {
                    Ok(SignInDecision::SessionAllowed(grant)) => {
                        response = response.with_effect(Effect::EstablishSession(grant));
                    }
                    Ok(SignInDecision::FactorRequired { .. }) => {}
                    Err(error) if is_invalid_credentials(&error) => {}
                    Err(error) => return Err(error.into()),
                }
            }
        }
        Ok(WireResponse::from_effects(response))
    }

    async fn handle_login<S: AuthSchema>(
        &self,
        context: &RequestContext<'_, S>,
    ) -> PluginResult<WireResponse> {
        let Some(email) = body_string(context.request, "email") else {
            return Ok(bad_request("email is required"));
        };
        let Some(password) = body_string(context.request, "password") else {
            return Ok(bad_request("password is required"));
        };
        let identity = normalize_email(&email);
        match acquire(
            context,
            "password.login",
            &identity,
            self.config.login_policy,
        )
        .await
        {
            Gate::Proceed => {}
            Gate::Respond(response) => return Ok(response),
        }
        let status = match self.lockout.guarded_status(&identity).await {
            Ok(status) => status,
            Err(_) => return Ok(unavailable()),
        };
        if status.is_locked {
            let _ = self
                .provider
                .perform_authentication_work(&email, &SecretString::from(password))
                .await;
            return Ok(invalid_credentials_response());
        }
        let metadata = request_metadata(context.request);
        let ip = metadata.ip_address.clone();
        let attempt = PasswordAttempt {
            email,
            password: SecretString::from(password),
            metadata,
        };
        let (principal, rehash) = match self.provider.authenticate_with_outcome(attempt).await {
            Ok(success) => success,
            Err(error) if is_invalid_credentials(&error) => {
                let _ = self
                    .lockout
                    .record_failed_attempt(&identity, ip.as_deref())
                    .await;
                return Ok(invalid_credentials_response());
            }
            Err(error) => return Err(error.into()),
        };
        self.lockout.reset_attempts(&identity).await?;
        if let RehashReport::Failed { message } = &rehash {
            tracing::warn!(error = %message, "post-login rehash failure surfaced to host");
        }
        let auth_context = principal.context().clone();
        let user_id = principal.user_id().to_owned();
        let decision = match context
            .plugin
            .factor_gate()
            .complete_sign_in(principal, auth_context)
            .await
        {
            Ok(decision) => decision,
            Err(error) if is_invalid_credentials(&error) => {
                return Ok(invalid_credentials_response());
            }
            Err(error) => return Err(error.into()),
        };
        match decision {
            SignInDecision::SessionAllowed(grant) => {
                let mut response =
                    EffectResponse::json(generic_ok()).with_effect(Effect::EstablishSession(grant));
                if wants_remember(context.request)
                    && let Some(remember) = &self.remember
                {
                    response = response
                        .with_effect(Effect::IssueRemember(remember.issue_now(&user_id).await?));
                }
                Ok(WireResponse::from_effects(response))
            }
            SignInDecision::FactorRequired { challenge_selector } => {
                Ok(WireResponse::from_effects(EffectResponse::json(json!({
                    "two_factor_required": true,
                    "challenge_selector": challenge_selector,
                }))))
            }
        }
    }

    async fn handle_logout<S: AuthSchema>(
        &self,
        context: &RequestContext<'_, S>,
    ) -> PluginResult<WireResponse> {
        let Some(session) = context.session else {
            let mut response = EffectResponse::json(json!({"message": "unauthenticated"}));
            response.status = 401;
            return Ok(WireResponse::from_effects(response));
        };
        // Ordinary logout: revoke the presented session and retire every
        // remember-me row; other sessions and the epoch stay untouched.
        context
            .plugin
            .sessions()
            .revoke_session(session.session_id())
            .await?;
        if let Some(remember) = &self.remember {
            remember.revoke_all(session.user_id()).await?;
        }
        Ok(WireResponse::from_effects(
            EffectResponse::json(generic_ok()).with_effect(Effect::ClearSession),
        ))
    }
}

#[async_trait]
impl<S: AuthSchema> Plugin<S> for PasswordPlugin {
    fn name(&self) -> &str {
        "password"
    }

    fn routes(&self) -> Vec<RouteDescriptor> {
        vec![
            RouteDescriptor::new(Method::Post, "/register", "password.register")
                .with_feature("password"),
            RouteDescriptor::new(Method::Post, "/login", "password.login").with_feature("password"),
            RouteDescriptor::new(Method::Post, "/logout", "password.logout")
                .with_feature("password"),
        ]
    }

    async fn handle(&self, context: RequestContext<'_, S>) -> PluginResult<WireResponse> {
        match context.request.path.trim_matches('/') {
            "register" => self.handle_register(&context).await,
            "login" => self.handle_login(&context).await,
            "logout" => self.handle_logout(&context).await,
            other => Err(crate::plugin::PluginError::RouteNotFound {
                path: other.to_owned(),
            }),
        }
    }
}

fn invalid_credentials_response() -> WireResponse {
    let mut response = EffectResponse::json(json!({"message": "invalid credentials"}));
    response.status = 401;
    WireResponse::from_effects(response)
}

fn wants_remember(request: &WireRequest) -> bool {
    match &request.body {
        WireBody::Json(value) => value
            .get("remember")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        WireBody::Form(fields) => fields
            .get("remember")
            .is_some_and(|value| value == "true" || value == "1" || value == "on"),
        _ => false,
    }
}
