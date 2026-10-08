//! `Exists` against a real Postgres, the one backend that numbers its
//! placeholders and refuses to compare a text parameter with an integer
//! column.
//!
//! Run against a disposable database:
//!
//! ```text
//! PG_TEST_URL=postgres://... \
//!   cargo test -p suprnova --test validation postgres:: -- --ignored --test-threads=1
//! ```

use std::time::Duration;

use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection};
use serial_test::serial;
use suprnova::testing::TestContainer;
use suprnova::{AsyncRule, DbConnection, Exists, ValidationErrors};

async fn connect_postgres() -> DatabaseConnection {
    let url = std::env::var("PG_TEST_URL").expect("set PG_TEST_URL to a disposable Postgres");
    let mut options = ConnectOptions::new(url);
    options
        .max_connections(2)
        .min_connections(0)
        .connect_timeout(Duration::from_secs(5))
        .acquire_timeout(Duration::from_secs(5));
    Database::connect(options)
        .await
        .expect("Postgres test database must be reachable")
}

/// Team 7 owns tags 1 and 2; team 8 owns tag 3. The key columns are
/// `bigint`, as `id()` and `foreign_id()` make them.
async fn create_tags(conn: &DatabaseConnection) {
    for sql in [
        "DROP TABLE IF EXISTS validation_tags",
        "CREATE TABLE validation_tags (id BIGINT PRIMARY KEY, team_id BIGINT NOT NULL, slug TEXT NOT NULL)",
        "INSERT INTO validation_tags (id, team_id, slug) VALUES (1, 7, 'rust'), (2, 7, 'laravel'), (3, 8, 'php')",
    ] {
        conn.execute_unprepared(sql).await.expect(sql);
    }
}

fn sorted_keys(errs: &ValidationErrors) -> Vec<String> {
    let mut keys: Vec<String> = errs.errors.keys().cloned().collect();
    keys.sort();
    keys
}

#[tokio::test]
#[serial]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_exists_binds_typed_values_and_numbers_its_placeholders() {
    let conn = connect_postgres().await;
    create_tags(&conn).await;
    let _guard = TestContainer::fake();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));

    let mut errs = ValidationErrors::new();
    let id = Exists::new("validation_tags", "id");
    id.check_value(1i64, &mut errs, "known_id").await;
    id.check_value(9i64, &mut errs, "unknown_id").await;

    // Three placeholders, `$1` for the value and `$2`, `$3` for the scopes.
    let scoped = |team: i64| {
        Exists::new("validation_tags", "slug")
            .where_eq("team_id", team)
            .where_eq("id", 2i64)
    };
    scoped(7).check_async("laravel", &mut errs, "own_tag").await;
    scoped(8)
        .check_async("laravel", &mut errs, "foreign_tag")
        .await;

    Exists::new("validation_tags", "id")
        .where_eq("team_id", 7i64)
        .check_each(&[1i64, 3, 1], &mut errs, "tag_ids")
        .await;

    assert_eq!(
        sorted_keys(&errs),
        ["foreign_tag", "tag_ids.1", "unknown_id"]
    );

    // A text parameter against the `bigint` column: Postgres refuses the
    // comparison, and the field fails without the driver's words.
    let err = Exists::new("validation_tags", "id")
        .passes("1")
        .await
        .unwrap_err();
    assert_eq!(err.key, "validation-unchecked");
    assert!(!err.fallback.contains("bigint"), "{}", err.fallback);

    conn.execute_unprepared("DROP TABLE validation_tags")
        .await
        .expect("drop validation_tags");
}
