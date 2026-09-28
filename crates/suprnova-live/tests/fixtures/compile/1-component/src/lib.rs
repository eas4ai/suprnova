#![allow(
    dead_code,
    missing_docs,
    reason = "fixed compile-growth fixtures are expanded and checked, not called as an API"
)]
#![forbid(unsafe_code)]

use suprnova::live::{LiveComponent, live};

macro_rules! define_component {
    ($component:ident, $name:literal, $view:literal) => {
        #[derive(LiveComponent)]
        #[live(name = $name, view = $view)]
        pub struct $component {
            value: u64,
            #[model]
            query: String,
        }

        #[live]
        impl $component {
            #[action]
            pub fn submit(&mut self) -> suprnova::live::action::ActionOutcome {
                suprnova::live::action::ActionOutcome::NoRender
            }
        }
    };
}

define_component!(Component001, "compile.c001", "compile/c001.html");
