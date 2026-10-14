//! HRD-007: every provider-backed sign-in records the user's authentication
//! epoch, and every request that resolves the user from the session compares
//! it with the user's current one.
//!
//! The scenario is the one a password reset leaves behind when the eager
//! revocation of the user's sessions fails: the password hash changes, the
//! session store refuses to destroy the user's sessions, and the browser
//! that holds a session cookie from before the change comes back. Each test
//! drives a real `SessionMiddleware` over an in-memory store, with
//! `AuthMiddleware` or `BasicAuthMiddleware` in front of a route, so the
//! answer is the one an application would send.
//!
//! The remember-me half of HRD-007 is tested beside the other remember-me
//! tests, in `remember_me.rs`, where the `remember_tokens` table exists.
//!
//! The auth manager, the provider and the session store that revocation
//! resolves are bound in a container scope per test. Each test still runs
//! alone in a child process (see `own_process`): `SessionMiddleware`'s
//! constructor binds its store into the process container when none is
//! bound there, and this file's store refuses every revocation, which
//! would fail the revocations of the other files of this binary.

#![cfg(feature = "testing")]

use std::any::Any;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use async_trait::async_trait;
use once_cell::sync::Lazy;
use tokio::runtime::Runtime;

use suprnova::auth::request_state;
use suprnova::middleware::{Middleware, Next};
use suprnova::session::{DestroyedSessions, SessionConfig, SessionData, SessionStore};
use suprnova::testing::{TestContainer, TestDatabase};
use suprnova::{
    Auth, AuthConfig, AuthManager, AuthMiddleware, Authenticatable, BasicAuthMiddleware,
    CanResetPassword, Credentials, DB, EloquentUserProvider, FrameworkError, GuardConfig,
    HttpResponse, Model, MustVerifyEmail, SessionMiddleware, UserProvider, model,
};

use crate::eloquent_provider::TestUser;

static RT: Lazy<Runtime> = Lazy::new(|| Runtime::new().expect("tokio runtime"));

/// The plaintext every test user signs in with.
const PASSWORD: &str = "correct horse battery staple";

/// The stored form of `password`. The provider below checks a password by
/// rebuilding it, so the tests need no hashing cost; the epoch only reads
/// the string the model returns.
fn stored_hash(password: &str, generation: u32) -> String {
    format!("test-hash:{generation}:{password}")
}

#[derive(Clone)]
struct EpochUser {
    id: String,
    password_hash: String,
    /// Whether the model returns its hash from `get_auth_password`. A model
    /// that does not (an OAuth-only or passkey account) exposes no epoch.
    exposes_hash: bool,
}

impl Authenticatable for EpochUser {
    fn get_auth_identifier(&self) -> String {
        self.id.clone()
    }

    fn get_auth_password(&self) -> Option<&str> {
        self.exposes_hash.then_some(self.password_hash.as_str())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_arc_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

/// A provider over a map of users keyed by id, with emails for `attempt`.
struct EpochProvider {
    users: StdMutex<HashMap<String, (String, EpochUser)>>,
    generation: StdMutex<u32>,
    /// Whether `retrieve_by_id` fails, as a database outage makes it fail.
    lookups_fail: AtomicBool,
}

impl EpochProvider {
    fn with_user(id: &str, email: &str, exposes_hash: bool) -> Arc<Self> {
        let user = EpochUser {
            id: id.to_owned(),
            password_hash: stored_hash(PASSWORD, 0),
            exposes_hash,
        };
        let mut users = HashMap::new();
        users.insert(id.to_owned(), (email.to_owned(), user));
        Arc::new(Self {
            users: StdMutex::new(users),
            generation: StdMutex::new(0),
            lookups_fail: AtomicBool::new(false),
        })
    }

    /// Make `retrieve_by_id` fail (`true`) or answer again (`false`).
    fn fail_lookups(&self, fail: bool) {
        self.lookups_fail.store(fail, Ordering::SeqCst);
    }

    /// Rewrite the user's password through the provider, as a password
    /// reset does: the same plaintext gets a new hash, as a new salt gives
    /// it.
    async fn rewrite_password(&self, id: &str) {
        let generation = {
            let mut generation = self.generation.lock().unwrap();
            *generation += 1;
            *generation
        };
        self.set_password(id, &stored_hash(PASSWORD, generation))
            .await
            .expect("the provider stores the new hash");
    }
}

#[async_trait]
impl UserProvider for EpochProvider {
    async fn retrieve_by_id(
        &self,
        id: &str,
    ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        if self.lookups_fail.load(Ordering::SeqCst) {
            return Err(FrameworkError::database("the users table is unavailable"));
        }
        Ok(self
            .users
            .lock()
            .unwrap()
            .get(id)
            .map(|(_, user)| Arc::new(user.clone()) as Arc<dyn Authenticatable>))
    }

    async fn retrieve_by_credentials(
        &self,
        credentials: &serde_json::Value,
    ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        let email = credentials.get("email").and_then(serde_json::Value::as_str);
        Ok(self
            .users
            .lock()
            .unwrap()
            .values()
            .find(|(stored, _)| Some(stored.as_str()) == email)
            .map(|(_, user)| Arc::new(user.clone()) as Arc<dyn Authenticatable>))
    }

    async fn validate_credentials(
        &self,
        user: &dyn Authenticatable,
        credentials: &serde_json::Value,
    ) -> Result<bool, FrameworkError> {
        let Some(password) = credentials
            .get("password")
            .and_then(serde_json::Value::as_str)
        else {
            return Ok(false);
        };
        let generation = *self.generation.lock().unwrap();
        Ok(self
            .users
            .lock()
            .unwrap()
            .get(&user.get_auth_identifier())
            .is_some_and(|(_, stored)| stored.password_hash == stored_hash(password, generation)))
    }

    async fn set_password(&self, id: &str, hashed: &str) -> Result<(), FrameworkError> {
        let mut users = self.users.lock().unwrap();
        let (_, user) = users
            .get_mut(id)
            .ok_or_else(|| FrameworkError::internal("no such user"))?;
        user.password_hash = hashed.to_owned();
        Ok(())
    }
}

/// A session store in memory whose revocation of a user's sessions fails,
/// as a database outage during a password reset makes it fail.
#[derive(Default)]
struct RevocationFailsStore {
    sessions: StdMutex<HashMap<String, SessionData>>,
}

impl RevocationFailsStore {
    /// Whether any stored session still holds the guard `guard` signed in
    /// as `user_id`.
    fn holds(&self, guard: &str, user_id: &str) -> bool {
        self.sessions
            .lock()
            .unwrap()
            .values()
            .any(|session| session.is_signed_in_as(guard, user_id))
    }

    /// The stored guard entries of every session that holds the guard
    /// `guard` signed in as `user_id`.
    fn guard_entries(&self, guard: &str, user_id: &str) -> Vec<serde_json::Value> {
        self.sessions
            .lock()
            .unwrap()
            .values()
            .filter(|session| session.is_signed_in_as(guard, user_id))
            .filter_map(|session| {
                session
                    .data
                    .get("_auth_guards")
                    .and_then(|guards| guards.get(guard))
                    .cloned()
            })
            .collect()
    }
}

#[async_trait]
impl SessionStore for RevocationFailsStore {
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

    async fn destroy(&self, id: &str) -> Result<(), FrameworkError> {
        self.sessions.lock().unwrap().remove(id);
        Ok(())
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
        Ok(0)
    }
}

fn session_config() -> SessionConfig {
    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    config
}

fn percent_encode_cookie_value(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'=' => encoded.push_str("%3D"),
            b'+' => encoded.push_str("%2B"),
            b'/' => encoded.push_str("%2F"),
            b';' => encoded.push_str("%3B"),
            b' ' => encoded.push_str("%20"),
            b',' => encoded.push_str("%2C"),
            _ => encoded.push(byte as char),
        }
    }
    encoded
}

