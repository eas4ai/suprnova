//! Eager loading through a `MorphTo` relation: `with(["commentable"])`
//! and the nested `with(["commentable.user"])`.
//!
//! The loader reads every comment's `commentable_type` and
//! `commentable_id`, loads each target type that is present with one
//! query, splits the loaded targets by type, and runs the rest of the path
//! through each type's own eager loader: one query per (target type,
//! relation) pair, never one per row. A target type of the family that
//! does not declare the next relation is an error that names the type and
//! the relation - whether or not a loaded row is of that type, since a
//! path that is silently left unloaded is an N+1 nobody sees.
//!
//! The query log is process-wide, so every test on these `mn_*` tables is
//! `#[serial]` and counts only the SELECTs that name its own tables.

use serial_test::serial;
use suprnova::eloquent::scopes::{GlobalScope, ScopeRegistry};
use suprnova::testing::TestDatabase;
use suprnova::{Builder, Collection, DB, Model, attrs, model};

#[model(table = "mn_people")]
pub struct MnPerson {
    pub id: i64,
    pub name: String,
}

#[model(table = "mn_posts", morph_type = "mn_post", relations = {
    user: BelongsTo<MnPerson> { fk = "mn_person_id" },
    reactions: MorphMany<MnReaction> { name = "reactable" },
})]
pub struct MnPost {
    pub id: i64,
    pub mn_person_id: i64,
    pub title: String,
}

#[model(table = "mn_videos", morph_type = "mn_video", relations = {
    user: BelongsTo<MnPerson> { fk = "mn_person_id" },
})]
pub struct MnVideo {
    pub id: i64,
    pub mn_person_id: i64,
    pub url: String,
}

/// A target with no `user` relation.
#[model(table = "mn_photos", morph_type = "mn_photo")]
pub struct MnPhoto {
    pub id: i64,
    pub url: String,
}

#[model(table = "mn_comments", relations = {
    commentable: MorphTo { targets = [MnPost, MnVideo] },
})]
pub struct MnComment {
    pub id: i64,
    pub commentable_id: i64,
    pub commentable_type: String,
    pub body: String,
}

/// A family where one target (`MnPhoto`) lacks the `user` relation.
#[model(table = "mn_reactions", relations = {
    reactable: MorphTo { targets = [MnPost, MnPhoto] },
})]
pub struct MnReaction {
    pub id: i64,
    pub reactable_id: i64,
    pub reactable_type: String,
    pub kind: String,
}

/// A family whose id and type columns are nullable, so a row can hold
/// the values the model refuses to write: a null id, a null type.
#[model(table = "mn_loose", relations = {
    target: MorphTo { targets = [MnPost, MnVideo] },
})]
pub struct MnLoose {
    pub id: i64,
    pub target_id: Option<i64>,
    pub target_type: Option<String>,
}

/// A target with a global scope that hides the rows titled `hidden...`.
#[model(table = "mn_scoped_posts", morph_type = "mn_scoped_post")]
pub struct MnScopedPost {
    pub id: i64,
    pub title: String,
}

pub struct MnHideHidden;

impl GlobalScope<MnScopedPost> for MnHideHidden {
    fn apply(&self, query: Builder<MnScopedPost>) -> Builder<MnScopedPost> {
        query.filter_not_like("title", "hidden%")
    }
}

/// A family whose first target has a global scope.
#[model(table = "mn_scoped_notes", relations = {
    subject: MorphTo { targets = [MnScopedPost, MnPhoto] },
})]
pub struct MnScopedNote {
    pub id: i64,
    pub subject_id: i64,
    pub subject_type: String,
    pub body: String,
}

