//! Signed asynchronous-subscription descriptor contract tests.

use std::num::NonZeroU8;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::Value;
use suprnova_live::async_updates::{
    AuthorizationMemo, BoundedEventContracts, BoundedTargets, BoundedTopics, BrowserPayloadSchema,
    CapabilityVersion, EventCyclePolicy, EventOrder, EventSource, EventTarget, PollFallbackPolicy,
    PollInitialBehavior, PollVisibilityPolicy, ReconnectPolicy, StreamEpoch, StreamName,
    StreamPosition, StreamSequence, SubscriptionClaims, SubscriptionDescriptor,
    SubscriptionDescriptorCodec, SubscriptionErrorKind, SubscriptionEventContract, TopicName,
};
use suprnova_live::canonical::{parse_canonical_value, to_canonical_bytes};
use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing, SnapshotPurpose};
use suprnova_live::identity::{KeyId, UnixMillis};
use suprnova_live::limits::InputLimits;
use suprnova_live::metadata::{EventMetadata, EventPayloadMetadata};

struct OrderUpdated;

impl EventPayloadMetadata for OrderUpdated {
    const NAME: &'static str = "order.updated";
    const VERSION: u16 = 3;
    const PAYLOAD_CONTRACT: &'static str = "orders.order-updated";
    const SCHEMA: BrowserPayloadSchema = BrowserPayloadSchema::Json;
}

struct PresenceChanged;

impl EventPayloadMetadata for PresenceChanged {
    const NAME: &'static str = "presence.changed";
    const VERSION: u16 = 2;
    const PAYLOAD_CONTRACT: &'static str = "presence.member-changed";
    const SCHEMA: BrowserPayloadSchema = BrowserPayloadSchema::Boolean;
}

fn event_contract<T: EventPayloadMetadata + 'static>(
    targets: Vec<EventTarget>,
    cycle: EventCyclePolicy,
    maximum_fanout: u16,
) -> SubscriptionEventContract {
    let metadata = EventMetadata::from_payload_with_contract::<T>(
        EventSource::Stream,
        BoundedTargets::new(targets).expect("targets"),
        EventOrder::PerSourceSequence,
        cycle,
        maximum_fanout,
    )
    .expect("event metadata");
    SubscriptionEventContract::from_registered(&metadata).expect("registered event contract")
}

fn codec() -> SubscriptionDescriptorCodec {
    SubscriptionDescriptorCodec::new(key_ring())
}

fn key_ring() -> SnapshotKeyRing {
    let active = KeyRecord::new(
        KeyId::parse("async-key-1").expect("key id"),
        RootKey::new(vec![0x41; 32]).expect("root key"),
        UnixMillis::new(0),
        UnixMillis::new(50_000),
        UnixMillis::new(100_000),
    )
    .expect("key record");
    SnapshotKeyRing::new(active, Vec::new()).expect("key ring")
}

fn claims() -> SubscriptionClaims {
    claims_with_reconnect(ReconnectPolicy::ResumeOrRefresh {
        maximum_attempts: NonZeroU8::new(4).expect("attempts"),
    })
    .expect("claims")
}

fn claims_with_reconnect(
    reconnect: ReconnectPolicy,
) -> Result<SubscriptionClaims, suprnova_live::async_updates::SubscriptionError> {
    SubscriptionClaims::new(
        StreamName::parse("orders.activity").expect("stream"),
        1,
        CapabilityVersion::new(3).expect("capability"),
        BoundedTopics::new(vec![
            TopicName::parse("tenant/7/orders").expect("topic"),
            TopicName::parse("tenant/7/presence").expect("topic"),
        ])
        .expect("topics"),
        BoundedEventContracts::new(vec![
            event_contract::<OrderUpdated>(
                vec![EventTarget::SelfIsland, EventTarget::Document],
                EventCyclePolicy::ForbidRepeatedIsland,
                4,
            ),
            event_contract::<PresenceChanged>(
                vec![EventTarget::Parent],
                EventCyclePolicy::MaximumHops(NonZeroU8::new(3).expect("hops")),
                2,
            ),
        ])
        .expect("events"),
        AuthorizationMemo::parse("scope-v1:tenant-7:orders-panel").expect("memo"),
        StreamPosition::new(StreamEpoch::new(4), StreamSequence::new(19)),
        UnixMillis::new(20_000),
        reconnect,
        PollFallbackPolicy::new(
            10_000,
            1_500,
            PollInitialBehavior::AfterInterval,
            PollVisibilityPolicy::PauseWhenHidden,
        )
        .expect("fallback"),
    )
}

