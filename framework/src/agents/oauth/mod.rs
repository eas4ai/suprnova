//! The Magnetar authorization server, set up for the MCP endpoint
//! (AGENT-005, the client half of AGENT-010 and the framework pages of
//! AGENT-017). [`OAuthServer`] documents what it builds and mounts.

mod grants;
mod pages;
mod resolver;

use std::sync::Arc;
use std::time::Duration;

use magnetar::authorization_server::{
    AuthorizationServer, AuthorizationServerConfig, ClientMetadataFetcher, ClientMetadataPolicy,
    ScopeDefinition,
};
use magnetar::plugins::authorization_server::AuthorizationServerPlugin;
use sea_orm::DatabaseConnection;

pub use grants::{ClientGrant, ConnectedAgents, GrantScope};

use super::{AgentsConfig, CallLimit};
use crate::error::FrameworkError;
use crate::magnetar_integration::plugin_host::{HostUserEpochs, PluginMount};
use crate::routing::Router;

/// The authorization server's route names that clients call. They get no
/// session and need no CSRF token; the authorization endpoint and the two
/// approval routes are browser routes.
const CLIENT_ROUTES: [&str; 3] = [
    "authorization-server.token",
    "authorization-server.revoke",
    "authorization-server.register",
];

/// How an MCP client signs a user in: the Magnetar authorization server,
/// set up from the application's configuration.
///
/// [`AgentsConfig::oauth_server`] turns it on, and every setting has a
/// default, so `OAuthServer::new()` is a working server:
///
/// ```rust,ignore
/// use suprnova::agents::{AgentsConfig, OAuthServer};
///
/// pub fn register() -> Router {
///     Router::new()
///         .post("/posts", posts::store)
///         .agents(AgentsConfig::new().oauth_server(OAuthServer::new()))
/// }
/// ```
///
/// When the router is built, the framework builds the server from the
/// application's configuration:
///
/// - **issuer**: the application's public URL (`APP_URL`), path prefix
///   included, so its metadata is at `<issuer>/.well-known/...` under the
///   application's own root;
/// - **resource**: the MCP endpoint's canonical URL, the one a token is
///   bound to;
/// - **scopes**: every scope `tool_scopes!` declared, with its description;
/// - **sign-in**: the application's login route (`/login` with
///   `return_to`, unless set);
/// - **client metadata documents**: fetched through Magnetar's fenced
///   [`ReqwestPinnedClient`](magnetar::authorization_server::ReqwestPinnedClient);
/// - **budgets**: the framework's rate limiter;
/// - **storage**: the application's database connection.
///
/// It mounts the authorization server's routes under `/oauth`, the default
/// approval page at `/oauth/approve` and the default connected-agents page
/// at `/oauth/connected`, registers the resolver that turns its access
/// tokens into user callers, and serves its metadata at the well-known
/// locations. Its four tables come from [`OAuthServer::migrate`], or from the
/// Magnetar default engine's migrations when
/// [`init_magnetar`](crate::init_magnetar) applies them.
///
/// # Replacing the pages
///
/// The default pages are plain server-rendered HTML. An application that
/// renders its own approval page names it with
/// [`custom_approval_page`](Self::custom_approval_page); the authorization
/// endpoint then sends the user to that path with the pending
/// authorization's id in the `authorization` query parameter. The page
/// reads the pending authorization as JSON from
/// `GET /oauth/authorizations/{id}` and posts the user's choice to
/// `POST /oauth/authorizations/{id}` as the form field `decision`
/// (`approve` or `deny`) with the CSRF token. An application that lists
/// grants itself calls
/// [`custom_connected_agents_page`](Self::custom_connected_agents_page) and
/// reads and revokes them through [`ConnectedAgents`].
#[derive(Clone)]
pub struct OAuthServer {
    connection: Option<DatabaseConnection>,
    route_prefix: String,
    sign_in_path: String,
    return_to_parameter: String,
    approval_path: String,
    default_approval_page: bool,
    connected_path: Option<String>,
    fetcher: Option<Arc<ClientMetadataFetcher>>,
    access_token_lifetime: Option<Duration>,
    refresh_token_lifetime: Option<Duration>,
    registration_limit: Option<CallLimit>,
}

