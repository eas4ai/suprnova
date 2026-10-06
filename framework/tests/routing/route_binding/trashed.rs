//! BIND-008: soft-deleted rows. A route's `with_trashed()` makes every
//! binding of the route, scoped children and the bare `x::Model` form
//! included, go through the soft-deletable lookups; every other route goes
//! through the plain ones. A resource's `with_trashed()` applies to `show`,
//! `edit` and `update` unless it names actions.

use chrono::{DateTime, Utc};
use suprnova::database::EntityExt;
use suprnova::http::text;
use suprnova::testing::TestDatabase;
use suprnova::{FrameworkError, Response, RouteBinding, Router, handler, model, resource};

use super::{get, run_sql, send, serve};

#[model(table = "tr_authors", relations = {
    posts: HasMany<TrPost>,
})]
pub struct TrAuthor {
    pub id: i64,
    pub name: String,
}

#[model(table = "tr_posts", soft_deletes)]
pub struct TrPost {
    pub id: i64,
    pub tr_author_id: i64,
    pub slug: String,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl EntityExt for tr_post::Entity {}

/// A type that says which lookup the route chose.
pub struct TrProbe(&'static str);

#[suprnova::async_trait]
impl RouteBinding for TrProbe {
    fn route_key_name() -> &'static str {
        "key"
    }
    fn route_key(&self) -> String {
        self.0.to_owned()
    }
    async fn resolve_route_binding(
        _value: &str,
        _field: Option<&str>,
    ) -> Result<Option<Self>, FrameworkError> {
        Ok(Some(TrProbe("plain")))
    }
    async fn resolve_soft_deletable_route_binding(
        _value: &str,
        _field: Option<&str>,
    ) -> Result<Option<Self>, FrameworkError> {
        Ok(Some(TrProbe("soft-deletable")))
    }
}

#[handler]
pub async fn show(post: TrPost) -> Response {
    text(post.slug)
}

#[handler]
pub async fn show_bare(post: tr_post::Model) -> Response {
    text(post.slug)
}

#[handler]
pub async fn author_post(author: TrAuthor, post: TrPost) -> Response {
    text(format!("{} {}", author.name, post.slug))
}

#[handler]
pub async fn probe(probe: TrProbe) -> Response {
    text(probe.0)
}

/// The function form of a resource over `TrPost`.
pub mod posts {
    use super::*;

    #[handler]
    pub async fn show(post: TrPost) -> Response {
        text(format!("show {}", post.slug))
    }
    #[handler]
    pub async fn edit(post: TrPost) -> Response {
        text(format!("edit {}", post.slug))
    }
    #[handler]
    pub async fn update(post: TrPost) -> Response {
        text(format!("update {}", post.slug))
    }
    #[handler]
    pub async fn destroy(post: TrPost) -> Response {
        text(format!("destroy {}", post.slug))
    }
}

async fn fixture() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    run_sql(
        &db,
        &[
            "CREATE TABLE tr_authors (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
            "CREATE TABLE tr_posts (id INTEGER PRIMARY KEY AUTOINCREMENT, \
                tr_author_id INTEGER NOT NULL, slug TEXT NOT NULL, deleted_at TEXT)",
            "INSERT INTO tr_authors (id, name) VALUES (1, 'ada')",
            "INSERT INTO tr_posts (id, tr_author_id, slug, deleted_at) VALUES \
                (1, 1, 'live', NULL), (2, 1, 'gone', '2026-01-01T00:00:00+00:00')",
        ],
    )
    .await;
    db
}

#[tokio::test]
async fn bind_008_a_soft_deleted_row_binds_only_on_a_route_with_trashed() {
    let _db = fixture().await;
    let router: Router = Router::new()
        .get("/posts/{post}", show)
        .get("/trashed/posts/{post}", show)
        .with_trashed()
        .into();
    let addr = serve(router).await;
    assert_eq!(get(addr, "/posts/1").await, (200, "live".to_owned()));
    assert_eq!(get(addr, "/posts/2").await.0, 404);
    assert_eq!(
        get(addr, "/trashed/posts/2").await,
        (200, "gone".to_owned())
    );
}

#[tokio::test]
async fn bind_008_the_bare_form_follows_with_trashed() {
    let _db = fixture().await;
    let router: Router = Router::new()
        .get("/bare/{post}", show_bare)
        .get("/bare-trashed/{post}", show_bare)
        .with_trashed()
        .into();
    let addr = serve(router).await;
    assert_eq!(get(addr, "/bare/1").await, (200, "live".to_owned()));
    assert_eq!(get(addr, "/bare/2").await.0, 404);
    assert_eq!(get(addr, "/bare-trashed/2").await, (200, "gone".to_owned()));
}

#[tokio::test]
async fn bind_008_a_scoped_child_follows_with_trashed() {
    let _db = fixture().await;
    let router: Router = Router::new()
        .get("/authors/{author}/posts/{post:slug}", author_post)
        .get("/trashed/authors/{author}/posts/{post:slug}", author_post)
        .with_trashed()
        .into();
    let addr = serve(router).await;
    assert_eq!(get(addr, "/authors/1/posts/gone").await.0, 404);
    assert_eq!(
        get(addr, "/trashed/authors/1/posts/gone").await,
        (200, "ada gone".to_owned())
    );
}

#[tokio::test]
async fn bind_008_a_replaced_binding_receives_the_soft_deletable_lookup() {
    let router: Router = Router::new()
        .get("/probe/{probe}", probe)
        .get("/trashed/probe/{probe}", probe)
        .with_trashed()
        .into();
    let addr = serve(router).await;
    assert_eq!(get(addr, "/probe/x").await, (200, "plain".to_owned()));
    assert_eq!(
        get(addr, "/trashed/probe/x").await,
        (200, "soft-deletable".to_owned())
    );
}

#[tokio::test]
async fn bind_008_a_resource_with_trashed_applies_to_show_edit_and_update() {
    let _db = fixture().await;
    let router = resource!("posts", posts, only = [show, edit, update, destroy])
        .with_trashed(&[])
        .unnamed()
        .register(Router::new());
    let addr = serve(router).await;
    assert_eq!(get(addr, "/posts/2").await, (200, "show gone".to_owned()));
    assert_eq!(
        get(addr, "/posts/2/edit").await,
        (200, "edit gone".to_owned())
    );
    assert_eq!(
        send(addr, "PUT", "/posts/2", &[], None).await,
        (200, "update gone".to_owned())
    );
    assert_eq!(
        send(addr, "PATCH", "/posts/2", &[], None).await,
        (200, "update gone".to_owned())
    );
    assert_eq!(
        send(addr, "DELETE", "/posts/2", &[], None).await.0,
        404,
        "destroy is not among the default actions"
    );

    let router = resource!("posts", posts, only = [show, destroy])
        .with_trashed(&[suprnova::routing::ResourceAction::Destroy])
        .unnamed()
        .register(Router::new());
    let addr = serve(router).await;
    assert_eq!(get(addr, "/posts/2").await.0, 404, "show is not named");
    assert_eq!(
        send(addr, "DELETE", "/posts/2", &[], None).await,
        (200, "destroy gone".to_owned())
    );
}
