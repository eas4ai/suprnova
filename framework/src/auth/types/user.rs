//! User identity and account values exposed by Suprnova authentication.

use std::{fmt, str::FromStr};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::FrameworkError;

const BASE58_ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
const RANDOM_ID_LENGTH: usize = 20;
const MINIMUM_ID_BYTES: usize = 12;

/// A stable opaque user identifier.
///
/// The identifier is displayable and can be stored in framework sessions, but
/// callers must not infer structure from it. [`FromStr`] validates the
/// `usr_`-prefixed base58 format; [`UserId::new`] preserves the legacy
/// construction path for values loaded from an existing store.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct UserId(String);

impl UserId {
    /// Creates an opaque user identifier without validating it.
    ///
    /// Use [`FromStr`] when an identifier crosses an untrusted boundary.
    #[must_use]
    pub fn new(value: &str) -> Self {
        Self(value.to_owned())
    }

    /// Creates a new valid, random opaque user identifier.
    #[must_use]
    pub fn new_random() -> Self {
        let mut value = String::with_capacity("usr_".len() + RANDOM_ID_LENGTH);
        value.push_str("usr_");
        append_random_base58(&mut value);
        Self(value)
    }

    /// Consumes this identifier and returns its stored string.
    #[must_use]
    pub fn into_inner(self) -> String {
        self.0
    }

    /// Returns the stored opaque identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns whether this identifier has the canonical user-id format.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        is_valid_user_id(&self.0)
    }
}

impl Default for UserId {
    fn default() -> Self {
        Self::new_random()
    }
}

impl From<String> for UserId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for UserId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<UserId> for String {
    fn from(value: UserId) -> Self {
        value.into_inner()
    }
}

impl fmt::Display for UserId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for UserId {
    type Err = FrameworkError;

    /// Parses and validates a canonical `usr_`-prefixed base58 identifier.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let id = Self::new(value);
        if id.is_valid() {
            Ok(id)
        } else {
            Err(FrameworkError::bad_request(
                "expected a usr_-prefixed base58 identifier with at least 96 bits of entropy",
            ))
        }
    }
}

/// A Suprnova-compatible account returned by an authentication flow.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct User {
    /// The account's opaque stable identifier.
    pub id: UserId,
    /// The optional account display name.
    pub name: Option<String>,
    /// The account's normalized email address.
    pub email: String,
    /// When the email address was verified, if it has been verified.
    pub email_verified_at: Option<DateTime<Utc>>,
    /// When the account was locked, if it is currently locked.
    pub locked_at: Option<DateTime<Utc>>,
    /// When the account was created.
    pub created_at: DateTime<Utc>,
    /// When the account was last updated.
    pub updated_at: DateTime<Utc>,
}

impl User {
    /// Starts constructing a user with legacy-compatible defaults.
    #[must_use]
    pub fn builder() -> UserBuilder {
        UserBuilder::default()
    }

    /// Returns whether the account's email address has been verified.
    #[must_use]
    pub fn is_email_verified(&self) -> bool {
        self.email_verified_at.is_some()
    }

    /// Returns whether the account has a recorded lock timestamp.
    ///
    /// This reports the stored account state. A policy service remains
    /// responsible for deciding whether a lockout window is currently active.
    #[must_use]
    pub fn is_locked(&self) -> bool {
        self.locked_at.is_some()
    }
}

/// Builder for a [`User`].
#[derive(Default)]
pub struct UserBuilder {
    id: Option<UserId>,
    name: Option<String>,
    email: Option<String>,
    email_verified_at: Option<DateTime<Utc>>,
    locked_at: Option<DateTime<Utc>>,
    created_at: Option<DateTime<Utc>>,
    updated_at: Option<DateTime<Utc>>,
}

impl UserBuilder {
    /// Sets the account identifier.
    #[must_use]
    pub fn id(mut self, id: UserId) -> Self {
        self.id = Some(id);
        self
    }

    /// Sets the optional account display name.
    #[must_use]
    pub fn name(mut self, name: Option<String>) -> Self {
        self.name = name;
        self
    }

    /// Sets the required account email address.
    #[must_use]
    pub fn email(mut self, email: String) -> Self {
        self.email = Some(email);
        self
    }

    /// Sets the optional email-verification timestamp.
    #[must_use]
    pub fn email_verified_at(mut self, email_verified_at: Option<DateTime<Utc>>) -> Self {
        self.email_verified_at = email_verified_at;
        self
    }

    /// Sets the optional account-lock timestamp.
    #[must_use]
    pub fn locked_at(mut self, locked_at: Option<DateTime<Utc>>) -> Self {
        self.locked_at = locked_at;
        self
    }

    /// Sets the account creation time.
    #[must_use]
    pub fn created_at(mut self, created_at: DateTime<Utc>) -> Self {
        self.created_at = Some(created_at);
        self
    }

    /// Sets the account update time.
    #[must_use]
    pub fn updated_at(mut self, updated_at: DateTime<Utc>) -> Self {
        self.updated_at = Some(updated_at);
        self
    }

    /// Builds the account, generating an identifier and timestamps when absent.
    ///
    /// # Errors
    ///
    /// Returns [`FrameworkError`] when no email address was supplied.
    pub fn build(self) -> Result<User, FrameworkError> {
        let now = Utc::now();
        let email = self
            .email
            .ok_or_else(|| FrameworkError::bad_request("an email address is required"))?;

        Ok(User {
            id: self.id.unwrap_or_default(),
            name: self.name,
            email,
            email_verified_at: self.email_verified_at,
            locked_at: self.locked_at,
            created_at: self.created_at.unwrap_or(now),
            updated_at: self.updated_at.unwrap_or(now),
        })
    }
}

fn append_random_base58(value: &mut String) {
    let mut random = [0_u8; 32];
    let mut index = random.len();

    while value.len() < "usr_".len() + RANDOM_ID_LENGTH {
        if index == random.len() {
            for byte in &mut random {
                *byte = rand::random();
            }
            index = 0;
        }

        let byte = random[index];
        index += 1;
        if byte < 232 {
            value.push(char::from(BASE58_ALPHABET[usize::from(byte % 58)]));
        }
    }
}

fn is_valid_user_id(value: &str) -> bool {
    let Some(payload) = value.strip_prefix("usr_") else {
        return false;
    };
    if payload.is_empty() {
        return false;
    }

    let mut leading_zero_bytes = 0_usize;
    let mut remainder_digits = 0_usize;
    let mut remainder_value = 0_u128;

    for byte in payload.bytes() {
        let Some(digit) = base58_value(byte) else {
            return false;
        };

        if remainder_digits == 0 && digit == 0 {
            leading_zero_bytes += 1;
            continue;
        }

        remainder_digits += 1;
        if remainder_digits <= 16 {
            remainder_value = remainder_value * 58 + u128::from(digit);
        }
    }

    if leading_zero_bytes >= MINIMUM_ID_BYTES {
        return true;
    }

    let required_remainder_bytes = MINIMUM_ID_BYTES - leading_zero_bytes;
    if remainder_digits == 0 {
        return false;
    }
    if remainder_digits >= 17 {
        return true;
    }

    let smallest_value_for_width = 1_u128 << ((required_remainder_bytes - 1) * 8);
    remainder_value >= smallest_value_for_width
}

fn base58_value(byte: u8) -> Option<u8> {
    BASE58_ALPHABET
        .iter()
        .position(|candidate| *candidate == byte)
        .and_then(|index| u8::try_from(index).ok())
}
