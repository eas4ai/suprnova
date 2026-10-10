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

/// A SQLite database in a fresh directory with the RenderCache migration
/// applied, bound in the test container.
async fn database() -> tempfile::TempDir {
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
    TestContainer::singleton(conn);
    tempdir
}

async fn boot() -> Booted {
    support::ensure_crypt();
    support::install("http://localhost");
    App::init();
    suprnova::middleware::clear_global_middleware_for_test();
    let guard = TestContainer::fake();
    let tempdir = database().await;

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

#[tokio::test]
async fn pfx_008_a_refreshed_entry_is_served_only_under_the_root_it_was_rebuilt_for() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_008_a_refreshed_entry_is_served_only_under_the_root_it_was_rebuilt_for",
    )
    .await
    {
        return;
    }
    let booted = boot().await;
    // The root `/a` comes from `APP_URL` here, so the test seam that reads
    // the stored entry outside a request reads `/a`'s.
    support::install("http://localhost/a");
    assert_eq!(page(&booted, None).await, "<p>root=/a render=1</p>");
    let published = RenderCache::stored_fence_token_for_test("/page")
        .await
        .expect("`/a`'s entry is stored");

    // Stale but servable: the stale entry is served and a background rebuild
    // renders and publishes a fresh one for `/a`.
    booted.clock.0.fetch_add(90_000, Ordering::SeqCst);
    assert_eq!(page(&booted, None).await, "<p>root=/a render=1</p>");
    let mut republished = published;
    for _ in 0..250 {
        republished = RenderCache::stored_fence_token_for_test("/page")
            .await
            .expect("`/a`'s entry is stored");
        if republished != published {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_ne!(republished, published, "the background rebuild published");
    assert_eq!(RENDERS.load(Ordering::SeqCst), 2, "one background rebuild");
    assert_eq!(page(&booted, None).await, "<p>root=/a render=2</p>");

    // The refreshed entry is `/a`'s: `/b` and the host root render their own.
    assert_eq!(page(&booted, Some("/b")).await, "<p>root=/b render=3</p>");
    assert_eq!(page(&booted, Some("/")).await, "<p>root= render=4</p>");
    assert_eq!(page(&booted, None).await, "<p>root=/a render=2</p>");
    assert_eq!(page(&booted, Some("/b")).await, "<p>root=/b render=3</p>");
}

// ---- Stitched and nested entries ----------------------------------------
//
// `/leaf` is a public island whose document is stored Complete, and
// `/shell` an identity-bound island whose document is stored as a
// Composite entry: a shared shell with one slot, assembled for each request
// on a hit without the handler. Both are cached under the stitched class.
// Every Live document names its action endpoint under the root (PFX-006),
// so the bytes of a document show which root it was rendered under.

use suprnova::live::{
    CanonicalValue, LiveBootstrapOptions, LiveDocument, LiveMount, LiveTenantMiddleware, MountFlags,
};
use suprnova::render_cache::StorageLayers;
use suprnova::render_cache::testing::rewrite_composite_for_test;
use suprnova::view::{AssetSet, DocumentResponseIntent, ViewName};
use suprnova::{AuthMiddleware, StatusCode};
use suprnova_live::render_cache::composite::{Segment, SlotFailurePolicy};
use suprnova_live::render_cache::entry::EntryKind;
use suprnova_live::render_cache::key::RenderKey;

use crate::live_dogfood_support::{
    DogfoodCounter, DogfoodDocument, LoginHeader, MemorySessionStore, Tenantless,
};

/// The public document a stitched document names by reference.
const LEAF_PATH: &str = "/leaf";
const LEAF_KEY: &str = "pfx-leaf";

/// The identity-bound document stored as a Composite entry.
const SHELL_PATH: &str = "/shell";
const SHELL_KEY: &str = "pfx-shell";

/// How many times each document's handler rendered.
static LEAF_RENDERS: AtomicU64 = AtomicU64::new(0);
static SHELL_RENDERS: AtomicU64 = AtomicU64::new(0);

/// A handler that renders `mount`'s island in the dogfood document and
/// counts each render in `renders`.
fn document(
    mount: LiveMount<DogfoodCounter>,
    renders: &'static AtomicU64,
) -> impl Fn(Request) -> std::pin::Pin<Box<dyn std::future::Future<Output = suprnova::Response> + Send>>
+ Send
+ Sync
+ 'static {
    move |request: Request| {
        let mount = mount.clone();
        Box::pin(async move {
            renders.fetch_add(1, Ordering::SeqCst);
            let result: Result<HttpResponse, String> = async {
                let mut document =
                    LiveDocument::from_request(&request).map_err(|error| error.to_string())?;
                let island = document
                    .mount(
                        &mount,
                        CanonicalValue::Object(std::collections::BTreeMap::new()),
                        MountFlags::empty(),
                    )
                    .await
                    .map_err(|error| error.to_string())?;
                let bootstrap = document
                    .bootstrap(LiveBootstrapOptions::esm())
                    .map_err(|error| error.to_string())?;
                document
                    .render(
                        ViewName::parse("live/dogfood-document.html")
                            .map_err(|error| error.to_string())?,
                        &DogfoodDocument {
                            bootstrap: bootstrap.html(),
                            island: island.html(),
                        },
                        DocumentResponseIntent::html(StatusCode::OK)
                            .map_err(|error| error.to_string())?,
                        AssetSet::empty(),
                    )
                    .map_err(|error| error.to_string())
            }
            .await;
            result.map_err(|error| HttpResponse::text(error).status(500))
        })
    }
}

