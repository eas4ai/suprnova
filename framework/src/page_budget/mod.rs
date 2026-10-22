//! One evaluator for in-process pages, HTTP pages and captured HAR responses.

pub(crate) mod collect;
mod command;
mod discover;
mod har;

use crate::FrameworkError;
use regex::Regex;
use std::collections::BTreeSet;
use std::fmt;
use std::time::Duration;
use url::Url;

const DEFAULT_FINGERPRINT_PATTERN: &str = r"-[A-Za-z0-9_-]{8}$";

/// Limits shared by the test helper and deployment audit.
#[derive(Clone, Debug)]
pub struct PageBudget {
    /// Bound total response body transfer so a page stays usable on slower links.
    pub total_transfer_bytes: u64,
    /// Bound each asset so one resource cannot consume the page allowance.
    pub asset_bytes: u64,
    /// Require modern raster alternatives above this decoded response size.
    pub raster_threshold_bytes: u64,
    /// Allow this percentage above lossless PNG compression with the fast preset.
    pub png_margin_percent: u32,
    /// Bound the combined transfer of fonts referenced by the page.
    pub font_bytes: u64,
    /// Require the one-year public immutable policy on fingerprinted assets.
    pub require_immutable: bool,
    /// Reject static responses that issue any cookie.
    pub forbid_asset_set_cookie: bool,
    /// Reject captured static requests that carry cookies.
    pub forbid_asset_request_cookie: bool,
    /// Require assets to answer successfully without a cookie.
    pub require_cookie_independent: bool,
    /// Reject a redirect on the first document request.
    pub forbid_first_redirect: bool,
    /// Match the file stem so deployments can select their fingerprint convention.
    pub fingerprint_pattern: String,
    /// When present, reject external hosts outside this set. Hosts are always listed.
    pub allowed_third_party_hosts: Option<BTreeSet<String>>,
    /// Stop cyclic or unusually large document graphs before an unbounded fetch.
    pub max_resources: usize,
    /// Bound one response's memory use during collection.
    pub max_response_bytes: usize,
    /// Bound each outbound request.
    pub request_timeout: Duration,
    /// Bound the complete page collection, including redirects and assets.
    pub collection_timeout: Duration,
}

impl Default for PageBudget {
    fn default() -> Self {
        Self {
            total_transfer_bytes: 1_000_000,
            asset_bytes: 250_000,
            raster_threshold_bytes: 100_000,
            png_margin_percent: 10,
            font_bytes: 100_000,
            require_immutable: true,
            forbid_asset_set_cookie: true,
            forbid_asset_request_cookie: true,
            require_cookie_independent: true,
            forbid_first_redirect: true,
            // Vite's hash alphabet includes '-' and '_', including within the hash.
            fingerprint_pattern: DEFAULT_FINGERPRINT_PATTERN.into(),
            allowed_third_party_hosts: None,
            max_resources: 256,
            max_response_bytes: 16 * 1024 * 1024,
            request_timeout: Duration::from_secs(10),
            collection_timeout: Duration::from_secs(30),
        }
    }
}

/// A failed rule includes enough context to fix the offending response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BudgetFinding {
    /// Identify the response or page whose budget failed.
    pub asset: String,
    /// Name the failed rule consistently across both runners.
    pub rule: String,
    /// Preserve the observation without exposing cookie contents.
    pub measured: String,
    /// State the policy the response must meet.
    pub allowed: String,
}

/// Measurements and all failures from one page capture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageAudit {
    /// Keep every failed rule so one run reports the complete repair list.
    pub findings: Vec<BudgetFinding>,
    /// List hosts a page reaches even when the caller allows them.
    pub third_party_hosts: Vec<String>,
    /// Include all captured requests, including redirects and third parties in a HAR.
    pub requests: usize,
    /// Report response body transfer separately from decoded resource sizes.
    pub total_transfer_bytes: u64,
}

