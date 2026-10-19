//! The server tools a page offers in the browser (AGENT-014): what a page
//! lists for each viewer, how the list reaches an Inertia page, a Live page
//! and a plain view, the boot checks, the two checks on a browser tool call,
//! and RenderCache keeping one viewer's list from another.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};
use suprnova::agents::AgentsConfig;
use suprnova::live::testing::prepare_live_router_for_test;
use suprnova::live::{CanonicalValue, LiveBootstrapOptions, LiveDocument, LiveMount, MountFlags};
use suprnova::testing::{TestClient, TestResponse};
use suprnova::view::{AssetSet, DocumentResponseIntent, ViewName};
use suprnova::{
    FrameworkError, HttpResponse, InertiaResponse, JsonSchema, MiddlewareRegistry, PageTools,
    Request, Response, Router, StatusCode, Validate, handler, json_response, tool, tool_input,
};

use crate::fixture::{self, TestUser};
use crate::live_fixture::{self, AgentCounter, AgentPage, AgentQuota};

// ---------------------------------------------------------------- tools ----

/// The input of `notes.lookup`.
#[tool_input]
pub struct NoteQuery {
    /// What to look for.
    pub q: String,
}

#[tool(
    name = "notes.search",
    description = "List the latest notes.",
    callers = [public],
    annotations(read_only, untrusted_content),
)]
#[handler]
pub async fn search_notes(request: Request) -> Response {
    json_response!({ "q": request.query_param("q") })
}

#[tool(
    name = "notes.lookup",
    description = "Look a note up, reading the body on GET.",
    callers = [public],
    input = NoteQuery,
)]
#[handler]
pub async fn lookup_notes(form: NoteQuery) -> Response {
    json_response!({ "q": form.q })
}

/// The input of `notes.show`: its path parameter, and a scalar beside it.
#[tool_input]
pub struct NoteRef {
    /// The note.
    pub note: String,
    /// How to show it.
    pub format: Option<String>,
}

#[tool(
    name = "notes.show",
    description = "Show one note.",
    callers = [public],
    input = NoteRef,
    annotations(read_only),
)]
#[handler]
pub async fn show_note(note: String, form: NoteRef) -> Response {
    // The path value and the argument the query carried agree.
    json_response!({ "note": form.note, "format": form.format, "same": note == form.note })
}

/// What `notes.filter` narrows the notes by.
#[derive(serde::Deserialize, Validate, JsonSchema)]
#[schemars(crate = "suprnova::__schemars")]
pub struct NoteFilters {
    /// Whether to list drafts.
    pub drafts: bool,
}

/// The input of `notes.filter`: a structured field, which a query cannot
/// carry.
#[tool_input]
pub struct NoteFilter {
    /// The filters.
    #[validate(nested)]
    pub filters: NoteFilters,
}

#[tool(
    name = "notes.filter",
    description = "Filter the notes.",
    callers = [public],
    input = NoteFilter,
)]
#[handler]
pub async fn filter_notes(form: NoteFilter) -> Response {
    json_response!({ "drafts": form.filters.drafts })
}

#[tool(
    name = "audit.export",
    description = "Export the audit log for an agent account.",
    callers = [agent],
)]
#[handler]
pub async fn export_audit() -> Response {
    json_response!({ "exported": true })
}

// ------------------------------------------------------------- the app ----

/// Signs a user in for the request alone when it carries
/// `X-Stand-In-Token`, as a bearer token middleware would: a direct caller,
/// holding no scope.
struct StandInToken;

#[suprnova::async_trait]
impl suprnova::Middleware for StandInToken {
    async fn handle(&self, request: Request, next: suprnova::Next) -> Response {
        if request.header("X-Stand-In-Token").is_some() {
            suprnova::Auth::set_user(TestUser::arc("token-user"));
        }
        next(request).await
    }
}

async fn inertia_page(request: Request) -> Response {
    InertiaResponse::new("Posts/Index")
        .resolve(&request)
        .await
        .map_err(HttpResponse::from)
}

async fn inertia_with_additions(request: Request) -> Response {
    PageTools::add(["posts.create"])
        .await
        .map_err(HttpResponse::from)?;
    InertiaResponse::new("Posts/Index")
        .with_tools(["request.peek"])
        .resolve(&request)
        .await
        .map_err(HttpResponse::from)
}

