//! In-memory mock payment provider for tests.
//!
//! [`MockPaymentProvider`] implements all four universal traits — [`Checkout`],
//! [`Subscription`], [`CustomerStore`], and [`WebhookHandler`] — entirely in memory,
//! with no external dependencies. It deliberately does NOT implement [`Payment`](super::traits::Payment)
//! (server-capture) to exercise the Paddle-style "optional Payment" invariant: callers
//! that query `provider.as_payment()` will receive `None`.
//!
//! # Usage
//!
//! ```rust,no_run
//! use suprnova::payments::*;
//!
//! # async fn ex() -> Result<(), Box<dyn std::error::Error>> {
//! let provider = MockPaymentProvider::new();
//! let cus = provider.create_customer(CreateCustomerRequest {
//!     user_id: "user_1".into(),
//!     email: "test@example.com".into(),
//!     name: None,
//!     metadata: None,
//! }).await.unwrap();
//! # Ok(()) }
//! ```
//!
//! See `framework/tests/payments_mock_discriminator.rs` for the full E2E flow.

use crate::payments::{
    Checkout, CheckoutSessionState, CreateCustomerRequest, CreatePromotionCodeRequest, CustomerRef,
    CustomerSnapshot, CustomerStore, NeutralEventKind, PayloadIds, PaymentError, PaymentProvider,
    PaymentResult, PaymentSnapshot, PromotionCode, Promotions, SessionPayload, StartSessionRequest,
    SubscribeRequest, Subscription, SubscriptionItemSnapshot, SubscriptionResult,
    SubscriptionStatus, UpdateCustomerRequest, UpdateSubscriptionRequest, WebhookContext,
    WebhookEvent, WebhookHandler,
};
use async_trait::async_trait;
use chrono::Utc;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// In-memory implementation of all four universal payment traits.
///
/// Thread-safe: internal state is wrapped in `Arc<tokio::sync::RwLock<_>>`.
/// Instances can be cloned or wrapped in `Arc` and shared across async tasks.
///
/// Uses tokio's async `RwLock` rather than `std::sync::RwLock` so that the lock
/// surface is poison-immune (Domain 19 audit D19-B): a panic inside one trait
/// method can no longer poison the shared store and cascade `PaymentError`s
/// through every subsequent call.
#[derive(Default)]
pub struct MockPaymentProvider {
    customers: Arc<RwLock<HashMap<String, CustomerRef>>>,
    subscriptions: Arc<RwLock<HashMap<String, SubscriptionResult>>>,
    /// Every `start_session` request, in call order — lets tests assert on
    /// mode/price_refs/URLs without intercepting the wire.
    sessions: Arc<RwLock<Vec<(String, StartSessionRequest)>>>,
    /// Scripted `session_status` results, keyed by provider session id.
    session_states: Arc<RwLock<HashMap<String, CheckoutSessionState>>>,
    /// Every `create_promotion_code` request, in call order.
    promotion_requests: Arc<RwLock<Vec<CreatePromotionCodeRequest>>>,
    sequence: Arc<RwLock<u64>>,
}

impl MockPaymentProvider {
    /// Create a new, empty mock provider.
    pub fn new() -> Self {
        Self::default()
    }

    async fn next_id(&self, prefix: &str) -> String {
        let mut seq = self.sequence.write().await;
        *seq += 1;
        format!("{prefix}_mock_{}", *seq)
    }

    /// Script the [`CheckoutSessionState`] that `session_status` reports for
    /// `provider_session_id`. Unscripted-but-known sessions report
    /// [`CheckoutSessionState::Open`]; unknown ids report
    /// [`PaymentError::NotFound`].
    pub async fn script_session_status(
        &self,
        provider_session_id: impl Into<String>,
        state: CheckoutSessionState,
    ) {
        self.session_states
            .write()
            .await
            .insert(provider_session_id.into(), state);
    }

    /// Recorded `start_session` calls as `(provider_session_id, request)`
    /// pairs, in call order.
    pub async fn recorded_sessions(&self) -> Vec<(String, StartSessionRequest)> {
        self.sessions.read().await.clone()
    }

