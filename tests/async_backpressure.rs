//! Public document-owned asynchronous admission and dispatch tests.

use std::num::NonZeroUsize;
use std::panic::{AssertUnwindSafe, catch_unwind};

use suprnova_live::async_updates::{
    AsyncCloseCode, AsyncCodecLimits, AsyncDeliveryErrorKind, AsyncDispatchError, AsyncEnvelope,
    AsyncEnvelopeContext, AsyncEnvelopeDispatchPort, AsyncPayload, AsyncPolicy,
    AsyncTelemetryCounter, BoundedDocumentTransportSession, BufferDisposition, CloseDisposition,
    CompletionReason, DocumentTransportKind, EventTarget, Heartbeat, MAX_ASYNC_BUFFER_BYTES,
    MAX_ASYNC_BUFFER_EVENTS, MAX_ASYNC_PAYLOAD_BYTES, MAX_EVENT_FANOUT,
    MAX_REPLAY_TRANSCRIPT_ENVELOPES, RegisteredBrowserEvent, RegisteredRefresh,
    SequenceDegradation, SequenceDisposition, SequenceErrorKind, StreamErrorCode, SubscriptionId,
    VerifiedOrigin, encode_async_envelope,
};
use suprnova_live::canonical::CanonicalValue;
use suprnova_live::identity::{BrowserOperationName, ContentDigest};
use suprnova_live::resource::{PermitPool, ResourceBounds, Retirement};

#[allow(
    dead_code,
    reason = "the shared Task 4 fixture exposes controls not needed by every focused suite"
)]
#[path = "support/async_transport.rs"]
mod support;

use support::{
    DeliveryAuthorityDrift, ScriptItem, ScriptedSource, TransportFixture, position, subscription,
};

#[derive(Default)]
struct RecordingDispatcher {
    applied: usize,
    subscriptions: Vec<SubscriptionId>,
}

struct FailOnDispatcher {
    attempts: usize,
    fail_on: usize,
}

impl AsyncEnvelopeDispatchPort for FailOnDispatcher {
    fn dispatch(&mut self, _envelope: &AsyncEnvelope) -> Result<(), AsyncDispatchError> {
        self.attempts += 1;
        if self.attempts == self.fail_on {
            Err(AsyncDispatchError::failed())
        } else {
            Ok(())
        }
    }
}

struct PanickingDispatcher;

impl AsyncEnvelopeDispatchPort for PanickingDispatcher {
    fn dispatch(&mut self, _envelope: &AsyncEnvelope) -> Result<(), AsyncDispatchError> {
        panic!("controlled dispatcher panic")
    }
}

impl AsyncEnvelopeDispatchPort for RecordingDispatcher {
    fn dispatch(&mut self, _envelope: &AsyncEnvelope) -> Result<(), AsyncDispatchError> {
        self.applied += 1;
        self.subscriptions.push(_envelope.subscription().clone());
        Ok(())
    }
}

