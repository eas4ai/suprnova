//! `#[model(touches = [...])]` through a `MorphTo` relation.
//!
//! On create, save, update and delete of a comment, its owner is the row its
//! `commentable_type` and `commentable_id` columns name, found through the
//! morph registry, and that row's `updated_at` is written. An owner whose
//! model has `timestamps = false` or is soft deleted is skipped, a null
//! `commentable_id` touches nothing, and a `commentable_type` that names
//! none of the relation's targets fails the write before it runs: no row is
//! written, no observer is called and no other owner is touched. The touch
//! runs inside the caller's transaction when there is one.
//!
//! Every owner row starts with the same old `updated_at`, written by hand,
//! so a touch shows as a changed value without waiting on the clock.

use std::sync::Mutex;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use suprnova::eloquent::events::EventResult;
use suprnova::eloquent::observers::Observer;
use suprnova::testing::TestDatabase;
use suprnova::{Attrs, DB, FrameworkError, Model, attrs, model};

/// The `updated_at` every owner row is seeded with.
const STALE: &str = "2000-01-01T00:00:00+00:00";

#[model(table = "mt_posts", morph_type = "mt_post")]
pub struct MtPost {
    pub id: i64,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[model(table = "mt_videos", morph_type = "mt_video")]
pub struct MtVideo {
    pub id: i64,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// An owner that opts out of timestamps while its table still has an
/// `updated_at` column, which the touch must leave alone.
#[model(table = "mt_archives", timestamps = false, morph_type = "mt_archive")]
pub struct MtArchive {
    pub id: i64,
    pub label: String,
}

#[model(
    table = "mt_comments",
    touches = ["commentable"],
    relations = {
        commentable: MorphTo { targets = [MtPost, MtVideo, MtArchive] },
    },
)]
pub struct MtComment {
    pub id: i64,
    pub commentable_id: Option<i64>,
    pub commentable_type: Option<String>,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// An owner that is soft deleted when its `deleted_at` is set.
#[model(table = "mt_soft_owners", soft_deletes, morph_type = "mt_soft_owner")]
pub struct MtSoftOwner {
    pub id: i64,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[model(table = "mt_authors")]
pub struct MtAuthor {
    pub id: i64,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A child that touches a `BelongsTo` owner first and a `MorphTo` owner
/// second, so a `MorphTo` that cannot be resolved must leave the first
/// owner alone.
#[model(
    table = "mt_notes",
    touches = ["author", "subject"],
    relations = {
        author: BelongsTo<MtAuthor> { fk = "author_id" },
        subject: MorphTo { targets = [MtPost, MtSoftOwner] },
    },
)]
pub struct MtNote {
    pub id: i64,
    pub author_id: i64,
    pub subject_id: Option<i64>,
    pub subject_type: Option<String>,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A child whose `Saving` listener decides its owner: a body of `reroute`
/// moves the row to the video, a body of `poison` to a type no target has.
#[model(
    table = "mt_routed",
    touches = ["target"],
    relations = {
        target: MorphTo { targets = [MtPost, MtVideo] },
    },
)]
pub struct MtRouted {
    pub id: i64,
    pub target_id: Option<i64>,
    pub target_type: Option<String>,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// The bodies of the routed rows the `Saved` event has been dispatched for.
static SAVED_BODIES: Mutex<Vec<String>> = Mutex::new(Vec::new());

#[derive(Clone)]
struct Reroute;

#[async_trait]
impl Observer<MtRouted> for Reroute {
    async fn saving(&self, attrs: &mut Attrs, _is_creating: bool) -> EventResult {
        let target = match attrs.get("body").and_then(|v| v.as_str()) {
            Some("reroute") => Some("mt_video"),
            Some("poison") => Some("mt_nothing"),
            _ => None,
        };
        if let Some(target) = target {
            attrs.insert("target_type", target);
        }
        EventResult::Ok
    }

