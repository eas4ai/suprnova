//! I/O-free RFC 6749 §5 token-endpoint wire type tests: success parsing,
//! error-code classification, the `#[serde(other)]` fallback for
//! unregistered codes, and malformed-body rejection.

#![cfg(feature = "oauth")]

use magnetar::oauth::{
    OAuthErrorCode, OAuthProtocolError, TokenResponseBody, TokenSuccessResponse,
};
use secrecy::ExposeSecret;

fn parse(body: &str) -> TokenResponseBody {
    magnetar::oauth::parse_token_response_body(body).expect("body should parse")
}

#[test]
fn success_body_parses_into_success_variant() {
    let body = r#"{
        "access_token": "at_live_abc123",
        "token_type": "Bearer",
        "expires_in": 3600,
        "refresh_token": "rt_live_xyz789",
        "scope": "openid profile"
    }"#;

    match parse(body) {
        TokenResponseBody::Success(TokenSuccessResponse {
            access_token,
            token_type,
            expires_in,
            refresh_token,
            id_token,
            scope,
        }) => {
            assert_eq!(access_token.expose_secret(), "at_live_abc123");
            assert_eq!(token_type, "Bearer");
            assert_eq!(expires_in, Some(3600));
            assert_eq!(
                refresh_token.as_ref().map(ExposeSecret::expose_secret),
                Some("rt_live_xyz789")
            );
            assert!(id_token.is_none());
            assert_eq!(scope.as_deref(), Some("openid profile"));
        }
        TokenResponseBody::Error(error) => panic!("expected success body, got error: {error:?}"),
    }
}

#[test]
fn success_body_preserves_optional_apple_id_token_without_debug_exposure() {
    let apple_id_token = "apple-id-token-live-value";
    let body = format!(
        r#"{{"access_token":"access-token-live-value","token_type":"Bearer","id_token":"{apple_id_token}"}}"#
    );

    match parse(&body) {
        TokenResponseBody::Success(success) => {
            assert_eq!(
                success.id_token.as_ref().map(ExposeSecret::expose_secret),
                Some(apple_id_token)
            );
            let debugged = format!("{success:?}");
            assert!(!debugged.contains(apple_id_token), "{debugged}");
            assert!(!debugged.contains("access-token-live-value"), "{debugged}");
        }
        TokenResponseBody::Error(error) => panic!("expected success body, got error: {error:?}"),
    }
}

#[test]
fn success_body_without_optional_fields_still_parses() {
    let body = r#"{"access_token": "at_minimal", "token_type": "Bearer"}"#;

    match parse(body) {
        TokenResponseBody::Success(success) => {
            assert_eq!(success.access_token.expose_secret(), "at_minimal");
            assert_eq!(success.expires_in, None);
            assert!(success.refresh_token.is_none());
            assert!(success.scope.is_none());
        }
        TokenResponseBody::Error(error) => panic!("expected success body, got error: {error:?}"),
    }
}

#[test]
fn standard_error_code_classifies_correctly() {
    let body = r#"{"error": "invalid_grant", "error_description": "code already used"}"#;

    match parse(body) {
        TokenResponseBody::Error(error) => {
            assert_eq!(error.error, OAuthErrorCode::InvalidGrant);
            assert_eq!(
                error.error_description.as_deref(),
                Some("code already used")
            );
            assert!(error.error_uri.is_none());
        }
        TokenResponseBody::Success(_) => panic!("expected error body, got success"),
    }
}

#[test]
fn every_rfc6749_section_5_2_error_code_round_trips() {
    let cases = [
        ("invalid_request", OAuthErrorCode::InvalidRequest),
        ("invalid_client", OAuthErrorCode::InvalidClient),
        ("invalid_grant", OAuthErrorCode::InvalidGrant),
        ("unauthorized_client", OAuthErrorCode::UnauthorizedClient),
        (
            "unsupported_grant_type",
            OAuthErrorCode::UnsupportedGrantType,
        ),
        ("invalid_scope", OAuthErrorCode::InvalidScope),
    ];

    for (wire, expected) in cases {
        let body = format!(r#"{{"error": "{wire}"}}"#);
        match parse(&body) {
            TokenResponseBody::Error(error) => {
                assert_eq!(error.error, expected, "for wire code {wire}")
            }
            TokenResponseBody::Success(_) => panic!("expected error body for {wire}"),
        }
    }
}

#[test]
fn unregistered_error_code_falls_back_to_unknown_preserving_the_raw_code() {
    let body = r#"{"error": "some_future_extension_code"}"#;

    match parse(body) {
        TokenResponseBody::Error(error) => {
            assert_eq!(
                error.error,
                OAuthErrorCode::Unknown("some_future_extension_code".to_owned())
            );
        }
        TokenResponseBody::Success(_) => panic!("expected error body"),
    }
}

#[test]
fn malformed_body_is_rejected_as_malformed_token_response() {
    let outcome = magnetar::oauth::parse_token_response_body("not json at all");

    match outcome {
        Err(OAuthProtocolError::MalformedTokenResponse { message }) => {
            assert!(!message.is_empty());
        }
        Err(other) => panic!("expected MalformedTokenResponse, got {other:?}"),
        Ok(body) => panic!("expected parse failure, got {body:?}"),
    }
}

#[test]
fn body_matching_neither_shape_is_rejected() {
    // Neither an `access_token`/`token_type` pair nor an `error` field.
    let outcome = magnetar::oauth::parse_token_response_body(r#"{"unrelated": "field"}"#);

    assert!(matches!(
        outcome,
        Err(OAuthProtocolError::MalformedTokenResponse { .. })
    ));
}

#[test]
fn success_body_with_unknown_extra_fields_still_parses() {
    // TikTok-style success bodies carry extra fields this crate does not
    // model (`open_id`, `refresh_expires_in`); unknown fields must not
    // block a successful parse.
    let body = r#"{
        "access_token": "at_live_abc123",
        "token_type": "Bearer",
        "open_id": "tt_user_123",
        "refresh_expires_in": 86400
    }"#;

    match parse(body) {
        TokenResponseBody::Success(success) => {
            assert_eq!(success.access_token.expose_secret(), "at_live_abc123");
        }
        TokenResponseBody::Error(error) => panic!("expected success body, got error: {error:?}"),
    }
}

#[test]
fn empty_error_field_does_not_discard_an_issued_token() {
    // A body carrying both a valid access token and a benign/empty
    // `error` field must resolve to `Success`, not silently discard the
    // token behind a spurious empty error classification.
    let body = r#"{"access_token": "at_live_abc123", "token_type": "Bearer", "error": ""}"#;

    match parse(body) {
        TokenResponseBody::Success(success) => {
            assert_eq!(success.access_token.expose_secret(), "at_live_abc123");
        }
        TokenResponseBody::Error(error) => panic!("expected success body, got error: {error:?}"),
    }
}

#[test]
fn whitespace_only_error_value_is_rejected_as_an_error_code() {
    let outcome = magnetar::oauth::parse_token_response_body(r#"{"error": "   "}"#);

    assert!(matches!(
        outcome,
        Err(OAuthProtocolError::MalformedTokenResponse { .. })
    ));
}
