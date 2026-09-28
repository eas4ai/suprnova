//! Magnetar authentication integration for Suprnova.
//!
//! The framework owns its public authentication values and application binding;
//! Magnetar provides credential, ceremony, factor, session, OAuth, and storage
//! engines behind the facades in this module.

pub mod abuse_limiter;
pub mod default_engine;
/// Magnetar host-engine composition and installed dispatch support.
pub mod engine;
pub mod magic_link;
#[cfg(any(
    feature = "database-sqlite",
    feature = "database-postgres",
    feature = "database-mysql"
))]
pub mod middleware;
#[cfg(feature = "magnetar-oauth")]
pub mod oauth;
#[cfg(feature = "magnetar-oauth")]
pub mod oauth_transport;
pub mod passkey;
pub mod password;
mod sign_in;

use std::sync::{Arc, Mutex, OnceLock};

pub mod ceremony;

use crate::error::FrameworkError;

pub use crate::auth::{LockoutStatus, Session, SessionToken, User, UserId};
pub use default_engine::{MagnetarConfig, init_magnetar};
#[cfg(feature = "magnetar-oauth")]
pub use default_engine::{MagnetarOAuthOnlyConfig, init_magnetar_oauth_only};
pub use engine::MagnetarFactorAuthEngine;
pub use sign_in::SignInOutcome;

/// Completion facade for factor challenges returned by Magnetar sign-in providers.
pub struct FactorAuth;

impl FactorAuth {
    /// Complete one factor challenge and establish the framework session.
    ///
    /// Use the selector from [`SignInOutcome::FactorRequired`]. Successful
    /// completion rotates the framework session id and CSRF token before the
    /// request ends, matching an immediately authenticated sign-in.
    ///
    /// # Errors
    ///
    /// Returns an error when session middleware or the installed engine is
    /// unavailable, the selector or code is invalid or already used, user
    /// lookup fails, or session binding fails.
    pub async fn complete_challenge(
        &self,
        selector: &str,
        code: &str,
    ) -> Result<(User, Session), FrameworkError> {
        factor_bind_scope_preflight()?;
        let engine = factor_engine()?;
        let issued = engine
            .complete_challenge(selector, code)
            .await
            .map_err(password::map_magnetar_password_error)?;
        let user_id = issued.session.user_id.to_string();
        let lookup_engine = engine.clone();
        handoff_issued_session(engine.clone(), issued, false, async move {
            lookup_engine
                .user_by_id(&user_id)
                .await
                .map_err(password::map_magnetar_password_error)?
                .ok_or_else(|| {
                    FrameworkError::internal("factor-completed session user was not found")
                })
        })
        .await
    }
}

fn factor_bind_scope_preflight() -> Result<(), FrameworkError> {
    if !crate::session::middleware::session_scope_installed()
        || !crate::session::middleware::pending_cookies_scope_installed()
        || !crate::session::middleware::pending_remember_revocations_scope_installed()
    {
        return Err(FrameworkError::internal(
            "factor challenge completion requires active SessionMiddleware scopes",
        ));
    }
    bind_scope_preflight()
}

pub(crate) fn bind_scope_preflight() -> Result<(), FrameworkError> {
    let default_guard = crate::auth::Auth::default_guard_name();
    let replaces_remember =
        crate::auth::request_state::active_remember_selector_for_guard(&default_guard).is_some();
    if replaces_remember
        && (!crate::session::middleware::session_scope_installed()
            || !crate::session::middleware::pending_cookies_scope_installed()
            || !crate::session::middleware::pending_remember_revocations_scope_installed())
    {
        return Err(FrameworkError::internal(
            "Magnetar session binding requires active session, cookie, and remember-cleanup scopes",
        ));
    }
    Ok(())
}

pub(crate) fn bind_issued_session(
    issued: &engine::MagnetarIssuedSession,
    password_confirmed: bool,
) -> Result<(), FrameworkError> {
    bind_scope_preflight()?;
    let default_guard = crate::auth::Auth::default_guard_name();
    let verified_remember =
        crate::auth::Auth::prepare_guard_remember_identity_replacement(&default_guard);
    // Binding is deliberately synchronous: the fresh Magnetar session has
    // already been issued, so awaiting A's selector revocation here would make
    // a revoke failure strand durable B state behind a failed framework bind.
    // Retain exact cleanup in SessionMiddleware's request-local queue instead.
    // The preparation step also replaces A's queued carrier with a forget
    // directive and clears its request provenance.
    if let Some((owner, selector)) = verified_remember
        && !crate::session::middleware::push_pending_remember_revocation(
            &default_guard,
            owner.clone(),
            selector.clone(),
        )
    {
        crate::auth::request_state::set_verified_active_remember_carrier(
            &default_guard,
            &owner,
            &selector,
        );
        return Err(FrameworkError::internal(
            "fresh Magnetar session bind could not retain remember cleanup",
        ));
    }
    let user_id = issued.session.user_id.to_string();
    crate::session::session_mut(|session| {
        session.rotate_id(crate::session::generate_session_id());
        session.csrf_token = crate::session::generate_csrf_token();
        session.clear_magnetar_web_binding();
        session.replace_auth_guard_id(&default_guard, user_id.clone());
        session.user_id = Some(user_id.clone());
        session.set_auth_guard_magnetar_binding(&default_guard, issued.web_binding.clone());
        session.set_magnetar_web_binding(issued.web_binding.clone());
        if password_confirmed {
            session.password_confirmed();
        }
        session.dirty = true;
    });
    crate::auth::request_state::set_guard_user_id(&default_guard, user_id);
    crate::auth::request_state::set_guard_via_remember(&default_guard, false);
    crate::auth::request_state::clear_active_remember_carrier_for_guard(&default_guard);
    Ok(())
}

