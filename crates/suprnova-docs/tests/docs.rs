use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use serde_json::json;
use suprnova::testing::TestClient;
use suprnova::{AgentsConfig, MiddlewareRegistry, SecurityHeaders};
use suprnova_docs::{DocsConfig, DocsSite};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn config(root: &Path, allowed: Arc<AtomicBool>) -> DocsConfig {
    DocsConfig::new(root, "/guide", |_, page| Ok(format!(
        "<!doctype html><html lang=\"{}\"><head>{}</head><body data-app-layout>{}{}{}<main>{}</main>{}</body></html>",
        page.language, page.alternate_html, page.navigation_html, page.versions_html,
        page.languages_html, page.html, page.search_html,
    ))).unwrap().with_gate(move |_, _| {
        let admitted = allowed.load(Ordering::SeqCst);
        Box::pin(async move { Ok(admitted) })
    }).unwrap()
}

fn client(root: &Path, allowed: Arc<AtomicBool>) -> TestClient {
    suprnova::App::bind_if_absent::<dyn suprnova::CacheStore>(Arc::new(
        suprnova::InMemoryCache::new(),
    ));
    let router = DocsSite::mount(suprnova::Router::new(), config(root, allowed))
        .unwrap()
        .try_agents(AgentsConfig::new())
        .unwrap();
    TestClient::new(
        router,
        MiddlewareRegistry::new().append(SecurityHeaders::balanced()),
    )
}

fn guest() -> TestClient {
    client(&fixture("guide"), Arc::new(AtomicBool::new(false)))
}

#[tokio::test]
async fn docs_001_navigation_pages_render_in_app_layout_and_links_stay_in_version() {
    let client = guest();
    for lang in ["en", "fr"] {
        for path in ["index", "nested/setup"] {
            let response = client.get(format!("/guide/v2/{lang}/{path}")).send().await;
            assert_eq!(response.status(), 200, "{}", response.body_text());
            let body = response.body_text();
            assert!(body.contains("data-app-layout"));
            assert!(body.contains("aria-label=\"Version\""));
            assert!(body.contains("aria-label=\"Language\""));
            assert!(!body.contains("PRIVATE ORBITAL SECRET"));
            assert!(!body.contains("Launch codes"));
        }
    }
    let setup = client.get("/guide/v2/en/nested/setup").send().await;
    assert!(
        setup
            .body_text()
            .contains("href=\"/guide/v2/en/index?from=setup#top\"")
    );
    assert!(setup.body_text().contains("href=\"/guide/v2/en/index\""));
    assert!(
        setup
            .body_text()
            .contains("href=\"https://example.com/guide.md\"")
    );
    assert!(setup.body_text().contains("<table>"));
    assert!(setup.body_text().contains("<del>Draft</del>"));
    let home = client.get("/guide/v2/en/index").send().await;
    assert!(
        home.body_text()
            .contains("href=\"/guide/v2/en/nested/setup#orbital-alignment\"")
    );
    assert!(
        home.body_text()
            .contains("src=\"/guide/v2/_assets/orbit.svg\"")
    );
    assert!(!home.body_text().contains("alert("));
    assert_eq!(client.get("/guide").send().await.status(), 200);
    assert_eq!(client.get("/guide/en/index").send().await.status(), 200);
    let asset = client.get("/guide/v2/_assets/orbit.svg").send().await;
    assert_eq!(asset.status(), 200);
    assert!(
        asset
            .header("content-type")
            .unwrap()
            .starts_with("image/svg+xml")
    );
}

#[tokio::test]
async fn docs_001_private_page_requires_gate_and_single_version_has_no_switch() {
    let allowed = Arc::new(AtomicBool::new(false));
    let client = client(&fixture("guide"), allowed.clone());
    assert_eq!(client.get("/guide/v2/en/secret").send().await.status(), 404);
    allowed.store(true, Ordering::SeqCst);
    let page = client.get("/guide/v2/en/secret").send().await;
    assert_eq!(page.status(), 200);
    assert!(page.body_text().contains("PRIVATE ORBITAL SECRET"));
    let solo = self::client(&fixture("single"), Arc::new(AtomicBool::new(false)));
    let page = solo.get("/guide/v1/index").send().await;
    assert_eq!(page.status(), 200);
    assert!(!page.body_text().contains("aria-label=\"Version\""));
    assert!(!page.body_text().contains("aria-label=\"Language\""));
}

