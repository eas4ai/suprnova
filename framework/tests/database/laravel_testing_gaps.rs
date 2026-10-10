//! Database tests on the configured connection (PAR-176):
//! `TestDatabase::refresh`, `migrate`, `refresh_lazily` and `seed`, and the
//! `refresh` and `seed` options of `#[suprnova_test]`.
//!
//! The SQLite tests point `DATABASE_URL` at a new temporary file, under this
//! binary's environment lock, so none of them can pass on an in-memory
//! database. The `postgres_` and `mysql_` tests prove the same contract on a
//! real engine and run only when PG_TEST_URL or MYSQL_TEST_URL names a
//! disposable database:
//!
//! ```text
//! PG_TEST_URL=postgres://... MYSQL_TEST_URL=mysql://... \
//!   cargo nextest run -p suprnova --test database --run-ignored only \
//!   -E 'test(/laravel_testing_gaps::(postgres|mysql)_/)'
//! ```
//!
//! Every migrator here has a migration table of its own, so two engine
//! tests on one database never create the same ledger at once.

use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};
use sea_orm_migration::prelude::*;
use suprnova::database::{DatabaseConfig, DbConnection};
use suprnova::testing::TestDatabase;
use suprnova::{DB, FrameworkError, PrunedMigration, SchemaDump, Seeder, async_trait, seed};

use crate::env_snapshot::{EnvSnapshot, set_env};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// A migration that creates `table` and counts the times its `up` ran.
///
/// The table is created with `IF NOT EXISTS`, so a test can delete the
/// ledger rows and still tell, by the count, whether the migrations ran
/// again. While `fail` holds, `up` fails instead.
struct TableMigration {
    name: &'static str,
    table: &'static str,
    runs: &'static AtomicUsize,
    fail: Option<&'static AtomicBool>,
}

impl MigrationName for TableMigration {
    fn name(&self) -> &str {
        self.name
    }
}

#[async_trait::async_trait]
impl MigrationTrait for TableMigration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if self.fail.is_some_and(|fail| fail.load(Ordering::SeqCst)) {
            return Err(DbErr::Custom(format!(
                "the migration of {} failed on purpose",
                self.table
            )));
        }
        self.runs.fetch_add(1, Ordering::SeqCst);
        manager
            .create_table(
                Table::create()
                    .table(Alias::new(self.table))
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Alias::new("id"))
                            .integer()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Alias::new("label")).string().not_null())
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

/// The table the seeders below write to.
const SEEDED: &str = "ltg_seeded";

/// Counts the runs of the `ltg_seeded` migration, which every migrator that
/// is seeded lists. The count itself is never asserted.
static SEEDED_RUNS: AtomicUsize = AtomicUsize::new(0);

fn seeded_table(name: &'static str) -> TableMigration {
    TableMigration {
        name,
        table: SEEDED,
        runs: &SEEDED_RUNS,
        fail: None,
    }
}

/// A migrator named `$migrator`, with the migration table `$ledger` and the
/// listed migrations.
macro_rules! migrator {
    ($migrator:ident, $ledger:literal, [$($migration:expr),+ $(,)?]) => {
        struct $migrator;

        #[async_trait::async_trait]
        impl MigratorTrait for $migrator {
            fn migrations() -> Vec<Box<dyn MigrationTrait>> {
                vec![$(Box::new($migration) as Box<dyn MigrationTrait>),+]
            }

            fn migration_table_name() -> DynIden {
                Alias::new($ledger).into_iden()
            }
        }
    };
}

/// Writes one row to `ltg_seeded` through the connection the container
/// hands out, which is the helper's.
struct LabelSeeder;

#[async_trait]
impl Seeder for LabelSeeder {
    fn name() -> &'static str {
        "LaravelTestingGapsLabelSeeder"
    }

    async fn run() -> Result<(), FrameworkError> {
        DB::statement(
            "INSERT INTO ltg_seeded (id, label) VALUES (1, 'seeded')",
            [],
        )
        .await
        .map(drop)
    }
}

/// The root seeder of the registry test: one row with the label `root`.
struct RootSeeder;

#[async_trait]
impl Seeder for RootSeeder {
    fn name() -> &'static str {
        "LaravelTestingGapsRootSeeder"
    }

    async fn run() -> Result<(), FrameworkError> {
        DB::statement("INSERT INTO ltg_seeded (id, label) VALUES (2, 'root')", [])
            .await
            .map(drop)
    }
}

/// A seeder that fails, so `seed` has an error to return.
struct FailingSeeder;

#[async_trait]
impl Seeder for FailingSeeder {
    fn name() -> &'static str {
        "LaravelTestingGapsFailingSeeder"
    }

    async fn run() -> Result<(), FrameworkError> {
        Err(FrameworkError::internal("the seeder failed on purpose"))
    }
}

/// `DATABASE_URL` naming a new SQLite file while the value lives, under the
/// binary's environment lock. Declare it first in a test, so it drops last:
/// the variable is restored before the lock is released.
struct ConfiguredFile {
    url: String,
    _restore: EnvSnapshot,
    _dir: tempfile::TempDir,
    _env: tokio::sync::MutexGuard<'static, ()>,
}

async fn configured_file() -> ConfiguredFile {
    let env = crate::env_lock::lock_env_async().await;
    let dir = tempfile::tempdir().expect("create a temporary directory");
    let url = format!("sqlite://{}", dir.path().join("app.db").display());
    let restore = EnvSnapshot::capture(&["DATABASE_URL"]);
    set_env("DATABASE_URL", Some(&url));
    ConfiguredFile {
        url,
        _restore: restore,
        _dir: dir,
        _env: env,
    }
}

