//! Explicit audited boundary for deliberately unescaped HTML.

use std::error::Error;
use std::fmt;

use askama::Values;
use askama::filters::Safe;

const MAX_REASON_BYTES: usize = 160;
const MAX_SANITIZER_ID_BYTES: usize = 64;
const HARD_MAX_TRUSTED_HTML_BYTES: usize = 2 * 1024 * 1024;

/// Why trusted markup is required at one source-visible construction site.
#[derive(Clone, Eq, PartialEq)]
pub struct TrustedMarkupReason(String);

impl TrustedMarkupReason {
    /// Parses a nonempty bounded printable reason.
    pub fn new(reason: &str) -> Result<Self, TrustedMarkupError> {
        let valid = !reason.trim().is_empty()
            && reason.len() <= MAX_REASON_BYTES
            && reason
                .chars()
                .all(|character| !character.is_control() || character == '\n' || character == '\t');
        if !valid {
            return Err(TrustedMarkupError::new(
                TrustedMarkupErrorKind::InvalidReason,
            ));
        }
        Ok(Self(reason.to_owned()))
    }

    /// Returns the audited source-visible reason.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for TrustedMarkupReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<TrustedMarkupReason>")
    }
}

/// Stable registered sanitizer identity.
#[derive(Clone, Eq, PartialEq)]
pub struct SanitizerId(String);

impl SanitizerId {
    /// Parses a bounded ASCII sanitizer identity.
    pub fn parse(value: &str) -> Result<Self, TrustedMarkupError> {
        let valid = !value.is_empty()
            && value.len() <= MAX_SANITIZER_ID_BYTES
            && value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-')
            });
        if !valid {
            return Err(TrustedMarkupError::new(
                TrustedMarkupErrorKind::InvalidSanitizer,
            ));
        }
        Ok(Self(value.to_owned()))
    }
}

impl fmt::Debug for SanitizerId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<SanitizerId>")
    }
}

/// Opaque failure returned by an application sanitizer implementation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SanitizerFailure;

/// Registered sanitizer function that is the only owned-string trust path.
pub struct RegisteredSanitizer {
    id: SanitizerId,
    sanitize: fn(&str, &mut dyn fmt::Write) -> Result<(), SanitizerFailure>,
}

impl RegisteredSanitizer {
    /// Registers an auditable sanitizer implementation under a stable identity.
    #[must_use]
    pub const fn new(
        id: SanitizerId,
        sanitize: fn(&str, &mut dyn fmt::Write) -> Result<(), SanitizerFailure>,
    ) -> Self {
        Self { id, sanitize }
    }

    /// Sanitizes untrusted text and attaches a private registered-sanitizer proof.
    pub fn sanitize(
        &self,
        untrusted: &str,
        reason: TrustedMarkupReason,
        max_output_bytes: usize,
    ) -> Result<TrustedHtml, TrustedMarkupError> {
        if max_output_bytes == 0
            || max_output_bytes > HARD_MAX_TRUSTED_HTML_BYTES
            || untrusted.len() > HARD_MAX_TRUSTED_HTML_BYTES
        {
            return Err(TrustedMarkupError::new(
                TrustedMarkupErrorKind::MarkupTooLarge,
            ));
        }
        let mut output = BoundedTrustedMarkup::new(max_output_bytes);
        let result = (self.sanitize)(untrusted, &mut output);
        if output.overflowed {
            return Err(TrustedMarkupError::new(
                TrustedMarkupErrorKind::MarkupTooLarge,
            ));
        }
        result.map_err(|_| TrustedMarkupError::new(TrustedMarkupErrorKind::SanitizationFailed))?;
        Ok(TrustedHtml {
            html: output.html,
            reason,
            provenance: TrustedMarkupProvenance::RegisteredSanitizer(self.id.clone()),
        })
    }
}

struct BoundedTrustedMarkup {
    html: String,
    max_bytes: usize,
    overflowed: bool,
}

impl BoundedTrustedMarkup {
    fn new(max_bytes: usize) -> Self {
        Self {
            html: String::new(),
            max_bytes,
            overflowed: false,
        }
    }
}

impl fmt::Write for BoundedTrustedMarkup {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        if self.html.len().saturating_add(value.len()) > self.max_bytes {
            self.overflowed = true;
            return Err(fmt::Error);
        }
        self.html.push_str(value);
        Ok(())
    }
}

impl fmt::Debug for RegisteredSanitizer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<RegisteredSanitizer>")
    }
}

#[derive(Clone)]
enum TrustedMarkupProvenance {
    FrameworkStatic,
    FrameworkGenerated,
    EngineValidatedIsland,
    RegisteredSanitizer(SanitizerId),
}

