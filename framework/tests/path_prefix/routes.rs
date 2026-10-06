//! PFX-003: `route()` and its siblings return the root followed by the
//! route's path, and signed URLs are signed and verified over both.

use std::collections::HashMap;

use suprnova::{HttpResponse, MiddlewareRegistry, Request, Router, url};

use crate::support::{self, PREFIX};

fn router() -> Router {
    let router: Router = Router::new()
        .get("/invoices/{id}", |request: Request| async move {
            let verified = url::has_valid_signature(&request)
                .map_err(|error| HttpResponse::text(error.to_string()).status(500))?;
            Ok(HttpResponse::text(if verified {
                "verified"
            } else {
                "refused"
            }))
        })
        .name("invoices.show");
    router
        .get("/links", |_request: Request| async {
            let params = HashMap::from([("id".to_owned(), "7".to_owned())]);
            let urls = [
                suprnova::route("invoices.show", &[("id", "7")]).expect("route"),
                suprnova::routing::try_route("invoices.show", &[("id", "7")]).expect("try_route"),
                suprnova::routing::route_with_params("invoices.show", &params)
                    .expect("route_with_params"),
                suprnova::routing::try_route_with_params("invoices.show", &params)
                    .expect("try_route_with_params"),
            ];
            Ok(HttpResponse::text(urls.join("\n")))
        })
        .get("/sign", |_request: Request| async {
            let signed = url::signed_route("invoices.show", &[("id", "7")])
                .map_err(|error| HttpResponse::text(error.to_string()).status(500))?;
            Ok(HttpResponse::text(signed))
        })
        .into()
}

#[tokio::test]
async fn pfx_003_route_returns_the_root_followed_by_the_path() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_003_route_returns_the_root_followed_by_the_path",
    )
    .await
    {
        return;
    }
    support::install("http://localhost");
    let address = support::serve(router(), MiddlewareRegistry::new()).await;

    let behind = support::get_prefixed(address, "/links").await;
    assert_eq!(
        behind.body.lines().collect::<Vec<_>>(),
        ["/billing/invoices/7"; 4]
    );

    let at_host_root = support::get(address, "/links", &[]).await;
    assert_eq!(
        at_host_root.body.lines().collect::<Vec<_>>(),
        ["/invoices/7"; 4]
    );
}

/// Strip the root the way the proxy does before it forwards the request.
fn forwarded_path(signed: &str) -> &str {
    signed
        .strip_prefix(PREFIX)
        .unwrap_or_else(|| panic!("`{signed}` carries the root"))
}

#[tokio::test]
async fn pfx_003_a_url_signed_behind_a_prefix_verifies_behind_it_only() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_003_a_url_signed_behind_a_prefix_verifies_behind_it_only",
    )
    .await
    {
        return;
    }
    support::ensure_crypt();
    support::install("http://localhost");
    let address = support::serve(router(), MiddlewareRegistry::new()).await;

    let signed = support::get_prefixed(address, "/sign").await.body;
    assert!(signed.starts_with("/billing/invoices/7?"), "{signed}");
    let path = forwarded_path(&signed);

    let behind = support::get_prefixed(address, path).await;
    assert_eq!(behind.body, "verified");

    let other_root = support::get(address, path, &[("x-forwarded-prefix", "/other")]).await;
    assert_eq!(
        other_root.body, "refused",
        "a URL signed under one root verified under another"
    );

    let host_root = support::get(address, path, &[]).await;
    assert_eq!(
        host_root.body, "refused",
        "a URL signed under a root verified at the host root"
    );
}

#[tokio::test]
async fn pfx_003_a_url_signed_at_the_host_root_is_unchanged() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_003_a_url_signed_at_the_host_root_is_unchanged",
    )
    .await
    {
        return;
    }
    support::ensure_crypt();
    support::install("http://localhost");
    let address = support::serve(router(), MiddlewareRegistry::new()).await;
    let signed = support::get(address, "/sign", &[]).await.body;
    assert!(signed.starts_with("/invoices/7?signature="), "{signed}");
    assert_eq!(support::get(address, &signed, &[]).await.body, "verified");
}
