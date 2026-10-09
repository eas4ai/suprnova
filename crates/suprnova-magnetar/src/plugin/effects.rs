//! Typed response effects consumed by framework adapters.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::sessions::{RememberCredential, SessionGrant};
/// Compatibility alias emphasizing that effects belong to responses.
pub type ResponseEffect = Effect;

/// A response-side effect that a host adapter maps to its carrier/runtime.
#[derive(Debug)]
pub enum Effect {
    /// Move a host-issued grant into the selected session carrier.
    EstablishSession(SessionGrant),
    /// Clear the current host session carrier.
    ClearSession,
    /// Move an issued remember-me credential into the host's persistent
    /// carrier (an encrypted cookie today). The composite is exposed exactly
    /// once by consuming the credential.
    IssueRemember(RememberCredential),
    /// Redirect to a host-resolved location.
    Redirect {
        /// Redirect target.
        location: String,
        /// HTTP status, commonly 302 or 303.
        status: u16,
    },
    /// Override the response status.
    SetStatus(u16),
    /// Add or replace a response header.
    SetHeader {
        /// Header name.
        name: String,
        /// Header value.
        value: String,
    },
    /// Replace the response body with JSON.
    Json(Value),
}

/// Carrier-neutral response returned from a plugin handler.
#[derive(Debug)]
pub struct EffectResponse {
    /// Status code before effects are applied.
    pub status: u16,
    /// Headers before effects are applied.
    pub headers: BTreeMap<String, String>,
    /// Optional body before effects are applied.
    pub body: Option<Value>,
    /// Ordered effects for the host adapter.
    pub effects: Vec<Effect>,
}

impl Default for EffectResponse {
    fn default() -> Self {
        Self {
            status: 200,
            headers: BTreeMap::new(),
            body: None,
            effects: Vec::new(),
        }
    }
}

impl EffectResponse {
    /// Construct a successful empty response.
    pub fn ok() -> Self {
        Self::default()
    }
    /// Construct a JSON response.
    pub fn json(body: Value) -> Self {
        Self {
            body: Some(body),
            ..Self::default()
        }
    }
    /// Add one ordered effect.
    pub fn with_effect(mut self, effect: Effect) -> Self {
        self.effects.push(effect);
        self
    }
}
/// Compatibility alias for a typed effect response.
pub type PluginResponse = EffectResponse;
