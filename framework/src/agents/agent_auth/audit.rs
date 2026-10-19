//! Where agent auth's audit events go (AGENT-011): a structured log record,
//! the framework's lifecycle event and, for a claim, [`AgentClaimed`].

use async_trait::async_trait;
use magnetar::agent_auth::{DELIVERY_ATTEMPT, TrustedIssuer};
use magnetar::plugin::{DurableLifecycleDelivery, LifecycleEvent, LifecycleEventKind};
use serde_json::{Map, Value};

use super::AgentClaimed;
use crate::magnetar_integration::engine::MagnetarLifecycleEvent;

/// Records every audit event agent auth emits, after the mutation it
/// describes committed:
///
/// 1. one log record, to the channel the application named or the default
///    one, whose message is fixed (`agent auth: registration.created`) and
///    whose fields are the event's mutation id, user and every attribute,
///    the client address among them. Text an agent chose, such as
///    `agent_platform`, is a field and never part of the message, so it
///    cannot forge a log line;
/// 2. a [`MagnetarLifecycleEvent`] carrying the attributes;
/// 3. [`AgentClaimed`] for a confirmed claim.
///
/// A listener's error never turns the committed mutation into a failure.
/// The claim's event comes from agent auth's outbox, though: when an
/// [`AgentClaimed`] listener fails, `enqueue` answers `Err`, the event stays
/// in the outbox, and the next delivery pass hands it here again with the
/// same mutation id and a `delivery_attempt`. A retry writes one warning
/// naming the event and the attempt, and dispatches [`AgentClaimed`] only:
/// the record and the lifecycle event were written by the first attempt.
/// Every other event is delivered once, best effort.
pub(crate) struct AuditSink {
    channel: Option<String>,
    trusted: Vec<TrustedIssuer>,
}

impl AuditSink {
    pub(crate) fn new(channel: Option<String>, trusted: Vec<TrustedIssuer>) -> Self {
        Self { channel, trusted }
    }

    /// The record's message and fields.
    fn record(event: &LifecycleEvent) -> (String, Value) {
        let name = event.kind.audit_name().unwrap_or("lifecycle");
        let mut fields: Map<String, Value> = event
            .attributes
            .iter()
            .map(|(name, value)| (name.clone(), Value::String(value.clone())))
            .collect();
        fields.insert(
            "mutation_id".to_owned(),
            Value::String(event.mutation_id.clone()),
        );
        fields.insert("user_id".to_owned(), Value::String(event.user_id.clone()));
        fields.insert("event".to_owned(), Value::String(name.to_owned()));
        // `name` is one of auth.md's fixed event names, never input.
        (format!("agent auth: {name}"), Value::Object(fields))
    }

    /// Write a record at info level, or at warning level with `warning`.
    fn write(&self, message: &str, fields: Value, warning: bool) {
        if let Some(channel) = &self.channel {
            match crate::logging::Log::channel(channel) {
                Ok(logger) => {
                    if warning {
                        logger.warning_with(message, fields);
                    } else {
                        logger.info_with(message, fields);
                    }
                    return;
                }
                Err(error) => {
                    // The channel was checked when the router was built; a
                    // record is never dropped for a channel removed since.
                    tracing::error!(%error, channel, "agent auth's audit channel is gone");
                }
            }
        }
        if warning {
            crate::logging::Log::warning_with(message, fields);
        } else {
            crate::logging::Log::info_with(message, fields);
        }
    }

    /// The claim event for a confirmed claim.
    fn claimed(&self, event: &LifecycleEvent) -> AgentClaimed {
        let attribute = |name: &str| event.attributes.get(name).cloned();
        AgentClaimed {
            // The outbox event's identifier: the same on every delivery.
            event_id: event.mutation_id.clone(),
            registration_id: attribute("registration_id").unwrap_or_default(),
            registration_type: attribute("registration_type").unwrap_or_default(),
            user_id: event.user_id.clone(),
            // The trust list's name for the provider, never one it sent.
            provider: event.attributes.get("iss").and_then(|issuer| {
                self.trusted
                    .iter()
                    .find(|entry| &entry.issuer == issuer)
                    .map(|entry| entry.display_name.clone())
            }),
            client_address: attribute("client_address"),
        }
    }
}

