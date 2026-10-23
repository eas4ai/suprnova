use suprnova::page_budget::PageBudget;
use suprnova::testing::TestClient;
use suprnova::{HttpResponse, MiddlewareRegistry, Request, Router};

#[tokio::test]
async fn budget_001_fixture_passes_and_command_agrees() {
    let client = TestClient::new(
        Router::new().get("/", |_req: Request| async {
            Ok(HttpResponse::html("<p>Hello</p>"))
        }),
        MiddlewareRegistry::new(),
    );
    let audit = client
        .audit_page("http://localhost/", &PageBudget::default())
        .await
        .unwrap();
    audit.ensure_passes().unwrap();
    assert_eq!(audit.requests, 1);
}

#[test]
fn budget_001_shipped_har_reports_the_baseline() {
    let audit = PageBudget::default()
        .audit_har(include_str!("fixtures/suprnova.har.json"))
        .unwrap();
    assert_eq!(audit.requests, 11);
    for rule in [
        "total-transfer",
        "asset-size",
        "raster-format",
        "font-budget",
        "fingerprint-cache",
        "asset-request-cookie",
        "first-redirect",
    ] {
        assert!(
            audit.findings.iter().any(|f| f.rule == rule),
            "missing {rule}: {audit}"
        );
    }
}

fn har(entries: Vec<serde_json::Value>) -> String {
    serde_json::json!({"log": {"entries": entries}}).to_string()
}

fn entry(
    path: &str,
    status: u16,
    mime: &str,
    body: &str,
    size: u64,
    headers: &[(&str, &str)],
) -> serde_json::Value {
    serde_json::json!({
        "request": {"url": format!("http://localhost{path}"), "headers": []},
        "response": {"status": status, "bodySize": size,
            "headers": headers.iter().map(|(name,value)| serde_json::json!({"name": name,"value": value})).collect::<Vec<_>>(),
            "content": {"mimeType": mime,"size": size,"text": body}}
    })
}

#[test]
fn budget_001_each_failure_names_asset_and_values() {
    let mut asset = entry(
        "/large-ABcd1234.png",
        403,
        "image/png",
        "invalid png",
        600_000,
        &[
            ("Cache-Control", "public, max-age=86400"),
            ("Set-Cookie", "session=one"),
        ],
    );
    asset["request"]["headers"] = serde_json::json!([{"name":"Cookie","value":"session=one"}]);
    let audit = PageBudget::default()
        .audit_har(&har(vec![
            entry("/", 302, "text/html", "", 1, &[]),
            asset,
            entry("/font.woff2", 200, "font/woff2", "font", 500_000, &[]),
        ]))
        .unwrap();
    for rule in [
        "total-transfer",
        "asset-size",
        "raster-format",
        "png-compression",
        "font-budget",
        "fingerprint-cache",
        "asset-set-cookie",
        "asset-request-cookie",
        "cookie-independent",
        "first-redirect",
        "asset-status",
    ] {
        let finding = audit
            .findings
            .iter()
            .find(|f| f.rule == rule)
            .unwrap_or_else(|| panic!("missing {rule}: {audit}"));
        assert!(!finding.asset.is_empty());
        assert!(!finding.measured.is_empty());
        assert!(!finding.allowed.is_empty());
    }
    let message = audit.ensure_passes().unwrap_err().to_string();
    for finding in &audit.findings {
        assert!(message.contains(&finding.asset));
        assert!(message.contains(&finding.measured));
        assert!(message.contains(&finding.allowed));
    }
}

#[tokio::test]
async fn budget_001_command_fails_and_prints_every_baseline_finding() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/page_budget/fixtures/suprnova.har.json");
    let run = suprnova::console::test(["budget:audit", path.to_str().unwrap()])
        .run()
        .await;
    run.assert_exit_code(1);
    let audit = PageBudget::default()
        .audit_har(include_str!("fixtures/suprnova.har.json"))
        .unwrap();
    for f in &audit.findings {
        assert!(run.output().contains(&f.rule), "{}", run.output());
        assert!(run.output().contains(&f.asset));
    }
    for host in &audit.third_party_hosts {
        assert!(run.output().contains(host));
    }
}

const GRAPH_HTML: &str = r#"<!doctype html><base href="/site/">
<link rel="stylesheet" href="main-ABcd1234.css">
<script src="main-ABcd1234.js"></script>
<script>const ignored = '<img src="/not-a-real-image.png">';</script>
<link rel="icon" href="icon.svg"><link rel="apple-touch-icon" href="touch.png">
<link rel="manifest" href="app.webmanifest"><link rel="preload" href="preload.woff2" as="font">
<picture><source srcset="hero.webp 1x, hero@2x.webp 2x"><img src="hero.png" srcset="hero.png 1x"></picture>
<source src="clip.mp4"><img src="https://outside.example/image.png"><img src="data:image/png;base64,AAAA">
"#;

fn graph_body(path: &str) -> (&'static str, &'static str) {
    match path {
        "/" => ("text/html", GRAPH_HTML),
        "/site/main-ABcd1234.css" => (
            "text/css",
            "@import 'extra.css'; @font-face {font-family:Demo;src:url('font.woff2')} /* url('/not-real') */",
        ),
        "/site/extra.css" => (
            "text/css",
            "@font-face {src:url(font.woff2)} body{background:url(bg.svg)}",
        ),
        "/site/app.webmanifest" => (
            "application/manifest+json",
            r#"{"icons":[{"src":"manifest.png"}]}"#,
        ),
        "/site/main-ABcd1234.js" => ("text/javascript", "console.log('ok')"),
        "/site/preload.woff2" | "/site/font.woff2" => ("font/woff2", "font"),
        _ => ("image/svg+xml", "asset"),
    }
}

