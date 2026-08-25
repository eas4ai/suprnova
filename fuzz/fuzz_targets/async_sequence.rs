#![no_main]

use libfuzzer_sys::fuzz_target;
use suprnova_live::async_updates::{
    AsyncCodecLimits, AsyncContinuityAuthorityPort, AsyncContinuityRequest, AsyncEnvelope,
    BaselineDisposition, SequenceDisposition, SequenceMachine, StreamEpoch, StreamPosition,
    StreamSequence, decode_async_envelope,
};

const MAX_TRANSITIONS: usize = 256;
const MAX_FUZZ_REPLAY: u64 = 32;

mod support;

struct FuzzContinuityAuthority(StreamPosition);

impl AsyncContinuityAuthorityPort for FuzzContinuityAuthority {
    fn authoritative_refresh(
        &self,
        _request: AsyncContinuityRequest<'_>,
    ) -> Option<StreamPosition> {
        Some(self.0)
    }
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

fn heartbeat(position: StreamPosition, limits: &AsyncCodecLimits) -> AsyncEnvelope {
    let subscription = support::async_subscription_id().to_base64url();
    let encoded = format!(
        "{{\"payload\":{{\"kind\":\"heartbeat\"}},\"position\":{{\"epoch\":\"{}\",\"sequence\":\"{}\"}},\"protocol_version\":1,\"stream\":\"fuzz\",\"subscription\":\"{subscription}\"}}",
        position.epoch().get(),
        position.sequence().get(),
    );
    decode_async_envelope(encoded.as_bytes(), limits, support::async_context())
        .expect("generated bounded heartbeat")
}

fuzz_target!(|bytes: &[u8]| {
    if bytes.len() < 16 {
        return;
    }
    let baseline = StreamPosition::new(
        StreamEpoch::new(u64_at(bytes, 0)),
        StreamSequence::new(u64_at(bytes, 8)),
    );
    let context = support::async_context();
    let mut machine = SequenceMachine::new(context, baseline);
    let subscription = support::async_subscription_id().to_base64url();
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
                let envelope = decode_async_envelope(encoded.as_bytes(), &limits, context)
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
                    | SequenceDisposition::IgnoreStaleEpoch
                    | SequenceDisposition::ScopeMismatch => {
                        assert_eq!(machine.current(), before);
                    }
                }
            }
            1 => {
                let transcript = machine.high_water().and_then(|high_water| {
                    if high_water.epoch() != before.epoch() {
                        return None;
                    }
                    let first = before.sequence().get().checked_add(1)?;
                    let distance = high_water.sequence().get().checked_sub(first)?;
                    if distance >= MAX_FUZZ_REPLAY {
                        return None;
                    }
                    Some(
                        (first..=high_water.sequence().get())
                            .map(|value| {
                                heartbeat(
                                    StreamPosition::new(
                                        before.epoch(),
                                        StreamSequence::new(value),
                                    ),
                                    &limits,
                                )
                            })
                            .collect::<Vec<_>>(),
                    )
                });
                let result = machine.recover_from_replay(transcript.as_deref().unwrap_or(&[]));
                if result.is_ok() {
                    assert_eq!(machine.current().epoch(), before.epoch());
                    assert!(machine.current().sequence() > before.sequence());
                }
            }
            _ => {
                let result = machine
                    .recover_from_authoritative_refresh(&FuzzContinuityAuthority(position));
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
