//! Validated text and base64url identities carried by Live contracts.

use std::error::Error;
use std::fmt;
use std::hash::Hash;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Serialize;

/// Why construction of a typed identity failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityErrorKind {
    /// A text identity was empty, too long, or contained a forbidden byte.
    InvalidSyntax,
    /// A binary identity did not contain the required number of bytes.
    InvalidLength,
    /// A binary identity was not canonical unpadded base64url.
    InvalidEncoding,
    /// A decimal identity was empty, non-canonical, or overflowed `u64`.
    InvalidDecimal,
}

/// Safe construction error for a typed identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IdentityError {
    kind: IdentityErrorKind,
}

impl IdentityError {
    /// Returns the closed reason for rejection.
    #[must_use]
    pub const fn kind(self) -> IdentityErrorKind {
        self.kind
    }
}

impl fmt::Display for IdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self.kind {
            IdentityErrorKind::InvalidSyntax => "invalid_identifier",
            IdentityErrorKind::InvalidLength => "invalid_identity_length",
            IdentityErrorKind::InvalidEncoding => "invalid_base64url_identity",
            IdentityErrorKind::InvalidDecimal => "invalid_decimal_identity",
        };
        formatter.write_str(value)
    }
}

impl Error for IdentityError {}

fn parse_text_identity(value: &str, max_bytes: usize) -> Result<String, IdentityError> {
    let valid = !value.is_empty()
        && value.len() <= max_bytes
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'/' | b'-')
        });
    if !valid {
        return Err(IdentityError {
            kind: IdentityErrorKind::InvalidSyntax,
        });
    }
    Ok(value.to_owned())
}

macro_rules! text_identity {
    ($(#[$attribute:meta])* $name:ident, $max_bytes:expr) => {
        $(#[$attribute])*
        #[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Parses and validates the identity's bounded ASCII grammar.
            pub fn parse(value: &str) -> Result<Self, IdentityError> {
                parse_text_identity(value, $max_bytes).map(Self)
            }

            /// Returns the validated identity text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(concat!("<", stringify!($name), ">"))
            }
        }
    };
}

text_identity!(
    /// Stable registered Rust component identity.
    ComponentName,
    128
);
/// Stable checked relative external-template identity.
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ViewName(String);

impl ViewName {
    /// Parses a bounded relative template identity without traversal segments.
    pub fn parse(value: &str) -> Result<Self, IdentityError> {
        let syntax = parse_text_identity(value, 256)?;
        let valid = !syntax.starts_with('/')
            && !syntax.ends_with('/')
            && !syntax.contains(':')
            && syntax
                .split('/')
                .all(|segment| !segment.is_empty() && !matches!(segment, "." | ".."));
        if !valid {
            return Err(IdentityError {
                kind: IdentityErrorKind::InvalidSyntax,
            });
        }
        Ok(Self(syntax))
    }

    /// Returns the validated relative template identity.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ViewName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<ViewName>")
    }
}
text_identity!(
    /// Stable application build identity.
    BuildId,
    64
);
text_identity!(
    /// Stable island slot identity within a route document.
    IslandSlot,
    128
);
text_identity!(
    /// Explicit snapshot signing-key identity.
    KeyId,
    32
);
text_identity!(
    /// Registered Live action identity carried as untrusted protocol input.
    ActionName,
    128
);
text_identity!(
    /// Registered component model-field identity carried by synchronization input.
    ModelField,
    128
);
text_identity!(
    /// Declared browser event or registered effect identity.
    BrowserOperationName,
    128
);
/// Stable keyed DOM identity for one declared local-signal scope.
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct SignalScopeIdentity(String);

impl SignalScopeIdentity {
    /// Parses the browser-shared grammar: alphanumeric first, then bounded
    /// alphanumeric, dot, underscore, colon, or hyphen bytes.
    pub fn parse(value: &str) -> Result<Self, IdentityError> {
        let mut bytes = value.bytes();
        let valid = !value.is_empty()
            && value.len() <= 128
            && bytes
                .next()
                .is_some_and(|byte| byte.is_ascii_alphanumeric())
            && bytes.all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-')
            });
        if !valid {
            return Err(IdentityError {
                kind: IdentityErrorKind::InvalidSyntax,
            });
        }
        Ok(Self(value.to_owned()))
    }

    /// Returns the validated scope identity.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SignalScopeIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<SignalScopeIdentity>")
    }
}

#[cfg(test)]
mod signal_scope_tests {
    use proptest::prelude::*;

    use super::SignalScopeIdentity;

    #[test]
    fn signal_scope_grammar_matches_the_browser_contract() {
        for valid in ["a", "Root_1", "nested.panel:row-2", &"a".repeat(128)] {
            assert!(
                SignalScopeIdentity::parse(valid).is_ok(),
                "valid scope {valid}"
            );
        }
        for invalid in [
            "",
            "_root",
            "-root",
            ".root",
            ":root",
            "root/scope",
            &"a".repeat(129),
        ] {
            assert!(
                SignalScopeIdentity::parse(invalid).is_err(),
                "invalid scope {invalid}"
            );
        }
    }

