//! Global scopes, morph existence and physical destruction.

use suprnova::testing::TestDatabase;
use suprnova::{Model, attrs, model};

/// A soft-deleting row exposes the scoped lookup regression.
#[model(table = "gap_scoped_rows", soft_deletes)]
pub struct ScopedRow {
    /// The database assigns the key.
    pub id: i64,
    /// The scope filters this tenant.
    pub tenant_id: i64,
    /// A tombstone distinguishes a soft delete.
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}

async fn soft_fixture() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.expect("database");
    db.execute_unprepared("CREATE TABLE gap_scoped_rows (id INTEGER PRIMARY KEY AUTOINCREMENT, tenant_id INTEGER NOT NULL, deleted_at TEXT)").await.expect("schema");
    db
}

#[tokio::test]
async fn force_destroy_includes_trashed_and_counts_existing_keys_once() {
    let _db = soft_fixture().await;
    let alive = ScopedRow::create(attrs! {tenant_id: 1})
        .await
        .expect("alive");
    let trashed = ScopedRow::create(attrs! {tenant_id: 1})
        .await
        .expect("trashed");
    let key = trashed.id;
    trashed.delete().await.expect("soft delete");
    assert!(ScopedRow::find(key).await.expect("find").is_none());
    assert_eq!(
        ScopedRow::force_destroy([alive.id, key, key, 999])
            .await
            .expect("force destroy"),
        2
    );
    assert_eq!(ScopedRow::with_trashed().count().await.expect("count"), 0);
    assert_eq!(
        ScopedRow::force_destroy(Vec::<i64>::new())
            .await
            .expect("empty"),
        0
    );
}

/// A fixed scope checks that inspection and execution use the same clauses.
struct TenantScope;
impl suprnova::eloquent::scopes::GlobalScope<ScopedRow> for TenantScope {
    fn apply(&self, q: suprnova::Builder<ScopedRow>) -> suprnova::Builder<ScopedRow> {
        q.filter("tenant_id", 1)
    }
}

#[tokio::test]
async fn apply_scopes_is_idempotent_and_keeps_opt_outs_and_or_groups() {
    let _db = soft_fixture().await;
    suprnova::eloquent::scopes::ScopeRegistry::register::<ScopedRow, _>(TenantScope);
    let row = ScopedRow::create(attrs! {tenant_id: 1}).await.expect("row");
    ScopedRow::create(attrs! {tenant_id: 2})
        .await
        .expect("other tenant");
    let deleted = ScopedRow::create(attrs! {tenant_id: 1})
        .await
        .expect("deleted");
    deleted.delete().await.expect("delete");
    let q = ScopedRow::query()
        .filter("id", row.id)
        .or_where("tenant_id", 2)
        .apply_scopes();
    let sql = q.to_sql_for(suprnova::sea_orm::DbBackend::Sqlite);
    assert!(sql.contains("tenant_id ="), "{sql}");
    assert!(sql.contains("deleted_at IS NULL"), "{sql}");
    assert_eq!(
        sql,
        q.clone()
            .apply_scopes()
            .to_sql_for(suprnova::sea_orm::DbBackend::Sqlite)
    );
    assert_eq!(q.get().await.expect("scoped").len(), 1);
    let q = ScopedRow::query()
        .with_trashed()
        .without_global_scopes()
        .apply_scopes();
    let sql = q.to_sql_for(suprnova::sea_orm::DbBackend::Sqlite);
    assert!(!sql.contains("deleted_at"), "{sql}");
    assert!(!sql.contains("WHERE"), "{sql}");
    assert_eq!(q.get().await.expect("unscoped").len(), 3);
    assert!(
        ScopedRow::query()
            .filter("bad;column", 1)
            .apply_scopes()
            .try_to_sql_with_bindings_for(suprnova::sea_orm::DbBackend::Sqlite)
            .is_err()
    );
}

/// A morph owner gives existence queries a registered table and key.
#[model(table = "gap_morph_posts", soft_deletes, morph_type = "gap_post", morph_aliases = ["GapPost"])]
pub struct GapPost {
    /// The database assigns the key.
    pub id: i64,
    /// Constraints match the title.
    pub title: String,
    /// A tombstone checks the selected owner's default scope.
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// A second owner detects accidental matches across equal numeric keys.
#[model(table = "gap_morph_videos", morph_type = "gap_video")]
pub struct GapVideo {
    /// The database assigns the key.
    pub id: i64,
    /// Constraints match the title.
    pub title: String,
}

/// Nullable morph columns exercise Laravel's wildcard absence condition.
#[model(table = "gap_morph_notes", relations = {
    subject: MorphTo { name = "subject", targets = [GapPost, GapVideo] },
})]
pub struct GapNote {
    /// The database assigns the key.
    pub id: i64,
    /// A missing owner can leave the key null.
    pub subject_id: Option<i64>,
    /// A missing relation can leave the type null.
    pub subject_type: Option<String>,
    /// The body labels each test case.
    pub body: String,
}

async fn morph_fixture() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.expect("database");
    for sql in [
        "CREATE TABLE gap_morph_posts (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL, deleted_at TEXT)",
        "CREATE TABLE gap_morph_videos (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL)",
        "CREATE TABLE gap_morph_notes (id INTEGER PRIMARY KEY AUTOINCREMENT, subject_id INTEGER, subject_type TEXT, body TEXT NOT NULL)",
    ] {
        db.execute_unprepared(sql).await.expect("schema");
    }
    GapPost::create(attrs! {title: "match"})
        .await
        .expect("post");
    GapVideo::create(attrs! {title: "other"})
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
        GapNote::create(attrs! {body: body, subject_type: kind, subject_id: key})
            .await
            .expect("note");
    }
    db
}

