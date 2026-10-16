//! Search maintenance runs through the console dispatcher on a fresh SQLite database.

use sea_orm::{ConnectionTrait, DbBackend, Statement};
use std::sync::Arc;
use suprnova::{
    DB, Model, Searchable, console,
    search::{DatabaseSearchEngine, IndexSettings, SearchConfig, SearchEngine, SearchRegistry},
    testing::TestDatabase,
};

#[suprnova::model(table = "search_command_notes", timestamps = false, fillable = ["title"])]
#[derive(Searchable)]
#[searchable(
    index = "search_command_notes",
    settings = "command_settings",
    with = "prepare_chunk"
)]
pub(super) struct CommandNote {
    pub id: i64,
    #[searchable(text)]
    pub title: String,
}

pub(super) struct ImportBarrier {
    pub entered: tokio::sync::Notify,
    pub release: tokio::sync::Notify,
    claimed: std::sync::atomic::AtomicBool,
}

impl ImportBarrier {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            entered: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
            claimed: std::sync::atomic::AtomicBool::new(false),
        })
    }
}

tokio::task_local! { pub(super) static IMPORT_BARRIER: Arc<ImportBarrier>; }

impl CommandNote {
    async fn prepare_chunk(models: Vec<Self>) -> Result<Vec<Self>, suprnova::FrameworkError> {
        if let Ok(barrier) = IMPORT_BARRIER.try_with(Arc::clone)
            && !barrier
                .claimed
                .swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            barrier.entered.notify_one();
            barrier.release.notified().await;
        }
        Ok(models)
    }
}

tokio::task_local! { static SETTINGS: IndexSettings; }

fn command_settings() -> IndexSettings {
    SETTINGS.try_with(Clone::clone).unwrap_or_default()
}

pub(super) async fn database() -> TestDatabase {
    let database = TestDatabase::fresh::<super::database_support::Migrator>()
        .await
        .unwrap();
    DB::connection()
        .unwrap()
        .inner()
        .execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            "CREATE TABLE search_command_notes (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL)",
        ))
        .await
        .unwrap();
    database
}

