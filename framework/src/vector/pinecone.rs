//! Phase 9A — Pinecone vector driver, over Pinecone's REST API.
//!
//! A thin adapter that satisfies [`VectorDriver`] while preserving the
//! framework's `String` IDs and `serde_json::Value` payload contract.
//!
//! Construct via [`PineconeVectorDriver::from_api_key`] (or via env
//! through [`PineconeVectorDriver::from_env`]). The driver targets one
//! Pinecone account; each `store` name passed via the trait surface
//! maps to a single Pinecone index inside that account. Index hosts are
//! resolved lazily on first use via the control plane's
//! `GET /indexes/{name}`, then cached — or pinned up front with
//! [`PineconeVectorDriver::with_index_host`].
//!
//! # Why REST and not the SDK
//!
//! v1 of this driver used the official `pinecone-sdk` crate, which talks
//! gRPC. That crate's newest release (0.1.2, published 2024-09-06) pins
//! `tonic 0.11 → tokio-rustls 0.25 → rustls 0.22 → rustls-webpki 0.102`,
//! and `rustls-webpki 0.102` carries four RustSec advisories
//! (RUSTSEC-2026-0049, -0098, -0099, -0104) that are all fixed upstream
//! in `>= 0.103.13`. The rest of this workspace already resolves
//! 0.103.13; one abandoned crate held the whole tree back, and there was
//! no version of "wait for upstream" that was going to end.
//!
//! Pinecone exposes every operation this driver needs over plain HTTPS
//! and the framework already depends on `reqwest`, so the REST route
//! costs no new dependencies at all. The five operations behind the
//! trait surface are `GET /indexes/{name}` (control plane) plus
//! `POST /vectors/upsert`, `POST /query`, `POST /vectors/delete` and
//! `POST /describe_index_stats` (data plane, on the index host).
//!
//! # API version
//!
//! Pinecone versions its REST API by date and requires the version to be
//! pinned in an `X-Pinecone-Api-Version` header. This driver pins
//! [`DEFAULT_API_VERSION`] — the version whose request and response
//! shapes it was written and tested against. Newer versions are opt-in
//! via [`PineconeVectorDriver::with_api_version`] rather than automatic,
//! because a silent version float is exactly how a wire-shape change
//! becomes a production outage.
//!
//! # ID mapping — there is none
//!
//! Unlike Qdrant, Pinecone accepts arbitrary `String` ids natively.
//! [`VectorItem::id`] passes through to Pinecone unchanged; similarity
//! hits return that same string in [`VectorMatch::id`]. No reserved
//! payload keys, no derived UUIDs.
//!
//! # Namespaces
//!
//! Pinecone indexes carry namespaces (multi-tenant partitions inside
//! one index). One driver instance binds to one namespace
//! (default: empty, i.e. the unnamed namespace) — set via
//! [`PineconeVectorDriver::with_namespace`]. To target several
//! namespaces of the same index, register one driver per namespace
//! under different store names:
//!
//! ```rust,no_run
//! # use std::sync::Arc;
//! # use suprnova::Vector;
//! # use suprnova::vector::PineconeVectorDriver;
//! # fn ex() -> Result<(), Box<dyn std::error::Error>> {
//! Vector::register("docs-public", Arc::new(
//!     PineconeVectorDriver::from_env()?.with_namespace("public")
//! ));
//! Vector::register("docs-private", Arc::new(
//!     PineconeVectorDriver::from_env()?.with_namespace("private")
//! ));
//! # Ok(()) }
//! ```
//!
//! # Index creation
//!
//! The driver does **not** auto-create indexes. Pinecone index
//! creation requires picking a cloud (AWS/GCP/Azure), region, vector
//! dimension, distance metric, and deletion-protection setting — too
//! many trade-offs to default well. Create the index via the Pinecone
//! console, the Pinecone CLI, or a
//! [`control_plane_post`](PineconeVectorDriver::control_plane_post) call,
//! then register the driver with Suprnova.
//!
//! # Trapdoor
//!
//! When you outgrow the trait surface — filter expressions, sparse
//! vectors, fetch-by-id, index management — drop down to
//! [`PineconeVectorDriver::control_plane_get`],
//! [`PineconeVectorDriver::control_plane_post`] or
//! [`PineconeVectorDriver::data_plane_post`]. They handle auth, the
//! version header, host resolution, timeouts and error mapping, and
//! leave the request and response bodies entirely to you — so any
//! endpoint Pinecone ships is reachable without waiting on this driver.
//!
//! # Batch limits
//!
//! Pinecone documents a maximum of 1000 vectors per upsert request and
//! 1000 ids per delete request (and a 2 MB request-body ceiling). This
//! driver sends what you give it in one request rather than chunking
//! silently: a partial-success write is far harder to reason about than
//! a rejected one. Batch on your side if you exceed those limits.

