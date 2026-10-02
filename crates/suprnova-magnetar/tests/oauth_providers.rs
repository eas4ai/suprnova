//! `OAuthProvider` fixture tests
//! (`docs/specs/suprnova-magnetar/10-providers.md`'s five first-party
//! dossiers and their acceptance criteria).
//!
//! Every case here is offline: no provider is ever reached over the
//! network. Authorization/token requests are proven byte-identical to each
//! dossier by rendering through the same `render_authorization_request`/
//! `render_token_request` pair Task 1's `tests/oauth_request_shapes.rs`
//! exercises; identity extraction is proven against provider response
//! fixtures (userinfo/graph JSON bodies, or -- for Apple -- a locally
//! signed ID token plus a fake JWKS verifier); revocation is proven against
//! a recording [`RevocationTransport`] fake that never performs I/O.
//!
//! Each provider's tests live in their own `#[cfg(feature = "oauth-<x>")]`
//! module so `cargo check --features oauth-<x> --all-targets` compiles this
//! file with every other provider's section stripped out, matching the
//! fail-closed per-feature CI matrix. Facebook's and X's sections also
//! carry an end-to-end case (`feature = "seaorm-sqlite"` additionally
//! required) that feeds the provider's own `resolve_identity` output
//! through the real `IdentityResolver` from Task 2's harness, proving
//! spec 10's "withheld-email path exercises 09's email-completion outcome
//! end-to-end" (Facebook) and "sign-up via email-completion is X's
//! happy-path test" (X) acceptance criteria -- not merely that this
//! provider's own return value has the right shape in isolation.

#![cfg(feature = "oauth")]

#[cfg(feature = "seaorm-sqlite")]
#[path = "fixtures/oauth_harness.rs"]
mod oauth_harness;
#[cfg(feature = "seaorm-sqlite")]
#[path = "fixtures/storage_schema.rs"]
mod storage_schema;

#[cfg(any(
    feature = "oauth-apple",
    feature = "oauth-google",
    feature = "oauth-facebook",
    feature = "oauth-x",
    feature = "oauth-tiktok"
))]
use std::sync::{Arc, Mutex};

#[cfg(any(
    feature = "oauth-apple",
    feature = "oauth-google",
    feature = "oauth-facebook",
    feature = "oauth-x",
    feature = "oauth-tiktok"
))]
use async_trait::async_trait;
#[cfg(any(
    feature = "oauth-apple",
    feature = "oauth-facebook",
    feature = "oauth-x",
    feature = "oauth-tiktok"
))]
use magnetar::oauth::AuthorizationRequestParams;
#[cfg(any(feature = "oauth-facebook", feature = "oauth-x"))]
use magnetar::oauth::AuthorizationRequestShape;
#[cfg(any(
    feature = "oauth-apple",
    feature = "oauth-facebook",
    feature = "oauth-x",
    feature = "oauth-tiktok"
))]
use magnetar::oauth::render_authorization_request;
#[cfg(any(
    feature = "oauth-apple",
    feature = "oauth-google",
    feature = "oauth-facebook",
    feature = "oauth-x",
    feature = "oauth-tiktok"
))]
use magnetar::oauth::{
    EndpointOverrides, OAuthResult, ParamPlacement, ProviderResponse, RevocationRequest,
    RevocationTransport, TokenHint,
};
#[cfg(any(feature = "oauth-x", feature = "oauth-tiktok"))]
use magnetar::oauth::{TokenRequestParams, render_token_request};

#[cfg(any(
    feature = "oauth-apple",
    feature = "oauth-facebook",
    feature = "oauth-x",
    feature = "oauth-tiktok"
))]
fn find<'a>(wire: &'a [(String, String)], key: &str) -> Option<&'a str> {
    wire.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

#[cfg(any(feature = "oauth-apple", feature = "oauth-tiktok"))]
fn has_key(wire: &[(String, String)], key: &str) -> bool {
    wire.iter().any(|(k, _)| k == key)
}

#[cfg(any(
    feature = "oauth-apple",
    feature = "oauth-google",
    feature = "oauth-facebook",
    feature = "oauth-x",
    feature = "oauth-tiktok"
))]
/// A snapshot of one call into [`RecordingRevocationTransport`], captured
/// after the request has crossed the provider -> transport boundary (the
/// only place its secret-bearing fields are allowed to exist).
#[allow(dead_code)]
struct RecordedRevocation {
    method: &'static str,
    endpoint: String,
    placement: ParamPlacement,
    params: Vec<(String, String)>,
    headers: Vec<(String, String)>,
}

#[cfg(any(
    feature = "oauth-apple",
    feature = "oauth-google",
    feature = "oauth-facebook",
    feature = "oauth-x",
    feature = "oauth-tiktok"
))]
/// A [`RevocationTransport`] fake that never performs I/O: it only records
/// what each provider rendered, so tests can assert on it directly.
#[derive(Default)]
struct RecordingRevocationTransport {
    calls: Mutex<Vec<RecordedRevocation>>,
}

#[cfg(any(
    feature = "oauth-apple",
    feature = "oauth-google",
    feature = "oauth-facebook",
    feature = "oauth-x",
    feature = "oauth-tiktok"
))]
#[async_trait]
impl RevocationTransport for RecordingRevocationTransport {
    async fn send(&self, request: RevocationRequest) -> OAuthResult<()> {
        self.calls
            .lock()
            .expect("lock poisoned")
            .push(RecordedRevocation {
                method: request.method,
                endpoint: request.endpoint,
                placement: request.placement,
                params: request.params,
                headers: request.headers,
            });
        Ok(())
    }
}

// --- Apple -------------------------------------------------------------

#[cfg(feature = "oauth-apple")]
mod apple {
    use super::*;
    use magnetar::oauth::OAuthProtocolError;
    use magnetar::oauth::OAuthProvider as _;
    use magnetar::plugins::oauth_apple::{
        AppleClaims, AppleOAuthProvider, AppleProviderConfig, ApplePublicKeySource,
    };
    use secrecy::SecretString;

    /// A throwaway ECDSA P-256 PKCS8 test key -- never a real Apple key,
    /// generated locally for this test suite only (`openssl ecparam
    /// -genkey -name prime256v1 -noout | openssl pkcs8 -topk8 -nocrypt`).
    /// This is the standard format Apple's Developer portal issues `.p8`
    /// downloads in; [`AppleOAuthProvider::new`] decodes it directly (see
    /// that constructor's doc for why it does not hand this PEM straight
    /// to `suprnova-apple-rs`).
    const TEST_PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----\n\
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg9cld72Dlc09boGa+\n\
Hoo62Cptg9VEedeF9m5qGzMdBxKhRANCAATs0RvE+uOTJbWfyY5AZat92wGjXsQU\n\
zV7lmLsegC7z6Mp+xNC89mSD5mfuBaptjAab1AT0XEIyB9mXg47uB1bM\n\
-----END PRIVATE KEY-----\n";

    /// An offline [`ApplePublicKeySource`] fake: returns a canned claim set
    /// (or a canned failure) instead of verifying anything over JWKS.
    struct FakeKeySource {
        outcome: OAuthResult<AppleClaims>,
    }

