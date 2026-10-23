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

#[model(table = "suprnova_relagg_parents", timestamps = false, relations = {
    lines: HasMany<RelaggLine> { fk = "parent_id" },
})]
pub struct RelaggParent {
    pub id: i64,
    pub name: String,
}

#[model(table = "suprnova_relagg_lines", timestamps = false)]
pub struct RelaggLine {
    pub id: i64,
    pub parent_id: i64,
    pub qty: i32,
    pub amount: i64,
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

/// `with_min` / `with_max` / `with_sum` / `with_avg` read the totals
/// Postgres sends for integer columns: `integer` (int4) for the minimum
/// and maximum of an `INTEGER`, `numeric` for the sum of a `BIGINT` and
/// the average of any integer. No valid total reads as zero or as none.
#[tokio::test]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_relation_aggregates_read_integer_totals() {
    let conn = connect_postgres().await;
    let backend = conn.get_database_backend();
    for sql in [
        "DROP TABLE IF EXISTS suprnova_relagg_lines",
        "DROP TABLE IF EXISTS suprnova_relagg_parents",
        "CREATE TABLE suprnova_relagg_parents (id BIGINT PRIMARY KEY, name TEXT NOT NULL)",
        "CREATE TABLE suprnova_relagg_lines (\
             id BIGINT PRIMARY KEY,\
             parent_id BIGINT NOT NULL,\
             qty INTEGER NOT NULL,\
             amount BIGINT NOT NULL\
         )",
        "INSERT INTO suprnova_relagg_parents (id, name) VALUES (1, 'with lines'), (2, 'empty')",
        "INSERT INTO suprnova_relagg_lines (id, parent_id, qty, amount) VALUES \
             (1, 1, 3, 10), (2, 1, 5, 20)",
    ] {
        conn.execute_raw(Statement::from_string(backend, sql.to_string()))
            .await
            .expect("create relation aggregate fixture");
    }

    let _guard = TestContainer::fake();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));

    let parents = RelaggParent::query()
        .with_min(("lines", "qty"))
        .with_max(("lines", "qty"))
        .with_sum(("lines", "amount"))
        .with_avg(("lines", "qty"))
        .order_by("id", suprnova::Direction::Asc)
        .get()
        .await
        .expect("the relation aggregates read");
    let (full, empty) = (&parents[0], &parents[1]);
    assert_eq!(full.lines_min_of("qty"), Some(Some(3.0)));
    assert_eq!(full.lines_max_of("qty"), Some(Some(5.0)));
    assert_eq!(full.lines_sum_of("amount"), Some(30.0));
    assert_eq!(full.lines_avg_of("qty"), Some(4.0));
    assert_eq!(empty.lines_min_of("qty"), Some(None));
    assert_eq!(empty.lines_sum_of("amount"), Some(0.0));

    for sql in [
        "DROP TABLE suprnova_relagg_lines",
        "DROP TABLE suprnova_relagg_parents",
    ] {
        conn.execute_raw(Statement::from_string(backend, sql.to_string()))
            .await
            .expect("drop relation aggregate fixture");
    }
}
