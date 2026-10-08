//! The Inertia root document (RDOC-001 to RDOC-004 and RDOC-006).
//!
//! Without an application template the first visit is the document the
//! framework writes itself, pinned here byte for byte (RDOC-002).

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use suprnova::testing::TestContainer;
use suprnova::{Frontend, HttpResponse, InertiaConfig, InertiaRequestExt, InertiaResponse};

/// A request the tests build by hand: its path, query and headers.
struct MockReq {
    path: String,
    headers: HashMap<String, String>,
}

impl MockReq {
    fn new(path: &str) -> Self {
        Self {
            path: path.to_string(),
            headers: HashMap::new(),
        }
    }

    fn inertia(mut self) -> Self {
        self.headers.insert("X-Inertia".into(), "true".into());
        self
    }
}

impl InertiaRequestExt for MockReq {
    fn path(&self) -> &str {
        &self.path
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }
}

/// The status, every header in order, and the body of a response.
async fn parts(response: HttpResponse) -> (u16, Vec<(String, String)>, String) {
    let response = response.into_hyper();
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_string(),
                value.to_str().expect("an ASCII header").to_string(),
            )
        })
        .collect();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("the body")
        .to_bytes();
    (
        status,
        headers,
        String::from_utf8(body.to_vec()).expect("a UTF-8 body"),
    )
}

/// A one-page SSR worker answering every render with `head` and `body`.
async fn ssr_worker(head: &[&str], body: &str) -> SocketAddr {
    let payload = Bytes::from(
        serde_json::to_vec(&serde_json::json!({ "head": head, "body": body }))
            .expect("the worker's answer"),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a loopback port");
    let addr = listener.local_addr().expect("its address");
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let payload = payload.clone();
            tokio::spawn(async move {
                let service = service_fn(move |_req: hyper::Request<hyper::body::Incoming>| {
                    let payload = payload.clone();
                    async move {
                        Ok::<_, Infallible>(
                            hyper::Response::builder()
                                .status(200)
                                .header("content-type", "application/json")
                                .body(Full::new(payload))
                                .expect("a response"),
                        )
                    }
                });
                let _ = http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });
    addr
}

/// The page every pin renders: a prop with markup, an ampersand and a
/// character outside ASCII, so the escaping is part of what is pinned.
fn pinned_page() -> InertiaResponse {
    InertiaResponse::new("Home").with("message", "</script> & caf\u{e9}")
}

/// Renders the first visit to `/home` under the `en` locale.
async fn first_visit(response: InertiaResponse) -> (u16, Vec<(String, String)>, String) {
    let request = MockReq::new("/home");
    let response = suprnova::scope_locale(suprnova::Locale::parse("en").unwrap(), async {
        response.resolve(&request).await.expect("a first visit")
    })
    .await;
    parts(response).await
}

/// The headers every first visit carries today.
fn pinned_headers() -> Vec<(String, String)> {
    vec![
        ("content-type".into(), "text/html; charset=utf-8".into()),
        ("vary".into(), "X-Inertia".into()),
    ]
}

const PINNED_PAGE_JSON: &str = r#"{"component":"Home","props":{"errors":{},"message":"<\/script> & café"},"url":"\/home","version":"pinned"}"#;

/// RDOC-002: in development with the React preamble the document is
/// today's, byte for byte.
#[tokio::test]
async fn rdoc_002_the_development_document_with_the_react_preamble_is_unchanged() {
    let _container = TestContainer::fake();
    let config = InertiaConfig::new()
        .frontend(Frontend::React)
        .development(true)
        .vite_dev_server("http://localhost:5765")
        .version("pinned");
    let (status, headers, body) =
        first_visit(pinned_page().with_config(config).title("Pinned & <Title>")).await;

    assert_eq!(status, 200);
    assert_eq!(headers, pinned_headers());
    let expected = format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"UTF-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">\n\
         <meta name=\"csrf-token\" content=\"\">\n\
         <title>Pinned &amp; &lt;Title&gt;</title>\n\
         <script type=\"module\">\n\
         import RefreshRuntime from 'http://localhost:5765/@react-refresh'\n\
         RefreshRuntime.injectIntoGlobalHook(window)\n\
         window.$RefreshReg$ = () => {{}}\n\
         window.$RefreshSig$ = () => (type) => type\n\
         window.__vite_plugin_react_preamble_installed__ = true\n\
         </script>\n\
         <script type=\"module\" src=\"http://localhost:5765/@vite/client\"></script>\n\
         <script type=\"module\" src=\"http://localhost:5765/src/main.tsx\"></script>\n\
         </head>\n<body>\n\
         <script type=\"application/json\" data-page=\"app\">{PINNED_PAGE_JSON}</script>\n\
         <div id=\"app\"></div>\n</body>\n</html>"
    );
    assert_eq!(body, expected);
}

