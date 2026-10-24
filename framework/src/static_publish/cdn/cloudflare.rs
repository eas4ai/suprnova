//! Cloudflare serves an R2 bucket's custom domain; the driver owns the rules and purges.

use std::{collections::BTreeMap, sync::Mutex, time::Duration};

use async_trait::async_trait;
use http::header::{AUTHORIZATION, HeaderValue};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{CdnCapabilities, CdnDriver, Purge};
use crate::{FrameworkError, filesystem::S3Config};

/// The transform phase that rewrites a page path to its `index.html` object.
const TRANSFORM_PHASE: &str = "http_request_transform";
/// The phase that adds the published headers to every response on the host.
const HEADERS_PHASE: &str = "http_response_headers_transform";
/// The phase that makes HTML and JSON on the host eligible for Cloudflare's cache.
const CACHE_PHASE: &str = "http_request_cache_settings";
/// Every rule the driver owns carries a `ref` starting with this, so it never touches
/// a rule the application wrote on the same zone.
const REF_PREFIX: &str = "suprnova-static-publication-";

/// Headers R2 serves from the object's own HTTP metadata, so no rule repeats them.
const OBJECT_HEADERS: [&str; 6] = [
    "content-type",
    "cache-control",
    "content-encoding",
    "content-language",
    "content-disposition",
    "expires",
];

/// Headers no response header rule may set.
const BLOCKED_HEADERS: [&str; 20] = [
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
];

/// What the host in front of the publication is.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Mode {
    /// An R2 bucket's custom domain: the bucket is the origin, so the driver rewrites
    /// page paths to the mirrored `index.html` objects and carries the published headers.
    R2Domain {
        /// The disk's root inside the bucket, as a leading-slash prefix, or empty.
        root_prefix: String,
    },
    /// The application's own proxied hostname: the origin serves the published
    /// objects itself and sends their headers, so only the cache rule is needed.
    Origin,
}

/// A Cloudflare zone in front of a static publication: an R2 bucket's custom domain
/// (`new`), or the application's proxied hostname (`for_origin`).
///
/// Nothing runs at the edge. On a custom domain the bucket's stable mirror of the
/// live version (`<path>/index.html`, `<path>/index.json`, written after every
/// manifest commit) is what the domain serves, so the switch is the mirror and the
/// driver has no origin path to move. It owns four rules on the zone, each on
/// `http.host eq "<host>"` and identified by a `ref` that starts with
/// `suprnova-static-publication-`: two URL rewrites that map `/about` and `/about/`
/// to `/about/index.html` and leave paths with an extension or under `/_pub/` alone,
/// one response header rule that sets the headers the publisher validated (R2 serves
/// `Content-Type`, `Cache-Control` and the other object headers itself), and one cache
/// rule that makes a response on the host eligible for the cache only when its own
/// `Cache-Control` allows a shared cache, as every published object's does, and for
/// the time it names. The rules are created on the first switch and corrected on later
/// ones through the Rulesets API (phase entrypoints, `PUT` to create one, `POST`,
/// `PATCH` and `DELETE` on single rules); a first-publication rollback deletes them.
///
/// Purges go to `POST /zones/{zone_id}/purge_cache` with full URLs on the host,
/// batched at 100 per request, the limit on Free, Pro and Business (Enterprise
/// allows 500), or a prefix; every plan supports both. Tag purges clear only
/// responses that carried a `Cache-Tag` header, which the publication does not set,
/// so the driver leaves them to a caller that sets the header itself.
///
/// The application supplies a zone API token with Cache Purge, Transform Rules edit
/// and Cache Rules edit; the driver reads no environment variable, marks the header
/// sensitive and never copies the token into a log or an error.
pub struct CloudflareDriver {
    zone_id: String,
    host: String,
    mode: Mode,
    authorization: HeaderValue,
    endpoint: url::Url,
    client: reqwest::Client,
    /// The headers `validate_headers` accepted, carried into the response header rule.
    headers: Mutex<BTreeMap<String, String>>,
}