fn resign_with_mutation(mutator: impl FnOnce(&mut Value)) -> SubscriptionDescriptor {
    let original = codec()
        .sign(&claims(), UnixMillis::new(1_000))
        .expect("sign original descriptor");
    let encoded = original.as_str().split('.').nth(2).expect("claims body");
    let mut value: Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(encoded).expect("decode claims body"))
            .expect("claims JSON");
    mutator(&mut value);
    let limits = InputLimits::new(65_536, 8, 4_096, 8_192).expect("claims limits");
    let serialized = serde_json::to_vec(&value).expect("serialize mutated claims");
    let canonical = parse_canonical_value(&serialized, &limits).expect("canonical mutated claims");
    let body = to_canonical_bytes(&canonical, &limits).expect("encode canonical mutation");
    let signed = key_ring()
        .sign(
            SnapshotPurpose::AsyncSubscriptionV1,
            &body,
            UnixMillis::new(1_000),
        )
        .expect("sign mutated body");
    SubscriptionDescriptor::parse(&format!(
        "as1.{}.{}.{}",
        signed.key_id().as_str(),
        URL_SAFE_NO_PAD.encode(body),
        signed.signature().to_base64url()
    ))
    .expect("mutated descriptor envelope")
}

fn tamper_claim(pointer: &str, replacement: Value) -> SubscriptionDescriptor {
    let original = codec()
        .sign(&claims(), UnixMillis::new(1_000))
        .expect("sign original descriptor");
    let parts = original.as_str().split('.').collect::<Vec<_>>();
    let mut value: Value = serde_json::from_slice(
        &URL_SAFE_NO_PAD
            .decode(parts[2])
            .expect("decode original claims body"),
    )
    .expect("claims JSON");
    *value.pointer_mut(pointer).expect("claim mutation pointer") = replacement;
    let limits = InputLimits::new(65_536, 8, 4_096, 8_192).expect("claims limits");
    let serialized = serde_json::to_vec(&value).expect("serialize tampered claims");
    let canonical = parse_canonical_value(&serialized, &limits).expect("canonical tampered claims");
    let body = to_canonical_bytes(&canonical, &limits).expect("encode tampered claims");
    SubscriptionDescriptor::parse(&format!(
        "{}.{}.{}.{}",
        parts[0],
        parts[1],
        URL_SAFE_NO_PAD.encode(body),
        parts[3]
    ))
    .expect("tampered descriptor envelope")
}

#[test]
fn issued_descriptor_verifies_every_exact_claim_and_authoritative_baseline() {
    let claims = claims();
    let descriptor = codec()
        .sign(&claims, UnixMillis::new(1_000))
        .expect("sign descriptor");
    let verified = codec()
        .verify(&descriptor, UnixMillis::new(1_001))
        .expect("verify descriptor");

    assert_eq!(verified.claims(), &claims);
    assert_eq!(
        verified.baseline(),
        StreamPosition::new(StreamEpoch::new(4), StreamSequence::new(19))
    );
    assert_eq!(verified.expires_at(), UnixMillis::new(20_000));
}

#[test]
fn descriptor_tampering_and_exclusive_expiry_fail_closed() {
    let codec = codec();
    let descriptor = codec
        .sign(&claims(), UnixMillis::new(1_000))
        .expect("sign descriptor");
    let mut wire = descriptor.as_str().as_bytes().to_vec();
    let body_index = wire
        .iter()
        .position(|byte| *byte == b'.')
        .and_then(|first| {
            wire[first + 1..]
                .iter()
                .position(|byte| *byte == b'.')
                .map(|second| first + second + 2)
        })
        .expect("descriptor body");
    wire[body_index] = if wire[body_index] == b'A' { b'B' } else { b'A' };
    let tampered =
        SubscriptionDescriptor::parse(std::str::from_utf8(&wire).expect("ASCII descriptor"))
            .expect("structurally valid tampered descriptor");

    assert_eq!(
        codec
            .verify(&tampered, UnixMillis::new(1_001))
            .expect_err("tampering must fail")
            .kind(),
        SubscriptionErrorKind::InvalidDescriptor
    );
    assert_eq!(
        codec
            .verify(&descriptor, UnixMillis::new(20_000))
            .expect_err("expiry is exclusive")
            .kind(),
        SubscriptionErrorKind::DescriptorExpired
    );
}

#[test]
fn fallback_poll_and_reconnect_limits_are_hard_bounds() {
    assert_eq!(
        PollFallbackPolicy::new(
            999,
            0,
            PollInitialBehavior::Immediate,
            PollVisibilityPolicy::PauseWhenHidden,
        )
        .expect_err("subsecond fallback would amplify work")
        .kind(),
        SubscriptionErrorKind::InvalidPollFallback
    );
    assert_eq!(
        PollFallbackPolicy::new(
            10_000,
            10_001,
            PollInitialBehavior::Immediate,
            PollVisibilityPolicy::PauseWhenHidden,
        )
        .expect_err("jitter cannot exceed 100 percent")
        .kind(),
        SubscriptionErrorKind::InvalidPollFallback
    );

    assert_eq!(
        claims_with_reconnect(ReconnectPolicy::ResumeOrRefresh {
            maximum_attempts: NonZeroU8::new(17).expect("attempts"),
        })
        .expect_err("reconnect attempts are bounded")
        .kind(),
        SubscriptionErrorKind::InvalidReconnectPolicy
    );
}