use super::driver::{VectorDriver, VectorItem, VectorMatch};
use crate::FrameworkError;
use crate::http_client::vendor::{build_client, read_error_body};
use async_trait::async_trait;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use zeroize::Zeroizing;

/// Pinecone's default control-plane base URL.
pub const DEFAULT_CONTROL_PLANE: &str = "https://api.pinecone.io";

/// The Pinecone REST API version this driver pins in the
/// `X-Pinecone-Api-Version` header.
///
/// Override with [`PineconeVectorDriver::with_api_version`] when you have
/// verified a newer version against your own workload. Pinecone's
/// namespace-key convention in `describe_index_stats` is one of the
/// things that has changed between versions, and [`VectorDriver::count`]
/// reads that map — so a version bump is a change worth making
/// deliberately.
pub const DEFAULT_API_VERSION: &str = "2025-04";

/// Percent-encoding set for an index name interpolated into a URL path.
///
/// Index names are operator- or config-supplied, so they are not trusted
/// to be path-safe: `/` and `%` are encoded, which is what stops a store
/// name from escaping `/indexes/{name}` into some other endpoint. The
/// RFC 3986 unreserved characters stay literal so ordinary hyphenated
/// index names appear on the wire exactly as Pinecone knows them.
const INDEX_NAME_ENCODE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// One vector in Pinecone's REST wire shape.
///
/// Produced by [`PineconeVectorDriver::build_vector`]; serializes to
/// exactly the object Pinecone's `POST /vectors/upsert` expects inside
/// its `vectors` array. Exposed so callers mixing
/// [`data_plane_post`](PineconeVectorDriver::data_plane_post) with
/// framework upserts can produce identical payloads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PineconeVector {
    /// Caller-chosen merge key, passed through to Pinecone unchanged.
    pub id: String,
    /// Dense embedding. Length must match the index's dimension.
    pub values: Vec<f32>,
    /// Metadata object, omitted from the wire entirely when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Map<String, serde_json::Value>>,
}

/// One scored hit from Pinecone's `POST /query`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PineconeMatch {
    /// Id of the matched vector.
    pub id: String,
    /// Similarity under the index's configured metric. Higher is closer
    /// for `cosine` and `dotproduct`; for `euclidean` Pinecone still
    /// returns matches best-first, so ordering is what to rely on.
    #[serde(default)]
    pub score: f32,
    /// Metadata, present only because the driver asks for it.
    #[serde(default)]
    pub metadata: Option<serde_json::Map<String, serde_json::Value>>,
}

/// Pinecone-backed [`VectorDriver`].
pub struct PineconeVectorDriver {
    api_key: Zeroizing<String>,
    control_plane: String,
    api_version: String,
    namespace: String,
    http: reqwest::Client,
    hosts: RwLock<HashMap<String, Arc<String>>>,
}

