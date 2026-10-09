//! The authorization members of Laravel's auth surface that Suprnova lacked
//! (PAR-123): `Gate::inspect_current` and `Gate::none_current` answer for the
//! user of the route's guard, `Gate::after_with_arguments` hands the resource
//! to an after-hook, every check dispatches `GateEvaluated`, and
//! `GateResponse::authorize` keeps a denial's code.
//!
//! The checks that resolve the user run inside a request served by
//! `TestClient`, where `LoginAs` signs the default guard in. Every test is
//! `#[serial]`: the gate's hooks and default denial are process-global, as in
//! the rest of this suite.

use std::any::Any;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Once};

use serial_test::serial;
use suprnova::events::testing::dispatched;
use suprnova::http::text;
use suprnova::testing::{TestClient, TestContainer};
use suprnova::{
    Auth, AuthConfig, AuthManager, AuthMiddleware, Authenticatable, EventFacade, FrameworkError,
    Gate, GateEvaluated, GateResponse, GuardConfig, Listener, Middleware, MiddlewareRegistry, Next,
    Request, Response, Router, UserProvider, handler,
};

// ── Users and resources ──────────────────────────────────────────────────────

#[derive(Clone, Debug)]
struct GapUser {
    id: i64,
}

impl Authenticatable for GapUser {
    fn get_auth_identifier(&self) -> String {
        self.id.to_string()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn into_arc_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

#[derive(Debug, Default)]
pub struct GapPost {
    author_id: i64,
}

/// What the `gaps-preview` gate received, in order: the user's id, or
/// `None` for a guest.
static PREVIEW_SEEN: Mutex<Vec<Option<i64>>> = Mutex::new(Vec::new());

fn register_gates() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // A rich gate whose denial carries a message and a code.
        Gate::define_with::<GapUser, GapPost>("gaps-edit", |user, post| {
            if post.author_id == user.id {
                GateResponse::allow()
            } else {
                GateResponse::deny_with("Not your post.").with_code("not-owner")
            }
        });
        // A bool gate that allows every signed-in user; a guest never
        // reaches it.
        Gate::define::<GapUser, GapPost>("gaps-open", |_user, _post| true);
        // A nullable gate that records what it was handed.
        Gate::define_optional::<GapUser, GapPost>("gaps-preview", |user, _post| {
            PREVIEW_SEEN.lock().unwrap().push(user.map(|user| user.id));
            true
        });
        Gate::define::<GapUser, GapPost>("gaps-update", |user, post| post.author_id == user.id);
        Gate::define::<GapUser, GapPost>("gaps-create", |user, _post| user.id == 7);
    });
}

// ── Request plumbing ─────────────────────────────────────────────────────────

/// Signs the default guard in as the user whose id `X-Test-User` names.
struct LoginAs;

#[async_trait::async_trait]
impl Middleware for LoginAs {
    async fn handle(&self, request: Request, next: Next) -> Response {
        if let Some(id) = header_id(&request, "x-test-user") {
            Auth::set_user(Arc::new(GapUser { id }));
        }
        next(request).await
    }
}

fn header_id(request: &Request, name: &str) -> Option<i64> {
    request.header(name).and_then(|value| value.parse().ok())
}

fn post_of(request: &Request) -> GapPost {
    GapPost {
        author_id: header_id(request, "x-author").unwrap_or(0),
    }
}

/// Answers `inspect_current` for `X-Action` on a post by `X-Author`, then
/// `inspect_async` for the user `X-Explicit` names, `-` when it names none.
async fn compare(request: Request) -> Response {
    let action = request.header("x-action").unwrap_or_default().to_owned();
    let post = post_of(&request);
    let current = Gate::inspect_current(&action, &post).await?;
    let explicit = match header_id(&request, "x-explicit") {
        Some(id) => format!(
            "{:?}",
            Gate::inspect_async(&action, &GapUser { id }, &post).await
        ),
        None => "-".to_owned(),
    };
    text(format!("{current:?}|{explicit}"))
}

/// Answers `none_current` for the comma list `X-Actions` on a post by
/// `X-Author`.
async fn none(request: Request) -> Response {
    let actions: Vec<&str> = request
        .header("x-actions")
        .unwrap_or_default()
        .split(',')
        .filter(|action| !action.is_empty())
        .collect();
    let post = post_of(&request);
    let none = Gate::none_current(&actions, &post).await?;
    text(none.to_string())
}

