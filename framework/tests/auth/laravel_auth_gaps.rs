//! The authentication members of Laravel's auth surface that Suprnova lacked
//! (PAR-130): `Auth::via_request_with_provider` hands its resolver the
//! guard's user provider, as Laravel's `viaRequest` callback receives
//! `$request` and `$provider`, while `Auth::via_request` keeps taking a
//! resolver of the request alone.
//!
//! Requests run through `TestClient` inside a `TestContainer` scope, so the
//! scope's `AuthManager` is the one the middleware and the handler see.

use std::any::Any;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use suprnova::http::text;
use suprnova::testing::{TestClient, TestContainer};
use suprnova::{
    Auth, AuthConfig, AuthManager, AuthMiddleware, Authenticatable, FrameworkError, GuardConfig,
    MiddlewareRegistry, Request, Response, Router, UserProvider,
};

/// The key the partner `api-7` sends.
const PARTNER_KEY: &str = "partner-key-7f3a";

/// A user identified by a fixed string.
struct Named(String);

impl Authenticatable for Named {
    fn get_auth_identifier(&self) -> String {
        self.0.clone()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn into_arc_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

/// A provider that knows the users it lists, by id.
struct Directory(&'static [&'static str]);

#[async_trait::async_trait]
impl UserProvider for Directory {
    async fn retrieve_by_id(
        &self,
        id: &str,
    ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        Ok(self
            .0
            .contains(&id)
            .then(|| Arc::new(Named(id.to_owned())) as Arc<dyn Authenticatable>))
    }
}

/// The id the partner key stands for, `None` for any other key.
fn partner_id(request: &Request) -> Option<&'static str> {
    (request.header("x-api-key") == Some(PARTNER_KEY)).then_some("api-7")
}

/// Declares `partner` under the `partners` provider and `keyed` under the
/// `users` provider, both request guards.
fn install_manager() {
    let partner = GuardConfig::custom(AuthManager::via_request_driver("partner"), "partners");
    let keyed = GuardConfig::custom(AuthManager::via_request_driver("keyed"), "users");
    let config = AuthConfig::new("web")
        .guard("partner", partner)
        .guard("keyed", keyed);
    TestContainer::singleton(AuthManager::new(config));
}

/// Reports the user the guard `X-Guard` names sees.
async fn whoami(request: Request) -> Response {
    let guard = request.header("x-guard").unwrap_or("partner").to_owned();
    let user = Auth::guard(&guard)?.user().await?;
    text(format!("{:?}", user.map(|user| user.get_auth_identifier())))
}

/// A client whose routes require the request guard `guard`.
fn client(guard: &str) -> TestClient {
    let router = Router::new().get("/whoami", whoami);
    let registry = MiddlewareRegistry::new().append(AuthMiddleware::new().for_guard(guard));
    TestClient::new(router, registry)
}

/// The providers a resolver received, in order.
type Seen = Arc<Mutex<Vec<Arc<dyn UserProvider>>>>;

/// Registers a `partner` resolver that records the provider it receives
/// and looks the partner up through it.
fn register_recording_resolver(seen: &Seen) {
    let seen = seen.clone();
    Auth::via_request_with_provider("partner", move |request, provider| {
        seen.lock().unwrap().push(provider.clone());
        let id = partner_id(request);
        Box::pin(async move {
            match id {
                Some(id) => provider.retrieve_by_id(id).await,
                None => Ok(None),
            }
        })
    })
    .unwrap();
}

