//! `u64` model fields on every database (PAR-044), and what an application
//! gets without the `[package.metadata.suprnova]` settings of PAR-045.
//!
//! A table Laravel creates on MySQL has `BIGINT UNSIGNED` keys, so a model
//! over it declares them `u64`. Postgres and SQLite have no unsigned
//! integers: the same model stores `0..=i64::MAX` in a signed column there,
//! refuses a larger value before anything is sent, and refuses to read a
//! negative one. The SQLite tests run with the ordinary suite; the Postgres
//! and MySQL ones are ignored until `PG_TEST_URL` / `MYSQL_TEST_URL` name a
//! disposable database, like the rest of this binary.
//!
//! The settings themselves are read from an application's `Cargo.toml`, so
//! they are proven by `framework/tests/fixtures/laravel-defaults-probe`,
//! a package that carries them. This binary's package carries neither,
//! which is what the last case checks.

use chrono::{DateTime, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use sea_orm_migration::prelude::*;
use serial_test::serial;
use suprnova::eloquent::EloquentModel;
use suprnova::schema::Schema;
use suprnova::testing::TestContainer;
use suprnova::{DB, DbConnection, Model, attrs, model};

use super::cases::{count, drop_tables, run};
use super::catalog;
use super::mysql::connect_mysql;
use super::postgres::connect_postgres;
use super::sqlite::connect_sqlite;

/// A users table as Laravel's `id()` creates it. The model names no
/// `key_type`: the key type comes from the `id` field.
#[model(table = "ld_users", fillable = ["name"], relations = {
    orders: HasMany<LdOrder>,
})]
pub struct LdUser {
    pub id: u64,
    pub name: String,
}

/// An order with a `u64` key, a `u64` foreign key, and a nullable `u64`.
#[model(
    table = "ld_orders",
    fillable = ["ld_user_id", "quantity", "label"],
    relations = {
        user: BelongsTo<LdUser> { fk = "ld_user_id" },
    },
)]
pub struct LdOrder {
    pub id: u64,
    pub ld_user_id: u64,
    pub quantity: Option<u64>,
    pub label: String,
}

/// [`LdUser`] again, over its own table: nextest runs each case in its own
/// process against one database, so no two cases share a table.
#[model(table = "ld_wide_users", fillable = ["name"], relations = {
    orders: HasMany<LdWideOrder>,
})]
pub struct LdWideUser {
    pub id: u64,
    pub name: String,
}

/// [`LdOrder`] again, over its own table.
#[model(
    table = "ld_wide_orders",
    fillable = ["ld_wide_user_id", "quantity", "label"],
    relations = {
        user: BelongsTo<LdWideUser> { fk = "ld_wide_user_id" },
    },
)]
pub struct LdWideOrder {
    pub id: u64,
    pub ld_wide_user_id: u64,
    pub quantity: Option<u64>,
    pub label: String,
}

