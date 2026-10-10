//! The MySQL character set and collation, as Laravel's `mysql` and
//! `mariadb` connections read `DB_CHARSET` and `DB_COLLATION`: every
//! connection sends them, and every table `Schema::create` makes takes
//! them unless its blueprint names its own.
//!
//! The `mysql_` tests need a disposable MySQL or MariaDB database:
//!
//! ```text
//! MYSQL_TEST_URL=mysql://... \
//!   cargo nextest run -p suprnova --test schema --run-ignored only \
//!   -E 'test(/laravel_infra_gaps::mysql_/)' --test-threads 1
//! ```
//!
//! Explicit execution without the URL fails immediately; it never reports a
//! silent pass.

use sea_orm::{ConnectionTrait, DbBackend, Statement};
use sea_orm_migration::SchemaManager;
use suprnova::schema::Schema;
use suprnova::{Blueprint, ConnectionRegistry, DatabaseConfig, DbConnection};

use crate::env_snapshot::{EnvSnapshot, set_env};

const VARIABLES: &[&str] = &["DATABASE_URL", "DB_CHARSET", "DB_COLLATION"];

fn from_env_with(url: Option<&str>) -> DatabaseConfig {
    set_env("DATABASE_URL", url);
    DatabaseConfig::from_env()
}

// ---- DatabaseConfig ---------------------------------------------------------

#[test]
fn a_mysql_url_defaults_to_utf8mb4_and_its_unicode_collation() {
    let _env = crate::env_lock::lock_env();
    let _snap = EnvSnapshot::capture(VARIABLES);
    set_env("DB_CHARSET", None);
    set_env("DB_COLLATION", None);

    for url in ["mysql://u:p@h/db", "mariadb://u:p@h/db"] {
        let config = from_env_with(Some(url));
        assert_eq!(config.charset.as_deref(), Some("utf8mb4"), "{url}");
        assert_eq!(
            config.collation.as_deref(),
            Some("utf8mb4_unicode_ci"),
            "{url}"
        );
    }
}

#[test]
fn db_charset_and_db_collation_set_the_values() {
    let _env = crate::env_lock::lock_env();
    let _snap = EnvSnapshot::capture(VARIABLES);
    set_env("DB_CHARSET", Some("utf8mb3"));
    set_env("DB_COLLATION", Some("utf8mb4_bin"));

    let config = from_env_with(Some("mysql://u:p@h/db"));
    assert_eq!(config.charset.as_deref(), Some("utf8mb3"));
    assert_eq!(config.collation.as_deref(), Some("utf8mb4_bin"));
}

#[test]
fn a_url_parameter_wins_over_the_variables() {
    let _env = crate::env_lock::lock_env();
    let _snap = EnvSnapshot::capture(VARIABLES);
    set_env("DB_CHARSET", Some("utf8mb4"));
    set_env("DB_COLLATION", Some("utf8mb4_bin"));

    let config = from_env_with(Some("mysql://u:p@h/db?collation=utf8mb4_general_ci"));
    assert_eq!(config.collation.as_deref(), Some("utf8mb4_general_ci"));
    assert_eq!(config.charset.as_deref(), Some("utf8mb4"));

    let config = from_env_with(Some(
        "mariadb://u:p@h/db?ssl-mode=disabled&charset=latin1&collation=latin1_swedish_ci",
    ));
    assert_eq!(config.charset.as_deref(), Some("latin1"));
    assert_eq!(config.collation.as_deref(), Some("latin1_swedish_ci"));
}

#[test]
fn postgres_and_sqlite_urls_carry_no_charset_or_collation() {
    let _env = crate::env_lock::lock_env();
    let _snap = EnvSnapshot::capture(VARIABLES);
    set_env("DB_CHARSET", Some("utf8mb4"));
    set_env("DB_COLLATION", Some("utf8mb4_bin"));

    for url in [
        Some("postgres://u:p@h/db"),
        Some("postgresql://u:p@h/db?collation=x"),
        Some("sqlite://./database.db"),
        Some("sqlite::memory:"),
        None,
    ] {
        let config = from_env_with(url);
        assert_eq!(config.charset, None, "{url:?}");
        assert_eq!(config.collation, None, "{url:?}");
    }
}

