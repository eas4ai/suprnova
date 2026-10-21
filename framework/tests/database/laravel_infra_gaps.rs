//! Registered migrations (PAR-144): `Application::load_migrations_from`,
//! `register_migrations!` and the two-factor list, driven through every
//! migrate command of a real `Application` in a child process of this test
//! binary, on a temporary SQLite file.
//!
//! The registration below holds for every application this binary runs,
//! so its migration has a name and a table no other test uses.

use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use sea_orm::{ConnectionTrait, Database, DatabaseConnection, Statement};
use sea_orm_migration::prelude::*;

/// A migration that creates `table` and records itself as `name`.
struct CreateTable {
    name: &'static str,
    table: &'static str,
}

impl MigrationName for CreateTable {
    fn name(&self) -> &str {
        self.name
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateTable {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Alias::new(self.table))
                    .col(
                        ColumnDef::new(Alias::new("id"))
                            .integer()
                            .not_null()
                            .primary_key(),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Alias::new(self.table)).to_owned())
            .await
    }
}

const USERS: &str = "m_infra_gaps_000001_users";
const WIDGETS: &str = "m_infra_gaps_000002_widgets";
const REGISTERED: &str = "m_infra_gaps_000003_registered_gadgets";

fn users() -> Box<dyn MigrationTrait> {
    Box::new(CreateTable {
        name: USERS,
        table: "infra_gaps_users",
    })
}

/// The application's own migrator: the `users` migration and nothing else.
struct UsersOnly;

impl MigratorTrait for UsersOnly {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![users()]
    }
}

/// What the application loads: `users` again, which must run once, and
/// `widgets`.
fn loaded() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        users(),
        Box::new(CreateTable {
            name: WIDGETS,
            table: "infra_gaps_widgets",
        }),
    ]
}

/// What a crate registers with `register_migrations!`.
fn registered() -> Vec<Box<dyn MigrationTrait>> {
    vec![Box::new(CreateTable {
        name: REGISTERED,
        table: "infra_gaps_registered_gadgets",
    })]
}

suprnova::register_migrations!("laravel-infra-gaps-test", registered);

/// The arguments of the command [`migrate_child`] runs, as JSON.
const MIGRATE_CHILD: &str = "SUPRNOVA_INFRA_MIGRATE_CHILD";

/// Run `app <args>` for an application whose migrator is [`UsersOnly`] and
/// that loads [`loaded`], in a child process, against the SQLite file at
/// `path`.
fn app(path: &Path, args: &[&str]) -> Output {
    child(path, args, &[])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run the child")
}

fn child(path: &Path, args: &[&str], env: &[(&str, &str)]) -> Command {
    let mut command = Command::new(std::env::current_exe().expect("the test binary"));
    command
        .args([
            "--exact",
            "laravel_infra_gaps::migrate_child",
            "--nocapture",
        ])
        .env(
            MIGRATE_CHILD,
            serde_json::to_string(args).expect("the arguments as JSON"),
        )
        .env("DATABASE_URL", url(path))
        .env("APP_ENV", "testing");
    for (key, value) in env {
        command.env(key, value);
    }
    command
}

fn url(path: &Path) -> String {
    format!("sqlite://{}?mode=rwc", path.display())
}

/// The child half of [`app`].
#[test]
fn migrate_child() {
    let Ok(args) = std::env::var(MIGRATE_CHILD) else {
        return;
    };
    let args: Vec<String> = serde_json::from_str(&args).expect("the arguments");
    suprnova::boot::load_env().expect("load the configuration");
    let outcome = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a runtime")
        .block_on(async {
            suprnova::Application::new()
                .migrations::<UsersOnly>()
                .load_migrations_from(loaded)
                .run_with_args(std::iter::once("app".to_owned()).chain(args))
                .await
        });
    if let Err(error) = outcome {
        eprintln!("{}", error.message());
        std::process::exit(1);
    }
}

