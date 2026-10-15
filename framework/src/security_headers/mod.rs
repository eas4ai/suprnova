//! Browser security headers on the framework's middleware chain.
//!
//! Typed registration puts this middleware outside cache and error middleware,
//! regardless of registration order. Route overrides run directly inside it so
//! cached responses obey the route's policy too.

mod nonce;
mod policy;
pub use nonce::CspNonce;
pub(crate) use nonce::NonceState;
pub use policy::ContentSecurityPolicy;

use crate::{Config, FrameworkError, HttpResponse, Middleware, Next, Request, Response};
use async_trait::async_trait;
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

const STRICT_PERMISSIONS: &str = "camera=(), microphone=(), geolocation=(), payment=(), usb=(), accelerometer=(), gyroscope=(), magnetometer=(), browsing-topics=(), display-capture=()";
// Balanced leaves `payment` alone: Stripe's Apple Pay and Google Pay buttons
// use the Payment Request API from Stripe's frames.
const BALANCED_PERMISSIONS: &str = "camera=(), microphone=(), geolocation=(), usb=(), accelerometer=(), gyroscope=(), magnetometer=(), browsing-topics=(), display-capture=()";

tokio::task_local! {
    static POLICY: Arc<RwLock<SecurityHeaders>>;
}

/// Configures response headers and owns the nonce scope surrounding rendering.
///
/// Existing response headers win. HSTS requires a trusted secure request outside
/// local development, and nonce CSP headers apply only to HTML responses.
#[derive(Clone, Debug)]
pub struct SecurityHeaders {
    headers: BTreeMap<String, String>,
    csp: Option<ContentSecurityPolicy>,
    csp_report_only: Option<ContentSecurityPolicy>,
    report_only: bool,
    report_to: Option<String>,
    reporting_dsns: Vec<String>,
}

impl SecurityHeaders {
    /// Isolates documents and requires nonces for scripts and style elements.
    pub fn strict() -> Self {
        Self::preset(true)
    }

    /// Keeps inline component styles compatible while requiring nonced scripts.
    pub fn balanced() -> Self {
        Self::preset(false)
    }

    fn preset(strict: bool) -> Self {
        let mut policy = Self::relaxed();
        policy.csp = Some(if strict {
            ContentSecurityPolicy::strict()
        } else {
            ContentSecurityPolicy::balanced()
        });
        for (name, value) in [
            (
                "permissions-policy",
                if strict {
                    STRICT_PERMISSIONS
                } else {
                    BALANCED_PERMISSIONS
                },
            ),
            // A popup sign-in or checkout window keeps its link to the page
            // under balanced, so it can report back to the opener.
            (
                "cross-origin-opener-policy",
                if strict {
                    "same-origin"
                } else {
                    "same-origin-allow-popups"
                },
            ),
            (
                "cross-origin-resource-policy",
                if strict { "same-origin" } else { "same-site" },
            ),
        ] {
            policy.headers.insert(name.into(), value.into());
        }
        if strict {
            for (name, value) in [
                (
                    "strict-transport-security",
                    "max-age=63072000; includeSubDomains",
                ),
                ("x-frame-options", "DENY"),
                ("referrer-policy", "no-referrer"),
                ("cross-origin-embedder-policy", "require-corp"),
            ] {
                policy.headers.insert(name.into(), value.into());
            }
        }
        policy
    }

    /// Adds transport, framing, MIME and referrer safeguards without a nonce CSP.
    pub fn relaxed() -> Self {
        Self {
            headers: [
                ("strict-transport-security", "max-age=31536000"),
                ("x-frame-options", "SAMEORIGIN"),
                ("x-content-type-options", "nosniff"),
                ("referrer-policy", "strict-origin-when-cross-origin"),
            ]
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect(),
            ..Self::off()
        }
    }

    /// Retains registration while disabling all middleware-owned headers.
    pub fn off() -> Self {
        Self {
            headers: BTreeMap::new(),
            csp: None,
            csp_report_only: None,
            report_only: false,
            report_to: None,
            reporting_dsns: Vec::new(),
        }
    }

