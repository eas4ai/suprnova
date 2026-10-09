#![allow(dead_code)]

//! IDENTITY-039: a field the view sees may be named `component`. The view
//! reads the field under that name, and reaches the component itself as
//! `component_`, the first name of `component`, `component_`, ... that no
//! visible field takes.

use suprnova::live::{LiveComponent, live};

#[derive(LiveComponent)]
#[live(name = "named.component", view = "live/named_component.html")]
pub struct Named {
    component: String,
    count: u64,
}

impl Named {
    pub fn label(&self) -> String {
        format!("{} ({})", self.component, self.count)
    }
}

#[live]
impl Named {
    #[action]
    pub fn rename(&mut self, component: String) {
        self.component = component;
    }
}

fn main() {
    let descriptor = <Named as ::suprnova::live::__private::metadata::LiveComponentContract>::descriptor()
        .expect("generated metadata must be valid");
    let metadata = descriptor.metadata();
    assert_eq!(metadata.identity().as_str(), "named.component");
}
