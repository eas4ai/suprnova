//! Route TS keys must always be valid, unquoted TypeScript identifiers.
//!
//! When several routes in a module share one handler (e.g. a `static_files::serve`
//! whitelist mapping many favicon/asset URLs), the first keeps the handler name
//! and the rest get a key derived from the route name/path. Those derived keys
//! must be sanitized: a file extension ('.') or a leading digit would otherwise
//! produce output like `favicon_16x16.png: (...) => ...` that fails tsc/svelte-check.

use suprnova_cli::commands::generate_routes::{
    GeneratedRoute, HttpMethod, RouteDefinition, generate_typescript,
};

fn route(path: &str, handler_fn: &str, name: Option<&str>) -> GeneratedRoute {
    GeneratedRoute {
        definition: RouteDefinition {
            method: HttpMethod::Get,
            path: path.to_string(),
            handler_module: "controllers::static_files".to_string(),
            handler_fn: handler_fn.to_string(),
            name: name.map(|n| n.to_string()),
            path_params: Vec::new(),
            component: None,
        },
        handler_info: None,
        request_struct: None,
    }
}

/// Slice out the `static_files: { ... }` block from the generated controllers object.
/// A module block closes with a line at 2-space indent (`\n  }`); route lines are
/// indented 4 spaces, so that marker unambiguously ends the block (inline arrow-body
/// `}`s sit mid-line and are never preceded by a newline + 2 spaces).
fn static_files_block(ts: &str) -> String {
    let start = ts
        .find("static_files: {")
        .expect("static_files block not found");
    let after = &ts[start..];
    let end = after.find("\n  }").expect("block close not found");
    after[..end].to_string()
}

#[test]
fn duplicate_handler_keys_are_valid_identifiers() {
    // All five routes hit the same `serve` handler, so four get path/name-derived keys.
    let routes = vec![
        route("/favicon.ico", "serve", None),
        route("/favicon-16x16.png", "serve", None),
        route("/site.webmanifest", "serve", None),
        route("/2fa.json", "serve", None), // leading digit after sanitizing
        route("/whatever", "serve", Some("assets.hero-image")), // dashed name segment
    ];

    let ts = generate_typescript(&routes);
    let block = static_files_block(&ts);

    // First occurrence keeps the handler name.
    assert!(block.contains("serve:"), "block: {block}");

    // No key may contain a '.' - that is the exact bug (member access, not a key).
    assert!(
        !block.contains(".png:") && !block.contains(".webmanifest:") && !block.contains(".json:"),
        "leaked a dotted key: {block}"
    );

    // Extension dots become underscores.
    assert!(block.contains("favicon_16x16_png:"), "block: {block}");
    assert!(block.contains("site_webmanifest:"), "block: {block}");

    // A key that would start with a digit is prefixed so it stays a legal identifier.
    assert!(block.contains("_2fa_json:"), "block: {block}");

    // A dashed route-name segment is sanitized too (no '-' in an identifier).
    assert!(block.contains("hero_image:"), "block: {block}");
    assert!(!block.contains("hero-image:"), "leaked dashed key: {block}");
}

#[test]
fn unique_handler_names_are_untouched() {
    // Distinct handlers keep their clean names - no spurious sanitizing.
    let routes = vec![
        route("/favicon.ico", "serve", None),
        route("/health", "healthcheck", None),
    ];
    let ts = generate_typescript(&routes);
    let block = static_files_block(&ts);
    assert!(block.contains("serve:"), "block: {block}");
    assert!(block.contains("healthcheck:"), "block: {block}");
}

// PAR-070: a route helper carries the page component its handler renders,
// as Inertia's `UrlMethodPair.component`, so an instant visit can render
// the page before the server answers.

/// A project with `src/routes.rs` and `src/controllers/users.rs`, scanned
/// and generated the way `generate-types --routes` does.
fn routes_ts(routes: &str, users: &str) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    let src = dir.path().join("src");
    std::fs::create_dir_all(src.join("controllers")).expect("create src/controllers");
    std::fs::write(src.join("routes.rs"), routes).expect("write routes.rs");
    std::fs::write(src.join("controllers/users.rs"), users).expect("write users.rs");
    let scanned =
        suprnova_cli::commands::generate_routes::scan_routes(dir.path()).expect("scan the routes");
    generate_typescript(&scanned)
}

const USERS: &str = r#"
use suprnova::{handler, inertia_response, InertiaResponse, Request, Response};

#[handler]
pub async fn index(req: Request) -> Response {
    inertia_response!(&req, "Users/Index", { "users": [] })
}

#[handler]
pub async fn show(req: Request, id: i64) -> Response {
    InertiaResponse::new("Users/Show").with("id", id).resolve(&req).await
}

#[handler]
pub async fn listing(req: Request) -> Response {
    if req.query("archived").is_some() {
        inertia_response!(&req, "Users/Archived", { "users": [] })
    } else {
        inertia_response!(&req, "Users/Index", { "users": [] })
    }
}

#[handler]
pub async fn chosen(req: Request) -> Response {
    let page = if req.query("compact").is_some() { "Users/Compact" } else { "Users/Index" };
    InertiaResponse::new(page).resolve(&req).await
}

#[handler]
pub async fn store(_req: Request) -> Response {
    redirect!("/users")
}
"#;

const ROUTES: &str = r#"
use suprnova::{get, post, routes};

