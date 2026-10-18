//! Serving a Magnetar plugin's routes through the framework router.
//!
//! A Magnetar plugin reads a framework-neutral [`WireRequest`] and answers a
//! [`WireResponse`]; this host turns a framework [`Request`] into the one
//! and the other into a framework response. A plugin leaves a few
//! protections to its host (each plugin's "What the host must do"), and the
//! host gives every one of them to every route it mounts, so no plugin
//! depends on its caller remembering them:
//!
//! - **The client address.** `x-client-ip` is always the address the
//!   framework's trusted-proxy resolution gives ([`Request::ip`]). A value
//!   the client sent is dropped, so a client cannot choose the budget its
//!   requests count against.
//! - **Repeated parameters.** A query, or an
//!   `application/x-www-form-urlencoded` body, that names a parameter twice
//!   is refused with 400 before the one-value-per-name maps the plugin
//!   reads are built (RFC 6749 section 3.1).
//! - **The body bound.** The body is read no further than the mount's
//!   bound, and a declared length over it is refused with 413 before a byte
//!   is read.
//! - **CSRF.** A route a client calls, named with
//!   [`PluginMount::client_route`], is agent traffic: it gets no session, no
//!   CSRF check and no cookie (AGENT-011). Every other route is a browser
//!   route: it reaches the plugin with the user's session, and one that
//!   changes state runs [`CsrfMiddleware`] itself, whether or not the
//!   application installed it globally. A route left unnamed is therefore
//!   protected, never exempt.
//! - **The session.** A browser route hands the plugin the signed-in user
//!   as the [`VerifiedSession`] Magnetar's engines take (see
//!   [`signed_in_session`]). A client route hands it none.
//! - **The answer.** The plugin's status, headers and body are sent as it
//!   wrote them and its redirect becomes a `Location`. An effect this host
//!   does not perform, such as signing a user in or out, fails the request
//!   rather than being dropped.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use chrono::Utc;
use magnetar::auth::{AuthenticationContext, FactorGate, SignInDecision, VerifiedPrincipal};
use magnetar::default_schema::DefaultAuthSchema;
use magnetar::default_schema::sql_stores::SqlSessionStore;
use magnetar::plugin::{
    Effect, EffectResponse, Encryptor, HttpRequest, HttpResponse as PluginHttpResponse,
    HttpTransport, InitContext, LinkGenerator, MailDriver, MailMessage, Method as WireMethod,
    Plugin, PluginContext, PluginError, RequestContext, RouteDescriptor, WireBody, WireRequest,
    WireResponse,
};
use magnetar::sessions::{
    HostSessionApproval, OpaqueConfig, OpaqueSessionProvider, OpaqueSessionStore, SessionGrant,
    SessionMetadata, SessionQueries, StoredSession, VerifiedSession, WebSessionBinding,
};
use magnetar::storage::SeaOrmStorage;
use sea_orm::DatabaseConnection;
use serde_json::{Value, json};

use super::abuse_limiter::AppAbuseLimiter;
use crate::auth::{Auth, Authenticatable};
use crate::csrf::CsrfMiddleware;
use crate::error::FrameworkError;
use crate::http::{HttpResponse, NOT_FOUND_BODY, Request, Response};
use crate::routing::{RouteBuilder, Router};

/// The epoch a user whose model exposes neither an epoch counter nor a
/// password hash is held at. HRD-007 does not fence such a user's
/// sessions, so a grant to one is not fenced either; if the user later
/// gains a credential, the epoch moves off this value and the grant ends.
const NO_EPOCH: u64 = 0;

/// How long a session built for one browser request stays valid. It is
/// never stored or sent anywhere: the plugin reads it during the request.
const HOST_SESSION_LIFETIME: chrono::Duration = chrono::Duration::minutes(5);

/// One Magnetar plugin, ready to mount on a router.
pub(crate) struct PluginMount {
    plugin: Arc<dyn Plugin<DefaultAuthSchema>>,
    context: PluginContext<DefaultAuthSchema>,
    client_routes: Vec<String>,
    max_body_bytes: usize,
}

impl PluginMount {
    /// Mount `plugin`, with `connection` behind the storage and session
    /// queries its context offers. Every route is a browser route until
    /// [`client_route`](Self::client_route) names it, and bodies are bound
    /// to 16 KiB until [`max_body_bytes`](Self::max_body_bytes) says
    /// otherwise.
    pub(crate) fn new(
        plugin: Arc<dyn Plugin<DefaultAuthSchema>>,
        connection: DatabaseConnection,
    ) -> Self {
        Self {
            plugin,
            context: host_context(connection),
            client_routes: Vec::new(),
            max_body_bytes: 16 * 1024,
        }
    }

