//! Exact reviewed Live browser artifacts served through Suprnova, plus the
//! typed bootstrap markup a document emits to load them.
//!
//! Artifacts are addressed by the manifest-derived asset identity, so every
//! URL is immutable and safe to cache for a year. The only executable code a
//! document loads is external: the reviewed artifacts and three deterministic
//! boot scripts, all with integrity metadata, so a strict `script-src 'self'`
//! policy needs no inline exception.

use std::collections::BTreeSet;
use std::fmt;
use std::sync::OnceLock;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use bytes::Bytes;
use hyper::Method;
use sha2::{Digest as _, Sha256};
use suprnova_live::artifacts::{ARTIFACT_CACHE_CONTROL, ARTIFACT_CONTENT_TYPE, MANIFEST_FILE};
pub use suprnova_live::artifacts::{
    ArtifactError, ArtifactErrorKind, ArtifactRole, PreloadRelation, RuntimeArtifact, ScriptKind,
};
use suprnova_live::view::{TrustedHtml, TrustedMarkupReason};

use crate::{HttpResponse, Request, Response};

use super::LiveConfig;
use super::routes::LIVE_UPDATE_PATH;

/// Reserved path prefix under which every Live artifact is served.
pub const LIVE_ASSET_PATH_PREFIX: &str = "/__live/v1/assets";
pub(crate) const LIVE_ASSET_ROUTE: &str = "/__live/v1/assets/{identity}/{file}";
pub(crate) const LIVE_ASSET_MISS_ROUTE: &str = "/__live/v1/assets/{file}";

const MANIFEST_CONTENT_TYPE: &str = "application/json; charset=utf-8";
const MANIFEST_CACHE_CONTROL: &str = "public, max-age=0, must-revalidate";
const CLOSED_CACHE_CONTROL: &str = "no-store";
const ESM_BOOT_FILE: &str = "suprnova-live.boot.esm.js";
const ESM_ASYNC_BOOT_FILE: &str = "suprnova-live.boot.async.esm.js";
const CLASSIC_BOOT_FILE: &str = "suprnova-live.boot.classic.js";
const ESM_BOOT_SOURCE: &str = "import { boot } from \"./suprnova-live.esm.js\";\nboot();\n";
/// The asynchronous role needs the runtime's default browser host configured
/// before boot; the ESM boot for documents with that role imports it from the
/// reviewed asynchronous artifact.
const ESM_ASYNC_BOOT_SOURCE: &str = "import { boot } from \"./suprnova-live.esm.js\";\nimport { browserAsyncOptions, configureAsync } from \"./suprnova-live.async.esm.js\";\nconfigureAsync(browserAsyncOptions());\nboot();\n";
/// The classic asynchronous artifact publishes its default host on a global;
/// the classic boot configures it when present and boots either way.
const CLASSIC_BOOT_SOURCE: &str = "var suprnovaLiveAsync = window.SuprnovaLiveAsync;\nif (suprnovaLiveAsync && typeof suprnovaLiveAsync.browserOptions === \"function\") {\n  window[Symbol.for(\"suprnova.live.features.v1\")].configureAsync(suprnovaLiveAsync.browserOptions());\n}\nwindow.SuprnovaLive.boot();\n";
const REQUEST_TIMEOUT_MS: u32 = 5_000;
const MAX_QUEUED_PER_ISLAND: u8 = 8;
const MAX_PARALLEL_PER_ISLAND: u8 = 1;
/// The browser runtime accepts only this response budget range in its
/// configuration. `LiveConfig` may set a larger or smaller server-side
/// response limit; the configuration element reports the configured value
/// bounded to this range, which is the budget the runtime actually applies.
const MIN_RUNTIME_RESPONSE_BYTES: usize = 1_024;
const MAX_RUNTIME_RESPONSE_BYTES: usize = 4_194_304;
const MAX_NONCE_BYTES: usize = 256;
const CONFIG_ELEMENT_ID: &str = "suprnova-live-config";

/// Which script delivery form a document loads the runtime with.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum LiveBootstrapStrategy {
    /// ES modules with `modulepreload`.
    Esm,
    /// Classic deferred scripts exposing `window.SuprnovaLive`.
    Classic,
}

