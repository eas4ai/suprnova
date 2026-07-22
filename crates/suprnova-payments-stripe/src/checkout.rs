//! Implementation of the `Checkout` trait for `StripeProvider`.
//!
//! For `SessionMode::OneOff` there are two paths:
//! - Non-empty `price_refs` → a hosted Checkout Session (`mode=payment`)
//!   built from predefined Prices; returns `StripeCheckoutRedirect` so the
//!   frontend issues a top-level navigation. Promotion codes are accepted
//!   at Stripe's page, and the provider's Managed Payments flag rides
//!   along when configured (see `StripeProvider::with_managed_payments`).
//! - Empty `price_refs` with `amount_hint` → a PaymentIntent; returns its
//!   `client_secret` + publishable key so the frontend can mount Stripe
//!   Elements.
//!
//! For `SessionMode::Subscription` we create a hosted Checkout Session and
//! return its url so the frontend redirects.
//!
//! `session_status` retrieves a Checkout Session and maps it to the
//! provider-neutral `CheckoutSessionState` — the verification primitive
//! return pages and reconciliation sweeps run against.

use async_trait::async_trait;
use serde::Serialize;
use stripe_client_core::{RequestBuilder, StripeMethod};
use stripe_shared::{
    CheckoutSession, CheckoutSessionPaymentStatus, CheckoutSessionStatus, PaymentIntent,
};

use suprnova::payments::{
    Checkout, CheckoutSessionState, PaymentError, PaymentResult, SessionMode, SessionPayload,
    StartSessionRequest,
};

use crate::StripeProvider;
use crate::payment::stripe_currency_to_money;

#[derive(Serialize)]
struct CreatePaymentIntentParams<'a> {
    amount: i64,
    currency: &'a str,
    customer: &'a str,
    #[serde(rename = "automatic_payment_methods[enabled]")]
    automatic_payment_methods_enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'a str>,
}

#[derive(Serialize)]
struct CreateCheckoutSessionParams<'a> {
    mode: &'static str,
    customer: &'a str,
    success_url: &'a str,
    cancel_url: &'a str,
    /// Stripe expects line_items as form-encoded array fields, e.g.
    /// `line_items[0][price]=price_xxx&line_items[0][quantity]=1`. We
    /// serialize a single line-item per price_ref via a custom serializer.
    #[serde(serialize_with = "serialize_line_items")]
    line_items: Vec<LineItem<'a>>,
    /// Let the customer enter promotion codes on Stripe's hosted page.
    /// Omitted (Stripe default: false) when not set.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    allow_promotion_codes: bool,
    /// Stripe Managed Payments (merchant-of-record) flag. Omitted entirely
    /// when off — accounts not enrolled in the program reject the field.
    #[serde(
        rename = "managed_payments[enabled]",
        skip_serializing_if = "std::ops::Not::not"
    )]
    managed_payments_enabled: bool,
}

#[derive(Serialize)]
struct LineItem<'a> {
    price: &'a str,
    quantity: u32,
}

