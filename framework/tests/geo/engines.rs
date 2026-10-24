use std::time::Duration;

use sea_orm::{ConnectOptions, Database};
use suprnova::DbConnection;
use suprnova::testing::TestContainer;

use super::{assert_shared_contract, execute, seed};

async fn engine(variable: &str) {
    let url = std::env::var(variable).expect("set the test URL to a disposable engine database");
    let mut options = ConnectOptions::new(url);
    options
        .max_connections(2)
        .min_connections(0)
        .connect_timeout(Duration::from_secs(2))
        .acquire_timeout(Duration::from_secs(2));
    let connection = Database::connect(options)
        .await
        .expect("test engine reachable");
    let _guard = TestContainer::fake();
    TestContainer::singleton(DbConnection::from_raw(connection.clone()));
    suprnova::context::Context::test_clear_query();
    seed(&connection).await;
    assert_shared_contract().await;
    execute(&connection, "DROP TABLE geo_notes").await;
    execute(&connection, "DROP TABLE geo_places").await;
}

#[tokio::test]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn geo_001_fixture_postgres() {
    engine("PG_TEST_URL").await;
}

#[tokio::test]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn geo_001_fixture_mysql() {
    engine("MYSQL_TEST_URL").await;
}

#[tokio::test]
#[ignore = "requires disposable MariaDB at MARIADB_TEST_URL or MYSQL_TEST_URL"]
async fn geo_001_fixture_mariadb() {
    let variable = if std::env::var_os("MARIADB_TEST_URL").is_some() {
        "MARIADB_TEST_URL"
    } else {
        "MYSQL_TEST_URL"
    };
    engine(variable).await;
}