    #[async_trait]
    impl ApplePublicKeySource for FakeKeySource {
        async fn verify(
            &self,
            _id_token: &str,
            _audience: &str,
            _nonce: Option<&str>,
        ) -> OAuthResult<AppleClaims> {
            self.outcome.clone()
        }
    }

    fn config() -> AppleProviderConfig {
        AppleProviderConfig {
            client_id: "com.example.app".to_owned(),
            team_id: "TEAMID1234".to_owned(),
            key_id: "KEYID1234".to_owned(),
            private_key_pem: SecretString::from(TEST_PRIVATE_KEY_PEM),
            redirect_uri: Some("https://example.com/callback".to_owned()),
            scopes: vec!["name".to_owned(), "email".to_owned()],
            endpoints: EndpointOverrides::default(),
        }
    }

    fn provider(
        outcome: OAuthResult<AppleClaims>,
    ) -> (AppleOAuthProvider, Arc<RecordingRevocationTransport>) {
        let transport = Arc::new(RecordingRevocationTransport::default());
        let key_source = Arc::new(FakeKeySource { outcome });
        let provider = AppleOAuthProvider::new(config(), key_source, transport.clone())
            .expect("test key parses");
        (provider, transport)
    }

    #[test]
    fn authorization_shape_disables_pkce_and_forces_form_post() {
        let (provider, _transport) = provider(Ok(AppleClaims {
            subject: "s".to_owned(),
            email: None,
            email_verified: false,
            is_private_email: false,
        }));
        let shape = provider.authorization_shape();
        let params = AuthorizationRequestParams {
            client_id: "com.example.app".to_owned(),
            redirect_uri: Some("https://example.com/callback".to_owned()),
            scopes: vec!["name".to_owned(), "email".to_owned()],
            state: Some("state123".to_owned()),
            code_challenge: None,
            nonce: Some("nonce123".to_owned()),
        };
        let wire = render_authorization_request(&shape, &params).expect("no PKCE required");
        assert!(!has_key(&wire, "code_challenge"));
        assert!(!has_key(&wire, "code_challenge_method"));
        assert_eq!(find(&wire, "response_mode"), Some("form_post"));
        assert_eq!(find(&wire, "client_id"), Some("com.example.app"));
        assert_eq!(find(&wire, "nonce"), Some("nonce123"));
    }

    #[test]
    fn authorization_shape_requires_nonce() {
        let (provider, _transport) = provider(Ok(AppleClaims {
            subject: "s".to_owned(),
            email: None,
            email_verified: false,
            is_private_email: false,
        }));
        let shape = provider.authorization_shape();
        assert!(shape.requires_nonce);
        let params = AuthorizationRequestParams {
            client_id: "com.example.app".to_owned(),
            redirect_uri: Some("https://example.com/callback".to_owned()),
            scopes: vec!["name".to_owned(), "email".to_owned()],
            state: Some("state123".to_owned()),
            code_challenge: None,
            nonce: None,
        };
        let error = render_authorization_request(&shape, &params).unwrap_err();
        assert!(matches!(
            error,
            magnetar::oauth::OAuthProtocolError::InvalidRequestShape { ref field, .. }
                if field == "nonce"
        ));
    }

    #[test]
    fn token_shape_is_rfc_default() {
        let (provider, _transport) = provider(Ok(AppleClaims {
            subject: "s".to_owned(),
            email: None,
            email_verified: false,
            is_private_email: false,
        }));
        let shape = provider.token_shape();
        assert_eq!(shape.client_id_param, "client_id");
        assert!(!shape.accept_http_success_error_body);
    }

    /// Unlike the canned `FakeKeySource` above, this fake actually checks
    /// the passed nonce against `expected` -- proving
    /// `resolve_identity` rejects a nonce mismatch (rather than merely
    /// forwarding whatever the JWKS-verification seam returns) and
    /// accepts a match. I6: with PKCE disabled, this is Apple's only
    /// replay/injection defense.
    struct NonceCheckingKeySource {
        expected: String,
        claims: AppleClaims,
    }

    #[async_trait]
    impl ApplePublicKeySource for NonceCheckingKeySource {
        async fn verify(
            &self,
            _id_token: &str,
            _audience: &str,
            nonce: Option<&str>,
        ) -> OAuthResult<AppleClaims> {
            if nonce != Some(self.expected.as_str()) {
                return Err(OAuthProtocolError::IdentityVerificationFailed {
                    provider: "apple",
                    reason: "nonce mismatch".to_owned(),
                });
            }
            Ok(self.claims.clone())
        }
    }

    fn nonce_checking_provider(expected: &str) -> AppleOAuthProvider {
        let key_source = Arc::new(NonceCheckingKeySource {
            expected: expected.to_owned(),
            claims: AppleClaims {
                subject: "s".to_owned(),
                email: None,
                email_verified: false,
                is_private_email: false,
            },
        });
        AppleOAuthProvider::new(
            config(),
            key_source,
            Arc::new(RecordingRevocationTransport::default()),
        )
        .expect("test key parses")
    }

    #[tokio::test]
    async fn nonce_mismatch_is_rejected() {
        let provider = nonce_checking_provider("expected-nonce");
        let response = ProviderResponse::AppleIdToken {
            id_token: SecretString::from("unused"),
            nonce: Some("attacker-supplied-nonce".to_owned()),
            form_post_user: None,
        };
        let error = provider.resolve_identity(response).await.unwrap_err();
        assert!(matches!(
            error,
            OAuthProtocolError::IdentityVerificationFailed { .. }
        ));
    }

    #[tokio::test]
    async fn matching_nonce_resolves() {
        let provider = nonce_checking_provider("expected-nonce");
        let response = ProviderResponse::AppleIdToken {
            id_token: SecretString::from("unused"),
            nonce: Some("expected-nonce".to_owned()),
            form_post_user: None,
        };
        let identity = provider
            .resolve_identity(response)
            .await
            .expect("nonce matches");
        assert_eq!(identity.subject, "s");
    }

    #[tokio::test]
    async fn first_authorization_resolves_email_and_form_post_name() {
        let (provider, _transport) = provider(Ok(AppleClaims {
            subject: "001837.abc123.1234".to_owned(),
            email: Some("relay@privaterelay.appleid.com".to_owned()),
            email_verified: true,
            is_private_email: true,
        }));
        let response = ProviderResponse::AppleIdToken {
            id_token: SecretString::from("unused-by-the-fake"),
            nonce: Some("nonce123".to_owned()),
            form_post_user: Some(
                r#"{"name":{"firstName":"Ada","lastName":"Lovelace"}}"#.to_owned(),
            ),
        };
        let identity = provider.resolve_identity(response).await.expect("resolves");
        assert_eq!(identity.provider, "apple");
        assert_eq!(identity.subject, "001837.abc123.1234");
        assert_eq!(
            identity.email.as_deref(),
            Some("relay@privaterelay.appleid.com")
        );
        assert!(identity.email_verified);
        assert_eq!(identity.display_name.as_deref(), Some("Ada Lovelace"));
    }

