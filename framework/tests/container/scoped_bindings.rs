//! Scoped bindings: one value per unit of work.
//!
//! `App::scoped` and `App::bind_scoped` register a factory whose value
//! lives for one container scope, and the server opens a scope for each
//! request. These tests drive real requests through `handle_request` and
//! check what the handler resolved: one value per request and a new one for
//! the next, the value dropped when the request ends, the error outside any
//! scope, `App::spawn_scoped` against a bare `tokio::spawn`, a test fake
//! winning over the scoped binding, a factory that resolves itself, two
//! requests in flight at once, an after-commit callback that resolves the
//! value of the request that registered it, and a streamed body, which is
//! produced after the handler has returned and keeps the scope until the
//! body ends or the client drops it.
//!
//! The rest drive the scope directly: `App::run_scoped` outside a request
//! and nested inside a scope, `App::in_current_scope` handed to
//! `tokio::spawn`, and the other units of work, each with a scope of its
//! own: a queued event listener, an attempt of a queued job, a console
//! command, a scheduled run and a WebSocket session.
//!
//! Every test binds its own type: the bindings are process-global and the
//! tests of this binary run in parallel.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use serde::{Deserialize, Serialize};
use serial_test::serial;
use tokio::sync::Notify;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

use suprnova::http::{Request, text};
use suprnova::queue::worker::{WorkerConfig, register_job, run_worker};
use suprnova::testing::{TestContainer, TestDatabase};
use suprnova::ws::{OriginPolicy, WebSocketHandler, WsConfig, WsSocket};
use suprnova::{
    App, BackoffSchedule, DB, Envelope, Event, EventDispatcher, FrameworkError, HttpResponse, Job,
    Listener, MemoryQueueDriver, MiddlewareRegistry, Queue, QueueDriver, Reservation,
    ReservationToken, Router, Schedule, async_trait, command, console, handle_request,
};

/// Serve `router` on an ephemeral port through `handle_request`, accepting
/// up to `accepts` connections, each on a task of its own.
async fn spawn_server(router: impl Into<Router>, accepts: usize) -> SocketAddr {
    let router = Arc::new(router.into());
    let middleware = Arc::new(MiddlewareRegistry::new());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral listener");
    let addr = listener.local_addr().expect("local_addr");

    tokio::spawn(async move {
        for _ in 0..accepts {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let io = TokioIo::new(stream);
            let router = router.clone();
            let middleware = middleware.clone();
            tokio::spawn(async move {
                let svc = service_fn(move |req: hyper::Request<Incoming>| {
                    let router = router.clone();
                    let middleware = middleware.clone();
                    async move { Ok::<_, Infallible>(handle_request(router, middleware, req).await) }
                });
                // `with_upgrades` lets a WebSocket handshake complete; a
                // plain request is served as without it.
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, svc)
                    .with_upgrades()
                    .await;
            });
        }
    });

    addr
}

/// Issue one GET and return `(status, body)`.
async fn get(addr: SocketAddr, path: &str) -> (u16, String) {
    let stream = tokio::net::TcpStream::connect(addr)
        .await
        .expect("connect to the test server");
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake::<_, Full<Bytes>>(io)
        .await
        .expect("HTTP/1.1 handshake");
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let req = hyper::Request::builder()
        .method("GET")
        .uri(path)
        .header("Host", "localhost")
        .header("Content-Length", "0")
        .body(Full::new(Bytes::new()))
        .expect("a valid request");

    // A cap, not a measurement: a wedged server fails the test instead of
    // hanging the test process.
    let resp = tokio::time::timeout(Duration::from_secs(5), sender.send_request(req))
        .await
        .expect("send_request timeout")
        .expect("hyper send_request");

    let status = resp.status().as_u16();
    let bytes = resp
        .into_body()
        .collect()
        .await
        .expect("response body")
        .to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// Numbers every `PerRequest` the factory builds.
static PER_REQUEST_BUILT: AtomicUsize = AtomicUsize::new(0);

struct PerRequest {
    id: usize,
}

/// One value per request, the same value for every resolution inside it,
/// and a new value for the next request. Uses the concrete form,
/// `App::scoped` with `App::get`.
#[tokio::test]
async fn one_value_per_request_and_a_new_one_for_the_next() {
    App::scoped(|| {
        Arc::new(PerRequest {
            id: PER_REQUEST_BUILT.fetch_add(1, Ordering::SeqCst),
        })
    });

    let router = Router::new().get("/value", |_req| async {
        let first = App::get::<Arc<PerRequest>>();
        let second = App::get::<Arc<PerRequest>>();
        match (first, second) {
            (Some(first), Some(second)) => {
                text(format!("{} {}", first.id, Arc::ptr_eq(&first, &second)))
            }
            _ => text("unresolved"),
        }
    });
    let addr = spawn_server(router, 2).await;

    let (status_a, body_a) = get(addr, "/value").await;
    let (status_b, body_b) = get(addr, "/value").await;

    assert_eq!((status_a, status_b), (200, 200));
    let (id_a, same_a) = body_a.split_once(' ').expect("`<id> <same>`");
    let (id_b, same_b) = body_b.split_once(' ').expect("`<id> <same>`");
    assert_eq!(
        same_a, "true",
        "two resolutions in one request share the value"
    );
    assert_eq!(
        same_b, "true",
        "two resolutions in one request share the value"
    );
    assert_ne!(id_a, id_b, "the next request gets a value of its own");
    assert_eq!(
        PER_REQUEST_BUILT.load(Ordering::SeqCst),
        2,
        "the factory runs once per request"
    );
}

trait Dropping: Send + Sync {}

struct DropCounted {
    dropped: Arc<AtomicUsize>,
}

impl Dropping for DropCounted {}

impl Drop for DropCounted {
    fn drop(&mut self) {
        self.dropped.fetch_add(1, Ordering::SeqCst);
    }
}

/// The scope drops its value when the request ends. The server drops the
/// scope before it writes the response, so the count is settled by the
/// time the client has the body.
#[tokio::test]
async fn the_value_is_dropped_when_the_request_ends() {
    let built = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicUsize::new(0));
    let (built_in, dropped_in) = (Arc::clone(&built), Arc::clone(&dropped));
    App::bind_scoped::<dyn Dropping, _>(move || {
        built_in.fetch_add(1, Ordering::SeqCst);
        Arc::new(DropCounted {
            dropped: Arc::clone(&dropped_in),
        }) as Arc<dyn Dropping>
    });

    let router = Router::new().get("/drop", |_req| async {
        text(App::make::<dyn Dropping>().is_some().to_string())
    });
    let addr = spawn_server(router, 1).await;

    let (status, body) = get(addr, "/drop").await;

    assert_eq!((status, body.as_str()), (200, "true"));
    assert_eq!(built.load(Ordering::SeqCst), 1);
    assert_eq!(
        dropped.load(Ordering::SeqCst),
        1,
        "the value must be dropped with the request's scope"
    );
}

