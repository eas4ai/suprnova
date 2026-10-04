//! `sum` and `avg` over integer columns on every database.
//!
//! The database chooses the type of an aggregate: `SUM` of an integer
//! column is `numeric` on Postgres and `DECIMAL` on MySQL, `AVG` is
//! `numeric` / `DECIMAL`, and SQLite answers an integer or a real. The
//! terminals read whichever arrives, exactly: a sum that does not fit the
//! type asked for, or has a fraction, is an error, never truncated.

use sea_orm::DatabaseConnection;
use sea_orm_migration::prelude::*;
use serial_test::serial;
use suprnova::schema::Schema;
use suprnova::testing::TestContainer;
use suprnova::{DbConnection, Model, attrs, model};

use super::cases::drop_tables;
use super::mysql::connect_mysql;
use super::postgres::connect_postgres;
use super::sqlite::connect_sqlite;

/// The owner of ledger rows, for the relation aggregates.
#[model(table = "ia_owners", fillable = ["name"], relations = {
    entries: HasMany<IaEntry>,
})]
pub struct IaOwner {
    pub id: i64,
    pub name: String,
}

/// A ledger row with a signed, an unsigned and a 32-bit integer column, a
/// double and a single-precision real.
#[model(
    table = "ia_entries",
    fillable = ["ia_owner_id", "amount", "hits", "small", "ratio", "weight", "tag"],
)]
pub struct IaEntry {
    pub id: i64,
    pub ia_owner_id: Option<i64>,
    pub amount: i64,
    pub hits: u64,
    pub small: i32,
    pub ratio: f64,
    pub weight: f32,
    pub tag: String,
}

const TABLES: &[&str] = &["ia_entries", "ia_owners"];

async fn create_entries(conn: &DatabaseConnection) {
    let manager = SchemaManager::new(conn);
    drop_tables(conn, TABLES).await;
    Schema::create(&manager, "ia_owners", |t| {
        t.id();
        t.string("name");
    })
    .await
    .expect("create ia_owners");
    Schema::create(&manager, "ia_entries", |t| {
        t.id();
        t.big_integer("ia_owner_id").nullable();
        t.big_integer("amount");
        t.unsigned_big_integer("hits");
        t.integer("small");
        t.double("ratio");
        t.float("weight");
        t.string("tag");
    })
    .await
    .expect("create ia_entries");
}

/// Adds an entry whose single-precision `weight` equals its `ratio`.
async fn add(amount: i64, hits: u64, small: i32, ratio: f64, tag: &str) {
    let weight = ratio as f32;
    IaEntry::create(attrs! {
        amount: amount, hits: hits, small: small, ratio: ratio, weight: weight, tag: tag
    })
    .await
    .expect("create an entry");
}