    async fn saved(&self, routed: &MtRouted) -> Result<(), FrameworkError> {
        SAVED_BODIES
            .lock()
            .expect("the recorder is never poisoned")
            .push(routed.body.clone());
        Ok(())
    }
}

/// Whether the `Saved` event was dispatched for a routed row with `body`.
fn saved_for(body: &str) -> bool {
    SAVED_BODIES
        .lock()
        .expect("the recorder is never poisoned")
        .iter()
        .any(|b| b == body)
}

/// The bodies of the comments the `Created` event has been dispatched for.
static CREATED_BODIES: Mutex<Vec<String>> = Mutex::new(Vec::new());

#[derive(Clone)]
struct RecordCreated;

#[async_trait]
impl Observer<MtComment> for RecordCreated {
    async fn created(&self, comment: &MtComment) -> Result<(), FrameworkError> {
        CREATED_BODIES
            .lock()
            .expect("the recorder is never poisoned")
            .push(comment.body.clone());
        Ok(())
    }
}

/// Whether the `Created` event was dispatched for a comment with `body`.
fn created_for(body: &str) -> bool {
    CREATED_BODIES
        .lock()
        .expect("the recorder is never poisoned")
        .iter()
        .any(|b| b == body)
}

/// One post, one video and one archive, all with the id 1, and a comment
/// per case: on the post (1), on the video (2), on the archive (3), with
/// null columns (4), and with a type no target has (5). The author 1 and
/// the soft-deletable owners 1 (trashed) and 2 (live) serve the notes.
async fn fixture() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    for sql in [
        "CREATE TABLE mt_posts (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL, \
         created_at TEXT NOT NULL, updated_at TEXT NOT NULL)"
            .to_string(),
        "CREATE TABLE mt_videos (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL, \
         created_at TEXT NOT NULL, updated_at TEXT NOT NULL)"
            .to_string(),
        "CREATE TABLE mt_archives (id INTEGER PRIMARY KEY AUTOINCREMENT, label TEXT NOT NULL, \
         updated_at TEXT NOT NULL DEFAULT 'never')"
            .to_string(),
        "CREATE TABLE mt_comments (id INTEGER PRIMARY KEY AUTOINCREMENT, \
         commentable_id INTEGER, commentable_type TEXT, body TEXT NOT NULL, \
         created_at TEXT NOT NULL, updated_at TEXT NOT NULL)"
            .to_string(),
        "CREATE TABLE mt_soft_owners (id INTEGER PRIMARY KEY AUTOINCREMENT, \
         title TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL, \
         deleted_at TEXT)"
            .to_string(),
        "CREATE TABLE mt_authors (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, \
         created_at TEXT NOT NULL, updated_at TEXT NOT NULL)"
            .to_string(),
        "CREATE TABLE mt_notes (id INTEGER PRIMARY KEY AUTOINCREMENT, author_id INTEGER NOT NULL, \
         subject_id INTEGER, subject_type TEXT, body TEXT NOT NULL, \
         created_at TEXT NOT NULL, updated_at TEXT NOT NULL)"
            .to_string(),
        "CREATE TABLE mt_routed (id INTEGER PRIMARY KEY AUTOINCREMENT, \
         target_id INTEGER, target_type TEXT, body TEXT NOT NULL, \
         created_at TEXT NOT NULL, updated_at TEXT NOT NULL)"
            .to_string(),
        format!(
            "INSERT INTO mt_routed (id, target_id, target_type, body, created_at, updated_at) \
             VALUES (1, 1, 'mt_post', 'seed', '{STALE}', '{STALE}')"
        ),
        format!(
            "INSERT INTO mt_authors (id, name, created_at, updated_at) \
             VALUES (1, 'a', '{STALE}', '{STALE}')"
        ),
        format!(
            "INSERT INTO mt_soft_owners (id, title, created_at, updated_at, deleted_at) \
             VALUES (1, 'trashed', '{STALE}', '{STALE}', '{STALE}')"
        ),
        format!(
            "INSERT INTO mt_soft_owners (id, title, created_at, updated_at, deleted_at) \
             VALUES (2, 'live', '{STALE}', '{STALE}', NULL)"
        ),
        format!(
            "INSERT INTO mt_posts (id, title, created_at, updated_at) \
             VALUES (1, 'p', '{STALE}', '{STALE}')"
        ),
        format!(
            "INSERT INTO mt_videos (id, title, created_at, updated_at) \
             VALUES (1, 'v', '{STALE}', '{STALE}')"
        ),
        "INSERT INTO mt_archives (id, label) VALUES (1, 'a')".to_string(),
        format!(
            "INSERT INTO mt_comments \
             (id, commentable_id, commentable_type, body, created_at, updated_at) VALUES \
             (1, 1, 'mt_post', 'on post', '{STALE}', '{STALE}')"
        ),
        format!(
            "INSERT INTO mt_comments \
             (id, commentable_id, commentable_type, body, created_at, updated_at) VALUES \
             (2, 1, 'mt_video', 'on video', '{STALE}', '{STALE}')"
        ),
        format!(
            "INSERT INTO mt_comments \
             (id, commentable_id, commentable_type, body, created_at, updated_at) VALUES \
             (3, 1, 'mt_archive', 'on archive', '{STALE}', '{STALE}')"
        ),
        format!(
            "INSERT INTO mt_comments \
             (id, commentable_id, commentable_type, body, created_at, updated_at) VALUES \
             (4, NULL, NULL, 'on nothing', '{STALE}', '{STALE}')"
        ),
        format!(
            "INSERT INTO mt_comments \
             (id, commentable_id, commentable_type, body, created_at, updated_at) VALUES \
             (5, 1, 'mt_nothing', 'on no target', '{STALE}', '{STALE}')"
        ),
    ] {
        db.execute_unprepared(&sql).await.unwrap();
    }
    db
}

