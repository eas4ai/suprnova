use crate::eager::{EgPost, EgUser};
use crate::relations_morph::{MorphComment, MorphPost};
use crate::relations_through::{HoMembership, HoProfile, HoUser, ThCountry, ThPost, ThUser};
use suprnova::testing::TestDatabase;
use suprnova::{Model, attrs, model, when_exists_loaded};

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

/// The `MorphTo` probe asks the owner types `commentable` declares, so the
/// schema holds only that family's tables.
async fn morph_fixture() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.expect("database");
    for sql in [
        "CREATE TABLE morph_posts (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL)",
        "CREATE TABLE morph_videos (id INTEGER PRIMARY KEY AUTOINCREMENT, url TEXT NOT NULL)",
        "CREATE TABLE morph_comments (id INTEGER PRIMARY KEY AUTOINCREMENT, commentable_id INTEGER NOT NULL, commentable_type TEXT NOT NULL, body TEXT NOT NULL)",
    ] {
        db.execute_unprepared(sql).await.expect("schema");
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

async fn through_fixture() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.expect("database");
    for sql in [
        "CREATE TABLE th_countries (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
        "CREATE TABLE th_users (id INTEGER PRIMARY KEY AUTOINCREMENT, th_country_id INTEGER NOT NULL, name TEXT NOT NULL)",
        "CREATE TABLE th_posts (id INTEGER PRIMARY KEY AUTOINCREMENT, th_user_id INTEGER NOT NULL, title TEXT NOT NULL, views INTEGER NOT NULL DEFAULT 0)",
    ] {
        db.execute_unprepared(sql).await.expect("schema");
    }
    db
}

/// Countries named after what they reach: "posts" has a user with two
/// posts, "silent" a user without posts, "empty" no user. User 1 lives in
/// country 2 and user 2 in country 1, so a probe that matches a user's id
/// against a country's id gives the wrong countries.
async fn seed_countries() {
    for name in ["silent", "posts", "empty"] {
        ThCountry::create(attrs! { name: name })
            .await
            .expect("country");
    }
    for (country, name) in [(2, "writer"), (1, "quiet")] {
        ThUser::create(attrs! { th_country_id: country, name: name })
            .await
            .expect("user");
    }
    for title in ["first", "second"] {
        ThPost::create(attrs! { th_user_id: 1, title: title, views: 0 })
            .await
            .expect("post");
    }
}

fn names(countries: &[ThCountry]) -> Vec<&str> {
    let mut names: Vec<&str> = countries.iter().map(|row| row.name.as_str()).collect();
    names.sort_unstable();
    names
}

#[tokio::test]
async fn existence_flags_load_for_a_has_many_through_relation() {
    let _db = through_fixture().await;
    assert!(
        ThCountry::query()
            .with_exists("posts")
            .get()
            .await
            .expect("no rows")
            .is_empty()
    );
    seed_countries().await;
    let countries = ThCountry::query()
        .with_exists("posts")
        .order_by_asc("id")
        .get()
        .await
        .expect("existence flags");
    let flags: Vec<(&str, Option<bool>)> = countries
        .iter()
        .map(|row| (row.name.as_str(), row.__eager.get_exists("posts")))
        .collect();
    assert_eq!(
        flags,
        [
            ("silent", Some(false)),
            ("posts", Some(true)),
            ("empty", Some(false)),
        ]
    );
    assert!(countries.iter().all(|row| !row.__eager.has("posts")));
}

#[tokio::test]
async fn has_many_through_existence_queries_correlate_through_the_intermediate() {
    let _db = through_fixture().await;
    seed_countries().await;
    let has = ThCountry::query().has("posts").get().await.expect("has");
    assert_eq!(names(&has), ["posts"]);
    let missing = ThCountry::query()
        .doesnt_have("posts")
        .get()
        .await
        .expect("doesnt_have");
    assert_eq!(names(&missing), ["empty", "silent"]);
    let two = ThCountry::query()
        .has_count("posts", ">=", 2)
        .get()
        .await
        .expect("has_count");
    assert_eq!(names(&two), ["posts"]);
    let three = ThCountry::query()
        .has_count("posts", ">=", 3)
        .get()
        .await
        .expect("has_count above");
    assert!(three.is_empty());
    let second = ThCountry::query()
        .where_has::<ThPost, _>("posts", |q| q.filter("title", "second"))
        .get()
        .await
        .expect("where_has");
    assert_eq!(names(&second), ["posts"]);
    let none = ThCountry::query()
        .where_has::<ThPost, _>("posts", |q| q.filter("title", "third"))
        .get()
        .await
        .expect("where_has without a match");
    assert!(none.is_empty());
    let related = ThCountry::query()
        .where_relation("posts", "title", "first")
        .get()
        .await
        .expect("where_relation");
    assert_eq!(names(&related), ["posts"]);
}

async fn one_through_fixture() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.expect("database");
    for sql in [
        "CREATE TABLE ho_users (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
        "CREATE TABLE ho_memberships (id INTEGER PRIMARY KEY AUTOINCREMENT, ho_user_id INTEGER NOT NULL)",
        "CREATE TABLE ho_profiles (id INTEGER PRIMARY KEY AUTOINCREMENT, ho_membership_id INTEGER NOT NULL, bio TEXT NOT NULL)",
    ] {
        db.execute_unprepared(sql).await.expect("schema");
    }
    db
}

