//! A deployment-owned destination keeps semantic requests and credentials on one host.

use super::{EmbedInput, Embedder, Embedding, ModelProfile, Pooling, profiles, validate_embedding};
use crate::{FrameworkError, config::Config};
use async_trait::async_trait;
use reqwest::{
    Client, Url,
    header::{AUTHORIZATION, HeaderValue},
};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime},
};
use tokenizers::Tokenizer;

/// Resolve deployment settings once, before any request or tenant can supply a destination.
/// This type deliberately has no Debug implementation because it contains a credential.
#[derive(Clone)]
pub struct HttpEmbedderConfig {
    /// The configured base must be HTTPS except for explicit loopback hosts.
    pub base_url: String,
    /// The wire model id remains independent of a shipped profile's canonical id.
    pub model: String,
    /// A bearer credential is kept off diagnostics and sent only to the pinned URL.
    pub api_key: Option<String>,
    /// A requested dimension must match the returned vector and profile capabilities.
    pub dimensions: Option<usize>,
    /// Batches bound each service request's work.
    pub batch: usize,
    /// The deadline includes receiving the response body.
    pub timeout_secs: u64,
    /// The body is capped while streaming, before deserialization.
    pub max_response_bytes: usize,
    /// Only HTTP 429 and 5xx may consume these additional attempts.
    pub retries: u32,
    /// Unknown models may declare their retrieval instructions without application glue.
    pub query_prefix: Option<String>,
    /// Unknown models may declare their document preparation without application glue.
    pub document_template: Option<String>,
    /// The model's own `tokenizer.json` makes the context cut exact; without it the cut is in
    /// bytes, which never exceeds the context but keeps less text.
    pub tokenizer: Option<PathBuf>,
}
impl Default for HttpEmbedderConfig {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            model: String::new(),
            api_key: None,
            dimensions: None,
            batch: 64,
            timeout_secs: 30,
            max_response_bytes: 16 * 1024 * 1024,
            retries: 3,
            query_prefix: None,
            document_template: None,
            tokenizer: None,
        }
    }
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Variables {
    base_url: Option<String>,
    model: Option<String>,
    api_key: Option<String>,
    dimensions: Option<String>,
    batch: Option<String>,
    timeout_secs: Option<String>,
    max_response_bytes: Option<String>,
    retries: Option<String>,
    query_prefix: Option<String>,
    document_template: Option<String>,
    tokenizer: Option<String>,
}
impl HttpEmbedderConfig {
    /// Registered configuration wins over environment settings, matching the framework's boot pattern.
    pub fn from_env() -> Result<Self, FrameworkError> {
        if let Some(config) = Config::get::<Self>() {
            return Ok(config);
        }
        let raw = Config::resolve_prefixed::<Variables>("SEARCH_EMBED_")?;
        let default = Self::default();
        Ok(Self {
            base_url: raw.base_url.ok_or_else(|| {
                FrameworkError::param("SEARCH_EMBED_BASE_URL is required for http")
            })?,
            model: raw
                .model
                .ok_or_else(|| FrameworkError::param("SEARCH_EMBED_MODEL is required for http"))?,
            api_key: raw.api_key,
            dimensions: raw
                .dimensions
                .map(|v| parse(Some(v), 0, "SEARCH_EMBED_DIMENSIONS"))
                .transpose()?,
            batch: parse(raw.batch, default.batch, "SEARCH_EMBED_BATCH")?,
            timeout_secs: parse(
                raw.timeout_secs,
                default.timeout_secs,
                "SEARCH_EMBED_TIMEOUT_SECS",
            )?,
            max_response_bytes: parse(
                raw.max_response_bytes,
                default.max_response_bytes,
                "SEARCH_EMBED_MAX_RESPONSE_BYTES",
            )?,
            retries: parse(raw.retries, default.retries, "SEARCH_EMBED_RETRIES")?,
            query_prefix: raw.query_prefix,
            document_template: raw.document_template,
            tokenizer: raw.tokenizer.map(PathBuf::from),
        })
    }
    fn validate(&self) -> Result<Url, FrameworkError> {
        if self.model.trim().is_empty() {
            return Err(FrameworkError::param(
                "SEARCH_EMBED_MODEL is required for http",
            ));
        }
        for (name, zero) in [
            ("SEARCH_EMBED_BATCH", self.batch == 0),
            ("SEARCH_EMBED_TIMEOUT_SECS", self.timeout_secs == 0),
            (
                "SEARCH_EMBED_MAX_RESPONSE_BYTES",
                self.max_response_bytes == 0,
            ),
            ("SEARCH_EMBED_DIMENSIONS", self.dimensions == Some(0)),
        ] {
            if zero {
                return Err(FrameworkError::param(format!("{name} must be positive")));
            }
        }
        if self.retries > 16 {
            return Err(FrameworkError::param(
                "SEARCH_EMBED_RETRIES must be at most 16",
            ));
        }
        let mut url = Url::parse(&self.base_url).map_err(|_| {
            FrameworkError::param("SEARCH_EMBED_BASE_URL must be an absolute HTTP or HTTPS URL")
        })?;
        let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        if url.host().is_none()
            || !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(FrameworkError::param(
                "SEARCH_EMBED_BASE_URL requires HTTPS unless localhost, 127.0.0.1 or ::1; credentials, query and fragment are forbidden",
            ));
        }
        url.set_path(&format!("{}/embeddings", url.path().trim_end_matches('/')));
        Ok(url)
    }
}
pub(in crate::search) fn parse<T: std::str::FromStr>(
    value: Option<String>,
    default: T,
    name: &str,
) -> Result<T, FrameworkError> {
    value.map_or(Ok(default), |value| {
        value.parse().map_err(|_| FrameworkError::param(name))
    })
}

