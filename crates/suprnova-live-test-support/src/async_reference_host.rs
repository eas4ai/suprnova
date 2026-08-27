//! Deterministic async browser-conformance authority for the thin Rust host.

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU8;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use suprnova_live::async_updates::{
    AuthorizationMemo, BoundedEventContracts, BoundedTargets, BoundedTopics, BrowserPayloadSchema,
    CapabilityVersion, EventCyclePolicy, EventOrder, EventSource, EventTarget, PollFallbackPolicy,
    PollInitialBehavior, PollVisibilityPolicy, ReconnectPolicy, StreamEpoch, StreamName,
    StreamPosition, StreamSequence, SubscriptionClaims, SubscriptionDescriptor,
    SubscriptionDescriptorCodec, SubscriptionEventContract, TopicName,
};
use suprnova_live::crypto::{KeyRecord, RootKey, SnapshotKeyRing};
use suprnova_live::identity::{KeyId, UnixMillis};
use suprnova_live::metadata::{EventMetadata, EventPayloadMetadata};

const MAX_REFERENCE_HISTORY: usize = 1_024;

/// Exact static origin of the production-artifact scenario document.
pub const ASYNC_REFERENCE_ORIGIN: &str = "http://127.0.0.1:4174";
/// Exact authenticated scenario session.
pub const ASYNC_REFERENCE_SESSION: &str = "task9-session";
/// Exact authenticated scenario principal.
pub const ASYNC_REFERENCE_PRINCIPAL: &str = "task9-principal";
/// Exact document authorization scope.
pub const ASYNC_REFERENCE_SCOPE: &str = "task9-reference-document";

/// A deterministic fault that the browser suite may ask the Rust host to schedule.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AsyncReferenceFault {
    /// Emit position `N + 2` on the still-open transport before recovery supplies `N + 1`.
    SequenceGap,
    /// Attempt old-envelope, old-acknowledgment, and late-authorization delivery.
    LateWork,
    /// Close the physical transport without manufacturing a sequence gap.
    TransportLoss,
    /// Emit a registered terminal completion.
    ServerShutdown,
}

/// Static facts for the Task 9 browser-conformance scenario.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AsyncReferenceScenario {
    /// Human-readable scenario identity.
    pub name: &'static str,
    /// Registered stream selected by the checked directive.
    pub stream: &'static str,
    /// Canonical subscription identity used by the browser decoder.
    pub subscription_id: &'static str,
}

impl AsyncReferenceScenario {
    /// Returns the deterministic Task 9 lifecycle scenario.
    #[must_use]
    pub const fn lifecycle() -> Self {
        Self {
            name: "async-lifecycle-accessibility",
            stream: "orders",
            subscription_id: "c3Vic2NyaXB0aW9uLTAwMQ",
        }
    }

    /// Returns the bounded deterministic fault schedule exercised by the browser suite.
    #[must_use]
    pub const fn faults(self) -> [AsyncReferenceFault; 4] {
        [
            AsyncReferenceFault::SequenceGap,
            AsyncReferenceFault::LateWork,
            AsyncReferenceFault::TransportLoss,
            AsyncReferenceFault::ServerShutdown,
        ]
    }

    /// Produces one canonical heartbeat envelope for the production browser decoder.
    #[must_use]
    pub fn heartbeat(self, sequence: u64) -> String {
        self.envelope(sequence, json!({ "kind": "heartbeat" }))
    }

    /// Produces one canonical registered fresh-render envelope.
    #[must_use]
    pub fn refresh(self, sequence: u64) -> String {
        self.envelope(sequence, json!({ "kind": "refresh", "name": "refresh" }))
    }

    /// Produces one canonical completion envelope for the production browser decoder.
    #[must_use]
    pub fn completion(self, sequence: u64) -> String {
        self.envelope(
            sequence,
            json!({ "kind": "complete", "reason": "server_shutdown" }),
        )
    }

    /// Wraps a canonical envelope in the exact bounded SSE wire record shape.
    #[must_use]
    pub fn sse_record(self, sequence: u64, encoded: &str) -> Vec<u8> {
        format!(
            "id:{}/{}/{}\nevent:suprnova-live-async\ndata:{}\n\n",
            self.subscription_id, 1, sequence, encoded
        )
        .into_bytes()
    }

    fn envelope(self, sequence: u64, payload: Value) -> String {
        serde_json::to_string(&json!({
            "payload": payload,
            "position": { "epoch": "1", "sequence": sequence.to_string() },
            "protocol_version": 1,
            "stream": self.stream,
            "subscription": self.subscription_id,
        }))
        .expect("static async reference envelope serializes")
    }
}

