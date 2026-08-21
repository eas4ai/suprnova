//! Bounded in-process promotion admission and retry reservations.

use std::collections::{BTreeMap, HashMap};

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
const MAX_EXPIRED_EVENTS_PER_PRUNE: usize = 64;

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

#[derive(Clone, Copy, Eq, PartialEq)]
enum ReservationDeadlineKind {
    PendingLease,
    Accepted,
    Abandoned,
}

struct ReservationDeadline {
    key: ReservationKey,
    kind: ReservationDeadlineKind,
}

enum DeadlineDisposition {
    Ignore,
    Abandon(UnixMillis),
    RemoveOutstanding,
    RemoveRetained,
}

pub(crate) struct PromotionPolicyState {
    reservations: HashMap<ReservationKey, Reservation>,
    rate_buckets: HashMap<ScopeFingerprint, RateBucket>,
    reservation_deadlines: BTreeMap<UnixMillis, Vec<ReservationDeadline>>,
    rate_deadlines: BTreeMap<UnixMillis, Vec<ScopeFingerprint>>,
    outstanding_by_scope: HashMap<ScopeFingerprint, usize>,
    outstanding_by_route_component: HashMap<RouteComponentKey, usize>,
}

impl PromotionPolicyState {
    pub(crate) fn new() -> Self {
        Self {
            reservations: HashMap::new(),
            rate_buckets: HashMap::new(),
            reservation_deadlines: BTreeMap::new(),
            rate_deadlines: BTreeMap::new(),
            outstanding_by_scope: HashMap::new(),
            outstanding_by_route_component: HashMap::new(),
        }
    }

    pub(crate) fn begin(
        &mut self,
        request: AdmissionRequest,
        now: UnixMillis,
        limits: PromotionLimits,
    ) -> Result<Admission, PromotionError> {
        self.prune(now);
        self.prune_requested_reservation(&request.reservation_key, now);
        self.prune_requested_rate_bucket(&request.reservation_key.scope, now);
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

        if self.outstanding_for_scope(&request.reservation_key.scope)
            >= limits.max_outstanding_per_scope()
        {
            return Err(PromotionError::new(PromotionErrorKind::OutstandingLimit));
        }
        if self.outstanding_for_route(&request.route_component)
            >= limits.max_outstanding_per_route_component()
        {
            return Err(PromotionError::new(PromotionErrorKind::RouteComponentLimit));
        }

        let admission = Admission {
            instance_id: request.proposed_instance_id.clone(),
            expires_at: request.expires_at,
        };
        let reservation_key = request.reservation_key;
        let route_component = request.route_component;
        let lease_expires_at = request.lease_expires_at;
        self.increment_outstanding(&reservation_key.scope, &route_component);
        self.reservations.insert(
            reservation_key.clone(),
            Reservation {
                route_component,
                request_digest: request.request_digest,
                instance_id: request.proposed_instance_id,
                expires_at: request.expires_at,
                status: ReservationStatus::Pending {
                    lease_expires_at,
                    abandon_expires_at: request.abandon_expires_at,
                },
            },
        );
        self.schedule_reservation_deadline(
            lease_expires_at,
            reservation_key,
            ReservationDeadlineKind::PendingLease,
        );
        Ok(admission)
    }

    pub(crate) fn accept(
        &mut self,
        key: &ReservationKey,
        request_digest: &ContentDigest,
        instance_id: InstanceId,
        expires_at: UnixMillis,
        now: UnixMillis,
    ) -> bool {
        self.prune_requested_reservation(key, now);
        {
            let Some(reservation) = self.reservations.get_mut(key) else {
                return false;
            };
            if reservation.request_digest != *request_digest {
                return false;
            }
            if matches!(reservation.status, ReservationStatus::Accepted) {
                return reservation.instance_id == instance_id
                    && reservation.expires_at == expires_at;
            }
            if !matches!(reservation.status, ReservationStatus::Pending { .. }) {
                return false;
            }
            reservation.instance_id = instance_id;
            reservation.expires_at = expires_at;
            reservation.status = ReservationStatus::Accepted;
        }
        self.schedule_reservation_deadline(
            expires_at,
            key.clone(),
            ReservationDeadlineKind::Accepted,
        );
        true
    }

