//! Mounted agent auth routes (auth.md v0.6.0).
//!
//! Under the configured prefix (`/agent` by default), plus the document:
//!
//! | Method | Path | Purpose |
//! |---|---|---|
//! | POST | `identity` | Identity endpoint: register by `anonymous`, `service_auth` or `identity_assertion` |
//! | POST | `identity/claim` | Claim endpoint: start a claim attempt with the claim token and the user's email |
//! | GET | `identity/claim/attempt` | The pending claim, as JSON for the host's claim page (query `claim_attempt_token`) |
//! | POST | `identity/claim/complete` | The claim page's form action: `claim_attempt_token` and `user_code` |
//! | POST | `event/notify` | Security Event Tokens from trusted providers (RFC 8935); only with ID-JAGs on |
//! | GET | the skill path, `/auth.md` by default | The instructions an agent reads |
//!
//! The token endpoint grants (`urn:ietf:params:oauth:grant-type:jwt-bearer`
//! and `urn:workos:agent-auth:grant-type:claim`) and revocation live on the
//! authorization server's routes; [`AgentAuth::install`] adds them.
//!
//! # What the host must do
//!
//! The plugin sees a [`WireRequest`] the host has already read and parsed,
//! so a few protections belong to the host:
//!
//! - **CSRF.** The confirm route (`identity/claim/complete`) changes state
//!   on the signed-in user's behalf, so the host protects it with its CSRF
//!   check; the plugin refuses it without a signed-in user. The identity,
//!   claim, events and document routes are called by agents and providers
//!   and must not need a CSRF token, as the authorization server's token,
//!   revocation and registration routes must not.
//! - **The session.** The two claim page routes read the signed-in user
//!   from the session the host binds, and compare the session's epoch with
//!   the user's.
//! - **Repeated parameters.** The query and form maps hold one value per
//!   name, so the host must refuse a request that repeats one before
//!   building them.
//! - **The body bound.** The plugin's size check measures a body already in
//!   memory. The host must bound the body before reading it, at least to
//!   [`max_request_bytes`](crate::agent_auth::AgentAuthConfig::max_request_bytes).
//!   The events route takes the raw `application/secevent+jwt` bytes.
//! - **The client address.** Registration budgets and audit events use the
//!   `x-client-ip` header. The host must always set it from its trusted
//!   proxy resolution and drop any value the client sent.
//! - **The document body.** The `/auth.md` route answers the document text
//!   as a JSON string with `content-type: text/markdown`; the host writes
//!   the string out as the body.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::agent_auth::{AgentAuth, SignedInUser};
use crate::plugin::{
    EffectResponse, Method, Plugin, PluginError, PluginResult, RequestContext, RouteDescriptor,
    WireBody, WireRequest, WireResponse,
};
use crate::schema::AuthSchema;

const FEATURE: &str = "agent-auth";

/// The agent auth route group.
pub struct AgentAuthPlugin {
    engine: Arc<AgentAuth>,
}

impl AgentAuthPlugin {
    /// Mount `engine`'s routes.
    #[must_use]
    pub fn new(engine: Arc<AgentAuth>) -> Self {
        Self { engine }
    }

    /// The engine behind the routes.
    #[must_use]
    pub fn engine(&self) -> &Arc<AgentAuth> {
        &self.engine
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

const NO_STORE: (&str, &str) = ("cache-control", "no-store");

fn invalid_request(status: u16, description: &str) -> WireResponse {
    respond(
        status,
        Some(json!({
            "error": "invalid_request",
            "error_description": description,
            "message": description,
        })),
        &[NO_STORE],
    )
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

fn media_type(content_type: Option<&String>) -> Option<String> {
    content_type.and_then(|value| {
        value
            .split(';')
            .next()
            .map(|media| media.trim().to_ascii_lowercase())
    })
}

/// The JSON body of an agent request.
fn json_body(request: &WireRequest) -> Result<Value, &'static str> {
    match &request.body {
        WireBody::Json(value) => Ok(value.clone()),
        WireBody::Bytes {
            content_type,
            bytes,
        } if media_type(content_type.as_ref()).is_none_or(|media| media == "application/json") => {
            serde_json::from_slice(bytes).map_err(|_| "the body is not JSON")
        }
        _ => Err("the body must be application/json"),
    }
}

/// The fields of the claim page's form post, as a form or a JSON object.
fn form_fields(request: &WireRequest) -> Result<BTreeMap<String, String>, &'static str> {
    match &request.body {
        WireBody::Form(fields) => Ok(fields.clone()),
        WireBody::Json(Value::Object(fields)) => Ok(fields
            .iter()
            .filter_map(|(name, value)| {
                value.as_str().map(|value| (name.clone(), value.to_owned()))
            })
            .collect()),
        WireBody::Bytes {
            content_type,
            bytes,
        } if media_type(content_type.as_ref()).as_deref()
            == Some("application/x-www-form-urlencoded") =>
        {
            let mut fields = BTreeMap::new();
            for (name, value) in url::form_urlencoded::parse(bytes) {
                if fields
                    .insert(name.into_owned(), value.into_owned())
                    .is_some()
                {
                    return Err("a field is repeated");
                }
            }
            Ok(fields)
        }
        _ => Err("the body must be a form"),
    }
}

fn client_address(request: &WireRequest) -> Option<&str> {
    request.headers.get("x-client-ip").map(String::as_str)
}

impl AgentAuthPlugin {
    fn signed_in<S: AuthSchema>(context: &RequestContext<'_, S>) -> Option<SignedInUser> {
        context.session.map(|session| SignedInUser {
            user_id: session.user_id().to_owned(),
            auth_epoch: Some(session.auth_epoch()),
        })
    }