fn policy() -> AsyncPolicy {
    AsyncPolicy {
        max_payload_bytes: NonZeroUsize::new(32 * 1024).expect("payload bound"),
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

fn heartbeat_at(context: &AsyncEnvelopeContext, epoch: u64, sequence: u64) -> AsyncEnvelope {
    AsyncEnvelope::new(
        context,
        position(epoch, sequence),
        AsyncPayload::Heartbeat(Heartbeat),
    )
    .expect("heartbeat envelope")
}

async fn bounded_document(
    fixture: &TransportFixture,
    subscription_marker: u8,
    payloads: Vec<AsyncPayload>,
) -> (
    BoundedDocumentTransportSession,
    suprnova_live::async_updates::AuthorizedTransportSubscription,
) {
    bounded_document_with_bounds(
        fixture,
        subscription_marker,
        payloads,
        ResourceBounds::new(64, 256 * 1024).expect("document bounds"),
    )
    .await
}

async fn bounded_document_with_bounds(
    fixture: &TransportFixture,
    subscription_marker: u8,
    payloads: Vec<AsyncPayload>,
    bounds: ResourceBounds,
) -> (
    BoundedDocumentTransportSession,
    suprnova_live::async_updates::AuthorizedTransportSubscription,
) {
    bounded_document_with_config(fixture, subscription_marker, payloads, bounds, 4, policy()).await
}

async fn bounded_document_with_config(
    fixture: &TransportFixture,
    subscription_marker: u8,
    payloads: Vec<AsyncPayload>,
    bounds: ResourceBounds,
    permits: usize,
    async_policy: AsyncPolicy,
) -> (
    BoundedDocumentTransportSession,
    suprnova_live::async_updates::AuthorizedTransportSubscription,
) {
    let origin = VerifiedOrigin::parse("https://example.test").expect("origin");
    let mut document = fixture.document(
        origin.clone(),
        DocumentTransportKind::ServerSentEvents,
        subscription_marker,
        4,
    );
    let request = fixture.request(subscription(subscription_marker), origin);
    let source = ScriptedSource::new(vec![
        payloads
            .into_iter()
            .enumerate()
            .map(|(index, payload)| ScriptItem::Envelope(position(7, index as u64 + 1), payload))
            .collect(),
    ]);
    let pending = document.prepare_add(request.clone()).expect("prepare add");
    let authorized = pending.authorize().await.expect("authorize add");
    let establishing = document
        .prepare_establish(authorized)
        .expect("prepare establish");
    let ready = establishing.establish(&source).await.expect("establish");
    document.commit_add(ready).expect("commit add");
    let bounded = BoundedDocumentTransportSession::new(
        document,
        bounds,
        PermitPool::new(permits).expect("shared permits"),
        async_policy,
    )
    .expect("bounded document");
    (bounded, request)
}

async fn bounded_two_memberships(
    fixture: &TransportFixture,
    document_marker: u8,
    first_marker: u8,
    second_marker: u8,
    scripts: Vec<Vec<ScriptItem>>,
    bounds: ResourceBounds,
) -> (
    BoundedDocumentTransportSession,
    suprnova_live::async_updates::AuthorizedTransportSubscription,
    suprnova_live::async_updates::AuthorizedTransportSubscription,
) {
    let origin = VerifiedOrigin::parse("https://example.test").expect("origin");
    let mut document = fixture.document(
        origin.clone(),
        DocumentTransportKind::ServerSentEvents,
        document_marker,
        4,
    );
    let first = fixture.request(subscription(first_marker), origin.clone());
    let second = fixture.request(subscription(second_marker), origin);
    let source = ScriptedSource::new(scripts);
    for authorization in [first.clone(), second.clone()] {
        let pending = document.prepare_add(authorization).expect("prepare add");
        let authorized = pending.authorize().await.expect("authorize add");
        let establishing = document
            .prepare_establish(authorized)
            .expect("prepare establish");
        let ready = establishing.establish(&source).await.expect("establish");
        document.commit_add(ready).expect("commit add");
    }
    let bounded = BoundedDocumentTransportSession::new(
        document,
        bounds,
        PermitPool::new(1).expect("shared delivery permit"),
        policy(),
    )
    .expect("bounded document");
    (bounded, first, second)
}

#[tokio::test]
async fn document_owned_admission_dispatches_without_exposing_a_raw_lease() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let origin = VerifiedOrigin::parse("https://example.test").expect("origin");
    let mut document = fixture.document(
        origin.clone(),
        DocumentTransportKind::ServerSentEvents,
        0xb1,
        2,
    );
    let request = fixture.request(subscription(0x60), origin.clone());
    let _same_membership = fixture.request(subscription(0x60), origin);
    let source = ScriptedSource::new(vec![vec![ScriptItem::Envelope(
        position(7, 1),
        AsyncPayload::Refresh(RegisteredRefresh),
    )]]);
    let pending = document.prepare_add(request.clone()).expect("prepare add");
    let authorized = pending.authorize().await.expect("authorize add");
    let establishing = document
        .prepare_establish(authorized)
        .expect("prepare establish");
    let ready = establishing.establish(&source).await.expect("establish");
    document.commit_add(ready).expect("commit add");

    let mut bounded = BoundedDocumentTransportSession::new(
        document,
        ResourceBounds::new(4, 256 * 1024).expect("document bounds"),
        PermitPool::new(1).expect("shared permit"),
        policy(),
    )
    .expect("bounded document");
    assert_eq!(
        bounded
            .pump_next(fixture.registry.as_ref())
            .await
            .expect("document-owned admission"),
        Some(BufferDisposition::Queued)
    );

    let mut dispatcher = RecordingDispatcher::default();
    assert_eq!(
        bounded.dispatch_next(fixture.registry.as_ref(), &mut dispatcher),
        Ok(Some(SequenceDisposition::Apply))
    );
    assert_eq!(dispatcher.applied, 1);
    assert_eq!(bounded.sequence_position(&request), Some(position(7, 1)));
}

async fn assert_post_seal_drift_rejects(
    fixture: &TransportFixture,
    drift: DeliveryAuthorityDrift,
    payload: AsyncPayload,
) {
    let (mut bounded, _) = bounded_document(fixture, 0x61, vec![payload]).await;
    fixture.registry.drift_after_delivery_validation(1, drift);

    assert!(bounded.pump_next(fixture.registry.as_ref()).await.is_err());
    assert_eq!(bounded.retained_events(), 0);
    assert_eq!(bounded.retained_bytes(), 0);
    assert_eq!(bounded.active_permits(), 0);
    assert!(bounded.is_degraded());
}

#[tokio::test]
async fn final_admission_rejects_expiry_revocation_and_membership_removal_without_mutation() {
    let expiry = TransportFixture::new(position(7, 0)).await;
    assert_post_seal_drift_rejects(
        &expiry,
        DeliveryAuthorityDrift::Expire(suprnova_live::identity::UnixMillis::new(5_000)),
        AsyncPayload::Refresh(RegisteredRefresh),
    )
    .await;

    let revoked = TransportFixture::new(position(7, 0)).await;
    assert_post_seal_drift_rejects(
        &revoked,
        DeliveryAuthorityDrift::Revoke,
        AsyncPayload::Refresh(RegisteredRefresh),
    )
    .await;

    let removed = TransportFixture::new(position(7, 0)).await;
    assert_post_seal_drift_rejects(
        &removed,
        DeliveryAuthorityDrift::RemoveSubscription(subscription(0x61)),
        AsyncPayload::Refresh(RegisteredRefresh),
    )
    .await;
}

#[tokio::test]
async fn final_admission_rejects_contract_binding_and_scope_drift_without_mutation() {
    let contract = TransportFixture::new(position(7, 0)).await;
    let revised = TransportFixture::new_with_contract_revision(position(7, 0)).await;
    assert_post_seal_drift_rejects(
        &contract,
        DeliveryAuthorityDrift::EventContracts(revised.registry.event_contracts()),
        AsyncPayload::Refresh(RegisteredRefresh),
    )
    .await;

    let binding = TransportFixture::new(position(7, 0)).await;
    let other_key = TransportFixture::new_with_signing_key(position(7, 0), "alternate", 0x9a).await;
    let other_binding = other_key
        .request(
            subscription(0x61),
            VerifiedOrigin::parse("https://example.test").expect("origin"),
        )
        .binding()
        .clone();
    assert_post_seal_drift_rejects(
        &binding,
        DeliveryAuthorityDrift::Binding(other_binding),
        AsyncPayload::Refresh(RegisteredRefresh),
    )
    .await;

    let document_scope = TransportFixture::new(position(7, 0)).await;
    assert_post_seal_drift_rejects(
        &document_scope,
        DeliveryAuthorityDrift::DocumentScope,
        AsyncPayload::Refresh(RegisteredRefresh),
    )
    .await;

    let authorization_scope = TransportFixture::new(position(7, 0)).await;
    assert_post_seal_drift_rejects(
        &authorization_scope,
        DeliveryAuthorityDrift::AuthorizationScope,
        AsyncPayload::Refresh(RegisteredRefresh),
    )
    .await;
}

#[tokio::test]
async fn final_admission_rejects_target_count_and_target_set_drift_without_mutation() {
    let count = TransportFixture::new(position(7, 0)).await;
    let count_context = count
        .request(
            subscription(0x61),
            VerifiedOrigin::parse("https://example.test").expect("origin"),
        )
        .context()
        .clone();
    assert_post_seal_drift_rejects(
        &count,
        DeliveryAuthorityDrift::TargetCount(2),
        browser_event(&count_context, 1).payload().clone(),
    )
    .await;

    let target_set = TransportFixture::new(position(7, 0)).await;
    let target_context = target_set
        .request(
            subscription(0x61),
            VerifiedOrigin::parse("https://example.test").expect("origin"),
        )
        .context()
        .clone();
    assert_post_seal_drift_rejects(
        &target_set,
        DeliveryAuthorityDrift::TargetScope(
            ContentDigest::from_bytes(&[0xc7; 32]).expect("target scope"),
        ),
        browser_event(&target_context, 1).payload().clone(),
    )
    .await;
}

#[tokio::test]
async fn replay_final_validation_is_all_or_nothing_when_authority_drifts_mid_batch() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let (mut bounded, authorization) = bounded_document(&fixture, 0x62, vec![]).await;
    let context = authorization.context();
    let transcript = vec![refresh(context, 1), refresh(context, 2)];
    fixture
        .registry
        .drift_after_delivery_validation(3, DeliveryAuthorityDrift::DocumentScope);

    assert!(
        bounded
            .admit_replay(&authorization, transcript, fixture.registry.as_ref())
            .is_err()
    );
    assert_eq!(bounded.retained_events(), 0);
    assert_eq!(bounded.retained_bytes(), 0);
    assert_eq!(bounded.active_permits(), 0);
}

