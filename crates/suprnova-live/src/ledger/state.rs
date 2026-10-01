//! The instance-ledger state machine as pure functions over one record.
//!
//! Every rule about what a ledger operation does to an instance lives here,
//! and nothing here can reach a store, a clock, or a lock. A transition takes
//! the record a caller read, the instant the caller read it at, and the
//! configured limits, and returns the record to write back (or `None` when
//! nothing changed) together with what the caller observes.
//!
//! That shape is what lets one state machine serve every tier. The kernel in
//! [`distributed`](super::distributed) loads a record from a store, applies a
//! function from this module, and compare-and-stores the result; the Tier 0
//! provider is that kernel over an in-process store. Neither owns a rule the
//! other does not.

use std::collections::VecDeque;

use super::{
    AcceptedOutcome, AcceptedOutcomeMetadata, ClaimOutcome, ClaimRequest, InstanceRecordKey,
    LedgerError, LedgerErrorKind, LedgerInspection, LedgerLimits, LedgerPhase, RefreshReason,
};
use crate::identity::{ContentDigest, IdempotencyKey, InstanceId, Revision, UnixMillis};

/// The authority an exact promotion retry recovers.
pub(crate) struct PromotionReservation {
    pub(crate) request_digest: ContentDigest,
    pub(crate) instance_id: InstanceId,
    pub(crate) initial_revision: Revision,
    pub(crate) expires_at: UnixMillis,
}

/// One successor revision claimed and not yet resolved.
#[derive(Clone, Debug)]
pub(crate) struct PendingClaim {
    pub(crate) claim_id: u64,
    pub(crate) base_revision: Revision,
    pub(crate) successor_revision: Revision,
    pub(crate) idempotency_key: IdempotencyKey,
    pub(crate) request_digest: ContentDigest,
    pub(crate) lease_expires_at: UnixMillis,
}

/// The instance lifecycle phase.
#[derive(Clone, Debug)]
pub(crate) enum InstancePhase {
    Ready,
    Pending(PendingClaim),
    Consumed {
        reason: RefreshReason,
        claim_id: Option<u64>,
    },
}

impl InstancePhase {
    pub(crate) const fn public_phase(&self) -> LedgerPhase {
        match self {
            Self::Ready => LedgerPhase::Ready,
            Self::Pending(_) => LedgerPhase::Pending,
            Self::Consumed { .. } => LedgerPhase::Consumed,
        }
    }
}

/// One instance's complete authority: what a store holds, encoded.
#[derive(Clone, Debug)]
pub(crate) struct InstanceRecord {
    pub(crate) component_contract: Option<ContentDigest>,
    pub(crate) current_revision: Revision,
    pub(crate) expires_at: UnixMillis,
    pub(crate) phase: InstancePhase,
    pub(crate) accepted: VecDeque<AcceptedOutcomeMetadata>,
}

/// What one transition decided: the record to write back, and the result.
///
/// `stored` is `None` when the transition read the record and changed
/// nothing, which is the common case for a classified rejection. A caller
/// that writes only when `stored` is `Some` never spends a
/// compare-and-store on a read.
pub(crate) struct Transition<T> {
    pub(crate) stored: Option<InstanceRecord>,
    pub(crate) outcome: T,
}

impl<T> Transition<T> {
    /// A transition that changed nothing.
    const fn unchanged(outcome: T) -> Self {
        Self {
            stored: None,
            outcome,
        }
    }

    /// A transition whose record must reach the store before the outcome is
    /// true.
    const fn stored(record: InstanceRecord, outcome: T) -> Self {
        Self {
            stored: Some(record),
            outcome,
        }
    }
}

/// What one claim decided, before a provider binds a token to it.
///
/// A grant needs an opaque single-use token, and a token belongs to the
/// provider that issued it, so the state machine names the grant and the
/// provider mints it.
pub(crate) enum ClaimDecision {
    /// The request claimed the successor revision exclusively.
    Granted { successor_revision: Revision },
    /// The request was classified without claiming anything.
    Classified(ClaimOutcome),
}

