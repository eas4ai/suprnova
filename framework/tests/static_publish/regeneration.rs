//! PUB-004 generation notifications wait for the transaction's durable outcome.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use sea_orm_migration::{MigrationTrait, MigratorTrait};
use suprnova::render_cache::DependencyIdentity;
use suprnova::render_cache::ledger::{advance_in_current_transaction, advance_via_handle};
use suprnova::static_publish::cdn::MemoryCdnDriver;
use suprnova::static_publish::{PublishConfig, Publisher, StaticPublication};
use suprnova::testing::TestDatabase;
use suprnova::{DB, FrameworkError, HttpResponse, InertiaProps, InertiaResponse, Request, Router};

async fn ten_independent_commits() -> Result<(), FrameworkError> {
    for index in 0..10 {
        DB::transaction(move |_| {
            Box::pin(async move {
                DB::statement(
                    "UPDATE publication_content SET title = ? WHERE id = 1",
                    vec![format!("after {index}").into()],
                )
                .await?;
                advance_in_current_transaction(&[DependencyIdentity::table("publication_content")])
                    .await
            })
        })
        .await?;
        let runtime = StaticPublication::current()?
            .ok_or_else(|| FrameworkError::internal("publication runtime missing"))?;
        assert_eq!(
            runtime.status().await.pending,
            0,
            "the complete request/job owns the notification batch"
        );
        tokio::task::yield_now().await;
    }
    Ok(())
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct TenCommits;

#[async_trait::async_trait]
impl suprnova::Job for TenCommits {
    fn job_name() -> &'static str {
        "pub004::TenIndependentCommits"
    }
    async fn handle(self) -> Result<(), FrameworkError> {
        ten_independent_commits().await
    }
}

fn job_envelope() -> suprnova::queue::Envelope {
    suprnova::queue::Envelope {
        schema_version: suprnova::queue::CURRENT_SCHEMA_VERSION,
        id: uuid::Uuid::new_v4(),
        job_name: <TenCommits as suprnova::Job>::job_name().into(),
        queue: None,
        payload: serde_json::to_value(TenCommits).expect("job payload"),
        dispatched_at: chrono::Utc::now(),
        available_at: chrono::Utc::now(),
        attempts: 0,
        max_tries: 1,
        backoff: suprnova::queue::BackoffSchedule::default(),
        timeout_secs: None,
        fail_on_timeout: false,
        idempotency_key: None,
        message_group: None,
        deduplication_id: None,
        unique_lock_owner: None,
        debounce_id: None,
        debounce_owner: None,
        batch_id: None,
        chain_remaining: Vec::new(),
        context: None,
    }
}

