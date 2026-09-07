//! Provider conformance for [`LiveInstanceLedger`], expressed against the
//! trait alone.
//!
//! Every scenario the Tier 0 integration suite covers is written here in
//! terms a distributed provider can answer: mount, promotion and its exact
//! retry, the expiry of a retry identity, claim and its classified
//! rejections, two claims racing for one base revision, commit, abandon, the
//! two synchronous drop paths, claim-lease and instance expiry, bounded
//! retained outcomes, and configured capacity. Nothing here reaches for
//! provider internals or diagnostics, so the same suite runs over the
//! engine's kernels and over a framework adapter against a real backend.
//!
//! One thing the Tier 0 suite proves is deliberately not here. Its
//! `opaque_claim_tokens_are_bound_to_the_provider_that_issued_them` needs two
//! providers, so it lives in [`run_two_node`] rather than [`run_all`].
//!
//! Two rules make the suite portable. It never waits: time only moves when
//! the caller's [`ControlledClock`] is set, and the suite sets it. And it
//! never assumes a fresh key space beyond its own, so it needs a ledger that
//! is empty and configured with [`conformance_limits`].

use std::sync::Arc;

use suprnova_live::clock::Clock;
use suprnova_live::identity::{
    ContentDigest, IdempotencyKey, InstanceId, Revision, ScopeFingerprint, UnixMillis,
};
use suprnova_live::ledger::{
    AcceptedOutcome, AcceptedOutcomeKind, ClaimGrant, ClaimOutcome, ClaimRequest, ClaimToken,
    LedgerError, LedgerErrorKind, LedgerLimits, LiveInstanceLedger, MountInstanceRecord,
    PromotionOutcome, PromotionRecord, RefreshReason,
};

use crate::ControlledClock;

/// Claim lease every conformance scenario is written against.
pub const CONFORMANCE_CLAIM_LEASE_MS: u64 = 100;

/// Longest instance lifetime the suite asks a provider for.
pub const CONFORMANCE_INSTANCE_LIFETIME_MS: u64 = 10_000;

/// Retained accepted outcomes the suite expects a provider to keep.
pub const CONFORMANCE_MAX_ACCEPTED_OUTCOMES: usize = 2;

/// Live instances the suite expects a provider to admit.
pub const CONFORMANCE_MAX_INSTANCES: usize = 64;

/// The limits a ledger under conformance must be built with.
///
/// The suite reasons about lease expiry, retention, and capacity, so it has
/// to know the bounds the provider was configured with. Build the ledger with
/// exactly these.
///
/// # Panics
///
/// Panics when the constants above stop being a valid [`LedgerLimits`], which
/// is a bug in this module rather than in a provider.
#[must_use]
pub fn conformance_limits() -> LedgerLimits {
    LedgerLimits::new(
        CONFORMANCE_CLAIM_LEASE_MS,
        CONFORMANCE_INSTANCE_LIFETIME_MS,
        CONFORMANCE_MAX_ACCEPTED_OUTCOMES,
        CONFORMANCE_MAX_INSTANCES,
    )
    .expect("the conformance limits are valid")
}

/// Runs every conformance scenario against one provider.
///
/// The ledger must be empty and built with [`conformance_limits`] over
/// `clock`. The suite moves `clock` forward, so a provider that shares it
/// with a store sees one consistent timeline, which is what the Tier 0
/// provider does.
///
/// # Errors
///
/// Returns the first scenario failure, naming the contract that was violated
/// and never a record, an identity, or a token.
pub async fn run_all(
    ledger: Arc<dyn LiveInstanceLedger>,
    clock: Arc<ControlledClock>,
) -> Result<(), String> {
    let ledger = ledger.as_ref();
    let clock = clock.as_ref();
    mount_creates_authority_and_is_create_only(ledger, clock).await?;
    promotion_recovers_an_exact_retry_and_refuses_a_changed_one(ledger, clock).await?;
    a_retry_identity_is_free_again_once_its_reservation_elapses(ledger, clock).await?;
    a_claim_advances_authority_and_an_exact_duplicate_observes_it(ledger, clock).await?;
    two_claims_for_one_base_revision_grant_exactly_one_token(ledger, clock).await?;
    stale_bases_and_reused_retry_identities_are_classified(ledger, clock).await?;
    an_abandoned_claim_leaves_no_authority(ledger, clock).await?;
    a_dropped_claim_releases_its_base_revision(ledger, clock).await?;
    a_fenced_claim_never_restores_its_base_revision(ledger, clock).await?;
    an_elapsed_claim_lease_is_terminal(ledger, clock).await?;
    missing_and_elapsed_instances_are_told_apart(ledger, clock).await?;
    retained_outcomes_are_bounded(ledger, clock).await?;
    configured_capacity_is_enforced_without_partial_authority(ledger, clock).await
}