struct Stitched {
    address: std::net::SocketAddr,
    _guard: TestContainerGuard,
    _tempdir: tempfile::TempDir,
}

/// Both documents cached under the stitched class, behind the
/// production-shaped global stack (session, CSRF, and the test sign-in),
/// served on a loopback socket whose peer is a trusted proxy.
async fn boot_stitched(app_url: &str) -> Stitched {
    support::install(app_url);
    suprnova::middleware::clear_global_middleware_for_test();
    let guard = TestContainer::fake();
    crate::live_dogfood_support::fixture();
    let tempdir = database().await;

    let leaf = LiveMount::<DogfoodCounter>::public_seed(LEAF_PATH, "counter", LEAF_KEY)
        .expect("declare the leaf's island");
    let shell = LiveMount::<DogfoodCounter>::identity_bound(SHELL_PATH, "counter", SHELL_KEY)
        .expect("declare the shell's island");
    let router: Router = Router::new()
        .get(LEAF_PATH, document(leaf.clone(), &LEAF_RENDERS))
        .get(SHELL_PATH, document(shell.clone(), &SHELL_RENDERS))
        .middleware(AuthMiddleware::new())
        .middleware(LiveTenantMiddleware::new(Arc::new(Tenantless)))
        .into();
    let policy = RenderCachePolicy::builder(RepresentationClass::PublicShellStitched)
        .freshness(FreshnessPolicy::new(200_000_000, 0, 0).expect("a freshness"))
        .layers(StorageLayers::l0_and_l1())
        .build()
        .expect("a stitched policy");
    let router = router
        .try_live_with(crate::live_dogfood_support::public_live_guard)
        .expect("install the Live routes")
        .try_live_mount(&leaf)
        .expect("register the leaf's island")
        .try_live_mount(&shell)
        .expect("register the shell's island")
        .try_render_cache(LEAF_PATH, GroupPolicy::from(policy.clone()))
        .expect("cache the leaf")
        .try_render_cache(SHELL_PATH, GroupPolicy::from(policy))
        .expect("cache the shell");

    let mut session = suprnova::SessionConfig::default();
    session.cookie_secure = false;
    suprnova::middleware::register_global_middleware(suprnova::SessionMiddleware::with_store(
        session,
        Arc::new(MemorySessionStore::default()),
    ));
    suprnova::middleware::register_global_middleware(suprnova::CsrfMiddleware::new());
    suprnova::middleware::register_global_middleware(LoginHeader);

    let mut config = RenderCacheConfig::from_env()
        .expect("a RenderCache configuration")
        .with_clock(Arc::new(TestClock(AtomicU64::new(1_000_000))) as Arc<dyn Clock>);
    config.enabled = true;
    config.l1 = L1Config::Disabled;
    config.coordinator = CoordinatorConfig::Local {
        lease_ms: 30_000,
        max_waiters: 128,
    };
    let router = RenderCache::install(router, config)
        .await
        .expect("install the RenderCache");
    suprnova::live::testing::prepare_live_router_for_test(&router)
        .expect("prepare the Live runtime");
    let address = support::serve(router, MiddlewareRegistry::from_global()).await;
    Stitched {
        address,
        _guard: guard,
        _tempdir: tempdir,
    }
}