const HANDOFF_CLEANUP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

#[derive(Clone, Copy)]
pub(crate) enum HandoffCleanupOutcome {
    Retired,
    Unconfirmed,
    BackendFailure,
    TimedOut,
}

struct IssuedSessionHandoffOwner {
    authority: Arc<dyn engine::MagnetarFactorAuthEngine>,
    session_id: String,
    pending: Option<Arc<Mutex<Vec<crate::session::middleware::PendingOpaqueSession>>>>,
    armed: bool,
}

impl IssuedSessionHandoffOwner {
    fn new(
        authority: Arc<dyn engine::MagnetarFactorAuthEngine>,
        session_id: String,
        pending: Option<Arc<Mutex<Vec<crate::session::middleware::PendingOpaqueSession>>>>,
    ) -> Self {
        Self {
            authority,
            session_id,
            pending,
            armed: true,
        }
    }

    async fn abort(&mut self, reason: &'static str) -> Result<(), FrameworkError> {
        self.armed = false;
        let outcome = if self.pending.is_some() {
            retire_issued_session_batch(
                self.authority.clone(),
                self.pending.clone(),
                vec![self.session_id.clone()],
                1,
                reason,
            )
            .await
        } else {
            let Some(cleanup) = spawn_issued_session_batch_cleanup(
                self.authority.clone(),
                None,
                vec![self.session_id.clone()],
                2,
                reason,
            ) else {
                return Err(FrameworkError::internal(
                    "fresh Magnetar session cleanup could not start",
                ));
            };
            cleanup
                .await
                .unwrap_or(HandoffCleanupOutcome::BackendFailure)
        };
        match outcome {
            HandoffCleanupOutcome::Retired => Ok(()),
            HandoffCleanupOutcome::Unconfirmed => Err(FrameworkError::internal(
                "fresh Magnetar session cleanup could not confirm retirement",
            )),
            HandoffCleanupOutcome::BackendFailure | HandoffCleanupOutcome::TimedOut => Err(
                FrameworkError::internal("fresh Magnetar session cleanup did not complete"),
            ),
        }
    }

    fn release(mut self) {
        self.armed = false;
    }
}

impl Drop for IssuedSessionHandoffOwner {
    fn drop(&mut self) {
        if self.armed && self.pending.is_none() {
            let _ = spawn_issued_session_batch_cleanup(
                self.authority.clone(),
                None,
                vec![self.session_id.clone()],
                2,
                "handoff future cancelled",
            );
        }
    }
}

fn spawn_issued_session_batch_cleanup(
    authority: Arc<dyn engine::MagnetarFactorAuthEngine>,
    pending: Option<Arc<Mutex<Vec<crate::session::middleware::PendingOpaqueSession>>>>,
    session_ids: Vec<String>,
    attempts_per_session: usize,
    reason: &'static str,
) -> Option<tokio::task::JoinHandle<HandoffCleanupOutcome>> {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        tracing::warn!(
            operation = "opaque_session_retirement",
            reason,
            classification = "runtime_unavailable",
            "fresh Magnetar session cleanup could not start"
        );
        return None;
    };
    Some(runtime.spawn(retire_issued_session_batch(
        authority,
        pending,
        session_ids,
        attempts_per_session,
        reason,
    )))
}

pub(crate) async fn retire_issued_session_batch(
    authority: Arc<dyn engine::MagnetarFactorAuthEngine>,
    pending: Option<Arc<Mutex<Vec<crate::session::middleware::PendingOpaqueSession>>>>,
    session_ids: Vec<String>,
    attempts_per_session: usize,
    reason: &'static str,
) -> HandoffCleanupOutcome {
    let revocations = session_ids.into_iter().map(|session_id| {
        let authority = authority.clone();
        let pending = pending.clone();
        async move {
            let mut outcome = HandoffCleanupOutcome::BackendFailure;
            for _ in 0..attempts_per_session {
                outcome = match authority.revoke_session(&session_id).await {
                    Ok(true) => {
                        if let Some(pending) = pending.as_ref() {
                            crate::session::middleware::confirm_pending_opaque_session_retired(
                                pending,
                                &session_id,
                            );
                        }
                        return HandoffCleanupOutcome::Retired;
                    }
                    Ok(false) => HandoffCleanupOutcome::Unconfirmed,
                    Err(_) => HandoffCleanupOutcome::BackendFailure,
                };
            }
            outcome
        }
    });
    let outcome = match tokio::time::timeout(
        HANDOFF_CLEANUP_TIMEOUT,
        futures_util::future::join_all(revocations),
    )
    .await
    {
        Err(_) => HandoffCleanupOutcome::TimedOut,
        Ok(outcomes)
            if outcomes
                .iter()
                .all(|outcome| matches!(outcome, HandoffCleanupOutcome::Retired)) =>
        {
            HandoffCleanupOutcome::Retired
        }
        Ok(outcomes)
            if outcomes
                .iter()
                .any(|outcome| matches!(outcome, HandoffCleanupOutcome::BackendFailure)) =>
        {
            HandoffCleanupOutcome::BackendFailure
        }
        Ok(_) => HandoffCleanupOutcome::Unconfirmed,
    };
    if !matches!(outcome, HandoffCleanupOutcome::Retired) {
        let classification = match outcome {
            HandoffCleanupOutcome::Retired => "retired",
            HandoffCleanupOutcome::Unconfirmed => "retirement_unconfirmed",
            HandoffCleanupOutcome::BackendFailure => "backend_failure",
            HandoffCleanupOutcome::TimedOut => "timeout",
        };
        tracing::warn!(
            operation = "opaque_session_retirement",
            reason,
            classification,
            "fresh Magnetar session cleanup did not complete"
        );
    }
    outcome
}