#[tokio::test]
async fn morph_existence_uses_type_and_key_and_constraints_per_type() {
    let _db = morph_fixture().await;
    assert_eq!(
        GapNote::query()
            .has_morph("subject", ["gap_post"])
            .count()
            .await
            .expect("has"),
        2
    );
    assert_eq!(
        GapNote::query()
            .doesnt_have_morph("subject", ["gap_post"])
            .count()
            .await
            .expect("missing"),
        2
    );
    let rows = GapNote::query()
        .where_has_morph(
            "subject",
            ["gap_post", "gap_video"],
            |q: suprnova::Builder<()>, kind| {
                assert!(["gap_post", "gap_video"].contains(&kind));
                q.filter("title", "match")
            },
        )
        .get()
        .await
        .expect("constraint");
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter()
            .all(|row| row.body == "post" || row.body == "alias")
    );
    assert_eq!(
        GapNote::query()
            .where_doesnt_have_morph("subject", ["gap_video"], |q: suprnova::Builder<()>, _| q
                .filter("title", "match"))
            .count()
            .await
            .expect("constrained missing"),
        1
    );
    assert_eq!(
        GapNote::query()
            .has_morph("subject", Vec::<String>::new())
            .count()
            .await
            .expect("empty"),
        0
    );
    assert_eq!(
        GapNote::query()
            .doesnt_have_morph("subject", Vec::<String>::new())
            .count()
            .await
            .expect("empty absence"),
        6
    );
    assert!(
        GapNote::query()
            .has_morph("subject", ["unknown"])
            .get()
            .await
            .is_err()
    );
    assert!(
        GapNote::query()
            .has_morph("missing", ["gap_post"])
            .get()
            .await
            .is_err()
    );
    assert!(
        GapNote::query()
            .where_has_morph("subject", ["gap_post"], |q: suprnova::Builder<()>, _| q
                .filter("bad;column", 1))
            .get()
            .await
            .is_err()
    );
    assert_eq!(
        GapNote::query()
            .has_morph("subject", ["GapPost"])
            .count()
            .await
            .expect("type alias"),
        2
    );
}

#[tokio::test]
async fn wildcard_morph_absence_includes_null_type() {
    let db = morph_fixture().await;
    // A complete application schema has a table for every registered model.
    for entry in suprnova::morph_types() {
        let sql = format!(
            "CREATE TABLE IF NOT EXISTS {} ({} INTEGER PRIMARY KEY, deleted_at TEXT, removed_at TEXT)",
            entry.table, entry.primary_key
        );
        db.execute_unprepared(&sql).await.expect("registered table");
    }
    let missing = GapNote::query()
        .doesnt_have_morph("subject", "*")
        .get()
        .await
        .expect("wildcard absence");
    assert_eq!(missing.len(), 3);
    assert!(missing.iter().any(|row| row.body == "null"));
    assert_eq!(
        GapNote::query()
            .has_morph("subject", "*")
            .count()
            .await
            .expect("wildcard existence"),
        3
    );
}

#[tokio::test]
async fn force_destroy_propagates_delete_failure_without_losing_the_row() {
    let db = soft_fixture().await;
    let row = ScopedRow::create(attrs! {tenant_id: 1}).await.expect("row");
    let key = row.id;
    row.delete().await.expect("trash");
    db.execute_unprepared("CREATE TRIGGER refuse_destroy BEFORE DELETE ON gap_scoped_rows BEGIN SELECT RAISE(ABORT, 'delete refused'); END").await.expect("trigger");
    assert!(ScopedRow::force_destroy([key]).await.is_err());
    assert_eq!(
        ScopedRow::with_trashed()
            .count()
            .await
            .expect("row remains"),
        1
    );
}

/// Select an owner's visible title to check target scopes inside existence queries.
struct PostTitleScope;
impl suprnova::GlobalScope<GapPost> for PostTitleScope {
    fn apply(&self, query: suprnova::Builder<GapPost>) -> suprnova::Builder<GapPost> {
        query.filter("title", "match")
    }
}

#[tokio::test]
async fn morph_existence_applies_each_targets_scopes_and_soft_delete_filter() {
    let _db = morph_fixture().await;
    suprnova::ScopeRegistry::register::<GapPost, _>(PostTitleScope);
    let hidden = GapPost::create(attrs! {title: "hidden"})
        .await
        .expect("hidden owner");
    let trashed = GapPost::create(attrs! {title: "match"})
        .await
        .expect("trashed owner");
    GapNote::create(attrs! {body: "hidden", subject_type: "gap_post", subject_id: hidden.id})
        .await
        .expect("hidden note");
    GapNote::create(attrs! {body: "trashed", subject_type: "gap_post", subject_id: trashed.id})
        .await
        .expect("trashed note");
    trashed.delete().await.expect("trash owner");
    assert_eq!(
        GapNote::query()
            .has_morph("subject", ["gap_post"])
            .count()
            .await
            .expect("visible owners"),
        2
    );
    assert_eq!(
        GapNote::query()
            .doesnt_have_morph("subject", ["gap_post"])
            .count()
            .await
            .expect("missing owners"),
        4
    );
    assert_eq!(
        GapNote::query()
            .has_morph("subject", ["GapVideo"])
            .count()
            .await
            .expect("Rust type name"),
        1
    );
}