    fn is(&self, path: &str, suffix: &str) -> bool {
        path == self.engine.route_path(suffix).trim_end_matches('/')
    }
}

#[async_trait]
impl<S: AuthSchema> Plugin<S> for AgentAuthPlugin {
    fn name(&self) -> &str {
        "agent-auth"
    }

    fn routes(&self) -> Vec<RouteDescriptor> {
        let route = |method: Method, suffix: &str, name: &str| {
            RouteDescriptor::new(method, self.engine.route_path(suffix), name).with_feature(FEATURE)
        };
        let mut events = route(Method::Post, "event/notify", "agent-auth.events");
        if !self.engine.core().serves_events() {
            events = events.disabled();
        }
        vec![
            route(Method::Post, "identity", "agent-auth.identity"),
            route(Method::Post, "identity/claim", "agent-auth.claim"),
            route(
                Method::Get,
                "identity/claim/attempt",
                "agent-auth.claim-attempt",
            ),
            route(
                Method::Post,
                "identity/claim/complete",
                "agent-auth.claim-complete",
            ),
            events,
            RouteDescriptor::new(
                Method::Get,
                self.engine.config().skill_path.clone(),
                "agent-auth.document",
            )
            .with_feature(FEATURE),
        ]
    }

    async fn handle(&self, context: RequestContext<'_, S>) -> PluginResult<WireResponse> {
        let request = context.request;
        let engine = &self.engine;
        let path = request.path.trim_end_matches('/');
        let address = client_address(request);
        let limit = engine.config().max_request_bytes;
        let oversized = request_size(request, limit) > limit;
        if request.method == Method::Post && self.is(path, "event/notify") {
            if !engine.core().serves_events() {
                return Err(PluginError::RouteNotFound {
                    path: request.path.clone(),
                });
            }
            if oversized {
                return Ok(respond(
                    413,
                    Some(
                        json!({"err": "invalid_request", "description": "the request is too large"}),
                    ),
                    &[NO_STORE],
                ));
            }
            let (content_type, bytes) = match &request.body {
                WireBody::Bytes {
                    content_type,
                    bytes,
                } => (content_type.as_deref(), bytes.as_slice()),
                _ => (None, &[][..]),
            };
            return Ok(engine.receive_event(content_type, bytes, address).await);
        }
        if oversized {
            return Ok(invalid_request(413, "the request is too large"));
        }
        if request.method == Method::Get && path == engine.config().skill_path.trim_end_matches('/')
        {
            return Ok(respond(
                200,
                Some(Value::String(engine.auth_md())),
                &[("content-type", "text/markdown; charset=utf-8")],
            ));
        }
        if request.method == Method::Post && self.is(path, "identity") {
            return Ok(match json_body(request) {
                Ok(body) => engine.identity(&body, address).await,
                Err(reason) => invalid_request(400, reason),
            });
        }
        if request.method == Method::Post && self.is(path, "identity/claim") {
            return Ok(match json_body(request) {
                Ok(body) => engine.initiate_claim(&body, address).await,
                Err(reason) => invalid_request(400, reason),
            });
        }
        if request.method == Method::Get && self.is(path, "identity/claim/attempt") {
            let token = request
                .query
                .get("claim_attempt_token")
                .map_or("", String::as_str);
            let signed_in = Self::signed_in(&context);
            return Ok(engine.pending_claim(token, signed_in.as_ref()).await);
        }
        if request.method == Method::Post && self.is(path, "identity/claim/complete") {
            let signed_in = Self::signed_in(&context);
            if signed_in.is_none() {
                // Nothing about the post is read before sign-in.
                return Ok(engine.confirm_claim("", "", None, address).await);
            }
            let fields = match form_fields(request) {
                Ok(fields) => fields,
                Err(reason) => {
                    let mut response = invalid_request(400, reason).into_effects();
                    response
                        .headers
                        .insert("x-frame-options".to_owned(), "DENY".to_owned());
                    response.headers.insert(
                        "content-security-policy".to_owned(),
                        "frame-ancestors 'none'".to_owned(),
                    );
                    return Ok(WireResponse::from_effects(response));
                }
            };
            let token = fields.get("claim_attempt_token").map_or("", String::as_str);
            let code = fields.get("user_code").map_or("", String::as_str);
            return Ok(engine
                .confirm_claim(token, code, signed_in.as_ref(), address)
                .await);
        }
        Err(PluginError::RouteNotFound {
            path: request.path.clone(),
        })
    }
}
