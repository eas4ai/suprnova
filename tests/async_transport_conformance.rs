//! Shared transport conformance for multiplexed asynchronous sessions.

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll, Waker};

use proptest::prelude::*;
use suprnova_live::async_updates::{
    AsyncCodecLimits, AsyncDispatchError, AsyncEnvelope, AsyncEnvelopeDispatchPort,
    AsyncEventSession, AsyncEventSource, AsyncPayload, AsyncTransportErrorKind,
    AsyncTransportFuture, AuthorizedTransportSubscription, CloseDisposition, CompletionReason,
    DocumentTransportHandle, DocumentTransportKind, DocumentTransportLimits,
    DocumentTransportSession, Heartbeat, MAX_DOCUMENT_TRANSPORT_MEMBERSHIPS, RegisteredRefresh,
    SequenceDegradation, SequenceDisposition, SequenceMachine, SseEncoder, SseMembershipControl,
    SseResponseContract, StreamErrorCode, SubscriptionMode, VerifiedOrigin,
    WebSocketAuthentication, WebSocketCodec, WebSocketControlRecord, WebSocketFrame,
    WebSocketMembershipControl, WebSocketOriginPolicy, decode_async_envelope,
};
use suprnova_live::identity::UnixMillis;

#[path = "support/async_transport.rs"]
mod support;

