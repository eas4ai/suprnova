#![no_main]

use std::collections::VecDeque;
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use libfuzzer_sys::fuzz_target;
use suprnova_live::async_updates::{
    AsyncCodecLimits, AsyncContinuityAuthorityPort, AsyncContinuityRequest,
    AsyncDeliveryDisposition, AsyncDispatchError, AsyncEnvelope, AsyncEnvelopeDispatchPort,
    AsyncEventSession, AsyncEventSource, AsyncPolicy, AsyncTransportError, AsyncTransportFuture,
    BaselineDisposition, BoundedDocumentTransportSession, BufferDisposition, CloseDisposition,
    MAX_REPLAY_TRANSCRIPT_ENVELOPES, ResolvedAsyncDelivery, SequenceDisposition, SequenceState,
    StreamEpoch, StreamPosition, StreamSequence, decode_async_envelope,
};
use suprnova_live::resource::{PermitPool, ResourceBounds};

const MAX_TRANSITIONS: usize = 256;

mod support;

struct FuzzContinuityAuthority(StreamPosition);
struct AcceptingDispatcher;

impl AsyncEnvelopeDispatchPort for AcceptingDispatcher {
    fn dispatch(&mut self, _delivery: ResolvedAsyncDelivery<'_>) -> Result<(), AsyncDispatchError> {
        Ok(())
    }
}

impl AsyncContinuityAuthorityPort for FuzzContinuityAuthority {
    fn authoritative_refresh(
        &self,
        _request: AsyncContinuityRequest<'_>,
    ) -> Option<StreamPosition> {
        Some(self.0)
    }
}

struct QueueSource {
    baseline: StreamPosition,
    queue: Arc<Mutex<VecDeque<AsyncEnvelope>>>,
}

impl AsyncEventSource for QueueSource {
    fn subscribe<'a>(
        &'a self,
        _request: &'a suprnova_live::async_updates::AuthorizedTransportSubscription,
    ) -> AsyncTransportFuture<'a, Result<Pin<Box<dyn AsyncEventSession>>, AsyncTransportError>> {
        Box::pin(async move {
            Ok(Box::pin(QueueSession {
                baseline: self.baseline,
                queue: Arc::clone(&self.queue),
                closed: false,
            }) as Pin<Box<dyn AsyncEventSession>>)
        })
    }
}

struct QueueSession {
    baseline: StreamPosition,
    queue: Arc<Mutex<VecDeque<AsyncEnvelope>>>,
    closed: bool,
}

impl AsyncEventSession for QueueSession {
    fn baseline(&self) -> StreamPosition {
        self.baseline
    }

    fn poll_next(
        self: Pin<&mut Self>,
        _task: &mut Context<'_>,
    ) -> Poll<Result<Option<AsyncEnvelope>, AsyncTransportError>> {
        let this = self.get_mut();
        if this.closed {
            return Poll::Ready(Ok(None));
        }
        match this.queue.lock().expect("fuzz queue lock").pop_front() {
            Some(envelope) => Poll::Ready(Ok(Some(envelope))),
            None => Poll::Pending,
        }
    }