#[handler]
#[authorize("gaps-create", GapPost)]
pub async fn gated_create() -> Response {
    text("created")
}

/// A denial with a code, propagated through `?`.
async fn coded(_request: Request) -> Response {
    GateResponse::deny_with("quota")
        .with_code("over-limit")
        .authorize()?;
    text("unreachable")
}

fn client(registry: MiddlewareRegistry) -> TestClient {
    register_gates();
    let router = Router::new()
        .get("/compare", compare)
        .get("/none", none)
        .get("/authorized", gated_create)
        .get("/coded", coded);
    TestClient::new(router, registry)
}

fn signed_in_client() -> TestClient {
    client(MiddlewareRegistry::new().append(LoginAs))
}

/// The two answers `/compare` gave: `inspect_current`'s and `inspect_async`'s.
async fn compare_as(client: &TestClient, headers: &[(&str, &str)]) -> (String, String) {
    let mut request = client.get("/compare");
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = request.send().await;
    response.assert_ok();
    let body = response.body_text();
    let (current, explicit) = body.split_once('|').expect("two answers");
    (current.to_owned(), explicit.to_owned())
}

async fn none_as(client: &TestClient, headers: &[(&str, &str)]) -> String {
    let mut request = client.get("/none");
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = request.send().await;
    response.assert_ok();
    response.body_text()
}

/// Clears the process-global default denial when dropped, so a test that
/// set one cannot leak it into the next.
struct DefaultDenial;

impl DefaultDenial {
    fn set(response: GateResponse) -> Self {
        Gate::clear_default_denial_response();
        Gate::default_denial_response(response);
        Self
    }
}

impl Drop for DefaultDenial {
    fn drop(&mut self) {
        Gate::clear_default_denial_response();
    }
}

// ── inspect_current and none_current ─────────────────────────────────────────

#[tokio::test]
#[serial]
async fn inspect_current_answers_as_inspect_async_for_the_signed_in_user() {
    let client = signed_in_client();
    let user7 = [("X-Test-User", "7"), ("X-Explicit", "7")];

    for (action, author) in [
        ("gaps-edit", "7"),
        ("gaps-edit", "8"),
        ("gaps-update", "7"),
        ("gaps-update", "8"),
        ("gaps-undefined", "7"),
    ] {
        let mut headers = user7.to_vec();
        headers.extend([("X-Action", action), ("X-Author", author)]);
        let (current, explicit) = compare_as(&client, &headers).await;
        assert_eq!(current, explicit, "{action} on a post by {author}");
    }

    // The owner is allowed; anyone else gets the rich denial, code included.
    let headers = [
        ("X-Test-User", "7"),
        ("X-Action", "gaps-edit"),
        ("X-Author", "7"),
    ];
    let (current, _) = compare_as(&client, &headers).await;
    assert!(current.contains("allowed: true"), "{current}");
    let headers = [
        ("X-Test-User", "7"),
        ("X-Action", "gaps-edit"),
        ("X-Author", "8"),
    ];
    let (current, _) = compare_as(&client, &headers).await;
    assert!(current.contains("allowed: false"), "{current}");
    assert!(current.contains("Not your post."), "{current}");
    assert!(current.contains("not-owner"), "{current}");
}

/// The provider the request guard is declared with. The checks under test
/// never ask it for a user.
struct NoUsers;

#[async_trait::async_trait]
impl UserProvider for NoUsers {
    async fn retrieve_by_id(
        &self,
        _id: &str,
    ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        Ok(None)
    }
}

/// Installs a container-scoped manager with the default `web` guard, which
/// `LoginAs` signs in, and the `partner` request guard, whose user is the
/// one `X-Partner` names.
fn install_partner_guard() {
    let driver = AuthManager::via_request_driver("partner");
    let config = AuthConfig::new("web").guard("partner", GuardConfig::custom(driver, "users"));
    TestContainer::singleton(AuthManager::new(config));
    Auth::register_provider("users", Arc::new(NoUsers)).unwrap();
    Auth::via_request("partner", |request| {
        let user = header_id(request, "x-partner")
            .map(|id| Arc::new(GapUser { id }) as Arc<dyn Authenticatable>);
        Box::pin(async move { Ok(user) })
    })
    .unwrap();
}

