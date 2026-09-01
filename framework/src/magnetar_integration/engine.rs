//! Magnetar storage and hybrid-dispatch binding for framework authentication.
//!
//! This module binds Magnetar's application-owned SeaORM descriptors to the
//! framework connection. When an engine is installed, password facades and
//! bearer resolution prefer Magnetar; Magnetar remains the explicit fallback for
//! absent engines and bearer tokens Magnetar does not recognize.

#[cfg(feature = "magnetar-oauth")]
use std::collections::HashMap;
use std::{marker::PhantomData, sync::Arc};

#[cfg(feature = "magnetar-oauth")]
use magnetar::{
    abuse::AbuseLimiter,
    oauth::{
        authorization::{
            CeremonyBinding, OAuthAuthorizationConfig, OAuthAuthorizationService, OAuthBeginInput,
            OAuthCallbackInput, OAuthIntent,
        },
        errors::OAuthProtocolError,
        grants::authorization_code,
        identity::{AutoLinkPolicy, IdentityOutcome, IdentityResolver, VerifiedProviderIdentity},
        provider::{OAuthProvider, OAuthProviderRegistry, ProviderResponse},
        request_shape::{AuthorizationRequestParams, render_authorization_request},
    },
    plugin::{HttpRequest, HttpTransport},
    storage::{CredentialActor, LinkedAccountStore},
};
use secrecy::{ExposeSecret, SecretString};

use async_trait::async_trait;
use magnetar::{
    Error, Result,
    auth::{FactorGate, FactorVerifier, OpaqueFactorGate, SignInDecision, VerifiedPrincipal},
    crypto::Encryptor,
    first_email_proof::{FirstEmailProofMutation, FirstEmailProofStore},
    passkey::{
        BegunAuthentication, BegunRegistration, PasskeyAuthService, PasskeyConfig,
        RegistrationIntent,
    },
    password::{PasswordVerifier, normalize_email, validate_password},
    plugin::{LifecycleEvent, LifecycleEventKind},
    plugins::{
        magic_link::{MagicLinkIssued, MagicLinkService, RegistrationPolicy},
        password::{PasswordAttempt, PasswordAuthProvider, RegisterInput, RegistrationOutcome},
        password_management::{PASSWORD_RESET_TTL, PasswordResetFlowOutcome},
    },
    schema::{
        AuthSchema, CeremonyFields, PasskeyFields, SessionEpoch, SessionFields, TokenFields,
        UserBinding, UserOptionalFields,
    },
    sessions::{
        HostSessionApproval, OpaqueConfig, OpaqueSessionProvider, OpaqueSessionStore,
        RememberCredential, RememberService, RememberSignInService, RememberStore, SessionGrant,
        SessionMetadata, SessionQueries, SessionSummary, VerifiedSession, WebSessionBinding,
    },
    storage::{
        CeremonyStore, IssueToken, PASSWORD_RESET_PURPOSE, PresentedToken, SeaOrmStorage,
        TokenStore, UserStore,
    },
};
use sea_orm::DatabaseConnection;
use uuid::Uuid;
use webauthn_rs::prelude::{PublicKeyCredential, RegisterPublicKeyCredential};

use super::{LockoutStatus, Session, SessionToken, User, UserId};

/// The connection and typed Magnetar storage used by an authentication engine.
///
/// A binding alone does not select a dispatcher. Once a Magnetar engine is
/// installed, framework password facades and bearer middleware use this schema
/// and connection first, preserving Magnetar only as the documented fallback.
pub trait MagnetarAuthStore: Send + Sync {
    /// The application-owned descriptor set used by Magnetar storage.
    type Schema: AuthSchema;

    /// Returns the framework application's SeaORM connection.
    fn database(&self) -> &DatabaseConnection;

    /// Returns Magnetar storage bound to the same application connection.
    fn storage(&self) -> &SeaOrmStorage<Self::Schema>;
}

/// Default framework binding of one application `AuthSchema` to SeaORM.
///
/// Construct this during application boot with the same connection owned by
/// Suprnova. It creates no tables and performs no authentication dispatch.
#[derive(Clone)]
pub struct MagnetarBinding<S: AuthSchema> {
    database: DatabaseConnection,
    storage: SeaOrmStorage<S>,
    schema: PhantomData<S>,
}

impl<S: AuthSchema> MagnetarBinding<S> {
    /// Binds Magnetar storage to an application-owned SeaORM connection.
    #[must_use]
    pub fn new(database: DatabaseConnection) -> Self {
        Self {
            storage: SeaOrmStorage::new(database.clone()),
            database,
            schema: PhantomData,
        }
    }
}

impl<S: AuthSchema> MagnetarAuthStore for MagnetarBinding<S> {
    type Schema = S;

    fn database(&self) -> &DatabaseConnection {
        &self.database
    }

    fn storage(&self) -> &SeaOrmStorage<Self::Schema> {
        &self.storage
    }
}

/// Converts a host-owned application user row into Suprnova's public user.
///
/// Magnetar's [`magnetar::storage::UserRecord`] intentionally does not carry
/// arbitrary application columns such as display name and audit timestamps.
/// The host loads its own row by id and maps it directly into [`User`]; the
/// engine never fabricates fields or aliases one [`AuthSchema`] role to another.
#[async_trait]
pub trait HostUserAdapter: Send + Sync + 'static {
    /// The Suprnova user value returned by this host.
    type User: Send + Sync + 'static;

    /// Load the application-owned row for `user_id` and map it without losing
    /// host-owned fields.
    async fn user_for_id(&self, user_id: &str) -> Result<Self::User>;
}

/// Host-supplied password lockout state around one primary-auth attempt.
///
/// Implementations must fail closed when the state cannot be read or mutated.
/// The engine invokes these operations in order: check before verification,
/// record after verification failure, and reset after verification success.
#[async_trait]
pub trait HostPasswordLockout: Send + Sync + 'static {
    /// Return the current lockout state for this normalized identity.
    async fn status(&self, identity: &str) -> Result<LockoutStatus>;

    /// Record one failed primary-auth verification.
    async fn record_failure(
        &self,
        identity: &str,
        ip_address: Option<&str>,
    ) -> Result<LockoutStatus>;

    /// Clear failures after successful authentication.
    async fn reset_after_success(&self, identity: &str) -> Result<()>;

    /// Force-unlock an account and report whether it was locked.
    async fn unlock(&self, identity: &str) -> Result<bool>;
}

/// Session output converted from one successful Magnetar gate decision.
///
/// The opaque session id and digest-only web binding remain separate from the
/// framework session. The bearer moves exactly once into the framework
/// session and never enters the web binding.
#[derive(Debug)]
pub struct MagnetarIssuedSession {
    /// The opaque row identifier for single-session revocation.
    pub session_id: String,
    /// The digest-only carrier for a server-side web session.
    pub web_binding: WebSessionBinding,
    /// The framework session. Its token exists only on this fresh result.
    pub session: Session,
}

/// Successful remembered sign-in converted for the framework middleware.
#[derive(Debug)]
pub struct MagnetarRememberSignIn {
    /// Fresh opaque session issued through Magnetar's atomic session path.
    pub session: Box<MagnetarIssuedSession>,
    /// Rotated, single-use replacement remember credential.
    pub replacement: RememberCredential,
}

impl TryFrom<SessionGrant> for MagnetarIssuedSession {
    type Error = Error;

    fn try_from(grant: SessionGrant) -> Result<Self> {
        let web_binding = grant.web_binding();
        let bearer = grant.into_bearer();
        let session_id = bearer.session_id().to_owned();
        let user_id = UserId::new(bearer.user_id());
        let expires_at = bearer.expires_at();
        let metadata = bearer.metadata().clone();
        let token_secret = bearer.expose_token_once();
        let token = SessionToken::new(token_secret.expose_secret());
        let session = Session::builder()
            .token(token)
            .user_id(user_id)
            .user_agent(metadata.user_agent)
            .ip_address(metadata.ip_address)
            .expires_at(expires_at)
            .build()
            .map_err(|error| Error::Internal {
                message: format!("build framework session: {error}"),
            })?;
        Ok(Self {
            session_id,
            web_binding,
            session,
        })
    }
}

