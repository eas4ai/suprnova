//! The priority list starts as Laravel's four entries that have a Suprnova
//! type, and an application can replace it or insert relative to an entry.
//!
//! Every test that changes the list restores the default when it ends,
//! passed or not, and runs `#[serial]` with the other tests of this binary
//! that change the list.

use std::any::TypeId;
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex, OnceLock};

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use serial_test::serial;
use suprnova::magnetar_integration::middleware::BearerTokenMiddleware;
use suprnova::middleware::{
    add_to_middleware_priority_after, add_to_middleware_priority_before,
    append_middleware_priority, clear_middleware_priority_for_test, default_middleware_priority,
    middleware_priority, prepend_middleware_priority, reset_middleware_priority_for_test,
    set_middleware_priority,
};
use suprnova::session::{SessionConfig, SessionData, SessionMiddleware, SessionStore};
use suprnova::{
    AuthMiddleware, BasicAuthMiddleware, Crypt, EncryptionKey, FrameworkError, Middleware,
    MiddlewareRegistry, Next, Precognitive, Request, Response, Router, ThrottleRequestsMiddleware,
    async_trait,
};

/// A middleware an application adds to the list.
struct Audit;

#[async_trait]
impl Middleware for Audit {
    async fn handle(&self, request: Request, next: Next) -> Response {
        next(request).await
    }
}

/// Another application middleware.
struct Tenant;

#[async_trait]
impl Middleware for Tenant {
    async fn handle(&self, request: Request, next: Next) -> Response {
        next(request).await
    }
}

fn laravel_four() -> Vec<TypeId> {
    vec![
        TypeId::of::<Precognitive>(),
        TypeId::of::<SessionMiddleware>(),
        TypeId::of::<AuthMiddleware>(),
        TypeId::of::<ThrottleRequestsMiddleware>(),
    ]
}

/// Restores the default list when the test ends, passed or not.
struct RestoreDefault;

impl RestoreDefault {
    fn starting_from_default() -> Self {
        reset_middleware_priority_for_test();
        Self
    }
}

impl Drop for RestoreDefault {
    fn drop(&mut self) {
        reset_middleware_priority_for_test();
    }
}

#[test]
fn a_fresh_process_starts_with_laravels_four() {
    crate::own_process::run_alone(
        "laravel_infra_gaps::priority_defaults::a_fresh_process_starts_with_laravels_four_child",
    );
}

/// Runs alone: it reads the list before anything in the process wrote it.
#[test]
fn a_fresh_process_starts_with_laravels_four_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let list = middleware_priority();
    assert_eq!(list, laravel_four());
    assert!(list.contains(&TypeId::of::<SessionMiddleware>()));
}

#[test]
fn default_middleware_priority_answers_laravels_order() {
    assert_eq!(default_middleware_priority(), laravel_four());
}

#[test]
fn the_default_list_leaves_out_middleware_that_check_a_credential_themselves() {
    let list = default_middleware_priority();
    assert!(!list.contains(&TypeId::of::<BasicAuthMiddleware>()));
    assert!(!list.contains(&TypeId::of::<BearerTokenMiddleware>()));
}

#[test]
#[serial]
fn set_middleware_priority_replaces_the_list() {
    let _restore = RestoreDefault::starting_from_default();
    set_middleware_priority([
        TypeId::of::<Tenant>(),
        TypeId::of::<AuthMiddleware>(),
        TypeId::of::<Tenant>(),
    ]);
    assert_eq!(
        middleware_priority(),
        vec![TypeId::of::<Tenant>(), TypeId::of::<AuthMiddleware>()],
        "no earlier entry remains, and a repeated type is listed once"
    );
    set_middleware_priority([]);
    assert!(middleware_priority().is_empty());
}

#[test]
#[serial]
fn add_before_puts_the_middleware_just_before_the_existing_one() {
    let _restore = RestoreDefault::starting_from_default();
    add_to_middleware_priority_before::<AuthMiddleware, Audit>();
    let mut expected = laravel_four();
    expected.insert(2, TypeId::of::<Audit>());
    assert_eq!(middleware_priority(), expected);
}

#[test]
#[serial]
fn add_after_puts_the_middleware_just_after_the_existing_one() {
    let _restore = RestoreDefault::starting_from_default();
    add_to_middleware_priority_after::<AuthMiddleware, Audit>();
    let mut expected = laravel_four();
    expected.insert(3, TypeId::of::<Audit>());
    assert_eq!(middleware_priority(), expected);

    add_to_middleware_priority_after::<ThrottleRequestsMiddleware, Tenant>();
    assert_eq!(middleware_priority().last(), Some(&TypeId::of::<Tenant>()));
}

#[test]
#[serial]
fn a_relative_insert_appends_when_the_existing_one_is_absent() {
    let _restore = RestoreDefault::starting_from_default();
    set_middleware_priority([TypeId::of::<SessionMiddleware>()]);
    add_to_middleware_priority_before::<AuthMiddleware, Audit>();
    add_to_middleware_priority_after::<AuthMiddleware, Tenant>();
    assert_eq!(
        middleware_priority(),
        vec![
            TypeId::of::<SessionMiddleware>(),
            TypeId::of::<Audit>(),
            TypeId::of::<Tenant>(),
        ]
    );
}

#[test]
#[serial]
fn a_relative_insert_leaves_the_list_when_the_middleware_is_listed() {
    let _restore = RestoreDefault::starting_from_default();
    add_to_middleware_priority_before::<Precognitive, ThrottleRequestsMiddleware>();
    add_to_middleware_priority_after::<ThrottleRequestsMiddleware, SessionMiddleware>();
    assert_eq!(middleware_priority(), laravel_four());
}

