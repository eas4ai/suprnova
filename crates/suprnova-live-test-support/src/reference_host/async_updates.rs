//! Deterministic HTTP, SSE, and WebSocket adapters over production async authority.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use axum::body::Bytes;
use serde::Deserialize;
use serde_json::{Value, json};
use suprnova_live::async_updates::{
    AuthorizedTransportSubscription, DocumentTransportKind, DocumentTransportSession, SseEncoder,
    SseMembershipControl, WebSocketCodec, WebSocketControlRecord, WebSocketFrame,
    WebSocketMembershipControl,
};
use suprnova_live::endpoint::LiveEndpointResponse;
use suprnova_live::identity::UnixMillis;
use tokio::sync::Mutex;

use crate::{
    ASYNC_REFERENCE_PRINCIPAL, ASYNC_REFERENCE_SCOPE, ASYNC_REFERENCE_SESSION,
    AsyncReferenceAuthority, AsyncReferenceMembershipRequest, AsyncReferencePollRequest,
    AsyncReferencePosition, AsyncReferenceScenario,
};

use super::engine_async::{EngineAsyncFixture, EngineSource};
use super::faults::ReferenceFaultSchedule;
use super::{ResourceCounter, ResourceLease};

const NOW: UnixMillis = UnixMillis::new(1_000);
const MEMBERSHIP_IDS: [&str; 2] = ["c3Vic2NyaXB0aW9uLTAwMQ", "c3Vic2NyaXB0aW9uLTAwMg"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TransportKind {
    Sse,
    WebSocket,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TransportCreateRequest {
    kind: String,
    subscription: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MembershipRequest {
    authority: String,
    control_nonce: String,
    operation: String,
}

pub(super) struct PollRequest {
    pub(super) subscription: String,
    pub(super) authority: String,
}

struct IssuedMembership {
    authority: AsyncReferenceAuthority,
    authority_transport: u64,
    credential: String,
    descriptor: String,
    binding: String,
    generation: u64,
    engine_authorization: AuthorizedTransportSubscription,
    open: bool,
    lease: Option<ResourceLease>,
}

struct IssuedTransport {
    kind: TransportKind,
    document: DocumentTransportSession,
    memberships: BTreeMap<String, IssuedMembership>,
}

struct AsyncState {
    engine: EngineAsyncFixture,
    transports: BTreeMap<String, IssuedTransport>,
    next_transport: u64,
    retired: bool,
}

pub(super) struct WebSocketControlOutcome {
    pub(super) messages: Vec<Vec<u8>>,
}

pub(super) struct AsyncRuntime {
    state: Mutex<AsyncState>,
    fault: ReferenceFaultSchedule,
    fault_applied: AtomicBool,
    maximum_memberships: AtomicUsize,
    logical_memberships: Arc<ResourceCounter>,
}

impl AsyncRuntime {
    pub(super) async fn new(
        fault: ReferenceFaultSchedule,
        logical_memberships: Arc<ResourceCounter>,
    ) -> Result<Self, String> {
        Ok(Self {
            state: Mutex::new(AsyncState {
                engine: EngineAsyncFixture::new().await?,
                transports: BTreeMap::new(),
                next_transport: 1,
                retired: false,
            }),
            fault,
            fault_applied: AtomicBool::new(false),
            maximum_memberships: AtomicUsize::new(0),
            logical_memberships,
        })
    }

    pub(super) async fn create(
        &self,
        request: TransportCreateRequest,
        origin: &str,
    ) -> Result<Value, &'static str> {
        let kind = match request.kind.as_str() {
            "sse" => TransportKind::Sse,
            "websocket" => TransportKind::WebSocket,
            _ => return Err("transport_facts_invalid"),
        };
        if request.subscription != AsyncReferenceScenario::lifecycle().stream {
            return Err("transport_facts_invalid");
        }
        let mut state = self.state.lock().await;
        if state.retired {
            return Err("transport_retired");
        }
        let sequence = state.next_transport;
        state.next_transport = state.next_transport.saturating_add(1);
        let transport_id = format!("transport-{sequence}");
        let document_kind = match kind {
            TransportKind::Sse => DocumentTransportKind::ServerSentEvents,
            TransportKind::WebSocket => DocumentTransportKind::WebSocket,
        };
        let marker = u8::try_from(sequence).unwrap_or(u8::MAX).max(1);
        let document = state.engine.document(origin, document_kind, marker)?;
        let mut memberships = BTreeMap::new();
        let mut response_memberships = Vec::new();
        for subscription_id in MEMBERSHIP_IDS {
            let engine_authorization = state.engine.authorization(subscription_id, origin)?;
            let mut authority = AsyncReferenceAuthority::new_with_origin_subscription(
                NOW,
                origin.to_owned(),
                subscription_id.to_owned(),
            );
            authority.install_external_authority(
                state.engine.descriptor().to_owned(),
                state.engine.descriptor_binding().to_owned(),
                state.engine.credential().to_owned(),
                state.engine.expires_at(),
            )?;
            let authority_transport =
                authority.open_transport(origin, state.engine.credential(), 1, NOW)?;
            let credential = state.engine.credential().to_owned();
            let descriptor = state.engine.descriptor().to_owned();
            let binding = state.engine.descriptor_binding().to_owned();
            response_memberships.push(json!({
                "authority": credential,
                "descriptor": descriptor,
                "descriptor_binding": binding,
                "subscription": subscription_id,
            }));
            memberships.insert(
                subscription_id.to_owned(),
                IssuedMembership {
                    authority,
                    authority_transport,
                    credential,
                    descriptor,
                    binding,
                    generation: 1,
                    engine_authorization,
                    open: false,
                    lease: None,
                },
            );
        }
        let first = response_memberships
            .first()
            .ok_or("authority_issue_failed")?;
        let response = json!({
            "transport": transport_id,
            "subscription": first["subscription"],
            "authority": first["authority"],
            "descriptor": first["descriptor"],
            "descriptor_binding": first["descriptor_binding"],
            "memberships": response_memberships,
            "kind": request.kind,
        });
        state.transports.insert(
            transport_id,
            IssuedTransport {
                kind,
                document,
                memberships,
            },
        );
        Ok(response)
    }

    pub(super) async fn membership(
        &self,
        transport_id: &str,
        subscription: &str,
        request: MembershipRequest,
    ) -> Result<Value, &'static str> {
        let mut state = self.state.lock().await;
        let AsyncState {
            engine, transports, ..
        } = &mut *state;
        let transport = transports
            .get_mut(transport_id)
            .ok_or("authority_missing")?;
        if transport.kind != TransportKind::Sse {
            return Err("membership_transport_invalid");
        }
        let membership = transport
            .memberships
            .get_mut(subscription)
            .ok_or("authority_missing")?;
        if request.authority != membership.credential {
            return Err("membership_authority_invalid");
        }
        let exact = AsyncReferenceMembershipRequest {
            control_nonce: request.control_nonce,
            descriptor: membership.descriptor.clone(),
            descriptor_binding: membership.binding.clone(),
            operation: request.operation,
            principal: ASYNC_REFERENCE_PRINCIPAL.to_owned(),
            scope: ASYNC_REFERENCE_SCOPE.to_owned(),
            session: ASYNC_REFERENCE_SESSION.to_owned(),
            stream: AsyncReferenceScenario::lifecycle().stream.to_owned(),
            subscription_id: subscription.to_owned(),
            transport_generation: membership.generation,
        };
        let response = match exact.operation.as_str() {
            "subscribe" => {
                let reference_origin = membership.authority.origin().to_owned();
                let prepared_reference = membership.authority.prepare_membership(
                    &reference_origin,
                    &membership.credential,
                    &exact,
                    NOW,
                )?;
                let lease = self
                    .logical_memberships
                    .acquire()
                    .ok_or("transport_retired")?;
                let pending = SseMembershipControl::prepare_subscribe(
                    &transport.document,
                    transport.document.handle(),
                    transport.document.origin(),
                    membership.engine_authorization.clone(),
                )
                .map_err(|_| "engine_membership_rejected")?;
                let authorized = pending
                    .authorize()
                    .await
                    .map_err(|_| "engine_membership_rejected")?;
                let establishing = transport
                    .document
                    .prepare_establish(authorized)
                    .map_err(|_| "engine_membership_rejected")?;
                let ready = establishing
                    .establish(&EngineSource)
                    .await
                    .map_err(|_| "engine_membership_rejected")?;
                transport
                    .document
                    .commit_add(ready)
                    .map_err(|_| "engine_membership_rejected")?;
                let response = membership
                    .authority
                    .commit_membership(prepared_reference, NOW)?;
                membership.open = true;
                membership.lease = Some(lease);
                response
            }
            "unsubscribe" => {
                let reference_origin = membership.authority.origin().to_owned();
                let prepared_reference = membership.authority.prepare_membership(
                    &reference_origin,
                    &membership.credential,
                    &exact,
                    NOW,
                )?;
                let pending = SseMembershipControl::prepare_unsubscribe(
                    &transport.document,
                    transport.document.handle(),
                    transport.document.origin(),
                    &membership.engine_authorization,
                )
                .map_err(|_| "engine_membership_rejected")?;
                let ready = pending
                    .authorize()
                    .await
                    .map_err(|_| "engine_membership_rejected")?;
                transport
                    .document
                    .commit_remove(ready)
                    .map_err(|_| "engine_membership_rejected")?;
                let response = membership
                    .authority
                    .commit_membership(prepared_reference, NOW)?;
                engine.remove(&membership.engine_authorization);
                membership.open = false;
                drop(membership.lease.take());
                response
            }
            _ => return Err("membership_facts_invalid"),
        };
        self.maximum_memberships
            .fetch_max(transport.document.membership_count(), Ordering::SeqCst);
        Ok(response)
    }

    pub(super) async fn poll(
        &self,
        request: PollRequest,
        body: Bytes,
    ) -> Result<LiveEndpointResponse, &'static str> {
        let endpoint = {
            let state = self.state.lock().await;
            let membership = state
                .transports
                .values()
                .flat_map(|transport| transport.memberships.values())
                .find(|membership| {
                    membership
                        .engine_authorization
                        .subscription()
                        .to_base64url()
                        == request.subscription
                        && membership.credential == request.authority
                })
                .ok_or("poll_authority_invalid")?;
            let current = membership.authority.current_sequence();
            let _continuity = membership.authority.poll(
                membership.authority.origin(),
                &membership.credential,
                &AsyncReferencePollRequest {
                    descriptor_binding: membership.binding.clone(),
                    position: AsyncReferencePosition {
                        epoch: "1".to_owned(),
                        sequence: current.to_string(),
                    },
                    subscription_id: request.subscription,
                },
                NOW,
            )?;
            state.engine.fresh_render_endpoint()
        };
        endpoint.handle(body).await
    }

    pub(super) async fn fresh_render_request(
        &self,
        correlation: &str,
        seed: u8,
    ) -> Result<String, &'static str> {
        let endpoint = {
            let state = self.state.lock().await;
            state.engine.fresh_render_endpoint()
        };
        endpoint.request(correlation, seed).await
    }

    pub(super) async fn fresh_render_document(&self) -> String {
        let endpoint = {
            let state = self.state.lock().await;
            state.engine.fresh_render_endpoint()
        };
        endpoint.initial_html().to_owned()
    }

    pub(super) async fn sse_batch(&self, transport_id: &str) -> Result<Vec<u8>, &'static str> {
        let mut state = self.state.lock().await;
        if state.retired {
            return Err("transport_retired");
        }
        let AsyncState {
            engine, transports, ..
        } = &mut *state;
        let transport = transports
            .get_mut(transport_id)
            .ok_or("transport_authority_invalid")?;
        if transport.kind != TransportKind::Sse {
            return Err("transport_authority_invalid");
        }
        let mut bytes = Vec::new();
        for membership in transport
            .memberships
            .values_mut()
            .filter(|membership| membership.open)
        {
            if !membership
                .authority
                .may_deliver_on(membership.authority_transport)
            {
                continue;
            }
            let sequence = if self.fault == ReferenceFaultSchedule::SequenceGapOnce
                && !self.fault_applied.swap(true, Ordering::SeqCst)
            {
                membership.authority.sequence_gap().0
            } else {
                membership.authority.next_heartbeat().0
            };
            let envelope = engine.envelope(&membership.engine_authorization, sequence)?;
            let event =
                SseEncoder::encode_envelope(&envelope).map_err(|_| "engine_envelope_invalid")?;
            bytes.extend_from_slice(event.as_bytes());
        }
        if bytes.is_empty() {
            return Err("transport_authority_invalid");
        }
        Ok(bytes)
    }

    pub(super) async fn websocket_control(
        &self,
        transport_id: &str,
        payload: &[u8],
    ) -> Result<WebSocketControlOutcome, &'static str> {
        let codec = WebSocketCodec::v1();
        let mut state = self.state.lock().await;
        let AsyncState {
            engine, transports, ..
        } = &mut *state;
        let transport = transports
            .get_mut(transport_id)
            .ok_or("transport_authority_invalid")?;
        if transport.kind != TransportKind::WebSocket {
            return Err("transport_authority_invalid");
        }
        if let Ok(request) = codec.decode_membership_request(WebSocketFrame::Text {
            payload,
            final_fragment: true,
        }) {
            let subscription = request.subscription().to_base64url();
            let membership = transport
                .memberships
                .get_mut(&subscription)
                .ok_or("membership_authority_invalid")?;
            let reference = AsyncReferenceMembershipRequest {
                control_nonce: request.control_nonce().to_owned(),
                descriptor: membership.descriptor.clone(),
                descriptor_binding: membership.binding.clone(),
                operation: "subscribe".to_owned(),
                principal: ASYNC_REFERENCE_PRINCIPAL.to_owned(),
                scope: ASYNC_REFERENCE_SCOPE.to_owned(),
                session: ASYNC_REFERENCE_SESSION.to_owned(),
                stream: AsyncReferenceScenario::lifecycle().stream.to_owned(),
                subscription_id: subscription,
                transport_generation: request.transport_generation(),
            };
            let reference_origin = membership.authority.origin().to_owned();
            let prepared_reference = membership.authority.prepare_membership(
                &reference_origin,
                &membership.credential,
                &reference,
                NOW,
            )?;
            let lease = self
                .logical_memberships
                .acquire()
                .ok_or("transport_retired")?;
            let pending = WebSocketMembershipControl::prepare_authenticated_subscribe(
                &transport.document,
                request,
                membership.engine_authorization.clone(),
            )
            .map_err(|_| "websocket_control_invalid")?;
            let authorized = pending
                .authorize()
                .await
                .map_err(|_| "websocket_control_invalid")?;
            let establishing = authorized
                .prepare_establish(&transport.document)
                .map_err(|_| "websocket_control_invalid")?;
            let ready = establishing
                .establish(&EngineSource)
                .await
                .map_err(|_| "websocket_control_invalid")?;
            let receipt = WebSocketMembershipControl::commit_authenticated_subscribe(
                &mut transport.document,
                ready,
            )
            .map_err(|_| "websocket_control_invalid")?;
            membership
                .authority
                .commit_membership(prepared_reference, NOW)?;
            membership.open = true;
            membership.lease = Some(lease);
            self.maximum_memberships
                .fetch_max(transport.document.membership_count(), Ordering::SeqCst);
            let acknowledgment = WebSocketMembershipControl::acknowledge_committed(receipt);
            let ack = codec
                .encode_membership_acknowledgment(&acknowledgment)
                .map_err(|_| "websocket_control_invalid")?;
            let envelope = engine.envelope(&membership.engine_authorization, 1)?;
            let envelope = codec
                .encode_envelope(&envelope)
                .map_err(|_| "websocket_control_invalid")?;
            return Ok(WebSocketControlOutcome {
                messages: vec![ack, envelope],
            });
        }
        let control = codec
            .decode_control(WebSocketFrame::Text {
                payload,
                final_fragment: true,
            })
            .map_err(|_| "websocket_control_invalid")?;
        let WebSocketControlRecord::Unsubscribe(subscription) = control else {
            return Err("websocket_control_invalid");
        };
        let subscription_wire = subscription.to_base64url();
        let membership = transport
            .memberships
            .get_mut(&subscription_wire)
            .ok_or("membership_authority_invalid")?;
        let reference = AsyncReferenceMembershipRequest {
            control_nonce: "ws-unsubscribe-1".to_owned(),
            descriptor: membership.descriptor.clone(),
            descriptor_binding: membership.binding.clone(),
            operation: "unsubscribe".to_owned(),
            principal: ASYNC_REFERENCE_PRINCIPAL.to_owned(),
            scope: ASYNC_REFERENCE_SCOPE.to_owned(),
            session: ASYNC_REFERENCE_SESSION.to_owned(),
            stream: AsyncReferenceScenario::lifecycle().stream.to_owned(),
            subscription_id: subscription_wire.clone(),
            transport_generation: membership.generation,
        };
        let reference_origin = membership.authority.origin().to_owned();
        let prepared_reference = membership.authority.prepare_membership(
            &reference_origin,
            &membership.credential,
            &reference,
            NOW,
        )?;
        let unsubscribe = WebSocketControlRecord::Unsubscribe(subscription);
        let pending = WebSocketMembershipControl::prepare_unsubscribe(
            &transport.document,
            &unsubscribe,
            &membership.engine_authorization,
        )
        .map_err(|_| "websocket_control_invalid")?;
        let ready = pending
            .authorize()
            .await
            .map_err(|_| "websocket_control_invalid")?;
        transport
            .document
            .commit_remove(ready)
            .map_err(|_| "websocket_control_invalid")?;
        membership
            .authority
            .commit_membership(prepared_reference, NOW)?;
        engine.remove(&membership.engine_authorization);
        membership.open = false;
        drop(membership.lease.take());
        Ok(WebSocketControlOutcome {
            messages: vec![
                serde_json::to_vec(&json!({
                    "kind": "unsubscribed",
                    "subscription": subscription_wire
                }))
                .map_err(|_| "websocket_control_invalid")?,
            ],
        })
    }

    pub(super) fn fault_count(&self) -> usize {
        usize::from(self.fault_applied.load(Ordering::SeqCst))
    }

    pub(super) fn maximum_memberships(&self) -> usize {
        self.maximum_memberships.load(Ordering::SeqCst)
    }

    pub(super) async fn retire(&self) {
        let mut state = self.state.lock().await;
        state.retired = true;
        for transport in state.transports.values_mut() {
            for membership in transport.memberships.values_mut() {
                membership
                    .authority
                    .close_transport(membership.authority_transport);
                membership.open = false;
                drop(membership.lease.take());
            }
            let _ = transport.document.close().await;
        }
        state.transports.clear();
    }
}