async fn real_worker_batch(job: bool) -> Result<(), Box<dyn std::error::Error>> {
    let _env = crate::env_lock::lock_env_async().await;
    let _snapshot = crate::env_snapshot::EnvSnapshot::capture(&[
        "APP_ENV",
        "CACHE_DRIVER",
        "QUEUE_DRIVER",
        "MAIL_DRIVER",
        "RATE_LIMIT_DRIVER",
    ]);
    for (key, value) in [
        ("APP_ENV", "testing"),
        ("CACHE_DRIVER", "memory"),
        ("QUEUE_DRIVER", "memory"),
        ("MAIL_DRIVER", "log"),
        ("RATE_LIMIT_DRIVER", "memory"),
    ] {
        crate::env_snapshot::set_env(key, Some(value));
    }
    let _container = suprnova::testing::TestContainer::fake();
    let _database = boot().await;
    let disk = opendal::Operator::new(opendal::services::Memory::default())?;
    let renders = Arc::new(AtomicUsize::new(0));
    let cdn = Arc::new(MemoryCdnDriver::default());
    let runtime = runtime(renders.clone(), disk, cdn.clone());
    let mutation_router = Router::new().post("/mutate", |_: Request| async {
        ten_independent_commits()
            .await
            .map_err(HttpResponse::from)?;
        Ok(HttpResponse::text("committed"))
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    drop(listener);
    let server = tokio::spawn(
        suprnova::Server::new(mutation_router)
            .host("127.0.0.1")
            .port(address.port())
            .run(),
    );
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if cdn
                .events()
                .expect("CDN events")
                .iter()
                .any(|event| event.starts_with("switch:"))
                && runtime.status().await.pending == 0
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    assert_eq!(renders.load(Ordering::SeqCst), 1);
    if job {
        suprnova::queue::worker::register_job::<TenCommits>();
        suprnova::queue::worker::run_through_middleware(job_envelope()).await?;
    } else {
        let (status, _, _) = crate::http_wire::request(address, "POST", "/mutate", &[]).await;
        assert_eq!(status, 200);
    }
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if cdn
                .events()
                .expect("CDN events")
                .iter()
                .filter(|event| event.starts_with("switch:"))
                .count()
                == 2
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        renders.load(Ordering::SeqCst),
        2,
        "ten independently committed writes produce one regeneration with the real boot worker running"
    );
    assert_eq!(
        cdn.events()?
            .iter()
            .filter(|event| event.starts_with("purge:"))
            .count(),
        2
    );
    assert_eq!(runtime.status().await.pending, 0);
    server.abort();
    let _ = server.await;
    Ok(())
}

#[tokio::test]
async fn pub_004_ten_independent_request_transactions_publish_once_with_worker_running()
-> Result<(), Box<dyn std::error::Error>> {
    if crate::own_process_async::delegate(
        module_path!(),
        "pub_004_ten_independent_request_transactions_publish_once_with_worker_running",
    )
    .await
    {
        return Ok(());
    }
    real_worker_batch(false).await
}

#[tokio::test]
async fn pub_004_ten_independent_job_transactions_publish_once_with_worker_running()
-> Result<(), Box<dyn std::error::Error>> {
    if crate::own_process_async::delegate(
        module_path!(),
        "pub_004_ten_independent_job_transactions_publish_once_with_worker_running",
    )
    .await
    {
        return Ok(());
    }
    real_worker_batch(true).await
}

#[derive(InertiaProps)]
#[inertia(cacheable)]
struct Props {
    #[inertia(public)]
    title: String,
}

struct Migrator;
#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(suprnova::render_cache::migration::Migration)]
    }
}

async fn boot() -> TestDatabase {
    suprnova::render_cache::mark_installed();
    let database = TestDatabase::fresh::<Migrator>()
        .await
        .expect("SQLite generation ledger");
    DB::statement(
        "CREATE TABLE publication_content (id INTEGER PRIMARY KEY, title TEXT NOT NULL)",
        vec![],
    )
    .await
    .expect("content table");
    DB::statement(
        "INSERT INTO publication_content (id, title) VALUES (1, 'before')",
        vec![],
    )
    .await
    .expect("initial content");
    database
}

fn runtime(
    renders: Arc<AtomicUsize>,
    disk: opendal::Operator,
    cdn: Arc<MemoryCdnDriver>,
) -> Arc<StaticPublication> {
    let router: Router = Router::new()
        .get("/public", move |request: Request| {
            let renders = renders.clone();
            async move {
                renders.fetch_add(1, Ordering::SeqCst);
                let row = DB::table("publication_content")
                    .first_or_fail()
                    .await
                    .map_err(HttpResponse::from)?;
                let title = row.get_string("title").map_err(HttpResponse::from)?;
                InertiaResponse::new("Public")
                    .with_config(
                        suprnova::InertiaConfig::new()
                            .version("v1")
                            .ssr_disabled()
                            .development(true),
                    )
                    .try_with("title", title)
                    .map_err(HttpResponse::from)?
                    .resolve(&request)
                    .await
                    .map_err(HttpResponse::from)
            }
        })
        .into();
    let router = router
        .public_page::<Props>("/public", "Public")
        .expect("typed public page");
    let publisher = Publisher::from_operator(disk, "v1", PublishConfig::default())
        .expect("publisher")
        .with_cdn(cdn);
    StaticPublication::install(&router, publisher).expect("runtime installation")
}

