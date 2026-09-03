//! The application's upload finalizer: records every committed upload so a
//! later reconciliation (or a test) can find it by its idempotency key.

use std::collections::HashMap;
use std::sync::Mutex;

use suprnova::live::{
    DurableUpload, DurableUploadId, FailedFinalize, FinalizeRequest, FinalizeToken,
    PreparedFinalize, UploadError, UploadFinalizer, UploadFuture,
};

/// In-process finalizer that keeps committed uploads in memory.
#[derive(Default)]
pub struct AppUploadFinalizer {
    durable: Mutex<HashMap<String, DurableUpload>>,
}

impl AppUploadFinalizer {
    /// The durable ids committed so far, in no particular order.
    pub fn committed(&self) -> Vec<String> {
        self.durable
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .map(|durable| durable.id().as_str().to_owned())
            .collect()
    }

    fn token_for(key: &str) -> String {
        format!("app-finalize:{key}")
    }
}

impl UploadFinalizer for AppUploadFinalizer {
    fn prepare<'a>(
        &'a self,
        request: FinalizeRequest<'a>,
    ) -> UploadFuture<'a, Result<PreparedFinalize, UploadError>> {
        Box::pin(async move {
            let token = FinalizeToken::parse(&Self::token_for(request.idempotency_key().as_str()))?;
            Ok(PreparedFinalize::new(&request, token))
        })
    }

    fn commit<'a>(
        &'a self,
        prepared: PreparedFinalize,
    ) -> UploadFuture<'a, Result<DurableUpload, UploadError>> {
        Box::pin(async move {
            let key = prepared.token().as_str().to_owned();
            let durable = DurableUpload::new(
                &prepared,
                DurableUploadId::parse(&format!("durable:{key}"))?,
            );
            self.durable
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(key, durable.clone());
            Ok(durable)
        })
    }

    fn compensate<'a>(
        &'a self,
        failed: FailedFinalize,
    ) -> UploadFuture<'a, Result<(), UploadError>> {
        Box::pin(async move {
            self.durable
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(failed.prepared().token().as_str());
            Ok(())
        })
    }

    fn reconcile<'a>(
        &'a self,
        request: FinalizeRequest<'a>,
    ) -> UploadFuture<'a, Result<Option<DurableUpload>, UploadError>> {
        Box::pin(async move {
            Ok(self
                .durable
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&Self::token_for(request.idempotency_key().as_str()))
                .cloned())
        })
    }
}