    #[tokio::test]
    async fn repeat_authorization_has_no_name_or_email() {
        let (provider, _transport) = provider(Ok(AppleClaims {
            subject: "001837.abc123.1234".to_owned(),
            email: None,
            email_verified: false,
            is_private_email: false,
        }));
        let response = ProviderResponse::AppleIdToken {
            id_token: SecretString::from("unused-by-the-fake"),
            nonce: Some("nonce123".to_owned()),
            form_post_user: None,
        };
        let identity = provider.resolve_identity(response).await.expect("resolves");
        assert_eq!(identity.subject, "001837.abc123.1234");
        assert!(identity.email.is_none());
        assert!(identity.display_name.is_none());
    }

    #[tokio::test]
    async fn identity_verification_failure_propagates() {
        let (provider, _transport) =
            provider(Err(OAuthProtocolError::IdentityVerificationFailed {
                provider: "apple",
                reason: "signature invalid".to_owned(),
            }));
        let response = ProviderResponse::AppleIdToken {
            id_token: SecretString::from("bad"),
            nonce: Some("nonce123".to_owned()),
            form_post_user: None,
        };
        let error = provider.resolve_identity(response).await.unwrap_err();
        assert!(matches!(
            error,
            OAuthProtocolError::IdentityVerificationFailed { .. }
        ));
    }

    /// N1: with PKCE disabled, the nonce is Apple's only replay/injection
    /// defense; `suprnova-apple-rs`'s JWKS verifier skips the nonce check
    /// entirely when `None` is passed rather than failing closed, so this
    /// provider must reject an absent nonce itself -- and do so *before*
    /// ever calling the key source (a `PanicsIfCalledKeySource` proves it,
    /// not just that the final error variant matches).
    #[tokio::test]
    async fn nonce_absent_is_rejected_before_calling_key_source() {
        struct PanicsIfCalledKeySource;
        #[async_trait]
        impl ApplePublicKeySource for PanicsIfCalledKeySource {
            async fn verify(
                &self,
                _id_token: &str,
                _audience: &str,
                _nonce: Option<&str>,
            ) -> OAuthResult<AppleClaims> {
                panic!("key source must not be called when the response carries no nonce");
            }
        }
        let transport = Arc::new(RecordingRevocationTransport::default());
        let provider =
            AppleOAuthProvider::new(config(), Arc::new(PanicsIfCalledKeySource), transport)
                .expect("test key parses");
        let response = ProviderResponse::AppleIdToken {
            id_token: SecretString::from("unused"),
            nonce: None,
            form_post_user: None,
        };
        let error = provider.resolve_identity(response).await.unwrap_err();
        assert!(matches!(
            error,
            OAuthProtocolError::IdentityVerificationFailed {
                provider: "apple",
                ..
            }
        ));
    }

    #[tokio::test]
    async fn resolve_identity_rejects_userinfo_response() {
        let (provider, _transport) = provider(Ok(AppleClaims {
            subject: "s".to_owned(),
            email: None,
            email_verified: false,
            is_private_email: false,
        }));
        let error = provider
            .resolve_identity(ProviderResponse::UserInfo {
                body: "{}".to_owned(),
            })
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            OAuthProtocolError::MalformedProviderResponse { .. }
        ));
    }

    #[tokio::test]
    async fn revoke_mints_signed_jwt_client_secret_and_never_leaks_key_material() {
        let (provider, transport) = provider(Ok(AppleClaims {
            subject: "s".to_owned(),
            email: None,
            email_verified: false,
            is_private_email: false,
        }));
        provider
            .revoke("access-token-value", TokenHint::Access)
            .await
            .expect("revocation transport always succeeds in this fake");

        let calls = transport.calls.lock().expect("lock poisoned");
        assert_eq!(calls.len(), 1);
        let call = &calls[0];
        assert_eq!(call.method, "POST");
        assert_eq!(call.endpoint, "https://appleid.apple.com/auth/revoke");
        assert_eq!(find(&call.params, "client_id"), Some("com.example.app"));
        assert_eq!(find(&call.params, "token"), Some("access-token-value"));
        assert_eq!(find(&call.params, "token_type_hint"), Some("access_token"));
        let client_secret = find(&call.params, "client_secret").expect("client_secret present");

        // Decode the header and verify the ES256 signature against the
        // test key's own public half -- not merely "it has two dots".
        let header =
            jsonwebtoken::decode_header(client_secret).expect("well-formed compact JWT header");
        assert_eq!(header.alg, jsonwebtoken::Algorithm::ES256);
        assert_eq!(header.kid.as_deref(), Some("KEYID1234"));

        #[derive(serde::Deserialize)]
        struct AppleClientSecretClaims {
            iss: String,
            sub: String,
            aud: String,
            iat: i64,
            exp: i64,
        }

        let test_secret_key = {
            use p256::pkcs8::DecodePrivateKey as _;
            p256::SecretKey::from_pkcs8_pem(TEST_PRIVATE_KEY_PEM).expect("test key parses")
        };
        let public_key_pem = {
            use p256::pkcs8::EncodePublicKey as _;
            test_secret_key
                .public_key()
                .to_public_key_pem(p256::pkcs8::LineEnding::LF)
                .expect("public key encodes")
        };
        let decoding_key = jsonwebtoken::DecodingKey::from_ec_pem(public_key_pem.as_bytes())
            .expect("valid EC public key PEM");
        let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::ES256);
        validation.set_audience(&["https://appleid.apple.com"]);
        let decoded = jsonwebtoken::decode::<AppleClientSecretClaims>(
            client_secret,
            &decoding_key,
            &validation,
        )
        .expect("signature verifies against the test key's public half");
        assert_eq!(decoded.claims.iss, "TEAMID1234");
        assert_eq!(decoded.claims.sub, "com.example.app");
        assert_eq!(decoded.claims.aud, "https://appleid.apple.com");
        assert_eq!(decoded.claims.exp - decoded.claims.iat, 300);

        // Key material never appears in a Debug-formatted config.
        let debug = format!("{:?}", config());
        assert!(!debug.contains("BEGIN PRIVATE KEY"));
        assert!(!debug.contains("MIGHAgEAMBMGByqGSM49AgEG"));
    }

    #[test]
    fn refresh_policy_uses_signed_jwt_authentication() {
        let (provider, _transport) = provider(Ok(AppleClaims {
            subject: "s".to_owned(),
            email: None,
            email_verified: false,
            is_private_email: false,
        }));
        let policy = provider.refresh_policy();
        assert!(policy.supported);
        assert_eq!(
            policy.token_client_authentication,
            magnetar::oauth::ClientAuthentication::SignedJwt
        );
    }
}

#[cfg(all(feature = "oauth-apple", feature = "seaorm-sqlite"))]
mod apple_end_to_end {
    use std::sync::Arc;

    use async_trait::async_trait;
    use magnetar::oauth::{
        AutoLinkPolicy, EndpointOverrides, IdentityOutcome, IdentityResolver, OAuthIntent,
        OAuthProvider as _, OAuthResult, ProviderResponse,
    };
    use magnetar::plugins::oauth_apple::{
        AppleClaims, AppleOAuthProvider, AppleProviderConfig, ApplePublicKeySource,
    };
    use magnetar::sessions::SessionMetadata;
    use secrecy::SecretString;