#[tokio::test]
async fn terminal_final_admission_failure_releases_its_detached_drain_and_sequence_lane() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let (mut bounded, authorization) = bounded_document(
        &fixture,
        0x6c,
        vec![AsyncPayload::Complete(CompletionReason::StreamCompleted)],
    )
    .await;
    fixture
        .registry
        .drift_after_delivery_validation(1, DeliveryAuthorityDrift::Revoke);

    assert!(bounded.pump_next(fixture.registry.as_ref()).await.is_err());
    assert_eq!(bounded.transport().membership_count(), 0);
    assert_eq!(bounded.retained_events(), 0);
    assert_eq!(bounded.retained_bytes(), 0);
    assert_eq!(bounded.sequence_position(&authorization), None);
    assert!(bounded.is_degraded());
}

#[tokio::test]
async fn post_pop_revocation_is_rechecked_before_registered_dispatch() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let (mut bounded, authorization) = bounded_document(
        &fixture,
        0x63,
        vec![AsyncPayload::Refresh(RegisteredRefresh)],
    )
    .await;
    assert_eq!(
        bounded
            .pump_next(fixture.registry.as_ref())
            .await
            .expect("initial admission"),
        Some(BufferDisposition::Queued)
    );
    fixture
        .registry
        .drift_after_delivery_validation(1, DeliveryAuthorityDrift::Revoke);
    let mut dispatcher = RecordingDispatcher::default();

    let error = bounded
        .dispatch_next(fixture.registry.as_ref(), &mut dispatcher)
        .expect_err("revocation after dequeue must deny dispatch");
    assert_eq!(error.kind(), AsyncDeliveryErrorKind::AuthorizationLost);
    assert_eq!(dispatcher.applied, 0);
    assert_eq!(
        bounded.sequence_position(&authorization),
        Some(position(7, 0))
    );
    assert_eq!(bounded.active_permits(), 0);
    assert!(bounded.is_degraded());
}

async fn assert_post_pop_drift_denies(
    fixture: &TransportFixture,
    drift: DeliveryAuthorityDrift,
    payload: AsyncPayload,
) {
    let (mut bounded, authorization) = bounded_document(fixture, 0x69, vec![payload]).await;
    bounded
        .pump_next(fixture.registry.as_ref())
        .await
        .expect("initial admission");
    fixture.registry.drift_after_delivery_validation(1, drift);
    let mut dispatcher = RecordingDispatcher::default();

    let error = bounded
        .dispatch_next(fixture.registry.as_ref(), &mut dispatcher)
        .expect_err("post-pop authority drift must deny dispatch");
    assert_eq!(error.kind(), AsyncDeliveryErrorKind::AuthorizationLost);
    assert_eq!(dispatcher.applied, 0);
    assert_eq!(
        bounded.sequence_position(&authorization),
        Some(position(7, 0))
    );
    assert_eq!(bounded.active_permits(), 0);
    assert!(bounded.is_degraded());
}

#[tokio::test]
async fn post_pop_expiry_removal_and_contract_drift_prevent_dispatch() {
    let expiry = TransportFixture::new(position(7, 0)).await;
    assert_post_pop_drift_denies(
        &expiry,
        DeliveryAuthorityDrift::Expire(suprnova_live::identity::UnixMillis::new(5_000)),
        AsyncPayload::Refresh(RegisteredRefresh),
    )
    .await;

    let removed = TransportFixture::new(position(7, 0)).await;
    assert_post_pop_drift_denies(
        &removed,
        DeliveryAuthorityDrift::RemoveSubscription(subscription(0x69)),
        AsyncPayload::Refresh(RegisteredRefresh),
    )
    .await;

    let contract = TransportFixture::new(position(7, 0)).await;
    let revised = TransportFixture::new_with_contract_revision(position(7, 0)).await;
    assert_post_pop_drift_denies(
        &contract,
        DeliveryAuthorityDrift::EventContracts(revised.registry.event_contracts()),
        AsyncPayload::Refresh(RegisteredRefresh),
    )
    .await;
}

#[tokio::test]
async fn post_pop_target_binding_and_document_scope_drift_prevent_dispatch() {
    let target = TransportFixture::new(position(7, 0)).await;
    let target_context = target
        .request(
            subscription(0x69),
            VerifiedOrigin::parse("https://example.test").expect("origin"),
        )
        .context()
        .clone();
    assert_post_pop_drift_denies(
        &target,
        DeliveryAuthorityDrift::TargetScope(
            ContentDigest::from_bytes(&[0x6a; 32]).expect("target scope"),
        ),
        browser_event(&target_context, 1).payload().clone(),
    )
    .await;

    let binding = TransportFixture::new(position(7, 0)).await;
    let other_key = TransportFixture::new_with_signing_key(position(7, 0), "post-pop", 0x6b).await;
    let other_binding = other_key
        .request(
            subscription(0x69),
            VerifiedOrigin::parse("https://example.test").expect("origin"),
        )
        .binding()
        .clone();
    assert_post_pop_drift_denies(
        &binding,
        DeliveryAuthorityDrift::Binding(other_binding),
        AsyncPayload::Refresh(RegisteredRefresh),
    )
    .await;

    let document_scope = TransportFixture::new(position(7, 0)).await;
    assert_post_pop_drift_denies(
        &document_scope,
        DeliveryAuthorityDrift::DocumentScope,
        AsyncPayload::Refresh(RegisteredRefresh),
    )
    .await;
}

