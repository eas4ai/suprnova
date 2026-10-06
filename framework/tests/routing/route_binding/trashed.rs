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

/// A soft-deleting parent of a scoped child.
#[model(table = "tr_shelves", soft_deletes, relations = {
    books: HasMany<TrBook>,
})]
pub struct TrShelf {
    pub id: i64,
    pub name: String,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[model(table = "tr_books")]
pub struct TrBook {
    pub id: i64,
    pub tr_shelf_id: i64,
    pub slug: String,
}

#[handler]
pub async fn shelf_book(shelf: TrShelf, book: TrBook) -> Response {
    text(format!("{} {}", shelf.name, book.slug))
}

#[tokio::test]
async fn bind_008_a_soft_deleted_parent_binds_only_on_a_route_with_trashed() {
    let db = fixture().await;
    run_sql(
        &db,
        &[
            "CREATE TABLE tr_shelves (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, \
                deleted_at TEXT)",
            "CREATE TABLE tr_books (id INTEGER PRIMARY KEY AUTOINCREMENT, \
                tr_shelf_id INTEGER NOT NULL, slug TEXT NOT NULL)",
            "INSERT INTO tr_shelves (id, name, deleted_at) VALUES (1, 'open', NULL), \
                (2, 'closed', '2026-01-01T00:00:00+00:00')",
            "INSERT INTO tr_books (id, tr_shelf_id, slug) VALUES (1, 1, 'dune'), (2, 2, 'emma')",
        ],
    )
    .await;
    let router: Router = Router::new()
        .get("/shelves/{shelf}/books/{book:slug}", shelf_book)
        .get("/trashed/shelves/{shelf}/books/{book:slug}", shelf_book)
        .with_trashed()
        .get("/unscoped/shelves/{shelf}/books/{book}", shelf_book)
        .get("/trashed/unscoped/shelves/{shelf}/books/{book}", shelf_book)
        .with_trashed()
        .into();
    let addr = serve(router).await;
    // A live parent binds on both routes.
    assert_eq!(
        get(addr, "/shelves/1/books/dune").await,
        (200, "open dune".to_owned())
    );
    assert_eq!(
        get(addr, "/trashed/shelves/1/books/dune").await,
        (200, "open dune".to_owned())
    );
    // A soft-deleted parent, of a scoped child and of an unscoped one,
    // binds only where the route asks for trashed rows.
    assert_eq!(get(addr, "/shelves/2/books/emma").await.0, 404);
    assert_eq!(
        get(addr, "/trashed/shelves/2/books/emma").await,
        (200, "closed emma".to_owned())
    );
    assert_eq!(get(addr, "/unscoped/shelves/2/books/2").await.0, 404);
    assert_eq!(
        get(addr, "/trashed/unscoped/shelves/2/books/2").await,
        (200, "closed emma".to_owned())
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

// ── Router-wide model binders ───────────────────────────────────────────────

#[tokio::test]
async fn bind_008_a_model_binder_follows_with_trashed() {
    let _db = fixture().await;
    // The fallback answers its own 404, so a fallback run is told apart
    // from the row.
    let router: Router = Router::new()
        .model::<TrPost, _, _>("post", |_value| async {
            Err(FrameworkError::model_not_found("the fallback"))
        })
        .get("/posts/{post}", show)
        .get("/trashed/posts/{post}", show)
        .with_trashed()
        .into();
    let addr = serve(router).await;
    assert_eq!(get(addr, "/posts/1").await, (200, "live".to_owned()));
    let (status, body) = get(addr, "/posts/2").await;
    assert_eq!(status, 404, "{body}");
    assert!(body.contains("the fallback"), "{body}");
    assert_eq!(
        get(addr, "/trashed/posts/2").await,
        (200, "gone".to_owned()),
        "a model binder on a route with `with_trashed()` must use the soft-deletable lookup"
    );
}

#[tokio::test]
async fn bind_008_a_model_binder_gives_a_replaced_binding_the_lookup_the_route_selects() {
    let router: Router = Router::new()
        .model::<TrProbe, _, _>("probe", |_value| async { Ok(TrProbe("fallback")) })
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

// ── `with_trashed()` keeps global scopes ────────────────────────────────────

#[model(table = "tr_notes", soft_deletes)]
pub struct TrNote {
    pub id: i64,
    pub tenant_id: i64,
    pub body: String,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// The current tenant is tenant 1.
pub struct TrTenantScope;

impl suprnova::eloquent::scopes::GlobalScope<TrNote> for TrTenantScope {
    fn apply(&self, query: suprnova::Builder<TrNote>) -> suprnova::Builder<TrNote> {
        query.filter("tenant_id", 1_i64)
    }
}

#[handler]
pub async fn note(note: TrNote) -> Response {
    text(note.body)
}

#[tokio::test]
async fn bind_002_with_trashed_lifts_only_the_soft_delete_filter() {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    run_sql(
        &db,
        &[
            "CREATE TABLE tr_notes (id INTEGER PRIMARY KEY AUTOINCREMENT, \
                tenant_id INTEGER NOT NULL, body TEXT NOT NULL, deleted_at TEXT)",
            "INSERT INTO tr_notes (id, tenant_id, body, deleted_at) VALUES \
                (1, 1, 'mine', NULL), \
                (2, 1, 'mine, deleted', '2026-01-01T00:00:00+00:00'), \
                (3, 2, 'theirs', NULL), \
                (4, 2, 'theirs, deleted', '2026-01-01T00:00:00+00:00')",
        ],
    )
    .await;
    suprnova::eloquent::scopes::ScopeRegistry::register::<TrNote, _>(TrTenantScope);
    let router: Router = Router::new()
        .get("/notes/{note}", note)
        .get("/trashed/notes/{note}", note)
        .with_trashed()
        .into();
    let addr = serve(router).await;
    assert_eq!(get(addr, "/notes/1").await, (200, "mine".to_owned()));
    for id in [2, 3, 4] {
        assert_eq!(get(addr, &format!("/notes/{id}")).await.0, 404, "note {id}");
    }
    // `with_trashed()` lifts the soft-delete filter for the current
    // tenant's rows; another tenant's rows, deleted or not, stay hidden.
    assert_eq!(
        get(addr, "/trashed/notes/1").await,
        (200, "mine".to_owned())
    );
    assert_eq!(
        get(addr, "/trashed/notes/2").await,
        (200, "mine, deleted".to_owned())
    );
    for id in [3, 4] {
        assert_eq!(
            get(addr, &format!("/trashed/notes/{id}")).await.0,
            404,
            "another tenant's note {id} must stay hidden on a `with_trashed()` route"
        );
    }
}
