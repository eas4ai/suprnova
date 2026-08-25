//! The `WebAuthnAuthProvider` domain: passkey ceremonies, credential
//! storage, and the binding invariants that keep them honest.
//!
//! Ported from the deployed division of labor: storage never claims to
//! authenticate (the `968b0be` honesty discipline - lookups by credential
//! id are lookups over a public value), and webauthn verification is the
//! only path that treats a row as authenticated. Existing-account
//! enrollment is bound to the exact authenticated owner within the
//! three-hour password-reauth window; a genuinely new email remains an
//! unauthenticated signup. Every successful assertion passes through the
//! shared factor gate before any session exists.

pub mod ceremony;
pub mod envelope;

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use chrono::{DateTime, Utc};
use webauthn_rs::prelude::{
    CreationChallengeResponse, Passkey, PasskeyAuthentication, PasskeyRegistration,
    PublicKeyCredential, RegisterPublicKeyCredential, RequestChallengeResponse, Url, Uuid,
    Webauthn, WebauthnBuilder,
};

use crate::auth::reauth::{ReauthStamp, validate_reauth};
use crate::auth::{
    AuthenticationContext, FactorGate, SignInDecision, SignInMethod, VerifiedPrincipal,
};
use crate::crypto::Encryptor;
use crate::password::normalize_email;
use crate::sessions::SessionMetadata;
use crate::storage::{
    CeremonyStore, CredentialActor, NewUser, PasskeyRow, PasskeyStore, UserStore,
};
use crate::{Error, Result};

use ceremony::{AUTHENTICATION_KIND, BoundCeremony, REGISTRATION_KIND};
use envelope::PasskeyEnvelope;

/// Relying-party configuration, env-driven with localhost defaults in
/// scaffolds. Changing these against existing credentials is a documented
/// operator hazard: WebAuthn scopes credentials to the relying party, so a
/// changed `rp_id` orphans every previously registered passkey rather than
/// migrating it.
#[derive(Clone, Debug)]
pub struct PasskeyConfig {
    /// The relying-party identifier (an effective domain of the origin).
    pub rp_id: String,
    /// The relying-party origin.
    pub rp_origin: String,
}

impl Default for PasskeyConfig {
    /// Scaffold defaults: boot with no environment, exactly as today.
    fn default() -> Self {
        Self {
            rp_id: "localhost".to_owned(),
            rp_origin: "http://localhost".to_owned(),
        }
    }
}

/// Begin-registration input. `actor` and `reauthenticated_at` carry the
/// authenticated owner witness and password-confirmation stamp for the
/// existing-account branch; a genuinely new email needs neither.
#[derive(Clone, Debug)]
pub struct RegistrationIntent {
    /// Target email address.
    pub email: String,
    /// The authenticated caller, when any. This witness must come from the
    /// verified request session rather than a caller-supplied user id.
    pub actor: Option<CredentialActor>,
    /// When the caller last confirmed their password.
    pub reauthenticated_at: Option<DateTime<Utc>>,
}

/// The wire half of one begun ceremony: the opaque selector plus the
/// standard WebAuthn options. Serialized webauthn state never leaves the
/// server; selector placement (data session or wire) is the host's choice.
#[derive(Debug)]
pub struct BegunRegistration {
    /// Opaque ceremony selector.
    pub selector: String,
    /// Options for `navigator.credentials.create()`.
    pub options: CreationChallengeResponse,
}

/// The wire half of one begun authentication.
#[derive(Debug)]
pub struct BegunAuthentication {
    /// Opaque ceremony selector.
    pub selector: String,
    /// Options for `navigator.credentials.get()`.
    pub options: RequestChallengeResponse,
}

/// A listing view over one stored credential.
#[derive(Clone, Debug)]
pub struct PasskeySummary {
    /// Census removal handle accepted by the actor-bound method store.
    pub passkey_id: String,
    /// Base64-standard credential identifier.
    pub credential_id: String,
    /// Display name, when set.
    pub name: Option<String>,
    /// Row creation time.
    pub created_at: DateTime<Utc>,
    /// Last successful authentication, when any.
    pub last_used_at: Option<DateTime<Utc>>,
}

