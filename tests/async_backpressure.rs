//! Server-side asynchronous fanout and bounded-buffer policy tests.

use std::num::NonZeroUsize;

use suprnova_live::async_updates::EventTarget;
use suprnova_live::async_updates::{
    AsyncBackpressure, AsyncBufferEntry, AsyncCloseCode, AsyncCodecLimits, AsyncDispatchError,
    AsyncEnvelope, AsyncEnvelopeContext, AsyncEnvelopeDispatchPort, AsyncPayload, AsyncPolicy,
    AsyncTelemetryCounter, BrowserPayloadSchema, BufferDisposition, Heartbeat,
    MAX_ASYNC_BUFFER_BYTES, MAX_ASYNC_BUFFER_EVENTS, MAX_ASYNC_PAYLOAD_BYTES, MAX_EVENT_FANOUT,
    MAX_REPLAY_TRANSCRIPT_ENVELOPES, PresentationSignalContract, RegisteredBrowserEvent,
    RegisteredPresentationSignal, RegisteredRefresh, SequenceDisposition, SequenceMachine,
    VerifiedOrigin, encode_async_envelope,
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

use support::{NOW, ScriptItem, ScriptedSource};
use support::{TransportFixture, position, subscription};

const KIB: usize = 1024;

#[derive(Default)]
struct RecordingDispatcher {
    applied: usize,
}

impl AsyncEnvelopeDispatchPort for RecordingDispatcher {
    fn dispatch(&mut self, _envelope: &AsyncEnvelope) -> Result<(), AsyncDispatchError> {
        self.applied += 1;
        Ok(())
    }
}

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
    browser_event_for_target(context, sequence, EventTarget::SelfIsland)
}

fn browser_event_for_target(
    context: &AsyncEnvelopeContext,
    sequence: u64,
    target: EventTarget,
) -> AsyncEnvelope {
    let event = RegisteredBrowserEvent::new(
        context,
        BrowserOperationName::parse("orders.updated").expect("event name"),
        1,
        target,
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

struct PressureMembership {
    document: suprnova_live::async_updates::DocumentTransportSession,
    request: suprnova_live::async_updates::AuthorizedTransportSubscription,
}

impl PressureMembership {
    fn context(&self) -> &AsyncEnvelopeContext {
        self.request.context()
    }

    fn seal(
        &self,
        fixture: &TransportFixture,
        envelope: AsyncEnvelope,
    ) -> suprnova_live::async_updates::AuthorizedAsyncBufferEntry {
        self.document
            .authorize_async_delivery(&self.request, envelope, fixture.registry.as_ref(), NOW)
            .expect("fresh exact document delivery")
    }
}

async fn establish_for_pressure(
    fixture: &TransportFixture,
    subscription_id: suprnova_live::async_updates::SubscriptionId,
) -> PressureMembership {
    let origin = VerifiedOrigin::parse("https://example.test").expect("origin");
    let mut document = fixture.document(
        origin.clone(),
        suprnova_live::async_updates::DocumentTransportKind::ServerSentEvents,
        0xa1,
        4,
    );
    let committed = fixture.request(subscription_id.clone(), origin.clone());
    let admission = fixture.request(subscription_id, origin);
    let source = ScriptedSource::new(vec![vec![ScriptItem::Pending]]);
    let pending = document.prepare_add(committed).expect("prepare add");
    let authorized = pending.authorize().await.expect("authorize add");
    let establishing = document
        .prepare_establish(authorized)
        .expect("prepare establish");
    let ready = establishing.establish(&source).await.expect("establish");
    document.commit_add(ready).expect("commit add");
    PressureMembership {
        document,
        request: admission,
    }
}

#[tokio::test]
async fn pressure_accepts_only_a_fresh_document_membership_sealed_entry() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let membership = establish_for_pressure(&fixture, subscription(0x40)).await;
    let mut pressure = buffer(
        ResourceBounds::new(2, 256 * KIB).expect("sealed admission bounds"),
        2,
        policy(),
    );
    let sealed = membership.seal(&fixture, refresh(membership.context(), 1));

    assert_eq!(pressure.offer(sealed), Ok(BufferDisposition::Queued));
}

#[tokio::test]
async fn dequeued_delivery_consumes_the_same_admitted_guard_in_the_existing_sequence_machine() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let membership = establish_for_pressure(&fixture, subscription(0x3e)).await;
    let mut pressure = buffer(
        ResourceBounds::new(1, 256 * KIB).expect("dispatch bounds"),
        1,
        policy(),
    );
    pressure
        .offer(membership.seal(&fixture, refresh(membership.context(), 1)))
        .expect("sealed queue entry");
    let delivery = pressure.try_start_delivery().expect("bounded delivery");
    let mut sequence = SequenceMachine::new(membership.context());
    let mut dispatcher = RecordingDispatcher::default();

    assert_eq!(
        delivery.dispatch(&mut sequence, NOW, &mut dispatcher),
        Ok(SequenceDisposition::Apply)
    );
    assert_eq!(dispatcher.applied, 1);
    assert_eq!(sequence.current(), position(7, 1));
    assert_eq!(pressure.active_permits(), 0);
}

