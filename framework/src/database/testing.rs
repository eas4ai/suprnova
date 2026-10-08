//! Testing utilities for database operations
//!
//! Provides `TestDatabase` for setting up isolated test environments with
//! in-memory SQLite databases and automatic migration support.
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

use sea_orm::DatabaseConnection;
use sea_orm_migration::MigratorTrait;

use super::config::DatabaseConfig;
use super::connection::DbConnection;
use crate::container::testing::{TestContainer, TestContainerGuard};
use crate::error::FrameworkError;

/// Test database wrapper that provides isolated database environments
///
/// Each `TestDatabase` creates a fresh in-memory SQLite database with
/// migrations applied. The database is automatically registered in the
/// test container, so any code using `DB::connection()` or `#[inject] db: Database`
/// will receive this test database.
///
/// When the `TestDatabase` is dropped, the test container is cleared,
/// ensuring complete isolation between tests.
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
        })
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
