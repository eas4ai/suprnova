#![allow(dead_code)]

use suprnova::live::{LiveComponent, live};

#[derive(LiveComponent)]
#[live(name = "counter", view = "live/counter.html")]
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

fn main() {
    let descriptor = <Counter as ::suprnova::live::__private::metadata::LiveComponentContract>::descriptor()
        .expect("generated metadata must be valid");
    assert_eq!(descriptor.metadata().identity().as_str(), "counter");
}
