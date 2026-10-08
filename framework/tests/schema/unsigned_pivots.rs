//! Pivot writes of a `u64` key above `i64::MAX` (PAR-044 follow-ups).
//!
//! `attach`, `detach` and `sync` write ids into a pivot table by raw
//! statements. Each id binds by the type its column has: the pivot model's
//! field, or else the key the column points at. MySQL's unsigned columns
//! take the whole `u64` range exactly. Postgres and SQLite store a `u64`
//! signed, so a write of one above `i64::MAX` is refused before anything is
//! sent, as a model's write is, where it used to store a rounded REAL on
//! SQLite and reach Postgres as text.

use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use sea_orm_migration::prelude::*;
use serial_test::serial;
use suprnova::schema::Schema;
use suprnova::testing::TestContainer;
use suprnova::{DB, DbConnection, FrameworkError, Model, attrs, model};

use super::cases::{drop_tables, run};
use super::mysql::connect_mysql;
use super::postgres::connect_postgres;
use super::sqlite::connect_sqlite;

#[model(table = "up_owners", fillable = ["name"], relations = {
    roles: BelongsToMany<UpRole, UpOwnerRole>,
    tags: MorphToMany<UpTag, UpTaggable> { name = "taggable" },
})]
pub struct UpOwner {
    pub id: i64,
    pub name: String,
}

/// A role keyed the way Laravel's `id()` keys a MySQL table.
#[model(table = "up_roles", fillable = ["name"])]
pub struct UpRole {
    pub id: u64,
    pub name: String,
}

/// The pivot, with a `u64` extra column.
#[model(table = "up_owner_roles")]
pub struct UpOwnerRole {
    pub id: i64,
    pub up_owner_id: i64,
    pub up_role_id: u64,
    pub note: Option<u64>,
}

/// A tag keyed by a `u64`, attached polymorphically.
#[model(table = "up_tags", fillable = ["name"])]
pub struct UpTag {
    pub id: u64,
    pub name: String,
}

/// The polymorphic pivot.
#[model(table = "up_taggables")]
pub struct UpTaggable {
    pub id: i64,
    pub up_tag_id: u64,
    pub taggable_id: i64,
    pub taggable_type: String,
}

const TABLES: &[&str] = &[
    "up_owner_roles",
    "up_taggables",
    "up_owners",
    "up_roles",
    "up_tags",
];

async fn create_tables(conn: &DatabaseConnection) {
    let manager = SchemaManager::new(conn);
    drop_tables(conn, TABLES).await;
    Schema::create(&manager, "up_owners", |t| {
        t.id();
        t.string("name");
    })
    .await
    .expect("create up_owners");
    for table in ["up_roles", "up_tags"] {
        Schema::create(&manager, table, |t| {
            t.unsigned_id();
            t.string("name");
        })
        .await
        .expect("create a keyed table");
    }
    Schema::create(&manager, "up_owner_roles", |t| {
        t.id();
        t.big_integer("up_owner_id");
        t.unsigned_big_integer("up_role_id");
        t.unsigned_big_integer("note").nullable();
    })
    .await
    .expect("create up_owner_roles");
    Schema::create(&manager, "up_taggables", |t| {
        t.id();
        t.unsigned_big_integer("up_tag_id");
        t.big_integer("taggable_id");
        t.string("taggable_type");
    })
    .await
    .expect("create up_taggables");
}

/// `column` of every row of `table`, as text, in order.
async fn column_text(conn: &DatabaseConnection, table: &str, column: &str) -> Vec<String> {
    let backend = conn.get_database_backend();
    let text = if backend == DbBackend::MySql {
        format!("CAST({column} AS CHAR)")
    } else {
        format!("CAST({column} AS TEXT)")
    };
    conn.query_all_raw(Statement::from_string(
        backend,
        format!("SELECT {text} AS v FROM {table} ORDER BY {column}"),
    ))
    .await
    .expect("read the pivot")
    .iter()
    .map(|row| {
        row.try_get::<Option<String>>("", "v")
            .expect("the value as text")
            .unwrap_or_default()
    })
    .collect()
}

/// The SQL of each statement the query log recorded.
fn sent() -> Vec<String> {
    DB::get_query_log()
        .expect("the query log")
        .into_iter()
        .map(|query| query.sql)
        .collect()
}