#[tokio::test]
async fn docs_002_twins_accept_and_discovery_preserve_the_source() {
    let client = guest();
    let source = std::fs::read_to_string(fixture("guide/v2/en/index.md")).unwrap();
    for path in ["/guide/v2/en/index.md", "/guide/en/index.md"] {
        let twin = client.get(path).send().await;
        assert_eq!(twin.status(), 200);
        assert_eq!(twin.body_text(), source);
        assert_eq!(
            twin.header("content-type"),
            Some("text/markdown; charset=utf-8")
        );
        assert_eq!(twin.header("x-robots-tag"), Some("noindex"));
        assert!(twin.header("cache-control").unwrap().contains("no-store"));
    }
    let negotiated = client
        .get("/guide/v2/en/index")
        .header("Accept", "text/html, text/markdown; charset=utf-8")
        .send()
        .await;
    assert_eq!(negotiated.body_text(), source);
    assert!(negotiated.header("vary").unwrap().contains("Accept"));
    let html = client
        .get("/guide/v2/en/index")
        .header("Accept", "text/markdown;q=0, text/html")
        .send()
        .await;
    assert!(html.body_text().contains("data-app-layout"));
    assert!(
        html.header("link")
            .unwrap()
            .contains("/guide/v2/en/index.md")
    );
    assert!(
        html.body_text()
            .contains("rel=\"alternate\" type=\"text/markdown\"")
    );
    assert_eq!(
        client.get("/guide/v2/en/secret.md").send().await.status(),
        404
    );
    assert_eq!(
        client
            .get("/guide/v2/en/secret")
            .header("Accept", "text/markdown")
            .send()
            .await
            .status(),
        404
    );
}

#[tokio::test]
async fn docs_002_indexes_filter_private_pages_and_full_matches_navigation_order() {
    let allowed = Arc::new(AtomicBool::new(false));
    let client = client(&fixture("guide"), allowed.clone());
    for version in ["v1", "v2"] {
        for language in ["en", "fr"] {
            let base = format!("/guide/{version}/{language}");
            let index = client.get(format!("{base}/llms.txt")).send().await;
            assert_eq!(index.status(), 200);
            assert!(!index.body_text().contains("secret"));
            let mut parts = Vec::new();
            for line in index
                .body_text()
                .lines()
                .filter(|line| line.starts_with("- ["))
            {
                let url = line.split("](").nth(1).unwrap().trim_end_matches(')');
                let response = client.get(url).send().await;
                assert_eq!(response.status(), 200);
                parts.push(response.body_text().to_owned());
            }
            assert!(!parts.is_empty());
            assert_eq!(
                client
                    .get(format!("{base}/llms-full.txt"))
                    .send()
                    .await
                    .body_text(),
                parts.concat()
            );
        }
    }
    assert_eq!(
        client.get("/guide/llms-full.txt").send().await.body_text(),
        client
            .get("/guide/v2/en/llms-full.txt")
            .send()
            .await
            .body_text()
    );
    assert_eq!(
        client.get("/guide/v1/llms.txt").send().await.body_text(),
        client.get("/guide/v1/en/llms.txt").send().await.body_text()
    );
    allowed.store(true, Ordering::SeqCst);
    assert!(
        client
            .get("/guide/llms.txt")
            .send()
            .await
            .body_text()
            .contains("secret.md")
    );
    assert!(
        client
            .get("/guide/llms-full.txt")
            .send()
            .await
            .body_text()
            .ends_with("PRIVATE ORBITAL SECRET\n")
    );
    allowed.store(false, Ordering::SeqCst);
    assert!(
        !client
            .get("/guide/llms-full.txt")
            .send()
            .await
            .body_text()
            .contains("PRIVATE ORBITAL SECRET")
    );
}

#[tokio::test]
async fn docs_003_boot_index_contains_headings_and_script_uses_response_nonce() {
    let client = guest();
    let response = client.get("/guide/v2/en/search.json").send().await;
    assert_eq!(response.status(), 200);
    let index = response.json();
    let setup = index
        .as_array()
        .unwrap()
        .iter()
        .find(|page| page["title"] == "Setup")
        .unwrap();
    assert!(setup["headings"].to_string().contains("Orbital alignment"));
    assert!(setup["text"].as_str().unwrap().contains("silver antenna"));
    assert!(!response.body_text().contains("PRIVATE ORBITAL SECRET"));
    let page = client.get("/guide/v2/en/index").send().await;
    let body = page.body_text();
    let nonce = body
        .split("<script nonce=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    assert!(
        page.header("content-security-policy")
            .unwrap()
            .contains(&format!("'nonce-{nonce}'"))
    );
    assert!(page.body_text().contains("/guide/v2/en/search.json"));
    assert!(page.body_text().contains("textContent = page.title"));
    let next = client.get("/guide/v2/en/index").send().await;
    assert!(!next.body_text().contains(&format!("nonce=\"{nonce}\"")));
}

