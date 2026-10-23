use super::*;
#[tokio::test]
async fn budget_001_all_cache_control_lines_count_in_every_runner() {
    use suprnova::http_client::{FakeResponse, Http};
    let html = "<script src='/main-BjmpJyAU.js'></script>";
    let policy = "public, max-age=31536000, immutable";
    let capture = har(vec![
        entry("/", 200, "text/html", html, html.len() as u64, &[]),
        entry(
            "/main-BjmpJyAU.js",
            200,
            "text/plain",
            "js",
            2,
            &[("Cache-Control", policy), ("Cache-Control", "no-cache")],
        ),
    ]);
    let har_audit = PageBudget::default().audit_har(&capture).unwrap();
    assert!(
        har_audit
            .findings
            .iter()
            .any(|f| f.rule == "fingerprint-cache"),
        "{har_audit}"
    );
    let router = Router::new()
        .get("/", move |_req: Request| async move {
            Ok(HttpResponse::html(html))
        })
        .get("/main-BjmpJyAU.js", move |_req: Request| async move {
            Ok(HttpResponse::text("js")
                .header("Cache-Control", policy)
                .header("Cache-Control", "no-cache"))
        });
    let audit = TestClient::new(router, MiddlewareRegistry::new())
        .audit_page("/", &PageBudget::default())
        .await
        .unwrap();
    assert_eq!(audit, har_audit);
    Http::fake(|| async {
        Http::fake_url(
            "http://localhost/",
            FakeResponse::new(200)
                .body(html)
                .header("Content-Type", "text/html"),
        );
        Http::fake_url(
            "http://localhost/main-BjmpJyAU.js",
            FakeResponse::new(200)
                .body("js")
                .header("Content-Type", "text/plain")
                .header("Cache-Control", policy)
                .header("Cache-Control", "no-cache"),
        );
        assert_eq!(
            PageBudget::default()
                .audit_url("http://localhost/")
                .await
                .unwrap(),
            har_audit
        );
    })
    .await;
}
#[tokio::test]
async fn budget_001_discovery_deduplicates_repeated_references() {
    let html = "<img src='/hero.jpg'>".repeat(10_000);
    let router = Router::new()
        .get("/", move |_req: Request| {
            let html = html.clone();
            async move { Ok(HttpResponse::html(html)) }
        })
        .get("/hero.jpg", |_req: Request| async {
            Ok(HttpResponse::text("small"))
        });
    let budget = PageBudget {
        max_resources: 2,
        ..PageBudget::default()
    };
    let audit = TestClient::new(router, MiddlewareRegistry::new())
        .audit_page("/", &budget)
        .await
        .unwrap();
    assert_eq!(audit.requests, 2);
}
#[tokio::test]
async fn budget_001_discovery_caps_distinct_references_before_fetching() {
    let html = (0..100)
        .map(|i| format!("<img src='/image{i}.jpg'>"))
        .collect::<String>();
    let fetched = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = fetched.clone();
    let router = suprnova::fallback!(move |_req: Request| {
        let count = count.clone();
        async move {
            count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(HttpResponse::text("small"))
        }
    })
    .register(
        Router::new()
            .get("/", move |_req: Request| {
                let html = html.clone();
                async move { Ok(HttpResponse::html(html)) }
            })
            .into(),
    );
    let budget = PageBudget {
        max_resources: 3,
        ..PageBudget::default()
    };
    let error = TestClient::new(router, MiddlewareRegistry::new())
        .audit_page("/", &budget)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("resource"));
    assert_eq!(
        fetched.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "reject a document before fetching its oversized graph"
    );
}

#[tokio::test]
async fn budget_001_http_retains_cache_control_lines_with_non_ascii_octets() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        for asset in [false, true] {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut chunk = [0; 1024];
                let count = stream.read(&mut chunk).await.unwrap();
                assert_ne!(count, 0, "request headers must complete");
                request.extend_from_slice(&chunk[..count]);
                assert!(
                    request.len() <= 8192,
                    "fixture request headers stay bounded"
                );
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let body = if asset {
                "js"
            } else {
                "<script src='/main-BjmpJyAU.js'></script>"
            };
            let mut response = format!("HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: {}\r\nContent-Type: {}\r\n",body.len(), if asset { "text/javascript" } else { "text/html" }).into_bytes();
            if asset {
                response.extend_from_slice(b"Cache-Control: public, max-age=31536000, immutable\r\nCache-Control: no-cache,\xff\r\n");
            }
            response.extend_from_slice(b"\r\n");
            response.extend_from_slice(body.as_bytes());
            stream.write_all(&response).await.unwrap();
        }
    });
    let audit = PageBudget::default()
        .audit_url(&format!("http://{address}/"))
        .await
        .unwrap();
    server.await.unwrap();
    assert!(
        audit.findings.iter().any(|f| f.rule == "fingerprint-cache"),
        "{audit}"
    );
}