/// Bounded browser facts accepted by the Rust authorization endpoint.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AsyncReferenceAuthorizationRequest {
    /// Current browser-observed position; absent only for a fresh mount.
    pub position: Option<AsyncReferencePosition>,
    /// Prior subscription identity; absent only for a fresh mount.
    pub prior_subscription_id: Option<String>,
    /// Authenticated principal fact.
    pub principal: String,
    /// Authenticated document scope fact.
    pub scope: String,
    /// Authenticated session fact.
    pub session: String,
    /// Checked stream selector.
    pub stream: String,
}

/// Decimal-wire stream position submitted as a non-authoritative observation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AsyncReferencePosition {
    /// Stream epoch.
    pub epoch: String,
    /// Stream sequence.
    pub sequence: String,
}

/// Exact external membership request checked against signed current Rust authority.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AsyncReferenceMembershipRequest {
    /// Browser control correlation nonce.
    pub control_nonce: String,
    /// Opaque signed descriptor returned by Rust authorization.
    pub descriptor: String,
    /// Compact descriptor binding returned by Rust authorization.
    pub descriptor_binding: String,
    /// Membership operation.
    pub operation: String,
    /// Authenticated principal fact.
    pub principal: String,
    /// Authenticated document scope fact.
    pub scope: String,
    /// Authenticated session fact.
    pub session: String,
    /// Checked stream selector.
    pub stream: String,
    /// Current logical subscription identity.
    pub subscription_id: String,
    /// Current physical transport generation.
    pub transport_generation: u64,
}

/// Exact fallback-poll facts checked against the current Rust-issued authority.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AsyncReferencePollRequest {
    /// Compact binding of the current signed descriptor.
    pub descriptor_binding: String,
    /// Browser-observed continuity position.
    pub position: AsyncReferencePosition,
    /// Current logical subscription identity.
    pub subscription_id: String,
}

#[derive(Clone)]
struct IssuedAuthority {
    binding: String,
    credential: String,
    descriptor: String,
    expires_at: u64,
    ever_opened: bool,
    membership_open: bool,
    open_transport: Option<u64>,
    transport_generation: u64,
    used_control_nonces: BTreeSet<String>,
}

/// Deterministic Rust-owned authority, continuity history, and exact membership ledger.
pub struct AsyncReferenceAuthority {
    codec: SubscriptionDescriptorCodec,
    current_sequence: u64,
    history: BTreeMap<u64, String>,
    issued: Option<IssuedAuthority>,
    next_credential: u64,
    next_transport: u64,
    open_transports: BTreeSet<u64>,
    subscription_id: String,
    origin: String,
}

impl AsyncReferenceAuthority {
    /// Creates one complete reference authority with a production descriptor codec.
    pub fn new(now: UnixMillis) -> Self {
        Self::new_with_subscription(
            now,
            AsyncReferenceScenario::lifecycle()
                .subscription_id
                .to_owned(),
        )
    }

    pub(crate) fn new_with_subscription(now: UnixMillis, subscription_id: String) -> Self {
        Self::new_with_origin_subscription(now, ASYNC_REFERENCE_ORIGIN.to_owned(), subscription_id)
    }

    pub(crate) fn new_with_origin_subscription(
        now: UnixMillis,
        origin: String,
        subscription_id: String,
    ) -> Self {
        let active = KeyRecord::new(
            KeyId::parse("task9-async-key").expect("static key id"),
            RootKey::new(vec![0x91; 32]).expect("static test key"),
            UnixMillis::new(now.get().saturating_sub(60_000)),
            UnixMillis::new(now.get().saturating_add(3_600_000)),
            UnixMillis::new(now.get().saturating_add(7_200_000)),
        )
        .expect("reference key window");
        Self {
            codec: SubscriptionDescriptorCodec::new(
                SnapshotKeyRing::new(active, Vec::new()).expect("reference key ring"),
            ),
            current_sequence: 0,
            history: BTreeMap::new(),
            issued: None,
            next_credential: 1,
            next_transport: 1,
            open_transports: BTreeSet::new(),
            subscription_id,
            origin,
        }
    }

    pub(crate) fn origin(&self) -> &str {
        &self.origin
    }