/// On Postgres and SQLite `attach`, `attach_with`, `sync` and the morph
/// `attach` of an id or an extra above `i64::MAX` fail with a database error
/// naming the column, send no write, and store nothing. A REAL that SQLite
/// already holds in the pivot is found by `detach`, by SQLite's own
/// comparison. On MySQL every write stores the exact value, `detach`
/// removes only the id it names, and `sync` and `get` read the unsigned
/// ids attached, so `sync` neither duplicates nor drops one and `get`
/// loads every related row. It fails while pivot ids bind as text, which
/// SQLite stores as a rounded REAL, Postgres refuses only after the
/// statement is sent, and while MySQL's unsigned pivot ids, which decode
/// as neither `i64` nor text, are not read back.
pub async fn pivot_writes_of_a_u64_are_exact_or_refused(conn: &DatabaseConnection) {
    create_tables(conn).await;
    let _guard = TestContainer::fake();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));
    let owner = UpOwner::create(attrs! { name: "owner" })
        .await
        .expect("create an owner");
    let backend = conn.get_database_backend();

    if backend == DbBackend::MySql {
        for id in [u64::MAX - 1, u64::MAX] {
            owner.roles().attach(id).await.expect("attach a large id");
        }
        owner
            .roles()
            .detach(u64::MAX)
            .await
            .expect("detach the largest id");
        assert_eq!(
            column_text(conn, "up_owner_roles", "up_role_id").await,
            vec![(u64::MAX - 1).to_string()],
            "detach removes only the id it names"
        );
        owner
            .roles()
            .sync(vec![1u64, u64::MAX - 1])
            .await
            .expect("sync over large ids");
        assert_eq!(
            column_text(conn, "up_owner_roles", "up_role_id").await,
            vec!["1".to_owned(), (u64::MAX - 1).to_string()],
            "sync keeps the attached id once and adds the other"
        );
        run(
            conn,
            &format!(
                "INSERT INTO up_roles (id, name) VALUES (1, 'one'), ({}, 'top')",
                u64::MAX - 1
            ),
        )
        .await
        .expect("seed the roles");
        let mut roles: Vec<u64> = owner
            .roles()
            .get()
            .await
            .expect("load the roles")
            .into_vec()
            .into_iter()
            .map(|role| role.id)
            .collect();
        roles.sort_unstable();
        assert_eq!(
            roles,
            vec![1, u64::MAX - 1],
            "get reads the unsigned pivot ids"
        );
        owner
            .roles()
            .attach_with(2u64, attrs! { note: u64::MAX })
            .await
            .expect("an extra above i64::MAX");
        assert!(
            column_text(conn, "up_owner_roles", "note")
                .await
                .contains(&u64::MAX.to_string())
        );
        owner
            .tags()
            .attach(u64::MAX)
            .await
            .expect("a morph attach of a large id");
        assert_eq!(
            column_text(conn, "up_taggables", "up_tag_id").await,
            vec![u64::MAX.to_string()]
        );
        run(
            conn,
            &format!(
                "INSERT INTO up_tags (id, name) VALUES ({}, 'top')",
                u64::MAX
            ),
        )
        .await
        .expect("seed the tag");
        let tags: Vec<u64> = owner
            .tags()
            .get()
            .await
            .expect("load the tags")
            .into_vec()
            .into_iter()
            .map(|tag| tag.id)
            .collect();
        assert_eq!(
            tags,
            vec![u64::MAX],
            "get reads the unsigned morph pivot id"
        );
        owner.tags().detach(u64::MAX).await.expect("a morph detach");
        assert!(
            column_text(conn, "up_taggables", "up_tag_id")
                .await
                .is_empty()
        );
        drop_tables(conn, TABLES).await;
        return;
    }

    DB::enable_query_log().expect("enable the query log");
    let refusals: Vec<(&str, Result<(), FrameworkError>, &str)> = vec![
        ("attach", owner.roles().attach(u64::MAX).await, "up_role_id"),
        (
            "attach_with",
            owner
                .roles()
                .attach_with(1u64, attrs! { note: u64::MAX })
                .await,
            "note",
        ),
        (
            "sync",
            owner.roles().sync(vec![1u64, u64::MAX - 1]).await,
            "up_role_id",
        ),
        (
            "morph attach",
            owner.tags().attach(u64::MAX).await,
            "up_tag_id",
        ),
    ];
    let writes: Vec<String> = sent()
        .into_iter()
        .filter(|sql| sql.contains("INSERT"))
        .collect();
    DB::disable_query_log().expect("disable the query log");
    for (operation, result, column) in refusals {
        let error = result.expect_err(operation);
        assert!(
            matches!(error, FrameworkError::Database(ref message) if message.contains(column)),
            "{operation}: a database error naming {column}: {error:?}"
        );
    }
    assert!(writes.is_empty(), "no write was sent: {writes:?}");
    assert!(
        column_text(conn, "up_owner_roles", "up_role_id")
            .await
            .is_empty()
    );
    assert!(
        column_text(conn, "up_taggables", "up_tag_id")
            .await
            .is_empty()
    );

    if backend == DbBackend::Sqlite {
        run(
            conn,
            &format!(
                "INSERT INTO up_owner_roles (up_owner_id, up_role_id) \
                 VALUES ({}, 1.8446744073709552e19)",
                owner.id
            ),
        )
        .await
        .expect("a REAL an older write left");
        owner
            .roles()
            .detach(u64::MAX)
            .await
            .expect("detach the REAL");
        assert!(
            column_text(conn, "up_owner_roles", "up_role_id")
                .await
                .is_empty(),
            "SQLite's own comparison finds the REAL"
        );
    } else {
        owner
            .roles()
            .detach(u64::MAX)
            .await
            .expect("detach of an id no row holds");
    }

    drop_tables(conn, TABLES).await;
}

#[tokio::test]
#[serial]
async fn sqlite_pivot_writes_of_a_u64_are_exact_or_refused() {
    pivot_writes_of_a_u64_are_exact_or_refused(&connect_sqlite().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_pivot_writes_of_a_u64_are_exact_or_refused() {
    pivot_writes_of_a_u64_are_exact_or_refused(&connect_postgres().await).await;
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable MySQL at MYSQL_TEST_URL"]
async fn mysql_pivot_writes_of_a_u64_are_exact_or_refused() {
    pivot_writes_of_a_u64_are_exact_or_refused(&connect_mysql().await).await;
}