    pub(crate) fn abandon(&mut self, key: &ReservationKey, request_digest: &ContentDigest) {
        let abandoned = {
            let Some(reservation) = self.reservations.get_mut(key) else {
                return;
            };
            if reservation.request_digest != *request_digest {
                return;
            }
            let ReservationStatus::Pending {
                abandon_expires_at, ..
            } = reservation.status
            else {
                return;
            };
            let route_component = reservation.route_component.clone();
            reservation.status = ReservationStatus::Abandoned {
                retain_until: abandon_expires_at,
            };
            (route_component, abandon_expires_at)
        };
        self.decrement_outstanding(&key.scope, &abandoned.0);
        self.schedule_reservation_deadline(
            abandoned.1,
            key.clone(),
            ReservationDeadlineKind::Abandoned,
        );
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
        self.rate_deadlines
            .entry(window_expires_at)
            .or_default()
            .push(scope.clone());
        Ok(())
    }

    fn prune(&mut self, now: UnixMillis) {
        self.prune_rate_buckets(now);
        self.prune_reservations(now);
    }

    fn prune_rate_buckets(&mut self, now: UnixMillis) {
        for _ in 0..MAX_EXPIRED_EVENTS_PER_PRUNE {
            let Some((deadline, scope)) = self.pop_due_rate_deadline(now) else {
                break;
            };
            if self
                .rate_buckets
                .get(&scope)
                .is_some_and(|bucket| bucket.window_expires_at == deadline)
            {
                self.rate_buckets.remove(&scope);
            }
        }
    }

    fn prune_reservations(&mut self, now: UnixMillis) {
        for _ in 0..MAX_EXPIRED_EVENTS_PER_PRUNE {
            let Some((deadline, event)) = self.pop_due_reservation_deadline(now) else {
                break;
            };
            let disposition = self.deadline_disposition(&event, deadline, now);
            match disposition {
                DeadlineDisposition::Ignore => {}
                DeadlineDisposition::Abandon(retain_until) => {
                    let route_component = {
                        let Some(reservation) = self.reservations.get_mut(&event.key) else {
                            continue;
                        };
                        reservation.status = ReservationStatus::Abandoned { retain_until };
                        reservation.route_component.clone()
                    };
                    self.decrement_outstanding(&event.key.scope, &route_component);
                    self.schedule_reservation_deadline(
                        retain_until,
                        event.key,
                        ReservationDeadlineKind::Abandoned,
                    );
                }
                DeadlineDisposition::RemoveOutstanding => {
                    if let Some(reservation) = self.reservations.remove(&event.key) {
                        self.decrement_outstanding(&event.key.scope, &reservation.route_component);
                    }
                }
                DeadlineDisposition::RemoveRetained => {
                    self.reservations.remove(&event.key);
                }
            }
        }
    }

    fn prune_requested_reservation(&mut self, key: &ReservationKey, now: UnixMillis) {
        let disposition = match self
            .reservations
            .get(key)
            .map(|reservation| &reservation.status)
        {
            Some(ReservationStatus::Pending {
                lease_expires_at,
                abandon_expires_at,
            }) if *lease_expires_at <= now => {
                if *abandon_expires_at <= now {
                    DeadlineDisposition::RemoveOutstanding
                } else {
                    DeadlineDisposition::Abandon(*abandon_expires_at)
                }
            }
            Some(ReservationStatus::Accepted)
                if self
                    .reservations
                    .get(key)
                    .is_some_and(|reservation| reservation.expires_at <= now) =>
            {
                DeadlineDisposition::RemoveOutstanding
            }
            Some(ReservationStatus::Abandoned { retain_until }) if *retain_until <= now => {
                DeadlineDisposition::RemoveRetained
            }
            _ => DeadlineDisposition::Ignore,
        };
        match disposition {
            DeadlineDisposition::Ignore => {}
            DeadlineDisposition::Abandon(retain_until) => {
                let route_component = {
                    let Some(reservation) = self.reservations.get_mut(key) else {
                        return;
                    };
                    reservation.status = ReservationStatus::Abandoned { retain_until };
                    reservation.route_component.clone()
                };
                self.decrement_outstanding(&key.scope, &route_component);
                self.schedule_reservation_deadline(
                    retain_until,
                    key.clone(),
                    ReservationDeadlineKind::Abandoned,
                );
            }
            DeadlineDisposition::RemoveOutstanding => {
                if let Some(reservation) = self.reservations.remove(key) {
                    self.decrement_outstanding(&key.scope, &reservation.route_component);
                }
            }
            DeadlineDisposition::RemoveRetained => {
                self.reservations.remove(key);
            }
        }
    }