async fn fixture() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    for ddl in [
        "CREATE TABLE mn_people (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
        "CREATE TABLE mn_posts (id INTEGER PRIMARY KEY AUTOINCREMENT, \
         mn_person_id INTEGER NOT NULL, title TEXT NOT NULL)",
        "CREATE TABLE mn_videos (id INTEGER PRIMARY KEY AUTOINCREMENT, \
         mn_person_id INTEGER NOT NULL, url TEXT NOT NULL)",
        "CREATE TABLE mn_photos (id INTEGER PRIMARY KEY AUTOINCREMENT, url TEXT NOT NULL)",
        "CREATE TABLE mn_comments (id INTEGER PRIMARY KEY AUTOINCREMENT, \
         commentable_id INTEGER NOT NULL, commentable_type TEXT NOT NULL, body TEXT NOT NULL)",
        "CREATE TABLE mn_reactions (id INTEGER PRIMARY KEY AUTOINCREMENT, \
         reactable_id INTEGER NOT NULL, reactable_type TEXT NOT NULL, kind TEXT NOT NULL)",
        "CREATE TABLE mn_loose (id INTEGER PRIMARY KEY AUTOINCREMENT, \
         target_id INTEGER, target_type TEXT)",
        "CREATE TABLE mn_scoped_posts (id INTEGER PRIMARY KEY AUTOINCREMENT, \
         title TEXT NOT NULL)",
        "CREATE TABLE mn_scoped_notes (id INTEGER PRIMARY KEY AUTOINCREMENT, \
         subject_id INTEGER NOT NULL, subject_type TEXT NOT NULL, body TEXT NOT NULL)",
    ] {
        db.execute_unprepared(ddl).await.unwrap();
    }
    db
}

/// Two people, a post and a video by each, and five comments. Post 1 is
/// ada's and video 1 is bob's, so a comment handed the target of the
/// other family would come back with the wrong owner. Each comment's body
/// is the name of the person who owns its target.
async fn seed_comments() {
    let ada = MnPerson::create(attrs! { name: "ada" }).await.unwrap();
    let bob = MnPerson::create(attrs! { name: "bob" }).await.unwrap();
    let ada_post = MnPost::create(attrs! { mn_person_id: ada.id, title: "ada-post" })
        .await
        .unwrap();
    let bob_post = MnPost::create(attrs! { mn_person_id: bob.id, title: "bob-post" })
        .await
        .unwrap();
    let bob_video = MnVideo::create(attrs! { mn_person_id: bob.id, url: "bob.mp4" })
        .await
        .unwrap();
    let ada_video = MnVideo::create(attrs! { mn_person_id: ada.id, url: "ada.mp4" })
        .await
        .unwrap();
    assert_eq!(ada_post.id, bob_video.id, "the families share ids");
    for (id, kind, body) in [
        (ada_post.id, "mn_post", "ada"),
        (bob_video.id, "mn_video", "bob"),
        (bob_post.id, "mn_post", "bob"),
        (ada_video.id, "mn_video", "ada"),
        (ada_post.id, "mn_post", "ada"),
    ] {
        MnComment::create(attrs! { commentable_id: id, commentable_type: kind, body: body })
            .await
            .unwrap();
    }
}

/// The type of each comment's target, in comment order: the order the
/// seed wrote them in.
const SEEDED_KINDS: [&str; 5] = ["post", "video", "post", "video", "post"];

/// Starts the query log empty.
fn start_query_log() {
    DB::enable_query_log().unwrap();
    DB::flush_query_log().unwrap();
}

/// For each of `tables`, the SELECTs logged since [`start_query_log`]
/// that read it; then the log switched off and emptied, so a failed
/// assertion after this call does not leave it running for the next test.
fn stop_query_log_counting_reads(tables: &[&str]) -> Vec<usize> {
    let selects: Vec<String> = DB::get_query_log()
        .unwrap()
        .into_iter()
        .map(|q| q.sql)
        .filter(|sql| sql.trim_start().to_uppercase().starts_with("SELECT"))
        .collect();
    DB::disable_query_log().unwrap();
    DB::flush_query_log().unwrap();
    tables
        .iter()
        .map(|table| selects.iter().filter(|sql| sql.contains(table)).count())
        .collect()
}

