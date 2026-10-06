//! PFX-008 and PFX-002: the RenderCache key carries the root, so a page
//! rendered under one root is never served under another, and a background
//! rebuild renders under the root its key was derived under.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use sea_orm_migration::{MigrationTrait, MigratorTrait};
use suprnova::render_cache::config::RenderCacheConfig;
use suprnova::render_cache::registry::GroupPolicy;
use suprnova::render_cache::{
    CoordinatorConfig, FreshnessPolicy, L1Config, RenderCache, RenderCachePolicy,
    RepresentationClass,
};
use suprnova::testing::{TestContainer, TestContainerGuard};
use suprnova::{App, ConnectionTrait, HttpResponse, MiddlewareRegistry, Request, Router, url};
use suprnova_live::clock::{Clock, ClockError};
use suprnova_live::identity::UnixMillis;

use crate::support;

struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(suprnova::render_cache::migration::Migration)]
    }
}

/// A clock the test moves by hand, so an entry goes stale without a wait.
struct TestClock(AtomicU64);

impl Clock for TestClock {
    fn now(&self) -> Result<UnixMillis, ClockError> {
        Ok(UnixMillis::new(self.0.load(Ordering::SeqCst)))
    }
}

/// How many times the page handler ran.
static RENDERS: AtomicU64 = AtomicU64::new(0);

/// The root each render of the page handler saw, in order.
static ROOTS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

struct Booted {
    address: std::net::SocketAddr,
    clock: Arc<TestClock>,
    _guard: TestContainerGuard,
    _tempdir: tempfile::TempDir,
}

async fn boot() -> Booted {
    support::ensure_crypt();
    support::install("http://localhost");
    App::init();
    suprnova::middleware::clear_global_middleware_for_test();
    let guard = TestContainer::fake();
    let tempdir = tempfile::tempdir().expect("a database directory");
    let config = suprnova::database::DatabaseConfig::builder()
        .url(format!(
            "sqlite://{}",
            tempdir.path().join("render-cache.sqlite3").display()
        ))
        .max_connections(4)
        .min_connections(1)
        .logging(false)
        .build();
    let conn = suprnova::database::DbConnection::connect(&config)
        .await
        .expect("connect SQLite");
    conn.inner()
        .execute_unprepared("PRAGMA journal_mode=WAL")
        .await
        .expect("WAL journaling");
    Migrator::up(conn.inner(), None)
        .await
        .expect("the RenderCache migration");
    TestContainer::singleton(conn.clone());

    // Fresh for a minute, then servable stale for another minute while a
    // background rebuild refreshes it.
    let policy = RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(60_000, 60_000, 0).expect("a freshness"))
        .build()
        .expect("a policy");
    let router: Router = Router::new()
        .get("/page", |_request: Request| async {
            let render = RENDERS.fetch_add(1, Ordering::SeqCst) + 1;
            let root = url::root();
            ROOTS.lock().expect("the roots lock").push(root.clone());
            Ok(HttpResponse::html(format!(
                "<p>root={root} render={render}</p>"
            )))
        })
        .into();
    let router = router
        .try_render_cache("/page", GroupPolicy::from(policy))
        .expect("attach the policy");

    let clock = Arc::new(TestClock(AtomicU64::new(1_000_000)));
    let mut config = RenderCacheConfig::from_env()
        .expect("a RenderCache configuration")
        .with_clock(Arc::clone(&clock) as Arc<dyn Clock>);
    config.enabled = true;
    config.l1 = L1Config::Disabled;
    config.coordinator = CoordinatorConfig::Local {
        lease_ms: 30_000,
        max_waiters: 128,
    };
    let router = RenderCache::install(router, config)
        .await
        .expect("install the RenderCache");
    let address = support::serve(router, MiddlewareRegistry::from_global()).await;
    Booted {
        address,
        clock,
        _guard: guard,
        _tempdir: tempdir,
    }
}

async fn page(booted: &Booted, root: Option<&str>) -> String {
    let headers: Vec<(&str, &str)> = root
        .map(|root| vec![("x-forwarded-prefix", root)])
        .unwrap_or_default();
    let reply = support::get(booted.address, "/page", &headers).await;
    assert_eq!(reply.status, 200, "{}", reply.body);
    reply.body
}

#[tokio::test]
async fn pfx_008_a_page_cached_under_one_root_is_never_served_under_another() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_008_a_page_cached_under_one_root_is_never_served_under_another",
    )
    .await
    {
        return;
    }
    let booted = boot().await;
    assert_eq!(page(&booted, Some("/a")).await, "<p>root=/a render=1</p>");
    assert_eq!(
        page(&booted, Some("/a")).await,
        "<p>root=/a render=1</p>",
        "a second request under one root is a hit"
    );
    assert_eq!(page(&booted, Some("/b")).await, "<p>root=/b render=2</p>");
    assert_eq!(page(&booted, None).await, "<p>root= render=3</p>");
    assert_eq!(page(&booted, Some("/b")).await, "<p>root=/b render=2</p>");
}

#[tokio::test]
async fn pfx_008_a_background_rebuild_renders_under_the_root_of_its_key() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_008_a_background_rebuild_renders_under_the_root_of_its_key",
    )
    .await
    {
        return;
    }
    let booted = boot().await;
    assert_eq!(page(&booted, Some("/a")).await, "<p>root=/a render=1</p>");

    // Stale but servable: the stale entry is served and a rebuild starts in
    // a spawned task.
    booted.clock.0.fetch_add(90_000, Ordering::SeqCst);
    assert_eq!(page(&booted, Some("/a")).await, "<p>root=/a render=1</p>");

    // The rebuild runs on its own; wait for it to render.
    for _ in 0..250 {
        if RENDERS.load(Ordering::SeqCst) >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let roots = ROOTS.lock().expect("the roots lock").clone();
    assert_eq!(
        roots,
        ["/a", "/a"],
        "the background rebuild rendered under another root"
    );
}
