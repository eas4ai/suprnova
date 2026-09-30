//! The cases for the Laravel schema methods issue #122 asked for: unsigned
//! and tiny integers, medium and long text, enumerations, temporal precision
//! and `use_current`, primary keys other than `id()`, foreign keys on a
//! column declared on its own, and the per-backend fallbacks Laravel makes.
//! Like `cases.rs`, every backend runs them; the few that only one backend
//! can run say so.

use sea_orm::sea_query::{Alias, Query};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use sea_orm_migration::prelude::*;
use suprnova::schema::Schema;
use suprnova::testing::TestContainer;
use suprnova::{DbConnection, Model, attrs, model};

use super::cases::{count, drop_tables, migration_error, run};
use super::catalog;

/// A migration closure, boxed so the refusal cases can sit in one list.
type Define = Box<dyn FnOnce(&mut suprnova::schema::Blueprint) + Send>;

/// Reads one text column of every row, in `id` order.
async fn texts(conn: &DatabaseConnection, table: &str, column: &str) -> Vec<String> {
    let rows = conn
        .query_all_raw(Statement::from_string(
            conn.get_database_backend(),
            format!("SELECT {column} AS v FROM {table} ORDER BY id"),
        ))
        .await
        .expect("select texts");
    rows.iter()
        .map(|row| row.try_get("", "v").expect("text value"))
        .collect()
}

/// Inserts one row that sets only `column`, with the value bound, so a
/// quote or a backslash reaches the database as it is.
async fn insert_one(
    conn: &DatabaseConnection,
    table: &str,
    column: &str,
    value: sea_orm::Value,
) -> Result<(), sea_orm::DbErr> {
    let mut insert = Query::insert();
    insert
        .into_table(Alias::new(table))
        .columns([Alias::new(column)])
        .values_panic([value.into()]);
    let statement = conn.get_database_backend().build(&insert);
    conn.execute_raw(statement).await.map(|_| ())
}

/// The fractional-second digits the catalog reports for a column. SQLite
/// has none to report.
async fn datetime_precision(conn: &DatabaseConnection, table: &str, column: &str) -> Option<i64> {
    let sql = match conn.get_database_backend() {
        DbBackend::Postgres => format!(
            "SELECT datetime_precision::bigint AS p FROM information_schema.columns \
             WHERE table_schema = current_schema() AND table_name = '{table}' AND column_name = '{column}'"
        ),
        DbBackend::MySql => format!(
            "SELECT CAST(datetime_precision AS SIGNED) AS p FROM information_schema.columns \
             WHERE table_schema = DATABASE() AND table_name = '{table}' AND column_name = '{column}'"
        ),
        _ => return None,
    };
    let row = conn
        .query_one_raw(Statement::from_string(conn.get_database_backend(), sql))
        .await
        .expect("datetime_precision query")
        .expect("the column's catalog row");
    row.try_get("", "p").expect("datetime_precision value")
}

/// The columns of `table`'s primary key, in key order.
async fn primary_key_columns(conn: &DatabaseConnection, table: &str) -> Vec<String> {
    let backend = conn.get_database_backend();
    if backend == DbBackend::Sqlite {
        let rows = conn
            .query_all_raw(Statement::from_string(
                backend,
                format!("PRAGMA table_info('{table}')"),
            ))
            .await
            .expect("PRAGMA table_info");
        let mut keyed: Vec<(i64, String)> = rows
            .iter()
            .map(|row| {
                (
                    row.try_get("", "pk").expect("pk"),
                    row.try_get("", "name").expect("name"),
                )
            })
            .filter(|(pk, _)| *pk > 0)
            .collect();
        keyed.sort();
        return keyed.into_iter().map(|(_, name)| name).collect();
    }
    let (name, schema) = if backend == DbBackend::Postgres {
        ("kcu.column_name::text", "current_schema()")
    } else {
        ("CAST(kcu.column_name AS CHAR)", "DATABASE()")
    };
    let sql = format!(
        "SELECT {name} AS c FROM information_schema.table_constraints tc \
         JOIN information_schema.key_column_usage kcu \
           ON tc.constraint_name = kcu.constraint_name \
          AND tc.table_schema = kcu.table_schema AND tc.table_name = kcu.table_name \
         WHERE tc.constraint_type = 'PRIMARY KEY' AND tc.table_name = '{table}' \
           AND tc.table_schema = {schema} \
         ORDER BY kcu.ordinal_position"
    );
    let rows = conn
        .query_all_raw(Statement::from_string(backend, sql))
        .await
        .expect("primary key query");
    rows.iter()
        .map(|row| row.try_get("", "c").expect("key column"))
        .collect()
}