trait OutsideScope: Send + Sync {}

struct OutsideScopeImpl;

impl OutsideScope for OutsideScopeImpl {}

/// Outside any scope a scoped binding is an error that names the type and
/// says it is scoped, and no value is built.
#[test]
fn outside_a_scope_the_resolution_is_an_error_naming_the_type() {
    let built = Arc::new(AtomicUsize::new(0));
    let built_in = Arc::clone(&built);
    App::bind_scoped::<dyn OutsideScope, _>(move || {
        built_in.fetch_add(1, Ordering::SeqCst);
        Arc::new(OutsideScopeImpl) as Arc<dyn OutsideScope>
    });

    let Err(error) = App::resolve_make::<dyn OutsideScope>() else {
        panic!("a scoped binding must not resolve outside a scope");
    };
    let message = error.to_string();
    assert!(
        message.contains("OutsideScope"),
        "names the type: {message}"
    );
    assert!(
        message.contains("scoped binding"),
        "says it is scoped: {message}"
    );

    assert!(
        App::make::<dyn OutsideScope>().is_none(),
        "the Option form returns None"
    );
    assert_eq!(
        built.load(Ordering::SeqCst),
        0,
        "no value is built outside a scope"
    );
}

/// Numbers every `Spawned` value the factory builds.
static SPAWNED_BUILT: AtomicUsize = AtomicUsize::new(0);

trait Spawned: Send + Sync {
    fn id(&self) -> usize;
}

struct SpawnedImpl(usize);

impl Spawned for SpawnedImpl {
    fn id(&self) -> usize {
        self.0
    }
}

/// A task spawned with `App::spawn_scoped` shares the request's value; a
/// task spawned with a bare `tokio::spawn` has no scope and gets the error.
#[tokio::test]
async fn a_task_spawned_with_the_helper_shares_the_scope_and_a_bare_one_does_not() {
    App::bind_scoped::<dyn Spawned, _>(|| {
        Arc::new(SpawnedImpl(SPAWNED_BUILT.fetch_add(1, Ordering::SeqCst))) as Arc<dyn Spawned>
    });

    let router = Router::new().get("/spawned", |_req| async {
        let Some(own) = App::make::<dyn Spawned>() else {
            return text("unresolved");
        };
        let helper =
            App::spawn_scoped(async { App::make::<dyn Spawned>().map(|value| value.id()) });
        let helped = helper.await.ok().flatten();
        let bare = tokio::spawn(async {
            App::resolve_make::<dyn Spawned>()
                .map(|value| value.id())
                .map_err(|error| error.to_string())
        })
        .await;
        let bare = match bare {
            Ok(Ok(id)) => format!("resolved {id}"),
            Ok(Err(message)) => message,
            Err(join_error) => format!("join error: {join_error}"),
        };
        text(format!("{}|{helped:?}|{bare}", own.id()))
    });
    let addr = spawn_server(router, 1).await;

    let (status, body) = get(addr, "/spawned").await;

    assert_eq!(status, 200, "{body}");
    let mut parts = body.splitn(3, '|');
    let own = parts.next().expect("the request's own value");
    let helped = parts.next().expect("the helper task's value");
    let bare = parts.next().expect("the bare task's result");
    assert_eq!(
        helped,
        format!("Some({own})"),
        "the helper task resolves the request's value"
    );
    assert!(
        bare.contains("scoped binding") && bare.contains("Spawned"),
        "a bare spawn gets the no-scope error: {bare}"
    );
    assert_eq!(
        SPAWNED_BUILT.load(Ordering::SeqCst),
        1,
        "the request and its helper task share one value"
    );
}

trait Faked: Send + Sync {
    fn name(&self) -> &'static str;
}

struct RealFaked;

impl Faked for RealFaked {
    fn name(&self) -> &'static str {
        "real"
    }
}

struct FakeFaked {
    dropped: Arc<AtomicUsize>,
}

impl Faked for FakeFaked {
    fn name(&self) -> &'static str {
        "fake"
    }
}

impl Drop for FakeFaked {
    fn drop(&mut self) {
        self.dropped.fetch_add(1, Ordering::SeqCst);
    }
}

/// A test override wins over a scoped binding of the same type, and the
/// request's scope neither hides it nor drops it. `TestContainer::fake` is
/// thread-local, and this runtime runs the server's tasks on the test's
/// thread, so the handler sees the fake.
#[tokio::test]
async fn a_test_fake_wins_over_a_scoped_binding_and_survives_the_request() {
    let built = Arc::new(AtomicUsize::new(0));
    let built_in = Arc::clone(&built);
    App::bind_scoped::<dyn Faked, _>(move || {
        built_in.fetch_add(1, Ordering::SeqCst);
        Arc::new(RealFaked) as Arc<dyn Faked>
    });

    let _guard = TestContainer::fake();
    let fake_dropped = Arc::new(AtomicUsize::new(0));
    let fake: Arc<dyn Faked> = Arc::new(FakeFaked {
        dropped: Arc::clone(&fake_dropped),
    });
    // The test keeps only a weak handle. The test container holds the one
    // strong handle, so a scope that dropped the fake would drop it for good.
    let fake_handle = Arc::downgrade(&fake);
    TestContainer::bind::<dyn Faked>(fake);

    let router = Router::new().get("/faked", |_req| async {
        let name = App::make::<dyn Faked>().map_or("unresolved", |value| value.name());
        text(name)
    });
    let addr = spawn_server(router, 1).await;

    let (status, body) = get(addr, "/faked").await;

    assert_eq!((status, body.as_str()), (200, "fake"));
    assert_eq!(
        built.load(Ordering::SeqCst),
        0,
        "the scoped factory never runs under a fake"
    );
    assert_eq!(
        fake_dropped.load(Ordering::SeqCst),
        0,
        "the request's scope does not drop the fake"
    );
    let survivor = fake_handle
        .upgrade()
        .expect("the fake outlives the request");
    let after = App::make::<dyn Faked>().expect("the fake still resolves");
    assert!(Arc::ptr_eq(&after, &survivor), "the same fake instance");
}

