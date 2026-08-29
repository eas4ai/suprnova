//! Iteration 004 cross-feature adversarial regression matrix.

use std::cell::Cell;

use suprnova_live::async_updates::{
    AsyncTransportErrorKind, WebSocketAuthentication, WebSocketCodec, WebSocketFrame,
    WebSocketOriginPolicy,
};
use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
use suprnova_live::host::{
    HostScopeFacts, PrincipalFingerprint, SessionFingerprint, TenantFingerprint,
};
use suprnova_live::identity::{ComponentName, KeyId, ModelField, ScopeFingerprint, UnixMillis};
use suprnova_live::upload::{
    MediaHeaderProbe, TransferGrant, TransferGrantCodec, TransferGrantRequest, TransferGrantScope,
    TransitionDisposition, UploadErrorKind, UploadHandle, UploadIdempotencyKey, UploadRevision,
    UploadState, UploadStateMachine, UploadTransition, UploadTransitionRequest,
};

#[allow(
    dead_code,
    reason = "the shared deterministic transport fixture exposes controls used by sibling suites"
)]
#[path = "support/async_transport.rs"]
mod support;

const HANDLE: &str = "018f47c1-2af0-7cc4-a001-000000000001";
const OTHER_HANDLE: &str = "018f47c1-2af0-7cc4-a001-000000000002";
const ROOT_SENTINEL: &[u8; 32] = b"task4-upload-secret-never-leak-1";
const MAX_WEBSOCKET_ENVELOPE_BYTES: usize = 65_536;

fn fingerprint(byte: u8) -> [u8; 32] {
    [byte; 32]
}

fn host_scope(scope: u8, session: u8, principal: u8, tenant: u8) -> HostScopeFacts {
    HostScopeFacts::new(
        ScopeFingerprint::from_bytes(&fingerprint(scope)).expect("scope"),
        Some(SessionFingerprint::from_bytes(&fingerprint(session)).expect("session")),
        Some(PrincipalFingerprint::from_bytes(&fingerprint(principal)).expect("principal")),
        Some(TenantFingerprint::from_bytes(&fingerprint(tenant)).expect("tenant")),
    )
}

fn grant_codec() -> TransferGrantCodec {
    let active = KeyRecord::new(
        KeyId::parse("task4-upload-key").expect("key id"),
        RootKey::new(ROOT_SENTINEL.to_vec()).expect("root key"),
        UnixMillis::new(0),
        UnixMillis::new(50_000),
        UnixMillis::new(100_000),
    )
    .expect("key record");
    TransferGrantCodec::new(SnapshotKeyRing::new(active, Vec::new()).expect("key ring"))
}

fn upload_scope(handle: &str, scope: HostScopeFacts) -> TransferGrantScope {
    TransferGrantScope::new(
        UploadHandle::parse(handle).expect("handle"),
        ComponentName::parse("profile.edit").expect("component"),
        ModelField::parse("avatar").expect("field"),
        scope,
        1,
    )
}

#[test]
fn forged_and_cross_scope_upload_authority_is_typed_bounded_and_redacted() {
    let codec = grant_codec();
    let authority = upload_scope(HANDLE, host_scope(1, 2, 3, 4));
    let issued = codec
        .issue(
            TransferGrantRequest::new(authority.clone(), UnixMillis::new(20_000)),
            UnixMillis::new(1_000),
        )
        .expect("issue grant");
    let grant = TransferGrant::parse(issued.grant().expose_bearer()).expect("grant round trip");
    let forged_handle = upload_scope(OTHER_HANDLE, authority.host_scope().clone());
    let cross_scope = upload_scope(HANDLE, host_scope(9, 2, 3, 4));

    for (name, wrong, expected) in [
        (
            "forged_handle",
            forged_handle,
            UploadErrorKind::ScopeMismatch,
        ),
        ("cross_scope", cross_scope, UploadErrorKind::ScopeMismatch),
    ] {
        let error = codec
            .verify(&grant, &wrong, UnixMillis::new(1_001))
            .expect_err(name);
        assert_eq!(error.kind(), expected, "{name}");
        let diagnostic = format!("{error:?}:{error}");
        assert!(
            diagnostic.len() <= 256,
            "{name} diagnostic must stay bounded"
        );
        assert!(
            !diagnostic.contains(issued.grant().expose_bearer()),
            "{name}"
        );
        assert!(
            !diagnostic.contains(String::from_utf8_lossy(ROOT_SENTINEL).as_ref()),
            "{name}"
        );
    }

    let mut forged = grant.expose_bearer().as_bytes().to_vec();
    let last = forged.last_mut().expect("non-empty grant");
    *last = if *last == b'A' { b'B' } else { b'A' };
    let forged = TransferGrant::parse(std::str::from_utf8(&forged).expect("ASCII grant"))
        .expect("syntactically valid forged grant");
    let error = codec
        .verify(&forged, &authority, UnixMillis::new(1_001))
        .expect_err("forged signature");
    assert_eq!(error.kind(), UploadErrorKind::InvalidGrant);
    assert!(!format!("{error:?}").contains(forged.expose_bearer()));

    assert!(
        codec
            .verify(&grant, &authority, UnixMillis::new(1_001))
            .is_ok(),
        "an unrelated valid authority remains usable after hostile attempts"
    );
}