/// Normalized result of a factor-gate sign-in for framework callers.
#[derive(Debug)]
pub enum HostSignInDecision {
    /// A gate-approved framework session.
    SessionAllowed(Box<MagnetarIssuedSession>),
    /// A second-factor ceremony committed; no session exists yet.
    FactorRequired {
        /// Opaque selector supplied to the factor completion route.
        challenge_selector: String,
    },
}

/// Password-reset token issued for framework-owned mail delivery.
#[derive(Debug)]
pub struct HostPasswordResetIssued {
    /// Application user owning the token.
    pub user_id: String,
    /// Normalized destination mailbox.
    pub email: String,
    /// Single-use Magnetar token; its debug form redacts plaintext.
    pub token: magnetar::storage::IssuedToken,
}

/// Real passkey service composed over an initialized [`MagnetarHostEngine`].
///
/// This is deliberately built separately from [`MagnetarHostEngineParts`]:
/// adding a required relying-party configuration to the existing public parts
/// struct would break applications that only use its password and magic-link
/// paths. A host opts into this adapter with its exact WebAuthn configuration,
/// then installs it in the framework passkey facade.
pub struct MagnetarHostPasskeyService<A: HostUserAdapter> {
    service: PasskeyAuthService,
    users: Arc<A>,
}

impl<A: HostUserAdapter> MagnetarHostPasskeyService<A> {
    async fn finish_registration(
        &self,
        selector: &str,
        email: &str,
        response: &RegisterPublicKeyCredential,
    ) -> Result<User>
    where
        A: HostUserAdapter<User = User>,
    {
        let user_id = self
            .service
            .finish_registration(selector, email, response)
            .await?;
        self.users.user_for_id(&user_id).await
    }
}

/// The narrow passkey dispatch contract accepted by the retained framework
/// facade during the migration.
///
/// Implementations perform actual WebAuthn ceremony verification against the
/// host's application-owned schema. The facade has no implicit adapter or
/// fallback once one is installed.
#[async_trait]
pub trait MagnetarPasskeyAuthEngine: Send + Sync {
    /// Begin an owner-bound or signup registration ceremony.
    async fn passkey_begin_registration(
        &self,
        intent: RegistrationIntent,
    ) -> Result<BegunRegistration>;
    /// Consume and verify a registration ceremony, returning its host user.
    async fn passkey_finish_registration(
        &self,
        selector: &str,
        email: &str,
        response: &RegisterPublicKeyCredential,
    ) -> Result<User>;
    /// Begin a credential-bound authentication ceremony.
    async fn passkey_begin_authentication(&self, email: &str) -> Result<BegunAuthentication>;
    /// Consume and verify an assertion, then run the shared factor gate.
    async fn passkey_finish_authentication(
        &self,
        selector: &str,
        email: &str,
        response: &PublicKeyCredential,
        metadata: SessionMetadata,
    ) -> Result<HostSignInDecision>;
    /// Load the host users user for an issued passkey session.
    async fn passkey_user_by_id(&self, user_id: &str) -> Result<User>;
}

#[async_trait]
impl<A> MagnetarPasskeyAuthEngine for MagnetarHostPasskeyService<A>
where
    A: HostUserAdapter<User = User>,
{
    async fn passkey_begin_registration(
        &self,
        intent: RegistrationIntent,
    ) -> Result<BegunRegistration> {
        self.service.begin_registration(intent).await
    }

    async fn passkey_finish_registration(
        &self,
        selector: &str,
        email: &str,
        response: &RegisterPublicKeyCredential,
    ) -> Result<User> {
        self.finish_registration(selector, email, response).await
    }

    async fn passkey_begin_authentication(&self, email: &str) -> Result<BegunAuthentication> {
        self.service.begin_authentication(email).await
    }

    async fn passkey_finish_authentication(
        &self,
        selector: &str,
        email: &str,
        response: &PublicKeyCredential,
        metadata: SessionMetadata,
    ) -> Result<HostSignInDecision> {
        match self
            .service
            .finish_authentication(selector, email, response, metadata)
            .await?
        {
            SignInDecision::SessionAllowed(grant) => Ok(HostSignInDecision::SessionAllowed(
                Box::new(grant.try_into()?),
            )),
            SignInDecision::FactorRequired { challenge_selector } => {
                Ok(HostSignInDecision::FactorRequired { challenge_selector })
            }
        }
    }

    async fn passkey_user_by_id(&self, user_id: &str) -> Result<User> {
        self.users.user_for_id(user_id).await
    }
}

/// A framework event emitted after a committed Magnetar lifecycle mutation.
///
/// The event preserves Magnetar's mutation id so listeners can correlate
/// retries. The forwarder de-duplicates before dispatch, but external listener
/// effects still must tolerate at-least-once delivery after process failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MagnetarLifecycleEvent {
    /// Stable committed-mutation idempotency key.
    pub mutation_id: String,
    /// The committed mutation kind.
    pub kind: LifecycleEventKind,
    /// The application-owned affected user identifier.
    pub user_id: String,
}

impl From<&LifecycleEvent> for MagnetarLifecycleEvent {
    fn from(event: &LifecycleEvent) -> Self {
        Self {
            mutation_id: event.mutation_id.clone(),
            kind: event.kind,
            user_id: event.user_id.clone(),
        }
    }
}

impl crate::events::Event for MagnetarLifecycleEvent {
    fn event_name() -> &'static str {
        "MagnetarLifecycleEvent"
    }
}

/// Result of consulting the host's durable lifecycle-delivery ledger.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifecycleDeliveryClaim {
    /// This worker owns a bounded delivery lease and must dispatch the event.
    Deliver,
    /// The mutation was previously acknowledged after successful dispatch.
    AlreadyDelivered,
    /// Another worker owns an unexpired delivery lease.
    InFlight,
}

/// The host-owned durable idempotency ledger for lifecycle forwarding.
///
/// `claim` must atomically insert an in-flight mutation id or reclaim only an
/// expired lease. `mark_delivered` and `release` must affect only the caller's
/// `lease_id`. This blocks concurrent duplicate delivery while allowing a
/// retry after callback failure or process death.
#[async_trait]
pub trait HostLifecycleDeduplication: Send + Sync + 'static {
    /// Atomically claim a bounded delivery lease for one committed mutation.
    async fn claim(
        &self,
        mutation_id: &str,
        lease_id: &str,
        now: chrono::DateTime<chrono::Utc>,
        lease_until: chrono::DateTime<chrono::Utc>,
    ) -> Result<LifecycleDeliveryClaim>;

    /// Persist successful delivery for the caller-owned lease.
    async fn mark_delivered(&self, mutation_id: &str, lease_id: &str) -> Result<()>;

    /// Release a failed delivery so a retry can claim it immediately.
    async fn release(&self, mutation_id: &str, lease_id: &str) -> Result<()>;
}

/// Post-commit bridge from Magnetar lifecycle events to Suprnova's dispatcher.
///
/// The bridge is at-least-once, not crash-safe exactly-once. If event dispatch
/// completes and the process dies before the acknowledgement, an expired lease
/// dispatches the event again. If dispatch returns an error, the lease is
/// released for immediate retry. Callers submit only events for mutations that
/// have already committed.
pub struct MagnetarLifecycleForwarder<L: HostLifecycleDeduplication> {
    deliveries: Arc<L>,
    lease_duration: chrono::Duration,
}

impl<L: HostLifecycleDeduplication> MagnetarLifecycleForwarder<L> {
    /// Construct a lifecycle bridge with a positive host-owned lease interval.
    pub fn new(deliveries: Arc<L>, lease_duration: chrono::Duration) -> Result<Self> {
        if lease_duration <= chrono::Duration::zero() {
            return Err(Error::InvalidInput {
                field: "lifecycle_lease_duration".to_owned(),
                message: "must be positive".to_owned(),
            });
        }
        Ok(Self {
            deliveries,
            lease_duration,
        })
    }