/// The raw `updated_at` of row `id` of `table`.
async fn updated_at_of(db: &TestDatabase, table: &str, id: i64) -> String {
    let row = db
        .fetch_one(
            &format!("SELECT updated_at FROM {table} WHERE id = {id}"),
            vec![],
        )
        .await
        .unwrap();
    row.try_get::<String>("", "updated_at").unwrap()
}

/// The raw `updated_at` of row 1 of `table`.
async fn updated_at(db: &TestDatabase, table: &str) -> String {
    updated_at_of(db, table, 1).await
}

/// The number of rows of `table`.
async fn row_count(db: &TestDatabase, table: &str) -> i64 {
    let row = db
        .fetch_one(&format!("SELECT COUNT(*) AS n FROM {table}"), vec![])
        .await
        .unwrap();
    row.try_get::<i64>("", "n").unwrap()
}

/// A note by raw SQL, so its `subject_type` can be a value the model
/// refuses to write.
async fn insert_note(db: &TestDatabase, id: i64, subject_id: i64, subject_type: &str) {
    db.execute_unprepared(&format!(
        "INSERT INTO mt_notes \
         (id, author_id, subject_id, subject_type, body, created_at, updated_at) VALUES \
         ({id}, 1, {subject_id}, '{subject_type}', 'note', '{STALE}', '{STALE}')"
    ))
    .await
    .unwrap();
}

async fn note(id: i64) -> MtNote {
    MtNote::find(id).await.unwrap().expect("seeded note")
}

async fn comment(id: i64) -> MtComment {
    MtComment::find(id).await.unwrap().expect("seeded comment")
}

#[tokio::test]
async fn saving_a_child_touches_the_owner_its_type_names() {
    let db = fixture().await;

    let mut on_post = comment(1).await;
    on_post.body = "edited".into();
    on_post.save().await.unwrap();

    assert_ne!(
        updated_at(&db, "mt_posts").await,
        STALE,
        "the post is touched"
    );
    assert_eq!(
        updated_at(&db, "mt_videos").await,
        STALE,
        "the video with the same id is another family and is not touched"
    );
}

#[tokio::test]
async fn deleting_a_child_touches_its_owner() {
    let db = fixture().await;

    comment(2).await.delete().await.unwrap();

    assert_ne!(
        updated_at(&db, "mt_videos").await,
        STALE,
        "the video is touched"
    );
    assert_eq!(updated_at(&db, "mt_posts").await, STALE);
}

