//! Framework-owned adapters for the engine's asynchronous subscription ports.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};
use suprnova_live::async_updates::{
    AuthoritativeStreamPosition, CurrentSubscriptionRegistration, StreamPosition,
    SubscriptionAuthorizationDecision, SubscriptionAuthorizationPort,
    SubscriptionAuthorizationRequest, SubscriptionBaselineRequest, SubscriptionContinuityPort,
    SubscriptionCredentialPort, SubscriptionCredentialRequest,
    SubscriptionCredentialRotationOutcome, SubscriptionCredentialRotationRequest,
    SubscriptionError, SubscriptionErrorKind, SubscriptionFuture, SubscriptionRegistryPort,
    SubscriptionRegistryRequest, TransportCredential, TrustedMountParameters,
};
use suprnova_live::identity::UnixMillis;
use suprnova_live::registry::ComponentRegistry;

const CREDENTIAL_BYTES: usize = 32;
const MAX_CREDENTIAL_ENTRIES: usize = 65_536;

/// Current registry authority plus the trusted mount parameters of one request.
pub(crate) struct SuprnovaSubscriptionRegistry {
    registry: Arc<ComponentRegistry>,
    parameters: TrustedMountParameters,
}

impl SuprnovaSubscriptionRegistry {
    pub(crate) const fn new(
        registry: Arc<ComponentRegistry>,
        parameters: TrustedMountParameters,
    ) -> Self {
        Self {
            registry,
            parameters,
        }
    }
}

impl SubscriptionRegistryPort for SuprnovaSubscriptionRegistry {
    fn resolve<'a>(
        &'a self,
        request: SubscriptionRegistryRequest<'a>,
    ) -> SubscriptionFuture<'a, Result<CurrentSubscriptionRegistration, SubscriptionError>> {
        Box::pin(async move {
            let descriptor = self
                .registry
                .resolve(request.component())
                .map_err(|_| unregistered())?;
            if descriptor.contract_digest() != request.component_contract() {
                return Err(unregistered());
            }
            CurrentSubscriptionRegistration::from_registered(
                descriptor.metadata(),
                request.stream(),
                &self.parameters,
            )
        })
    }
}

/// Suprnova Gate adaptation for registered component streams.
pub(crate) struct SuprnovaSubscriptionAuthorization;

impl SubscriptionAuthorizationPort for SuprnovaSubscriptionAuthorization {
    fn authorize<'a>(
        &'a self,
        request: SubscriptionAuthorizationRequest<'a>,
    ) -> SubscriptionFuture<'a, Result<SubscriptionAuthorizationDecision, SubscriptionError>> {
        let ability = format!(
            "live:{}.stream.{}",
            request.component().as_str(),
            request.stream().as_str()
        );
        let resource = format!(
            "{}::{}",
            request.component().as_str(),
            request.stream().as_str()
        );
        Box::pin(async move {
            let Some(principal) = crate::auth::guard::Auth::id() else {
                return Ok(SubscriptionAuthorizationDecision::Deny);
            };
            let allowed =
                crate::authorization::Gate::allows_async(&ability, &principal, &resource).await;
            Ok(if allowed {
                SubscriptionAuthorizationDecision::Allow
            } else {
                SubscriptionAuthorizationDecision::Deny
            })
        })
    }
}

/// Continuity authority that reports the position the runtime already validated.
pub(crate) struct FixedSubscriptionBaseline(StreamPosition);

impl FixedSubscriptionBaseline {
    pub(crate) const fn new(position: StreamPosition) -> Self {
        Self(position)
    }
}

impl SubscriptionContinuityPort for FixedSubscriptionBaseline {
    fn authoritative_baseline<'a>(
        &'a self,
        _request: SubscriptionBaselineRequest<'a>,
    ) -> SubscriptionFuture<'a, Result<AuthoritativeStreamPosition, SubscriptionError>> {
        let position = self.0;
        Box::pin(async move { Ok(AuthoritativeStreamPosition::from_host_continuity(position)) })
    }
}

struct CredentialEntry {
    secret: Vec<u8>,
    expires_at: UnixMillis,
}

/// In-process descriptor-scoped credential store with atomic rotation.
///
/// Credentials never leave this process, so a restart invalidates every
/// outstanding subscription and browsers issue afresh.
#[derive(Default)]
pub(crate) struct SuprnovaSubscriptionCredentials {
    entries: Mutex<HashMap<String, CredentialEntry>>,
}

impl SuprnovaSubscriptionCredentials {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, CredentialEntry>> {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl SubscriptionCredentialPort for SuprnovaSubscriptionCredentials {
    fn issue<'a>(
        &'a self,
        request: SubscriptionCredentialRequest<'a>,
    ) -> SubscriptionFuture<'a, Result<TransportCredential, SubscriptionError>> {
        Box::pin(async move {
            let key = request.binding().to_base64url();
            let secret = mint_secret();
            let credential = TransportCredential::from_host_authority_bearer(secret.clone())?;
            let mut entries = self.lock();
            prune(&mut entries, request.now());
            if entries.len() >= MAX_CREDENTIAL_ENTRIES && !entries.contains_key(&key) {
                return Err(SubscriptionError::new(
                    SubscriptionErrorKind::CredentialUnavailable,
                ));
            }
            entries.insert(
                key,
                CredentialEntry {
                    secret,
                    expires_at: request.expires_at(),
                },
            );
            Ok(credential)
        })
    }

    fn consume_and_rotate<'a>(
        &'a self,
        request: SubscriptionCredentialRotationRequest<'a>,
    ) -> SubscriptionFuture<'a, SubscriptionCredentialRotationOutcome> {
        Box::pin(async move {
            let predecessor = request.predecessor();
            let successor = request.successor();
            let Some(presented) = predecessor.presented() else {
                return SubscriptionCredentialRotationOutcome::Reject;
            };
            let predecessor_key = predecessor.binding().to_base64url();
            let successor_key = successor.binding().to_base64url();
            let secret = mint_secret();
            let Ok(credential) = TransportCredential::from_host_authority_bearer(secret.clone())
            else {
                return SubscriptionCredentialRotationOutcome::Failed;
            };
            let mut entries = self.lock();
            prune(&mut entries, predecessor.now());
            let valid = entries.get(&predecessor_key).is_some_and(|entry| {
                entry.expires_at > predecessor.now()
                    && same_secret(&entry.secret, presented.expose_authorization_bearer())
            });
            if !valid {
                return SubscriptionCredentialRotationOutcome::Reject;
            }
            entries.remove(&predecessor_key);
            entries.insert(
                successor_key,
                CredentialEntry {
                    secret,
                    expires_at: successor.expires_at(),
                },
            );
            SubscriptionCredentialRotationOutcome::Rotated(credential)
        })
    }
}

fn prune(entries: &mut HashMap<String, CredentialEntry>, now: UnixMillis) {
    entries.retain(|_, entry| entry.expires_at > now);
}

fn mint_secret() -> Vec<u8> {
    let bytes: [u8; CREDENTIAL_BYTES] = rand::random();
    bytes.to_vec()
}

/// Compares two secrets through their digests so timing reveals nothing about the bytes.
fn same_secret(stored: &[u8], presented: &[u8]) -> bool {
    let stored: [u8; 32] = Sha256::digest(stored).into();
    let presented: [u8; 32] = Sha256::digest(presented).into();
    stored == presented
}

fn unregistered() -> SubscriptionError {
    SubscriptionError::new(SubscriptionErrorKind::UnregisteredSubscription)
}
