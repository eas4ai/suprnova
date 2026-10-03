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
        t.enumeration("status", &["draft", "o'clock", "semi;colon"])
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

    for value in ["draft", "o'clock", "semi;colon"] {
        insert_one(conn, table, "status", value.into())
            .await
            .unwrap_or_else(|e| panic!("insert the allowed value {value:?}: {e}"));
    }
    if backend == DbBackend::MySql {
        // MySQL refuses an out-of-list enum value, and an out-of-range
        // integer, only in strict mode; outside it, it stores a blank or a
        // clamped value with a warning.
        let mode = conn
            .query_one_raw(Statement::from_string(
                backend,
                "SELECT CAST(@@SESSION.sql_mode AS CHAR) AS m".to_owned(),
            ))
            .await
            .expect("sql_mode query")
            .expect("sql_mode row");
        let mode: String = mode.try_get("", "m").expect("sql_mode");
        assert!(
            mode.contains("STRICT_TRANS_TABLES") || mode.contains("STRICT_ALL_TABLES"),
            "these assertions need a strict sql_mode, got {mode}"
        );
    }
    assert_eq!(
        texts(conn, table, "status").await,
        ["draft", "o'clock", "semi;colon"],
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
        t.enumeration("tier", &["gold", "silver"]).default("silver");
        t.string("owner").nullable().index();
    })
    .await
    .expect("add a primary key, a use_current column, an enumeration and an index");
    assert!(
        manager
            .has_index("schema_badges", "schema_badges_owner_index")
            .await
            .expect("has_index")
    );
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
    assert!(
        run(
            conn,
            "INSERT INTO schema_badges (code, label, tier) VALUES ('b', 'y', 'bronze')"
        )
        .await
        .is_err(),
        "the added enumeration refuses a value outside its list"
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
    Schema::table(&manager, "schema_lite_badges", |t| {
        t.enumeration("tier", &["gold", "silver"]).default("silver");
    })
    .await
    .expect("SQLite adds an enumeration column with its CHECK");
    assert!(
        run(
            conn,
            "INSERT INTO schema_lite_badges (id, code, tier) VALUES (1, 'a', 'bronze')"
        )
        .await
        .is_err(),
        "the added CHECK refuses a value outside the list"
    );

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
        (
            &["use_current()", "default(..)"],
            Box::new(|t| {
                t.date_time("at")
                    .use_current()
                    .default("2031-03-14 00:00:00");
            }),
        ),
        (
            &["enumeration()", "backslash"],
            Box::new(|t| {
                t.enumeration("path", &["a\\b"]);
            }),
        ),
        (
            &["enumeration()", "`Draft ` is listed twice"],
            Box::new(|t| {
                t.enumeration("status", &["draft", "Draft "]);
            }),
        ),
        (
            &["default `paid`", "not one of its values"],
            Box::new(|t| {
                t.enumeration("status", &["draft"]).default("paid");
            }),
        ),
        (
            &["enumeration()", "`draft` is listed twice"],
            Box::new(|t| {
                t.enumeration("status", &["draft", "paid", "draft"]);
            }),
        ),
        (
            &["after()", "empty name"],
            Box::new(|t| {
                t.string("name").after("");
            }),
        ),
        (
            &["length", "string columns only"],
            Box::new(|t| {
                t.enumeration("status", &["draft"]).length(20);
            }),
        ),
        (
            &["sets it to NULL", "NOT NULL", "nullable()"],
            Box::new(|t| {
                t.foreign_id("parent_id")
                    .constrained("schema_parents")
                    .null_on_delete();
            }),
        ),
        (
            &["twice the same name", "schema_refused_parent_id_index"],
            Box::new(|t| {
                t.big_integer("parent_id");
                t.index(&["parent_id"]);
                t.foreign("parent_id")
                    .constrained("schema_parents")
                    .name("schema_refused_parent_id_index");
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

/// Misuse in `Schema::table` is refused before the first statement. It
/// fails when `id()` and `primary` in one call reach the database, or a
/// `null_on_delete` on a NOT NULL column is only refused by MySQL after the
/// column was added.
pub async fn laravel_alter_misuse_is_refused(conn: &DatabaseConnection) {
    let backend = conn.get_database_backend();
    let manager = SchemaManager::new(conn);
    let table = "schema_alter_refused";
    drop_tables(conn, &[table]).await;
    Schema::create(&manager, table, |t| {
        t.string("code").length(8);
    })
    .await
    .expect("create schema_alter_refused");
    let cases: Vec<(&[&str], Define)> = vec![
        (
            if backend == DbBackend::Sqlite {
                &["primary key column", "SQLite"]
            } else {
                &["second primary key"]
            },
            Box::new(|t| {
                t.string("first").nullable();
                t.id();
                t.primary(&["code"]);
            }),
        ),
        (
            if backend == DbBackend::Sqlite {
                &["foreign key", "SQLite"]
            } else {
                &["sets it to NULL", "NOT NULL"]
            },
            Box::new(|t| {
                t.string("first").nullable();
                t.foreign_id("parent_id")
                    .default(1)
                    .constrained("schema_parents")
                    .null_on_update();
            }),
        ),
    ];
    for (needles, define) in cases {
        let text = migration_error(Schema::table(&manager, table, define).await);
        for needle in needles {
            assert!(text.contains(needle), "{needle:?} missing from: {text}");
        }
        assert!(
            !Schema::has_column(&manager, table, "first")
                .await
                .expect("has_column"),
            "the refused call must run nothing: {text}"
        );
    }
    drop_tables(conn, &[table]).await;
}

// ---- referential actions -------------------------------------------------------

/// Every action shorthand sets the action it names, the `.index()` modifier
/// creates `{table}_{column}_index`, and `ForeignIdBuilder::name` names the
/// key. Each child row points at its own parent row, so one action's effect
/// cannot hide another's. It fails when a shorthand sets the wrong action
/// (a restrict that cascades, a cascade that restricts), `.index()` creates
/// nothing, or the key keeps its default name.
pub async fn action_shorthands(conn: &DatabaseConnection) {
    let backend = conn.get_database_backend();
    let manager = SchemaManager::new(conn);
    let children = [
        "schema_act_restrict_del",
        "schema_act_noaction_del",
        "schema_act_cascade_upd",
        "schema_act_null_upd",
        "schema_act_restrict_upd",
        "schema_act_noaction_upd",
    ];
    let mut all: Vec<&str> = children.to_vec();
    all.push("schema_act_parents");
    drop_tables(conn, &all).await;
    Schema::create(&manager, "schema_act_parents", |t| {
        t.id();
    })
    .await
    .expect("create schema_act_parents");
    type Child = Box<dyn FnOnce(&mut suprnova::schema::Blueprint) + Send>;
    let definitions: Vec<(&str, Child)> = vec![
        (
            children[0],
            Box::new(|t| {
                t.id();
                t.foreign_id("parent_id")
                    .constrained("schema_act_parents")
                    .restrict_on_delete()
                    .index();
            }),
        ),
        (
            children[1],
            Box::new(|t| {
                t.id();
                t.foreign_id("parent_id")
                    .constrained("schema_act_parents")
                    .no_action_on_delete()
                    .name("schema_act_named_fk");
            }),
        ),
        (
            children[2],
            Box::new(|t| {
                t.id();
                t.foreign_id("parent_id")
                    .constrained("schema_act_parents")
                    .cascade_on_update();
            }),
        ),
        (
            children[3],
            Box::new(|t| {
                t.id();
                t.foreign_id("parent_id")
                    .nullable()
                    .constrained("schema_act_parents")
                    .null_on_update();
            }),
        ),
        (
            children[4],
            Box::new(|t| {
                t.id();
                t.foreign_id("parent_id")
                    .constrained("schema_act_parents")
                    .restrict_on_update();
            }),
        ),
        (
            children[5],
            Box::new(|t| {
                t.id();
                t.foreign_id("parent_id")
                    .constrained("schema_act_parents")
                    .no_action_on_update();
            }),
        ),
    ];
    for (name, define) in definitions {
        Schema::create(&manager, name, define)
            .await
            .unwrap_or_else(|e| panic!("create {name}: {e}"));
    }
    run(
        conn,
        "INSERT INTO schema_act_parents (id) VALUES (1), (2), (3), (4), (5), (6)",
    )
    .await
    .expect("insert parents");
    for (position, child) in children.iter().enumerate() {
        run(
            conn,
            &format!(
                "INSERT INTO {child} (id, parent_id) VALUES (1, {})",
                position + 1
            ),
        )
        .await
        .unwrap_or_else(|e| panic!("insert into {child}: {e}"));
    }

    assert!(
        run(conn, "DELETE FROM schema_act_parents WHERE id = 1")
            .await
            .is_err(),
        "restrict_on_delete keeps a referenced parent"
    );
    assert!(
        run(conn, "DELETE FROM schema_act_parents WHERE id = 2")
            .await
            .is_err(),
        "no_action_on_delete keeps a referenced parent"
    );
    run(conn, "UPDATE schema_act_parents SET id = 33 WHERE id = 3")
        .await
        .expect("cascade_on_update lets the key change");
    assert_eq!(
        integers(conn, children[2], "parent_id").await,
        [Some(33)],
        "cascade_on_update copies the new key"
    );
    run(conn, "UPDATE schema_act_parents SET id = 44 WHERE id = 4")
        .await
        .expect("null_on_update lets the key change");
    assert_eq!(
        integers(conn, children[3], "parent_id").await,
        [None],
        "null_on_update clears the reference"
    );
    assert!(
        run(conn, "UPDATE schema_act_parents SET id = 55 WHERE id = 5")
            .await
            .is_err(),
        "restrict_on_update keeps a referenced key"
    );
    assert!(
        run(conn, "UPDATE schema_act_parents SET id = 66 WHERE id = 6")
            .await
            .is_err(),
        "no_action_on_update keeps a referenced key"
    );

    // The catalog tells `restrict` from `no action`, which behave alike here.
    for (child, expected) in [
        (children[0], (None, Some("RESTRICT"))),
        (children[1], (None, Some("NO ACTION"))),
        (children[2], (Some("CASCADE"), None)),
        (children[3], (Some("SET NULL"), None)),
        (children[4], (Some("RESTRICT"), None)),
        (children[5], (Some("NO ACTION"), None)),
    ] {
        let (update, delete) = foreign_key_rules(conn, child).await;
        if let Some(rule) = expected.0 {
            assert_eq!(update, rule, "{child} ON UPDATE");
        }
        if let Some(rule) = expected.1 {
            assert_eq!(delete, rule, "{child} ON DELETE");
        }
    }
    assert!(
        manager
            .has_index(children[0], "schema_act_restrict_del_parent_id_index")
            .await
            .expect("has_index"),
        ".index() creates {{table}}_{{column}}_index"
    );
    if backend != DbBackend::Sqlite {
        Schema::table(&manager, children[1], |t| {
            t.drop_foreign("schema_act_named_fk");
        })
        .await
        .expect("the foreign_id key carries the name .name() gave");
    }

    drop_tables(conn, &all).await;
}

/// The `(ON UPDATE, ON DELETE)` rules of the one foreign key on `table`, as
/// the catalog reports them, upper-cased.
async fn foreign_key_rules(conn: &DatabaseConnection, table: &str) -> (String, String) {
    let backend = conn.get_database_backend();
    let sql = match backend {
        DbBackend::Sqlite => {
            format!("SELECT on_update AS u, on_delete AS d FROM pragma_foreign_key_list('{table}')")
        }
        DbBackend::Postgres => format!(
            "SELECT rc.update_rule::text AS u, rc.delete_rule::text AS d \
             FROM information_schema.referential_constraints rc \
             JOIN information_schema.table_constraints tc \
               ON rc.constraint_name = tc.constraint_name \
              AND rc.constraint_schema = tc.constraint_schema \
             WHERE tc.table_name = '{table}' AND tc.table_schema = current_schema()"
        ),
        _ => format!(
            "SELECT CAST(update_rule AS CHAR) AS u, CAST(delete_rule AS CHAR) AS d \
             FROM information_schema.referential_constraints \
             WHERE table_name = '{table}' AND constraint_schema = DATABASE()"
        ),
    };
    let row = conn
        .query_one_raw(Statement::from_string(backend, sql))
        .await
        .expect("foreign key rules query")
        .expect("the table's foreign key");
    let rule = |column: &str| -> String {
        let value: String = row.try_get("", column).expect("rule");
        value.to_uppercase()
    };
    (rule("u"), rule("d"))
}

/// Reads one nullable integer column of every row, in `id` order.
async fn integers(conn: &DatabaseConnection, table: &str, column: &str) -> Vec<Option<i64>> {
    let rows = conn
        .query_all_raw(Statement::from_string(
            conn.get_database_backend(),
            format!("SELECT {column} AS v FROM {table} ORDER BY id"),
        ))
        .await
        .expect("select integers");
    rows.iter()
        .map(|row| row.try_get("", "v").expect("integer value"))
        .collect()
}

// ---- unsigned keys on every backend --------------------------------------------

/// `unsigned_id` and `unsigned_foreign_id` make a working key on every
/// backend: `BIGINT UNSIGNED` on MySQL, the signed `BIGINT` Laravel falls
/// back to on Postgres, `integer` on SQLite. It fails when the unsigned type
/// leaks off MySQL (sea-query would widen it on Postgres) or the key does
/// not hold.
pub async fn unsigned_keys_everywhere(conn: &DatabaseConnection) {
    unsigned_keys(conn, false).await;
}

/// Turns `unsigned_ids` back off when a case ends, even on a panic.
struct ResetUnsignedIds;

impl Drop for ResetUnsignedIds {
    fn drop(&mut self) {
        suprnova::boot::set_unsigned_ids(false);
    }
}

/// With `unsigned_ids = true` in the application's manifest, `id` and
/// `foreign_id` make the columns `unsigned_id` and `unsigned_foreign_id`
/// make, as Laravel's `id()` and `foreignId()` do: unsigned on MySQL only.
/// It fails when the default does not reach either method. The flag is
/// process-wide, so only the serial MySQL and Postgres runs call it.
pub async fn unsigned_ids_default_everywhere(conn: &DatabaseConnection) {
    suprnova::boot::set_unsigned_ids(true);
    let _reset = ResetUnsignedIds;
    unsigned_keys(conn, true).await;
}

async fn unsigned_keys(conn: &DatabaseConnection, by_default: bool) {
    let backend = conn.get_database_backend();
    let manager = SchemaManager::new(conn);
    drop_tables(conn, &["schema_uk_children", "schema_uk_parents"]).await;
    Schema::create(&manager, "schema_uk_parents", |t| {
        if by_default {
            t.id();
        } else {
            t.unsigned_id();
        }
    })
    .await
    .expect("create schema_uk_parents");
    Schema::create(&manager, "schema_uk_children", |t| {
        if by_default {
            t.id();
            t.foreign_id("parent_id")
        } else {
            t.unsigned_id();
            t.unsigned_foreign_id("parent_id")
        }
        .constrained("schema_uk_parents")
        .cascade_on_delete();
    })
    .await
    .expect("create schema_uk_children");
    for (table, name) in [
        ("schema_uk_parents", "id"),
        ("schema_uk_children", "id"),
        ("schema_uk_children", "parent_id"),
    ] {
        let columns = catalog::columns(conn, table).await;
        let column = columns
            .iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("{table}.{name}"));
        let declared = column.declared.to_lowercase();
        match backend {
            DbBackend::MySql => {
                assert!(declared.starts_with("bigint"), "{table}.{name}: {declared}");
                assert!(declared.contains("unsigned"), "{table}.{name}: {declared}");
            }
            DbBackend::Postgres => assert_eq!(column.family, "bigint", "{table}.{name}"),
            _ => assert_eq!(column.family, "integer", "{table}.{name}"),
        }
    }
    run(conn, "INSERT INTO schema_uk_parents (id) VALUES (1)")
        .await
        .expect("insert parent");
    run(
        conn,
        "INSERT INTO schema_uk_children (id, parent_id) VALUES (1, 1)",
    )
    .await
    .expect("insert child");
    assert!(
        run(
            conn,
            "INSERT INTO schema_uk_children (id, parent_id) VALUES (2, 99)"
        )
        .await
        .is_err(),
        "the key must refuse an orphan row"
    );
    drop_tables(conn, &["schema_uk_children", "schema_uk_parents"]).await;
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

/// An order pointing at [`SchemaUUser`] through an unsigned key. Its key
/// type comes from the `id` field, with no `key_type`.
#[model(
    table = "schema_u_orders",
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