/// Hand-written rather than derived, and deliberately so: a derived
/// `Debug` would print the API key, and this type is exactly the kind of
/// thing that ends up inside a struct somebody else derives `Debug` on.
/// The redaction has to live here, where the key does.
impl std::fmt::Debug for PineconeVectorDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PineconeVectorDriver")
            .field("api_key", &"<redacted>")
            .field("control_plane", &self.control_plane)
            .field("api_version", &self.api_version)
            .field("namespace", &self.namespace)
            .finish_non_exhaustive()
    }
}

impl PineconeVectorDriver {
    /// Construct against Pinecone with an explicit API key.
    ///
    /// Rejects an empty key here rather than letting every later call
    /// fail with a 401 — a missing environment variable should name
    /// itself at boot, not at first search.
    pub fn from_api_key(api_key: impl Into<String>) -> Result<Self, FrameworkError> {
        let api_key = Zeroizing::new(api_key.into());
        if api_key.trim().is_empty() {
            return Err(FrameworkError::param(
                "pinecone api key is empty; set PINECONE_API_KEY or pass a key explicitly",
            ));
        }
        Ok(Self {
            api_key,
            control_plane: DEFAULT_CONTROL_PLANE.to_string(),
            api_version: DEFAULT_API_VERSION.to_string(),
            namespace: String::new(),
            http: build_client(concat!("suprnova-pinecone/", env!("CARGO_PKG_VERSION"))),
            hosts: RwLock::new(HashMap::new()),
        })
    }

    /// Construct from the environment, using the same variable names the
    /// official SDKs use:
    ///
    /// - `PINECONE_API_KEY` (required)
    /// - `PINECONE_CONTROLLER_HOST` (optional) — control-plane base URL,
    ///   defaulting to [`DEFAULT_CONTROL_PLANE`]
    /// - `PINECONE_API_VERSION` (optional) — overrides
    ///   [`DEFAULT_API_VERSION`]
    pub fn from_env() -> Result<Self, FrameworkError> {
        let api_key = std::env::var("PINECONE_API_KEY").map_err(|_| {
            FrameworkError::param("PINECONE_API_KEY is not set; the Pinecone driver needs it")
        })?;
        let mut driver = Self::from_api_key(api_key)?;
        if let Ok(host) = std::env::var("PINECONE_CONTROLLER_HOST")
            && !host.trim().is_empty()
        {
            driver = driver.with_control_plane(host);
        }
        if let Ok(version) = std::env::var("PINECONE_API_VERSION")
            && !version.trim().is_empty()
        {
            driver = driver.with_api_version(version);
        }
        Ok(driver)
    }

    /// Bind this driver to a non-default namespace.
    pub fn with_namespace(mut self, name: impl Into<String>) -> Self {
        self.namespace = name.into();
        self
    }

    /// Point the control plane somewhere other than
    /// [`DEFAULT_CONTROL_PLANE`] — a proxy, a regional endpoint, or a
    /// local emulator. A bare host gains an `https://` scheme; any
    /// trailing slash is trimmed.
    pub fn with_control_plane(mut self, base_url: impl Into<String>) -> Self {
        self.control_plane = normalize_base_url(&base_url.into(), false);
        self
    }

    /// Pin the `X-Pinecone-Api-Version` header value.
    pub fn with_api_version(mut self, version: impl Into<String>) -> Self {
        self.api_version = version.into();
        self
    }

    /// Pin the data-plane host for `store`, skipping the control-plane
    /// lookup that would otherwise resolve it on first use.
    ///
    /// Pinecone's own guidance is to target an index by its host once you
    /// know it; doing so removes a round trip from cold start and one
    /// dependency from the request path. Unlike a host learned from the
    /// control plane — which is always contacted over `https` — a host
    /// pinned here is taken as given, so an emulator on `http://` works.
    pub fn with_index_host(mut self, store: impl Into<String>, host: impl Into<String>) -> Self {
        let base = normalize_base_url(&host.into(), false);
        // `get_mut` rather than `write().await`: the builder owns `self`
        // exclusively, so this needs no lock and no async context — which
        // is what keeps the builder chain usable outside a runtime.
        self.hosts.get_mut().insert(store.into(), Arc::new(base));
        self
    }

