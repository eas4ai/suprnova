//! Persistent generation states are the publication boundary for indexing work.
use super::database_support::{Migrator, descriptor, engine};
use suprnova::{DB, GenerationId, Model, SearchEngine, testing::TestDatabase};

#[tokio::test]
async fn generations_build_hidden_and_switch_current_and_retired_atomically() {
    let _db = TestDatabase::fresh::<Migrator>().await.unwrap();
    let descriptor = descriptor("generation_states");
    let engine = engine();
    let old = GenerationId::new("old").unwrap();
    let new = GenerationId::new("new").unwrap();
    engine
        .create_generation(&descriptor, &descriptor.index_settings(), &old)
        .await
        .unwrap();
    assert_eq!(
        DB::scalar::<String>(
            "SELECT state FROM search_generations WHERE generation = ?",
            vec!["old".into()]
        )
        .await
        .unwrap(),
        "building"
    );
    assert_eq!(
        engine.current_generation(descriptor.name).await.unwrap(),
        None
    );
    engine
        .switch_generation(&descriptor, None, old.clone())
        .await
        .unwrap();
    engine
        .create_generation(&descriptor, &descriptor.index_settings(), &new)
        .await
        .unwrap();
    assert_eq!(
        engine.current_generation(descriptor.name).await.unwrap(),
        Some(old.clone())
    );
    engine
        .switch_generation(&descriptor, Some(old), new.clone())
        .await
        .unwrap();
    assert_eq!(
        engine.current_generation(descriptor.name).await.unwrap(),
        Some(new)
    );
    assert_eq!(
        DB::scalar::<String>(
            "SELECT state FROM search_generations WHERE generation = ?",
            vec!["old".into()]
        )
        .await
        .unwrap(),
        "retired"
    );
}

#[tokio::test]
async fn expired_generation_creation_recovers_partial_storage_and_releases_its_lease() {
    let _db = TestDatabase::fresh::<Migrator>().await.unwrap();
    let descriptor = descriptor("creation_recovery");
    let engine = engine();
    let generation = GenerationId::new("partial").unwrap();
    let settings = descriptor.index_settings();
    engine
        .create_generation(&descriptor, &settings, &generation)
        .await
        .unwrap();
    DB::connection().unwrap().inner().execute_raw(Statement::from_sql_and_values(
        DbBackend::Sqlite,
        "UPDATE search_generations SET state='creating',creation_owner='dead-creator',creation_expires_at=? WHERE index_name=? AND generation=?",
        vec![(chrono::Utc::now().timestamp()-1).into(), descriptor.name.into(), generation.as_str().into()],
    )).await.unwrap();
    DB::unprepared("DROP TABLE search_creation_recovery_partial_vocab")
        .await
        .unwrap();

    engine
        .create_generation(&descriptor, &settings, &generation)
        .await
        .unwrap();
    assert_eq!(
        engine.current_generation(descriptor.name).await.unwrap(),
        None
    );
    assert_eq!(DB::scalar::<i64>(
        "SELECT COUNT(*) FROM search_generations WHERE index_name=? AND generation=? AND state='building' AND creation_owner IS NULL AND creation_expires_at IS NULL",
        vec![descriptor.name.into(), generation.as_str().into()],
    ).await.unwrap(), 1);
    let document =
        super::database_support::document(&descriptor, "1", "Recovered", "Complete storage");
    engine
        .index(&descriptor, &generation, &[document])
        .await
        .unwrap();
    assert_eq!(
        engine
            .query(
                &descriptor,
                &generation,
                &super::database_support::query("Recovered")
            )
            .await
            .unwrap()
            .total
            .value(),
        1
    );
}

