//! Signing a user in for a test (PAR-175): `TestClient::acting_as(user)`
//! makes `user` the authenticated user of every request the client sends,
//! through the default guard, and `acting_as_with_guard(user, guard)`
//! through the named guard, as Laravel's `actingAs` does. No test writes a
//! middleware to sign the user in.
//!
//! The provider of every guard here knows nobody, so a request that
//! answers as the user got that user from `acting_as`, never from a lookup.
//! Requests run inside a `TestContainer` scope, so the scope's
//! `AuthManager` is the one the client's requests see. The tests without
//! an `AuthManager` run in their own process under plain `cargo test`
//! (`own_process_async::delegate`), because other tests of this binary
//! register one in the global container.

use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use suprnova::http::text;
use suprnova::testing::{TestClient, TestContainer};
use suprnova::{
    Auth, AuthConfig, AuthManager, AuthMiddleware, Authenticatable, FrameworkError, GuardConfig,
    MiddlewareRegistry, Request, Response, Router, UserProvider,
};

/// The user the tests act as.
#[derive(Clone)]
struct Member {
    id: String,
}

impl Member {
    fn new(id: &str) -> Self {
        Self { id: id.to_owned() }
    }
}

impl Authenticatable for Member {
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

/// A provider that knows nobody.
struct Nobody;

#[async_trait::async_trait]
impl UserProvider for Nobody {
    async fn retrieve_by_id(
        &self,
        _id: &str,
    ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        Ok(None)
    }
}

/// Declares the default `web` session guard, the `api` token guard and an
/// `admin` session guard, all over the `users` provider, which knows
/// nobody.
fn install_manager() {
    let config = AuthConfig::new("web").guard("admin", GuardConfig::session("users"));
    TestContainer::singleton(AuthManager::new(config));
    Auth::register_provider("users", Arc::new(Nobody)).unwrap();
}

/// Answers the default guard's user, as `Auth::user()` and
/// `Auth::user_as::<Member>()` see it.
async fn me(_request: Request) -> Response {
    let user = Auth::user().await?.map(|user| user.get_auth_identifier());
    let member = Auth::user_as::<Member>().await?.map(|member| member.id);
    text(format!("user={user:?} member={member:?}"))
}

/// Answers the user the guard `X-Guard` names sees.
async fn guard_user(request: Request) -> Response {
    let guard = request.header("x-guard").unwrap_or("web").to_owned();
    let user = Auth::guard(&guard)?.user().await?;
    text(format!("{:?}", user.map(|user| user.get_auth_identifier())))
}

/// A client whose `/me` route sits behind the default guard's auth
/// middleware, `/login-first` behind the one that redirects to the login
/// page, `/admin` and `/api` behind those guards' middleware, and `/guards`
/// behind none. `reached` counts the requests a handler answered.
fn client(reached: &Arc<AtomicUsize>) -> TestClient {
    let router = Router::new()
        .get("/me", counted(reached, me))
        .middleware(AuthMiddleware::new())
        .get("/login-first", counted(reached, me))
        .middleware(AuthMiddleware::redirect_to("/login"))
        .get("/admin", counted(reached, guard_user))
        .middleware(AuthMiddleware::new().for_guard("admin"))
        .get("/api", counted(reached, guard_user))
        .middleware(AuthMiddleware::new().for_guard("api"))
        .get("/guards", counted(reached, guard_user));
    TestClient::new(router, MiddlewareRegistry::new())
}

/// `handler`, counting in `reached` every request it answers.
fn counted<H, Fut>(
    reached: &Arc<AtomicUsize>,
    handler: H,
) -> impl Fn(Request) -> Fut + Send + Sync + 'static
where
    H: Fn(Request) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Response> + Send + 'static,
{
    let reached = reached.clone();
    move |request| {
        reached.fetch_add(1, Ordering::SeqCst);
        handler(request)
    }
}

/// What `/guards` answers for the guard `guard`.
async fn user_of(client: &TestClient, guard: &str) -> String {
    let response = client.get("/guards").header("X-Guard", guard).send().await;
    response.assert_ok();
    response.body_text()
}

#[tokio::test]
async fn acting_as_signs_the_user_into_the_default_guard_for_every_request() {
    TestContainer::scope(async {
        install_manager();
        let reached = Arc::new(AtomicUsize::new(0));
        let guest = client(&reached);

        // Before `acting_as` the route behind the auth middleware refuses.
        guest.get("/me").send().await.assert_status(401);
        assert_eq!(reached.load(Ordering::SeqCst), 0);

        // The builder returns a new client; the clone keeps `guest` a guest.
        let client = guest.clone().acting_as(&Member::new("7"));
        for _ in 0..2 {
            client
                .get("/me")
                .send()
                .await
                .assert_ok()
                .assert_see(r#"user=Some("7") member=Some("7")"#);
        }
        assert_eq!(user_of(&client, "web").await, r#"Some("7")"#);

        // Another `acting_as` replaces the user.
        let client = client.acting_as(&Member::new("8"));
        client
            .get("/me")
            .send()
            .await
            .assert_see(r#"user=Some("8")"#);

        // The client it was built from stays a guest.
        guest.get("/me").send().await.assert_status(401);
    })
    .await;
}

#[tokio::test]
async fn a_web_route_redirects_a_guest_to_the_login_page_and_answers_the_user() {
    TestContainer::scope(async {
        install_manager();
        let reached = Arc::new(AtomicUsize::new(0));
        let guest = client(&reached);

        guest
            .get("/login-first")
            .send()
            .await
            .assert_redirect(Some("/login"));

        guest
            .acting_as(&Member::new("7"))
            .get("/login-first")
            .send()
            .await
            .assert_ok()
            .assert_see(r#"user=Some("7")"#);
    })
    .await;
}

#[tokio::test]
async fn acting_as_with_guard_signs_the_user_into_the_named_token_guard_only() {
    TestContainer::scope(async {
        install_manager();
        let reached = Arc::new(AtomicUsize::new(0));
        let guest = client(&reached);
        guest.get("/api").send().await.assert_status(401);

        // A user the test holds as `Arc<dyn Authenticatable>` works too.
        let user: Arc<dyn Authenticatable> = Arc::new(Member::new("7"));
        let client = guest.acting_as_with_guard(&user, "api");

        client
            .get("/api")
            .header("X-Guard", "api")
            .send()
            .await
            .assert_ok()
            .assert_see(r#"Some("7")"#);
        assert_eq!(user_of(&client, "api").await, r#"Some("7")"#);

        // The default guard has no user: its route refuses, and it names
        // nobody.
        client.get("/me").send().await.assert_status(401);
        assert_eq!(user_of(&client, "web").await, "None");
        assert_eq!(user_of(&client, "admin").await, "None");
    })
    .await;
}

#[tokio::test]
async fn acting_as_with_guard_signs_the_user_into_the_named_session_guard_only() {
    TestContainer::scope(async {
        install_manager();
        let reached = Arc::new(AtomicUsize::new(0));
        let client = client(&reached).acting_as_with_guard(&Member::new("9"), "admin");

        client
            .get("/admin")
            .header("X-Guard", "admin")
            .send()
            .await
            .assert_ok()
            .assert_see(r#"Some("9")"#);

        client.get("/me").send().await.assert_status(401);
        client.get("/api").send().await.assert_status(401);
        assert_eq!(user_of(&client, "web").await, "None");
        assert_eq!(user_of(&client, "api").await, "None");
    })
    .await;
}

#[tokio::test]
async fn acting_as_with_the_default_guard_name_is_acting_as() {
    TestContainer::scope(async {
        install_manager();
        let reached = Arc::new(AtomicUsize::new(0));
        let client = client(&reached).acting_as_with_guard(&Member::new("7"), "web");

        client
            .get("/me")
            .send()
            .await
            .assert_ok()
            .assert_see(r#"user=Some("7")"#);
    })
    .await;
}

/// Asserts `response` failed with a report naming the guard `name`.
fn assert_fails_naming(response: &suprnova::testing::TestResponse, name: &str) {
    response.assert_status(500);
    let report = response
        .error_report()
        .expect("the failed request carries an error report")
        .to_string();
    assert!(
        report.contains(&format!("'{name}'")),
        "the report must name the guard {name:?}: {report}"
    );
}

#[tokio::test]
async fn an_unregistered_guard_fails_the_first_request_naming_it() {
    TestContainer::scope(async {
        install_manager();
        let reached = Arc::new(AtomicUsize::new(0));
        let client = client(&reached).acting_as_with_guard(&Member::new("7"), "nope");

        // The first request fails before any handler runs, so the user is
        // signed in nowhere.
        assert_fails_naming(&client.get("/guards").send().await, "nope");
        assert_fails_naming(&client.get("/me").send().await, "nope");
        assert_eq!(reached.load(Ordering::SeqCst), 0);
    })
    .await;
}

#[tokio::test]
async fn a_guard_whose_provider_is_missing_fails_naming_the_guard() {
    TestContainer::scope(async {
        // `admin` is declared, but nothing registered its `users` provider.
        let config = AuthConfig::new("web").guard("admin", GuardConfig::session("users"));
        TestContainer::singleton(AuthManager::new(config));
        let reached = Arc::new(AtomicUsize::new(0));
        let client = client(&reached).acting_as_with_guard(&Member::new("7"), "admin");

        assert_fails_naming(&client.get("/guards").send().await, "admin");
        assert_eq!(reached.load(Ordering::SeqCst), 0);
    })
    .await;
}

#[tokio::test]
async fn without_an_auth_manager_acting_as_signs_the_user_into_the_default_guard() {
    if crate::own_process_async::delegate(
        module_path!(),
        "without_an_auth_manager_acting_as_signs_the_user_into_the_default_guard",
    )
    .await
    {
        return;
    }
    TestContainer::scope(async {
        let reached = Arc::new(AtomicUsize::new(0));
        let guest = client(&reached);
        guest.get("/me").send().await.assert_status(401);

        guest
            .clone()
            .acting_as(&Member::new("7"))
            .get("/me")
            .send()
            .await
            .assert_ok()
            .assert_see(r#"user=Some("7") member=Some("7")"#);
        guest
            .acting_as_with_guard(&Member::new("7"), "web")
            .get("/me")
            .send()
            .await
            .assert_ok()
            .assert_see(r#"user=Some("7")"#);
    })
    .await;
}

#[tokio::test]
async fn without_an_auth_manager_a_named_guard_fails_naming_it() {
    if crate::own_process_async::delegate(
        module_path!(),
        "without_an_auth_manager_a_named_guard_fails_naming_it",
    )
    .await
    {
        return;
    }
    TestContainer::scope(async {
        let reached = Arc::new(AtomicUsize::new(0));
        let client = client(&reached).acting_as_with_guard(&Member::new("7"), "api");

        assert_fails_naming(&client.get("/me").send().await, "api");
        assert_eq!(reached.load(Ordering::SeqCst), 0);
    })
    .await;
}
