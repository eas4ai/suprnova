//! CloudFront's signed REST API preserves the distribution configuration while changing one origin.

use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use quick_xml::{Reader, events::Event};
use reqsign_aws_v4::{
    Credential, DefaultCredentialProvider, RequestSigner, StaticCredentialProvider,
};
use reqsign_core::{Context, OsEnv, ProvideCredentialChain, Signer};
use reqsign_file_read_tokio::TokioFileRead;
use sha2::{Digest, Sha256};

use super::{CdnCapabilities, CdnDriver, Purge};
use crate::{FrameworkError, filesystem::S3Config};

/// CloudFront viewer-response function that restores public object headers from S3 metadata.
/// Publish this source with runtime `cloudfront-js-2.0` and attach it to public page behaviors.
pub const RESPONSE_HEADERS_FUNCTION: &str = include_str!("cloudfront_headers.js");

const API: &str = "2020-05-31";
const XMLNS: &str = "http://cloudfront.amazonaws.com/doc/2020-05-31/";

/// An AWS CloudFront distribution, using the S3 disk's credentials or the AWS provider chain.
///
/// The configured origin alone changes. `switch_version` waits until CloudFront reports
/// `Deployed`; a failed update, permission refusal or deployment timeout is an error.
pub struct CloudFrontDriver {
    response_headers_function: Option<String>,
    root_prefix: String,
    distribution_id: String,
    origin_id: String,
    endpoint: url::Url,
    client: reqwest::Client,
    signer: Signer<Credential>,
    poll_interval: Duration,
    deployment_timeout: Duration,
    rollback_origin_path: tokio::sync::Mutex<Option<String>>,
}

#[derive(Debug, Clone)]
struct CredentialHttp(reqwest::Client);

impl reqsign_core::HttpSend for CredentialHttp {
    async fn http_send(
        &self,
        request: http::Request<Bytes>,
    ) -> reqsign_core::Result<http::Response<Bytes>> {
        let (parts, body) = request.into_parts();
        let response = self
            .0
            .request(parts.method, parts.uri.to_string())
            .headers(parts.headers)
            .body(body)
            .send()
            .await
            .map_err(|error| {
                reqsign_core::Error::unexpected("send a CloudFront credential request")
                    .with_source(error)
            })?;
        let status = response.status();
        let headers = response.headers().clone();
        let body = response.bytes().await.map_err(|error| {
            reqsign_core::Error::unexpected("read a CloudFront credential response")
                .with_source(error)
        })?;
        let mut response = http::Response::new(body);
        *response.status_mut() = status;
        *response.headers_mut() = headers;
        Ok(response)
    }
}

