//! Signed asynchronous-subscription descriptor contract tests.

use std::num::NonZeroU8;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use suprnova_live::async_updates::{
    AuthorizationMemo, BoundedEventContracts, BoundedTopics, CapabilityVersion, PollFallbackPolicy,
    PollInitialBehavior, PollVisibilityPolicy, ReconnectPolicy, StreamEpoch, StreamName,
    StreamPosition, StreamSequence, SubscriptionClaims, SubscriptionDescriptor,
    SubscriptionDescriptorCodec, SubscriptionErrorKind, TopicName,
};
use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing, SnapshotPurpose};
use suprnova_live::identity::{BrowserOperationName, KeyId, UnixMillis};

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
            BrowserOperationName::parse("order.updated").expect("event"),
            BrowserOperationName::parse("presence.changed").expect("event"),
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
