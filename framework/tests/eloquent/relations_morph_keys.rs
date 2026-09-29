//! Polymorphic relations over targets whose primary key is not an `i64`.
//!
//! The id of a morph target travels as the JSON value of its key - the
//! value `Model::field_value` gives - and every morph relation binds it as
//! it is. A family of `String`, UUID or ULID keyed targets therefore
//! resolves lazily (`note.<rel>().get()`), eagerly (`with([...])`) and from
//! the parent side (`MorphMany` / `MorphOne`) exactly as an `i64` family
//! does.
//!
//! A family that mixes key types is refused at compile time, so no test in
//! this binary can hold one: the `compile_fail` doctest on
//! `suprnova::eloquent::relations::morph::MorphTargetsShareKey` pins it,
//! next to a twin that compiles once the keys agree.
//!
//! Each child names its relation after the key type, so the per-family
//! enums (`IntSubjectMorph`, `StrSubjectMorph`, ...) don't collide in this
//! module, while `name = "subject"` keeps the columns `subject_id` and
//! `subject_type` in every family.

use suprnova::testing::TestDatabase;
use suprnova::{Model, attrs, model};

// ---- i64 keys ------------------------------------------------------------

#[model(table = "mk_int_posts", morph_type = "mk_int_post")]
pub struct MkIntPost {
    pub id: i64,
    pub title: String,
}

#[model(table = "mk_int_videos", morph_type = "mk_int_video")]
pub struct MkIntVideo {
    pub id: i64,
    pub url: String,
}

#[model(table = "mk_int_notes", relations = {
    int_subject: MorphTo { name = "subject", targets = [MkIntPost, MkIntVideo] },
})]
pub struct MkIntNote {
    pub id: i64,
    pub subject_id: i64,
    pub subject_type: String,
    pub body: String,
}

// ---- String keys ---------------------------------------------------------

#[model(
    table = "mk_str_posts",
    key_type = "String",
    auto_increment = false,
    fillable = ["id", "title"],
    morph_type = "mk_str_post",
    relations = {
        notes: MorphMany<MkStrNote> { name = "subject" },
    },
)]
pub struct MkStrPost {
    pub id: String,
    pub title: String,
}

#[model(
    table = "mk_str_videos",
    key_type = "String",
    auto_increment = false,
    fillable = ["id", "url"],
    morph_type = "mk_str_video"
)]
pub struct MkStrVideo {
    pub id: String,
    pub url: String,
}

#[model(table = "mk_str_notes", relations = {
    str_subject: MorphTo { name = "subject", targets = [MkStrPost, MkStrVideo] },
})]
pub struct MkStrNote {
    pub id: i64,
    pub subject_id: String,
    pub subject_type: String,
    pub body: String,
}

// ---- UUID keys -----------------------------------------------------------

#[model(
    table = "mk_uuid_posts",
    primary_key = "id",
    key_type = "String",
    auto_increment = false,
    unique_id = "uuid",
    morph_type = "mk_uuid_post",
    relations = {
        notes: MorphMany<MkUuidNote> { name = "subject" },
    },
)]
pub struct MkUuidPost {
    pub id: String,
    pub title: String,
}

#[model(
    table = "mk_uuid_videos",
    primary_key = "id",
    key_type = "String",
    auto_increment = false,
    unique_id = "uuid",
    morph_type = "mk_uuid_video"
)]
pub struct MkUuidVideo {
    pub id: String,
    pub url: String,
}

#[model(table = "mk_uuid_notes", relations = {
    uuid_subject: MorphTo { name = "subject", targets = [MkUuidPost, MkUuidVideo] },
})]
pub struct MkUuidNote {
    pub id: i64,
    pub subject_id: String,
    pub subject_type: String,
    pub body: String,
}

// ---- ULID keys -----------------------------------------------------------

#[model(
    table = "mk_ulid_posts",
    primary_key = "id",
    key_type = "String",
    auto_increment = false,
    unique_id = "ulid",
    morph_type = "mk_ulid_post",
    relations = {
        first_note: MorphOne<MkUlidNote> { name = "subject" },
    },
)]
pub struct MkUlidPost {
    pub id: String,
    pub title: String,
}

#[model(
    table = "mk_ulid_videos",
    primary_key = "id",
    key_type = "String",
    auto_increment = false,
    unique_id = "ulid",
    morph_type = "mk_ulid_video"
)]
pub struct MkUlidVideo {
    pub id: String,
    pub url: String,
}

#[model(table = "mk_ulid_notes", relations = {
    ulid_subject: MorphTo { name = "subject", targets = [MkUlidPost, MkUlidVideo] },
})]
pub struct MkUlidNote {
    pub id: i64,
    pub subject_id: String,
    pub subject_type: String,
    pub body: String,
}

// ---- Fixture -------------------------------------------------------------