/// A connection of its own to the database at `url`, outside every helper,
/// to see what the database holds as other connections see it.
async fn outside(url: &str) -> DbConnection {
    let config = DatabaseConfig::builder()
        .url(url)
        .max_connections(1)
        .logging(false)
        .build();
    DbConnection::connect(&config)
        .await
        .expect("connect to the database outside the helper")
}

async fn has_table(conn: &DbConnection, table: &str) -> bool {
    SchemaManager::new(conn.inner())
        .has_table(table)
        .await
        .expect("read the database's tables")
}

async fn count(conn: &DatabaseConnection, table: &str) -> i64 {
    let backend = conn.get_database_backend();
    let row = conn
        .query_one_raw(Statement::from_string(
            backend,
            format!("SELECT COUNT(*) AS n FROM {table}"),
        ))
        .await
        .expect("count the rows")
        .expect("COUNT returns one row");
    row.try_get::<i64>("", "n").expect("read the count")
}

async fn insert(table: &str, id: i64, label: &str) -> Result<bool, FrameworkError> {
    DB::statement(
        &format!("INSERT INTO {table} (id, label) VALUES ({id}, '{label}')"),
        [],
    )
    .await
}

// ---------------------------------------------------------------------------
// refresh
// ---------------------------------------------------------------------------

static FILE_RUNS: AtomicUsize = AtomicUsize::new(0);
migrator!(
    FileMigrator,
    "ltg_file_ledger",
    [TableMigration {
        name: "m_ltg_000001_file",
        table: "ltg_file",
        runs: &FILE_RUNS,
        fail: None,
    }]
);

/// Falsifier: `refresh` migrates an in-memory SQLite database instead of
/// the configured one. The migration lands in the file `DATABASE_URL`
/// names, and a row the test writes is the test transaction's: the
/// container's connection and the helper's see it, another connection
/// does not.
#[tokio::test]
async fn refresh_migrates_the_configured_database_and_holds_the_test_in_a_transaction() {
    let file = configured_file().await;
    let db = TestDatabase::refresh::<FileMigrator>()
        .await
        .expect("refresh the configured database");

    let outside = outside(&file.url).await;
    assert!(
        has_table(&outside, "ltg_file").await,
        "the migrations ran on the file DATABASE_URL names"
    );

    insert("ltg_file", 1, "a")
        .await
        .expect("write through the container's connection");
    assert_eq!(
        count(db.conn(), "ltg_file").await,
        1,
        "the helper's connection and the container's are one transaction"
    );
    assert_eq!(
        count(outside.inner(), "ltg_file").await,
        0,
        "another connection does not see the test's uncommitted row"
    );
}

static NEXT_RUNS: AtomicUsize = AtomicUsize::new(0);
migrator!(
    NextMigrator,
    "ltg_next_ledger",
    [TableMigration {
        name: "m_ltg_000002_next",
        table: "ltg_next",
        runs: &NEXT_RUNS,
        fail: None,
    }]
);

/// Falsifier: a row one `refresh` test writes is visible to the next. The
/// rows survive neither a committed `DB::transaction` (a savepoint inside
/// the test transaction) nor the helper; an inner transaction that fails
/// rolls back to its savepoint and leaves the test's earlier rows.
#[tokio::test]
async fn a_row_one_refresh_test_writes_is_gone_for_the_next() {
    let file = configured_file().await;
    {
        let db = TestDatabase::refresh::<NextMigrator>()
            .await
            .expect("refresh for the first test");
        insert("ltg_next", 1, "plain").await.expect("write a row");
        DB::transaction(|_tx| {
            Box::pin(async move {
                insert("ltg_next", 2, "committed").await?;
                Ok::<(), FrameworkError>(())
            })
        })
        .await
        .expect("an inner transaction commits");
        let failed = DB::transaction(|_tx| {
            Box::pin(async move {
                insert("ltg_next", 3, "rolled back").await?;
                Err::<(), FrameworkError>(FrameworkError::internal("roll back on purpose"))
            })
        })
        .await;
        assert!(failed.is_err(), "the inner transaction failed");
        assert_eq!(
            count(db.conn(), "ltg_next").await,
            2,
            "the failed inner transaction rolled back to its savepoint only"
        );
    }

    let db = TestDatabase::refresh::<NextMigrator>()
        .await
        .expect("refresh for the next test");
    assert_eq!(
        count(db.conn(), "ltg_next").await,
        0,
        "the next test starts without the rows of the first"
    );
    let outside = outside(&file.url).await;
    assert_eq!(count(outside.inner(), "ltg_next").await, 0);
}

static ONCE_RUNS: AtomicUsize = AtomicUsize::new(0);
migrator!(
    OnceMigrator,
    "ltg_once_ledger",
    [TableMigration {
        name: "m_ltg_000003_once",
        table: "ltg_once",
        runs: &ONCE_RUNS,
        fail: None,
    }]
);

