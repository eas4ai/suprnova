//! Application-owned upload capabilities installed before Live runtime assembly.

use std::fmt;
use std::sync::Arc;

use suprnova_live::upload::{
    DirectUploadProvider, UploadApplicationValidator, UploadFinalizer, UploadScanner,
};

/// Optional application-owned upload capabilities resolved during Live boot.
///
/// Omitted capabilities remain explicitly fail-closed; registering this value
/// never changes upload identity, lifecycle, or authorization semantics.
#[derive(Clone, Default)]
pub struct LiveUploadHost {
    finalizer: Option<Arc<dyn UploadFinalizer>>,
    direct_provider: Option<Arc<dyn DirectUploadProvider>>,
    scanner: Option<Arc<dyn UploadScanner>>,
    application_validator: Option<Arc<dyn UploadApplicationValidator>>,
}

impl LiveUploadHost {
    /// Starts an empty fail-closed host capability set.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            finalizer: None,
            direct_provider: None,
            scanner: None,
            application_validator: None,
        }
    }

    /// Installs the application-owned durable finalization capability.
    #[must_use]
    pub fn with_finalizer(mut self, finalizer: Arc<dyn UploadFinalizer>) -> Self {
        self.finalizer = Some(finalizer);
        self
    }

    /// Installs an optional constrained direct-to-storage provider.
    #[must_use]
    pub fn with_direct_provider(mut self, provider: Arc<dyn DirectUploadProvider>) -> Self {
        self.direct_provider = Some(provider);
        self
    }

    /// Installs an application-owned content scanner.
    #[must_use]
    pub fn with_scanner(mut self, scanner: Arc<dyn UploadScanner>) -> Self {
        self.scanner = Some(scanner);
        self
    }

    /// Installs an application-owned authoritative content validator.
    #[must_use]
    pub fn with_application_validator(
        mut self,
        validator: Arc<dyn UploadApplicationValidator>,
    ) -> Self {
        self.application_validator = Some(validator);
        self
    }

    pub(crate) fn finalizer(&self) -> Option<Arc<dyn UploadFinalizer>> {
        self.finalizer.clone()
    }

    pub(crate) fn direct_provider(&self) -> Option<Arc<dyn DirectUploadProvider>> {
        self.direct_provider.clone()
    }

    pub(crate) fn scanner(&self) -> Option<Arc<dyn UploadScanner>> {
        self.scanner.clone()
    }

    pub(crate) fn application_validator(&self) -> Option<Arc<dyn UploadApplicationValidator>> {
        self.application_validator.clone()
    }
}

impl fmt::Debug for LiveUploadHost {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LiveUploadHost")
            .field("finalizer", &self.finalizer.is_some())
            .field("direct_provider", &self.direct_provider.is_some())
            .field("scanner", &self.scanner.is_some())
            .field(
                "application_validator",
                &self.application_validator.is_some(),
            )
            .finish()
    }
}