/// Date-time fields with no cast of their own, in a package without
/// `[package.metadata.suprnova.model]`.
#[model(table = "ld_stamps", fillable = ["title"])]
pub struct LdStamp {
    pub id: i64,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Compiles only while `#[model]` takes each key type from the `id` field:
/// with the old `i64` default the two keys would not convert to `u64`.
fn inferred_keys(
    user: <LdUser as EloquentModel>::Key,
    order: <LdOrder as EloquentModel>::Key,
) -> (u64, u64) {
    (user, order)
}

/// Compiles only while a `DateTime<Utc>` field without a cast stores text,
/// as it did before the settings existed.
fn stamps_stay_text(row: ld_stamp::Model) -> (String, String) {
    (row.created_at, row.updated_at)
}

const TABLES: &[&str] = &["ld_orders", "ld_users"];
const WIDE_TABLES: &[&str] = &["ld_wide_orders", "ld_wide_users"];

/// Creates a users table and an orders table pointing at it, as Laravel's
/// `id()` and `foreignId()` make them: `[orders, users]`.
async fn create_tables(conn: &DatabaseConnection, tables: &[&str]) {
    let (orders, users) = (tables[0], tables[1]);
    let manager = SchemaManager::new(conn);
    drop_tables(conn, tables).await;
    Schema::create(&manager, users, |t| {
        t.unsigned_id();
        t.string("name");
    })
    .await
    .expect("create the users table");
    let foreign_key = format!("{}_id", users.trim_end_matches('s'));
    Schema::create(&manager, orders, |t| {
        t.unsigned_id();
        t.unsigned_foreign_id(&foreign_key).constrained(users);
        t.unsigned_big_integer("quantity").nullable();
        t.string("label");
    })
    .await
    .expect("create the orders table");
}

/// Reads one integer column as the database stores it, through text, so
/// neither a signed nor an unsigned decoder stands between the test and
/// the stored value.
async fn stored(conn: &DatabaseConnection, table: &str, column: &str, id: u64) -> Option<String> {
    let backend = conn.get_database_backend();
    let text = match backend {
        DbBackend::MySql => format!("CAST({column} AS CHAR)"),
        _ => format!("CAST({column} AS TEXT)"),
    };
    let row = conn
        .query_one_raw(Statement::from_string(
            backend,
            format!("SELECT {text} AS v FROM {table} WHERE id = {id}"),
        ))
        .await
        .expect("read the stored value")
        .expect("the row");
    row.try_get("", "v").expect("the stored value as text")
}

/// Every operation PAR-044 names, with keys and values that fit in an
/// `i64`, so each database stores them: create with a generated id,
/// update and save, `find`, `find_many`, a `where` on the key, both
/// relations directly and eagerly, `with_count`, a paginated query and
/// `model_keys`. It fails when the macro gives a `u64` key field an `i64`
/// key, when Postgres or SQLite cannot read a `u64` (SeaORM reads one on
/// MySQL only), or when any of them returns a wrong id, field, related
/// record, order or count.
pub async fn u64_fields_round_trip(conn: &DatabaseConnection) {
    create_tables(conn, TABLES).await;
    let _guard = TestContainer::fake();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));

    let ada = LdUser::create(attrs! { name: "ada" })
        .await
        .expect("create ada");
    let grace = LdUser::create(attrs! { name: "grace" })
        .await
        .expect("create grace");
    let (ada_id, grace_id) = inferred_keys(ada.id, grace.id);
    assert_eq!(
        stored(conn, "ld_users", "name", ada_id).await.as_deref(),
        Some("ada"),
        "create returns the id the database generated"
    );
    assert_ne!(ada_id, grace_id);

    let first = LdOrder::create(attrs! { ld_user_id: ada.id, quantity: 3, label: "first" })
        .await
        .expect("create the first order");
    let second =
        LdOrder::create(attrs! { ld_user_id: ada.id, quantity: None::<u64>, label: "second" })
            .await
            .expect("create the second order");
    assert_eq!(
        (first.ld_user_id, first.quantity, second.quantity),
        (ada.id, Some(3), None)
    );
    assert_eq!(
        stored(conn, "ld_orders", "label", second.id)
            .await
            .as_deref(),
        Some("second")
    );

    let first = first
        .update(attrs! { quantity: 5 })
        .await
        .expect("update a u64 field");
    assert_eq!(first.quantity, Some(5));
    let mut second_changed = second.clone();
    second_changed.quantity = Some(7);
    second_changed.save().await.expect("save a u64 field");
    assert_eq!(
        stored(conn, "ld_orders", "quantity", second.id)
            .await
            .as_deref(),
        Some("7")
    );

    let found = LdOrder::find(first.id)
        .await
        .expect("find")
        .expect("the first order");
    assert_eq!(
        (
            found.id,
            found.ld_user_id,
            found.quantity,
            found.label.as_str()
        ),
        (first.id, ada.id, Some(5), "first")
    );

    let many = LdOrder::find_many([second.id, first.id])
        .await
        .expect("find_many");
    assert_eq!(
        many.iter().map(|o| o.id).collect::<Vec<_>>(),
        vec![second.id, first.id]
    );

    let by_key = LdOrder::query()
        .filter("id", second.id)
        .first()
        .await
        .expect("where on the key")
        .expect("the second order");
    assert_eq!(
        (by_key.label.as_str(), by_key.quantity),
        ("second", Some(7))
    );
    assert_eq!(
        LdOrder::query()
            .where_in("id", vec![first.id])
            .count()
            .await
            .expect("where_in on the key"),
        1
    );
    assert_eq!(
        LdOrder::query()
            .filter("ld_user_id", ada.id)
            .count()
            .await
            .expect("where on the foreign key"),
        2
    );

    let owner = first
        .user()
        .first()
        .await
        .expect("belongs_to")
        .expect("the owner");
    assert_eq!((owner.id, owner.name.as_str()), (ada.id, "ada"));
    let ada_orders = ada.orders().get().await.expect("has_many");
    let mut labels: Vec<_> = ada_orders.iter().map(|o| o.label.clone()).collect();
    labels.sort();
    assert_eq!(labels, vec!["first", "second"]);
    assert!(grace.orders().get().await.expect("has_many").is_empty());

    let users = LdUser::query()
        .with(["orders"])
        .order_by_asc("id")
        .get()
        .await
        .expect("eager has_many");
    assert_eq!(
        users
            .iter()
            .map(|u| (u.id, u.orders_loaded().len()))
            .collect::<Vec<_>>(),
        vec![(ada.id, 2), (grace.id, 0)]
    );
    let orders = LdOrder::query()
        .with(["user"])
        .order_by_asc("id")
        .get()
        .await
        .expect("eager belongs_to");
    for order in orders.iter() {
        let owner = order.user_loaded().expect("the eager owner");
        assert_eq!(owner.id, ada.id);
    }

    let counted = LdUser::with_count(["orders"])
        .order_by_asc("id")
        .get()
        .await
        .expect("with_count");
    assert_eq!(
        counted
            .iter()
            .map(|u| (u.id, u.orders_count()))
            .collect::<Vec<_>>(),
        vec![(ada.id, 2), (grace.id, 0)]
    );

    let page = LdOrder::query()
        .order_by_asc("id")
        .paginate(1)
        .await
        .expect("paginate");
    assert_eq!(page.total, 2);
    assert_eq!(
        page.data.iter().map(|o| o.id).collect::<Vec<_>>(),
        vec![first.id]
    );

    assert_eq!(
        LdOrder::query()
            .order_by_asc("id")
            .model_keys()
            .await
            .expect("model_keys"),
        vec![first.id, second.id]
    );

    drop_tables(conn, TABLES).await;
}

