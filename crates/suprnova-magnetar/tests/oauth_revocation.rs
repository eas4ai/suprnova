//! RFC 7009 revocation orchestration suite (Task 4).

#![cfg(all(feature = "oauth", feature = "seaorm-sqlite"))]

#[path = "fixtures/grants_harness.rs"]
mod grants_harness;
#[path = "fixtures/oauth_harness.rs"]
mod oauth_harness;
#[path = "fixtures/storage_schema.rs"]
mod storage_schema;

use std::sync::Arc;

use magnetar::oauth::grants::revocation;
use magnetar::oauth::{OAuthProtocolError, ParamPlacement, TokenHint};

use grants_harness::{MockOAuthProvider, RecordingRevocationTransport};

#[tokio::test]
async fn empty_token_is_rejected_before_reaching_the_provider() {
    let transport = Arc::new(RecordingRevocationTransport::default());
    let provider = MockOAuthProvider::new("mock", "https://mock.test/token", transport.clone());

    let err = revocation::execute(&provider, "", TokenHint::Access)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        OAuthProtocolError::InvalidRequestShape { .. }
    ));
    assert_eq!(transport.requests.lock().len(), 0);
}

#[tokio::test]
async fn happy_path_sends_body_placed_request_with_hint() {
    let transport = Arc::new(RecordingRevocationTransport::default());
    let provider = MockOAuthProvider::new("mock", "https://mock.test/token", transport.clone());

    revocation::execute(&provider, "tok-1", TokenHint::Refresh)
        .await
        .expect("revocation succeeds");

    let sent = transport.last();
    assert_eq!(sent.placement, ParamPlacement::Body);
    assert!(
        sent.params
            .iter()
            .any(|(k, v)| k == "token" && v == "tok-1")
    );
    assert!(
        sent.params
            .iter()
            .any(|(k, v)| k == "token_type_hint" && v == "refresh_token")
    );
}

#[tokio::test]
async fn unsupported_provider_surfaces_documented_posture_not_panic() {
    let transport = Arc::new(RecordingRevocationTransport::default());
    let mut provider = MockOAuthProvider::new("mock", "https://mock.test/token", transport.clone());
    provider.endpoints.revocation_endpoint = None;

    let err = revocation::execute(&provider, "tok-1", TokenHint::Access)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        OAuthProtocolError::ProviderConfiguration { .. }
    ));
    assert_eq!(transport.requests.lock().len(), 0);
}

#[tokio::test]
async fn provider_failure_propagates_and_is_never_treated_as_success() {
    let transport = Arc::new(RecordingRevocationTransport::default());
    transport.fail(OAuthProtocolError::UpstreamUnavailable {
        provider: "mock",
        message: "revocation endpoint returned 503".to_owned(),
        retry_after_seconds: None,
    });
    let provider = MockOAuthProvider::new("mock", "https://mock.test/token", transport.clone());

    let err = revocation::execute(&provider, "tok-1", TokenHint::Access)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        OAuthProtocolError::UpstreamUnavailable { .. }
    ));
}

// --- Real first-party provider dossiers: Body vs Query placement -----------

#[cfg(feature = "oauth-google")]
mod google_dossier {
    use std::sync::Arc;

    use magnetar::oauth::grants::revocation;
    use magnetar::oauth::{EndpointOverrides, ParamPlacement, TokenHint};
    use magnetar::plugins::oauth_google::{GoogleOAuthProvider, GoogleProviderConfig};
    use secrecy::SecretString;

    use crate::grants_harness::RecordingRevocationTransport;

