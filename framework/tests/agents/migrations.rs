//! AGENT-017: the two plugins' tables as migrations an application appends
//! to its migrator (`suprnova::agents::migrations()`).
//!
//! Each engine runs the set the way a migrator does, inside the
//! transaction PostgreSQL migrations run in, twice: the second run changes
//! nothing. The PostgreSQL and MySQL runs share their database with other
//! engine tests, so they never drop the plugins' tables; SQLite, on a
//! database of its own, checks `down` too.

use std::time::Duration;

use sea_orm::{
    ConnectionTrait, Database, DatabaseConnection, DbBackend, Statement, TransactionTrait,
};
use sea_orm_migration::prelude::*;
use suprnova::agents::{AgentAuth, OAuthServer};

use crate::agent_auth::{agent_world_on, anonymous};
use crate::oauth::container;

/// Every table the set creates.
const TABLES: [&str; 8] = [
    "auth_oauth_clients",
    "auth_oauth_authorizations",
    "auth_oauth_grants",
    "auth_oauth_tokens",
    "auth_agent_registrations",
    "auth_agent_grants",
    "auth_agent_replays",
    "auth_agent_events",
];

/// The unique indexes a digest lookup depends on.
const UNIQUE_DIGESTS: [(&str, &str); 4] = [
    ("auth_oauth_tokens", "auth_oauth_tokens_token_hash_unique"),
    (
        "auth_agent_registrations",
        "auth_agent_registrations_claim_token_unique",
    ),
    (
        "auth_agent_registrations",
        "auth_agent_registrations_attempt_token_unique",
    ),
    (
        "auth_agent_registrations",
        "auth_agent_registrations_delegation_unique",
    ),
];

/// Run every migration's `up` as a migrator does: each inside a
/// transaction, as SeaORM runs them on PostgreSQL.
async fn up_in_transactions(db: &DatabaseConnection) {
    for migration in suprnova::agents::migrations() {
        let transaction = db.begin().await.expect("begin");
        migration
            .up(&SchemaManager::new(&transaction))
            .await
            .unwrap_or_else(|error| panic!("{}: {error}", migration.name()));
        transaction.commit().await.expect("commit");
    }
}

/// The set's tables and indexes as the engine describes them: one line
/// each, sorted, so two runs compare whole.
async fn schema_of(db: &DatabaseConnection) -> Vec<String> {
    let backend = db.get_database_backend();
    let names = TABLES
        .iter()
        .map(|table| format!("'{table}'"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = match backend {
        DbBackend::Sqlite => format!(
            "SELECT type || ' ' || name || ' ' || tbl_name || ' ' || COALESCE(sql, '') \
             FROM sqlite_master WHERE tbl_name IN ({names})"
        ),
        DbBackend::Postgres => format!(
            "SELECT 'table ' || table_name || ' ' || column_name || ' ' || data_type \
             FROM information_schema.columns \
             WHERE table_schema = current_schema() AND table_name IN ({names}) \
             UNION ALL \
             SELECT 'index ' || indexname || ' ' || indexdef FROM pg_indexes \
             WHERE schemaname = current_schema() AND tablename IN ({names})"
        ),
        DbBackend::MySql => format!(
            "SELECT CONCAT('table ', TABLE_NAME, ' ', COLUMN_NAME, ' ', COLUMN_TYPE) \
             FROM information_schema.COLUMNS \
             WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME IN ({names}) \
             UNION ALL \
             SELECT CONCAT('index ', INDEX_NAME, ' ', TABLE_NAME, ' ', COLUMN_NAME, ' ', \
             NON_UNIQUE, ' ', SEQ_IN_INDEX) FROM information_schema.STATISTICS \
             WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME IN ({names})"
        ),
        _ => panic!("an engine the plugins support"),
    };
    let mut lines: Vec<String> = db
        .query_all_raw(Statement::from_string(backend, sql))
        .await
        .expect("the schema")
        .into_iter()
        .map(|row| row.try_get_by_index::<String>(0).expect("a line"))
        .collect();
    lines.sort();
    lines
}

/// Whether `table` has the index `index`, unique.
async fn has_unique_index(db: &DatabaseConnection, table: &str, index: &str) -> bool {
    let backend = db.get_database_backend();
    let statement = match backend {
        DbBackend::Sqlite => Statement::from_sql_and_values(
            backend,
            "SELECT 1 FROM pragma_index_list(?) WHERE name = ? AND \"unique\" = 1",
            [table.into(), index.into()],
        ),
        DbBackend::Postgres => Statement::from_sql_and_values(
            backend,
            "SELECT 1 FROM pg_indexes WHERE schemaname = current_schema() AND tablename = $1 \
             AND indexname = $2 AND indexdef LIKE 'CREATE UNIQUE INDEX%'",
            [table.into(), index.into()],
        ),
        DbBackend::MySql => Statement::from_sql_and_values(
            backend,
            "SELECT 1 FROM information_schema.STATISTICS WHERE TABLE_SCHEMA = DATABASE() \
             AND TABLE_NAME = ? AND INDEX_NAME = ? AND NON_UNIQUE = 0 LIMIT 1",
            [table.into(), index.into()],
        ),
        _ => panic!("an engine the plugins support"),
    };
    db.query_one_raw(statement)
        .await
        .expect("the index lookup")
        .is_some()
}

/// Whether `table` exists.
async fn has_table(db: &DatabaseConnection, table: &str) -> bool {
    let backend = db.get_database_backend();
    let statement = match backend {
        DbBackend::Sqlite => Statement::from_sql_and_values(
            backend,
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?",
            [table.into()],
        ),
        DbBackend::Postgres => Statement::from_sql_and_values(
            backend,
            "SELECT 1 FROM information_schema.tables \
             WHERE table_schema = current_schema() AND table_name = $1",
            [table.into()],
        ),
        DbBackend::MySql => Statement::from_sql_and_values(
            backend,
            "SELECT 1 FROM information_schema.TABLES \
             WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = ?",
            [table.into()],
        ),
        _ => panic!("an engine the plugins support"),
    };
    db.query_one_raw(statement)
        .await
        .expect("the table lookup")
        .is_some()
}

/// Run the set twice on `db` and check the second run changed nothing.
async fn twice_changes_nothing(db: &DatabaseConnection) {
    up_in_transactions(db).await;
    let first = schema_of(db).await;
    up_in_transactions(db).await;
    let second = schema_of(db).await;
    assert_eq!(first, second, "the second run changed the schema");
    for table in TABLES {
        assert!(has_table(db, table).await, "{table}");
    }
    for (table, index) in UNIQUE_DIGESTS {
        assert!(has_unique_index(db, table, index).await, "{index}");
    }
}

/// A migrator that appends the set to an application's own migrations,
/// as a kit's does.
struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        let mut migrations: Vec<Box<dyn MigrationTrait>> = Vec::new();
        migrations.extend(suprnova::agents::migrations());
        migrations
    }
}

