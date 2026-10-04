//! The schema builder cases against in-memory SQLite, plus the refusals that
//! only SQLite makes. These run with the ordinary suite.

use sea_orm::{Database, DatabaseConnection, DbErr};
use sea_orm_migration::SchemaManager;
use suprnova::schema::Schema;

use super::{cases, laravel_cases};

pub(super) async fn connect_sqlite() -> DatabaseConnection {
    Database::connect("sqlite::memory:?mode=rwc")
        .await
        .expect("in-memory SQLite")
}

fn migration_error(result: Result<(), DbErr>) -> String {
    match result {
        Err(DbErr::Migration(text)) => text,
        other => panic!("expected DbErr::Migration, got {other:?}"),
    }
}

#[tokio::test]
async fn sqlite_every_column_type() {
    cases::every_column_type(&connect_sqlite().await).await;
}

#[tokio::test]
async fn sqlite_model_round_trip() {
    cases::model_round_trip(&connect_sqlite().await).await;
}

#[cfg(feature = "testing")]
#[tokio::test]
async fn sqlite_native_timestamps_round_trip() {
    cases::native_timestamps_round_trip(&connect_sqlite().await).await;
}

#[tokio::test]
async fn sqlite_foreign_key_cascade() {
    cases::foreign_key_cascade(&connect_sqlite().await).await;
}

#[tokio::test]
async fn sqlite_unique_index_refuses_duplicate() {
    cases::unique_index_refuses_duplicate(&connect_sqlite().await).await;
}

#[tokio::test]
async fn sqlite_alter_indexes() {
    cases::alter_indexes(&connect_sqlite().await).await;
}

#[tokio::test]
async fn sqlite_alter_columns() {
    cases::alter_columns(&connect_sqlite().await).await;
}

#[tokio::test]
async fn sqlite_tables_drop_and_rename() {
    cases::tables_drop_and_rename(&connect_sqlite().await).await;
}

#[tokio::test]
async fn sqlite_misuse_is_refused_before_any_statement() {
    cases::misuse_is_refused_before_any_statement(&connect_sqlite().await).await;
}

#[tokio::test]
async fn sqlite_migrator_runs_both_styles() {
    cases::migrator_runs_both_styles(&connect_sqlite().await).await;
}

#[tokio::test]
async fn sqlite_undeclared_index_column_is_refused() {
    cases::undeclared_index_column_is_refused(&connect_sqlite().await).await;
}

#[tokio::test]
async fn sqlite_duplicate_added_column_is_refused() {
    cases::duplicate_added_column_is_refused(&connect_sqlite().await).await;
}

#[tokio::test]
async fn sqlite_quoted_string_default_round_trips() {
    cases::quoted_string_default_round_trips(&connect_sqlite().await).await;
}

/// Adding a foreign key to an existing table is refused with an error that
/// names the operation and the table, and the columns recorded before the
/// key are not added either, because the whole call is refused before its
/// first statement. It fails when the builder reaches sea-query (which panics
/// on this statement) or runs the earlier alterations first.
#[tokio::test]
async fn sqlite_refuses_adding_a_foreign_key_and_runs_nothing() {
    let conn = connect_sqlite().await;
    let manager = SchemaManager::new(&conn);
    Schema::create(&manager, "schema_lite_child", |t| {
        t.id();
        t.string("name");
    })
    .await
    .expect("create schema_lite_child");

    let text = migration_error(
        Schema::table(&manager, "schema_lite_child", |t| {
            t.string("extra").nullable();
            t.foreign_id("parent_id").constrained("schema_lite_parent");
        })
        .await,
    );
    assert!(text.contains("schema_lite_child"), "{text}");
    assert!(text.contains("foreign key"), "{text}");
    assert!(text.contains("SQLite"), "{text}");
    assert!(text.contains("Schema::create"), "{text}");
    assert!(text.contains("SchemaManager"), "{text}");

    for column in ["extra", "parent_id"] {
        assert!(
            !Schema::has_column(&manager, "schema_lite_child", column)
                .await
                .expect("has_column"),
            "the refused call must not add {column}"
        );
    }
}