/// Cloning the provider shares a connection pool and an immutable model contract.
#[derive(Clone)]
pub struct HttpEmbedder {
    client: Client,
    endpoint: Url,
    authorization: Option<HeaderValue>,
    config: HttpEmbedderConfig,
    profile: ModelProfile,
    native_profile: ModelProfile,
    tokenizer: Option<Arc<Tokenizer>>,
    special_tokens: usize,
}
#[derive(Serialize)]
struct WireRequest<'a> {
    model: &'a str,
    input: &'a [String],
    #[serde(skip_serializing_if = "Option::is_none")]
    dimensions: Option<usize>,
    encoding_format: &'static str,
}
#[derive(Deserialize)]
struct WireResponse {
    data: Vec<WireVector>,
}
#[derive(Deserialize)]
struct WireVector {
    index: usize,
    embedding: Vec<f32>,
}
impl HttpEmbedder {
    /// Boot resolves unknown dimensions once; indexing and query paths perform no probing.
    pub async fn from_env() -> Result<Self, FrameworkError> {
        Self::new(HttpEmbedderConfig::from_env()?).await
    }
    /// Shipped preparation conventions win; an unknown model uses configured or probed dimensions.
    pub async fn new(config: HttpEmbedderConfig) -> Result<Self, FrameworkError> {
        Self::with_profile(config, None).await
    }
    /// Explicit profiles permit deployments to declare context and Matryoshka capabilities.
    /// The generic HTTP protocol has no tokenizer metadata, so the deployment names the model's
    /// `tokenizer.json` for an exact context cut.
    pub async fn with_profile(
        config: HttpEmbedderConfig,
        profile: Option<ModelProfile>,
    ) -> Result<Self, FrameworkError> {
        let endpoint = config.validate()?;
        let mut authorization = config
            .api_key
            .as_ref()
            .map(|key| {
                HeaderValue::from_str(&format!("Bearer {key}")).map_err(|_| {
                    FrameworkError::param("SEARCH_EMBED_API_KEY is not a valid bearer header")
                })
            })
            .transpose()?;
        if let Some(value) = &mut authorization {
            value.set_sensitive(true);
        }
        let client = crate::http_client::client_for(true, Some(Duration::from_secs(10)))?;
        let (tokenizer, special_tokens) = match &config.tokenizer {
            Some(path) => {
                let (tokenizer, special_tokens) = load_tokenizer(path)?;
                (Some(Arc::new(tokenizer)), special_tokens)
            }
            None => (None, CONTEXT_RESERVE),
        };
        let known = profile.or_else(|| profiles::profile(&config.model).ok());
        let unknown = known.is_none();
        let profile = known.unwrap_or_else(|| {
            profiles::text_profile(
                &config.model,
                config.dimensions.unwrap_or(1),
                8192,
                Pooling::Mean,
            )
        });
        let mut provider = Self {
            client,
            endpoint,
            authorization,
            native_profile: profile.clone(),
            profile,
            config,
            tokenizer,
            special_tokens,
        };
        if unknown {
            if let Some(prefix) = &provider.config.query_prefix {
                provider.profile.query_prefix = prefix.clone();
            }
            if let Some(template) = &provider.config.document_template {
                provider.profile.document_template = template.clone();
            }
            provider.profile.validate()?;
            if provider.config.dimensions.is_none() {
                let data = provider.request(&["probe".into()]).await?;
                let vector = data.into_iter().next().ok_or_else(|| {
                    FrameworkError::param("SEARCH_EMBED_MODEL dimension probe returned no vector")
                })?;
                provider.profile.dimensions = vector.len();
                validate_embedding(vector, &provider.profile, None)?;
            }
        }
        provider.profile.validate()?;
        if provider.profile.inputs.image
            || provider.profile.inputs.audio
            || provider.profile.inputs.video
        {
            return Err(FrameworkError::param(
                "SEARCH_EMBED_MODEL http text profiles cannot enable image, audio or video input",
            ));
        }
        provider.native_profile = provider.profile.clone();
        provider.profile = provider
            .native_profile
            .with_dimensions(provider.config.dimensions)?;
        if provider.profile.max_tokens <= provider.special_tokens {
            return Err(FrameworkError::param(format!(
                "embedding model {} max_tokens must exceed the {} tokens reserved for special tokens",
                provider.profile.id, provider.special_tokens
            )));
        }
        Ok(provider)
    }
    /// The service rejects or silently drops input past the model's context, so every prepared
    /// input is cut here. With the model's tokenizer the cut falls after the last token that
    /// fits; without it the cut is in UTF-8 bytes, because every token a WordPiece,
    /// SentencePiece or byte-level BPE tokenizer emits covers at least one input byte.
    fn within_context(&self, text: String) -> Result<String, FrameworkError> {
        let budget = self.profile.max_tokens - self.special_tokens;
        let end = match &self.tokenizer {
            Some(tokenizer) => {
                let encoding = tokenizer.encode(text.as_str(), false).map_err(|e| {
                    FrameworkError::param(format!(
                        "SEARCH_EMBED_TOKENIZER could not encode input: {e}"
                    ))
                })?;
                if encoding.len() <= budget {
                    return Ok(text);
                }
                encoding.get_offsets()[budget - 1].1
            }
            None if text.len() <= budget => return Ok(text),
            None => budget,
        };
        let mut end = end.min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let mut text = text;
        text.truncate(end);
        Ok(text)
    }
    fn diagnostic(&self, status: u16, body: &[u8]) -> FrameworkError {
        let text = String::from_utf8_lossy(body);
        let redacted = match self.config.api_key.as_deref().filter(|key| !key.is_empty()) {
            Some(key) => text.replace(key, "[redacted]"),
            None => text.into_owned(),
        };
        let snippet = String::from_utf8_lossy(&redacted.as_bytes()[..redacted.len().min(200)]);
        FrameworkError::internal(format!(
            "embedding host {} status {status}: {snippet}",
            self.endpoint.host_str().unwrap_or("configured host")
        ))
    }
    async fn request(&self, input: &[String]) -> Result<Vec<Vec<f32>>, FrameworkError> {
        let timeout = Duration::from_secs(self.config.timeout_secs);
        for attempt in 0..=self.config.retries {
            let mut request = self
                .client
                .post(self.endpoint.clone())
                .timeout(timeout)
                .json(&WireRequest {
                    model: &self.config.model,
                    input,
                    dimensions: self.config.dimensions,
                    encoding_format: "float",
                });
            if let Some(value) = &self.authorization {
                request = request.header(AUTHORIZATION, value);
            }
            let mut response = request.send().await.map_err(|error| {
                FrameworkError::internal(format!(
                    "SEARCH_EMBED_BASE_URL host {} request failed: {}",
                    self.endpoint.host_str().unwrap_or("configured host"),
                    error.without_url()
                ))
            })?;
            let status = response.status().as_u16();
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(retry_after);
            if response
                .content_length()
                .is_some_and(|len| len > self.config.max_response_bytes as u64)
            {
                return Err(FrameworkError::param(format!(
                    "SEARCH_EMBED_MAX_RESPONSE_BYTES exceeded by host {} status {status}",
                    self.endpoint.host_str().unwrap_or("configured host")
                )));
            }
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|error| {
                FrameworkError::internal(format!(
                    "SEARCH_EMBED_TIMEOUT_SECS or response from host {} status {status}: {}",
                    self.endpoint.host_str().unwrap_or("configured host"),
                    error.without_url()
                ))
            })? {
                if chunk.len() > self.config.max_response_bytes.saturating_sub(body.len()) {
                    return Err(FrameworkError::param(format!(
                        "SEARCH_EMBED_MAX_RESPONSE_BYTES exceeded at status {status}"
                    )));
                }
                body.extend_from_slice(&chunk);
            }
            if !(200..300).contains(&status) {
                if (status == 429 || (500..600).contains(&status)) && attempt < self.config.retries
                {
                    let jitter = Duration::from_millis(u64::from(rand::random::<u8>()));
                    let backoff = Duration::from_millis(100_u64.saturating_mul(1_u64 << attempt))
                        .min(timeout)
                        + jitter;
                    let wait = retry_after.unwrap_or(backoff).max(backoff);
                    // A server cannot turn a bounded operation into an indefinite sleep.
                    if wait > timeout {
                        return Err(self.diagnostic(status, &body));
                    }
                    tokio::time::sleep(wait).await;
                    continue;
                }
                return Err(self.diagnostic(status, &body));
            }
            let mut data = serde_json::from_slice::<WireResponse>(&body)
                .map_err(|_| self.diagnostic(status, &body))?
                .data;
            data.sort_by_key(|vector| vector.index);
            if data.len() != input.len()
                || data
                    .iter()
                    .enumerate()
                    .any(|(index, vector)| vector.index != index)
            {
                return Err(FrameworkError::param(format!(
                    "embedding status {status} expected {} unique contiguous data index values",
                    input.len()
                )));
            }
            return Ok(data.into_iter().map(|vector| vector.embedding).collect());
        }
        Err(FrameworkError::internal("SEARCH_EMBED_RETRIES exhausted"))
    }
    async fn embed(&self, texts: &[String]) -> Result<Vec<Embedding>, FrameworkError> {
        let mut out = Vec::with_capacity(texts.len());
        for batch in texts.chunks(self.config.batch) {
            for vector in self.request(batch).await? {
                if let Some(dimensions) = self.config.dimensions
                    && vector.len() != dimensions
                {
                    return Err(FrameworkError::param(format!(
                        "SEARCH_EMBED_DIMENSIONS expected {dimensions} dimensions from {}",
                        self.profile.id
                    )));
                }
                out.push(validate_embedding(
                    vector,
                    &self.native_profile,
                    self.config.dimensions,
                )?);
            }
        }
        Ok(out)
    }
}
/// Room for the special tokens a model adds around its input when no tokenizer says how many.
const CONTEXT_RESERVE: usize = 4;