    /// Validates every security environment setting so misspellings fail at boot.
    ///
    /// An unset `SECURITY_HEADERS` selects balanced. Invalid settings return an
    /// error naming their variable; no invalid value falls back to a preset.
    pub fn from_env() -> Result<Self, FrameworkError> {
        Self::from_values(|name| match std::env::var(name) {
            Ok(value) => Ok(Some(value)),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(_) => Err(FrameworkError::internal(format!(
                "{name}: expected Unicode text"
            ))),
        })
    }

    fn from_values(
        mut read: impl FnMut(&str) -> Result<Option<String>, FrameworkError>,
    ) -> Result<Self, FrameworkError> {
        let mut policy = match read("SECURITY_HEADERS")?.as_deref().unwrap_or("balanced") {
            "strict" => Self::strict(),
            "balanced" => Self::balanced(),
            "relaxed" => Self::relaxed(),
            "off" => Self::off(),
            _ => {
                return Err(FrameworkError::internal(
                    "SECURITY_HEADERS: expected strict, balanced, relaxed or off",
                ));
            }
        };
        policy = policy.report_only(read_bool(
            "SECURITY_HEADERS_CSP_REPORT_ONLY",
            read("SECURITY_HEADERS_CSP_REPORT_ONLY")?,
        )?);
        let preload = read_bool(
            "SECURITY_HEADERS_HSTS_PRELOAD",
            read("SECURITY_HEADERS_HSTS_PRELOAD")?,
        )?;
        if let Some(endpoint) =
            read("SECURITY_HEADERS_CSP_REPORT_TO")?.filter(|value| !value.is_empty())
        {
            policy = policy.report_to(Some(&endpoint)).map_err(|_| {
                FrameworkError::internal(
                    "SECURITY_HEADERS_CSP_REPORT_TO: expected an HTTP(S) endpoint URL",
                )
            })?;
        }
        for name in PUBLIC_DSN_SETTINGS {
            if let Some(dsn) = read(name)?.filter(|value| !value.is_empty()) {
                reporting_origin(&dsn).map_err(|_| invalid_public_dsn(name))?;
                policy.reporting_dsns.push(dsn);
            }
        }
        Ok(policy.hsts_preload(preload))
    }

    /// Changes or removes a header using validated HTTP names and values.
    ///
    /// CSP nonce sources must use `'nonce-{nonce}'`, so a fixed application
    /// value cannot accidentally reuse one nonce across requests.
    pub fn header(mut self, name: &str, value: Option<&str>) -> Result<Self, FrameworkError> {
        let name = validate_name(name)?;
        if let Some(value) = value {
            validate_value(&name, value)?;
        }
        match name.as_str() {
            "content-security-policy" => {
                self.csp = value.map(ContentSecurityPolicy::new).transpose()?;
            }
            "content-security-policy-report-only" => {
                self.csp_report_only = value.map(ContentSecurityPolicy::new).transpose()?;
            }
            "reporting-endpoints" if value.is_none() => {
                self.report_to = None;
                self.headers.remove(&name);
            }
            _ => {
                if let Some(value) = value {
                    self.headers.insert(name, value.into());
                } else {
                    self.headers.remove(&name);
                }
            }
        }
        Ok(self)
    }

    /// Replaces or disables the nonce policy before the handler renders its tags.
    pub fn content_security_policy(mut self, policy: Option<ContentSecurityPolicy>) -> Self {
        if self.report_only {
            self.csp_report_only = policy;
        } else {
            self.csp = policy;
        }
        self
    }

    /// Moves the selected policy between enforcing and report-only mode.
    /// Named header changes remain independent; this builder selects the mode
    /// used by [`Self::content_security_policy`] without duplicating a preset.
    pub fn report_only(mut self, enabled: bool) -> Self {
        if enabled != self.report_only {
            if enabled {
                self.csp_report_only = self.csp.take();
            } else {
                self.csp = self.csp_report_only.take();
            }
        }
        self.report_only = enabled;
        self
    }

