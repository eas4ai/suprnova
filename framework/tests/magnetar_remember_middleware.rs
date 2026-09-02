//! RED contracts for framework remember hydration through an installed Magnetar engine.

#![cfg(all(feature = "testing", feature = "database-sqlite"))]

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex as StdMutex};

use async_trait::async_trait;
use bytes::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use sea_orm::{ConnectOptions, Database, EntityTrait};
use suprnova::middleware::Middleware;
use suprnova::session::{SessionConfig, SessionData, SessionStore};
use suprnova::{
    Auth, Crypt, EncryptionKey, FrameworkError, MagnetarConfig, RateLimiterDriver, Request,
    SessionMiddleware, SlidingWindowConfig, init_magnetar,
};
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex, OnceCell, oneshot};

struct AllowingLimiter;

static MAGNETAR_SETUP: OnceCell<sea_orm::DatabaseConnection> = OnceCell::const_new();
static MAGNETAR_TEST_LOCK: Mutex<()> = Mutex::const_new(());

fn magnetar_db_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "magnetar-remember-middleware-{}.sqlite",
        std::process::id()
    ))
}

async fn magnetar_connection() -> sea_orm::DatabaseConnection {
    MAGNETAR_SETUP
        .get_or_init(|| async {
            Crypt::init(EncryptionKey::generate());
            suprnova::App::bind::<dyn RateLimiterDriver>(Arc::new(AllowingLimiter));
            // Each `#[tokio::test]` owns a runtime. If the runtime that opened
            // an in-memory SQLite connection exits, sqlx may reopen the pool's
            // sole connection against an empty database. A file-backed fixture
            // keeps the schema intact across those per-test runtime boundaries.
            let db_path = magnetar_db_path();
            for suffix in ["", "-wal", "-shm"] {
                let mut path = db_path.clone().into_os_string();
                path.push(suffix);
                let _ = std::fs::remove_file(std::path::PathBuf::from(path));
            }
            let mut options =
                ConnectOptions::new(format!("sqlite://{}?mode=rwc", db_path.display()));
            options.max_connections(1).min_connections(1);
            let connection = Database::connect(options).await.expect("connect SQLite");
            init_magnetar(MagnetarConfig::from_sea_orm(connection.clone()))
                .await
                .expect("install default Magnetar engine");
            connection
        })
        .await
        .clone()
}

#[async_trait]
impl RateLimiterDriver for AllowingLimiter {
    async fn try_acquire(&self, _: &str, _: &SlidingWindowConfig) -> Result<bool, FrameworkError> {
        Ok(true)
    }

    async fn retry_after(
        &self,
        _: &str,
        _: &SlidingWindowConfig,
    ) -> Result<Option<std::time::Duration>, FrameworkError> {
        Ok(None)
    }
}

#[derive(Default)]
struct MemorySessionStore(Mutex<HashMap<String, SessionData>>);

impl MemorySessionStore {
    async fn seed(&self, mut session: SessionData) {
        session.loaded_from_store = true;
        self.0.lock().await.insert(session.id.clone(), session);
    }

    async fn stored(&self, id: &str) -> Option<SessionData> {
        self.0.lock().await.get(id).cloned()
    }
}

#[async_trait]
impl SessionStore for MemorySessionStore {
    async fn read(&self, id: &str) -> Result<Option<SessionData>, FrameworkError> {
        Ok(self.0.lock().await.get(id).cloned().map(|mut session| {
            session.loaded_from_store = true;
            session
        }))
    }

    async fn write(&self, session: &SessionData) -> Result<(), FrameworkError> {
        self.0
            .lock()
            .await
            .insert(session.id.clone(), session.clone());
        Ok(())
    }

    async fn destroy(&self, id: &str) -> Result<(), FrameworkError> {
        self.0.lock().await.remove(id);
        Ok(())
    }

    async fn destroy_for_user(&self, user_id: &str) -> Result<u64, FrameworkError> {
        let mut sessions = self.0.lock().await;
        let before = sessions.len();
        sessions.retain(|_, session| session.user_id.as_deref() != Some(user_id));
        Ok((before - sessions.len()) as u64)
    }

