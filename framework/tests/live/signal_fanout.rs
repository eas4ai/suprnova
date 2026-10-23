use std::sync::Arc;

use suprnova::Router;
use suprnova::broadcasting::{BroadcastHub, InMemoryBroadcastHub};
use suprnova::container::testing::TestContainer;
use suprnova::live::testing::prepare_live_router_for_test;
use suprnova::live::{LiveRegistry, LiveTopics};

#[tokio::test]
#[serial_test::serial]
async fn signal_005_two_runtimes_share_the_hubs_topic_cursor() {
    let _container = TestContainer::fake();
    crate::live_async_support::ensure_crypt();
    let hub = Arc::new(InMemoryBroadcastHub::new());
    TestContainer::bind::<dyn BroadcastHub>(hub);
    TestContainer::singleton(
        LiveRegistry::builder()
            .public_topic("signal.fanout")
            .unwrap()
            .build(),
    );

    let first = prepare_live_router_for_test(&Router::new()).unwrap();
    let second = prepare_live_router_for_test(&Router::new()).unwrap();
    let first = LiveTopics::from_runtime(&first);
    let second = LiveTopics::from_runtime(&second);

    assert_eq!(first.cursor("signal.fanout").await.unwrap(), 0);
    assert_eq!(second.cursor("signal.fanout").await.unwrap(), 0);

    first.changed("signal.fanout", &["posts"]).await.unwrap();

    assert_eq!(first.cursor("signal.fanout").await.unwrap(), 1);
    assert_eq!(second.cursor("signal.fanout").await.unwrap(), 1);
}
