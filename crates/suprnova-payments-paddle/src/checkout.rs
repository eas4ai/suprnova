//! Implementation of the `Checkout` trait for `PaddleProvider`.
//!
//! Paddle is checkout-driven: both `OneOff` and `Subscription` modes route
//! through `transaction_create`. Paddle dispatches on price-kind implicitly
//! (recurring prices → subscription, one-off prices → single charge). The
//! returned `transaction_id` is opened by the frontend via paddle.js with
//! the `client_token`.

use std::time::Duration;

use async_trait::async_trait;
use paddle_rust_sdk::enums::TransactionStatus;
use suprnova::payments::{
    Checkout, CheckoutSessionState, Currency, Money, PaymentError, PaymentResult, SessionPayload,
    StartSessionRequest,
};

use crate::PaddleProvider;

// The SDK constructs a reqwest client without a request deadline.
const TRANSACTION_TIMEOUT: Duration = Duration::from_secs(30);

#[async_trait]
impl Checkout for PaddleProvider {
    async fn start_session(&self, req: StartSessionRequest) -> PaymentResult<SessionPayload> {
        if req.price_refs.is_empty() {
            return Err(PaymentError::Validation(
                "start_session requires at least one price_ref".into(),
            ));
        }
        crate::reject_unsupported_idempotency_key(
            req.idempotency_key.as_deref(),
            "transaction creation",
        )?;

        let mut builder = self.client().transaction_create();
        for price_ref in &req.price_refs {
            builder.append_catalog_item(price_ref.clone(), 1);
        }
        builder.customer_id(req.customer_ref.clone());
        if let Some(metadata) = crate::customer::metadata_to_string_map(req.metadata.as_ref()) {
            builder.custom_data(metadata);
        }

        let resp = tokio::time::timeout(TRANSACTION_TIMEOUT, builder.send())
            .await
            .map_err(|_| PaymentError::Provider(
                "paddle transaction_create timed out; outcome is unknown, reconcile before retrying".into(),
            ))?
            .map_err(|e| PaymentError::Provider(format!("paddle transaction_create: {e}")))?;

        Ok(SessionPayload::PaddleInline {
            transaction_id: resp.data.id.to_string(),
            // The transaction already identifies the customer. A ctm_ ID is
            // not the pca_ token accepted by Paddle.js customerAuthToken.
            customer_token: None,
            client_token: self.client_token().to_string(),
        })
    }

    async fn session_status(
        &self,
        provider_session_id: &str,
    ) -> PaymentResult<CheckoutSessionState> {
        if !provider_session_id
            .strip_prefix("txn_")
            .is_some_and(|suffix| {
                !suffix.is_empty()
                    && suffix
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            })
        {
            return Err(PaymentError::Validation(
                "Paddle session ID must be a transaction identifier".into(),
            ));
        }
        let transaction = tokio::time::timeout(
            TRANSACTION_TIMEOUT,
            self.client().transaction_get(provider_session_id).send(),
        )
        .await
        .map_err(|_| PaymentError::Provider("paddle transaction_get timed out".into()))?
        .map_err(|e| PaymentError::Provider(format!("paddle transaction_get: {e}")))?
        .data;
        match transaction.status {
            TransactionStatus::Draft
            | TransactionStatus::Ready
            | TransactionStatus::Billed
            | TransactionStatus::PastDue => Ok(CheckoutSessionState::Open),
            TransactionStatus::Canceled => Ok(CheckoutSessionState::Expired),
            TransactionStatus::Paid | TransactionStatus::Completed => {
                let amount = transaction.details.totals.total.parse::<i64>().map_err(|_| {
                    PaymentError::Provider("paddle transaction_get: details.totals.total must contain integer minor units".into())
                })?;
                let code = serde_json::to_value(transaction.currency_code).map_err(|e| {
                    PaymentError::Provider(format!("paddle transaction_get currency: {e}"))
                })?;
                let currency = code.as_str().and_then(Currency::from_code).ok_or_else(|| {
                    PaymentError::Provider(format!(
                        "paddle transaction_get: unknown currency {code}"
                    ))
                })?;
                Ok(CheckoutSessionState::Complete {
                    paid: true,
                    payment_ref: Some(transaction.id.to_string()),
                    amount_total: Some(Money::from_minor_units(amount, currency)),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests;