/// Rejects an expiry that has already elapsed, cannot be measured, or asks
/// for more lifetime than the provider is configured to give.
pub(crate) fn validate_expiry(
    now: UnixMillis,
    expires_at: UnixMillis,
    limits: LedgerLimits,
) -> Result<(), LedgerError> {
    let lifetime = expires_at
        .get()
        .checked_sub(now.get())
        .filter(|lifetime| *lifetime > 0)
        .ok_or_else(|| LedgerError::new(LedgerErrorKind::InvalidExpiry))?;
    if lifetime > limits.max_instance_lifetime_ms() {
        return Err(LedgerError::new(LedgerErrorKind::InvalidExpiry));
    }
    Ok(())
}

/// Builds the record one newly created instance starts from.
pub(crate) fn created_record(
    component_contract: Option<ContentDigest>,
    initial_revision: Revision,
    expires_at: UnixMillis,
) -> InstanceRecord {
    InstanceRecord {
        component_contract,
        current_revision: initial_revision,
        expires_at,
        phase: InstancePhase::Ready,
        accepted: VecDeque::new(),
    }
}

/// Whether the record's pending claim has outlived its lease.
fn lease_elapsed(record: &InstanceRecord, now: UnixMillis) -> bool {
    matches!(
        &record.phase,
        InstancePhase::Pending(pending) if pending.lease_expires_at <= now
    )
}

/// Terminally consumes a pending claim whose lease elapsed, reporting whether
/// it did.
fn expire_pending(record: &mut InstanceRecord, now: UnixMillis) -> bool {
    if !lease_elapsed(record, now) {
        return false;
    }
    let claim_id = match &record.phase {
        InstancePhase::Pending(pending) => Some(pending.claim_id),
        _ => None,
    };
    record.phase = InstancePhase::Consumed {
        reason: RefreshReason::ClaimExpired,
        claim_id,
    };
    true
}

/// The coarse phase a reader sees, with an elapsed claim lease projected onto
/// it.
///
/// Reads never write. A lease that has run out is terminal whether or not a
/// record has been rewritten to say so, and the next operation that does
/// write records it, so projecting the phase here costs a reader nothing and
/// tells it the truth.
fn projected_phase(record: &InstanceRecord, now: UnixMillis) -> LedgerPhase {
    if lease_elapsed(record, now) {
        return LedgerPhase::Consumed;
    }
    record.phase.public_phase()
}

/// Bounded metadata-only inspection of one record.
pub(crate) fn inspection(record: &InstanceRecord, now: UnixMillis) -> LedgerInspection {
    LedgerInspection {
        current_revision: record.current_revision,
        accepted_outcome_count: record.accepted.len(),
        phase: projected_phase(record, now),
    }
}

/// The revision an authorization read may rely on, or `None` when authority
/// is terminal.
///
/// A pending successor is not accepted authority, so a record mid-claim
/// answers with the base its claim was taken against.
pub(crate) fn accepted_revision(record: &InstanceRecord, now: UnixMillis) -> Option<Revision> {
    if lease_elapsed(record, now) {
        return None;
    }
    match &record.phase {
        InstancePhase::Ready => Some(record.current_revision),
        InstancePhase::Pending(pending) => Some(pending.base_revision),
        InstancePhase::Consumed { .. } => None,
    }
}

/// A successor that does not advance its base is not a state this build
/// writes, and honouring one would let a single base revision be claimed or
/// accepted twice.
///
/// [`decode_record`](super::record::decode_record) refuses such a record at
/// the frame, so this is the second gate: it runs at transition time, on the
/// record a store actually handed back, before a pending claim's revisions
/// become an observable outcome.
fn monotonic(pending: &PendingClaim) -> Result<(), LedgerError> {
    if pending.successor_revision <= pending.base_revision {
        return Err(LedgerError::new(LedgerErrorKind::InvalidConfiguration));
    }
    Ok(())
}

