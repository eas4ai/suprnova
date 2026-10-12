//! Closed E100/1K server-owner proof used by the benchmark harness.

use std::collections::BTreeMap;
use std::num::NonZeroUsize;

use serde::Serialize;
use suprnova_live::async_updates::{
    AsyncDeliveryDisposition, AsyncDispatchError, AsyncEnvelopeDispatchPort, AsyncPolicy,
    BoundedDocumentTransportSession, BufferDisposition, DocumentTransportKind,
    MAX_REPLAY_TRANSCRIPT_ENVELOPES, ResolvedAsyncDelivery, SequenceDisposition, SubscriptionId,
};
use suprnova_live::resource::{PermitPool, ResourceBounds};

use super::engine_async::EngineAsyncFixture;

const SUBSCRIPTIONS: usize = 100;
const PRESENTATION_EVENTS_PER_SUBSCRIPTION: usize = 10;
const REFRESHES_PER_SUBSCRIPTION: usize = 1;
const PRESENTATION_EVENT_BYTES: usize = 1_024;
const QUEUE_BATCH: usize = 32;

/// Exact server-owner evidence produced through `BoundedDocumentTransportSession`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AsyncBudgetOwnerEvidence {
    /// Number of accepted and dispatched envelopes.
    pub dispatches: usize,
    /// Logical memberships at the authoritative current position.
    pub final_current_subscriptions: usize,
    /// Maximum fairness lead observed between logical lanes.
    pub fairness_maximum_lead: usize,
    /// Logical memberships sharing the physical document transport.
    pub logical_memberships: usize,
    /// Maximum queued bytes owned by the bounded document session.
    pub max_queued_bytes: usize,
    /// Maximum queued envelopes owned by the bounded document session.
    pub max_queued_events: usize,
    /// Number of physical document transports constructed by the proof.
    pub physical_document_transports: usize,
    /// Product owner exercised by the proof.
    pub provider_path: &'static str,
    /// Sequence or subscription mismatches observed after dispatch.
    pub sequence_mismatches: usize,
}

#[derive(Default)]
struct BudgetDispatcher {
    dispatches: usize,
    by_subscription: BTreeMap<String, usize>,
    sequence_mismatches: usize,
}

impl AsyncEnvelopeDispatchPort for BudgetDispatcher {
    fn dispatch(&mut self, delivery: ResolvedAsyncDelivery<'_>) -> Result<(), AsyncDispatchError> {
        self.dispatches = self.dispatches.saturating_add(1);
        let subscription = delivery.envelope().subscription().to_base64url();
        let expected = self
            .by_subscription
            .get(&subscription)
            .copied()
            .unwrap_or_default()
            .saturating_add(1);
        if delivery.envelope().position().sequence().get() != expected as u64 {
            self.sequence_mismatches = self.sequence_mismatches.saturating_add(1);
        }
        self.by_subscription.insert(subscription, expected);
        Ok(())
    }
}

