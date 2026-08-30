//! Verify-before-create public-seed promotion orchestration.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use sha2::{Digest as _, Sha256};

use super::policy::{
    AdmissionRequest, PromotionPolicyState, ReservationKey, RouteComponentKey, checked_deadline,
};
use super::{PromotionError, PromotionErrorKind, PromotionLimits, TrustedPromotionContext};
use crate::clock::Clock;
use crate::crypto::SnapshotKeyRing;
use crate::identity::{
    BrowserNonce, ContentDigest, IdempotencyKey, InstanceId, Revision, UnixMillis,
};
use crate::ledger::{InstanceAuthority, LiveInstanceLedger, PromotionOutcome, PromotionRecord};
use crate::random::InstanceIdGenerator;
use crate::snapshot::{GenerationMemo, SnapshotLimits, VerifiedSeedV1, verify_seed};

const PROMOTION_DIGEST_DOMAIN: &[u8] = b"suprnova-live/promotion-request/v1";

/// Whether the first action must refresh authoritative component data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefreshBeforeAction {
    /// The component opted into authoritative refresh after seed promotion.
    Required,
    /// The verified public seed may supply initial state to the first action.
    NotRequired,
}

/// Newly promoted authority and an engine-internal verified public-seed capability.
pub struct PromotedInstance {
    authority: InstanceAuthority,
    verified_seed: VerifiedSeedV1,
    refresh_before_action: RefreshBeforeAction,
}

impl PromotedInstance {
    /// Returns the server-assigned instance identity.
    #[must_use]
    pub const fn instance_id(&self) -> &InstanceId {
        self.authority.instance_id()
    }

    /// Returns the initial authoritative revision.
    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.authority.revision()
    }

    /// Returns the exclusive instance expiry deadline.
    #[must_use]
    pub const fn expires_at(&self) -> UnixMillis {
        self.authority.expires_at()
    }

    /// Returns the component's typed first-action refresh decision.
    #[must_use]
    pub const fn refresh_before_action(&self) -> RefreshBeforeAction {
        self.refresh_before_action
    }

    /// Returns verified dependency observations that remain advisory, never authority.
    #[must_use]
    pub fn advisory_generations(&self) -> &[GenerationMemo] {
        self.verified_seed.body().advisory_generations()
    }

    pub(crate) fn into_parts(self) -> (InstanceAuthority, VerifiedSeedV1, RefreshBeforeAction) {
        (
            self.authority,
            self.verified_seed,
            self.refresh_before_action,
        )
    }
}

impl fmt::Debug for PromotedInstance {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<PromotedInstance:redacted>")
    }
}

/// Bounded service that promotes verified public seeds into ledger authority.
pub struct PromotionService {
    ledger: Arc<dyn LiveInstanceLedger>,
    clock: Arc<dyn Clock>,
    instance_ids: Arc<dyn InstanceIdGenerator>,
    keys: Arc<SnapshotKeyRing>,
    snapshot_limits: SnapshotLimits,
    promotion_limits: PromotionLimits,
    policy: Mutex<PromotionPolicyState>,
}

impl PromotionService {
    /// Creates a promotion service after validating cross-boundary limits.
    pub fn new(
        ledger: Arc<dyn LiveInstanceLedger>,
        clock: Arc<dyn Clock>,
        instance_ids: Arc<dyn InstanceIdGenerator>,
        keys: Arc<SnapshotKeyRing>,
        snapshot_limits: SnapshotLimits,
        promotion_limits: PromotionLimits,
    ) -> Result<Self, PromotionError> {
        if promotion_limits.max_seed_bytes() > snapshot_limits.input().max_bytes()
            || promotion_limits.instance_lifetime_ms() > snapshot_limits.max_instance_lifetime_ms()
        {
            return Err(PromotionError::new(
                PromotionErrorKind::InvalidConfiguration,
            ));
        }
        Ok(Self {
            ledger,
            clock,
            instance_ids,
            keys,
            snapshot_limits,
            promotion_limits,
            policy: Mutex::new(PromotionPolicyState::new()),
        })
    }