async fn fixture() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    for ddl in [
        "CREATE TABLE mk_int_posts (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL)",
        "CREATE TABLE mk_int_videos (id INTEGER PRIMARY KEY AUTOINCREMENT, url TEXT NOT NULL)",
        "CREATE TABLE mk_int_notes (id INTEGER PRIMARY KEY AUTOINCREMENT, \
         subject_id INTEGER NOT NULL, subject_type TEXT NOT NULL, body TEXT NOT NULL)",
        "CREATE TABLE mk_str_posts (id TEXT PRIMARY KEY, title TEXT NOT NULL)",
        "CREATE TABLE mk_str_videos (id TEXT PRIMARY KEY, url TEXT NOT NULL)",
        "CREATE TABLE mk_str_notes (id INTEGER PRIMARY KEY AUTOINCREMENT, \
         subject_id TEXT NOT NULL, subject_type TEXT NOT NULL, body TEXT NOT NULL)",
        "CREATE TABLE mk_uuid_posts (id TEXT PRIMARY KEY, title TEXT NOT NULL)",
        "CREATE TABLE mk_uuid_videos (id TEXT PRIMARY KEY, url TEXT NOT NULL)",
        "CREATE TABLE mk_uuid_notes (id INTEGER PRIMARY KEY AUTOINCREMENT, \
         subject_id TEXT NOT NULL, subject_type TEXT NOT NULL, body TEXT NOT NULL)",
        "CREATE TABLE mk_ulid_posts (id TEXT PRIMARY KEY, title TEXT NOT NULL)",
        "CREATE TABLE mk_ulid_videos (id TEXT PRIMARY KEY, url TEXT NOT NULL)",
        "CREATE TABLE mk_ulid_notes (id INTEGER PRIMARY KEY AUTOINCREMENT, \
         subject_id TEXT NOT NULL, subject_type TEXT NOT NULL, body TEXT NOT NULL)",
    ] {
        db.execute_unprepared(ddl).await.unwrap();
    }
    db
}

// ---- Tests ---------------------------------------------------------------

#[tokio::test]
async fn integer_keys_resolve_eagerly_as_they_do_lazily() {
    let _db = fixture().await;
    let post = MkIntPost::create(attrs! { title: "p" }).await.unwrap();
    let video = MkIntVideo::create(attrs! { url: "v.mp4" }).await.unwrap();
    // The post and the video share the id 1: only the type column tells
    // them apart.
    assert_eq!(post.id, video.id);
    for (id, kind, body) in [
        (post.id, "mk_int_post", "on post"),
        (video.id, "mk_int_video", "on video"),
        (999, "mk_int_post", "dangling"),
    ] {
        MkIntNote::create(attrs! { subject_id: id, subject_type: kind, body: body })
            .await
            .unwrap();
    }

    let notes = MkIntNote::query()
        .order_by_asc("id")
        .with(["int_subject"])
        .get()
        .await
        .unwrap();
    assert_eq!(notes.len(), 3);
    match notes[0].int_subject_loaded() {
        Some(IntSubjectMorph::MkIntPost(p)) => assert_eq!(p.title, "p"),
        other => panic!("expected the post, got {other:?}"),
    }
    match notes[1].int_subject_loaded() {
        Some(IntSubjectMorph::MkIntVideo(v)) => assert_eq!(v.url, "v.mp4"),
        other => panic!("expected the video, got {other:?}"),
    }
    match notes[2].int_subject_loaded() {
        Some(IntSubjectMorph::Unknown(kind, id)) => {
            assert_eq!(kind, "mk_int_post");
            assert_eq!(*id, 999);
        }
        other => panic!("expected Unknown for a missing row, got {other:?}"),
    }

    // The lazy path lands on the same variant.
    match notes[1].int_subject().get().await.unwrap() {
        IntSubjectMorph::MkIntVideo(v) => assert_eq!(v.id, video.id),
        other => panic!("expected the video, got {other:?}"),
    }
}