    /// Names the reporting endpoint and rejects URLs unsafe for a response header.
    pub fn report_to(mut self, endpoint: Option<&str>) -> Result<Self, FrameworkError> {
        self.report_to = endpoint
            .map(|endpoint| {
                validate_value("Reporting-Endpoints", endpoint)?;
                let url = url::Url::parse(endpoint).map_err(|_| {
                    FrameworkError::internal(
                        "Reporting-Endpoints: expected an HTTP(S) endpoint URL",
                    )
                })?;
                if !matches!(url.scheme(), "http" | "https")
                    || url.host_str().is_none()
                    || !url.username().is_empty()
                    || url.password().is_some()
                    || endpoint.contains(['"', '\\'])
                {
                    return Err(FrameworkError::internal(
                        "Reporting-Endpoints: expected an HTTP(S) endpoint URL",
                    ));
                }
                Ok(endpoint.to_string())
            })
            .transpose()?;
        Ok(self)
    }

    /// Opts into the browser preload program without enabling HSTS on HTTP or local.
    pub fn hsts_preload(mut self, enabled: bool) -> Self {
        if let Some(value) = self.headers.get_mut("strict-transport-security") {
            *value = value
                .split(';')
                .map(str::trim)
                .filter(|part| !part.eq_ignore_ascii_case("preload"))
                .collect::<Vec<_>>()
                .join("; ");
            if enabled {
                value.push_str("; preload");
            }
        }
        self
    }

    /// Apply the same policy to server-built responses while preserving their
    /// body, status and existing headers. A surrounding scope supplies the
    /// effective request nonce when application middleware has already run.
    pub(crate) fn decorate_server_response<B>(
        &self,
        mut response: hyper::Response<B>,
        secure: bool,
    ) -> hyper::Response<B> {
        let metadata =
            HttpResponse::new().with_headers(response.headers().iter().filter_map(
                |(name, value)| value.to_str().ok().map(|value| (name.as_str(), value)),
            ));
        let local = Config::is_development();
        let origin = if local { vite_origin() } else { None };
        let decorated = self
            .decorate(
                metadata,
                secure,
                local,
                CspNonce::current().as_deref(),
                origin.as_deref(),
            )
            .into_hyper();
        for (name, value) in decorated.headers() {
            response
                .headers_mut()
                .entry(name.clone())
                .or_insert_with(|| value.clone());
        }
        response
    }

    fn uses_nonce(&self) -> bool {
        self.csp
            .iter()
            .chain(self.csp_report_only.iter())
            .any(ContentSecurityPolicy::uses_nonce)
    }

    fn decorate(
        &self,
        mut response: HttpResponse,
        secure: bool,
        local: bool,
        nonce: Option<&str>,
        origin: Option<&str>,
    ) -> HttpResponse {
        for (name, value) in &self.headers {
            if name == "strict-transport-security" && (!secure || local) {
                continue;
            }
            if response.header_value(name).is_none() {
                response = response.header(name, value);
            }
        }
        let public_dsns = if self.reporting_dsns.is_empty() {
            public_reporting_dsns()
        } else {
            self.reporting_dsns.clone()
        };
        for (name, csp) in [
            ("content-security-policy", self.csp.as_ref()),
            (
                "content-security-policy-report-only",
                self.csp_report_only.as_ref(),
            ),
        ] {
            let Some(csp) = csp else { continue };
            // Boot validates configured values. Direct preset builders also pick
            // up valid public DSNs, so their default policy cannot block reports.
            let mut csp = csp.clone();
            for dsn in &public_dsns {
                if let Ok(widened) = csp.clone().with_reporting_dsn(Some(dsn)) {
                    csp = widened;
                }
            }
            let html = response.header_value("content-type").is_some_and(|value| {
                value
                    .split(';')
                    .next()
                    .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("text/html"))
            });
            if !csp.uses_nonce() || html {
                if response.header_value(name).is_none() {
                    response =
                        response.header(name, csp.render(nonce, origin, self.report_to.is_some()));
                }
                if let Some(endpoint) = &self.report_to
                    && response.header_value("reporting-endpoints").is_none()
                {
                    response =
                        response.header("reporting-endpoints", format!("csp=\"{endpoint}\""));
                }
            }
        }
        response
    }
}

