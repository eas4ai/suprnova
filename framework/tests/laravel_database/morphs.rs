//! LDB-005: a polymorphic relation resolves the PHP class name Laravel
//! stores, `App\Models\Post`, as the model's `morph_type`, and the alias
//! `post` the model registers, on every read; writes store the
//! `morph_type`.
//!
//! The fixture's comments 1 and 2 and image 1 name post 1 by its class,
//! comments 3 and 4 name post 1 by the alias, and comment 5 and image 2
//! name post 3 by the alias.

use suprnova::{HasRoles, Model};

use crate::models::{
    CommentableMorph, ImageableMorph, LdbComment, LdbImage, LdbPost, LdbTag, LdbUser,
    create_tag_tables,
};
use crate::on_every_engine;
use crate::support::{self, Engine};

const CLASS: &str = "App\\Models\\Post";
const ALIAS: &str = "post";

fn sorted(mut ids: Vec<u64>) -> Vec<u64> {
    ids.sort_unstable();
    ids
}

/// The post a `MorphTo` resolved to, or a panic naming what it found.
fn post_of_comment(morph: &CommentableMorph) -> u64 {
    match morph {
        CommentableMorph::LdbPost(post) => post.id,
        CommentableMorph::Unknown(kind, id) => panic!("unresolved owner {kind} {id}"),
    }
}

fn post_of_image(morph: &ImageableMorph) -> u64 {
    match morph {
        ImageableMorph::LdbPost(post) => post.id,
        ImageableMorph::Unknown(kind, id) => panic!("unresolved owner {kind} {id}"),
    }
}

/// `MorphMany`, `MorphOne` and `MorphTo`, direct and eager, and `has`,
/// `with_count` and `with_max`, match rows holding the class or the alias.
async fn reads_accept_the_class_and_the_alias(engine: Engine) {
    let (db, fixture) = support::laravel(engine).await;
    assert_eq!((fixture.morph.class.as_str(), fixture.morph.alias.as_str()), (CLASS, ALIAS));
    let _bound = support::bind(&db.conn);
    let stored: Vec<String> = support::rows(&db.conn, "SELECT commentable_type FROM comments")
        .await
        .iter()
        .map(|row| support::text(row, "commentable_type"))
        .collect();
    assert!(stored.iter().any(|t| t == CLASS) && stored.iter().any(|t| t == ALIAS));

    // MorphMany and MorphOne, direct.
    let first = LdbPost::find(1u64).await.expect("find").expect("post 1");
    let third = LdbPost::find(3u64).await.expect("find").expect("post 3");
    let ids = |rows: Vec<LdbComment>| sorted(rows.iter().map(|c| c.id).collect());
    assert_eq!(
        ids(first.comments().get().await.expect("comments").into_vec()),
        [1, 2, 3, 4],
        "{engine:?}: post 1's comments under the class and the alias"
    );
    assert_eq!(
        ids(third.comments().get().await.expect("comments").into_vec()),
        [5]
    );
    assert_eq!(first.comments().count().await.expect("count"), 4);
    let image = |found: Option<LdbImage>| found.map(|image| image.id);
    assert_eq!(image(first.image().first().await.expect("image")), Some(1));
    assert_eq!(
        image(third.image().first().await.expect("image")),
        Some(2),
        "{engine:?}: post 3's image is stored under the alias"
    );

    // MorphMany and MorphOne, eager.
    let posts = LdbPost::with(["comments", "image"])
        .get()
        .await
        .expect("eager posts");
    for post in posts.iter() {
        let comments = sorted(post.comments_loaded().iter().map(|c| c.id).collect());
        let image = post.image_loaded().map(|image| image.id);
        match post.id {
            1 => assert_eq!((comments, image), (vec![1, 2, 3, 4], Some(1)), "{engine:?}"),
            3 => assert_eq!((comments, image), (vec![5], Some(2)), "{engine:?}"),
            other => panic!("post {other} is trashed or unknown"),
        }
    }

    // MorphTo, direct and eager.
    for comment in LdbComment::query().get().await.expect("comments").iter() {
        let expected = if comment.id == 5 { 3 } else { 1 };
        let owner = comment.commentable().get().await.expect("owner");
        assert_eq!(
            post_of_comment(&owner),
            expected,
            "{engine:?}: comment {} ({})",
            comment.id,
            comment.commentable_type
        );
    }
    for comment in LdbComment::with(["commentable"])
        .get()
        .await
        .expect("eager comments")
        .iter()
    {
        let expected = if comment.id == 5 { 3 } else { 1 };
        let owner = comment.commentable_loaded().expect("an eager owner");
        assert_eq!(post_of_comment(owner), expected, "{engine:?}: eager comment {}", comment.id);
    }
    for image in LdbImage::with(["imageable"])
        .get()
        .await
        .expect("eager images")
        .iter()
    {
        let expected = if image.id == 2 { 3 } else { 1 };
        let direct = image.imageable().get().await.expect("owner");
        assert_eq!(post_of_image(&direct), expected, "{engine:?}: image {}", image.id);
        let eager = image.imageable_loaded().expect("an eager owner");
        assert_eq!(post_of_image(eager), expected, "{engine:?}: eager image {}", image.id);
    }

    // `has`, `with_count` and `with_max` over the same rows.
    let with_comments = LdbPost::query().has("comments").get().await.expect("has");
    assert_eq!(
        sorted(with_comments.iter().map(|p| p.id).collect()),
        [1, 3],
        "{engine:?}: post 3's only comment is stored under the alias"
    );
    let counted = LdbPost::query()
        .with_count(["comments"])
        .with_max(("comments", "id"))
        .get()
        .await
        .expect("with_count");
    for post in counted.iter() {
        let max = post
            .__eager
            .get_aggregate::<Option<f64>>("comments_max_id")
            .copied()
            .flatten();
        match post.id {
            1 => assert_eq!((post.comments_count(), max), (4, Some(4.0)), "{engine:?}"),
            3 => assert_eq!((post.comments_count(), max), (1, Some(5.0)), "{engine:?}"),
            other => panic!("post {other} is trashed or unknown"),
        }
    }
}

