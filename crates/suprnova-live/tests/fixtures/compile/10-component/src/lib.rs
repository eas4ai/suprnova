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
define_component!(Component002, "compile.c002", "compile/c002.html");
define_component!(Component003, "compile.c003", "compile/c003.html");
define_component!(Component004, "compile.c004", "compile/c004.html");
define_component!(Component005, "compile.c005", "compile/c005.html");
define_component!(Component006, "compile.c006", "compile/c006.html");
define_component!(Component007, "compile.c007", "compile/c007.html");
define_component!(Component008, "compile.c008", "compile/c008.html");
define_component!(Component009, "compile.c009", "compile/c009.html");
define_component!(Component010, "compile.c010", "compile/c010.html");
