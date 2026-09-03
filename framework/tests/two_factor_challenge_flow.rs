#![cfg(feature = "testing")]

//! End-to-end tests for the 2FA challenge promotion flow.
//!
//! Covers the path `Auth::password().register(...)` → `TwoFactor::enroll`
//! → `TwoFactor::confirm` → `TwoFactor::start_challenge(_, remember)` →
//! `TwoFactor::complete_challenge(valid_totp)` and asserts the contract
//! the framework promises for the final step:
//!
//! * the session id rotates (session fixation defence);
//! * the CSRF token rotates;
//! * the standard `Auth\Login` + `Auth\Authenticated` lifecycle events
//!   fire, in addition to the 2FA-specific `TwoFactor\Challenged`;
//! * a fresh remember-me cookie is queued when the original login form
//!   set `remember=true`, and **no** cookie is queued when it was
//!   `false`.
//!
//! Uses one runtime and a serialized event-fake critical section.

#[path = "common/magnetar_auth.rs"]
mod magnetar_auth;

use once_cell::sync::Lazy;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, EntityTrait, IntoActiveModel};
use sea_orm_migration::MigratorTrait;
use sea_orm_migration::prelude::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::runtime::Runtime;
use tokio::sync::Mutex;

use suprnova::auth::events::{Authenticated, Login};
use suprnova::auth_flows::events::{AccountLocked, TwoFactorChallengeFailed, TwoFactorChallenged};
use suprnova::auth_flows::two_factor::migration::Migration as TwoFactorMigration;
use suprnova::auth_flows::two_factor::migration_replay::Migration as TwoFactorReplayMigration;
use suprnova::auth_flows::{BruteForce, TwoFactor, TwoFactorUser};
use suprnova::events::testing::{assert_dispatched, assert_not_dispatched, dispatched_count};
use suprnova::http::cookie::Cookie;
use suprnova::middleware::{Middleware, Next};
use suprnova::session::store::SessionMigrationError;
use suprnova::session::{SessionConfig, SessionData, SessionMiddleware, SessionStore};
use suprnova::{Auth, Crypt, EncryptionKey, EventFacade, FrameworkError};

/// Shared runtime - SQLx pools die with their creating runtime, so
/// every DB-touching path runs here.
static RT: Lazy<Runtime> = Lazy::new(|| Runtime::new().expect("tokio runtime"));

/// Serialises the event-fake critical sections; the fake store is
/// process-global.
static TEST_LOCK: Mutex<()> = Mutex::const_new(());

/// One-shot init for crypto, framework storage, and Magnetar auth.
static SETUP: Lazy<()> = Lazy::new(|| {
    Crypt::init(EncryptionKey::generate());

    RT.block_on(async {
        // Framework DB - the `App::singleton(DbConnection)` install is
        // what backs `DB::connection()` for the 2FA + remember-me code.
        let config = suprnova::database::DatabaseConfig::builder()
            .url("sqlite::memory:")
            .max_connections(1)
            .min_connections(1)
            .logging(false)
            .build();
        let conn = suprnova::database::DbConnection::connect(&config)
            .await
            .expect("connect framework db");
        LocalMigrator::up(conn.inner(), None)
            .await
            .expect("run local migrator");
        suprnova::App::singleton(conn);

        magnetar_auth::install().await;
    });
});

struct LocalMigrator;

#[async_trait::async_trait]
impl MigratorTrait for LocalMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(TwoFactorMigration),
            Box::new(TwoFactorReplayMigration),
            Box::new(CreateRememberTokensTable),
        ]
    }
}

/// Mirrors the canonical `remember_tokens` shape from
/// `tests/auth_session_guard.rs` / `tests/remember_me.rs` - the schema
/// consumer apps own and ship with their own migrator. The framework
/// does not ship this migration; tests recreate it.
struct CreateRememberTokensTable;

