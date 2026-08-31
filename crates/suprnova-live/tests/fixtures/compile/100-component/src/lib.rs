#![allow(
    dead_code,
    missing_docs,
    reason = "fixed compile-growth fixtures are expanded and checked, not called as an API"
)]
#![forbid(unsafe_code)]

extern crate self as suprnova;

pub mod live {
    pub use suprnova_live::*;
    pub use suprnova_macros::{LiveComponent, live};

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
define_component!(Component002, "compile.c002", "compile/c002.html");
define_component!(Component003, "compile.c003", "compile/c003.html");
define_component!(Component004, "compile.c004", "compile/c004.html");
define_component!(Component005, "compile.c005", "compile/c005.html");
define_component!(Component006, "compile.c006", "compile/c006.html");
define_component!(Component007, "compile.c007", "compile/c007.html");
define_component!(Component008, "compile.c008", "compile/c008.html");
define_component!(Component009, "compile.c009", "compile/c009.html");
define_component!(Component010, "compile.c010", "compile/c010.html");
define_component!(Component011, "compile.c011", "compile/c011.html");
define_component!(Component012, "compile.c012", "compile/c012.html");
define_component!(Component013, "compile.c013", "compile/c013.html");
define_component!(Component014, "compile.c014", "compile/c014.html");
define_component!(Component015, "compile.c015", "compile/c015.html");
define_component!(Component016, "compile.c016", "compile/c016.html");
define_component!(Component017, "compile.c017", "compile/c017.html");
define_component!(Component018, "compile.c018", "compile/c018.html");
define_component!(Component019, "compile.c019", "compile/c019.html");
define_component!(Component020, "compile.c020", "compile/c020.html");
define_component!(Component021, "compile.c021", "compile/c021.html");
define_component!(Component022, "compile.c022", "compile/c022.html");
define_component!(Component023, "compile.c023", "compile/c023.html");
define_component!(Component024, "compile.c024", "compile/c024.html");
define_component!(Component025, "compile.c025", "compile/c025.html");
define_component!(Component026, "compile.c026", "compile/c026.html");
define_component!(Component027, "compile.c027", "compile/c027.html");
define_component!(Component028, "compile.c028", "compile/c028.html");
define_component!(Component029, "compile.c029", "compile/c029.html");
define_component!(Component030, "compile.c030", "compile/c030.html");
define_component!(Component031, "compile.c031", "compile/c031.html");
define_component!(Component032, "compile.c032", "compile/c032.html");
define_component!(Component033, "compile.c033", "compile/c033.html");
define_component!(Component034, "compile.c034", "compile/c034.html");
define_component!(Component035, "compile.c035", "compile/c035.html");
define_component!(Component036, "compile.c036", "compile/c036.html");
define_component!(Component037, "compile.c037", "compile/c037.html");
define_component!(Component038, "compile.c038", "compile/c038.html");
define_component!(Component039, "compile.c039", "compile/c039.html");
define_component!(Component040, "compile.c040", "compile/c040.html");
define_component!(Component041, "compile.c041", "compile/c041.html");
define_component!(Component042, "compile.c042", "compile/c042.html");
define_component!(Component043, "compile.c043", "compile/c043.html");
define_component!(Component044, "compile.c044", "compile/c044.html");
define_component!(Component045, "compile.c045", "compile/c045.html");
define_component!(Component046, "compile.c046", "compile/c046.html");
define_component!(Component047, "compile.c047", "compile/c047.html");
define_component!(Component048, "compile.c048", "compile/c048.html");
define_component!(Component049, "compile.c049", "compile/c049.html");
define_component!(Component050, "compile.c050", "compile/c050.html");
define_component!(Component051, "compile.c051", "compile/c051.html");
define_component!(Component052, "compile.c052", "compile/c052.html");
define_component!(Component053, "compile.c053", "compile/c053.html");
define_component!(Component054, "compile.c054", "compile/c054.html");
define_component!(Component055, "compile.c055", "compile/c055.html");
define_component!(Component056, "compile.c056", "compile/c056.html");
define_component!(Component057, "compile.c057", "compile/c057.html");
define_component!(Component058, "compile.c058", "compile/c058.html");
define_component!(Component059, "compile.c059", "compile/c059.html");
define_component!(Component060, "compile.c060", "compile/c060.html");
define_component!(Component061, "compile.c061", "compile/c061.html");
define_component!(Component062, "compile.c062", "compile/c062.html");
define_component!(Component063, "compile.c063", "compile/c063.html");
define_component!(Component064, "compile.c064", "compile/c064.html");
define_component!(Component065, "compile.c065", "compile/c065.html");
define_component!(Component066, "compile.c066", "compile/c066.html");
define_component!(Component067, "compile.c067", "compile/c067.html");
define_component!(Component068, "compile.c068", "compile/c068.html");
define_component!(Component069, "compile.c069", "compile/c069.html");
define_component!(Component070, "compile.c070", "compile/c070.html");
define_component!(Component071, "compile.c071", "compile/c071.html");
define_component!(Component072, "compile.c072", "compile/c072.html");
define_component!(Component073, "compile.c073", "compile/c073.html");
define_component!(Component074, "compile.c074", "compile/c074.html");
define_component!(Component075, "compile.c075", "compile/c075.html");
define_component!(Component076, "compile.c076", "compile/c076.html");
define_component!(Component077, "compile.c077", "compile/c077.html");
define_component!(Component078, "compile.c078", "compile/c078.html");
define_component!(Component079, "compile.c079", "compile/c079.html");
define_component!(Component080, "compile.c080", "compile/c080.html");
define_component!(Component081, "compile.c081", "compile/c081.html");
define_component!(Component082, "compile.c082", "compile/c082.html");
define_component!(Component083, "compile.c083", "compile/c083.html");
define_component!(Component084, "compile.c084", "compile/c084.html");
define_component!(Component085, "compile.c085", "compile/c085.html");
define_component!(Component086, "compile.c086", "compile/c086.html");
define_component!(Component087, "compile.c087", "compile/c087.html");
define_component!(Component088, "compile.c088", "compile/c088.html");
define_component!(Component089, "compile.c089", "compile/c089.html");
define_component!(Component090, "compile.c090", "compile/c090.html");
define_component!(Component091, "compile.c091", "compile/c091.html");
define_component!(Component092, "compile.c092", "compile/c092.html");
define_component!(Component093, "compile.c093", "compile/c093.html");
define_component!(Component094, "compile.c094", "compile/c094.html");
define_component!(Component095, "compile.c095", "compile/c095.html");
define_component!(Component096, "compile.c096", "compile/c096.html");
define_component!(Component097, "compile.c097", "compile/c097.html");
define_component!(Component098, "compile.c098", "compile/c098.html");
define_component!(Component099, "compile.c099", "compile/c099.html");
define_component!(Component100, "compile.c100", "compile/c100.html");