impl std::fmt::Debug for OAuthServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthServer")
            .field("route_prefix", &self.route_prefix)
            .field("sign_in_path", &self.sign_in_path)
            .field("approval_path", &self.approval_path)
            .field("default_approval_page", &self.default_approval_page)
            .field("connected_path", &self.connected_path)
            .finish_non_exhaustive()
    }
}

impl Default for OAuthServer {
    fn default() -> Self {
        Self::new()
    }
}

impl OAuthServer {
    /// The server over the application's database connection, with its
    /// routes under `/oauth`, sign-in at `/login?return_to=...`, the default
    /// approval page at `/oauth/approve` and the default connected-agents
    /// page at `/oauth/connected`, one-hour access tokens, 30-day refresh
    /// tokens and 20 dynamic registrations an hour per client address.
    pub fn new() -> Self {
        Self {
            connection: None,
            route_prefix: "/oauth".to_owned(),
            sign_in_path: "/login".to_owned(),
            return_to_parameter: "return_to".to_owned(),
            approval_path: "/oauth/approve".to_owned(),
            default_approval_page: true,
            connected_path: Some("/oauth/connected".to_owned()),
            fetcher: None,
            access_token_lifetime: None,
            refresh_token_lifetime: None,
            registration_limit: None,
        }
    }

    /// Keep clients, grants and token digests through `connection` rather
    /// than the application's default connection. It holds only the
    /// authorization server's own tables: where Magnetar owns the default
    /// guard's sessions, a grant is checked against the user's epoch through
    /// the installed engine, wherever that keeps its users.
    pub fn connection(mut self, connection: DatabaseConnection) -> Self {
        self.connection = Some(connection);
        self
    }