    /// Serve the route named `name` to clients: no session, no cookie and
    /// no CSRF check. Mounting fails if the plugin declares no such route,
    /// so a misspelt name cannot leave a route quietly unprotected or
    /// unreachable.
    pub(crate) fn client_route(mut self, name: &str) -> Self {
        self.client_routes.push(name.to_owned());
        self
    }

    /// The largest request body any route reads, in bytes. A plugin's own
    /// bound measures a body already in memory, so this one must be at
    /// least as tight.
    pub(crate) fn max_body_bytes(mut self, bytes: usize) -> Self {
        self.max_body_bytes = bytes;
        self
    }
}

/// A mounted plugin, shared by its routes' handlers.
struct Mounted {
    plugin: Arc<dyn Plugin<DefaultAuthSchema>>,
    context: PluginContext<DefaultAuthSchema>,
    routes: Vec<MountedRoute>,
    max_body_bytes: usize,
    initialized: tokio::sync::OnceCell<Result<(), String>>,
}

struct MountedRoute {
    descriptor: RouteDescriptor,
    client: bool,
}

impl Router {
    /// Serve `mount`'s enabled routes (see the module documentation for
    /// what each route gets).
    ///
    /// # Errors
    ///
    /// When a route name the mount gave is not one of the plugin's, a route
    /// uses a method the router does not serve, or a route's path is
    /// already registered.
    pub(crate) fn try_mount_magnetar_plugin(
        self,
        mount: PluginMount,
    ) -> Result<Router, FrameworkError> {
        let descriptors: Vec<RouteDescriptor> = mount
            .plugin
            .routes()
            .into_iter()
            .filter(|route| route.enabled)
            .collect();
        for name in &mount.client_routes {
            if !descriptors.iter().any(|route| &route.name == name) {
                return Err(FrameworkError::internal(format!(
                    "the Magnetar plugin '{}' declares no route named '{name}'",
                    mount.plugin.name()
                )));
            }
        }
        let routes: Vec<MountedRoute> = descriptors
            .into_iter()
            .map(|descriptor| MountedRoute {
                client: mount.client_routes.contains(&descriptor.name),
                descriptor,
            })
            .collect();
        let plan: Vec<(WireMethod, String, bool)> = routes
            .iter()
            .map(|route| {
                (
                    route.descriptor.method.clone(),
                    route.descriptor.path.clone(),
                    route.client,
                )
            })
            .collect();
        let mounted = Arc::new(Mounted {
            plugin: mount.plugin,
            context: mount.context,
            routes,
            max_body_bytes: mount.max_body_bytes,
            initialized: tokio::sync::OnceCell::new(),
        });
        let mut router = self;
        for (index, (method, path, client)) in plan.into_iter().enumerate() {
            let state = Arc::clone(&mounted);
            let handler = move |request: Request| {
                let state = Arc::clone(&state);
                async move { serve(state, index, request).await }
            };
            let (builder, method): (RouteBuilder, hyper::Method) = match method {
                WireMethod::Get => (router.try_get(&path, handler)?, hyper::Method::GET),
                WireMethod::Post => (router.try_post(&path, handler)?, hyper::Method::POST),
                WireMethod::Put => (router.try_put(&path, handler)?, hyper::Method::PUT),
                WireMethod::Patch => (router.try_patch(&path, handler)?, hyper::Method::PATCH),
                WireMethod::Delete => (router.try_delete(&path, handler)?, hyper::Method::DELETE),
                other => {
                    return Err(FrameworkError::internal(format!(
                        "the Magnetar plugin route {path} uses {other:?}, which the plugin host \
                         does not serve"
                    )));
                }
            };
            let reads = matches!(method, hyper::Method::GET);
            router = if client || reads {
                builder.into()
            } else {
                // The plugin acts on the signed-in user's behalf here, so the
                // token is checked even where the application installed no
                // CSRF middleware. A global one hands out the cookie; this
                // one only checks.
                builder
                    .middleware(CsrfMiddleware::new().without_xsrf_cookie())
                    .into()
            };
            // A descriptor's template uses `{parameter}` segments, the
            // router's own syntax, so the path is the pattern the router
            // matches and marks requests by.
            if client {
                router.mark_client_route(method, path);
            }
        }
        Ok(router)
    }
}

