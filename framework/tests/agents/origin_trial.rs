//! The WebMCP Origin Trial token (AGENT-014): with
//! `WEBMCP_ORIGIN_TRIAL_TOKEN` set, every HTML response carries the
//! `Origin-Trial` header, and nothing else does. The token is process-wide
//! configuration, so each test runs alone.

use serde_json::json;
use suprnova::agents::AgentsConfig;
use suprnova::testing::{TestClient, TestResponse};
use suprnova::{
    Config, HttpResponse, InertiaResponse, MiddlewareRegistry, Request, Response, Router,
    WebMcpConfig,
};

use crate::{fixture, live_fixture};

/// The token the tests configure: base64, as Chrome issues one.
const TOKEN: &str = "QXBwbGljYXRpb24gdG9rZW4gZm9yIFdlYk1DUA==";

async fn inertia_page(request: Request) -> Response {
    InertiaResponse::new("Home")
        .resolve(&request)
        .await
        .map_err(HttpResponse::from)
}

/// The fixture's routes with pages of every kind of answer.
fn routes() -> Router {
    fixture::define_policies();
    let router: Router = fixture::routes()
        .get("/inertia", inertia_page)
        .get("/view", |_request: Request| async {
            Ok(HttpResponse::html(
                "<!doctype html><html><head></head><body>view</body></html>",
            ))
        })
        .get("/view-upper", |_request: Request| async {
            Ok(HttpResponse::new().header("Content-Type", "TEXT/HTML; charset=UTF-8"))
        })
        .get("/missing", |_request: Request| async {
            Err::<HttpResponse, HttpResponse>(HttpResponse::html("<h1>Not Found</h1>").status(404))
        })
        .get("/text", |_request: Request| async {
            Ok(HttpResponse::text("plain"))
        })
        .get("/away", |_request: Request| async {
            Ok(HttpResponse::new().status(302).header("Location", "/view"))
        })
        .into();
    router.agents(AgentsConfig::new())
}

fn client() -> TestClient {
    TestClient::new(routes(), MiddlewareRegistry::new())
}

/// A client serving `dir`'s files when no route matches.
fn static_client(dir: &std::path::Path) -> TestClient {
    let router =
        suprnova::fallback!(suprnova::StaticFiles::from_dir(dir).handler()).register(Router::new());
    TestClient::new(router, MiddlewareRegistry::new())
}

fn trial(response: &TestResponse) -> Option<&str> {
    response.header("origin-trial")
}

fn configure_token() {
    Config::register(
        WebMcpConfig::new()
            .with_origin_trial_token(TOKEN)
            .expect("a base64 token"),
    );
}

#[tokio::test]
async fn agent_014_every_html_response_carries_the_origin_trial_header() {
    if crate::own_process_async::delegate(
        module_path!(),
        "agent_014_every_html_response_carries_the_origin_trial_header",
    )
    .await
    {
        return;
    }
    let _container = fixture::setup();
    configure_token();
    let client = client();

    let inertia = client.get("/inertia").send().await;
    inertia.assert_ok();
    assert!(
        inertia
            .header("content-type")
            .is_some_and(|value| value.starts_with("text/html"))
    );
    assert_eq!(
        trial(&inertia),
        Some(TOKEN),
        "an Inertia page's first visit"
    );

    let view = client.get("/view").send().await;
    assert_eq!(trial(&view), Some(TOKEN), "a plain view");

    let upper = client.get("/view-upper").send().await;
    assert_eq!(trial(&upper), Some(TOKEN), "a media type in upper case");

    let missing = client
        .get("/missing")
        .header("Accept", "text/html")
        .send()
        .await;
    assert_eq!(missing.status(), 404);
    assert!(
        missing
            .header("content-type")
            .is_some_and(|value| value.starts_with("text/html")),
        "{:?}",
        missing.header("content-type")
    );
    assert_eq!(trial(&missing), Some(TOKEN), "an HTML error page");

    let unrouted = client
        .get("/no-such-page")
        .header("Accept", "text/html")
        .send()
        .await;
    assert_eq!(unrouted.status(), 404);
    match unrouted.header("content-type") {
        Some(value) if value.starts_with("text/html") => {
            assert_eq!(trial(&unrouted), Some(TOKEN), "the router's own HTML 404");
        }
        _ => assert_eq!(trial(&unrouted), None, "the router's own 404 is not HTML"),
    }

    let dir = tempfile::tempdir().expect("a public directory");
    std::fs::write(dir.path().join("about.html"), "<!doctype html><p>about</p>")
        .expect("write a static page");
    let file = static_client(dir.path()).get("/about.html").send().await;
    file.assert_ok();
    assert_eq!(trial(&file), Some(TOKEN), "a static .html file");

    let live = live_fixture::live_client()
        .get(live_fixture::PAGE)
        .send()
        .await;
    assert_eq!(live.status(), 200, "{}", live.body_text());
    assert_eq!(trial(&live), Some(TOKEN), "a Live document");
}