impl CloudFrontDriver {
    /// Uses CloudFront's global endpoint and the `us-east-1` signing region.
    pub fn new(
        distribution_id: impl Into<String>,
        origin_id: impl Into<String>,
        config: &S3Config,
    ) -> Result<Self, FrameworkError> {
        let distribution_id = distribution_id.into();
        let origin_id = origin_id.into();
        if distribution_id.is_empty() || !distribution_id.bytes().all(|c| c.is_ascii_alphanumeric())
        {
            return Err(failure(
                "distribution id must contain only ASCII letters and digits",
            ));
        }
        if origin_id.is_empty() || origin_id.chars().any(char::is_control) {
            return Err(failure(
                "origin id must be nonempty and contain no control characters",
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
        let root_prefix = if root.is_empty() {
            String::new()
        } else {
            format!("/{root}")
        };
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| failure(format!("build HTTP client: {error}")))?;
        let context = Context::new()
            .with_file_read(TokioFileRead)
            .with_http_send(CredentialHttp(client.clone()))
            .with_env(OsEnv);
        let mut chain =
            ProvideCredentialChain::new().push(DefaultCredentialProvider::builder().build());
        match (&config.access_key_id, &config.secret_access_key) {
            (Some(key), Some(secret)) if !key.is_empty() && !secret.is_empty() => {
                chain = chain.push_front(StaticCredentialProvider::new(key, secret));
            }
            (None, None) => {}
            _ => {
                return Err(failure(
                    "the S3 credentials must contain both an access key and secret key",
                ));
            }
        }
        Ok(Self {
            response_headers_function: None,
            root_prefix,
            distribution_id,
            origin_id,
            endpoint: url::Url::parse("https://cloudfront.amazonaws.com/")
                .map_err(|error| failure(format!("parse AWS endpoint: {error}")))?,
            client,
            signer: Signer::new(
                context,
                chain,
                RequestSigner::new("cloudfront", "us-east-1"),
            ),
            poll_interval: Duration::from_secs(5),
            deployment_timeout: Duration::from_secs(900),
            rollback_origin_path: tokio::sync::Mutex::new(None),
        })
    }

    /// Requires the deployed header bridge on every behavior that uses the page origin.
    /// S3 user metadata alone does not create browser-enforced security headers.
    pub fn with_response_headers_function(mut self, arn: &str) -> Result<Self, FrameworkError> {
        if !arn.starts_with("arn:aws:cloudfront::")
            || !arn.contains(":function/")
            || !arn
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b":/-_".contains(&byte))
        {
            return Err(failure("expected a CloudFront function ARN"));
        }
        self.response_headers_function = Some(arn.to_owned());
        Ok(self)
    }

    /// Overrides the endpoint for a local mock. Only loopback HTTP endpoints are accepted.
    pub fn with_mock_endpoint(mut self, endpoint: &str) -> Result<Self, FrameworkError> {
        let endpoint = url::Url::parse(endpoint)
            .map_err(|error| failure(format!("invalid mock endpoint: {error}")))?;
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
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || endpoint.path() != "/"
        {
            return Err(failure(
                "a mock endpoint must be an HTTP loopback origin without credentials, path, query or fragment",
            ));
        }
        self.endpoint = endpoint;
        Ok(self)
    }

    /// Bounds deployment polling. Both durations must be nonzero, and polling cannot exceed the timeout.
    pub fn with_deployment_wait(
        mut self,
        interval: Duration,
        timeout: Duration,
    ) -> Result<Self, FrameworkError> {
        if interval.is_zero() || timeout.is_zero() || interval > timeout {
            return Err(failure(
                "deployment polling needs a nonzero interval no longer than its timeout",
            ));
        }
        self.poll_interval = interval;
        self.deployment_timeout = timeout;
        Ok(self)
    }

    async fn request(
        &self,
        method: http::Method,
        suffix: &str,
        body: String,
        etag: Option<&str>,
        switching: bool,
    ) -> Result<(http::HeaderMap, String), FrameworkError> {
        let url = self
            .endpoint
            .join(&format!(
                "{API}/distribution/{}{suffix}",
                self.distribution_id
            ))
            .map_err(|error| failure(format!("build API URL: {error}")))?;
        let mut request = http::Request::builder()
            .method(method)
            .uri(url.as_str())
            .header(http::header::CONTENT_TYPE, "application/xml")
            .header(
                "x-amz-content-sha256",
                hex::encode(Sha256::digest(body.as_bytes())),
            );
        if let Some(etag) = etag {
            request = request.header(http::header::IF_MATCH, etag);
        }
        let (mut parts, ()) = request
            .body(())
            .map_err(|error| failure(format!("build request: {error}")))?
            .into_parts();
        self.signer
            .sign(&mut parts, None)
            .await
            .map_err(|error| failure(format!("sign request: {error}")))?;
        let response = self
            .client
            .request(parts.method, parts.uri.to_string())
            .headers(parts.headers)
            .body(body)
            .send()
            .await
            .map_err(|error| failure(format!("send API request: {error}")))?;
        if !response.status().is_success() {
            if switching {
                // A definite refusal did not change this origin. In particular a 412
                // must never turn rollback into an overwrite of an operator's update.
                self.rollback_origin_path.lock().await.take();
            }
            // Provider response bodies can contain account identifiers and credentials.
            return Err(failure(format!(
                "API request failed with HTTP {}",
                response.status()
            )));
        }
        let headers = response.headers().clone();
        let body = response
            .text()
            .await
            .map_err(|error| failure(format!("read API response: {error}")))?;
        Ok((headers, body))
    }

    async fn wait_deployed(&self) -> Result<(), FrameworkError> {
        tokio::time::timeout(self.deployment_timeout, async {
            loop {
                let (_, body) = self
                    .request(http::Method::GET, "", String::new(), None, false)
                    .await?;
                match element_text(&body, &["Distribution", "Status"])?.as_deref() {
                    Some("Deployed") => return Ok(()),
                    Some("InProgress") => tokio::time::sleep(self.poll_interval).await,
                    _ => {
                        return Err(failure(
                            "distribution response has no recognized deployment status",
                        ));
                    }
                }
            }
        })
        .await
        .map_err(|_| failure("timed out waiting for the distribution to become Deployed"))?
    }
}

#[async_trait]
impl CdnDriver for CloudFrontDriver {
    fn capabilities(&self) -> CdnCapabilities {
        CdnCapabilities {
            tags: false,
            urls: true,
            prefix: true,
        }
    }

