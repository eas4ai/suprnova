use crate::{NowPaymentsProvider, https_url};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use suprnova::payments::{
    Checkout, CheckoutSessionState, Money, PaymentError, PaymentResult, SessionMode,
    SessionPayload, StartSessionRequest,
};

/// Result of creating a hosted invoice. Its ID cannot be used as a payment ID.
#[derive(Debug, Clone, PartialEq)]
pub struct NowPaymentsInvoice {
    /// Provider invoice ID; store alongside the merchant order before redirecting.
    pub invoice_id: String,
    /// Validated HTTPS checkout destination for the configured environment.
    pub invoice_url: String,
    /// Merchant-supplied order reference.
    pub order_id: String,
}

/// Separates a known rejection from an invoice that may already exist remotely.
#[derive(Debug, thiserror::Error)]
pub enum InvoiceCreationError {
    /// Validation or a definite provider rejection; no successful invoice is known.
    #[error("invoice rejected: {0}")]
    Rejected(#[source] PaymentError),
    /// Timeout, server error or malformed success response; do not blindly retry.
    #[error("invoice creation outcome unknown; reconcile the merchant order before retrying: {0}")]
    Unknown(#[source] PaymentError),
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct InvoiceOptions {
    order_id: String,
    order_description: Option<String>,
    pay_currency: Option<String>,
    is_fixed_rate: Option<bool>,
    is_fee_paid_by_user: Option<bool>,
}

#[derive(Serialize)]
struct InvoiceRequest {
    price_amount: String,
    price_currency: String,
    order_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    order_description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pay_currency: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    is_fixed_rate: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    is_fee_paid_by_user: Option<bool>,
    ipn_callback_url: String,
    success_url: String,
    cancel_url: String,
}

impl NowPaymentsProvider {
    /// Create an invoice with an explicit ambiguous-outcome error.
    ///
    /// Use `OneOff`, an empty `customer_ref` and `price_refs`, a positive
    /// `amount_hint`, and metadata containing `order_id`. Optional metadata
    /// keys are `order_description`, `pay_currency`, `is_fixed_rate` and
    /// `is_fee_paid_by_user`. Other keys and any idempotency key are rejected.
    /// The caller owns order persistence and any fulfillment decision.
    pub async fn create_invoice(
        &self,
        request: StartSessionRequest,
    ) -> Result<NowPaymentsInvoice, InvoiceCreationError> {
        let request = self
            .invoice_request(request)
            .map_err(InvoiceCreationError::Rejected)?;
        let url = self.api_url.join("invoice").map_err(|_| {
            InvoiceCreationError::Rejected(PaymentError::Internal(
                "invalid NOWPayments invoice endpoint".into(),
            ))
        })?;
        let response = self
            .send_json(self.client.post(url).json(&request))
            .await
            .map_err(|error| match error {
                PaymentError::Validation(_)
                | PaymentError::Authentication(_)
                | PaymentError::NotFound(_) => InvoiceCreationError::Rejected(error),
                _ => InvoiceCreationError::Unknown(error),
            })?;
        self.invoice_response(response.value, &request.order_id)
            .map_err(InvoiceCreationError::Unknown)
    }

    fn invoice_request(&self, request: StartSessionRequest) -> PaymentResult<InvoiceRequest> {
        if request.mode != SessionMode::OneOff
            || !request.customer_ref.is_empty()
            || !request.price_refs.is_empty()
        {
            return Err(PaymentError::NotSupported(
                "NOWPayments invoices require OneOff, empty customer_ref and empty price_refs"
                    .into(),
            ));
        }
        if request.idempotency_key.is_some() {
            return Err(PaymentError::NotSupported(
                "NOWPayments invoice creation has no provider idempotency-key contract".into(),
            ));
        }
        let amount: Money = request
            .amount_hint
            .ok_or_else(|| PaymentError::Validation("amount_hint is required".into()))?;
        if amount.minor_units() <= 0 || amount.currency().exponent().is_none() {
            return Err(PaymentError::Validation(
                "amount_hint must be positive with a defined fiat currency exponent".into(),
            ));
        }
        let options: InvoiceOptions = serde_json::from_value(request.metadata.unwrap_or(Value::Null))
            .map_err(|_| PaymentError::Validation("metadata requires order_id and supports only documented NOWPayments invoice options".into()))?;
        validate_text(&options.order_id, 128, "order_id")?;
        if let Some(description) = &options.order_description {
            validate_text(description, 500, "order_description")?;
        }
        if let Some(currency) = &options.pay_currency
            && (currency.is_empty()
                || currency.len() > 32
                || !currency
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()))
        {
            return Err(PaymentError::Validation(
                "pay_currency must be a lowercase provider currency ticker".into(),
            ));
        }
        let success_url = self.return_url(&request.success_return_url, "success_return_url")?;
        let cancel_url = self.return_url(&request.cancel_return_url, "cancel_return_url")?;
        Ok(InvoiceRequest {
            price_amount: amount.as_decimal().to_string(),
            price_currency: amount.currency().code().to_ascii_lowercase(),
            order_id: options.order_id,
            order_description: options.order_description,
            pay_currency: options.pay_currency,
            is_fixed_rate: options.is_fixed_rate,
            is_fee_paid_by_user: options.is_fee_paid_by_user,
            ipn_callback_url: self.callback_url.to_string(),
            success_url,
            cancel_url,
        })
    }

    fn return_url(&self, raw: &str, name: &str) -> PaymentResult<String> {
        let url = https_url(raw, name)?;
        if url.origin() != self.callback_url.origin() {
            return Err(PaymentError::Validation(format!(
                "{name} must use the configured callback origin"
            )));
        }
        Ok(url.to_string())
    }

    fn invoice_response(
        &self,
        response: Value,
        order_id: &str,
    ) -> PaymentResult<NowPaymentsInvoice> {
        let invalid = || {
            PaymentError::Provider("NOWPayments returned an invalid or mismatched invoice".into())
        };
        let invoice_id = crate::status::identifier(response.get("id"), "invoice id")?;
        if response.get("order_id").and_then(Value::as_str) != Some(order_id) {
            return Err(invalid());
        }
        let raw_url = response
            .get("invoice_url")
            .and_then(Value::as_str)
            .ok_or_else(invalid)?;
        let url = https_url(raw_url, "invoice_url").map_err(|_| invalid())?;
        let ids: Vec<_> = url
            .query_pairs()
            .filter(|(key, _)| key == "iid")
            .map(|(_, value)| value.into_owned())
            .collect();
        if url.host_str() != Some(self.environment.checkout_host())
            || url.port_or_known_default() != Some(443)
            || url.path() != "/payment/"
            || ids != [invoice_id.clone()]
        {
            return Err(invalid());
        }
        Ok(NowPaymentsInvoice {
            invoice_id,
            invoice_url: url.to_string(),
            order_id: order_id.into(),
        })
    }
}

fn validate_text(value: &str, max: usize, field: &str) -> PaymentResult<()> {
    if value.trim().is_empty() || value.len() > max || value.chars().any(char::is_control) {
        return Err(PaymentError::Validation(format!(
            "{field} must be nonblank, at most {max} bytes, and contain no control characters"
        )));
    }
    Ok(())
}

#[async_trait]
impl Checkout for NowPaymentsProvider {
    async fn start_session(&self, request: StartSessionRequest) -> PaymentResult<SessionPayload> {
        match self.create_invoice(request).await {
            Ok(invoice) => Ok(SessionPayload::Redirect {
                url: invoice.invoice_url,
                provider_session_id: invoice.invoice_id,
            }),
            Err(InvoiceCreationError::Rejected(error)) => Err(error),
            Err(error @ InvoiceCreationError::Unknown(_)) => {
                Err(PaymentError::Provider(error.to_string()))
            }
        }
    }

    async fn session_status(&self, _: &str) -> PaymentResult<CheckoutSessionState> {
        Err(PaymentError::NotSupported("NOWPayments invoice IDs are not payment IDs; use payment_status with a verified IPN payment_id".into()))
    }
}
