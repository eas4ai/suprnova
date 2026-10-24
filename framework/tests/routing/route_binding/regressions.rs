//! BIND-003: routes written before route binding keep working. A handler
//! that binds with `RouteParam<T>` or the bare `x::Model` form binds as it
//! did, and a primitive `#[authorize]` target still compiles and checks the
//! same value. Folded from `route_binding_route_param_scoped.rs` and
//! `route_param_destructured.rs`, with the two changes BIND-002 and BIND-008
//! make: a malformed key is a miss, and the bare form leaves trashed rows
//! out unless the route asks for them.

use std::any::Any;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use suprnova::database::AutoRouteBinding;
use suprnova::error::FrameworkError;
use suprnova::http::text;
use suprnova::testing::TestDatabase;
use suprnova::{
    Auth, Authenticatable, Gate, Middleware, MiddlewareRegistry, Model, Next, Request, Response,
    RouteParam, Router, attrs, handler, model,
};

use super::{get, run_sql, send, serve, serve_with};

#[model(table = "rbsd_users", soft_deletes, fillable = ["name", "email"])]
pub struct RbSdUser {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub deleted_at: Option<DateTime<Utc>>,
}

// The application opts the inner SeaORM entity into `EntityExt`, so the
// bare `rb_sd_user::Model` form binds through the blanket impl.
impl suprnova::database::EntityExt for rb_sd_user::Entity {}
impl suprnova::database::EntityExtMut for rb_sd_user::Entity {}

async fn users() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    run_sql(
        &db,
        &["CREATE TABLE rbsd_users (\
            id INTEGER PRIMARY KEY AUTOINCREMENT, \
            name TEXT NOT NULL, \
            email TEXT NOT NULL, \
            deleted_at TEXT\
         )"],
    )
    .await;
    db
}

#[tokio::test]
async fn bind_003_route_param_still_hides_a_trashed_row() {
    let _db = users().await;
    let user = RbSdUser::create(attrs! { name: "Alice", email: "a@x.com" })
        .await
        .unwrap();
    let user_id = user.id;

    let bound = <RouteParam<RbSdUser> as AutoRouteBinding>::from_route_param(&user_id.to_string())
        .await
        .expect("scoped binding finds alive row");
    assert_eq!(bound.id, user_id);
    assert_eq!(bound.name, "Alice");

    user.delete().await.unwrap();

    let err = <RouteParam<RbSdUser> as AutoRouteBinding>::from_route_param(&user_id.to_string())
        .await
        .expect_err("scoped binding hides trashed row");
    match err {
        FrameworkError::ModelNotFound { model_name } => assert_eq!(model_name, "RbSdUser"),
        other => panic!("expected ModelNotFound, got {other:?}"),
    }
}

#[tokio::test]
async fn bind_003_the_bare_form_still_binds_by_primary_key() {
    let _db = users().await;
    let user = RbSdUser::create(attrs! { name: "Bob", email: "b@x.com" })
        .await
        .unwrap();

    let row: rb_sd_user::Model =
        <rb_sd_user::Model as AutoRouteBinding>::from_route_param(&user.id.to_string())
            .await
            .expect("the bare form binds a live row by primary key");
    assert_eq!(row.id, user.id);
    assert_eq!(row.name, "Bob");

    // BIND-008: a trashed row is reached through the soft-deletable lookup
    // alone, which a route's `with_trashed()` selects.
    let id = user.id;
    user.delete().await.unwrap();
    let err = <rb_sd_user::Model as AutoRouteBinding>::from_route_param(&id.to_string())
        .await
        .expect_err("the plain lookup leaves the trashed row out");
    assert!(
        matches!(err, FrameworkError::ModelNotFound { .. }),
        "{err:?}"
    );
    let trashed =
        <rb_sd_user::Model as suprnova::RouteBinding>::resolve_soft_deletable_route_binding(
            &id.to_string(),
            None,
        )
        .await
        .unwrap()
        .expect("the soft-deletable lookup finds it");
    assert!(trashed.deleted_at.is_some());
}

#[tokio::test]
async fn bind_003_route_param_answers_model_not_found_for_a_missing_id() {
    let _db = users().await;
    let err = <RouteParam<RbSdUser> as AutoRouteBinding>::from_route_param("99999")
        .await
        .expect_err("missing id");
    assert!(matches!(err, FrameworkError::ModelNotFound { .. }));
}