// ---- column types ------------------------------------------------------------

/// The new column methods create the type each backend's Laravel grammar
/// creates, an enumeration refuses a value outside its list and keeps every
/// value in it (a quote and a backslash included), `use_current` fills the
/// column, and `precision` reaches the catalog. It fails against a wrong
/// fallback (an unsigned type widened on Postgres, a `tinyint` on SQLite), an
/// unescaped enumeration value, or a precision that is not applied.
pub async fn laravel_column_types(conn: &DatabaseConnection) {
    let backend = conn.get_database_backend();
    let manager = SchemaManager::new(conn);
    let table = "schema_laravel_types";
    drop_tables(conn, &[table]).await;
    Schema::create(&manager, table, |t| {
        t.id();
        t.tiny_integer("tiny_col").nullable();
        t.unsigned_tiny_integer("utiny_col").nullable();
        t.unsigned_small_integer("usmall_col").nullable();
        t.unsigned_integer("uint_col").nullable();
        t.unsigned_big_integer("ubig_col").nullable();
        t.medium_text("medium_col").nullable();
        t.long_text("long_col").nullable();
        t.enumeration("status", &["draft", "o'clock", "back\\slash"])
            .default("draft");
        t.remember_token();
        t.date_time("stamp_col").precision(6).use_current();
        t.timestamp_tz("tz_col").precision(3).nullable();
        t.time("time_col").precision(0).nullable();
    })
    .await
    .expect("create schema_laravel_types");

    let columns = catalog::columns(conn, table).await;
    let column = |name: &str| {
        columns
            .iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("column {name} missing"))
    };
    let declared = |name: &str| column(name).declared.to_lowercase();
    match backend {
        DbBackend::MySql => {
            for (name, ty, unsigned) in [
                ("tiny_col", "tinyint", false),
                ("utiny_col", "tinyint", true),
                ("usmall_col", "smallint", true),
                ("uint_col", "int", true),
                ("ubig_col", "bigint", true),
            ] {
                let declared = declared(name);
                assert!(declared.starts_with(ty), "{name}: {declared}");
                assert_eq!(
                    declared.contains("unsigned"),
                    unsigned,
                    "{name}: {declared}"
                );
            }
            assert_eq!(column("medium_col").family, "mediumtext");
            assert_eq!(column("long_col").family, "longtext");
            assert_eq!(column("status").family, "enum");
        }
        DbBackend::Postgres => {
            // Laravel keeps the signed type; sea-query alone would widen an
            // unsigned integer to the next size.
            for (name, family) in [
                ("tiny_col", "smallint"),
                ("utiny_col", "smallint"),
                ("usmall_col", "smallint"),
                ("uint_col", "integer"),
                ("ubig_col", "bigint"),
                ("medium_col", "text"),
                ("long_col", "text"),
                ("status", "character varying"),
            ] {
                assert_eq!(column(name).family, family, "{name}");
            }
            assert_eq!(column("status").length, Some(255));
        }
        _ => {
            for name in [
                "tiny_col",
                "utiny_col",
                "usmall_col",
                "uint_col",
                "ubig_col",
            ] {
                assert_eq!(column(name).family, "integer", "{name}");
            }
            assert_eq!(
                declared("tiny_col"),
                "integer",
                "Laravel's SQLite tiny integer"
            );
            for name in ["medium_col", "long_col", "status"] {
                assert_eq!(column(name).family, "text", "{name}");
            }
        }
    }
    let token = column("remember_token");
    assert!(token.nullable);
    assert_eq!(token.length, Some(100));
    if backend != DbBackend::Sqlite {
        assert_eq!(datetime_precision(conn, table, "stamp_col").await, Some(6));
        assert_eq!(datetime_precision(conn, table, "tz_col").await, Some(3));
        assert_eq!(datetime_precision(conn, table, "time_col").await, Some(0));
    }

    for value in ["draft", "o'clock", "back\\slash"] {
        insert_one(conn, table, "status", value.into())
            .await
            .unwrap_or_else(|e| panic!("insert the allowed value {value:?}: {e}"));
    }
    assert_eq!(
        texts(conn, table, "status").await,
        ["draft", "o'clock", "back\\slash"],
        "every allowed value round-trips as it was declared"
    );
    assert!(
        insert_one(conn, table, "status", "bogus".into())
            .await
            .is_err(),
        "a value outside the list must be refused"
    );
    let stamped = conn
        .query_one_raw(Statement::from_string(
            backend,
            format!("SELECT COUNT(*) AS n FROM {table} WHERE stamp_col IS NOT NULL"),
        ))
        .await
        .expect("count stamped")
        .expect("count row");
    assert_eq!(
        stamped.try_get::<i64>("", "n").expect("n"),
        3,
        "use_current fills the column"
    );

    if backend == DbBackend::MySql {
        insert_one(conn, table, "uint_col", 4_294_967_295_i64.into())
            .await
            .expect("an unsigned int holds 2^32 - 1");
        assert!(
            insert_one(conn, table, "uint_col", (-1_i64).into())
                .await
                .is_err(),
            "an unsigned int refuses a negative value"
        );
    }

    drop_tables(conn, &[table]).await;
}