trait SelfResolving: Send + Sync {
    fn nested(&self) -> &str;
}

struct SelfResolvingImpl {
    nested: String,
}

impl SelfResolving for SelfResolvingImpl {
    fn nested(&self) -> &str {
        &self.nested
    }
}

/// A factory that resolves its own binding gets an error for the inner
/// resolution, and the outer one completes: no deadlock, no unbounded
/// recursion.
#[tokio::test]
async fn a_factory_that_resolves_itself_is_an_error() {
    App::bind_scoped::<dyn SelfResolving, _>(|| {
        let nested = match App::resolve_make::<dyn SelfResolving>() {
            Ok(_) => "resolved".to_string(),
            Err(error) => error.to_string(),
        };
        Arc::new(SelfResolvingImpl { nested }) as Arc<dyn SelfResolving>
    });

    let router = Router::new().get("/self", |_req| async {
        match App::make::<dyn SelfResolving>() {
            Some(value) => text(value.nested().to_string()),
            None => text("unresolved"),
        }
    });
    let addr = spawn_server(router, 1).await;

    let (status, body) = get(addr, "/self").await;

    assert_eq!(status, 200, "{body}");
    assert!(
        body.contains("SelfResolving") && body.contains("dependency cycle"),
        "the inner resolution names the cycle: {body}"
    );
}

/// Numbers every `Inner` value the factory builds.
static INNER_BUILT: AtomicUsize = AtomicUsize::new(0);

trait Inner: Send + Sync {
    fn id(&self) -> usize;
}

struct InnerImpl(usize);

impl Inner for InnerImpl {
    fn id(&self) -> usize {
        self.0
    }
}

trait Outer: Send + Sync {
    fn inner_id(&self) -> Option<usize>;
}

struct OuterImpl(Option<usize>);

impl Outer for OuterImpl {
    fn inner_id(&self) -> Option<usize> {
        self.0
    }
}

/// A factory may resolve another scoped binding: it gets the value of the
/// same scope, the one the handler resolves directly afterwards.
#[tokio::test]
async fn a_factory_may_resolve_another_scoped_binding() {
    App::bind_scoped::<dyn Inner, _>(|| {
        Arc::new(InnerImpl(INNER_BUILT.fetch_add(1, Ordering::SeqCst))) as Arc<dyn Inner>
    });
    App::bind_scoped::<dyn Outer, _>(|| {
        let inner = App::make::<dyn Inner>().map(|value| value.id());
        Arc::new(OuterImpl(inner)) as Arc<dyn Outer>
    });

    let router = Router::new().get("/nested", |_req| async {
        let outer = App::make::<dyn Outer>().and_then(|value| value.inner_id());
        let inner = App::make::<dyn Inner>().map(|value| value.id());
        text(format!("{outer:?} {inner:?}"))
    });
    let addr = spawn_server(router, 1).await;

    let (status, body) = get(addr, "/nested").await;

    assert_eq!(status, 200, "{body}");
    let (outer, inner) = body.split_once(' ').expect("`<outer> <inner>`");
    assert!(
        outer.starts_with("Some("),
        "the outer factory resolved Inner: {body}"
    );
    assert_eq!(outer, inner, "one Inner per scope: {body}");
    assert_eq!(INNER_BUILT.load(Ordering::SeqCst), 1);
}

/// Numbers every `Concurrent` value the factory builds.
static CONCURRENT_BUILT: AtomicUsize = AtomicUsize::new(0);

trait Concurrent: Send + Sync {
    fn id(&self) -> usize;
}

struct ConcurrentImpl(usize);

impl Concurrent for ConcurrentImpl {
    fn id(&self) -> usize {
        self.0
    }
}

/// Two requests in flight at once each see their own value. The barrier
/// holds both handlers between their two resolutions until both have made
/// the first, so the scopes overlap without depending on timing.
#[tokio::test]
async fn two_concurrent_requests_do_not_see_each_others_value() {
    App::bind_scoped::<dyn Concurrent, _>(|| {
        Arc::new(ConcurrentImpl(
            CONCURRENT_BUILT.fetch_add(1, Ordering::SeqCst),
        )) as Arc<dyn Concurrent>
    });

    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let router = Router::new().get("/concurrent", move |_req| {
        let barrier = Arc::clone(&barrier);
        async move {
            let before = App::make::<dyn Concurrent>().map(|value| value.id());
            barrier.wait().await;
            let after = App::make::<dyn Concurrent>().map(|value| value.id());
            text(format!("{before:?} {after:?}"))
        }
    });
    let addr = spawn_server(router, 2).await;

    let ((status_a, body_a), (status_b, body_b)) =
        tokio::join!(get(addr, "/concurrent"), get(addr, "/concurrent"));

    assert_eq!((status_a, status_b), (200, 200));
    let (before_a, after_a) = body_a.split_once(' ').expect("`<before> <after>`");
    let (before_b, after_b) = body_b.split_once(' ').expect("`<before> <after>`");
    assert!(before_a.starts_with("Some("), "{body_a}");
    assert!(before_b.starts_with("Some("), "{body_b}");
    assert_eq!(
        before_a, after_a,
        "a request keeps its value across the wait"
    );
    assert_eq!(
        before_b, after_b,
        "a request keeps its value across the wait"
    );
    assert_ne!(before_a, before_b, "each request has a value of its own");
    assert_eq!(CONCURRENT_BUILT.load(Ordering::SeqCst), 2);
}

