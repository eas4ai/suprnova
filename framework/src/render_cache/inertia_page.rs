//! Inertia reuses a structured public shell and renders the final response per request.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use bytes::Bytes;
use sha2::{Digest, Sha256};
use suprnova_live::render_cache::RenderCacheErrorKind;
use suprnova_live::render_cache::composite::{
    AssemblyInput, CheckedJson, CompositeEntry, Segment, SegmentGraph, SlotFailurePolicy,
    SlotOutcome, StitchSlot, assemble, surrounding_digest,
};
use suprnova_live::render_cache::entry::{DecodedEntry, EntryHeader, SafeHeaders, Validator};
use suprnova_live::render_cache::hot::{HotRequest, ResponseParts, respond as respond_with_engine};

use crate::http::{Request, Response};
use crate::inertia::cache::InertiaDocumentFacts;
use crate::middleware::Next;

use super::middleware::{FoundEntry, RenderCacheRuntime};
use super::{RenderCache, collector};

const SHELL_MEDIA: &str = "application/vnd.suprnova.inertia-shell+json";

/// Structured entries must never be replayed as an HTTP document.
pub(crate) fn is_inertia(entry: &CompositeEntry) -> bool {
    entry
        .header()
        .headers
        .iter()
        .any(|(name, value)| name == "content-type" && value == SHELL_MEDIA)
}

/// Publication binds the public payload to the typed composite slot.
pub(crate) fn build_entry(
    runtime: &RenderCacheRuntime,
    mut header: EntryHeader,
    body: &[u8],
    facts: &InertiaDocumentFacts,
) -> Result<DecodedEntry, ()> {
    // The capture must still describe the response after route middleware ran.
    if <[u8; 32]>::from(Sha256::digest(body)) != facts.digest
        || header.content_encoding.is_some()
        || facts.shell.len() > runtime.limits.max_body_bytes
        || crate::CspNonce::current().is_some()
        || header.headers.iter().any(|(name, value)| {
            name == "content-security-policy" && value.to_ascii_lowercase().contains("'nonce-")
        })
    {
        return Err(());
    }
    let mut pairs: Vec<(String, String)> = header
        .headers
        .iter()
        .filter(|(name, _)| *name != "content-type")
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect();
    pairs.push(("content-type".to_owned(), SHELL_MEDIA.to_owned()));
    header.headers = SafeHeaders::from_pairs(pairs).map_err(|_| ())?;
    let mut shell = Vec::with_capacity(facts.shell.len() + 3);
    shell.push(b'[');
    shell.extend_from_slice(&facts.shell);
    shell.extend_from_slice(b",]");
    let len = u32::try_from(shell.len() - 1).map_err(|_| ())?;
    let mut graph = SegmentGraph {
        segments: vec![
            Segment::Literal { len },
            Segment::Slot { index: 0 },
            Segment::Literal { len: 1 },
        ],
        slots: vec![StitchSlot {
            route: super::middleware::route_identity(&facts.route).to_base64url(),
            slot: "inertia-props".to_owned(),
            document_key: "inertia-props".to_owned(),
            component: "inertia-props".to_owned(),
            contract_digest: URL_SAFE_NO_PAD.encode(Sha256::digest(&facts.shell)),
            protocol: 1,
            build: runtime.build.as_str().to_owned(),
            parameters: "{}".to_owned(),
            flags: std::collections::BTreeMap::new(),
            on_failure: SlotFailurePolicy::FailDocument,
            surrounding: String::new(),
        }],
        shell_islands: Vec::new(),
        nonce_headers: Vec::new(),
    };
    graph.slots[0].surrounding = surrounding_digest(&graph, &shell, 0).map_err(|_| ())?;
    CompositeEntry::new(header, graph, Bytes::from(shell))
        .map(DecodedEntry::Composite)
        .map_err(|_| ())
}

/// Only registered Inertia routes cross this boundary; Live keeps its own completion.
pub(crate) async fn serve_prepared(request: Request, next: Next) -> Response {
    let Some(route) = request.route_pattern().map(str::to_owned) else {
        return next(request).await;
    };
    let registry = crate::App::inertia_registry();
    let registered = crate::lock::read(&registry.cache_pages, "Inertia cache pages")
        .map(|pages| pages.keys().any(|(pattern, _)| pattern == &route))
        .unwrap_or(false);
    if !registered {
        return next(request).await;
    }
    crate::inertia::cache::with_route(route, serve_registered(request, next)).await
}

async fn serve_registered(mut request: Request, next: Next) -> Response {
    collector::begin_handler();
    if crate::CspNonce::current().is_some()
        || crate::inertia::cache::route_observes_public_session(
            request.route_pattern().unwrap_or(""),
        )?
    {
        return next(request).await;
    }
    let Some(hit) = request.take_prepared_hit() else {
        return next(request).await;
    };
    let Some(_runtime) = RenderCache::runtime() else {
        return next(request).await;
    };
    let published_at = hit.entry.published_at_ms();
    let FoundEntry::Decoded(decoded) = hit.entry else {
        return next(request).await;
    };
    let DecodedEntry::Composite(entry) = decoded.entry else {
        return next(request).await;
    };
    if !is_inertia(&entry) || !crate::search::render_cache::replay_allowed(&entry.header().observed)
    {
        return next(request).await;
    }
    let Ok(shell) = public_shell(&entry) else {
        return next(request).await;
    };
    let header = entry.header().clone();
    let method = request.method().clone();
    let response = crate::inertia::cache::with_shell(shell, entry, next(request)).await;
    response.and_then(|response| {
        if response.status_code() != 200 {
            return Ok(response);
        }
        form_response(
            response,
            &method,
            &header,
            &hit.policy,
            published_at,
            hit.now_ms,
            hit.warning,
        )
        .map_err(Into::into)
    })
}