#[test]
fn malformed_media_headers_have_one_bounded_fail_closed_disposition() {
    let cases: [(&str, &[u8]); 6] = [
        ("truncated_png", b"\x89PNG\r\n\x1a\n"),
        (
            "zero_png_width",
            b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\0\0\0\0\x01",
        ),
        ("truncated_gif", b"GIF89a\xff"),
        ("oversized_webp_claim", b"RIFF\xff\xff\xff\xffWEBPVP8X"),
        ("truncated_jpeg_segment", b"\xff\xd8\xff\xe1\xff\xff"),
        ("truncated_webp", b"RIFF\xff\xff\xff\xffWEBPVP8 "),
    ];

    for (name, header) in cases {
        let error = MediaHeaderProbe::probe(header).expect_err(name);
        assert_eq!(error.kind(), UploadErrorKind::MediaHeaderUnproved, "{name}");
        assert!(format!("{error:?}:{error}").len() <= 256, "{name}");
    }

    let valid_png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0";
    assert!(MediaHeaderProbe::probe(valid_png).is_ok());
}

fn transition_request(
    revision: u64,
    key: &str,
    transition: UploadTransition,
) -> UploadTransitionRequest {
    UploadTransitionRequest::new(
        UploadHandle::parse(HANDLE).expect("handle"),
        UploadRevision::new(revision),
        UploadIdempotencyKey::parse(key).expect("idempotency key"),
        transition,
    )
}

#[test]
fn duplicate_completion_cancel_finalize_and_expire_finalize_are_monotonic() {
    let mut completing = UploadStateMachine::new(
        UploadHandle::parse(HANDLE).expect("handle"),
        UploadState::Transferring,
        UploadRevision::new(7),
    );
    let complete = transition_request(7, "complete-once", UploadTransition::Complete);
    let first = completing
        .apply(complete.clone())
        .expect("first completion");
    let replay = completing.apply(complete).expect("duplicate completion");
    assert_eq!(first.disposition(), TransitionDisposition::Applied);
    assert_eq!(replay.disposition(), TransitionDisposition::ExistingOutcome);
    assert_eq!(completing.state(), UploadState::Verifying);
    assert_eq!(completing.revision(), UploadRevision::new(8));

    for (name, terminal) in [
        ("cancel_finalize", UploadTransition::Cancel),
        ("expire_finalize", UploadTransition::Expire),
    ] {
        let mut machine = UploadStateMachine::new(
            UploadHandle::parse(HANDLE).expect("handle"),
            UploadState::Ready,
            UploadRevision::new(8),
        );
        machine
            .apply(transition_request(8, name, terminal))
            .expect("terminal transition wins");
        let terminal_state = machine.state();
        let error = machine
            .apply(transition_request(
                8,
                &format!("{name}-finalize"),
                UploadTransition::BeginFinalize,
            ))
            .expect_err("stale finalize must not resurrect terminal upload");
        assert_eq!(error.kind(), UploadErrorKind::UploadConflict, "{name}");
        assert_eq!(machine.state(), terminal_state, "{name}");
        assert!(machine.state().is_terminal(), "{name}");
        assert_eq!(machine.revision(), UploadRevision::new(9), "{name}");
    }
}

