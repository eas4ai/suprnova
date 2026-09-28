//! The count of the connections to a database, and the event for a
//! database that has too many.
//!
//! A database server takes a number of connections and refuses the next
//! one. [`DB::monitor`] asks the server of each connection how many it
//! has, from every client, and dispatches [`DatabaseBusy`] for a server
//! at or over the number the application gives. It is the check of
//! Laravel's `db:monitor`, and the console has the command as well.
//!
//! The server does the counting, so the check gives the same answer from
//! every process that reaches the database. That is what lets it run on
//! the schedule, which is a process of its own:
//!
//! ```rust,no_run
//! # use suprnova::Schedule;
//! # fn ex(schedule: &mut Schedule) {
//! schedule.add(schedule.command("db:monitor --max 80").every_minute());
//! # }
//! ```
//!
//! Nothing runs the check by itself.
//!
//! [`DbConnection::connections_in_use`] is another number: the
//! connections of the pool of this process that are out of the pool now.
//! It says nothing about another process.

use super::events::DatabaseBusy;
use super::{ConnectionRegistry, DB, DbConnection, PRIMARY_CONNECTION_NAME};
use crate::error::FrameworkError;
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};

/// The connections a database server has, as one connection of the
/// application sees them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionCount {
    /// The name of the connection: `__primary__` for the default one, and
    /// the name a connection was registered under otherwise.
    pub connection_name: String,
    /// The connections the server has, from every client. `None` for a
    /// database with no server to count them, which SQLite is.
    pub connections: Option<u32>,
}

/// The connections of a pool that are out of it: the ones it has opened,
/// less the ones that are idle.
#[cfg(any(
    feature = "database-postgres",
    feature = "database-mysql",
    feature = "database-sqlite"
))]
fn out_of_the_pool(size: u32, idle: usize) -> u32 {
    size.saturating_sub(u32::try_from(idle).unwrap_or(u32::MAX))
}

/// The connections of the pool behind `connection` that are in use.
///
/// The pool is reached through the variant of the connection. Its
/// accessors panic on a connection of another kind, and a mock
/// connection has a backend and no pool.
fn in_use(connection: &DatabaseConnection) -> u32 {
    match &connection.inner {
        #[cfg(feature = "database-postgres")]
        sea_orm::DatabaseConnectionType::SqlxPostgresPoolConnection(_) => {
            let pool = connection.get_postgres_connection_pool();
            out_of_the_pool(pool.size(), pool.num_idle())
        }
        #[cfg(feature = "database-mysql")]
        sea_orm::DatabaseConnectionType::SqlxMySqlPoolConnection(_) => {
            let pool = connection.get_mysql_connection_pool();
            out_of_the_pool(pool.size(), pool.num_idle())
        }
        #[cfg(feature = "database-sqlite")]
        sea_orm::DatabaseConnectionType::SqlxSqlitePoolConnection(_) => {
            let pool = connection.get_sqlite_connection_pool();
            out_of_the_pool(pool.size(), pool.num_idle())
        }
        // No pool that this build knows: a mock, or a database whose
        // feature of this crate is off.
        _ => 0,
    }
}

/// The statement that asks a server for the number of its connections.
/// They are the statements of Laravel's `compileThreadCount`.
fn count_statement(backend: DbBackend, is_mariadb: bool) -> Option<&'static str> {
    match backend {
        DbBackend::Postgres => Some(r#"select count(*) as "Value" from pg_stat_activity"#),
        DbBackend::MySql if is_mariadb => Some(
            "select variable_value as `Value` from information_schema.global_status \
             where variable_name = 'THREADS_CONNECTED'",
        ),
        DbBackend::MySql => Some(
            "select variable_value as `Value` from performance_schema.session_status \
             where variable_name = 'threads_connected'",
        ),
        _ => None,
    }
}

fn count_failed(what: &str, error: impl std::fmt::Display) -> FrameworkError {
    FrameworkError::database(format!(
        "the connections of the database could not be counted: {what}: {error}"
    ))
}

