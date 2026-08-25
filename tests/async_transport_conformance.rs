//! Shared transport conformance for multiplexed asynchronous sessions.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::task::{Context, Poll, Wake, Waker};

use proptest::prelude::*;
use suprnova_live::async_updates::{
    AsyncCodecLimits, AsyncDispatchError, AsyncEnvelope, AsyncEnvelopeDispatchPort,
    AsyncEventSession, AsyncEventSource, AsyncPayload, AsyncTransportError,
    AsyncTransportErrorKind, AsyncTransportFuture, AuthorizedTransportSubscription,
    CloseDisposition, CompletionReason, DocumentAuthorizationScope, DocumentTransportHandle,
    DocumentTransportKind, DocumentTransportLimits, DocumentTransportSession, Heartbeat,
    MAX_DOCUMENT_TRANSPORT_MEMBERSHIPS, RegisteredRefresh, SequenceDegradation,
    SequenceDisposition, SequenceMachine, SseEncoder, SseMembershipControl, SseResponseContract,
    StreamErrorCode, SubscriptionMode, VerifiedOrigin, WebSocketAuthentication, WebSocketCodec,
    WebSocketControlRecord, WebSocketFrame, WebSocketMembershipControl, WebSocketOriginPolicy,
    decode_async_envelope,
};
use suprnova_live::host::{
    HostScopeFacts, PrincipalFingerprint, SessionFingerprint, TenantFingerprint,
};
use suprnova_live::identity::{ContentDigest, ScopeFingerprint, UnixMillis};

#[path = "support/async_transport.rs"]
mod support;

use support::{
    ControlledSubscribeSource, ScriptItem, ScriptedSource, TransportFixture, WakeGate, position,
    subscription,
};

#[test]
fn task_four_transport_surface_is_present() {
    fn require_source<T: AsyncEventSource + ?Sized>() {}
    fn require_session<T: AsyncEventSession + ?Sized>() {}

    let _ = require_source::<dyn AsyncEventSource>;
    let _ = require_session::<dyn AsyncEventSession>;
    let _future: Option<AsyncTransportFuture<'static, ()>> = None;
    let _close = CloseDisposition::Closed;
    let _document = std::mem::size_of::<DocumentTransportSession>();
    let _sse = std::mem::size_of::<SseEncoder>();
    let _websocket = std::mem::size_of::<WebSocketCodec>();
    let _origin = std::mem::size_of::<VerifiedOrigin>();
}

async fn establish_membership(
    document: &mut DocumentTransportSession,
    source: &dyn AsyncEventSource,
    authorization: AuthorizedTransportSubscription,
) -> Result<(), AsyncTransportError> {
    let pending = document.prepare_add(authorization)?;
    let authorized = pending.authorize().await?;
    let establishing = document.prepare_establish(authorized)?;
    let ready = establishing.establish(source).await?;
    document.commit_add(ready)
}

async fn remove_membership(
    document: &mut DocumentTransportSession,
    authorization: &AuthorizedTransportSubscription,
) -> Result<CloseDisposition, AsyncTransportError> {
    let pending = document.prepare_remove(authorization)?;
    let ready = pending.authorize().await?;
    document.commit_remove(ready)
}

async fn sse_subscribe(
    document: &mut DocumentTransportSession,
    handle: &DocumentTransportHandle,
    origin: &VerifiedOrigin,
    source: &dyn AsyncEventSource,
    authorization: AuthorizedTransportSubscription,
) -> Result<(), AsyncTransportError> {
    let pending = SseMembershipControl::prepare_subscribe(document, handle, origin, authorization)?;
    let authorized = pending.authorize().await?;
    let establishing = document.prepare_establish(authorized)?;
    let ready = establishing.establish(source).await?;
    document.commit_add(ready)
}

async fn sse_unsubscribe(
    document: &mut DocumentTransportSession,
    handle: &DocumentTransportHandle,
    origin: &VerifiedOrigin,
    authorization: &AuthorizedTransportSubscription,
) -> Result<CloseDisposition, AsyncTransportError> {
    let pending =
        SseMembershipControl::prepare_unsubscribe(document, handle, origin, authorization)?;
    let ready = pending.authorize().await?;
    document.commit_remove(ready)
}

async fn websocket_subscribe(
    document: &mut DocumentTransportSession,
    control: &WebSocketControlRecord,
    source: &dyn AsyncEventSource,
    authorization: AuthorizedTransportSubscription,
) -> Result<(), AsyncTransportError> {
    let pending = WebSocketMembershipControl::prepare_subscribe(document, control, authorization)?;
    let authorized = pending.authorize().await?;
    let establishing = document.prepare_establish(authorized)?;
    let ready = establishing.establish(source).await?;
    document.commit_add(ready)
}

async fn websocket_unsubscribe(
    document: &mut DocumentTransportSession,
    control: &WebSocketControlRecord,
    authorization: &AuthorizedTransportSubscription,
) -> Result<CloseDisposition, AsyncTransportError> {
    let pending =
        WebSocketMembershipControl::prepare_unsubscribe(document, control, authorization)?;
    let ready = pending.authorize().await?;
    document.commit_remove(ready)
}

trait LegacyDocumentTestControl {
    fn add<'a>(
        &'a mut self,
        source: &'a dyn AsyncEventSource,
        authorization: AuthorizedTransportSubscription,
    ) -> AsyncTransportFuture<'a, Result<(), AsyncTransportError>>;

    fn remove<'a>(
        &'a mut self,
        authorization: &'a AuthorizedTransportSubscription,
    ) -> AsyncTransportFuture<'a, Result<CloseDisposition, AsyncTransportError>>;
}

impl LegacyDocumentTestControl for DocumentTransportSession {
    fn add<'a>(
        &'a mut self,
        source: &'a dyn AsyncEventSource,
        authorization: AuthorizedTransportSubscription,
    ) -> AsyncTransportFuture<'a, Result<(), AsyncTransportError>> {
        Box::pin(async move {
            let pending = self.prepare_add(authorization)?;
            let authorized = pending.authorize().await?;
            let establishing = self.prepare_establish(authorized)?;
            let ready = establishing.establish(source).await?;
            self.commit_add(ready)
        })
    }

    fn remove<'a>(
        &'a mut self,
        authorization: &'a AuthorizedTransportSubscription,
    ) -> AsyncTransportFuture<'a, Result<CloseDisposition, AsyncTransportError>> {
        Box::pin(async move {
            let pending = self.prepare_remove(authorization)?;
            let ready = pending.authorize().await?;
            self.commit_remove(ready)
        })
    }
}

#[derive(Default)]
struct WakeCounter(AtomicUsize);

impl Wake for WakeCounter {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::AcqRel);
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::AcqRel);
    }
}

impl WakeCounter {
    fn count(&self) -> usize {
        self.0.load(Ordering::Acquire)
    }
}

#[tokio::test]
async fn pending_admission_leaves_document_progress_available_and_stale_commit_cleans_up() {
    let fixture = TransportFixture::new(position(19, 0)).await;
    let origin = VerifiedOrigin::parse("https://app.example.test").expect("origin");
    let source = ScriptedSource::new(vec![vec![ScriptItem::Envelope(
        position(19, 1),
        AsyncPayload::Heartbeat(Heartbeat),
    )]]);
    let mut document = fixture.document(
        origin.clone(),
        DocumentTransportKind::ServerSentEvents,
        0xb1,
        3,
    );
    let active = fixture.request(subscription(0xb1), origin.clone());
    establish_membership(&mut document, &source, active)
        .await
        .expect("active sibling");

    let controlled = ControlledSubscribeSource::new();
    let pending = document
        .prepare_add(fixture.request(subscription(0xb2), origin.clone()))
        .expect("pending admission snapshot");
    let authorized = pending.authorize().await.expect("fresh add authority");
    let source_establishment = document
        .prepare_establish(authorized)
        .expect("pre-source document gate");
    let mut establishing = Box::pin(source_establishment.establish(&controlled));
    let waker = Waker::noop();
    let mut task = Context::from_waker(waker);
    assert!(establishing.as_mut().poll(&mut task).is_pending());
    assert!(controlled.observed());

    let delivered = document
        .next()
        .await
        .expect("healthy sibling progress")
        .expect("heartbeat");
    assert_eq!(delivered.subscription(), &subscription(0xb1));
    let active_removal = fixture.request(subscription(0xb1), origin);
    let removal = document
        .prepare_remove(&active_removal)
        .expect("concurrent control snapshot");
    let ready_removal = removal
        .authorize()
        .await
        .expect("current removal authority");
    document
        .commit_remove(ready_removal)
        .expect("concurrent removal commits");

    controlled.release();
    let ready = establishing.await.expect("opened logical session");
    let error = document
        .commit_add(ready)
        .expect_err("document mutation invalidates pending ready admission");
    assert_eq!(error.kind(), AsyncTransportErrorKind::StaleControl);
    assert_eq!(controlled.close_count(), 1);
    assert_eq!(controlled.drop_count(), 1);
}

#[tokio::test]
async fn pending_authority_leaves_document_delivery_and_other_controls_available() {
    let fixture = TransportFixture::new(position(19, 10)).await;
    let origin = VerifiedOrigin::parse("https://app.example.test").expect("origin");
    let source = ScriptedSource::new(vec![vec![ScriptItem::Envelope(
        position(19, 11),
        AsyncPayload::Heartbeat(Heartbeat),
    )]]);
    let mut document = fixture.document(
        origin.clone(),
        DocumentTransportKind::ServerSentEvents,
        0xbc,
        3,
    );
    let active = fixture.request(subscription(0xbc), origin.clone());
    establish_membership(&mut document, &source, active)
        .await
        .expect("active sibling");

    let gate = fixture.registry.pause_authority_on_call(3);
    let controlled = ControlledSubscribeSource::new();
    let pending = document
        .prepare_add(fixture.request(subscription(0xbd), origin.clone()))
        .expect("pending authority snapshot");
    let mut authorization = Box::pin(pending.authorize());
    let waker = Waker::noop();
    let mut task = Context::from_waker(waker);
    assert!(authorization.as_mut().poll(&mut task).is_pending());
    assert!(gate.observed());
    assert!(gate.waiter_registered());

    let delivered = document
        .next()
        .await
        .expect("healthy sibling progresses during authority wait")
        .expect("heartbeat");
    assert_eq!(delivered.subscription(), &subscription(0xbc));

    let removal_authorization = fixture.request(subscription(0xbc), origin);
    let removal = document
        .prepare_remove(&removal_authorization)
        .expect("independent control snapshot");
    let ready_removal = removal
        .authorize()
        .await
        .expect("independent control authority");
    document
        .commit_remove(ready_removal)
        .expect("independent control commits");

    gate.release();
    let authorized = authorization.await.expect("held authority resumes");
    let error = document
        .prepare_establish(authorized)
        .expect_err("document mutation invalidates held authority result");
    assert_eq!(error.kind(), AsyncTransportErrorKind::StaleControl);
    assert!(
        !controlled.observed(),
        "stale control must fail before source work"
    );
}

#[tokio::test]
async fn ready_control_is_bound_to_the_exact_document_owner_not_a_matching_tuple() {
    let fixture = TransportFixture::new(position(19, 20)).await;
    let origin = VerifiedOrigin::parse("https://app.example.test").expect("origin");
    let source = ControlledSubscribeSource::new();
    source.release();
    let first = fixture.document(
        origin.clone(),
        DocumentTransportKind::ServerSentEvents,
        0xbe,
        1,
    );
    let mut matching_but_distinct = fixture.document(
        origin.clone(),
        DocumentTransportKind::ServerSentEvents,
        0xbe,
        1,
    );
    let pending = first
        .prepare_add(fixture.request(subscription(0xbe), origin))
        .expect("first document snapshot");
    let authorized = pending.authorize().await.expect("fresh authority");
    let establishing = first
        .prepare_establish(authorized)
        .expect("first document pre-source gate");
    let ready = establishing
        .establish(&source)
        .await
        .expect("opened session");

    let error = matching_but_distinct
        .commit_add(ready)
        .expect_err("a matching public tuple is not the exact document owner");
    assert_eq!(error.kind(), AsyncTransportErrorKind::StaleControl);
    assert_eq!(source.close_count(), 1);
    assert_eq!(source.drop_count(), 1);
}

#[tokio::test]
async fn canceled_post_subscribe_authorization_releases_the_opened_session_owner() {
    let fixture = TransportFixture::new(position(20, 0)).await;
    let origin = VerifiedOrigin::parse("https://app.example.test").expect("origin");
    let gate = fixture.registry.pause_authority_on_call(2);
    let source = ControlledSubscribeSource::new();
    source.release();
    let document = fixture.document(
        origin.clone(),
        DocumentTransportKind::ServerSentEvents,
        0xb2,
        2,
    );
    let pending = document
        .prepare_add(fixture.request(subscription(0xb3), origin))
        .expect("pending admission snapshot");
    let authorized = pending.authorize().await.expect("fresh add authority");
    let source_establishment = document
        .prepare_establish(authorized)
        .expect("pre-source document gate");
    let mut establishing = Box::pin(source_establishment.establish(&source));
    let waker = Waker::noop();
    let mut task = Context::from_waker(waker);
    assert!(establishing.as_mut().poll(&mut task).is_pending());
    assert!(gate.observed());
    assert!(gate.waiter_registered());
    assert_eq!(fixture.registry.authority_call_count(), 2);

    drop(establishing);
    assert_eq!(source.close_count(), 1);
    assert_eq!(source.drop_count(), 1);
}