#[tokio::test]
async fn string_keys_resolve_lazily_eagerly_and_from_the_parent() {
    let _db = fixture().await;
    let post = MkStrPost::create(attrs! { id: "post-a", title: "a" })
        .await
        .unwrap();
    let video = MkStrVideo::create(attrs! { id: "video-b", url: "b.mp4" })
        .await
        .unwrap();
    for (id, kind, body) in [
        (post.id.clone(), "mk_str_post", "on post"),
        (video.id.clone(), "mk_str_video", "on video"),
        // The video's key under the post's type: no post has it.
        (video.id.clone(), "mk_str_post", "wrong family"),
    ] {
        MkStrNote::create(attrs! { subject_id: id, subject_type: kind, body: body })
            .await
            .unwrap();
    }
    let notes = MkStrNote::query().order_by_asc("id").get().await.unwrap();

    match notes[0].str_subject().get().await.unwrap() {
        StrSubjectMorph::MkStrPost(p) => assert_eq!(p.id, "post-a"),
        other => panic!("expected the post, got {other:?}"),
    }
    match notes[1].str_subject().get().await.unwrap() {
        StrSubjectMorph::MkStrVideo(v) => assert_eq!(v.url, "b.mp4"),
        other => panic!("expected the video, got {other:?}"),
    }
    match notes[2].str_subject().get().await.unwrap() {
        StrSubjectMorph::Unknown(kind, id) => {
            assert_eq!(kind, "mk_str_post");
            assert_eq!(
                id, "video-b",
                "the id comes back as the string the column holds"
            );
        }
        other => panic!("expected Unknown, got {other:?}"),
    }

    let eager = MkStrNote::query()
        .order_by_asc("id")
        .with(["str_subject"])
        .get()
        .await
        .unwrap();
    assert!(matches!(
        eager[0].str_subject_loaded(),
        Some(StrSubjectMorph::MkStrPost(p)) if p.id == "post-a"
    ));
    assert!(matches!(
        eager[1].str_subject_loaded(),
        Some(StrSubjectMorph::MkStrVideo(v)) if v.id == "video-b"
    ));
    assert!(matches!(
        eager[2].str_subject_loaded(),
        Some(StrSubjectMorph::Unknown(kind, _)) if kind == "mk_str_post"
    ));

    // The parent side binds its string key as it is, lazily and eagerly.
    let from_post = post.notes().get().await.unwrap();
    assert_eq!(from_post.len(), 1);
    assert_eq!(from_post[0].body, "on post");
    let posts = MkStrPost::with(["notes"]).get().await.unwrap();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].notes_loaded().len(), 1);
    assert_eq!(posts[0].notes_loaded()[0].body, "on post");
}

#[tokio::test]
async fn uuid_keys_resolve_lazily_eagerly_and_from_the_parent() {
    let _db = fixture().await;
    let post = MkUuidPost::create(attrs! { title: "u" }).await.unwrap();
    let video = MkUuidVideo::create(attrs! { url: "u.mp4" }).await.unwrap();
    assert_eq!(post.id.len(), 36, "the post has a generated UUID key");
    for (id, kind, body) in [
        (post.id.clone(), "mk_uuid_post", "on post"),
        (video.id.clone(), "mk_uuid_video", "on video"),
    ] {
        MkUuidNote::create(attrs! { subject_id: id, subject_type: kind, body: body })
            .await
            .unwrap();
    }
    let notes = MkUuidNote::query()
        .order_by_asc("id")
        .with(["uuid_subject"])
        .get()
        .await
        .unwrap();

    match notes[0].uuid_subject_loaded() {
        Some(UuidSubjectMorph::MkUuidPost(p)) => assert_eq!(p.id, post.id),
        other => panic!("expected the post, got {other:?}"),
    }
    match notes[1].uuid_subject_loaded() {
        Some(UuidSubjectMorph::MkUuidVideo(v)) => assert_eq!(v.id, video.id),
        other => panic!("expected the video, got {other:?}"),
    }
    match notes[0].uuid_subject().get().await.unwrap() {
        UuidSubjectMorph::MkUuidPost(p) => assert_eq!(p.title, "u"),
        other => panic!("expected the post, got {other:?}"),
    }

    let posts = MkUuidPost::with(["notes"]).get().await.unwrap();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].notes_loaded().len(), 1);
    assert_eq!(posts[0].notes_loaded()[0].body, "on post");
}

#[tokio::test]
async fn ulid_keys_resolve_lazily_eagerly_and_from_the_parent() {
    let _db = fixture().await;
    let post = MkUlidPost::create(attrs! { title: "l" }).await.unwrap();
    let video = MkUlidVideo::create(attrs! { url: "l.mp4" }).await.unwrap();
    assert_eq!(post.id.len(), 26, "the post has a generated ULID key");
    for (id, kind, body) in [
        (video.id.clone(), "mk_ulid_video", "on video"),
        (post.id.clone(), "mk_ulid_post", "on post"),
    ] {
        MkUlidNote::create(attrs! { subject_id: id, subject_type: kind, body: body })
            .await
            .unwrap();
    }
    let notes = MkUlidNote::query()
        .order_by_asc("id")
        .with(["ulid_subject"])
        .get()
        .await
        .unwrap();

    match notes[0].ulid_subject_loaded() {
        Some(UlidSubjectMorph::MkUlidVideo(v)) => assert_eq!(v.id, video.id),
        other => panic!("expected the video, got {other:?}"),
    }
    match notes[1].ulid_subject_loaded() {
        Some(UlidSubjectMorph::MkUlidPost(p)) => assert_eq!(p.id, post.id),
        other => panic!("expected the post, got {other:?}"),
    }
    match notes[0].ulid_subject().get().await.unwrap() {
        UlidSubjectMorph::MkUlidVideo(v) => assert_eq!(v.url, "l.mp4"),
        other => panic!("expected the video, got {other:?}"),
    }

    // `MorphOne` from a ULID-keyed parent.
    let first = post.first_note().first().await.unwrap();
    assert_eq!(first.map(|n| n.body), Some("on post".to_string()));
}