#[tokio::test]
async fn dispatcher_runs_all_seven_search_commands_and_publishes_complete_generations() {
    let _database = database().await;
    SearchConfig::scope(
        SearchConfig {
            queue: false,
            ..Default::default()
        },
        SearchRegistry::isolated(async {
            let engine = Arc::new(DatabaseSearchEngine::new());
            SearchRegistry::register_engine("database", engine.clone()).unwrap();
            console::test(["search:index", "search_command_notes"])
                .run()
                .await
                .assert_successful();
            let first = SearchRegistry::generation_for("search_command_notes")
                .await
                .unwrap();
            CommandNote::without_syncing_to_search(|| async {
                CommandNote::create(suprnova::attrs! {title: "Complete import"}).await
            })
            .await
            .unwrap();
            console::test(["search:import", "CommandNote"])
                .run()
                .await
                .assert_successful();
            assert_eq!(
                CommandNote::search("Complete")
                    .get()
                    .await
                    .unwrap()
                    .total
                    .value(),
                1
            );
            let status = console::test(["search:status", "search_command_notes", "--prune"])
                .run()
                .await;
            status.assert_successful();
            assert!(status.output().contains("search_command_notes"));
            assert!(status.output().contains(first.as_str()));
            let sync = console::test(["search:sync-index-settings", "search_command_notes"])
                .run()
                .await;
            sync.assert_successful();
            assert!(sync.output().contains("search_command_notes"));
            let reindex = console::test(["search:reindex", "CommandNote"]).run().await;
            reindex.assert_successful();
            assert!(reindex.output().contains("search_command_notes"));
            let second = SearchRegistry::generation_for("search_command_notes")
                .await
                .unwrap();
            assert_ne!(first, second);
            assert_eq!(
                CommandNote::search("Complete")
                    .get()
                    .await
                    .unwrap()
                    .total
                    .value(),
                1
            );
            console::test(["search:flush", "CommandNote"])
                .run()
                .await
                .assert_successful();
            assert_eq!(
                CommandNote::search("Complete")
                    .get()
                    .await
                    .unwrap()
                    .total
                    .value(),
                0
            );
            console::test(["search:delete-index", "search_command_notes"])
                .run()
                .await
                .assert_successful();
            assert_eq!(
                engine
                    .current_generation("search_command_notes")
                    .await
                    .unwrap(),
                None
            );
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn synced_query_settings_preserve_generation_and_index_settings_build_a_new_one() {
    let _database = database().await;
    SearchConfig::scope(
        SearchConfig {
            queue: false,
            ..Default::default()
        },
        SearchRegistry::isolated(async {
            let engine = Arc::new(DatabaseSearchEngine::new());
            SearchRegistry::register_engine("database", engine.clone()).unwrap();
            console::test(["search:index", "search_command_notes"])
                .run()
                .await
                .assert_successful();
            CommandNote::create(suprnova::attrs! {title: "Settings import"})
                .await
                .unwrap();
            let first = SearchRegistry::generation_for("search_command_notes")
                .await
                .unwrap();
            let mut settings = command_settings();
            settings.query.highlight_pre_tag = "<strong>".into();
            settings.query.highlight_post_tag = "</strong>".into();
            SETTINGS
                .scope(settings.clone(), async {
                    let run = console::test(["search:sync-index-settings", "search_command_notes"])
                        .run()
                        .await;
                    run.assert_successful();
                    assert!(run.output().contains("query"));
                    assert_eq!(
                        SearchRegistry::generation_for("search_command_notes")
                            .await
                            .unwrap(),
                        first
                    );
                    assert_eq!(
                        engine
                            .read_settings(CommandNote::descriptor(), &first)
                            .await
                            .unwrap()
                            .query,
                        settings.query
                    );
                })
                .await;
            settings.index_time.embed_template = "Changed template: {text}".into();
            SETTINGS
                .scope(settings, async {
                    let run = console::test(["search:sync-index-settings", "search_command_notes"])
                        .run()
                        .await;
                    run.assert_successful();
                    assert!(run.output().contains("index"));
                    let second = SearchRegistry::generation_for("search_command_notes")
                        .await
                        .unwrap();
                    assert_ne!(first, second);
                    assert_eq!(
                        engine
                            .read_settings(CommandNote::descriptor(), &second)
                            .await
                            .unwrap(),
                        CommandNote::descriptor().index_settings()
                    );
                    assert_eq!(
                        CommandNote::search("Settings")
                            .get()
                            .await
                            .unwrap()
                            .total
                            .value(),
                        1
                    );
                })
                .await;
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn dispatcher_rejects_missing_extra_unknown_and_model_only_arguments() {
    for command in [
        "search:import",
        "search:flush",
        "search:index",
        "search:delete-index",
        "search:reindex",
    ] {
        let run = console::test([command]).run().await;
        run.assert_failed();
        assert!(run.errors().contains("requires a"), "{}", run.errors());
    }
    for command in [
        "search:import",
        "search:flush",
        "search:index",
        "search:delete-index",
        "search:reindex",
        "search:sync-index-settings",
        "search:status",
    ] {
        for arguments in [
            vec![command, "missing-index"],
            vec![command, "search_command_notes", "extra"],
            vec![command, "--unexpected"],
        ] {
            let run = console::test(arguments).run().await;
            run.assert_failed();
            assert!(run.errors().contains(command), "{}", run.errors());
        }
    }
    for command in [
        "search:index",
        "search:delete-index",
        "search:sync-index-settings",
        "search:status",
    ] {
        console::test([command, "CommandNote"])
            .run()
            .await
            .assert_failed();
    }
    console::test(["search:status", "--prune", "--prune"])
        .run()
        .await
        .assert_failed();
    console::test(["search:import", ""])
        .run()
        .await
        .assert_failed();
}