/// A request for `/` carrying `cookie` as the session cookie, and `headers`.
async fn request(
    config: &SessionConfig,
    cookie: Option<&str>,
    headers: &[(&str, &str)],
) -> suprnova::Request {
    use bytes::Bytes;
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper_util::rt::TokioIo;
    use std::convert::Infallible;
    use tokio::io::AsyncWriteExt;
    use tokio::sync::oneshot;

    let cookie_header = cookie
        .map(|value| {
            format!(
                "Cookie: {}={}\r\n",
                config.cookie_name,
                percent_encode_cookie_value(value)
            )
        })
        .unwrap_or_default();
    let other_headers = headers
        .iter()
        .map(|(name, value)| format!("{name}: {value}\r\n"))
        .collect::<String>();
    let http_bytes = format!(
        "GET / HTTP/1.1\r\nHost: localhost\r\n{cookie_header}{other_headers}Content-Length: 0\r\n\r\n"
    )
    .into_bytes();
    let (req_tx, req_rx) = oneshot::channel();
    let req_tx = StdMutex::new(Some(req_tx));
    let (client_io, server_io) = tokio::io::duplex(http_bytes.len() + 64 * 1024);

    tokio::spawn(async move {
        let service = service_fn(move |request: hyper::Request<hyper::body::Incoming>| {
            if let Some(tx) = req_tx.lock().unwrap().take() {
                let _ = tx.send(suprnova::Request::new(request));
            }
            async {
                Ok::<_, Infallible>(hyper::Response::new(
                    http_body_util::Full::new(Bytes::new()),
                ))
            }
        });
        let _ = http1::Builder::new()
            .serve_connection(TokioIo::new(server_io), service)
            .await;
    });

    let mut client = client_io;
    client.write_all(&http_bytes).await.unwrap();
    drop(client);
    req_rx.await.expect("request captured")
}

/// What one request answered: its status, the session cookie it set, and
/// whether it carried a Basic challenge.
struct Answer {
    status: u16,
    session_cookie: Option<String>,
    challenged: bool,
}

/// Serve one request through `middleware`, with `handler` behind it, inside
/// the request's auth scope as the server installs it.
async fn serve(
    middleware: &SessionMiddleware,
    config: &SessionConfig,
    cookie: Option<&str>,
    handler: Next,
) -> Answer {
    serve_with_headers(middleware, config, cookie, &[], handler).await
}

/// [`serve`] for a request that also carries `headers`.
async fn serve_with_headers(
    middleware: &SessionMiddleware,
    config: &SessionConfig,
    cookie: Option<&str>,
    headers: &[(&str, &str)],
    handler: Next,
) -> Answer {
    let response = request_state::request_state_scope_for_test(
        middleware.handle(request(config, cookie, headers).await, handler),
    )
    .await;
    let response = match response {
        Ok(response) | Err(response) => response,
    };
    let status = response.status_code();
    let response = response.into_hyper();
    let challenged = response
        .headers()
        .get("www-authenticate")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("Basic realm="));
    let prefix = format!("{}=", config.cookie_name);
    let session_cookie = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find_map(|header| {
            header
                .strip_prefix(&prefix)
                .and_then(|rest| rest.split(';').next())
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        });
    Answer {
        status,
        session_cookie,
        challenged,
    }
}

/// A route behind `AuthMiddleware::new()` for `guard`, or the default
/// guard: `200` for a signed-in user, the middleware's `401` for a guest.
fn authenticated_route(guard: Option<&'static str>) -> Next {
    let route: Next = Arc::new(|_request| Box::pin(async { Ok(HttpResponse::text("dashboard")) }));
    Arc::new(move |request| {
        let route = route.clone();
        Box::pin(async move {
            let middleware = match guard {
                Some(guard) => AuthMiddleware::new().for_guard(guard),
                None => AuthMiddleware::new(),
            };
            middleware.handle(request, route).await
        })
    })
}

/// A route behind `BasicAuthMiddleware::new()` for `guard`, or the default
/// guard: `200` for a signed-in session or valid credentials, the
/// middleware's `401` challenge otherwise.
fn basic_route(guard: Option<&'static str>) -> Next {
    let route: Next = Arc::new(|_request| Box::pin(async { Ok(HttpResponse::text("dashboard")) }));
    Arc::new(move |request| {
        let route = route.clone();
        Box::pin(async move {
            let middleware = match guard {
                Some(guard) => BasicAuthMiddleware::new().for_guard(guard),
                None => BasicAuthMiddleware::new(),
            };
            middleware.handle(request, route).await
        })
    })
}

/// The `Authorization` header value of HTTP Basic credentials.
fn basic_header(user: &str, password: &str) -> String {
    use base64::Engine as _;

    let encoded = base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"));
    format!("Basic {encoded}")
}

/// A handler that signs `email` in through `Auth::attempt`.
fn attempt_route(email: &'static str) -> Next {
    attempt_route_for(None, email)
}

