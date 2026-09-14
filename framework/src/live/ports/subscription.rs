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
/// The Gate ability and resource a stream subscription is authorized
/// against: `live:{component}.stream.{stream}` over `{component}::{stream}`.
/// One definition for issuance and for delivery (LIVE-016), so the two can
/// never drift apart.
pub(crate) fn stream_ability(
    component: &suprnova_live::identity::ComponentName,
    stream: &suprnova_live::async_updates::StreamName,
) -> (String, String) {
    (
        format!("live:{}.stream.{}", component.as_str(), stream.as_str()),
        format!("{}::{}", component.as_str(), stream.as_str()),
    )
}

pub(crate) struct SuprnovaSubscriptionAuthorization;

impl SubscriptionAuthorizationPort for SuprnovaSubscriptionAuthorization {
    fn authorize<'a>(
        &'a self,
        request: SubscriptionAuthorizationRequest<'a>,
    ) -> SubscriptionFuture<'a, Result<SubscriptionAuthorizationDecision, SubscriptionError>> {
        let (ability, resource) = stream_ability(request.component(), request.stream());
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

/// Every unconsumed secret issued for one descriptor binding, oldest first.
///
/// Concurrent issuances of one scope in the same millisecond mint identical
/// descriptors and therefore one binding; each needs its own secret kept
/// until it is consumed or expires, or the earlier issuances cannot connect
/// (LIVE-022).
type CredentialTable = HashMap<String, Vec<CredentialEntry>>;

/// In-process descriptor-scoped credential store with atomic rotation.
///
/// Credentials never leave this process, so a restart invalidates every
/// outstanding subscription and browsers issue afresh.
#[derive(Default)]
pub(crate) struct SuprnovaSubscriptionCredentials {
    entries: Mutex<CredentialTable>,
}

impl SuprnovaSubscriptionCredentials {
    fn lock(&self) -> std::sync::MutexGuard<'_, CredentialTable> {
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
            if secrets_held(&entries) >= MAX_CREDENTIAL_ENTRIES {
                return Err(SubscriptionError::new(
                    SubscriptionErrorKind::CredentialUnavailable,
                ));
            }
            entries.entry(key).or_default().push(CredentialEntry {
                secret,
                expires_at: request.expires_at(),
            });
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
            if !consume(
                &mut entries,
                &predecessor_key,
                presented.expose_authorization_bearer(),
            ) {
                return SubscriptionCredentialRotationOutcome::Reject;
            }
            entries
                .entry(successor_key)
                .or_default()
                .push(CredentialEntry {
                    secret,
                    expires_at: successor.expires_at(),
                });
            SubscriptionCredentialRotationOutcome::Rotated(credential)
        })
    }
}

/// Removes the one secret under `key` that matches `presented`, reporting
/// whether there was one. Every secret is compared, so the time taken does
/// not depend on which of a binding's secrets matched.
fn consume(entries: &mut CredentialTable, key: &str, presented: &[u8]) -> bool {
    let Some(secrets) = entries.get_mut(key) else {
        return false;
    };
    let matched = secrets
        .iter()
        .map(|entry| same_secret(&entry.secret, presented))
        .collect::<Vec<_>>();
    let Some(index) = matched.iter().position(|hit| *hit) else {
        return false;
    };
    secrets.remove(index);
    if secrets.is_empty() {
        entries.remove(key);
    }
    true
}

fn prune(entries: &mut CredentialTable, now: UnixMillis) {
    entries.retain(|_, secrets| {
        secrets.retain(|entry| entry.expires_at > now);
        !secrets.is_empty()
    });
}

/// The number of unconsumed secrets across every binding, which the entry
/// cap bounds.
fn secrets_held(entries: &CredentialTable) -> usize {
    entries.values().map(Vec::len).sum()
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