#[test]
fn the_builder_sets_the_values_on_a_mysql_url_only() {
    let _env = crate::env_lock::lock_env();
    let _snap = EnvSnapshot::capture(VARIABLES);
    set_env("DATABASE_URL", None);
    set_env("DB_CHARSET", None);
    set_env("DB_COLLATION", None);

    let config = DatabaseConfig::builder()
        .url("mysql://u:p@h/db?collation=utf8mb4_general_ci")
        .charset("utf8mb4")
        .collation("utf8mb4_bin")
        .build();
    assert_eq!(config.charset.as_deref(), Some("utf8mb4"));
    assert_eq!(
        config.collation.as_deref(),
        Some("utf8mb4_bin"),
        "a value set in code is the most specific"
    );

    let config = DatabaseConfig::builder().url("mysql://u:p@h/db").build();
    assert_eq!(config.charset.as_deref(), Some("utf8mb4"));
    assert_eq!(config.collation.as_deref(), Some("utf8mb4_unicode_ci"));

    let config = DatabaseConfig::builder()
        .url("postgres://u:p@h/db")
        .charset("utf8mb4")
        .collation("utf8mb4_bin")
        .build();
    assert_eq!(config.charset, None, "Postgres keeps ignoring them");
    assert_eq!(config.collation, None);
}

// ---- Blueprint ----------------------------------------------------------

#[test]
fn create_sql_on_mysql_shows_the_blueprints_own_collation() {
    let _env = crate::env_lock::lock_env();
    let _snap = EnvSnapshot::capture(VARIABLES);
    set_env("DATABASE_URL", Some("mysql://u:p@h/db"));
    set_env("DB_CHARSET", None);
    set_env("DB_COLLATION", None);

    let sql = Blueprint::create_sql("binary_names", DbBackend::MySql, |t| {
        t.id();
        t.string("name");
        t.collation("utf8mb4_bin");
    })
    .expect("plan")
    .join("; ");
    assert!(sql.contains("COLLATE=utf8mb4_bin"), "{sql}");
    assert!(sql.contains("CHARSET=utf8mb4"), "{sql}");
    assert!(!sql.contains("utf8mb4_unicode_ci"), "{sql}");

    let sql = Blueprint::create_sql("latin_names", DbBackend::MySql, |t| {
        t.id();
        t.charset("latin1");
        t.collation("latin1_swedish_ci");
    })
    .expect("plan")
    .join("; ");
    assert!(sql.contains("CHARSET=latin1"), "{sql}");
    assert!(sql.contains("COLLATE=latin1_swedish_ci"), "{sql}");
}

#[test]
fn create_sql_on_mysql_takes_the_configured_defaults() {
    let _env = crate::env_lock::lock_env();
    let _snap = EnvSnapshot::capture(VARIABLES);
    set_env("DB_CHARSET", None);
    set_env("DB_COLLATION", None);
    set_env("DATABASE_URL", Some("mysql://u:p@h/db"));

    let sql = Blueprint::create_sql("plain", DbBackend::MySql, |t| {
        t.id();
    })
    .expect("plan")
    .join("; ");
    assert!(sql.contains("CHARSET=utf8mb4"), "{sql}");
    assert!(sql.contains("COLLATE=utf8mb4_unicode_ci"), "{sql}");

    set_env("DB_COLLATION", Some("utf8mb4_bin"));
    let sql = Blueprint::create_sql("plain", DbBackend::MySql, |t| {
        t.id();
    })
    .expect("plan")
    .join("; ");
    assert!(sql.contains("COLLATE=utf8mb4_bin"), "{sql}");
}

#[test]
fn postgres_and_sqlite_tables_ignore_the_charset_and_collation() {
    let _env = crate::env_lock::lock_env();
    let _snap = EnvSnapshot::capture(VARIABLES);
    set_env("DATABASE_URL", Some("mysql://u:p@h/db"));

    for backend in [DbBackend::Postgres, DbBackend::Sqlite] {
        let sql = Blueprint::create_sql("names", backend, |t| {
            t.id();
            t.charset("utf8mb4");
            t.collation("utf8mb4_bin");
        })
        .expect("plan")
        .join("; ");
        assert!(!sql.contains("CHARSET"), "{backend:?}: {sql}");
        assert!(!sql.contains("COLLATE"), "{backend:?}: {sql}");
    }
}

