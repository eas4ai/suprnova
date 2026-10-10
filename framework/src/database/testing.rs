//! Testing utilities for database operations
//!
//! Provides `TestDatabase` for setting up isolated test environments: an
//! in-memory SQLite database per test ([`TestDatabase::fresh`]), or the
//! database `DATABASE_URL` names, with each test in a transaction
//! ([`TestDatabase::refresh`], [`TestDatabase::refresh_lazily`]) or with
//! its migrations rolled back ([`TestDatabase::migrate`]).
//!
//! # Example
//!
//! ```rust,ignore
//! use suprnova::test_database;
//!
//! #[tokio::test]
//! async fn test_create_user() {
//!     let db = test_database!();
//!
//!     // Your test code here - actions using DB::connection()
//!     // will automatically use this test database
//! }
//! ```

use std::any::TypeId;
use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use futures::future::LocalBoxFuture;
use sea_orm::{ConnectOptions, DatabaseConnection};
use sea_orm_migration::MigratorTrait;

use super::config::{DatabaseConfig, DatabaseType};
use super::connection::DbConnection;
use crate::container::testing::{TestContainer, TestContainerGuard};
use crate::error::FrameworkError;
use crate::seed::Seeder;

/// Test database wrapper that provides isolated database environments
///
/// [`Self::fresh`], the default, creates an in-memory SQLite database
/// with migrations applied. [`Self::refresh`], [`Self::refresh_lazily`]
/// and [`Self::migrate`] run the test on the database `DATABASE_URL`
/// names instead, so a suite can run on the engine the application ships
/// on. Every constructor registers the connection in the test container,
/// so any code using `DB::connection()` or `#[inject] db: Database` will
/// receive this test database. [`Self::seed`] and [`Self::seed_root`] run
/// a seeder on it before the test body.
///
/// When the `TestDatabase` is dropped, the test container is cleared and
/// the test's changes are undone: the in-memory database goes with its
/// connection, the transaction of `refresh` is rolled back, and the
/// migrations `migrate` ran are rolled back.
///
/// # Example
///
/// ```rust,ignore
/// use suprnova::testing::TestDatabase;
/// use crate::migrations::Migrator;
///
/// #[tokio::test]
/// async fn test_user_creation() {
///     let db = TestDatabase::fresh::<Migrator>().await.unwrap();
///
///     // Actions using DB::connection() automatically get this test database
///     let action = CreateUserAction::new();
///     let user = action.execute("test@example.com").await.unwrap();
///
///     // Query directly using db.conn()
///     let found = users::Entity::find_by_id(user.id)
///         .one(db.conn())
///         .await
///         .unwrap();
///     assert!(found.is_some());
/// }
/// ```
pub struct TestDatabase {
    conn: DbConnection,
    _guard: TestContainerGuard,
    /// What dropping the helper undoes besides clearing the container: a
    /// [`TestTransaction`] or [`AppliedMigrations`], held for its drop,
    /// which runs last, after the container's copy of the connection is
    /// gone. `None` for `fresh`, whose database goes with its connection.
    _undo: Option<Box<dyn Send + Sync>>,
}

impl TestDatabase {
    /// Create a fresh test database with migrations applied
    ///
    /// This creates an in-memory SQLite database, loads the project's
    /// SQLite schema dump (`database/schema/sqlite-schema.sql`) when there
    /// is one, runs the migrations the dump does not record, and registers
    /// the connection in the test container.
    ///
    /// # Type Parameters
    ///
    /// * `M` - The migrator type implementing `MigratorTrait`. Typically
    ///   this is `crate::migrations::Migrator` from your application.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Database connection fails
    /// - Migration execution fails
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use suprnova::testing::TestDatabase;
    /// use crate::migrations::Migrator;
    ///
    /// #[tokio::test]
    /// async fn test_example() {
    ///     let db = TestDatabase::fresh::<Migrator>().await.unwrap();
    ///     // ...
    /// }
    /// ```
    pub async fn fresh<M: MigratorTrait>() -> Result<Self, FrameworkError> {
        // 1. Create test container guard for isolation
        let guard = TestContainer::fake();

        // 2. Create in-memory SQLite database
        let config = DatabaseConfig::builder()
            .url("sqlite::memory:")
            .max_connections(1)
            .min_connections(1)
            .logging(false)
            .build();

        let conn = DbConnection::connect(&config).await?;

        // 3. Load the SQLite schema dump, when the project has one and it
        //    belongs to `M` (every migration its ledger records is in `M`'s
        //    list), so the migrations it covers are not run again. A test
        //    with a Migrator of its own runs that Migrator from scratch.
        let dump = crate::database_path("schema/sqlite-schema.sql");
        if dump.is_file() {
            let sql = std::fs::read_to_string(&dump).map_err(|e| {
                FrameworkError::database(format!("could not read {}: {e}", dump.display()))
            })?;
            let listed: std::collections::HashSet<String> = M::migrations()
                .iter()
                .map(|migration| migration.name().to_owned())
                .collect();
            let recorded = crate::database::schema_dump::ledger_versions(&sql);
            if !recorded.is_empty() && recorded.iter().all(|version| listed.contains(version)) {
                crate::database::schema_dump::load_sqlite(conn.inner(), &sql).await?;
            }
        }

        // 4. Run migrations
        M::up(conn.inner(), None)
            .await
            .map_err(|e| FrameworkError::database(format!("Migration failed: {}", e)))?;

        // 5. Register in TestContainer - this is the key integration!
        // Any code calling DB::connection() or App::resolve::<DbConnection>()
        // will now get this test database
        TestContainer::singleton(conn.clone());

        Ok(Self {
            conn,
            _guard: guard,
            _undo: None,
        })
    }