use support::{
    ControlledSubscribeSource, ScriptItem, ScriptedSource, TransportFixture, position, subscription,
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
    assert_eq!(document.membership_count(), 1);
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
        match self.adapter {
            AdapterKind::Sse => {
                SseMembershipControl::subscribe(
                    &mut self.document,
                    &self.handle,
                    &self.origin,
                    source,
                    authorization,
                )
                .await
            }
            AdapterKind::WebSocket => {
                let control =
                    WebSocketControlRecord::Subscribe(authorization.subscription().clone());
                let encoded = self.websocket.encode_control(&control)?;
                let decoded = self.websocket.decode_control(
                    WebSocketFrame::Text {
                        payload: &encoded,
                        final_fragment: true,
                    },
                    &self.document,
                )?;
                WebSocketMembershipControl::subscribe(
                    &mut self.document,
                    &decoded,
                    source,
                    authorization,
                )
                .await
            }
        }
    }

    async fn unsubscribe(
        &mut self,
        authorization: &AuthorizedTransportSubscription,
        coverage: &mut AdapterCoverage,
    ) -> Result<CloseDisposition, suprnova_live::async_updates::AsyncTransportError> {
        coverage.unsubscribe_controls += 1;
        match self.adapter {
            AdapterKind::Sse => {
                SseMembershipControl::unsubscribe(
                    &mut self.document,
                    &self.handle,
                    &self.origin,
                    authorization,
                )
                .await
            }
            AdapterKind::WebSocket => {
                let control =
                    WebSocketControlRecord::Unsubscribe(authorization.subscription().clone());
                let encoded = self.websocket.encode_control(&control)?;
                let decoded = self.websocket.decode_control(
                    WebSocketFrame::Text {
                        payload: &encoded,
                        final_fragment: true,
                    },
                    &self.document,
                )?;
                WebSocketMembershipControl::unsubscribe(&mut self.document, &decoded, authorization)
                    .await
            }
        }
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

    // Signed baseline, ordered replay, heartbeat, completion, and typed wire error.
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
            AsyncPayload::Complete(CompletionReason::StreamCompleted),
        ),
        ScriptItem::Envelope(
            position(30, 43),
            AsyncPayload::Error(StreamErrorCode::ReplayUnavailable),
        ),
        ScriptItem::End,
    ]]);
    let mut harness = AdapterHarness::connect(adapter, 2, 0x70, &mut coverage);
    harness
        .subscribe(&source, authorization, &mut coverage)
        .await
        .expect("adapter baseline connect");
    let first = harness
        .next_wire(&context, &mut coverage)
        .await
        .expect("heartbeat")
        .expect("heartbeat envelope");
    let second = harness
        .next_wire(&context, &mut coverage)
        .await
        .expect("completion")
        .expect("completion envelope");
    let third = harness
        .next_wire(&context, &mut coverage)
        .await
        .expect("typed error")
        .expect("typed error envelope");
    assert_eq!(first.position(), position(30, 41));
    assert!(matches!(first.payload(), AsyncPayload::Heartbeat(_)));
    assert!(matches!(second.payload(), AsyncPayload::Complete(_)));
    assert!(matches!(third.payload(), AsyncPayload::Error(_)));
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
    let mut harness = AdapterHarness::connect(adapter, 1, 0x71, &mut coverage);
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
    let mut harness = AdapterHarness::connect(adapter, 1, 0x72, &mut coverage);
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
    let mut harness = AdapterHarness::connect(adapter, 1, 0x73, &mut coverage);
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
    let mut harness = AdapterHarness::connect(adapter, 1, 0x74, &mut coverage);
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
    let mut harness = AdapterHarness::connect(adapter, 1, 0x75, &mut coverage);
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
    let mut harness = AdapterHarness::connect(adapter, 2, 0x76, &mut coverage);
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
    let mut harness = AdapterHarness::connect(adapter, 1, 0x77, &mut coverage);
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
    let mut harness = AdapterHarness::connect(adapter, 1, 0x78, &mut coverage);
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

    // Cancelled shutdown retains authority; resumed close is idempotent.
    coverage.case();
    let fixture = TransportFixture::new(position(39, 0)).await;
    let source = ScriptedSource::new(vec![vec![ScriptItem::Pending]]).with_pending_first_close();
    let mut harness = AdapterHarness::connect(adapter, 1, 0x79, &mut coverage);
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
        assert_eq!(coverage.semantic_cases, 10, "{coverage:?}");
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

    let duplicate_snapshot_fixture = TransportFixture::new(position(16, 30)).await;
    duplicate_snapshot_fixture.registry.accept_twice();
    let duplicate_snapshot_source = ScriptedSource::new(vec![vec![ScriptItem::End]]);
    let mut duplicate_snapshot_document = DocumentTransportSession::new(
        VerifiedOrigin::parse("https://app.example.test").expect("origin"),
        DocumentTransportKind::ServerSentEvents,
        DocumentTransportHandle::from_bytes(&[0x6a; 16]).expect("handle"),
        DocumentTransportLimits::new(1).expect("limits"),
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
    );
    let mut mode_add = Box::pin(SseMembershipControl::subscribe(
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
    );
    let websocket_subscribe = WebSocketControlRecord::Subscribe(subscription(37));
    assert_eq!(
        WebSocketMembershipControl::subscribe(
            &mut websocket_document,
            &websocket_subscribe,
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
    );
    assert_eq!(
        SseMembershipControl::subscribe(
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
    );
    SseMembershipControl::subscribe(
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
    );
    let rejected = SseMembershipControl::subscribe(
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
    );
    SseMembershipControl::subscribe(&mut document, &handle, &origin, &source, authorization)
        .await
        .expect("same-origin control");
    assert_eq!(document.membership_count(), 1);

    let wrong_handle = DocumentTransportHandle::from_bytes(&[0x48; 16]).expect("handle");
    let rejected = SseMembershipControl::unsubscribe(
        &mut document,
        &wrong_handle,
        &origin,
        &fixture.request(subscription(13), origin.clone()),
    )
    .await
    .expect_err("correlation handle mismatch");
    assert_eq!(rejected.kind(), AsyncTransportErrorKind::RoutingMismatch);
    let wrong_origin = VerifiedOrigin::parse("https://other.example.test").expect("origin");
    let rejected = SseMembershipControl::unsubscribe(
        &mut document,
        &handle,
        &wrong_origin,
        &fixture.request(subscription(13), origin.clone()),
    )
    .await
    .expect_err("same-origin control is mandatory");
    assert_eq!(rejected.kind(), AsyncTransportErrorKind::OriginMismatch);
    assert_eq!(
        SseMembershipControl::unsubscribe(
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

        let application = VerifiedOrigin::parse("https://app.example.test").expect("origin");
        let document = DocumentTransportSession::new(
            application,
            DocumentTransportKind::WebSocket,
            DocumentTransportHandle::from_bytes(&[0x54; 16]).expect("handle"),
            DocumentTransportLimits::new(1).expect("limits"),
        );
        let _ = WebSocketCodec::v1().decode_control(
            WebSocketFrame::Text {
                payload: &frame_bytes,
                final_fragment: true,
            },
            &document,
        );
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
    );
    let rejected = WebSocketMembershipControl::subscribe(
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
    );
    WebSocketMembershipControl::subscribe(
        &mut document,
        &initial_subscribe,
        &source,
        authorization,
    )
    .await
    .expect("authenticated membership");

    let subscribe = WebSocketControlRecord::Subscribe(subscription(15));
    let subscribe_bytes = codec.encode_control(&subscribe).expect("subscribe frame");
    assert_eq!(
        codec
            .decode_control(
                WebSocketFrame::Text {
                    payload: &subscribe_bytes,
                    final_fragment: true,
                },
                &document,
            )
            .expect("new subscribe"),
        subscribe
    );
    let unknown_unsubscribe = codec
        .decode_control(
            WebSocketFrame::Text {
                payload: br#"{"kind":"unsubscribe","subscription":"EREREREREREREREREREREQ"}"#,
                final_fragment: true,
            },
            &document,
        )
        .expect_err("unknown unsubscribe");
    assert_eq!(
        unknown_unsubscribe.kind(),
        AsyncTransportErrorKind::UnknownMembership
    );
    for hostile in [
        br#"{"kind":"subscribe","kind":"unsubscribe","subscription":"Dw8PDw8PDw8PDw8PDw8PDw"}"#
            .as_slice(),
        br#"{"extra":1,"kind":"subscribe","subscription":"Dw8PDw8PDw8PDw8PDw8PDw"}"#.as_slice(),
        br#"{ "kind":"subscribe","subscription":"Dw8PDw8PDw8PDw8PDw8PDw"}"#.as_slice(),
    ] {
        assert!(
            codec
                .decode_control(
                    WebSocketFrame::Text {
                        payload: hostile,
                        final_fragment: true,
                    },
                    &document,
                )
                .is_err()
        );
    }
    let oversized_control = vec![b' '; 513];
    assert_eq!(
        codec
            .decode_control(
                WebSocketFrame::Text {
                    payload: &oversized_control,
                    final_fragment: true,
                },
                &document,
            )
            .expect_err("oversized control frame")
            .kind(),
        AsyncTransportErrorKind::FrameTooLarge
    );
    let invalid_utf8_at_control_limit = vec![0xff; 512];
    assert_eq!(
        codec
            .decode_control(
                WebSocketFrame::Text {
                    payload: &invalid_utf8_at_control_limit,
                    final_fragment: true,
                },
                &document,
            )
            .expect_err("invalid UTF-8 at the exact control limit")
            .kind(),
        AsyncTransportErrorKind::UnsupportedFrame
    );
    let oversized_invalid_utf8_control = vec![0xff; 513];
    assert_eq!(
        codec
            .decode_control(
                WebSocketFrame::Text {
                    payload: &oversized_invalid_utf8_control,
                    final_fragment: true,
                },
                &document,
            )
            .expect_err("size preflight precedes control UTF-8 validation")
            .kind(),
        AsyncTransportErrorKind::FrameTooLarge
    );
    for payload in [vec![b'a'; 1_048_576], vec![0xff; 1_048_576]] {
        assert_eq!(
            codec
                .decode_control(
                    WebSocketFrame::Text {
                        payload: &payload,
                        final_fragment: true,
                    },
                    &document,
                )
                .expect_err("very large control preflight")
                .kind(),
            AsyncTransportErrorKind::FrameTooLarge
        );
    }
    let forged_control = WebSocketControlRecord::Unsubscribe(subscription(16));
    let forged = WebSocketMembershipControl::unsubscribe(
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
        .decode_control(
            WebSocketFrame::Text {
                payload: &unsubscribe_bytes,
                final_fragment: true,
            },
            &document,
        )
        .expect("current unsubscribe");
    assert_eq!(
        WebSocketMembershipControl::unsubscribe(
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
