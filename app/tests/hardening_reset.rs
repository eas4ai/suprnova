//! HRD-004: the dogfood password reset reports success only when the
//! password write and both revocations succeeded.
//!
//! `POST /auth/password/reset` rotates the password, then revokes the
//! user's other sessions and remember-me tokens. A revocation that fails
//! leaves a stolen session or remember-me cookie working after the reset,
//! so the reset must not answer the `302 /?reset=ok` of the success path.
//!
//! HRD-008: the same failure queues `RevokeUserCredentials` for the user
//! before the reset answers, so a worker retries the revocations until both
//! stores confirm them.
//!
//! These drive the real router and global middleware. The session store
//! that revocation resolves is a container binding, bound here before
//! `register_http_stack`: `SessionMiddleware::new` binds the store it builds
//! only when none is bound, so the one bound here is the one the reset
//! revokes through. The middleware keeps its own database store, so the
//! session and the CSRF check work as they do in production.

mod test_environment;

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use bytes::Bytes;
use chrono::Utc;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use sea_orm_migration::MigratorTrait;
use serde_json::Value;

use app::migrations::Migrator;
use app::models::users::User;
use suprnova::auth_flows::token_store::TokenStore;
use suprnova::auth_flows::{RevokeUserCredentials, TokenPurpose};
use suprnova::http::cookie::Cookie;
use suprnova::logging::{LogConfig, LogFormat};
use suprnova::queue::testing::pushed;
use suprnova::session::driver::database::DatabaseSessionDriver;
use suprnova::session::{
    DestroyedSessions, SessionConfig, SessionData, SessionStore, generate_csrf_token,
    generate_session_id,
};
use suprnova::{
    App, EloquentUserProvider, FrameworkError, Log, LogLevel, MessageLogged, MiddlewareRegistry,
    Model, Queue, UserProvider, async_trait, attrs, bind, handle_request,
};

/// `Crypt`, the database and the container bindings are process-global, so
/// the tests take turns.
static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A session store that keeps sessions in the database, as the configured
/// driver does, and fails every revocation.
struct RevocationFailsStore {
    sessions: DatabaseSessionDriver,
}

#[async_trait]
impl SessionStore for RevocationFailsStore {
    async fn read(&self, id: &str) -> Result<Option<SessionData>, FrameworkError> {
        self.sessions.read(id).await
    }

    async fn write(&self, session: &SessionData) -> Result<(), FrameworkError> {
        self.sessions.write(session).await
    }

    async fn destroy(&self, id: &str) -> Result<(), FrameworkError> {
        self.sessions.destroy(id).await
    }

    async fn destroy_for_user(&self, _user_id: &str) -> Result<u64, FrameworkError> {
        Err(FrameworkError::database("the session store is unavailable"))
    }

    async fn destroy_guard_sessions(
        &self,
        _guard: &str,
        _user_id: &str,
    ) -> Result<DestroyedSessions, FrameworkError> {
        Err(FrameworkError::database("the session store is unavailable"))
    }

    async fn gc(&self) -> Result<u64, FrameworkError> {
        self.sessions.gc().await
    }
}

fn database_sessions() -> DatabaseSessionDriver {
    DatabaseSessionDriver::new(SessionConfig::default().lifetime)
}

/// Every warning- and error-level line this process logs, from the first
/// call on.
///
/// Installs the framework's subscriber as the process default, once, so a
/// line the handler logs on a server task reaches `Log::listen`.
fn logged_lines() -> Arc<Mutex<Vec<MessageLogged>>> {
    static LINES: OnceLock<Arc<Mutex<Vec<MessageLogged>>>> = OnceLock::new();
    Arc::clone(LINES.get_or_init(|| {
        let subscriber = suprnova::logging::build_subscriber(LogConfig {
            level: "warn".to_owned(),
            format: LogFormat::Pretty,
        })
        .expect("the log subscriber builds");
        tracing::subscriber::set_global_default(subscriber)
            .expect("no other subscriber is installed in this test process");
        let lines = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&lines);
        Log::listen(move |line: &MessageLogged| {
            if matches!(line.level, LogLevel::Error | LogLevel::Warning) {
                sink_of(&sink).push(line.clone());
            }
        });
        lines
    }))
}

/// The lines the sink holds. A test that fails while it reads them must not
/// poison the sink for the tests after it, so a poisoned lock is recovered.
fn sink_of(lines: &Mutex<Vec<MessageLogged>>) -> MutexGuard<'_, Vec<MessageLogged>> {
    lines.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Whether `lines` holds a line at `level` whose `user_id` field is
/// `user_id` and whose message contains `text`.
fn logged(lines: &Mutex<Vec<MessageLogged>>, level: LogLevel, user_id: &str, text: &str) -> bool {
    sink_of(lines).iter().any(|line| {
        line.level == level
            && line.context.get("user_id").and_then(Value::as_str) == Some(user_id)
            && line.message.contains(text)
    })
}