    /// The namespace this driver binds writes and queries to. Empty
    /// string means Pinecone's unnamed default namespace.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// The control-plane base URL this driver resolves index hosts against.
    pub fn control_plane(&self) -> &str {
        &self.control_plane
    }

    /// The REST API version this driver pins.
    pub fn api_version(&self) -> &str {
        &self.api_version
    }

    /// Convert a JSON object/null into Pinecone's metadata shape.
    /// Returns a `param` error for any non-object, non-null JSON value,
    /// matching the Qdrant and MariaDB drivers.
    ///
    /// Pinecone itself constrains metadata *values* to strings, numbers,
    /// booleans and lists of strings — it rejects nested objects and
    /// mixed-type lists server-side. The driver does not re-implement
    /// that check: Pinecone's rules are versioned and ours would drift.
    pub fn metadata_from_json(
        value: serde_json::Value,
    ) -> Result<Option<serde_json::Map<String, serde_json::Value>>, FrameworkError> {
        match value {
            serde_json::Value::Null => Ok(None),
            serde_json::Value::Object(map) => Ok(Some(map)),
            other => Err(FrameworkError::param(format!(
                "vector metadata must be a JSON object or null, got: {other}"
            ))),
        }
    }

    /// Convert Pinecone's metadata back into JSON. `None` becomes
    /// [`serde_json::Value::Null`] for symmetry with
    /// [`Self::metadata_from_json`].
    pub fn metadata_to_json(
        metadata: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> serde_json::Value {
        metadata.map_or(serde_json::Value::Null, serde_json::Value::Object)
    }

    /// Convert a [`VectorItem`] into Pinecone's wire shape.
    /// Pure-function helper exposed for power users.
    pub fn build_vector(item: VectorItem) -> Result<PineconeVector, FrameworkError> {
        Ok(PineconeVector {
            id: item.id,
            values: item.embedding,
            metadata: Self::metadata_from_json(item.metadata)?,
        })
    }

    /// Decode a Pinecone hit into a framework-side [`VectorMatch`].
    pub fn decode_match(hit: PineconeMatch) -> VectorMatch {
        VectorMatch {
            id: hit.id,
            score: hit.score,
            metadata: Self::metadata_to_json(hit.metadata),
        }
    }

    /// Resolve (and cache) the data-plane base URL for `store`.
    ///
    /// This used to take the cache's write lock and hold it across *both*
    /// `describe_index` and the SDK's index handshake — two network round
    /// trips — so every other acquisition queued behind it. Because
    /// `tokio::sync::RwLock` is fair, a waiting writer also blocks
    /// subsequent readers: one cold index stalled every warm one, and a
    /// caller that would simply have hit the cache for an unrelated index
    /// paid the full latency of somebody else's cold start.
    ///
    /// The internal `vector::handle_cache::get_or_build` helper builds
    /// outside the lock and takes it only to insert. See that module for
    /// why the trade — a possible duplicate `describe_index` on a genuine
    /// race — is the right one.
    pub async fn index_host(&self, store: &str) -> Result<Arc<String>, FrameworkError> {
        crate::vector::handle_cache::get_or_build(&self.hosts, store, || async {
            let described: IndexDescription = self
                .control_plane_get(&format!(
                    "/indexes/{}",
                    utf8_percent_encode(store, INDEX_NAME_ENCODE)
                ))
                .await?;
            if described.host.trim().is_empty() {
                return Err(FrameworkError::internal(format!(
                    "pinecone describe_index '{store}' returned no host; \
                     the index may still be initializing"
                )));
            }
            // Always https: the host arrives over the network, and a
            // scheme learned from a response is a scheme an attacker
            // who can answer for the control plane gets to choose.
            // Operators who genuinely need cleartext pin it themselves
            // through `with_index_host`.
            Ok(Arc::new(normalize_base_url(&described.host, true)))
        })
        .await
    }

    /// `GET {control plane}{path}`, authenticated and version-pinned,
    /// decoding the JSON response into `R`.
    ///
    /// `path` starts with `/` and is used verbatim — percent-encode any
    /// interpolated segment yourself.
    pub async fn control_plane_get<R: DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<R, FrameworkError> {
        let url = format!("{}{path}", self.control_plane);
        let request = self.authenticated(self.http.get(&url));
        self.send_json(request, "GET", &url).await
    }

    /// `POST {control plane}{path}` with a JSON body, decoding the JSON
    /// response into `R`. This is the escape hatch for index management
    /// (`/indexes`, `/collections`, …).
    pub async fn control_plane_post<B, R>(&self, path: &str, body: &B) -> Result<R, FrameworkError>
    where
        B: Serialize + ?Sized,
        R: DeserializeOwned,
    {
        let url = format!("{}{path}", self.control_plane);
        let request = self.authenticated(self.http.post(&url)).json(body);
        self.send_json(request, "POST", &url).await
    }

    /// `POST {index host}{path}` with a JSON body, resolving `store`'s
    /// host first, and decoding the JSON response into `R`.
    ///
    /// Use this for the data-plane endpoints the trait surface doesn't
    /// cover — `/vectors/fetch`, `/vectors/update`, `/query` with a
    /// metadata filter or a sparse vector, `/vectors/list`.
    pub async fn data_plane_post<B, R>(
        &self,
        store: &str,
        path: &str,
        body: &B,
    ) -> Result<R, FrameworkError>
    where
        B: Serialize + ?Sized,
        R: DeserializeOwned,
    {
        let host = self.index_host(store).await?;
        let url = format!("{host}{path}");
        let request = self.authenticated(self.http.post(&url)).json(body);
        self.send_json(request, "POST", &url).await
    }

    fn authenticated(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        builder
            .header("Api-Key", self.api_key.as_str())
            .header("X-Pinecone-Api-Version", &self.api_version)
            .header("Accept", "application/json")
    }

    /// Send, enforce a 2xx status, and decode the body.
    ///
    /// The error message carries the method, URL, status and a capped
    /// slice of the response body. It never carries the API key: the key
    /// travels in a header, is wrapped in [`Zeroizing`], and is not part
    /// of anything formatted here.
    async fn send_json<R: DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
        method: &str,
        url: &str,
    ) -> Result<R, FrameworkError> {
        let response = request
            .send()
            .await
            .map_err(|e| FrameworkError::internal(format!("pinecone {method} {url}: {e}")))?;

        let status = response.status();
        if !status.is_success() {
            let body = read_error_body(response).await;
            return Err(FrameworkError::internal(format!(
                "pinecone {method} {url} failed with HTTP {}: {body}",
                status.as_u16()
            )));
        }

        response.json::<R>().await.map_err(|e| {
            FrameworkError::internal(format!(
                "pinecone {method} {url} returned undecodable JSON: {e}"
            ))
        })
    }
}