#[tokio::test]
async fn budget_001_graph_helper_and_command_share_all_measurements() {
    use std::sync::{Arc, Mutex};
    let seen = Arc::new(Mutex::new(Vec::new()));
    let requests = seen.clone();
    let router = suprnova::fallback!(move |req: Request| {
        let seen = requests.clone();
        async move {
            assert!(
                req.header("Cookie").is_none(),
                "assets must use anonymous requests"
            );
            seen.lock().unwrap().push(req.path().to_owned());
            let (mime, body) = graph_body(req.path());
            Ok(
                HttpResponse::bytes(bytes::Bytes::from_static(body.as_bytes()), mime)
                    .header("Cache-Control", "public, max-age=31536000, immutable"),
            )
        }
    })
    .register(Router::new());
    let client = TestClient::new(router, MiddlewareRegistry::new())
        .with_header("Cookie", "should-not-leak=1");
    let budget = PageBudget::default();
    let audit = client.audit_page("/", &budget).await.unwrap();
    audit.ensure_passes().unwrap();
    assert_eq!(audit.third_party_hosts, ["outside.example"]);
    let paths = seen.lock().unwrap().clone();
    for expected in [
        "/site/main-ABcd1234.css",
        "/site/main-ABcd1234.js",
        "/site/icon.svg",
        "/site/touch.png",
        "/site/app.webmanifest",
        "/site/preload.woff2",
        "/site/hero.webp",
        "/site/hero@2x.webp",
        "/site/hero.png",
        "/site/clip.mp4",
        "/site/font.woff2",
        "/site/extra.css",
        "/site/bg.svg",
        "/site/manifest.png",
    ] {
        assert!(
            paths.iter().any(|p| p == expected),
            "missing {expected}: {paths:?}"
        );
    }
    assert_eq!(
        paths.len(),
        15,
        "deduplicate and fetch no unreferenced assets: {paths:?}"
    );
    let entries = paths
        .iter()
        .map(|path| {
            let (mime, body) = graph_body(path);
            entry(
                path,
                200,
                mime,
                body,
                body.len() as u64,
                &[("Cache-Control", "public, max-age=31536000, immutable")],
            )
        })
        .collect();
    let capture = har(entries);
    assert_eq!(audit, budget.audit_har(&capture).unwrap());
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), capture).unwrap();
    let run = suprnova::console::test(["budget:audit", file.path().to_str().unwrap()])
        .run()
        .await;
    run.assert_successful();
    assert!(run.output().contains(&audit.to_string()));
}

#[test]
fn budget_001_limits_and_fingerprints_are_configurable() {
    let capture = har(vec![
        entry("/", 200, "text/html", "", 1, &[]),
        entry("/custom_deadbeef.js", 200, "text/javascript", "x", 20, &[]),
    ]);
    let defaults = PageBudget::default().audit_har(&capture).unwrap();
    defaults.ensure_passes().unwrap();
    let strict = PageBudget {
        total_transfer_bytes: 10,
        asset_bytes: 10,
        fingerprint_pattern: "_[a-f0-9]{8}$".into(),
        ..PageBudget::default()
    };
    let audit = strict.audit_har(&capture).unwrap();
    for rule in ["total-transfer", "asset-size", "fingerprint-cache"] {
        assert!(audit.findings.iter().any(|f| f.rule == rule));
    }
    let relaxed = PageBudget {
        total_transfer_bytes: u64::MAX,
        asset_bytes: u64::MAX,
        raster_threshold_bytes: u64::MAX,
        font_bytes: u64::MAX,
        require_immutable: false,
        forbid_asset_set_cookie: false,
        forbid_asset_request_cookie: false,
        require_cookie_independent: false,
        forbid_first_redirect: false,
        ..PageBudget::default()
    };
    let audit = relaxed
        .audit_har(&har(vec![
            entry("/", 302, "text/html", "", 1, &[]),
            entry(
                "/main-ABcd1234.js",
                302,
                "text/javascript",
                "",
                1,
                &[("Set-Cookie", "x=1")],
            ),
        ]))
        .unwrap();
    assert_eq!(
        audit
            .findings
            .iter()
            .map(|f| f.rule.as_str())
            .collect::<Vec<_>>(),
        ["asset-status"]
    );
}

#[tokio::test]
async fn budget_001_failures_do_not_hide_later_assets() {
    let router = Router::new()
        .get("/", |_req: Request| async {
            Ok(HttpResponse::html(
                "<script src='/main-ABcd1234.js'></script><img src='/blocked.png'>",
            ))
        })
        .get("/main-ABcd1234.js", |_req: Request| async {
            Ok(HttpResponse::text("script").header("Set-Cookie", "session=secret"))
        })
        .get("/blocked.png", |_req: Request| async {
            Ok(HttpResponse::text("sign in").status(403))
        });
    let audit = TestClient::new(router, MiddlewareRegistry::new())
        .audit_page("/", &PageBudget::default())
        .await
        .unwrap();
    for rule in [
        "asset-set-cookie",
        "fingerprint-cache",
        "cookie-independent",
        "asset-status",
    ] {
        assert!(audit.findings.iter().any(|f| f.rule == rule), "{audit}");
    }
    assert!(!audit.to_string().contains("secret"));
}

