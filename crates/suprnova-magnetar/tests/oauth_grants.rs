//! Grant-executor suites (Task 4): authorization_code, client_credentials,
//! jwt_bearer, refresh_token
//! (`docs/specs/suprnova-magnetar/09-oauth-engine.md`'s "Refresh and
//! revocation" and "Client credentials and JWT bearer" sections).

#![cfg(all(feature = "oauth", feature = "seaorm-sqlite"))]

#[path = "fixtures/grants_harness.rs"]
mod grants_harness;
#[path = "fixtures/oauth_harness.rs"]
mod oauth_harness;
#[path = "fixtures/storage_schema.rs"]
mod storage_schema;

use std::sync::Arc;

use magnetar::oauth::authorization::{CeremonyBinding, OAuthCeremony, OAuthIntent};
use magnetar::oauth::grants::{authorization_code, client_credentials, jwt_bearer, refresh};
use magnetar::oauth::{OAuthProtocolError, TokenRequestShape};
use secrecy::{ExposeSecret, SecretString};

use grants_harness::{MockOAuthProvider, RecordingRevocationTransport, test_signing_key};

fn provider(endpoint: &str) -> MockOAuthProvider {
    MockOAuthProvider::new(
        "mock",
        endpoint,
        Arc::new(RecordingRevocationTransport::default()),
    )
}

fn ceremony(provider: &str, verifier: Option<&str>) -> OAuthCeremony {
    OAuthCeremony {
        selector: "sel-1".to_owned(),
        provider: provider.to_owned(),
        verifier: verifier.map(|v| SecretString::from(v.to_owned())),
        nonce: None,
        intent: OAuthIntent::SignIn,
        binding: CeremonyBinding::StateOnly,
    }
}

// --- authorization_code -----------------------------------------------

#[tokio::test]
async fn authorization_code_happy_path_threads_pkce_verifier_from_ceremony() {
    let h = grants_harness::harness().await;
    let p = provider("https://mock.test/token");
    h.http.push_json(
        200,
        r#"{"access_token":"AT1","token_type":"Bearer","expires_in":3600}"#,
    );

    let result = authorization_code::execute(
        &p,
        h.http.as_ref(),
        &ceremony("mock", Some("verifier-123")),
        SecretString::from("code-abc".to_owned()),
        Some("https://app.test/callback".to_owned()),
        vec!["read".to_owned()],
    )
    .await
    .expect("token exchange succeeds");

    assert_eq!(result.access_token.expose_secret(), "AT1");
    let sent = h.http.last_request();
    // W5: an incidental `{request:?}` (e.g. a log line) must never
    // reproduce the secret material this request just rendered.
    let debugged = format!("{sent:?}");
    assert!(!debugged.contains("verifier-123"), "{debugged}");
    assert!(!debugged.contains("code-abc"), "{debugged}");
    assert!(!debugged.contains("mock-secret"), "{debugged}");
    let body = String::from_utf8(sent.body).unwrap();
    assert!(body.contains("code_verifier=verifier-123"), "{body}");
    assert!(body.contains("grant_type=authorization_code"), "{body}");
    assert!(body.contains("code=code-abc"), "{body}");
    assert!(body.contains("client_secret=mock-secret"), "{body}");
    assert!(body.contains("redirect_uri="), "{body}");

    // RF2: the token-endpoint *response* carries the live access token in
    // plaintext JSON -- its `Debug` must redact it too, not just the
    // request. Built from the exact JSON body this exchange was scripted
    // to receive (line 51 above), so the assertion cannot pass vacuously
    // against an empty or unrelated body.
    let response_body = r#"{"access_token":"AT1","token_type":"Bearer","expires_in":3600}"#;
    let response = magnetar::plugin::HttpResponse {
        status: 200,
        headers: vec![("Content-Type".to_owned(), "application/json".to_owned())],
        body: response_body.as_bytes().to_vec(),
    };
    let response_debugged = format!("{response:?}");
    assert!(!response_debugged.contains("AT1"), "{response_debugged}");
}

