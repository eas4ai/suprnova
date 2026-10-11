//! `where_relation` and its kin read a `MorphTo` relation through the morph
//! engine, as Laravel's `whereRelation` reaches `hasMorph($relation, ['*'])`
//! through `whereHas`: the column condition runs against every owner type
//! the relation declares, under each owner's own scopes.
//!
//! Every post the fixture keeps is titled `match`, so a global scope another
//! test registers on `GapPost` (it keeps that title) changes no result here.

use crate::laravel_gaps::scopes::{GapNote, GapPost, GapVideo};
use suprnova::sea_orm::DbBackend;
use suprnova::testing::TestDatabase;
use suprnova::{Builder, Collection, Model, attrs};

/// Notes on a post titled `match` (by its type string and by its alias), a
/// trashed post with the same title, a video titled `match`, a video with
/// another title, a missing post and a null type.
async fn fixture() -> TestDatabase {
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
    let trashed = GapPost::create(attrs! { title: "match" })
        .await
        .expect("trashed post");
    let video = GapVideo::create(attrs! { title: "match" })
        .await
        .expect("video");
    let other = GapVideo::create(attrs! { title: "other" })
        .await
        .expect("other video");
    for (body, kind, key) in [
        ("post", Some("gap_post"), Some(post.id)),
        ("alias", Some("GapPost"), Some(post.id)),
        ("trashed", Some("gap_post"), Some(trashed.id)),
        ("video", Some("gap_video"), Some(video.id)),
        ("other video", Some("gap_video"), Some(other.id)),
        ("dangling", Some("gap_post"), Some(999)),
        ("null", None, Some(post.id)),
    ] {
        GapNote::create(attrs! { body: body, subject_type: kind, subject_id: key })
            .await
            .expect("note");
    }
    trashed.delete().await.expect("trash the post");
    db
}

/// The bodies of `rows`, sorted, so a test reads the set it matched.
fn bodies(rows: Collection<GapNote>) -> Vec<String> {
    let mut bodies: Vec<String> = rows.iter().map(|row| row.body.clone()).collect();
    bodies.sort_unstable();
    bodies
}

#[tokio::test]
async fn where_relation_matches_the_owners_of_every_declared_type() {
    let _db = fixture().await;
    let rows = GapNote::query()
        .where_relation("subject", "title", "match")
        .get()
        .await
        .expect("where_relation");
    assert_eq!(bodies(rows), ["alias", "post", "video"]);
}

#[tokio::test]
async fn where_relation_op_compares_with_its_operator_on_every_owner_type() {
    let _db = fixture().await;
    let rows = GapNote::query()
        .where_relation_op("subject", "title", "!=", "match")
        .get()
        .await
        .expect("where_relation_op");
    assert_eq!(bodies(rows), ["other video"]);
    let rows = GapNote::query()
        .where_relation_op("subject", "title", "=", "match")
        .get()
        .await
        .expect("where_relation_op with =");
    assert_eq!(bodies(rows), ["alias", "post", "video"]);
}

#[tokio::test]
async fn or_where_relation_keeps_its_disjunction() {
    let _db = fixture().await;
    let rows = GapNote::query()
        .filter("body", "dangling")
        .or_where_relation("subject", "title", "other")
        .get()
        .await
        .expect("or_where_relation");
    assert_eq!(bodies(rows), ["dangling", "other video"]);
    // AND binds tighter than OR, so the second filter narrows only the
    // first row set and the relation clause stays one term.
    let rows = GapNote::query()
        .filter("body", "null")
        .filter("subject_type", "gap_video")
        .or_where_relation("subject", "title", "match")
        .get()
        .await
        .expect("or_where_relation after a conjunction");
    assert_eq!(bodies(rows), ["alias", "post", "video"]);
}

#[tokio::test]
async fn where_relation_refuses_an_operator_or_column_it_cannot_render() {
    let _db = fixture().await;
    let error = GapNote::query()
        .where_relation_op("subject", "title", "== 1 OR 1", "match")
        .get()
        .await
        .expect_err("an operator outside the allowlist");
    assert_eq!(error.status_code(), 400, "{error}");
    let error = GapNote::query()
        .where_relation("subject", "title; DROP TABLE gap_morph_notes", "match")
        .get()
        .await
        .expect_err("a column that is no identifier");
    assert_eq!(error.status_code(), 400, "{error}");
}

#[test]
fn where_relation_renders_the_morph_engines_query() {
    let render = |query: Builder<GapNote>| {
        query
            .try_to_sql_with_bindings_for(DbBackend::Postgres)
            .expect("render")
    };
    let relation = render(GapNote::query().where_relation("subject", "title", "match"));
    let morph = render(GapNote::query().where_has_morph(
        "subject",
        "*",
        |q: Builder<GapPost>, _| q.filter("title", "match"),
    ));
    assert_eq!(relation, morph);
    assert!(
        relation.0.contains("gap_morph_posts") && relation.0.contains("gap_morph_videos"),
        "{}",
        relation.0
    );
    assert!(!relation.0.contains("1 = 0"), "{}", relation.0);
}
