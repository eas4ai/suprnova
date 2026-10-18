//! PostgreSQL regression coverage for nullable Eloquent mass writes.
//!
//! PostgreSQL cannot infer a non-text target type from a text-typed bound
//! null. These checks use real nullable `BIGINT`, `BOOLEAN`, and `TIMESTAMPTZ`
//! columns so regressions in typed Eloquent or model-less `DB::table` writes
//! fail at the database boundary instead of passing under SQLite.
//!
//! This target is ignored during the normal test suite. Run it with a
//! disposable database:
//!
//! ```text
//! PG_TEST_URL=postgres://... \
//!   cargo test -p suprnova --test eloquent_mass_write_postgres -- \
//!   --ignored --test-threads=1
//! ```
//!
//! Explicit execution without `PG_TEST_URL` fails immediately; it never
//! reports a silent pass.

use sea_orm::{ConnectOptions, ConnectionTrait, Database, Statement};
use std::time::Duration;
use suprnova::testing::TestContainer;
use suprnova::{DB, DbConnection, Model, attrs, model};

#[model(table = "suprnova_mass_write_probe", timestamps = false)]
pub struct MassWriteProbe {
    pub id: i64,
    pub probe_key: String,
    pub nullable_bigint: Option<i64>,
    pub nullable_boolean: Option<bool>,
}

#[model(table = "suprnova_mass_write_owners", relations = {
    roles: BelongsToMany<MassWriteRole, MassWriteRoleOwner> {
        pivot_table = "suprnova_mass_write_role_owners",
        pivot_foreign_key = "owner_ref_id",
        pivot_related_key = "role_ref_id",
        with_timestamps,
    },
})]
pub struct MassWriteOwner {
    pub id: i64,
    pub name: String,
}

#[model(table = "suprnova_mass_write_roles")]
pub struct MassWriteRole {
    pub id: i64,
    pub name: String,
}

#[model(table = "suprnova_mass_write_role_owners", primary_key = "id")]
pub struct MassWriteRoleOwner {
    pub id: i64,
    pub owner_ref_id: i64,
    pub role_ref_id: i64,
    pub nullable_bigint: Option<i64>,
    pub nullable_boolean: Option<bool>,
    pub marker: String,
}

#[model(
    table = "suprnova_mass_write_posts",
    morph_type = "mass_write_post",
    relations = {
        tags: MorphToMany<MassWriteTag, MassWriteTaggable> {
            name = "subject",
            pivot_table = "suprnova_mass_write_taggables",
            pivot_related_key = "tag_ref_id",
            with_timestamps,
        },
    }
)]
pub struct MassWritePost {
    pub id: i64,
    pub title: String,
}

#[model(table = "suprnova_mass_write_tags")]
pub struct MassWriteTag {
    pub id: i64,
    pub name: String,
}

#[model(table = "suprnova_mass_write_taggables", primary_key = "id")]
pub struct MassWriteTaggable {
    pub id: i64,
    pub tag_ref_id: i64,
    pub subject_id: i64,
    pub subject_type: String,
    pub nullable_bigint: Option<i64>,
    pub nullable_boolean: Option<bool>,
    pub marker: String,
}

async fn connect_postgres() -> sea_orm::DatabaseConnection {
    let url = std::env::var("PG_TEST_URL").expect("set PG_TEST_URL to a disposable Postgres");
    let mut options = ConnectOptions::new(url);
    options
        .max_connections(2)
        .min_connections(0)
        .connect_timeout(Duration::from_secs(2))
        .acquire_timeout(Duration::from_secs(2));
    Database::connect(options)
        .await
        .expect("Postgres test database must be reachable")
}

