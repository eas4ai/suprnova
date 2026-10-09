use http_body_util::BodyExt;
use suprnova::{HttpResponse, markdown};

#[tokio::test]
async fn markdown_constructor_preserves_content_and_charset_on_the_wire() {
    for content in ["# Hi", "", "# Cafe\n\n**café** <tag>\r\n"] {
        let response = HttpResponse::markdown(content).into_hyper();
        assert_eq!(response.status(), 200);
        assert_eq!(
            response.headers()["content-type"],
            "text/markdown; charset=utf-8"
        );
        assert_eq!(
            response
                .into_body()
                .collect()
                .await
                .expect("body")
                .to_bytes(),
            content
        );
    }
}

#[tokio::test]
async fn markdown_helper_builds_the_same_wire_response() {
    let response = markdown("# Hi")
        .unwrap_or_else(|_| panic!("markdown response"))
        .into_hyper();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers()["content-type"],
        "text/markdown; charset=utf-8"
    );
    assert_eq!(
        response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes(),
        "# Hi"
    );
}

#[tokio::test]
async fn markdown_response_still_rejects_invalid_header_values() {
    let response = HttpResponse::markdown("# Hi")
        .header("X-Injected", "one\r\nX-Other: two")
        .into_hyper();
    assert!(response.headers().get("x-injected").is_none());
    assert!(response.headers().get("x-other").is_none());
    assert_eq!(
        response.headers()["content-type"],
        "text/markdown; charset=utf-8"
    );
    assert_eq!(
        response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes(),
        "# Hi"
    );
}
