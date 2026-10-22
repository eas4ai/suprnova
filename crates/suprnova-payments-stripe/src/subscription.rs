//! Implementation of the `Subscription` trait for `StripeProvider`.
//!
//! Maps Suprnova's provider-neutral subscription lifecycle onto Stripe's
//! `/v1/subscriptions` API.

use std::collections::HashSet;

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use serde::Serialize;
use stripe_client_core::{RequestBuilder, StripeMethod};
use stripe_shared::{Subscription as StripeSubscription, SubscriptionStatus as StripeSubStatus};

use suprnova::payments::{
    Money, PaymentError, PaymentResult, Proration, SubscribeRequest, Subscription,
    SubscriptionItemSnapshot, SubscriptionResult, SubscriptionStatus, UpdateSubscriptionRequest,
};

use crate::StripeProvider;
use crate::deadline;
use crate::sdk_error;

#[derive(Serialize)]
struct CreateSubscriptionParams<'a> {
    customer: &'a str,
    #[serde(serialize_with = "serialize_items")]
    items: Vec<ItemParam<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    trial_period_days: Option<u32>,
}

#[derive(Serialize)]
struct ItemParam<'a> {
    price: &'a str,
    quantity: u32,
}

fn serialize_items<S>(items: &[ItemParam<'_>], s: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::ser::SerializeSeq;
    let mut seq = s.serialize_seq(Some(items.len()))?;
    for item in items {
        seq.serialize_element(item)?;
    }
    seq.end()
}

#[derive(Serialize)]
struct UpdateSubscriptionParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    cancel_at_period_end: Option<bool>,
}

#[derive(Serialize)]
struct CancelSubscriptionParams {
    invoice_now: bool,
    prorate: bool,
}

/// The form of a price change: `items[n][...]` for each item change, and
/// the proration behavior only when an item changes.
#[derive(Serialize)]
struct ChangePricesParams<'a> {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    items: Vec<ItemChange<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    proration_behavior: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cancel_at_period_end: Option<bool>,
}

/// One entry of `items` in a subscription update: an item to delete (`id`
/// and `deleted`) or a price to add (`price` and `quantity`). An item that
/// stays is not in the list, so Stripe leaves it and its quantity alone.
#[derive(Serialize)]
struct ItemChange<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    deleted: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    price: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    quantity: Option<u32>,
}

/// Refuse a price set before any request: an empty one would leave the
/// subscription with nothing to bill, and one that names a price twice says
/// two different things about its quantity.
fn check_price_set(prices: &[String]) -> PaymentResult<()> {
    if prices.is_empty() {
        return Err(PaymentError::Validation(
            "new_price_refs requires at least one price".into(),
        ));
    }
    let mut seen = HashSet::with_capacity(prices.len());
    if !prices.iter().all(|price| seen.insert(price.as_str())) {
        return Err(PaymentError::Validation(
            "new_price_refs names a price more than once".into(),
        ));
    }
    Ok(())
}

/// The Stripe `proration_behavior` of a [`Proration`]; no mode is
/// `create_prorations`, the default of Stripe.
fn proration_behavior(proration: Option<Proration>) -> PaymentResult<&'static str> {
    match proration {
        None | Some(Proration::ProrateAtRenewal) => Ok("create_prorations"),
        Some(Proration::ProrateNow) => Ok("always_invoice"),
        Some(Proration::DoNotProrate) => Ok("none"),
        Some(other) => Err(PaymentError::NotSupported(format!(
            "Stripe has no proration behavior for {other:?}"
        ))),
    }
}

/// The quantity of an item Stripe reported, as the `u32` of an item of an
/// update: 1 for a price without quantities. A quantity that does not fit
/// is refused, not sent changed.
fn item_quantity(quantity: Option<u64>) -> PaymentResult<u32> {
    match quantity {
        None => Ok(1),
        Some(quantity) => u32::try_from(quantity).map_err(|_| {
            PaymentError::Provider(
                "stripe subscriptions.retrieve: an item quantity is out of range".into(),
            )
        }),
    }
}

fn ts_to_dt(ts: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(ts, 0).single().unwrap_or_else(Utc::now)
}

fn map_status(s: &StripeSubStatus) -> SubscriptionStatus {
    match s {
        StripeSubStatus::Trialing => SubscriptionStatus::Trialing,
        StripeSubStatus::Active => SubscriptionStatus::Active,
        StripeSubStatus::PastDue => SubscriptionStatus::PastDue,
        StripeSubStatus::Canceled => SubscriptionStatus::Canceled,
        StripeSubStatus::Incomplete | StripeSubStatus::IncompleteExpired => {
            SubscriptionStatus::Incomplete
        }
        StripeSubStatus::Paused => SubscriptionStatus::Paused,
        StripeSubStatus::Unpaid => SubscriptionStatus::PastDue,
        // SubscriptionStatus is #[non_exhaustive] - surface new states as Incomplete (safest
        // default; callers should treat as not-yet-billable until clarified by a webhook).
        _ => SubscriptionStatus::Incomplete,
    }
}

