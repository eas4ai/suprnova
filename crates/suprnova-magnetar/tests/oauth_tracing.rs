#![cfg(feature = "oauth")]

use magnetar::oauth::{OAuthErrorClass, OAuthProtocolError};

#[test]
fn display_and_debug_redact_secret_error_details() {
    let secrets = [
        "raw-access-token",
        "raw-refresh-token",
        "authorization-code",
        "pkce-verifier",
        "client-secret",
        "provider-response-body",
    ];
    let errors = [
        OAuthProtocolError::InvalidRequestShape {
            field: "code_verifier".to_owned(),
            message: secrets[3].to_owned(),
        },
        OAuthProtocolError::MalformedTokenResponse {
            message: secrets[5].to_owned(),
        },
        OAuthProtocolError::MalformedProviderResponse {
            provider: "google",
            message: secrets[5].to_owned(),
        },
        OAuthProtocolError::ProviderReportedError {
            provider: "tiktok",
            code: "invalid_grant".to_owned(),
            message: Some(secrets[5].to_owned()),
        },
        OAuthProtocolError::IdentityVerificationFailed {
            provider: "apple",
            reason: secrets[3].to_owned(),
        },
        OAuthProtocolError::UpstreamUnavailable {
            provider: "google",
            message: secrets[5].to_owned(),
            retry_after_seconds: Some(15),
        },
        OAuthProtocolError::ProviderConfiguration {
            provider: "apple",
            message: secrets[4].to_owned(),
        },
    ];

    for error in errors {
        let display = error.to_string();
        let debug = format!("{error:?}");
        for secret in secrets {
            assert!(
                !display.contains(secret),
                "Display leaked {secret}: {display}"
            );
            assert!(!debug.contains(secret), "Debug leaked {secret}: {debug}");
        }
    }
}

#[test]
fn trace_context_contains_only_non_secret_structured_fields() {
    let error = OAuthProtocolError::UpstreamUnavailable {
        provider: "google",
        message: "raw-provider-response-body".to_owned(),
        retry_after_seconds: Some(42),
    };
    let context = error.trace_context("authorization_code", "oauth.authorization", "corr-123");
    let debug = format!("{context:?}");

    assert_eq!(context.class, OAuthErrorClass::UpstreamError);
    assert_eq!(context.provider, Some("google"));
    assert_eq!(context.grant, "authorization_code");
    assert_eq!(context.ceremony_kind, "oauth.authorization");
    assert_eq!(context.correlation_id, "corr-123");
    assert!(!debug.contains("raw-provider-response-body"));
    assert!(!debug.contains("access-token"));
    assert!(!debug.contains("refresh-token"));
    assert!(!debug.contains("client-secret"));
}
