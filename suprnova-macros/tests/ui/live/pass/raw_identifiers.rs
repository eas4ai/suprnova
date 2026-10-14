#![allow(dead_code)]

//! A raw identifier names its field or argument without the `r#`, so the
//! component registers instead of panicking on an invalid identity.

use suprnova::live::{LiveComponent, live};

#[derive(LiveComponent)]
#[live(name = "names.raw", view = "live/counter.html")]
pub struct Raw {
    count: u64,
    r#type: String,
}

#[live]
impl Raw {
    #[action]
    pub fn retype(&mut self, r#type: String) {
        self.count += 1;
        self.r#type = r#type;
    }
}

fn main() {
    let descriptor = <Raw as ::suprnova::live::__private::metadata::LiveComponentContract>::descriptor()
        .expect("generated metadata must be valid");
    let metadata = descriptor.metadata();
    assert!(metadata.fields().iter().any(|field| field.name().as_str() == "type"));
    assert_eq!(metadata.actions()[0].name().as_str(), "retype");
}
