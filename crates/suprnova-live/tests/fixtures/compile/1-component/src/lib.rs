#![allow(
    dead_code,
    missing_docs,
    reason = "fixed compile-growth fixtures are expanded and checked, not called as an API"
)]
#![forbid(unsafe_code)]

extern crate self as suprnova;

pub mod live {
    pub use suprnova_live::*;
    pub use suprnova_live_macros::{LiveComponent, live};

    #[doc(hidden)]
    pub mod __private {
        pub use suprnova_live::*;
    }
}

use live::{LiveComponent, live};

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
            pub fn submit(&mut self) -> crate::live::action::ActionOutcome {
                crate::live::action::ActionOutcome::NoRender
            }
        }
    };
}

define_component!(Component001, "compile.c001", "compile/c001.html");
