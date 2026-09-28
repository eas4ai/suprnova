//! Global scopes and the soft-delete filter are folded in when a query
//! runs, not when it is built. These tests pin what that buys:
//!
//! - the trashed views keep every registered scope, so a tenant's restore
//!   screen never lists another tenant's rows;
//! - an opt-out chained onto `Model::query()` lands, where it used to
//!   compile and do nothing;
//! - an `or_*` call cannot fold into a scope's term and widen past it;
//! - a mass update or delete reaches only the rows a read would return.
//!
//! One model, one constant tenant: the scope registry is process-wide and
//! the tests run in parallel, so nothing here changes what the scope reads.

use std::sync::Once;

use chrono::{DateTime, Utc};
use suprnova::eloquent::scopes::{GlobalScope, ScopeRegistry};
use suprnova::testing::TestDatabase;
use suprnova::{Builder, Model, attrs, model};

#[model(table = "rs_docs", soft_deletes, fillable = ["tenant_id", "title"])]
pub struct RsDoc {
    pub id: i64,
    pub tenant_id: i64,
    pub title: String,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Every query a tenant runs is limited to tenant 1's rows.
pub struct RsTenantOne;

impl GlobalScope<RsDoc> for RsTenantOne {
    fn apply(&self, query: Builder<RsDoc>) -> Builder<RsDoc> {
        query.filter("tenant_id", 1_i64)
    }
}

static REGISTER: Once = Once::new();

/// Tenant 1 owns `mine` and the trashed `mine-trashed`; tenant 2 owns
/// `theirs` and the trashed `theirs-trashed`.
async fn docs() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    db.execute_unprepared(
        "CREATE TABLE rs_docs (\
            id INTEGER PRIMARY KEY AUTOINCREMENT, \
            tenant_id INTEGER NOT NULL, \
            title TEXT NOT NULL, \
            deleted_at TEXT\
         )",
    )
    .await
    .unwrap();
    // Seeded before the scope exists for this process or around it: the
    // rows go in through plain SQL so no scope decides what is written.
    db.execute_unprepared(
        "INSERT INTO rs_docs (tenant_id, title, deleted_at) VALUES \
            (1, 'mine', NULL), \
            (1, 'mine-trashed', '2026-01-01T00:00:00+00:00'), \
            (2, 'theirs', NULL), \
            (2, 'theirs-trashed', '2026-01-01T00:00:00+00:00')",
    )
    .await
    .unwrap();
    REGISTER.call_once(|| ScopeRegistry::register::<RsDoc, _>(RsTenantOne));
    db
}

async fn titles(query: Builder<RsDoc>) -> Vec<String> {
    let mut titles: Vec<String> = query
        .get()
        .await
        .unwrap()
        .iter()
        .map(|doc| doc.title.clone())
        .collect();
    titles.sort();
    titles
}

async fn every_title(db: &TestDatabase) -> Vec<String> {
    let _ = db;
    titles(RsDoc::query().without_global_scopes().with_trashed()).await
}

#[tokio::test]
async fn the_trashed_views_keep_every_registered_scope() {
    let _db = docs().await;

    assert_eq!(
        titles(RsDoc::with_trashed()).await,
        ["mine", "mine-trashed"]
    );
    assert_eq!(titles(RsDoc::only_trashed()).await, ["mine-trashed"]);
    assert_eq!(
        titles(RsDoc::query().with_trashed()).await,
        ["mine", "mine-trashed"]
    );
    assert_eq!(
        titles(RsDoc::query().only_trashed()).await,
        ["mine-trashed"]
    );
}

#[tokio::test]
async fn an_opt_out_chained_onto_query_lands() {
    let _db = docs().await;

    assert_eq!(titles(RsDoc::query()).await, ["mine"]);
    assert_eq!(
        titles(RsDoc::query().without_global_scope::<RsTenantOne>()).await,
        ["mine", "theirs"],
        "the tenant scope is lifted and the soft-delete filter stays"
    );
    assert_eq!(
        titles(
            RsDoc::query()
                .filter_op("id", ">", 0_i64)
                .without_global_scopes()
        )
        .await,
        ["mine", "theirs"],
        "the opt-out lands after a filter too"
    );
    assert_eq!(
        titles(RsDoc::without_global_scope::<RsTenantOne>()).await,
        ["mine", "theirs"],
        "the static form is the same query"
    );
}