#[tokio::test]
async fn authorization_code_callback_result_preserves_raw_body_and_redacts_secrets() {
    let h = grants_harness::harness().await;
    let p = provider("https://mock.test/token");
    let body = r#"{"access_token":"access-token-live-value","token_type":"Bearer","refresh_token":"refresh-token-live-value","id_token":"apple-id-token-live-value","provider_extension":{"identity_reference":"provider-identity-live-value"}}"#;
    h.http.push_json(200, body);

    let result = authorization_code::execute_with_raw(
        &p,
        h.http.as_ref(),
        &ceremony("mock", Some("verifier-123")),
        SecretString::from("code-abc".to_owned()),
        Some("https://app.test/callback".to_owned()),
        vec!["openid".to_owned()],
    )
    .await
    .expect("callback token exchange succeeds");

    assert_eq!(result.raw_body().expose_secret(), body);
    assert_eq!(
        result
            .response
            .id_token
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("apple-id-token-live-value")
    );
    for debugged in [format!("{result:?}"), format!("{:?}", result.raw_body())] {
        for secret in [
            "access-token-live-value",
            "refresh-token-live-value",
            "apple-id-token-live-value",
            "provider-identity-live-value",
        ] {
            assert!(!debugged.contains(secret), "{debugged}");
        }
    }
}

#[tokio::test]
async fn authorization_code_requires_verifier_when_pkce_required_and_never_calls_transport() {
    let h = grants_harness::harness().await;
    let p = provider("https://mock.test/token");

    let err = authorization_code::execute(
        &p,
        h.http.as_ref(),
        &ceremony("mock", None),
        SecretString::from("code-abc".to_owned()),
        None,
        Vec::new(),
    )
    .await
    .unwrap_err();

    assert!(matches!(
        err,
        OAuthProtocolError::InvalidRequestShape { .. }
    ));
    assert_eq!(h.http.request_count(), 0);
}

#[tokio::test]
async fn authorization_code_rejects_ceremony_provider_mismatch() {
    let h = grants_harness::harness().await;
    let p = provider("https://mock.test/token");

    let err = authorization_code::execute(
        &p,
        h.http.as_ref(),
        &ceremony("someone-else", Some("v")),
        SecretString::from("code-abc".to_owned()),
        None,
        Vec::new(),
    )
    .await
    .unwrap_err();

    assert!(matches!(
        err,
        OAuthProtocolError::InvalidRequestShape { .. }
    ));
    assert_eq!(h.http.request_count(), 0);
}

#[tokio::test]
async fn authorization_code_maps_rfc6749_error_body_on_4xx() {
    let h = grants_harness::harness().await;
    let p = provider("https://mock.test/token");
    h.http.push_json(
        400,
        r#"{"error":"invalid_grant","error_description":"bad code"}"#,
    );

    let err = authorization_code::execute(
        &p,
        h.http.as_ref(),
        &ceremony("mock", Some("v")),
        SecretString::from("code-abc".to_owned()),
        None,
        Vec::new(),
    )
    .await
    .unwrap_err();

    match err {
        OAuthProtocolError::ProviderReportedError { code, .. } => assert_eq!(code, "invalid_grant"),
        other => panic!("expected ProviderReportedError, got {other:?}"),
    }
}

#[tokio::test]
async fn authorization_code_maps_5xx_to_upstream_unavailable() {
    let h = grants_harness::harness().await;
    let p = provider("https://mock.test/token");
    h.http.push_json(503, "service unavailable");

    let err = authorization_code::execute(
        &p,
        h.http.as_ref(),
        &ceremony("mock", Some("v")),
        SecretString::from("code-abc".to_owned()),
        None,
        Vec::new(),
    )
    .await
    .unwrap_err();

    assert!(matches!(
        err,
        OAuthProtocolError::UpstreamUnavailable { .. }
    ));
}

