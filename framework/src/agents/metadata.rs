//! OAuth discovery for MCP clients (AGENT-004).
//!
//! An MCP client that is refused with 401 reads the protected resource
//! metadata (RFC 9728) the `WWW-Authenticate` header names, learns the
//! authorization servers that issue tokens for the endpoint, and reads
//! their metadata (RFC 8414). Every document and URL is built from the
//! application's public root, so an application served under a path prefix
//! is discoverable without owning the host root.

use serde_json::json;

use super::{AgentRoute, AgentsConfig};
use crate::http::{HttpResponse, Request, Response};

/// The well-known path of protected resource metadata (RFC 9728).
pub(crate) const PROTECTED_RESOURCE_PATH: &str = "/.well-known/oauth-protected-resource";

/// The canonical URL of the MCP endpoint: the resource a token must be
/// bound to.
pub(crate) fn resource_url(config: &AgentsConfig) -> String {
    crate::routing::url::to(&config.path)
}

/// The URL of the endpoint's protected resource metadata, as the
/// `resource_metadata` of a `WWW-Authenticate` header names it.
pub(crate) fn resource_metadata_url(config: &AgentsConfig) -> String {
    crate::routing::url::to(&format!("{PROTECTED_RESOURCE_PATH}{}", config.path))
}

/// The application's public URL, the issuer of an authorization server the
/// application runs itself.
fn issuer_url() -> String {
    crate::routing::url::to("")
        .trim_end_matches('/')
        .to_string()
}

/// The route of the protected resource metadata.
pub(crate) async fn protected_resource(request: Request) -> Response {
    let Some(AgentRoute::ProtectedResource(config)) = route_of(&request) else {
        return Ok(not_found());
    };
    let Some(endpoint) = request.agent_mark().and_then(|mark| mark.as_endpoint()) else {
        return Ok(not_found());
    };
    let scopes: Vec<&str> = match endpoint.router.tool_registry() {
        Ok(registry) => registry.scopes().iter().map(|scope| scope.name()).collect(),
        Err(error) => return Err(error.into()),
    };
    let mut servers: Vec<String> = config
        .resolvers
        .iter()
        .flat_map(|resolver| resolver.authorization_servers())
        .collect();
    if let Some(provider) = &config.authorization_server {
        servers.push(provider.issuer(&issuer_url()));
    }
    servers.sort();
    servers.dedup();
    let name = config.resource_name.clone().unwrap_or_else(app_name);
    Ok(HttpResponse::json(json!({
        "resource": resource_url(&config),
        "authorization_servers": servers,
        "scopes_supported": scopes,
        "bearer_methods_supported": ["header"],
        "resource_name": name,
    })))
}

/// The route of the authorization server metadata: the registered
/// provider's document, or 404 without one.
pub(crate) async fn authorization_server(request: Request) -> Response {
    let Some(AgentRoute::AuthorizationServer(config, kind)) = route_of(&request) else {
        return Ok(not_found());
    };
    let Some(provider) = &config.authorization_server else {
        return Ok(not_found());
    };
    match provider.document(&issuer_url(), kind).await {
        Some(document) => Ok(HttpResponse::json(document)),
        None => Ok(not_found()),
    }
}

/// What the agents route the request matched serves.
fn route_of(request: &Request) -> Option<AgentRoute> {
    request
        .agent_mark()
        .and_then(|mark| mark.as_endpoint())
        .map(|endpoint| endpoint.route.clone())
}

/// The router's own 404, so an unserved document reads as an unrouted path.
fn not_found() -> HttpResponse {
    HttpResponse::text(crate::http::NOT_FOUND_BODY).status(404)
}

/// The application's name, `APP_NAME`.
pub(crate) fn app_name() -> String {
    crate::config::Config::get::<crate::config::AppConfig>()
        .map(|config| config.name)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "Suprnova".to_string())
}