#[tokio::test]
async fn creation_recovery_refuses_live_owners_and_different_settings() {
    let _db = TestDatabase::fresh::<Migrator>().await.unwrap();
    let descriptor = descriptor("creation_owner");
    let engine = engine();
    let generation = GenerationId::new("owned").unwrap();
    let settings = descriptor.index_settings();
    engine
        .create_generation(&descriptor, &settings, &generation)
        .await
        .unwrap();
    DB::connection().unwrap().inner().execute_raw(Statement::from_sql_and_values(
        DbBackend::Sqlite,
        "UPDATE search_generations SET state='creating',creation_owner='live-creator',creation_expires_at=? WHERE index_name=? AND generation=?",
        vec![(chrono::Utc::now().timestamp()+60).into(), descriptor.name.into(), generation.as_str().into()],
    )).await.unwrap();
    let error = engine
        .create_generation(&descriptor, &settings, &generation)
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("creation lease is still active"),
        "{error}"
    );
    assert_eq!(
        DB::scalar::<String>(
            "SELECT creation_owner FROM search_generations WHERE index_name=? AND generation=?",
            vec![descriptor.name.into(), generation.as_str().into()]
        )
        .await
        .unwrap(),
        "live-creator"
    );

    DB::connection().unwrap().inner().execute_raw(Statement::from_sql_and_values(
        DbBackend::Sqlite,
        "UPDATE search_generations SET creation_expires_at=? WHERE index_name=? AND generation=?",
        vec![(chrono::Utc::now().timestamp()-1).into(), descriptor.name.into(), generation.as_str().into()],
    )).await.unwrap();
    let mut different = settings;
    different.index_time.embed_template = "Different {text}".into();
    let error = engine
        .create_generation(&descriptor, &different, &generation)
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("different state or settings"),
        "{error}"
    );
    assert_eq!(
        DB::scalar::<String>(
            "SELECT creation_owner FROM search_generations WHERE index_name=? AND generation=?",
            vec![descriptor.name.into(), generation.as_str().into()]
        )
        .await
        .unwrap(),
        "live-creator"
    );
}

use super::commands::{CommandNote, IMPORT_BARRIER, ImportBarrier, database};
use sea_orm::{ConnectionTrait, DbBackend, Statement, TransactionTrait};
use std::{sync::Arc, time::Duration};
use suprnova::Job;
use suprnova::{
    Queue, Searchable, console,
    search::{
        DatabaseSearchEngine, IndexOperation, SearchConfig, SearchIndexJob, SearchRegistry,
        generations,
    },
};

