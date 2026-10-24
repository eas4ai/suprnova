use super::support::{Harness, debug_mode, exchange, page, request};
use bytes::Bytes;
use serde_json::json;
use std::sync::Arc;
use suprnova::{HttpResponse, MiddlewareRegistry, Request, Router, SecurityHeaders};

#[tokio::test]
async fn dbg_002_injects_before_body_under_the_response_nonce_without_assets() {
    let _mode = debug_mode(true, &[]).await;
    let app = Harness::with_middleware(
        Router::new()
            .get("/", |request: Request| async move {
                let nonce = request.csp_nonce().expect("security middleware's nonce");
                Ok(
                    HttpResponse::html(format!("<html><body><p>{nonce}</p></BoDy></html>"))
                        .header("content-length", "1")
                        .header("etag", "old-body"),
                )
            })
            .into(),
        MiddlewareRegistry::new().append(SecurityHeaders::strict()),
    );
    let reply = app
        .get("/", &[("accept", "application/json, text/html;q=0.8")])
        .await;
    assert_eq!(reply.status, 200);
    let policy = reply.header("content-security-policy").expect("CSP");
    let nonce = policy
        .split("'nonce-")
        .nth(1)
        .and_then(|part| part.split('\'').next())
        .expect("nonce source");
    assert_eq!(reply.body.matches(&format!("nonce=\"{nonce}\"")).count(), 3);
    assert!(
        reply.body.find("id=\"suprnova-debugbar\"").unwrap() < reply.body.find("</BoDy>").unwrap()
    );
    assert!(reply.body.contains(reply.id()));
    assert!(reply.body.contains("XMLHttpRequest.prototype.send"));
    assert!(reply.body.contains("window.fetch ="));
    assert!(!reply.body.contains("<script src="));
    assert!(!reply.body.contains("<link "));
    assert_eq!(reply.header("etag"), None);
    assert_eq!(reply.header("cache-control"), Some("no-store"));
    assert_eq!(
        reply
            .header("content-length")
            .unwrap()
            .parse::<usize>()
            .unwrap(),
        reply.body.len()
    );
}