    use super::RecordingRevocationTransport;
    use crate::oauth_harness;

    const TEST_PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----\n\
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg9cld72Dlc09boGa+\n\
Hoo62Cptg9VEedeF9m5qGzMdBxKhRANCAATs0RvE+uOTJbWfyY5AZat92wGjXsQU\n\
zV7lmLsegC7z6Mp+xNC89mSD5mfuBaptjAab1AT0XEIyB9mXg47uB1bM\n\
-----END PRIVATE KEY-----\n";

    /// Returns a fixed claim set every call, ignoring the id_token content
    /// (the token content is irrelevant here -- the point is proving the
    /// *resolver's* stored-identity sign-in, not JWKS verification, which
    /// the `apple` module's suite already covers).
    struct FakeKeySource {
        claims: AppleClaims,
    }

    #[async_trait]
    impl ApplePublicKeySource for FakeKeySource {
        async fn verify(
            &self,
            _id_token: &str,
            _audience: &str,
            _nonce: Option<&str>,
        ) -> OAuthResult<AppleClaims> {
            Ok(self.claims.clone())
        }
    }

    fn resolver(h: &oauth_harness::OAuthHarness) -> IdentityResolver {
        IdentityResolver::new(
            h.storage.clone(),
            h.storage.clone(),
            h.storage.clone(),
            h.first_proof.clone(),
            h.encryptor.clone(),
            AutoLinkPolicy::ExplicitLinkRequired,
        )
    }

    fn provider(claims: AppleClaims) -> AppleOAuthProvider {
        AppleOAuthProvider::new(
            AppleProviderConfig {
                client_id: "com.example.app".to_owned(),
                team_id: "TEAMID1234".to_owned(),
                key_id: "KEYID1234".to_owned(),
                private_key_pem: SecretString::from(TEST_PRIVATE_KEY_PEM),
                redirect_uri: Some("https://example.com/callback".to_owned()),
                scopes: vec!["name".to_owned(), "email".to_owned()],
                endpoints: EndpointOverrides::default(),
            },
            Arc::new(FakeKeySource { claims }),
            Arc::new(RecordingRevocationTransport::default()),
        )
        .expect("test key parses")
    }

    /// Spec 10's Apple acceptance criterion: "second authorization for the
    /// same Apple user (no name/email in payload) signs in cleanly against
    /// the stored identity." First authorization carries email + a
    /// form_post `user` payload (name); second authorization carries
    /// neither, per Apple's first-auth-only quirk -- both must resolve to
    /// the same stored `(provider, subject)` through a real
    /// `IdentityResolver`.
    #[tokio::test]
    async fn second_authorization_signs_in_against_stored_identity() {
        let h = oauth_harness::harness().await;

        let first = provider(AppleClaims {
            subject: "001837.abc123.1234".to_owned(),
            email: Some("user@example.com".to_owned()),
            email_verified: true,
            is_private_email: false,
        });
        let first_identity = first
            .resolve_identity(ProviderResponse::AppleIdToken {
                id_token: SecretString::from("unused"),
                nonce: Some("nonce-first".to_owned()),
                form_post_user: Some(
                    r#"{"name":{"firstName":"Ada","lastName":"Lovelace"}}"#.to_owned(),
                ),
            })
            .await
            .expect("resolves");
        assert_eq!(first_identity.display_name.as_deref(), Some("Ada Lovelace"));

        let first_outcome = resolver(&h)
            .resolve(
                first_identity,
                OAuthIntent::SignIn,
                None,
                SessionMetadata::default(),
            )
            .await
            .expect("first authorization resolves");
        let created_user_id = match first_outcome {
            IdentityOutcome::Create { user_id, .. } => user_id,
            _ => panic!("expected Create on first authorization (new verified email)"),
        };

        // Second authorization: no email, no form_post user payload.
        let second = provider(AppleClaims {
            subject: "001837.abc123.1234".to_owned(),
            email: None,
            email_verified: false,
            is_private_email: false,
        });
        let second_identity = second
            .resolve_identity(ProviderResponse::AppleIdToken {
                id_token: SecretString::from("unused"),
                nonce: Some("nonce-second".to_owned()),
                form_post_user: None,
            })
            .await
            .expect("resolves");
        assert!(second_identity.email.is_none());
        assert!(second_identity.display_name.is_none());

        let second_outcome = resolver(&h)
            .resolve(
                second_identity,
                OAuthIntent::SignIn,
                None,
                SessionMetadata::default(),
            )
            .await
            .expect("second authorization resolves cleanly against the stored identity");
        match second_outcome {
            IdentityOutcome::SignIn(principal) => {
                assert_eq!(principal.user_id(), created_user_id);
            }
            _ => panic!("expected SignIn against the stored identity on second authorization"),
        }
    }
}

// --- OAuthProviderRegistry (I4) --------------------------------------------

#[cfg(all(feature = "oauth-google", feature = "oauth-facebook"))]
mod registry {
    use std::sync::Arc;

    use magnetar::oauth::{EndpointOverrides, OAuthProtocolError, OAuthProviderRegistry};
    use magnetar::plugins::oauth_facebook::{FacebookOAuthProvider, FacebookProviderConfig};
    use magnetar::plugins::oauth_google::{GoogleOAuthProvider, GoogleProviderConfig};
    use secrecy::SecretString;

    use super::RecordingRevocationTransport;

    fn google() -> GoogleOAuthProvider {
        GoogleOAuthProvider::new(
            GoogleProviderConfig {
                client_id: "g".to_owned(),
                client_secret: SecretString::from("gs".to_owned()),
                redirect_uri: None,
                scopes: Vec::new(),
                endpoints: EndpointOverrides::default(),
            },
            Arc::new(RecordingRevocationTransport::default()),
        )
    }

    fn facebook() -> FacebookOAuthProvider {
        FacebookOAuthProvider::new(
            FacebookProviderConfig {
                client_id: "f".to_owned(),
                client_secret: SecretString::from("fs".to_owned()),
                redirect_uri: None,
                scopes: Vec::new(),
                graph_api_version: "v26.0".to_owned(),
                endpoints: EndpointOverrides::default(),
            },
            Arc::new(RecordingRevocationTransport::default()),
        )
    }

    #[test]
    fn register_get_and_names_round_trip() {
        let mut registry = OAuthProviderRegistry::new();
        registry.register(Arc::new(google())).expect("registers");
        registry.register(Arc::new(facebook())).expect("registers");

        assert!(registry.get("google").is_some());
        assert!(registry.get("facebook").is_some());
        assert!(registry.get("nonexistent").is_none());
        // Sorted for deterministic iteration.
        assert_eq!(registry.names(), vec!["facebook", "google"]);
    }

