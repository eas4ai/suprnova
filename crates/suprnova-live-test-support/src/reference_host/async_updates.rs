//! Deterministic HTTP, SSE, and WebSocket adapters over production async authority.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

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
use tokio::sync::Notify;

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
const MAX_DOCUMENT_TRANSPORTS: usize = 2;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum TransportKind {
    Sse,
    WebSocket,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TransportCreateRequest {
    kind: String,
    position: Option<TransportPosition>,
    prior_subscription: Option<String>,
    subscription: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TransportPosition {
    epoch: String,
    sequence: String,
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
    control_in_flight: bool,
    lease: Option<ResourceLease>,
}

struct IssuedTransport {
    kind: TransportKind,
    generation: u64,
    reader_active: bool,
    document: DocumentTransportSession,
    memberships: BTreeMap<String, IssuedMembership>,
}

struct AsyncState {
    engine: Arc<EngineAsyncFixture>,
    transports: BTreeMap<String, IssuedTransport>,
    by_kind: BTreeMap<TransportKind, String>,
    continuity_authorities: BTreeMap<(TransportKind, String), AsyncReferenceAuthority>,
    next_transport: u64,
    retired: bool,
}

#[derive(Default)]
struct AsyncPhasePause {
    subscription: Mutex<Option<String>>,
    entered: Notify,
    release: Notify,
}

impl AsyncPhasePause {
    async fn pause_if_selected(&self, subscription: &str) {
        if self
            .subscription
            .lock()
            .expect("async phase pause lock")
            .as_deref()
            == Some(subscription)
        {
            self.entered.notify_one();
            self.release.notified().await;
        }
    }

    #[cfg(test)]
    fn select(&self, subscription: &str) {
        *self.subscription.lock().expect("async phase pause lock") = Some(subscription.to_owned());
    }

    #[cfg(test)]
    async fn wait_until_entered(&self) {
        self.entered.notified().await;
    }

    #[cfg(test)]
    fn resume(&self) {
        self.subscription
            .lock()
            .expect("async phase pause lock")
            .take();
        self.release.notify_waiters();
    }
}

pub(super) struct WebSocketControlOutcome {
    pub(super) messages: Vec<Vec<u8>>,
}

pub(super) struct AsyncRuntime {
    state: Arc<Mutex<AsyncState>>,
    fault: ReferenceFaultSchedule,
    fault_applied: AtomicBool,
    maximum_memberships: AtomicUsize,
    logical_memberships: Arc<ResourceCounter>,
    phase_pause: Arc<AsyncPhasePause>,
}

pub(super) struct TransportReaderLease {
    state: Arc<Mutex<AsyncState>>,
    transport: String,
    generation: u64,
}

impl Drop for TransportReaderLease {
    fn drop(&mut self) {
        let mut state = self.state.lock().expect("async runtime lock");
        let Some(current) = state.transports.get(&self.transport) else {
            return;
        };
        if current.generation != self.generation {
            return;
        }
        let kind = current.kind;
        let Some(mut transport) = state.transports.remove(&self.transport) else {
            return;
        };
        state.by_kind.remove(&kind);
        let engine = Arc::clone(&state.engine);
        for (subscription, mut membership) in transport.memberships {
            membership
                .authority
                .close_transport(membership.authority_transport);
            membership.open = false;
            engine.remove(&membership.engine_authorization);
            drop(membership.lease.take());
            state
                .continuity_authorities
                .insert((kind, subscription), membership.authority);
        }
        drop(state);
        tokio::spawn(async move {
            let _ = transport.document.close().await;
        });
    }
}

struct MembershipControlLease {
    state: Arc<Mutex<AsyncState>>,
    transport: String,
    subscription: String,
    generation: u64,
}

impl Drop for MembershipControlLease {
    fn drop(&mut self) {
        let mut state = self.state.lock().expect("async runtime lock");
        if let Some(transport) = state.transports.get_mut(&self.transport)
            && transport.generation == self.generation
            && let Some(membership) = transport.memberships.get_mut(&self.subscription)
        {
            membership.control_in_flight = false;
        }
    }
}

impl AsyncRuntime {
    pub(super) async fn new(
        fault: ReferenceFaultSchedule,
        logical_memberships: Arc<ResourceCounter>,
    ) -> Result<Self, String> {
        Ok(Self {
            state: Arc::new(Mutex::new(AsyncState {
                engine: Arc::new(EngineAsyncFixture::new().await?),
                transports: BTreeMap::new(),
                by_kind: BTreeMap::new(),
                continuity_authorities: BTreeMap::new(),
                next_transport: 1,
                retired: false,
            })),
            fault,
            fault_applied: AtomicBool::new(false),
            maximum_memberships: AtomicUsize::new(0),
            logical_memberships,
            phase_pause: Arc::new(AsyncPhasePause::default()),
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
        let replay_from = match (&request.prior_subscription, &request.position) {
            (None, None) => None,
            (Some(subscription), Some(position))
                if MEMBERSHIP_IDS.contains(&subscription.as_str()) =>
            {
                Some((subscription.as_str(), position))
            }
            _ => return Err("transport_facts_invalid"),
        };
        let mut state = self.state.lock().expect("async runtime lock");
        if state.retired {
            return Err("transport_retired");
        }
        if let Some(transport_id) = state.by_kind.get(&kind).cloned() {
            let transport = state
                .transports
                .get(&transport_id)
                .ok_or("transport_authority_invalid")?;
            return transport_response(
                &transport_id,
                transport,
                state.engine.as_ref(),
                replay_from,
            );
        }
        if state.transports.len() >= MAX_DOCUMENT_TRANSPORTS {
            return Err("transport_capacity_exceeded");
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
        for subscription_id in MEMBERSHIP_IDS {
            let engine_authorization = state.engine.authorization(subscription_id, origin)?;
            let mut authority = state
                .continuity_authorities
                .remove(&(kind, subscription_id.to_owned()))
                .unwrap_or_else(|| {
                    AsyncReferenceAuthority::new_with_origin_subscription(
                        NOW,
                        origin.to_owned(),
                        subscription_id.to_owned(),
                    )
                });
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
                    control_in_flight: false,
                    lease: None,
                },
            );
        }
        state.transports.insert(
            transport_id.clone(),
            IssuedTransport {
                kind,
                generation: 1,
                reader_active: false,
                document,
                memberships,
            },
        );
        state.by_kind.insert(kind, transport_id.clone());
        let transport = state
            .transports
            .get(&transport_id)
            .ok_or("transport_authority_invalid")?;
        transport_response(&transport_id, transport, state.engine.as_ref(), replay_from)
    }

    pub(super) async fn membership(
        &self,
        transport_id: &str,
        subscription: &str,
        request: MembershipRequest,
    ) -> Result<Value, &'static str> {
        let operation = request.operation.clone();
        let (exact, authorization, generation) = {
            let state = self.state.lock().expect("async runtime lock");
            let transport = state
                .transports
                .get(transport_id)
                .ok_or("authority_missing")?;
            if transport.kind != TransportKind::Sse {
                return Err("membership_transport_invalid");
            }
            let membership = transport
                .memberships
                .get(subscription)
                .ok_or("authority_missing")?;
            if request.authority != membership.credential {
                return Err("membership_authority_invalid");
            }
            (
                AsyncReferenceMembershipRequest {
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
                },
                membership.engine_authorization.clone(),
                transport.generation,
            )
        };
        match operation.as_str() {
            "subscribe" => {
                let (prepared_reference, pending, control) = {
                    let mut state = self.state.lock().expect("async runtime lock");
                    let transport = state
                        .transports
                        .get_mut(transport_id)
                        .filter(|transport| transport.generation == generation)
                        .ok_or("transport_authority_invalid")?;
                    let membership = transport
                        .memberships
                        .get_mut(subscription)
                        .ok_or("authority_missing")?;
                    let reference_origin = membership.authority.origin().to_owned();
                    let prepared_reference = membership.authority.prepare_membership(
                        &reference_origin,
                        &membership.credential,
                        &exact,
                        NOW,
                    )?;
                    if membership.control_in_flight {
                        return Err("membership_control_in_flight");
                    }
                    let pending = SseMembershipControl::prepare_subscribe(
                        &transport.document,
                        transport.document.handle(),
                        transport.document.origin(),
                        authorization,
                    )
                    .map_err(|_| "engine_membership_rejected")?;
                    membership.control_in_flight = true;
                    let control = MembershipControlLease {
                        state: Arc::clone(&self.state),
                        transport: transport_id.to_owned(),
                        subscription: subscription.to_owned(),
                        generation,
                    };
                    (prepared_reference, pending, control)
                };
                let lease = self
                    .logical_memberships
                    .acquire()
                    .ok_or("transport_retired")?;
                self.phase_pause.pause_if_selected(subscription).await;
                let authorized = pending
                    .authorize()
                    .await
                    .map_err(|_| "engine_membership_rejected")?;
                let establishing = {
                    let mut state = self.state.lock().expect("async runtime lock");
                    let transport = state
                        .transports
                        .get_mut(transport_id)
                        .filter(|transport| transport.generation == generation)
                        .ok_or("transport_authority_invalid")?;
                    transport
                        .document
                        .prepare_establish(authorized)
                        .map_err(|_| "engine_membership_rejected")?
                };
                let ready = establishing
                    .establish(&EngineSource)
                    .await
                    .map_err(|_| "engine_membership_rejected")?;
                let response = {
                    let mut state = self.state.lock().expect("async runtime lock");
                    let transport = state
                        .transports
                        .get_mut(transport_id)
                        .filter(|transport| transport.generation == generation)
                        .ok_or("transport_authority_invalid")?;
                    transport
                        .document
                        .commit_add(ready)
                        .map_err(|_| "engine_membership_rejected")?;
                    let membership = transport
                        .memberships
                        .get_mut(subscription)
                        .ok_or("authority_missing")?;
                    let response = membership
                        .authority
                        .commit_membership(prepared_reference, NOW)?;
                    membership.open = true;
                    membership.lease = Some(lease);
                    self.maximum_memberships
                        .fetch_max(transport.document.membership_count(), Ordering::SeqCst);
                    response
                };
                drop(control);
                Ok(response)
            }
            "unsubscribe" => {
                let (prepared_reference, pending, control) = {
                    let mut state = self.state.lock().expect("async runtime lock");
                    let transport = state
                        .transports
                        .get_mut(transport_id)
                        .filter(|transport| transport.generation == generation)
                        .ok_or("transport_authority_invalid")?;
                    let membership = transport
                        .memberships
                        .get_mut(subscription)
                        .ok_or("authority_missing")?;
                    let reference_origin = membership.authority.origin().to_owned();
                    let prepared_reference = membership.authority.prepare_membership(
                        &reference_origin,
                        &membership.credential,
                        &exact,
                        NOW,
                    )?;
                    if membership.control_in_flight {
                        return Err("membership_control_in_flight");
                    }
                    let pending = SseMembershipControl::prepare_unsubscribe(
                        &transport.document,
                        transport.document.handle(),
                        transport.document.origin(),
                        &authorization,
                    )
                    .map_err(|_| "engine_membership_rejected")?;
                    membership.control_in_flight = true;
                    let control = MembershipControlLease {
                        state: Arc::clone(&self.state),
                        transport: transport_id.to_owned(),
                        subscription: subscription.to_owned(),
                        generation,
                    };
                    (prepared_reference, pending, control)
                };
                let ready = pending
                    .authorize()
                    .await
                    .map_err(|_| "engine_membership_rejected")?;
                let response = {
                    let mut state = self.state.lock().expect("async runtime lock");
                    let engine = Arc::clone(&state.engine);
                    let transport = state
                        .transports
                        .get_mut(transport_id)
                        .filter(|transport| transport.generation == generation)
                        .ok_or("transport_authority_invalid")?;
                    transport
                        .document
                        .commit_remove(ready)
                        .map_err(|_| "engine_membership_rejected")?;
                    let membership = transport
                        .memberships
                        .get_mut(subscription)
                        .ok_or("authority_missing")?;
                    let response = membership
                        .authority
                        .commit_membership(prepared_reference, NOW)?;
                    engine.remove(&membership.engine_authorization);
                    membership.open = false;
                    drop(membership.lease.take());
                    response
                };
                drop(control);
                Ok(response)
            }
            _ => Err("membership_facts_invalid"),
        }
    }

    pub(super) async fn poll(
        &self,
        request: PollRequest,
        body: Bytes,
    ) -> Result<LiveEndpointResponse, &'static str> {
        let endpoint = {
            let state = self.state.lock().expect("async runtime lock");
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
            let state = self.state.lock().expect("async runtime lock");
            state.engine.fresh_render_endpoint()
        };
        endpoint.request(correlation, seed).await
    }

    pub(super) async fn fresh_render_document(&self) -> String {
        let endpoint = {
            let state = self.state.lock().expect("async runtime lock");
            state.engine.fresh_render_endpoint()
        };
        endpoint.initial_html().to_owned()
    }

    pub(super) async fn reset_fresh_render(&self) -> Result<(), String> {
        let engine = {
            let state = self.state.lock().expect("async runtime lock");
            Arc::clone(&state.engine)
        };
        engine.reset_fresh_render().await
    }

    pub(super) fn pause_fresh_render(&self) {
        let endpoint = {
            let state = self.state.lock().expect("async runtime lock");
            state.engine.fresh_render_endpoint()
        };
        endpoint.pause_render();
    }

    pub(super) async fn wait_until_fresh_render_paused(&self) {
        let endpoint = {
            let state = self.state.lock().expect("async runtime lock");
            state.engine.fresh_render_endpoint()
        };
        endpoint.wait_until_render_paused().await;
    }

    pub(super) fn resume_fresh_render(&self) {
        let endpoint = {
            let state = self.state.lock().expect("async runtime lock");
            state.engine.fresh_render_endpoint()
        };
        endpoint.resume_render();
    }

    pub(super) async fn execute_fresh_render_direct(
        &self,
        body: Bytes,
    ) -> Result<LiveEndpointResponse, &'static str> {
        let endpoint = {
            let state = self.state.lock().expect("async runtime lock");
            state.engine.fresh_render_endpoint()
        };
        endpoint.handle(body).await
    }

    pub(super) async fn sse_batch(&self, transport_id: &str) -> Result<Vec<u8>, &'static str> {
        let mut state = self.state.lock().expect("async runtime lock");
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
        if let Ok(request) = codec.decode_membership_request(WebSocketFrame::Text {
            payload,
            final_fragment: true,
        }) {
            let subscription = request.subscription().to_base64url();
            let (prepared_reference, pending, generation, control) = {
                let mut state = self.state.lock().expect("async runtime lock");
                let transport = state
                    .transports
                    .get_mut(transport_id)
                    .ok_or("transport_authority_invalid")?;
                if transport.kind != TransportKind::WebSocket {
                    return Err("transport_authority_invalid");
                }
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
                    subscription_id: subscription.clone(),
                    transport_generation: request.transport_generation(),
                };
                let reference_origin = membership.authority.origin().to_owned();
                let prepared_reference = membership.authority.prepare_membership(
                    &reference_origin,
                    &membership.credential,
                    &reference,
                    NOW,
                )?;
                if membership.control_in_flight {
                    return Err("membership_control_in_flight");
                }
                let pending = WebSocketMembershipControl::prepare_authenticated_subscribe(
                    &transport.document,
                    request,
                    membership.engine_authorization.clone(),
                )
                .map_err(|_| "websocket_control_invalid")?;
                membership.control_in_flight = true;
                let generation = transport.generation;
                let control = MembershipControlLease {
                    state: Arc::clone(&self.state),
                    transport: transport_id.to_owned(),
                    subscription: subscription.clone(),
                    generation,
                };
                (prepared_reference, pending, generation, control)
            };
            let lease = self
                .logical_memberships
                .acquire()
                .ok_or("transport_retired")?;
            let authorized = pending
                .authorize()
                .await
                .map_err(|_| "websocket_control_invalid")?;
            let establishing = {
                let state = self.state.lock().expect("async runtime lock");
                let transport = state
                    .transports
                    .get(transport_id)
                    .filter(|transport| transport.generation == generation)
                    .ok_or("transport_authority_invalid")?;
                authorized
                    .prepare_establish(&transport.document)
                    .map_err(|_| "websocket_control_invalid")?
            };
            let ready = establishing
                .establish(&EngineSource)
                .await
                .map_err(|_| "websocket_control_invalid")?;
            let (receipt, envelope) = {
                let mut state = self.state.lock().expect("async runtime lock");
                let engine = Arc::clone(&state.engine);
                let transport = state
                    .transports
                    .get_mut(transport_id)
                    .filter(|transport| transport.generation == generation)
                    .ok_or("transport_authority_invalid")?;
                let receipt = WebSocketMembershipControl::commit_authenticated_subscribe(
                    &mut transport.document,
                    ready,
                )
                .map_err(|_| "websocket_control_invalid")?;
                let membership = transport
                    .memberships
                    .get_mut(&subscription)
                    .ok_or("membership_authority_invalid")?;
                membership
                    .authority
                    .commit_membership(prepared_reference, NOW)?;
                membership.open = true;
                membership.lease = Some(lease);
                self.maximum_memberships
                    .fetch_max(transport.document.membership_count(), Ordering::SeqCst);
                let sequence = membership.authority.next_heartbeat().0;
                let envelope = engine.envelope(&membership.engine_authorization, sequence)?;
                (receipt, envelope)
            };
            drop(control);
            let acknowledgment = WebSocketMembershipControl::acknowledge_committed(receipt);
            let ack = codec
                .encode_membership_acknowledgment(&acknowledgment)
                .map_err(|_| "websocket_control_invalid")?;
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
        let unsubscribe = WebSocketControlRecord::Unsubscribe(subscription);
        let (prepared_reference, authorization, generation, control) = {
            let mut state = self.state.lock().expect("async runtime lock");
            let transport = state
                .transports
                .get_mut(transport_id)
                .ok_or("transport_authority_invalid")?;
            if transport.kind != TransportKind::WebSocket {
                return Err("transport_authority_invalid");
            }
            let membership = transport
                .memberships
                .get_mut(&subscription_wire)
                .ok_or("membership_authority_invalid")?;
            let reference = AsyncReferenceMembershipRequest {
                control_nonce: format!(
                    "ws-unsubscribe-{}",
                    membership.authority.current_sequence()
                ),
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
            if membership.control_in_flight {
                return Err("membership_control_in_flight");
            }
            membership.control_in_flight = true;
            let generation = transport.generation;
            let control = MembershipControlLease {
                state: Arc::clone(&self.state),
                transport: transport_id.to_owned(),
                subscription: subscription_wire.clone(),
                generation,
            };
            (
                prepared_reference,
                membership.engine_authorization.clone(),
                generation,
                control,
            )
        };
        let pending = {
            let state = self.state.lock().expect("async runtime lock");
            let transport = state
                .transports
                .get(transport_id)
                .filter(|transport| transport.generation == generation)
                .ok_or("transport_authority_invalid")?;
            WebSocketMembershipControl::prepare_unsubscribe(
                &transport.document,
                &unsubscribe,
                &authorization,
            )
            .map_err(|_| "websocket_control_invalid")?
        };
        let ready = pending
            .authorize()
            .await
            .map_err(|_| "websocket_control_invalid")?;
        {
            let mut state = self.state.lock().expect("async runtime lock");
            let engine = Arc::clone(&state.engine);
            let transport = state
                .transports
                .get_mut(transport_id)
                .filter(|transport| transport.generation == generation)
                .ok_or("transport_authority_invalid")?;
            transport
                .document
                .commit_remove(ready)
                .map_err(|_| "websocket_control_invalid")?;
            let membership = transport
                .memberships
                .get_mut(&subscription_wire)
                .ok_or("membership_authority_invalid")?;
            membership
                .authority
                .commit_membership(prepared_reference, NOW)?;
            engine.remove(&membership.engine_authorization);
            membership.open = false;
            drop(membership.lease.take());
        }
        drop(control);
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

    pub(super) fn acquire_reader(
        &self,
        transport_id: &str,
        kind: DocumentTransportKind,
    ) -> Result<TransportReaderLease, &'static str> {
        let mut state = self.state.lock().expect("async runtime lock");
        if state.retired {
            return Err("transport_retired");
        }
        let transport = state
            .transports
            .get_mut(transport_id)
            .ok_or("transport_authority_invalid")?;
        let expected = match transport.kind {
            TransportKind::Sse => DocumentTransportKind::ServerSentEvents,
            TransportKind::WebSocket => DocumentTransportKind::WebSocket,
        };
        if expected != kind {
            return Err("transport_authority_invalid");
        }
        if transport.reader_active {
            return Err("transport_reader_exists");
        }
        transport.reader_active = true;
        Ok(TransportReaderLease {
            state: Arc::clone(&self.state),
            transport: transport_id.to_owned(),
            generation: transport.generation,
        })
    }

    pub(super) fn websocket_batch(&self, transport_id: &str) -> Result<Vec<Vec<u8>>, &'static str> {
        let codec = WebSocketCodec::v1();
        let mut state = self.state.lock().expect("async runtime lock");
        if state.retired {
            return Err("transport_retired");
        }
        let engine = Arc::clone(&state.engine);
        let transport = state
            .transports
            .get_mut(transport_id)
            .ok_or("transport_authority_invalid")?;
        if transport.kind != TransportKind::WebSocket {
            return Err("transport_authority_invalid");
        }
        let mut messages = Vec::new();
        for membership in transport
            .memberships
            .values_mut()
            .filter(|membership| membership.open)
        {
            let sequence = membership.authority.next_heartbeat().0;
            let envelope = engine.envelope(&membership.engine_authorization, sequence)?;
            messages.push(
                codec
                    .encode_envelope(&envelope)
                    .map_err(|_| "engine_envelope_invalid")?,
            );
        }
        Ok(messages)
    }

    pub(super) fn fault_count(&self) -> usize {
        usize::from(self.fault_applied.load(Ordering::SeqCst))
    }

    pub(super) fn reset_sequence_gap(&self) -> bool {
        if self.fault != ReferenceFaultSchedule::SequenceGapOnce {
            return false;
        }
        self.fault_applied.store(false, Ordering::SeqCst);
        true
    }

    pub(super) fn maximum_memberships(&self) -> usize {
        self.maximum_memberships.load(Ordering::SeqCst)
    }

    pub(super) async fn retire(&self) -> Result<(), String> {
        let mut transports = {
            let mut state = self.state.lock().expect("async runtime lock");
            state.retired = true;
            state.by_kind.clear();
            std::mem::take(&mut state.transports)
        };
        let mut first_error = None;
        for transport in transports.values_mut() {
            for membership in transport.memberships.values_mut() {
                membership
                    .authority
                    .close_transport(membership.authority_transport);
                membership.open = false;
                drop(membership.lease.take());
            }
            if let Err(error) = transport.document.close().await
                && first_error.is_none()
            {
                first_error = Some(format!("async document retirement: {error:?}"));
            }
        }
        transports.clear();
        first_error.map_or(Ok(()), Err)
    }
}

