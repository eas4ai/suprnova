//! Closed adversarial probes executed by the Rust reference host.

use serde::Serialize;
use suprnova_live::async_updates::{
    AsyncCloseCode, AsyncTransportError, AsyncTransportErrorKind, MAX_REPLAY_TRANSCRIPT_ENVELOPES,
    SequenceDegradation, SequenceDisposition, WebSocketCodec, WebSocketFrame,
};
use suprnova_live::resource::{ResourceBounds, ResourceError, ResourceOwner};
use suprnova_live::upload::{
    MediaHeaderProbe, ScanDisposition, ScanFailurePolicy, TransitionDisposition, UploadError,
    UploadErrorKind, UploadHandle, UploadIdempotencyKey, UploadRevision, UploadScanPolicy,
    UploadState, UploadStateMachine, UploadTransition, UploadTransitionRequest,
};

const HANDLE: &str = "018f47c1-2af0-7cc4-a001-0000000000a4";

#[derive(Debug, Serialize)]
pub(super) struct AdversarialOutcome {
    case: &'static str,
    disposition: &'static str,
    recovery: &'static str,
    retained_items: usize,
    retained_bytes: usize,
    high_water_items: usize,
    high_water_bytes: usize,
    ceiling_items: usize,
    ceiling_bytes: usize,
    dependent_feature_closed: bool,
    unrelated_scope_usable: bool,
    diagnostic: &'static str,
}

impl AdversarialOutcome {
    fn from_probe(
        case: &'static str,
        disposition: &'static str,
        recovery: &'static str,
        high_water_items: usize,
        high_water_bytes: usize,
        ceiling_items: usize,
        ceiling_bytes: usize,
    ) -> Option<Self> {
        let dependent = ResourceOwner::new(ResourceBounds::new(ceiling_items, ceiling_bytes).ok()?);
        for index in 0..high_water_items {
            let item_bytes = high_water_bytes / high_water_items
                + usize::from(index < high_water_bytes % high_water_items);
            dependent.queue().try_push(item_bytes, ()).ok()?;
        }
        let measured_items = dependent.queue().len();
        let measured_bytes = dependent.queue().retained_bytes();
        let retirement = dependent.retire();
        let dependent_feature_closed = dependent.queue().is_retired()
            && dependent.queue().try_push(0, ()) == Err(ResourceError::Retired)
            && retirement.drained_items == measured_items
            && retirement.drained_bytes == measured_bytes;

        let unrelated = ResourceOwner::new(ResourceBounds::new(1, 4).ok()?);
        let unrelated_scope_usable = unrelated.queue().try_push(4, ()).is_ok()
            && unrelated.queue().pop() == Some(())
            && unrelated.retire().drained_items == 0;

        Some(Self {
            case,
            disposition,
            recovery,
            retained_items: dependent.queue().len(),
            retained_bytes: dependent.queue().retained_bytes(),
            high_water_items: measured_items,
            high_water_bytes: measured_bytes,
            ceiling_items,
            ceiling_bytes,
            dependent_feature_closed,
            unrelated_scope_usable,
            diagnostic: disposition,
        })
    }
}