#[tokio::test]
async fn dbg_002_mixed_case_csp_directives_receive_nonce_without_duplicates() {
    let _mode = debug_mode(true, &[]).await;
    for policy in [
        "Script-Src 'self'; Style-Src 'self'",
        "Default-Src 'NONE'; Script-Src 'NONE'; Style-Src 'NONE'; Script-Src-Elem 'self'; Style-Src-Elem 'self'",
        "Script-Src 'UNSAFE-INLINE' 'SHA256-47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU='; Style-Src 'self'",
    ] {
        let app = Harness::new(
            Router::new()
                .get("/", move |_: Request| async move {
                    Ok(HttpResponse::html("<body>mixed case policy</body>")
                        .header("content-security-policy", policy))
                })
                .into(),
        );
        let reply = app.get("/", &[("accept", "text/html")]).await;
        let nonce = reply
            .body
            .split(" nonce=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        let adjusted = reply.header("content-security-policy").expect("CSP");
        if policy.starts_with("Default-Src 'NONE'") {
            assert!(adjusted.contains("connect-src 'self'"));
        }
        for name in [
            "script-src",
            "style-src",
            "script-src-elem",
            "style-src-elem",
        ] {
            let directives: Vec<_> = adjusted
                .split(';')
                .filter(|directive| {
                    directive
                        .split_whitespace()
                        .next()
                        .is_some_and(|token| token.eq_ignore_ascii_case(name))
                })
                .collect();
            if name.ends_with("-elem") && !policy.to_ascii_lowercase().contains(name) {
                assert!(directives.is_empty());
                continue;
            }
            assert_eq!(directives.len(), 1, "duplicate {name}: {adjusted}");
            let sources: Vec<_> = directives[0].split_whitespace().skip(1).collect();
            assert!(
                sources.contains(&format!("'nonce-{nonce}'").as_str()),
                "missing nonce in {name}: {adjusted}"
            );
            assert!(
                !sources
                    .iter()
                    .any(|source| source.eq_ignore_ascii_case("'none'"))
            );
        }
    }
}

#[tokio::test]
async fn dbg_002_mixed_case_csp_keywords_preserve_inline_and_existing_nonce_sources() {
    let _mode = debug_mode(true, &[]).await;
    for policy in [
        "default-src 'self' 'UNSAFE-INLINE'",
        "script-src 'NoNcE-FiXtUrEnOnCe'; style-src 'NONCE-FiXtUrEnOnCe'",
    ] {
        let app = Harness::new(
            Router::new()
                .get("/", move |_: Request| async move {
                    Ok(HttpResponse::html("<body>mixed case keywords</body>")
                        .header("content-security-policy", policy))
                })
                .into(),
        );
        let reply = app.get("/", &[("accept", "text/html")]).await;
        assert_eq!(reply.header("content-security-policy"), Some(policy));
        if policy.contains("NoNcE-") {
            assert_eq!(reply.body.matches("nonce=\"FiXtUrEnOnCe\"").count(), 3);
        }
    }
}

#[tokio::test]
async fn dbg_002_excludes_json_redirect_file_stream_head_and_inertia_responses() {
    let _mode = debug_mode(true, &[]).await;
    let app = Harness::new(
        Router::new()
            .get("/html", page)
            .get("/json", |_: Request| async {
                Ok(HttpResponse::json(json!({"ok":true})))
            })
            .get("/redirect", |_: Request| async {
                Ok(HttpResponse::html("<body>redirect</body>")
                    .status(302)
                    .header("location", "/html"))
            })
            .get("/file", |_: Request| async {
                Ok(HttpResponse::html("<body>file</body>")
                    .header("content-disposition", "attachment; filename=page.html"))
            })
            .get("/stream", |_: Request| async {
                Ok(HttpResponse::stream_bytes(futures::stream::iter([Ok::<
                    _,
                    std::convert::Infallible,
                >(
                    Bytes::from_static(b"<body>stream</body>"),
                )]))
                .header("content-type", "text/html"))
            })
            .get("/inertia", |_: Request| async {
                Ok(HttpResponse::json(json!({"component":"Home"})).header("x-inertia", "true"))
            })
            .into(),
    );
    for path in ["/json", "/redirect", "/file", "/stream", "/inertia"] {
        let reply = app.get(path, &[("accept", "text/html")]).await;
        assert!(reply.header("x-debugbar-id").is_some(), "{path} has no id");
        assert!(
            !reply.body.contains("suprnova-debugbar"),
            "injected into {path}"
        );
    }
    for headers in [
        vec![("accept", "text/html"), ("x-inertia", "true")],
        vec![("accept", "application/json")],
        vec![],
    ] {
        let reply = app.get("/html", &headers).await;
        assert!(!reply.body.contains("suprnova-debugbar"));
        assert!(reply.header("x-debugbar-id").is_some());
    }
    let head = exchange(
        Arc::clone(&app.router),
        Arc::clone(&app.registry),
        request("HEAD", "/html", &[("accept", "text/html")]),
    )
    .await;
    assert!(head.body.is_empty());
    assert!(head.header("x-debugbar-id").is_some());
}

#[tokio::test]
async fn dbg_002_inertia_and_xhr_records_are_fetchable_by_header_id() {
    let _mode = debug_mode(true, &[]).await;
    let app = Harness::new(
        Router::new()
            .get("/visit", |_: Request| async {
                Ok(HttpResponse::json(json!({"ok":true})))
            })
            .into(),
    );
    for headers in [
        vec![("x-inertia", "true")],
        vec![("x-requested-with", "XMLHttpRequest")],
    ] {
        let reply = app.get("/visit", &headers).await;
        let record = app.record(&reply).await;
        assert_eq!(record["id"], reply.id());
        assert_eq!(record["request"]["path"], "\"/visit\"");
    }
}

#[tokio::test]
async fn dbg_002_keeps_only_the_configured_recent_requests() {
    let _mode = debug_mode(true, &[("DEBUGBAR_KEEP", "2")]).await;
    let middleware = suprnova::DebugBar::middleware().expect("debug bar");
    let bounded = Harness {
        router: Arc::new(Router::new().get("/", page).into()),
        registry: Arc::new(MiddlewareRegistry::new().append(middleware.clone())),
    };
    for _ in 0..8 {
        let reply = bounded.get("/", &[("accept", "text/html")]).await;
        assert!(reply.body.contains("data-keep=\"2\""));
        assert!(middleware.record_count_for_test().expect("count") <= 2);
    }
    // Each lookup is itself a request, so inspect each position in its own store.
    for (position, expected) in [(0, 404), (1, 200), (2, 200)] {
        let app = Harness::new(Router::new().get("/", page).into());
        let mut ids = Vec::new();
        for _ in 0..3 {
            ids.push(app.get("/", &[]).await.id().to_owned());
        }
        let reply = app.get(&format!("/_debugbar/{}", ids[position]), &[]).await;
        assert_eq!(reply.status, expected);
        assert!(reply.header("x-debugbar-id").is_some());
        if expected == 200 {
            assert_eq!(reply.json()["id"], ids[position]);
        }
    }
}

#[tokio::test]
async fn dbg_002_embedded_data_cannot_terminate_its_script_or_create_markup() {
    let _mode = debug_mode(true, &[]).await;
    let app = Harness::new(Router::new().get("/", page).into());
    let reply = app
        .get(
            "/?note=%3C%2Fscript%3E%3Cimg%20src%3Dx%20onerror%3Dalert(1)%3E",
            &[("accept", "text/html")],
        )
        .await;
    assert!(reply.body.contains("\\u003c/script\\u003e"));
    assert!(!reply.body.contains("<img src=x"));
    assert_eq!(reply.body.matches("</script>").count(), 2);
}

#[tokio::test]
async fn dbg_002_lookup_and_toolbar_follow_the_application_path_prefix() {
    let _mode = debug_mode(true, &[("APP_URL", "http://localhost/tools")]).await;
    let app = Harness::new(Router::new().get("/", page).into());
    // The reverse proxy strips the public prefix from ordinary application requests.
    let reply = app.get("/", &[("accept", "text/html")]).await;
    assert_eq!(reply.status, 200);
    assert!(reply.body.contains("data-endpoint=\"/tools/_debugbar/\""));
    let lookup = app
        .get(&format!("/tools/_debugbar/{}", reply.id()), &[])
        .await;
    assert_eq!(lookup.status, 200);
    assert_eq!(lookup.json()["id"], reply.id());
}

#[tokio::test]
async fn dbg_002_error_pages_and_report_only_policies_allow_the_toolbar_nonce() {
    let _mode = debug_mode(true, &[]).await;
    let app = Harness::new(Router::new()
        .get("/error", |_: Request| async { Err(HttpResponse::from(suprnova::FrameworkError::internal("debug bar error fixture"))) })
        .get("/stream-error", |_: Request| async {
            let report = HttpResponse::from(suprnova::FrameworkError::internal("stream error fixture"));
            Ok(HttpResponse::stream_bytes(futures::stream::iter([Ok::<_, std::convert::Infallible>(Bytes::from_static(b"stream failed"))])).status(500).with_error_report_of(report))
        })
        .get("/report-only", |_: Request| async {
            Ok(HttpResponse::html("<body>report only</body>").header("content-security-policy-report-only", "default-src 'none'; script-src 'nonce-fixturenonce'; style-src 'nonce-fixturenonce'; connect-src 'self'"))
        })
        .get("/legacy", |_: Request| async {
            Ok(HttpResponse::html("<body>legacy inline policy</body>").header("content-security-policy", "default-src 'self' 'unsafe-inline'"))
        }).into());
    let error = app.get("/error", &[("accept", "text/html")]).await;
    assert_eq!(error.status, 500);
    assert!(error.body.contains("suprnova-debugbar"));
    let policy = error
        .header("content-security-policy")
        .expect("error page CSP");
    let nonce = policy
        .split("'nonce-")
        .nth(1)
        .and_then(|value| value.split('\'').next())
        .expect("error toolbar nonce");
    assert_eq!(error.body.matches(&format!("nonce=\"{nonce}\"")).count(), 3);
    assert!(policy.contains("script-src"));
    assert!(policy.contains("connect-src 'self'"));
    let stream_error = app.get("/stream-error", &[("accept", "text/html")]).await;
    assert_eq!(stream_error.status, 500);
    assert!(stream_error.body.contains("suprnova-debugbar"));
    let report_only = app.get("/report-only", &[("accept", "text/html")]).await;
    assert_eq!(
        report_only.body.matches("nonce=\"fixturenonce\"").count(),
        3
    );
    let legacy = app.get("/legacy", &[("accept", "text/html")]).await;
    assert_eq!(
        legacy.header("content-security-policy"),
        Some("default-src 'self' 'unsafe-inline'")
    );
}