async fn server_connections(
    connection: &DatabaseConnection,
) -> Result<Option<u32>, FrameworkError> {
    let backend = connection.get_database_backend();
    // MariaDB and MySQL are one backend and keep the number in two places.
    let is_mariadb = if backend == DbBackend::MySql {
        let row = connection
            .query_one_raw(Statement::from_string(backend, "select version()"))
            .await
            .map_err(|e| count_failed("the version of the server", e))?;
        let version: String = match row {
            Some(row) => row
                .try_get_by_index(0)
                .map_err(|e| count_failed("the version of the server", e))?,
            None => String::new(),
        };
        version.to_ascii_lowercase().contains("mariadb")
    } else {
        false
    };
    let Some(sql) = count_statement(backend, is_mariadb) else {
        return Ok(None);
    };

    let row = connection
        .query_one_raw(Statement::from_string(backend, sql))
        .await
        .map_err(|e| count_failed("the query", e))?
        .ok_or_else(|| count_failed("the query", "the server returned no row"))?;
    let count = match backend {
        DbBackend::Postgres => {
            let count: i64 = row
                .try_get_by_index(0)
                .map_err(|e| count_failed("the count", e))?;
            u32::try_from(count).map_err(|e| count_failed("the count", e))?
        }
        _ => {
            let count: String = row
                .try_get_by_index(0)
                .map_err(|e| count_failed("the count", e))?;
            count
                .trim()
                .parse::<u32>()
                .map_err(|e| count_failed("the count", e))?
        }
    };
    Ok(Some(count))
}

impl DbConnection {
    /// The connections of the pool of this process that are in use now:
    /// the ones the pool has opened, less the ones that are idle.
    ///
    /// A connection is given back by a task of the pool, a moment after
    /// the query or the transaction that held it has returned. A read
    /// right behind one may still count it.
    ///
    /// The number is 0 for a connection with no pool this build knows,
    /// which is a mock connection, and a database whose feature of this
    /// crate is off while another crate turned the driver on.
    pub fn connections_in_use(&self) -> u32 {
        in_use(self.inner())
    }

    /// The connections the database server has, from every client, and
    /// `None` for a database with no server, which SQLite is.
    ///
    /// PostgreSQL counts the rows of `pg_stat_activity`, which has the
    /// workers of the server in it as well. MySQL and MariaDB give
    /// `threads_connected`.
    ///
    /// # Errors
    ///
    /// When the server cannot be asked or gives no number.
    pub async fn server_connections(&self) -> Result<Option<u32>, FrameworkError> {
        server_connections(self.inner()).await
    }
}

impl DB {
    /// The connections of the server of every connection the application
    /// has: the default connection first, and every named connection, by
    /// name.
    ///
    /// # Errors
    ///
    /// When the registry of the named connections cannot be read, and
    /// when a server cannot be asked.
    pub async fn connection_counts() -> Result<Vec<ConnectionCount>, FrameworkError> {
        let mut connections = Vec::new();
        if DB::is_connected() {
            connections.push((PRIMARY_CONNECTION_NAME.to_owned(), DB::connection()?));
        }
        let mut named = ConnectionRegistry::all().await?;
        named.sort_by(|a, b| a.0.cmp(&b.0));
        connections.extend(named);

        let mut counts = Vec::with_capacity(connections.len());
        for (connection_name, connection) in connections {
            counts.push(ConnectionCount {
                connection_name,
                connections: connection.server_connections().await?,
            });
        }
        Ok(counts)
    }

    /// Dispatch [`DatabaseBusy`] for every connection whose server has
    /// `max` connections or more, and return the events that were
    /// dispatched. A connection with no server to count is left out.
    ///
    /// A listener is where the application decides what a busy database
    /// means for it: a page to whoever is on call, a metric, a log line.
    /// A listener that fails does not stop the check. Its error is
    /// logged, and the connections behind it are still looked at.
    ///
    /// # Errors
    ///
    /// When `max` is 0, which every server is at or over. Every error of
    /// [`Self::connection_counts`].
    pub async fn monitor(max: u32) -> Result<Vec<DatabaseBusy>, FrameworkError> {
        let counts = Self::connection_counts().await?;
        Self::dispatch_busy(&counts, max).await
    }

