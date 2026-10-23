//! The existence filters read a `MorphTo` relation through the morph
//! engine: `has` and its kin ask every owner type the relation declares,
//! as Laravel's `has` routes a `MorphTo` to `hasMorph($relation, ['*'])`,
//! and a typed `where_has` predicate narrows to its own model's owners.

use crate::eager::{EgPost, EgUser};
use crate::laravel_gaps::scopes::{GapNote, GapPost, GapVideo};
use crate::relations_morph::{MorphComment, MorphImage, MorphPost, MorphVideo};
use suprnova::sea_orm::DbBackend;
use suprnova::testing::TestDatabase;
use suprnova::{Builder, Collection, Model, attrs};

/// Comments on a kept post, a deleted post, a post with another title, a
/// video and an unregistered type. The unregistered comment points at the
/// kept post's key, so only its type keeps it from counting.
async fn fixture() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.expect("database");
    for sql in [
        "CREATE TABLE morph_posts (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL)",
        "CREATE TABLE morph_videos (id INTEGER PRIMARY KEY AUTOINCREMENT, url TEXT NOT NULL)",
        "CREATE TABLE morph_comments (id INTEGER PRIMARY KEY AUTOINCREMENT, commentable_id INTEGER NOT NULL, commentable_type TEXT NOT NULL, body TEXT NOT NULL)",
    ] {
        db.execute_unprepared(sql).await.expect("schema");
    }
    let kept = MorphPost::create(attrs! { title: "kept" })
        .await
        .expect("kept post");
    let removed = MorphPost::create(attrs! { title: "removed" })
        .await
        .expect("removed post");
    let other = MorphPost::create(attrs! { title: "other" })
        .await
        .expect("other post");
    let video = MorphVideo::create(attrs! { url: "clip.mp4" })
        .await
        .expect("video");
    for (body, kind, key) in [
        ("on kept post", "post", kept.id),
        ("on deleted post", "post", removed.id),
        ("on other post", "post", other.id),
        ("on video", "video", video.id),
        ("unregistered", "unregistered_owner", kept.id),
    ] {
        MorphComment::create(attrs! {
            commentable_id: key,
            commentable_type: kind,
            body: body,
        })
        .await
        .expect("comment");
    }
    removed.delete().await.expect("delete the post");
    db
}

/// The bodies of `rows`, sorted, so a test reads the set it matched.
fn bodies(rows: Collection<MorphComment>) -> Vec<String> {
    let mut bodies: Vec<String> = rows.iter().map(|row| row.body.clone()).collect();
    bodies.sort_unstable();
    bodies
}

const WITH_OWNER: [&str; 3] = ["on kept post", "on other post", "on video"];
const WITHOUT_OWNER: [&str; 2] = ["on deleted post", "unregistered"];

#[tokio::test]
async fn has_and_doesnt_have_split_comments_by_their_owner() {
    let _db = fixture().await;
    let has = MorphComment::query()
        .has("commentable")
        .get()
        .await
        .expect("has");
    assert_eq!(bodies(has), WITH_OWNER);
    let missing = MorphComment::query()
        .doesnt_have("commentable")
        .get()
        .await
        .expect("doesnt_have");
    assert_eq!(bodies(missing), WITHOUT_OWNER);
}

#[tokio::test]
async fn or_forms_keep_their_disjunction() {
    let _db = fixture().await;
    let rows = MorphComment::query()
        .filter("body", "unregistered")
        .or_has("commentable")
        .get()
        .await
        .expect("or_has");
    assert_eq!(
        bodies(rows),
        ["on kept post", "on other post", "on video", "unregistered"]
    );
    let rows = MorphComment::query()
        .filter("body", "on video")
        .or_doesnt_have("commentable")
        .get()
        .await
        .expect("or_doesnt_have");
    assert_eq!(
        bodies(rows),
        ["on deleted post", "on video", "unregistered"]
    );
    // AND binds tighter than OR, so the second filter narrows only the
    // first row set and the existence clause stays one term.
    let rows = MorphComment::query()
        .filter("body", "on deleted post")
        .filter("commentable_type", "video")
        .or_has("commentable")
        .get()
        .await
        .expect("or_has after a conjunction");
    assert_eq!(bodies(rows), WITH_OWNER);
}

