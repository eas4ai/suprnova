//! The session guard - Laravel's `SessionGuard`.
//!
//! Resolves and persists authentication through the session
//! (`crate::session`) and the remember-me token table
//! (`crate::auth::remember`). It is the implementation behind the
//! default `web` guard and the sugar that the static [`Auth`] facade
//! delegates to.
//!
//! The guard owns no per-request state itself (it is a container
//! singleton). The guard-keyed "who is authenticated this request" cache and
//! via-remember provenance live in [`crate::auth::request_state`], scoped once
//! per request. Persisted identities are likewise keyed by the complete guard
//! name; only the configured default guard mirrors the generic [`Auth`] view.

use std::sync::Arc;

use async_trait::async_trait;

use super::authenticatable::Authenticatable;
use super::contract::{Credentials, Guard, StatefulGuard};
use super::guard::Auth;
use super::provider::UserProvider;
use super::{events, request_state};
use crate::error::FrameworkError;
use crate::events::EventFacade;

/// Session-backed authentication guard.
///
/// Construct one with a [`UserProvider`]; the manager wires it up under
/// a name. Most apps reach it through the static [`Auth`] facade rather
/// than constructing it directly.
///
/// ```rust,no_run
/// use suprnova::{SessionGuard, StatefulGuard, Credentials};
/// use std::sync::Arc;
/// # use suprnova::DatabaseUserProvider;
/// # async fn ex() -> Result<(), Box<dyn std::error::Error>> {
/// # let my_provider = DatabaseUserProvider::new("users");
/// let guard = SessionGuard::new(Arc::new(my_provider));
/// let user = guard
///     .attempt(&Credentials::password("alice@example.com", "s3cret"), true)
///     .await?;
/// # Ok(()) }
/// ```
pub struct SessionGuard {
    /// The guard's name (e.g. `"web"`), carried on dispatched events.
    name: String,
    /// The user provider this guard resolves and validates against.
    provider: Arc<dyn UserProvider>,
    /// Remember-me token + cookie lifetime in minutes.
    remember_ttl_minutes: i64,
}

impl SessionGuard {
    /// Create a session guard named `"web"` with the given provider, using
    /// the environment's remember-me lifetime (`REMEMBER_LIFETIME`, default
    /// 30 days).
    pub fn new(provider: Arc<dyn UserProvider>) -> Self {
        Self::named("web", provider)
    }

    /// Create a session guard with an explicit name.
    ///
    /// The complete name keys this guard's persisted identity, request cache,
    /// remember provenance, and lifecycle events. Only the configured default
    /// guard mirrors the generic [`Auth`] facade identity.
    pub fn named(name: impl Into<String>, provider: Arc<dyn UserProvider>) -> Self {
        let remember_ttl_minutes = i64::try_from(
            crate::session::SessionConfig::from_env()
                .remember_lifetime
                .as_secs()
                / 60,
        )
        .unwrap_or(i64::MAX);
        Self {
            name: name.into(),
            provider,
            remember_ttl_minutes,
        }
    }

    /// Override the remember-me token/cookie lifetime (minutes).
    pub fn with_remember_ttl(mut self, minutes: i64) -> Self {
        self.remember_ttl_minutes = minutes;
        self
    }
}

#[async_trait]
impl Guard for SessionGuard {
    async fn user(&self) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        // Per-request cache: a prior resolution, or a `once`/`set_user`.
        if let Some(user) = request_state::guard_user(&self.name) {
            return Ok(Some(user));
        }

        let id = match crate::session::middleware::guard_auth_user_id(&self.name) {
            Some(id) => id,
            None => return Ok(None),
        };

        let user = self.provider.retrieve_by_id(&id).await?;
        if let Some(user) = &user {
            request_state::set_guard_user(&self.name, user.clone());
        }
        Ok(user)
    }

    async fn id(&self) -> Result<Option<String>, FrameworkError> {
        Ok(crate::session::middleware::guard_auth_user_id(&self.name))
    }

    async fn validate(&self, credentials: &Credentials) -> Result<bool, FrameworkError> {
        let creds = credentials.as_value();
        match self.provider.retrieve_by_credentials(&creds).await? {
            Some(user) => self.provider.validate_credentials(&*user, &creds).await,
            None => Ok(false),
        }
    }

    async fn set_user(&self, user: Arc<dyn Authenticatable>) {
        request_state::set_guard_user(&self.name, user);
    }

    async fn has_user(&self) -> bool {
        request_state::has_guard_user(&self.name)
    }
}

