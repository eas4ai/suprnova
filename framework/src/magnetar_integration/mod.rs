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
pub use sign_in::SignInOutcome;

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

#[cfg(test)]
mod tests {
    use super::*;

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
/// engine bundle. Neither adapter is visible until both are ready.
pub fn install_magnetar_engines(
    password: Arc<dyn engine::MagnetarPasswordAuthEngine>,
    passkey: Arc<dyn engine::MagnetarPasskeyAuthEngine>,
) -> Result<(), FrameworkError> {
    reserve_magnetar_engines()?.install(password, passkey, no_oauth_engine())
}

/// Test-only password adapter installation for isolated facade harnesses.
#[cfg(feature = "testing")]
#[doc(hidden)]
pub fn install_magnetar_password_engine_for_test(
    engine: Arc<dyn engine::MagnetarPasswordAuthEngine>,
) -> Result<(), FrameworkError> {
    let guard = engine_install_guard()?;
    if guard.reserved || MAGNETAR_PASSWORD_ENGINE.get().is_some() {
        return Err(FrameworkError::internal(
            "Magnetar password engine is already installed or reserved",
        ));
    }
    MAGNETAR_PASSWORD_ENGINE
        .set(engine)
        .map_err(|_| FrameworkError::internal("Magnetar password engine installation raced"))
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

pub(crate) struct EngineInstallReservation {
    active: bool,
}

impl EngineInstallReservation {
    pub(crate) fn install(
        mut self,
        password: Arc<dyn engine::MagnetarPasswordAuthEngine>,
        passkey: Arc<dyn engine::MagnetarPasskeyAuthEngine>,
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
    ) -> Result<(), FrameworkError> {
        let mut guard = engine_install_guard()?;
        if !self.active || !guard.reserved {
            return Err(FrameworkError::internal(
                "Magnetar engine installation reservation is not active",
            ));
        }
        if MAGNETAR_PASSWORD_ENGINE.get().is_some()
            || MAGNETAR_PASSKEY_ENGINE.get().is_some()
            || MAGNETAR_OAUTH_ENGINE.get().is_some()
        {
            return Err(FrameworkError::internal(
                "Magnetar authentication engines are already installed",
            ));
        }
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
        || oauth_engine_is_installed()
    {
        return Err(FrameworkError::internal(
            "Magnetar authentication engines are already installed or reserved",
        ));
    }
    guard.reserved = true;
    Ok(EngineInstallReservation { active: true })
}

pub(crate) fn password_engine()
-> Result<Arc<dyn engine::MagnetarPasswordAuthEngine>, FrameworkError> {
    let guard = engine_install_guard()?;
    if guard.reserved {
        return Err(FrameworkError::internal(
            "Magnetar engine installation is still in progress",
        ));
    }
    MAGNETAR_PASSWORD_ENGINE
        .get()
        .cloned()
        .ok_or_else(|| FrameworkError::internal("Magnetar engine is not installed"))
}

pub(crate) fn password_engine_if_installed()
-> Result<Option<Arc<dyn engine::MagnetarPasswordAuthEngine>>, FrameworkError> {
    let guard = engine_install_guard()?;
    if guard.reserved {
        return Err(FrameworkError::internal(
            "Magnetar engine installation is still in progress",
        ));
    }
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
    if guard.reserved {
        return Err(FrameworkError::internal(
            "Magnetar engine installation is still in progress",
        ));
    }
    MAGNETAR_PASSKEY_ENGINE
        .get()
        .cloned()
        .ok_or_else(|| FrameworkError::internal("Magnetar passkey engine is not installed"))
}

fn engine_install_guard()
-> Result<std::sync::MutexGuard<'static, EngineInstallState>, FrameworkError> {
    ENGINE_INSTALL_GUARD
        .lock()
        .map_err(|_| FrameworkError::internal("Magnetar engine install lock poisoned"))
}
/// Install an OAuth adapter assembled by a custom retained host engine.
///
/// Default-engine applications configure OAuth through
/// [`MagnetarConfig::oauth`] so password, passkey, and OAuth adapters publish
/// under one reservation. Replacing an adapter is rejected.
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

/// Revoke one active Magnetar session by its stable row identifier.
pub async fn revoke_session(session_id: &str) -> Result<bool, FrameworkError> {
    let engine = password_engine()?;
    engine
        .revoke_session(session_id)
        .await
        .map_err(|error| FrameworkError::internal(format!("revoke Magnetar session: {error}")))
}

/// Revoke every active Magnetar session for one application user.
pub async fn revoke_all_sessions(user_id: &str) -> Result<u64, FrameworkError> {
    let engine = password_engine()?;
    engine
        .revoke_all_sessions(user_id)
        .await
        .map_err(|error| FrameworkError::internal(format!("revoke Magnetar sessions: {error}")))
}

/// List active Magnetar sessions for one application user.
pub async fn list_sessions(
    user_id: &str,
) -> Result<Vec<magnetar::sessions::SessionSummary>, FrameworkError> {
    let engine = password_engine()?;
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
