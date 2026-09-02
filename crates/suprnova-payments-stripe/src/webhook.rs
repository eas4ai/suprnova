//! Implementation of the `WebhookHandler` trait for `StripeProvider`.
//!
//! Verifies Stripe's `t=<ts>,v1=<hex_sig>` signature format using HMAC-SHA256
//! and parses the incoming event body into a `WebhookEvent`.

use crate::{StripeProvider, event_map::stripe_event_to_neutral};
use async_trait::async_trait;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use suprnova::payments::{
    CustomerSnapshot, NeutralEventKind, PayloadIds, PaymentError, PaymentResult, PaymentSnapshot,
    WebhookContext, WebhookEvent, WebhookHandler, constant_time_eq,
};

type HmacSha256 = Hmac<Sha256>;

fn stripe_snapshot_error(event: &WebhookEvent, field: &str, expectation: &str) -> PaymentError {
    PaymentError::Provider(format!(
        "malformed stripe {} snapshot: {field} {expectation}",
        event.provider_event_type
    ))
}

fn required_stripe_string(
    event: &WebhookEvent,
    object: &serde_json::Value,
    field: &str,
) -> PaymentResult<String> {
    object
        .get(field)
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| stripe_snapshot_error(event, field, "must be a non-empty string"))
}

fn stripe_expandable_id(value: Option<&serde_json::Value>) -> Option<&str> {
    match value {
        Some(serde_json::Value::String(value)) => Some(value),
        Some(serde_json::Value::Object(value)) => value.get("id").and_then(|id| id.as_str()),
        _ => None,
    }
}

fn optional_stripe_expandable_value(
    event: &WebhookEvent,
    value: Option<&serde_json::Value>,
    field: &str,
) -> PaymentResult<Option<String>> {
    match value {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => stripe_expandable_id(Some(value))
            .filter(|id| !id.trim().is_empty())
            .map(str::to_owned)
            .map(Some)
            .ok_or_else(|| {
                stripe_snapshot_error(event, field, "must be a non-empty identifier when present")
            }),
    }
}

fn optional_stripe_expandable_id(
    event: &WebhookEvent,
    object: &serde_json::Value,
    field: &str,
) -> PaymentResult<Option<String>> {
    optional_stripe_expandable_value(event, object.get(field), field)
}

fn optional_expanded_relation_id(
    event: &WebhookEvent,
    object: &serde_json::Value,
    relation: &str,
    field: &str,
) -> PaymentResult<Option<String>> {
    match object.get(relation) {
        None | Some(serde_json::Value::Null | serde_json::Value::String(_)) => Ok(None),
        Some(serde_json::Value::Object(expanded)) => {
            optional_stripe_expandable_value(event, expanded.get(field), field)
        }
        Some(_) => Err(stripe_snapshot_error(
            event,
            relation,
            "must be an identifier or expanded object",
        )),
    }
}

fn required_stripe_i64(
    event: &WebhookEvent,
    value: Option<&serde_json::Value>,
    field: &str,
) -> PaymentResult<i64> {
    value
        .and_then(|value| value.as_i64())
        .ok_or_else(|| stripe_snapshot_error(event, field, "must be an integer"))
}

fn optional_stripe_i64(
    event: &WebhookEvent,
    value: Option<&serde_json::Value>,
    field: &str,
) -> PaymentResult<i64> {
    match value {
        None | Some(serde_json::Value::Null) => Ok(0),
        Some(value) => value
            .as_i64()
            .ok_or_else(|| stripe_snapshot_error(event, field, "must be an integer when present")),
    }
}

fn stripe_invoice_tax_minor(
    event: &WebhookEvent,
    object: &serde_json::Value,
) -> PaymentResult<i64> {
    match object.get("total_taxes") {
        None | Some(serde_json::Value::Null) => {
            optional_stripe_i64(event, object.get("tax"), "tax")
        }
        Some(serde_json::Value::Array(taxes)) => {
            let mut total = 0_i64;
            for (index, tax) in taxes.iter().enumerate() {
                let field = format!("total_taxes[{index}].amount");
                let amount = tax
                    .get("amount")
                    .and_then(|value| value.as_i64())
                    .ok_or_else(|| stripe_snapshot_error(event, &field, "must be an integer"))?;
                total = total.checked_add(amount).ok_or_else(|| {
                    stripe_snapshot_error(event, "total_taxes", "sum is outside the valid range")
                })?;
            }
            Ok(total)
        }
        Some(_) => Err(stripe_snapshot_error(
            event,
            "total_taxes",
            "must be an array when present",
        )),
    }
}

fn required_stripe_currency(
    event: &WebhookEvent,
    object: &serde_json::Value,
) -> PaymentResult<String> {
    let currency = required_stripe_string(event, object, "currency")?;
    if currency.len() == 3 && currency.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        Ok(currency.to_uppercase())
    } else {
        Err(stripe_snapshot_error(
            event,
            "currency",
            "must be a three-letter code",
        ))
    }
}

fn optional_stripe_timestamp(
    event: &WebhookEvent,
    value: Option<&serde_json::Value>,
    field: &str,
) -> PaymentResult<Option<chrono::DateTime<chrono::Utc>>> {
    match value {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => {
            let seconds = value.as_i64().ok_or_else(|| {
                stripe_snapshot_error(event, field, "must be an integer timestamp when present")
            })?;
            chrono::DateTime::from_timestamp(seconds, 0)
                .map(Some)
                .ok_or_else(|| stripe_snapshot_error(event, field, "is outside the valid range"))
        }
    }
}

