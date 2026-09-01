//! Closed endpoint outcome and duplicate-body recovery mappings.

mod endpoint_support;

use std::sync::Arc;

use http::StatusCode;
use suprnova_live::endpoint::{EndpointOutcomeKind, LIVE_MEDIA_TYPE_V1};
use suprnova_live::protocol::ResponseOutcome;

use endpoint_support::{StaticKernel, context, request, response_body, service};

#[tokio::test]
async fn every_closed_kernel_outcome_has_one_http_mapping() {
    for (kind, protocol, status) in [
        (
            EndpointOutcomeKind::Duplicate,
            ResponseOutcome::Duplicate,
            StatusCode::OK,
        ),
        (
            EndpointOutcomeKind::Rejected,
            ResponseOutcome::Rejected,
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            EndpointOutcomeKind::Conflict,
            ResponseOutcome::Rejected,
            StatusCode::CONFLICT,
        ),
        (
            EndpointOutcomeKind::RefreshRequired,
            ResponseOutcome::RefreshRequired,
            StatusCode::CONFLICT,
        ),
        (
            EndpointOutcomeKind::Fatal,
            ResponseOutcome::Fatal,
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
    ] {
        let kernel = Arc::new(StaticKernel::new(kind, response_body(protocol)));
        let response = service(kernel).handle(request(context())).await;
        assert_eq!(response.status, status, "{kind:?}");
        assert_eq!(
            response.headers[http::header::CONTENT_TYPE],
            LIVE_MEDIA_TYPE_V1
        );
    }
}

#[tokio::test]
async fn accepted_outcome_requires_engine_sealed_response_capability() {
    let kernel = Arc::new(StaticKernel::new(
        EndpointOutcomeKind::Accepted,
        response_body(ResponseOutcome::Accepted),
    ));
    let response = service(kernel).handle(request(context())).await;
    assert_eq!(response.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(response.body.is_empty());
}

#[tokio::test]
async fn authorization_concealment_never_returns_kernel_body_or_media_metadata() {
    let kernel = Arc::new(StaticKernel::new(
        EndpointOutcomeKind::Concealed,
        bytes::Bytes::from_static(b"secret resource state"),
    ));
    let response = service(kernel).handle(request(context())).await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
    assert!(response.body.is_empty());
    assert!(!response.headers.contains_key(http::header::CONTENT_TYPE));
}

#[tokio::test]
async fn duplicate_without_retained_body_is_refresh_required() {
    let kernel = Arc::new(StaticKernel::new(
        EndpointOutcomeKind::Duplicate,
        bytes::Bytes::new(),
    ));
    let response = service(kernel).handle(request(context())).await;
    assert_eq!(response.status, StatusCode::CONFLICT);
    assert!(response.body.is_empty());
}
