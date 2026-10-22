use super::*;

async fn audit(
    html: String,
    budget: PageBudget,
) -> Result<suprnova::page_budget::PageAudit, suprnova::FrameworkError> {
    let router = suprnova::fallback!(|_req: Request| async { Ok(HttpResponse::text("small")) })
        .register(
            Router::new()
                .get("/", move |_req: Request| {
                    let html = html.clone();
                    async move { Ok(HttpResponse::html(html)) }
                })
                .into(),
        );
    TestClient::new(router, MiddlewareRegistry::new())
        .audit_page("/", &budget)
        .await
}
#[test]
fn budget_001_image_set_declares_a_modern_fallback_group() {
    let html = "<style>body{background:image-set('/hero.avif' 1x type('image/avif'), '/hero.jpg' 1x type('image/jpeg'))}</style>";
    PageBudget::default()
        .audit_har(&har(vec![
            entry("/", 200, "text/html", html, 100, &[]),
            entry("/hero.avif", 200, "image/avif", "avif", 120_000, &[]),
            entry("/hero.jpg", 200, "image/jpeg", "jpeg", 120_000, &[]),
        ]))
        .unwrap()
        .ensure_passes()
        .unwrap();
}
#[tokio::test]
async fn budget_001_resource_limit_counts_normalized_http_urls_only() {
    for html in [
        "<img src='/hero.jpg'><img src='./hero.jpg'><img src='http://localhost/hero.jpg'><img src='data:image/png;base64,AAAA'><img src='#local'>",
        "<div style=\"background:url('/hero.jpg'),url('./hero.jpg'),url('http://localhost/hero.jpg'),url('data:image/png;base64,AAAA'),url('#local')\"></div>",
    ] {
        let result = audit(
            html.into(),
            PageBudget {
                max_resources: 2,
                ..PageBudget::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(result.requests, 2, "{result}");
        result.ensure_passes().unwrap();
    }
}
#[tokio::test]
async fn budget_001_html_overflow_stops_tokenization() {
    let html = (0..10_000)
        .map(|i| format!("<img src='/{i}.svg'>"))
        .collect();
    let error = audit(
        html,
        PageBudget {
            max_resources: 3,
            ..PageBudget::default()
        },
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(error.contains("HTML tokens processed: 3"), "{error}");
}
#[tokio::test]
async fn budget_001_inline_and_block_css_overflow_stop_html_processing() {
    let css = (0..10_000)
        .map(|i| format!("background:url('/{i}.svg');"))
        .collect::<String>();
    for (html, tokens) in [
        (
            format!(
                "<div style=\"{css}\"></div>{}",
                "<p>tail</p>".repeat(10_000)
            ),
            1,
        ),
        (
            format!("<style>{css}</style>{}", "<p>tail</p>".repeat(10_000)),
            2,
        ),
    ] {
        let error = audit(
            html,
            PageBudget {
                max_resources: 3,
                ..PageBudget::default()
            },
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(
            error.contains(&format!("HTML tokens processed: {tokens}")),
            "{error}"
        );
    }
}
#[tokio::test]
async fn budget_001_ignored_reference_tokens_have_a_separate_work_bound() {
    let error = audit(
        "<img src='#fragment'>".repeat(70_000),
        PageBudget {
            max_resources: 2,
            ..PageBudget::default()
        },
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(error.contains("discovery work limit"), "{error}");
}
#[tokio::test]
async fn budget_001_manifest_refuses_icons_before_materializing_the_tail() {
    let manifest = r#"{"icons":[{"src":"/0.svg"},{"src":"/1.svg"},{"src":"/2.svg"}, THIS_TAIL_MUST_NOT_BE_DESERIALIZED]}"#;
    let router = Router::new()
        .get("/", |_: Request| async {
            Ok(HttpResponse::html(
                "<link rel=manifest href='/app.webmanifest'>",
            ))
        })
        .get("/app.webmanifest", move |_: Request| async move {
            Ok(HttpResponse::bytes_body(
                manifest,
                "application/manifest+json",
            ))
        });
    let error = TestClient::new(router, MiddlewareRegistry::new())
        .audit_page(
            "/",
            &PageBudget {
                max_resources: 4,
                ..PageBudget::default()
            },
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("resource limit") && error.contains("icons processed: 3"),
        "{error}"
    );
}

#[tokio::test]
async fn budget_001_discovery_work_is_shared_across_stylesheet_responses() {
    let router = Router::new()
        .get("/", |_: Request| async {
            Ok(HttpResponse::html("<link rel=stylesheet href='/a.css'>"))
        })
        .get("/a.css", |_: Request| async {
            Ok(HttpResponse::bytes_body("url('/hero.jpg')", "text/css"))
        })
        .get("/hero.jpg", |_: Request| async {
            Ok(HttpResponse::text("small"))
        });
    let error = TestClient::new(router, MiddlewareRegistry::new())
        .audit_page(
            "/",
            &PageBudget {
                max_discovery_tokens: 4,
                ..PageBudget::default()
            },
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("discovery work limit of 4 tokens"),
        "{error}"
    );
}
#[tokio::test]
async fn budget_001_style_character_chunks_preserve_quoted_urls_and_comments() {
    let result=audit("<style>/* url('/ignored') */ body{background:url('/hero<.jpg')} .other{background:url('/hero2.jpg')}</style>".into(),PageBudget::default()).await.unwrap();
    assert_eq!(result.requests, 3, "{result}");
    result.ensure_passes().unwrap();
}