#[async_trait]
impl StatefulGuard for SessionGuard {
    async fn attempt(
        &self,
        credentials: &Credentials,
        remember: bool,
    ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        EventFacade::dispatch(events::Attempting {
            guard: self.name.clone(),
            remember,
        })
        .await?;

        let creds = credentials.as_value();
        if let Some(user) = self.provider.retrieve_by_credentials(&creds).await? {
            if self.provider.validate_credentials(&*user, &creds).await? {
                // login() fires Login + Authenticated.
                self.login(user.clone(), remember).await?;
                // The caller just proved the password, so stamp the
                // confirmation window. Without this, reauth-gated actions
                // (passkey enrollment against an existing account - see
                // SEC-01 in `magnetar_integration::passkey`) would demand a
                // password confirmation that nothing in the framework ever
                // produces, making them unsatisfiable rather than merely
                // guarded.
                //
                // Deliberately stamped here and not in `login`: `login` is
                // also reached via `login_using_id` and impersonation flows,
                // where no password was presented and a confirmation stamp
                // would be a lie. Only the credential-verified path earns it.
                //
                // After `login`, not before: `login` regenerates the session
                // id to defeat fixation, and the stamp must land on the
                // session the caller ends up holding. The stamp is the legacy
                // generic/default-guard confirmation gate, so a named guard's
                // password proof must never authorize it.
                if self.name == Auth::default_guard_name() {
                    crate::session::session_mut(|s| s.password_confirmed());
                }
                return Ok(Some(user));
            }
            // Identifier matched, credentials did not.
            EventFacade::dispatch(events::Failed {
                guard: self.name.clone(),
                user_id: Some(user.get_auth_identifier()),
            })
            .await?;
            return Ok(None);
        }

        // No user matched the supplied credentials. Drive a dummy
        // password-verify so the unknown-identifier wall-clock
        // matches the known-identifier-wrong-password wall-clock -
        // otherwise the difference (cheap DB-miss vs full bcrypt
        // cost) is a side-channel that lets an attacker probe the
        // user database without ever triggering the brute-force
        // lockout (which only counts attempts against KNOWN
        // accounts).
        let _ = self.provider.dummy_verify().await;
        EventFacade::dispatch(events::Failed {
            guard: self.name.clone(),
            user_id: None,
        })
        .await?;
        Ok(None)
    }

    async fn once(&self, credentials: &Credentials) -> Result<bool, FrameworkError> {
        EventFacade::dispatch(events::Attempting {
            guard: self.name.clone(),
            remember: false,
        })
        .await?;

        let creds = credentials.as_value();
        if let Some(user) = self.provider.retrieve_by_credentials(&creds).await? {
            if self.provider.validate_credentials(&*user, &creds).await? {
                let user_id = user.get_auth_identifier();
                request_state::set_guard_user(&self.name, user);
                request_state::set_guard_via_remember(&self.name, false);
                EventFacade::dispatch(events::Authenticated {
                    guard: self.name.clone(),
                    user_id,
                })
                .await?;
                return Ok(true);
            }
            EventFacade::dispatch(events::Failed {
                guard: self.name.clone(),
                user_id: Some(user.get_auth_identifier()),
            })
            .await?;
            return Ok(false);
        }

        // No user matched - drive dummy_verify to equalise timing
        // against the wrong-password branch above. See `attempt` for
        // the full rationale.
        let _ = self.provider.dummy_verify().await;
        EventFacade::dispatch(events::Failed {
            guard: self.name.clone(),
            user_id: None,
        })
        .await?;
        Ok(false)
    }