struct TestApp {
    addr: SocketAddr,
    _lock: tokio::sync::MutexGuard<'static, ()>,
}

/// Serve the application's router and global middleware, with `store` as
/// the session store revocation resolves.
async fn spawn_app(store: Arc<dyn SessionStore>) -> TestApp {
    let lock = TEST_LOCK.lock().await;
    test_environment::testing();

    // `SessionMiddleware` fails closed without `Crypt`.
    suprnova::Crypt::init(suprnova::crypto::EncryptionKey::generate());

    let conn = sea_orm::Database::connect("sqlite::memory:")
        .await
        .expect("connect sqlite::memory:");
    Migrator::up(&conn, None)
        .await
        .expect("migrate sqlite::memory:");
    App::singleton(suprnova::DbConnection::from_raw(conn));
    // The cache store `Server::run` boots from `CACHE_DRIVER`; the session
    // middleware's blocking and the issuance limiter use it.
    App::bind_if_absent::<dyn suprnova::CacheStore>(Arc::new(suprnova::InMemoryCache::new()));
    bind!(dyn UserProvider, EloquentUserProvider::<User>::new());
    App::bind::<dyn SessionStore>(store);

    app::bootstrap::register_http_stack();
    let middleware = Arc::new(MiddlewareRegistry::from_global());
    let router = Arc::new(app::routes::register());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral listener");
    let addr = listener.local_addr().expect("local_addr");
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let io = TokioIo::new(stream);
            let router = Arc::clone(&router);
            let middleware = Arc::clone(&middleware);
            tokio::spawn(async move {
                let svc = service_fn(move |req: hyper::Request<Incoming>| {
                    let router = Arc::clone(&router);
                    let middleware = Arc::clone(&middleware);
                    async move { Ok::<_, Infallible>(handle_request(router, middleware, req).await) }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, svc)
                    .await;
            });
        }
    });

    TestApp { addr, _lock: lock }
}

/// A verified user with a live reset token, and the session the reset form
/// would be posted from.
struct ResetFixture {
    user: User,
    token: String,
    cookie: String,
    csrf: String,
}

async fn reset_fixture() -> ResetFixture {
    let mut user = User::create(attrs! {
        name: "Reset Test User",
        email: "reset@example.suprnova.app",
        password: "hash-before-reset",
    })
    .await
    .expect("insert the user");
    // Only a verified account may reset its password.
    user.email_verified_at = Some(Utc::now());
    user.save().await.expect("mark the user verified");

    let token = TokenStore::issue(
        &user.id.to_string(),
        TokenPurpose::PasswordReset,
        TokenPurpose::PasswordReset.default_ttl(),
    )
    .await
    .expect("issue a reset token");

    // `CsrfMiddleware` is global, so the form post carries a session and
    // its token, as a browser would.
    let session_id = generate_session_id();
    let mut session = SessionData::new(session_id.clone(), generate_csrf_token());
    session.dirty = true;
    database_sessions()
        .write(&session)
        .await
        .expect("write the session");
    let cookie = Cookie::encrypted("suprnova_session", &session_id)
        .expect("Crypt installed by spawn_app")
        .value()
        .to_string();

    ResetFixture {
        user,
        token,
        cookie,
        csrf: session.csrf_token.clone(),
    }
}

/// The status, `Location` header and body of `POST /auth/password/reset`.
async fn complete_reset(
    addr: SocketAddr,
    fixture: &ResetFixture,
    new_password: &str,
) -> (u16, Option<String>, Bytes) {
    let stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .expect("handshake");
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let form = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("token", &fixture.token)
        .append_pair("new_password", new_password)
        .finish();
    let req = hyper::Request::builder()
        .method("POST")
        .uri("/auth/password/reset")
        .header("Host", "localhost")
        .header("Cookie", format!("suprnova_session={}", fixture.cookie))
        .header("X-CSRF-TOKEN", &fixture.csrf)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(Full::new(Bytes::from(form)))
        .expect("request");

    let response = sender.send_request(req).await.expect("send");
    let status = response.status().as_u16();
    let location = response
        .headers()
        .get("Location")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("read the body")
        .to_bytes();
    (status, location, body)
}