/// Evaluates one expected-revision claim against the record it read.
pub(crate) fn claim(
    mut record: InstanceRecord,
    request: &ClaimRequest,
    now: UnixMillis,
    limits: LedgerLimits,
    claim_id: u64,
) -> Result<Transition<ClaimDecision>, LedgerError> {
    if expire_pending(&mut record, now) {
        return Ok(Transition::stored(
            record,
            ClaimDecision::Classified(ClaimOutcome::RefreshRequired(RefreshReason::ClaimExpired)),
        ));
    }
    if let InstancePhase::Consumed { reason, .. } = record.phase {
        return Ok(Transition::unchanged(ClaimDecision::Classified(
            ClaimOutcome::RefreshRequired(reason),
        )));
    }

    if let Some(accepted) = record
        .accepted
        .iter()
        .find(|accepted| accepted.base_revision == request.base_revision)
        && accepted.idempotency_key == request.idempotency_key
    {
        let outcome = if accepted.request_digest == request.request_digest {
            ClaimOutcome::Accepted(accepted.clone())
        } else {
            ClaimOutcome::IdempotencyConflict
        };
        return Ok(Transition::unchanged(ClaimDecision::Classified(outcome)));
    }

    if let InstancePhase::Pending(pending) = &record.phase {
        if pending.base_revision == request.base_revision {
            let outcome = if pending.idempotency_key == request.idempotency_key
                && pending.request_digest == request.request_digest
            {
                monotonic(pending)?;
                ClaimOutcome::InProgress {
                    successor_revision: pending.successor_revision,
                }
            } else {
                ClaimOutcome::IdempotencyConflict
            };
            return Ok(Transition::unchanged(ClaimDecision::Classified(outcome)));
        }
        return Ok(Transition::unchanged(ClaimDecision::Classified(
            ClaimOutcome::Stale {
                current_revision: record.current_revision,
            },
        )));
    }

    if record.current_revision != request.base_revision {
        return Ok(Transition::unchanged(ClaimDecision::Classified(
            ClaimOutcome::Stale {
                current_revision: record.current_revision,
            },
        )));
    }

    let Ok(successor_revision) = request.base_revision.checked_next() else {
        record.phase = InstancePhase::Consumed {
            reason: RefreshReason::RevisionExhausted,
            claim_id: None,
        };
        return Ok(Transition::stored(
            record,
            ClaimDecision::Classified(ClaimOutcome::RefreshRequired(
                RefreshReason::RevisionExhausted,
            )),
        ));
    };
    let lease_expires_at = now
        .get()
        .checked_add(limits.claim_lease_ms())
        .map(UnixMillis::new)
        .ok_or_else(|| LedgerError::new(LedgerErrorKind::CounterExhausted))?;

    record.current_revision = successor_revision;
    record.phase = InstancePhase::Pending(PendingClaim {
        claim_id,
        base_revision: request.base_revision,
        successor_revision,
        idempotency_key: request.idempotency_key.clone(),
        request_digest: request.request_digest.clone(),
        lease_expires_at,
    });
    Ok(Transition::stored(
        record,
        ClaimDecision::Granted { successor_revision },
    ))
}

/// Retains one accepted outcome for exactly the matching pending claim.
pub(crate) fn commit(
    mut record: InstanceRecord,
    key: &InstanceRecordKey,
    claim_id: u64,
    outcome: AcceptedOutcome,
    now: UnixMillis,
    limits: LedgerLimits,
) -> Transition<Result<(), LedgerError>> {
    let metadata = match &record.phase {
        InstancePhase::Pending(pending) if pending.claim_id == claim_id => {
            if pending.lease_expires_at <= now {
                record.phase = InstancePhase::Consumed {
                    reason: RefreshReason::ClaimExpired,
                    claim_id: Some(claim_id),
                };
                return Transition::stored(
                    record,
                    Err(LedgerError::new(LedgerErrorKind::ClaimExpired)),
                );
            }
            if let Err(error) = monotonic(pending) {
                return Transition::unchanged(Err(error));
            }
            AcceptedOutcomeMetadata {
                scope: key.scope.clone(),
                instance_id: key.instance_id.clone(),
                base_revision: pending.base_revision,
                successor_revision: pending.successor_revision,
                idempotency_key: pending.idempotency_key.clone(),
                request_digest: pending.request_digest.clone(),
                outcome,
            }
        }
        InstancePhase::Consumed {
            reason: RefreshReason::ClaimExpired,
            claim_id: Some(expired_claim_id),
        } if *expired_claim_id == claim_id => {
            return Transition::unchanged(Err(LedgerError::new(LedgerErrorKind::ClaimExpired)));
        }
        _ => return Transition::unchanged(Err(LedgerError::new(LedgerErrorKind::ClaimMismatch))),
    };

    record.phase = InstancePhase::Ready;
    record.accepted.push_back(metadata);
    while record.accepted.len() > limits.max_accepted_outcomes() {
        record.accepted.pop_front();
    }
    Transition::stored(record, Ok(()))
}

