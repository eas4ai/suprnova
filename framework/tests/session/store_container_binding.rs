//! SEC-02(b) regression: constructing a `SessionMiddleware` must publish
//! the store it was built with into the application container as
//! `dyn SessionStore`, so `session::destroy_all_for_user` - and, through
//! it, `PasswordReset::complete` and any future forced-logout / 2FA-
//! disable hook - resolves the store the app actually configured
//! instead of silently constructing an unrelated fresh
//! `DatabaseSessionDriver` that revokes nothing on a custom-store app.
//!
//! Kept in its own test binary (its own OS process): the registration
//! writes to the process-global container via `App::bind_if_absent`
//! (see `session::middleware::register_configured_store`), and this is
//! the one test in the whole suite that deliberately exercises that
//! real global write end-to-end. Sharing a process with any other test
//! that also constructs a `SessionMiddleware` would race for that
//! global slot - every other session test that cares about a specific
//! store instead overrides hermetically via `TestContainer`.

use async_trait::async_trait;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use suprnova::FrameworkError;
use suprnova::session::{SessionConfig, SessionData, SessionMiddleware, SessionStore};

/// Records the last user id it was asked to revoke and returns a
/// sentinel row count no real `DatabaseSessionDriver` could ever
/// produce against an empty/nonexistent table - that sentinel is what
/// proves `destroy_all_for_user` reached THIS store rather than falling
/// back to a fresh default driver.
struct RecordingStore {
    revoked_user_id: std::sync::Mutex<Option<String>>,
    revoke_calls: AtomicU64,
}

#[async_trait]
impl SessionStore for RecordingStore {
    async fn read(&self, _id: &str) -> Result<Option<SessionData>, FrameworkError> {
        Ok(None)
    }
    async fn write(&self, _session: &SessionData) -> Result<(), FrameworkError> {
        Ok(())
    }
    async fn destroy(&self, _id: &str) -> Result<(), FrameworkError> {
        Ok(())
    }
    async fn destroy_for_user(&self, user_id: &str) -> Result<u64, FrameworkError> {
        *self.revoked_user_id.lock().unwrap() = Some(user_id.to_string());
        self.revoke_calls.fetch_add(1, Ordering::SeqCst);
        // Sentinel value: a real DatabaseSessionDriver against no
        // matching rows returns 0, and one hitting an uninitialised /
        // wrong DB errors outright - neither path can produce 4242.
        Ok(4242)
    }
    async fn gc(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }
}

#[tokio::test]
async fn session_middleware_registers_its_store_for_revocation_resolution() {
    let recording = Arc::new(RecordingStore {
        revoked_user_id: std::sync::Mutex::new(None),
        revoke_calls: AtomicU64::new(0),
    });

    // Constructing the middleware with a custom store must publish it
    // into the container - no separate `App::bind` call required from
    // application bootstrap code.
    let _middleware = SessionMiddleware::with_store(SessionConfig::default(), recording.clone());

    let revoked = suprnova::session::destroy_all_for_user("sec02b-target-user")
        .await
        .expect("destroy_all_for_user must succeed");

    assert_eq!(
        revoked, 4242,
        "destroy_all_for_user must return the CONFIGURED store's result, \
         not a fresh DatabaseSessionDriver's (which cannot produce 4242)"
    );
    assert_eq!(
        recording.revoke_calls.load(Ordering::SeqCst),
        1,
        "the configured store's destroy_for_user must have been called exactly once"
    );
    assert_eq!(
        recording.revoked_user_id.lock().unwrap().as_deref(),
        Some("sec02b-target-user"),
        "the configured store must have been asked to revoke the right user id"
    );
}