#[async_trait]
impl Middleware for SecurityHeaders {
    async fn handle(&self, mut request: Request, next: Next) -> Response {
        let secure = request.secure();
        let local = Config::is_development();
        let ambient = POLICY.try_with(Clone::clone);
        let server_decorates = ambient.is_ok();
        let effective = ambient.unwrap_or_else(|_| Arc::new(RwLock::new(self.clone())));
        *effective.write().unwrap_or_else(|p| p.into_inner()) = self.clone();
        let nonce = match make_nonce(self.uses_nonce()) {
            Ok(nonce) => nonce,
            Err(error) => return Err(error.into()),
        };
        let state = if server_decorates {
            CspNonce::state().unwrap_or_default()
        } else {
            NonceState::default()
        };
        *state.0.write().unwrap_or_else(|p| p.into_inner()) = nonce;
        request.set_csp_nonce(state.clone());
        POLICY
            .scope(
                effective.clone(),
                CspNonce::scope_state(state.clone(), async move {
                    let response = next(request).await;
                    // The server may still change HTML into an Inertia JSON
                    // error. Its boundary decorates once the body is final.
                    if server_decorates {
                        return response;
                    }
                    let policy = effective.read().unwrap_or_else(|p| p.into_inner()).clone();
                    let nonce = state.get();
                    let origin = if local { vite_origin() } else { None };
                    match response {
                        Ok(response) => Ok(policy.decorate(
                            response,
                            secure,
                            local,
                            nonce.as_deref(),
                            origin.as_deref(),
                        )),
                        Err(response) => Err(policy.decorate(
                            response,
                            secure,
                            local,
                            nonce.as_deref(),
                            origin.as_deref(),
                        )),
                    }
                }),
            )
            .await
    }
}

/// Route or group middleware that changes the outer policy before cache lookup.
///
/// Use ordinary `.middleware(...)` registration. The chain places these directly
/// inside `SecurityHeaders`, in group-to-route order; the innermost override wins.
#[derive(Clone, Debug, Default)]
pub struct SecurityHeadersOverride {
    headers: BTreeMap<String, Option<String>>,
    csp: Option<Option<ContentSecurityPolicy>>,
}

impl SecurityHeadersOverride {
    /// Starts with no changes to the inherited global policy.
    pub fn new() -> Self {
        Self::default()
    }

    /// Changes or removes one header without disturbing handler-owned headers.
    pub fn header(mut self, name: &str, value: Option<&str>) -> Result<Self, FrameworkError> {
        let name = validate_name(name)?;
        // Reuse global validation, including CSP placeholders.
        SecurityHeaders::off().header(&name, value)?;
        self.headers.insert(name, value.map(str::to_string));
        Ok(self)
    }

    /// Sets a route's nonce policy, or disables it before rendering the document.
    pub fn content_security_policy(mut self, policy: Option<ContentSecurityPolicy>) -> Self {
        self.csp = Some(policy);
        self
    }
}

#[async_trait]
impl Middleware for SecurityHeadersOverride {
    async fn handle(&self, request: Request, next: Next) -> Response {
        let Some(effective) = POLICY.try_with(Clone::clone).ok() else {
            return next(request).await;
        };
        let updated = {
            let mut policy = effective.read().unwrap_or_else(|p| p.into_inner()).clone();
            for (name, value) in &self.headers {
                policy = match policy.header(name, value.as_deref()) {
                    Ok(policy) => policy,
                    Err(error) => return Err(error.into()),
                };
            }
            if let Some(csp) = &self.csp {
                policy = policy.content_security_policy(csp.clone());
            }
            policy
        };
        if let Some(state) = request.csp_nonce_state() {
            let nonce = if updated.uses_nonce() {
                match state.get() {
                    Some(nonce) => Some(nonce),
                    None => match make_nonce(true) {
                        Ok(nonce) => nonce,
                        Err(error) => return Err(error.into()),
                    },
                }
            } else {
                None
            };
            *state.0.write().unwrap_or_else(|p| p.into_inner()) = nonce;
        }
        *effective.write().unwrap_or_else(|p| p.into_inner()) = updated;
        next(request).await
    }
}

