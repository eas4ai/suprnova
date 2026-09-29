//! The cases every backend runs. Each function takes a connection, drops the
//! tables it uses before it starts (so a rerun on the same database works),
//! and drops them again when it ends.

use chrono::{DateTime, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, DbErr, Statement};
use sea_orm_migration::prelude::*;
use suprnova::schema::Schema;
use suprnova::testing::TestContainer;
use suprnova::{DbConnection, Model, attrs, model};

use super::catalog;

#[model(table = "schema_posts", soft_deletes, fillable = ["title"])]
pub struct SchemaPost {
    pub id: i64,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

async fn drop_tables(conn: &DatabaseConnection, tables: &[&str]) {
    let manager = SchemaManager::new(conn);
    for table in tables {
        Schema::drop_if_exists(&manager, table)
            .await
            .expect("drop_if_exists");
    }
}

async fn run(conn: &DatabaseConnection, sql: &str) -> Result<(), DbErr> {
    conn.execute_unprepared(sql).await.map(|_| ())
}

async fn count(conn: &DatabaseConnection, table: &str) -> i64 {
    let row = conn
        .query_one_raw(Statement::from_string(
            conn.get_database_backend(),
            format!("SELECT COUNT(*) AS n FROM {table}"),
        ))
        .await
        .expect("count query")
        .expect("count row");
    row.try_get("", "n").expect("count value")
}

/// The text of the `DbErr::Migration` a call must return.
fn migration_error(result: Result<(), DbErr>) -> String {
    match result {
        Err(DbErr::Migration(text)) => text,
        other => panic!("expected DbErr::Migration, got {other:?}"),
    }
}

// ---- every column type -----------------------------------------------------

struct Expect {
    name: &'static str,
    logical: &'static str,
    nullable: bool,
    default: Option<&'static [&'static str]>,
    length: Option<i64>,
    decimal: Option<(i64, i64)>,
}

fn expect(name: &'static str, logical: &'static str) -> Expect {
    Expect {
        name,
        logical,
        nullable: false,
        default: None,
        length: None,
        decimal: None,
    }
}

impl Expect {
    fn nullable(mut self) -> Self {
        self.nullable = true;
        self
    }

    fn default(mut self, needles: &'static [&'static str]) -> Self {
        self.default = Some(needles);
        self
    }

    fn length(mut self, length: i64) -> Self {
        self.length = Some(length);
        self
    }

    fn decimal(mut self, precision: i64, scale: i64) -> Self {
        self.decimal = Some((precision, scale));
        self
    }
}

/// The type families the catalog may report for a logical type. On SQLite the
/// family is the affinity, on Postgres and MySQL it is `data_type`. MariaDB
/// stores `JSON` as `longtext`, so both spellings pass on MySQL.
fn families(backend: DbBackend, logical: &str) -> &'static [&'static str] {
    match (backend, logical) {
        (DbBackend::Sqlite, "id" | "bigint" | "int" | "smallint") => &["integer"],
        (DbBackend::Sqlite, "bool") => &["numeric"],
        (DbBackend::Sqlite, "float" | "double" | "decimal") => &["real"],
        (DbBackend::Sqlite, "binary") => &["blob"],
        (DbBackend::Sqlite, _) => &["text"],
        (DbBackend::Postgres, "id" | "bigint") => &["bigint"],
        (DbBackend::Postgres, "int") => &["integer"],
        (DbBackend::Postgres, "smallint") => &["smallint"],
        (DbBackend::Postgres, "bool") => &["boolean"],
        (DbBackend::Postgres, "varchar") => &["character varying"],
        (DbBackend::Postgres, "char" | "ulid") => &["character"],
        (DbBackend::Postgres, "text") => &["text"],
        (DbBackend::Postgres, "float") => &["real"],
        (DbBackend::Postgres, "double") => &["double precision"],
        (DbBackend::Postgres, "decimal") => &["numeric"],
        (DbBackend::Postgres, "date") => &["date"],
        (DbBackend::Postgres, "time") => &["time without time zone"],
        (DbBackend::Postgres, "datetime") => &["timestamp without time zone"],
        (DbBackend::Postgres, "tz") => &["timestamp with time zone"],
        (DbBackend::Postgres, "json") => &["jsonb"],
        (DbBackend::Postgres, "uuid") => &["uuid"],
        (DbBackend::Postgres, "binary") => &["bytea"],
        (_, "id" | "bigint") => &["bigint"],
        (_, "int") => &["int"],
        (_, "smallint") => &["smallint"],
        (_, "bool") => &["tinyint"],
        (_, "varchar") => &["varchar"],
        (_, "char" | "uuid" | "ulid") => &["char"],
        (_, "text") => &["text"],
        (_, "float") => &["float"],
        (_, "double") => &["double"],
        (_, "decimal") => &["decimal"],
        (_, "date") => &["date"],
        (_, "time") => &["time"],
        (_, "datetime") => &["datetime"],
        (_, "tz") => &["timestamp"],
        (_, "json") => &["json", "longtext"],
        (_, "binary") => &["blob"],
        (_, other) => panic!("no expected family for the logical type {other}"),
    }
}

