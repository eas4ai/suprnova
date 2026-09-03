//! Request-to-request regressions for the session guard's login commit boundary.

#![cfg(feature = "testing")]

use async_trait::async_trait;
use once_cell::sync::Lazy;
use std::any::Any;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use suprnova::auth::events::{Authenticated, Login};
use suprnova::auth::request_state;
use suprnova::middleware::{Middleware, Next};
use suprnova::session::{SessionConfig, SessionData, SessionMiddleware, SessionStore};
use suprnova::{
    Auth, AuthConfig, AuthManager, Authenticatable, EventFacade, FrameworkError, Guard, Listener,
    SessionGuard, StatefulGuard, UserProvider,
};
use tokio::runtime::Runtime;
use tokio::sync::Mutex;

static RT: Lazy<Runtime> = Lazy::new(|| Runtime::new().expect("tokio runtime"));
static TEST_LOCK: Mutex<()> = Mutex::const_new(());

static SETUP: Lazy<()> = Lazy::new(|| {
    let key = suprnova::EncryptionKey::generate();
    let _ = suprnova::crypto::_test_install_key(key);

    RT.block_on(async {
        // Intentionally do not migrate remember_tokens. The remember=true
        // regression uses the missing table as a real issuance failure after
        // the framework session identity has been committed in memory.
        let config = suprnova::database::DatabaseConfig::builder()
            .url("sqlite::memory:")
            .max_connections(1)
            .min_connections(1)
            .logging(false)
            .build();
        let conn = suprnova::database::DbConnection::connect(&config)
            .await
            .expect("connect in-memory sqlite");
        suprnova::App::singleton(conn);
        suprnova::App::singleton(AuthManager::new(AuthConfig::default()));
        Auth::register_provider("users", Arc::new(FakeProvider)).expect("register users provider");
    });
});

#[derive(Clone)]
struct TestUser;