    /// Run the test on the database `DATABASE_URL` names, inside a
    /// transaction that is rolled back when the helper drops, as Laravel's
    /// `RefreshDatabase` does.
    ///
    /// The first `refresh` of a test process for this database and `M`
    /// migrates the database the way the application's `migrate` command
    /// does: it loads the schema dump into a database that has run no
    /// migration, then runs `M`'s pending migrations. The others do not
    /// migrate again, and a test that starts while that migration runs
    /// waits for it. A migration that fails is not remembered, so the
    /// next `refresh` tries again.
    ///
    /// The helper then opens a pool of one connection, begins a
    /// transaction on it and registers it in the test container. Every
    /// query of the test runs in that transaction: through
    /// `DB::connection()`, a model, a factory, a seeder, [`Self::conn`],
    /// the database session driver, and a database queue driver built
    /// after the helper. A `DB::transaction` inside the test
    /// is a savepoint: it commits or rolls back its own work, and its
    /// after-commit callbacks run when it commits, as they do in Laravel,
    /// where the test transaction does not count. When the helper drops,
    /// its pool closes and the database discards the transaction, so a row
    /// one test writes is gone for the next.
    ///
    /// The pool holds one connection, as under [`Self::fresh`]: code that
    /// holds a transaction of its own (`DB::begin_transaction`) and runs a
    /// query outside it waits for the connection until the acquire
    /// timeout. A statement that ends the transaction itself, such as DDL
    /// on MySQL or a raw `COMMIT`, defeats the rollback. Postgres and MySQL
    /// test processes that share one database see none of each other's
    /// rows, while SQLite allows one writer, so tests that share a SQLite
    /// file wait for each other's writes up to the busy timeout.
    ///
    /// # Errors
    ///
    /// Returns an error when `DATABASE_URL` is unset or names an in-memory
    /// SQLite database (use [`Self::fresh`] for that), when the migrations
    /// fail, or when the connection cannot be opened.
    ///
    /// # Panics
    ///
    /// Dropping the helper panics, unless the thread is already panicking,
    /// when the test's connection was lost and a new one opened during the
    /// test, since the rows the test wrote before were rolled back early.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use suprnova::testing::TestDatabase;
    /// use crate::migrations::Migrator;
    ///
    /// #[tokio::test]
    /// async fn test_on_postgres() {
    ///     // DATABASE_URL=postgres://localhost/app_test
    ///     let db = TestDatabase::refresh::<Migrator>().await.unwrap();
    ///     // ... rows written here are gone for the next test
    /// }
    /// ```
    pub async fn refresh<M: MigratorTrait + 'static>() -> Result<Self, FrameworkError> {
        const HELPER: &str = "TestDatabase::refresh";
        Self::transactional::<M>(HELPER, configured_database(HELPER)?, false).await
    }

    /// [`Self::refresh`], with the migration and the transaction done on
    /// the first query instead of when the helper is built, as Laravel's
    /// `LazilyRefreshDatabase` does: a test that never touches the
    /// database never migrates it.
    ///
    /// The helper registers its connection at once and opens it on the
    /// first query, a transaction or a seeder: then it begins the
    /// transaction, migrates as `refresh` does, and runs the query.
    ///
    /// # Errors
    ///
    /// Returns an error when `DATABASE_URL` is unset or names an in-memory
    /// SQLite database, or when the pool cannot be built. A migration that
    /// fails on the first query cannot fail that query from inside the
    /// pool: the query runs on the database as it is, and dropping the
    /// helper fails the test with the migration's error.
    ///
    /// # Panics
    ///
    /// Dropping the helper panics, unless the thread is already panicking,
    /// when the migrations failed on the first query, or when the test's
    /// connection was lost and a new one opened during the test.
    pub async fn refresh_lazily<M: MigratorTrait + 'static>() -> Result<Self, FrameworkError> {
        const HELPER: &str = "TestDatabase::refresh_lazily";
        Self::transactional::<M>(HELPER, configured_database(HELPER)?, true).await
    }

    /// Run `M`'s migrations on the database `DATABASE_URL` names and roll
    /// them back when the helper drops, as Laravel's `DatabaseMigrations`
    /// does.
    ///
    /// The migrations run the way the application's `migrate` command
    /// runs them, schema dump included, on the connection the helper
    /// registers. There is no transaction around the test, so another
    /// connection sees its rows. When the helper drops, it rolls back the
    /// migrations it ran and no others, on a connection of its own, and the
    /// next [`Self::refresh`] of the process migrates again. Tables a
    /// schema dump created stay, as a dump has nothing to roll back. Two
    /// `migrate` tests on one database must not run at once.
    ///
    /// # Errors
    ///
    /// Returns an error when `DATABASE_URL` is unset or names an in-memory
    /// SQLite database, when the connection cannot be opened, or when a
    /// migration fails. The migrations that ran before the one that failed
    /// are rolled back first.
    ///
    /// # Panics
    ///
    /// Dropping the helper panics, unless the thread is already panicking,
    /// when the rollback fails, so a test cannot pass while it leaves its
    /// tables behind.
    pub async fn migrate<M: MigratorTrait + 'static>() -> Result<Self, FrameworkError> {
        const HELPER: &str = "TestDatabase::migrate";
        let config = configured_database(HELPER)?;
        let guard = TestContainer::fake();
        let conn = DbConnection::connect(&config).await?;
        crate::database::schema_dump::load_when_empty::<M>(&config.url, conn.inner(), None).await?;
        let pending = pending_count::<M>(conn.inner()).await?;
        if let Err(error) = M::up(conn.inner(), None).await {
            let failed = format!("{HELPER}: a migration failed: {error}");
            return Err(match roll_back_partial::<M>(conn.inner(), pending).await {
                Ok(()) => FrameworkError::database(failed),
                Err(undo) => FrameworkError::database(format!(
                    "{failed}; the migrations that ran before it could not be rolled back: {undo}"
                )),
            });
        }
        TestContainer::singleton(conn.clone());
        Ok(Self {
            conn,
            _guard: guard,
            _undo: Some(Box::new(AppliedMigrations {
                url: config.url,
                count: pending,
                roll_back: roll_back_with::<M>,
            })),
        })
    }

    /// Run the seeder `S` on this database before the test body, as
    /// Laravel's `#[Seeder]` attribute does, and hand the helper back.
    ///
    /// `S` runs whether or not it is registered, without progress lines;
    /// the seeders it calls are found in the registry. Its queries go to
    /// this database, so under [`Self::refresh`] the seeded rows belong to
    /// the test transaction and are gone for the next test.
    ///
    /// # Errors
    ///
    /// Returns the seeder's error. The helper is dropped then, which
    /// undoes the test's changes as usual.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let db = TestDatabase::refresh::<Migrator>()
    ///     .await?
    ///     .seed::<UsersSeeder>()
    ///     .await?;
    /// ```
    pub async fn seed<S: Seeder + 'static>(self) -> Result<Self, FrameworkError> {
        crate::seed::run_type::<S>().await?;
        Ok(self)
    }

    /// Run the root seeder on this database before the test body, as
    /// Laravel's `#[Seed]` attribute runs `DatabaseSeeder`, and hand the
    /// helper back.
    ///
    /// It runs what a bare `db:seed` runs: the seeder registered with
    /// [`crate::seed::register_root`], or every registered seeder in order
    /// when no root is registered.
    ///
    /// # Errors
    ///
    /// Returns an error when no seeder is registered, rather than seed
    /// nothing, and returns a seeder's error.
    pub async fn seed_root(self) -> Result<Self, FrameworkError> {
        if crate::seed::try_count()? == 0 {
            return Err(FrameworkError::not_found(
                "TestDatabase::seed_root: no seeder is registered; register the root seeder \
                 with seed::register_root, or name one with TestDatabase::seed",
            ));
        }
        crate::seed::run_root().await?;
        Ok(self)
    }

    /// The body of [`Self::refresh`] and [`Self::refresh_lazily`], on
    /// `config`'s database.
    async fn transactional<M: MigratorTrait + 'static>(
        helper: &'static str,
        config: DatabaseConfig,
        lazily: bool,
    ) -> Result<Self, FrameworkError> {
        let key = MigrationKey::of::<M>(&config.url);
        let migrate: MigrateFn = migrate_with::<M>;
        if !lazily {
            let key = key.clone();
            tokio::task::spawn_blocking(move || migrate_once(&key, migrate))
                .await
                .map_err(|e| {
                    FrameworkError::internal(format!(
                        "{helper}: the migration ended without a result: {e}"
                    ))
                })?
                .map_err(|e| FrameworkError::database(format!("{helper}: {e}")))?;
        }
        let state = Arc::new(TransactionState {
            helper,
            lazy: lazily.then_some((key, migrate)),
            built: AtomicBool::new(false),
            lazy_tried: AtomicBool::new(false),
            opened: AtomicUsize::new(0),
            failure: Mutex::new(None),
        });
        let guard = TestContainer::fake();
        let conn = DbConnection::connect_test_transaction(&config, |options| {
            hold_one_transaction(options, &state)
        })
        .await?;
        let db = Self {
            conn: conn.clone(),
            _guard: guard,
            _undo: Some(Box::new(TestTransaction {
                conn,
                state: Arc::clone(&state),
            })),
        };
        if !lazily {
            // Opening the connection begins the transaction, so a database
            // that cannot be reached fails here rather than on a query.
            db.conn.inner().ping().await.map_err(|e| {
                FrameworkError::database(format!("{helper}: could not open the connection: {e}"))
            })?;
        }
        TestContainer::singleton(db.conn.clone());
        state.built.store(true, Ordering::SeqCst);
        Ok(db)
    }

    /// Get a reference to the underlying database connection
    ///
    /// Use this when you need to execute queries directly in your tests.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let db = test_database!();
    /// let users = users::Entity::find().all(db.conn()).await?;
    /// ```
    pub fn conn(&self) -> &DatabaseConnection {
        self.conn.inner()
    }

    /// Get the `DbConnection` wrapper
    ///
    /// Use this when you need the full `DbConnection` type.
    pub fn db(&self) -> &DbConnection {
        &self.conn
    }

    /// Connect to an in-memory SQLite database WITHOUT running any migrations.
    ///
    /// Companion to [`Self::fresh`] for tests that build their own ad-hoc tables
    /// via [`Self::execute_unprepared`]. Same container registration semantics
    /// as `fresh` - any code calling `DB::connection()` resolves to this DB.
    ///
    /// Use `fresh::<M>()` for end-to-end tests with a real migrator; use
    /// `sqlite_memory()` for unit tests that need precise column-shape control.
    pub async fn sqlite_memory() -> Result<Self, FrameworkError> {
        let guard = TestContainer::fake();
        let config = DatabaseConfig::builder()
            .url("sqlite::memory:")
            .max_connections(1)
            .min_connections(1)
            .logging(false)
            .build();
        let conn = DbConnection::connect(&config).await?;
        TestContainer::singleton(conn.clone());
        Ok(Self {
            conn,
            _guard: guard,
            _undo: None,
        })
    }

    /// Execute a DDL / DML statement with no placeholders. Delegator over
    /// `ConnectionTrait::execute_unprepared` so tests don't need to import
    /// the trait nor reach through `self.conn()`.
    pub async fn execute_unprepared(&self, sql: &str) -> Result<(), FrameworkError> {
        use sea_orm::ConnectionTrait;
        self.conn
            .inner()
            .execute_unprepared(sql)
            .await
            .map(|_| ())
            .map_err(|e| FrameworkError::database(e.to_string()))
    }

    /// Run a SELECT and return the first row (errors if zero rows). Used by
    /// cast tests to assert on raw storage shape after a round-trip.
    pub async fn fetch_one(
        &self,
        sql: &str,
        bindings: Vec<sea_orm::Value>,
    ) -> Result<sea_orm::QueryResult, FrameworkError> {
        use sea_orm::ConnectionTrait;
        let backend = self.conn.inner().get_database_backend();
        let stmt = sea_orm::Statement::from_sql_and_values(backend, sql, bindings);
        self.conn
            .inner()
            .query_one_raw(stmt)
            .await
            .map_err(|e| FrameworkError::database(e.to_string()))?
            .ok_or_else(|| FrameworkError::not_found("fetch_one: no rows"))
    }

    /// Run a SELECT and return every row. Companion to [`Self::fetch_one`].
    pub async fn fetch_all(
        &self,
        sql: &str,
        bindings: Vec<sea_orm::Value>,
    ) -> Result<Vec<sea_orm::QueryResult>, FrameworkError> {
        use sea_orm::ConnectionTrait;
        let backend = self.conn.inner().get_database_backend();
        let stmt = sea_orm::Statement::from_sql_and_values(backend, sql, bindings);
        self.conn
            .inner()
            .query_all_raw(stmt)
            .await
            .map_err(|e| FrameworkError::database(e.to_string()))
    }
}