async fn pending_count() -> i64 {
    DB::scalar::<i64>(
        "SELECT COUNT(*) FROM search_jobs_pending WHERE index_name = ? AND completed = FALSE",
        vec!["search_command_notes".into()],
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn live_update_and_delete_during_hidden_import_survive_publication() {
    let _db = database().await;
    SearchConfig::scope(
        SearchConfig {
            queue: false,
            chunk: 2,
            ..Default::default()
        },
        SearchRegistry::isolated(async {
            SearchRegistry::register_engine("database", Arc::new(DatabaseSearchEngine::new()))
                .unwrap();
            generations::create_index("search_command_notes")
                .await
                .unwrap();
            let updated = CommandNote::create(suprnova::attrs! {title: "Before update"})
                .await
                .unwrap();
            let removed = CommandNote::create(suprnova::attrs! {title: "Before delete"})
                .await
                .unwrap();
            let old = SearchRegistry::generation_for("search_command_notes")
                .await
                .unwrap();
            let barrier = ImportBarrier::new();
            IMPORT_BARRIER
                .scope(barrier.clone(), async {
                    let rebuild = generations::reindex("search_command_notes");
                    let during = async {
                        barrier.entered.notified().await;
                        assert_eq!(
                            SearchRegistry::generation_for("search_command_notes")
                                .await
                                .unwrap(),
                            old
                        );
                        assert_eq!(
                            CommandNote::search("Before")
                                .get()
                                .await
                                .unwrap()
                                .total
                                .value(),
                            2
                        );
                        updated
                            .update(suprnova::attrs! {title: "After update"})
                            .await
                            .unwrap();
                        removed.force_delete().await.unwrap();
                        assert_eq!(
                            SearchRegistry::generation_for("search_command_notes")
                                .await
                                .unwrap(),
                            old
                        );
                        assert_eq!(
                            CommandNote::search("After")
                                .get()
                                .await
                                .unwrap()
                                .total
                                .value(),
                            1
                        );
                        barrier.release.notify_one();
                    };
                    let (result, ()) = tokio::time::timeout(Duration::from_secs(10), async {
                        tokio::join!(rebuild, during)
                    })
                    .await
                    .unwrap();
                    result.unwrap();
                })
                .await;
            assert_ne!(
                SearchRegistry::generation_for("search_command_notes")
                    .await
                    .unwrap(),
                old
            );
            assert_eq!(
                CommandNote::search("After")
                    .get()
                    .await
                    .unwrap()
                    .total
                    .value(),
                1
            );
            assert_eq!(
                CommandNote::search("Before")
                    .get()
                    .await
                    .unwrap()
                    .total
                    .value(),
                0
            );
            assert_eq!(pending_count().await, 0);
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn queued_dual_writes_pin_both_generations_and_hold_publication_until_handled() {
    let _db = database().await;
    let _queue = Queue::fake();
    SearchConfig::scope(SearchConfig { queue: true, ..Default::default() }, SearchRegistry::isolated(async {
        SearchRegistry::register_engine("database", Arc::new(DatabaseSearchEngine::new())).unwrap();
        generations::create_index("search_command_notes").await.unwrap();
        let note = CommandNote::without_syncing_to_search(|| async {
            CommandNote::create(suprnova::attrs! {title: "Queued before"}).await
        }).await.unwrap();
        SearchConfig::scope(SearchConfig { queue: false, ..Default::default() }, generations::import_index("search_command_notes")).await.unwrap().unwrap();
        let old = SearchRegistry::generation_for("search_command_notes").await.unwrap();
        let barrier = ImportBarrier::new();
        IMPORT_BARRIER.scope(barrier.clone(), async {
            let rebuild = generations::reindex("search_command_notes");
            let during = async {
                barrier.entered.notified().await;
                note.update(suprnova::attrs! {title: "Queued after"}).await.unwrap();
                let jobs = suprnova::queue::testing::pending_jobs();
                assert_eq!(jobs.len(), 2);
                let pinned: std::collections::BTreeSet<_> = jobs.iter().map(|job| job.payload["generation"].as_str().unwrap().to_owned()).collect();
                assert_eq!(pinned.len(), 2);
                assert!(pinned.contains(old.as_str()));
                assert_eq!(pending_count().await, 3);
                barrier.release.notify_one();
                tokio::time::timeout(Duration::from_secs(5), async {
                    loop {
                        let watermark = DB::scalar::<i64>("SELECT COALESCE(MAX(switch_watermark), 0) FROM search_generations WHERE index_name = ? AND state = 'building'", vec!["search_command_notes".into()]).await.unwrap();
                        if watermark > 0 { break; }
                        tokio::task::yield_now().await;
                    }
                }).await.unwrap();
                assert_eq!(SearchRegistry::generation_for("search_command_notes").await.unwrap(), old);
                assert_eq!(pending_count().await, 2);
                for job in jobs {
                    suprnova::queue::worker::dispatch_by_name(&job.name, job.payload).await.unwrap();
                }
            };
            let (result, ()) = tokio::time::timeout(Duration::from_secs(10), async { tokio::join!(rebuild, during) }).await.unwrap();
            result.unwrap();
        }).await;
        assert_ne!(SearchRegistry::generation_for("search_command_notes").await.unwrap(), old);
        assert_eq!(CommandNote::search("after").get().await.unwrap().total.value(), 1);
        assert_eq!(pending_count().await, 0);
    })).await.unwrap();
}

#[tokio::test]
async fn retired_generation_references_prevent_drop_until_job_success_then_status_prunes() {
    let _db = database().await;
    SearchConfig::scope(
        SearchConfig {
            queue: false,
            ..Default::default()
        },
        SearchRegistry::isolated(async {
            let engine = Arc::new(DatabaseSearchEngine::new());
            SearchRegistry::register_engine("database", engine.clone()).unwrap();
            generations::create_index("search_command_notes")
                .await
                .unwrap();
            let note = CommandNote::create(suprnova::attrs! {title: "Retained"})
                .await
                .unwrap();
            let old = SearchRegistry::generation_for("search_command_notes")
                .await
                .unwrap();
            let key = note.id.to_string();
            let version = note.to_search_document().unwrap().source_version + 1;
            let replacement = GenerationId::new("replacement").unwrap();
            let descriptor = CommandNote::descriptor();
            engine
                .create_generation(descriptor, &descriptor.index_settings(), &replacement)
                .await
                .unwrap();
            engine
                .switch_generation(descriptor, Some(old.clone()), replacement)
                .await
                .unwrap();
            // A delayed job still naming a retired generation keeps its tables alive.
            generations::reserve(
                "search_command_notes",
                &[(key.clone(), version)],
                Some(&old),
            )
            .await
            .unwrap();
            generations::published("search_command_notes", &old, &[(key.clone(), version)])
                .await
                .unwrap();
            assert!(engine.drop_generation(descriptor, &old).await.is_err());
            let run = console::test(["search:status", "search_command_notes", "--prune"])
                .run()
                .await;
            run.assert_successful();
            assert!(run.output().contains(old.as_str()));
            assert!(run.output().contains("retired"));
            let duplicate = SearchIndexJob {
                index: "search_command_notes".into(),
                key,
                source_version: version,
                op: IndexOperation::Index,
                generation: Some(old.clone()),
            };
            duplicate.clone().handle().await.unwrap();
            assert_eq!(pending_count().await, 0);
            console::test(["search:status", "search_command_notes", "--prune"])
                .run()
                .await
                .assert_successful();
            assert!(
                !engine
                    .generation_states("search_command_notes")
                    .await
                    .unwrap()
                    .iter()
                    .any(|(id, _)| id == &old)
            );
            duplicate.handle().await.unwrap();
            assert_eq!(pending_count().await, 0);
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn status_prunes_expired_tombstones_and_keeps_fresh_removal_fences() {
    let _db = database().await;
    SearchConfig::scope(SearchConfig { queue: false, tombstone_hours: 1, ..Default::default() }, SearchRegistry::isolated(async {
        let engine = Arc::new(DatabaseSearchEngine::new());
        SearchRegistry::register_engine("database", engine.clone()).unwrap();
        generations::create_index("search_command_notes").await.unwrap();
        let current = SearchRegistry::generation_for("search_command_notes").await.unwrap();
        engine.remove(CommandNote::descriptor(), &current, &[("expired".into(),10),("fresh".into(),20)]).await.unwrap();
        DB::connection().unwrap().inner().execute_raw(Statement::from_sql_and_values(DbBackend::Sqlite, "UPDATE search_tombstones SET removed_at = ? WHERE index_name = ? AND document_key = ?", vec![(chrono::Utc::now().timestamp()-7200).into(),"search_command_notes".into(),"expired".into()])).await.unwrap();
        let run = console::test(["search:status", "search_command_notes", "--prune"]).run().await;
        run.assert_successful();
        assert!(run.output().contains("pruned 1 tombstones"));
        assert_eq!(DB::scalar::<i64>("SELECT COUNT(*) FROM search_tombstones WHERE index_name = ?", vec!["search_command_notes".into()]).await.unwrap(), 1);
        assert_eq!(DB::scalar::<String>("SELECT document_key FROM search_tombstones WHERE index_name = ?", vec!["search_command_notes".into()]).await.unwrap(), "fresh");
    })).await.unwrap();
}

#[tokio::test]
async fn status_recovers_a_reserved_payload_that_was_never_published() {
    let _db = database().await;
    SearchConfig::scope(
        SearchConfig {
            queue: false,
            ..Default::default()
        },
        SearchRegistry::isolated(async {
            SearchRegistry::register_engine("database", Arc::new(DatabaseSearchEngine::new()))
                .unwrap();
            generations::create_index("search_command_notes")
                .await
                .unwrap();
            let note = CommandNote::without_syncing_to_search(|| async {
                CommandNote::create(suprnova::attrs! {title: "Recover reservation"}).await
            })
            .await
            .unwrap();
            let current = SearchRegistry::generation_for("search_command_notes")
                .await
                .unwrap();
            generations::reserve(
                "search_command_notes",
                &[(note.id.to_string(), 100)],
                Some(&current),
            )
            .await
            .unwrap();
            assert_eq!(pending_count().await, 1);
            assert_eq!(
                CommandNote::search("reservation")
                    .get()
                    .await
                    .unwrap()
                    .total
                    .value(),
                0
            );
            console::test(["search:status", "search_command_notes", "--prune"])
                .run()
                .await
                .assert_successful();
            assert_eq!(pending_count().await, 0);
            assert_eq!(
                CommandNote::search("reservation")
                    .get()
                    .await
                    .unwrap()
                    .total
                    .value(),
                1
            );
            console::test(["search:status", "search_command_notes", "--prune"])
                .run()
                .await
                .assert_successful();
            assert_eq!(pending_count().await, 0);
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn driver_refuses_publication_with_pending_work_created_after_watermark() {
    let _db = database().await;
    SearchConfig::scope(SearchConfig { queue: false, ..Default::default() }, SearchRegistry::isolated(async {
        let engine = Arc::new(DatabaseSearchEngine::new());
        SearchRegistry::register_engine("database", engine.clone()).unwrap();
        generations::create_index("search_command_notes").await.unwrap();
        let old = SearchRegistry::generation_for("search_command_notes").await.unwrap();
        let descriptor = CommandNote::descriptor();
        let new = GenerationId::new("post_watermark").unwrap();
        engine.create_generation(descriptor, &descriptor.index_settings(), &new).await.unwrap();
        DB::connection().unwrap().inner().execute_raw(Statement::from_sql_and_values(DbBackend::Sqlite,
            "UPDATE search_generations SET switch_watermark=0 WHERE index_name=? AND generation=?",
            vec!["search_command_notes".into(),new.as_str().to_owned().into()])).await.unwrap();
        let keys = vec![("late".into(),100)];
        let targets = generations::reserve("search_command_notes", &keys, None).await.unwrap();
        assert_eq!(targets.len(), 2);
        assert!(engine.switch_generation(descriptor, Some(old.clone()), new.clone()).await.is_err());
        assert_eq!(engine.current_generation("search_command_notes").await.unwrap(), Some(old.clone()));
        for target in targets { generations::complete("search_command_notes", &target, &keys).await.unwrap(); }
        engine.switch_generation(descriptor, Some(old), new.clone()).await.unwrap();
        assert_eq!(engine.current_generation("search_command_notes").await.unwrap(), Some(new));
    })).await.unwrap();
}

#[tokio::test]
async fn committed_transactional_intent_recovers_after_missing_callback_and_rollback_does_not() {
    let _db = database().await;
    SearchConfig::scope(SearchConfig { queue: false, ..Default::default() }, SearchRegistry::isolated(async {
        SearchRegistry::register_engine("database", Arc::new(DatabaseSearchEngine::new())).unwrap();
        generations::create_index("search_command_notes").await.unwrap();
        let committed = CommandNote::without_syncing_to_search(|| async {
            CommandNote::create(suprnova::attrs! {title: "Committed intent"}).await
        }).await.unwrap();
        let rolled_back = CommandNote::without_syncing_to_search(|| async {
            CommandNote::create(suprnova::attrs! {title: "Rolledback intent"}).await
        }).await.unwrap();
        for (id,note,commit) in [("commit-intent",committed,true),("rollback-intent",rolled_back,false)] {
            let payload = serde_json::to_string(&SearchIndexJob {
                index: "search_command_notes".into(), key: note.id.to_string(), source_version: 100,
                op: IndexOperation::Index, generation: None,
            }).unwrap();
            let tx = DB::connection().unwrap().inner().begin().await.unwrap();
            tx.execute_raw(Statement::from_sql_and_values(DbBackend::Sqlite,
                "INSERT INTO search_job_intents(intent_id,index_name,job_name,payload_json,created_at) VALUES (?,?,?,?,?)",
                vec![id.into(),"search_command_notes".into(),"Suprnova::SearchIndex".into(),payload.into(),chrono::Utc::now().timestamp().into()])).await.unwrap();
            if commit { tx.commit().await.unwrap(); } else { tx.rollback().await.unwrap(); }
        }
        assert_eq!(DB::scalar::<i64>("SELECT COUNT(*) FROM search_job_intents WHERE index_name=?",vec!["search_command_notes".into()]).await.unwrap(),1);
        console::test(["search:status", "search_command_notes", "--prune"]).run().await.assert_successful();
        assert_eq!(CommandNote::search("Committed").get().await.unwrap().total.value(),1);
        assert_eq!(CommandNote::search("Rolledback").get().await.unwrap().total.value(),0);
        assert_eq!(DB::scalar::<i64>("SELECT COUNT(*) FROM search_job_intents WHERE index_name=?",vec!["search_command_notes".into()]).await.unwrap(),0);
        assert_eq!(pending_count().await,0);
    })).await.unwrap();
}

#[tokio::test]
async fn a_duplicate_flush_receipt_keeps_documents_saved_after_its_cutoff() {
    let _db = database().await;
    let _queue = Queue::fake();
    SearchConfig::scope(
        SearchConfig {
            queue: false,
            ..Default::default()
        },
        SearchRegistry::isolated(async {
            SearchRegistry::register_engine("database", Arc::new(DatabaseSearchEngine::new()))
                .unwrap();
            generations::create_index("search_command_notes")
                .await
                .unwrap();
            CommandNote::create(suprnova::attrs! {title: "Before flush"})
                .await
                .unwrap();
            SearchConfig::scope(
                SearchConfig {
                    queue: true,
                    ..Default::default()
                },
                generations::flush_index("search_command_notes"),
            )
            .await
            .unwrap()
            .unwrap();
            let jobs = suprnova::queue::testing::pending_jobs();
            assert_eq!(jobs.len(), 1);
            let flush = &jobs[0];
            assert_eq!(flush.name, "Suprnova::SearchFlush");
            suprnova::queue::worker::dispatch_by_name(&flush.name, flush.payload.clone())
                .await
                .unwrap();
            CommandNote::create(suprnova::attrs! {title: "After flush"})
                .await
                .unwrap();
            assert_eq!(
                CommandNote::search("After")
                    .get()
                    .await
                    .unwrap()
                    .total
                    .value(),
                1
            );
            suprnova::queue::worker::dispatch_by_name(&flush.name, flush.payload.clone())
                .await
                .unwrap();
            assert_eq!(
                CommandNote::search("After")
                    .get()
                    .await
                    .unwrap()
                    .total
                    .value(),
                1
            );
            assert_eq!(
                CommandNote::search("Before")
                    .get()
                    .await
                    .unwrap()
                    .total
                    .value(),
                0
            );
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn repeated_version_bounded_sweeps_never_remove_rows_newer_than_the_original_cutoff() {
    let _db = database().await;
    SearchConfig::scope(
        SearchConfig {
            queue: false,
            ..Default::default()
        },
        SearchRegistry::isolated(async {
            let engine = Arc::new(DatabaseSearchEngine::new());
            SearchRegistry::register_engine("database", engine.clone()).unwrap();
            generations::create_index("search_command_notes")
                .await
                .unwrap();
            let current = SearchRegistry::generation_for("search_command_notes")
                .await
                .unwrap();
            let descriptor = CommandNote::descriptor();
            let make_document = |key: &str, version: i64| {
                let mut document =
                    super::database_support::document(descriptor, key, "Cutoff corpus", "unused");
                document
                    .document
                    .attributes
                    .retain(|name, _| name == "title");
                document.document.source_version = version;
                document
            };
            engine
                .index(
                    descriptor,
                    &current,
                    &[
                        make_document("old-a", 10),
                        make_document("old-b", 20),
                        make_document("new", 100),
                    ],
                )
                .await
                .unwrap();
            let cutoff = 50;
            let selected = engine
                .keys_before_version(descriptor, &current, cutoff, 1)
                .await
                .unwrap();
            assert_eq!(selected.len(), 1);
            // A concurrent save advances a selected row before the original sweep removes it.
            engine
                .index(descriptor, &current, &[make_document(&selected[0].0, 101)])
                .await
                .unwrap();
            let removed: Vec<_> = selected.into_iter().map(|(key, _)| (key, cutoff)).collect();
            let result = engine.remove(descriptor, &current, &removed).await.unwrap();
            assert_eq!(result.skipped_older, 1);
            loop {
                let selected = engine
                    .keys_before_version(descriptor, &current, cutoff, 1)
                    .await
                    .unwrap();
                if selected.is_empty() {
                    break;
                }
                let removed: Vec<_> = selected.into_iter().map(|(key, _)| (key, cutoff)).collect();
                engine.remove(descriptor, &current, &removed).await.unwrap();
            }
            assert!(
                engine
                    .keys_before_version(descriptor, &current, cutoff, 1)
                    .await
                    .unwrap()
                    .is_empty()
            );
            let query = suprnova::search::testing::conformance::empty_query();
            assert_eq!(
                engine
                    .count(descriptor, &current, &query)
                    .await
                    .unwrap()
                    .value(),
                2
            );
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn sqlite_database_indexing_waits_for_ambient_and_explicit_commit_and_discards_savepoints() {
    use sea_orm_migration::MigratorTrait;
    use suprnova::{DatabaseConfig, DbConnection, FrameworkError, testing::TestContainer};
    let _db = database().await;
    // A second connection observes the index while the source transaction holds the first.
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("transaction-search.sqlite");
    let connection = DbConnection::connect(
        &DatabaseConfig::builder()
            .url(format!("sqlite://{}?mode=rwc", path.display()))
            .max_connections(2)
            .min_connections(2)
            .logging(false)
            .build(),
    )
    .await
    .unwrap();
    connection
        .inner()
        .execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "PRAGMA journal_mode=WAL",
        ))
        .await
        .unwrap();
    super::database_support::Migrator::up(connection.inner(), None)
        .await
        .unwrap();
    connection.inner().execute_raw(Statement::from_string(DbBackend::Sqlite,
        "CREATE TABLE search_command_notes (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL)")).await.unwrap();
    TestContainer::singleton(connection);
    SearchConfig::scope(
        SearchConfig {
            queue: false,
            ..Default::default()
        },
        SearchRegistry::isolated(async {
            let engine = Arc::new(DatabaseSearchEngine::new());
            SearchRegistry::register_engine("database", engine.clone()).unwrap();
            generations::create_index("search_command_notes")
                .await
                .unwrap();
            let current = SearchRegistry::generation_for("search_command_notes")
                .await
                .unwrap();
            let query = suprnova::search::testing::conformance::empty_query();
            let observer = || async {
                tokio::time::timeout(
                    Duration::from_secs(5),
                    engine.count(CommandNote::descriptor(), &current, &query),
                )
                .await
                .unwrap()
                .unwrap()
                .value()
            };
            let engine_copy = engine.clone();
            let current_copy = current.clone();
            let query_copy = query.clone();
            let rollback: Result<(), FrameworkError> = DB::transaction(move |_| {
                Box::pin(async move {
                    CommandNote::create(suprnova::attrs! {title: "Ambient rollback"}).await?;
                    let count = tokio::time::timeout(
                        Duration::from_secs(5),
                        engine_copy.count(CommandNote::descriptor(), &current_copy, &query_copy),
                    )
                    .await
                    .unwrap()?;
                    assert_eq!(count.value(), 0);
                    Err(FrameworkError::param("rollback ambient search write"))
                })
            })
            .await;
            assert!(rollback.is_err());
            assert_eq!(observer().await, 0);
            let tx = DB::begin_transaction().await.unwrap();
            CommandNote::create_with_tx(&tx, suprnova::attrs! {title: "Explicit rollback"})
                .await
                .unwrap();
            assert_eq!(observer().await, 0);
            tx.rollback().await.unwrap();
            assert_eq!(observer().await, 0);
            let engine_copy = engine.clone();
            let current_copy = current.clone();
            let query_copy = query.clone();
            DB::transaction(move |_| {
                Box::pin(async move {
                    let child: Result<(), FrameworkError> = DB::transaction(|_| {
                        Box::pin(async {
                            CommandNote::create(suprnova::attrs! {title: "Nested rollback"})
                                .await?;
                            Err(FrameworkError::param("rollback child search write"))
                        })
                    })
                    .await;
                    assert!(child.is_err());
                    assert_eq!(
                        tokio::time::timeout(
                            Duration::from_secs(5),
                            engine_copy.count(
                                CommandNote::descriptor(),
                                &current_copy,
                                &query_copy
                            )
                        )
                        .await
                        .unwrap()?
                        .value(),
                        0
                    );
                    Ok(())
                })
            })
            .await
            .unwrap();
            assert_eq!(observer().await, 0);
            let engine_copy = engine.clone();
            let current_copy = current.clone();
            let query_copy = query.clone();
            DB::transaction(move |_| {
                Box::pin(async move {
                    CommandNote::create(suprnova::attrs! {title: "Committed inline"}).await?;
                    assert_eq!(
                        tokio::time::timeout(
                            Duration::from_secs(5),
                            engine_copy.count(
                                CommandNote::descriptor(),
                                &current_copy,
                                &query_copy
                            )
                        )
                        .await
                        .unwrap()?
                        .value(),
                        0
                    );
                    Ok(())
                })
            })
            .await
            .unwrap();
            assert_eq!(observer().await, 1);
            assert_eq!(
                CommandNote::search("Committed")
                    .get()
                    .await
                    .unwrap()
                    .total
                    .value(),
                1
            );
            assert_eq!(
                CommandNote::search("rollback")
                    .get()
                    .await
                    .unwrap()
                    .total
                    .value(),
                0
            );
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn a_flush_dispatched_before_an_older_queued_create_prevents_resurrection() {
    let _db = database().await;
    let _queue = Queue::fake();
    SearchConfig::scope(
        SearchConfig {
            queue: true,
            ..Default::default()
        },
        SearchRegistry::isolated(async {
            let engine = Arc::new(DatabaseSearchEngine::new());
            SearchRegistry::register_engine("database", engine.clone()).unwrap();
            generations::create_index("search_command_notes")
                .await
                .unwrap();
            CommandNote::create(suprnova::attrs! {title: "Queued before removal"})
                .await
                .unwrap();
            generations::flush_index("search_command_notes")
                .await
                .unwrap();
            let jobs = suprnova::queue::testing::pending_jobs();
            assert_eq!(jobs.len(), 2);
            let create = jobs
                .iter()
                .find(|job| job.name == "Suprnova::SearchIndex")
                .unwrap();
            let flush = jobs
                .iter()
                .find(|job| job.name == "Suprnova::SearchFlush")
                .unwrap();
            assert!(
                flush.payload["source_version"].as_i64().unwrap()
                    > create.payload["source_version"].as_i64().unwrap()
            );
            suprnova::queue::worker::dispatch_by_name(&flush.name, flush.payload.clone())
                .await
                .unwrap();
            suprnova::queue::worker::dispatch_by_name(&create.name, create.payload.clone())
                .await
                .unwrap();
            let current = SearchRegistry::generation_for("search_command_notes")
                .await
                .unwrap();
            let query = suprnova::search::testing::conformance::empty_query();
            assert_eq!(
                engine
                    .count(CommandNote::descriptor(), &current, &query)
                    .await
                    .unwrap()
                    .value(),
                0
            );
            assert_eq!(pending_count().await, 0);
            // A replayed old envelope remains harmless after the completion receipt exists.
            suprnova::queue::worker::dispatch_by_name(&create.name, create.payload.clone())
                .await
                .unwrap();
            assert_eq!(
                engine
                    .count(CommandNote::descriptor(), &current, &query)
                    .await
                    .unwrap()
                    .value(),
                0
            );
        }),
    )
    .await
    .unwrap();
}