#[tokio::test]
async fn authorization_code_maps_transport_failure_to_upstream_unavailable() {
    let h = grants_harness::harness().await;
    let p = provider("https://mock.test/token");
    h.http.push_transport_error();

    let err = authorization_code::execute(
        &p,
        h.http.as_ref(),
        &ceremony("mock", Some("v")),
        SecretString::from("code-abc".to_owned()),
        None,
        Vec::new(),
    )
    .await
    .unwrap_err();

    assert!(matches!(
        err,
        OAuthProtocolError::UpstreamUnavailable { .. }
    ));
}

// --- client_credentials --------------------------------------------------

#[tokio::test]
async fn client_credentials_happy_path() {
    let h = grants_harness::harness().await;
    let p = provider("https://mock.test/token");
    h.http
        .push_json(200, r#"{"access_token":"AT3","token_type":"Bearer"}"#);

    let result = client_credentials::execute(&p, h.http.as_ref(), &[])
        .await
        .expect("client credentials succeeds");
    assert_eq!(result.access_token.expose_secret(), "AT3");
    let body = String::from_utf8(h.http.last_request().body).unwrap();
    assert!(body.contains("grant_type=client_credentials"), "{body}");
    assert!(!body.contains("scope="), "no scopes requested: {body}");
}

#[tokio::test]
async fn client_credentials_sends_requested_scope() {
    let h = grants_harness::harness().await;
    let p = provider("https://mock.test/token");
    h.http
        .push_json(200, r#"{"access_token":"AT3b","token_type":"Bearer"}"#);

    client_credentials::execute(
        &p,
        h.http.as_ref(),
        &["read".to_owned(), "write".to_owned()],
    )
    .await
    .unwrap();
    let body = String::from_utf8(h.http.last_request().body).unwrap();
    assert!(
        body.contains("scope=read+write") || body.contains("scope=read%20write"),
        "{body}"
    );
}

#[tokio::test]
async fn client_credentials_probes_http_200_error_body_when_shape_allows() {
    let h = grants_harness::harness().await;
    let mut p = provider("https://mock.test/token");
    p.token_shape_value = TokenRequestShape {
        accept_http_success_error_body: true,
        ..TokenRequestShape::default()
    };
    h.http.push_json(
        200,
        r#"{"error":"invalid_request","error_description":"bad scope"}"#,
    );

    let err = client_credentials::execute(&p, h.http.as_ref(), &[])
        .await
        .unwrap_err();
    match err {
        OAuthProtocolError::ProviderReportedError { code, .. } => {
            assert_eq!(code, "invalid_request")
        }
        other => panic!("expected ProviderReportedError, got {other:?}"),
    }
}

#[tokio::test]
async fn client_credentials_rejects_error_body_on_success_status_when_shape_does_not_allow() {
    let h = grants_harness::harness().await;
    let p = provider("https://mock.test/token");
    h.http.push_json(
        200,
        r#"{"error":"invalid_request","error_description":"bad scope"}"#,
    );

    let err = client_credentials::execute(&p, h.http.as_ref(), &[])
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        OAuthProtocolError::MalformedTokenResponse { .. }
    ));
}

// --- refresh_token ---------------------------------------------------------

#[tokio::test]
async fn refresh_happy_path_sends_refresh_token_grant() {
    let h = grants_harness::harness().await;
    let p = provider("https://mock.test/token");
    h.http.push_json(
        200,
        r#"{"access_token":"AT2","token_type":"Bearer","refresh_token":"RT2"}"#,
    );

    let result = refresh::execute(
        &p,
        h.http.as_ref(),
        SecretString::from("old-rt".to_owned()),
        &["read".to_owned()],
    )
    .await
    .expect("refresh succeeds");
    assert_eq!(result.access_token.expose_secret(), "AT2");
    assert_eq!(
        result
            .refresh_token
            .expect("rotated refresh token")
            .expose_secret(),
        "RT2"
    );
    let body = String::from_utf8(h.http.last_request().body).unwrap();
    assert!(body.contains("grant_type=refresh_token"), "{body}");
    assert!(body.contains("refresh_token=old-rt"), "{body}");
}