#[test]
fn budget_001_har_sizes_missing_bodies_and_errors_are_honest() {
    let mut asset = entry("/image.png", 200, "image/png", "", 200_000, &[]);
    asset["response"]["bodySize"] = serde_json::json!(10);
    asset["response"]["content"]
        .as_object_mut()
        .unwrap()
        .remove("text");
    let audit = PageBudget::default()
        .audit_har(&har(vec![entry("/", 200, "text/html", "", 1, &[]), asset]))
        .unwrap();
    assert_eq!(audit.total_transfer_bytes, 11);
    assert!(
        audit
            .findings
            .iter()
            .any(|f| f.rule == "png-compression" && f.measured == "PNG body unavailable")
    );
    assert!(audit.findings.iter().any(|f| f.rule == "raster-format"));
    assert!(PageBudget::default().audit_har("{}").is_err());
    assert!(PageBudget::default().audit_har(&har(vec![])).is_err());
    let invalid = PageBudget {
        fingerprint_pattern: "[".into(),
        ..PageBudget::default()
    };
    assert!(
        invalid
            .audit_har(&har(vec![entry("/", 200, "text/html", "", 1, &[])]))
            .is_err()
    );
}

#[tokio::test]
async fn budget_001_http_runner_records_redirects_and_never_fetches_external_hosts() {
    use suprnova::http_client::{FakeResponse, Http, assert_not_sent, assert_sent};
    Http::fake(|| async {
        Http::prevent_stray_requests(true);
        Http::fake_url("http://audit.test/", FakeResponse::new(302).header("Location","https://audit.test/page"));
        Http::fake_url("https://audit.test/page", FakeResponse::new(200).body("<script src='/main-ABcd1234.js'></script><img src='https://external.test/image.png'>").header("Content-Type","text/html"));
        Http::fake_url("https://audit.test/main-ABcd1234.js", FakeResponse::text(200,"ok").header("Content-Type","text/javascript").header("Cache-Control","public, max-age=31536000, immutable"));
        let audit = PageBudget::default().audit_url("http://audit.test/").await.unwrap();
        assert_eq!(audit.requests,3);
        assert_eq!(audit.third_party_hosts,["external.test"]);
        assert_eq!(audit.findings.len(),1,"{audit}");
        assert_eq!(audit.findings[0].rule,"first-redirect");
        assert_not_sent(|r| r.url.contains("external.test"));
        assert_not_sent(|r| r.header("Cookie").is_some());
        assert_sent(|r| r.url == "https://audit.test/main-ABcd1234.js");
    }).await;
}

#[tokio::test]
async fn budget_001_url_mode_counts_compressed_transfer_and_reads_decoded_bodies() {
    use std::io::Write;
    use suprnova::http_client::{FakeResponse, Http, assert_sent};
    let html = "<script src='/main-ABcd1234.js'></script>";
    let script = "export const answer = 42;\n".repeat(400);
    let gzip = |text: &str| {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(text.as_bytes()).unwrap();
        encoder.finish().unwrap()
    };
    let gzipped = gzip(&script);
    let page = gzip(html);
    assert!(gzipped.len() * 4 < script.len(), "the fixture compresses");
    Http::fake(|| async {
        Http::prevent_stray_requests(true);
        Http::fake_url(
            "https://audit.test/",
            FakeResponse::new(200)
                .body(page.clone())
                .header("Content-Type", "text/html")
                .header("Content-Encoding", "gzip"),
        );
        Http::fake_url(
            "https://audit.test/main-ABcd1234.js",
            FakeResponse::new(200)
                .body(gzipped.clone())
                .header("Content-Type", "text/javascript")
                .header("Content-Encoding", "gzip")
                .header("Cache-Control", "public, max-age=31536000, immutable"),
        );
        // The decoded script is larger than the per-asset limit; what crosses
        // the wire is not, and the size rules judge the wire.
        let budget = PageBudget {
            asset_bytes: (gzipped.len() + 64) as u64,
            ..PageBudget::default()
        };
        assert!(script.len() as u64 > budget.asset_bytes);
        let audit = budget.audit_url("https://audit.test/").await.unwrap();
        // The script is found only by reading the decoded document.
        assert_eq!(audit.requests, 2, "{audit}");
        assert_eq!(
            audit.total_transfer_bytes,
            (page.len() + gzipped.len()) as u64,
            "{audit}"
        );
        audit.ensure_passes().unwrap();
        assert_sent(|r| {
            r.url == "https://audit.test/main-ABcd1234.js"
                && r.header("Accept-Encoding")
                    .is_some_and(|v| v.contains("gzip"))
        });
    })
    .await;
}

#[tokio::test]
async fn budget_001_url_mode_refuses_an_encoding_it_did_not_ask_for() {
    use suprnova::http_client::{FakeResponse, Http};
    Http::fake(|| async {
        Http::prevent_stray_requests(true);
        Http::fake_url(
            "https://audit.test/",
            FakeResponse::new(200)
                .body("not brotli")
                .header("Content-Type", "text/html")
                .header("Content-Encoding", "br"),
        );
        let error = PageBudget::default()
            .audit_url("https://audit.test/")
            .await
            .unwrap_err();
        assert!(error.to_string().contains("br"), "{error}");
    })
    .await;
}

