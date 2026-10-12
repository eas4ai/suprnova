//! Models declared where a migration's imports are in scope.
//!
//! `sea_orm_migration::prelude::*` brings `ExprTrait` into scope, and
//! `ExprTrait` has `max`, `is_null` and a few dozen other methods on every
//! type a query can bind, taking `self` by value. A by-value trait method
//! wins over an inherent `&self` method of the same name, so generated
//! code that wrote `n.max(0)` or `self.morph_id.is_null()` stopped
//! compiling (or picked the wrong method) in any module with that import.
//! The generated code names such methods by path; this module is the
//! guard, and it only has to compile and run once.

use sea_orm_migration::prelude::*;
use suprnova::testing::TestDatabase;
use suprnova::{Model, attrs, model};

#[model(table = "hy_authors", relations = {
    posts: HasMany<HyPost>,
})]
pub struct HyAuthor {
    pub id: i64,
    pub name: String,
}

#[model(table = "hy_posts", morph_type = "hy_post")]
pub struct HyPost {
    pub id: i64,
    pub hy_author_id: i64,
    pub title: String,
}

#[model(table = "hy_notes", relations = {
    subject: MorphTo { name = "subject", targets = [HyPost] },
})]
pub struct HyNote {
    pub id: i64,
    pub subject_id: i64,
    pub subject_type: String,
    pub body: String,
}

#[tokio::test]
async fn relations_work_beside_the_migration_prelude() {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    for ddl in [
        "CREATE TABLE hy_authors (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
        "CREATE TABLE hy_posts (id INTEGER PRIMARY KEY AUTOINCREMENT, \
         hy_author_id INTEGER NOT NULL, title TEXT NOT NULL)",
        "CREATE TABLE hy_notes (id INTEGER PRIMARY KEY AUTOINCREMENT, \
         subject_id INTEGER NOT NULL, subject_type TEXT NOT NULL, body TEXT NOT NULL)",
    ] {
        db.execute_unprepared(ddl).await.unwrap();
    }
    let ada = HyAuthor::create(attrs! { name: "Ada" }).await.unwrap();
    let post = HyPost::create(attrs! { hy_author_id: ada.id, title: "Notes" })
        .await
        .unwrap();
    HyNote::create(attrs! { subject_id: post.id, subject_type: "hy_post", body: "on post" })
        .await
        .unwrap();

    let authors = HyAuthor::with_count(["posts"]).get().await.unwrap();
    assert_eq!(authors[0].posts_count(), 1);

    let notes = HyNote::query().with(["subject"]).get().await.unwrap();
    match notes[0].subject_loaded() {
        Some(SubjectMorph::HyPost(p)) => assert_eq!(p.title, "Notes"),
        other => panic!("expected the post, got {other:?}"),
    }
    match notes[0].subject().get().await.unwrap() {
        SubjectMorph::HyPost(p) => assert_eq!(p.id, post.id),
        other => panic!("expected the post, got {other:?}"),
    }
    // `Expr` comes from the prelude; naming it keeps the import honest.
    let _ = Expr::col(Alias::new("id"));
}