#[tokio::test]
async fn bind_002_a_malformed_key_is_the_same_miss_as_a_missing_row() {
    // BIND-002 changes this case: a key that does not parse used to be a
    // 400 that repeated the value; it is now the miss a missing row is.
    let _db = users().await;
    let err = <RouteParam<RbSdUser> as AutoRouteBinding>::from_route_param("not-an-int")
        .await
        .expect_err("non-numeric id");
    match err {
        FrameworkError::ModelNotFound { model_name } => assert_eq!(model_name, "RbSdUser"),
        other => panic!("expected ModelNotFound, got {other:?}"),
    }
}

// ── A SeaORM entity no `#[model]` defines ──────────────────────────────────

/// A SeaORM entity written by hand, as `db:sync` writes one, that no
/// `#[model]` defines. Its `deleted_at` column belongs to no soft-deleting
/// `#[model]`, so the bare form treats no row as soft-deleted (BIND-008).
pub mod rg_note {
    use sea_orm::entity::prelude::*;
    use serde::{Deserialize, Serialize};

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Serialize, Deserialize)]
    #[sea_orm(table_name = "rg_notes")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        pub body: String,
        pub deleted_at: Option<String>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

impl suprnova::database::EntityExt for rg_note::Entity {}

#[handler]
pub async fn show_note(note: rg_note::Model) -> Response {
    text(note.body)
}

#[tokio::test]
async fn bind_003_the_bare_form_binds_an_entity_no_model_defines() {
    let _db = {
        let db = TestDatabase::sqlite_memory().await.unwrap();
        run_sql(
            &db,
            &[
                "CREATE TABLE rg_notes (id INTEGER PRIMARY KEY AUTOINCREMENT, \
                    body TEXT NOT NULL, deleted_at TEXT)",
                "INSERT INTO rg_notes (id, body, deleted_at) VALUES \
                    (1, 'first', NULL), (2, 'dated', '2026-01-01T00:00:00+00:00')",
            ],
        )
        .await;
        db
    };
    let router: Router = Router::new()
        .get("/notes/{note}", show_note)
        .get("/trashed/notes/{note}", show_note)
        .with_trashed()
        .into();
    router
        .prepare_bindings()
        .expect("the bare form of a hand-written entity passes the startup checks");
    let addr = serve(router).await;

    // By primary key, as before route binding.
    assert_eq!(get(addr, "/notes/1").await, (200, "first".to_owned()));
    // No `#[model]` covers `rg_notes`, so a set `deleted_at` is just a
    // column: the row binds on every route.
    assert_eq!(get(addr, "/notes/2").await, (200, "dated".to_owned()));
    assert_eq!(
        get(addr, "/trashed/notes/2").await,
        (200, "dated".to_owned())
    );
    // A missing row and a malformed key are the same 404, naming the
    // entity's module.
    let (missing_status, missing) = get(addr, "/notes/99").await;
    let (malformed_status, malformed) = get(addr, "/notes/1x").await;
    assert_eq!((missing_status, malformed_status), (404, 404));
    assert_eq!(super::message(&missing), "rg_note not found");
    assert_eq!(super::message(&malformed), "rg_note not found");
    assert!(!malformed.contains("1x"), "{malformed}");

    // The trait, called directly, binds the same row by primary key.
    let row = <rg_note::Model as AutoRouteBinding>::from_route_param("2")
        .await
        .expect("the bare form binds by primary key");
    assert_eq!(row.body, "dated");
    assert_eq!(
        <rg_note::Model as suprnova::RouteBinding>::route_key_name(),
        "id"
    );
    assert_eq!(suprnova::RouteBinding::route_key(&row), "2");
}

// A model without `soft_deletes` and a registered tenant scope: the binding
// applies the scope through the builder every `Model::query()` read uses.
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

#[handler]
pub async fn show_scoped_article(article: RbScArticle) -> Response {
    text(article.title)
}