#[test]
fn agent_017_agents_migrations_are_named_and_ordered() {
    let names: Vec<String> = suprnova::agents::migrations()
        .iter()
        .map(|migration| migration.name().to_owned())
        .collect();
    assert_eq!(
        names,
        [
            "m20261019_000001_create_oauth_server_tables",
            "m20261019_000002_create_agent_auth_tables",
        ]
    );
}

#[tokio::test]
async fn agent_017_agents_migrations_create_both_plugins_tables_on_sqlite() {
    let db = Database::connect("sqlite::memory:").await.expect("SQLite");
    twice_changes_nothing(&db).await;
    // The plugins serve on the tables the set made.
    let _container = container();
    let world = agent_world_on(db.clone(), |agent| agent);
    let registration = anonymous(&world.client).await;
    assert!(
        registration["identity_assertion"].is_string(),
        "{registration}"
    );
    // Beside the boot-time migrate, which creates the same tables, the set
    // still runs.
    AgentAuth::migrate(&db).await.expect("agent auth's tables");
    up_in_transactions(&db).await;
}

/// The rows of `table`.
async fn rows(db: &DatabaseConnection, table: &str) -> i64 {
    db.query_one_raw(Statement::from_string(
        db.get_database_backend(),
        format!("SELECT COUNT(*) FROM {table}"),
    ))
    .await
    .expect("the count")
    .expect("a row")
    .try_get_by_index::<i64>(0)
    .expect("a number")
}

#[tokio::test]
async fn agent_017_agents_migrations_down_leaves_the_tables_and_their_rows() {
    // The tables already exist, made at boot, and hold live credentials.
    let db = Database::connect("sqlite::memory:").await.expect("SQLite");
    OAuthServer::migrate(&db)
        .await
        .expect("the OAuth server's tables");
    AgentAuth::migrate(&db).await.expect("agent auth's tables");
    let _container = container();
    let world = agent_world_on(db.clone(), |agent| agent);
    anonymous(&world.client).await;
    let registrations = rows(&db, "auth_agent_registrations").await;
    assert_eq!(registrations, 1);
    // The set runs over them, and a rollback undoes nothing it did not do.
    up_in_transactions(&db).await;
    for migration in suprnova::agents::migrations().iter().rev() {
        migration
            .down(&SchemaManager::new(&db))
            .await
            .unwrap_or_else(|error| panic!("{}: {error}", migration.name()));
    }
    for table in TABLES {
        assert!(has_table(&db, table).await, "{table} stays");
    }
    assert_eq!(rows(&db, "auth_agent_registrations").await, registrations);
}

#[tokio::test]
async fn agent_017_agents_migrations_run_from_an_applications_migrator() {
    let db = Database::connect("sqlite::memory:").await.expect("SQLite");
    Migrator::up(&db, None).await.expect("up");
    for table in TABLES {
        assert!(has_table(&db, table).await, "{table}");
    }
    Migrator::down(&db, None).await.expect("down");
    // The migrator forgets them; the tables, which other paths also
    // create, stay for an operator to drop.
    for table in TABLES {
        assert!(has_table(&db, table).await, "{table} stays");
    }
    let pending = Migrator::get_pending_migrations(&db)
        .await
        .expect("the pending migrations");
    assert_eq!(pending.len(), 2, "both run again on the next up");
}

/// A disposable database the variable `variable` names.
async fn engine(variable: &str) -> DatabaseConnection {
    let url = std::env::var(variable)
        .unwrap_or_else(|_| panic!("set {variable} to a disposable database"));
    let mut options = sea_orm::ConnectOptions::new(url);
    options
        .max_connections(2)
        .min_connections(0)
        .connect_timeout(Duration::from_secs(5))
        .acquire_timeout(Duration::from_secs(5));
    Database::connect(options)
        .await
        .expect("the test database must be reachable")
}

#[tokio::test]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn agent_017_agents_migrations_on_postgres() {
    let db = engine("PG_TEST_URL").await;
    twice_changes_nothing(&db).await;
}

#[tokio::test]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn agent_017_agents_migrations_on_mysql() {
    let db = engine("MYSQL_TEST_URL").await;
    twice_changes_nothing(&db).await;
}
