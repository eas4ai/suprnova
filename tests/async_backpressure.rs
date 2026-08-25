//! Public document-owned asynchronous admission and dispatch tests.

use std::num::NonZeroUsize;
use std::panic::{AssertUnwindSafe, catch_unwind};

use suprnova_live::async_updates::{
    AsyncCloseCode, AsyncCodecLimits, AsyncDeliveryDisposition, AsyncDeliveryErrorKind,
    AsyncDispatchError, AsyncEnvelope, AsyncEnvelopeContext, AsyncEnvelopeDispatchPort,
    AsyncPayload, AsyncPolicy, AsyncTelemetryCounter, BoundedDocumentTransportSession,
    BufferDisposition, CloseDisposition, CompletionReason, DocumentTransportKind, EventTarget,
    Heartbeat, MAX_ASYNC_BUFFER_BYTES, MAX_ASYNC_BUFFER_EVENTS, MAX_ASYNC_PAYLOAD_BYTES,
    MAX_EVENT_FANOUT, MAX_REPLAY_TRANSCRIPT_ENVELOPES, RegisteredBrowserEvent, RegisteredRefresh,
    SequenceDegradation, SequenceDisposition, SequenceErrorKind, SequenceState, StreamErrorCode,
    SubscriptionId, VerifiedOrigin, encode_async_envelope,
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
    DeliveryAuthorityDrift, ScriptItem, ScriptedSource, TransportFixture, WakeGate, position,
    subscription,
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

async fn bounded_document_with_script(
    fixture: &TransportFixture,
    subscription_marker: u8,
    script: Vec<ScriptItem>,
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
    let source = ScriptedSource::new(vec![script]);
    let pending = document.prepare_add(request.clone()).expect("prepare add");
    let authorized = pending.authorize().await.expect("authorize add");
    let establishing = document
        .prepare_establish(authorized)
        .expect("prepare establish");
    let ready = establishing.establish(&source).await.expect("establish");
    document.commit_add(ready).expect("commit add");
    let bounded = BoundedDocumentTransportSession::new(
        document,
        ResourceBounds::new(64, 256 * 1024).expect("document bounds"),
        PermitPool::new(4).expect("shared permits"),
        policy(),
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

async fn add_empty_membership(
    bounded: &mut BoundedDocumentTransportSession,
    authorization: suprnova_live::async_updates::AuthorizedTransportSubscription,
) {
    let source = ScriptedSource::new(vec![vec![ScriptItem::End]]);
    try_add_membership(bounded, authorization, &source)
        .await
        .expect("commit empty membership");
}

async fn try_add_membership(
    bounded: &mut BoundedDocumentTransportSession,
    authorization: suprnova_live::async_updates::AuthorizedTransportSubscription,
    source: &ScriptedSource,
) -> Result<(), suprnova_live::async_updates::AsyncTransportError> {
    let pending = bounded
        .prepare_add(authorization)
        .expect("prepare empty membership");
    let authorized = pending
        .authorize()
        .await
        .expect("authorize empty membership");
    let establishing = bounded
        .prepare_establish(authorized)
        .expect("prepare empty establishment");
    let ready = establishing
        .establish(source)
        .await
        .expect("establish empty membership");
    bounded.commit_add(ready)
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
        Ok(Some(AsyncDeliveryDisposition::Sequence(
            SequenceDisposition::Apply
        )))
    );
    assert_eq!(dispatcher.applied, 1);
    assert_eq!(bounded.sequence_position(&request), Some(position(7, 1)));
}

#[tokio::test]
async fn final_authority_validation_follows_the_last_host_clock_callback_before_admission() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let (mut bounded, _) = bounded_document(
        &fixture,
        0x5a,
        vec![AsyncPayload::Refresh(RegisteredRefresh)],
    )
    .await;
    fixture
        .registry
        .drift_after_now_call(3, DeliveryAuthorityDrift::Revoke);

    assert!(bounded.pump_next(fixture.registry.as_ref()).await.is_err());
    assert_eq!(bounded.retained_events(), 0);
    assert_eq!(bounded.retained_bytes(), 0);
}

#[tokio::test]
async fn final_authority_validation_follows_the_last_host_clock_callback_before_dispatch() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let (mut bounded, authorization) = bounded_document(
        &fixture,
        0x5b,
        vec![AsyncPayload::Refresh(RegisteredRefresh)],
    )
    .await;
    assert_eq!(
        bounded.pump_next(fixture.registry.as_ref()).await,
        Ok(Some(BufferDisposition::Queued))
    );
    fixture
        .registry
        .drift_after_now_call(3, DeliveryAuthorityDrift::Revoke);

    let mut dispatcher = RecordingDispatcher::default();
    assert_eq!(
        bounded
            .dispatch_next(fixture.registry.as_ref(), &mut dispatcher)
            .expect_err("host drift must deny registered dispatch")
            .kind(),
        AsyncDeliveryErrorKind::AuthorizationLost
    );
    assert_eq!(dispatcher.applied, 0);
    assert_eq!(
        bounded.sequence_position(&authorization),
        Some(position(7, 0))
    );
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
async fn replay_final_validation_follows_the_last_host_clock_callback_before_batch_commit() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let (mut bounded, authorization) = bounded_document(&fixture, 0x6e, vec![]).await;
    fixture
        .registry
        .drift_after_now_call(5, DeliveryAuthorityDrift::Revoke);

    assert!(
        bounded
            .admit_replay(
                &authorization,
                vec![
                    heartbeat_at(authorization.context(), 7, 1),
                    heartbeat_at(authorization.context(), 7, 2),
                ],
                fixture.registry.as_ref(),
            )
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
async fn empty_eof_prunes_terminal_drain_and_lane_before_exact_identity_reuse() {
    let first = TransportFixture::new(position(7, 0)).await;
    let (mut bounded, first_authorization) = bounded_document(&first, 0x6d, vec![]).await;

    assert_eq!(bounded.delivery_lane_count(), 1);
    assert_eq!(bounded.terminal_drain_count(), 0);
    assert_eq!(bounded.pump_next(first.registry.as_ref()).await, Ok(None));
    assert_eq!(bounded.delivery_lane_count(), 0);
    assert_eq!(bounded.terminal_drain_count(), 0);
    assert_eq!(bounded.sequence_position(&first_authorization), None);

    add_empty_membership(&mut bounded, first_authorization.clone()).await;
    assert_eq!(bounded.delivery_lane_count(), 1);
    assert_eq!(bounded.pump_next(first.registry.as_ref()).await, Ok(None));
    assert_eq!(bounded.delivery_lane_count(), 0);
    assert_eq!(bounded.terminal_drain_count(), 0);

    for (key_id, marker) in [("rotated-a", 0xa1), ("rotated-b", 0xa2)] {
        let rotated = TransportFixture::new_with_signing_key(position(7, 0), key_id, marker).await;
        let authorization = rotated.request(
            subscription(0x6d),
            VerifiedOrigin::parse("https://example.test").expect("origin"),
        );
        add_empty_membership(&mut bounded, authorization.clone()).await;
        assert_eq!(bounded.delivery_lane_count(), 1);
        assert_eq!(bounded.terminal_drain_count(), 0);
        assert_eq!(bounded.pump_next(rotated.registry.as_ref()).await, Ok(None));
        assert_eq!(bounded.delivery_lane_count(), 0);
        assert_eq!(bounded.terminal_drain_count(), 0);
        assert_eq!(bounded.sequence_position(&authorization), None);
    }
}

#[tokio::test]
async fn queued_terminal_fences_exact_and_rotated_readmission_until_delivery() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let (mut bounded, authorization) = bounded_document(
        &fixture,
        0x75,
        vec![AsyncPayload::Complete(CompletionReason::StreamCompleted)],
    )
    .await;
    assert_eq!(
        bounded.pump_next(fixture.registry.as_ref()).await,
        Ok(Some(BufferDisposition::Queued))
    );
    assert_eq!(bounded.terminal_drain_count(), 1);
    assert_eq!(bounded.delivery_lane_count(), 1);

    let exact_source = ScriptedSource::new(vec![vec![ScriptItem::End]]);
    let exact = try_add_membership(&mut bounded, authorization.clone(), &exact_source)
        .await
        .expect_err("the retained exact sequence lane fences readmission");
    assert_eq!(
        exact.kind(),
        suprnova_live::async_updates::AsyncTransportErrorKind::DuplicateMembership
    );
    assert_eq!(exact_source.drop_count(), 1);
    assert_eq!(bounded.terminal_drain_count(), 1);
    assert_eq!(bounded.delivery_lane_count(), 1);

    let rotated_fixture =
        TransportFixture::new_with_signing_key(position(7, 0), "terminal-rotated", 0x76).await;
    let rotated = rotated_fixture.request(
        subscription(0x75),
        VerifiedOrigin::parse("https://example.test").expect("origin"),
    );
    let rotated_source = ScriptedSource::new(vec![vec![ScriptItem::End]]);
    let mismatch = try_add_membership(&mut bounded, rotated, &rotated_source)
        .await
        .expect_err("a rotated binding cannot overlap a retained predecessor lane");
    assert_eq!(
        mismatch.kind(),
        suprnova_live::async_updates::AsyncTransportErrorKind::DescriptorMismatch
    );
    assert_eq!(rotated_source.drop_count(), 1);
    assert_eq!(bounded.terminal_drain_count(), 1);
    assert_eq!(bounded.delivery_lane_count(), 1);

    let mut dispatcher = RecordingDispatcher::default();
    assert!(matches!(
        bounded.dispatch_next(fixture.registry.as_ref(), &mut dispatcher),
        Ok(Some(AsyncDeliveryDisposition::Sequence(
            SequenceDisposition::Apply
        )))
    ));
    assert_eq!(bounded.terminal_drain_count(), 0);
    assert_eq!(bounded.delivery_lane_count(), 0);

    add_empty_membership(&mut bounded, authorization.clone()).await;
    assert_eq!(bounded.delivery_lane_count(), 1);
    assert_eq!(
        bounded.sequence_position(&authorization),
        Some(position(7, 0))
    );
    assert_eq!(bounded.pump_next(fixture.registry.as_ref()).await, Ok(None));
    assert_eq!(bounded.delivery_lane_count(), 0);

    let rotated = rotated_fixture.request(
        subscription(0x75),
        VerifiedOrigin::parse("https://example.test").expect("origin"),
    );
    add_empty_membership(&mut bounded, rotated.clone()).await;
    assert_eq!(bounded.delivery_lane_count(), 1);
    assert_eq!(bounded.sequence_position(&authorization), None);
    assert_eq!(bounded.sequence_position(&rotated), Some(position(7, 0)));
}

#[tokio::test]
async fn empty_eof_is_pruned_even_when_a_healthy_sibling_is_returned_in_the_same_pump() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let (mut bounded, completed, healthy) = bounded_two_memberships(
        &fixture,
        0x6f,
        0x70,
        0x71,
        vec![
            vec![ScriptItem::End],
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
    assert_eq!(bounded.terminal_drain_count(), 0);
    assert_eq!(bounded.delivery_lane_count(), 1);
    assert_eq!(bounded.sequence_position(&completed), None);
    assert_eq!(bounded.sequence_position(&healthy), Some(position(7, 0)));
}

#[tokio::test]
async fn empty_eof_is_pruned_when_the_same_pump_observes_a_sibling_source_failure() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let (mut bounded, completed, failing) = bounded_two_memberships(
        &fixture,
        0x72,
        0x73,
        0x74,
        vec![
            vec![ScriptItem::End],
            vec![ScriptItem::Error(
                suprnova_live::async_updates::AsyncTransportErrorKind::SourceFailed,
            )],
        ],
        ResourceBounds::new(4, 256 * 1024).expect("aggregate bounds"),
    )
    .await;

    assert_eq!(
        bounded
            .pump_next(fixture.registry.as_ref())
            .await
            .expect_err("sibling source failure")
            .kind(),
        suprnova_live::async_updates::AsyncTransportErrorKind::SourceFailed
    );
    assert_eq!(bounded.terminal_drain_count(), 0);
    assert_eq!(bounded.delivery_lane_count(), 0);
    assert_eq!(bounded.sequence_position(&completed), None);
    assert_eq!(bounded.sequence_position(&failing), None);
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
        Ok(Some(AsyncDeliveryDisposition::Sequence(
            SequenceDisposition::Apply
        )))
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
    let provisional = fixture.request(
        subscription(0x67),
        VerifiedOrigin::parse("https://example.test").expect("origin"),
    );
    let (mut bounded, authorization) = bounded_document_with_script(
        &fixture,
        0x67,
        vec![
            ScriptItem::RawEnvelope(heartbeat_at(provisional.context(), 7, 0)),
            ScriptItem::RawEnvelope(heartbeat_at(provisional.context(), 6, 99)),
            ScriptItem::RawEnvelope(heartbeat_at(provisional.context(), 7, 2)),
        ],
    )
    .await;
    let mut dispatcher = RecordingDispatcher::default();

    bounded
        .pump_next(fixture.registry.as_ref())
        .await
        .expect("duplicate admission");
    assert_eq!(
        bounded.dispatch_next(fixture.registry.as_ref(), &mut dispatcher),
        Ok(Some(AsyncDeliveryDisposition::Sequence(
            SequenceDisposition::IgnoreDuplicate
        )))
    );
    assert!(!bounded.is_degraded());
    bounded
        .pump_next(fixture.registry.as_ref())
        .await
        .expect("stale admission");
    assert_eq!(
        bounded.dispatch_next(fixture.registry.as_ref(), &mut dispatcher),
        Ok(Some(AsyncDeliveryDisposition::Sequence(
            SequenceDisposition::IgnoreStaleEpoch
        )))
    );
    assert!(!bounded.is_degraded());
    bounded
        .pump_next(fixture.registry.as_ref())
        .await
        .expect("gap admission");
    assert_eq!(
        bounded.dispatch_next(fixture.registry.as_ref(), &mut dispatcher),
        Ok(Some(AsyncDeliveryDisposition::Sequence(
            SequenceDisposition::Degraded(SequenceDegradation::Gap)
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
        Ok(Some(AsyncDeliveryDisposition::Sequence(
            SequenceDisposition::Apply
        )))
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
        Ok(Some(AsyncDeliveryDisposition::Sequence(
            SequenceDisposition::Apply
        )))
    );
    assert_eq!(
        bounded.dispatch_next(fixture.registry.as_ref(), &mut dispatcher),
        Ok(Some(AsyncDeliveryDisposition::Sequence(
            SequenceDisposition::Apply
        )))
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
        Ok(Some(AsyncDeliveryDisposition::Sequence(
            SequenceDisposition::Apply
        )))
    );
    assert_eq!(
        bounded.dispatch_next(fixture.registry.as_ref(), &mut dispatcher),
        Ok(Some(AsyncDeliveryDisposition::Sequence(
            SequenceDisposition::Apply
        )))
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
        Ok(Some(AsyncDeliveryDisposition::Sequence(
            SequenceDisposition::Apply
        )))
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
    assert_eq!(bounded.terminal_drain_count(), 1);
    assert_eq!(bounded.sequence_position(&completed), Some(position(7, 0)));
    bounded
        .pump_next(fixture.registry.as_ref())
        .await
        .expect("error ingress");
    assert_eq!(bounded.transport().membership_count(), 1);
    assert_eq!(bounded.terminal_drain_count(), 1);
    assert_eq!(bounded.sequence_position(&completed), Some(position(7, 0)));

    let mut dispatcher = RecordingDispatcher::default();
    assert_eq!(
        bounded.dispatch_next(fixture.registry.as_ref(), &mut dispatcher),
        Ok(Some(AsyncDeliveryDisposition::Sequence(
            SequenceDisposition::Apply
        )))
    );
    assert_eq!(bounded.sequence_position(&completed), None);
    assert_eq!(
        bounded.dispatch_next(fixture.registry.as_ref(), &mut dispatcher),
        Ok(Some(AsyncDeliveryDisposition::Sequence(
            SequenceDisposition::Apply
        )))
    );
    assert_eq!(bounded.sequence_position(&erroring), Some(position(7, 1)));
    bounded
        .pump_next(fixture.registry.as_ref())
        .await
        .expect("post-error heartbeat");
    assert_eq!(
        bounded.dispatch_next(fixture.registry.as_ref(), &mut dispatcher),
        Ok(Some(AsyncDeliveryDisposition::Sequence(
            SequenceDisposition::Apply
        )))
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
        Ok(Some(AsyncDeliveryDisposition::Sequence(
            SequenceDisposition::Degraded(SequenceDegradation::Gap)
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
    assert_eq!(fixture.registry.delivery_validation_call_count(), 0);

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
    assert_eq!(limited.registry.delivery_validation_call_count(), 0);

    let huge = TransportFixture::new(position(7, 0)).await;
    let (mut bounded, authorization) = bounded_document(&huge, 0x83, vec![]).await;
    let envelope = heartbeat_at(authorization.context(), 7, 1);
    assert_eq!(
        bounded
            .admit_replay(
                &authorization,
                vec![envelope; MAX_REPLAY_TRANSCRIPT_ENVELOPES + 1],
                huge.registry.as_ref(),
            )
            .expect("global over-count replay classification"),
        BufferDisposition::Degraded
    );
    assert_eq!(huge.registry.delivery_validation_call_count(), 0);
    assert_eq!(bounded.retained_events(), 0);
    assert_eq!(bounded.retained_bytes(), 0);
}

#[tokio::test]
async fn complete_replay_dispatches_as_one_recovery_unit_and_restores_currentness() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let (mut bounded, _authorization) = bounded_document(
        &fixture,
        0x91,
        vec![
            AsyncPayload::Heartbeat(Heartbeat),
            AsyncPayload::Heartbeat(Heartbeat),
        ],
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
    let mut dispatcher = RecordingDispatcher::default();
    assert_eq!(
        bounded.dispatch_next(fixture.registry.as_ref(), &mut dispatcher),
        Ok(Some(AsyncDeliveryDisposition::Sequence(
            SequenceDisposition::Apply
        )))
    );
    assert_eq!(
        bounded.dispatch_next(fixture.registry.as_ref(), &mut dispatcher),
        Ok(Some(AsyncDeliveryDisposition::Sequence(
            SequenceDisposition::Apply
        )))
    );

    let gap_fixture = TransportFixture::new(position(7, 0)).await;
    let context = gap_fixture
        .request(
            subscription(0x92),
            VerifiedOrigin::parse("https://example.test").expect("origin"),
        )
        .context()
        .clone();
    let (mut bounded, authorization) = bounded_document_with_script(
        &gap_fixture,
        0x92,
        vec![ScriptItem::RawEnvelope(heartbeat_at(&context, 7, 2))],
    )
    .await;
    assert_eq!(
        bounded.pump_next(gap_fixture.registry.as_ref()).await,
        Ok(Some(BufferDisposition::Queued))
    );
    let mut dispatcher = RecordingDispatcher::default();
    assert_eq!(
        bounded.dispatch_next(gap_fixture.registry.as_ref(), &mut dispatcher),
        Ok(Some(AsyncDeliveryDisposition::Sequence(
            SequenceDisposition::Degraded(SequenceDegradation::Gap)
        )))
    );
    assert!(bounded.is_degraded());
    assert_eq!(
        bounded
            .admit_replay(
                &authorization,
                vec![
                    heartbeat_at(authorization.context(), 7, 1),
                    heartbeat_at(authorization.context(), 7, 2),
                ],
                gap_fixture.registry.as_ref(),
            )
            .expect("complete replay admission"),
        BufferDisposition::Queued
    );

    let outcome = bounded
        .dispatch_next(gap_fixture.registry.as_ref(), &mut dispatcher)
        .expect("replay dispatch")
        .expect("replay outcome");
    let AsyncDeliveryDisposition::Replay(outcome) = outcome else {
        panic!("replay transcript must preserve its recovery boundary")
    };
    assert_eq!(outcome.applied(), 2);
    assert_eq!(outcome.current(), position(7, 2));
    assert_eq!(outcome.state(), SequenceState::Current);
    assert_eq!(
        bounded.sequence_position(&authorization),
        Some(position(7, 2))
    );
    assert_eq!(
        bounded.sequence_state(&authorization),
        Some(SequenceState::Current)
    );
    assert!(!bounded.is_degraded());
    assert_eq!(bounded.retained_events(), 0);
    assert_eq!(bounded.active_permits(), 0);
}

#[tokio::test]
async fn ordinary_replaceable_tail_cannot_split_an_admitted_replay_group() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let provisional = fixture.request(
        subscription(0x95),
        VerifiedOrigin::parse("https://example.test").expect("origin"),
    );
    let (mut bounded, authorization) = bounded_document_with_script(
        &fixture,
        0x95,
        vec![
            ScriptItem::RawEnvelope(refresh(provisional.context(), 3)),
            ScriptItem::RawEnvelope(refresh(provisional.context(), 4)),
        ],
    )
    .await;
    bounded
        .pump_next(fixture.registry.as_ref())
        .await
        .expect("gap admission");
    let mut dispatcher = RecordingDispatcher::default();
    bounded
        .dispatch_next(fixture.registry.as_ref(), &mut dispatcher)
        .expect("gap classification");
    assert_eq!(
        bounded
            .admit_replay(
                &authorization,
                vec![
                    refresh(authorization.context(), 1),
                    refresh(authorization.context(), 2),
                    refresh(authorization.context(), 3),
                ],
                fixture.registry.as_ref(),
            )
            .expect("replay admission"),
        BufferDisposition::Queued
    );
    assert_eq!(
        bounded.pump_next(fixture.registry.as_ref()).await,
        Ok(Some(BufferDisposition::Queued))
    );
    assert_eq!(bounded.retained_events(), 4);

    let outcome = bounded
        .dispatch_next(fixture.registry.as_ref(), &mut dispatcher)
        .expect("replay dispatch")
        .expect("replay outcome");
    assert!(matches!(outcome, AsyncDeliveryDisposition::Replay(_)));
    assert_eq!(
        bounded.dispatch_next(fixture.registry.as_ref(), &mut dispatcher),
        Ok(Some(AsyncDeliveryDisposition::Sequence(
            SequenceDisposition::Apply
        )))
    );
    assert_eq!(
        bounded.sequence_position(&authorization),
        Some(position(7, 4))
    );
    assert_eq!(bounded.retained_events(), 0);
    assert!(
        !bounded.is_degraded(),
        "a proven replay recovery clears after the unrelated queued successor drains"
    );
}

#[tokio::test]
async fn authenticated_sibling_removal_commits_a_proven_deferred_recovery_when_queue_empties() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let origin = VerifiedOrigin::parse("https://example.test").expect("origin");
    let first = fixture.request(subscription(0xa3), origin.clone());
    let (mut bounded, first) = bounded_document_with_script(
        &fixture,
        0xa3,
        vec![ScriptItem::RawEnvelope(heartbeat_at(first.context(), 7, 3))],
    )
    .await;
    bounded
        .pump_next(fixture.registry.as_ref())
        .await
        .expect("first gap admission");
    let mut dispatcher = RecordingDispatcher::default();
    assert!(matches!(
        bounded.dispatch_next(fixture.registry.as_ref(), &mut dispatcher),
        Ok(Some(AsyncDeliveryDisposition::Sequence(
            SequenceDisposition::Degraded(SequenceDegradation::Gap)
        )))
    ));
    assert_eq!(
        bounded
            .admit_replay(
                &first,
                vec![
                    heartbeat_at(first.context(), 7, 1),
                    heartbeat_at(first.context(), 7, 2),
                    heartbeat_at(first.context(), 7, 3),
                ],
                fixture.registry.as_ref(),
            )
            .expect("first recovery admission"),
        BufferDisposition::Queued
    );

    let second = fixture.request(subscription(0xa4), origin);
    let second_source = ScriptedSource::new(vec![vec![ScriptItem::RawEnvelope(heartbeat_at(
        second.context(),
        7,
        1,
    ))]]);
    try_add_membership(&mut bounded, second.clone(), &second_source)
        .await
        .expect("commit second membership");
    assert_eq!(
        bounded.pump_next(fixture.registry.as_ref()).await,
        Ok(Some(BufferDisposition::Queued))
    );
    assert_eq!(bounded.retained_events(), 4);

    assert!(matches!(
        bounded.dispatch_next(fixture.registry.as_ref(), &mut dispatcher),
        Ok(Some(AsyncDeliveryDisposition::Replay(_)))
    ));
    assert_eq!(bounded.retained_events(), 1);
    assert_eq!(bounded.sequence_state(&first), None);
    assert!(bounded.is_degraded());
    assert_eq!(bounded.unresolved_pressure_cause_count(), 1);

    let remove = bounded
        .prepare_remove(&second)
        .expect("prepare sibling removal");
    let ready = remove.authorize().await.expect("authorize sibling removal");
    assert_eq!(bounded.commit_remove(ready), Ok(CloseDisposition::Closed));
    assert_eq!(bounded.retained_events(), 0);
    assert_eq!(bounded.unresolved_pressure_cause_count(), 0);
    assert!(!bounded.is_degraded());
}

#[tokio::test]
async fn provider_failure_purge_commits_only_a_proven_siblings_deferred_recovery() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let origin = VerifiedOrigin::parse("https://example.test").expect("origin");
    let first = fixture.request(subscription(0xa5), origin.clone());
    let (mut bounded, first) = bounded_document_with_script(
        &fixture,
        0xa5,
        vec![ScriptItem::RawEnvelope(heartbeat_at(first.context(), 7, 3))],
    )
    .await;
    bounded
        .pump_next(fixture.registry.as_ref())
        .await
        .expect("first gap admission");
    let mut dispatcher = RecordingDispatcher::default();
    bounded
        .dispatch_next(fixture.registry.as_ref(), &mut dispatcher)
        .expect("first gap classification");
    assert_eq!(
        bounded
            .admit_replay(
                &first,
                vec![
                    heartbeat_at(first.context(), 7, 1),
                    heartbeat_at(first.context(), 7, 2),
                    heartbeat_at(first.context(), 7, 3),
                ],
                fixture.registry.as_ref(),
            )
            .expect("first recovery admission"),
        BufferDisposition::Queued
    );

    let second = fixture.request(subscription(0xa6), origin);
    let second_source = ScriptedSource::new(vec![vec![
        ScriptItem::RawEnvelope(heartbeat_at(second.context(), 7, 1)),
        ScriptItem::Error(suprnova_live::async_updates::AsyncTransportErrorKind::SourceFailed),
    ]]);
    try_add_membership(&mut bounded, second, &second_source)
        .await
        .expect("commit failing sibling membership");
    assert_eq!(
        bounded.pump_next(fixture.registry.as_ref()).await,
        Ok(Some(BufferDisposition::Queued))
    );
    assert!(matches!(
        bounded.dispatch_next(fixture.registry.as_ref(), &mut dispatcher),
        Ok(Some(AsyncDeliveryDisposition::Replay(_)))
    ));
    assert_eq!(bounded.retained_events(), 1);
    assert_eq!(bounded.unresolved_pressure_cause_count(), 1);

    let error = bounded
        .pump_next(fixture.registry.as_ref())
        .await
        .expect_err("failing sibling provider must be purged");
    assert_eq!(
        error.kind(),
        suprnova_live::async_updates::AsyncTransportErrorKind::SourceFailed
    );
    assert_eq!(bounded.retained_events(), 0);
    assert_eq!(
        bounded.unresolved_pressure_cause_count(),
        1,
        "the proven sibling recovery commits while the failed membership remains degraded"
    );
    assert!(bounded.is_degraded());
}

#[tokio::test]
async fn partial_replay_dispatch_failure_keeps_truthful_prefix_and_degraded_continuity() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let provisional = fixture.request(
        subscription(0x93),
        VerifiedOrigin::parse("https://example.test").expect("origin"),
    );
    let (mut bounded, authorization) = bounded_document_with_script(
        &fixture,
        0x93,
        vec![ScriptItem::RawEnvelope(heartbeat_at(
            provisional.context(),
            7,
            3,
        ))],
    )
    .await;
    bounded
        .pump_next(fixture.registry.as_ref())
        .await
        .expect("gap admission");
    let mut initial = RecordingDispatcher::default();
    assert_eq!(
        bounded.dispatch_next(fixture.registry.as_ref(), &mut initial),
        Ok(Some(AsyncDeliveryDisposition::Sequence(
            SequenceDisposition::Degraded(SequenceDegradation::Gap)
        )))
    );
    assert_eq!(
        bounded
            .admit_replay(
                &authorization,
                vec![
                    heartbeat_at(authorization.context(), 7, 1),
                    heartbeat_at(authorization.context(), 7, 2),
                    heartbeat_at(authorization.context(), 7, 3),
                ],
                fixture.registry.as_ref(),
            )
            .expect("replay admission"),
        BufferDisposition::Queued
    );

    let mut dispatcher = FailOnDispatcher {
        attempts: 0,
        fail_on: 2,
    };
    let error = bounded
        .dispatch_next(fixture.registry.as_ref(), &mut dispatcher)
        .expect_err("middle replay failure");
    assert_eq!(
        error.kind(),
        AsyncDeliveryErrorKind::Sequence(SequenceErrorKind::DispatchFailed)
    );
    let replay_error = error
        .replay_error()
        .expect("replay failure retains its truthful committed prefix");
    assert_eq!(replay_error.applied(), 1);
    assert_eq!(replay_error.current(), position(7, 1));
    assert_eq!(replay_error.state(), SequenceState::Degraded);
    assert_eq!(replay_error.high_water(), Some(position(7, 3)));
    assert_eq!(dispatcher.attempts, 2);
    assert_eq!(
        bounded.sequence_position(&authorization),
        Some(position(7, 1))
    );
    assert_eq!(
        bounded.sequence_state(&authorization),
        Some(SequenceState::Degraded)
    );
    assert!(bounded.is_degraded());
    assert_eq!(bounded.retained_events(), 0);
    assert_eq!(bounded.active_permits(), 0);
}

#[tokio::test]
async fn replay_authority_loss_after_dequeue_keeps_recovery_degraded_without_dispatch() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let provisional = fixture.request(
        subscription(0x94),
        VerifiedOrigin::parse("https://example.test").expect("origin"),
    );
    let (mut bounded, authorization) = bounded_document_with_script(
        &fixture,
        0x94,
        vec![ScriptItem::RawEnvelope(heartbeat_at(
            provisional.context(),
            7,
            2,
        ))],
    )
    .await;
    bounded
        .pump_next(fixture.registry.as_ref())
        .await
        .expect("gap admission");
    let mut initial = RecordingDispatcher::default();
    bounded
        .dispatch_next(fixture.registry.as_ref(), &mut initial)
        .expect("gap classification");
    bounded
        .admit_replay(
            &authorization,
            vec![
                heartbeat_at(authorization.context(), 7, 1),
                heartbeat_at(authorization.context(), 7, 2),
            ],
            fixture.registry.as_ref(),
        )
        .expect("replay admission");
    fixture
        .registry
        .drift_after_delivery_validation(1, DeliveryAuthorityDrift::Revoke);

    let mut dispatcher = RecordingDispatcher::default();
    let error = bounded
        .dispatch_next(fixture.registry.as_ref(), &mut dispatcher)
        .expect_err("replay authorization loss");
    assert_eq!(error.kind(), AsyncDeliveryErrorKind::AuthorizationLost);
    assert_eq!(dispatcher.applied, 0);
    assert_eq!(
        bounded.sequence_position(&authorization),
        Some(position(7, 0))
    );
    assert_eq!(
        bounded.sequence_state(&authorization),
        Some(SequenceState::Degraded)
    );
    assert!(bounded.is_degraded());
    assert_eq!(bounded.retained_events(), 0);
    assert_eq!(bounded.active_permits(), 0);
}

#[tokio::test]
async fn one_membership_replay_cannot_clear_a_healthy_documents_other_degraded_lane() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let (mut bounded, first, second) = bounded_two_memberships(
        &fixture,
        0x96,
        0x97,
        0x98,
        vec![
            vec![ScriptItem::Envelope(
                position(7, 2),
                AsyncPayload::Heartbeat(Heartbeat),
            )],
            vec![ScriptItem::Envelope(
                position(7, 2),
                AsyncPayload::Heartbeat(Heartbeat),
            )],
        ],
        ResourceBounds::new(8, 256 * 1024).expect("aggregate bounds"),
    )
    .await;
    let mut dispatcher = RecordingDispatcher::default();
    for _ in 0..2 {
        bounded
            .pump_next(fixture.registry.as_ref())
            .await
            .expect("gap ingress");
        bounded
            .dispatch_next(fixture.registry.as_ref(), &mut dispatcher)
            .expect("gap classification");
    }
    assert_eq!(
        bounded.sequence_state(&first),
        Some(SequenceState::Degraded)
    );
    assert_eq!(
        bounded.sequence_state(&second),
        Some(SequenceState::Degraded)
    );
    assert!(bounded.is_degraded());

    bounded
        .admit_replay(
            &first,
            vec![
                heartbeat_at(first.context(), 7, 1),
                heartbeat_at(first.context(), 7, 2),
            ],
            fixture.registry.as_ref(),
        )
        .expect("first replay admission");
    bounded
        .dispatch_next(fixture.registry.as_ref(), &mut dispatcher)
        .expect("first replay dispatch");
    assert_eq!(bounded.sequence_state(&first), Some(SequenceState::Current));
    assert_eq!(
        bounded.sequence_state(&second),
        Some(SequenceState::Degraded)
    );
    assert!(bounded.is_degraded());

    bounded
        .admit_replay(
            &second,
            vec![
                heartbeat_at(second.context(), 7, 1),
                heartbeat_at(second.context(), 7, 2),
            ],
            fixture.registry.as_ref(),
        )
        .expect("second replay admission");
    bounded
        .dispatch_next(fixture.registry.as_ref(), &mut dispatcher)
        .expect("second replay dispatch");
    assert_eq!(
        bounded.sequence_state(&second),
        Some(SequenceState::Current)
    );
    assert!(!bounded.is_degraded());
}

#[tokio::test]
async fn sibling_replay_cannot_clear_an_exact_ordered_overflow_cause() {
    use std::sync::Arc;

    let fixture = TransportFixture::new(position(7, 0)).await;
    let origin = VerifiedOrigin::parse("https://example.test").expect("origin");
    let first = fixture.request(subscription(0x99), origin.clone());
    let mut document = fixture.document(
        origin.clone(),
        DocumentTransportKind::ServerSentEvents,
        0x9a,
        4,
    );
    let first_gate = Arc::new(WakeGate::new());
    let first_source = ScriptedSource::new(vec![vec![
        ScriptItem::RawEnvelope(browser_event(first.context(), 1)),
        ScriptItem::RawEnvelope(browser_event(first.context(), 2)),
        ScriptItem::Wait(first_gate.clone()),
        ScriptItem::RawEnvelope(browser_event(first.context(), 3)),
    ]]);
    let pending = document.prepare_add(first.clone()).expect("prepare first");
    let authorized = pending.authorize().await.expect("authorize first");
    let establishing = document
        .prepare_establish(authorized)
        .expect("prepare first establishment");
    let ready = establishing
        .establish(&first_source)
        .await
        .expect("establish first");
    document.commit_add(ready).expect("commit first");
    let mut bounded = BoundedDocumentTransportSession::new(
        document,
        ResourceBounds::new(2, 256 * 1024).expect("two-item document bound"),
        PermitPool::new(1).expect("shared delivery permit"),
        policy(),
    )
    .expect("bounded document");

    let mut dispatcher = RecordingDispatcher::default();
    for _ in 0..2 {
        assert_eq!(
            bounded.pump_next(fixture.registry.as_ref()).await,
            Ok(Some(BufferDisposition::Queued))
        );
        assert!(matches!(
            bounded.dispatch_next(fixture.registry.as_ref(), &mut dispatcher),
            Ok(Some(AsyncDeliveryDisposition::Sequence(
                SequenceDisposition::Apply
            )))
        ));
    }

    let second = fixture.request(subscription(0x9b), origin.clone());
    let filler = fixture.request(subscription(0x9c), origin);
    let sibling_source = ScriptedSource::new(vec![
        vec![ScriptItem::RawEnvelope(heartbeat_at(
            second.context(),
            7,
            2,
        ))],
        vec![ScriptItem::RawEnvelope(heartbeat_at(
            filler.context(),
            7,
            1,
        ))],
    ]);
    try_add_membership(&mut bounded, second.clone(), &sibling_source)
        .await
        .expect("commit second membership");
    try_add_membership(&mut bounded, filler.clone(), &sibling_source)
        .await
        .expect("commit filler membership");

    assert_eq!(
        bounded.pump_next(fixture.registry.as_ref()).await,
        Ok(Some(BufferDisposition::Queued))
    );
    assert_eq!(
        bounded.pump_next(fixture.registry.as_ref()).await,
        Ok(Some(BufferDisposition::Queued))
    );
    first_gate.release();
    assert_eq!(
        bounded.pump_next(fixture.registry.as_ref()).await,
        Ok(Some(BufferDisposition::Degraded))
    );
    assert_eq!(bounded.sequence_state(&first), Some(SequenceState::Current));
    assert!(bounded.is_degraded());

    assert!(matches!(
        bounded.dispatch_next(fixture.registry.as_ref(), &mut dispatcher),
        Ok(Some(AsyncDeliveryDisposition::Sequence(
            SequenceDisposition::Degraded(SequenceDegradation::Gap)
        )))
    ));
    assert!(matches!(
        bounded.dispatch_next(fixture.registry.as_ref(), &mut dispatcher),
        Ok(Some(AsyncDeliveryDisposition::Sequence(
            SequenceDisposition::Apply
        )))
    ));

    assert_eq!(
        bounded
            .admit_replay(
                &second,
                vec![
                    heartbeat_at(second.context(), 7, 1),
                    heartbeat_at(second.context(), 7, 2),
                ],
                fixture.registry.as_ref(),
            )
            .expect("second replay admission"),
        BufferDisposition::Queued
    );
    bounded
        .dispatch_next(fixture.registry.as_ref(), &mut dispatcher)
        .expect("second replay dispatch");
    assert_eq!(
        bounded.sequence_state(&second),
        Some(SequenceState::Current)
    );
    assert_eq!(bounded.sequence_state(&first), Some(SequenceState::Current));
    assert!(bounded.is_degraded());

    assert_eq!(
        bounded
            .admit_replay(
                &first,
                vec![browser_event(first.context(), 3)],
                fixture.registry.as_ref(),
            )
            .expect("first replay admission"),
        BufferDisposition::Queued
    );
    bounded
        .dispatch_next(fixture.registry.as_ref(), &mut dispatcher)
        .expect("first exact replay dispatch");
    assert_eq!(bounded.sequence_state(&first), Some(SequenceState::Current));
    assert!(!bounded.is_degraded());
}

#[tokio::test]
async fn authenticated_exact_retirement_discharges_only_that_memberships_pressure_obligation() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let origin = VerifiedOrigin::parse("https://example.test").expect("origin");
    let authorization = fixture.request(subscription(0x9d), origin.clone());
    let mut document = fixture.document(
        origin.clone(),
        DocumentTransportKind::ServerSentEvents,
        0x9d,
        4,
    );
    let source = ScriptedSource::new(vec![vec![
        ScriptItem::RawEnvelope(browser_event(authorization.context(), 1)),
        ScriptItem::RawEnvelope(browser_event(authorization.context(), 2)),
        ScriptItem::Pending,
    ]]);
    let pending = document
        .prepare_add(authorization.clone())
        .expect("prepare exact membership");
    let authorized = pending
        .authorize()
        .await
        .expect("authorize exact membership");
    let establishing = document
        .prepare_establish(authorized)
        .expect("prepare exact establishment");
    let ready = establishing
        .establish(&source)
        .await
        .expect("establish exact membership");
    document.commit_add(ready).expect("commit exact membership");
    let mut bounded = BoundedDocumentTransportSession::new(
        document,
        ResourceBounds::new(1, 256 * 1024).expect("one-item document bound"),
        PermitPool::new(1).expect("shared delivery permit"),
        policy(),
    )
    .expect("bounded document");
    bounded
        .pump_next(fixture.registry.as_ref())
        .await
        .expect("first admission");
    assert_eq!(
        bounded.pump_next(fixture.registry.as_ref()).await,
        Ok(Some(BufferDisposition::Degraded))
    );
    assert!(bounded.is_degraded());
    let mut dispatcher = RecordingDispatcher::default();
    bounded
        .dispatch_next(fixture.registry.as_ref(), &mut dispatcher)
        .expect("retained exact successor dispatch");

    let sibling = fixture.request(subscription(0x9e), origin);
    let sibling_source = ScriptedSource::new(vec![vec![ScriptItem::RawEnvelope(heartbeat_at(
        sibling.context(),
        7,
        2,
    ))]]);
    try_add_membership(&mut bounded, sibling.clone(), &sibling_source)
        .await
        .expect("commit sibling membership");
    bounded
        .pump_next(fixture.registry.as_ref())
        .await
        .expect("sibling gap admission");
    bounded
        .dispatch_next(fixture.registry.as_ref(), &mut dispatcher)
        .expect("sibling gap classification");
    assert_eq!(
        bounded.sequence_state(&sibling),
        Some(SequenceState::Degraded)
    );

    let remove = bounded
        .prepare_remove(&authorization)
        .expect("prepare exact removal");
    let ready = remove.authorize().await.expect("authorize exact removal");
    assert_eq!(bounded.commit_remove(ready), Ok(CloseDisposition::Closed));
    assert_eq!(bounded.sequence_state(&authorization), None);
    assert!(bounded.is_degraded());
    assert_eq!(
        bounded.sequence_state(&sibling),
        Some(SequenceState::Degraded)
    );

    let remove = bounded
        .prepare_remove(&sibling)
        .expect("prepare sibling removal");
    let ready = remove.authorize().await.expect("authorize sibling removal");
    assert_eq!(bounded.commit_remove(ready), Ok(CloseDisposition::Closed));
    assert!(!bounded.is_degraded());
}

#[tokio::test]
async fn unresolved_pressure_cause_storage_is_hard_bounded_across_membership_churn() {
    let fixture = TransportFixture::new(position(7, 0)).await;
    let origin = VerifiedOrigin::parse("https://example.test").expect("origin");
    let document = fixture.document(
        origin.clone(),
        DocumentTransportKind::ServerSentEvents,
        0x9f,
        suprnova_live::async_updates::MAX_DOCUMENT_TRANSPORT_MEMBERSHIPS,
    );
    let mut bounded = BoundedDocumentTransportSession::new(
        document,
        ResourceBounds::new(64, 256 * 1024).expect("document bounds"),
        PermitPool::new(1).expect("shared delivery permit"),
        policy(),
    )
    .expect("bounded document");
    let cause_bound = suprnova_live::async_updates::MAX_DOCUMENT_TRANSPORT_MEMBERSHIPS * 4;

    for index in 0..=cause_bound {
        let mut bytes = [0u8; 16];
        bytes[..8].copy_from_slice(&(index as u64 + 1).to_be_bytes());
        let subscription = SubscriptionId::from_bytes(&bytes).expect("unique subscription");
        let authorization = fixture.request(subscription, origin.clone());
        let source = ScriptedSource::new(vec![vec![
            ScriptItem::RawEnvelope(heartbeat_at(authorization.context(), 7, 1)),
            ScriptItem::Error(suprnova_live::async_updates::AsyncTransportErrorKind::SourceFailed),
        ]]);
        try_add_membership(&mut bounded, authorization, &source)
            .await
            .expect("commit churn membership");
        assert_eq!(
            bounded.pump_next(fixture.registry.as_ref()).await,
            Ok(Some(BufferDisposition::Queued))
        );
        assert!(bounded.pump_next(fixture.registry.as_ref()).await.is_err());
    }

    assert_eq!(bounded.retained_events(), 0);
    assert_eq!(bounded.delivery_lane_count(), 0);
    assert!(bounded.is_degraded());
    assert_eq!(bounded.unresolved_pressure_cause_count(), cause_bound);
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
        Ok(Some(AsyncDeliveryDisposition::Sequence(
            SequenceDisposition::Apply
        )))
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