/// A handler that signs `email` in through the `attempt` of `guard`, or of
/// the default guard.
fn attempt_route_for(guard: Option<&'static str>, email: &'static str) -> Next {
    Arc::new(move |_request| {
        Box::pin(async move {
            let credentials = Credentials::password(email, PASSWORD);
            let attempted = match guard {
                Some(guard) => match Auth::stateful_guard(guard) {
                    Ok(guard) => guard.attempt(&credentials, false).await,
                    Err(error) => Err(error),
                },
                None => Auth::attempt(&credentials, false).await,
            };
            match attempted {
                Ok(Some(_)) => Ok(HttpResponse::text("signed in")),
                Ok(None) => Err(HttpResponse::text("wrong password").status(422)),
                Err(error) => Err(HttpResponse::text(error.to_string()).status(500)),
            }
        })
    })
}

/// A handler that signs user `"7"` in through `Auth::login_id`.
fn login_id_route() -> Next {
    Arc::new(|_request| {
        Box::pin(async {
            Auth::login_id("7")
                .map(|()| HttpResponse::text("signed in"))
                .map_err(|error| HttpResponse::text(error.to_string()).status(500))
        })
    })
}

/// How a handler writes the signed-in user's new password through the
/// model: the two write paths an application's own password form uses.
#[derive(Clone, Copy)]
enum PasswordWrite {
    /// Set the hash on the loaded model and `save` it.
    Save,
    /// `update` the loaded model with the hash as an attribute.
    Update,
}

/// An in-memory `users` table with one user, `one@example.test`, whose
/// password is [`PASSWORD`], for `EloquentUserProvider<TestUser>`: the
/// provider and the write path a scaffolded application's own password
/// form uses. Create it inside [`in_scope`], where the connection registers
/// in the test's container, and keep it for the whole test.
async fn eloquent_user() -> TestDatabase {
    let db = TestDatabase::sqlite_memory()
        .await
        .expect("an in-memory database");
    db.execute_unprepared(
        "CREATE TABLE users (\
            id INTEGER PRIMARY KEY AUTOINCREMENT, \
            email TEXT NOT NULL, \
            password TEXT NOT NULL, \
            email_verified_at TEXT\
         )",
    )
    .await
    .expect("the users table");
    // A bcrypt hash holds no quote, so it is interpolated as is.
    let hash = suprnova::hash(PASSWORD).expect("a hash of the password");
    db.execute_unprepared(&format!(
        "INSERT INTO users (email, password) VALUES ('one@example.test', '{hash}')"
    ))
    .await
    .expect("the user row");
    db
}

/// A route behind `AuthMiddleware::new()` whose handler changes the
/// signed-in user's own password through the model, by `write`: a new
/// hash of the same plaintext, as a password form stores one.
fn change_password_route(write: PasswordWrite) -> Next {
    let handler: Next = Arc::new(move |_request| {
        Box::pin(async move {
            let failed = |error: FrameworkError| HttpResponse::text(error.to_string()).status(500);
            let id = Auth::id()
                .and_then(|id| id.parse::<i64>().ok())
                .ok_or_else(|| HttpResponse::text("no signed-in user").status(401))?;
            let mut user = TestUser::find(id)
                .await
                .map_err(failed)?
                .ok_or_else(|| HttpResponse::text("the user row is gone").status(500))?;
            let hash = suprnova::hash(PASSWORD).map_err(failed)?;
            match write {
                PasswordWrite::Save => {
                    user.set_password_hash(&hash);
                    user.save().await.map_err(failed)?;
                }
                PasswordWrite::Update => {
                    user.update(suprnova::attrs! { password: hash })
                        .await
                        .map_err(failed)?;
                }
            }
            Ok(HttpResponse::text("password changed"))
        })
    });
    Arc::new(move |request| {
        let handler = handler.clone();
        Box::pin(async move { AuthMiddleware::new().handle(request, handler).await })
    })
}