#[tokio::test]
async fn budget_001_collection_bounds_and_redirect_cycles_return_errors() {
    let router = Router::new()
        .get("/", |_req: Request| async {
            Ok(HttpResponse::html(
                "<img src='/one.svg'><img src='/two.svg'>",
            ))
        })
        .get("/one.svg", |_req: Request| async {
            Ok(HttpResponse::text("asset"))
        })
        .get("/two.svg", |_req: Request| async {
            Ok(HttpResponse::text("asset"))
        })
        .get("/cycle", |_req: Request| async {
            Ok(HttpResponse::text("")
                .status(302)
                .header("Location", "/cycle"))
        })
        .get("/slow", |_req: Request| async {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            Ok(HttpResponse::text("late"))
        });
    let client = TestClient::new(router, MiddlewareRegistry::new());
    let limited = PageBudget {
        max_resources: 2,
        ..PageBudget::default()
    };
    assert!(
        client
            .audit_page("/", &limited)
            .await
            .unwrap_err()
            .to_string()
            .contains("resources")
    );
    let limited = PageBudget {
        max_response_bytes: 4,
        ..PageBudget::default()
    };
    assert!(client.audit_page("/", &limited).await.is_err());
    assert!(
        client
            .audit_page("/cycle", &PageBudget::default())
            .await
            .unwrap_err()
            .to_string()
            .contains("cycle")
    );
    let limited = PageBudget {
        request_timeout: std::time::Duration::from_millis(5),
        ..PageBudget::default()
    };
    assert!(
        client
            .audit_page("/slow", &limited)
            .await
            .unwrap_err()
            .to_string()
            .contains("timed out")
    );
}

#[test]
fn budget_001_large_modern_images_require_a_fallback_and_host_limits_apply() {
    let document = "<picture><source srcset='/hero.webp'><img src='/hero.jpg'></picture>";
    let capture = |html: &str| {
        har(vec![
            entry("/", 200, "text/html", html, html.len() as u64, &[]),
            entry("/hero.webp", 200, "image/webp", "webp", 120_000, &[]),
            entry("/hero.jpg", 200, "image/jpeg", "jpeg", 120_000, &[]),
        ])
    };
    PageBudget::default()
        .audit_har(&capture(document))
        .unwrap()
        .ensure_passes()
        .unwrap();
    let audit = PageBudget::default()
        .audit_har(&capture("<img src='/hero.webp'><img src='/hero.jpg'>"))
        .unwrap();
    assert_eq!(
        audit
            .findings
            .iter()
            .filter(|f| f.rule == "raster-format")
            .count(),
        2
    );
    let mut third = entry("/", 200, "text/html", "", 1, &[]);
    third["request"]["url"] = serde_json::json!("https://outside.test/asset.js");
    third["response"]["content"]["mimeType"] = serde_json::json!("text/javascript");
    let budget = PageBudget {
        allowed_third_party_hosts: Some(Default::default()),
        ..PageBudget::default()
    };
    let audit = budget
        .audit_har(&har(vec![entry("/", 200, "text/html", "", 1, &[]), third]))
        .unwrap();
    assert!(audit.findings.iter().any(|f| f.rule == "third-party-host"));
}

#[cfg(feature = "media")]
#[test]
fn budget_001_png_fast_margin_and_manifest_alternatives_use_real_bytes() {
    use base64::Engine;
    let mut pixels = Vec::new();
    for y in 0..64 {
        for x in 0..64 {
            pixels.extend_from_slice(&[x, y, x / 8 + y / 8, 255]);
        }
    }
    let mut options = oxideav_png::EncodeOptions::default();
    options.filter_strategy = oxideav_png::FilterStrategy::Fixed(oxideav_png::FilterType::None);
    options.compression_level = Some(1);
    let raw = oxideav_png::encode_plane(
        64,
        64,
        oxideav_png::PngPixelFormat::Rgba,
        256,
        &pixels,
        None,
        &options,
    )
    .unwrap();
    let fast = suprnova::recompress_png(&raw, suprnova::PngEffort::Fast).unwrap();
    assert!(raw.len() > fast.len());
    let capture = |bytes: &[u8]| {
        let mut png = entry("/icon.png", 200, "image/png", "", bytes.len() as u64, &[]);
        png["response"]["content"]["text"] =
            serde_json::json!(base64::engine::general_purpose::STANDARD.encode(bytes));
        png["response"]["content"]["encoding"] = serde_json::json!("base64");
        har(vec![
            entry(
                "/",
                200,
                "text/html",
                "<link rel='manifest' href='/app.webmanifest'>",
                50,
                &[],
            ),
            entry(
                "/app.webmanifest",
                200,
                "application/manifest+json",
                r#"{"icons":[{"src":"icon.png","sizes":"64x64","type":"image/png"},{"src":"icon.webp","sizes":"64x64","type":"image/webp"}]}"#,
                30,
                &[],
            ),
            png,
            entry("/icon.webp", 200, "image/webp", "webp", 1, &[]),
        ])
    };
    let strict = PageBudget {
        raster_threshold_bytes: 1,
        png_margin_percent: 0,
        ..PageBudget::default()
    };
    let audit = strict.audit_har(&capture(&raw)).unwrap();
    assert!(
        audit.findings.iter().any(|f| f.rule == "png-compression"),
        "{audit}"
    );
    assert!(
        !audit.findings.iter().any(|f| f.rule == "raster-format"),
        "matching manifest alternatives must pass: {audit}"
    );
    strict
        .audit_har(&capture(&fast))
        .unwrap()
        .ensure_passes()
        .unwrap();
    let generous = PageBudget {
        png_margin_percent: 100_000,
        ..strict
    };
    generous
        .audit_har(&capture(&raw))
        .unwrap()
        .ensure_passes()
        .unwrap();
}