pub(crate) async fn retire_pending_issued_session(
    authority: Option<Arc<dyn engine::MagnetarFactorAuthEngine>>,
    pending: Arc<Mutex<Vec<crate::session::middleware::PendingOpaqueSession>>>,
    session_id: String,
    reason: &'static str,
) {
    let Some(authority) = authority else {
        tracing::error!(
            operation = "opaque_session_retirement",
            reason,
            classification = "authority_unavailable",
            "fresh Magnetar session cleanup remains pending"
        );
        return;
    };
    let _ =
        retire_issued_session_batch(authority, Some(pending), vec![session_id], 1, reason).await;
}

pub(crate) fn schedule_pending_issued_session_cleanup(
    authority: Option<Arc<dyn engine::MagnetarFactorAuthEngine>>,
    pending: Arc<Mutex<Vec<crate::session::middleware::PendingOpaqueSession>>>,
    reason: &'static str,
) {
    if pending.lock().unwrap().is_empty() {
        return;
    }
    let Some(authority) = authority else {
        tracing::error!(
            operation = "opaque_session_retirement",
            reason,
            classification = "authority_unavailable",
            "fresh Magnetar session cleanup remains pending"
        );
        return;
    };
    let session_ids = pending
        .lock()
        .unwrap()
        .iter()
        .map(|candidate| candidate.session_id.clone())
        .collect();
    let _ = spawn_issued_session_batch_cleanup(authority, Some(pending), session_ids, 1, reason);
}

/// Hand an already-persisted opaque session to framework persistence.
///
/// The opaque and framework stores cannot share a transaction. The request-local
/// owner therefore keeps the opaque row provisional, explicitly retires it on
/// every observed abort, and lets `SessionMiddleware` release ownership only
/// after the framework row and response cookie are both ready. A process crash
/// between stores remains bounded by the opaque session's configured lifetime.
/// Without middleware, a successful direct caller owns the returned opaque
/// session. A failed or cancelled direct handoff uses two bounded cleanup
/// attempts in a detached task; runtime shutdown can still defer retirement to
/// the opaque session lifetime.
pub(crate) async fn handoff_issued_session<F>(
    authority: Arc<dyn engine::MagnetarFactorAuthEngine>,
    issued: engine::MagnetarIssuedSession,
    password_confirmed: bool,
    resolve_user: F,
) -> Result<(User, Session), FrameworkError>
where
    F: std::future::Future<Output = Result<User, FrameworkError>>,
{
    let pending = crate::session::middleware::PendingOpaqueSession {
        guard_name: crate::auth::Auth::default_guard_name(),
        session_id: issued.session_id.clone(),
        binding: issued.web_binding.clone(),
    };
    let pending_slot = crate::session::middleware::pending_opaque_session_slot();
    let mut owner =
        IssuedSessionHandoffOwner::new(authority, issued.session_id.clone(), pending_slot.clone());
    if pending_slot.is_some()
        && let Err(error) = crate::session::middleware::register_pending_opaque_session(pending)
    {
        owner.abort("handoff registration failure").await?;
        return Err(error);
    }

    let user = match resolve_user.await {
        Ok(user) => user,
        Err(error) => {
            owner.abort("host user lookup failure").await?;
            return Err(error);
        }
    };
    if let Err(error) = bind_issued_session(&issued, password_confirmed) {
        owner.abort("framework binding failure").await?;
        return Err(error);
    }
    owner.release();
    Ok((user, issued.session))
}

pub use magnetar::passkey::PasskeyConfig;

/// Initialized Magnetar authentication engine.
///
/// Application boot supplies the real host engine; replacing it is rejected
/// so authentication state cannot split across independently configured stores.
struct EngineInstallState {
    reserved: bool,
}

static ENGINE_INSTALL_GUARD: Mutex<EngineInstallState> =
    Mutex::new(EngineInstallState { reserved: false });
static MAGNETAR_PASSWORD_ENGINE: OnceLock<Arc<dyn engine::MagnetarPasswordAuthEngine>> =
    OnceLock::new();
static MAGNETAR_FACTOR_ENGINE: OnceLock<Arc<dyn engine::MagnetarFactorAuthEngine>> =
    OnceLock::new();

struct PasswordFactorEngine {
    password: Arc<dyn engine::MagnetarPasswordAuthEngine>,
}

#[async_trait::async_trait]
impl engine::MagnetarFactorAuthEngine for PasswordFactorEngine {
    async fn complete_challenge(
        &self,
        selector: &str,
        code: &str,
    ) -> magnetar::Result<engine::MagnetarIssuedSession> {
        self.password.complete_challenge(selector, code).await
    }

    async fn user_by_id(&self, user_id: &str) -> magnetar::Result<Option<User>> {
        self.password.user_by_id(user_id).await
    }

