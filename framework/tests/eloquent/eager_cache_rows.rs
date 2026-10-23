//! The eager-load cache keeps the rows a query loaded.
//!
//! - A relation's rows and its `with_count` count are kept apart, so
//!   `with` and `with_count` on one relation keep both, in either order,
//!   and a count alone does not stand in for loaded rows.
//! - A nested load that fails, or that its caller stops awaiting, puts
//!   back the rows it took out of each parent to work on.

use suprnova::testing::TestDatabase;
use suprnova::{Model, model};

// ---- Row cells and count cells ------------------------------------------

#[model(table = "rd_users", relations = {
    profile: HasOne<RdProfile>,
    posts: HasMany<RdPost>,
})]
pub struct RdUser {
    pub id: i64,
    pub name: String,
}

#[model(table = "rd_profiles")]
pub struct RdProfile {
    pub id: i64,
    pub rd_user_id: i64,
    pub bio: String,
}

#[model(table = "rd_posts")]
pub struct RdPost {
    pub id: i64,
    pub rd_user_id: i64,
    pub title: String,
}

async fn user_fixture() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    for sql in [
        "CREATE TABLE rd_users (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
        "CREATE TABLE rd_profiles (id INTEGER PRIMARY KEY AUTOINCREMENT, \
            rd_user_id INTEGER NOT NULL, bio TEXT NOT NULL)",
        "CREATE TABLE rd_posts (id INTEGER PRIMARY KEY AUTOINCREMENT, \
            rd_user_id INTEGER NOT NULL, title TEXT NOT NULL)",
        "INSERT INTO rd_users (id, name) VALUES (1, 'una')",
        "INSERT INTO rd_profiles (rd_user_id, bio) VALUES (1, 'hello')",
        "INSERT INTO rd_posts (rd_user_id, title) VALUES (1, 'first'), (1, 'second')",
    ] {
        db.execute_unprepared(sql).await.unwrap();
    }
    db
}

/// `with` and `with_count` on the same relation keep both results, in
/// either order.
#[tokio::test]
async fn rows_and_count_of_one_relation_coexist() {
    let _db = user_fixture().await;

    let rows_first = RdUser::query()
        .with(["profile", "posts"])
        .with_count(["profile", "posts"])
        .get()
        .await
        .unwrap();
    let count_first = RdUser::query()
        .with_count(["profile", "posts"])
        .with(["profile", "posts"])
        .get()
        .await
        .unwrap();
    for users in [rows_first, count_first] {
        let user = &users[0];
        assert_eq!(user.profile_loaded().map(|p| p.bio.as_str()), Some("hello"));
        assert_eq!(user.posts_loaded().len(), 2);
        assert_eq!(user.profile_count(), 1);
        assert_eq!(user.posts_count(), 2);
    }
}

/// A count does not stand in for loaded rows: `load_missing` loads a
/// relation that only has a count.
#[tokio::test]
async fn load_missing_loads_rows_a_count_did_not() {
    let _db = user_fixture().await;
    let mut users = RdUser::query().with_count(["posts"]).get().await.unwrap();
    users.load_missing(["posts"]).await.unwrap();
    assert_eq!(users[0].posts_loaded().len(), 2);
    assert_eq!(users[0].posts_count(), 2);
}

// ---- A failed nested load keeps what was loaded -------------------------

#[model(table = "rd_shops", relations = {
    orders: HasMany<RdOrder>,
})]
pub struct RdShop {
    pub id: i64,
    pub name: String,
}

#[model(table = "rd_orders", relations = {
    lines: HasMany<RdLine>,
})]
pub struct RdOrder {
    pub id: i64,
    pub rd_shop_id: i64,
    pub total: i64,
}

#[model(table = "rd_lines")]
pub struct RdLine {
    pub id: i64,
    pub rd_order_id: i64,
    pub sku: String,
}

/// `load_missing(["orders.lines"])` that fails on the `lines` query
/// returns the error and leaves every shop's loaded orders in place.
#[tokio::test]
async fn a_failed_nested_load_keeps_the_rows_already_loaded() {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    for sql in [
        "CREATE TABLE rd_shops (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
        "CREATE TABLE rd_orders (id INTEGER PRIMARY KEY AUTOINCREMENT, \
            rd_shop_id INTEGER NOT NULL, total INTEGER NOT NULL)",
        "INSERT INTO rd_shops (id, name) VALUES (1, 'shop')",
        "INSERT INTO rd_orders (rd_shop_id, total) VALUES (1, 10), (1, 20)",
    ] {
        db.execute_unprepared(sql).await.unwrap();
    }
    let mut shops = RdShop::query().with(["orders"]).get().await.unwrap();
    assert_eq!(shops[0].orders_loaded().len(), 2);

    // `rd_lines` does not exist, so the nested query fails.
    let failed = shops.load_missing(["orders.lines"]).await;
    assert!(failed.is_err(), "the nested load reports the failure");
    let totals: Vec<i64> = shops[0].orders_loaded().iter().map(|o| o.total).collect();
    assert_eq!(totals, vec![10, 20], "the loaded orders are still there");
}