/// Postgres and SQLite store a `u64` in a signed column. A value above
/// `i64::MAX` is refused with an error naming the column, through create,
/// update, save and a mass update, before anything is sent: the row count
/// and the stored value stay as they were and the query log stays empty.
/// A stored negative value fails the read, naming the column. It fails when
/// the driver's binder panics on the conversion (sea-query-sqlx unwraps
/// it), when a value is truncated or wrapped, or when `-1` reads as a
/// `u64`.
pub async fn u64_above_i64_max_is_refused(conn: &DatabaseConnection) {
    let too_big = i64::MAX as u64 + 1;
    create_tables(conn, WIDE_TABLES).await;
    let _guard = TestContainer::fake();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));

    let ada = LdWideUser::create(attrs! { name: "ada" })
        .await
        .expect("create ada");
    let order = LdWideOrder::create(attrs! { ld_wide_user_id: ada.id, quantity: 1, label: "kept" })
        .await
        .expect("create an order");
    let rows = count(conn, "ld_wide_orders").await;

    let refusals = [
        (
            "create",
            LdWideOrder::create(
                attrs! { ld_wide_user_id: ada.id, quantity: too_big, label: "big" },
            )
            .await
            .map(|_| ()),
            "ld_wide_orders.quantity",
        ),
        (
            "create with a foreign key",
            LdWideOrder::create(attrs! { ld_wide_user_id: too_big, quantity: 1, label: "big" })
                .await
                .map(|_| ()),
            "ld_wide_orders.ld_wide_user_id",
        ),
        (
            "update",
            order
                .clone()
                .update(attrs! { quantity: too_big })
                .await
                .map(|_| ()),
            "ld_wide_orders.quantity",
        ),
        (
            "save",
            {
                let mut changed = order.clone();
                changed.quantity = Some(too_big);
                changed.save().await
            },
            "ld_wide_orders.quantity",
        ),
    ];
    for (operation, result, column) in refusals {
        let error = result.expect_err(operation).to_string();
        assert!(
            error.contains(column) && error.contains(&too_big.to_string()),
            "{operation}: the error names {column} and the value: {error}"
        );
    }

    DB::enable_query_log().expect("enable the query log");
    let error = LdWideOrder::query()
        .update_all(attrs! { quantity: too_big })
        .await
        .expect_err("a mass update above i64::MAX")
        .to_string();
    assert!(
        error.contains("quantity"),
        "the error names the column: {error}"
    );
    assert!(
        DB::get_query_log().expect("query log").is_empty(),
        "nothing reached the database"
    );
    DB::disable_query_log().expect("disable the query log");

    assert!(
        LdWideOrder::find(too_big).await.is_err(),
        "a key above i64::MAX is an error, not a panic in the binder"
    );
    assert_eq!(count(conn, "ld_wide_orders").await, rows);
    assert_eq!(
        stored(conn, "ld_wide_orders", "quantity", order.id)
            .await
            .as_deref(),
        Some("1")
    );

    run(
        conn,
        &format!(
            "UPDATE ld_wide_orders SET quantity = -1 WHERE id = {}",
            order.id
        ),
    )
    .await
    .expect("store a negative quantity");
    let error = LdWideOrder::find(order.id)
        .await
        .expect_err("a negative value does not read as a u64")
        .to_string();
    assert!(
        error.contains("quantity"),
        "the error names the column: {error}"
    );

    run(
        conn,
        "INSERT INTO ld_wide_users (id, name) VALUES (-5, 'negative')",
    )
    .await
    .expect("store a negative key");
    let error = LdWideUser::query()
        .filter("name", "negative")
        .first()
        .await
        .expect_err("a negative key does not read as a u64")
        .to_string();
    assert!(
        error.contains("`id`"),
        "the error names the column: {error}"
    );

    drop_tables(conn, WIDE_TABLES).await;
}

