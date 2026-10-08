//! The Inertia root document (RDOC-001 to RDOC-004 and RDOC-006).
//!
//! Without an application template the first visit is the document the
//! framework writes itself, pinned here byte for byte (RDOC-002). With one,
//! the application's Askama template places the framework's parts
//! (RDOC-001); the templates live under `framework/tests/templates/inertia/`.

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use suprnova::testing::TestContainer;
use suprnova::{
    Frontend, HttpResponse, InertiaConfig, InertiaRequestExt, InertiaResponse, InertiaRootTemplate,
};

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
        &[
            "<title>SSR Title</title>",
            "<meta name=\"ssr\" content=\"yes\">",
        ],
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
        body.contains("data-page=\"a&quot;b&lt;c\">")
            && body.contains("<div id=\"a&quot;b&lt;c\">"),
        "{body}"
    );
}

/// Custom filters the test templates name.
mod filters {
    use suprnova::view::{FilterResult, FilterValues};

    /// Always fails, so a template that applies it fails to render after
    /// writing part of its document.
    #[suprnova::view_filter]
    pub fn refuse(_value: &str, _: &dyn FilterValues) -> FilterResult<String> {
        Err(std::fmt::Error.into())
    }
}

/// A root document with markup of its own around every part.
#[suprnova::inertia_root(path = "inertia/root.html")]
struct AppDocument;

/// A root document whose last expression fails.
#[suprnova::inertia_root(path = "inertia/failing.html")]
struct FailingDocument;

/// A development config that renders through `AppDocument`.
fn templated() -> InertiaConfig {
    InertiaConfig::new()
        .frontend(Frontend::Svelte)
        .development(true)
        .vite_dev_server("http://localhost:5765")
        .version("pinned")
        .root_template(InertiaRootTemplate::of::<AppDocument>())
}

/// RDOC-001: a root template places a favicon, meta tags, a `<noscript>`
/// and attributes on `<html>` and `<body>` of its own.
#[tokio::test]
async fn rdoc_001_a_root_template_places_markup_of_its_own() {
    let _container = TestContainer::fake();
    let (status, headers, body) = first_visit(pinned_page().with_config(templated())).await;

    assert_eq!(status, 200);
    assert_eq!(headers, pinned_headers());
    for own in [
        "<html lang=\"en\" class=\"h-full\" data-theme=\"dark\">",
        "<link rel=\"icon\" href=\"/favicon.ico\">",
        "<meta name=\"description\" content=\"Rendered through the root template\">",
        "<body class=\"antialiased\">",
        "<noscript>This page needs JavaScript.</noscript>",
    ] {
        assert!(body.contains(own), "missing {own} in:\n{body}");
    }
}

/// RDOC-001: the markup parts are placed as markup, never escaped as text:
/// the title element, the CSRF tag and the Vite tags in the head, and the
/// page data element (the same page JSON the framework's own document
/// carries) and the mount element in the body.
#[tokio::test]
async fn rdoc_001_the_parts_are_placed_as_markup() {
    let _container = TestContainer::fake();
    let (_, _, body) = first_visit(pinned_page().with_config(templated()).title("A & <B>")).await;

    let expected_head = "<title>A &amp; &lt;B&gt;</title>\n\
         <meta name=\"csrf-token\" content=\"\">\n\
         <script type=\"module\" src=\"http://localhost:5765/@vite/client\"></script>\n\
         <script type=\"module\" src=\"http://localhost:5765/src/main.ts\"></script>\n";
    assert!(body.contains(expected_head), "{body}");
    let expected_body = format!(
        "<noscript>This page needs JavaScript.</noscript>\n\
         <script type=\"application/json\" data-page=\"app\">{PINNED_PAGE_JSON}</script>\n\
         <div id=\"app\"></div>\n</body>"
    );
    assert!(body.contains(&expected_body), "{body}");
    for escaped in ["&lt;title", "&lt;meta", "&lt;script", "&#60;", "&lt;div"] {
        assert!(!body.contains(escaped), "{escaped} in:\n{body}");
    }
}

/// RDOC-001: `lang`, `csrf_token` and `nonce` are values a template places
/// itself, escaped like any other value; `nonce` is absent without a nonce
/// policy.
#[tokio::test]
async fn rdoc_001_lang_csrf_token_and_nonce_are_values_the_template_places() {
    let _container = TestContainer::fake();
    let session = std::sync::Arc::new(std::sync::Mutex::new(Some(
        suprnova::session::SessionData::new("id".into(), "tok\"en<1".into()),
    )));
    let request = MockReq::new("/home");
    let response = suprnova::session::session_scope_for_test(
        session,
        suprnova::scope_locale(suprnova::Locale::parse("pt-BR").unwrap(), async {
            pinned_page()
                .with_config(templated())
                .resolve(&request)
                .await
                .expect("a first visit")
        }),
    )
    .await;
    let (_, _, body) = parts(response).await;

    assert!(body.contains("<html lang=\"pt-BR\""), "{body}");
    assert!(
        body.contains("<meta name=\"csrf-token\" content=\"tok&quot;en&lt;1\">"),
        "{body}"
    );
    assert!(
        body.contains("<meta name=\"csrf-copy\" content=\"tok&#34;en&#60;1\">"),
        "{body}"
    );
    assert!(!body.contains("name=\"nonce\""), "{body}");
}