    /// Deduplicate and dispatch one event after its source mutation commits.
    pub async fn forward(&self, event: LifecycleEvent) -> Result<LifecycleForwardResult> {
        if event.mutation_id.is_empty() {
            return Err(Error::InvalidInput {
                field: "mutation_id".to_owned(),
                message: "must not be empty".to_owned(),
            });
        }

        let now = chrono::Utc::now();
        let lease_id = Uuid::new_v4().to_string();
        match self
            .deliveries
            .claim(
                &event.mutation_id,
                &lease_id,
                now,
                now + self.lease_duration,
            )
            .await?
        {
            LifecycleDeliveryClaim::AlreadyDelivered => {
                return Ok(LifecycleForwardResult::AlreadyDelivered);
            }
            LifecycleDeliveryClaim::InFlight => return Ok(LifecycleForwardResult::InFlight),
            LifecycleDeliveryClaim::Deliver => {}
        }

        if let Err(dispatch_error) =
            crate::events::EventFacade::dispatch(MagnetarLifecycleEvent::from(&event)).await
        {
            return match self.deliveries.release(&event.mutation_id, &lease_id).await {
                Ok(()) => Err(Error::Internal {
                    message: format!("framework lifecycle dispatch failed: {dispatch_error}"),
                }),
                Err(release_error) => Err(Error::Internal {
                    message: format!(
                        "framework lifecycle dispatch failed: {dispatch_error}; \
                         lifecycle lease release failed: {release_error}"
                    ),
                }),
            };
        }

        self.deliveries
            .mark_delivered(&event.mutation_id, &lease_id)
            .await?;
        Ok(LifecycleForwardResult::Delivered)
    }
}

/// Observable outcome of a lifecycle-forwarding attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifecycleForwardResult {
    /// The framework event was dispatched and durably acknowledged.
    Delivered,
    /// A prior worker already durably acknowledged the mutation.
    AlreadyDelivered,
    /// Another worker currently owns the durable delivery lease.
    InFlight,
}

/// Host-supplied pieces required to compose a [`MagnetarHostEngine`].
pub struct MagnetarHostEngineParts<
    S: AuthSchema,
    O: OpaqueSessionStore,
    C: CeremonyStore,
    F: FactorVerifier,
    P: PasswordAuthProvider,
    A: HostUserAdapter,
    L: HostLifecycleDeduplication,
> {
    /// All-role application binding.
    pub binding: MagnetarBinding<S>,
    /// Application-row session store.
    pub session_store: Arc<O>,
    /// Selector-plus-verifier remember-me row store.
    pub remember_store: Arc<dyn RememberStore>,
    /// Durable ceremony storage.
    pub ceremonies: Arc<C>,
    /// Application factor verifier.
    pub factors: Arc<F>,
    /// Primary password provider backed by application storage.
    pub password: Arc<P>,
    /// Host-owned atomic first-email-proof transaction.
    pub first_email_proof: Arc<dyn FirstEmailProofStore>,
    /// Password hashing and multi-format verification policy.
    pub password_verifier: Arc<PasswordVerifier>,
    /// Application-owned password lockout policy and attempt state.
    pub password_lockout: Arc<dyn HostPasswordLockout>,
    /// Encryption boundary for ceremony payloads.
    pub encryptor: Arc<dyn Encryptor>,
    /// Opaque session issuance policy.
    pub session_config: OpaqueConfig,
    /// Host application-row users converter.
    pub users: Arc<A>,
    /// Durable at-least-once lifecycle ledger.
    pub lifecycle_deliveries: Arc<L>,
    /// Positive lease interval for one lifecycle forward attempt.
    pub lifecycle_lease_duration: chrono::Duration,
}
/// Concrete application composition for Magnetar password, factor, and session
/// execution.
///
/// The host supplies an all-role [`AuthSchema`], a concrete
/// [`OpaqueSessionStore`] over its own session rows, a ceremony store, factor
/// verifier, real password provider, users mapper, and durable
/// lifecycle ledger. This type creates Magnetar's public
/// [`OpaqueSessionProvider`] and [`OpaqueFactorGate`] over those capabilities.
/// It does not route any existing Magnetar facade; the future cutover must opt
/// individual dispatch paths into this engine.
pub struct MagnetarHostEngine<
    S: AuthSchema,
    O: OpaqueSessionStore,
    C: CeremonyStore,
    F: FactorVerifier,
    P: PasswordAuthProvider,
    A: HostUserAdapter,
    L: HostLifecycleDeduplication,
> {
    binding: MagnetarBinding<S>,
    session_store: Arc<O>,
    #[cfg(feature = "magnetar-oauth")]
    ceremonies: Arc<C>,
    session_provider: Arc<OpaqueSessionProvider<O>>,
    factor_gate: Arc<OpaqueFactorGate<C, F, O>>,
    remember: RememberSignInService<SeaOrmStorage<S>>,
    encryptor: Arc<dyn Encryptor>,
    magic_links: MagicLinkService,
    password: Arc<P>,
    first_email_proof: Arc<dyn FirstEmailProofStore>,
    password_verifier: Arc<PasswordVerifier>,
    password_lockout: Arc<dyn HostPasswordLockout>,
    users: Arc<A>,
    lifecycle: MagnetarLifecycleForwarder<L>,
}