    async fn resolve_web_binding(
        &self,
        binding: &magnetar::sessions::WebSessionBinding,
    ) -> magnetar::Result<magnetar::sessions::VerifiedSession> {
        self.password.resolve_web_binding(binding).await
    }

    async fn bearer_user_id(&self, token: &str) -> magnetar::Result<Option<String>> {
        self.password.bearer_user_id(token).await
    }

    async fn revoke_session(&self, session_id: &str) -> magnetar::Result<bool> {
        self.password.revoke_session(session_id).await
    }

    async fn revoke_all_sessions(&self, user_id: &str) -> magnetar::Result<u64> {
        self.password.revoke_all_sessions(user_id).await
    }

    async fn list_sessions(
        &self,
        user_id: &str,
    ) -> magnetar::Result<Vec<magnetar::sessions::SessionSummary>> {
        self.password.list_sessions(user_id).await
    }
}

pub(crate) fn password_factor_engine(
    password: Arc<dyn engine::MagnetarPasswordAuthEngine>,
) -> Arc<dyn engine::MagnetarFactorAuthEngine> {
    Arc::new(PasswordFactorEngine { password })
}

/// Initialized Magnetar passkey engine.
///
/// Passkey relying-party configuration is supplied explicitly by application boot.
static MAGNETAR_PASSKEY_ENGINE: OnceLock<Arc<dyn engine::MagnetarPasskeyAuthEngine>> =
    OnceLock::new();
/// Initialized Magnetar OAuth engine and authoritative provider registry.
#[cfg(feature = "magnetar-oauth")]
static MAGNETAR_OAUTH_ENGINE: OnceLock<Arc<dyn engine::MagnetarOAuthAuthEngine>> = OnceLock::new();

#[cfg(feature = "magnetar-oauth")]
type OptionalMagnetarOAuthEngine = Option<Arc<dyn engine::MagnetarOAuthAuthEngine>>;
#[cfg(not(feature = "magnetar-oauth"))]
type OptionalMagnetarOAuthEngine = ();

#[cfg(feature = "magnetar-oauth")]
fn no_oauth_engine() -> OptionalMagnetarOAuthEngine {
    None
}
#[cfg(not(feature = "magnetar-oauth"))]
fn no_oauth_engine() {}

#[cfg(feature = "magnetar-oauth")]
fn oauth_engine_is_installed() -> bool {
    MAGNETAR_OAUTH_ENGINE.get().is_some()
}
#[cfg(not(feature = "magnetar-oauth"))]
fn oauth_engine_is_installed() -> bool {
    false
}

/// Atomically install password/session and passkey adapters from one host
/// engine bundle. Factor completion delegates to the password adapter for
/// compatibility with existing custom bundles.
pub fn install_magnetar_engines(
    password: Arc<dyn engine::MagnetarPasswordAuthEngine>,
    passkey: Arc<dyn engine::MagnetarPasskeyAuthEngine>,
) -> Result<(), FrameworkError> {
    let factor = password_factor_engine(password.clone());
    reserve_magnetar_engines()?.install(password, passkey, factor, no_oauth_engine())
}

/// Atomically install distinct password, passkey, and factor/session owners.
///
/// Use this form when provider adapters are separate objects. Every adapter
/// that can return [`SignInOutcome::FactorRequired`] must issue selectors into
/// the supplied factor owner's store. Every provider-issued web binding,
/// including an immediately authenticated outcome, must use that same owner's
/// opaque-session store so [`crate::session::SessionMiddleware`] can validate
/// it through one authoritative path.
///
/// # Errors
///
/// Returns an error when another installation is reserved or already visible.
pub fn install_magnetar_engines_with_factor(
    password: Arc<dyn engine::MagnetarPasswordAuthEngine>,
    passkey: Arc<dyn engine::MagnetarPasskeyAuthEngine>,
    factor: Arc<dyn engine::MagnetarFactorAuthEngine>,
) -> Result<(), FrameworkError> {
    reserve_magnetar_engines()?.install(password, passkey, factor, no_oauth_engine())
}

/// Test-only password adapter installation for isolated facade harnesses.
#[cfg(feature = "testing")]
#[doc(hidden)]
pub fn install_magnetar_password_engine_for_test(
    engine: Arc<dyn engine::MagnetarPasswordAuthEngine>,
) -> Result<(), FrameworkError> {
    let guard = engine_install_guard()?;
    if guard.reserved
        || MAGNETAR_PASSWORD_ENGINE.get().is_some()
        || MAGNETAR_FACTOR_ENGINE.get().is_some()
    {
        return Err(FrameworkError::internal(
            "Magnetar password engine is already installed or reserved",
        ));
    }
    let factor = password_factor_engine(engine.clone());
    MAGNETAR_PASSWORD_ENGINE
        .set(engine)
        .map_err(|_| FrameworkError::internal("Magnetar password engine installation raced"))?;
    MAGNETAR_FACTOR_ENGINE
        .set(factor)
        .map_err(|_| FrameworkError::internal("Magnetar factor engine installation raced"))
}

