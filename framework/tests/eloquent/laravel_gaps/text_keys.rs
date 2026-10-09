//! The seeded SQLite order over a model whose primary key is text.
//!
//! SQLite's `%` reads a text key as the integer its leading digits spell,
//! or 0, so an order computed from the key gives most text-keyed rows one
//! value and falls back to key order for every seed. The model builder
//! shuffles each row's rowid instead, and a union carries that rowid
//! through its projection so the outer query can read it.

use crate::query_fixture::Fixture;
use crate::query_helpers_model::QhItem;
use sea_orm::DbBackend;
use suprnova::context::Context;
use suprnova::{Builder, Model, model};

/// A model whose keys start with letters, as a UUID or a ULID can.
#[model(
    table = "gap_text_keys",
    key_type = "String",
    auto_increment = false,
    fillable = ["id", "label"],
    timestamps = false
)]
pub struct GapTextKey {
    /// A text key that SQLite's `%` reads as 0.
    pub id: String,
    /// A second column, so a row is more than its key.
    pub label: String,
}

const KEYS: [&str; 8] = ["k_a", "k_b", "k_c", "k_d", "k_e", "k_f", "k_g", "k_h"];

fn all_keys() -> Vec<String> {
    KEYS.iter().map(|key| (*key).to_owned()).collect()
}

/// Eight text-keyed rows, inserted in key order, so key order and rowid
/// order agree and only a real shuffle moves them.
async fn text_key_fixture(table_options: &str) -> Fixture {
    let fx = Fixture::sqlite().await;
    fx.exec(&format!(
        "CREATE TABLE gap_text_keys (id TEXT PRIMARY KEY, label TEXT NOT NULL){table_options}"
    ))
    .await;
    let rows: Vec<String> = KEYS
        .iter()
        .map(|key| format!("('{key}', 'label {key}')"))
        .collect();
    fx.exec(&format!(
        "INSERT INTO gap_text_keys (id, label) VALUES {}",
        rows.join(", ")
    ))
    .await;
    fx
}

async fn text_ids(query: Builder<GapTextKey>) -> Vec<String> {
    query
        .get()
        .await
        .unwrap()
        .iter()
        .map(|row| row.id.clone())
        .collect()
}

fn sorted(mut ids: Vec<String>) -> Vec<String> {
    ids.sort();
    ids
}

/// Two queries whose rows overlap on `k_d` and `k_e`, so `UNION` has
/// duplicates to remove.
fn overlapping_union() -> Builder<GapTextKey> {
    GapTextKey::query()
        .filter_op("id", "<=", "k_e")
        .union(GapTextKey::query().filter_op("id", ">=", "k_d"))
}

#[tokio::test]
async fn seeded_order_shuffles_a_text_keyed_model_on_sqlite() {
    let _fx = text_key_fixture("").await;
    let one = text_ids(GapTextKey::query().in_random_order_seeded(1)).await;
    let two = text_ids(GapTextKey::query().in_random_order_seeded(2)).await;
    assert_ne!(one, two, "seeds 1 and 2 give one order");
    assert_ne!(one, all_keys(), "seed 1 keeps key order");
    assert_ne!(two, all_keys(), "seed 2 keeps key order");
    assert_eq!(
        one,
        text_ids(GapTextKey::query().in_random_order_seeded(1)).await
    );
    assert_eq!(
        two,
        text_ids(GapTextKey::query().in_random_order_seeded(2)).await
    );
    assert_eq!(sorted(one), all_keys());
    assert_eq!(sorted(two), all_keys());
}

