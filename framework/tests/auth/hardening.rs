//! HRD-007: every provider-backed sign-in records the user's authentication
//! epoch, and every request that resolves the user from the session compares
//! it with the user's current one.
//!
//! The scenario is the one a password reset leaves behind when the eager
//! revocation of the user's sessions fails: the password hash changes, the
//! session store refuses to destroy the user's sessions, and the browser
//! that holds a session cookie from before the change comes back. Each test
//! drives a real `SessionMiddleware` over an in-memory store, with
//! `AuthMiddleware` in front of a route, so the answer is the one an
//! application would send.
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
use std::sync::{Arc, Mutex as StdMutex};

use async_trait::async_trait;
use once_cell::sync::Lazy;
use tokio::runtime::Runtime;

use suprnova::auth::request_state;
use suprnova::middleware::{Middleware, Next};
use suprnova::session::{DestroyedSessions, SessionConfig, SessionData, SessionStore};
use suprnova::testing::TestContainer;
use suprnova::{
    Auth, AuthConfig, AuthManager, AuthMiddleware, Authenticatable, Credentials, FrameworkError,
    GuardConfig, HttpResponse, SessionMiddleware, UserProvider,
};

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
        })
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

/// A request for `/` carrying `cookie` as the session cookie.
async fn request(config: &SessionConfig, cookie: Option<&str>) -> suprnova::Request {
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
    let http_bytes =
        format!("GET / HTTP/1.1\r\nHost: localhost\r\n{cookie_header}Content-Length: 0\r\n\r\n")
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

/// What one request answered: its status and the session cookie it set.
struct Answer {
    status: u16,
    session_cookie: Option<String>,
}

/// Serve one request through `middleware`, with `handler` behind it, inside
/// the request's auth scope as the server installs it.
async fn serve(
    middleware: &SessionMiddleware,
    config: &SessionConfig,
    cookie: Option<&str>,
    handler: Next,
) -> Answer {
    let response = request_state::request_state_scope_for_test(
        middleware.handle(request(config, cookie).await, handler),
    )
    .await;
    let response = match response {
        Ok(response) | Err(response) => response,
    };
    let status = response.status_code();
    let response = response.into_hyper();
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

/// A handler that signs `email` in through `Auth::attempt`.
fn attempt_route(email: &'static str) -> Next {
    Arc::new(move |_request| {
        Box::pin(async move {
            match Auth::attempt(&Credentials::password(email, PASSWORD), false).await {
                Ok(Some(_)) => Ok(HttpResponse::text("signed in")),
                Ok(None) => Err(HttpResponse::text("wrong password").status(422)),
                Err(error) => Err(HttpResponse::text(error.to_string()).status(500)),
            }
        })
    })
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

/// `Auth::login_id` has no user at hand: the first request that resolves
/// the user records the epoch, and a later password change ends the
/// sign-in.
#[test]
fn hardening_login_id_records_its_epoch_at_the_first_resolution() {
    crate::own_process::run_alone(
        "hardening::hardening_login_id_records_its_epoch_at_the_first_resolution_child",
    );
}

#[test]
fn hardening_login_id_records_its_epoch_at_the_first_resolution_child() {
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
            let login_id: Next = Arc::new(|_request| {
                Box::pin(async {
                    Auth::login_id("7")
                        .map(|()| HttpResponse::text("signed in"))
                        .map_err(|error| HttpResponse::text(error.to_string()).status(500))
                })
            });
            let cookie = serve(&middleware, &config, None, login_id)
                .await
                .session_cookie
                .expect("the sign-in sets a session cookie");

            let first = serve(
                &middleware,
                &config,
                Some(&cookie),
                authenticated_route(None),
            )
            .await;
            assert_eq!(first.status, 200, "the first resolution records the epoch");

            provider.rewrite_password("7").await;
            assert!(suprnova::session::destroy_all_for_user("7").await.is_err());

            let after = serve(
                &middleware,
                &config,
                Some(&cookie),
                authenticated_route(None),
            )
            .await;
            assert_eq!(after.status, 401);
            assert!(!store.holds("web", "7"));
        },
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