async fn plain_page(request: Request) -> Response {
    let element = PageTools::element(&request).map_err(HttpResponse::from)?;
    Ok(HttpResponse::html(format!(
        "<!doctype html><html><head><title>Plain</title>{element}</head><body><p>plain</p></body></html>"
    )))
}

/// The fixture's routes, the page tests' tools and pages, with
/// `notes.search` shared by every page.
fn page_routes() -> Router {
    fixture::define_policies();
    let router: Router = fixture::routes()
        .get("/notes/search", search_notes)
        .get("/notes/lookup", lookup_notes)
        .get("/notes/filter", filter_notes)
        .get("/notes/{note}", show_note)
        .post("/audit", export_audit)
        .get("/inertia/posts", inertia_page)
        .with_tools(["echo", "users.profile", "posts.create", "posts.delete"])
        .middleware(StandInToken)
        .get("/inertia/notes", inertia_page)
        .with_tools(["notes.show"])
        .get("/plain/posts", plain_page)
        .with_tools(["echo", "users.profile", "posts.create"])
        .middleware(StandInToken)
        .get("/inertia/bare", inertia_page)
        .into();
    let router = suprnova::group!("/grouped", {
        suprnova::get!("/page", inertia_with_additions).with_tools(["echo"])
    })
    .with_tools(["users.profile"])
    .register(router);
    router.share_tools(["notes.search"])
}

/// A client of [`page_routes`] with no global middleware.
fn page_client() -> TestClient {
    TestClient::new(
        page_routes().agents(AgentsConfig::new()),
        MiddlewareRegistry::new(),
    )
}

fn seven() -> TestUser {
    TestUser {
        id: "7".to_string(),
    }
}

/// The Inertia page at `path`, as an Inertia visit.
async fn visit(client: TestClient, path: &str) -> TestResponse {
    let response = client.get(path).header("X-Inertia", "true").send().await;
    response.assert_ok();
    response
}

/// Whether `html` carries the page tools element inside its `<head>`.
fn element_in_head(html: &str) -> bool {
    let element = html.find("<script type=\"application/json\" id=\"suprnova-agent-tools\">");
    let head_end = html.find("</head>");
    let body = html.find("<body");
    matches!((element, head_end), (Some(element), Some(end)) if element < end)
        && body.is_none_or(|body| element.is_some_and(|element| element < body))
}

// --------------------------------------------------------- the viewers ----

#[tokio::test]
async fn agent_014_page_lists_only_tools_the_viewer_may_call() {
    let _container = fixture::setup();
    let client = page_client();

    let guest = visit(client.clone(), "/inertia/posts").await;
    assert_eq!(guest.page_tools(), ["echo", "notes.search"]);

    // A session user holds every scope.
    let session = visit(client.clone().acting_as(&seven()), "/inertia/posts").await;
    assert_eq!(
        session.page_tools(),
        [
            "echo",
            "notes.search",
            "posts.create",
            "posts.delete",
            "users.profile"
        ]
    );

    // A user signed in another way holds no scope.
    let direct = client
        .get("/inertia/posts")
        .header("X-Inertia", "true")
        .header("X-Stand-In-Token", "1")
        .send()
        .await;
    direct.assert_ok();
    assert_eq!(
        direct.page_tools(),
        ["echo", "notes.search", "users.profile"]
    );

    // The first visit's HTML document carries the same list.
    let first = client.get("/inertia/posts").send().await;
    first.assert_ok();
    assert_eq!(first.page_tools(), ["echo", "notes.search"]);

    // A page with no tool of its own offers the shared ones.
    let bare = visit(client.clone(), "/inertia/bare").await;
    assert_eq!(bare.page_tools(), ["notes.search"]);
}

#[tokio::test]
async fn agent_014_a_plain_view_carries_the_element_in_its_head() {
    let _container = fixture::setup();
    let client = page_client();
    let guest = client.get("/plain/posts").send().await;
    guest.assert_ok();
    let html = guest.body_text();
    assert!(element_in_head(&html), "{html}");
    assert_eq!(guest.page_tools(), ["echo", "notes.search"]);
    let session = client
        .clone()
        .acting_as(&seven())
        .get("/plain/posts")
        .send()
        .await;
    assert_eq!(
        session.page_tools(),
        ["echo", "notes.search", "posts.create", "users.profile"]
    );
}