    async fn login(
        &self,
        user: Arc<dyn Authenticatable>,
        remember: bool,
    ) -> Result<(), FrameworkError> {
        let user_id = user.get_auth_identifier();
        Auth::flush_pending_remember_revocations().await?;
        let remember_to_revoke = Auth::prepare_guard_remember_identity_replacement(&self.name);

        if let Some((previous_user_id, selector)) = remember_to_revoke {
            Auth::revoke_remember_selector(&self.name, &previous_user_id, &selector).await?;
        }

        // Delegate session persistence (+ remember-me row/cookie) to the
        // proven facade helpers: both regenerate the session id and CSRF
        // token to defeat session fixation.
        if remember {
            Auth::login_guard_id(&self.name, user_id.clone())?;
            Auth::issue_remember_cookie_for_guard(&self.name, &user_id, self.remember_ttl_minutes)
                .await?;
        } else {
            Auth::login_guard_id(&self.name, user_id.clone())?;
        }

        // Cache the resolved user for the rest of the request.
        request_state::set_guard_user(&self.name, user);
        request_state::set_guard_via_remember(&self.name, false);

        EventFacade::dispatch(events::Login {
            guard: self.name.clone(),
            user_id: user_id.clone(),
            remember,
        })
        .await?;
        EventFacade::dispatch(events::Authenticated {
            guard: self.name.clone(),
            user_id,
        })
        .await?;
        Ok(())
    }

    async fn login_using_id(
        &self,
        id: &str,
        remember: bool,
    ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        match self.provider.retrieve_by_id(id).await? {
            Some(user) => {
                self.login(user.clone(), remember).await?;
                Ok(Some(user))
            }
            None => Ok(None),
        }
    }

    async fn once_using_id(
        &self,
        id: &str,
    ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        match self.provider.retrieve_by_id(id).await? {
            Some(user) => {
                let user_id = user.get_auth_identifier();
                request_state::set_guard_user(&self.name, user.clone());
                request_state::set_guard_via_remember(&self.name, false);
                EventFacade::dispatch(events::Authenticated {
                    guard: self.name.clone(),
                    user_id,
                })
                .await?;
                Ok(Some(user))
            }
            None => Ok(None),
        }
    }

    fn via_remember(&self) -> bool {
        request_state::guard_via_remember(&self.name)
    }