/// The kit's user shape with a hashed password: assigning the plaintext
/// stores a hash, as `AsHashed` does, over the same `users` table as
/// [`TestUser`].
#[model(
    table = "users",
    fillable = ["email", "password"],
    casts = { password = suprnova::AsHashed },
)]
pub struct HashedUser {
    pub id: i64,
    pub email: String,
    pub password: String,
    pub email_verified_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl Authenticatable for HashedUser {
    fn get_auth_identifier(&self) -> String {
        self.id.to_string()
    }
    fn get_auth_password(&self) -> Option<&str> {
        Some(&self.password)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn into_arc_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

impl MustVerifyEmail for HashedUser {
    fn email(&self) -> &str {
        &self.email
    }
    fn email_verified_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.email_verified_at
    }
    fn set_email_verified_at(&mut self, at: Option<chrono::DateTime<chrono::Utc>>) {
        self.email_verified_at = at;
    }
}

impl CanResetPassword for HashedUser {
    fn email_for_reset(&self) -> &str {
        &self.email
    }
    fn set_password_hash(&mut self, hash: &str) {
        self.password = hash.to_string();
    }
}

/// `handler` behind `AuthMiddleware::new()`: the request resolves and
/// checks the default guard's user before the handler runs.
fn behind_auth(handler: Next) -> Next {
    Arc::new(move |request| {
        let handler = handler.clone();
        Box::pin(async move { AuthMiddleware::new().handle(request, handler).await })
    })
}

/// The signed-in user's id, as the handlers below read it.
fn signed_in_id() -> Result<i64, HttpResponse> {
    Auth::id()
        .and_then(|id| id.parse::<i64>().ok())
        .ok_or_else(|| HttpResponse::text("no signed-in user").status(401))
}

fn failed(error: FrameworkError) -> HttpResponse {
    HttpResponse::text(error.to_string()).status(500)
}

/// A route behind `AuthMiddleware::new()` whose handler assigns the
/// plaintext to a [`HashedUser`] and saves: the cast stores a new hash.
fn hashed_plaintext_change_route() -> Next {
    behind_auth(Arc::new(|_request| {
        Box::pin(async {
            let id = signed_in_id()?;
            let mut user = HashedUser::find(id)
                .await
                .map_err(failed)?
                .ok_or_else(|| HttpResponse::text("the user row is gone").status(500))?;
            user.password = PASSWORD.to_owned();
            user.save().await.map_err(failed)?;
            Ok(HttpResponse::text("password changed"))
        })
    }))
}

/// A route with no authentication middleware whose handler writes user 1's
/// password through the model without resolving the signed-in user, as a
/// public reset page does.
fn unresolved_password_write_route() -> Next {
    Arc::new(|_request| {
        Box::pin(async {
            let mut user = TestUser::find(1_i64)
                .await
                .map_err(failed)?
                .ok_or_else(|| HttpResponse::text("the user row is gone").status(500))?;
            let hash = suprnova::hash(PASSWORD).map_err(failed)?;
            user.set_password_hash(&hash);
            user.save().await.map_err(failed)?;
            Ok(HttpResponse::text("password written"))
        })
    })
}

/// A route behind `AuthMiddleware::new()` whose handler sees another
/// session change the password (a raw write of a new hash, after this
/// request resolved its user) and then updates only the email through the
/// model.
fn email_update_after_foreign_change_route() -> Next {
    behind_auth(Arc::new(|_request| {
        Box::pin(async {
            let id = signed_in_id()?;
            let hash = suprnova::hash(PASSWORD).map_err(failed)?;
            DB::unprepared(&format!("UPDATE users SET password = '{hash}'"))
                .await
                .map_err(failed)?;
            let user = TestUser::find(id)
                .await
                .map_err(failed)?
                .ok_or_else(|| HttpResponse::text("the user row is gone").status(500))?;
            user.update(suprnova::attrs! { email: "moved@example.test" })
                .await
                .map_err(failed)?;
            Ok(HttpResponse::text("email changed"))
        })
    }))
}

/// A route behind `AuthMiddleware::new()` whose handler writes a new
/// password inside a manual transaction and rolls it back.
fn rolled_back_change_route() -> Next {
    behind_auth(Arc::new(|_request| {
        Box::pin(async {
            let id = signed_in_id()?;
            let user = TestUser::find(id)
                .await
                .map_err(failed)?
                .ok_or_else(|| HttpResponse::text("the user row is gone").status(500))?;
            let hash = suprnova::hash(PASSWORD).map_err(failed)?;
            let tx = DB::begin_transaction().await.map_err(failed)?;
            user.update_with_tx(&tx, suprnova::attrs! { password: hash })
                .await
                .map_err(failed)?;
            tx.rollback().await.map_err(failed)?;
            Ok(HttpResponse::text("password write rolled back"))
        })
    }))
}

/// A route behind `AuthMiddleware::new()` whose handler changes the
/// password inside `DB::transaction`, which commits when the closure
/// returns.
fn transactional_change_route() -> Next {
    behind_auth(Arc::new(|_request| {
        Box::pin(async {
            let id = signed_in_id()?;
            DB::transaction(move |_tx| {
                Box::pin(async move {
                    let mut user = TestUser::find(id)
                        .await?
                        .ok_or_else(|| FrameworkError::internal("the user row is gone"))?;
                    user.set_password_hash(&suprnova::hash(PASSWORD)?);
                    user.save().await
                })
            })
            .await
            .map_err(failed)?;
            Ok(HttpResponse::text("password changed"))
        })
    }))
}

/// Run `test` on [`RT`] in a container scope that binds the auth manager
/// `config`, the provider `users` under the name `"users"` (and any
/// `extra` providers), and `store` as the session store revocation
/// resolves.
fn in_scope<F: std::future::Future>(
    config: AuthConfig,
    providers: Vec<(&'static str, Arc<dyn UserProvider>)>,
    store: Arc<RevocationFailsStore>,
    test: F,
) -> F::Output {
    let _ = suprnova::crypto::_test_install_key(suprnova::EncryptionKey::generate());
    let test = Box::pin(test);
    RT.block_on(TestContainer::scope(async move {
        let manager = AuthManager::new(config);
        for (name, provider) in providers {
            manager.register_provider(name, provider);
        }
        TestContainer::singleton(manager);
        TestContainer::bind::<dyn SessionStore>(store);
        test.await
    }))
}

/// HRD-007's falsifier: a session signed in before a password change must
/// not answer an authenticated route after the change, although the eager
/// revocation of the user's sessions failed.
#[test]
fn hardening_session_signed_in_before_a_password_change_answers_as_a_guest() {
    crate::own_process::run_alone(
        "hardening::hardening_session_signed_in_before_a_password_change_answers_as_a_guest_child",
    );
}

#[test]
fn hardening_session_signed_in_before_a_password_change_answers_as_a_guest_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let provider = EpochProvider::with_user("7", "seven@example.test", true);
    let store = Arc::new(RevocationFailsStore::default());
    in_scope(
        AuthConfig::default(),
        vec![("users", provider.clone() as Arc<dyn UserProvider>)],
        store.clone(),
        async move {
            let config = session_config();
            let middleware = SessionMiddleware::with_store(config.clone(), store.clone());

            let signed_in = serve(
                &middleware,
                &config,
                None,
                attempt_route("seven@example.test"),
            )
            .await;
            assert_eq!(signed_in.status, 200, "the sign-in succeeds");
            let cookie = signed_in
                .session_cookie
                .expect("the sign-in sets a session cookie");

            let before = serve(
                &middleware,
                &config,
                Some(&cookie),
                authenticated_route(None),
            )
            .await;
            assert_eq!(
                before.status, 200,
                "the session answers the authenticated route before the change"
            );

            // A password reset: the hash is rewritten, and the eager
            // revocation of the user's sessions fails.
            provider.rewrite_password("7").await;
            assert!(
                suprnova::session::destroy_all_for_user("7").await.is_err(),
                "the store refuses the revocation"
            );
            assert!(store.holds("web", "7"), "the stale session is still stored");

            let after = serve(
                &middleware,
                &config,
                Some(&cookie),
                authenticated_route(None),
            )
            .await;
            assert_eq!(
                after.status, 401,
                "a session from before the password change answers as a guest"
            );
            assert!(
                !store.holds("web", "7"),
                "the session no longer holds the guard's identity"
            );
        },
    );
}

/// The happy path beside the falsifier: a session whose user's password
/// did not change keeps answering, request after request.
#[test]
fn hardening_session_stays_signed_in_while_the_password_is_unchanged() {
    crate::own_process::run_alone(
        "hardening::hardening_session_stays_signed_in_while_the_password_is_unchanged_child",
    );
}

#[test]
fn hardening_session_stays_signed_in_while_the_password_is_unchanged_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let provider = EpochProvider::with_user("7", "seven@example.test", true);
    let store = Arc::new(RevocationFailsStore::default());
    in_scope(
        AuthConfig::default(),
        vec![("users", provider as Arc<dyn UserProvider>)],
        store.clone(),
        async move {
            let config = session_config();
            let middleware = SessionMiddleware::with_store(config.clone(), store.clone());
            let cookie = serve(
                &middleware,
                &config,
                None,
                attempt_route("seven@example.test"),
            )
            .await
            .session_cookie
            .expect("the sign-in sets a session cookie");
            for _ in 0..3 {
                let answer = serve(
                    &middleware,
                    &config,
                    Some(&cookie),
                    authenticated_route(None),
                )
                .await;
                assert_eq!(answer.status, 200);
            }
            assert!(store.holds("web", "7"));
        },
    );
}

/// A user who changes their own password keeps the session that made the
/// change: the request that wrote the new hash through the model binds its
/// sign-in to the new epoch, while every other session of the user ends on
/// its next request.
#[test]
fn hardening_password_change_through_save_keeps_the_changing_session() {
    crate::own_process::run_alone(
        "hardening::hardening_password_change_through_save_keeps_the_changing_session_child",
    );
}

#[test]
fn hardening_password_change_through_save_keeps_the_changing_session_child() {
    if !crate::own_process::is_child() {
        return;
    }
    password_change_keeps_the_changing_session(PasswordWrite::Save);
}

/// The same through `update`, whose written model is assembled from the
/// attributes before the write.
#[test]
fn hardening_password_change_through_update_keeps_the_changing_session() {
    crate::own_process::run_alone(
        "hardening::hardening_password_change_through_update_keeps_the_changing_session_child",
    );
}

#[test]
fn hardening_password_change_through_update_keeps_the_changing_session_child() {
    if !crate::own_process::is_child() {
        return;
    }
    password_change_keeps_the_changing_session(PasswordWrite::Update);
}

fn password_change_keeps_the_changing_session(write: PasswordWrite) {
    let provider = Arc::new(EloquentUserProvider::<TestUser>::new());
    let store = Arc::new(RevocationFailsStore::default());
    in_scope(
        AuthConfig::default(),
        vec![("users", provider as Arc<dyn UserProvider>)],
        store.clone(),
        async move {
            let _db = eloquent_user().await;
            let config = session_config();
            let middleware = SessionMiddleware::with_store(config.clone(), store.clone());
            let mut cookies = Vec::new();
            for _ in 0..2 {
                let signed_in = serve(
                    &middleware,
                    &config,
                    None,
                    attempt_route("one@example.test"),
                )
                .await;
                assert_eq!(signed_in.status, 200, "the sign-in succeeds");
                cookies.push(
                    signed_in
                        .session_cookie
                        .expect("the sign-in sets a session cookie"),
                );
            }
            let (changing, other) = (&cookies[0], &cookies[1]);

            let changed = serve(
                &middleware,
                &config,
                Some(changing),
                change_password_route(write),
            )
            .await;
            assert_eq!(
                changed.status, 200,
                "the signed-in user changes their own password"
            );

            let kept = serve(
                &middleware,
                &config,
                Some(changing),
                authenticated_route(None),
            )
            .await;
            assert_eq!(
                kept.status, 200,
                "the session that changed the password stays signed in"
            );

            let ended = serve(&middleware, &config, Some(other), authenticated_route(None)).await;
            assert_eq!(
                ended.status, 401,
                "another session of the same user ends on its next request"
            );
        },
    );
}

/// Finding 1 of the adversary review: a sign-in is rebound only by a
/// request that resolved and checked it. A stale session whose request
/// writes the user's password without resolving the signed-in user (a
/// public reset page) stays stale and ends on its next protected request.
#[test]
fn hardening_stale_session_is_not_renewed_by_a_write_it_did_not_resolve() {
    crate::own_process::run_alone(
        "hardening::hardening_stale_session_is_not_renewed_by_a_write_it_did_not_resolve_child",
    );
}

#[test]
fn hardening_stale_session_is_not_renewed_by_a_write_it_did_not_resolve_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let provider = Arc::new(EloquentUserProvider::<TestUser>::new());
    let store = Arc::new(RevocationFailsStore::default());
    in_scope(
        AuthConfig::default(),
        vec![("users", provider as Arc<dyn UserProvider>)],
        store.clone(),
        async move {
            let _db = eloquent_user().await;
            let config = session_config();
            let middleware = SessionMiddleware::with_store(config.clone(), store.clone());
            let stale = sign_in(&middleware, &config).await;
            let changing = sign_in(&middleware, &config).await;
            let changed = serve(
                &middleware,
                &config,
                Some(&changing),
                change_password_route(PasswordWrite::Save),
            )
            .await;
            assert_eq!(
                changed.status, 200,
                "the other session changes the password"
            );

            let written = serve(
                &middleware,
                &config,
                Some(&stale),
                unresolved_password_write_route(),
            )
            .await;
            assert_eq!(written.status, 200, "the public write succeeds");

            let after = serve(
                &middleware,
                &config,
                Some(&stale),
                authenticated_route(None),
            )
            .await;
            assert_eq!(
                after.status, 401,
                "a write the stale session did not resolve does not renew it"
            );
        },
    );
}

