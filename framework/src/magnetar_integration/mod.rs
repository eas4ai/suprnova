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
pub mod passkey;
pub mod password;

use std::sync::{Arc, Mutex, OnceLock};

pub mod ceremony;

use crate::error::FrameworkError;

pub use crate::auth::{LockoutStatus, Session, SessionToken, User, UserId};
pub use default_engine::{MagnetarConfig, init_magnetar};
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

/// Atomically install password/session and passkey adapters from one host
/// engine bundle. Neither adapter is visible until both are ready.
pub fn install_magnetar_engines(
    password: Arc<dyn engine::MagnetarPasswordAuthEngine>,
    passkey: Arc<dyn engine::MagnetarPasskeyAuthEngine>,
) -> Result<(), FrameworkError> {
    reserve_magnetar_engines()?.install(password, passkey)
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
    ) -> Result<(), FrameworkError> {
        let mut guard = engine_install_guard()?;
        if !self.active || !guard.reserved {
            return Err(FrameworkError::internal(
                "Magnetar engine installation reservation is not active",
            ));
        }
        if MAGNETAR_PASSWORD_ENGINE.get().is_some() || MAGNETAR_PASSKEY_ENGINE.get().is_some() {
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

#[cfg(any(
    feature = "database-sqlite",
    feature = "database-postgres",
    feature = "database-mysql"
))]
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
/// Install the real Magnetar OAuth adapter used only for its configured
/// provider registry. Replacing an adapter is rejected.
#[cfg(feature = "magnetar-oauth")]
pub fn install_magnetar_oauth_engine(
    engine: Arc<dyn engine::MagnetarOAuthAuthEngine>,
) -> Result<(), FrameworkError> {
    MAGNETAR_OAUTH_ENGINE
        .set(engine)
        .map_err(|_| FrameworkError::internal("Magnetar OAuth engine is already installed"))
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