    /// Mount the authorization server's routes under `prefix` instead of
    /// `/oauth`. A prefix without a leading `/` gets one.
    pub fn route_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.route_prefix = leading_slash(prefix.into());
        self
    }

    /// Send a user who is not signed in to `path`, with the address to
    /// return to in the query parameter `return_to_parameter`. The address
    /// is a path relative to the application's public root, as the
    /// framework's own redirects take one.
    pub fn sign_in(
        mut self,
        path: impl Into<String>,
        return_to_parameter: impl Into<String>,
    ) -> Self {
        self.sign_in_path = leading_slash(path.into());
        self.return_to_parameter = return_to_parameter.into();
        self
    }

    /// Serve the framework's approval page at `path` instead of
    /// `/oauth/approve`.
    pub fn approval_page(mut self, path: impl Into<String>) -> Self {
        self.approval_path = leading_slash(path.into());
        self.default_approval_page = true;
        self
    }

    /// Send users to the application's own approval page at `path`, and
    /// serve no default page. The page must require the signed-in user,
    /// name the client, its host and every requested scope, post the
    /// decision with the CSRF token and refuse to be framed (AGENT-005);
    /// the type's "Replacing the pages" section names the two routes it
    /// uses.
    pub fn custom_approval_page(mut self, path: impl Into<String>) -> Self {
        self.approval_path = leading_slash(path.into());
        self.default_approval_page = false;
        self
    }

    /// Serve the framework's connected-agents page at `path` instead of
    /// `/oauth/connected`.
    pub fn connected_agents_page(mut self, path: impl Into<String>) -> Self {
        self.connected_path = Some(leading_slash(path.into()));
        self
    }

    /// Serve no default connected-agents page: the application lists and
    /// revokes grants itself through [`ConnectedAgents`].
    pub fn custom_connected_agents_page(mut self) -> Self {
        self.connected_path = None;
        self
    }

    /// Fetch client metadata documents through `fetcher` instead of the
    /// system resolver and Magnetar's pinned HTTPS client. A test gives a
    /// fetcher over a resolver and client of its own, so no test reaches
    /// the network.
    pub fn client_metadata_fetcher(mut self, fetcher: Arc<ClientMetadataFetcher>) -> Self {
        self.fetcher = Some(fetcher);
        self
    }

    /// How long an access token works: one hour unless set. Whole seconds,
    /// at least one.
    pub fn access_token_lifetime(mut self, lifetime: Duration) -> Self {
        self.access_token_lifetime = Some(lifetime);
        self
    }

    /// How long a refresh token works if it is not used: 30 days unless
    /// set. Whole seconds, at least one.
    pub fn refresh_token_lifetime(mut self, lifetime: Duration) -> Self {
        self.refresh_token_lifetime = Some(lifetime);
        self
    }

    /// How many dynamic client registrations one client address may make
    /// in a window: 20 an hour unless set.
    pub fn registration_limit(mut self, limit: CallLimit) -> Self {
        self.registration_limit = Some(limit);
        self
    }

    /// Create the authorization server's tables on `connection` if they do
    /// not exist: `auth_oauth_clients`, `auth_oauth_authorizations`,
    /// `auth_oauth_grants` and `auth_oauth_tokens`. Safe to run on every
    /// boot. The Magnetar default engine's migrations create them too.
    ///
    /// # Errors
    ///
    /// When the database refuses a statement.
    pub async fn migrate(connection: &DatabaseConnection) -> Result<(), FrameworkError> {
        magnetar::authorization_server::migrate(connection)
            .await
            .map_err(|error| {
                FrameworkError::internal(format!(
                    "create the authorization server's tables: {error}"
                ))
            })
    }

    /// Build the engine for the endpoint `config` describes.
    ///
    /// The issuer is the application's public URL, and a client checks the
    /// `iss` of every authorization response against it, so an `APP_URL`
    /// left to its development default is refused outside the local and
    /// testing environments rather than published.
    pub(crate) fn build(self, config: &AgentsConfig) -> Result<BuiltOAuthServer, FrameworkError> {
        if !app_url_named()
            && !matches!(
                crate::config::Config::environment(),
                crate::config::Environment::Local | crate::config::Environment::Testing
            )
        {
            return Err(FrameworkError::internal(
                "the OAuth server's issuer is the application's public URL, and APP_URL is not \
                 set: set APP_URL to the URL clients reach the application at (it may be left \
                 unset only in the local and testing environments; an unset APP_ENV is \
                 production)",
            ));
        }
        let connection = match &self.connection {
            Some(connection) => connection.clone(),
            None => crate::DB::connection()
                .map_err(|error| {
                    FrameworkError::internal(format!(
                        "the OAuth server keeps its grants in the application's database, and \
                         none is connected: {error}"
                    ))
                })?
                .inner()
                .clone(),
        };
        let issuer = crate::routing::url::to("").trim_end_matches('/').to_owned();
        let resource = super::metadata::resource_url(config);
        let mut settings = AuthorizationServerConfig::new(issuer, vec![resource]);
        settings.route_prefix = self.route_prefix.clone();
        settings.scopes = super::declared_scopes()
            .into_iter()
            .map(|scope| ScopeDefinition::new(scope.name(), scope.description()))
            .collect();
        settings.sign_in_url = crate::routing::url::to(&self.sign_in_path);
        settings.return_to_parameter = self.return_to_parameter.clone();
        settings.approval_url = crate::routing::url::to(&self.approval_path);
        if let Some(lifetime) = self.access_token_lifetime {
            settings.access_token_lifetime = lifetime;
        }
        if let Some(lifetime) = self.refresh_token_lifetime {
            settings.refresh_token_lifetime = lifetime;
        }
        if let Some(limit) = self.registration_limit {
            settings.registration_policy = magnetar::abuse::AbusePolicy {
                max_requests: limit.max_calls,
                window: limit.window,
            };
        }
        let fetcher = self.fetcher.clone().unwrap_or_else(|| {
            Arc::new(ClientMetadataFetcher::system(
                ClientMetadataPolicy::default(),
            ))
        });
        let server = AuthorizationServer::new(
            settings,
            Arc::new(magnetar::authorization_server::SqlAuthorizationStore::new(
                connection.clone(),
            )),
            Arc::new(HostUserEpochs),
            Arc::new(crate::magnetar_integration::abuse_limiter::AppAbuseLimiter),
            fetcher,
        )
        .map_err(|error| {
            FrameworkError::internal(format!(
                "the OAuth server's configuration is invalid: {error}"
            ))
        })?;
        Ok(BuiltOAuthServer {
            server: Arc::new(server),
            setup: self,
            connection,
        })
    }
}