/// Runs the scenarios that need two providers over one backing store.
///
/// Both ledgers must be built with [`conformance_limits`] over `clock` and
/// must share one store. Neither may have been through [`run_all`], which
/// uses the same key space.
///
/// # Errors
///
/// Returns the first scenario failure, naming the contract that was violated.
pub async fn run_two_node(
    first: Arc<dyn LiveInstanceLedger>,
    second: Arc<dyn LiveInstanceLedger>,
    clock: Arc<ControlledClock>,
) -> Result<(), String> {
    let first = first.as_ref();
    let second = second.as_ref();
    let clock = clock.as_ref();
    authority_created_on_one_node_is_authority_on_the_other(first, second, clock).await?;
    a_token_is_bound_to_the_node_that_issued_it(first, second, clock).await?;
    a_release_on_one_node_frees_the_base_revision_on_the_other(first, second, clock).await
}

/// Fails a scenario with the contract it violated.
fn require(satisfied: bool, contract: &str) -> Result<(), String> {
    if satisfied {
        return Ok(());
    }
    Err(contract.to_owned())
}

/// Names the operation and the closed reason a provider failed with.
fn failed(operation: &str, error: LedgerError) -> String {
    format!("{operation} failed with {}", error.kind().as_str())
}

/// Names an operation that was expected to fail and did not.
fn unexpected_success(contract: &str) -> String {
    format!("{contract}, but the operation succeeded")
}

fn bytes<const LENGTH: usize>(start: u8) -> [u8; LENGTH] {
    std::array::from_fn(|offset| start.wrapping_add(offset as u8))
}

fn scope() -> ScopeFingerprint {
    ScopeFingerprint::from_bytes(&bytes::<32>(0x10)).expect("the conformance scope is valid")
}

fn instance(start: u8) -> InstanceId {
    InstanceId::from_bytes(&bytes::<16>(start)).expect("the conformance instance is valid")
}

fn idempotency(start: u8) -> IdempotencyKey {
    IdempotencyKey::from_bytes(&bytes::<16>(start)).expect("the conformance retry key is valid")
}

fn digest(start: u8) -> ContentDigest {
    ContentDigest::from_bytes(&bytes::<32>(start)).expect("the conformance digest is valid")
}

/// Reads the suite's clock.
fn read_now(clock: &ControlledClock) -> Result<UnixMillis, String> {
    clock.now().map_err(|error| {
        format!(
            "the conformance clock failed with {}",
            error.kind().as_str()
        )
    })
}

/// Moves the suite's clock forward. Nothing here ever waits for time.
fn advance(clock: &ControlledClock, milliseconds: u64) -> Result<(), String> {
    let next = read_now(clock)?
        .get()
        .checked_add(milliseconds)
        .ok_or_else(|| "the conformance clock overflowed".to_owned())?;
    clock.set(UnixMillis::new(next));
    Ok(())
}

/// An expiry one full configured lifetime away.
fn lifetime_from_now(clock: &ControlledClock) -> Result<UnixMillis, String> {
    let expires_at = read_now(clock)?
        .get()
        .checked_add(CONFORMANCE_INSTANCE_LIFETIME_MS)
        .ok_or_else(|| "the conformance clock overflowed".to_owned())?;
    Ok(UnixMillis::new(expires_at))
}

fn mount_record(instance_start: u8, expires_at: UnixMillis) -> MountInstanceRecord {
    MountInstanceRecord::new(
        scope(),
        instance(instance_start),
        digest(0x30),
        Revision::new(0),
        expires_at,
    )
}

fn claim_request(instance_start: u8, base: u64, retry: u8) -> ClaimRequest {
    ClaimRequest::new(
        scope(),
        instance(instance_start),
        Revision::new(base),
        idempotency(retry),
        digest(retry.wrapping_add(0x40)),
    )
}

