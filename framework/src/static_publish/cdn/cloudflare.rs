//! Cloudflare purges public URLs while a Worker selects R2 objects from the durable manifest.

use std::{collections::BTreeMap, time::Duration};

use async_trait::async_trait;
use http::header::{AUTHORIZATION, HeaderValue};
use serde::Deserialize;
use serde_json::json;

use super::{CdnCapabilities, CdnDriver, Purge};
use crate::{FrameworkError, filesystem::S3Config};

/// A zone's purge API, paired with the manifest-reading R2 Worker on `host`.
///
/// The application supplies a zone token with Cache Purge permission. No credentials
/// are read from the environment, and authorization is redacted from diagnostics.
/// The [API](https://developers.cloudflare.com/api/resources/cache/methods/purge/)
/// accepts `files`, `prefixes` or `tags` at `POST /zones/{zone_id}/purge_cache`.
/// [Single-file purges](https://developers.cloudflare.com/cache/how-to/purge-cache/purge-by-single-file/)
/// use full URLs. This driver batches 30 URLs conservatively; the current
/// [limits](https://developers.cloudflare.com/cache/how-to/purge-cache/#availability-and-limits)
/// allow 100 on Free, Pro and Business, and 500 on Enterprise. All plans support tags and prefixes.
pub struct CloudflareDriver {
    zone_id: String,
    host: String,
    authorization: HeaderValue,
    endpoint: url::Url,
    client: reqwest::Client,
    enterprise_purge: bool,
    manifest_ttl: Duration,
}

#[derive(Deserialize)]
struct Envelope {
    success: bool,
    #[serde(default)]
    errors: Vec<ApiError>,
}

#[derive(Deserialize)]
struct ApiError {
    code: i64,
    message: String,
}

impl CloudflareDriver {
    /// Validates the host and disk root so purge URLs and Worker keys share one namespace.
    pub fn new(
        zone_id: &str,
        host: &str,
        api_token: &str,
        config: &S3Config,
    ) -> Result<Self, FrameworkError> {
        if zone_id.is_empty()
            || zone_id.len() > 32
            || !zone_id.bytes().all(|byte| byte.is_ascii_alphanumeric())
        {
            return Err(failure(
                "zone id must contain at most 32 ASCII letters and digits",
            ));
        }
        if host.len() > 253
            || host.split('.').any(|label| {
                label.is_empty()
                    || label.len() > 63
                    || label.starts_with('-')
                    || label.ends_with('-')
                    || !label
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            })
        {
            return Err(failure(
                "host must be a hostname without a scheme, port or path",
            ));
        }
        let root = config.root.as_deref().unwrap_or("").trim_matches('/');
        if !root.is_empty()
            && (root
                .split('/')
                .any(|part| part.is_empty() || matches!(part, "." | ".."))
                || !root
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-_/./".contains(&byte)))
        {
            return Err(failure("S3 root must contain unambiguous path segments"));
        }
        if api_token.is_empty() || !api_token.bytes().all(|byte| byte.is_ascii_graphic()) {
            return Err(failure(
                "API token must be nonempty and contain no whitespace or control characters",
            ));
        }
        let mut authorization = HeaderValue::from_str(&format!("Bearer {api_token}"))
            .map_err(|_| failure("invalid API token header"))?;
        authorization.set_sensitive(true);
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| failure("build HTTP client"))?;
        Ok(Self {
            zone_id: zone_id.into(),
            host: host.to_ascii_lowercase(),
            authorization,
            endpoint: url::Url::parse("https://api.cloudflare.com/client/v4/")
                .map_err(|_| failure("parse API endpoint"))?,
            client,
            enterprise_purge: false,
            manifest_ttl: Duration::from_secs(5),
        })
    }

    /// Keeps the API path while redirecting tests to an HTTP loopback origin only.
    pub fn with_mock_endpoint(mut self, endpoint: &str) -> Result<Self, FrameworkError> {
        let mut endpoint =
            url::Url::parse(endpoint).map_err(|_| failure("invalid mock endpoint"))?;
        let loopback = endpoint
            .host_str()
            .and_then(|host| {
                host.trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .ok()
            })
            .is_some_and(|address| address.is_loopback());
        if endpoint.scheme() != "http"
            || !loopback
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.path() != "/"
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
        {
            return Err(failure(
                "a mock endpoint must be an HTTP loopback origin without credentials, path, query or fragment",
            ));
        }
        endpoint.set_path("/client/v4/");
        self.endpoint = endpoint;
        Ok(self)
    }

    /// Opts into prefix and tag purges so applications choose their wider invalidation scope.
    /// The name retains the Enterprise convention; Cloudflare now offers both on every plan.
    pub fn with_enterprise_purge(mut self) -> Result<Self, FrameworkError> {
        self.enterprise_purge = true;
        Ok(self)
    }

    /// Matches the Worker's `MANIFEST_TTL_SECONDS` so purges outlast cached manifest selections.
    /// Zero disables the wait for tests or Workers that read the manifest on every request.
    pub fn with_manifest_ttl(mut self, ttl: Duration) -> Result<Self, FrameworkError> {
        if tokio::time::Instant::now().checked_add(ttl).is_none() {
            return Err(failure("manifest TTL exceeds the clock range"));
        }
        self.manifest_ttl = ttl;
        Ok(self)
    }

    fn purge_url(&self, path: &str) -> Result<String, FrameworkError> {
        if !path.starts_with('/')
            || path.starts_with("//")
            || path.chars().any(char::is_control)
            || path.contains(['\\', '#', '*'])
        {
            return Err(failure(
                "purge paths must be local absolute paths without controls, backslashes, fragments or wildcards",
            ));
        }
        Ok(format!("https://{}{path}", self.host))
    }

