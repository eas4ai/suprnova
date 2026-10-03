//! Migrators and cases every engine runs.

use sea_orm::{ConnectionTrait, Database, DatabaseConnection, Statement};
use sea_orm_migration::prelude::*;
use std::path::Path;
use suprnova::{PrunedMigration, SchemaDump};

/// A migration from plain SQL every engine accepts. Its tables are created
/// without `IF NOT EXISTS`, so running it twice fails.
macro_rules! sql_migration {
    ($ty:ident, $name:literal, [$($up:literal),+], [$($down:literal),+]) => {
        pub struct $ty;

        impl MigrationName for $ty {
            fn name(&self) -> &str {
                $name
            }
        }

        #[async_trait::async_trait]
        impl MigrationTrait for $ty {
            async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
                $(manager.get_connection().execute_unprepared($up).await?;)+
                Ok(())
            }

            async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
                $(manager.get_connection().execute_unprepared($down).await?;)+
                Ok(())
            }
        }
    };
}

pub const USERS: &str = "m20260101_000001_create_sd_users";
pub const POSTS: &str = "m20260101_000002_create_sd_posts";
pub const COMMENTS: &str = "m20260101_000003_create_sd_comments";

sql_migration!(
    CreateUsers,
    "m20260101_000001_create_sd_users",
    [
        "CREATE TABLE sd_users (id INTEGER PRIMARY KEY, email VARCHAR(190) NOT NULL)",
        "CREATE INDEX sd_users_email ON sd_users (email)"
    ],
    ["DROP TABLE sd_users"]
);
sql_migration!(
    CreatePosts,
    "m20260101_000002_create_sd_posts",
    ["CREATE TABLE sd_posts (id INTEGER PRIMARY KEY, title VARCHAR(190) NOT NULL)"],
    ["DROP TABLE sd_posts"]
);
sql_migration!(
    CreateComments,
    "m20260101_000003_create_sd_comments",
    ["CREATE TABLE sd_comments (id INTEGER PRIMARY KEY, body VARCHAR(190) NOT NULL)"],
    ["DROP TABLE sd_comments"]
);

/// The first two migrations, the ones a dump covers.
pub struct First;

#[async_trait::async_trait]
impl MigratorTrait for First {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(CreateUsers), Box::new(CreatePosts)]
    }
}

/// The first two and a newer one.
pub struct Newer;

#[async_trait::async_trait]
impl MigratorTrait for Newer {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(CreateUsers),
            Box::new(CreatePosts),
            Box::new(CreateComments),
        ]
    }
}

/// The first two pruned, and the newer one.
pub struct Pruned;

#[async_trait::async_trait]
impl MigratorTrait for Pruned {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(PrunedMigration::new(USERS)),
            Box::new(PrunedMigration::new(POSTS)),
            Box::new(CreateComments),
        ]
    }
}

/// No migrations: its `fresh` drops every table.
struct Nothing;

#[async_trait::async_trait]
impl MigratorTrait for Nothing {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        Vec::new()
    }
}

/// Drop every table, the ledger included.
pub async fn wipe(db: &DatabaseConnection) {
    Nothing::fresh(db).await.expect("drop every table");
    db.execute_unprepared("DROP TABLE seaql_migrations")
        .await
        .expect("drop the ledger");
}

pub async fn has_table(db: &DatabaseConnection, table: &str) -> bool {
    SchemaManager::new(db)
        .has_table(table)
        .await
        .expect("look up a table")
}

/// The versions the ledger records, in order.
pub async fn ledger(db: &DatabaseConnection) -> Vec<String> {
    let rows = db
        .query_all_raw(Statement::from_string(
            db.get_database_backend(),
            "SELECT version FROM seaql_migrations ORDER BY version",
        ))
        .await
        .expect("read the ledger");
    rows.iter()
        .map(|row| row.try_get::<String>("", "version").expect("a version"))
        .collect()
}

/// PAR-038: the dump holds the schema and the ledger, never other rows,
/// and lands at the engine's default path.
pub async fn dump_holds_the_schema_and_the_ledger(url: &str, dir: &Path, engine: &str) {
    let db = Database::connect(url).await.expect("connect");
    wipe(&db).await;
    SchemaDump::migrate::<First>(url, None)
        .await
        .expect("migrate");
    db.execute_unprepared("INSERT INTO sd_users (id, email) VALUES (1, 'kept-out@example.com')")
        .await
        .expect("insert a row");

    let path = dir.join("dump.sql");
    SchemaDump::dump::<First>(url, &path).await.expect("dump");
    let sql = std::fs::read_to_string(&path).expect("read the dump");
    for name in ["sd_users", "sd_posts", "sd_users_email"] {
        assert!(sql.contains(name), "the dump creates {name}:\n{sql}");
    }
    assert!(
        !sql.contains("kept-out@example.com"),
        "a table's rows stay out of the dump"
    );
    let inserts: Vec<&str> = sql
        .lines()
        .filter(|line| line.trim_start().to_ascii_uppercase().starts_with("INSERT"))
        .collect();
    assert_eq!(inserts.len(), 2, "one INSERT per ledger row: {inserts:?}");
    for name in [USERS, POSTS] {
        assert!(
            inserts
                .iter()
                .any(|line| line.contains("seaql_migrations") && line.contains(name)),
            "the ledger row for {name}: {inserts:?}"
        );
    }

    assert_eq!(
        SchemaDump::default_path(url).await.expect("a default path"),
        suprnova::database_path(format!("schema/{engine}-schema.sql"))
    );
}