    #[test]
    fn duplicate_name_is_rejected_not_silently_clobbered() {
        let mut registry = OAuthProviderRegistry::new();
        registry.register(Arc::new(google())).expect("registers");
        let error = registry
            .register(Arc::new(google()))
            .err()
            .expect("second registration under the same name is rejected");
        assert!(matches!(
            error,
            OAuthProtocolError::ProviderConfiguration {
                provider: "google",
                ..
            }
        ));
        // The first registration is retained, not clobbered.
        assert_eq!(registry.names(), vec!["google"]);
    }
}

// --- Google --------------------------------------------------------------

#[cfg(feature = "oauth-google")]
mod google {
    use super::*;
    use magnetar::oauth::OAuthProtocolError;
    use magnetar::oauth::OAuthProvider as _;
    use magnetar::plugins::oauth_google::{GoogleOAuthProvider, GoogleProviderConfig};
    use secrecy::SecretString;

    fn provider() -> (GoogleOAuthProvider, Arc<RecordingRevocationTransport>) {
        let transport = Arc::new(RecordingRevocationTransport::default());
        let provider = GoogleOAuthProvider::new(
            GoogleProviderConfig {
                client_id: "client-123".to_owned(),
                client_secret: SecretString::from("shh".to_owned()),
                redirect_uri: Some("https://example.com/callback".to_owned()),
                scopes: vec!["openid".to_owned(), "email".to_owned()],
                endpoints: EndpointOverrides::default(),
            },
            transport.clone(),
        );
        (provider, transport)
    }

    #[test]
    fn shapes_are_rfc_default_no_quirk_handlers() {
        let (provider, _transport) = provider();
        assert_eq!(provider.authorization_shape(), Default::default());
        assert_eq!(provider.token_shape(), Default::default());
    }

    #[tokio::test]
    async fn resolves_verified_email_from_userinfo_payload() {
        let (provider, _transport) = provider();
        let body = r#"{"sub":"110169484474386276334","email":"user@example.com","email_verified":true,"name":"Ada Lovelace"}"#;
        let identity = provider
            .resolve_identity(ProviderResponse::UserInfo {
                body: body.to_owned(),
            })
            .await
            .expect("resolves");
        assert_eq!(identity.provider, "google");
        assert_eq!(identity.subject, "110169484474386276334");
        assert_eq!(identity.email.as_deref(), Some("user@example.com"));
        assert!(identity.email_verified);
        assert_eq!(identity.display_name.as_deref(), Some("Ada Lovelace"));
    }

    #[tokio::test]
    async fn unverified_email_surfaces_as_unverified_not_filtered_or_errored() {
        let (provider, _transport) = provider();
        let body =
            r#"{"sub":"110169484474386276334","email":"user@example.com","email_verified":false}"#;
        let identity = provider
            .resolve_identity(ProviderResponse::UserInfo {
                body: body.to_owned(),
            })
            .await
            .expect("resolves; verification policy is IdentityResolver's job, not the provider's");
        assert_eq!(identity.email.as_deref(), Some("user@example.com"));
        assert!(!identity.email_verified);
    }

    #[tokio::test]
    async fn missing_subject_is_a_malformed_response() {
        let (provider, _transport) = provider();
        let error = provider
            .resolve_identity(ProviderResponse::UserInfo {
                body: "{}".to_owned(),
            })
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            OAuthProtocolError::MalformedProviderResponse {
                provider: "google",
                ..
            }
        ));
    }

    #[tokio::test]
    async fn revoke_sends_bare_token_no_hint() {
        let (provider, transport) = provider();
        provider
            .revoke("tok", TokenHint::Refresh)
            .await
            .expect("succeeds");
        let calls = transport.calls.lock().expect("lock poisoned");
        let call = &calls[0];
        assert_eq!(call.method, "POST");
        assert_eq!(call.endpoint, "https://oauth2.googleapis.com/revoke");
        assert_eq!(call.params, vec![("token".to_owned(), "tok".to_owned())]);
    }

    #[test]
    fn refresh_requires_offline_access_and_reconsent() {
        let (provider, _transport) = provider();
        let policy = provider.refresh_policy();
        assert!(policy.supported);
        assert!(policy.requires_reconsent_for_reissue);
        assert!(
            policy
                .extra_authorization_params
                .contains(&("access_type".to_owned(), "offline".to_owned()))
        );
    }
}

// --- Facebook --------------------------------------------------------------

#[cfg(feature = "oauth-facebook")]
mod facebook {
    use super::*;
    use magnetar::oauth::OAuthProtocolError;
    use magnetar::oauth::OAuthProvider as _;
    use magnetar::plugins::oauth_facebook::{FacebookOAuthProvider, FacebookProviderConfig};
    use secrecy::SecretString;

    fn provider() -> (FacebookOAuthProvider, Arc<RecordingRevocationTransport>) {
        let transport = Arc::new(RecordingRevocationTransport::default());
        let provider = FacebookOAuthProvider::new(
            FacebookProviderConfig {
                client_id: "app-id".to_owned(),
                client_secret: SecretString::from("shh".to_owned()),
                redirect_uri: Some("https://example.com/callback".to_owned()),
                scopes: vec!["email".to_owned()],
                graph_api_version: "v26.0".to_owned(),
                endpoints: EndpointOverrides::default(),
            },
            transport.clone(),
        );
        (provider, transport)
    }

    #[tokio::test]
    async fn present_email_is_treated_as_verified() {
        let (provider, _transport) = provider();
        let body = r#"{"id":"10152","name":"Ada Lovelace","email":"user@example.com"}"#;
        let identity = provider
            .resolve_identity(ProviderResponse::UserInfo {
                body: body.to_owned(),
            })
            .await
            .expect("resolves");
        assert_eq!(identity.provider, "facebook");
        assert_eq!(identity.subject, "10152");
        assert_eq!(identity.email.as_deref(), Some("user@example.com"));
        assert!(identity.email_verified);
    }

    #[tokio::test]
    async fn absent_email_is_unverified_and_none() {
        let (provider, _transport) = provider();
        let body = r#"{"id":"10152","name":"Ada Lovelace"}"#;
        let identity = provider
            .resolve_identity(ProviderResponse::UserInfo {
                body: body.to_owned(),
            })
            .await
            .expect("resolves");
        assert!(identity.email.is_none());
        assert!(!identity.email_verified);
    }

