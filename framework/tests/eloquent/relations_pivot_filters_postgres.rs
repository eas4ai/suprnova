//! PostgreSQL regression coverage for the `where_pivot` family.
//!
//! The pivot filters splice into hand-built pivot SQL at six sites -
//! `BelongsToMany::get` / `count`, `MorphToMany::get` / `count`, and
//! `MorphedByMany::get` / `count`. Each of those seeds the placeholder
//! counter with the number of binds its own prefix already pushed (one
//! for the plain pivot, two for the polymorphic pivots' id + type pair).
//! PostgreSQL placeholders are positional, so a wrong seed makes `$2`
//! collide with `$1` and the statement fails or, worse, matches the
//! wrong row. SQLite writes every marker as `?`, so the entire SQLite
//! suite passes with the seed wrong. This file is what catches it.
//!
//! This target is ignored during the normal test suite. Run it with a
//! disposable database:
//!
//! ```text
//! PG_TEST_URL=postgres://... \
//!   cargo test -p suprnova --test eloquent_relations_pivot_filters_postgres -- \
//!   --ignored --test-threads=1
//! ```
//!
//! Explicit execution without `PG_TEST_URL` fails immediately; it never
//! reports a silent pass. Every table is `suprnova_pivot_filter_`-prefixed
//! so this file cannot clobber another Postgres file's fixtures.

use sea_orm::{ConnectOptions, ConnectionTrait, Database, Statement};
use std::time::Duration;
use suprnova::testing::TestContainer;
use suprnova::{DbConnection, Model, model};

#[model(table = "suprnova_pivot_filter_users", relations = {
    roles: BelongsToMany<PgPfRole, PgPfRoleUser> {
        with_pivot = ["active", "note"],
    },
})]
pub struct PgPfUser {
    pub id: i64,
    pub name: String,
}

#[model(table = "suprnova_pivot_filter_roles")]
pub struct PgPfRole {
    pub id: i64,
    pub name: String,
}

#[model(
    table = "suprnova_pivot_filter_role_user",
    primary_key = "id",
    timestamps = false
)]
pub struct PgPfRoleUser {
    pub id: i64,
    pub pg_pf_user_id: i64,
    pub pg_pf_role_id: i64,
    pub active: i64,
    pub note: Option<String>,
}

#[model(table = "suprnova_pivot_filter_posts", morph_type = "pivot_filter_post", relations = {
    tags: MorphToMany<PgPfTag, PgPfTaggable> { name = "subject" },
})]
pub struct PgPfPost {
    pub id: i64,
    pub title: String,
}

#[model(table = "suprnova_pivot_filter_tags", relations = {
    posts: MorphedByMany<PgPfPost, PgPfTaggable> {
        name = "subject",
        target_morph_type = "pivot_filter_post",
    },
})]
pub struct PgPfTag {
    pub id: i64,
    pub label: String,
}

#[model(
    table = "suprnova_pivot_filter_taggables",
    primary_key = "id",
    timestamps = false
)]
pub struct PgPfTaggable {
    pub id: i64,
    pub pg_pf_tag_id: i64,
    pub subject_id: i64,
    pub subject_type: String,
    pub active: i64,
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

fn role_ids(rows: &[PgPfRole]) -> Vec<i64> {
    let mut ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
    ids.sort_unstable();
    ids
}

#[tokio::test]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_pivot_filters_bind_positionally_on_every_hand_built_statement() {
    let conn = connect_postgres().await;
    let backend = conn.get_database_backend();
    for sql in [
        "DROP TABLE IF EXISTS suprnova_pivot_filter_taggables",
        "DROP TABLE IF EXISTS suprnova_pivot_filter_tags",
        "DROP TABLE IF EXISTS suprnova_pivot_filter_posts",
        "DROP TABLE IF EXISTS suprnova_pivot_filter_role_user",
        "DROP TABLE IF EXISTS suprnova_pivot_filter_roles",
        "DROP TABLE IF EXISTS suprnova_pivot_filter_users",
        "CREATE TABLE suprnova_pivot_filter_users (\
             id BIGINT PRIMARY KEY,\
             name TEXT NOT NULL\
         )",
        "CREATE TABLE suprnova_pivot_filter_roles (\
             id BIGINT PRIMARY KEY,\
             name TEXT NOT NULL\
         )",
        "CREATE TABLE suprnova_pivot_filter_role_user (\
             id BIGINT PRIMARY KEY,\
             pg_pf_user_id BIGINT NOT NULL,\
             pg_pf_role_id BIGINT NOT NULL,\
             active BIGINT NOT NULL,\
             note TEXT NULL,\
             UNIQUE (pg_pf_user_id, pg_pf_role_id)\
         )",
        "CREATE TABLE suprnova_pivot_filter_posts (\
             id BIGINT PRIMARY KEY,\
             title TEXT NOT NULL\
         )",
        "CREATE TABLE suprnova_pivot_filter_tags (\
             id BIGINT PRIMARY KEY,\
             label TEXT NOT NULL\
         )",
        "CREATE TABLE suprnova_pivot_filter_taggables (\
             id BIGINT PRIMARY KEY,\
             pg_pf_tag_id BIGINT NOT NULL,\
             subject_id BIGINT NOT NULL,\
             subject_type TEXT NOT NULL,\
             active BIGINT NOT NULL\
         )",
        // One user, three roles: admin (active, noted), editor
        // (inactive, noted), viewer (active, no note).
        "INSERT INTO suprnova_pivot_filter_users (id, name) VALUES (1, 'Ada')",
        "INSERT INTO suprnova_pivot_filter_roles (id, name) VALUES \
             (10, 'admin'), (20, 'editor'), (30, 'viewer')",
        "INSERT INTO suprnova_pivot_filter_role_user \
             (id, pg_pf_user_id, pg_pf_role_id, active, note) VALUES \
             (1, 1, 10, 1, 'keep'),\
             (2, 1, 20, 0, 'keep'),\
             (3, 1, 30, 1, NULL)",
        // One live post and one dead post; the `rust` tag is attached to
        // both, active on the live one only. The `draft` tag is attached
        // to the live post, inactive.
        "INSERT INTO suprnova_pivot_filter_posts (id, title) VALUES \
             (100, 'live'), (110, 'dead')",
        "INSERT INTO suprnova_pivot_filter_tags (id, label) VALUES \
             (200, 'rust'), (210, 'draft')",
        "INSERT INTO suprnova_pivot_filter_taggables \
             (id, pg_pf_tag_id, subject_id, subject_type, active) VALUES \
             (1, 200, 100, 'pivot_filter_post', 1),\
             (2, 210, 100, 'pivot_filter_post', 0),\
             (3, 200, 110, 'pivot_filter_post', 0)",
    ] {
        conn.execute_raw(Statement::from_string(backend, sql.to_owned()))
            .await
            .unwrap_or_else(|error| panic!("fixture setup failed on {sql:?}: {error}"));
    }