#[tokio::test]
async fn trusted_current_fanout_and_document_scope_are_checked_before_queue_mutation() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let membership = establish_for_pressure(&fixture, subscription(0x3f)).await;
    let mut pressure = buffer(
        ResourceBounds::new(2, 256 * KIB).expect("authority bounds"),
        2,
        policy(),
    );

    fixture.registry.set_resolved_event_fanout(1);
    let admitted = membership
        .document
        .authorize_async_delivery(
            &membership.request,
            browser_event(membership.context(), 1),
            fixture.registry.as_ref(),
            NOW,
        )
        .expect("registered current fanout");
    assert_eq!(pressure.offer(admitted), Ok(BufferDisposition::Queued));

    fixture.registry.set_resolved_event_fanout(2);
    assert!(
        membership
            .document
            .authorize_async_delivery(
                &membership.request,
                browser_event(membership.context(), 2),
                fixture.registry.as_ref(),
                NOW,
            )
            .is_err()
    );
    assert_eq!(pressure.retained_events(), 1);

    fixture.registry.set_resolved_event_fanout(1);
    fixture.registry.change_document_scope();
    assert!(
        membership
            .document
            .authorize_async_delivery(
                &membership.request,
                refresh(membership.context(), 2),
                fixture.registry.as_ref(),
                NOW,
            )
            .is_err()
    );
    assert_eq!(pressure.retained_events(), 1);
}

#[tokio::test]
async fn same_subscription_under_different_descriptor_bindings_never_coalesces() {
    let first_fixture = TransportFixture::new(position(7, 0)).await;
    let second_fixture =
        TransportFixture::new_with_signing_key(position(7, 0), "async-overlap-k2", 0x92).await;
    let logical_id = subscription(0x30);
    let first = establish_for_pressure(&first_fixture, logical_id.clone()).await;
    let second = establish_for_pressure(&second_fixture, logical_id).await;
    let mut pressure = buffer(
        ResourceBounds::new(4, 256 * KIB).expect("binding isolation bounds"),
        4,
        policy(),
    );

    assert_eq!(
        pressure.offer(first.seal(&first_fixture, refresh(first.context(), 1))),
        Ok(BufferDisposition::Queued)
    );
    assert_eq!(
        pressure.offer(second.seal(&second_fixture, refresh(second.context(), 2))),
        Ok(BufferDisposition::Queued)
    );
    assert_eq!(pressure.retained_events(), 2);
}

#[tokio::test]
async fn revoked_and_expired_memberships_cannot_mint_queue_entries() {
    let revoked_fixture = TransportFixture::new(position(7, 0)).await;
    let revoked = establish_for_pressure(&revoked_fixture, subscription(0x2e)).await;
    let revoked_envelope = refresh(revoked.context(), 1);
    revoked_fixture.registry.revoke();
    assert!(
        revoked
            .document
            .authorize_async_delivery(
                &revoked.request,
                revoked_envelope,
                revoked_fixture.registry.as_ref(),
                NOW,
            )
            .is_err()
    );

    let expired_fixture = TransportFixture::new(position(7, 0)).await;
    let expired = establish_for_pressure(&expired_fixture, subscription(0x2f)).await;
    assert!(
        expired
            .document
            .authorize_async_delivery(
                &expired.request,
                refresh(expired.context(), 1),
                expired_fixture.registry.as_ref(),
                suprnova_live::identity::UnixMillis::new(5_000),
            )
            .is_err()
    );
}