#[tokio::test]
async fn budget_001_failed_fixture_matches_the_har_and_console() {
    let html = "<script src='/main-ABcd1234.js'></script><img src='/blocked.png'>";
    let router = Router::new()
        .get("/", move |_req: Request| async move {
            Ok(HttpResponse::html(html))
        })
        .get("/main-ABcd1234.js", |_req: Request| async {
            Ok(HttpResponse::text("script").header("Set-Cookie", "session=secret"))
        })
        .get("/blocked.png", |_req: Request| async {
            Ok(HttpResponse::text("sign in").status(403))
        });
    let audit = TestClient::new(router, MiddlewareRegistry::new())
        .audit_page("/", &PageBudget::default())
        .await
        .unwrap();
    let capture = har(vec![
        entry("/", 200, "text/html", html, html.len() as u64, &[]),
        entry(
            "/main-ABcd1234.js",
            200,
            "text/plain",
            "script",
            6,
            &[("Set-Cookie", "session=redacted")],
        ),
        entry("/blocked.png", 403, "text/plain", "sign in", 7, &[]),
    ]);
    assert_eq!(audit, PageBudget::default().audit_har(&capture).unwrap());
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), capture).unwrap();
    let run = suprnova::console::test(["budget:audit", file.path().to_str().unwrap()])
        .run()
        .await;
    run.assert_exit_code(1);
    assert!(run.output().contains(&audit.to_string()));
}

fn origin_client(root: &std::path::Path) -> (TestClient, std::sync::Arc<CountingSessionStore>) {
    suprnova::testing::install_test_encryption_key();
    let config = suprnova::SessionConfig::default();
    let store = std::sync::Arc::new(CountingSessionStore::default());
    let router = suprnova::fallback!(suprnova::StaticFiles::from_dir(root).handler()).register(
        Router::new()
            .get("/", |_req: Request| async {
                Ok(HttpResponse::html(
                    "<script src='/main-BjmpJyAU.js'></script>",
                ))
            })
            .into(),
    );
    (
        TestClient::new(
            router,
            MiddlewareRegistry::new()
                .append(suprnova::SessionMiddleware::with_store(
                    config.clone(),
                    store.clone(),
                ))
                .append(suprnova::CsrfMiddleware::new().with_session_config(&config)),
        ),
        store,
    )
}

#[derive(Default)]
struct CountingSessionStore {
    writes: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl suprnova::SessionStore for CountingSessionStore {
    async fn read(
        &self,
        _: &str,
    ) -> Result<Option<suprnova::SessionData>, suprnova::FrameworkError> {
        Ok(None)
    }
    async fn write(&self, _: &suprnova::SessionData) -> Result<(), suprnova::FrameworkError> {
        self.writes
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
    async fn destroy(&self, _: &str) -> Result<(), suprnova::FrameworkError> {
        Ok(())
    }
    async fn destroy_for_user(&self, _: &str) -> Result<u64, suprnova::FrameworkError> {
        Ok(0)
    }
    async fn gc(&self) -> Result<u64, suprnova::FrameworkError> {
        Ok(0)
    }
}

#[tokio::test]
async fn budget_002_fingerprinted_first_visit_is_public_immutable_and_cookie_free() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("main-BjmpJyAU.js"), "export {};").unwrap();
    let (client, store) = origin_client(dir.path());
    let response = client.get("/main-BjmpJyAU.js").send().await;
    response.assert_ok();
    assert_eq!(
        response.header("Cache-Control"),
        Some("public, max-age=31536000, immutable")
    );
    assert_eq!(response.header("Set-Cookie"), None);
    assert_eq!(store.writes.load(std::sync::atomic::Ordering::SeqCst), 0);
    let head = client
        .send(suprnova::Method::HEAD, "/main-BjmpJyAU.js")
        .send()
        .await;
    assert_eq!(
        head.header("Cache-Control"),
        response.header("Cache-Control")
    );
    assert_eq!(head.header("Set-Cookie"), None);
    client
        .audit_page("/", &PageBudget::default())
        .await
        .unwrap()
        .ensure_passes()
        .unwrap();
}

#[tokio::test]
async fn budget_002_other_files_keep_short_or_configured_cache_lifetimes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("logo.svg"), "<svg/>").unwrap();
    for name in [
        "apple-touch-icon.png",
        "android-chrome-512x512.png",
        "not-fingerprint.js",
        "site-manifest.json",
        "app-settings.js",
    ] {
        std::fs::write(dir.path().join(name), "asset").unwrap();
        let response = origin_client(dir.path())
            .0
            .get(format!("/{name}"))
            .send()
            .await;
        assert_eq!(
            response.header("Cache-Control"),
            Some("public, max-age=3600"),
            "{name}"
        );
    }
    let response = origin_client(dir.path()).0.get("/logo.svg").send().await;
    assert_eq!(
        response.header("Cache-Control"),
        Some("public, max-age=3600")
    );
    assert_eq!(response.header("Set-Cookie"), None);
    let router = suprnova::fallback!(
        suprnova::StaticFiles::from_dir(dir.path())
            .cache_control("public, max-age=60")
            .handler()
    )
    .register(Router::new());
    let client = TestClient::new(router, MiddlewareRegistry::new());
    assert_eq!(
        client.get("/logo.svg").send().await.header("Cache-Control"),
        Some("public, max-age=60")
    );
}