#[test]
fn websocket_origin_matrix_rejects_before_authority_and_cross_site_cookie_use() {
    let application = suprnova_live::async_updates::VerifiedOrigin::parse("https://app.test")
        .expect("application origin");
    let allowed = suprnova_live::async_updates::VerifiedOrigin::parse("https://allowed.test")
        .expect("allowed origin");
    let policy = WebSocketOriginPolicy::new(application, vec![allowed]).expect("origin policy");

    for (name, origins) in [
        ("missing", vec![]),
        ("null", vec!["null"]),
        ("wildcard", vec!["*"]),
        ("malformed", vec!["https:// app.test"]),
        ("unapproved", vec!["https://evil.test"]),
        ("duplicate", vec!["https://app.test", "https://app.test"]),
    ] {
        let consulted = Cell::new(false);
        let error = policy
            .authorize_upgrade(&origins, || {
                consulted.set(true);
                Ok(WebSocketAuthentication::Cookie("session-secret-sentinel"))
            })
            .expect_err(name);
        assert_eq!(
            error.kind(),
            AsyncTransportErrorKind::InvalidOrigin,
            "{name}"
        );
        assert!(!consulted.get(), "{name} must reject before authentication");
        assert!(!format!("{error:?}:{error}").contains("session-secret-sentinel"));
    }

    let error = policy
        .authorize_upgrade(&["https://allowed.test"], || {
            Ok(WebSocketAuthentication::Cookie("session-secret-sentinel"))
        })
        .expect_err("cross-site cookie authority");
    assert_eq!(
        error.kind(),
        AsyncTransportErrorKind::AuthorizationScopeMismatch
    );
    assert!(!format!("{error:?}:{error}").contains("session-secret-sentinel"));

    assert!(
        policy
            .authorize_upgrade(&["https://app.test"], || {
                Ok(WebSocketAuthentication::Cookie(()))
            })
            .is_ok(),
        "ordinary same-origin transport remains usable"
    );
}

#[tokio::test]
async fn oversized_truncated_and_fragmented_messages_are_typed_and_bounded() {
    let fixture = support::TransportFixture::new(support::position(7, 0)).await;
    let context = fixture
        .request(
            support::subscription(0x91),
            suprnova_live::async_updates::VerifiedOrigin::parse("https://example.test")
                .expect("origin"),
        )
        .context()
        .clone();
    let oversized = vec![b'x'; MAX_WEBSOCKET_ENVELOPE_BYTES + 1];
    let cases = [
        (
            "oversized",
            WebSocketFrame::Text {
                payload: oversized.as_slice(),
                final_fragment: true,
            },
            AsyncTransportErrorKind::FrameTooLarge,
        ),
        (
            "truncated",
            WebSocketFrame::Text {
                payload: br#"{"async_protocol":1,"#,
                final_fragment: true,
            },
            AsyncTransportErrorKind::InvalidEnvelope,
        ),
        (
            "non_final",
            WebSocketFrame::Text {
                payload: b"{}",
                final_fragment: false,
            },
            AsyncTransportErrorKind::UnsupportedFrame,
        ),
        (
            "binary",
            WebSocketFrame::Binary(b"{}"),
            AsyncTransportErrorKind::UnsupportedFrame,
        ),
        (
            "continuation",
            WebSocketFrame::Continuation(b"{}"),
            AsyncTransportErrorKind::UnsupportedFrame,
        ),
    ];

    for (name, frame, expected) in cases {
        let error = WebSocketCodec::v1()
            .decode_envelope(frame, &context)
            .expect_err(name);
        assert_eq!(error.kind(), expected, "{name}");
        assert!(format!("{error:?}:{error}").len() <= 256, "{name}");
    }

    let valid = suprnova_live::async_updates::AsyncEnvelope::new(
        &context,
        support::position(7, 1),
        suprnova_live::async_updates::AsyncPayload::Heartbeat(
            suprnova_live::async_updates::Heartbeat,
        ),
    )
    .expect("valid envelope");
    let encoded = WebSocketCodec::v1()
        .encode_envelope(&valid)
        .expect("encode valid envelope");
    assert!(
        WebSocketCodec::v1()
            .decode_envelope(
                WebSocketFrame::Text {
                    payload: &encoded,
                    final_fragment: true,
                },
                &context,
            )
            .is_ok(),
        "valid delivery remains usable after hostile messages"
    );
}
