//! `DB::monitor` and `db:monitor` on SQLite, which has no server that
//! counts connections, and `DbConnection::connections_in_use`.
//!
//! The count of a server is tested against a live PostgreSQL and a live
//! MySQL, beside the code in `framework/src/database/monitor.rs`.

use serial_test::serial;
use suprnova::database::{ConnectionCount, ConnectionRegistry, DatabaseConfig, DbConnection};
use suprnova::events::testing::dispatched_count;
use suprnova::testing::TestDatabase;
use suprnova::{DB, DatabaseBusy, EventFacade, PRIMARY_CONNECTION_NAME, console};

async fn one_connection() -> DbConnection {
    let config = DatabaseConfig::builder()
        .url("sqlite::memory:")
        .max_connections(1)
        .min_connections(1)
        .logging(false)
        .build();
    DbConnection::connect(&config)
        .await
        .expect("an in-memory database")
}

/// Wait until the pool has `expected` connections in use.
///
/// A connection that is given back is given back by a task of the pool,
/// after the transaction that held it has returned. So this lets the
/// runtime run until the count is there, and fails when it is not there
/// in time.
async fn settles_at(connection: &DbConnection, expected: u32) {
    let settled = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while connection.connections_in_use() != expected {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert!(
        settled.is_ok(),
        "the pool has {} connections in use, expected {expected}",
        connection.connections_in_use()
    );
}

#[tokio::test]
#[serial]
async fn a_connection_in_use_is_counted_until_it_is_given_back() {
    let db = TestDatabase::sqlite_memory().await.expect("a database");
    settles_at(db.db(), 0).await;

    let held = DB::begin_transaction().await.expect("a transaction");
    assert_eq!(db.db().connections_in_use(), 1);

    held.commit().await.expect("a commit");
    settles_at(db.db(), 0).await;
}

#[tokio::test]
#[serial]
async fn a_database_with_no_server_is_listed_and_never_busy() {
    let db = TestDatabase::sqlite_memory().await.expect("a database");
    let _events = EventFacade::fake();
    ConnectionRegistry::register_existing("reports", one_connection().await)
        .await
        .expect("a name that is free");

    assert_eq!(db.db().server_connections().await.expect("no server"), None);
    assert_eq!(
        DB::connection_counts().await.expect("the connections"),
        [
            ConnectionCount {
                connection_name: PRIMARY_CONNECTION_NAME.to_owned(),
                connections: None
            },
            ConnectionCount {
                connection_name: "reports".to_owned(),
                connections: None
            },
        ],
        "the default connection first, and the named ones by name"
    );

    // A connection in use of the pool is nothing the server counted.
    let held = DB::begin_transaction().await.expect("a transaction");
    assert!(DB::monitor(1).await.expect("the check runs").is_empty());
    assert_eq!(dispatched_count::<DatabaseBusy>(|_| true), 0);
    held.rollback().await.expect("a rollback");

    ConnectionRegistry::clear();
}

#[tokio::test]
#[serial]
async fn a_limit_of_zero_is_refused() {
    let _db = TestDatabase::sqlite_memory().await.expect("a database");

    let error = DB::monitor(0).await.expect_err("every database is at 0");
    assert!(error.to_string().contains("over 0"), "{error}");

    let run = console::test(["db:monitor", "--max", "0"]).run().await;
    run.assert_failed().assert_errors_contain("--max");
}

#[tokio::test]
#[serial]
async fn the_console_command_lists_the_connections() {
    let _db = TestDatabase::sqlite_memory().await.expect("a database");
    let _events = EventFacade::fake();

    for argv in [vec!["db:monitor"], vec!["db:monitor", "--max", "5"]] {
        let run = console::test(argv.clone()).run().await;

        run.assert_successful();
        let line = run
            .output()
            .lines()
            .find(|line| line.contains(PRIMARY_CONNECTION_NAME))
            .unwrap_or_else(|| panic!("{argv:?} lists the connection:\n{}", run.output()));
        assert!(line.ends_with(" not counted, no server"), "{line}");
    }
    assert_eq!(dispatched_count::<DatabaseBusy>(|_| true), 0);
}

#[tokio::test]
#[serial]
async fn the_console_command_fails_when_there_is_no_database() {
    let _container = suprnova::testing::TestContainer::fake();

    let run = console::test(["db:monitor", "--max", "5"]).run().await;

    run.assert_failed()
        .assert_errors_contain("no database connection");
    assert_eq!(run.output(), "");
}
