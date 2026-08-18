//! MariaDB/MySQL regression coverage for nullable Eloquent pivot writes.
//!
//! This target proves that both many-to-many relation writers can mix
//! explicit SQL `NULL` values for non-text columns with ordinary bound values
//! and framework-generated timestamps. The pivot timestamp columns have no
//! database defaults, so successful inserts also prove that `attach_with`
//! supplied typed `created_at` and `updated_at` values.
//!
//! The test is ignored during the normal suite. Run it against a
//! disposable database:
//!
//! ```text
//! MYSQL_TEST_URL=mysql://... \
//!   cargo test -p suprnova --test eloquent_mass_write_mysql -- \
//!   --ignored --test-threads=1
//! ```
//!
//! Explicit execution without `MYSQL_TEST_URL` fails immediately; it never
//! reports a silent pass.

use sea_orm::{ConnectOptions, ConnectionTrait, Database, Statement};
use std::time::Duration;
use suprnova::testing::TestContainer;
use suprnova::{DbConnection, Model, attrs, model};

#[model(table = "suprnova_mysql_pivot_owners", relations = {
    roles: BelongsToMany<MysqlPivotRole, MysqlRoleOwner> {
        pivot_table = "suprnova_mysql_role_owners",
        pivot_foreign_key = "owner_ref_id",
        pivot_related_key = "role_ref_id",
        with_timestamps,
    },
})]
pub struct MysqlPivotOwner {
    pub id: i64,
    pub name: String,
}

#[model(table = "suprnova_mysql_pivot_roles")]
pub struct MysqlPivotRole {
    pub id: i64,
    pub name: String,
}

#[model(table = "suprnova_mysql_role_owners", primary_key = "id")]
pub struct MysqlRoleOwner {
    pub id: i64,
    pub owner_ref_id: i64,
    pub role_ref_id: i64,
    pub nullable_bigint: Option<i64>,
    pub nullable_boolean: Option<bool>,
    pub marker: String,
}

#[model(
    table = "suprnova_mysql_pivot_posts",
    morph_type = "mysql_pivot_post",
    relations = {
        tags: MorphToMany<MysqlPivotTag, MysqlTaggable> {
            name = "subject",
            pivot_table = "suprnova_mysql_taggables",
            pivot_related_key = "tag_ref_id",
            with_timestamps,
        },
    }
)]
pub struct MysqlPivotPost {
    pub id: i64,
    pub title: String,
}

#[model(table = "suprnova_mysql_pivot_tags")]
pub struct MysqlPivotTag {
    pub id: i64,
    pub name: String,
}

#[model(table = "suprnova_mysql_taggables", primary_key = "id")]
pub struct MysqlTaggable {
    pub id: i64,
    pub tag_ref_id: i64,
    pub subject_id: i64,
    pub subject_type: String,
    pub nullable_bigint: Option<i64>,
    pub nullable_boolean: Option<bool>,
    pub marker: String,
}

async fn connect_mysql() -> sea_orm::DatabaseConnection {
    let url = std::env::var("MYSQL_TEST_URL")
        .expect("set MYSQL_TEST_URL to a disposable MariaDB/MySQL database");
    let mut options = ConnectOptions::new(url);
    options
        .max_connections(2)
        .min_connections(0)
        .connect_timeout(Duration::from_secs(2))
        .acquire_timeout(Duration::from_secs(2));
    Database::connect(options)
        .await
        .expect("MariaDB/MySQL test database must be reachable")
}