    #[tokio::test]
    async fn revocation_places_token_in_body_with_no_hint_param() {
        let transport = Arc::new(RecordingRevocationTransport::default());
        let provider = GoogleOAuthProvider::new(
            GoogleProviderConfig {
                client_id: "g".to_owned(),
                client_secret: SecretString::from("gs".to_owned()),
                redirect_uri: None,
                scopes: Vec::new(),
                endpoints: EndpointOverrides::default(),
            },
            transport.clone(),
        );
        revocation::execute(&provider, "gtok", TokenHint::Access)
            .await
            .unwrap();
        let sent = transport.last();
        assert_eq!(sent.placement, ParamPlacement::Body);
        assert_eq!(sent.endpoint, "https://oauth2.googleapis.com/revoke");
        assert!(!sent.params.iter().any(|(k, _)| k == "token_type_hint"));
    }
}

#[cfg(feature = "oauth-facebook")]
mod facebook_dossier {
    use std::sync::Arc;

    use magnetar::oauth::grants::revocation;
    use magnetar::oauth::{ParamPlacement, TokenHint};
    use magnetar::plugins::oauth_facebook::{FacebookOAuthProvider, FacebookProviderConfig};
    use secrecy::SecretString;

    use crate::grants_harness::RecordingRevocationTransport;

    #[tokio::test]
    async fn revocation_places_token_as_a_query_parameter_on_a_delete() {
        let transport = Arc::new(RecordingRevocationTransport::default());
        let provider = FacebookOAuthProvider::new(
            FacebookProviderConfig {
                client_id: "f".to_owned(),
                client_secret: SecretString::from("fs".to_owned()),
                ..FacebookProviderConfig::default()
            },
            transport.clone(),
        );
        revocation::execute(&provider, "ftok", TokenHint::Access)
            .await
            .unwrap();
        let sent = transport.last();
        assert_eq!(sent.method, "DELETE");
        assert_eq!(sent.placement, ParamPlacement::Query);
        assert!(sent.endpoint.ends_with("/me/permissions"));
        assert!(
            sent.params
                .iter()
                .any(|(k, v)| k == "access_token" && v == "ftok")
        );
    }
}

#[cfg(feature = "oauth-x")]
mod x_dossier {
    use std::sync::Arc;

    use magnetar::oauth::grants::revocation;
    use magnetar::oauth::{EndpointOverrides, ParamPlacement, TokenHint};
    use magnetar::plugins::oauth_x::{XOAuthProvider, XProviderConfig};
    use secrecy::SecretString;

    use crate::grants_harness::RecordingRevocationTransport;

    #[tokio::test]
    async fn revocation_authenticates_with_http_basic_and_sends_hint() {
        let transport = Arc::new(RecordingRevocationTransport::default());
        let provider = XOAuthProvider::new(
            XProviderConfig {
                client_id: "x".to_owned(),
                client_secret: SecretString::from("xs".to_owned()),
                redirect_uri: None,
                scopes: Vec::new(),
                endpoints: EndpointOverrides::default(),
            },
            transport.clone(),
        );
        revocation::execute(&provider, "xtok", TokenHint::Refresh)
            .await
            .unwrap();
        let sent = transport.last();
        assert_eq!(sent.placement, ParamPlacement::Body);
        assert!(
            sent.headers
                .iter()
                .any(|(k, v)| k == "Authorization" && v.starts_with("Basic "))
        );
        assert!(
            sent.params
                .iter()
                .any(|(k, v)| k == "token_type_hint" && v == "refresh_token")
        );
    }
}

#[cfg(feature = "oauth-apple")]
mod apple_dossier {
    use std::sync::Arc;

    use async_trait::async_trait;
    use magnetar::oauth::grants::revocation;
    use magnetar::oauth::{EndpointOverrides, OAuthResult, ParamPlacement, TokenHint};
    use magnetar::plugins::oauth_apple::{
        AppleClaims, AppleOAuthProvider, AppleProviderConfig, ApplePublicKeySource,
    };
    use secrecy::SecretString;

    use crate::grants_harness::RecordingRevocationTransport;