#[tokio::test]
async fn refresh_maps_rfc6749_error_body_on_4xx() {
    let h = grants_harness::harness().await;
    let p = provider("https://mock.test/token");
    h.http.push_json(
        400,
        r#"{"error":"invalid_grant","error_description":"refresh token revoked"}"#,
    );

    let err = refresh::execute(
        &p,
        h.http.as_ref(),
        SecretString::from("old-rt".to_owned()),
        &[],
    )
    .await
    .unwrap_err();
    match err {
        OAuthProtocolError::ProviderReportedError { code, .. } => assert_eq!(code, "invalid_grant"),
        other => panic!("expected ProviderReportedError, got {other:?}"),
    }
}

#[tokio::test]
async fn refresh_rejects_provider_without_refresh_support_before_any_network_call() {
    let h = grants_harness::harness().await;
    let mut p = provider("https://mock.test/token");
    p.refresh_supported = false;

    let err = refresh::execute(
        &p,
        h.http.as_ref(),
        SecretString::from("old-rt".to_owned()),
        &[],
    )
    .await
    .unwrap_err();
    assert!(matches!(
        err,
        OAuthProtocolError::ProviderConfiguration { .. }
    ));
    assert_eq!(h.http.request_count(), 0);
}

// --- jwt_bearer --------------------------------------------------------

#[tokio::test]
async fn jwt_bearer_mints_assertion_with_expected_claims_and_exchanges_it() {
    let h = grants_harness::harness().await;
    let p = provider("https://mock.test/token");
    h.http
        .push_json(200, r#"{"access_token":"AT4","token_type":"Bearer"}"#);

    let key = test_signing_key();
    let signing_key = jwt_bearer::JwtBearerSigningKey {
        algorithm: jsonwebtoken::Algorithm::HS256,
        encoding_key: jsonwebtoken::EncodingKey::from_secret(key.expose_secret().as_bytes()),
    };
    let assertion = jwt_bearer::JwtBearerAssertion {
        issuer: "svc-account-1".to_owned(),
        subject: "svc-account-1".to_owned(),
        audience: "https://mock.test/token".to_owned(),
        key_id: Some("kid-1".to_owned()),
    };

    let before = chrono::Utc::now().timestamp();
    let result = jwt_bearer::execute(
        &p,
        h.http.as_ref(),
        &assertion,
        &signing_key,
        chrono::Duration::minutes(5),
        &[],
    )
    .await
    .expect("jwt-bearer exchange succeeds");
    assert_eq!(result.access_token.expose_secret(), "AT4");

    let body = String::from_utf8(h.http.last_request().body).unwrap();
    assert!(
        body.contains("grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer"),
        "{body}"
    );
    let signed = body
        .split('&')
        .find_map(|pair| pair.strip_prefix("assertion="))
        .expect("assertion parameter present");

    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256);
    validation.set_audience(&["https://mock.test/token"]);
    let decoding_key = jsonwebtoken::DecodingKey::from_secret(key.expose_secret().as_bytes());
    let decoded = jsonwebtoken::decode::<serde_json::Value>(signed, &decoding_key, &validation)
        .expect("assertion verifies under the same key/algorithm");
    assert_eq!(decoded.header.alg, jsonwebtoken::Algorithm::HS256);
    assert_eq!(decoded.header.kid.as_deref(), Some("kid-1"));
    assert_eq!(decoded.claims["iss"], "svc-account-1");
    assert_eq!(decoded.claims["sub"], "svc-account-1");
    assert_eq!(decoded.claims["aud"], "https://mock.test/token");
    let iat = decoded.claims["iat"].as_i64().unwrap();
    let exp = decoded.claims["exp"].as_i64().unwrap();
    assert!(iat >= before);
    assert_eq!(exp - iat, 5 * 60);
}

