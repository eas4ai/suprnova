//! Implementation of the `WebhookHandler` trait for `PaddleProvider`.
//!
//! Uses `Paddle::unmarshal` for signature verification - it handles the
//! `Paddle-Signature` header format (`ts=…,h1=…`) and HMAC validation with
//! timestamp-skew tolerance. No manual HMAC code needed.

use async_trait::async_trait;
use paddle_rust_sdk::{Paddle, webhooks::MaximumVariance};
use suprnova::payments::{
    CustomerSnapshot, NeutralEventKind, PayloadIds, PaymentError, PaymentResult, PaymentSnapshot,
    WebhookContext, WebhookEvent, WebhookHandler,
};

use crate::{PaddleProvider, event_map::paddle_event_to_neutral};

fn paddle_snapshot_error(event: &WebhookEvent, field: &str, expectation: &str) -> PaymentError {
    PaymentError::Provider(format!(
        "malformed paddle {} snapshot: {field} {expectation}",
        event.provider_event_type
    ))
}

fn required_paddle_string(
    event: &WebhookEvent,
    data: &serde_json::Value,
    field: &str,
) -> PaymentResult<String> {
    data.get(field)
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| paddle_snapshot_error(event, field, "must be a non-empty string"))
}

fn optional_paddle_string(
    event: &WebhookEvent,
    data: &serde_json::Value,
    field: &str,
) -> PaymentResult<Option<String>> {
    match data.get(field) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(value)) if !value.trim().is_empty() => {
            Ok(Some(value.clone()))
        }
        Some(_) => Err(paddle_snapshot_error(
            event,
            field,
            "must be a non-empty string when present",
        )),
    }
}

/// Parse a Paddle minor-unit amount field, accepting either the decimal-string
/// form Paddle normally sends (`"1234"`) or a bare JSON number (`1234`).
fn parse_minor(
    event: &WebhookEvent,
    value: Option<&serde_json::Value>,
    field: &str,
    required: bool,
) -> PaymentResult<i64> {
    match value {
        None | Some(serde_json::Value::Null) if !required => Ok(0),
        None | Some(serde_json::Value::Null) => {
            Err(paddle_snapshot_error(event, field, "must be present"))
        }
        Some(serde_json::Value::String(value)) => value
            .parse::<i64>()
            .map_err(|_| paddle_snapshot_error(event, field, "must contain integer minor units")),
        Some(value) => value
            .as_i64()
            .ok_or_else(|| paddle_snapshot_error(event, field, "must contain integer minor units")),
    }
}

fn required_paddle_currency(
    event: &WebhookEvent,
    data: &serde_json::Value,
) -> PaymentResult<String> {
    let currency = required_paddle_string(event, data, "currency_code")?;
    if currency.len() == 3 && currency.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        Ok(currency.to_uppercase())
    } else {
        Err(paddle_snapshot_error(
            event,
            "currency_code",
            "must be a three-letter code",
        ))
    }
}

fn optional_paddle_timestamp(
    event: &WebhookEvent,
    value: Option<&serde_json::Value>,
    field: &str,
) -> PaymentResult<Option<chrono::DateTime<chrono::Utc>>> {
    match value {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(value)) => chrono::DateTime::parse_from_rfc3339(value)
            .map(|timestamp| Some(timestamp.with_timezone(&chrono::Utc)))
            .map_err(|_| {
                paddle_snapshot_error(event, field, "must be an RFC 3339 timestamp when present")
            }),
        Some(_) => Err(paddle_snapshot_error(
            event,
            field,
            "must be an RFC 3339 timestamp when present",
        )),
    }
}

