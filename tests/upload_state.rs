//! Revisioned upload state-machine and fixture contract tests.

use proptest::prelude::*;
use serde::Deserialize;
use std::num::NonZeroUsize;
use suprnova_live::upload::{
    AcceptedChunk, TransitionDisposition, UploadChecksum, UploadErrorKind, UploadHandle,
    UploadIdempotencyKey, UploadRevision, UploadState, UploadStateMachine, UploadTransition,
    UploadTransitionRequest,
};

const HANDLE: &str = "018f47c1-2af0-7cc4-a001-000000000001";
const OTHER_HANDLE: &str = "018f8f3a-7b2c-4d5e-8f90-abcdef012345";

#[derive(Deserialize)]
struct Fixture {
    states: Vec<String>,
    terminal_states: Vec<String>,
    transition_cases: Vec<TransitionCase>,
}

#[derive(Deserialize)]
struct TransitionCase {
    id: String,
    from: String,
    operation: String,
    chunk_index: Option<u32>,
    idempotency_key: Option<String>,
    expected_revision: String,
    to: String,
    next_revision: String,
    expected: String,
}

fn fixture() -> Fixture {
    serde_json::from_str(include_str!("../fixtures/v4/upload-protocol.json"))
        .expect("upload fixture")
}

fn handle() -> UploadHandle {
    UploadHandle::parse(HANDLE).expect("fixture handle")
}

fn key(value: &str) -> UploadIdempotencyKey {
    UploadIdempotencyKey::parse(value).expect("idempotency key")
}

fn transition(case: &TransitionCase) -> UploadTransition {
    match case.operation.as_str() {
        "queue" => UploadTransition::Queue,
        "begin_transfer" => UploadTransition::BeginTransfer,
        "put_chunk" => UploadTransition::PutChunk(
            AcceptedChunk::new(
                case.chunk_index.expect("chunk index"),
                262_144,
                UploadChecksum::parse(
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                )
                .expect("checksum"),
            )
            .expect("accepted chunk"),
        ),
        "complete" => UploadTransition::Complete,
        "accept" => UploadTransition::Accept,
        "begin_finalize" => UploadTransition::BeginFinalize,
        "commit_finalize" => UploadTransition::CommitFinalize,
        "cancel" => UploadTransition::Cancel,
        "reject" => UploadTransition::Reject,
        "expire" => UploadTransition::Expire,
        other => panic!("unmapped fixture transition {other}"),
    }
}

fn request(case: &TransitionCase) -> UploadTransitionRequest {
    UploadTransitionRequest::new(
        handle(),
        UploadRevision::parse(&case.expected_revision).expect("revision"),
        key(case.idempotency_key.as_deref().unwrap_or(&case.id)),
        transition(case),
    )
}

#[test]
fn locked_v4_transition_cases_have_exact_typed_outcomes() {
    let fixture = fixture();

    assert_eq!(
        fixture.states,
        UploadState::ALL
            .iter()
            .map(|state| state.as_str().to_owned())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        fixture.terminal_states,
        UploadState::ALL
            .iter()
            .filter(|state| state.is_terminal())
            .map(|state| state.as_str().to_owned())
            .collect::<Vec<_>>()
    );

    for case in fixture.transition_cases {
        let expected_revision = UploadRevision::parse(&case.expected_revision).expect("expected");
        let next_revision = UploadRevision::parse(&case.next_revision).expect("next");
        let from = UploadState::parse(&case.from).expect("from state");

        match case.expected.as_str() {
            "applied" => {
                let mut machine = UploadStateMachine::new(handle(), from, expected_revision);
                let outcome = machine.apply(request(&case)).expect("applied transition");
                assert_eq!(
                    outcome.disposition(),
                    TransitionDisposition::Applied,
                    "{}",
                    case.id
                );
                assert_eq!(outcome.state().as_str(), case.to, "{}", case.id);
                assert_eq!(outcome.revision(), next_revision, "{}", case.id);
            }
            "existing_outcome" => {
                let mut machine = UploadStateMachine::new(handle(), from, expected_revision);
                let first = machine.apply(request(&case)).expect("first application");
                let duplicate = machine.apply(request(&case)).expect("duplicate replay");
                assert_eq!(first.disposition(), TransitionDisposition::Applied);
                assert_eq!(
                    duplicate.disposition(),
                    TransitionDisposition::ExistingOutcome
                );
                assert_eq!(duplicate.state().as_str(), case.to);
                assert_eq!(duplicate.revision(), next_revision);
            }
            "conflict" => {
                let mut machine = UploadStateMachine::new(handle(), from, next_revision);
                assert_eq!(
                    machine
                        .apply(request(&case))
                        .expect_err("stale conflict")
                        .kind(),
                    UploadErrorKind::UploadConflict
                );
            }
            other => panic!("unknown expected disposition {other}"),
        }
    }
}