/// RDOC-001: the mount id reaches a root template's page data and mount
/// elements.
#[tokio::test]
async fn rdoc_001_the_mount_id_reaches_a_root_template() {
    let _container = TestContainer::fake();
    let (_, _, body) = first_visit(pinned_page().with_config(templated().mount_id("root"))).await;

    assert!(
        body.contains(&format!(
            "<script type=\"application/json\" data-page=\"root\">{PINNED_PAGE_JSON}</script>\n<div id=\"root\"></div>"
        )),
        "{body}"
    );
    assert!(!body.contains("\"app\""), "{body}");
}

/// RDOC-001: the `title` part is empty when the SSR head carries a
/// `<title>`, whatever the response or the config set, and present when
/// the SSR head carries none.
#[tokio::test]
async fn rdoc_001_the_title_part_is_empty_when_the_ssr_head_carries_a_title() {
    let _container = TestContainer::fake();
    let ssr_body = "<script type=\"application/json\" data-page=\"app\">{}</script><div data-server-rendered=\"true\" id=\"app\"></div>";
    let titled = ssr_worker(&["<title>SSR Title</title>"], ssr_body).await;
    let config = templated()
        .default_title("Default")
        .ssr(format!("http://{titled}"));
    let (_, _, body) = first_visit(pinned_page().with_config(config).title("Response")).await;
    assert_eq!(body.matches("<title").count(), 1, "{body}");
    assert!(body.contains("<title>SSR Title</title>"), "{body}");

    let untitled = ssr_worker(&["<meta name=\"ssr\" content=\"yes\">"], ssr_body).await;
    let config = templated()
        .default_title("Default")
        .ssr(format!("http://{untitled}"));
    let (_, _, body) = first_visit(pinned_page().with_config(config)).await;
    assert!(body.contains("<title>Default</title>"), "{body}");
}

/// RDOC-001: `ssr` is true only for a response the SSR server rendered,
/// whose head and body the parts then carry; without SSR, and when the
/// worker cannot be reached, the template places its fallback.
#[tokio::test]
async fn rdoc_001_ssr_is_true_only_when_the_ssr_server_rendered_the_response() {
    let _container = TestContainer::fake();
    let (_, _, body) = first_visit(pinned_page().with_config(templated())).await;
    assert!(body.contains("content=\"csr\""), "no SSR:\n{body}");

    let unreachable = templated().ssr("http://127.0.0.1:1");
    let (_, _, body) = first_visit(pinned_page().with_config(unreachable)).await;
    assert!(body.contains("content=\"csr\""), "worker down:\n{body}");
    assert!(!body.contains("content=\"ssr\""), "worker down:\n{body}");

    let ssr_body = "<script type=\"application/json\" data-page=\"app\">{}</script><div data-server-rendered=\"true\" id=\"app\"><main>SSR</main></div>";
    let addr = ssr_worker(&["<meta name=\"ssr\" content=\"yes\">"], ssr_body).await;
    let (_, _, body) =
        first_visit(pinned_page().with_config(templated().ssr(format!("http://{addr}")))).await;
    assert!(
        body.contains("<meta name=\"rendered-by\" content=\"ssr\">"),
        "{body}"
    );
    assert!(
        body.contains("<meta name=\"ssr\" content=\"yes\">"),
        "{body}"
    );
    assert!(body.contains(ssr_body), "{body}");
    assert_eq!(body.matches("data-page").count(), 1, "{body}");
}

/// RDOC-001: a root template that fails to render makes the response an
/// error naming the template; it neither panics nor sends the part of the
/// document it wrote before failing.
#[tokio::test]
async fn rdoc_001_a_template_that_fails_to_render_is_an_error() {
    let _container = TestContainer::fake();
    let config = InertiaConfig::new()
        .development(true)
        .version("pinned")
        .root_template(InertiaRootTemplate::of::<FailingDocument>());
    let request = MockReq::new("/home");
    let error = match pinned_page().with_config(config).resolve(&request).await {
        Ok(response) => panic!("a failed render answered {}", response.status_code()),
        Err(error) => error,
    };
    assert!(error.to_string().contains("FailingDocument"), "{error}");

    let (status, _, body) = parts(HttpResponse::from(error)).await;
    assert_eq!(status, 500);
    for partial in ["<!DOCTYPE", "<html", "csrf-token", "data-page"] {
        assert!(!body.contains(partial), "{partial} in:\n{body}");
    }
}

/// RDOC-001: a root template that names a value the framework does not
/// supply does not compile.
#[test]
fn rdoc_001_a_template_naming_an_unsupplied_value_does_not_compile() {
    // trybuild compiles each case as a crate of its own under the target
    // directory, and Askama reads that crate's `templates/`.
    let target = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .parent()
        .expect("the target directory");
    let templates = target.join("tests/trybuild/suprnova/templates/inertia");
    std::fs::create_dir_all(&templates).expect("the trybuild templates directory");
    std::fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/inertia/compile_fail/unsupplied_value.html"),
        templates.join("unsupplied_value.html"),
    )
    .expect("copy the template");
    trybuild::TestCases::new().compile_fail("tests/inertia/compile_fail/*.rs");
}