/// Passkey ceremonies and credential management.
pub struct PasskeyAuthService {
    users: Arc<dyn UserStore>,
    passkeys: Arc<dyn PasskeyStore>,
    ceremonies: Arc<dyn CeremonyStore>,
    encryptor: Arc<dyn Encryptor>,
    gate: Arc<dyn FactorGate>,
    webauthn: Webauthn,
}

impl PasskeyAuthService {
    /// Build the service; fails when `rp_id` is not an effective domain of
    /// `rp_origin` (a webauthn constraint).
    pub fn new(
        config: &PasskeyConfig,
        users: Arc<dyn UserStore>,
        passkeys: Arc<dyn PasskeyStore>,
        ceremonies: Arc<dyn CeremonyStore>,
        encryptor: Arc<dyn Encryptor>,
        gate: Arc<dyn FactorGate>,
    ) -> Result<Self> {
        let origin = Url::parse(&config.rp_origin).map_err(|error| Error::InvalidInput {
            field: "rp_origin".to_owned(),
            message: error.to_string(),
        })?;
        let webauthn = WebauthnBuilder::new(&config.rp_id, &origin)
            .and_then(webauthn_rs::WebauthnBuilder::build)
            .map_err(|error| Error::InvalidInput {
                field: "rp_id".to_owned(),
                message: format!("invalid relying-party configuration: {error:?}"),
            })?;
        Ok(Self {
            users,
            passkeys,
            ceremonies,
            encryptor,
            gate,
            webauthn,
        })
    }

    /// Begin a registration ceremony.
    ///
    /// A brand-new email is a signup and needs no authentication. An email
    /// with an account on file is an enrollment: the caller must be the
    /// exact authenticated owner with a password-confirmation stamp no
    /// older than three hours - identity comes from the authenticated
    /// actor, never from the caller-supplied email alone (SEC-01).
    pub async fn begin_registration(
        &self,
        intent: RegistrationIntent,
    ) -> Result<BegunRegistration> {
        let email = normalize_email(&intent.email);
        if email.is_empty() {
            return Err(Error::InvalidInput {
                field: "email".to_owned(),
                message: "must not be empty".to_owned(),
            });
        }
        let (user, actor) = match self.users.find_by_email(&email).await? {
            Some(existing) => {
                let actor = intent.actor.ok_or_else(|| Error::InvalidInput {
                    field: "actor".to_owned(),
                    message: "enrolling a passkey on an existing account requires the \
                              authenticated owner"
                        .to_owned(),
                })?;
                if actor.user_id() != existing.user_id {
                    return Err(Error::InvalidInput {
                        field: "actor".to_owned(),
                        message: "enrolling a passkey on an existing account requires the \
                                  authenticated owner"
                            .to_owned(),
                    });
                }
                let confirmed_at =
                    intent
                        .reauthenticated_at
                        .ok_or_else(|| Error::InvalidInput {
                            field: "reauth".to_owned(),
                            message: "confirm your password before adding a passkey".to_owned(),
                        })?;
                validate_reauth(
                    &existing.user_id,
                    ReauthStamp {
                        owner_user_id: existing.user_id.clone(),
                        password_confirmed_at: confirmed_at,
                    },
                    Utc::now(),
                )
                .map_err(|_| Error::InvalidInput {
                    field: "reauth".to_owned(),
                    message: "confirm your password before adding a passkey".to_owned(),
                })?;
                (existing, actor)
            }
            None => {
                // A brand-new email registering a passkey IS a signup.
                let created = self
                    .users
                    .create_user(NewUser {
                        email: email.clone(),
                        password_hash: None,
                    })
                    .await?;
                if created.password_hash.is_some() {
                    return Err(Error::Internal {
                        message: "user binding cannot represent passwordless accounts; \
                                  passkey signup is unavailable"
                            .to_owned(),
                    });
                }
                let actor = CredentialActor::verified_primary(&created.user_id, created.auth_epoch);
                (created, actor)
            }
        };

        // Deterministic v5 UUID over the opaque user id, so webauthn always
        // sees the same user handle for the same account.
        let user_uuid = Uuid::new_v5(&Uuid::NAMESPACE_URL, user.user_id.as_bytes());
        let existing_ids: Vec<webauthn_rs::prelude::CredentialID> = self
            .passkeys
            .passkeys_for_user(&user.user_id)
            .await?
            .iter()
            .map(|row| decode_credential_id(&row.credential_id).map(Into::into))
            .collect::<Result<_>>()?;
        let exclude = if existing_ids.is_empty() {
            None
        } else {
            Some(existing_ids)
        };

        let (options, state) = self
            .webauthn
            .start_passkey_registration(user_uuid, &email, &email, exclude)
            .map_err(|error| Error::Internal {
                message: format!("webauthn start_passkey_registration: {error:?}"),
            })?;

        let selector = ceremony::store(
            &self.ceremonies,
            &self.encryptor,
            REGISTRATION_KIND,
            &BoundCeremony {
                state,
                email,
                user_id: user.user_id,
                auth_epoch: actor.issuance_epoch(),
                opaque_session_id: actor.opaque_session_id().map(ToOwned::to_owned),
                actor_expires_at: actor.expires_at(),
            },
        )
        .await?;
        Ok(BegunRegistration { selector, options })
    }