#[test]
fn a_name_that_is_not_one_is_refused_before_any_sql() {
    let error = Blueprint::create_sql("names", DbBackend::MySql, |t| {
        t.id();
        t.collation("utf8mb4_bin; DROP TABLE users");
    })
    .expect_err("an injected collation is refused");
    assert!(error.to_string().contains("collation"), "{error}");

    let error = Blueprint::create_sql("names", DbBackend::MySql, |t| {
        t.id();
        t.charset("");
    })
    .expect_err("an empty charset is refused");
    assert!(error.to_string().contains("charset"), "{error}");
}

#[tokio::test]
async fn schema_table_refuses_a_charset_or_collation() {
    let conn = super::sqlite::connect_sqlite().await;
    let manager = SchemaManager::new(&conn);
    Schema::create(&manager, "infra_names", |t| {
        t.id();
    })
    .await
    .expect("create");

    let error = Schema::table(&manager, "infra_names", |t| {
        t.collation("utf8mb4_bin");
    })
    .await
    .expect_err("a table's collation is set when it is created");
    assert!(error.to_string().contains("Schema::create"), "{error}");
}

// ---- Against MySQL -----------------------------------------------------------

fn mysql_url() -> String {
    std::env::var("MYSQL_TEST_URL").expect("set MYSQL_TEST_URL to a disposable MySQL database")
}

async fn one_string(conn: &impl ConnectionTrait, sql: &str) -> String {
    let row = conn
        .query_one_raw(Statement::from_string(DbBackend::MySql, sql.to_owned()))
        .await
        .expect("query")
        .expect("a row");
    row.try_get_by_index::<String>(0).expect("a string")
}

async fn table_collation(conn: &impl ConnectionTrait, table: &str) -> String {
    one_string(
        conn,
        &format!(
            "SELECT TABLE_COLLATION FROM information_schema.TABLES \
             WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = '{table}'"
        ),
    )
    .await
}

#[tokio::test]
#[serial_test::serial]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn mysql_every_connection_uses_the_configured_collation() {
    let _env = crate::env_lock::lock_env_async().await;
    let _snap = EnvSnapshot::capture(VARIABLES);
    set_env("DB_CHARSET", None);
    set_env("DB_COLLATION", None);
    let url = mysql_url();

    let default = DbConnection::connect(&DatabaseConfig::builder().url(url.clone()).build())
        .await
        .expect("connect");
    assert_eq!(
        one_string(default.inner(), "SELECT @@collation_connection").await,
        "utf8mb4_unicode_ci"
    );
    assert_eq!(
        one_string(default.inner(), "SELECT @@character_set_connection").await,
        "utf8mb4"
    );

    set_env("DB_COLLATION", Some("utf8mb4_bin"));
    let name = format!("infra_gaps_{}", uuid::Uuid::new_v4().simple());
    ConnectionRegistry::register(&name, DatabaseConfig::builder().url(url).build())
        .await
        .expect("register a named connection");
    let named = ConnectionRegistry::get(&name).await.expect("named");
    assert_eq!(
        one_string(named.inner(), "SELECT @@collation_connection").await,
        "utf8mb4_bin",
        "a named connection sends the configured collation too"
    );
}

#[tokio::test]
#[serial_test::serial]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn mysql_schema_create_takes_the_configured_collation() {
    let _env = crate::env_lock::lock_env_async().await;
    let _snap = EnvSnapshot::capture(VARIABLES);
    let url = mysql_url();
    set_env("DATABASE_URL", Some(&url));
    set_env("DB_CHARSET", None);
    set_env("DB_COLLATION", None);
    let conn = DbConnection::connect(&DatabaseConfig::from_env())
        .await
        .expect("connect");
    let manager = SchemaManager::new(conn.inner());
    Schema::drop_if_exists(&manager, "infra_gaps_default_collation")
        .await
        .expect("clean");
    Schema::drop_if_exists(&manager, "infra_gaps_binary_collation")
        .await
        .expect("clean");

    Schema::create(&manager, "infra_gaps_default_collation", |t| {
        t.id();
        t.string("name");
    })
    .await
    .expect("create with the configured collation");
    Schema::create(&manager, "infra_gaps_binary_collation", |t| {
        t.id();
        t.string("name");
        t.collation("utf8mb4_bin");
    })
    .await
    .expect("create with the blueprint's collation");

    let default = table_collation(conn.inner(), "infra_gaps_default_collation").await;
    let binary = table_collation(conn.inner(), "infra_gaps_binary_collation").await;
    Schema::drop(&manager, "infra_gaps_default_collation")
        .await
        .expect("drop");
    Schema::drop(&manager, "infra_gaps_binary_collation")
        .await
        .expect("drop");
    assert_eq!(default, "utf8mb4_unicode_ci");
    assert_eq!(binary, "utf8mb4_bin");
}