/// The kind of target and the owner's name each comment's loaded
/// `commentable` carries, when its `user` is loaded too.
fn loaded_targets(comments: &[MnComment]) -> Vec<(&'static str, Option<String>)> {
    comments
        .iter()
        .map(|comment| match comment.commentable_loaded() {
            Some(CommentableMorph::MnPost(post)) => {
                ("post", post.user_loaded().map(|p| p.name.clone()))
            }
            Some(CommentableMorph::MnVideo(video)) => {
                ("video", video.user_loaded().map(|p| p.name.clone()))
            }
            other => panic!("comment {} has no loaded target: {other:?}", comment.id),
        })
        .collect()
}

#[tokio::test]
#[serial]
async fn with_morph_to_loads_each_target_type_once() {
    let _db = fixture().await;
    seed_comments().await;

    start_query_log();
    let comments = MnComment::query()
        .order_by_asc("id")
        .with(["commentable"])
        .get()
        .await
        .unwrap();
    let reads = stop_query_log_counting_reads(&["mn_comments", "mn_posts", "mn_videos"]);

    assert_eq!(
        reads,
        vec![1, 1, 1],
        "one query for the comments and one per target type"
    );
    let kinds: Vec<&str> = loaded_targets(&comments)
        .into_iter()
        .map(|(kind, _)| kind)
        .collect();
    assert_eq!(kinds, SEEDED_KINDS);
    // For a target without a global scope, each loaded target is the one
    // the lazy path resolves.
    for comment in comments.iter() {
        let lazy = comment.commentable().get().await.unwrap();
        match (comment.commentable_loaded(), &lazy) {
            (Some(CommentableMorph::MnPost(a)), CommentableMorph::MnPost(b)) => {
                assert_eq!(a.id, b.id)
            }
            (Some(CommentableMorph::MnVideo(a)), CommentableMorph::MnVideo(b)) => {
                assert_eq!(a.id, b.id)
            }
            (eager, lazy) => panic!("eager {eager:?} and lazy {lazy:?} disagree"),
        }
    }
}

#[tokio::test]
#[serial]
async fn nested_path_through_morph_to_runs_one_query_per_type_and_relation() {
    let _db = fixture().await;
    seed_comments().await;

    start_query_log();
    let comments = MnComment::query()
        .order_by_asc("id")
        .with(["commentable.user"])
        .get()
        .await
        .unwrap();
    let reads =
        stop_query_log_counting_reads(&["mn_comments", "mn_posts", "mn_videos", "mn_people"]);

    assert_eq!(
        reads,
        vec![1, 1, 1, 2],
        "one query for the comments, one per target type, and one per \
         (target type, relation) pair - never one per comment"
    );
    assert_eq!(comments.len(), 5);
    for (comment, (_, owner)) in comments.iter().zip(loaded_targets(&comments)) {
        assert_eq!(
            owner.as_deref(),
            Some(comment.body.as_str()),
            "comment {} carries the owner of another row",
            comment.id,
        );
    }
}

#[tokio::test]
#[serial]
async fn load_missing_fills_only_the_tail_of_a_morph_path() {
    let _db = fixture().await;
    seed_comments().await;
    let mut comments = MnComment::query()
        .order_by_asc("id")
        .with(["commentable"])
        .get()
        .await
        .unwrap();

    start_query_log();
    comments.load_missing(["commentable.user"]).await.unwrap();
    let reads = stop_query_log_counting_reads(&["mn_posts", "mn_videos", "mn_people"]);

    assert_eq!(
        reads,
        vec![0, 0, 2],
        "the loaded targets are kept; only their owners are read"
    );
    for (comment, (_, owner)) in comments.iter().zip(loaded_targets(&comments)) {
        assert_eq!(owner.as_deref(), Some(comment.body.as_str()));
    }
}

