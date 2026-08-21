//! Development-only fixture for the final `::suprnova::live` facade shape.

#![forbid(unsafe_code)]
#![deny(missing_docs, rustdoc::broken_intra_doc_links)]

/// Final Live facade shape used only by standalone macro compile tests.
pub mod live {
    pub use suprnova_live::*;

    /// Hidden generated-code support surface used only by macro compile tests.
    #[doc(hidden)]
    pub mod __private {
        pub use suprnova_live::*;
    }
}