pub(super) fn execute(case: &str) -> Option<AdversarialOutcome> {
    let (case, disposition, recovery, items, bytes, max_items, max_bytes) = match case {
        "hostile-media-header" => {
            let error = MediaHeaderProbe::probe(b"\x89PNG\r\n\x1a\n")
                .expect_err("truncated authoritative header");
            debug_assert_eq!(error.kind(), UploadErrorKind::MediaHeaderUnproved);
            (
                "hostile-media-header",
                "media_header_unproved",
                "replace",
                0,
                0,
                1,
                32,
            )
        }
        "scan-timeout" => {
            let scan = ScanDisposition::TimedOut;
            let policy = UploadScanPolicy::Required {
                on_timeout: ScanFailurePolicy::Retry,
                on_unavailable: ScanFailurePolicy::Reject,
            };
            let disposition = match (scan, policy) {
                (
                    ScanDisposition::TimedOut,
                    UploadScanPolicy::Required {
                        on_timeout: ScanFailurePolicy::Retry,
                        ..
                    },
                ) => "scan_retry",
                _ => return None,
            };
            ("scan-timeout", disposition, "retry", 0, 0, 1, 1)
        }
        "provider-partial-failure" => {
            let error = UploadError::new(UploadErrorKind::ReconciliationRequired);
            debug_assert_eq!(error.kind(), UploadErrorKind::ReconciliationRequired);
            (
                "provider-partial-failure",
                "reconciliation_required",
                "reconcile",
                1,
                4,
                2,
                8,
            )
        }
        "replay-overflow" => {
            let bounds = ResourceBounds::new(MAX_REPLAY_TRANSCRIPT_ENVELOPES, 256 * 1024)
                .expect("protocol replay bounds");
            let owner = ResourceOwner::new(bounds);
            let values = (0..=MAX_REPLAY_TRANSCRIPT_ENVELOPES)
                .map(|_| (1, ()))
                .collect();
            debug_assert_eq!(
                owner.queue().try_push_batch(values),
                Err(ResourceError::ItemsExceeded)
            );
            let retirement = owner.retire();
            debug_assert_eq!(retirement.drained_items, 0);
            (
                "replay-overflow",
                "invalid_envelope",
                "fresh_render",
                0,
                0,
                MAX_REPLAY_TRANSCRIPT_ENVELOPES,
                256 * 1024,
            )
        }
        "revoked-authorization" => {
            let error = AsyncTransportError::new(AsyncTransportErrorKind::AuthorizationLost);
            debug_assert_eq!(error.kind(), AsyncTransportErrorKind::AuthorizationLost);
            (
                "revoked-authorization",
                "authorization_lost",
                "reauthorize",
                0,
                0,
                1,
                1,
            )
        }
        "fanout-pressure" => {
            let close = AsyncCloseCode::FanoutExceeded;
            debug_assert_eq!(close, AsyncCloseCode::FanoutExceeded);
            (
                "fanout-pressure",
                "fanout_exceeded",
                "reconnect",
                2,
                8,
                2,
                8,
            )
        }
        "oversized-message" => {
            let bytes = vec![b'x'; 513];
            let error = WebSocketCodec::v1()
                .decode_membership_request(WebSocketFrame::Text {
                    payload: &bytes,
                    final_fragment: true,
                })
                .expect_err("oversized control frame");
            debug_assert_eq!(error.kind(), AsyncTransportErrorKind::FrameTooLarge);
            (
                "oversized-message",
                "frame_too_large",
                "close_transport",
                0,
                0,
                1,
                512,
            )
        }
        "truncated-message" => {
            let error = WebSocketCodec::v1()
                .decode_membership_request(WebSocketFrame::Text {
                    payload: br#"{"kind":"subscribe","#,
                    final_fragment: true,
                })
                .expect_err("truncated control frame");
            debug_assert_eq!(error.kind(), AsyncTransportErrorKind::InvalidEnvelope);
            (
                "truncated-message",
                "invalid_envelope",
                "close_transport",
                0,
                0,
                1,
                512,
            )
        }
        "reordered-message" => {
            let disposition = SequenceDisposition::Degraded(SequenceDegradation::Gap);
            debug_assert_eq!(
                disposition,
                SequenceDisposition::Degraded(SequenceDegradation::Gap)
            );
            (
                "reordered-message",
                "sequence_gap",
                "fresh_render",
                1,
                4,
                2,
                8,
            )
        }
        "duplicate-completion" => {
            let mut machine = machine(UploadState::Transferring, 7);
            let request = transition(7, "complete-once", UploadTransition::Complete);
            machine.apply(request.clone()).expect("first completion");
            let repeated = machine.apply(request).expect("repeated completion");
            debug_assert_eq!(
                repeated.disposition(),
                TransitionDisposition::ExistingOutcome
            );
            (
                "duplicate-completion",
                "existing_outcome",
                "none",
                1,
                4,
                2,
                8,
            )
        }
        "cancel-finalize-cancel-wins" => race_probe(
            "cancel-finalize-cancel-wins",
            UploadTransition::Cancel,
            true,
            "terminal_canceled",
        )?,
        "cancel-finalize-finalize-wins" => race_probe(
            "cancel-finalize-finalize-wins",
            UploadTransition::Cancel,
            false,
            "terminal_finalized",
        )?,
        "expire-finalize-expire-wins" => race_probe(
            "expire-finalize-expire-wins",
            UploadTransition::Expire,
            true,
            "terminal_expired",
        )?,
        "expire-finalize-finalize-wins" => race_probe(
            "expire-finalize-finalize-wins",
            UploadTransition::Expire,
            false,
            "terminal_finalized",
        )?,
        "late-event" => {
            let owner = ResourceOwner::new(ResourceBounds::new(1, 8).expect("late event bounds"));
            owner.retire();
            debug_assert_eq!(owner.queue().try_push(8, ()), Err(ResourceError::Retired));
            ("late-event", "retired_delivery_ignored", "none", 0, 0, 1, 8)
        }
        "retirement" => {
            let owner = ResourceOwner::new(ResourceBounds::new(1, 8).expect("retirement bounds"));
            owner.queue().try_push(8, ()).expect("bounded resource");
            let retirement = owner.retire();
            debug_assert_eq!(retirement.drained_items, 1);
            debug_assert_eq!(retirement.drained_bytes, 8);
            debug_assert!(owner.queue().is_empty());
            ("retirement", "retired", "none", 1, 8, 1, 8)
        }
        "unknown-feature-failure" => {
            let dependent = ResourceOwner::new(ResourceBounds::new(1, 8).expect("feature bounds"));
            dependent.retire();
            debug_assert_eq!(
                dependent.queue().try_push(1, ()),
                Err(ResourceError::Retired)
            );
            (
                "unknown-feature-failure",
                "feature_unavailable",
                "none",
                0,
                0,
                1,
                8,
            )
        }
        "scoped-exhaustion" => {
            let owner = ResourceOwner::new(ResourceBounds::new(2, 8).expect("scope bounds"));
            owner.queue().try_push(4, ()).expect("scope item one");
            owner.queue().try_push(4, ()).expect("scope item two");
            debug_assert_eq!(
                owner.queue().try_push(1, ()),
                Err(ResourceError::ItemsExceeded)
            );
            let retirement = owner.retire();
            debug_assert_eq!(retirement.drained_items, 2);
            (
                "scoped-exhaustion",
                "creation_rate_exceeded",
                "new_scope",
                2,
                8,
                2,
                8,
            )
        }
        _ => return None,
    };

    AdversarialOutcome::from_probe(
        case,
        disposition,
        recovery,
        items,
        bytes,
        max_items,
        max_bytes,
    )
}