/// RDOC-002: in production with a Vite manifest the document is today's,
/// byte for byte.
#[tokio::test]
async fn rdoc_002_the_production_document_with_a_manifest_is_unchanged() {
    let _container = TestContainer::fake();
    let dir = tempfile::tempdir().expect("a directory");
    let manifest = dir.path().join("manifest.json");
    std::fs::write(
        &manifest,
        r#"{
            "src/main.ts": {
                "file": "main-Q9zSqcUL.js",
                "isEntry": true,
                "css": ["main-3R4lN-AT.css"],
                "imports": ["_runtime-DTQbz0Cz.js"]
            },
            "_runtime-DTQbz0Cz.js": { "file": "runtime-DTQbz0Cz.js" }
        }"#,
    )
    .expect("write the manifest");
    let config = InertiaConfig::new()
        .frontend(Frontend::Svelte)
        .production()
        .manifest_path(&manifest)
        .version("pinned");
    let (status, headers, body) = first_visit(pinned_page().with_config(config)).await;

    assert_eq!(status, 200);
    assert_eq!(headers, pinned_headers());
    let expected = format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"UTF-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">\n\
         <meta name=\"csrf-token\" content=\"\">\n\
         <title>Suprnova</title>\n\
         <link rel=\"stylesheet\" href=\"/assets/main-3R4lN-AT.css\">\n\
         <script type=\"module\" src=\"/assets/main-Q9zSqcUL.js\"></script>\n\
         <link rel=\"modulepreload\" href=\"/assets/runtime-DTQbz0Cz.js\">\n\
         </head>\n<body>\n\
         <script type=\"application/json\" data-page=\"app\">{PINNED_PAGE_JSON}</script>\n\
         <div id=\"app\"></div>\n</body>\n</html>"
    );
    assert_eq!(body, expected);
}

/// RDOC-002: in production without a manifest (the legacy fallback) the
/// document is today's, byte for byte.
#[tokio::test]
async fn rdoc_002_the_legacy_fallback_document_is_unchanged() {
    let _container = TestContainer::fake();
    let dir = tempfile::tempdir().expect("a directory");
    let config = InertiaConfig::new()
        .frontend(Frontend::Vue)
        .production()
        .manifest_path(dir.path().join("missing.json"))
        .version("pinned");
    let (status, headers, body) = first_visit(pinned_page().with_config(config)).await;

    assert_eq!(status, 200);
    assert_eq!(headers, pinned_headers());
    let expected = format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"UTF-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">\n\
         <meta name=\"csrf-token\" content=\"\">\n\
         <title>Suprnova</title>\n\
         <script type=\"module\" src=\"/assets/main.js\"></script>\n\
         <link rel=\"stylesheet\" href=\"/assets/main.css\">\n\
         </head>\n<body>\n\
         <script type=\"application/json\" data-page=\"app\">{PINNED_PAGE_JSON}</script>\n\
         <div id=\"app\"></div>\n</body>\n</html>"
    );
    assert_eq!(body, expected);
}