    pub(crate) fn install_external_authority(
        &mut self,
        descriptor: String,
        binding: String,
        credential: String,
        expires_at: UnixMillis,
    ) -> Result<(), &'static str> {
        let parsed =
            SubscriptionDescriptor::parse(&descriptor).map_err(|_| "descriptor_invalid")?;
        let expected = URL_SAFE_NO_PAD.encode(Sha256::digest(parsed.as_str().as_bytes()));
        if expected != binding || expires_at <= UnixMillis::new(1_000) {
            return Err("authority_issue_failed");
        }
        self.issued = Some(IssuedAuthority {
            binding,
            credential,
            descriptor,
            expires_at: expires_at.get(),
            ever_opened: false,
            membership_open: false,
            open_transport: None,
            transport_generation: 0,
            used_control_nonces: BTreeSet::new(),
        });
        Ok(())
    }

    /// Returns the current authoritative sequence.
    #[must_use]
    pub const fn current_sequence(&self) -> u64 {
        self.current_sequence
    }

    /// Registers one exact open physical transport before membership may commit.
    pub fn open_transport(
        &mut self,
        origin: &str,
        bearer: &str,
        transport_generation: u64,
        now: UnixMillis,
    ) -> Result<u64, &'static str> {
        if origin != self.origin {
            return Err("origin_mismatch");
        }
        let issued = self.issued.as_mut().ok_or("authority_missing")?;
        if bearer != issued.credential || now.get() >= issued.expires_at {
            return Err("transport_authority_invalid");
        }
        if transport_generation == 0 || issued.open_transport.is_some() {
            return Err("transport_generation_invalid");
        }
        let transport = self.next_transport;
        self.next_transport = self.next_transport.saturating_add(1);
        if !self.open_transports.insert(transport) {
            return Err("transport_generation_invalid");
        }
        issued.open_transport = Some(transport);
        issued.transport_generation = transport_generation;
        Ok(transport)
    }

    /// Retires one exact physical transport.
    pub fn close_transport(&mut self, transport: u64) {
        self.open_transports.remove(&transport);
        if let Some(issued) = self.issued.as_mut()
            && issued.open_transport == Some(transport)
        {
            issued.open_transport = None;
            issued.membership_open = false;
        }
    }

    /// Issues signed current authority and a complete replay from trusted Rust history.
    pub fn authorize(
        &mut self,
        origin: &str,
        request: &AsyncReferenceAuthorizationRequest,
        now: UnixMillis,
    ) -> Result<Value, &'static str> {
        self.validate_common(origin, &request.session, &request.principal, &request.scope)?;
        let scenario = AsyncReferenceScenario::lifecycle();
        if request.stream != scenario.stream {
            return Err("stream_mismatch");
        }
        let observed = match (&request.prior_subscription_id, &request.position) {
            (None, None) => self.current_sequence,
            (Some(prior), Some(position)) if prior == &self.subscription_id => {
                parse_position(position)?
            }
            _ => return Err("continuity_facts_invalid"),
        };
        if observed > self.current_sequence {
            return Err("future_position_rejected");
        }
        if request.prior_subscription_id.is_some()
            && self
                .issued
                .as_ref()
                .is_none_or(|issued| !issued.ever_opened)
        {
            return Err("prior_membership_missing");
        }
        let claims = claims(observed, now.get().saturating_add(60_000))?;
        let descriptor = self
            .codec
            .sign(&claims, now)
            .map_err(|_| "descriptor_issue_failed")?;
        let binding = URL_SAFE_NO_PAD.encode(Sha256::digest(descriptor.as_str().as_bytes()));
        let credential = format!("task9-reference-credential-{:016}", self.next_credential);
        self.next_credential = self.next_credential.saturating_add(1);
        let expires_at = claims.expires_at().get();
        self.issued = Some(IssuedAuthority {
            binding: binding.clone(),
            credential: credential.clone(),
            descriptor: descriptor.as_str().to_owned(),
            expires_at,
            ever_opened: false,
            membership_open: false,
            open_transport: None,
            transport_generation: 0,
            used_control_nonces: BTreeSet::new(),
        });
        let replay = self
            .history
            .range(observed.saturating_add(1)..)
            .map(|(_, envelope)| envelope.clone())
            .collect::<Vec<_>>();
        Ok(json!({
            "proof": if replay.is_empty() { "authoritative_no_tail" } else { "complete_replay" },
            "replay": replay,
            "subscription": {
                "authorization": { "credential": credential, "kind": "bearer" },
                "baseline": { "epoch": "1", "sequence": observed.to_string() },
                "descriptor": descriptor.as_str(),
                "descriptor_binding": binding,
                "document": {
                    "authorization_scope": ASYNC_REFERENCE_SCOPE,
                    "origin": "http://127.0.0.1:4174",
                    "transport": "sse"
                },
                "events": [],
                "expires_at": expires_at,
                "fallback_poll": {
                    "initial": "wait",
                    "interval_ms": 30_000,
                    "jitter_ratio": 0,
                    "visibility": "visible"
                },
                "heartbeat_timeout_ms": 10_000,
                "presentation_signals": [],
                "reconnect": {
                    "kind": "resume_or_refresh",
                    "maximum_attempts": 4,
                    "maximum_delay_ms": 1_000,
                    "minimum_delay_ms": 250
                },
                "stream": scenario.stream,
                "subscription_id": self.subscription_id
            }
        }))
    }

    /// Authenticates one exact membership and returns a one-use canonical acknowledgment.
    pub fn membership(
        &mut self,
        origin: &str,
        bearer: &str,
        request: &AsyncReferenceMembershipRequest,
        now: UnixMillis,
    ) -> Result<Value, &'static str> {
        self.validate_common(origin, &request.session, &request.principal, &request.scope)?;
        let scenario = AsyncReferenceScenario::lifecycle();
        if (request.operation != "subscribe" && request.operation != "unsubscribe")
            || request.stream != scenario.stream
            || request.subscription_id != self.subscription_id
            || request.control_nonce.is_empty()
            || request.control_nonce.len() > 128
            || request.transport_generation == 0
        {
            return Err("membership_facts_invalid");
        }
        let issued = self.issued.as_mut().ok_or("authority_missing")?;
        if bearer != issued.credential
            || request.descriptor != issued.descriptor
            || request.descriptor_binding != issued.binding
            || now.get() >= issued.expires_at
            || issued.transport_generation != request.transport_generation
            || issued
                .open_transport
                .is_none_or(|transport| !self.open_transports.contains(&transport))
            || issued.used_control_nonces.contains(&request.control_nonce)
        {
            return Err("membership_authority_invalid");
        }
        if (request.operation == "subscribe" && issued.membership_open)
            || (request.operation == "unsubscribe" && !issued.membership_open)
        {
            return Err("membership_authority_invalid");
        }
        let descriptor =
            SubscriptionDescriptor::parse(&request.descriptor).map_err(|_| "descriptor_invalid")?;
        self.codec
            .verify(&descriptor, now)
            .map_err(|_| "descriptor_invalid")?;
        issued.membership_open = request.operation == "subscribe";
        issued.ever_opened = true;
        issued.transport_generation = request.transport_generation;
        issued
            .used_control_nonces
            .insert(request.control_nonce.clone());
        Ok(json!({
            "controlNonce": request.control_nonce,
            "descriptorBinding": issued.binding,
            "kind": "authenticated",
            "operation": request.operation,
            "stream": scenario.stream,
            "subscriptionId": self.subscription_id,
            "transportGeneration": request.transport_generation
        }))
    }

    /// Authorizes one fallback poll and returns an exact replay or no-tail proof.
    pub fn poll(
        &self,
        origin: &str,
        bearer: &str,
        request: &AsyncReferencePollRequest,
        now: UnixMillis,
    ) -> Result<Value, &'static str> {
        if origin != self.origin || request.subscription_id != self.subscription_id {
            return Err("poll_authority_invalid");
        }
        let issued = self.issued.as_ref().ok_or("poll_authority_invalid")?;
        if bearer != issued.credential
            || request.descriptor_binding != issued.binding
            || now.get() >= issued.expires_at
        {
            return Err("poll_authority_invalid");
        }
        let observed = parse_position(&request.position)?;
        if observed > self.current_sequence {
            return Err("future_position_rejected");
        }
        let envelopes = self
            .history
            .range(observed.saturating_add(1)..)
            .map(|(_, envelope)| envelope.clone())
            .collect::<Vec<_>>();
        Ok(json!({
            "current_position": {
                "epoch": "1",
                "sequence": self.current_sequence.to_string()
            },
            "envelopes": envelopes,
            "fallback": {
                "interval_ms": 30_000,
                "visibility": "visible"
            },
            "proof": if observed == self.current_sequence {
                "authoritative_no_tail"
            } else {
                "complete_replay"
            }
        }))
    }

    /// Emits a genuine `N + 2` gap while retaining `N + 1` and `N + 2` for complete replay.
    pub fn sequence_gap(&mut self) -> (u64, String) {
        let missing = self.current_sequence.saturating_add(1);
        let gap = self.current_sequence.saturating_add(2);
        self.record_history(
            missing,
            envelope_for(
                &self.subscription_id,
                missing,
                json!({ "kind": "heartbeat" }),
            ),
        );
        let refresh = envelope_for(
            &self.subscription_id,
            gap,
            json!({ "kind": "refresh", "name": "refresh" }),
        );
        self.record_history(gap, refresh.clone());
        self.current_sequence = gap;
        (gap, refresh)
    }

    /// Advances the authoritative sequence by one heartbeat.
    pub fn next_heartbeat(&mut self) -> (u64, String) {
        self.current_sequence = self.current_sequence.saturating_add(1);
        let envelope = envelope_for(
            &self.subscription_id,
            self.current_sequence,
            json!({ "kind": "heartbeat" }),
        );
        self.record_history(self.current_sequence, envelope.clone());
        (self.current_sequence, envelope)
    }

    /// Advances the authoritative sequence by one terminal completion.
    pub fn completion(&mut self) -> (u64, String) {
        self.current_sequence = self.current_sequence.saturating_add(1);
        let envelope = envelope_for(
            &self.subscription_id,
            self.current_sequence,
            json!({ "kind": "complete", "reason": "server_shutdown" }),
        );
        self.record_history(self.current_sequence, envelope.clone());
        (self.current_sequence, envelope)
    }

    /// Marks current membership retired without changing continuity.
    pub fn retire_membership(&mut self) {
        if let Some(issued) = self.issued.as_mut() {
            issued.membership_open = false;
        }
    }

    /// Returns whether one exact physical transport owns current authenticated membership.
    #[must_use]
    pub fn may_deliver_on(&self, transport: u64) -> bool {
        self.issued.as_ref().is_some_and(|issued| {
            issued.membership_open
                && issued.open_transport == Some(transport)
                && self.open_transports.contains(&transport)
        })
    }

    /// Returns whether the exact physical transport remains authoritative.
    #[must_use]
    pub fn transport_is_open(&self, transport: u64) -> bool {
        self.open_transports.contains(&transport)
    }

    fn record_history(&mut self, sequence: u64, envelope: String) {
        self.history.insert(sequence, envelope);
        while self.history.len() > MAX_REFERENCE_HISTORY {
            self.history.pop_first();
        }
    }

    fn validate_common(
        &self,
        origin: &str,
        session: &str,
        principal: &str,
        scope: &str,
    ) -> Result<(), &'static str> {
        if origin != self.origin {
            return Err("origin_mismatch");
        }
        if session != ASYNC_REFERENCE_SESSION
            || principal != ASYNC_REFERENCE_PRINCIPAL
            || scope != ASYNC_REFERENCE_SCOPE
        {
            return Err("identity_scope_mismatch");
        }
        Ok(())
    }
}

