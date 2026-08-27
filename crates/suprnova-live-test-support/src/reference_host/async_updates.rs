//! Deterministic HTTP, SSE, and WebSocket adapters over the async authority.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use serde::Deserialize;
use serde_json::{Value, json};
use suprnova_live::async_updates::{WebSocketCodec, WebSocketControlRecord, WebSocketFrame};
use suprnova_live::identity::UnixMillis;
use tokio::sync::Mutex;

use crate::{
    ASYNC_REFERENCE_ORIGIN, ASYNC_REFERENCE_PRINCIPAL, ASYNC_REFERENCE_SCOPE,
    ASYNC_REFERENCE_SESSION, AsyncReferenceAuthority, AsyncReferenceAuthorizationRequest,
    AsyncReferenceMembershipRequest, AsyncReferencePollRequest, AsyncReferencePosition,
    AsyncReferenceScenario,
};

use super::faults::ReferenceFaultSchedule;

const NOW: UnixMillis = UnixMillis::new(1_000);

#[derive(Deserialize)]
pub(super) struct TransportCreateRequest {
    kind: String,
    subscription: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MembershipRequest {
    authority: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PollRequest {
    subscription: String,
    authority: String,
}

struct IssuedTransport {
    transport_id: String,
    physical_id: u64,
    subscription_id: String,
    credential: String,
    descriptor: String,
    binding: String,
    generation: u64,
    membership_open: bool,
}

struct AsyncState {
    authority: AsyncReferenceAuthority,
    issued: Option<IssuedTransport>,
    poll_generation: u64,
}

pub(super) struct AsyncRuntime {
    state: Mutex<AsyncState>,
    fault: ReferenceFaultSchedule,
    fault_applied: AtomicBool,
    maximum_memberships: AtomicUsize,
}

impl AsyncRuntime {
    pub(super) fn new(fault: ReferenceFaultSchedule) -> Self {
        Self {
            state: Mutex::new(AsyncState {
                authority: AsyncReferenceAuthority::new(NOW),
                issued: None,
                poll_generation: 0,
            }),
            fault,
            fault_applied: AtomicBool::new(false),
            maximum_memberships: AtomicUsize::new(0),
        }
    }

    pub(super) async fn create(
        &self,
        request: TransportCreateRequest,
    ) -> Result<Value, &'static str> {
        if !matches!(request.kind.as_str(), "sse" | "websocket")
            || request.subscription != AsyncReferenceScenario::lifecycle().stream
        {
            return Err("transport_facts_invalid");
        }
        let mut state = self.state.lock().await;
        if let Some(previous) = state.issued.take() {
            state.authority.close_transport(previous.physical_id);
        }
        state.authority.next_heartbeat();
        let issued = state.authority.authorize(
            ASYNC_REFERENCE_ORIGIN,
            &AsyncReferenceAuthorizationRequest {
                position: None,
                prior_subscription_id: None,
                principal: ASYNC_REFERENCE_PRINCIPAL.to_owned(),
                scope: ASYNC_REFERENCE_SCOPE.to_owned(),
                session: ASYNC_REFERENCE_SESSION.to_owned(),
                stream: request.subscription,
            },
            NOW,
        )?;
        let subscription = &issued["subscription"];
        let credential = subscription["authorization"]["credential"]
            .as_str()
            .ok_or("authority_issue_failed")?
            .to_owned();
        let descriptor = subscription["descriptor"]
            .as_str()
            .ok_or("authority_issue_failed")?
            .to_owned();
        let binding = subscription["descriptor_binding"]
            .as_str()
            .ok_or("authority_issue_failed")?
            .to_owned();
        let subscription_id = subscription["subscription_id"]
            .as_str()
            .ok_or("authority_issue_failed")?
            .to_owned();
        let generation = 1;
        let physical_id =
            state
                .authority
                .open_transport(ASYNC_REFERENCE_ORIGIN, &credential, generation, NOW)?;
        let transport_id = format!("transport-{physical_id}");
        state.issued = Some(IssuedTransport {
            transport_id: transport_id.clone(),
            physical_id,
            subscription_id: subscription_id.clone(),
            credential: credential.clone(),
            descriptor,
            binding,
            generation,
            membership_open: false,
        });
        Ok(json!({
            "transport": transport_id,
            "subscription": subscription_id,
            "authority": credential,
            "descriptor": issued["subscription"]["descriptor"],
            "descriptor_binding": issued["subscription"]["descriptor_binding"],
            "kind": request.kind,
        }))
    }

    pub(super) async fn membership(
        &self,
        transport: &str,
        subscription: &str,
        request: MembershipRequest,
    ) -> Result<Value, &'static str> {
        let mut state = self.state.lock().await;
        let issued = state.issued.as_ref().ok_or("authority_missing")?;
        if issued.transport_id != transport
            || issued.subscription_id != subscription
            || issued.credential != request.authority
        {
            return Err("membership_authority_invalid");
        }
        let membership = AsyncReferenceMembershipRequest {
            control_nonce: "reference-http-membership-1".to_owned(),
            descriptor: issued.descriptor.clone(),
            descriptor_binding: issued.binding.clone(),
            operation: "subscribe".to_owned(),
            principal: ASYNC_REFERENCE_PRINCIPAL.to_owned(),
            scope: ASYNC_REFERENCE_SCOPE.to_owned(),
            session: ASYNC_REFERENCE_SESSION.to_owned(),
            stream: AsyncReferenceScenario::lifecycle().stream.to_owned(),
            subscription_id: issued.subscription_id.clone(),
            transport_generation: issued.generation,
        };
        let credential = issued.credential.clone();
        let response =
            state
                .authority
                .membership(ASYNC_REFERENCE_ORIGIN, &credential, &membership, NOW)?;
        state
            .issued
            .as_mut()
            .ok_or("authority_missing")?
            .membership_open = true;
        self.maximum_memberships.fetch_max(1, Ordering::SeqCst);
        Ok(response)
    }

