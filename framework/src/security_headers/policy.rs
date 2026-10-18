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
    pub fn with_reporting_dsn(self, dsn: Option<&str>) -> Result<Self, FrameworkError> {
        let Some(dsn) = dsn.filter(|value| !value.is_empty()) else {
            return Ok(self);
        };
        let origin = super::reporting_origin(dsn)?;
        self.with_source("connect-src", &origin)
    }

    /// Adds `source` to `directive`, keeping every source the policy already
    /// allows there, so an application can admit a third-party origin
    /// without copying a preset:
    ///
    /// ```rust
    /// use suprnova::{ContentSecurityPolicy, FrameworkError};
    ///
    /// fn policy() -> Result<ContentSecurityPolicy, FrameworkError> {
    ///     ContentSecurityPolicy::balanced().with_source("connect-src", "https://plausible.io")
    /// }
    /// ```
    ///
    /// A directive the policy leaves out starts from the sources the browser
    /// falls back to for it (`frame-src` from `child-src`, then
    /// `default-src`), so the new source narrows nothing. A directive with
    /// no fallback in the policy already allows every source and is left as
    /// it is. `'none'` is dropped from a directive that gains a source.
    ///
    /// # Errors
    ///
    /// Returns an error when `directive` is not a directive that takes a
    /// source list, or `source` is not one source expression: empty,
    /// holding whitespace, `;` or `,`, `'none'`, or a nonce other than
    /// `'nonce-{nonce}'`.
    pub fn with_source(self, directive: &str, source: &str) -> Result<Self, FrameworkError> {
        let (directive, fallbacks) = source_directive(directive, source)?;
        let mut directives = self
            .template
            .split(';')
            .map(str::trim)
            .filter(|directive| !directive.is_empty())
            .collect::<Vec<_>>();
        // A browser applies the first occurrence of a directive and ignores
        // later ones, so the first is the one to widen or to start from.
        let first = |name: &str| {
            directives.iter().position(|candidate| {
                candidate
                    .split_whitespace()
                    .next()
                    .is_some_and(|word| word.eq_ignore_ascii_case(name))
            })
        };
        let (at, inherited) = match first(directive) {
            Some(at) => (Some(at), directives[at]),
            None => match fallbacks.iter().find_map(|name| first(name)) {
                Some(from) => (None, directives[from]),
                None => return Ok(self),
            },
        };
        let mut sources = inherited
            .split_whitespace()
            .skip(1)
            .filter(|word| !word.eq_ignore_ascii_case("'none'"))
            .collect::<Vec<_>>();
        if !sources.contains(&source) {
            sources.push(source);
        }
        let widened = std::iter::once(directive)
            .chain(sources)
            .collect::<Vec<_>>()
            .join(" ");
        match at {
            Some(at) => directives[at] = &widened,
            None => directives.push(&widened),
        }
        Self::new(directives.join("; "))
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

/// Checks that `source` is one source expression for `directive`.
pub(super) fn check_source(directive: &str, source: &str) -> Result<(), FrameworkError> {
    source_directive(directive, source).map(|_| ())
}

/// Checks that `source` is one source expression for `directive`, and
/// answers the directive's lower-case name with the directives a browser
/// falls back to when the policy leaves it out, nearest first (CSP Level 3,
/// "Get the effective directive" and its fallback list).
fn source_directive(
    directive: &str,
    source: &str,
) -> Result<(&'static str, &'static [&'static str]), FrameworkError> {
    let (name, fallbacks): (&'static str, &'static [&'static str]) =
        match directive.to_ascii_lowercase().as_str() {
            "default-src" => ("default-src", &[]),
            "script-src" => ("script-src", &["default-src"]),
            "script-src-elem" => ("script-src-elem", &["script-src", "default-src"]),
            "script-src-attr" => ("script-src-attr", &["script-src", "default-src"]),
            "style-src" => ("style-src", &["default-src"]),
            "style-src-elem" => ("style-src-elem", &["style-src", "default-src"]),
            "style-src-attr" => ("style-src-attr", &["style-src", "default-src"]),
            "img-src" => ("img-src", &["default-src"]),
            "font-src" => ("font-src", &["default-src"]),
            "connect-src" => ("connect-src", &["default-src"]),
            "media-src" => ("media-src", &["default-src"]),
            "object-src" => ("object-src", &["default-src"]),
            "manifest-src" => ("manifest-src", &["default-src"]),
            "child-src" => ("child-src", &["default-src"]),
            "frame-src" => ("frame-src", &["child-src", "default-src"]),
            "worker-src" => ("worker-src", &["child-src", "script-src", "default-src"]),
            "form-action" => ("form-action", &[]),
            "frame-ancestors" => ("frame-ancestors", &[]),
            "base-uri" => ("base-uri", &[]),
            _ => {
                return Err(FrameworkError::internal(format!(
                    "Content-Security-Policy: `{directive}` is not a directive that takes sources"
                )));
            }
        };
    let lower = source.to_ascii_lowercase();
    if source.is_empty()
        || !source.is_ascii()
        || source.contains(|c: char| c.is_whitespace() || c.is_control() || c == ';' || c == ',')
        || lower == "'none'"
        || (lower.starts_with("'nonce-") && source != "'nonce-{nonce}'")
    {
        return Err(FrameworkError::internal(format!(
            "Content-Security-Policy: `{name}` takes one source expression at a time, \
             not 'none' or a fixed nonce"
        )));
    }
    Ok((name, fallbacks))
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

#[cfg(test)]
mod source_tests {
    use super::*;

    fn widened(template: &str, directive: &str, source: &str) -> String {
        ContentSecurityPolicy::new(template)
            .unwrap()
            .with_source(directive, source)
            .unwrap()
            .template
    }

    #[test]
    fn an_absent_directive_starts_from_its_nearest_fallback() {
        assert_eq!(
            widened(
                "default-src 'none'; script-src 'self' 'nonce-{nonce}'",
                "worker-src",
                "https://cdn.example"
            ),
            "default-src 'none'; script-src 'self' 'nonce-{nonce}'; worker-src 'self' 'nonce-{nonce}' https://cdn.example"
        );
        assert_eq!(
            widened("default-src 'none'", "connect-src", "https://a.example"),
            "default-src 'none'; connect-src https://a.example"
        );
    }

    #[test]
    fn a_directive_with_no_fallback_in_the_policy_stays_unrestricted() {
        for (template, directive) in [
            ("script-src 'self'", "connect-src"),
            ("default-src 'self'", "form-action"),
        ] {
            assert_eq!(widened(template, directive, "https://a.example"), template);
        }
    }

    #[test]
    fn the_first_occurrence_gains_the_source_once_and_loses_none() {
        assert_eq!(
            widened(
                "Connect-Src 'none'; connect-src https://ignored.example",
                "CONNECT-SRC",
                "https://a.example"
            ),
            "connect-src https://a.example; connect-src https://ignored.example"
        );
        let template = "default-src 'self'; connect-src 'self' https://a.example";
        assert_eq!(
            widened(template, "connect-src", "https://a.example"),
            template
        );
    }
}