fn envelope_for(subscription_id: &str, sequence: u64, payload: Value) -> String {
    serde_json::to_string(&json!({
        "payload": payload,
        "position": { "epoch": "1", "sequence": sequence.to_string() },
        "protocol_version": 1,
        "stream": AsyncReferenceScenario::lifecycle().stream,
        "subscription": subscription_id,
    }))
    .expect("static async reference envelope serializes")
}

fn parse_position(position: &AsyncReferencePosition) -> Result<u64, &'static str> {
    if position.epoch != "1" {
        return Err("epoch_mismatch");
    }
    position
        .sequence
        .parse::<u64>()
        .map_err(|_| "sequence_invalid")
}

fn claims(baseline: u64, expires_at: u64) -> Result<SubscriptionClaims, &'static str> {
    struct OrderUpdated;
    impl EventPayloadMetadata for OrderUpdated {
        const NAME: &'static str = "orders.updated";
        const VERSION: u16 = 1;
        const SCHEMA: BrowserPayloadSchema = BrowserPayloadSchema::Json;
    }
    let metadata = EventMetadata::from_payload_with_contract::<OrderUpdated>(
        EventSource::Stream,
        BoundedTargets::new(vec![EventTarget::SelfIsland]).map_err(|_| "target_invalid")?,
        EventOrder::PerSourceSequence,
        EventCyclePolicy::ForbidRepeatedIsland,
        1,
    )
    .map_err(|_| "event_contract_invalid")?;
    let event = SubscriptionEventContract::from_registered(&metadata)
        .map_err(|_| "event_contract_invalid")?;
    SubscriptionClaims::new(
        StreamName::parse(AsyncReferenceScenario::lifecycle().stream)
            .map_err(|_| "stream_invalid")?,
        1,
        CapabilityVersion::new(1).map_err(|_| "capability_invalid")?,
        BoundedTopics::new(vec![
            TopicName::parse("orders").map_err(|_| "topic_invalid")?,
        ])
        .map_err(|_| "topics_invalid")?,
        BoundedEventContracts::new(vec![event]).map_err(|_| "events_invalid")?,
        AuthorizationMemo::parse("task9-reference-memo").map_err(|_| "memo_invalid")?,
        StreamPosition::new(StreamEpoch::new(1), StreamSequence::new(baseline)),
        UnixMillis::new(expires_at),
        ReconnectPolicy::ResumeOrRefresh {
            maximum_attempts: NonZeroU8::new(4).expect("four is nonzero"),
        },
        PollFallbackPolicy::new(
            30_000,
            0,
            PollInitialBehavior::AfterInterval,
            PollVisibilityPolicy::PauseWhenHidden,
        )
        .map_err(|_| "fallback_invalid")?,
    )
    .map_err(|_| "claims_invalid")
}

