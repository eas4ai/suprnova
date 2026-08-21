//! Bounded in-process promotion admission and retry reservations.

use std::collections::HashMap;

use crate::identity::{
    ComponentName, ContentDigest, IdempotencyKey, InstanceId, RouteIdentity, ScopeFingerprint,
    UnixMillis,
};

use super::{PromotionError, PromotionErrorKind};

const MAX_SEED_BYTES: usize = 16 * 1024 * 1024;
const MAX_WINDOW_MS: u64 = 3_600_000;
const MAX_PROMOTIONS_PER_WINDOW: usize = 100_000;
const MAX_OUTSTANDING: usize = 100_000;
const MAX_PROMOTION_LEASE_MS: u64 = 60_000;
const MAX_ABANDONED_RETENTION_MS: u64 = 86_400_000;
const MAX_INSTANCE_LIFETIME_MS: u64 = 604_800_000;
const MAX_POLICY_ENTRIES: usize = 1_000_000;

/// Raw promotion policy values validated by [`PromotionLimits`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PromotionLimitConfig {
    /// Maximum encoded seed bytes accepted before snapshot parsing.
    pub max_seed_bytes: usize,
    /// Fixed rate-window duration in milliseconds.
    pub window_ms: u64,
    /// Maximum new nonce admissions per scope and fixed window.
    pub max_promotions_per_window: usize,
    /// Maximum pending or accepted instances per scope.
    pub max_outstanding_per_scope: usize,
    /// Maximum pending or accepted instances per scoped route/component pair.
    pub max_outstanding_per_route_component: usize,
    /// Maximum time a cancelled in-progress reservation remains pending.
    pub promotion_lease_ms: u64,
    /// Retention after failed or cancelled promotion before nonce reuse.
    pub abandoned_retention_ms: u64,
    /// Lifetime assigned to newly promoted instance authority.
    pub instance_lifetime_ms: u64,
    /// Maximum pending, accepted, and abandoned reservations.
    pub max_reservations: usize,
    /// Maximum concurrently retained scope rate buckets.
    pub max_rate_buckets: usize,
}

/// Validated bounded promotion admission policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PromotionLimits(PromotionLimitConfig);

impl PromotionLimits {
    /// Validates all promotion counts, durations, and storage cardinalities.
    pub fn new(config: PromotionLimitConfig) -> Result<Self, PromotionError> {
        let valid = config.max_seed_bytes > 0
            && config.max_seed_bytes <= MAX_SEED_BYTES
            && config.window_ms > 0
            && config.window_ms <= MAX_WINDOW_MS
            && config.max_promotions_per_window > 0
            && config.max_promotions_per_window <= MAX_PROMOTIONS_PER_WINDOW
            && config.max_outstanding_per_scope > 0
            && config.max_outstanding_per_scope <= MAX_OUTSTANDING
            && config.max_outstanding_per_route_component > 0
            && config.max_outstanding_per_route_component <= MAX_OUTSTANDING
            && config.promotion_lease_ms > 0
            && config.promotion_lease_ms <= MAX_PROMOTION_LEASE_MS
            && config.abandoned_retention_ms > 0
            && config.abandoned_retention_ms <= MAX_ABANDONED_RETENTION_MS
            && config.instance_lifetime_ms > 0
            && config.instance_lifetime_ms <= MAX_INSTANCE_LIFETIME_MS
            && config.max_reservations > 0
            && config.max_reservations <= MAX_POLICY_ENTRIES
            && config.max_rate_buckets > 0
            && config.max_rate_buckets <= MAX_POLICY_ENTRIES;
        if !valid {
            return Err(PromotionError::new(
                PromotionErrorKind::InvalidConfiguration,
            ));
        }
        Ok(Self(config))
    }

    pub(crate) const fn max_seed_bytes(self) -> usize {
        self.0.max_seed_bytes
    }

    pub(crate) const fn window_ms(self) -> u64 {
        self.0.window_ms
    }

    pub(crate) const fn max_promotions_per_window(self) -> usize {
        self.0.max_promotions_per_window
    }