impl<S, O, C, F, P, A, L> MagnetarHostEngine<S, O, C, F, P, A, L>
where
    S: AuthSchema,
    O: OpaqueSessionStore + 'static,
    C: CeremonyStore + 'static,
    F: FactorVerifier + 'static,
    P: PasswordAuthProvider,
    A: HostUserAdapter,
    L: HostLifecycleDeduplication,
    S::User: UserBinding + UserOptionalFields + SessionEpoch,
    S::Session: SessionFields,
    S::Token: TokenFields,
{
    /// Compose host-owned password, session, ceremony, factor, users,
    /// and lifecycle boundaries without changing Magnetar dispatch.
    pub fn new(
        MagnetarHostEngineParts {
            binding,
            session_store,
            ceremonies,
            remember_store,
            factors,
            password,
            first_email_proof,
            password_verifier,
            password_lockout,
            encryptor,
            session_config,
            users,
            lifecycle_deliveries,
            lifecycle_lease_duration,
        }: MagnetarHostEngineParts<S, O, C, F, P, A, L>,
    ) -> Result<Self> {
        let session_provider = Arc::new(OpaqueSessionProvider::new(
            Arc::clone(&session_store),
            session_config,
        ));
        let factor_gate = Arc::new(OpaqueFactorGate::new(
            Arc::clone(&ceremonies),
            factors,
            Arc::clone(&encryptor),
            Arc::clone(&session_provider),
        ));
        let remember = RememberSignInService::new(
            Arc::new(RememberService::new(
                remember_store,
                chrono::Duration::days(30),
            )?),
            Arc::new(SeaOrmStorage::<S>::new(binding.database().clone())),
            factor_gate.clone(),
        );
        let magic_storage = Arc::new(SeaOrmStorage::<S>::new(binding.database().clone()));
        let magic_links = MagicLinkService::new(
            magic_storage.clone(),
            magic_storage,
            first_email_proof.clone(),
            factor_gate.clone(),
            RegistrationPolicy::Open,
        );
        let lifecycle =
            MagnetarLifecycleForwarder::new(lifecycle_deliveries, lifecycle_lease_duration)?;
        Ok(Self {
            binding,
            session_store,
            #[cfg(feature = "magnetar-oauth")]
            ceremonies,
            session_provider,
            factor_gate,
            remember,
            encryptor,
            magic_links,
            password,
            first_email_proof,
            password_verifier,
            password_lockout,
            users,
            lifecycle,
        })
    }

    /// Return the concrete all-role application binding owned by this engine.
    #[must_use]
    pub fn binding(&self) -> &MagnetarBinding<S> {
        &self.binding
    }

    /// Return the host's real application-row session store.
    #[must_use]
    pub fn session_store(&self) -> &O {
        &self.session_store
    }

    /// Return Magnetar's query/revocation provider over the host row store.
    #[must_use]
    pub fn session_provider(&self) -> &OpaqueSessionProvider<O> {
        &self.session_provider
    }

    /// Return the shared concrete factor gate.
    #[must_use]
    pub fn factor_gate(&self) -> &OpaqueFactorGate<C, F, O> {
        self.factor_gate.as_ref()
    }

    /// Return the application users mapper.
    #[must_use]
    pub fn users(&self) -> &A {
        &self.users
    }

    /// Compose a real passkey adapter over this engine's application-owned
    /// schema, ceremony store, encryptor, and factor gate.
    ///
    /// The caller supplies the relying-party identity instead of relying on a
    /// default: it must match the application configuration used by browsers
    /// and previously enrolled credentials. Constructing this adapter does not
    /// change dispatch; install it atomically with the password adapter through
    /// [`super::install_magnetar_engines`].
    pub fn passkey_service(&self, config: &PasskeyConfig) -> Result<MagnetarHostPasskeyService<A>>
    where
        S::Passkey: PasskeyFields,
        S::Ceremony: CeremonyFields,
    {
        let storage = Arc::new(SeaOrmStorage::<S>::new(self.binding.database().clone()));
        let factor_gate: Arc<dyn FactorGate> = self.factor_gate.clone();
        let service = PasskeyAuthService::new(
            config,
            storage.clone(),
            storage.clone(),
            storage,
            Arc::clone(&self.encryptor),
            factor_gate,
        )?;
        Ok(MagnetarHostPasskeyService {
            service,
            users: Arc::clone(&self.users),
        })
    }

    /// Issue an epoch-bound remember credential for one current user row.
    pub async fn issue_remember(
        &self,
        user_id: &str,
        lifetime: chrono::Duration,
    ) -> Result<RememberCredential> {
        self.remember
            .issue_with_lifetime(user_id, chrono::Utc::now(), lifetime)
            .await
    }

    /// Consume a remember credential through the shared factor gate.
    pub async fn remember_sign_in(
        &self,
        credential: RememberCredential,
        metadata: SessionMetadata,
        replacement_lifetime: chrono::Duration,
    ) -> Result<MagnetarRememberSignIn> {
        let outcome = self
            .remember
            .sign_in_with_lifetime(
                credential,
                metadata,
                chrono::Utc::now(),
                replacement_lifetime,
            )
            .await?;
        Ok(MagnetarRememberSignIn {
            session: Box::new(outcome.session.try_into()?),
            replacement: outcome.replacement,
        })
    }

    /// Resolve a digest-only binding against the current opaque session row and user epoch.
    pub async fn resolve_web_binding(
        &self,
        binding: &WebSessionBinding,
    ) -> Result<VerifiedSession> {
        self.session_provider
            .resolve_web_binding(binding, &HostSessionApproval::authenticated())
            .await
    }

    /// Revoke every remember credential for one user.
    pub async fn revoke_remember(&self, user_id: &str) -> Result<u64> {
        self.remember.revoke_all_for_user(user_id).await
    }

    /// Revoke exactly one remember credential by owner and non-secret selector.
    ///
    /// Returns `false` without mutation when no owner/selector row matches,
    /// including owner mismatch.
    ///
    /// # Errors
    ///
    /// Propagates the remember service's exact-revocation error, including an
    /// ambiguous-row conflict and its fail-closed unsupported-capability result.
    pub async fn revoke_remember_selector(&self, user_id: &str, selector: &str) -> Result<bool> {
        self.remember.revoke_selector(user_id, selector).await
    }

    /// Register through the host's real Magnetar password provider.
    pub async fn register_password(&self, input: RegisterInput) -> Result<RegistrationOutcome> {
        self.password.register(input).await
    }

    /// Check host lockout state, verify one password, and mutate lockout state
    /// in the required before/failure/success order.
    pub async fn authenticate_password(
        &self,
        mut input: PasswordAttempt,
    ) -> Result<VerifiedPrincipal> {
        let identity = magnetar::password::normalize_email(&input.email);
        input.email.clone_from(&identity);
        if self.password_lockout.status(&identity).await?.is_locked {
            return Err(Error::Conflict {
                resource: "password sign-in".to_owned(),
                message: "authentication unavailable".to_owned(),
            });
        }
        match self.password.authenticate(input).await {
            Ok(principal) => {
                self.password_lockout.reset_after_success(&identity).await?;
                Ok(principal)
            }
            Err(error) => {
                if matches!(
                    &error,
                    Error::InvalidInput { field, .. } if field == "credentials"
                ) {
                    let _ = self
                        .password_lockout
                        .record_failure(&identity, None)
                        .await?;
                }
                Err(error)
            }
        }
    }
    /// Complete a verified primary sign-in through the real factor gate.
    ///
    /// The primary-auth metadata is copied from the unforgeable principal so
    /// user-agent and source-address fields survive to the issued session.
    pub async fn complete_sign_in(
        &self,
        principal: VerifiedPrincipal,
    ) -> Result<HostSignInDecision> {
        let context = principal.context().clone();
        match self
            .factor_gate
            .complete_sign_in(principal, context)
            .await?
        {
            SignInDecision::SessionAllowed(grant) => Ok(HostSignInDecision::SessionAllowed(
                Box::new(grant.try_into()?),
            )),
            SignInDecision::FactorRequired { challenge_selector } => {
                Ok(HostSignInDecision::FactorRequired { challenge_selector })
            }
        }
    }
    /// Run one password-primary attempt through the initialized Magnetar
    /// provider and return the host-mapped user with the factor-gate result.
    ///
    /// A successful primary verification does not imply a session: callers
    /// must handle [`HostSignInDecision::FactorRequired`] explicitly.
    pub async fn password_sign_in(
        &self,
        input: PasswordAttempt,
    ) -> Result<(User, HostSignInDecision)>
    where
        A: HostUserAdapter<User = User>,
    {
        let principal = self.authenticate_password(input).await?;
        let user = self.users.user_for_id(principal.user_id()).await?;
        let decision = self.complete_sign_in(principal).await?;
        Ok((user, decision))
    }
    /// Register through the initialized Magnetar password provider and map the
    /// resulting application row through the host users adapter.
    pub async fn password_register(&self, input: RegisterInput) -> Result<User>
    where
        A: HostUserAdapter<User = User>,
    {
        let user_id = match self.register_password(input).await? {
            RegistrationOutcome::Created { user_id, .. }
            | RegistrationOutcome::Existing { user_id } => user_id,
        };

        self.users.user_for_id(&user_id).await
    }
    /// Mint one plaintext magic-link token through Magnetar's single-use
    /// token store. The returned plaintext is for app-owned delivery only.
    pub async fn magic_link_send(&self, email: &str) -> Result<String> {
        use secrecy::ExposeSecret;

        match self.magic_links.issue(email).await? {
            MagicLinkIssued::Minted(token) => Ok(token.expose_secret().to_owned()),
            MagicLinkIssued::Suppressed => Err(Error::Conflict {
                resource: "magic link".to_owned(),
                message: "the configured registration policy suppressed issuance".to_owned(),
            }),
        }
    }

    /// Atomically consume a magic link and complete the shared factor gate.
    pub async fn magic_link_consume(
        &self,
        token: &str,
        metadata: magnetar::sessions::SessionMetadata,
    ) -> Result<HostSignInDecision> {
        match self.magic_links.consume(token, metadata).await? {
            SignInDecision::SessionAllowed(grant) => Ok(HostSignInDecision::SessionAllowed(
                Box::new(grant.try_into()?),
            )),
            SignInDecision::FactorRequired { challenge_selector } => {
                Ok(HostSignInDecision::FactorRequired { challenge_selector })
            }
        }
    }

    /// Complete one second-factor ceremony and convert the issued session.
    pub async fn complete_challenge(
        &self,
        selector: &str,
        code: &str,
    ) -> Result<MagnetarIssuedSession> {
        self.factor_gate
            .complete_challenge(selector, code)
            .await?
            .try_into()
    }

    /// Forward one post-commit Magnetar lifecycle event through Suprnova.
    pub async fn forward_lifecycle(&self, event: LifecycleEvent) -> Result<LifecycleForwardResult> {
        self.lifecycle.forward(event).await
    }
}
/// Authentication operations exposed to Suprnova's public facades.
///
/// Implementations are initialized Magnetar engines; there is no fallback
/// implementation.
#[async_trait]
pub trait MagnetarPasswordAuthEngine: Send + Sync {
    /// Verify a password and run its shared factor gate.
    async fn password_sign_in(&self, input: PasswordAttempt) -> Result<(User, HostSignInDecision)>;
    /// Issue a password-reset token through Magnetar's unified token store.
    async fn issue_password_reset(&self, email: &str) -> Result<Option<HostPasswordResetIssued>>;
    /// Check one password-reset token without consuming it.
    async fn check_password_reset(&self, token: SecretString) -> Result<bool>;
    /// Consume a reset and run the atomic first-email-proof transition.
    async fn complete_password_reset(
        &self,
        token: SecretString,
        password: SecretString,
    ) -> Result<PasswordResetFlowOutcome>;
    /// Register one password credential through Magnetar and map its user row.
    async fn password_register(&self, input: RegisterInput) -> Result<User>;
    /// Resolve one bearer token through the initialized Magnetar session store.
    async fn bearer_user_id(&self, token: &str) -> Result<Option<String>>;
    /// Issue an epoch-bound remember credential.
    async fn issue_remember(
        &self,
        user_id: &str,
        lifetime: chrono::Duration,
    ) -> Result<RememberCredential>;
    /// Consume and rotate a remember credential through the shared factor gate.
    async fn remember_sign_in(
        &self,
        credential: RememberCredential,
        metadata: SessionMetadata,
        replacement_lifetime: chrono::Duration,
    ) -> Result<MagnetarRememberSignIn>;
    /// Resolve a digest-only web binding against the current session row and user epoch.
    async fn resolve_web_binding(&self, binding: &WebSessionBinding) -> Result<VerifiedSession>;
    /// Revoke every remember credential for one user.
    async fn revoke_remember(&self, user_id: &str) -> Result<u64>;
    /// Revoke exactly one remember credential by owner and non-secret selector.
    ///
    /// Returns `false` without mutation when no owner/selector row matches,
    /// including owner mismatch.
    ///
    /// # Errors
    ///
    /// Returns [`Error::DependencyUnavailable`] by default so an older engine
    /// implementation cannot silently broaden a guard-scoped logout into
    /// revoking every remember credential for the user. Implementations return
    /// [`Error::Conflict`] when multiple persisted rows match.
    async fn revoke_remember_selector(&self, user_id: &str, selector: &str) -> Result<bool> {
        let _ = (user_id, selector);
        Err(Error::DependencyUnavailable {
            dependency: "Magnetar password authentication engine".to_owned(),
            message: "exact remember credential revocation is unavailable".to_owned(),
        })
    }
    /// Load a host-mapped users user by its opaque application id.
    async fn user_by_id(&self, user_id: &str) -> Result<Option<User>>;
    /// Revoke one opaque session by its stable row identifier.
    async fn revoke_session(&self, session_id: &str) -> Result<bool>;
    /// Revoke all active sessions for one application user.
    async fn revoke_all_sessions(&self, user_id: &str) -> Result<u64>;
    /// List active sessions for one application user.
    async fn list_sessions(&self, user_id: &str) -> Result<Vec<SessionSummary>>;
    /// Record one failed attempt through the host lockout store.
    async fn record_failed_attempt(
        &self,
        email: &str,
        ip_address: Option<&str>,
    ) -> Result<LockoutStatus>;
    /// Read the current lockout state.
    async fn lockout_status(&self, email: &str) -> Result<LockoutStatus>;
    /// Clear failed attempts after success.
    async fn reset_attempts(&self, email: &str) -> Result<()>;
    /// Force-unlock an account.
    async fn unlock_account(&self, email: &str) -> Result<bool>;
    /// Mint a single-use magic-link plaintext for app-owned delivery.
    async fn magic_link_send(&self, email: &str) -> Result<String>;
    /// Atomically consume a magic link and complete the shared factor gate.
    async fn magic_link_consume(
        &self,
        token: &str,
        metadata: magnetar::sessions::SessionMetadata,
    ) -> Result<HostSignInDecision>;
}