#[tokio::test]
async fn one_document_transport_fans_in_fairly_to_one_aggregate_bounded_queue() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let origin = VerifiedOrigin::parse("https://example.test").expect("origin");
    let mut document = fixture.document(
        origin.clone(),
        suprnova_live::async_updates::DocumentTransportKind::ServerSentEvents,
        0xa2,
        4,
    );
    let chatty = fixture.request(subscription(0x31), origin.clone());
    let healthy = fixture.request(subscription(0x32), origin);
    let source = ScriptedSource::new(vec![
        vec![
            ScriptItem::Envelope(position(7, 1), AsyncPayload::Refresh(RegisteredRefresh)),
            ScriptItem::Envelope(position(7, 2), AsyncPayload::Refresh(RegisteredRefresh)),
        ],
        vec![ScriptItem::Envelope(
            position(7, 1),
            AsyncPayload::Heartbeat(Heartbeat),
        )],
    ]);
    for authorization in [chatty, healthy] {
        let pending = document.prepare_add(authorization).expect("prepare add");
        let authorized = pending.authorize().await.expect("authorize add");
        let establishing = document
            .prepare_establish(authorized)
            .expect("prepare establish");
        let ready = establishing.establish(&source).await.expect("establish");
        document.commit_add(ready).expect("commit add");
    }

    let mut bounded = suprnova_live::async_updates::BoundedDocumentTransportSession::new(
        document,
        ResourceOwner::new(ResourceBounds::new(2, 256 * KIB).expect("aggregate bounds")),
        PermitPool::new(1).expect("shared permit"),
        policy(),
    )
    .expect("bounded document");

    assert_eq!(
        bounded
            .pump_next(fixture.registry.as_ref(), NOW)
            .await
            .expect("chatty ingress"),
        Some(BufferDisposition::Queued)
    );
    assert_eq!(
        bounded
            .pump_next(fixture.registry.as_ref(), NOW)
            .await
            .expect("healthy ingress"),
        Some(BufferDisposition::Queued)
    );
    assert_eq!(bounded.retained_events(), 2);
    let first = bounded.try_start_delivery().expect("first delivery");
    assert_eq!(first.envelope().subscription(), &subscription(0x31));
    assert!(bounded.try_start_delivery().is_none());
    drop(first);
    let healthy = bounded.try_start_delivery().expect("healthy delivery");
    assert_eq!(healthy.envelope().subscription(), &subscription(0x32));
    drop(healthy);
}

#[tokio::test]
async fn removing_one_logical_membership_purges_only_its_queued_delivery() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let origin = VerifiedOrigin::parse("https://example.test").expect("origin");
    let mut document = fixture.document(
        origin.clone(),
        suprnova_live::async_updates::DocumentTransportKind::ServerSentEvents,
        0xa3,
        4,
    );
    let removed_id = subscription(0x33);
    let healthy_id = subscription(0x34);
    let removal = fixture.request(removed_id.clone(), origin.clone());
    let source = ScriptedSource::new(vec![
        vec![ScriptItem::Envelope(
            position(7, 1),
            AsyncPayload::Heartbeat(Heartbeat),
        )],
        vec![ScriptItem::Envelope(
            position(7, 1),
            AsyncPayload::Heartbeat(Heartbeat),
        )],
    ]);
    for authorization in [
        fixture.request(removed_id, origin.clone()),
        fixture.request(healthy_id.clone(), origin),
    ] {
        let pending = document.prepare_add(authorization).expect("prepare add");
        let authorized = pending.authorize().await.expect("authorize add");
        let establishing = document
            .prepare_establish(authorized)
            .expect("prepare establish");
        let ready = establishing.establish(&source).await.expect("establish");
        document.commit_add(ready).expect("commit add");
    }
    let mut bounded = suprnova_live::async_updates::BoundedDocumentTransportSession::new(
        document,
        ResourceOwner::new(ResourceBounds::new(4, 256 * KIB).expect("aggregate bounds")),
        PermitPool::new(2).expect("shared permits"),
        policy(),
    )
    .expect("bounded document");
    bounded
        .pump_next(fixture.registry.as_ref(), NOW)
        .await
        .expect("removed ingress");
    bounded
        .pump_next(fixture.registry.as_ref(), NOW)
        .await
        .expect("healthy ingress");
    assert_eq!(bounded.retained_events(), 2);

    let pending = bounded.prepare_remove(&removal).expect("prepare remove");
    let ready = pending.authorize().await.expect("authorize remove");
    assert_eq!(
        bounded.commit_remove(ready),
        Ok(suprnova_live::async_updates::CloseDisposition::Closed)
    );
    assert_eq!(bounded.retained_events(), 1);
    let delivery = bounded.try_start_delivery().expect("healthy remains");
    assert_eq!(delivery.envelope().subscription(), &healthy_id);
}