impl LiveBootstrapStrategy {
    const fn core(self) -> ArtifactRole {
        match self {
            Self::Esm => ArtifactRole::CoreEsm,
            Self::Classic => ArtifactRole::CoreClassic,
        }
    }

    const fn stimulus(self) -> ArtifactRole {
        match self {
            Self::Esm => ArtifactRole::StimulusEsm,
            Self::Classic => ArtifactRole::StimulusClassic,
        }
    }

    const fn uploads(self) -> ArtifactRole {
        match self {
            Self::Esm => ArtifactRole::UploadsEsm,
            Self::Classic => ArtifactRole::UploadsClassic,
        }
    }

    const fn async_updates(self) -> ArtifactRole {
        match self {
            Self::Esm => ArtifactRole::AsyncEsm,
            Self::Classic => ArtifactRole::AsyncClassic,
        }
    }
}

/// Typed choices for one document's bootstrap markup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveBootstrapOptions {
    strategy: LiveBootstrapStrategy,
    stimulus: bool,
    nonce: Option<String>,
}

impl LiveBootstrapOptions {
    /// Loads the runtime as ES modules.
    #[must_use]
    pub const fn esm() -> Self {
        Self {
            strategy: LiveBootstrapStrategy::Esm,
            stimulus: false,
            nonce: None,
        }
    }

    /// Loads the runtime as classic deferred scripts.
    #[must_use]
    pub const fn classic() -> Self {
        Self {
            strategy: LiveBootstrapStrategy::Classic,
            stimulus: false,
            nonce: None,
        }
    }

    /// Also loads the optional Stimulus bridge; the application supplies Stimulus itself.
    #[must_use]
    pub const fn with_stimulus(mut self) -> Self {
        self.stimulus = true;
        self
    }

    /// Stamps every emitted script element with a Content Security Policy nonce.
    #[must_use]
    pub fn with_nonce(mut self, nonce: impl Into<String>) -> Self {
        self.nonce = Some(nonce.into());
        self
    }

    /// Returns the delivery form.
    #[must_use]
    pub const fn strategy(&self) -> LiveBootstrapStrategy {
        self.strategy
    }
}

/// Deterministic external script that starts the runtime after it loads.
#[derive(Clone, Debug)]
pub struct BootScript {
    strategy: LiveBootstrapStrategy,
    file: &'static str,
    bytes: &'static [u8],
    sha256_hex: String,
    sri: String,
}

impl BootScript {
    fn new(strategy: LiveBootstrapStrategy, file: &'static str, source: &'static str) -> Self {
        let digest = Sha256::digest(source.as_bytes());
        Self {
            strategy,
            file,
            bytes: source.as_bytes(),
            sha256_hex: hex(&digest),
            sri: format!("sha256-{}", BASE64.encode(digest)),
        }
    }

    /// Returns the delivery form this script starts.
    #[must_use]
    pub const fn strategy(&self) -> LiveBootstrapStrategy {
        self.strategy
    }

    /// Returns the single-segment file name.
    #[must_use]
    pub const fn file(&self) -> &'static str {
        self.file
    }

    /// Returns the exact bytes.
    #[must_use]
    pub const fn bytes(&self) -> &'static [u8] {
        self.bytes
    }

    /// Returns the lower-case hexadecimal SHA-256 digest of the bytes.
    #[must_use]
    pub fn sha256_hex(&self) -> &str {
        &self.sha256_hex
    }

    /// Returns the Subresource Integrity value of the bytes.
    #[must_use]
    pub fn sri(&self) -> &str {
        &self.sri
    }
}

/// Everything Suprnova serves under the Live asset namespace.
pub struct LiveAssetCatalog {
    manifest: &'static suprnova_live::artifacts::RuntimeArtifactManifest,
    boot: [BootScript; 3],
}

impl LiveAssetCatalog {
    /// Returns the manifest-derived identity every artifact URL is scoped by.
    #[must_use]
    pub fn identity(&self) -> &str {
        self.manifest.asset_identity()
    }

