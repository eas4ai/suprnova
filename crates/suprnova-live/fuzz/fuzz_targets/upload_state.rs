#![no_main]

use libfuzzer_sys::fuzz_target;
use suprnova_live::upload::{
    AcceptedChunk, TransitionDisposition, UploadChecksum, UploadHandle, UploadIdempotencyKey,
    UploadRevision, UploadState, UploadStateMachine, UploadTransition, UploadTransitionRequest,
};

const MAX_TRANSITIONS: usize = 256;
const PRIMARY_HANDLE: &str = "018f47c1-2af0-7cc4-a001-000000000001";
const OTHER_HANDLE: &str = "018f47c1-2af0-7cc4-a001-000000000002";
const CHECKSUM: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fuzz_target!(|data: &[u8]| {
    let primary = UploadHandle::parse(PRIMARY_HANDLE).expect("fixed primary handle is valid");
    let other = UploadHandle::parse(OTHER_HANDLE).expect("fixed alternate handle is valid");
    let checksum = UploadChecksum::parse(CHECKSUM).expect("fixed checksum is valid");
    let mut machine = UploadStateMachine::new(
        primary.clone(),
        UploadState::Created,
        UploadRevision::initial(),
    );
    let mut last_applied: Option<UploadTransitionRequest> = None;

    for (step, bytes) in data.chunks(2).take(MAX_TRANSITIONS).enumerate() {
        let selector = bytes[0];
        let flags = bytes.get(1).copied().unwrap_or_default();
        let before_state = machine.state();
        let before_revision = machine.revision();

        if flags & 0x80 != 0
            && let Some(request) = last_applied.clone()
        {
            let replay = machine
                .apply(request)
                .expect("an exact retained idempotency replay remains accepted");
            assert_eq!(replay.disposition(), TransitionDisposition::ExistingOutcome);
            assert_eq!(machine.state(), before_state);
            assert_eq!(machine.revision(), before_revision);
            continue;
        }

        let transition = transition(selector, step as u32, checksum.clone());
        let wrong_handle = flags & 0x01 != 0;
        let stale_revision = flags & 0x02 != 0;
        let expected_revision = if stale_revision {
            UploadRevision::new(before_revision.get().saturating_sub(1))
        } else {
            before_revision
        };
        let idempotency_key = UploadIdempotencyKey::parse(&format!("fuzz-{step}"))
            .expect("bounded generated idempotency key is valid");
        let request = UploadTransitionRequest::new(
            if wrong_handle {
                other.clone()
            } else {
                primary.clone()
            },
            expected_revision,
            idempotency_key,
            transition,
        );
        let outcome = machine.apply(request.clone());

        assert!(machine.revision().get() >= before_revision.get());
        assert!(machine.state().rank() >= before_state.rank());
        if wrong_handle || stale_revision || before_state.is_terminal() {
            assert!(outcome.is_err());
            assert_eq!(machine.state(), before_state);
            assert_eq!(machine.revision(), before_revision);
            continue;
        }
        match outcome {
            Ok(applied) => {
                assert_eq!(applied.disposition(), TransitionDisposition::Applied);
                assert_eq!(applied.state(), machine.state());
                assert_eq!(applied.revision(), machine.revision());
                assert_eq!(applied.revision().get(), before_revision.get() + 1);
                last_applied = Some(request);
            }
            Err(_) => {
                assert_eq!(machine.state(), before_state);
                assert_eq!(machine.revision(), before_revision);
            }
        }
    }
});

fn transition(selector: u8, index: u32, checksum: UploadChecksum) -> UploadTransition {
    match selector % 11 {
        0 => UploadTransition::Queue,
        1 => UploadTransition::BeginTransfer,
        2 => UploadTransition::PutChunk(
            AcceptedChunk::new(index, u64::from(selector) + 1, checksum)
                .expect("generated accepted chunk is nonzero"),
        ),
        3 => UploadTransition::Complete,
        4 => UploadTransition::Accept,
        5 => UploadTransition::BeginFinalize,
        6 => UploadTransition::CommitFinalize,
        7 => UploadTransition::Cancel,
        8 => UploadTransition::Reject,
        9 => UploadTransition::Expire,
        _ => UploadTransition::Fail,
    }
}