    #[tokio::test]
    async fn missing_id_is_a_malformed_response() {
        let (provider, _transport) = provider();
        let error = provider
            .resolve_identity(ProviderResponse::UserInfo {
                body: "{}".to_owned(),
            })
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            OAuthProtocolError::MalformedProviderResponse {
                provider: "facebook",
                ..
            }
        ));
    }

    #[tokio::test]
    async fn revoke_deauthorizes_via_delete_with_query_placement() {
        let (provider, transport) = provider();
        provider
            .revoke("tok", TokenHint::Access)
            .await
            .expect("succeeds");
        let calls = transport.calls.lock().expect("lock poisoned");
        let call = &calls[0];
        assert_eq!(call.method, "DELETE");
        assert_eq!(
            call.endpoint,
            "https://graph.facebook.com/v26.0/me/permissions"
        );
        assert_eq!(call.placement, magnetar::oauth::ParamPlacement::Query);
        assert_eq!(find(&call.params, "access_token"), Some("tok"));
    }

    #[test]
    fn refresh_is_not_rfc6749_shaped() {
        let (provider, _transport) = provider();
        assert!(!provider.refresh_policy().supported);
    }

    #[test]
    fn authorization_shape_keeps_pkce_required_default() {
        // C1: Facebook's own reference doesn't *mention* PKCE, but that is
        // silence, not evidence of rejection -- 09's default-on posture
        // stands absent live evidence Facebook rejects it, and independent
        // evidence (Facebook's `oauth_code_verification_failed` error)
        // suggests the endpoint validates it.
        let (provider, _transport) = provider();
        let shape = provider.authorization_shape();
        assert_eq!(shape, AuthorizationRequestShape::default());
        let params = AuthorizationRequestParams {
            client_id: "app-id".to_owned(),
            redirect_uri: Some("https://example.com/callback".to_owned()),
            scopes: vec!["email".to_owned()],
            state: Some("state123".to_owned()),
            code_challenge: Some("challenge".to_owned()),
            nonce: None,
        };
        let wire = render_authorization_request(&shape, &params).expect("PKCE supplied");
        assert_eq!(find(&wire, "code_challenge"), Some("challenge"));
        assert_eq!(find(&wire, "code_challenge_method"), Some("S256"));
    }

    #[tokio::test]
    async fn client_authentication_supplies_client_secret_for_request_body() {
        let (provider, _transport) = provider();
        let material = provider
            .client_authentication()
            .await
            .expect("renders client authentication");
        assert_eq!(find(&material.params, "client_secret"), Some("shh"));
        assert!(material.headers.is_empty());
    }
}

#[cfg(all(feature = "oauth-facebook", feature = "seaorm-sqlite"))]
mod facebook_end_to_end {
    use std::sync::Arc;

    use magnetar::oauth::{
        AutoLinkPolicy, EndpointOverrides, IdentityOutcome, IdentityResolver, OAuthIntent,
        OAuthProvider as _, ProviderResponse,
    };
    use magnetar::plugins::oauth_facebook::{FacebookOAuthProvider, FacebookProviderConfig};
    use magnetar::sessions::SessionMetadata;
    use secrecy::SecretString;

    use super::RecordingRevocationTransport;
    use crate::oauth_harness;

    fn resolver(h: &oauth_harness::OAuthHarness) -> IdentityResolver {
        IdentityResolver::new(
            h.storage.clone(),
            h.storage.clone(),
            h.storage.clone(),
            h.first_proof.clone(),
            h.encryptor.clone(),
            AutoLinkPolicy::ExplicitLinkRequired,
        )
    }

    fn provider() -> FacebookOAuthProvider {
        FacebookOAuthProvider::new(
            FacebookProviderConfig {
                client_id: "app-id".to_owned(),
                client_secret: SecretString::from("shh".to_owned()),
                redirect_uri: Some("https://example.com/callback".to_owned()),
                scopes: vec!["email".to_owned()],
                graph_api_version: "v26.0".to_owned(),
                endpoints: EndpointOverrides::default(),
            },
            Arc::new(RecordingRevocationTransport::default()),
        )
    }

    /// Spec 10's Facebook acceptance criterion: "the withheld-email path
    /// exercises 09's email-completion outcome end-to-end." Facebook's own
    /// `resolve_identity` produces the identity from a real (fixture)
    /// Graph API body with no `email` field; that identity is then fed,
    /// unmodified, into a real `IdentityResolver` backed by Task 2's
    /// harness (real SeaORM storage over SQLite) -- not asserted on in
    /// isolation.
    #[tokio::test]
    async fn withheld_email_drives_email_completion_end_to_end() {
        let h = oauth_harness::harness().await;
        let facebook = provider();
        let body = r#"{"id":"10152","name":"Ada Lovelace"}"#;
        let identity = facebook
            .resolve_identity(ProviderResponse::UserInfo {
                body: body.to_owned(),
            })
            .await
            .expect("resolves even without an email");
        assert!(identity.email.is_none());

        let outcome = resolver(&h)
            .resolve(
                identity,
                OAuthIntent::SignIn,
                None,
                SessionMetadata::default(),
            )
            .await
            .expect("resolver accepts a no-email identity");
        let IdentityOutcome::EmailCompletionRequired { pending_id } = outcome else {
            panic!("expected EmailCompletionRequired for Facebook's withheld-email identity");
        };
        assert!(!pending_id.is_empty());
    }
}

// --- X ---------------------------------------------------------------------

#[cfg(feature = "oauth-x")]
mod x {
    use super::*;
    use base64::Engine as _;
    use magnetar::oauth::OAuthProtocolError;
    use magnetar::oauth::OAuthProvider as _;
    use magnetar::plugins::oauth_x::{XOAuthProvider, XProviderConfig};
    use secrecy::SecretString;

    fn provider() -> (XOAuthProvider, Arc<RecordingRevocationTransport>) {
        let transport = Arc::new(RecordingRevocationTransport::default());
        let provider = XOAuthProvider::new(
            XProviderConfig {
                client_id: "client-123".to_owned(),
                client_secret: SecretString::from("secret-xyz".to_owned()),
                redirect_uri: Some("https://example.com/callback".to_owned()),
                scopes: vec!["tweet.read".to_owned(), "users.read".to_owned()],
                endpoints: EndpointOverrides::default(),
            },
            transport.clone(),
        );
        (provider, transport)
    }

    #[tokio::test]
    async fn always_resolves_with_no_email_even_if_the_body_carries_one() {
        let (provider, _transport) = provider();
        // A body that *does* include an email-shaped field must still not
        // surface it -- X's no-email path is the happy path, not a parse
        // failure.
        let body = r#"{"data":{"id":"9876","username":"ada","name":"Ada Lovelace","confirmed_email":"user@example.com"}}"#;
        let identity = provider
            .resolve_identity(ProviderResponse::UserInfo {
                body: body.to_owned(),
            })
            .await
            .expect("resolves");
        assert_eq!(identity.provider, "x");
        assert_eq!(identity.subject, "9876");
        assert_eq!(identity.display_name.as_deref(), Some("Ada Lovelace"));
        assert!(identity.email.is_none());
        assert!(!identity.email_verified);
    }

