//! The `{stem}` integration tests.

use suprnova::testing::TestClient;
use suprnova::{MiddlewareRegistry, Request, Response, Router};

async fn home(_req: Request) -> Response {
    suprnova::http::text("Hello")
}

/// `TestClient` sends each request through the framework's own pipeline,
/// with no port. Build the router and middleware under test here.
#[tokio::test]
async fn example() {
    let client = TestClient::new(Router::new().get("/", home), MiddlewareRegistry::new());

    client.get("/").send().await.assert_ok().assert_see("Hello");
}