#[tokio::test]
async fn jwt_bearer_maps_rfc6749_error_body_on_4xx() {
    let h = grants_harness::harness().await;
    let p = provider("https://mock.test/token");
    h.http.push_json(
        400,
        r#"{"error":"invalid_grant","error_description":"assertion expired"}"#,
    );

    let key = test_signing_key();
    let signing_key = jwt_bearer::JwtBearerSigningKey {
        algorithm: jsonwebtoken::Algorithm::HS256,
        encoding_key: jsonwebtoken::EncodingKey::from_secret(key.expose_secret().as_bytes()),
    };
    let assertion = jwt_bearer::JwtBearerAssertion {
        issuer: "svc-account-1".to_owned(),
        subject: "svc-account-1".to_owned(),
        audience: "https://mock.test/token".to_owned(),
        key_id: None,
    };

    let err = jwt_bearer::execute(
        &p,
        h.http.as_ref(),
        &assertion,
        &signing_key,
        chrono::Duration::minutes(5),
        &[],
    )
    .await
    .unwrap_err();
    match err {
        OAuthProtocolError::ProviderReportedError { code, .. } => assert_eq!(code, "invalid_grant"),
        other => panic!("expected ProviderReportedError, got {other:?}"),
    }
}

#[tokio::test]
async fn jwt_bearer_signing_failure_maps_to_provider_configuration() {
    let h = grants_harness::harness().await;
    let p = provider("https://mock.test/token");
    // An RS256 algorithm paired with an HMAC-shaped secret key fails to
    // encode inside `jsonwebtoken` -- a host-supplied-key-material fault,
    // never a network call.
    let signing_key = jwt_bearer::JwtBearerSigningKey {
        algorithm: jsonwebtoken::Algorithm::RS256,
        encoding_key: jsonwebtoken::EncodingKey::from_secret(b"not-an-rsa-key"),
    };
    let assertion = jwt_bearer::JwtBearerAssertion {
        issuer: "svc".to_owned(),
        subject: "svc".to_owned(),
        audience: "https://mock.test/token".to_owned(),
        key_id: None,
    };

    let err = jwt_bearer::execute(
        &p,
        h.http.as_ref(),
        &assertion,
        &signing_key,
        chrono::Duration::minutes(5),
        &[],
    )
    .await
    .unwrap_err();
    assert!(matches!(
        err,
        OAuthProtocolError::ProviderConfiguration { .. }
    ));
    assert_eq!(h.http.request_count(), 0);
}

// --- EndpointOverrides -----------------------------------------------------

#[cfg(feature = "oauth-google")]
mod endpoint_overrides {
    //! Proves `EndpointOverrides` actually redirects a *real* provider's
    //! `token_endpoint()` -- not just that the field compiles with
    //! `::default()` everywhere else in this suite.

    use std::sync::Arc;

    use magnetar::oauth::EndpointOverrides;
    use magnetar::oauth::grants::client_credentials;
    use magnetar::plugins::oauth_google::{GoogleOAuthProvider, GoogleProviderConfig};
    use secrecy::SecretString;

    use crate::grants_harness;

    #[tokio::test]
    async fn overridden_token_endpoint_reaches_the_grant_executor_not_the_real_google_url() {
        let h = grants_harness::harness().await;
        let provider = GoogleOAuthProvider::new(
            GoogleProviderConfig {
                client_id: "g".to_owned(),
                client_secret: SecretString::from("gs".to_owned()),
                redirect_uri: None,
                scopes: Vec::new(),
                endpoints: EndpointOverrides {
                    token_endpoint: Some("https://fake-google.test/o/token".to_owned()),
                    ..EndpointOverrides::default()
                },
            },
            Arc::new(grants_harness::RecordingRevocationTransport::default()),
        );
        h.http
            .push_json(200, r#"{"access_token":"AT","token_type":"Bearer"}"#);

        client_credentials::execute(&provider, h.http.as_ref(), &[])
            .await
            .expect("client credentials succeeds against the overridden endpoint");

        assert_eq!(
            h.http.last_request().url,
            "https://fake-google.test/o/token"
        );
    }
}