#[async_trait]
impl VectorDriver for PineconeVectorDriver {
    async fn upsert(&self, store: &str, items: Vec<VectorItem>) -> Result<(), FrameworkError> {
        if items.is_empty() {
            return Ok(());
        }
        let dim = items[0].embedding.len();
        if dim == 0 {
            return Err(FrameworkError::param(
                "vector::upsert items have zero-length embedding",
            ));
        }
        let vectors: Vec<PineconeVector> = items
            .into_iter()
            .map(Self::build_vector)
            .collect::<Result<_, _>>()?;

        // Pinecone answers `{"upsertedCount": n}`; the trait promises
        // nothing about a count, so the body is only decoded far enough
        // to prove the response was JSON at all.
        let _: serde_json::Value = self
            .data_plane_post(
                store,
                "/vectors/upsert",
                &UpsertRequest {
                    vectors: &vectors,
                    namespace: &self.namespace,
                },
            )
            .await?;
        Ok(())
    }

    async fn similar(
        &self,
        store: &str,
        query: Vec<f32>,
        k: usize,
    ) -> Result<Vec<VectorMatch>, FrameworkError> {
        if k == 0 {
            return Ok(Vec::new());
        }
        if query.is_empty() {
            return Err(FrameworkError::param("vector::similar query is empty"));
        }
        let q_norm: f32 = query.iter().map(|x| x * x).sum::<f32>().sqrt();
        if q_norm == 0.0 {
            return Err(FrameworkError::param(
                "vector::similar query is zero-vector",
            ));
        }
        let top_k = u32::try_from(k).map_err(|_| {
            FrameworkError::param(format!(
                "vector::similar k={k} exceeds Pinecone's u32 limit"
            ))
        })?;

        let response: QueryResponse = self
            .data_plane_post(
                store,
                "/query",
                &QueryRequest {
                    namespace: &self.namespace,
                    top_k,
                    vector: &query,
                    include_values: false,
                    include_metadata: true,
                },
            )
            .await?;
        Ok(response
            .matches
            .into_iter()
            .map(Self::decode_match)
            .collect())
    }