    proptest! {
        #[test]
        fn signal_scope_acceptance_is_exactly_the_shared_ascii_grammar(value in any::<String>()) {
            let expected = !value.is_empty()
                && value.len() <= 128
                && value.as_bytes()[0].is_ascii_alphanumeric()
                && value.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-')
                });
            prop_assert_eq!(SignalScopeIdentity::parse(&value).is_ok(), expected);
        }
    }
}

fn parse_binary_identity(value: &str, min: usize, max: usize) -> Result<Vec<u8>, IdentityError> {
    let minimum_encoded_bytes = min.saturating_mul(8).div_ceil(6);
    let maximum_encoded_bytes = max.saturating_mul(8).div_ceil(6);
    if value.len() < minimum_encoded_bytes
        || value.len() > maximum_encoded_bytes
        || value.contains('=')
    {
        return Err(IdentityError {
            kind: IdentityErrorKind::InvalidLength,
        });
    }
    let decoded = URL_SAFE_NO_PAD.decode(value).map_err(|_| IdentityError {
        kind: IdentityErrorKind::InvalidEncoding,
    })?;
    if decoded.len() < min || decoded.len() > max {
        return Err(IdentityError {
            kind: IdentityErrorKind::InvalidLength,
        });
    }
    if URL_SAFE_NO_PAD.encode(&decoded) != value {
        return Err(IdentityError {
            kind: IdentityErrorKind::InvalidEncoding,
        });
    }
    Ok(decoded)
}

macro_rules! binary_identity {
    ($(#[$attribute:meta])* $name:ident, $min:expr, $max:expr) => {
        $(#[$attribute])*
        #[derive(Clone, Eq, Hash, PartialEq)]
        pub struct $name(Vec<u8>);

        impl $name {
            /// Parses canonical unpadded base64url and validates byte strength.
            pub fn parse(value: &str) -> Result<Self, IdentityError> {
                parse_binary_identity(value, $min, $max).map(Self)
            }

            /// Constructs the identity from already available bytes.
            pub fn from_bytes(value: &[u8]) -> Result<Self, IdentityError> {
                if value.len() < $min || value.len() > $max {
                    return Err(IdentityError {
                        kind: IdentityErrorKind::InvalidLength,
                    });
                }
                Ok(Self(value.to_vec()))
            }

            /// Returns the validated identity bytes.
            #[must_use]
            pub fn as_bytes(&self) -> &[u8] {
                &self.0
            }

            /// Encodes the identity as canonical unpadded base64url.
            #[must_use]
            pub fn to_base64url(&self) -> String {
                URL_SAFE_NO_PAD.encode(&self.0)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                serializer.serialize_str(&self.to_base64url())
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(concat!("<", stringify!($name), ">"))
            }
        }
    };
}

binary_identity!(
    /// At-least-128-bit browser-proposed identity input for seed promotion.
    BrowserNonce,
    16,
    32
);
binary_identity!(
    /// Server-assigned opaque Live island instance identity.
    InstanceId,
    16,
    32
);
binary_identity!(
    /// End-to-end request correlation identity safe only after validation.
    CorrelationId,
    16,
    32
);
binary_identity!(
    /// Bounded retry identity scoped by the ledger contract.
    IdempotencyKey,
    16,
    32
);
binary_identity!(
    /// Purpose-specific digest of the trusted principal/session/tenant scope.
    ScopeFingerprint,
    32,
    32
);
binary_identity!(
    /// Purpose-specific digest of the canonical route identity.
    RouteIdentity,
    32,
    32
);
binary_identity!(
    /// Purpose-specific digest used for bounded metadata comparisons.
    ContentDigest,
    32,
    32
);

fn parse_decimal(value: &str) -> Result<u64, IdentityError> {
    let canonical = value == "0"
        || (!value.starts_with('0')
            && !value.is_empty()
            && value.bytes().all(|byte| byte.is_ascii_digit()));
    if !canonical {
        return Err(IdentityError {
            kind: IdentityErrorKind::InvalidDecimal,
        });
    }
    value.parse().map_err(|_| IdentityError {
        kind: IdentityErrorKind::InvalidDecimal,
    })
}

macro_rules! decimal_identity {
    ($(#[$attribute:meta])* $name:ident) => {
        $(#[$attribute])*
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(u64);

        impl $name {
            /// Creates the identity from trusted integer state.
            #[must_use]
            pub const fn new(value: u64) -> Self {
                Self(value)
            }

            /// Parses the canonical unsigned decimal wire representation.
            pub fn parse(value: &str) -> Result<Self, IdentityError> {
                parse_decimal(value).map(Self)
            }

            /// Returns the underlying integer.
            #[must_use]
            pub const fn get(self) -> u64 {
                self.0
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                serializer.collect_str(&self.0)
            }
        }
    };
}

decimal_identity!(
    /// Monotonic revision of one Live instance.
    Revision
);
decimal_identity!(
    /// Unix epoch milliseconds encoded as a decimal string on the wire.
    UnixMillis
);
decimal_identity!(
    /// Dependency generation encoded as a decimal string on the wire.
    Generation
);
decimal_identity!(
    /// Millisecond duration encoded as a decimal string on the wire.
    DurationMillis
);

impl Revision {
    /// Returns the monotonic successor or an error at `u64::MAX`.
    pub fn checked_next(self) -> Result<Self, IdentityError> {
        self.0.checked_add(1).map(Self).ok_or(IdentityError {
            kind: IdentityErrorKind::InvalidDecimal,
        })
    }
}