// ---- primary keys ------------------------------------------------------------

/// `primary(&[..])` makes a composite key and `.primary()` a key on a column
/// other than `id`; the catalog reports them as the primary key and a
/// duplicate is refused. It fails when the key is a plain unique index or
/// covers the wrong columns.
pub async fn primary_keys(conn: &DatabaseConnection) {
    let manager = SchemaManager::new(conn);
    drop_tables(conn, &["schema_post_tag", "schema_codes"]).await;
    Schema::create(&manager, "schema_post_tag", |t| {
        t.big_integer("post_id");
        t.big_integer("tag_id");
        t.primary(&["post_id", "tag_id"]);
    })
    .await
    .expect("create schema_post_tag");
    assert_eq!(
        primary_key_columns(conn, "schema_post_tag").await,
        ["post_id", "tag_id"]
    );
    run(
        conn,
        "INSERT INTO schema_post_tag (post_id, tag_id) VALUES (1, 1), (1, 2), (2, 1)",
    )
    .await
    .expect("distinct pairs");
    assert!(
        run(
            conn,
            "INSERT INTO schema_post_tag (post_id, tag_id) VALUES (1, 2)"
        )
        .await
        .is_err(),
        "a repeated pair must be refused"
    );

    Schema::create(&manager, "schema_codes", |t| {
        t.string("code").length(8).primary();
        t.string("label");
    })
    .await
    .expect("create schema_codes");
    assert_eq!(primary_key_columns(conn, "schema_codes").await, ["code"]);
    run(
        conn,
        "INSERT INTO schema_codes (code, label) VALUES ('a', 'x')",
    )
    .await
    .expect("first code");
    assert!(
        run(
            conn,
            "INSERT INTO schema_codes (code, label) VALUES ('a', 'y')"
        )
        .await
        .is_err(),
        "a repeated code must be refused"
    );

    drop_tables(conn, &["schema_post_tag", "schema_codes"]).await;
}

// ---- foreign keys --------------------------------------------------------------

/// `foreign(column)` creates a key on a column declared on its own, with the
/// name `.name()` gave and the action a shorthand set. It fails when the key
/// is missing, ignores `cascade_on_delete`, or carries the default name (the
/// `drop_foreign` by the custom name on Postgres and MySQL would fail).
pub async fn foreign_on_a_declared_column(conn: &DatabaseConnection) {
    let backend = conn.get_database_backend();
    let manager = SchemaManager::new(conn);
    drop_tables(conn, &["schema_orders", "schema_states"]).await;
    Schema::create(&manager, "schema_states", |t| {
        t.id();
        t.string("name");
    })
    .await
    .expect("create schema_states");
    Schema::create(&manager, "schema_orders", |t| {
        t.id();
        t.big_integer("state_id");
        t.foreign("state_id")
            .references("schema_states", "id")
            .name("orders_state_fk")
            .cascade_on_delete();
    })
    .await
    .expect("create schema_orders");

    run(
        conn,
        "INSERT INTO schema_states (id, name) VALUES (1, 'a'), (2, 'b')",
    )
    .await
    .expect("insert states");
    run(
        conn,
        "INSERT INTO schema_orders (id, state_id) VALUES (1, 1), (2, 2)",
    )
    .await
    .expect("insert orders");
    assert!(
        run(
            conn,
            "INSERT INTO schema_orders (id, state_id) VALUES (3, 99)"
        )
        .await
        .is_err(),
        "the key must refuse an orphan row"
    );
    run(conn, "DELETE FROM schema_states WHERE id = 1")
        .await
        .expect("delete a state");
    assert_eq!(count(conn, "schema_orders").await, 1, "cascade_on_delete");

    if backend != DbBackend::Sqlite {
        Schema::table(&manager, "schema_orders", |t| {
            t.drop_foreign("orders_state_fk");
        })
        .await
        .expect("the key carries the name .name() gave");
        run(
            conn,
            "INSERT INTO schema_orders (id, state_id) VALUES (4, 99)",
        )
        .await
        .expect("without the key an orphan row is accepted");
    }

    drop_tables(conn, &["schema_orders", "schema_states"]).await;
}