#[tokio::test]
async fn provider_failure_purges_earlier_queued_work_for_the_detached_membership() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let origin = VerifiedOrigin::parse("https://example.test").expect("origin");
    let mut document = fixture.document(
        origin.clone(),
        suprnova_live::async_updates::DocumentTransportKind::ServerSentEvents,
        0xa4,
        2,
    );
    let source = ScriptedSource::new(vec![vec![
        ScriptItem::Envelope(position(7, 1), AsyncPayload::Heartbeat(Heartbeat)),
        ScriptItem::Error(suprnova_live::async_updates::AsyncTransportErrorKind::SourceFailed),
    ]]);
    let pending = document
        .prepare_add(fixture.request(subscription(0x35), origin))
        .expect("prepare add");
    let authorized = pending.authorize().await.expect("authorize add");
    let establishing = document
        .prepare_establish(authorized)
        .expect("prepare establish");
    let ready = establishing.establish(&source).await.expect("establish");
    document.commit_add(ready).expect("commit add");
    let mut bounded = suprnova_live::async_updates::BoundedDocumentTransportSession::new(
        document,
        ResourceOwner::new(ResourceBounds::new(4, 256 * KIB).expect("aggregate bounds")),
        PermitPool::new(1).expect("shared permit"),
        policy(),
    )
    .expect("bounded document");

    bounded
        .pump_next(fixture.registry.as_ref(), NOW)
        .await
        .expect("first ingress");
    assert_eq!(bounded.retained_events(), 1);
    assert_eq!(
        bounded
            .pump_next(fixture.registry.as_ref(), NOW)
            .await
            .expect_err("provider failure")
            .kind(),
        suprnova_live::async_updates::AsyncTransportErrorKind::SourceFailed
    );
    assert_eq!(bounded.retained_events(), 0);
    assert!(bounded.try_start_delivery().is_none());
}

#[tokio::test]
async fn complete_is_delivered_once_after_task_four_detaches_but_error_is_nonterminal() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let origin = VerifiedOrigin::parse("https://example.test").expect("origin");
    let completed_id = subscription(0x36);
    let error_id = subscription(0x37);
    let mut document = fixture.document(
        origin.clone(),
        suprnova_live::async_updates::DocumentTransportKind::ServerSentEvents,
        0xa5,
        3,
    );
    let source = ScriptedSource::new(vec![
        vec![ScriptItem::Envelope(
            position(7, 1),
            AsyncPayload::Complete(suprnova_live::async_updates::CompletionReason::StreamCompleted),
        )],
        vec![
            ScriptItem::Envelope(
                position(7, 1),
                AsyncPayload::Error(suprnova_live::async_updates::StreamErrorCode::Backpressure),
            ),
            ScriptItem::Envelope(position(7, 2), AsyncPayload::Heartbeat(Heartbeat)),
        ],
    ]);
    for authorization in [
        fixture.request(completed_id.clone(), origin.clone()),
        fixture.request(error_id.clone(), origin),
    ] {
        let pending = document.prepare_add(authorization).expect("prepare add");
        let authorized = pending.authorize().await.expect("authorize add");
        let establishing = document
            .prepare_establish(authorized)
            .expect("prepare establish");
        let ready = establishing.establish(&source).await.expect("establish");
        document.commit_add(ready).expect("commit add");
    }
    let mut bounded = suprnova_live::async_updates::BoundedDocumentTransportSession::new(
        document,
        ResourceOwner::new(ResourceBounds::new(4, 256 * KIB).expect("aggregate bounds")),
        PermitPool::new(2).expect("shared permits"),
        policy(),
    )
    .expect("bounded document");

    bounded
        .pump_next(fixture.registry.as_ref(), NOW)
        .await
        .expect("complete ingress");
    assert_eq!(bounded.transport().membership_count(), 1);
    bounded
        .pump_next(fixture.registry.as_ref(), NOW)
        .await
        .expect("error ingress");
    assert_eq!(bounded.transport().membership_count(), 1);
    let complete = bounded.try_start_delivery().expect("terminal delivery");
    assert_eq!(complete.envelope().subscription(), &completed_id);
    drop(complete);
    let error = bounded.try_start_delivery().expect("error delivery");
    assert_eq!(error.envelope().subscription(), &error_id);
    assert!(matches!(error.envelope().payload(), AsyncPayload::Error(_)));
    drop(error);
    bounded
        .pump_next(fixture.registry.as_ref(), NOW)
        .await
        .expect("post-error heartbeat");
    assert!(bounded.try_start_delivery().is_some());
}