impl PageAudit {
    /// Return every finding as one error so tests can use `?` without a panic.
    pub fn ensure_passes(&self) -> Result<(), FrameworkError> {
        if self.findings.is_empty() {
            Ok(())
        } else {
            Err(FrameworkError::bad_request(self.to_string()))
        }
    }

    fn fail(
        &mut self,
        asset: impl ToString,
        rule: &str,
        measured: impl ToString,
        allowed: impl ToString,
    ) {
        self.findings.push(BudgetFinding {
            asset: asset.to_string(),
            rule: rule.into(),
            measured: measured.to_string(),
            allowed: allowed.to_string(),
        });
    }
}

impl fmt::Display for PageAudit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "{} requests, {} transferred bytes",
            self.requests, self.total_transfer_bytes
        )?;
        writeln!(f, "asset | rule | measured | allowed")?;
        // Escape controls because a HAR is untrusted terminal input.
        let clean = |s: &str| s.chars().flat_map(char::escape_default).collect::<String>();
        for finding in &self.findings {
            writeln!(
                f,
                "{} | {} | {} | {}",
                clean(&finding.asset),
                clean(&finding.rule),
                clean(&finding.measured),
                clean(&finding.allowed)
            )?;
        }
        writeln!(
            f,
            "Third-party hosts: {}",
            if self.third_party_hosts.is_empty() {
                "none".into()
            } else {
                self.third_party_hosts
                    .iter()
                    .map(|s| clean(s))
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        )
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Resource {
    pub url: Url,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub transfer: u64,
    pub decoded: u64,
    pub request_cookie: bool,
    pub document: bool,
    pub manifest_icon: bool,
    pub manifest: bool,
}

impl Resource {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    fn mime(&self) -> &str {
        self.header("content-type")
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
    }
}

pub(crate) fn parse_url(value: &str) -> Result<Url, FrameworkError> {
    let url = if value.starts_with('/') {
        Url::parse("http://localhost").and_then(|base| base.join(value))
    } else {
        Url::parse(value)
    }
    .map_err(|e| FrameworkError::bad_request(format!("page budget URL {value:?}: {e}")))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(FrameworkError::bad_request(
            "page budget requires an HTTP(S) URL without credentials",
        ));
    }
    Ok(url)
}

pub(crate) fn same_origin(left: &Url, right: &Url) -> bool {
    left.origin() == right.origin()
}

pub(crate) fn fingerprint_regex(pattern: &str) -> Result<Regex, FrameworkError> {
    Regex::new(pattern)
        .map_err(|e| FrameworkError::bad_request(format!("page budget fingerprint pattern: {e}")))
}

pub(crate) fn fingerprinted(url: &Url, pattern: &Regex) -> bool {
    let file = url.path().rsplit('/').next().unwrap_or("");
    let stem = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
    pattern.is_match(stem)
}

pub(crate) fn default_fingerprinted_path(path: &std::path::Path) -> bool {
    static PATTERN: std::sync::LazyLock<Result<Regex, regex::Error>> =
        std::sync::LazyLock::new(|| Regex::new(DEFAULT_FINGERPRINT_PATTERN));
    let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
        return false;
    };
    PATTERN.as_ref().is_ok_and(|pattern| pattern.is_match(stem))
}

impl PageBudget {
    /// Apply the shared evaluator to a HAR export without any network requests.
    pub fn audit_har(&self, json: &str) -> Result<PageAudit, FrameworkError> {
        let capture = har::read(json, self)?;
        self.evaluate(capture)
    }

    /// Fetch the document and same-origin assets through the framework HTTP client.
    pub async fn audit_url(&self, url: &str) -> Result<PageAudit, FrameworkError> {
        collect::audit_http(self, parse_url(url)?).await
    }