#[tokio::test]
async fn agent_014_a_page_that_offers_no_tool_carries_no_element() {
    let _container = fixture::setup();
    // No shared tool, and the route names none.
    let router: Router = fixture::routes().get("/plain/none", plain_page).into();
    let client = TestClient::new(
        router.agents(AgentsConfig::new()),
        MiddlewareRegistry::new(),
    );
    let page = client.get("/plain/none").send().await;
    page.assert_ok();
    assert!(
        !page.body_text().contains("suprnova-agent-tools"),
        "{}",
        page.body_text()
    );
    // A page whose only tool the viewer cannot call carries none either.
    let router: Router = fixture::routes()
        .get("/plain/users", plain_page)
        .with_tools(["users.profile"])
        .into();
    let client = TestClient::new(
        router.agents(AgentsConfig::new()),
        MiddlewareRegistry::new(),
    );
    let guest = client.get("/plain/users").send().await;
    guest.assert_ok();
    assert!(
        !guest.body_text().contains("suprnova-agent-tools"),
        "{}",
        guest.body_text()
    );
    let inertia = TestClient::new(
        {
            let router: Router = fixture::routes()
                .get("/inertia/users", inertia_page)
                .with_tools(["users.profile"])
                .into();
            router.agents(AgentsConfig::new())
        },
        MiddlewareRegistry::new(),
    );
    let page = visit(inertia, "/inertia/users").await;
    assert!(
        page.inertia_page().get("agentTools").is_none(),
        "{}",
        page.body_text()
    );
}

#[tokio::test]
async fn agent_014_response_tools_join_route_group_and_shared_tools() {
    let _container = fixture::setup();
    let client = page_client().acting_as(&seven());
    let page = visit(client, "/grouped/page").await;
    assert_eq!(
        page.page_tools(),
        [
            "echo",
            "notes.search",
            "posts.create",
            "request.peek",
            "users.profile"
        ],
        "the group's, the route's, the shared, the request's and the response's tools"
    );
}

#[tokio::test]
async fn agent_014_a_response_naming_a_tool_a_browser_cannot_call_fails() {
    let _container = fixture::setup();
    async fn naming(request: Request, name: &'static str) -> Response {
        InertiaResponse::new("Posts/Index")
            .with_tools([name])
            .resolve(&request)
            .await
            .map_err(HttpResponse::from)
    }
    let router: Router = fixture::routes()
        .get("/unknown", |request: Request| {
            naming(request, "no.such.tool")
        })
        .get("/agent-only", |request: Request| {
            naming(request, "audit.export")
        })
        .post("/audit", export_audit)
        .into();
    let client = TestClient::new(
        router.agents(AgentsConfig::new()),
        MiddlewareRegistry::new(),
    );
    for path in ["/unknown", "/agent-only"] {
        let page = client.get(path).header("X-Inertia", "true").send().await;
        assert_eq!(page.status(), 500, "{path}: {}", page.body_text());
    }
}

// --------------------------------------------------------- annotations ----

#[tokio::test]
async fn agent_014_page_tool_annotations_follow_the_markings() {
    let _container = fixture::setup();
    let router = page_routes().agents(AgentsConfig::new());
    let registry = router.tool_registry().expect("the tools check");
    let client = TestClient::new(router, MiddlewareRegistry::new()).acting_as(&seven());
    let page = visit(client, "/inertia/posts").await;
    let payload = page.page_tools_payload().expect("the page offers tools");
    assert_eq!(payload["v"], 1);
    assert_eq!(
        payload["limits"],
        json!({ "maxResultBytes": 8 * 1024 * 1024, "callTimeoutMs": 30_000 })
    );

    let create = page.page_tool("posts.create").expect("posts.create");
    assert_eq!(create["title"], "Create a post");
    assert_eq!(
        create["description"],
        "Create a draft post for the signed-in user."
    );
    assert_eq!(
        create["annotations"],
        json!({ "readOnlyHint": false, "untrustedContentHint": false, "consequentialHint": false })
    );
    assert_eq!(create["method"], "POST");
    assert_eq!(create["path"], json!([{ "text": "/posts" }]));
    assert_eq!(
        &create["inputSchema"],
        registry
            .get("posts.create")
            .expect("registered")
            .input_schema(),
        "the schema the MCP endpoint lists"
    );
    assert!(create.get("outputSchema").is_none(), "{create}");

    let echo = page.page_tool("echo").expect("echo");
    assert!(
        echo.get("title").is_none(),
        "a tool without a title has none: {echo}"
    );
    assert_eq!(
        echo["annotations"],
        json!({ "readOnlyHint": true, "untrustedContentHint": false, "consequentialHint": false })
    );

    let delete = page.page_tool("posts.delete").expect("posts.delete");
    assert_eq!(
        delete["annotations"],
        json!({ "readOnlyHint": false, "untrustedContentHint": false, "consequentialHint": true })
    );
    assert_eq!(delete["method"], "DELETE");
    assert_eq!(
        delete["path"],
        json!([
            { "text": "/posts/" },
            { "param": "post", "optional": false, "catchAll": false },
        ])
    );

    let search = page.page_tool("notes.search").expect("notes.search");
    assert_eq!(
        search["annotations"],
        json!({ "readOnlyHint": true, "untrustedContentHint": true, "consequentialHint": false })
    );
    assert_eq!(search["method"], "GET");
}

