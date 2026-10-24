use super::support::{Harness, debug_mode, exchange, page};
use std::sync::Arc;
use std::time::Duration;
use suprnova::{
    App, DebugBar, HttpResponse, Middleware, MiddlewareRegistry, Next, Request, Response, Router,
    async_trait,
};

struct SessionAndLocale;
#[async_trait]
impl Middleware for SessionAndLocale {
    async fn handle(&self, request: Request, next: Next) -> Response {
        let slot = suprnova::session::new_session_slot_for_test();
        suprnova::session::session_scope_for_test(slot, async {
            suprnova::session::session_mut(|session| session.user_id = Some("principal-42".into()));
            suprnova::localization::scope_locale(
                suprnova::Locale::parse("fr").expect("locale"),
                next(request),
            )
            .await
        })
        .await
    }
}

#[tokio::test]
async fn dbg_001_records_session_principal_and_active_locale_without_persisting_session_data() {
    let _mode = debug_mode(true, &[]).await;
    let app = Harness::with_middleware(
        Router::new().get("/", page).into(),
        MiddlewareRegistry::new().append(SessionAndLocale),
    );
    let reply = app.get("/", &[]).await;
    let record = app.record(&reply).await;
    assert_eq!(record["request"]["session_id"], "\"test_session\"");
    assert_eq!(record["request"]["principal"], "\"principal-42\"");
    assert!(
        record["request"]["locale"]
            .as_str()
            .expect("locale Debug")
            .contains("fr")
    );
    assert!(!record.to_string().contains("test_csrf_token"));
}

#[tokio::test]
async fn dbg_001_samples_peak_rss_while_the_request_owns_memory() {
    let _mode = debug_mode(true, &[]).await;
    let app = Harness::new(
        Router::new()
            .get("/memory", |_: Request| async {
                let allocation = std::hint::black_box(vec![42_u8; 32 * 1024 * 1024]);
                tokio::time::sleep(Duration::from_millis(40)).await;
                std::hint::black_box(&allocation);
                drop(allocation);
                Ok(HttpResponse::text("released"))
            })
            .into(),
    );
    let reply = app.get("/memory", &[]).await;
    let record = app.record(&reply).await;
    assert!(record["peak_memory_bytes"].as_u64().expect("RSS on Linux") >= 16 * 1024 * 1024);
}

#[tokio::test]
async fn dbg_001_live_mounts_and_actions_are_in_their_respective_request_records() {
    let _mode = debug_mode(true, &[]).await;
    let _container = suprnova::testing::TestContainer::fake();
    use super::live_dogfood_support as live;
    live::fixture();
    let router = live::build_public_router();
    suprnova::live::testing::prepare_live_router_for_test(&router).expect("prepare Live runtime");
    let registry = live::production_middleware();
    let registry = registry
        .global_middleware()
        .iter()
        .fold(MiddlewareRegistry::new(), |registry, middleware| {
            registry.append_boxed(Arc::clone(middleware))
        });
    let app = Harness::with_middleware(router, registry);
    let mounted = app
        .get(live::DOCUMENT_PATH, &[("accept", "text/html")])
        .await;
    assert_eq!(mounted.status, 200, "{}", mounted.body);
    let record = app.record(&mounted).await;
    assert!(record["panels"]["live"].to_string().contains("render"));
    assert!(
        record["panels"]["live"]
            .to_string()
            .contains("tests.dogfood-counter"),
        "{}",
        record["panels"]["live"]
    );
    let cookie = mounted
        .headers
        .iter()
        .filter(|(name, _)| name == "set-cookie")
        .map(|(_, value)| value.split(';').next().unwrap())
        .find(|cookie| !cookie.starts_with("XSRF-TOKEN="))
        .expect("session cookie");
    let action = live::action_request(live::ActionRequest {
        snapshot: live::decoded_snapshot(mounted.body.as_bytes()),
        cookie,
        fetch_site: Some("same-origin"),
        login: None,
        idempotency_key: "ICEiIyQlJicoKSorLC0uLw",
    });
    let action = exchange(Arc::clone(&app.router), Arc::clone(&app.registry), action).await;
    assert_eq!(action.status, 200);
    let record = app.record(&action).await;
    assert!(record["panels"]["live"].to_string().contains("increment"));
    assert!(record["panels"]["live"].to_string().contains("render"));
    assert!(!action.body.contains("suprnova-debugbar"));
}

struct CacheMigrator;
#[async_trait]
impl sea_orm_migration::MigratorTrait for CacheMigrator {
    fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
        vec![Box::new(suprnova::render_cache::migration::Migration)]
    }
}

#[tokio::test]
async fn dbg_001_render_cache_reports_miss_store_and_hit_without_caching_a_toolbar() {
    let _mode = debug_mode(true, &[]).await;
    use suprnova::render_cache::{
        CoordinatorConfig, FreshnessPolicy, HintsConfig, L1Config, RenderCache, RenderCacheConfig,
        RenderCachePolicy, RepresentationClass,
    };
    App::init();
    suprnova::middleware::clear_global_middleware_for_test();
    suprnova::testing::install_test_encryption_key();
    let _db = suprnova::testing::TestDatabase::fresh::<CacheMigrator>()
        .await
        .expect("cache schema");
    suprnova::global_middleware!(DebugBar::middleware().expect("debug bar"));
    let policy = RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(60_000, 0, 0).expect("freshness"))
        .build()
        .expect("policy");
    let router: Router = Router::new().get("/cached", page).into();
    let router = router
        .try_render_cache("/cached", policy)
        .expect("declare cache");
    let mut config = RenderCacheConfig::from_env().expect("config");
    config.enabled = true;
    config.l1 = L1Config::Disabled;
    config.hints = HintsConfig::Disabled;
    config.coordinator = CoordinatorConfig::Local {
        lease_ms: 30_000,
        max_waiters: 128,
    };
    let router = RenderCache::install(router, config)
        .await
        .expect("install cache");
    let app = Harness {
        router: Arc::new(router),
        registry: Arc::new(MiddlewareRegistry::from_global()),
    };
    let first = app.get("/cached", &[("accept", "text/html")]).await;
    assert_eq!(first.status, 200);
    let recorded = app.record(&first).await;
    let cache = recorded["panels"]["render_cache"].to_string();
    assert!(cache.contains("miss"));
    assert!(cache.contains("store"));
    let second = app.get("/cached", &[("accept", "text/html")]).await;
    let recorded = app.record(&second).await;
    assert!(
        recorded["panels"]["render_cache"]
            .to_string()
            .contains("l0")
    );
    assert_ne!(first.id(), second.id());
    assert!(!second.body.contains(first.id()));
    assert_eq!(second.body.matches("id=\"suprnova-debugbar\"").count(), 1);
}
