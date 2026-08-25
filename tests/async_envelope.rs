//! Independently versioned asynchronous envelope and sequence-authority tests.

use std::fs;
use std::num::{NonZeroU8, NonZeroU16};

use proptest::prelude::*;
use serde_json::Value;
use suprnova_live::SUPPORTED_PROTOCOL_VERSIONS;
use suprnova_live::async_updates::{
    AsyncCodecLimits, AsyncEnvelope, AsyncEnvelopeContext, AsyncEnvelopeErrorKind, AsyncPayload,
    BaselineDisposition, BoundedEventContracts, BoundedPresentationSignalContracts, BoundedTargets,
    BrowserPayloadSchema, CompletionReason, ContinuityProof, EventCyclePolicy, EventOrder,
    EventSource, EventTarget, PresentationSignalContract, RegisteredBrowserEvent,
    RegisteredPresentationSignal, SUPPORTED_ASYNC_PROTOCOL_VERSIONS, SequenceDisposition,
    SequenceErrorKind, SequenceMachine, SequenceState, StreamEpoch, StreamErrorCode, StreamName,
    StreamPosition, StreamSequence, SubscriptionEventContract, SubscriptionId,
    decode_async_envelope, encode_async_envelope,
};
use suprnova_live::canonical::CanonicalValue;
use suprnova_live::conformance::{FixtureVersion, fixture_directory};
use suprnova_live::identity::BrowserOperationName;
use suprnova_live::metadata::{EventMetadata, EventPayloadMetadata};

struct OrdersUpdated;

impl EventPayloadMetadata for OrdersUpdated {
    const NAME: &'static str = "orders.updated";
    const VERSION: u16 = 1;
    const PAYLOAD_CONTRACT: &'static str = "orders.updated.payload";
    const SCHEMA: BrowserPayloadSchema = BrowserPayloadSchema::Json;
}

fn event_contract() -> SubscriptionEventContract {
    let metadata = EventMetadata::from_payload_with_contract::<OrdersUpdated>(
        EventSource::Stream,
        BoundedTargets::new(vec![EventTarget::SelfIsland, EventTarget::Document])
            .expect("bounded targets"),
        EventOrder::PerSourceSequence,
        EventCyclePolicy::MaximumHops(NonZeroU8::new(4).expect("nonzero hops")),
        4,
    )
    .expect("registered event metadata");
    SubscriptionEventContract::from_registered(&metadata).expect("event contract")
}

fn subscription_id() -> SubscriptionId {
    SubscriptionId::from_bytes(b"subscription-001").expect("subscription id")
}

fn stream() -> StreamName {
    StreamName::parse("orders").expect("stream")
}

fn context() -> AsyncEnvelopeContext {
    AsyncEnvelopeContext::new(
        subscription_id(),
        stream(),
        BoundedEventContracts::new(vec![event_contract()]).expect("events"),
        BoundedPresentationSignalContracts::new(vec![PresentationSignalContract::new(
            BrowserOperationName::parse("completion_percent").expect("signal name"),
            BrowserPayloadSchema::U64,
        )])
        .expect("signals"),
    )
}

fn limits() -> AsyncCodecLimits {
    AsyncCodecLimits::v1()
}

fn fixture_position(value: &Value) -> StreamPosition {
    StreamPosition::new(
        StreamEpoch::new(
            value["epoch"]
                .as_str()
                .expect("fixture epoch")
                .parse()
                .expect("decimal epoch"),
        ),
        StreamSequence::new(
            value["sequence"]
                .as_str()
                .expect("fixture sequence")
                .parse()
                .expect("decimal sequence"),
        ),
    )
}

fn wire(payload: &str, epoch: u64, sequence: u64) -> Vec<u8> {
    format!(
        "{{\"payload\":{payload},\"position\":{{\"epoch\":\"{epoch}\",\"sequence\":\"{sequence}\"}},\"protocol_version\":1,\"stream\":\"orders\",\"subscription\":\"{}\"}}",
        subscription_id().to_base64url(),
    )
    .into_bytes()
}

