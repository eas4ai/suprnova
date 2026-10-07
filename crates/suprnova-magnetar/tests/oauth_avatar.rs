//! The account picture every first-party provider reports through
//! `OAuthProvider::avatar_url` (PAR-041).
//!
//! Each case reads the picture from the same already-fetched profile
//! response `resolve_identity` then consumes, the order the host engine
//! uses on a callback. A picture comes back exactly as the provider
//! reported it; a profile without one, or with an empty one, reports none
//! and still resolves an identity. The profile requests themselves must
//! name the fields that carry the picture, or the provider never sends it.
//!
//! Provider sections are gated on their own features, like
//! `tests/oauth_providers.rs`, so the per-feature matrix compiles this file
//! with every other provider stripped out.

#![cfg(feature = "oauth")]

use async_trait::async_trait;
use magnetar::oauth::{
    AuthorizationRequestShape, ClientAuthentication, ClientAuthenticationMaterial,
    InvalidGrantMeaning, OAuthProtocolError, OAuthProvider, OAuthResult, ProviderIdentity,
    ProviderResponse, RefreshPolicy, TokenHint, TokenRequestShape,
};

/// One callback's identity step: the picture is read before
/// `resolve_identity` takes the response, and sign-in must succeed whether
/// or not the profile carries a picture.
async fn sign_in(provider: &dyn OAuthProvider, body: &str) -> (ProviderIdentity, Option<String>) {
    let response = ProviderResponse::UserInfo {
        body: body.to_owned(),
    };
    let avatar_url = provider.avatar_url(&response);
    let identity = provider
        .resolve_identity(response)
        .await
        .unwrap_or_else(|error| panic!("the profile must still sign in: {error}"));
    (identity, avatar_url)
}

/// The values of one query parameter of `url`, split on commas, as a
/// provider's field-selection parameter lists them.
#[cfg(any(feature = "oauth-facebook", feature = "oauth-x"))]
fn requested_fields(url: &str, parameter: &str) -> Vec<String> {
    let query = url.split_once('?').map(|(_, query)| query).unwrap_or("");
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .filter(|(name, _)| *name == parameter)
        .flat_map(|(_, value)| value.split(','))
        .map(str::to_owned)
        .collect()
}

/// JSON values that are present but are not the string a picture URL
/// should be. The picture is optional profile data, so each must report no
/// picture and leave sign-in as it was.
#[cfg(any(
    feature = "oauth-google",
    feature = "oauth-x",
    feature = "oauth-tiktok"
))]
const UNEXPECTED_STRING_SHAPES: [&str; 5] = [
    "42",
    "true",
    r#"["https://cdn.example.test/ada.jpg"]"#,
    r#"{"url":"https://cdn.example.test/ada.jpg"}"#,
    r#"{"data":{"url":"https://cdn.example.test/ada.jpg"}}"#,
];

#[cfg(any(
    feature = "oauth-apple",
    feature = "oauth-google",
    feature = "oauth-facebook",
    feature = "oauth-x",
    feature = "oauth-tiktok"
))]
struct NoRevocation;

#[cfg(any(
    feature = "oauth-apple",
    feature = "oauth-google",
    feature = "oauth-facebook",
    feature = "oauth-x",
    feature = "oauth-tiktok"
))]
#[async_trait]
impl magnetar::oauth::RevocationTransport for NoRevocation {
    async fn send(&self, _request: magnetar::oauth::RevocationRequest) -> OAuthResult<()> {
        Err(OAuthProtocolError::UpstreamUnavailable {
            provider: "avatar-test",
            message: "avatar tests never revoke".to_owned(),
            retry_after_seconds: None,
        })
    }
}

// --- A provider written before `avatar_url` existed ----------------------

/// A community provider that implements every required method and nothing
/// else. It must keep compiling, report no picture, and sign in.
struct LegacyProvider;

