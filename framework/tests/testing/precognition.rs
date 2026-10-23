//! PAR-087 and PAR-088: test helpers and the manual's Precognition contract.

use std::path::Path;

use bytes::Bytes;
use suprnova::testing::{TestClient, TestResponse};
use suprnova::{MiddlewareRegistry, Request, Router};

#[tokio::test]
async fn with_precognition_sends_the_header() {
    let router = Router::new().post("/inspect", |req: Request| async move {
        suprnova::http::text(req.header("Precognition").unwrap_or("missing").to_string())
    });
    let client = TestClient::new(router, MiddlewareRegistry::new());

    let response = client.post("/inspect").with_precognition().send().await;
    response.assert_ok();
    assert_eq!(response.body_text(), "true");
}

#[tokio::test]
async fn with_precognition_replaces_an_existing_header_without_case() {
    let router = Router::new().post("/inspect", |req: Request| async move {
        suprnova::http::text(req.header("Precognition").unwrap_or("missing").to_string())
    });
    let client = TestClient::new(router, MiddlewareRegistry::new());

    let response = client
        .post("/inspect")
        .header("pReCoGnItIoN", "false")
        .with_precognition()
        .send()
        .await;
    response.assert_ok();
    assert_eq!(response.body_text(), "true");
}

#[test]
fn successful_precognition_assertion_returns_the_response_for_chaining() {
    let response = TestResponse::new(
        204,
        [("pReCoGnItIoN-SuCcEsS".to_string(), "true".to_string())],
        Bytes::new(),
    );
    let asserted = response.assert_successful_precognition();

    assert!(std::ptr::eq(asserted, &response));
    asserted.assert_status(204);
}

#[test]
#[should_panic(expected = "assert_successful_precognition")]
fn successful_precognition_assertion_panics_without_the_success_header() {
    TestResponse::new(204, [], Bytes::new()).assert_successful_precognition();
}

#[test]
#[should_panic(expected = "assert_successful_precognition")]
fn successful_precognition_assertion_panics_with_a_false_success_header() {
    TestResponse::new(
        204,
        [("Precognition-Success".to_string(), "false".to_string())],
        Bytes::new(),
    )
    .assert_successful_precognition();
}

#[test]
#[should_panic(expected = "assert_successful_precognition")]
fn successful_precognition_assertion_panics_on_200_even_with_the_success_header() {
    TestResponse::new(
        200,
        [("Precognition-Success".to_string(), "true".to_string())],
        Bytes::new(),
    )
    .assert_successful_precognition();
}

#[test]
fn manual_links_the_chapter_and_covers_each_precognition_topic_in_order() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let contents = std::fs::read_to_string(root.join("manual/documentation.md"))
        .expect("read the manual contents");
    assert!(contents.contains("[Precognition](precognition.md)"));

    let chapter = std::fs::read_to_string(root.join("manual/precognition.md"))
        .expect("read the Precognition chapter");
    let mut lines = chapter.lines();
    for heading in [
        "## Opt in to Precognition",
        "## Live validation with Inertia 3.8",
        "## Client configuration",
        "## Validating arrays and wildcards",
        "## Customizing rules for live validation",
        "## File uploads and query data",
        "## Managing side effects",
        "## The 422 body",
        "## Testing",
        "### Why Suprnova diverges",
        "## Next",
    ] {
        assert!(
            lines.any(|line| line == heading),
            "Precognition chapter lacks {heading} in the required order"
        );
    }
}

#[test]
fn manual_no_longer_claims_unlisted_parse_failures_block_precognition() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let old_claim = "A Precognition request that asks about a field which parses, while\n  \
                     another field doesn't, gets those other fields' errors rather than a\n  \
                     `204`: the rules for the field it asked about haven't run.";
    for path in ["manual/validation.md", "manual/requests.md"] {
        let text = std::fs::read_to_string(root.join(path)).expect("read a validation chapter");
        assert!(
            !text.contains(old_claim),
            "{path} still carries the old claim"
        );
        assert!(text.contains("[Precognition](precognition.md)"));
    }
}
