//! End-to-end tests for remember-me cookie flow (codex review finding #13).
//!
//! Covers:
//!
//! 1. `login_remember_issues_cookie_and_persists_token` - issuing a
//!    token writes a hashed row and the middleware emits an encrypted
//!    `remember_me` cookie.
//! 2. `remember_cookie_authenticates_after_session_expiry` - when the
//!    session cookie is absent, a valid `remember_me` cookie hydrates
//!    the session through verify_and_rotate.
//! 3. `remember_cookie_rotates_on_use` - a successful verify deletes
//!    the matched row and issues a fresh one; the old cookie cannot
//!    authenticate twice.
//! 4. `revoke_remember_tokens_clears_all_rows_for_user` - calling the
//!    revoke helper deletes every row for the user (multi-device
//!    "log out everywhere").
//! 5. `expired_token_rejected_and_cleaned_up_by_prune` - `expires_at`
//!    in the past never authenticates and is removed by `prune_expired`.
//! 6. `forged_cookie_does_not_authenticate` - a random plaintext does
//!    not match any hashed row; verify returns None.
//!
//! # Harness
//!
//! - One tokio `Runtime` (`RT`) shared across the binary; the SQLx
//!   pool is bound to the runtime that created it (mirrors
//!   `magnetar_integration.rs`).
//! - `LocalMigrator` materialises only the `remember_tokens` and
//!   `sessions` tables - `Auth::login_remember` writes to one and the
//!   middleware reads from the other. We do not need users/magnetar to
//!   exercise the remember-me path; remember-me operates on an opaque
//!   `user_id: String`.
//! - `Crypt` is installed once via `_test_install_key` (the test-only
//!   helper exposed at `framework/src/crypto/mod.rs`). The `OnceLock`
//!   is process-wide so subsequent calls are silent no-ops.

use once_cell::sync::Lazy;
use sea_orm_migration::MigratorTrait;
use sea_orm_migration::prelude::*;
#[cfg(feature = "testing")]
use std::any::Any;
#[cfg(feature = "testing")]
use std::sync::Arc;
use tokio::runtime::Runtime;

#[cfg(feature = "testing")]
use suprnova::auth::request_state;
#[cfg(feature = "testing")]
use suprnova::http::cookie::Cookie;
use suprnova::session::SessionConfig;
#[cfg(feature = "testing")]
use suprnova::{
    Auth, Authenticatable, Credentials, FrameworkError, Guard, SessionGuard, StatefulGuard,
    UserProvider,
};

/// Shared runtime - SQLx pools die with their creating runtime.
static RT: Lazy<Runtime> = Lazy::new(|| Runtime::new().expect("tokio runtime"));

/// One-shot setup: install Crypt, build a shared in-memory SQLite
/// connection registered in the global App container, run the local
/// migrator. All tests reuse the same DB; each test inserts under a
/// unique `user_id` to avoid cross-test interference on the verify
/// scan.
///
/// We bypass `TestDatabase` because it registers the connection in a
/// thread-local `TestContainer`. cargo test spreads tests across
/// worker threads, so a thread-local registration is invisible to
/// every test except the one that wrote it. Registering directly in
/// `App::singleton` (process-global, RwLock-backed) makes the
/// connection visible to all worker threads.
static SETUP: Lazy<()> = Lazy::new(|| {
    // Install Crypt with a fresh key. `_test_install_key` is
    // idempotent - returns false if a key already exists, which is
    // fine.
    #[cfg(feature = "testing")]
    {
        let key = suprnova::EncryptionKey::generate();
        let _ = suprnova::crypto::_test_install_key(key);
    }

    RT.block_on(async {
        let config = suprnova::database::DatabaseConfig::builder()
            .url("sqlite::memory:")
            .max_connections(1)
            .min_connections(1)
            .logging(false)
            .build();
        let conn = suprnova::database::DbConnection::connect(&config)
            .await
            .expect("connect in-memory sqlite");
        // Migrate before publishing - every test reads through
        // `DB::connection()` and assumes the tables already exist.
        LocalMigrator::up(conn.inner(), None)
            .await
            .expect("run local migrator");
        // Publish to the process-global App container. `App::resolve`
        // and `DB::connection` will return this connection from every
        // worker thread.
        suprnova::App::singleton(conn);
    });
});

/// Local migrator: just the `sessions` and `remember_tokens` tables.
/// The framework's auth/remember code does not need anything else.
struct LocalMigrator;

#[async_trait::async_trait]
impl MigratorTrait for LocalMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(CreateSessionsTable),
            Box::new(CreateRememberTokensTable),
        ]
    }
}

struct CreateSessionsTable;