fn expected_columns(backend: DbBackend) -> Vec<Expect> {
    let mut id = expect("id", "id");
    if backend == DbBackend::Postgres {
        id = id.default(&["nextval"]);
    }
    let uuid_length = if backend == DbBackend::Postgres {
        None
    } else {
        Some(36)
    };
    let mut uuid = expect("uuid_col", "uuid");
    uuid.length = uuid_length;
    vec![
        id,
        expect("owner_id", "bigint"),
        expect("big_col", "bigint"),
        expect("int_col", "int"),
        expect("small_col", "smallint"),
        expect("bool_col", "bool"),
        expect("str_col", "varchar").length(255),
        expect("short_str_col", "varchar").length(120),
        expect("char_col", "char").length(8),
        expect("text_col", "text"),
        expect("float_col", "float"),
        expect("double_col", "double"),
        expect("decimal_col", "decimal").decimal(10, 2),
        expect("date_col", "date"),
        expect("time_col", "time"),
        expect("datetime_col", "datetime"),
        expect("tz_col", "tz"),
        expect("json_col", "json"),
        uuid,
        expect("ulid_col", "ulid").length(26),
        expect("binary_col", "binary"),
        expect("nullable_col", "varchar").nullable().length(255),
        expect("default_int", "int").default(&["7"]),
        expect("default_str", "varchar").default(&["x"]).length(255),
        expect("default_bool", "bool").default(&["true", "1"]),
        expect("created_at", "varchar").length(255),
        expect("updated_at", "varchar").length(255),
        expect("deleted_at", "varchar").nullable().length(255),
    ]
}