    /// Recorded `create_promotion_code` requests, in call order.
    pub async fn recorded_promotion_requests(&self) -> Vec<CreatePromotionCodeRequest> {
        self.promotion_requests.read().await.clone()
    }
}

impl PaymentProvider for MockPaymentProvider {
    fn name(&self) -> &'static str {
        "mock"
    }
    // `as_payment()` intentionally uses the default `None` — no `Payment` impl.

    /// The mock mints promotion codes so app tests can drive upsell flows.
    fn as_promotions(&self) -> Option<&dyn Promotions> {
        Some(self)
    }
}

#[async_trait]
impl Checkout for MockPaymentProvider {
    async fn start_session(&self, req: StartSessionRequest) -> PaymentResult<SessionPayload> {
        let session_id = self.next_id("ses").await;
        let url = format!("https://mock.example/{}/{}", req.customer_ref, session_id);
        self.sessions.write().await.push((session_id.clone(), req));
        Ok(SessionPayload::Redirect {
            url,
            provider_session_id: session_id,
        })
    }

    async fn session_status(
        &self,
        provider_session_id: &str,
    ) -> PaymentResult<CheckoutSessionState> {
        if let Some(state) = self.session_states.read().await.get(provider_session_id) {
            return Ok(state.clone());
        }
        let known = self
            .sessions
            .read()
            .await
            .iter()
            .any(|(id, _)| id == provider_session_id);
        if known {
            Ok(CheckoutSessionState::Open)
        } else {
            Err(PaymentError::NotFound(format!(
                "mock session {provider_session_id}"
            )))
        }
    }
}

#[async_trait]
impl Promotions for MockPaymentProvider {
    async fn create_promotion_code(
        &self,
        req: CreatePromotionCodeRequest,
    ) -> PaymentResult<PromotionCode> {
        let id = self.next_id("promo").await;
        self.promotion_requests.write().await.push(req);
        Ok(PromotionCode {
            code: id.to_uppercase(),
            provider_promotion_id: id,
        })
    }
}

#[async_trait]
impl Subscription for MockPaymentProvider {
    async fn subscribe(&self, req: SubscribeRequest) -> PaymentResult<SubscriptionResult> {
        let id = self.next_id("sub").await;
        let now = Utc::now();
        let result = SubscriptionResult {
            provider_subscription_id: id.clone(),
            provider_customer_id: req.customer_ref.clone(),
            status: SubscriptionStatus::Active,
            items: req
                .price_refs
                .iter()
                .map(|price_ref| SubscriptionItemSnapshot {
                    provider_item_id: format!("{id}_item_{price_ref}"),
                    provider_price_id: price_ref.clone(),
                    quantity: 1,
                    unit_amount: None,
                })
                .collect(),
            current_period_start: now,
            current_period_end: now + chrono::Duration::days(30),
            cancel_at_period_end: false,
            provider_metadata: json!({}),
        };
        self.subscriptions.write().await.insert(id, result.clone());
        Ok(result)
    }

    async fn update(&self, req: UpdateSubscriptionRequest) -> PaymentResult<SubscriptionResult> {
        let mut store = self.subscriptions.write().await;
        let sub = store
            .get_mut(&req.provider_subscription_id)
            .ok_or_else(|| PaymentError::NotFound(req.provider_subscription_id.clone()))?;
        if let Some(c) = req.cancel_at_period_end {
            sub.cancel_at_period_end = c;
        }
        if let Some(prices) = req.new_price_refs {
            let sub_id = sub.provider_subscription_id.clone();
            sub.items = prices
                .iter()
                .map(|price_ref| SubscriptionItemSnapshot {
                    provider_item_id: format!("{sub_id}_item_{price_ref}"),
                    provider_price_id: price_ref.clone(),
                    quantity: 1,
                    unit_amount: None,
                })
                .collect();
        }
        Ok(sub.clone())
    }

