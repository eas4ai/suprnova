use crate::eager::{EgPost, EgUser};
use crate::relations_morph::{MorphComment, MorphPost};
use crate::relations_through::ThCountry;
use suprnova::testing::TestDatabase;
use suprnova::{Model, attrs, when_exists_loaded};

async fn fixture() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    db.execute_unprepared("CREATE TABLE eg_users (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
        .await
        .unwrap();
    db.execute_unprepared("CREATE TABLE eg_posts (id INTEGER PRIMARY KEY, eg_user_id INTEGER NOT NULL, title TEXT NOT NULL, views INTEGER NOT NULL)").await.unwrap();
    EgUser::create(attrs! { id: 1, name: "with" })
        .await
        .unwrap();
    EgUser::create(attrs! { id: 2, name: "without" })
        .await
        .unwrap();
    EgPost::create(attrs! { id: 1, eg_user_id: 1, title: "post", views: 0 })
        .await
        .unwrap();
    db
}

#[tokio::test]
async fn existence_flags_do_not_load_rows_or_counts() {
    let _db = fixture().await;
    let query = EgUser::query().with_exists("posts");
    for users in [
        query.clone().get().await.unwrap(),
        query.get().await.unwrap(),
    ] {
        for user in &users {
            assert!(!user.__eager.has("posts"));
            assert_eq!(user.__eager.get_count("posts"), None);
            assert_eq!(user.__eager.get_exists("posts"), Some(user.id == 1));
            let value = when_exists_loaded(user, "posts", |exists| exists);
            assert_eq!(value.into_option(), Some(user.id == 1));
            let clone = user.clone();
            assert_eq!(clone.__eager.get_exists("posts"), Some(user.id == 1));
        }
    }
    let plain = EgUser::query().first().await.unwrap().unwrap();
    assert!(
        when_exists_loaded(&plain, "posts", |_| panic!("missing flags never evaluate"))
            .is_missing()
    );
    let both = EgUser::query()
        .with(["posts"])
        .with_count(["posts"])
        .with_exists("posts")
        .get()
        .await
        .unwrap();
    assert_eq!(both[0].posts_loaded().len() as u64, both[0].posts_count());
}

#[tokio::test]
async fn existence_flags_handle_empty_results_and_reject_unknown_relations() {
    let _db = fixture().await;
    assert!(
        EgUser::query()
            .filter("id", 99)
            .with_exists("posts")
            .get()
            .await
            .unwrap()
            .is_empty()
    );
    assert!(EgUser::query().with_exists("missing").get().await.is_err());
    assert!(
        EgUser::query()
            .filter("id", 99)
            .with_exists("missing")
            .get()
            .await
            .is_err()
    );
}

#[derive(Debug, Clone, suprnova::Data, suprnova::Validate)]
#[json_resource("users")]
struct ExistsResource {
    id: i64,
    posts_exists: suprnova::Maybe<bool>,
}

#[tokio::test]
async fn existence_resources_emit_loaded_false_and_omit_unloaded_flags() {
    let _db = fixture().await;
    suprnova::DB::enable_query_log().unwrap();
    suprnova::DB::flush_query_log().unwrap();
    let loaded = EgUser::query()
        .with_exists("posts")
        .order_by_asc("id")
        .get()
        .await
        .unwrap();
    let log = suprnova::DB::get_query_log().unwrap();
    suprnova::DB::disable_query_log().unwrap();
    suprnova::DB::flush_query_log().unwrap();
    assert_eq!(log.len(), 2, "existence is batched across parent rows");
    assert!(log[1].sql.contains("EXISTS"));
    assert!(
        !log.iter()
            .any(|query| query.sql.contains("FROM eg_posts") && !query.sql.contains("EXISTS"))
    );
    let plain = EgUser::query()
        .filter("id", 2)
        .first()
        .await
        .unwrap()
        .unwrap();
    for (user, expected) in [
        (&loaded[0], Some(true)),
        (&loaded[1], Some(false)),
        (&plain, None),
    ] {
        let resource = ExistsResource {
            id: user.id,
            posts_exists: when_exists_loaded(user, "posts", |exists| exists),
        };
        let response = suprnova::Resource::single(resource).render().await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(
            body["data"]["attributes"]
                .get("posts_exists")
                .and_then(serde_json::Value::as_bool),
            expected
        );
    }
}