fn decode(payload: &str, epoch: u64, sequence: u64) -> suprnova_live::async_updates::AsyncEnvelope {
    decode_async_envelope(&wire(payload, epoch, sequence), &limits(), &context())
        .expect("valid async envelope")
}

#[test]
fn async_protocol_is_independent_from_live_action_and_morph_versions() {
    assert_eq!(SUPPORTED_ASYNC_PROTOCOL_VERSIONS, &[1]);
    assert_eq!(SUPPORTED_PROTOCOL_VERSIONS, &[1, 2]);
}

#[test]
fn every_closed_payload_kind_decodes_and_round_trips_canonically() {
    let payloads = [
        "{\"kind\":\"refresh\",\"name\":\"refresh\"}",
        "{\"event\":\"orders.updated\",\"kind\":\"browser_event\",\"payload\":{\"count\":1},\"schema_version\":1,\"target\":\"self\"}",
        "{\"kind\":\"presentation_signal\",\"name\":\"completion_percent\",\"value\":50}",
        "{\"kind\":\"heartbeat\"}",
        "{\"kind\":\"complete\",\"reason\":\"server_shutdown\"}",
        "{\"code\":\"authorization_lost\",\"kind\":\"error\"}",
    ];

    for (offset, payload) in payloads.into_iter().enumerate() {
        let encoded = wire(payload, 4, 41 + offset as u64);
        let envelope = decode_async_envelope(&encoded, &limits(), &context()).expect("decode");
        assert_eq!(envelope.protocol_version(), 1);
        assert_eq!(envelope.subscription(), &subscription_id());
        assert_eq!(envelope.stream(), &stream());
        assert_eq!(
            envelope.position(),
            StreamPosition::new(StreamEpoch::new(4), StreamSequence::new(41 + offset as u64))
        );
        assert_eq!(
            encode_async_envelope(&envelope, &limits()).expect("encode"),
            encoded
        );
    }
}

#[test]
fn server_authored_envelopes_require_the_current_registered_context() {
    let context = context();
    let event = RegisteredBrowserEvent::new(
        &context,
        BrowserOperationName::parse("orders.updated").expect("event name"),
        1,
        EventTarget::SelfIsland,
        CanonicalValue::Null,
    )
    .expect("registered event");
    let envelope = AsyncEnvelope::new(
        &context,
        StreamPosition::new(StreamEpoch::new(8), StreamSequence::new(21)),
        AsyncPayload::BrowserEvent(event),
    )
    .expect("server-authored envelope");
    let encoded = encode_async_envelope(&envelope, &limits()).expect("encode");
    assert_eq!(
        decode_async_envelope(&encoded, &limits(), &context).expect("decode"),
        envelope
    );

    let signal = RegisteredPresentationSignal::new(
        &context,
        BrowserOperationName::parse("completion_percent").expect("signal name"),
        CanonicalValue::String("wrong schema".to_owned()),
    )
    .expect_err("signal schema must match current registration");
    assert_eq!(signal.kind(), AsyncEnvelopeErrorKind::UnregisteredPayload);

    let oversized = RegisteredBrowserEvent::new(
        &context,
        BrowserOperationName::parse("orders.updated").expect("event name"),
        1,
        EventTarget::SelfIsland,
        CanonicalValue::String("x".repeat(32_769)),
    )
    .expect_err("server-authored payloads must be bounded before envelope construction");
    assert_eq!(oversized.kind(), AsyncEnvelopeErrorKind::StringTooLong);
}

