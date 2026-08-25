//! Server-side asynchronous fanout and bounded-buffer policy tests.

use std::num::NonZeroUsize;

use suprnova_live::async_updates::EventTarget;
use suprnova_live::async_updates::{
    AsyncBackpressure, AsyncBufferEntry, AsyncCloseCode, AsyncCodecLimits, AsyncEnvelope,
    AsyncEnvelopeContext, AsyncPayload, AsyncPolicy, AsyncTelemetryCounter, BrowserPayloadSchema,
    BufferDisposition, Heartbeat, MAX_ASYNC_BUFFER_BYTES, MAX_ASYNC_BUFFER_EVENTS,
    MAX_ASYNC_PAYLOAD_BYTES, MAX_EVENT_FANOUT, MAX_REPLAY_TRANSCRIPT_ENVELOPES,
    PresentationSignalContract, RegisteredBrowserEvent, RegisteredPresentationSignal,
    RegisteredRefresh, VerifiedOrigin, encode_async_envelope,
};
use suprnova_live::canonical::CanonicalValue;
use suprnova_live::identity::BrowserOperationName;
use suprnova_live::resource::{PermitPool, ResourceBounds, ResourceOwner, Retirement};

#[allow(
    dead_code,
    reason = "the shared Task 4 fixture exposes controls not needed by every focused suite"
)]
#[path = "support/async_transport.rs"]
mod support;

use support::{TransportFixture, position, subscription};

const KIB: usize = 1024;

fn policy() -> AsyncPolicy {
    AsyncPolicy {
        max_payload_bytes: NonZeroUsize::new(32 * KIB).expect("payload bound"),
        max_replay_events: NonZeroUsize::new(1_024).expect("replay bound"),
        max_fanout: NonZeroUsize::new(100).expect("fanout bound"),
    }
}

fn refresh(context: &AsyncEnvelopeContext, sequence: u64) -> AsyncEnvelope {
    AsyncEnvelope::new(
        context,
        position(7, sequence),
        AsyncPayload::Refresh(RegisteredRefresh),
    )
    .expect("registered refresh envelope")
}

fn heartbeat(context: &AsyncEnvelopeContext, sequence: u64) -> AsyncEnvelope {
    AsyncEnvelope::new(
        context,
        position(7, sequence),
        AsyncPayload::Heartbeat(Heartbeat),
    )
    .expect("heartbeat envelope")
}

fn browser_event(context: &AsyncEnvelopeContext, sequence: u64) -> AsyncEnvelope {
    let event = RegisteredBrowserEvent::new(
        context,
        BrowserOperationName::parse("orders.updated").expect("event name"),
        1,
        EventTarget::SelfIsland,
        CanonicalValue::Null,
    )
    .expect("registered browser event");
    AsyncEnvelope::new(
        context,
        position(7, sequence),
        AsyncPayload::BrowserEvent(event),
    )
    .expect("browser-event envelope")
}

fn presentation_signal(
    context: &AsyncEnvelopeContext,
    sequence: u64,
    name: &str,
    value: bool,
) -> AsyncEnvelope {
    let signal = RegisteredPresentationSignal::new(
        context,
        BrowserOperationName::parse(name).expect("signal name"),
        CanonicalValue::Bool(value),
    )
    .expect("registered presentation signal");
    AsyncEnvelope::new(
        context,
        position(7, sequence),
        AsyncPayload::PresentationSignal(signal),
    )
    .expect("presentation-signal envelope")
}

fn buffer(bounds: ResourceBounds, permits: usize, policy: AsyncPolicy) -> AsyncBackpressure {
    AsyncBackpressure::new(
        ResourceOwner::<AsyncBufferEntry>::new(bounds),
        PermitPool::new(permits).expect("delivery permits"),
        policy,
    )
    .expect("valid async pressure policy")
}