#[tokio::test]
async fn dispatcher_failure_releases_the_lease_without_advancing_sequence() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let (mut bounded, authorization) = bounded_document(
        &fixture,
        0x64,
        vec![AsyncPayload::Refresh(RegisteredRefresh)],
    )
    .await;
    bounded
        .pump_next(fixture.registry.as_ref())
        .await
        .expect("initial admission");
    let mut dispatcher = FailOnDispatcher {
        attempts: 0,
        fail_on: 1,
    };

    let error = bounded
        .dispatch_next(fixture.registry.as_ref(), &mut dispatcher)
        .expect_err("dispatcher failure");
    assert_eq!(
        error.kind(),
        AsyncDeliveryErrorKind::Sequence(SequenceErrorKind::DispatchFailed)
    );
    assert_eq!(
        bounded.sequence_position(&authorization),
        Some(position(7, 0))
    );
    assert_eq!(bounded.active_permits(), 0);
    assert!(bounded.is_degraded());
}

#[tokio::test]
async fn middle_dispatch_failure_commits_only_the_successful_prefix() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let (mut bounded, authorization) = bounded_document(
        &fixture,
        0x65,
        vec![
            AsyncPayload::Heartbeat(Heartbeat),
            AsyncPayload::Heartbeat(Heartbeat),
            AsyncPayload::Heartbeat(Heartbeat),
        ],
    )
    .await;
    for _ in 0..3 {
        bounded
            .pump_next(fixture.registry.as_ref())
            .await
            .expect("bounded admission");
    }
    let mut dispatcher = FailOnDispatcher {
        attempts: 0,
        fail_on: 2,
    };

    assert_eq!(
        bounded.dispatch_next(fixture.registry.as_ref(), &mut dispatcher),
        Ok(Some(SequenceDisposition::Apply))
    );
    assert!(
        bounded
            .dispatch_next(fixture.registry.as_ref(), &mut dispatcher)
            .is_err()
    );
    assert_eq!(
        bounded.sequence_position(&authorization),
        Some(position(7, 1))
    );
    assert_eq!(bounded.retained_events(), 1);
    assert_eq!(bounded.active_permits(), 0);
    assert!(bounded.is_degraded());
}

#[tokio::test]
async fn unwinding_dispatcher_drops_the_lease_and_marks_continuity_degraded() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let (mut bounded, authorization) = bounded_document(
        &fixture,
        0x66,
        vec![AsyncPayload::Refresh(RegisteredRefresh)],
    )
    .await;
    bounded
        .pump_next(fixture.registry.as_ref())
        .await
        .expect("initial admission");
    let mut dispatcher = PanickingDispatcher;

    let panic = catch_unwind(AssertUnwindSafe(|| {
        let _ = bounded.dispatch_next(fixture.registry.as_ref(), &mut dispatcher);
    }));
    assert!(panic.is_err());
    assert_eq!(
        bounded.sequence_position(&authorization),
        Some(position(7, 0))
    );
    assert_eq!(bounded.active_permits(), 0);
    assert!(bounded.is_degraded());
}

#[tokio::test]
async fn duplicate_stale_and_gap_outcomes_remain_truthful_under_pressure() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let (mut bounded, authorization) = bounded_document(&fixture, 0x67, vec![]).await;
    let mut dispatcher = RecordingDispatcher::default();

    bounded
        .admit_replay(
            &authorization,
            vec![heartbeat_at(authorization.context(), 7, 0)],
            fixture.registry.as_ref(),
        )
        .expect("duplicate admission");
    assert_eq!(
        bounded.dispatch_next(fixture.registry.as_ref(), &mut dispatcher),
        Ok(Some(SequenceDisposition::IgnoreDuplicate))
    );
    assert!(!bounded.is_degraded());
    bounded
        .admit_replay(
            &authorization,
            vec![heartbeat_at(authorization.context(), 6, 99)],
            fixture.registry.as_ref(),
        )
        .expect("stale admission");
    assert_eq!(
        bounded.dispatch_next(fixture.registry.as_ref(), &mut dispatcher),
        Ok(Some(SequenceDisposition::IgnoreStaleEpoch))
    );
    assert!(!bounded.is_degraded());
    bounded
        .admit_replay(
            &authorization,
            vec![heartbeat_at(authorization.context(), 7, 2)],
            fixture.registry.as_ref(),
        )
        .expect("gap admission");
    assert_eq!(
        bounded.dispatch_next(fixture.registry.as_ref(), &mut dispatcher),
        Ok(Some(SequenceDisposition::Degraded(
            SequenceDegradation::Gap
        )))
    );
    assert_eq!(dispatcher.applied, 0);
    assert_eq!(
        bounded.sequence_position(&authorization),
        Some(position(7, 0))
    );
    assert_eq!(bounded.active_permits(), 0);
    assert!(bounded.is_degraded());
}

#[tokio::test]
async fn ordered_overflow_never_drops_an_event_while_claiming_current() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let context = fixture
        .request(
            subscription(0x68),
            VerifiedOrigin::parse("https://example.test").expect("origin"),
        )
        .context()
        .clone();
    let (mut bounded, _authorization) = bounded_document_with_bounds(
        &fixture,
        0x68,
        vec![
            browser_event(&context, 1).payload().clone(),
            browser_event(&context, 2).payload().clone(),
        ],
        ResourceBounds::new(1, 256 * 1024).expect("one-item document bound"),
    )
    .await;
    assert_eq!(
        bounded
            .pump_next(fixture.registry.as_ref())
            .await
            .expect("first ordered admission"),
        Some(BufferDisposition::Queued)
    );
    assert_eq!(
        bounded
            .pump_next(fixture.registry.as_ref())
            .await
            .expect("overflow classification"),
        Some(BufferDisposition::Degraded)
    );
    assert_eq!(bounded.retained_events(), 1);
    assert!(bounded.is_degraded());
    let mut dispatcher = RecordingDispatcher::default();
    assert_eq!(
        bounded.dispatch_next(fixture.registry.as_ref(), &mut dispatcher),
        Ok(Some(SequenceDisposition::Apply))
    );
    assert_eq!(dispatcher.applied, 1);
    assert_eq!(bounded.retained_events(), 0);
    assert_eq!(bounded.active_permits(), 0);
}

