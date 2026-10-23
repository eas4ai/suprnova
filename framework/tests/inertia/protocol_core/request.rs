//! PAR-046: a request is an Inertia visit when `X-Inertia` holds any value
//! PHP's boolean cast reads as true.

use serde_json::Value;
use suprnova::{InertiaRequestExt, MiddlewareRegistry};

use super::support::{MockReq, page_router, spawn_server};
use crate::http_wire::request;

#[tokio::test]
async fn inp_x_inertia_counts_for_any_value_php_reads_as_true() {
    // Laravel's `Request::inertia()` is `(bool) $this->header('X-Inertia')`:
    // every value but "" and "0" is an Inertia visit. A client sending
    // `1` got JSON from Laravel and the HTML document from Suprnova.
    let addr = spawn_server(page_router(), MiddlewareRegistry::new(), 8).await;
    for value in ["true", "1", "false", "yes"] {
        let (status, headers, body) = request(addr, "GET", "/page", &[("X-Inertia", value)]).await;
        assert_eq!(status, 200, "X-Inertia: {value}");
        assert_eq!(
            headers.get("x-inertia").map(String::as_str),
            Some("true"),
            "X-Inertia: {value} must get the JSON page object with X-Inertia: true; got {body}"
        );
        let page: Value = serde_json::from_str(&body).expect("a JSON page object");
        assert_eq!(page["component"], "Page");
    }
    for value in ["0", ""] {
        let (status, headers, body) = request(addr, "GET", "/page", &[("X-Inertia", value)]).await;
        assert_eq!(status, 200, "X-Inertia: {value:?}");
        assert!(
            !headers.contains_key("x-inertia"),
            "X-Inertia: {value:?} is false in PHP and must get the HTML document"
        );
        assert!(body.starts_with("<!DOCTYPE html>"), "got {body}");
    }
}

#[test]
fn inp_the_request_ext_default_reads_x_inertia_as_php_casts_it() {
    // The trait's provided `is_inertia` is what a hand-built request
    // (an adapter, a test mock) gets, so it follows the same rule.
    for (value, expected) in [("1", true), ("true", true), ("0", false), ("", false)] {
        let req = MockReq::new("/").header("X-Inertia", value);
        assert_eq!(req.is_inertia(), expected, "X-Inertia: {value:?}");
    }
    assert!(
        !MockReq::new("/").is_inertia(),
        "no header, no Inertia visit"
    );
}
