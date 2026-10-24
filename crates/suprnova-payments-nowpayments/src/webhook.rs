use crate::{MAX_BODY_BYTES, NowPaymentsProvider};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha512;
use suprnova::payments::{
    PayloadIds, PaymentError, PaymentResult, WebhookContext, WebhookEvent, WebhookHandler,
};

impl WebhookHandler for NowPaymentsProvider {
    fn mirrors_payment_transactions(&self) -> bool {
        false
    }

    fn verify(&self, ctx: &WebhookContext<'_>) -> PaymentResult<()> {
        let invalid = || {
            PaymentError::WebhookSignature("invalid NOWPayments IPN signature or payload".into())
        };
        if ctx.body.len() > MAX_BODY_BYTES {
            return Err(invalid());
        }
        let mut values = ctx.headers.get_all("x-nowpayments-sig").iter();
        let signature = values
            .next()
            .and_then(|value| value.to_str().ok())
            .ok_or_else(invalid)?;
        if values.next().is_some() || signature.len() != 128 {
            return Err(invalid());
        }
        let mut decoded = [0u8; 64];
        for (slot, pair) in decoded
            .iter_mut()
            .zip(signature.as_bytes().as_chunks::<2>().0)
        {
            let nibble = |byte: u8| (byte as char).to_digit(16).map(|value| value as u8);
            *slot =
                nibble(pair[0]).ok_or_else(invalid)? * 16 + nibble(pair[1]).ok_or_else(invalid)?;
        }
        let value = crate::json::parse(ctx.body).map_err(|_| invalid())?;
        if !value.is_object() {
            return Err(invalid());
        }
        let canonical = crate::json::canonical(&value).map_err(|_| invalid())?;
        let mut mac =
            Hmac::<Sha512>::new_from_slice(self.ipn_secret.as_bytes()).map_err(|_| invalid())?;
        mac.update(&canonical);
        mac.verify_slice(&decoded).map_err(|_| invalid())
    }

    fn parse_event(&self, body: &[u8]) -> PaymentResult<WebhookEvent> {
        if body.len() > MAX_BODY_BYTES {
            return Err(PaymentError::Validation(
                "NOWPayments IPN exceeds 64 KiB".into(),
            ));
        }
        let raw = crate::json::parse(body)
            .map_err(|_| PaymentError::Validation("NOWPayments IPN is invalid JSON".into()))?;
        let payment = crate::status::parse_payment(raw, body)?;
        let status = crate::status::status_name(&payment.raw)?;
        // IPN has no separate event ID. One receipt per payment/status makes
        // retries stable even when updated_at changes. Reconciliation reads
        // current provider state; a late pending IPN cannot undo settlement.
        Ok(WebhookEvent {
            provider: "nowpayments".into(),
            provider_event_id: format!("{}:{status}", payment.payment_id),
            provider_event_type: format!("payment.{status}"),
            neutral: payment.status.neutral_event(),
            raw_payload: payment.raw,
        })
    }

    fn extract_payload_ids(&self, event: &WebhookEvent) -> PayloadIds {
        PayloadIds {
            transaction_id: crate::status::identifier(
                event.raw_payload.get("payment_id"),
                "payment_id",
            )
            .ok(),
            ..PayloadIds::default()
        }
    }
}
