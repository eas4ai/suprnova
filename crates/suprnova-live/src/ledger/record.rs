//! Versioned canonical encoding of one [`InstanceRecord`].
//!
//! A distributed instance ledger keeps its authority in a store that holds
//! bytes and nothing else, so every rule about what a record may contain
//! lives here. One record is [`RECORD_VERSION`] followed by the RFC
//! 8785-compatible canonical JSON of a mirror of the in-memory record, and
//! the whole frame is bounded by [`MAX_RECORD_BYTES`].
//!
//! The mirror exists because the engine's identities serialize but never
//! deserialize. Every field travels as text and comes back through the
//! identity's own validating constructor, so a record read from a store that
//! another process can write is validated exactly as protocol input is:
//! decoding classifies and never panics, whatever the bytes are.
//!
//! Records carry revision metadata only. No component state, rendered HTML,
//! action arguments, or response bytes reach a store, and no error raised
//! here repeats a record's bytes or identities.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use super::contract::MAX_ACCEPTED_OUTCOMES;
use super::state::{InstancePhase, InstanceRecord, PendingClaim};
use super::{
    AcceptedOutcome, AcceptedOutcomeKind, AcceptedOutcomeMetadata, LedgerError, LedgerErrorKind,
    RefreshReason,
};
use crate::canonical::{
    CanonicalError, CanonicalErrorKind, CanonicalValue, parse_canonical_value, to_canonical_bytes,
};
use crate::identity::{
    ContentDigest, IdempotencyKey, InstanceId, Revision, ScopeFingerprint, UnixMillis,
};
use crate::limits::InputLimits;

/// Encoding version of the record frame this build writes and accepts.
pub const RECORD_VERSION: u8 = 1;

/// Maximum bytes one encoded record may occupy, version byte included.
pub const MAX_RECORD_BYTES: usize = 16_384;

/// The canonical JSON body's share of [`MAX_RECORD_BYTES`]; the leading
/// version byte takes the rest.
const MAX_BODY_BYTES: usize = MAX_RECORD_BYTES - 1;

/// Container nesting a record body may reach: the record, its phase or
/// accepted array, one accepted entry, and that entry's outcome.
const MAX_BODY_DEPTH: usize = 8;

/// Array elements plus object members a record body may carry. A record at
/// the retained-outcome ceiling needs about two thirds of this.
const MAX_BODY_ENTRIES: usize = 1_024;

/// Bytes one string in a record body may carry. The longest the codec writes
/// is a 32-byte identity in base64url, which is 43.
const MAX_BODY_STRING_BYTES: usize = 64;

/// A record whose shape, version, or identities are not what this build
/// writes.
fn shape_error() -> LedgerError {
    LedgerError::new(LedgerErrorKind::InvalidConfiguration)
}

/// A record that does not fit a bound the codec enforces.
fn size_error() -> LedgerError {
    LedgerError::new(LedgerErrorKind::CapacityExceeded)
}

/// Maps a canonical codec failure onto the bound or the shape it violated.
fn map_canonical(error: CanonicalError) -> LedgerError {
    match error.kind() {
        CanonicalErrorKind::TooLarge
        | CanonicalErrorKind::TooDeep
        | CanonicalErrorKind::TooManyEntries
        | CanonicalErrorKind::StringTooLong => size_error(),
        _ => shape_error(),
    }
}

/// The bounds a record body is parsed and serialized under.
fn body_limits() -> Result<InputLimits, LedgerError> {
    InputLimits::new(
        MAX_BODY_BYTES,
        MAX_BODY_DEPTH,
        MAX_BODY_ENTRIES,
        MAX_BODY_STRING_BYTES,
    )
    .map_err(|_| shape_error())
}