#[tokio::test]
#[serial_test::serial]
async fn pub_004_committed_writes_republish_both_variants_once_per_tag_and_purge() {
    let _container = suprnova::testing::TestContainer::fake();
    let _database = boot().await;
    let disk = opendal::Operator::new(opendal::services::Memory::default()).expect("memory");
    let renders = Arc::new(AtomicUsize::new(0));
    let cdn = Arc::new(MemoryCdnDriver::default());
    let runtime = runtime(renders.clone(), disk.clone(), cdn.clone());
    runtime.publish().await.expect("initial publication");
    assert_eq!(renders.load(Ordering::SeqCst), 1);
    let before = disk
        .read("_pub/manifest.json")
        .await
        .expect("initial manifest")
        .to_bytes();
    DB::transaction(|_transaction| {
        Box::pin(async {
            DB::statement(
                "UPDATE publication_content SET title = 'after' WHERE id = 1",
                vec![],
            )
            .await?;
            for _ in 0..10 {
                advance_in_current_transaction(&[DependencyIdentity::table("publication_content")])
                    .await?;
            }
            let runtime = StaticPublication::current()?
                .ok_or_else(|| FrameworkError::internal("runtime missing"))?;
            assert_eq!(
                runtime.status().await.pending,
                0,
                "precommit generation hints never schedule publication"
            );
            Ok::<_, FrameworkError>(())
        })
    })
    .await
    .expect("commit content and generation advances");
    assert_eq!(
        runtime.status().await.pending,
        2,
        "ten writes coalesce by table tag alongside the raw write broad tag"
    );
    let outcome = runtime
        .regenerate_pending()
        .await
        .expect("regenerate")
        .expect("dependent page republished");
    assert_eq!(renders.load(Ordering::SeqCst), 2);
    assert!(
        runtime
            .regenerate_pending()
            .await
            .expect("drained queue")
            .is_none()
    );
    let manifest_bytes = disk
        .read("_pub/manifest.json")
        .await
        .expect("new manifest")
        .to_bytes();
    assert_ne!(manifest_bytes, before);
    let manifest: suprnova::static_publish::Manifest =
        serde_json::from_slice(&manifest_bytes).expect("manifest");
    assert_eq!(manifest.live, outcome.version);
    let objects = &manifest.versions[&manifest.live].objects;
    for extension in [".html", ".json"] {
        let object = objects
            .iter()
            .find(|object| object.path.ends_with(extension))
            .expect("page representation");
        let bytes = disk
            .read(&object.path)
            .await
            .expect("published representation")
            .to_bytes();
        assert!(String::from_utf8_lossy(&bytes).contains("after"));
    }
    let events = cdn.events().expect("CDN operations");
    assert_eq!(
        events
            .iter()
            .filter(|event| event.starts_with("switch:"))
            .count(),
        2
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.starts_with("purge:"))
            .count(),
        2
    );
    assert!(events.last().expect("purge event").contains(&hex::encode(
        DependencyIdentity::table("publication_content").digest()
    )));
}

#[tokio::test]
#[serial_test::serial]
async fn pub_004_rollback_discards_generation_publication_callbacks() {
    let _container = suprnova::testing::TestContainer::fake();
    let _database = boot().await;
    let disk = opendal::Operator::new(opendal::services::Memory::default()).expect("memory");
    let renders = Arc::new(AtomicUsize::new(0));
    let runtime = runtime(
        renders.clone(),
        disk.clone(),
        Arc::new(MemoryCdnDriver::default()),
    );
    runtime.publish().await.expect("initial publication");
    let before = disk
        .read("_pub/manifest.json")
        .await
        .expect("manifest")
        .to_bytes();
    let result = DB::transaction(|_transaction| {
        Box::pin(async {
            DB::statement(
                "UPDATE publication_content SET title = 'rollback' WHERE id = 1",
                vec![],
            )
            .await?;
            advance_in_current_transaction(&[DependencyIdentity::table("publication_content")])
                .await?;
            Err::<(), _>(FrameworkError::internal("deliberate rollback"))
        })
    })
    .await;
    assert!(result.is_err());
    assert_eq!(runtime.status().await.pending, 0);
    assert!(
        runtime
            .regenerate_pending()
            .await
            .expect("nothing queued")
            .is_none()
    );
    assert_eq!(
        disk.read("_pub/manifest.json")
            .await
            .expect("manifest preserved")
            .to_bytes(),
        before
    );
    assert_eq!(renders.load(Ordering::SeqCst), 1);
}