#[tokio::test]
async fn an_owner_without_timestamps_is_skipped_not_written() {
    let db = fixture().await;

    let mut on_archive = comment(3).await;
    on_archive.body = "edited".into();
    on_archive
        .save()
        .await
        .expect("a timestamps = false owner is skipped, not an error");

    assert_eq!(updated_at(&db, "mt_archives").await, "never");
}

#[tokio::test]
async fn a_null_morph_id_touches_nothing() {
    let db = fixture().await;

    let mut orphan = comment(4).await;
    assert!(orphan.commentable_id.is_none());
    orphan.body = "edited".into();
    orphan.save().await.expect("a null id is not an error");

    assert_eq!(updated_at(&db, "mt_posts").await, STALE);
    assert_eq!(updated_at(&db, "mt_videos").await, STALE);
}

#[tokio::test]
async fn a_type_that_names_no_target_fails_the_save() {
    let db = fixture().await;

    let mut stray = comment(5).await;
    stray.body = "edited".into();
    let Err(err) = stray.save().await else {
        panic!("`mt_nothing` names no target of `commentable`: the save must fail");
    };
    let message = err.to_string();
    assert!(message.contains("mt_nothing"), "names the type: {message}");
    assert!(
        message.contains("commentable"),
        "names the relation: {message}"
    );
    assert!(
        !message.chars().any(|c| c.is_ascii_digit()),
        "the error does not quote the row's id: {message}"
    );
    assert_eq!(updated_at(&db, "mt_posts").await, STALE);
    assert_eq!(comment(5).await.body, "on no target", "nothing was written");
}

#[tokio::test]
async fn create_with_a_type_that_names_no_target_writes_nothing_and_fires_no_observer() {
    let db = fixture().await;
    MtComment::observe(RecordCreated).await;
    let before = row_count(&db, "mt_comments").await;

    let Err(err) = MtComment::create(attrs! {
        commentable_id: 1,
        commentable_type: "mt_nothing",
        body: "create-with-no-target",
    })
    .await
    else {
        panic!("`mt_nothing` names no target of `commentable`: the create must fail");
    };
    assert!(
        err.to_string().contains("mt_nothing"),
        "names the type: {err}"
    );

    assert_eq!(
        row_count(&db, "mt_comments").await,
        before,
        "no row was inserted"
    );
    assert!(
        !created_for("create-with-no-target"),
        "the Created observer was not called"
    );

    // The observer is wired: a create that resolves fires it.
    MtComment::create(attrs! {
        commentable_id: 1,
        commentable_type: "mt_post",
        body: "create-with-a-target",
    })
    .await
    .unwrap();
    assert!(created_for("create-with-a-target"));
}

#[tokio::test]
async fn save_with_a_type_that_names_no_target_leaves_the_row_as_it_was() {
    let db = fixture().await;

    let mut moved = comment(1).await;
    moved.commentable_type = Some("mt_nothing".into());
    moved.body = "moved".into();
    assert!(moved.save().await.is_err());

    let fresh = comment(1).await;
    assert_eq!(fresh.body, "on post");
    assert_eq!(fresh.commentable_type.as_deref(), Some("mt_post"));
    assert_eq!(updated_at(&db, "mt_posts").await, STALE);
}

#[tokio::test]
async fn delete_of_a_row_with_a_type_that_names_no_target_keeps_the_row() {
    let _db = fixture().await;

    assert!(comment(5).await.delete().await.is_err());
    assert!(
        MtComment::find(5).await.unwrap().is_some(),
        "the row is still there after delete"
    );

    assert!(comment(5).await.force_delete().await.is_err());
    assert!(
        MtComment::find(5).await.unwrap().is_some(),
        "the row is still there after force_delete"
    );
}