#[tokio::test]
async fn budget_002_pages_keep_their_session_and_csrf_cookies() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("main-BjmpJyAU.js"), "export {};").unwrap();
    let (client, store) = origin_client(dir.path());
    let page = client.get("/").send().await;
    assert!(page.cookie("suprnova_session").is_some());
    assert!(page.cookie("XSRF-TOKEN").is_some());
    assert_eq!(store.writes.load(std::sync::atomic::Ordering::SeqCst), 1);
    let asset = client.get("/main-BjmpJyAU.js").send().await;
    assert_eq!(asset.header("Set-Cookie"), None);
    assert_eq!(store.writes.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(
        asset.header("Cache-Control"),
        Some("public, max-age=31536000, immutable")
    );
}

#[test]
fn budget_001_fallback_pairs_use_response_types_for_extensionless_images() {
    let html = "<picture><source srcset='/modern'><img src='/fallback'></picture>";
    let capture = har(vec![
        entry("/", 200, "text/html", html, html.len() as u64, &[]),
        entry("/modern", 200, "image/avif", "avif", 120_000, &[]),
        entry("/fallback", 200, "image/jpeg", "jpeg", 120_000, &[]),
    ]);
    PageBudget::default()
        .audit_har(&capture)
        .unwrap()
        .ensure_passes()
        .unwrap();
}

#[test]
fn budget_001_conflicting_cache_directives_fail_the_immutable_rule() {
    for policy in [
        "public, max-age=31536000, max-age=86400, immutable",
        "public, max-age=31536000, immutable, no-cache=\"Content-Type\"",
    ] {
        let capture = har(vec![
            entry("/", 200, "text/html", "", 1, &[]),
            entry(
                "/main-BjmpJyAU.js",
                200,
                "text/javascript",
                "x",
                1,
                &[("Cache-Control", policy)],
            ),
        ]);
        let audit = PageBudget::default().audit_har(&capture).unwrap();
        assert!(
            audit.findings.iter().any(|f| f.rule == "fingerprint-cache"),
            "{policy}: {audit}"
        );
    }
}

#[test]
fn budget_001_a_cookie_bearing_har_cannot_prove_cookie_independence() {
    let mut asset = entry("/asset.js", 200, "text/javascript", "x", 1, &[]);
    asset["request"]["headers"] = serde_json::json!([{"name":"Cookie","value":"redacted=1"}]);
    let budget = PageBudget {
        forbid_asset_request_cookie: false,
        ..PageBudget::default()
    };
    let audit = budget
        .audit_har(&har(vec![entry("/", 200, "text/html", "", 1, &[]), asset]))
        .unwrap();
    assert!(
        audit
            .findings
            .iter()
            .any(|f| f.rule == "cookie-independent"),
        "{audit}"
    );
}

#[test]
fn budget_001_initial_https_upgrade_does_not_list_the_page_host_as_third_party() {
    let first = entry(
        "/",
        302,
        "text/html",
        "",
        0,
        &[("Location", "https://localhost/page")],
    );
    let mut page = entry("/page", 200, "text/html", "<p>Page</p>", 11, &[]);
    page["request"]["url"] = serde_json::json!("https://localhost/page");
    let audit = PageBudget::default()
        .audit_har(&har(vec![first, page]))
        .unwrap();
    assert!(audit.third_party_hosts.is_empty(), "{audit}");
    assert_eq!(audit.findings.len(), 1);
    assert_eq!(audit.findings[0].rule, "first-redirect");
}

mod review_discovery;
mod review_env;
mod review_limits;
mod review_manifest;

mod css_work;
mod re_review;