/// Numbers every `CommitProbe` value the factory builds.
static COMMIT_BUILT: AtomicUsize = AtomicUsize::new(0);

trait CommitProbe: Send + Sync {
    fn id(&self) -> usize;
}

struct CommitProbeValue(usize);

impl CommitProbe for CommitProbeValue {
    fn id(&self) -> usize {
        self.0
    }
}

/// A job whose push waits for the surrounding transaction to commit.
#[derive(Serialize, Deserialize)]
struct CommitProbeJob;

#[async_trait]
impl Job for CommitProbeJob {
    fn job_name() -> &'static str {
        "scoped-bindings-commit-probe"
    }

    fn after_commit() -> bool {
        true
    }

    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}

/// A queue driver whose `push` runs inside the after-commit callback of a
/// deferred push, and records the scoped value it resolves there.
#[derive(Default)]
struct CommitProbeDriver {
    seen: Mutex<Vec<Option<usize>>>,
}

#[async_trait]
impl QueueDriver for CommitProbeDriver {
    async fn push(&self, _env: Envelope) -> Result<(), FrameworkError> {
        let id = App::make::<dyn CommitProbe>().map(|value| value.id());
        self.seen.lock().expect("unpoisoned").push(id);
        Ok(())
    }

    async fn pop(
        &self,
        _visibility_timeout: Duration,
    ) -> Result<Option<Reservation>, FrameworkError> {
        Ok(None)
    }

    async fn ack(&self, _token: &ReservationToken) -> Result<(), FrameworkError> {
        Ok(())
    }

    async fn nack(
        &self,
        _token: &ReservationToken,
        _requeue_delay: Duration,
    ) -> Result<(), FrameworkError> {
        Ok(())
    }

    fn name(&self) -> &'static str {
        "commit-probe"
    }
}

/// An after-commit callback belongs to the request that registered it: it
/// runs on a task of its own after the commit, and resolves the request's
/// value. `TestDatabase` and the test server share this test's thread, so
/// the handler sees the thread-local database. Serial with the other test
/// that installs the process-wide queue driver.
#[tokio::test]
#[serial]
async fn an_after_commit_callback_resolves_the_value_of_the_request_that_registered_it() {
    App::bind_scoped::<dyn CommitProbe, _>(|| {
        Arc::new(CommitProbeValue(
            COMMIT_BUILT.fetch_add(1, Ordering::SeqCst),
        )) as Arc<dyn CommitProbe>
    });
    let driver = Arc::new(CommitProbeDriver::default());
    Queue::set_driver(driver.clone());
    let _db = TestDatabase::sqlite_memory()
        .await
        .expect("in-memory sqlite");

    let router = Router::new().get("/commit", |_req| async {
        let own = App::make::<dyn CommitProbe>().map(|value| value.id());
        let pushed = DB::transaction(|_tx| Box::pin(async { Queue::push(CommitProbeJob).await }))
            .await
            .is_ok();
        text(format!("{own:?} {pushed}"))
    });
    let addr = spawn_server(router, 1).await;

    let (status, body) = get(addr, "/commit").await;

    assert_eq!(status, 200, "{body}");
    let (own, pushed) = body.split_once(' ').expect("`<own> <pushed>`");
    assert_eq!(pushed, "true", "the deferred push succeeded");
    assert!(
        own.starts_with("Some("),
        "the request resolved a value: {body}"
    );
    let seen = driver.seen.lock().expect("unpoisoned").clone();
    assert_eq!(seen.len(), 1, "one push, after the commit");
    assert_eq!(
        format!("{:?}", seen[0]),
        own,
        "the callback resolves the value of the request that registered it"
    );
    assert_eq!(COMMIT_BUILT.load(Ordering::SeqCst), 1);
}

trait RunScopedProbe: Send + Sync {
    fn id(&self) -> usize;
}

struct RunScopedProbeValue {
    id: usize,
    dropped: Arc<AtomicUsize>,
}

impl RunScopedProbe for RunScopedProbeValue {
    fn id(&self) -> usize {
        self.id
    }
}

impl Drop for RunScopedProbeValue {
    fn drop(&mut self) {
        self.dropped.fetch_add(1, Ordering::SeqCst);
    }
}

/// `App::run_scoped` opens a scope outside any request: one value inside
/// it, dropped when its future completes.
#[tokio::test]
async fn run_scoped_gives_one_value_and_drops_it_at_the_end() {
    let built = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicUsize::new(0));
    let (built_in, dropped_in) = (Arc::clone(&built), Arc::clone(&dropped));
    App::bind_scoped::<dyn RunScopedProbe, _>(move || {
        Arc::new(RunScopedProbeValue {
            id: built_in.fetch_add(1, Ordering::SeqCst),
            dropped: Arc::clone(&dropped_in),
        }) as Arc<dyn RunScopedProbe>
    });

    let (first, second) = App::run_scoped(async {
        let first = App::make::<dyn RunScopedProbe>().map(|value| value.id());
        let second = App::make::<dyn RunScopedProbe>().map(|value| value.id());
        (first, second)
    })
    .await;

    assert_eq!(first, Some(0), "the scope builds the value at first use");
    assert_eq!(first, second, "one value for the whole scope");
    assert_eq!(built.load(Ordering::SeqCst), 1);
    assert_eq!(
        dropped.load(Ordering::SeqCst),
        1,
        "the value is dropped when run_scoped returns"
    );
    assert!(
        App::make::<dyn RunScopedProbe>().is_none(),
        "no scope is active after run_scoped returns"
    );
}

/// Numbers every `NestedProbe` value the factory builds.
static NESTED_BUILT: AtomicUsize = AtomicUsize::new(0);

trait NestedProbe: Send + Sync {
    fn id(&self) -> usize;
}

struct NestedProbeValue(usize);

impl NestedProbe for NestedProbeValue {
    fn id(&self) -> usize {
        self.0
    }
}

