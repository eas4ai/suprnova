use std::sync::Arc;

use suprnova::{
    AbuseLimiter, AutoLinkPolicy, DB, DatabaseConnection, EndpointOverrides, GoogleOAuthProvider,
    GoogleProviderConfig, MagnetarConfig, MagnetarOAuthHostConfig, MagnetarOAuthProviderConfig,
    OAuthAuthorizationConfig, OAuthHttpTransport, PasskeyConfig, RevocationTransport, SecretString,
    init_magnetar,
};

fn auth_config(
    database: DatabaseConnection,
    transport: Arc<dyn OAuthHttpTransport>,
    revocation: Arc<dyn RevocationTransport>,
    limiter: Arc<dyn AbuseLimiter>,
) -> MagnetarConfig {
    let provider = Arc::new(GoogleOAuthProvider::new(
        GoogleProviderConfig {
            client_id: "google-client".to_owned(),
            client_secret: SecretString::from("google-secret".to_owned()),
            redirect_uri: Some("https://app.example.com/auth/google/callback".to_owned()),
            scopes: vec!["openid".to_owned(), "email".to_owned()],
            endpoints: EndpointOverrides::default(),
        },
        revocation,
    ));
    let oauth = MagnetarOAuthHostConfig::new(
        vec![MagnetarOAuthProviderConfig {
            provider,
            redirect_uri: "https://app.example.com/auth/google/callback".to_owned(),
            scopes: vec!["openid".to_owned(), "email".to_owned()],
        }],
        transport,
        limiter,
        OAuthAuthorizationConfig::default(),
        AutoLinkPolicy::default(),
    )
    .expect("valid OAuth host configuration");

    MagnetarConfig::from_sea_orm(database)
        .passkey_config(PasskeyConfig {
            rp_id: "app.example.com".to_owned(),
            rp_origin: "https://app.example.com".to_owned(),
        })
        .oauth(oauth)
}

pub async fn register_auth(
    transport: Arc<dyn OAuthHttpTransport>,
    revocation: Arc<dyn RevocationTransport>,
    limiter: Arc<dyn AbuseLimiter>,
) -> Result<(), suprnova::FrameworkError> {
    let database = DB::connection()?;
    init_magnetar(auth_config(
        database.inner().clone(),
        transport,
        revocation,
        limiter,
    ))
    .await
}

fn main() {}