    pub(crate) const fn max_outstanding_per_scope(self) -> usize {
        self.0.max_outstanding_per_scope
    }

    pub(crate) const fn max_outstanding_per_route_component(self) -> usize {
        self.0.max_outstanding_per_route_component
    }

    pub(crate) const fn promotion_lease_ms(self) -> u64 {
        self.0.promotion_lease_ms
    }

    pub(crate) const fn abandoned_retention_ms(self) -> u64 {
        self.0.abandoned_retention_ms
    }

    pub(crate) const fn instance_lifetime_ms(self) -> u64 {
        self.0.instance_lifetime_ms
    }

    pub(crate) const fn max_reservations(self) -> usize {
        self.0.max_reservations
    }

    pub(crate) const fn max_rate_buckets(self) -> usize {
        self.0.max_rate_buckets
    }
}

#[derive(Clone, Eq, Hash, PartialEq)]
pub(crate) struct ReservationKey {
    pub(crate) scope: ScopeFingerprint,
    pub(crate) idempotency_key: IdempotencyKey,
}

#[derive(Clone, Eq, Hash, PartialEq)]
pub(crate) struct RouteComponentKey {
    pub(crate) scope: ScopeFingerprint,
    pub(crate) route: RouteIdentity,
    pub(crate) component: ComponentName,
}

pub(crate) struct AdmissionRequest {
    pub(crate) reservation_key: ReservationKey,
    pub(crate) route_component: RouteComponentKey,
    pub(crate) request_digest: ContentDigest,
    pub(crate) proposed_instance_id: InstanceId,
    pub(crate) expires_at: UnixMillis,
    pub(crate) lease_expires_at: UnixMillis,
    pub(crate) abandon_expires_at: UnixMillis,
}

pub(crate) struct Admission {
    pub(crate) instance_id: InstanceId,
    pub(crate) expires_at: UnixMillis,
}

enum ReservationStatus {
    Pending {
        lease_expires_at: UnixMillis,
        abandon_expires_at: UnixMillis,
    },
    Accepted,
    Abandoned {
        retain_until: UnixMillis,
    },
}

struct Reservation {
    route_component: RouteComponentKey,
    request_digest: ContentDigest,
    instance_id: InstanceId,
    expires_at: UnixMillis,
    status: ReservationStatus,
}

struct RateBucket {
    window_expires_at: UnixMillis,
    count: usize,
}

pub(crate) struct PromotionPolicyState {
    reservations: HashMap<ReservationKey, Reservation>,
    rate_buckets: HashMap<ScopeFingerprint, RateBucket>,
}

impl PromotionPolicyState {
    pub(crate) fn new() -> Self {
        Self {
            reservations: HashMap::new(),
            rate_buckets: HashMap::new(),
        }
    }

    pub(crate) fn begin(
        &mut self,
        request: AdmissionRequest,
        now: UnixMillis,
        limits: PromotionLimits,
    ) -> Result<Admission, PromotionError> {
        self.prune(now);
        if let Some(existing) = self.reservations.get(&request.reservation_key) {
            if existing.request_digest != request.request_digest {
                return Err(PromotionError::new(PromotionErrorKind::NonceConflict));
            }
            return match existing.status {
                ReservationStatus::Pending { .. } => {
                    Err(PromotionError::new(PromotionErrorKind::InProgress))
                }
                ReservationStatus::Accepted => Ok(Admission {
                    instance_id: existing.instance_id.clone(),
                    expires_at: existing.expires_at,
                }),
                ReservationStatus::Abandoned { .. } => {
                    Err(PromotionError::new(PromotionErrorKind::AbandonedRetention))
                }
            };
        }

        if self.reservations.len() >= limits.max_reservations() {
            return Err(PromotionError::new(PromotionErrorKind::StorageLimit));
        }
        self.consume_rate(&request.reservation_key.scope, now, limits)?;

        let outstanding_for_scope = self
            .reservations
            .iter()
            .filter(|(key, reservation)| {
                key.scope == request.reservation_key.scope
                    && matches!(
                        reservation.status,
                        ReservationStatus::Pending { .. } | ReservationStatus::Accepted
                    )
            })
            .count();
        if outstanding_for_scope >= limits.max_outstanding_per_scope() {
            return Err(PromotionError::new(PromotionErrorKind::OutstandingLimit));
        }
        let outstanding_for_route = self
            .reservations
            .values()
            .filter(|reservation| {
                reservation.route_component == request.route_component
                    && matches!(
                        reservation.status,
                        ReservationStatus::Pending { .. } | ReservationStatus::Accepted
                    )
            })
            .count();
        if outstanding_for_route >= limits.max_outstanding_per_route_component() {
            return Err(PromotionError::new(PromotionErrorKind::RouteComponentLimit));
        }

        let admission = Admission {
            instance_id: request.proposed_instance_id.clone(),
            expires_at: request.expires_at,
        };
        self.reservations.insert(
            request.reservation_key,
            Reservation {
                route_component: request.route_component,
                request_digest: request.request_digest,
                instance_id: request.proposed_instance_id,
                expires_at: request.expires_at,
                status: ReservationStatus::Pending {
                    lease_expires_at: request.lease_expires_at,
                    abandon_expires_at: request.abandon_expires_at,
                },
            },
        );
        Ok(admission)
    }