/// Every column method creates the type the brief names, `NOT NULL` unless
/// marked nullable, with the default the modifier set. It fails against a
/// wrong type mapping, a lost `NOT NULL`, a dropped default, a `string`
/// without its length, and a `timestamps()` or `soft_deletes()` that stops
/// being a string column.
pub async fn every_column_type(conn: &DatabaseConnection) {
    let backend = conn.get_database_backend();
    let manager = SchemaManager::new(conn);
    drop_tables(conn, &["schema_kinds"]).await;

    Schema::create(&manager, "schema_kinds", |t| {
        t.id();
        t.foreign_id("owner_id");
        t.big_integer("big_col");
        t.integer("int_col");
        t.small_integer("small_col");
        t.boolean("bool_col");
        t.string("str_col");
        t.string("short_str_col").length(120);
        t.char("char_col", 8);
        t.text("text_col");
        t.float("float_col");
        t.double("double_col");
        t.decimal("decimal_col", 10, 2);
        t.date("date_col");
        t.time("time_col");
        t.date_time("datetime_col");
        t.timestamp_tz("tz_col");
        t.json("json_col");
        t.uuid("uuid_col");
        t.ulid("ulid_col");
        t.binary("binary_col");
        t.string("nullable_col").nullable();
        t.integer("default_int").default(7);
        t.string("default_str").default("x");
        t.boolean("default_bool").default(true);
        t.timestamps();
        t.soft_deletes();
    })
    .await
    .expect("create schema_kinds");

    let actual = catalog::columns(conn, "schema_kinds").await;
    let expected = expected_columns(backend);
    let actual_names: Vec<&str> = actual.iter().map(|c| c.name.as_str()).collect();
    let expected_names: Vec<&str> = expected.iter().map(|e| e.name).collect();
    assert_eq!(actual_names, expected_names, "the created columns");

    for (column, want) in actual.iter().zip(&expected) {
        let name = want.name;
        let accepted = families(backend, want.logical);
        assert!(
            accepted.contains(&column.family.as_str()),
            "column {name}: type family {:?} is not one of {accepted:?}",
            column.family
        );
        assert_eq!(column.nullable, want.nullable, "column {name}: nullable");
        match (&column.default, want.default) {
            (Some(actual_default), Some(needles)) => assert!(
                needles.iter().any(|needle| actual_default.contains(needle)),
                "column {name}: default {actual_default:?} holds none of {needles:?}"
            ),
            (None, Some(needles)) => panic!("column {name}: no default, wanted one of {needles:?}"),
            // MariaDB reports the absence of a default on a nullable column as the text NULL.
            (Some(actual_default), None) => assert!(
                actual_default == "null",
                "column {name}: unexpected default {actual_default:?}"
            ),
            (None, None) => {}
        }
        if let Some(length) = want.length {
            assert_eq!(column.length, Some(length), "column {name}: length");
        }
        if let Some((precision, scale)) = want.decimal {
            assert_eq!(
                column.precision,
                Some(precision),
                "column {name}: precision"
            );
            assert_eq!(column.scale, Some(scale), "column {name}: scale");
        }
        // MySQL shows `unsigned` in the column type; a key column that
        // differs in signedness from `id()` is refused by the database.
        if matches!(want.logical, "id" | "bigint") {
            assert!(
                !column.declared.to_lowercase().contains("unsigned"),
                "column {name}: {:?} must be signed",
                column.declared
            );
        }
    }

    drop_tables(conn, &["schema_kinds"]).await;
}

// ---- the model round trip --------------------------------------------------

/// A `#[suprnova::model]` with timestamps and soft deletes creates, updates,
/// soft deletes, restores and force deletes a row of a table from
/// `Schema::create`. It fails when `timestamps()` or `soft_deletes()` makes a
/// column type the model's default `DateTime<Utc>` cast cannot write, which is
/// what a native `timestamptz` column does on Postgres.
pub async fn model_round_trip(conn: &DatabaseConnection) {
    let manager = SchemaManager::new(conn);
    drop_tables(conn, &["schema_posts"]).await;
    Schema::create(&manager, "schema_posts", |t| {
        t.id();
        t.string("title");
        t.timestamps();
        t.soft_deletes();
    })
    .await
    .expect("create schema_posts");

    let _guard = TestContainer::fake();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));

    let post = SchemaPost::create(attrs! { title: "first" })
        .await
        .expect("create");
    let id = post.id;
    assert_eq!(post.title, "first");
    assert!(post.deleted_at.is_none());

    let post = post
        .update(attrs! { title: "second" })
        .await
        .expect("update");
    assert_eq!(post.title, "second");
    assert!(post.updated_at >= post.created_at);

    post.delete().await.expect("soft delete");
    assert!(
        SchemaPost::find(id)
            .await
            .expect("find after delete")
            .is_none()
    );
    let trashed = SchemaPost::with_trashed()
        .filter("id", id)
        .first()
        .await
        .expect("with_trashed")
        .expect("the soft deleted row");
    assert!(trashed.deleted_at.is_some());

    trashed.restore().await.expect("restore");
    let restored = SchemaPost::find(id)
        .await
        .expect("find after restore")
        .expect("the restored row");
    assert!(restored.deleted_at.is_none());

    restored.force_delete().await.expect("force delete");
    assert!(
        SchemaPost::with_trashed()
            .filter("id", id)
            .first()
            .await
            .expect("with_trashed after force delete")
            .is_none()
    );

    drop_tables(conn, &["schema_posts"]).await;
}

// ---- foreign keys ----------------------------------------------------------