/// Keeps the effective policy alive across the server's panic boundary and
/// final Inertia error callback, which run after application middleware returns.
pub(crate) async fn scope_response<F: std::future::Future<Output = HttpResponse>>(
    enabled: bool,
    secure: bool,
    future: F,
) -> HttpResponse {
    if !enabled {
        return future.await;
    }
    let effective = Arc::new(RwLock::new(SecurityHeaders::off()));
    let nonce = NonceState::default();
    POLICY
        .scope(
            effective.clone(),
            CspNonce::scope_state(nonce.clone(), async move {
                let response = future.await;
                let policy = effective.read().unwrap_or_else(|p| p.into_inner()).clone();
                let local = Config::is_development();
                let origin = if local { vite_origin() } else { None };
                policy.decorate(
                    response,
                    secure,
                    local,
                    nonce.get().as_deref(),
                    origin.as_deref(),
                )
            }),
        )
        .await
}

/// Keep policy changes alive across every upgrade fallback, including a panic
/// after route overrides and refusals before application middleware can run.
pub(crate) async fn scope_server_response<B, F>(
    policy: Option<SecurityHeaders>,
    secure: bool,
    future: F,
) -> hyper::Response<B>
where
    F: std::future::Future<Output = hyper::Response<B>>,
{
    let Some(policy) = policy else {
        return future.await;
    };
    let effective = Arc::new(RwLock::new(policy));
    POLICY
        .scope(
            effective.clone(),
            CspNonce::scope_state(NonceState::default(), async move {
                let response = future.await;
                let policy = effective.read().unwrap_or_else(|p| p.into_inner()).clone();
                policy.decorate_server_response(response, secure)
            }),
        )
        .await
}

fn make_nonce(enabled: bool) -> Result<Option<String>, FrameworkError> {
    enabled
        .then(|| {
            suprnova_live::render_cache::composite::fresh_nonce()
                .map_err(|_| FrameworkError::internal("could not generate CSP nonce"))
        })
        .transpose()
}

fn validate_name(name: &str) -> Result<String, FrameworkError> {
    hyper::header::HeaderName::from_bytes(name.as_bytes())
        .map(|name| name.as_str().to_string())
        .map_err(|_| FrameworkError::internal("invalid security header name"))
}

fn validate_value(name: &str, value: &str) -> Result<(), FrameworkError> {
    hyper::header::HeaderValue::from_str(value)
        .map(|_| ())
        .map_err(|_| FrameworkError::internal(format!("{name}: invalid security header value")))
}

fn read_bool(name: &str, value: Option<String>) -> Result<bool, FrameworkError> {
    match value.as_deref().unwrap_or("false") {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        _ => Err(FrameworkError::internal(format!(
            "{name}: expected true or false"
        ))),
    }
}

fn vite_origin() -> Option<String> {
    let config = crate::App::inertia_registry()
        .installed_config()
        .unwrap_or_default();
    let server = config.hot_file_url().unwrap_or(config.vite_dev_server);
    let url = url::Url::parse(&server).ok()?;
    matches!(url.scheme(), "http" | "https").then(|| url.origin().ascii_serialization())
}

/// The settings that name a public browser DSN: Live's runtime reporter reads
/// `REPORTING_PUBLIC_DSN`, the Inertia kits' builds read `VITE_REPORTING_DSN`
/// from the same `.env`. The policy allows the tracker origin of each, so
/// setting either one is enough for its reports to get through.
const PUBLIC_DSN_SETTINGS: [&str; 2] = ["REPORTING_PUBLIC_DSN", "VITE_REPORTING_DSN"];

/// The boot error for a public DSN setting that is not a public DSN. It names
/// the setting and never echoes the value, which may hold a credential.
fn invalid_public_dsn(name: &str) -> FrameworkError {
    FrameworkError::internal(format!(
        "{name}: expected a public HTTP(S) Sentry DSN without a password, query or fragment"
    ))
}