#[tokio::test]
async fn document_bounds_owned_pending_controls_without_borrowing_document_state() {
    let fixture = TransportFixture::new(position(20, 10)).await;
    let origin = VerifiedOrigin::parse("https://app.example.test").expect("origin");
    let document = fixture.document(
        origin.clone(),
        DocumentTransportKind::ServerSentEvents,
        0xbb,
        1,
    );
    let mut pending = Vec::new();
    for marker in 0..MAX_DOCUMENT_TRANSPORT_MEMBERSHIPS {
        pending.push(
            document
                .prepare_add(fixture.request(subscription(marker as u8), origin.clone()))
                .expect("bounded pending control"),
        );
    }
    assert_eq!(
        document
            .prepare_add(fixture.request(subscription(0xfe), origin.clone()))
            .expect_err("first control beyond the hard in-flight bound")
            .kind(),
        AsyncTransportErrorKind::MembershipLimit
    );
    pending.pop();
    document
        .prepare_remove(&fixture.request(subscription(0xfd), origin))
        .expect("add and remove controls share the released bounded permit");
}

#[tokio::test]
async fn retiring_membership_fences_exact_id_binding_and_capacity_until_woken_cleanup() {
    let baseline = position(21, 0);
    let first = TransportFixture::new_with_signing_key(baseline, "retiring-old", 0x81).await;
    let second = TransportFixture::new_with_signing_key(baseline, "retiring-new", 0x82).await;
    assert_eq!(
        first.authorized.verified().claims(),
        second.authorized.verified().claims()
    );
    assert_ne!(first.authorized.binding(), second.authorized.binding());

    let origin = VerifiedOrigin::parse("https://app.example.test").expect("origin");
    let close_gate = Arc::new(WakeGate::new());
    let source = ScriptedSource::new(vec![vec![ScriptItem::Pending]])
        .with_controlled_close(close_gate.clone());
    let mut document = first.document(
        origin.clone(),
        DocumentTransportKind::ServerSentEvents,
        0xb3,
        1,
    );
    let installed = first.request(subscription(0xb4), origin.clone());
    establish_membership(&mut document, &source, installed)
        .await
        .expect("installed membership");
    remove_membership(
        &mut document,
        &first.request(subscription(0xb4), origin.clone()),
    )
    .await
    .expect("logical removal");
    assert_eq!(document.membership_count(), 0);
    assert_eq!(document.retiring_count(), 1);
    assert!(close_gate.waiter_registered());

    let duplicate_pending = document
        .prepare_add(first.request(subscription(0xb4), origin.clone()))
        .expect("same binding preparation");
    let duplicate_authorized = duplicate_pending
        .authorize()
        .await
        .expect("same binding authority");
    assert_eq!(
        document
            .prepare_establish(duplicate_authorized)
            .expect_err("retirement fence rejects exact duplicate")
            .kind(),
        AsyncTransportErrorKind::DuplicateMembership
    );

    let overlap_pending = document
        .prepare_add(second.request(subscription(0xb4), origin.clone()))
        .expect("overlap binding preparation");
    let overlap_authorized = overlap_pending
        .authorize()
        .await
        .expect("overlap binding authority");
    assert_eq!(
        document
            .prepare_establish(overlap_authorized)
            .expect_err("retirement fence rejects another exact signed wire")
            .kind(),
        AsyncTransportErrorKind::DescriptorMismatch
    );

    let capacity_pending = document
        .prepare_add(first.request(subscription(0xb5), origin.clone()))
        .expect("capacity preparation");
    let capacity_authorized = capacity_pending
        .authorize()
        .await
        .expect("capacity authority");
    assert_eq!(
        document
            .prepare_establish(capacity_authorized)
            .expect_err("retiring membership consumes capacity")
            .kind(),
        AsyncTransportErrorKind::MembershipLimit
    );

    close_gate.release();
    assert!(document.next().await.expect("cleanup poll").is_none());
    assert_eq!(document.retiring_count(), 0);
    assert_eq!(source.drop_count(), 1);
    let replacement_source = ScriptedSource::new(vec![vec![ScriptItem::Pending]]);
    establish_membership(
        &mut document,
        &replacement_source,
        second.request(subscription(0xb4), origin),
    )
    .await
    .expect("replacement is admitted only after old cleanup leaves the fence");
}

#[tokio::test]
async fn controlled_read_and_close_pending_states_register_and_use_exact_wakers() {
    let fixture = TransportFixture::new(position(22, 0)).await;
    let origin = VerifiedOrigin::parse("https://app.example.test").expect("origin");
    let read_gate = Arc::new(WakeGate::new());
    let close_gate = Arc::new(WakeGate::new());
    let source = ScriptedSource::new(vec![vec![
        ScriptItem::Wait(read_gate.clone()),
        ScriptItem::Envelope(position(22, 1), AsyncPayload::Heartbeat(Heartbeat)),
    ]])
    .with_controlled_close(close_gate.clone());
    let mut document = fixture.document(
        origin.clone(),
        DocumentTransportKind::ServerSentEvents,
        0xb4,
        1,
    );
    establish_membership(
        &mut document,
        &source,
        fixture.request(subscription(0xb6), origin),
    )
    .await
    .expect("membership");

    let read_wakes = Arc::new(WakeCounter::default());
    let read_waker = Waker::from(read_wakes.clone());
    let mut read_task = Context::from_waker(&read_waker);
    let mut next = Box::pin(document.next());
    assert!(next.as_mut().poll(&mut read_task).is_pending());
    assert!(read_gate.waiter_registered());
    read_gate.release();
    assert_eq!(read_wakes.count(), 1);
    assert!(next.await.expect("woken read").is_some());

    let close_wakes = Arc::new(WakeCounter::default());
    let close_waker = Waker::from(close_wakes.clone());
    let mut close_task = Context::from_waker(&close_waker);
    let mut close = Box::pin(document.close());
    assert!(close.as_mut().poll(&mut close_task).is_pending());
    assert!(close_gate.waiter_registered());
    close_gate.release();
    assert_eq!(close_wakes.count(), 1);
    assert_eq!(close.await.expect("woken close"), CloseDisposition::Closed);
}

#[tokio::test]
async fn websocket_decode_is_state_blind_and_unauthorized_removal_never_gets_membership_oracle() {
    let fixture = TransportFixture::new(position(23, 0)).await;
    let overlap = TransportFixture::new_with_signing_key(position(23, 0), "oracle-new", 0x91).await;
    let origin = VerifiedOrigin::parse("https://app.example.test").expect("origin");
    let source = ScriptedSource::new(vec![vec![ScriptItem::Pending]]);
    let mut document = fixture.document(origin.clone(), DocumentTransportKind::WebSocket, 0xb5, 2);
    establish_membership(
        &mut document,
        &source,
        fixture.request(subscription(0xb7), origin.clone()),
    )
    .await
    .expect("existing membership");

    let codec = WebSocketCodec::v1();
    let unknown_control = WebSocketControlRecord::Unsubscribe(subscription(0xb8));
    let unknown_bytes = codec
        .encode_control(&unknown_control)
        .expect("control bytes");
    assert_eq!(
        codec
            .decode_control(WebSocketFrame::Text {
                payload: &unknown_bytes,
                final_fragment: true,
            })
            .expect("syntactically valid unknown membership remains opaque"),
        unknown_control
    );

    fixture.registry.deny_unsubscribe();
    overlap.registry.deny_unsubscribe();
    let denied = [
        fixture.request(subscription(0xb7), origin.clone()),
        fixture.request(subscription(0xb8), origin.clone()),
        overlap.request(subscription(0xb7), origin),
    ];
    for authorization in &denied {
        let control = WebSocketControlRecord::Unsubscribe(authorization.subscription().clone());
        let pending =
            WebSocketMembershipControl::prepare_unsubscribe(&document, &control, authorization)
                .expect("state-blind WebSocket removal preparation");
        assert_eq!(
            pending
                .authorize()
                .await
                .expect_err("denied authority precedes membership classification")
                .kind(),
            AsyncTransportErrorKind::AuthorizationLost
        );
    }
    assert_eq!(document.membership_count(), 1);
    assert_eq!(fixture.registry.authority_call_count(), 4);
    assert_eq!(overlap.registry.authority_call_count(), 1);
}

#[tokio::test]
async fn distinct_components_share_physical_scope_and_authority_observes_every_exact_fact() {
    let baseline = position(24, 0);
    let first = TransportFixture::new_with_component_name(baseline, "tests.async.alpha").await;
    let second = TransportFixture::new_with_component_name(baseline, "tests.async.beta").await;
    assert_ne!(first.component_name, second.component_name);
    assert_eq!(first.document_scope, second.document_scope);

    let origin = VerifiedOrigin::parse("https://app.example.test").expect("origin");
    let handle = DocumentTransportHandle::from_bytes(&[0xb6; 16]).expect("handle");
    let source = ScriptedSource::new(vec![vec![ScriptItem::Pending], vec![ScriptItem::Pending]]);
    let mut document = DocumentTransportSession::new(
        origin.clone(),
        DocumentTransportKind::WebSocket,
        handle.clone(),
        DocumentTransportLimits::new(2).expect("limits"),
        first.document_scope.clone(),
    );
    let first_request = first.request(subscription(0xb9), origin.clone());
    let first_binding = first_request.binding().clone();
    let second_request = second.request(subscription(0xba), origin.clone());
    let second_binding = second_request.binding().clone();
    establish_membership(&mut document, &source, first_request)
        .await
        .expect("first component");
    establish_membership(&mut document, &source, second_request)
        .await
        .expect("second component");

    for (fixture, binding, identity) in [
        (&first, first_binding, subscription(0xb9)),
        (&second, second_binding, subscription(0xba)),
    ] {
        let observations = fixture.registry.authority_observations();
        assert_eq!(observations.len(), 2);
        for observation in observations {
            assert_eq!(
                observation.operation,
                suprnova_live::async_updates::TransportMembershipOperation::Subscribe
            );
            assert_eq!(observation.origin, origin);
            assert_eq!(observation.kind, DocumentTransportKind::WebSocket);
            assert_eq!(observation.handle, handle);
            assert_eq!(observation.document_scope, fixture.document_scope);
            assert_eq!(
                observation.component_memo,
                fixture
                    .authorized
                    .verified()
                    .claims()
                    .authorization_memo()
                    .clone()
            );
            assert_eq!(observation.binding, binding);
            assert_eq!(observation.subscription, identity);
        }
    }
}

fn host_scope(marker: u8) -> HostScopeFacts {
    host_scope_parts(
        marker,
        marker.wrapping_add(1),
        marker.wrapping_add(2),
        marker.wrapping_add(3),
    )
}

fn host_scope_parts(scope: u8, session: u8, principal: u8, tenant: u8) -> HostScopeFacts {
    HostScopeFacts::new(
        ScopeFingerprint::from_bytes(&[scope; 32]).expect("aggregate scope"),
        Some(SessionFingerprint::from_bytes(&[session; 32]).expect("session")),
        Some(PrincipalFingerprint::from_bytes(&[principal; 32]).expect("principal")),
        Some(TenantFingerprint::from_bytes(&[tenant; 32]).expect("tenant")),
    )
}

fn transport_policy(marker: u8) -> ContentDigest {
    ContentDigest::from_bytes(&[marker; 32]).expect("transport policy")
}

#[test]
fn document_authorization_scope_is_canonical_redacted_and_identity_exact() {
    let facts = host_scope(0x21);
    let policy = transport_policy(0x31);
    let scope = DocumentAuthorizationScope::derive(&facts, &policy).expect("document scope");

    assert_eq!(
        scope,
        DocumentAuthorizationScope::derive(&facts, &policy).expect("same scope")
    );
    assert_ne!(
        scope,
        DocumentAuthorizationScope::derive(&host_scope_parts(0x22, 0x22, 0x23, 0x24), &policy)
            .expect("different aggregate scope")
    );
    assert_ne!(
        scope,
        DocumentAuthorizationScope::derive(&host_scope_parts(0x21, 0x26, 0x23, 0x24), &policy)
            .expect("different session")
    );
    assert_ne!(
        scope,
        DocumentAuthorizationScope::derive(&host_scope_parts(0x21, 0x22, 0x26, 0x24), &policy)
            .expect("different principal")
    );
    assert_ne!(
        scope,
        DocumentAuthorizationScope::derive(&host_scope_parts(0x21, 0x22, 0x23, 0x26), &policy)
            .expect("different tenant")
    );
    assert_ne!(
        scope,
        DocumentAuthorizationScope::derive(&facts, &transport_policy(0x32))
            .expect("different transport policy")
    );
    assert_eq!(
        format!("{scope:?}"),
        "<DocumentAuthorizationScope:redacted>"
    );
}

