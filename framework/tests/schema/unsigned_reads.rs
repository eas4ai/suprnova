//! A read by a `u64` above `i64::MAX` on every database (PAR-044
//! follow-ups).
//!
//! Postgres and SQLite store a `u64` in a signed `BIGINT`, so no row there
//! holds a value above `i64::MAX`. A read by such a value answers what is
//! true of every row rather than failing, and the answer is the one MySQL
//! gives for the same rows. A write of one is still refused, and the
//! refusal reaches the log, never the client.

use chrono::{DateTime, Utc};
use sea_orm::DatabaseConnection;
use serial_test::serial;
use suprnova::database::EntityExt;
use suprnova::eloquent::Builder;
use suprnova::http::text;
use suprnova::testing::TestContainer;
use suprnova::{
    AppConfig, Config, DbConnection, Environment, Model, Response, RouteParam, Router, attrs,
    handler, model,
};

use super::cases::drop_tables;
use super::mysql::connect_mysql;
use super::postgres::connect_postgres;
use super::sqlite::connect_sqlite;
use super::unsigned_keys::{
    TABLES, UkOrder, create_tables, get, message, serve, show_order, show_raw_order, uk_order,
};

/// A customer whose orders the relation filters read through.
#[model(table = "uk_customers", fillable = ["name"], relations = {
    orders: HasMany<UkOrder>,
})]
pub struct UkCustomer {
    pub id: u64,
    pub name: String,
}

