//! Trusted-host gating: refuse a request for a host the application does
//! not serve. The middleware and its rules are documented on
//! [`TrustHosts`], the public item.

use async_trait::async_trait;

use crate::config::{AppConfig, Config, Environment};
use crate::error::FrameworkError;
use crate::http::{HttpResponse, Request, Response};
use crate::middleware::{Middleware, Next};

/// Middleware that answers `400 Bad request.` to a request whose host the
/// application does not serve. Mirrors Laravel's `TrustHosts`.
///
/// The `Host` header is written by the client. An application that builds
/// an absolute URL from it - a password reset link, a redirect, a cache
/// key - hands a forged host back to whoever sent it: a reset mail that
/// links to the attacker's site, or a cached page poisoned for every
/// visitor. [`Request::host`] already refuses a host with characters no
/// host has; this middleware also refuses a well-formed host the
/// application does not answer to.
///
/// # Configuration
///
/// [`TrustHosts::new`] trusts the host of `APP_URL` and its subdomains,
/// Laravel's default. [`TrustHosts::at`] lists the hosts you trust as
/// regular expressions, and adds the `APP_URL` host and its subdomains
/// when its second argument is `true`:
///
/// ```rust,no_run
/// use suprnova::{TrustHosts, global_middleware};
///
/// # fn ex() -> Result<(), suprnova::FrameworkError> {
/// global_middleware!(TrustHosts::at([r"^example\.com$", r"^(.+\.)?example\.org$"], false)?);
/// # Ok(())
/// # }
/// ```
///
/// A pattern is matched without regard to case, and anywhere in the host
/// unless it is anchored with `^` and `$`, as Symfony's `setTrustedHosts`
/// matches one. A request whose host is invalid, or matches no pattern,
/// is answered `400` with the message `Bad request.`, the answer Laravel
/// gives an untrusted host. With no pattern at all - [`TrustHosts::at`]
/// with an empty list and `false`, or an `APP_URL` without a host - every
/// valid host passes, as it does in Laravel.
///
/// The middleware reads the application configuration on each request,
/// as Laravel's reads `app.url`: the [`AppConfig`] the application
/// registered, or the `APP_URL` and `APP_ENV` environment variables
/// without one.
///
/// # Environments and proxies
///
/// In the local environment (`APP_ENV=local`, the default when `APP_ENV`
/// is unset) the middleware trusts every host, as Laravel's does, so a
/// development server answers on `localhost`, a LAN address and a tunnel
/// alike. Set `APP_ENV` everywhere else.
///
/// Behind a terminating proxy the host comes from `X-Forwarded-Host` when
/// the proxy is trusted (see
/// [`TrustedProxiesConfig`](crate::http::TrustedProxiesConfig)), so the
/// patterns name the public host the browser asked for.
#[derive(Clone, Debug)]
pub struct TrustHosts {
    /// The caller's patterns, compiled once, each matched without regard
    /// to case.
    patterns: Vec<regex::Regex>,
    /// Whether the `APP_URL` host and its subdomains are trusted too.
    application_url: bool,
}

impl TrustHosts {
    /// Trust the `APP_URL` host and its subdomains, and nothing else.
    ///
    /// Laravel's default: with `APP_URL=https://example.com`, a request
    /// for `example.com` or `api.example.com` passes and one for
    /// `evil.test` is answered `400`.
    pub fn new() -> Self {
        Self {
            patterns: Vec::new(),
            application_url: true,
        }
    }