/// Answer one request to the mounted route `index`.
async fn serve(state: Arc<Mounted>, index: usize, request: Request) -> Response {
    let route = &state.routes[index];
    let browser = !route.client;
    let query = match single_valued(request.query().unwrap_or_default().as_bytes()) {
        Ok(query) => query,
        Err(Repeated) => return Ok(refusal(400, "a parameter is repeated", browser)),
    };
    let Some(captures) = route.descriptor.match_path(request.path()) else {
        return Ok(not_found());
    };
    let headers = wire_headers(&request);
    let method = WireMethod::parse(request.method().as_str());
    let path = request.path().to_owned();
    let content_type = request.content_type().map(str::to_owned);
    let bytes = match request.body_bytes_with_cap(state.max_body_bytes).await {
        Ok((_, bytes)) => bytes,
        Err(error) if error.status_code() == 413 => {
            return Ok(refusal(413, "the request is too large", browser));
        }
        Err(error) => return Err(error.into()),
    };
    if content_type
        .as_deref()
        .is_some_and(crate::http::body::is_form_urlencoded)
        && single_valued(&bytes).is_err()
    {
        return Ok(refusal(400, "a parameter is repeated", browser));
    }
    let body = if bytes.is_empty() {
        WireBody::Empty
    } else {
        WireBody::Bytes {
            content_type,
            bytes: bytes.to_vec(),
        }
    };
    let session = if browser {
        signed_in_session().await?
    } else {
        None
    };
    let initialized = state
        .initialized
        .get_or_init(|| async {
            state
                .plugin
                .init(InitContext::new(&state.context))
                .await
                .map_err(|error| error.to_string())
        })
        .await;
    if let Err(error) = initialized {
        return Err(FrameworkError::internal(format!(
            "the Magnetar plugin '{}' failed to initialize: {error}",
            state.plugin.name()
        ))
        .into());
    }
    let wire = WireRequest {
        method,
        path,
        path_params: captures.into_iter().collect(),
        query,
        headers,
        body,
    };
    let context = RequestContext::with_session(&state.context, &wire, session.as_ref());
    match state.plugin.handle(context).await {
        Ok(response) => into_response(response).map_err(Into::into),
        Err(PluginError::RouteNotFound { .. }) => Ok(not_found()),
        Err(error) => {
            tracing::error!(
                plugin = state.plugin.name(),
                %error,
                "a mounted Magnetar plugin failed a request"
            );
            Err(FrameworkError::internal(format!(
                "the Magnetar plugin '{}' failed the request",
                state.plugin.name()
            ))
            .into())
        }
    }
}

/// A parameter named twice.
struct Repeated;

/// The pairs of a query string or urlencoded form, one value per name, or
/// [`Repeated`] when a name appears twice. Names are compared after
/// percent-decoding, so `a=1&%61=2` repeats `a`.
fn single_valued(raw: &[u8]) -> Result<BTreeMap<String, String>, Repeated> {
    let mut pairs = BTreeMap::new();
    for (name, value) in url::form_urlencoded::parse(raw) {
        if pairs
            .insert(name.into_owned(), value.into_owned())
            .is_some()
        {
            return Err(Repeated);
        }
    }
    Ok(pairs)
}

/// The request's headers as the plugin reads them: names lowercased, a
/// repeated header joined with `, `, values that are not text left out.
/// The cookie never reaches a plugin, which gets the session from the host
/// instead, and `x-client-ip` is the framework's own resolution of the
/// client address, never the client's.
fn wire_headers(request: &Request) -> BTreeMap<String, String> {
    let mut headers: BTreeMap<String, String> = BTreeMap::new();
    for (name, value) in request.headers() {
        let name = name.as_str().to_ascii_lowercase();
        if name == "cookie" || name == "x-client-ip" {
            continue;
        }
        let Ok(value) = value.to_str() else {
            continue;
        };
        headers
            .entry(name)
            .and_modify(|joined| {
                joined.push_str(", ");
                joined.push_str(value);
            })
            .or_insert_with(|| value.to_owned());
    }
    if let Some(address) = request.ip() {
        headers.insert("x-client-ip".to_owned(), address);
    }
    headers
}

