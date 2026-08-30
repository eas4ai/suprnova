//! Transport, trusted-context, batch, and signed-authority failure matrix.

mod endpoint_support;

use std::sync::Arc;

use bytes::Bytes;
use http::{Method, StatusCode};
use suprnova_live::endpoint::{
    EndpointErrorKind, EndpointOutcomeKind, LiveEndpointRequest, ParsedLiveMediaType,
    RequestCachePolicy,
};
use suprnova_live::protocol::ResponseOutcome;

use endpoint_support::{
    SequenceClock, StaticKernel, component_support, context, request, request_body,
    request_body_with_snapshot, request_with_body, response_body, service, service_at,
    service_at_with_registry, service_with_clock, signed_instance_with,
};

#[tokio::test]
async fn transport_and_trusted_context_fail_before_kernel_dispatch() {
    let kernel = Arc::new(StaticKernel::new(
        EndpointOutcomeKind::Accepted,
        response_body(ResponseOutcome::Accepted),
    ));
    let endpoint = service(kernel.clone());

    let mut wrong_method = request(context());
    wrong_method.method = Method::GET;
    assert_eq!(
        endpoint.handle(wrong_method).await.status,
        StatusCode::METHOD_NOT_ALLOWED
    );

    let mut oversized = request(context());
    oversized.body = Bytes::from(vec![b'x'; 70 * 1024]);
    assert_eq!(
        endpoint.handle(oversized).await.status,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(kernel.calls(), 0);
}

#[test]
fn missing_context_and_cache_attempts_never_become_endpoint_requests() {
    let media =
        ParsedLiveMediaType::parse("application/vnd.suprnova.live+json; charset=utf-8; version=1")
            .expect("media type");
    let missing = LiveEndpointRequest::try_new(
        Method::POST,
        media,
        Bytes::from_static(b"{}"),
        None,
        RequestCachePolicy::Bypass,
    )
    .expect_err("missing context");
    assert_eq!(missing.kind(), EndpointErrorKind::MissingContext);

    let cache = LiveEndpointRequest::try_new(
        Method::POST,
        media,
        request_body(&context()),
        Some(context()),
        RequestCachePolicy::Attempted,
    )
    .expect_err("cache attempt");
    assert_eq!(cache.kind(), EndpointErrorKind::CacheAttempt);
    assert!(!format!("{cache:?}").contains("tests.trace"));
}

#[tokio::test]
async fn malformed_batches_and_snapshot_binding_mismatches_are_refresh_safe() {
    let kernel = Arc::new(StaticKernel::new(
        EndpointOutcomeKind::Accepted,
        response_body(ResponseOutcome::Accepted),
    ));
    let endpoint = service(kernel.clone());

    let mut malformed = request(context());
    malformed.body = Bytes::from_static(b"{\"protocol_version\":1}");
    assert_eq!(
        endpoint.handle(malformed).await.status,
        StatusCode::BAD_REQUEST
    );

    let mut incompatible = request(context());
    incompatible.body = Bytes::from(
        String::from_utf8(incompatible.body.to_vec())
            .expect("request UTF-8")
            .replace(
                r#""operations":[{"arguments":{},"kind":"invoke_action","name":"execute"}]"#,
                r#""operations":[{"arguments":{},"kind":"invoke_action","name":"execute"},{"arguments":{},"kind":"invoke_action","name":"execute"}]"#,
            ),
    );
    assert_eq!(
        endpoint.handle(incompatible).await.status,
        StatusCode::BAD_REQUEST
    );

    let mut mismatched = request(context());
    mismatched.body = Bytes::from(
        String::from_utf8(mismatched.body.to_vec())
            .expect("request UTF-8")
            .replace("build-lifecycle-tests", "build-hostile-tests"),
    );
    assert_eq!(
        endpoint.handle(mismatched).await.status,
        StatusCode::CONFLICT
    );
    assert_eq!(kernel.calls(), 0);
}

#[tokio::test]
async fn expired_media_mismatched_and_catalog_drifted_requests_never_dispatch() {
    let kernel = Arc::new(StaticKernel::new(
        EndpointOutcomeKind::Accepted,
        response_body(ResponseOutcome::Accepted),
    ));
    let expired = service_at(
        kernel.clone(),
        suprnova_live::identity::UnixMillis::new(2_000),
    )
    .handle(request(context()))
    .await;
    assert_eq!(expired.status, StatusCode::CONFLICT);

    let mut media_mismatch = request(context());
    media_mismatch.content_type =
        ParsedLiveMediaType::parse("application/vnd.suprnova.live+json; charset=utf-8; version=2")
            .expect("v2 media");
    assert_eq!(
        service(kernel.clone()).handle(media_mismatch).await.status,
        StatusCode::BAD_REQUEST
    );

    let component_mismatch = request_with_body(
        context(),
        Bytes::from(
            String::from_utf8(request_body(&context()).to_vec())
                .expect("request UTF-8")
                .replace("tests.trace", "tests.other"),
        ),
    );
    assert_eq!(
        service(kernel.clone())
            .handle(component_mismatch)
            .await
            .status,
        StatusCode::NOT_FOUND
    );

    let empty_registry = suprnova_live::registry::ComponentRegistryBuilder::new().build();
    assert_eq!(
        service_at_with_registry(
            kernel.clone(),
            suprnova_live::identity::UnixMillis::new(1_200),
            empty_registry,
        )
        .handle(request(context()))
        .await
        .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(kernel.calls(), 0);
}

#[tokio::test]
async fn correctly_signed_route_slot_scope_and_revision_mismatches_fail_closed() {
    use suprnova_live::identity::{BuildId, IslandSlot, ScopeFingerprint};

    let kernel = Arc::new(StaticKernel::new(
        EndpointOutcomeKind::Accepted,
        response_body(ResponseOutcome::Accepted),
    ));
    let endpoint = service(kernel.clone());
    let build = BuildId::parse("build-lifecycle-tests").expect("build");
    let base = context();
    let cases = [
        signed_instance_with(
            0,
            build.clone(),
            component_support::snapshot_support::route(0x31),
            base.mount().slot().clone(),
            base.scope().clone(),
        ),
        signed_instance_with(
            0,
            build.clone(),
            base.mount().route().clone(),
            IslandSlot::parse("other-slot").expect("slot"),
            base.scope().clone(),
        ),
        signed_instance_with(
            0,
            build,
            base.mount().route().clone(),
            base.mount().slot().clone(),
            ScopeFingerprint::from_bytes(&component_support::bytes::<32>(0x55)).expect("scope"),
        ),
    ];
    for snapshot in cases {
        let request = request_with_body(context(), request_body_with_snapshot(snapshot));
        assert_eq!(endpoint.handle(request).await.status, StatusCode::CONFLICT);
    }

    let revision_mismatch = Bytes::from(
        String::from_utf8(request_body(&context()).to_vec())
            .expect("request UTF-8")
            .replace("\"base_revision\":\"0\"", "\"base_revision\":\"1\""),
    );
    assert_eq!(
        endpoint
            .handle(request_with_body(context(), revision_mismatch))
            .await
            .status,
        StatusCode::CONFLICT
    );
    assert_eq!(kernel.calls(), 0);
}

#[tokio::test]
async fn context_is_rechecked_after_kernel_completion_before_output_publication() {
    use suprnova_live::identity::UnixMillis;

    let kernel = Arc::new(StaticKernel::new(
        EndpointOutcomeKind::Accepted,
        response_body(ResponseOutcome::Accepted),
    ));
    let clock = Arc::new(SequenceClock::new(vec![
        UnixMillis::new(1_200),
        UnixMillis::new(2_000),
    ]));
    let response = service_with_clock(kernel.clone(), clock)
        .handle(request(context()))
        .await;
    assert_eq!(response.status, StatusCode::CONFLICT);
    assert!(response.body.is_empty());
    assert_eq!(kernel.calls(), 1);
}