routes! {
    get!("/users", controllers::users::index).name("users.index"),
    get!("/users/{id}", controllers::users::show),
    get!("/listing", controllers::users::listing),
    get!("/chosen", controllers::users::chosen),
    post!("/users", controllers::users::store),
}
"#;

/// The helper line whose key is `key`.
fn helper<'a>(ts: &'a str, key: &str) -> &'a str {
    ts.lines()
        .find(|line| line.trim_start().starts_with(&format!("{key}: (")))
        .unwrap_or_else(|| panic!("no `{key}` helper in:\n{ts}"))
}

#[test]
fn intt_a_handler_naming_one_component_carries_it() {
    let ts = routes_ts(ROUTES, USERS);
    assert!(
        helper(&ts, "index")
            .ends_with("({ url: '/users', method: 'get', component: 'Users/Index' }),"),
        "{ts}"
    );
    assert!(
        helper(&ts, "show").contains("method: 'get', component: 'Users/Show' })"),
        "{ts}"
    );
}

#[test]
fn intt_a_handler_naming_none_or_several_components_carries_none() {
    let ts = routes_ts(ROUTES, USERS);
    for key in ["listing", "chosen", "store"] {
        assert!(
            !helper(&ts, key).contains("component"),
            "`{key}` names no single component: {ts}"
        );
    }
}

#[test]
fn intt_a_router_inertia_route_carries_its_component() {
    let ts = routes_ts(
        r#"
use suprnova::Router;
use serde_json::json;

pub fn register() -> Router {
    Router::new()
        .inertia("/about", "About", json!({ "team_size": 4 }))
        .name("about")
}
"#,
        USERS,
    );
    assert!(
        helper(&ts, "about").contains("({ url: '/about', method: 'get', component: 'About' })"),
        "{ts}"
    );
    assert!(
        ts.contains("'about': controllers.inertia.about"),
        "the named lookup reaches the helper: {ts}"
    );
}

#[test]
fn intt_route_config_declares_the_component() {
    let ts = generate_typescript(&[]);
    let config = ts
        .split("export interface RouteConfig<TData = void> {")
        .nth(1)
        .and_then(|rest| rest.split("\n}\n").next())
        .unwrap_or_else(|| panic!("no RouteConfig in:\n{ts}"));
    assert!(config.contains("  component?: string;"), "{config}");
    assert!(config.contains("UrlMethodPair"), "{config}");
}

// BIND-003: a handler inside an `impl` block is registered through its
// type's path, and its helper is read from the method as a free
// function's is from the function.

/// A project holding `files` under `src/`, scanned and generated the way
/// `generate-types --routes` does.
fn project_routes_ts(files: &[(&str, &str)]) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    for (path, body) in files {
        let path = dir.path().join("src").join(path);
        std::fs::create_dir_all(path.parent().expect("a file under src/"))
            .expect("create source directory");
        std::fs::write(path, body).expect("write source");
    }
    let scanned =
        suprnova_cli::commands::generate_routes::scan_routes(dir.path()).expect("scan the routes");
    generate_typescript(&scanned)
}

/// The body of the `module: { .. }` block of the `controllers` object.
fn module_block<'a>(ts: &'a str, module: &str) -> &'a str {
    let head = format!("\n  {module}: {{\n");
    let start = ts
        .find(&head)
        .unwrap_or_else(|| panic!("no `{module}` module in:\n{ts}"))
        + head.len();
    let end = ts[start..].find("\n  }").expect("module block close") + start;
    &ts[start..end]
}

const POSTS: &str = r#"
use suprnova::{handler, inertia_response, request, Request, Response};

#[request]
pub struct UserFilters {
    pub search: String,
}

#[request]
pub struct PostFilters {
    pub tag: String,
}

pub struct Posts;

impl Posts {
    #[handler(Self = Posts)]
    pub async fn index(req: Request, filters: UserFilters) -> Response {
        inertia_response!(&req, "Users/Index", { "search": filters.search })
    }
}

#[handler]
pub async fn index(req: Request, filters: PostFilters) -> Response {
    inertia_response!(&req, "Posts/Index", { "tag": filters.tag })
}
"#;

const POSTS_ROUTES: &str = r#"
use suprnova::{get, routes};

routes! {
    get!("/users", controllers::posts::Posts::index).name("users.index"),
    get!("/posts", controllers::posts::index).name("posts.index"),
}
"#;

#[test]
fn intt_an_associated_handler_carries_its_component_and_request_type() {
    let ts = project_routes_ts(&[("routes.rs", POSTS_ROUTES), ("controllers/posts.rs", POSTS)]);
    assert_eq!(
        module_block(&ts, "Posts"),
        "    index: (data: UserFilters): RouteConfig<UserFilters> => \
         ({ url: '/users', method: 'get', data, component: 'Users/Index' })",
        "`Posts::index` is read from the method inside `impl Posts`:\n{ts}"
    );
    assert!(ts.contains("export interface UserFilters {"), "{ts}");
}

#[test]
fn intt_a_free_handler_beside_an_associated_one_keeps_its_own() {
    let ts = project_routes_ts(&[("routes.rs", POSTS_ROUTES), ("controllers/posts.rs", POSTS)]);
    assert_eq!(
        module_block(&ts, "posts"),
        "    index: (data: PostFilters): RouteConfig<PostFilters> => \
         ({ url: '/posts', method: 'get', data, component: 'Posts/Index' })",
        "the free `index` is not the method of the same name:\n{ts}"
    );
}