/// A nested `App::run_scoped` is a unit of work of its own: it does not see
/// the outer value, and the outer value is visible again after it.
#[tokio::test]
async fn a_nested_run_scoped_does_not_see_the_outer_value() {
    App::bind_scoped::<dyn NestedProbe, _>(|| {
        Arc::new(NestedProbeValue(
            NESTED_BUILT.fetch_add(1, Ordering::SeqCst),
        )) as Arc<dyn NestedProbe>
    });

    let (outer, inner, outer_after) = App::run_scoped(async {
        let outer = App::make::<dyn NestedProbe>().map(|value| value.id());
        let nested =
            App::run_scoped(async { App::make::<dyn NestedProbe>().map(|value| value.id()) });
        let inner = nested.await;
        let outer_after = App::make::<dyn NestedProbe>().map(|value| value.id());
        (outer, inner, outer_after)
    })
    .await;

    assert!(outer.is_some(), "the outer scope resolves a value");
    assert!(inner.is_some(), "the nested scope resolves a value");
    assert_ne!(outer, inner, "the nested scope builds a value of its own");
    assert_eq!(outer, outer_after, "the outer value is visible again");
    assert_eq!(NESTED_BUILT.load(Ordering::SeqCst), 2);
}

/// Numbers every `CarriedProbe` value the factory builds.
static CARRIED_BUILT: AtomicUsize = AtomicUsize::new(0);

trait CarriedProbe: Send + Sync {
    fn id(&self) -> usize;
}

struct CarriedProbeValue(usize);

impl CarriedProbe for CarriedProbeValue {
    fn id(&self) -> usize {
        self.0
    }
}

/// `App::in_current_scope` handed to a bare `tokio::spawn` carries the
/// caller's scope into the task, which shares the caller's value. Outside a
/// scope the wrapped future behaves as the bare one.
#[tokio::test]
async fn in_current_scope_carries_the_caller_scope_into_tokio_spawn() {
    App::bind_scoped::<dyn CarriedProbe, _>(|| {
        Arc::new(CarriedProbeValue(
            CARRIED_BUILT.fetch_add(1, Ordering::SeqCst),
        )) as Arc<dyn CarriedProbe>
    });

    let (own, carried) = App::run_scoped(async {
        let own = App::make::<dyn CarriedProbe>().map(|value| value.id());
        let task = tokio::spawn(App::in_current_scope(async {
            App::make::<dyn CarriedProbe>().map(|value| value.id())
        }));
        (own, task.await.ok().flatten())
    })
    .await;

    assert!(own.is_some(), "the caller resolves a value");
    assert_eq!(own, carried, "the spawned task shares the caller's value");
    assert_eq!(CARRIED_BUILT.load(Ordering::SeqCst), 1);

    let without_a_scope =
        App::in_current_scope(async { App::resolve_make::<dyn CarriedProbe>().is_err() });
    assert!(
        without_a_scope.await,
        "outside a scope the wrapped future has no scope either"
    );
}

/// Numbers every `ListenerProbe` value the factory builds.
static LISTENER_BUILT: AtomicUsize = AtomicUsize::new(0);

trait ListenerProbe: Send + Sync {
    fn id(&self) -> usize;
}

struct ListenerProbeValue(usize);

impl ListenerProbe for ListenerProbeValue {
    fn id(&self) -> usize {
        self.0
    }
}

#[derive(Debug, Clone)]
struct ScopedPing;

impl Event for ScopedPing {
    fn event_name() -> &'static str {
        "ScopedPing"
    }

    fn queued() -> bool {
        true
    }
}

/// Records the scoped value a queued listener resolves.
struct ScopedPingListener(Arc<Mutex<Vec<Option<usize>>>>);

#[async_trait]
impl Listener<ScopedPing> for ScopedPingListener {
    async fn handle(&self, _event: &ScopedPing) -> Result<(), FrameworkError> {
        let id = App::make::<dyn ListenerProbe>().map(|value| value.id());
        self.0.lock().expect("unpoisoned").push(id);
        Ok(())
    }
}

/// A queued listener is a unit of work of its own: it gets a value of its
/// own, not the value of the scope that dispatched the event.
#[tokio::test]
async fn a_queued_listener_gets_a_value_of_its_own() {
    App::bind_scoped::<dyn ListenerProbe, _>(|| {
        Arc::new(ListenerProbeValue(
            LISTENER_BUILT.fetch_add(1, Ordering::SeqCst),
        )) as Arc<dyn ListenerProbe>
    });
    let dispatcher = EventDispatcher::new();
    let seen = Arc::new(Mutex::new(Vec::new()));
    dispatcher
        .listen::<ScopedPing, _>(Arc::new(ScopedPingListener(Arc::clone(&seen))))
        .await;

    let own = App::run_scoped(async {
        let own = App::make::<dyn ListenerProbe>().map(|value| value.id());
        dispatcher.dispatch(ScopedPing).await.expect("dispatch");
        // A cap, not a measurement: the listener runs on a task of its own.
        let unfinished = dispatcher.drain_queued(Duration::from_secs(5)).await;
        assert_eq!(unfinished, 0, "the queued listener finished");
        own
    })
    .await;

    let seen = seen.lock().expect("unpoisoned").clone();
    assert_eq!(seen.len(), 1, "the listener ran once");
    assert!(own.is_some(), "the dispatching scope resolves a value");
    assert!(
        seen[0].is_some(),
        "the listener resolves a value in its own scope"
    );
    assert_ne!(
        seen[0], own,
        "the listener does not share the dispatcher's value"
    );
    assert_eq!(LISTENER_BUILT.load(Ordering::SeqCst), 2);
}

/// The value a unit of work resolved at its start and at its end.
type ResolvedTwice = (Option<usize>, Option<usize>);

/// Numbers every `StreamProbe` value the factory builds.
static STREAM_BUILT: AtomicUsize = AtomicUsize::new(0);

trait StreamProbe: Send + Sync {
    fn id(&self) -> usize;
}

struct StreamProbeValue(usize);

impl StreamProbe for StreamProbeValue {
    fn id(&self) -> usize {
        self.0
    }
}