fn map_subscription(s: StripeSubscription) -> SubscriptionResult {
    let customer_id = match s.customer {
        stripe_types::Expandable::Id(id) => id.as_str().to_string(),
        stripe_types::Expandable::Object(obj) => obj.id.as_str().to_string(),
    };

    // In Stripe API 2023-08-16+ the billing-period timestamps moved from Subscription to
    // each SubscriptionItem. Multi-item subscriptions can theoretically have divergent
    // periods, but in practice all items share the parent's cycle. Take the first item's
    // period as the parent period.
    let (period_start, period_end) = s
        .items
        .data
        .first()
        .map(|item| (item.current_period_start, item.current_period_end))
        .unwrap_or((0, 0));

    let items: Vec<SubscriptionItemSnapshot> = s
        .items
        .data
        .iter()
        .map(|item| {
            let unit_amount = item.price.unit_amount.and_then(|amount| {
                let code = format!("{}", item.price.currency).to_uppercase();
                suprnova::payments::Currency::from_code(&code)
                    .map(|iso| Money::from_minor_units(amount, iso))
            });

            SubscriptionItemSnapshot {
                provider_item_id: item.id.as_str().to_string(),
                provider_price_id: item.price.id.as_str().to_string(),
                // Saturate a u64 quantity into u32 rather than truncating the
                // high bits; default to 1 when Stripe omits it (non-quantitative
                // prices).
                quantity: item
                    .quantity
                    .map(|q| u32::try_from(q).unwrap_or(u32::MAX))
                    .unwrap_or(1),
                unit_amount,
            }
        })
        .collect();

    SubscriptionResult {
        provider_subscription_id: s.id.as_str().to_string(),
        provider_customer_id: customer_id,
        status: map_status(&s.status),
        items,
        current_period_start: ts_to_dt(period_start),
        current_period_end: ts_to_dt(period_end),
        cancel_at_period_end: s.cancel_at_period_end,
        provider_metadata: serde_json::json!({
            "stripe_status": format!("{:?}", s.status),
        }),
    }
}

impl StripeProvider {
    /// Replace the prices of a subscription.
    ///
    /// Stripe changes the items of a subscription one by one, so the
    /// subscription is read first: an item whose price is not in `prices`
    /// is deleted, a price no item has is added, and an item whose price
    /// stays is not sent. A price that replaces exactly one deleted item
    /// takes over its quantity; any other new price has a quantity of 1. If
    /// the read fails, nothing is sent.
    async fn change_prices(
        &self,
        req: &UpdateSubscriptionRequest,
        prices: &[String],
    ) -> PaymentResult<SubscriptionResult> {
        check_price_set(prices)?;
        let behavior = proration_behavior(req.proration)?;
        let key = crate::idempotency_key(req.idempotency_key.as_deref())?;

        let path = format!("/subscriptions/{}", req.provider_subscription_id);
        let call = RequestBuilder::new(StripeMethod::Get, &path)
            .customize::<StripeSubscription>()
            .send(self.client());
        let current: StripeSubscription = deadline::read("subscriptions.retrieve", call)
            .await?
            .map_err(|e| sdk_error::provider_error("subscriptions.retrieve", e))?;
        if current.items.has_more {
            // The update would leave the items past the first page as they
            // are, whatever `prices` says.
            return Err(PaymentError::Provider(
                "stripe subscriptions.retrieve: the items span more than one page".into(),
            ));
        }

        let items = &current.items.data;
        let removed: Vec<_> = items
            .iter()
            .filter(|item| {
                !prices
                    .iter()
                    .any(|price| price.as_str() == item.price.id.as_str())
            })
            .collect();
        let added: Vec<&str> = prices
            .iter()
            .map(String::as_str)
            .filter(|price| !items.iter().any(|item| item.price.id.as_str() == *price))
            .collect();

        // A swap of one price for another keeps the quantity of the price
        // it replaces; any other new price starts at 1.
        let new_quantity = match (removed.as_slice(), added.len()) {
            ([replaced], 1) => item_quantity(replaced.quantity)?,
            _ => 1,
        };

        let mut changes = Vec::with_capacity(removed.len() + added.len());
        for item in &removed {
            changes.push(ItemChange {
                id: Some(item.id.as_str().to_string()),
                deleted: Some(true),
                price: None,
                quantity: None,
            });
        }
        for price in added {
            changes.push(ItemChange {
                id: None,
                deleted: None,
                price: Some(price),
                quantity: Some(new_quantity),
            });
        }
        if changes.is_empty() && req.cancel_at_period_end.is_none() {
            // The same prices: nothing is sent that could bill for a
            // change, and a retry of an update that went through ends here.
            return Ok(map_subscription(current));
        }

        let proration_behavior = (!changes.is_empty()).then_some(behavior);
        let params = ChangePricesParams {
            items: changes,
            proration_behavior,
            cancel_at_period_end: req.cancel_at_period_end,
        };
        let request = RequestBuilder::new(StripeMethod::Post, &path)
            .form(&params)
            .customize::<StripeSubscription>();
        let call = crate::with_idempotency_key(request, key).send(self.client());
        let sub = deadline::change("subscriptions.update", call)
            .await?
            .map_err(|e| sdk_error::provider_error("subscriptions.update", e))?;
        Ok(map_subscription(sub))
    }
}