#[async_trait]
impl<S, O, C, F, P, A, L> MagnetarPasswordAuthEngine for MagnetarHostEngine<S, O, C, F, P, A, L>
where
    S::User: UserBinding + UserOptionalFields + SessionEpoch,
    S::Session: SessionFields,
    S::Token: TokenFields,
    S: AuthSchema,
    O: OpaqueSessionStore + 'static,
    C: CeremonyStore + 'static,
    F: FactorVerifier + 'static,
    P: PasswordAuthProvider,
    A: HostUserAdapter<User = User>,
    L: HostLifecycleDeduplication,
{
    async fn password_sign_in(&self, input: PasswordAttempt) -> Result<(User, HostSignInDecision)> {
        MagnetarHostEngine::password_sign_in(self, input).await
    }

    async fn password_register(&self, input: RegisterInput) -> Result<User> {
        MagnetarHostEngine::password_register(self, input).await
    }

    async fn issue_password_reset(&self, email: &str) -> Result<Option<HostPasswordResetIssued>> {
        let normalized = normalize_email(email);
        let Some(user) = self.binding.storage().find_by_email(&normalized).await? else {
            return Ok(None);
        };
        let issued = self
            .binding
            .storage()
            .issue(IssueToken {
                user_id: user.user_id.clone(),
                purpose: PASSWORD_RESET_PURPOSE.to_owned(),
                ttl: PASSWORD_RESET_TTL,
            })
            .await?;
        Ok(Some(HostPasswordResetIssued {
            user_id: user.user_id,
            email: user.email,
            token: issued,
        }))
    }

    async fn check_password_reset(&self, token: SecretString) -> Result<bool> {
        self.binding
            .storage()
            .check(PresentedToken(token), PASSWORD_RESET_PURPOSE)
            .await
    }

    async fn complete_password_reset(
        &self,
        token: SecretString,
        password: SecretString,
    ) -> Result<PasswordResetFlowOutcome> {
        validate_password(password.expose_secret())?;
        let password_hash = self.password_verifier.mint_target(&password)?;
        let commit = self
            .first_email_proof
            .apply(FirstEmailProofMutation::PasswordReset {
                token: PresentedToken(token),
                expected_user_id: None,
                new_password_hash: SecretString::from(password_hash),
            })
            .await?
            .into_commit()?;
        let lockout_cleared = match self.binding.storage().find_by_id(&commit.user_id).await {
            Ok(Some(user)) => self.password_lockout.unlock(&user.email).await,
            Ok(None) => Err(Error::NotFound {
                resource: "user".to_owned(),
                identifier: commit.user_id.clone(),
            }),
            Err(error) => Err(error),
        };
        Ok(PasswordResetFlowOutcome {
            user_id: commit.user_id,
            auth_epoch: commit.auth_epoch,
            revoked_sessions: commit.revoked_sessions,
            remember_rows_revoked: commit.revoked_remember_rows,
            lockout_cleared,
        })
    }

    async fn bearer_user_id(&self, token: &str) -> Result<Option<String>> {
        match self.session_provider.verify_bearer(token).await {
            Ok(session) => Ok(Some(session.user_id().to_owned())),
            Err(Error::NotFound { .. } | Error::InvalidInput { .. }) => Ok(None),
            Err(error) => Err(error),
        }
    }

    async fn issue_remember(
        &self,
        user_id: &str,
        lifetime: chrono::Duration,
    ) -> Result<RememberCredential> {
        MagnetarHostEngine::issue_remember(self, user_id, lifetime).await
    }

    async fn remember_sign_in(
        &self,
        credential: RememberCredential,
        metadata: SessionMetadata,
        replacement_lifetime: chrono::Duration,
    ) -> Result<MagnetarRememberSignIn> {
        MagnetarHostEngine::remember_sign_in(self, credential, metadata, replacement_lifetime).await
    }

    async fn resolve_web_binding(&self, binding: &WebSessionBinding) -> Result<VerifiedSession> {
        MagnetarHostEngine::resolve_web_binding(self, binding).await
    }

    async fn revoke_remember(&self, user_id: &str) -> Result<u64> {
        MagnetarHostEngine::revoke_remember(self, user_id).await
    }

    async fn revoke_remember_selector(&self, user_id: &str, selector: &str) -> Result<bool> {
        MagnetarHostEngine::revoke_remember_selector(self, user_id, selector).await
    }

    async fn user_by_id(&self, user_id: &str) -> Result<Option<User>> {
        match self.users.user_for_id(user_id).await {
            Ok(user) => Ok(Some(user)),
            Err(Error::NotFound { .. }) => Ok(None),
            Err(error) => Err(error),
        }
    }

    async fn revoke_session(&self, session_id: &str) -> Result<bool> {
        self.session_provider.revoke_session(session_id).await
    }

    async fn revoke_all_sessions(&self, user_id: &str) -> Result<u64> {
        self.session_provider.revoke_all_for_user(user_id).await
    }

    async fn list_sessions(&self, user_id: &str) -> Result<Vec<SessionSummary>> {
        self.session_provider.list_for_user(user_id).await
    }

    async fn record_failed_attempt(
        &self,
        email: &str,
        ip_address: Option<&str>,
    ) -> Result<LockoutStatus> {
        self.password_lockout
            .record_failure(email, ip_address)
            .await
    }

    async fn lockout_status(&self, email: &str) -> Result<LockoutStatus> {
        self.password_lockout.status(email).await
    }

    async fn reset_attempts(&self, email: &str) -> Result<()> {
        self.password_lockout.reset_after_success(email).await
    }

    async fn unlock_account(&self, email: &str) -> Result<bool> {
        self.password_lockout.unlock(email).await
    }

    async fn magic_link_send(&self, email: &str) -> Result<String> {
        MagnetarHostEngine::magic_link_send(self, email).await
    }

    async fn magic_link_consume(
        &self,
        token: &str,
        metadata: magnetar::sessions::SessionMetadata,
    ) -> Result<HostSignInDecision> {
        MagnetarHostEngine::magic_link_consume(self, token, metadata).await
    }
}

