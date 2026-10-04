//! Framework login paths under the installed Magnetar engine.
//!
//! With a password/session engine installed, `SessionMiddleware` signs out
//! every default-guard identity that carries no binding to a live Magnetar
//! session. The framework's own login paths - `Auth::login_id`, the session
//! guard behind `Auth::attempt`, and the TOTP challenge of
//! `TwoFactor::complete_challenge` - authenticate outside Magnetar. These
//! tests drive the manual's flows through `handle_request` over a loopback
//! socket and check the user is still signed in on the next request.

#![cfg(all(feature = "testing", feature = "database-sqlite"))]

use std::any::Any;
use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use sea_orm::{ConnectOptions, ConnectionTrait, Database};
use sea_orm_migration::MigratorTrait;
use suprnova::auth_flows::two_factor::migration::Migration as TwoFactorMigration;
use suprnova::auth_flows::two_factor::migration_attempts::Migration as TwoFactorAttemptsMigration;
use suprnova::auth_flows::two_factor::migration_replay::Migration as TwoFactorReplayMigration;
use suprnova::auth_flows::{TwoFactor, TwoFactorUser};
use suprnova::database::DbConnection;
use suprnova::session::SessionConfig;
use suprnova::{
    App, Auth, AuthConfig, AuthManager, Authenticatable, Credentials, Crypt, EncryptionKey,
    FrameworkError, HttpResponse, MagnetarConfig, MiddlewareRegistry, RateLimiterDriver,
    Registration, Request, Response, Router, SessionMiddleware, SlidingWindowConfig, UserProvider,
    handle_request, init_magnetar,
};
use tokio::sync::OnceCell;

const PASSWORD: &str = "correct-horse-battery";

static SETUP: OnceCell<()> = OnceCell::const_new();

/// The Magnetar user the framework provider below resolves.
static ACCOUNT: OnceCell<Account> = OnceCell::const_new();

#[derive(Clone)]
struct Account {
    id: String,
    email: String,
}

struct AllowingLimiter;

#[async_trait]
impl RateLimiterDriver for AllowingLimiter {
    async fn try_acquire(&self, _: &str, _: &SlidingWindowConfig) -> Result<bool, FrameworkError> {
        Ok(true)
    }

    async fn retry_after(
        &self,
        _: &str,
        _: &SlidingWindowConfig,
    ) -> Result<Option<Duration>, FrameworkError> {
        Ok(None)
    }
}

struct TwoFactorMigrator;

impl MigratorTrait for TwoFactorMigrator {
    fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
        vec![
            Box::new(TwoFactorMigration),
            Box::new(TwoFactorReplayMigration),
            Box::new(TwoFactorAttemptsMigration),
        ]
    }
}

/// Install the default Magnetar engine, the framework 2FA tables, and a
/// session guard whose provider resolves the registered Magnetar user, the
/// way an application that keeps its own login forms is wired.
async fn setup() -> Account {
    SETUP
        .get_or_init(|| async {
            Crypt::init(EncryptionKey::generate());
            App::bind::<dyn RateLimiterDriver>(Arc::new(AllowingLimiter));
            // A file-backed fixture survives the per-test runtime boundary
            // that would reopen an in-memory pool against an empty database.
            let db_path = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
                "magnetar-framework-login-{}.sqlite",
                std::process::id()
            ));
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
            TwoFactorMigrator::up(&connection, None)
                .await
                .expect("two-factor migrations");
            connection
                .execute_unprepared(
                    "CREATE TABLE sessions (\
                        id TEXT PRIMARY KEY, \
                        user_id TEXT NULL, \
                        payload TEXT NOT NULL, \
                        csrf_token TEXT NOT NULL, \
                        last_activity TEXT NOT NULL\
                     )",
                )
                .await
                .expect("sessions table");
            App::singleton(DbConnection::from_raw(connection));

            let user = Auth::password()
                .register("framework-login@example.test", PASSWORD)
                .await
                .expect("register the Magnetar user")
                .created()
                .expect("registration creates a new account");
            ACCOUNT
                .set(Account {
                    id: user.id.to_string(),
                    email: user.email.clone(),
                })
                .unwrap_or_else(|_| panic!("account is set once"));

            App::singleton(AuthManager::new(AuthConfig::default()));
            Auth::register_provider("users", Arc::new(AccountProvider))
                .expect("register users provider");
        })
        .await;
    ACCOUNT.get().expect("account registered").clone()
}