/// Finding 2: a request that resolved its user and then writes the user
/// without touching the password column does not adopt a password change
/// another session committed meanwhile.
#[test]
fn hardening_an_unrelated_write_does_not_adopt_another_sessions_password_change() {
    crate::own_process::run_alone(
        "hardening::hardening_an_unrelated_write_does_not_adopt_another_sessions_password_change_child",
    );
}

#[test]
fn hardening_an_unrelated_write_does_not_adopt_another_sessions_password_change_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let provider = Arc::new(EloquentUserProvider::<TestUser>::new());
    let store = Arc::new(RevocationFailsStore::default());
    in_scope(
        AuthConfig::default(),
        vec![("users", provider as Arc<dyn UserProvider>)],
        store.clone(),
        async move {
            let _db = eloquent_user().await;
            let config = session_config();
            let middleware = SessionMiddleware::with_store(config.clone(), store.clone());
            let cookie = sign_in(&middleware, &config).await;
            let updated = serve(
                &middleware,
                &config,
                Some(&cookie),
                email_update_after_foreign_change_route(),
            )
            .await;
            assert_eq!(updated.status, 200, "the email update succeeds");

            let after = serve(
                &middleware,
                &config,
                Some(&cookie),
                authenticated_route(None),
            )
            .await;
            assert_eq!(
                after.status, 401,
                "the session that did not change the password ends after another session did"
            );
        },
    );
}

