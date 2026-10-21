//! Hostile adapter and application-kernel output containment.

mod endpoint_support;

use std::sync::Arc;

use bytes::Bytes;
use http::StatusCode;
use suprnova_live::endpoint::{EndpointKernel, EndpointOutcomeKind};
use suprnova_live::protocol::ResponseOutcome;

use endpoint_support::{
    FailingKernel, StaticKernel, context, request, response_body, response_body_at_revision,
    service,
};

#[tokio::test]
async fn hostile_kernel_cannot_smuggle_external_redirect_or_partial_protocol_bytes() {
    let unsafe_redirect = response_body(ResponseOutcome::Accepted).to_vec();
    let unsafe_redirect = String::from_utf8(unsafe_redirect)
        .expect("response UTF-8")
        .replace(
            "\"render\":{\"kind\":\"no_render\"}",
            "\"redirect\":\"https://evil.example\"",
        );
    let kernel = Arc::new(StaticKernel::new(
        EndpointOutcomeKind::Accepted,
        Bytes::from(unsafe_redirect),
    ));
    let response = service(kernel).handle(request(context())).await;
    assert_eq!(response.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(response.body.is_empty());

    let partial = Arc::new(StaticKernel::new(
        EndpointOutcomeKind::Accepted,
        Bytes::from_static(b"{\"protocol_version\":1"),
    ));
    let response = service(partial).handle(request(context())).await;
    assert_eq!(response.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(response.body.is_empty());
}

#[tokio::test]
async fn kernel_failures_and_debug_output_are_redacted() {
    let kernel: Arc<dyn EndpointKernel> = Arc::new(FailingKernel);
    let response = service(kernel).handle(request(context())).await;
    assert_eq!(response.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(response.body.is_empty());

    let request = request(context());
    let debug = format!("{request:?}");
    assert!(!debug.contains("snapshot"));
    assert!(!debug.contains("tests.trace"));
}

#[tokio::test]
async fn response_class_version_and_correlation_cannot_disagree() {
    let wrong_class = Arc::new(StaticKernel::new(
        EndpointOutcomeKind::Accepted,
        response_body(ResponseOutcome::Rejected),
    ));
    assert_eq!(
        service(wrong_class).handle(request(context())).await.status,
        StatusCode::INTERNAL_SERVER_ERROR
    );

    let wrong_correlation = Arc::new(StaticKernel::new(
        EndpointOutcomeKind::Accepted,
        Bytes::from(
            String::from_utf8(response_body(ResponseOutcome::Accepted).to_vec())
                .expect("response UTF-8")
                .replace(
                    &endpoint_support::identity::<16>(0x10),
                    &endpoint_support::identity::<16>(0x11),
                ),
        ),
    ));
    let response = service(wrong_correlation).handle(request(context())).await;
    assert_eq!(response.status, StatusCode::INTERNAL_SERVER_ERROR);

    let wrong_version = Arc::new(StaticKernel::new(
        EndpointOutcomeKind::Accepted,
        Bytes::from(
            String::from_utf8(response_body(ResponseOutcome::Accepted).to_vec())
                .expect("response UTF-8")
                .replace("\"protocol_version\":1", "\"protocol_version\":2"),
        ),
    ));
    assert_eq!(
        service(wrong_version)
            .handle(request(context()))
            .await
            .status,
        StatusCode::INTERNAL_SERVER_ERROR
    );
}

#[tokio::test]
async fn successor_snapshot_must_verify_and_match_the_response_revision() {
    let tampered_snapshot = Arc::new(StaticKernel::new(
        EndpointOutcomeKind::Accepted,
        Bytes::from(
            String::from_utf8(response_body(ResponseOutcome::Accepted).to_vec())
                .expect("response UTF-8")
                .replace("\"serial\":1", "\"serial\":2"),
        ),
    ));
    assert_eq!(
        service(tampered_snapshot)
            .handle(request(context()))
            .await
            .status,
        StatusCode::INTERNAL_SERVER_ERROR
    );

    let mismatched_revision = Arc::new(StaticKernel::new(
        EndpointOutcomeKind::Accepted,
        Bytes::from(
            String::from_utf8(response_body(ResponseOutcome::Accepted).to_vec())
                .expect("response UTF-8")
                .replace("\"accepted_revision\":\"1\"", "\"accepted_revision\":\"2\""),
        ),
    ));
    assert_eq!(
        service(mismatched_revision)
            .handle(request(context()))
            .await
            .status,
        StatusCode::INTERNAL_SERVER_ERROR
    );
}

#[tokio::test]
async fn committed_snapshot_must_be_the_immediate_successor_revision() {
    let skipped_revision = Arc::new(StaticKernel::new(
        EndpointOutcomeKind::Accepted,
        response_body_at_revision(ResponseOutcome::Accepted, 2),
    ));

    assert_eq!(
        service(skipped_revision)
            .handle(request(context()))
            .await
            .status,
        StatusCode::INTERNAL_SERVER_ERROR
    );
}