#[test]
fn decoded_payloads_are_closed_registered_values() {
    let refresh = decode("{\"kind\":\"refresh\",\"name\":\"refresh\"}", 1, 1);
    assert!(matches!(refresh.payload(), AsyncPayload::Refresh(_)));

    let event = decode(
        "{\"event\":\"orders.updated\",\"kind\":\"browser_event\",\"payload\":{\"count\":1},\"schema_version\":1,\"target\":\"self\"}",
        1,
        2,
    );
    let AsyncPayload::BrowserEvent(event) = event.payload() else {
        panic!("browser event payload")
    };
    assert_eq!(event.name().as_str(), OrdersUpdated::NAME);
    assert_eq!(event.schema_version(), OrdersUpdated::VERSION);
    assert_eq!(event.target(), &EventTarget::SelfIsland);
    assert!(matches!(event.payload(), CanonicalValue::Object(_)));

    let signal = decode(
        "{\"kind\":\"presentation_signal\",\"name\":\"completion_percent\",\"value\":50}",
        1,
        3,
    );
    let AsyncPayload::PresentationSignal(signal) = signal.payload() else {
        panic!("presentation signal payload")
    };
    assert_eq!(signal.name().as_str(), "completion_percent");

    assert!(matches!(
        decode(
            "{\"kind\":\"complete\",\"reason\":\"server_shutdown\"}",
            1,
            4
        )
        .payload(),
        AsyncPayload::Complete(CompletionReason::ServerShutdown)
    ));
    assert!(matches!(
        decode("{\"code\":\"authorization_lost\",\"kind\":\"error\"}", 1, 5).payload(),
        AsyncPayload::Error(StreamErrorCode::AuthorizationLost)
    ));
}

#[test]
fn envelope_debug_output_never_exposes_raw_payload_values() {
    const SENTINEL: &str = "async_payload_secret_sentinel";
    let envelope = decode(
        &format!(
            "{{\"event\":\"orders.updated\",\"kind\":\"browser_event\",\"payload\":{{\"secret\":\"{SENTINEL}\"}},\"schema_version\":1,\"target\":\"self\"}}"
        ),
        1,
        1,
    );

    assert!(!format!("{envelope:?}").contains(SENTINEL));
    assert!(!format!("{:?}", envelope.payload()).contains(SENTINEL));
}

#[test]
fn unknown_major_duplicate_unknown_and_noncanonical_fields_fail_closed() {
    let id = subscription_id().to_base64url();
    let cases = [
        (
            format!(
                "{{\"payload\":{{\"kind\":\"heartbeat\"}},\"position\":{{\"epoch\":\"4\",\"sequence\":\"41\"}},\"protocol_version\":2,\"stream\":\"orders\",\"subscription\":\"{id}\"}}"
            ),
            AsyncEnvelopeErrorKind::UnsupportedProtocol,
        ),
        (
            format!(
                "{{\"payload\":{{\"kind\":\"heartbeat\"}},\"position\":{{\"epoch\":\"4\",\"sequence\":\"41\"}},\"protocol_version\":1,\"protocol_version\":1,\"stream\":\"orders\",\"subscription\":\"{id}\"}}"
            ),
            AsyncEnvelopeErrorKind::DuplicateField,
        ),
        (
            format!(
                "{{\"payload\":{{\"kind\":\"heartbeat\"}},\"position\":{{\"epoch\":\"4\",\"sequence\":\"41\"}},\"protocol_version\":1,\"stream\":\"orders\",\"subscription\":\"{id}\",\"unexpected\":true}}"
            ),
            AsyncEnvelopeErrorKind::InvalidEnvelope,
        ),
        (
            format!(
                "{{ \"payload\":{{\"kind\":\"heartbeat\"}},\"position\":{{\"epoch\":\"4\",\"sequence\":\"41\"}},\"protocol_version\":1,\"stream\":\"orders\",\"subscription\":\"{id}\"}}"
            ),
            AsyncEnvelopeErrorKind::NonCanonical,
        ),
    ];

    for (encoded, expected) in cases {
        assert_eq!(
            decode_async_envelope(encoded.as_bytes(), &limits(), &context())
                .expect_err("hostile envelope")
                .kind(),
            expected,
        );
    }
}