    async fn logout(&self) -> Result<(), FrameworkError> {
        // Capture the id before clearing so the Logout event is attributed.
        let user_id = self.id().await?;

        // Tear down session + remember-me + request-scoped user. We call the
        // event-free primitive rather than `Auth::logout` so the Logout event
        // is dispatched exactly once, here, attributed to *this* guard's name.
        Auth::clear_guard_authentication(&self.name).await?;

        EventFacade::dispatch(events::Logout {
            guard: self.name.clone(),
            user_id,
        })
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::Any;

    use crate::auth::request_state;
    use crate::session::{new_session_slot_for_test, session, session_mut, session_scope_for_test};

    #[derive(Clone)]
    struct TestUser {
        id: String,
    }

    impl Authenticatable for TestUser {
        fn get_auth_identifier(&self) -> String {
            self.id.clone()
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
        fn into_arc_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
            self
        }
    }

    /// A provider that knows one user: id `"7"`, email `"a@b.com"`,
    /// password `"secret"`.
    struct FakeProvider;

    /// A provider with exactly one fixed identity. Named-guard isolation
    /// tests use different instances so a guard can never accidentally
    /// resolve the other guard's principal through its provider.
    struct FixedProvider {
        id: &'static str,
    }

    fn the_user() -> Arc<dyn Authenticatable> {
        Arc::new(TestUser { id: "7".into() })
    }

    #[async_trait]
    impl UserProvider for FakeProvider {
        async fn retrieve_by_id(
            &self,
            id: &str,
        ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
            Ok((id == "7").then(the_user))
        }

        async fn retrieve_by_credentials(
            &self,
            credentials: &serde_json::Value,
        ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
            let email = credentials.get("email").and_then(|v| v.as_str());
            Ok((email == Some("a@b.com")).then(the_user))
        }

        async fn validate_credentials(
            &self,
            _user: &dyn Authenticatable,
            credentials: &serde_json::Value,
        ) -> Result<bool, FrameworkError> {
            Ok(credentials.get("password").and_then(|v| v.as_str()) == Some("secret"))
        }
    }

    #[async_trait]
    impl UserProvider for FixedProvider {
        async fn retrieve_by_id(
            &self,
            id: &str,
        ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
            Ok((id == self.id).then(|| {
                Arc::new(TestUser {
                    id: self.id.to_string(),
                }) as Arc<dyn Authenticatable>
            }))
        }

        async fn retrieve_by_credentials(
            &self,
            _credentials: &serde_json::Value,
        ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
            Ok(None)
        }

        async fn validate_credentials(
            &self,
            _user: &dyn Authenticatable,
            _credentials: &serde_json::Value,
        ) -> Result<bool, FrameworkError> {
            Ok(false)
        }
    }

    fn guard() -> SessionGuard {
        SessionGuard::new(Arc::new(FakeProvider))
    }

    /// Run `fut` inside both a fresh session scope and a fresh auth
    /// request-state scope - the two task-locals `SessionGuard` reads and
    /// writes at runtime.
    async fn with_scopes<F: std::future::Future>(fut: F) -> F::Output {
        let slot = new_session_slot_for_test();
        session_scope_for_test(slot, request_state::scope(fut)).await
    }

    #[tokio::test]
    async fn guest_when_no_session_user() {
        with_scopes(async {
            let g = guard();
            assert_eq!(g.id().await.unwrap(), None);
            assert!(!g.check().await.unwrap());
            assert!(g.guest().await.unwrap());
            assert!(g.user().await.unwrap().is_none());
            assert!(!g.has_user().await);
            assert!(!g.via_remember());
        })
        .await;
    }

    #[tokio::test]
    async fn login_persists_to_session_and_caches_user() {
        with_scopes(async {
            let g = guard();
            g.login(the_user(), false).await.unwrap();

            // Persisted to the session.
            assert_eq!(session().unwrap().user_id, Some("7".to_string()));
            // Visible through the guard.
            assert_eq!(g.id().await.unwrap(), Some("7".to_string()));
            assert!(g.check().await.unwrap());
            assert!(g.has_user().await);
            // user() returns the cached instance.
            let u = g.user().await.unwrap().expect("user resolved");
            assert_eq!(u.get_auth_identifier(), "7");
        })
        .await;
    }

    #[tokio::test]
    async fn named_session_guards_keep_independent_identities() {
        with_scopes(async {
            let web = SessionGuard::named("web", Arc::new(FixedProvider { id: "7" }));
            let admin = SessionGuard::named("admin", Arc::new(FixedProvider { id: "9" }));

            web.login(
                Arc::new(TestUser { id: "7".into() }) as Arc<dyn Authenticatable>,
                false,
            )
            .await
            .unwrap();

            assert_eq!(web.id().await.unwrap().as_deref(), Some("7"));
            assert_eq!(admin.id().await.unwrap(), None);

            admin
                .login(
                    Arc::new(TestUser { id: "9".into() }) as Arc<dyn Authenticatable>,
                    false,
                )
                .await
                .unwrap();

            assert_eq!(web.id().await.unwrap().as_deref(), Some("7"));
            assert_eq!(admin.id().await.unwrap().as_deref(), Some("9"));
            assert_eq!(Auth::id().as_deref(), Some("7"));
        })
        .await;
    }

    #[tokio::test]
    async fn fresh_logins_discard_stale_guard_remember_state() {
        with_scopes(async {
            let stale_binding = magnetar::sessions::WebSessionBinding {
                session_id: "stale-magnetar-session".to_owned(),
                token_digest: [7; 32],
            };
            session_mut(|session| {
                session.set_auth_guard_id("admin", "7");
                session.set_auth_guard_remember_selector("admin", "stale-admin-selector");
                session.set_auth_guard_magnetar_binding("admin", stale_binding.clone());

                session.set_auth_guard_id("web", "7");
                session.set_auth_guard_remember_selector("web", "stale-web-selector");
                session.set_auth_guard_magnetar_binding("web", stale_binding.clone());
                session.user_id = Some("7".to_owned());
                session.set_magnetar_web_binding(stale_binding);
            });

            let admin = SessionGuard::named("admin", Arc::new(FixedProvider { id: "9" }));
            admin
                .login(
                    Arc::new(TestUser { id: "9".into() }) as Arc<dyn Authenticatable>,
                    false,
                )
                .await
                .unwrap();

            let web = SessionGuard::named("web", Arc::new(FixedProvider { id: "7" }));
            web.login(
                Arc::new(TestUser { id: "7".into() }) as Arc<dyn Authenticatable>,
                false,
            )
            .await
            .unwrap();

            let session = session().expect("fresh logins retain the data session");
            assert_eq!(session.auth_guard_id("admin").as_deref(), Some("9"));
            assert_eq!(session.auth_guard_remember_selector("admin"), None);
            assert_eq!(session.auth_guard_magnetar_binding("admin"), None);
            assert_eq!(session.auth_guard_id("web").as_deref(), Some("7"));
            assert_eq!(session.auth_guard_remember_selector("web"), None);
            assert_eq!(session.auth_guard_magnetar_binding("web"), None);
            assert_eq!(session.magnetar_web_binding(), None);
        })
        .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn magnetar_binding_reconciles_default_guard_request_state() {
        use sea_orm::EntityTrait;

        let database = crate::testing::TestDatabase::sqlite_memory().await.unwrap();
        database
            .execute_unprepared(
                "CREATE TABLE remember_tokens (\
                    id INTEGER PRIMARY KEY AUTOINCREMENT, \
                    user_id TEXT NOT NULL, \
                    selector TEXT NOT NULL, \
                    token_hash TEXT NOT NULL, \
                    expires_at TIMESTAMP NOT NULL, \
                    created_at TIMESTAMP NOT NULL, \
                    last_used_at TIMESTAMP NULL\
                )",
            )
            .await
            .unwrap();
        with_scopes(async {
            request_state::set_guard_user(
                "web",
                Arc::new(TestUser { id: "7".into() }) as Arc<dyn Authenticatable>,
            );
            request_state::set_guard_via_remember("web", true);
            session_mut(|session| {
                session.set_auth_guard_id("web", "7");
                session.set_auth_guard_remember_selector("web", "stale-selector");
                session.user_id = Some("7".to_owned());
            });
            let issued = crate::magnetar_integration::engine::MagnetarIssuedSession {
                session_id: "magnetar-session-for-9".to_owned(),
                web_binding: magnetar::sessions::WebSessionBinding {
                    session_id: "magnetar-session-for-9".to_owned(),
                    token_digest: [9; 32],
                },
                session: crate::auth::Session::builder()
                    .user_id(crate::auth::UserId::new("9"))
                    .build()
                    .unwrap(),
            };

            crate::magnetar_integration::bind_issued_session(&issued, true);

            assert_eq!(Auth::id().as_deref(), Some("9"));
            let web = SessionGuard::named("web", Arc::new(FixedProvider { id: "9" }));
            assert_eq!(
                web.user()
                    .await
                    .unwrap()
                    .map(|user| user.get_auth_identifier())
                    .as_deref(),
                Some("9")
            );
            assert!(!web.via_remember());
            let session = session().expect("Magnetar binding retains the data session");
            assert_eq!(session.auth_guard_id("web").as_deref(), Some("9"));
            assert_eq!(session.user_id.as_deref(), Some("9"));
            assert_eq!(session.auth_guard_remember_selector("web"), None);

            database
                .execute_unprepared(
                    "INSERT INTO remember_tokens \
                        (user_id, selector, token_hash, expires_at, created_at, last_used_at) \
                     VALUES \
                        ('7', 'selector-for-7', 'sha256:seven', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP, NULL), \
                        ('9', 'selector-for-9', 'sha256:nine', CURRENT_TIMESTAMP, CURRENT_TIMESTAMP, NULL)",
                )
                .await
                .unwrap();
            web.logout().await.unwrap();
            let remaining = crate::auth::remember::entity::Entity::find()
                .all(database.conn())
                .await
                .unwrap();
            assert_eq!(
                remaining
                    .iter()
                    .map(|row| row.user_id.as_str())
                    .collect::<Vec<_>>(),
                vec!["7"],
                "logout must revoke issued user 9, not stale cached user 7"
            );
        })
        .await;
    }

    // A login→logout round-trip exercises remember-me revocation, which
    // needs a database; that path (and the lifecycle events) is covered by
    // the `tests/auth_session_guard.rs` integration test. Here we only
    // assert the DB-free guarantee: logging out when nobody is logged in is
    // safe and idempotent (no DB call, no panic).
    #[tokio::test]
    async fn logout_when_not_logged_in_is_safe() {
        with_scopes(async {
            let g = guard();
            g.logout().await.unwrap();
            assert!(g.guest().await.unwrap());
            assert!(!g.has_user().await);
        })
        .await;
    }

    #[tokio::test]
    async fn attempt_with_valid_credentials_logs_in() {
        with_scopes(async {
            let g = guard();
            let user = g
                .attempt(&Credentials::password("a@b.com", "secret"), false)
                .await
                .unwrap();
            assert_eq!(user.map(|u| u.get_auth_identifier()), Some("7".to_string()));
            assert_eq!(session().unwrap().user_id, Some("7".to_string()));
            assert!(g.check().await.unwrap());
        })
        .await;
    }

    #[tokio::test]
    async fn attempt_with_wrong_password_does_not_log_in() {
        with_scopes(async {
            let g = guard();
            let user = g
                .attempt(&Credentials::password("a@b.com", "wrong"), false)
                .await
                .unwrap();
            assert!(user.is_none());
            assert_eq!(session().unwrap().user_id, None);
            assert!(g.guest().await.unwrap());
        })
        .await;
    }

    #[tokio::test]
    async fn attempt_with_unknown_user_does_not_log_in() {
        with_scopes(async {
            let g = guard();
            let user = g
                .attempt(&Credentials::password("nobody@b.com", "secret"), false)
                .await
                .unwrap();
            assert!(user.is_none());
            assert!(g.guest().await.unwrap());
        })
        .await;
    }

    #[tokio::test]
    async fn validate_checks_credentials_without_logging_in() {
        with_scopes(async {
            let g = guard();
            assert!(
                g.validate(&Credentials::password("a@b.com", "secret"))
                    .await
                    .unwrap()
            );
            assert!(
                !g.validate(&Credentials::password("a@b.com", "wrong"))
                    .await
                    .unwrap()
            );
            // validate never authenticates.
            assert!(g.guest().await.unwrap());
            assert_eq!(session().unwrap().user_id, None);
        })
        .await;
    }

    #[tokio::test]
    async fn once_authenticates_without_persisting() {
        with_scopes(async {
            let g = guard();
            assert!(
                g.once(&Credentials::password("a@b.com", "secret"))
                    .await
                    .unwrap()
            );
            // Authenticated this request...
            assert!(g.check().await.unwrap());
            assert_eq!(g.id().await.unwrap(), Some("7".to_string()));
            assert!(g.has_user().await);
            // ...but never written to the session.
            assert_eq!(session().unwrap().user_id, None);
        })
        .await;
    }

    #[tokio::test]
    async fn once_with_bad_credentials_returns_false() {
        with_scopes(async {
            let g = guard();
            assert!(
                !g.once(&Credentials::password("a@b.com", "wrong"))
                    .await
                    .unwrap()
            );
            assert!(g.guest().await.unwrap());
        })
        .await;
    }

    #[tokio::test]
    async fn login_using_id_resolves_known_user() {
        with_scopes(async {
            let g = guard();
            let ok = g.login_using_id("7", false).await.unwrap();
            assert_eq!(ok.map(|u| u.get_auth_identifier()), Some("7".to_string()));
            assert!(g.check().await.unwrap());
            assert_eq!(session().unwrap().user_id, Some("7".to_string()));
        })
        .await;
    }

    #[tokio::test]
    async fn login_using_id_with_unknown_id_does_not_log_in() {
        with_scopes(async {
            let g = guard();
            let missing = g.login_using_id("999", false).await.unwrap();
            assert!(missing.is_none());
            assert!(g.guest().await.unwrap());
            assert_eq!(session().unwrap().user_id, None);
        })
        .await;
    }

    #[tokio::test]
    async fn once_using_id_authenticates_without_persisting() {
        with_scopes(async {
            let g = guard();
            let user = g.once_using_id("7").await.unwrap();
            assert_eq!(user.map(|u| u.get_auth_identifier()), Some("7".to_string()));
            assert!(g.check().await.unwrap());
            assert_eq!(session().unwrap().user_id, None);

            assert!(g.once_using_id("999").await.unwrap().is_none());
        })
        .await;
    }

    #[tokio::test]
    async fn set_user_sets_request_user_without_persisting() {
        with_scopes(async {
            let g = guard();
            assert!(!g.has_user().await);
            g.set_user(the_user()).await;
            assert!(g.has_user().await);
            assert_eq!(g.id().await.unwrap(), Some("7".to_string()));
            // Not persisted.
            assert_eq!(session().unwrap().user_id, None);
        })
        .await;
    }
}
