use super::*;

async fn audit_reference(html: &str) -> suprnova::page_budget::PageAudit {
    let html = html.to_owned();
    let router = Router::new()
        .get("/", move |_req: Request| {
            let html = html.clone();
            async move { Ok(HttpResponse::html(html)) }
        })
        .get("/hero.jpg", |_req: Request| async {
            Ok(HttpResponse::bytes_body(vec![0u8; 120_001], "image/jpeg"))
        });
    TestClient::new(router, MiddlewareRegistry::new())
        .audit_page("/", &PageBudget::default())
        .await
        .unwrap()
}
async fn must_discover(html: &str) {
    let audit = audit_reference(html).await;
    assert_eq!(audit.requests, 2, "{html}: {audit}");
    assert!(
        audit
            .findings
            .iter()
            .any(|f| f.rule == "raster-format" && f.asset.ends_with("/hero.jpg")),
        "{html}: {audit}"
    );
}
#[tokio::test]
async fn budget_001_srcset_after_a_data_url() {
    must_discover("<img srcset='data:image/png;base64,AAAA, /hero.jpg 2x'>").await;
}
#[tokio::test]
async fn budget_001_imagesrcset_after_a_data_url() {
    must_discover(
        "<link rel=preload as=image imagesrcset='data:image/png;base64,AAAA, /hero.jpg 2x'>",
    )
    .await;
}
#[tokio::test]
async fn budget_001_style_attribute_raster_fails() {
    must_discover("<div style=\"background:url('/hero.jpg')\"></div>").await;
}
#[tokio::test]
async fn budget_001_video_src_is_discovered() {
    must_discover("<video src='/hero.jpg'></video>").await;
}
#[tokio::test]
async fn budget_001_video_poster_is_discovered() {
    must_discover("<video poster='/hero.jpg'></video>").await;
}
#[tokio::test]
async fn budget_001_audio_src_is_discovered() {
    must_discover("<audio src='/hero.jpg'></audio>").await;
}
#[tokio::test]
async fn budget_001_media_source_src_is_discovered() {
    must_discover("<video><source src='/hero.jpg'></video>").await;
    must_discover("<audio><source src='/hero.jpg'></audio>").await;
}
#[tokio::test]
async fn budget_001_css_image_set_strings_are_discovered() {
    must_discover("<style>div {background: image-set('/hero.jpg' 1x, '/hero.jpg' 2x)}</style>")
        .await;
}