#[tokio::test]
async fn a_type_that_names_no_target_stops_the_write_inside_a_transaction_too() {
    let db = fixture().await;
    // Counted before the transaction takes the one connection of the
    // test database: a read through the pool would wait for it.
    let before = row_count(&db, "mt_comments").await;
    let tx = DB::begin_transaction().await.unwrap();

    let result = MtComment::create_with_tx(
        &tx,
        attrs! {
            commentable_id: 1,
            commentable_type: "mt_nothing",
            body: "in-a-transaction",
        },
    )
    .await;
    assert!(result.is_err());
    tx.commit().await.unwrap();

    assert_eq!(row_count(&db, "mt_comments").await, before);
}

#[tokio::test]
async fn a_bad_morph_name_leaves_the_owner_of_an_earlier_name_untouched() {
    let db = fixture().await;
    insert_note(&db, 1, 1, "mt_nothing").await;

    let mut stray = note(1).await;
    stray.body = "edited".into();
    assert!(
        stray.save().await.is_err(),
        "`mt_nothing` names no target of `subject`"
    );

    assert_eq!(
        updated_at(&db, "mt_authors").await,
        STALE,
        "the BelongsTo owner named first is not touched when the MorphTo name fails"
    );
    assert_eq!(note(1).await.body, "note", "nothing was written");
}

#[tokio::test]
async fn a_soft_deleted_owner_is_not_touched() {
    let db = fixture().await;
    insert_note(&db, 1, 1, "mt_soft_owner").await;
    insert_note(&db, 2, 2, "mt_soft_owner").await;

    let mut on_trashed = note(1).await;
    on_trashed.body = "edited".into();
    on_trashed.save().await.unwrap();
    assert_eq!(
        updated_at_of(&db, "mt_soft_owners", 1).await,
        STALE,
        "the trashed owner is left alone"
    );

    let mut on_live = note(2).await;
    on_live.body = "edited".into();
    on_live.save().await.unwrap();
    assert_ne!(
        updated_at_of(&db, "mt_soft_owners", 2).await,
        STALE,
        "a live owner of the same model is touched"
    );
}

#[tokio::test]
async fn create_touches_the_owner_its_type_names() {
    let db = fixture().await;

    MtComment::create(attrs! {
        commentable_id: 1,
        commentable_type: "mt_video",
        body: "new",
    })
    .await
    .unwrap();

    assert_ne!(
        updated_at(&db, "mt_videos").await,
        STALE,
        "the video is touched"
    );
    assert_eq!(updated_at(&db, "mt_posts").await, STALE);
}

#[tokio::test]
async fn update_that_moves_a_child_touches_the_owner_the_row_has_after_it() {
    let db = fixture().await;

    comment(1)
        .await
        .update(attrs! { commentable_type: "mt_video" })
        .await
        .unwrap();

    assert_ne!(
        updated_at(&db, "mt_videos").await,
        STALE,
        "the new owner is touched"
    );
    assert_eq!(
        updated_at(&db, "mt_posts").await,
        STALE,
        "the old owner is not"
    );
}

#[tokio::test]
async fn save_that_moves_a_child_touches_the_owner_the_row_has_after_it() {
    let db = fixture().await;

    let mut moved = comment(1).await;
    moved.commentable_type = Some("mt_video".into());
    moved.save().await.unwrap();

    assert_ne!(updated_at(&db, "mt_videos").await, STALE);
    assert_eq!(updated_at(&db, "mt_posts").await, STALE);
}

#[tokio::test]
async fn force_delete_touches_the_owner() {
    let db = fixture().await;

    comment(1).await.force_delete().await.unwrap();

    assert_ne!(
        updated_at(&db, "mt_posts").await,
        STALE,
        "the post is touched"
    );
    assert_eq!(updated_at(&db, "mt_videos").await, STALE);
}

#[tokio::test]
async fn without_touching_on_silences_a_morph_to_owner_of_that_type_only() {
    let db = fixture().await;

    suprnova::eloquent::without_touching_on::<MtPost, _, _>(async {
        let mut on_post = comment(1).await;
        on_post.body = "quiet".into();
        on_post.save().await.unwrap();

        let mut on_video = comment(2).await;
        on_video.body = "loud".into();
        on_video.save().await.unwrap();
    })
    .await;

    assert_eq!(
        updated_at(&db, "mt_posts").await,
        STALE,
        "the silenced owner type is not touched"
    );
    assert_ne!(
        updated_at(&db, "mt_videos").await,
        STALE,
        "an owner of another type still is"
    );
}