#[derive(Deserialize)]
struct Envelope {
    success: bool,
    #[serde(default)]
    errors: Vec<ApiError>,
    #[serde(default)]
    result: Value,
}

#[derive(Deserialize)]
struct ApiError {
    code: i64,
    message: String,
}

/// One rule the driver wants on the zone, in the form the Rulesets API takes.
struct Desired {
    phase: &'static str,
    rule: Value,
}

/// A phase entrypoint ruleset as the API returned it.
struct Entrypoint {
    id: String,
    rules: Vec<Value>,
}

impl CloudflareDriver {
    /// The driver for an R2 bucket's custom domain `host`; `config` is the bucket disk,
    /// whose root prefix the rewrites prepend.
    pub fn new(
        zone_id: &str,
        host: &str,
        api_token: &str,
        config: &S3Config,
    ) -> Result<Self, FrameworkError> {
        let root = config.root.as_deref().unwrap_or("").trim_matches('/');
        if !root.is_empty()
            && (root
                .split('/')
                .any(|part| part.is_empty() || matches!(part, "." | ".."))
                || !root
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-_/.".contains(&byte)))
        {
            return Err(failure("S3 root must contain unambiguous path segments"));
        }
        let root_prefix = if root.is_empty() {
            String::new()
        } else {
            format!("/{root}")
        };
        Self::build(zone_id, host, api_token, Mode::R2Domain { root_prefix })
    }

    /// The driver for the application's own proxied hostname: the origin serves the
    /// published objects and their headers, so the driver keeps the cache rule and
    /// purges, and writes no rewrite or header rule.
    pub fn for_origin(zone_id: &str, host: &str, api_token: &str) -> Result<Self, FrameworkError> {
        Self::build(zone_id, host, api_token, Mode::Origin)
    }

    fn build(
        zone_id: &str,
        host: &str,
        api_token: &str,
        mode: Mode,
    ) -> Result<Self, FrameworkError> {
        if zone_id.is_empty()
            || zone_id.len() > 32
            || !zone_id.bytes().all(|byte| byte.is_ascii_alphanumeric())
        {
            return Err(failure(
                "zone id must contain at most 32 ASCII letters and digits",
            ));
        }
        if host.is_empty()
            || host.len() > 253
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
            mode,
            authorization,
            endpoint: url::Url::parse("https://api.cloudflare.com/client/v4/")
                .map_err(|_| failure("parse API endpoint"))?,
            client,
            headers: Mutex::new(BTreeMap::new()),
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

    /// The rules this driver wants on the zone right now, in phase order.
    fn desired_rules(&self) -> Result<Vec<Desired>, FrameworkError> {
        let host = format!("http.host eq {}", quote(&self.host));
        let mut rules = Vec::new();
        if let Mode::R2Domain { root_prefix } = &self.mode {
            let prefix = quote(root_prefix);
            rules.push(Desired {
                phase: TRANSFORM_PHASE,
                rule: json!({
                    "ref": format!("{REF_PREFIX}index-slash"),
                    "description": "Suprnova static publication: a path ending in / serves its index.html",
                    "expression": format!("({host} and ends_with(http.request.uri.path, \"/\"))"),
                    "action": "rewrite",
                    "action_parameters": {"uri": {"path": {"expression": format!("concat({prefix}, http.request.uri.path, \"index.html\")")}}},
                }),
            });
            rules.push(Desired {
                phase: TRANSFORM_PHASE,
                rule: json!({
                    "ref": format!("{REF_PREFIX}index"),
                    "description": "Suprnova static publication: a page path serves its index.html",
                    "expression": format!(
                        "({host} and not ends_with(http.request.uri.path, \"/\") and not http.request.uri.path contains \".\" and not starts_with(http.request.uri.path, \"/_pub/\"))"
                    ),
                    "action": "rewrite",
                    "action_parameters": {"uri": {"path": {"expression": format!("concat({prefix}, http.request.uri.path, \"/index.html\")")}}},
                }),
            });
            if !root_prefix.is_empty() {
                rules.push(Desired {
                    phase: TRANSFORM_PHASE,
                    rule: json!({
                        "ref": format!("{REF_PREFIX}objects"),
                        "description": "Suprnova static publication: assets live below the disk root",
                        "expression": format!(
                            "({host} and (http.request.uri.path contains \".\" or starts_with(http.request.uri.path, \"/_pub/\")))"
                        ),
                        "action": "rewrite",
                        "action_parameters": {"uri": {"path": {"expression": format!("concat({prefix}, http.request.uri.path)")}}},
                    }),
                });
            }
            let headers = self
                .headers
                .lock()
                .map_err(|_| failure("header lock poisoned"))?
                .clone();
            let carried: serde_json::Map<String, Value> = headers
                .iter()
                .filter(|(name, _)| !OBJECT_HEADERS.contains(&name.as_str()))
                .map(|(name, value)| (name.clone(), json!({"operation": "set", "value": value})))
                .collect();
            if !carried.is_empty() {
                rules.push(Desired {
                    phase: HEADERS_PHASE,
                    rule: json!({
                        "ref": format!("{REF_PREFIX}headers"),
                        "description": "Suprnova static publication: the published response headers",
                        "expression": format!("({host})"),
                        "action": "rewrite",
                        "action_parameters": {"headers": Value::Object(carried)},
                    }),
                });
            }
        }
        rules.push(Desired {
            phase: CACHE_PHASE,
            rule: json!({
                "ref": format!("{REF_PREFIX}cache"),
                "description": "Suprnova static publication: responses that say they are cacheable are cached for the time they say",
                "expression": format!("({host})"),
                "action": "set_cache_settings",
                "action_parameters": {
                    "cache": true,
                    // Bypass unless the response's own Cache-Control allows a shared
                    // cache: the published pages say `public, s-maxage`, and a dynamic
                    // page on the same host that says nothing is never cached, so no
                    // visitor's page reaches another visitor.
                    "edge_ttl": {"mode": "bypass_by_default"},
                    "browser_ttl": {"mode": "respect_origin"},
                },
            }),
        });
        Ok(rules)
    }

    /// Make the zone's rules match `desired_rules`: create a missing phase entrypoint
    /// with the rules it needs, add or correct the driver's rules in an existing one,
    /// and never write a rule whose `ref` is not the driver's.
    async fn sync_rules(&self) -> Result<(), FrameworkError> {
        let desired = self.desired_rules()?;
        for phase in [TRANSFORM_PHASE, HEADERS_PHASE, CACHE_PHASE] {
            let wanted: Vec<&Value> = desired
                .iter()
                .filter(|d| d.phase == phase)
                .map(|d| &d.rule)
                .collect();
            if wanted.is_empty() {
                continue;
            }
            let Some(entrypoint) = self.entrypoint(phase).await? else {
                self.api(
                    reqwest::Method::PUT,
                    &format!("zones/{}/rulesets/phases/{phase}/entrypoint", self.zone_id),
                    Some(json!({"rules": wanted})),
                    "create ruleset",
                )
                .await?;
                continue;
            };
            for rule in wanted {
                let reference = rule["ref"].as_str().unwrap_or_default();
                match entrypoint
                    .rules
                    .iter()
                    .find(|existing| existing["ref"] == reference)
                {
                    Some(existing) if same_rule(existing, rule) => {}
                    Some(existing) => {
                        let id = existing["id"].as_str().unwrap_or_default();
                        self.api(
                            reqwest::Method::PATCH,
                            &format!(
                                "zones/{}/rulesets/{}/rules/{id}",
                                self.zone_id, entrypoint.id
                            ),
                            Some(rule.clone()),
                            "update rule",
                        )
                        .await?;
                    }
                    None => {
                        self.api(
                            reqwest::Method::POST,
                            &format!("zones/{}/rulesets/{}/rules", self.zone_id, entrypoint.id),
                            Some(rule.clone()),
                            "add rule",
                        )
                        .await?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Delete every rule of the driver's on the zone, leaving the rest as they are.
    async fn delete_rules(&self) -> Result<(), FrameworkError> {
        for phase in [TRANSFORM_PHASE, HEADERS_PHASE, CACHE_PHASE] {
            let Some(entrypoint) = self.entrypoint(phase).await? else {
                continue;
            };
            for rule in &entrypoint.rules {
                if rule["ref"]
                    .as_str()
                    .is_some_and(|reference| reference.starts_with(REF_PREFIX))
                {
                    let id = rule["id"].as_str().unwrap_or_default();
                    self.api(
                        reqwest::Method::DELETE,
                        &format!(
                            "zones/{}/rulesets/{}/rules/{id}",
                            self.zone_id, entrypoint.id
                        ),
                        None,
                        "delete rule",
                    )
                    .await?;
                }
            }
        }
        Ok(())
    }

    /// The phase's entrypoint ruleset, or `None` when the zone has none yet.
    async fn entrypoint(&self, phase: &str) -> Result<Option<Entrypoint>, FrameworkError> {
        let path = format!("zones/{}/rulesets/phases/{phase}/entrypoint", self.zone_id);
        let (status, envelope) = self
            .send(reqwest::Method::GET, &path, None, "read ruleset")
            .await?;
        if status == 404 {
            return Ok(None);
        }
        let envelope = self.checked(status, envelope, "read ruleset")?;
        let id = envelope.result["id"]
            .as_str()
            .ok_or_else(|| failure("read ruleset: the entrypoint has no id"))?
            .to_owned();
        let rules = envelope.result["rules"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        Ok(Some(Entrypoint { id, rules }))
    }

    async fn api(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
        operation: &str,
    ) -> Result<Envelope, FrameworkError> {
        let (status, envelope) = self.send(method, path, body, operation).await?;
        self.checked(status, envelope, operation)
    }

    async fn send(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
        operation: &str,
    ) -> Result<(u16, Envelope), FrameworkError> {
        let url = self
            .endpoint
            .join(path)
            .map_err(|_| failure(format!("{operation}: build API URL")))?;
        let mut request = self
            .client
            .request(method, url)
            .header(AUTHORIZATION, self.authorization.clone());
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request
            .send()
            .await
            .map_err(|_| failure(format!("{operation}: send API request failed")))?;
        let status = response.status().as_u16();
        let bytes = crate::http_client::read_capped(response, 2 * 1024 * 1024)
            .await
            .map_err(|error| failure(format!("{operation}: HTTP {status}; {error}")))?;
        let envelope = serde_json::from_slice::<Envelope>(&bytes).map_err(|_| {
            failure(format!(
                "{operation}: HTTP {status}; unreadable Cloudflare envelope"
            ))
        })?;
        Ok((status, envelope))
    }

    /// Turn a failed envelope into an error naming the status and the first error,
    /// with the token redacted should the provider echo it.
    fn checked(
        &self,
        status: u16,
        envelope: Envelope,
        operation: &str,
    ) -> Result<Envelope, FrameworkError> {
        if (200..300).contains(&status) && envelope.success {
            return Ok(envelope);
        }
        let token = self
            .authorization
            .to_str()
            .ok()
            .and_then(|header| header.strip_prefix("Bearer "))
            .map(str::to_owned);
        let detail = envelope
            .errors
            .first()
            .map(|error| {
                let message = match &token {
                    Some(token) => error.message.replace(token, "[redacted]"),
                    None => "unreadable provider error".into(),
                };
                format!("code {}: {message}", error.code)
            })
            .unwrap_or_else(|| "no Cloudflare error details".into());
        Err(failure(format!("{operation}: HTTP {status}; {detail}")))
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
}

/// Whether an existing rule already says what the driver wants: the fields the
/// driver writes compare equal; the id, version and timestamps the API adds do not.
fn same_rule(existing: &Value, wanted: &Value) -> bool {
    [
        "expression",
        "action",
        "action_parameters",
        "description",
        "enabled",
    ]
    .iter()
    .all(|field| match (existing.get(field), wanted.get(field)) {
        (_, None) => true,
        (Some(have), Some(want)) => have == want,
        (None, Some(_)) => false,
    })
}

/// A string literal of the Rules language.
fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

#[async_trait]
impl CdnDriver for CloudflareDriver {
    fn capabilities(&self) -> CdnCapabilities {
        CdnCapabilities {
            urls: true,
            prefix: true,
            tags: false,
        }
    }

    /// The custom domain serves the bucket's stable mirror; the application's own
    /// origin serves the versioned objects itself and needs none.
    fn serves_mirror(&self) -> bool {
        matches!(self.mode, Mode::R2Domain { .. })
    }

    fn validate_headers(&self, headers: &BTreeMap<String, String>) -> Result<(), FrameworkError> {
        let mut accepted = BTreeMap::new();
        for (name, value) in headers {
            let lower = name.to_ascii_lowercase();
            if BLOCKED_HEADERS.contains(&lower.as_str())
                || ["cf-", "x-accel-"]
                    .iter()
                    .any(|prefix| lower.starts_with(prefix))
            {
                return Err(failure(format!(
                    "header {lower} cannot be set by a response header rule"
                )));
            }
            http::HeaderName::from_bytes(lower.as_bytes())
                .map_err(|_| failure(format!("invalid public header {lower}")))?;
            HeaderValue::from_str(value)
                .map_err(|_| failure(format!("invalid public header value for {lower}")))?;
            accepted.insert(lower, value.clone());
        }
        *self
            .headers
            .lock()
            .map_err(|_| failure("header lock poisoned"))? = accepted;
        Ok(())
    }

    /// Unknown on purpose: the mirror of the live version is what the domain serves,
    /// so the publisher realigns with its durable manifest.
    async fn selected_version(&self) -> Result<Option<String>, FrameworkError> {
        Ok(None)
    }

    /// The mirror written after the manifest commit is the switch; the rules that
    /// serve it are made right here, on every switch, so a rule someone removed
    /// comes back before the next publication is served.
    async fn switch_version(&self, _version: &str) -> Result<(), FrameworkError> {
        self.sync_rules().await
    }

    /// A later rollback has nothing to move; the first publication's rollback removes
    /// the rules the driver created, so the zone is as it was.
    async fn rollback_version(&self, previous: Option<&str>) -> Result<(), FrameworkError> {
        match previous {
            Some(_) => Ok(()),
            None => self.delete_rules().await,
        }
    }

    async fn purge(&self, request: Purge) -> Result<(), FrameworkError> {
        let bodies = match request {
            Purge::Urls(paths) => {
                let urls = paths
                    .iter()
                    .map(|path| self.purge_url(path))
                    .collect::<Result<Vec<_>, _>>()?;
                urls.chunks(100)
                    .map(|batch| json!({"files": batch}))
                    .collect::<Vec<_>>()
            }
            Purge::Prefix(prefix) => {
                self.purge_url(&prefix)?;
                if prefix.contains('?') {
                    return Err(failure("prefix purge does not accept a query string"));
                }
                vec![json!({"prefixes": [format!("{}{prefix}", self.host)]})]
            }
            Purge::Tags(tags) => {
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
        for body in bodies {
            self.api(
                reqwest::Method::POST,
                &format!("zones/{}/purge_cache", self.zone_id),
                Some(body),
                "purge cache",
            )
            .await?;
        }
        Ok(())
    }
}

fn failure(message: impl std::fmt::Display) -> FrameworkError {
    super::super::error(format!("Cloudflare: {message}"))
}