#[tokio::test]
async fn repeated_refresh_pressure_coalesces_inside_shared_resource_bounds() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let request = fixture.request(
        subscription(0x41),
        VerifiedOrigin::parse("https://example.test").expect("origin"),
    );
    let mut pressure = buffer(
        ResourceBounds::new(64, 256 * KIB).expect("document async bounds"),
        8,
        policy(),
    );

    for sequence in 1..=1_000 {
        let envelope = refresh(request.context(), sequence);
        let disposition = pressure.offer(envelope, 1);
        assert!(matches!(
            disposition,
            Ok(BufferDisposition::Queued | BufferDisposition::Coalesced)
        ));
    }

    assert_eq!(pressure.retained_events(), 1);
    assert!(pressure.retained_bytes() <= 256 * KIB);
    assert_eq!(pressure.active_permits(), 0);
    assert!(pressure.is_degraded());
}

#[tokio::test]
async fn registered_browser_events_never_coalesce_or_disappear_on_overflow() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let request = fixture.request(
        subscription(0x42),
        VerifiedOrigin::parse("https://example.test").expect("origin"),
    );
    let mut pressure = buffer(
        ResourceBounds::new(2, 256 * KIB).expect("event bounds"),
        2,
        policy(),
    );

    assert_eq!(
        pressure.offer(browser_event(request.context(), 1), 1),
        Ok(BufferDisposition::Queued)
    );
    assert_eq!(
        pressure.offer(browser_event(request.context(), 2), 1),
        Ok(BufferDisposition::Queued)
    );
    assert_eq!(
        pressure.offer(browser_event(request.context(), 3), 1),
        Ok(BufferDisposition::Degraded)
    );
    assert_eq!(pressure.retained_events(), 2);
    assert!(pressure.is_degraded());
}

#[tokio::test]
async fn fanout_is_rejected_against_registered_and_policy_authority_before_allocation() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let request = fixture.request(
        subscription(0x43),
        VerifiedOrigin::parse("https://example.test").expect("origin"),
    );
    let mut pressure = buffer(
        ResourceBounds::new(4, 256 * KIB).expect("fanout bounds"),
        4,
        policy(),
    );

    assert_eq!(
        pressure.offer(browser_event(request.context(), 1), 4),
        Ok(BufferDisposition::Queued)
    );
    assert_eq!(
        pressure.offer(browser_event(request.context(), 2), 5),
        Ok(BufferDisposition::Closed(AsyncCloseCode::FanoutExceeded))
    );
    assert_eq!(pressure.retained_events(), 0);
    assert_eq!(pressure.active_permits(), 0);
    assert_eq!(
        pressure.offer(browser_event(request.context(), 3), 1),
        Ok(BufferDisposition::Closed(AsyncCloseCode::FanoutExceeded))
    );
    assert_eq!(
        pressure
            .telemetry_snapshot()
            .count(AsyncTelemetryCounter::Closed),
        1
    );

    let mut policy_limited = buffer(
        ResourceBounds::new(1, 256 * KIB).expect("policy fanout bounds"),
        1,
        AsyncPolicy {
            max_fanout: NonZeroUsize::new(3).expect("policy fanout"),
            ..policy()
        },
    );
    assert_eq!(
        policy_limited.offer(browser_event(request.context(), 4), 4),
        Ok(BufferDisposition::Closed(AsyncCloseCode::FanoutExceeded))
    );
    assert_eq!(policy_limited.retained_events(), 0);
    assert_eq!(policy_limited.active_permits(), 0);
}