/// Runs the deterministic server-side E100/1K ownership proof.
pub async fn measure_async_budget_owner() -> Result<AsyncBudgetOwnerEvidence, String> {
    let engine = EngineAsyncFixture::new().await?;
    let origin = "https://async-budget.example.test";
    let mut document = engine
        .document_with_limit(
            origin,
            DocumentTransportKind::ServerSentEvents,
            0xa4,
            SUBSCRIPTIONS,
        )
        .map_err(str::to_owned)?;
    let mut authorizations = Vec::with_capacity(SUBSCRIPTIONS);
    for index in 0..SUBSCRIPTIONS {
        let subscription =
            SubscriptionId::from_bytes(format!("subscription-{index:03}").as_bytes())
                .map_err(|_| "async budget subscription")?
                .to_base64url();
        let authorization = engine
            .authorization(&subscription, origin)
            .map_err(str::to_owned)?;
        let authorized = document
            .prepare_add(authorization.clone())
            .map_err(|error| error.kind().as_str().to_owned())?
            .authorize()
            .await
            .map_err(|error| error.kind().as_str().to_owned())?;
        let ready = document
            .prepare_establish(authorized)
            .map_err(|error| error.kind().as_str().to_owned())?
            .establish(engine.source())
            .await
            .map_err(|error| error.kind().as_str().to_owned())?;
        document
            .commit_add(ready)
            .map_err(|error| error.kind().as_str().to_owned())?;
        authorizations.push(authorization);
    }
    if document.membership_count() != SUBSCRIPTIONS {
        return Err("async budget membership count".to_owned());
    }
    let mut owner = BoundedDocumentTransportSession::new(
        document,
        ResourceBounds::new(64, 256 * 1_024).map_err(|_| "async budget bounds")?,
        PermitPool::new(1).map_err(|_| "async budget permits")?,
        AsyncPolicy {
            max_payload_bytes: NonZeroUsize::new(32 * 1_024).expect("non-zero async payload bound"),
            max_replay_events: NonZeroUsize::new(MAX_REPLAY_TRANSCRIPT_ENVELOPES)
                .expect("non-zero replay bound"),
            max_fanout: NonZeroUsize::new(1).expect("non-zero fanout bound"),
        },
    )
    .map_err(|_| "async budget pressure".to_owned())?;
    let mut dispatcher = BudgetDispatcher::default();
    let mut max_queued_events = 0;
    let mut max_queued_bytes = 0;
    let mut fairness_maximum_lead = 0;

    for sequence in 1..=PRESENTATION_EVENTS_PER_SUBSCRIPTION + REFRESHES_PER_SUBSCRIPTION {
        let refresh_round = sequence == 6;
        for authorization in &authorizations {
            let envelope = if refresh_round {
                engine
                    .refresh_envelope(authorization, sequence as u64)
                    .map_err(str::to_owned)?
            } else {
                engine
                    .padded_browser_event_envelope(
                        authorization,
                        sequence as u64,
                        PRESENTATION_EVENT_BYTES,
                    )
                    .map_err(str::to_owned)?
            };
            engine.queue(envelope).map_err(str::to_owned)?;
        }

        let mut remaining = SUBSCRIPTIONS;
        while remaining > 0 {
            let batch = remaining.min(QUEUE_BATCH);
            for _ in 0..batch {
                let disposition = owner
                    .pump_next(engine.registry())
                    .await
                    .map_err(|error| error.kind().as_str().to_owned())?;
                if disposition != Some(BufferDisposition::Queued) {
                    return Err("async budget queue disposition".to_owned());
                }
            }
            max_queued_events = max_queued_events.max(owner.retained_events());
            max_queued_bytes = max_queued_bytes.max(owner.retained_bytes());
            for _ in 0..batch {
                let disposition = owner
                    .dispatch_next(engine.registry(), &mut dispatcher)
                    .map_err(|_| "async budget dispatch".to_owned())?;
                if !matches!(
                    disposition,
                    Some(AsyncDeliveryDisposition::Sequence(
                        SequenceDisposition::Apply
                    ))
                ) {
                    return Err("async budget dispatch disposition".to_owned());
                }
                let minimum = dispatcher
                    .by_subscription
                    .values()
                    .copied()
                    .min()
                    .unwrap_or_default();
                let maximum = dispatcher
                    .by_subscription
                    .values()
                    .copied()
                    .max()
                    .unwrap_or_default();
                fairness_maximum_lead = fairness_maximum_lead.max(maximum - minimum);
            }
            remaining -= batch;
        }
    }

    let final_current_subscriptions = authorizations
        .iter()
        .filter(|authorization| {
            owner
                .sequence_position(authorization)
                .is_some_and(|position| {
                    position.epoch().get() == 1
                        && position.sequence().get()
                            == (PRESENTATION_EVENTS_PER_SUBSCRIPTION + REFRESHES_PER_SUBSCRIPTION)
                                as u64
                })
        })
        .count();
    let sequence_mismatches = dispatcher.sequence_mismatches
        + dispatcher
            .by_subscription
            .values()
            .filter(|count| {
                **count != PRESENTATION_EVENTS_PER_SUBSCRIPTION + REFRESHES_PER_SUBSCRIPTION
            })
            .count();

    Ok(AsyncBudgetOwnerEvidence {
        dispatches: dispatcher.dispatches,
        final_current_subscriptions,
        fairness_maximum_lead,
        logical_memberships: owner.transport().membership_count(),
        max_queued_bytes,
        max_queued_events,
        physical_document_transports: 1,
        provider_path: "BoundedDocumentTransportSession",
        sequence_mismatches,
    })
}