// ------------------------------------------------------- the boot check ----

/// [`page_routes`]' tools behind one more route offering `name`, checked.
fn boot_with_route_tool(name: &'static str) -> Result<(), FrameworkError> {
    let router: Router = fixture::routes()
        .get("/notes/search", search_notes)
        .get("/notes/lookup", lookup_notes)
        .get("/notes/filter", filter_notes)
        .get("/notes/{note}", show_note)
        .post("/audit", export_audit)
        .get("/offering", plain_page)
        .with_tools([name])
        .into();
    router.agents(AgentsConfig::new()).prepare_tools()
}

/// The same, with `name` shared by every page.
fn boot_with_shared_tool(name: &'static str) -> Result<(), FrameworkError> {
    let router: Router = fixture::routes()
        .get("/notes/search", search_notes)
        .get("/notes/lookup", lookup_notes)
        .get("/notes/filter", filter_notes)
        .get("/notes/{note}", show_note)
        .post("/audit", export_audit)
        .into();
    router
        .share_tools([name])
        .agents(AgentsConfig::new())
        .prepare_tools()
}

fn message(error: &FrameworkError) -> String {
    format!("{error} {error:?}")
}

#[tokio::test]
async fn agent_014_unknown_live_or_agent_only_page_tool_fails_boot() {
    let _container = fixture::setup();
    for name in [
        "no.such.tool",
        "counter.peek",
        "counter.add",
        "audit.export",
    ] {
        let error = boot_with_route_tool(name).expect_err(name);
        let text = message(&error);
        assert!(text.contains(name), "{name}: {text}");
        assert!(text.contains("/offering"), "{name} names the route: {text}");
        let error = boot_with_shared_tool(name).expect_err(name);
        let text = message(&error);
        assert!(text.contains(name), "{name}: {text}");
        assert!(text.contains("share_tools"), "{name}: {text}");
    }
    // A routed tool a browser can call passes.
    boot_with_route_tool("notes.search").expect("a public GET tool with no input");
    boot_with_route_tool("posts.create").expect("a user tool");
    boot_with_shared_tool("echo").expect("a public tool");
}

#[tokio::test]
async fn agent_014_a_get_page_tool_whose_input_has_an_object_field_fails_boot() {
    let _container = fixture::setup();
    let error = boot_with_route_tool("notes.filter").expect_err("an object field on GET");
    let text = message(&error);
    assert!(text.contains("notes.filter"), "{text}");
    assert!(text.contains("`filters`"), "the field is named: {text}");
    assert!(text.contains("query"), "{text}");
    // A GET tool whose input holds scalars reads it from the query.
    boot_with_route_tool("notes.lookup").expect("a GET tool with a scalar input");
}

// ---------------------------------------------------- the call's checks ----

#[tokio::test]
async fn agent_014_tool_call_header_must_name_the_routes_tool() {
    let _container = fixture::setup();
    let client = fixture::client();
    let mismatched = client
        .post("/echo")
        .json(&json!({ "text": "hi" }))
        .header("Suprnova-Tool-Call", "slow.wait")
        .send()
        .await;
    assert_eq!(mismatched.status(), 400, "{}", mismatched.body_text());
    assert!(
        mismatched.body_text().contains("slow.wait"),
        "{}",
        mismatched.body_text()
    );
    // The route's own tool passes.
    client
        .tool("echo", json!({ "text": "hi" }))
        .send()
        .await
        .assert_ok();
}