on_every_engine!(reads_accept_the_class_and_the_alias =>
    ldb_005_morph_reads_accept_the_class_and_the_alias_sqlite,
    ldb_005_morph_reads_accept_the_class_and_the_alias_postgres,
    ldb_005_morph_reads_accept_the_class_and_the_alias_mysql);

/// A `MorphToMany` attach and a spatie assignment store the `morph_type`,
/// never the alias.
async fn writes_store_the_morph_type(engine: Engine) {
    let (db, _) = support::laravel(engine).await;
    create_tag_tables(&db.conn).await;
    let _bound = support::bind(&db.conn);

    let post = LdbPost::find(3u64).await.expect("find").expect("post 3");
    let tag = <LdbTag as Model>::create(suprnova::attrs! { name: "laravel" })
        .await
        .expect("a tag");
    post.tags().attach(tag.id).await.expect("attach");
    let stored = support::rows(&db.conn, "SELECT taggable_type FROM ldb_taggables").await;
    assert_eq!(
        support::text(&stored[0], "taggable_type"),
        CLASS,
        "{engine:?}: an attach stores the morph_type"
    );

    let abigail = LdbUser::find(2u64).await.expect("find").expect("user 2");
    abigail.assign_role("writer").await.expect("assign a role");
    abigail
        .give_permission_to("publish articles")
        .await
        .expect("give a permission");
    for table in ["model_has_roles", "model_has_permissions"] {
        let stored =
            support::rows(&db.conn, &format!("SELECT model_type FROM {table} WHERE model_id = 2"))
                .await;
        assert!(!stored.is_empty(), "{engine:?}: {table} has abigail's row");
        for row in &stored {
            assert_eq!(
                support::text(row, "model_type"),
                "App\\Models\\User",
                "{engine:?}: {table} stores the morph_type, not the alias `user`"
            );
        }
    }
}

on_every_engine!(writes_store_the_morph_type =>
    ldb_005_morph_writes_store_the_morph_type_sqlite,
    ldb_005_morph_writes_store_the_morph_type_postgres,
    ldb_005_morph_writes_store_the_morph_type_mysql);