// ---- altering a table ----------------------------------------------------------

/// Postgres and MySQL: `Schema::table` places a column with `after` on MySQL
/// (and appends it on Postgres, as Laravel does), adds a foreign key with
/// `null_on_delete`, drops it and its column with
/// `drop_constrained_foreign_id`, adds a primary key, and adds a column with
/// `use_current`. It fails when `after` is dropped on MySQL, a shorthand sets
/// the wrong action, the constrained drop leaves the column, or `primary`
/// adds no key.
pub async fn alter_laravel_additions(conn: &DatabaseConnection) {
    let backend = conn.get_database_backend();
    let manager = SchemaManager::new(conn);
    drop_tables(conn, &["schema_members", "schema_teams", "schema_badges"]).await;
    Schema::create(&manager, "schema_teams", |t| {
        t.id();
    })
    .await
    .expect("create schema_teams");
    Schema::create(&manager, "schema_members", |t| {
        t.id();
        t.string("name");
    })
    .await
    .expect("create schema_members");

    Schema::table(&manager, "schema_members", |t| {
        t.string("nickname").nullable().after("id");
        t.foreign_id("team_id")
            .nullable()
            .constrained("schema_teams")
            .null_on_delete();
    })
    .await
    .expect("alter schema_members");
    let order: Vec<String> = catalog::columns(conn, "schema_members")
        .await
        .into_iter()
        .map(|c| c.name)
        .collect();
    let expected: &[&str] = if backend == DbBackend::MySql {
        &["id", "nickname", "name", "team_id"]
    } else {
        &["id", "name", "nickname", "team_id"]
    };
    assert_eq!(order, expected);

    run(conn, "INSERT INTO schema_teams (id) VALUES (1)")
        .await
        .expect("insert team");
    run(
        conn,
        "INSERT INTO schema_members (id, name, team_id) VALUES (1, 'ann', 1)",
    )
    .await
    .expect("insert member");
    run(conn, "DELETE FROM schema_teams WHERE id = 1")
        .await
        .expect("delete team");
    let orphaned = conn
        .query_one_raw(Statement::from_string(
            backend,
            "SELECT COUNT(*) AS n FROM schema_members WHERE team_id IS NULL".to_owned(),
        ))
        .await
        .expect("count nulled")
        .expect("count row");
    assert_eq!(
        orphaned.try_get::<i64>("", "n").expect("n"),
        1,
        "null_on_delete"
    );

    Schema::table(&manager, "schema_members", |t| {
        t.drop_constrained_foreign_id("team_id");
    })
    .await
    .expect("drop the key and its column");
    assert!(
        !Schema::has_column(&manager, "schema_members", "team_id")
            .await
            .expect("has_column")
    );

    Schema::create(&manager, "schema_badges", |t| {
        t.string("code").length(8);
        t.string("label");
    })
    .await
    .expect("create schema_badges");
    Schema::table(&manager, "schema_badges", |t| {
        t.primary(&["code"]);
        t.timestamp_tz("issued_at").use_current();
    })
    .await
    .expect("add a primary key and a use_current column");
    assert_eq!(primary_key_columns(conn, "schema_badges").await, ["code"]);
    run(
        conn,
        "INSERT INTO schema_badges (code, label) VALUES ('a', 'x')",
    )
    .await
    .expect("first badge");
    assert!(
        run(
            conn,
            "INSERT INTO schema_badges (code, label) VALUES ('a', 'y')"
        )
        .await
        .is_err(),
        "the added key must refuse a repeated code"
    );
    let issued = conn
        .query_one_raw(Statement::from_string(
            backend,
            "SELECT COUNT(*) AS n FROM schema_badges WHERE issued_at IS NOT NULL".to_owned(),
        ))
        .await
        .expect("count issued")
        .expect("count row");
    assert_eq!(issued.try_get::<i64>("", "n").expect("n"), 1);

    drop_tables(conn, &["schema_members", "schema_teams", "schema_badges"]).await;
}

