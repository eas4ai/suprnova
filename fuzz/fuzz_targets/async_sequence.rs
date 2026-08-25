#![no_main]

use std::num::NonZeroU8;

use libfuzzer_sys::fuzz_target;
use suprnova_live::async_updates::{
    AsyncCodecLimits, AsyncEnvelopeContext, BaselineDisposition, BoundedEventContracts,
    BoundedPresentationSignalContracts, BoundedTargets, BrowserPayloadSchema, ContinuityProof,
    EventCyclePolicy, EventOrder, EventSource, EventTarget, SequenceDisposition, SequenceMachine,
    StreamEpoch, StreamName, StreamPosition, StreamSequence, SubscriptionEventContract,
    SubscriptionId, decode_async_envelope,
};
use suprnova_live::metadata::{EventMetadata, EventPayloadMetadata};

const MAX_TRANSITIONS: usize = 256;

struct FuzzEvent;

impl EventPayloadMetadata for FuzzEvent {
    const NAME: &'static str = "fuzz.event";
    const VERSION: u16 = 1;
    const PAYLOAD_CONTRACT: &'static str = "fuzz.event.payload";
    const SCHEMA: BrowserPayloadSchema = BrowserPayloadSchema::Json;
}

fn u64_at(bytes: &[u8], start: usize) -> u64 {
    let mut value = [0_u8; 8];
    if start >= bytes.len() {
        return 0;
    }
    let available = bytes.len().saturating_sub(start).min(8);
    value[..available].copy_from_slice(&bytes[start..start + available]);
    u64::from_le_bytes(value)
}

fn context() -> AsyncEnvelopeContext {
    let metadata = EventMetadata::from_payload_with_contract::<FuzzEvent>(
        EventSource::Stream,
        BoundedTargets::new(vec![EventTarget::SelfIsland]).expect("static target"),
        EventOrder::PerSourceSequence,
        EventCyclePolicy::MaximumHops(NonZeroU8::new(1).expect("static hop")),
        1,
    )
    .expect("static event metadata");
    AsyncEnvelopeContext::new(
        SubscriptionId::from_bytes(b"fuzz-subscription").expect("static subscription"),
        StreamName::parse("fuzz").expect("static stream"),
        BoundedEventContracts::new(vec![
            SubscriptionEventContract::from_registered(&metadata).expect("static event contract"),
        ])
        .expect("static event set"),
        BoundedPresentationSignalContracts::new(Vec::new()).expect("empty signal set"),
    )
}

fuzz_target!(|bytes: &[u8]| {
    if bytes.len() < 16 {
        return;
    }
    let baseline = StreamPosition::new(
        StreamEpoch::new(u64_at(bytes, 0)),
        StreamSequence::new(u64_at(bytes, 8)),
    );
    let mut machine = SequenceMachine::new(baseline);
    let subscription = SubscriptionId::from_bytes(b"fuzz-subscription")
        .expect("static subscription")
        .to_base64url();
    let context = context();
    let limits = AsyncCodecLimits::v1();

    for chunk in bytes[16..].chunks(17).take(MAX_TRANSITIONS) {
        let operation = chunk[0] % 3;
        let epoch = u64_at(chunk, 1);
        let sequence = u64_at(chunk, 9);
        let position = StreamPosition::new(StreamEpoch::new(epoch), StreamSequence::new(sequence));
        let before = machine.current();
        match operation {
            0 => {
                let encoded = format!(
                    "{{\"payload\":{{\"kind\":\"heartbeat\"}},\"position\":{{\"epoch\":\"{epoch}\",\"sequence\":\"{sequence}\"}},\"protocol_version\":1,\"stream\":\"fuzz\",\"subscription\":\"{subscription}\"}}"
                );
                let envelope = decode_async_envelope(encoded.as_bytes(), &limits, &context)
                    .expect("generated bounded heartbeat");
                match machine.observe(&envelope) {
                    SequenceDisposition::Apply => {
                        assert_eq!(machine.current(), position);
                        assert_eq!(position.epoch(), before.epoch());
                        assert_eq!(
                            position.sequence().get(),
                            before.sequence().get().checked_add(1).expect("applied successor"),
                        );
                    }
                    SequenceDisposition::Degraded(_)
                    | SequenceDisposition::AwaitingRecovery
                    | SequenceDisposition::IgnoreDuplicate
                    | SequenceDisposition::IgnoreStaleEpoch => {
                        assert_eq!(machine.current(), before);
                    }
                }
            }
            1 => {
                let result = machine.adopt(ContinuityProof::Replay {
                    from: before,
                    through: position,
                });
                if result.is_ok() {
                    assert_eq!(position.epoch(), before.epoch());
                    assert!(position.sequence() >= before.sequence());
                }
            }
            _ => {
                let result = machine.adopt(ContinuityProof::AuthoritativeRefresh {
                    baseline: position,
                });
                if matches!(
                    result,
                    Ok(BaselineDisposition::Adopted | BaselineDisposition::AlreadyCurrent)
                ) {
                    assert!(
                        position.epoch() > before.epoch()
                            || (position.epoch() == before.epoch()
                                && position.sequence() >= before.sequence())
                    );
                }
            }
        }
    }
});
