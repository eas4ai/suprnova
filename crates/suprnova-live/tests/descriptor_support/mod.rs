//! Shared deterministic values for registered-event descriptor conformance tests.

use std::collections::BTreeSet;
use std::num::NonZeroU8;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::Value;
use suprnova_live::async_updates::{
    AuthorizationMemo, BoundedEventContracts, BoundedTargets, BoundedTopics, BrowserPayloadSchema,
    CapabilityVersion, EventCyclePolicy, EventOrder, EventSource, EventTarget, PollFallbackPolicy,
    PollInitialBehavior, PollVisibilityPolicy, ReconnectPolicy, StreamEpoch, StreamName,
    StreamPosition, StreamSequence, SubscriptionClaims, SubscriptionDescriptor,
    SubscriptionDescriptorCodec, SubscriptionEventContract, TopicName,
};
use suprnova_live::canonical::{parse_canonical_value, to_canonical_bytes};
use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing, SnapshotPurpose};
use suprnova_live::identity::{KeyId, UnixMillis};
use suprnova_live::limits::InputLimits;
use suprnova_live::metadata::{EventMetadata, EventPayloadMetadata};

struct PresenceChanged;

impl EventPayloadMetadata for PresenceChanged {
    const NAME: &'static str = "presence.changed";
    const VERSION: u16 = 2;
    const PAYLOAD_CONTRACT: &'static str = "presence.member-changed";
    const SCHEMA: BrowserPayloadSchema = BrowserPayloadSchema::Boolean;
}

/// Returns the fixed claims-body canonicalization limits used across descriptor tests.
pub(crate) fn claims_limits() -> InputLimits {
    InputLimits::new(65_536, 8, 4_096, 8_192).expect("claims limits are valid")
}

/// Returns a deterministic descriptor-signing key ring.
pub(crate) fn key_ring() -> SnapshotKeyRing {
    let active = KeyRecord::new(
        KeyId::parse("descriptor-fixture-1").expect("key id is valid"),
        RootKey::new(vec![0x37; 32]).expect("root key is strong"),
        UnixMillis::new(0),
        UnixMillis::new(50_000),
        UnixMillis::new(100_000),
    )
    .expect("key record window is valid");
    SnapshotKeyRing::new(active, Vec::new()).expect("key ring is valid")
}

/// Returns a codec bound to the deterministic descriptor-signing key ring.
pub(crate) fn codec() -> SubscriptionDescriptorCodec {
    SubscriptionDescriptorCodec::new(key_ring())
}

/// Returns the one registered-event contract shared by the descriptor fixture cases.
///
/// Uses the bounded-hops cycle so the fixture's key set includes `maximum_hops`
/// alongside `maximum_fanout` and `payload_contract`.
pub(crate) fn presence_changed_event() -> SubscriptionEventContract {
    let metadata = EventMetadata::from_payload_with_contract::<PresenceChanged>(
        EventSource::Stream,
        BoundedTargets::new(vec![EventTarget::Parent]).expect("targets are valid"),
        EventOrder::PerSourceSequence,
        EventCyclePolicy::MaximumHops(NonZeroU8::new(3).expect("hops is nonzero")),
        2,
    )
    .expect("event metadata is valid");
    SubscriptionEventContract::from_registered(&metadata).expect("registered event contract")
}

/// Returns the one valid claims object every descriptor fixture case signs from.
pub(crate) fn claims() -> SubscriptionClaims {
    SubscriptionClaims::new(
        StreamName::parse("tenant.presence").expect("stream is valid"),
        1,
        CapabilityVersion::new(1).expect("capability is valid"),
        BoundedTopics::new(vec![
            TopicName::parse("tenant/7/presence").expect("topic is valid"),
        ])
        .expect("topics are valid"),
        BoundedEventContracts::new(vec![presence_changed_event()]).expect("events are valid"),
        AuthorizationMemo::parse("scope-v1:tenant-7:presence-panel").expect("memo is valid"),
        StreamPosition::new(StreamEpoch::new(4), StreamSequence::new(19)),
        UnixMillis::new(20_000),
        ReconnectPolicy::ResumeOrRefresh {
            maximum_attempts: NonZeroU8::new(4).expect("attempts is nonzero"),
        },
        PollFallbackPolicy::new(
            10_000,
            1_500,
            PollInitialBehavior::AfterInterval,
            PollVisibilityPolicy::PauseWhenHidden,
        )
        .expect("fallback poll is valid"),
    )
    .expect("claims are valid")
}

/// Signs the shared claims, decodes the canonical body back to a JSON value, mutates it,
/// re-signs the mutated bytes with the same deterministic key, and returns the resulting
/// descriptor. Mirrors the wire exactly except for the mutation, so a rejection is
/// attributable to the mutation alone.
pub(crate) fn resign_with_mutation(mutator: impl FnOnce(&mut Value)) -> SubscriptionDescriptor {
    let original = codec()
        .sign(&claims(), UnixMillis::new(1_000))
        .expect("sign original descriptor");
    let encoded = original.as_str().split('.').nth(2).expect("claims body");
    let mut value: Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(encoded).expect("decode claims body"))
            .expect("claims JSON");
    mutator(&mut value);
    let limits = claims_limits();
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

/// Returns the real, currently-signed `events[0]` projection as a plain JSON value.
pub(crate) fn real_event_zero() -> Value {
    let descriptor = codec()
        .sign(&claims(), UnixMillis::new(1_000))
        .expect("sign descriptor");
    let encoded = descriptor.as_str().split('.').nth(2).expect("claims body");
    let body: Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(encoded).expect("decode claims body"))
            .expect("claims JSON");
    body["events"][0].clone()
}

/// Collects every object key reachable from `value` by recursing into nested objects only.
///
/// Deliberately does not recurse into arrays: `targets` is an array of plain strings in
/// the browser projection but an array of `{kind, value}` objects in the engine's internal
/// wire (a real, intentional shape difference unrelated to field-naming), and this
/// comparison exists to catch naming drift, not that unrelated shape difference.
pub(crate) fn key_set(value: &Value) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    collect_keys(value, &mut keys);
    keys
}

fn collect_keys(value: &Value, keys: &mut BTreeSet<String>) {
    if let Value::Object(fields) = value {
        for (key, nested) in fields {
            keys.insert(key.clone());
            collect_keys(nested, keys);
        }
    }
}