/// Test-only passkey adapter installation for isolated facade harnesses.
#[cfg(feature = "testing")]
#[doc(hidden)]
pub fn install_magnetar_passkey_engine_for_test(
    engine: Arc<dyn engine::MagnetarPasskeyAuthEngine>,
) -> Result<(), FrameworkError> {
    let guard = engine_install_guard()?;
    if guard.reserved || MAGNETAR_PASSKEY_ENGINE.get().is_some() {
        return Err(FrameworkError::internal(
            "Magnetar passkey engine is already installed or reserved",
        ));
    }
    MAGNETAR_PASSKEY_ENGINE
        .set(engine)
        .map_err(|_| FrameworkError::internal("Magnetar passkey engine installation raced"))
}

/// Install a passkey adapter together with the authority that owns its sessions.
#[cfg(feature = "testing")]
#[doc(hidden)]
pub fn install_magnetar_passkey_engine_with_factor_for_test(
    passkey: Arc<dyn engine::MagnetarPasskeyAuthEngine>,
    factor: Arc<dyn engine::MagnetarFactorAuthEngine>,
) -> Result<(), FrameworkError> {
    let guard = engine_install_guard()?;
    if guard.reserved
        || MAGNETAR_PASSKEY_ENGINE.get().is_some()
        || MAGNETAR_FACTOR_ENGINE.get().is_some()
    {
        return Err(FrameworkError::internal(
            "Magnetar passkey or factor engine is already installed or reserved",
        ));
    }
    MAGNETAR_PASSKEY_ENGINE
        .set(passkey)
        .map_err(|_| FrameworkError::internal("Magnetar passkey engine installation raced"))?;
    MAGNETAR_FACTOR_ENGINE
        .set(factor)
        .map_err(|_| FrameworkError::internal("Magnetar factor engine installation raced"))
}

pub(crate) struct EngineInstallReservation {
    active: bool,
}

impl EngineInstallReservation {
    pub(crate) fn install(
        mut self,
        password: Arc<dyn engine::MagnetarPasswordAuthEngine>,
        passkey: Arc<dyn engine::MagnetarPasskeyAuthEngine>,
        factor: Arc<dyn engine::MagnetarFactorAuthEngine>,
        oauth: OptionalMagnetarOAuthEngine,
    ) -> Result<(), FrameworkError> {
        let mut guard = engine_install_guard()?;
        if !self.active || !guard.reserved {
            return Err(FrameworkError::internal(
                "Magnetar engine installation reservation is not active",
            ));
        }
        if MAGNETAR_PASSWORD_ENGINE.get().is_some()
            || MAGNETAR_PASSKEY_ENGINE.get().is_some()
            || MAGNETAR_FACTOR_ENGINE.get().is_some()
            || oauth_engine_is_installed()
        {
            return Err(FrameworkError::internal(
                "Magnetar authentication engines are already installed",
            ));
        }
        MAGNETAR_PASSWORD_ENGINE
            .set(password)
            .map_err(|_| FrameworkError::internal("Magnetar password engine installation raced"))?;
        MAGNETAR_PASSKEY_ENGINE
            .set(passkey)
            .map_err(|_| FrameworkError::internal("Magnetar passkey engine installation raced"))?;
        MAGNETAR_FACTOR_ENGINE
            .set(factor)
            .map_err(|_| FrameworkError::internal("Magnetar factor engine installation raced"))?;
        #[cfg(feature = "magnetar-oauth")]
        if let Some(oauth) = oauth {
            MAGNETAR_OAUTH_ENGINE.set(oauth).map_err(|_| {
                FrameworkError::internal("Magnetar OAuth engine installation raced")
            })?;
        }
        #[cfg(not(feature = "magnetar-oauth"))]
        let _ = oauth;
        guard.reserved = false;
        self.active = false;
        Ok(())
    }

    #[cfg(feature = "magnetar-oauth")]
    pub(crate) fn install_oauth_only(
        mut self,
        oauth: Arc<dyn engine::MagnetarOAuthAuthEngine>,
        factor: Arc<dyn engine::MagnetarFactorAuthEngine>,
    ) -> Result<(), FrameworkError> {
        let mut guard = engine_install_guard()?;
        if !self.active || !guard.reserved {
            return Err(FrameworkError::internal(
                "Magnetar engine installation reservation is not active",
            ));
        }
        if MAGNETAR_PASSWORD_ENGINE.get().is_some()
            || MAGNETAR_PASSKEY_ENGINE.get().is_some()
            || MAGNETAR_FACTOR_ENGINE.get().is_some()
            || MAGNETAR_OAUTH_ENGINE.get().is_some()
        {
            return Err(FrameworkError::internal(
                "Magnetar authentication engines are already installed",
            ));
        }
        MAGNETAR_FACTOR_ENGINE
            .set(factor)
            .map_err(|_| FrameworkError::internal("Magnetar factor engine installation raced"))?;
        MAGNETAR_OAUTH_ENGINE
            .set(oauth)
            .map_err(|_| FrameworkError::internal("Magnetar OAuth engine installation raced"))?;
        guard.reserved = false;
        self.active = false;
        Ok(())
    }
}

impl Drop for EngineInstallReservation {
    fn drop(&mut self) {
        if self.active {
            let mut guard = ENGINE_INSTALL_GUARD
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard.reserved = false;
        }
    }
}

pub(crate) fn reserve_magnetar_engines() -> Result<EngineInstallReservation, FrameworkError> {
    let mut guard = engine_install_guard()?;
    if guard.reserved
        || MAGNETAR_PASSWORD_ENGINE.get().is_some()
        || MAGNETAR_PASSKEY_ENGINE.get().is_some()
        || MAGNETAR_FACTOR_ENGINE.get().is_some()
        || oauth_engine_is_installed()
    {
        return Err(FrameworkError::internal(
            "Magnetar authentication engines are already installed or reserved",
        ));
    }
    guard.reserved = true;
    Ok(EngineInstallReservation { active: true })
}