#[tokio::test]
async fn slow_delivery_holds_one_shared_permit_without_popping_a_sibling() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let request = fixture.request(
        subscription(0x44),
        VerifiedOrigin::parse("https://example.test").expect("origin"),
    );
    let mut pressure = buffer(
        ResourceBounds::new(2, 256 * KIB).expect("slow-client bounds"),
        1,
        policy(),
    );
    pressure
        .offer(browser_event(request.context(), 1), 1)
        .expect("first offer");
    pressure
        .offer(heartbeat(request.context(), 2), 1)
        .expect("second offer");

    let first = pressure
        .try_start_delivery()
        .expect("first delivery starts");
    assert_eq!(first.envelope().position(), position(7, 1));
    assert_eq!(pressure.active_permits(), 1);
    assert_eq!(pressure.retained_events(), 1);
    assert!(pressure.try_start_delivery().is_none());
    assert_eq!(pressure.retained_events(), 1);

    drop(first);
    let second = pressure
        .try_start_delivery()
        .expect("released permit admits sibling");
    assert_eq!(second.envelope().position(), position(7, 2));
    drop(second);
    assert_eq!(pressure.active_permits(), 0);
    assert_eq!(pressure.retained_events(), 0);
}

#[tokio::test]
async fn retirement_cancels_and_drains_retained_delivery_once() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let request = fixture.request(
        subscription(0x45),
        VerifiedOrigin::parse("https://example.test").expect("origin"),
    );
    let mut pressure = buffer(
        ResourceBounds::new(4, 256 * KIB).expect("retirement bounds"),
        2,
        policy(),
    );
    pressure
        .offer(browser_event(request.context(), 1), 1)
        .expect("queued event");
    let retained_bytes = pressure.retained_bytes();

    assert_eq!(
        pressure.retire(),
        Retirement {
            canceled: true,
            drained_items: 1,
            drained_bytes: retained_bytes,
        }
    );
    assert_eq!(pressure.retire(), Retirement::already_retired());
    assert_eq!(pressure.retained_events(), 0);
    assert_eq!(pressure.retained_bytes(), 0);
    assert_eq!(
        pressure.offer(browser_event(request.context(), 2), 1),
        Ok(BufferDisposition::Closed(AsyncCloseCode::Retired))
    );
}

#[tokio::test]
async fn external_owner_cancellation_stops_delivery_and_cleans_up_once() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let request = fixture.request(
        subscription(0x56),
        VerifiedOrigin::parse("https://example.test").expect("origin"),
    );
    let owner = ResourceOwner::<AsyncBufferEntry>::new(
        ResourceBounds::new(2, 256 * KIB).expect("cancellation bounds"),
    );
    let cancellation = owner.cancellation();
    let mut pressure = AsyncBackpressure::new(
        owner,
        PermitPool::new(1).expect("delivery permit"),
        policy(),
    )
    .expect("valid async pressure policy");
    pressure
        .offer(browser_event(request.context(), 1), 1)
        .expect("queued before cancellation");

    assert!(cancellation.cancel());
    assert_eq!(
        pressure.offer_replay(Vec::new()),
        Ok(BufferDisposition::Closed(AsyncCloseCode::Retired))
    );
    assert!(pressure.try_start_delivery().is_none());
    assert_eq!(pressure.retained_events(), 0);
    let telemetry = pressure.telemetry_snapshot();
    assert_eq!(telemetry.count(AsyncTelemetryCounter::Closed), 1);
    assert_eq!(telemetry.count(AsyncTelemetryCounter::Cleanup), 1);
}

#[tokio::test]
async fn payload_boundary_is_exact_and_distinct_from_envelope_bytes() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let request = fixture.request(
        subscription(0x46),
        VerifiedOrigin::parse("https://example.test").expect("origin"),
    );
    let envelope = refresh(request.context(), 1);
    let exact_payload_bytes = br#"{"kind":"refresh","name":"refresh"}"#.len();

    let mut exact = buffer(
        ResourceBounds::new(1, 256 * KIB).expect("exact payload bounds"),
        1,
        AsyncPolicy {
            max_payload_bytes: NonZeroUsize::new(exact_payload_bytes).expect("exact payload"),
            ..policy()
        },
    );
    assert_eq!(
        exact.offer(envelope.clone(), 1),
        Ok(BufferDisposition::Queued)
    );

    let mut first_rejected = buffer(
        ResourceBounds::new(1, 256 * KIB).expect("rejected payload bounds"),
        1,
        AsyncPolicy {
            max_payload_bytes: NonZeroUsize::new(exact_payload_bytes - 1)
                .expect("first rejected payload"),
            ..policy()
        },
    );
    assert_eq!(
        first_rejected.offer(envelope, 1),
        Ok(BufferDisposition::Closed(AsyncCloseCode::PayloadTooLarge))
    );
    assert_eq!(first_rejected.retained_events(), 0);
}