/// Returns the base revision to a retryable state for exactly the matching
/// pending claim, which is what a dropped uncommitted claim needs.
pub(crate) fn release(
    mut record: InstanceRecord,
    claim_id: u64,
    now: UnixMillis,
) -> Transition<Result<(), LedgerError>> {
    match &record.phase {
        InstancePhase::Pending(pending) if pending.claim_id == claim_id => {
            if pending.lease_expires_at <= now {
                record.phase = InstancePhase::Consumed {
                    reason: RefreshReason::ClaimExpired,
                    claim_id: Some(claim_id),
                };
                return Transition::stored(
                    record,
                    Err(LedgerError::new(LedgerErrorKind::ClaimExpired)),
                );
            }
            record.current_revision = pending.base_revision;
            record.phase = InstancePhase::Ready;
            Transition::stored(record, Ok(()))
        }
        InstancePhase::Consumed {
            reason: RefreshReason::ClaimExpired,
            claim_id: Some(expired_claim_id),
        } if *expired_claim_id == claim_id => {
            Transition::unchanged(Err(LedgerError::new(LedgerErrorKind::ClaimExpired)))
        }
        _ => Transition::unchanged(Err(LedgerError::new(LedgerErrorKind::ClaimMismatch))),
    }
}

/// Terminally consumes authority for exactly the matching pending claim.
pub(crate) fn abandon(
    mut record: InstanceRecord,
    claim_id: u64,
    now: UnixMillis,
) -> Transition<Result<(), LedgerError>> {
    match &record.phase {
        InstancePhase::Pending(pending) if pending.claim_id == claim_id => {
            let elapsed = pending.lease_expires_at <= now;
            record.phase = InstancePhase::Consumed {
                reason: if elapsed {
                    RefreshReason::ClaimExpired
                } else {
                    RefreshReason::Consumed
                },
                claim_id: Some(claim_id),
            };
            let outcome = if elapsed {
                Err(LedgerError::new(LedgerErrorKind::ClaimExpired))
            } else {
                Ok(())
            };
            Transition::stored(record, outcome)
        }
        InstancePhase::Consumed {
            reason: RefreshReason::ClaimExpired,
            claim_id: Some(expired_claim_id),
        } if *expired_claim_id == claim_id => {
            Transition::unchanged(Err(LedgerError::new(LedgerErrorKind::ClaimExpired)))
        }
        _ => Transition::unchanged(Err(LedgerError::new(LedgerErrorKind::ClaimMismatch))),
    }
}