/// Explicit OAuth dossier for one provider delegated to Magnetar.
#[cfg(feature = "magnetar-oauth")]
pub struct MagnetarOAuthProviderConfig {
    /// Provider implementation with its own client-authentication dossier.
    pub provider: Arc<dyn OAuthProvider>,
    /// Exact callback URI registered with this provider.
    pub redirect_uri: String,
    /// Scopes requested by this host flow.
    pub scopes: Vec<String>,
}

/// Host configuration for Magnetar OAuth.
///
/// Once installed, this provider registry is authoritative; unknown providers
/// fail closed instead of falling through to another authentication engine.
#[cfg(feature = "magnetar-oauth")]
pub struct MagnetarOAuthHostConfig {
    providers: OAuthProviderRegistry,
    provider_config: HashMap<&'static str, MagnetarOAuthProviderSettings>,
    transport: Arc<dyn HttpTransport>,
    limiter: Arc<dyn AbuseLimiter>,
    authorization: OAuthAuthorizationConfig,
    auto_link: AutoLinkPolicy,
}

#[cfg(feature = "magnetar-oauth")]
struct MagnetarOAuthProviderSettings {
    redirect_uri: String,
    scopes: Vec<String>,
}

#[cfg(feature = "magnetar-oauth")]
type OAuthProviderConfigLookup = (Arc<dyn OAuthProvider>, String, Vec<String>);

#[cfg(feature = "magnetar-oauth")]
impl MagnetarOAuthHostConfig {
    /// Validate and construct the explicit host configuration.
    pub fn new(
        providers: Vec<MagnetarOAuthProviderConfig>,
        transport: Arc<dyn HttpTransport>,
        limiter: Arc<dyn AbuseLimiter>,
        authorization: OAuthAuthorizationConfig,
        auto_link: AutoLinkPolicy,
    ) -> std::result::Result<Self, HostOAuthError> {
        let mut registry = OAuthProviderRegistry::new();
        let mut provider_config = HashMap::new();
        for config in providers {
            if config.redirect_uri.is_empty() {
                return Err(HostOAuthError::Auth(Error::InvalidInput {
                    field: "redirect_uri".to_owned(),
                    message: "must not be empty".to_owned(),
                }));
            }
            let name = config.provider.name();
            if provider_config
                .insert(
                    name,
                    MagnetarOAuthProviderSettings {
                        redirect_uri: config.redirect_uri,
                        scopes: config.scopes,
                    },
                )
                .is_some()
            {
                return Err(HostOAuthError::Auth(Error::Conflict {
                    resource: "OAuth provider".to_owned(),
                    message: format!("provider '{name}' is configured more than once"),
                }));
            }
            registry
                .register(config.provider)
                .map_err(HostOAuthError::Protocol)?;
        }
        Ok(Self {
            providers: registry,
            provider_config,
            transport,
            limiter,
            authorization,
            auto_link,
        })
    }
}

/// Host-selected binding for a new OAuth ceremony.
#[cfg(feature = "magnetar-oauth")]
pub struct MagnetarOAuthBegin {
    /// Normalized provider key in [`MagnetarOAuthHostConfig`].
    pub provider: String,
    /// Identity-resolution intent stored with the ceremony.
    pub intent: OAuthIntent,
    /// Trusted authenticated actor for a link intent. Sign-in must omit it.
    pub actor: Option<CredentialActor>,
    /// Web cookie binding or explicit API state-only binding.
    pub binding: CeremonyBinding,
    /// Host-selected abuse-limiter key.
    pub limiter_identity: String,
}

/// Callback data supplied by the host router.
#[cfg(feature = "magnetar-oauth")]
pub struct MagnetarOAuthCallback {
    /// Provider key expected to own the ceremony.
    pub provider: String,
    /// Opaque ceremony selector returned by [`MagnetarOAuthAuthEngine::oauth_begin`].
    pub state: String,
    /// Single-use authorization code.
    pub code: SecretString,
    /// Present only for a browser cookie flow.
    pub host_session_digest: Option<[u8; 32]>,
    /// Apple `form_post` user payload, if supplied on the first callback.
    pub form_post_user: Option<String>,
    /// Session metadata that a known identity carries into the factor gate.
    pub metadata: SessionMetadata,
}

/// Authorization redirect created by the real Magnetar ceremony service.
#[cfg(feature = "magnetar-oauth")]
pub struct MagnetarOAuthKickoff {
    /// Provider authorization endpoint plus rendered parameters.
    pub authorization_url: String,
    /// Opaque state selector.
    pub state: String,
}

/// Result after provider proof verification and identity resolution.
#[cfg(feature = "magnetar-oauth")]
pub enum MagnetarOAuthCompletion {
    /// Existing linked identity passed the factor gate and received an opaque session.
    SessionAllowed {
        /// Host users user.
        user: User,
        /// Opaque session converted by the framework facade only on success.
        session: Box<MagnetarIssuedSession>,
    },
    /// Existing linked identity requires a factor challenge.
    FactorRequired {
        /// Opaque factor challenge selector.
        challenge_selector: String,
    },
    /// First-time identity created an account but cannot forge a principal.
    ///
    /// A public Magnetar primary-auth witness constructor is required before
    /// this outcome may mint a session.
    AccountCreated {
        /// Newly created user.
        user: User,
        /// Provider account selector.
        provider_account_id: String,
    },
    /// An account was linked without a sign-in witness.
    AccountLinked {
        /// Host users user.
        user: User,
        /// Provider account selector.
        provider_account_id: String,
    },
    /// A verified matching email needs an authenticated explicit-link flow.
    ExplicitLinkRequired {
        /// Normalized matching email.
        normalized_email: String,
    },
    /// An unverified/no-email identity must complete its email proof.
    EmailCompletionRequired {
        /// Pending identity ceremony selector.
        pending_id: String,
    },
}

