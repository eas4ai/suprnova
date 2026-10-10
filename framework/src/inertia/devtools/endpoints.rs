//! The two endpoints the extension reads entries from: Laravel's
//! `EntriesController` behind `Authorize`, `PreserveFlashData` and
//! `PreventPreviousUrlTracking`.
//!
//! The extension fetches an entry the moment a response's headers arrive,
//! racing the redirect the application is about to follow. So an endpoint
//! request keeps the flash data a `POST` left for the page after the
//! redirect, and never becomes the session's previous URL, which the
//! application's next `back()` would otherwise send the visitor to.

use std::sync::Arc;

use serde_json::{Value, json};

use super::config::DevToolsConfig;
use super::store::EntriesRepository;
use super::ulid::is_ulid;
use crate::http::{HttpResponse, Request, Response};

/// The path the endpoints are under, without its leading slash.
const PREFIX: &str = "_inertia/devtools/entries";

/// One of the two endpoints.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Endpoint {
    /// `GET /_inertia/devtools/entries`: every entry's metadata.
    List,
    /// `GET /_inertia/devtools/entries/{id}`: one stored entry.
    Show(String),
}

impl Endpoint {
    /// The endpoint `request` asks for, if it asks for one.
    pub(crate) fn of(request: &Request) -> Option<Self> {
        if *request.method() != hyper::Method::GET {
            return None;
        }
        let path = request.path().trim_matches('/');
        let rest = path.strip_prefix(PREFIX)?;
        if rest.is_empty() {
            return Some(Self::List);
        }
        let id = rest.strip_prefix('/')?;
        (!id.is_empty() && !id.contains('/')).then(|| Self::Show(id.to_string()))
    }
}

/// `{"message": message}` with `status`.
fn message(status: u16, message: &str) -> HttpResponse {
    HttpResponse::json(json!({"message": message})).status(status)
}

/// Answer `endpoint` for `request`.
///
/// The request is kept from becoming the previous URL and the session is
/// reflashed whatever the answer, a `403` included, as Laravel's
/// middleware runs both around its authorization.
pub(crate) async fn answer(
    config: &Arc<DevToolsConfig>,
    repository: &Arc<EntriesRepository>,
    endpoint: Endpoint,
    request: &Request,
) -> Response {
    crate::session::middleware::exempt_from_previous_url();
    let response = if allows(config).await {
        match endpoint {
            Endpoint::List => list(repository, request).await,
            Endpoint::Show(id) => show(repository, id).await,
        }
    } else {
        message(403, "Forbidden.")
    };
    crate::session::session_mut(|session| session.reflash());
    Ok(response)
}

/// Whether the request may read entries, Laravel's `Authorize`: always
/// when `APP_ENV` names the `local` environment, since a failing gate would
/// lock a developer out of their own tools; elsewhere, an unset `APP_ENV`
/// included, only when the configured gate ability allows the signed-in
/// user, or a guest when no one is signed in. A guest is asked about as `()`, as is the resource, so a gate that
/// admits users is defined as `Gate::define::<User, ()>(ability, ...)`.
async fn allows(config: &DevToolsConfig) -> bool {
    if super::config::app_env_names_local() {
        return true;
    }
    let Some(ability) = config.gate.as_deref().filter(|gate| !gate.is_empty()) else {
        return false;
    };
    match crate::auth::Auth::user().await {
        Ok(Some(user)) => {
            let user = user.into_arc_any();
            crate::authorization::Gate::inspect_erased_async(ability, &*user, &())
                .await
                .allowed()
        }
        Ok(None) => crate::authorization::Gate::allows_async(ability, &(), &()).await,
        Err(error) => {
            tracing::debug!(%error, "Inertia DevTools: the user could not be resolved; denied");
            false
        }
    }
}

/// A comma list of request types: trimmed, empty ones dropped.
fn type_list(value: Option<&String>) -> Vec<String> {
    value
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|kind| !kind.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The `limit` query value as PHP's `is_numeric` and `(int)` read it.
fn numeric(value: &str) -> Option<i64> {
    let value = value.trim();
    value.parse::<i64>().ok().or_else(|| {
        value
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite())
            .map(|n| n as i64)
    })
}

/// Every entry's metadata, newest first, filtered by `component`, `type`
/// and `exclude`, then `offset` and `limit`.
async fn list(repository: &Arc<EntriesRepository>, request: &Request) -> HttpResponse {
    let query = request.query_params();
    let component = query.get("component").filter(|c| !c.is_empty()).cloned();
    let include = type_list(query.get("type"));
    let exclude = type_list(query.get("exclude"));
    let offset = query
        .get("offset")
        .and_then(|offset| numeric(offset))
        .unwrap_or(0)
        .max(0) as usize;
    let limit = query
        .get("limit")
        .and_then(|limit| numeric(limit))
        .map(|limit| limit.max(1) as usize);
    let repository = Arc::clone(repository);
    let all = tokio::task::spawn_blocking(move || repository.all())
        .await
        .unwrap_or_default();
    let kind = |meta: &Value| {
        meta.get("requestType")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let entries: Vec<Value> = all
        .into_iter()
        .filter(|meta| {
            component.as_deref().is_none_or(|component| {
                meta.get("component").and_then(Value::as_str) == Some(component)
            })
        })
        .filter(|meta| include.is_empty() || include.contains(&kind(meta)))
        .filter(|meta| !exclude.contains(&kind(meta)))
        .skip(offset)
        .take(limit.unwrap_or(usize::MAX))
        .collect();
    HttpResponse::json(Value::Array(entries))
}

/// The entry stored under `id`, or `404 {"message": "Not found."}`.
async fn show(repository: &Arc<EntriesRepository>, id: String) -> HttpResponse {
    if !is_ulid(&id) {
        return message(404, "Not found.");
    }
    let repository = Arc::clone(repository);
    let found = tokio::task::spawn_blocking(move || repository.get(&id))
        .await
        .ok()
        .flatten();
    match found {
        Some(entry) => HttpResponse::json(entry),
        None => message(404, "Not found."),
    }
}