    fn validate_headers(
        &self,
        headers: &std::collections::BTreeMap<String, String>,
    ) -> Result<(), FrameworkError> {
        for (name, value) in headers {
            let lower = name.to_ascii_lowercase();
            let blocked = [
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
                "cloudfront-viewer-cert-pem",
                "client-cert",
                "client-cert-chain",
                "set-cookie",
            ];
            if blocked.contains(&lower.as_str())
                || ["x-amz-cf-", "x-amzn-", "x-edge-", "x-accel-", "x-amz-meta-"]
                    .iter()
                    .any(|prefix| lower.starts_with(prefix))
            {
                return Err(failure(format!(
                    "header {lower} cannot be carried by the viewer-response function"
                )));
            }
            http::HeaderName::from_bytes(lower.as_bytes())
                .map_err(|_| failure(format!("invalid public header {lower}")))?;
            http::HeaderValue::from_str(value)
                .map_err(|_| failure(format!("invalid public header value for {lower}")))?;
        }
        Ok(())
    }
    async fn switch_version(&self, version: &str) -> Result<(), FrameworkError> {
        if version.is_empty()
            || version.split('/').any(|part| {
                part.is_empty()
                    || matches!(part, "." | "..")
                    || !part
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
            })
        {
            return Err(failure(
                "publication version must contain only safe path segments",
            ));
        }
        let (headers, config) = self
            .request(http::Method::GET, "/config", String::new(), None, false)
            .await?;
        let etag = headers
            .get(http::header::ETAG)
            .and_then(|value| value.to_str().ok())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| failure("GetDistributionConfig returned no ETag"))?;
        let function = self.response_headers_function.as_deref().ok_or_else(|| {
            failure("configure and associate the response headers function before publication")
        })?;
        validate_header_function(&config, &self.origin_id, function)?;
        let (updated, original_path) = replace_origin_path(
            &config,
            &self.origin_id,
            &format!("{}/_pub/{version}", self.root_prefix),
        )?;
        *self.rollback_origin_path.lock().await = Some(original_path);
        self.request(http::Method::PUT, "/config", updated, Some(etag), true)
            .await?;
        self.wait_deployed().await
    }

