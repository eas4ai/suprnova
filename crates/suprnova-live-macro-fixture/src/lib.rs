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
#[allow(dead_code, reason = "fixture fields exist to exercise macro metadata")]
pub struct ExpandFixture {
    count: u64,
    #[model(debounce = 250)]
    #[url(key = "q", mode = "reflect", omit_default)]
    query: String,
    #[session]
    locale: String,
}

#[live]
impl ExpandFixture {
    /// Constructs the fixture from one explicit typed mount parameter.
    #[mount]
    pub fn mount(query: String) -> Self {
        Self {
            count: 0,
            query,
            locale: "en".to_owned(),
        }
    }

    /// Increments the fixture counter through a registered action.
    #[action]
    pub async fn increment(&mut self) {
        self.count += 1;
    }

    /// Applies a separately authorized parent parameter update.
    #[params_changed]
    pub async fn params_changed(&mut self) {}

    /// Completes deferred work through the ordinary lifecycle.
    #[lazy_complete]
    pub async fn lazy_complete(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::ExpandFixture;
    use crate::live::metadata::LiveComponentContract as _;
    use crate::live::snapshot::state::{FieldCategory, StateCodec};

    #[test]
    fn generated_descriptor_uses_stable_default_metadata() {
        let _attach_runtime_hooks: fn(
            crate::live::component::ComponentHooks,
        ) -> Result<
            crate::live::registry::ComponentDescriptor,
            crate::live::metadata::MetadataError,
        > = ExpandFixture::descriptor_with_hooks;
        let descriptor = ExpandFixture::descriptor().expect("generated descriptor");
        let metadata = descriptor.metadata();

        assert_eq!(metadata.identity().as_str(), "fixture.counter");
        assert_eq!(metadata.view().as_str(), "live/fixture/counter.html");
        assert_eq!(metadata.fields()[0].name().as_str(), "count");
        assert_eq!(metadata.fields()[0].category(), FieldCategory::State);
        assert_eq!(metadata.fields()[0].codec(), StateCodec::U64Decimal);
        let query = metadata
            .fields()
            .iter()
            .find(|field| field.name().as_str() == "query")
            .expect("query metadata");
        assert_eq!(
            query
                .binding_timing()
                .and_then(|timing| timing.debounce_millis()),
            Some(250)
        );
        assert_eq!(query.url_binding().expect("URL metadata").query_key(), "q");
        let locale = metadata
            .fields()
            .iter()
            .find(|field| field.name().as_str() == "locale")
            .expect("session metadata");
        assert_eq!(
            locale.session_codec(),
            Some(&crate::live::state::ModelCodec::String)
        );
        assert_eq!(metadata.actions()[0].name().as_str(), "increment");
        assert_eq!(metadata.actions()[0].version(), 1);
        assert_eq!(descriptor.parameter_schema().len(), 1);
        assert!(descriptor.supports_params_changed());
        assert!(descriptor.supports_lazy_complete());
    }
}