    async fn cancel(
        &self,
        provider_subscription_id: &str,
        at_period_end: bool,
    ) -> PaymentResult<SubscriptionResult> {
        let mut store = self.subscriptions.write().await;
        let sub = store
            .get_mut(provider_subscription_id)
            .ok_or_else(|| PaymentError::NotFound(provider_subscription_id.to_string()))?;
        if at_period_end {
            sub.cancel_at_period_end = true;
        } else {
            sub.status = SubscriptionStatus::Canceled;
        }
        Ok(sub.clone())
    }

    async fn get(&self, provider_subscription_id: &str) -> PaymentResult<SubscriptionResult> {
        self.subscriptions
            .read()
            .await
            .get(provider_subscription_id)
            .cloned()
            .ok_or_else(|| PaymentError::NotFound(provider_subscription_id.to_string()))
    }
}

#[async_trait]
impl CustomerStore for MockPaymentProvider {
    async fn create_customer(&self, req: CreateCustomerRequest) -> PaymentResult<CustomerRef> {
        let id = self.next_id("cus").await;
        let cr = CustomerRef {
            provider_customer_id: id.clone(),
            user_id: Some(req.user_id),
            email: req.email,
            provider_metadata: req.metadata.unwrap_or(json!({})),
        };
        self.customers.write().await.insert(id, cr.clone());
        Ok(cr)
    }

    async fn update_customer(&self, req: UpdateCustomerRequest) -> PaymentResult<CustomerRef> {
        let mut store = self.customers.write().await;
        let cr = store
            .get_mut(&req.provider_customer_id)
            .ok_or_else(|| PaymentError::NotFound(req.provider_customer_id.clone()))?;
        if let Some(e) = req.email {
            cr.email = e;
        }
        if let Some(m) = req.metadata {
            cr.provider_metadata = m;
        }
        Ok(cr.clone())
    }

    async fn get_customer(&self, provider_customer_id: &str) -> PaymentResult<CustomerRef> {
        self.customers
            .read()
            .await
            .get(provider_customer_id)
            .cloned()
            .ok_or_else(|| PaymentError::NotFound(provider_customer_id.to_string()))
    }

    async fn delete_customer(&self, provider_customer_id: &str) -> PaymentResult<()> {
        self.customers
            .write()
            .await
            .remove(provider_customer_id)
            .map(|_| ())
            .ok_or_else(|| PaymentError::NotFound(provider_customer_id.to_string()))
    }
}

#[async_trait]
impl WebhookHandler for MockPaymentProvider {
    /// The mock signs nothing — it accepts every webhook so local dev and
    /// the test bed can exercise the ingress path without a real provider
    /// secret. That makes it a no-op verifier, so it must never run as a
    /// registered provider outside development: a forged
    /// `POST /webhooks/payments/mock` would otherwise hydrate mirror rows.
    ///
    /// This mirrors the framework's APP_KEY fail-closed contract
    /// (`crate::crypto::resolve_boot_keyring`): permissive in
    /// `local` / `development` / `testing`, hard-rejecting in any other
    /// `APP_ENV` (production, staging, or a custom environment).
    fn verify(&self, _ctx: &WebhookContext<'_>) -> PaymentResult<()> {
        use crate::config::Environment;
        match Environment::detect() {
            Environment::Local | Environment::Development | Environment::Testing => Ok(()),
            env => Err(PaymentError::WebhookSignature(format!(
                "MockPaymentProvider accepts every webhook unverified and refuses \
                 to run outside a development environment (APP_ENV={env}). Register \
                 a real provider with signature verification in production/staging."
            ))),
        }
    }

    fn parse_event(&self, body: &[u8]) -> PaymentResult<WebhookEvent> {
        let raw: serde_json::Value = serde_json::from_slice(body)
            .map_err(|e| PaymentError::Validation(format!("invalid mock webhook body: {e}")))?;
        let provider_event_type = raw
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("mock.event")
            .to_string();
        let neutral = match provider_event_type.as_str() {
            "subscription.created" => Some(NeutralEventKind::SubscriptionCreated),
            "subscription.updated" => Some(NeutralEventKind::SubscriptionUpdated),
            "subscription.canceled" => Some(NeutralEventKind::SubscriptionCanceled),
            "payment.succeeded" => Some(NeutralEventKind::PaymentSucceeded),
            "payment.failed" => Some(NeutralEventKind::PaymentFailed),
            "payment.refunded" => Some(NeutralEventKind::PaymentRefunded),
            "payment.disputed" => Some(NeutralEventKind::PaymentDisputed),
            "invoice.paid" => Some(NeutralEventKind::InvoicePaid),
            "invoice.failed" => Some(NeutralEventKind::InvoiceFailed),
            "customer.created" => Some(NeutralEventKind::CustomerCreated),
            "customer.updated" => Some(NeutralEventKind::CustomerUpdated),
            _ => None,
        };
        Ok(WebhookEvent {
            provider: "mock".into(),
            provider_event_id: raw
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("evt_mock")
                .to_string(),
            provider_event_type,
            neutral,
            raw_payload: raw,
        })
    }