/// Mounts one fresh instance for a scenario.
async fn mounted(
    ledger: &dyn LiveInstanceLedger,
    clock: &ControlledClock,
    instance_start: u8,
) -> Result<(), String> {
    let expires_at = lifetime_from_now(clock)?;
    ledger
        .mount_instance(mount_record(instance_start, expires_at))
        .await
        .map(|_| ())
        .map_err(|error| failed("mount_instance", error))
}

/// Takes one grant, or names the outcome that was returned instead.
fn granted(outcome: ClaimOutcome, contract: &str) -> Result<ClaimGrant, String> {
    match outcome {
        ClaimOutcome::Granted(grant) => Ok(grant),
        other => Err(format!("{contract}, got {}", classify(&other))),
    }
}

/// A stable low-cardinality name for one claim outcome, safe to report.
fn classify(outcome: &ClaimOutcome) -> &'static str {
    match outcome {
        ClaimOutcome::Granted(_) => "granted",
        ClaimOutcome::InProgress { .. } => "in progress",
        ClaimOutcome::Accepted(_) => "accepted",
        ClaimOutcome::Stale { .. } => "stale",
        ClaimOutcome::IdempotencyConflict => "an idempotency conflict",
        ClaimOutcome::RefreshRequired(reason) => match reason {
            RefreshReason::Missing => "refresh required, missing",
            RefreshReason::InstanceExpired => "refresh required, instance expired",
            RefreshReason::Consumed => "refresh required, consumed",
            RefreshReason::ClaimExpired => "refresh required, claim expired",
            RefreshReason::RevisionExhausted => "refresh required, revision exhausted",
        },
    }
}

/// Claims and commits one revision, which several scenarios need as setup.
async fn claim_and_commit(
    ledger: &dyn LiveInstanceLedger,
    instance_start: u8,
    base: u64,
    retry: u8,
) -> Result<(), String> {
    let grant = granted(
        ledger
            .claim(claim_request(instance_start, base, retry))
            .await
            .map_err(|error| failed("claim", error))?,
        "the current base revision must be claimable",
    )?;
    ledger
        .commit(
            &grant.into_token(),
            AcceptedOutcome::new(AcceptedOutcomeKind::Rendered, digest(retry)),
        )
        .await
        .map_err(|error| failed("commit", error))
}

/// Reads the accepted revision of one instance.
async fn accepted(
    ledger: &dyn LiveInstanceLedger,
    instance_start: u8,
) -> Result<Option<Revision>, String> {
    ledger
        .current_accepted_revision(&scope(), &instance(instance_start))
        .await
        .map_err(|error| failed("current_accepted_revision", error))
}

async fn mount_creates_authority_and_is_create_only(
    ledger: &dyn LiveInstanceLedger,
    clock: &ControlledClock,
) -> Result<(), String> {
    let expires_at = lifetime_from_now(clock)?;
    let record = mount_record(0x20, expires_at);

    let authority = ledger
        .mount_instance(record.clone())
        .await
        .map_err(|error| failed("mount_instance", error))?;
    require(
        authority.revision() == Revision::new(0),
        "a mount starts at the initial revision it was asked for",
    )?;
    require(
        authority.expires_at() == expires_at,
        "a mount keeps the expiry it was asked for",
    )?;
    require(
        accepted(ledger, 0x20).await? == Some(Revision::new(0)),
        "a mounted instance is current authority",
    )?;

    let conflict = ledger
        .mount_instance(record)
        .await
        .err()
        .ok_or_else(|| unexpected_success("a repeated mount identity is a conflict"))?;
    require(
        conflict.kind() == LedgerErrorKind::InstanceConflict,
        "a repeated mount identity is an instance conflict",
    )?;

    let elapsed = ledger
        .mount_instance(mount_record(0x21, read_now(clock)?))
        .await
        .err()
        .ok_or_else(|| unexpected_success("an elapsed expiry creates no authority"))?;
    require(
        elapsed.kind() == LedgerErrorKind::InvalidExpiry,
        "an expiry that has already elapsed is refused",
    )?;
    require(
        accepted(ledger, 0x21).await?.is_none(),
        "a refused mount leaves no authority behind",
    )
}