fn ensure_engine_installation_ready(reserved: bool) -> Result<(), FrameworkError> {
    if reserved {
        return Err(FrameworkError::internal(
            "Magnetar authentication subsystem initialization is in progress; finish application bootstrap before serving requests by awaiting init_magnetar(...) or init_magnetar_oauth_only(...), whichever was started",
        ));
    }
    Ok(())
}

fn factor_engine() -> Result<Arc<dyn engine::MagnetarFactorAuthEngine>, FrameworkError> {
    let guard = engine_install_guard()?;
    ensure_engine_installation_ready(guard.reserved)?;
    MAGNETAR_FACTOR_ENGINE
        .get()
        .cloned()
        .ok_or_else(|| {
            FrameworkError::internal(
                "Magnetar factor/session authentication subsystem was not initialized during application bootstrap; call init_magnetar(...), install_magnetar_engines(...), install_magnetar_engines_with_factor(...), or install_magnetar_oauth_engine_with_factor(...) during bootstrap",
            )
        })
}

pub(crate) fn optional_factor_engine() -> Option<Arc<dyn engine::MagnetarFactorAuthEngine>> {
    let guard = engine_install_guard().ok()?;
    if guard.reserved {
        return None;
    }
    MAGNETAR_FACTOR_ENGINE.get().cloned()
}

pub(crate) fn password_engine()
-> Result<Arc<dyn engine::MagnetarPasswordAuthEngine>, FrameworkError> {
    let guard = engine_install_guard()?;
    ensure_engine_installation_ready(guard.reserved)?;
    MAGNETAR_PASSWORD_ENGINE
        .get()
        .cloned()
        .ok_or_else(|| {
            FrameworkError::internal(
                "Magnetar password authentication subsystem was not initialized during application bootstrap; call init_magnetar(...) or install_magnetar_engines(...)/install_magnetar_engines_with_factor(...) during bootstrap",
            )
        })
}

pub(crate) fn password_engine_if_installed()
-> Result<Option<Arc<dyn engine::MagnetarPasswordAuthEngine>>, FrameworkError> {
    let guard = engine_install_guard()?;
    ensure_engine_installation_ready(guard.reserved)?;
    Ok(MAGNETAR_PASSWORD_ENGINE.get().cloned())
}

pub(crate) fn optional_password_engine() -> Option<Arc<dyn engine::MagnetarPasswordAuthEngine>> {
    let guard = engine_install_guard().ok()?;
    if guard.reserved {
        return None;
    }
    MAGNETAR_PASSWORD_ENGINE.get().cloned()
}

pub(crate) fn passkey_engine() -> Result<Arc<dyn engine::MagnetarPasskeyAuthEngine>, FrameworkError>
{
    let guard = engine_install_guard()?;
    ensure_engine_installation_ready(guard.reserved)?;
    MAGNETAR_PASSKEY_ENGINE
        .get()
        .cloned()
        .ok_or_else(|| {
            FrameworkError::internal(
                "Magnetar passkey authentication subsystem was not initialized during application bootstrap; call init_magnetar(...) or install_magnetar_engines(...)/install_magnetar_engines_with_factor(...) during bootstrap",
            )
        })
}

fn engine_install_guard()
-> Result<std::sync::MutexGuard<'static, EngineInstallState>, FrameworkError> {
    ENGINE_INSTALL_GUARD
        .lock()
        .map_err(|_| FrameworkError::internal("Magnetar engine install lock poisoned"))
}
/// Install an OAuth adapter for identity verification.
///
/// Default-engine applications configure OAuth through
/// [`MagnetarConfig::oauth`] so password, passkey, and OAuth adapters publish
/// under one reservation. This compatibility installer publishes no
/// factor/session authority. OAuth identity verification remains available,
/// but sign-in completion fails before consuming the callback unless another
/// installation already supplied that authority. Use
/// [`install_magnetar_oauth_engine_with_factor`] for a standalone custom OAuth
/// sign-in installation. Replacing an adapter is rejected.
#[cfg(feature = "magnetar-oauth")]
pub fn install_magnetar_oauth_engine(
    engine: Arc<dyn engine::MagnetarOAuthAuthEngine>,
) -> Result<(), FrameworkError> {
    let guard = engine_install_guard()?;
    if guard.reserved || MAGNETAR_OAUTH_ENGINE.get().is_some() {
        return Err(FrameworkError::internal(
            "Magnetar OAuth engine is already installed or reserved",
        ));
    }
    MAGNETAR_OAUTH_ENGINE
        .set(engine)
        .map_err(|_| FrameworkError::internal("Magnetar OAuth engine installation raced"))
}

/// Atomically install a custom OAuth adapter and its factor/session authority.
///
/// The supplied authority must own every factor selector and opaque session
/// issued by the OAuth adapter.
///
/// # Errors
///
/// Returns an error without publishing either component when another Magnetar
/// engine installation is reserved or already visible.
#[cfg(feature = "magnetar-oauth")]
pub fn install_magnetar_oauth_engine_with_factor(
    oauth: Arc<dyn engine::MagnetarOAuthAuthEngine>,
    factor: Arc<dyn engine::MagnetarFactorAuthEngine>,
) -> Result<(), FrameworkError> {
    reserve_magnetar_engines()?.install_oauth_only(oauth, factor)
}

