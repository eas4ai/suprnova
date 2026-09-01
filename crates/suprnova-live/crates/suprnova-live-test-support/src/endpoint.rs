//! Verified endpoint fixtures for engine tests that need one-shot response sealing.

use std::sync::{Arc, Mutex};

use bytes::Bytes;
use suprnova_live::clock::Clock;
use suprnova_live::crypto::SnapshotKeyRing;
use suprnova_live::endpoint::{
    AcceptedResponseRequestBinding, AcceptedResponseSealer, EndpointDispatch, EndpointFuture,
    EndpointKernel, EndpointKernelError, EndpointOutcomeKind, LiveEndpointConfig,
    LiveEndpointRequest, LiveEndpointService, VerifiedEndpointRequest,
};
use suprnova_live::registry::ComponentRegistry;

struct CaptureKernel {
    sealer: Mutex<Option<VerifiedResponseSealing>>,
}

impl CaptureKernel {
    fn take(&self) -> Option<VerifiedResponseSealing> {
        self.sealer.lock().ok()?.take()
    }
}

/// One endpoint-admitted sealer paired with its independently verified request binding.
pub struct VerifiedResponseSealing {
    sealer: AcceptedResponseSealer,
    binding: AcceptedResponseRequestBinding,
}

impl VerifiedResponseSealing {
    pub(crate) const fn new(
        sealer: AcceptedResponseSealer,
        binding: AcceptedResponseRequestBinding,
    ) -> Self {
        Self { sealer, binding }
    }

    /// Consumes the test fixture capability into engine execution inputs.
    #[must_use]
    pub fn into_parts(self) -> (AcceptedResponseSealer, AcceptedResponseRequestBinding) {
        (self.sealer, self.binding)
    }
}

impl EndpointKernel for CaptureKernel {
    fn dispatch<'request>(
        &'request self,
        request: VerifiedEndpointRequest<'request>,
    ) -> EndpointFuture<'request> {
        let (request, sealer) = request.into_execution_parts();
        let sealing = VerifiedResponseSealing::new(sealer, request.response_binding());
        let stored = self
            .sealer
            .lock()
            .map(|mut captured| captured.replace(sealing))
            .is_ok();
        Box::pin(async move {
            if !stored {
                return Err(EndpointKernelError::unavailable());
            }
            Ok(EndpointDispatch::new(
                EndpointOutcomeKind::Concealed,
                Bytes::new(),
            ))
        })
    }
}

/// Admits one real endpoint request and returns its single response-sealing capability.
///
/// `None` means endpoint admission or capture failed; no synthetic constructor is used.
pub async fn capture_verified_response_sealer(
    config: LiveEndpointConfig,
    registry: Arc<ComponentRegistry>,
    clock: Arc<dyn Clock>,
    keys: Arc<SnapshotKeyRing>,
    request: LiveEndpointRequest,
) -> Option<VerifiedResponseSealing> {
    let kernel = Arc::new(CaptureKernel {
        sealer: Mutex::new(None),
    });
    let service = LiveEndpointService::new(config, registry, clock, keys, kernel.clone());
    let _ = service.handle(request).await;
    kernel.take()
}