#[cfg(test)]
mod tests {
    use super::{
        ASYNC_REFERENCE_ORIGIN, ASYNC_REFERENCE_PRINCIPAL, ASYNC_REFERENCE_SCOPE,
        ASYNC_REFERENCE_SESSION, AsyncReferenceAuthority, AsyncReferenceAuthorizationRequest,
        AsyncReferenceMembershipRequest, AsyncReferencePollRequest, AsyncReferencePosition,
    };
    use serde_json::Value;
    use suprnova_live::identity::UnixMillis;

    fn request() -> AsyncReferenceAuthorizationRequest {
        AsyncReferenceAuthorizationRequest {
            position: None,
            prior_subscription_id: None,
            principal: ASYNC_REFERENCE_PRINCIPAL.to_owned(),
            scope: ASYNC_REFERENCE_SCOPE.to_owned(),
            session: ASYNC_REFERENCE_SESSION.to_owned(),
            stream: "orders".to_owned(),
        }
    }

    fn membership_request(
        issued: &Value,
        control_nonce: &str,
        operation: &str,
        transport_generation: u64,
    ) -> (String, AsyncReferenceMembershipRequest) {
        let subscription = &issued["subscription"];
        (
            subscription["authorization"]["credential"]
                .as_str()
                .expect("credential")
                .to_owned(),
            AsyncReferenceMembershipRequest {
                control_nonce: control_nonce.to_owned(),
                descriptor: subscription["descriptor"]
                    .as_str()
                    .expect("descriptor")
                    .to_owned(),
                descriptor_binding: subscription["descriptor_binding"]
                    .as_str()
                    .expect("binding")
                    .to_owned(),
                operation: operation.to_owned(),
                principal: ASYNC_REFERENCE_PRINCIPAL.to_owned(),
                scope: ASYNC_REFERENCE_SCOPE.to_owned(),
                session: ASYNC_REFERENCE_SESSION.to_owned(),
                stream: "orders".to_owned(),
                subscription_id: "c3Vic2NyaXB0aW9uLTAwMQ".to_owned(),
                transport_generation,
            },
        )
    }