struct AccountUser(Account);

impl Authenticatable for AccountUser {
    fn get_auth_identifier(&self) -> String {
        self.0.id.clone()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn into_arc_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

impl TwoFactorUser for Account {
    fn user_id(&self) -> &str {
        &self.id
    }
    fn email(&self) -> &str {
        &self.email
    }
}

/// The application's own provider: it checks the password itself and
/// resolves the same ids the Magnetar user table holds.
struct AccountProvider;

#[async_trait]
impl UserProvider for AccountProvider {
    async fn retrieve_by_id(
        &self,
        id: &str,
    ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        let account = ACCOUNT.get().expect("account registered");
        Ok((id == account.id).then(|| Arc::new(AccountUser(account.clone())) as _))
    }

    async fn retrieve_by_credentials(
        &self,
        credentials: &serde_json::Value,
    ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        let account = ACCOUNT.get().expect("account registered");
        let email = credentials.get("email").and_then(|v| v.as_str());
        Ok((email == Some(account.email.as_str()))
            .then(|| Arc::new(AccountUser(account.clone())) as _))
    }

    async fn validate_credentials(
        &self,
        _user: &dyn Authenticatable,
        credentials: &serde_json::Value,
    ) -> Result<bool, FrameworkError> {
        Ok(credentials.get("password").and_then(|v| v.as_str()) == Some(PASSWORD))
    }
}

fn failure(error: FrameworkError) -> Response {
    Err(HttpResponse::text(error.to_string()).status(error.status_code()))
}

fn header(request: &Request, name: &str) -> String {
    request.header(name).unwrap_or_default().to_owned()
}

/// The manual's handlers: a login form that gates on 2FA, the challenge
/// form, and a page that reports who is signed in.
fn router() -> Router {
    Router::new()
        // Register, then sign a new account in: the pattern an app with its
        // own registration form writes. An address that already has an
        // account gets the same answer, and nobody is signed in.
        .get("/register", |request: Request| async move {
            let registration = match Auth::password()
                .register(
                    &header(&request, "x-email"),
                    &header(&request, "x-password"),
                )
                .await
            {
                Ok(registration) => registration,
                Err(error) => return failure(error),
            };
            if let Registration::Created(user) = registration
                && let Err(error) = Auth::login_id(user.id.to_string())
            {
                return failure(error);
            }
            Ok(HttpResponse::text("registered"))
        })
        .get("/login-id", |request: Request| async move {
            match Auth::login_id(header(&request, "x-user-id")) {
                Ok(()) => Ok(HttpResponse::text("signed in")),
                Err(error) => failure(error),
            }
        })
        .get("/login", |request: Request| async move {
            let credentials =
                Credentials::password(header(&request, "x-email"), header(&request, "x-password"));
            let user = match Auth::attempt(&credentials, false).await {
                Ok(Some(user)) => user,
                Ok(None) => return Ok(HttpResponse::text("invalid credentials").status(401)),
                Err(error) => return failure(error),
            };
            let user_id = user.get_auth_identifier();
            match TwoFactor::is_enabled_by_id(&user_id).await {
                Ok(true) => match TwoFactor::start_challenge(user_id, false).await {
                    Ok(()) => Ok(HttpResponse::text("two-factor challenge")),
                    Err(error) => failure(error),
                },
                Ok(false) => Ok(HttpResponse::text("signed in")),
                Err(error) => failure(error),
            }
        })
        .get("/two-factor-challenge", |request: Request| async move {
            match TwoFactor::complete_challenge(&header(&request, "x-code")).await {
                Ok(user) => Ok(HttpResponse::text(user.id.to_string())),
                Err(error) => failure(error),
            }
        })
        .get("/whoami", |_request: Request| async {
            Ok(HttpResponse::text(
                Auth::id().unwrap_or_else(|| "guest".to_owned()),
            ))
        })
        .into()
}

/// A browser: it keeps the cookies the server sets and sends them back.
struct Browser {
    addr: SocketAddr,
    cookies: HashMap<String, String>,
}

impl Browser {
    async fn open() -> Self {
        let mut config = SessionConfig::default();
        config.cookie_secure = false;
        // The framework's database driver: it migrates a promoted 2FA
        // session atomically, as production does.
        let middleware = SessionMiddleware::new(config);
        let registry = Arc::new(MiddlewareRegistry::new().append(middleware));
        let router = Arc::new(router());

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral listener");
        let addr = listener.local_addr().expect("local_addr");
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let router = router.clone();
                let registry = registry.clone();
                tokio::spawn(async move {
                    let service = service_fn(move |request: hyper::Request<Incoming>| {
                        let router = router.clone();
                        let registry = registry.clone();
                        async move {
                            Ok::<_, Infallible>(handle_request(router, registry, request).await)
                        }
                    });
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        });
        Self {
            addr,
            cookies: HashMap::new(),
        }
    }

    /// Send a GET with the jar's cookies; returns `(status, body)`.
    async fn get(&mut self, path: &str, headers: &[(&str, &str)]) -> (u16, String) {
        let stream = tokio::net::TcpStream::connect(self.addr)
            .await
            .expect("connect");
        let (mut sender, connection) =
            hyper::client::conn::http1::handshake::<_, Full<Bytes>>(TokioIo::new(stream))
                .await
                .expect("handshake");
        tokio::spawn(async move {
            let _ = connection.await;
        });
        let mut builder = hyper::Request::builder()
            .method("GET")
            .uri(path)
            .header("Host", "localhost");
        if !self.cookies.is_empty() {
            let jar = self
                .cookies
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<_>>()
                .join("; ");
            builder = builder.header("Cookie", jar);
        }
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let response = tokio::time::timeout(
            Duration::from_secs(10),
            sender.send_request(builder.body(Full::new(Bytes::new())).expect("request")),
        )
        .await
        .expect("response in time")
        .expect("response");
        let (parts, body) = response.into_parts();
        for set_cookie in parts.headers.get_all("set-cookie") {
            let set_cookie = set_cookie.to_str().expect("ascii set-cookie");
            let pair = set_cookie.split(';').next().unwrap_or_default();
            let Some((name, value)) = pair.split_once('=') else {
                continue;
            };
            let expired = set_cookie.to_ascii_lowercase().contains("max-age=0");
            if expired || value.is_empty() {
                self.cookies.remove(name);
            } else {
                self.cookies.insert(name.to_owned(), value.to_owned());
            }
        }
        let body = body.collect().await.expect("body").to_bytes();
        (
            parts.status.as_u16(),
            String::from_utf8_lossy(&body).into_owned(),
        )
    }

    async fn whoami(&mut self) -> String {
        let (status, body) = self.get("/whoami", &[]).await;
        assert_eq!(status, 200, "{body}");
        body
    }
}

fn current_code(otpauth_url: &str) -> String {
    let url = url::Url::parse(otpauth_url).expect("otpauth url");
    let secret = url
        .query_pairs()
        .find(|(key, _)| key == "secret")
        .map(|(_, value)| value.into_owned())
        .expect("secret query parameter");
    let bytes = totp_rs::Secret::Encoded(secret)
        .to_bytes()
        .expect("decode secret");
    totp_rs::TOTP::new(
        totp_rs::Algorithm::SHA1,
        6,
        1,
        30,
        bytes,
        None,
        "label".into(),
    )
    .expect("totp")
    .generate_current()
    .expect("generate code")
}

/// The manual's challenge flow with remember-me off: password login demotes
/// to a pending challenge, the challenge form promotes it, and the user is
/// still signed in on the request after that.
#[tokio::test]
async fn a_completed_two_factor_challenge_without_remember_me_stays_signed_in() {
    let account = setup().await;
    let enrollment = TwoFactor::enroll(&account).await.expect("enroll");
    let code = current_code(&enrollment.otpauth_url);
    TwoFactor::confirm(&account, &code).await.expect("confirm");

    let mut browser = Browser::open().await;
    let (status, body) = browser
        .get(
            "/login",
            &[("x-email", &account.email), ("x-password", PASSWORD)],
        )
        .await;
    assert_eq!((status, body.as_str()), (200, "two-factor challenge"));
    assert_eq!(browser.whoami().await, "guest", "pending is not signed in");

    let (status, body) = browser
        .get("/two-factor-challenge", &[("x-code", &code)])
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, account.id);

    assert_eq!(
        browser.whoami().await,
        account.id,
        "the promoted user must still be signed in on the next request"
    );
    TwoFactor::disable(&account).await.expect("disable");
}

/// `Auth::attempt` on the default session guard, as the backend scaffold's
/// login form calls it.
#[tokio::test]
async fn a_password_login_through_the_session_guard_stays_signed_in() {
    let account = setup().await;
    let mut browser = Browser::open().await;

    let (status, body) = browser
        .get(
            "/login",
            &[("x-email", &account.email), ("x-password", PASSWORD)],
        )
        .await;
    assert_eq!((status, body.as_str()), (200, "signed in"));
    assert_eq!(
        browser.whoami().await,
        account.id,
        "the attempt's login must still hold on the next request"
    );
}

/// `Auth::login_id`, the synchronous primitive.
#[tokio::test]
async fn a_login_by_id_stays_signed_in() {
    let account = setup().await;
    let mut browser = Browser::open().await;

    let (status, body) = browser
        .get("/login-id", &[("x-user-id", &account.id)])
        .await;
    assert_eq!((status, body.as_str()), (200, "signed in"));
    assert_eq!(
        browser.whoami().await,
        account.id,
        "the login must still hold on the next request"
    );
    let sessions = suprnova::magnetar_integration::list_sessions(&account.id)
        .await
        .expect("list Magnetar sessions");
    assert!(
        !sessions.is_empty(),
        "the login is backed by a Magnetar session that revocation reaches"
    );
}

/// A login the engine cannot back with a session - here an id it does not
/// know - fails closed: the response is an error, and nothing signs in.
#[tokio::test]
async fn a_login_the_engine_cannot_bind_fails_closed() {
    setup().await;
    let mut browser = Browser::open().await;

    let (status, _body) = browser.get("/login-id", &[("x-user-id", "999999")]).await;
    assert_eq!(status, 500, "an unbindable login must not report success");
    assert_eq!(browser.whoami().await, "guest");
}

/// Registering an address that already has an account, with another
/// password, must neither hand back that account nor sign anyone in as it,
/// on this request or the ones after it.
#[tokio::test]
async fn registering_an_existing_address_never_signs_in_as_its_owner() {
    let victim = setup().await;
    let mut attacker = Browser::open().await;

    let (status, body) = attacker
        .get(
            "/register",
            &[
                ("x-email", &victim.email),
                ("x-password", "attacker-chosen-pass"),
            ],
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body, "registered",
        "the answer matches a fresh registration"
    );
    for _ in 0..2 {
        let whoami = attacker.whoami().await;
        assert_ne!(
            whoami, victim.id,
            "the attacker must never act as the owner"
        );
        assert_eq!(whoami, "guest");
    }

    // A fresh address registers and signs in as the new account.
    let mut newcomer = Browser::open().await;
    let (status, body) = newcomer
        .get(
            "/register",
            &[
                ("x-email", "newcomer@example.test"),
                ("x-password", "newcomer-password"),
            ],
        )
        .await;
    assert_eq!((status, body.as_str()), (200, "registered"));
    let newcomer_id = newcomer.whoami().await;
    assert_ne!(newcomer_id, "guest");
    assert_ne!(newcomer_id, victim.id);
}

/// Second-factor failures have a counter of their own. A correct password
/// check between wrong codes must not clear them: otherwise four wrong
/// codes and one sign-in, repeated, guess codes forever.
#[tokio::test]
async fn a_password_success_does_not_clear_second_factor_failures() {
    let account = setup().await;
    let enrollment = TwoFactor::enroll(&account).await.expect("enroll");
    let code = current_code(&enrollment.otpauth_url);
    TwoFactor::confirm(&account, &code).await.expect("confirm");

    let mut evaluated = 0;
    let mut refused = 0;
    for _round in 0..2 {
        for _ in 0..4 {
            match TwoFactor::verify(&account, "000000").await {
                Ok(false) => evaluated += 1,
                Err(error) if error.status_code() == 429 => refused += 1,
                other => panic!("unexpected verify outcome: {other:?}"),
            }
        }
        Auth::password()
            .authenticate(&account.email, PASSWORD, None, None)
            .await
            .expect("the password is right");
    }
    assert_eq!(evaluated, 5, "the counter locks at the threshold");
    assert_eq!(refused, 3, "every later guess is refused");
    let error = TwoFactor::verify(&account, &code)
        .await
        .expect_err("the right code is refused while locked");
    assert_eq!(error.status_code(), 429);
}