    async fn rollback_version(&self, previous: Option<&str>) -> Result<(), FrameworkError> {
        let Some(original_path) = self.rollback_origin_path.lock().await.clone() else {
            // No mutation was attempted, or AWS definitely refused it.
            return Ok(());
        };
        let (headers, current) = self
            .request(http::Method::GET, "/config", String::new(), None, false)
            .await?;
        let etag = headers
            .get(http::header::ETAG)
            .and_then(|value| value.to_str().ok())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| failure("rollback GetDistributionConfig returned no ETag"))?;
        let target = previous
            .map(|version| format!("{}/_pub/{version}", self.root_prefix))
            .unwrap_or(original_path);
        // Restore only the field this driver changed, using a fresh config and ETag.
        let (restored, _) = replace_origin_path(&current, &self.origin_id, &target)?;
        self.request(http::Method::PUT, "/config", restored, Some(etag), false)
            .await?;
        self.wait_deployed().await
    }

    async fn purge(&self, request: Purge) -> Result<(), FrameworkError> {
        let paths = match request {
            Purge::Tags(_) => {
                return Err(failure(
                    "CloudFront does not support dependency tag invalidation",
                ));
            }
            Purge::Urls(paths) => paths,
            Purge::Prefix(prefix) => vec![format!("{}*", prefix.trim_end_matches('*'))],
        };
        if paths.is_empty() {
            return Ok(());
        }
        if paths.len() > 3000 {
            return Err(failure(
                "CloudFront accepts at most 3000 invalidation paths per request",
            ));
        }
        let mut xml = format!(
            "<InvalidationBatch xmlns=\"{XMLNS}\"><CallerReference>{}</CallerReference><Paths><Quantity>{}</Quantity><Items>",
            uuid::Uuid::new_v4(),
            paths.len()
        );
        for path in paths {
            if !path.starts_with('/')
                || path.starts_with("//")
                || path.chars().any(char::is_control)
            {
                return Err(failure(
                    "invalidation paths must be local absolute paths without control characters",
                ));
            }
            xml.push_str(&format!(
                "<Path>{}</Path>",
                quick_xml::escape::escape(&path)
            ));
        }
        xml.push_str("</Items></Paths></InvalidationBatch>");
        self.request(http::Method::POST, "/invalidation", xml, None, false)
            .await?;
        Ok(())
    }
}

fn failure(message: impl std::fmt::Display) -> FrameworkError {
    super::super::error(format!("CloudFront: {message}"))
}

fn element_text(xml: &str, path: &[&str]) -> Result<Option<String>, FrameworkError> {
    let mut reader = Reader::from_str(xml);
    let mut stack = Vec::new();
    loop {
        match reader
            .read_event()
            .map_err(|error| failure(format!("invalid XML response: {error}")))?
        {
            Event::Start(event) => {
                stack.push(String::from_utf8_lossy(event.local_name().as_ref()).into_owned());
                if stack.iter().map(String::as_str).eq(path.iter().copied()) {
                    let value = reader
                        .read_text(event.name())
                        .map_err(|error| failure(format!("invalid XML field: {error}")))?;
                    let value = value
                        .decode()
                        .map_err(|error| failure(format!("decode XML field: {error}")))?;
                    return quick_xml::escape::unescape(&value)
                        .map(|value| Some(value.into_owned()))
                        .map_err(|error| failure(format!("invalid XML entity: {error}")));
                }
            }
            Event::End(_) => {
                stack.pop();
            }
            Event::DocType(_) => return Err(failure("DOCTYPE is not allowed in an API response")),
            Event::Eof => return Ok(None),
            _ => {}
        }
    }
}