    fn evaluate(&self, capture: collect::Capture) -> Result<PageAudit, FrameworkError> {
        let pattern = fingerprint_regex(&self.fingerprint_pattern)?;
        let resources = capture.resources;
        let by_url: std::collections::BTreeMap<_, _> =
            resources.iter().map(|r| (r.url.as_str(), r)).collect();
        let mut alternatives = capture.alternatives;
        for group in capture.image_groups {
            let variants: Vec<_> = group
                .iter()
                .filter_map(|url| by_url.get(url.as_str()).copied())
                .collect();
            if variants.iter().any(|r| is_modern(r))
                && variants.iter().any(|r| is_raster(r) && !is_modern(r))
            {
                alternatives.extend(group);
            }
        }
        let total = resources
            .iter()
            .fold(0u64, |total, r| total.saturating_add(r.transfer));
        let mut audit = PageAudit {
            findings: Vec::new(),
            third_party_hosts: capture.hosts.into_iter().collect(),
            requests: resources.len(),
            total_transfer_bytes: total,
        };
        let page = capture.page.as_str();
        if total > self.total_transfer_bytes {
            audit.fail(
                page,
                "total-transfer",
                format!("{total} bytes"),
                format!("<= {} bytes", self.total_transfer_bytes),
            );
        }
        let mut fonts = Vec::new();
        for (index, resource) in resources.iter().enumerate() {
            let url = resource.url.as_str();
            let status = resource.status;
            if index == 0 && self.forbid_first_redirect && (300..400).contains(&status) {
                audit.fail(url, "first-redirect", status, "no redirect (2xx)");
            }
            if resource.document {
                if !(200..400).contains(&status) {
                    audit.fail(url, "document-status", status, "2xx document");
                }
                continue;
            }
            if resource.transfer > self.asset_bytes {
                audit.fail(
                    url,
                    "asset-size",
                    format!("{} bytes", resource.transfer),
                    format!("<= {} bytes", self.asset_bytes),
                );
            }
            if is_font(resource) {
                fonts.push(resource);
            }
            if !(200..300).contains(&status) {
                audit.fail(url, "asset-status", status, "2xx");
            }
            if self.require_cookie_independent
                && (resource.request_cookie || matches!(status, 300..=399 | 401 | 403))
            {
                audit.fail(
                    url,
                    "cookie-independent",
                    if resource.request_cookie {
                        "Cookie sent; anonymous access not proven".to_owned()
                    } else {
                        format!("{status} without proven anonymous access")
                    },
                    "2xx without cookies",
                );
            }
            if self.forbid_asset_set_cookie && resource.header("set-cookie").is_some() {
                audit.fail(
                    url,
                    "asset-set-cookie",
                    "Set-Cookie present",
                    "no Set-Cookie",
                );
            }
            if self.forbid_asset_request_cookie && resource.request_cookie {
                audit.fail(url, "asset-request-cookie", "Cookie present", "no Cookie");
            }
            if self.require_immutable
                && fingerprinted(&resource.url, &pattern)
                && !immutable(resource.header("cache-control").unwrap_or(""))
            {
                audit.fail(
                    url,
                    "fingerprint-cache",
                    resource.header("cache-control").unwrap_or("missing"),
                    "public, max-age=31536000, immutable",
                );
            }
            if resource.decoded > self.raster_threshold_bytes && is_raster(resource) {
                if !resource.manifest_icon && !alternatives.contains(url) {
                    audit.fail(
                        url,
                        "raster-format",
                        format!(
                            "{}; {} bytes; no modern/fallback pair",
                            resource.mime(),
                            resource.decoded
                        ),
                        format!(
                            "WebP or AVIF with fallback above {} bytes",
                            self.raster_threshold_bytes
                        ),
                    );
                }
                if is_png(resource) {
                    self.check_png(&mut audit, resource)?;
                }
            }
        }
        let font_total = fonts
            .iter()
            .fold(0u64, |total, r| total.saturating_add(r.transfer));
        if font_total > self.font_bytes {
            audit.fail(
                fonts
                    .iter()
                    .map(|r| r.url.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                "font-budget",
                format!("{font_total} bytes"),
                format!("<= {} bytes", self.font_bytes),
            );
        }
        if let Some(allowed) = &self.allowed_third_party_hosts {
            for host in audit.third_party_hosts.clone() {
                if !allowed.contains(&host) {
                    audit.fail(
                        host,
                        "third-party-host",
                        "unlisted host",
                        if allowed.is_empty() {
                            "no third-party hosts".to_owned()
                        } else {
                            allowed.iter().cloned().collect::<Vec<_>>().join(", ")
                        },
                    );
                }
            }
        }
        Ok(audit)
    }

    fn check_png(&self, audit: &mut PageAudit, resource: &Resource) -> Result<(), FrameworkError> {
        let url = resource.url.as_str();
        let allowed = format!("<= fast PNG size + {}%", self.png_margin_percent);
        let Some(body) = &resource.body else {
            audit.fail(url, "png-compression", "PNG body unavailable", allowed);
            return Ok(());
        };
        #[cfg(feature = "media")]
        match crate::media::recompress_png(body, crate::media::PngEffort::Fast) {
            Ok(compressed) => {
                let limit = (compressed.len() as u64)
                    .saturating_mul(100 + u64::from(self.png_margin_percent))
                    / 100;
                if body.len() as u64 > limit {
                    audit.fail(
                        url,
                        "png-compression",
                        format!("{} bytes", body.len()),
                        format!(
                            "<= {limit} bytes (fast {} bytes + {}%)",
                            compressed.len(),
                            self.png_margin_percent
                        ),
                    );
                }
            }
            Err(error) => audit.fail(
                url,
                "png-compression",
                format!("invalid PNG: {error}"),
                allowed,
            ),
        }
        #[cfg(not(feature = "media"))]
        {
            let _ = body;
            audit.fail(
                url,
                "png-compression",
                "media feature disabled",
                "enable media to check PNG compression",
            );
        }
        Ok(())
    }
}

fn extension(resource: &Resource) -> &str {
    resource.url.path().rsplit('.').next().unwrap_or("")
}
fn is_font(r: &Resource) -> bool {
    r.mime().starts_with("font/")
        || matches!(
            r.mime(),
            "application/font-woff" | "application/vnd.ms-fontobject"
        )
        || matches!(extension(r), "woff" | "woff2" | "ttf" | "otf" | "eot")
}
fn is_png(r: &Resource) -> bool {
    r.mime() == "image/png"
        || extension(r).eq_ignore_ascii_case("png")
        || r.body
            .as_ref()
            .is_some_and(|b| b.starts_with(b"\x89PNG\r\n\x1a\n"))
}
fn is_raster(r: &Resource) -> bool {
    matches!(
        r.mime(),
        "image/png" | "image/jpeg" | "image/gif" | "image/bmp" | "image/webp" | "image/avif"
    ) || matches!(
        extension(r),
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "avif"
    )
}
fn is_modern(resource: &Resource) -> bool {
    matches!(resource.mime(), "image/webp" | "image/avif")
        || matches!(extension(resource), "webp" | "avif")
}

fn immutable(value: &str) -> bool {
    let directives: Vec<_> = value
        .split(',')
        .map(|s| s.trim().to_ascii_lowercase())
        .collect();
    let mut ages = directives.iter().filter(|s| {
        s.split('=')
            .next()
            .is_some_and(|key| key.trim() == "max-age")
    });
    let age = ages.next();
    directives.iter().any(|s| s == "public")
        && directives.iter().any(|s| s == "immutable")
        && age.is_some_and(|s| {
            s.split_once('=').is_some_and(|(_, value)| {
                value.trim().trim_matches('"').parse::<u64>() == Ok(31_536_000)
            })
        })
        && ages.next().is_none()
        && !directives.iter().any(|s| {
            matches!(
                s.split('=').next().unwrap_or("").trim(),
                "private" | "no-store" | "no-cache"
            )
        })
}