fn transport_response(
    transport_id: &str,
    transport: &IssuedTransport,
    engine: &EngineAsyncFixture,
    replay_from: Option<(&str, &TransportPosition)>,
) -> Result<Value, &'static str> {
    let transport_kind = match transport.kind {
        TransportKind::Sse => "sse",
        TransportKind::WebSocket => "websocket",
    };
    let memberships = transport
        .memberships
        .iter()
        .map(|(subscription, membership)| {
            let (replay, baseline_epoch, baseline_sequence) = match replay_from {
                Some((prior, position)) if prior == subscription => {
                    let proof = membership.authority.poll(
                        membership.authority.origin(),
                        &membership.credential,
                        &AsyncReferencePollRequest {
                            descriptor_binding: membership.binding.clone(),
                            position: AsyncReferencePosition {
                                epoch: position.epoch.clone(),
                                sequence: position.sequence.clone(),
                            },
                            subscription_id: subscription.clone(),
                        },
                        NOW,
                    )?;
                    let count = proof["envelopes"]
                        .as_array()
                        .ok_or("continuity_proof_invalid")?
                        .len();
                    let current = membership.authority.current_sequence();
                    let first = current
                        .checked_add(1)
                        .and_then(|after| after.checked_sub(count as u64))
                        .ok_or("continuity_proof_invalid")?;
                    let replay = (first..=current)
                        .map(|sequence| {
                            let envelope =
                                engine.envelope(&membership.engine_authorization, sequence)?;
                            let encoded = WebSocketCodec::v1()
                                .encode_envelope(&envelope)
                                .map_err(|_| "engine_envelope_invalid")?;
                            String::from_utf8(encoded).map_err(|_| "engine_envelope_invalid")
                        })
                        .collect::<Result<Vec<_>, &'static str>>()?;
                    (replay, position.epoch.clone(), position.sequence.clone())
                }
                _ => (
                    Vec::new(),
                    "1".to_owned(),
                    membership.authority.current_sequence().to_string(),
                ),
            };
            Ok(json!({
                "authority": membership.credential,
                "descriptor": membership.descriptor,
                "descriptor_binding": membership.binding,
                "subscription": subscription,
                "browser_authorization": {
                    "authorization": {
                        "credential": membership.credential,
                        "kind": "bearer",
                    },
                    "baseline": {
                        "epoch": baseline_epoch,
                        "sequence": baseline_sequence,
                    },
                    "document": {
                        "authorization_scope": ASYNC_REFERENCE_SCOPE,
                        "transport": transport_kind,
                    },
                    "events": [],
                    "expires_at": 60_000,
                    "fallback_poll": {
                        "initial": "wait",
                        "interval_ms": 30_000,
                        "jitter_ratio": 0,
                        "visibility": "visible",
                    },
                    "heartbeat_timeout_ms": 10_000,
                    "presentation_signals": [],
                    "replay": replay,
                    "reconnect": {
                        "kind": "resume_or_refresh",
                        "maximum_attempts": 4,
                        "maximum_delay_ms": 1_000,
                        "minimum_delay_ms": 250,
                    },
                    "stream": AsyncReferenceScenario::lifecycle().stream,
                    "subscription_id": subscription,
                },
            }))
        })
        .collect::<Result<Vec<_>, &'static str>>()?;
    let first = memberships.first().expect("reference memberships");
    Ok(json!({
        "transport": transport_id,
        "subscription": first["subscription"],
        "authority": first["authority"],
        "descriptor": first["descriptor"],
        "descriptor_binding": first["descriptor_binding"],
        "memberships": memberships,
        "kind": transport_kind,
        "transport_generation": transport.generation,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{Duration, timeout};

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn websocket_initial_and_ongoing_frames_share_one_sequence_authority() {
        let resources = Arc::new(ResourceCounter::default());
        let runtime = AsyncRuntime::new(ReferenceFaultSchedule::None, Arc::clone(&resources))
            .await
            .expect("async runtime");
        let created = runtime
            .create(
                TransportCreateRequest {
                    kind: "websocket".to_owned(),
                    position: None,
                    prior_subscription: None,
                    subscription: "orders".to_owned(),
                },
                "http://127.0.0.1:4197",
            )
            .await
            .expect("websocket transport");
        let transport = created["transport"].as_str().expect("transport");
        let membership = &created["memberships"][0];
        let subscription = membership["subscription"].as_str().expect("subscription");
        let binding = membership["descriptor_binding"]
            .as_str()
            .expect("descriptor binding");
        let control = serde_json::to_vec(&json!({
            "control_nonce": "0000000000000001",
            "descriptor_binding": binding,
            "kind": "subscribe",
            "stream": "orders",
            "subscription": subscription,
            "transport_generation": 1,
        }))
        .expect("control JSON");
        let initial = runtime
            .websocket_control(transport, &control)
            .await
            .expect("subscribe control");
        let initial: Value =
            serde_json::from_slice(&initial.messages[1]).expect("initial engine envelope");
        let ongoing = runtime.websocket_batch(transport).expect("ongoing batch");
        let ongoing: Value = serde_json::from_slice(&ongoing[0]).expect("ongoing engine envelope");
        let sequence = |value: &Value| {
            value["position"]["sequence"]
                .as_str()
                .expect("sequence")
                .parse::<u64>()
                .expect("decimal sequence")
        };
        assert!(sequence(&ongoing) > sequence(&initial));
        runtime.retire().await.expect("runtime retires");
        assert_eq!(resources.current(), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stalled_membership_phase_does_not_block_a_sibling_and_cancellation_is_retryable() {
        let resources = Arc::new(ResourceCounter::default());
        let runtime = Arc::new(
            AsyncRuntime::new(ReferenceFaultSchedule::None, Arc::clone(&resources))
                .await
                .expect("async runtime"),
        );
        let origin = "http://127.0.0.1:4197";
        let created = runtime
            .create(
                TransportCreateRequest {
                    kind: "sse".to_owned(),
                    position: None,
                    prior_subscription: None,
                    subscription: "orders".to_owned(),
                },
                origin,
            )
            .await
            .expect("transport");
        let transport = created["transport"].as_str().expect("transport").to_owned();
        let memberships = created["memberships"].as_array().expect("memberships");
        let first = memberships[0]["subscription"]
            .as_str()
            .expect("first subscription")
            .to_owned();
        let first_authority = memberships[0]["authority"]
            .as_str()
            .expect("first authority")
            .to_owned();
        let second = memberships[1]["subscription"]
            .as_str()
            .expect("second subscription")
            .to_owned();
        let second_authority = memberships[1]["authority"]
            .as_str()
            .expect("second authority")
            .to_owned();

        runtime.phase_pause.select(&first);
        let stalled_runtime = Arc::clone(&runtime);
        let stalled_transport = transport.clone();
        let stalled_first = first.clone();
        let stalled_authority = first_authority.clone();
        let stalled = tokio::spawn(async move {
            stalled_runtime
                .membership(
                    &stalled_transport,
                    &stalled_first,
                    MembershipRequest {
                        authority: stalled_authority,
                        control_nonce: "paused-subscribe".to_owned(),
                        operation: "subscribe".to_owned(),
                    },
                )
                .await
        });
        timeout(
            Duration::from_secs(1),
            runtime.phase_pause.wait_until_entered(),
        )
        .await
        .expect("first membership stalled");
        timeout(
            Duration::from_millis(250),
            runtime.membership(
                &transport,
                &second,
                MembershipRequest {
                    authority: second_authority,
                    control_nonce: "sibling-subscribe".to_owned(),
                    operation: "subscribe".to_owned(),
                },
            ),
        )
        .await
        .expect("sibling is not blocked")
        .expect("sibling subscribes");

        stalled.abort();
        assert!(
            stalled
                .await
                .expect_err("stalled phase aborted")
                .is_cancelled()
        );
        runtime.phase_pause.resume();
        runtime
            .membership(
                &transport,
                &first,
                MembershipRequest {
                    authority: first_authority,
                    control_nonce: "paused-subscribe".to_owned(),
                    operation: "subscribe".to_owned(),
                },
            )
            .await
            .expect("canceled external authority remains retryable");
        assert_eq!(resources.current(), 2);
        runtime.retire().await.expect("runtime retires");
        assert_eq!(resources.current(), 0);
    }
}