#[test]
fn invalid_policy_and_document_bounds_fail_before_delivery() {
    let invalid_payload = AsyncBackpressure::new(
        ResourceOwner::<AsyncBufferEntry>::new(
            ResourceBounds::new(1, 256 * KIB).expect("resource bounds"),
        ),
        PermitPool::new(1).expect("permit"),
        AsyncPolicy {
            max_payload_bytes: NonZeroUsize::new(MAX_ASYNC_PAYLOAD_BYTES + 1)
                .expect("invalid payload policy"),
            ..policy()
        },
    )
    .expect_err("payload policy above protocol cap");
    assert_eq!(invalid_payload.close_code(), AsyncCloseCode::InvalidPolicy);

    let invalid_replay = AsyncBackpressure::new(
        ResourceOwner::<AsyncBufferEntry>::new(
            ResourceBounds::new(1, 256 * KIB).expect("resource bounds"),
        ),
        PermitPool::new(1).expect("permit"),
        AsyncPolicy {
            max_replay_events: NonZeroUsize::new(MAX_REPLAY_TRANSCRIPT_ENVELOPES + 1)
                .expect("invalid replay policy"),
            ..policy()
        },
    )
    .expect_err("replay policy above protocol cap");
    assert_eq!(invalid_replay.close_code(), AsyncCloseCode::InvalidPolicy);

    let invalid_items = AsyncBackpressure::new(
        ResourceOwner::<AsyncBufferEntry>::new(
            ResourceBounds::new(MAX_ASYNC_BUFFER_EVENTS + 1, MAX_ASYNC_BUFFER_BYTES)
                .expect("shared resource allows wider generic bounds"),
        ),
        PermitPool::new(1).expect("permit"),
        policy(),
    )
    .expect_err("async document item cap");
    assert_eq!(invalid_items.close_code(), AsyncCloseCode::InvalidPolicy);

    let invalid_bytes = AsyncBackpressure::new(
        ResourceOwner::<AsyncBufferEntry>::new(
            ResourceBounds::new(1, MAX_ASYNC_BUFFER_BYTES + 1)
                .expect("shared resource allows wider generic byte bounds"),
        ),
        PermitPool::new(1).expect("permit"),
        policy(),
    )
    .expect_err("async document byte cap");
    assert_eq!(invalid_bytes.close_code(), AsyncCloseCode::InvalidPolicy);

    let invalid_fanout = AsyncBackpressure::new(
        ResourceOwner::<AsyncBufferEntry>::new(
            ResourceBounds::new(1, MAX_ASYNC_BUFFER_BYTES).expect("resource bounds"),
        ),
        PermitPool::new(1).expect("permit"),
        AsyncPolicy {
            max_fanout: NonZeroUsize::new(usize::from(MAX_EVENT_FANOUT) + 1)
                .expect("invalid fanout policy"),
            ..policy()
        },
    )
    .expect_err("fanout policy above registered engine cap");
    assert_eq!(invalid_fanout.close_code(), AsyncCloseCode::InvalidPolicy);
}