/// Encodes one record as [`RECORD_VERSION`] followed by canonical JSON.
///
/// A record holding more than [`MAX_ACCEPTED_OUTCOMES`] retained outcomes, or
/// whose body does not fit [`MAX_RECORD_BYTES`], is reported rather than
/// truncated: dropping retained authority silently would let an exact
/// duplicate be executed twice.
#[allow(
    dead_code,
    reason = "the codec's caller is the distributed kernel that follows; this module's tests prove it"
)]
pub(crate) fn encode_record(record: &InstanceRecord) -> Result<Vec<u8>, LedgerError> {
    if record.accepted.len() > MAX_ACCEPTED_OUTCOMES {
        return Err(size_error());
    }
    let serde_value =
        serde_json::to_value(RecordV1::from_record(record)).map_err(|_| shape_error())?;
    let canonical = CanonicalValue::from_serde_value(serde_value).map_err(map_canonical)?;
    let body = to_canonical_bytes(&canonical, &body_limits()?).map_err(map_canonical)?;
    let mut frame = Vec::with_capacity(body.len().saturating_add(1));
    frame.push(RECORD_VERSION);
    frame.extend_from_slice(&body);
    Ok(frame)
}

/// Decodes one record frame, validating every identity through its own
/// constructor.
///
/// The byte bound is checked before anything is parsed, and every remaining
/// bound is the canonical parser's, so arbitrary input costs bounded work and
/// is classified rather than trusted.
#[allow(
    dead_code,
    reason = "the codec's caller is the distributed kernel that follows; this module's tests prove it"
)]
pub(crate) fn decode_record(frame: &[u8]) -> Result<InstanceRecord, LedgerError> {
    if frame.len() > MAX_RECORD_BYTES {
        return Err(size_error());
    }
    let (version, body) = frame.split_first().ok_or_else(shape_error)?;
    if *version != RECORD_VERSION {
        return Err(shape_error());
    }
    let canonical = parse_canonical_value(body, &body_limits()?).map_err(map_canonical)?;
    let serde_value = canonical.to_serde_value().map_err(map_canonical)?;
    let mirror: RecordV1 = serde_json::from_value(serde_value).map_err(|_| shape_error())?;
    mirror.into_record()
}

/// Writes one unsigned integer as the canonical decimal the codec reads back.
fn decimal(value: u64) -> String {
    value.to_string()
}

/// Parses the canonical unsigned decimal the codec writes: digits only, no
/// leading zero except `"0"` itself, and within `u64`.
fn decode_decimal(value: &str) -> Result<u64, LedgerError> {
    let canonical = value == "0"
        || (!value.is_empty()
            && !value.starts_with('0')
            && value.bytes().all(|byte| byte.is_ascii_digit()));
    if !canonical {
        return Err(shape_error());
    }
    value.parse().map_err(|_| shape_error())
}

fn decode_revision(value: &str) -> Result<Revision, LedgerError> {
    Revision::parse(value).map_err(|_| shape_error())
}

fn decode_millis(value: &str) -> Result<UnixMillis, LedgerError> {
    UnixMillis::parse(value).map_err(|_| shape_error())
}

fn decode_scope(value: &str) -> Result<ScopeFingerprint, LedgerError> {
    ScopeFingerprint::parse(value).map_err(|_| shape_error())
}

fn decode_instance(value: &str) -> Result<InstanceId, LedgerError> {
    InstanceId::parse(value).map_err(|_| shape_error())
}

fn decode_idempotency(value: &str) -> Result<IdempotencyKey, LedgerError> {
    IdempotencyKey::parse(value).map_err(|_| shape_error())
}

fn decode_digest(value: &str) -> Result<ContentDigest, LedgerError> {
    ContentDigest::parse(value).map_err(|_| shape_error())
}

/// Version 1 of the stored record: the in-memory record with every identity
/// as the text its own constructor accepts.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RecordV1 {
    accepted: Vec<AcceptedV1>,
    component_contract: Option<String>,
    current_revision: String,
    expires_at: String,
    phase: PhaseV1,
}