/// Falsifier: the migrations run again for a second `refresh` test in the
/// same process. Between the two, the ledger rows are deleted, so a second
/// migration would run the migration's `up` again and count it.
#[tokio::test]
async fn refresh_migrates_once_per_process() {
    let file = configured_file().await;
    drop(
        TestDatabase::refresh::<OnceMigrator>()
            .await
            .expect("the first refresh"),
    );
    assert_eq!(
        ONCE_RUNS.load(Ordering::SeqCst),
        1,
        "the first refresh migrated"
    );

    let outside = outside(&file.url).await;
    outside
        .inner()
        .execute_unprepared("DELETE FROM ltg_once_ledger")
        .await
        .expect("forget the ledger rows");

    let db = TestDatabase::refresh::<OnceMigrator>()
        .await
        .expect("the second refresh");
    assert_eq!(
        ONCE_RUNS.load(Ordering::SeqCst),
        1,
        "the second refresh in this process did not migrate again"
    );
    assert_eq!(count(db.conn(), "ltg_once").await, 0);
}

static PARALLEL_RUNS: AtomicUsize = AtomicUsize::new(0);
migrator!(
    ParallelMigrator,
    "ltg_parallel_ledger",
    [TableMigration {
        name: "m_ltg_000004_parallel",
        table: "ltg_parallel",
        runs: &PARALLEL_RUNS,
        fail: None,
    }]
);

/// Tests that run at once in one process share the one migration: four
/// threads, each with a runtime of its own as `#[tokio::test]` gives it,
/// refresh the same database together and the migration runs once.
#[tokio::test]
async fn parallel_refresh_tests_in_one_process_share_one_migration() {
    let _file = configured_file().await;
    let barrier = std::sync::Barrier::new(4);
    let counts: Vec<i64> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..4)
            .map(|_| {
                scope.spawn(|| {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("a runtime for the thread");
                    barrier.wait();
                    runtime.block_on(async {
                        let db = TestDatabase::refresh::<ParallelMigrator>()
                            .await
                            .expect("refresh from a parallel test");
                        count(db.conn(), "ltg_parallel").await
                    })
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("the parallel test passed"))
            .collect()
    });
    assert_eq!(counts, vec![0, 0, 0, 0]);
    assert_eq!(
        PARALLEL_RUNS.load(Ordering::SeqCst),
        1,
        "four parallel refreshes migrated once"
    );
}

static RETRY_RUNS: AtomicUsize = AtomicUsize::new(0);
static RETRY_FAILS: AtomicBool = AtomicBool::new(true);
migrator!(
    RetryMigrator,
    "ltg_retry_ledger",
    [TableMigration {
        name: "m_ltg_000005_retry",
        table: "ltg_retry",
        runs: &RETRY_RUNS,
        fail: Some(&RETRY_FAILS),
    }]
);

/// A migration that fails is an error of `refresh`, and is not remembered
/// as done: the next `refresh` of the process migrates again.
#[tokio::test]
async fn a_failed_refresh_migration_is_returned_and_tried_again() {
    let file = configured_file().await;
    RETRY_FAILS.store(true, Ordering::SeqCst);
    let error = TestDatabase::refresh::<RetryMigrator>()
        .await
        .err()
        .expect("a failing migration fails refresh");
    assert!(
        error.to_string().contains("failed on purpose"),
        "the error names the migration's failure: {error}"
    );

    RETRY_FAILS.store(false, Ordering::SeqCst);
    let db = TestDatabase::refresh::<RetryMigrator>()
        .await
        .expect("the next refresh migrates again");
    assert_eq!(RETRY_RUNS.load(Ordering::SeqCst), 1);
    assert_eq!(count(db.conn(), "ltg_retry").await, 0);
    assert!(has_table(&outside(&file.url).await, "ltg_retry").await);
}

/// `refresh` needs `DATABASE_URL`: unset, it returns an error that names
/// the variable instead of falling back to the development database; set
/// to an in-memory SQLite database, it returns an error that points to
/// `fresh`, which is the in-memory helper.
#[tokio::test]
async fn refresh_refuses_an_unset_or_in_memory_database_url() {
    let _env = crate::env_lock::lock_env_async().await;
    let _restore = EnvSnapshot::capture(&["DATABASE_URL"]);

    set_env("DATABASE_URL", None);
    let error = TestDatabase::refresh::<FileMigrator>()
        .await
        .err()
        .expect("refresh without DATABASE_URL fails");
    assert!(error.to_string().contains("DATABASE_URL"), "got: {error}");

    for url in ["sqlite::memory:", "sqlite://:memory:"] {
        set_env("DATABASE_URL", Some(url));
        let error = TestDatabase::refresh::<FileMigrator>()
            .await
            .err()
            .expect("refresh on an in-memory database fails");
        assert!(
            error.to_string().contains("TestDatabase::fresh"),
            "got: {error}"
        );
        let error = TestDatabase::migrate::<FileMigrator>()
            .await
            .err()
            .expect("migrate on an in-memory database fails");
        assert!(
            error.to_string().contains("TestDatabase::fresh"),
            "got: {error}"
        );
    }
}

// ---------------------------------------------------------------------------
// migrate
// ---------------------------------------------------------------------------

static MIGRATE_RUNS: AtomicUsize = AtomicUsize::new(0);
migrator!(
    MigrateMigrator,
    "ltg_migrate_ledger",
    [TableMigration {
        name: "m_ltg_000006_migrate",
        table: "ltg_migrate",
        runs: &MIGRATE_RUNS,
        fail: None,
    }]
);