    let _guard = TestContainer::fake();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));

    let user = PgPfUser::find(1)
        .await
        .expect("read the seeded user")
        .expect("the seeded user must exist");
    let post = PgPfPost::find(100)
        .await
        .expect("read the seeded post")
        .expect("the seeded post must exist");
    let rust = PgPfTag::find(200)
        .await
        .expect("read the seeded tag")
        .expect("the seeded tag must exist");

    // ---- Baselines: the unfiltered statements still work ------------
    assert_eq!(user.roles().count().await.unwrap(), 3);
    assert_eq!(
        role_ids(&user.roles().get().await.unwrap().into_vec()),
        vec![10, 20, 30]
    );
    assert_eq!(post.tags().count().await.unwrap(), 2);
    assert_eq!(rust.posts().count().await.unwrap(), 2);

    // ---- BelongsToMany::get / count - one prefix bind, filter is $2 --
    let active = user
        .roles()
        .where_pivot("active", 1i64)
        .get()
        .await
        .expect("a filtered BelongsToMany read must render valid Postgres")
        .into_vec();
    assert_eq!(role_ids(&active), vec![10, 30], "admin + viewer");
    assert_eq!(
        user.roles()
            .where_pivot("active", 1i64)
            .count()
            .await
            .unwrap(),
        2
    );

    // ---- BelongsToMany with a closure group - filter binds are $2..$4 -
    // `note IS NULL OR (active = 1 AND note = 'keep')` -> viewer + admin.
    // Three binds after the parent key, so a mis-seeded counter shows up
    // here as a collision rather than an off-by-one that still parses.
    let grouped = user
        .roles()
        .where_pivot_null("note")
        .or_where_pivot_group(|q| q.filter("active", 1i64).filter("note", "keep"))
        .get()
        .await
        .expect("a grouped BelongsToMany read must render valid Postgres")
        .into_vec();
    assert_eq!(
        role_ids(&grouped),
        vec![10, 30],
        "admin + viewer, not editor"
    );
    assert_eq!(
        user.roles()
            .where_pivot_null("note")
            .or_where_pivot_group(|q| q.filter("active", 1i64).filter("note", "keep"))
            .count()
            .await
            .unwrap(),
        2
    );

    // ---- MorphToMany::get / count - two prefix binds, filter is $3 ---
    let live_tags = post
        .tags()
        .where_pivot("active", 1i64)
        .get()
        .await
        .expect("a filtered MorphToMany read must render valid Postgres")
        .into_vec();
    assert_eq!(live_tags.len(), 1);
    assert_eq!(live_tags[0].id, 200);
    assert_eq!(
        post.tags()
            .where_pivot("active", 1i64)
            .count()
            .await
            .unwrap(),
        1
    );

    // ---- MorphedByMany::get / count - two prefix binds on count -----
    let live_posts = rust
        .posts()
        .where_pivot("active", 1i64)
        .get()
        .await
        .expect("a filtered MorphedByMany read must render valid Postgres")
        .into_vec();
    assert_eq!(live_posts.len(), 1);
    assert_eq!(live_posts[0].id, 100, "only the active taggable row");
    assert_eq!(
        rust.posts()
            .where_pivot("active", 1i64)
            .count()
            .await
            .unwrap(),
        1
    );

    // ---- The write refusal holds on Postgres too --------------------
    let err = post
        .tags()
        .where_pivot("active", 1i64)
        .detach(200i64)
        .await
        .expect_err("a filtered detach must refuse rather than delete unfiltered");
    assert!(
        format!("{err}").contains("reads only"),
        "message must explain the refusal, got: {err}"
    );
    assert_eq!(
        post.tags().count().await.unwrap(),
        2,
        "the refusal must not have deleted anything"
    );

    for sql in [
        "DROP TABLE suprnova_pivot_filter_taggables",
        "DROP TABLE suprnova_pivot_filter_tags",
        "DROP TABLE suprnova_pivot_filter_posts",
        "DROP TABLE suprnova_pivot_filter_role_user",
        "DROP TABLE suprnova_pivot_filter_roles",
        "DROP TABLE suprnova_pivot_filter_users",
    ] {
        conn.execute_raw(Statement::from_string(backend, sql.to_owned()))
            .await
            .unwrap_or_else(|error| panic!("fixture cleanup failed on {sql:?}: {error}"));
    }
}