/// Retires a claim whose coordinated host effects may already have committed,
/// never restoring base-revision authority.
///
/// The claim's own lease is not consulted: an elapsed lease is already
/// terminal and a live one must become terminal, so both end in the same
/// place. Committing the matching outcome first makes this an idempotent
/// no-op, which is what lets a finalizing claim be dropped safely.
pub(crate) fn fence(
    mut record: InstanceRecord,
    claim_id: u64,
) -> Transition<Result<(), LedgerError>> {
    match &record.phase {
        InstancePhase::Pending(pending) if pending.claim_id == claim_id => {
            record.phase = InstancePhase::Consumed {
                reason: RefreshReason::Consumed,
                claim_id: Some(claim_id),
            };
            Transition::stored(record, Ok(()))
        }
        InstancePhase::Ready
            if record
                .accepted
                .back()
                .is_some_and(|accepted| accepted.successor_revision == record.current_revision) =>
        {
            Transition::unchanged(Ok(()))
        }
        InstancePhase::Consumed { claim_id: held, .. }
            if held.is_none_or(|held| held == claim_id) =>
        {
            Transition::unchanged(Ok(()))
        }
        _ => Transition::unchanged(Err(LedgerError::new(LedgerErrorKind::ClaimMismatch))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::ScopeFingerprint;
    use crate::ledger::AcceptedOutcomeKind;

    fn bytes<const LENGTH: usize>(start: u8) -> [u8; LENGTH] {
        std::array::from_fn(|offset| start.wrapping_add(offset as u8))
    }

    fn digest(start: u8) -> ContentDigest {
        ContentDigest::from_bytes(&bytes::<32>(start)).expect("digest is valid")
    }

    fn idempotency(start: u8) -> IdempotencyKey {
        IdempotencyKey::from_bytes(&bytes::<16>(start)).expect("idempotency key is valid")
    }

    fn key() -> InstanceRecordKey {
        InstanceRecordKey {
            scope: ScopeFingerprint::from_bytes(&bytes::<32>(0x10)).expect("scope is valid"),
            instance_id: InstanceId::from_bytes(&bytes::<16>(0x20)).expect("instance is valid"),
        }
    }

    fn limits() -> LedgerLimits {
        LedgerLimits::new(100, 10_000, 2, 64).expect("limits are valid")
    }

    fn request(base: u64) -> ClaimRequest {
        ClaimRequest::new(
            key().scope,
            key().instance_id,
            Revision::new(base),
            idempotency(0x30),
            digest(0x40),
        )
    }

    fn ready() -> InstanceRecord {
        created_record(None, Revision::new(0), UnixMillis::new(5_000))
    }

    #[test]
    fn a_classified_rejection_asks_for_no_write() {
        let stale = claim(ready(), &request(7), UnixMillis::new(1_000), limits(), 1)
            .expect("the claim classifies");

        assert!(
            stale.stored.is_none(),
            "a read that changes nothing must not cost a write"
        );
        assert!(matches!(
            stale.outcome,
            ClaimDecision::Classified(ClaimOutcome::Stale { .. })
        ));
    }

    #[test]
    fn a_transition_reads_its_input_and_returns_a_new_record() {
        let before = ready();
        let granted = claim(
            before.clone(),
            &request(0),
            UnixMillis::new(1_000),
            limits(),
            42,
        )
        .expect("the claim classifies");

        let after = granted.stored.expect("a grant is a write");
        assert_eq!(
            before.current_revision,
            Revision::new(0),
            "the input stands"
        );
        assert_eq!(after.current_revision, Revision::new(1));
        assert!(matches!(after.phase, InstancePhase::Pending(_)));
    }

    #[test]
    fn a_pending_claim_that_does_not_advance_its_base_is_refused_at_transition_time() {
        let mut record = ready();
        record.phase = InstancePhase::Pending(PendingClaim {
            claim_id: 7,
            base_revision: Revision::new(4),
            successor_revision: Revision::new(4),
            idempotency_key: idempotency(0x30),
            request_digest: digest(0x40),
            lease_expires_at: UnixMillis::new(2_000),
        });

        let committed = commit(
            record,
            &key(),
            7,
            AcceptedOutcome::new(AcceptedOutcomeKind::Rendered, digest(0x50)),
            UnixMillis::new(1_000),
            limits(),
        );

        assert!(
            committed.stored.is_none(),
            "a refused commit writes nothing"
        );
        assert_eq!(
            committed
                .outcome
                .expect_err("a non-advancing successor is not authority")
                .kind(),
            LedgerErrorKind::InvalidConfiguration
        );
    }
}