/// Falsifier: `migrate` leaves its tables after the helper drops. The
/// migrations run on the configured database with no transaction around
/// the test, so another connection sees the rows; dropping the helper
/// rolls the migrations back, and the table and the ledger the helper
/// created are gone.
#[tokio::test]
async fn migrate_rolls_its_migrations_back_when_the_helper_drops() {
    let file = configured_file().await;
    let outside = outside(&file.url).await;
    {
        let db = TestDatabase::migrate::<MigrateMigrator>()
            .await
            .expect("migrate the configured database");
        assert!(has_table(&outside, "ltg_migrate").await);
        insert("ltg_migrate", 1, "a").await.expect("write a row");
        assert_eq!(count(db.conn(), "ltg_migrate").await, 1);
        assert_eq!(
            count(outside.inner(), "ltg_migrate").await,
            1,
            "migrate holds no transaction around the test"
        );
    }
    assert!(
        !has_table(&outside, "ltg_migrate").await,
        "the helper rolled its migration back when it dropped"
    );
    assert!(
        !has_table(&outside, "ltg_migrate_ledger").await,
        "the helper dropped the ledger it created"
    );
}

static KEPT_RUNS: AtomicUsize = AtomicUsize::new(0);
migrator!(
    KeptMigrator,
    "ltg_kept_ledger",
    [TableMigration {
        name: "m_ltg_000007_kept",
        table: "ltg_kept",
        runs: &KEPT_RUNS,
        fail: None,
    }]
);

/// `migrate` rolls back the migrations it ran and no others: on a database
/// a `refresh` already migrated it runs nothing and leaves the table. A
/// `refresh` after it still finds its tables.
#[tokio::test]
async fn migrate_rolls_back_only_the_migrations_it_ran() {
    let file = configured_file().await;
    drop(
        TestDatabase::refresh::<KeptMigrator>()
            .await
            .expect("refresh migrates first"),
    );
    drop(
        TestDatabase::migrate::<KeptMigrator>()
            .await
            .expect("migrate finds nothing to run"),
    );
    let outside = outside(&file.url).await;
    assert!(has_table(&outside, "ltg_kept").await, "the table stays");
    assert_eq!(count(outside.inner(), "ltg_kept_ledger").await, 1);

    let db = TestDatabase::refresh::<KeptMigrator>()
        .await
        .expect("refresh after migrate");
    assert_eq!(count(db.conn(), "ltg_kept").await, 0);
}

static PARTIAL_RUNS: AtomicUsize = AtomicUsize::new(0);
static ALWAYS_FAILS: AtomicBool = AtomicBool::new(true);
migrator!(
    PartialMigrator,
    "ltg_partial_ledger",
    [
        TableMigration {
            name: "m_ltg_000008_partial_first",
            table: "ltg_partial_first",
            runs: &PARTIAL_RUNS,
            fail: None,
        },
        TableMigration {
            name: "m_ltg_000009_partial_second",
            table: "ltg_partial_second",
            runs: &PARTIAL_RUNS,
            fail: Some(&ALWAYS_FAILS),
        },
    ]
);

/// A migration that fails is the error of `migrate`, and the migrations
/// that ran before it are rolled back, so the failure leaves no table.
#[tokio::test]
async fn a_failed_migrate_rolls_back_the_migrations_that_ran() {
    let file = configured_file().await;
    let error = TestDatabase::migrate::<PartialMigrator>()
        .await
        .err()
        .expect("the second migration fails migrate");
    assert!(
        error.to_string().contains("failed on purpose"),
        "got: {error}"
    );
    assert_eq!(
        PARTIAL_RUNS.load(Ordering::SeqCst),
        1,
        "the first migration ran"
    );
    let outside = outside(&file.url).await;
    assert!(
        !has_table(&outside, "ltg_partial_first").await,
        "the migration that ran was rolled back"
    );
    assert!(
        !has_table(&outside, "ltg_partial_ledger").await,
        "the ledger the failed migrate created was dropped"
    );
}

/// The database directory moved to a new temporary one while the value
/// lives, so a test can write the schema dump the helpers read, at
/// `database_path("schema/sqlite-schema.sql")`, without touching the
/// project's. Declare it after [`configured_file`], whose lock covers it,
/// so it drops first.
struct DumpDirectory {
    previous: PathBuf,
    dir: tempfile::TempDir,
}

impl DumpDirectory {
    fn new() -> Self {
        let previous = suprnova::database_path("");
        let dir = tempfile::tempdir().expect("create a temporary database directory");
        suprnova::use_database_path(dir.path());
        Self { previous, dir }
    }

    /// Writes the SQLite dump of `M` where the helpers read it: `M`'s
    /// migrations run on a database of their own, which is dumped with its
    /// ledger rows. Returns the dump's path.
    async fn write<M: MigratorTrait>(&self) -> PathBuf {
        let source = format!("sqlite://{}", self.dir.path().join("source.db").display());
        let conn = outside(&source).await;
        M::up(conn.inner(), None)
            .await
            .expect("migrate the database the dump is taken from");
        let path = suprnova::database_path("schema/sqlite-schema.sql");
        SchemaDump::dump::<M>(&source, &path)
            .await
            .expect("dump the schema and the ledger");
        path
    }
}

impl Drop for DumpDirectory {
    fn drop(&mut self) {
        suprnova::use_database_path(&self.previous);
    }
}

static DUMPED_RUNS: AtomicUsize = AtomicUsize::new(0);
migrator!(
    DumpedMigrator,
    "ltg_dumped_ledger",
    [
        TableMigration {
            name: "m_ltg_000016_dumped_first",
            table: "ltg_dumped_first",
            runs: &DUMPED_RUNS,
            fail: None,
        },
        TableMigration {
            name: "m_ltg_000017_dumped_second",
            table: "ltg_dumped_second",
            runs: &DUMPED_RUNS,
            fail: None,
        },
    ]
);

