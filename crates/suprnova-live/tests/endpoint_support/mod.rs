#![allow(
    dead_code,
    reason = "shared by separate endpoint integration-test crates"
)]

#[path = "../component_support.rs"]
pub(crate) mod component_support;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use bytes::Bytes;
use http::Method;
use suprnova_live::clock::{Clock, ClockError};
use suprnova_live::endpoint::{
    EndpointDispatch, EndpointFuture, EndpointKernel, EndpointKernelError, EndpointOutcomeKind,
    LiveEndpointConfig, LiveEndpointRequest, LiveEndpointService, ParsedLiveMediaType,
    RequestCachePolicy, VerifiedEndpointRequest,
};
use suprnova_live::host::TrustedLiveRequestContext;
use suprnova_live::identity::{
    BuildId, InstanceId, IslandSlot, Revision, RouteIdentity, ScopeFingerprint, UnixMillis,
};
use suprnova_live::limits::InputLimits;
use suprnova_live::protocol::{ProtocolLimitConfig, ProtocolLimits, ResponseOutcome};
use suprnova_live::registry::{ComponentDescriptor, ComponentRegistry, ComponentRegistryBuilder};
use suprnova_live::snapshot::{InstanceBodyV1, InstanceFieldsV1};

#[derive(Clone, Copy)]
pub(crate) struct FixedClock(pub(crate) UnixMillis);

impl Clock for FixedClock {
    fn now(&self) -> Result<UnixMillis, ClockError> {
        Ok(self.0)
    }
}

pub(crate) struct SequenceClock {
    values: Mutex<Vec<UnixMillis>>,
}

impl SequenceClock {
    pub(crate) fn new(values: Vec<UnixMillis>) -> Self {
        Self {
            values: Mutex::new(values),
        }
    }
}

impl Clock for SequenceClock {
    fn now(&self) -> Result<UnixMillis, ClockError> {
        let mut values = self.values.lock().expect("clock lock");
        if values.len() > 1 {
            Ok(values.remove(0))
        } else {
            values
                .first()
                .copied()
                .ok_or_else(ClockError::timestamp_overflow)
        }
    }
}

pub(crate) struct StaticKernel {
    outcome: EndpointOutcomeKind,
    response: Bytes,
    calls: AtomicUsize,
}

impl StaticKernel {
    pub(crate) fn new(outcome: EndpointOutcomeKind, response: impl Into<Bytes>) -> Self {
        Self {
            outcome,
            response: response.into(),
            calls: AtomicUsize::new(0),
        }
    }

    pub(crate) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl EndpointKernel for StaticKernel {
    fn dispatch<'request>(
        &'request self,
        request: VerifiedEndpointRequest<'request>,
    ) -> EndpointFuture<'request> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let response = self.response.clone();
        let outcome = self.outcome;
        Box::pin(async move {
            let (request, _) = request.into_execution_parts();
            assert_eq!(request.component().as_str(), "tests.trace");
            Ok(EndpointDispatch::new(outcome, response))
        })
    }
}

pub(crate) struct FailingKernel;

impl EndpointKernel for FailingKernel {
    fn dispatch<'request>(
        &'request self,
        _request: VerifiedEndpointRequest<'request>,
    ) -> Pin<
        Box<dyn Future<Output = Result<EndpointDispatch, EndpointKernelError>> + Send + 'request>,
    > {
        Box::pin(async { Err(EndpointKernelError::unavailable()) })
    }
}

pub(crate) fn context() -> TrustedLiveRequestContext {
    component_support::trusted_context()
}

pub(crate) fn protocol_limits() -> ProtocolLimits {
    ProtocolLimits::new(ProtocolLimitConfig {
        input: InputLimits::new(64 * 1024, 12, 512, 40 * 1024).expect("input limits"),
        max_snapshot_bytes: 32 * 1024,
        max_html_bytes: 32 * 1024,
        max_model_proposals: 8,
        max_operations: 8,
        max_arguments: 16,
        max_validation_entries: 16,
        max_events: 8,
        max_effects: 8,
        max_extensions: 8,
    })
    .expect("protocol limits")
}