    #[tokio::test]
    async fn missing_data_envelope_is_a_malformed_response() {
        let (provider, _transport) = provider();
        let error = provider
            .resolve_identity(ProviderResponse::UserInfo {
                body: "{}".to_owned(),
            })
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            OAuthProtocolError::MalformedProviderResponse { provider: "x", .. }
        ));
    }

    #[tokio::test]
    async fn revoke_uses_http_basic_client_authentication() {
        let (provider, transport) = provider();
        provider
            .revoke("tok", TokenHint::Refresh)
            .await
            .expect("succeeds");
        let calls = transport.calls.lock().expect("lock poisoned");
        let call = &calls[0];
        assert_eq!(call.method, "POST");
        assert_eq!(call.endpoint, "https://api.twitter.com/2/oauth2/revoke");
        assert_eq!(find(&call.params, "token"), Some("tok"));
        assert_eq!(find(&call.params, "token_type_hint"), Some("refresh_token"));
        let expected = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode("client-123:secret-xyz")
        );
        assert_eq!(
            call.headers
                .iter()
                .find(|(k, _)| k == "Authorization")
                .map(|(_, v)| v.as_str()),
            Some(expected.as_str())
        );
    }

    #[test]
    fn refresh_requires_offline_access_scope() {
        let (provider, _transport) = provider();
        let policy = provider.refresh_policy();
        assert!(policy.supported);
        assert!(
            policy
                .required_scopes
                .contains(&"offline.access".to_owned())
        );
        assert_eq!(
            policy.token_client_authentication,
            magnetar::oauth::ClientAuthentication::HttpBasic
        );
    }

    #[test]
    fn authorization_and_token_shapes_are_rfc_default_pkce_required() {
        // M4: X's spec-mandated "PKCE required" posture was declared in
        // code but never fixture-rendered.
        let (provider, _transport) = provider();
        let auth_shape = provider.authorization_shape();
        assert_eq!(auth_shape, AuthorizationRequestShape::default());
        let params = AuthorizationRequestParams {
            client_id: "client-123".to_owned(),
            redirect_uri: Some("https://example.com/callback".to_owned()),
            scopes: vec!["tweet.read".to_owned()],
            state: Some("state123".to_owned()),
            code_challenge: Some("challenge".to_owned()),
            nonce: None,
        };
        let wire = render_authorization_request(&auth_shape, &params).expect("PKCE supplied");
        assert_eq!(find(&wire, "code_challenge"), Some("challenge"));
        assert_eq!(find(&wire, "code_challenge_method"), Some("S256"));
        assert_eq!(find(&wire, "client_id"), Some("client-123"));

        let token_shape = provider.token_shape();
        let token_params = TokenRequestParams {
            client_id: "client-123".to_owned(),
            code: secrecy::SecretString::from("auth-code".to_owned()),
            redirect_uri: Some("https://example.com/callback".to_owned()),
            code_verifier: Some(secrecy::SecretString::from("verifier".to_owned())),
            scopes: Vec::new(),
        };
        let token_wire = render_token_request(&token_shape, &token_params);
        assert_eq!(find(&token_wire, "code_verifier"), Some("verifier"));
    }

    #[tokio::test]
    async fn client_authentication_renders_basic_header_only() {
        let (provider, _transport) = provider();
        let material = provider
            .client_authentication()
            .await
            .expect("renders client authentication");
        assert!(material.params.is_empty());
        let expected = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode("client-123:secret-xyz")
        );
        assert_eq!(
            material
                .headers
                .iter()
                .find(|(k, _)| k == "Authorization")
                .map(|(_, v)| v.as_str()),
            Some(expected.as_str())
        );
    }
}

#[cfg(all(feature = "oauth-x", feature = "seaorm-sqlite"))]
mod x_end_to_end {
    use std::sync::Arc;

    use magnetar::oauth::{
        AutoLinkPolicy, EndpointOverrides, IdentityOutcome, IdentityResolver, OAuthIntent,
        OAuthProvider as _, ProviderResponse,
    };
    use magnetar::plugins::oauth_x::{XOAuthProvider, XProviderConfig};
    use magnetar::sessions::SessionMetadata;
    use secrecy::SecretString;

    use super::RecordingRevocationTransport;
    use crate::oauth_harness;

    fn resolver(h: &oauth_harness::OAuthHarness) -> IdentityResolver {
        IdentityResolver::new(
            h.storage.clone(),
            h.storage.clone(),
            h.storage.clone(),
            h.first_proof.clone(),
            h.encryptor.clone(),
            AutoLinkPolicy::ExplicitLinkRequired,
        )
    }

    fn provider() -> XOAuthProvider {
        XOAuthProvider::new(
            XProviderConfig {
                client_id: "client-123".to_owned(),
                client_secret: SecretString::from("secret-xyz".to_owned()),
                redirect_uri: Some("https://example.com/callback".to_owned()),
                scopes: vec!["tweet.read".to_owned(), "users.read".to_owned()],
                endpoints: EndpointOverrides::default(),
            },
            Arc::new(RecordingRevocationTransport::default()),
        )
    }

    /// Spec 10's X acceptance criterion: "sign-up via email-completion is
    /// X's happy-path test." A brand-new X identity, resolved from a real
    /// (fixture) `/2/users/me` body, must drive `EmailCompletionRequired`
    /// through a real `IdentityResolver` -- this is the *expected*, normal
    /// outcome of an X sign-up, not an edge case.
    #[tokio::test]
    async fn sign_up_is_email_completion_end_to_end() {
        let h = oauth_harness::harness().await;
        let x = provider();
        let body = r#"{"data":{"id":"9876","username":"ada","name":"Ada Lovelace"}}"#;
        let identity = x
            .resolve_identity(ProviderResponse::UserInfo {
                body: body.to_owned(),
            })
            .await
            .expect("resolves");
        assert!(identity.email.is_none());

        let outcome = resolver(&h)
            .resolve(
                identity,
                OAuthIntent::SignIn,
                None,
                SessionMetadata::default(),
            )
            .await
            .expect("resolver accepts a no-email identity");
        let IdentityOutcome::EmailCompletionRequired { pending_id } = outcome else {
            panic!("expected EmailCompletionRequired as X's normal sign-up outcome");
        };
        assert!(!pending_id.is_empty());
    }
}

// --- TikTok ------------------------------------------------------------

#[cfg(feature = "oauth-tiktok")]
mod tiktok {
    use super::*;
    use magnetar::oauth::OAuthProtocolError;
    use magnetar::oauth::OAuthProvider as _;
    use magnetar::plugins::oauth_tiktok::{TikTokOAuthProvider, TikTokProviderConfig};
    use secrecy::SecretString;

    fn provider() -> (TikTokOAuthProvider, Arc<RecordingRevocationTransport>) {
        let transport = Arc::new(RecordingRevocationTransport::default());
        let provider = TikTokOAuthProvider::new(
            TikTokProviderConfig {
                client_id: "ck-123".to_owned(),
                client_secret: SecretString::from("cs-456".to_owned()),
                redirect_uri: Some("https://example.com/callback".to_owned()),
                scopes: vec!["user.info.basic".to_owned()],
                endpoints: EndpointOverrides::default(),
            },
            transport.clone(),
        );
        (provider, transport)
    }

    #[test]
    fn authorization_shape_uses_client_key_and_comma_scopes_always_sent() {
        let (provider, _transport) = provider();
        let shape = provider.authorization_shape();
        let params = AuthorizationRequestParams {
            client_id: "ck-123".to_owned(),
            redirect_uri: Some("https://example.com/callback".to_owned()),
            scopes: Vec::new(),
            state: Some("state123".to_owned()),
            code_challenge: Some("challenge".to_owned()),
            nonce: None,
        };
        let wire = render_authorization_request(&shape, &params).expect("PKCE supplied");
        assert_eq!(find(&wire, "client_key"), Some("ck-123"));
        assert!(!has_key(&wire, "client_id"));
        // Empty requested scopes still emit `scope` (always_send_scope).
        assert_eq!(find(&wire, "scope"), Some(""));
    }