/// MySQL's unsigned columns hold the whole `u64` range. The two users' keys
/// are one apart at the top of the range, where an `f64` cannot tell them
/// apart, so a lookup that lost precision on the way would find both.
/// Every operation of [`u64_fields_round_trip`] returns exactly the right
/// row.
pub async fn u64_full_range_on_mysql(conn: &DatabaseConnection) {
    let top = u64::MAX;
    let below = u64::MAX - 1;
    create_tables(conn, WIDE_TABLES).await;
    let _guard = TestContainer::fake();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));
    run(
        conn,
        &format!("INSERT INTO ld_wide_users (id, name) VALUES ({below}, 'below'), ({top}, 'top')"),
    )
    .await
    .expect("insert keys above i64::MAX");

    let order =
        LdWideOrder::create(attrs! { ld_wide_user_id: below, quantity: top, label: "huge" })
            .await
            .expect("create with values above i64::MAX");
    assert_eq!((order.ld_wide_user_id, order.quantity), (below, Some(top)));
    assert_eq!(
        stored(conn, "ld_wide_orders", "quantity", order.id)
            .await
            .as_deref(),
        Some(top.to_string().as_str())
    );
    let order = order
        .update(attrs! { quantity: below })
        .await
        .expect("update above i64::MAX");
    assert_eq!(order.quantity, Some(below));

    let found = LdWideUser::find(below)
        .await
        .expect("find")
        .expect("the user below the top");
    assert_eq!(found.name, "below");
    let many = LdWideUser::find_many([top, below])
        .await
        .expect("find_many");
    assert_eq!(
        many.iter().map(|u| u.name.as_str()).collect::<Vec<_>>(),
        vec!["top", "below"]
    );
    let filtered = LdWideUser::query()
        .filter("id", below)
        .get()
        .await
        .expect("where on the key");
    assert_eq!(
        filtered.iter().map(|u| u.name.as_str()).collect::<Vec<_>>(),
        vec!["below"]
    );
    assert_eq!(
        LdWideUser::query()
            .where_in("id", vec![top])
            .get()
            .await
            .expect("where_in on the key")
            .iter()
            .map(|u| u.name.clone())
            .collect::<Vec<_>>(),
        vec!["top"]
    );

    let owner = order
        .user()
        .first()
        .await
        .expect("belongs_to")
        .expect("the owner");
    assert_eq!(owner.name, "below");
    assert_eq!(found.orders().get().await.expect("has_many").len(), 1);
    let users = LdWideUser::query()
        .with(["orders"])
        .order_by_asc("id")
        .get()
        .await
        .expect("eager has_many");
    assert_eq!(
        users
            .iter()
            .map(|u| (u.id, u.orders_loaded().len()))
            .collect::<Vec<_>>(),
        vec![(below, 1), (top, 0)]
    );
    let counted = LdWideUser::with_count(["orders"])
        .order_by_asc("id")
        .get()
        .await
        .expect("with_count");
    assert_eq!(
        counted
            .iter()
            .map(|u| (u.id, u.orders_count()))
            .collect::<Vec<_>>(),
        vec![(below, 1), (top, 0)]
    );
    let page = LdWideUser::query()
        .order_by_asc("id")
        .paginate(1)
        .await
        .expect("paginate");
    assert_eq!(
        (
            page.total,
            page.data.iter().map(|u| u.id).collect::<Vec<_>>()
        ),
        (2, vec![below])
    );
    assert_eq!(
        LdWideUser::query()
            .order_by_asc("id")
            .model_keys()
            .await
            .expect("model_keys"),
        vec![below, top]
    );

    drop_tables(conn, WIDE_TABLES).await;
}

