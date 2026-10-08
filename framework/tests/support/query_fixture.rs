//! A connection for query-builder tests: an in-memory SQLite database by
//! default, or one connection to a disposable server for the `postgres_`
//! and `mysql_` variants.
//!
//! The live variants create their tables as `TEMPORARY` on a single
//! connection, so a run leaves nothing behind on the server, and two
//! tests running at once never see each other's rows. SQLite accepts the
//! same statements.

use sea_orm::{ConnectionTrait, DatabaseBackend};
use serde_json::Value;
use suprnova::DynamicRow;
use suprnova::database::{DatabaseConfig, DbConnection};
use suprnova::testing::{TestContainer, TestContainerGuard, TestDatabase};

enum Holder {
    Sqlite(TestDatabase),
    Live(TestContainerGuard, DbConnection),
}

/// The database a scenario runs against, installed as the default
/// connection for the test's duration.
pub struct Fixture {
    holder: Holder,
    /// The backend the statements run on.
    pub backend: DatabaseBackend,
}

impl Fixture {
    /// A fresh in-memory SQLite database.
    pub async fn sqlite() -> Self {
        let db = TestDatabase::sqlite_memory()
            .await
            .expect("in-memory SQLite");
        Self {
            holder: Holder::Sqlite(db),
            backend: DatabaseBackend::Sqlite,
        }
    }

    /// One connection to the server named by the environment variable
    /// `env`. Panics when it is unset: these tests never pick a database
    /// on their own.
    pub async fn live(env: &str, backend: DatabaseBackend) -> Self {
        let url = std::env::var(env).expect("explicit disposable database URL required");
        let guard = TestContainer::fake();
        let config = DatabaseConfig::builder()
            .url(url)
            .max_connections(1)
            .min_connections(1)
            .logging(false)
            .build();
        let connection = DbConnection::connect(&config)
            .await
            .expect("connect test database");
        TestContainer::singleton(connection.clone());
        Self {
            holder: Holder::Live(guard, connection),
            backend,
        }
    }

    fn connection(&self) -> &DbConnection {
        match &self.holder {
            Holder::Sqlite(db) => db.db(),
            Holder::Live(_, connection) => connection,
        }
    }

    /// Run one statement with no bindings.
    pub async fn exec(&self, sql: &str) {
        self.connection()
            .inner()
            .execute_unprepared(sql)
            .await
            .unwrap_or_else(|e| panic!("fixture statement failed: {sql}: {e}"));
    }

    /// The character the backend quotes identifiers with.
    pub fn quote(&self) -> char {
        if self.backend == DatabaseBackend::MySql {
            '`'
        } else {
            '"'
        }
    }

    /// Close a live connection, which drops its temporary tables.
    pub async fn close(self) {
        if let Holder::Live(guard, connection) = self.holder {
            drop(guard);
            connection
                .inner()
                .clone()
                .close()
                .await
                .expect("close test connection");
        }
    }
}

/// Rows as plain JSON objects, so two result sets compare with `==`.
pub fn json_rows(rows: Vec<DynamicRow>) -> Vec<Value> {
    rows.into_iter()
        .map(|row| Value::Object(row.into_map()))
        .collect()
}