/// Falsifier: `migrate` leaves its tables after the helper drops, here on
/// an empty database beside a schema dump that records every migration of
/// the migrator. `migrate` does not load the dump: it runs every migration
/// and rolls every one back, so neither the tables nor the ledger stay, and
/// a row the test wrote with no transaction around it is gone for the next
/// test.
#[tokio::test]
async fn migrate_beside_a_schema_dump_leaves_no_table_behind() {
    let file = configured_file().await;
    let dump = DumpDirectory::new();
    let path = dump.write::<DumpedMigrator>().await;
    let sql = std::fs::read_to_string(&path).expect("read the dump");
    for name in ["m_ltg_000016_dumped_first", "m_ltg_000017_dumped_second"] {
        assert!(sql.contains(name), "the dump records {name}");
    }

    let outside = outside(&file.url).await;
    {
        let _db = TestDatabase::migrate::<DumpedMigrator>()
            .await
            .expect("migrate the empty configured database");
        insert("ltg_dumped_first", 1, "committed")
            .await
            .expect("write a row");
        assert_eq!(
            count(outside.inner(), "ltg_dumped_first").await,
            1,
            "migrate holds no transaction around the test"
        );
    }
    for table in ["ltg_dumped_first", "ltg_dumped_second", "ltg_dumped_ledger"] {
        assert!(
            !has_table(&outside, table).await,
            "migrate left {table} behind"
        );
    }

    let db = TestDatabase::migrate::<DumpedMigrator>()
        .await
        .expect("migrate for the next test");
    assert_eq!(
        count(db.conn(), "ltg_dumped_first").await,
        0,
        "the row the first test wrote is gone"
    );
}

static UNDUMPED_RUNS: AtomicUsize = AtomicUsize::new(0);
migrator!(
    UndumpedMigrator,
    "ltg_undumped_ledger",
    [
        TableMigration {
            name: "m_ltg_000018_undumped_first",
            table: "ltg_undumped_first",
            runs: &UNDUMPED_RUNS,
            fail: None,
        },
        TableMigration {
            name: "m_ltg_000019_undumped_second",
            table: "ltg_undumped_second",
            runs: &UNDUMPED_RUNS,
            fail: None,
        },
    ]
);

/// On an empty database with no schema dump, `migrate` runs every
/// migration and, when the helper drops, rolls every one back and drops the
/// migration table it created, so the database is as empty as it was.
#[tokio::test]
async fn migrate_on_an_empty_database_rolls_back_every_migration() {
    let file = configured_file().await;
    let _no_dump = DumpDirectory::new();
    let outside = outside(&file.url).await;
    {
        let _db = TestDatabase::migrate::<UndumpedMigrator>()
            .await
            .expect("migrate the empty configured database");
        assert_eq!(
            UNDUMPED_RUNS.load(Ordering::SeqCst),
            2,
            "every migration ran"
        );
        insert("ltg_undumped_first", 1, "a")
            .await
            .expect("write a row");
        insert("ltg_undumped_second", 1, "b")
            .await
            .expect("write a row");
    }
    for table in [
        "ltg_undumped_first",
        "ltg_undumped_second",
        "ltg_undumped_ledger",
    ] {
        assert!(
            !has_table(&outside, table).await,
            "migrate left {table} behind"
        );
    }
}

static PRUNED_SOURCE_RUNS: AtomicUsize = AtomicUsize::new(0);
migrator!(
    PrunedSourceMigrator,
    "ltg_pruned_ledger",
    [TableMigration {
        name: "m_ltg_000020_pruned",
        table: "ltg_pruned",
        runs: &PRUNED_SOURCE_RUNS,
        fail: None,
    }]
);
migrator!(
    PrunedMigrator,
    "ltg_pruned_ledger",
    [PrunedMigration::new("m_ltg_000020_pruned")]
);

/// A migrator whose migrations were pruned into a schema dump cannot run
/// under `migrate`, which does not load the dump: the error names the dump
/// and points to `refresh`, which loads it, and the failed `migrate` leaves
/// no table behind.
#[tokio::test]
async fn migrate_refuses_a_migrator_pruned_into_a_schema_dump() {
    let file = configured_file().await;
    let dump = DumpDirectory::new();
    let path = dump.write::<PrunedSourceMigrator>().await;

    let error = TestDatabase::migrate::<PrunedMigrator>()
        .await
        .err()
        .expect("a pruned migration cannot run under migrate");
    let message = error.to_string();
    assert!(
        message.contains("was pruned into a schema dump"),
        "got: {message}"
    );
    assert!(message.contains("TestDatabase::refresh"), "got: {message}");
    assert!(
        message.contains(&path.display().to_string()),
        "the error names the dump: {message}"
    );
    let outside = outside(&file.url).await;
    for table in ["ltg_pruned", "ltg_pruned_ledger"] {
        assert!(
            !has_table(&outside, table).await,
            "the failed migrate left {table} behind"
        );
    }

    let db = TestDatabase::refresh::<PrunedMigrator>()
        .await
        .expect("refresh loads the dump");
    assert_eq!(count(db.conn(), "ltg_pruned").await, 0);
}

// ---------------------------------------------------------------------------
// refresh_lazily
// ---------------------------------------------------------------------------

static LAZY_RUNS: AtomicUsize = AtomicUsize::new(0);
migrator!(
    LazyMigrator,
    "ltg_lazy_ledger",
    [TableMigration {
        name: "m_ltg_000010_lazy",
        table: "ltg_lazy",
        runs: &LAZY_RUNS,
        fail: None,
    }]
);

