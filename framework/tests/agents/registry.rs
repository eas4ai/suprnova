//! AGENT-001: what is a tool, and the boot check of every tool against the
//! application's routes.

use suprnova::agents::AgentsConfig;
use suprnova::{MiddlewareRegistry, Router};

use crate::fixture::{self, echo, page, peek, slow, throttled};

#[test]
fn agent_001_marked_handlers_are_tools_and_nothing_else_is() {
    let router = fixture::router();
    let registry = router.tool_registry().expect("the fixture's tools check");
    let names: Vec<&str> = registry.tools().iter().map(|tool| tool.name()).collect();
    assert_eq!(
        names,
        vec![
            "echo",
            "page.html",
            "posts.create",
            "posts.delete",
            "posts.show",
            "request.peek",
            "slow.wait",
            "throttled.ping",
            "users.profile",
        ]
    );
    // `/plain` is a routed `#[handler]` without `#[tool]`, and `/posts/new`
    // a closure route: neither is offered.
    assert!(registry.get("plain").is_none());
    let show = registry.get("posts.show").expect("posts.show is a tool");
    assert_eq!(show.pattern(), "/posts/{post}");
    assert_eq!(show.method().as_str(), "GET");
}

/// The fixture's routes without `users.profile`'s.
fn routes_without_profile() -> Router {
    Router::new()
        .post("/echo", echo)
        .post("/posts", fixture::create_post)
        .get("/posts/{post}", fixture::show_post)
        .delete("/posts/{post}", fixture::delete_post)
        .post("/slow", slow)
        .post("/peek", peek)
        .post("/throttled", throttled)
        .get("/page", page)
        .into()
}

#[tokio::test]
async fn agent_001_unrouted_tool_refuses_boot() {
    let _container = fixture::setup();
    let router = routes_without_profile().agents(AgentsConfig::new());
    let error = router
        .prepare_tools()
        .expect_err("a tool whose handler is not routed refuses boot");
    assert!(error.to_string().contains("users.profile"), "{error}");

    // A router whose check fails answers every request with the error, so
    // the application's own tests fail too.
    let client = suprnova::testing::TestClient::new(router, MiddlewareRegistry::new());
    client
        .post("/echo")
        .json(&serde_json::json!({ "text": "hi" }))
        .send()
        .await
        .assert_status(500);
}

#[test]
fn agent_001_a_handler_on_two_routes_needs_a_route_name() {
    let router: Router = fixture::routes().post("/me-again", fixture::profile).into();
    let error = router
        .prepare_tools()
        .expect_err("a tool on two routes names one of them");
    assert!(error.to_string().contains("route = "), "{error}");
}

#[test]
fn agent_001_a_path_parameter_must_be_an_input_field() {
    let router: Router = routes_without_profile()
        .post("/me/{tenant}", fixture::profile)
        .into();
    let error = router
        .prepare_tools()
        .expect_err("a route parameter the input lacks refuses boot");
    assert!(error.to_string().contains("`tenant`"), "{error}");
}

#[tokio::test]
async fn agent_001_an_unmarked_route_still_answers_as_a_route() {
    let _container = fixture::setup();
    let client = fixture::client();
    client
        .post("/plain")
        .send()
        .await
        .assert_ok()
        .assert_json(serde_json::json!({ "plain": true }));
}

/// The fixture's routes with `AuthMiddleware` on `posts.create`, a tool that
/// accepts agent callers.
fn routes_with_auth_on_an_agent_tool() -> Router {
    Router::new()
        .post("/echo", echo)
        .post("/posts", fixture::create_post)
        .middleware(suprnova::AuthMiddleware::new())
        .get("/posts/{post}", fixture::show_post)
        .delete("/posts/{post}", fixture::delete_post)
        .post("/me", fixture::profile)
        .post("/slow", slow)
        .post("/peek", peek)
        .post("/throttled", throttled)
        .get("/page", page)
        .into()
}

#[test]
#[tracing_test::traced_test]
fn agent_002_an_agent_tool_behind_auth_warns_at_boot() {
    let router = routes_with_auth_on_an_agent_tool();
    router
        .prepare_tools()
        .expect("the warning does not refuse boot");
    assert!(logs_contain("posts.create"));
    assert!(logs_contain("behind AuthMiddleware"));
}