/// Truncation and padding in the file would hide the input's real length, so both are cleared.
fn load_tokenizer(path: &std::path::Path) -> Result<(Tokenizer, usize), FrameworkError> {
    let failed = |e: &dyn std::fmt::Display| {
        FrameworkError::param(format!(
            "SEARCH_EMBED_TOKENIZER could not load {}: {e}",
            path.display()
        ))
    };
    let mut tokenizer = Tokenizer::from_file(path).map_err(|e| failed(&e))?;
    tokenizer.with_truncation(None).map_err(|e| failed(&e))?;
    tokenizer.with_padding(None);
    let special_tokens = tokenizer.encode("", true).map_err(|e| failed(&e))?.len();
    Ok((tokenizer, special_tokens))
}
fn retry_after(value: &str) -> Option<Duration> {
    value
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
        .or_else(|| {
            httpdate::parse_http_date(value).ok().map(|time| {
                time.duration_since(SystemTime::now())
                    .unwrap_or(Duration::ZERO)
            })
        })
}
#[async_trait]
impl Embedder for HttpEmbedder {
    fn profile(&self) -> &ModelProfile {
        &self.profile
    }
    async fn embed_documents(
        &self,
        inputs: &[EmbedInput],
    ) -> Result<Vec<Embedding>, FrameworkError> {
        let texts = inputs
            .iter()
            .map(|input| self.within_context(self.profile.document_input(input)?))
            .collect::<Result<Vec<_>, _>>()?;
        self.embed(&texts).await
    }
    async fn embed_query(&self, text: &str) -> Result<Embedding, FrameworkError> {
        self.embed(&[self.within_context(self.profile.query_input(text)?)?])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| {
                FrameworkError::internal("SEARCH_EMBED_MODEL query returned no embedding")
            })
    }
}
