//! `OAuthIdentity::avatar_url` through `Auth::oauth(..).verify_oauth_identity`
//! (PAR-041).
//!
//! Every case drives a real first-party provider through the installed
//! Magnetar OAuth engine: `begin`, the token exchange and the profile
//! fetch run against a fake HTTP transport that answers with a fixture
//! profile and records each outgoing request. A picture in the profile
//! must reach `OAuthIdentity` exactly as the provider reported it; a
//! profile without one, or with an empty one, must still sign in and
//! report none. The provider uses its real profile URL, so the recorded
//! request proves which fields Facebook and X ask for.
//!
//! The OAuth engine is a process-wide install, so each test installs its
//! own and runs alone in a child process (`own_process_async::delegate`);
//! nextest already gives every test a process of its own.

#![cfg(all(feature = "magnetar-oauth", feature = "testing"))]

use std::sync::{Arc, Mutex};

use sea_orm::{ActiveModelTrait, ActiveValue::Set};
use suprnova::{
    AbuseLimiter, AbusePolicy, Auth, AuthorizationRequestShape, AutoLinkPolicy,
    ClientAuthentication, ClientAuthenticationMaterial, Crypt, EncryptionKey, EndpointOverrides,
    FacebookOAuthProvider, FacebookProviderConfig, GoogleOAuthProvider, GoogleProviderConfig,
    InvalidGrantMeaning, MagnetarOAuthHostConfig, MagnetarOAuthOnlyConfig,
    MagnetarOAuthProviderConfig, MagnetarResult, OAuthAuthorizationConfig, OAuthHttpRequest,
    OAuthHttpResponse, OAuthHttpTransport, OAuthProtocolError, OAuthProvider, OAuthResult, Permit,
    ProviderIdentity, ProviderResponse, RefreshPolicy, RevocationRequest, RevocationTransport,
    SecretString, SignInOutcome, TikTokOAuthProvider, TikTokProviderConfig, TokenHint,
    TokenRequestShape, XOAuthProvider, XProviderConfig, init_magnetar_oauth_only,
};

/// Answers the token exchange with a bearer token and the profile fetch
/// with one fixture profile, and records every request it was sent.
struct ProfileTransport {
    profile: String,
    requests: Mutex<Vec<(String, String)>>,
}

impl ProfileTransport {
    fn profile_requests(&self) -> Vec<String> {
        self.requests
            .lock()
            .expect("recorded requests")
            .iter()
            .filter(|(method, _)| method == "GET")
            .map(|(_, url)| url.clone())
            .collect()
    }
}

#[suprnova::async_trait]
impl OAuthHttpTransport for ProfileTransport {
    async fn send(&self, request: OAuthHttpRequest) -> MagnetarResult<OAuthHttpResponse> {
        self.requests
            .lock()
            .expect("recorded requests")
            .push((request.method.clone(), request.url.clone()));
        let body = if request.method == "GET" {
            self.profile.clone().into_bytes()
        } else {
            br#"{"access_token":"avatar-access-token","token_type":"Bearer"}"#.to_vec()
        };
        Ok(OAuthHttpResponse {
            status: 200,
            headers: Vec::new(),
            body,
        })
    }
}

struct NoRevocation;

#[suprnova::async_trait]
impl RevocationTransport for NoRevocation {
    async fn send(&self, _request: RevocationRequest) -> OAuthResult<()> {
        Err(OAuthProtocolError::UpstreamUnavailable {
            provider: "avatar-test",
            message: "avatar tests never revoke".to_owned(),
            retry_after_seconds: None,
        })
    }
}

struct AllowAll;

#[suprnova::async_trait]
impl AbuseLimiter for AllowAll {
    async fn acquire(&self, _key: &str, _policy: AbusePolicy) -> MagnetarResult<Permit> {
        Ok(Permit::Allowed { retry_after: None })
    }
}

/// A community provider written before `avatar_url` existed: it implements
/// only the required methods. Its `ProviderIdentity` literal has no `..`,
/// so a new field on that type fails to compile here.
struct CommunityProvider;