/// Find byte ranges with an XML parser, then change one field without reserializing unknown fields.
fn replace_origin_path(
    xml: &str,
    origin_id: &str,
    path: &str,
) -> Result<(String, String), FrameworkError> {
    let mut reader = Reader::from_str(xml);
    let mut stack: Vec<String> = Vec::new();
    let origin_path = ["DistributionConfig", "Origins", "Items", "Origin"];
    let mut current_id = None;
    let mut current_range = None;
    let mut replacements = Vec::new();
    loop {
        let before = reader.buffer_position() as usize;
        match reader
            .read_event()
            .map_err(|error| failure(format!("invalid distribution XML: {error}")))?
        {
            Event::Start(event) => {
                let name = String::from_utf8_lossy(event.local_name().as_ref()).into_owned();
                if stack.iter().map(String::as_str).eq(origin_path) && name == "Id" {
                    let text = reader
                        .read_text(event.name())
                        .map_err(|error| failure(format!("read origin id: {error}")))?;
                    let text = text
                        .decode()
                        .map_err(|error| failure(format!("decode origin id: {error}")))?;
                    current_id = Some(
                        quick_xml::escape::unescape(&text)
                            .map_err(|error| failure(format!("read origin id: {error}")))?
                            .into_owned(),
                    );
                } else if stack.iter().map(String::as_str).eq(origin_path) && name == "OriginPath" {
                    let start = reader.buffer_position() as usize;
                    let original = reader
                        .read_text(event.name())
                        .map_err(|error| failure(format!("read origin path: {error}")))?;
                    let original = original
                        .decode()
                        .map_err(|error| failure(format!("decode origin path: {error}")))?;
                    let original = quick_xml::escape::unescape(&original)
                        .map_err(|error| failure(format!("read origin path entity: {error}")))?
                        .into_owned();
                    let end = xml[..reader.buffer_position() as usize]
                        .rfind('<')
                        .ok_or_else(|| failure("origin path has no closing element"))?;
                    if current_range
                        .replace((
                            start..end,
                            quick_xml::escape::escape(path).into_owned(),
                            original,
                        ))
                        .is_some()
                    {
                        return Err(failure("an origin has duplicate OriginPath fields"));
                    }
                } else {
                    stack.push(name);
                }
            }
            Event::Empty(event)
                if stack.iter().map(String::as_str).eq(origin_path)
                    && event.local_name().as_ref() == b"OriginPath" =>
            {
                let end = reader.buffer_position() as usize;
                let name = String::from_utf8_lossy(event.name().as_ref()).into_owned();
                let opening = xml[before..end].trim_end().trim_end_matches("/>");
                let replacement = format!("{opening}>{}</{name}>", quick_xml::escape::escape(path));
                if current_range
                    .replace((before..end, replacement, String::new()))
                    .is_some()
                {
                    return Err(failure("an origin has duplicate OriginPath fields"));
                }
            }
            Event::End(_) => {
                if stack.iter().map(String::as_str).eq(origin_path) {
                    if current_id.as_deref() == Some(origin_id) {
                        replacements.push(current_range.take().ok_or_else(|| {
                            failure("the configured origin has no OriginPath field")
                        })?);
                    }
                    current_id = None;
                    current_range = None;
                }
                stack.pop();
            }
            Event::DocType(_) => {
                return Err(failure(
                    "DOCTYPE is not allowed in a distribution configuration",
                ));
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if replacements.len() != 1 {
        return Err(failure(
            "the distribution must contain exactly one origin with the configured id",
        ));
    }
    let (range, replacement, original) = replacements.remove(0);
    let mut updated = xml.to_owned();
    updated.replace_range(range, &replacement);
    Ok((updated, original))
}

// Each matching cache behavior needs its own bridge, including the default behavior.
fn validate_header_function(xml: &str, origin: &str, arn: &str) -> Result<(), FrameworkError> {
    let mut matched = false;
    for name in ["DefaultCacheBehavior", "CacheBehavior"] {
        for behavior in xml_elements(xml, name)? {
            if element_text(behavior, &[name, "TargetOriginId"])?.as_deref() != Some(origin) {
                continue;
            }
            matched = true;
            let mut bridged = false;
            for association in xml_elements(behavior, "FunctionAssociation")? {
                if element_text(association, &["FunctionAssociation", "EventType"])?.as_deref()
                    == Some("viewer-response")
                    && element_text(association, &["FunctionAssociation", "FunctionARN"])?
                        .as_deref()
                        == Some(arn)
                {
                    bridged = true;
                }
            }
            if !bridged {
                return Err(failure(
                    "a public origin cache behavior lacks the configured viewer-response header function",
                ));
            }
        }
    }
    if !matched {
        return Err(failure("no cache behavior uses the public page origin"));
    }
    Ok(())
}

fn xml_elements<'a>(xml: &'a str, name: &str) -> Result<Vec<&'a str>, FrameworkError> {
    let mut reader = Reader::from_str(xml);
    let mut elements = Vec::new();
    loop {
        let start = reader.buffer_position() as usize;
        match reader
            .read_event()
            .map_err(|error| failure(format!("read cache behavior: {error}")))?
        {
            Event::Start(event) if event.local_name().as_ref() == name.as_bytes() => {
                reader
                    .read_to_end(event.name())
                    .map_err(|error| failure(format!("read cache behavior: {error}")))?;
                elements.push(&xml[start..reader.buffer_position() as usize]);
            }
            Event::DocType(_) => {
                return Err(failure(
                    "DOCTYPE is not allowed in a distribution configuration",
                ));
            }
            Event::Eof => return Ok(elements),
            _ => {}
        }
    }
}