/// Falsifier: `refresh_lazily` migrates before the first query. Building
/// the helper migrates nothing; the first query through the container
/// migrates, inside the same test transaction as `refresh`, and the next
/// helper of the process does not migrate again.
#[tokio::test]
async fn refresh_lazily_migrates_on_the_first_query() {
    let file = configured_file().await;
    let outside = outside(&file.url).await;
    {
        let db = TestDatabase::refresh_lazily::<LazyMigrator>()
            .await
            .expect("a lazy helper builds");
        assert_eq!(LAZY_RUNS.load(Ordering::SeqCst), 0, "nothing migrated yet");
        assert!(!has_table(&outside, "ltg_lazy").await);

        insert("ltg_lazy", 1, "first query")
            .await
            .expect("the first query migrates, then runs");
        assert_eq!(
            LAZY_RUNS.load(Ordering::SeqCst),
            1,
            "the first query migrated"
        );
        assert!(has_table(&outside, "ltg_lazy").await);
        assert_eq!(count(db.conn(), "ltg_lazy").await, 1);
        assert_eq!(
            count(outside.inner(), "ltg_lazy").await,
            0,
            "in a transaction"
        );
    }

    let db = TestDatabase::refresh_lazily::<LazyMigrator>()
        .await
        .expect("a second lazy helper builds");
    assert_eq!(count(db.conn(), "ltg_lazy").await, 0, "the row rolled back");
    assert_eq!(
        LAZY_RUNS.load(Ordering::SeqCst),
        1,
        "migrated once per process"
    );
}

static LAZY_FAILING_RUNS: AtomicUsize = AtomicUsize::new(0);
static LAZY_FAILS: AtomicBool = AtomicBool::new(true);
migrator!(
    LazyFailingMigrator,
    "ltg_lazy_failing_ledger",
    [TableMigration {
        name: "m_ltg_000011_lazy_failing",
        table: "ltg_lazy_failing",
        runs: &LAZY_FAILING_RUNS,
        fail: Some(&LAZY_FAILS),
    }]
);

/// A lazy migration that fails cannot fail the first query from inside the
/// pool, so the query runs on the database as it is and fails there, and
/// dropping the helper fails the test with the migration's error.
#[tokio::test]
async fn a_failed_lazy_migration_fails_the_test_when_the_helper_drops() {
    let _file = configured_file().await;
    let db = TestDatabase::refresh_lazily::<LazyFailingMigrator>()
        .await
        .expect("a lazy helper builds without migrating");
    insert("ltg_lazy_failing", 1, "x")
        .await
        .expect_err("the table the failed migration would create is missing");
    let panic = std::panic::catch_unwind(AssertUnwindSafe(|| drop(db)))
        .expect_err("dropping the helper reports the failed migration");
    let message = panic.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(message.contains("failed on purpose"), "got: {message}");
}

// ---------------------------------------------------------------------------
// seed
// ---------------------------------------------------------------------------

static SEED_RUNS: AtomicUsize = AtomicUsize::new(0);
migrator!(
    SeedMigrator,
    "ltg_seed_ledger",
    [
        TableMigration {
            name: "m_ltg_000012_seed",
            table: "ltg_seed",
            runs: &SEED_RUNS,
            fail: None,
        },
        seeded_table("m_ltg_000013_seeded"),
    ]
);

/// Falsifier: a seeder given to `seed` has not run when the test body
/// starts. Under `refresh` the seeded rows belong to the test transaction
/// and are gone for the next test; under `fresh` the seeder runs on the
/// in-memory database.
#[tokio::test]
async fn seed_runs_the_named_seeder_before_the_body() {
    let file = configured_file().await;
    {
        let db = TestDatabase::refresh::<SeedMigrator>()
            .await
            .expect("refresh")
            .seed::<LabelSeeder>()
            .await
            .expect("seed");
        assert_eq!(count(db.conn(), SEEDED).await, 1, "the seeder ran");
        assert_eq!(
            count(outside(&file.url).await.inner(), SEEDED).await,
            0,
            "the seeded row is the test transaction's"
        );
    }
    let db = TestDatabase::refresh::<SeedMigrator>()
        .await
        .expect("refresh for the next test");
    assert_eq!(
        count(db.conn(), SEEDED).await,
        0,
        "the seeded row rolled back"
    );
    drop(db);

    let db = TestDatabase::fresh::<SeedMigrator>()
        .await
        .expect("fresh")
        .seed::<LabelSeeder>()
        .await
        .expect("seed the in-memory database");
    assert_eq!(count(db.conn(), SEEDED).await, 1);

    let error = TestDatabase::fresh::<SeedMigrator>()
        .await
        .expect("fresh")
        .seed::<FailingSeeder>()
        .await
        .err()
        .expect("a failing seeder fails seed");
    assert!(
        error.to_string().contains("failed on purpose"),
        "got: {error}"
    );
}

/// `seed_root` runs the registered root seeder, as a bare `db:seed` does,
/// and is an error when no seeder is registered, instead of seeding
/// nothing. The registry is one for the process, so the test holds the
/// environment lock while it fills and clears it.
#[tokio::test]
async fn seed_root_runs_the_registered_root_seeder() {
    let _env = crate::env_lock::lock_env_async().await;
    seed::clear();
    let error = TestDatabase::fresh::<SeedMigrator>()
        .await
        .expect("fresh")
        .seed_root()
        .await
        .err()
        .expect("no registered seeder is an error");
    assert!(
        error.to_string().contains("no seeder is registered"),
        "got: {error}"
    );

    seed::register_root::<RootSeeder>().expect("register the root seeder");
    let seeded = TestDatabase::fresh::<SeedMigrator>()
        .await
        .expect("fresh")
        .seed_root()
        .await;
    seed::clear();
    let db = seeded.expect("seed with the root seeder");
    let labels = db
        .fetch_all("SELECT label FROM ltg_seeded", vec![])
        .await
        .expect("read the seeded rows");
    assert_eq!(labels.len(), 1);
    assert_eq!(labels[0].try_get::<String>("", "label").unwrap(), "root");
}