    async fn gc(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }
}

async fn request_with_cookies(cookies: &[(&str, &str)]) -> Request {
    let mut http_bytes = Vec::new();
    http_bytes.extend_from_slice(b"GET / HTTP/1.1\r\nHost: localhost\r\n");
    if !cookies.is_empty() {
        let header = cookies
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ");
        http_bytes.extend_from_slice(format!("Cookie: {header}\r\n").as_bytes());
    }
    http_bytes.extend_from_slice(b"Content-Length: 0\r\n\r\n");

    let (request_tx, request_rx) = oneshot::channel::<Request>();
    let request_tx = Arc::new(StdMutex::new(Some(request_tx)));
    let (mut client_io, server_io) = tokio::io::duplex(http_bytes.len() + 64 * 1024);
    tokio::spawn(async move {
        let service = service_fn(move |request: hyper::Request<hyper::body::Incoming>| {
            let wrapped = Request::new(request);
            if let Ok(mut guard) = request_tx.lock()
                && let Some(sender) = guard.take()
            {
                let _ = sender.send(wrapped);
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
    client_io
        .write_all(&http_bytes)
        .await
        .expect("write in-memory request");
    drop(client_io);
    request_rx.await.expect("server received request")
}

fn response_cookie(headers: &hyper::HeaderMap, name: &str) -> String {
    headers
        .get_all("Set-Cookie")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find_map(|header| {
            header
                .split(';')
                .next()
                .and_then(|pair| pair.strip_prefix(&format!("{name}=")))
                .map(str::to_owned)
        })
        .unwrap_or_else(|| panic!("response must set {name}"))
}

#[tokio::test]
async fn installed_engine_remember_hydration_rotates_and_binds_both_sessions() {
    let _test_guard = MAGNETAR_TEST_LOCK.lock().await;
    let connection = magnetar_connection().await;

    let user = Auth::password()
        .register("remember-middleware@example.test", "correct-password")
        .await
        .expect("register Magnetar user");
    let store = Arc::new(MemorySessionStore::default());
    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    config.remember_lifetime = std::time::Duration::from_secs(24 * 60 * 60);
    let middleware = SessionMiddleware::with_store(config.clone(), store.clone());

    let remembered_user = user.id.to_string();
    let issue_next: suprnova::middleware::Next = Arc::new(move |_request| {
        let remembered_user = remembered_user.clone();
        Box::pin(async move {
            Auth::login_remember(remembered_user, 24 * 60)
                .await
                .expect("issue Magnetar remember credential");
            Ok(suprnova::HttpResponse::text("issued"))
        })
    });
    let issue_response = match middleware
        .handle(request_with_cookies(&[]).await, issue_next)
        .await
    {
        Ok(response) => response.into_hyper(),
        Err(_) => panic!("issue request must succeed"),
    };
    let remember_cookie = response_cookie(issue_response.headers(), "remember_me");
    let issued_row = magnetar::default_schema::remembers::Entity::find()
        .one(&connection)
        .await
        .expect("query issued Magnetar remember row")
        .expect("issuing remember-me persists a Magnetar row");
    assert!(
        issued_row.expires_at
            <= chrono::Utc::now() + chrono::Duration::days(1) + chrono::Duration::seconds(5),
        "the issued Magnetar credential must honor the configured one-day lifetime",
    );

    let anonymous_id = "a".repeat(40);
    let anonymous_csrf = "csrf-before-remember";
    store
        .seed(SessionData::new(
            anonymous_id.clone(),
            anonymous_csrf.to_owned(),
        ))
        .await;
    let anonymous_cookie = Crypt::encrypt_string(suprnova::CryptPurpose::Cookie, &anonymous_id)
        .expect("encrypt anonymous data-session cookie");

    let observed = Arc::new(Mutex::new(None::<SessionData>));
    let observed_in_handler = observed.clone();
    let hydrate_next: suprnova::middleware::Next = Arc::new(move |_request| {
        let observed = observed_in_handler.clone();
        Box::pin(async move {
            *observed.lock().await = suprnova::session::session();
            Ok(suprnova::HttpResponse::text("hydrated"))
        })
    });
    let hydrate_response = match middleware
        .handle(
            request_with_cookies(&[
                (&config.cookie_name, &anonymous_cookie),
                ("remember_me", &remember_cookie),
            ])
            .await,
            hydrate_next,
        )
        .await
    {
        Ok(response) => response.into_hyper(),
        Err(_) => panic!("remember hydration request must succeed"),
    };

    let hydrated = observed
        .lock()
        .await
        .clone()
        .expect("handler sees data session");
    assert_eq!(hydrated.user_id.as_deref(), Some(user.id.as_str()));
    assert_ne!(
        hydrated.id, anonymous_id,
        "remember hydration rotates the data-session id"
    );
    assert_ne!(
        hydrated.csrf_token, anonymous_csrf,
        "remember hydration rotates the CSRF token",
    );
    assert!(
        store.stored(&anonymous_id).await.is_none(),
        "the pre-authentication data session must be destroyed",
    );

    let binding = hydrated
        .magnetar_web_binding()
        .expect("installed-engine hydration persists a digest-only Magnetar web binding");
    assert_ne!(
        binding.session_id, hydrated.id,
        "Magnetar and framework session ids stay distinct"
    );
    assert!(
        suprnova::magnetar_integration::revoke_session(&binding.session_id)
            .await
            .expect("revoke bound Magnetar session"),
        "the persisted binding must name a real opaque Magnetar session",
    );

    let rotated_data_cookie = response_cookie(hydrate_response.headers(), &config.cookie_name);
    let rotated_remember_cookie = response_cookie(hydrate_response.headers(), "remember_me");
    let rotated_row = magnetar::default_schema::remembers::Entity::find()
        .one(&connection)
        .await
        .expect("query rotated Magnetar remember row")
        .expect("remember hydration persists its sole replacement row");
    assert!(
        rotated_row.expires_at
            <= chrono::Utc::now() + chrono::Duration::days(1) + chrono::Duration::seconds(5),
        "the rotated Magnetar credential must preserve the configured one-day lifetime",
    );
    assert_ne!(
        rotated_remember_cookie, remember_cookie,
        "remember credential rotates on hydration"
    );

    let observed_after_revoke = Arc::new(Mutex::new(None::<String>));
    let observed_in_handler = observed_after_revoke.clone();
    let revoked_next: suprnova::middleware::Next = Arc::new(move |_request| {
        let observed = observed_in_handler.clone();
        Box::pin(async move {
            *observed.lock().await = Auth::id();
            Ok(suprnova::HttpResponse::text("checked"))
        })
    });
    if middleware
        .handle(
            request_with_cookies(&[(&config.cookie_name, &rotated_data_cookie)]).await,
            revoked_next,
        )
        .await
        .is_err()
    {
        panic!("revoked binding request must remain anonymous");
    }
    assert_eq!(
        *observed_after_revoke.lock().await,
        None,
        "an installed engine must validate the Magnetar binding, never trust bare user_id",
    );

    // Named provider-backed session guards do not own Magnetar bindings and
    // must survive merely because an engine is installed. Named identities
    // that do carry a binding remain fail-closed, including malformed values.
    let named_session_id = "b".repeat(40);
    let mut named_session = SessionData::new(named_session_id.clone(), "named-csrf".to_owned());
    named_session.data.insert(
        "_auth_guards".to_owned(),
        serde_json::json!({
            "provider": { "id": "provider-user" },
            "revoked": {
                "id": "revoked-user",
                "magnetar_web_binding": {
                    "session_id": "missing-session",
                    "token_digest": vec![0_u8; 32],
                },
            },
            "malformed": {
                "id": "malformed-user",
                "magnetar_web_binding": "not-a-binding",
            },
        }),
    );
    store.seed(named_session).await;
    let named_cookie = Crypt::encrypt_string(suprnova::CryptPurpose::Cookie, &named_session_id)
        .expect("encrypt named-guard data-session cookie");
    let observed_named = Arc::new(Mutex::new(None::<SessionData>));
    let observed_in_handler = observed_named.clone();
    let named_next: suprnova::middleware::Next = Arc::new(move |_request| {
        let observed = observed_in_handler.clone();
        Box::pin(async move {
            *observed.lock().await = suprnova::session::session();
            Ok(suprnova::HttpResponse::text("named checked"))
        })
    });
    assert!(
        middleware
            .handle(
                request_with_cookies(&[(&config.cookie_name, &named_cookie)]).await,
                named_next,
            )
            .await
            .is_ok(),
        "named binding validation request must reach the handler",
    );
    let observed_named = observed_named
        .lock()
        .await
        .clone()
        .expect("handler sees named-guard data session");
    let guards = observed_named
        .data
        .get("_auth_guards")
        .and_then(serde_json::Value::as_object)
        .expect("guard container remains for the provider-backed identity");
    assert!(
        guards.contains_key("provider"),
        "binding-less provider-backed named guards must survive",
    );
    assert!(
        !guards.contains_key("revoked"),
        "an explicitly bound named guard must fail closed when its binding is revoked",
    );
    assert!(
        !guards.contains_key("malformed"),
        "a present but malformed named binding must fail closed",
    );
}

#[tokio::test]
async fn installed_engine_rejects_default_guard_identity_without_compatibility_user_id() {
    let _test_guard = MAGNETAR_TEST_LOCK.lock().await;
    let _connection = magnetar_connection().await;
    let store = Arc::new(MemorySessionStore::default());
    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    let middleware = SessionMiddleware::with_store(config.clone(), store.clone());

    let binding = serde_json::json!({
        "session_id": "missing-session",
        "token_digest": vec![0_u8; 32],
    });
    for (case, session_byte, top_level_binding) in [
        ("binding-present", 'c', true),
        ("binding-absent", 'd', false),
    ] {
        let session_id = session_byte.to_string().repeat(40);
        let mut session = SessionData::new(session_id.clone(), format!("{case}-csrf"));
        if top_level_binding {
            session
                .data
                .insert("auth.magnetar_web_binding".to_owned(), binding.clone());
        }
        let guard_state = if top_level_binding {
            serde_json::json!({
                "id": "one-sided-user",
                "magnetar_web_binding": binding,
            })
        } else {
            serde_json::json!({ "id": "one-sided-user" })
        };
        session.data.insert(
            "_auth_guards".to_owned(),
            serde_json::json!({ "web": guard_state }),
        );
        store.seed(session).await;
        let session_cookie = Crypt::encrypt_string(suprnova::CryptPurpose::Cookie, &session_id)
            .expect("encrypt one-sided data-session cookie");

        let observed = Arc::new(Mutex::new(None::<(Option<String>, SessionData)>));
        let observed_in_handler = observed.clone();
        let next: suprnova::middleware::Next = Arc::new(move |_request| {
            let observed = observed_in_handler.clone();
            Box::pin(async move {
                let session = suprnova::session::session().expect("handler sees data session");
                *observed.lock().await = Some((Auth::id(), session));
                Ok(suprnova::HttpResponse::text("checked"))
            })
        });

        assert!(
            middleware
                .handle(
                    request_with_cookies(&[(&config.cookie_name, &session_cookie)]).await,
                    next,
                )
                .await
                .is_ok(),
            "{case} default identity request must reach the handler",
        );
        let (auth_id, observed_session) = observed
            .lock()
            .await
            .clone()
            .expect("handler records reconciled state");
        assert_eq!(auth_id, None, "{case} default identity must fail closed");
        assert!(
            observed_session
                .data
                .get("_auth_guards")
                .and_then(serde_json::Value::as_object)
                .is_none_or(|guards| !guards.contains_key("web")),
            "{case} unverified default guard record must be removed",
        );
    }
}

#[tokio::test]
async fn installed_engine_fresh_binding_clears_hydrated_identity_carrier() {
    let _test_guard = MAGNETAR_TEST_LOCK.lock().await;
    let _connection = magnetar_connection().await;
    let previous = Auth::password()
        .register("remember-switch-previous@example.test", "previous-password")
        .await
        .expect("register previous remembered identity");
    let fresh = Auth::password()
        .register("remember-switch-fresh@example.test", "fresh-password")
        .await
        .expect("register fresh identity");

    let store = Arc::new(MemorySessionStore::default());
    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    config.remember_lifetime = std::time::Duration::from_secs(24 * 60 * 60);
    let middleware = SessionMiddleware::with_store(config, store);

    let previous_user_id = previous.id.clone();
    let issue_next: suprnova::middleware::Next = Arc::new(move |_request| {
        let previous_user_id = previous_user_id.clone();
        Box::pin(async move {
            Auth::login_remember(previous_user_id, 24 * 60)
                .await
                .expect("issue previous identity's Magnetar remember credential");
            Ok(suprnova::HttpResponse::text("issued"))
        })
    });
    let issue_response = suprnova::auth::request_state::request_state_scope_for_test(
        middleware.handle(request_with_cookies(&[]).await, issue_next),
    )
    .await;
    let issue_response = match issue_response {
        Ok(response) => response.into_hyper(),
        Err(_) => panic!("remember issue request must reach the handler"),
    };
    let previous_remember_cookie = response_cookie(issue_response.headers(), "remember_me");

    let previous_user_id = previous.id.clone();
    let fresh_user_id = fresh.id.clone();
    let switch_next: suprnova::middleware::Next = Arc::new(move |_request| {
        let previous_user_id = previous_user_id.clone();
        let fresh_user_id = fresh_user_id.clone();
        Box::pin(async move {
            assert_eq!(Auth::id().as_deref(), Some(previous_user_id.as_str()));
            assert!(Auth::via_remember());
            let (authenticated, _) = Auth::password()
                .authenticate(
                    "remember-switch-fresh@example.test",
                    "fresh-password",
                    None,
                    None,
                )
                .await
                .expect("fresh Magnetar password authentication succeeds");
            assert_eq!(authenticated.id, fresh_user_id);
            assert_eq!(Auth::id().as_deref(), Some(authenticated.id.as_str()));
            assert!(!Auth::via_remember());
            Ok(suprnova::HttpResponse::text("switched"))
        })
    });
    let switch_response =
        suprnova::auth::request_state::request_state_scope_for_test(middleware.handle(
            request_with_cookies(&[("remember_me", &previous_remember_cookie)]).await,
            switch_next,
        ))
        .await;
    let switch_response = match switch_response {
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
    let mut browser_remember_cookie = Some(previous_remember_cookie);
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

    // Drop B's data-session cookie and replay only what the browser retained.
    let replay_request = match browser_remember_cookie.as_deref() {
        Some(cookie) => request_with_cookies(&[("remember_me", cookie)]).await,
        None => request_with_cookies(&[]).await,
    };
    let observed_after_expiry = Arc::new(Mutex::new(None::<String>));
    let observed_in_handler = observed_after_expiry.clone();
    let replay_next: suprnova::middleware::Next = Arc::new(move |_request| {
        let observed = observed_in_handler.clone();
        Box::pin(async move {
            *observed.lock().await = Auth::id();
            Ok(suprnova::HttpResponse::text("replayed"))
        })
    });
    if suprnova::auth::request_state::request_state_scope_for_test(
        middleware.handle(replay_request, replay_next),
    )
    .await
    .is_err()
    {
        panic!("post-expiry request must reach the handler");
    }

    assert_eq!(
        remember_headers.len(),
        1,
        "fresh binding must replace A's queued carrier with one forget directive"
    );
    assert_eq!(browser_remember_cookie, None);
    assert_eq!(
        *observed_after_expiry.lock().await,
        None,
        "after B's data session expires, the browser must not reauthenticate A"
    );
}