fn printed(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

async fn connect(path: &Path) -> DatabaseConnection {
    Database::connect(url(path)).await.expect("connect")
}

/// The names the migration ledger records, in the order they were applied.
async fn ledger(db: &DatabaseConnection) -> Vec<String> {
    db.query_all_raw(Statement::from_string(
        db.get_database_backend(),
        "SELECT version FROM seaql_migrations ORDER BY applied_at, rowid",
    ))
    .await
    .expect("read the ledger")
    .iter()
    .map(|row| row.try_get::<String>("", "version").expect("a version"))
    .collect()
}

async fn has_table(db: &DatabaseConnection, table: &str) -> bool {
    !db.query_all_raw(Statement::from_string(
        db.get_database_backend(),
        format!("SELECT name FROM sqlite_master WHERE type = 'table' AND name = '{table}'"),
    ))
    .await
    .expect("read the catalog")
    .is_empty()
}

#[tokio::test]
async fn migrate_runs_the_loaded_and_registered_migrations_once_each() {
    let dir = tempfile::tempdir().expect("a directory");
    let path = dir.path().join("app.sqlite");

    let output = app(&path, &["migrate"]);
    assert!(output.status.success(), "{}", printed(&output));

    let db = connect(&path).await;
    assert!(has_table(&db, "infra_gaps_users").await);
    assert!(has_table(&db, "infra_gaps_widgets").await);
    assert!(
        has_table(&db, "infra_gaps_registered_gadgets").await,
        "the register_migrations! registration ran"
    );
    assert_eq!(
        ledger(&db).await,
        [USERS, WIDGETS, REGISTERED],
        "the application's own list first, each name once"
    );

    let again = app(&path, &["migrate"]);
    assert!(
        again.status.success(),
        "a second migrate finds nothing to run: {}",
        printed(&again)
    );
    assert_eq!(ledger(&db).await, [USERS, WIDGETS, REGISTERED]);
}

#[test]
fn migrate_status_lists_the_loaded_migrations() {
    let dir = tempfile::tempdir().expect("a directory");
    let path = dir.path().join("app.sqlite");

    let before = app(&path, &["migrate:status"]);
    let pending = String::from_utf8_lossy(&before.stdout).into_owned();
    assert!(before.status.success(), "{}", printed(&before));
    for name in [USERS, WIDGETS, REGISTERED] {
        let line = pending
            .lines()
            .find(|line| line.contains(name))
            .unwrap_or_else(|| panic!("`{name}` is not listed: {pending}"));
        assert!(line.contains("Pending"), "{line}");
    }

    let migrated = app(&path, &["migrate"]);
    assert!(migrated.status.success(), "{}", printed(&migrated));
    let after = app(&path, &["migrate:status"]);
    let applied = String::from_utf8_lossy(&after.stdout).into_owned();
    for name in [USERS, WIDGETS, REGISTERED] {
        let line = applied
            .lines()
            .find(|line| line.contains(name))
            .unwrap_or_else(|| panic!("`{name}` is not listed: {applied}"));
        assert!(line.contains("Applied"), "{line}");
    }
    assert_eq!(
        applied.matches(USERS).count(),
        1,
        "a name both lists hold is listed once: {applied}"
    );
}

#[tokio::test]
async fn rollback_and_fresh_reach_the_loaded_migrations() {
    let dir = tempfile::tempdir().expect("a directory");
    let path = dir.path().join("app.sqlite");
    assert!(app(&path, &["migrate"]).status.success());

    let rolled = app(&path, &["migrate:rollback", "2"]);
    assert!(rolled.status.success(), "{}", printed(&rolled));
    let db = connect(&path).await;
    assert_eq!(ledger(&db).await, [USERS]);
    assert!(!has_table(&db, "infra_gaps_widgets").await);
    assert!(!has_table(&db, "infra_gaps_registered_gadgets").await);

    let fresh = app(&path, &["migrate:fresh"]);
    assert!(fresh.status.success(), "{}", printed(&fresh));
    assert_eq!(ledger(&db).await, [USERS, WIDGETS, REGISTERED]);
    assert!(has_table(&db, "infra_gaps_widgets").await);
}

#[test]
fn schema_dump_records_the_loaded_migrations() {
    let dir = tempfile::tempdir().expect("a directory");
    let path = dir.path().join("app.sqlite");
    assert!(app(&path, &["migrate"]).status.success());
    let dump = dir.path().join("schema.sql");

    let output = app(
        &path,
        &[
            "schema:dump",
            "--path",
            dump.to_str().expect("a UTF-8 path"),
        ],
    );

    assert!(output.status.success(), "{}", printed(&output));
    let sql = std::fs::read_to_string(&dump).expect("the dump");
    for name in [USERS, WIDGETS, REGISTERED] {
        assert!(sql.contains(name), "`{name}` is not in the dump:\n{sql}");
    }
    assert!(sql.contains("infra_gaps_widgets"));
}

/// The migration `serve` runs before it starts the server. The child is
/// stopped once the tables exist: whether the server then comes up is not
/// this test's business.
#[tokio::test]
async fn the_migration_on_boot_runs_the_loaded_migrations() {
    let dir = tempfile::tempdir().expect("a directory");
    let path = dir.path().join("app.sqlite");
    let mut serving = child(
        &path,
        &["serve"],
        &[("SERVER_HOST", "127.0.0.1"), ("SERVER_PORT", "0")],
    )
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .expect("start the child");

    let deadline = Instant::now() + Duration::from_secs(60);
    let mut migrated = false;
    while Instant::now() < deadline {
        if path.exists()
            && let Ok(db) = Database::connect(url(&path)).await
            && let Ok(rows) = db
                .query_all_raw(Statement::from_string(
                    db.get_database_backend(),
                    "SELECT version FROM seaql_migrations",
                ))
                .await
            && rows.len() == 3
        {
            migrated = true;
            break;
        }
        if serving.try_wait().expect("poll the child").is_some() {
            // It ended: read the ledger once more below.
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let _ = serving.kill();
    let _ = serving.wait();

    let db = connect(&path).await;
    assert!(
        migrated || ledger(&db).await.len() == 3,
        "the boot migration did not run every migration: {:?}",
        ledger(&db).await
    );
    assert_eq!(ledger(&db).await, [USERS, WIDGETS, REGISTERED]);
}

#[test]
fn the_two_factor_migrations_are_one_list() {
    let names: Vec<String> = suprnova::auth_flows::two_factor::migrations()
        .iter()
        .map(|migration| migration.name().to_owned())
        .collect();
    let expected: Vec<String> = [
        Box::new(suprnova::auth_flows::two_factor::migration::Migration) as Box<dyn MigrationTrait>,
        Box::new(suprnova::auth_flows::two_factor::migration_replay::Migration),
        Box::new(suprnova::auth_flows::two_factor::migration_attempts::Migration),
        Box::new(suprnova::auth_flows::two_factor::migration_rotation::Migration),
    ]
    .iter()
    .map(|migration| migration.name().to_owned())
    .collect();
    assert_eq!(names, expected);
}
