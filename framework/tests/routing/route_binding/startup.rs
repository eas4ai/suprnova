//! BIND-004 and BIND-013: binding fields and the startup checks. A
//! `{name:column}` segment registers the parameter `name` and binds by
//! `column`; a field that is not a parseable column of the bound model, a
//! parameter an argument reads that the path does not declare, at any
//! registration site, refuses the router before the first request, naming
//! the route and the parameter, as an error and never a panic.

use suprnova::http::text;
use suprnova::testing::TestDatabase;
use suprnova::{
    BoundChild, FrameworkError, Request, ResourceController, Response, RouteBinding,
    RouteBindingInfo, Router, Server, any, fallback, get, group, handler, model, request, resource,
    route, routes,
};

use super::{get as get_path, refusal, run_sql, serve};

#[model(table = "su_posts", fillable = ["slug", "title"])]
pub struct SuPost {
    pub id: i64,
    pub slug: String,
    pub title: String,
    pub meta: Option<serde_json::Value>,
}

/// A type that records no columns: a binding field reaches its lookup
/// unchecked, and the lookup echoes the field it got.
pub struct SuEcho {
    field: Option<String>,
}

#[suprnova::async_trait]
impl RouteBinding for SuEcho {
    fn route_key_name() -> &'static str {
        "key"
    }
    fn route_key(&self) -> String {
        String::new()
    }
    async fn resolve_route_binding(
        _value: &str,
        field: Option<&str>,
    ) -> Result<Option<Self>, FrameworkError> {
        Ok(Some(SuEcho {
            field: field.map(str::to_owned),
        }))
    }
    async fn resolve_child_route_binding(
        &self,
        _child: &str,
        _value: &str,
        _field: Option<&str>,
    ) -> Result<Option<BoundChild>, FrameworkError> {
        Ok(None)
    }
    fn route_binding_info() -> RouteBindingInfo {
        RouteBindingInfo::of::<Self>()
    }
}

#[handler]
pub async fn show(post: SuPost) -> Response {
    text(post.title)
}

#[handler]
pub async fn echo(thing: SuEcho) -> Response {
    text(thing.field.unwrap_or_else(|| "no field".to_owned()))
}

#[handler]
pub async fn raw(req: Request) -> Response {
    text(req.param("post").map(str::to_owned).unwrap_or_default())
}

#[handler]
pub async fn by_id(id: i64) -> Response {
    text(id.to_string())
}

async fn fixture() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    run_sql(
        &db,
        &[
            "CREATE TABLE su_posts (id INTEGER PRIMARY KEY AUTOINCREMENT, slug TEXT NOT NULL, \
                title TEXT NOT NULL, meta TEXT)",
            "INSERT INTO su_posts (id, slug, title) VALUES (1, 'hello', 'Hello'), (2, '1', 'Two')",
        ],
    )
    .await;
    db
}

#[tokio::test]
async fn bind_004_a_binding_field_registers_the_parameter_by_its_name() {
    let _db = fixture().await;
    let router: Router = Router::new()
        .get("/posts/{post:slug}", show)
        .name("su.posts.show")
        .get("/raw/{post:slug}", raw)
        .where_alpha("post")
        .into();
    router
        .prepare_bindings()
        .expect("slug is a column of SuPost");
    let addr = serve(router).await;
    // The binding matches the field.
    assert_eq!(
        get_path(addr, "/posts/hello").await,
        (200, "Hello".to_owned())
    );
    assert_eq!(get_path(addr, "/posts/1").await, (200, "Two".to_owned()));
    // `req.param("post")`, the constraint and `route()` all use `post`.
    assert_eq!(get_path(addr, "/raw/abc").await, (200, "abc".to_owned()));
    assert_eq!(
        get_path(addr, "/raw/a1").await.0,
        404,
        "the constraint holds `post`"
    );
    assert_eq!(
        route("su.posts.show", &[("post", "hello")]).as_deref(),
        Some("/posts/hello")
    );
}

#[tokio::test]
async fn bind_004_an_optional_binding_field_registers_an_optional_parameter() {
    let _db = fixture().await;
    #[handler]
    pub async fn maybe(post: Option<SuPost>) -> Response {
        text(
            post.map(|post| post.title)
                .unwrap_or_else(|| "none".to_owned()),
        )
    }
    let addr = serve(Router::new().get("/maybe/{post:slug?}", maybe).into()).await;
    assert_eq!(get_path(addr, "/maybe").await, (200, "none".to_owned()));
    assert_eq!(
        get_path(addr, "/maybe/hello").await,
        (200, "Hello".to_owned())
    );
}

#[test]
fn bind_004_a_field_that_is_not_a_column_is_refused_naming_route_parameter_and_field() {
    let router: Router = Router::new().get("/posts/{post:nope}", show).into();
    let error = refusal(&router);
    assert!(error.contains("GET /posts/{post:nope}"), "{error}");
    assert!(error.contains("`nope`"), "{error}");
    assert!(error.contains("parameter `post`"), "{error}");
}