    /// Finish a registration ceremony. The ceremony is consumed before any
    /// check, so a mismatched or failed finish leaves no retry oracle.
    pub async fn finish_registration(
        &self,
        selector: &str,
        email: &str,
        response: &RegisterPublicKeyCredential,
    ) -> Result<String> {
        let ceremony: BoundCeremony<PasskeyRegistration> = ceremony::take(
            &self.ceremonies,
            &self.encryptor,
            REGISTRATION_KIND,
            selector,
        )
        .await?;
        if !normalize_email(email).eq_ignore_ascii_case(&ceremony.email) {
            return Err(Error::InvalidInput {
                field: "email".to_owned(),
                message: "passkey registration email mismatch - the ceremony was begun for a \
                          different account"
                    .to_owned(),
            });
        }
        let passkey = self
            .webauthn
            .finish_passkey_registration(response, &ceremony.state)
            .map_err(|error| Error::InvalidInput {
                field: "credential".to_owned(),
                message: format!("webauthn registration verification failed: {error:?}"),
            })?;

        let actor = CredentialActor::from_snapshot(
            ceremony.user_id,
            ceremony.auth_epoch,
            ceremony.opaque_session_id,
            ceremony.actor_expires_at,
        );
        // Belt-and-braces: resolve through the ceremony-bound identity and
        // require it to still be the same account.
        let user = self
            .users
            .find_by_email(&ceremony.email)
            .await?
            .ok_or_else(|| Error::Internal {
                message: "passkey: user disappeared between begin and finish".to_owned(),
            })?;
        if user.user_id != actor.user_id() {
            return Err(Error::Internal {
                message: "passkey: user changed between begin and finish".to_owned(),
            });
        }

        let envelope = PasskeyEnvelope::for_new_credential(&passkey, None)?;
        self.passkeys
            .insert_passkey(
                &actor,
                &STANDARD.encode(passkey.cred_id()),
                &envelope.to_json(),
            )
            .await?;
        Ok(user.user_id)
    }

    /// Begin an authentication ceremony. Absent accounts and accounts with
    /// no credentials fail identically.
    pub async fn begin_authentication(&self, email: &str) -> Result<BegunAuthentication> {
        let email = normalize_email(email);
        let user = self
            .users
            .find_by_email(&email)
            .await?
            .ok_or_else(authentication_failed)?;
        let rows = self.passkeys.passkeys_for_user(&user.user_id).await?;
        if rows.is_empty() {
            return Err(authentication_failed());
        }
        let passkeys = decode_passkeys(&rows)?;
        let (options, state) = self
            .webauthn
            .start_passkey_authentication(&passkeys)
            .map_err(|error| Error::Internal {
                message: format!("webauthn start_passkey_authentication: {error:?}"),
            })?;
        let actor = CredentialActor::verified_primary(&user.user_id, user.auth_epoch);
        let selector = ceremony::store(
            &self.ceremonies,
            &self.encryptor,
            AUTHENTICATION_KIND,
            &BoundCeremony {
                state,
                email,
                user_id: user.user_id,
                auth_epoch: actor.issuance_epoch(),
                opaque_session_id: None,
                actor_expires_at: actor.expires_at(),
            },
        )
        .await?;
        Ok(BegunAuthentication { selector, options })
    }