impl RecordV1 {
    fn from_record(record: &InstanceRecord) -> Self {
        Self {
            accepted: record
                .accepted
                .iter()
                .map(AcceptedV1::from_metadata)
                .collect(),
            component_contract: record
                .component_contract
                .as_ref()
                .map(ContentDigest::to_base64url),
            current_revision: decimal(record.current_revision.get()),
            expires_at: decimal(record.expires_at.get()),
            phase: PhaseV1::from_phase(&record.phase),
        }
    }

    fn into_record(self) -> Result<InstanceRecord, LedgerError> {
        let Self {
            accepted,
            component_contract,
            current_revision,
            expires_at,
            phase,
        } = self;
        if accepted.len() > MAX_ACCEPTED_OUTCOMES {
            return Err(size_error());
        }
        let mut outcomes = VecDeque::with_capacity(accepted.len());
        for entry in accepted {
            outcomes.push_back(entry.into_metadata()?);
        }
        Ok(InstanceRecord {
            component_contract: component_contract
                .as_deref()
                .map(decode_digest)
                .transpose()?,
            current_revision: decode_revision(&current_revision)?,
            expires_at: decode_millis(&expires_at)?,
            phase: phase.into_phase()?,
            accepted: outcomes,
        })
    }
}

/// The lifecycle phase, tagged by the closed `kind` the grammar names.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case", tag = "kind")]
enum PhaseV1 {
    Ready,
    Pending {
        base_revision: String,
        claim_id: String,
        idempotency_key: String,
        lease_expires_at: String,
        request_digest: String,
        successor_revision: String,
    },
    Consumed {
        claim_id: Option<String>,
        reason: ReasonV1,
    },
}

impl PhaseV1 {
    fn from_phase(phase: &InstancePhase) -> Self {
        match phase {
            InstancePhase::Ready => Self::Ready,
            InstancePhase::Pending(pending) => Self::Pending {
                base_revision: decimal(pending.base_revision.get()),
                claim_id: decimal(pending.claim_id),
                idempotency_key: pending.idempotency_key.to_base64url(),
                lease_expires_at: decimal(pending.lease_expires_at.get()),
                request_digest: pending.request_digest.to_base64url(),
                successor_revision: decimal(pending.successor_revision.get()),
            },
            InstancePhase::Consumed { reason, claim_id } => Self::Consumed {
                claim_id: claim_id.map(decimal),
                reason: ReasonV1::from_reason(*reason),
            },
        }
    }

    fn into_phase(self) -> Result<InstancePhase, LedgerError> {
        match self {
            Self::Ready => Ok(InstancePhase::Ready),
            Self::Pending {
                base_revision,
                claim_id,
                idempotency_key,
                lease_expires_at,
                request_digest,
                successor_revision,
            } => Ok(InstancePhase::Pending(PendingClaim {
                claim_id: decode_decimal(&claim_id)?,
                base_revision: decode_revision(&base_revision)?,
                successor_revision: decode_revision(&successor_revision)?,
                idempotency_key: decode_idempotency(&idempotency_key)?,
                request_digest: decode_digest(&request_digest)?,
                lease_expires_at: decode_millis(&lease_expires_at)?,
            })),
            Self::Consumed { claim_id, reason } => Ok(InstancePhase::Consumed {
                reason: reason.into_reason(),
                claim_id: claim_id.as_deref().map(decode_decimal).transpose()?,
            }),
        }
    }
}

/// The closed fresh-render reason a consumed record retains.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
enum ReasonV1 {
    Missing,
    InstanceExpired,
    Consumed,
    ClaimExpired,
    RevisionExhausted,
}

impl ReasonV1 {
    const fn from_reason(reason: RefreshReason) -> Self {
        match reason {
            RefreshReason::Missing => Self::Missing,
            RefreshReason::InstanceExpired => Self::InstanceExpired,
            RefreshReason::Consumed => Self::Consumed,
            RefreshReason::ClaimExpired => Self::ClaimExpired,
            RefreshReason::RevisionExhausted => Self::RevisionExhausted,
        }
    }