#[test]
#[serial]
fn prepend_and_append_keep_their_meaning_on_the_default_list() {
    let _restore = RestoreDefault::starting_from_default();
    prepend_middleware_priority::<Tenant>();
    append_middleware_priority::<Audit>();
    let list = middleware_priority();
    assert_eq!(list.first(), Some(&TypeId::of::<Tenant>()));
    assert_eq!(list.last(), Some(&TypeId::of::<Audit>()));
    assert_eq!(&list[1..5], laravel_four().as_slice());
}

#[test]
#[serial]
fn the_reset_helper_brings_the_default_back_after_the_clearing_one() {
    let _restore = RestoreDefault::starting_from_default();
    clear_middleware_priority_for_test();
    assert!(middleware_priority().is_empty());
    reset_middleware_priority_for_test();
    assert_eq!(middleware_priority(), laravel_four());
}

// --- End to end: the session reaches authentication first ---------------

/// An in-memory session store.
#[derive(Default)]
struct Store {
    rows: Mutex<HashMap<String, SessionData>>,
}

#[async_trait]
impl SessionStore for Store {
    async fn read(&self, id: &str) -> Result<Option<SessionData>, FrameworkError> {
        Ok(self.rows.lock().expect("store lock").get(id).cloned())
    }

    async fn write(&self, session: &SessionData) -> Result<(), FrameworkError> {
        let mut session = session.clone();
        session.mark_clean();
        self.rows
            .lock()
            .expect("store lock")
            .insert(session.id.clone(), session);
        Ok(())
    }

    async fn destroy(&self, id: &str) -> Result<(), FrameworkError> {
        self.rows.lock().expect("store lock").remove(id);
        Ok(())
    }

    async fn destroy_for_user(&self, user_id: &str) -> Result<u64, FrameworkError> {
        let mut rows = self.rows.lock().expect("store lock");
        let before = rows.len();
        rows.retain(|_, session| session.user_id.as_deref() != Some(user_id));
        Ok((before - rows.len()) as u64)
    }

    async fn gc(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }
}

const SESSION_ID: &str = "0123456789abcdef0123456789abcdef01234567";

fn config() -> SessionConfig {
    static CRYPT: OnceLock<()> = OnceLock::new();
    CRYPT.get_or_init(|| Crypt::init(EncryptionKey::generate()));
    let mut config = SessionConfig::default();
    config.cookie_secure = false;
    config
}

/// A store holding one session signed in as user 7, and its cookie.
fn signed_in() -> (Arc<Store>, String) {
    let config = config();
    let mut session = SessionData::new(SESSION_ID.to_owned(), "t".repeat(40));
    session.user_id = Some("7".to_owned());
    session.mark_clean();
    let store = Arc::new(Store::default());
    store
        .rows
        .lock()
        .expect("store lock")
        .insert(session.id.clone(), session);
    let cookie = suprnova::http::cookie::Cookie::encrypted(&config.cookie_name, SESSION_ID)
        .expect("encrypted cookie")
        .to_header_value()
        .split(';')
        .next()
        .expect("cookie pair")
        .to_owned();
    (store, cookie)
}

/// Serve one GET of `/account` from a route that lists `AuthMiddleware`
/// before `SessionMiddleware`, and answer the status.
async fn account_status(store: Arc<Store>, cookie: Option<&str>) -> u16 {
    let router: Router = Router::new()
        .get("/account", |_request: Request| async {
            suprnova::text("account")
        })
        .middleware(AuthMiddleware::new())
        .middleware(SessionMiddleware::with_store(config(), store))
        .into();
    let router = Arc::new(router);
    let registry = Arc::new(MiddlewareRegistry::new());
    let (client, server) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        let service = service_fn(move |request| {
            let router = router.clone();
            let registry = registry.clone();
            async move { Ok::<_, Infallible>(suprnova::handle_request(router, registry, request).await) }
        });
        let _ = http1::Builder::new()
            .serve_connection(TokioIo::new(server), service)
            .await;
    });
    let (mut sender, connection) =
        hyper::client::conn::http1::handshake::<_, Full<Bytes>>(TokioIo::new(client))
            .await
            .expect("client handshake");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let mut request = hyper::Request::builder()
        .method("GET")
        .uri("/account")
        .header("host", "localhost");
    if let Some(cookie) = cookie {
        request = request.header("cookie", cookie);
    }
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        sender.send_request(request.body(Full::new(Bytes::new())).expect("request")),
    )
    .await
    .expect("response timeout")
    .expect("response");
    let status = response.status().as_u16();
    let _ = response.into_body().collect().await;
    status
}

#[tokio::test]
#[serial]
async fn a_route_listing_auth_before_the_session_lets_a_signed_in_session_through() {
    let _restore = RestoreDefault::starting_from_default();
    let (store, cookie) = signed_in();
    assert_eq!(account_status(store, Some(&cookie)).await, 200);
}

#[tokio::test]
#[serial]
async fn the_same_route_without_a_session_cookie_is_refused() {
    let _restore = RestoreDefault::starting_from_default();
    let (store, _cookie) = signed_in();
    assert_eq!(account_status(store, None).await, 401);
}

#[tokio::test]
#[serial]
async fn with_an_empty_list_the_route_order_runs_and_refuses_the_session() {
    let _restore = RestoreDefault::starting_from_default();
    clear_middleware_priority_for_test();
    let (store, cookie) = signed_in();
    assert_eq!(
        account_status(store, Some(&cookie)).await,
        401,
        "authentication ran before the session was loaded"
    );
}