/// The headers that keep a browser route's answer out of frames and caches.
const UNFRAMED: [(&str, &str); 2] = [
    ("x-frame-options", "DENY"),
    ("content-security-policy", "frame-ancestors 'none'"),
];

/// The host's own refusal: an OAuth `invalid_request` error, never cached,
/// and kept out of frames on a browser route.
fn refusal(status: u16, description: &str, browser: bool) -> HttpResponse {
    let mut response = HttpResponse::json(json!({
        "error": "invalid_request",
        "error_description": description,
    }))
    .status(status)
    .header("Cache-Control", "no-store");
    if browser {
        for (name, value) in UNFRAMED {
            response = response.header(name, value);
        }
    }
    response
}

/// The router's own 404, so a path the plugin does not serve reads as an
/// unrouted one.
fn not_found() -> HttpResponse {
    HttpResponse::text(NOT_FOUND_BODY).status(404)
}

/// Headers the server writes for itself; a plugin's value would conflict
/// with the body the host sends.
const FRAMING_HEADERS: [&str; 3] = ["content-length", "transfer-encoding", "connection"];

/// The framework response for a plugin's answer: its status, headers and
/// body, with its effects applied in order.
fn into_response(response: WireResponse) -> Result<HttpResponse, FrameworkError> {
    let EffectResponse {
        mut status,
        headers,
        mut body,
        effects,
    } = response.into_effects();
    let mut headers: BTreeMap<String, String> = headers
        .into_iter()
        .map(|(name, value)| (name.to_ascii_lowercase(), value))
        .collect();
    for effect in effects {
        match effect {
            Effect::Redirect {
                location,
                status: code,
            } => {
                status = code;
                headers.insert("location".to_owned(), location);
            }
            Effect::SetStatus(code) => status = code,
            Effect::SetHeader { name, value } => {
                headers.insert(name.to_ascii_lowercase(), value);
            }
            Effect::Json(value) => body = Some(value),
            Effect::EstablishSession(_) | Effect::ClearSession | Effect::IssueRemember(_) => {
                return Err(FrameworkError::internal(
                    "a mounted Magnetar plugin asked its host to sign a user in or out; the \
                     framework signs users in through its own flows, never through a plugin",
                ));
            }
        }
    }
    if !(100..=599).contains(&status) {
        return Err(FrameworkError::internal(format!(
            "a mounted Magnetar plugin answered with status {status}"
        )));
    }
    let content_type = headers.remove("content-type");
    let (payload, content_type) = match (body, content_type) {
        (None, content_type) => (Bytes::new(), content_type),
        // A document the plugin labels as something other than JSON, such
        // as agent auth's `/auth.md`, is its text, sent as written.
        (Some(Value::String(text)), Some(content_type)) if !names_json(&content_type) => {
            (Bytes::from(text), Some(content_type))
        }
        (Some(value), content_type) => (
            Bytes::from(value.to_string()),
            Some(content_type.unwrap_or_else(|| "application/json".to_owned())),
        ),
    };
    let mut http = match content_type {
        Some(content_type) => HttpResponse::bytes_body(payload, content_type),
        None => HttpResponse::new(),
    }
    .status(status);
    for (name, value) in headers {
        if FRAMING_HEADERS.contains(&name.as_str()) {
            continue;
        }
        if hyper::header::HeaderName::from_bytes(name.as_bytes()).is_err()
            || hyper::header::HeaderValue::from_str(&value).is_err()
        {
            return Err(FrameworkError::internal(format!(
                "a mounted Magnetar plugin answered with an invalid header '{name}'"
            )));
        }
        http = http.header(name, value);
    }
    Ok(http)
}

/// Whether a content type names JSON (`application/json` or a `+json`
/// type).
fn names_json(content_type: &str) -> bool {
    let media = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    media == "application/json" || media.ends_with("+json")
}