    fn prune_requested_rate_bucket(&mut self, scope: &ScopeFingerprint, now: UnixMillis) {
        if self
            .rate_buckets
            .get(scope)
            .is_some_and(|bucket| bucket.window_expires_at <= now)
        {
            self.rate_buckets.remove(scope);
        }
    }

    fn pop_due_rate_deadline(&mut self, now: UnixMillis) -> Option<(UnixMillis, ScopeFingerprint)> {
        loop {
            let mut entry = self.rate_deadlines.first_entry()?;
            let deadline = *entry.key();
            if deadline > now {
                return None;
            }
            let scope = entry.get_mut().pop();
            if entry.get().is_empty() {
                entry.remove();
            }
            if let Some(scope) = scope {
                return Some((deadline, scope));
            }
        }
    }

    fn pop_due_reservation_deadline(
        &mut self,
        now: UnixMillis,
    ) -> Option<(UnixMillis, ReservationDeadline)> {
        loop {
            let mut entry = self.reservation_deadlines.first_entry()?;
            let deadline = *entry.key();
            if deadline > now {
                return None;
            }
            let event = entry.get_mut().pop();
            if entry.get().is_empty() {
                entry.remove();
            }
            if let Some(event) = event {
                return Some((deadline, event));
            }
        }
    }

    fn deadline_disposition(
        &self,
        event: &ReservationDeadline,
        deadline: UnixMillis,
        now: UnixMillis,
    ) -> DeadlineDisposition {
        let Some(reservation) = self.reservations.get(&event.key) else {
            return DeadlineDisposition::Ignore;
        };
        match (&reservation.status, event.kind) {
            (
                ReservationStatus::Pending {
                    lease_expires_at,
                    abandon_expires_at,
                },
                ReservationDeadlineKind::PendingLease,
            ) if *lease_expires_at == deadline => {
                if *abandon_expires_at <= now {
                    DeadlineDisposition::RemoveOutstanding
                } else {
                    DeadlineDisposition::Abandon(*abandon_expires_at)
                }
            }
            (ReservationStatus::Accepted, ReservationDeadlineKind::Accepted)
                if reservation.expires_at == deadline =>
            {
                DeadlineDisposition::RemoveOutstanding
            }
            (ReservationStatus::Abandoned { retain_until }, ReservationDeadlineKind::Abandoned)
                if *retain_until == deadline =>
            {
                DeadlineDisposition::RemoveRetained
            }
            _ => DeadlineDisposition::Ignore,
        }
    }

    fn schedule_reservation_deadline(
        &mut self,
        deadline: UnixMillis,
        key: ReservationKey,
        kind: ReservationDeadlineKind,
    ) {
        self.reservation_deadlines
            .entry(deadline)
            .or_default()
            .push(ReservationDeadline { key, kind });
    }

    fn outstanding_for_scope(&self, scope: &ScopeFingerprint) -> usize {
        self.outstanding_by_scope.get(scope).copied().unwrap_or(0)
    }

    fn outstanding_for_route(&self, route_component: &RouteComponentKey) -> usize {
        self.outstanding_by_route_component
            .get(route_component)
            .copied()
            .unwrap_or(0)
    }

    fn increment_outstanding(
        &mut self,
        scope: &ScopeFingerprint,
        route_component: &RouteComponentKey,
    ) {
        *self.outstanding_by_scope.entry(scope.clone()).or_default() += 1;
        *self
            .outstanding_by_route_component
            .entry(route_component.clone())
            .or_default() += 1;
    }