#[tokio::test]
async fn budget_001_cross_origin_scope_agrees_in_all_modes() {
    if std::env::var_os("BUDGET_SCOPE_CHILD").is_none() {
        for hosts in ["fonts.example", ""] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "budget_001_cross_origin_scope_agrees_in_all_modes",
                    "--nocapture",
                ])
                .env("BUDGET_SCOPE_CHILD", "1")
                .env("PAGE_BUDGET_ALLOWED_THIRD_PARTY_HOSTS", hosts)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        return;
    }
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let html = "<link rel=preload as=font href='https://fonts.example/font-ABcd1234.woff2'><script src='/app-ABcd1234.js'></script>";
    let router = Router::new()
        .get("/", move |_: Request| async move {
            Ok(HttpResponse::html(html))
        })
        .get("/app-ABcd1234.js", |_: Request| async {
            Ok(HttpResponse::text("ok")
                .header("Cache-Control", "public, max-age=31536000, immutable"))
        });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let count = stream.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..count]);
            let (body, mime) = if request.starts_with("GET / ") {
                (html, "text/html")
            } else {
                ("ok", "text/javascript")
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nCache-Control: public, max-age=31536000, immutable\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        }
    });
    let url = format!("http://{address}/");
    let mut entries = vec![
        entry("/", 200, "text/html", html, html.len() as u64, &[]),
        entry(
            "/app-ABcd1234.js",
            200,
            "text/javascript",
            "ok",
            2,
            &[("Cache-Control", "public, max-age=31536000, immutable")],
        ),
        entry(
            "/font-ABcd1234.woff2",
            403,
            "font/woff2",
            "",
            2_000_000,
            &[("Set-Cookie", "external=1")],
        ),
    ];
    entries[0]["request"]["url"] = serde_json::json!(url);
    entries[1]["request"]["url"] = serde_json::json!(format!("{url}app-ABcd1234.js"));
    entries[2]["request"]["url"] = serde_json::json!("https://fonts.example/font-ABcd1234.woff2");
    entries[2]["request"]["cookies"] = serde_json::json!([{"name":"external","value":"1"}]);
    let budget = PageBudget::from_env().unwrap();
    let helper = TestClient::new(router, MiddlewareRegistry::new())
        .audit_page(&url, &budget)
        .await
        .unwrap();
    let live = budget.audit_url(&url).await.unwrap();
    let captured = budget.audit_har(&har(entries)).unwrap();
    assert_eq!(helper, live);
    assert_eq!(helper, captured);
    assert_eq!(helper.third_party_hosts, ["fonts.example"]);
    assert_eq!(helper.requests, 2);
    let allowed = budget
        .allowed_third_party_hosts
        .as_ref()
        .unwrap()
        .contains("fonts.example");
    if allowed {
        helper.ensure_passes().unwrap();
    } else {
        assert_eq!(helper.findings.len(), 1);
        assert_eq!(helper.findings[0].rule, "third-party-host");
    }
    let command = suprnova::console::test(["budget:audit", &url]).run().await;
    command.assert_exit_code(if allowed { 0 } else { 1 });
    assert!(
        command.output().contains(&helper.to_string()),
        "{}",
        command.output()
    );
    server.abort();
}

#[tokio::test]
async fn budget_001_manifest_output_files_require_immutable_in_all_modes() {
    if std::env::var_os("BUDGET_MANIFEST_CHILD").is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "budget_001_manifest_output_files_require_immutable_in_all_modes",
                "--nocapture",
            ])
            .env("BUDGET_MANIFEST_CHILD", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    use suprnova::http_client::{FakeResponse, Http};
    let _container = suprnova::testing::TestContainer::fake();
    let dir = tempfile::tempdir().unwrap();
    let manifest = dir.path().join("custom-manifest.json");
    std::fs::write(&manifest, include_str!("fixtures/build-manifest.json")).unwrap();
    suprnova::Inertia::install(
        &suprnova::InertiaConfig::new()
            .development(true)
            .manifest_path(&manifest)
            .assets_base_url("/build"),
    )
    .unwrap();
    let html = "<script src='/build/app-deadbeef.js'></script><link rel=stylesheet href='/build/app-abcdefgh.css'><link rel=preload as=font href='/build/font-abcdefgh.woff2'><link rel=modulepreload href='/build/lazy-abcdefgh.js'><link rel=manifest href='/build/site-manifest.json'>";
    let resources = [
        ("/build/app-deadbeef.js", "text/javascript", "ok"),
        ("/build/app-abcdefgh.css", "text/css", "body{}"),
        ("/build/font-abcdefgh.woff2", "font/woff2", "font"),
        ("/build/lazy-abcdefgh.js", "text/javascript", "ok"),
        (
            "/build/site-manifest.json",
            "application/manifest+json",
            "{}",
        ),
    ];
    let mut router = Router::new().get("/", move |_: Request| async move {
        Ok(HttpResponse::html(html))
    });
    for (path, mime, body) in resources {
        router =
            router.get(path, move |_: Request| async move {
                Ok(HttpResponse::bytes_body(body, mime)
                    .header("Cache-Control", "public, max-age=3600"))
            });
    }
    let helper = TestClient::new(router, MiddlewareRegistry::new())
        .audit_page("/", &PageBudget::default())
        .await
        .unwrap();
    let mut entries = vec![entry("/", 200, "text/html", html, html.len() as u64, &[])];
    for (path, mime, body) in resources {
        entries.push(entry(
            path,
            200,
            mime,
            body,
            body.len() as u64,
            &[("Cache-Control", "public, max-age=3600")],
        ));
    }
    let capture = har(entries);
    let captured = PageBudget::default().audit_har(&capture).unwrap();
    assert_eq!(helper, captured);
    let har_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(har_file.path(), &capture).unwrap();
    let command = suprnova::console::test(["budget:audit", har_file.path().to_str().unwrap()])
        .run()
        .await;
    command.assert_exit_code(1);
    assert!(
        command.output().contains(&helper.to_string()),
        "{}",
        command.output()
    );
    assert_eq!(helper.findings.len(), 4, "{helper}");
    assert!(
        helper
            .findings
            .iter()
            .all(|finding| finding.rule == "fingerprint-cache")
    );
    Http::fake(|| async {
        Http::prevent_stray_requests(true);
        Http::fake_url(
            "http://localhost/",
            FakeResponse::new(200)
                .body(html)
                .header("Content-Type", "text/html"),
        );
        for (path, mime, body) in resources {
            Http::fake_url(
                &format!("http://localhost{path}"),
                FakeResponse::new(200)
                    .body(body)
                    .header("Content-Type", mime)
                    .header("Cache-Control", "public, max-age=3600"),
            );
        }
        let live = PageBudget::default()
            .audit_url("http://localhost/")
            .await
            .unwrap();
        assert_eq!(helper, live);
        let command = suprnova::console::test(["budget:audit", "http://localhost/"])
            .run()
            .await;
        command.assert_exit_code(1);
        assert!(
            command.output().contains(&helper.to_string()),
            "{}",
            command.output()
        );
    })
    .await;
}