/// A streamed body is produced after the handler has returned, and still
/// in the request's scope: the stream resolves the value the handler
/// resolved.
#[tokio::test]
async fn a_streamed_body_resolves_the_value_of_its_request() {
    App::bind_scoped::<dyn StreamProbe, _>(|| {
        Arc::new(StreamProbeValue(
            STREAM_BUILT.fetch_add(1, Ordering::SeqCst),
        )) as Arc<dyn StreamProbe>
    });

    let router = Router::new().get("/streamed", |_req| async {
        let own = App::make::<dyn StreamProbe>().map(|value| value.id());
        // Polled by the server once this handler has returned.
        let chunks = futures::stream::once(async move {
            let streamed = App::make::<dyn StreamProbe>().map(|value| value.id());
            Ok::<_, Infallible>(Bytes::from(format!("{own:?} {streamed:?}")))
        });
        HttpResponse::stream_bytes(chunks).ok()
    });
    let addr = spawn_server(router, 1).await;

    let (status, body) = get(addr, "/streamed").await;

    assert_eq!(status, 200, "{body}");
    let (own, streamed) = body.split_once(' ').expect("`<own> <streamed>`");
    assert!(
        own.starts_with("Some("),
        "the handler resolved a value: {body}"
    );
    assert_eq!(own, streamed, "the stream resolves the request's value");
    assert_eq!(STREAM_BUILT.load(Ordering::SeqCst), 1);
}

/// The client half of one request whose response body is still open: the
/// sender that keeps the connection usable, the response, and the task that
/// drives the connection.
type OpenStream = (
    hyper::client::conn::http1::SendRequest<Full<Bytes>>,
    hyper::Response<Incoming>,
    tokio::task::JoinHandle<()>,
);

/// Issue one GET and return as soon as the response head arrives, leaving
/// the body unread.
async fn open_stream(addr: SocketAddr, path: &str) -> OpenStream {
    let stream = tokio::net::TcpStream::connect(addr)
        .await
        .expect("connect to the test server");
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake::<_, Full<Bytes>>(io)
        .await
        .expect("HTTP/1.1 handshake");
    let driver = tokio::spawn(async move {
        let _ = conn.await;
    });
    let req = hyper::Request::builder()
        .method("GET")
        .uri(path)
        .header("Host", "localhost")
        .header("Content-Length", "0")
        .body(Full::new(Bytes::new()))
        .expect("a valid request");
    // A cap against a wedged server, not a measurement.
    let resp = tokio::time::timeout(Duration::from_secs(5), sender.send_request(req))
        .await
        .expect("send_request timeout")
        .expect("hyper send_request");
    (sender, resp, driver)
}

/// The next chunk of a response body, `None` once the body has ended. The
/// timeout only keeps a wedged server from hanging the test.
async fn next_chunk(body: &mut Incoming) -> Option<Bytes> {
    let frame = tokio::time::timeout(Duration::from_secs(5), body.frame())
        .await
        .expect("body frame timeout")?;
    Some(
        frame
            .expect("a readable frame")
            .into_data()
            .unwrap_or_default(),
    )
}

/// Wait until `dropped` reaches `expected`. Each drop fires `signal`, so the
/// wait is on the drop itself; the timeout only keeps a value that is never
/// dropped from hanging the test.
async fn wait_for_drops(dropped: &AtomicUsize, signal: &Notify, expected: usize) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while dropped.load(Ordering::SeqCst) < expected {
            signal.notified().await;
        }
    })
    .await
    .expect("the scoped value must be dropped once its body is gone");
}

trait EndProbe: Send + Sync {}

trait AbandonProbe: Send + Sync {}

/// A scoped value that counts its drops and announces each one.
struct DropSignal {
    dropped: Arc<AtomicUsize>,
    signal: Arc<Notify>,
}

impl EndProbe for DropSignal {}

impl AbandonProbe for DropSignal {}

impl Drop for DropSignal {
    fn drop(&mut self) {
        self.dropped.fetch_add(1, Ordering::SeqCst);
        self.signal.notify_one();
    }
}

/// The scope of a streamed body ends with the body: the value the handler
/// resolved lives while the body is open, and is dropped once the client
/// has read the body to its end.
#[tokio::test]
async fn the_scope_of_a_streamed_body_ends_when_the_body_ends() {
    let built = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicUsize::new(0));
    let signal = Arc::new(Notify::new());
    let (built_in, dropped_in, signal_in) = (built.clone(), dropped.clone(), signal.clone());
    App::bind_scoped::<dyn EndProbe, _>(move || {
        built_in.fetch_add(1, Ordering::SeqCst);
        Arc::new(DropSignal {
            dropped: dropped_in.clone(),
            signal: signal_in.clone(),
        }) as Arc<dyn EndProbe>
    });

    let gate = Arc::new(Notify::new());
    let gate_in = gate.clone();
    let router = Router::new().get("/end", move |_req| {
        let gate = gate_in.clone();
        async move {
            // The handler holds no handle past its return: the scope alone
            // keeps the value.
            let resolved = App::make::<dyn EndProbe>().is_some();
            let first = futures::stream::once(async move {
                Ok::<_, Infallible>(Bytes::from(format!("resolved={resolved} ")))
            });
            let last = futures::stream::once(async move {
                gate.notified().await;
                Ok::<_, Infallible>(Bytes::from_static(b"last"))
            });
            HttpResponse::stream_bytes(first.chain(last)).ok()
        }
    });
    let addr = spawn_server(router, 1).await;

    let (_sender, mut response, _driver) = open_stream(addr, "/end").await;
    assert_eq!(response.status().as_u16(), 200);
    let first = next_chunk(response.body_mut())
        .await
        .expect("the first chunk");
    assert_eq!(first.as_ref(), b"resolved=true ");
    assert_eq!(
        dropped.load(Ordering::SeqCst),
        0,
        "the value lives while the body is open"
    );

    gate.notify_one();
    let last = next_chunk(response.body_mut())
        .await
        .expect("the last chunk");
    assert_eq!(last.as_ref(), b"last");
    assert!(
        next_chunk(response.body_mut()).await.is_none(),
        "the body ends after its last chunk"
    );

    wait_for_drops(&dropped, &signal, 1).await;
    assert_eq!(built.load(Ordering::SeqCst), 1);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
}