#[tokio::test]
async fn one_document_global_outage_stays_aggregate_bounded_and_preserves_healthy_fairness() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let origin = VerifiedOrigin::parse("https://example.test").expect("origin");
    let chatty_id = subscription(0x38);
    let healthy_id = subscription(0x39);
    let mut document = fixture.document(
        origin.clone(),
        suprnova_live::async_updates::DocumentTransportKind::ServerSentEvents,
        0xa6,
        3,
    );
    let chatty_script = (1..=100)
        .map(|sequence| {
            ScriptItem::Envelope(position(7, sequence), AsyncPayload::Heartbeat(Heartbeat))
        })
        .collect::<Vec<_>>();
    let source = ScriptedSource::new(vec![
        chatty_script,
        vec![ScriptItem::Envelope(
            position(7, 1),
            AsyncPayload::Heartbeat(Heartbeat),
        )],
    ]);
    for authorization in [
        fixture.request(chatty_id.clone(), origin.clone()),
        fixture.request(healthy_id.clone(), origin),
    ] {
        let pending = document.prepare_add(authorization).expect("prepare add");
        let authorized = pending.authorize().await.expect("authorize add");
        let establishing = document
            .prepare_establish(authorized)
            .expect("prepare establish");
        let ready = establishing.establish(&source).await.expect("establish");
        document.commit_add(ready).expect("commit add");
    }
    let mut bounded = suprnova_live::async_updates::BoundedDocumentTransportSession::new(
        document,
        ResourceOwner::new(
            ResourceBounds::new(MAX_ASYNC_BUFFER_EVENTS, MAX_ASYNC_BUFFER_BYTES)
                .expect("document hard bounds"),
        ),
        PermitPool::new(1).expect("slow wire permit"),
        policy(),
    )
    .expect("bounded document");

    for _ in 0..101 {
        let _ = bounded
            .pump_next(fixture.registry.as_ref(), NOW)
            .await
            .expect("bounded outage ingress");
    }
    assert_eq!(bounded.retained_events(), MAX_ASYNC_BUFFER_EVENTS);
    assert!(bounded.retained_bytes() <= MAX_ASYNC_BUFFER_BYTES);
    let first = bounded.try_start_delivery().expect("chatty first");
    assert_eq!(first.envelope().subscription(), &chatty_id);
    assert!(bounded.try_start_delivery().is_none());
    drop(first);
    let second = bounded.try_start_delivery().expect("healthy second");
    assert_eq!(second.envelope().subscription(), &healthy_id);
}

#[tokio::test]
async fn repeated_refresh_pressure_coalesces_inside_shared_resource_bounds() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let request = establish_for_pressure(&fixture, subscription(0x41)).await;
    let mut pressure = buffer(
        ResourceBounds::new(64, 256 * KIB).expect("document async bounds"),
        8,
        policy(),
    );

    for sequence in 1..=1_000 {
        let envelope = refresh(request.context(), sequence);
        let disposition = pressure.offer(request.seal(&fixture, envelope));
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
    let request = establish_for_pressure(&fixture, subscription(0x42)).await;
    let mut pressure = buffer(
        ResourceBounds::new(2, 256 * KIB).expect("event bounds"),
        2,
        policy(),
    );

    assert_eq!(
        pressure.offer(request.seal(&fixture, browser_event(request.context(), 1))),
        Ok(BufferDisposition::Queued)
    );
    assert_eq!(
        pressure.offer(request.seal(&fixture, browser_event(request.context(), 2))),
        Ok(BufferDisposition::Queued)
    );
    assert_eq!(
        pressure.offer(request.seal(&fixture, browser_event(request.context(), 3))),
        Ok(BufferDisposition::Degraded)
    );
    assert_eq!(pressure.retained_events(), 2);
    assert!(pressure.is_degraded());
}