/// `foreign_id(..).constrained(..).on_delete(Cascade)` in `Schema::create`
/// makes a key the database enforces: an orphan insert fails and deleting the
/// parent deletes the child rows. It fails when the key is missing, or when
/// the action is not applied.
pub async fn foreign_key_cascade(conn: &DatabaseConnection) {
    let manager = SchemaManager::new(conn);
    drop_tables(conn, &["schema_books", "schema_authors"]).await;
    Schema::create(&manager, "schema_authors", |t| {
        t.id();
        t.string("name");
    })
    .await
    .expect("create schema_authors");
    Schema::create(&manager, "schema_books", |t| {
        t.id();
        t.foreign_id("author_id")
            .constrained("schema_authors")
            .on_delete(ForeignKeyAction::Cascade);
        t.string("title");
    })
    .await
    .expect("create schema_books");

    run(
        conn,
        "INSERT INTO schema_authors (id, name) VALUES (1, 'ann'), (2, 'bob')",
    )
    .await
    .expect("insert authors");
    run(
        conn,
        "INSERT INTO schema_books (id, author_id, title) VALUES (1, 1, 'a'), (2, 1, 'b'), (3, 2, 'c')",
    )
    .await
    .expect("insert books");
    assert_eq!(count(conn, "schema_books").await, 3);

    let orphan = run(
        conn,
        "INSERT INTO schema_books (id, author_id, title) VALUES (9, 99, 'orphan')",
    )
    .await;
    assert!(orphan.is_err(), "the key must refuse an orphan row");

    run(conn, "DELETE FROM schema_authors WHERE id = 1")
        .await
        .expect("delete author");
    assert_eq!(count(conn, "schema_books").await, 1);

    drop_tables(conn, &["schema_books", "schema_authors"]).await;
}

/// `Schema::table` adds a `foreign_id(..).constrained(..)` column and key to
/// an existing table, and `drop_foreign` removes the key by its documented
/// name. Postgres and MySQL only: SQLite refuses both. It fails when the key
/// is not created, is not named `{table}_{column}_foreign`, or is not dropped.
pub async fn alter_foreign_key(conn: &DatabaseConnection) {
    let manager = SchemaManager::new(conn);
    drop_tables(conn, &["schema_pets", "schema_owners"]).await;
    Schema::create(&manager, "schema_owners", |t| {
        t.id();
        t.string("name");
    })
    .await
    .expect("create schema_owners");
    Schema::create(&manager, "schema_pets", |t| {
        t.id();
        t.string("name");
    })
    .await
    .expect("create schema_pets");

    Schema::table(&manager, "schema_pets", |t| {
        t.foreign_id("owner_id")
            .constrained("schema_owners")
            .on_delete(ForeignKeyAction::Cascade);
    })
    .await
    .expect("add the foreign key");

    run(
        conn,
        "INSERT INTO schema_owners (id, name) VALUES (1, 'ann')",
    )
    .await
    .expect("insert owner");
    run(
        conn,
        "INSERT INTO schema_pets (id, name, owner_id) VALUES (1, 'rex', 1)",
    )
    .await
    .expect("insert pet");
    let orphan = run(
        conn,
        "INSERT INTO schema_pets (id, name, owner_id) VALUES (2, 'lost', 99)",
    )
    .await;
    assert!(orphan.is_err(), "the added key must refuse an orphan row");
    run(conn, "DELETE FROM schema_owners WHERE id = 1")
        .await
        .expect("delete owner");
    assert_eq!(count(conn, "schema_pets").await, 0);

    Schema::table(&manager, "schema_pets", |t| {
        t.drop_foreign("schema_pets_owner_id_foreign");
    })
    .await
    .expect("drop the foreign key by its name");
    run(
        conn,
        "INSERT INTO schema_pets (id, name, owner_id) VALUES (3, 'free', 99)",
    )
    .await
    .expect("without the key an orphan row is accepted");

    drop_tables(conn, &["schema_pets", "schema_owners"]).await;
}

// ---- indexes ---------------------------------------------------------------

