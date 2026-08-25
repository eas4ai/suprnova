//! Shared transport conformance for multiplexed asynchronous sessions.

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll, Waker};

use proptest::prelude::*;
use suprnova_live::async_updates::{
    AsyncCodecLimits, AsyncDispatchError, AsyncEnvelope, AsyncEnvelopeDispatchPort,
    AsyncEventSession, AsyncEventSource, AsyncPayload, AsyncTransportErrorKind,
    AsyncTransportFuture, CloseDisposition, CompletionReason, DocumentTransportHandle,
    DocumentTransportKind, DocumentTransportLimits, DocumentTransportSession, Heartbeat,
    MAX_DOCUMENT_TRANSPORT_MEMBERSHIPS, RegisteredRefresh, SequenceDegradation,
    SequenceDisposition, SequenceMachine, SseEncoder, SseMembershipControl, SseResponseContract,
    StreamErrorCode, VerifiedOrigin, WebSocketAuthentication, WebSocketCodec,
    WebSocketControlRecord, WebSocketFrame, WebSocketMembershipControl, WebSocketOriginPolicy,
    decode_async_envelope,
};
use suprnova_live::identity::UnixMillis;

#[path = "support/async_transport.rs"]
mod support;

use support::{ScriptItem, ScriptedSource, TransportFixture, position, subscription};

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