/// Rejects private credentials before a DSN can enter browser markup or headers.
pub(crate) fn reporting_origin(dsn: &str) -> Result<String, FrameworkError> {
    let invalid = || {
        FrameworkError::internal(
            "REPORTING_PUBLIC_DSN: expected a public HTTP(S) Sentry DSN without a password, query or fragment",
        )
    };
    if dsn
        .chars()
        .any(|character| character.is_control() || character.is_whitespace())
        || dsn.contains('\\')
    {
        return Err(invalid());
    }
    let url = url::Url::parse(dsn).map_err(|_| invalid())?;
    let project = url
        .path_segments()
        .and_then(|mut parts| parts.next_back())
        .unwrap_or_default();
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || url.username().is_empty()
        || !url
            .username()
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || project.is_empty()
        || !project.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(invalid());
    }
    Ok(url.origin().ascii_serialization())
}

/// Every valid public DSN the environment names, for a policy built without
/// the environment. The server DSN is deliberately never one of them.
fn public_reporting_dsns() -> Vec<String> {
    PUBLIC_DSN_SETTINGS
        .into_iter()
        .filter_map(crate::config::env_optional::<String>)
        .filter(|dsn| !dsn.is_empty() && reporting_origin(dsn).is_ok())
        .collect()
}

