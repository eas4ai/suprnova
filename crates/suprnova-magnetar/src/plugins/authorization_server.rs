//! Mounted OAuth 2.1 authorization server routes.
//!
//! Six routes under the configured prefix (`/oauth` by default):
//!
//! | Method | Path | Purpose |
//! |---|---|---|
//! | GET | `authorize` | Authorization endpoint |
//! | GET | `authorizations/{id}` | The pending authorization, as JSON for the host's approval page |
//! | POST | `authorizations/{id}` | The user's decision, form field `decision` = `approve` or `deny` |
//! | POST | `token` | Token endpoint |
//! | POST | `revoke` | Revocation endpoint (RFC 7009) |
//! | POST | `register` | Dynamic client registration (RFC 7591) |
//!
//! The two approval routes need the signed-in user's session and answer
//! with headers that forbid framing. Every route bounds the request it
//! reads.
//!
//! # What the host must do
//!
//! The plugin sees a [`WireRequest`] the host has already read and parsed,
//! so a few protections belong to the host:
//!
//! - **CSRF.** The decision route changes state on the user's behalf, so
//!   the host protects it with its CSRF check. The token, revocation and
//!   registration routes are called by clients and must not need one.
//! - **Repeated parameters.** RFC 6749 section 3.1 forbids sending an OAuth
//!   parameter twice. The query and form maps the plugin receives hold one
//!   value per name, so the host must refuse a request that repeats one
//!   before building them. A form body handed over as raw bytes is checked
//!   here.
//! - **The body bound.** The plugin's size check measures a body already
//!   in memory. The host must bound the body before reading it, at least
//!   to [`max_request_bytes`](crate::authorization_server::AuthorizationServerConfig::max_request_bytes).
//! - **The client address.** The registration and metadata-fetch budgets
//!   are counted per `x-client-ip` header. The host must always set it from
//!   its trusted proxy resolution and drop any value the client sent;
//!   without it every request shares one anonymous budget.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::Error;
use crate::authorization_server::{
    AuthorizationServer, AuthorizeOutcome, AuthorizeRequest, ClientMetadata, DecisionOutcome,
    RegistrationOutcome, TokenError, TokenRequest,
};
use crate::plugin::{
    Effect, EffectResponse, Method, Plugin, PluginError, PluginResult, RequestContext,
    RouteDescriptor, WireBody, WireRequest, WireResponse,
};
use crate::schema::AuthSchema;

const FEATURE: &str = "authorization-server";

/// Headers that keep the approval routes out of frames, so a page cannot
/// overlay them and trick the user into approving.
const NO_FRAMING: [(&str, &str); 2] = [
    ("x-frame-options", "DENY"),
    ("content-security-policy", "frame-ancestors 'none'"),
];

/// Headers that keep tokens and registrations out of caches (RFC 6749
/// section 5.1).
const NO_STORE: [(&str, &str); 2] = [("cache-control", "no-store"), ("pragma", "no-cache")];

/// The authorization server route group.
pub struct AuthorizationServerPlugin {
    server: Arc<AuthorizationServer>,
}

impl AuthorizationServerPlugin {
    /// Mount `server`'s endpoints.
    #[must_use]
    pub fn new(server: Arc<AuthorizationServer>) -> Self {
        Self { server }
    }

    /// The engine behind the routes, for the verification API and the
    /// metadata document.
    #[must_use]
    pub fn server(&self) -> &Arc<AuthorizationServer> {
        &self.server
    }
}

fn respond(status: u16, body: Option<Value>, headers: &[(&str, &str)]) -> WireResponse {
    let mut response = EffectResponse::ok();
    response.status = status;
    response.body = body;
    for (name, value) in headers {
        response
            .headers
            .insert((*name).to_owned(), (*value).to_owned());
    }
    WireResponse::from_effects(response)
}

fn redirect(location: String, status: u16, headers: &[(&str, &str)]) -> WireResponse {
    let mut response = EffectResponse::json(json!({"redirect_to": location}));
    response.status = status;
    for (name, value) in headers {
        response
            .headers
            .insert((*name).to_owned(), (*value).to_owned());
    }
    WireResponse::from_effects(response.with_effect(Effect::Redirect { location, status }))
}

fn oauth_error(
    status: u16,
    error: &str,
    description: &str,
    headers: &[(&str, &str)],
) -> WireResponse {
    respond(
        status,
        Some(json!({"error": error, "error_description": description})),
        headers,
    )
}

/// Answer a storage or limiter failure without its detail.
fn failure(error: &Error, headers: &[(&str, &str)]) -> WireResponse {
    tracing::error!(error = %error, "authorization server request failed");
    match error {
        Error::DependencyUnavailable { .. } => respond(
            503,
            Some(json!({"error": "temporarily_unavailable"})),
            headers,
        ),
        _ => respond(500, Some(json!({"error": "server_error"})), headers),
    }
}

fn token_error(error: &TokenError) -> WireResponse {
    respond(error.status(), Some(error.to_json()), &NO_STORE)
}