/// The test transaction of [`TestDatabase::refresh`] and
/// [`TestDatabase::refresh_lazily`]: the pool, whose one connection holds
/// the transaction, and the state its connect hook shares.
struct TestTransaction {
    conn: DbConnection,
    state: Arc<TransactionState>,
}

impl Drop for TestTransaction {
    fn drop(&mut self) {
        discard(self.conn.inner());
        let failure = crate::lock::recover(&self.state.failure).take();
        if let Some(message) = failure {
            fail_the_test(message);
        }
    }
}

/// What the connect hook of a `refresh` pool shares with its helper.
struct TransactionState {
    /// The constructor's name, for messages.
    helper: &'static str,
    /// The migration a lazy helper runs on the first use of its pool.
    lazy: Option<(MigrationKey, MigrateFn)>,
    /// Set once the helper is built. The uses before are the pool's own:
    /// SeaORM reads the SQLite version as it builds a pool, which opens
    /// the connection and begins the transaction, but is no query of the
    /// test's.
    built: AtomicBool,
    /// Whether the lazy migration has been tried.
    lazy_tried: AtomicBool,
    /// The connections the pool has opened. There is one, unless the
    /// connection was lost.
    opened: AtomicUsize,
    /// A failure the hook could not return: an error from a connect hook
    /// only makes the pool try again, until it times out.
    failure: Mutex<Option<String>>,
}