    async fn delete(&self, store: &str, ids: Vec<String>) -> Result<(), FrameworkError> {
        if ids.is_empty() {
            return Ok(());
        }
        // Pinecone answers `{}` here; decoding into `Value` keeps the
        // status check while tolerating whatever shape it grows.
        let _: serde_json::Value = self
            .data_plane_post(
                store,
                "/vectors/delete",
                &DeleteRequest {
                    ids: &ids,
                    namespace: &self.namespace,
                },
            )
            .await?;
        Ok(())
    }

    async fn count(&self, store: &str) -> Result<usize, FrameworkError> {
        let stats: StatsResponse = self
            .data_plane_post(store, "/describe_index_stats", &StatsRequest {})
            .await?;
        // Count is per-namespace: Pinecone returns a summary keyed by
        // name, and the unnamed default namespace lives under an
        // empty-string key. A missing key means zero vectors — a
        // namespace that has never been written to is simply absent.
        Ok(stats
            .namespaces
            .get(&self.namespace)
            .map(|ns| ns.vector_count as usize)
            .unwrap_or(0))
    }
}

// ----------------------------------------------------------------------
// Wire types. Private because they are this driver's request shapes, not
// a surface to program against — `data_plane_post` takes your own types.
// ----------------------------------------------------------------------

#[derive(Serialize)]
struct UpsertRequest<'a> {
    vectors: &'a [PineconeVector],
    namespace: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct QueryRequest<'a> {
    namespace: &'a str,
    top_k: u32,
    vector: &'a [f32],
    include_values: bool,
    include_metadata: bool,
}

#[derive(Deserialize)]
struct QueryResponse {
    #[serde(default)]
    matches: Vec<PineconeMatch>,
}

#[derive(Serialize)]
struct DeleteRequest<'a> {
    ids: &'a [String],
    namespace: &'a str,
}

#[derive(Serialize)]
struct StatsRequest {}

#[derive(Deserialize)]
struct StatsResponse {
    #[serde(default)]
    namespaces: HashMap<String, NamespaceSummary>,
}

#[derive(Deserialize)]
struct NamespaceSummary {
    #[serde(default, rename = "vectorCount")]
    vector_count: u64,
}

#[derive(Deserialize)]
struct IndexDescription {
    #[serde(default)]
    host: String,
}

// ----------------------------------------------------------------------
// URL normalization
// ----------------------------------------------------------------------

