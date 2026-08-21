//! Development-only fixture for the final `::suprnova::live` facade shape.

#![forbid(unsafe_code)]
#![deny(missing_docs, rustdoc::broken_intra_doc_links)]

extern crate self as suprnova;

/// Final Live facade shape used only by standalone macro compile tests.
pub mod live {
    pub use suprnova_live::*;
    pub use suprnova_live_macros::{LiveComponent, live};

    /// Hidden generated-code support surface used only by macro compile tests.
    #[doc(hidden)]
    pub mod __private {
        pub use suprnova_live::*;
    }
}

use live::{LiveComponent, live};

/// One-component fixture used to inspect final-facade macro expansion.
#[derive(LiveComponent)]
#[live(name = "fixture.counter", view = "live/fixture/counter.html")]
pub struct ExpandFixture {
    count: u64,
}

#[live]
impl ExpandFixture {
    /// Increments the fixture counter through a registered action.
    #[action]
    pub async fn increment(&mut self) {
        self.count += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::ExpandFixture;
    use crate::live::metadata::LiveComponentContract as _;
    use crate::live::snapshot::state::{FieldCategory, StateCodec};

    #[test]
    fn generated_descriptor_uses_stable_default_metadata() {
        let descriptor = ExpandFixture::descriptor().expect("generated descriptor");
        let metadata = descriptor.metadata();

        assert_eq!(metadata.identity().as_str(), "fixture.counter");
        assert_eq!(metadata.view().as_str(), "live/fixture/counter.html");
        assert_eq!(metadata.fields()[0].name().as_str(), "count");
        assert_eq!(metadata.fields()[0].category(), FieldCategory::State);
        assert_eq!(metadata.fields()[0].codec(), StateCodec::U64Decimal);
        assert_eq!(metadata.actions()[0].name().as_str(), "increment");
        assert_eq!(metadata.actions()[0].version(), 1);
    }
}