impl TransactionState {
    /// Runs when the pool has opened a connection and begun its
    /// transaction, before the connection is used.
    fn opened(&self) {
        if self.opened.fetch_add(1, Ordering::SeqCst) > 0 {
            self.fail(format!(
                "{}: the test's connection was lost and a new one opened, so the rows the test \
                 wrote before were rolled back early",
                self.helper
            ));
        }
        self.used();
    }

    /// Runs before each use of the pool's connection: on the first use
    /// after the helper is built, a lazy helper migrates, before the query
    /// runs. The pool hands out its one connection to one user at a time,
    /// so two uses never run this at once.
    fn used(&self) {
        if !self.built.load(Ordering::SeqCst) {
            return;
        }
        if let Some((key, migrate)) = &self.lazy
            && !self.lazy_tried.swap(true, Ordering::SeqCst)
            && let Err(error) = migrate_once(key, *migrate)
        {
            self.fail(format!(
                "{}: the migrations failed on the first query: {error}",
                self.helper
            ));
        }
    }

    fn fail(&self, message: String) {
        tracing::error!(target: "suprnova::testing", "{message}");
        crate::lock::recover(&self.failure).get_or_insert(message);
    }
}

/// The migrations [`TestDatabase::migrate`] ran, to roll back when the
/// helper drops.
struct AppliedMigrations {
    url: String,
    count: u32,
    roll_back: RollBackFn,
}

impl Drop for AppliedMigrations {
    fn drop(&mut self) {
        // The tables are gone, or going: the next `refresh` of the
        // process must migrate again, as Laravel resets its state.
        forget_migrations(&self.url);
        if self.count == 0 {
            return;
        }
        let (url, count, roll_back) = (self.url.clone(), self.count, self.roll_back);
        if let Err(error) = on_own_runtime(move || roll_back(url, count)) {
            let message = format!(
                "TestDatabase::migrate: the migrations it ran were not rolled back: {error}"
            );
            tracing::error!(target: "suprnova::testing", "{message}");
            fail_the_test(message);
        }
    }
}