/// The signed-in user of the default guard as the [`VerifiedSession`] a
/// Magnetar engine takes, or `None` for a guest.
///
/// The session middleware has already checked the sign-in when this runs,
/// and [`Auth::user`] resolves the user through the guard's provider. Which
/// session stands for them depends on who owns the guard's sessions:
///
/// - **Magnetar** ([`magnetar_fences`](crate::auth::epoch::magnetar_fences)):
///   the guard's sign-in carries a binding to a Magnetar session, and the
///   installed engine resolves that binding to the session itself, at
///   Magnetar's epoch. A binding that no longer resolves, or resolves to
///   another user, is a guest.
/// - **The framework's own session guard**: no Magnetar session exists, so
///   one is built for this request through Magnetar's public session
///   provider, over a view whose only row is the signed-in user at their
///   HRD-007 epoch ([`framework_epoch`]). It is never stored and never
///   leaves the request.
///
/// Either way the session carries the epoch [`HostUserEpochs`] reads back
/// for the same user, so a grant approved in it ends when the user's
/// sessions do.
///
/// # Errors
///
/// When the guard's provider or the Magnetar engine cannot answer.
pub(crate) async fn signed_in_session() -> Result<Option<VerifiedSession>, FrameworkError> {
    let guard_name = Auth::default_guard_name();
    let user = match crate::App::get::<crate::auth::AuthManager>() {
        Some(manager) => manager.guard(&guard_name)?.user().await?,
        None => Auth::user().await?,
    };
    let Some(user) = user else {
        return Ok(None);
    };
    let user_id = user.get_auth_identifier();
    if crate::auth::epoch::magnetar_fences(&guard_name) {
        return magnetar_session(&guard_name, &user_id).await;
    }
    framework_session(&user_id, framework_epoch(user.as_ref()))
        .await
        .map(Some)
        .map_err(|error| {
            FrameworkError::internal(format!("build the signed-in user's session: {error}"))
        })
}

/// The Magnetar session the guard's sign-in is bound to.
async fn magnetar_session(
    guard_name: &str,
    user_id: &str,
) -> Result<Option<VerifiedSession>, FrameworkError> {
    let binding = crate::session::session().and_then(|session| {
        session
            .auth_guard_magnetar_binding(guard_name)
            .or_else(|| session.magnetar_web_binding())
    });
    let (Some(binding), Some(authority)) = (binding, super::host_sign_in_authority()) else {
        return Ok(None);
    };
    match authority.resolve_web_binding(&binding).await {
        Ok(verified) if verified.user_id() == user_id => Ok(Some(verified)),
        Ok(_)
        | Err(
            magnetar::Error::InvalidInput { .. }
            | magnetar::Error::NotFound { .. }
            | magnetar::Error::Conflict { .. },
        ) => Ok(None),
        Err(error) => {
            tracing::error!(%error, "the Magnetar engine could not resolve the session binding");
            Err(FrameworkError::domain(
                "the sign-in engine is unavailable",
                503,
            ))
        }
    }
}

/// The bits of an HRD-007 epoch a grant keeps: Magnetar stores a grant's
/// epoch in a signed 64-bit column, and a password-hash digest uses all 64
/// bits, so half of all digests would not fit. Dropping the top bit leaves
/// a 63-bit digest, which still changes with every password write but for
/// a 2^-63 chance.
const GRANT_EPOCH_BITS: u64 = i64::MAX as u64;

/// The epoch a grant approved by `user` records under the framework's own
/// session guard: the HRD-007 epoch, the one the session middleware
/// compares the user's sign-ins against, kept to the 63 bits a grant
/// stores. The session handed to the plugin and [`HostUserEpochs`] both
/// read it here, so the value recorded at approval is the value compared
/// on every later use.
pub(crate) fn framework_epoch(user: &dyn Authenticatable) -> u64 {
    crate::auth::epoch::credential_epoch(user).map_or(NO_EPOCH, |epoch| epoch & GRANT_EPOCH_BITS)
}

/// A session for `user_id` at `epoch`, built through Magnetar's opaque
/// session provider over a view that holds only that session.
async fn framework_session(user_id: &str, epoch: u64) -> magnetar::Result<VerifiedSession> {
    let mut random = [0_u8; 48];
    getrandom::fill(&mut random).map_err(|error| magnetar::Error::Internal {
        message: format!("operating system randomness: {error}"),
    })?;
    let session_id = hex::encode(&random[..16]);
    let mut digest = [0_u8; 32];
    digest.copy_from_slice(&random[16..]);
    let session = StoredSession {
        session_id: session_id.clone(),
        user_id: user_id.to_owned(),
        auth_epoch: epoch,
        token_hash: digest,
        token_digest: digest,
        expires_at: Utc::now() + HOST_SESSION_LIFETIME,
        revoked_at: None,
        metadata: SessionMetadata::default(),
    };
    let provider =
        OpaqueSessionProvider::new(Arc::new(SignedInView(session)), OpaqueConfig::default());
    provider
        .resolve_web_binding(
            &WebSessionBinding {
                session_id,
                token_digest: digest,
            },
            &HostSessionApproval::authenticated(),
        )
        .await
}

