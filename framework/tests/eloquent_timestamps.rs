//! Phase 10A T9 — Auto-managed timestamps + `touch()`.
//!
//! When a model has both `created_at` and `updated_at` fields the
//! macro auto-detects timestamps and:
//! - sets BOTH to `Utc::now()` on `create()`
//! - bumps `updated_at` on every `save()` (and `update(attrs)`)
//! - emits a `Touchable` impl so `user.touch().await?` updates
//!   `updated_at` without changing any other column.
//!
//! Auto-detect: if the struct has NEITHER column the macro skips
//! injection silently (Laravel-parity for pivots / join tables /
//! no-history models). If the struct has EXACTLY ONE of the two
//! the macro emits a `compile_error!` — almost certainly a typo.
//!
//! `#[model(timestamps = false)]` is the explicit opt-out and works
//! regardless of which columns are on the struct.
//!
//! `#[model(touches = ["post"])]` names `BelongsTo` relations whose
//! parent row gets its `updated_at` bumped after the child is
//! created, saved, updated, or deleted. A parent whose model has
//! timestamps disabled is skipped - not an error, not a write.

use std::time::Duration;

use chrono::{DateTime, Utc};
use suprnova::testing::TestDatabase;
use suprnova::{DB, EloquentModel, FrameworkError, Model, Touchable, attrs, model};

// ---- Models ------------------------------------------------------------