/// A database and a migrator, the unit `refresh` migrates once per process.
#[derive(Clone, PartialEq, Eq, Hash)]
struct MigrationKey {
    url: String,
    migrator: TypeId,
}

impl MigrationKey {
    fn of<M: 'static>(url: &str) -> Self {
        Self {
            url: url.to_owned(),
            migrator: TypeId::of::<M>(),
        }
    }
}

/// Migrates the database at a URL the way the `migrate` command does.
type MigrateFn = fn(String) -> LocalBoxFuture<'static, Result<(), FrameworkError>>;

/// Rolls back the last migrations of the database at a URL.
type RollBackFn = fn(String, u32) -> LocalBoxFuture<'static, Result<(), FrameworkError>>;

fn migrate_with<M: MigratorTrait + 'static>(
    url: String,
) -> LocalBoxFuture<'static, Result<(), FrameworkError>> {
    Box::pin(async move {
        crate::database::SchemaDump::migrate::<M>(&url, None)
            .await
            .map(|_loaded| ())
    })
}

fn roll_back_with<M: MigratorTrait + 'static>(
    url: String,
    count: u32,
) -> LocalBoxFuture<'static, Result<(), FrameworkError>> {
    Box::pin(async move {
        let config = DatabaseConfig::builder().url(url).logging(false).build();
        let conn = DbConnection::connect(&config).await?;
        M::down(conn.inner(), Some(count))
            .await
            .map_err(|e| FrameworkError::database(e.to_string()))
    })
}

/// The migrations `refresh` has run in this process. Each key has a slot
/// of its own, held while its migration runs, so a test that wants the
/// same migration waits for it and the others go on.
static MIGRATED: LazyLock<Mutex<HashMap<MigrationKey, Arc<Mutex<bool>>>>> =
    LazyLock::new(Mutex::default);

/// Runs `migrate` on the key's database unless a `refresh` of this process
/// already did. It blocks: a pool's connect hook cannot await the work on
/// its own runtime, so the migration runs on a runtime of its own.
fn migrate_once(key: &MigrationKey, migrate: MigrateFn) -> Result<(), FrameworkError> {
    let slot = {
        let mut slots = crate::lock::lock(&MIGRATED, "test database migrations")?;
        Arc::clone(slots.entry(key.clone()).or_default())
    };
    let mut migrated = crate::lock::lock(&slot, "test database migration")?;
    if *migrated {
        return Ok(());
    }
    let url = key.url.clone();
    on_own_runtime(move || migrate(url))?;
    *migrated = true;
    Ok(())
}

/// Forget that `refresh` migrated the database at `url`, after `migrate`
/// rolled migrations back there.
fn forget_migrations(url: &str) {
    crate::lock::recover(&MIGRATED).retain(|key, _| key.url != url);
}

/// Runs the future `work` builds on a runtime of its own, on a thread of
/// its own, and waits for it.
///
/// The callers cannot await: a pool's connect hook, where the caller's
/// runtime must not yield until the migration is done, and a drop. The
/// work opens its own connection, so it needs nothing from the caller's
/// runtime while that one is blocked.
fn on_own_runtime<T, F, Fut>(work: F) -> Result<T, FrameworkError>
where
    T: Send,
    F: FnOnce() -> Fut + Send,
    Fut: Future<Output = Result<T, FrameworkError>>,
{
    std::thread::scope(|scope| {
        scope
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| {
                        FrameworkError::internal(format!(
                            "could not start a runtime for the test database: {e}"
                        ))
                    })?;
                runtime.block_on(work())
            })
            .join()
            .unwrap_or_else(|_| {
                Err(FrameworkError::internal(
                    "the test database's migration thread panicked",
                ))
            })
    })
}

/// The configuration of the database `DATABASE_URL` names, for the
/// helpers that run on it.
fn configured_database(helper: &str) -> Result<DatabaseConfig, FrameworkError> {
    let config = DatabaseConfig::from_env();
    if !config.is_configured() {
        return Err(FrameworkError::database(format!(
            "{helper} runs on the database DATABASE_URL names, and DATABASE_URL is not set; set \
             it to the test database, or use TestDatabase::fresh for an in-memory one"
        )));
    }
    if names_memory_database(&config) {
        return Err(FrameworkError::database(format!(
            "{helper} needs a database that outlives one connection, and DATABASE_URL names an \
             in-memory SQLite database; use TestDatabase::fresh, which gives each test its own \
             migrated in-memory database"
        )));
    }
    Ok(config)
}

/// Whether `config` names an in-memory SQLite database, which lives and
/// dies with one connection.
fn names_memory_database(config: &DatabaseConfig) -> bool {
    if config.database_type() != DatabaseType::Sqlite {
        return false;
    }
    let url = config.url.as_str();
    let rest = url
        .strip_prefix("sqlite://")
        .or_else(|| url.strip_prefix("sqlite:"))
        .unwrap_or(url);
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let path = path.trim_start_matches("./");
    path.is_empty()
        || path.starts_with(":memory:")
        || query.split('&').any(|pair| pair == "mode=memory")
}