#[async_trait]
impl OAuthProvider for LegacyProvider {
    fn name(&self) -> &'static str {
        "legacy"
    }

    fn authorization_shape(&self) -> AuthorizationRequestShape {
        AuthorizationRequestShape::default()
    }

    fn token_shape(&self) -> TokenRequestShape {
        TokenRequestShape::default()
    }

    async fn resolve_identity(&self, response: ProviderResponse) -> OAuthResult<ProviderIdentity> {
        let ProviderResponse::UserInfo { body } = response else {
            return Err(OAuthProtocolError::MalformedProviderResponse {
                provider: "legacy",
                message: "fixture requires a userinfo response".to_owned(),
            });
        };
        let profile: serde_json::Value = serde_json::from_str(&body).map_err(|error| {
            OAuthProtocolError::MalformedProviderResponse {
                provider: "legacy",
                message: error.to_string(),
            }
        })?;
        Ok(ProviderIdentity {
            provider: "legacy".to_owned(),
            subject: profile["id"].as_str().unwrap_or_default().to_owned(),
            email: None,
            email_verified: false,
            display_name: None,
        })
    }

    async fn revoke(&self, _token: &str, _hint: TokenHint) -> OAuthResult<()> {
        Ok(())
    }

    fn client_id(&self) -> &str {
        "legacy-client"
    }

    fn token_endpoint(&self) -> String {
        "https://legacy.test/token".to_owned()
    }

    fn authorization_endpoint(&self) -> String {
        "https://legacy.test/authorize".to_owned()
    }

    fn userinfo_endpoint(&self) -> Option<String> {
        Some("https://legacy.test/user".to_owned())
    }

    fn refresh_policy(&self) -> RefreshPolicy {
        RefreshPolicy {
            supported: false,
            token_client_authentication: ClientAuthentication::RequestBody,
            extra_authorization_params: Vec::new(),
            required_scopes: Vec::new(),
            requires_reconsent_for_reissue: false,
            invalid_grant_meaning: InvalidGrantMeaning::OrdinaryRevocation,
        }
    }

    async fn client_authentication(&self) -> OAuthResult<ClientAuthenticationMaterial> {
        Ok(ClientAuthenticationMaterial::default())
    }
}

#[tokio::test]
async fn a_provider_without_avatar_url_reports_none_and_signs_in() {
    let (identity, avatar_url) = sign_in(
        &LegacyProvider,
        r#"{"id":"legacy-subject","picture":"https://legacy.test/me.png"}"#,
    )
    .await;
    assert_eq!(identity.subject, "legacy-subject");
    assert_eq!(avatar_url, None);
}