/// Revoke one active Magnetar session by its stable row identifier.
pub async fn revoke_session(session_id: &str) -> Result<bool, FrameworkError> {
    let engine = factor_engine()?;
    engine
        .revoke_session(session_id)
        .await
        .map_err(|error| FrameworkError::internal(format!("revoke Magnetar session: {error}")))
}

/// Revoke every active Magnetar session for one application user.
pub async fn revoke_all_sessions(user_id: &str) -> Result<u64, FrameworkError> {
    let engine = factor_engine()?;
    engine
        .revoke_all_sessions(user_id)
        .await
        .map_err(|error| FrameworkError::internal(format!("revoke Magnetar sessions: {error}")))
}

/// List active Magnetar sessions for one application user.
pub async fn list_sessions(
    user_id: &str,
) -> Result<Vec<magnetar::sessions::SessionSummary>, FrameworkError> {
    let engine = factor_engine()?;
    engine
        .list_sessions(user_id)
        .await
        .map_err(|error| FrameworkError::internal(format!("list Magnetar sessions: {error}")))
}

#[cfg(any(
    feature = "database-sqlite",
    feature = "database-postgres",
    feature = "database-mysql"
))]
pub(crate) async fn record_failed_attempt(
    email: &str,
    ip_address: Option<&str>,
) -> Result<LockoutStatus, FrameworkError> {
    let engine = password_engine()?;
    engine
        .record_failed_attempt(email, ip_address)
        .await
        .map_err(|error| FrameworkError::internal(format!("record failed attempt: {error}")))
}

#[cfg(any(
    feature = "database-sqlite",
    feature = "database-postgres",
    feature = "database-mysql"
))]
pub(crate) async fn admit_attempt(
    email: &str,
    context: Option<&str>,
) -> Result<engine::LockoutAdmission, FrameworkError> {
    let engine = password_engine()?;
    engine
        .admit_attempt(email, context)
        .await
        .map_err(|error| FrameworkError::internal(format!("admit authentication attempt: {error}")))
}

#[cfg(any(
    feature = "database-sqlite",
    feature = "database-postgres",
    feature = "database-mysql"
))]
pub(crate) async fn cancel_attempt(
    email: &str,
    admission: &engine::LockoutAdmission,
) -> Result<(), FrameworkError> {
    let engine = password_engine()?;
    engine
        .cancel_attempt(email, admission)
        .await
        .map_err(|error| {
            FrameworkError::internal(format!("cancel authentication attempt: {error}"))
        })
}

#[cfg(any(
    feature = "database-sqlite",
    feature = "database-postgres",
    feature = "database-mysql"
))]
pub(crate) async fn finalize_failed_attempt(
    email: &str,
    admission: &engine::LockoutAdmission,
) -> Result<engine::LockoutFinalization, FrameworkError> {
    let engine = password_engine()?;
    engine
        .finalize_failed_attempt(email, admission)
        .await
        .map_err(|error| {
            FrameworkError::internal(format!("finalize authentication attempt: {error}"))
        })
}

#[cfg(any(
    feature = "database-sqlite",
    feature = "database-postgres",
    feature = "database-mysql"
))]
pub(crate) async fn lockout_status(email: &str) -> Result<LockoutStatus, FrameworkError> {
    let engine = password_engine()?;
    engine
        .lockout_status(email)
        .await
        .map_err(|error| FrameworkError::internal(format!("read lockout status: {error}")))
}

#[cfg(any(
    feature = "database-sqlite",
    feature = "database-postgres",
    feature = "database-mysql"
))]
pub(crate) async fn reset_attempts(email: &str) -> Result<(), FrameworkError> {
    let engine = password_engine()?;
    engine
        .reset_attempts(email)
        .await
        .map_err(|error| FrameworkError::internal(format!("reset failed attempts: {error}")))
}

#[cfg(any(
    feature = "database-sqlite",
    feature = "database-postgres",
    feature = "database-mysql"
))]
pub(crate) async fn reset_admitted_attempts(
    email: &str,
    admission: &engine::LockoutAdmission,
) -> Result<(), FrameworkError> {
    let engine = password_engine()?;
    engine
        .reset_admitted_attempts(email, admission)
        .await
        .map_err(|error| {
            FrameworkError::internal(format!("reset admitted authentication attempt: {error}"))
        })
}

#[cfg(any(
    feature = "database-sqlite",
    feature = "database-postgres",
    feature = "database-mysql"
))]
pub(crate) async fn unlock_account(email: &str) -> Result<bool, FrameworkError> {
    let engine = password_engine()?;
    engine
        .unlock_account(email)
        .await
        .map_err(|error| FrameworkError::internal(format!("unlock account: {error}")))
}