#[test]
fn unsupported_or_malformed_operations_cannot_become_dispatch_authority() {
    let payloads = [
        "{\"html\":\"<p>unsafe</p>\",\"kind\":\"html\"}",
        "{\"action\":\"delete\",\"kind\":\"action\"}",
        "{\"kind\":\"effect\",\"name\":\"eval\"}",
        "{\"kind\":\"snapshot\",\"value\":\"secret\"}",
    ];
    for payload in payloads {
        assert_eq!(
            decode_async_envelope(&wire(payload, 1, 1), &limits(), &context())
                .expect_err("unsupported operation")
                .kind(),
            AsyncEnvelopeErrorKind::UnsupportedPayload,
        );
    }

    for payload in [
        "{\"extra\":true,\"kind\":\"heartbeat\"}",
        "{\"kind\":\"refresh\",\"name\":\"save\"}",
        "{\"kind\":\"complete\",\"reason\":\"run_action\"}",
        "{\"code\":\"arbitrary\",\"kind\":\"error\"}",
    ] {
        assert_eq!(
            decode_async_envelope(&wire(payload, 1, 1), &limits(), &context())
                .expect_err("malformed operation")
                .kind(),
            AsyncEnvelopeErrorKind::InvalidPayload,
        );
    }
}

#[test]
fn event_and_signal_payloads_require_current_registered_contracts() {
    let cases = [
        "{\"event\":\"orders.deleted\",\"kind\":\"browser_event\",\"payload\":{},\"schema_version\":1,\"target\":\"self\"}",
        "{\"event\":\"orders.updated\",\"kind\":\"browser_event\",\"payload\":{},\"schema_version\":2,\"target\":\"self\"}",
        "{\"event\":\"orders.updated\",\"kind\":\"browser_event\",\"payload\":{},\"schema_version\":1,\"target\":\"parent\"}",
        "{\"kind\":\"presentation_signal\",\"name\":\"unknown_signal\",\"value\":50}",
        "{\"kind\":\"presentation_signal\",\"name\":\"completion_percent\",\"value\":\"fifty\"}",
    ];
    for payload in cases {
        assert_eq!(
            decode_async_envelope(&wire(payload, 1, 1), &limits(), &context())
                .expect_err("unregistered payload")
                .kind(),
            AsyncEnvelopeErrorKind::UnregisteredPayload,
        );
    }
}

#[test]
fn byte_depth_entry_string_and_payload_limits_are_enforced() {
    let tiny = AsyncCodecLimits::new(256, 4, 16, 32, 64).expect("tiny limits");
    assert_eq!(
        decode_async_envelope(&vec![b'x'; 257], &tiny, &context())
            .expect_err("raw byte limit")
            .kind(),
        AsyncEnvelopeErrorKind::TooLarge,
    );

    let deeply_nested = wire(
        "{\"event\":\"orders.updated\",\"kind\":\"browser_event\",\"payload\":[[[[[null]]]]],\"schema_version\":1,\"target\":\"self\"}",
        1,
        1,
    );
    assert_eq!(
        decode_async_envelope(&deeply_nested, &tiny, &context())
            .expect_err("depth limit")
            .kind(),
        AsyncEnvelopeErrorKind::TooDeep,
    );

    let large_payload = format!(
        "{{\"event\":\"orders.updated\",\"kind\":\"browser_event\",\"payload\":\"{}\",\"schema_version\":1,\"target\":\"self\"}}",
        "x".repeat(65),
    );
    let payload_limited = AsyncCodecLimits::new(1_024, 8, 64, 128, 64).expect("limits");
    assert_eq!(
        decode_async_envelope(&wire(&large_payload, 1, 1), &payload_limited, &context())
            .expect_err("payload byte limit")
            .kind(),
        AsyncEnvelopeErrorKind::PayloadTooLarge,
    );

    let long_string = AsyncCodecLimits::new(1_024, 8, 64, 8, 512).expect("limits");
    assert_eq!(
        decode_async_envelope(
            &wire("{\"kind\":\"heartbeat\"}", 1, 1),
            &long_string,
            &context()
        )
        .expect_err("string limit")
        .kind(),
        AsyncEnvelopeErrorKind::StringTooLong,
    );

    let duplicate_with_oversized_second_payload = format!(
        "{{\"payload\":{{\"kind\":\"heartbeat\"}},\"payload\":\"{}\",\"position\":{{\"epoch\":\"1\",\"sequence\":\"1\"}},\"protocol_version\":1,\"stream\":\"orders\",\"subscription\":\"{}\"}}",
        "x".repeat(65),
        subscription_id().to_base64url(),
    );
    assert_eq!(
        decode_async_envelope(
            duplicate_with_oversized_second_payload.as_bytes(),
            &payload_limited,
            &context(),
        )
        .expect_err("payload is bounded before duplicate-field parsing")
        .kind(),
        AsyncEnvelopeErrorKind::PayloadTooLarge,
    );
}