#[tokio::test]
async fn fanout_is_rejected_against_registered_and_policy_authority_before_allocation() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let request = establish_for_pressure(&fixture, subscription(0x43)).await;
    let mut pressure = buffer(
        ResourceBounds::new(4, 256 * KIB).expect("fanout bounds"),
        4,
        policy(),
    );

    assert_eq!(
        pressure.offer(request.seal(&fixture, browser_event(request.context(), 1))),
        Ok(BufferDisposition::Queued)
    );
    fixture.registry.set_resolved_event_fanout(2);
    assert!(
        request
            .document
            .authorize_async_delivery(
                &request.request,
                browser_event(request.context(), 2),
                fixture.registry.as_ref(),
                NOW,
            )
            .is_err()
    );
    assert_eq!(pressure.retained_events(), 1);
    assert_eq!(pressure.active_permits(), 0);

    let mut policy_limited = buffer(
        ResourceBounds::new(1, 256 * KIB).expect("policy fanout bounds"),
        1,
        AsyncPolicy {
            max_fanout: NonZeroUsize::new(3).expect("policy fanout"),
            ..policy()
        },
    );
    fixture.registry.set_resolved_event_fanout(4);
    assert_eq!(
        policy_limited.offer(request.seal(
            &fixture,
            browser_event_for_target(request.context(), 4, EventTarget::Document),
        )),
        Ok(BufferDisposition::Closed(AsyncCloseCode::FanoutExceeded))
    );
    assert_eq!(policy_limited.retained_events(), 0);
    assert_eq!(policy_limited.active_permits(), 0);
}

#[tokio::test]
async fn slow_delivery_holds_one_shared_permit_without_popping_a_sibling() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let request = establish_for_pressure(&fixture, subscription(0x44)).await;
    let mut pressure = buffer(
        ResourceBounds::new(2, 256 * KIB).expect("slow-client bounds"),
        1,
        policy(),
    );
    pressure
        .offer(request.seal(&fixture, browser_event(request.context(), 1)))
        .expect("first offer");
    pressure
        .offer(request.seal(&fixture, heartbeat(request.context(), 2)))
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
    let request = establish_for_pressure(&fixture, subscription(0x45)).await;
    let mut pressure = buffer(
        ResourceBounds::new(4, 256 * KIB).expect("retirement bounds"),
        2,
        policy(),
    );
    pressure
        .offer(request.seal(&fixture, browser_event(request.context(), 1)))
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
        pressure.offer(request.seal(&fixture, browser_event(request.context(), 2))),
        Ok(BufferDisposition::Closed(AsyncCloseCode::Retired))
    );
}

#[tokio::test]
async fn external_owner_cancellation_stops_delivery_and_cleans_up_once() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let request = establish_for_pressure(&fixture, subscription(0x56)).await;
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
        .offer(request.seal(&fixture, browser_event(request.context(), 1)))
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
    let request = establish_for_pressure(&fixture, subscription(0x46)).await;
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
        exact.offer(request.seal(&fixture, envelope.clone())),
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
        first_rejected.offer(request.seal(&fixture, envelope)),
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
    let request = establish_for_pressure(&fixture, subscription(0x47)).await;
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
            request.seal(&fixture, heartbeat(request.context(), 1)),
            request.seal(&fixture, heartbeat(request.context(), 2)),
            request.seal(&fixture, heartbeat(request.context(), 3)),
        ]),
        Ok(BufferDisposition::Degraded)
    );
    assert_eq!(pressure.retained_events(), 0);

    assert_eq!(
        pressure.offer_replay(vec![
            request.seal(&fixture, heartbeat(request.context(), 1)),
            request.seal(&fixture, heartbeat(request.context(), 2)),
        ]),
        Ok(BufferDisposition::Queued)
    );
    assert_eq!(pressure.retained_events(), 2);
}

#[tokio::test]
async fn replay_batch_never_crosses_exact_membership_binding_or_stream_scope() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let first = establish_for_pressure(&fixture, subscription(0x27)).await;
    let second = establish_for_pressure(&fixture, subscription(0x28)).await;
    let mut pressure = buffer(
        ResourceBounds::new(4, 256 * KIB).expect("replay isolation bounds"),
        4,
        policy(),
    );

    assert_eq!(
        pressure.offer_replay(vec![
            first.seal(&fixture, heartbeat(first.context(), 1)),
            second.seal(&fixture, heartbeat(second.context(), 2)),
        ]),
        Ok(BufferDisposition::Degraded)
    );
    assert_eq!(pressure.retained_events(), 0);
    assert_eq!(pressure.retained_bytes(), 0);
}