/// A unique index refuses a duplicate, on one column and on two, and the
/// indexes carry the documented names. It fails when `.unique()` or
/// `unique(&[..])` creates no unique index, or names it differently.
pub async fn unique_index_refuses_duplicate(conn: &DatabaseConnection) {
    let manager = SchemaManager::new(conn);
    drop_tables(conn, &["schema_accounts"]).await;
    Schema::create(&manager, "schema_accounts", |t| {
        t.id();
        t.string("email").unique();
        t.string("given_name");
        t.string("family_name");
        t.unique(&["given_name", "family_name"]);
        t.index(&["family_name"]);
    })
    .await
    .expect("create schema_accounts");

    for index in [
        "schema_accounts_email_unique",
        "schema_accounts_given_name_family_name_unique",
        "schema_accounts_family_name_index",
    ] {
        assert!(
            manager
                .has_index("schema_accounts", index)
                .await
                .expect("has_index"),
            "index {index} must exist"
        );
    }

    run(
        conn,
        "INSERT INTO schema_accounts (id, email, given_name, family_name) VALUES (1, 'a@x', 'ann', 'lee')",
    )
    .await
    .expect("first row");
    let same_email = run(
        conn,
        "INSERT INTO schema_accounts (id, email, given_name, family_name) VALUES (2, 'a@x', 'bob', 'kim')",
    )
    .await;
    assert!(same_email.is_err(), "a duplicate email must be refused");
    let same_name = run(
        conn,
        "INSERT INTO schema_accounts (id, email, given_name, family_name) VALUES (3, 'c@x', 'ann', 'lee')",
    )
    .await;
    assert!(same_name.is_err(), "a duplicate name pair must be refused");
    run(
        conn,
        "INSERT INTO schema_accounts (id, email, given_name, family_name) VALUES (4, 'd@x', 'ann', 'kim')",
    )
    .await
    .expect("a row that differs in both keys is accepted");

    drop_tables(conn, &["schema_accounts"]).await;
}

/// `Schema::table` adds an index and a unique index, then drops both by name.
/// It fails when the index is not created, is not enforced, or `drop_index`
/// leaves it behind.
pub async fn alter_indexes(conn: &DatabaseConnection) {
    let manager = SchemaManager::new(conn);
    drop_tables(conn, &["schema_articles"]).await;
    Schema::create(&manager, "schema_articles", |t| {
        t.id();
        t.string("title");
        t.string("slug");
    })
    .await
    .expect("create schema_articles");

    Schema::table(&manager, "schema_articles", |t| {
        t.index(&["title"]);
        t.unique(&["slug"]);
    })
    .await
    .expect("add indexes");
    for index in ["schema_articles_title_index", "schema_articles_slug_unique"] {
        assert!(
            manager
                .has_index("schema_articles", index)
                .await
                .expect("has_index"),
            "index {index} must exist"
        );
    }
    run(
        conn,
        "INSERT INTO schema_articles (id, title, slug) VALUES (1, 'one', 's')",
    )
    .await
    .expect("first row");
    let duplicate = run(
        conn,
        "INSERT INTO schema_articles (id, title, slug) VALUES (2, 'two', 's')",
    )
    .await;
    assert!(
        duplicate.is_err(),
        "the added unique index must be enforced"
    );

    Schema::table(&manager, "schema_articles", |t| {
        t.drop_index("schema_articles_title_index");
        t.drop_index("schema_articles_slug_unique");
    })
    .await
    .expect("drop indexes");
    for index in ["schema_articles_title_index", "schema_articles_slug_unique"] {
        assert!(
            !manager
                .has_index("schema_articles", index)
                .await
                .expect("has_index"),
            "index {index} must be gone"
        );
    }
    run(
        conn,
        "INSERT INTO schema_articles (id, title, slug) VALUES (2, 'two', 's')",
    )
    .await
    .expect("the duplicate is accepted once the unique index is gone");

    drop_tables(conn, &["schema_articles"]).await;
}

// ---- altering columns ------------------------------------------------------