    /// Returns the exact manifest bytes.
    #[must_use]
    pub const fn manifest_bytes(&self) -> &'static [u8] {
        self.manifest.manifest_bytes()
    }

    /// Returns every reviewed artifact in role order.
    #[must_use]
    pub fn artifacts(&self) -> &[RuntimeArtifact] {
        self.manifest.artifacts()
    }

    /// Returns the reviewed artifact serving `role`.
    #[must_use]
    pub fn artifact(&self, role: ArtifactRole) -> &RuntimeArtifact {
        self.manifest.artifact(role)
    }

    /// Returns the three framework-owned boot scripts.
    #[must_use]
    pub fn boot_scripts(&self) -> &[BootScript] {
        &self.boot
    }

    /// Returns the boot script for `strategy` on a document without the
    /// asynchronous role.
    #[must_use]
    pub const fn boot_script(&self, strategy: LiveBootstrapStrategy) -> &BootScript {
        self.boot_script_for(strategy, false)
    }

    /// Returns the boot script for `strategy`; an ESM document carrying the
    /// asynchronous role boots through the variant that configures the
    /// runtime's default browser host first.
    #[must_use]
    pub const fn boot_script_for(
        &self,
        strategy: LiveBootstrapStrategy,
        asynchronous: bool,
    ) -> &BootScript {
        match (strategy, asynchronous) {
            (LiveBootstrapStrategy::Esm, false) => &self.boot[0],
            (LiveBootstrapStrategy::Esm, true) => &self.boot[2],
            (LiveBootstrapStrategy::Classic, _) => &self.boot[1],
        }
    }

    /// Returns the absolute path one served file is addressed by.
    #[must_use]
    pub fn url(&self, file: &str) -> String {
        format!("{LIVE_ASSET_PATH_PREFIX}/{}/{file}", self.identity())
    }

    fn lookup(&self, file: &str) -> Option<ServedAsset> {
        if file == MANIFEST_FILE {
            return Some(ServedAsset {
                bytes: self.manifest.manifest_bytes(),
                content_type: MANIFEST_CONTENT_TYPE,
                cache_control: MANIFEST_CACHE_CONTROL,
                etag: format!(
                    "\"{}\"",
                    hex(&Sha256::digest(self.manifest.manifest_bytes()))
                ),
            });
        }
        if let Some(artifact) = self.manifest.artifact_by_file(file) {
            return Some(ServedAsset {
                bytes: artifact.bytes(),
                content_type: ARTIFACT_CONTENT_TYPE,
                cache_control: ARTIFACT_CACHE_CONTROL,
                etag: format!("\"{}\"", artifact.sha256_hex()),
            });
        }
        self.boot
            .iter()
            .find(|script| script.file == file)
            .map(|script| ServedAsset {
                bytes: script.bytes,
                content_type: ARTIFACT_CONTENT_TYPE,
                cache_control: ARTIFACT_CACHE_CONTROL,
                etag: format!("\"{}\"", script.sha256_hex),
            })
    }
}

impl fmt::Debug for LiveAssetCatalog {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LiveAssetCatalog")
            .field("identity", &self.identity())
            .finish_non_exhaustive()
    }
}

struct ServedAsset {
    bytes: &'static [u8],
    content_type: &'static str,
    cache_control: &'static str,
    etag: String,
}

/// Returns the validated catalog, or the closed reason the embedded artifacts cannot be served.
pub fn live_asset_catalog() -> Result<&'static LiveAssetCatalog, ArtifactError> {
    static CATALOG: OnceLock<Result<LiveAssetCatalog, ArtifactError>> = OnceLock::new();
    CATALOG
        .get_or_init(|| {
            suprnova_live::artifacts::runtime_artifacts().map(|manifest| LiveAssetCatalog {
                manifest,
                boot: [
                    BootScript::new(LiveBootstrapStrategy::Esm, ESM_BOOT_FILE, ESM_BOOT_SOURCE),
                    BootScript::new(
                        LiveBootstrapStrategy::Classic,
                        CLASSIC_BOOT_FILE,
                        CLASSIC_BOOT_SOURCE,
                    ),
                    BootScript::new(
                        LiveBootstrapStrategy::Esm,
                        ESM_ASYNC_BOOT_FILE,
                        ESM_ASYNC_BOOT_SOURCE,
                    ),
                ],
            })
        })
        .as_ref()
        .map_err(|error| *error)
}