#[tokio::test]
async fn has_count_compares_the_owner_count() {
    let _db = fixture().await;
    let every = [
        "on deleted post",
        "on kept post",
        "on other post",
        "on video",
        "unregistered",
    ];
    let cases: [(&str, i64, &[&str]); 7] = [
        ("=", 1, &WITH_OWNER),
        (">=", 1, &WITH_OWNER),
        ("=", 0, &WITHOUT_OWNER),
        ("<", 1, &WITHOUT_OWNER),
        ("!=", 1, &WITHOUT_OWNER),
        (">", 1, &[]),
        (">=", 0, &every),
    ];
    for (op, count, expected) in cases {
        let rows = MorphComment::query()
            .has_count("commentable", op, count)
            .get()
            .await
            .expect("has_count");
        assert_eq!(bodies(rows), expected, "{op} {count}");
    }
}

#[tokio::test]
async fn has_count_refuses_an_operator_that_does_not_compare_counts() {
    let _db = fixture().await;
    for op in ["LIKE", "== 1 OR 1"] {
        let error = MorphComment::query()
            .has_count("commentable", op, 1)
            .get()
            .await
            .expect_err("a count is compared, not matched");
        let message = error.to_string();
        assert!(
            message.contains("`commentable`") && message.contains(op),
            "{message}"
        );
    }
}

#[tokio::test]
async fn where_has_applies_a_typed_predicate_to_that_types_owners() {
    let _db = fixture().await;
    let kept = MorphComment::query()
        .where_has::<MorphPost, _>("commentable", |q| q.filter("title", "kept"))
        .get()
        .await
        .expect("where_has");
    assert_eq!(bodies(kept), ["on kept post"]);
    let rest = MorphComment::query()
        .where_doesnt_have::<MorphPost, _>("commentable", |q| q.filter("title", "kept"))
        .get()
        .await
        .expect("where_doesnt_have");
    assert_eq!(
        bodies(rest),
        [
            "on deleted post",
            "on other post",
            "on video",
            "unregistered"
        ]
    );
    let video = MorphComment::query()
        .where_has::<MorphVideo, _>("commentable", |q| q.filter("url", "clip.mp4"))
        .get()
        .await
        .expect("where_has on the second owner type");
    assert_eq!(bodies(video), ["on video"]);
    let rows = MorphComment::query()
        .filter("body", "unregistered")
        .or_where_has::<MorphPost, _>("commentable", |q| q.filter("title", "kept"))
        .get()
        .await
        .expect("or_where_has");
    assert_eq!(bodies(rows), ["on kept post", "unregistered"]);
    let rows = MorphComment::query()
        .filter("body", "on kept post")
        .or_where_doesnt_have::<MorphPost, _>("commentable", |q| q.filter("title", "other"))
        .get()
        .await
        .expect("or_where_doesnt_have");
    assert_eq!(
        bodies(rows),
        [
            "on deleted post",
            "on kept post",
            "on video",
            "unregistered"
        ]
    );
}

#[tokio::test]
async fn where_has_refuses_a_model_that_owns_no_row_of_the_relation() {
    let _db = fixture().await;
    let error = MorphComment::query()
        .where_has::<MorphImage, _>("commentable", |q| q)
        .get()
        .await
        .expect_err("an image is no owner type of `commentable`");
    let message = error.to_string();
    assert!(
        message.contains("MorphImage") && message.contains("`commentable`"),
        "{message}"
    );
}

#[test]
fn the_family_renders_the_morph_engines_queries() {
    let render = |query: Builder<MorphComment>| {
        query
            .try_to_sql_with_bindings_for(DbBackend::Postgres)
            .expect("render")
    };
    let has = render(MorphComment::query().has("commentable"));
    assert_eq!(
        has,
        render(MorphComment::query().has_morph("commentable", "*"))
    );
    assert!(
        has.0.contains("morph_posts") && has.0.contains("morph_videos"),
        "{}",
        has.0
    );
    assert!(!has.0.contains("1 = 0"), "{}", has.0);
    let typed = render(
        MorphComment::query()
            .where_has::<MorphPost, _>("commentable", |q| q.filter("title", "kept")),
    );
    let morph = render(MorphComment::query().where_has_morph(
        "commentable",
        ["post"],
        |q: Builder<MorphPost>, _| q.filter("title", "kept"),
    ));
    assert_eq!(typed, morph);
}