/// `sum::<i64>`, `sum::<u64>`, `sum::<i32>` and `sum::<f64>` over integer
/// and real columns, single-precision included, and `avg`, give the same
/// answer on SQLite, Postgres
/// and MySQL, zero for no rows. A sum outside the type asked for, a
/// negative sum asked for as a `u64`, and a fractional sum asked for as an
/// integer are errors. It fails while Postgres's `numeric` and MySQL's
/// `DECIMAL` do not decode as the integer or float asked for.
pub async fn integer_aggregates_read_on_every_database(conn: &DatabaseConnection) {
    create_entries(conn).await;
    let _guard = TestContainer::fake();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));
    add(10, 3, 1, 0.5, "kept").await;
    add(25, 4, 2, 0.25, "kept").await;
    add(30, 5, 6, 1.0, "other").await;
    let kept = || IaEntry::query().filter("tag", "kept");
    let none = || IaEntry::query().filter("tag", "missing");

    assert_eq!(IaEntry::sum::<i64>("amount").await.expect("sum::<i64>"), 65);
    assert_eq!(IaEntry::sum::<u64>("hits").await.expect("sum::<u64>"), 12);
    assert_eq!(IaEntry::sum::<i32>("small").await.expect("sum::<i32>"), 9);
    assert_eq!(
        IaEntry::sum::<i64>("small")
            .await
            .expect("sum::<i64> of an int"),
        9
    );
    assert_eq!(
        IaEntry::sum::<f64>("amount").await.expect("sum::<f64>"),
        65.0
    );
    assert_eq!(
        IaEntry::sum::<f64>("ratio").await.expect("sum of a real"),
        1.75
    );
    assert_eq!(
        IaEntry::sum::<f64>("weight")
            .await
            .expect("sum of a single-precision real"),
        1.75
    );
    assert_eq!(
        kept().sum::<i64>("amount").await.expect("a filtered sum"),
        35
    );

    assert_eq!(IaEntry::avg("hits").await.expect("avg of a u64"), 4.0);
    assert_eq!(IaEntry::avg("small").await.expect("avg of an int"), 3.0);
    assert_eq!(kept().avg("amount").await.expect("avg of an i64"), 17.5);
    assert_eq!(kept().avg("ratio").await.expect("avg of a real"), 0.375);

    assert_eq!(none().sum::<i64>("amount").await.expect("an empty sum"), 0);
    assert_eq!(
        none().sum::<u64>("hits").await.expect("an empty u64 sum"),
        0
    );
    assert_eq!(none().avg("amount").await.expect("an empty avg"), 0.0);

    let fraction = IaEntry::sum::<i64>("ratio")
        .await
        .expect_err("1.75 is not an i64");
    assert!(
        fraction.to_string().contains("1.75"),
        "the error quotes the sum: {fraction}"
    );

    add(i64::from(i32::MAX), 1, 0, 0.0, "wide").await;
    let wide = IaEntry::sum::<i32>("amount")
        .await
        .expect_err("a sum above i32::MAX is not an i32");
    assert!(
        wide.to_string().contains("2147483712"),
        "the error quotes the sum: {wide}"
    );
    assert_eq!(
        IaEntry::sum::<i64>("amount")
            .await
            .expect("the same sum as an i64"),
        65 + i64::from(i32::MAX)
    );

    add(-100, 1, 0, 0.0, "negative").await;
    let negative = IaEntry::query()
        .filter("tag", "negative")
        .sum::<u64>("amount")
        .await
        .expect_err("-100 is not a u64");
    assert!(
        negative.to_string().contains("-100"),
        "the error quotes the sum: {negative}"
    );

    add(i64::MAX, 1, 0, 0.0, "huge").await;
    add(i64::MAX, 1, 0, 0.0, "huge").await;
    assert!(
        IaEntry::query()
            .filter("tag", "huge")
            .sum::<i64>("amount")
            .await
            .is_err(),
        "a sum above i64::MAX is not an i64"
    );

    drop_tables(conn, TABLES).await;
}

/// `with_sum` and `with_avg` over an integer column read the same values on
/// SQLite, Postgres and MySQL. It fails while the relation aggregate reads
/// Postgres's `numeric` and MySQL's `DECIMAL` as nothing and stores `0.0`.
pub async fn relation_aggregates_read_on_every_database(conn: &DatabaseConnection) {
    create_entries(conn).await;
    let _guard = TestContainer::fake();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));
    let owner = IaOwner::create(attrs! { name: "owner" })
        .await
        .expect("create an owner");
    for amount in [10i64, 25] {
        IaEntry::create(attrs! {
            ia_owner_id: owner.id, amount: amount, hits: 1, small: 1, ratio: 0.5, weight: 0.5f32,
            tag: "kept"
        })
        .await
        .expect("create an entry");
    }

    let owners = IaOwner::query()
        .with_sum(("entries", "amount"))
        .with_avg(("entries", "amount"))
        .get()
        .await
        .expect("relation aggregates");
    let owner = owners.first().expect("the owner");
    assert_eq!(owner.entries_sum_of("amount"), Some(35.0), "with_sum");
    assert_eq!(owner.entries_avg_of("amount"), Some(17.5), "with_avg");

    drop_tables(conn, TABLES).await;
}

#[tokio::test]
async fn sqlite_relation_aggregates_read_on_every_database() {
    relation_aggregates_read_on_every_database(&connect_sqlite().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_relation_aggregates_read_on_every_database() {
    relation_aggregates_read_on_every_database(&connect_postgres().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn mysql_relation_aggregates_read_on_every_database() {
    relation_aggregates_read_on_every_database(&connect_mysql().await).await;
}

#[tokio::test]
async fn sqlite_integer_aggregates_read_on_every_database() {
    integer_aggregates_read_on_every_database(&connect_sqlite().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_integer_aggregates_read_on_every_database() {
    integer_aggregates_read_on_every_database(&connect_postgres().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn mysql_integer_aggregates_read_on_every_database() {
    integer_aggregates_read_on_every_database(&connect_mysql().await).await;
}