/// `GET path` signed in as `user`, behind a trusted proxy that names `root`
/// when one is given.
async fn visit(stitched: &Stitched, path: &str, user: &str, root: Option<&str>) -> String {
    let mut headers = vec![("x-test-login", user)];
    if let Some(root) = root {
        headers.push(("x-forwarded-prefix", root));
    }
    let reply = support::get(stitched.address, path, &headers).await;
    assert_eq!(reply.status, 200, "{path} under {root:?}: {}", reply.body);
    reply.body
}

/// Whether a document names its Live action endpoint under `root`.
fn rendered_under(body: &str, root: &str) -> bool {
    body.contains(&format!("\"endpoint\":\"{root}/__live/action\""))
}

#[tokio::test]
async fn pfx_008_a_stitched_document_cached_under_one_root_is_never_assembled_under_another() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_008_a_stitched_document_cached_under_one_root_is_never_assembled_under_another",
    )
    .await
    {
        return;
    }
    let stitched = boot_stitched("http://localhost").await;
    let island = format!("data-suprnova-live-document-key=\"{SHELL_KEY}\"");
    let renders = || SHELL_RENDERS.load(Ordering::SeqCst);

    let leader_a = visit(&stitched, SHELL_PATH, "user-a", Some("/a")).await;
    assert_eq!(renders(), 1);
    assert!(rendered_under(&leader_a, "/a"), "{leader_a}");

    let hit_a = visit(&stitched, SHELL_PATH, "user-b", Some("/a")).await;
    assert_eq!(renders(), 1, "a second request under `/a` is assembled");
    assert!(rendered_under(&hit_a, "/a"), "{hit_a}");
    assert!(hit_a.contains(&island), "{hit_a}");

    let leader_b = visit(&stitched, SHELL_PATH, "user-b", Some("/b")).await;
    assert_eq!(renders(), 2, "`/a`'s entry was assembled under `/b`");
    assert!(rendered_under(&leader_b, "/b"), "{leader_b}");
    assert!(!leader_b.contains("/a/"), "{leader_b}");

    let hit_b = visit(&stitched, SHELL_PATH, "user-c", Some("/b")).await;
    assert_eq!(renders(), 2, "a second request under `/b` is assembled");
    assert!(rendered_under(&hit_b, "/b"), "{hit_b}");
    assert!(!hit_b.contains("/a/"), "{hit_b}");

    let host_root = visit(&stitched, SHELL_PATH, "user-c", None).await;
    assert_eq!(renders(), 3, "an entry under a prefix served the host root");
    assert!(rendered_under(&host_root, ""), "{host_root}");
    let stored = RenderCache::inspect_route_for_test(SHELL_PATH)
        .await
        .expect("the host root's entry is stored");
    assert_eq!(stored.kind, EntryKind::Composite);

    let again_a = visit(&stitched, SHELL_PATH, "user-d", Some("/a")).await;
    assert_eq!(renders(), 3, "`/a`'s entry is still its own");
    assert!(rendered_under(&again_a, "/a"), "{again_a}");
}

/// Publish the leaf under the current root and return what a document that
/// names it by reference stores: its key, its stored version, and its
/// length.
async fn publish_leaf(stitched: &Stitched) -> (RenderKey, u64, u32) {
    let leaf = visit(stitched, LEAF_PATH, "user-leaf", None).await;
    let key = RenderKey::from_base64url(&RenderCache::key_for_route_for_test(LEAF_PATH, &[], None))
        .expect("the leaf's key");
    let version = RenderCache::stored_fence_token_for_test(LEAF_PATH)
        .await
        .expect("the leaf is stored under the current root");
    let len = u32::try_from(leaf.len()).expect("the leaf fits in a u32");
    (key, version, len)
}