#[tokio::test]
async fn replay_preflight_rejects_empty_count_and_aggregate_bytes_without_partial_admission() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let request = fixture.request(
        subscription(0x47),
        VerifiedOrigin::parse("https://example.test").expect("origin"),
    );
    let replay_policy = AsyncPolicy {
        max_replay_events: NonZeroUsize::new(2).expect("replay bound"),
        ..policy()
    };
    let mut pressure = buffer(
        ResourceBounds::new(2, 256 * KIB).expect("replay resource bounds"),
        2,
        replay_policy,
    );

    assert_eq!(
        pressure.offer_replay(Vec::new()),
        Ok(BufferDisposition::Degraded)
    );
    assert_eq!(pressure.retained_events(), 0);

    assert_eq!(
        pressure.offer_replay(vec![
            (heartbeat(request.context(), 1), 1),
            (heartbeat(request.context(), 2), 1),
            (heartbeat(request.context(), 3), 1),
        ]),
        Ok(BufferDisposition::Degraded)
    );
    assert_eq!(pressure.retained_events(), 0);

    assert_eq!(
        pressure.offer_replay(vec![
            (heartbeat(request.context(), 1), 1),
            (heartbeat(request.context(), 2), 1),
        ]),
        Ok(BufferDisposition::Queued)
    );
    assert_eq!(pressure.retained_events(), 2);
}

#[tokio::test]
async fn replay_aggregate_byte_boundary_is_exact_and_first_over_is_atomic() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let request = fixture.request(
        subscription(0x51),
        VerifiedOrigin::parse("https://example.test").expect("origin"),
    );
    let first = heartbeat(request.context(), 1);
    let second = heartbeat(request.context(), 2);
    let aggregate = encode_async_envelope(&first, &AsyncCodecLimits::v1())
        .expect("first wire")
        .len()
        + encode_async_envelope(&second, &AsyncCodecLimits::v1())
            .expect("second wire")
            .len();

    let mut exact = buffer(
        ResourceBounds::new(2, aggregate).expect("exact replay bytes"),
        2,
        AsyncPolicy {
            max_replay_events: NonZeroUsize::new(2).expect("replay count"),
            ..policy()
        },
    );
    assert_eq!(
        exact.offer_replay(vec![(first.clone(), 1), (second.clone(), 1)]),
        Ok(BufferDisposition::Queued)
    );
    assert_eq!(exact.retained_bytes(), aggregate);

    let mut first_over = buffer(
        ResourceBounds::new(2, aggregate - 1).expect("first-over replay bytes"),
        2,
        AsyncPolicy {
            max_replay_events: NonZeroUsize::new(2).expect("replay count"),
            ..policy()
        },
    );
    assert_eq!(
        first_over.offer_replay(vec![(first, 1), (second, 1)]),
        Ok(BufferDisposition::Degraded)
    );
    assert_eq!(first_over.retained_events(), 0);
    assert_eq!(first_over.retained_bytes(), 0);
}

#[tokio::test]
async fn heartbeat_pressure_never_evicts_a_required_browser_event() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let request = fixture.request(
        subscription(0x52),
        VerifiedOrigin::parse("https://example.test").expect("origin"),
    );
    let mut pressure = buffer(
        ResourceBounds::new(1, 256 * KIB).expect("heartbeat pressure bounds"),
        1,
        policy(),
    );
    assert_eq!(
        pressure.offer(browser_event(request.context(), 1), 1),
        Ok(BufferDisposition::Queued)
    );
    assert_eq!(
        pressure.offer(heartbeat(request.context(), 2), 1),
        Ok(BufferDisposition::Degraded)
    );

    let retained = pressure
        .try_start_delivery()
        .expect("required browser event remains");
    assert!(matches!(
        retained.envelope().payload(),
        AsyncPayload::BrowserEvent(_)
    ));
}