// ---------------------------------------------------------------------------
// #[suprnova_test(refresh, seed)]
// ---------------------------------------------------------------------------

/// The `seed = ..` option runs the seeder after the migrations and before
/// the body, here on the default in-memory database.
#[suprnova::suprnova_test(migrator = SeedMigrator, seed = LabelSeeder)]
async fn suprnova_test_seed_option_runs_the_seeder_before_the_body(db: TestDatabase) {
    assert_eq!(count(db.conn(), SEEDED).await, 1);
}

static MACRO_RUNS: AtomicUsize = AtomicUsize::new(0);
migrator!(
    MacroMigrator,
    "ltg_macro_ledger",
    [
        TableMigration {
            name: "m_ltg_000014_macro",
            table: "ltg_macro",
            runs: &MACRO_RUNS,
            fail: None,
        },
        seeded_table("m_ltg_000015_macro_seeded"),
    ]
);

/// The `refresh` option runs the test on the configured database: the
/// child test below runs in a process of its own with `DATABASE_URL`
/// naming a file, and the file holds the migrated tables afterwards, with
/// the seeded row rolled back.
#[tokio::test]
async fn suprnova_test_refresh_option_runs_on_the_configured_database() {
    let _env = crate::env_lock::lock_env_async().await;
    let dir = tempfile::tempdir().expect("create a temporary directory");
    let url = format!("sqlite://{}", dir.path().join("app.db").display());
    let output = crate::own_process::child_command(
        "laravel_testing_gaps::suprnova_test_refresh_option_child",
    )
    .env("DATABASE_URL", &url)
    .output()
    .expect("run the child test");
    crate::own_process::assert_child_passed(&output);

    let outside = outside(&url).await;
    assert!(
        has_table(&outside, "ltg_macro").await,
        "the child migrated the file"
    );
    assert_eq!(
        count(outside.inner(), SEEDED).await,
        0,
        "the seeded row rolled back"
    );
}

/// The body of the test above, run only in its child process.
#[suprnova::suprnova_test(refresh, migrator = MacroMigrator, seed = LabelSeeder)]
#[ignore = "runs only as the child of suprnova_test_refresh_option_runs_on_the_configured_database, which sets DATABASE_URL"]
async fn suprnova_test_refresh_option_child(db: TestDatabase) {
    let url = std::env::var("DATABASE_URL").expect("the parent set DATABASE_URL");
    assert_eq!(count(db.conn(), SEEDED).await, 1, "seeded before the body");
    let outside = outside(&url).await;
    assert!(
        has_table(&outside, "ltg_macro").await,
        "migrated in the file"
    );
    assert_eq!(
        count(outside.inner(), SEEDED).await,
        0,
        "in the test transaction"
    );
}

// ---------------------------------------------------------------------------
// Engines
// ---------------------------------------------------------------------------

