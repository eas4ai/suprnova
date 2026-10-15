//! Validated CSP templates keep nonce substitution separate from application input.

use crate::FrameworkError;

/// A CSP template whose `'nonce-{nonce}'` sources use the current request's nonce.
///
/// Constructing the template validates it before requests arrive. Fixed policies
/// without a nonce remain useful for non-HTML responses as well.
#[derive(Clone, Debug)]
pub struct ContentSecurityPolicy {
    pub(crate) template: String,
}

impl ContentSecurityPolicy {
    /// Rejects invalid header bytes and unknown placeholders at configuration time.
    pub fn new(template: impl Into<String>) -> Result<Self, FrameworkError> {
        let template = template.into();
        super::validate_value("Content-Security-Policy", &template)?;
        let remainder = template.replace("'nonce-{nonce}'", "");
        if remainder.contains(['{', '}']) || remainder.to_ascii_lowercase().contains("'nonce-") {
            return Err(FrameworkError::internal(
                "Content-Security-Policy: nonce sources must use 'nonce-{nonce}'",
            ));
        }
        Ok(Self { template })
    }

    /// Uses nonces for scripts and styles, and disallows framing and plugins.
    pub fn strict() -> Self {
        Self { template: "default-src 'self'; script-src 'nonce-{nonce}' 'strict-dynamic'; style-src 'self' 'nonce-{nonce}'; img-src 'self' data:; font-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'".into() }
    }

    /// Runs the application's own script files and nonced scripts, and keeps
    /// component style attributes. No `'strict-dynamic'`: it would switch off
    /// `'self'`, and the Live components load their scripts by plain tags from
    /// the application's origin.
    pub fn balanced() -> Self {
        Self { template: "default-src 'self'; script-src 'self' 'nonce-{nonce}'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob: https:; font-src 'self' data: https:; connect-src 'self'; object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'self'".into() }
    }

    /// Keeps the browser reporter reachable without granting script access to its host.
    pub fn with_reporting_dsn(mut self, dsn: Option<&str>) -> Result<Self, FrameworkError> {
        let Some(dsn) = dsn.filter(|value| !value.is_empty()) else {
            return Ok(self);
        };
        let origin = super::reporting_origin(dsn)?;
        let mut found = false;
        let mut fallback = "*".to_string();
        let mut directives = Vec::new();
        for directive in self.template.split(';').map(str::trim) {
            let mut words = directive.split_whitespace();
            let name = words.next().unwrap_or_default();
            if name.eq_ignore_ascii_case("default-src") {
                fallback = words
                    .clone()
                    .filter(|word| *word != "'none'")
                    .collect::<Vec<_>>()
                    .join(" ");
            }
            if name.eq_ignore_ascii_case("connect-src") && !found {
                found = true;
                let mut sources = words
                    .filter(|word| *word != "'none'")
                    .map(str::to_string)
                    .collect::<Vec<_>>();
                if !sources.contains(&origin) {
                    sources.push(origin.clone());
                }
                directives.push(format!("connect-src {}", sources.join(" ")));
            } else {
                directives.push(directive.to_string());
            }
        }
        if !found {
            directives.push(format!("connect-src {fallback} {origin}"));
        }
        self.template = directives.join("; ");
        Ok(self)
    }

    pub(crate) fn uses_nonce(&self) -> bool {
        self.template.contains("'nonce-{nonce}'")
    }

    pub(crate) fn render(
        &self,
        nonce: Option<&str>,
        dev_origin: Option<&str>,
        reporting: bool,
    ) -> String {
        let mut policy = self.template.replace("{nonce}", nonce.unwrap_or_default());
        if let Some(origin) = dev_origin {
            let websocket = origin
                .replacen("https://", "wss://", 1)
                .replacen("http://", "ws://", 1);
            policy = policy
                .split(';')
                .map(|directive| {
                    let directive = directive.trim();
                    if directive.starts_with("script-src ") {
                        format!("{directive} {origin}")
                    } else if directive.starts_with("connect-src ") {
                        format!("{directive} {origin} {websocket}")
                    } else {
                        directive.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join("; ");
        }
        if reporting {
            policy.push_str("; report-to csp");
        }
        policy
    }
}

#[cfg(test)]
mod reporting_tests {
    use super::*;

    #[test]
    fn report_007_reporting_origin_is_added_only_to_connect_src() {
        for policy in [
            ContentSecurityPolicy::strict(),
            ContentSecurityPolicy::balanced(),
            ContentSecurityPolicy::new("default-src 'self'").unwrap(),
        ] {
            let rendered = policy
                .with_reporting_dsn(Some("https://public@tracker.example.test:9443/prefix/42"))
                .unwrap()
                .render(Some("nonce"), None, false);
            let connect = rendered
                .split(';')
                .find(|part| part.trim().starts_with("connect-src "))
                .unwrap();
            assert!(connect.contains("https://tracker.example.test:9443"));
            assert!(!rendered.contains("public@"));
            assert!(!rendered.contains("/prefix/42"));
            assert_eq!(
                rendered
                    .matches("https://tracker.example.test:9443")
                    .count(),
                1
            );
        }
    }

    #[test]
    fn report_007_invalid_public_dsns_fail_without_revealing_the_value() {
        for dsn in [
            "javascript:secret",
            "https://public:secret@tracker.test/42",
            "https://tracker.test/42",
            "https://public@tracker.test/42?token=secret",
            "https://public@tracker.test/",
        ] {
            let error = ContentSecurityPolicy::balanced()
                .with_reporting_dsn(Some(dsn))
                .unwrap_err();
            assert!(error.to_string().contains("REPORTING_PUBLIC_DSN"));
            assert!(!error.to_string().contains("secret"));
        }
        let policy = ContentSecurityPolicy::balanced();
        assert_eq!(
            policy.clone().with_reporting_dsn(None).unwrap().template,
            policy.template
        );
    }
}