/// The size of what a route would read: the query and the body, counted
/// only as far as `limit` so a huge JSON value is not serialized whole.
fn request_size(request: &WireRequest, limit: usize) -> usize {
    let pairs = |fields: &BTreeMap<String, String>| {
        fields.iter().fold(0_usize, |total, (name, value)| {
            total.saturating_add(name.len() + value.len() + 2)
        })
    };
    let body = match &request.body {
        WireBody::Empty => 0,
        WireBody::Form(fields) => pairs(fields),
        WireBody::Bytes { bytes, .. } => bytes.len(),
        WireBody::Json(value) => json_size(value, limit),
    };
    pairs(&request.query).saturating_add(body)
}

fn json_size(value: &Value, limit: usize) -> usize {
    struct Counter {
        written: usize,
        limit: usize,
    }
    impl std::io::Write for Counter {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.written = self.written.saturating_add(buffer.len());
            if self.written > self.limit {
                return Err(std::io::Error::other("over the request bound"));
            }
            Ok(buffer.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter { written: 0, limit };
    // Stopping early is the point: the error only says the bound was
    // passed, which `written` already shows.
    let _ = serde_json::to_writer(&mut counter, value);
    counter.written
}

fn has_content_type(content_type: Option<&String>, expected: &str) -> bool {
    content_type.is_some_and(|value| {
        value
            .split(';')
            .next()
            .is_some_and(|media| media.trim().eq_ignore_ascii_case(expected))
    })
}

/// The form parameters of a request. A repeated parameter is refused, as
/// RFC 6749 section 3.2 requires.
fn form_params(request: &WireRequest) -> Result<BTreeMap<String, String>, &'static str> {
    match &request.body {
        WireBody::Empty => Ok(BTreeMap::new()),
        WireBody::Form(fields) => Ok(fields.clone()),
        WireBody::Bytes {
            content_type,
            bytes,
        } if has_content_type(content_type.as_ref(), "application/x-www-form-urlencoded") => {
            let mut params = BTreeMap::new();
            for (name, value) in url::form_urlencoded::parse(bytes) {
                if params
                    .insert(name.into_owned(), value.into_owned())
                    .is_some()
                {
                    return Err("a parameter is repeated");
                }
            }
            Ok(params)
        }
        _ => Err("the body must be application/x-www-form-urlencoded"),
    }
}

/// The client metadata of a registration request's JSON body.
fn registration_body(request: &WireRequest) -> Result<ClientMetadata, String> {
    let parsed = match &request.body {
        WireBody::Json(value) => serde_json::from_value(value.clone()),
        WireBody::Bytes {
            content_type,
            bytes,
        } if has_content_type(content_type.as_ref(), "application/json") => {
            serde_json::from_slice(bytes)
        }
        _ => return Err("the body must be a JSON object".to_owned()),
    };
    parsed.map_err(|error| crate::crypto::decode_failure(&error))
}

/// The `decision` field of a form or JSON body.
fn decision(request: &WireRequest) -> Option<String> {
    match &request.body {
        WireBody::Json(value) => value
            .get("decision")
            .and_then(Value::as_str)
            .map(str::to_owned),
        _ => form_params(request).ok()?.remove("decision"),
    }
}

fn client_address(request: &WireRequest) -> Option<&str> {
    request.headers.get("x-client-ip").map(String::as_str)
}

/// The path and query of the request, to come back to after sign-in.
fn return_address(request: &WireRequest) -> String {
    if request.query.is_empty() {
        return request.path.clone();
    }
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    for (name, value) in &request.query {
        query.append_pair(name, value);
    }
    format!("{}?{}", request.path, query.finish())
}

impl AuthorizationServerPlugin {
    async fn authorize<S: AuthSchema>(&self, context: &RequestContext<'_, S>) -> WireResponse {
        let request = context.request;
        let return_to = return_address(request);
        match self
            .server
            .authorize(AuthorizeRequest {
                query: &request.query,
                session: context.session,
                client_address: client_address(request),
                return_to: &return_to,
            })
            .await
        {
            Ok(AuthorizeOutcome::Redirect(location)) => redirect(location, 302, &NO_FRAMING),
            Ok(AuthorizeOutcome::Refused { error, description }) => {
                let headers = [NO_FRAMING[0], NO_FRAMING[1], NO_STORE[0]];
                oauth_error(400, &error, &description, &headers)
            }
            Err(error) => failure(&error, &NO_FRAMING),
        }
    }

    async fn pending<S: AuthSchema>(
        &self,
        context: &RequestContext<'_, S>,
        id: &str,
    ) -> WireResponse {
        let headers = [NO_FRAMING[0], NO_FRAMING[1], NO_STORE[0]];
        let Some(session) = context.session else {
            return oauth_error(401, "login_required", "sign in to continue", &headers);
        };
        match self.server.pending_authorization(id, session).await {
            Ok(Some(view)) => respond(200, Some(view.to_json()), &headers),
            Ok(None) => oauth_error(404, "not_found", "no such pending authorization", &headers),
            Err(error) => failure(&error, &headers),
        }
    }

    async fn decide<S: AuthSchema>(
        &self,
        context: &RequestContext<'_, S>,
        id: &str,
    ) -> WireResponse {
        let headers = [NO_FRAMING[0], NO_FRAMING[1], NO_STORE[0]];
        let Some(session) = context.session else {
            return oauth_error(401, "login_required", "sign in to continue", &headers);
        };
        let approve = match decision(context.request).as_deref() {
            Some("approve") => true,
            Some("deny") => false,
            _ => {
                return oauth_error(
                    400,
                    "invalid_request",
                    "decision must be approve or deny",
                    &headers,
                );
            }
        };
        match self.server.decide(id, session, approve).await {
            Ok(DecisionOutcome::Redirect(location)) => redirect(location, 303, &headers),
            Ok(DecisionOutcome::NotFound) => {
                oauth_error(404, "not_found", "no such pending authorization", &headers)
            }
            Ok(DecisionOutcome::StaleSession) => {
                oauth_error(401, "login_required", "sign in again to continue", &headers)
            }
            Err(error) => failure(&error, &headers),
        }
    }

    async fn token(&self, request: &WireRequest) -> WireResponse {
        let params = match form_params(request) {
            Ok(params) => params,
            Err(reason) => return token_error(&TokenError::invalid_request(reason)),
        };
        let token_request = TokenRequest::new(params, client_address(request).map(str::to_owned));
        match self.server.token(&token_request).await {
            Ok(response) => respond(200, Some(response.to_json()), &NO_STORE),
            Err(error) => token_error(&error),
        }
    }

    async fn revoke(&self, request: &WireRequest) -> WireResponse {
        let params = match form_params(request) {
            Ok(params) => params,
            Err(reason) => return token_error(&TokenError::invalid_request(reason)),
        };
        match self
            .server
            .revoke_from(&params, client_address(request))
            .await
        {
            Ok(()) => respond(200, None, &NO_STORE),
            Err(error) => token_error(&error),
        }
    }

    async fn register(&self, request: &WireRequest) -> WireResponse {
        let metadata = match registration_body(request) {
            Ok(metadata) => metadata,
            Err(reason) => {
                return oauth_error(400, "invalid_client_metadata", &reason, &NO_STORE);
            }
        };
        match self
            .server
            .register_client(metadata, client_address(request))
            .await
        {
            Ok(RegistrationOutcome::Registered(body)) => respond(201, Some(body), &NO_STORE),
            Ok(RegistrationOutcome::Refused(error)) => {
                respond(400, Some(error.to_json()), &NO_STORE)
            }
            Ok(RegistrationOutcome::RateLimited { retry_after }) => {
                let seconds = retry_after.as_secs().max(1).to_string();
                oauth_error(
                    429,
                    "too_many_requests",
                    "too many registrations from this address",
                    &[NO_STORE[0], ("retry-after", seconds.as_str())],
                )
            }
            Err(error) => failure(&error, &NO_STORE),
        }
    }
}

#[async_trait]
impl<S: AuthSchema> Plugin<S> for AuthorizationServerPlugin {
    fn name(&self) -> &str {
        "authorization-server"
    }

    fn routes(&self) -> Vec<RouteDescriptor> {
        let route = |method: Method, suffix: &str, name: &str| {
            RouteDescriptor::new(method, self.server.route_path(suffix), name).with_feature(FEATURE)
        };
        vec![
            route(Method::Get, "authorize", "authorization-server.authorize"),
            route(
                Method::Get,
                "authorizations/{id}",
                "authorization-server.pending",
            ),
            route(
                Method::Post,
                "authorizations/{id}",
                "authorization-server.decide",
            ),
            route(Method::Post, "token", "authorization-server.token"),
            route(Method::Post, "revoke", "authorization-server.revoke"),
            route(Method::Post, "register", "authorization-server.register"),
        ]
    }

    async fn handle(&self, context: RequestContext<'_, S>) -> PluginResult<WireResponse> {
        let request = context.request;
        let limit = self.server.config().max_request_bytes;
        if request_size(request, limit) > limit {
            return Ok(oauth_error(
                413,
                "invalid_request",
                "the request is too large",
                &NO_STORE,
            ));
        }
        let path = request.path.trim_end_matches('/');
        let server = &self.server;
        if path == server.route_path("authorize") && request.method == Method::Get {
            return Ok(self.authorize(&context).await);
        }
        if request.method == Method::Post {
            if path == server.route_path("token") {
                return Ok(self.token(request).await);
            }
            if path == server.route_path("revoke") {
                return Ok(self.revoke(request).await);
            }
            if path == server.route_path("register") {
                return Ok(self.register(request).await);
            }
        }
        if let Some(id) = request.path_params.get("id")
            && path == server.route_path(&format!("authorizations/{id}"))
        {
            match request.method {
                Method::Get => return Ok(self.pending(&context, id).await),
                Method::Post => return Ok(self.decide(&context, id).await),
                _ => {}
            }
        }
        Err(PluginError::RouteNotFound {
            path: request.path.clone(),
        })
    }
}
