//! A closed HTTP index allowlist exposes declared documents through generic scoped search.

use super::{AttributeValue, Filter, Search, SearchConfig, SearchRegistry, SortDirection};
use crate::{CsrfMiddleware, FrameworkError, HttpResponse, Request, Response, Router};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::sync::Arc;

impl Router {
    /// Install search for explicitly named indexes so private indexes cannot be discovered.
    ///
    /// GET `/search` accepts `q`, `index`, `filter`, comma-separated `facets`,
    /// comma-separated `sort=attribute:asc|desc`, `page` and `per_page`.
    /// Results expose only declared attributes, scores and escaped formatted fields.
    /// The numeric `total` is accompanied by `total_estimated` so semantic windows
    /// cannot be confused with exact lexical counts. Impressions require a server session.
    /// POST `/search/click` accepts JSON `{impression, key, position}` with zero-based
    /// positions and is protected by CSRF. Attach session and optional authentication
    /// middleware to the application, and rate limiting through the application's
    /// existing middleware policy. No rate or authenticated-user policy is forced here.
    pub fn try_search_endpoint(self, indexes: &[&str]) -> Result<Self, FrameworkError> {
        let mut allowed = BTreeSet::new();
        for index in indexes {
            SearchRegistry::by_index(index)?;
            if !allowed.insert((*index).to_owned()) {
                return Err(FrameworkError::param(format!(
                    "search index '{index}' is named more than once"
                )));
            }
        }
        let allowed = Arc::new(allowed);
        let router: Router = self
            .try_get("/search", move |request| {
                let allowed = Arc::clone(&allowed);
                async move { search(request, &allowed).await.map_err(Into::into) }
            })?
            .into();
        Ok(router
            .try_post("/search/click", click)?
            .middleware(CsrfMiddleware::new())
            .into())
    }
}

#[derive(Serialize)]
struct EndpointHit {
    attributes: std::collections::BTreeMap<String, AttributeValue>,
    key: String,
    score: f64,
    normalized_score: f64,
    formatted: std::collections::BTreeMap<String, String>,
}

async fn search(
    request: Request,
    allowed: &BTreeSet<String>,
) -> Result<HttpResponse, FrameworkError> {
    let index = request.query_param("index").unwrap_or_default();
    if !allowed.contains(&index) {
        return Err(FrameworkError::domain("Not found", 404));
    }
    let config = SearchConfig::current()?;
    if config.analytics && crate::session::session().is_none_or(|session| session.id.is_empty()) {
        return Err(FrameworkError::internal(format!(
            "SEARCH_ANALYTICS requires session middleware for impressions in index '{index}'"
        )));
    }
    let descriptor = (SearchRegistry::by_index(&index)?.descriptor)();
    let cap = config
        .hits_cap
        .min(descriptor.index_settings().query.hits_cap);
    let page = positive_number(&request, "page", 1)?;
    let per_page = positive_number(&request, "per_page", 20.min(cap))?;
    if per_page > cap {
        return Err(FrameworkError::validation(
            "per_page",
            format!("per_page must not exceed {cap} for index '{index}'"),
        ));
    }
    let offset = (page - 1).checked_mul(per_page).ok_or_else(|| {
        FrameworkError::validation("page", "page causes a pagination offset overflow")
    })?;
    let q = request.query_param("q").unwrap_or_default();
    if let Some(message) = super::expansion::query_bounds_error(&super::query::QueryText::parse(&q))
    {
        return Err(FrameworkError::validation("q", message));
    }
    let mut builder = Search::for_index(&index)?
        .search(q)
        .highlight(
            descriptor
                .attributes
                .iter()
                .filter(|attr| attr.searchable.is_some())
                .map(|attr| attr.name.clone()),
        )
        .limit(per_page)
        .offset(offset);
    if let Some(filter) = request
        .query_param("filter")
        .filter(|value| !value.trim().is_empty())
    {
        let filter = Filter::parse(&filter, descriptor)
            .map_err(|error| FrameworkError::validation("filter", error.to_string()))?;
        builder = builder.filter(filter);
    }
    if let Some(facets) = request
        .query_param("facets")
        .filter(|value| !value.is_empty())
    {
        let mut attributes = Vec::new();
        for name in facets.split(',').map(str::trim) {
            let attr = descriptor
                .attribute(name)
                .map_err(|error| FrameworkError::validation("facets", error.to_string()))?;
            if !attr.facet {
                return Err(FrameworkError::validation(
                    "facets",
                    format!("facets attribute '{name}' is not a facet of index '{index}'"),
                ));
            }
            attributes.push(name.to_owned());
        }
        builder = builder.facets(attributes);
    }
    if let Some(sort) = request
        .query_param("sort")
        .filter(|value| !value.is_empty())
    {
        for clause in sort.split(',').map(str::trim) {
            let (name, direction) = clause.split_once(':').unwrap_or((clause, "asc"));
            let attr = descriptor
                .attribute(name)
                .map_err(|error| FrameworkError::validation("sort", error.to_string()))?;
            if !attr.sortable {
                return Err(FrameworkError::validation(
                    "sort",
                    format!("sort attribute '{name}' is not sortable in index '{index}'"),
                ));
            }
            let direction = match direction {
                "asc" => SortDirection::Asc,
                "desc" => SortDirection::Desc,
                _ => {
                    return Err(FrameworkError::validation(
                        "sort",
                        format!("sort attribute '{name}' requires asc or desc"),
                    ));
                }
            };
            builder = builder.sort_by(name, direction);
        }
    }
    let result = builder.get().await?;
    let hits: Vec<_> = result
        .hits
        .into_iter()
        .map(|hit| EndpointHit {
            attributes: hit.model.attributes,
            key: hit.key,
            score: hit.engine_score,
            normalized_score: hit.normalized_score,
            formatted: hit.formatted,
        })
        .collect();
    let mut body = serde_json::json!({
        "hits": hits, "facets": result.facets, "total": result.total.value(),
        "page": page, "per_page": per_page, "processing_time_ms": result.processing_time_ms,
        "total_estimated": matches!(result.total, super::Total::Estimated(_)),
    });
    if let Some(impression) = result.impression {
        body["impression"] = serde_json::Value::String(impression);
    }
    Ok(HttpResponse::json(body))
}

fn positive_number(
    request: &Request,
    parameter: &str,
    default: usize,
) -> Result<usize, FrameworkError> {
    match request.query_param(parameter) {
        None => Ok(default),
        Some(value) => value
            .parse::<usize>()
            .ok()
            .filter(|number| *number > 0)
            .ok_or_else(|| {
                FrameworkError::validation(
                    parameter,
                    format!("{parameter} must be a positive integer"),
                )
            }),
    }
}

#[derive(Deserialize)]
struct Click {
    impression: String,
    key: String,
    position: usize,
}

async fn click(request: Request) -> Response {
    let click: Click = request.json().await?;
    Search::click(&click.impression, &click.key, click.position).await?;
    Ok(HttpResponse::json(serde_json::json!({"recorded": true})))
}