/// The server DSN is deliberately never a fallback for the browser's public key.
pub(crate) fn public_reporting_dsn() -> Option<String> {
    crate::config::env_optional::<String>("REPORTING_PUBLIC_DSN")
        .filter(|dsn| !dsn.is_empty() && reporting_origin(dsn).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sec_001_presets_and_every_environment_setting_fail_loudly_on_bad_values() {
        for preset in ["strict", "balanced", "relaxed", "off"] {
            assert!(
                SecurityHeaders::from_values(|key| Ok(
                    (key == "SECURITY_HEADERS").then(|| preset.to_string())
                ))
                .is_ok()
            );
        }
        for (variable, value) in [
            ("SECURITY_HEADERS", "typo"),
            ("SECURITY_HEADERS_CSP_REPORT_ONLY", "maybe"),
            ("SECURITY_HEADERS_HSTS_PRELOAD", "maybe"),
            ("SECURITY_HEADERS_CSP_REPORT_TO", "javascript:alert(1)"),
        ] {
            let error =
                SecurityHeaders::from_values(
                    |key| Ok((key == variable).then(|| value.to_string())),
                )
                .unwrap_err();
            assert!(error.to_string().contains(variable), "{error}");
        }
        let default = SecurityHeaders::from_values(|_| Ok(None)).unwrap();
        assert!(default.uses_nonce());
        assert_eq!(default.headers["x-frame-options"], "SAMEORIGIN");
        assert!(!default.headers["strict-transport-security"].contains("preload"));
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    async fn sec_003_standalone_policy_preserves_an_enclosing_nonce_scope() {
        let _container = crate::testing::TestContainer::fake();
        CspNonce::scope(Some("enclosing-nonce".to_string()), async {
            let client = crate::testing::TestClient::new(
                crate::Router::new().get("/", |request| async move {
                    let next: Next = Arc::new(|request| {
                        Box::pin(async move {
                            Ok(HttpResponse::html(request.csp_nonce().unwrap_or_default()))
                        })
                    });
                    SecurityHeaders::balanced().handle(request, next).await
                }),
                crate::MiddlewareRegistry::new(),
            );
            let response = client.get("/").send().await;
            assert_eq!(response.body_text().len(), 22);
            assert!(
                response
                    .header("content-security-policy")
                    .unwrap()
                    .contains(&response.body_text())
            );
            assert_eq!(CspNonce::current().as_deref(), Some("enclosing-nonce"));
        })
        .await;
        assert!(CspNonce::current().is_none());
    }

    #[test]
    fn sec_002_builders_reject_header_injection_and_invalid_nonce_templates() {
        assert!(
            SecurityHeaders::strict()
                .header("invalid\nname", Some("x"))
                .is_err()
        );
        assert!(
            SecurityHeaders::strict()
                .header("referrer-policy", Some("safe\r\ninjected: value"))
                .is_err()
        );
        for endpoint in [
            "javascript:alert(1)",
            "https://x/\"; injected",
            "https://user:password@x/",
            "not a URL",
        ] {
            assert!(SecurityHeaders::strict().report_to(Some(endpoint)).is_err());
        }
        for csp in [
            "script-src 'nonce-fixed'",
            "script-src {unknown}",
            "script-src '{nonce}'",
        ] {
            assert!(ContentSecurityPolicy::new(csp).is_err());
        }
    }
}

#[cfg(test)]
mod reporting_tests {
    use super::*;

    #[test]
    fn report_007_environment_policy_rejects_private_dsns_without_echoing_them() {
        let error = SecurityHeaders::from_values(|name| {
            Ok((name == "REPORTING_PUBLIC_DSN")
                .then(|| "https://public:private-secret@tracker.test/42".to_string()))
        })
        .unwrap_err();
        assert!(error.to_string().contains("REPORTING_PUBLIC_DSN"));
        assert!(!error.to_string().contains("private-secret"));
    }

    #[test]
    fn report_007_a_vite_only_public_dsn_opens_connect_src() {
        // An Inertia kit reports with the DSN its build read from
        // VITE_REPORTING_DSN; the server's policy must allow it even when
        // REPORTING_PUBLIC_DSN is unset.
        let policy = SecurityHeaders::from_values(|key| {
            Ok(match key {
                "SECURITY_HEADERS" => Some("balanced".to_string()),
                "VITE_REPORTING_DSN" => Some("https://public@vite-tracker.test/7".to_string()),
                _ => None,
            })
        })
        .unwrap();
        let response =
            policy.decorate(HttpResponse::html("page"), true, false, Some("nonce"), None);
        let header = response.header_value("content-security-policy").unwrap();
        assert!(
            header.contains("connect-src 'self' https://vite-tracker.test"),
            "{header}"
        );
        assert!(!header.contains("public@"), "{header}");
    }

    #[test]
    fn report_007_both_public_dsns_open_both_origins() {
        let policy = SecurityHeaders::from_values(|key| {
            Ok(match key {
                "SECURITY_HEADERS" => Some("strict".to_string()),
                "REPORTING_PUBLIC_DSN" => Some("https://public@live-tracker.test/1".to_string()),
                "VITE_REPORTING_DSN" => Some("https://public@vite-tracker.test/7".to_string()),
                _ => None,
            })
        })
        .unwrap();
        let response =
            policy.decorate(HttpResponse::html("page"), true, false, Some("nonce"), None);
        let header = response.header_value("content-security-policy").unwrap();
        assert!(header.contains("https://live-tracker.test"), "{header}");
        assert!(header.contains("https://vite-tracker.test"), "{header}");
    }

    #[test]
    fn report_007_a_private_vite_dsn_fails_naming_it_without_echoing_it() {
        let error = SecurityHeaders::from_values(|name| {
            Ok((name == "VITE_REPORTING_DSN")
                .then(|| "https://public:private-secret@tracker.test/42".to_string()))
        })
        .unwrap_err();
        assert!(error.to_string().contains("VITE_REPORTING_DSN"), "{error}");
        assert!(!error.to_string().contains("private-secret"));
    }

    #[test]
    fn report_007_every_csp_preset_permits_only_the_public_tracker_origin() {
        for preset in ["strict", "balanced", "relaxed", "off"] {
            let policy = SecurityHeaders::from_values(|key| {
                Ok(match key {
                    "SECURITY_HEADERS" => Some(preset.to_string()),
                    "REPORTING_PUBLIC_DSN" => {
                        Some("https://public@tracker.test/prefix/42".to_string())
                    }
                    _ => None,
                })
            })
            .unwrap();
            let response =
                policy.decorate(HttpResponse::html("page"), true, false, Some("nonce"), None);
            if matches!(preset, "strict" | "balanced") {
                let header = response.header_value("content-security-policy").unwrap();
                assert!(header.contains("connect-src 'self' https://tracker.test"));
                assert_eq!(header.matches("https://tracker.test").count(), 1);
                assert!(!header.contains("public@"));
            } else {
                assert!(response.header_value("content-security-policy").is_none());
            }
        }
    }
}
