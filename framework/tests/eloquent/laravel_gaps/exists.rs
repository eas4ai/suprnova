use crate::eager::{EgPost, EgUser};
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