/// A client that drops the response unread ends the body, and with it the
/// scope: the value the handler resolved is dropped, not kept for as long as
/// the stream could have run.
#[tokio::test]
async fn the_scope_of_a_streamed_body_ends_when_the_client_drops_it() {
    let built = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicUsize::new(0));
    let signal = Arc::new(Notify::new());
    let (built_in, dropped_in, signal_in) = (built.clone(), dropped.clone(), signal.clone());
    App::bind_scoped::<dyn AbandonProbe, _>(move || {
        built_in.fetch_add(1, Ordering::SeqCst);
        Arc::new(DropSignal {
            dropped: dropped_in.clone(),
            signal: signal_in.clone(),
        }) as Arc<dyn AbandonProbe>
    });

    let router = Router::new().get("/abandoned", |_req| async {
        let resolved = App::make::<dyn AbandonProbe>().is_some();
        let first = futures::stream::once(async move {
            Ok::<_, Infallible>(Bytes::from(format!("resolved={resolved} ")))
        });
        // The stream never ends: only the client leaving can end the body.
        let never = futures::stream::once(std::future::pending::<Result<Bytes, Infallible>>());
        HttpResponse::stream_bytes(first.chain(never)).ok()
    });
    let addr = spawn_server(router, 1).await;

    let (sender, mut response, driver) = open_stream(addr, "/abandoned").await;
    let first = next_chunk(response.body_mut())
        .await
        .expect("the first chunk");
    assert_eq!(first.as_ref(), b"resolved=true ");
    assert_eq!(
        dropped.load(Ordering::SeqCst),
        0,
        "the value lives while the body is open"
    );

    drop(response);
    drop(sender);
    driver.abort();

    wait_for_drops(&dropped, &signal, 1).await;
    assert_eq!(built.load(Ordering::SeqCst), 1);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
}

/// What each attempt of `RetriedProbeJob` resolved.
static ATTEMPTS_SEEN: Mutex<Vec<ResolvedTwice>> = Mutex::new(Vec::new());

/// Numbers every `AttemptProbe` value the factory builds.
static ATTEMPT_BUILT: AtomicUsize = AtomicUsize::new(0);

trait AttemptProbe: Send + Sync {
    fn id(&self) -> usize;
}

struct AttemptProbeValue(usize);

impl AttemptProbe for AttemptProbeValue {
    fn id(&self) -> usize {
        self.0
    }
}

/// A job that fails its first attempt and succeeds on the retry.
#[derive(Serialize, Deserialize)]
struct RetriedProbeJob;

#[async_trait]
impl Job for RetriedProbeJob {
    fn job_name() -> &'static str {
        "scoped-bindings-retried-probe"
    }

    fn max_tries() -> u32 {
        2
    }

    fn backoff() -> BackoffSchedule {
        BackoffSchedule::Fixed { secs: 0 }
    }

    async fn handle(self) -> Result<(), FrameworkError> {
        let first = App::make::<dyn AttemptProbe>().map(|value| value.id());
        let second = App::make::<dyn AttemptProbe>().map(|value| value.id());
        let attempt = {
            let mut seen = ATTEMPTS_SEEN.lock().expect("unpoisoned");
            seen.push((first, second));
            seen.len()
        };
        if attempt == 1 {
            Err(FrameworkError::internal("the first attempt fails"))
        } else {
            Ok(())
        }
    }
}

/// Each attempt of a queued job is a unit of work: one value for the
/// attempt, and a new value for the retry. Serial with the other test that
/// installs the process-wide queue driver.
#[tokio::test]
#[serial]
async fn each_attempt_of_a_queued_job_gets_a_scope_of_its_own() {
    App::bind_scoped::<dyn AttemptProbe, _>(|| {
        Arc::new(AttemptProbeValue(
            ATTEMPT_BUILT.fetch_add(1, Ordering::SeqCst),
        )) as Arc<dyn AttemptProbe>
    });
    let driver = Arc::new(MemoryQueueDriver::new());
    Queue::set_driver(driver.clone());
    register_job::<RetriedProbeJob>();
    Queue::push(RetriedProbeJob).await.expect("push the job");

    let config = WorkerConfig {
        poll_interval: Duration::from_millis(5),
        max_jobs: Some(2),
        ..WorkerConfig::default()
    };
    // A cap, not a measurement: the worker stops by itself after the two
    // attempts settle.
    tokio::time::timeout(
        Duration::from_secs(10),
        run_worker(driver, config, CancellationToken::new()),
    )
    .await
    .expect("the worker settles both attempts");

    let seen = ATTEMPTS_SEEN.lock().expect("unpoisoned").clone();
    assert_eq!(seen.len(), 2, "one failed attempt and its retry: {seen:?}");
    for (first, second) in &seen {
        assert!(first.is_some(), "an attempt resolves the scoped binding");
        assert_eq!(first, second, "one value for the whole attempt");
    }
    assert_ne!(seen[0].0, seen[1].0, "the retry gets a new value");
}

/// What each run of the `scoped-bindings:probe` command resolved.
static COMMANDS_SEEN: Mutex<Vec<ResolvedTwice>> = Mutex::new(Vec::new());

/// Numbers every `CommandProbe` value the factory builds.
static COMMAND_BUILT: AtomicUsize = AtomicUsize::new(0);

trait CommandProbe: Send + Sync {
    fn id(&self) -> usize;
}

struct CommandProbeValue(usize);

impl CommandProbe for CommandProbeValue {
    fn id(&self) -> usize {
        self.0
    }
}

#[command(
    name = "scoped-bindings:probe",
    description = "Resolves a scoped binding twice"
)]
async fn scoped_bindings_probe(_args: Vec<String>) -> Result<(), FrameworkError> {
    let first = App::make::<dyn CommandProbe>().map(|value| value.id());
    let second = App::make::<dyn CommandProbe>().map(|value| value.id());
    let mut seen = COMMANDS_SEEN.lock().expect("unpoisoned");
    seen.push((first, second));
    Ok(())
}