#[tokio::test]
async fn a_trashed_owner_an_alias_and_a_null_type_follow_the_owner_scopes() {
    let db = TestDatabase::sqlite_memory().await.expect("database");
    for sql in [
        "CREATE TABLE gap_morph_posts (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL, deleted_at TEXT)",
        "CREATE TABLE gap_morph_videos (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL)",
        "CREATE TABLE gap_morph_notes (id INTEGER PRIMARY KEY AUTOINCREMENT, subject_id INTEGER, subject_type TEXT, body TEXT NOT NULL)",
    ] {
        db.execute_unprepared(sql).await.expect("schema");
    }
    let post = GapPost::create(attrs! { title: "match" })
        .await
        .expect("post");
    GapVideo::create(attrs! { title: "other" })
        .await
        .expect("video");
    for (body, kind, key) in [
        ("post", Some("gap_post"), Some(1)),
        ("alias", Some("GapPost"), Some(1)),
        ("video", Some("gap_video"), Some(1)),
        ("dangling", Some("gap_post"), Some(999)),
        ("no key", Some("gap_post"), None),
        ("null", None, Some(1)),
    ] {
        GapNote::create(attrs! { body: body, subject_type: kind, subject_id: key })
            .await
            .expect("note");
    }
    let notes = |rows: Collection<GapNote>| {
        let mut bodies: Vec<String> = rows.iter().map(|row| row.body.clone()).collect();
        bodies.sort_unstable();
        bodies
    };
    let has = GapNote::query().has("subject").get().await.expect("has");
    assert_eq!(notes(has), ["alias", "post", "video"]);
    let missing = GapNote::query()
        .doesnt_have("subject")
        .get()
        .await
        .expect("doesnt_have");
    assert_eq!(notes(missing), ["dangling", "no key", "null"]);
    let none = GapNote::query()
        .has_count("subject", "=", 0)
        .get()
        .await
        .expect("has_count = 0");
    assert_eq!(notes(none), ["dangling", "no key", "null"]);
    let matched = GapNote::query()
        .where_has::<GapPost, _>("subject", |q| q.filter("title", "match"))
        .get()
        .await
        .expect("where_has");
    assert_eq!(notes(matched), ["alias", "post"]);

    post.delete().await.expect("trash the post");
    let has = GapNote::query()
        .has("subject")
        .get()
        .await
        .expect("has after trash");
    assert_eq!(notes(has), ["video"]);
    let missing = GapNote::query()
        .doesnt_have("subject")
        .get()
        .await
        .expect("doesnt_have after trash");
    assert_eq!(
        notes(missing),
        ["alias", "dangling", "no key", "null", "post"]
    );
    let matched = GapNote::query()
        .where_has::<GapPost, _>("subject", |q| q.filter("title", "match"))
        .count()
        .await
        .expect("where_has after trash");
    assert_eq!(matched, 0);
}

#[tokio::test]
async fn a_has_many_relation_keeps_its_correlated_exists() {
    let db = TestDatabase::sqlite_memory().await.expect("database");
    for sql in [
        "CREATE TABLE eg_users (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
        "CREATE TABLE eg_posts (id INTEGER PRIMARY KEY AUTOINCREMENT, eg_user_id INTEGER NOT NULL, title TEXT NOT NULL, views INTEGER NOT NULL)",
    ] {
        db.execute_unprepared(sql).await.expect("schema");
    }
    let writer = EgUser::create(attrs! { name: "writer" })
        .await
        .expect("writer");
    EgUser::create(attrs! { name: "reader" })
        .await
        .expect("reader");
    EgPost::create(attrs! { eg_user_id: writer.id, title: "post", views: 0 })
        .await
        .expect("post");
    assert_eq!(
        EgUser::query().has("posts").to_sql_for(DbBackend::Postgres),
        "SELECT * FROM eg_users WHERE EXISTS (SELECT 1 FROM eg_posts WHERE eg_posts.eg_user_id = eg_users.id)"
    );
    assert_eq!(
        EgUser::query()
            .has_count("posts", ">=", 2)
            .to_sql_for(DbBackend::Postgres),
        "SELECT * FROM eg_users WHERE (SELECT COUNT(*) FROM eg_posts WHERE eg_posts.eg_user_id = eg_users.id) >= 2"
    );
    let names = |rows: Collection<EgUser>| {
        let mut names: Vec<String> = rows.iter().map(|row| row.name.clone()).collect();
        names.sort_unstable();
        names
    };
    let titled = |q: Builder<EgPost>| q.filter("title", "post");
    for (query, expected) in [
        (EgUser::query().has("posts"), vec!["writer"]),
        (EgUser::query().doesnt_have("posts"), vec!["reader"]),
        (EgUser::query().has_count("posts", "=", 1), vec!["writer"]),
        (
            EgUser::query().where_has::<EgPost, _>("posts", titled),
            vec!["writer"],
        ),
        (
            EgUser::query().where_doesnt_have::<EgPost, _>("posts", titled),
            vec!["reader"],
        ),
        (
            EgUser::query().filter("name", "reader").or_has("posts"),
            vec!["reader", "writer"],
        ),
    ] {
        assert_eq!(names(query.get().await.expect("query")), expected);
    }
}
