//! PAR-005: `sync_without_detaching` attaches only what is missing.
//!
//! On a relation that holds roles 1 and 2, `sync_without_detaching([3])`
//! leaves it holding 1, 2 and 3, and the pivot rows of 1 and 2 exactly as
//! they were: same row ids, same extra pivot columns, same timestamps.
//! Laravel's `InteractsWithPivotTable::syncWithoutDetaching`, which both
//! `BelongsToMany` and `MorphToMany` offer.
//!
//! The clock is frozen and moved forward before each call, so a pivot row
//! the call rewrote, or deleted and inserted again, would come back with a
//! different `updated_at` or `id`.
//!
//! The `postgres_` and `mysql_` variants run the main scenario against a
//! disposable server and are ignored by default.

use chrono::{DateTime, Duration, Utc};
use serde_json::Value;
use suprnova::database::{DatabaseConfig, DbConnection};
use suprnova::testing::{TestClock, TestContainer, TestContainerGuard, TestDatabase};
use suprnova::{DB, FrameworkError, Model, attrs, model};

// ---- Models -------------------------------------------------------------

#[model(table = "swd_users", relations = {
    roles: BelongsToMany<SwdRole, SwdRoleUser> {
        with_pivot = ["note"],
        with_timestamps,
    },
})]
pub struct SwdUser {
    pub id: i64,
    pub name: String,
}

#[model(table = "swd_roles")]
pub struct SwdRole {
    pub id: i64,
    pub name: String,
}

#[model(table = "swd_role_user", primary_key = "id")]
pub struct SwdRoleUser {
    pub id: i64,
    pub swd_user_id: i64,
    pub swd_role_id: i64,
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[model(table = "swd_posts", morph_type = "swd_post", relations = {
    tags: MorphToMany<SwdTag, SwdTaggable> { name = "taggable" },
})]
pub struct SwdPost {
    pub id: i64,
    pub title: String,
}

#[model(table = "swd_videos", morph_type = "swd_video", relations = {
    tags: MorphToMany<SwdTag, SwdTaggable> { name = "taggable" },
})]
pub struct SwdVideo {
    pub id: i64,
    pub url: String,
}

#[model(table = "swd_tags")]
pub struct SwdTag {
    pub id: i64,
    pub name: String,
}

#[model(table = "swd_taggables", primary_key = "id", timestamps = false)]
pub struct SwdTaggable {
    pub id: i64,
    pub swd_tag_id: i64,
    pub taggable_id: i64,
    pub taggable_type: String,
    pub weight: Option<i64>,
}

// ---- Fixtures -----------------------------------------------------------

/// Role ids at or above this are refused by the pivot table's CHECK
/// constraint: the failure-mode tests use it to make one attach fail.
const REFUSED_ROLE_ID: i64 = 1000;

async fn sqlite() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    for sql in [
        "CREATE TABLE swd_users (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
        "CREATE TABLE swd_roles (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
        "CREATE TABLE swd_role_user (\
            id INTEGER PRIMARY KEY AUTOINCREMENT, \
            swd_user_id INTEGER NOT NULL, \
            swd_role_id INTEGER NOT NULL CHECK (swd_role_id < 1000), \
            note TEXT, \
            created_at TEXT NOT NULL, \
            updated_at TEXT NOT NULL, \
            UNIQUE(swd_user_id, swd_role_id))",
        "CREATE TABLE swd_posts (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL)",
        "CREATE TABLE swd_videos (id INTEGER PRIMARY KEY AUTOINCREMENT, url TEXT NOT NULL)",
        "CREATE TABLE swd_tags (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
        "CREATE TABLE swd_taggables (\
            id INTEGER PRIMARY KEY AUTOINCREMENT, \
            swd_tag_id INTEGER NOT NULL, \
            taggable_id INTEGER NOT NULL, \
            taggable_type TEXT NOT NULL, \
            weight INTEGER, \
            UNIQUE(swd_tag_id, taggable_id, taggable_type))",
    ] {
        db.execute_unprepared(sql).await.unwrap();
    }
    db
}

/// A user holding roles 1 and 2, each pivot row with its own note, plus
/// a third role it does not hold yet.
async fn user_holding_two_roles() -> (SwdUser, [SwdRole; 3]) {
    let user = SwdUser::create(attrs! { name: "Ada" }).await.unwrap();
    let mut roles = Vec::new();
    for name in ["reader", "writer", "admin"] {
        roles.push(SwdRole::create(attrs! { name: name }).await.unwrap());
    }
    user.roles()
        .attach_with(roles[0].id, attrs! { note: "first" })
        .await
        .unwrap();
    user.roles()
        .attach_with(roles[1].id, attrs! { note: "second" })
        .await
        .unwrap();
    let roles: [SwdRole; 3] = roles.try_into().unwrap();
    (user, roles)
}

/// Every pivot row of `user`, as JSON, ordered by role id.
async fn pivot_rows(user: &SwdUser) -> Vec<Value> {
    let mut rows: Vec<SwdRoleUser> = SwdRoleUser::query()
        .filter("swd_user_id", user.id)
        .get()
        .await
        .unwrap()
        .into_vec();
    rows.sort_by_key(|r| r.swd_role_id);
    rows.iter()
        .map(|r| serde_json::to_value(r).unwrap())
        .collect()
}