    #[test]
    fn token_shape_uses_client_key_and_accepts_http_200_errors() {
        let (provider, _transport) = provider();
        let shape = provider.token_shape();
        assert_eq!(shape.client_id_param, "client_key");
        assert_eq!(shape.scope_delimiter, ",");
        assert!(shape.always_send_scope);
        assert!(shape.accept_http_success_error_body);
        let params = TokenRequestParams {
            client_id: "ck-123".to_owned(),
            code: SecretString::from("auth-code".to_owned()),
            redirect_uri: Some("https://example.com/callback".to_owned()),
            code_verifier: Some(SecretString::from("verifier".to_owned())),
            scopes: vec!["a".to_owned(), "b".to_owned()],
        };
        let wire = render_token_request(&shape, &params);
        assert_eq!(find(&wire, "client_key"), Some("ck-123"));
        assert_eq!(find(&wire, "scope"), Some("a,b"));
    }

    #[tokio::test]
    async fn resolves_identity_with_no_email_field_ever() {
        let (provider, _transport) = provider();
        let body = r#"{"data":{"user":{"open_id":"open-1","union_id":"union-1","display_name":"Ada"}},"error":{"code":"ok","message":"","log_id":"1"}}"#;
        let identity = provider
            .resolve_identity(ProviderResponse::UserInfo {
                body: body.to_owned(),
            })
            .await
            .expect("resolves");
        assert_eq!(identity.provider, "tiktok");
        assert_eq!(identity.subject, "open-1");
        assert_eq!(identity.display_name.as_deref(), Some("Ada"));
        assert!(identity.email.is_none());
        assert!(!identity.email_verified);
    }

    #[tokio::test]
    async fn http_200_error_body_classifies_as_provider_reported_error() {
        let (provider, _transport) = provider();
        let body = r#"{"data":{},"error":{"code":"access_token_invalid","message":"The access token is invalid or expired","log_id":"1"}}"#;
        let error = provider
            .resolve_identity(ProviderResponse::UserInfo {
                body: body.to_owned(),
            })
            .await
            .unwrap_err();
        match error {
            OAuthProtocolError::ProviderReportedError {
                provider,
                code,
                message,
            } => {
                assert_eq!(provider, "tiktok");
                assert_eq!(code, "access_token_invalid");
                assert_eq!(
                    message.as_deref(),
                    Some("The access token is invalid or expired")
                );
            }
            other => panic!("expected ProviderReportedError, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn missing_user_is_a_malformed_response() {
        let (provider, _transport) = provider();
        let body = r#"{"data":{},"error":{"code":"ok","message":"","log_id":"1"}}"#;
        let error = provider
            .resolve_identity(ProviderResponse::UserInfo {
                body: body.to_owned(),
            })
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            OAuthProtocolError::MalformedProviderResponse {
                provider: "tiktok",
                ..
            }
        ));
    }

    #[tokio::test]
    async fn revoke_sends_client_key_and_secret_in_body_no_hint() {
        let (provider, transport) = provider();
        provider
            .revoke("tok", TokenHint::Access)
            .await
            .expect("succeeds");
        let calls = transport.calls.lock().expect("lock poisoned");
        let call = &calls[0];
        assert_eq!(call.method, "POST");
        assert_eq!(
            call.endpoint,
            "https://open.tiktokapis.com/v2/oauth/revoke/"
        );
        assert_eq!(find(&call.params, "token"), Some("tok"));
        assert_eq!(find(&call.params, "client_key"), Some("ck-123"));
        assert_eq!(find(&call.params, "client_secret"), Some("cs-456"));
        assert!(!has_key(&call.params, "token_type_hint"));
    }

    #[test]
    fn refresh_policy_uses_request_body_authentication_and_signals_rotation_reuse() {
        let (provider, _transport) = provider();
        let policy = provider.refresh_policy();
        assert_eq!(
            policy.token_client_authentication,
            magnetar::oauth::ClientAuthentication::RequestBody
        );
        // TikTok rotates refresh tokens on every refresh call, so an
        // invalid_grant can mean a stale/already-rotated-out token, unlike
        // the other four providers' non-rotating refresh tokens.
        assert_eq!(
            policy.invalid_grant_meaning,
            magnetar::oauth::InvalidGrantMeaning::ReuseOrExternalRevocation
        );
    }
}

#[cfg(all(feature = "oauth-tiktok", feature = "seaorm-sqlite"))]
mod tiktok_end_to_end {
    use std::sync::Arc;

    use magnetar::oauth::{
        AutoLinkPolicy, EndpointOverrides, IdentityOutcome, IdentityResolver, OAuthIntent,
        OAuthProvider as _, ProviderResponse,
    };
    use magnetar::plugins::oauth_tiktok::{TikTokOAuthProvider, TikTokProviderConfig};
    use magnetar::sessions::SessionMetadata;
    use secrecy::SecretString;

    use super::RecordingRevocationTransport;
    use crate::oauth_harness;

    fn resolver(h: &oauth_harness::OAuthHarness) -> IdentityResolver {
        IdentityResolver::new(
            h.storage.clone(),
            h.storage.clone(),
            h.storage.clone(),
            h.first_proof.clone(),
            h.encryptor.clone(),
            AutoLinkPolicy::ExplicitLinkRequired,
        )
    }

    fn provider() -> TikTokOAuthProvider {
        TikTokOAuthProvider::new(
            TikTokProviderConfig {
                client_id: "ck-123".to_owned(),
                client_secret: SecretString::from("cs-456".to_owned()),
                redirect_uri: Some("https://example.com/callback".to_owned()),
                scopes: vec!["user.info.basic".to_owned()],
                endpoints: EndpointOverrides::default(),
            },
            Arc::new(RecordingRevocationTransport::default()),
        )
    }

    /// Spec 10's TikTok acceptance criterion: "sign-up always routes
    /// through email-completion" -- TikTok's user-info schema has no email
    /// field at all, so every TikTok identity, resolved from a real
    /// (fixture) `/v2/user/info/` body, must drive
    /// `EmailCompletionRequired` through a real `IdentityResolver`.
    #[tokio::test]
    async fn sign_up_always_routes_through_email_completion_end_to_end() {
        let h = oauth_harness::harness().await;
        let tiktok = provider();
        let body = r#"{"data":{"user":{"open_id":"open-e2e-1","union_id":"union-e2e-1","display_name":"Ada"}},"error":{"code":"ok","message":"","log_id":"1"}}"#;
        let identity = tiktok
            .resolve_identity(ProviderResponse::UserInfo {
                body: body.to_owned(),
            })
            .await
            .expect("resolves");
        assert!(identity.email.is_none());

        let outcome = resolver(&h)
            .resolve(
                identity,
                OAuthIntent::SignIn,
                None,
                SessionMetadata::default(),
            )
            .await
            .expect("resolver accepts a no-email identity");
        let IdentityOutcome::EmailCompletionRequired { pending_id } = outcome else {
            panic!("expected EmailCompletionRequired as TikTok's normal sign-up outcome");
        };
        assert!(!pending_id.is_empty());
    }
}
