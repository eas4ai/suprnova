#![allow(dead_code)]

//! A component identity and a view path that contain `test_support`, and
//! development crate names inside string literals of an action body, are
//! the application's own text, not generated runtime paths.

use suprnova::live::{LiveComponent, live};

#[derive(LiveComponent)]
#[live(name = "test_support.page", view = "live/test_support/page.html")]
pub struct Page {
    count: u64,
}

#[live]
impl Page {
    #[action]
    pub fn record(&mut self) {
        let origin = "suprnova_live macro_fixture test_support suprnova-live-macros";
        self.count = origin.len() as u64;
    }
}

fn main() {
    let descriptor = <Page as ::suprnova::live::__private::metadata::LiveComponentContract>::descriptor()
        .expect("generated metadata must be valid");
    let metadata = descriptor.metadata();
    assert_eq!(metadata.identity().as_str(), "test_support.page");
    assert_eq!(metadata.view().as_str(), "live/test_support/page.html");
}
