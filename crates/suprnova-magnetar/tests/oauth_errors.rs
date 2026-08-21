#![cfg(feature = "oauth")]

use magnetar::oauth::{OAuthErrorClass, OAuthProtocolError};

#[test]
fn every_oauth_error_maps_to_one_http_class() {
    let cases = [
        (
            OAuthProtocolError::InvalidRequestShape {
                field: "state".to_owned(),
                message: "missing verifier".to_owned(),
            },
            OAuthErrorClass::ClientError,
            400,
        ),
        (
            OAuthProtocolError::MalformedTokenResponse {
                message: "unexpected provider body".to_owned(),
            },
            OAuthErrorClass::ClientError,
            400,
        ),
        (
            OAuthProtocolError::MalformedProviderResponse {
                provider: "google",
                message: "missing sub".to_owned(),
            },
            OAuthErrorClass::ClientError,
            400,
        ),
        (
            OAuthProtocolError::ProviderReportedError {
                provider: "tiktok",
                code: "invalid_grant".to_owned(),
                message: Some("provider detail".to_owned()),
            },
            OAuthErrorClass::ClientError,
            400,
        ),
        (
            OAuthProtocolError::IdentityVerificationFailed {
                provider: "apple",
                reason: "bad signature".to_owned(),
            },
            OAuthErrorClass::IdentityError,
            401,
        ),
        (
            OAuthProtocolError::UpstreamUnavailable {
                provider: "google",
                message: "JWKS timeout".to_owned(),
                retry_after_seconds: Some(30),
            },
            OAuthErrorClass::UpstreamError,
            502,
        ),
        (
            OAuthProtocolError::ProviderConfiguration {
                provider: "apple",
                message: "missing signing key".to_owned(),
            },
            OAuthErrorClass::ServerError,
            500,
        ),
    ];

    for (error, expected_class, expected_status) in cases {
        assert_eq!(error.class(), expected_class);
        assert_eq!(error.class().status(), expected_status);
    }
}

#[test]
fn provider_is_available_only_for_provider_scoped_errors() {
    let local = OAuthProtocolError::InvalidRequestShape {
        field: "state".to_owned(),
        message: "secret-verifier".to_owned(),
    };
    let malformed = OAuthProtocolError::MalformedTokenResponse {
        message: "secret-body".to_owned(),
    };
    let provider = OAuthProtocolError::ProviderConfiguration {
        provider: "apple",
        message: "secret-key".to_owned(),
    };

    assert_eq!(local.provider(), None);
    assert_eq!(malformed.provider(), None);
    assert_eq!(provider.provider(), Some("apple"));
}
