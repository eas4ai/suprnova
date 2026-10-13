#![cfg(feature = "magnetar-oauth")]

use suprnova::{
    OAuthHttpRequest, OAuthHttpTransport, ParamPlacement, ReqwestOAuthTransport, RevocationRequest,
    RevocationTransport,
};
use wiremock::matchers::{body_string_contains, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn oauth_transport_sends_headers_and_returns_a_bounded_response() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/userinfo"))
        .and(header("authorization", "Bearer access-token"))
        .and(header("user-agent", "community-provider"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("x-provider", "offline")
                .set_body_string("verified-user"),
        )
        .mount(&server)
        .await;

    let transport = ReqwestOAuthTransport::try_default().expect("build reqwest transport");
    let response = OAuthHttpTransport::send(
        &transport,
        OAuthHttpRequest {
            method: "GET".to_owned(),
            url: format!("{}/userinfo", server.uri()),
            headers: vec![
                ("Authorization".to_owned(), "Bearer access-token".to_owned()),
                ("User-Agent".to_owned(), "community-provider".to_owned()),
            ],
            body: Vec::new(),
        },
    )
    .await
    .expect("userinfo request");
    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"verified-user");
    assert!(
        response
            .headers
            .iter()
            .any(|(name, value)| { name.eq_ignore_ascii_case("x-provider") && value == "offline" })
    );

    let bounded = ReqwestOAuthTransport::try_default()
        .expect("build bounded reqwest transport")
        .with_max_response_bytes(4)
        .expect("positive response cap");
    let error = OAuthHttpTransport::send(
        &bounded,
        OAuthHttpRequest {
            method: "GET".to_owned(),
            url: format!("{}/userinfo", server.uri()),
            headers: vec![
                ("Authorization".to_owned(), "Bearer access-token".to_owned()),
                ("User-Agent".to_owned(), "community-provider".to_owned()),
            ],
            body: Vec::new(),
        },
    )
    .await
    .expect_err("response above the configured cap must fail closed");
    assert!(error.to_string().contains("response body exceeded"));
}

#[tokio::test]
async fn revocation_transport_honors_body_and_query_placement() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/body-revoke"))
        .and(body_string_contains("token=body-token"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/query-revoke"))
        .and(query_param("access_token", "query-token"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let transport = ReqwestOAuthTransport::try_default().expect("build reqwest transport");
    RevocationTransport::send(
        &transport,
        RevocationRequest {
            method: "POST",
            endpoint: format!("{}/body-revoke", server.uri()),
            placement: ParamPlacement::Body,
            params: vec![("token".to_owned(), "body-token".to_owned())],
            headers: Vec::new(),
        },
    )
    .await
    .expect("body revocation");
    RevocationTransport::send(
        &transport,
        RevocationRequest {
            method: "DELETE",
            endpoint: format!("{}/query-revoke", server.uri()),
            placement: ParamPlacement::Query,
            params: vec![("access_token".to_owned(), "query-token".to_owned())],
            headers: Vec::new(),
        },
    )
    .await
    .expect("query revocation");
}