/// A soft-deleting `u64`-keyed model: its `find` goes through the query
/// builder rather than SeaORM's `find_by_id`.
#[model(table = "uk_notes", soft_deletes, fillable = ["body"])]
pub struct UkNote {
    pub id: u64,
    pub body: String,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// The two edges of the values no signed column holds.
pub(super) const BEYOND: [u64; 2] = [i64::MAX as u64 + 1, u64::MAX];

#[handler]
pub async fn show_note(note: RouteParam<UkNote>) -> Response {
    text(note.body.clone())
}

#[handler]
pub async fn update_beyond() -> Response {
    UkOrder::query()
        .update_all(attrs! { quantity: u64::MAX })
        .await?;
    text("updated")
}

#[handler]
pub async fn create_beyond() -> Response {
    UkOrder::create(attrs! { label: "big", quantity: u64::MAX }).await?;
    text("created")
}

fn router() -> Router {
    Router::new()
        .get("/orders/{order}", show_order)
        .get("/raw-orders/{order}", show_raw_order)
        .get("/notes/{note}", show_note)
        .get("/update-beyond", update_beyond)
        .get("/create-beyond", create_beyond)
        .into()
}

/// Fails when `body` names the database engine, a table or a column, which
/// would tell a client how the data is stored.
fn assert_names_no_storage(body: &str, context: &str) {
    for word in [
        "Postgres",
        "postgres",
        "Sqlite",
        "SQLite",
        "MySql",
        "BIGINT",
        "unsigned",
        "uk_orders",
        "uk_notes",
        "quantity",
        "`id`",
    ] {
        assert!(
            !body.contains(word),
            "{context}: the body names {word}: {body}"
        );
    }
}

/// The labels of the orders `query` matches, sorted.
async fn labels(query: Builder<UkOrder>) -> Vec<String> {
    let mut labels: Vec<String> = query
        .get()
        .await
        .expect("the query runs")
        .iter()
        .map(|order| order.label.clone())
        .collect();
    labels.sort();
    labels
}

/// Three orders: `a` with quantity 1, `b` with 5 and `c` with none.
pub(super) async fn seed_orders() -> UkCustomer {
    let shop = UkCustomer::create(attrs! { name: "shop" })
        .await
        .expect("create a customer");
    for (label, quantity) in [("a", Some(1u64)), ("b", Some(5)), ("c", None)] {
        UkOrder::create(attrs! { uk_customer_id: shop.id, label: label, quantity: quantity })
            .await
            .expect("create an order");
    }
    shop
}

/// A read by a value above `i64::MAX` answers what is true of every row:
/// on Postgres and SQLite no row holds such a value, and on MySQL none of
/// these rows does, so every database gives the same answer. `find` and the
/// bare entity's `find_by_pk` return nothing, `find_many` skips it, `=`,
/// `>`, `>=` and `IN` match no row, `!=`, `<>`, `<`, `<=` and `NOT IN`
/// match every row whose column is not NULL, and `NOT` of one keeps SQL's
/// meaning for a NULL row. A route that binds the key answers 404, and the
/// body names neither the engine nor a table or column. It fails while
/// Postgres and SQLite refuse the value with a 422 that names the engine,
/// the table and the column.
pub async fn reads_beyond_a_signed_column_answer_truthfully(conn: &DatabaseConnection) {
    create_tables(conn).await;
    let _guard = TestContainer::fake();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));
    let shop = seed_orders().await;
    UkNote::create(attrs! { body: "kept" })
        .await
        .expect("create a note");
    let a = UkOrder::query()
        .filter("label", "a")
        .first()
        .await
        .expect("read a")
        .expect("a");
    let addr = serve(router()).await;
    let none: Vec<&str> = Vec::new();

    for beyond in BEYOND {
        assert!(
            UkOrder::find(beyond).await.expect("find").is_none(),
            "find {beyond}"
        );
        assert!(
            UkNote::find(beyond)
                .await
                .expect("a soft-deleting find")
                .is_none(),
            "a soft-deleting find {beyond}"
        );
        assert!(
            uk_order::Entity::find_by_pk(beyond)
                .await
                .expect("find_by_pk")
                .is_none(),
            "find_by_pk {beyond}"
        );
        assert_eq!(
            UkOrder::find_many([beyond, a.id])
                .await
                .expect("find_many")
                .iter()
                .map(|order| order.label.as_str())
                .collect::<Vec<_>>(),
            vec!["a"],
            "find_many {beyond}"
        );

        let cases: Vec<(&str, Builder<UkOrder>, Vec<&str>)> = vec![
            (
                "filter",
                UkOrder::query().filter("quantity", beyond),
                none.clone(),
            ),
            (
                "=",
                UkOrder::query().filter_op("quantity", "=", beyond),
                none.clone(),
            ),
            (
                "!=",
                UkOrder::query().filter_op("quantity", "!=", beyond),
                vec!["a", "b"],
            ),
            (
                "<>",
                UkOrder::query().filter_op("quantity", "<>", beyond),
                vec!["a", "b"],
            ),
            (
                "<",
                UkOrder::query().filter_op("quantity", "<", beyond),
                vec!["a", "b"],
            ),
            (
                "<=",
                UkOrder::query().filter_op("quantity", "<=", beyond),
                vec!["a", "b"],
            ),
            (
                ">",
                UkOrder::query().filter_op("quantity", ">", beyond),
                none.clone(),
            ),
            (
                ">=",
                UkOrder::query().filter_op("quantity", ">=", beyond),
                none.clone(),
            ),
            (
                "IN",
                UkOrder::query().where_in("quantity", vec![beyond]),
                none.clone(),
            ),
            (
                "IN with a held value",
                UkOrder::query().where_in("quantity", vec![beyond, 1]),
                vec!["a"],
            ),
            (
                "NOT IN",
                UkOrder::query().where_not_in("quantity", vec![beyond]),
                vec!["a", "b"],
            ),
            (
                "NOT IN with a held value",
                UkOrder::query().where_not_in("quantity", vec![beyond, 1]),
                vec!["b"],
            ),
            (
                "BETWEEN up to it",
                UkOrder::query().where_between("quantity", 2..=beyond),
                vec!["b"],
            ),
            (
                "BETWEEN from it",
                UkOrder::query().where_between("quantity", beyond..=beyond),
                none.clone(),
            ),
            (
                "NOT BETWEEN up to it",
                UkOrder::query().where_not_between("quantity", 2..=beyond),
                vec!["a"],
            ),
            (
                "NOT BETWEEN from it",
                UkOrder::query().where_not_between("quantity", beyond..=beyond),
                vec!["a", "b"],
            ),
            (
                "NOT =",
                UkOrder::query().filter_not("quantity", beyond),
                vec!["a", "b"],
            ),
            (
                "NOT !=",
                UkOrder::query().where_none(["quantity"], "!=", beyond),
                none.clone(),
            ),
            (
                "OR",
                UkOrder::query()
                    .filter("label", "c")
                    .or_where("quantity", beyond),
                vec!["c"],
            ),
            (
                "OR IN",
                UkOrder::query()
                    .filter("label", "c")
                    .or_where_in("quantity", vec![beyond]),
                vec!["c"],
            ),
            (
                "on the key",
                UkOrder::query().filter("id", beyond),
                none.clone(),
            ),
            (
                "where_key",
                UkOrder::query().where_key(beyond),
                none.clone(),
            ),
            (
                "below it on the key",
                UkOrder::query().filter_op("id", "<", beyond),
                vec!["a", "b", "c"],
            ),
        ];
        for (name, query, expected) in cases {
            assert_eq!(labels(query).await, expected, "{name} {beyond}");
        }

        assert_eq!(
            UkOrder::query()
                .in_order_of("quantity", [beyond, 5, 1])
                .get()
                .await
                .expect("in_order_of")
                .iter()
                .map(|order| order.label.as_str())
                .collect::<Vec<_>>(),
            vec!["b", "a", "c"],
            "in_order_of {beyond}"
        );
        assert!(
            UkCustomer::query()
                .where_has("orders", |q: Builder<UkOrder>| q.filter("quantity", beyond))
                .get()
                .await
                .expect("where_has")
                .is_empty(),
            "where_has = {beyond}"
        );
        assert_eq!(
            UkCustomer::query()
                .where_has("orders", |q: Builder<UkOrder>| {
                    q.filter_op("quantity", "<", beyond)
                })
                .get()
                .await
                .expect("where_has")
                .iter()
                .map(|customer| customer.id)
                .collect::<Vec<_>>(),
            vec![shop.id],
            "where_has < {beyond}"
        );
        assert!(
            UkCustomer::query()
                .where_relation("orders", "quantity", beyond)
                .get()
                .await
                .expect("where_relation")
                .is_empty(),
            "where_relation {beyond}"
        );

        for path in [
            format!("/orders/{beyond}"),
            format!("/raw-orders/{beyond}"),
            format!("/notes/{beyond}"),
        ] {
            let (status, body) = get(addr, &path).await;
            assert_eq!(status, 404, "{path}: {body}");
            assert_names_no_storage(&body, &path);
        }
    }

    drop_tables(conn, TABLES).await;
}