/// Without the `[package.metadata.suprnova]` tables, which this package does
/// not carry, `id()` and `foreign_id()` create signed columns and a
/// `DateTime<Utc>` field without a cast stores text, as before PAR-045.
pub async fn nothing_changes_without_the_settings(conn: &DatabaseConnection) {
    let manager = SchemaManager::new(conn);
    drop_tables(conn, &["ld_stamps", "ld_signed"]).await;
    Schema::create(&manager, "ld_signed", |t| {
        t.id();
        t.foreign_id("owner_id");
    })
    .await
    .expect("create ld_signed");
    for column in catalog::columns(conn, "ld_signed").await {
        match conn.get_database_backend() {
            DbBackend::MySql => assert_eq!(column.declared, "bigint", "{}", column.name),
            DbBackend::Postgres => assert_eq!(column.family, "bigint", "{}", column.name),
            _ => assert_eq!(column.family, "integer", "{}", column.name),
        }
    }

    Schema::create(&manager, "ld_stamps", |t| {
        t.id();
        t.string("title");
        t.timestamps();
    })
    .await
    .expect("create ld_stamps");
    let _guard = TestContainer::fake();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));
    let stamp = LdStamp::create(attrs! { title: "text" })
        .await
        .expect("a text cast writes the string columns timestamps() creates");
    let row: ld_stamp::Model = stamp.clone().into();
    let (created_at, _) = stamps_stay_text(row);
    assert_eq!(created_at, stamp.created_at.to_rfc3339());

    drop_tables(conn, &["ld_stamps", "ld_signed"]).await;
}

#[tokio::test]
async fn sqlite_u64_fields_round_trip() {
    u64_fields_round_trip(&connect_sqlite().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_u64_fields_round_trip() {
    u64_fields_round_trip(&connect_postgres().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn mysql_u64_fields_round_trip() {
    u64_fields_round_trip(&connect_mysql().await).await;
}

#[tokio::test]
async fn sqlite_u64_above_i64_max_is_refused() {
    u64_above_i64_max_is_refused(&connect_sqlite().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_u64_above_i64_max_is_refused() {
    u64_above_i64_max_is_refused(&connect_postgres().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn mysql_u64_full_range() {
    u64_full_range_on_mysql(&connect_mysql().await).await;
}

#[tokio::test]
async fn sqlite_nothing_changes_without_the_settings() {
    nothing_changes_without_the_settings(&connect_sqlite().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_nothing_changes_without_the_settings() {
    nothing_changes_without_the_settings(&connect_postgres().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn mysql_nothing_changes_without_the_settings() {
    nothing_changes_without_the_settings(&connect_mysql().await).await;
}