#[tokio::test]
async fn agent_014_tool_call_header_on_a_route_without_a_tool_is_refused() {
    let _container = fixture::setup();
    let client = fixture::client();
    // `post = "new"` on `/posts/{post}` reaches the literal `/posts/new`.
    let steered = client
        .get("/posts/new")
        .header("Suprnova-Tool-Call", "posts.show")
        .send()
        .await;
    assert_eq!(steered.status(), 400, "{}", steered.body_text());
    let plain = client
        .post("/plain")
        .header("Suprnova-Tool-Call", "echo")
        .send()
        .await;
    assert_eq!(plain.status(), 400, "{}", plain.body_text());
    // Without the header the routes answer as before.
    client.get("/posts/new").send().await.assert_ok();
    client.post("/plain").send().await.assert_ok();
}

#[tokio::test]
async fn agent_014_a_get_tool_call_reads_its_input_from_the_query() {
    let _container = fixture::setup();
    let client = page_client();
    let call = client
        .tool("notes.lookup", json!({ "q": "café & co" }))
        .send()
        .await;
    call.assert_ok();
    assert_eq!(call.json(), json!({ "q": "café & co" }));
}

#[tokio::test]
async fn agent_014_a_get_tool_on_a_path_parameter_reads_it_from_the_query() {
    let _container = fixture::setup();
    let client = page_client();
    // The page offers it: its input holds scalars only.
    let page = visit(client.clone(), "/inertia/notes").await;
    assert!(
        page.page_tools().contains(&"notes.show".to_string()),
        "{:?}",
        page.page_tools()
    );
    let call = client
        .tool("notes.show", json!({ "note": "n/7", "format": "short" }))
        .send()
        .await;
    call.assert_ok();
    assert_eq!(
        call.json(),
        json!({ "note": "n/7", "format": "short", "same": true })
    );
    // A null argument stays out of the query.
    let call = client
        .tool("notes.show", json!({ "note": "8", "format": null }))
        .send()
        .await;
    call.assert_ok();
    assert_eq!(
        call.json(),
        json!({ "note": "8", "format": null, "same": true })
    );
}

#[tokio::test]
async fn agent_014_a_get_tool_call_sends_its_arguments_in_the_query() {
    let _container = fixture::setup();
    let client = page_client();
    let call = client
        .tool("notes.search", json!({ "q": "café & co" }))
        .send()
        .await;
    call.assert_ok();
    assert_eq!(call.json(), json!({ "q": "café & co" }));
}

// ---------------------------------------------------------- Live pages ----

/// The Live page offering server tools.
const TOOLS_PAGE: &str = "/agents/tools-page";

/// [`live_fixture::live_router`]'s shape, serving [`TOOLS_PAGE`] with the
/// route's tools, and the document adding `request.peek`.
fn live_tools_router() -> Router {
    fixture::define_policies();
    let mount = LiveMount::<AgentCounter>::public_seed(TOOLS_PAGE, "counter", "main")
        .expect("declare the counter's mount");
    let side = LiveMount::<AgentQuota>::public_seed(TOOLS_PAGE, "quota", "side")
        .expect("declare the quota's mount");
    let handler_mounts = (mount.clone(), side.clone());
    let router: Router = fixture::routes()
        .get(TOOLS_PAGE, move |request: Request| {
            let (mount, side) = handler_mounts.clone();
            async move { render_tools_page(request, mount, side).await }
        })
        .with_tools(["echo", "users.profile", "posts.create"])
        .into();
    router
        .try_live_with(live_fixture::guard)
        .expect("install Live")
        .try_live_mount(&mount)
        .expect("register the counter's mount")
        .try_live_mount(&side)
        .expect("register the quota's mount")
}