#[tokio::test]
#[serial_test::serial]
async fn pub_004_explicit_handle_callbacks_wait_for_commit_and_savepoint_rollback() {
    let _container = suprnova::testing::TestContainer::fake();
    let _database = boot().await;
    let disk = opendal::Operator::new(opendal::services::Memory::default()).expect("memory");
    let runtime = runtime(
        Arc::new(AtomicUsize::new(0)),
        disk,
        Arc::new(MemoryCdnDriver::default()),
    );
    runtime.publish().await.expect("initial publication");
    let transaction = DB::begin_transaction().await.expect("manual transaction");
    transaction
        .savepoint("before_advance")
        .await
        .expect("savepoint");
    advance_via_handle(
        &transaction.handle(),
        &[DependencyIdentity::table("publication_content")],
    )
    .await
    .expect("explicit generation advance");
    assert_eq!(runtime.status().await.pending, 0);
    transaction
        .rollback_to("before_advance")
        .await
        .expect("savepoint rollback");
    transaction.commit().await.expect("outer commit");
    assert_eq!(
        runtime.status().await.pending,
        0,
        "savepoint rollback discards notification"
    );
    let transaction = DB::begin_transaction().await.expect("manual transaction");
    advance_via_handle(
        &transaction.handle(),
        &[DependencyIdentity::table("publication_content")],
    )
    .await
    .expect("explicit advance");
    transaction.commit().await.expect("commit");
    assert_eq!(
        runtime.status().await.pending,
        1,
        "manual commit invokes the handle registry"
    );
    runtime
        .regenerate_pending()
        .await
        .expect("drain committed notification");
}

#[tokio::test(start_paused = true)]
#[serial_test::serial]
async fn pub_004_scheduler_respects_configured_revalidate_interval() -> Result<(), FrameworkError> {
    let _container = suprnova::testing::TestContainer::fake();
    let clock = suprnova::clock::TestClock::freeze();
    let renders = Arc::new(AtomicUsize::new(0));
    let count = renders.clone();
    let router: Router = Router::new()
        .get("/public", move |request: Request| {
            let count = count.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                InertiaResponse::new("Public")
                    .with_config(
                        suprnova::InertiaConfig::new()
                            .version("v1")
                            .ssr_disabled()
                            .development(true),
                    )
                    .try_with("title", "Public")
                    .map_err(HttpResponse::from)?
                    .resolve(&request)
                    .await
                    .map_err(HttpResponse::from)
            }
        })
        .into();
    let router = router.public_page::<Props>("/public", "Public")?;
    suprnova::Storage::register_memory("pub-scheduler");
    let runtime = StaticPublication::install(
        &router,
        Publisher::new("pub-scheduler", "v1", PublishConfig::default())?,
    )?;
    let mut schedule = suprnova::Schedule::new();
    runtime.schedule(&mut schedule)?;
    assert!(
        runtime.schedule(&mut schedule).is_err(),
        "duplicate registration returns an error"
    );
    schedule
        .run_task("static:revalidate")
        .await
        .expect("registered task")?;
    clock.advance(chrono::Duration::seconds(61));
    tokio::time::advance(Duration::from_secs(30)).await;
    schedule
        .run_task("static:revalidate")
        .await
        .expect("registered task")?;
    assert_eq!(renders.load(Ordering::SeqCst), 1);
    clock.advance(chrono::Duration::seconds(61));
    tokio::time::advance(Duration::from_secs(31)).await;
    schedule
        .run_task("static:revalidate")
        .await
        .expect("registered task")?;
    assert_eq!(renders.load(Ordering::SeqCst), 2);
    Ok(())
}