fn checked_paddle_payment_snapshot(event: &WebhookEvent) -> PaymentResult<Option<PaymentSnapshot>> {
    let Some(kind) = event.neutral else {
        return Ok(None);
    };
    if !matches!(
        kind,
        NeutralEventKind::PaymentSucceeded
            | NeutralEventKind::PaymentFailed
            | NeutralEventKind::PaymentRefunded
            | NeutralEventKind::PaymentDisputed
            | NeutralEventKind::InvoicePaid
            | NeutralEventKind::InvoiceFailed
    ) {
        return Ok(None);
    }

    let data = event
        .raw_payload
        .get("data")
        .ok_or_else(|| paddle_snapshot_error(event, "data", "must be present"))?;
    let provider_customer_id = optional_paddle_string(event, data, "customer_id")?;
    let provider_subscription_id = optional_paddle_string(event, data, "subscription_id")?;
    let currency = required_paddle_currency(event, data)?;

    let snapshot = match kind {
        NeutralEventKind::PaymentRefunded | NeutralEventKind::PaymentDisputed => {
            let provider_transaction_id = required_paddle_string(event, data, "transaction_id")?;
            let amount_total_minor =
                parse_minor(event, data.pointer("/totals/total"), "totals.total", true)?;
            let amount_tax_minor =
                parse_minor(event, data.pointer("/totals/tax"), "totals.tax", false)?;
            let provider_customer_id = provider_customer_id.ok_or_else(|| {
                paddle_snapshot_error(event, "customer_id", "must be a non-empty string")
            })?;
            PaymentSnapshot {
                provider_transaction_id,
                provider_customer_id,
                provider_subscription_id,
                amount_total_minor,
                amount_tax_minor,
                currency,
                status: if kind == NeutralEventKind::PaymentRefunded {
                    "refunded".to_owned()
                } else {
                    "disputed".to_owned()
                },
                paid_at: None,
                provider_metadata: data.clone(),
            }
        }
        NeutralEventKind::PaymentSucceeded
        | NeutralEventKind::PaymentFailed
        | NeutralEventKind::InvoicePaid
        | NeutralEventKind::InvoiceFailed => {
            let provider_transaction_id = required_paddle_string(event, data, "id")?;
            let amount_total_minor = parse_minor(
                event,
                data.pointer("/details/totals/total"),
                "details.totals.total",
                true,
            )?;
            let amount_tax_minor = parse_minor(
                event,
                data.pointer("/details/totals/tax"),
                "details.totals.tax",
                false,
            )?;
            let paid_at = optional_paddle_timestamp(event, data.get("billed_at"), "billed_at")?;
            let Some(provider_customer_id) = provider_customer_id else {
                return Ok(None);
            };
            PaymentSnapshot {
                provider_transaction_id,
                provider_customer_id,
                provider_subscription_id,
                amount_total_minor,
                amount_tax_minor,
                currency,
                status: if matches!(
                    kind,
                    NeutralEventKind::PaymentSucceeded | NeutralEventKind::InvoicePaid
                ) {
                    "succeeded".to_owned()
                } else {
                    "failed".to_owned()
                },
                paid_at,
                provider_metadata: data.clone(),
            }
        }
        _ => unreachable!(),
    };

    Ok(Some(snapshot))
}

/// Reject a `paddle-signature` header whose digest is not well-formed hex
/// before the SDK sees it.
///
/// The header is `ts=<unix>;h1=<hex>`, possibly with more `hN=` digests as
/// Paddle rotates schemes. Any `h`-prefixed value must decode as bytes:
/// even length, hex alphabet only. An odd-length value panics inside the
/// pinned SDK, so this check is what keeps a malformed header a 401
/// instead of an unwind.
///
/// Deliberately permissive about everything else - key order, unknown
/// keys, the timestamp - because the SDK owns that parsing and already
/// errors cleanly on it. This guards exactly the input that does not
/// error cleanly.
fn validate_signature_digests(signature: &str) -> PaymentResult<()> {
    for segment in signature.split(';') {
        let Some((key, value)) = segment.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if !key.starts_with('h') {
            continue;
        }
        if !value.len().is_multiple_of(2) {
            return Err(PaymentError::WebhookSignature(format!(
                "paddle signature digest `{key}` has an odd number of hex \
                 characters ({}); refusing to decode",
                value.len()
            )));
        }
        if let Some(bad) = value.chars().find(|c| !c.is_ascii_hexdigit()) {
            return Err(PaymentError::WebhookSignature(format!(
                "paddle signature digest `{key}` contains a non-hex character `{bad}`"
            )));
        }
    }
    Ok(())
}

#[async_trait]
impl WebhookHandler for PaddleProvider {
    fn verify(&self, ctx: &WebhookContext<'_>) -> PaymentResult<()> {
        // A blank notification secret is an empty-key HMAC, which any caller
        // can forge. `from_env` refuses to construct one, but `new` takes the
        // key verbatim, so guard the boundary that actually decides trust -
        // that covers every construction path, not just the documented one.
        if self.webhook_key().trim().is_empty() {
            return Err(PaymentError::WebhookSignature(
                "paddle webhook key is empty - refusing to verify against an empty-key HMAC".into(),
            ));
        }

        let signature = ctx
            .headers
            .get("paddle-signature")
            .ok_or_else(|| {
                PaymentError::WebhookSignature("missing paddle-signature header".into())
            })?
            .to_str()
            .map_err(|_| PaymentError::WebhookSignature("non-ascii signature header".into()))?;

        // The pinned SDK panics on an odd-length hex digest rather than
        // returning an error - verified by probe, not assumed:
        //
        //     paddle-signature: ts=1671552777;h1=abc   →  panic
        //     paddle-signature: ts=1671552777;h1=zzzz  →  Err (fine)
        //
        // The header is attacker-controlled and this endpoint is
        // unauthenticated by definition - verifying the signature is what
        // authenticates it - so anyone who knows the URL can reach the
        // panic. Check the digest before handing it over.
        validate_signature_digests(signature)?;

        let body_str = std::str::from_utf8(ctx.body)
            .map_err(|_| PaymentError::WebhookSignature("non-utf8 webhook body".into()))?;

        Paddle::unmarshal(
            body_str,
            self.webhook_key(),
            signature,
            MaximumVariance::default(),
        )
        .map_err(|e| PaymentError::WebhookSignature(format!("paddle signature verify: {e}")))?;

        Ok(())
    }