#[tokio::test]
async fn replay_aggregate_byte_boundary_is_exact_and_first_over_is_atomic() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let request = establish_for_pressure(&fixture, subscription(0x51)).await;
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
        exact.offer_replay(vec![
            request.seal(&fixture, first.clone()),
            request.seal(&fixture, second.clone()),
        ]),
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
        first_over.offer_replay(vec![
            request.seal(&fixture, first),
            request.seal(&fixture, second),
        ]),
        Ok(BufferDisposition::Degraded)
    );
    assert_eq!(first_over.retained_events(), 0);
    assert_eq!(first_over.retained_bytes(), 0);
}

#[tokio::test]
async fn heartbeat_pressure_never_evicts_a_required_browser_event() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let request = establish_for_pressure(&fixture, subscription(0x52)).await;
    let mut pressure = buffer(
        ResourceBounds::new(1, 256 * KIB).expect("heartbeat pressure bounds"),
        1,
        policy(),
    );
    assert_eq!(
        pressure.offer(request.seal(&fixture, browser_event(request.context(), 1))),
        Ok(BufferDisposition::Queued)
    );
    assert_eq!(
        pressure.offer(request.seal(&fixture, heartbeat(request.context(), 2))),
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
    let first = establish_for_pressure(&fixture, subscription(0x48)).await;
    let second = establish_for_pressure(&fixture, subscription(0x49)).await;
    let mut pressure = buffer(
        ResourceBounds::new(8, 256 * KIB).expect("coalescing bounds"),
        8,
        policy(),
    );

    assert_eq!(
        pressure.offer(first.seal(
            &fixture,
            presentation_signal(first.context(), 1, "open", false),
        )),
        Ok(BufferDisposition::Queued)
    );
    assert_eq!(
        pressure.offer(first.seal(
            &fixture,
            presentation_signal(first.context(), 2, "open", true),
        )),
        Ok(BufferDisposition::Coalesced)
    );
    assert_eq!(pressure.retained_events(), 1);
    assert_eq!(
        pressure.offer(first.seal(
            &fixture,
            presentation_signal(first.context(), 4, "open", false),
        )),
        Ok(BufferDisposition::Degraded)
    );
    assert_eq!(pressure.retained_events(), 1);

    assert_eq!(
        pressure.offer(first.seal(
            &fixture,
            presentation_signal(first.context(), 3, "busy", true),
        )),
        Ok(BufferDisposition::Queued)
    );
    assert_eq!(
        pressure.offer(second.seal(
            &fixture,
            presentation_signal(second.context(), 1, "busy", true),
        )),
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
    let first = establish_for_pressure(&fixture, logical_subscription.clone()).await;
    let boolean_envelope = presentation_signal(first.context(), 1, "status", true);
    let boolean_entry = first.seal(&fixture, boolean_envelope);

    fixture
        .registry
        .set_presentation_signals(vec![PresentationSignalContract::new(
            signal_name.clone(),
            BrowserPayloadSchema::String,
        )]);
    let revised = establish_for_pressure(&fixture, logical_subscription).await;
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

    assert_eq!(pressure.offer(boolean_entry), Ok(BufferDisposition::Queued));
    assert_eq!(
        pressure.offer(revised.seal(&fixture, string_envelope)),
        Ok(BufferDisposition::Queued)
    );
    assert_eq!(pressure.retained_events(), 2);
}

#[tokio::test]
async fn telemetry_is_closed_low_cardinality_and_distinguishes_pressure_outcomes() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let request = establish_for_pressure(&fixture, subscription(0x50)).await;
    let mut pressure = buffer(
        ResourceBounds::new(1, 256 * KIB).expect("telemetry bounds"),
        1,
        AsyncPolicy {
            max_payload_bytes: NonZeroUsize::new(br#"{"kind":"refresh","name":"refresh"}"#.len())
                .expect("refresh payload boundary"),
            ..policy()
        },
    );
    pressure
        .offer(request.seal(&fixture, refresh(request.context(), 1)))
        .expect("queued");
    pressure
        .offer(request.seal(&fixture, refresh(request.context(), 2)))
        .expect("coalesced");
    pressure
        .offer(request.seal(&fixture, refresh(request.context(), 4)))
        .expect("degraded");
    let _ = pressure.offer(request.seal(&fixture, browser_event(request.context(), 5)));
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
    let request = establish_for_pressure(&fixture, subscription(0x55)).await;
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
    pressure
        .offer(request.seal(&fixture, envelope))
        .expect("queued event");

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
