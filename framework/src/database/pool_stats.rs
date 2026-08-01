//! Connection-pool gauges.
//!
//! Two numbers — how many connections the pool holds and how many are
//! idle — read live from the underlying sqlx pool. See
//! [`DB::pool_stats`](crate::DB::pool_stats) for why they are worth
//! surfacing and what their sampling caveat is.

use sea_orm::DatabaseConnection;

/// A point-in-time sample of the connection pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct PoolStats {
    /// Connections the pool currently holds, idle and busy together.
    ///
    /// This is not the configured maximum — sqlx opens connections
    /// lazily, so a pool with `max_connections = 100` that has never been
    /// busy reports a much smaller size. A `size` pinned at the
    /// configured maximum with `idle` at zero is the signature of a
    /// saturated pool.
    pub size: u32,
    /// Connections available to be checked out right now.
    pub idle: usize,
}

impl PoolStats {
    /// Connections currently checked out.
    ///
    /// Saturating, because `size` and `idle` are read separately from a
    /// live pool: under churn a connection can be returned between the
    /// two reads, making `idle` momentarily exceed `size`. A gauge that
    /// panicked on that would fail precisely when the pool was busiest,
    /// which is the only time anyone reads it.
    pub fn in_use(&self) -> u32 {
        self.size.saturating_sub(self.idle as u32)
    }

    /// Read the gauges from a SeaORM connection.
    ///
    /// Matches the connection's concrete variant rather than its
    /// [`DatabaseBackend`](sea_orm::DatabaseBackend). The accessors below
    /// panic when handed the wrong kind of connection, and a
    /// `MockDatabase` reports whichever backend it was configured with
    /// while holding no pool at all — so dispatching on the backend would
    /// turn every mock-backed test that touched this into a panic.
    pub(crate) fn read(conn: &DatabaseConnection) -> Option<Self> {
        #[allow(unreachable_patterns)]
        match conn {
            #[cfg(feature = "database-postgres")]
            DatabaseConnection::SqlxPostgresPoolConnection(_) => {
                let pool = conn.get_postgres_connection_pool();
                Some(Self {
                    size: pool.size(),
                    idle: pool.num_idle(),
                })
            }
            #[cfg(feature = "database-mysql")]
            DatabaseConnection::SqlxMySqlPoolConnection(_) => {
                let pool = conn.get_mysql_connection_pool();
                Some(Self {
                    size: pool.size(),
                    idle: pool.num_idle(),
                })
            }
            #[cfg(feature = "database-sqlite")]
            DatabaseConnection::SqlxSqlitePoolConnection(_) => {
                let pool = conn.get_sqlite_connection_pool();
                Some(Self {
                    size: pool.size(),
                    idle: pool.num_idle(),
                })
            }
            // A mock connection, a disconnected handle, or a backend this
            // build was not compiled with. `None` rather than zeroes:
            // "no pool to read" and "a pool with nothing in it" are
            // different facts, and a caller plotting saturation needs to
            // be able to tell them apart.
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_use_is_size_minus_idle() {
        let stats = PoolStats { size: 10, idle: 3 };
        assert_eq!(stats.in_use(), 7);
    }

    #[test]
    fn in_use_saturates_when_idle_exceeds_size() {
        // Not hypothetical: the two counters are read from a live pool in
        // separate calls, so a connection returned between them produces
        // exactly this.
        let stats = PoolStats { size: 2, idle: 3 };
        assert_eq!(stats.in_use(), 0, "must saturate, not underflow");
    }

    #[test]
    fn an_idle_pool_reports_nothing_in_use() {
        let stats = PoolStats { size: 5, idle: 5 };
        assert_eq!(stats.in_use(), 0);
    }

    /// A handle with no pool behind it must read as `None`, not panic.
    /// The pool accessors this dispatches to panic when handed the wrong
    /// kind of connection, so the variant match is what stands between a
    /// `/debug` endpoint and a 500 on a disconnected handle.
    #[test]
    fn a_connection_with_no_pool_reports_none() {
        assert_eq!(PoolStats::read(&DatabaseConnection::Disconnected), None);
    }

    #[tokio::test]
    #[cfg(feature = "database-sqlite")]
    async fn a_real_sqlite_pool_reports_its_gauges() {
        let conn = sea_orm::Database::connect("sqlite::memory:")
            .await
            .expect("connect sqlite::memory:");
        let stats = PoolStats::read(&conn).expect("a real pool must report gauges");
        assert!(stats.size >= 1, "an established pool holds a connection");
        assert!(stats.idle <= stats.size as usize + 1);
    }
}