/// Serves one immutable artifact, boot script, or the manifest by exact identity and name.
pub(crate) async fn handle(request: Request) -> Response {
    let is_head = request.method() == Method::HEAD;
    if request.method() != Method::GET && !is_head {
        return Ok(closed_response(405).header("Allow", "GET, HEAD"));
    }
    let Ok(catalog) = live_asset_catalog() else {
        return Ok(closed_response(503));
    };
    if request.query().is_some() {
        return Ok(closed_response(404));
    }
    let identity_matches = request
        .param("identity")
        .is_ok_and(|identity| identity == catalog.identity());
    let Some(asset) = request
        .param("file")
        .ok()
        .filter(|_| identity_matches)
        .and_then(|file| catalog.lookup(file))
    else {
        return Ok(closed_response(404));
    };
    if if_none_match_hits(request.header("if-none-match"), &asset.etag) {
        return Ok(HttpResponse::new()
            .status(304)
            .header("ETag", asset.etag)
            .header("Cache-Control", asset.cache_control)
            .header("X-Content-Type-Options", "nosniff"));
    }
    let body = if is_head {
        Bytes::new()
    } else {
        Bytes::from_static(asset.bytes)
    };
    Ok(HttpResponse::bytes_body(body, asset.content_type)
        .header("Content-Length", asset.bytes.len().to_string())
        .header("Cache-Control", asset.cache_control)
        .header("ETag", asset.etag)
        .header("X-Content-Type-Options", "nosniff"))
}

/// Answers the single-segment asset path with a closed miss.
pub(crate) async fn handle_miss(request: Request) -> Response {
    let is_head = request.method() == Method::HEAD;
    if request.method() != Method::GET && !is_head {
        return Ok(closed_response(405).header("Allow", "GET, HEAD"));
    }
    Ok(closed_response(404))
}

fn if_none_match_hits(header: Option<&str>, etag: &str) -> bool {
    header.is_some_and(|value| {
        value.split(',').any(|candidate| {
            let candidate = candidate.trim();
            candidate == "*" || candidate.strip_prefix("W/").unwrap_or(candidate) == etag
        })
    })
}

fn closed_response(status: u16) -> HttpResponse {
    HttpResponse::new()
        .header("Cache-Control", CLOSED_CACHE_CONTROL)
        .header("X-Content-Type-Options", "nosniff")
        .header("Content-Length", "0")
        .status(status)
}

/// Bootstrap markup for one document: inert configuration plus ordered artifact tags.
pub struct LiveBootstrap {
    html: TrustedHtml,
    roles: Vec<ArtifactRole>,
    strategy: LiveBootstrapStrategy,
}

impl LiveBootstrap {
    /// Returns the markup for `|trusted_html` insertion inside `<head>`.
    #[must_use]
    pub const fn html(&self) -> &TrustedHtml {
        &self.html
    }

    /// Returns the artifact roles the document loads, in load order.
    #[must_use]
    pub fn roles(&self) -> &[ArtifactRole] {
        &self.roles
    }

    /// Returns the delivery form.
    #[must_use]
    pub const fn strategy(&self) -> LiveBootstrapStrategy {
        self.strategy
    }
}

impl fmt::Debug for LiveBootstrap {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LiveBootstrap")
            .field("strategy", &self.strategy)
            .field("roles", &self.roles)
            .finish()
    }
}

/// Optional capabilities one document's islands require.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum RequiredCapability {
    Uploads,
    AsyncUpdates,
}

/// Closed reasons bootstrap markup cannot be produced.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BootstrapFailure {
    AssetsUnavailable,
    InvalidNonce,
    MarkupRejected,
}