/// RDOC-002: under SSR the document is today's, byte for byte: the
/// worker's head after the CSRF tag and in place of the default title it
/// carries, and its body verbatim.
#[tokio::test]
async fn rdoc_002_the_ssr_document_is_unchanged() {
    let _container = TestContainer::fake();
    let ssr_body = "<script type=\"application/json\" data-page=\"app\">{\"component\":\"Home\"}</script><div data-server-rendered=\"true\" id=\"app\"><main>SSR</main></div>";
    let addr = ssr_worker(
        &["<title>SSR Title</title>", "<meta name=\"ssr\" content=\"yes\">"],
        ssr_body,
    )
    .await;
    let config = InertiaConfig::new()
        .frontend(Frontend::Svelte)
        .development(true)
        .vite_dev_server("http://localhost:5765")
        .ssr(format!("http://{addr}"))
        .version("pinned");
    let (status, headers, body) = first_visit(pinned_page().with_config(config)).await;

    assert_eq!(status, 200);
    assert_eq!(headers, pinned_headers());
    let expected = format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"UTF-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">\n\
         <meta name=\"csrf-token\" content=\"\">\n\
         <title>SSR Title</title>\n<meta name=\"ssr\" content=\"yes\">\
         <script type=\"module\" src=\"http://localhost:5765/@vite/client\"></script>\n\
         <script type=\"module\" src=\"http://localhost:5765/src/main.ts\"></script>\n\
         </head>\n<body>\n{ssr_body}\n</body>\n</html>"
    );
    assert_eq!(body, expected);
}

/// The XHR visit is the page JSON alone; a pin for the headers it has
/// always carried, so a template path cannot leak into it.
#[tokio::test]
async fn rdoc_002_the_inertia_visit_is_unchanged() {
    let _container = TestContainer::fake();
    let config = InertiaConfig::new().development(true).version("pinned");
    let request = MockReq::new("/home").inertia();
    let response = pinned_page()
        .with_config(config)
        .resolve(&request)
        .await
        .expect("an Inertia visit");
    let (status, headers, body) = parts(response).await;

    assert_eq!(status, 200);
    assert_eq!(
        headers,
        vec![
            ("content-type".to_string(), "application/json".to_string()),
            ("x-inertia".to_string(), "true".to_string()),
            ("vary".to_string(), "X-Inertia".to_string()),
        ]
    );
    assert_eq!(
        body,
        r#"{"component":"Home","props":{"errors":{},"message":"</script> & café"},"url":"/home","version":"pinned"}"#
    );
}

/// RDOC-001: the page data element and the mount element take their id
/// from `InertiaConfig::mount_id`, so a client mounting on another id finds
/// its element; the default stays `app`, and the id is escaped as an
/// attribute value.
#[tokio::test]
async fn rdoc_001_the_mount_id_names_the_page_data_and_mount_elements() {
    let _container = TestContainer::fake();
    assert_eq!(InertiaConfig::new().mount_id, "app");

    let config = InertiaConfig::new()
        .development(true)
        .version("pinned")
        .mount_id("root");
    let (_, _, body) = first_visit(pinned_page().with_config(config)).await;
    assert!(
        body.contains(&format!(
            "<script type=\"application/json\" data-page=\"root\">{PINNED_PAGE_JSON}</script>\n<div id=\"root\"></div>"
        )),
        "{body}"
    );
    assert!(!body.contains("\"app\""), "{body}");

    let config = InertiaConfig::new()
        .development(true)
        .version("pinned")
        .mount_id("a\"b<c");
    let (_, _, body) = first_visit(pinned_page().with_config(config)).await;
    assert!(
        body.contains("data-page=\"a&quot;b&lt;c\">") && body.contains("<div id=\"a&quot;b&lt;c\">"),
        "{body}"
    );
}