    /// Trust the hosts `patterns` match, and the `APP_URL` host and its
    /// subdomains as well when `subdomains` is `true`. Mirrors Laravel's
    /// `TrustHosts::at($hosts, $subdomains)`.
    ///
    /// Each pattern is a regular expression matched against the request
    /// host without regard to case. Anchor it with `^` and `$` to match the
    /// whole host: `^example\.com$` trusts `example.com` and nothing else,
    /// while `example\.com` also trusts `example.com.evil.test`.
    ///
    /// # Errors
    ///
    /// When a pattern is not a valid regular expression. The error names
    /// the pattern, so a typing error fails at boot instead of refusing
    /// every request.
    pub fn at<I, S>(patterns: I, subdomains: bool) -> Result<Self, FrameworkError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let patterns = patterns
            .into_iter()
            .map(|pattern| {
                let pattern = pattern.as_ref();
                regex::RegexBuilder::new(pattern)
                    .case_insensitive(true)
                    .build()
                    .map_err(|error| {
                        FrameworkError::internal(format!(
                            "TrustHosts pattern `{pattern}` is not a valid regular expression: \
                             {error}"
                        ))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            patterns,
            application_url: subdomains,
        })
    }

    /// Whether `host`, already lowercased and checked by
    /// [`Request::host`], is trusted when the `APP_URL` host is
    /// `application_host`.
    ///
    /// With no pattern at all, every host is trusted: Laravel hands
    /// Symfony an empty list then, and Symfony checks nothing.
    fn trusts(&self, host: &str, application_host: Option<&str>) -> bool {
        let application_host = application_host.filter(|_| self.application_url);
        if self.patterns.is_empty() && application_host.is_none() {
            return true;
        }
        application_host.is_some_and(|application| is_host_or_subdomain(host, application))
            || self.patterns.iter().any(|pattern| pattern.is_match(host))
    }
}

impl Default for TrustHosts {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Middleware for TrustHosts {
    async fn handle(&self, request: Request, next: Next) -> Response {
        let config = Config::get::<AppConfig>();
        let environment = config
            .as_ref()
            .map(|config| config.environment.clone())
            .unwrap_or_else(Environment::detect);
        // Laravel's `shouldSpecifyTrustedHosts`: the local environment
        // answers on every host a developer reaches it by.
        if environment == Environment::Local {
            return next(request).await;
        }
        let Some(host) = request.host() else {
            return bad_request();
        };
        let application_host = application_host(&crate::routing::url::app_url_of(config.as_ref()));
        if self.trusts(&host, application_host.as_deref()) {
            next(request).await
        } else {
            bad_request()
        }
    }
}

/// The host of `APP_URL`, lowercased, or `None` when the URL has none.
fn application_host(app_url: &str) -> Option<String> {
    url::Url::parse(app_url)
        .ok()?
        .host_str()
        .filter(|host| !host.is_empty())
        .map(str::to_ascii_lowercase)
}

/// Whether `host` is `application` or one of its subdomains: Laravel's
/// pattern `^(.+\.)?<host>$`, compared without a regular expression
/// because both sides are lowercase hosts.
fn is_host_or_subdomain(host: &str, application: &str) -> bool {
    host == application
        || host
            .strip_suffix(application)
            .and_then(|rest| rest.strip_suffix('.'))
            .is_some_and(|label| !label.is_empty())
}

/// The answer to an untrusted or invalid host: `400` with Laravel's
/// `Bad request.`, through the framework's error rendering so the body
/// and the error report follow every other refusal.
fn bad_request() -> Response {
    Err(HttpResponse::from(FrameworkError::domain(
        "Bad request.",
        400,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_application_host_and_its_subdomains_are_trusted() {
        let trust = TrustHosts::new();
        assert!(trust.trusts("example.com", Some("example.com")));
        assert!(trust.trusts("api.example.com", Some("example.com")));
        assert!(trust.trusts("a.b.example.com", Some("example.com")));
        assert!(!trust.trusts("evil.test", Some("example.com")));
        assert!(!trust.trusts("notexample.com", Some("example.com")));
        assert!(!trust.trusts(".example.com", Some("example.com")));
        assert!(!trust.trusts("example.com.evil.test", Some("example.com")));
    }

    #[test]
    fn patterns_match_without_regard_to_case_and_subdomains_add_the_application_host() {
        let trust = TrustHosts::at([r"^Example\.COM$"], false).expect("valid pattern");
        assert!(trust.trusts("example.com", Some("app.test")));
        assert!(!trust.trusts("api.example.com", Some("app.test")));
        assert!(
            !trust.trusts("app.test", Some("app.test")),
            "without subdomains the application host is not added"
        );

        let trust = TrustHosts::at([r"^example\.com$"], true).expect("valid pattern");
        assert!(trust.trusts("example.com", Some("app.test")));
        assert!(trust.trusts("www.app.test", Some("app.test")));
        assert!(!trust.trusts("evil.test", Some("app.test")));
    }

    #[test]
    fn with_no_pattern_at_all_every_host_is_trusted() {
        let trust = TrustHosts::at(Vec::<&str>::new(), false).expect("no pattern");
        assert!(trust.trusts("anything.test", Some("example.com")));
        assert!(TrustHosts::new().trusts("anything.test", None));
    }

    #[test]
    fn an_invalid_pattern_is_an_error_that_names_it() {
        let error = TrustHosts::at(["(unclosed"], false).expect_err("invalid pattern");
        assert!(error.to_string().contains("(unclosed"), "{error}");
    }

    #[test]
    fn the_application_host_comes_from_the_url() {
        assert_eq!(
            application_host("https://Example.com/billing").as_deref(),
            Some("example.com")
        );
        assert_eq!(
            application_host("http://[::1]:8080").as_deref(),
            Some("[::1]")
        );
        assert_eq!(application_host("not a url"), None);
    }
}