#[tokio::test]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_attribute_writes_preserve_nullable_non_text_types() {
    let conn = connect_postgres().await;
    let backend = conn.get_database_backend();
    for sql in [
        "DROP TABLE IF EXISTS suprnova_mass_write_taggables",
        "DROP TABLE IF EXISTS suprnova_mass_write_tags",
        "DROP TABLE IF EXISTS suprnova_mass_write_posts",
        "DROP TABLE IF EXISTS suprnova_mass_write_role_owners",
        "DROP TABLE IF EXISTS suprnova_mass_write_roles",
        "DROP TABLE IF EXISTS suprnova_mass_write_owners",
        "DROP TABLE IF EXISTS suprnova_mass_write_probe",
        "CREATE TABLE suprnova_mass_write_probe (\
             id BIGINT PRIMARY KEY,\
             probe_key TEXT NOT NULL UNIQUE,\
             nullable_bigint BIGINT NULL,\
             nullable_boolean BOOLEAN NULL,\
             nullable_timestamp TIMESTAMPTZ NULL\
         )",
        "INSERT INTO suprnova_mass_write_probe \
             (id, probe_key, nullable_bigint, nullable_boolean, nullable_timestamp) VALUES \
             (1, 'update-all', 11, TRUE, '2026-08-13T12:00:00Z'),\
             (2, 'upsert', 22, FALSE, '2026-08-13T12:00:00Z'),\
             (3, 'control', 33, TRUE, '2026-08-13T12:00:00Z'),\
             (4, 'raw-update', 44, FALSE, '2026-08-13T12:00:00Z')",
        "CREATE TABLE suprnova_mass_write_owners (\
             id BIGSERIAL PRIMARY KEY,\
             name TEXT NOT NULL\
         )",
        "CREATE TABLE suprnova_mass_write_roles (\
             id BIGSERIAL PRIMARY KEY,\
             name TEXT NOT NULL\
         )",
        "CREATE TABLE suprnova_mass_write_role_owners (\
             id BIGSERIAL PRIMARY KEY,\
             owner_ref_id BIGINT NOT NULL,\
             role_ref_id BIGINT NOT NULL,\
             nullable_bigint BIGINT NULL,\
             nullable_boolean BOOLEAN NULL,\
             nullable_timestamp TIMESTAMPTZ NULL,\
             marker TEXT NOT NULL,\
             created_at TIMESTAMPTZ NOT NULL,\
             updated_at TIMESTAMPTZ NOT NULL,\
             UNIQUE (owner_ref_id, role_ref_id)\
         )",
        "CREATE TABLE suprnova_mass_write_posts (\
             id BIGSERIAL PRIMARY KEY,\
             title TEXT NOT NULL\
         )",
        "CREATE TABLE suprnova_mass_write_tags (\
             id BIGSERIAL PRIMARY KEY,\
             name TEXT NOT NULL\
         )",
        "CREATE TABLE suprnova_mass_write_taggables (\
             id BIGSERIAL PRIMARY KEY,\
             tag_ref_id BIGINT NOT NULL,\
             subject_id BIGINT NOT NULL,\
             subject_type TEXT NOT NULL,\
             nullable_bigint BIGINT NULL,\
             nullable_boolean BOOLEAN NULL,\
             nullable_timestamp TIMESTAMPTZ NULL,\
             marker TEXT NOT NULL,\
             created_at TIMESTAMPTZ NOT NULL,\
             updated_at TIMESTAMPTZ NOT NULL,\
             UNIQUE (tag_ref_id, subject_id, subject_type)\
         )",
    ] {
        conn.execute_raw(Statement::from_string(backend, sql.to_owned()))
            .await
            .unwrap_or_else(|error| panic!("fixture setup failed on {sql:?}: {error}"));
    }

    let _guard = TestContainer::fake();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));

    let updated = MassWriteProbe::query()
        .filter("probe_key", "update-all")
        .update_all(attrs! {
            nullable_bigint: Option::<i64>::None,
            nullable_boolean: Option::<bool>::None,
            nullable_timestamp: Option::<chrono::DateTime<chrono::Utc>>::None,
        })
        .await
        .expect("update_all must accept NULL for nullable non-text columns");
    assert_eq!(updated, 1, "update_all must affect only its filtered row");

    let updated_row = MassWriteProbe::query()
        .filter("probe_key", "update-all")
        .first()
        .await
        .expect("read update_all row")
        .expect("update_all row must exist");
    assert_eq!(updated_row.nullable_bigint, None);
    assert_eq!(updated_row.nullable_boolean, None);
    assert_probe_timestamp_null(&conn, backend, "update-all", true).await;

    let upserted = MassWriteProbe::query()
        .upsert(
            vec![
                attrs! {
                    id: 2_i64,
                    probe_key: "upsert",
                    nullable_bigint: Option::<i64>::None,
                    nullable_boolean: Option::<bool>::None,
                    nullable_timestamp: Option::<chrono::DateTime<chrono::Utc>>::None,
                },
                attrs! {
                    id: 6_i64,
                    probe_key: "upsert-new",
                    nullable_bigint: Some(66_i64),
                    nullable_boolean: Option::<bool>::None,
                    nullable_timestamp: Option::<chrono::DateTime<chrono::Utc>>::None,
                },
            ],
            vec!["probe_key"],
            Some(vec![
                "nullable_bigint",
                "nullable_boolean",
                "nullable_timestamp",
            ]),
        )
        .await
        .expect("upsert must accept NULL for nullable non-text columns");
    assert_eq!(
        upserted, 2,
        "the conflict update and insert must affect two rows"
    );

    let upserted_row = MassWriteProbe::query()
        .filter("probe_key", "upsert")
        .first()
        .await
        .expect("read upsert row")
        .expect("upsert row must exist");
    assert_eq!(upserted_row.nullable_bigint, None);
    assert_eq!(upserted_row.nullable_boolean, None);
    assert_probe_timestamp_null(&conn, backend, "upsert", true).await;

    let inserted_upsert_row = MassWriteProbe::query()
        .filter("probe_key", "upsert-new")
        .first()
        .await
        .expect("read inserted upsert row")
        .expect("inserted upsert row must exist");
    assert_eq!(inserted_upsert_row.nullable_bigint, Some(66));
    assert_eq!(inserted_upsert_row.nullable_boolean, None);
    assert_probe_timestamp_null(&conn, backend, "upsert-new", true).await;

    let inserted_id = DB::table("suprnova_mass_write_probe")
        .insert(attrs! {
            id: 5_i64,
            probe_key: "raw-insert",
            nullable_bigint: Option::<i64>::None,
            nullable_boolean: Option::<bool>::None,
            nullable_timestamp: Option::<chrono::DateTime<chrono::Utc>>::None,
        })
        .await
        .expect("DB::table insert must preserve nullable non-text column types");
    assert_eq!(inserted_id, 5);

    let raw_updated = DB::table("suprnova_mass_write_probe")
        .filter("probe_key", "raw-update")
        .update_all(attrs! {
            nullable_bigint: Option::<i64>::None,
            nullable_boolean: Option::<bool>::None,
            nullable_timestamp: Option::<chrono::DateTime<chrono::Utc>>::None,
        })
        .await
        .expect("DB::table update must preserve nullable non-text column types");
    assert_eq!(raw_updated, 1);

    for key in ["raw-insert", "raw-update"] {
        let row = MassWriteProbe::query()
            .filter("probe_key", key)
            .first()
            .await
            .expect("read model-less write row")
            .expect("model-less write row must exist");
        assert_eq!(row.nullable_bigint, None, "{key} bigint");
        assert_eq!(row.nullable_boolean, None, "{key} boolean");
        assert_probe_timestamp_null(&conn, backend, key, true).await;
    }

    let control_row = MassWriteProbe::query()
        .filter("probe_key", "control")
        .first()
        .await
        .expect("read control row")
        .expect("control row must exist");
    assert_eq!(control_row.nullable_bigint, Some(33));
    assert_eq!(control_row.nullable_boolean, Some(true));
    assert_probe_timestamp_null(&conn, backend, "control", false).await;

    let owner = MassWriteOwner::create(attrs! { name: "pivot-owner" })
        .await
        .expect("create belongs-to-many owner");
    let role = MassWriteRole::create(attrs! { name: "pivot-role" })
        .await
        .expect("create belongs-to-many role");
    owner
        .roles()
        .attach_with(
            role.id,
            attrs! {
                nullable_bigint: Option::<i64>::None,
                nullable_boolean: Option::<bool>::None,
                nullable_timestamp: Option::<chrono::DateTime<chrono::Utc>>::None,
                marker: "belongs-to-many",
            },
        )
        .await
        .expect("BelongsToMany::attach_with must accept explicit non-text NULL extras");

    let role_owner = MassWriteRoleOwner::query()
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
    assert_pivot_timestamps(
        &conn,
        backend,
        "suprnova_mass_write_role_owners",
        "owner_ref_id",
        owner.id,
    )
    .await;

    let post = MassWritePost::create(attrs! { title: "pivot-post" })
        .await
        .expect("create morph parent");
    let tag = MassWriteTag::create(attrs! { name: "pivot-tag" })
        .await
        .expect("create morph related row");
    post.tags()
        .attach_with(
            tag.id,
            attrs! {
                nullable_bigint: Option::<i64>::None,
                nullable_boolean: Option::<bool>::None,
                nullable_timestamp: Option::<chrono::DateTime<chrono::Utc>>::None,
                marker: "morph-to-many",
            },
        )
        .await
        .expect("MorphToMany::attach_with must accept explicit non-text NULL extras");

    let taggable = MassWriteTaggable::query()
        .filter("tag_ref_id", tag.id)
        .filter("subject_id", post.id)
        .filter("subject_type", "mass_write_post")
        .first()
        .await
        .expect("read morph-to-many pivot")
        .expect("morph-to-many pivot must exist");
    assert_eq!(taggable.tag_ref_id, tag.id);
    assert_eq!(taggable.subject_id, post.id);
    assert_eq!(taggable.subject_type, "mass_write_post");
    assert_eq!(taggable.nullable_bigint, None);
    assert_eq!(taggable.nullable_boolean, None);
    assert_eq!(taggable.marker, "morph-to-many");
    assert_pivot_timestamps(
        &conn,
        backend,
        "suprnova_mass_write_taggables",
        "subject_id",
        post.id,
    )
    .await;

    for sql in [
        "DROP TABLE suprnova_mass_write_taggables",
        "DROP TABLE suprnova_mass_write_tags",
        "DROP TABLE suprnova_mass_write_posts",
        "DROP TABLE suprnova_mass_write_role_owners",
        "DROP TABLE suprnova_mass_write_roles",
        "DROP TABLE suprnova_mass_write_owners",
        "DROP TABLE suprnova_mass_write_probe",
    ] {
        conn.execute_raw(Statement::from_string(backend, sql.to_owned()))
            .await
            .unwrap_or_else(|error| panic!("fixture cleanup failed on {sql:?}: {error}"));
    }
}

