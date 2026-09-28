//! In-flight WebAuthn ceremony state on the unified ceremony store.
//!
//! # `danger-allow-state-serialisation`, re-argued for this storage
//!
//! webauthn-rs gates serde support for `PasskeyRegistration` /
//! `PasskeyAuthentication` behind that feature because serialized challenge
//! state is dangerous when it can be tampered with or replayed. Both
//! conditions are satisfied here, more strongly than the encrypted-cookie
//! argument the deployed adapter originally made:
//!
//! 1. **Tamper-proofing.** The state never crosses the wire: the browser
//!    receives only the standard WebAuthn options JSON and an opaque
//!    selector. At rest the payload is additionally sealed under
//!    [`CryptoPurpose::CeremonyState`] AEAD, so even a database reader
//!    cannot alter it undetected.
//! 2. **Single-use.** Consumption goes through the ceremony store's atomic
//!    conditional consume; a replayed or concurrently raced finish gets
//!    nothing.
//!
//! Each ceremony additionally binds the begin-time email and credential actor
//! snapshot, so a finisher can neither retarget the ceremony nor substitute
//! fresher authorization after the ceremony begins.

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::crypto::{CryptoPurpose, Encryptor};
use crate::storage::{CeremonyStore, NewCeremony};
use crate::{Error, Result};

/// Ceremony namespace for registrations.
pub const REGISTRATION_KIND: &str = "passkey.registration";
/// Ceremony namespace for authentications.
pub const AUTHENTICATION_KIND: &str = "passkey.authentication";
/// Minutes-scale ceremony lifetime, as the deployed ceremony table bounds.
pub const CEREMONY_TTL_MINUTES: i64 = 5;

const CEREMONY_PENDING: &str = "pending";

/// Serialized webauthn state bound to the begin-time principal.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct BoundCeremony<State> {
    /// The serialized webauthn state machine.
    pub state: State,
    /// Normalized begin-time email.
    pub email: String,
    /// Begin-time user identifier.
    pub user_id: String,
    /// Authentication epoch carried by the begin-time actor.
    pub auth_epoch: u64,
    /// Opaque begin-time session id for session-authorized enrollment.
    pub opaque_session_id: Option<String>,
    /// Optional expiry carried by the begin-time authenticated session.
    pub actor_expires_at: Option<DateTime<Utc>>,
}

/// Store one bound ceremony and return its selector.
pub(crate) async fn store<State: Serialize>(
    ceremonies: &Arc<dyn CeremonyStore>,
    encryptor: &Arc<dyn Encryptor>,
    kind: &str,
    ceremony: &BoundCeremony<State>,
) -> Result<String> {
    let plaintext = serde_json::to_vec(ceremony).map_err(|error| Error::Internal {
        message: format!("serialize passkey ceremony: {error}"),
    })?;
    let payload = encryptor.encrypt(CryptoPurpose::CeremonyState, &plaintext)?;
    let selector = format!("passkey-{:032x}", rand::random::<u128>());
    ceremonies
        .create(NewCeremony {
            selector: selector.clone(),
            kind: kind.to_owned(),
            state: CEREMONY_PENDING.to_owned(),
            payload,
            expires_at: Utc::now() + Duration::minutes(CEREMONY_TTL_MINUTES),
        })
        .await?;
    Ok(selector)
}

/// Atomically consume one bound ceremony. A missing, expired, or already
/// consumed selector is a caller problem and fails identically.
pub(crate) async fn take<State: DeserializeOwned>(
    ceremonies: &Arc<dyn CeremonyStore>,
    encryptor: &Arc<dyn Encryptor>,
    kind: &str,
    selector: &str,
) -> Result<BoundCeremony<State>> {
    if selector.is_empty() {
        return Err(missing());
    }
    let record = ceremonies
        .consume(selector, kind)
        .await?
        .ok_or_else(missing)?;
    let plaintext = encryptor.decrypt(CryptoPurpose::CeremonyState, &record.payload)?;
    serde_json::from_slice(&plaintext).map_err(|error| Error::Internal {
        message: format!("stored passkey ceremony is malformed: {error}"),
    })
}

fn missing() -> Error {
    Error::InvalidInput {
        field: "selector".to_owned(),
        message: "no in-flight passkey ceremony; begin again".to_owned(),
    }
}