#[tokio::test]
async fn seeded_order_shuffles_a_union_of_text_keyed_queries_on_sqlite() {
    let _fx = text_key_fixture("").await;
    let mut orders = Vec::new();
    for seed in [1, 2] {
        let first = text_ids(overlapping_union().in_random_order_seeded(seed)).await;
        assert_eq!(
            first,
            text_ids(overlapping_union().in_random_order_seeded(seed)).await,
            "seed {seed} repeats"
        );
        assert_eq!(sorted(first.clone()), all_keys(), "seed {seed}");
        assert_eq!(
            first,
            text_ids(GapTextKey::query().in_random_order_seeded(seed)).await,
            "a union shuffles the same rowids as a plain query"
        );
        orders.push(first);
    }
    assert_ne!(orders[0], orders[1], "seeds 1 and 2 give one union order");
    // An operand that is itself a union with its own seeded order: every
    // operand of the outer union projects the rowid, so their column
    // counts agree.
    let nested = |outer: Option<u64>| {
        let inner = GapTextKey::query()
            .filter_op("id", ">=", "k_c")
            .filter_op("id", "<", "k_f")
            .union(GapTextKey::query().filter_op("id", ">=", "k_f"))
            .in_random_order_seeded(2);
        let query = GapTextKey::query().filter_op("id", "<", "k_c").union(inner);
        match outer {
            Some(seed) => query.in_random_order_seeded(seed),
            None => query,
        }
    };
    assert_eq!(sorted(text_ids(nested(None)).await), all_keys());
    let outer = text_ids(nested(Some(1))).await;
    assert_eq!(outer, orders[0], "the outer seed orders the whole union");
    assert!(
        GapTextKey::query()
            .filter("id", "k_z")
            .union(GapTextKey::query().filter("id", "k_y"))
            .in_random_order_seeded(1)
            .get()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn paginating_a_seeded_text_keyed_union_returns_stable_pages_on_sqlite() {
    let _fx = text_key_fixture("").await;
    Context::test_clear_query();
    let plain = text_ids(GapTextKey::query().in_random_order_seeded(2)).await;
    let mut walked = Vec::new();
    for page in 1..=3_usize {
        Context::test_set_query("text_page", page.to_string());
        let ids = |p: suprnova::LengthAwarePaginator<GapTextKey>| -> Vec<String> {
            p.data.iter().map(|row| row.id.clone()).collect()
        };
        let first = overlapping_union()
            .in_random_order_seeded(2)
            .paginate_using("text_page", 3)
            .await
            .unwrap();
        assert_eq!(first.total, 8, "page {page}");
        let first = ids(first);
        let again = overlapping_union()
            .in_random_order_seeded(2)
            .paginate_using("text_page", 3)
            .await
            .unwrap();
        assert_eq!(first, ids(again), "page {page} repeats");
        let start = (page - 1) * 3;
        assert_eq!(first, plain[start..(start + 3).min(8)], "page {page}");
        walked.extend(first);
    }
    Context::test_clear_query();
    assert_eq!(walked, plain);
    let simple = overlapping_union()
        .in_random_order_seeded(2)
        .simple_paginate(5)
        .await
        .unwrap();
    assert!(simple.has_more);
    assert_eq!(
        simple
            .data
            .iter()
            .map(|row| row.id.clone())
            .collect::<Vec<_>>(),
        plain[..5]
    );
}

#[tokio::test]
async fn joined_update_over_a_seeded_text_keyed_union_compares_one_key_on_sqlite() {
    // A write through a join on SQLite selects its rows by key with
    // `IN (...)`, which compares one column; the union's projected rowid
    // must not reach it.
    let fx = text_key_fixture("").await;
    fx.exec("CREATE TABLE gap_text_tags (key_id TEXT NOT NULL, tag TEXT NOT NULL)")
        .await;
    fx.exec("INSERT INTO gap_text_tags VALUES ('k_b', 'hot'), ('k_c', 'cold')")
        .await;
    let updated = GapTextKey::query()
        .join(
            "gap_text_tags",
            "gap_text_keys.id",
            "=",
            "gap_text_tags.key_id",
        )
        .filter("gap_text_tags.tag", "hot")
        .union(GapTextKey::query().filter("id", "k_h"))
        .in_random_order_seeded(1)
        .update([("label", "picked")])
        .await
        .unwrap();
    assert_eq!(updated, 2);
    assert_eq!(
        text_ids(
            GapTextKey::query()
                .filter("label", "picked")
                .order_by_asc("id")
        )
        .await,
        vec!["k_b", "k_h"]
    );
}

#[tokio::test]
async fn seeded_order_still_shuffles_an_integer_keyed_model_on_sqlite() {
    let fx = Fixture::sqlite().await;
    fx.exec("CREATE TABLE qh_items (id INTEGER PRIMARY KEY, a INTEGER, b INTEGER, c INTEGER, label TEXT, code TEXT, description TEXT, deleted_at TEXT)").await;
    let rows: Vec<String> = (1..=8)
        .map(|id| format!("({id}, {id}, {}, 1, 'x', 'code{id}', '', NULL)", id % 2))
        .collect();
    fx.exec(&format!("INSERT INTO qh_items VALUES {}", rows.join(", ")))
        .await;
    let ids = |query: Builder<QhItem>| async move {
        query
            .get()
            .await
            .unwrap()
            .iter()
            .map(|row| row.id)
            .collect::<Vec<_>>()
    };
    let one = ids(QhItem::query().in_random_order_seeded(1)).await;
    let two = ids(QhItem::query().in_random_order_seeded(2)).await;
    assert_ne!(one, two);
    assert_ne!(one, (1..=8).collect::<Vec<_>>());
    assert_eq!(one, ids(QhItem::query().in_random_order_seeded(1)).await);
    assert_eq!(two, ids(QhItem::query().in_random_order_seeded(2)).await);
    let union = || {
        QhItem::query()
            .filter("b", 0)
            .union(QhItem::query().filter("b", 1))
            .in_random_order_seeded(1)
    };
    assert_eq!(ids(union()).await, one);
    let mut all = one;
    all.sort_unstable();
    assert_eq!(all, (1..=8).collect::<Vec<_>>());
}

#[tokio::test]
async fn seeded_order_on_a_table_without_a_rowid_returns_the_engine_error_on_sqlite() {
    let _fx = text_key_fixture(" WITHOUT ROWID").await;
    let error = GapTextKey::query()
        .in_random_order_seeded(1)
        .get()
        .await
        .unwrap_err();
    assert!(error.to_string().contains("rowid"), "{error}");
    // An unseeded order needs no rowid.
    assert_eq!(
        sorted(text_ids(GapTextKey::query().in_random_order()).await),
        all_keys()
    );
}

#[tokio::test]
async fn seeded_order_sql_reads_the_rowid_on_sqlite_and_is_unchanged_elsewhere() {
    let plain = GapTextKey::query().in_random_order_seeded(1);
    let union = overlapping_union().in_random_order_seeded(1);
    assert_eq!(
        plain.to_sql_for(DbBackend::MySql),
        "SELECT * FROM gap_text_keys ORDER BY RAND(1)"
    );
    assert_eq!(
        union.to_sql_for(DbBackend::MySql),
        "SELECT * FROM (SELECT * FROM gap_text_keys WHERE id <= ? UNION SELECT * FROM gap_text_keys WHERE id >= ?) AS __suprnova_union ORDER BY RAND(1)"
    );
    let postgres_order = "ORDER BY CASE WHEN (SELECT setseed(5.42101086242752217e-20)) IS NULL THEN RANDOM() ELSE RANDOM() END";
    assert_eq!(
        plain.to_sql_for(DbBackend::Postgres),
        format!("SELECT * FROM gap_text_keys {postgres_order}")
    );
    assert_eq!(
        union.to_sql_for(DbBackend::Postgres),
        format!(
            "SELECT * FROM (SELECT * FROM gap_text_keys WHERE id <= $1 UNION SELECT * FROM gap_text_keys WHERE id >= $2) AS __suprnova_union {postgres_order}"
        )
    );
    let shuffle = |key: &str| {
        format!("(((({key} % 4294967296) + 1) % 4294967296) * 1640531527) % 4294967296, {key}")
    };
    assert_eq!(
        plain.to_sql_for(DbBackend::Sqlite),
        format!(
            "SELECT * FROM gap_text_keys ORDER BY {}",
            shuffle("gap_text_keys.rowid")
        )
    );
    assert_eq!(
        union.to_sql_for(DbBackend::Sqlite),
        format!(
            "SELECT * FROM (SELECT *, gap_text_keys.rowid AS __suprnova_row FROM gap_text_keys WHERE id <= ? UNION SELECT *, gap_text_keys.rowid AS __suprnova_row FROM gap_text_keys WHERE id >= ?) AS __suprnova_union ORDER BY {}",
            shuffle("__suprnova_row")
        )
    );
    // Without a seeded order over it, a union projects what it did.
    for unseeded in [
        overlapping_union(),
        overlapping_union().in_random_order(),
        GapTextKey::query()
            .filter_op("id", "<=", "k_e")
            .in_random_order_seeded(1)
            .take(2)
            .union(GapTextKey::query().filter_op("id", ">=", "k_d")),
    ] {
        let sql = unseeded.to_sql_for(DbBackend::Sqlite);
        assert!(!sql.contains("__suprnova_row"), "{sql}");
    }
}