#[tokio::test]
#[serial]
async fn a_target_without_the_next_relation_is_an_error() {
    let _db = fixture().await;
    let ada = MnPerson::create(attrs! { name: "ada" }).await.unwrap();
    let post = MnPost::create(attrs! { mn_person_id: ada.id, title: "p" })
        .await
        .unwrap();
    // Only a post is reacted to: no loaded row is an `MnPhoto`, and the
    // path is refused all the same.
    MnReaction::create(attrs! { reactable_id: post.id, reactable_type: "mn_post", kind: "like" })
        .await
        .unwrap();

    let Err(err) = MnReaction::query().with(["reactable.user"]).get().await else {
        panic!("`reactable.user` must be refused: MnPhoto has no `user` relation");
    };
    let message = err.to_string();
    assert!(message.contains("MnPhoto"), "names the target: {message}");
    assert!(message.contains("`user`"), "names the relation: {message}");

    // The flat path over the same family loads.
    let reactions = MnReaction::with(["reactable"]).get().await.unwrap();
    assert!(matches!(
        reactions[0].reactable_loaded(),
        Some(ReactableMorph::MnPost(p)) if p.id == post.id
    ));
}

#[tokio::test]
#[serial]
async fn lazy_and_eager_loads_differ_for_a_target_with_a_global_scope() {
    let _db = fixture().await;
    ScopeRegistry::register::<MnScopedPost, _>(MnHideHidden);
    let post = MnScopedPost::create(attrs! { title: "hidden-post" })
        .await
        .unwrap();
    MnScopedNote::create(
        attrs! { subject_id: post.id, subject_type: "mn_scoped_post", body: "note" },
    )
    .await
    .unwrap();
    assert!(
        MnScopedPost::query().first().await.unwrap().is_none(),
        "the scope hides the post from a query"
    );

    let notes = MnScopedNote::with(["subject"]).get().await.unwrap();
    assert!(
        matches!(
            notes[0].subject_loaded(),
            Some(SubjectMorph::Unknown(kind, id)) if kind == "mn_scoped_post" && *id == post.id
        ),
        "the eager load runs the target's query, so the scope hides the post: {:?}",
        notes[0].subject_loaded(),
    );

    let lazy = notes[0].subject().get().await.unwrap();
    assert!(
        matches!(&lazy, SubjectMorph::MnScopedPost(p) if p.id == post.id),
        "the lazy load finds the target by key and applies no global scope: {lazy:?}"
    );
}

#[tokio::test]
#[serial]
async fn a_nested_path_is_checked_against_the_family_when_no_row_loads() {
    let _db = fixture().await;

    let Err(err) = MnReaction::query().with(["reactable.user"]).get().await else {
        panic!("`reactable.user` must be refused on an empty table too: MnPhoto has no `user`");
    };
    let message = err.to_string();
    assert!(message.contains("MnPhoto"), "names the target: {message}");
    assert!(message.contains("`user`"), "names the relation: {message}");

    // The flat path over the empty table loads nothing and is not an error.
    let none = MnReaction::with(["reactable"]).get().await.unwrap();
    assert!(none.is_empty());
}

#[tokio::test]
#[serial]
async fn a_nested_path_through_a_relation_with_no_children_still_checks_the_family() {
    let _db = fixture().await;
    let ada = MnPerson::create(attrs! { name: "ada" }).await.unwrap();
    MnPost::create(attrs! { mn_person_id: ada.id, title: "no reactions" })
        .await
        .unwrap();

    let Err(err) = MnPost::query()
        .with(["reactions.reactable.user"])
        .get()
        .await
    else {
        panic!("the path must be refused although no post has a reaction");
    };
    assert!(
        err.to_string().contains("MnPhoto"),
        "names the target: {err}"
    );
}

#[tokio::test]
#[serial]
async fn load_on_an_empty_collection_checks_the_path_against_the_family() {
    let _db = fixture().await;
    let mut none: Collection<MnReaction> = Collection::from_vec(Vec::new());

    let Err(err) = none.load(["reactable.user"]).await else {
        panic!("`reactable.user` must be refused on an empty collection: MnPhoto has no `user`");
    };
    let message = err.to_string();
    assert!(message.contains("MnPhoto"), "names the target: {message}");
    assert!(message.contains("`user`"), "names the relation: {message}");

    // A flat path over no rows loads nothing and is not an error.
    none.load(["reactable"]).await.unwrap();
    assert!(none.is_empty());
}