fn checked_stripe_payment_snapshot(event: &WebhookEvent) -> PaymentResult<Option<PaymentSnapshot>> {
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

    let object = event
        .raw_payload
        .pointer("/data/object")
        .ok_or_else(|| stripe_snapshot_error(event, "data.object", "must be present"))?;
    let currency = required_stripe_currency(event, object)?;

    let snapshot = match kind {
        NeutralEventKind::PaymentSucceeded | NeutralEventKind::PaymentFailed => {
            let provider_transaction_id = required_stripe_string(event, object, "id")?;
            let provider_customer_id = optional_stripe_expandable_id(event, object, "customer")?;
            let amount_total_minor = required_stripe_i64(event, object.get("amount"), "amount")?;
            let status = match kind {
                NeutralEventKind::PaymentSucceeded => "succeeded",
                NeutralEventKind::PaymentFailed => "failed",
                _ => unreachable!(),
            };
            let paid_at = if kind == NeutralEventKind::PaymentSucceeded {
                optional_stripe_timestamp(event, object.get("created"), "created")?
            } else {
                None
            };
            let Some(provider_customer_id) = provider_customer_id else {
                return Ok(None);
            };
            PaymentSnapshot {
                provider_transaction_id,
                provider_customer_id,
                provider_subscription_id: None,
                amount_total_minor,
                amount_tax_minor: 0,
                currency,
                status: status.to_owned(),
                paid_at,
                provider_metadata: object.clone(),
            }
        }
        NeutralEventKind::PaymentRefunded => {
            let provider_transaction_id =
                match optional_stripe_expandable_id(event, object, "payment_intent")? {
                    Some(payment_intent_id) => payment_intent_id,
                    None => required_stripe_string(event, object, "id")?,
                };
            let provider_customer_id = optional_stripe_expandable_id(event, object, "customer")?;
            let amount_total_minor = required_stripe_i64(event, object.get("amount"), "amount")?;
            let Some(provider_customer_id) = provider_customer_id else {
                return Ok(None);
            };
            PaymentSnapshot {
                provider_transaction_id,
                provider_customer_id,
                provider_subscription_id: None,
                amount_total_minor,
                amount_tax_minor: 0,
                currency,
                status: "refunded".to_owned(),
                paid_at: None,
                provider_metadata: object.clone(),
            }
        }
        NeutralEventKind::PaymentDisputed => {
            let payment_intent_id = optional_stripe_expandable_id(event, object, "payment_intent")?;
            let charge_payment_intent_id =
                optional_expanded_relation_id(event, object, "charge", "payment_intent")?;
            let charge_id = optional_stripe_expandable_id(event, object, "charge")?;
            let provider_transaction_id = payment_intent_id
                .or(charge_payment_intent_id)
                .or(charge_id)
                .ok_or_else(|| {
                    stripe_snapshot_error(
                        event,
                        "payment_intent/charge",
                        "must identify the disputed payment",
                    )
                })?;
            let payment_intent_customer =
                optional_expanded_relation_id(event, object, "payment_intent", "customer")?;
            let charge_customer =
                optional_expanded_relation_id(event, object, "charge", "customer")?;
            let amount_total_minor = required_stripe_i64(event, object.get("amount"), "amount")?;
            let Some(provider_customer_id) = payment_intent_customer.or(charge_customer) else {
                // Stripe's ordinary Dispute webhook contains relationship IDs,
                // not an expanded Charge or PaymentIntent, so it cannot supply
                // the complete snapshot required by the mirror table.
                return Ok(None);
            };
            PaymentSnapshot {
                provider_transaction_id,
                provider_customer_id,
                provider_subscription_id: None,
                amount_total_minor,
                amount_tax_minor: 0,
                currency,
                status: "disputed".to_owned(),
                paid_at: None,
                provider_metadata: object.clone(),
            }
        }
        NeutralEventKind::InvoicePaid | NeutralEventKind::InvoiceFailed => {
            let provider_transaction_id = required_stripe_string(event, object, "id")?;
            let provider_customer_id = optional_stripe_expandable_id(event, object, "customer")?;
            let amount_total_minor = match object.get("amount_paid") {
                None | Some(serde_json::Value::Null) => {
                    required_stripe_i64(event, object.get("amount_due"), "amount_due")?
                }
                value => required_stripe_i64(event, value, "amount_paid")?,
            };
            let amount_tax_minor = stripe_invoice_tax_minor(event, object)?;
            let provider_subscription_id =
                optional_stripe_expandable_id(event, object, "subscription")?;
            let paid_at_value = match object.get("status_transitions") {
                None | Some(serde_json::Value::Null) => None,
                Some(serde_json::Value::Object(transitions)) => transitions.get("paid_at"),
                Some(_) => {
                    return Err(stripe_snapshot_error(
                        event,
                        "status_transitions",
                        "must be an object when present",
                    ));
                }
            };
            let paid_at =
                optional_stripe_timestamp(event, paid_at_value, "status_transitions.paid_at")?;
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
                status: if kind == NeutralEventKind::InvoicePaid {
                    "succeeded".to_owned()
                } else {
                    "failed".to_owned()
                },
                paid_at,
                provider_metadata: object.clone(),
            }
        }
        _ => unreachable!(),
    };

    Ok(Some(snapshot))
}

