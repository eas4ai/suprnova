//! Public-facade integration coverage for Live component macros.

use suprnova::live::{LiveComponent, LiveRegistry, live};
use suprnova::{LiveComponent as RootLiveComponent, live as root_live};

#[derive(LiveComponent)]
#[live(name = "framework.counter", view = "live/framework/counter.html")]
pub struct Counter {
    count: u64,
}

#[live]
impl Counter {
    #[action]
    pub async fn increment(&mut self) {
        self.count += 1;
    }
}

#[root_live(
    name = "framework.root-counter",
    view = "live/framework/root-counter.html"
)]
#[derive(RootLiveComponent)]
pub struct RootCounter {
    count: u64,
}

#[root_live]
impl RootCounter {
    #[action]
    pub async fn increment(&mut self) {
        self.count += 1;
    }
}

#[test]
fn live_macros_expand_only_through_the_public_framework_facade() {
    let registry = LiveRegistry::builder()
        .register::<Counter>()
        .expect("generated component registration must be valid")
        .register::<RootCounter>()
        .expect("crate-root macro re-exports must generate a valid registration")
        .build();

    assert_eq!(registry.len(), 2);
}