/// Turn a configured or discovered host into a base URL with no trailing
/// slash.
///
/// `force_https` strips any scheme the input carries and imposes `https`.
/// That is right for a host learned from the control plane's response and
/// wrong for one an operator pinned by hand, who may legitimately be
/// pointing at `http://localhost:8080`.
fn normalize_base_url(raw: &str, force_https: bool) -> String {
    let trimmed = raw.trim().trim_end_matches('/');
    let bare = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or(trimmed);

    if force_https {
        return format!("https://{bare}");
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("https://{bare}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_host_gains_https() {
        assert_eq!(
            normalize_base_url("idx-abc.svc.pinecone.io", false),
            "https://idx-abc.svc.pinecone.io"
        );
    }

    #[test]
    fn a_trailing_slash_is_trimmed_so_paths_do_not_double_up() {
        assert_eq!(
            normalize_base_url("https://api.pinecone.io/", false),
            "https://api.pinecone.io"
        );
    }

    /// A pinned host keeps the scheme the operator chose — that is the
    /// whole point of pinning one.
    #[test]
    fn an_operator_pinned_http_host_is_left_alone() {
        assert_eq!(
            normalize_base_url("http://127.0.0.1:9000", false),
            "http://127.0.0.1:9000"
        );
    }

    /// The security property behind `force_https`: a host arriving in a
    /// control-plane response never gets to select cleartext.
    #[test]
    fn a_discovered_host_cannot_downgrade_to_cleartext() {
        assert_eq!(
            normalize_base_url("http://evil.example", true),
            "https://evil.example"
        );
        assert_eq!(
            normalize_base_url("idx.svc.pinecone.io", true),
            "https://idx.svc.pinecone.io"
        );
    }

    #[test]
    fn an_empty_api_key_is_rejected_at_construction() {
        let err = PineconeVectorDriver::from_api_key("   ").unwrap_err();
        assert!(
            format!("{err}").contains("PINECONE_API_KEY"),
            "the error should name the variable to set: {err}"
        );
    }

    /// A store name is interpolated into a URL path, so it must not be
    /// able to leave the `/indexes/` segment.
    #[test]
    fn a_store_name_cannot_escape_its_path_segment() {
        let encoded = utf8_percent_encode("../../secrets", INDEX_NAME_ENCODE).to_string();
        assert!(
            !encoded.contains('/'),
            "slashes must be encoded or a crafted store name reaches another endpoint: {encoded}"
        );
    }

    /// …while an ordinary Pinecone index name survives untouched, so the
    /// encoding does not turn correct names into 404s.
    #[test]
    fn an_ordinary_index_name_is_not_mangled() {
        let encoded = utf8_percent_encode("my-test-index-1", INDEX_NAME_ENCODE).to_string();
        assert_eq!(encoded, "my-test-index-1");
    }

    #[test]
    fn the_query_body_uses_pinecones_camel_case_field_names() {
        let body = serde_json::to_value(QueryRequest {
            namespace: "ns",
            top_k: 7,
            vector: &[1.0, 0.0],
            include_values: false,
            include_metadata: true,
        })
        .expect("serializes");
        assert_eq!(body["topK"], 7);
        assert_eq!(body["includeMetadata"], true);
        assert_eq!(body["includeValues"], false);
        assert_eq!(body["namespace"], "ns");
        assert_eq!(body["vector"][0], 1.0);
    }

    /// `null` metadata must vanish from the wire rather than serialize as
    /// `"metadata": null`, which Pinecone rejects.
    #[test]
    fn null_metadata_is_omitted_from_the_upsert_body() {
        let vector = PineconeVectorDriver::build_vector(VectorItem::new(
            "id",
            vec![1.0],
            serde_json::Value::Null,
        ))
        .expect("builds");
        let body = serde_json::to_value(UpsertRequest {
            vectors: std::slice::from_ref(&vector),
            namespace: "",
        })
        .expect("serializes");
        assert!(
            body["vectors"][0].get("metadata").is_none(),
            "metadata must be absent, not null: {body}"
        );
    }
}