pub(crate) fn render_bootstrap(
    options: &LiveBootstrapOptions,
    required: &BTreeSet<RequiredCapability>,
    config: LiveConfig,
    protocol: (u16, u16),
) -> Result<LiveBootstrap, BootstrapFailure> {
    let catalog = live_asset_catalog().map_err(|_| BootstrapFailure::AssetsUnavailable)?;
    let nonce = match options.nonce.as_deref() {
        None => None,
        Some(value) if valid_nonce(value) => Some(value),
        Some(_) => return Err(BootstrapFailure::InvalidNonce),
    };
    let strategy = options.strategy;
    let mut optional = Vec::new();
    if options.stimulus {
        optional.push(strategy.stimulus());
    }
    if required.contains(&RequiredCapability::Uploads) {
        optional.push(strategy.uploads());
    }
    if required.contains(&RequiredCapability::AsyncUpdates) {
        optional.push(strategy.async_updates());
    }
    let core = strategy.core();
    let mut roles = optional.clone();
    roles.push(core);

    let mut html = String::new();
    html.push_str(&config_element(catalog.identity(), config, protocol));
    let core_artifact = catalog.artifact(core);
    let core_url = catalog.url(core_artifact.file());
    match strategy {
        LiveBootstrapStrategy::Esm => {
            html.push_str(&format!(
                "<link rel=\"modulepreload\" href=\"{core_url}\" integrity=\"{}\" crossorigin=\"anonymous\">",
                core_artifact.sri()
            ));
            for role in &optional {
                let artifact = catalog.artifact(*role);
                html.push_str(&module_script(
                    &catalog.url(artifact.file()),
                    artifact.sri(),
                    nonce,
                ));
            }
            let boot = catalog.boot_script_for(
                strategy,
                required.contains(&RequiredCapability::AsyncUpdates),
            );
            html.push_str(&module_script(&catalog.url(boot.file()), boot.sri(), nonce));
        }
        LiveBootstrapStrategy::Classic => {
            html.push_str(&format!(
                "<link rel=\"preload\" as=\"script\" href=\"{core_url}\" integrity=\"{}\" crossorigin=\"anonymous\">",
                core_artifact.sri()
            ));
            for role in &optional {
                let artifact = catalog.artifact(*role);
                html.push_str(&classic_script(
                    &catalog.url(artifact.file()),
                    artifact.sri(),
                    nonce,
                ));
            }
            html.push_str(&classic_script(&core_url, core_artifact.sri(), nonce));
            let boot = catalog.boot_script(strategy);
            html.push_str(&classic_script(
                &catalog.url(boot.file()),
                boot.sri(),
                nonce,
            ));
        }
    }
    let reason = TrustedMarkupReason::new(
        "Live bootstrap markup assembled from validated artifact identities and digests",
    )
    .map_err(|_| BootstrapFailure::MarkupRejected)?;
    let html = TrustedHtml::framework_generated(html, reason)
        .map_err(|_| BootstrapFailure::MarkupRejected)?;
    Ok(LiveBootstrap {
        html,
        roles,
        strategy,
    })
}

fn module_script(url: &str, sri: &str, nonce: Option<&str>) -> String {
    format!(
        "<script type=\"module\" src=\"{url}\" integrity=\"{sri}\" crossorigin=\"anonymous\"{}></script>",
        nonce_attribute(nonce)
    )
}

fn classic_script(url: &str, sri: &str, nonce: Option<&str>) -> String {
    format!(
        "<script defer src=\"{url}\" integrity=\"{sri}\" crossorigin=\"anonymous\"{}></script>",
        nonce_attribute(nonce)
    )
}

fn nonce_attribute(nonce: Option<&str>) -> String {
    nonce.map_or_else(String::new, |nonce| format!(" nonce=\"{nonce}\""))
}

fn valid_nonce(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_NONCE_BYTES
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'=' | b'-' | b'_')
        })
}

fn config_element(identity: &str, config: LiveConfig, protocol: (u16, u16)) -> String {
    let max_response_bytes = config
        .max_response_bytes()
        .clamp(MIN_RUNTIME_RESPONSE_BYTES, MAX_RUNTIME_RESPONSE_BYTES);
    let mut protocol_object = serde_json::Map::new();
    protocol_object.insert("maximum".to_owned(), serde_json::Value::from(protocol.1));
    protocol_object.insert("minimum".to_owned(), serde_json::Value::from(protocol.0));
    let mut object = serde_json::Map::new();
    object.insert(
        "asset_identity".to_owned(),
        serde_json::Value::String(identity.to_owned()),
    );
    object.insert(
        "credentials".to_owned(),
        serde_json::Value::String("same-origin".to_owned()),
    );
    object.insert(
        "endpoint".to_owned(),
        serde_json::Value::String(LIVE_UPDATE_PATH.to_owned()),
    );
    object.insert(
        "max_parallel_per_island".to_owned(),
        serde_json::Value::from(MAX_PARALLEL_PER_ISLAND),
    );
    object.insert(
        "max_queued_per_island".to_owned(),
        serde_json::Value::from(MAX_QUEUED_PER_ISLAND),
    );
    object.insert(
        "max_response_bytes".to_owned(),
        serde_json::Value::from(max_response_bytes),
    );
    object.insert(
        "protocol".to_owned(),
        serde_json::Value::Object(protocol_object),
    );
    object.insert(
        "request_timeout_ms".to_owned(),
        serde_json::Value::from(REQUEST_TIMEOUT_MS),
    );
    object.insert(
        "runtime_contract_version".to_owned(),
        serde_json::Value::from(suprnova_live::artifacts::RUNTIME_CONTRACT_VERSION),
    );
    let json = serde_json::Value::Object(object)
        .to_string()
        .replace('<', "\\u003c");
    format!("<script id=\"{CONFIG_ELEMENT_ID}\" type=\"application/json\">{json}</script>")
}