/// SQLite: `after` is ignored and the column appended, as Laravel does; a
/// primary key, a `use_current` column and `drop_constrained_foreign_id` are
/// refused before any statement of the call runs. It fails when a refusal
/// comes after an earlier statement ran, or `after` is refused.
pub async fn sqlite_alter_laravel_refusals(conn: &DatabaseConnection) {
    let manager = SchemaManager::new(conn);
    drop_tables(conn, &["schema_lite_badges"]).await;
    Schema::create(&manager, "schema_lite_badges", |t| {
        t.id();
        t.string("code").length(8);
    })
    .await
    .expect("create schema_lite_badges");
    Schema::table(&manager, "schema_lite_badges", |t| {
        t.string("label").nullable().after("id");
    })
    .await
    .expect("after() is ignored on SQLite");
    let last = catalog::columns(conn, "schema_lite_badges")
        .await
        .pop()
        .expect("columns");
    assert_eq!(last.name, "label");

    let refusals: Vec<(&str, Define)> = vec![
        (
            "primary key",
            Box::new(|t| {
                t.string("first").nullable();
                t.primary(&["code"]);
            }),
        ),
        (
            "use_current()",
            Box::new(|t| {
                t.string("first").nullable();
                t.date_time("seen_at").use_current();
            }),
        ),
        (
            "foreign key",
            Box::new(|t| {
                t.string("first").nullable();
                t.drop_constrained_foreign_id("code");
            }),
        ),
    ];
    for (needle, define) in refusals {
        let text = migration_error(Schema::table(&manager, "schema_lite_badges", define).await);
        assert!(text.contains(needle), "{text}");
        assert!(text.contains("SQLite"), "{text}");
        assert!(
            !Schema::has_column(&manager, "schema_lite_badges", "first")
                .await
                .expect("has_column"),
            "the refused call must run nothing ({needle})"
        );
    }

    drop_tables(conn, &["schema_lite_badges"]).await;
}

// ---- refusals ------------------------------------------------------------------

/// Every misuse of the new methods is refused before the first statement,
/// with a text that names it, and creates no table. It fails when a misuse
/// reaches the database (or is silently ignored) or the text does not say
/// what went wrong.
pub async fn laravel_misuse_is_refused(conn: &DatabaseConnection) {
    let manager = SchemaManager::new(conn);
    let table = "schema_refused";
    drop_tables(conn, &[table, "schema_parents"]).await;
    let cases: Vec<(&[&str], Define)> = vec![
        (
            &["after(`id`)", "CREATE TABLE"],
            Box::new(|t| {
                t.id();
                t.string("name").after("id");
            }),
        ),
        (
            &["unsigned()", "integer columns"],
            Box::new(|t| {
                t.string("name").unsigned();
            }),
        ),
        (
            &["precision()", "date_time"],
            Box::new(|t| {
                t.integer("n").precision(3);
            }),
        ),
        (
            &["precision()", "0 to 6"],
            Box::new(|t| {
                t.date_time("at").precision(7);
            }),
        ),
        (
            &["use_current()"],
            Box::new(|t| {
                t.integer("n").use_current();
            }),
        ),
        (
            &["enumeration()", "at least one value"],
            Box::new(|t| {
                t.enumeration("status", &[]);
            }),
        ),
        (
            &["foreign(`parent_id`)", "references"],
            Box::new(|t| {
                t.big_integer("parent_id");
                t.foreign("parent_id").cascade_on_delete();
            }),
        ),
        (
            &["does not declare", "parent_id"],
            Box::new(|t| {
                t.id();
                t.foreign("parent_id").constrained("schema_parents");
            }),
        ),
        (
            &["twice", "same_fk"],
            Box::new(|t| {
                t.big_integer("a");
                t.big_integer("b");
                t.foreign("a").constrained("schema_parents").name("same_fk");
                t.foreign("b").constrained("schema_parents").name("same_fk");
            }),
        ),
        (
            &["second primary key"],
            Box::new(|t| {
                t.id();
                t.string("code").primary();
            }),
        ),
        (
            &["second primary key"],
            Box::new(|t| {
                t.string("code");
                t.primary(&["code"]);
                t.id();
            }),
        ),
        (
            &["nullable", "primary key"],
            Box::new(|t| {
                t.string("code").nullable();
                t.primary(&["code"]);
            }),
        ),
        (
            &["primary key", "does not declare", "missing"],
            Box::new(|t| {
                t.string("code");
                t.primary(&["missing"]);
            }),
        ),
        (
            &["without named columns"],
            Box::new(|t| {
                t.string("code");
                t.primary(&[]);
            }),
        ),
    ];
    for (needles, define) in cases {
        let text = migration_error(Schema::create(&manager, table, define).await);
        for needle in needles {
            assert!(text.contains(needle), "{needle:?} missing from: {text}");
        }
        assert!(
            !Schema::has_table(&manager, table).await.expect("has_table"),
            "a refused create must not create the table: {text}"
        );
    }
}