/// Publish the shell under the current root and make its stored graph name
/// `leaf`, as a document that includes another cached route does.
async fn nest(stitched: &Stitched, (key, version, assembled_len): (RenderKey, u64, u32)) {
    let before = SHELL_RENDERS.load(Ordering::SeqCst);
    visit(stitched, SHELL_PATH, "user-outer", None).await;
    assert_eq!(
        SHELL_RENDERS.load(Ordering::SeqCst),
        before + 1,
        "the shell renders under a root it has no entry for"
    );
    rewrite_composite_for_test(SHELL_PATH, |graph| {
        graph.segments.push(Segment::Nested {
            key,
            version,
            assembled_len,
            on_failure: SlotFailurePolicy::Omit,
        });
    })
    .await;
}

#[tokio::test]
async fn pfx_008_a_nested_entry_is_resolved_only_under_the_root_it_was_cached_under() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_008_a_nested_entry_is_resolved_only_under_the_root_it_was_cached_under",
    )
    .await
    {
        return;
    }
    // The root here is the path in `APP_URL` (PFX-002), so the test seams
    // that derive a key outside a request derive it under the same root as
    // the requests do.
    let stitched = boot_stitched("http://localhost/a").await;
    let leaf_marker = format!("data-suprnova-live-document-key=\"{LEAF_KEY}\"");
    let renders = || SHELL_RENDERS.load(Ordering::SeqCst);
    let leaf_a = publish_leaf(&stitched).await;
    let leaf_key_a = leaf_a.0.clone();
    nest(&stitched, leaf_a).await;

    let assembled_a = visit(&stitched, SHELL_PATH, "user-a", None).await;
    assert_eq!(renders(), 1, "`/a`'s shell is assembled");
    assert!(
        assembled_a.contains(&leaf_marker),
        "the nested leaf resolves under `/a`: {assembled_a}"
    );
    assert!(rendered_under(&assembled_a, "/a"), "{assembled_a}");
    assert!(!assembled_a.contains("/b/"), "{assembled_a}");

    // Under `/b` the leaf's key is another key, and nothing is stored under
    // it: the leaf `/a` cached is not `/b`'s.
    support::install("http://localhost/b");
    let leaf_key_b =
        RenderKey::from_base64url(&RenderCache::key_for_route_for_test(LEAF_PATH, &[], None))
            .expect("the leaf's key under `/b`");
    assert_ne!(leaf_key_a, leaf_key_b, "the nested key omits the root");
    assert_eq!(
        RenderCache::stored_fence_token_for_test(LEAF_PATH).await,
        None,
        "`/a`'s leaf is found under `/b`"
    );

    let leaf_b = publish_leaf(&stitched).await;
    assert_eq!(
        LEAF_RENDERS.load(Ordering::SeqCst),
        2,
        "`/b` renders its own leaf"
    );
    nest(&stitched, leaf_b).await;
    assert_eq!(renders(), 2, "`/a`'s shell was assembled under `/b`");
    let assembled_b = visit(&stitched, SHELL_PATH, "user-b", None).await;
    assert_eq!(renders(), 2, "`/b`'s shell is assembled");
    assert!(
        assembled_b.contains(&leaf_marker),
        "the nested leaf resolves under `/b`: {assembled_b}"
    );
    assert!(rendered_under(&assembled_b, "/b"), "{assembled_b}");
    assert!(!assembled_b.contains("/a/"), "{assembled_b}");

    // `/a` still assembles its own shell around its own leaf.
    support::install("http://localhost/a");
    let again_a = visit(&stitched, SHELL_PATH, "user-c", None).await;
    assert_eq!(renders(), 2, "`/a`'s shell is still its own");
    assert!(again_a.contains(&leaf_marker), "{again_a}");
    assert!(rendered_under(&again_a, "/a"), "{again_a}");
    assert!(!again_a.contains("/b/"), "{again_a}");
}