#[test]
fn membership_and_stream_binding_are_validated_before_sequence_observation() {
    let mut machine = SequenceMachine::new(StreamPosition::new(
        StreamEpoch::new(4),
        StreamSequence::new(40),
    ));
    let current = machine.current();
    let other_id = SubscriptionId::from_bytes(b"subscription-002").expect("other id");
    let wrong_subscription = wire("{\"kind\":\"heartbeat\"}", 4, 41);
    let wrong_context = AsyncEnvelopeContext::new(
        other_id,
        stream(),
        BoundedEventContracts::new(vec![event_contract()]).expect("events"),
        BoundedPresentationSignalContracts::new(vec![]).expect("signals"),
    );
    assert_eq!(
        decode_async_envelope(&wrong_subscription, &limits(), &wrong_context)
            .expect_err("inactive subscription")
            .kind(),
        AsyncEnvelopeErrorKind::SubscriptionMismatch,
    );
    assert_eq!(machine.current(), current);

    let wrong_stream = AsyncEnvelopeContext::new(
        subscription_id(),
        StreamName::parse("other").expect("other stream"),
        BoundedEventContracts::new(vec![event_contract()]).expect("events"),
        BoundedPresentationSignalContracts::new(vec![]).expect("signals"),
    );
    assert_eq!(
        decode_async_envelope(&wrong_subscription, &limits(), &wrong_stream)
            .expect_err("wrong stream")
            .kind(),
        AsyncEnvelopeErrorKind::StreamMismatch,
    );
    assert_eq!(machine.current(), current);

    let valid = decode("{\"kind\":\"heartbeat\"}", 4, 41);
    assert_eq!(machine.observe(&valid), SequenceDisposition::Apply);
    assert_eq!(machine.current(), valid.position());
}

#[test]
fn sequence_machine_applies_only_next_and_degrades_on_gaps_or_new_epochs() {
    let baseline = StreamPosition::new(StreamEpoch::new(4), StreamSequence::new(40));
    let mut machine = SequenceMachine::new(baseline);

    assert_eq!(
        machine.observe(&decode("{\"kind\":\"heartbeat\"}", 4, 40)),
        SequenceDisposition::IgnoreDuplicate
    );
    assert_eq!(
        machine.observe(&decode("{\"kind\":\"heartbeat\"}", 3, 99)),
        SequenceDisposition::IgnoreStaleEpoch
    );
    assert_eq!(
        machine.observe(&decode("{\"kind\":\"heartbeat\"}", 4, 41)),
        SequenceDisposition::Apply
    );
    assert_eq!(machine.state(), SequenceState::Current);

    assert!(matches!(
        machine.observe(&decode("{\"kind\":\"heartbeat\"}", 4, 43)),
        SequenceDisposition::Degraded(_)
    ));
    assert_eq!(machine.state(), SequenceState::Degraded);
    assert_eq!(
        machine.observe(&decode("{\"kind\":\"heartbeat\"}", 4, 41)),
        SequenceDisposition::IgnoreDuplicate
    );
    assert_eq!(
        machine.observe(&decode("{\"kind\":\"heartbeat\"}", 4, 42)),
        SequenceDisposition::AwaitingRecovery
    );
    assert_eq!(machine.current().sequence(), StreamSequence::new(41));

    assert_eq!(
        machine.adopt(ContinuityProof::Replay {
            from: StreamPosition::new(StreamEpoch::new(4), StreamSequence::new(41)),
            through: StreamPosition::new(StreamEpoch::new(4), StreamSequence::new(43)),
        }),
        Ok(BaselineDisposition::Adopted)
    );
    assert_eq!(machine.state(), SequenceState::Current);
    assert_eq!(
        machine.current(),
        StreamPosition::new(StreamEpoch::new(4), StreamSequence::new(43))
    );

    assert!(matches!(
        machine.observe(&decode("{\"kind\":\"heartbeat\"}", 5, 1)),
        SequenceDisposition::Degraded(_)
    ));
    assert_eq!(
        machine.adopt(ContinuityProof::AuthoritativeRefresh {
            baseline: StreamPosition::new(StreamEpoch::new(5), StreamSequence::new(7)),
        }),
        Ok(BaselineDisposition::Adopted)
    );
    assert_eq!(machine.state(), SequenceState::Current);
}