/// Dropping a foreign key is refused the same way, and the alteration
/// recorded before it does not run. It fails when the refusal is a panic or
/// comes after the first statement.
#[tokio::test]
async fn sqlite_refuses_dropping_a_foreign_key_and_runs_nothing() {
    let conn = connect_sqlite().await;
    let manager = SchemaManager::new(&conn);
    Schema::create(&manager, "schema_lite_parent", |t| {
        t.id();
    })
    .await
    .expect("create schema_lite_parent");
    Schema::create(&manager, "schema_lite_child", |t| {
        t.id();
        t.foreign_id("parent_id").constrained("schema_lite_parent");
    })
    .await
    .expect("create schema_lite_child");

    let text = migration_error(
        Schema::table(&manager, "schema_lite_child", |t| {
            t.string("extra").nullable();
            t.drop_foreign("schema_lite_child_parent_id_foreign");
        })
        .await,
    );
    assert!(text.contains("schema_lite_child"), "{text}");
    assert!(text.contains("drop"), "{text}");
    assert!(text.contains("SQLite"), "{text}");
    assert!(
        !Schema::has_column(&manager, "schema_lite_child", "extra")
            .await
            .expect("has_column"),
        "the refused call must not add extra"
    );
}

/// SQLite cannot add a primary key column, nor a `NOT NULL` column without a
/// default, to an existing table. Both are refused with the table and column
/// named, and nothing is added. It fails when the refusal is left to SQLite
/// after earlier statements ran.
#[tokio::test]
async fn sqlite_refuses_columns_it_cannot_add_and_runs_nothing() {
    let conn = connect_sqlite().await;
    let manager = SchemaManager::new(&conn);
    Schema::create(&manager, "schema_lite_notes", |t| {
        t.string("title");
    })
    .await
    .expect("create schema_lite_notes");

    let text = migration_error(
        Schema::table(&manager, "schema_lite_notes", |t| {
            t.string("extra").nullable();
            t.id();
        })
        .await,
    );
    assert!(text.contains("schema_lite_notes"), "{text}");
    assert!(text.contains("`id`"), "{text}");

    let text = migration_error(
        Schema::table(&manager, "schema_lite_notes", |t| {
            t.string("extra").nullable();
            t.string("required");
        })
        .await,
    );
    assert!(text.contains("schema_lite_notes"), "{text}");
    assert!(text.contains("`required`"), "{text}");

    assert!(
        !Schema::has_column(&manager, "schema_lite_notes", "extra")
            .await
            .expect("has_column"),
        "a refused call must not add extra"
    );
}

#[tokio::test]
async fn sqlite_a_name_longer_than_the_limit_is_refused() {
    cases::a_name_longer_than_the_limit_is_refused(&connect_sqlite().await).await;
}

#[tokio::test]
async fn sqlite_laravel_column_types() {
    laravel_cases::laravel_column_types(&connect_sqlite().await).await;
}

#[tokio::test]
async fn sqlite_primary_keys() {
    laravel_cases::primary_keys(&connect_sqlite().await).await;
}

#[tokio::test]
async fn sqlite_foreign_on_a_declared_column() {
    laravel_cases::foreign_on_a_declared_column(&connect_sqlite().await).await;
}

#[tokio::test]
async fn sqlite_laravel_misuse_is_refused() {
    laravel_cases::laravel_misuse_is_refused(&connect_sqlite().await).await;
}

#[tokio::test]
async fn sqlite_alter_laravel_refusals() {
    laravel_cases::sqlite_alter_laravel_refusals(&connect_sqlite().await).await;
}

#[tokio::test]
async fn sqlite_laravel_alter_misuse_is_refused() {
    laravel_cases::laravel_alter_misuse_is_refused(&connect_sqlite().await).await;
}

#[tokio::test]
async fn sqlite_action_shorthands() {
    laravel_cases::action_shorthands(&connect_sqlite().await).await;
}

#[tokio::test]
async fn sqlite_unsigned_keys_everywhere() {
    laravel_cases::unsigned_keys_everywhere(&connect_sqlite().await).await;
}