    /// Verifies a reusable seed and atomically promotes it into scoped authority.
    ///
    /// Integrity, compatibility, trusted binding, byte, and nonce checks occur
    /// before random identity generation, policy reservation, or ledger writes.
    pub async fn promote(
        &self,
        encoded_seed: &[u8],
        browser_nonce: BrowserNonce,
        context: &TrustedPromotionContext,
    ) -> Result<PromotedInstance, PromotionError> {
        if encoded_seed.len() > self.promotion_limits.max_seed_bytes() {
            return Err(PromotionError::new(PromotionErrorKind::InputTooLarge));
        }
        let now = self
            .clock
            .now()
            .map_err(|_| PromotionError::new(PromotionErrorKind::ProviderInvariant))?;
        if !context.ensure_current(now) {
            return Err(PromotionError::new(PromotionErrorKind::ContextRejected));
        }
        let verified = verify_seed(
            encoded_seed,
            &context.expected_seed,
            &self.keys,
            now,
            &self.snapshot_limits,
        )
        .map_err(|_| PromotionError::new(PromotionErrorKind::SnapshotRejected))?;

        let idempotency_key = IdempotencyKey::from_bytes(browser_nonce.as_bytes())
            .map_err(|_| PromotionError::new(PromotionErrorKind::ProviderInvariant))?;
        let request_digest = promotion_digest(encoded_seed, &browser_nonce)?;
        let proposed_instance_id = self
            .instance_ids
            .generate()
            .map_err(|_| PromotionError::new(PromotionErrorKind::RandomUnavailable))?;
        let expires_at = checked_deadline(now, self.promotion_limits.instance_lifetime_ms())?;
        let lease_expires_at = checked_deadline(now, self.promotion_limits.promotion_lease_ms())?;
        let abandon_expires_at = checked_deadline(
            lease_expires_at,
            self.promotion_limits.abandoned_retention_ms(),
        )?;
        let reservation_key = ReservationKey {
            scope: context.scope.clone(),
            idempotency_key: idempotency_key.clone(),
        };
        let route_component = RouteComponentKey {
            scope: context.scope.clone(),
            route: verified.body().route().clone(),
            component: verified.body().component().name().clone(),
        };
        let admission = self.lock_policy()?.begin(
            AdmissionRequest {
                reservation_key: reservation_key.clone(),
                route_component,
                request_digest: request_digest.clone(),
                proposed_instance_id,
                expires_at,
                lease_expires_at,
                abandon_expires_at,
            },
            now,
            self.promotion_limits,
        )?;

        let ledger_outcome = self
            .ledger
            .promote(PromotionRecord::new(
                context.scope.clone(),
                admission.instance_id,
                idempotency_key,
                request_digest.clone(),
                Revision::new(0),
                admission.expires_at,
            ))
            .await;
        let authority = match ledger_outcome {
            Ok(PromotionOutcome::Created(authority) | PromotionOutcome::Existing(authority)) => {
                authority
            }
            Ok(PromotionOutcome::IdempotencyConflict) => {
                self.abandon(&reservation_key, &request_digest)?;
                return Err(PromotionError::new(PromotionErrorKind::NonceConflict));
            }
            Err(_) => {
                self.abandon(&reservation_key, &request_digest)?;
                return Err(PromotionError::new(PromotionErrorKind::LedgerUnavailable));
            }
        };
        let completed_at = match self.clock.now() {
            Ok(completed_at) => completed_at,
            Err(_) => {
                self.abandon(&reservation_key, &request_digest)?;
                return Err(PromotionError::new(PromotionErrorKind::ProviderInvariant));
            }
        };
        if completed_at < now
            || authority.revision() != Revision::new(0)
            || authority.expires_at() <= completed_at
            || authority
                .expires_at()
                .get()
                .saturating_sub(completed_at.get())
                > self.promotion_limits.instance_lifetime_ms()
        {
            self.abandon(&reservation_key, &request_digest)?;
            return Err(PromotionError::new(PromotionErrorKind::ProviderInvariant));
        }
        if !self.lock_policy()?.accept(
            &reservation_key,
            &request_digest,
            authority.instance_id().clone(),
            authority.expires_at(),
            completed_at,
        ) {
            self.abandon(&reservation_key, &request_digest)?;
            return Err(PromotionError::new(PromotionErrorKind::ProviderInvariant));
        }

        let refresh_before_action = if verified.body().refresh_on_promote() {
            RefreshBeforeAction::Required
        } else {
            RefreshBeforeAction::NotRequired
        };
        Ok(PromotedInstance {
            authority,
            verified_seed: verified,
            refresh_before_action,
        })
    }

    fn abandon(
        &self,
        reservation_key: &ReservationKey,
        request_digest: &ContentDigest,
    ) -> Result<(), PromotionError> {
        self.lock_policy()?.abandon(reservation_key, request_digest);
        Ok(())
    }

    fn lock_policy(&self) -> Result<MutexGuard<'_, PromotionPolicyState>, PromotionError> {
        self.policy
            .lock()
            .map_err(|_| PromotionError::new(PromotionErrorKind::ProviderInvariant))
    }
}

fn promotion_digest(
    encoded_seed: &[u8],
    browser_nonce: &BrowserNonce,
) -> Result<ContentDigest, PromotionError> {
    let seed_length = u64::try_from(encoded_seed.len())
        .map_err(|_| PromotionError::new(PromotionErrorKind::InputTooLarge))?;
    let mut digest = Sha256::new();
    digest.update(PROMOTION_DIGEST_DOMAIN);
    digest.update(seed_length.to_be_bytes());
    digest.update(encoded_seed);
    digest.update(browser_nonce.as_bytes());
    ContentDigest::from_bytes(&digest.finalize())
        .map_err(|_| PromotionError::new(PromotionErrorKind::ProviderInvariant))
}
