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
