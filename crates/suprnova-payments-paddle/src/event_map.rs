//! Maps Paddle event_type strings to `NeutralEventKind`.

use suprnova::payments::NeutralEventKind;

/// Map a Paddle event_type string (e.g. `"transaction.completed"`) to the
/// framework's provider-agnostic `NeutralEventKind`, or `None` if no mapping
/// exists. Callers should fall through to `provider_event_type` + raw payload
/// for unmapped events. Adjustment events require their payload's action and
/// approval status, so only `WebhookHandler::parse_event` classifies them.
/// An issued invoice (`transaction.billed`) does not confirm collection.
pub fn paddle_event_to_neutral(t: &str) -> Option<NeutralEventKind> {
    Some(match t {
        "transaction.completed" | "transaction.paid" => NeutralEventKind::PaymentSucceeded,
        "transaction.payment_failed" => NeutralEventKind::PaymentFailed,
        "subscription.created" => NeutralEventKind::SubscriptionCreated,
        "subscription.updated"
        | "subscription.activated"
        | "subscription.paused"
        | "subscription.resumed"
        | "subscription.trialing" => NeutralEventKind::SubscriptionUpdated,
        "subscription.canceled" => NeutralEventKind::SubscriptionCanceled,
        "customer.created" => NeutralEventKind::CustomerCreated,
        "customer.updated" => NeutralEventKind::CustomerUpdated,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_events_map_correctly() {
        assert_eq!(
            paddle_event_to_neutral("transaction.completed"),
            Some(NeutralEventKind::PaymentSucceeded)
        );
        assert_eq!(
            paddle_event_to_neutral("subscription.created"),
            Some(NeutralEventKind::SubscriptionCreated)
        );
        assert_eq!(
            paddle_event_to_neutral("subscription.canceled"),
            Some(NeutralEventKind::SubscriptionCanceled)
        );
        assert_eq!(paddle_event_to_neutral("adjustment.created"), None);
    }

    #[test]
    fn unknown_event_returns_none() {
        assert_eq!(paddle_event_to_neutral("address.created"), None);
        assert_eq!(paddle_event_to_neutral(""), None);
    }
}