#[tokio::test]
async fn global_outage_and_chatty_island_pressure_remain_independently_bounded() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let chatty_request = fixture.request(
        subscription(0x53),
        VerifiedOrigin::parse("https://example.test").expect("origin"),
    );
    let healthy_request = fixture.request(
        subscription(0x54),
        VerifiedOrigin::parse("https://example.test").expect("origin"),
    );
    let mut chatty = buffer(
        ResourceBounds::new(2, 256 * KIB).expect("chatty bounds"),
        1,
        policy(),
    );
    let mut healthy = buffer(
        ResourceBounds::new(2, 256 * KIB).expect("healthy bounds"),
        1,
        policy(),
    );

    for sequence in 1..=1_000 {
        let _ = chatty
            .offer(browser_event(chatty_request.context(), sequence), 1)
            .expect("bounded outage offer");
    }
    assert_eq!(chatty.retained_events(), 2);
    assert!(chatty.retained_bytes() <= 256 * KIB);
    assert!(chatty.is_degraded());

    assert_eq!(
        healthy.offer(browser_event(healthy_request.context(), 1), 1),
        Ok(BufferDisposition::Queued)
    );
    let delivery = healthy
        .try_start_delivery()
        .expect("healthy island progresses independently");
    assert_eq!(delivery.envelope().subscription(), &subscription(0x54));
    assert_eq!(chatty.retained_events(), 2);
}

#[tokio::test]
async fn coalescing_never_crosses_subscription_epoch_signal_or_sequence_gap() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    fixture.registry.set_presentation_signals(vec![
        PresentationSignalContract::new(
            BrowserOperationName::parse("open").expect("open signal"),
            BrowserPayloadSchema::Boolean,
        ),
        PresentationSignalContract::new(
            BrowserOperationName::parse("busy").expect("busy signal"),
            BrowserPayloadSchema::Boolean,
        ),
    ]);
    let first = fixture.request(
        subscription(0x48),
        VerifiedOrigin::parse("https://example.test").expect("origin"),
    );
    let second = fixture.request(
        subscription(0x49),
        VerifiedOrigin::parse("https://example.test").expect("origin"),
    );
    let mut pressure = buffer(
        ResourceBounds::new(8, 256 * KIB).expect("coalescing bounds"),
        8,
        policy(),
    );

    assert_eq!(
        pressure.offer(presentation_signal(first.context(), 1, "open", false), 1),
        Ok(BufferDisposition::Queued)
    );
    assert_eq!(
        pressure.offer(presentation_signal(first.context(), 2, "open", true), 1),
        Ok(BufferDisposition::Coalesced)
    );
    assert_eq!(pressure.retained_events(), 1);
    assert_eq!(
        pressure.offer(presentation_signal(first.context(), 4, "open", false), 1),
        Ok(BufferDisposition::Degraded)
    );
    assert_eq!(pressure.retained_events(), 1);

    assert_eq!(
        pressure.offer(presentation_signal(first.context(), 3, "busy", true), 1),
        Ok(BufferDisposition::Queued)
    );
    assert_eq!(
        pressure.offer(presentation_signal(second.context(), 1, "busy", true), 1),
        Ok(BufferDisposition::Queued)
    );
    assert_eq!(pressure.retained_events(), 3);
}

#[tokio::test]
async fn presentation_signal_coalescing_keeps_exact_registered_contract_identity() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let signal_name = BrowserOperationName::parse("status").expect("signal name");
    fixture
        .registry
        .set_presentation_signals(vec![PresentationSignalContract::new(
            signal_name.clone(),
            BrowserPayloadSchema::Boolean,
        )]);
    let logical_subscription = subscription(0x57);
    let first = fixture.request(
        logical_subscription.clone(),
        VerifiedOrigin::parse("https://example.test").expect("origin"),
    );
    let boolean_envelope = presentation_signal(first.context(), 1, "status", true);

    fixture
        .registry
        .set_presentation_signals(vec![PresentationSignalContract::new(
            signal_name.clone(),
            BrowserPayloadSchema::String,
        )]);
    let revised = fixture.request(
        logical_subscription,
        VerifiedOrigin::parse("https://example.test").expect("origin"),
    );
    let string_signal = RegisteredPresentationSignal::new(
        revised.context(),
        signal_name,
        CanonicalValue::String("ready".to_owned()),
    )
    .expect("registered string signal");
    let string_envelope = AsyncEnvelope::new(
        revised.context(),
        position(7, 2),
        AsyncPayload::PresentationSignal(string_signal),
    )
    .expect("string signal envelope");
    let mut pressure = buffer(
        ResourceBounds::new(2, 256 * KIB).expect("contract isolation bounds"),
        2,
        policy(),
    );

    assert_eq!(
        pressure.offer(boolean_envelope, 1),
        Ok(BufferDisposition::Queued)
    );
    assert_eq!(
        pressure.offer(string_envelope, 1),
        Ok(BufferDisposition::Queued)
    );
    assert_eq!(pressure.retained_events(), 2);
}

