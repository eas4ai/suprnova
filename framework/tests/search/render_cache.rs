//! Search pages use the same dependency ledger as ordinary model reads.
use http_body_util::BodyExt;
use sea_orm::ConnectionTrait;
use sea_orm_migration::{MigrationTrait, MigratorTrait, SchemaManager};
use std::sync::{Arc, Once};
use suprnova::{
    DB, DbConnection, HttpResponse, MiddlewareRegistry, Model, Request, Response, Router, Search,
    SearchConfig, SearchDriver, SearchIndexJob, SearchRegistry, Searchable,
    queue::Job,
    render_cache::{
        FreshnessPolicy, QueryPolicy, RenderCache, RenderCacheConfig, RenderCachePolicy,
        RepresentationClass, registry::GroupPolicy, telemetry,
    },
    testing::{TestContainer, TestContainerGuard},
};

use super::common;

#[suprnova::model(table = "cached_search_a", timestamps = false, fillable = ["title"])]
#[derive(Searchable)]
#[searchable(index = "cached_a", settings = "cache_settings")]
struct Article {
    pub id: i64,
    #[searchable(text, facet)]
    pub title: String,
}

#[suprnova::model(table = "cached_search_b", timestamps = false, fillable = ["title"])]
#[derive(Searchable)]
#[searchable(index = "cached_b")]
struct OtherArticle {
    pub id: i64,
    #[searchable(text)]
    pub title: String,
}

tokio::task_local! { static SETTINGS: suprnova::IndexSettings; }
fn cache_settings() -> suprnova::IndexSettings {
    SETTINGS.try_with(Clone::clone).unwrap_or_default()
}

struct Harness {
    router: Arc<Router>,
    middleware: Arc<MiddlewareRegistry>,
    _guard: TestContainerGuard,
    _directory: tempfile::TempDir,
}

async fn boot() -> Harness {
    boot_with_connections(5).await
}

async fn boot_with_connections(connections: u32) -> Harness {
    static CRYPT: Once = Once::new();
    CRYPT.call_once(|| suprnova::Crypt::init(suprnova::EncryptionKey::generate()));
    suprnova::middleware::clear_global_middleware_for_test();
    RenderCache::uninstall_for_test();
    let guard = TestContainer::fake();
    let directory = tempfile::tempdir().unwrap();
    let config = suprnova::database::DatabaseConfig::builder()
        .url(format!(
            "sqlite://{}",
            directory.path().join("search.sqlite").display()
        ))
        .max_connections(connections)
        .min_connections(1)
        .logging(false)
        .build();
    let connection = DbConnection::connect(&config).await.unwrap();
    connection
        .inner()
        .execute_unprepared("PRAGMA journal_mode=WAL")
        .await
        .unwrap();
    super::derive::SearchMigrator::up(connection.inner(), None)
        .await
        .unwrap();
    let manager = SchemaManager::new(connection.inner());
    for migration in suprnova::search::migrations() {
        migration.up(&manager).await.unwrap();
    }
    suprnova::render_cache::migration::Migration
        .up(&manager)
        .await
        .unwrap();
    for table in ["cached_search_a", "cached_search_b"] {
        connection
            .inner()
            .execute_unprepared(&format!(
                "CREATE TABLE {table}(id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL)"
            ))
            .await
            .unwrap();
    }
    TestContainer::singleton(connection);
    let router: Router = Router::new().get("/results/{kind}", results).into();
    let policy = RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(60_000, 0, 0).unwrap())
        .query(QueryPolicy::declared(["page"]))
        .build()
        .unwrap();
    let router = router
        .try_render_cache("/results/{kind}", GroupPolicy::from(policy))
        .unwrap();
    let mut config = RenderCacheConfig::from_env().unwrap();
    config.enabled = true;
    config.l1 = suprnova::render_cache::L1Config::Disabled;
    let router = RenderCache::install(router, config).await.unwrap();
    Harness {
        router: Arc::new(router),
        middleware: Arc::new(MiddlewareRegistry::from_global()),
        _guard: guard,
        _directory: directory,
    }
}