async fn render_tools_page(
    request: Request,
    mount: LiveMount<AgentCounter>,
    side: LiveMount<AgentQuota>,
) -> Result<HttpResponse, HttpResponse> {
    let result: Result<HttpResponse, FrameworkError> = async {
        let mut document = LiveDocument::from_request(&request)
            .map_err(|error| FrameworkError::internal(format!("from_request {error}")))?;
        document
            .with_tools(["request.peek"])
            .await
            .map_err(|error| FrameworkError::internal(format!("with_tools {error}")))?;
        let start = CanonicalValue::Object(BTreeMap::from([
            ("$live".to_owned(), CanonicalValue::String("u64".to_owned())),
            ("value".to_owned(), CanonicalValue::String("1".to_owned())),
        ]));
        let island = document
            .mount(
                &mount,
                CanonicalValue::Object(BTreeMap::from([
                    ("start".to_owned(), start),
                    (
                        "label".to_owned(),
                        CanonicalValue::String("Page".to_owned()),
                    ),
                    (
                        "owner".to_owned(),
                        CanonicalValue::String("page".to_owned()),
                    ),
                ])),
                MountFlags::empty(),
            )
            .await
            .map_err(|error| FrameworkError::internal(format!("mount {error}")))?;
        let side = document
            .mount(
                &side,
                CanonicalValue::Object(BTreeMap::new()),
                MountFlags::empty(),
            )
            .await
            .map_err(|error| FrameworkError::internal(format!("mount side {error}")))?;
        let bootstrap = document
            .bootstrap(LiveBootstrapOptions::esm())
            .map_err(|error| FrameworkError::internal(format!("bootstrap {error}")))?;
        // Too late now: the element is written.
        let late = document.with_tools(["echo"]).await;
        if late.is_ok() {
            return Err(FrameworkError::internal(
                "with_tools after bootstrap was accepted",
            ));
        }
        document
            .render(
                ViewName::parse("live/tests/agent-page.html")
                    .map_err(|_| FrameworkError::internal("view identity"))?,
                &AgentPage {
                    bootstrap: bootstrap.html(),
                    island: island.html(),
                    side: side.html(),
                },
                DocumentResponseIntent::html(StatusCode::OK)
                    .map_err(|_| FrameworkError::internal("response intent"))?,
                AssetSet::empty(),
            )
            .map_err(FrameworkError::from)
    }
    .await;
    result.map_err(|error| HttpResponse::text(format!("page failed: {error}")).status(500))
}

fn live_tools_client() -> TestClient {
    live_fixture::register_components();
    live_fixture::define_live_policies();
    let router = live_tools_router();
    prepare_live_router_for_test(&router).expect("prepare the Live runtime");
    TestClient::new(router, live_fixture::browser_middleware())
}

#[tokio::test]
async fn agent_014_live_document_lists_only_tools_the_viewer_may_call() {
    let _container = fixture::setup();
    let client = live_tools_client();
    let guest = client.get(TOOLS_PAGE).send().await;
    assert_eq!(guest.status(), 200, "{}", guest.body_text());
    assert!(element_in_head(&guest.body_text()), "{}", guest.body_text());
    assert_eq!(guest.page_tools(), ["echo", "request.peek"]);
    let user = client
        .clone()
        .acting_as(&TestUser::arc("alice"))
        .get(TOOLS_PAGE)
        .send()
        .await;
    assert_eq!(user.status(), 200, "{}", user.body_text());
    assert_eq!(
        user.page_tools(),
        ["echo", "posts.create", "request.peek", "users.profile"]
    );
}

