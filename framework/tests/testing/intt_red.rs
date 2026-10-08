//! The violating example `par-inertia-testing` is bound on: the test client
//! PAR-063 names does not exist yet, so this binary does not compile and
//! the check records a fail. Removed once the mechanism is bound.

use suprnova::testing::TestClient;

#[tokio::test]
async fn intt_red_the_test_client_exists() {
    let client = TestClient::new(suprnova::Router::new(), suprnova::MiddlewareRegistry::new());
    let _ = client;
}