#[tokio::test]
async fn an_or_cannot_widen_past_a_scope_or_the_soft_delete_filter() {
    let _db = docs().await;

    assert!(
        titles(RsDoc::query().or_where("title", "theirs"))
            .await
            .is_empty(),
        "an or_where with nothing before it is a plain filter under the scopes"
    );
    assert_eq!(
        titles(
            RsDoc::query()
                .filter("title", "mine")
                .or_where("title", "theirs")
                .or_where("title", "mine-trashed")
        )
        .await,
        ["mine"],
        "the disjunction is one atom: scopes AND (mine OR theirs OR mine-trashed)"
    );
}

#[tokio::test]
async fn the_rendered_statement_shows_the_scopes() {
    let _db = docs().await;

    let sql = RsDoc::query().filter("title", "mine").to_sql();

    assert!(sql.contains("deleted_at IS NULL"), "{sql}");
    assert!(sql.contains("tenant_id"), "{sql}");
    assert!(
        !RsDoc::query()
            .without_global_scopes()
            .with_trashed()
            .to_sql()
            .contains("WHERE"),
        "with every opt-out set the statement has no filter"
    );
}

#[tokio::test]
async fn aggregates_count_only_the_scoped_rows() {
    let _db = docs().await;

    assert_eq!(RsDoc::query().count().await.unwrap(), 1);
    assert_eq!(RsDoc::with_trashed().count().await.unwrap(), 2);
    assert!(
        RsDoc::query()
            .filter("title", "theirs")
            .exists()
            .await
            .is_ok_and(|found| !found)
    );
    assert_eq!(
        RsDoc::query().pluck::<String>("title").await.unwrap(),
        ["mine"]
    );
}

#[tokio::test]
async fn a_mass_update_reaches_only_the_scoped_rows() {
    let db = docs().await;

    let touched = RsDoc::query()
        .update_all(attrs! { title: "renamed" })
        .await
        .unwrap();

    assert_eq!(touched, 1);
    assert_eq!(
        every_title(&db).await,
        ["mine-trashed", "renamed", "theirs", "theirs-trashed"]
    );
}

#[tokio::test]
async fn a_mass_delete_cannot_reach_another_tenants_row() {
    let db = docs().await;

    let removed = RsDoc::query()
        .filter("title", "theirs")
        .delete_all()
        .await
        .unwrap();

    assert_eq!(removed, 0);
    assert_eq!(
        every_title(&db).await,
        ["mine", "mine-trashed", "theirs", "theirs-trashed"]
    );
}

// ---- Mass delete on a soft-delete model ---------------------------------

#[tokio::test]
async fn a_mass_delete_on_a_soft_delete_model_trashes_the_rows() {
    let db = docs().await;

    let trashed = RsDoc::query().delete_all().await.unwrap();

    assert_eq!(trashed, 1, "only the tenant's one live row is in reach");
    assert!(titles(RsDoc::query()).await.is_empty());
    assert_eq!(
        titles(RsDoc::only_trashed()).await,
        ["mine", "mine-trashed"],
        "the row is trashed, not gone"
    );
    assert_eq!(
        every_title(&db).await,
        ["mine", "mine-trashed", "theirs", "theirs-trashed"]
    );
}

#[tokio::test]
async fn force_delete_all_removes_the_rows_for_good() {
    let db = docs().await;

    let removed = RsDoc::only_trashed().force_delete_all().await.unwrap();

    assert_eq!(removed, 1, "the tenant's one trashed row");
    assert_eq!(every_title(&db).await, ["mine", "theirs", "theirs-trashed"]);
}

#[model(table = "rs_notes", soft_deletes, fillable = ["body"])]
pub struct RsNote {
    pub id: i64,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[tokio::test]
async fn a_mass_soft_delete_stamps_updated_at_with_the_tombstone() {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    db.execute_unprepared(
        "CREATE TABLE rs_notes (\
            id INTEGER PRIMARY KEY AUTOINCREMENT, \
            body TEXT NOT NULL, \
            created_at TEXT NOT NULL, \
            updated_at TEXT NOT NULL, \
            deleted_at TEXT\
         )",
    )
    .await
    .unwrap();
    let note = RsNote::create(attrs! { body: "draft" }).await.unwrap();
    let written_at = note.updated_at;

    let trashed = RsNote::query().delete_all().await.unwrap();

    assert_eq!(trashed, 1);
    let after = RsNote::only_trashed()
        .first()
        .await
        .unwrap()
        .expect("the note is trashed, and reads back through the model's casts");
    let tombstone = after.deleted_at.expect("the tombstone is set");
    assert!(tombstone >= written_at);
    assert!(
        after.updated_at >= written_at,
        "updated_at moved with the delete"
    );
    assert_eq!(after.created_at, note.created_at);
}