    /// Mock follows Stripe's envelope convention: `data.object.*` carries the
    /// entity, with `id`, `customer`, and `subscription` as canonical pointers.
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
                | NeutralEventKind::PaymentDisputed
                | NeutralEventKind::InvoicePaid
                | NeutralEventKind::InvoiceFailed,
            ) => {
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

    fn extract_payment_snapshot(&self, event: &WebhookEvent) -> Option<PaymentSnapshot> {
        let obj = event.raw_payload.pointer("/data/object")?;
        let provider_transaction_id = obj.get("id")?.as_str()?.to_string();
        let provider_customer_id = obj
            .get("customer")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let provider_subscription_id = obj
            .get("subscription")
            .and_then(|v| v.as_str())
            .map(String::from);
        let amount_total_minor = obj.get("amount").and_then(|v| v.as_i64()).unwrap_or(0);
        let amount_tax_minor = obj.get("tax").and_then(|v| v.as_i64()).unwrap_or(0);
        let currency = obj
            .get("currency")
            .and_then(|v| v.as_str())
            .unwrap_or("USD")
            .to_uppercase();
        let status = match event.neutral? {
            NeutralEventKind::PaymentSucceeded | NeutralEventKind::InvoicePaid => "succeeded",
            NeutralEventKind::PaymentFailed | NeutralEventKind::InvoiceFailed => "failed",
            NeutralEventKind::PaymentRefunded => "refunded",
            NeutralEventKind::PaymentDisputed => "disputed",
            _ => return None,
        }
        .to_string();
        let paid_at = obj
            .get("paid_at")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc));
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

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderMap;

    /// Save `APP_ENV`, set it to `value`, run `f`, then restore. Callers
    /// must be `#[serial]` because `APP_ENV` is process-global.
    fn with_app_env(value: Option<&str>, f: impl FnOnce()) {
        let prior = std::env::var("APP_ENV").ok();
        // SAFETY: mutates a process-global env var; callers serialize on
        // the shared `app_config_env` key so this never races a sibling.
        unsafe {
            match value {
                Some(v) => std::env::set_var("APP_ENV", v),
                None => std::env::remove_var("APP_ENV"),
            }
        }
        f();
        unsafe {
            match prior {
                Some(v) => std::env::set_var("APP_ENV", v),
                None => std::env::remove_var("APP_ENV"),
            }
        }
    }

    fn empty_ctx<'a>(headers: &'a HeaderMap, body: &'a [u8]) -> WebhookContext<'a> {
        WebhookContext {
            body,
            headers,
            remote_addr: None,
        }
    }

    #[test]
    #[serial_test::serial(app_config_env)]
    fn verify_accepts_in_development_environments() {
        let provider = MockPaymentProvider::new();
        let headers = HeaderMap::new();
        for env in ["local", "development", "dev", "testing", "test"] {
            with_app_env(Some(env), || {
                let ctx = empty_ctx(&headers, b"{}");
                assert!(
                    provider.verify(&ctx).is_ok(),
                    "mock verify should accept in APP_ENV={env}"
                );
            });
        }
        // Unset APP_ENV defaults to Local — also permissive.
        with_app_env(None, || {
            let ctx = empty_ctx(&headers, b"{}");
            assert!(provider.verify(&ctx).is_ok());
        });
    }

    #[test]
    #[serial_test::serial(app_config_env)]
    fn verify_fails_closed_outside_development() {
        let provider = MockPaymentProvider::new();
        let headers = HeaderMap::new();
        for env in ["production", "prod", "staging", "stage", "k8s-prod"] {
            with_app_env(Some(env), || {
                let ctx = empty_ctx(&headers, b"{}");
                let err = provider
                    .verify(&ctx)
                    .expect_err("mock verify must reject outside development");
                assert!(
                    matches!(err, PaymentError::WebhookSignature(_)),
                    "expected WebhookSignature in APP_ENV={env}, got {err:?}"
                );
            });
        }
    }

    fn one_off_request(price_ref: &str) -> StartSessionRequest {
        StartSessionRequest {
            mode: crate::payments::SessionMode::OneOff,
            customer_ref: "cus_mock_1".into(),
            price_refs: vec![price_ref.into()],
            success_return_url: "https://app.example/return".into(),
            cancel_return_url: "https://app.example/cancel".into(),
            amount_hint: None,
            idempotency_key: None,
            metadata: None,
        }
    }

    #[tokio::test]
    async fn start_session_records_request_and_session_status_defaults_open() {
        let provider = MockPaymentProvider::new();
        let payload = provider
            .start_session(one_off_request("price_1"))
            .await
            .unwrap();
        let SessionPayload::Redirect {
            provider_session_id,
            ..
        } = payload
        else {
            panic!("mock must return Redirect payload");
        };

        let recorded = provider.recorded_sessions().await;
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].0, provider_session_id);
        assert_eq!(recorded[0].1.price_refs, vec!["price_1".to_string()]);

        // Known-but-unscripted session reports Open.
        assert_eq!(
            provider.session_status(&provider_session_id).await.unwrap(),
            CheckoutSessionState::Open
        );
    }

    #[tokio::test]
    async fn session_status_returns_scripted_state_and_not_found_for_unknown() {
        let provider = MockPaymentProvider::new();
        let SessionPayload::Redirect {
            provider_session_id,
            ..
        } = provider
            .start_session(one_off_request("price_1"))
            .await
            .unwrap()
        else {
            panic!("mock must return Redirect payload");
        };

        let complete = CheckoutSessionState::Complete {
            paid: true,
            payment_ref: Some("pi_mock_1".into()),
            amount_total: None,
        };
        provider
            .script_session_status(provider_session_id.clone(), complete.clone())
            .await;
        assert_eq!(
            provider.session_status(&provider_session_id).await.unwrap(),
            complete
        );

        let err = provider
            .session_status("ses_never_started")
            .await
            .unwrap_err();
        assert!(matches!(err, PaymentError::NotFound(_)));
    }

    #[tokio::test]
    async fn promotions_mints_codes_and_records_requests() {
        let provider = MockPaymentProvider::new();
        let promotions = provider
            .as_promotions()
            .expect("mock advertises the Promotions capability");

        let req = CreatePromotionCodeRequest {
            coupon_ref: "coupon_15".into(),
            customer_ref: "cus_mock_1".into(),
            expires_at: Some(Utc::now() + chrono::Duration::days(7)),
            max_redemptions: Some(1),
        };
        let minted = promotions.create_promotion_code(req.clone()).await.unwrap();
        assert!(!minted.code.is_empty());
        assert!(!minted.provider_promotion_id.is_empty());

        let recorded = provider.recorded_promotion_requests().await;
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].coupon_ref, "coupon_15");
        assert_eq!(recorded[0].max_redemptions, Some(1));
    }

    #[tokio::test]
    async fn default_session_status_is_not_supported() {
        /// A provider that only implements `start_session` — exercises the
        /// trait's default `session_status` body.
        struct MinimalCheckout;

        #[async_trait]
        impl Checkout for MinimalCheckout {
            async fn start_session(
                &self,
                _req: StartSessionRequest,
            ) -> PaymentResult<SessionPayload> {
                unimplemented!("not exercised")
            }
        }

        let err = MinimalCheckout
            .session_status("ses_x")
            .await
            .expect_err("default impl must report NotSupported");
        assert!(matches!(err, PaymentError::NotSupported(_)));
    }
}