#[tokio::test]
#[serial]
async fn inspect_current_answers_for_the_user_of_the_route_guard() {
    TestContainer::scope(async {
        install_partner_guard();
        let client = client(
            MiddlewareRegistry::new()
                .append(LoginAs)
                .append(AuthMiddleware::new().for_guard("partner")),
        );

        // The default guard holds user 8 and the route's guard user 7: the
        // check answers for 7, as `#[authorize]` would.
        let headers = [
            ("X-Test-User", "8"),
            ("X-Partner", "7"),
            ("X-Explicit", "7"),
            ("X-Action", "gaps-edit"),
            ("X-Author", "7"),
        ];
        let (current, explicit) = compare_as(&client, &headers).await;
        assert!(current.contains("allowed: true"), "{current}");
        assert_eq!(current, explicit);

        let headers = [
            ("X-Test-User", "8"),
            ("X-Partner", "7"),
            ("X-Explicit", "7"),
            ("X-Action", "gaps-edit"),
            ("X-Author", "8"),
        ];
        let (current, explicit) = compare_as(&client, &headers).await;
        assert!(current.contains("allowed: false"), "{current}");
        assert_eq!(current, explicit);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn a_guest_is_denied_a_gate_that_is_not_optional_with_the_default_denial() {
    let client = signed_in_client();
    let guest = [("X-Action", "gaps-open"), ("X-Author", "7")];

    // A signed-in user passes the gate; a guest never reaches it, and the
    // answer is a response, not a 401.
    let mut signed_in = guest.to_vec();
    signed_in.push(("X-Test-User", "7"));
    let (current, _) = compare_as(&client, &signed_in).await;
    assert!(current.contains("allowed: true"), "{current}");

    let (current, _) = compare_as(&client, &guest).await;
    assert_eq!(current, format!("{:?}", GateResponse::deny()));

    // The default denial is the answer, whatever it is set to.
    let _default = DefaultDenial::set(GateResponse::deny_as_not_found());
    let (current, _) = compare_as(&client, &guest).await;
    assert_eq!(current, format!("{:?}", GateResponse::deny_as_not_found()));

    // An undefined ability is denied the same way.
    let undefined = [("X-Action", "gaps-undefined"), ("X-Author", "7")];
    let (current, _) = compare_as(&client, &undefined).await;
    assert_eq!(current, format!("{:?}", GateResponse::deny_as_not_found()));
}

#[tokio::test]
#[serial]
async fn an_optional_gate_receives_none_for_a_guest_and_the_user_otherwise() {
    let client = signed_in_client();
    PREVIEW_SEEN.lock().unwrap().clear();

    let guest = [("X-Action", "gaps-preview"), ("X-Author", "7")];
    let (current, _) = compare_as(&client, &guest).await;
    assert!(current.contains("allowed: true"), "{current}");
    assert_eq!(*PREVIEW_SEEN.lock().unwrap(), vec![None]);

    let signed_in = [
        ("X-Action", "gaps-preview"),
        ("X-Author", "7"),
        ("X-Test-User", "7"),
    ];
    compare_as(&client, &signed_in).await;
    assert_eq!(*PREVIEW_SEEN.lock().unwrap(), vec![None, Some(7)]);
}

#[tokio::test]
#[serial]
async fn none_current_answers_for_the_route_user_and_a_guest() {
    let client = signed_in_client();
    let user7 = ("X-Test-User", "7");

    // Denied on someone else's post: none of the actions allow.
    let headers = [
        user7,
        ("X-Actions", "gaps-edit,gaps-update"),
        ("X-Author", "8"),
    ];
    assert_eq!(none_as(&client, &headers).await, "true");
    // One action allows on their own post.
    let headers = [
        user7,
        ("X-Actions", "gaps-undefined,gaps-edit"),
        ("X-Author", "7"),
    ];
    assert_eq!(none_as(&client, &headers).await, "false");
    // No actions: vacuously none.
    let headers = [user7, ("X-Actions", ""), ("X-Author", "7")];
    assert_eq!(none_as(&client, &headers).await, "true");

    // A guest passes only an optional gate.
    let headers = [("X-Actions", "gaps-open,gaps-update"), ("X-Author", "7")];
    assert_eq!(none_as(&client, &headers).await, "true");
    let headers = [("X-Actions", "gaps-open,gaps-preview"), ("X-Author", "7")];
    assert_eq!(none_as(&client, &headers).await, "false");
}

#[tokio::test]
#[serial]
async fn a_guard_that_cannot_resolve_the_user_fails_the_check() {
    TestContainer::scope(async {
        // The default guard names the `users` provider, and nobody registered
        // one: the check cannot resolve the user and does not guess.
        TestContainer::singleton(AuthManager::new(AuthConfig::new("web")));
        let response = client(MiddlewareRegistry::new())
            .get("/compare")
            .header("X-Action", "gaps-open")
            .header("X-Author", "7")
            .send()
            .await;
        assert_eq!(response.status(), 500, "{}", response.body_text());
        let report = response.error_report().expect("an error response");
        assert!(
            report
                .chain()
                .iter()
                .any(|link| link.contains("No UserProvider registered under 'users'")),
            "{:?}",
            report.chain()
        );
    })
    .await;
}

// ── after_with_arguments ─────────────────────────────────────────────────────

/// A user type of its own, so the after-hooks registered for it reach no
/// other test.
struct AfterUser;

struct AfterPost {
    id: i64,
}

/// A resource of another type, which the hook is not registered for.
struct AfterOther;

/// What the hook saw: the action, the running decision, the post's id.
static AFTER_SEEN: Mutex<Vec<(String, Option<bool>, i64)>> = Mutex::new(Vec::new());

fn register_after_hooks() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        Gate::define::<AfterUser, AfterPost>("after-denied", |_user, _post| false);
        Gate::define::<AfterUser, AfterPost>("after-allowed", |_user, _post| true);
        Gate::define::<AfterUser, AfterOther>("after-other-denied", |_user, _other| false);
        // Answers `true` every time; only an undecided result takes it.
        Gate::after_with_arguments::<AfterUser, AfterPost>(|_user, action, decided, post| {
            AFTER_SEEN
                .lock()
                .unwrap()
                .push((action.to_owned(), decided, post.id));
            Some(true)
        });
    });
}