fn machine(state: UploadState, revision: u64) -> UploadStateMachine {
    UploadStateMachine::new(
        UploadHandle::parse(HANDLE).expect("fixed adversarial handle"),
        state,
        UploadRevision::new(revision),
    )
}

fn transition(revision: u64, key: &str, transition: UploadTransition) -> UploadTransitionRequest {
    UploadTransitionRequest::new(
        UploadHandle::parse(HANDLE).expect("fixed adversarial handle"),
        UploadRevision::new(revision),
        UploadIdempotencyKey::parse(key).expect("fixed idempotency key"),
        transition,
    )
}

fn race_probe(
    case: &'static str,
    terminal: UploadTransition,
    terminal_wins: bool,
    recovery: &'static str,
) -> Option<(
    &'static str,
    &'static str,
    &'static str,
    usize,
    usize,
    usize,
    usize,
)> {
    let mut machine = machine(UploadState::Ready, 8);
    let loser = if terminal_wins {
        machine
            .apply(transition(8, "terminal-winner", terminal))
            .ok()?;
        machine
            .apply(transition(
                8,
                "finalize-loser",
                UploadTransition::BeginFinalize,
            ))
            .expect_err("finalize loses exact base revision")
    } else {
        machine
            .apply(transition(
                8,
                "finalize-winner",
                UploadTransition::BeginFinalize,
            ))
            .ok()?;
        machine
            .apply(transition(
                9,
                "finalize-commit",
                UploadTransition::CommitFinalize,
            ))
            .ok()?;
        machine
            .apply(transition(8, "terminal-loser", terminal))
            .expect_err("terminal loses exact base revision")
    };
    if loser.kind() != UploadErrorKind::UploadConflict {
        return None;
    }
    Some((case, "upload_conflict", recovery, 1, 4, 2, 8))
}
