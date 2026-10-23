//! Keyset walks over `u64` keys (PAR-044 follow-ups).
//!
//! MySQL's unsigned keys reach `u64::MAX`, so `chunk_by_id`, `lazy_by_id`
//! and `cursor_paginate` walk the whole range there. On Postgres and
//! SQLite no key passes `i64::MAX`, and the walks are unchanged.

use std::sync::{Arc, Mutex};

use sea_orm::{DatabaseConnection, DbBackend};
use serial_test::serial;
use suprnova::context::Context;
use suprnova::testing::TestContainer;
use suprnova::{DbConnection, EncryptionKey, Model};

use super::cases::{drop_tables, run};
use super::mysql::connect_mysql;
use super::postgres::connect_postgres;
use super::sqlite::connect_sqlite;
use super::unsigned_keys::{TABLES, UkOrder, create_tables};

/// `cursor_paginate` encrypts its cursors, so the process needs a key.
/// Any key serves: whichever test of this binary installs one first, the
/// cursors are sealed and opened under it.
fn install_a_key() {
    let _first =
        suprnova::testing::install_test_encryption_keyring(EncryptionKey::generate(), Vec::new());
}

/// `chunk_by_id`, `lazy_by_id` and `cursor_paginate` walk every key in
/// order, in batches of two. On MySQL the keys sit on both sides of
/// `i64::MAX`, up to `u64::MAX`; on Postgres and SQLite, where no key can
/// pass `i64::MAX`, the walk is unchanged. It fails while a key above
/// `i64::MAX` ends the walk with an error, or while a cursor above it
/// binds as a string, which MySQL compares as a rounded double.
pub async fn keyset_walks_cover_the_whole_u64_range(conn: &DatabaseConnection) {
    create_tables(conn).await;
    let _guard = TestContainer::fake();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));
    let top = i64::MAX as u64;
    let keys: Vec<u64> = if conn.get_database_backend() == DbBackend::MySql {
        vec![1, 2, top - 1, top, top + 1, u64::MAX - 1, u64::MAX]
    } else {
        vec![1, 2, 3, 4, 5]
    };
    for key in &keys {
        run(
            conn,
            &format!("INSERT INTO uk_orders (id, label) VALUES ({key}, 'k{key}')"),
        )
        .await
        .expect("insert a key");
    }

    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    UkOrder::query()
        .chunk_by_id(2, move |batch| {
            let sink = sink.clone();
            async move {
                sink.lock()
                    .expect("the walked keys")
                    .extend(batch.iter().map(|order| order.id));
                Ok(())
            }
        })
        .await
        .expect("chunk_by_id");
    assert_eq!(*seen.lock().expect("the walked keys"), keys, "chunk_by_id");

    let mut lazy = UkOrder::query().lazy_by_id(2);
    let mut walked = Vec::new();
    while let Some(order) = lazy.next().await {
        walked.push(order.expect("lazy_by_id").id);
    }
    assert_eq!(walked, keys, "lazy_by_id");

    install_a_key();
    Context::test_clear_query();
    let mut paged = Vec::new();
    loop {
        let page = UkOrder::query()
            .cursor_paginate(2)
            .await
            .expect("cursor_paginate");
        paged.extend(page.data.iter().map(|order| order.id));
        match page.next_cursor.clone() {
            Some(cursor) if paged.len() <= keys.len() => {
                Context::test_set_query("cursor", &cursor);
            }
            _ => break,
        }
    }
    Context::test_clear_query();
    assert_eq!(paged, keys, "cursor_paginate");

    drop_tables(conn, TABLES).await;
}

#[tokio::test]
async fn sqlite_keyset_walks_cover_the_whole_u64_range() {
    keyset_walks_cover_the_whole_u64_range(&connect_sqlite().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_keyset_walks_cover_the_whole_u64_range() {
    keyset_walks_cover_the_whole_u64_range(&connect_postgres().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn mysql_keyset_walks_cover_the_whole_u64_range() {
    keyset_walks_cover_the_whole_u64_range(&connect_mysql().await).await;
}