#[tokio::test]
async fn docs_003_tools_are_on_existing_mcp_endpoint_and_return_gated_markdown() {
    let allowed = Arc::new(AtomicBool::new(false));
    let client = client(&fixture("guide"), allowed.clone());
    let mcp = client.mcp();
    let names = mcp.list().await.tool_names();
    for name in ["search_docs", "read_page", "list_pages"] {
        assert!(names.contains(&name.to_owned()), "{names:?}");
    }
    let result = mcp
        .call("search_docs", json!({"query":"orbital alignment"}))
        .await;
    assert!(!result.is_error(), "{result:?}");
    assert_eq!(result.error_code(), None, "{result:?}");
    assert!(result.text().contains("/guide/v2/en/nested/setup"));
    assert!(result.text().contains("## Orbital alignment"));
    assert!(!result.text().contains("<h2"));
    let page = mcp
        .call("read_page", json!({"path":"nested/setup", "language":"fr"}))
        .await;
    assert!(!page.is_error(), "{page:?}");
    assert!(page.text().contains("/guide/v2/fr/nested/setup"));
    assert!(page.text().contains("# Setup"));
    assert!(
        page.structured().unwrap()["markdown"]
            .as_str()
            .unwrap()
            .starts_with("# Setup\n")
    );
    assert_eq!(
        page.structured().unwrap()["pages"][0]["address"],
        "/guide/v2/fr/nested/setup"
    );
    assert!(
        mcp.call("read_page", json!({"path":"secret"}))
            .await
            .is_error()
    );
    let listing = mcp.call("list_pages", json!({})).await;
    assert!(listing.text().contains("index.md"), "{listing:?}");
    assert!(!listing.text().contains("secret"));
    assert!(
        !mcp.call("search_docs", json!({"query":"Classified"}))
            .await
            .text()
            .contains("PRIVATE ORBITAL SECRET")
    );
    allowed.store(true, Ordering::SeqCst);
    let private = mcp.call("read_page", json!({"path":"secret"})).await;
    assert!(!private.is_error(), "{private:?}");
    assert!(private.text().contains("PRIVATE ORBITAL SECRET"));
    assert!(
        mcp.call("list_pages", json!({}))
            .await
            .text()
            .contains("secret.md")
    );
    assert!(
        mcp.call("search_docs", json!({"query":"Classified"}))
            .await
            .text()
            .contains("PRIVATE ORBITAL SECRET")
    );
    assert!(
        client
            .get("/guide/search.json")
            .send()
            .await
            .body_text()
            .contains("PRIVATE ORBITAL SECRET")
    );
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &to.join(entry.file_name()));
        } else {
            std::fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
        }
    }
}

#[tokio::test]
async fn docs_001_set_and_version_privacy_cover_assets_indexes_and_tools() {
    for whole_set in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        copy_tree(&fixture("guide"), temp.path());
        let path = temp.path().join("docs.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        if whole_set {
            manifest["private"] = json!(true);
        } else {
            manifest["versions"][0]["private"] = json!(true);
        }
        std::fs::write(&path, manifest.to_string()).unwrap();
        let allowed = Arc::new(AtomicBool::new(false));
        let client = client(temp.path(), allowed.clone());
        for path in [
            "/guide/v2/en/index",
            "/guide/v2/en/index.md",
            "/guide/llms.txt",
            "/guide/llms-full.txt",
            "/guide/search.json",
            "/guide/v2/_assets/orbit.svg",
        ] {
            assert_eq!(client.get(path).send().await.status(), 404, "{path}");
        }
        for name in ["list_pages", "search_docs", "read_page"] {
            assert!(
                client
                    .mcp()
                    .call(name, json!({"path":"index","query":"Welcome"}))
                    .await
                    .is_error()
            );
        }
        allowed.store(true, Ordering::SeqCst);
        assert_eq!(
            client
                .get("/guide/v2/_assets/orbit.svg")
                .send()
                .await
                .status(),
            200
        );
        assert_eq!(client.get("/guide/v2/en/index").send().await.status(), 200);
    }
}

#[test]
fn docs_001_invalid_sources_fail_mount_without_exposing_files() {
    for navigation in [
        "[{\"title\":\"Escape\",\"path\":\"../outside\"}]",
        "[{\"title\":\"Missing\",\"path\":\"missing\"}]",
        "[{\"title\":\"Duplicate\",\"path\":\"index\"},{\"title\":\"Again\",\"path\":\"index\"}]",
        "[{\"title\":\"Reserved\",\"path\":\"search.json\"}]",
    ] {
        let temp = tempfile::tempdir().unwrap();
        copy_tree(&fixture("single"), temp.path());
        std::fs::write(temp.path().join("v1/navigation.json"), navigation).unwrap();
        assert!(
            DocsSite::mount(
                suprnova::Router::new(),
                config(temp.path(), Arc::new(AtomicBool::new(false)))
            )
            .is_err()
        );
    }
    assert!(DocsConfig::new(fixture("single"), "/../escape", |_, _| Ok(String::new())).is_err());
    assert!(DocsConfig::new(fixture("single"), "missing-slash", |_, _| Ok(String::new())).is_err());
}