#[test]
fn bounded_canonical_claims_are_required_even_with_a_valid_mac() {
    let descriptor = codec()
        .sign(&claims(), UnixMillis::new(1_000))
        .expect("sign descriptor");
    let encoded_body = descriptor.as_str().split('.').nth(2).expect("encoded body");
    let body = URL_SAFE_NO_PAD.decode(encoded_body).expect("body bytes");
    let mut noncanonical = Vec::with_capacity(body.len() + 1);
    noncanonical.push(b'{');
    noncanonical.push(b' ');
    noncanonical.extend_from_slice(&body[1..]);
    let signed = key_ring()
        .sign(
            SnapshotPurpose::AsyncSubscriptionV1,
            &noncanonical,
            UnixMillis::new(1_000),
        )
        .expect("sign noncanonical body");
    let wire = format!(
        "as1.{}.{}.{}",
        signed.key_id().as_str(),
        URL_SAFE_NO_PAD.encode(noncanonical),
        signed.signature().to_base64url()
    );
    let noncanonical_descriptor =
        SubscriptionDescriptor::parse(&wire).expect("bounded structural descriptor");

    assert_eq!(
        codec()
            .verify(&noncanonical_descriptor, UnixMillis::new(1_001))
            .expect_err("noncanonical signed claims must fail")
            .kind(),
        SubscriptionErrorKind::InvalidDescriptor
    );
    assert_eq!(
        SubscriptionDescriptor::parse(&"x".repeat(16_385))
            .expect_err("descriptor bytes are bounded before ownership")
            .kind(),
        SubscriptionErrorKind::InvalidDescriptor
    );
}

#[test]
fn semantic_array_order_is_canonical_even_with_a_valid_mac() {
    let mutations = [
        resign_with_mutation(|claims| {
            claims["topics"].as_array_mut().expect("topics").reverse();
        }),
        resign_with_mutation(|claims| {
            claims["events"].as_array_mut().expect("events").reverse();
        }),
        resign_with_mutation(|claims| {
            claims["events"][0]["targets"]
                .as_array_mut()
                .expect("targets")
                .reverse();
        }),
    ];

    for mutation in mutations {
        assert_eq!(
            codec()
                .verify(&mutation, UnixMillis::new(1_001))
                .expect_err("semantically noncanonical array order must fail")
                .kind(),
            SubscriptionErrorKind::InvalidDescriptor
        );
    }
}

#[test]
fn signed_non_stream_source_and_unknown_order_are_not_event_authority() {
    let mutations = [
        resign_with_mutation(|claims| {
            claims["events"][0]["source"] = serde_json::json!("component");
        }),
        resign_with_mutation(|claims| {
            claims["events"][0]["order"] = serde_json::json!("global");
        }),
    ];

    for mutation in mutations {
        assert_eq!(
            codec()
                .verify(&mutation, UnixMillis::new(1_001))
                .expect_err("stream descriptors accept only registered stream event semantics")
                .kind(),
            SubscriptionErrorKind::InvalidDescriptor
        );
    }
}

#[test]
fn every_security_significant_claim_and_event_field_is_signature_bound() {
    let mutations = vec![
        ("/v", serde_json::json!(2)),
        ("/stream", serde_json::json!("orders.other")),
        ("/protocol", serde_json::json!(2)),
        ("/capability", serde_json::json!(4)),
        ("/topics/0", serde_json::json!("tenant/8/orders")),
        ("/events/0/name", serde_json::json!("order.revised")),
        ("/events/0/version", serde_json::json!(4)),
        (
            "/events/0/payload_contract",
            serde_json::json!("orders.revised"),
        ),
        ("/events/0/schema", serde_json::json!("string")),
        ("/events/0/source", serde_json::json!("component")),
        ("/events/0/targets/0/kind", serde_json::json!("parent")),
        ("/events/0/order", serde_json::json!("global")),
        ("/events/0/cycle/kind", serde_json::json!("maximum_hops")),
        ("/events/0/cycle/maximum_hops", serde_json::json!(2)),
        ("/events/0/maximum_fanout", serde_json::json!(5)),
        (
            "/authorization_memo",
            serde_json::json!("scope-v1:tenant-8:orders-panel"),
        ),
        ("/baseline/epoch", serde_json::json!("5")),
        ("/baseline/sequence", serde_json::json!("20")),
        ("/expires_at", serde_json::json!("19000")),
        ("/reconnect/kind", serde_json::json!("refresh_on_reconnect")),
        ("/reconnect/maximum_attempts", serde_json::json!(5)),
        ("/fallback_poll/initial", serde_json::json!("immediate")),
        ("/fallback_poll/interval_ms", serde_json::json!("11000")),
        (
            "/fallback_poll/jitter_basis_points",
            serde_json::json!(1600),
        ),
        (
            "/fallback_poll/visibility",
            serde_json::json!("continue_when_hidden"),
        ),
    ];

    for (pointer, replacement) in mutations {
        assert_eq!(
            codec()
                .verify(&tamper_claim(pointer, replacement), UnixMillis::new(1_001),)
                .expect_err("an individually mutated claim must fail verification")
                .kind(),
            SubscriptionErrorKind::InvalidDescriptor,
            "mutation at {pointer} was not signature bound"
        );
    }
}
