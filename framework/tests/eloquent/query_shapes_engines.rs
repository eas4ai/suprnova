//! Query shapes whose SQL differs by engine: unions with pages, counts and
//! orderings, random order, an offset with no limit, aggregates over a
//! query that already has a projection, JSON containment, and
//! `create_or_first` inside a transaction.
//!
//! The SQLite tests run by default; the `postgres_` and `mysql_` tests run
//! every scenario against a disposable server:
//!
//! ```text
//! PG_TEST_URL=postgres://... MYSQL_TEST_URL=mysql://... \
//!   cargo test -p suprnova --test eloquent query_shapes_engines:: -- --ignored --test-threads=1
//! ```

use sea_orm::DatabaseBackend;
use serde_json::json;
use suprnova::eloquent::FirstOrCreate;
use suprnova::{Builder, DB, FrameworkError, Model, attrs, model};

use crate::query_fixture::Fixture;

#[model(
    table = "qs_items",
    auto_increment = false,
    timestamps = false,
    fillable = ["id", "grp", "score", "email"]
)]
pub struct QsItem {
    pub id: i64,
    pub grp: String,
    pub score: i64,
    pub email: String,
}

/// JSON documents. The columns are read through `pluck` only, so their
/// native type never has to hydrate into a Rust field.
#[model(table = "qs_docs", timestamps = false)]
pub struct QsDoc {
    pub id: i64,
    pub meta: String,
    pub tags: String,
}

/// Six rows in three groups of two: `a` holds 1 and 2, `b` holds 3 and 4,
/// `c` holds 5 and 6. Scores run 10 to 60 with the id.
///
/// MySQL cannot name one `TEMPORARY` table twice in a statement, and a
/// union names its table once per arm, so there the table is an ordinary
/// one, dropped first in case a failed run left it and dropped again by
/// [`finish`].
async fn seed(fx: &Fixture) {
    let temporary = if fx.backend == DatabaseBackend::MySql {
        fx.exec("DROP TABLE IF EXISTS qs_items").await;
        ""
    } else {
        "TEMPORARY "
    };
    fx.exec(&format!(
        "CREATE {temporary}TABLE qs_items (id BIGINT PRIMARY KEY, grp VARCHAR(10) NOT NULL, \
         score BIGINT NOT NULL, email VARCHAR(50) NOT NULL UNIQUE)"
    ))
    .await;
    fx.exec(
        "INSERT INTO qs_items (id, grp, score, email) VALUES \
         (1, 'a', 10, 'e1'), (2, 'a', 20, 'e2'), (3, 'b', 30, 'e3'), \
         (4, 'b', 40, 'e4'), (5, 'c', 50, 'e5'), (6, 'c', 60, 'e6')",
    )
    .await;
}

/// Drop the ordinary table MySQL needed, then close the connection, which
/// drops the temporary ones.
async fn finish(fx: Fixture) {
    if fx.backend == DatabaseBackend::MySql {
        fx.exec("DROP TABLE IF EXISTS qs_items").await;
    }
    fx.close().await;
}

/// The JSON table, in each engine's own JSON types. On Postgres `meta` is
/// `jsonb` and `tags` is `json`, so containment is proven on both.
async fn seed_docs(fx: &Fixture) {
    let (meta, tags) = match fx.backend {
        DatabaseBackend::Postgres => ("JSONB", "JSON"),
        _ => ("JSON", "JSON"),
    };
    fx.exec(&format!(
        "CREATE TEMPORARY TABLE qs_docs (id BIGINT PRIMARY KEY, meta {meta} NOT NULL, \
         tags {tags} NOT NULL)"
    ))
    .await;
    fx.exec(
        "INSERT INTO qs_docs (id, meta, tags) VALUES \
         (1, '{\"active\": true, \"tier\": 1}', '[\"admin\", \"user\"]'), \
         (2, '{\"active\": false, \"tier\": 2}', '[\"user\"]'), \
         (3, '{\"active\": true}', '[]')",
    )
    .await;
}

fn items() -> Builder<QsItem> {
    QsItem::query()
}

// ---------- Scenarios ---------------------------------------------------------

/// DATA-052: inside a transaction, `create_or_first` that loses to an
/// existing row returns that row, and the transaction stays usable.
async fn create_or_first_inside_a_transaction() {
    let (found, rows) = DB::transaction(|_tx| {
        Box::pin(async move {
            let found = QsItem::create_or_first(
                attrs! { email: "e1" },
                attrs! { id: 99, grp: "z", score: 0 },
            )
            .await?;
            let rows = QsItem::query().count().await?;
            Ok::<_, FrameworkError>((found, rows))
        })
    })
    .await
    .unwrap_or_else(|e| panic!("create_or_first inside a transaction: {e}"));
    assert_eq!(found.id, 1, "the existing row is returned");
    assert_eq!(rows, 6, "the transaction still answers after the conflict");
    assert_eq!(items().count().await.expect("count"), 6, "no row was added");
}

/// DATA-053: JSON containment binds the candidate as a JSON document.
async fn json_containment() {
    let pluck = |q: Builder<QsDoc>| async move {
        q.order_by_asc("id")
            .pluck::<i64>("id")
            .await
            .unwrap_or_else(|e| panic!("JSON containment: {e}"))
    };
    assert_eq!(
        pluck(QsDoc::query().filter_json_contains("meta", json!({ "active": true }))).await,
        vec![1, 3]
    );
    assert_eq!(
        pluck(QsDoc::query().filter_json_contains("meta", json!({ "tier": 1 }))).await,
        vec![1]
    );
    assert_eq!(
        pluck(QsDoc::query().filter_json_contains("tags", json!("admin"))).await,
        vec![1],
        "a string is a JSON string, not bare text"
    );
    assert_eq!(
        pluck(QsDoc::query().filter_json_contains("tags", json!(["user"]))).await,
        vec![1, 2]
    );
}

// ---------- SQLite ------------------------------------------------------------

async fn seeded_sqlite() -> Fixture {
    let fx = Fixture::sqlite().await;
    seed(&fx).await;
    fx
}

#[tokio::test]
async fn sqlite_create_or_first_inside_a_transaction() {
    let _fx = seeded_sqlite().await;
    create_or_first_inside_a_transaction().await;
}

// ---------- Live engines ----------------------------------------------------

/// One connection to the server named by `env`, seeded with both tables.
async fn live(env: &str, backend: DatabaseBackend) -> Fixture {
    let fx = Fixture::live(env, backend).await;
    seed(&fx).await;
    seed_docs(&fx).await;
    fx
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL at PG_TEST_URL"]
async fn postgres_create_or_first_inside_a_transaction() {
    let fx = live("PG_TEST_URL", DatabaseBackend::Postgres).await;
    create_or_first_inside_a_transaction().await;
    finish(fx).await;
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL at PG_TEST_URL"]
async fn postgres_json_containment() {
    let fx = live("PG_TEST_URL", DatabaseBackend::Postgres).await;
    json_containment().await;
    finish(fx).await;
}

#[tokio::test]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_create_or_first_inside_a_transaction() {
    let fx = live("MYSQL_TEST_URL", DatabaseBackend::MySql).await;
    create_or_first_inside_a_transaction().await;
    finish(fx).await;
}

#[tokio::test]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_json_containment() {
    let fx = live("MYSQL_TEST_URL", DatabaseBackend::MySql).await;
    json_containment().await;
    finish(fx).await;
}