    /// Finish an authentication ceremony: verify the assertion against the
    /// bound challenge, persist the counter and last-used stamp atomically,
    /// then pass the verified principal through the shared factor gate.
    pub async fn finish_authentication(
        &self,
        selector: &str,
        email: &str,
        response: &PublicKeyCredential,
        metadata: SessionMetadata,
    ) -> Result<SignInDecision> {
        let ceremony: BoundCeremony<PasskeyAuthentication> = ceremony::take(
            &self.ceremonies,
            &self.encryptor,
            AUTHENTICATION_KIND,
            selector,
        )
        .await?;
        if !normalize_email(email).eq_ignore_ascii_case(&ceremony.email) {
            return Err(Error::InvalidInput {
                field: "email".to_owned(),
                message: "passkey authentication email mismatch - the ceremony was begun for a \
                          different account"
                    .to_owned(),
            });
        }
        let auth_result = self
            .webauthn
            .finish_passkey_authentication(response, &ceremony.state)
            .map_err(|error| Error::InvalidInput {
                field: "credential".to_owned(),
                message: format!("webauthn authentication verification failed: {error:?}"),
            })?;
        // The begin-time primary actor is usable only after WebAuthn has
        // verified the assertion that created it.
        let actor = CredentialActor::from_snapshot(
            ceremony.user_id,
            ceremony.auth_epoch,
            ceremony.opaque_session_id,
            ceremony.actor_expires_at,
        );
        let user = self
            .users
            .find_by_email(&ceremony.email)
            .await?
            .ok_or_else(authentication_failed)?;
        if user.user_id != actor.user_id() {
            return Err(Error::Internal {
                message: "passkey: user changed between begin and finish".to_owned(),
            });
        }

        // Rewrite the matched credential's counter and last-used stamp in
        // one atomic envelope update; webauthn proved membership in the
        // allow-list, so a missing row is internal inconsistency.
        let used_b64 = STANDARD.encode(auth_result.cred_id());
        let rows = self.passkeys.passkeys_for_user(actor.user_id()).await?;
        let row = rows
            .iter()
            .find(|row| row.credential_id == used_b64)
            .ok_or_else(|| Error::Internal {
                message: "authenticated credential not found in stored set".to_owned(),
            })?;
        let stored = PasskeyEnvelope::parse(&row.envelope_json)?;
        let mut passkey = stored.passkey()?;
        passkey.update_credential(&auth_result);
        let updated = stored.with_updated_credential(&passkey, Utc::now())?;
        self.passkeys
            .update_passkey_envelope(&actor, &used_b64, &updated.to_json())
            .await?;

        let principal = VerifiedPrincipal::new(
            actor.user_id().to_owned(),
            SignInMethod::Passkey,
            AuthenticationContext::new(metadata, actor.issuance_epoch(), Utc::now()),
        )?;
        let context = principal.context().clone();
        self.gate.complete_sign_in(principal, context).await
    }

    /// List a user's credentials for account UIs. Removal goes through the
    /// census-guarded method store, never through this service.
    pub async fn list(&self, user_id: &str) -> Result<Vec<PasskeySummary>> {
        let rows = self.passkeys.passkeys_for_user(user_id).await?;
        rows.iter()
            .map(|row| {
                let envelope = PasskeyEnvelope::parse(&row.envelope_json)?;
                Ok(PasskeySummary {
                    passkey_id: row.passkey_id.clone(),
                    credential_id: row.credential_id.clone(),
                    name: envelope.name(),
                    created_at: row.created_at,
                    last_used_at: envelope.last_used_at(),
                })
            })
            .collect()
    }
}

fn decode_passkeys(rows: &[PasskeyRow]) -> Result<Vec<Passkey>> {
    rows.iter()
        .map(|row| PasskeyEnvelope::parse(&row.envelope_json)?.passkey())
        .collect()
}

fn decode_credential_id(credential_id_b64: &str) -> Result<Vec<u8>> {
    STANDARD
        .decode(credential_id_b64)
        .map_err(|_| Error::Internal {
            message: "stored credential id is not base64".to_owned(),
        })
}

/// The one generic authentication failure: existence is never confirmed or
/// denied beyond what the protocol unavoidably reveals.
fn authentication_failed() -> Error {
    Error::InvalidInput {
        field: "credentials".to_owned(),
        message: "passkey authentication failed".to_owned(),
    }
}