#[test]
fn invalid_replay_and_regressing_refresh_proofs_cannot_rewrite_authority() {
    let baseline = StreamPosition::new(StreamEpoch::new(4), StreamSequence::new(41));
    let mut machine = SequenceMachine::new(baseline);
    let _ = machine.observe(&decode("{\"kind\":\"heartbeat\"}", 4, 43));

    for proof in [
        ContinuityProof::Replay {
            from: StreamPosition::new(StreamEpoch::new(4), StreamSequence::new(40)),
            through: StreamPosition::new(StreamEpoch::new(4), StreamSequence::new(43)),
        },
        ContinuityProof::Replay {
            from: baseline,
            through: StreamPosition::new(StreamEpoch::new(5), StreamSequence::new(1)),
        },
    ] {
        assert_eq!(
            machine
                .adopt(proof)
                .expect_err("invalid replay proof")
                .kind(),
            SequenceErrorKind::InvalidReplayProof
        );
        assert_eq!(machine.current(), baseline);
        assert_eq!(machine.state(), SequenceState::Degraded);
    }

    assert_eq!(
        machine
            .adopt(ContinuityProof::AuthoritativeRefresh {
                baseline: StreamPosition::new(StreamEpoch::new(4), StreamSequence::new(40)),
            })
            .expect_err("same-epoch baseline regression")
            .kind(),
        SequenceErrorKind::BaselineRegression
    );
    assert_eq!(machine.current(), baseline);
}

#[test]
fn sequence_overflow_never_wraps_or_applies() {
    let baseline = StreamPosition::new(StreamEpoch::new(9), StreamSequence::new(u64::MAX));
    let mut machine = SequenceMachine::new(baseline);
    assert_eq!(
        machine.observe(&decode("{\"kind\":\"heartbeat\"}", 9, u64::MAX)),
        SequenceDisposition::IgnoreDuplicate
    );
    assert!(matches!(
        machine.observe(&decode("{\"kind\":\"heartbeat\"}", 10, 0)),
        SequenceDisposition::Degraded(_)
    ));
    assert_eq!(machine.current(), baseline);
}

proptest! {
    #[test]
    fn sequence_machine_never_applies_a_gap(observed in prop::collection::vec((0_u64..4, any::<u64>()), 0..128)) {
        let baseline = StreamPosition::new(StreamEpoch::new(2), StreamSequence::new(10));
        let mut machine = SequenceMachine::new(baseline);
        for (epoch, sequence) in observed {
            let before = machine.current();
            let envelope = decode("{\"kind\":\"heartbeat\"}", epoch, sequence);
            if machine.observe(&envelope) == SequenceDisposition::Apply {
                prop_assert_eq!(epoch, before.epoch().get());
                prop_assert_eq!(sequence, before.sequence().get() + 1);
                prop_assert_eq!(machine.current(), envelope.position());
            }
        }
    }

    #[test]
    fn canonical_round_trip_is_stable(epoch in any::<u64>(), sequence in any::<u64>()) {
        let encoded = wire("{\"kind\":\"heartbeat\"}", epoch, sequence);
        let envelope = decode_async_envelope(&encoded, &limits(), &context()).expect("decode");
        prop_assert_eq!(encode_async_envelope(&envelope, &limits()).expect("encode"), encoded);
    }
}