#[test]
fn bind_004_a_field_whose_type_cannot_be_parsed_from_a_segment_is_refused() {
    let router: Router = Router::new().get("/posts/{post:meta}", show).into();
    let error = refusal(&router);
    assert!(error.contains("`meta`"), "{error}");
    assert!(error.contains("cannot be parsed"), "{error}");
}

#[tokio::test]
async fn bind_004_a_field_on_a_type_without_columns_reaches_its_lookup_unchecked() {
    let router: Router = Router::new().get("/echo/{thing:anything}", echo).into();
    router
        .prepare_bindings()
        .expect("SuEcho records no columns");
    let addr = serve(router).await;
    assert_eq!(
        get_path(addr, "/echo/x").await,
        (200, "anything".to_owned())
    );
}

#[test]
fn bind_013_a_bound_argument_for_an_undeclared_parameter_is_refused() {
    let router: Router = Router::new().get("/users/{user}", show).into();
    let error = refusal(&router);
    assert!(error.contains("GET /users/{user}"), "{error}");
    assert!(error.contains("`post`"), "{error}");
    assert!(error.contains("does not declare"), "{error}");
}

#[test]
fn bind_013_a_primitive_for_an_undeclared_parameter_is_refused() {
    let router: Router = Router::new().get("/users/{user}", by_id).into();
    let error = refusal(&router);
    assert!(error.contains("GET /users/{user}"), "{error}");
    assert!(error.contains("`id`"), "{error}");
}

pub mod posts {
    use super::*;

    #[handler]
    pub async fn index() -> Response {
        text("index")
    }
    #[handler]
    pub async fn show(id: i64) -> Response {
        text(id.to_string())
    }
}

routes! {
    group!("/grouped", {
        get!("/{user}", by_id),
    }),
}

#[test]
fn bind_004_every_registration_site_is_checked() {
    // The same handler reads `id`, which no path below declares, through
    // each place the router boxes a handler. Every one must be refused.
    let sites: Vec<(&str, Router)> = vec![
        ("Router::get", Router::new().get("/a/{x}", by_id).into()),
        ("Router::post", Router::new().post("/a/{x}", by_id).into()),
        ("Router::put", Router::new().put("/a/{x}", by_id).into()),
        ("Router::patch", Router::new().patch("/a/{x}", by_id).into()),
        (
            "Router::delete",
            Router::new().delete("/a/{x}", by_id).into(),
        ),
        ("Router::head", Router::new().head("/a/{x}", by_id).into()),
        (
            "Router::options",
            Router::new().options("/a/{x}", by_id).into(),
        ),
        ("Router::any", Router::new().any("/a/{x}", by_id).into()),
        (
            "Router::methods",
            Router::new()
                .methods(&[hyper::Method::GET, hyper::Method::POST], "/a/{x}", by_id)
                .into(),
        ),
        (
            "fluent group",
            Router::new().group("/g", |r| r.get("/{x}", by_id)).into(),
        ),
        (
            "fluent group any",
            Router::new().group("/g", |r| r.any("/{x}", by_id)).into(),
        ),
        ("get! macro", get!("/a/{x}", by_id).register(Router::new())),
        ("any! macro", any!("/a/{x}", by_id).register(Router::new())),
        ("group! macro", register()),
        (
            "nested group! macro",
            group!("/outer", { group!("/inner", { get!("/{x}", by_id) }) }).register(Router::new()),
        ),
        ("fallback! macro", fallback!(by_id).register(Router::new())),
        (
            "resource! function form",
            resource!("things", posts, only = [index, show]).register(Router::new()),
        ),
    ];
    for (site, router) in sites {
        let error = router
            .prepare_bindings()
            .err()
            .unwrap_or_else(|| panic!("{site}: the undeclared `id` was not refused"));
        assert!(error.to_string().contains("`id`"), "{site}: {error}");
    }
}

#[tokio::test]
async fn bind_004_the_checks_run_for_a_router_driven_through_handle_request() {
    let _db = fixture().await;
    let router: Router = Router::new()
        .get("/users/{user}", show)
        .get("/fine/{post}", show)
        .into();
    let addr = serve(router).await;
    // A router whose checks fail answers every request with the error, as
    // a server built from it would refuse to start.
    let (status, body) = get_path(addr, "/fine/1").await;
    assert_eq!(status, 500, "{body}");
}

#[test]
fn bind_004_a_refusal_is_an_error_from_the_boot_path() {
    let router: Router = Router::new().get("/users/{user}", show).into();
    let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Server::from_config(router)));
    let outcome = result.expect("a refused router must not panic the boot");
    let error = outcome.err().expect("the boot must return the refusal");
    assert!(error.to_string().contains("GET /users/{user}"), "{error}");
}

// ── The fallback and the `missing()` handlers ─────────────────────────────

/// A fallback that reads `id`, which no fallback path declares.
#[handler]
pub async fn lost(id: i64) -> Response {
    text(format!("fallback {id}"))
}

