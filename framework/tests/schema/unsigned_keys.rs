//! Reads by a `u64` key on every database (PAR-044 follow-ups).
//!
//! A `u64` key is stored in the SeaORM entity as `StoredU64`, so whatever a
//! route parameter or a lookup needs from the key type, `StoredU64` has to
//! provide. This module holds the fixtures the other `unsigned_*` modules
//! share and the route-binding cases.
//!
//! The SQLite cases run with the ordinary suite; the Postgres and MySQL
//! ones are ignored until `PG_TEST_URL` / `MYSQL_TEST_URL` name a
//! disposable database, like the rest of this binary.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use sea_orm::{DatabaseConnection, DbBackend};
use sea_orm_migration::prelude::*;
use serial_test::serial;
use suprnova::database::EntityExt;
use suprnova::http::text;
use suprnova::schema::Schema;
use suprnova::testing::TestContainer;
use suprnova::{
    DbConnection, MiddlewareRegistry, Model, Response, RouteParam, Router, attrs, handle_request,
    handler, model,
};

use super::cases::{drop_tables, run};
use super::http_wire;
use super::mysql::connect_mysql;
use super::postgres::connect_postgres;
use super::sqlite::connect_sqlite;

/// An order keyed the way Laravel's `id()` keys a MySQL table, with a
/// nullable `u64` column for the comparisons, whose answer on a NULL row
/// is part of what they mean.
#[model(table = "uk_orders", fillable = ["uk_customer_id", "label", "quantity"])]
pub struct UkOrder {
    pub id: u64,
    pub uk_customer_id: Option<u64>,
    pub label: String,
    pub quantity: Option<u64>,
}

/// The bare SeaORM entity binds from a route too, once the application opts
/// it into `EntityExt`.
impl EntityExt for uk_order::Entity {}

/// An `i64`-keyed model, for what a route segment that is not a number
/// answers today. Its table is never read: the segment fails to parse first.
#[model(table = "uk_signed_orders", fillable = ["label"])]
pub struct UkSignedOrder {
    pub id: i64,
    pub label: String,
}

#[handler]
pub async fn show_order(order: RouteParam<UkOrder>) -> Response {
    text(format!("{} {}", order.id, order.label))
}

#[handler]
pub async fn show_raw_order(order: uk_order::Model) -> Response {
    text(format!("{} {}", order.id, order.label))
}

#[handler]
pub async fn show_signed_order(order: RouteParam<UkSignedOrder>) -> Response {
    text(order.label.clone())
}

pub(super) const TABLES: &[&str] = &["uk_orders", "uk_customers", "uk_notes"];

/// Creates the tables as Laravel's `id()` makes them: unsigned keys on
/// MySQL, signed `BIGINT`s on Postgres and SQLite.
pub(super) async fn create_tables(conn: &DatabaseConnection) {
    let manager = SchemaManager::new(conn);
    drop_tables(conn, TABLES).await;
    Schema::create(&manager, "uk_customers", |t| {
        t.unsigned_id();
        t.string("name");
    })
    .await
    .expect("create uk_customers");
    Schema::create(&manager, "uk_orders", |t| {
        t.unsigned_id();
        t.unsigned_big_integer("uk_customer_id").nullable();
        t.string("label");
        t.unsigned_big_integer("quantity").nullable();
    })
    .await
    .expect("create uk_orders");
    Schema::create(&manager, "uk_notes", |t| {
        t.unsigned_id();
        t.string("body");
        t.soft_deletes();
    })
    .await
    .expect("create uk_notes");
}

/// Serves `router` through `handle_request` on a loopback socket, so each
/// route parameter really comes from a matched path. Each connection runs
/// on this test's thread, where the test container holds the connection.
pub(super) async fn serve(router: Router) -> SocketAddr {
    let router = Arc::new(router);
    let registry = Arc::new(MiddlewareRegistry::new());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a loopback listener");
    let addr = listener.local_addr().expect("the listener's address");
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let router = router.clone();
            let registry = registry.clone();
            tokio::spawn(async move {
                let svc = service_fn(move |req: hyper::Request<Incoming>| {
                    let router = router.clone();
                    let registry = registry.clone();
                    async move { Ok::<_, Infallible>(handle_request(router, registry, req).await) }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), svc)
                    .await;
            });
        }
    });
    addr
}

pub(super) async fn get(addr: SocketAddr, path: &str) -> (u16, String) {
    let (status, _headers, body) = http_wire::request(addr, "GET", path, &[]).await;
    (status, body)
}

/// The `message` of a JSON error body.
pub(super) fn message(body: &str) -> String {
    let value: serde_json::Value = serde_json::from_str(body).expect("a JSON error body");
    value["message"]
        .as_str()
        .expect("the body's message")
        .to_owned()
}

/// A `u64`-keyed model binds from a route through `RouteParam<M>` and as its
/// bare SeaORM model, and finds its row. A segment that is not a number
/// answers 400, as it does for an `i64` key, naming the type the model
/// declares rather than the type that stores it. On MySQL a key at the top
/// of the `u64` range binds too. It fails to compile while the stored key
/// type cannot be parsed from a route segment or displayed.
pub async fn route_binding_finds_a_u64_key(conn: &DatabaseConnection) {
    create_tables(conn).await;
    let _guard = TestContainer::fake();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));
    let order = UkOrder::create(attrs! { label: "first", quantity: 3 })
        .await
        .expect("create an order");
    let router: Router = Router::new()
        .get("/orders/{order}", show_order)
        .get("/raw-orders/{order}", show_raw_order)
        .get("/signed-orders/{order}", show_signed_order)
        .into();
    let addr = serve(router).await;

    assert_eq!(
        get(addr, &format!("/orders/{}", order.id)).await,
        (200, format!("{} first", order.id))
    );
    assert_eq!(
        get(addr, &format!("/raw-orders/{}", order.id)).await,
        (200, format!("{} first", order.id))
    );
    let (status, body) = get(addr, "/orders/999999").await;
    assert_eq!(status, 404, "a missing row: {body}");

    let (signed_status, signed_body) = get(addr, "/signed-orders/abc").await;
    assert_eq!(
        (signed_status, message(&signed_body).as_str()),
        (400, "Invalid parameter 'abc': expected i64"),
        "what an i64 key answers"
    );
    for path in ["/orders/abc", "/raw-orders/abc"] {
        let (status, body) = get(addr, path).await;
        assert_eq!(
            (status, message(&body).as_str()),
            (400, "Invalid parameter 'abc': expected u64"),
            "{path}"
        );
    }

    if conn.get_database_backend() == DbBackend::MySql {
        run(
            conn,
            &format!(
                "INSERT INTO uk_orders (id, label) VALUES ({}, 'top')",
                u64::MAX
            ),
        )
        .await
        .expect("insert a key above i64::MAX");
        for prefix in ["/orders", "/raw-orders"] {
            assert_eq!(
                get(addr, &format!("{prefix}/{}", u64::MAX)).await,
                (200, format!("{} top", u64::MAX)),
                "{prefix}"
            );
        }
    }

    drop_tables(conn, TABLES).await;
}

#[tokio::test]
async fn sqlite_route_binding_finds_a_u64_key() {
    route_binding_finds_a_u64_key(&connect_sqlite().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_route_binding_finds_a_u64_key() {
    route_binding_finds_a_u64_key(&connect_postgres().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn mysql_route_binding_finds_a_u64_key() {
    route_binding_finds_a_u64_key(&connect_mysql().await).await;
}