    pub(super) async fn poll(&self, request: PollRequest) -> Result<Value, &'static str> {
        let mut state = self.state.lock().await;
        let issued = state.issued.as_ref().ok_or("poll_authority_invalid")?;
        if issued.subscription_id != request.subscription || issued.credential != request.authority
        {
            return Err("poll_authority_invalid");
        }
        let credential = issued.credential.clone();
        let binding = issued.binding.clone();
        let subscription_id = issued.subscription_id.clone();
        let current = state.authority.current_sequence();
        let continuity = state.authority.poll(
            ASYNC_REFERENCE_ORIGIN,
            &credential,
            &AsyncReferencePollRequest {
                descriptor_binding: binding,
                position: AsyncReferencePosition {
                    epoch: "1".to_owned(),
                    sequence: current.to_string(),
                },
                subscription_id,
            },
            NOW,
        )?;
        state.poll_generation = state.poll_generation.saturating_add(1);
        Ok(json!({
            "render": format!(
                "<section live:stream=\"orders\" data-live-poll-generation=\"{}\"></section>",
                state.poll_generation
            ),
            "continuity": continuity,
            "operation": "fresh_render",
        }))
    }

    pub(super) async fn sse_event(
        &self,
        transport: &str,
        credential: &str,
    ) -> Result<Vec<u8>, &'static str> {
        let mut state = self.state.lock().await;
        let issued = state.issued.as_ref().ok_or("transport_authority_invalid")?;
        if issued.transport_id != transport
            || issued.credential != credential
            || !issued.membership_open
            || !state.authority.may_deliver_on(issued.physical_id)
        {
            return Err("transport_authority_invalid");
        }
        let (sequence, envelope) = if self.fault == ReferenceFaultSchedule::SequenceGapOnce
            && !self.fault_applied.swap(true, Ordering::SeqCst)
        {
            state.authority.sequence_gap()
        } else {
            state.authority.next_heartbeat()
        };
        Ok(AsyncReferenceScenario::lifecycle().sse_record(sequence, &envelope))
    }

    pub(super) async fn websocket_control(
        &self,
        transport: &str,
        payload: &[u8],
    ) -> Result<Vec<u8>, &'static str> {
        let codec = WebSocketCodec::v1();
        let control = codec
            .decode_control(WebSocketFrame::Text {
                payload,
                final_fragment: true,
            })
            .map_err(|_| "websocket_control_invalid")?;
        let state = self.state.lock().await;
        let issued = state.issued.as_ref().ok_or("transport_authority_invalid")?;
        if issued.transport_id != transport || !issued.membership_open {
            return Err("transport_authority_invalid");
        }
        let (kind, subscription) = match control {
            WebSocketControlRecord::Subscribe(subscription) => {
                ("subscribed", subscription.to_base64url())
            }
            WebSocketControlRecord::Unsubscribe(subscription) => {
                ("unsubscribed", subscription.to_base64url())
            }
        };
        if subscription != issued.subscription_id {
            return Err("membership_authority_invalid");
        }
        serde_json::to_vec(&json!({"kind": kind, "subscription": subscription}))
            .map_err(|_| "websocket_control_invalid")
    }

    pub(super) fn fault_count(&self) -> usize {
        usize::from(self.fault_applied.load(Ordering::SeqCst))
    }

    pub(super) fn maximum_memberships(&self) -> usize {
        self.maximum_memberships.load(Ordering::SeqCst)
    }

    pub(super) async fn retire(&self) {
        let mut state = self.state.lock().await;
        if let Some(issued) = state.issued.take() {
            state.authority.close_transport(issued.physical_id);
        }
    }
}