    fn poll_close(
        self: Pin<&mut Self>,
        _task: &mut Context<'_>,
    ) -> Poll<Result<CloseDisposition, AsyncTransportError>> {
        self.get_mut().closed = true;
        Poll::Ready(Ok(CloseDisposition::Closed))
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

fn heartbeat(
    position: StreamPosition,
    limits: &AsyncCodecLimits,
    context: &suprnova_live::async_updates::AsyncEnvelopeContext,
) -> AsyncEnvelope {
    let subscription = support::async_subscription_id().to_base64url();
    let encoded = format!(
        "{{\"payload\":{{\"kind\":\"heartbeat\"}},\"position\":{{\"epoch\":\"{}\",\"sequence\":\"{}\"}},\"protocol_version\":1,\"stream\":\"fuzz\",\"subscription\":\"{subscription}\"}}",
        position.epoch().get(),
        position.sequence().get(),
    );
    decode_async_envelope(encoded.as_bytes(), limits, context)
        .expect("generated bounded heartbeat")
}

fuzz_target!(|bytes: &[u8]| {
    if bytes.len() < 16 {
        return;
    }
    let support::FuzzTransportSetup {
        mut document,
        request,
        registry,
    } = support::async_transport_setup(bytes[0]);
    let queue = Arc::new(Mutex::new(VecDeque::new()));
    let source = QueueSource {
        baseline: request.baseline(),
        queue: Arc::clone(&queue),
    };
    let pending = document.prepare_add(request.clone()).expect("fuzz prepare add");
    let authorized = support::block_on_ready(pending.authorize()).expect("fuzz authorize add");
    let establishing = document
        .prepare_establish(authorized)
        .expect("fuzz prepare establish");
    let ready = support::block_on_ready(establishing.establish(&source))
        .expect("fuzz establish source");
    document.commit_add(ready).expect("fuzz commit add");
    let mut bounded = BoundedDocumentTransportSession::new(
        document,
        ResourceBounds::new(64, 256 * 1024).expect("fuzz document bounds"),
        PermitPool::new(4).expect("fuzz permits"),
        AsyncPolicy {
            max_payload_bytes: NonZeroUsize::new(32 * 1024).expect("payload"),
            max_replay_events: NonZeroUsize::new(MAX_REPLAY_TRANSCRIPT_ENVELOPES)
                .expect("replay"),
            max_fanout: NonZeroUsize::new(100).expect("fanout"),
        },
    )
    .expect("fuzz bounded document");
    let limits = AsyncCodecLimits::hostile_test();
    let mut dispatcher = AcceptingDispatcher;

    for chunk in bytes[16..].chunks(17).take(MAX_TRANSITIONS) {
        let operation = chunk[0] % 3;
        let position = StreamPosition::new(
            StreamEpoch::new(u64_at(chunk, 1)),
            StreamSequence::new(u64_at(chunk, 9)),
        );
        let before = bounded
            .sequence_position(&request)
            .expect("active fuzz sequence lane");
        let state_before = bounded
            .sequence_state(&request)
            .expect("active fuzz sequence state");
        match operation {
            0 => {
                queue
                    .lock()
                    .expect("fuzz queue lock")
                    .push_back(heartbeat(position, &limits, request.context()));
                if matches!(
                    support::block_on_ready(bounded.pump_next(registry.as_ref())),
                    Ok(Some(BufferDisposition::Queued | BufferDisposition::Coalesced))
                ) {
                    match bounded.dispatch_next(registry.as_ref(), &mut dispatcher) {
                        Ok(Some(AsyncDeliveryDisposition::Sequence(
                            SequenceDisposition::Apply,
                        ))) => {
                            assert_eq!(position.epoch(), before.epoch());
                            assert_eq!(
                                before.sequence().get().checked_add(1),
                                Some(position.sequence().get()),
                            );
                            assert_eq!(bounded.sequence_position(&request), Some(position));
                            assert_eq!(
                                bounded.sequence_state(&request),
                                Some(SequenceState::Current),
                            );
                        }
                        Ok(Some(AsyncDeliveryDisposition::Sequence(
                            SequenceDisposition::Degraded(_) | SequenceDisposition::AwaitingRecovery,
                        ))) => {
                            assert_eq!(bounded.sequence_position(&request), Some(before));
                            assert_eq!(
                                bounded.sequence_state(&request),
                                Some(SequenceState::Degraded),
                            );
                        }
                        Ok(Some(AsyncDeliveryDisposition::Sequence(_))) | Ok(None) | Err(_) => {
                            assert_eq!(bounded.sequence_position(&request), Some(before));
                            assert_eq!(bounded.sequence_state(&request), Some(state_before));
                        }
                        Ok(Some(AsyncDeliveryDisposition::Replay(_))) => {
                            panic!("ordinary envelope dispatch returned a replay outcome");
                        }
                    }
                }
            }
            1 => {
                if position.epoch() != before.epoch()
                    || position.sequence() <= before.sequence()
                    || position.sequence().get() - before.sequence().get()
                        > MAX_REPLAY_TRANSCRIPT_ENVELOPES as u64
                {
                    continue;
                }
                let expected_applied = usize::try_from(
                    position.sequence().get() - before.sequence().get(),
                )
                .expect("fuzz replay is bounded to usize");
                let transcript = (before.sequence().get() + 1..=position.sequence().get())
                    .map(|value| {
                        heartbeat(
                            StreamPosition::new(before.epoch(), StreamSequence::new(value)),
                            &limits,
                            request.context(),
                        )
                    })
                    .collect::<Vec<_>>();
                if matches!(
                    bounded.admit_replay(&request, transcript, registry.as_ref()),
                    Ok(BufferDisposition::Queued)
                ) {
                    match bounded.dispatch_next(registry.as_ref(), &mut dispatcher) {
                        Ok(Some(AsyncDeliveryDisposition::Replay(outcome))) => {
                            assert_eq!(outcome.applied(), expected_applied);
                            assert_eq!(outcome.current(), position);
                            assert_eq!(outcome.state(), SequenceState::Current);
                            assert_eq!(bounded.sequence_position(&request), Some(position));
                            assert_eq!(
                                bounded.sequence_state(&request),
                                Some(SequenceState::Current),
                            );
                        }
                        Err(error) => {
                            if let Some(replay) = error.replay_error() {
                                assert!(replay.applied() <= expected_applied);
                                assert_eq!(replay.current().epoch(), before.epoch());
                                assert_eq!(
                                    replay.current().sequence().get(),
                                    before.sequence().get() + replay.applied() as u64,
                                );
                                assert_eq!(
                                    bounded.sequence_position(&request),
                                    Some(replay.current()),
                                );
                            } else {
                                assert_eq!(bounded.sequence_position(&request), Some(before));
                                assert_eq!(bounded.sequence_state(&request), Some(state_before));
                            }
                        }
                        Ok(None) => {
                            assert_eq!(bounded.sequence_position(&request), Some(before));
                        }
                        Ok(Some(AsyncDeliveryDisposition::Sequence(_))) => {
                            panic!("admitted replay dispatched as an ordinary envelope");
                        }
                    }
                }
            }
            _ => {
                let result = bounded.recover_from_authoritative_refresh(
                    &request,
                    registry.as_ref(),
                    &FuzzContinuityAuthority(position),
                );
                if matches!(
                    result,
                    Ok(BaselineDisposition::Adopted | BaselineDisposition::AlreadyCurrent)
                ) {
                    assert!(
                        position.epoch() > before.epoch()
                            || (position.epoch() == before.epoch()
                                  && position.sequence() >= before.sequence())
                    );
                    assert_eq!(bounded.sequence_position(&request), Some(position));
                    assert_eq!(
                        bounded.sequence_state(&request),
                        Some(SequenceState::Current),
                    );
                } else {
                    assert_eq!(bounded.sequence_position(&request), Some(before));
                    assert_eq!(bounded.sequence_state(&request), Some(state_before));
                }
            }
        }
    }
});