/// `Schema::table` adds columns, renames one and drops one, several
/// operations in one closure, each as its own statement. Rows survive.
/// It fails when an operation is skipped, when the operations of one closure
/// are sent as a single statement (SQLite refuses that), or when data is lost.
pub async fn alter_columns(conn: &DatabaseConnection) {
    let manager = SchemaManager::new(conn);
    drop_tables(conn, &["schema_profiles"]).await;
    Schema::create(&manager, "schema_profiles", |t| {
        t.id();
        t.string("name");
    })
    .await
    .expect("create schema_profiles");
    run(
        conn,
        "INSERT INTO schema_profiles (id, name) VALUES (1, 'ann')",
    )
    .await
    .expect("insert");

    Schema::table(&manager, "schema_profiles", |t| {
        t.string("nickname").nullable();
        t.integer("visits").default(0);
        t.rename_column("name", "full_name");
    })
    .await
    .expect("add two columns and rename one in one call");
    for (column, present) in [
        ("nickname", true),
        ("visits", true),
        ("name", false),
        ("full_name", true),
    ] {
        assert_eq!(
            Schema::has_column(&manager, "schema_profiles", column)
                .await
                .expect("has_column"),
            present,
            "column {column}"
        );
    }
    run(
        conn,
        "UPDATE schema_profiles SET nickname = 'annie' WHERE full_name = 'ann'",
    )
    .await
    .expect("the renamed column keeps its data");

    Schema::table(&manager, "schema_profiles", |t| {
        t.drop_column("nickname");
    })
    .await
    .expect("drop a column");
    assert!(
        !Schema::has_column(&manager, "schema_profiles", "nickname")
            .await
            .expect("has_column")
    );
    assert_eq!(count(conn, "schema_profiles").await, 1);

    drop_tables(conn, &["schema_profiles"]).await;
}

// ---- tables ----------------------------------------------------------------

/// `drop_if_exists` succeeds on a missing table, `drop` fails on one, and
/// `rename` and `has_table` agree about which table exists. It fails when
/// `drop_if_exists` errors on a missing table, when `drop` hides the missing
/// table, or when `rename` leaves the old name.
pub async fn tables_drop_and_rename(conn: &DatabaseConnection) {
    let manager = SchemaManager::new(conn);
    drop_tables(conn, &["schema_renamed", "schema_original"]).await;

    Schema::drop_if_exists(&manager, "schema_original")
        .await
        .expect("drop_if_exists on a missing table");
    assert!(
        Schema::drop(&manager, "schema_original").await.is_err(),
        "drop on a missing table must fail"
    );
    assert!(
        !Schema::has_table(&manager, "schema_original")
            .await
            .expect("has_table")
    );

    Schema::create(&manager, "schema_original", |t| {
        t.id();
    })
    .await
    .expect("create schema_original");
    assert!(
        Schema::has_table(&manager, "schema_original")
            .await
            .expect("has_table")
    );

    Schema::rename(&manager, "schema_original", "schema_renamed")
        .await
        .expect("rename");
    assert!(
        !Schema::has_table(&manager, "schema_original")
            .await
            .expect("has_table")
    );
    assert!(
        Schema::has_column(&manager, "schema_renamed", "id")
            .await
            .expect("has_column")
    );

    Schema::drop(&manager, "schema_renamed")
        .await
        .expect("drop an existing table");
    assert!(
        !Schema::has_table(&manager, "schema_renamed")
            .await
            .expect("has_table")
    );
}

// ---- refusals that need no particular backend ------------------------------