fn serialize_line_items<S>(items: &[LineItem<'_>], s: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    // Stripe wire format for arrays: line_items[0][price]=...&line_items[0][quantity]=...
    // serde_urlencoded handles this naturally via tuple-serialization, but the array-index
    // syntax requires a flat string. We emit a single comma-separated price list as
    // line_items[0][price] for v1 — most subscriptions are single-price.
    use serde::ser::SerializeSeq;
    let mut seq = s.serialize_seq(Some(items.len()))?;
    for item in items {
        seq.serialize_element(item)?;
    }
    seq.end()
}

/// Map a retrieved Checkout Session onto the provider-neutral state enum.
fn session_to_state(session: CheckoutSession) -> PaymentResult<CheckoutSessionState> {
    let status = session.status.ok_or_else(|| {
        PaymentError::Provider("CheckoutSession missing status on retrieve".into())
    })?;

    match status {
        CheckoutSessionStatus::Open => Ok(CheckoutSessionState::Open),
        CheckoutSessionStatus::Expired => Ok(CheckoutSessionState::Expired),
        CheckoutSessionStatus::Complete => {
            let paid = matches!(
                session.payment_status,
                CheckoutSessionPaymentStatus::Paid
                    | CheckoutSessionPaymentStatus::NoPaymentRequired
            );
            let payment_ref = session.payment_intent.map(|exp| match exp {
                stripe_types::Expandable::Id(id) => id.as_str().to_string(),
                stripe_types::Expandable::Object(pi) => pi.id.as_str().to_string(),
            });
            let amount_total = match (session.amount_total, session.currency) {
                (Some(amount), Some(currency)) => Some(stripe_currency_to_money(amount, currency)?),
                _ => None,
            };
            Ok(CheckoutSessionState::Complete {
                paid,
                payment_ref,
                amount_total,
            })
        }
        // CheckoutSessionStatus is #[non_exhaustive] — surface unrecognized
        // statuses honestly rather than guessing a mapping.
        other => Err(PaymentError::Provider(format!(
            "CheckoutSession has unrecognized status: {}",
            other.as_str()
        ))),
    }
}

#[async_trait]
impl Checkout for StripeProvider {
    async fn start_session(&self, req: StartSessionRequest) -> PaymentResult<SessionPayload> {
        match req.mode {
            SessionMode::OneOff => {
                // Predefined Prices → hosted Checkout Session (mode=payment).
                // This is the only one-off path compatible with Managed
                // Payments, which requires Stripe-hosted checkout.
                if !req.price_refs.is_empty() {
                    let line_items: Vec<LineItem> = req
                        .price_refs
                        .iter()
                        .map(|p| LineItem {
                            price: p,
                            quantity: 1,
                        })
                        .collect();

                    let params = CreateCheckoutSessionParams {
                        mode: "payment",
                        customer: &req.customer_ref,
                        success_url: &req.success_return_url,
                        cancel_url: &req.cancel_return_url,
                        line_items,
                        allow_promotion_codes: true,
                        managed_payments_enabled: self.managed_payments(),
                    };

                    let session: CheckoutSession =
                        RequestBuilder::new(StripeMethod::Post, "/checkout/sessions")
                            .form(&params)
                            .customize::<CheckoutSession>()
                            .send(self.client())
                            .await
                            .map_err(|e| {
                                PaymentError::Provider(format!(
                                    "stripe checkout.sessions.create: {e}"
                                ))
                            })?;

                    let url = session.url.ok_or_else(|| {
                        PaymentError::Provider("CheckoutSession missing url on create".into())
                    })?;

                    return Ok(SessionPayload::StripeCheckoutRedirect {
                        url,
                        provider_session_id: session.id.as_str().to_string(),
                    });
                }

                // Ad-hoc amount → PaymentIntent for a Stripe Elements embed.
                let amount = req.amount_hint.ok_or_else(|| {
                    PaymentError::Validation(
                        "OneOff mode requires price_refs (hosted Checkout) or amount_hint (Elements)"
                            .into(),
                    )
                })?;
                let currency = amount.currency().code().to_lowercase();

                let params = CreatePaymentIntentParams {
                    amount: amount.minor_units(),
                    currency: &currency,
                    customer: &req.customer_ref,
                    automatic_payment_methods_enabled: true,
                    description: None,
                };

                let intent: PaymentIntent =
                    RequestBuilder::new(StripeMethod::Post, "/payment_intents")
                        .form(&params)
                        .customize::<PaymentIntent>()
                        .send(self.client())
                        .await
                        .map_err(|e| {
                            PaymentError::Provider(format!("stripe payment_intents.create: {e}"))
                        })?;

                let client_secret = intent.client_secret.ok_or_else(|| {
                    PaymentError::Provider("PaymentIntent missing client_secret on create".into())
                })?;

                Ok(SessionPayload::StripeElements {
                    client_secret,
                    publishable_key: self.publishable_key().to_string(),
                    provider_session_id: intent.id.as_str().to_string(),
                })
            }
            SessionMode::Subscription => {
                let line_items: Vec<LineItem> = req
                    .price_refs
                    .iter()
                    .map(|p| LineItem {
                        price: p,
                        quantity: 1,
                    })
                    .collect();

                if line_items.is_empty() {
                    return Err(PaymentError::Validation(
                        "Subscription mode requires at least one price_ref".into(),
                    ));
                }

                let params = CreateCheckoutSessionParams {
                    mode: "subscription",
                    customer: &req.customer_ref,
                    success_url: &req.success_return_url,
                    cancel_url: &req.cancel_return_url,
                    line_items,
                    // Preserve pre-v0.6.5 subscription behavior: no promo
                    // field is sent (Stripe defaults to disallowed).
                    allow_promotion_codes: false,
                    managed_payments_enabled: false,
                };

                let session: CheckoutSession =
                    RequestBuilder::new(StripeMethod::Post, "/checkout/sessions")
                        .form(&params)
                        .customize::<CheckoutSession>()
                        .send(self.client())
                        .await
                        .map_err(|e| {
                            PaymentError::Provider(format!("stripe checkout.sessions.create: {e}"))
                        })?;

                let url = session.url.ok_or_else(|| {
                    PaymentError::Provider("CheckoutSession missing url on create".into())
                })?;

                Ok(SessionPayload::StripeCheckoutRedirect {
                    url,
                    provider_session_id: session.id.as_str().to_string(),
                })
            }
        }
    }

    /// Retrieve a Checkout Session and report its provider-neutral state.
    async fn session_status(
        &self,
        provider_session_id: &str,
    ) -> PaymentResult<CheckoutSessionState> {
        let path = format!("/checkout/sessions/{provider_session_id}");

        let session: CheckoutSession = RequestBuilder::new(StripeMethod::Get, &path)
            .customize::<CheckoutSession>()
            .send(self.client())
            .await
            .map_err(|e| {
                PaymentError::Provider(format!("stripe checkout.sessions.retrieve: {e}"))
            })?;

        session_to_state(session)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use suprnova::payments::{Currency, Money};

    // -----------------------------------------------------------------
    // Param serialization — field names and skip behavior. serde_json
    // sees the same serde attributes the wire encoder does, so renames
    // and skip_serializing_if gates are exercised without a network.
    // -----------------------------------------------------------------

    fn params<'a>(
        mode: &'static str,
        line_items: Vec<LineItem<'a>>,
        allow_promotion_codes: bool,
        managed_payments_enabled: bool,
    ) -> CreateCheckoutSessionParams<'a> {
        CreateCheckoutSessionParams {
            mode,
            customer: "cus_1",
            success_url: "https://app.example/return?session_id={CHECKOUT_SESSION_ID}",
            cancel_url: "https://app.example/cancel",
            line_items,
            allow_promotion_codes,
            managed_payments_enabled,
        }
    }

    #[test]
    fn hosted_one_off_params_serialize_payment_mode_with_flags() {
        let p = params(
            "payment",
            vec![LineItem {
                price: "price_x",
                quantity: 1,
            }],
            true,
            true,
        );
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["mode"], "payment");
        assert_eq!(v["line_items"][0]["price"], "price_x");
        assert_eq!(v["line_items"][0]["quantity"], 1);
        assert_eq!(v["allow_promotion_codes"], true);
        assert_eq!(v["managed_payments[enabled]"], true);
    }

    #[test]
    fn managed_payments_field_is_omitted_when_off() {
        let p = params(
            "payment",
            vec![LineItem {
                price: "price_x",
                quantity: 1,
            }],
            true,
            false,
        );
        let v = serde_json::to_value(&p).unwrap();
        // Off must mean ABSENT, not false — non-enrolled accounts reject
        // the field outright.
        assert!(v.get("managed_payments[enabled]").is_none());
    }

    #[test]
    fn subscription_params_keep_pre_065_shape() {
        let p = params(
            "subscription",
            vec![LineItem {
                price: "price_sub",
                quantity: 1,
            }],
            false,
            false,
        );
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["mode"], "subscription");
        assert!(v.get("allow_promotion_codes").is_none());
        assert!(v.get("managed_payments[enabled]").is_none());
    }

    // -----------------------------------------------------------------
    // session_to_state mapping — fixtures parsed through stripe-shared's
    // own Deserialize impl (dev-only `deserialize` feature).
    // -----------------------------------------------------------------

    /// Minimal Checkout Session JSON. `payment_status` and enough required
    /// scalar fields for stripe-shared's Deserialize to accept the object.
    fn session_json(status: &str, payment_status: &str, with_totals: bool) -> String {
        let totals = if with_totals {
            r#""amount_total": 2900, "currency": "usd", "payment_intent": "pi_123","#
        } else {
            r#""amount_total": null, "currency": null, "payment_intent": null,"#
        };
        format!(
            r#"{{
                "id": "cs_test_1",
                "object": "checkout.session",
                "automatic_tax": {{"enabled": false, "liability": null, "status": null}},
                "created": 1720000000,
                "expires_at": 1720086400,
                "livemode": false,
                "mode": "payment",
                "payment_method_types": ["card"],
                "shipping_options": [],
                "payment_status": "{payment_status}",
                "status": "{status}",
                {totals}
                "custom_fields": [],
                "custom_text": {{"after_submit": null, "shipping_address": null, "submit": null, "terms_of_service_acceptance": null}},
                "metadata": {{}}
            }}"#
        )
    }

    fn parse(json: &str) -> CheckoutSession {
        serde_json::from_str(json).expect("fixture must deserialize as CheckoutSession")
    }

    #[test]
    fn complete_paid_session_maps_with_payment_ref_and_total() {
        let state = session_to_state(parse(&session_json("complete", "paid", true))).unwrap();
        assert_eq!(
            state,
            CheckoutSessionState::Complete {
                paid: true,
                payment_ref: Some("pi_123".into()),
                amount_total: Some(Money::from_minor_units(2900, Currency::USD)),
            }
        );
    }

    #[test]
    fn complete_unpaid_session_maps_paid_false() {
        let state = session_to_state(parse(&session_json("complete", "unpaid", false))).unwrap();
        assert_eq!(
            state,
            CheckoutSessionState::Complete {
                paid: false,
                payment_ref: None,
                amount_total: None,
            }
        );
    }

    #[test]
    fn open_and_expired_sessions_map_directly() {
        assert_eq!(
            session_to_state(parse(&session_json("open", "unpaid", false))).unwrap(),
            CheckoutSessionState::Open
        );
        assert_eq!(
            session_to_state(parse(&session_json("expired", "unpaid", false))).unwrap(),
            CheckoutSessionState::Expired
        );
    }
}