pub(crate) fn signed_instance(context: &TrustedLiveRequestContext, revision: u64) -> Vec<u8> {
    signed_instance_with(
        revision,
        BuildId::parse("build-lifecycle-tests").expect("build identity"),
        context.mount().route().clone(),
        context.mount().slot().clone(),
        context.scope().clone(),
    )
}

pub(crate) fn signed_instance_with(
    revision: u64,
    build_id: BuildId,
    route: RouteIdentity,
    slot: IslandSlot,
    scope: ScopeFingerprint,
) -> Vec<u8> {
    let keys = component_support::key_ring();
    let descriptor = ComponentDescriptor::new(component_support::metadata().clone());
    let contract = suprnova_live::snapshot::ComponentContract::new(
        component_support::metadata().identity().clone(),
        descriptor.contract_digest().clone(),
        1,
        1,
        1,
    )
    .expect("component contract");
    let state = component_support::snapshot_support::public_value(r#"{"serial":1}"#);
    let memo = component_support::snapshot_support::public_value("{}");
    InstanceBodyV1::new(
        InstanceFieldsV1 {
            component: contract,
            build_id,
            route,
            slot,
            key_id: keys.active_key_id().clone(),
            scope,
            instance_id: InstanceId::from_bytes(&component_support::bytes::<16>(0xa0))
                .expect("instance identity"),
            revision: Revision::new(revision),
            issued_at: UnixMillis::new(1_000),
            expires_at: UnixMillis::new(1_900),
            state,
            memo,
            extensions: Default::default(),
        },
        &component_support::schema_set(),
        &component_support::snapshot_limits(),
    )
    .expect("instance constructs")
    .sign(
        &keys,
        UnixMillis::new(1_100),
        &component_support::snapshot_limits(),
    )
    .expect("instance signs")
}

pub(crate) fn request_body(context: &TrustedLiveRequestContext) -> Bytes {
    request_body_with_snapshot(signed_instance(context, 0))
}

pub(crate) fn request_body_with_snapshot(snapshot: Vec<u8>) -> Bytes {
    let snapshot = String::from_utf8(snapshot).expect("snapshot UTF-8");
    Bytes::from(format!(
        r#"{{"base_revision":"0","component":"tests.trace","correlation_id":"{}","extensions":{{}},"idempotency_key":"{}","model_proposals":{{}},"operations":[{{"arguments":{{}},"kind":"invoke_action","name":"execute"}}],"protocol_version":1,"runtime_contract_version":1,"snapshot":{{"envelope":{},"kind":"instance"}},"snapshot_schema_version":1}}"#,
        identity::<16>(0x10),
        identity::<16>(0x30),
        snapshot,
    ))
}

pub(crate) fn response_body(outcome: ResponseOutcome) -> Bytes {
    response_body_at_revision(outcome, 1)
}

pub(crate) fn response_body_at_revision(outcome: ResponseOutcome, revision: u64) -> Bytes {
    let correlation = identity::<16>(0x10);
    let value = match outcome {
        ResponseOutcome::Accepted | ResponseOutcome::Duplicate => {
            let snapshot = String::from_utf8(signed_instance(&context(), revision))
                .expect("response snapshot UTF-8");
            format!(
                r#"{{"accepted_revision":"{revision}","correlation_id":"{correlation}","effects":[],"events":[],"extensions":{{}},"outcome":"{}","protocol_version":1,"render":{{"kind":"no_render"}},"snapshot":{snapshot},"validation":{{}}}}"#,
                if outcome == ResponseOutcome::Accepted {
                    "accepted"
                } else {
                    "duplicate"
                }
            )
        }
        ResponseOutcome::Rejected => format!(
            r#"{{"correlation_id":"{correlation}","effects":[],"error":{{"category":"validation","detail":"invalid_identifier","recovery":"retain_dom"}},"events":[],"extensions":{{}},"outcome":"rejected","protocol_version":1,"validation":{{"query":"invalid"}}}}"#
        ),
        ResponseOutcome::RefreshRequired => format!(
            r#"{{"correlation_id":"{correlation}","effects":[],"error":{{"category":"snapshot","detail":"signature_invalid","recovery":"refresh_island"}},"events":[],"extensions":{{}},"outcome":"refresh_required","protocol_version":1,"validation":{{}}}}"#
        ),
        ResponseOutcome::Fatal => format!(
            r#"{{"correlation_id":"{correlation}","effects":[],"error":{{"category":"internal","detail":"serialization_failed","recovery":"stop"}},"events":[],"extensions":{{}},"outcome":"fatal","protocol_version":1,"validation":{{}}}}"#
        ),
    };
    Bytes::from(value)
}

pub(crate) fn identity<const LENGTH: usize>(start: u8) -> String {
    use base64::Engine as _;

    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(std::array::from_fn::<_, LENGTH, _>(
        |index| start.wrapping_add(index as u8),
    ))
}

pub(crate) fn registry() -> ComponentRegistry {
    let descriptor = ComponentDescriptor::new(component_support::metadata().clone());
    ComponentRegistryBuilder::new()
        .register(descriptor)
        .expect("registry entry")
        .build()
}

pub(crate) fn service(kernel: Arc<dyn EndpointKernel>) -> LiveEndpointService {
    service_at_with_registry(kernel, UnixMillis::new(1_200), registry())
}

pub(crate) fn service_at(kernel: Arc<dyn EndpointKernel>, now: UnixMillis) -> LiveEndpointService {
    service_at_with_registry(kernel, now, registry())
}

pub(crate) fn service_at_with_registry(
    kernel: Arc<dyn EndpointKernel>,
    now: UnixMillis,
    registry: ComponentRegistry,
) -> LiveEndpointService {
    LiveEndpointService::new(
        LiveEndpointConfig::new(protocol_limits(), component_support::snapshot_limits())
            .expect("endpoint config"),
        Arc::new(registry),
        Arc::new(FixedClock(now)),
        Arc::new(component_support::key_ring()),
        kernel,
    )
}

pub(crate) fn service_with_response_limit(
    kernel: Arc<dyn EndpointKernel>,
    max_response_bytes: usize,
) -> LiveEndpointService {
    LiveEndpointService::new(
        LiveEndpointConfig::new(protocol_limits(), component_support::snapshot_limits())
            .expect("endpoint config")
            .with_max_response_bytes(max_response_bytes)
            .expect("response limit"),
        Arc::new(registry()),
        Arc::new(FixedClock(UnixMillis::new(1_200))),
        Arc::new(component_support::key_ring()),
        kernel,
    )
}

pub(crate) fn service_with_clock(
    kernel: Arc<dyn EndpointKernel>,
    clock: Arc<dyn Clock>,
) -> LiveEndpointService {
    LiveEndpointService::new(
        LiveEndpointConfig::new(protocol_limits(), component_support::snapshot_limits())
            .expect("endpoint config"),
        Arc::new(registry()),
        clock,
        Arc::new(component_support::key_ring()),
        kernel,
    )
}

pub(crate) fn request(context: TrustedLiveRequestContext) -> LiveEndpointRequest {
    let body = request_body(&context);
    LiveEndpointRequest::try_new(
        Method::POST,
        ParsedLiveMediaType::parse("application/vnd.suprnova.live+json; charset=utf-8; version=1")
            .expect("media type"),
        body,
        Some(context),
        RequestCachePolicy::Bypass,
    )
    .expect("normalized endpoint request")
}

pub(crate) fn request_with_body(
    context: TrustedLiveRequestContext,
    body: Bytes,
) -> LiveEndpointRequest {
    LiveEndpointRequest::try_new(
        Method::POST,
        ParsedLiveMediaType::parse("application/vnd.suprnova.live+json; charset=utf-8; version=1")
            .expect("media type"),
        body,
        Some(context),
        RequestCachePolicy::Bypass,
    )
    .expect("normalized endpoint request")
}