#[test]
fn cross_handle_stale_alternative_and_terminal_retry_rules_fail_closed() {
    let mut machine =
        UploadStateMachine::new(handle(), UploadState::Finalizing, UploadRevision::new(7));
    let request = UploadTransitionRequest::new(
        handle(),
        UploadRevision::new(7),
        key("commit-01"),
        UploadTransition::CommitFinalize,
    );
    let applied = machine.apply(request.clone()).expect("commit finalization");
    assert_eq!(applied.state(), UploadState::Finalized);
    assert_eq!(
        machine
            .apply(request)
            .expect("terminal duplicate")
            .disposition(),
        TransitionDisposition::ExistingOutcome
    );

    let stale_alternative = UploadTransitionRequest::new(
        handle(),
        UploadRevision::new(7),
        key("commit-02"),
        UploadTransition::CommitFinalize,
    );
    assert_eq!(
        machine
            .apply(stale_alternative)
            .expect_err("stale alternative")
            .kind(),
        UploadErrorKind::UploadConflict
    );

    let mut transferring =
        UploadStateMachine::new(handle(), UploadState::Transferring, UploadRevision::new(3));
    let cross_handle = UploadTransitionRequest::new(
        UploadHandle::parse(OTHER_HANDLE).expect("other handle"),
        UploadRevision::new(3),
        key("chunk-cross"),
        UploadTransition::PutChunk(
            AcceptedChunk::new(
                0,
                1,
                UploadChecksum::parse(
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                )
                .expect("checksum"),
            )
            .expect("accepted chunk"),
        ),
    );
    assert_eq!(
        transferring
            .apply(cross_handle)
            .expect_err("cross-handle chunk")
            .kind(),
        UploadErrorKind::ScopeMismatch
    );
}

#[test]
fn retained_idempotency_outcomes_have_a_configurable_hard_bound() {
    let mut machine = UploadStateMachine::with_outcome_limit(
        handle(),
        UploadState::Transferring,
        UploadRevision::new(3),
        NonZeroUsize::new(2).expect("non-zero"),
    )
    .expect("bounded history");
    for index in 0..2 {
        let request = UploadTransitionRequest::new(
            handle(),
            machine.revision(),
            key(&format!("bounded-{index}")),
            UploadTransition::PutChunk(
                AcceptedChunk::new(
                    index,
                    1,
                    UploadChecksum::parse(
                        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    )
                    .expect("checksum"),
                )
                .expect("accepted chunk"),
            ),
        );
        machine.apply(request).expect("within bound");
    }
    let overflow = UploadTransitionRequest::new(
        handle(),
        machine.revision(),
        key("bounded-overflow"),
        UploadTransition::PutChunk(
            AcceptedChunk::new(
                2,
                1,
                UploadChecksum::parse(
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                )
                .expect("checksum"),
            )
            .expect("accepted chunk"),
        ),
    );
    assert_eq!(
        machine.apply(overflow).expect_err("history bound").kind(),
        UploadErrorKind::IdempotencyHistoryFull
    );

    assert!(
        UploadStateMachine::with_outcome_limit(
            handle(),
            UploadState::Created,
            UploadRevision::initial(),
            NonZeroUsize::new(100_001).expect("non-zero"),
        )
        .is_err()
    );
}

#[test]
fn accepted_chunk_constructor_rejects_zero_bytes() {
    assert_eq!(
        AcceptedChunk::new(
            0,
            0,
            UploadChecksum::parse(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            )
            .expect("checksum"),
        )
        .expect_err("zero-byte accepted chunk")
        .kind(),
        UploadErrorKind::InvalidField
    );
}

proptest! {
    #[test]
    fn accepted_transitions_never_regress(choices in prop::collection::vec(any::<u8>(), 0..64)) {
        let mut machine = UploadStateMachine::new(
            handle(),
            UploadState::Created,
            UploadRevision::initial(),
        );

        for (index, choice) in choices.into_iter().enumerate() {
            let before_state = machine.state();
            let before_revision = machine.revision();
            let transition = match choice % 11 {
                0 => UploadTransition::Queue,
                1 => UploadTransition::BeginTransfer,
                2 => UploadTransition::PutChunk(AcceptedChunk::new(
                    index as u32,
                    1,
                    UploadChecksum::parse("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").expect("checksum"),
                ).expect("accepted chunk")),
                3 => UploadTransition::Complete,
                4 => UploadTransition::Accept,
                5 => UploadTransition::BeginFinalize,
                6 => UploadTransition::CommitFinalize,
                7 => UploadTransition::Cancel,
                8 => UploadTransition::Reject,
                9 => UploadTransition::Expire,
                _ => UploadTransition::Fail,
            };
            let request = UploadTransitionRequest::new(
                handle(),
                before_revision,
                key(&format!("property-{index}")),
                transition,
            );
            if let Ok(outcome) = machine.apply(request) {
                prop_assert!(outcome.revision() > before_revision);
                prop_assert!(outcome.state().rank() >= before_state.rank());
            }
        }
    }
}