    #[test]
    fn authority_rejects_browser_proposed_scope_and_future_position() {
        let now = UnixMillis::new(1_000_000);
        let mut authority = AsyncReferenceAuthority::new(now);
        let mut forged = request();
        forged.scope = "forged".to_owned();
        assert_eq!(
            authority.authorize(ASYNC_REFERENCE_ORIGIN, &forged, now),
            Err("identity_scope_mismatch")
        );
        assert_eq!(
            authority.authorize("https://forged.example", &request(), now),
            Err("origin_mismatch")
        );
        authority
            .authorize(ASYNC_REFERENCE_ORIGIN, &request(), now)
            .expect("fresh authority");
        authority.next_heartbeat();
        let mut future = request();
        future.prior_subscription_id = Some("c3Vic2NyaXB0aW9uLTAwMQ".to_owned());
        future.position = Some(AsyncReferencePosition {
            epoch: "1".to_owned(),
            sequence: "99".to_owned(),
        });
        assert_eq!(
            authority.authorize(ASYNC_REFERENCE_ORIGIN, &future, now),
            Err("future_position_rejected")
        );
    }

    #[test]
    fn gap_is_n_plus_two_and_complete_replay_remains_rust_owned() {
        let now = UnixMillis::new(1_000_000);
        let mut authority = AsyncReferenceAuthority::new(now);
        authority
            .authorize(ASYNC_REFERENCE_ORIGIN, &request(), now)
            .expect("fresh authority");
        let issued = authority.issued.as_mut().expect("issued");
        issued.membership_open = true;
        issued.ever_opened = true;
        let (gap, _) = authority.sequence_gap();
        assert_eq!(gap, 2);
        let replay = authority
            .authorize(
                ASYNC_REFERENCE_ORIGIN,
                &AsyncReferenceAuthorizationRequest {
                    position: Some(AsyncReferencePosition {
                        epoch: "1".to_owned(),
                        sequence: "0".to_owned(),
                    }),
                    prior_subscription_id: Some("c3Vic2NyaXB0aW9uLTAwMQ".to_owned()),
                    ..request()
                },
                now,
            )
            .expect("current replay");
        assert_eq!(replay["proof"], "complete_replay");
        assert_eq!(replay["replay"].as_array().expect("replay").len(), 2);
    }