async fn results(request: Request) -> Response {
    let kind = request.param("kind")?;
    let page = request
        .query_param("page")
        .and_then(|page| page.parse().ok())
        .unwrap_or(1);
    if kind == "race" {
        let engine = SearchRegistry::engine_for("cached_a")?;
        let generation = SearchRegistry::generation_for("cached_a").await?;
        let query = suprnova::EngineQuery {
            expanded: Default::default(),
            filter: None,
            sort: vec![],
            depth: 10,
            highlights: false,
            crop: Default::default(),
        };
        let result = engine
            .query(Article::descriptor(), &generation, &query)
            .await?;
        let keys = result
            .candidates
            .iter()
            .map(|hit| (hit.document.key.clone(), 0))
            .collect::<Vec<_>>();
        if !keys.is_empty() {
            // Spawn without the render's task-local transaction: the external
            // mutation and its ledger advance commit before window close.
            tokio::spawn(async move {
                engine
                    .remove(Article::descriptor(), &generation, &keys)
                    .await
            })
            .await
            .unwrap()?;
        }
        let titles = result
            .candidates
            .into_iter()
            .filter_map(|hit| match hit.document.attributes.get("title") {
                Some(suprnova::AttributeValue::Text(title)) => Some(title.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        return Ok(HttpResponse::text(titles.join(",")));
    }
    if kind == "suggest" {
        return Ok(HttpResponse::text(
            Search::suggest("cached_a", "ru", 5).await?.join(","),
        ));
    }
    if kind == "facet" {
        let facets = Article::search("rust")
            .facet_search("title", "rust")
            .await?;
        return Ok(HttpResponse::text(serde_json::to_string(&facets).unwrap()));
    }
    if kind == "federated" || kind == "federated-optout" {
        let builder = Search::federated(["cached_a", "cached_b"]).search("rust");
        let builder = if kind.ends_with("optout") {
            builder.without_render_cache()
        } else {
            builder
        };
        let hits = builder.get().await?.hits;
        return Ok(HttpResponse::text(serde_json::to_string(&hits).unwrap()));
    }
    if kind == "index" || kind == "index-optout" {
        let builder = Search::for_index("cached_a")?.search("rust");
        let builder = if kind.ends_with("optout") {
            builder.without_render_cache()
        } else {
            builder
        };
        return Ok(HttpResponse::text(
            serde_json::to_string(&builder.get().await?.hits).unwrap(),
        ));
    }
    let titles = if kind == "scoped" {
        super::derive::SearchNote::search("rust")
            .get()
            .await?
            .hits
            .into_iter()
            .map(|hit| hit.model.title)
            .collect::<Vec<_>>()
    } else if kind == "b" {
        OtherArticle::search("rust")
            .get()
            .await?
            .hits
            .into_iter()
            .map(|hit| hit.model.title)
            .collect::<Vec<_>>()
    } else {
        let builder = Article::search("rust");
        let builder = match kind {
            "optout" => builder.without_render_cache(),
            "viewer" => builder.for_viewer(suprnova::Viewer::Guest),
            _ => builder,
        };
        let result = if request.query_param("page").is_some() {
            builder.paginate(1, page).await?
        } else {
            builder.get().await?
        };
        if kind == "raw" {
            let _: i64 = DB::scalar("SELECT 1", []).await?;
        }
        result
            .hits
            .into_iter()
            .map(|hit| hit.model.title)
            .collect::<Vec<_>>()
    };
    Ok(HttpResponse::text(titles.join(",")))
}

async fn get(harness: &Harness, path: &str) -> (String, Vec<telemetry::RecordedLookup>) {
    telemetry::reset_recorded_lookups_for_test();
    let response = suprnova::handle_request(
        harness.router.clone(),
        harness.middleware.clone(),
        common::incoming_get_request(path, &[]).await,
    )
    .await;
    let status = response.status();
    let body = String::from_utf8(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert_eq!(status, 200, "{body}");
    (body, telemetry::recorded_lookups_for_test())
}

async fn article(title: &str) -> Article {
    suprnova::without_syncing_to_search(|| Article::create(suprnova::attrs! {title: title}))
        .await
        .unwrap()
}

async fn index(model: &Article) {
    SearchIndexJob {
        index: "cached_a".into(),
        key: model.id.to_string(),
        source_version: 0,
        op: suprnova::IndexOperation::Index,
        generation: None,
    }
    .handle()
    .await
    .unwrap();
}

fn hit(lookups: &[telemetry::RecordedLookup]) -> bool {
    lookups
        .iter()
        .any(|lookup| matches!(lookup.outcome, "l0" | "l1" | "conditional" | "stale"))
}

async fn cache_and_invalidate(driver: SearchDriver) {
    SearchConfig::scope(
        SearchConfig {
            driver,
            queue: false,
            ..Default::default()
        },
        SearchRegistry::isolated(async {
            let harness = boot().await;
            suprnova::search::generations::create_index("cached_a")
                .await
                .unwrap();
            let first = article("rust first").await;
            let second = article("rust second").await;
            index(&first).await;
            let (body, _) = get(&harness, "/results/a").await;
            assert_eq!(body, "rust first");
            assert!(
                hit(&get(&harness, "/results/a").await.1),
                "search page must be stored"
            );
            index(&second).await;
            let (_, lookups) = get(&harness, "/results/a").await;
            assert!(!hit(&lookups), "index write must invalidate the page");
            assert!(get(&harness, "/results/a").await.0.contains("rust second"));
            assert!(hit(&get(&harness, "/results/a").await.1));
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
#[serial_test::serial]
async fn search_page_tracks_collection_index() {
    cache_and_invalidate(SearchDriver::Collection).await;
}

#[tokio::test]
#[serial_test::serial]
async fn search_page_tracks_sqlite_index() {
    cache_and_invalidate(SearchDriver::Database).await;
}

#[derive(Clone, Copy)]
enum Case {
    Remove,
    Generation,
    Settings,
    Declines,
    Pages,
    Independent,
    ReadSurfaces,
    AtomicFailure,
    Bulk,
    ExternalRollback,
    ExternalSavepoint,
    Maintenance,
    ExternalReadRace,
    NamedAdvance,
}

async fn scenario(driver: SearchDriver, case: Case) {
    SearchConfig::scope(SearchConfig { driver, queue: false, ..Default::default() },
        SearchRegistry::isolated(async {
            let harness = boot().await;
            for name in ["cached_a", "cached_b", "test_search_notes"] {
                suprnova::search::generations::create_index(name).await.unwrap();
            }
            let first = article("rust first").await;
            let second = article("rust second").await;
            let other = suprnova::without_syncing_to_search(|| OtherArticle::create(suprnova::attrs! {title: "rust other"})).await.unwrap();
            index(&first).await;
            let engine = SearchRegistry::engine_for("cached_a").unwrap();
            let generation = SearchRegistry::generation_for("cached_a").await.unwrap();
            let descriptor = Article::descriptor();
            assert_eq!(get(&harness, "/results/a").await.0, "rust first");
            assert!(hit(&get(&harness, "/results/a").await.1), "prime a stored search page");
            match case {
                Case::Remove => {
                    suprnova::unsearchable::<Article>(&[first.id.to_string()]).await.unwrap();
                    let (body, lookups) = get(&harness, "/results/a").await;
                    assert!(!hit(&lookups), "removal must invalidate");
                    assert_eq!(body, "");
                    assert!(hit(&get(&harness, "/results/a").await.1));
                }
                Case::Generation => {
                    suprnova::search::generations::reindex("cached_a").await.unwrap();
                    assert_ne!(engine.current_generation("cached_a").await.unwrap(), Some(generation));
                    let (body, lookups) = get(&harness, "/results/a").await;
                    assert!(!hit(&lookups), "generation publication must invalidate");
                    assert!(body.contains("rust second"));
                    assert!(hit(&get(&harness, "/results/a").await.1));
                }
                Case::Settings => {
                    index(&second).await;
                    assert!(get(&harness, "/results/a").await.0.contains("rust second"));
                    assert!(hit(&get(&harness, "/results/a").await.1));
                    let mut settings = cache_settings();
                    settings.query.hits_cap = 1;
                    SETTINGS.scope(settings, suprnova::search::generations::sync_settings("cached_a")).await.unwrap();
                    assert_eq!(engine.current_generation("cached_a").await.unwrap(), Some(generation));
                    let (body, lookups) = get(&harness, "/results/a").await;
                    assert!(!hit(&lookups), "query-time settings sync must invalidate");
                    assert_eq!(body.split(',').count(), 1);
                    assert!(hit(&get(&harness, "/results/a").await.1));
                }
                Case::Declines => {
                    for (kind, reason) in [("scoped", "viewer_scoped_search"), ("viewer", "viewer_scoped_search"),
                        ("optout", "search_opt_out"), ("index-optout", "search_opt_out"),
                        ("federated-optout", "search_opt_out"), ("raw", "observation_overflowed")] {
                        for _ in 0..2 {
                            let (_, lookups) = get(&harness, &format!("/results/{kind}")).await;
                            assert!(!hit(&lookups), "{kind} must never be a hit");
                            assert!(lookups.iter().any(|lookup| lookup.reason == Some(reason)), "{kind} must decline with {reason}");
                        }
                    }
                    // Enable analytics while an analytics-off entry still exists.
                    let on = SearchConfig { driver, queue: false, analytics: true, ..Default::default() };
                    SearchConfig::scope(on, async {
                        for kind in ["a", "index", "federated", "facet", "suggest"] {
                            for _ in 0..2 {
                                let (_, lookups) = get(&harness, &format!("/results/{kind}")).await;
                                assert!(!hit(&lookups), "analytics-on {kind} must never replay a stored entry");
                                assert!(lookups.iter().any(|lookup| lookup.reason == Some("analytics_search")));
                            }
                        }
                        let count: i64 = DB::scalar("SELECT COUNT(*) FROM search_queries", []).await.unwrap();
                        assert_eq!(count, 4, "model and runtime-index searches record every request");
                    }).await.unwrap();
                }
                Case::Pages => {
                    index(&second).await;
                    let one = get(&harness, "/results/a?page=1").await.0;
                    let two = get(&harness, "/results/a?page=2").await.0;
                    assert_ne!(one, two);
                    for (path, body) in [("/results/a?page=1", one), ("/results/a?page=2", two)] {
                        let (again, lookups) = get(&harness, path).await;
                        assert!(hit(&lookups), "each page is independently stored");
                        assert_eq!(again, body, "a hit replays only that page");
                    }
                }
                Case::Independent => {
                    SearchIndexJob { index: "cached_b".into(), key: other.id.to_string(), source_version: 0,
                        op: suprnova::IndexOperation::Index, generation: None }.handle().await.unwrap();
                    assert_eq!(get(&harness, "/results/b").await.0, "rust other");
                    assert!(hit(&get(&harness, "/results/b").await.1));
                    index(&second).await;
                    let (body, lookups) = get(&harness, "/results/b").await;
                    assert!(hit(&lookups), "index A must not evict a page depending only on B");
                    assert_eq!(body, "rust other");
                    assert!(!hit(&get(&harness, "/results/a").await.1));
                }
                Case::ReadSurfaces => {
                    for kind in ["index", "federated", "facet", "suggest"] {
                        let path = format!("/results/{kind}");
                        get(&harness, &path).await;
                        assert!(hit(&get(&harness, &path).await.1), "{kind} reads must be storable");
                    }
                    index(&second).await;
                    for kind in ["index", "federated", "facet", "suggest"] {
                        let path = format!("/results/{kind}");
                        assert!(!hit(&get(&harness, &path).await.1), "{kind} must observe the index");
                        assert!(hit(&get(&harness, &path).await.1));
                    }
                    // Hydration remains an ordinary model dependency.
                    suprnova::without_syncing_to_search(|| first.update(suprnova::attrs! {title: "rust changed"})).await.unwrap();
                    let (body, lookups) = get(&harness, "/results/a").await;
                    assert!(!hit(&lookups));
                    assert!(body.contains("rust changed"));
                }
                Case::AtomicFailure => {
                    if driver == SearchDriver::Database {
                        let connection = DB::connection().unwrap();
                        connection.inner().execute_unprepared("ALTER TABLE suprnova_render_generation_log RENAME TO held_generation_log").await.unwrap();
                        let removed = engine.remove(descriptor, &generation, &[(first.id.to_string(), i64::MAX)]).await;
                        connection.inner().execute_unprepared("ALTER TABLE held_generation_log RENAME TO suprnova_render_generation_log").await.unwrap();
                        assert!(removed.is_err(), "a failed ledger write must fail the document transaction");
                        assert_eq!(Article::search("rust").get().await.unwrap().hits.len(), 1, "the document removal rolls back with its advance");
                        engine.write_settings(descriptor, &generation, &descriptor.index_settings()).await.unwrap();
                        assert!(!hit(&get(&harness, "/results/a").await.1));
                        assert!(hit(&get(&harness, "/results/a").await.1), "a later confirmed write repairs the failed advance and resumes serving");
                    }
                }
                Case::NamedAdvance => {
                    let config = suprnova::database::DatabaseConfig::builder()
                        .url("sqlite::memory:").max_connections(1).min_connections(1).logging(false).build();
                    let auxiliary = DbConnection::connect(&config).await.unwrap();
                    // The auxiliary has no ledger: its presence must not redirect
                    // an external engine's generation authority off primary.
                    suprnova::ConnectionRegistry::register_existing("search_cache_aux", auxiliary).await.unwrap();
                    DB::with_default_connection("search_cache_aux", engine.write_settings(descriptor, &generation, &descriptor.index_settings()))
                        .await.unwrap();
                    assert!(!hit(&get(&harness, "/results/a").await.1), "named execution must advance primary authority");
                    assert!(hit(&get(&harness, "/results/a").await.1));
                }
                Case::ExternalReadRace => {
                    assert_eq!(get(&harness, "/results/race").await.0, "rust first");
                    let (body, lookups) = get(&harness, "/results/race").await;
                    assert!(!hit(&lookups), "a write between external retrieval and the first SQL read must prevent stale publication");
                    assert!(body.is_empty());
                    assert!(hit(&get(&harness, "/results/race").await.1));
                }
                Case::ExternalRollback | Case::ExternalSavepoint => {
                    let engine = engine.clone();
                    let generation = generation.clone();
                    let key = first.id.to_string();
                    let result: Result<(), suprnova::FrameworkError> = DB::transaction(move |tx| Box::pin(async move {
                        if matches!(case, Case::ExternalSavepoint) { tx.savepoint("search_write").await?; }
                        engine.remove(Article::descriptor(), &generation, &[(key, 0)]).await?;
                        if matches!(case, Case::ExternalSavepoint) {
                            tx.rollback_to("search_write").await?;
                            Ok(())
                        } else { Err(suprnova::FrameworkError::param("roll back caller")) }
                    })).await;
                    assert_eq!(result.is_err(), matches!(case, Case::ExternalRollback));
                    let (body, lookups) = get(&harness, "/results/a").await;
                    assert!(!hit(&lookups), "a caller rollback cannot erase an external mutation's invalidation");
                    assert!(body.is_empty(), "collection removal survives SQL rollback");
                    assert!(hit(&get(&harness, "/results/a").await.1), "repair restores normal caching");
                }
                Case::Maintenance => {
                    use suprnova::render_cache::{DependencyIdentity, GenerationLedger};
                    let dependency = DependencyIdentity::table("search:cached_a");
                    let ledger = suprnova::render_cache::ledger::SqlGenerationLedger::new();
                    let before = ledger.current(&[dependency.digest()]).await.unwrap().get(&dependency);
                    let hidden = suprnova::GenerationId::new("hidden").unwrap();
                    engine.create_generation(descriptor, &descriptor.index_settings(), &hidden).await.unwrap();
                    assert!(!hit(&get(&harness, "/results/a").await.1), "generation creation invalidates");
                    assert!(hit(&get(&harness, "/results/a").await.1));
                    engine.drop_generation(descriptor, &hidden).await.unwrap();
                    assert!(!hit(&get(&harness, "/results/a").await.1), "generation drop invalidates");
                    assert!(hit(&get(&harness, "/results/a").await.1));
                    engine.remove(descriptor, &generation, &[(first.id.to_string(), 0)]).await.unwrap();
                    get(&harness, "/results/a").await;
                    assert!(hit(&get(&harness, "/results/a").await.1));
                    if driver == SearchDriver::Database {
                        assert_eq!(suprnova::search::DatabaseSearchEngine::new().prune_tombstones(0).await.unwrap(), 1);
                        assert!(!hit(&get(&harness, "/results/a").await.1), "tombstone pruning advances only affected indexes");
                    }
                    let after = ledger.current(&[dependency.digest()]).await.unwrap().get(&dependency);
                    assert_ne!(before, after);
                    suprnova::search::generations::delete_index("cached_a").await.unwrap();
                    let retired = ledger.current(&[dependency.digest()]).await.unwrap().get(&dependency);
                    assert_ne!(after, retired, "retirement advances before deleting generation metadata");
                }
                Case::Bulk => {
                    Article::make_all_searchable().await.unwrap();
                    let (body, lookups) = get(&harness, "/results/a").await;
                    assert!(!hit(&lookups));
                    assert!(body.contains("rust second"));
                    assert!(hit(&get(&harness, "/results/a").await.1));
                    Article::remove_all_from_search().await.unwrap();
                    let (body, lookups) = get(&harness, "/results/a").await;
                    assert!(!hit(&lookups));
                    assert!(body.is_empty());
                }
            }
        })).await.unwrap();
}

macro_rules! cases {
    ($($name:ident, $driver:ident, $case:ident;)*) => { $(
        #[tokio::test]
        #[serial_test::serial]
        async fn $name() { scenario(SearchDriver::$driver, Case::$case).await; }
    )* };
}
cases! {
    removal_collection, Collection, Remove;
    removal_sqlite, Database, Remove;
    generation_collection, Collection, Generation;
    generation_sqlite, Database, Generation;
    settings_collection, Collection, Settings;
    settings_sqlite, Database, Settings;
    declines_collection, Collection, Declines;
    declines_sqlite, Database, Declines;
    pages_collection, Collection, Pages;
    pages_sqlite, Database, Pages;
    independent_indexes_collection, Collection, Independent;
    independent_indexes_sqlite, Database, Independent;
    read_surfaces_collection, Collection, ReadSurfaces;
    read_surfaces_sqlite, Database, ReadSurfaces;
    database_write_and_advance_roll_back_together, Database, AtomicFailure;
    bulk_collection, Collection, Bulk;
    bulk_sqlite, Database, Bulk;
    external_write_survives_caller_rollback, Collection, ExternalRollback;
    external_write_survives_savepoint_rollback, Collection, ExternalSavepoint;
    maintenance_collection, Collection, Maintenance;
    maintenance_sqlite, Database, Maintenance;
    external_read_before_first_sql_keeps_the_earlier_generation, Collection, ExternalReadRace;
    named_external_write_advances_primary_authority, Collection, NamedAdvance;
}

#[tokio::test]
#[serial_test::serial]
#[ignore = "requires Qdrant at QDRANT_TEST_URL"]
async fn search_page_tracks_single_connection_qdrant() {
    use futures::FutureExt;
    use suprnova::{
        GenerationId,
        search::{QdrantSearchEngine, SearchEngine},
    };
    SearchConfig::scope(
        SearchConfig {
            driver: SearchDriver::Qdrant,
            queue: false,
            ..Default::default()
        },
        SearchRegistry::isolated(async {
            let harness = boot_with_connections(1).await;
            let engine = Arc::new(
                QdrantSearchEngine::from_url(&std::env::var("QDRANT_TEST_URL").unwrap(), None, 10)
                    .unwrap(),
            );
            SearchRegistry::register_engine("qdrant", engine.clone()).unwrap();
            let generation =
                GenerationId::new(format!("rc_{}", uuid::Uuid::new_v4().simple())).unwrap();
            let descriptor = Article::descriptor();
            engine
                .create_generation(descriptor, &descriptor.index_settings(), &generation)
                .await
                .unwrap();
            engine
                .switch_generation(descriptor, None, generation.clone())
                .await
                .unwrap();
            let result = std::panic::AssertUnwindSafe(async {
                let model = article("rust qdrant").await;
                index(&model).await;
                assert_eq!(get(&harness, "/results/a").await.0, "rust qdrant");
                assert!(
                    hit(&get(&harness, "/results/a").await.1),
                    "Qdrant searches must store without borrowing a second snapshot connection"
                );
                engine
                    .remove(descriptor, &generation, &[(model.id.to_string(), 0)])
                    .await
                    .unwrap();
                let (body, lookups) = get(&harness, "/results/a").await;
                assert!(!hit(&lookups));
                assert!(body.is_empty());
                assert!(hit(&get(&harness, "/results/a").await.1));
            })
            .catch_unwind()
            .await;
            engine
                .client()
                .unwrap()
                .delete_collection(format!("search_cached_a_{}", generation.as_str()))
                .await
                .unwrap();
            if let Err(error) = result {
                std::panic::resume_unwind(error);
            }
        }),
    )
    .await
    .unwrap();
}