    pub(crate) fn accept(
        &mut self,
        key: &ReservationKey,
        request_digest: &ContentDigest,
        instance_id: InstanceId,
        expires_at: UnixMillis,
    ) -> bool {
        let Some(reservation) = self.reservations.get_mut(key) else {
            return false;
        };
        if reservation.request_digest != *request_digest {
            return false;
        }
        reservation.instance_id = instance_id;
        reservation.expires_at = expires_at;
        reservation.status = ReservationStatus::Accepted;
        true
    }

    pub(crate) fn abandon(&mut self, key: &ReservationKey, request_digest: &ContentDigest) {
        let Some(reservation) = self.reservations.get_mut(key) else {
            return;
        };
        if reservation.request_digest != *request_digest {
            return;
        }
        if let ReservationStatus::Pending {
            abandon_expires_at, ..
        } = reservation.status
        {
            reservation.status = ReservationStatus::Abandoned {
                retain_until: abandon_expires_at,
            };
        }
    }

    fn consume_rate(
        &mut self,
        scope: &ScopeFingerprint,
        now: UnixMillis,
        limits: PromotionLimits,
    ) -> Result<(), PromotionError> {
        if let Some(bucket) = self.rate_buckets.get_mut(scope) {
            if bucket.count >= limits.max_promotions_per_window() {
                return Err(PromotionError::new(PromotionErrorKind::RateLimited));
            }
            bucket.count += 1;
            return Ok(());
        }
        if self.rate_buckets.len() >= limits.max_rate_buckets() {
            return Err(PromotionError::new(PromotionErrorKind::StorageLimit));
        }
        let window_expires_at = checked_deadline(now, limits.window_ms())?;
        self.rate_buckets.insert(
            scope.clone(),
            RateBucket {
                window_expires_at,
                count: 1,
            },
        );
        Ok(())
    }

    fn prune(&mut self, now: UnixMillis) {
        self.rate_buckets
            .retain(|_, bucket| bucket.window_expires_at > now);
        self.reservations
            .retain(|_, reservation| match &mut reservation.status {
                ReservationStatus::Pending {
                    lease_expires_at,
                    abandon_expires_at,
                } if *lease_expires_at <= now => {
                    if *abandon_expires_at <= now {
                        return false;
                    }
                    reservation.status = ReservationStatus::Abandoned {
                        retain_until: *abandon_expires_at,
                    };
                    true
                }
                ReservationStatus::Accepted => reservation.expires_at > now,
                ReservationStatus::Abandoned { retain_until } => *retain_until > now,
                ReservationStatus::Pending { .. } => true,
            });
    }
}

pub(crate) fn checked_deadline(
    now: UnixMillis,
    duration_ms: u64,
) -> Result<UnixMillis, PromotionError> {
    now.get()
        .checked_add(duration_ms)
        .map(UnixMillis::new)
        .ok_or_else(|| PromotionError::new(PromotionErrorKind::InvalidConfiguration))
}