#[tokio::test]
async fn agent_014_non_html_responses_carry_no_origin_trial_header() {
    if crate::own_process_async::delegate(
        module_path!(),
        "agent_014_non_html_responses_carry_no_origin_trial_header",
    )
    .await
    {
        return;
    }
    let _container = fixture::setup();
    configure_token();
    let client = client();

    let tool = client.tool("echo", json!({ "text": "hi" })).send().await;
    tool.assert_ok();
    assert_eq!(trial(&tool), None, "a tool's JSON answer");

    let visit = client
        .get("/inertia")
        .header("X-Inertia", "true")
        .send()
        .await;
    visit.assert_ok();
    assert_eq!(trial(&visit), None, "an Inertia JSON visit");

    let redirect = client.get("/away").send().await;
    assert!(
        (300..400).contains(&redirect.status()),
        "{}",
        redirect.status()
    );
    assert_eq!(trial(&redirect), None, "a redirect");

    let text = client.get("/text").send().await;
    assert_eq!(trial(&text), None, "plain text");

    let mcp = client.mcp().list().await;
    assert_eq!(trial(mcp.response()), None, "the MCP endpoint");

    let dir = tempfile::tempdir().expect("a public directory");
    std::fs::write(dir.path().join("app.css"), "body{}").expect("write a stylesheet");
    let asset = static_client(dir.path()).get("/app.css").send().await;
    asset.assert_ok();
    assert_eq!(trial(&asset), None, "an asset");
}

#[tokio::test]
async fn agent_014_no_token_no_header() {
    if crate::own_process_async::delegate(module_path!(), "agent_014_no_token_no_header").await {
        return;
    }
    let _container = fixture::setup();
    let client = client();
    let view = client.get("/view").send().await;
    view.assert_ok();
    assert_eq!(trial(&view), None);
    Config::register(WebMcpConfig::new());
    let view = client.get("/view").send().await;
    assert_eq!(trial(&view), None, "a configuration without a token");
}

#[test]
fn agent_014_an_invalid_origin_trial_token_stops_boot() {
    crate::own_process::run_alone(
        "origin_trial::agent_014_an_invalid_origin_trial_token_stops_boot_child",
    );
}

#[test]
fn agent_014_an_invalid_origin_trial_token_stops_boot_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let root = tempfile::tempdir().expect("a project root without .env");
    // SAFETY: this test runs alone in its own process, before any thread of
    // its own starts; nothing else reads the environment meanwhile.
    unsafe {
        std::env::set_var("WEBMCP_ORIGIN_TRIAL_TOKEN", "not a token!");
    }
    let error = Config::init(root.path()).expect_err("an invalid token stops boot");
    let text = format!("{error} {error:?}");
    assert!(text.contains("WEBMCP_ORIGIN_TRIAL_TOKEN"), "{text}");
    assert!(
        Config::get::<WebMcpConfig>().is_none(),
        "a failed boot registers nothing"
    );
    // SAFETY: as above.
    unsafe {
        std::env::set_var("WEBMCP_ORIGIN_TRIAL_TOKEN", TOKEN);
    }
    Config::init(root.path()).expect("a valid token boots");
    assert_eq!(
        Config::get::<WebMcpConfig>()
            .and_then(|config| config.origin_trial_token().map(str::to_string))
            .as_deref(),
        Some(TOKEN)
    );
}