/// Finding 3: the sign-in is bound to the values sent to storage. A model
/// with a hashed password cast is assigned the plaintext and saved; the
/// session records the stored hash's epoch and stays signed in.
#[test]
fn hardening_password_change_through_a_hashed_cast_keeps_the_changing_session() {
    crate::own_process::run_alone(
        "hardening::hardening_password_change_through_a_hashed_cast_keeps_the_changing_session_child",
    );
}

#[test]
fn hardening_password_change_through_a_hashed_cast_keeps_the_changing_session_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let provider = Arc::new(EloquentUserProvider::<HashedUser>::new());
    let store = Arc::new(RevocationFailsStore::default());
    in_scope(
        AuthConfig::default(),
        vec![("users", provider as Arc<dyn UserProvider>)],
        store.clone(),
        async move {
            let _db = eloquent_user().await;
            let config = session_config();
            let middleware = SessionMiddleware::with_store(config.clone(), store.clone());
            let changing = sign_in(&middleware, &config).await;
            let other = sign_in(&middleware, &config).await;
            let changed = serve(
                &middleware,
                &config,
                Some(&changing),
                hashed_plaintext_change_route(),
            )
            .await;
            assert_eq!(changed.status, 200, "the plaintext is stored as a hash");

            let kept = serve(
                &middleware,
                &config,
                Some(&changing),
                authenticated_route(None),
            )
            .await;
            assert_eq!(
                kept.status, 200,
                "the session that changed the password stays signed in"
            );
            let ended = serve(
                &middleware,
                &config,
                Some(&other),
                authenticated_route(None),
            )
            .await;
            assert_eq!(ended.status, 401, "another session of the same user ends");
        },
    );
}

/// Finding 4: a password write that rolls back changes nothing, so the
/// sign-in stays bound as it was and keeps answering.
#[test]
fn hardening_a_rolled_back_password_write_leaves_the_sign_in_as_it_was() {
    crate::own_process::run_alone(
        "hardening::hardening_a_rolled_back_password_write_leaves_the_sign_in_as_it_was_child",
    );
}

#[test]
fn hardening_a_rolled_back_password_write_leaves_the_sign_in_as_it_was_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let provider = Arc::new(EloquentUserProvider::<TestUser>::new());
    let store = Arc::new(RevocationFailsStore::default());
    in_scope(
        AuthConfig::default(),
        vec![("users", provider as Arc<dyn UserProvider>)],
        store.clone(),
        async move {
            let _db = eloquent_user().await;
            let config = session_config();
            let middleware = SessionMiddleware::with_store(config.clone(), store.clone());
            let cookie = sign_in(&middleware, &config).await;
            let rolled_back = serve(
                &middleware,
                &config,
                Some(&cookie),
                rolled_back_change_route(),
            )
            .await;
            assert_eq!(rolled_back.status, 200, "the write rolls back");

            let after = serve(
                &middleware,
                &config,
                Some(&cookie),
                authenticated_route(None),
            )
            .await;
            assert_eq!(
                after.status, 200,
                "a rolled-back password write leaves the sign-in as it was"
            );
        },
    );
}

/// Finding 4, the other side: a password change inside `DB::transaction`
/// rebinds the sign-in once the transaction commits, and the other
/// session of the user ends.
#[test]
fn hardening_a_password_change_inside_a_transaction_rebinds_after_the_commit() {
    crate::own_process::run_alone(
        "hardening::hardening_a_password_change_inside_a_transaction_rebinds_after_the_commit_child",
    );
}

#[test]
fn hardening_a_password_change_inside_a_transaction_rebinds_after_the_commit_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let provider = Arc::new(EloquentUserProvider::<TestUser>::new());
    let store = Arc::new(RevocationFailsStore::default());
    in_scope(
        AuthConfig::default(),
        vec![("users", provider as Arc<dyn UserProvider>)],
        store.clone(),
        async move {
            let _db = eloquent_user().await;
            let config = session_config();
            let middleware = SessionMiddleware::with_store(config.clone(), store.clone());
            let changing = sign_in(&middleware, &config).await;
            let other = sign_in(&middleware, &config).await;
            let changed = serve(
                &middleware,
                &config,
                Some(&changing),
                transactional_change_route(),
            )
            .await;
            assert_eq!(changed.status, 200, "the transaction commits");

            let kept = serve(
                &middleware,
                &config,
                Some(&changing),
                authenticated_route(None),
            )
            .await;
            assert_eq!(
                kept.status, 200,
                "the session that changed the password stays signed in"
            );
            let ended = serve(
                &middleware,
                &config,
                Some(&other),
                authenticated_route(None),
            )
            .await;
            assert_eq!(ended.status, 401, "another session of the same user ends");
        },
    );
}

/// Sign user 1 in through `Auth::attempt` and return the session cookie.
async fn sign_in(middleware: &SessionMiddleware, config: &SessionConfig) -> String {
    let signed_in = serve(middleware, config, None, attempt_route("one@example.test")).await;
    assert_eq!(signed_in.status, 200, "the sign-in succeeds");
    signed_in
        .session_cookie
        .expect("the sign-in sets a session cookie")
}

/// HRD-007's exception: a user whose model exposes neither an epoch nor a
/// password hash records no epoch and stays signed in after any change.
#[test]
fn hardening_user_without_hash_or_epoch_records_none_and_stays_signed_in() {
    crate::own_process::run_alone(
        "hardening::hardening_user_without_hash_or_epoch_records_none_and_stays_signed_in_child",
    );
}

#[test]
fn hardening_user_without_hash_or_epoch_records_none_and_stays_signed_in_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let provider = EpochProvider::with_user("7", "seven@example.test", false);
    let store = Arc::new(RevocationFailsStore::default());
    in_scope(
        AuthConfig::default(),
        vec![("users", provider.clone() as Arc<dyn UserProvider>)],
        store.clone(),
        async move {
            let config = session_config();
            let middleware = SessionMiddleware::with_store(config.clone(), store.clone());
            let cookie = serve(
                &middleware,
                &config,
                None,
                attempt_route("seven@example.test"),
            )
            .await
            .session_cookie
            .expect("the sign-in sets a session cookie");

            let entries = store.guard_entries("web", "7");
            assert!(!entries.is_empty(), "the session holds the web guard");
            for entry in &entries {
                assert_eq!(
                    entry.get("auth_epoch"),
                    None,
                    "a user that exposes no epoch records none: {entry}"
                );
            }

            provider.rewrite_password("7").await;
            let after = serve(
                &middleware,
                &config,
                Some(&cookie),
                authenticated_route(None),
            )
            .await;
            assert_eq!(after.status, 200, "the sign-in is not checked");
            assert!(store.holds("web", "7"));
        },
    );
}

