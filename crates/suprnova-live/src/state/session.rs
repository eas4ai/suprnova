//! Host-neutral session access through registered session-only field metadata.

use std::error::Error;
use std::fmt;

use async_trait::async_trait;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::canonical::CanonicalValue;
use crate::identity::ModelField;
use crate::limits::InputLimits;
use crate::metadata::FieldMetadata;
use crate::snapshot::state::FieldCategory;

use super::ModelCodec;

const MAX_SESSION_INTENTS: usize = 256;

/// Closed session integration failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionErrorKind {
    /// A non-session field attempted to enter the session port.
    InvalidField,
    /// A session value failed its registered codec.
    InvalidValue,
    /// The request emitted more session intents than its configured bound.
    CapacityExceeded,
    /// The host session implementation failed without exposing its payload.
    HostFailure,
}

impl SessionErrorKind {
    /// Returns the stable safe machine value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidField => "invalid_session_field",
            Self::InvalidValue => "invalid_session_value",
            Self::CapacityExceeded => "session_intent_capacity_exceeded",
            Self::HostFailure => "session_host_failure",
        }
    }
}

/// Redacted session integration error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionError {
    kind: SessionErrorKind,
}

impl SessionError {
    const fn new(kind: SessionErrorKind) -> Self {
        Self { kind }
    }

    /// Creates the closed error exposed by a failing host session adapter.
    #[must_use]
    pub const fn host_failure() -> Self {
        Self::new(SessionErrorKind::HostFailure)
    }

    /// Returns the closed session failure.
    #[must_use]
    pub const fn kind(self) -> SessionErrorKind {
        self.kind
    }
}

impl fmt::Display for SessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind.as_str())
    }
}

impl Error for SessionError {}

/// Registered session-only field and typed value codec.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionField {
    name: ModelField,
    codec: ModelCodec,
}

impl SessionField {
    /// Projects a session field from canonical component metadata.
    pub fn from_metadata(field: &FieldMetadata) -> Result<Self, SessionError> {
        if field.category() != FieldCategory::Session {
            return Err(SessionError::new(SessionErrorKind::InvalidField));
        }
        let codec = field
            .session_codec()
            .cloned()
            .ok_or_else(|| SessionError::new(SessionErrorKind::InvalidField))?;
        Ok(Self {
            name: field.name().clone(),
            codec,
        })
    }

    /// Returns the registered session field identity.
    #[must_use]
    pub const fn name(&self) -> &ModelField {
        &self.name
    }

    /// Returns the registered typed codec.
    #[must_use]
    pub const fn codec(&self) -> &ModelCodec {
        &self.codec
    }
}

/// Typed canonical session value whose diagnostics never expose its payload.
#[derive(Clone)]
pub struct SessionValue(CanonicalValue);

impl SessionValue {
    /// Validates canonical host data against its registered session field.
    pub fn from_canonical(
        field: &SessionField,
        value: CanonicalValue,
        limits: &InputLimits,
    ) -> Result<Self, SessionError> {
        field
            .codec
            .validate(&value, limits)
            .map_err(|_| SessionError::new(SessionErrorKind::InvalidValue))?;
        Ok(Self(value))
    }

    /// Decodes session data through exactly the field codec used to validate it.
    pub fn decode<T: DeserializeOwned + 'static>(
        &self,
        field: &SessionField,
        limits: &InputLimits,
    ) -> Result<T, SessionError> {
        field
            .codec
            .decode(&self.0, limits)
            .map_err(|_| SessionError::new(SessionErrorKind::InvalidValue))
    }

    /// Returns canonical bytes to a trusted host session adapter.
    #[must_use]
    pub const fn canonical(&self) -> &CanonicalValue {
        &self.0
    }
}

impl fmt::Debug for SessionValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<SessionValue>")
    }
}

/// Closed session mutation intent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionIntentKind {
    /// Replace the registered field with a typed value.
    Set,
    /// Remove the registered field.
    Remove,
}

/// Bounded host session intent whose payload is redacted from diagnostics.
#[derive(Clone)]
pub struct SessionIntent {
    field: SessionField,
    kind: SessionIntentKind,
    value: Option<SessionValue>,
}

impl SessionIntent {
    /// Encodes a typed session value into a set intent.
    pub fn set<T: Serialize + 'static>(
        field: SessionField,
        value: &T,
        limits: &InputLimits,
    ) -> Result<Self, SessionError> {
        let value = field
            .codec
            .encode(value, limits)
            .map_err(|_| SessionError::new(SessionErrorKind::InvalidValue))?;
        Ok(Self {
            field,
            kind: SessionIntentKind::Set,
            value: Some(SessionValue(value)),
        })
    }

    /// Creates a remove intent for one registered session field.
    #[must_use]
    pub fn remove(field: SessionField) -> Self {
        Self {
            field,
            kind: SessionIntentKind::Remove,
            value: None,
        }
    }

    /// Returns the registered session field.
    #[must_use]
    pub const fn field(&self) -> &SessionField {
        &self.field
    }

    /// Returns set or remove without exposing a value.
    #[must_use]
    pub const fn kind(&self) -> SessionIntentKind {
        self.kind
    }

    /// Returns the typed payload to a trusted host session adapter.
    #[must_use]
    pub const fn value(&self) -> Option<&SessionValue> {
        self.value.as_ref()
    }
}

impl fmt::Debug for SessionIntent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionIntent")
            .field("field", &self.field.name)
            .field("kind", &self.kind)
            .field("value", &self.value.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// Bounded request-local collection of session intents.
#[derive(Debug)]
pub struct SessionIntents {
    max: usize,
    intents: Vec<SessionIntent>,
}

impl SessionIntents {
    /// Creates a nonzero collection below the engine hard ceiling.
    pub fn new(max: usize) -> Result<Self, SessionError> {
        if max == 0 || max > MAX_SESSION_INTENTS {
            return Err(SessionError::new(SessionErrorKind::CapacityExceeded));
        }
        Ok(Self {
            max,
            intents: Vec::new(),
        })
    }

    /// Appends one validated intent without exceeding the configured bound.
    pub fn push(&mut self, intent: SessionIntent) -> Result<(), SessionError> {
        if self.intents.len() >= self.max {
            return Err(SessionError::new(SessionErrorKind::CapacityExceeded));
        }
        self.intents.push(intent);
        Ok(())
    }

    /// Returns validated intents in emission order.
    #[must_use]
    pub fn as_slice(&self) -> &[SessionIntent] {
        &self.intents
    }
}

/// Current-host session boundary; no cookie, token, or backend type enters Live.
#[async_trait]
pub trait SessionPort: Send + Sync {
    /// Reads one registered session-only field from current host authority.
    async fn read(&self, field: &SessionField) -> Result<Option<SessionValue>, SessionError>;

    /// Applies one bounded typed session intent after the accepted component outcome.
    async fn apply(&self, intent: &SessionIntent) -> Result<(), SessionError>;
}