#[tokio::test]
async fn document_transport_multiplexes_exact_logical_memberships_and_closes_once() {
    let baseline = position(4, 40);
    let fixture = TransportFixture::new(baseline).await;
    let origin = VerifiedOrigin::parse("https://app.example.test").expect("origin");
    let handle = DocumentTransportHandle::from_bytes(&[0x41; 16]).expect("handle");
    let limits = DocumentTransportLimits::new(2).expect("limits");
    let mut document = DocumentTransportSession::new(
        origin.clone(),
        DocumentTransportKind::ServerSentEvents,
        handle,
        limits,
        fixture.document_scope.clone(),
    );
    let source = ScriptedSource::new(vec![
        vec![
            ScriptItem::Envelope(position(4, 41), AsyncPayload::Heartbeat(Heartbeat)),
            ScriptItem::Envelope(
                position(4, 42),
                AsyncPayload::Complete(CompletionReason::StreamCompleted),
            ),
            ScriptItem::End,
        ],
        vec![
            ScriptItem::Envelope(position(4, 41), AsyncPayload::Refresh(RegisteredRefresh)),
            ScriptItem::Envelope(
                position(4, 42),
                AsyncPayload::Error(StreamErrorCode::ReplayUnavailable),
            ),
            ScriptItem::End,
        ],
    ]);
    let first = fixture.request(subscription(1), origin.clone());
    let second = fixture.request(subscription(2), origin);

    document
        .add(&source, first)
        .await
        .expect("first membership");
    document
        .add(&source, second)
        .await
        .expect("second membership");
    assert_eq!(document.membership_count(), 2);

    let first_event = document
        .next()
        .await
        .expect("first delivery")
        .expect("event");
    let second_event = document
        .next()
        .await
        .expect("second delivery")
        .expect("event");
    assert_eq!(first_event.subscription(), &subscription(1));
    assert_eq!(second_event.subscription(), &subscription(2));
    assert_eq!(first_event.position(), position(4, 41));
    assert_eq!(second_event.position(), position(4, 41));

    let completion = document
        .next()
        .await
        .expect("completion delivery")
        .expect("completion envelope");
    let stream_error = document
        .next()
        .await
        .expect("typed error delivery")
        .expect("typed error envelope");
    assert!(matches!(
        completion.payload(),
        AsyncPayload::Complete(CompletionReason::StreamCompleted)
    ));
    assert!(matches!(
        stream_error.payload(),
        AsyncPayload::Error(StreamErrorCode::ReplayUnavailable)
    ));
    assert!(document.next().await.expect("logical completion").is_none());
    assert_eq!(document.membership_count(), 0);

    assert_eq!(
        document.close().await.expect("close"),
        CloseDisposition::Closed
    );
    assert_eq!(
        document.close().await.expect("second close"),
        CloseDisposition::AlreadyClosed
    );
    assert_eq!(source.close_count(), 2);
}

#[tokio::test]
async fn membership_rejects_duplicate_limit_origin_descriptor_and_baseline_mismatch() {
    let baseline = position(7, 10);
    let fixture = TransportFixture::new(baseline).await;
    let origin = VerifiedOrigin::parse("https://app.example.test").expect("origin");
    let foreign_origin = VerifiedOrigin::parse("https://other.example.test").expect("origin");
    let source = ScriptedSource::new(vec![vec![ScriptItem::End], vec![ScriptItem::End]]);
    let mut document = DocumentTransportSession::new(
        origin.clone(),
        DocumentTransportKind::ServerSentEvents,
        DocumentTransportHandle::from_bytes(&[0x42; 16]).expect("handle"),
        DocumentTransportLimits::new(1).expect("limits"),
        fixture.document_scope.clone(),
    );

    document
        .add(&source, fixture.request(subscription(3), origin.clone()))
        .await
        .expect("first membership");
    let duplicate = document
        .add(&source, fixture.request(subscription(3), origin.clone()))
        .await
        .expect_err("duplicate membership");
    assert_eq!(
        duplicate.kind(),
        AsyncTransportErrorKind::DuplicateMembership
    );
    let limit = document
        .add(&source, fixture.request(subscription(4), origin.clone()))
        .await
        .expect_err("membership limit");
    assert_eq!(limit.kind(), AsyncTransportErrorKind::MembershipLimit);
    let cross_origin = document
        .add(&source, fixture.request(subscription(5), foreign_origin))
        .await
        .expect_err("cross-origin membership");
    assert_eq!(cross_origin.kind(), AsyncTransportErrorKind::OriginMismatch);

    let mismatched_source =
        ScriptedSource::new(vec![vec![ScriptItem::End]]).with_baseline_override(position(7, 9));
    let mut other_document = DocumentTransportSession::new(
        origin.clone(),
        DocumentTransportKind::ServerSentEvents,
        DocumentTransportHandle::from_bytes(&[0x43; 16]).expect("handle"),
        DocumentTransportLimits::new(1).expect("limits"),
        fixture.document_scope.clone(),
    );
    let mismatch = other_document
        .add(&mismatched_source, fixture.request(subscription(6), origin))
        .await
        .expect_err("baseline mismatch");
    assert_eq!(mismatch.kind(), AsyncTransportErrorKind::BaselineMismatch);
    assert_eq!(mismatched_source.close_count(), 1);

    assert!(DocumentTransportLimits::new(0).is_err());
    assert!(DocumentTransportLimits::new(MAX_DOCUMENT_TRANSPORT_MEMBERSHIPS + 1).is_err());
}

#[tokio::test]
async fn document_transport_rejects_cross_scope_and_misrouted_source_envelopes() {
    let baseline = position(7, 20);
    let fixture = TransportFixture::new(baseline).await;
    let foreign = TransportFixture::new_in_scope(baseline, 0xa1).await;
    let origin = VerifiedOrigin::parse("https://app.example.test").expect("origin");
    let expected = fixture.request(subscription(21), origin.clone());
    let foreign_authorization = foreign.request(subscription(22), origin.clone());
    let foreign_envelope = AsyncEnvelope::new(
        foreign_authorization.context(),
        position(7, 21),
        AsyncPayload::Heartbeat(Heartbeat),
    )
    .expect("foreign envelope");
    let source = ScriptedSource::new(vec![
        vec![ScriptItem::RawEnvelope(foreign_envelope)],
        vec![ScriptItem::Pending],
    ]);
    let mut document = DocumentTransportSession::new(
        origin,
        DocumentTransportKind::ServerSentEvents,
        DocumentTransportHandle::from_bytes(&[0x52; 16]).expect("handle"),
        DocumentTransportLimits::new(2).expect("limits"),
        fixture.document_scope.clone(),
    );
    document
        .add(&source, expected)
        .await
        .expect("first membership");

    let rejected_scope = document
        .add(&source, foreign_authorization)
        .await
        .expect_err("authorization scopes cannot share a transport");
    assert_eq!(
        rejected_scope.kind(),
        AsyncTransportErrorKind::AuthorizationScopeMismatch
    );
    let misrouted = document
        .next()
        .await
        .expect_err("logical source cannot route another subscription");
    assert_eq!(misrouted.kind(), AsyncTransportErrorKind::RoutingMismatch);
    assert_eq!(document.membership_count(), 0);
    assert_eq!(source.close_count(), 1);
}

#[tokio::test]
async fn physical_document_scope_allows_heterogeneous_components_but_rejects_identity_drift() {
    let baseline = position(7, 30);
    let first = TransportFixture::new(baseline).await;
    let revised_component = TransportFixture::new_with_contract_revision(baseline).await;
    let foreign_identity = TransportFixture::new_in_scope(baseline, 0xa7).await;
    let origin = VerifiedOrigin::parse("https://app.example.test").expect("origin");
    let source = ScriptedSource::new(vec![
        vec![ScriptItem::Pending],
        vec![ScriptItem::Pending],
        vec![ScriptItem::Pending],
    ]);
    let mut document = first.document(
        origin.clone(),
        DocumentTransportKind::ServerSentEvents,
        0x5a,
        3,
    );

    document
        .add(&source, first.request(subscription(31), origin.clone()))
        .await
        .expect("first component membership");
    document
        .add(
            &source,
            revised_component.request(subscription(32), origin.clone()),
        )
        .await
        .expect("different component contract shares the physical scope");
    assert_eq!(document.membership_count(), 2);

    assert_eq!(
        document
            .add(
                &source,
                foreign_identity.request(subscription(33), origin.clone()),
            )
            .await
            .expect_err("different principal/session/tenant scope")
            .kind(),
        AsyncTransportErrorKind::AuthorizationScopeMismatch
    );

    assert_eq!(
        first
            .cross_component_request(&revised_component, subscription(34), origin)
            .expect_err("one component descriptor cannot bind another component registry")
            .kind(),
        AsyncTransportErrorKind::AuthorizationLost
    );
}

#[tokio::test]
async fn exact_descriptor_binding_controls_duplicate_remove_and_rotation_overlap_membership() {
    let baseline = position(7, 40);
    let first = TransportFixture::new_with_signing_key(baseline, "transport-old", 0x61).await;
    let second = TransportFixture::new_with_signing_key(baseline, "transport-current", 0x62).await;
    assert_eq!(
        first.authorized.verified().claims(),
        second.authorized.verified().claims()
    );
    assert_ne!(first.authorized.binding(), second.authorized.binding());

    let origin = VerifiedOrigin::parse("https://app.example.test").expect("origin");
    let source = ScriptedSource::new(vec![vec![ScriptItem::Pending], vec![ScriptItem::Pending]]);
    let mut document = first.document(
        origin.clone(),
        DocumentTransportKind::ServerSentEvents,
        0x5b,
        2,
    );
    let first_request = first.request(subscription(35), origin.clone());
    document
        .add(&source, first_request)
        .await
        .expect("old-key membership");

    let second_same_id = second.request(subscription(35), origin.clone());
    assert_eq!(
        document
            .add(&source, second_same_id)
            .await
            .expect_err("same claims under another signature cannot replace membership")
            .kind(),
        AsyncTransportErrorKind::DescriptorMismatch
    );
    assert_eq!(
        document
            .remove(&second.request(subscription(35), origin.clone()))
            .await
            .expect_err("another descriptor binding cannot remove membership")
            .kind(),
        AsyncTransportErrorKind::DescriptorMismatch
    );
    document
        .remove(&first.request(subscription(35), origin.clone()))
        .await
        .expect("exact descriptor removes membership");
    document
        .add(&source, second.request(subscription(36), origin))
        .await
        .expect("new signing key establishes a separate membership");
}

#[tokio::test]
async fn retained_duplicate_request_reauthorizes_before_membership_classification() {
    let fixture = TransportFixture::new(position(7, 50)).await;
    let origin = VerifiedOrigin::parse("https://app.example.test").expect("origin");
    let source = ScriptedSource::new(vec![vec![ScriptItem::Pending]]);
    let mut document = fixture.document(
        origin.clone(),
        DocumentTransportKind::ServerSentEvents,
        0x5f,
        1,
    );
    let first = fixture.request(subscription(37), origin.clone());
    let retained_duplicate = fixture.request(subscription(37), origin);
    document
        .add(&source, first)
        .await
        .expect("initial membership");
    fixture.registry.revoke();

    assert_eq!(
        document
            .add(&source, retained_duplicate)
            .await
            .expect_err("every external add reauthorizes before duplicate classification")
            .kind(),
        AsyncTransportErrorKind::AuthorizationLost
    );
}

#[tokio::test]
async fn transport_reports_authorization_loss_and_unknown_removal_without_leaking_scope() {
    let baseline = position(8, 0);
    let fixture = TransportFixture::new(baseline).await;
    let origin = VerifiedOrigin::parse("https://app.example.test").expect("origin");
    let source = ScriptedSource::new(vec![vec![ScriptItem::Error(
        AsyncTransportErrorKind::AuthorizationLost,
    )]]);
    let mut document = DocumentTransportSession::new(
        origin.clone(),
        DocumentTransportKind::ServerSentEvents,
        DocumentTransportHandle::from_bytes(&[0x44; 16]).expect("handle"),
        DocumentTransportLimits::new(2).expect("limits"),
        fixture.document_scope.clone(),
    );
    document
        .add(&source, fixture.request(subscription(7), origin.clone()))
        .await
        .expect("membership");

    let error = document.next().await.expect_err("authorization loss");
    assert_eq!(error.kind(), AsyncTransportErrorKind::AuthorizationLost);
    assert_eq!(
        format!("{error:?}"),
        "AsyncTransportError { kind: AuthorizationLost }"
    );
    assert_eq!(document.membership_count(), 0);
    assert_eq!(source.close_count(), 1);

    let unknown = document
        .remove(&fixture.request(subscription(8), origin))
        .await
        .expect_err("unknown membership");
    assert_eq!(unknown.kind(), AsyncTransportErrorKind::UnknownMembership);

    fixture.registry.revoke();
    let revoked = fixture
        .request_at(
            subscription(8),
            VerifiedOrigin::parse("https://app.example.test").expect("origin"),
            UnixMillis::new(1_200),
        )
        .expect_err("revoked current membership");
    assert_eq!(revoked.kind(), AsyncTransportErrorKind::AuthorizationLost);
}

#[tokio::test]
async fn pending_slow_membership_cannot_stall_another_ready_logical_session() {
    let fixture = TransportFixture::new(position(9, 0)).await;
    let origin = VerifiedOrigin::parse("https://app.example.test").expect("origin");
    let source = ScriptedSource::new(vec![
        vec![ScriptItem::Pending],
        vec![ScriptItem::Envelope(
            position(9, 1),
            AsyncPayload::Heartbeat(Heartbeat),
        )],
    ]);
    let mut document = DocumentTransportSession::new(
        origin.clone(),
        DocumentTransportKind::ServerSentEvents,
        DocumentTransportHandle::from_bytes(&[0x45; 16]).expect("handle"),
        DocumentTransportLimits::new(2).expect("limits"),
        fixture.document_scope.clone(),
    );
    document
        .add(&source, fixture.request(subscription(9), origin.clone()))
        .await
        .expect("slow membership");
    document
        .add(&source, fixture.request(subscription(10), origin))
        .await
        .expect("ready membership");

    let event = document.next().await.expect("delivery").expect("event");
    assert_eq!(event.subscription(), &subscription(10));
    assert_eq!(document.membership_count(), 2);
    document.close().await.expect("close all");
}