fn after_seen() -> Vec<(String, Option<bool>, i64)> {
    std::mem::take(&mut *AFTER_SEEN.lock().unwrap())
}

#[tokio::test]
#[serial]
async fn an_after_hook_with_arguments_receives_the_resource_and_fills_only_undecided() {
    register_after_hooks();
    after_seen();

    // Undecided: the hook decides, and it saw the post.
    assert!(Gate::inspect("after-undefined", &AfterUser, &AfterPost { id: 41 }).allowed());
    assert_eq!(after_seen(), vec![("after-undefined".to_owned(), None, 41)]);

    // Decided: the hook runs and sees the decision, but cannot override it.
    assert!(Gate::inspect("after-denied", &AfterUser, &AfterPost { id: 42 }).denied());
    assert_eq!(
        after_seen(),
        vec![("after-denied".to_owned(), Some(false), 42)]
    );
    assert!(Gate::inspect("after-allowed", &AfterUser, &AfterPost { id: 43 }).allowed());
    assert_eq!(
        after_seen(),
        vec![("after-allowed".to_owned(), Some(true), 43)]
    );

    // The async path hands it the resource too.
    assert!(
        Gate::inspect_async("after-undefined", &AfterUser, &AfterPost { id: 44 })
            .await
            .allowed()
    );
    assert!(
        Gate::inspect_async("after-denied", &AfterUser, &AfterPost { id: 45 })
            .await
            .denied()
    );
    assert_eq!(
        after_seen(),
        vec![
            ("after-undefined".to_owned(), None, 44),
            ("after-denied".to_owned(), Some(false), 45),
        ]
    );
}

#[tokio::test]
#[serial]
async fn an_after_hook_with_arguments_skips_a_resource_of_another_type() {
    register_after_hooks();
    after_seen();

    assert!(Gate::raw("after-undefined", &AfterUser, &AfterOther).is_none());
    assert!(Gate::inspect("after-other-denied", &AfterUser, &AfterOther).denied());
    assert!(after_seen().is_empty());
}

// ── GateEvaluated ────────────────────────────────────────────────────────────

fn evaluated(action: &str) -> Vec<GateEvaluated> {
    dispatched::<GateEvaluated>(|event| event.action == action)
}

