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
        // secret verbatim, so guard the boundary that actually decides trust —
        // that covers every construction path, not just the documented one.
        if self.webhook_signing_secret().trim().is_empty() {
            return Err(PaymentError::WebhookSignature(
                "stripe webhook signing secret is empty — refusing to verify against an \
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
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

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
    /// Stripe is consistent: every webhook puts the relevant entity at
    /// `data.object`, with `id` as its primary key and `customer` as the
    /// customer pointer where applicable. Invoice and PaymentIntent events
    /// also carry `subscription` when the charge is recurring.
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
                ids.customer_id = obj
                    .get("customer")
                    .and_then(|v| v.as_str())
                    .map(String::from);
            }
            Some(NeutralEventKind::CustomerCreated | NeutralEventKind::CustomerUpdated) => {
                ids.customer_id = obj.get("id").and_then(|v| v.as_str()).map(String::from);
            }
            Some(
                NeutralEventKind::PaymentSucceeded
                | NeutralEventKind::PaymentFailed
                | NeutralEventKind::PaymentRefunded
                | NeutralEventKind::PaymentDisputed,
            ) => {
                ids.transaction_id = obj.get("id").and_then(|v| v.as_str()).map(String::from);
                ids.customer_id = obj
                    .get("customer")
                    .and_then(|v| v.as_str())
                    .map(String::from);
            }
            Some(NeutralEventKind::InvoicePaid | NeutralEventKind::InvoiceFailed) => {
                ids.transaction_id = obj.get("id").and_then(|v| v.as_str()).map(String::from);
                ids.customer_id = obj
                    .get("customer")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                ids.subscription_id = obj
                    .get("subscription")
                    .and_then(|v| v.as_str())
                    .map(String::from);
            }
            None => {}
        }
        ids
    }

    /// Build a [`PaymentSnapshot`] from a Stripe payment / invoice payload.
    ///
    /// - `payment_intent.*` → uses `id`, `amount`, `currency`, `status`, `customer`
    /// - `charge.refunded` / `charge.dispute.created` → uses Charge fields
    /// - `invoice.*` → uses `id`, `amount_paid`, `tax`, `currency`, `customer`,
    ///   `subscription`, `status_transitions.paid_at`
    ///
    /// Returns `None` for subscription / customer events (those go through
    /// the `extract_payload_ids` + provider API path).
    fn extract_payment_snapshot(&self, event: &WebhookEvent) -> Option<PaymentSnapshot> {
        let obj = event.raw_payload.pointer("/data/object")?;
        let provider_transaction_id = obj.get("id")?.as_str()?.to_string();
        let provider_customer_id = obj
            .get("customer")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        match event.neutral? {
            NeutralEventKind::PaymentSucceeded
            | NeutralEventKind::PaymentFailed
            | NeutralEventKind::PaymentRefunded
            | NeutralEventKind::PaymentDisputed => {
                // PaymentIntent or Charge — both expose amount + currency at the top level.
                let amount_total_minor = obj.get("amount").and_then(|v| v.as_i64()).unwrap_or(0);
                let currency = obj
                    .get("currency")
                    .and_then(|v| v.as_str())
                    .unwrap_or("usd")
                    .to_uppercase();
                let status = match event.neutral? {
                    NeutralEventKind::PaymentSucceeded => "succeeded",
                    NeutralEventKind::PaymentFailed => "failed",
                    NeutralEventKind::PaymentRefunded => "refunded",
                    NeutralEventKind::PaymentDisputed => "disputed",
                    _ => unreachable!(),
                }
                .to_string();
                let paid_at = if matches!(event.neutral, Some(NeutralEventKind::PaymentSucceeded)) {
                    obj.get("created")
                        .and_then(|v| v.as_i64())
                        .and_then(|secs| chrono::DateTime::from_timestamp(secs, 0))
                } else {
                    None
                };
                Some(PaymentSnapshot {
                    provider_transaction_id,
                    provider_customer_id,
                    provider_subscription_id: None,
                    amount_total_minor,
                    amount_tax_minor: 0,
                    currency,
                    status,
                    paid_at,
                    provider_metadata: obj.clone(),
                })
            }
            NeutralEventKind::InvoicePaid | NeutralEventKind::InvoiceFailed => {
                let amount_total_minor = obj
                    .get("amount_paid")
                    .and_then(|v| v.as_i64())
                    .or_else(|| obj.get("amount_due").and_then(|v| v.as_i64()))
                    .unwrap_or(0);
                let amount_tax_minor = obj.get("tax").and_then(|v| v.as_i64()).unwrap_or(0);
                let currency = obj
                    .get("currency")
                    .and_then(|v| v.as_str())
                    .unwrap_or("usd")
                    .to_uppercase();
                let status = if matches!(event.neutral, Some(NeutralEventKind::InvoicePaid)) {
                    "succeeded"
                } else {
                    "failed"
                }
                .to_string();
                let provider_subscription_id = obj
                    .get("subscription")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                let paid_at = obj
                    .pointer("/status_transitions/paid_at")
                    .and_then(|v| v.as_i64())
                    .and_then(|secs| chrono::DateTime::from_timestamp(secs, 0));
                Some(PaymentSnapshot {
                    provider_transaction_id,
                    provider_customer_id,
                    provider_subscription_id,
                    amount_total_minor,
                    amount_tax_minor,
                    currency,
                    status,
                    paid_at,
                    provider_metadata: obj.clone(),
                })
            }
            _ => None,
        }
    }

    /// Build a [`CustomerSnapshot`] from Stripe `customer.created` /
    /// `customer.updated` payloads. Stripe puts the full Customer object at
    /// `data.object` — we pull `id` + `email` and keep the rest in
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

    /// Install the rustls ring CryptoProvider exactly once — `StripeProvider::new`
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
    fn verify_refuses_a_blank_signing_secret() {
        // `new` accepts the secret verbatim, so a config path that reads a
        // set-but-empty `STRIPE_WEBHOOK_SIGNING_SECRET` can still build a
        // provider whose HMAC key is empty. Verification must refuse it
        // outright rather than compare against a forgeable digest — and it
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
    // CI-06 — signature verification, positive and negative
    // ---------------------------------------------------------------
    //
    // Until these landed, `verify()` had exactly one test: the blank-secret
    // refusal. Nothing ever constructed a VALID signature, which means the
    // success path was unproven and — far worse — so was every rejection
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
            "a correctly signed payload must verify — without this the negatives prove nothing",
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
                    "tax": 234,
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

    /// CI-06 — a signature-valid event whose *body* is hostile must never
    /// panic the endpoint.
    ///
    /// Signature verification only proves the sender holds the key. It says
    /// nothing about the shape of what they sent, and a provider can change
    /// a payload shape without warning. Every extractor here reads through
    /// `and_then` / `unwrap_or` / `?` rather than indexing, so the intent is
    /// already right; nothing pinned it, so a later `[...]` or `.unwrap()`
    /// would have turned a surprising payload into a 500 — or a panic that
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
        // before any value conversion — a test built only from those passes
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