#[tokio::test]
async fn cancelled_document_close_retains_cleanup_authority_for_controlled_shutdown() {
    let fixture = TransportFixture::new(position(9, 10)).await;
    let origin = VerifiedOrigin::parse("https://app.example.test").expect("origin");
    let source = ScriptedSource::new(vec![vec![ScriptItem::Pending]]).with_pending_first_close();
    let mut document = DocumentTransportSession::new(
        origin.clone(),
        DocumentTransportKind::ServerSentEvents,
        DocumentTransportHandle::from_bytes(&[0x55; 16]).expect("handle"),
        DocumentTransportLimits::new(1).expect("limits"),
        fixture.document_scope.clone(),
    );
    document
        .add(&source, fixture.request(subscription(25), origin))
        .await
        .expect("membership");

    let mut close = Box::pin(document.close());
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    assert!(matches!(close.as_mut().poll(&mut context), Poll::Pending));
    drop(close);
    assert_eq!(document.membership_count(), 0);
    assert_eq!(document.retiring_count(), 1);
    assert_eq!(source.close_count(), 0);
    assert_eq!(
        document
            .next()
            .await
            .expect_err("closing transport cannot resume delivery")
            .kind(),
        AsyncTransportErrorKind::Closed
    );

    assert_eq!(
        document.close().await.expect("resumed shutdown"),
        CloseDisposition::Closed
    );
    assert_eq!(source.close_count(), 1);
    assert_eq!(document.membership_count(), 0);
}

#[tokio::test]
async fn terminal_completion_detaches_before_delivery_and_pending_cleanup_cannot_stall_a_sibling() {
    let fixture = TransportFixture::new(position(9, 20)).await;
    let origin = VerifiedOrigin::parse("https://app.example.test").expect("origin");
    let completing = ScriptedSource::new(vec![vec![
        ScriptItem::Envelope(
            position(9, 21),
            AsyncPayload::Complete(CompletionReason::StreamCompleted),
        ),
        ScriptItem::Envelope(
            position(9, 22),
            AsyncPayload::Error(StreamErrorCode::ReplayUnavailable),
        ),
    ]])
    .with_permanently_pending_close();
    let healthy = ScriptedSource::new(vec![vec![ScriptItem::Envelope(
        position(9, 21),
        AsyncPayload::Heartbeat(Heartbeat),
    )]]);
    let mut document = fixture.document(
        origin.clone(),
        DocumentTransportKind::ServerSentEvents,
        0x58,
        2,
    );
    document
        .add(
            &completing,
            fixture.request(subscription(27), origin.clone()),
        )
        .await
        .expect("terminal membership");
    document
        .add(&healthy, fixture.request(subscription(28), origin))
        .await
        .expect("healthy membership");

    let terminal = document
        .next()
        .await
        .expect("terminal delivery")
        .expect("terminal envelope");
    assert!(matches!(terminal.payload(), AsyncPayload::Complete(_)));
    assert_eq!(document.membership_count(), 1);
    assert_eq!(document.retiring_count(), 1);

    let sibling = document
        .next()
        .await
        .expect("healthy sibling delivery")
        .expect("healthy sibling envelope");
    assert_eq!(sibling.subscription(), &subscription(28));
    assert!(matches!(sibling.payload(), AsyncPayload::Heartbeat(_)));
    assert_eq!(completing.close_poll_count(), 2);
    assert_eq!(document.membership_count(), 1);
}

#[tokio::test]
async fn permanent_close_error_is_observable_without_monopolizing_healthy_delivery() {
    let fixture =
        TransportFixture::new_with_signing_key(position(9, 30), "failing-old", 0xa1).await;
    let overlap =
        TransportFixture::new_with_signing_key(position(9, 30), "failing-new", 0xa2).await;
    let origin = VerifiedOrigin::parse("https://app.example.test").expect("origin");
    let failing = ScriptedSource::new(vec![vec![ScriptItem::Envelope(
        position(9, 31),
        AsyncPayload::Complete(CompletionReason::StreamCompleted),
    )]])
    .with_close_error_attempts(usize::MAX, AsyncTransportErrorKind::SourceFailed);
    let healthy = ScriptedSource::new(vec![vec![ScriptItem::Envelope(
        position(9, 31),
        AsyncPayload::Heartbeat(Heartbeat),
    )]]);
    let mut document = fixture.document(
        origin.clone(),
        DocumentTransportKind::ServerSentEvents,
        0x59,
        2,
    );
    document
        .add(&failing, fixture.request(subscription(29), origin.clone()))
        .await
        .expect("failing-close membership");
    document
        .add(&healthy, fixture.request(subscription(30), origin.clone()))
        .await
        .expect("healthy membership");

    assert!(matches!(
        document
            .next()
            .await
            .expect("terminal")
            .expect("terminal envelope")
            .payload(),
        AsyncPayload::Complete(_)
    ));
    let sibling = document
        .next()
        .await
        .expect("healthy delivery despite cleanup failure")
        .expect("healthy envelope");
    assert_eq!(sibling.subscription(), &subscription(30));
    assert_eq!(document.retiring_count(), 1);
    assert_eq!(
        document.last_cleanup_error(),
        Some(AsyncTransportErrorKind::SourceFailed)
    );

    let same_pending = document
        .prepare_add(fixture.request(subscription(29), origin.clone()))
        .expect("same-binding preparation");
    let same_authorized = same_pending
        .authorize()
        .await
        .expect("same-binding authority");
    assert_eq!(
        document
            .prepare_establish(same_authorized)
            .expect_err("permanent cleanup error retains exact fence")
            .kind(),
        AsyncTransportErrorKind::DuplicateMembership
    );
    let overlap_pending = document
        .prepare_add(overlap.request(subscription(29), origin))
        .expect("overlap preparation");
    let overlap_authorized = overlap_pending
        .authorize()
        .await
        .expect("overlap authority");
    assert_eq!(
        document
            .prepare_establish(overlap_authorized)
            .expect_err("permanent cleanup error fences another signed binding")
            .kind(),
        AsyncTransportErrorKind::DescriptorMismatch
    );
}

#[tokio::test]
async fn removal_detaches_immediately_and_shutdown_retries_partial_cleanup_truthfully() {
    let fixture = TransportFixture::new(position(9, 40)).await;
    let origin = VerifiedOrigin::parse("https://app.example.test").expect("origin");
    let pending =
        ScriptedSource::new(vec![vec![ScriptItem::Pending]]).with_permanently_pending_close();
    let mut document = fixture.document(
        origin.clone(),
        DocumentTransportKind::ServerSentEvents,
        0x5c,
        2,
    );
    document
        .add(&pending, fixture.request(subscription(41), origin.clone()))
        .await
        .expect("pending-close membership");
    assert_eq!(
        document
            .remove(&fixture.request(subscription(41), origin.clone()))
            .await
            .expect("logical removal is not blocked by provider cleanup"),
        CloseDisposition::Closed
    );
    assert_eq!(document.membership_count(), 0);
    assert_eq!(document.retiring_count(), 1);

    let retrying = ScriptedSource::new(vec![vec![ScriptItem::Pending]])
        .with_close_error_attempts(1, AsyncTransportErrorKind::SourceFailed);
    let mut shutdown = fixture.document(
        origin.clone(),
        DocumentTransportKind::ServerSentEvents,
        0x5d,
        1,
    );
    shutdown
        .add(&retrying, fixture.request(subscription(42), origin))
        .await
        .expect("retrying cleanup membership");
    assert_eq!(
        shutdown
            .close()
            .await
            .expect_err("first cleanup failure is truthful")
            .kind(),
        AsyncTransportErrorKind::SourceFailed
    );
    assert_eq!(shutdown.membership_count(), 0);
    assert_eq!(shutdown.retiring_count(), 1);
    assert_eq!(
        shutdown.close().await.expect("cleanup retry succeeds"),
        CloseDisposition::Closed
    );
    assert_eq!(
        shutdown.close().await.expect("closed is idempotent"),
        CloseDisposition::AlreadyClosed
    );
}

#[tokio::test]
async fn many_pending_retirements_remain_bounded_and_ready_membership_is_polled_fairly() {
    let fixture = TransportFixture::new(position(9, 50)).await;
    let origin = VerifiedOrigin::parse("https://app.example.test").expect("origin");
    let retiring = ScriptedSource::new(
        (0..7)
            .map(|_| {
                vec![ScriptItem::Envelope(
                    position(9, 51),
                    AsyncPayload::Complete(CompletionReason::StreamCompleted),
                )]
            })
            .collect(),
    )
    .with_permanently_pending_close();
    let healthy = ScriptedSource::new(vec![vec![ScriptItem::Envelope(
        position(9, 51),
        AsyncPayload::Heartbeat(Heartbeat),
    )]]);
    let mut document = fixture.document(
        origin.clone(),
        DocumentTransportKind::ServerSentEvents,
        0x5e,
        8,
    );
    for id in 50..57 {
        document
            .add(&retiring, fixture.request(subscription(id), origin.clone()))
            .await
            .expect("retiring membership");
    }
    document
        .add(&healthy, fixture.request(subscription(57), origin.clone()))
        .await
        .expect("healthy membership");
    for _ in 0..7 {
        assert!(matches!(
            document
                .next()
                .await
                .expect("terminal delivery")
                .expect("terminal envelope")
                .payload(),
            AsyncPayload::Complete(_)
        ));
    }
    assert_eq!(document.retiring_count(), 7);
    let ready = document
        .next()
        .await
        .expect("bounded retirement polling")
        .expect("healthy envelope");
    assert_eq!(ready.subscription(), &subscription(57));
    assert_eq!(document.retiring_count(), 7);
    assert!(retiring.close_poll_count() >= 7);

    assert_eq!(
        document
            .add(&healthy, fixture.request(subscription(58), origin),)
            .await
            .expect_err("active plus retiring sessions remain hard bounded")
            .kind(),
        AsyncTransportErrorKind::MembershipLimit
    );
}

#[derive(Default)]
struct RecordingDispatcher {
    applied: Vec<suprnova_live::async_updates::StreamPosition>,
}