/// The epochs every stored entry of the guard `guard` for user `"7"`
/// records: `Some(text)` per entry, or `None` for an entry that records none.
fn stored_epochs(store: &RevocationFailsStore, guard: &str) -> Vec<Option<String>> {
    store
        .guard_entries(guard, "7")
        .iter()
        .map(|entry| {
            entry
                .get("auth_epoch")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
        })
        .collect()
}

/// `Auth::login_id` has no user at hand, so the request that signs in reads
/// the epoch through the guard's provider before the session is stored: the
/// stored sign-in carries a numeric epoch, never the unread mark, and a
/// password change before any authenticated request ends it.
#[test]
fn hardening_login_id_records_its_epoch_in_the_request_that_signs_in() {
    crate::own_process::run_alone(
        "hardening::hardening_login_id_records_its_epoch_in_the_request_that_signs_in_child",
    );
}

#[test]
fn hardening_login_id_records_its_epoch_in_the_request_that_signs_in_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let provider = EpochProvider::with_user("7", "seven@example.test", true);
    let store = Arc::new(RevocationFailsStore::default());
    in_scope(
        AuthConfig::default(),
        vec![("users", provider.clone() as Arc<dyn UserProvider>)],
        store.clone(),
        async move {
            let config = session_config();
            let middleware = SessionMiddleware::with_store(config.clone(), store.clone());
            let cookie = serve(&middleware, &config, None, login_id_route())
                .await
                .session_cookie
                .expect("the sign-in sets a session cookie");

            let epochs = stored_epochs(&store, "web");
            assert_eq!(epochs.len(), 1, "one session holds the web guard");
            for epoch in &epochs {
                assert!(
                    epoch
                        .as_deref()
                        .is_some_and(|epoch| epoch.parse::<u64>().is_ok()),
                    "the request that signs in records a numeric epoch: {epoch:?}"
                );
            }

            // The password changes before any authenticated request, and
            // the eager revocation fails.
            provider.rewrite_password("7").await;
            assert!(suprnova::session::destroy_all_for_user("7").await.is_err());

            let after = serve(
                &middleware,
                &config,
                Some(&cookie),
                authenticated_route(None),
            )
            .await;
            assert_eq!(
                after.status, 401,
                "a sign-in by id from before the password change answers as a guest"
            );
            assert!(!store.holds("web", "7"));
        },
    );
}

/// When the provider fails while the request that signed in by id reads the
/// epoch, the sign-in is not kept: the stored session holds no identity and
/// the next request is a guest's.
#[test]
fn hardening_login_id_whose_epoch_cannot_be_read_is_not_kept() {
    crate::own_process::run_alone(
        "hardening::hardening_login_id_whose_epoch_cannot_be_read_is_not_kept_child",
    );
}

#[test]
fn hardening_login_id_whose_epoch_cannot_be_read_is_not_kept_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let provider = EpochProvider::with_user("7", "seven@example.test", true);
    let store = Arc::new(RevocationFailsStore::default());
    in_scope(
        AuthConfig::default(),
        vec![("users", provider.clone() as Arc<dyn UserProvider>)],
        store.clone(),
        async move {
            let config = session_config();
            let middleware = SessionMiddleware::with_store(config.clone(), store.clone());

            provider.fail_lookups(true);
            let signed_in = serve(&middleware, &config, None, login_id_route()).await;
            provider.fail_lookups(false);
            assert!(
                !store.holds("web", "7"),
                "a sign-in whose epoch could not be read leaves no identity in the store"
            );

            let next = serve(
                &middleware,
                &config,
                signed_in.session_cookie.as_deref(),
                authenticated_route(None),
            )
            .await;
            assert_eq!(next.status, 401, "the next request is a guest's");
        },
    );
}

/// HRD-007's exception through `Auth::login_id`: the request that signs in
/// finds no epoch for a user whose model exposes neither value, so the
/// sign-in records none and a later password change leaves it alone.
#[test]
fn hardening_login_id_of_a_user_without_an_epoch_records_none() {
    crate::own_process::run_alone(
        "hardening::hardening_login_id_of_a_user_without_an_epoch_records_none_child",
    );
}

#[test]
fn hardening_login_id_of_a_user_without_an_epoch_records_none_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let provider = EpochProvider::with_user("7", "seven@example.test", false);
    let store = Arc::new(RevocationFailsStore::default());
    in_scope(
        AuthConfig::default(),
        vec![("users", provider.clone() as Arc<dyn UserProvider>)],
        store.clone(),
        async move {
            let config = session_config();
            let middleware = SessionMiddleware::with_store(config.clone(), store.clone());
            let cookie = serve(&middleware, &config, None, login_id_route())
                .await
                .session_cookie
                .expect("the sign-in sets a session cookie");
            assert_eq!(
                stored_epochs(&store, "web"),
                vec![None],
                "a user that exposes no epoch records none, and no unread mark is left"
            );

            provider.rewrite_password("7").await;
            let after = serve(
                &middleware,
                &config,
                Some(&cookie),
                authenticated_route(None),
            )
            .await;
            assert_eq!(after.status, 200, "the sign-in is not checked");
        },
    );
}