/// The `MorphTo` probe asks every registered owner type, so the schema has a
/// table for each, as a complete application schema does.
async fn morph_fixture() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.expect("database");
    for sql in [
        "CREATE TABLE morph_posts (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL)",
        "CREATE TABLE morph_comments (id INTEGER PRIMARY KEY AUTOINCREMENT, commentable_id INTEGER NOT NULL, commentable_type TEXT NOT NULL, body TEXT NOT NULL)",
    ] {
        db.execute_unprepared(sql).await.expect("schema");
    }
    for entry in suprnova::morph_types() {
        let sql = format!(
            "CREATE TABLE IF NOT EXISTS {} ({} INTEGER PRIMARY KEY, deleted_at TEXT, removed_at TEXT)",
            entry.table, entry.primary_key
        );
        db.execute_unprepared(&sql).await.expect("registered table");
    }
    db
}

/// Save one comment pointing at `kind` and `key`, then reload it through
/// `with_exists("commentable")` so the flag comes from the probe.
async fn commentable_flag(kind: &str, key: i64) -> MorphComment {
    let comment = MorphComment::create(attrs! {
        commentable_id: key,
        commentable_type: kind,
        body: "comment",
    })
    .await
    .expect("comment");
    MorphComment::query()
        .filter("id", comment.id)
        .with_exists("commentable")
        .first()
        .await
        .expect("existence flag")
        .expect("the comment")
}

#[tokio::test]
async fn morph_to_existence_flag_is_true_for_an_existing_owner() {
    let _db = morph_fixture().await;
    let post = MorphPost::create(attrs! { title: "kept" })
        .await
        .expect("post");
    let comment = commentable_flag("post", post.id).await;
    assert_eq!(comment.__eager.get_exists("commentable"), Some(true));
    assert!(
        !comment.__eager.has("commentable"),
        "the flag leaves the relation unloaded"
    );
}

#[tokio::test]
async fn morph_to_existence_flag_is_false_for_a_deleted_owner() {
    let _db = morph_fixture().await;
    let post = MorphPost::create(attrs! { title: "removed" })
        .await
        .expect("post");
    let key = post.id;
    post.delete().await.expect("delete owner");
    let comment = commentable_flag("post", key).await;
    assert_eq!(comment.__eager.get_exists("commentable"), Some(false));
}

#[tokio::test]
async fn morph_to_existence_flag_is_false_for_an_unregistered_type() {
    let _db = morph_fixture().await;
    let post = MorphPost::create(attrs! { title: "kept" })
        .await
        .expect("post");
    let comment = commentable_flag("unregistered_owner", post.id).await;
    assert_eq!(comment.__eager.get_exists("commentable"), Some(false));
}

#[derive(Debug, Clone, suprnova::Data, suprnova::Validate)]
#[json_resource("comments")]
struct CommentResource {
    id: i64,
    commentable_exists: suprnova::Maybe<bool>,
}

#[tokio::test]
async fn when_exists_loaded_reads_a_morph_to_flag_as_loaded_true() {
    let _db = morph_fixture().await;
    let post = MorphPost::create(attrs! { title: "kept" })
        .await
        .expect("post");
    let comment = commentable_flag("post", post.id).await;
    let flag = when_exists_loaded(&comment, "commentable", |exists| exists);
    assert_eq!(flag.clone().into_option(), Some(true));
    let resource = CommentResource {
        id: comment.id,
        commentable_exists: flag,
    };
    let response = suprnova::Resource::single(resource)
        .render()
        .await
        .expect("render");
    let body: serde_json::Value = serde_json::from_slice(response.body()).expect("json body");
    assert_eq!(
        body["data"]["attributes"]["commentable_exists"],
        serde_json::Value::Bool(true)
    );
}

#[tokio::test]
async fn existence_flags_refuse_a_relation_kind_the_probe_cannot_read() {
    let db = TestDatabase::sqlite_memory().await.expect("database");
    db.execute_unprepared(
        "CREATE TABLE th_countries (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
    )
    .await
    .expect("schema");
    for rows in [0, 1] {
        if rows == 1 {
            ThCountry::create(attrs! { name: "one" })
                .await
                .expect("country");
        }
        let error = ThCountry::query()
            .with_exists("posts")
            .get()
            .await
            .expect_err("a through relation has no existence probe");
        let message = error.to_string();
        assert!(
            message.contains("`posts`") && message.contains("HasManyThrough"),
            "{message}"
        );
    }
}