impl AsyncEnvelopeDispatchPort for RecordingDispatcher {
    fn dispatch(&mut self, envelope: &AsyncEnvelope) -> Result<(), AsyncDispatchError> {
        self.applied.push(envelope.position());
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AdapterKind {
    Sse,
    WebSocket,
}

#[derive(Debug)]
struct AdapterCoverage {
    adapter: AdapterKind,
    semantic_cases: usize,
    connects: usize,
    subscribe_controls: usize,
    unsubscribe_controls: usize,
    wire_records: usize,
}

impl AdapterCoverage {
    fn new(adapter: AdapterKind) -> Self {
        Self {
            adapter,
            semantic_cases: 0,
            connects: 0,
            subscribe_controls: 0,
            unsubscribe_controls: 0,
            wire_records: 0,
        }
    }

    fn case(&mut self) {
        self.semantic_cases += 1;
    }
}

struct AdapterHarness {
    adapter: AdapterKind,
    origin: VerifiedOrigin,
    handle: DocumentTransportHandle,
    document: DocumentTransportSession,
    websocket: WebSocketCodec,
}

impl AdapterHarness {
    fn connect(
        adapter: AdapterKind,
        maximum_memberships: usize,
        handle_byte: u8,
        authorization_scope: DocumentAuthorizationScope,
        coverage: &mut AdapterCoverage,
    ) -> Self {
        let origin = VerifiedOrigin::parse("https://app.example.test").expect("origin");
        let kind = match adapter {
            AdapterKind::Sse => {
                let headers = SseResponseContract::headers();
                assert_eq!(
                    headers[http::header::CACHE_CONTROL],
                    "no-store, no-transform"
                );
                assert_eq!(
                    headers[http::header::CONTENT_TYPE],
                    "text/event-stream; charset=utf-8"
                );
                DocumentTransportKind::ServerSentEvents
            }
            AdapterKind::WebSocket => {
                let policy = WebSocketOriginPolicy::new(origin.clone(), Vec::new())
                    .expect("strict origin policy");
                let origin_text = origin.to_string();
                let upgrade = policy
                    .authorize_upgrade(&[origin_text.as_str()], || {
                        Ok(WebSocketAuthentication::Cookie(()))
                    })
                    .expect("origin validated before same-origin cookie authentication");
                assert_eq!(upgrade.origin(), &origin);
                assert!(!upgrade.is_cross_origin());
                DocumentTransportKind::WebSocket
            }
        };
        coverage.connects += 1;
        let handle = DocumentTransportHandle::from_bytes(&[handle_byte; 16]).expect("handle");
        let document = DocumentTransportSession::new(
            origin.clone(),
            kind,
            handle.clone(),
            DocumentTransportLimits::new(maximum_memberships).expect("limits"),
            authorization_scope,
        );
        Self {
            adapter,
            origin,
            handle,
            document,
            websocket: WebSocketCodec::v1(),
        }
    }

    async fn subscribe(
        &mut self,
        source: &dyn AsyncEventSource,
        authorization: AuthorizedTransportSubscription,
        coverage: &mut AdapterCoverage,
    ) -> Result<(), suprnova_live::async_updates::AsyncTransportError> {
        coverage.subscribe_controls += 1;
        let pending = match self.adapter {
            AdapterKind::Sse => SseMembershipControl::prepare_subscribe(
                &self.document,
                &self.handle,
                &self.origin,
                authorization,
            ),
            AdapterKind::WebSocket => {
                let control =
                    WebSocketControlRecord::Subscribe(authorization.subscription().clone());
                let encoded = self.websocket.encode_control(&control)?;
                let decoded = self.websocket.decode_control(WebSocketFrame::Text {
                    payload: &encoded,
                    final_fragment: true,
                })?;
                WebSocketMembershipControl::prepare_subscribe(
                    &self.document,
                    &decoded,
                    authorization,
                )
            }
        }?;
        let authorized = pending.authorize().await?;
        let establishing = self.document.prepare_establish(authorized)?;
        let ready = establishing.establish(source).await?;
        self.document.commit_add(ready)
    }

    async fn unsubscribe(
        &mut self,
        authorization: &AuthorizedTransportSubscription,
        coverage: &mut AdapterCoverage,
    ) -> Result<CloseDisposition, suprnova_live::async_updates::AsyncTransportError> {
        coverage.unsubscribe_controls += 1;
        let pending = match self.adapter {
            AdapterKind::Sse => SseMembershipControl::prepare_unsubscribe(
                &self.document,
                &self.handle,
                &self.origin,
                authorization,
            ),
            AdapterKind::WebSocket => {
                let control =
                    WebSocketControlRecord::Unsubscribe(authorization.subscription().clone());
                let encoded = self.websocket.encode_control(&control)?;
                let decoded = self.websocket.decode_control(WebSocketFrame::Text {
                    payload: &encoded,
                    final_fragment: true,
                })?;
                WebSocketMembershipControl::prepare_unsubscribe(
                    &self.document,
                    &decoded,
                    authorization,
                )
            }
        }?;
        let ready = pending.authorize().await?;
        self.document.commit_remove(ready)
    }

    async fn next_wire(
        &mut self,
        context: &suprnova_live::async_updates::AsyncEnvelopeContext,
        coverage: &mut AdapterCoverage,
    ) -> Result<Option<AsyncEnvelope>, suprnova_live::async_updates::AsyncTransportError> {
        let Some(envelope) = self.document.next().await? else {
            return Ok(None);
        };
        coverage.wire_records += 1;
        let decoded = match self.adapter {
            AdapterKind::Sse => {
                let event = SseEncoder::encode_envelope(&envelope)?;
                assert_eq!(event.event(), "suprnova-live-async");
                decode_async_envelope(event.data(), &AsyncCodecLimits::v1(), context).map_err(
                    |_| {
                        suprnova_live::async_updates::AsyncTransportError::new(
                            AsyncTransportErrorKind::InvalidEnvelope,
                        )
                    },
                )?
            }
            AdapterKind::WebSocket => {
                let encoded = self.websocket.encode_envelope(&envelope)?;
                self.websocket.decode_envelope(
                    WebSocketFrame::Text {
                        payload: &encoded,
                        final_fragment: true,
                    },
                    context,
                )?
            }
        };
        Ok(Some(decoded))
    }
}

async fn assert_full_adapter_conformance(adapter: AdapterKind) -> AdapterCoverage {
    let mut coverage = AdapterCoverage::new(adapter);

    // Signed baseline, ordered replay, heartbeat, nonterminal typed error, and terminal completion.
    coverage.case();
    let fixture = TransportFixture::new(position(30, 40)).await;
    let authorization = fixture.request(
        subscription(70),
        VerifiedOrigin::parse("https://app.example.test").expect("origin"),
    );
    let context = authorization.context().clone();
    let source = ScriptedSource::new(vec![vec![
        ScriptItem::Envelope(position(30, 41), AsyncPayload::Heartbeat(Heartbeat)),
        ScriptItem::Envelope(
            position(30, 42),
            AsyncPayload::Error(StreamErrorCode::ReplayUnavailable),
        ),
        ScriptItem::Envelope(
            position(30, 43),
            AsyncPayload::Complete(CompletionReason::StreamCompleted),
        ),
        ScriptItem::Envelope(
            position(30, 44),
            AsyncPayload::Error(StreamErrorCode::ReplayUnavailable),
        ),
        ScriptItem::End,
    ]]);
    let mut harness = AdapterHarness::connect(
        adapter,
        2,
        0x70,
        fixture.document_scope.clone(),
        &mut coverage,
    );
    harness
        .subscribe(&source, authorization, &mut coverage)
        .await
        .expect("adapter baseline connect");
    let first = harness
        .next_wire(&context, &mut coverage)
        .await
        .expect("heartbeat")
        .expect("heartbeat envelope");
    let typed_error = harness
        .next_wire(&context, &mut coverage)
        .await
        .expect("typed error")
        .expect("typed error envelope");
    let completion = harness
        .next_wire(&context, &mut coverage)
        .await
        .expect("completion")
        .expect("completion envelope");
    assert_eq!(first.position(), position(30, 41));
    assert!(matches!(first.payload(), AsyncPayload::Heartbeat(_)));
    assert!(matches!(typed_error.payload(), AsyncPayload::Error(_)));
    assert!(matches!(completion.payload(), AsyncPayload::Complete(_)));
    assert!(
        harness
            .next_wire(&context, &mut coverage)
            .await
            .expect("logical completion")
            .is_none()
    );
    if adapter == AdapterKind::Sse {
        assert_eq!(
            SseEncoder::heartbeat_comment(),
            b": suprnova-live heartbeat\n\n"
        );
    }

    // Task 3 duplicate/gap authority remains independent after adapter decode.
    coverage.case();
    let fixture = TransportFixture::new(position(31, 10)).await;
    let authorization = fixture.request(
        subscription(71),
        VerifiedOrigin::parse("https://app.example.test").expect("origin"),
    );
    let context = authorization.context().clone();
    let source = ScriptedSource::new(vec![vec![
        ScriptItem::Envelope(position(31, 11), AsyncPayload::Refresh(RegisteredRefresh)),
        ScriptItem::Envelope(position(31, 11), AsyncPayload::Heartbeat(Heartbeat)),
        ScriptItem::Envelope(position(31, 13), AsyncPayload::Refresh(RegisteredRefresh)),
        ScriptItem::Envelope(position(31, 12), AsyncPayload::Refresh(RegisteredRefresh)),
        ScriptItem::Envelope(position(31, 13), AsyncPayload::Heartbeat(Heartbeat)),
    ]]);
    let mut harness = AdapterHarness::connect(
        adapter,
        1,
        0x71,
        fixture.document_scope.clone(),
        &mut coverage,
    );
    harness
        .subscribe(&source, authorization, &mut coverage)
        .await
        .expect("adapter sequence membership");
    let mut machine = SequenceMachine::new(&context);
    let mut dispatcher = RecordingDispatcher::default();
    let mut dispositions = Vec::new();
    for _ in 0..3 {
        let envelope = harness
            .next_wire(&context, &mut coverage)
            .await
            .expect("adapter delivery")
            .expect("envelope");
        let guard = context
            .admit(&envelope, fixture.registry.as_ref(), UnixMillis::new(1_200))
            .expect("fresh admission");
        dispositions.push(
            machine
                .dispatch(guard, UnixMillis::new(1_200), &mut dispatcher)
                .expect("sequence classification"),
        );
    }
    assert_eq!(
        dispositions,
        vec![
            SequenceDisposition::Apply,
            SequenceDisposition::IgnoreDuplicate,
            SequenceDisposition::Degraded(SequenceDegradation::Gap),
        ]
    );
    assert_eq!(dispatcher.applied, vec![position(31, 11)]);
    let mut replay = Vec::new();
    for _ in 0..2 {
        replay.push(
            harness
                .next_wire(&context, &mut coverage)
                .await
                .expect("adapter replay delivery")
                .expect("replay envelope"),
        );
    }
    let replay_guards = replay
        .iter()
        .map(|envelope| {
            context
                .admit(envelope, fixture.registry.as_ref(), UnixMillis::new(1_200))
                .expect("fresh replay admission")
        })
        .collect::<Vec<_>>();
    let replay_outcome = machine
        .recover_from_replay(replay_guards, UnixMillis::new(1_200), &mut dispatcher)
        .expect("complete adapter replay");
    assert_eq!(replay_outcome.applied(), 2);
    assert_eq!(machine.current(), position(31, 13));
    assert_eq!(
        dispatcher.applied,
        vec![position(31, 11), position(31, 12), position(31, 13)]
    );

    // Exact subscription routing is enforced before either adapter emits bytes.
    coverage.case();
    let fixture = TransportFixture::new(position(32, 0)).await;
    let first = fixture.request(
        subscription(72),
        VerifiedOrigin::parse("https://app.example.test").expect("origin"),
    );
    let second = fixture.request(
        subscription(73),
        VerifiedOrigin::parse("https://app.example.test").expect("origin"),
    );
    let foreign = AsyncEnvelope::new(
        second.context(),
        position(32, 1),
        AsyncPayload::Heartbeat(Heartbeat),
    )
    .expect("foreign envelope");
    let source = ScriptedSource::new(vec![vec![ScriptItem::RawEnvelope(foreign)]]);
    let mut harness = AdapterHarness::connect(
        adapter,
        1,
        0x72,
        fixture.document_scope.clone(),
        &mut coverage,
    );
    harness
        .subscribe(&source, first, &mut coverage)
        .await
        .expect("routing membership");
    assert_eq!(
        harness
            .document
            .next()
            .await
            .expect_err("cross-subscription route")
            .kind(),
        AsyncTransportErrorKind::RoutingMismatch
    );

    // Cancelling adapter-specific establishment commits no logical membership.
    coverage.case();
    let fixture = TransportFixture::new(position(33, 0)).await;
    let source = ControlledSubscribeSource::new();
    let mut harness = AdapterHarness::connect(
        adapter,
        1,
        0x73,
        fixture.document_scope.clone(),
        &mut coverage,
    );
    let mut pending = Box::pin(harness.subscribe(
        &source,
        fixture.request(subscription(74), harness.origin.clone()),
        &mut coverage,
    ));
    let waker = Waker::noop();
    let mut task = Context::from_waker(waker);
    assert!(matches!(pending.as_mut().poll(&mut task), Poll::Pending));
    drop(pending);
    assert_eq!(harness.document.membership_count(), 0);
    assert_eq!(source.close_count(), 0);

    // Current auth loss is observed through each real control path.
    coverage.case();
    let fixture = TransportFixture::new(position(34, 0)).await;
    let request = fixture.request(
        subscription(75),
        VerifiedOrigin::parse("https://app.example.test").expect("origin"),
    );
    fixture.registry.revoke();
    let source = ScriptedSource::new(vec![vec![ScriptItem::Pending]]);
    let mut harness = AdapterHarness::connect(
        adapter,
        1,
        0x74,
        fixture.document_scope.clone(),
        &mut coverage,
    );
    assert_eq!(
        harness
            .subscribe(&source, request, &mut coverage)
            .await
            .expect_err("adapter current authorization loss")
            .kind(),
        AsyncTransportErrorKind::AuthorizationLost
    );

    // Duplicate and hard membership limits are enforced before source work.
    coverage.case();
    let fixture = TransportFixture::new(position(35, 0)).await;
    let source = ScriptedSource::new(vec![vec![ScriptItem::Pending]]);
    let mut harness = AdapterHarness::connect(
        adapter,
        1,
        0x75,
        fixture.document_scope.clone(),
        &mut coverage,
    );
    harness
        .subscribe(
            &source,
            fixture.request(subscription(76), harness.origin.clone()),
            &mut coverage,
        )
        .await
        .expect("first membership");
    assert_eq!(
        harness
            .subscribe(
                &source,
                fixture.request(subscription(76), harness.origin.clone()),
                &mut coverage,
            )
            .await
            .expect_err("duplicate membership")
            .kind(),
        AsyncTransportErrorKind::DuplicateMembership
    );
    assert_eq!(
        harness
            .subscribe(
                &source,
                fixture.request(subscription(77), harness.origin.clone()),
                &mut coverage,
            )
            .await
            .expect_err("membership hard limit")
            .kind(),
        AsyncTransportErrorKind::MembershipLimit
    );
    harness.document.close().await.expect("limit cleanup");

    // One pending logical client cannot stall a ready sibling on the same transport.
    coverage.case();
    let fixture = TransportFixture::new(position(36, 0)).await;
    let source = ScriptedSource::new(vec![
        vec![ScriptItem::Pending],
        vec![ScriptItem::Envelope(
            position(36, 1),
            AsyncPayload::Heartbeat(Heartbeat),
        )],
    ]);
    let mut harness = AdapterHarness::connect(
        adapter,
        2,
        0x76,
        fixture.document_scope.clone(),
        &mut coverage,
    );
    let first = fixture.request(subscription(78), harness.origin.clone());
    let second = fixture.request(subscription(79), harness.origin.clone());
    let second_context = second.context().clone();
    harness
        .subscribe(&source, first, &mut coverage)
        .await
        .expect("slow membership");
    harness
        .subscribe(&source, second, &mut coverage)
        .await
        .expect("ready membership");
    let ready = harness
        .next_wire(&second_context, &mut coverage)
        .await
        .expect("bounded fan-in")
        .expect("ready sibling");
    assert_eq!(ready.subscription(), &subscription(79));
    harness.document.close().await.expect("slow cleanup");

    // Typed provider errors retire only their logical session.
    coverage.case();
    let fixture = TransportFixture::new(position(37, 0)).await;
    let source = ScriptedSource::new(vec![vec![ScriptItem::Error(
        AsyncTransportErrorKind::SourceFailed,
    )]]);
    let mut harness = AdapterHarness::connect(
        adapter,
        1,
        0x77,
        fixture.document_scope.clone(),
        &mut coverage,
    );
    harness
        .subscribe(
            &source,
            fixture.request(subscription(80), harness.origin.clone()),
            &mut coverage,
        )
        .await
        .expect("error membership");
    assert_eq!(
        harness
            .document
            .next()
            .await
            .expect_err("typed provider error")
            .kind(),
        AsyncTransportErrorKind::SourceFailed
    );
    assert_eq!(harness.document.membership_count(), 0);

    // Authenticated adapter-specific unsubscribe removes exactly one membership.
    coverage.case();
    let fixture = TransportFixture::new(position(38, 0)).await;
    let source = ScriptedSource::new(vec![vec![ScriptItem::Pending]]);
    let mut harness = AdapterHarness::connect(
        adapter,
        1,
        0x78,
        fixture.document_scope.clone(),
        &mut coverage,
    );
    harness
        .subscribe(
            &source,
            fixture.request(subscription(81), harness.origin.clone()),
            &mut coverage,
        )
        .await
        .expect("unsubscribe membership");
    assert_eq!(
        harness
            .unsubscribe(
                &fixture.request(subscription(81), harness.origin.clone()),
                &mut coverage,
            )
            .await
            .expect("adapter unsubscribe"),
        CloseDisposition::Closed
    );
    assert_eq!(harness.document.membership_count(), 0);

    // Exact descriptor bindings and physical document scope survive real control paths.
    coverage.case();
    let old =
        TransportFixture::new_with_signing_key(position(38, 10), "adapter-old-key", 0x81).await;
    let current =
        TransportFixture::new_with_signing_key(position(38, 10), "adapter-current-key", 0x82).await;
    let revised = TransportFixture::new_with_contract_revision(position(38, 10)).await;
    let foreign = TransportFixture::new_in_scope(position(38, 10), 0x83).await;
    let source = ScriptedSource::new(vec![vec![ScriptItem::Pending], vec![ScriptItem::Pending]]);
    let mut harness =
        AdapterHarness::connect(adapter, 3, 0x7a, old.document_scope.clone(), &mut coverage);
    harness
        .subscribe(
            &source,
            old.request(subscription(83), harness.origin.clone()),
            &mut coverage,
        )
        .await
        .expect("old-key adapter membership");
    harness
        .subscribe(
            &source,
            revised.request(subscription(84), harness.origin.clone()),
            &mut coverage,
        )
        .await
        .expect("heterogeneous component shares physical scope");
    assert_eq!(
        harness
            .unsubscribe(
                &current.request(subscription(83), harness.origin.clone()),
                &mut coverage,
            )
            .await
            .expect_err("another exact descriptor cannot control removal")
            .kind(),
        AsyncTransportErrorKind::DescriptorMismatch
    );
    assert_eq!(
        harness
            .subscribe(
                &source,
                current.request(subscription(83), harness.origin.clone()),
                &mut coverage,
            )
            .await
            .expect_err("another exact descriptor cannot replace membership")
            .kind(),
        AsyncTransportErrorKind::DescriptorMismatch
    );
    assert_eq!(
        harness
            .subscribe(
                &source,
                foreign.request(subscription(85), harness.origin.clone()),
                &mut coverage,
            )
            .await
            .expect_err("another physical scope cannot join document")
            .kind(),
        AsyncTransportErrorKind::AuthorizationScopeMismatch
    );
    harness
        .unsubscribe(
            &old.request(subscription(83), harness.origin.clone()),
            &mut coverage,
        )
        .await
        .expect("exact old-key removal");
    harness
        .unsubscribe(
            &revised.request(subscription(84), harness.origin.clone()),
            &mut coverage,
        )
        .await
        .expect("exact revised-component removal");

    // Cancelled shutdown retains authority; resumed close is idempotent.
    coverage.case();
    let fixture = TransportFixture::new(position(39, 0)).await;
    let source = ScriptedSource::new(vec![vec![ScriptItem::Pending]]).with_pending_first_close();
    let mut harness = AdapterHarness::connect(
        adapter,
        1,
        0x79,
        fixture.document_scope.clone(),
        &mut coverage,
    );
    harness
        .subscribe(
            &source,
            fixture.request(subscription(82), harness.origin.clone()),
            &mut coverage,
        )
        .await
        .expect("shutdown membership");
    let mut close = Box::pin(harness.document.close());
    let mut task = Context::from_waker(waker);
    assert!(matches!(close.as_mut().poll(&mut task), Poll::Pending));
    drop(close);
    assert_eq!(source.close_count(), 0);
    assert_eq!(
        harness.document.close().await.expect("resumed close"),
        CloseDisposition::Closed
    );
    assert_eq!(
        harness.document.close().await.expect("idempotent close"),
        CloseDisposition::AlreadyClosed
    );
    assert_eq!(source.close_count(), 1);

    coverage
}

#[tokio::test]
async fn full_shared_conformance_runs_through_both_real_adapter_paths() {
    let sse = assert_full_adapter_conformance(AdapterKind::Sse).await;
    let websocket = assert_full_adapter_conformance(AdapterKind::WebSocket).await;

    for coverage in [&sse, &websocket] {
        assert_eq!(coverage.semantic_cases, 11, "{coverage:?}");
        assert!(coverage.connects >= coverage.semantic_cases, "{coverage:?}");
        assert!(
            coverage.subscribe_controls >= coverage.semantic_cases,
            "{coverage:?}"
        );
        assert!(coverage.unsubscribe_controls >= 1, "{coverage:?}");
        assert!(coverage.wire_records >= 9, "{coverage:?}");
    }
    assert_eq!(sse.adapter, AdapterKind::Sse);
    assert_eq!(websocket.adapter, AdapterKind::WebSocket);
}

#[tokio::test]
async fn multiplexing_preserves_independent_task_three_duplicate_and_gap_authority() {
    let baseline = position(13, 40);
    let fixture = TransportFixture::new(baseline).await;
    let origin = VerifiedOrigin::parse("https://app.example.test").expect("origin");
    let authorization = fixture.request(subscription(18), origin.clone());
    let context = authorization.context().clone();
    let source = ScriptedSource::new(vec![vec![
        ScriptItem::Envelope(position(13, 41), AsyncPayload::Refresh(RegisteredRefresh)),
        ScriptItem::Envelope(position(13, 41), AsyncPayload::Heartbeat(Heartbeat)),
        ScriptItem::Envelope(position(13, 43), AsyncPayload::Refresh(RegisteredRefresh)),
        ScriptItem::End,
    ]]);
    let mut document = DocumentTransportSession::new(
        origin,
        DocumentTransportKind::ServerSentEvents,
        DocumentTransportHandle::from_bytes(&[0x51; 16]).expect("handle"),
        DocumentTransportLimits::new(1).expect("limits"),
        fixture.document_scope.clone(),
    );
    document
        .add(&source, authorization)
        .await
        .expect("logical session");

    let mut machine = SequenceMachine::new(&context);
    let mut dispatcher = RecordingDispatcher::default();
    let mut dispositions = Vec::new();
    for _ in 0..3 {
        let envelope = document.next().await.expect("delivery").expect("envelope");
        let guard = context
            .admit(&envelope, fixture.registry.as_ref(), UnixMillis::new(1_200))
            .expect("fresh logical membership");
        dispositions.push(
            machine
                .dispatch(guard, UnixMillis::new(1_200), &mut dispatcher)
                .expect("sequence classification"),
        );
    }

    assert_eq!(
        dispositions,
        vec![
            SequenceDisposition::Apply,
            SequenceDisposition::IgnoreDuplicate,
            SequenceDisposition::Degraded(SequenceDegradation::Gap),
        ]
    );
    assert_eq!(machine.current(), position(13, 41));
    assert_eq!(dispatcher.applied, vec![position(13, 41)]);
    assert!(document.next().await.expect("completion").is_none());
}

#[tokio::test]
async fn logical_session_delivers_a_complete_ordered_replay_from_the_signed_baseline() {
    let baseline = position(16, 70);
    let fixture = TransportFixture::new(baseline).await;
    let origin = VerifiedOrigin::parse("https://app.example.test").expect("origin");
    let authorization = fixture.request(subscription(24), origin.clone());
    let context = authorization.context().clone();
    let source = ScriptedSource::new(vec![vec![
        ScriptItem::Envelope(position(16, 72), AsyncPayload::Refresh(RegisteredRefresh)),
        ScriptItem::Envelope(position(16, 71), AsyncPayload::Refresh(RegisteredRefresh)),
        ScriptItem::Envelope(position(16, 72), AsyncPayload::Heartbeat(Heartbeat)),
        ScriptItem::End,
    ]]);
    let mut document = DocumentTransportSession::new(
        origin,
        DocumentTransportKind::ServerSentEvents,
        DocumentTransportHandle::from_bytes(&[0x53; 16]).expect("handle"),
        DocumentTransportLimits::new(1).expect("limits"),
        fixture.document_scope.clone(),
    );
    document
        .add(&source, authorization)
        .await
        .expect("logical replay session");

    let mut machine = SequenceMachine::new(&context);
    let mut dispatcher = RecordingDispatcher::default();
    let observed_gap = document
        .next()
        .await
        .expect("gap delivery")
        .expect("gap envelope");
    let gap_guard = context
        .admit(
            &observed_gap,
            fixture.registry.as_ref(),
            UnixMillis::new(1_200),
        )
        .expect("fresh gap membership");
    assert_eq!(
        machine
            .dispatch(gap_guard, UnixMillis::new(1_200), &mut dispatcher)
            .expect("gap classification"),
        SequenceDisposition::Degraded(SequenceDegradation::Gap)
    );

    let mut transcript = Vec::new();
    while let Some(envelope) = document.next().await.expect("ordered replay delivery") {
        transcript.push(envelope);
    }
    let guards = transcript
        .iter()
        .map(|envelope| {
            context
                .admit(envelope, fixture.registry.as_ref(), UnixMillis::new(1_200))
                .expect("fresh replay membership")
        })
        .collect::<Vec<_>>();
    let outcome = machine
        .recover_from_replay(guards, UnixMillis::new(1_200), &mut dispatcher)
        .expect("complete contiguous replay");

    assert_eq!(outcome.applied(), 2);
    assert_eq!(machine.current(), position(16, 72));
    assert_eq!(dispatcher.applied, vec![position(16, 71), position(16, 72)]);
}

#[tokio::test]
async fn membership_requires_unexpired_descriptor_and_exact_descriptor_on_remove() {
    let fixture = TransportFixture::new(position(10, 0)).await;
    let origin = VerifiedOrigin::parse("https://app.example.test").expect("origin");
    let expired = fixture
        .request_at(
            subscription(11),
            origin.clone(),
            suprnova_live::identity::UnixMillis::new(5_000),
        )
        .expect_err("exclusive expiry");
    assert_eq!(expired.kind(), AsyncTransportErrorKind::AuthorizationLost);

    let source = ScriptedSource::new(vec![vec![ScriptItem::Pending]]);
    let mut document = DocumentTransportSession::new(
        origin.clone(),
        DocumentTransportKind::ServerSentEvents,
        DocumentTransportHandle::from_bytes(&[0x46; 16]).expect("handle"),
        DocumentTransportLimits::new(2).expect("limits"),
        fixture.document_scope.clone(),
    );
    document
        .add(&source, fixture.request(subscription(12), origin.clone()))
        .await
        .expect("membership");

    let foreign = TransportFixture::new(position(10, 1)).await;
    let mismatch = document
        .remove(&foreign.request(subscription(12), origin))
        .await
        .expect_err("descriptor mismatch");
    assert_eq!(mismatch.kind(), AsyncTransportErrorKind::DescriptorMismatch);
    document.close().await.expect("close");
}

#[tokio::test]
async fn membership_rechecks_expiry_revocation_and_scope_before_source_work() {
    let origin = VerifiedOrigin::parse("https://app.example.test").expect("origin");

    let expired_fixture = TransportFixture::new(position(16, 0)).await;
    let expired_request = expired_fixture.request(subscription(31), origin.clone());
    expired_fixture.registry.set_now(UnixMillis::new(5_000));
    let expired_source = ControlledSubscribeSource::new();
    let mut expired_document = DocumentTransportSession::new(
        origin.clone(),
        DocumentTransportKind::ServerSentEvents,
        DocumentTransportHandle::from_bytes(&[0x61; 16]).expect("handle"),
        DocumentTransportLimits::new(1).expect("limits"),
        expired_fixture.document_scope.clone(),
    );
    assert_eq!(
        expired_document
            .add(&expired_source, expired_request)
            .await
            .expect_err("held authorization expires before consumption")
            .kind(),
        AsyncTransportErrorKind::AuthorizationLost
    );
    assert_eq!(expired_document.membership_count(), 0);
    assert!(!expired_source.observed());

    let revoked_fixture = TransportFixture::new(position(16, 10)).await;
    let revoked_request = revoked_fixture.request(subscription(32), origin.clone());
    revoked_fixture.registry.revoke();
    let revoked_source = ControlledSubscribeSource::new();
    let mut revoked_document = DocumentTransportSession::new(
        origin.clone(),
        DocumentTransportKind::ServerSentEvents,
        DocumentTransportHandle::from_bytes(&[0x62; 16]).expect("handle"),
        DocumentTransportLimits::new(1).expect("limits"),
        revoked_fixture.document_scope.clone(),
    );
    assert_eq!(
        revoked_document
            .add(&revoked_source, revoked_request)
            .await
            .expect_err("membership revoked before consumption")
            .kind(),
        AsyncTransportErrorKind::AuthorizationLost
    );
    assert!(!revoked_source.observed());

    let scope_fixture = TransportFixture::new(position(16, 20)).await;
    let scope_request = scope_fixture.request(subscription(33), origin.clone());
    scope_fixture.registry.change_authorization_scope();
    let scope_source = ControlledSubscribeSource::new();
    let mut scope_document = DocumentTransportSession::new(
        origin,
        DocumentTransportKind::ServerSentEvents,
        DocumentTransportHandle::from_bytes(&[0x63; 16]).expect("handle"),
        DocumentTransportLimits::new(1).expect("limits"),
        scope_fixture.document_scope.clone(),
    );
    assert_eq!(
        scope_document
            .add(&scope_source, scope_request)
            .await
            .expect_err("principal/session/tenant/component scope changed")
            .kind(),
        AsyncTransportErrorKind::AuthorizationLost
    );
    assert!(!scope_source.observed());

    let document_scope_fixture = TransportFixture::new(position(16, 25)).await;
    let document_scope_request = document_scope_fixture.request(
        subscription(43),
        VerifiedOrigin::parse("https://app.example.test").expect("origin"),
    );
    document_scope_fixture.registry.change_document_scope();
    let document_scope_source = ControlledSubscribeSource::new();
    let mut document_scope_document = document_scope_fixture.document(
        VerifiedOrigin::parse("https://app.example.test").expect("origin"),
        DocumentTransportKind::ServerSentEvents,
        0x6b,
        1,
    );
    assert_eq!(
        document_scope_document
            .add(&document_scope_source, document_scope_request)
            .await
            .expect_err("connection identity/policy scope changed before consumption")
            .kind(),
        AsyncTransportErrorKind::AuthorizationLost
    );
    assert!(!document_scope_source.observed());

    let duplicate_snapshot_fixture = TransportFixture::new(position(16, 30)).await;
    duplicate_snapshot_fixture.registry.accept_twice();
    let duplicate_snapshot_source = ScriptedSource::new(vec![vec![ScriptItem::End]]);
    let mut duplicate_snapshot_document = DocumentTransportSession::new(
        VerifiedOrigin::parse("https://app.example.test").expect("origin"),
        DocumentTransportKind::ServerSentEvents,
        DocumentTransportHandle::from_bytes(&[0x6a; 16]).expect("handle"),
        DocumentTransportLimits::new(1).expect("limits"),
        duplicate_snapshot_fixture.document_scope.clone(),
    );
    assert_eq!(
        duplicate_snapshot_document
            .add(
                &duplicate_snapshot_source,
                duplicate_snapshot_fixture.request(
                    subscription(40),
                    VerifiedOrigin::parse("https://app.example.test").expect("origin"),
                ),
            )
            .await
            .expect_err("authority port may accept only one coherent snapshot")
            .kind(),
        AsyncTransportErrorKind::AuthorizationLost
    );
}

#[tokio::test]
async fn membership_revalidates_after_subscribe_and_disposes_revoked_session_once() {
    let fixture = TransportFixture::new(position(17, 0)).await;
    let origin = VerifiedOrigin::parse("https://app.example.test").expect("origin");
    let source = ControlledSubscribeSource::new();
    let mut document = DocumentTransportSession::new(
        origin.clone(),
        DocumentTransportKind::ServerSentEvents,
        DocumentTransportHandle::from_bytes(&[0x64; 16]).expect("handle"),
        DocumentTransportLimits::new(1).expect("limits"),
        fixture.document_scope.clone(),
    );

    let mut add = Box::pin(document.add(&source, fixture.request(subscription(34), origin)));
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    assert!(matches!(add.as_mut().poll(&mut context), Poll::Pending));
    assert!(source.observed());
    fixture.registry.revoke();
    source.release();
    assert_eq!(
        add.await
            .expect_err("revocation during source subscribe")
            .kind(),
        AsyncTransportErrorKind::AuthorizationLost
    );
    assert_eq!(source.close_count(), 1);
    assert_eq!(document.membership_count(), 0);
}

#[tokio::test]
async fn membership_revalidates_expiry_and_registration_mode_after_subscribe_wait() {
    let origin = VerifiedOrigin::parse("https://app.example.test").expect("origin");

    let expired_fixture = TransportFixture::new(position(17, 10)).await;
    let expired_source = ControlledSubscribeSource::new();
    let mut expired_document = DocumentTransportSession::new(
        origin.clone(),
        DocumentTransportKind::ServerSentEvents,
        DocumentTransportHandle::from_bytes(&[0x65; 16]).expect("handle"),
        DocumentTransportLimits::new(1).expect("limits"),
        expired_fixture.document_scope.clone(),
    );
    let mut expired_add = Box::pin(expired_document.add(
        &expired_source,
        expired_fixture.request(subscription(35), origin.clone()),
    ));
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    assert!(matches!(
        expired_add.as_mut().poll(&mut context),
        Poll::Pending
    ));
    expired_fixture.registry.set_now(UnixMillis::new(5_000));
    expired_source.release();
    assert_eq!(
        expired_add
            .await
            .expect_err("expiry during source subscribe")
            .kind(),
        AsyncTransportErrorKind::AuthorizationLost
    );
    assert_eq!(expired_source.close_count(), 1);
    assert_eq!(expired_document.membership_count(), 0);

    let mode_fixture = TransportFixture::new(position(17, 20)).await;
    let mode_source = ControlledSubscribeSource::new();
    let mode_handle = DocumentTransportHandle::from_bytes(&[0x66; 16]).expect("handle");
    let mut mode_document = DocumentTransportSession::new(
        origin.clone(),
        DocumentTransportKind::ServerSentEvents,
        mode_handle.clone(),
        DocumentTransportLimits::new(1).expect("limits"),
        mode_fixture.document_scope.clone(),
    );
    let mut mode_add = Box::pin(sse_subscribe(
        &mut mode_document,
        &mode_handle,
        &origin,
        &mode_source,
        mode_fixture.request(subscription(36), origin.clone()),
    ));
    let mut context = Context::from_waker(waker);
    assert!(matches!(
        mode_add.as_mut().poll(&mut context),
        Poll::Pending
    ));
    mode_fixture
        .registry
        .set_modes(vec![SubscriptionMode::ServerSentEvents]);
    mode_source.release();
    assert_eq!(
        mode_add
            .await
            .expect_err("same-name mode revision during source subscribe")
            .kind(),
        AsyncTransportErrorKind::AuthorizationLost
    );
    assert_eq!(mode_source.close_count(), 1);
    assert_eq!(mode_document.membership_count(), 0);

    let scope_fixture = TransportFixture::new(position(17, 30)).await;
    let scope_source = ControlledSubscribeSource::new();
    let mut scope_document = scope_fixture.document(
        origin.clone(),
        DocumentTransportKind::ServerSentEvents,
        0x6c,
        1,
    );
    let mut scope_add = Box::pin(scope_document.add(
        &scope_source,
        scope_fixture.request(subscription(44), origin),
    ));
    let mut context = Context::from_waker(waker);
    assert!(matches!(
        scope_add.as_mut().poll(&mut context),
        Poll::Pending
    ));
    scope_fixture.registry.change_document_scope();
    scope_source.release();
    assert_eq!(
        scope_add
            .await
            .expect_err("document identity/policy drift during source subscribe")
            .kind(),
        AsyncTransportErrorKind::AuthorizationLost
    );
    assert_eq!(scope_source.close_count(), 1);
    assert_eq!(scope_document.membership_count(), 0);
}

#[tokio::test]
async fn registered_modes_are_authority_and_external_remove_rechecks_policy() {
    let origin = VerifiedOrigin::parse("https://app.example.test").expect("origin");
    let source = ScriptedSource::new(vec![vec![ScriptItem::Pending]]);

    let sse_only =
        TransportFixture::new_with_modes(position(18, 0), vec![SubscriptionMode::ServerSentEvents])
            .await;
    let websocket_handle = DocumentTransportHandle::from_bytes(&[0x67; 16]).expect("handle");
    let mut websocket_document = DocumentTransportSession::new(
        origin.clone(),
        DocumentTransportKind::WebSocket,
        websocket_handle,
        DocumentTransportLimits::new(1).expect("limits"),
        sse_only.document_scope.clone(),
    );
    let websocket_control = WebSocketControlRecord::Subscribe(subscription(37));
    assert_eq!(
        websocket_subscribe(
            &mut websocket_document,
            &websocket_control,
            &source,
            sse_only.request(subscription(37), origin.clone()),
        )
        .await
        .expect_err("SSE-only registration cannot use WebSocket")
        .kind(),
        AsyncTransportErrorKind::TransportMismatch
    );

    let ws_only =
        TransportFixture::new_with_modes(position(18, 10), vec![SubscriptionMode::WebSocket]).await;
    let sse_handle = DocumentTransportHandle::from_bytes(&[0x68; 16]).expect("handle");
    let mut sse_document = DocumentTransportSession::new(
        origin.clone(),
        DocumentTransportKind::ServerSentEvents,
        sse_handle.clone(),
        DocumentTransportLimits::new(1).expect("limits"),
        ws_only.document_scope.clone(),
    );
    assert_eq!(
        sse_subscribe(
            &mut sse_document,
            &sse_handle,
            &origin,
            &source,
            ws_only.request(subscription(38), origin.clone()),
        )
        .await
        .expect_err("WebSocket-only registration cannot use SSE")
        .kind(),
        AsyncTransportErrorKind::TransportMismatch
    );

    let both = TransportFixture::new(position(18, 20)).await;
    let both_source = ScriptedSource::new(vec![vec![ScriptItem::Pending]]);
    let both_handle = DocumentTransportHandle::from_bytes(&[0x69; 16]).expect("handle");
    let mut both_document = DocumentTransportSession::new(
        origin.clone(),
        DocumentTransportKind::ServerSentEvents,
        both_handle.clone(),
        DocumentTransportLimits::new(1).expect("limits"),
        both.document_scope.clone(),
    );
    sse_subscribe(
        &mut both_document,
        &both_handle,
        &origin,
        &both_source,
        both.request(subscription(39), origin.clone()),
    )
    .await
    .expect("both-mode registration accepts SSE");
    both.registry.deny_unsubscribe();
    assert_eq!(
        both_document
            .remove(&both.request(subscription(39), origin))
            .await
            .expect_err("external remove must reauthorize")
            .kind(),
        AsyncTransportErrorKind::AuthorizationLost
    );
    assert_eq!(both_document.membership_count(), 1);
    assert_eq!(both_source.close_count(), 0);
    assert_eq!(
        both_document.close().await.expect("internal retirement"),
        CloseDisposition::Closed
    );
    assert_eq!(both_source.close_count(), 1);
}

#[tokio::test]
async fn sse_encodes_canonical_envelopes_heartbeats_headers_and_same_origin_control() {
    let fixture = TransportFixture::new(position(11, 20)).await;
    let origin = VerifiedOrigin::parse("https://app.example.test").expect("origin");
    let handle = DocumentTransportHandle::from_bytes(&[0x47; 16]).expect("handle");
    let authorization = fixture.request(subscription(13), origin.clone());
    let envelope = suprnova_live::async_updates::AsyncEnvelope::new(
        authorization.context(),
        position(11, 21),
        AsyncPayload::Heartbeat(Heartbeat),
    )
    .expect("envelope");

    let event = SseEncoder::encode_envelope(&envelope).expect("SSE event");
    assert_eq!(event.event(), "suprnova-live-async");
    assert!(!event.id().contains(['\r', '\n']));
    assert_eq!(
        event
            .as_bytes()
            .iter()
            .filter(|byte| **byte == b'\n')
            .count(),
        4
    );
    let decoded = decode_async_envelope(
        event.data(),
        &AsyncCodecLimits::v1(),
        authorization.context(),
    )
    .expect("canonical data");
    assert_eq!(decoded, envelope);
    assert_eq!(
        SseEncoder::heartbeat_comment(),
        b": suprnova-live heartbeat\n\n"
    );

    let headers = SseResponseContract::headers();
    assert_eq!(
        headers[http::header::CONTENT_TYPE],
        "text/event-stream; charset=utf-8"
    );
    assert_eq!(
        headers[http::header::CACHE_CONTROL],
        "no-store, no-transform"
    );
    assert_eq!(headers["x-accel-buffering"], "no");
    assert_eq!(headers[http::header::X_CONTENT_TYPE_OPTIONS], "nosniff");

    let source = ScriptedSource::new(vec![vec![ScriptItem::End]]);
    let mut wrong_transport = DocumentTransportSession::new(
        origin.clone(),
        DocumentTransportKind::WebSocket,
        handle.clone(),
        DocumentTransportLimits::new(1).expect("limits"),
        fixture.document_scope.clone(),
    );
    let rejected = sse_subscribe(
        &mut wrong_transport,
        &handle,
        &origin,
        &source,
        fixture.request(subscription(13), origin.clone()),
    )
    .await
    .expect_err("SSE membership cannot cross transport kind");
    assert_eq!(rejected.kind(), AsyncTransportErrorKind::TransportMismatch);

    let mut document = DocumentTransportSession::new(
        origin.clone(),
        DocumentTransportKind::ServerSentEvents,
        handle.clone(),
        DocumentTransportLimits::new(1).expect("limits"),
        fixture.document_scope.clone(),
    );
    sse_subscribe(&mut document, &handle, &origin, &source, authorization)
        .await
        .expect("same-origin control");
    assert_eq!(document.membership_count(), 1);

    let wrong_handle = DocumentTransportHandle::from_bytes(&[0x48; 16]).expect("handle");
    let rejected = sse_unsubscribe(
        &mut document,
        &wrong_handle,
        &origin,
        &fixture.request(subscription(13), origin.clone()),
    )
    .await
    .expect_err("correlation handle mismatch");
    assert_eq!(rejected.kind(), AsyncTransportErrorKind::RoutingMismatch);
    let wrong_origin = VerifiedOrigin::parse("https://other.example.test").expect("origin");
    let rejected = sse_unsubscribe(
        &mut document,
        &handle,
        &wrong_origin,
        &fixture.request(subscription(13), origin.clone()),
    )
    .await
    .expect_err("same-origin control is mandatory");
    assert_eq!(rejected.kind(), AsyncTransportErrorKind::OriginMismatch);
    assert_eq!(
        sse_unsubscribe(
            &mut document,
            &handle,
            &origin,
            &fixture.request(subscription(13), origin.clone()),
        )
        .await
        .expect("authorized removal"),
        CloseDisposition::Closed
    );
    assert_eq!(document.membership_count(), 0);
    document.close().await.expect("close");
}

#[tokio::test]
async fn sse_and_websocket_share_exact_ordered_envelope_semantics() {
    let fixture = TransportFixture::new(position(15, 50)).await;
    let origin = VerifiedOrigin::parse("https://app.example.test").expect("origin");
    let authorization = fixture.request(subscription(23), origin);
    let codec = WebSocketCodec::v1();
    let payloads = [
        AsyncPayload::Heartbeat(Heartbeat),
        AsyncPayload::Refresh(RegisteredRefresh),
        AsyncPayload::Complete(CompletionReason::StreamCompleted),
        AsyncPayload::Error(StreamErrorCode::ReplayUnavailable),
    ];

    for (offset, payload) in payloads.into_iter().enumerate() {
        let sequence = 51 + u64::try_from(offset).expect("small sequence offset");
        let envelope = AsyncEnvelope::new(authorization.context(), position(15, sequence), payload)
            .expect("bounded envelope");
        let sse = SseEncoder::encode_envelope(&envelope).expect("SSE record");
        let from_sse =
            decode_async_envelope(sse.data(), &AsyncCodecLimits::v1(), authorization.context())
                .expect("SSE canonical data");
        let websocket = codec.encode_envelope(&envelope).expect("WebSocket text");
        let from_websocket = codec
            .decode_envelope(
                WebSocketFrame::Text {
                    payload: &websocket,
                    final_fragment: true,
                },
                authorization.context(),
            )
            .expect("WebSocket canonical data");
        assert_eq!(from_sse, envelope);
        assert_eq!(from_websocket, envelope);
    }
}

#[test]
fn websocket_origin_policy_normalizes_exact_origins_and_runs_authentication_second() {
    let application = VerifiedOrigin::parse("https://APP.example.test:443").expect("origin");
    assert_eq!(application.to_string(), "https://app.example.test");
    assert_eq!(
        application,
        VerifiedOrigin::parse("https://app.example.test").expect("default port")
    );
    assert_eq!(
        VerifiedOrigin::parse("https://[2001:db8::1]:4443")
            .expect("IPv6")
            .to_string(),
        "https://[2001:db8::1]:4443"
    );
    VerifiedOrigin::parse("https://xn--bcher-kva.example").expect("serialized IDNA");
    for invalid in [
        "null",
        "*",
        "ftp://app.example.test",
        "https://user@app.example.test",
        "https://app.example.test/",
        "https://app.example.test?q=1",
        "https://app.example.test#fragment",
        "https://bücher.example",
        "https://[fe80::1%25eth0]",
    ] {
        assert!(
            VerifiedOrigin::parse(invalid).is_err(),
            "accepted {invalid}"
        );
    }

    let allowed = VerifiedOrigin::parse("https://embed.example.test:8443").expect("allowed");
    let policy =
        WebSocketOriginPolicy::new(application.clone(), vec![allowed.clone()]).expect("policy");
    for rejected_origins in [
        &[][..],
        &["null"][..],
        &["*"][..],
        &["https://app.example.test/path"][..],
        &["https://unapproved.example.test"][..],
        &["https://app.example.test", "https://app.example.test"][..],
    ] {
        let called = AtomicBool::new(false);
        let rejected = policy
            .authorize_upgrade::<()>(rejected_origins, || {
                called.store(true, Ordering::Release);
                Ok(WebSocketAuthentication::Cookie(()))
            })
            .expect_err("origin rejected before authentication");
        assert_eq!(rejected.kind(), AsyncTransportErrorKind::InvalidOrigin);
        assert!(!called.load(Ordering::Acquire));
    }

    let oversized_allowlist = (0..17)
        .map(|index| {
            VerifiedOrigin::parse(&format!("https://embed-{index}.example.test")).expect("origin")
        })
        .collect();
    assert!(WebSocketOriginPolicy::new(application.clone(), oversized_allowlist).is_err());

    let allowed_text = allowed.to_string();
    let cross_cookie = policy
        .authorize_upgrade(&[allowed_text.as_str()], || {
            Ok(WebSocketAuthentication::Cookie(()))
        })
        .expect_err("cross-origin cookie authority");
    assert_eq!(
        cross_cookie.kind(),
        AsyncTransportErrorKind::AuthorizationScopeMismatch
    );
    let cross = policy
        .authorize_upgrade(&[allowed_text.as_str()], || {
            Ok(WebSocketAuthentication::SeparateCredential(()))
        })
        .expect("separate cross-origin credential");
    assert!(cross.is_cross_origin());
    let application_text = application.to_string();
    let same = policy
        .authorize_upgrade(&[application_text.as_str()], || {
            Ok(WebSocketAuthentication::Cookie(()))
        })
        .expect("same-origin cookie");
    assert!(!same.is_cross_origin());
}

proptest! {
    #[test]
    fn arbitrary_origin_and_websocket_control_bytes_remain_closed_and_bounded(
        origin_bytes in proptest::collection::vec(any::<u8>(), 0..4_096),
        frame_bytes in proptest::collection::vec(any::<u8>(), 0..1_024),
    ) {
        let origin_text = String::from_utf8_lossy(&origin_bytes);
        let _ = VerifiedOrigin::parse(&origin_text);

        let _ = WebSocketCodec::v1().decode_control(WebSocketFrame::Text {
            payload: &frame_bytes,
            final_fragment: true,
        });
    }
}

#[tokio::test]
async fn websocket_codec_round_trips_envelopes_and_rejects_hostile_frames_and_membership() {
    let fixture = TransportFixture::new(position(12, 30)).await;
    let origin = VerifiedOrigin::parse("https://app.example.test").expect("origin");
    let authorization = fixture.request(subscription(14), origin.clone());
    let envelope = suprnova_live::async_updates::AsyncEnvelope::new(
        authorization.context(),
        position(12, 31),
        AsyncPayload::Refresh(RegisteredRefresh),
    )
    .expect("envelope");
    let codec = WebSocketCodec::v1();
    let encoded = codec.encode_envelope(&envelope).expect("text payload");
    let decoded = codec
        .decode_envelope(
            WebSocketFrame::Text {
                payload: &encoded,
                final_fragment: true,
            },
            authorization.context(),
        )
        .expect("decoded envelope");
    assert_eq!(decoded, envelope);

    for frame in [
        WebSocketFrame::Binary(&encoded),
        WebSocketFrame::Continuation(&encoded),
        WebSocketFrame::Text {
            payload: &encoded,
            final_fragment: false,
        },
        WebSocketFrame::Text {
            payload: &[0xff, 0xfe],
            final_fragment: true,
        },
    ] {
        assert!(
            codec
                .decode_envelope(frame, authorization.context())
                .is_err()
        );
    }
    assert_eq!(
        format!(
            "{:?}",
            WebSocketFrame::Text {
                payload: b"credential-sentinel",
                final_fragment: true,
            }
        ),
        "WebSocketFrame::Text { bytes: 19, final_fragment: true }"
    );
    let oversized_envelope = vec![b' '; 65_537];
    assert_eq!(
        codec
            .decode_envelope(
                WebSocketFrame::Text {
                    payload: &oversized_envelope,
                    final_fragment: true,
                },
                authorization.context(),
            )
            .expect_err("oversized envelope frame")
            .kind(),
        AsyncTransportErrorKind::FrameTooLarge
    );
    let invalid_utf8_at_envelope_limit = vec![0xff; 65_536];
    assert_eq!(
        codec
            .decode_envelope(
                WebSocketFrame::Text {
                    payload: &invalid_utf8_at_envelope_limit,
                    final_fragment: true,
                },
                authorization.context(),
            )
            .expect_err("invalid UTF-8 at the exact envelope limit")
            .kind(),
        AsyncTransportErrorKind::UnsupportedFrame
    );
    let oversized_invalid_utf8_envelope = vec![0xff; 65_537];
    assert_eq!(
        codec
            .decode_envelope(
                WebSocketFrame::Text {
                    payload: &oversized_invalid_utf8_envelope,
                    final_fragment: true,
                },
                authorization.context(),
            )
            .expect_err("size preflight precedes envelope UTF-8 validation")
            .kind(),
        AsyncTransportErrorKind::FrameTooLarge
    );
    for payload in [vec![b'a'; 1_048_576], vec![0xff; 1_048_576]] {
        assert_eq!(
            codec
                .decode_envelope(
                    WebSocketFrame::Text {
                        payload: &payload,
                        final_fragment: true,
                    },
                    authorization.context(),
                )
                .expect_err("very large envelope preflight")
                .kind(),
            AsyncTransportErrorKind::FrameTooLarge
        );
    }
    assert_eq!(
        codec
            .decode_envelope(
                WebSocketFrame::Text {
                    payload: &[0xff],
                    final_fragment: false,
                },
                authorization.context(),
            )
            .expect_err("fragment shape precedes UTF-8 validation")
            .kind(),
        AsyncTransportErrorKind::UnsupportedFrame
    );

    let source = ScriptedSource::new(vec![vec![ScriptItem::Pending]]);
    let initial_subscribe = WebSocketControlRecord::Subscribe(subscription(14));
    let mut wrong_transport = DocumentTransportSession::new(
        origin.clone(),
        DocumentTransportKind::ServerSentEvents,
        DocumentTransportHandle::from_bytes(&[0x56; 16]).expect("handle"),
        DocumentTransportLimits::new(2).expect("limits"),
        fixture.document_scope.clone(),
    );
    let rejected = websocket_subscribe(
        &mut wrong_transport,
        &initial_subscribe,
        &source,
        fixture.request(subscription(14), origin.clone()),
    )
    .await
    .expect_err("WebSocket membership cannot cross transport kind");
    assert_eq!(rejected.kind(), AsyncTransportErrorKind::TransportMismatch);

    let mut document = DocumentTransportSession::new(
        origin,
        DocumentTransportKind::WebSocket,
        DocumentTransportHandle::from_bytes(&[0x49; 16]).expect("handle"),
        DocumentTransportLimits::new(2).expect("limits"),
        fixture.document_scope.clone(),
    );
    websocket_subscribe(&mut document, &initial_subscribe, &source, authorization)
        .await
        .expect("authenticated membership");

    let subscribe = WebSocketControlRecord::Subscribe(subscription(15));
    let subscribe_bytes = codec.encode_control(&subscribe).expect("subscribe frame");
    assert_eq!(
        codec
            .decode_control(WebSocketFrame::Text {
                payload: &subscribe_bytes,
                final_fragment: true,
            })
            .expect("new subscribe"),
        subscribe
    );
    let unknown_unsubscribe = codec
        .decode_control(WebSocketFrame::Text {
            payload: br#"{"kind":"unsubscribe","subscription":"EREREREREREREREREREREQ"}"#,
            final_fragment: true,
        })
        .expect("syntactically valid unsubscribe remains state-blind");
    assert_eq!(
        unknown_unsubscribe,
        WebSocketControlRecord::Unsubscribe(subscription(17))
    );
    for hostile in [
        br#"{"kind":"subscribe","kind":"unsubscribe","subscription":"Dw8PDw8PDw8PDw8PDw8PDw"}"#
            .as_slice(),
        br#"{"extra":1,"kind":"subscribe","subscription":"Dw8PDw8PDw8PDw8PDw8PDw"}"#.as_slice(),
        br#"{ "kind":"subscribe","subscription":"Dw8PDw8PDw8PDw8PDw8PDw"}"#.as_slice(),
    ] {
        assert!(
            codec
                .decode_control(WebSocketFrame::Text {
                    payload: hostile,
                    final_fragment: true,
                })
                .is_err()
        );
    }
    let oversized_control = vec![b' '; 513];
    assert_eq!(
        codec
            .decode_control(WebSocketFrame::Text {
                payload: &oversized_control,
                final_fragment: true,
            })
            .expect_err("oversized control frame")
            .kind(),
        AsyncTransportErrorKind::FrameTooLarge
    );
    let invalid_utf8_at_control_limit = vec![0xff; 512];
    assert_eq!(
        codec
            .decode_control(WebSocketFrame::Text {
                payload: &invalid_utf8_at_control_limit,
                final_fragment: true,
            })
            .expect_err("invalid UTF-8 at the exact control limit")
            .kind(),
        AsyncTransportErrorKind::UnsupportedFrame
    );
    let oversized_invalid_utf8_control = vec![0xff; 513];
    assert_eq!(
        codec
            .decode_control(WebSocketFrame::Text {
                payload: &oversized_invalid_utf8_control,
                final_fragment: true,
            })
            .expect_err("size preflight precedes control UTF-8 validation")
            .kind(),
        AsyncTransportErrorKind::FrameTooLarge
    );
    for payload in [vec![b'a'; 1_048_576], vec![0xff; 1_048_576]] {
        assert_eq!(
            codec
                .decode_control(WebSocketFrame::Text {
                    payload: &payload,
                    final_fragment: true,
                })
                .expect_err("very large control preflight")
                .kind(),
            AsyncTransportErrorKind::FrameTooLarge
        );
    }
    let forged_control = WebSocketControlRecord::Unsubscribe(subscription(16));
    let forged = websocket_unsubscribe(
        &mut document,
        &forged_control,
        &fixture.request(
            subscription(14),
            VerifiedOrigin::parse("https://app.example.test").expect("origin"),
        ),
    )
    .await
    .expect_err("control identity cannot replace signed membership authority");
    assert_eq!(forged.kind(), AsyncTransportErrorKind::RoutingMismatch);

    let unsubscribe = WebSocketControlRecord::Unsubscribe(subscription(14));
    let unsubscribe_bytes = codec
        .encode_control(&unsubscribe)
        .expect("unsubscribe frame");
    let decoded_unsubscribe = codec
        .decode_control(WebSocketFrame::Text {
            payload: &unsubscribe_bytes,
            final_fragment: true,
        })
        .expect("current unsubscribe");
    assert_eq!(
        websocket_unsubscribe(
            &mut document,
            &decoded_unsubscribe,
            &fixture.request(
                subscription(14),
                VerifiedOrigin::parse("https://app.example.test").expect("origin"),
            ),
        )
        .await
        .expect("authenticated unsubscribe"),
        CloseDisposition::Closed
    );
    assert_eq!(document.membership_count(), 0);
    document.close().await.expect("close");
}
