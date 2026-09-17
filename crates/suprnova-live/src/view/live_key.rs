//! The `live_key` and `live_key_digest` checked filters.
//!
//! The checker accepts a `live:key` inside a `{% for %}` loop only when the
//! expression passes through one of these filters, because a loop key comes
//! from data the checker cannot see at compile time. `live_key` enforces at
//! render time exactly the rule the checker enforces on literal keys, so a
//! keyed loop item carries the same guarantee as a static one, and a value
//! outside the rule fails the island's render. `live_key_digest` turns any
//! value into a key inside the rule, so it never fails.

use std::error::Error;
use std::fmt;

/// The longest key the morph identity plan accepts, in bytes.
pub(crate) const MAX_KEY_BYTES: usize = 128;

/// Why a rendered loop key was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiveKeyErrorKind {
    /// The rendered key was empty.
    Empty,
    /// The rendered key exceeded 128 bytes.
    TooLong,
    /// The rendered key held a byte outside ASCII letters, digits, `_`, `-`,
    /// `.` and `:`, or began with a byte other than a letter or a digit.
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

/// Whether a key is spelled in the stable-key alphabet the browser runtime
/// accepts (`SAFE_KEY` in `browser/src/morph/keys.ts`): an ASCII letter or
/// digit, then ASCII letters, digits, `_`, `-`, `.` and `:`. The checker,
/// this filter, and the runtime share this one alphabet (LIVE-033); the
/// length bound is checked beside it.
pub(crate) fn in_key_alphabet(key: &str) -> bool {
    key.bytes()
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'))
}

/// Validates one rendered key against the checker's key rule.
pub fn check_live_key(key: &str) -> Result<(), LiveKeyError> {
    let kind = if key.is_empty() {
        LiveKeyErrorKind::Empty
    } else if key.len() > MAX_KEY_BYTES {
        LiveKeyErrorKind::TooLong
    } else if in_key_alphabet(key) {
        return Ok(());
    } else {
        LiveKeyErrorKind::ForbiddenByte
    };
    Err(LiveKeyError { kind })
}

/// Derives a stable key from any value: `k` followed by the first 32
/// lowercase hex digits of the SHA-256 of the value's text. One value always
/// yields the same key, and every key is in the alphabet and 33 bytes long,
/// so a loop keyed by data the application does not shape (an email address,
/// a display name, a URL) renders for every value (LIVE-035). Two values
/// share a key only on a 128-bit digest collision.
#[must_use]
pub fn live_key_digest(value: &str) -> String {
    use sha2::{Digest as _, Sha256};

    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(value.as_bytes());
    let mut key = String::with_capacity(33);
    key.push('k');
    for byte in digest.iter().take(16) {
        key.push(char::from(HEX[usize::from(byte >> 4)]));
        key.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    key
}

/// Checked Askama filters owned by the loop-key rule.
#[allow(
    missing_docs,
    reason = "Askama filter_fn generates public compatibility helpers"
)]
pub mod filters {
    use std::fmt;

    use askama::Values;

    /// Emits a loop key only when it satisfies the checker's key rule; any
    /// other value fails the render of the island that holds it.
    #[askama::filter_fn]
    pub fn live_key(value: &dyn fmt::Display, _: &dyn Values) -> askama::Result<String> {
        let key = value.to_string();
        super::check_live_key(&key).map_err(askama::Error::custom)?;
        Ok(key)
    }

    /// Emits the stable digest key of any value, which always satisfies the
    /// checker's key rule.
    #[askama::filter_fn]
    pub fn live_key_digest(value: &dyn fmt::Display, _: &dyn Values) -> askama::Result<String> {
        Ok(super::live_key_digest(&value.to_string()))
    }
}