#[tokio::test]
async fn telemetry_is_closed_low_cardinality_and_distinguishes_pressure_outcomes() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let request = fixture.request(
        subscription(0x50),
        VerifiedOrigin::parse("https://example.test").expect("origin"),
    );
    let mut pressure = buffer(
        ResourceBounds::new(1, 256 * KIB).expect("telemetry bounds"),
        1,
        policy(),
    );
    pressure
        .offer(refresh(request.context(), 1), 1)
        .expect("queued");
    pressure
        .offer(refresh(request.context(), 2), 1)
        .expect("coalesced");
    pressure
        .offer(refresh(request.context(), 4), 1)
        .expect("degraded");
    pressure
        .offer(browser_event(request.context(), 5), 101)
        .expect("closed");
    pressure.retire();

    let snapshot = pressure.telemetry_snapshot();
    assert_eq!(snapshot.count(AsyncTelemetryCounter::Queued), 1);
    assert_eq!(snapshot.count(AsyncTelemetryCounter::Coalesced), 1);
    assert_eq!(snapshot.count(AsyncTelemetryCounter::Degraded), 1);
    assert_eq!(snapshot.count(AsyncTelemetryCounter::Closed), 1);
    assert_eq!(snapshot.count(AsyncTelemetryCounter::Rejected), 1);
    assert_eq!(snapshot.count(AsyncTelemetryCounter::Cleanup), 1);
    assert_eq!(AsyncTelemetryCounter::ALL.len(), 6);
    assert!(
        AsyncTelemetryCounter::ALL
            .iter()
            .all(|counter| counter.as_str().len() <= 32)
    );
    let debug = format!("{snapshot:?}");
    assert!(!debug.contains("orders"));
    assert!(!debug.contains(&subscription(0x50).to_base64url()));
}

#[tokio::test]
async fn pressure_and_delivery_debug_never_retain_payload_or_routing_strings() {
    const SENTINEL: &str = "async-pressure-payload-secret-sentinel";

    let fixture = TransportFixture::new(position(7, 0)).await;
    let request = fixture.request(
        subscription(0x55),
        VerifiedOrigin::parse("https://example.test").expect("origin"),
    );
    let event = RegisteredBrowserEvent::new(
        request.context(),
        BrowserOperationName::parse("orders.updated").expect("event name"),
        1,
        EventTarget::SelfIsland,
        CanonicalValue::String(SENTINEL.to_owned()),
    )
    .expect("registered JSON browser event");
    let envelope = AsyncEnvelope::new(
        request.context(),
        position(7, 1),
        AsyncPayload::BrowserEvent(event),
    )
    .expect("browser-event envelope");
    let mut pressure = buffer(
        ResourceBounds::new(1, 256 * KIB).expect("redaction bounds"),
        1,
        policy(),
    );
    pressure.offer(envelope, 1).expect("queued event");

    let pressure_debug = format!("{pressure:?}");
    assert!(!pressure_debug.contains(SENTINEL));
    assert!(!pressure_debug.contains("orders.updated"));
    let delivery = pressure
        .try_start_delivery()
        .expect("bounded delivery starts");
    let delivery_debug = format!("{delivery:?}");
    assert!(!delivery_debug.contains(SENTINEL));
    assert!(!delivery_debug.contains("orders.updated"));
}