    const fn into_reason(self) -> RefreshReason {
        match self {
            Self::Missing => RefreshReason::Missing,
            Self::InstanceExpired => RefreshReason::InstanceExpired,
            Self::Consumed => RefreshReason::Consumed,
            Self::ClaimExpired => RefreshReason::ClaimExpired,
            Self::RevisionExhausted => RefreshReason::RevisionExhausted,
        }
    }
}

/// One retained accepted outcome, which is what lets an exact duplicate
/// observe the outcome that already committed instead of executing again.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct AcceptedV1 {
    base_revision: String,
    idempotency_key: String,
    instance_id: String,
    outcome: OutcomeV1,
    request_digest: String,
    scope: String,
    successor_revision: String,
}

impl AcceptedV1 {
    fn from_metadata(metadata: &AcceptedOutcomeMetadata) -> Self {
        Self {
            base_revision: decimal(metadata.base_revision.get()),
            idempotency_key: metadata.idempotency_key.to_base64url(),
            instance_id: metadata.instance_id.to_base64url(),
            outcome: OutcomeV1 {
                digest: metadata.outcome.digest().to_base64url(),
                kind: OutcomeKindV1::from_kind(metadata.outcome.kind()),
            },
            request_digest: metadata.request_digest.to_base64url(),
            scope: metadata.scope.to_base64url(),
            successor_revision: decimal(metadata.successor_revision.get()),
        }
    }

    fn into_metadata(self) -> Result<AcceptedOutcomeMetadata, LedgerError> {
        Ok(AcceptedOutcomeMetadata {
            scope: decode_scope(&self.scope)?,
            instance_id: decode_instance(&self.instance_id)?,
            base_revision: decode_revision(&self.base_revision)?,
            successor_revision: decode_revision(&self.successor_revision)?,
            idempotency_key: decode_idempotency(&self.idempotency_key)?,
            request_digest: decode_digest(&self.request_digest)?,
            outcome: AcceptedOutcome::new(
                self.outcome.kind.into_kind(),
                decode_digest(&self.outcome.digest)?,
            ),
        })
    }
}

/// The bounded category and digest one accepted outcome retained.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OutcomeV1 {
    digest: String,
    kind: OutcomeKindV1,
}

/// The closed accepted-outcome category.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
enum OutcomeKindV1 {
    Rendered,
    Validation,
    NoRender,
    Redirect,
    Recovery,
}

impl OutcomeKindV1 {
    const fn from_kind(kind: AcceptedOutcomeKind) -> Self {
        match kind {
            AcceptedOutcomeKind::Rendered => Self::Rendered,
            AcceptedOutcomeKind::Validation => Self::Validation,
            AcceptedOutcomeKind::NoRender => Self::NoRender,
            AcceptedOutcomeKind::Redirect => Self::Redirect,
            AcceptedOutcomeKind::Recovery => Self::Recovery,
        }
    }