#[tokio::test]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_relation_pivots_accept_null_non_text_extras_and_typed_timestamps() {
    let conn = connect_mysql().await;
    let backend = conn.get_database_backend();

    for sql in [
        "DROP TABLE IF EXISTS suprnova_mysql_taggables",
        "DROP TABLE IF EXISTS suprnova_mysql_pivot_tags",
        "DROP TABLE IF EXISTS suprnova_mysql_pivot_posts",
        "DROP TABLE IF EXISTS suprnova_mysql_role_owners",
        "DROP TABLE IF EXISTS suprnova_mysql_pivot_roles",
        "DROP TABLE IF EXISTS suprnova_mysql_pivot_owners",
        "CREATE TABLE suprnova_mysql_pivot_owners (\
             id BIGINT AUTO_INCREMENT PRIMARY KEY,\
             name VARCHAR(255) NOT NULL\
         )",
        "CREATE TABLE suprnova_mysql_pivot_roles (\
             id BIGINT AUTO_INCREMENT PRIMARY KEY,\
             name VARCHAR(255) NOT NULL\
         )",
        "CREATE TABLE suprnova_mysql_role_owners (\
             id BIGINT AUTO_INCREMENT PRIMARY KEY,\
             owner_ref_id BIGINT NOT NULL,\
             role_ref_id BIGINT NOT NULL,\
             nullable_bigint BIGINT NULL,\
             nullable_boolean BOOLEAN NULL,\
             nullable_datetime DATETIME(6) NULL,\
             marker VARCHAR(64) NOT NULL,\
             created_at DATETIME(6) NOT NULL,\
             updated_at DATETIME(6) NOT NULL,\
             UNIQUE KEY uq_suprnova_mysql_role_owner (owner_ref_id, role_ref_id)\
         )",
        "CREATE TABLE suprnova_mysql_pivot_posts (\
             id BIGINT AUTO_INCREMENT PRIMARY KEY,\
             title VARCHAR(255) NOT NULL\
         )",
        "CREATE TABLE suprnova_mysql_pivot_tags (\
             id BIGINT AUTO_INCREMENT PRIMARY KEY,\
             name VARCHAR(255) NOT NULL\
         )",
        "CREATE TABLE suprnova_mysql_taggables (\
             id BIGINT AUTO_INCREMENT PRIMARY KEY,\
             tag_ref_id BIGINT NOT NULL,\
             subject_id BIGINT NOT NULL,\
             subject_type VARCHAR(255) NOT NULL,\
             nullable_bigint BIGINT NULL,\
             nullable_boolean BOOLEAN NULL,\
             nullable_datetime DATETIME(6) NULL,\
             marker VARCHAR(64) NOT NULL,\
             created_at DATETIME(6) NOT NULL,\
             updated_at DATETIME(6) NOT NULL,\
             UNIQUE KEY uq_suprnova_mysql_taggable (tag_ref_id, subject_id, subject_type)\
         )",
    ] {
        conn.execute(Statement::from_string(backend, sql.to_owned()))
            .await
            .unwrap_or_else(|error| panic!("fixture setup failed on {sql:?}: {error}"));
    }

    let _guard = TestContainer::fake();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));

    let owner = MysqlPivotOwner::create(attrs! { name: "pivot-owner" })
        .await
        .expect("create belongs-to-many owner");
    let role = MysqlPivotRole::create(attrs! { name: "pivot-role" })
        .await
        .expect("create belongs-to-many role");
    owner
        .roles()
        .attach_with(
            role.id,
            attrs! {
                nullable_bigint: Option::<i64>::None,
                nullable_boolean: Option::<bool>::None,
                nullable_datetime: Option::<chrono::NaiveDateTime>::None,
                marker: "belongs-to-many",
            },
        )
        .await
        .expect("BelongsToMany::attach_with must accept explicit non-text NULL extras");

    let role_owner = MysqlRoleOwner::query()
        .filter("owner_ref_id", owner.id)
        .filter("role_ref_id", role.id)
        .first()
        .await
        .expect("read belongs-to-many pivot")
        .expect("belongs-to-many pivot must exist");
    assert_eq!(role_owner.owner_ref_id, owner.id);
    assert_eq!(role_owner.role_ref_id, role.id);
    assert_eq!(role_owner.nullable_bigint, None);
    assert_eq!(role_owner.nullable_boolean, None);
    assert_eq!(role_owner.marker, "belongs-to-many");
    assert_mysql_pivot_values(
        &conn,
        backend,
        "suprnova_mysql_role_owners",
        "owner_ref_id",
        owner.id,
        "belongs-to-many",
    )
    .await;

    let post = MysqlPivotPost::create(attrs! { title: "pivot-post" })
        .await
        .expect("create morph parent");
    let tag = MysqlPivotTag::create(attrs! { name: "pivot-tag" })
        .await
        .expect("create morph related row");
    post.tags()
        .attach_with(
            tag.id,
            attrs! {
                nullable_bigint: Option::<i64>::None,
                nullable_boolean: Option::<bool>::None,
                nullable_datetime: Option::<chrono::NaiveDateTime>::None,
                marker: "morph-to-many",
            },
        )
        .await
        .expect("MorphToMany::attach_with must accept explicit non-text NULL extras");

    let taggable = MysqlTaggable::query()
        .filter("tag_ref_id", tag.id)
        .filter("subject_id", post.id)
        .filter("subject_type", "mysql_pivot_post")
        .first()
        .await
        .expect("read morph-to-many pivot")
        .expect("morph-to-many pivot must exist");
    assert_eq!(taggable.tag_ref_id, tag.id);
    assert_eq!(taggable.subject_id, post.id);
    assert_eq!(taggable.subject_type, "mysql_pivot_post");
    assert_eq!(taggable.nullable_bigint, None);
    assert_eq!(taggable.nullable_boolean, None);
    assert_eq!(taggable.marker, "morph-to-many");
    assert_mysql_pivot_values(
        &conn,
        backend,
        "suprnova_mysql_taggables",
        "subject_id",
        post.id,
        "morph-to-many",
    )
    .await;

    for sql in [
        "DROP TABLE suprnova_mysql_taggables",
        "DROP TABLE suprnova_mysql_pivot_tags",
        "DROP TABLE suprnova_mysql_pivot_posts",
        "DROP TABLE suprnova_mysql_role_owners",
        "DROP TABLE suprnova_mysql_pivot_roles",
        "DROP TABLE suprnova_mysql_pivot_owners",
    ] {
        conn.execute(Statement::from_string(backend, sql.to_owned()))
            .await
            .unwrap_or_else(|error| panic!("fixture cleanup failed on {sql:?}: {error}"));
    }
}

async fn assert_mysql_pivot_values(
    conn: &sea_orm::DatabaseConnection,
    backend: sea_orm::DbBackend,
    table: &str,
    id_column: &str,
    id: i64,
    expected_marker: &str,
) {
    let sql = format!(
        "SELECT nullable_bigint IS NULL AS bigint_is_null, \
         nullable_boolean IS NULL AS boolean_is_null, \
         nullable_datetime IS NULL AS datetime_is_null, \
         marker, \
         created_at IS NOT NULL AS created_present, \
         updated_at IS NOT NULL AS updated_present, \
         created_at = updated_at AS timestamps_match \
         FROM {table} WHERE {id_column} = ?"
    );
    let row = conn
        .query_one(Statement::from_sql_and_values(backend, sql, [id.into()]))
        .await
        .expect("query MySQL pivot values")
        .expect("MySQL pivot row must exist");

    for column in [
        "bigint_is_null",
        "boolean_is_null",
        "datetime_is_null",
        "created_present",
        "updated_present",
        "timestamps_match",
    ] {
        assert!(
            row.try_get::<bool>("", column)
                .unwrap_or_else(|error| panic!("decode {column}: {error}")),
            "expected {column} for {table}",
        );
    }
    assert_eq!(
        row.try_get::<String>("", "marker")
            .expect("decode pivot marker"),
        expected_marker,
    );
}