impl MigrationName for CreateSessionsTable {
    fn name(&self) -> &str {
        "m20240101_000001_create_sessions_table"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateSessionsTable {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Sessions::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Sessions::Id)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Sessions::UserId).string().null())
                    .col(ColumnDef::new(Sessions::Payload).text().not_null())
                    .col(ColumnDef::new(Sessions::CsrfToken).string().not_null())
                    .col(
                        ColumnDef::new(Sessions::LastActivity)
                            .timestamp()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Sessions::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum Sessions {
    Table,
    Id,
    UserId,
    Payload,
    CsrfToken,
    LastActivity,
}

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
                    .name("idx_test_remember_tokens_selector")
                    .table(RememberTokens::Table)
                    .col(RememberTokens::Selector)
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

// Helpers

/// Count rows in `remember_tokens` for a specific `user_id`.
async fn count_tokens_for(user_id: &str) -> u64 {
    use sea_orm::ColumnTrait;
    use sea_orm::EntityTrait;
    use sea_orm::QueryFilter;
    let conn = suprnova::DB::connection().expect("db connection");
    suprnova::auth::remember::entity::Entity::find()
        .filter(suprnova::auth::remember::entity::Column::UserId.eq(user_id))
        .all(conn.inner())
        .await
        .expect("count tokens query")
        .len() as u64
}

/// Drive `fut` inside a fresh session-scope AND pending-cookies-scope.
/// Returns `(handler_result, captured_pending_cookies)`. The pending
/// cookies are what the session middleware would have attached to the
/// outgoing response.
#[cfg(feature = "testing")]
async fn run_in_request<F, T>(fut: F) -> (T, Vec<Cookie>)
where
    F: std::future::Future<Output = T>,
{
    let session_slot = suprnova::session::new_session_slot_for_test();
    let pending_slot = suprnova::session::new_pending_cookies_slot_for_test();
    let result = suprnova::session::session_scope_for_test(
        session_slot,
        suprnova::session::pending_cookies_scope_for_test(pending_slot.clone(), fut),
    )
    .await;
    let pending = std::mem::take(&mut *pending_slot.lock().unwrap());
    (result, pending)
}

#[cfg(feature = "testing")]
#[derive(Clone)]
struct NamedRememberUser {
    id: String,
}

#[cfg(feature = "testing")]
impl Authenticatable for NamedRememberUser {
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

#[cfg(feature = "testing")]
struct NamedRememberProvider {
    id: &'static str,
}

#[cfg(feature = "testing")]
struct RequestOverrideProvider {
    persisted_id: &'static str,
    override_id: &'static str,
}

#[cfg(feature = "testing")]
#[async_trait::async_trait]
impl UserProvider for NamedRememberProvider {
    async fn retrieve_by_id(
        &self,
        id: &str,
    ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        Ok((id == self.id).then(|| {
            Arc::new(NamedRememberUser {
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

#[cfg(feature = "testing")]
#[async_trait::async_trait]
impl UserProvider for RequestOverrideProvider {
    async fn retrieve_by_id(
        &self,
        id: &str,
    ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        Ok([self.persisted_id, self.override_id]
            .contains(&id)
            .then(|| Arc::new(NamedRememberUser { id: id.to_owned() }) as Arc<dyn Authenticatable>))
    }

    async fn retrieve_by_credentials(
        &self,
        credentials: &serde_json::Value,
    ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        Ok(
            (credentials.get("email").and_then(serde_json::Value::as_str)
                == Some("override@example.test"))
            .then(|| {
                Arc::new(NamedRememberUser {
                    id: self.override_id.to_owned(),
                }) as Arc<dyn Authenticatable>
            }),
        )
    }

    async fn validate_credentials(
        &self,
        _user: &dyn Authenticatable,
        credentials: &serde_json::Value,
    ) -> Result<bool, FrameworkError> {
        Ok(credentials
            .get("password")
            .and_then(serde_json::Value::as_str)
            == Some("secret"))
    }
}

#[cfg(feature = "testing")]
async fn request_with_cookies(cookies: &[(&str, &str)]) -> suprnova::Request {
    use bytes::Bytes;
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper_util::rt::TokioIo;
    use std::convert::Infallible;
    use tokio::io::AsyncWriteExt;
    use tokio::sync::oneshot;

    let mut http_bytes = Vec::new();
    http_bytes.extend_from_slice(b"GET / HTTP/1.1\r\n");
    http_bytes.extend_from_slice(b"Host: localhost\r\n");
    if !cookies.is_empty() {
        let cookie_header = cookies
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ");
        http_bytes.extend_from_slice(format!("Cookie: {cookie_header}\r\n").as_bytes());
    }
    http_bytes.extend_from_slice(b"Content-Length: 0\r\n\r\n");

    let (req_tx, req_rx) = oneshot::channel::<suprnova::Request>();
    let req_tx = std::sync::Mutex::new(Some(req_tx));
    let duplex_cap = http_bytes.len() + 64 * 1024;
    let (client_io, server_io) = tokio::io::duplex(duplex_cap);

    tokio::spawn(async move {
        let svc = service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
            let wrapped = suprnova::Request::new(req);
            if let Ok(mut guard) = req_tx.lock()
                && let Some(tx) = guard.take()
            {
                let _ = tx.send(wrapped);
            }
            async {
                std::future::pending::<()>().await;
                Ok::<_, Infallible>(hyper::Response::new(http_body_util::Empty::<Bytes>::new()))
            }
        });
        let _ = http1::Builder::new()
            .serve_connection(TokioIo::new(server_io), svc)
            .await;
    });

    {
        let mut client = client_io;
        client.write_all(&http_bytes).await.unwrap();
    }

    req_rx.await.expect("server received request")
}

#[cfg(feature = "testing")]
async fn request_with_remember_cookie(ciphertext: &str) -> suprnova::Request {
    request_with_cookies(&[("remember_me", ciphertext)]).await
}

#[cfg(feature = "testing")]
async fn observe_named_guard_ids_from_remember_cookie(
    ciphertext: &str,
    user_id: &'static str,
) -> (Option<String>, Option<String>, Option<String>) {
    use suprnova::middleware::Middleware;

    let request = request_with_remember_cookie(ciphertext).await;
    type Observation = (Option<String>, Option<String>, Option<String>);
    let observed = Arc::new(std::sync::Mutex::new(None::<Observation>));
    let observed_clone = observed.clone();
    let next: suprnova::middleware::Next = Arc::new(move |_req| {
        let observed = observed_clone.clone();
        Box::pin(async move {
            let web = SessionGuard::named("web", Arc::new(NamedRememberProvider { id: user_id }));
            let admin =
                SessionGuard::named("admin", Arc::new(NamedRememberProvider { id: user_id }));
            *observed.lock().unwrap() = Some((
                web.id().await.unwrap(),
                admin.id().await.unwrap(),
                Auth::id(),
            ));
            Ok(suprnova::HttpResponse::text("ok"))
        })
    });

    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    config.remember_lifetime = std::time::Duration::from_secs(60 * 60 * 24);
    let response = request_state::request_state_scope_for_test(
        suprnova::SessionMiddleware::new(config).handle(request, next),
    )
    .await;
    assert!(response.is_ok(), "middleware must continue to the handler");
    observed
        .lock()
        .unwrap()
        .clone()
        .expect("handler captured guard identities")
}

#[cfg(feature = "testing")]
struct IdentitySwitchBrowserOutcome {
    remember_set_cookie_count: usize,
    installed_remember_cookie: Option<String>,
    identity_after_session_expiry: Option<String>,
    previous_identity_rows: u64,
    fresh_identity_rows: u64,
}

#[cfg(feature = "testing")]
#[derive(Clone, Copy)]
enum IdentitySwitchPath {
    SessionGuard { remember: bool },
    AuthLoginId,
    AuthLoginRemember,
    AuthLoginIdThenIssueRemember,
}

#[cfg(feature = "testing")]
async fn exercise_session_guard_identity_switch(
    previous_user_id: &'static str,
    fresh_user_id: &'static str,
    path: IdentitySwitchPath,
) -> IdentitySwitchBrowserOutcome {
    use suprnova::middleware::Middleware;

    let ttl_minutes = 60 * 24;
    let previous_plaintext = suprnova::auth::remember::issue(previous_user_id, ttl_minutes)
        .await
        .expect("issue the previous identity's remember credential");
    let previous_cookie =
        suprnova::Crypt::encrypt_string(suprnova::CryptPurpose::Cookie, &previous_plaintext)
            .expect("encrypt the previous identity's browser carrier");

    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    config.remember_lifetime =
        std::time::Duration::from_secs((ttl_minutes as u64).saturating_mul(60));
    let middleware = suprnova::SessionMiddleware::new(config.clone());
    let switch_next: suprnova::middleware::Next = Arc::new(move |_request| {
        Box::pin(async move {
            assert_eq!(Auth::id().as_deref(), Some(previous_user_id));
            assert!(Auth::via_remember());

            match path {
                IdentitySwitchPath::SessionGuard { remember } => {
                    SessionGuard::named(
                        "web",
                        Arc::new(NamedRememberProvider { id: fresh_user_id }),
                    )
                    .with_remember_ttl(ttl_minutes)
                    .login(
                        Arc::new(NamedRememberUser {
                            id: fresh_user_id.to_owned(),
                        }) as Arc<dyn Authenticatable>,
                        remember,
                    )
                    .await
                    .expect("fresh SessionGuard login succeeds");
                }
                IdentitySwitchPath::AuthLoginId => {
                    Auth::login_id(fresh_user_id).expect("fresh Auth::login_id succeeds");
                }
                IdentitySwitchPath::AuthLoginRemember => {
                    Auth::login_remember(fresh_user_id, ttl_minutes)
                        .await
                        .expect("fresh Auth::login_remember succeeds");
                }
                IdentitySwitchPath::AuthLoginIdThenIssueRemember => {
                    Auth::login_id(fresh_user_id).expect("fresh Auth::login_id succeeds");
                    Auth::issue_remember_cookie(fresh_user_id, ttl_minutes)
                        .await
                        .expect("fresh Auth::issue_remember_cookie succeeds");
                }
            }
            assert_eq!(Auth::id().as_deref(), Some(fresh_user_id));
            assert!(!Auth::via_remember());
            Ok(suprnova::HttpResponse::text("switched"))
        })
    });
    let switch_response = match request_state::request_state_scope_for_test(middleware.handle(
        request_with_remember_cookie(&previous_cookie).await,
        switch_next,
    ))
    .await
    {
        Ok(response) => response.into_hyper(),
        Err(_) => panic!("identity-switch request must reach the handler"),
    };

    let remember_headers = switch_response
        .headers()
        .get_all("Set-Cookie")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .filter(|header| header.starts_with("remember_me="))
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let mut browser_remember_cookie = Some(previous_cookie);
    for header in &remember_headers {
        if header.contains("Max-Age=0") {
            browser_remember_cookie = None;
        } else {
            browser_remember_cookie = header
                .split(';')
                .next()
                .and_then(|pair| pair.strip_prefix("remember_me="))
                .map(ToOwned::to_owned);
        }
    }

    // Simulate expiry/eviction of B's data-session cookie. The next request
    // carries only what the browser jar retained from Set-Cookie processing.
    let replay_request = match browser_remember_cookie.as_deref() {
        Some(cookie) => request_with_remember_cookie(cookie).await,
        None => request_with_cookies(&[]).await,
    };
    let observed_after_expiry = Arc::new(std::sync::Mutex::new(None::<String>));
    let observed_in_handler = observed_after_expiry.clone();
    let replay_next: suprnova::middleware::Next = Arc::new(move |_request| {
        let observed = observed_in_handler.clone();
        Box::pin(async move {
            *observed.lock().unwrap() = Auth::id();
            Ok(suprnova::HttpResponse::text("replayed"))
        })
    });
    if request_state::request_state_scope_for_test(middleware.handle(replay_request, replay_next))
        .await
        .is_err()
    {
        panic!("post-expiry request must reach the handler");
    }
    let identity_after_session_expiry = observed_after_expiry.lock().unwrap().clone();

    IdentitySwitchBrowserOutcome {
        remember_set_cookie_count: remember_headers.len(),
        installed_remember_cookie: browser_remember_cookie,
        identity_after_session_expiry,
        previous_identity_rows: count_tokens_for(previous_user_id).await,
        fresh_identity_rows: count_tokens_for(fresh_user_id).await,
    }
}

/// Extract the raw selector+verifier credential from the versioned carrier
/// queued by `Auth::login_remember`.
#[cfg(feature = "testing")]
fn decode_remember_cookie(cookies: &[Cookie]) -> String {
    let cookie = cookies
        .iter()
        .find(|c| c.name() == suprnova::auth::remember::COOKIE_NAME)
        .expect("a remember_me cookie should have been queued");
    let carrier = Cookie::read_encrypted_for(suprnova::auth::remember::COOKIE_NAME, cookie.value())
        .expect("remember_me cookie must decrypt");
    decode_versioned_remember_carrier(&carrier).1
}

#[cfg(feature = "testing")]
fn decode_versioned_remember_carrier(carrier: &str) -> (String, String) {
    let encoded = carrier
        .strip_prefix("suprnova.remember.v1:")
        .expect("new remember cookies must carry the versioned guard envelope");
    let envelope: serde_json::Value =
        serde_json::from_str(encoded).expect("remember carrier must be valid JSON");
    let guard = envelope
        .get("guard")
        .and_then(serde_json::Value::as_str)
        .expect("remember carrier must name its guard")
        .to_owned();
    let credential = envelope
        .get("credential")
        .and_then(serde_json::Value::as_str)
        .expect("remember carrier must contain its credential")
        .to_owned();
    (guard, credential)
}

#[cfg(feature = "testing")]
fn remember_selector(credential: &str) -> String {
    credential
        .split_once('.')
        .map(|(selector, _)| selector.to_owned())
        .expect("issued remember credential must contain a selector")
}

#[cfg(feature = "testing")]
fn encrypted_guard_carrier(guard: &str, credential: &str) -> String {
    let envelope = serde_json::json!({
        "guard": guard,
        "credential": credential,
    });
    let plaintext = format!("suprnova.remember.v1:{envelope}");
    Cookie::encrypted(suprnova::auth::remember::COOKIE_NAME, &plaintext)
        .expect("encrypt versioned remember carrier")
        .value()
        .to_owned()
}

#[cfg(feature = "testing")]
fn session_with_remember_guards(
    session_byte: char,
    guards: &[(&str, &str, &str)],
) -> suprnova::session::SessionData {
    let mut session = suprnova::session::SessionData::new(
        session_byte.to_string().repeat(40),
        format!("remember-{session_byte}-csrf"),
    );
    let mut guard_state = serde_json::Map::new();
    for (guard, user_id, credential) in guards {
        guard_state.insert(
            (*guard).to_owned(),
            serde_json::json!({
                "id": user_id,
                "remember_selector": remember_selector(credential),
            }),
        );
        if *guard == "web" {
            session.user_id = Some((*user_id).to_owned());
        }
    }
    session.data.insert(
        "_auth_guards".to_owned(),
        serde_json::Value::Object(guard_state),
    );
    session
}

#[cfg(feature = "testing")]
async fn persist_session_cookie(
    middleware: &suprnova::SessionMiddleware,
    session: &suprnova::session::SessionData,
) -> String {
    middleware
        .store()
        .write(session)
        .await
        .expect("persist prepared remember session");
    suprnova::Crypt::encrypt_string(suprnova::CryptPurpose::Cookie, &session.id)
        .expect("encrypt prepared data-session cookie")
}

/// Insert a raw row directly into `remember_tokens` (bypassing
/// `issue`). Used for the expired-token scenario where we need a row
/// whose `expires_at` is in the past - `issue` always generates fresh
/// future-expiring rows.
async fn insert_raw_token(
    user_id: &str,
    selector: &str,
    token_hash: &str,
    expires_at: chrono::DateTime<chrono::Utc>,
) {
    use sea_orm::EntityTrait;
    use sea_orm::Set;
    let conn = suprnova::DB::connection().expect("db connection");
    let now = chrono::Utc::now();
    let model = suprnova::auth::remember::entity::ActiveModel {
        user_id: Set(user_id.to_string()),
        selector: Set(selector.to_string()),
        token_hash: Set(token_hash.to_string()),
        expires_at: Set(expires_at.naive_utc()),
        created_at: Set(now.naive_utc()),
        last_used_at: Set(None),
        ..Default::default()
    };
    suprnova::auth::remember::entity::Entity::insert(model)
        .exec(conn.inner())
        .await
        .expect("insert raw token");
}

// Tests

/// Test 1: `login_remember` writes a hashed row and queues an
/// encrypted `remember_me` cookie. The cookie is HttpOnly, has a
/// Max-Age, and its value is NOT the raw plaintext token (it's
/// encrypted under Crypt).
#[cfg(feature = "testing")]
#[test]
fn login_remember_issues_cookie_and_persists_token() {
    Lazy::force(&SETUP);

    RT.block_on(async {
        let user_id = "test-user-issue";
        let ttl_minutes: i64 = 60 * 24; // 1 day

        let (result, pending) =
            run_in_request(async { Auth::login_remember(user_id, ttl_minutes).await }).await;
        result.expect("login_remember should succeed");

        // Row inserted.
        let count = count_tokens_for(user_id).await;
        assert_eq!(count, 1, "exactly one remember_tokens row expected");

        // Cookie queued and decrypts to a composite "selector.verifier"
        // plaintext (22-char selector + '.' + 43-char verifier = 66
        // chars total).
        let plaintext = decode_remember_cookie(&pending);
        assert_eq!(
            decode_versioned_remember_carrier(
                &Cookie::read_encrypted_for(
                    suprnova::auth::remember::COOKIE_NAME,
                    pending
                        .iter()
                        .find(|cookie| cookie.name() == "remember_me")
                        .expect("remember cookie queued")
                        .value(),
                )
                .expect("remember cookie decrypts"),
            )
            .0,
            "web"
        );
        assert_eq!(
            plaintext.len(),
            66,
            "selector.verifier composite token expected"
        );
        let (sel, ver) = plaintext
            .split_once('.')
            .expect("composite token must contain a '.' separator");
        assert_eq!(sel.len(), 22, "selector is 22 base64 chars (16 bytes)");
        assert_eq!(ver.len(), 43, "verifier is 43 base64 chars (32 bytes)");

        let cookie = pending
            .iter()
            .find(|c| c.name() == "remember_me")
            .expect("remember_me cookie queued");

        // Wire-format value must NOT equal the plaintext - that would
        // mean we stored a bearer credential in cleartext.
        assert_ne!(
            cookie.value(),
            plaintext,
            "cookie value must be the encrypted blob, never the plaintext token"
        );

        let header = cookie.to_header_value();
        assert!(header.contains("HttpOnly"), "cookie must be HttpOnly");
        assert!(header.contains("SameSite=Lax"), "default SameSite=Lax");
        // Cookie's Max-Age must MATCH the row's TTL - codex finding
        // #13 required "expires-at matches token expiration." 1 day
        // = 86400 s.
        let expected_max_age = (ttl_minutes as u64) * 60;
        assert!(
            header.contains(&format!("Max-Age={expected_max_age}")),
            "Max-Age must match ttl_minutes -> seconds (expected {expected_max_age}), got: {header}"
        );
    });
}

/// Test 2: with no session active, a valid `remember_me` cookie
/// hydrates a new session and the response carries a freshly-rotated
/// cookie. Simulates the "browser was closed, session cookie evicted,
/// user returns" path.
#[test]
fn remember_cookie_authenticates_after_session_expiry() {
    Lazy::force(&SETUP);

    RT.block_on(async {
        let user_id = "test-user-reauth";
        let ttl_minutes: i64 = 60 * 24;

        // Step 1: issue a token directly (no session - simulating the
        // server-side state right after the original login_remember).
        let plaintext = suprnova::auth::remember::issue(user_id, ttl_minutes)
            .await
            .expect("issue token");
        assert_eq!(count_tokens_for(user_id).await, 1);

        // Step 2: drive the middleware path - verify_and_rotate is
        // what the middleware calls when the session is missing.
        let result = suprnova::auth::remember::verify_and_rotate(&plaintext, ttl_minutes)
            .await
            .expect("verify_and_rotate query");

        let (hydrated_user_id, new_plaintext) = result.expect("token should match");
        assert_eq!(hydrated_user_id, user_id);
        assert_eq!(
            count_tokens_for(user_id).await,
            1,
            "rotation: old row deleted + new row inserted = still 1"
        );

        // The new plaintext is different and itself a valid token.
        assert_ne!(new_plaintext, plaintext, "rotation must mint a new token");
        let third = suprnova::auth::remember::verify_and_rotate(&new_plaintext, ttl_minutes)
            .await
            .expect("verify new plaintext")
            .expect("new plaintext must verify");
        assert_eq!(third.0, user_id);
    });
}

/// Test 3 (rotation invariant): an already-used cookie cannot
/// authenticate again. The matched row is DELETED on first use; replay
/// returns `Ok(None)`.
#[test]
fn remember_cookie_rotates_on_use() {
    Lazy::force(&SETUP);

    RT.block_on(async {
        let user_id = "test-user-rotate";
        let ttl_minutes: i64 = 60 * 24;

        let plaintext_a = suprnova::auth::remember::issue(user_id, ttl_minutes)
            .await
            .expect("issue A");

        // First use: succeeds, mints plaintext_b.
        let (uid, plaintext_b) =
            suprnova::auth::remember::verify_and_rotate(&plaintext_a, ttl_minutes)
                .await
                .expect("verify A")
                .expect("A must match");
        assert_eq!(uid, user_id);
        assert_ne!(plaintext_a, plaintext_b);

        // Second use of plaintext_a: row gone, must NOT verify.
        let replay = suprnova::auth::remember::verify_and_rotate(&plaintext_a, ttl_minutes)
            .await
            .expect("verify A replay");
        assert!(
            replay.is_none(),
            "already-rotated token must not re-authenticate"
        );

        // plaintext_b is the new live token.
        let (uid_b, _) = suprnova::auth::remember::verify_and_rotate(&plaintext_b, ttl_minutes)
            .await
            .expect("verify B")
            .expect("B must match");
        assert_eq!(uid_b, user_id);
    });
}

/// Test 4: `revoke_all_for_user` deletes EVERY row for the user
/// (two-device scenario). Subsequent verify of either captured
/// plaintext fails.
#[test]
fn revoke_remember_tokens_clears_all_rows_for_user() {
    Lazy::force(&SETUP);

    RT.block_on(async {
        let user_id = "test-user-revoke";
        let ttl_minutes: i64 = 60 * 24;

        let pt1 = suprnova::auth::remember::issue(user_id, ttl_minutes)
            .await
            .expect("issue 1");
        let pt2 = suprnova::auth::remember::issue(user_id, ttl_minutes)
            .await
            .expect("issue 2");
        assert_eq!(count_tokens_for(user_id).await, 2);

        let removed = suprnova::auth::remember::revoke_all_for_user(user_id)
            .await
            .expect("revoke_all");
        assert_eq!(removed, 2, "both rows must be removed");
        assert_eq!(count_tokens_for(user_id).await, 0);

        assert!(
            suprnova::auth::remember::verify_and_rotate(&pt1, ttl_minutes)
                .await
                .expect("verify post-revoke pt1")
                .is_none()
        );
        assert!(
            suprnova::auth::remember::verify_and_rotate(&pt2, ttl_minutes)
                .await
                .expect("verify post-revoke pt2")
                .is_none()
        );
    });
}

/// Test 5: a token whose `expires_at` is already in the past must NOT
/// authenticate (verify filters on `expires_at > now`).
/// `prune_expired` then removes it.
#[test]
fn expired_token_rejected_and_cleaned_up_by_prune() {
    Lazy::force(&SETUP);

    RT.block_on(async {
        let user_id = "test-user-expired";
        let ttl_minutes: i64 = 60 * 24;

        // Generate a real (selector, verifier, hash) triple, but
        // insert with expires_at in the past. Bypasses `issue` (which
        // always uses now + TTL).
        let (selector, verifier, hash) = suprnova::auth::remember::generate_token()
            .await
            .expect("generate token");
        let composite = format!("{selector}.{verifier}");
        let past_expiry = chrono::Utc::now() - chrono::Duration::seconds(60);
        insert_raw_token(user_id, &selector, &hash, past_expiry).await;
        assert_eq!(count_tokens_for(user_id).await, 1);

        // Verify rejects expired rows up front (the WHERE expires_at > now
        // filter excludes them - they never reach the bcrypt compare).
        let result = suprnova::auth::remember::verify_and_rotate(&composite, ttl_minutes)
            .await
            .expect("verify expired");
        assert!(result.is_none(), "expired token must not authenticate");

        // Row is still there until pruned.
        assert_eq!(count_tokens_for(user_id).await, 1);

        let removed = suprnova::auth::remember::prune_expired()
            .await
            .expect("prune");
        assert!(
            removed >= 1,
            "prune must remove at least our expired row (removed={removed})"
        );
        assert_eq!(count_tokens_for(user_id).await, 0);
    });
}

/// Test 6: a forged plaintext that does not match any hashed row must
/// not authenticate. `verify_and_rotate` returns `Ok(None)` and no DB
/// rows change.
#[test]
fn forged_cookie_does_not_authenticate() {
    Lazy::force(&SETUP);

    RT.block_on(async {
        let user_id = "test-user-forged";
        let ttl_minutes: i64 = 60 * 24;

        // Issue one legitimate token so the verify scan has something
        // to compare against (proves the rejection isn't from an empty
        // table).
        let _legit = suprnova::auth::remember::issue(user_id, ttl_minutes)
            .await
            .expect("issue legit");
        let before = count_tokens_for(user_id).await;
        assert_eq!(before, 1);

        // A forged composite "<selector>.<verifier>" whose selector
        // cannot collide with any issued one (random uppercase F's are
        // not produced by URL_SAFE_NO_PAD base64).
        let forged = "FFFFFFFFFFFFFFFFFFFFFF.FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF";
        let result = suprnova::auth::remember::verify_and_rotate(forged, ttl_minutes)
            .await
            .expect("verify forged");
        assert!(result.is_none(), "forged token must not authenticate");

        // A malformed token (no '.' separator) must also reject without
        // any DB hit or bcrypt cost.
        let malformed = "noseparatorhere";
        let result = suprnova::auth::remember::verify_and_rotate(malformed, ttl_minutes)
            .await
            .expect("verify malformed");
        assert!(result.is_none(), "malformed token must not authenticate");

        // Row count unchanged - verify on a non-match must not mutate.
        assert_eq!(count_tokens_for(user_id).await, before);
    });
}

/// Test 6b (concurrency invariant): two concurrent verifications of
/// the SAME captured token must result in exactly one successful
/// rotation and one None. This proves the audit-fix is real - the
/// previous design could mint two replacement tokens for one captured
/// cookie (ChatGPT audit `auth` HIGH #1: "remember-me token rotation
/// is not single-use under concurrency").
#[test]
fn verify_and_rotate_is_single_use_under_concurrency() {
    Lazy::force(&SETUP);

    RT.block_on(async {
        let user_id = "test-user-race";
        let ttl_minutes: i64 = 60 * 24;

        // Issue one token; capture the plaintext that both racers will
        // attempt to verify simultaneously.
        let captured = suprnova::auth::remember::issue(user_id, ttl_minutes)
            .await
            .expect("issue captured");
        assert_eq!(count_tokens_for(user_id).await, 1);

        // Race: two concurrent verify_and_rotate calls against the same
        // plaintext. Whichever wins the DELETE rotates; the loser must
        // see rows_affected == 0 and return None - NOT mint a second
        // replacement.
        let c1 = captured.clone();
        let c2 = captured.clone();
        let (a, b) = tokio::join!(
            suprnova::auth::remember::verify_and_rotate(&c1, ttl_minutes),
            suprnova::auth::remember::verify_and_rotate(&c2, ttl_minutes),
        );
        let r1 = a.expect("racer 1 db ok");
        let r2 = b.expect("racer 2 db ok");

        // Exactly one Some, exactly one None.
        let success_count = [r1.is_some(), r2.is_some()].iter().filter(|x| **x).count();
        assert_eq!(
            success_count, 1,
            "exactly one racer must succeed; the other must return None - got r1={r1:?} r2={r2:?}"
        );

        // After the race, exactly ONE row exists for the user: the
        // winner's freshly-issued replacement. If the loser had also
        // minted a replacement (the pre-fix bug), we'd see 2 rows.
        assert_eq!(
            count_tokens_for(user_id).await,
            1,
            "single-use rotation: exactly one row survives the race"
        );
    });
}

/// Test 7: the middleware helper `create_forget_remember_cookie`
/// produces a Max-Age=0 cookie. Wired as a unit test here rather than
/// the e2e suite because it exercises the helper directly - no DB
/// needed.
#[test]
fn forget_remember_cookie_clears_the_cookie() {
    let config = SessionConfig::default();
    let clear = suprnova::session::middleware::create_forget_remember_cookie(&config);
    assert_eq!(clear.name(), "remember_me");
    let header = clear.to_header_value();
    assert!(
        header.contains("Max-Age=0"),
        "forget cookie must carry Max-Age=0"
    );
}

/// Test 8: `create_remember_cookie` respects
/// `SessionConfig::cookie_secure` - when secure=true the Set-Cookie
/// header carries the `Secure` attribute; when secure=false (local
/// dev), it doesn't.
#[cfg(feature = "testing")]
#[test]
fn remember_cookie_respects_secure_flag() {
    Lazy::force(&SETUP);

    let secure_config = SessionConfig::default(); // cookie_secure = true
    let plaintext = "any-encrypted-plaintext";
    let max_age = std::time::Duration::from_secs(60 * 60); // 1 hour
    let cookie =
        suprnova::session::middleware::create_remember_cookie(&secure_config, plaintext, max_age)
            .expect("encrypted cookie");
    let header = cookie.to_header_value();
    assert!(
        header.contains("Secure"),
        "production: cookie must be Secure"
    );
    assert!(header.contains("HttpOnly"));
    assert!(header.contains("SameSite=Lax"));
    assert!(
        header.contains("Max-Age=3600"),
        "max_age parameter must control Max-Age, got: {header}"
    );

    let mut insecure_config = SessionConfig::default();
    insecure_config.cookie_secure = false;
    let cookie =
        suprnova::session::middleware::create_remember_cookie(&insecure_config, plaintext, max_age)
            .expect("encrypted cookie");
    let header = cookie.to_header_value();
    assert!(
        !header.contains("Secure"),
        "local dev: Secure flag must be absent so cookies work over http"
    );
}

// ── End-to-end middleware test ────────────────────────────────────────

/// Test 9 (end-to-end): drive a real request through `SessionMiddleware`
/// carrying ONLY a `remember_me` cookie (no session cookie). The
/// middleware must:
///
/// 1. Decrypt the cookie and find the matching row.
/// 2. Rotate the token (delete + insert) so DB row count stays at 1.
/// 3. Hydrate the request-scoped session with the user's id so the
///    inner handler can call `Auth::id()` and observe the user.
/// 4. Attach a freshly-encrypted `remember_me` cookie to the response.
///
/// This binary intentionally installs no Magnetar engine: it pins the
/// compatibility fallback that may directly hydrate the legacy user id only
/// when the engine is absent. The installed-engine contract lives in
/// `magnetar_remember_middleware.rs`.
#[cfg(feature = "testing")]
#[test]
fn middleware_without_magnetar_engine_uses_legacy_remember_fallback() {
    use bytes::Bytes;
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper_util::rt::TokioIo;
    use std::convert::Infallible;
    use std::sync::Arc;
    use suprnova::Request;
    use suprnova::middleware::Middleware;
    use tokio::io::AsyncWriteExt;
    use tokio::sync::oneshot;

    Lazy::force(&SETUP);

    RT.block_on(async {
        let user_id = "test-user-middleware";
        let ttl_minutes: i64 = 60 * 24; // 1 day

        // Step 1: issue a token directly and encrypt the plaintext
        // into the wire format the middleware will receive.
        //
        // Compat-window regression: this still uses v1 `Crypt::encrypt_string`
        // with name-unbound AAD so middleware still accepts pre-upgrade cookies.
        let plaintext = suprnova::auth::remember::issue(user_id, ttl_minutes)
            .await
            .expect("issue token");
        let encrypted =
            suprnova::Crypt::encrypt_string(suprnova::CryptPurpose::Cookie, &plaintext)
                .expect("encrypt cookie");
        assert_eq!(
            count_tokens_for(user_id).await,
            1,
            "fixture: one row before middleware runs"
        );

        // Step 2: build a real `Request` carrying just the remember-me
        // cookie. Use the same duplex-pipe pattern as
        // `framework/tests/common.rs::request_from_http_bytes`,
        // inlined here so this test does not pull in `common.rs`
        // (which is module-private).
        let mut http_bytes = Vec::new();
        http_bytes.extend_from_slice(b"GET / HTTP/1.1\r\n");
        http_bytes.extend_from_slice(b"Host: localhost\r\n");
        http_bytes.extend_from_slice(
            format!("Cookie: remember_me={encrypted}\r\n").as_bytes(),
        );
        http_bytes.extend_from_slice(b"Content-Length: 0\r\n\r\n");

        let (req_tx, req_rx) = oneshot::channel::<Request>();
        let req_tx = std::sync::Mutex::new(Some(req_tx));
        let duplex_cap = http_bytes.len() + 64 * 1024;
        let (client_io, server_io) = tokio::io::duplex(duplex_cap);

        tokio::spawn(async move {
            let svc = service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
                let wrapped = Request::new(req);
                if let Ok(mut guard) = req_tx.lock()
                    && let Some(tx) = guard.take()
                {
                    let _ = tx.send(wrapped);
                }
                async {
                    std::future::pending::<()>().await;
                    Ok::<_, Infallible>(hyper::Response::new(
                        http_body_util::Empty::<Bytes>::new(),
                    ))
                }
            });
            let _ = http1::Builder::new()
                .serve_connection(TokioIo::new(server_io), svc)
                .await;
        });

        {
            let mut client = client_io;
            client.write_all(&http_bytes).await.unwrap();
        }
        let request = req_rx.await.expect("server received request");

        // Step 3: build a tiny handler that captures `Auth::id()` -
        // proof that the middleware hydrated the session before
        // calling next.
        let observed = Arc::new(std::sync::Mutex::new(None::<String>));
        let observed_clone = observed.clone();
        let next: suprnova::middleware::Next = Arc::new(move |_req| {
            let observed = observed_clone.clone();
            Box::pin(async move {
                let id = suprnova::Auth::id();
                *observed.lock().unwrap() = id;
                Ok(suprnova::HttpResponse::text("ok"))
            })
        });

        // Step 4: run the middleware. Use `cookie_secure(false)` so
        // we don't have to think about HTTPS in the test.
        let mut config = SessionConfig::default();
        config.cookie_secure = false;
        config.remember_lifetime =
            std::time::Duration::from_secs((ttl_minutes as u64) * 60);
        let middleware = suprnova::SessionMiddleware::new(config);
        let response = middleware.handle(request, next).await;

        // Step 5: handler must have observed the hydrated user id.
        let captured = observed.lock().unwrap().clone();
        assert_eq!(
            captured.as_deref(),
            Some(user_id),
            "middleware must hydrate the session BEFORE calling next"
        );

        // Step 6: rotation invariant - still exactly one row for the
        // user (old row deleted, new row inserted).
        assert_eq!(
            count_tokens_for(user_id).await,
            1,
            "rotation: old row deleted + new row inserted = still 1"
        );

        // Step 7: response carries a fresh remember_me cookie whose
        // ciphertext is different from the inbound one (verifying we
        // rotated, not just echoed the input back). `HttpResponse`
        // does not expose its headers directly - go through
        // `into_hyper()` which gives access to `hyper::HeaderMap`.
        let response = match response {
            Ok(r) => r,
            Err(_) => panic!("middleware should not short-circuit the request"),
        };
        let hyper_resp = response.into_hyper();
        let remember_cookies: Vec<String> = hyper_resp
            .headers()
            .get_all("Set-Cookie")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .filter(|c| c.starts_with("remember_me="))
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            remember_cookies.len(),
            1,
            "exactly one rotated remember_me cookie expected, got: {remember_cookies:?}"
        );

        let rotated_header = &remember_cookies[0];
        // Extract the cookie value: "remember_me=<value>; Path=...".
        let value_segment = rotated_header
            .split(';')
            .next()
            .expect("at least one segment");
        let new_ciphertext = value_segment
            .strip_prefix("remember_me=")
            .expect("starts with remember_me=");
        assert_ne!(
            new_ciphertext, encrypted,
            "rotated cookie must carry a different ciphertext than the input"
        );

        // Rotated cookie's Max-Age must match the new row's TTL so
        // the browser stops sending the cookie when the row expires.
        let expected_max_age = (ttl_minutes as u64) * 60;
        assert!(
            rotated_header.contains(&format!("Max-Age={expected_max_age}")),
            "rotated cookie's Max-Age must match the TTL (expected {expected_max_age}), got: {rotated_header}"
        );

        // The rotated cookie's plaintext must verify against the live
        // row (the post-rotation row). It is a v2 cookie, so decrypt
        // through the remember-me logical name.
        let rotated_carrier =
            Cookie::read_encrypted_for(suprnova::auth::remember::COOKIE_NAME, new_ciphertext)
                .expect("rotated cookie must decrypt");
        let (rotated_guard, rotated_plaintext) =
            decode_versioned_remember_carrier(&rotated_carrier);
        assert_eq!(rotated_guard, "web");
        let third =
            suprnova::auth::remember::verify_and_rotate(&rotated_plaintext, ttl_minutes)
                .await
                .expect("verify rotated plaintext")
                .expect("rotated plaintext must match the live row");
        assert_eq!(third.0, user_id);
    });
}

#[cfg(feature = "testing")]
#[test]
fn session_guard_identity_switch_replaces_prior_browser_carrier() {
    Lazy::force(&SETUP);

    RT.block_on(async {
        let without_remember = exercise_session_guard_identity_switch(
            "test-user-switch-old-session-only",
            "test-user-switch-fresh-session-only",
            IdentitySwitchPath::SessionGuard { remember: false },
        )
        .await;
        let with_remember = exercise_session_guard_identity_switch(
            "test-user-switch-old-remembered",
            "test-user-switch-fresh-remembered",
            IdentitySwitchPath::SessionGuard { remember: true },
        )
        .await;

        assert_eq!(
            without_remember.remember_set_cookie_count, 1,
            "fresh session-only login must replace A's queued carrier with one forget directive"
        );
        assert_eq!(without_remember.installed_remember_cookie, None);
        assert_eq!(without_remember.identity_after_session_expiry, None);
        assert_eq!(
            without_remember.previous_identity_rows, 0,
            "the async SessionGuard path must exact-revoke A's verified selector"
        );
        assert_eq!(without_remember.fresh_identity_rows, 0);

        assert_eq!(
            with_remember.remember_set_cookie_count, 1,
            "fresh remembered login must atomically replace A's queued carrier with B's"
        );
        assert!(with_remember.installed_remember_cookie.is_some());
        assert_eq!(
            with_remember.identity_after_session_expiry.as_deref(),
            Some("test-user-switch-fresh-remembered"),
            "after B's data session expires, the browser may remember B but never A"
        );
        assert_eq!(with_remember.previous_identity_rows, 0);
        assert_eq!(with_remember.fresh_identity_rows, 1);
    });
}

#[cfg(feature = "testing")]
#[test]
fn auth_facade_login_id_identity_switch_replaces_prior_browser_carrier() {
    Lazy::force(&SETUP);

    RT.block_on(async {
        let outcome = exercise_session_guard_identity_switch(
            "test-user-auth-login-id-old",
            "test-user-auth-login-id-fresh",
            IdentitySwitchPath::AuthLoginId,
        )
        .await;

        assert_eq!(outcome.remember_set_cookie_count, 1);
        assert_eq!(outcome.installed_remember_cookie, None);
        assert_eq!(outcome.identity_after_session_expiry, None);
        assert_eq!(outcome.previous_identity_rows, 0);
        assert_eq!(outcome.fresh_identity_rows, 0);
    });
}

#[cfg(feature = "testing")]
#[test]
fn auth_facade_login_remember_identity_switch_replaces_prior_browser_carrier() {
    Lazy::force(&SETUP);

    RT.block_on(async {
        let outcome = exercise_session_guard_identity_switch(
            "test-user-auth-login-remember-old",
            "test-user-auth-login-remember-fresh",
            IdentitySwitchPath::AuthLoginRemember,
        )
        .await;

        assert_eq!(outcome.remember_set_cookie_count, 1);
        assert!(outcome.installed_remember_cookie.is_some());
        assert_eq!(
            outcome.identity_after_session_expiry.as_deref(),
            Some("test-user-auth-login-remember-fresh")
        );
        assert_eq!(outcome.previous_identity_rows, 0);
        assert_eq!(outcome.fresh_identity_rows, 1);
    });
}

#[cfg(feature = "testing")]
#[test]
fn auth_facade_issue_remember_cookie_replaces_prior_browser_carrier() {
    Lazy::force(&SETUP);

    RT.block_on(async {
        let outcome = exercise_session_guard_identity_switch(
            "test-user-auth-issue-remember-old",
            "test-user-auth-issue-remember-fresh",
            IdentitySwitchPath::AuthLoginIdThenIssueRemember,
        )
        .await;

        assert_eq!(outcome.remember_set_cookie_count, 1);
        assert!(outcome.installed_remember_cookie.is_some());
        assert_eq!(
            outcome.identity_after_session_expiry.as_deref(),
            Some("test-user-auth-issue-remember-fresh")
        );
        assert_eq!(outcome.previous_identity_rows, 0);
        assert_eq!(outcome.fresh_identity_rows, 1);
    });
}

#[cfg(feature = "testing")]
#[test]
fn named_guard_remember_carrier_hydrates_only_encoded_guard() {
    use suprnova::middleware::Middleware;

    Lazy::force(&SETUP);

    RT.block_on(async {
        let user_id = "test-user-named-admin";
        let ttl_minutes: i64 = 60 * 24;
        let admin = SessionGuard::named("admin", Arc::new(NamedRememberProvider { id: user_id }))
            .with_remember_ttl(ttl_minutes);
        let user = Arc::new(NamedRememberUser {
            id: user_id.to_string(),
        }) as Arc<dyn Authenticatable>;

        let (result, pending) = run_in_request(request_state::request_state_scope_for_test(
            admin.login(user, true),
        ))
        .await;
        result.expect("named guard remember login should succeed");
        let remember_cookie = pending
            .iter()
            .find(|cookie| cookie.name() == suprnova::auth::remember::COOKIE_NAME)
            .expect("named guard login must queue a remember cookie");
        let request = request_with_remember_cookie(remember_cookie.value()).await;

        type Observation = (
            Option<String>,
            Option<String>,
            Option<String>,
            bool,
            bool,
            bool,
        );
        let observed = Arc::new(std::sync::Mutex::new(None::<Observation>));
        let observed_clone = observed.clone();
        let next: suprnova::middleware::Next = Arc::new(move |_req| {
            let observed = observed_clone.clone();
            Box::pin(async move {
                let web =
                    SessionGuard::named("web", Arc::new(NamedRememberProvider { id: user_id }));
                let admin =
                    SessionGuard::named("admin", Arc::new(NamedRememberProvider { id: user_id }));
                let observation = (
                    web.id().await.unwrap(),
                    admin.id().await.unwrap(),
                    Auth::id(),
                    web.via_remember(),
                    admin.via_remember(),
                    Auth::via_remember(),
                );
                *observed.lock().unwrap() = Some(observation);
                Ok(suprnova::HttpResponse::text("ok"))
            })
        });

        let mut config = SessionConfig::default();
        config.cookie_secure = false;
        config.remember_lifetime =
            std::time::Duration::from_secs((ttl_minutes as u64).saturating_mul(60));
        let middleware = suprnova::SessionMiddleware::new(config);
        let response =
            request_state::request_state_scope_for_test(middleware.handle(request, next)).await;
        assert!(response.is_ok(), "middleware must continue to the handler");

        let observation = observed
            .lock()
            .unwrap()
            .clone()
            .expect("handler captured guard attribution");
        assert_eq!(observation.0, None, "web guard must stay unauthenticated");
        assert_eq!(observation.1.as_deref(), Some(user_id));
        assert_eq!(
            observation.2, None,
            "generic Auth identity must remain the default web guard"
        );
        assert!(
            !observation.3,
            "web guard must not inherit remember provenance"
        );
        assert!(observation.4, "admin guard must record remember provenance");
        assert!(
            !observation.5,
            "generic Auth remember provenance belongs to the default guard only"
        );
    });
}

#[cfg(feature = "testing")]
#[test]
fn named_guard_remember_logout_revokes_only_its_selector() {
    use suprnova::middleware::Middleware;

    Lazy::force(&SETUP);

    RT.block_on(async {
        let user_id = "test-user-named-selector-logout";
        let ttl_minutes: i64 = 60 * 24;
        let web_credential = suprnova::auth::remember::issue(user_id, ttl_minutes)
            .await
            .expect("issue web remember credential");
        let admin_credential = suprnova::auth::remember::issue(user_id, ttl_minutes)
            .await
            .expect("issue admin remember credential");
        let session = session_with_remember_guards(
            'e',
            &[
                ("web", user_id, &web_credential),
                ("admin", user_id, &admin_credential),
            ],
        );
        let web_carrier = encrypted_guard_carrier("web", &web_credential);
        let admin_carrier = encrypted_guard_carrier("admin", &admin_credential);
        assert_eq!(count_tokens_for(user_id).await, 2);

        let mut config = SessionConfig::default();
        config.cookie_secure = false;
        config.remember_lifetime =
            std::time::Duration::from_secs((ttl_minutes as u64).saturating_mul(60));
        let middleware = suprnova::SessionMiddleware::new(config.clone());
        let session_cookie = persist_session_cookie(&middleware, &session).await;
        let request = request_with_cookies(&[
            (&config.cookie_name, &session_cookie),
            (suprnova::auth::remember::COOKIE_NAME, &admin_carrier),
        ])
        .await;
        let next: suprnova::middleware::Next = Arc::new(move |_request| {
            Box::pin(async move {
                let web =
                    SessionGuard::named("web", Arc::new(NamedRememberProvider { id: user_id }))
                        .with_remember_ttl(ttl_minutes);
                let admin =
                    SessionGuard::named("admin", Arc::new(NamedRememberProvider { id: user_id }))
                        .with_remember_ttl(ttl_minutes);
                admin.logout().await.expect("named logout succeeds");

                assert!(admin.guest().await.expect("read admin guest state"));
                assert_eq!(
                    web.id().await.expect("read surviving web guard").as_deref(),
                    Some(user_id)
                );
                assert_eq!(Auth::id().as_deref(), Some(user_id));
                Ok(suprnova::HttpResponse::text("logged out"))
            })
        });
        let response =
            request_state::request_state_scope_for_test(middleware.handle(request, next)).await;
        assert!(response.is_ok(), "named remember logout must succeed");
        assert_eq!(
            count_tokens_for(user_id).await,
            1,
            "named logout must revoke only its active selector"
        );

        let revoked_admin =
            observe_named_guard_ids_from_remember_cookie(&admin_carrier, user_id).await;
        assert_eq!(
            revoked_admin,
            (None, None, None),
            "the logged-out guard's selector must no longer authenticate"
        );

        let surviving_web =
            observe_named_guard_ids_from_remember_cookie(&web_carrier, user_id).await;
        assert_eq!(surviving_web.0.as_deref(), Some(user_id));
        assert_eq!(surviving_web.1, None);
        assert_eq!(surviving_web.2.as_deref(), Some(user_id));
    });
}

#[cfg(feature = "testing")]
#[test]
fn named_logout_does_not_revoke_an_unverified_other_user_carrier() {
    use suprnova::middleware::Middleware;

    Lazy::force(&SETUP);

    RT.block_on(async {
        let carrier_owner_id = "test-user-unverified-carrier-owner";
        let authenticated_user_id = "test-user-unverified-carrier-logout";
        let ttl_minutes: i64 = 60 * 24;

        let (result, pending) =
            run_in_request(request_state::request_state_scope_for_test(async {
                let admin = SessionGuard::named(
                    "admin",
                    Arc::new(NamedRememberProvider {
                        id: carrier_owner_id,
                    }),
                )
                .with_remember_ttl(ttl_minutes);
                admin
                    .login(
                        Arc::new(NamedRememberUser {
                            id: carrier_owner_id.to_owned(),
                        }) as Arc<dyn Authenticatable>,
                        true,
                    )
                    .await
            }))
            .await;
        result.expect("carrier owner remember login succeeds");
        let other_user_carrier = pending
            .iter()
            .find(|cookie| cookie.name() == suprnova::auth::remember::COOKIE_NAME)
            .map(|cookie| cookie.value().to_owned())
            .expect("carrier owner login queues a remember cookie");

        let (authenticated_session, _) =
            run_in_request(request_state::request_state_scope_for_test(async {
                let admin = SessionGuard::named(
                    "admin",
                    Arc::new(NamedRememberProvider {
                        id: authenticated_user_id,
                    }),
                );
                admin
                    .login(
                        Arc::new(NamedRememberUser {
                            id: authenticated_user_id.to_owned(),
                        }) as Arc<dyn Authenticatable>,
                        false,
                    )
                    .await
                    .expect("authenticated user session login succeeds");
                suprnova::session::session().expect("login leaves a session")
            }))
            .await;

        let mut config = SessionConfig::default();
        config.cookie_secure = false;
        config.remember_lifetime =
            std::time::Duration::from_secs((ttl_minutes as u64).saturating_mul(60));
        let middleware = suprnova::SessionMiddleware::new(config.clone());
        middleware
            .store()
            .write(&authenticated_session)
            .await
            .expect("persist authenticated user's session");
        let session_cookie = suprnova::Crypt::encrypt_string(
            suprnova::CryptPurpose::Cookie,
            &authenticated_session.id,
        )
        .expect("encrypt authenticated user's data-session cookie");
        let request = request_with_cookies(&[
            (&config.cookie_name, &session_cookie),
            (suprnova::auth::remember::COOKIE_NAME, &other_user_carrier),
        ])
        .await;
        let next: suprnova::middleware::Next = Arc::new(move |_request| {
            Box::pin(async move {
                SessionGuard::named(
                    "admin",
                    Arc::new(NamedRememberProvider {
                        id: authenticated_user_id,
                    }),
                )
                .logout()
                .await
                .expect("authenticated user's named logout succeeds");
                Ok(suprnova::HttpResponse::text("logged out"))
            })
        });
        let response =
            match request_state::request_state_scope_for_test(middleware.handle(request, next))
                .await
            {
                Ok(response) => response,
                Err(_) => panic!("logout request must reach the handler"),
            };

        let cleared = response
            .into_hyper()
            .headers()
            .get_all("Set-Cookie")
            .iter()
            .filter_map(|value| value.to_str().ok())
            .any(|header| header.starts_with("remember_me=") && header.contains("Max-Age=0"));
        assert!(cleared, "logout must clear the presented browser carrier");
        assert_eq!(
            count_tokens_for(carrier_owner_id).await,
            1,
            "logout must not delete a selector owned by a different user"
        );
        assert_eq!(
            observe_named_guard_ids_from_remember_cookie(&other_user_carrier, carrier_owner_id,)
                .await,
            (None, Some(carrier_owner_id.to_owned()), None),
            "storage must retain the unverified other-user carrier"
        );
    });
}

#[cfg(feature = "testing")]
#[test]
fn named_guard_logout_preserves_newer_sibling_carrier() {
    Lazy::force(&SETUP);

    RT.block_on(async {
        let admin_user_id = "test-user-carrier-admin";
        let web_user_id = "test-user-carrier-web";
        let ttl_minutes: i64 = 60 * 24;
        let (result, pending) =
            run_in_request(request_state::request_state_scope_for_test(async {
                let admin = SessionGuard::named(
                    "admin",
                    Arc::new(NamedRememberProvider { id: admin_user_id }),
                )
                .with_remember_ttl(ttl_minutes);
                let web =
                    SessionGuard::named("web", Arc::new(NamedRememberProvider { id: web_user_id }))
                        .with_remember_ttl(ttl_minutes);
                let remembered_user = |id: &str| {
                    Arc::new(NamedRememberUser { id: id.to_owned() }) as Arc<dyn Authenticatable>
                };

                admin.login(remembered_user(admin_user_id), true).await?;
                web.login(remembered_user(web_user_id), true).await?;
                admin.logout().await?;

                assert!(admin.guest().await?);
                assert_eq!(web.id().await?.as_deref(), Some(web_user_id));
                Ok::<(), FrameworkError>(())
            }))
            .await;
        result.expect("reverse-order named remember lifecycle should succeed");

        let remember_cookies = pending
            .iter()
            .filter(|cookie| cookie.name() == suprnova::auth::remember::COOKIE_NAME)
            .collect::<Vec<_>>();
        assert_eq!(
            remember_cookies.len(),
            1,
            "the single browser slot must retain only web's newer carrier"
        );
        assert_eq!(count_tokens_for(admin_user_id).await, 0);
        assert_eq!(count_tokens_for(web_user_id).await, 1);
        assert_eq!(
            observe_named_guard_ids_from_remember_cookie(remember_cookies[0].value(), web_user_id,)
                .await,
            (
                Some(web_user_id.to_owned()),
                None,
                Some(web_user_id.to_owned())
            ),
            "the sibling's effective outbound carrier must remain usable"
        );
    });
}

#[cfg(feature = "testing")]
#[test]
fn named_logout_revokes_persisted_and_active_same_guard_selectors() {
    use suprnova::middleware::Middleware;

    Lazy::force(&SETUP);

    RT.block_on(async {
        let user_id = "test-user-same-guard-selector-mismatch";
        let ttl_minutes: i64 = 60 * 24;
        let retained_credential = suprnova::auth::remember::issue(user_id, ttl_minutes)
            .await
            .expect("issue retained remember credential");
        let presented_credential = suprnova::auth::remember::issue(user_id, ttl_minutes)
            .await
            .expect("issue presented remember credential");
        let session =
            session_with_remember_guards('f', &[("admin", user_id, &retained_credential)]);
        let older_carrier = encrypted_guard_carrier("admin", &presented_credential);
        assert_eq!(count_tokens_for(user_id).await, 2);

        let mut config = SessionConfig::default();
        config.cookie_secure = false;
        config.remember_lifetime =
            std::time::Duration::from_secs((ttl_minutes as u64).saturating_mul(60));
        let middleware = suprnova::SessionMiddleware::new(config.clone());
        let session_cookie = persist_session_cookie(&middleware, &session).await;
        let request = request_with_cookies(&[
            (&config.cookie_name, &session_cookie),
            (suprnova::auth::remember::COOKIE_NAME, &older_carrier),
        ])
        .await;
        let next: suprnova::middleware::Next = Arc::new(move |_request| {
            Box::pin(async move {
                SessionGuard::named("admin", Arc::new(NamedRememberProvider { id: user_id }))
                    .logout()
                    .await
                    .expect("named logout succeeds");
                Ok(suprnova::HttpResponse::text("logged out"))
            })
        });
        let response =
            request_state::request_state_scope_for_test(middleware.handle(request, next)).await;
        assert!(response.is_ok(), "logout request must reach the handler");

        assert_eq!(
            count_tokens_for(user_id).await,
            0,
            "logout must revoke both the retained and actually presented selectors"
        );
        assert_eq!(
            observe_named_guard_ids_from_remember_cookie(&older_carrier, user_id).await,
            (None, None, None),
            "the presented older carrier must not survive logout"
        );
    });
}

#[cfg(feature = "testing")]
#[test]
fn logout_and_invalidate_revokes_named_guard_selectors() {
    use suprnova::middleware::Middleware;

    Lazy::force(&SETUP);

    RT.block_on(async {
        let web_user_id = "test-user-invalidate-web";
        let admin_user_id = "test-user-invalidate-admin";
        let ttl_minutes: i64 = 60 * 24;
        let web_credential = suprnova::auth::remember::issue(web_user_id, ttl_minutes)
            .await
            .expect("issue web remember credential");
        let admin_credential = suprnova::auth::remember::issue(admin_user_id, ttl_minutes)
            .await
            .expect("issue admin remember credential");
        let session = session_with_remember_guards(
            'g',
            &[
                ("web", web_user_id, &web_credential),
                ("admin", admin_user_id, &admin_credential),
            ],
        );
        let web_carrier = encrypted_guard_carrier("web", &web_credential);
        let admin_carrier = encrypted_guard_carrier("admin", &admin_credential);

        let mut config = SessionConfig::default();
        config.cookie_secure = false;
        config.remember_lifetime =
            std::time::Duration::from_secs((ttl_minutes as u64).saturating_mul(60));
        let middleware = suprnova::SessionMiddleware::new(config.clone());
        let session_cookie = persist_session_cookie(&middleware, &session).await;
        let request = request_with_cookies(&[
            (&config.cookie_name, &session_cookie),
            (suprnova::auth::remember::COOKIE_NAME, &admin_carrier),
        ])
        .await;
        let next: suprnova::middleware::Next = Arc::new(move |_request| {
            Box::pin(async move {
                Auth::logout_and_invalidate().await?;
                Ok(suprnova::HttpResponse::text("invalidated"))
            })
        });
        let response =
            request_state::request_state_scope_for_test(middleware.handle(request, next)).await;
        assert!(
            response.is_ok(),
            "full session invalidation should revoke every guard"
        );

        assert_eq!(count_tokens_for(web_user_id).await, 0);
        assert_eq!(
            count_tokens_for(admin_user_id).await,
            0,
            "full invalidation must revoke the named guard's retained selector"
        );
        assert_eq!(
            observe_named_guard_ids_from_remember_cookie(&admin_carrier, admin_user_id).await,
            (None, None, None),
            "a copied named carrier must not survive full invalidation"
        );
        assert_eq!(
            observe_named_guard_ids_from_remember_cookie(&web_carrier, web_user_id).await,
            (None, None, None),
            "the default guard carrier must not survive full invalidation"
        );
    });
}

#[cfg(feature = "testing")]
#[test]
fn logout_and_invalidate_revokes_persisted_and_active_named_selectors() {
    use suprnova::middleware::Middleware;

    Lazy::force(&SETUP);

    RT.block_on(async {
        let user_id = "test-user-full-invalidate-selector-mismatch";
        let ttl_minutes: i64 = 60 * 24;
        let retained_credential = suprnova::auth::remember::issue(user_id, ttl_minutes)
            .await
            .expect("issue retained remember credential");
        let presented_credential = suprnova::auth::remember::issue(user_id, ttl_minutes)
            .await
            .expect("issue presented remember credential");
        let session =
            session_with_remember_guards('h', &[("admin", user_id, &retained_credential)]);
        let older_carrier = encrypted_guard_carrier("admin", &presented_credential);
        assert_eq!(count_tokens_for(user_id).await, 2);

        let mut config = SessionConfig::default();
        config.cookie_secure = false;
        config.remember_lifetime =
            std::time::Duration::from_secs((ttl_minutes as u64).saturating_mul(60));
        let middleware = suprnova::SessionMiddleware::new(config.clone());
        let session_cookie = persist_session_cookie(&middleware, &session).await;
        let request = request_with_cookies(&[
            (&config.cookie_name, &session_cookie),
            (suprnova::auth::remember::COOKIE_NAME, &older_carrier),
        ])
        .await;
        let next: suprnova::middleware::Next = Arc::new(move |_request| {
            Box::pin(async move {
                Auth::logout_and_invalidate()
                    .await
                    .expect("full invalidation succeeds");
                Ok(suprnova::HttpResponse::text("invalidated"))
            })
        });
        let response =
            match request_state::request_state_scope_for_test(middleware.handle(request, next))
                .await
            {
                Ok(response) => response,
                Err(_) => panic!("invalidation request must reach the handler"),
            };
        let cleared = response
            .into_hyper()
            .headers()
            .get_all("Set-Cookie")
            .iter()
            .filter_map(|value| value.to_str().ok())
            .any(|header| header.starts_with("remember_me=") && header.contains("Max-Age=0"));
        assert!(cleared, "full invalidation must clear the active carrier");

        assert_eq!(
            count_tokens_for(user_id).await,
            0,
            "full invalidation must revoke both persisted and presented named selectors"
        );
        assert_eq!(
            observe_named_guard_ids_from_remember_cookie(&older_carrier, user_id).await,
            (None, None, None),
            "the presented older carrier must not survive full invalidation"
        );
    });
}

#[cfg(feature = "testing")]
#[test]
fn request_override_does_not_change_named_remember_revocation_owner() {
    use suprnova::middleware::Middleware;

    Lazy::force(&SETUP);

    RT.block_on(async {
        let default_owner_id = "test-user-request-override-default-owner";
        let default_override_id = "test-user-request-override-default-override";
        let ttl_minutes: i64 = 60 * 24;
        let (default_logout_result, _) =
            run_in_request(request_state::request_state_scope_for_test(async {
                let web = SessionGuard::named(
                    "web",
                    Arc::new(RequestOverrideProvider {
                        persisted_id: default_owner_id,
                        override_id: default_override_id,
                    }),
                )
                .with_remember_ttl(ttl_minutes);
                web.login(
                    Arc::new(NamedRememberUser {
                        id: default_owner_id.to_owned(),
                    }) as Arc<dyn Authenticatable>,
                    true,
                )
                .await?;
                suprnova::auth::remember::issue(default_override_id, ttl_minutes).await?;
                assert!(
                    web.once(&Credentials::password("override@example.test", "secret"))
                        .await?
                );
                assert_eq!(Auth::id().as_deref(), Some(default_override_id));

                web.logout().await
            }))
            .await;
        default_logout_result.expect("default logout after a once override succeeds");
        assert_eq!(
            count_tokens_for(default_owner_id).await,
            0,
            "default logout must revoke the persisted remembered owner"
        );
        assert_eq!(
            count_tokens_for(default_override_id).await,
            1,
            "default logout must not target a request-only once override"
        );

        let logout_owner_id = "test-user-request-override-logout-owner";
        let logout_override_id = "test-user-request-override-logout-override";
        let (logout_result, _) =
            run_in_request(request_state::request_state_scope_for_test(async {
                let admin = SessionGuard::named(
                    "admin",
                    Arc::new(NamedRememberProvider {
                        id: logout_owner_id,
                    }),
                )
                .with_remember_ttl(ttl_minutes);
                admin
                    .login(
                        Arc::new(NamedRememberUser {
                            id: logout_owner_id.to_owned(),
                        }) as Arc<dyn Authenticatable>,
                        true,
                    )
                    .await?;
                suprnova::auth::remember::issue(logout_override_id, ttl_minutes).await?;
                admin
                    .set_user(Arc::new(NamedRememberUser {
                        id: logout_override_id.to_owned(),
                    }))
                    .await;
                assert_eq!(admin.id().await?.as_deref(), Some(logout_override_id));

                admin.logout().await
            }))
            .await;
        logout_result.expect("named logout after a request-only override succeeds");
        assert_eq!(
            count_tokens_for(logout_owner_id).await,
            0,
            "named logout must revoke the persisted remembered owner's selector"
        );
        assert_eq!(
            count_tokens_for(logout_override_id).await,
            1,
            "named logout must not target a request-only override"
        );

        let invalidate_owner_id = "test-user-request-override-invalidate-owner";
        let invalidate_override_id = "test-user-request-override-invalidate-override";
        let retained_credential = suprnova::auth::remember::issue(invalidate_owner_id, ttl_minutes)
            .await
            .expect("issue retained owner credential");
        let presented_credential =
            suprnova::auth::remember::issue(invalidate_owner_id, ttl_minutes)
                .await
                .expect("issue presented owner credential");
        suprnova::auth::remember::issue(invalidate_override_id, ttl_minutes)
            .await
            .expect("issue request-only override sentinel");
        let session = session_with_remember_guards(
            'i',
            &[("admin", invalidate_owner_id, &retained_credential)],
        );
        let older_carrier = encrypted_guard_carrier("admin", &presented_credential);
        assert_eq!(count_tokens_for(invalidate_owner_id).await, 2);

        let mut config = SessionConfig::default();
        config.cookie_secure = false;
        config.remember_lifetime =
            std::time::Duration::from_secs((ttl_minutes as u64).saturating_mul(60));
        let middleware = suprnova::SessionMiddleware::new(config.clone());
        let session_cookie = persist_session_cookie(&middleware, &session).await;
        let request = request_with_cookies(&[
            (&config.cookie_name, &session_cookie),
            (suprnova::auth::remember::COOKIE_NAME, &older_carrier),
        ])
        .await;
        let next: suprnova::middleware::Next = Arc::new(move |_request| {
            Box::pin(async move {
                let admin = SessionGuard::named(
                    "admin",
                    Arc::new(NamedRememberProvider {
                        id: invalidate_owner_id,
                    }),
                );
                admin
                    .set_user(Arc::new(NamedRememberUser {
                        id: invalidate_override_id.to_owned(),
                    }))
                    .await;
                assert_eq!(
                    admin
                        .id()
                        .await
                        .expect("read request-only override")
                        .as_deref(),
                    Some(invalidate_override_id)
                );
                Auth::logout_and_invalidate()
                    .await
                    .expect("full invalidation succeeds");
                Ok(suprnova::HttpResponse::text("invalidated"))
            })
        });
        let response =
            request_state::request_state_scope_for_test(middleware.handle(request, next)).await;
        assert!(
            response.is_ok(),
            "invalidation request must reach the handler"
        );
        assert_eq!(
            count_tokens_for(invalidate_owner_id).await,
            0,
            "full invalidation must revoke both named selectors for the persisted owner"
        );
        assert_eq!(
            count_tokens_for(invalidate_override_id).await,
            1,
            "full invalidation must not target a request-only override"
        );
    });
}

#[cfg(feature = "testing")]
#[test]
fn full_invalidation_reports_ambiguous_selector_after_safe_teardown() {
    use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};

    Lazy::force(&SETUP);

    RT.block_on(async {
        let ambiguous_owner_id = "test-user-ambiguous-invalidation-owner";
        let sibling_owner_id = "test-user-ambiguous-invalidation-sibling";
        let ttl_minutes: i64 = 60 * 24;
        let (result, _) = run_in_request(request_state::request_state_scope_for_test(async {
            let admin = SessionGuard::named(
                "admin",
                Arc::new(NamedRememberProvider {
                    id: ambiguous_owner_id,
                }),
            )
            .with_remember_ttl(ttl_minutes);
            let staff = SessionGuard::named(
                "staff",
                Arc::new(NamedRememberProvider {
                    id: sibling_owner_id,
                }),
            )
            .with_remember_ttl(ttl_minutes);
            admin
                .login(
                    Arc::new(NamedRememberUser {
                        id: ambiguous_owner_id.to_owned(),
                    }) as Arc<dyn Authenticatable>,
                    true,
                )
                .await
                .expect("issue ambiguous-owner remember carrier");
            staff
                .login(
                    Arc::new(NamedRememberUser {
                        id: sibling_owner_id.to_owned(),
                    }) as Arc<dyn Authenticatable>,
                    true,
                )
                .await
                .expect("issue sibling remember carrier");

            let connection = suprnova::DB::connection().expect("remember database connection");
            let row = suprnova::auth::remember::entity::Entity::find()
                .filter(suprnova::auth::remember::entity::Column::UserId.eq(ambiguous_owner_id))
                .one(connection.inner())
                .await
                .expect("query ambiguous-owner row")
                .expect("ambiguous owner has one issued row");
            suprnova::auth::remember::entity::ActiveModel {
                user_id: Set(row.user_id),
                selector: Set(row.selector),
                token_hash: Set(row.token_hash),
                expires_at: Set(row.expires_at),
                created_at: Set(row.created_at),
                last_used_at: Set(row.last_used_at),
                ..Default::default()
            }
            .insert(connection.inner())
            .await
            .expect("insert duplicate owner/selector row");

            let invalidation = Auth::logout_and_invalidate().await;
            assert!(
                admin
                    .guest()
                    .await
                    .expect("read admin guard after teardown")
            );
            assert!(
                staff
                    .guest()
                    .await
                    .expect("read staff guard after teardown")
            );
            assert!(
                suprnova::session::session()
                    .expect("invalidation retains an empty data session")
                    .user_id
                    .is_none()
            );
            invalidation
        }))
        .await;

        let error = result.expect_err("ambiguous exact revocation must be reported");
        assert!(
            error
                .to_string()
                .contains("remember selector matched multiple exact rows"),
            "unexpected ambiguity error: {error}"
        );
        assert_eq!(
            count_tokens_for(ambiguous_owner_id).await,
            2,
            "ambiguous rows must remain untouched after rollback"
        );
        assert_eq!(
            count_tokens_for(sibling_owner_id).await,
            0,
            "full invalidation must continue revoking sibling selectors after the first error"
        );
    });
}

/// Test 10 (end-to-end, negative): a forged `remember_me` cookie does
/// NOT authenticate AND the middleware queues a clear cookie so the
/// client stops sending garbage.
#[cfg(feature = "testing")]
#[test]
fn middleware_clears_forged_remember_cookie() {
    use bytes::Bytes;
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper_util::rt::TokioIo;
    use std::convert::Infallible;
    use std::sync::Arc;
    use suprnova::Request;
    use suprnova::middleware::Middleware;
    use tokio::io::AsyncWriteExt;
    use tokio::sync::oneshot;

    Lazy::force(&SETUP);

    RT.block_on(async {
        // A forged plaintext encrypted under the legitimate key - ciphertext
        // valid, but no matching hashed row.
        //
        // Compat-window regression: this still uses v1 wire-format minting; middleware
        // must reject it and clear the cookie.
        let forged_plaintext = "FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF";
        let encrypted =
            suprnova::Crypt::encrypt_string(suprnova::CryptPurpose::Cookie, forged_plaintext)
                .expect("encrypt forged");

        let mut http_bytes = Vec::new();
        http_bytes.extend_from_slice(b"GET / HTTP/1.1\r\n");
        http_bytes.extend_from_slice(b"Host: localhost\r\n");
        http_bytes.extend_from_slice(format!("Cookie: remember_me={encrypted}\r\n").as_bytes());
        http_bytes.extend_from_slice(b"Content-Length: 0\r\n\r\n");

        let (req_tx, req_rx) = oneshot::channel::<Request>();
        let req_tx = std::sync::Mutex::new(Some(req_tx));
        let duplex_cap = http_bytes.len() + 64 * 1024;
        let (client_io, server_io) = tokio::io::duplex(duplex_cap);

        tokio::spawn(async move {
            let svc = service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
                let wrapped = Request::new(req);
                if let Ok(mut guard) = req_tx.lock()
                    && let Some(tx) = guard.take()
                {
                    let _ = tx.send(wrapped);
                }
                async {
                    std::future::pending::<()>().await;
                    Ok::<_, Infallible>(hyper::Response::new(http_body_util::Empty::<Bytes>::new()))
                }
            });
            let _ = http1::Builder::new()
                .serve_connection(TokioIo::new(server_io), svc)
                .await;
        });

        {
            let mut client = client_io;
            client.write_all(&http_bytes).await.unwrap();
        }
        let request = req_rx.await.expect("server received request");

        let observed = Arc::new(std::sync::Mutex::new(None::<String>));
        let observed_clone = observed.clone();
        let next: suprnova::middleware::Next = Arc::new(move |_req| {
            let observed = observed_clone.clone();
            Box::pin(async move {
                *observed.lock().unwrap() = suprnova::Auth::id();
                Ok(suprnova::HttpResponse::text("ok"))
            })
        });

        let mut config = SessionConfig::default();
        config.cookie_secure = false;
        let middleware = suprnova::SessionMiddleware::new(config);
        let response = middleware.handle(request, next).await;

        // Handler must NOT have seen a user - the cookie didn't match.
        let captured = observed.lock().unwrap().clone();
        assert_eq!(captured, None, "forged cookie must not authenticate");

        // Response must clear the remember cookie (Max-Age=0).
        let response = match response {
            Ok(r) => r,
            Err(_) => panic!("middleware should not short-circuit the request"),
        };
        let hyper_resp = response.into_hyper();
        let cleared = hyper_resp
            .headers()
            .get_all("Set-Cookie")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .any(|c| c.starts_with("remember_me=") && c.contains("Max-Age=0"));
        assert!(
            cleared,
            "middleware must clear the cookie when the token does not match"
        );
    });
}

/// An older middleware must fail closed without destroying a carrier owned by
/// a newer deployment version. This keeps rolling deployments recoverable:
/// the same browser can reach a v2-aware node on its next request.
#[cfg(feature = "testing")]
#[test]
fn middleware_preserves_unknown_remember_carrier_version() {
    use suprnova::middleware::Middleware;

    Lazy::force(&SETUP);

    RT.block_on(async {
        let future_carrier = concat!(
            "suprnova.remember.v2:",
            r#"{"guard":"admin","credential":"future-selector.future-verifier"}"#,
        );
        let encrypted = Cookie::encrypted(suprnova::auth::remember::COOKIE_NAME, future_carrier)
            .expect("encrypt future-version remember carrier");
        let request = request_with_remember_cookie(encrypted.value()).await;

        type Observation = (Option<String>, Option<String>);
        let observed = Arc::new(std::sync::Mutex::new(None::<Observation>));
        let observed_clone = observed.clone();
        let next: suprnova::middleware::Next = Arc::new(move |_request| {
            let observed = observed_clone.clone();
            Box::pin(async move {
                let admin = SessionGuard::named(
                    "admin",
                    Arc::new(NamedRememberProvider {
                        id: "future-version-user",
                    }),
                );
                *observed.lock().unwrap() = Some((Auth::id(), admin.id().await.unwrap()));
                Ok(suprnova::HttpResponse::text("ok"))
            })
        });

        let mut config = SessionConfig::default();
        config.cookie_secure = false;
        let response = request_state::request_state_scope_for_test(
            suprnova::SessionMiddleware::new(config).handle(request, next),
        )
        .await;
        let response = match response {
            Ok(response) => response.into_hyper(),
            Err(_) => panic!("unknown carrier version must not short-circuit the request"),
        };

        assert_eq!(
            *observed.lock().unwrap(),
            Some((None, None)),
            "an unknown carrier version must authenticate neither default nor encoded guard"
        );
        let remember_headers = response
            .headers()
            .get_all("Set-Cookie")
            .iter()
            .filter_map(|value| value.to_str().ok())
            .filter(|header| header.starts_with("remember_me="))
            .collect::<Vec<_>>();
        assert!(
            remember_headers.is_empty(),
            "an unknown carrier version must remain untouched, got {remember_headers:?}"
        );
    });
}

/// A carrier with the supported version marker but an invalid payload is not
/// forward-compatible data. The current middleware owns it and must clear it.
#[cfg(feature = "testing")]
#[test]
fn middleware_clears_malformed_supported_remember_carrier() {
    use suprnova::middleware::Middleware;

    Lazy::force(&SETUP);

    RT.block_on(async {
        let encrypted = Cookie::encrypted(
            suprnova::auth::remember::COOKIE_NAME,
            "suprnova.remember.v1:not-json",
        )
        .expect("encrypt malformed supported remember carrier");
        let request = request_with_remember_cookie(encrypted.value()).await;

        let observed = Arc::new(std::sync::Mutex::new(None::<String>));
        let observed_clone = observed.clone();
        let next: suprnova::middleware::Next = Arc::new(move |_request| {
            let observed = observed_clone.clone();
            Box::pin(async move {
                *observed.lock().unwrap() = Auth::id();
                Ok(suprnova::HttpResponse::text("ok"))
            })
        });

        let mut config = SessionConfig::default();
        config.cookie_secure = false;
        let response = request_state::request_state_scope_for_test(
            suprnova::SessionMiddleware::new(config).handle(request, next),
        )
        .await;
        let response = match response {
            Ok(response) => response.into_hyper(),
            Err(_) => panic!("malformed supported carrier must not short-circuit the request"),
        };

        assert_eq!(
            *observed.lock().unwrap(),
            None,
            "a malformed supported carrier must not authenticate"
        );
        let cleared = response
            .headers()
            .get_all("Set-Cookie")
            .iter()
            .filter_map(|value| value.to_str().ok())
            .any(|header| header.starts_with("remember_me=") && header.contains("Max-Age=0"));
        assert!(
            cleared,
            "middleware must clear a malformed supported remember carrier"
        );
    });
}

/// A remembered login in the same request must replace middleware's queued
/// clear for a malformed carrier. Emitting both directives leaves the final
/// browser state dependent on duplicate-header ordering.
#[cfg(feature = "testing")]
#[test]
fn session_guard_remember_login_replaces_malformed_carrier_clear_cookie() {
    use suprnova::middleware::Middleware;

    Lazy::force(&SETUP);

    RT.block_on(async {
        let fresh_user_id = "fresh-after-malformed-carrier";
        let ttl_minutes = 60 * 24;
        let encrypted = Cookie::encrypted(
            suprnova::auth::remember::COOKIE_NAME,
            "suprnova.remember.v1:not-json",
        )
        .expect("encrypt malformed supported remember carrier");
        let request = request_with_remember_cookie(encrypted.value()).await;

        let next: suprnova::middleware::Next = Arc::new(move |_request| {
            Box::pin(async move {
                assert_eq!(Auth::id(), None);
                SessionGuard::named("web", Arc::new(NamedRememberProvider { id: fresh_user_id }))
                    .with_remember_ttl(ttl_minutes)
                    .login(
                        Arc::new(NamedRememberUser {
                            id: fresh_user_id.to_owned(),
                        }) as Arc<dyn Authenticatable>,
                        true,
                    )
                    .await
                    .expect("remembered SessionGuard login succeeds");
                assert_eq!(Auth::id().as_deref(), Some(fresh_user_id));
                Ok(suprnova::HttpResponse::text("logged-in"))
            })
        });

        let mut config = SessionConfig::default();
        config.cookie_secure = false;
        config.remember_lifetime =
            std::time::Duration::from_secs((ttl_minutes as u64).saturating_mul(60));
        let response = request_state::request_state_scope_for_test(
            suprnova::SessionMiddleware::new(config).handle(request, next),
        )
        .await;
        let response = match response {
            Ok(response) => response.into_hyper(),
            Err(_) => panic!("remembered login must complete after a malformed carrier"),
        };

        let remember_headers = response
            .headers()
            .get_all("Set-Cookie")
            .iter()
            .filter_map(|value| value.to_str().ok())
            .filter(|header| header.starts_with("remember_me="))
            .collect::<Vec<_>>();
        assert_eq!(
            remember_headers.len(),
            1,
            "the fresh carrier must replace the queued clear: {remember_headers:?}"
        );
        assert!(
            !remember_headers[0].contains("Max-Age=0"),
            "the sole directive must install the fresh carrier: {remember_headers:?}"
        );
        let installed_value = remember_headers[0]
            .split(';')
            .next()
            .and_then(|pair| pair.strip_prefix("remember_me="))
            .expect("fresh remember cookie carries a value");
        let carrier =
            Cookie::read_encrypted_for(suprnova::auth::remember::COOKIE_NAME, installed_value)
                .expect("fresh remember carrier decrypts");
        let (guard, credential) = decode_versioned_remember_carrier(&carrier);
        assert_eq!(guard, "web");
        assert!(!credential.is_empty());
    });
}
