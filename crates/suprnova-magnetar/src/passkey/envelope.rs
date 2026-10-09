//! The deployed passkey storage envelope, byte-compatible with the fork.
//!
//! `data_json` is an outer JSON object - `credential_id` (base64-standard),
//! `public_key` (base64-standard over the serialized `webauthn_rs::Passkey`),
//! `name`, `created_at`, `last_used_at` - serialized with serde_json's
//! default alphabetical key order, exactly as torii's repository writes it.
//! Updates edit fields inside the parsed object rather than reserializing a
//! typed struct, so untouched fields round-trip byte-for-byte and the
//! migration domain can copy whole rows without transformation.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use webauthn_rs::prelude::Passkey;

use crate::{Error, Result};

/// A read view over one stored envelope.
#[derive(Clone, Debug)]
pub struct PasskeyEnvelope {
    value: Value,
}

impl PasskeyEnvelope {
    /// Build the envelope for a freshly registered credential, matching the
    /// deployed shape and key order.
    pub fn for_new_credential(passkey: &Passkey, name: Option<&str>) -> Result<Self> {
        let passkey_bytes = serde_json::to_vec(passkey).map_err(|error| Error::Internal {
            message: format!("serialize webauthn credential: {error}"),
        })?;
        Ok(Self {
            value: json!({
                "credential_id": STANDARD.encode(passkey.cred_id()),
                "public_key": STANDARD.encode(passkey_bytes),
                "name": name,
                "created_at": Utc::now(),
                "last_used_at": Option::<DateTime<Utc>>::None,
            }),
        })
    }

    /// Parse a stored envelope.
    pub fn parse(envelope_json: &str) -> Result<Self> {
        let value: Value =
            serde_json::from_str(envelope_json).map_err(|error| Error::Internal {
                message: format!("stored passkey envelope is not JSON: {error}"),
            })?;
        Ok(Self { value })
    }

    /// The base64-standard credential identifier.
    pub fn credential_id_b64(&self) -> Result<String> {
        self.value["credential_id"]
            .as_str()
            .map(ToOwned::to_owned)
            .ok_or_else(|| malformed("credential_id"))
    }

    /// The stored display name, when set.
    #[must_use]
    pub fn name(&self) -> Option<String> {
        self.value["name"].as_str().map(ToOwned::to_owned)
    }

    /// The last successful authentication, when any.
    #[must_use]
    pub fn last_used_at(&self) -> Option<DateTime<Utc>> {
        serde_json::from_value(self.value["last_used_at"].clone()).unwrap_or(None)
    }

    /// Deserialize the stored webauthn credential.
    pub fn passkey(&self) -> Result<Passkey> {
        let encoded = self.value["public_key"]
            .as_str()
            .ok_or_else(|| malformed("public_key"))?;
        let bytes = STANDARD
            .decode(encoded)
            .map_err(|_| malformed("public_key"))?;
        serde_json::from_slice(&bytes).map_err(|error| Error::Internal {
            message: format!("stored passkey blob is not a serialized credential: {error}"),
        })
    }

    /// Rewrite only the credential blob and the last-used stamp, preserving
    /// every other field byte-for-byte (the deployed counter-update shape).
    pub fn with_updated_credential(
        mut self,
        passkey: &Passkey,
        last_used_at: DateTime<Utc>,
    ) -> Result<Self> {
        let passkey_bytes = serde_json::to_vec(passkey).map_err(|error| Error::Internal {
            message: format!("serialize updated webauthn credential: {error}"),
        })?;
        self.value["public_key"] = Value::String(STANDARD.encode(passkey_bytes));
        self.value["last_used_at"] = json!(last_used_at);
        Ok(self)
    }

    /// Serialize the envelope for storage.
    #[must_use]
    pub fn to_json(&self) -> String {
        self.value.to_string()
    }
}

fn malformed(field: &str) -> Error {
    Error::Internal {
        message: format!("stored passkey envelope is missing {field}"),
    }
}
