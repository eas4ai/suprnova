//! Task-local default connection routing for table and raw queries.

use sea_orm::ConnectionTrait;
use suprnova::testing::TestDatabase;
use suprnova::{ConnectionRegistry, DB, DbConnection, FrameworkError, attrs};

async fn pools() -> (TestDatabase, String) {
    let primary = TestDatabase::sqlite_memory().await.expect("primary");
    primary
        .execute_unprepared(
            "CREATE TABLE delta_connections (id INTEGER PRIMARY KEY, label TEXT NOT NULL)",
        )
        .await
        .expect("table");
    primary
        .execute_unprepared("INSERT INTO delta_connections VALUES (1, 'primary')")
        .await
        .expect("seed");
    let reporting = DbConnection::from_raw(
        sea_orm::Database::connect("sqlite::memory:")
            .await
            .expect("reporting"),
    );
    reporting
        .inner()
        .execute_unprepared(
            "CREATE TABLE delta_connections (id INTEGER PRIMARY KEY, label TEXT NOT NULL)",
        )
        .await
        .expect("table");
    reporting
        .inner()
        .execute_unprepared("INSERT INTO delta_connections VALUES (1, 'reporting')")
        .await
        .expect("seed");
    let name = format!("reporting_{}", uuid::Uuid::new_v4());
    ConnectionRegistry::register_existing(&name, reporting)
        .await
        .expect("register");
    (primary, name)
}

async fn label() -> String {
    DB::table("delta_connections")
        .first()
        .await
        .expect("query")
        .expect("row")
        .get_string("label")
        .expect("label")
}

#[tokio::test]
async fn scope_routes_tables_raw_queries_direct_connections_and_transactions() {
    let (_primary, reporting) = pools().await;
    assert_eq!(DB::default_connection(), suprnova::PRIMARY_CONNECTION_NAME);
    DB::with_default_connection(&reporting, async {
        assert_eq!(DB::default_connection(), reporting);
        assert_eq!(label().await, "reporting");
        let raw: String = DB::scalar("SELECT label FROM delta_connections", vec![]).await?;
        assert_eq!(raw, "reporting");
        DB::connection()?
            .inner()
            .execute_unprepared("INSERT INTO delta_connections VALUES (2, 'direct')")
            .await
            .map_err(|e| FrameworkError::database(e.to_string()))?;
        DB::table("delta_connections")
            .insert(attrs! { id: 3, label: "table" })
            .await?;
        DB::transaction(|_tx| {
            Box::pin(async {
                DB::table("delta_connections")
                    .insert(attrs! { id: 4, label: "transaction" })
                    .await?;
                Ok(())
            })
        })
        .await?;
        assert_eq!(DB::table("delta_connections").count().await?, 4);
        Ok(())
    })
    .await
    .expect("scope");
    assert_eq!(DB::default_connection(), suprnova::PRIMARY_CONNECTION_NAME);
    assert_eq!(
        DB::table("delta_connections")
            .count()
            .await
            .expect("primary count"),
        1
    );
}

#[tokio::test]
async fn concurrent_tasks_keep_their_own_default_and_nested_scopes_restore_it() {
    let (_primary, reporting) = pools().await;
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let other_barrier = barrier.clone();
    let other = tokio::spawn(async move {
        other_barrier.wait().await;
        assert_eq!(DB::default_connection(), suprnova::PRIMARY_CONNECTION_NAME);
        assert_eq!(label().await, "primary");
        other_barrier.wait().await;
    });
    DB::with_default_connection(&reporting, async {
        barrier.wait().await;
        assert_eq!(label().await, "reporting");
        DB::with_default_connection(suprnova::PRIMARY_CONNECTION_NAME, async {
            assert_eq!(label().await, "primary");
            Ok(())
        })
        .await?;
        assert_eq!(DB::default_connection(), reporting);
        barrier.wait().await;
        Ok(())
    })
    .await
    .expect("scope");
    other.await.expect("concurrent task");
    assert_eq!(DB::default_connection(), suprnova::PRIMARY_CONNECTION_NAME);
}

#[tokio::test]
async fn scope_errors_restore_default_and_unknown_connection_never_polls_future() {
    let (_primary, reporting) = pools().await;
    let result = DB::with_default_connection(&reporting, async {
        assert_eq!(label().await, "reporting");
        Err::<(), _>(FrameworkError::internal("operation failed"))
    })
    .await;
    assert_eq!(result.expect_err("failure").message(), "operation failed");
    assert_eq!(DB::default_connection(), suprnova::PRIMARY_CONNECTION_NAME);
    let polled = std::sync::atomic::AtomicBool::new(false);
    assert!(
        DB::with_default_connection("missing_delta_connection", async {
            polled.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok::<(), FrameworkError>(())
        })
        .await
        .is_err()
    );
    assert!(!polled.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(label().await, "primary");
}

#[tokio::test]
async fn cancelling_or_panicking_a_scope_restores_the_enclosing_default() {
    use futures::FutureExt;
    let (_primary, reporting) = pools().await;
    let cancelled = tokio::time::timeout(
        std::time::Duration::from_millis(10),
        DB::with_default_connection(&reporting, async {
            assert_eq!(DB::default_connection(), reporting);
            std::future::pending::<Result<(), FrameworkError>>().await
        }),
    )
    .await;
    assert!(cancelled.is_err());
    assert_eq!(DB::default_connection(), suprnova::PRIMARY_CONNECTION_NAME);
    let panicked =
        std::panic::AssertUnwindSafe(DB::with_default_connection::<_, ()>(&reporting, async {
            assert_eq!(DB::default_connection(), reporting);
            std::future::ready(()).await;
            panic!("scope panic")
        }))
        .catch_unwind()
        .await;
    assert!(panicked.is_err());
    assert_eq!(DB::default_connection(), suprnova::PRIMARY_CONNECTION_NAME);
}

#[tokio::test]
async fn an_explicit_primary_scope_precedes_the_automatic_read_replica() {
    let (_primary, reporting) = pools().await;
    let replica = DbConnection::from_raw(
        sea_orm::Database::connect("sqlite::memory:")
            .await
            .expect("replica"),
    );
    replica
        .inner()
        .execute_unprepared(
            "CREATE TABLE delta_connections (id INTEGER PRIMARY KEY, label TEXT NOT NULL)",
        )
        .await
        .expect("table");
    replica
        .inner()
        .execute_unprepared("INSERT INTO delta_connections VALUES (1, 'replica')")
        .await
        .expect("seed");
    ConnectionRegistry::register_existing(suprnova::READ_REPLICA_CONNECTION_NAME, replica)
        .await
        .expect("register replica");
    assert_eq!(label().await, "replica");
    DB::with_default_connection(&reporting, async {
        assert_eq!(label().await, "reporting");
        DB::with_default_connection(suprnova::PRIMARY_CONNECTION_NAME, async {
            assert_eq!(label().await, "primary");
            Ok(())
        })
        .await?;
        assert_eq!(label().await, "reporting");
        Ok(())
    })
    .await
    .expect("scope");
    assert_eq!(label().await, "replica");
}