/// Engine metadata and stored safe headers follow the same rules as Live assembly.
fn form_response(
    mut response: crate::HttpResponse,
    method: &hyper::Method,
    header: &EntryHeader,
    policy: &super::RenderCachePolicy,
    published_at: u64,
    now_ms: u64,
    warning: Option<&'static str>,
) -> Result<crate::HttpResponse, crate::FrameworkError> {
    let mut pairs: Vec<(String, String)> = header
        .headers
        .iter()
        .filter(|(name, _)| *name != "content-type")
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect();
    if let Some(content_type) = response.header_value("Content-Type") {
        pairs.push(("content-type".to_owned(), content_type.to_owned()));
    }
    let headers = SafeHeaders::from_pairs(pairs).map_err(|error| {
        crate::FrameworkError::internal(format!("Inertia cache headers: {error}"))
    })?;
    let Some(body) = response.buffered_bytes() else {
        return Ok(response.replace_header("Cache-Control", "private, no-store"));
    };
    let validator = Validator::strong_for(body);
    let freshness = policy.freshness();
    let formed = respond_with_engine(
        ResponseParts {
            status: header.status,
            class: header.class,
            shared: policy.shared(),
            freshness: &freshness,
            headers: &headers,
            variance: &header.variance,
            validator: &validator,
            body,
            published_at_ms: published_at,
            seed_deadline_ms: header.seed_deadline_ms,
            cache_control_override: Some("private, no-store"),
            content_encoding: None,
        },
        HotRequest {
            method,
            if_none_match: None,
            now_ms,
        },
        warning,
    )
    .map_err(|error| {
        crate::FrameworkError::internal(format!("Inertia assembled response: {error}"))
    })?;
    for (name, value) in formed.headers() {
        let value = std::str::from_utf8(value.as_bytes()).map_err(|error| {
            crate::FrameworkError::internal(format!("Inertia cache header: {error}"))
        })?;
        response = response.replace_header(name.as_str(), value);
    }
    Ok(response)
}

fn public_shell(entry: &CompositeEntry) -> Result<serde_json::Value, ()> {
    let shell = entry.shell();
    if !matches!(entry.graph().segments.as_slice(),
        [Segment::Literal { len }, Segment::Slot { index: 0 }, Segment::Literal { len: 1 }] if *len as usize == shell.len().saturating_sub(1))
        || entry.graph().slots.len() != 1
        || entry.graph().slots[0].component != "inertia-props"
        || !shell.starts_with(b"[")
        || !shell.ends_with(b",]")
    {
        return Err(());
    }
    serde_json::from_slice(&shell[1..shell.len() - 2]).map_err(|_| ())
}

/// The engine checks slot identity, graph integrity and size before values are merged.
pub(crate) fn assemble_page(
    entry: &CompositeEntry,
    schema: &crate::inertia::cache::CachePage,
    mut page: serde_json::Value,
) -> Result<serde_json::Value, ()> {
    let public = public_shell(entry)?;
    let current = page.as_object_mut().ok_or(())?;
    let props = current
        .get_mut("props")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or(())?;
    if public["contract"].as_str() != Some(schema.contract().as_str()) {
        return Err(());
    }
    // A newly selected public prop not retained by this representation stays in the request slot.
    props.retain(|key, _| public["props"].get(key).is_none());
    let slot = entry
        .graph()
        .slots
        .first()
        .ok_or(())?
        .parse()
        .map_err(|_| ())?;
    let json = CheckedJson::new(&page, slot.slot, slot.document_key).map_err(|_| ())?;
    let runtime = RenderCache::runtime().ok_or(())?;
    let assembled = match assemble(
        entry,
        AssemblyInput {
            outcomes: vec![SlotOutcome::Json(json)],
            nonce: None,
        },
        runtime.limits.max_body_bytes,
    ) {
        Ok(assembled) => assembled,
        // The engine has checked the graph and slot identity before this size refusal.
        // Keep the current values without repeating guards, loaders or session effects.
        Err(error) if error.kind() == RenderCacheErrorKind::AssemblyTooLarge => {
            return merge_public_props(&public, page);
        }
        Err(_) => return Err(()),
    };
    let mut parts: Vec<serde_json::Value> =
        serde_json::from_slice(assembled.body()).map_err(|_| ())?;
    if parts.len() != 2 || parts.first() != Some(&public) {
        return Err(());
    }
    merge_public_props(&public, parts.pop().ok_or(())?)
}

fn merge_public_props(
    public: &serde_json::Value,
    mut page: serde_json::Value,
) -> Result<serde_json::Value, ()> {
    let private = page
        .get_mut("props")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or(())?;
    let mut props = public["props"].as_object().ok_or(())?.clone();
    props.append(private);
    *private = props;
    Ok(page)
}
