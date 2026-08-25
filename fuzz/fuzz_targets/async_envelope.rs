#![no_main]

use std::num::NonZeroU8;

use libfuzzer_sys::fuzz_target;
use suprnova_live::async_updates::{
    AsyncCodecLimits, AsyncEnvelopeContext, BoundedEventContracts,
    BoundedPresentationSignalContracts, BoundedTargets, BrowserPayloadSchema, EventCyclePolicy,
    EventOrder, EventSource, EventTarget, StreamName, SubscriptionEventContract, SubscriptionId,
    decode_async_envelope, encode_async_envelope,
};
use suprnova_live::metadata::{EventMetadata, EventPayloadMetadata};

struct FuzzEvent;

impl EventPayloadMetadata for FuzzEvent {
    const NAME: &'static str = "fuzz.event";
    const VERSION: u16 = 1;
    const PAYLOAD_CONTRACT: &'static str = "fuzz.event.payload";
    const SCHEMA: BrowserPayloadSchema = BrowserPayloadSchema::Json;
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
    let limits = AsyncCodecLimits::v1();
    if let Ok(envelope) = decode_async_envelope(bytes, &limits, &context()) {
        assert_eq!(
            encode_async_envelope(&envelope, &limits).expect("validated envelope re-encodes"),
            bytes,
        );
    }
});