#[suprnova::async_trait]
impl OAuthProvider for CommunityProvider {
    fn name(&self) -> &'static str {
        "community"
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
                provider: "community",
                message: "fixture requires a userinfo response".to_owned(),
            });
        };
        let profile: serde_json::Value = serde_json::from_str(&body).map_err(|error| {
            OAuthProtocolError::MalformedProviderResponse {
                provider: "community",
                message: error.to_string(),
            }
        })?;
        Ok(ProviderIdentity {
            provider: "community".to_owned(),
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
        "community-client"
    }

    fn token_endpoint(&self) -> String {
        "https://community.test/token".to_owned()
    }

    fn authorization_endpoint(&self) -> String {
        "https://community.test/authorize".to_owned()
    }

    fn userinfo_endpoint(&self) -> Option<String> {
        Some("https://community.test/user".to_owned())
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

/// The community provider with `avatar_url` added, as the manual shows: it
/// copies the profile's `picture` without filtering it, so an empty string
/// reaches the engine as `Some("")`.
struct PictureCommunityProvider(CommunityProvider);

#[suprnova::async_trait]
impl OAuthProvider for PictureCommunityProvider {
    fn name(&self) -> &'static str {
        self.0.name()
    }

    fn authorization_shape(&self) -> AuthorizationRequestShape {
        self.0.authorization_shape()
    }

    fn token_shape(&self) -> TokenRequestShape {
        self.0.token_shape()
    }

    async fn resolve_identity(&self, response: ProviderResponse) -> OAuthResult<ProviderIdentity> {
        self.0.resolve_identity(response).await
    }

    fn avatar_url(&self, response: &ProviderResponse) -> Option<String> {
        let ProviderResponse::UserInfo { body } = response else {
            return None;
        };
        let profile: serde_json::Value = serde_json::from_str(body).ok()?;
        profile["picture"].as_str().map(str::to_owned)
    }

    async fn revoke(&self, token: &str, hint: TokenHint) -> OAuthResult<()> {
        self.0.revoke(token, hint).await
    }

    fn client_id(&self) -> &str {
        self.0.client_id()
    }

    fn token_endpoint(&self) -> String {
        self.0.token_endpoint()
    }

    fn authorization_endpoint(&self) -> String {
        self.0.authorization_endpoint()
    }

    fn userinfo_endpoint(&self) -> Option<String> {
        self.0.userinfo_endpoint()
    }

    fn refresh_policy(&self) -> RefreshPolicy {
        self.0.refresh_policy()
    }

    async fn client_authentication(&self) -> OAuthResult<ClientAuthenticationMaterial> {
        self.0.client_authentication().await
    }
}

fn google() -> Arc<dyn OAuthProvider> {
    Arc::new(GoogleOAuthProvider::new(
        GoogleProviderConfig {
            client_id: "google-client".to_owned(),
            client_secret: SecretString::from("google-secret".to_owned()),
            redirect_uri: None,
            scopes: vec!["openid".to_owned(), "profile".to_owned()],
            endpoints: EndpointOverrides::default(),
        },
        Arc::new(NoRevocation),
    ))
}

fn tiktok() -> Arc<dyn OAuthProvider> {
    Arc::new(TikTokOAuthProvider::new(
        TikTokProviderConfig {
            client_id: "tiktok-client".to_owned(),
            client_secret: SecretString::from("tiktok-secret".to_owned()),
            redirect_uri: None,
            scopes: vec!["user.info.basic".to_owned()],
            endpoints: EndpointOverrides::default(),
        },
        Arc::new(NoRevocation),
    ))
}

fn facebook() -> Arc<dyn OAuthProvider> {
    Arc::new(FacebookOAuthProvider::new(
        FacebookProviderConfig {
            client_id: "facebook-app".to_owned(),
            client_secret: SecretString::from("facebook-secret".to_owned()),
            scopes: vec!["email".to_owned(), "public_profile".to_owned()],
            ..FacebookProviderConfig::default()
        },
        Arc::new(NoRevocation),
    ))
}

fn x() -> Arc<dyn OAuthProvider> {
    Arc::new(XOAuthProvider::new(
        XProviderConfig {
            client_id: "x-client".to_owned(),
            client_secret: SecretString::from("x-secret".to_owned()),
            redirect_uri: None,
            scopes: vec!["users.read".to_owned()],
            endpoints: EndpointOverrides::default(),
        },
        Arc::new(NoRevocation),
    ))
}

/// Install the OAuth-only engine with one provider whose profile fetch
/// answers `profile`, and link `subject` at that provider to user 1 so the
/// full sign-in resolves to an existing account.
async fn install(
    provider: Arc<dyn OAuthProvider>,
    subject: &str,
    profile: &str,
) -> Arc<ProfileTransport> {
    Crypt::init(EncryptionKey::generate());
    let database = sea_orm::Database::connect("sqlite::memory:")
        .await
        .expect("connect SQLite");
    magnetar::default_schema::migrate(&database)
        .await
        .expect("migrate the Magnetar schema");
    let now = suprnova::chrono::Utc::now();
    magnetar::default_schema::users::ActiveModel {
        id: Set(1),
        email: Set("avatar@example.test".to_owned()),
        email_verified_at: Set(Some(now)),
        auth_epoch: Set(0),
        ..Default::default()
    }
    .insert(&database)
    .await
    .expect("seed the signed-in user");
    magnetar::default_schema::accounts::ActiveModel {
        id: Set(1),
        user_id: Set(1),
        provider: Set(provider.name().to_owned()),
        provider_account_id: Set(subject.to_owned()),
        created_at: Set(Some(now)),
        updated_at: Set(Some(now)),
    }
    .insert(&database)
    .await
    .expect("link the provider account");

    let transport = Arc::new(ProfileTransport {
        profile: profile.to_owned(),
        requests: Mutex::new(Vec::new()),
    });
    let name = provider.name();
    let oauth = MagnetarOAuthHostConfig::new(
        vec![MagnetarOAuthProviderConfig {
            provider,
            redirect_uri: format!("https://app.test/auth/{name}/callback"),
            scopes: vec!["profile".to_owned()],
        }],
        transport.clone(),
        Arc::new(AllowAll),
        OAuthAuthorizationConfig::default(),
        AutoLinkPolicy::default(),
    )
    .expect("OAuth host configuration");
    init_magnetar_oauth_only(
        MagnetarOAuthOnlyConfig::from_sea_orm(database, oauth).apply_migrations(false),
    )
    .await
    .expect("install the OAuth engine");
    transport
}

/// One callback through `verify_oauth_identity`: the subject it verified
/// and the picture it reported.
async fn verify(provider: &str) -> (String, Option<String>) {
    let slot = suprnova::session::new_session_slot_for_test();
    let identity = suprnova::session::session_scope_for_test(slot, async {
        let kickoff = Auth::oauth(provider).begin().await?;
        Auth::oauth(provider)
            .verify_oauth_identity("code", &kickoff.state)
            .await
    })
    .await
    .unwrap_or_else(|error| panic!("{provider} callback must verify: {error}"));
    (identity.subject, identity.avatar_url)
}

/// One full sign-in through `complete_outcome`, which must authenticate
/// the linked user whatever the profile says about a picture.
async fn assert_signs_in(provider: &str) {
    let slot = suprnova::session::new_session_slot_for_test();
    let outcome = suprnova::session::session_bind_scopes_for_test(slot, async {
        let kickoff = Auth::oauth(provider).begin().await?;
        Auth::oauth(provider)
            .complete_outcome("code", &kickoff.state)
            .await
    })
    .await
    .unwrap_or_else(|error| panic!("{provider} sign-in must proceed: {error}"));
    let SignInOutcome::Authenticated { user, .. } = outcome else {
        panic!("{provider} sign-in must authenticate the linked user");
    };
    assert_eq!(user.id.as_str(), "1");
}

/// Run a callback for `profile` and check the reported picture, then check
/// that a full sign-in still proceeds.
async fn callback(
    provider: Arc<dyn OAuthProvider>,
    subject: &str,
    profile: &str,
    expected_avatar: Option<&str>,
) -> Arc<ProfileTransport> {
    let name = provider.name();
    let transport = install(provider, subject, profile).await;
    let (verified_subject, avatar_url) = verify(name).await;
    assert_eq!(verified_subject, subject);
    assert_eq!(avatar_url.as_deref(), expected_avatar);
    assert_signs_in(name).await;
    transport
}

/// The values of one query parameter of `url`, split on commas.
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

// --- Google --------------------------------------------------------------

const GOOGLE_PICTURE: &str = "https://lh3.googleusercontent.com/a/ada=s96-c";

#[tokio::test]
async fn google_picture_reaches_oauth_identity() {
    if crate::own_process_async::delegate(module_path!(), "google_picture_reaches_oauth_identity")
        .await
    {
        return;
    }
    callback(
        google(),
        "g-1",
        &format!(r#"{{"sub":"g-1","name":"Ada","picture":"{GOOGLE_PICTURE}"}}"#),
        Some(GOOGLE_PICTURE),
    )
    .await;
}

#[tokio::test]
async fn google_without_picture_signs_in_with_none() {
    if crate::own_process_async::delegate(
        module_path!(),
        "google_without_picture_signs_in_with_none",
    )
    .await
    {
        return;
    }
    callback(google(), "g-1", r#"{"sub":"g-1","name":"Ada"}"#, None).await;
}

#[tokio::test]
async fn google_empty_picture_signs_in_with_none() {
    if crate::own_process_async::delegate(module_path!(), "google_empty_picture_signs_in_with_none")
        .await
    {
        return;
    }
    callback(
        google(),
        "g-1",
        r#"{"sub":"g-1","name":"Ada","picture":""}"#,
        None,
    )
    .await;
}

#[tokio::test]
async fn google_picture_of_an_unexpected_shape_signs_in_with_none() {
    if crate::own_process_async::delegate(
        module_path!(),
        "google_picture_of_an_unexpected_shape_signs_in_with_none",
    )
    .await
    {
        return;
    }
    callback(
        google(),
        "g-1",
        &format!(r#"{{"sub":"g-1","name":"Ada","picture":{{"url":"{GOOGLE_PICTURE}"}}}}"#),
        None,
    )
    .await;
}

// --- TikTok --------------------------------------------------------------

const TIKTOK_PICTURE: &str = "https://p16-sign.tiktokcdn.com/ada.jpeg";

#[tokio::test]
async fn tiktok_avatar_url_reaches_oauth_identity() {
    if crate::own_process_async::delegate(
        module_path!(),
        "tiktok_avatar_url_reaches_oauth_identity",
    )
    .await
    {
        return;
    }
    callback(
        tiktok(),
        "t-1",
        &format!(
            r#"{{"data":{{"user":{{"open_id":"t-1","display_name":"Ada","avatar_url":"{TIKTOK_PICTURE}"}}}},"error":{{"code":"ok","message":""}}}}"#
        ),
        Some(TIKTOK_PICTURE),
    )
    .await;
}

#[tokio::test]
async fn tiktok_without_picture_signs_in_with_none() {
    if crate::own_process_async::delegate(
        module_path!(),
        "tiktok_without_picture_signs_in_with_none",
    )
    .await
    {
        return;
    }
    callback(
        tiktok(),
        "t-1",
        r#"{"data":{"user":{"open_id":"t-1","display_name":"Ada"}},"error":{"code":"ok","message":""}}"#,
        None,
    )
    .await;
}

#[tokio::test]
async fn tiktok_empty_picture_signs_in_with_none() {
    if crate::own_process_async::delegate(module_path!(), "tiktok_empty_picture_signs_in_with_none")
        .await
    {
        return;
    }
    callback(
        tiktok(),
        "t-1",
        r#"{"data":{"user":{"open_id":"t-1","display_name":"Ada","avatar_url":""}},"error":{"code":"ok","message":""}}"#,
        None,
    )
    .await;
}

#[tokio::test]
async fn tiktok_picture_of_an_unexpected_shape_signs_in_with_none() {
    if crate::own_process_async::delegate(
        module_path!(),
        "tiktok_picture_of_an_unexpected_shape_signs_in_with_none",
    )
    .await
    {
        return;
    }
    callback(
        tiktok(),
        "t-1",
        r#"{"data":{"user":{"open_id":"t-1","display_name":"Ada","avatar_url":42}},"error":{"code":"ok","message":""}}"#,
        None,
    )
    .await;
}

// --- Facebook ------------------------------------------------------------

const FACEBOOK_PICTURE: &str = "https://platform-lookaside.fbsbx.com/ada.jpg";

#[tokio::test]
async fn facebook_picture_reaches_oauth_identity() {
    if crate::own_process_async::delegate(module_path!(), "facebook_picture_reaches_oauth_identity")
        .await
    {
        return;
    }
    callback(
        facebook(),
        "f-1",
        &format!(
            r#"{{"id":"f-1","name":"Ada","email":"avatar@example.test","picture":{{"data":{{"height":50,"is_silhouette":false,"url":"{FACEBOOK_PICTURE}","width":50}}}}}}"#
        ),
        Some(FACEBOOK_PICTURE),
    )
    .await;
}

#[tokio::test]
async fn facebook_without_picture_signs_in_with_none() {
    if crate::own_process_async::delegate(
        module_path!(),
        "facebook_without_picture_signs_in_with_none",
    )
    .await
    {
        return;
    }
    callback(facebook(), "f-1", r#"{"id":"f-1","name":"Ada"}"#, None).await;
}

#[tokio::test]
async fn facebook_empty_picture_signs_in_with_none() {
    if crate::own_process_async::delegate(
        module_path!(),
        "facebook_empty_picture_signs_in_with_none",
    )
    .await
    {
        return;
    }
    callback(
        facebook(),
        "f-1",
        r#"{"id":"f-1","name":"Ada","picture":{"data":{"url":""}}}"#,
        None,
    )
    .await;
}

#[tokio::test]
async fn facebook_picture_of_an_unexpected_shape_signs_in_with_none() {
    if crate::own_process_async::delegate(
        module_path!(),
        "facebook_picture_of_an_unexpected_shape_signs_in_with_none",
    )
    .await
    {
        return;
    }
    // The URL itself where Graph's `{"data":{"url":...}}` belongs.
    callback(
        facebook(),
        "f-1",
        &format!(r#"{{"id":"f-1","name":"Ada","picture":"{FACEBOOK_PICTURE}"}}"#),
        None,
    )
    .await;
}

#[tokio::test]
async fn facebook_profile_request_names_email_and_picture() {
    if crate::own_process_async::delegate(
        module_path!(),
        "facebook_profile_request_names_email_and_picture",
    )
    .await
    {
        return;
    }
    let transport = callback(
        facebook(),
        "f-1",
        r#"{"id":"f-1","name":"Ada","email":"avatar@example.test"}"#,
        None,
    )
    .await;
    let requests = transport.profile_requests();
    assert!(!requests.is_empty(), "the callback fetched a profile");
    for url in requests {
        assert!(
            url.starts_with("https://graph.facebook.com/v26.0/me?"),
            "the profile request goes to the Graph API /me: {url}"
        );
        let fields = requested_fields(&url, "fields");
        for field in ["id", "name", "email", "picture"] {
            assert!(
                fields.iter().any(|requested| requested == field),
                "Facebook's profile request must name `{field}`: {url}"
            );
        }
    }
}

// --- X -------------------------------------------------------------------

const X_PICTURE: &str = "https://pbs.twimg.com/profile_images/1/ada_normal.jpg";

#[tokio::test]
async fn x_profile_image_url_reaches_oauth_identity() {
    if crate::own_process_async::delegate(
        module_path!(),
        "x_profile_image_url_reaches_oauth_identity",
    )
    .await
    {
        return;
    }
    callback(
        x(),
        "x-1",
        &format!(
            r#"{{"data":{{"id":"x-1","name":"Ada","username":"ada","profile_image_url":"{X_PICTURE}"}}}}"#
        ),
        Some(X_PICTURE),
    )
    .await;
}

#[tokio::test]
async fn x_without_picture_signs_in_with_none() {
    if crate::own_process_async::delegate(module_path!(), "x_without_picture_signs_in_with_none")
        .await
    {
        return;
    }
    callback(
        x(),
        "x-1",
        r#"{"data":{"id":"x-1","name":"Ada","username":"ada"}}"#,
        None,
    )
    .await;
}

#[tokio::test]
async fn x_empty_picture_signs_in_with_none() {
    if crate::own_process_async::delegate(module_path!(), "x_empty_picture_signs_in_with_none")
        .await
    {
        return;
    }
    callback(
        x(),
        "x-1",
        r#"{"data":{"id":"x-1","name":"Ada","username":"ada","profile_image_url":""}}"#,
        None,
    )
    .await;
}

#[tokio::test]
async fn x_picture_of_an_unexpected_shape_signs_in_with_none() {
    if crate::own_process_async::delegate(
        module_path!(),
        "x_picture_of_an_unexpected_shape_signs_in_with_none",
    )
    .await
    {
        return;
    }
    callback(
        x(),
        "x-1",
        &format!(
            r#"{{"data":{{"id":"x-1","name":"Ada","username":"ada","profile_image_url":["{X_PICTURE}"]}}}}"#
        ),
        None,
    )
    .await;
}

#[tokio::test]
async fn x_profile_request_names_profile_image_url() {
    if crate::own_process_async::delegate(
        module_path!(),
        "x_profile_request_names_profile_image_url",
    )
    .await
    {
        return;
    }
    let transport = callback(
        x(),
        "x-1",
        r#"{"data":{"id":"x-1","name":"Ada","username":"ada"}}"#,
        None,
    )
    .await;
    let requests = transport.profile_requests();
    assert!(!requests.is_empty(), "the callback fetched a profile");
    for url in requests {
        assert!(
            url.starts_with("https://api.twitter.com/2/users/me?"),
            "the profile request goes to /2/users/me: {url}"
        );
        let fields = requested_fields(&url, "user.fields");
        assert!(
            fields.iter().any(|field| field == "profile_image_url"),
            "X's profile request must name profile_image_url: {url}"
        );
    }
}

// --- A provider without `avatar_url` -------------------------------------

#[tokio::test]
async fn a_provider_without_avatar_url_signs_in_with_none() {
    if crate::own_process_async::delegate(
        module_path!(),
        "a_provider_without_avatar_url_signs_in_with_none",
    )
    .await
    {
        return;
    }
    callback(
        Arc::new(CommunityProvider),
        "community-subject",
        r#"{"id":"community-subject","picture":"https://community.test/me.png"}"#,
        None,
    )
    .await;
}

// --- A community provider that adds `avatar_url` -------------------------

#[tokio::test]
async fn a_community_provider_avatar_url_reaches_oauth_identity() {
    if crate::own_process_async::delegate(
        module_path!(),
        "a_community_provider_avatar_url_reaches_oauth_identity",
    )
    .await
    {
        return;
    }
    callback(
        Arc::new(PictureCommunityProvider(CommunityProvider)),
        "community-subject",
        r#"{"id":"community-subject","picture":"https://community.test/me.png"}"#,
        Some("https://community.test/me.png"),
    )
    .await;
}

#[tokio::test]
async fn an_empty_url_from_a_community_provider_reports_none() {
    if crate::own_process_async::delegate(
        module_path!(),
        "an_empty_url_from_a_community_provider_reports_none",
    )
    .await
    {
        return;
    }
    callback(
        Arc::new(PictureCommunityProvider(CommunityProvider)),
        "community-subject",
        r#"{"id":"community-subject","picture":""}"#,
        None,
    )
    .await;
}