    #[test]
    fn membership_binds_current_authority_transport_and_one_use_control() {
        let now = UnixMillis::new(1_000_000);
        let mut authority = AsyncReferenceAuthority::new(now);
        let issued = authority
            .authorize(ASYNC_REFERENCE_ORIGIN, &request(), now)
            .expect("fresh authority");
        let (credential, subscribe) = membership_request(&issued, "control-1", "subscribe", 1);
        let transport = authority
            .open_transport(ASYNC_REFERENCE_ORIGIN, &credential, 1, now)
            .expect("transport");

        let acknowledgment = authority
            .membership(ASYNC_REFERENCE_ORIGIN, &credential, &subscribe, now)
            .expect("membership");
        assert_eq!(acknowledgment["controlNonce"], "control-1");
        assert_eq!(
            authority.membership(ASYNC_REFERENCE_ORIGIN, &credential, &subscribe, now),
            Err("membership_authority_invalid")
        );

        let (_, mut forged) = membership_request(&issued, "control-2", "unsubscribe", 1);
        forged.descriptor_binding = "forged-binding".to_owned();
        assert_eq!(
            authority.membership(ASYNC_REFERENCE_ORIGIN, &credential, &forged, now),
            Err("membership_authority_invalid")
        );
        authority.close_transport(transport);
        let (_, after_close) = membership_request(&issued, "control-3", "unsubscribe", 1);
        assert_eq!(
            authority.membership(ASYNC_REFERENCE_ORIGIN, &credential, &after_close, now),
            Err("membership_authority_invalid")
        );
    }

    #[test]
    fn successor_authority_may_reuse_browser_generation_without_stale_close_aliasing() {
        let now = UnixMillis::new(1_000_000);
        let mut authority = AsyncReferenceAuthority::new(now);
        let first = authority
            .authorize(ASYNC_REFERENCE_ORIGIN, &request(), now)
            .expect("first authority");
        let (first_credential, first_membership) =
            membership_request(&first, "control-first", "subscribe", 1);
        let first_transport = authority
            .open_transport(ASYNC_REFERENCE_ORIGIN, &first_credential, 1, now)
            .expect("first transport");
        authority
            .membership(
                ASYNC_REFERENCE_ORIGIN,
                &first_credential,
                &first_membership,
                now,
            )
            .expect("first membership");

        let successor = authority
            .authorize(
                ASYNC_REFERENCE_ORIGIN,
                &AsyncReferenceAuthorizationRequest {
                    position: Some(AsyncReferencePosition {
                        epoch: "1".to_owned(),
                        sequence: "0".to_owned(),
                    }),
                    prior_subscription_id: Some("c3Vic2NyaXB0aW9uLTAwMQ".to_owned()),
                    ..request()
                },
                now,
            )
            .expect("successor authority");
        let (successor_credential, successor_membership) =
            membership_request(&successor, "control-successor", "subscribe", 1);
        let successor_transport = authority
            .open_transport(ASYNC_REFERENCE_ORIGIN, &successor_credential, 1, now)
            .expect("successor transport");
        assert_ne!(successor_transport, first_transport);

        authority.close_transport(first_transport);
        authority
            .membership(
                ASYNC_REFERENCE_ORIGIN,
                &successor_credential,
                &successor_membership,
                now,
            )
            .expect("stale close cannot retire successor");
        assert!(authority.transport_is_open(successor_transport));
    }