async fn assert_probe_timestamp_null(
    conn: &sea_orm::DatabaseConnection,
    backend: sea_orm::DbBackend,
    probe_key: &str,
    expected: bool,
) {
    let row = conn
        .query_one_raw(Statement::from_sql_and_values(
            backend,
            "SELECT nullable_timestamp IS NULL AS is_null \
     FROM suprnova_mass_write_probe WHERE probe_key = $1",
            [probe_key.to_owned().into()],
        ))
        .await
        .expect("query probe timestamp")
        .expect("probe row must exist");
    assert_eq!(
        row.try_get::<bool>("", "is_null")
            .expect("decode timestamp null state"),
        expected,
        "timestamp null state for {probe_key}",
    );
}

async fn assert_pivot_timestamps(
    conn: &sea_orm::DatabaseConnection,
    backend: sea_orm::DbBackend,
    table: &str,
    id_column: &str,
    id: i64,
) {
    let sql = format!(
        "SELECT nullable_timestamp IS NULL AS nullable_is_null, \
         created_at IS NOT NULL AS created_present, \
         updated_at IS NOT NULL AS updated_present, \
         created_at = updated_at AS timestamps_match \
         FROM {table} WHERE {id_column} = $1"
    );
    let row = conn
        .query_one_raw(Statement::from_sql_and_values(backend, sql, [id.into()]))
        .await
        .expect("query pivot timestamp state")
        .expect("pivot row must exist");
    for column in [
        "nullable_is_null",
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
}