/// Markup that crossed one explicit framework-owned trust path.
///
/// Ordinary strings have no convenience conversion into this type:
///
/// ```compile_fail
/// # use suprnova_live as suprnova;
/// use suprnova::view::TrustedHtml;
/// let untrusted = String::from("<strong>untrusted</strong>");
/// let _: TrustedHtml = untrusted.into();
/// ```
pub struct TrustedHtml {
    html: String,
    reason: TrustedMarkupReason,
    provenance: TrustedMarkupProvenance,
}

impl TrustedHtml {
    /// Trusts bounded compile-time framework markup with an explicit reason.
    pub fn framework_static(
        html: &'static str,
        reason: TrustedMarkupReason,
    ) -> Result<Self, TrustedMarkupError> {
        if html.len() > HARD_MAX_TRUSTED_HTML_BYTES {
            return Err(TrustedMarkupError::new(
                TrustedMarkupErrorKind::MarkupTooLarge,
            ));
        }
        Ok(Self {
            html: html.to_owned(),
            reason,
            provenance: TrustedMarkupProvenance::FrameworkStatic,
        })
    }

    /// Trusts bounded markup a framework host generated from its own typed facts.
    ///
    /// This is the same trust level as [`Self::framework_static`]: the caller
    /// is framework code assembling markup from validated identities,
    /// digests, and URLs, never from request or application data.
    pub fn framework_generated(
        html: String,
        reason: TrustedMarkupReason,
    ) -> Result<Self, TrustedMarkupError> {
        if html.len() > HARD_MAX_TRUSTED_HTML_BYTES {
            return Err(TrustedMarkupError::new(
                TrustedMarkupErrorKind::MarkupTooLarge,
            ));
        }
        Ok(Self {
            html,
            reason,
            provenance: TrustedMarkupProvenance::FrameworkGenerated,
        })
    }

    /// Returns the audited construction reason without exposing it through Debug.
    #[must_use]
    pub const fn reason(&self) -> &TrustedMarkupReason {
        &self.reason
    }

    pub(crate) fn engine_validated_island(html: String) -> Self {
        Self {
            html,
            reason: TrustedMarkupReason("engine-validated Live island".to_owned()),
            provenance: TrustedMarkupProvenance::EngineValidatedIsland,
        }
    }
}

impl fmt::Display for TrustedHtml {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.html)
    }
}

impl fmt::Debug for TrustedHtml {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.provenance {
            TrustedMarkupProvenance::FrameworkStatic => {
                formatter.write_str("<TrustedHtml:framework-static>")
            }
            TrustedMarkupProvenance::FrameworkGenerated => {
                formatter.write_str("<TrustedHtml:framework-generated>")
            }
            TrustedMarkupProvenance::EngineValidatedIsland => {
                formatter.write_str("<TrustedHtml:engine-validated-island>")
            }
            TrustedMarkupProvenance::RegisteredSanitizer(id) => {
                let _ = id;
                formatter.write_str("<TrustedHtml:registered-sanitizer>")
            }
        }
    }
}

/// Checked Askama filters owned by the Suprnova view boundary.
#[allow(
    missing_docs,
    reason = "Askama filter_fn generates public compatibility helpers"
)]
pub mod filters {
    use super::{Safe, TrustedHtml, Values};

    /// Emits only a [`TrustedHtml`] value without Askama's ordinary escaping.
    #[askama::filter_fn]
    pub fn trusted_html<'a>(
        value: &'a TrustedHtml,
        _: &dyn Values,
    ) -> askama::Result<Safe<&'a TrustedHtml>> {
        Ok(Safe(value))
    }
}

/// Closed trusted-markup construction failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrustedMarkupErrorKind {
    /// The reason was empty, too large, or contained forbidden control text.
    InvalidReason,
    /// The sanitizer identity was empty, too large, or malformed.
    InvalidSanitizer,
    /// Input, output, or a configured output bound exceeded the hard ceiling.
    MarkupTooLarge,
    /// The registered sanitizer rejected its input.
    SanitizationFailed,
}

/// Redacted error from a trusted-markup construction path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TrustedMarkupError {
    kind: TrustedMarkupErrorKind,
}

impl TrustedMarkupError {
    const fn new(kind: TrustedMarkupErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the closed construction failure class.
    #[must_use]
    pub const fn kind(self) -> TrustedMarkupErrorKind {
        self.kind
    }
}

impl fmt::Display for TrustedMarkupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self.kind {
            TrustedMarkupErrorKind::InvalidReason => "invalid_trusted_markup_reason",
            TrustedMarkupErrorKind::InvalidSanitizer => "invalid_sanitizer_identity",
            TrustedMarkupErrorKind::MarkupTooLarge => "trusted_markup_too_large",
            TrustedMarkupErrorKind::SanitizationFailed => "registered_sanitizer_failed",
        };
        formatter.write_str(message)
    }
}

impl Error for TrustedMarkupError {}