#[tokio::test]
async fn has_one_through_existence_reads_the_intermediate() {
    let _db = one_through_fixture().await;
    for name in ["member", "profiled"] {
        HoUser::create(attrs! { name: name }).await.expect("user");
    }
    // Membership 1 belongs to user 2, membership 2 to user 1, so only the
    // intermediate tells which user the profile reaches.
    for user in [2, 1] {
        HoMembership::create(attrs! { ho_user_id: user })
            .await
            .expect("membership");
    }
    HoProfile::create(attrs! { ho_membership_id: 1, bio: "bio" })
        .await
        .expect("profile");
    HoUser::create(attrs! { name: "alone" })
        .await
        .expect("user");
    let users = HoUser::query()
        .with_exists("profile")
        .order_by_asc("id")
        .get()
        .await
        .expect("existence flags");
    let flags: Vec<(&str, Option<bool>)> = users
        .iter()
        .map(|row| (row.name.as_str(), row.__eager.get_exists("profile")))
        .collect();
    assert_eq!(
        flags,
        [
            ("member", Some(false)),
            ("profiled", Some(true)),
            ("alone", Some(false)),
        ]
    );
    let has: Vec<String> = HoUser::query()
        .has("profile")
        .get()
        .await
        .expect("has")
        .iter()
        .map(|row| row.name.clone())
        .collect();
    assert_eq!(has, ["profiled"]);
    let mut missing: Vec<String> = HoUser::query()
        .doesnt_have("profile")
        .get()
        .await
        .expect("doesnt_have")
        .iter()
        .map(|row| row.name.clone())
        .collect();
    missing.sort_unstable();
    assert_eq!(missing, ["alone", "member"]);
}

/// A region reaches its desks through offices on custom keys: the region's
/// `code`, the office's `region_code` and its `uid` primary key.
#[model(table = "gap_regions", relations = {
    desks: HasManyThrough<GapOffice, GapDesk> {
        first_key = "region_code",
        second_key = "office_uid",
        lk = "code",
    },
})]
pub struct GapRegion {
    /// The database assigns the key.
    pub id: i64,
    /// The offices name the region by this code.
    pub code: String,
}

/// A soft-deleting intermediate keyed on `uid`.
#[model(table = "gap_offices", primary_key = "uid", soft_deletes)]
pub struct GapOffice {
    /// The database assigns the key.
    pub uid: i64,
    /// The region's code.
    pub region_code: String,
    /// A tombstone hides the office and what it reaches.
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// A soft-deleting target.
#[model(table = "gap_desks", soft_deletes)]
pub struct GapDesk {
    /// The database assigns the key.
    pub id: i64,
    /// The office's `uid`.
    pub office_uid: i64,
    /// The label names each case.
    pub label: String,
    /// A tombstone hides the desk.
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[tokio::test]
async fn through_existence_uses_declared_keys_and_leaves_out_trashed_rows() {
    let db = TestDatabase::sqlite_memory().await.expect("database");
    for sql in [
        "CREATE TABLE gap_regions (id INTEGER PRIMARY KEY AUTOINCREMENT, code TEXT NOT NULL)",
        "CREATE TABLE gap_offices (uid INTEGER PRIMARY KEY AUTOINCREMENT, region_code TEXT NOT NULL, deleted_at TEXT)",
        "CREATE TABLE gap_desks (id INTEGER PRIMARY KEY AUTOINCREMENT, office_uid INTEGER NOT NULL, label TEXT NOT NULL, deleted_at TEXT)",
    ] {
        db.execute_unprepared(sql).await.expect("schema");
    }
    for code in ["live", "closed office", "removed desk", "none"] {
        GapRegion::create(attrs! { code: code })
            .await
            .expect("region");
    }
    for code in ["live", "closed office", "removed desk"] {
        let office = GapOffice::create(attrs! { region_code: code })
            .await
            .expect("office");
        let desk = GapDesk::create(attrs! { office_uid: office.uid, label: code })
            .await
            .expect("desk");
        match code {
            "closed office" => office.delete().await.expect("trash office"),
            "removed desk" => desk.delete().await.expect("trash desk"),
            _ => {}
        }
    }
    let regions = GapRegion::query()
        .with_exists("desks")
        .order_by_asc("id")
        .get()
        .await
        .expect("existence flags");
    let flags: Vec<(&str, Option<bool>)> = regions
        .iter()
        .map(|row| (row.code.as_str(), row.__eager.get_exists("desks")))
        .collect();
    assert_eq!(
        flags,
        [
            ("live", Some(true)),
            ("closed office", Some(false)),
            ("removed desk", Some(false)),
            ("none", Some(false)),
        ]
    );
    let has: Vec<String> = GapRegion::query()
        .has("desks")
        .get()
        .await
        .expect("has")
        .iter()
        .map(|row| row.code.clone())
        .collect();
    assert_eq!(has, ["live"]);
    assert_eq!(
        GapRegion::query()
            .doesnt_have("desks")
            .count()
            .await
            .expect("doesnt_have"),
        3
    );
}