impl MigrationName for CreateRememberTokensTable {
    fn name(&self) -> &str {
        "m20240101_000002_create_remember_tokens_table"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateRememberTokensTable {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(RememberTokens::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(RememberTokens::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(RememberTokens::UserId).string().not_null())
                    .col(ColumnDef::new(RememberTokens::Selector).string().not_null())
                    .col(
                        ColumnDef::new(RememberTokens::TokenHash)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(RememberTokens::ExpiresAt)
                            .timestamp()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(RememberTokens::CreatedAt)
                            .timestamp()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        ColumnDef::new(RememberTokens::LastUsedAt)
                            .timestamp()
                            .null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_two_factor_challenge_remember_selector")
                    .table(RememberTokens::Table)
                    .col(RememberTokens::Selector)
                    .unique()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(RememberTokens::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum RememberTokens {
    Table,
    Id,
    UserId,
    Selector,
    TokenHash,
    ExpiresAt,
    CreatedAt,
    LastUsedAt,
}

/// Minimal `TwoFactorUser` for the enroll/confirm steps. The
/// challenge promotion itself reads pending state from the session;
/// it does not need a `TwoFactorUser` impl.
struct ChallengeUser {
    user_id: String,
    email: String,
}

impl TwoFactorUser for ChallengeUser {
    fn user_id(&self) -> &str {
        &self.user_id
    }
    fn email(&self) -> &str {
        &self.email
    }
}

/// Compute the live TOTP for an otpauth URL exactly like an
/// authenticator app would.
fn totp_code_for(otpauth_url: &str) -> String {
    use totp_rs::{Algorithm, Secret, TOTP};
    let url = url::Url::parse(otpauth_url).unwrap();
    let secret = url
        .query_pairs()
        .find(|(k, _)| k == "secret")
        .map(|(_, v)| v.into_owned())
        .expect("otpauth url must contain a secret query param");
    let bytes = Secret::Encoded(secret).to_bytes().unwrap();
    TOTP::new(Algorithm::SHA1, 6, 1, 30, bytes, None, "user".into())
        .unwrap()
        .generate_current()
        .unwrap()
}

/// Drive `fut` inside the three task-local scopes a real request
/// installs: the session, the pending-cookies bag, and the auth
/// request state. The caller passes the pending-cookies slot in so
/// they can keep an `Arc` clone outside the closure and inspect the
/// queued cookies live from inside the closure - same pattern as
/// `tests/remember_me.rs`.
async fn run_in_request_with_slot<F, T>(pending_slot: Arc<StdMutex<Vec<Cookie>>>, fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    let session_slot = suprnova::session::new_session_slot_for_test();
    suprnova::session::session_scope_for_test(
        session_slot,
        suprnova::session::pending_cookies_scope_for_test(
            pending_slot,
            suprnova::auth::request_state::request_state_scope_for_test(fut),
        ),
    )
    .await
}

/// Convenience: tests that don't care about pending cookies don't have
/// to thread an inspector slot through. Creates a throwaway slot.
async fn run_in_request<F, T>(fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    let pending_slot = suprnova::session::new_pending_cookies_slot_for_test();
    run_in_request_with_slot(pending_slot, fut).await
}

/// Helper: read the current session id (panics if no scope installed).
fn current_session_id() -> String {
    suprnova::session::session()
        .map(|s| s.id)
        .expect("session scope must be installed")
}

/// Helper: read the current CSRF token (panics if no scope installed).
fn current_csrf() -> String {
    suprnova::session::session()
        .map(|s| s.csrf_token)
        .expect("session scope must be installed")
}

/// Helper: register a fresh magnetar user + enroll/confirm 2FA against
/// it. Returns `(user_id, email, otpauth_url)` so the caller can drive
/// `start_challenge` / `complete_challenge` with valid codes.
async fn register_and_enroll(label: &str) -> (String, String, String) {
    let (user_id, email, otpauth_url, _) = register_and_enroll_with_recovery(label).await;
    (user_id, email, otpauth_url)
}

async fn register_and_enroll_with_recovery(label: &str) -> (String, String, String, Vec<String>) {
    let email = format!("{label}@2fa.test");
    let user = Auth::password()
        .register(&email, "p@ssw0rd")
        .await
        .expect("magnetar register");
    let user_id = user.id.to_string();

    let tf_user = ChallengeUser {
        user_id: user_id.clone(),
        email: email.clone(),
    };
    let resp = TwoFactor::enroll(&tf_user).await.expect("enroll");
    let confirm_code = totp_code_for(&resp.otpauth_url);
    TwoFactor::confirm(&tf_user, &confirm_code)
        .await
        .expect("confirm");
    (user_id, email, resp.otpauth_url, resp.recovery_codes)
}

struct PromotionWriteFailsOnceStore {
    sessions: StdMutex<HashMap<String, SessionData>>,
    remaining_write_failures: AtomicUsize,
}

struct PromotionCommitAckUnknownStore {
    sessions: StdMutex<HashMap<String, SessionData>>,
}

#[async_trait::async_trait]
impl SessionStore for PromotionCommitAckUnknownStore {
    async fn read(&self, id: &str) -> Result<Option<SessionData>, FrameworkError> {
        Ok(self.sessions.lock().unwrap().get(id).cloned())
    }

    async fn write(&self, session: &SessionData) -> Result<(), FrameworkError> {
        self.sessions
            .lock()
            .unwrap()
            .insert(session.id.clone(), session.clone());
        Ok(())
    }

    async fn migrate_two_factor_session(
        &self,
        old_id: &str,
        session: &SessionData,
    ) -> Result<(), SessionMigrationError> {
        let mut sessions = self.sessions.lock().unwrap();
        sessions.remove(old_id);
        sessions.insert(session.id.clone(), session.clone());
        Err(SessionMigrationError::OutcomeUnknown(
            FrameworkError::internal("simulated commit applied but acknowledgement was lost"),
        ))
    }

    async fn destroy(&self, id: &str) -> Result<(), FrameworkError> {
        self.sessions.lock().unwrap().remove(id);
        Ok(())
    }

    async fn destroy_for_user(&self, user_id: &str) -> Result<u64, FrameworkError> {
        let mut sessions = self.sessions.lock().unwrap();
        let before = sessions.len();
        sessions.retain(|_, session| session.user_id.as_deref() != Some(user_id));
        Ok((before - sessions.len()) as u64)
    }

    async fn gc(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }
}

#[async_trait::async_trait]
impl SessionStore for PromotionWriteFailsOnceStore {
    async fn read(&self, id: &str) -> Result<Option<SessionData>, FrameworkError> {
        Ok(self.sessions.lock().unwrap().get(id).cloned())
    }

    async fn write(&self, session: &SessionData) -> Result<(), FrameworkError> {
        if session.user_id.is_some()
            && self
                .remaining_write_failures
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
        {
            return Err(FrameworkError::internal(
                "simulated authenticated session write failure",
            ));
        }
        self.sessions
            .lock()
            .unwrap()
            .insert(session.id.clone(), session.clone());
        Ok(())
    }

    async fn migrate_two_factor_session(
        &self,
        old_id: &str,
        session: &SessionData,
    ) -> Result<(), SessionMigrationError> {
        if self
            .remaining_write_failures
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                remaining.checked_sub(1)
            })
            .is_ok()
        {
            return Err(SessionMigrationError::RolledBack(FrameworkError::internal(
                "simulated authenticated session write failure",
            )));
        }

        let mut sessions = self.sessions.lock().unwrap();
        if !sessions.contains_key(old_id) {
            return Err(SessionMigrationError::RolledBack(FrameworkError::internal(
                "pending session is missing",
            )));
        }
        if sessions.contains_key(&session.id) {
            return Err(SessionMigrationError::RolledBack(FrameworkError::internal(
                "replacement session id already exists",
            )));
        }
        sessions.remove(old_id);
        sessions.insert(session.id.clone(), session.clone());
        Ok(())
    }

    async fn destroy(&self, id: &str) -> Result<(), FrameworkError> {
        self.sessions.lock().unwrap().remove(id);
        Ok(())
    }

    async fn destroy_for_user(&self, user_id: &str) -> Result<u64, FrameworkError> {
        let mut sessions = self.sessions.lock().unwrap();
        let before = sessions.len();
        sessions.retain(|_, session| session.user_id.as_deref() != Some(user_id));
        Ok((before - sessions.len()) as u64)
    }

    async fn gc(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }
}

async fn post_request_with_cookie(name: &str, value: &str) -> suprnova::Request {
    use bytes::Bytes;
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper_util::rt::TokioIo;
    use std::convert::Infallible;
    use tokio::io::AsyncWriteExt;
    use tokio::sync::oneshot;

    let encoded = value
        .replace('=', "%3D")
        .replace('+', "%2B")
        .replace('/', "%2F");
    let http_bytes = format!(
        "POST /two-factor-challenge HTTP/1.1\r\nHost: localhost\r\nCookie: {name}={encoded}\r\nContent-Length: 0\r\n\r\n"
    )
    .into_bytes();
    let (request_tx, request_rx) = oneshot::channel();
    let request_tx = StdMutex::new(Some(request_tx));
    let (mut client_io, server_io) = tokio::io::duplex(http_bytes.len() + 64 * 1024);

    tokio::spawn(async move {
        let service = service_fn(move |request: hyper::Request<hyper::body::Incoming>| {
            let request = suprnova::Request::new(request);
            if let Some(tx) = request_tx.lock().unwrap().take() {
                let _ = tx.send(request);
            }
            async {
                std::future::pending::<()>().await;
                Ok::<_, Infallible>(hyper::Response::new(http_body_util::Empty::<Bytes>::new()))
            }
        });
        let _ = http1::Builder::new()
            .serve_connection(TokioIo::new(server_io), service)
            .await;
    });
    client_io.write_all(&http_bytes).await.unwrap();
    request_rx.await.expect("server received request")
}

fn session_cookie(config: &SessionConfig, id: &str) -> Cookie {
    Cookie::encrypted(&config.cookie_name, id).expect("encrypt session cookie")
}

fn response_session_cookie(response: &suprnova::HttpResponse, name: &str) -> Option<String> {
    response
        .headers()
        .filter(|(header_name, _)| header_name.eq_ignore_ascii_case("set-cookie"))
        .map(|(_, value)| value)
        .find(|header| header.starts_with(&format!("{name}=")))
        .map(ToOwned::to_owned)
}

fn response_set_cookies(response: &suprnova::HttpResponse) -> Vec<String> {
    response
        .headers()
        .filter(|(name, _)| name.eq_ignore_ascii_case("set-cookie"))
        .map(|(_, value)| value.to_owned())
        .collect()
}

#[test]
fn remembered_recovery_promotion_rollback_retires_carrier_and_preserves_pending_session() {
    Lazy::force(&SETUP);
    RT.block_on(async {
        let _serial = TEST_LOCK.lock().await;
        let _fake = EventFacade::fake();
        magnetar_auth::reset_remember_tracking();
        let (user_id, _, _, recovery_codes) =
            register_and_enroll_with_recovery("remembered-recovery-persist-retry").await;

        let old_id = "r".repeat(40);
        let mut pending = SessionData::new(old_id.clone(), "c".repeat(40));
        pending.data.insert(
            "_two_factor_pending_user_id".to_owned(),
            serde_json::Value::String(user_id.clone()),
        );
        pending.data.insert(
            "_two_factor_pending_remember".to_owned(),
            serde_json::Value::Bool(true),
        );
        let store = Arc::new(PromotionWriteFailsOnceStore {
            sessions: StdMutex::new(HashMap::from([(old_id.clone(), pending)])),
            remaining_write_failures: AtomicUsize::new(1),
        });
        let mut config = SessionConfig::default();
        config.cookie_secure = false;
        config.cookie_name = "remembered_two_factor_promotion".to_owned();
        let old_cookie = session_cookie(&config, &old_id);
        let middleware = SessionMiddleware::with_store(config.clone(), store.clone());

        let first_code = recovery_codes[0].clone();
        let first_next: Next = Arc::new(move |_request| {
            let code = first_code.clone();
            Box::pin(async move {
                Cookie::queue(Cookie::new("promotion_notice", "kept").secure(false));
                TwoFactor::complete_challenge(&code).await?;
                Ok(suprnova::HttpResponse::text("authenticated"))
            })
        });
        let first = match middleware
            .handle(
                post_request_with_cookie(&config.cookie_name, old_cookie.value()).await,
                first_next,
            )
            .await
        {
            Err(response) => response,
            Ok(_) => panic!("atomic promotion rollback must fail closed"),
        };
        assert_eq!(first.status_code(), 500);
        let cookies = response_set_cookies(&first);
        assert!(
            cookies
                .iter()
                .any(|header| header.starts_with("promotion_notice=kept")),
            "unrelated queued cookies remain intentional response state"
        );
        assert!(
            cookies.iter().all(|header| {
                !header.starts_with("remember_me=") || header.starts_with("remember_me=;")
            }),
            "a remember credential minted for an uncommitted promotion must never reach the client"
        );
        assert!(
            response_session_cookie(&first, &config.cookie_name).is_none(),
            "rollback must not issue the authenticated framework session cookie"
        );
        assert_eq!(
            magnetar_auth::live_remember_selector_count(),
            0,
            "the exact minted remember credential must be retired"
        );
        assert_eq!(magnetar_auth::remember_selector_revocation_count(), 1);
        assert!(store.sessions.lock().unwrap().contains_key(&old_id));

        let second_code = recovery_codes[1].clone();
        let second_next: Next = Arc::new(move |_request| {
            let code = second_code.clone();
            Box::pin(async move {
                TwoFactor::complete_challenge(&code).await?;
                Ok(suprnova::HttpResponse::text("authenticated"))
            })
        });
        let retry = match middleware
            .handle(
                post_request_with_cookie(&config.cookie_name, old_cookie.value()).await,
                second_next,
            )
            .await
        {
            Ok(response) => response,
            Err(response) => panic!(
                "a fresh proof must promote the rollback-preserved pending session; got {}",
                response.status_code()
            ),
        };
        assert_eq!(retry.status_code(), 200);
        let retry_cookies = response_set_cookies(&retry);
        assert!(
            retry_cookies
                .iter()
                .any(|header| header.starts_with("remember_me=")
                    && !header.starts_with("remember_me=;")),
            "the committed retry may deliver its fresh remember credential"
        );
        assert!(response_session_cookie(&retry, &config.cookie_name).is_some());
        assert_eq!(magnetar_auth::live_remember_selector_count(), 1);
    });
}

#[test]
fn remembered_promotion_retirement_failure_still_suppresses_the_carrier() {
    Lazy::force(&SETUP);
    RT.block_on(async {
        let _serial = TEST_LOCK.lock().await;
        let _fake = EventFacade::fake();
        magnetar_auth::reset_remember_tracking();
        let (user_id, _, _, recovery_codes) =
            register_and_enroll_with_recovery("remembered-retirement-failure").await;

        let old_id = "f".repeat(40);
        let mut pending = SessionData::new(old_id.clone(), "c".repeat(40));
        pending.data.insert(
            "_two_factor_pending_user_id".to_owned(),
            serde_json::Value::String(user_id),
        );
        pending.data.insert(
            "_two_factor_pending_remember".to_owned(),
            serde_json::Value::Bool(true),
        );
        let store = Arc::new(PromotionWriteFailsOnceStore {
            sessions: StdMutex::new(HashMap::from([(old_id.clone(), pending)])),
            remaining_write_failures: AtomicUsize::new(1),
        });
        let mut config = SessionConfig::default();
        config.cookie_secure = false;
        config.cookie_name = "remember_retirement_failure".to_owned();
        let old_cookie = session_cookie(&config, &old_id);
        magnetar_auth::fail_next_remember_revoke();
        let code = recovery_codes[0].clone();
        let next: Next = Arc::new(move |_request| {
            let code = code.clone();
            Box::pin(async move {
                TwoFactor::complete_challenge(&code).await?;
                Ok(suprnova::HttpResponse::text("authenticated"))
            })
        });

        let response = match SessionMiddleware::with_store(config.clone(), store.clone())
            .handle(
                post_request_with_cookie(&config.cookie_name, old_cookie.value()).await,
                next,
            )
            .await
        {
            Err(response) => response,
            Ok(_) => panic!("promotion rollback must fail closed"),
        };
        assert_eq!(response.status_code(), 500);
        assert!(
            response_set_cookies(&response).iter().all(|header| {
                !header.starts_with("remember_me=") || header.starts_with("remember_me=;")
            }),
            "a failed retirement may leave a TTL-bounded server row but never its bearer carrier"
        );
        assert_eq!(magnetar_auth::remember_selector_revocation_count(), 1);
        assert_eq!(
            magnetar_auth::live_remember_selector_count(),
            1,
            "the scripted backend failure leaves the server-side credential unconfirmed"
        );
        assert!(store.sessions.lock().unwrap().contains_key(&old_id));
        magnetar_auth::reset_remember_tracking();
    });
}

#[test]
fn remembered_promotion_cookie_construction_failure_retires_carrier_before_migration() {
    Lazy::force(&SETUP);
    RT.block_on(async {
        let _serial = TEST_LOCK.lock().await;
        let _fake = EventFacade::fake();
        magnetar_auth::reset_remember_tracking();
        let (user_id, _, _, recovery_codes) =
            register_and_enroll_with_recovery("remembered-cookie-build-failure").await;

        let old_id = "k".repeat(40);
        let mut pending = SessionData::new(old_id.clone(), "c".repeat(40));
        pending.data.insert(
            "_two_factor_pending_user_id".to_owned(),
            serde_json::Value::String(user_id.clone()),
        );
        pending.data.insert(
            "_two_factor_pending_remember".to_owned(),
            serde_json::Value::Bool(true),
        );
        let store = Arc::new(PromotionWriteFailsOnceStore {
            sessions: StdMutex::new(HashMap::from([(old_id.clone(), pending)])),
            remaining_write_failures: AtomicUsize::new(0),
        });
        let mut config = SessionConfig::default();
        config.cookie_secure = false;
        config.cookie_name = "remember_cookie_build_failure".to_owned();
        let old_cookie = session_cookie(&config, &old_id);
        let middleware = SessionMiddleware::with_store(config.clone(), store.clone());

        suprnova::session::middleware::fail_next_session_cookie_construction_for_test();
        let first_code = recovery_codes[0].clone();
        let first_next: Next = Arc::new(move |_request| {
            let code = first_code.clone();
            Box::pin(async move {
                Cookie::queue(Cookie::new("cookie_build_notice", "kept").secure(false));
                TwoFactor::complete_challenge(&code).await?;
                Ok(suprnova::HttpResponse::text("authenticated"))
            })
        });
        let first = match middleware
            .handle(
                post_request_with_cookie(&config.cookie_name, old_cookie.value()).await,
                first_next,
            )
            .await
        {
            Err(response) => response,
            Ok(_) => panic!("session-cookie construction failure must fail closed"),
        };
        assert_eq!(first.status_code(), 500);
        assert!(
            response_set_cookies(&first)
                .iter()
                .any(|header| header.starts_with("cookie_build_notice=kept")),
            "unrelated pending cookies must survive the fail-closed response"
        );
        assert!(
            response_set_cookies(&first).iter().all(|header| {
                !header.starts_with("remember_me=") || header.starts_with("remember_me=;")
            }),
            "the pre-migration failure must not deliver its remember bearer"
        );
        assert!(response_session_cookie(&first, &config.cookie_name).is_none());
        assert_eq!(magnetar_auth::live_remember_selector_count(), 0);
        assert_eq!(magnetar_auth::remember_selector_revocation_count(), 1);
        assert!(store.sessions.lock().unwrap().contains_key(&old_id));

        let second_code = recovery_codes[1].clone();
        let second_next: Next = Arc::new(move |_request| {
            let code = second_code.clone();
            Box::pin(async move {
                TwoFactor::complete_challenge(&code).await?;
                Ok(suprnova::HttpResponse::text("authenticated"))
            })
        });
        let retry = match middleware
            .handle(
                post_request_with_cookie(&config.cookie_name, old_cookie.value()).await,
                second_next,
            )
            .await
        {
            Ok(response) => response,
            Err(response) => panic!(
                "a fresh proof must complete the preserved challenge; got {}",
                response.status_code()
            ),
        };
        assert_eq!(retry.status_code(), 200);
        assert!(response_session_cookie(&retry, &config.cookie_name).is_some());
        assert!(!store.sessions.lock().unwrap().contains_key(&old_id));
        assert_eq!(
            store
                .sessions
                .lock()
                .unwrap()
                .values()
                .filter(|session| session.user_id.as_deref() == Some(user_id.as_str()))
                .count(),
            1
        );
    });
}

#[test]
fn promotion_commit_ack_unknown_expires_client_state_and_reconciles_replacement() {
    Lazy::force(&SETUP);
    RT.block_on(async {
        let _serial = TEST_LOCK.lock().await;
        let _fake = EventFacade::fake();
        magnetar_auth::reset_remember_tracking();
        let (user_id, _, _, recovery_codes) =
            register_and_enroll_with_recovery("promotion-ack-unknown").await;

        let old_id = "u".repeat(40);
        let mut pending = SessionData::new(old_id.clone(), "c".repeat(40));
        pending.data.insert(
            "_two_factor_pending_user_id".to_owned(),
            serde_json::Value::String(user_id),
        );
        pending.data.insert(
            "_two_factor_pending_remember".to_owned(),
            serde_json::Value::Bool(true),
        );
        let store = Arc::new(PromotionCommitAckUnknownStore {
            sessions: StdMutex::new(HashMap::from([(old_id.clone(), pending)])),
        });
        let mut config = SessionConfig::default();
        config.cookie_secure = false;
        config.cookie_name = "unknown_two_factor_promotion".to_owned();
        let old_cookie = session_cookie(&config, &old_id);
        let code = recovery_codes[0].clone();
        let next: Next = Arc::new(move |_request| {
            let code = code.clone();
            Box::pin(async move {
                TwoFactor::complete_challenge(&code).await?;
                Ok(suprnova::HttpResponse::text("authenticated"))
            })
        });

        let response = match SessionMiddleware::with_store(config.clone(), store.clone())
            .handle(
                post_request_with_cookie(&config.cookie_name, old_cookie.value()).await,
                next,
            )
            .await
        {
            Err(response) => response,
            Ok(_) => panic!("an unknown commit outcome must fail closed"),
        };
        assert_eq!(response.status_code(), 500);
        let cookies = response_set_cookies(&response);
        assert!(
            cookies.iter().any(|header| {
                header.starts_with(&format!("{}=;", config.cookie_name))
                    && header.contains("Max-Age=0")
            }),
            "unknown commit acknowledgement must invalidate the old browser credential"
        );
        assert!(
            cookies.iter().all(|header| {
                !header.starts_with(&format!("{}=", config.cookie_name))
                    || header.starts_with(&format!("{}=;", config.cookie_name))
            }),
            "unknown outcome must never deliver the replacement authenticated cookie"
        );
        assert!(
            cookies.iter().all(|header| {
                !header.starts_with("remember_me=") || header.starts_with("remember_me=;")
            }),
            "unknown outcome must suppress its freshly minted remember credential"
        );
        assert_eq!(magnetar_auth::live_remember_selector_count(), 0);
        assert_eq!(magnetar_auth::remember_selector_revocation_count(), 1);
        assert!(
            store.sessions.lock().unwrap().is_empty(),
            "best-effort exact reconciliation must remove the possibly committed replacement"
        );
    });
}

#[test]
fn recovery_promotion_write_failure_preserves_pending_session_for_retry() {
    Lazy::force(&SETUP);
    RT.block_on(async {
        let _serial = TEST_LOCK.lock().await;
        let _fake = EventFacade::fake();
        let (user_id, _, _, recovery_codes) =
            register_and_enroll_with_recovery("recovery-persist-retry").await;

        let old_id = "p".repeat(40);
        let mut pending = SessionData::new(old_id.clone(), "c".repeat(40));
        pending.data.insert(
            "_two_factor_pending_user_id".to_owned(),
            serde_json::Value::String(user_id.clone()),
        );
        let store = Arc::new(PromotionWriteFailsOnceStore {
            sessions: StdMutex::new(HashMap::from([(old_id.clone(), pending)])),
            remaining_write_failures: AtomicUsize::new(1),
        });
        let mut config = SessionConfig::default();
        config.cookie_secure = false;
        config.cookie_name = "two_factor_promotion".to_owned();
        let old_cookie = session_cookie(&config, &old_id);

        let first_code = recovery_codes[0].clone();
        let first_next: Next = Arc::new(move |_request| {
            let code = first_code.clone();
            Box::pin(async move {
                TwoFactor::complete_challenge(&code).await?;
                Ok(suprnova::HttpResponse::text("authenticated"))
            })
        });
        let middleware = SessionMiddleware::with_store(config.clone(), store.clone());
        let first = match middleware
            .handle(
                post_request_with_cookie(&config.cookie_name, old_cookie.value()).await,
                first_next,
            )
            .await
        {
            Err(response) => response,
            Ok(_) => panic!("authenticated session persistence failure must fail closed"),
        };
        assert_eq!(first.status_code(), 500);
        assert!(
            response_session_cookie(&first, &config.cookie_name).is_none(),
            "failed promotion must not issue an authenticated session cookie"
        );
        {
            let sessions = store.sessions.lock().unwrap();
            let restored = sessions
                .get(&old_id)
                .expect("the persisted pending session must remain available");
            assert_eq!(
                restored
                    .data
                    .get("_two_factor_pending_user_id")
                    .and_then(serde_json::Value::as_str),
                Some(user_id.as_str())
            );
            assert!(
                sessions.values().all(|session| session.user_id.is_none()),
                "failed promotion must leave no reachable authenticated session"
            );
        }

        let replayed_code = recovery_codes[0].clone();
        let replay_next: Next = Arc::new(move |_request| {
            let code = replayed_code.clone();
            Box::pin(async move {
                TwoFactor::complete_challenge(&code).await?;
                Ok(suprnova::HttpResponse::text("authenticated"))
            })
        });
        let replay = match middleware
            .handle(
                post_request_with_cookie(&config.cookie_name, old_cookie.value()).await,
                replay_next,
            )
            .await
        {
            Err(response) => response,
            Ok(_) => panic!("the consumed recovery code must remain single-use"),
        };
        assert_eq!(replay.status_code(), 401);
        assert!(
            store.sessions.lock().unwrap().contains_key(&old_id),
            "rejecting a replay must leave the pending challenge recoverable"
        );

        let second_code = recovery_codes[1].clone();
        let second_next: Next = Arc::new(move |_request| {
            let code = second_code.clone();
            Box::pin(async move {
                TwoFactor::complete_challenge(&code).await?;
                Ok(suprnova::HttpResponse::text("authenticated"))
            })
        });
        let second = match middleware
            .handle(
                post_request_with_cookie(&config.cookie_name, old_cookie.value()).await,
                second_next,
            )
            .await
        {
            Ok(response) => response,
            Err(response) => panic!(
                "a second unused recovery code must complete the preserved challenge; got {}",
                response.status_code()
            ),
        };
        assert_eq!(second.status_code(), 200);
        assert!(
            response_session_cookie(&second, &config.cookie_name).is_some(),
            "successful retry must issue the authenticated session cookie"
        );
        let sessions = store.sessions.lock().unwrap();
        assert!(!sessions.contains_key(&old_id));
        assert_eq!(
            sessions
                .values()
                .filter(|session| session.user_id.as_deref() == Some(user_id.as_str()))
                .count(),
            1,
            "exactly one authenticated session must be reachable after retry"
        );
    });
}

#[test]
fn totp_promotion_write_failure_preserves_pending_session_and_replay_claim() {
    Lazy::force(&SETUP);
    RT.block_on(async {
        let _serial = TEST_LOCK.lock().await;
        let _fake = EventFacade::fake();
        let (user_id, _, otpauth_url) = register_and_enroll("totp-persist-retry").await;

        let old_id = "t".repeat(40);
        let mut pending = SessionData::new(old_id.clone(), "c".repeat(40));
        pending.data.insert(
            "_two_factor_pending_user_id".to_owned(),
            serde_json::Value::String(user_id),
        );
        let store = Arc::new(PromotionWriteFailsOnceStore {
            sessions: StdMutex::new(HashMap::from([(old_id.clone(), pending)])),
            remaining_write_failures: AtomicUsize::new(1),
        });
        let mut config = SessionConfig::default();
        config.cookie_secure = false;
        config.cookie_name = "two_factor_totp_promotion".to_owned();
        let old_cookie = session_cookie(&config, &old_id);
        let totp = totp_code_for(&otpauth_url);

        let first_code = totp.clone();
        let first_next: Next = Arc::new(move |_request| {
            let code = first_code.clone();
            Box::pin(async move {
                TwoFactor::complete_challenge(&code).await?;
                Ok(suprnova::HttpResponse::text("authenticated"))
            })
        });
        let middleware = SessionMiddleware::with_store(config.clone(), store.clone());
        let first = match middleware
            .handle(
                post_request_with_cookie(&config.cookie_name, old_cookie.value()).await,
                first_next,
            )
            .await
        {
            Err(response) => response,
            Ok(_) => panic!("authenticated session persistence failure must fail closed"),
        };
        assert_eq!(first.status_code(), 500);
        assert!(response_session_cookie(&first, &config.cookie_name).is_none());
        assert!(store.sessions.lock().unwrap().contains_key(&old_id));

        let replay_next: Next = Arc::new(move |_request| {
            let code = totp.clone();
            Box::pin(async move {
                TwoFactor::complete_challenge(&code).await?;
                Ok(suprnova::HttpResponse::text("authenticated"))
            })
        });
        let replay = match middleware
            .handle(
                post_request_with_cookie(&config.cookie_name, old_cookie.value()).await,
                replay_next,
            )
            .await
        {
            Err(response) => response,
            Ok(_) => panic!("the accepted TOTP timestep must remain single-use"),
        };
        assert_eq!(replay.status_code(), 401);
        assert!(
            store.sessions.lock().unwrap().contains_key(&old_id),
            "rejecting the replay must preserve the pending challenge for a later timestep"
        );
    });
}

#[test]
fn complete_challenge_rotates_session_id_and_csrf() {
    Lazy::force(&SETUP);
    RT.block_on(async {
        let _serial = TEST_LOCK.lock().await;
        let _fake = EventFacade::fake();

        let (user_id, _email, otpauth_url) = register_and_enroll("rotate").await;

        let (before_id, before_csrf, after_id, after_csrf) = run_in_request(async {
            let before_id = current_session_id();
            let before_csrf = current_csrf();

            TwoFactor::start_challenge(&user_id, false)
                .await
                .expect("start_challenge");
            // confirm above stamped no replay claim; verify inside
            // complete_challenge will stamp the current timestep.
            let totp = totp_code_for(&otpauth_url);
            TwoFactor::complete_challenge(&totp)
                .await
                .expect("complete_challenge");

            let after_id = current_session_id();
            let after_csrf = current_csrf();
            (before_id, before_csrf, after_id, after_csrf)
        })
        .await;

        assert_ne!(
            before_id, after_id,
            "session id must rotate on challenge complete to defeat session fixation"
        );
        assert_ne!(
            before_csrf, after_csrf,
            "CSRF token must rotate on challenge complete"
        );
    });
}

#[test]
fn complete_challenge_dispatches_login_and_authenticated_and_challenged() {
    Lazy::force(&SETUP);
    RT.block_on(async {
        let _serial = TEST_LOCK.lock().await;
        let _fake = EventFacade::fake();

        let (user_id, _email, otpauth_url) = register_and_enroll("events").await;
        let captured_user_id = user_id.clone();

        run_in_request(async {
            TwoFactor::start_challenge(&user_id, false)
                .await
                .expect("start_challenge");
            let totp = totp_code_for(&otpauth_url);
            TwoFactor::complete_challenge(&totp)
                .await
                .expect("complete_challenge");
        })
        .await;

        assert_dispatched::<Login>(|e| e.user_id == captured_user_id && !e.remember);
        assert_dispatched::<Authenticated>(|e| e.user_id == captured_user_id);
        assert_dispatched::<TwoFactorChallenged>(|e| e.user_id == captured_user_id);
    });
}

#[test]
fn complete_challenge_with_remember_true_reissues_remember_me_cookie() {
    Lazy::force(&SETUP);
    RT.block_on(async {
        let _serial = TEST_LOCK.lock().await;
        let _fake = EventFacade::fake();

        let (user_id, _email, otpauth_url) = register_and_enroll("remember-true").await;
        let captured_user_id = user_id.clone();

        // Pre-create the slot so we can clone it for live inspection
        // inside the closure - the scope retains the original, our
        // clone gives a read window from outside the scope's borrow.
        let pending_slot = suprnova::session::new_pending_cookies_slot_for_test();
        let inspector = pending_slot.clone();

        let (after_start, after_complete) = run_in_request_with_slot(pending_slot, async move {
            TwoFactor::start_challenge(&user_id, true)
                .await
                .expect("start_challenge");
            let after_start = inspector.lock().unwrap().clone();
            let totp = totp_code_for(&otpauth_url);
            TwoFactor::complete_challenge(&totp)
                .await
                .expect("complete_challenge");
            let after_complete = inspector.lock().unwrap().clone();
            (after_start, after_complete)
        })
        .await;

        // start_challenge queues a clear directive for the browser's single
        // remember-me slot. Completing the challenge replaces that directive
        // with one fresh credential instead of leaving duplicate headers whose
        // order could determine the browser's final state.
        assert_eq!(
            after_start.len(),
            1,
            "start_challenge must queue one cookie"
        );
        assert_eq!(after_start[0].name(), "remember_me");
        assert!(
            after_start[0].value().is_empty(),
            "start_challenge must clear the prior remember_me cookie"
        );

        assert_eq!(
            after_complete.len(),
            1,
            "complete_challenge must replace, not append, the remember_me directive"
        );
        assert_eq!(after_complete[0].name(), "remember_me");
        assert!(
            !after_complete[0].value().is_empty(),
            "remember=true must queue a fresh remember_me cookie with a non-empty value"
        );

        assert_dispatched::<Login>(|e| e.user_id == captured_user_id && e.remember);
        assert_dispatched::<Authenticated>(|e| e.user_id == captured_user_id);
    });
}

#[test]
fn complete_challenge_survives_remember_issue_failure_without_claiming_remembered_login() {
    Lazy::force(&SETUP);
    RT.block_on(async {
        let _serial = TEST_LOCK.lock().await;
        let _fake = EventFacade::fake();

        let (user_id, _email, otpauth_url) = register_and_enroll("remember-failure").await;
        let captured_user_id = user_id.clone();
        let pending_slot = suprnova::session::new_pending_cookies_slot_for_test();
        let inspector = pending_slot.clone();

        let (
            outcome,
            session_id_before,
            session_id_after,
            csrf_before,
            csrf_after,
            pending_user,
            pending_remember,
            auth_user,
            queued_cookies,
            failure_hook_unconsumed,
        ) = run_in_request_with_slot(pending_slot, async move {
            TwoFactor::start_challenge(&user_id, true)
                .await
                .expect("start_challenge");
            let session_id_before = current_session_id();
            let csrf_before = current_csrf();
            let totp = totp_code_for(&otpauth_url);

            magnetar_auth::fail_next_remember_issue();
            let outcome = TwoFactor::complete_challenge(&totp).await;
            let failure_hook_unconsumed = magnetar_auth::take_unconsumed_remember_issue_failure();

            (
                outcome,
                session_id_before,
                current_session_id(),
                csrf_before,
                current_csrf(),
                TwoFactor::pending_user_id(),
                suprnova::session::two_factor_pending_remember(),
                suprnova::session::auth_user_id(),
                inspector.lock().unwrap().clone(),
                failure_hook_unconsumed,
            )
        })
        .await;

        let user = outcome.expect("remember failure must not undo accepted challenge proof");
        assert!(
            !failure_hook_unconsumed,
            "complete_challenge must attempt remember issuance when it was requested"
        );
        assert_eq!(user.id.to_string(), captured_user_id);
        assert_ne!(session_id_after, session_id_before);
        assert_ne!(csrf_after, csrf_before);
        assert_eq!(pending_user, None);
        assert!(!pending_remember);
        assert_eq!(auth_user.as_deref(), Some(captured_user_id.as_str()));
        assert_eq!(queued_cookies.len(), 1);
        assert_eq!(queued_cookies[0].name(), "remember_me");
        assert!(queued_cookies[0].value().is_empty());
        assert_dispatched::<Login>(|e| e.user_id == captured_user_id && !e.remember);
        assert_not_dispatched::<Login>(|e| e.user_id == captured_user_id && e.remember);
        assert_dispatched::<Authenticated>(|e| e.user_id == captured_user_id);
        assert_dispatched::<TwoFactorChallenged>(|e| e.user_id == captured_user_id);
        assert_not_dispatched::<TwoFactorChallengeFailed>(|e| e.user_id == captured_user_id);
    });
}

#[test]
fn complete_challenge_with_remember_false_does_not_issue_remember_me_cookie() {
    Lazy::force(&SETUP);
    RT.block_on(async {
        let _serial = TEST_LOCK.lock().await;
        let _fake = EventFacade::fake();

        let (user_id, _email, otpauth_url) = register_and_enroll("remember-false").await;
        let captured_user_id = user_id.clone();

        let pending_slot = suprnova::session::new_pending_cookies_slot_for_test();
        let inspector = pending_slot.clone();

        let (after_start, after_complete) = run_in_request_with_slot(pending_slot, async move {
            TwoFactor::start_challenge(&user_id, false)
                .await
                .expect("start_challenge");
            let after_start = inspector.lock().unwrap().clone();
            let totp = totp_code_for(&otpauth_url);
            TwoFactor::complete_challenge(&totp)
                .await
                .expect("complete_challenge");
            let after_complete = inspector.lock().unwrap().clone();
            (after_start, after_complete)
        })
        .await;

        // remember=false → complete_challenge must NOT push a new
        // cookie. The slot may still hold the clear cookie that
        // start_challenge queued; complete_challenge adds nothing.
        assert_eq!(
            after_complete.len(),
            after_start.len(),
            "remember=false must not queue any cookie at complete_challenge; \
             before={before}, after={after}",
            before = after_start.len(),
            after = after_complete.len(),
        );

        assert_dispatched::<Login>(|e| e.user_id == captured_user_id && !e.remember);
        assert_dispatched::<Authenticated>(|e| e.user_id == captured_user_id);
        // Sanity: `Login{remember:true}` was NOT dispatched.
        assert_not_dispatched::<Login>(|e| e.remember);
    });
}

#[test]
fn complete_challenge_with_bad_code_records_single_brute_force_attempt() {
    Lazy::force(&SETUP);
    RT.block_on(async {
        let _serial = TEST_LOCK.lock().await;
        let _fake = EventFacade::fake();

        let (user_id, email, _otpauth_url) = register_and_enroll("bf-single").await;

        // Baseline: zero failed attempts.
        let before = BruteForce::get_lockout_status(&email).await.unwrap();
        assert_eq!(
            before.failed_attempts, 0,
            "fresh user must start with zero failed attempts"
        );

        run_in_request(async {
            TwoFactor::start_challenge(&user_id, false)
                .await
                .expect("start_challenge");
            // "000000" is overwhelmingly likely to not be the current
            // TOTP and not a recovery code (recovery codes are 8-char
            // alnum). Both validation paths reject it.
            let err = TwoFactor::complete_challenge("000000")
                .await
                .expect_err("bad code must fail");
            assert_eq!(err.status_code(), 401, "wrong code is 401, not 429");
        })
        .await;

        // The single bad submission must count as ONE attempt, not two
        // (one from verify failing + one from consume_recovery_code
        // failing). The fix factors out silent verify/consume_recovery
        // cores and records the canonical attempt at the outer layer.
        let after = BruteForce::get_lockout_status(&email).await.unwrap();
        assert_eq!(
            after.failed_attempts, 1,
            "bad code must record exactly one failed attempt; got {}",
            after.failed_attempts
        );
    });
}

#[test]
fn complete_challenge_fails_closed_when_attempt_admission_cannot_persist() {
    Lazy::force(&SETUP);
    RT.block_on(async {
        let _serial = TEST_LOCK.lock().await;
        let _fake = EventFacade::fake();

        let (user_id, _email, _otpauth_url) = register_and_enroll("admission-write-failure").await;
        magnetar_auth::fail_next_attempt_write();

        run_in_request(async {
            TwoFactor::start_challenge(&user_id, false)
                .await
                .expect("start_challenge");
            let error = TwoFactor::complete_challenge("invalid-code")
                .await
                .expect_err("an unavailable attempt store must close challenge completion");
            assert_eq!(error.status_code(), 503);
        })
        .await;
    });
}

#[test]
fn complete_challenge_cancels_attempt_after_totp_read_error() {
    Lazy::force(&SETUP);
    RT.block_on(async {
        let _serial = TEST_LOCK.lock().await;
        let _fake = EventFacade::fake();

        let (user_id, email, _otpauth_url) = register_and_enroll("totp-read-error").await;
        let db = suprnova::DB::connection().unwrap();
        let row = suprnova::auth_flows::two_factor::entity::Entity::find_by_id(user_id.clone())
            .one(db.inner())
            .await
            .unwrap()
            .unwrap();
        let mut row = row.into_active_model();
        row.secret = Set("not-valid-ciphertext".to_owned());
        row.update(db.inner()).await.unwrap();

        run_in_request(async {
            TwoFactor::start_challenge(&user_id, false)
                .await
                .expect("start_challenge");
            TwoFactor::complete_challenge("000000")
                .await
                .expect_err("corrupt TOTP state must fail");
        })
        .await;

        assert_eq!(
            BruteForce::get_lockout_status(&email)
                .await
                .unwrap()
                .failed_attempts,
            0
        );
        assert_not_dispatched::<AccountLocked>(|event| event.email == email);
    });
}

#[test]
fn complete_challenge_cancels_attempt_after_recovery_read_error() {
    Lazy::force(&SETUP);
    RT.block_on(async {
        let _serial = TEST_LOCK.lock().await;
        let _fake = EventFacade::fake();

        let (user_id, email, _otpauth_url) = register_and_enroll("recovery-read-error").await;
        let db = suprnova::DB::connection().unwrap();
        let row = suprnova::auth_flows::two_factor::entity::Entity::find_by_id(user_id.clone())
            .one(db.inner())
            .await
            .unwrap()
            .unwrap();
        let mut row = row.into_active_model();
        row.recovery_codes = Set(Some("not-valid-ciphertext".to_owned()));
        row.update(db.inner()).await.unwrap();

        run_in_request(async {
            TwoFactor::start_challenge(&user_id, false)
                .await
                .expect("start_challenge");
            TwoFactor::complete_challenge("000000")
                .await
                .expect_err("corrupt recovery state must fail");
        })
        .await;

        assert_eq!(
            BruteForce::get_lockout_status(&email)
                .await
                .unwrap()
                .failed_attempts,
            0
        );
        assert_not_dispatched::<AccountLocked>(|event| event.email == email);
    });
}

#[test]
fn cancellation_failure_returns_state_uncertain_and_keeps_capacity_reserved() {
    Lazy::force(&SETUP);
    RT.block_on(async {
        let _serial = TEST_LOCK.lock().await;
        let _fake = EventFacade::fake();

        let (user_id, email, _otpauth_url) = register_and_enroll("cancel-failure").await;
        for _ in 0..4 {
            BruteForce::record_failed_attempt(&email, None)
                .await
                .expect("seed failed attempt");
        }
        let db = suprnova::DB::connection().unwrap();
        let row = suprnova::auth_flows::two_factor::entity::Entity::find_by_id(user_id.clone())
            .one(db.inner())
            .await
            .unwrap()
            .unwrap();
        let mut row = row.into_active_model();
        row.secret = Set("not-valid-ciphertext".to_owned());
        row.update(db.inner()).await.unwrap();
        magnetar_auth::fail_next_attempt_cancel();

        run_in_request(async {
            TwoFactor::start_challenge(&user_id, false)
                .await
                .expect("start_challenge");
            let error = TwoFactor::complete_challenge("000000")
                .await
                .expect_err("failed cancellation must close with uncertain state");
            assert_eq!(error.status_code(), 503);
        })
        .await;

        assert_eq!(
            BruteForce::get_lockout_status(&email)
                .await
                .unwrap()
                .failed_attempts,
            4,
            "a pending reservation is not a finalized public failure"
        );
        assert_not_dispatched::<AccountLocked>(|event| event.email == email);

        run_in_request(async {
            TwoFactor::start_challenge(&user_id, false)
                .await
                .expect("restart challenge");
            let error = TwoFactor::complete_challenge("000000")
                .await
                .expect_err("uncertain pending reservation must retain admission capacity");
            assert_eq!(error.status_code(), 429);
        })
        .await;
    });
}

#[test]
fn complete_challenge_caps_concurrent_proof_evaluation_at_attempt_limit() {
    Lazy::force(&SETUP);
    RT.block_on(async {
        let _serial = TEST_LOCK.lock().await;
        let _fake = EventFacade::fake();

        const ATTEMPT_LIMIT: usize = 5;
        let (user_id, _email, otpauth_url) = register_and_enroll("admission-race").await;
        let valid_code = totp_code_for(&otpauth_url);
        let mut codes = (0..(ATTEMPT_LIMIT + 1))
            .map(|index| format!("invalid-{index}"))
            .collect::<Vec<_>>();
        codes.push(valid_code);
        let _barrier = magnetar_auth::synchronize_attempt_admission(codes.len());

        let mut tasks = Vec::new();
        for code in codes {
            let user_id = user_id.clone();
            tasks.push(tokio::spawn(async move {
                run_in_request(async move {
                    TwoFactor::start_challenge(&user_id, false)
                        .await
                        .expect("start_challenge");
                    TwoFactor::complete_challenge(&code).await
                })
                .await
            }));
        }

        let mut statuses = Vec::new();
        for task in tasks {
            statuses.push(match task.await.expect("challenge attempt task joins") {
                Ok(_) => 200,
                Err(error) => error.status_code(),
            });
        }

        assert_eq!(
            statuses.iter().filter(|status| **status == 429).count(),
            2,
            "requests beyond the atomic admission budget must be rejected before proof evaluation"
        );
        assert_eq!(
            statuses
                .iter()
                .filter(|status| matches!(**status, 200 | 401))
                .count(),
            ATTEMPT_LIMIT,
            "only admitted requests may evaluate the submitted proof"
        );
    });
}

#[test]
fn threshold_crossing_invalid_challenge_dispatches_account_locked() {
    Lazy::force(&SETUP);
    RT.block_on(async {
        let _serial = TEST_LOCK.lock().await;
        let _fake = EventFacade::fake();

        let (user_id, email, _otpauth_url) = register_and_enroll("admission-lock-event").await;
        for _ in 0..4 {
            BruteForce::record_failed_attempt(&email, None)
                .await
                .expect("seed failed attempt");
        }

        run_in_request(async {
            TwoFactor::start_challenge(&user_id, false)
                .await
                .expect("start_challenge");
            let error = TwoFactor::complete_challenge("invalid-code")
                .await
                .expect_err("invalid threshold attempt must fail");
            assert_eq!(error.status_code(), 401);
        })
        .await;

        assert_eq!(
            dispatched_count::<AccountLocked>(|event| event.email == email),
            1,
            "invalid threshold finalization must emit exactly one lock event"
        );
    });
}

#[test]
fn finalized_failure_repairs_lock_and_dispatches_event_on_next_admission() {
    Lazy::force(&SETUP);
    RT.block_on(async {
        let _serial = TEST_LOCK.lock().await;
        let _fake = EventFacade::fake();

        let (user_id, email, _otpauth_url) = register_and_enroll("admission-lock-repair").await;
        for _ in 0..4 {
            BruteForce::record_failed_attempt(&email, None)
                .await
                .expect("seed failed attempt");
        }
        magnetar_auth::fail_next_attempt_lock();

        run_in_request(async {
            TwoFactor::start_challenge(&user_id, false)
                .await
                .expect("start challenge before scripted lock failure");
            let error = TwoFactor::complete_challenge("invalid-code")
                .await
                .expect_err("uncertain durable lock transition must fail closed");
            assert_eq!(error.status_code(), 503);
        })
        .await;
        assert_not_dispatched::<AccountLocked>(|event| event.email == email);
        assert_eq!(
            BruteForce::get_lockout_status(&email)
                .await
                .unwrap()
                .failed_attempts,
            5,
            "the failed proof is durable even though the user lock write failed"
        );

        run_in_request(async {
            TwoFactor::start_challenge(&user_id, false)
                .await
                .expect("restart challenge for lock repair");
            let error = TwoFactor::complete_challenge("invalid-code")
                .await
                .expect_err("repaired locked account remains rejected");
            assert_eq!(error.status_code(), 429);
        })
        .await;
        assert_eq!(
            dispatched_count::<AccountLocked>(|event| event.email == email),
            1,
            "the rejected admission that wins the repair transition owns the event"
        );

        run_in_request(async {
            TwoFactor::start_challenge(&user_id, false)
                .await
                .expect("restart challenge after repair");
            let error = TwoFactor::complete_challenge("invalid-code")
                .await
                .expect_err("already-repaired locked account remains rejected");
            assert_eq!(error.status_code(), 429);
        })
        .await;
        assert_eq!(
            dispatched_count::<AccountLocked>(|event| event.email == email),
            1,
            "subsequent rejected admissions must not duplicate the lock event"
        );
    });
}

#[test]
fn threshold_reservation_with_valid_challenge_resets_without_lock_event() {
    Lazy::force(&SETUP);
    RT.block_on(async {
        let _serial = TEST_LOCK.lock().await;
        let _fake = EventFacade::fake();

        let (user_id, email, otpauth_url) = register_and_enroll("admission-valid-reset").await;
        for _ in 0..4 {
            BruteForce::record_failed_attempt(&email, None)
                .await
                .expect("seed failed attempt");
        }

        run_in_request(async {
            TwoFactor::start_challenge(&user_id, false)
                .await
                .expect("start_challenge");
            TwoFactor::complete_challenge(&totp_code_for(&otpauth_url))
                .await
                .expect("valid threshold reservation completes");
        })
        .await;

        assert_not_dispatched::<AccountLocked>(|event| event.email == email);
        assert_eq!(
            BruteForce::get_lockout_status(&email)
                .await
                .unwrap()
                .failed_attempts,
            0
        );
    });
}

#[test]
fn complete_challenge_with_bad_code_dispatches_failed_event_and_no_login() {
    Lazy::force(&SETUP);
    RT.block_on(async {
        let _serial = TEST_LOCK.lock().await;
        let _fake = EventFacade::fake();

        let (user_id, _email, _otpauth_url) = register_and_enroll("failed-event").await;
        let captured_user_id = user_id.clone();

        run_in_request(async {
            TwoFactor::start_challenge(&user_id, false)
                .await
                .expect("start_challenge");
            let err = TwoFactor::complete_challenge("000000")
                .await
                .expect_err("bad code must fail");
            assert_eq!(err.status_code(), 401);
        })
        .await;

        assert_dispatched::<TwoFactorChallengeFailed>(|e| e.user_id == captured_user_id);
        // The standard auth lifecycle events MUST NOT fire on a
        // failed challenge - listeners would otherwise see a "Login"
        // for a user who never actually got in.
        assert_not_dispatched::<Login>(|_| true);
        assert_not_dispatched::<Authenticated>(|_| true);
        assert_not_dispatched::<TwoFactorChallenged>(|_| true);
    });
}

#[test]
fn complete_challenge_rejects_locked_account_without_checking_code() {
    Lazy::force(&SETUP);
    RT.block_on(async {
        let _serial = TEST_LOCK.lock().await;
        let _fake = EventFacade::fake();

        let (user_id, email, otpauth_url) = register_and_enroll("locked").await;
        let captured_user_id = user_id.clone();

        // Drive the failed-attempt counter past the default threshold
        // (5) so the account is genuinely locked. Mirrors the lockout
        // setup pattern in `tests/brute_force.rs`.
        for _ in 0..6 {
            BruteForce::record_failed_attempt(&email, None)
                .await
                .expect("record_failed_attempt");
        }
        assert!(
            BruteForce::is_locked(&email).await.unwrap(),
            "lockout precondition: account must be locked before complete_challenge"
        );

        // Even the CORRECT TOTP code must be rejected with 429 - a
        // locked account cannot bypass the lockout by submitting the
        // right code. This is the symmetric counterpart of the
        // password path's `LoginThrottleMiddleware` gate.
        run_in_request(async {
            TwoFactor::start_challenge(&user_id, false)
                .await
                .expect("start_challenge");
            let valid_totp = totp_code_for(&otpauth_url);
            let err = TwoFactor::complete_challenge(&valid_totp)
                .await
                .expect_err("locked account must be rejected");
            assert_eq!(
                err.status_code(),
                429,
                "locked-account rejection must be 429 Too Many Requests, not 401"
            );
        })
        .await;

        assert_dispatched::<TwoFactorChallengeFailed>(|e| e.user_id == captured_user_id);
        assert_not_dispatched::<Login>(|_| true);
        assert_not_dispatched::<Authenticated>(|_| true);
        assert_not_dispatched::<TwoFactorChallenged>(|_| true);
    });
}
