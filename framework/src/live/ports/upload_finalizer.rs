//! Fail-closed durable finalizer until an application explicitly owns persistence.

use suprnova_live::upload::{
    DurableUpload, FailedFinalize, FinalizeRequest, PreparedFinalize, UploadError, UploadErrorKind,
    UploadFinalizer, UploadFuture,
};

pub(crate) struct SuprnovaUploadFinalizer;

impl UploadFinalizer for SuprnovaUploadFinalizer {
    fn prepare<'a>(
        &'a self,
        _request: FinalizeRequest<'a>,
    ) -> UploadFuture<'a, Result<PreparedFinalize, UploadError>> {
        unavailable()
    }

    fn commit<'a>(
        &'a self,
        _prepared: PreparedFinalize,
    ) -> UploadFuture<'a, Result<DurableUpload, UploadError>> {
        unavailable()
    }

    fn compensate<'a>(
        &'a self,
        _failed: FailedFinalize,
    ) -> UploadFuture<'a, Result<(), UploadError>> {
        unavailable()
    }

    fn reconcile<'a>(
        &'a self,
        _request: FinalizeRequest<'a>,
    ) -> UploadFuture<'a, Result<Option<DurableUpload>, UploadError>> {
        unavailable()
    }
}

fn unavailable<'a, T>() -> UploadFuture<'a, Result<T, UploadError>> {
    Box::pin(async { Err(UploadError::new(UploadErrorKind::FinalizationFailed)) })
}
