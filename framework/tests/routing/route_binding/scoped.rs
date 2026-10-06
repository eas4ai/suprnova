//! BIND-006: scoped bindings. A bound child whose parameter names a column,
//! or on a route or group that calls `scope_bindings()`, is looked up
//! through its parent's relation named by the child in the plural, so a row
//! the parent does not own answers 404. `without_scoped_bindings()` and an
//! explicit binder turn it off. A relation the parent does not declare, one
//! of another type, or a `MorphTo`, refuses the route at startup.

use suprnova::http::text;
use suprnova::testing::TestDatabase;
use suprnova::{Response, Router, get, group, handler, model};

use super::{get as get_path, refusal, run_sql, serve};

#[model(table = "sc_users", relations = {
    posts: HasMany<ScPost>,
    roles: BelongsToMany<ScRole, ScUserRole>,
    comments: HasManyThrough<ScPost, ScComment>,
    notes: HasMany<ScComment> { fk = "sc_user_id" },
})]
pub struct ScUser {
    pub id: i64,
    pub name: String,
}

#[model(table = "sc_posts", morph_type = "sc_post")]
pub struct ScPost {
    pub id: i64,
    pub sc_user_id: i64,
    pub slug: String,
}

#[model(table = "sc_roles")]
pub struct ScRole {
    pub id: i64,
    pub slug: String,
}

#[model(table = "sc_user_roles")]
pub struct ScUserRole {
    pub id: i64,
    pub sc_user_id: i64,
    pub sc_role_id: i64,
}

#[model(table = "sc_comments", relations = {
    commentables: MorphTo { name = "commentable", targets = [ScPost] },
})]
pub struct ScComment {
    pub id: i64,
    pub sc_post_id: i64,
    pub sc_user_id: i64,
    pub slug: String,
    pub commentable_id: i64,
    pub commentable_type: String,
}

#[handler]
pub async fn user_post(user: ScUser, post: ScPost) -> Response {
    text(format!("{} {}", user.name, post.slug))
}

#[handler]
pub async fn user_role(user: ScUser, role: ScRole) -> Response {
    text(format!("{} {}", user.name, role.slug))
}

#[handler]
pub async fn user_comment(user: ScUser, comment: ScComment) -> Response {
    text(format!("{} {}", user.name, comment.slug))
}

#[handler]
pub async fn post_only(post: ScPost) -> Response {
    text(post.slug)
}

#[handler]
pub async fn user_note(user: ScUser, note: ScPost) -> Response {
    text(format!("{} {}", user.name, note.slug))
}

#[handler]
pub async fn comment_commentable(comment: ScComment, commentable: ScPost) -> Response {
    text(format!("{} {}", comment.slug, commentable.slug))
}

#[handler]
pub async fn post_tag(post: ScPost, tag: ScRole) -> Response {
    text(format!("{} {}", post.slug, tag.slug))
}

async fn fixture() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    run_sql(
        &db,
        &[
            "CREATE TABLE sc_users (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
            "CREATE TABLE sc_posts (id INTEGER PRIMARY KEY AUTOINCREMENT, \
                sc_user_id INTEGER NOT NULL, slug TEXT NOT NULL)",
            "CREATE TABLE sc_roles (id INTEGER PRIMARY KEY AUTOINCREMENT, slug TEXT NOT NULL)",
            "CREATE TABLE sc_user_roles (id INTEGER PRIMARY KEY AUTOINCREMENT, \
                sc_user_id INTEGER NOT NULL, sc_role_id INTEGER NOT NULL)",
            "CREATE TABLE sc_comments (id INTEGER PRIMARY KEY AUTOINCREMENT, \
                sc_post_id INTEGER NOT NULL, sc_user_id INTEGER NOT NULL, slug TEXT NOT NULL, \
                commentable_id INTEGER NOT NULL, commentable_type TEXT NOT NULL)",
            "INSERT INTO sc_users (id, name) VALUES (1, 'ada'), (2, 'grace')",
            "INSERT INTO sc_posts (id, sc_user_id, slug) VALUES (1, 1, 'ada-post'), \
                (2, 2, 'grace-post')",
            "INSERT INTO sc_roles (id, slug) VALUES (1, 'admin'), (2, 'editor')",
            "INSERT INTO sc_user_roles (sc_user_id, sc_role_id) VALUES (1, 1)",
            "INSERT INTO sc_comments (id, sc_post_id, sc_user_id, slug, commentable_id, \
                commentable_type) VALUES (1, 1, 2, 'on-ada', 1, 'sc_post'), \
                (2, 2, 1, 'on-grace', 2, 'sc_post')",
        ],
    )
    .await;
    db
}

#[tokio::test]
async fn bind_006_a_child_with_a_binding_field_is_found_through_its_parent() {
    let _db = fixture().await;
    let addr = serve(
        Router::new()
            .get("/users/{user}/posts/{post:slug}", user_post)
            .into(),
    )
    .await;
    assert_eq!(
        get_path(addr, "/users/1/posts/ada-post").await,
        (200, "ada ada-post".to_owned())
    );
    assert_eq!(
        get_path(addr, "/users/1/posts/grace-post").await.0,
        404,
        "Grace's post is not Ada's"
    );
}