/// PAR-039: an empty database loads the dump, then runs only the newer
/// migration; `fresh` does the same after dropping everything.
pub async fn the_dump_loads_before_newer_migrations(url: &str, dir: &Path) {
    let db = Database::connect(url).await.expect("connect");
    wipe(&db).await;
    SchemaDump::migrate::<First>(url, None)
        .await
        .expect("migrate");
    let path = dir.join("dump.sql");
    SchemaDump::dump::<First>(url, &path).await.expect("dump");

    wipe(&db).await;
    SchemaDump::migrate::<Newer>(url, Some(&path))
        .await
        .expect("load, then run the newer migration");
    for table in ["sd_users", "sd_posts", "sd_comments"] {
        assert!(has_table(&db, table).await, "{table} after the load");
    }
    assert_eq!(ledger(&db).await, [USERS, POSTS, COMMENTS]);

    SchemaDump::fresh::<Newer>(url, Some(&path))
        .await
        .expect("fresh loads the dump too");
    for table in ["sd_users", "sd_posts", "sd_comments"] {
        assert!(has_table(&db, table).await, "{table} after fresh");
    }
    assert_eq!(ledger(&db).await, [USERS, POSTS, COMMENTS]);
}

/// PAR-039: a database whose ledger records a migration is never loaded,
/// and a load that fails runs no migration.
pub async fn only_an_empty_ledger_loads(url: &str, dir: &Path) {
    let db = Database::connect(url).await.expect("connect");
    wipe(&db).await;
    SchemaDump::migrate::<First>(url, None)
        .await
        .expect("migrate");
    let path = dir.join("dump.sql");
    SchemaDump::dump::<First>(url, &path).await.expect("dump");
    let mut sql = std::fs::read_to_string(&path).expect("read the dump");
    sql.push_str("\nCREATE TABLE sd_only_in_dump (id INTEGER PRIMARY KEY);\n");
    std::fs::write(&path, sql).expect("write the dump");

    SchemaDump::migrate::<Newer>(url, Some(&path))
        .await
        .expect("migrate a migrated database");
    assert!(
        !has_table(&db, "sd_only_in_dump").await,
        "a database with an applied migration is not loaded"
    );
    assert!(has_table(&db, "sd_comments").await);

    wipe(&db).await;
    let broken = dir.join("broken.sql");
    std::fs::write(&broken, "THIS IS NOT SQL;\n").expect("write a broken dump");
    assert!(
        SchemaDump::migrate::<Newer>(url, Some(&broken))
            .await
            .is_err(),
        "a failed load is an error"
    );
    for table in ["sd_users", "sd_comments"] {
        assert!(
            !has_table(&db, table).await,
            "no migration runs after a failed load: {table}"
        );
    }
}

/// PAR-040: pruned migrations migrate on where the ledger has them, load
/// from the dump where it doesn't, and fail loudly with neither.
pub async fn pruned_migrations_keep_their_names(url: &str, dir: &Path) {
    let db = Database::connect(url).await.expect("connect");
    wipe(&db).await;
    SchemaDump::migrate::<First>(url, None)
        .await
        .expect("migrate");
    let path = dir.join("dump.sql");
    SchemaDump::dump::<First>(url, &path).await.expect("dump");
    SchemaDump::migrate::<Pruned>(url, None)
        .await
        .expect("a database that applied the pruned migrations migrates on");
    assert!(has_table(&db, "sd_comments").await);

    wipe(&db).await;
    SchemaDump::migrate::<Pruned>(url, Some(&path))
        .await
        .expect("an empty database loads the dump, then the newer migration");
    assert_eq!(ledger(&db).await, [USERS, POSTS, COMMENTS]);
    let err = Pruned::down(&db, None)
        .await
        .expect_err("a pruned migration cannot be rolled back");
    assert!(err.to_string().contains(POSTS), "{err}");

    wipe(&db).await;
    let err = SchemaDump::migrate::<Pruned>(url, Some(&dir.join("absent.sql")))
        .await
        .expect_err("a pruned migration with no dump to load fails");
    let message = err.to_string();
    assert!(
        message.contains(USERS) && message.contains("database/schema"),
        "{message}"
    );
}