    /// [`Self::monitor`] on counts that were taken already, so a caller
    /// that shows the counts judges the ones it shows.
    pub(crate) async fn dispatch_busy(
        counts: &[ConnectionCount],
        max: u32,
    ) -> Result<Vec<DatabaseBusy>, FrameworkError> {
        if max == 0 {
            return Err(FrameworkError::internal(
                "DB::monitor needs a limit over 0: every database has 0 connections or more",
            ));
        }
        let mut busy = Vec::new();
        for count in counts {
            let Some(connections) = count.connections.filter(|connections| *connections >= max)
            else {
                continue;
            };
            let event = DatabaseBusy {
                connection_name: count.connection_name.clone(),
                connections,
            };
            if let Err(e) = crate::EventFacade::dispatch_best_effort(event.clone()).await {
                tracing::warn!(
                    target: "suprnova::database",
                    connection = %event.connection_name,
                    error = %e,
                    "DatabaseBusy listener returned error; ignoring",
                );
            }
            busy.push(event);
        }
        Ok(busy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::testing::{TestContainer, TestContainerGuard};
    use crate::events::testing::{assert_dispatched, dispatched_count};

    #[test]
    fn each_server_is_asked_the_way_laravel_asks_it() {
        assert_eq!(
            count_statement(DbBackend::Postgres, false),
            Some(r#"select count(*) as "Value" from pg_stat_activity"#)
        );
        let mysql = count_statement(DbBackend::MySql, false).expect("a statement");
        assert!(
            mysql.contains("performance_schema.session_status"),
            "{mysql}"
        );
        let mariadb = count_statement(DbBackend::MySql, true).expect("a statement");
        assert!(
            mariadb.contains("information_schema.global_status"),
            "{mariadb}"
        );
        assert_eq!(count_statement(DbBackend::Sqlite, false), None);
    }

    // Live database tests. The gate runs them with
    //
    //   PG_TEST_URL=... cargo test -p suprnova --lib -- --ignored live_postgres
    //   MYSQL_TEST_URL=... cargo test -p suprnova --lib -- --ignored live_mysql

    async fn connect(url: &str) -> (TestContainerGuard, DbConnection) {
        let mut options = sea_orm::ConnectOptions::new(url.to_owned());
        options
            .connect_timeout(std::time::Duration::from_secs(2))
            .acquire_timeout(std::time::Duration::from_secs(2));
        let connection = DbConnection::from_raw(
            sea_orm::Database::connect(options)
                .await
                .expect("the live test database is not reachable - check the URL"),
        );
        let guard = TestContainer::fake();
        TestContainer::singleton(connection.clone());
        (guard, connection)
    }

    async fn assert_the_server_is_counted_and_judged(url: &str) {
        let (_container, connection) = connect(url).await;
        let _events = crate::EventFacade::fake();

        let counted = connection
            .server_connections()
            .await
            .expect("the server answers")
            .expect("a server counts its connections");
        assert!(counted >= 1, "this connection is one of them: {counted}");

        // The count moves with the other clients of the server, so the
        // limits are the two ends: one that every server is at, and one
        // that no server reaches.
        let busy = DB::monitor(1).await.expect("the servers are asked");
        assert_eq!(busy.len(), 1);
        assert_eq!(busy[0].connection_name, PRIMARY_CONNECTION_NAME);
        assert!(busy[0].connections >= 1);
        assert_dispatched::<DatabaseBusy>(|event| {
            event.connection_name == PRIMARY_CONNECTION_NAME && event.connections >= 1
        });

        assert!(
            DB::monitor(u32::MAX)
                .await
                .expect("the servers are asked")
                .is_empty()
        );
        assert_eq!(dispatched_count::<DatabaseBusy>(|_| true), 1);
    }

    #[tokio::test]
    #[ignore = "requires live Postgres; run with --ignored live_postgres"]
    async fn live_postgres_the_server_is_counted_and_judged() {
        let url = std::env::var("PG_TEST_URL").expect("set PG_TEST_URL to a Postgres");
        assert_the_server_is_counted_and_judged(&url).await;
    }

    #[tokio::test]
    #[ignore = "requires live MySQL; run with --ignored live_mysql"]
    async fn live_mysql_the_server_is_counted_and_judged() {
        let url = std::env::var("MYSQL_TEST_URL").expect("set MYSQL_TEST_URL to a MySQL");
        assert_the_server_is_counted_and_judged(&url).await;
    }
}