/// The one session the framework's guard signed in this request, as
/// Magnetar's opaque session store reads it. Nothing is ever written.
struct SignedInView(StoredSession);

#[async_trait]
impl OpaqueSessionStore for SignedInView {
    async fn insert_session_if_epoch_current(
        &self,
        _session: StoredSession,
    ) -> magnetar::Result<()> {
        Err(unoffered("session issuance"))
    }

    async fn find_by_token_hash(
        &self,
        _token_hash: [u8; 32],
    ) -> magnetar::Result<Option<StoredSession>> {
        Ok(None)
    }

    async fn find_by_web_binding(
        &self,
        binding: &WebSessionBinding,
    ) -> magnetar::Result<Option<StoredSession>> {
        Ok((binding.session_id == self.0.session_id).then(|| self.0.clone()))
    }

    async fn revoke_all_sessions(
        &self,
        _user_id: &str,
        _at: chrono::DateTime<Utc>,
    ) -> magnetar::Result<u64> {
        Ok(0)
    }

    async fn revoke_session(
        &self,
        _session_id: &str,
        _at: chrono::DateTime<Utc>,
    ) -> magnetar::Result<bool> {
        Ok(false)
    }

    async fn list_active_sessions(
        &self,
        _user_id: &str,
        _now: chrono::DateTime<Utc>,
    ) -> magnetar::Result<Vec<StoredSession>> {
        Ok(Vec::new())
    }
}

/// The user's authentication epoch as a Magnetar engine reads it, decided
/// the way [`signed_in_session`] decides the epoch a session carries: where
/// Magnetar fences the default guard, Magnetar's own epoch from its user
/// table; otherwise the HRD-007 epoch of the user the guard's provider
/// returns. A grant records the session's epoch at approval and stops
/// working once this moves, as the user's sessions do (AGENT-005).
pub(crate) struct HostUserEpochs {
    magnetar_users: Arc<SeaOrmStorage<DefaultAuthSchema>>,
}

impl HostUserEpochs {
    /// Epochs over `connection`, whose Magnetar user table is read when
    /// Magnetar owns the default guard's sessions.
    pub(crate) fn new(connection: DatabaseConnection) -> Self {
        Self {
            magnetar_users: Arc::new(SeaOrmStorage::new(connection)),
        }
    }
}

#[async_trait]
impl magnetar::authorization_server::UserEpochs for HostUserEpochs {
    async fn current_epoch(&self, user_id: &str) -> magnetar::Result<Option<u64>> {
        let guard_name = Auth::default_guard_name();
        if crate::auth::epoch::magnetar_fences(&guard_name) {
            return magnetar::authorization_server::UserEpochs::current_epoch(
                self.magnetar_users.as_ref(),
                user_id,
            )
            .await;
        }
        let Some(provider) = crate::auth::epoch::guard_provider(&guard_name) else {
            return Err(magnetar::Error::DependencyUnavailable {
                dependency: "user provider".to_owned(),
                message: format!("the default guard '{guard_name}' has no user provider"),
            });
        };
        match provider.retrieve_by_id(user_id).await {
            Ok(Some(user)) => Ok(Some(framework_epoch(user.as_ref()))),
            Ok(None) => Ok(None),
            Err(error) => Err(magnetar::Error::DependencyUnavailable {
                dependency: "user provider".to_owned(),
                message: error.to_string(),
            }),
        }
    }
}

/// The context mounted plugins get: storage and session queries over the
/// application's connection, and the framework's rate limiter for their
/// budgets. The framework signs users in, encrypts, mails, fetches and
/// links through its own facades, so a plugin that reaches for one of those
/// capabilities here gets an error naming it rather than a stand-in.
fn host_context(connection: DatabaseConnection) -> PluginContext<DefaultAuthSchema> {
    let sessions: Arc<dyn SessionQueries> = Arc::new(OpaqueSessionProvider::new(
        Arc::new(SqlSessionStore(connection.clone())),
        OpaqueConfig::default(),
    ));
    PluginContext::new(
        Arc::new(SeaOrmStorage::<DefaultAuthSchema>::new(connection)),
        sessions,
        Arc::new(Unoffered),
        Arc::new(Unoffered),
        Arc::new(AppAbuseLimiter),
        Arc::new(Unoffered),
        Arc::new(Unoffered),
        Arc::new(Unoffered),
    )
}