#[test]
fn version_four_async_fixture_is_executable_not_documentary() {
    let root: Value = serde_json::from_slice(
        &fs::read(fixture_directory(FixtureVersion::V4).join("async-envelope.json"))
            .expect("fixture bytes"),
    )
    .expect("fixture JSON");

    assert_eq!(root["protocol_versions"], serde_json::json!([1]));
    assert_eq!(root["live_protocol_versions"], serde_json::json!([1, 2]));
    for case in root["envelope_cases"].as_array().expect("envelope cases") {
        let encoded = case["encoded"].as_str().expect("encoded case").as_bytes();
        let result = decode_async_envelope(encoded, &limits(), &context());
        match case["expected"].as_str().expect("expected disposition") {
            "accepted" => {
                let envelope = result.expect("accepted fixture");
                assert_eq!(
                    encode_async_envelope(&envelope, &limits()).expect("encode fixture"),
                    encoded
                );
            }
            "unsupported_protocol" => assert_eq!(
                result.expect_err("unsupported protocol").kind(),
                AsyncEnvelopeErrorKind::UnsupportedProtocol
            ),
            "duplicate_field" => assert_eq!(
                result.expect_err("duplicate field").kind(),
                AsyncEnvelopeErrorKind::DuplicateField
            ),
            "unsupported_payload" => assert_eq!(
                result.expect_err("unsupported payload").kind(),
                AsyncEnvelopeErrorKind::UnsupportedPayload
            ),
            other => panic!("unknown expected fixture disposition: {other}"),
        }
    }

    for case in root["continuity_cases"]
        .as_array()
        .expect("continuity cases")
    {
        let baseline = fixture_position(&case["baseline"]);
        let mut machine = SequenceMachine::new(baseline);
        let disposition = if let Some(observed) = case.get("observed") {
            let observed = fixture_position(observed);
            let envelope = decode(
                "{\"kind\":\"heartbeat\"}",
                observed.epoch().get(),
                observed.sequence().get(),
            );
            match machine.observe(&envelope) {
                SequenceDisposition::Apply => "apply",
                SequenceDisposition::IgnoreDuplicate => "ignore_duplicate",
                SequenceDisposition::Degraded(_) => "degrade",
                SequenceDisposition::IgnoreStaleEpoch => "ignore_stale_epoch",
                SequenceDisposition::AwaitingRecovery => "awaiting_recovery",
            }
        } else {
            let proof = &case["proof"];
            let through = fixture_position(&proof["through"]);
            let authority = match proof["kind"].as_str().expect("proof kind") {
                "replay" => ContinuityProof::Replay {
                    from: baseline,
                    through,
                },
                "authoritative_refresh" => {
                    ContinuityProof::AuthoritativeRefresh { baseline: through }
                }
                other => panic!("unknown proof kind: {other}"),
            };
            assert_eq!(machine.adopt(authority), Ok(BaselineDisposition::Adopted));
            "adopt_baseline"
        };
        assert_eq!(
            disposition,
            case["expected"].as_str().expect("continuity expected"),
            "continuity fixture {}",
            case["id"].as_str().expect("continuity id"),
        );
        assert_eq!(
            match machine.state() {
                SequenceState::Current => "current",
                SequenceState::Degraded => "degraded",
            },
            case["state"].as_str().expect("continuity state"),
        );
    }
}

#[test]
fn event_fanout_remains_nonzero_and_bounded_in_the_registered_contract() {
    assert_eq!(
        event_contract().maximum_fanout(),
        NonZeroU16::new(4).unwrap()
    );
}
