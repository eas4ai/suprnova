//! PAR-056: the page `url` carries the query as Laravel's `fullUrl()`
//! normalises it through Symfony.

use suprnova::{InertiaResponse, InertiaVersionMiddleware, MiddlewareRegistry};

use super::support::{MockReq, page_of, page_router, spawn_server};
use crate::http_wire::request;

#[tokio::test]
async fn inp_the_page_url_sorts_and_reencodes_the_query() {
    for (query, expected) in [
        ("b=2&a=1%20x", "/s?a=1%20x&b=2"),
        ("a=%2Fx", "/s?a=%2Fx"),
        ("q=a+b", "/s?q=a%20b"),
        ("z=1&a[]=2&a[]=3", "/s?a%5B0%5D=2&a%5B1%5D=3&z=1"),
        ("flag&x=1", "/s?flag=&x=1"),
        ("", "/s"),
    ] {
        let resp = InertiaResponse::new("S")
            .resolve(&MockReq::new("/s").query(query).inertia())
            .await
            .unwrap();
        assert_eq!(page_of(resp).await["url"], expected, "query {query:?}");
    }
}

#[tokio::test]
async fn inp_a_real_request_and_its_version_bounce_carry_the_normalised_query() {
    let registry = MiddlewareRegistry::new().append(InertiaVersionMiddleware::new("v2"));
    let addr = spawn_server(page_router(), registry, 2).await;

    let (status, _, body) = request(
        addr,
        "GET",
        "/page?b=2&a=1%20x",
        &[("X-Inertia", "true"), ("X-Inertia-Version", "v2")],
    )
    .await;
    assert_eq!(status, 200);
    let page: serde_json::Value = serde_json::from_str(&body).expect("a JSON page object");
    assert_eq!(page["url"], "/page?a=1%20x&b=2");

    let (status, headers, _) = request(
        addr,
        "GET",
        "/page?b=2&a=1%20x",
        &[("X-Inertia", "true"), ("X-Inertia-Version", "v1")],
    )
    .await;
    assert_eq!(status, 409);
    assert_eq!(
        headers.get("x-inertia-location").map(String::as_str),
        Some("http://localhost/page?a=1%20x&b=2"),
        "the bounce names the page URL made absolute"
    );
}