// Default `timestamps = true` (implicit via auto-detect since the
// struct carries both columns).
#[model(table = "t9_users", fillable = ["name"])]
pub struct T9User {
    pub id: i64,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// Explicit opt-out via `timestamps = false`. Struct has neither
// column; this exercises the opt-out branch independently of the
// auto-detect skip case below.
#[model(table = "t9_no_ts", fillable = ["name"], timestamps = false)]
pub struct T9NoTs {
    pub id: i64,
    pub name: String,
}

// Auto-detect skip: default `timestamps = true` but the struct lacks
// both columns, so the macro silently disables injection.
#[model(table = "t9_auto_skip", fillable = ["label"])]
pub struct T9AutoSkip {
    pub id: i64,
    pub label: String,
}

// ---- Touch fixtures ----------------------------------------------------
//
// Three parents, one child. `T9Post` and `T9Video` are timestamped
// owners; `T9Archive` opts out with `timestamps = false` while its
// TABLE still carries an `updated_at` column - the exact shape
// laravel/framework#61073 patched. `T9Comment` touches all three.

#[model(table = "t9_posts", fillable = ["title"])]
pub struct T9Post {
    pub id: i64,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[model(table = "t9_videos", fillable = ["title"])]
pub struct T9Video {
    pub id: i64,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[model(table = "t9_archives", fillable = ["label"], timestamps = false)]
pub struct T9Archive {
    pub id: i64,
    pub label: String,
}

#[model(
    table = "t9_comments",
    fillable = ["post_id", "video_id", "archive_id", "body"],
    touches = ["post", "video", "archive"],
    relations = {
        post: BelongsTo<T9Post> { fk = "post_id" },
        video: BelongsTo<T9Video> { fk = "video_id" },
        archive: BelongsTo<T9Archive> { fk = "archive_id" },
    },
)]
pub struct T9Comment {
    pub id: i64,
    pub post_id: i64,
    pub video_id: Option<i64>,
    pub archive_id: i64,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ---- Helpers -----------------------------------------------------------

async fn migrate_users(db: &TestDatabase) {
    db.execute_unprepared(
        "CREATE TABLE t9_users (\
            id INTEGER PRIMARY KEY AUTOINCREMENT, \
            name TEXT NOT NULL, \
            created_at TEXT NOT NULL, \
            updated_at TEXT NOT NULL\
         )",
    )
    .await
    .unwrap();
}

// ---- Tests -------------------------------------------------------------

#[tokio::test]
async fn create_sets_both_timestamps() {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    migrate_users(&db).await;

    let u = T9User::create(attrs! { name: "Alice" }).await.unwrap();
    let now = Utc::now();
    assert!(
        (now - u.created_at).num_seconds().abs() < 5,
        "created_at not within 5s of now: created_at={} now={}",
        u.created_at,
        now,
    );
    assert!(
        (now - u.updated_at).num_seconds().abs() < 5,
        "updated_at not within 5s of now: updated_at={} now={}",
        u.updated_at,
        now,
    );
}

#[tokio::test]
async fn save_bumps_updated_at_only() {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    migrate_users(&db).await;

    let original = T9User::create(attrs! { name: "Alice" }).await.unwrap();
    let original_created = original.created_at;
    let original_updated = original.updated_at;

    // Sleep enough that a re-read's `updated_at` is observably newer
    // than the original. 1.2s comfortably exceeds the 1-second
    // resolution of SeaORM's chrono->TEXT format.
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;

    let mut handle = original.clone();
    handle.name = "Alice B".into();
    handle.save().await.unwrap();

    let reread = T9User::find(handle.id).await.unwrap().unwrap();
    assert_eq!(
        reread.created_at, original_created,
        "save() must NOT touch created_at"
    );
    assert!(
        reread.updated_at > original_updated,
        "save() must bump updated_at: reread.updated_at={} original={}",
        reread.updated_at,
        original_updated,
    );
    assert_eq!(reread.name, "Alice B");
}

#[tokio::test]
async fn update_attrs_bumps_updated_at_only() {
    // Covers the `Model::update(attrs)` path which routes through
    // `apply_attrs_to_active_model` rather than
    // `into_active_model_for_update`. The injection must catch BOTH
    // create() and update(attrs) — they share the apply_attrs hook.
    let db = TestDatabase::sqlite_memory().await.unwrap();
    migrate_users(&db).await;

    let original = T9User::create(attrs! { name: "Alice" }).await.unwrap();
    let original_created = original.created_at;
    let original_updated = original.updated_at;
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;

    let updated = original.update(attrs! { name: "Alice B" }).await.unwrap();

    assert_eq!(
        updated.created_at, original_created,
        "update(attrs) must NOT touch created_at"
    );
    assert!(
        updated.updated_at > original_updated,
        "update(attrs) must bump updated_at: updated.updated_at={} original={}",
        updated.updated_at,
        original_updated,
    );
    assert_eq!(updated.name, "Alice B");
}

#[tokio::test]
async fn touch_bumps_updated_at_without_other_changes() {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    migrate_users(&db).await;

    let u = T9User::create(attrs! { name: "Alice" }).await.unwrap();
    let original_updated = u.updated_at;
    let original_created = u.created_at;
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;

    u.touch().await.unwrap();

    let reread = T9User::find(u.id).await.unwrap().unwrap();
    assert!(
        reread.updated_at > original_updated,
        "touch() must bump updated_at: reread.updated_at={} original={}",
        reread.updated_at,
        original_updated,
    );
    assert_eq!(
        reread.created_at, original_created,
        "touch() must NOT touch created_at"
    );
    assert_eq!(
        reread.name, "Alice",
        "touch() must NOT change other columns"
    );
}

#[tokio::test]
async fn timestamps_disabled_via_attribute() {
    // Struct has neither column AND `timestamps = false`. The opt-out
    // is explicit; auto-detect would also skip here, but this branch
    // remains the canonical way to disable timestamps for models that
    // DO carry created_at/updated_at columns for unrelated reasons.
    let db = TestDatabase::sqlite_memory().await.unwrap();
    db.execute_unprepared(
        "CREATE TABLE t9_no_ts (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
    )
    .await
    .unwrap();

    let u = T9NoTs::create(attrs! { name: "Alice" }).await.unwrap();
    assert_eq!(u.name, "Alice");
}

#[tokio::test]
async fn timestamps_auto_detect_skips_when_fields_absent() {
    // Default `timestamps = true` + struct lacks both columns →
    // macro auto-detects and skips injection silently. No
    // `timestamps = false` opt-out needed.
    let db = TestDatabase::sqlite_memory().await.unwrap();
    db.execute_unprepared(
        "CREATE TABLE t9_auto_skip (id INTEGER PRIMARY KEY AUTOINCREMENT, label TEXT NOT NULL)",
    )
    .await
    .unwrap();

    let u = T9AutoSkip::create(attrs! { label: "x" }).await.unwrap();
    assert_eq!(u.label, "x");
}

// ---- Touch helpers -----------------------------------------------------

/// The touch-fixture tables. `t9_archives.updated_at` exists in SQL
/// but not on the model - it is there to prove the toucher never
/// writes it.
async fn migrate_touches(db: &TestDatabase) {
    for sql in [
        "CREATE TABLE t9_posts (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL, \
         created_at TEXT NOT NULL, updated_at TEXT NOT NULL)",
        "CREATE TABLE t9_videos (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL, \
         created_at TEXT NOT NULL, updated_at TEXT NOT NULL)",
        "CREATE TABLE t9_archives (id INTEGER PRIMARY KEY AUTOINCREMENT, label TEXT NOT NULL, \
         updated_at TEXT NOT NULL DEFAULT 'never')",
        "CREATE TABLE t9_comments (id INTEGER PRIMARY KEY AUTOINCREMENT, post_id INTEGER NOT NULL, \
         video_id INTEGER, archive_id INTEGER NOT NULL, body TEXT NOT NULL, \
         created_at TEXT NOT NULL, updated_at TEXT NOT NULL)",
    ] {
        db.execute_unprepared(sql).await.unwrap();
    }
}

/// Read `t9_archives.updated_at` raw - the model has timestamps
/// disabled, so there is no typed accessor for it.
async fn archive_updated_at(db: &TestDatabase, id: i64) -> String {
    let row = db
        .fetch_one(
            "SELECT updated_at FROM t9_archives WHERE id = ?",
            vec![id.into()],
        )
        .await
        .unwrap();
    row.try_get::<String>("", "updated_at").unwrap()
}

/// Seed one of each parent plus a comment pointing at all three, then
/// sleep so any later bump is observably newer. The parents are
/// re-read because creating the comment already touched them.
async fn seed(db: &TestDatabase) -> (T9Post, T9Video, T9Archive, T9Comment) {
    migrate_touches(db).await;
    let post = T9Post::create(attrs! { title: "p" }).await.unwrap();
    let video = T9Video::create(attrs! { title: "v" }).await.unwrap();
    let archive = T9Archive::create(attrs! { label: "a" }).await.unwrap();
    let comment = T9Comment::create(attrs! {
        post_id: post.id, video_id: video.id, archive_id: archive.id, body: "hi",
    })
    .await
    .unwrap();
    let post = T9Post::find(post.id).await.unwrap().unwrap();
    let video = T9Video::find(video.id).await.unwrap().unwrap();
    tokio::time::sleep(Duration::from_millis(1200)).await;
    (post, video, archive, comment)
}

// ---- Tests: relation touching ------------------------------------------

#[tokio::test]
// The two `HAS_TIMESTAMPS` checks below assert on an associated const
// pulled through a generic type parameter, not on a literal — clippy's
// constant folder still resolves it at lint time and flags it as a
// no-op assertion. It isn't: this is exactly what proves the macro
// emitted the right value per model.
#[allow(clippy::assertions_on_constants)]
async fn touches_const_lists_the_declared_relations() {
    // `TOUCHES` lives on `EloquentModel` now, not on an inherent impl,
    // so a `Model` trait default can read it through `Self::TOUCHES`.
    assert_eq!(
        <T9Comment as EloquentModel>::TOUCHES,
        &["post", "video", "archive"]
    );
    assert!(<T9Post as EloquentModel>::HAS_TIMESTAMPS);
    assert!(!<T9Archive as EloquentModel>::HAS_TIMESTAMPS);
}

#[tokio::test]
async fn child_create_bumps_the_parent_updated_at() {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    let (post, _video, _archive, _comment) = seed(&db).await;

    let _ = T9Comment::create(attrs! {
        post_id: post.id, video_id: 1_i64, archive_id: 1_i64, body: "second",
    })
    .await
    .unwrap();

    let reread = T9Post::find(post.id).await.unwrap().unwrap();
    assert!(
        reread.updated_at > post.updated_at,
        "create must bump the parent"
    );
}

#[tokio::test]
async fn child_save_bumps_the_parent_updated_at() {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    let (post, _video, _archive, comment) = seed(&db).await;

    let mut handle = comment.clone();
    handle.body = "edited".into();
    handle.save().await.unwrap();

    let reread = T9Post::find(post.id).await.unwrap().unwrap();
    assert!(
        reread.updated_at > post.updated_at,
        "save must bump the parent"
    );
}

#[tokio::test]
async fn child_delete_bumps_the_parent_updated_at() {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    let (post, _video, _archive, comment) = seed(&db).await;

    comment.delete().await.unwrap();

    let reread = T9Post::find(post.id).await.unwrap().unwrap();
    assert!(
        reread.updated_at > post.updated_at,
        "delete must bump the parent"
    );
}

#[tokio::test]
async fn parent_with_timestamps_disabled_is_skipped_not_written() {
    // laravel/framework#61073. `T9Archive` has `timestamps = false`
    // while its table still carries `updated_at`. The child save must
    // succeed AND leave the column byte-identical.
    let db = TestDatabase::sqlite_memory().await.unwrap();
    let (_post, _video, archive, comment) = seed(&db).await;
    assert_eq!(archive_updated_at(&db, archive.id).await, "never");

    let mut handle = comment.clone();
    handle.body = "edited".into();
    handle
        .save()
        .await
        .expect("save must return Ok, not an error");

    assert_eq!(
        archive_updated_at(&db, archive.id).await,
        "never",
        "a timestamps = false parent must be skipped, not written"
    );
}

#[tokio::test]
async fn a_null_foreign_key_touches_nothing_and_returns_ok() {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    let (post, video, _archive, _comment) = seed(&db).await;

    // `video_id` omitted -> NULL. No parent row to identify, so the
    // video relation is skipped while the post still bumps.
    let orphan = T9Comment::create(attrs! {
        post_id: post.id, archive_id: 1_i64, body: "no video",
    })
    .await
    .expect("a null FK must not fail the write");
    assert!(orphan.video_id.is_none());

    let video_reread = T9Video::find(video.id).await.unwrap().unwrap();
    assert_eq!(
        video_reread.updated_at, video.updated_at,
        "null FK touches no video"
    );
    let post_reread = T9Post::find(post.id).await.unwrap().unwrap();
    assert!(post_reread.updated_at > post.updated_at);
}

#[tokio::test]
async fn without_touching_suppresses_relation_touching() {
    // The existing scope gated direct `.touch()` only. It must gate
    // the relation cascade too.
    let db = TestDatabase::sqlite_memory().await.unwrap();
    let (post, video, _archive, comment) = seed(&db).await;

    suprnova::eloquent::without_touching(async {
        let mut handle = comment.clone();
        handle.body = "quiet".into();
        handle.save().await
    })
    .await
    .unwrap();

    let post_reread = T9Post::find(post.id).await.unwrap().unwrap();
    let video_reread = T9Video::find(video.id).await.unwrap().unwrap();
    assert_eq!(
        post_reread.updated_at, post.updated_at,
        "post cascade suppressed"
    );
    assert_eq!(
        video_reread.updated_at, video.updated_at,
        "video cascade suppressed"
    );
}

#[tokio::test]
async fn without_touching_on_suppresses_only_the_named_parent() {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    let (post, video, _archive, comment) = seed(&db).await;

    suprnova::eloquent::without_touching_on::<T9Post, _, _>(async {
        let mut handle = comment.clone();
        handle.body = "selective".into();
        handle.save().await?;
        // The per-type scope also gates a direct `.touch()` on the
        // ignored type - Laravel's `Model::withoutTouching` is exactly
        // `withoutTouchingOn([static::class])`.
        post.touch().await
    })
    .await
    .unwrap();

    let post_reread = T9Post::find(post.id).await.unwrap().unwrap();
    let video_reread = T9Video::find(video.id).await.unwrap().unwrap();
    assert_eq!(
        post_reread.updated_at, post.updated_at,
        "T9Post is in the ignore set"
    );
    assert!(
        video_reread.updated_at > video.updated_at,
        "T9Video is not ignored and must still bump"
    );
}

#[tokio::test]
async fn the_touch_lands_inside_the_callers_transaction() {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    let (post, _video, _archive, comment) = seed(&db).await;

    let err = DB::transaction(|_tx| {
        Box::pin(async move {
            let mut handle = comment.clone();
            handle.body = "rolled back".into();
            handle.save().await?;
            Err::<(), FrameworkError>(FrameworkError::internal("force rollback"))
        })
    })
    .await
    .unwrap_err();
    assert!(format!("{err}").contains("force rollback"));

    let reread = T9Post::find(post.id).await.unwrap().unwrap();
    assert_eq!(
        reread.updated_at, post.updated_at,
        "the touch must roll back with the caller's transaction"
    );
}