    fn decrement_outstanding(
        &mut self,
        scope: &ScopeFingerprint,
        route_component: &RouteComponentKey,
    ) {
        let remove_scope = match self.outstanding_by_scope.get_mut(scope) {
            Some(count) if *count > 1 => {
                *count -= 1;
                false
            }
            Some(_) => true,
            None => false,
        };
        if remove_scope {
            self.outstanding_by_scope.remove(scope);
        }
        let remove_route = match self.outstanding_by_route_component.get_mut(route_component) {
            Some(count) if *count > 1 => {
                *count -= 1;
                false
            }
            Some(_) => true,
            None => false,
        };
        if remove_route {
            self.outstanding_by_route_component.remove(route_component);
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes<const N: usize>(start: u8) -> [u8; N] {
        std::array::from_fn(|offset| start.wrapping_add(offset as u8))
    }

    fn limits() -> PromotionLimits {
        PromotionLimits::new(PromotionLimitConfig {
            max_seed_bytes: 4_096,
            window_ms: 1_000,
            max_promotions_per_window: 8,
            max_outstanding_per_scope: 8,
            max_outstanding_per_route_component: 8,
            promotion_lease_ms: 100,
            abandoned_retention_ms: 200,
            instance_lifetime_ms: 1_000,
            max_reservations: 64,
            max_rate_buckets: 32,
        })
        .expect("test policy is valid")
    }

    fn admission(start: u8, scope: &ScopeFingerprint) -> AdmissionRequest {
        AdmissionRequest {
            reservation_key: ReservationKey {
                scope: scope.clone(),
                idempotency_key: IdempotencyKey::from_bytes(&bytes::<16>(start))
                    .expect("test idempotency key is valid"),
            },
            route_component: RouteComponentKey {
                scope: scope.clone(),
                route: RouteIdentity::from_bytes(&bytes::<32>(0x40)).expect("test route is valid"),
                component: ComponentName::parse("Search").expect("test component is valid"),
            },
            request_digest: ContentDigest::from_bytes(&bytes::<32>(start.wrapping_add(1)))
                .expect("test digest is valid"),
            proposed_instance_id: InstanceId::from_bytes(&bytes::<16>(start.wrapping_add(2)))
                .expect("test instance is valid"),
            expires_at: UnixMillis::new(200),
            lease_expires_at: UnixMillis::new(150),
            abandon_expires_at: UnixMillis::new(180),
        }
    }

    #[test]
    fn outstanding_indexes_follow_accept_abandon_and_expiry() {
        let scope = ScopeFingerprint::from_bytes(&bytes::<32>(0x20)).expect("test scope is valid");
        let route_component = admission(0x60, &scope).route_component;
        let mut state = PromotionPolicyState::new();

        let first = admission(0x60, &scope);
        let first_key = first.reservation_key.clone();
        let first_digest = first.request_digest.clone();
        let first_instance = first.proposed_instance_id.clone();
        state
            .begin(first, UnixMillis::new(100), limits())
            .expect("first admission succeeds");
        assert_eq!(state.outstanding_for_scope(&scope), 1);
        assert_eq!(state.outstanding_for_route(&route_component), 1);

        assert!(state.accept(
            &first_key,
            &first_digest,
            first_instance,
            UnixMillis::new(200),
            UnixMillis::new(100),
        ));
        assert_eq!(state.outstanding_for_scope(&scope), 1);

        let second = admission(0x80, &scope);
        let second_key = second.reservation_key.clone();
        let second_digest = second.request_digest.clone();
        state
            .begin(second, UnixMillis::new(100), limits())
            .expect("second admission succeeds");
        assert_eq!(state.outstanding_for_scope(&scope), 2);

        state.abandon(&second_key, &second_digest);
        assert_eq!(state.outstanding_for_scope(&scope), 1);
        assert_eq!(state.outstanding_for_route(&route_component), 1);

        state
            .begin(admission(0xa0, &scope), UnixMillis::new(201), limits())
            .expect("expired entries are pruned before admission");
        assert_eq!(state.outstanding_for_scope(&scope), 1);
        assert_eq!(state.outstanding_for_route(&route_component), 1);
    }

    #[test]
    fn accept_cannot_resurrect_a_reservation_after_its_lease() {
        let scope = ScopeFingerprint::from_bytes(&bytes::<32>(0x30)).expect("test scope is valid");
        let mut state = PromotionPolicyState::new();
        let first = admission(0x50, &scope);
        let first_key = first.reservation_key.clone();
        let first_digest = first.request_digest.clone();
        let first_instance = first.proposed_instance_id.clone();
        state
            .begin(first, UnixMillis::new(100), limits())
            .expect("first admission succeeds");

        state
            .begin(admission(0x70, &scope), UnixMillis::new(151), limits())
            .expect("a new admission prunes the expired lease");
        assert!(!state.accept(
            &first_key,
            &first_digest,
            first_instance,
            UnixMillis::new(300),
            UnixMillis::new(151),
        ));
        assert_eq!(state.outstanding_for_scope(&scope), 1);
    }

    #[test]
    fn accept_rejects_completion_after_lease_without_needing_another_admission() {
        let scope = ScopeFingerprint::from_bytes(&bytes::<32>(0x31)).expect("test scope is valid");
        let mut state = PromotionPolicyState::new();
        let request = admission(0x51, &scope);
        let key = request.reservation_key.clone();
        let digest = request.request_digest.clone();
        let instance = request.proposed_instance_id.clone();
        state
            .begin(request, UnixMillis::new(100), limits())
            .expect("admission succeeds");

        assert!(!state.accept(
            &key,
            &digest,
            instance,
            UnixMillis::new(300),
            UnixMillis::new(151),
        ));
        assert_eq!(state.outstanding_for_scope(&scope), 0);
    }

    #[test]
    fn one_admission_performs_only_bounded_expiry_cleanup() {
        let scope = ScopeFingerprint::from_bytes(&bytes::<32>(0x10)).expect("test scope is valid");
        let bounded_limits = PromotionLimits::new(PromotionLimitConfig {
            max_seed_bytes: 4_096,
            window_ms: 1_000,
            max_promotions_per_window: 1_000,
            max_outstanding_per_scope: 1_000,
            max_outstanding_per_route_component: 1_000,
            promotion_lease_ms: 100,
            abandoned_retention_ms: 200,
            instance_lifetime_ms: 1_000,
            max_reservations: 1_000,
            max_rate_buckets: 32,
        })
        .expect("test policy is valid");
        let mut state = PromotionPolicyState::new();
        for start in 0..(MAX_EXPIRED_EVENTS_PER_PRUNE + 2) {
            state
                .begin(
                    admission(start as u8, &scope),
                    UnixMillis::new(100),
                    bounded_limits,
                )
                .expect("setup admission succeeds");
        }

        state
            .begin(
                admission(0xf0, &scope),
                UnixMillis::new(201),
                bounded_limits,
            )
            .expect("new admission succeeds after bounded cleanup");

        assert_eq!(state.reservations.len(), 3);
        assert_eq!(state.outstanding_for_scope(&scope), 3);
    }

    #[test]
    fn requested_nonce_is_pruned_even_behind_the_cleanup_backlog() {
        let scope = ScopeFingerprint::from_bytes(&bytes::<32>(0x11)).expect("test scope is valid");
        let bounded_limits = PromotionLimits::new(PromotionLimitConfig {
            max_seed_bytes: 4_096,
            window_ms: 1_000,
            max_promotions_per_window: 1_000,
            max_outstanding_per_scope: 1_000,
            max_outstanding_per_route_component: 1_000,
            promotion_lease_ms: 100,
            abandoned_retention_ms: 200,
            instance_lifetime_ms: 1_000,
            max_reservations: 1_000,
            max_rate_buckets: 32,
        })
        .expect("test policy is valid");
        let mut state = PromotionPolicyState::new();
        for start in 0..(MAX_EXPIRED_EVENTS_PER_PRUNE + 2) {
            state
                .begin(
                    admission(start as u8, &scope),
                    UnixMillis::new(100),
                    bounded_limits,
                )
                .expect("setup admission succeeds");
        }

        state
            .begin(admission(0, &scope), UnixMillis::new(201), bounded_limits)
            .expect("the requested expired nonce is refreshed despite the cleanup backlog");

        assert_eq!(state.reservations.len(), 2);
        assert_eq!(state.outstanding_for_scope(&scope), 2);
    }

    #[test]
    fn requested_rate_scope_is_refreshed_even_behind_the_cleanup_backlog() {
        let bounded_limits = PromotionLimits::new(PromotionLimitConfig {
            max_seed_bytes: 4_096,
            window_ms: 1_000,
            max_promotions_per_window: 1,
            max_outstanding_per_scope: 1_000,
            max_outstanding_per_route_component: 1_000,
            promotion_lease_ms: 100,
            abandoned_retention_ms: 200,
            instance_lifetime_ms: 1_000,
            max_reservations: 1_000,
            max_rate_buckets: 1_000,
        })
        .expect("test policy is valid");
        let mut state = PromotionPolicyState::new();
        for start in 0..(MAX_EXPIRED_EVENTS_PER_PRUNE + 2) {
            let scope = ScopeFingerprint::from_bytes(&bytes::<32>(start as u8))
                .expect("test scope is valid");
            state
                .begin(
                    admission(0x70, &scope),
                    UnixMillis::new(100),
                    bounded_limits,
                )
                .expect("setup admission succeeds");
        }
        let requested_scope =
            ScopeFingerprint::from_bytes(&bytes::<32>(0)).expect("test scope is valid");

        state
            .begin(
                admission(0xf0, &requested_scope),
                UnixMillis::new(1_101),
                bounded_limits,
            )
            .expect("the requested expired rate window refreshes despite the cleanup backlog");
    }
}