async fn promotion_recovers_an_exact_retry_and_refuses_a_changed_one(
    ledger: &dyn LiveInstanceLedger,
    clock: &ControlledClock,
) -> Result<(), String> {
    let expires_at = lifetime_from_now(clock)?;
    let record = PromotionRecord::new(
        scope(),
        instance(0x22),
        idempotency(0x40),
        digest(0x50),
        Revision::new(0),
        expires_at,
    );

    match ledger
        .promote(record.clone())
        .await
        .map_err(|error| failed("promote", error))?
    {
        PromotionOutcome::Created(authority) => require(
            authority.instance_id() == &instance(0x22),
            "a promotion creates the instance it proposed",
        )?,
        _ => return Err("a first promotion must create authority".to_owned()),
    }

    match ledger
        .promote(record.clone().with_instance_id(instance(0x23)))
        .await
        .map_err(|error| failed("promote", error))?
    {
        PromotionOutcome::Existing(authority) => require(
            authority.instance_id() == &instance(0x22),
            "an exact retry recovers the instance the first promotion created",
        )?,
        _ => return Err("an exact promotion retry must recover authority".to_owned()),
    }

    match ledger
        .promote(
            record
                .with_instance_id(instance(0x24))
                .with_request_digest(digest(0x51)),
        )
        .await
        .map_err(|error| failed("promote", error))?
    {
        PromotionOutcome::IdempotencyConflict => Ok(()),
        _ => Err("a retry identity reused for another request is a conflict".to_owned()),
    }
}

async fn a_retry_identity_is_free_again_once_its_reservation_elapses(
    ledger: &dyn LiveInstanceLedger,
    clock: &ControlledClock,
) -> Result<(), String> {
    let short = read_now(clock)?
        .get()
        .checked_add(200)
        .ok_or_else(|| "the conformance clock overflowed".to_owned())?;
    let record = PromotionRecord::new(
        scope(),
        instance(0x2d),
        idempotency(0x4c),
        digest(0x57),
        Revision::new(0),
        UnixMillis::new(short),
    );

    match ledger
        .promote(record.clone())
        .await
        .map_err(|error| failed("promote", error))?
    {
        PromotionOutcome::Created(_) => {}
        _ => return Err("a first promotion must create authority".to_owned()),
    }
    match ledger
        .promote(record.clone().with_instance_id(instance(0x2e)))
        .await
        .map_err(|error| failed("promote", error))?
    {
        PromotionOutcome::Existing(authority) => require(
            authority.instance_id() == &instance(0x2d),
            "an exact retry inside the reservation's life recovers authority",
        )?,
        _ => return Err("an exact retry must recover authority while it can".to_owned()),
    }

    advance(clock, 200)?;

    // The same retry identity and the same request, after the reservation
    // that held it elapsed. Nothing may recover the authority it named.
    let renewed = PromotionRecord::new(
        scope(),
        instance(0x2f),
        idempotency(0x4c),
        digest(0x57),
        Revision::new(0),
        lifetime_from_now(clock)?,
    );
    match ledger
        .promote(renewed)
        .await
        .map_err(|error| failed("promote", error))?
    {
        PromotionOutcome::Created(authority) => require(
            authority.instance_id() == &instance(0x2f),
            "an elapsed reservation frees its retry identity for a new promotion",
        ),
        _ => Err("an elapsed retry identity must not recover the authority it held".to_owned()),
    }
}

async fn two_claims_for_one_base_revision_grant_exactly_one_token(
    ledger: &dyn LiveInstanceLedger,
    clock: &ControlledClock,
) -> Result<(), String> {
    mounted(ledger, clock, 0x30).await?;
    let request = claim_request(0x30, 0, 0x4d);

    // Over a provider whose store never yields these run in order and prove
    // that a second claim joins the first; over one that does, they interleave
    // and prove that a stale read never becomes a second grant. Either way,
    // exactly one token comes out.
    let (first, second) = tokio::join!(ledger.claim(request.clone()), ledger.claim(request));

    let outcomes = [
        first.map_err(|error| failed("claim", error))?,
        second.map_err(|error| failed("claim", error))?,
    ];
    let granted = outcomes
        .iter()
        .filter(|outcome| matches!(outcome, ClaimOutcome::Granted(_)))
        .count();
    let joined = outcomes
        .iter()
        .filter(|outcome| matches!(outcome, ClaimOutcome::InProgress { .. }))
        .count();
    require(
        granted == 1,
        "exactly one of two claims takes the successor",
    )?;
    require(
        joined == 1,
        "the claim that did not take the successor observes the one that did",
    )
}