async fn held_role_ids(user: &SwdUser) -> Vec<i64> {
    let mut ids: Vec<i64> = user
        .roles()
        .get()
        .await
        .unwrap()
        .iter()
        .map(|r| r.id)
        .collect();
    ids.sort_unstable();
    ids
}

// ---- BelongsToMany ------------------------------------------------------

#[tokio::test]
async fn attaches_the_missing_id_and_leaves_existing_rows_untouched() {
    let _db = sqlite().await;
    let clock = TestClock::freeze();
    let (user, [r1, r2, r3]) = user_holding_two_roles().await;
    let before = pivot_rows(&user).await;
    assert_eq!(before.len(), 2);

    clock.advance(Duration::seconds(60));
    user.roles().sync_without_detaching([r3.id]).await.unwrap();

    assert_eq!(held_role_ids(&user).await, vec![r1.id, r2.id, r3.id]);
    let after = pivot_rows(&user).await;
    assert_eq!(after.len(), 3);
    assert_eq!(
        &after[..2],
        &before[..],
        "the pivot rows of the roles already held are unchanged"
    );
    let added: SwdRoleUser = serde_json::from_value(after[2].clone()).unwrap();
    assert_eq!(added.swd_role_id, r3.id);
    assert_eq!(added.note, None);
    assert_eq!(
        added.created_at.timestamp(),
        suprnova::clock::now().timestamp(),
        "the new pivot row is stamped with the time of the call"
    );
}

#[tokio::test]
async fn ids_the_relation_already_holds_are_neither_duplicated_nor_rewritten() {
    let _db = sqlite().await;
    let clock = TestClock::freeze();
    let (user, [r1, r2, r3]) = user_holding_two_roles().await;
    let before = pivot_rows(&user).await;

    clock.advance(Duration::seconds(60));
    user.roles()
        .sync_without_detaching([r2.id, r3.id, r1.id])
        .await
        .unwrap();

    assert_eq!(held_role_ids(&user).await, vec![r1.id, r2.id, r3.id]);
    let after = pivot_rows(&user).await;
    assert_eq!(&after[..2], &before[..]);
}

#[tokio::test]
async fn an_empty_list_changes_nothing() {
    let _db = sqlite().await;
    let (user, [r1, r2, _r3]) = user_holding_two_roles().await;
    let before = pivot_rows(&user).await;

    user.roles()
        .sync_without_detaching(Vec::<i64>::new())
        .await
        .unwrap();

    assert_eq!(held_role_ids(&user).await, vec![r1.id, r2.id]);
    assert_eq!(pivot_rows(&user).await, before);
}

#[tokio::test]
async fn a_repeated_id_is_attached_once() {
    let _db = sqlite().await;
    let (user, [r1, r2, r3]) = user_holding_two_roles().await;

    user.roles()
        .sync_without_detaching([r3.id, r3.id])
        .await
        .unwrap();

    assert_eq!(held_role_ids(&user).await, vec![r1.id, r2.id, r3.id]);
    assert_eq!(pivot_rows(&user).await.len(), 3);
}

#[tokio::test]
async fn one_failed_attach_rolls_back_the_whole_call() {
    let _db = sqlite().await;
    let (user, [r1, r2, r3]) = user_holding_two_roles().await;
    let before = pivot_rows(&user).await;

    user.roles()
        .sync_without_detaching([r3.id, REFUSED_ROLE_ID])
        .await
        .expect_err("the CHECK constraint refuses the second attach");

    assert_eq!(
        held_role_ids(&user).await,
        vec![r1.id, r2.id],
        "the attach of r3 rolled back with the failed one"
    );
    assert_eq!(pivot_rows(&user).await, before);
}

#[tokio::test]
async fn inside_a_transaction_that_rolls_back_nothing_is_attached() {
    let _db = sqlite().await;
    let (user, [r1, r2, r3]) = user_holding_two_roles().await;
    let relation_owner = user.clone();

    let result: Result<(), FrameworkError> = DB::transaction(|_tx| {
        Box::pin(async move {
            relation_owner
                .roles()
                .sync_without_detaching([r3.id])
                .await?;
            Err(FrameworkError::internal("roll the transaction back"))
        })
    })
    .await;
    assert!(result.is_err());

    assert_eq!(held_role_ids(&user).await, vec![r1.id, r2.id]);
}

#[tokio::test]
async fn a_relation_with_a_pivot_filter_refuses_to_write() {
    let _db = sqlite().await;
    let (user, [r1, r2, r3]) = user_holding_two_roles().await;

    let err = user
        .roles()
        .where_pivot("note", "first")
        .sync_without_detaching([r3.id])
        .await
        .expect_err("a pivot filter constrains reads only");
    assert!(err.to_string().contains("reads only"), "{err}");
    assert_eq!(held_role_ids(&user).await, vec![r1.id, r2.id]);
}

// ---- MorphToMany --------------------------------------------------------