/// One migration that writes the collation its own connection speaks, so a
/// test can read back what the `migrate` command connected with.
struct RecordCollation;

impl sea_orm_migration::MigrationName for RecordCollation {
    fn name(&self) -> &str {
        "m20261009_000001_infra_gaps_record_collation"
    }
}

#[async_trait::async_trait]
impl sea_orm_migration::MigrationTrait for RecordCollation {
    async fn up(&self, manager: &SchemaManager) -> Result<(), sea_orm::DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "CREATE TABLE infra_gaps_migrator_collation (value VARCHAR(64) NOT NULL)",
        )
        .await?;
        db.execute_unprepared(
            "INSERT INTO infra_gaps_migrator_collation (value) SELECT @@collation_connection",
        )
        .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), sea_orm::DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS infra_gaps_migrator_collation")
            .await?;
        Ok(())
    }
}

struct RecordingMigrator;

#[async_trait::async_trait]
impl sea_orm_migration::MigratorTrait for RecordingMigrator {
    fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
        vec![Box::new(RecordCollation)]
    }

    fn migration_table_name() -> sea_orm::DynIden {
        use sea_orm::sea_query::IntoIden;
        sea_orm::sea_query::Alias::new("infra_gaps_migrator_migrations").into_iden()
    }
}

/// PAR-135: every connection the framework opens to the database sends the
/// configured character set and collation, the one the `migrate` command
/// runs on included. The migration records `@@collation_connection` as it
/// runs; a migrate connection opened outside the pool builder would record
/// the server's default instead of `utf8mb4_bin`. The command loads the
/// environment before the runtime, as `#[suprnova::main]` does, so this is
/// a plain test that builds its own runtime.
#[test]
#[serial_test::serial]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
fn mysql_the_migrate_command_connects_with_the_configured_collation() {
    let _env = crate::env_lock::lock_env();
    let _snap = EnvSnapshot::capture(&["DATABASE_URL", "DB_CHARSET", "DB_COLLATION", "APP_ENV"]);
    let url = mysql_url();
    set_env("DATABASE_URL", Some(&url));
    set_env("DB_CHARSET", None);
    set_env("DB_COLLATION", Some("utf8mb4_bin"));
    set_env("APP_ENV", Some("testing"));
    suprnova::boot::load_env().expect("the environment loads before the runtime");

    let recorded = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(async {
            let probe = DbConnection::connect(&DatabaseConfig::builder().url(url.clone()).build())
                .await
                .expect("connect");
            let cleanup = [
                "DROP TABLE IF EXISTS infra_gaps_migrator_collation",
                "DROP TABLE IF EXISTS infra_gaps_migrator_migrations",
            ];
            for sql in cleanup {
                probe
                    .inner()
                    .execute_unprepared(sql)
                    .await
                    .expect("a clean start");
            }

            suprnova::Application::new()
                .migrations::<RecordingMigrator>()
                .run_with_args(["app", "migrate"])
                .await
                .expect("the migrate command runs");

            let recorded = one_string(
                probe.inner(),
                "SELECT value FROM infra_gaps_migrator_collation",
            )
            .await;
            for sql in cleanup {
                probe
                    .inner()
                    .execute_unprepared(sql)
                    .await
                    .expect("cleanup");
            }
            recorded
        });
    assert_eq!(
        recorded, "utf8mb4_bin",
        "the migrate command's connection sends the configured collation"
    );
}