// ---- unsigned keys through models ----------------------------------------------

/// A table Laravel's `id()` and `foreignId()` would create on MySQL.
#[model(table = "schema_u_users", key_type = "u64", fillable = ["name"], relations = {
    orders: HasMany<SchemaUOrder>,
})]
pub struct SchemaUUser {
    pub id: u64,
    pub name: String,
}

/// An order pointing at [`SchemaUUser`] through an unsigned key.
#[model(
    table = "schema_u_orders",
    key_type = "u64",
    fillable = ["schema_u_user_id", "status"],
    relations = {
        user: BelongsTo<SchemaUUser> { fk = "schema_u_user_id" },
    },
)]
pub struct SchemaUOrder {
    pub id: u64,
    pub schema_u_user_id: u64,
    pub status: String,
}

/// The same users table read with the default `i64` key.
#[model(table = "schema_u_users")]
pub struct SchemaSignedUser {
    pub id: i64,
    pub name: String,
}

/// MySQL: tables made with `unsigned_id` and `unsigned_foreign_id` accept
/// the key, and models with `u64` keys write, read and query across the
/// relation, including a key above `i64::MAX`; an `i64` key cannot read the
/// unsigned column, which is why the manual asks for `u64`. It fails when the
/// foreign key's type does not match (MySQL refuses the table), when the
/// model macro cannot take a `u64` key, or when the driver starts reading
/// unsigned columns into `i64` (the manual's advice would be stale).
pub async fn unsigned_keys_through_models(conn: &DatabaseConnection) {
    let manager = SchemaManager::new(conn);
    drop_tables(conn, &["schema_u_orders", "schema_u_users"]).await;
    Schema::create(&manager, "schema_u_users", |t| {
        t.unsigned_id();
        t.string("name");
    })
    .await
    .expect("create schema_u_users");
    Schema::create(&manager, "schema_u_orders", |t| {
        t.unsigned_id();
        t.unsigned_foreign_id("schema_u_user_id")
            .constrained("schema_u_users")
            .cascade_on_delete();
        t.enumeration("status", &["draft", "paid"]).default("draft");
    })
    .await
    .expect("create schema_u_orders: the unsigned key matches the unsigned id");

    let _guard = TestContainer::fake();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));
    let ada = SchemaUUser::create(attrs! { name: "ada" })
        .await
        .expect("create a user");
    let order = SchemaUOrder::create(attrs! { schema_u_user_id: ada.id, status: "paid" })
        .await
        .expect("create an order");
    let found = SchemaUOrder::find(order.id)
        .await
        .expect("find the order")
        .expect("the order");
    assert_eq!(
        (found.schema_u_user_id, found.status.as_str()),
        (ada.id, "paid")
    );
    assert_eq!(
        SchemaUOrder::query()
            .where_relation("user", "name", "ada")
            .count()
            .await
            .expect("where_relation across the unsigned key"),
        1
    );
    assert_eq!(
        SchemaUUser::query()
            .has("orders")
            .count()
            .await
            .expect("has across the unsigned key"),
        1
    );

    let big = u64::MAX - 1;
    run(
        conn,
        &format!("INSERT INTO schema_u_users (id, name) VALUES ({big}, 'grace')"),
    )
    .await
    .expect("insert a key above i64::MAX");
    let grace = SchemaUUser::find(big)
        .await
        .expect("find a key above i64::MAX")
        .expect("the user");
    assert_eq!(grace.name, "grace");

    let signed =
        SchemaSignedUser::find(<i64 as TryFrom<u64>>::try_from(ada.id).expect("a small id")).await;
    assert!(
        signed.is_err(),
        "the driver reads BIGINT UNSIGNED into an i64 now; revise the manual's u64 advice"
    );

    drop_tables(conn, &["schema_u_orders", "schema_u_users"]).await;
}