    // A freshly generated, throwaway PKCS8 PEM EC P-256 key -- not a real
    // Apple key, only valid enough for `AppleOAuthProvider::new` to parse
    // and sign a client-secret JWT with.
    const TEST_PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----\n\
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgHd1S3YpMhdn1IHhF\n\
3dPpgMmp/x0etTS2KsDuwrGH0EyhRANCAASyhjIGqz4Sq/VCMoUgmwlT9IFLqzMa\n\
LpP4p10EVUWWBVAOIsYmWnp9niNIRB5b9IHDbj5RA0yfK3ORoMIW3IoQ\n\
-----END PRIVATE KEY-----\n";

    struct UnusedKeySource;
    #[async_trait]
    impl ApplePublicKeySource for UnusedKeySource {
        async fn verify(
            &self,
            _id_token: &str,
            _audience: &str,
            _nonce: Option<&str>,
        ) -> OAuthResult<AppleClaims> {
            unimplemented!("revocation never resolves identity")
        }
    }

    #[tokio::test]
    async fn revocation_signs_a_fresh_client_secret_and_places_everything_in_body() {
        let transport = Arc::new(RecordingRevocationTransport::default());
        let provider = AppleOAuthProvider::new(
            AppleProviderConfig {
                client_id: "com.example.app".to_owned(),
                team_id: "TEAMID1234".to_owned(),
                key_id: "KEYID6789A".to_owned(),
                private_key_pem: SecretString::from(TEST_PRIVATE_KEY_PEM.to_owned()),
                redirect_uri: None,
                scopes: Vec::new(),
                endpoints: EndpointOverrides::default(),
            },
            Arc::new(UnusedKeySource),
            transport.clone(),
        )
        .expect("valid pkcs8 pem constructs a provider");

        revocation::execute(&provider, "atok", TokenHint::Refresh)
            .await
            .unwrap();
        let sent = transport.last();
        assert_eq!(sent.placement, ParamPlacement::Body);
        assert!(
            sent.params
                .iter()
                .any(|(k, v)| k == "client_id" && v == "com.example.app")
        );
        assert!(
            sent.params
                .iter()
                .any(|(k, v)| k == "token_type_hint" && v == "refresh_token")
        );
        // A fresh signed ES256 JWT client secret, not a static value.
        let secret = sent
            .params
            .iter()
            .find(|(k, _)| k == "client_secret")
            .map(|(_, v)| v.clone())
            .expect("client_secret present");
        assert_eq!(secret.split('.').count(), 3, "compact jwt has 3 segments");
    }
}

#[cfg(feature = "oauth-tiktok")]
mod tiktok_dossier {
    use std::sync::Arc;

    use magnetar::oauth::grants::revocation;
    use magnetar::oauth::{EndpointOverrides, ParamPlacement, TokenHint};
    use magnetar::plugins::oauth_tiktok::{TikTokOAuthProvider, TikTokProviderConfig};
    use secrecy::SecretString;

    use crate::grants_harness::RecordingRevocationTransport;

    #[tokio::test]
    async fn revocation_uses_client_key_naming_and_has_no_hint_param() {
        let transport = Arc::new(RecordingRevocationTransport::default());
        let provider = TikTokOAuthProvider::new(
            TikTokProviderConfig {
                client_id: "tt-client-key".to_owned(),
                client_secret: SecretString::from("tts".to_owned()),
                redirect_uri: None,
                scopes: Vec::new(),
                endpoints: EndpointOverrides::default(),
            },
            transport.clone(),
        );
        revocation::execute(&provider, "ttok", TokenHint::Access)
            .await
            .unwrap();
        let sent = transport.last();
        assert_eq!(sent.placement, ParamPlacement::Body);
        assert!(sent.endpoint.ends_with("/oauth/revoke/"));
        assert!(
            sent.params
                .iter()
                .any(|(k, v)| k == "client_key" && v == "tt-client-key")
        );
        assert!(!sent.params.iter().any(|(k, _)| k == "token_type_hint"));
    }
}