#[tokio::test]
#[serial]
async fn load_missing_on_an_empty_collection_checks_the_path_against_the_family() {
    let _db = fixture().await;
    let mut none: Collection<MnReaction> = Collection::from_vec(Vec::new());

    let Err(err) = none.load_missing(["reactable.user"]).await else {
        panic!("`reactable.user` must be refused on an empty collection: MnPhoto has no `user`");
    };
    let message = err.to_string();
    assert!(message.contains("MnPhoto"), "names the target: {message}");
    assert!(message.contains("`user`"), "names the relation: {message}");

    none.load_missing(["reactable"]).await.unwrap();
    assert!(none.is_empty());
}

/// Each row's loaded `target`, as text: `post:<owner>` and `video:<owner>`
/// for a target with its `user` loaded, `unknown:<type>:<id>` for a row
/// that points at no target.
fn describe_loose(rows: &[MnLoose]) -> Vec<String> {
    rows.iter()
        .map(|row| match row.target_loaded() {
            Some(TargetMorph::MnPost(post)) => format!(
                "post:{}",
                post.user_loaded()
                    .map(|p| p.name.clone())
                    .unwrap_or_default()
            ),
            Some(TargetMorph::MnVideo(video)) => format!(
                "video:{}",
                video
                    .user_loaded()
                    .map(|p| p.name.clone())
                    .unwrap_or_default()
            ),
            Some(TargetMorph::Unknown(kind, id)) => format!("unknown:{kind}:{id}"),
            None => "not loaded".to_string(),
        })
        .collect()
}

#[tokio::test]
#[serial]
async fn unknown_values_keep_their_place_in_a_nested_path() {
    let db = fixture().await;
    let ada = MnPerson::create(attrs! { name: "ada" }).await.unwrap();
    let bob = MnPerson::create(attrs! { name: "bob" }).await.unwrap();
    let ada_post = MnPost::create(attrs! { mn_person_id: ada.id, title: "ada-post" })
        .await
        .unwrap();
    let bob_post = MnPost::create(attrs! { mn_person_id: bob.id, title: "bob-post" })
        .await
        .unwrap();
    let bob_video = MnVideo::create(attrs! { mn_person_id: bob.id, url: "bob.mp4" })
        .await
        .unwrap();
    // Known targets and values that name none, interleaved: a dangling
    // id, a type no target has, a null id and type, a null id.
    for (id, target_id, target_type) in [
        (1, ada_post.id.to_string(), "'mn_post'"),
        (2, "999".to_string(), "'mn_video'"),
        (3, bob_video.id.to_string(), "'mn_video'"),
        (4, "1".to_string(), "'mn_nothing'"),
        (5, "NULL".to_string(), "NULL"),
        (6, bob_post.id.to_string(), "'mn_post'"),
        (7, "NULL".to_string(), "'mn_post'"),
    ] {
        db.execute_unprepared(&format!(
            "INSERT INTO mn_loose (id, target_id, target_type) \
             VALUES ({id}, {target_id}, {target_type})"
        ))
        .await
        .unwrap();
    }
    let expected = vec![
        "post:ada",
        "unknown:mn_video:999",
        "video:bob",
        "unknown:mn_nothing:1",
        "unknown::null",
        "post:bob",
        "unknown:mn_post:null",
    ];

    let rows = MnLoose::query()
        .order_by_asc("id")
        .with(["target.user"])
        .get()
        .await
        .unwrap();
    assert_eq!(describe_loose(&rows), expected, "with(...)");

    let mut rows = MnLoose::query()
        .order_by_asc("id")
        .with(["target"])
        .get()
        .await
        .unwrap();
    rows.load_missing(["target.user"]).await.unwrap();
    assert_eq!(describe_loose(&rows), expected, "load_missing(...)");
}