#[tokio::test]
async fn bind_006_scope_bindings_scopes_a_child_without_a_field() {
    let _db = fixture().await;
    let router: Router = Router::new()
        .get("/plain/users/{user}/posts/{post}", user_post)
        .get("/scoped/users/{user}/posts/{post}", user_post)
        .scope_bindings()
        .into();
    let addr = serve(router).await;
    assert_eq!(
        get_path(addr, "/plain/users/1/posts/2").await,
        (200, "ada grace-post".to_owned()),
        "without a field or scope_bindings the child is not scoped"
    );
    assert_eq!(get_path(addr, "/scoped/users/1/posts/2").await.0, 404);
    assert_eq!(
        get_path(addr, "/scoped/users/1/posts/1").await,
        (200, "ada ada-post".to_owned())
    );
}

#[tokio::test]
async fn bind_006_a_group_scopes_its_routes() {
    let _db = fixture().await;
    let router = group!("/g", { get!("/users/{user}/posts/{post}", user_post) }).scope_bindings();
    let addr = serve(router.register(Router::new())).await;
    assert_eq!(get_path(addr, "/g/users/1/posts/2").await.0, 404);
    assert_eq!(get_path(addr, "/g/users/2/posts/2").await.0, 200);
}

#[tokio::test]
async fn bind_006_without_scoped_bindings_does_not_scope() {
    let _db = fixture().await;
    let router: Router = Router::new()
        .get("/users/{user}/posts/{post:slug}", user_post)
        .without_scoped_bindings()
        .into();
    let addr = serve(router).await;
    assert_eq!(
        get_path(addr, "/users/1/posts/grace-post").await,
        (200, "ada grace-post".to_owned())
    );
}

#[tokio::test]
async fn bind_006_a_child_bound_by_bind_is_not_scoped() {
    let _db = fixture().await;
    let router: Router = Router::new()
        .get("/users/{user}/posts/{post:slug}", user_post)
        .into();
    let router = router.bind("post", |value: String, _route| async move {
        use suprnova::Model;
        ScPost::query().filter("slug", value).first().await
    });
    let addr = serve(router).await;
    assert_eq!(
        get_path(addr, "/users/1/posts/grace-post").await,
        (200, "ada grace-post".to_owned())
    );
}

#[tokio::test]
async fn bind_006_a_child_without_a_bound_parent_is_not_scoped() {
    // Only `post` binds: the parameter before it is not bound, so there is
    // no parent to scope to.
    let _db = fixture().await;
    let addr = serve(
        Router::new()
            .get("/users/{user}/posts/{post:slug}", post_only)
            .into(),
    )
    .await;
    assert_eq!(
        get_path(addr, "/users/1/posts/grace-post").await,
        (200, "grace-post".to_owned())
    );
}

#[tokio::test]
async fn bind_006_scoping_through_a_belongs_to_many_relation() {
    let _db = fixture().await;
    let addr = serve(
        Router::new()
            .get("/users/{user}/roles/{role:slug}", user_role)
            .into(),
    )
    .await;
    assert_eq!(
        get_path(addr, "/users/1/roles/admin").await,
        (200, "ada admin".to_owned())
    );
    assert_eq!(get_path(addr, "/users/1/roles/editor").await.0, 404);
    assert_eq!(get_path(addr, "/users/2/roles/admin").await.0, 404);
}

#[tokio::test]
async fn bind_006_scoping_through_a_has_many_through_relation() {
    let _db = fixture().await;
    let addr = serve(
        Router::new()
            .get("/users/{user}/comments/{comment:slug}", user_comment)
            .into(),
    )
    .await;
    // `on-ada` is on Ada's post; `on-grace` is on Grace's.
    assert_eq!(
        get_path(addr, "/users/1/comments/on-ada").await,
        (200, "ada on-ada".to_owned())
    );
    assert_eq!(get_path(addr, "/users/1/comments/on-grace").await.0, 404);
}

#[test]
fn bind_006_a_relation_the_parent_does_not_declare_is_refused_at_startup() {
    let router: Router = Router::new()
        .get("/posts/{post}/tags/{tag:slug}", post_tag)
        .into();
    let error = refusal(&router);
    assert!(
        error.contains("GET /posts/{post}/tags/{tag:slug}"),
        "{error}"
    );
    assert!(error.contains("`ScPost`"), "{error}");
    assert!(error.contains("`tags`"), "{error}");
}

#[test]
fn bind_006_a_relation_of_another_type_is_refused_at_startup() {
    // `notes` returns comments, but the child binds a post.
    let router: Router = Router::new()
        .get("/users/{user}/notes/{note:slug}", user_note)
        .into();
    let error = refusal(&router);
    assert!(error.contains("`notes`"), "{error}");
    assert!(error.contains("ScComment"), "{error}");
}

#[test]
fn bind_006_a_morph_to_relation_is_refused_at_startup() {
    let router: Router = Router::new()
        .get(
            "/comments/{comment}/commentables/{commentable:slug}",
            comment_commentable,
        )
        .into();
    let error = refusal(&router);
    assert!(error.contains("`commentables`"), "{error}");
    assert!(error.contains("MorphTo"), "{error}");
}