    const fn into_kind(self) -> AcceptedOutcomeKind {
        match self {
            Self::Rendered => AcceptedOutcomeKind::Rendered,
            Self::Validation => AcceptedOutcomeKind::Validation,
            Self::NoRender => AcceptedOutcomeKind::NoRender,
            Self::Redirect => AcceptedOutcomeKind::Redirect,
            Self::Recovery => AcceptedOutcomeKind::Recovery,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes<const LENGTH: usize>(start: u8) -> [u8; LENGTH] {
        std::array::from_fn(|offset| start.wrapping_add(offset as u8))
    }

    fn scope(start: u8) -> ScopeFingerprint {
        ScopeFingerprint::from_bytes(&bytes::<32>(start)).expect("scope is valid")
    }

    fn instance(start: u8) -> InstanceId {
        InstanceId::from_bytes(&bytes::<16>(start)).expect("instance is valid")
    }

    fn idempotency(start: u8) -> IdempotencyKey {
        IdempotencyKey::from_bytes(&bytes::<16>(start)).expect("idempotency key is valid")
    }

    fn digest(start: u8) -> ContentDigest {
        ContentDigest::from_bytes(&bytes::<32>(start)).expect("digest is valid")
    }

    fn metadata(start: u8, base: u64) -> AcceptedOutcomeMetadata {
        AcceptedOutcomeMetadata {
            scope: scope(start),
            instance_id: instance(start),
            base_revision: Revision::new(base),
            successor_revision: Revision::new(base + 1),
            idempotency_key: idempotency(start),
            request_digest: digest(start.wrapping_add(1)),
            outcome: AcceptedOutcome::new(
                AcceptedOutcomeKind::Recovery,
                digest(start.wrapping_add(2)),
            ),
        }
    }

    fn record(phase: InstancePhase, accepted: usize) -> InstanceRecord {
        InstanceRecord {
            component_contract: Some(digest(0x10)),
            current_revision: Revision::new(7),
            expires_at: UnixMillis::new(5_000),
            phase,
            accepted: (0..accepted)
                .map(|index| metadata(0x20_u8.wrapping_add(index as u8), index as u64))
                .collect::<VecDeque<_>>(),
        }
    }

    fn pending() -> InstancePhase {
        InstancePhase::Pending(PendingClaim {
            claim_id: 42,
            base_revision: Revision::new(6),
            successor_revision: Revision::new(7),
            idempotency_key: idempotency(0x30),
            request_digest: digest(0x40),
            lease_expires_at: UnixMillis::new(4_000),
        })
    }

    fn frame(body: &str) -> Vec<u8> {
        let mut bytes = vec![RECORD_VERSION];
        bytes.extend_from_slice(body.as_bytes());
        bytes
    }

    fn assert_same_phase(decoded: &InstancePhase, original: &InstancePhase) {
        match (decoded, original) {
            (InstancePhase::Ready, InstancePhase::Ready) => {}
            (InstancePhase::Pending(decoded), InstancePhase::Pending(original)) => {
                assert_eq!(decoded.claim_id, original.claim_id);
                assert_eq!(decoded.base_revision, original.base_revision);
                assert_eq!(decoded.successor_revision, original.successor_revision);
                assert_eq!(decoded.idempotency_key, original.idempotency_key);
                assert_eq!(decoded.request_digest, original.request_digest);
                assert_eq!(decoded.lease_expires_at, original.lease_expires_at);
            }
            (
                InstancePhase::Consumed {
                    reason: decoded_reason,
                    claim_id: decoded_claim,
                },
                InstancePhase::Consumed {
                    reason: original_reason,
                    claim_id: original_claim,
                },
            ) => {
                assert_eq!(decoded_reason, original_reason);
                assert_eq!(decoded_claim, original_claim);
            }
            _ => panic!("the decoded phase is not the phase that was encoded"),
        }
    }

    #[test]
    fn a_record_in_every_phase_round_trips_byte_exactly() {
        let phases = [
            InstancePhase::Ready,
            pending(),
            InstancePhase::Consumed {
                reason: RefreshReason::ClaimExpired,
                claim_id: Some(9),
            },
            InstancePhase::Consumed {
                reason: RefreshReason::RevisionExhausted,
                claim_id: None,
            },
        ];

        for phase in phases {
            let original = record(phase, 2);
            let encoded = encode_record(&original).expect("the record encodes");
            let decoded = decode_record(&encoded).expect("the record decodes");

            assert_eq!(
                encode_record(&decoded).expect("the decoded record encodes"),
                encoded,
                "a decoded record re-encodes to exactly the bytes it came from"
            );
            assert_eq!(decoded.component_contract, original.component_contract);
            assert_eq!(decoded.current_revision, original.current_revision);
            assert_eq!(decoded.expires_at, original.expires_at);
            assert_eq!(decoded.accepted, original.accepted);
            assert_same_phase(&decoded.phase, &original.phase);
        }
    }

    #[test]
    fn a_record_with_no_component_contract_and_no_outcomes_round_trips() {
        let original = InstanceRecord {
            component_contract: None,
            current_revision: Revision::new(0),
            expires_at: UnixMillis::new(1),
            phase: InstancePhase::Ready,
            accepted: VecDeque::new(),
        };

        let encoded = encode_record(&original).expect("the record encodes");
        let decoded = decode_record(&encoded).expect("the record decodes");

        assert!(decoded.component_contract.is_none());
        assert!(decoded.accepted.is_empty());
        assert_eq!(decoded.current_revision, Revision::new(0));
        assert_eq!(
            encode_record(&decoded).expect("the decoded record encodes"),
            encoded
        );
    }

    #[test]
    fn the_frame_is_the_version_byte_and_then_canonical_json() {
        let encoded = encode_record(&InstanceRecord {
            component_contract: None,
            current_revision: Revision::new(7),
            expires_at: UnixMillis::new(5_000),
            phase: InstancePhase::Ready,
            accepted: VecDeque::new(),
        })
        .expect("the record encodes");

        assert_eq!(encoded.first().copied(), Some(RECORD_VERSION));
        assert_eq!(
            std::str::from_utf8(&encoded[1..]).expect("the body is UTF-8"),
            concat!(
                r#"{"accepted":[],"component_contract":null,"current_revision":"7","#,
                r#""expires_at":"5000","phase":{"kind":"ready"}}"#
            ),
            "the body is canonical JSON with sorted keys and decimal-string integers"
        );
    }

    #[test]
    fn a_frame_with_another_version_byte_is_not_decoded() {
        let mut encoded = encode_record(&record(InstancePhase::Ready, 1)).expect("encodes");
        encoded[0] = RECORD_VERSION.wrapping_add(1);

        assert_eq!(
            decode_record(&encoded)
                .expect_err("an unknown record version is not read")
                .kind(),
            LedgerErrorKind::InvalidConfiguration
        );
        assert_eq!(
            decode_record(&[])
                .expect_err("an empty frame carries no version")
                .kind(),
            LedgerErrorKind::InvalidConfiguration
        );
    }

    #[test]
    fn a_frame_longer_than_the_byte_bound_is_rejected_before_it_is_parsed() {
        assert_eq!(
            decode_record(&vec![RECORD_VERSION; MAX_RECORD_BYTES + 1])
                .expect_err("the bound holds before any allocation")
                .kind(),
            LedgerErrorKind::CapacityExceeded
        );
    }

    #[test]
    fn more_accepted_outcomes_than_the_ledger_retains_is_a_capacity_failure() {
        assert_eq!(
            encode_record(&record(InstancePhase::Ready, MAX_ACCEPTED_OUTCOMES + 1))
                .expect_err("the retained-outcome bound holds")
                .kind(),
            LedgerErrorKind::CapacityExceeded
        );
    }

    #[test]
    fn the_byte_bound_stops_short_of_the_retained_outcome_ceiling() {
        // One retained outcome costs about 333 bytes of canonical JSON: five
        // base64url identities plus their field names. `MAX_RECORD_BYTES`
        // therefore holds 48 of them, fewer than the `MAX_ACCEPTED_OUTCOMES`
        // a `LedgerLimits` may configure, and a record at that ceiling is
        // reported rather than truncated. This pins the budget: changing
        // either bound, or a field name, moves this line.
        let largest = encode_record(&record(pending(), 48)).expect("the record encodes");
        assert!(
            largest.len() <= MAX_RECORD_BYTES,
            "48 retained outcomes fit the record bound, at {} bytes",
            largest.len()
        );
        assert_eq!(
            encode_record(&record(pending(), MAX_ACCEPTED_OUTCOMES))
                .expect_err("the retention ceiling does not fit the record bound")
                .kind(),
            LedgerErrorKind::CapacityExceeded
        );
    }

    #[test]
    fn decoding_arbitrary_bytes_reports_a_bound_or_a_shape_and_never_panics() {
        let valid = encode_record(&record(pending(), 2)).expect("the record encodes");
        let mut inputs: Vec<Vec<u8>> = vec![
            Vec::new(),
            vec![RECORD_VERSION],
            vec![0],
            vec![RECORD_VERSION, b'{'],
            vec![RECORD_VERSION, b'n', b'u', b'l', b'l'],
            vec![RECORD_VERSION, 0xff, 0xfe],
            vec![RECORD_VERSION; MAX_RECORD_BYTES + 1],
        ];
        let mut nested = vec![RECORD_VERSION];
        nested.extend(std::iter::repeat_n(b'[', 512));
        nested.extend(std::iter::repeat_n(b']', 512));
        inputs.push(nested);
        for cut in 0..valid.len() {
            inputs.push(valid[..cut].to_vec());
        }
        for index in 0..valid.len() {
            let mut flipped = valid.clone();
            flipped[index] ^= 0x80;
            inputs.push(flipped);
        }

        for input in inputs {
            match decode_record(&input) {
                Ok(decoded) => {
                    encode_record(&decoded).expect("anything that decodes re-encodes");
                }
                Err(error) => assert!(
                    matches!(
                        error.kind(),
                        LedgerErrorKind::InvalidConfiguration | LedgerErrorKind::CapacityExceeded
                    ),
                    "a rejected record names a bound or a shape, not {:?}",
                    error.kind()
                ),
            }
        }
    }

    #[test]
    fn a_field_the_phase_grammar_does_not_name_is_rejected_too() {
        let encoded = encode_record(&record(pending(), 0)).expect("the record encodes");
        let body = std::str::from_utf8(&encoded[1..])
            .expect("the body is UTF-8")
            .replace(r#""phase":{"#, r#""phase":{"extra":"x","#);

        assert_eq!(
            decode_record(&frame(&body))
                .expect_err("the phase grammar is closed")
                .kind(),
            LedgerErrorKind::InvalidConfiguration
        );
    }

    #[test]
    fn a_record_whose_shape_or_identities_are_wrong_is_rejected() {
        let bad = [
            // An unknown field.
            concat!(
                r#"{"accepted":[],"component_contract":null,"current_revision":"7","#,
                r#""expires_at":"5000","extra":"x","phase":{"kind":"ready"}}"#
            ),
            // A missing field.
            r#"{"accepted":[],"component_contract":null,"current_revision":"7","expires_at":"5000"}"#,
            // A non-canonical decimal.
            concat!(
                r#"{"accepted":[],"component_contract":null,"current_revision":"07","#,
                r#""expires_at":"5000","phase":{"kind":"ready"}}"#
            ),
            // An integer where the codec writes a decimal string.
            concat!(
                r#"{"accepted":[],"component_contract":null,"current_revision":7,"#,
                r#""expires_at":"5000","phase":{"kind":"ready"}}"#
            ),
            // A digest that is not 32 bytes of base64url.
            concat!(
                r#"{"accepted":[],"component_contract":"AAAA","current_revision":"7","#,
                r#""expires_at":"5000","phase":{"kind":"ready"}}"#
            ),
            // A phase the closed grammar does not name.
            concat!(
                r#"{"accepted":[],"component_contract":null,"current_revision":"7","#,
                r#""expires_at":"5000","phase":{"kind":"leased"}}"#
            ),
            // A consumed phase whose reason is not a closed refresh reason.
            concat!(
                r#"{"accepted":[],"component_contract":null,"current_revision":"7","#,
                r#""expires_at":"5000","phase":{"claim_id":"1","kind":"consumed","reason":"why"}}"#
            ),
            // A claim identity that is not a canonical unsigned decimal.
            concat!(
                r#"{"accepted":[],"component_contract":null,"current_revision":"7","#,
                r#""expires_at":"5000","phase":{"claim_id":"-1","kind":"consumed","#,
                r#""reason":"consumed"}}"#
            ),
        ];

        for body in bad {
            let error = decode_record(&frame(body))
                .err()
                .unwrap_or_else(|| panic!("this record must not decode: {body}"));
            assert_eq!(
                error.kind(),
                LedgerErrorKind::InvalidConfiguration,
                "a malformed record names its shape: {body}"
            );
        }
    }
}