#[tokio::test]
#[serial]
async fn every_sync_check_dispatches_one_event_carrying_its_decision() {
    register_gates();
    let _events = EventFacade::fake();
    let user7 = GapUser { id: 7 };
    let own = GapPost { author_id: 7 };

    let answer = Gate::inspect("gaps-update", &user7, &own);
    assert!(answer.allowed());
    assert_eq!(
        evaluated("gaps-update"),
        vec![GateEvaluated {
            user_type: Some(std::any::type_name::<GapUser>()),
            user_id: None,
            action: "gaps-update".to_owned(),
            resource_type: std::any::type_name::<GapPost>(),
            decision: Some(true),
        }]
    );

    // A denial, through `allows`.
    assert!(!Gate::allows(
        "gaps-edit",
        &user7,
        &GapPost { author_id: 8 }
    ));
    let events = evaluated("gaps-edit");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].decision, Some(false));

    // Nothing decided: `raw` and `inspect` each report `None`.
    assert!(Gate::raw("gaps-nothing", &user7, &own).is_none());
    assert!(Gate::inspect("gaps-nothing", &user7, &own).denied());
    let events = evaluated("gaps-nothing");
    assert_eq!(events.len(), 2);
    assert!(events.iter().all(|event| event.decision.is_none()));

    // A multi-action check reports each action it evaluated.
    assert!(Gate::any(&["gaps-open", "gaps-create"], &user7, &own));
    assert_eq!(evaluated("gaps-open").len(), 1);
    assert!(
        evaluated("gaps-create").is_empty(),
        "`any` stops at the first allow"
    );
}

#[tokio::test]
#[serial]
async fn every_async_check_dispatches_one_event_carrying_its_decision() {
    register_gates();
    let _events = EventFacade::fake();
    let user7 = GapUser { id: 7 };

    assert!(Gate::allows_async("gaps-update", &user7, &GapPost { author_id: 7 }).await);
    assert!(!Gate::allows_async("gaps-update", &user7, &GapPost { author_id: 8 }).await);
    let decisions: Vec<_> = evaluated("gaps-update")
        .into_iter()
        .map(|event| (event.decision, event.user_id))
        .collect();
    assert_eq!(decisions, vec![(Some(true), None), (Some(false), None)]);

    assert!(
        Gate::raw_async("gaps-nothing", &user7, &GapPost::default())
            .await
            .is_none()
    );
    assert_eq!(evaluated("gaps-nothing")[0].decision, None);
}

#[tokio::test]
#[serial]
async fn a_check_that_resolves_the_user_reports_the_users_identifier() {
    let client = signed_in_client();
    let _events = EventFacade::fake();

    // `inspect_current` for user 7.
    let headers = [
        ("X-Test-User", "7"),
        ("X-Action", "gaps-edit"),
        ("X-Author", "8"),
    ];
    compare_as(&client, &headers).await;
    assert_eq!(
        evaluated("gaps-edit"),
        vec![GateEvaluated {
            user_type: Some(std::any::type_name::<GapUser>()),
            user_id: Some("7".to_owned()),
            action: "gaps-edit".to_owned(),
            resource_type: std::any::type_name::<GapPost>(),
            decision: Some(false),
        }]
    );

    // `#[authorize]` for user 7.
    client
        .get("/authorized")
        .header("X-Test-User", "7")
        .send()
        .await
        .assert_ok();
    let events = evaluated("gaps-create");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].user_id.as_deref(), Some("7"));
    assert_eq!(events[0].user_type, Some(std::any::type_name::<GapUser>()));
    assert_eq!(events[0].decision, Some(true));

    // A guest has no user to name.
    let guest = [("X-Action", "gaps-open"), ("X-Author", "7")];
    compare_as(&client, &guest).await;
    let events = evaluated("gaps-open");
    assert_eq!(events.len(), 1);
    assert_eq!(
        (events[0].user_type, events[0].user_id.as_deref()),
        (None, None)
    );
    assert_eq!(events[0].decision, None);
}

/// Counts the events it receives and fails every one.
struct FailingAudit(Arc<AtomicUsize>);

#[async_trait::async_trait]
impl Listener<GateEvaluated> for FailingAudit {
    async fn handle(&self, _event: &GateEvaluated) -> Result<(), FrameworkError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(FrameworkError::internal("audit log unavailable"))
    }
}