#[tokio::test]
async fn one_document_fairly_dispatches_chatty_and_healthy_memberships() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let (mut bounded, chatty, healthy) = bounded_two_memberships(
        &fixture,
        0x70,
        0x71,
        0x72,
        vec![
            vec![
                ScriptItem::Envelope(position(7, 1), AsyncPayload::Heartbeat(Heartbeat)),
                ScriptItem::Envelope(position(7, 2), AsyncPayload::Heartbeat(Heartbeat)),
            ],
            vec![ScriptItem::Envelope(
                position(7, 1),
                AsyncPayload::Heartbeat(Heartbeat),
            )],
        ],
        ResourceBounds::new(4, 256 * 1024).expect("aggregate bounds"),
    )
    .await;
    assert_eq!(
        bounded.pump_next(fixture.registry.as_ref()).await,
        Ok(Some(BufferDisposition::Queued))
    );
    assert_eq!(
        bounded.pump_next(fixture.registry.as_ref()).await,
        Ok(Some(BufferDisposition::Queued))
    );
    assert_eq!(bounded.retained_events(), 2);

    let mut dispatcher = RecordingDispatcher::default();
    assert_eq!(
        bounded.dispatch_next(fixture.registry.as_ref(), &mut dispatcher),
        Ok(Some(SequenceDisposition::Apply))
    );
    assert_eq!(
        bounded.dispatch_next(fixture.registry.as_ref(), &mut dispatcher),
        Ok(Some(SequenceDisposition::Apply))
    );
    assert_eq!(
        dispatcher.subscriptions,
        vec![subscription(0x71), subscription(0x72)]
    );
    assert_eq!(bounded.sequence_position(&chatty), Some(position(7, 1)));
    assert_eq!(bounded.sequence_position(&healthy), Some(position(7, 1)));
    assert_eq!(bounded.active_permits(), 0);
}

#[tokio::test]
async fn global_outage_stays_aggregate_bounded_and_keeps_a_healthy_sibling_reachable() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let chatty_script = (1..=100)
        .map(|sequence| {
            ScriptItem::Envelope(position(7, sequence), AsyncPayload::Heartbeat(Heartbeat))
        })
        .collect();
    let (mut bounded, _, _) = bounded_two_memberships(
        &fixture,
        0x73,
        0x74,
        0x75,
        vec![
            chatty_script,
            vec![ScriptItem::Envelope(
                position(7, 1),
                AsyncPayload::Heartbeat(Heartbeat),
            )],
        ],
        ResourceBounds::new(MAX_ASYNC_BUFFER_EVENTS, MAX_ASYNC_BUFFER_BYTES)
            .expect("hard document bounds"),
    )
    .await;

    for _ in 0..101 {
        let _ = bounded
            .pump_next(fixture.registry.as_ref())
            .await
            .expect("bounded outage ingress");
    }
    assert_eq!(bounded.retained_events(), MAX_ASYNC_BUFFER_EVENTS);
    assert!(bounded.retained_bytes() <= MAX_ASYNC_BUFFER_BYTES);
    assert!(bounded.is_degraded());

    let mut dispatcher = RecordingDispatcher::default();
    assert_eq!(
        bounded.dispatch_next(fixture.registry.as_ref(), &mut dispatcher),
        Ok(Some(SequenceDisposition::Apply))
    );
    assert_eq!(
        bounded.dispatch_next(fixture.registry.as_ref(), &mut dispatcher),
        Ok(Some(SequenceDisposition::Apply))
    );
    assert_eq!(
        dispatcher.subscriptions,
        vec![subscription(0x74), subscription(0x75)]
    );
}

#[tokio::test]
async fn removing_one_membership_purges_only_its_queue_and_sequence_lane() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let (mut bounded, removed, healthy) = bounded_two_memberships(
        &fixture,
        0x76,
        0x77,
        0x78,
        vec![
            vec![ScriptItem::Envelope(
                position(7, 1),
                AsyncPayload::Heartbeat(Heartbeat),
            )],
            vec![ScriptItem::Envelope(
                position(7, 1),
                AsyncPayload::Heartbeat(Heartbeat),
            )],
        ],
        ResourceBounds::new(4, 256 * 1024).expect("aggregate bounds"),
    )
    .await;
    bounded
        .pump_next(fixture.registry.as_ref())
        .await
        .expect("removed ingress");
    bounded
        .pump_next(fixture.registry.as_ref())
        .await
        .expect("healthy ingress");

    let pending = bounded.prepare_remove(&removed).expect("prepare remove");
    let ready = pending.authorize().await.expect("authorize remove");
    assert_eq!(bounded.commit_remove(ready), Ok(CloseDisposition::Closed));
    assert_eq!(bounded.retained_events(), 1);
    assert_eq!(bounded.sequence_position(&removed), None);

    let mut dispatcher = RecordingDispatcher::default();
    assert_eq!(
        bounded.dispatch_next(fixture.registry.as_ref(), &mut dispatcher),
        Ok(Some(SequenceDisposition::Apply))
    );
    assert_eq!(dispatcher.subscriptions, vec![subscription(0x78)]);
    assert_eq!(bounded.sequence_position(&healthy), Some(position(7, 1)));
}

#[tokio::test]
async fn source_failure_purges_undelivered_work_and_does_not_claim_current() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let origin = VerifiedOrigin::parse("https://example.test").expect("origin");
    let mut document = fixture.document(
        origin.clone(),
        DocumentTransportKind::ServerSentEvents,
        0x79,
        2,
    );
    let authorization = fixture.request(subscription(0x79), origin);
    let source = ScriptedSource::new(vec![vec![
        ScriptItem::Envelope(position(7, 1), AsyncPayload::Heartbeat(Heartbeat)),
        ScriptItem::Error(suprnova_live::async_updates::AsyncTransportErrorKind::SourceFailed),
    ]]);
    let pending = document
        .prepare_add(authorization.clone())
        .expect("prepare add");
    let authorized = pending.authorize().await.expect("authorize add");
    let establishing = document
        .prepare_establish(authorized)
        .expect("prepare establish");
    let ready = establishing.establish(&source).await.expect("establish");
    document.commit_add(ready).expect("commit add");
    let mut bounded = BoundedDocumentTransportSession::new(
        document,
        ResourceBounds::new(4, 256 * 1024).expect("aggregate bounds"),
        PermitPool::new(1).expect("shared permit"),
        policy(),
    )
    .expect("bounded document");

    bounded
        .pump_next(fixture.registry.as_ref())
        .await
        .expect("first ingress");
    assert_eq!(bounded.retained_events(), 1);
    assert_eq!(
        bounded
            .pump_next(fixture.registry.as_ref())
            .await
            .expect_err("source failure")
            .kind(),
        suprnova_live::async_updates::AsyncTransportErrorKind::SourceFailed
    );
    assert_eq!(bounded.retained_events(), 0);
    assert_eq!(bounded.sequence_position(&authorization), None);
    assert!(bounded.is_degraded());
}