    fn parse_event(&self, body: &[u8]) -> PaymentResult<WebhookEvent> {
        let raw: serde_json::Value = serde_json::from_slice(body)
            .map_err(|e| PaymentError::Validation(format!("invalid paddle webhook body: {e}")))?;

        let provider_event_id = raw
            .get("event_id")
            .and_then(|value| value.as_str())
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                PaymentError::Validation(
                    "paddle webhook event_id must be a non-empty string".to_owned(),
                )
            })?
            .to_owned();
        let provider_event_type = raw
            .get("event_type")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let neutral: Option<NeutralEventKind> = paddle_event_to_neutral(&provider_event_type);

        Ok(WebhookEvent {
            provider: "paddle".into(),
            provider_event_id,
            provider_event_type,
            neutral,
            raw_payload: raw,
        })
    }

    /// Extract IDs from Paddle's `data.*` envelope.
    ///
    /// Paddle puts the entity directly under `data`, with `id` as its primary
    /// key and `customer_id` as the customer pointer. Transaction events also
    /// carry `subscription_id` when they belong to a subscription billing
    /// cycle.
    ///
    /// Adjustment events (`adjustment.created` / `adjustment.updated`, mapped
    /// to [`NeutralEventKind::PaymentRefunded`]) are NOT transactions: their
    /// `id` is the adjustment id (`adj_…`) and the transaction they adjust is
    /// in a separate `transaction_id` field (`txn_…`). The mirror must be
    /// keyed off `transaction_id` so a refund updates the original transaction
    /// row rather than inserting a phantom row keyed by the adjustment id.
    fn extract_payload_ids(&self, event: &WebhookEvent) -> PayloadIds {
        let data = match event.raw_payload.get("data") {
            Some(d) => d,
            None => return PayloadIds::default(),
        };

        let mut ids = PayloadIds::default();

        match event.neutral {
            Some(
                NeutralEventKind::SubscriptionCreated
                | NeutralEventKind::SubscriptionUpdated
                | NeutralEventKind::SubscriptionCanceled,
            ) => {
                ids.subscription_id = data.get("id").and_then(|v| v.as_str()).map(String::from);
                ids.customer_id = data
                    .get("customer_id")
                    .and_then(|v| v.as_str())
                    .map(String::from);
            }
            Some(NeutralEventKind::CustomerCreated | NeutralEventKind::CustomerUpdated) => {
                ids.customer_id = data.get("id").and_then(|v| v.as_str()).map(String::from);
            }
            // Adjustment payload: key off the referenced transaction, not the
            // adjustment's own `id`.
            Some(NeutralEventKind::PaymentRefunded | NeutralEventKind::PaymentDisputed) => {
                ids.transaction_id = data
                    .get("transaction_id")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                ids.customer_id = data
                    .get("customer_id")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                ids.subscription_id = data
                    .get("subscription_id")
                    .and_then(|v| v.as_str())
                    .map(String::from);
            }
            // Transaction payload: `id` is the transaction id.
            Some(
                NeutralEventKind::PaymentSucceeded
                | NeutralEventKind::PaymentFailed
                | NeutralEventKind::InvoicePaid
                | NeutralEventKind::InvoiceFailed,
            ) => {
                ids.transaction_id = data.get("id").and_then(|v| v.as_str()).map(String::from);
                ids.customer_id = data
                    .get("customer_id")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                ids.subscription_id = data
                    .get("subscription_id")
                    .and_then(|v| v.as_str())
                    .map(String::from);
            }
            None => {}
        }
        ids
    }

    /// Build a [`PaymentSnapshot`] from a Paddle payload.
    ///
    /// Paddle sends two structurally different shapes that both land on the
    /// `payments_transactions` mirror:
    ///
    /// - **Transaction** (`transaction.*` → succeeded / failed / invoice
    ///   paid). `data.id` is the transaction id (`txn_…`); totals live under
    ///   `data.details.totals.{total,tax}` as decimal-string minor units;
    ///   currency is `data.currency_code`; settle time is `data.billed_at`.
    /// - **Adjustment** (`adjustment.*` → refunded / chargeback). `data.id`
    ///   is the adjustment id (`adj_…`) - NOT a transaction - and the
    ///   transaction it adjusts is `data.transaction_id`. Totals live at
    ///   `data.totals.{total,tax}` (there is no `data.details`), currency is
    ///   `data.currency_code` at the top level. The mirror is keyed off
    ///   `transaction_id` so the refund/chargeback updates the original
    ///   transaction row instead of inserting a phantom `adj_…` row with a
    ///   zero amount.
    fn extract_payment_snapshot(&self, event: &WebhookEvent) -> Option<PaymentSnapshot> {
        checked_paddle_payment_snapshot(event).ok().flatten()
    }

    fn try_extract_payment_snapshot(
        &self,
        event: &WebhookEvent,
    ) -> PaymentResult<Option<PaymentSnapshot>> {
        checked_paddle_payment_snapshot(event)
    }

    /// Build a [`CustomerSnapshot`] from Paddle `customer.created` /
    /// `customer.updated` payloads. Paddle puts the Customer object directly
    /// under `data` (no `data.object` wrapper).
    fn extract_customer_snapshot(&self, event: &WebhookEvent) -> Option<CustomerSnapshot> {
        match event.neutral? {
            NeutralEventKind::CustomerCreated | NeutralEventKind::CustomerUpdated => {
                let data = event.raw_payload.get("data")?;
                let provider_customer_id = data.get("id")?.as_str()?.to_string();
                let email = data
                    .get("email")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                Some(CustomerSnapshot {
                    provider_customer_id,
                    email,
                    provider_metadata: data.clone(),
                })
            }
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PaddleEnvironment, PaddleProvider};

    fn provider() -> PaddleProvider {
        // Dummy keys are fine - extractor tests don't hit the Paddle HTTP API.
        PaddleProvider::new(
            "pdl_test_apikey",
            "pdl_test_whkey",
            "test_clienttoken",
            PaddleEnvironment::Sandbox,
        )
        .expect("paddle provider construction")
    }

    #[test]
    fn parse_event_rejects_invalid_ids_and_preserves_nonblank_ids() {
        let provider = provider();
        for payload in [
            serde_json::json!({ "event_type": "transaction.completed" }),
            serde_json::json!({ "event_id": null, "event_type": "transaction.completed" }),
            serde_json::json!({ "event_id": 42, "event_type": "transaction.completed" }),
            serde_json::json!({ "event_id": "", "event_type": "transaction.completed" }),
            serde_json::json!({ "event_id": " \t\r\n", "event_type": "transaction.completed" }),
        ] {
            let body = serde_json::to_vec(&payload).expect("serialize payload");
            let error = provider
                .parse_event(&body)
                .expect_err("an invalid Paddle event id must be rejected");
            assert!(matches!(
                error,
                PaymentError::Validation(ref message) if message.contains("event_id")
            ));
        }

        let event = provider
            .parse_event(br#"{"event_id":" evt_preserved ","event_type":"transaction.completed"}"#)
            .expect("a nonblank Paddle event id is valid");
        assert_eq!(event.provider_event_id, " evt_preserved ");
    }

    fn event(neutral: NeutralEventKind, payload: serde_json::Value) -> WebhookEvent {
        WebhookEvent {
            provider: "paddle".into(),
            provider_event_id: "evt_test".into(),
            provider_event_type: format!("{neutral:?}"),
            neutral: Some(neutral),
            raw_payload: payload,
        }
    }

    #[test]
    fn verify_refuses_a_blank_webhook_key() {
        // `new` accepts the key verbatim, so a config path that reads a
        // set-but-empty `PADDLE_WEBHOOK_KEY` can still build a provider whose
        // HMAC key is empty. Verification must refuse it rather than compare
        // against a signature anyone could compute.
        let p = PaddleProvider::new(
            "pdl_test_apikey",
            "",
            "test_clienttoken",
            PaddleEnvironment::Sandbox,
        )
        .expect("paddle provider construction");
        let headers = http::HeaderMap::new();
        let ctx = WebhookContext {
            body: b"{}",
            headers: &headers,
            remote_addr: None,
        };

        let err = p
            .verify(&ctx)
            .expect_err("blank webhook key must not verify");
        assert!(
            matches!(err, PaymentError::WebhookSignature(ref m) if m.contains("empty")),
            "expected an empty-key signature refusal, got: {err:?}"
        );
    }

    #[test]
    fn verify_refuses_a_whitespace_only_webhook_key() {
        let p = PaddleProvider::new(
            "pdl_test_apikey",
            "   ",
            "test_clienttoken",
            PaddleEnvironment::Sandbox,
        )
        .expect("paddle provider construction");
        let headers = http::HeaderMap::new();
        let ctx = WebhookContext {
            body: b"{}",
            headers: &headers,
            remote_addr: None,
        };

        // Assert the *reason*, not just that it errored: a bare `is_err()`
        // would also pass on the "missing paddle-signature header" path this
        // fixture would hit anyway, so it would stay green with the guard
        // removed.
        let err = p
            .verify(&ctx)
            .expect_err("a whitespace-only key is as forgeable as an empty one");
        assert!(
            matches!(err, PaymentError::WebhookSignature(ref m) if m.contains("empty")),
            "expected an empty-key signature refusal, got: {err:?}"
        );
    }

    #[test]
    fn require_nonempty_rejects_blank_credentials() {
        use crate::require_nonempty;
        assert!(require_nonempty("PADDLE_WEBHOOK_KEY", String::new()).is_err());
        assert!(require_nonempty("PADDLE_WEBHOOK_KEY", "   ".into()).is_err());
        assert!(require_nonempty("PADDLE_WEBHOOK_KEY", "pdl_ntfset_ok".into()).is_ok());
    }

    #[test]
    fn extract_payload_ids_subscription_event() {
        let p = provider();
        let e = event(
            NeutralEventKind::SubscriptionCreated,
            serde_json::json!({ "data": { "id": "sub_pdl", "customer_id": "ctm_xyz" } }),
        );
        let ids = p.extract_payload_ids(&e);
        assert_eq!(ids.subscription_id.as_deref(), Some("sub_pdl"));
        assert_eq!(ids.customer_id.as_deref(), Some("ctm_xyz"));
        assert!(ids.transaction_id.is_none());
    }

    #[test]
    fn extract_payload_ids_transaction_event_includes_subscription_link() {
        let p = provider();
        let e = event(
            NeutralEventKind::PaymentSucceeded,
            serde_json::json!({ "data": {
                "id": "txn_done",
                "customer_id": "ctm_pay",
                "subscription_id": "sub_pdl"
            } }),
        );
        let ids = p.extract_payload_ids(&e);
        assert_eq!(ids.transaction_id.as_deref(), Some("txn_done"));
        assert_eq!(ids.customer_id.as_deref(), Some("ctm_pay"));
        assert_eq!(ids.subscription_id.as_deref(), Some("sub_pdl"));
    }

    #[test]
    fn extract_payment_snapshot_parses_string_totals() {
        let p = provider();
        let e = event(
            NeutralEventKind::PaymentSucceeded,
            serde_json::json!({ "data": {
                "id": "txn_str",
                "customer_id": "ctm_x",
                "currency_code": "eur",
                "details": { "totals": { "total": "1234", "tax": "100" } },
                "billed_at": "2026-05-22T12:00:00Z"
            } }),
        );
        let snap = p.extract_payment_snapshot(&e).expect("snapshot present");
        assert_eq!(snap.amount_total_minor, 1234, "string totals must parse");
        assert_eq!(snap.amount_tax_minor, 100);
        assert_eq!(snap.currency, "EUR");
        assert_eq!(snap.status, "succeeded");
        assert!(snap.paid_at.is_some(), "billed_at must parse to paid_at");
    }

    #[test]
    fn extract_payment_snapshot_handles_numeric_totals() {
        let p = provider();
        let e = event(
            NeutralEventKind::InvoicePaid,
            serde_json::json!({ "data": {
                "id": "txn_num",
                "customer_id": "ctm_n",
                "currency_code": "USD",
                "details": { "totals": { "total": 500, "tax": 50 } }
            } }),
        );
        let snap = p.extract_payment_snapshot(&e).expect("snapshot present");
        assert_eq!(snap.amount_total_minor, 500);
        assert_eq!(snap.amount_tax_minor, 50);
    }

    #[test]
    fn checked_payment_snapshot_rejects_malformed_fields() {
        let p = provider();
        let cases = [
            (
                "id",
                serde_json::json!({ "data": {
                    "id": 42,
                    "customer_id": "ctm_test",
                    "currency_code": "USD",
                    "details": { "totals": { "total": "4200" } }
                } }),
            ),
            (
                "customer_id",
                serde_json::json!({ "data": {
                    "id": "txn_test",
                    "customer_id": 42,
                    "currency_code": "USD",
                    "details": { "totals": { "total": "4200" } }
                } }),
            ),
            (
                "total",
                serde_json::json!({ "data": {
                    "id": "txn_test",
                    "customer_id": "ctm_test",
                    "currency_code": "USD",
                    "details": { "totals": { "total": "not-a-number" } }
                } }),
            ),
            (
                "tax",
                serde_json::json!({ "data": {
                    "id": "txn_test",
                    "customer_id": "ctm_test",
                    "currency_code": "USD",
                    "details": { "totals": { "total": "4200", "tax": false } }
                } }),
            ),
            (
                "currency_code",
                serde_json::json!({ "data": {
                    "id": "txn_test",
                    "customer_id": "ctm_test",
                    "currency_code": 840,
                    "details": { "totals": { "total": "4200" } }
                } }),
            ),
            (
                "subscription_id",
                serde_json::json!({ "data": {
                    "id": "txn_test",
                    "customer_id": "ctm_test",
                    "subscription_id": 42,
                    "currency_code": "USD",
                    "details": { "totals": { "total": "4200" } }
                } }),
            ),
            (
                "billed_at",
                serde_json::json!({ "data": {
                    "id": "txn_test",
                    "customer_id": "ctm_test",
                    "currency_code": "USD",
                    "billed_at": "yesterday",
                    "details": { "totals": { "total": "4200" } }
                } }),
            ),
        ];

        for (field, payload) in cases {
            let event = event(NeutralEventKind::PaymentSucceeded, payload);
            let error = p
                .try_extract_payment_snapshot(&event)
                .expect_err("malformed mapped payment must fail extraction");
            assert!(
                matches!(error, PaymentError::Provider(ref message) if message.contains(field)),
                "error for {field} did not identify the field: {error:?}"
            );
        }
    }

    #[test]
    fn nullable_customer_is_an_intentional_partial_snapshot() {
        let p = provider();
        let nullable = event(
            NeutralEventKind::PaymentSucceeded,
            serde_json::json!({ "data": {
                "id": "txn_guest",
                "customer_id": null,
                "currency_code": "USD",
                "details": { "totals": { "total": "4200" } }
            } }),
        );
        assert!(
            p.try_extract_payment_snapshot(&nullable)
                .expect("a nullable provider relationship is valid")
                .is_none()
        );

        let malformed = event(
            NeutralEventKind::PaymentSucceeded,
            serde_json::json!({ "data": {
                "id": "txn_guest_malformed",
                "customer_id": null,
                "currency_code": "USD",
                "details": { "totals": { "total": false } }
            } }),
        );
        let error = p
            .try_extract_payment_snapshot(&malformed)
            .expect_err("nullable customer must not hide a malformed amount");
        assert!(matches!(
            error,
            PaymentError::Provider(ref message) if message.contains("total")
        ));
    }

    #[test]
    fn checked_adjustment_snapshot_requires_transaction_and_total() {
        let p = provider();
        for (field, payload) in [
            (
                "customer_id",
                serde_json::json!({ "data": {
                    "id": "adj_test",
                    "transaction_id": "txn_test",
                    "currency_code": "USD",
                    "totals": { "total": "4200" }
                } }),
            ),
            (
                "customer_id",
                serde_json::json!({ "data": {
                    "id": "adj_test",
                    "transaction_id": "txn_test",
                    "customer_id": null,
                    "currency_code": "USD",
                    "totals": { "total": "4200" }
                } }),
            ),
            (
                "transaction_id",
                serde_json::json!({ "data": {
                    "id": "adj_test",
                    "customer_id": "ctm_test",
                    "currency_code": "USD",
                    "totals": { "total": "4200" }
                } }),
            ),
            (
                "total",
                serde_json::json!({ "data": {
                    "id": "adj_test",
                    "transaction_id": "txn_test",
                    "customer_id": "ctm_test",
                    "currency_code": "USD",
                    "totals": {}
                } }),
            ),
        ] {
            let event = event(NeutralEventKind::PaymentRefunded, payload);
            let error = p
                .try_extract_payment_snapshot(&event)
                .expect_err("malformed adjustment must fail extraction");
            assert!(
                matches!(error, PaymentError::Provider(ref message) if message.contains(field)),
                "error for {field} did not identify the field: {error:?}"
            );
        }
    }

    /// A realistic `adjustment.created` (refund) body: `data.id` is the
    /// adjustment id, the original transaction is in `data.transaction_id`,
    /// amounts live at `data.totals.*`, and currency is the top-level
    /// `data.currency_code`. The snapshot must key off `transaction_id` and
    /// carry the real amount - not insert a phantom `adj_…` row with amount 0.
    #[test]
    fn extract_payment_snapshot_adjustment_keys_off_transaction_id_with_real_amount() {
        let p = provider();
        let e = event(
            NeutralEventKind::PaymentRefunded,
            serde_json::json!({ "data": {
                "id": "adj_01h8xce4qhqc",
                "action": "refund",
                "transaction_id": "txn_01h8xc...original",
                "subscription_id": "sub_01h8x...",
                "customer_id": "ctm_01h8x...",
                "currency_code": "gbp",
                "reason": "Customer requested a refund",
                "status": "approved",
                "totals": {
                    "subtotal": "1000",
                    "tax": "200",
                    "total": "1200",
                    "fee": "60",
                    "earnings": "940",
                    "currency_code": "GBP"
                }
            } }),
        );
        let snap = p.extract_payment_snapshot(&e).expect("snapshot present");
        assert_eq!(
            snap.provider_transaction_id, "txn_01h8xc...original",
            "adjustment must key off the referenced transaction id, not adj_…"
        );
        assert_eq!(
            snap.amount_total_minor, 1200,
            "adjustment total must come from data.totals.total, not 0"
        );
        assert_ne!(snap.amount_total_minor, 0, "refund amount must not be 0");
        assert_eq!(snap.amount_tax_minor, 200);
        assert_eq!(snap.currency, "GBP");
        assert_eq!(snap.status, "refunded");
        assert_eq!(
            snap.provider_subscription_id.as_deref(),
            Some("sub_01h8x...")
        );
        assert_eq!(snap.provider_customer_id, "ctm_01h8x...");
        // No settlement time on an adjustment - preserve the original txn's.
        assert!(snap.paid_at.is_none());
    }

    /// `extract_payload_ids` for an adjustment must surface `transaction_id`
    /// as the transaction pointer - the framework keys the mirror upsert off
    /// `PayloadIds::transaction_id`, so reading `data.id` (the adjustment id)
    /// here would mis-route the refund.
    #[test]
    fn extract_payload_ids_adjustment_uses_transaction_id() {
        let p = provider();
        let e = event(
            NeutralEventKind::PaymentRefunded,
            serde_json::json!({ "data": {
                "id": "adj_refund_99",
                "transaction_id": "txn_being_refunded",
                "customer_id": "ctm_adj",
                "subscription_id": "sub_adj"
            } }),
        );
        let ids = p.extract_payload_ids(&e);
        assert_eq!(
            ids.transaction_id.as_deref(),
            Some("txn_being_refunded"),
            "must point at the referenced transaction, not the adjustment id"
        );
        assert_ne!(
            ids.transaction_id.as_deref(),
            Some("adj_refund_99"),
            "adjustment id must never become the transaction key"
        );
        assert_eq!(ids.customer_id.as_deref(), Some("ctm_adj"));
        assert_eq!(ids.subscription_id.as_deref(), Some("sub_adj"));
    }

    /// A transaction payload still reads `data.id` and `data.details.totals.*` -
    /// the adjustment branch must not regress the transaction path.
    #[test]
    fn extract_payload_ids_transaction_event_uses_data_id() {
        let p = provider();
        let e = event(
            NeutralEventKind::PaymentSucceeded,
            serde_json::json!({ "data": {
                "id": "txn_normal",
                "customer_id": "ctm_n",
                "subscription_id": "sub_n"
            } }),
        );
        let ids = p.extract_payload_ids(&e);
        assert_eq!(ids.transaction_id.as_deref(), Some("txn_normal"));
        assert_eq!(ids.customer_id.as_deref(), Some("ctm_n"));
        assert_eq!(ids.subscription_id.as_deref(), Some("sub_n"));
    }

    #[test]
    fn extract_customer_snapshot_reads_data_directly() {
        let p = provider();
        let e = event(
            NeutralEventKind::CustomerUpdated,
            serde_json::json!({ "data": {
                "id": "ctm_email",
                "email": "buyer@example.com",
                "name": "Buyer"
            } }),
        );
        let snap = p.extract_customer_snapshot(&e).expect("snapshot present");
        assert_eq!(snap.provider_customer_id, "ctm_email");
        assert_eq!(snap.email.as_deref(), Some("buyer@example.com"));
        assert_eq!(snap.provider_metadata["name"], "Buyer");
    }
    /// Every neutral kind, so the hostile-payload sweep below covers the
    /// dispatch arm for each one rather than whichever happened to be
    /// convenient. A new variant added without a matching extractor arm
    /// shows up here as a compile error, which is the point.
    const ALL_EVENT_KINDS: &[NeutralEventKind] = &[
        NeutralEventKind::PaymentSucceeded,
        NeutralEventKind::PaymentFailed,
        NeutralEventKind::PaymentRefunded,
        NeutralEventKind::PaymentDisputed,
        NeutralEventKind::SubscriptionCreated,
        NeutralEventKind::SubscriptionUpdated,
        NeutralEventKind::SubscriptionCanceled,
        NeutralEventKind::InvoicePaid,
        NeutralEventKind::InvoiceFailed,
        NeutralEventKind::CustomerCreated,
        NeutralEventKind::CustomerUpdated,
    ];

    /// CI-06 - a signature-valid event whose *body* is hostile must never
    /// panic the endpoint.
    ///
    /// Signature verification only proves the sender holds the key. It says
    /// nothing about the shape of what they sent, and a provider can change
    /// a payload shape without warning. Every extractor here reads through
    /// `and_then` / `unwrap_or` / `?` rather than indexing, so the intent is
    /// already right; nothing pinned it, so a later `[...]` or `.unwrap()`
    /// would have turned a surprising payload into a 500 - or a panic that
    /// takes the worker down, since these run inside the webhook handler.
    ///
    /// `catch_unwind` rather than a plain call: the claim is specifically
    /// "does not unwind", and a plain call that panics fails with a stack
    /// trace instead of this explanation.
    #[test]
    fn hostile_event_payloads_never_unwind_the_extractors() {
        let p = provider();

        // Two tiers, and the second is the one that matters.
        //
        // Shallow payloads exercise the early `?` guards, but they bail out
        // long before any parsing - a test built only from those passes
        // even with a `.unwrap()` planted in `parse_minor`, which is
        // exactly what happened on the first draft of this test.
        //
        // So the deep payloads below are shaped like REAL Paddle events -
        // they carry the `id` / `transaction_id` and the `/details/totals`
        // and `/totals` paths each arm actually reads - and are hostile only
        // in the TYPE of a leaf field. That is the realistic attack and the
        // realistic provider-drift, and it is the only shape that reaches
        // the code that converts values.
        let hostile = [
            // Shallow: must not unwind on the way to an early return.
            serde_json::json!(null),
            serde_json::json!({}),
            serde_json::json!({ "data": null }),
            serde_json::json!({ "data": [] }),
            serde_json::json!({ "data": "a string where an object belongs" }),
            serde_json::json!({ "data": { "id": 12345 } }),
            serde_json::json!({ "data": { "id": { "nested": "object" } } }),
            // Deep: well-formed enough to reach the value conversions,
            // hostile in a leaf type. `total` as a non-numeric string is the
            // case a naive `parse().unwrap()` dies on.
            serde_json::json!({
                "id": "txn_1",
                "transaction_id": "txn_1",
                "currency_code": "USD",
                "totals": { "total": "not-a-number", "tax": "also-not" },
                "details": { "totals": { "total": "not-a-number", "tax": "also-not" } },
            }),
            serde_json::json!({
                "id": "txn_2",
                "transaction_id": "txn_2",
                "totals": { "total": [1, 2, 3], "tax": {} },
                "details": { "totals": { "total": [1, 2, 3], "tax": {} } },
            }),
            serde_json::json!({
                "id": "txn_3",
                "transaction_id": "txn_3",
                // i64::MAX + 1 as a string, and a float where an int belongs.
                "totals": { "total": "9223372036854775808", "tax": 1.5 },
                "details": { "totals": { "total": "9223372036854775808", "tax": 1.5 } },
            }),
            serde_json::json!({
                "id": "txn_4",
                "transaction_id": "txn_4",
                "currency_code": 42,
                "billed_at": "not-a-timestamp",
                "totals": { "total": null, "tax": true },
                "details": { "totals": { "total": null, "tax": true } },
            }),
        ];

        for payload in hostile {
            // Each payload is tried both bare and wrapped in `data`, since
            // the shallow cases carry their own `data` key and the deep ones
            // describe the object that lives *inside* it.
            let shapes = [payload.clone(), serde_json::json!({ "data": payload })];
            for shape in &shapes {
                for kind in ALL_EVENT_KINDS {
                    let ev = event(*kind, shape.clone());
                    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let _ = p.extract_payload_ids(&ev);
                        let _ = p.extract_payment_snapshot(&ev);
                        let _ = p.extract_customer_snapshot(&ev);
                    }));
                    assert!(
                        outcome.is_ok(),
                        "extracting from a {kind:?} event with payload {shape} unwound; \
                     an authenticated sender must not be able to panic the handler \
                     by changing a field's type"
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod signature_hex_tests {
    //! P2-11 - a malformed signature digest must be a refusal, not a panic.

    use super::*;
    use crate::{PaddleEnvironment, PaddleProvider};

    fn provider() -> PaddleProvider {
        PaddleProvider::new(
            "pdl_test_apikey",
            "pdl_ntfset_testwebhookkey",
            "test_clienttoken",
            PaddleEnvironment::Sandbox,
        )
        .expect("paddle provider construction")
    }

    fn verify_with(signature: &str) -> PaymentResult<()> {
        let p = provider();
        let mut headers = http::HeaderMap::new();
        headers.insert(
            "paddle-signature",
            signature.parse().expect("header value must be valid"),
        );
        let ctx = WebhookContext {
            body: br#"{"event_id":"evt_test"}"#,
            headers: &headers,
            remote_addr: None,
        };
        p.verify(&ctx)
    }

    /// The headline case. Probed against the pinned SDK before writing the
    /// fix: `h1=abc` panicked inside `Paddle::unmarshal` rather than
    /// returning an error. `catch_unwind` rather than a plain call, so this
    /// keeps failing if the guard is removed even after a future SDK bump
    /// converts the panic into something else.
    #[test]
    fn odd_length_digest_is_refused_without_unwinding() {
        for signature in [
            "ts=1671552777;h1=abc",
            "ts=1671552777;h1=0123456789abcde",
            "ts=1671552777;h1=f",
        ] {
            let outcome =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| verify_with(signature)));

            let result = outcome.unwrap_or_else(|_| {
                panic!(
                    "verifying `{signature}` unwound - an attacker-controlled \
                     header must never panic the webhook endpoint"
                )
            });
            let err = result.expect_err("a malformed digest must not verify");
            assert!(
                matches!(err, PaymentError::WebhookSignature(ref m) if m.contains("odd number")),
                "expected an odd-length refusal naming the cause, got: {err:?}"
            );
        }
    }

    #[test]
    fn non_hex_digest_is_refused() {
        let err = verify_with("ts=1671552777;h1=zzzz").expect_err("non-hex must not verify");
        assert!(
            matches!(err, PaymentError::WebhookSignature(ref m) if m.contains("non-hex")),
            "expected a non-hex refusal, got: {err:?}"
        );
    }

    /// The guard must not swallow well-formed-but-wrong signatures into its
    /// own error: those still belong to the SDK, which checks them against
    /// the key. A guard that rejected everything would pass the tests above
    /// while breaking every real webhook.
    #[test]
    fn a_well_formed_digest_reaches_the_sdk() {
        let err = verify_with("ts=1671552777;h1=0123456789abcdef")
            .expect_err("a wrong-but-well-formed digest must still fail verification");
        let message = format!("{err}");
        assert!(
            !message.contains("odd number") && !message.contains("non-hex"),
            "a well-formed digest must be rejected by the SDK's own \
             verification, not by the hex guard; got: {message}"
        );
    }

    /// Malformed headers the SDK already handles cleanly must keep their
    /// existing behaviour - the guard is additive, not a replacement.
    #[test]
    fn structurally_invalid_headers_still_error_cleanly() {
        for signature in ["not-a-signature", "ts=1671552777", ""] {
            let outcome =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| verify_with(signature)));
            let result =
                outcome.unwrap_or_else(|_| panic!("verifying `{signature}` unwound; it must not"));
            assert!(
                result.is_err(),
                "`{signature}` must not verify against a real key"
            );
        }
    }

    /// A body that is not JSON at all must be a clean error, not a panic.
    #[test]
    fn non_json_bodies_are_rejected_without_unwinding() {
        let p = provider();
        for body in [
            &b""[..],
            b"not json",
            b"{",
            b"[[[[[[[[[[",
            b"\x00\x01\x02",
            b"{\"id\": ",
        ] {
            let outcome =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| p.parse_event(body)));
            let result =
                outcome.unwrap_or_else(|_| panic!("parsing {body:?} unwound; it must return Err"));
            assert!(
                result.is_err(),
                "{body:?} is not a valid event body and must be refused"
            );
        }
    }
}