/// HRD-007 through `BasicAuthMiddleware::new()` for `guard` (`None` for the
/// default guard, stored as `"web"`), whose provider `provider` knows user
/// `"7"` at `email`. Two sessions sign the user in before the password
/// changes and the eager revocation fails. After the change, the first,
/// without an `Authorization` header, gets the `401` challenge and loses the
/// guard's identity; the second, with valid credentials, is signed in
/// afresh, and that sign-in passes without a header.
async fn basic_auth_challenges_a_session_from_before_a_password_change(
    guard: Option<&'static str>,
    email: &'static str,
    provider: Arc<EpochProvider>,
    store: Arc<RevocationFailsStore>,
) {
    let guard_name = guard.unwrap_or("web");
    let config = session_config();
    let middleware = SessionMiddleware::with_store(config.clone(), store.clone());
    let first = serve(&middleware, &config, None, attempt_route_for(guard, email))
        .await
        .session_cookie
        .expect("the first sign-in sets a session cookie");
    let second = serve(&middleware, &config, None, attempt_route_for(guard, email))
        .await
        .session_cookie
        .expect("the second sign-in sets a session cookie");

    let before = serve(&middleware, &config, Some(&first), basic_route(guard)).await;
    assert_eq!(
        before.status, 200,
        "the signed-in session passes Basic auth without a header"
    );

    provider.rewrite_password("7").await;
    assert!(
        suprnova::session::destroy_all_for_user("7").await.is_err(),
        "the store refuses the revocation"
    );
    assert_eq!(
        store.guard_entries(guard_name, "7").len(),
        2,
        "both sessions from before the change are still stored"
    );

    let stale = serve(&middleware, &config, Some(&first), basic_route(guard)).await;
    assert_eq!(
        stale.status, 401,
        "a session from before the password change does not pass Basic auth"
    );
    assert!(stale.challenged, "the 401 carries the Basic challenge");
    assert_eq!(
        store.guard_entries(guard_name, "7").len(),
        1,
        "the stale session no longer holds the guard's identity"
    );

    let header = basic_header(email, PASSWORD);
    let fresh = serve_with_headers(
        &middleware,
        &config,
        Some(&second),
        &[("Authorization", &header)],
        basic_route(guard),
    )
    .await;
    assert_eq!(
        fresh.status, 200,
        "valid credentials sign the user in afresh over a stale session"
    );
    let fresh_cookie = fresh
        .session_cookie
        .expect("the fresh sign-in sets a session cookie");
    assert_eq!(
        store.guard_entries(guard_name, "7").len(),
        1,
        "only the fresh sign-in holds the guard"
    );

    let after = serve(
        &middleware,
        &config,
        Some(&fresh_cookie),
        basic_route(guard),
    )
    .await;
    assert_eq!(
        after.status, 200,
        "the fresh sign-in passes Basic auth without a header"
    );
}

/// `BasicAuthMiddleware::new()` on the default guard resolves the session's
/// user and compares its epoch instead of trusting the stored id.
#[test]
fn hardening_basic_auth_challenges_a_session_from_before_a_password_change() {
    crate::own_process::run_alone(
        "hardening::hardening_basic_auth_challenges_a_session_from_before_a_password_change_child",
    );
}

#[test]
fn hardening_basic_auth_challenges_a_session_from_before_a_password_change_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let provider = EpochProvider::with_user("7", "seven@example.test", true);
    let store = Arc::new(RevocationFailsStore::default());
    in_scope(
        AuthConfig::default(),
        vec![("users", provider.clone() as Arc<dyn UserProvider>)],
        store.clone(),
        basic_auth_challenges_a_session_from_before_a_password_change(
            None,
            "seven@example.test",
            provider,
            store,
        ),
    );
}

/// The same through `BasicAuthMiddleware::new().for_guard("admin")`: the
/// named guard resolves its own user and compares its own epoch.
#[test]
fn hardening_basic_auth_for_a_named_guard_challenges_a_session_from_before_a_password_change() {
    crate::own_process::run_alone(
        "hardening::hardening_basic_auth_for_a_named_guard_challenges_a_session_from_before_a_password_change_child",
    );
}

#[test]
fn hardening_basic_auth_for_a_named_guard_challenges_a_session_from_before_a_password_change_child()
{
    if !crate::own_process::is_child() {
        return;
    }
    let users = EpochProvider::with_user("1", "one@example.test", true);
    let admins = EpochProvider::with_user("7", "admin@example.test", true);
    let store = Arc::new(RevocationFailsStore::default());
    in_scope(
        AuthConfig::default().guard("admin", GuardConfig::session("admins")),
        vec![
            ("users", users as Arc<dyn UserProvider>),
            ("admins", admins.clone() as Arc<dyn UserProvider>),
        ],
        store.clone(),
        basic_auth_challenges_a_session_from_before_a_password_change(
            Some("admin"),
            "admin@example.test",
            admins,
            store,
        ),
    );
}

/// Every guard records its own epoch: a password change of the `admin`
/// guard's user ends that guard's sign-in and leaves the web guard's
/// sign-in in the same session alone.
#[test]
fn hardening_stale_named_guard_sign_in_ends_only_that_guard() {
    crate::own_process::run_alone(
        "hardening::hardening_stale_named_guard_sign_in_ends_only_that_guard_child",
    );
}

#[test]
fn hardening_stale_named_guard_sign_in_ends_only_that_guard_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let users = EpochProvider::with_user("7", "seven@example.test", true);
    let admins = EpochProvider::with_user("9", "nine@example.test", true);
    let store = Arc::new(RevocationFailsStore::default());
    let config = AuthConfig::default().guard("admin", GuardConfig::session("admins"));
    in_scope(
        config,
        vec![
            ("users", users as Arc<dyn UserProvider>),
            ("admins", admins.clone() as Arc<dyn UserProvider>),
        ],
        store.clone(),
        async move {
            let config = session_config();
            let middleware = SessionMiddleware::with_store(config.clone(), store.clone());
            let both: Next = Arc::new(|_request| {
                Box::pin(async {
                    let web = Auth::attempt(
                        &Credentials::password("seven@example.test", PASSWORD),
                        false,
                    )
                    .await
                    .map_err(|error| HttpResponse::text(error.to_string()).status(500))?;
                    let admin = Auth::stateful_guard("admin")
                        .map_err(|error| HttpResponse::text(error.to_string()).status(500))?
                        .attempt(&Credentials::password("nine@example.test", PASSWORD), false)
                        .await
                        .map_err(|error| HttpResponse::text(error.to_string()).status(500))?;
                    match (web, admin) {
                        (Some(_), Some(_)) => Ok(HttpResponse::text("both signed in")),
                        _ => Err(HttpResponse::text("a sign-in failed").status(422)),
                    }
                })
            });
            let signed_in = serve(&middleware, &config, None, both).await;
            assert_eq!(signed_in.status, 200);
            let cookie = signed_in
                .session_cookie
                .expect("the sign-ins set a session cookie");

            admins.rewrite_password("9").await;

            let admin_route = serve(
                &middleware,
                &config,
                Some(&cookie),
                authenticated_route(Some("admin")),
            )
            .await;
            assert_eq!(admin_route.status, 401, "the admin sign-in is stale");
            assert!(!store.holds("admin", "9"));

            let web_route = serve(
                &middleware,
                &config,
                Some(&cookie),
                authenticated_route(None),
            )
            .await;
            assert_eq!(web_route.status, 200, "the web sign-in stands");
            assert!(store.holds("web", "7"));
        },
    );
}