fn hex(digest: &[u8]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conditional_requests_match_strong_weak_and_wildcard_validators() {
        let etag = "\"abc\"";
        assert!(if_none_match_hits(Some("\"abc\""), etag));
        assert!(if_none_match_hits(Some("W/\"abc\""), etag));
        assert!(if_none_match_hits(Some("\"zzz\", \"abc\""), etag));
        assert!(if_none_match_hits(Some("*"), etag));
        assert!(!if_none_match_hits(Some("\"abd\""), etag));
        assert!(!if_none_match_hits(None, etag));
    }

    #[test]
    fn nonces_are_bounded_base64_tokens() {
        assert!(valid_nonce("r4nd0m+/="));
        assert!(!valid_nonce(""));
        assert!(!valid_nonce("has space"));
        assert!(!valid_nonce("quote\""));
        assert!(!valid_nonce(&"a".repeat(MAX_NONCE_BYTES + 1)));
    }

    #[test]
    fn boot_scripts_are_deterministic_and_contain_no_dynamic_code() {
        let catalog = live_asset_catalog().expect("embedded artifacts validate");
        let esm = catalog.boot_script(LiveBootstrapStrategy::Esm);
        assert_eq!(esm.file(), "suprnova-live.boot.esm.js");
        assert_eq!(
            std::str::from_utf8(esm.bytes()).expect("utf-8"),
            ESM_BOOT_SOURCE
        );
        let classic = catalog.boot_script(LiveBootstrapStrategy::Classic);
        assert_eq!(classic.file(), "suprnova-live.boot.classic.js");
        assert_eq!(
            std::str::from_utf8(classic.bytes()).expect("utf-8"),
            CLASSIC_BOOT_SOURCE
        );
        for script in catalog.boot_scripts() {
            assert!(script.sri().starts_with("sha256-"));
            assert_eq!(script.sha256_hex().len(), 64);
        }
    }

    #[test]
    fn the_reported_response_budget_is_bounded_to_the_runtime_range() {
        let small = LiveConfig::builder()
            .max_response_bytes(16)
            .build()
            .expect("a small server limit is legal");
        assert!(config_element("id", small, (1, 2)).contains("\"max_response_bytes\":1024"));
        let large = LiveConfig::builder()
            .max_request_bytes(16 * 1024 * 1024)
            .max_response_bytes(8 * 1024 * 1024)
            .build()
            .expect("a large server limit is legal");
        assert!(config_element("id", large, (1, 2)).contains("\"max_response_bytes\":4194304"));
        let standard = LiveConfig::standard();
        assert!(
            (MIN_RUNTIME_RESPONSE_BYTES..=MAX_RUNTIME_RESPONSE_BYTES)
                .contains(&standard.max_response_bytes())
        );
        assert!(config_element("id", standard, (1, 2)).contains(&format!(
            "\"max_response_bytes\":{}",
            standard.max_response_bytes()
        )));
    }

    #[test]
    fn the_configuration_element_is_canonical_and_inert() {
        let element = config_element("suprnova-live-0.1.0-abcdef", LiveConfig::standard(), (1, 2));
        assert!(
            element.starts_with("<script id=\"suprnova-live-config\" type=\"application/json\">")
        );
        assert!(element.ends_with("</script>"));
        assert!(
            element.contains("\"asset_identity\":\"suprnova-live-0.1.0-abcdef\",\"credentials\"")
        );
        assert!(element.contains("\"protocol\":{\"maximum\":2,\"minimum\":1}"));
        assert!(!element.contains('<') || element.matches('<').count() == 2);
    }
}