async fn a_claim_advances_authority_and_an_exact_duplicate_observes_it(
    ledger: &dyn LiveInstanceLedger,
    clock: &ControlledClock,
) -> Result<(), String> {
    mounted(ledger, clock, 0x25).await?;
    let request = claim_request(0x25, 0, 0x41);

    let grant = granted(
        ledger
            .claim(request.clone())
            .await
            .map_err(|error| failed("claim", error))?,
        "the current base revision must be claimable",
    )?;
    require(
        grant.successor_revision() == Revision::new(1),
        "a claim takes the monotonic successor of its base",
    )?;
    require(
        accepted(ledger, 0x25).await? == Some(Revision::new(0)),
        "a claimed successor is not accepted authority",
    )?;

    let duplicate = ledger
        .claim(request.clone())
        .await
        .map_err(|error| failed("claim", error))?;
    match duplicate {
        ClaimOutcome::InProgress { successor_revision } => require(
            successor_revision == Revision::new(1),
            "an exact duplicate observes the successor already claimed",
        )?,
        other => {
            return Err(format!(
                "an exact duplicate of a pending claim is in progress, got {}",
                classify(&other)
            ));
        }
    }

    let outcome = AcceptedOutcome::new(AcceptedOutcomeKind::Rendered, digest(0x52));
    ledger
        .commit(&grant.into_token(), outcome.clone())
        .await
        .map_err(|error| failed("commit", error))?;
    require(
        accepted(ledger, 0x25).await? == Some(Revision::new(1)),
        "a committed successor becomes accepted authority",
    )?;

    match ledger
        .claim(request)
        .await
        .map_err(|error| failed("claim", error))?
    {
        ClaimOutcome::Accepted(metadata) => {
            require(
                metadata.base_revision() == Revision::new(0)
                    && metadata.successor_revision() == Revision::new(1),
                "a retained outcome names the revisions it was accepted for",
            )?;
            require(
                metadata.outcome() == &outcome,
                "a retained outcome is the one that was committed",
            )?;
        }
        other => {
            return Err(format!(
                "an exact duplicate of a committed claim observes its outcome, got {}",
                classify(&other)
            ));
        }
    }

    let next = granted(
        ledger
            .claim(claim_request(0x25, 1, 0x42))
            .await
            .map_err(|error| failed("claim", error))?,
        "the new accepted revision must be claimable",
    )?;
    require(
        next.successor_revision() == Revision::new(2),
        "authority advances one revision at a time",
    )?;
    ledger
        .abandon(&next.into_token())
        .await
        .map_err(|error| failed("abandon", error))
}

async fn stale_bases_and_reused_retry_identities_are_classified(
    ledger: &dyn LiveInstanceLedger,
    clock: &ControlledClock,
) -> Result<(), String> {
    mounted(ledger, clock, 0x26).await?;
    let grant = granted(
        ledger
            .claim(claim_request(0x26, 0, 0x43))
            .await
            .map_err(|error| failed("claim", error))?,
        "the current base revision must be claimable",
    )?;

    let changed_retry = ClaimRequest::new(
        scope(),
        instance(0x26),
        Revision::new(0),
        idempotency(0x44),
        digest(0x83),
    );
    match ledger
        .claim(changed_retry)
        .await
        .map_err(|error| failed("claim", error))?
    {
        ClaimOutcome::IdempotencyConflict => {}
        other => {
            return Err(format!(
                "another request cannot join a pending claim, got {}",
                classify(&other)
            ));
        }
    }

    let changed_digest = ClaimRequest::new(
        scope(),
        instance(0x26),
        Revision::new(0),
        idempotency(0x43),
        digest(0x84),
    );
    match ledger
        .claim(changed_digest)
        .await
        .map_err(|error| failed("claim", error))?
    {
        ClaimOutcome::IdempotencyConflict => {}
        other => {
            return Err(format!(
                "a retry identity reused for other metadata is a conflict, got {}",
                classify(&other)
            ));
        }
    }

    ledger
        .commit(
            &grant.into_token(),
            AcceptedOutcome::new(AcceptedOutcomeKind::NoRender, digest(0x53)),
        )
        .await
        .map_err(|error| failed("commit", error))?;

    match ledger
        .claim(claim_request(0x26, 0, 0x45))
        .await
        .map_err(|error| failed("claim", error))?
    {
        ClaimOutcome::Stale { current_revision } => require(
            current_revision == Revision::new(1),
            "a stale request is told the current revision",
        )?,
        other => {
            return Err(format!(
                "a superseded base revision is stale, got {}",
                classify(&other)
            ));
        }
    }

    let accepted_mismatch = ClaimRequest::new(
        scope(),
        instance(0x26),
        Revision::new(0),
        idempotency(0x43),
        digest(0x85),
    );
    match ledger
        .claim(accepted_mismatch)
        .await
        .map_err(|error| failed("claim", error))?
    {
        ClaimOutcome::IdempotencyConflict => Ok(()),
        other => Err(format!(
            "a retry identity reused against a retained outcome is a conflict, got {}",
            classify(&other)
        )),
    }
}