#[tokio::test]
async fn the_resolver_receives_the_provider_its_guard_configuration_names() {
    TestContainer::scope(async {
        install_manager();
        let users: Arc<dyn UserProvider> = Arc::new(Directory(&[]));
        let partners: Arc<dyn UserProvider> = Arc::new(Directory(&["api-7"]));
        Auth::register_provider("users", users.clone()).unwrap();
        Auth::register_provider("partners", partners.clone()).unwrap();
        let seen = Seen::default();
        register_recording_resolver(&seen);

        // The partner is found through the provider the resolver received:
        // only `partners` knows `api-7`.
        let response = client("partner")
            .get("/whoami")
            .header("X-Api-Key", PARTNER_KEY)
            .send()
            .await;
        response.assert_ok().assert_see(r#"Some("api-7")"#);

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert!(Arc::ptr_eq(&seen[0], &partners));
        assert!(!Arc::ptr_eq(&seen[0], &users));
    })
    .await;
}

#[tokio::test]
async fn the_provider_is_the_one_registered_when_the_resolver_runs() {
    TestContainer::scope(async {
        install_manager();
        let seen = Seen::default();
        // The resolver is registered before any provider, and the provider
        // replaced after the first request.
        register_recording_resolver(&seen);
        let first: Arc<dyn UserProvider> = Arc::new(Directory(&["api-7"]));
        Auth::register_provider("partners", first.clone()).unwrap();
        let client = client("partner");
        client
            .get("/whoami")
            .header("X-Api-Key", PARTNER_KEY)
            .send()
            .await
            .assert_ok();

        let second: Arc<dyn UserProvider> = Arc::new(Directory(&[]));
        Auth::register_provider("partners", second.clone()).unwrap();
        // `second` does not know the partner, so the guard refuses.
        client
            .get("/whoami")
            .header("X-Api-Key", PARTNER_KEY)
            .send()
            .await
            .assert_status(401);

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert!(Arc::ptr_eq(&seen[0], &first));
        assert!(Arc::ptr_eq(&seen[1], &second));
    })
    .await;
}

#[tokio::test]
async fn a_provider_nobody_registered_fails_the_request_before_the_resolver_runs() {
    TestContainer::scope(async {
        install_manager();
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        Auth::via_request_with_provider("partner", move |_request, _provider| {
            counted.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(None) })
        })
        .unwrap();

        let response = client("partner")
            .get("/whoami")
            .header("X-Api-Key", PARTNER_KEY)
            .send()
            .await;
        response.assert_status(500);
        let report = response.error_report().expect("an error response");
        assert!(
            report
                .chain()
                .iter()
                .any(|link| link.contains("No UserProvider registered under 'partners'")),
            "{:?}",
            report.chain()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    })
    .await;
}

#[tokio::test]
async fn a_resolver_error_fails_the_request() {
    TestContainer::scope(async {
        install_manager();
        Auth::register_provider("partners", Arc::new(Directory(&["api-7"]))).unwrap();
        Auth::via_request_with_provider("partner", |_request, _provider| {
            Box::pin(async { Err(FrameworkError::internal("partner key store unavailable")) })
        })
        .unwrap();

        client("partner")
            .get("/whoami")
            .header("X-Api-Key", PARTNER_KEY)
            .send()
            .await
            .assert_status(500);
    })
    .await;
}

#[tokio::test]
async fn via_request_still_takes_a_resolver_of_the_request_alone() {
    TestContainer::scope(async {
        install_manager();
        Auth::register_provider("users", Arc::new(Directory(&[]))).unwrap();
        // The resolver names no provider and finds the user itself.
        Auth::via_request("keyed", |request| {
            let user = partner_id(request).map(|id| Arc::new(Named(id.to_owned())));
            Box::pin(async move { Ok(user.map(|user| user as Arc<dyn Authenticatable>)) })
        })
        .unwrap();

        client("keyed")
            .get("/whoami")
            .header("X-Guard", "keyed")
            .header("X-Api-Key", PARTNER_KEY)
            .send()
            .await
            .assert_ok()
            .assert_see(r#"Some("api-7")"#);
        client("keyed")
            .get("/whoami")
            .header("X-Guard", "keyed")
            .send()
            .await
            .assert_status(401);
    })
    .await;
}

#[tokio::test]
async fn a_guard_name_that_cannot_qualify_a_principal_is_refused() {
    TestContainer::scope(async {
        install_manager();
        for refused in ["", "a:b"] {
            let error = Auth::via_request_with_provider(refused, |_request, _provider| {
                Box::pin(async { Ok(None) })
            })
            .expect_err("the registration is refused");
            let message = error.to_string();
            assert!(
                message.contains("cannot be empty") || message.contains("cannot contain ':'"),
                "{message}"
            );
        }
    })
    .await;
}