async fn tag_ids(post: &SwdPost) -> Vec<i64> {
    let mut ids: Vec<i64> = post
        .tags()
        .get()
        .await
        .unwrap()
        .iter()
        .map(|t| t.id)
        .collect();
    ids.sort_unstable();
    ids
}

async fn taggable_rows() -> Vec<Value> {
    let mut rows: Vec<SwdTaggable> = SwdTaggable::query().get().await.unwrap().into_vec();
    rows.sort_by_key(|r| r.id);
    rows.iter()
        .map(|r| serde_json::to_value(r).unwrap())
        .collect()
}

#[tokio::test]
async fn morph_to_many_attaches_the_missing_id_and_leaves_existing_rows_untouched() {
    let _db = sqlite().await;
    let post = SwdPost::create(attrs! { title: "Hello" }).await.unwrap();
    let video = SwdVideo::create(attrs! { url: "https://example.com/v" })
        .await
        .unwrap();
    let mut tags = Vec::new();
    for name in ["rust", "web", "orm"] {
        tags.push(SwdTag::create(attrs! { name: name }).await.unwrap());
    }
    post.tags()
        .attach_with(tags[0].id, attrs! { weight: 5i64 })
        .await
        .unwrap();
    post.tags()
        .attach_with(tags[1].id, attrs! { weight: 7i64 })
        .await
        .unwrap();
    // The same tag on another morph family must stay out of the post's set.
    video.tags().attach(tags[2].id).await.unwrap();
    let before = taggable_rows().await;

    post.tags()
        .sync_without_detaching([tags[1].id, tags[2].id])
        .await
        .unwrap();

    assert_eq!(
        tag_ids(&post).await,
        vec![tags[0].id, tags[1].id, tags[2].id]
    );
    let after = taggable_rows().await;
    assert_eq!(after.len(), 4);
    assert_eq!(
        &after[..3],
        &before[..],
        "the existing pivot rows, the video's included, are unchanged"
    );
    let added: SwdTaggable = serde_json::from_value(after[3].clone()).unwrap();
    assert_eq!(added.taggable_id, post.id);
    assert_eq!(added.taggable_type, "swd_post");
    assert_eq!(added.swd_tag_id, tags[2].id);
    assert_eq!(added.weight, None);
}

// ---- Live engines -------------------------------------------------------

async fn connect_live(env: &str) -> (TestContainerGuard, DbConnection) {
    let url = std::env::var(env).expect("explicit disposable database URL required");
    let guard = TestContainer::fake();
    let config = DatabaseConfig::builder()
        .url(url)
        .max_connections(1)
        .min_connections(1)
        .logging(false)
        .build();
    let database = DbConnection::connect(&config)
        .await
        .expect("connect test database");
    TestContainer::singleton(database.clone());
    (guard, database)
}

async fn live_sync_without_detaching(env: &str) {
    use sea_orm::ConnectionTrait;

    let (guard, database) = connect_live(env).await;
    let id_column = match database.inner().get_database_backend() {
        sea_orm::DatabaseBackend::Postgres => "id BIGSERIAL PRIMARY KEY",
        _ => "id BIGINT AUTO_INCREMENT PRIMARY KEY",
    };
    // `VARCHAR(255)` timestamps, as `t.timestamps()` creates them: a
    // `DateTime<Utc>` field without a cast stores RFC 3339 text.
    let timestamp_type = "VARCHAR(255)";
    for sql in [
        format!("CREATE TEMPORARY TABLE swd_users ({id_column}, name VARCHAR(255) NOT NULL)"),
        format!("CREATE TEMPORARY TABLE swd_roles ({id_column}, name VARCHAR(255) NOT NULL)"),
        format!(
            "CREATE TEMPORARY TABLE swd_role_user ({id_column}, \
             swd_user_id BIGINT NOT NULL, swd_role_id BIGINT NOT NULL, \
             note VARCHAR(255) NULL, created_at {timestamp_type} NOT NULL, \
             updated_at {timestamp_type} NOT NULL, \
             UNIQUE (swd_user_id, swd_role_id))"
        ),
    ] {
        database
            .inner()
            .execute_unprepared(&sql)
            .await
            .expect("create isolated temporary table");
    }

    let clock = TestClock::freeze();
    let (user, [r1, r2, r3]) = user_holding_two_roles().await;
    let before = pivot_rows(&user).await;
    clock.advance(Duration::seconds(60));

    user.roles()
        .sync_without_detaching([r2.id, r3.id])
        .await
        .unwrap();

    assert_eq!(held_role_ids(&user).await, vec![r1.id, r2.id, r3.id]);
    let after = pivot_rows(&user).await;
    assert_eq!(after.len(), 3);
    assert_eq!(&after[..2], &before[..]);

    drop(clock);
    drop(guard);
    database.inner().clone().close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL at PG_TEST_URL"]
async fn postgres_sync_without_detaching_leaves_existing_rows_untouched() {
    live_sync_without_detaching("PG_TEST_URL").await;
}

#[tokio::test]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_sync_without_detaching_leaves_existing_rows_untouched() {
    live_sync_without_detaching("MYSQL_TEST_URL").await;
}