#[async_trait]
impl DurableLifecycleDelivery for AuditSink {
    async fn enqueue(&self, event: LifecycleEvent) -> magnetar::plugin::PluginResult<()> {
        match event.attributes.get(DELIVERY_ATTEMPT) {
            // A retry: the first attempt wrote the record and dispatched the
            // lifecycle event, so a retry writes one warning instead.
            Some(attempt) => {
                let name = event.kind.audit_name().unwrap_or("lifecycle");
                self.write(
                    &format!("agent auth: retrying the delivery of {name}"),
                    serde_json::json!({
                        "event_id": event.mutation_id,
                        "attempt": attempt,
                        "event": name,
                    }),
                    true,
                );
            }
            None => {
                let (message, fields) = Self::record(&event);
                self.write(&message, fields, false);
                if let Err(error) = crate::events::EventFacade::dispatch_best_effort(
                    MagnetarLifecycleEvent::from(&event),
                )
                .await
                {
                    tracing::error!(%error, "a listener of an agent auth lifecycle event failed");
                }
            }
        }
        // The dispatcher logs the listener's own error. The answer agent auth
        // keeps in its outbox names only the event, so no detail a listener
        // put in its error, such as a user's address, is stored there.
        // At once, never deferred: inside an `Event::defer` scope a deferred
        // dispatch would answer before any listener ran, and the outbox
        // would mark the event delivered whatever the listener did.
        if event.kind == LifecycleEventKind::AgentClaimConfirmed
            && crate::events::EventFacade::dispatch_best_effort_now(self.claimed(&event))
                .await
                .is_err()
        {
            return Err(magnetar::plugin::PluginError::Request {
                plugin: "agent-auth".to_owned(),
                message: "a listener of AgentClaimed failed".to_owned(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_011_audit_record_keeps_agent_text_in_fields() {
        let event = LifecycleEvent::new(
            "agent-auth:1",
            LifecycleEventKind::AgentRegistrationCreated,
            "",
        )
        .with_attribute("registration_id", "reg_1")
        .with_attribute(
            "agent_platform",
            "evil\nagent auth: claim.confirmed {user_id}",
        )
        .with_attribute("client_address", "203.0.113.9");
        let (message, fields) = AuditSink::record(&event);
        assert_eq!(message, "agent auth: registration.created");
        assert_eq!(
            fields["agent_platform"],
            "evil\nagent auth: claim.confirmed {user_id}"
        );
        assert_eq!(fields["client_address"], "203.0.113.9");
        assert_eq!(fields["mutation_id"], "agent-auth:1");
        assert_eq!(fields["event"], "registration.created");
    }

    #[test]
    fn agent_009_claim_event_names_the_trust_list_provider() {
        let sink = AuditSink::new(
            None,
            vec![TrustedIssuer::new(
                "https://idp.test",
                "https://idp.test/jwks",
                "Trusted Name",
            )],
        );
        let event = LifecycleEvent::new(
            "agent-auth:2",
            LifecycleEventKind::AgentClaimConfirmed,
            "user-7",
        )
        .with_attribute("registration_id", "reg_2")
        .with_attribute("registration_type", "identity_assertion")
        .with_attribute("iss", "https://idp.test")
        .with_attribute("client_address", "203.0.113.9");
        assert_eq!(
            sink.claimed(&event),
            AgentClaimed {
                event_id: "agent-auth:2".to_owned(),
                registration_id: "reg_2".to_owned(),
                registration_type: "identity_assertion".to_owned(),
                user_id: "user-7".to_owned(),
                provider: Some("Trusted Name".to_owned()),
                client_address: Some("203.0.113.9".to_owned()),
            }
        );
    }
}