#[tokio::test]
async fn a_saving_listener_that_moves_the_type_moves_the_touch_with_it() {
    let db = fixture().await;
    MtRouted::observe(Reroute).await;

    let mut routed = MtRouted::find(1).await.unwrap().expect("seeded row");
    routed.body = "reroute".into();
    routed.save().await.unwrap();

    assert_ne!(
        updated_at(&db, "mt_videos").await,
        STALE,
        "the owner the listener chose is touched"
    );
    assert_eq!(
        updated_at(&db, "mt_posts").await,
        STALE,
        "the owner the row had before the listener is not"
    );
}

#[tokio::test]
async fn a_saving_listener_that_moves_the_type_moves_the_touch_of_a_create() {
    let db = fixture().await;
    MtRouted::observe(Reroute).await;

    MtRouted::create(attrs! { target_id: 1, target_type: "mt_post", body: "reroute" })
        .await
        .unwrap();

    assert_ne!(updated_at(&db, "mt_videos").await, STALE);
    assert_eq!(updated_at(&db, "mt_posts").await, STALE);
}

#[tokio::test]
async fn a_saving_listener_that_writes_a_type_no_target_has_fails_the_save() {
    let db = fixture().await;
    MtRouted::observe(Reroute).await;

    let mut routed = MtRouted::find(1).await.unwrap().expect("seeded row");
    routed.body = "poison".into();
    let Err(err) = routed.save().await else {
        panic!("the listener wrote `mt_nothing`, which names no target: the save must fail");
    };
    assert!(
        err.to_string().contains("mt_nothing"),
        "names the type: {err}"
    );

    let fresh = MtRouted::find(1).await.unwrap().expect("seeded row");
    assert_eq!(fresh.body, "seed", "the statement did not run");
    assert_eq!(fresh.target_type.as_deref(), Some("mt_post"));
    assert!(!saved_for("poison"), "no Saved listener ran");
    assert_eq!(updated_at(&db, "mt_posts").await, STALE);
    assert_eq!(updated_at(&db, "mt_videos").await, STALE);
}

#[tokio::test]
async fn the_touch_commits_with_the_callers_transaction() {
    let db = fixture().await;
    let on_post = comment(1).await;

    DB::transaction(|_tx| {
        Box::pin(async move {
            let mut handle = on_post.clone();
            handle.body = "committed".into();
            handle.save().await
        })
    })
    .await
    .unwrap();

    assert_ne!(updated_at(&db, "mt_posts").await, STALE);
}

#[tokio::test]
async fn the_touch_rolls_back_with_the_callers_transaction() {
    let db = fixture().await;
    let on_post = comment(1).await;

    let err = DB::transaction(|_tx| {
        Box::pin(async move {
            let mut handle = on_post.clone();
            handle.body = "rolled back".into();
            handle.save().await?;
            Err::<(), FrameworkError>(FrameworkError::internal("force rollback"))
        })
    })
    .await
    .unwrap_err();
    assert!(format!("{err}").contains("force rollback"));

    assert_eq!(
        updated_at(&db, "mt_posts").await,
        STALE,
        "the touch ran inside the transaction and rolled back with it"
    );
}

#[tokio::test]
async fn the_touch_of_save_with_tx_runs_on_that_transaction() {
    let db = fixture().await;
    let mut on_video = comment(2).await;

    let tx = DB::begin_transaction().await.unwrap();
    on_video.body = "in an explicit transaction".into();
    on_video.save_with_tx(&tx).await.unwrap();
    tx.rollback().await.unwrap();

    assert_eq!(
        updated_at(&db, "mt_videos").await,
        STALE,
        "the touch ran on the explicit transaction and rolled back with it"
    );
}