/// A provider protocol error or Magnetar service error, preserving its
/// classification for the framework route mapper.
#[cfg(feature = "magnetar-oauth")]
#[derive(Debug)]
pub enum HostOAuthError {
    /// OAuth provider/grant error with its 400/401/502/500 class intact.
    Protocol(OAuthProtocolError),
    /// Magnetar ceremony, identity, storage, or factor error.
    Auth(Error),
}

#[cfg(feature = "magnetar-oauth")]
impl From<Error> for HostOAuthError {
    fn from(error: Error) -> Self {
        Self::Auth(error)
    }
}
/// Narrow installed-engine contract consumed by the retained OAuth facade.
#[cfg(feature = "magnetar-oauth")]
#[async_trait]
pub trait MagnetarOAuthAuthEngine: Send + Sync {
    /// Whether this engine owns an explicitly configured provider.
    fn oauth_supports_provider(&self, provider: &str) -> bool;
    /// Start a real Magnetar authorization ceremony.
    async fn oauth_begin(
        &self,
        input: MagnetarOAuthBegin,
    ) -> std::result::Result<MagnetarOAuthKickoff, HostOAuthError>;
    /// Complete callback, identity mapping, factor gate, and session conversion.
    async fn oauth_complete(
        &self,
        input: MagnetarOAuthCallback,
    ) -> std::result::Result<MagnetarOAuthCompletion, HostOAuthError>;
    /// Complete only provider proof for the retained legacy identity hook.
    async fn oauth_verify_identity(
        &self,
        input: MagnetarOAuthCallback,
    ) -> std::result::Result<VerifiedProviderIdentity, HostOAuthError>;
}

/// Concrete OAuth execution assembled from the installed host engine.
#[cfg(feature = "magnetar-oauth")]
pub struct MagnetarHostOAuthEngine<S, O, C, F, A>
where
    S: AuthSchema,
    O: OpaqueSessionStore,
    C: CeremonyStore,
    F: FactorVerifier,
    A: HostUserAdapter,
{
    authorization: OAuthAuthorizationService,
    identity: IdentityResolver,
    providers: OAuthProviderRegistry,
    provider_config: HashMap<&'static str, MagnetarOAuthProviderSettings>,
    transport: Arc<dyn HttpTransport>,
    factor_gate: Arc<OpaqueFactorGate<C, F, O>>,
    users: Arc<A>,
    _schema: PhantomData<S>,
}

#[cfg(feature = "magnetar-oauth")]
impl<S, O, C, F, P, A, L> MagnetarHostEngine<S, O, C, F, P, A, L>
where
    S: AuthSchema,
    O: OpaqueSessionStore + 'static,
    C: CeremonyStore + 'static,
    F: FactorVerifier + 'static,
    P: PasswordAuthProvider,
    A: HostUserAdapter<User = User>,
    L: HostLifecycleDeduplication,
    S::User: UserBinding + UserOptionalFields + SessionEpoch,
    S::Session: SessionFields,
    S::Token: TokenFields,
    S::LinkedAccount: magnetar::schema::LinkedAccountFields,
{
    /// Compose the opt-in OAuth delegate from real host storage and factor
    /// services. It does not install itself; callers must use the explicit
    /// Magnetar integration installer.
    pub fn oauth_service(
        &self,
        config: MagnetarOAuthHostConfig,
    ) -> std::result::Result<MagnetarHostOAuthEngine<S, O, C, F, A>, HostOAuthError> {
        let ceremony_store: Arc<dyn CeremonyStore> = self.ceremonies.clone();
        let storage = Arc::new(SeaOrmStorage::<S>::new(self.binding.database().clone()));
        let users: Arc<dyn UserStore> = storage.clone();
        let accounts: Arc<dyn LinkedAccountStore> = storage;
        Ok(MagnetarHostOAuthEngine {
            authorization: OAuthAuthorizationService::new(
                ceremony_store.clone(),
                Arc::clone(&self.encryptor),
                config.limiter,
                config.authorization,
            ),
            identity: IdentityResolver::new(
                users,
                accounts,
                ceremony_store,
                Arc::clone(&self.first_email_proof),
                Arc::clone(&self.encryptor),
                config.auto_link,
            ),
            providers: config.providers,
            provider_config: config.provider_config,
            transport: config.transport,
            factor_gate: Arc::clone(&self.factor_gate),
            users: Arc::clone(&self.users),
            _schema: PhantomData,
        })
    }
}

