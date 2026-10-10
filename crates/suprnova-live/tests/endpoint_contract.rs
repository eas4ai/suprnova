//! Exact normalized endpoint, response media, and security-header contract.

mod endpoint_support;

use std::sync::Arc;

use http::{StatusCode, header};
use suprnova_live::endpoint::{
    EndpointOutcomeKind, LIVE_MEDIA_TYPE_V1, LiveEndpointConfig, ParsedLiveMediaType,
};
use suprnova_live::protocol::ResponseOutcome;

use endpoint_support::{
    StaticKernel, component_support, context, protocol_limits, request, request_body,
    response_body, service, service_with_response_limit,
};

#[tokio::test]
async fn nonaccepted_response_is_canonical_complete_and_security_bounded() {
    let noncanonical = bytes::Bytes::from(
        String::from_utf8(response_body(ResponseOutcome::Rejected).to_vec())
            .expect("response UTF-8")
            .replace(',', ", "),
    );
    let kernel = Arc::new(StaticKernel::new(
        EndpointOutcomeKind::Rejected,
        noncanonical,
    ));
    let response = service(kernel.clone()).handle(request(context())).await;

    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(response.headers[header::CONTENT_TYPE], LIVE_MEDIA_TYPE_V1);
    assert_eq!(response.headers[header::CACHE_CONTROL], "no-store");
    assert_eq!(
        response.headers[header::CONTENT_LENGTH],
        response.body.len().to_string()
    );
    assert_eq!(response.headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert_eq!(response.headers[header::REFERRER_POLICY], "no-referrer");
    assert_eq!(
        response.headers[header::CONTENT_SECURITY_POLICY],
        "default-src 'none'; frame-ancestors 'none'"
    );
    assert_eq!(kernel.calls(), 1);
    assert!(!response.body.windows(2).any(|window| window == b", "));
}

#[tokio::test]
async fn unsealed_accepted_kernel_bytes_are_rejected() {
    let body = String::from_utf8(response_body(ResponseOutcome::Accepted).to_vec())
        .expect("response UTF-8")
        .replace(
            r#""render":{"kind":"no_render"}"#,
            r#""render":{"html":"<div data-suprnova-live-island data-suprnova-live-revision=\"1\">ok</div>","kind":"html"}"#,
        );
    let response = service(Arc::new(StaticKernel::new(
        EndpointOutcomeKind::Accepted,
        bytes::Bytes::from(body),
    )))
    .handle(request(context()))
    .await;

    assert_eq!(response.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(response.body.is_empty());
}

#[test]
fn normalization_failures_receive_closed_empty_http_responses() {
    use bytes::Bytes;
    use http::Method;
    use suprnova_live::endpoint::{LiveEndpointRequest, ParsedLiveMediaType, RequestCachePolicy};

    let endpoint = service(Arc::new(StaticKernel::new(
        EndpointOutcomeKind::Accepted,
        response_body(ResponseOutcome::Accepted),
    )));
    let media = ParsedLiveMediaType::parse(LIVE_MEDIA_TYPE_V1).expect("media");
    let error = LiveEndpointRequest::try_new(
        Method::POST,
        media,
        Bytes::new(),
        None,
        RequestCachePolicy::Bypass,
    )
    .expect_err("context is required");
    let response = endpoint.error_response(error);
    assert_eq!(response.status, StatusCode::NOT_FOUND);
    assert_eq!(response.headers[header::CACHE_CONTROL], "no-store");
    assert_eq!(response.headers[header::CONTENT_LENGTH], "0");
    assert!(response.body.is_empty());
}

#[tokio::test]
async fn complete_response_bytes_are_bounded_before_http_success_exists() {
    let kernel = Arc::new(StaticKernel::new(
        EndpointOutcomeKind::Rejected,
        response_body(ResponseOutcome::Rejected),
    ));
    let response = service_with_response_limit(kernel, 64)
        .handle(request(context()))
        .await;
    assert_eq!(response.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(response.headers[header::CONTENT_LENGTH], "0");
    assert!(response.body.is_empty());
}

#[test]
fn media_type_is_exact_and_versioned() {
    use suprnova_live::endpoint::{EndpointErrorKind, ParsedLiveMediaType};

    let parsed = ParsedLiveMediaType::parse(LIVE_MEDIA_TYPE_V1).expect("v1 media type");
    assert_eq!(parsed.protocol_version(), 1);
    assert_eq!(parsed.to_string(), LIVE_MEDIA_TYPE_V1);

    for (value, expected) in [
        (
            "application/json; charset=utf-8; version=1",
            EndpointErrorKind::UnsupportedMediaType,
        ),
        (
            "application/vnd.suprnova.live+json; charset=latin1; version=1",
            EndpointErrorKind::UnsupportedCharset,
        ),
        (
            "application/vnd.suprnova.live+json; charset=utf-8; version=99",
            EndpointErrorKind::UnsupportedVersion,
        ),
    ] {
        assert_eq!(
            ParsedLiveMediaType::parse(value)
                .expect_err("media must fail")
                .kind(),
            expected
        );
    }
}

#[test]
fn bounded_engine_inspection_returns_only_catalog_selection_facts() {
    let context = context();
    let body = request_body(&context);
    let config = LiveEndpointConfig::new(protocol_limits(), component_support::snapshot_limits())
        .expect("endpoint config");
    let media = ParsedLiveMediaType::parse(LIVE_MEDIA_TYPE_V1).expect("media");

    let selection = config
        .inspect_mount(&body, media)
        .expect("bounded mount selection");

    assert_eq!(selection.route(), context.mount().route());
    assert_eq!(selection.slot(), context.mount().slot());
    assert_eq!(selection.component(), context.mount().component());
    assert_eq!(
        selection.contract_digest(),
        context.mount().contract_digest()
    );
    assert_eq!(selection.protocol(), 1);
}
