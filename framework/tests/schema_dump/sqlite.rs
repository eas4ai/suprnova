//! SQLite, which needs no service and no client tool.

use crate::cases;
use std::path::PathBuf;
use suprnova::SchemaDump;

/// A temporary directory that is also the database directory, and the
/// URL of a SQLite file inside it.
fn database() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().expect("a temporary directory");
    suprnova::use_database_path(dir.path());
    let url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("app.sqlite").display()
    );
    (dir, url)
}

#[tokio::test]
async fn the_dump_holds_the_schema_and_the_ledger() {
    let (dir, url) = database();
    cases::dump_holds_the_schema_and_the_ledger(&url, dir.path(), "sqlite").await;
}

#[tokio::test]
async fn the_dump_loads_before_newer_migrations() {
    let (dir, url) = database();
    cases::the_dump_loads_before_newer_migrations(&url, dir.path()).await;
}

#[tokio::test]
async fn only_an_empty_ledger_loads() {
    let (dir, url) = database();
    cases::only_an_empty_ledger_loads(&url, dir.path()).await;
}

#[tokio::test]
async fn pruned_migrations_keep_their_names() {
    let (dir, url) = database();
    cases::pruned_migrations_keep_their_names(&url, dir.path()).await;
}

#[tokio::test]
async fn migrate_finds_the_dump_at_its_default_path() {
    let (_dir, url) = database();
    SchemaDump::migrate::<cases::First>(&url, None)
        .await
        .expect("migrate");
    let path = SchemaDump::default_path(&url)
        .await
        .expect("a default path");
    SchemaDump::dump::<cases::First>(&url, &path)
        .await
        .expect("dump to the default path");

    let (_fresh_dir, fresh_url) = database();
    let moved: PathBuf = SchemaDump::default_path(&fresh_url)
        .await
        .expect("a default path");
    std::fs::create_dir_all(moved.parent().expect("a parent")).expect("mkdir");
    std::fs::copy(&path, &moved).expect("copy the dump");
    SchemaDump::migrate::<cases::Newer>(&fresh_url, None)
        .await
        .expect("load from the default path, then migrate");
    let db = sea_orm::Database::connect(&fresh_url)
        .await
        .expect("connect");
    assert_eq!(
        cases::ledger(&db).await,
        [cases::USERS, cases::POSTS, cases::COMMENTS]
    );
}

#[cfg(feature = "testing")]
#[tokio::test]
async fn test_database_fresh_loads_the_sqlite_dump() {
    let (_dir, url) = database();
    SchemaDump::migrate::<cases::First>(&url, None)
        .await
        .expect("migrate");
    SchemaDump::dump::<cases::First>(&url, &suprnova::database_path("schema/sqlite-schema.sql"))
        .await
        .expect("dump");

    let test_db = suprnova::testing::TestDatabase::fresh::<cases::Pruned>()
        .await
        .expect("TestDatabase loads the dump, then the newer migration");
    for table in ["sd_users", "sd_posts", "sd_comments"] {
        assert!(cases::has_table(test_db.conn(), table).await, "{table}");
    }
}

/// PAR-038: a missing tool is an error that names it, never the password,
/// and an earlier dump stays as it was. Postgres is the engine whose tool
/// is looked up before any connection, so no server is needed.
#[tokio::test]
async fn a_missing_dump_tool_keeps_the_earlier_dump() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("postgres-schema.sql");
    std::fs::write(&path, "-- an earlier dump\n").expect("write an earlier dump");
    let empty = dir.path().join("bin");
    std::fs::create_dir(&empty).expect("mkdir");
    // SAFETY: nextest runs each test in its own process, so no other
    // thread reads PATH while it changes.
    unsafe { std::env::set_var("PATH", &empty) };

    let err =
        SchemaDump::dump::<cases::First>("postgres://dumper:hunter2@127.0.0.1:1/absent", &path)
            .await
            .expect_err("no pg_dump on PATH");
    let message = err.to_string();
    assert!(message.contains("pg_dump"), "{message}");
    assert!(!message.contains("hunter2"), "{message}");
    assert_eq!(
        std::fs::read_to_string(&path).expect("read"),
        "-- an earlier dump\n"
    );
}