    async fn purge_request(&self, body: serde_json::Value) -> Result<(), FrameworkError> {
        let url = self
            .endpoint
            .join(&format!("zones/{}/purge_cache", self.zone_id))
            .map_err(|_| failure("purge cache: build API URL"))?;
        let response = self
            .client
            .post(url)
            .header(AUTHORIZATION, self.authorization.clone())
            .json(&body)
            .send()
            .await
            .map_err(|_| failure("purge cache: send API request failed"))?;
        let status = response.status();
        let envelope = response.json::<Envelope>().await.map_err(|_| {
            failure(format!(
                "purge cache: HTTP {status}; unreadable Cloudflare envelope"
            ))
        })?;
        if !status.is_success() || !envelope.success {
            let detail = envelope
                .errors
                .first()
                .map(|error| {
                    // A provider may echo authorization in its message. Never copy the token into an error.
                    let token = self
                        .authorization
                        .to_str()
                        .ok()
                        .and_then(|header| header.strip_prefix("Bearer "));
                    let message = match token {
                        Some(token) => error.message.replace(token, "[redacted]"),
                        None => "unreadable provider error".into(),
                    };
                    format!("code {}: {message}", error.code)
                })
                .unwrap_or_else(|| "no Cloudflare error details".into());
            return Err(failure(format!("purge cache: HTTP {status}; {detail}")));
        }
        Ok(())
    }
}

#[async_trait]
impl CdnDriver for CloudflareDriver {
    fn capabilities(&self) -> CdnCapabilities {
        CdnCapabilities {
            urls: true,
            prefix: self.enterprise_purge,
            tags: self.enterprise_purge,
        }
    }

    fn validate_headers(&self, headers: &BTreeMap<String, String>) -> Result<(), FrameworkError> {
        for (name, value) in headers {
            let lower = name.to_ascii_lowercase();
            if [
                "connection",
                "expect",
                "keep-alive",
                "proxy-authenticate",
                "proxy-authorization",
                "proxy-connection",
                "trailer",
                "upgrade",
                "warning",
                "via",
                "host",
                "te",
                "transfer-encoding",
                "x-cache",
                "x-forwarded-proto",
                "x-real-ip",
                "cf-ray",
                "client-cert",
                "client-cert-chain",
                "set-cookie",
            ]
            .contains(&lower.as_str())
                || ["cf-", "x-accel-"]
                    .iter()
                    .any(|prefix| lower.starts_with(prefix))
            {
                return Err(failure(format!(
                    "header {lower} cannot be carried by the Worker"
                )));
            }
            http::HeaderName::from_bytes(lower.as_bytes())
                .map_err(|_| failure(format!("invalid public header {lower}")))?;
            HeaderValue::from_str(value)
                .map_err(|_| failure(format!("invalid public header value for {lower}")))?;
        }
        Ok(())
    }

    /// Returns unknown so the publisher realigns with its durable manifest; the Worker selects `live`.
    async fn selected_version(&self) -> Result<Option<String>, FrameworkError> {
        Ok(None)
    }

    /// Changes no provider state because the manifest is the switch. Waits out cached selections.
    async fn switch_version(&self, _version: &str) -> Result<(), FrameworkError> {
        if !self.manifest_ttl.is_zero() {
            tokio::time::sleep(self.manifest_ttl).await;
        }
        Ok(())
    }

    /// Has nothing to restore for either an initial or later publication: only the manifest selects R2.
    async fn rollback_version(&self, _previous: Option<&str>) -> Result<(), FrameworkError> {
        Ok(())
    }

    async fn purge(&self, request: Purge) -> Result<(), FrameworkError> {
        let bodies = match request {
            Purge::Urls(paths) => {
                let urls = paths
                    .iter()
                    .map(|path| self.purge_url(path))
                    .collect::<Result<Vec<_>, _>>()?;
                urls.chunks(30)
                    .map(|batch| json!({"files": batch}))
                    .collect::<Vec<_>>()
            }
            Purge::Prefix(prefix) => {
                if !self.enterprise_purge {
                    return Err(failure(
                        "prefix purge requires with_enterprise_purge(); available on all current Cloudflare plans",
                    ));
                }
                self.purge_url(&prefix)?;
                if prefix.contains('?') {
                    return Err(failure("prefix purge does not accept a query string"));
                }
                vec![json!({"prefixes": [format!("{}{prefix}", self.host)]})]
            }
            Purge::Tags(tags) => {
                if !self.enterprise_purge {
                    return Err(failure(
                        "tag purge requires with_enterprise_purge(); available on all current Cloudflare plans",
                    ));
                }
                if tags
                    .iter()
                    .any(|tag| tag.is_empty() || tag.chars().any(char::is_control))
                {
                    return Err(failure(
                        "purge tags must be nonempty without control characters",
                    ));
                }
                tags.chunks(100)
                    .map(|batch| json!({"tags": batch}))
                    .collect()
            }
        };
        if bodies.is_empty() {
            return Ok(());
        }
        // Publisher currently calls switch_version before committing the manifest.
        // This post-commit wait prevents a purge from racing an isolate's old selection.
        if !self.manifest_ttl.is_zero() {
            tokio::time::sleep(self.manifest_ttl).await;
        }
        for body in bodies {
            self.purge_request(body).await?;
        }
        Ok(())
    }
}

fn failure(message: impl std::fmt::Display) -> FrameworkError {
    super::super::error(format!("Cloudflare: {message}"))
}
