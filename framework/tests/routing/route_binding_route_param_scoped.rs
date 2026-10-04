//! Regression for the `RouteParam<M>` scoped route-binding contract
//! introduced in deep-dive 6 (A2-M-005).
//!
//! Two binding shapes ship today, and the test pins their contracts:
//!
//! | Handler param shape          | Scope policy                            |
//! |------------------------------|-----------------------------------------|
//! | `RouteParam<User>`           | Reads through `User::query()` - applies global scopes + soft-delete filter |
//! | bare `<inner>::Model`        | Bypasses the Eloquent scope - exposes trashed rows |
//!
//! The wrapped path is the safe default (Laravel-equivalent
//! `Route::model(...)`). The raw `Model` path remains the escape hatch
//! for admin tools that must reach trashed rows by id (Laravel's
//! `->withTrashed()` opt-in).

use chrono::{DateTime, Utc};
use suprnova::database::AutoRouteBinding;
use suprnova::error::FrameworkError;
use suprnova::testing::TestDatabase;
use suprnova::{Model, RouteParam, attrs, model};

#[model(table = "rbsd_users", soft_deletes, fillable = ["name", "email"])]
pub struct RbSdUser {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub deleted_at: Option<DateTime<Utc>>,
}

// Opt the inner SeaORM `Entity` into `EntityExt` / `EntityExtMut` so
// the raw `<rb_sd_user::Model as AutoRouteBinding>` path resolves
// through the blanket impl. The `#[model]` macro deliberately leaves
// these opt-in (per `framework/src/database/model.rs` policy) and
// app code adds them next to its entity definitions.
impl suprnova::database::EntityExt for rb_sd_user::Entity {}
impl suprnova::database::EntityExtMut for rb_sd_user::Entity {}

async fn migrate(db: &TestDatabase) {
    db.execute_unprepared(
        "CREATE TABLE rbsd_users (\
            id INTEGER PRIMARY KEY AUTOINCREMENT, \
            name TEXT NOT NULL, \
            email TEXT NOT NULL, \
            deleted_at TEXT\
         )",
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn route_param_filters_trashed_rows() {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    migrate(&db).await;

    let user = RbSdUser::create(attrs! { name: "Alice", email: "a@x.com" })
        .await
        .unwrap();
    let user_id = user.id;

    // Sanity: alive row is reachable through the wrapped path.
    let bound: RouteParam<RbSdUser> =
        <RouteParam<RbSdUser> as AutoRouteBinding>::from_route_param(&user_id.to_string())
            .await
            .expect("scoped binding finds alive row");
    assert_eq!(bound.id, user_id);
    assert_eq!(bound.name, "Alice");

    // Soft-delete the row.
    user.delete().await.unwrap();

    // Wrapped path now refuses to bind - soft-delete scope applies.
    let err = <RouteParam<RbSdUser> as AutoRouteBinding>::from_route_param(&user_id.to_string())
        .await
        .expect_err("scoped binding hides trashed row");
    match err {
        FrameworkError::ModelNotFound { model_name } => {
            assert_eq!(model_name, "RbSdUser");
        }
        other => panic!("expected ModelNotFound, got {other:?}"),
    }
}

#[tokio::test]
async fn raw_model_binding_exposes_trashed_rows() {
    // The bare SeaORM `Model` path goes through `EntityExt::find_by_pk`,
    // which is intentionally unscoped - it's the escape hatch for admin
    // surfaces that must reach trashed rows by id. This test pins the
    // contract so we notice if a future change accidentally routes the
    // raw path through the scoped finder.
    let db = TestDatabase::sqlite_memory().await.unwrap();
    migrate(&db).await;

    let user = RbSdUser::create(attrs! { name: "Bob", email: "b@x.com" })
        .await
        .unwrap();
    let user_id = user.id;
    user.delete().await.unwrap();

    // Raw Model path - no scope filter, trashed row returned.
    let row: rb_sd_user::Model =
        <rb_sd_user::Model as AutoRouteBinding>::from_route_param(&user_id.to_string())
            .await
            .expect("raw Model binding exposes trashed row");
    assert_eq!(row.id, user_id);
    assert_eq!(row.name, "Bob");
    assert!(
        row.deleted_at.is_some(),
        "raw Model binding returns the trashed row with deleted_at populated"
    );
}

#[tokio::test]
async fn route_param_404_for_missing_id() {
    // Wrapped path returns ModelNotFound for ids that never existed,
    // identical to the trashed-row case from the caller's perspective.
    let db = TestDatabase::sqlite_memory().await.unwrap();
    migrate(&db).await;

    let err = <RouteParam<RbSdUser> as AutoRouteBinding>::from_route_param("99999")
        .await
        .expect_err("missing id");
    assert!(matches!(err, FrameworkError::ModelNotFound { .. }));
}

#[tokio::test]
async fn route_param_param_parse_error_for_non_integer() {
    // Wrapped path surfaces ParamParse (400) when the captured string
    // can't parse into the model's PK type - same error shape as the
    // raw path.
    let db = TestDatabase::sqlite_memory().await.unwrap();
    migrate(&db).await;

    let err = <RouteParam<RbSdUser> as AutoRouteBinding>::from_route_param("not-an-int")
        .await
        .expect_err("non-numeric id");
    assert!(matches!(err, FrameworkError::ParamParse { .. }));
}

// A model without `soft_deletes` and a registered tenant scope: the
// scoped binding must apply the scope through the same builder every
// `Model::query()` read uses, not only the soft-delete filter. The
// model type is unique to these tests, so the process-wide
// `ScopeRegistry` entry cannot reach any other test.
#[model(table = "rbsc_articles", fillable = ["tenant_id", "title"])]
pub struct RbScArticle {
    pub id: i64,
    pub tenant_id: i64,
    pub title: String,
}

pub struct RbScTenantScope;

impl suprnova::eloquent::scopes::GlobalScope<RbScArticle> for RbScTenantScope {
    fn apply(&self, query: suprnova::Builder<RbScArticle>) -> suprnova::Builder<RbScArticle> {
        query.filter("tenant_id", 1_i64)
    }
}

#[tokio::test]
async fn route_param_applies_global_scopes_on_a_model_without_soft_deletes() {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    db.execute_unprepared(
        "CREATE TABLE rbsc_articles (\
            id INTEGER PRIMARY KEY AUTOINCREMENT, \
            tenant_id INTEGER NOT NULL, \
            title TEXT NOT NULL\
         )",
    )
    .await
    .unwrap();
    // Rows are written before the scope is registered, through raw SQL,
    // so neither insert depends on the scope's write behaviour.
    db.execute_unprepared(
        "INSERT INTO rbsc_articles (id, tenant_id, title) VALUES \
            (1, 1, 'mine'), (2, 2, 'another tenant')",
    )
    .await
    .unwrap();
    suprnova::eloquent::scopes::ScopeRegistry::register::<RbScArticle, _>(RbScTenantScope);

    // The row the scope admits binds.
    let bound = <RouteParam<RbScArticle> as AutoRouteBinding>::from_route_param("1")
        .await
        .expect("scoped binding finds the current tenant's row");
    assert_eq!(bound.title, "mine");

    // The row the scope hides is a 404, as a missing row is: a guessed id
    // of another tenant's row must not bind.
    let err = <RouteParam<RbScArticle> as AutoRouteBinding>::from_route_param("2")
        .await
        .expect_err("scoped binding hides another tenant's row");
    match err {
        FrameworkError::ModelNotFound { model_name } => {
            assert_eq!(model_name, "RbScArticle");
        }
        other => panic!("expected ModelNotFound, got {other:?}"),
    }
}