#[tokio::test]
async fn bind_002_the_lookup_applies_global_scopes() {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    run_sql(
        &db,
        &[
            "CREATE TABLE rbsc_articles (\
                id INTEGER PRIMARY KEY AUTOINCREMENT, \
                tenant_id INTEGER NOT NULL, \
                title TEXT NOT NULL\
             )",
            "INSERT INTO rbsc_articles (id, tenant_id, title) VALUES \
                (1, 1, 'mine'), (2, 2, 'another tenant')",
        ],
    )
    .await;
    suprnova::eloquent::scopes::ScopeRegistry::register::<RbScArticle, _>(RbScTenantScope);

    let bound = <RouteParam<RbScArticle> as AutoRouteBinding>::from_route_param("1")
        .await
        .expect("scoped binding finds the current tenant's row");
    assert_eq!(bound.title, "mine");
    let err = <RouteParam<RbScArticle> as AutoRouteBinding>::from_route_param("2")
        .await
        .expect_err("scoped binding hides another tenant's row");
    assert!(
        matches!(err, FrameworkError::ModelNotFound { .. }),
        "{err:?}"
    );

    // The same through a route, bound by type.
    let addr = serve(
        Router::new()
            .get("/articles/{article}", show_scoped_article)
            .into(),
    )
    .await;
    assert_eq!(get(addr, "/articles/1").await, (200, "mine".to_owned()));
    assert_eq!(get(addr, "/articles/2").await.0, 404);
}

// ── The destructured and plain `RouteParam` forms ──────────────────────────

#[model(table = "rd_users", fillable = ["name"])]
pub struct RdUser {
    pub id: i64,
    pub name: String,
}

#[handler]
pub async fn show_destructured(RouteParam(user): RouteParam<RdUser>) -> Response {
    text(format!("destructured {}", user.name))
}

#[handler]
pub async fn show_destructured_mut(RouteParam(mut user): RouteParam<RdUser>) -> Response {
    user.name.push('!');
    text(format!("mut {}", user.name))
}

#[handler]
pub async fn show_plain(user: RouteParam<RdUser>) -> Response {
    text(format!("plain {}", user.name))
}

async fn rd_server() -> (TestDatabase, std::net::SocketAddr) {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    run_sql(
        &db,
        &["CREATE TABLE rd_users (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)"],
    )
    .await;
    RdUser::create(attrs! { name: "Alice" }).await.unwrap();
    let router: Router = Router::new()
        .get("/destructured/{user}", show_destructured)
        .get("/destructured-mut/{user}", show_destructured_mut)
        .get("/plain/{user}", show_plain)
        .into();
    let addr = serve(router).await;
    (db, addr)
}

#[tokio::test]
async fn bind_003_a_destructured_route_param_binds_the_parameter_named_by_its_binding() {
    let (_db, addr) = rd_server().await;
    assert_eq!(
        get(addr, "/destructured/1").await,
        (200, "destructured Alice".to_string())
    );
    assert_eq!(
        get(addr, "/destructured-mut/1").await,
        (200, "mut Alice!".to_string())
    );
}

#[tokio::test]
async fn bind_003_a_destructured_route_param_answers_404_for_a_missing_row() {
    let (_db, addr) = rd_server().await;
    let (status, body) = get(addr, "/destructured/999").await;
    assert_eq!(status, 404, "body: {body}");
}

#[tokio::test]
async fn bind_003_a_plain_route_param_binds_as_before() {
    let (_db, addr) = rd_server().await;
    assert_eq!(
        get(addr, "/plain/1").await,
        (200, "plain Alice".to_string())
    );
}

// ── A primitive `#[authorize]` target ──────────────────────────────────────

#[derive(Clone)]
struct RgUser {
    id: i64,
}

impl Authenticatable for RgUser {
    fn get_auth_identifier(&self) -> String {
        self.id.to_string()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn into_arc_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

/// Signs in the user the `X-User` header names.
struct RgLogin;

#[async_trait::async_trait]
impl Middleware for RgLogin {
    async fn handle(&self, request: Request, next: Next) -> Response {
        if let Some(id) = request.header("X-User").and_then(|id| id.parse().ok()) {
            Auth::set_user(Arc::new(RgUser { id }));
        }
        next(request).await
    }
}

#[handler]
#[suprnova::authorize("rg-read-own", id)]
pub async fn read_own(id: i64) -> Response {
    text(format!("own {id}"))
}

#[tokio::test]
async fn bind_003_a_primitive_authorize_target_checks_the_path_value() {
    Gate::define::<RgUser, i64>("rg-read-own", |user, id| user.id == *id);
    let router: Router = Router::new().get("/own/{id}", read_own).into();
    router
        .prepare_bindings()
        .expect("a primitive target reads a declared parameter");
    let addr = serve_with(router, MiddlewareRegistry::new().append(RgLogin)).await;
    assert_eq!(
        send(addr, "GET", "/own/7", &[("X-User", "7")], None).await,
        (200, "own 7".to_owned())
    );
    let (status, _) = send(addr, "GET", "/own/8", &[("X-User", "7")], None).await;
    assert_eq!(status, 403, "the check saw the path value 8");
}