/// Look up a Suprnova [`User`] by its opaque application identifier.
///
/// # Errors
///
/// Returns an error when the Magnetar engine is not installed or the host user
/// adapter fails.
pub async fn find_user_by_id(user_id: &str) -> Result<Option<User>, FrameworkError> {
    let engine = password_engine()?;
    engine
        .user_by_id(user_id)
        .await
        .map_err(|error| FrameworkError::internal(format!("Magnetar user lookup: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    struct PendingCleanupAuthority;

    #[async_trait]
    impl engine::MagnetarFactorAuthEngine for PendingCleanupAuthority {
        async fn complete_challenge(
            &self,
            _: &str,
            _: &str,
        ) -> magnetar::Result<engine::MagnetarIssuedSession> {
            std::future::pending().await
        }

        async fn user_by_id(&self, _: &str) -> magnetar::Result<Option<User>> {
            Ok(None)
        }

        async fn resolve_web_binding(
            &self,
            _: &magnetar::sessions::WebSessionBinding,
        ) -> magnetar::Result<magnetar::sessions::VerifiedSession> {
            std::future::pending().await
        }

        async fn bearer_user_id(&self, _: &str) -> magnetar::Result<Option<String>> {
            Ok(None)
        }

        async fn revoke_session(&self, _: &str) -> magnetar::Result<bool> {
            std::future::pending().await
        }

        async fn revoke_all_sessions(&self, _: &str) -> magnetar::Result<u64> {
            Ok(0)
        }

        async fn list_sessions(
            &self,
            _: &str,
        ) -> magnetar::Result<Vec<magnetar::sessions::SessionSummary>> {
            Ok(Vec::new())
        }
    }

    fn issued_session() -> engine::MagnetarIssuedSession {
        engine::MagnetarIssuedSession {
            session_id: "fresh-magnetar-session".to_owned(),
            web_binding: magnetar::sessions::WebSessionBinding {
                session_id: "fresh-magnetar-session".to_owned(),
                token_digest: [7; 32],
            },
            session: Session::builder()
                .token(SessionToken::new("fresh-bearer"))
                .user_id(UserId::new("new-user"))
                .build()
                .expect("build issued framework session"),
        }
    }

    #[tokio::test]
    async fn fresh_session_bind_queues_verified_retry_carrier_retirement() {
        let session_slot = crate::session::new_session_slot_for_test();
        let pending_cookies = crate::session::new_pending_cookies_slot_for_test();
        let pending_revocations = Arc::new(Mutex::new(Vec::new()));
        let issued = issued_session();

        crate::session::session_scope_for_test(
            session_slot,
            crate::session::pending_cookies_scope_for_test(
                pending_cookies,
                crate::auth::request_state::request_state_scope_for_test(
                    crate::session::middleware::PENDING_REMEMBER_REVOCATIONS.scope(
                        pending_revocations.clone(),
                        async {
                            crate::auth::request_state::set_verified_active_remember_carrier(
                                "web",
                                "remembered-user",
                                "rotated-selector",
                            );

                            bind_issued_session(&issued, true)
                                .expect("scoped fresh session bind succeeds");
                        },
                    ),
                ),
            ),
        )
        .await;

        assert_eq!(
            *pending_revocations.lock().unwrap(),
            vec![(
                "web".to_owned(),
                "remembered-user".to_owned(),
                "rotated-selector".to_owned(),
            )],
            "a fresh identity bind must retain exact cleanup for the replaced retry carrier",
        );
    }

    #[tokio::test]
    async fn missing_bind_scopes_preserve_retry_provenance_and_request_identity() {
        let issued = issued_session();
        crate::auth::request_state::request_state_scope_for_test(async {
            crate::auth::request_state::set_verified_active_remember_carrier(
                "web",
                "remembered-user",
                "rotated-selector",
            );

            assert!(
                bind_issued_session(&issued, true).is_err(),
                "missing bind scopes must fail before publishing identity",
            );

            assert_eq!(
                crate::auth::request_state::verified_active_remember_carrier_for_guard("web"),
                Some(("remembered-user".to_owned(), "rotated-selector".to_owned(),)),
                "missing bind scopes must not discard retry provenance",
            );
            assert_eq!(
                crate::auth::Auth::id(),
                None,
                "missing bind scopes must not publish the newly issued identity",
            );
        })
        .await;
    }

    #[tokio::test]
    async fn headless_preflight_succeeds_without_an_active_remember_carrier() {
        assert!(
            bind_scope_preflight().is_ok(),
            "headless flows without a remember carrier do not need request scopes",
        );
    }

    #[test]
    fn dropping_handoff_owner_without_a_runtime_does_not_panic() {
        let authority: Arc<dyn engine::MagnetarFactorAuthEngine> =
            Arc::new(PendingCleanupAuthority);
        drop(IssuedSessionHandoffOwner::new(
            authority,
            "headless-session".to_owned(),
            None,
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn issued_session_cleanup_timeout_is_bounded() {
        let authority: Arc<dyn engine::MagnetarFactorAuthEngine> =
            Arc::new(PendingCleanupAuthority);
        let result = spawn_issued_session_batch_cleanup(
            authority,
            None,
            vec!["headless-session".to_owned()],
            1,
            "test timeout",
        )
        .expect("test runtime is installed")
        .await
        .expect("cleanup task completes");
        assert!(matches!(result, HandoffCleanupOutcome::TimedOut));
    }

    #[test]
    fn reserved_engine_lookup_diagnostic_names_only_async_initializers() {
        assert!(ensure_engine_installation_ready(false).is_ok());

        let error = ensure_engine_installation_ready(true)
            .expect_err("a reserved installation must reject request handling");
        assert_eq!(
            error.to_string(),
            "Internal server error: Magnetar authentication subsystem initialization is in progress; finish application bootstrap before serving requests by awaiting init_magnetar(...) or init_magnetar_oauth_only(...), whichever was started",
        );
    }
}