#[tokio::test]
async fn complete_drains_once_while_error_and_heartbeat_remain_nonterminal() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let (mut bounded, completed, erroring) = bounded_two_memberships(
        &fixture,
        0x7a,
        0x7b,
        0x7c,
        vec![
            vec![ScriptItem::Envelope(
                position(7, 1),
                AsyncPayload::Complete(CompletionReason::StreamCompleted),
            )],
            vec![
                ScriptItem::Envelope(
                    position(7, 1),
                    AsyncPayload::Error(StreamErrorCode::Backpressure),
                ),
                ScriptItem::Envelope(position(7, 2), AsyncPayload::Heartbeat(Heartbeat)),
            ],
        ],
        ResourceBounds::new(4, 256 * 1024).expect("aggregate bounds"),
    )
    .await;
    bounded
        .pump_next(fixture.registry.as_ref())
        .await
        .expect("complete ingress");
    assert_eq!(bounded.transport().membership_count(), 1);
    bounded
        .pump_next(fixture.registry.as_ref())
        .await
        .expect("error ingress");
    assert_eq!(bounded.transport().membership_count(), 1);

    let mut dispatcher = RecordingDispatcher::default();
    assert_eq!(
        bounded.dispatch_next(fixture.registry.as_ref(), &mut dispatcher),
        Ok(Some(SequenceDisposition::Apply))
    );
    assert_eq!(bounded.sequence_position(&completed), None);
    assert_eq!(
        bounded.dispatch_next(fixture.registry.as_ref(), &mut dispatcher),
        Ok(Some(SequenceDisposition::Apply))
    );
    assert_eq!(bounded.sequence_position(&erroring), Some(position(7, 1)));
    bounded
        .pump_next(fixture.registry.as_ref())
        .await
        .expect("post-error heartbeat");
    assert_eq!(
        bounded.dispatch_next(fixture.registry.as_ref(), &mut dispatcher),
        Ok(Some(SequenceDisposition::Apply))
    );
    assert_eq!(bounded.sequence_position(&erroring), Some(position(7, 2)));
}

#[tokio::test]
async fn one_thousand_replaceable_refreshes_coalesce_inside_document_bounds() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let payloads = (0..1_000)
        .map(|_| AsyncPayload::Refresh(RegisteredRefresh))
        .collect();
    let (mut bounded, authorization) = bounded_document(&fixture, 0x7d, payloads).await;
    for _ in 0..1_000 {
        assert!(matches!(
            bounded.pump_next(fixture.registry.as_ref()).await,
            Ok(Some(
                BufferDisposition::Queued | BufferDisposition::Coalesced
            ))
        ));
    }
    assert_eq!(bounded.retained_events(), 1);
    assert!(bounded.retained_bytes() <= MAX_ASYNC_BUFFER_BYTES);
    assert!(bounded.is_degraded());

    let mut dispatcher = RecordingDispatcher::default();
    assert_eq!(
        bounded.dispatch_next(fixture.registry.as_ref(), &mut dispatcher),
        Ok(Some(SequenceDisposition::Degraded(
            SequenceDegradation::Gap
        )))
    );
    assert_eq!(
        bounded.sequence_position(&authorization),
        Some(position(7, 0))
    );
    assert_eq!(dispatcher.applied, 0);
}

#[tokio::test]
async fn trusted_fanout_is_enforced_before_queue_allocation() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let context = fixture
        .request(
            subscription(0x7e),
            VerifiedOrigin::parse("https://example.test").expect("origin"),
        )
        .context()
        .clone();
    let (mut policy_limited, _) = bounded_document_with_config(
        &fixture,
        0x7e,
        vec![
            browser_event_for_target(&context, 1, EventTarget::Document)
                .payload()
                .clone(),
        ],
        ResourceBounds::new(4, 256 * 1024).expect("fanout bounds"),
        1,
        AsyncPolicy {
            max_fanout: NonZeroUsize::new(3).expect("policy fanout"),
            ..policy()
        },
    )
    .await;
    fixture.registry.set_resolved_event_fanout(4);
    assert_eq!(
        policy_limited
            .pump_next(fixture.registry.as_ref())
            .await
            .expect("trusted fanout classification"),
        Some(BufferDisposition::Closed(AsyncCloseCode::FanoutExceeded))
    );
    assert_eq!(policy_limited.retained_events(), 0);
    assert_eq!(policy_limited.retained_bytes(), 0);

    let registered = TransportFixture::new(position(7, 0)).await;
    let registered_context = registered
        .request(
            subscription(0x7f),
            VerifiedOrigin::parse("https://example.test").expect("origin"),
        )
        .context()
        .clone();
    let (mut over_registered, _) = bounded_document(
        &registered,
        0x7f,
        vec![
            browser_event_for_target(&registered_context, 1, EventTarget::Document)
                .payload()
                .clone(),
        ],
    )
    .await;
    registered.registry.set_resolved_event_fanout(5);
    assert!(
        over_registered
            .pump_next(registered.registry.as_ref())
            .await
            .is_err()
    );
    assert_eq!(over_registered.retained_events(), 0);
}

#[tokio::test]
async fn aggregate_retirement_cancels_and_drains_exactly_once() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let (mut bounded, authorization) =
        bounded_document(&fixture, 0x80, vec![AsyncPayload::Heartbeat(Heartbeat)]).await;
    bounded
        .pump_next(fixture.registry.as_ref())
        .await
        .expect("queued before retirement");
    let retained_bytes = bounded.retained_bytes();

    assert_eq!(
        bounded.retire_delivery(),
        Retirement {
            canceled: true,
            drained_items: 1,
            drained_bytes: retained_bytes,
        }
    );
    assert_eq!(bounded.retire_delivery(), Retirement::already_retired());
    assert_eq!(bounded.retained_events(), 0);
    assert_eq!(bounded.retained_bytes(), 0);
    assert_eq!(bounded.sequence_position(&authorization), None);
    let mut dispatcher = RecordingDispatcher::default();
    assert_eq!(
        bounded.dispatch_next(fixture.registry.as_ref(), &mut dispatcher),
        Ok(None)
    );
    assert_eq!(bounded.active_permits(), 0);
}

