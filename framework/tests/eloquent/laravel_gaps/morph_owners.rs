//! A `MorphTo` existence query reads the owner types its relation declares.

use crate::relations_morph_keys::MkIntNote;
use suprnova::sea_orm::DbBackend;
use suprnova::testing::TestDatabase;
use suprnova::{Model, attrs, model};

/// A registered owner: its type string is its `morph_type`.
#[model(table = "gap_owned_articles", morph_type = "gap_owned_article")]
pub struct GapOwnedArticle {
    /// The database assigns the key.
    pub id: i64,
    /// A title gives the row content.
    pub title: String,
}

/// An owner with no `morph_type`: its type string is its snake-cased name.
#[model(table = "gap_owned_photos")]
pub struct GapOwnedPhoto {
    /// The database assigns the key.
    pub id: i64,
    /// A caption gives the row content.
    pub caption: String,
}

/// A remark points at an article or a photo.
#[model(table = "gap_owned_remarks", relations = {
    remarkable: MorphTo { targets = [GapOwnedArticle, GapOwnedPhoto] },
})]
pub struct GapOwnedRemark {
    /// The database assigns the key.
    pub id: i64,
    /// The owner's key.
    pub remarkable_id: i64,
    /// The owner's type string.
    pub remarkable_type: String,
    /// The body labels each case.
    pub body: String,
}

/// Only the family's own tables exist, so a probe that reads any other
/// owner table fails.
async fn fixture() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.expect("database");
    for sql in [
        "CREATE TABLE gap_owned_articles (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL)",
        "CREATE TABLE gap_owned_photos (id INTEGER PRIMARY KEY AUTOINCREMENT, caption TEXT NOT NULL)",
        "CREATE TABLE gap_owned_remarks (id INTEGER PRIMARY KEY AUTOINCREMENT, remarkable_id INTEGER NOT NULL, remarkable_type TEXT NOT NULL, body TEXT NOT NULL)",
    ] {
        db.execute_unprepared(sql).await.expect("schema");
    }
    db
}

async fn remark(body: &str, kind: &str, key: i64) -> GapOwnedRemark {
    GapOwnedRemark::create(attrs! { remarkable_id: key, remarkable_type: kind, body: body })
        .await
        .expect("remark")
}

#[tokio::test]
async fn existence_flag_reads_a_declared_owner_without_a_morph_type() {
    let _db = fixture().await;
    let article = GapOwnedArticle::create(attrs! { title: "a" })
        .await
        .expect("article");
    let photo = GapOwnedPhoto::create(attrs! { caption: "p" })
        .await
        .expect("photo");
    remark("photo", "gap_owned_photo", photo.id).await;
    remark("article", "gap_owned_article", article.id).await;
    remark("dangling photo", "gap_owned_photo", 999).await;
    remark("undeclared", "gap_owned_video", photo.id).await;
    let remarks = GapOwnedRemark::query()
        .with_exists("remarkable")
        .order_by_asc("id")
        .get()
        .await
        .expect("existence flags");
    let flags: Vec<(&str, Option<bool>)> = remarks
        .iter()
        .map(|row| (row.body.as_str(), row.__eager.get_exists("remarkable")))
        .collect();
    assert_eq!(
        flags,
        [
            ("photo", Some(true)),
            ("article", Some(true)),
            ("dangling photo", Some(false)),
            ("undeclared", Some(false)),
        ]
    );
}

#[tokio::test]
async fn wildcard_and_named_morph_queries_cover_the_declared_owners() {
    let _db = fixture().await;
    let photo = GapOwnedPhoto::create(attrs! { caption: "p" })
        .await
        .expect("photo");
    remark("photo", "gap_owned_photo", photo.id).await;
    remark("missing article", "gap_owned_article", 999).await;
    let bodies = |rows: suprnova::Collection<GapOwnedRemark>| {
        rows.iter().map(|row| row.body.clone()).collect::<Vec<_>>()
    };
    assert_eq!(
        bodies(
            GapOwnedRemark::query()
                .has_morph("remarkable", "*")
                .get()
                .await
                .expect("wildcard existence")
        ),
        ["photo"]
    );
    assert_eq!(
        bodies(
            GapOwnedRemark::query()
                .doesnt_have_morph("remarkable", "*")
                .get()
                .await
                .expect("wildcard absence")
        ),
        ["missing article"]
    );
    for name in ["gap_owned_photo", "GapOwnedPhoto"] {
        let rows = GapOwnedRemark::query()
            .where_has_morph("remarkable", [name], |q: suprnova::Builder<()>, kind| {
                assert_eq!(kind, "gap_owned_photo");
                q.filter("caption", "p")
            })
            .get()
            .await
            .expect("named owner");
        assert_eq!(bodies(rows), ["photo"], "{name}");
    }
    let refused = GapOwnedRemark::query()
        .has_morph("remarkable", ["gap_owned_video"])
        .get()
        .await
        .expect_err("a name no model answers to is still refused");
    let message = refused.to_string();
    assert!(
        message.contains("`gap_owned_video`") && message.contains("`remarkable`"),
        "{message}"
    );
}

#[test]
fn wildcard_sql_names_only_the_relations_own_owner_tables() {
    let (sql, _) = GapOwnedRemark::query()
        .has_morph("remarkable", "*")
        .try_to_sql_with_bindings_for(DbBackend::Postgres)
        .expect("render");
    assert!(sql.contains("gap_owned_articles"), "{sql}");
    assert!(sql.contains("gap_owned_photos"), "{sql}");
    let (sql, _) = MkIntNote::query()
        .has_morph("int_subject", "*")
        .try_to_sql_with_bindings_for(DbBackend::Postgres)
        .expect("render");
    assert!(
        sql.contains("mk_int_posts") && sql.contains("mk_int_videos"),
        "{sql}"
    );
    for foreign in [
        "mk_str_",
        "mk_uuid_",
        "mk_ulid_",
        "morph_posts",
        "gap_owned_",
    ] {
        assert!(!sql.contains(foreign), "{foreign} in {sql}");
    }
}