/// An authorization server built for one router, ready to mount.
pub(crate) struct BuiltOAuthServer {
    server: Arc<AuthorizationServer>,
    setup: OAuthServer,
    connection: DatabaseConnection,
}

impl BuiltOAuthServer {
    /// The resolver that turns the server's access tokens into user
    /// callers.
    pub(crate) fn resolver(&self) -> Arc<dyn super::CallerResolver> {
        Arc::new(resolver::UserTokens::new(Arc::clone(&self.server)))
    }

    /// The server's metadata, for the well-known routes.
    pub(crate) fn metadata(&self) -> Arc<dyn super::AuthorizationServerMetadata> {
        Arc::new(resolver::ServerMetadata::new(Arc::clone(&self.server)))
    }

    /// The server, which every request the router routes carries for
    /// [`ConnectedAgents`].
    pub(crate) fn server(&self) -> Arc<AuthorizationServer> {
        Arc::clone(&self.server)
    }

    /// Mount the server's routes and the default pages on `router`.
    pub(crate) fn mount(self, router: Router) -> Result<Router, FrameworkError> {
        let max_body_bytes = self.server.config().max_request_bytes;
        let mut mount = PluginMount::new(
            Arc::new(AuthorizationServerPlugin::new(Arc::clone(&self.server))),
            self.connection,
        )
        .max_body_bytes(max_body_bytes);
        for name in CLIENT_ROUTES {
            mount = mount.client_route(name);
        }
        let mut router = router.try_mount_magnetar_plugin(mount)?;
        let pages = Arc::new(pages::Pages::new(Arc::clone(&self.server), &self.setup));
        if self.setup.default_approval_page {
            router = pages::mount_approval(router, Arc::clone(&pages), &self.setup.approval_path)?;
        }
        if let Some(path) = &self.setup.connected_path {
            router = pages::mount_connected(router, Arc::clone(&pages), path)?;
        }
        Ok(router)
    }
}

impl AgentsConfig {
    /// Let MCP clients sign users in through the Magnetar authorization
    /// server (AGENT-005), set up as [`OAuthServer`] describes: its routes,
    /// the default approval and connected-agents pages, the resolver that
    /// accepts its access tokens, asked before any other, and its metadata
    /// at the well-known locations.
    pub fn oauth_server(mut self, server: OAuthServer) -> Self {
        self.oauth = Some(server);
        self
    }
}

/// Whether the application names its public URL: `APP_URL` is set, or a
/// registered `AppConfig` carries a URL other than the default it takes
/// when `APP_URL` is unset.
fn app_url_named() -> bool {
    std::env::var("APP_URL").is_ok_and(|url| !url.trim().is_empty())
        || crate::config::Config::get::<crate::config::AppConfig>().is_some_and(|config| {
            !config.url.trim().is_empty() && config.url != crate::config::providers::DEFAULT_APP_URL
        })
}

/// `path` with a leading `/`.
fn leading_slash(path: String) -> String {
    if path.starts_with('/') {
        path
    } else {
        format!("/{path}")
    }
}