#[tokio::test]
async fn sqlite_reads_beyond_a_signed_column_answer_truthfully() {
    reads_beyond_a_signed_column_answer_truthfully(&connect_sqlite().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_reads_beyond_a_signed_column_answer_truthfully() {
    reads_beyond_a_signed_column_answer_truthfully(&connect_postgres().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn mysql_reads_beyond_a_signed_column_answer_truthfully() {
    reads_beyond_a_signed_column_answer_truthfully(&connect_mysql().await).await;
}

/// On Postgres and SQLite a write of a value above `i64::MAX` is still
/// refused before anything is sent, and with debug off the client gets the
/// generic 500 body: the refusal, which names the engine, the table and the
/// column, goes to the log only. It fails while a mass update's refusal
/// answers 422 with that text.
pub async fn writes_beyond_a_signed_column_name_nothing_to_the_client(conn: &DatabaseConnection) {
    create_tables(conn).await;
    let _guard = TestContainer::fake();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));
    Config::register(
        AppConfig::builder()
            .name("unsigned-keys-test")
            .environment(Environment::Production)
            .debug(false)
            .url("http://localhost:0")
            .build(),
    );
    seed_orders().await;
    let addr = serve(router()).await;

    for path in ["/update-beyond", "/create-beyond"] {
        let (status, body) = get(addr, path).await;
        assert_eq!(
            (status, message(&body).as_str()),
            (500, "Internal Server Error"),
            "{path}: {body}"
        );
        assert_names_no_storage(&body, path);
    }
    assert_eq!(
        labels(UkOrder::query()).await,
        vec!["a", "b", "c"],
        "nothing was written"
    );

    drop_tables(conn, TABLES).await;
}

#[tokio::test]
#[serial]
async fn sqlite_writes_beyond_a_signed_column_name_nothing_to_the_client() {
    writes_beyond_a_signed_column_name_nothing_to_the_client(&connect_sqlite().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_writes_beyond_a_signed_column_name_nothing_to_the_client() {
    writes_beyond_a_signed_column_name_nothing_to_the_client(&connect_postgres().await).await;
}