async fn an_abandoned_claim_leaves_no_authority(
    ledger: &dyn LiveInstanceLedger,
    clock: &ControlledClock,
) -> Result<(), String> {
    mounted(ledger, clock, 0x27).await?;
    let grant = granted(
        ledger
            .claim(claim_request(0x27, 0, 0x46))
            .await
            .map_err(|error| failed("claim", error))?,
        "the current base revision must be claimable",
    )?;

    ledger
        .abandon(&grant.into_token())
        .await
        .map_err(|error| failed("abandon", error))?;

    require(
        accepted(ledger, 0x27).await?.is_none(),
        "terminally consumed authority is not an accepted revision",
    )?;
    for base in [0, 1] {
        match ledger
            .claim(claim_request(0x27, base, 0x46))
            .await
            .map_err(|error| failed("claim", error))?
        {
            ClaimOutcome::RefreshRequired(RefreshReason::Consumed) => {}
            other => {
                return Err(format!(
                    "consumed authority cannot be reclaimed, got {}",
                    classify(&other)
                ));
            }
        }
    }
    Ok(())
}

async fn a_dropped_claim_releases_its_base_revision(
    ledger: &dyn LiveInstanceLedger,
    clock: &ControlledClock,
) -> Result<(), String> {
    mounted(ledger, clock, 0x28).await?;
    let grant = granted(
        ledger
            .claim(claim_request(0x28, 0, 0x47))
            .await
            .map_err(|error| failed("claim", error))?,
        "the current base revision must be claimable",
    )?;

    ledger.abandon_on_drop(grant.into_token());

    let retry = granted(
        ledger
            .claim(claim_request(0x28, 0, 0x47))
            .await
            .map_err(|error| failed("claim", error))?,
        "a released base revision accepts an exact retry",
    )?;
    require(
        retry.successor_revision() == Revision::new(1),
        "a retry after a release claims the same successor",
    )?;
    ledger
        .abandon(&retry.into_token())
        .await
        .map_err(|error| failed("abandon", error))
}

async fn a_fenced_claim_never_restores_its_base_revision(
    ledger: &dyn LiveInstanceLedger,
    clock: &ControlledClock,
) -> Result<(), String> {
    mounted(ledger, clock, 0x29).await?;
    let grant = granted(
        ledger
            .claim(claim_request(0x29, 0, 0x48))
            .await
            .map_err(|error| failed("claim", error))?,
        "the current base revision must be claimable",
    )?;

    ledger.fence_on_drop(grant.into_token());

    match ledger
        .claim(claim_request(0x29, 0, 0x48))
        .await
        .map_err(|error| failed("claim", error))?
    {
        ClaimOutcome::RefreshRequired(RefreshReason::Consumed) => {}
        other => {
            return Err(format!(
                "a fenced claim never returns its base revision, got {}",
                classify(&other)
            ));
        }
    }
    require(
        accepted(ledger, 0x29).await?.is_none(),
        "a fenced claim leaves no accepted authority",
    )
}

