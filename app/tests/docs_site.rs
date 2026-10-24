//! Exercise the mounted plugin over the framework's in-process HTTP adapter.

mod test_environment;

use std::any::Any;
use std::sync::Arc;

use serde_json::json;
use suprnova::testing::TestClient;
use suprnova::{Authenticatable, MiddlewareRegistry, Router, SecurityHeaders, ToolCaller};

#[derive(Clone)]
struct Crew;
impl Authenticatable for Crew {
    fn get_auth_identifier(&self) -> String {
        "crew-7".to_owned()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn into_arc_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

fn client() -> TestClient {
    test_environment::testing();
    suprnova::App::bind_if_absent::<dyn suprnova::CacheStore>(Arc::new(
        suprnova::InMemoryCache::new(),
    ));
    let router = app::docs::mount(Router::new())
        .expect("mount dogfood docs")
        .try_agents(suprnova::AgentsConfig::new())
        .expect("mount the application's MCP endpoint");
    TestClient::new(
        router,
        MiddlewareRegistry::new().append(SecurityHeaders::balanced()),
    )
}

#[tokio::test]
async fn docs_site_pages_twins_indexes_search_and_agent_tools_use_the_plugin() {
    let client = client();
    for path in [
        "/docs",
        "/docs/v2/index",
        "/docs/v2/launch",
        "/docs/v1/index",
    ] {
        let page = client.get(path).send().await;
        assert_eq!(page.status(), 200, "{path}: {}", page.body_text());
        assert!(page.body_text().contains("data-docs-layout"));
        assert!(page.body_text().contains("aria-label=\"Version\""));
    }
    let page = client.get("/docs/v2/index").send().await;
    assert!(page.body_text().contains("href=\"/docs/v2/launch\""));
    let twin = client.get("/docs/v2/launch.md").send().await;
    assert_eq!(twin.status(), 200);
    assert_eq!(
        twin.header("content-type"),
        Some("text/markdown; charset=utf-8")
    );
    assert!(twin.body_text().contains("## Orbital alignment"));
    assert_eq!(
        client
            .get("/docs/v2/launch")
            .header("Accept", "text/markdown")
            .send()
            .await
            .body_text(),
        twin.body_text()
    );
    assert_eq!(
        client
            .get("/docs/v2/_assets/orbit.svg")
            .send()
            .await
            .status(),
        200
    );
    assert_eq!(client.get("/docs/v2/crew").send().await.status(), 404);
    assert_eq!(client.get("/docs/v2/crew.md").send().await.status(), 404);
    for path in [
        "/docs/llms.txt",
        "/docs/v2/llms.txt",
        "/docs/llms-full.txt",
        "/docs/search.json",
    ] {
        let index = client.get(path).send().await;
        assert_eq!(index.status(), 200, "{}", index.body_text());
        assert!(!index.body_text().contains("Private crew briefing"));
        assert!(!index.body_text().contains("crew.md"));
    }
    let search = client.get("/docs/search.json").send().await;
    assert!(search.body_text().contains("Orbital alignment"));
    let mcp = client.mcp();
    let names = mcp.list().await.tool_names();
    for name in ["search_docs", "read_page", "list_pages"] {
        assert!(names.contains(&name.to_owned()), "{names:?}");
    }
    let search = mcp
        .call("search_docs", json!({"query":"Orbital alignment"}))
        .await;
    assert!(!search.is_error(), "{search:?}");
    assert!(search.text().contains("/docs/v2/launch"));
    let page = mcp.call("read_page", json!({"path":"launch"})).await;
    assert!(!page.is_error(), "{page:?}");
    assert!(page.text().contains("## Orbital alignment"));
    assert!(!page.text().contains("<h2"));
    assert!(
        mcp.call("read_page", json!({"path":"crew"}))
            .await
            .is_error()
    );
    assert!(
        !mcp.call("list_pages", json!({}))
            .await
            .text()
            .contains("crew.md")
    );
    assert!(
        !mcp.call("search_docs", json!({"query":"docking override"}))
            .await
            .text()
            .contains("docking override")
    );
}

#[tokio::test]
async fn docs_site_signed_in_reader_and_mcp_user_share_the_private_gate() {
    let client = client();
    let reader = client.clone().acting_as(&Crew);
    for path in ["/docs/v2/crew", "/docs/v2/crew.md", "/docs/crew.md"] {
        let response = reader.get(path).send().await;
        assert_eq!(response.status(), 200, "{path}: {}", response.body_text());
        assert!(response.body_text().contains("docking override"));
        assert!(
            response
                .header("cache-control")
                .unwrap()
                .contains("no-store")
        );
    }
    assert!(
        reader
            .get("/docs/llms.txt")
            .send()
            .await
            .body_text()
            .contains("crew.md")
    );
    assert!(
        reader
            .get("/docs/search.json")
            .send()
            .await
            .body_text()
            .contains("Private crew briefing")
    );
    let mcp = client.mcp().as_caller(ToolCaller::user(Arc::new(Crew)));
    let page = mcp.call("read_page", json!({"path":"crew"})).await;
    assert!(!page.is_error(), "{page:?}");
    assert_eq!(page.error_code(), None, "{page:?}");
    assert!(page.text().contains("docking override"));
    assert!(
        mcp.call("list_pages", json!({}))
            .await
            .text()
            .contains("crew.md")
    );
    assert!(
        mcp.call("search_docs", json!({"query":"Private crew briefing"}))
            .await
            .text()
            .contains("docking override")
    );
    assert_eq!(client.get("/docs/v2/crew.md").send().await.status(), 404);
}