/// A fallback the checks accept.
#[handler]
pub async fn lost_quietly() -> Response {
    text("fallback ran")
}

#[request]
pub struct SuForm {
    pub title: String,
}

/// A `missing()` handler that reads `id`, which `/posts/{post}` does not
/// declare.
#[handler]
pub async fn missing_by_id(id: i64) -> Response {
    text(format!("missing {id}"))
}

/// A `missing()` handler that reads the body twice.
#[handler]
pub async fn missing_twice(first: SuForm, second: SuForm) -> Response {
    text(format!("{} {}", first.title, second.title))
}

/// The function form of a resource over `SuPost`.
pub mod bound_posts {
    use super::*;

    #[handler]
    pub async fn show(post: SuPost) -> Response {
        text(post.title)
    }
}

/// A resource controller whose actions take the request.
pub struct SuController;

impl ResourceController for SuController {}

#[tokio::test]
async fn bind_013_a_fallback_reading_an_undeclared_parameter_is_refused_through_handle_request() {
    let router = fallback!(lost).register(Router::new());
    let error = refusal(&router);
    assert!(error.contains("the fallback route"), "{error}");
    assert!(error.contains("`id`"), "{error}");
    // An unmatched request reaches the fallback only past the checks.
    let addr = serve(router).await;
    let (status, body) = get_path(addr, "/nowhere/7").await;
    assert_eq!(status, 500, "the fallback ran past the refusal: {body}");
}

#[tokio::test]
async fn bind_004_a_fallback_beside_a_refused_route_answers_the_refusal() {
    let _db = fixture().await;
    let router = fallback!(lost_quietly).register(Router::new().get("/users/{user}", show).into());
    refusal(&router);
    let addr = serve(router).await;
    let (status, body) = get_path(addr, "/nowhere").await;
    assert_eq!(
        status, 500,
        "a router whose checks fail answers every request with the refusal, \
         the fallback's too: {body}"
    );
}

/// Every place a `missing()` handler is installed, each on a route whose
/// own handler the checks accept.
fn missing_sites<H, Fut>(hook: H) -> Vec<(&'static str, Router)>
where
    H: Fn(Request) -> Fut + Clone + Send + Sync + 'static,
    Fut: std::future::Future<Output = Response> + Send + 'static,
{
    vec![
        (
            "RouteBuilder::missing",
            Router::new()
                .get("/posts/{post}", show)
                .missing(hook.clone())
                .into(),
        ),
        (
            "any route missing",
            Router::new()
                .any("/posts/{post}", show)
                .missing(hook.clone())
                .into(),
        ),
        (
            "route macro missing",
            get!("/posts/{post}", show)
                .missing(hook.clone())
                .register(Router::new()),
        ),
        (
            "group! macro missing",
            group!("/g", { get!("/posts/{post}", show) })
                .missing(hook.clone())
                .register(Router::new()),
        ),
        (
            "fluent group missing",
            Router::new()
                .group("/g", |r| r.get("/posts/{post}", show))
                .missing(hook.clone())
                .into(),
        ),
        (
            "resource! function form missing",
            resource!("posts", bound_posts, only = [show])
                .missing(hook.clone())
                .unnamed()
                .register(Router::new()),
        ),
        (
            "resource controller missing",
            Router::new()
                .resource("posts", SuController)
                .missing(hook)
                .unnamed()
                .into(),
        ),
    ]
}

#[test]
fn bind_013_a_missing_handler_reading_an_undeclared_parameter_is_refused() {
    for (site, router) in missing_sites(missing_by_id) {
        let error = router
            .prepare_bindings()
            .err()
            .unwrap_or_else(|| panic!("{site}: the hook's undeclared `id` was not refused"));
        let error = error.to_string();
        assert!(error.contains("`missing()` handler"), "{site}: {error}");
        assert!(error.contains("`id`"), "{site}: {error}");
        assert!(error.contains("/posts/{post}"), "{site}: {error}");
    }
}

#[test]
fn bind_015_a_missing_handler_with_two_body_readers_is_refused() {
    for (site, router) in missing_sites(missing_twice) {
        let error = router
            .prepare_bindings()
            .err()
            .unwrap_or_else(|| panic!("{site}: the hook's two body readers were not refused"));
        let error = error.to_string();
        assert!(error.contains("`first: SuForm`"), "{site}: {error}");
        assert!(error.contains("`second: SuForm`"), "{site}: {error}");
    }
}

#[tokio::test]
async fn bind_013_a_refused_missing_handler_refuses_a_router_driven_through_handle_request() {
    let _db = fixture().await;
    let router: Router = Router::new()
        .get("/posts/{post}", show)
        .missing(missing_by_id)
        .into();
    let addr = serve(router).await;
    // The row exists, so the hook would never run; the refusal still
    // answers, as the server would refuse to start.
    let (status, body) = get_path(addr, "/posts/1").await;
    assert_eq!(status, 500, "{body}");
}