/// Each console command is a unit of work: one value for the run, and a
/// new value for the next run.
#[tokio::test]
async fn each_console_command_gets_a_scope_of_its_own() {
    App::bind_scoped::<dyn CommandProbe, _>(|| {
        Arc::new(CommandProbeValue(
            COMMAND_BUILT.fetch_add(1, Ordering::SeqCst),
        )) as Arc<dyn CommandProbe>
    });

    for _ in 0..2 {
        console::test(["scoped-bindings:probe"])
            .run()
            .await
            .assert_successful();
    }

    let seen = COMMANDS_SEEN.lock().expect("unpoisoned").clone();
    assert_eq!(seen.len(), 2, "two runs: {seen:?}");
    for (first, second) in &seen {
        assert!(first.is_some(), "a command resolves the scoped binding");
        assert_eq!(first, second, "one value for the whole run");
    }
    assert_ne!(seen[0].0, seen[1].0, "the next run gets a new value");
}

/// What each scheduled run resolved.
static TASKS_SEEN: Mutex<Vec<ResolvedTwice>> = Mutex::new(Vec::new());

/// Numbers every `TaskProbe` value the factory builds.
static TASK_BUILT: AtomicUsize = AtomicUsize::new(0);

trait TaskProbe: Send + Sync {
    fn id(&self) -> usize;
}

struct TaskProbeValue(usize);

impl TaskProbe for TaskProbeValue {
    fn id(&self) -> usize {
        self.0
    }
}

/// The body of both scheduled tasks below.
async fn record_task_probe() -> Result<(), FrameworkError> {
    let first = App::make::<dyn TaskProbe>().map(|value| value.id());
    let second = App::make::<dyn TaskProbe>().map(|value| value.id());
    let mut seen = TASKS_SEEN.lock().expect("unpoisoned");
    seen.push((first, second));
    Ok(())
}

/// Each scheduled run is a unit of work. Two tasks, because a task runs at
/// most once a minute: each run gets one value, and the two runs get two.
#[tokio::test]
async fn each_scheduled_run_gets_a_scope_of_its_own() {
    App::bind_scoped::<dyn TaskProbe, _>(|| {
        Arc::new(TaskProbeValue(TASK_BUILT.fetch_add(1, Ordering::SeqCst))) as Arc<dyn TaskProbe>
    });
    let mut schedule = Schedule::new();
    let first = schedule
        .call(record_task_probe)
        .name("scoped-bindings-first");
    schedule.add(first);
    let second = schedule
        .call(record_task_probe)
        .name("scoped-bindings-second");
    schedule.add(second);

    let results = schedule.run_all_tasks().await;

    assert_eq!(results.len(), 2, "both tasks ran: {results:?}");
    assert!(
        results.iter().all(|(_, result)| result.is_ok()),
        "both tasks succeeded: {results:?}"
    );
    let seen = TASKS_SEEN.lock().expect("unpoisoned").clone();
    assert_eq!(seen.len(), 2, "two runs: {seen:?}");
    for (first, second) in &seen {
        assert!(first.is_some(), "a run resolves the scoped binding");
        assert_eq!(first, second, "one value for the whole run");
    }
    assert_ne!(seen[0].0, seen[1].0, "each run gets its own value");
}

/// Numbers every `SessionProbe` value the factory builds.
static SESSION_BUILT: AtomicUsize = AtomicUsize::new(0);

trait SessionProbe: Send + Sync {
    fn id(&self) -> usize;
}

struct SessionProbeValue(usize);

impl SessionProbe for SessionProbeValue {
    fn id(&self) -> usize {
        self.0
    }
}

/// Answers every text frame with the scoped value it resolves.
struct SessionProbeHandler;

#[async_trait]
impl WebSocketHandler for SessionProbeHandler {
    async fn handle(&self, mut socket: WsSocket, _req: Request) -> Result<(), FrameworkError> {
        while socket.recv_text().await?.is_some() {
            let id = App::make::<dyn SessionProbe>().map(|value| value.id());
            socket.send_text(format!("{id:?}")).await?;
        }
        Ok(())
    }
}

/// The client end of a WebSocket session in these tests.
type ClientSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Send `text` on `ws` and return the text of the reply. The timeout is a
/// cap, not a measurement: a handler that never answers fails the test.
async fn exchange(ws: &mut ClientSocket, text: &str) -> String {
    ws.send(Message::text(text.to_owned()))
        .await
        .expect("send a frame");
    let reply = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .expect("a reply within the cap")
        .expect("the socket is open")
        .expect("no error on the reply");
    reply.to_text().expect("the reply is text").to_owned()
}

/// Each WebSocket session is a unit of work: one value for the session
/// across its messages, and a new value for the next session.
#[tokio::test]
async fn each_websocket_session_gets_a_scope_of_its_own() {
    App::bind_scoped::<dyn SessionProbe, _>(|| {
        Arc::new(SessionProbeValue(
            SESSION_BUILT.fetch_add(1, Ordering::SeqCst),
        )) as Arc<dyn SessionProbe>
    });
    // The test client sends no `Origin` header.
    let config = WsConfig {
        origin_policy: OriginPolicy::AllowAny,
        ..Default::default()
    };
    let router = Router::new().ws_with_config("/ws/scoped", SessionProbeHandler, config);
    let addr = spawn_server(router, 2).await;
    let url = format!("ws://{addr}/ws/scoped");

    let mut sessions = Vec::new();
    for _ in 0..2 {
        let (mut ws, _response) = tokio_tungstenite::connect_async(url.as_str())
            .await
            .expect("open a WebSocket session");
        let first = exchange(&mut ws, "first").await;
        let second = exchange(&mut ws, "second").await;
        sessions.push((first, second));
    }

    for (first, second) in &sessions {
        assert!(
            first.starts_with("Some("),
            "a session resolves a value: {first}"
        );
        assert_eq!(first, second, "one value for the whole session");
    }
    assert_ne!(
        sessions[0].0, sessions[1].0,
        "the next session gets a new value"
    );
}