#[tokio::test]
async fn empty_and_over_count_replay_are_atomic_degraded_outcomes() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let (mut empty, authorization) = bounded_document(&fixture, 0x81, vec![]).await;
    assert_eq!(
        empty
            .admit_replay(&authorization, Vec::new(), fixture.registry.as_ref())
            .expect("empty replay classification"),
        BufferDisposition::Degraded
    );
    assert_eq!(empty.retained_events(), 0);

    let limited = TransportFixture::new(position(7, 0)).await;
    let (mut bounded, authorization) = bounded_document_with_config(
        &limited,
        0x82,
        vec![],
        ResourceBounds::new(4, 256 * 1024).expect("replay bounds"),
        1,
        AsyncPolicy {
            max_replay_events: NonZeroUsize::new(1).expect("one replay event"),
            ..policy()
        },
    )
    .await;
    assert_eq!(
        bounded
            .admit_replay(
                &authorization,
                vec![
                    heartbeat_at(authorization.context(), 7, 1),
                    heartbeat_at(authorization.context(), 7, 2),
                ],
                limited.registry.as_ref(),
            )
            .expect("over-count replay classification"),
        BufferDisposition::Degraded
    );
    assert_eq!(bounded.retained_events(), 0);
    assert_eq!(bounded.retained_bytes(), 0);
}

#[tokio::test]
async fn replay_aggregate_byte_boundary_is_exact_and_first_over_is_atomic() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let provisional = fixture.request(
        subscription(0x84),
        VerifiedOrigin::parse("https://example.test").expect("origin"),
    );
    let first = heartbeat_at(provisional.context(), 7, 1);
    let second = heartbeat_at(provisional.context(), 7, 2);
    let aggregate = encode_async_envelope(&first, &AsyncCodecLimits::v1())
        .expect("first wire")
        .len()
        + encode_async_envelope(&second, &AsyncCodecLimits::v1())
            .expect("second wire")
            .len();
    let (mut exact, authorization) = bounded_document_with_config(
        &fixture,
        0x84,
        vec![],
        ResourceBounds::new(2, aggregate).expect("exact replay bytes"),
        1,
        AsyncPolicy {
            max_replay_events: NonZeroUsize::new(2).expect("replay count"),
            ..policy()
        },
    )
    .await;
    assert_eq!(
        exact
            .admit_replay(
                &authorization,
                vec![first, second],
                fixture.registry.as_ref(),
            )
            .expect("exact replay admission"),
        BufferDisposition::Queued
    );
    assert_eq!(exact.retained_events(), 2);
    assert_eq!(exact.retained_bytes(), aggregate);

    let first_over_fixture = TransportFixture::new(position(7, 0)).await;
    let provisional = first_over_fixture.request(
        subscription(0x85),
        VerifiedOrigin::parse("https://example.test").expect("origin"),
    );
    let first = heartbeat_at(provisional.context(), 7, 1);
    let second = heartbeat_at(provisional.context(), 7, 2);
    let aggregate = encode_async_envelope(&first, &AsyncCodecLimits::v1())
        .expect("first wire")
        .len()
        + encode_async_envelope(&second, &AsyncCodecLimits::v1())
            .expect("second wire")
            .len();
    let (mut first_over, authorization) = bounded_document_with_config(
        &first_over_fixture,
        0x85,
        vec![],
        ResourceBounds::new(2, aggregate - 1).expect("first-over replay bytes"),
        1,
        AsyncPolicy {
            max_replay_events: NonZeroUsize::new(2).expect("replay count"),
            ..policy()
        },
    )
    .await;
    assert_eq!(
        first_over
            .admit_replay(
                &authorization,
                vec![first, second],
                first_over_fixture.registry.as_ref(),
            )
            .expect("first-over replay classification"),
        BufferDisposition::Degraded
    );
    assert_eq!(first_over.retained_events(), 0);
    assert_eq!(first_over.retained_bytes(), 0);
}

#[tokio::test]
async fn payload_boundary_is_exact_and_distinct_from_envelope_bytes() {
    let exact_payload_bytes = br#"{"kind":"refresh","name":"refresh"}"#.len();
    let exact_fixture = TransportFixture::new(position(7, 0)).await;
    let (mut exact, _) = bounded_document_with_config(
        &exact_fixture,
        0x86,
        vec![AsyncPayload::Refresh(RegisteredRefresh)],
        ResourceBounds::new(1, MAX_ASYNC_BUFFER_BYTES).expect("exact payload bounds"),
        1,
        AsyncPolicy {
            max_payload_bytes: NonZeroUsize::new(exact_payload_bytes).expect("exact payload"),
            ..policy()
        },
    )
    .await;
    assert_eq!(
        exact.pump_next(exact_fixture.registry.as_ref()).await,
        Ok(Some(BufferDisposition::Queued))
    );

    let rejected_fixture = TransportFixture::new(position(7, 0)).await;
    let (mut rejected, _) = bounded_document_with_config(
        &rejected_fixture,
        0x87,
        vec![AsyncPayload::Refresh(RegisteredRefresh)],
        ResourceBounds::new(1, MAX_ASYNC_BUFFER_BYTES).expect("rejected payload bounds"),
        1,
        AsyncPolicy {
            max_payload_bytes: NonZeroUsize::new(exact_payload_bytes - 1)
                .expect("first rejected payload"),
            ..policy()
        },
    )
    .await;
    assert_eq!(
        rejected.pump_next(rejected_fixture.registry.as_ref()).await,
        Ok(Some(BufferDisposition::Closed(
            AsyncCloseCode::PayloadTooLarge
        )))
    );
    assert_eq!(rejected.retained_events(), 0);
}

