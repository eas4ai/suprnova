//! The `live_key` checked filter.
//!
//! The checker accepts a `live:key` inside a `{% for %}` loop only when the
//! expression passes through this filter, because a loop key comes from
//! data the checker cannot see at compile time. The filter enforces at
//! render time exactly the rule the checker enforces on literal keys, so a
//! keyed loop item carries the same guarantee as a static one.

use std::error::Error;
use std::fmt;

/// The longest key the morph identity plan accepts, in bytes.
const MAX_KEY_BYTES: usize = 128;

/// Why a rendered loop key was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiveKeyErrorKind {
    /// The rendered key was empty.
    Empty,
    /// The rendered key exceeded 128 bytes.
    TooLong,
    /// The rendered key held a byte outside ASCII letters, digits, `_`, `-`, `.` and `:`.
    ForbiddenByte,
}

/// Rejection of a rendered loop key by the `live_key` filter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LiveKeyError {
    kind: LiveKeyErrorKind,
}

impl LiveKeyError {
    /// Returns the closed rejection class.
    #[must_use]
    pub const fn kind(self) -> LiveKeyErrorKind {
        self.kind
    }
}

impl fmt::Display for LiveKeyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            LiveKeyErrorKind::Empty => "empty_live_key",
            LiveKeyErrorKind::TooLong => "live_key_too_long",
            LiveKeyErrorKind::ForbiddenByte => "live_key_forbidden_byte",
        })
    }
}

impl Error for LiveKeyError {}

/// Validates one rendered key against the checker's key rule.
pub fn check_live_key(key: &str) -> Result<(), LiveKeyError> {
    let kind = if key.is_empty() {
        LiveKeyErrorKind::Empty
    } else if key.len() > MAX_KEY_BYTES {
        LiveKeyErrorKind::TooLong
    } else if key
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'))
    {
        return Ok(());
    } else {
        LiveKeyErrorKind::ForbiddenByte
    };
    Err(LiveKeyError { kind })
}

/// Checked Askama filters owned by the loop-key rule.
#[allow(
    missing_docs,
    reason = "Askama filter_fn generates public compatibility helpers"
)]
pub mod filters {
    use std::fmt;

    use askama::Values;

    /// Emits a loop key only when it satisfies the checker's key rule.
    #[askama::filter_fn]
    pub fn live_key(value: &dyn fmt::Display, _: &dyn Values) -> askama::Result<String> {
        let key = value.to_string();
        super::check_live_key(&key).map_err(askama::Error::custom)?;
        Ok(key)
    }
}
