//! Applications can own HTTP policy while the embedder owns the embedding protocol.

use crate::FrameworkError;
use async_trait::async_trait;
use http::HeaderMap;
use reqwest::{Client, Method, Url};
use std::time::Duration;

/// Routes inference through the application's admission, credentials and deadlines.
/// The transport receives only URLs derived from the validated, pinned endpoint.
/// The application decides credentials and must preserve the destination boundary.
#[async_trait]
pub trait EmbeddingTransport: Send + Sync {
    /// Send one attempt, enforcing the timeout and body cap while receiving the body.
    /// Returning status and headers lets the embedder keep retry and protocol decisions.
    async fn send(&self, request: TransportRequest) -> Result<TransportResponse, FrameworkError>;
}

/// Carries one bounded HTTP attempt so an application can apply its own transport policy.
#[derive(Debug)]
pub struct TransportRequest {
    /// Distinguishes embedding POSTs from model-list GETs without exposing protocol logic.
    pub method: Method,
    /// The validated destination keeps request data from selecting another endpoint.
    pub url: Url,
    /// Includes a sensitive bearer header when configured; the application may add credentials.
    pub headers: HeaderMap,
    /// Serialized JSON preserves the framework's wire format; model-list GETs carry none.
    pub body: Option<Vec<u8>>,
    /// Bounds the attempt through receipt of its complete response body.
    pub timeout: Duration,
    /// Caps received bytes while streaming so a response cannot consume unbounded memory.
    pub max_response_bytes: usize,
}

/// Preserves response metadata so status handling and parsing stay in the embedder.
#[derive(Debug)]
pub struct TransportResponse {
    /// The HTTP status lets the embedder decide whether an attempt may be retried.
    pub status: u16,
    /// Response headers retain retry timing such as Retry-After.
    pub headers: HeaderMap,
    /// Bounded bytes let the embedder validate the size again before parsing JSON.
    pub body: Vec<u8>,
}

pub(super) struct ReqwestTransport {
    client: Client,
}

impl ReqwestTransport {
    pub(super) fn new() -> Result<Self, FrameworkError> {
        Ok(Self {
            client: crate::http_client::client_for(true, Some(Duration::from_secs(10)))?,
        })
    }

    async fn read_body(
        mut response: reqwest::Response,
        max_response_bytes: usize,
        host: &str,
    ) -> Result<Vec<u8>, FrameworkError> {
        let status = response.status().as_u16();
        if response
            .content_length()
            .is_some_and(|len| len > max_response_bytes as u64)
        {
            return Err(FrameworkError::param(format!(
                "SEARCH_EMBED_MAX_RESPONSE_BYTES exceeded by host {host} status {status}"
            )));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|error| {
            FrameworkError::internal(format!(
                "SEARCH_EMBED_TIMEOUT_SECS or response from host {host} status {status}: {}",
                error.without_url()
            ))
        })? {
            if chunk.len() > max_response_bytes.saturating_sub(body.len()) {
                return Err(FrameworkError::param(format!(
                    "SEARCH_EMBED_MAX_RESPONSE_BYTES exceeded at status {status}"
                )));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }
}

#[async_trait]
impl EmbeddingTransport for ReqwestTransport {
    async fn send(&self, request: TransportRequest) -> Result<TransportResponse, FrameworkError> {
        let host = request
            .url
            .host_str()
            .unwrap_or("configured host")
            .to_owned();
        let purpose = if request.method == Method::GET {
            "model list request failed"
        } else {
            "request failed"
        };
        let mut builder = self
            .client
            .request(request.method, request.url)
            .headers(request.headers)
            .timeout(request.timeout);
        if let Some(body) = request.body {
            builder = builder.body(body);
        }
        let response = builder.send().await.map_err(|error| {
            FrameworkError::internal(format!(
                "SEARCH_EMBED_BASE_URL host {host} {purpose}: {}",
                error.without_url()
            ))
        })?;
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let body = Self::read_body(response, request.max_response_bytes, &host).await?;
        Ok(TransportResponse {
            status,
            headers,
            body,
        })
    }
}
