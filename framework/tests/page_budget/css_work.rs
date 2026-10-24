use super::*;

fn css_capture(
    css: &str,
    limit: usize,
) -> Result<suprnova::page_budget::PageAudit, suprnova::FrameworkError> {
    PageBudget {
        max_discovery_tokens: limit,
        ..PageBudget::default()
    }
    .audit_har(&har(vec![
        entry(
            "/",
            200,
            "text/html",
            "<link rel=stylesheet href='/a.css'>",
            100,
            &[],
        ),
        entry("/a.css", 200, "text/css", css, 100, &[]),
        entry("/hero.jpg", 200, "image/jpeg", "small", 100, &[]),
    ]))
}

#[test]
fn budget_001_css_comments_consume_discovery_work() {
    for css in [
        "/* x */".repeat(1000),
        format!("body{{{}}}", "/* x */".repeat(1000)),
        format!(
            "body{{background:image-set('/hero.jpg' 1x {})}}",
            "/* x */".repeat(1000)
        ),
    ] {
        let error = css_capture(&css, 32).unwrap_err().to_string();
        assert!(
            error.contains("discovery work limit of 32 tokens"),
            "{error}"
        );
    }
}

#[test]
fn budget_001_css_url_argument_tail_consumes_discovery_work() {
    for tail in ["1 ".repeat(1000), "f(/* nested */ 1) ".repeat(1000)] {
        let css = format!("body{{background:url('/hero.jpg' {tail})}}");
        let error = css_capture(&css, 32).unwrap_err().to_string();
        assert!(
            error.contains("discovery work limit of 32 tokens"),
            "{error}"
        );
    }
}

#[tokio::test]
async fn budget_001_css_meter_preserves_valid_url_discovery() {
    let router = Router::new()
        .get("/", |_: Request| async { Ok(HttpResponse::html("<link rel=stylesheet href='/a.css'>")) })
        .get("/a.css", |_: Request| async { Ok(HttpResponse::bytes_body(r#"@import /* gap */ '/nested.css'; body{background:image-set('/hero.avif' 1x type('image/avif'),url('/hero.jpg') 1x type('image/jpeg'));mask:url( '/mask.svg' )}"#, "text/css")) })
        .get("/nested.css", |_: Request| async { Ok(HttpResponse::bytes_body("@font-face{font-family:Example;src:url('/font.woff2')}", "text/css")) })
        .get("/hero.avif", |_: Request| async { Ok(HttpResponse::bytes_body("avif", "image/avif")) })
        .get("/hero.jpg", |_: Request| async { Ok(HttpResponse::bytes_body("jpeg", "image/jpeg")) })
        .get("/mask.svg", |_: Request| async { Ok(HttpResponse::bytes_body("<svg/>", "image/svg+xml")) })
        .get("/font.woff2", |_: Request| async { Ok(HttpResponse::bytes_body("font", "font/woff2")) });
    let result = TestClient::new(router, MiddlewareRegistry::new())
        .audit_page("/", &PageBudget::default())
        .await
        .unwrap();
    assert_eq!(result.requests, 7, "{result}");
    result.ensure_passes().unwrap();
}
