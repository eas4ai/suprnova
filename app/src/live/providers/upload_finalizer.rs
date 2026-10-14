//! The application's upload finalizer: records every committed upload so a
//! later reconciliation (or a test) can find it by its idempotency key.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use suprnova::live::{
    DurableUpload, DurableUploadId, FailedFinalize, FinalizeRequest, FinalizeToken,
    PreparedFinalize, UploadError, UploadFinalizer, UploadFuture,
};

/// Committed uploads retained in memory; the oldest is evicted past this.
const MAX_RETAINED: usize = 1_024;

/// In-process finalizer that keeps a bounded window of committed uploads.
#[derive(Default)]
pub struct AppUploadFinalizer {
    durable: Mutex<Retained>,
}

#[derive(Default)]
struct Retained {
    by_token: HashMap<String, DurableUpload>,
    order: VecDeque<String>,
}

impl AppUploadFinalizer {
    /// The durable ids committed so far, in no particular order.
    pub fn committed(&self) -> Vec<String> {
        self.durable
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .by_token
            .values()
            .map(|durable| durable.id().as_str().to_owned())
            .collect()
    }

    /// The idempotency key is the finalize identity: both share the engine's
    /// 128-byte budget, so a prefix would overflow a protocol-legal key.
    fn token_for(key: &str) -> Result<FinalizeToken, UploadError> {
        FinalizeToken::parse(key)
    }
}

impl UploadFinalizer for AppUploadFinalizer {
    fn prepare<'a>(
        &'a self,
        request: FinalizeRequest<'a>,
    ) -> UploadFuture<'a, Result<PreparedFinalize, UploadError>> {
        Box::pin(async move {
            let token = Self::token_for(request.idempotency_key().as_str())?;
            Ok(PreparedFinalize::new(&request, token))
        })
    }

    fn commit<'a>(
        &'a self,
        prepared: PreparedFinalize,
    ) -> UploadFuture<'a, Result<DurableUpload, UploadError>> {
        Box::pin(async move {
            let key = prepared.token().as_str().to_owned();
            let durable = DurableUpload::new(&prepared, DurableUploadId::parse(&key)?);
            let mut retained = self
                .durable
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if retained
                .by_token
                .insert(key.clone(), durable.clone())
                .is_none()
            {
                retained.order.push_back(key);
                while retained.order.len() > MAX_RETAINED {
                    if let Some(oldest) = retained.order.pop_front() {
                        retained.by_token.remove(&oldest);
                    }
                }
            }
            Ok(durable)
        })
    }

    fn compensate<'a>(
        &'a self,
        failed: FailedFinalize,
    ) -> UploadFuture<'a, Result<(), UploadError>> {
        Box::pin(async move {
            let key = failed.prepared().token().as_str();
            let mut retained = self
                .durable
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            retained.by_token.remove(key);
            retained.order.retain(|retained_key| retained_key != key);
            Ok(())
        })
    }

    fn reconcile<'a>(
        &'a self,
        request: FinalizeRequest<'a>,
    ) -> UploadFuture<'a, Result<Option<DurableUpload>, UploadError>> {
        Box::pin(async move {
            let token = Self::token_for(request.idempotency_key().as_str())?;
            Ok(self
                .durable
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .by_token
                .get(token.as_str())
                .cloned())
        })
    }
}