// ---------------------------------------------------------------------------
// Trait implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl WebhookHandler for StripeProvider {
    /// Verify a Stripe webhook signature.
    ///
    /// Stripe sends a `Stripe-Signature` header with the format:
    /// `t=<unix_timestamp>,v1=<hex_hmac_sha256>[,v1=<additional_sig>]`
    ///
    /// We recompute HMAC-SHA256 over `"<timestamp>.<body>"` using the webhook
    /// signing secret and do a constant-time comparison against every `v1=` value
    /// in the header (Stripe can rotate keys without instant cutover).
    ///
    /// The timestamp is also compared against the local clock and rejected
    /// when the absolute delta exceeds
    /// [`StripeProvider::with_signature_tolerance`] (default 300 seconds,
    /// matching Stripe's official libraries). Without this check a signature
    /// remains valid forever, so a captured signed body could be replayed.
    fn verify(&self, ctx: &WebhookContext<'_>) -> PaymentResult<()> {
        // A blank signing secret is an empty-key HMAC, which any caller can
        // forge. `from_env` refuses to construct one, but `new` takes the
        // secret verbatim, so guard the boundary that actually decides trust -
        // that covers every construction path, not just the documented one.
        if self.webhook_signing_secret().trim().is_empty() {
            return Err(PaymentError::WebhookSignature(
                "stripe webhook signing secret is empty - refusing to verify against an \
                 empty-key HMAC"
                    .into(),
            ));
        }

        let header = ctx
            .headers
            .get("stripe-signature")
            .ok_or_else(|| {
                PaymentError::WebhookSignature("missing stripe-signature header".into())
            })?
            .to_str()
            .map_err(|_| {
                PaymentError::WebhookSignature("non-ascii stripe-signature header".into())
            })?;

        let mut timestamp: Option<&str> = None;
        let mut v1_sigs: Vec<&str> = Vec::new();

        for pair in header.split(',') {
            let mut it = pair.splitn(2, '=');
            match (it.next(), it.next()) {
                (Some("t"), Some(v)) => timestamp = Some(v),
                (Some("v1"), Some(v)) => v1_sigs.push(v),
                _ => {}
            }
        }

        let timestamp = timestamp.ok_or_else(|| {
            PaymentError::WebhookSignature("missing timestamp in stripe-signature header".into())
        })?;

        // Reject implausible timestamps before any HMAC work: stale events
        // can be discarded without spending CPU cycles, and a malformed `t=`
        // value is itself a signature failure rather than a silent bypass.
        let ts: i64 = timestamp.parse().map_err(|_| {
            PaymentError::WebhookSignature(format!(
                "non-numeric timestamp in stripe-signature header: {timestamp}"
            ))
        })?;
        let tolerance = self.webhook_signature_tolerance_seconds();
        let now = chrono::Utc::now().timestamp();
        if (now - ts).abs() > tolerance {
            return Err(PaymentError::WebhookSignature(format!(
                "timestamp outside tolerance window of {tolerance}s (now={now}, sig_ts={ts})"
            )));
        }

        if v1_sigs.is_empty() {
            return Err(PaymentError::WebhookSignature(
                "no v1 signature in stripe-signature header".into(),
            ));
        }

        let mut mac = HmacSha256::new_from_slice(self.webhook_signing_secret().as_bytes())
            .map_err(|_| PaymentError::Internal("HMAC key error".into()))?;
        mac.update(timestamp.as_bytes());
        mac.update(b".");
        mac.update(ctx.body);
        let expected_bytes = mac.finalize().into_bytes();
        let expected_hex = hex::encode(expected_bytes);

        if v1_sigs
            .iter()
            .any(|s| constant_time_eq(s.as_bytes(), expected_hex.as_bytes()))
        {
            Ok(())
        } else {
            Err(PaymentError::WebhookSignature(
                "no matching v1 signature".into(),
            ))
        }
    }

    /// Parse a raw Stripe webhook body into a `WebhookEvent`.
    ///
    /// Extracts `id` and `type` from the JSON envelope and maps `type` to a
    /// `NeutralEventKind` via `stripe_event_to_neutral`. The full raw JSON
    /// is preserved in `raw_payload` for provider-specific handlers.
    fn parse_event(&self, body: &[u8]) -> PaymentResult<WebhookEvent> {
        let raw: serde_json::Value = serde_json::from_slice(body)
            .map_err(|e| PaymentError::Validation(format!("invalid stripe webhook body: {e}")))?;

        let provider_event_id = raw
            .get("id")
            .and_then(|value| value.as_str())
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                PaymentError::Validation("stripe webhook id must be a non-empty string".to_owned())
            })?
            .to_owned();

        let provider_event_type = raw
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let neutral = stripe_event_to_neutral(&provider_event_type);

        Ok(WebhookEvent {
            provider: "stripe".into(),
            provider_event_id,
            provider_event_type,
            neutral,
            raw_payload: raw,
        })
    }

    /// Extract IDs from Stripe's `data.object.*` envelope.
    ///
    /// Every webhook puts its entity at `data.object`, but refund and dispute
    /// events must be keyed by their referenced PaymentIntent or Charge rather
    /// than by the adjustment object's own identifier.
    fn extract_payload_ids(&self, event: &WebhookEvent) -> PayloadIds {
        let obj = match event.raw_payload.pointer("/data/object") {
            Some(o) => o,
            None => return PayloadIds::default(),
        };

        let mut ids = PayloadIds::default();

        match event.neutral {
            Some(
                NeutralEventKind::SubscriptionCreated
                | NeutralEventKind::SubscriptionUpdated
                | NeutralEventKind::SubscriptionCanceled,
            ) => {
                ids.subscription_id = obj.get("id").and_then(|v| v.as_str()).map(String::from);
                ids.customer_id = stripe_expandable_id(obj.get("customer")).map(String::from);
            }
            Some(NeutralEventKind::CustomerCreated | NeutralEventKind::CustomerUpdated) => {
                ids.customer_id = obj.get("id").and_then(|v| v.as_str()).map(String::from);
            }
            Some(NeutralEventKind::PaymentSucceeded | NeutralEventKind::PaymentFailed) => {
                ids.transaction_id = obj.get("id").and_then(|v| v.as_str()).map(String::from);
                ids.customer_id = stripe_expandable_id(obj.get("customer")).map(String::from);
            }
            Some(NeutralEventKind::PaymentRefunded) => {
                ids.transaction_id = stripe_expandable_id(obj.get("payment_intent"))
                    .or_else(|| obj.get("id").and_then(|value| value.as_str()))
                    .map(String::from);
                ids.customer_id = stripe_expandable_id(obj.get("customer")).map(String::from);
            }
            Some(NeutralEventKind::PaymentDisputed) => {
                ids.transaction_id = stripe_expandable_id(obj.get("payment_intent"))
                    .or_else(|| {
                        obj.get("charge")
                            .and_then(|charge| charge.as_object())
                            .and_then(|charge| stripe_expandable_id(charge.get("payment_intent")))
                    })
                    .or_else(|| stripe_expandable_id(obj.get("charge")))
                    .map(String::from);
                ids.customer_id = obj
                    .get("payment_intent")
                    .and_then(|payment| payment.as_object())
                    .and_then(|payment| stripe_expandable_id(payment.get("customer")))
                    .or_else(|| {
                        obj.get("charge")
                            .and_then(|charge| charge.as_object())
                            .and_then(|charge| stripe_expandable_id(charge.get("customer")))
                    })
                    .map(String::from);
            }
            Some(NeutralEventKind::InvoicePaid | NeutralEventKind::InvoiceFailed) => {
                ids.transaction_id = obj.get("id").and_then(|v| v.as_str()).map(String::from);
                ids.customer_id = stripe_expandable_id(obj.get("customer")).map(String::from);
                ids.subscription_id =
                    stripe_expandable_id(obj.get("subscription")).map(String::from);
            }
            None => {}
        }
        ids
    }

    /// Build a [`PaymentSnapshot`] from a Stripe payment / invoice payload.
    ///
    /// - `payment_intent.*` → uses `id`, `amount`, `currency`, `status`, `customer`
    /// - `charge.refunded` → uses Charge fields and prefers `payment_intent`
    ///   as the transaction key
    /// - `charge.dispute.created` → uses the Dispute's referenced
    ///   PaymentIntent / Charge; an unexpanded relationship is audit-only
    /// - `invoice.*` → uses `id`, `amount_paid`, `total_taxes[].amount`,
    ///   `currency`, `customer`, `subscription`, `status_transitions.paid_at`
    ///   (with scalar `tax` accepted for legacy payloads)
    ///
    /// Returns `None` for subscription / customer events (those go through
    /// the `extract_payload_ids` + provider API path).
    fn extract_payment_snapshot(&self, event: &WebhookEvent) -> Option<PaymentSnapshot> {
        checked_stripe_payment_snapshot(event).ok().flatten()
    }

    fn try_extract_payment_snapshot(
        &self,
        event: &WebhookEvent,
    ) -> PaymentResult<Option<PaymentSnapshot>> {
        checked_stripe_payment_snapshot(event)
    }

    /// Build a [`CustomerSnapshot`] from Stripe `customer.created` /
    /// `customer.updated` payloads. Stripe puts the full Customer object at
    /// `data.object` - we pull `id` + `email` and keep the rest in
    /// `provider_metadata` for downstream readers.
    fn extract_customer_snapshot(&self, event: &WebhookEvent) -> Option<CustomerSnapshot> {
        match event.neutral? {
            NeutralEventKind::CustomerCreated | NeutralEventKind::CustomerUpdated => {
                let obj = event.raw_payload.pointer("/data/object")?;
                let provider_customer_id = obj.get("id")?.as_str()?.to_string();
                let email = obj
                    .get("email")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                Some(CustomerSnapshot {
                    provider_customer_id,
                    email,
                    provider_metadata: obj.clone(),
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

    /// Install the rustls ring CryptoProvider exactly once - `StripeProvider::new`
    /// constructs a hyper-rustls client which panics at TLS init when both
    /// `aws-lc-rs` and `ring` are in the dep graph (as they are via async-stripe).
    fn install_crypto_provider() {
        static ONCE: std::sync::OnceLock<()> = std::sync::OnceLock::new();
        ONCE.get_or_init(|| {
            let _ = rustls::crypto::ring::default_provider().install_default();
        });
    }

    fn provider() -> StripeProvider {
        install_crypto_provider();
        StripeProvider::new("sk_test_dummy", "pk_test_dummy", "whsec_dummy")
    }

    #[test]
    fn parse_event_rejects_invalid_ids_and_preserves_nonblank_ids() {
        let provider = provider();
        for payload in [
            serde_json::json!({ "type": "payment_intent.succeeded" }),
            serde_json::json!({ "id": null, "type": "payment_intent.succeeded" }),
            serde_json::json!({ "id": 42, "type": "payment_intent.succeeded" }),
            serde_json::json!({ "id": "", "type": "payment_intent.succeeded" }),
            serde_json::json!({ "id": " \t\r\n", "type": "payment_intent.succeeded" }),
        ] {
            let body = serde_json::to_vec(&payload).expect("serialize payload");
            let error = provider
                .parse_event(&body)
                .expect_err("an invalid Stripe event id must be rejected");
            assert!(matches!(
                error,
                PaymentError::Validation(ref message) if message.contains("id")
            ));
        }

        let event = provider
            .parse_event(br#"{"id":" evt_preserved ","type":"payment_intent.succeeded"}"#)
            .expect("a nonblank Stripe event id is valid");
        assert_eq!(event.provider_event_id, " evt_preserved ");
    }

    #[test]
    fn verify_refuses_a_blank_signing_secret() {
        // `new` accepts the secret verbatim, so a config path that reads a
        // set-but-empty `STRIPE_WEBHOOK_SIGNING_SECRET` can still build a
        // provider whose HMAC key is empty. Verification must refuse it
        // outright rather than compare against a forgeable digest - and it
        // must refuse before parsing the header, so a well-formed signature
        // over an empty key never reaches the comparison.
        install_crypto_provider();
        let p = StripeProvider::new("sk_test_dummy", "pk_test_dummy", "");
        let mut headers = http::HeaderMap::new();
        headers.insert(
            "stripe-signature",
            http::HeaderValue::from_static("t=1,v1=deadbeef"),
        );
        let ctx = WebhookContext {
            body: b"{}",
            headers: &headers,
            remote_addr: None,
        };

        let err = p
            .verify(&ctx)
            .expect_err("blank signing secret must not verify");
        assert!(
            matches!(err, PaymentError::WebhookSignature(ref m) if m.contains("empty")),
            "expected an empty-key signature refusal, got: {err:?}"
        );
    }

    // ---------------------------------------------------------------
    // CI-06 - signature verification, positive and negative
    // ---------------------------------------------------------------
    //
    // Until these landed, `verify()` had exactly one test: the blank-secret
    // refusal. Nothing ever constructed a VALID signature, which means the
    // success path was unproven and - far worse - so was every rejection
    // that depends on reaching the HMAC comparison. A regression that made
    // `verify()` accept anything would have passed the suite, because no
    // test could tell "correctly rejected" apart from "rejected because the
    // test never built a real signature".
    //
    // So the positive case comes first, and every negative case below is a
    // single-field mutation of it. That is what makes them meaningful: each
    // one differs from a signature known to work in exactly one way.

    const TEST_SECRET: &str = "whsec_ci06_fixed_key";

    fn signed_provider() -> StripeProvider {
        install_crypto_provider();
        StripeProvider::new("sk_test_dummy", "pk_test_dummy", TEST_SECRET)
    }

    /// Compute the signature Stripe would send for this timestamp and body.
    fn sign(timestamp: i64, body: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(TEST_SECRET.as_bytes()).expect("hmac key");
        mac.update(timestamp.to_string().as_bytes());
        mac.update(b".");
        mac.update(body);
        hex::encode(mac.finalize().into_bytes())
    }

    fn headers_with(value: &str) -> http::HeaderMap {
        let mut headers = http::HeaderMap::new();
        headers.insert(
            "stripe-signature",
            http::HeaderValue::from_str(value).expect("ascii header"),
        );
        headers
    }

    #[test]
    fn verify_accepts_a_correctly_signed_payload() {
        let p = signed_provider();
        let body = br#"{"id":"evt_1","type":"invoice.paid"}"#;
        let ts = chrono::Utc::now().timestamp();
        let headers = headers_with(&format!("t={ts},v1={}", sign(ts, body)));

        p.verify(&WebhookContext {
            body,
            headers: &headers,
            remote_addr: None,
        })
        .expect(
            "a correctly signed payload must verify - without this the negatives prove nothing",
        );
    }

    #[test]
    fn verify_rejects_a_tampered_body() {
        // The signature is genuine, computed over the original body; only
        // the body changed. This is the attack the endpoint exists to stop:
        // replaying a real Stripe event with the amount edited.
        let p = signed_provider();
        let original = br#"{"id":"evt_1","type":"invoice.paid","amount":100}"#;
        let tampered = br#"{"id":"evt_1","type":"invoice.paid","amount":999999}"#;
        let ts = chrono::Utc::now().timestamp();
        let headers = headers_with(&format!("t={ts},v1={}", sign(ts, original)));

        let err = p
            .verify(&WebhookContext {
                body: tampered,
                headers: &headers,
                remote_addr: None,
            })
            .expect_err("a body that does not match its signature must be refused");
        assert!(
            matches!(err, PaymentError::WebhookSignature(ref m) if m.contains("no matching")),
            "expected a signature mismatch, got: {err:?}"
        );
    }

    #[test]
    fn verify_binds_the_signature_to_its_timestamp() {
        // A signature valid for timestamp T must not verify when presented
        // with timestamp T'. Stripe signs `timestamp.body`, so this is what
        // stops an attacker refreshing a captured signature past the
        // tolerance window by editing `t=`.
        let p = signed_provider();
        let body = br#"{"id":"evt_1","type":"invoice.paid"}"#;
        let signed_ts = chrono::Utc::now().timestamp();
        let claimed_ts = signed_ts + 1;
        let headers = headers_with(&format!("t={claimed_ts},v1={}", sign(signed_ts, body)));

        let err = p
            .verify(&WebhookContext {
                body,
                headers: &headers,
                remote_addr: None,
            })
            .expect_err("a signature must not verify under a timestamp it did not cover");
        assert!(
            matches!(err, PaymentError::WebhookSignature(ref m) if m.contains("no matching")),
            "expected a signature mismatch, got: {err:?}"
        );
    }

    #[test]
    fn verify_rejects_a_replayed_event_outside_the_tolerance_window() {
        // Correctly signed, genuinely from Stripe, just old. Replay
        // protection is the only thing standing between a captured webhook
        // and an unlimited number of re-deliveries.
        let p = signed_provider();
        let body = br#"{"id":"evt_1","type":"invoice.paid"}"#;
        let stale = chrono::Utc::now().timestamp() - (p.webhook_signature_tolerance_seconds() + 60);
        let headers = headers_with(&format!("t={stale},v1={}", sign(stale, body)));

        let err = p
            .verify(&WebhookContext {
                body,
                headers: &headers,
                remote_addr: None,
            })
            .expect_err("a stale but validly signed event must be refused");
        assert!(
            matches!(err, PaymentError::WebhookSignature(ref m) if m.contains("tolerance")),
            "expected a tolerance-window refusal, got: {err:?}"
        );
    }

    #[test]
    fn verify_rejects_headers_that_carry_no_usable_signature() {
        // Each of these is a distinct way the header can fail to authorize
        // the request, and every one must be an error rather than a
        // fall-through. A missing `v1=` in particular must not read as
        // "nothing to compare, so allow".
        let p = signed_provider();
        let body = br#"{"id":"evt_1","type":"invoice.paid"}"#;
        let ts = chrono::Utc::now().timestamp();

        let cases: [(&str, &str); 4] = [
            ("", "missing"),
            (&format!("t={ts}"), "no v1 signature"),
            (&format!("v1={}", sign(ts, body)), "missing timestamp"),
            (
                &format!("t=not-a-number,v1={}", sign(ts, body)),
                "non-numeric timestamp",
            ),
        ];

        for (header, expected) in cases {
            let headers = if header.is_empty() {
                http::HeaderMap::new()
            } else {
                headers_with(header)
            };
            let err = p
                .verify(&WebhookContext {
                    body,
                    headers: &headers,
                    remote_addr: None,
                })
                .expect_err(&format!(
                    "header `{header}` authorizes nothing and must not verify"
                ));
            // Assert *which* refusal, not merely that one happened: these
            // four fail for four different reasons, and a single collapsed
            // error message would hide three of them regressing.
            assert!(
                matches!(err, PaymentError::WebhookSignature(ref m) if m.contains(expected)),
                "header `{header}` should have been refused with a message \
                 containing `{expected}`, got: {err:?}"
            );
        }
    }

    fn event(neutral: NeutralEventKind, payload: serde_json::Value) -> WebhookEvent {
        WebhookEvent {
            provider: "stripe".into(),
            provider_event_id: "evt_test".into(),
            provider_event_type: format!("{neutral:?}"),
            neutral: Some(neutral),
            raw_payload: payload,
        }
    }

    #[test]
    fn extract_payload_ids_subscription_created() {
        let p = provider();
        let e = event(
            NeutralEventKind::SubscriptionCreated,
            serde_json::json!({
                "data": { "object": { "id": "sub_abc", "customer": "cus_xyz" } }
            }),
        );
        let ids = p.extract_payload_ids(&e);
        assert_eq!(ids.subscription_id.as_deref(), Some("sub_abc"));
        assert_eq!(ids.customer_id.as_deref(), Some("cus_xyz"));
        assert!(ids.transaction_id.is_none());
    }

    #[test]
    fn extract_payload_ids_invoice_paid_carries_subscription() {
        let p = provider();
        let e = event(
            NeutralEventKind::InvoicePaid,
            serde_json::json!({
                "data": { "object": {
                    "id": "in_99",
                    "customer": "cus_77",
                    "subscription": "sub_44"
                }}
            }),
        );
        let ids = p.extract_payload_ids(&e);
        assert_eq!(ids.transaction_id.as_deref(), Some("in_99"));
        assert_eq!(ids.customer_id.as_deref(), Some("cus_77"));
        assert_eq!(ids.subscription_id.as_deref(), Some("sub_44"));
    }

    #[test]
    fn extract_payload_ids_returns_empty_when_data_object_missing() {
        let p = provider();
        let e = event(
            NeutralEventKind::PaymentSucceeded,
            serde_json::json!({ "unexpected": "shape" }),
        );
        let ids = p.extract_payload_ids(&e);
        assert!(ids.subscription_id.is_none());
        assert!(ids.customer_id.is_none());
        assert!(ids.transaction_id.is_none());
    }

    #[test]
    fn extract_payment_snapshot_payment_succeeded() {
        let p = provider();
        let e = event(
            NeutralEventKind::PaymentSucceeded,
            serde_json::json!({
                "data": { "object": {
                    "id": "pi_test",
                    "customer": "cus_1",
                    "amount": 4242,
                    "currency": "usd",
                    "created": 1717000000
                }}
            }),
        );
        let snap = p.extract_payment_snapshot(&e).expect("snapshot present");
        assert_eq!(snap.provider_transaction_id, "pi_test");
        assert_eq!(snap.provider_customer_id, "cus_1");
        assert_eq!(snap.amount_total_minor, 4242);
        assert_eq!(snap.currency, "USD", "currency must be uppercased");
        assert_eq!(snap.status, "succeeded");
        assert!(
            snap.paid_at.is_some(),
            "PaymentSucceeded must parse `created` as paid_at"
        );
    }

    #[test]
    fn checked_payment_snapshot_rejects_malformed_fields() {
        let p = provider();
        let cases = [
            (
                "id",
                serde_json::json!({ "data": { "object": {
                    "id": 42,
                    "customer": "cus_test",
                    "amount": 4200,
                    "currency": "usd"
                } } }),
            ),
            (
                "customer",
                serde_json::json!({ "data": { "object": {
                    "id": "pi_test",
                    "customer": 42,
                    "amount": 4200,
                    "currency": "usd"
                } } }),
            ),
            (
                "amount",
                serde_json::json!({ "data": { "object": {
                    "id": "pi_test",
                    "customer": "cus_test",
                    "amount": "not-a-number",
                    "currency": "usd"
                } } }),
            ),
            (
                "currency",
                serde_json::json!({ "data": { "object": {
                    "id": "pi_test",
                    "customer": "cus_test",
                    "amount": 4200,
                    "currency": 840
                } } }),
            ),
            (
                "created",
                serde_json::json!({ "data": { "object": {
                    "id": "pi_test",
                    "customer": "cus_test",
                    "amount": 4200,
                    "currency": "usd",
                    "created": "yesterday"
                } } }),
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
        for (kind, payload) in [
            (
                NeutralEventKind::PaymentSucceeded,
                serde_json::json!({ "data": { "object": {
                    "id": "pi_guest",
                    "customer": null,
                    "amount": 4200,
                    "currency": "usd"
                } } }),
            ),
            (
                NeutralEventKind::InvoicePaid,
                serde_json::json!({ "data": { "object": {
                    "id": "in_guest",
                    "customer": null,
                    "amount_paid": 4200,
                    "currency": "usd"
                } } }),
            ),
        ] {
            let event = event(kind, payload);
            assert!(
                p.try_extract_payment_snapshot(&event)
                    .expect("a nullable provider relationship is valid")
                    .is_none()
            );
        }

        let malformed = event(
            NeutralEventKind::PaymentSucceeded,
            serde_json::json!({ "data": { "object": {
                "id": "pi_guest_malformed",
                "customer": null,
                "amount": "not-a-number",
                "currency": "usd"
            } } }),
        );
        let error = p
            .try_extract_payment_snapshot(&malformed)
            .expect_err("nullable customer must not hide a malformed amount");
        assert!(matches!(
            error,
            PaymentError::Provider(ref message) if message.contains("amount")
        ));
    }

    #[test]
    fn checked_dispute_snapshot_uses_referenced_payment() {
        let p = provider();
        let unexpanded = event(
            NeutralEventKind::PaymentDisputed,
            serde_json::json!({ "data": { "object": {
                "id": "dp_test",
                "charge": "ch_test",
                "payment_intent": "pi_test",
                "amount": 4200,
                "currency": "usd"
            } } }),
        );
        let ids = p.extract_payload_ids(&unexpanded);
        assert_eq!(
            ids.transaction_id.as_deref(),
            Some("pi_test"),
            "the dispute id is not the transaction mirror key"
        );
        assert!(
            p.try_extract_payment_snapshot(&unexpanded)
                .expect("an ordinary unexpanded dispute is valid")
                .is_none(),
            "without an expanded customer, the route must use its partial-update path"
        );

        let charge_only = event(
            NeutralEventKind::PaymentDisputed,
            serde_json::json!({ "data": { "object": {
                "id": "dp_charge_only",
                "charge": "ch_charge_only",
                "payment_intent": null,
                "amount": 4200,
                "currency": "usd"
            } } }),
        );
        let ids = p.extract_payload_ids(&charge_only);
        assert_eq!(ids.transaction_id.as_deref(), Some("ch_charge_only"));
        assert!(
            p.try_extract_payment_snapshot(&charge_only)
                .expect("a charge-only dispute is valid")
                .is_none()
        );

        let expanded_charge = event(
            NeutralEventKind::PaymentDisputed,
            serde_json::json!({ "data": { "object": {
                "id": "dp_expanded_charge",
                "charge": {
                    "id": "ch_expanded",
                    "payment_intent": "pi_from_charge",
                    "customer": "cus_from_charge"
                },
                "payment_intent": null,
                "amount": 1800,
                "currency": "gbp"
            } } }),
        );
        let ids = p.extract_payload_ids(&expanded_charge);
        assert_eq!(ids.transaction_id.as_deref(), Some("pi_from_charge"));
        let snapshot = p
            .try_extract_payment_snapshot(&expanded_charge)
            .expect("an expanded charge dispute must parse")
            .expect("the expanded charge supplies a complete snapshot");
        assert_eq!(snapshot.provider_transaction_id, "pi_from_charge");
        assert_eq!(snapshot.provider_customer_id, "cus_from_charge");
        assert_eq!(snapshot.amount_total_minor, 1800);
        assert_eq!(snapshot.currency, "GBP");

        let expanded = event(
            NeutralEventKind::PaymentDisputed,
            serde_json::json!({ "data": { "object": {
                "id": "dp_expanded",
                "charge": "ch_expanded",
                "payment_intent": {
                    "id": "pi_expanded",
                    "customer": "cus_expanded"
                },
                "amount": 1200,
                "currency": "eur"
            } } }),
        );
        let snapshot = p
            .try_extract_payment_snapshot(&expanded)
            .expect("expanded dispute must parse")
            .expect("expanded customer makes a complete snapshot");
        assert_eq!(snapshot.provider_transaction_id, "pi_expanded");
        assert_eq!(snapshot.provider_customer_id, "cus_expanded");
        assert_eq!(snapshot.amount_total_minor, 1200);
        assert_eq!(snapshot.currency, "EUR");
        assert_eq!(snapshot.status, "disputed");
    }

    #[test]
    fn checked_invoice_snapshot_rejects_invalid_optional_fields() {
        let p = provider();
        let cases = [
            (
                "total_taxes",
                serde_json::json!({ "data": { "object": {
                    "id": "in_test",
                    "customer": "cus_test",
                    "amount_paid": 4200,
                    "total_taxes": [{ "amount": "not-a-number" }],
                    "currency": "usd"
                } } }),
            ),
            (
                "total_taxes",
                serde_json::json!({ "data": { "object": {
                    "id": "in_test",
                    "customer": "cus_test",
                    "amount_paid": 4200,
                    "total_taxes": [
                        { "amount": i64::MAX },
                        { "amount": 1 }
                    ],
                    "currency": "usd"
                } } }),
            ),
            (
                "subscription",
                serde_json::json!({ "data": { "object": {
                    "id": "in_test",
                    "customer": "cus_test",
                    "subscription": 42,
                    "amount_paid": 4200,
                    "currency": "usd"
                } } }),
            ),
            (
                "paid_at",
                serde_json::json!({ "data": { "object": {
                    "id": "in_test",
                    "customer": "cus_test",
                    "amount_paid": 4200,
                    "currency": "usd",
                    "status_transitions": { "paid_at": "yesterday" }
                } } }),
            ),
        ];

        for (field, payload) in cases {
            let event = event(NeutralEventKind::InvoicePaid, payload);
            let error = p
                .try_extract_payment_snapshot(&event)
                .expect_err("invalid optional field must fail when present");
            assert!(
                matches!(error, PaymentError::Provider(ref message) if message.contains(field)),
                "error for {field} did not identify the field: {error:?}"
            );
        }
    }

    #[test]
    fn extract_payment_snapshot_invoice_paid_uses_amount_paid_and_tax() {
        let p = provider();
        let e = event(
            NeutralEventKind::InvoicePaid,
            serde_json::json!({
                "data": { "object": {
                    "id": "in_x",
                    "customer": "cus_x",
                    "subscription": "sub_x",
                    "amount_paid": 12345,
                    "amount_due": 99999,
                    "total_taxes": [
                        { "amount": 200, "taxability_reason": "standard_rated" },
                        { "amount": 34, "taxability_reason": "standard_rated" }
                    ],
                    "currency": "EUR",
                    "status_transitions": { "paid_at": 1717000000 }
                }}
            }),
        );
        let snap = p.extract_payment_snapshot(&e).expect("snapshot present");
        assert_eq!(
            snap.amount_total_minor, 12345,
            "amount_paid takes precedence"
        );
        assert_eq!(snap.amount_tax_minor, 234);
        assert_eq!(snap.currency, "EUR");
        assert_eq!(snap.provider_subscription_id.as_deref(), Some("sub_x"));
        assert!(snap.paid_at.is_some());

        for (total_taxes, expected_tax) in [(None, 91), (Some(serde_json::Value::Null), 92)] {
            let mut object = serde_json::json!({
                "id": "in_legacy_tax",
                "customer": "cus_legacy_tax",
                "amount_paid": 1000,
                "tax": expected_tax,
                "currency": "USD"
            });
            if let Some(total_taxes) = total_taxes {
                object["total_taxes"] = total_taxes;
            }
            let legacy = event(
                NeutralEventKind::InvoicePaid,
                serde_json::json!({ "data": { "object": object } }),
            );
            let snapshot = p
                .try_extract_payment_snapshot(&legacy)
                .expect("legacy scalar tax must parse")
                .expect("legacy invoice snapshot");
            assert_eq!(snapshot.amount_tax_minor, expected_tax);
        }
    }

    #[test]
    fn extract_payment_snapshot_falls_back_to_amount_due_when_amount_paid_absent() {
        let p = provider();
        let e = event(
            NeutralEventKind::InvoiceFailed,
            serde_json::json!({
                "data": { "object": {
                    "id": "in_fail",
                    "customer": "cus_y",
                    "amount_due": 5500,
                    "currency": "GBP"
                }}
            }),
        );
        let snap = p.extract_payment_snapshot(&e).expect("snapshot present");
        assert_eq!(snap.amount_total_minor, 5500);
        assert_eq!(snap.status, "failed");
    }

    #[test]
    fn extract_payment_snapshot_returns_none_for_subscription_event() {
        let p = provider();
        let e = event(
            NeutralEventKind::SubscriptionUpdated,
            serde_json::json!({
                "data": { "object": { "id": "sub_x" } }
            }),
        );
        assert!(p.extract_payment_snapshot(&e).is_none());
    }

    #[test]
    fn extract_customer_snapshot_pulls_email_from_data_object() {
        let p = provider();
        let e = event(
            NeutralEventKind::CustomerUpdated,
            serde_json::json!({
                "data": { "object": {
                    "id": "cus_email_test",
                    "email": "new@example.com",
                    "name": "Test User"
                }}
            }),
        );
        let snap = p.extract_customer_snapshot(&e).expect("snapshot present");
        assert_eq!(snap.provider_customer_id, "cus_email_test");
        assert_eq!(snap.email.as_deref(), Some("new@example.com"));
        assert_eq!(snap.provider_metadata["name"], "Test User");
    }

    #[test]
    fn extract_customer_snapshot_returns_none_for_non_customer_events() {
        let p = provider();
        let e = event(
            NeutralEventKind::PaymentSucceeded,
            serde_json::json!({"data": {"object": {"id": "pi_x", "email": "x@x.com"}}}),
        );
        assert!(p.extract_customer_snapshot(&e).is_none());
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
        // Shallow payloads exercise the early `?` guards but bail out long
        // before any value conversion - a test built only from those passes
        // even with a `.unwrap()` planted in the parsing, which is exactly
        // what the first draft of the sibling Paddle test did.
        //
        // The deep payloads are shaped like REAL Stripe events: the
        // `/data/object` wrapper, an `id`, and the `amount` / `amount_paid`
        // / `created` fields each arm actually reads. They are hostile only
        // in a leaf field's TYPE, which is both the realistic attack and the
        // realistic provider drift.
        let hostile = [
            // Shallow: must not unwind on the way to an early return.
            serde_json::json!(null),
            serde_json::json!({}),
            serde_json::json!({ "data": null }),
            serde_json::json!({ "data": [] }),
            serde_json::json!({ "data": "a string where an object belongs" }),
            serde_json::json!({ "data": { "object": null } }),
            serde_json::json!({ "data": { "object": [] } }),
            serde_json::json!({ "data": { "object": { "id": 12345 } } }),
            serde_json::json!({ "data": { "object": { "id": { "nested": "obj" } } } }),
            // Deep: reaches the conversions.
            serde_json::json!({ "data": { "object": {
                "id": "pi_1",
                "customer": 42,
                "amount": "not-a-number",
                "amount_paid": "not-a-number",
                "amount_due": [],
                "currency": 7,
                "created": "not-a-timestamp",
            } } }),
            serde_json::json!({ "data": { "object": {
                "id": "pi_2",
                "amount": {},
                "amount_paid": null,
                "amount_due": true,
                "currency": [],
                // i64::MAX seconds is far outside any representable date.
                "created": 9_223_372_036_854_775_807_i64,
            } } }),
            serde_json::json!({ "data": { "object": {
                "id": "pi_3",
                "amount": 1.5,
                "amount_paid": -1,
                "total_tax_amounts": "not-an-array",
                "created": -62_135_596_801_i64,
                "email": 0,
            } } }),
        ];

        for payload in hostile {
            for kind in ALL_EVENT_KINDS {
                let ev = event(*kind, payload.clone());
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _ = p.extract_payload_ids(&ev);
                    let _ = p.extract_payment_snapshot(&ev);
                    let _ = p.extract_customer_snapshot(&ev);
                }));
                assert!(
                    outcome.is_ok(),
                    "extracting from a {kind:?} event with payload {payload} unwound; \
                     an authenticated sender must not be able to panic the handler \
                     by changing a field's type"
                );
            }
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