/// A description the builder cannot honour is refused with a `DbErr::Migration`
/// that names the table and what to do, and no statement runs. It fails when
/// an alteration in `Schema::create`, a length on a `text` column, a duplicate
/// column or an action without a referenced table is accepted, or when the
/// table appears in the database after the refusal.
pub async fn misuse_is_refused_before_any_statement(conn: &DatabaseConnection) {
    let manager = SchemaManager::new(conn);
    drop_tables(conn, &["schema_misuse"]).await;

    let text = migration_error(
        Schema::create(&manager, "schema_misuse", |t| {
            t.id();
            t.drop_column("gone");
        })
        .await,
    );
    assert!(text.contains("schema_misuse"), "{text}");
    assert!(text.contains("drop_column"), "{text}");
    assert!(text.contains("Schema::table"), "{text}");

    let text = migration_error(
        Schema::create(&manager, "schema_misuse", |t| {
            t.id();
            t.text("body").length(10);
        })
        .await,
    );
    assert!(text.contains("body"), "{text}");
    assert!(text.contains("schema_misuse"), "{text}");

    let text = migration_error(
        Schema::create(&manager, "schema_misuse", |t| {
            t.id();
            t.string("name");
            t.string("name");
        })
        .await,
    );
    assert!(text.contains("name"), "{text}");

    let text = migration_error(
        Schema::create(&manager, "schema_misuse", |t| {
            t.id();
            t.foreign_id("owner_id")
                .on_delete(ForeignKeyAction::Cascade);
        })
        .await,
    );
    assert!(text.contains("owner_id"), "{text}");

    let text = migration_error(Schema::create(&manager, "schema_misuse", |_t| {}).await);
    assert!(text.contains("schema_misuse"), "{text}");

    assert!(
        !Schema::has_table(&manager, "schema_misuse")
            .await
            .expect("has_table"),
        "a refused description must leave no table behind"
    );
}

/// An index over a column the closure did not declare is refused before any
/// statement runs, so no empty table is left behind (on MySQL and SQLite the
/// migrator has no transaction to roll one back). It fails when the plan lets
/// `CREATE TABLE` run and the `CREATE INDEX` fail after it.
pub async fn undeclared_index_column_is_refused(conn: &DatabaseConnection) {
    let manager = SchemaManager::new(conn);
    drop_tables(conn, &["schema_undeclared"]).await;

    let text = migration_error(
        Schema::create(&manager, "schema_undeclared", |t| {
            t.id();
            t.string("name");
            t.index(&["nme"]);
        })
        .await,
    );
    assert!(text.contains("schema_undeclared"), "{text}");
    assert!(text.contains("schema_undeclared_nme_index"), "{text}");
    assert!(text.contains("`nme`"), "{text}");
    assert!(
        !Schema::has_table(&manager, "schema_undeclared")
            .await
            .expect("has_table"),
        "a refused description must leave no table behind"
    );

    let text = migration_error(
        Schema::create(&manager, "schema_undeclared", |t| {
            t.id();
            t.string("name");
            t.unique(&["name", "nmae"]);
        })
        .await,
    );
    assert!(text.contains("`nmae`"), "{text}");
    assert!(
        !Schema::has_table(&manager, "schema_undeclared")
            .await
            .expect("has_table")
    );
}

/// A column added twice in one `Schema::table` call is refused before any
/// statement runs, so the table is unchanged. It fails when the first
/// `ADD COLUMN` runs and the second fails after it.
pub async fn duplicate_added_column_is_refused(conn: &DatabaseConnection) {
    let manager = SchemaManager::new(conn);
    drop_tables(conn, &["schema_dupes"]).await;
    Schema::create(&manager, "schema_dupes", |t| {
        t.id();
        t.string("name");
    })
    .await
    .expect("create schema_dupes");

    let text = migration_error(
        Schema::table(&manager, "schema_dupes", |t| {
            t.string("nickname").nullable();
            t.string("nickname").nullable();
        })
        .await,
    );
    assert!(text.contains("schema_dupes"), "{text}");
    assert!(text.contains("`nickname`"), "{text}");
    assert!(
        !Schema::has_column(&manager, "schema_dupes", "nickname")
            .await
            .expect("has_column"),
        "a refused call must leave the table unchanged"
    );
    let columns = catalog::columns(conn, "schema_dupes").await;
    assert_eq!(columns.len(), 2, "the table keeps its two columns");

    drop_tables(conn, &["schema_dupes"]).await;
}