/// Forgets every `GateEvaluated` listener when dropped.
struct ForgetListeners;

impl Drop for ForgetListeners {
    fn drop(&mut self) {
        EventFacade::forget::<GateEvaluated>();
    }
}

#[tokio::test]
#[serial]
async fn a_failing_listener_never_changes_the_decision() {
    register_gates();
    let calls = Arc::new(AtomicUsize::new(0));
    let _forget = ForgetListeners;
    EventFacade::listen::<GateEvaluated, _>(Arc::new(FailingAudit(calls.clone()))).await;
    let user7 = GapUser { id: 7 };

    // The synchronous check delivers in place: the listener has run when it
    // returns.
    assert!(Gate::allows(
        "gaps-update",
        &user7,
        &GapPost { author_id: 7 }
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(Gate::denies(
        "gaps-update",
        &user7,
        &GapPost { author_id: 8 }
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 2);

    assert!(Gate::allows_async("gaps-update", &user7, &GapPost { author_id: 7 }).await);
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[test]
#[serial]
fn a_sync_check_outside_a_runtime_dispatches_its_event() {
    register_gates();
    assert!(
        tokio::runtime::Handle::try_current().is_err(),
        "this test runs outside a Tokio runtime"
    );
    let _events = EventFacade::fake();
    let user7 = GapUser { id: 7 };

    assert!(Gate::allows(
        "gaps-update",
        &user7,
        &GapPost { author_id: 7 }
    ));
    assert!(
        Gate::raw("gaps-update", &user7, &GapPost { author_id: 8 }).is_some_and(|r| r.denied())
    );
    assert!(Gate::raw("gaps-nothing", &user7, &GapPost::default()).is_none());

    let event = |decision| GateEvaluated {
        user_type: Some(std::any::type_name::<GapUser>()),
        user_id: None,
        action: "gaps-update".to_owned(),
        resource_type: std::any::type_name::<GapPost>(),
        decision,
    };
    assert_eq!(
        evaluated("gaps-update"),
        vec![event(Some(true)), event(Some(false))]
    );
    let nothing = evaluated("gaps-nothing");
    assert_eq!(nothing.len(), 1);
    assert_eq!(nothing[0].decision, None);
}

/// Counts the `GapUser` events it receives once a Tokio timer has fired,
/// then fails every one. The timer needs a runtime, so the count shows the
/// dispatch ran to completion on one.
struct SleepyFailingAudit(Arc<AtomicUsize>);

#[async_trait::async_trait]
impl Listener<GateEvaluated> for SleepyFailingAudit {
    async fn handle(&self, event: &GateEvaluated) -> Result<(), FrameworkError> {
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        if event.user_type == Some(std::any::type_name::<GapUser>()) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
        Err(FrameworkError::internal("audit log unavailable"))
    }
}

#[test]
#[serial]
fn a_sync_check_outside_a_runtime_runs_its_listeners_and_keeps_its_decision() {
    register_gates();
    let calls = Arc::new(AtomicUsize::new(0));
    let _forget = ForgetListeners;
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("a runtime to register the listener on")
        .block_on(EventFacade::listen::<GateEvaluated, _>(Arc::new(
            SleepyFailingAudit(calls.clone()),
        )));
    assert!(
        tokio::runtime::Handle::try_current().is_err(),
        "the checks below run outside a Tokio runtime"
    );
    let user7 = GapUser { id: 7 };

    // The listener has run when the check returns, and its failure does
    // not change the answer.
    assert!(Gate::allows(
        "gaps-update",
        &user7,
        &GapPost { author_id: 7 }
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(Gate::denies(
        "gaps-update",
        &user7,
        &GapPost { author_id: 8 }
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

/// A signed-in user whose type registers no gate, policy or hook: only the
/// user itself can name its type.
#[derive(Debug)]
struct BareUser {
    id: i64,
}

impl Authenticatable for BareUser {
    fn get_auth_identifier(&self) -> String {
        self.id.to_string()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn into_arc_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

/// Signs the default guard in as the `BareUser` whose id `X-Bare-User` names.
struct LoginAsBare;

#[async_trait::async_trait]
impl Middleware for LoginAsBare {
    async fn handle(&self, request: Request, next: Next) -> Response {
        if let Some(id) = header_id(&request, "x-bare-user") {
            Auth::set_user(Arc::new(BareUser { id }));
        }
        next(request).await
    }
}

#[tokio::test]
#[serial]
async fn the_event_names_a_signed_in_user_whose_type_registered_nothing() {
    let client = client(MiddlewareRegistry::new().append(LoginAsBare));
    let _events = EventFacade::fake();

    // `inspect_current`: nothing answers for a `BareUser`, so the answer is
    // the default denial and the event names the user's own type.
    let headers = [
        ("X-Bare-User", "9"),
        ("X-Action", "gaps-bare-undefined"),
        ("X-Author", "9"),
    ];
    let (current, _) = compare_as(&client, &headers).await;
    assert_eq!(current, format!("{:?}", GateResponse::deny()));
    assert_eq!(
        evaluated("gaps-bare-undefined"),
        vec![GateEvaluated {
            user_type: Some(std::any::type_name::<BareUser>()),
            user_id: Some("9".to_owned()),
            action: "gaps-bare-undefined".to_owned(),
            resource_type: std::any::type_name::<GapPost>(),
            decision: None,
        }]
    );

    // `#[authorize]`: `gaps-create` is defined for `GapUser` only, so a
    // `BareUser` gets the default denial, and the event names it too.
    client
        .get("/authorized")
        .header("X-Bare-User", "9")
        .send()
        .await
        .assert_status(403);
    let events = evaluated("gaps-create");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].user_type, Some(std::any::type_name::<BareUser>()));
    assert_eq!(events[0].user_id.as_deref(), Some("9"));
    assert_eq!(events[0].decision, None);
}

// ── Response::authorize keeps the code ───────────────────────────────────────

#[tokio::test]
#[serial]
async fn authorize_keeps_the_code_of_a_denial() {
    let error = GateResponse::deny_with("quota")
        .with_code("over-limit")
        .authorize()
        .expect_err("a denial is an error");
    assert_eq!(error.code(), Some("over-limit"));
    assert_eq!(error.status_code(), 403);
    assert!(matches!(
        &error,
        FrameworkError::Denial { message, status_code: 403, code }
            if message == "quota" && code == "over-limit"
    ));

    // A status set on the denial survives beside the code.
    let error = GateResponse::deny_with("quota")
        .with_code("over-limit")
        .with_status(429)
        .authorize()
        .expect_err("a denial is an error");
    assert_eq!(
        (error.status_code(), error.code()),
        (429, Some("over-limit"))
    );

    // A code alone keeps the default message.
    let error = GateResponse::deny()
        .with_code("blocked")
        .authorize()
        .expect_err("a denial is an error");
    assert_eq!(error.to_string(), "This action is unauthorized.");
    assert_eq!(error.code(), Some("blocked"));

    // Context keeps the code and the status.
    let error = error.context("billing");
    assert_eq!((error.status_code(), error.code()), (403, Some("blocked")));
    assert!(error.to_string().starts_with("billing: "), "{error}");
}

#[tokio::test]
#[serial]
async fn a_denial_without_a_code_maps_as_before() {
    assert!(matches!(
        GateResponse::deny().authorize(),
        Err(FrameworkError::Unauthorized)
    ));
    let error = GateResponse::deny_with("quota")
        .authorize()
        .expect_err("a denial is an error");
    assert!(matches!(
        error,
        FrameworkError::Domain {
            status_code: 403,
            ..
        }
    ));
    assert_eq!(error.code(), None);
    assert!(GateResponse::allow().authorize().is_ok());
}

#[tokio::test]
#[serial]
async fn the_code_reaches_the_caller_of_a_gate_and_the_error_response() {
    register_gates();
    let error = Gate::authorize_async("gaps-edit", &GapUser { id: 7 }, &GapPost { author_id: 8 })
        .await
        .expect_err("not the owner");
    assert_eq!(error.code(), Some("not-owner"));
    assert_eq!(error.to_string(), "Not your post.");

    let response = signed_in_client().get("/coded").send().await;
    response.assert_status(403);
    response.assert_json_path("message", "quota");
    let report = response.error_report().expect("an error response");
    assert_eq!(report.type_name(), Some("FrameworkError::Denial"));
}