async fn pending_count<M: MigratorTrait>(db: &DatabaseConnection) -> Result<u32, FrameworkError> {
    let pending = M::get_pending_migrations(db).await.map_err(|e| {
        FrameworkError::database(format!(
            "TestDatabase::migrate: could not read the pending migrations: {e}"
        ))
    })?;
    Ok(u32::try_from(pending.len()).unwrap_or(u32::MAX))
}

/// Rolls back the migrations that ran of the `pending` ones, after one of
/// them failed.
async fn roll_back_partial<M: MigratorTrait>(
    db: &DatabaseConnection,
    pending: u32,
) -> Result<(), FrameworkError> {
    let ran = pending.saturating_sub(pending_count::<M>(db).await?);
    if ran > 0 {
        M::down(db, Some(ran))
            .await
            .map_err(|e| FrameworkError::database(e.to_string()))?;
    }
    Ok(())
}

/// Shapes the pool of a test transaction: one connection that stays open,
/// opened on first use, with a transaction begun on it as it opens.
fn hold_one_transaction(options: &mut ConnectOptions, state: &Arc<TransactionState>) {
    options
        .max_connections(1)
        .min_connections(0)
        .connect_lazy(true)
        .idle_timeout(None::<Duration>)
        .max_lifetime(None::<Duration>)
        .test_before_acquire(false);
    #[cfg(feature = "database-sqlite")]
    {
        let state = Arc::clone(state);
        options.map_sqlx_sqlite_pool_opts(move |pool| begin_on_connect(pool, Arc::clone(&state)));
    }
    #[cfg(feature = "database-postgres")]
    {
        let state = Arc::clone(state);
        options.map_sqlx_postgres_pool_opts(move |pool| begin_on_connect(pool, Arc::clone(&state)));
    }
    #[cfg(feature = "database-mysql")]
    {
        let state = Arc::clone(state);
        options.map_sqlx_mysql_pool_opts(move |pool| begin_on_connect(pool, Arc::clone(&state)));
    }
    #[cfg(not(any(
        feature = "database-sqlite",
        feature = "database-postgres",
        feature = "database-mysql"
    )))]
    let _no_driver_to_hook = state;
}

/// Begins a transaction on each connection the pool opens and leaves it
/// open, so every statement of the connection runs in it, and tells the
/// helper of each use of the connection.
#[cfg(any(
    feature = "database-sqlite",
    feature = "database-postgres",
    feature = "database-mysql"
))]
fn begin_on_connect<DB: sea_orm::sqlx::Database>(
    pool: sea_orm::sqlx::pool::PoolOptions<DB>,
    state: Arc<TransactionState>,
) -> sea_orm::sqlx::pool::PoolOptions<DB> {
    let on_acquire = Arc::clone(&state);
    pool.after_connect(move |connection, _metadata| {
        let state = Arc::clone(&state);
        Box::pin(async move {
            let transaction = sea_orm::sqlx::Connection::begin(connection).await?;
            // The handle would queue a rollback when dropped. Forgotten, it
            // leaves the transaction open on the connection, and sqlx still
            // counts it, so a `BEGIN` on the connection is a savepoint.
            std::mem::forget(transaction);
            state.opened();
            Ok(())
        })
    })
    // Replaces the idle ping a `DB_PING_AFTER_IDLE` sets: the connection
    // must stay, and a failed ping would close it with its transaction.
    .before_acquire(move |_connection, _metadata| {
        let state = Arc::clone(&on_acquire);
        Box::pin(async move {
            state.used();
            Ok(true)
        })
    })
}

/// Ends the test transaction of `connection` without committing it: the
/// idle connection is closed now, and the pool is closed, so a connection
/// still checked out closes when it comes back instead of returning. A
/// database discards the open transaction of a connection that closes.
fn discard(connection: &DatabaseConnection) {
    match &connection.inner {
        #[cfg(feature = "database-sqlite")]
        sea_orm::DatabaseConnectionType::SqlxSqlitePoolConnection(_) => {
            close_pool(connection.get_sqlite_connection_pool());
        }
        #[cfg(feature = "database-postgres")]
        sea_orm::DatabaseConnectionType::SqlxPostgresPoolConnection(_) => {
            close_pool(connection.get_postgres_connection_pool());
        }
        #[cfg(feature = "database-mysql")]
        sea_orm::DatabaseConnectionType::SqlxMySqlPoolConnection(_) => {
            close_pool(connection.get_mysql_connection_pool());
        }
        _ => {}
    }
}

#[cfg(any(
    feature = "database-sqlite",
    feature = "database-postgres",
    feature = "database-mysql"
))]
fn close_pool<DB: sea_orm::sqlx::Database>(pool: &sea_orm::sqlx::Pool<DB>) {
    if let Some(idle) = pool.try_acquire() {
        drop(idle.detach());
    }
    // Calling `close` marks the pool closed at once; the future it returns
    // only waits for the connections to finish closing, which a drop
    // cannot do.
    let _closing = pool.close();
}

/// Fails the test with `message` from a drop, which has no error to
/// return: it panics, unless the thread is already panicking, where a
/// second panic would abort the process and the message goes to standard
/// error instead.
fn fail_the_test(message: String) {
    if std::thread::panicking() {
        eprintln!("{message}");
    } else {
        panic!("{message}");
    }
}