    #[test]
    fn delivery_requires_current_authenticated_membership_on_the_exact_transport() {
        let now = UnixMillis::new(1_000_000);
        let mut authority = AsyncReferenceAuthority::new(now);
        let issued = authority
            .authorize(ASYNC_REFERENCE_ORIGIN, &request(), now)
            .expect("fresh authority");
        let (credential, membership) =
            membership_request(&issued, "control-delivery", "subscribe", 1);
        let transport = authority
            .open_transport(ASYNC_REFERENCE_ORIGIN, &credential, 1, now)
            .expect("transport");

        assert!(!authority.may_deliver_on(transport));
        authority
            .membership(ASYNC_REFERENCE_ORIGIN, &credential, &membership, now)
            .expect("membership");
        assert!(authority.may_deliver_on(transport));

        let successor = authority
            .authorize(
                ASYNC_REFERENCE_ORIGIN,
                &AsyncReferenceAuthorizationRequest {
                    position: Some(AsyncReferencePosition {
                        epoch: "1".to_owned(),
                        sequence: "0".to_owned(),
                    }),
                    prior_subscription_id: Some("c3Vic2NyaXB0aW9uLTAwMQ".to_owned()),
                    ..request()
                },
                now,
            )
            .expect("successor authority");
        assert!(!authority.may_deliver_on(transport));

        let (successor_credential, successor_membership) =
            membership_request(&successor, "control-successor-delivery", "subscribe", 2);
        let successor_transport = authority
            .open_transport(ASYNC_REFERENCE_ORIGIN, &successor_credential, 2, now)
            .expect("successor transport");
        authority
            .membership(
                ASYNC_REFERENCE_ORIGIN,
                &successor_credential,
                &successor_membership,
                now,
            )
            .expect("successor membership");
        assert!(authority.may_deliver_on(successor_transport));
        authority.close_transport(successor_transport);
        assert!(!authority.may_deliver_on(successor_transport));
    }

    #[test]
    fn poll_validates_current_authority_and_returns_authoritative_continuity() {
        let now = UnixMillis::new(1_000_000);
        let mut authority = AsyncReferenceAuthority::new(now);
        let issued = authority
            .authorize(ASYNC_REFERENCE_ORIGIN, &request(), now)
            .expect("fresh authority");
        let subscription = &issued["subscription"];
        let credential = subscription["authorization"]["credential"]
            .as_str()
            .expect("credential");
        let poll = AsyncReferencePollRequest {
            descriptor_binding: subscription["descriptor_binding"]
                .as_str()
                .expect("binding")
                .to_owned(),
            position: AsyncReferencePosition {
                epoch: "1".to_owned(),
                sequence: "0".to_owned(),
            },
            subscription_id: "c3Vic2NyaXB0aW9uLTAwMQ".to_owned(),
        };

        let no_tail = authority
            .poll(ASYNC_REFERENCE_ORIGIN, credential, &poll, now)
            .expect("current poll");
        assert_eq!(no_tail["proof"], "authoritative_no_tail");
        assert_eq!(no_tail["envelopes"].as_array().expect("envelopes").len(), 0);

        authority.next_heartbeat();
        let replay = authority
            .poll(ASYNC_REFERENCE_ORIGIN, credential, &poll, now)
            .expect("replay poll");
        assert_eq!(replay["proof"], "complete_replay");
        assert_eq!(replay["envelopes"].as_array().expect("envelopes").len(), 1);

        let mut forged_binding = poll.clone();
        forged_binding.descriptor_binding = "forged".to_owned();
        assert_eq!(
            authority.poll(ASYNC_REFERENCE_ORIGIN, credential, &forged_binding, now),
            Err("poll_authority_invalid")
        );
        let mut future = poll.clone();
        future.position.sequence = "99".to_owned();
        assert_eq!(
            authority.poll(ASYNC_REFERENCE_ORIGIN, credential, &future, now),
            Err("future_position_rejected")
        );
        assert_eq!(
            authority.poll(ASYNC_REFERENCE_ORIGIN, "stale-credential", &poll, now),
            Err("poll_authority_invalid")
        );
        assert_eq!(
            authority.poll(
                ASYNC_REFERENCE_ORIGIN,
                credential,
                &poll,
                UnixMillis::new(1_061_000),
            ),
            Err("poll_authority_invalid")
        );
    }
}