async fn an_elapsed_claim_lease_is_terminal(
    ledger: &dyn LiveInstanceLedger,
    clock: &ControlledClock,
) -> Result<(), String> {
    mounted(ledger, clock, 0x2a).await?;
    let grant = granted(
        ledger
            .claim(claim_request(0x2a, 0, 0x49))
            .await
            .map_err(|error| failed("claim", error))?,
        "the current base revision must be claimable",
    )?;
    let token: ClaimToken = grant.into_token();

    advance(clock, CONFORMANCE_CLAIM_LEASE_MS + 1)?;

    match ledger
        .claim(claim_request(0x2a, 1, 0x4a))
        .await
        .map_err(|error| failed("claim", error))?
    {
        ClaimOutcome::RefreshRequired(RefreshReason::ClaimExpired) => {}
        other => {
            return Err(format!(
                "an elapsed claim lease is terminal, got {}",
                classify(&other)
            ));
        }
    }

    let error = ledger
        .commit(
            &token,
            AcceptedOutcome::new(AcceptedOutcomeKind::Recovery, digest(0x54)),
        )
        .await
        .err()
        .ok_or_else(|| unexpected_success("an elapsed claim cannot commit"))?;
    require(
        error.kind() == LedgerErrorKind::ClaimExpired,
        "an elapsed claim commits nothing and says why",
    )
}

async fn missing_and_elapsed_instances_are_told_apart(
    ledger: &dyn LiveInstanceLedger,
    clock: &ControlledClock,
) -> Result<(), String> {
    match ledger
        .claim(claim_request(0x2b, 0, 0x4b))
        .await
        .map_err(|error| failed("claim", error))?
    {
        ClaimOutcome::RefreshRequired(RefreshReason::Missing) => {}
        other => {
            return Err(format!(
                "an instance that was never created is missing, got {}",
                classify(&other)
            ));
        }
    }

    let short = read_now(clock)?
        .get()
        .checked_add(200)
        .ok_or_else(|| "the conformance clock overflowed".to_owned())?;
    ledger
        .mount_instance(mount_record(0x2b, UnixMillis::new(short)))
        .await
        .map_err(|error| failed("mount_instance", error))?;

    advance(clock, 200)?;

    match ledger
        .claim(claim_request(0x2b, 0, 0x4b))
        .await
        .map_err(|error| failed("claim", error))?
    {
        ClaimOutcome::RefreshRequired(RefreshReason::InstanceExpired) => {}
        other => {
            return Err(format!(
                "an instance whose lifetime elapsed says so, got {}",
                classify(&other)
            ));
        }
    }
    require(
        accepted(ledger, 0x2b).await?.is_none(),
        "an elapsed instance is not accepted authority",
    )
}

async fn retained_outcomes_are_bounded(
    ledger: &dyn LiveInstanceLedger,
    clock: &ControlledClock,
) -> Result<(), String> {
    mounted(ledger, clock, 0x2c).await?;
    let retained = u64::try_from(CONFORMANCE_MAX_ACCEPTED_OUTCOMES)
        .map_err(|_| "the retained-outcome bound does not fit a revision".to_owned())?;

    for base in 0..=retained {
        let retry = 0x60_u8.wrapping_add(u8::try_from(base).unwrap_or(0));
        claim_and_commit(ledger, 0x2c, base, retry).await?;
    }

    require(
        accepted(ledger, 0x2c).await? == Some(Revision::new(retained + 1)),
        "every committed claim advanced accepted authority",
    )?;
    match ledger
        .claim(claim_request(0x2c, 0, 0x60))
        .await
        .map_err(|error| failed("claim", error))?
    {
        ClaimOutcome::Stale { current_revision } => require(
            current_revision == Revision::new(retained + 1),
            "a duplicate whose outcome was dropped is told the current revision",
        ),
        other => Err(format!(
            "a duplicate past the retained-outcome bound is stale, got {}",
            classify(&other)
        )),
    }
}

async fn configured_capacity_is_enforced_without_partial_authority(
    ledger: &dyn LiveInstanceLedger,
    clock: &ControlledClock,
) -> Result<(), String> {
    let expires_at = lifetime_from_now(clock)?;
    let mut refused = None;

    for offset in 0..=CONFORMANCE_MAX_INSTANCES {
        let start = 0x80_u8.wrapping_add(u8::try_from(offset).unwrap_or(0));
        match ledger.mount_instance(mount_record(start, expires_at)).await {
            Ok(_) => {}
            Err(error) if error.kind() == LedgerErrorKind::CapacityExceeded => {
                refused = Some(start);
                break;
            }
            Err(error) => return Err(failed("mount_instance", error)),
        }
    }

    let refused =
        refused.ok_or_else(|| "the configured instance capacity is enforced".to_owned())?;
    require(
        accepted(ledger, refused).await?.is_none(),
        "a mount refused for capacity leaves no partial authority",
    )
}