/// The base names of the tools the island keyed `key` lists in `page`.
fn island_tools(page: &str, key: &str) -> Vec<String> {
    for root in page.split("<div").skip(1) {
        let attributes = root.split('>').next().unwrap_or_default();
        let keyed = format!(" data-suprnova-live-document-key=\"{key}\"");
        if !attributes.contains(&keyed) {
            continue;
        }
        let Some(start) = attributes.find(" data-suprnova-live-tools=\"") else {
            return Vec::new();
        };
        let rest = &attributes[start + " data-suprnova-live-tools=\"".len()..];
        let value = rest[..rest.find('"').unwrap_or(rest.len())]
            .replace("&quot;", "\"")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&#39;", "'")
            .replace("&amp;", "&");
        let tools: Value = serde_json::from_str(&value).expect("the tools attribute is JSON");
        return tools["tools"]
            .as_array()
            .map(|tools| {
                tools
                    .iter()
                    .filter_map(|tool| tool["name"].as_str())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
    }
    panic!("the page has no island keyed {key}: {page}");
}

#[tokio::test]
async fn agent_014_a_live_island_lists_only_tools_the_viewer_can_call() {
    let _container = fixture::setup();
    let client = live_fixture::live_client();
    let guest = client.get(live_fixture::PAGE).send().await;
    assert_eq!(guest.status(), 200, "{}", guest.body_text());
    assert_eq!(
        island_tools(&guest.body_text(), "main"),
        ["counter.peek.main"],
        "a guest sees the public tool only"
    );
    let user = client
        .clone()
        .acting_as(&TestUser::arc("alice"))
        .get(live_fixture::PAGE)
        .send()
        .await;
    assert_eq!(user.status(), 200, "{}", user.body_text());
    let mut listed = island_tools(&user.body_text(), "main");
    listed.sort();
    assert_eq!(
        listed,
        [
            "counter.add.main",
            "counter.check.main",
            "counter.peek.main",
            "counter.rename.main"
        ],
        "a session user sees the user tools and the public one"
    );
    // The quota's one tool accepts the public, so every viewer sees it.
    assert_eq!(
        island_tools(&guest.body_text(), "side"),
        ["quota.read.side"]
    );
    assert_eq!(island_tools(&user.body_text(), "side"), ["quota.read.side"]);
}

// --------------------------------------------------------- RenderCache ----

/// How many times a cached page rendered.
static CACHED_RENDERS: AtomicUsize = AtomicUsize::new(0);

async fn cached_page(request: Request) -> Response {
    let render = CACHED_RENDERS.fetch_add(1, Ordering::SeqCst) + 1;
    let element = PageTools::element(&request).map_err(HttpResponse::from)?;
    Ok(HttpResponse::html(format!(
        "<!doctype html><html><head>{element}</head><body><p>render {render}</p></body></html>"
    )))
}

/// Signs `x-test-login` in for the request, through request state, before
/// RenderCache derives its key.
struct LoginHeader;

#[suprnova::async_trait]
impl suprnova::Middleware for LoginHeader {
    async fn handle(&self, request: Request, next: suprnova::Next) -> Response {
        if let Some(id) = request.header("x-test-login") {
            suprnova::Auth::set_user(TestUser::arc(id));
        }
        next(request).await
    }
}

struct CacheMigrator;

#[suprnova::async_trait]
impl sea_orm_migration::MigratorTrait for CacheMigrator {
    fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
        vec![Box::new(suprnova::render_cache::migration::Migration)]
    }
}

/// A client of the pages behind an installed RenderCache: `/mixed` offers
/// a public and a user tool and varies by principal, `/leaky` offers the
/// same and declares no variance, `/public` offers public tools only and
/// declares no variance. Process-wide state: run the caller alone.
async fn cached_client() -> (TestClient, impl Drop, tempfile::TempDir) {
    use sea_orm_migration::MigratorTrait as _;
    use suprnova::render_cache::config::RenderCacheConfig;
    use suprnova::render_cache::registry::GroupPolicy;
    use suprnova::render_cache::{
        FreshnessPolicy, RenderCache, RenderCachePolicy, RepresentationClass, VarianceDimension,
    };

    static CRYPT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    CRYPT.get_or_init(|| suprnova::Crypt::init(suprnova::EncryptionKey::generate()));
    suprnova::App::init();
    suprnova::middleware::clear_global_middleware_for_test();
    let container = fixture::setup();
    let directory = tempfile::tempdir().expect("a directory for the cache's database");
    let config = suprnova::database::DatabaseConfig::builder()
        .url(format!(
            "sqlite://{}",
            directory.path().join("cache.sqlite3").display()
        ))
        .max_connections(4)
        .min_connections(1)
        .logging(false)
        .build();
    let connection = suprnova::database::DbConnection::connect(&config)
        .await
        .expect("connect the cache's database");
    CacheMigrator::up(connection.inner(), None)
        .await
        .expect("the RenderCache tables");
    suprnova::testing::TestContainer::singleton(connection);

    let principal = RenderCachePolicy::builder(RepresentationClass::PrivateCached)
        .freshness(FreshnessPolicy::new(60_000, 0, 0).expect("freshness"))
        .vary(VarianceDimension::Principal)
        .build()
        .expect("a principal policy");
    let shared = || {
        RenderCachePolicy::builder(RepresentationClass::PublicShared)
            .freshness(FreshnessPolicy::new(60_000, 0, 0).expect("freshness"))
            .build()
            .expect("a shared policy")
    };
    let router: Router = fixture::routes()
        .get("/mixed", cached_page)
        .with_tools(["echo", "users.profile"])
        .get("/leaky", cached_page)
        .with_tools(["echo", "users.profile"])
        .get("/public", cached_page)
        .with_tools(["echo", "throttled.ping"])
        .into();
    let router = router
        .agents(AgentsConfig::new())
        .try_render_cache("/mixed", GroupPolicy::from(principal))
        .expect("the principal policy")
        .try_render_cache("/leaky", GroupPolicy::from(shared()))
        .expect("the shared policy")
        .try_render_cache("/public", GroupPolicy::from(shared()))
        .expect("the shared policy");
    suprnova::middleware::register_global_middleware(LoginHeader);
    let mut cache = RenderCacheConfig::from_env().expect("a valid render cache configuration");
    cache.enabled = true;
    cache.l1 = suprnova::render_cache::L1Config::Disabled;
    let router = RenderCache::install(router, cache)
        .await
        .expect("install RenderCache");
    let client = TestClient::new(router, MiddlewareRegistry::from_global());
    (client, container, directory)
}

async fn cached(client: &TestClient, path: &str, login: Option<&str>) -> TestResponse {
    let mut request = client.get(path);
    if let Some(login) = login {
        request = request.header("x-test-login", login);
    }
    let response = request.send().await;
    response.assert_ok();
    response
}

#[tokio::test]
async fn agent_014_a_cached_page_never_shows_one_viewers_tools_to_another() {
    if crate::own_process_async::delegate(
        module_path!(),
        "agent_014_a_cached_page_never_shows_one_viewers_tools_to_another",
    )
    .await
    {
        return;
    }
    let (client, _container, _directory) = cached_client().await;
    // `/leaky` declares no principal variance: a list that reads the viewer
    // is never stored under its one shared key.
    let guest = cached(&client, "/leaky", None).await;
    assert_eq!(guest.page_tools(), ["echo"]);
    let alice = cached(&client, "/leaky", Some("alice")).await;
    assert_eq!(alice.page_tools(), ["echo", "users.profile"]);
    let guest_again = cached(&client, "/leaky", None).await;
    assert_eq!(guest_again.page_tools(), ["echo"]);
    // `/mixed` varies by principal: each viewer gets their own list.
    let alice = cached(&client, "/mixed", Some("alice")).await;
    assert_eq!(alice.page_tools(), ["echo", "users.profile"]);
    let guest = cached(&client, "/mixed", None).await;
    assert_eq!(guest.page_tools(), ["echo"]);
    let alice_again = cached(&client, "/mixed", Some("alice")).await;
    assert_eq!(alice_again.page_tools(), ["echo", "users.profile"]);
}

#[tokio::test]
async fn agent_014_guests_share_one_cached_copy_of_a_page_with_mixed_tools() {
    if crate::own_process_async::delegate(
        module_path!(),
        "agent_014_guests_share_one_cached_copy_of_a_page_with_mixed_tools",
    )
    .await
    {
        return;
    }
    let (client, _container, _directory) = cached_client().await;
    let first = cached(&client, "/mixed", None).await;
    assert_eq!(first.page_tools(), ["echo"]);
    let renders = CACHED_RENDERS.load(Ordering::SeqCst);
    let second = cached(&client, "/mixed", None).await;
    assert_eq!(second.page_tools(), ["echo"]);
    assert_eq!(
        CACHED_RENDERS.load(Ordering::SeqCst),
        renders,
        "a guest's read resolved no identity, so guests share the anonymous entry"
    );
    assert_eq!(first.body_text(), second.body_text());
    // A signed-in viewer renders their own.
    let alice = cached(&client, "/mixed", Some("alice")).await;
    assert_eq!(alice.page_tools(), ["echo", "users.profile"]);
    assert_eq!(CACHED_RENDERS.load(Ordering::SeqCst), renders + 1);
}

#[tokio::test]
async fn agent_014_a_page_of_public_tools_reads_no_caller() {
    if crate::own_process_async::delegate(
        module_path!(),
        "agent_014_a_page_of_public_tools_reads_no_caller",
    )
    .await
    {
        return;
    }
    let (client, _container, _directory) = cached_client().await;
    let guest = cached(&client, "/public", None).await;
    assert_eq!(guest.page_tools(), ["echo", "throttled.ping"]);
    let renders = CACHED_RENDERS.load(Ordering::SeqCst);
    // `/public` declares no variance, so a page that read the viewer would
    // not be stored: one shared copy serves every viewer.
    let alice = cached(&client, "/public", Some("alice")).await;
    assert_eq!(alice.page_tools(), ["echo", "throttled.ping"]);
    let bob = cached(&client, "/public", Some("bob")).await;
    assert_eq!(bob.body_text(), guest.body_text());
    assert_eq!(
        CACHED_RENDERS.load(Ordering::SeqCst),
        renders,
        "one shared copy for every viewer"
    );
}