/// A string default holding a single quote survives the statement and is
/// read back unchanged, in `Schema::create` and in `Schema::table`. It fails
/// when the default is not escaped.
pub async fn quoted_string_default_round_trips(conn: &DatabaseConnection) {
    let manager = SchemaManager::new(conn);
    drop_tables(conn, &["schema_quotes"]).await;
    Schema::create(&manager, "schema_quotes", |t| {
        t.id();
        t.string("motto").default("it's");
    })
    .await
    .expect("create schema_quotes");
    Schema::table(&manager, "schema_quotes", |t| {
        t.string("alias").default("o'neil");
    })
    .await
    .expect("add a column with a quoted default");

    run(conn, "INSERT INTO schema_quotes (id) VALUES (1)")
        .await
        .expect("insert without the defaulted columns");
    let row = conn
        .query_one_raw(Statement::from_string(
            conn.get_database_backend(),
            "SELECT motto, alias FROM schema_quotes WHERE id = 1".to_owned(),
        ))
        .await
        .expect("select")
        .expect("the row");
    let motto: String = row.try_get("", "motto").expect("motto");
    let alias: String = row.try_get("", "alias").expect("alias");
    assert_eq!(motto, "it's");
    assert_eq!(alias, "o'neil");

    drop_tables(conn, &["schema_quotes"]).await;
}

// ---- one Migrator, both styles ---------------------------------------------

struct CreateLegacyTable;

impl MigrationName for CreateLegacyTable {
    fn name(&self) -> &str {
        "m20260101_000001_create_schema_mix_legacy"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateLegacyTable {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Alias::new("schema_mix_legacy"))
                    .col(
                        ColumnDef::new(Alias::new("id"))
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(Alias::new("schema_mix_legacy"))
                    .to_owned(),
            )
            .await
    }
}

struct CreateModernTable;

impl MigrationName for CreateModernTable {
    fn name(&self) -> &str {
        "m20260101_000002_create_schema_mix_modern"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateModernTable {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        Schema::create(manager, "schema_mix_modern", |t| {
            t.id();
            t.foreign_id("legacy_id").constrained("schema_mix_legacy");
        })
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        Schema::drop(manager, "schema_mix_modern").await
    }
}

struct MixedMigrator;

#[async_trait::async_trait]
impl MigratorTrait for MixedMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(CreateLegacyTable), Box::new(CreateModernTable)]
    }

    fn migration_table_name() -> DynIden {
        Alias::new("schema_test_migrations").into_iden()
    }
}

/// One `Migrator` holds a SeaORM-style migration and a `Schema` migration
/// that references its table, and runs both up and down. It fails when the
/// builder cannot share a `Migrator` with hand-written SeaORM migrations, or
/// when the `Schema` migration does not run on the migrator's connection.
pub async fn migrator_runs_both_styles(conn: &DatabaseConnection) {
    let manager = SchemaManager::new(conn);
    drop_tables(
        conn,
        &[
            "schema_mix_modern",
            "schema_mix_legacy",
            "schema_test_migrations",
        ],
    )
    .await;

    MixedMigrator::up(conn, None).await.expect("migrate up");
    assert!(
        Schema::has_table(&manager, "schema_mix_legacy")
            .await
            .expect("has_table")
    );
    assert!(
        Schema::has_table(&manager, "schema_mix_modern")
            .await
            .expect("has_table")
    );

    MixedMigrator::down(conn, None).await.expect("migrate down");
    assert!(
        !Schema::has_table(&manager, "schema_mix_modern")
            .await
            .expect("has_table")
    );
    assert!(
        !Schema::has_table(&manager, "schema_mix_legacy")
            .await
            .expect("has_table")
    );

    drop_tables(conn, &["schema_test_migrations"]).await;
}

pub async fn a_name_longer_than_the_limit_is_refused(conn: &DatabaseConnection) {
    let manager = SchemaManager::new(conn);
    drop_tables(conn, &["schema_long_names"]).await;
    let column = "a_column_whose_name_makes_the_index_name_too_long";

    let text = migration_error(
        Schema::create(&manager, "schema_long_names", |t| {
            t.id();
            t.string(column);
            t.index(&[column]);
        })
        .await,
    );
    assert!(text.contains("63 bytes"), "{text}");
    assert!(text.contains("schema_long_names"), "{text}");
    assert!(
        !Schema::has_table(&manager, "schema_long_names")
            .await
            .expect("has_table"),
        "a refused description must leave no table behind, on every backend"
    );
}