/// The falsifier of HRD-004: the session store cannot revoke, so the reset
/// must not report success. The password has changed, the failure is
/// logged at error level with the user's id, and the answer is the
/// framework's `500`. The queue fake takes the retry the reset queues, so
/// the outcome reaches the handler; a retry that cannot be queued is
/// `hardening_a_reset_whose_retry_cannot_be_queued_still_answers_the_error`.
#[tokio::test]
async fn hardening_a_reset_whose_revocation_fails_does_not_report_success() {
    let lines = logged_lines();
    let app = spawn_app(Arc::new(RevocationFailsStore {
        sessions: database_sessions(),
    }))
    .await;
    let _queue = Queue::fake();
    let fixture = reset_fixture().await;

    let (status, location, body) = complete_reset(app.addr, &fixture, "a-new-password").await;

    assert_ne!(
        location.as_deref(),
        Some("/?reset=ok"),
        "a reset whose session revocation failed redirected to the success page"
    );
    assert_eq!(
        status, 500,
        "the failed revocation was not answered as an error"
    );
    let body: Value = serde_json::from_slice(&body).expect("the framework's JSON error body");
    assert_eq!(body["message"], "Internal Server Error");
    assert!(
        body.get("request_id").is_some(),
        "the error did not go through the framework's error path: {body}"
    );

    let user = User::find(fixture.user.id)
        .await
        .expect("read the user")
        .expect("the user exists");
    assert_ne!(
        user.password, fixture.user.password,
        "the password write is the step before the revocations and must stand"
    );

    let user_id = fixture.user.id.to_string();
    assert!(
        logged(&lines, LogLevel::Error, &user_id, "revoking the user's"),
        "no error-level line named user {user_id}: {:?}",
        sink_of(&lines)
    );
}

/// The control: with a store that revokes, the reset redirects to the
/// success page.
#[tokio::test]
async fn hardening_a_reset_whose_revocations_succeed_redirects() {
    let app = spawn_app(Arc::new(database_sessions())).await;
    let fixture = reset_fixture().await;

    let (status, location, body) = complete_reset(app.addr, &fixture, "a-new-password").await;

    assert_eq!(
        (status, location.as_deref()),
        (302, Some("/?reset=ok")),
        "a reset whose revocations succeeded did not redirect: {}",
        String::from_utf8_lossy(&body)
    );
}

/// The falsifier of HRD-008: the session store cannot revoke, so the reset
/// answers its error with one `RevokeUserCredentials` queued for the user,
/// and logs the queued retry with the user's id.
#[tokio::test]
async fn hardening_a_reset_whose_revocation_fails_queues_a_revocation_retry() {
    let lines = logged_lines();
    let app = spawn_app(Arc::new(RevocationFailsStore {
        sessions: database_sessions(),
    }))
    .await;
    sink_of(&lines).clear();
    let _queue = Queue::fake();
    let fixture = reset_fixture().await;

    let (status, location, _) = complete_reset(app.addr, &fixture, "a-new-password").await;

    assert_eq!(
        (status, location.as_deref()),
        (500, None),
        "the failed revocation was not answered as an error"
    );
    let user_id = fixture.user.id.to_string();
    assert_eq!(
        pushed::<RevokeUserCredentials>(),
        vec![RevokeUserCredentials::new(user_id.clone())],
        "the reset did not queue exactly one revocation retry for the user"
    );
    assert!(
        logged(&lines, LogLevel::Warning, &user_id, "queued a retry"),
        "no line logged the queued retry for user {user_id}: {:?}",
        sink_of(&lines)
    );
}

/// With both stores healthy the reset succeeds and queues no retry.
#[tokio::test]
async fn hardening_a_reset_whose_revocations_succeed_queues_no_retry() {
    let app = spawn_app(Arc::new(database_sessions())).await;
    let _queue = Queue::fake();
    let fixture = reset_fixture().await;

    let (status, location, body) = complete_reset(app.addr, &fixture, "a-new-password").await;

    assert_eq!(
        (status, location.as_deref()),
        (302, Some("/?reset=ok")),
        "a reset whose revocations succeeded did not redirect: {}",
        String::from_utf8_lossy(&body)
    );
    assert!(
        pushed::<RevokeUserCredentials>().is_empty(),
        "a reset whose revocations succeeded queued a retry"
    );
}

/// A retry that cannot be queued is logged with the user's id, and the
/// reset still answers the error of HRD-004.
#[tokio::test]
async fn hardening_a_reset_whose_retry_cannot_be_queued_still_answers_the_error() {
    let lines = logged_lines();
    let app = spawn_app(Arc::new(RevocationFailsStore {
        sessions: database_sessions(),
    }))
    .await;
    sink_of(&lines).clear();
    // The retry takes the real path, and with no driver installed the push
    // fails.
    let _queue = Queue::fake_except(&["Suprnova::RevokeUserCredentials"]);
    suprnova::queue::testing::forget_driver();
    let fixture = reset_fixture().await;

    let (status, location, _) = complete_reset(app.addr, &fixture, "a-new-password").await;

    assert_eq!(
        (status, location.as_deref()),
        (500, None),
        "a reset whose retry could not be queued did not answer the error"
    );
    assert!(
        pushed::<RevokeUserCredentials>().is_empty(),
        "the excepted retry was recorded by the fake"
    );
    let user_id = fixture.user.id.to_string();
    assert!(
        logged(&lines, LogLevel::Error, &user_id, "could not be queued"),
        "no error-level line logged the failed push for user {user_id}: {:?}",
        sink_of(&lines)
    );
}
