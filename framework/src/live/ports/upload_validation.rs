//! Separate scanner, application-validation, and immutable evidence adapters.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

use suprnova_live::upload::{
    ApplicationValidationDecision, ApplicationValidationInput, ScanDisposition, ScanInput,
    UploadApplicationValidator, UploadError, UploadFuture, UploadHandle, UploadScanner,
    UploadValidationStore, ValidatedUpload, ValidationStoreDisposition,
};

pub(crate) struct SuprnovaUploadScanner;

impl UploadScanner for SuprnovaUploadScanner {
    fn scan<'a>(
        &'a self,
        _input: ScanInput<'a>,
    ) -> UploadFuture<'a, Result<ScanDisposition, UploadError>> {
        Box::pin(async { Ok(ScanDisposition::Unavailable) })
    }
}

pub(crate) struct SuprnovaUploadApplicationValidator;

impl UploadApplicationValidator for SuprnovaUploadApplicationValidator {
    fn validate<'a>(
        &'a self,
        _input: ApplicationValidationInput<'a>,
    ) -> UploadFuture<'a, Result<ApplicationValidationDecision, UploadError>> {
        Box::pin(async { Ok(ApplicationValidationDecision::Allow) })
    }
}

#[derive(Default)]
pub(crate) struct SuprnovaUploadValidationStore {
    evidence: Mutex<HashMap<UploadHandle, ValidatedUpload>>,
}

impl UploadValidationStore for SuprnovaUploadValidationStore {
    fn put<'a>(
        &'a self,
        evidence: ValidatedUpload,
    ) -> UploadFuture<'a, Result<ValidationStoreDisposition, UploadError>> {
        Box::pin(async move {
            let mut stored = lock(&self.evidence);
            match stored.get(evidence.handle()) {
                Some(existing) if existing == &evidence => {
                    Ok(ValidationStoreDisposition::ExistingOutcome)
                }
                Some(_) => Err(UploadError::new(
                    suprnova_live::upload::UploadErrorKind::ValidationEvidenceUnavailable,
                )),
                None => {
                    stored.insert(evidence.handle().clone(), evidence);
                    Ok(ValidationStoreDisposition::Stored)
                }
            }
        })
    }

    fn load<'a>(
        &'a self,
        handle: &'a UploadHandle,
    ) -> UploadFuture<'a, Result<Option<ValidatedUpload>, UploadError>> {
        Box::pin(async move { Ok(lock(&self.evidence).get(handle).cloned()) })
    }

    fn remove<'a>(&'a self, handle: &'a UploadHandle) -> UploadFuture<'a, Result<(), UploadError>> {
        Box::pin(async move {
            lock(&self.evidence).remove(handle);
            Ok(())
        })
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
