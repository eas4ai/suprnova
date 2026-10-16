//! Framework-neutral inbound wire types and route dispatch helpers.

use std::collections::BTreeMap;

use serde_json::Value;

use super::effects::EffectResponse;

/// HTTP methods recognized by route descriptors.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Method {
    /// GET request.
    #[default]
    Get,
    /// POST request.
    Post,
    /// PUT request.
    Put,
    /// PATCH request.
    Patch,
    /// DELETE request.
    Delete,
    /// HEAD request.
    Head,
    /// OPTIONS request.
    Options,
    /// Any extension method represented by a token.
    Other(String),
}

impl Method {
    /// Parse a method token without depending on an HTTP framework.
    pub fn parse(value: &str) -> Self {
        match value.to_ascii_uppercase().as_str() {
            "GET" => Self::Get,
            "POST" => Self::Post,
            "PUT" => Self::Put,
            "PATCH" => Self::Patch,
            "DELETE" => Self::Delete,
            "HEAD" => Self::Head,
            "OPTIONS" => Self::Options,
            other => Self::Other(other.to_owned()),
        }
    }
}

/// Compatibility alias used by hosts that spell out HTTP method.
pub type HttpMethod = Method;

/// Body supplied on a wire request.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum WireBody {
    /// No body.
    #[default]
    Empty,
    /// JSON body.
    Json(Value),
    /// Form-encoded fields (including `form_post` callbacks).
    Form(BTreeMap<String, String>),
    /// Opaque bytes with a host-supplied content type.
    Bytes {
        /// Content type, when available.
        content_type: Option<String>,
        /// Body bytes.
        bytes: Vec<u8>,
    },
}

/// Framework-neutral inbound request.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WireRequest {
    /// HTTP method.
    pub method: Method,
    /// Request path without query string.
    pub path: String,
    /// Captured route parameters.
    pub path_params: BTreeMap<String, String>,
    /// Parsed query parameters.
    pub query: BTreeMap<String, String>,
    /// Case-preserving request headers.
    pub headers: BTreeMap<String, String>,
    /// Parsed or opaque body.
    pub body: WireBody,
}

impl WireRequest {
    /// Construct a request from method and path.
    pub fn new(method: Method, path: impl Into<String>) -> Self {
        Self {
            method,
            path: path.into(),
            ..Self::default()
        }
    }
}

/// Framework-neutral plugin response.
#[derive(Debug)]
pub struct WireResponse(pub EffectResponse);

impl WireResponse {
    /// Construct a response from typed host effects.
    pub fn from_effects(effects: EffectResponse) -> Self {
        Self(effects)
    }
    /// Construct an empty successful response.
    pub fn ok() -> Self {
        Self(EffectResponse::ok())
    }
    /// Construct a JSON response.
    pub fn json(body: Value) -> Self {
        Self(EffectResponse::json(body))
    }
    /// Borrow the typed effects for host mapping.
    pub fn effects(&self) -> &[super::effects::Effect] {
        &self.0.effects
    }
    /// Consume the response into its effect representation.
    pub fn into_effects(self) -> EffectResponse {
        self.0
    }
}