#[tokio::test]
async fn budget_001_cross_origin_redirect_port_keeps_the_same_scope() {
    if std::env::var_os("BUDGET_PORT_CHILD").is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "budget_001_cross_origin_redirect_port_keeps_the_same_scope",
                "--nocapture",
            ])
            .env("BUDGET_PORT_CHILD", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    use suprnova::http_client::{FakeResponse, Http};
    let client = TestClient::new(
        Router::new().get("/", |_: Request| async {
            Ok(HttpResponse::text("")
                .status(302)
                .header("Location", "http://audit.test:8080/"))
        }),
        MiddlewareRegistry::new(),
    );
    let helper = client
        .audit_page("http://audit.test/", &PageBudget::default())
        .await
        .unwrap();
    let mut first = entry(
        "/",
        302,
        "text/html",
        "",
        0,
        &[("Location", "http://audit.test:8080/")],
    );
    first["request"]["url"] = serde_json::json!("http://audit.test/");
    let mut external = entry("/", 200, "text/html", "", 2_000_000, &[]);
    external["request"]["url"] = serde_json::json!("http://audit.test:8080/");
    let captured = PageBudget::default()
        .audit_har(&har(vec![first, external]))
        .unwrap();
    assert_eq!(helper, captured);
    assert_eq!(helper.third_party_hosts, ["audit.test"]);
    assert_eq!(helper.findings.len(), 1);
    assert_eq!(helper.findings[0].rule, "first-redirect");
    Http::fake(|| async {
        Http::prevent_stray_requests(true);
        Http::fake_url(
            "http://audit.test/",
            FakeResponse::new(302).header("Location", "http://audit.test:8080/"),
        );
        assert_eq!(
            helper,
            PageBudget::default()
                .audit_url("http://audit.test/")
                .await
                .unwrap()
        );
    })
    .await;
}

#[tokio::test]
async fn budget_001_review_manifest_mutable_names_keep_revalidation() {
    if std::env::var_os("REVIEW_MANIFEST_CHILD").is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "budget_001_review_manifest_mutable_names_keep_revalidation",
                "--nocapture",
            ])
            .env("REVIEW_MANIFEST_CHILD", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let _container = suprnova::testing::TestContainer::fake();
    let dir = tempfile::tempdir().unwrap();
    let manifest = dir.path().join("review-manifest.json");
    std::fs::write(&manifest, include_str!("fixtures/review-manifest.json")).unwrap();
    suprnova::Inertia::install(
        &suprnova::InertiaConfig::new()
            .development(true)
            .manifest_path(&manifest)
            .assets_base_url("/build"),
    )
    .unwrap();
    let files = [
        "index.html",
        "app.js",
        "theme.css",
        "logo.svg",
        "app-deadbeef.js",
        "escape-abcdefgh.js",
        "abs-abcdefgh.js",
        "site-manifest.json",
    ];
    let mut entries = vec![entry("/", 200, "text/html", "", 0, &[])];
    for file in files {
        entries.push(entry(
            &format!("/build/{file}"),
            200,
            "text/javascript",
            "asset",
            5,
            &[("Cache-Control", "public, max-age=3600")],
        ));
    }
    let audit = PageBudget::default().audit_har(&har(entries)).unwrap();
    assert_eq!(audit.findings.len(), 1, "{audit}");
    assert_eq!(audit.findings[0].rule, "fingerprint-cache");
    assert!(audit.findings[0].asset.ends_with("/build/app-deadbeef.js"));
    std::fs::write(&manifest, r#"{"entry":{"file":"app.deadbeef.js","css":["app-abcdefghijkl.css"],"assets":["short-abcdefg.js","noextension-deadbeef"]}}"#).unwrap();
    suprnova::Inertia::install(
        &suprnova::InertiaConfig::new()
            .development(true)
            .manifest_path(&manifest)
            .assets_base_url("/build"),
    )
    .unwrap();
    let mut entries = vec![entry("/", 200, "text/html", "", 0, &[])];
    for file in [
        "app.deadbeef.js",
        "app-abcdefghijkl.css",
        "short-abcdefg.js",
        "noextension-deadbeef",
    ] {
        entries.push(entry(
            &format!("/build/{file}"),
            200,
            "text/javascript",
            "asset",
            5,
            &[("Cache-Control", "public, max-age=3600")],
        ));
    }
    let audit = PageBudget::default().audit_har(&har(entries)).unwrap();
    assert_eq!(audit.findings.len(), 1, "{audit}");
    assert!(
        audit
            .findings
            .iter()
            .any(|finding| finding.asset.ends_with("/app.deadbeef.js"))
    );
    assert!(
        !audit
            .findings
            .iter()
            .any(|finding| finding.asset.ends_with("/app-abcdefghijkl.css")),
        "a twelve-letter segment is not Vite's default hash"
    );
}