impl Authenticatable for TestUser {
    fn get_auth_identifier(&self) -> String {
        "7".to_owned()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_arc_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

struct FakeProvider;

#[async_trait]
impl UserProvider for FakeProvider {
    async fn retrieve_by_id(
        &self,
        id: &str,
    ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        Ok((id == "7").then(|| Arc::new(TestUser) as Arc<dyn Authenticatable>))
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

fn user() -> Arc<dyn Authenticatable> {
    Arc::new(TestUser)
}

#[derive(Default)]
struct MemoryStore {
    sessions: StdMutex<HashMap<String, SessionData>>,
}

#[async_trait]
impl SessionStore for MemoryStore {
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

async fn request(cookie: Option<(&str, &str)>) -> suprnova::Request {
    use bytes::Bytes;
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper_util::rt::TokioIo;
    use std::convert::Infallible;
    use tokio::io::AsyncWriteExt;
    use tokio::sync::oneshot;

    let cookie_header = cookie
        .map(|(name, value)| format!("Cookie: {name}={}\r\n", percent_encode_cookie_value(value)))
        .unwrap_or_default();
    let http_bytes = format!(
        "POST /login HTTP/1.1\r\nHost: localhost\r\n{cookie_header}Content-Length: 0\r\n\r\n"
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

fn config() -> SessionConfig {
    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    config
}

fn cookie_value(
    response: &hyper::Response<
        http_body_util::combinators::BoxBody<bytes::Bytes, std::convert::Infallible>,
    >,
    name: &str,
) -> Option<String> {
    let prefix = format!("{name}=");
    response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find_map(|header| {
            header
                .strip_prefix(&prefix)
                .and_then(|rest| rest.split(';').next())
                .map(ToOwned::to_owned)
        })
}

async fn assert_next_request_identity(
    middleware: &SessionMiddleware,
    config: &SessionConfig,
    session_cookie: &str,
) {
    let observed = Arc::new(StdMutex::new(None));
    let observed_for_handler = observed.clone();
    let next: Next = Arc::new(move |_request| {
        let observed = observed_for_handler.clone();
        Box::pin(async move {
            request_state::request_state_scope_for_test(async move {
                let guard = guard();
                let identity = guard.id().await.expect("resolve next-request identity");
                *observed.lock().unwrap() = Some((identity, guard.via_remember()));
                Ok(suprnova::HttpResponse::text("next"))
            })
            .await
        })
    });
    let response = middleware
        .handle(
            request(Some((&config.cookie_name, session_cookie))).await,
            next,
        )
        .await;
    assert!(
        response.is_ok(),
        "next request must load the committed login"
    );
    assert_eq!(
        observed.lock().unwrap().clone(),
        Some((Some("7".to_owned()), false))
    );
}

struct LoginRecorder {
    calls: Arc<AtomicUsize>,
    remembered: Arc<AtomicBool>,
}

#[async_trait]
impl Listener<Login> for LoginRecorder {
    async fn handle(&self, event: &Login) -> Result<(), FrameworkError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.remembered.store(event.remember, Ordering::SeqCst);
        Ok(())
    }
}

struct FailingLifecycleListener {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Listener<Login> for FailingLifecycleListener {
    async fn handle(&self, _event: &Login) -> Result<(), FrameworkError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(FrameworkError::internal("injected login listener failure"))
    }
}

#[async_trait]
impl Listener<Authenticated> for FailingLifecycleListener {
    async fn handle(&self, _event: &Authenticated) -> Result<(), FrameworkError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(FrameworkError::internal(
            "injected authenticated listener failure",
        ))
    }
}

struct AuthenticatedRecorder {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Listener<Authenticated> for AuthenticatedRecorder {
    async fn handle(&self, _event: &Authenticated) -> Result<(), FrameworkError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[test]
fn remember_issue_failure_is_a_successful_non_remembered_login() {
    Lazy::force(&SETUP);
    RT.block_on(async {
        let _serial = TEST_LOCK.lock().await;
        EventFacade::forget::<Login>();
        EventFacade::forget::<Authenticated>();

        let login_calls = Arc::new(AtomicUsize::new(0));
        let remembered = Arc::new(AtomicBool::new(true));
        let authenticated_calls = Arc::new(AtomicUsize::new(0));
        EventFacade::listen::<Login, _>(Arc::new(LoginRecorder {
            calls: login_calls.clone(),
            remembered: remembered.clone(),
        }))
        .await;
        EventFacade::listen::<Authenticated, _>(Arc::new(AuthenticatedRecorder {
            calls: authenticated_calls.clone(),
        }))
        .await;

        let store = Arc::new(MemoryStore::default());
        let config = config();
        let middleware = SessionMiddleware::with_store(config.clone(), store);
        let login_succeeded = Arc::new(AtomicBool::new(false));
        let login_succeeded_for_handler = login_succeeded.clone();
        let next: Next = Arc::new(move |_request| {
            let login_succeeded = login_succeeded_for_handler.clone();
            Box::pin(async move {
                request_state::request_state_scope_for_test(async move {
                    match guard().login(user(), true).await {
                        Ok(()) => {
                            login_succeeded.store(true, Ordering::SeqCst);
                            Ok(suprnova::HttpResponse::text("login-ok"))
                        }
                        Err(_) => Err(suprnova::HttpResponse::text("login-failed").status(500)),
                    }
                })
                .await
            })
        });

        let response = middleware.handle(request(None).await, next).await;
        assert!(login_succeeded.load(Ordering::SeqCst));
        let response = match response {
            Ok(response) => response,
            Err(_) => panic!("post-commit remember failure must not fail the response"),
        };
        assert_eq!(response.status_code(), 200);
        let response = response.into_hyper();
        let session_cookie = cookie_value(&response, &config.cookie_name)
            .expect("committed identity must be carried by a session cookie");
        assert_eq!(cookie_value(&response, "remember_me"), None);
        assert_next_request_identity(&middleware, &config, &session_cookie).await;

        assert_eq!(login_calls.load(Ordering::SeqCst), 1);
        assert!(!remembered.load(Ordering::SeqCst));
        assert_eq!(authenticated_calls.load(Ordering::SeqCst), 1);

        EventFacade::forget::<Login>();
        EventFacade::forget::<Authenticated>();
    });
}

#[test]
fn synchronous_listener_failure_does_not_reverse_committed_login() {
    Lazy::force(&SETUP);
    RT.block_on(async {
        let _serial = TEST_LOCK.lock().await;
        EventFacade::forget::<Login>();
        EventFacade::forget::<Authenticated>();

        let login_calls = Arc::new(AtomicUsize::new(0));
        let authenticated_calls = Arc::new(AtomicUsize::new(0));
        EventFacade::listen::<Login, _>(Arc::new(FailingLifecycleListener {
            calls: login_calls.clone(),
        }))
        .await;
        EventFacade::listen::<Authenticated, _>(Arc::new(FailingLifecycleListener {
            calls: authenticated_calls.clone(),
        }))
        .await;

        let store = Arc::new(MemoryStore::default());
        let config = config();
        let middleware = SessionMiddleware::with_store(config.clone(), store);
        let login_succeeded = Arc::new(AtomicBool::new(false));
        let login_succeeded_for_handler = login_succeeded.clone();
        let next: Next = Arc::new(move |_request| {
            let login_succeeded = login_succeeded_for_handler.clone();
            Box::pin(async move {
                request_state::request_state_scope_for_test(async move {
                    match guard().login(user(), false).await {
                        Ok(()) => {
                            login_succeeded.store(true, Ordering::SeqCst);
                            Ok(suprnova::HttpResponse::text("login-ok"))
                        }
                        Err(_) => Err(suprnova::HttpResponse::text("login-failed").status(500)),
                    }
                })
                .await
            })
        });

        let response = middleware.handle(request(None).await, next).await;
        assert!(login_succeeded.load(Ordering::SeqCst));
        let response = match response {
            Ok(response) => response,
            Err(_) => panic!("post-commit listener failure must not fail the response"),
        };
        assert_eq!(response.status_code(), 200);
        let response = response.into_hyper();
        let session_cookie = cookie_value(&response, &config.cookie_name)
            .expect("committed identity must be carried by a session cookie");
        assert_eq!(cookie_value(&response, "remember_me"), None);
        assert_next_request_identity(&middleware, &config, &session_cookie).await;

        assert_eq!(login_calls.load(Ordering::SeqCst), 1);
        assert_eq!(authenticated_calls.load(Ordering::SeqCst), 1);

        EventFacade::forget::<Login>();
        EventFacade::forget::<Authenticated>();
    });
}