/// The tables, migration tables and counters of one engine's run.
struct Engine {
    refresh: &'static str,
    migrate: &'static str,
    lazy: &'static str,
    ledgers: [&'static str; 3],
    refresh_runs: &'static AtomicUsize,
    lazy_runs: &'static AtomicUsize,
}

impl Engine {
    async fn drop_tables(&self, outside: &DbConnection) {
        for table in [self.refresh, self.migrate, self.lazy, SEEDED]
            .into_iter()
            .chain(self.ledgers)
        {
            outside
                .inner()
                .execute_unprepared(&format!("DROP TABLE IF EXISTS {table}"))
                .await
                .expect("drop a table of an earlier run");
        }
    }
}

/// The SQLite contract on the engine `variable` names: rows gone for the
/// next test, savepoints, one migration per process, the seeder, `migrate`
/// leaving no table, and the lazy trigger.
async fn engine_contract<R, G, L>(variable: &str, engine: &Engine)
where
    R: MigratorTrait + 'static,
    G: MigratorTrait + 'static,
    L: MigratorTrait + 'static,
{
    let url = std::env::var(variable).unwrap_or_else(|_| panic!("set {variable}"));
    let _env = crate::env_lock::lock_env_async().await;
    let _restore = EnvSnapshot::capture(&["DATABASE_URL"]);
    set_env("DATABASE_URL", Some(&url));
    let outside = outside(&url).await;
    engine.drop_tables(&outside).await;

    {
        let db = TestDatabase::refresh::<R>().await.expect("refresh");
        assert_eq!(engine.refresh_runs.load(Ordering::SeqCst), 1);
        assert!(
            has_table(&outside, engine.refresh).await,
            "migrated on the engine"
        );
        insert(engine.refresh, 1, "plain").await.expect("write");
        DB::transaction(|_tx| {
            let table = engine.refresh;
            Box::pin(async move {
                insert(table, 2, "savepoint").await?;
                Ok::<(), FrameworkError>(())
            })
        })
        .await
        .expect("an inner transaction commits to its savepoint");
        assert_eq!(count(db.conn(), engine.refresh).await, 2);
        assert_eq!(count(outside.inner(), engine.refresh).await, 0);
    }

    outside
        .inner()
        .execute_unprepared(&format!("DELETE FROM {}", engine.ledgers[0]))
        .await
        .expect("forget the ledger rows");
    {
        let db = TestDatabase::refresh::<R>()
            .await
            .expect("refresh again")
            .seed::<LabelSeeder>()
            .await
            .expect("seed");
        assert_eq!(
            engine.refresh_runs.load(Ordering::SeqCst),
            1,
            "migrated once"
        );
        assert_eq!(
            count(db.conn(), engine.refresh).await,
            0,
            "the rows are gone"
        );
        assert_eq!(count(db.conn(), SEEDED).await, 1, "seeded before the body");
    }
    assert_eq!(
        count(outside.inner(), SEEDED).await,
        0,
        "the seeded row rolled back"
    );

    {
        let _db = TestDatabase::migrate::<G>().await.expect("migrate");
        assert!(has_table(&outside, engine.migrate).await);
    }
    assert!(
        !has_table(&outside, engine.migrate).await,
        "migrate left no table"
    );

    {
        let _db = TestDatabase::refresh_lazily::<L>()
            .await
            .expect("refresh lazily");
        assert_eq!(engine.lazy_runs.load(Ordering::SeqCst), 0);
        assert!(!has_table(&outside, engine.lazy).await, "not migrated yet");
        insert(engine.lazy, 1, "first")
            .await
            .expect("the first query");
        assert_eq!(engine.lazy_runs.load(Ordering::SeqCst), 1);
        assert!(has_table(&outside, engine.lazy).await);
    }

    engine.drop_tables(&outside).await;
}

static PG_REFRESH_RUNS: AtomicUsize = AtomicUsize::new(0);
static PG_MIGRATE_RUNS: AtomicUsize = AtomicUsize::new(0);
static PG_LAZY_RUNS: AtomicUsize = AtomicUsize::new(0);
migrator!(
    PgRefreshMigrator,
    "ltg_pg_refresh_ledger",
    [
        TableMigration {
            name: "m_ltg_000101_pg_refresh",
            table: "ltg_pg_refresh",
            runs: &PG_REFRESH_RUNS,
            fail: None,
        },
        seeded_table("m_ltg_000102_pg_seeded"),
    ]
);
migrator!(
    PgMigrateMigrator,
    "ltg_pg_migrate_ledger",
    [TableMigration {
        name: "m_ltg_000103_pg_migrate",
        table: "ltg_pg_migrate",
        runs: &PG_MIGRATE_RUNS,
        fail: None,
    }]
);
migrator!(
    PgLazyMigrator,
    "ltg_pg_lazy_ledger",
    [TableMigration {
        name: "m_ltg_000104_pg_lazy",
        table: "ltg_pg_lazy",
        runs: &PG_LAZY_RUNS,
        fail: None,
    }]
);

#[tokio::test]
#[ignore = "requires a disposable Postgres at PG_TEST_URL"]
async fn postgres_refresh_migrate_lazily_and_seed_on_the_configured_database() {
    engine_contract::<PgRefreshMigrator, PgMigrateMigrator, PgLazyMigrator>(
        "PG_TEST_URL",
        &Engine {
            refresh: "ltg_pg_refresh",
            migrate: "ltg_pg_migrate",
            lazy: "ltg_pg_lazy",
            ledgers: [
                "ltg_pg_refresh_ledger",
                "ltg_pg_migrate_ledger",
                "ltg_pg_lazy_ledger",
            ],
            refresh_runs: &PG_REFRESH_RUNS,
            lazy_runs: &PG_LAZY_RUNS,
        },
    )
    .await;
}

static MY_REFRESH_RUNS: AtomicUsize = AtomicUsize::new(0);
static MY_MIGRATE_RUNS: AtomicUsize = AtomicUsize::new(0);
static MY_LAZY_RUNS: AtomicUsize = AtomicUsize::new(0);
migrator!(
    MyRefreshMigrator,
    "ltg_my_refresh_ledger",
    [
        TableMigration {
            name: "m_ltg_000201_my_refresh",
            table: "ltg_my_refresh",
            runs: &MY_REFRESH_RUNS,
            fail: None,
        },
        seeded_table("m_ltg_000202_my_seeded"),
    ]
);
migrator!(
    MyMigrateMigrator,
    "ltg_my_migrate_ledger",
    [TableMigration {
        name: "m_ltg_000203_my_migrate",
        table: "ltg_my_migrate",
        runs: &MY_MIGRATE_RUNS,
        fail: None,
    }]
);
migrator!(
    MyLazyMigrator,
    "ltg_my_lazy_ledger",
    [TableMigration {
        name: "m_ltg_000204_my_lazy",
        table: "ltg_my_lazy",
        runs: &MY_LAZY_RUNS,
        fail: None,
    }]
);

#[tokio::test]
#[ignore = "requires a disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_refresh_migrate_lazily_and_seed_on_the_configured_database() {
    engine_contract::<MyRefreshMigrator, MyMigrateMigrator, MyLazyMigrator>(
        "MYSQL_TEST_URL",
        &Engine {
            refresh: "ltg_my_refresh",
            migrate: "ltg_my_migrate",
            lazy: "ltg_my_lazy",
            ledgers: [
                "ltg_my_refresh_ledger",
                "ltg_my_migrate_ledger",
                "ltg_my_lazy_ledger",
            ],
            refresh_runs: &MY_REFRESH_RUNS,
            lazy_runs: &MY_LAZY_RUNS,
        },
    )
    .await;
}