/// Counts the statements one database connection executes, so a test can
/// prove what a request cost the database: a response the RenderCache
/// served from a validation lease costs no statement at all.
///
/// It counts the prepared statements run through the connection, on its
/// pool and on every transaction started from it: the queries and executes
/// built from a `Statement`, which is what the query builder, the Eloquent
/// models and `DB::statement` run. Unprepared SQL (`DB::unprepared`, the
/// SQL the framework runs for a savepoint) and transaction control (`BEGIN`,
/// `COMMIT`) are not counted, so a request whose only database work is one
/// of those reads zero. It sees nothing else of a statement: not the SQL
/// text and not a bound value, because a count is all a test needs. Clones
/// share one count, so a test can keep one handle and give another to its
/// harness.
///
/// Compiled only with the `testing` feature.
///
/// # Example
///
/// ```rust,no_run
/// use suprnova::database::testing::StatementCounter;
/// use suprnova::database::{DatabaseConfig, DbConnection};
///
/// # async fn example() -> Result<(), suprnova::FrameworkError> {
/// let config = DatabaseConfig::builder().url("sqlite::memory:").build();
/// let mut conn = DbConnection::connect(&config).await?;
/// // Before the connection is cloned or bound into the container.
/// let statements = StatementCounter::install(&mut conn)?;
///
/// // ... bind `conn`, then dispatch the request under test ...
/// assert_eq!(statements.count(), 0, "the cache answered without the database");
/// # Ok(())
/// # }
/// ```
#[cfg(any(test, feature = "testing"))]
#[derive(Clone, Debug)]
pub struct StatementCounter {
    count: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

#[cfg(any(test, feature = "testing"))]
impl StatementCounter {
    /// Installs a counter on `conn` and returns its handle, starting at
    /// zero. It counts the prepared statements run through the connection;
    /// unprepared SQL and transaction control are not counted.
    ///
    /// Installing needs sole ownership of the connection pool, which a
    /// connection has only before it is cloned or shared: in practice,
    /// immediately after [`DbConnection::connect`]. A second install on the
    /// same connection replaces the first counter, which stops counting.
    ///
    /// # Errors
    ///
    /// Returns a [`FrameworkError`] when `conn` is already shared. Nothing is
    /// installed then, and a counter that read zero would be a false proof.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use suprnova::database::testing::StatementCounter;
    /// use suprnova::database::{DatabaseConfig, DbConnection};
    ///
    /// # async fn example() -> Result<(), suprnova::FrameworkError> {
    /// let config = DatabaseConfig::builder().url("sqlite::memory:").build();
    /// let mut conn = DbConnection::connect(&config).await?;
    /// let statements = StatementCounter::install(&mut conn)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn install(conn: &mut DbConnection) -> Result<Self, FrameworkError> {
        let counter = Self {
            count: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        };
        let observed = std::sync::Arc::clone(&counter.count);
        let installed = conn.observe_statements(move || {
            observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });
        if installed {
            Ok(counter)
        } else {
            Err(FrameworkError::internal(
                "StatementCounter::install: the connection is already shared (cloned, or bound \
                 into the container), so its statement callback cannot be set; install the \
                 counter immediately after DbConnection::connect",
            ))
        }
    }

