//! PostgreSQL regression coverage for Eloquent aggregate aliases and decoding.

use sea_orm::{ConnectOptions, ConnectionTrait, Database, Statement};
use std::time::Duration;
use suprnova::testing::TestContainer;
use suprnova::{DbConnection, Model, model};

#[model(table = "suprnova_aggregate_probe", timestamps = false)]
pub struct AggregateProbe {
    pub id: i64,
    pub category: String,
    pub amount: f64,
}

async fn connect_postgres() -> sea_orm::DatabaseConnection {
    let url = std::env::var("PG_TEST_URL").expect("set PG_TEST_URL to a disposable Postgres");
    let mut options = ConnectOptions::new(url);
    options
        .max_connections(2)
        .min_connections(0)
        .connect_timeout(Duration::from_secs(2))
        .acquire_timeout(Duration::from_secs(2));
    Database::connect(options)
        .await
        .expect("Postgres test database must be reachable")
}

#[tokio::test]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_aggregates_decode_by_a_stable_alias() {
    let conn = connect_postgres().await;
    let backend = conn.get_database_backend();
    for sql in [
        "DROP TABLE IF EXISTS suprnova_aggregate_probe",
        "CREATE TABLE suprnova_aggregate_probe (\
             id BIGINT PRIMARY KEY,\
             category TEXT NOT NULL,\
             amount DOUBLE PRECISION NOT NULL\
         )",
        "INSERT INTO suprnova_aggregate_probe (id, category, amount) VALUES \
             (1, 'kept', 10.0), (2, 'kept', 20.0), (3, 'other', 30.0)",
    ] {
        conn.execute_raw(Statement::from_string(backend, sql.to_string()))
            .await
            .expect("create aggregate fixture");
    }

    let _guard = TestContainer::fake();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));

    assert_eq!(AggregateProbe::count().await.unwrap(), 3);
    assert_eq!(
        AggregateProbe::query()
            .filter("category", "kept")
            .count()
            .await
            .unwrap(),
        2
    );
    assert_eq!(AggregateProbe::sum::<f64>("amount").await.unwrap(), 60.0);
    assert_eq!(AggregateProbe::avg::<f64>("amount").await.unwrap(), 20.0);
    assert_eq!(
        AggregateProbe::min::<f64>("amount").await.unwrap(),
        Some(10.0)
    );
    assert_eq!(
        AggregateProbe::max::<f64>("amount").await.unwrap(),
        Some(30.0)
    );

    let empty = AggregateProbe::query().filter("category", "missing");
    assert_eq!(empty.clone().sum::<f64>("amount").await.unwrap(), 0.0);
    assert_eq!(empty.clone().avg::<f64>("amount").await.unwrap(), 0.0);
    assert_eq!(empty.clone().min::<f64>("amount").await.unwrap(), None);
    assert_eq!(empty.max::<f64>("amount").await.unwrap(), None);

    conn.execute_raw(Statement::from_string(
        backend,
        "DROP TABLE suprnova_aggregate_probe".to_string(),
    ))
    .await
    .expect("drop aggregate fixture");
}