#[async_trait]
impl Subscription for StripeProvider {
    async fn subscribe(&self, req: SubscribeRequest) -> PaymentResult<SubscriptionResult> {
        if req.price_refs.is_empty() {
            return Err(PaymentError::Validation(
                "subscribe requires at least one price_ref".into(),
            ));
        }

        let items: Vec<ItemParam> = req
            .price_refs
            .iter()
            .map(|p| ItemParam {
                price: p,
                quantity: 1,
            })
            .collect();

        let params = CreateSubscriptionParams {
            customer: &req.customer_ref,
            items,
            trial_period_days: req.trial_days,
        };

        let request = RequestBuilder::new(StripeMethod::Post, "/subscriptions")
            .form(&params)
            .customize::<StripeSubscription>();
        let call =
            crate::apply_idempotency(request, req.idempotency_key.as_deref())?.send(self.client());
        let sub = deadline::change("subscriptions.create", call)
            .await?
            .map_err(|e| sdk_error::provider_error("subscriptions.create", e))?;

        Ok(map_subscription(sub))
    }

    async fn update(&self, req: UpdateSubscriptionRequest) -> PaymentResult<SubscriptionResult> {
        if let Some(prices) = &req.new_price_refs {
            return self.change_prices(&req, prices).await;
        }

        let params = UpdateSubscriptionParams {
            cancel_at_period_end: req.cancel_at_period_end,
        };

        let path = format!("/subscriptions/{}", req.provider_subscription_id);
        let request = RequestBuilder::new(StripeMethod::Post, &path)
            .form(&params)
            .customize::<StripeSubscription>();
        let call =
            crate::apply_idempotency(request, req.idempotency_key.as_deref())?.send(self.client());
        let sub = deadline::change("subscriptions.update", call)
            .await?
            .map_err(|e| sdk_error::provider_error("subscriptions.update", e))?;

        Ok(map_subscription(sub))
    }

    async fn cancel(
        &self,
        provider_subscription_id: &str,
        at_period_end: bool,
    ) -> PaymentResult<SubscriptionResult> {
        if at_period_end {
            let path = format!("/subscriptions/{provider_subscription_id}");
            let params = UpdateSubscriptionParams {
                cancel_at_period_end: Some(true),
            };
            let call = RequestBuilder::new(StripeMethod::Post, &path)
                .form(&params)
                .customize::<StripeSubscription>()
                .send(self.client());
            let sub: StripeSubscription = deadline::change("subscriptions.update(cape)", call)
                .await?
                .map_err(|e| sdk_error::provider_error("subscriptions.update(cape)", e))?;
            Ok(map_subscription(sub))
        } else {
            let path = format!("/subscriptions/{provider_subscription_id}");
            let params = CancelSubscriptionParams {
                invoice_now: false,
                prorate: false,
            };
            let call = RequestBuilder::new(StripeMethod::Delete, &path)
                .form(&params)
                .customize::<StripeSubscription>()
                .send(self.client());
            let sub: StripeSubscription = deadline::change("subscriptions.cancel", call)
                .await?
                .map_err(|e| sdk_error::provider_error("subscriptions.cancel", e))?;
            Ok(map_subscription(sub))
        }
    }

    async fn get(&self, provider_subscription_id: &str) -> PaymentResult<SubscriptionResult> {
        let path = format!("/subscriptions/{provider_subscription_id}");
        let call = RequestBuilder::new(StripeMethod::Get, &path)
            .customize::<StripeSubscription>()
            .send(self.client());
        let sub: StripeSubscription = deadline::read("subscriptions.retrieve", call)
            .await?
            .map_err(|e| sdk_error::provider_error("subscriptions.retrieve", e))?;

        Ok(map_subscription(sub))
    }
}

#[cfg(test)]
mod tests;