#[tokio::test]
async fn invalid_policy_and_document_bounds_fail_before_delivery() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let origin = VerifiedOrigin::parse("https://example.test").expect("origin");
    let invalid_payload = BoundedDocumentTransportSession::new(
        fixture.document(
            origin.clone(),
            DocumentTransportKind::ServerSentEvents,
            0x88,
            1,
        ),
        ResourceBounds::new(1, MAX_ASYNC_BUFFER_BYTES).expect("resource bounds"),
        PermitPool::new(1).expect("permit"),
        AsyncPolicy {
            max_payload_bytes: NonZeroUsize::new(MAX_ASYNC_PAYLOAD_BYTES + 1)
                .expect("invalid payload policy"),
            ..policy()
        },
    )
    .expect_err("payload policy above protocol cap");
    assert_eq!(invalid_payload.close_code(), AsyncCloseCode::InvalidPolicy);

    let invalid_replay = BoundedDocumentTransportSession::new(
        fixture.document(
            origin.clone(),
            DocumentTransportKind::ServerSentEvents,
            0x89,
            1,
        ),
        ResourceBounds::new(1, MAX_ASYNC_BUFFER_BYTES).expect("resource bounds"),
        PermitPool::new(1).expect("permit"),
        AsyncPolicy {
            max_replay_events: NonZeroUsize::new(MAX_REPLAY_TRANSCRIPT_ENVELOPES + 1)
                .expect("invalid replay policy"),
            ..policy()
        },
    )
    .expect_err("replay policy above protocol cap");
    assert_eq!(invalid_replay.close_code(), AsyncCloseCode::InvalidPolicy);

    let invalid_items = BoundedDocumentTransportSession::new(
        fixture.document(
            origin.clone(),
            DocumentTransportKind::ServerSentEvents,
            0x8a,
            1,
        ),
        ResourceBounds::new(MAX_ASYNC_BUFFER_EVENTS + 1, MAX_ASYNC_BUFFER_BYTES)
            .expect("shared generic item bounds"),
        PermitPool::new(1).expect("permit"),
        policy(),
    )
    .expect_err("async document item cap");
    assert_eq!(invalid_items.close_code(), AsyncCloseCode::InvalidPolicy);

    let invalid_bytes = BoundedDocumentTransportSession::new(
        fixture.document(
            origin.clone(),
            DocumentTransportKind::ServerSentEvents,
            0x8b,
            1,
        ),
        ResourceBounds::new(1, MAX_ASYNC_BUFFER_BYTES + 1).expect("shared generic byte bounds"),
        PermitPool::new(1).expect("permit"),
        policy(),
    )
    .expect_err("async document byte cap");
    assert_eq!(invalid_bytes.close_code(), AsyncCloseCode::InvalidPolicy);

    let invalid_fanout = BoundedDocumentTransportSession::new(
        fixture.document(origin, DocumentTransportKind::ServerSentEvents, 0x8c, 1),
        ResourceBounds::new(1, MAX_ASYNC_BUFFER_BYTES).expect("resource bounds"),
        PermitPool::new(1).expect("permit"),
        AsyncPolicy {
            max_fanout: NonZeroUsize::new(usize::from(MAX_EVENT_FANOUT) + 1)
                .expect("invalid fanout policy"),
            ..policy()
        },
    )
    .expect_err("fanout policy above engine cap");
    assert_eq!(invalid_fanout.close_code(), AsyncCloseCode::InvalidPolicy);
}

#[tokio::test]
async fn heartbeat_pressure_never_evicts_a_required_browser_event() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let context = fixture
        .request(
            subscription(0x8d),
            VerifiedOrigin::parse("https://example.test").expect("origin"),
        )
        .context()
        .clone();
    let (mut bounded, _) = bounded_document_with_bounds(
        &fixture,
        0x8d,
        vec![
            browser_event(&context, 1).payload().clone(),
            AsyncPayload::Heartbeat(Heartbeat),
        ],
        ResourceBounds::new(1, MAX_ASYNC_BUFFER_BYTES).expect("heartbeat pressure bounds"),
    )
    .await;
    assert_eq!(
        bounded.pump_next(fixture.registry.as_ref()).await,
        Ok(Some(BufferDisposition::Queued))
    );
    assert_eq!(
        bounded.pump_next(fixture.registry.as_ref()).await,
        Ok(Some(BufferDisposition::Degraded))
    );
    let mut dispatcher = RecordingDispatcher::default();
    assert_eq!(
        bounded.dispatch_next(fixture.registry.as_ref(), &mut dispatcher),
        Ok(Some(SequenceDisposition::Apply))
    );
    assert_eq!(dispatcher.applied, 1);
}

#[tokio::test]
async fn low_cardinality_telemetry_tracks_pressure_without_identity_labels() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let (mut bounded, _) = bounded_document(
        &fixture,
        0x83,
        vec![
            AsyncPayload::Refresh(RegisteredRefresh),
            AsyncPayload::Refresh(RegisteredRefresh),
        ],
    )
    .await;
    assert_eq!(
        bounded.pump_next(fixture.registry.as_ref()).await,
        Ok(Some(BufferDisposition::Queued))
    );
    assert_eq!(
        bounded.pump_next(fixture.registry.as_ref()).await,
        Ok(Some(BufferDisposition::Coalesced))
    );
    let snapshot = bounded.telemetry_snapshot();
    assert_eq!(snapshot.count(AsyncTelemetryCounter::Queued), 1);
    assert_eq!(snapshot.count(AsyncTelemetryCounter::Coalesced), 1);
    assert_eq!(snapshot.count(AsyncTelemetryCounter::Rejected), 0);

    let debug = format!("{bounded:?}");
    assert!(!debug.contains("orders.updated"));
    assert!(!debug.contains("refresh"));
    assert!(!debug.contains(&subscription(0x83).to_base64url()));

    let closed_fixture = TransportFixture::new(position(7, 0)).await;
    let context = closed_fixture
        .request(
            subscription(0x8e),
            VerifiedOrigin::parse("https://example.test").expect("origin"),
        )
        .context()
        .clone();
    let (mut closed, _) = bounded_document_with_config(
        &closed_fixture,
        0x8e,
        vec![
            browser_event_for_target(&context, 1, EventTarget::Document)
                .payload()
                .clone(),
        ],
        ResourceBounds::new(1, MAX_ASYNC_BUFFER_BYTES).expect("telemetry bounds"),
        1,
        AsyncPolicy {
            max_fanout: NonZeroUsize::new(1).expect("one recipient"),
            ..policy()
        },
    )
    .await;
    closed_fixture.registry.set_resolved_event_fanout(2);
    assert_eq!(
        closed.pump_next(closed_fixture.registry.as_ref()).await,
        Ok(Some(BufferDisposition::Closed(
            AsyncCloseCode::FanoutExceeded
        )))
    );
    let closed_snapshot = closed.telemetry_snapshot();
    assert_eq!(closed_snapshot.count(AsyncTelemetryCounter::Closed), 1);
    assert_eq!(closed_snapshot.count(AsyncTelemetryCounter::Rejected), 1);
    assert_eq!(closed_snapshot.count(AsyncTelemetryCounter::Cleanup), 1);
    assert_eq!(AsyncTelemetryCounter::ALL.len(), 6);
    assert!(
        AsyncTelemetryCounter::ALL
            .iter()
            .all(|counter| counter.as_str().len() <= 32)
    );
}