#[cfg(feature = "magnetar-oauth")]
impl<S, O, C, F, A> MagnetarHostOAuthEngine<S, O, C, F, A>
where
    S: AuthSchema,
    O: OpaqueSessionStore + 'static,
    C: CeremonyStore + 'static,
    F: FactorVerifier + 'static,
    A: HostUserAdapter<User = User>,
    S::User: UserBinding + UserOptionalFields + SessionEpoch,
    S::Token: TokenFields,
    S::LinkedAccount: magnetar::schema::LinkedAccountFields,
{
    /// True only for a provider explicitly owned by this host delegate.
    #[must_use]
    pub fn supports_provider(&self, provider: &str) -> bool {
        self.providers.get(provider).is_some() && self.provider_config.contains_key(provider)
    }

    /// Begin a ceremony using either the host web-session digest or StateOnly.
    pub async fn begin(
        &self,
        input: MagnetarOAuthBegin,
    ) -> std::result::Result<MagnetarOAuthKickoff, HostOAuthError> {
        let (provider, redirect_uri, scopes) = self.provider_config(&input.provider)?;
        let ceremony = self
            .authorization
            .begin(
                OAuthBeginInput {
                    provider: input.provider,
                    intent: input.intent,
                    actor: input.actor,
                    binding: input.binding,
                },
                provider.authorization_shape().pkce,
                provider.authorization_shape().requires_nonce,
                &input.limiter_identity,
            )
            .await
            .map_err(HostOAuthError::Auth)?;
        let rendered = render_authorization_request(
            &provider.authorization_shape(),
            &AuthorizationRequestParams {
                client_id: provider.client_id().to_owned(),
                redirect_uri: Some(redirect_uri),
                scopes,
                state: Some(ceremony.selector.clone()),
                code_challenge: ceremony.code_challenge,
                nonce: ceremony.nonce,
            },
        )
        .map_err(HostOAuthError::Protocol)?;
        let mut url = reqwest::Url::parse(&provider.authorization_endpoint()).map_err(|_| {
            HostOAuthError::Protocol(OAuthProtocolError::ProviderConfiguration {
                provider: provider.name(),
                message: "authorization endpoint is not an absolute URL".to_owned(),
            })
        })?;
        url.query_pairs_mut().extend_pairs(rendered.iter());
        Ok(MagnetarOAuthKickoff {
            authorization_url: url.into(),
            state: ceremony.selector,
        })
    }

    /// Execute grant and provider identity verification without account mapping.
    pub async fn verify_identity(
        &self,
        input: MagnetarOAuthCallback,
    ) -> std::result::Result<VerifiedProviderIdentity, HostOAuthError> {
        self.callback_identity(input)
            .await
            .map(|(identity, _, _)| identity)
    }

    /// Execute grant, provider identity resolution, and the factor gate where
    /// Magnetar already possesses a verified principal.
    pub async fn complete(
        &self,
        input: MagnetarOAuthCallback,
    ) -> std::result::Result<MagnetarOAuthCompletion, HostOAuthError> {
        let metadata = input.metadata.clone();
        let (identity, intent, actor) = self.callback_identity(input).await?;
        let outcome = self
            .identity
            .resolve(identity.clone(), intent.clone(), actor, metadata.clone())
            .await
            .map_err(HostOAuthError::Auth)?;
        match outcome {
            IdentityOutcome::SignIn(principal) => self.complete_principal(principal).await,
            IdentityOutcome::Create {
                user_id,
                provider_account_id,
            } => {
                self.complete_new_sign_in(identity, metadata, user_id, provider_account_id)
                    .await
            }
            IdentityOutcome::Link { .. } if matches!(intent, OAuthIntent::SignIn) => {
                self.complete_linked_sign_in(identity, metadata).await
            }
            IdentityOutcome::Link {
                actor_user_id,
                provider_account_id,
            } => Ok(MagnetarOAuthCompletion::AccountLinked {
                user: self
                    .users
                    .user_for_id(&actor_user_id)
                    .await
                    .map_err(HostOAuthError::Auth)?,
                provider_account_id,
            }),
            IdentityOutcome::ExplicitLinkRequired { normalized_email } => {
                Ok(MagnetarOAuthCompletion::ExplicitLinkRequired { normalized_email })
            }
            IdentityOutcome::EmailCompletionRequired { pending_id } => {
                Ok(MagnetarOAuthCompletion::EmailCompletionRequired { pending_id })
            }
        }
    }

    async fn complete_new_sign_in(
        &self,
        identity: VerifiedProviderIdentity,
        metadata: SessionMetadata,
        _: String,
        _: String,
    ) -> std::result::Result<MagnetarOAuthCompletion, HostOAuthError> {
        match self
            .identity
            .resolve(identity, OAuthIntent::SignIn, None, metadata)
            .await
            .map_err(HostOAuthError::Auth)?
        {
            IdentityOutcome::SignIn(principal) => self.complete_principal(principal).await,
            _ => Err(HostOAuthError::Auth(Error::Internal {
                message: "newly linked sign-in identity did not resolve to a principal".to_owned(),
            })),
        }
    }
    async fn complete_linked_sign_in(
        &self,
        identity: VerifiedProviderIdentity,
        metadata: SessionMetadata,
    ) -> std::result::Result<MagnetarOAuthCompletion, HostOAuthError> {
        match self
            .identity
            .resolve(identity, OAuthIntent::SignIn, None, metadata)
            .await
            .map_err(HostOAuthError::Auth)?
        {
            IdentityOutcome::SignIn(principal) => self.complete_principal(principal).await,
            _ => Err(HostOAuthError::Auth(Error::Internal {
                message: "linked sign-in identity did not resolve to a principal".to_owned(),
            })),
        }
    }

    async fn complete_principal(
        &self,
        principal: VerifiedPrincipal,
    ) -> std::result::Result<MagnetarOAuthCompletion, HostOAuthError> {
        let user = self
            .users
            .user_for_id(principal.user_id())
            .await
            .map_err(HostOAuthError::Auth)?;
        let context = principal.context().clone();
        match self
            .factor_gate
            .complete_sign_in(principal, context)
            .await
            .map_err(HostOAuthError::Auth)?
        {
            SignInDecision::SessionAllowed(grant) => Ok(MagnetarOAuthCompletion::SessionAllowed {
                user,
                session: Box::new(grant.try_into().map_err(HostOAuthError::Auth)?),
            }),
            SignInDecision::FactorRequired { challenge_selector } => {
                Ok(MagnetarOAuthCompletion::FactorRequired { challenge_selector })
            }
        }
    }

    async fn callback_identity(
        &self,
        input: MagnetarOAuthCallback,
    ) -> std::result::Result<
        (
            VerifiedProviderIdentity,
            OAuthIntent,
            Option<CredentialActor>,
        ),
        HostOAuthError,
    > {
        let (provider, redirect_uri, scopes) = self.provider_config(&input.provider)?;
        let ceremony = self
            .authorization
            .complete(OAuthCallbackInput {
                provider: input.provider.clone(),
                state: input.state.clone(),
                host_session_digest: input.host_session_digest,
            })
            .await
            .map_err(HostOAuthError::Auth)?;
        let intent = ceremony.intent.clone();
        let actor = ceremony.actor.clone();
        let token = authorization_code::execute_with_raw(
            provider.as_ref(),
            self.transport.as_ref(),
            &ceremony,
            input.code,
            Some(redirect_uri),
            scopes,
        )
        .await
        .map_err(HostOAuthError::Protocol)?;
        let response = if provider.name() == "apple" {
            let id_token = token.response.id_token.clone().ok_or_else(|| {
                HostOAuthError::Protocol(OAuthProtocolError::MalformedTokenResponse {
                    message: "Apple token response omitted id_token".to_owned(),
                })
            })?;
            ProviderResponse::AppleIdToken {
                id_token,
                nonce: ceremony.nonce,
                form_post_user: input.form_post_user,
            }
        } else {
            let endpoint = provider.userinfo_endpoint().ok_or_else(|| {
                HostOAuthError::Protocol(OAuthProtocolError::ProviderConfiguration {
                    provider: provider.name(),
                    message: "provider requires a userinfo endpoint or an Apple callback"
                        .to_owned(),
                })
            })?;
            let mut headers = provider.userinfo_headers();
            if headers
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("authorization"))
            {
                return Err(HostOAuthError::Protocol(
                    OAuthProtocolError::ProviderConfiguration {
                        provider: provider.name(),
                        message: "userinfo_headers must not override Authorization".to_owned(),
                    },
                ));
            }
            headers.push((
                "Authorization".to_owned(),
                format!("Bearer {}", token.response.access_token.expose_secret()),
            ));
            let response = self
                .transport
                .send(HttpRequest {
                    method: "GET".to_owned(),
                    url: endpoint,
                    headers,
                    body: Vec::new(),
                })
                .await
                .map_err(HostOAuthError::Auth)?;
            if !(200..300).contains(&response.status) {
                return Err(HostOAuthError::Protocol(
                    OAuthProtocolError::UpstreamUnavailable {
                        provider: provider.name(),
                        message: format!("userinfo endpoint returned HTTP {}", response.status),
                        retry_after_seconds: None,
                    },
                ));
            }
            ProviderResponse::UserInfo {
                body: String::from_utf8(response.body).map_err(|_| {
                    HostOAuthError::Protocol(OAuthProtocolError::MalformedProviderResponse {
                        provider: provider.name(),
                        message: "userinfo response was not UTF-8".to_owned(),
                    })
                })?,
            }
        };
        provider
            .resolve_identity(response)
            .await
            .map_err(HostOAuthError::Protocol)
            .map(|identity| (identity, intent, actor))
    }

    fn provider_config(
        &self,
        name: &str,
    ) -> std::result::Result<OAuthProviderConfigLookup, HostOAuthError> {
        let provider = self.providers.get(name).ok_or_else(|| {
            HostOAuthError::Auth(Error::NotFound {
                resource: "OAuth provider".to_owned(),
                identifier: name.to_owned(),
            })
        })?;
        let config = self.provider_config.get(name).ok_or_else(|| {
            HostOAuthError::Auth(Error::NotFound {
                resource: "OAuth provider configuration".to_owned(),
                identifier: name.to_owned(),
            })
        })?;
        Ok((provider, config.redirect_uri.clone(), config.scopes.clone()))
    }
}

#[cfg(feature = "magnetar-oauth")]
#[async_trait]
impl<S, O, C, F, A> MagnetarOAuthAuthEngine for MagnetarHostOAuthEngine<S, O, C, F, A>
where
    S: AuthSchema + Send + Sync,
    O: OpaqueSessionStore + 'static,
    C: CeremonyStore + 'static,
    F: FactorVerifier + 'static,
    A: HostUserAdapter<User = User>,
    S::User: UserBinding + UserOptionalFields + SessionEpoch,
    S::Token: TokenFields,
    S::LinkedAccount: magnetar::schema::LinkedAccountFields,
{
    fn oauth_supports_provider(&self, provider: &str) -> bool {
        self.supports_provider(provider)
    }

    async fn oauth_begin(
        &self,
        input: MagnetarOAuthBegin,
    ) -> std::result::Result<MagnetarOAuthKickoff, HostOAuthError> {
        self.begin(input).await
    }

    async fn oauth_complete(
        &self,
        input: MagnetarOAuthCallback,
    ) -> std::result::Result<MagnetarOAuthCompletion, HostOAuthError> {
        self.complete(input).await
    }

    async fn oauth_verify_identity(
        &self,
        input: MagnetarOAuthCallback,
    ) -> std::result::Result<VerifiedProviderIdentity, HostOAuthError> {
        self.verify_identity(input).await
    }
}