    /// The number of prepared statements the connection has run since the
    /// install or since the last [`Self::reset`].
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use suprnova::database::testing::StatementCounter;
    /// # fn example(statements: &StatementCounter) {
    /// let before = statements.count();
    /// // ... dispatch the request under test ...
    /// assert_eq!(statements.count(), before, "the request reached no database");
    /// # }
    /// ```
    #[must_use]
    pub fn count(&self) -> u64 {
        self.count.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Sets the count back to zero, so the next reading covers only the
    /// statements that run after this call.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use suprnova::database::testing::StatementCounter;
    /// # fn example(statements: &StatementCounter) {
    /// statements.reset();
    /// // ... dispatch the request under test ...
    /// assert_eq!(statements.count(), 0, "the request reached no database");
    /// # }
    /// ```
    pub fn reset(&self) {
        self.count.store(0, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Create a test database with default migrator
///
/// This macro creates a `TestDatabase` using `crate::migrations::Migrator` as the
/// default migrator. This follows the suprnova convention where migrations are defined
/// in `src/migrations/mod.rs`.
///
/// # Example
///
/// ```rust,ignore
/// use suprnova::test_database;
///
/// #[tokio::test]
/// async fn test_user_creation() {
///     let db = test_database!();
///
///     let action = CreateUserAction::new();
///     let user = action.execute("test@example.com").await.unwrap();
///     assert!(user.id > 0);
/// }
/// ```
///
/// # With Custom Migrator
///
/// ```rust,ignore
/// let db = test_database!(my_crate::CustomMigrator);
/// ```
#[macro_export]
macro_rules! test_database {
    // The migrator lives in the calling crate, the application's
    // `src/migrations/mod.rs`, so the path starts at that crate's root:
    // `crate`, which a `macro_rules!` body resolves where the macro is
    // called. `$crate` would name suprnova, which has no migrations. The
    // root is handed to the arm below as a token so the path is written
    // once, from a fragment.
    () => {
        $crate::test_database!(@root crate)
    };
    // Internal: the no-argument form, with the calling crate's root.
    (@root $root:ident) => {
        $crate::testing::TestDatabase::fresh::<$root::migrations::Migrator>()
            .await
            .expect("Failed to set up test database")
    };
    ($migrator:ty) => {
        $crate::testing::TestDatabase::fresh::<$migrator>()
            .await
            .expect("Failed to set up test database")
    };
}

#[cfg(test)]
mod statement_counter_tests {
    use sea_orm::{ConnectionTrait, Statement};

    use super::StatementCounter;
    use crate::database::{DatabaseConfig, DbConnection};

    async fn connect() -> DbConnection {
        let config = DatabaseConfig::builder()
            .url("sqlite::memory:")
            .max_connections(1)
            .min_connections(1)
            .logging(false)
            .build();
        DbConnection::connect(&config)
            .await
            .expect("connect an in-memory database")
    }

    async fn run_one_statement(conn: &DbConnection) {
        let backend = conn.inner().get_database_backend();
        let row = conn
            .inner()
            .query_one_raw(Statement::from_string(backend, "SELECT 1"))
            .await
            .expect("run a statement");
        assert!(row.is_some(), "SELECT 1 returns one row");
    }

    #[tokio::test]
    async fn it_counts_every_statement_and_resets_to_zero() {
        let mut conn = connect().await;
        let statements =
            StatementCounter::install(&mut conn).expect("an unshared connection takes a counter");
        let shared = statements.clone();
        assert_eq!(statements.count(), 0, "a new counter starts at zero");

        run_one_statement(&conn).await;
        assert_eq!(statements.count(), 1);
        run_one_statement(&conn).await;
        assert_eq!(statements.count(), 2);
        assert_eq!(shared.count(), 2, "a clone reads the same count");

        statements.reset();
        assert_eq!(shared.count(), 0, "one reset clears both handles");
        run_one_statement(&conn).await;
        assert_eq!(statements.count(), 1);
    }

    #[tokio::test]
    async fn unprepared_sql_is_not_counted_and_a_prepared_statement_is() {
        let mut conn = connect().await;
        let statements =
            StatementCounter::install(&mut conn).expect("an unshared connection takes a counter");

        conn.inner()
            .execute_unprepared("CREATE TABLE counted (id INTEGER)")
            .await
            .expect("run unprepared SQL");
        conn.inner()
            .execute_unprepared("SELECT 1")
            .await
            .expect("run unprepared SQL");
        assert_eq!(
            statements.count(),
            0,
            "unprepared SQL leaves the count unchanged"
        );

        run_one_statement(&conn).await;
        assert_eq!(
            statements.count(),
            1,
            "the prepared statement after them is the one counted"
        );
    }

    #[tokio::test]
    async fn a_shared_connection_is_refused_with_an_error() {
        let mut conn = connect().await;
        let _container_copy = conn.clone();

        let error = StatementCounter::install(&mut conn)
            .expect_err("a shared connection cannot take a counter");
        assert!(
            error.to_string().contains("already shared"),
            "the error says why nothing was installed: {error}"
        );
    }
}

#[cfg(test)]
mod test_transaction_tests {
    //! The isolation guard of `transaction_settled`: under `refresh` a
    //! `BEGIN` is a savepoint of the test transaction, where Postgres and
    //! MySQL refuse an isolation level, so the render cache's
    //! `REPEATABLE READ` transaction must open without one.

    use sea_orm::IsolationLevel;
    use sea_orm_migration::prelude::*;

    use super::TestDatabase;
    use crate::database::{DB, DatabaseConfig};
    use crate::error::FrameworkError;

    /// No migration, and a migration table of its own, so an engine run
    /// never races another test's ledger.
    struct NoMigrations;

    impl MigratorTrait for NoMigrations {
        fn migrations() -> Vec<Box<dyn MigrationTrait>> {
            Vec::new()
        }

        fn migration_table_name() -> DynIden {
            Alias::new("suprnova_isolation_guard_ledger").into_iden()
        }
    }

    async fn transaction_with_isolation_inside_the_test_transaction(url: String) {
        let config = DatabaseConfig::builder().url(url).logging(false).build();
        let db =
            TestDatabase::transactional::<NoMigrations>("TestDatabase::refresh", config, false)
                .await
                .expect("hold a test transaction");
        assert!(db.db().in_test_transaction());

        let (value, callback_error) =
            DB::transaction_with_isolation(Some(IsolationLevel::RepeatableRead), |_tx| {
                Box::pin(async { Ok::<_, FrameworkError>(7) })
            })
            .await
            .expect("the savepoint opens without setting an isolation level")
            .expect("the closure ran and the savepoint was released");
        assert_eq!(value, 7);
        assert!(callback_error.is_none());
    }

    #[tokio::test]
    async fn a_transaction_with_an_isolation_level_is_a_savepoint_of_the_test_transaction() {
        let dir = tempfile::tempdir().expect("create a temporary directory");
        transaction_with_isolation_inside_the_test_transaction(format!(
            "sqlite://{}",
            dir.path().join("isolation.db").display()
        ))
        .await;
    }

    #[tokio::test]
    async fn a_pool_of_its_own_is_not_a_test_transaction() {
        let db = TestDatabase::fresh::<NoMigrations>()
            .await
            .expect("an in-memory database");
        assert!(!db.db().in_test_transaction());
    }

    #[tokio::test]
    #[ignore = "requires a disposable Postgres at PG_TEST_URL"]
    async fn postgres_a_transaction_with_an_isolation_level_is_a_savepoint_of_the_test_transaction()
    {
        let url = std::env::var("PG_TEST_URL").expect("set PG_TEST_URL");
        transaction_with_isolation_inside_the_test_transaction(url).await;
    }

    #[tokio::test]
    #[ignore = "requires a disposable MariaDB/MySQL at MYSQL_TEST_URL"]
    async fn mysql_a_transaction_with_an_isolation_level_is_a_savepoint_of_the_test_transaction() {
        let url = std::env::var("MYSQL_TEST_URL").expect("set MYSQL_TEST_URL");
        transaction_with_isolation_inside_the_test_transaction(url).await;
    }
}