#[tokio::test]
async fn provider_identity_keeps_exactly_its_five_fields() {
    let (identity, _) = sign_in(&LegacyProvider, r#"{"id":"legacy-subject"}"#).await;
    // No `..`: a sixth field on `ProviderIdentity` fails to compile here.
    let ProviderIdentity {
        provider,
        subject,
        email,
        email_verified,
        display_name,
    } = identity;
    assert_eq!(provider, "legacy");
    assert_eq!(subject, "legacy-subject");
    assert_eq!(email, None);
    assert!(!email_verified);
    assert_eq!(display_name, None);
}

// --- Google: the userinfo `picture` claim --------------------------------

#[cfg(feature = "oauth-google")]
mod google {
    use std::sync::Arc;

    use magnetar::oauth::EndpointOverrides;
    use magnetar::plugins::oauth_google::{GoogleOAuthProvider, GoogleProviderConfig};
    use secrecy::SecretString;

    use super::{NoRevocation, sign_in};

    fn provider() -> GoogleOAuthProvider {
        GoogleOAuthProvider::new(
            GoogleProviderConfig {
                client_id: "google-client".to_owned(),
                client_secret: SecretString::from("google-secret".to_owned()),
                redirect_uri: Some("https://app.test/auth/google/callback".to_owned()),
                scopes: vec!["openid".to_owned(), "profile".to_owned()],
                endpoints: EndpointOverrides::default(),
            },
            Arc::new(NoRevocation),
        )
    }

    #[tokio::test]
    async fn the_picture_claim_is_the_avatar() {
        let (identity, avatar_url) = sign_in(
            &provider(),
            r#"{"sub":"g-1","email":"ada@example.test","email_verified":true,"name":"Ada","picture":"https://lh3.googleusercontent.com/a/ada=s96-c"}"#,
        )
        .await;
        assert_eq!(identity.subject, "g-1");
        assert_eq!(
            avatar_url.as_deref(),
            Some("https://lh3.googleusercontent.com/a/ada=s96-c")
        );
    }

    #[tokio::test]
    async fn a_profile_without_a_picture_reports_none_and_signs_in() {
        let (identity, avatar_url) = sign_in(&provider(), r#"{"sub":"g-1","name":"Ada"}"#).await;
        assert_eq!(identity.subject, "g-1");
        assert_eq!(avatar_url, None);
    }

    #[tokio::test]
    async fn an_empty_picture_reports_none_and_signs_in() {
        let (identity, avatar_url) =
            sign_in(&provider(), r#"{"sub":"g-1","name":"Ada","picture":""}"#).await;
        assert_eq!(identity.subject, "g-1");
        assert_eq!(avatar_url, None);
    }

    #[tokio::test]
    async fn a_picture_of_an_unexpected_shape_reports_none_and_signs_in() {
        for picture in super::UNEXPECTED_STRING_SHAPES {
            let (identity, avatar_url) = sign_in(
                &provider(),
                &format!(r#"{{"sub":"g-1","name":"Ada","picture":{picture}}}"#),
            )
            .await;
            assert_eq!(identity.subject, "g-1", "picture {picture}");
            assert_eq!(identity.display_name.as_deref(), Some("Ada"));
            assert_eq!(avatar_url, None, "picture {picture}");
        }
    }
}

// --- TikTok: `data.user.avatar_url` --------------------------------------

#[cfg(feature = "oauth-tiktok")]
mod tiktok {
    use std::sync::Arc;

    use magnetar::oauth::{EndpointOverrides, OAuthProvider as _};
    use magnetar::plugins::oauth_tiktok::{TikTokOAuthProvider, TikTokProviderConfig};
    use secrecy::SecretString;

    use super::{NoRevocation, sign_in};

    fn provider() -> TikTokOAuthProvider {
        TikTokOAuthProvider::new(
            TikTokProviderConfig {
                client_id: "tiktok-client".to_owned(),
                client_secret: SecretString::from("tiktok-secret".to_owned()),
                redirect_uri: Some("https://app.test/auth/tiktok/callback".to_owned()),
                scopes: vec!["user.info.basic".to_owned()],
                endpoints: EndpointOverrides::default(),
            },
            Arc::new(NoRevocation),
        )
    }

    #[tokio::test]
    async fn the_avatar_url_field_is_the_avatar() {
        let (identity, avatar_url) = sign_in(
            &provider(),
            r#"{"data":{"user":{"open_id":"t-1","display_name":"Ada","avatar_url":"https://p16-sign.tiktokcdn.com/ada.jpeg"}},"error":{"code":"ok","message":""}}"#,
        )
        .await;
        assert_eq!(identity.subject, "t-1");
        assert_eq!(
            avatar_url.as_deref(),
            Some("https://p16-sign.tiktokcdn.com/ada.jpeg")
        );
    }

    #[tokio::test]
    async fn a_profile_without_a_picture_reports_none_and_signs_in() {
        let (identity, avatar_url) = sign_in(
            &provider(),
            r#"{"data":{"user":{"open_id":"t-1","display_name":"Ada"}},"error":{"code":"ok","message":""}}"#,
        )
        .await;
        assert_eq!(identity.subject, "t-1");
        assert_eq!(avatar_url, None);
    }

    #[tokio::test]
    async fn an_empty_picture_reports_none_and_signs_in() {
        let (identity, avatar_url) = sign_in(
            &provider(),
            r#"{"data":{"user":{"open_id":"t-1","display_name":"Ada","avatar_url":""}},"error":{"code":"ok","message":""}}"#,
        )
        .await;
        assert_eq!(identity.subject, "t-1");
        assert_eq!(avatar_url, None);
    }

    #[tokio::test]
    async fn a_picture_of_an_unexpected_shape_reports_none_and_signs_in() {
        for picture in super::UNEXPECTED_STRING_SHAPES {
            let (identity, avatar_url) = sign_in(
                &provider(),
                &format!(
                    r#"{{"data":{{"user":{{"open_id":"t-1","display_name":"Ada","avatar_url":{picture}}}}},"error":{{"code":"ok","message":""}}}}"#
                ),
            )
            .await;
            assert_eq!(identity.subject, "t-1", "picture {picture}");
            assert_eq!(identity.display_name.as_deref(), Some("Ada"));
            assert_eq!(avatar_url, None, "picture {picture}");
        }
    }

    #[test]
    fn the_profile_request_asks_for_the_avatar_url_field() {
        let endpoint = provider()
            .userinfo_endpoint()
            .expect("TikTok fetches a user-info profile");
        assert!(
            endpoint.contains("avatar_url"),
            "TikTok's user-info request must name avatar_url: {endpoint}"
        );
    }
}

// --- Facebook: `picture.data.url`, requested with the profile fields -----

#[cfg(feature = "oauth-facebook")]
mod facebook {
    use std::sync::Arc;

    use magnetar::oauth::OAuthProvider as _;
    use magnetar::plugins::oauth_facebook::{FacebookOAuthProvider, FacebookProviderConfig};
    use secrecy::SecretString;

    use super::{NoRevocation, requested_fields, sign_in};

    fn provider() -> FacebookOAuthProvider {
        FacebookOAuthProvider::new(
            FacebookProviderConfig {
                client_id: "facebook-app".to_owned(),
                client_secret: SecretString::from("facebook-secret".to_owned()),
                redirect_uri: Some("https://app.test/auth/facebook/callback".to_owned()),
                scopes: vec!["email".to_owned(), "public_profile".to_owned()],
                ..FacebookProviderConfig::default()
            },
            Arc::new(NoRevocation),
        )
    }

    #[tokio::test]
    async fn the_picture_data_url_is_the_avatar() {
        let (identity, avatar_url) = sign_in(
            &provider(),
            r#"{"id":"f-1","name":"Ada","email":"ada@example.test","picture":{"data":{"height":50,"is_silhouette":false,"url":"https://platform-lookaside.fbsbx.com/ada.jpg","width":50}}}"#,
        )
        .await;
        assert_eq!(identity.subject, "f-1");
        assert_eq!(identity.email.as_deref(), Some("ada@example.test"));
        assert_eq!(
            avatar_url.as_deref(),
            Some("https://platform-lookaside.fbsbx.com/ada.jpg")
        );
    }

    #[tokio::test]
    async fn a_profile_without_a_picture_reports_none_and_signs_in() {
        let (identity, avatar_url) = sign_in(&provider(), r#"{"id":"f-1","name":"Ada"}"#).await;
        assert_eq!(identity.subject, "f-1");
        assert_eq!(avatar_url, None);
    }

    #[tokio::test]
    async fn an_empty_picture_reports_none_and_signs_in() {
        let (identity, avatar_url) = sign_in(
            &provider(),
            r#"{"id":"f-1","name":"Ada","picture":{"data":{"url":""}}}"#,
        )
        .await;
        assert_eq!(identity.subject, "f-1");
        assert_eq!(avatar_url, None);
    }

    #[tokio::test]
    async fn a_picture_of_an_unexpected_shape_reports_none_and_signs_in() {
        for picture in [
            // The URL itself where Graph's `{"data":{"url":...}}` belongs.
            r#""https://platform-lookaside.fbsbx.com/ada.jpg""#,
            "42",
            "true",
            r#"["https://platform-lookaside.fbsbx.com/ada.jpg"]"#,
            r#"{"data":"https://platform-lookaside.fbsbx.com/ada.jpg"}"#,
            r#"{"data":[{"url":"https://platform-lookaside.fbsbx.com/ada.jpg"}]}"#,
            r#"{"data":{"url":42}}"#,
            r#"{"data":{"url":{"href":"https://platform-lookaside.fbsbx.com/ada.jpg"}}}"#,
        ] {
            let (identity, avatar_url) = sign_in(
                &provider(),
                &format!(
                    r#"{{"id":"f-1","name":"Ada","email":"ada@example.test","picture":{picture}}}"#
                ),
            )
            .await;
            assert_eq!(identity.subject, "f-1", "picture {picture}");
            assert_eq!(identity.email.as_deref(), Some("ada@example.test"));
            assert_eq!(avatar_url, None, "picture {picture}");
        }
    }

    #[test]
    fn the_profile_request_names_id_name_email_and_picture() {
        let endpoint = provider()
            .userinfo_endpoint()
            .expect("Facebook fetches a Graph API profile");
        assert!(
            endpoint.starts_with("https://graph.facebook.com/v26.0/me?"),
            "the profile request stays on the configured Graph API version: {endpoint}"
        );
        let fields = requested_fields(&endpoint, "fields");
        for field in ["id", "name", "email", "picture"] {
            assert!(
                fields.iter().any(|requested| requested == field),
                "Facebook's profile request must name `{field}`: {endpoint}"
            );
        }
    }
}

// --- X: `data.profile_image_url`, requested as a user field --------------

#[cfg(feature = "oauth-x")]
mod x {
    use std::sync::Arc;

    use magnetar::oauth::{EndpointOverrides, OAuthProvider as _};
    use magnetar::plugins::oauth_x::{XOAuthProvider, XProviderConfig};
    use secrecy::SecretString;

    use super::{NoRevocation, requested_fields, sign_in};

    fn provider() -> XOAuthProvider {
        XOAuthProvider::new(
            XProviderConfig {
                client_id: "x-client".to_owned(),
                client_secret: SecretString::from("x-secret".to_owned()),
                redirect_uri: Some("https://app.test/auth/x/callback".to_owned()),
                scopes: vec!["users.read".to_owned()],
                endpoints: EndpointOverrides::default(),
            },
            Arc::new(NoRevocation),
        )
    }

    #[tokio::test]
    async fn the_profile_image_url_is_the_avatar() {
        let (identity, avatar_url) = sign_in(
            &provider(),
            r#"{"data":{"id":"x-1","name":"Ada","username":"ada","profile_image_url":"https://pbs.twimg.com/profile_images/1/ada_normal.jpg"}}"#,
        )
        .await;
        assert_eq!(identity.subject, "x-1");
        assert_eq!(
            avatar_url.as_deref(),
            Some("https://pbs.twimg.com/profile_images/1/ada_normal.jpg")
        );
    }

    #[tokio::test]
    async fn a_profile_without_a_picture_reports_none_and_signs_in() {
        let (identity, avatar_url) = sign_in(
            &provider(),
            r#"{"data":{"id":"x-1","name":"Ada","username":"ada"}}"#,
        )
        .await;
        assert_eq!(identity.subject, "x-1");
        assert_eq!(avatar_url, None);
    }

    #[tokio::test]
    async fn an_empty_picture_reports_none_and_signs_in() {
        let (identity, avatar_url) = sign_in(
            &provider(),
            r#"{"data":{"id":"x-1","name":"Ada","username":"ada","profile_image_url":""}}"#,
        )
        .await;
        assert_eq!(identity.subject, "x-1");
        assert_eq!(avatar_url, None);
    }

    #[tokio::test]
    async fn a_picture_of_an_unexpected_shape_reports_none_and_signs_in() {
        for picture in super::UNEXPECTED_STRING_SHAPES {
            let (identity, avatar_url) = sign_in(
                &provider(),
                &format!(
                    r#"{{"data":{{"id":"x-1","name":"Ada","username":"ada","profile_image_url":{picture}}}}}"#
                ),
            )
            .await;
            assert_eq!(identity.subject, "x-1", "picture {picture}");
            assert_eq!(identity.display_name.as_deref(), Some("Ada"));
            assert_eq!(avatar_url, None, "picture {picture}");
        }
    }

    #[test]
    fn the_profile_request_names_profile_image_url_among_its_user_fields() {
        let endpoint = provider()
            .userinfo_endpoint()
            .expect("X fetches a /2/users/me profile");
        assert!(
            endpoint.starts_with("https://api.twitter.com/2/users/me?"),
            "the profile request stays on /2/users/me: {endpoint}"
        );
        let fields = requested_fields(&endpoint, "user.fields");
        assert!(
            fields.iter().any(|field| field == "profile_image_url"),
            "X's profile request must name profile_image_url: {endpoint}"
        );
    }
}

// --- Apple: no picture ---------------------------------------------------

#[cfg(feature = "oauth-apple")]
mod apple {
    use std::sync::Arc;

    use async_trait::async_trait;
    use magnetar::oauth::{EndpointOverrides, OAuthProvider as _, OAuthResult, ProviderResponse};
    use magnetar::plugins::oauth_apple::{
        AppleClaims, AppleOAuthProvider, AppleProviderConfig, ApplePublicKeySource,
    };
    use secrecy::SecretString;

    use super::NoRevocation;

    /// The throwaway P-256 key `tests/oauth_providers.rs` uses; never a
    /// real Apple key.
    const TEST_PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----\n\
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg9cld72Dlc09boGa+\n\
Hoo62Cptg9VEedeF9m5qGzMdBxKhRANCAATs0RvE+uOTJbWfyY5AZat92wGjXsQU\n\
zV7lmLsegC7z6Mp+xNC89mSD5mfuBaptjAab1AT0XEIyB9mXg47uB1bM\n\
-----END PRIVATE KEY-----\n";

    struct FixedClaims;

    #[async_trait]
    impl ApplePublicKeySource for FixedClaims {
        async fn verify(
            &self,
            _id_token: &str,
            _audience: &str,
            _nonce: Option<&str>,
        ) -> OAuthResult<AppleClaims> {
            Ok(AppleClaims {
                subject: "apple-subject".to_owned(),
                email: Some("ada@privaterelay.appleid.com".to_owned()),
                email_verified: true,
                is_private_email: true,
            })
        }
    }

    #[tokio::test]
    async fn apple_reports_no_picture_and_signs_in() {
        let provider = AppleOAuthProvider::new(
            AppleProviderConfig {
                client_id: "com.example.app".to_owned(),
                team_id: "TEAMID1234".to_owned(),
                key_id: "KEYID1234".to_owned(),
                private_key_pem: SecretString::from(TEST_PRIVATE_KEY_PEM),
                redirect_uri: Some("https://app.test/auth/apple/callback".to_owned()),
                scopes: vec!["name".to_owned(), "email".to_owned()],
                endpoints: EndpointOverrides::default(),
            },
            Arc::new(FixedClaims),
            Arc::new(NoRevocation),
        )
        .expect("the test key parses");
        let response = ProviderResponse::AppleIdToken {
            id_token: SecretString::from("header.payload.signature".to_owned()),
            nonce: Some("apple-nonce".to_owned()),
            form_post_user: Some(
                r#"{"name":{"firstName":"Ada","lastName":"Lovelace"}}"#.to_owned(),
            ),
        };
        assert_eq!(provider.avatar_url(&response), None);
        let identity = provider
            .resolve_identity(response)
            .await
            .expect("Apple sign-in proceeds without a picture");
        assert_eq!(identity.subject, "apple-subject");
    }
}
