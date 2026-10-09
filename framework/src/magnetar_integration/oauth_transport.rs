//! Reqwest-backed OAuth token, userinfo, and revocation transport.

use std::time::Duration;

use magnetar::oauth::{
    OAuthProtocolError, OAuthResult, ParamPlacement, RevocationRequest, RevocationTransport,
};
use magnetar::plugin::{HttpRequest, HttpResponse, HttpTransport};
use reqwest::header::{HeaderName, HeaderValue};
use reqwest::{Client, Method, RequestBuilder};

use crate::error::FrameworkError;

const DEFAULT_MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// OAuth transport backed by the framework's reqwest stack.
///
/// [`Self::try_default`] disables redirects so provider credentials are never
/// forwarded to a redirected host, sets a bounded request timeout, and supplies
/// a default `User-Agent`. [`Self::new`] accepts a host-configured client when
/// proxy, certificate, or observability policy must be shared with the app.
#[derive(Clone)]
pub struct ReqwestOAuthTransport {
    client: Client,
    max_response_bytes: usize,
}

impl ReqwestOAuthTransport {
    /// Build the secure framework default.
    pub fn try_default() -> Result<Self, FrameworkError> {
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(DEFAULT_TIMEOUT)
            .user_agent(concat!("suprnova/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| FrameworkError::internal("failed to build OAuth HTTP client"))?;
        Ok(Self::new(client))
    }

    /// Wrap a host-configured reqwest client.
    ///
    /// The host is responsible for redirect and timeout policy on this path.
    #[must_use]
    pub fn new(client: Client) -> Self {
        Self {
            client,
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
        }
    }

    /// Set the maximum response body accepted from token and userinfo endpoints.
    pub fn with_max_response_bytes(mut self, bytes: usize) -> Result<Self, FrameworkError> {
        if bytes == 0 {
            return Err(FrameworkError::bad_request(
                "OAuth response limit must be greater than zero",
            ));
        }
        self.max_response_bytes = bytes;
        Ok(self)
    }

    fn method(value: &str) -> magnetar::Result<Method> {
        Method::from_bytes(value.as_bytes()).map_err(|_| magnetar::Error::InvalidInput {
            field: "OAuth HTTP method".to_owned(),
            message: "contains an invalid method token".to_owned(),
        })
    }

    fn request_headers(
        mut request: RequestBuilder,
        headers: Vec<(String, String)>,
    ) -> magnetar::Result<RequestBuilder> {
        for (name, value) in headers {
            let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
                magnetar::Error::InvalidInput {
                    field: "OAuth HTTP header".to_owned(),
                    message: "contains an invalid header name".to_owned(),
                }
            })?;
            let value =
                HeaderValue::from_str(&value).map_err(|_| magnetar::Error::InvalidInput {
                    field: "OAuth HTTP header".to_owned(),
                    message: "contains an invalid header value".to_owned(),
                })?;
            request = request.header(name, value);
        }
        Ok(request)
    }

    async fn send_request(&self, request: HttpRequest) -> magnetar::Result<HttpResponse> {
        let HttpRequest {
            method,
            url,
            headers,
            body,
        } = request;
        let method = Self::method(&method)?;
        let request = self.client.request(method, url).body(body);
        let request = Self::request_headers(request, headers)?;
        let mut response =
            request
                .send()
                .await
                .map_err(|_| magnetar::Error::DependencyUnavailable {
                    dependency: "OAuth HTTP transport".to_owned(),
                    message: "outbound provider request failed".to_owned(),
                })?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.as_str().to_owned(), value.to_owned()))
            })
            .collect();
        let mut body = Vec::new();
        while let Some(chunk) =
            response
                .chunk()
                .await
                .map_err(|_| magnetar::Error::DependencyUnavailable {
                    dependency: "OAuth HTTP transport".to_owned(),
                    message: "provider response body failed while streaming".to_owned(),
                })?
        {
            if body.len().saturating_add(chunk.len()) > self.max_response_bytes {
                return Err(magnetar::Error::DependencyUnavailable {
                    dependency: "OAuth HTTP transport".to_owned(),
                    message: "provider response body exceeded the configured limit".to_owned(),
                });
            }
            body.extend_from_slice(&chunk);
        }
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }

    fn revocation_error(message: &'static str) -> OAuthProtocolError {
        OAuthProtocolError::ProviderConfiguration {
            provider: "oauth",
            message: message.to_owned(),
        }
    }
}

#[async_trait::async_trait]
impl HttpTransport for ReqwestOAuthTransport {
    async fn send(&self, request: HttpRequest) -> magnetar::Result<HttpResponse> {
        self.send_request(request).await
    }
}

#[async_trait::async_trait]
impl RevocationTransport for ReqwestOAuthTransport {
    async fn send(&self, request: RevocationRequest) -> OAuthResult<()> {
        let method = Method::from_bytes(request.method.as_bytes())
            .map_err(|_| Self::revocation_error("invalid revocation HTTP method"))?;
        let mut outgoing = self.client.request(method, request.endpoint);
        for (name, value) in request.headers {
            let name = HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| Self::revocation_error("invalid revocation header name"))?;
            let value = HeaderValue::from_str(&value)
                .map_err(|_| Self::revocation_error("invalid revocation header value"))?;
            outgoing = outgoing.header(name, value);
        }
        outgoing = match request.placement {
            ParamPlacement::Body => outgoing.form(&request.params),
            ParamPlacement::Query => outgoing.query(&request.params),
        };
        let response =
            outgoing
                .send()
                .await
                .map_err(|_| OAuthProtocolError::UpstreamUnavailable {
                    provider: "oauth",
                    message: "revocation request failed".to_owned(),
                    retry_after_seconds: None,
                })?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(OAuthProtocolError::UpstreamUnavailable {
                provider: "oauth",
                message: format!("revocation endpoint returned HTTP {}", response.status()),
                retry_after_seconds: None,
            })
        }
    }
}