async fn authority_created_on_one_node_is_authority_on_the_other(
    first: &dyn LiveInstanceLedger,
    second: &dyn LiveInstanceLedger,
    clock: &ControlledClock,
) -> Result<(), String> {
    let expires_at = lifetime_from_now(clock)?;
    let record = mount_record(0x60, expires_at);
    first
        .mount_instance(record.clone())
        .await
        .map_err(|error| failed("mount_instance", error))?;

    require(
        accepted(second, 0x60).await? == Some(Revision::new(0)),
        "authority lives in the store, not in the node that created it",
    )?;
    let conflict = second
        .mount_instance(record)
        .await
        .err()
        .ok_or_else(|| unexpected_success("a mount is create-only across nodes"))?;
    require(
        conflict.kind() == LedgerErrorKind::InstanceConflict,
        "a repeated mount identity is an instance conflict on every node",
    )?;

    let grant = granted(
        first
            .claim(claim_request(0x60, 0, 0x61))
            .await
            .map_err(|error| failed("claim", error))?,
        "the current base revision must be claimable",
    )?;
    first
        .commit(
            &grant.into_token(),
            AcceptedOutcome::new(AcceptedOutcomeKind::Rendered, digest(0x55)),
        )
        .await
        .map_err(|error| failed("commit", error))?;
    require(
        accepted(second, 0x60).await? == Some(Revision::new(1)),
        "a commit on one node is accepted authority on the other",
    )
}

async fn a_token_is_bound_to_the_node_that_issued_it(
    first: &dyn LiveInstanceLedger,
    second: &dyn LiveInstanceLedger,
    clock: &ControlledClock,
) -> Result<(), String> {
    mounted(first, clock, 0x61).await?;
    let grant = granted(
        first
            .claim(claim_request(0x61, 0, 0x62))
            .await
            .map_err(|error| failed("claim", error))?,
        "the current base revision must be claimable",
    )?;
    let token = grant.into_token();

    let crossed = second
        .commit(
            &token,
            AcceptedOutcome::new(AcceptedOutcomeKind::NoRender, digest(0x56)),
        )
        .await
        .err()
        .ok_or_else(|| unexpected_success("a token cannot cross providers"))?;
    require(
        crossed.kind() == LedgerErrorKind::ClaimMismatch,
        "a token presented to another provider is a claim mismatch",
    )?;

    first
        .commit(
            &token,
            AcceptedOutcome::new(AcceptedOutcomeKind::NoRender, digest(0x56)),
        )
        .await
        .map_err(|error| failed("commit", error))?;
    require(
        accepted(second, 0x61).await? == Some(Revision::new(1)),
        "the issuing node still commits after a rejected crossing",
    )
}

async fn a_release_on_one_node_frees_the_base_revision_on_the_other(
    first: &dyn LiveInstanceLedger,
    second: &dyn LiveInstanceLedger,
    clock: &ControlledClock,
) -> Result<(), String> {
    mounted(first, clock, 0x62).await?;
    let grant = granted(
        first
            .claim(claim_request(0x62, 0, 0x63))
            .await
            .map_err(|error| failed("claim", error))?,
        "the current base revision must be claimable",
    )?;

    first.abandon_on_drop(grant.into_token());
    // A retirement a drop queued is applied at the provider's next
    // asynchronous operation, so this read is here to make one happen.
    accepted(first, 0x62).await?;

    let retry = granted(
        second
            .claim(claim_request(0x62, 0, 0x64))
            .await
            .map_err(|error| failed("claim", error))?,
        "a released base revision is claimable on every node",
    )?;
    second
        .abandon(&retry.into_token())
        .await
        .map_err(|error| failed("abandon", error))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use suprnova_live::clock::Clock;
    use suprnova_live::identity::UnixMillis;
    use suprnova_live::ledger::MemoryInstanceLedger;

    use super::{ControlledClock, LiveInstanceLedger, conformance_limits, run_all};

    #[tokio::test]
    async fn the_suite_passes_over_the_embedded_provider() {
        let clock = Arc::new(ControlledClock::new(UnixMillis::new(1_000)));
        let ledger: Arc<dyn LiveInstanceLedger> = Arc::new(MemoryInstanceLedger::new(
            Arc::clone(&clock) as Arc<dyn Clock>,
            conformance_limits(),
        ));

        run_all(ledger, clock)
            .await
            .expect("the embedded provider conforms");
    }
}