/// A capability the framework's plugin host does not offer mounted plugins.
struct Unoffered;

fn unoffered(capability: &str) -> magnetar::Error {
    magnetar::Error::DependencyUnavailable {
        dependency: capability.to_owned(),
        message: "the framework's Magnetar plugin host does not offer it to mounted plugins"
            .to_owned(),
    }
}

#[async_trait]
impl FactorGate for Unoffered {
    async fn complete_sign_in(
        &self,
        _principal: VerifiedPrincipal,
        _context: AuthenticationContext,
    ) -> magnetar::Result<SignInDecision> {
        Err(unoffered("sign-in"))
    }

    async fn complete_challenge(
        &self,
        _selector: &str,
        _code: &str,
    ) -> magnetar::Result<SessionGrant> {
        Err(unoffered("sign-in"))
    }
}

#[async_trait]
impl Encryptor for Unoffered {
    async fn encrypt(&self, _plaintext: &[u8]) -> magnetar::Result<Vec<u8>> {
        Err(unoffered("encryption"))
    }

    async fn decrypt(&self, _ciphertext: &[u8]) -> magnetar::Result<Vec<u8>> {
        Err(unoffered("encryption"))
    }
}

#[async_trait]
impl MailDriver for Unoffered {
    async fn send(&self, _message: MailMessage) -> magnetar::Result<()> {
        Err(unoffered("mail"))
    }
}

#[async_trait]
impl HttpTransport for Unoffered {
    async fn send(&self, _request: HttpRequest) -> magnetar::Result<PluginHttpResponse> {
        Err(unoffered("outbound HTTP"))
    }
}

#[async_trait]
impl LinkGenerator for Unoffered {
    async fn url_for(
        &self,
        _route_name: &str,
        _params: &[(String, String)],
    ) -> magnetar::Result<String> {
        Err(unoffered("link generation"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_005_single_valued_refuses_a_repeated_name_after_decoding() {
        assert!(single_valued(b"a=1&b=2").is_ok());
        assert!(single_valued(b"a=1&a=2").is_err());
        assert!(single_valued(b"a=1&%61=2").is_err());
        assert_eq!(
            single_valued(b"code=x%20y")
                .ok()
                .and_then(|pairs| pairs.get("code").cloned()),
            Some("x y".to_owned())
        );
    }

    #[test]
    fn agent_005_plugin_effects_become_the_response() {
        let mut effects = EffectResponse::json(json!({"redirect_to": "https://c.test/cb"}));
        effects.status = 200;
        effects
            .headers
            .insert("Cache-Control".to_owned(), "no-store".to_owned());
        let effects = effects.with_effect(Effect::Redirect {
            location: "https://c.test/cb?code=1".to_owned(),
            status: 303,
        });
        let response = into_response(WireResponse::from_effects(effects))
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(response.status_code(), 303);
    }

    struct Hashed(String);

    impl Authenticatable for Hashed {
        fn get_auth_identifier(&self) -> String {
            "7".to_owned()
        }
        fn get_auth_password(&self) -> Option<&str> {
            Some(&self.0)
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn into_arc_any(self: Arc<Self>) -> Arc<dyn std::any::Any + Send + Sync> {
            self
        }
    }

    #[test]
    fn agent_005_framework_epoch_fits_the_grant_column() {
        // Half of all password-hash digests have the top bit set; find one,
        // so the check does not depend on the hash a run happens to use.
        let high = (0..)
            .map(|n| Hashed(format!("$argon2id$hash-{n}")))
            .find(|user| {
                crate::auth::epoch::credential_epoch(user)
                    .is_some_and(|epoch| epoch > GRANT_EPOCH_BITS)
            })
            .unwrap_or_else(|| Hashed(String::new()));
        let epoch = framework_epoch(&high);
        assert!(
            i64::try_from(epoch).is_ok(),
            "{epoch} must fit a signed column"
        );
        let changed = framework_epoch(&Hashed(format!("{}-changed", high.0)));
        assert_ne!(epoch, changed, "a new hash is a new epoch");
    }

    #[test]
    fn agent_005_a_sign_in_effect_fails_the_request() {
        let effects = EffectResponse::ok().with_effect(Effect::ClearSession);
        assert!(into_response(WireResponse::from_effects(effects)).is_err());
    }
}