#[cfg(unix)]
#[test]
fn docs_001_symlink_cannot_read_outside_the_source_tree() {
    let temp = tempfile::tempdir().unwrap();
    copy_tree(&fixture("single"), temp.path());
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("private.md"), "private").unwrap();
    std::fs::remove_file(temp.path().join("v1/index.md")).unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("private.md"),
        temp.path().join("v1/index.md"),
    )
    .unwrap();
    assert!(
        DocsSite::mount(
            suprnova::Router::new(),
            config(temp.path(), Arc::new(AtomicBool::new(false)))
        )
        .is_err()
    );
}

#[tokio::test]
async fn docs_003_index_is_built_at_boot_and_gate_errors_fail_closed() {
    let temp = tempfile::tempdir().unwrap();
    copy_tree(&fixture("single"), temp.path());
    let client = client(temp.path(), Arc::new(AtomicBool::new(false)));
    std::fs::write(temp.path().join("v1/index.md"), "changed after boot").unwrap();
    assert!(
        client
            .get("/guide/v1/index.md")
            .send()
            .await
            .body_text()
            .contains("# Solo")
    );
    assert!(
        client
            .get("/guide/search.json")
            .send()
            .await
            .body_text()
            .contains("One release")
    );
    let config = config(&fixture("guide"), Arc::new(AtomicBool::new(false)))
        .with_gate(|_, _| {
            Box::pin(async { Err(suprnova::FrameworkError::internal("gate offline")) })
        })
        .unwrap();
    let router = DocsSite::mount(suprnova::Router::new(), config)
        .unwrap()
        .try_agents(AgentsConfig::new())
        .unwrap();
    let client = TestClient::new(router, MiddlewareRegistry::new());
    assert_eq!(client.get("/guide/llms.txt").send().await.status(), 500);
    assert_eq!(client.get("/guide/search.json").send().await.status(), 500);
    assert!(client.mcp().call("list_pages", json!({})).await.is_error());
}

#[tokio::test]
async fn docs_001_root_mount_and_default_private_gate_are_safe() {
    let config = DocsConfig::new(fixture("single"), "/", |_, page| {
        Ok(format!(
            "<!doctype html><html><body>{}{}</body></html>",
            page.html, page.search_html,
        ))
    })
    .unwrap();
    let router = DocsSite::mount(suprnova::Router::new(), config).unwrap();
    let client = TestClient::new(router, MiddlewareRegistry::new());
    for path in ["/", "/v1", "/index", "/v1/index"] {
        assert_eq!(client.get(path).send().await.status(), 200, "{path}");
    }
    let page = client.get("/").send().await;
    let body = page.body_text();
    let nonce = body
        .split("<script nonce=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    assert!(
        page.header("content-security-policy")
            .unwrap()
            .contains(&format!("'nonce-{nonce}'"))
    );
    let head = client
        .send(suprnova::Method::HEAD, "/v1/index.md")
        .send()
        .await;
    assert_eq!(head.status(), 200);
    assert_eq!(
        head.header("content-type"),
        Some("text/markdown; charset=utf-8")
    );
    assert!(head.body_text().is_empty());
    assert_ne!(client.post("/v1/index.md").send().await.status(), 200);
    let config =
        DocsConfig::new(fixture("guide"), "/guide", |_, page| Ok(page.html.clone())).unwrap();
    let router = DocsSite::mount(suprnova::Router::new(), config).unwrap();
    let client = TestClient::new(router, MiddlewareRegistry::new());
    assert_eq!(
        client.get("/guide/v2/en/secret.md").send().await.status(),
        404
    );
    assert!(
        !client
            .get("/guide/llms.txt")
            .send()
            .await
            .body_text()
            .contains("secret.md")
    );
}

#[tokio::test]
async fn docs_001_language_switch_does_not_fail_an_admitted_page() {
    let temp = tempfile::tempdir().unwrap();
    copy_tree(&fixture("guide"), temp.path());
    let path = temp.path().join("docs.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    manifest["private"] = json!(true);
    std::fs::write(path, manifest.to_string()).unwrap();
    let config = config(temp.path(), Arc::new(AtomicBool::new(false)))
        .with_gate(|_, access| Box::pin(async move { Ok(access.language == "en") }))
        .unwrap();
    let router = DocsSite::mount(suprnova::Router::new(), config).unwrap();
    let client = TestClient::new(router, MiddlewareRegistry::new());
    let page = client.get("/guide/v2/en/index").send().await;
    assert_eq!(page.status(), 200);
    assert!(!page.body_text().contains("href=\"/guide/v2/fr/"));
    assert_eq!(client.get("/guide/v2/fr/index").send().await.status(), 404);
    assert_eq!(
        client.get("/guide/v2/fr/search.json").send().await.status(),
        404
    );
}
