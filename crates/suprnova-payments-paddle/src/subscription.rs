//! Implementation of the `Subscription` trait for `PaddleProvider`.
//!
//! `subscribe()` returns `NotSupported` - Paddle subscriptions are created
//! indirectly via checkout completion, NOT directly. Domain code should call
//! `Checkout::start_session` with `SessionMode::Subscription` and react to
//! the `SubscriptionCreated` webhook for the resulting subscription_id.
//!
//! `cancel`, `update`, and `get` wire through the SDK. An `update` with
//! `new_price_refs` reads the subscription, then replaces its items with
//! one PATCH of `items` and a `proration_billing_mode`. `cancel` cancels
//! from the end of the billing period or at once, as `at_period_end` says.
//! Every call runs under the deadline of `crate::deadline`.

use std::collections::HashSet;

use async_trait::async_trait;
use paddle_rust_sdk::entities::{Subscription as PaddleSubscription, SubscriptionWithInclude};
use paddle_rust_sdk::enums::{EffectiveFrom, ProrationBillingMode};
use paddle_rust_sdk::transactions::TransactionItem;
use suprnova::payments::{
    PaymentError, PaymentResult, Proration, SubscribeRequest, Subscription,
    SubscriptionItemSnapshot, SubscriptionResult, SubscriptionStatus, UpdateSubscriptionRequest,
};

use crate::PaddleProvider;
use crate::deadline;
use crate::sdk_error;

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

/// The Paddle mode of a [`Proration`]; no mode is Paddle's
/// `prorated_next_billing_period`, the mode of `Proration::ProrateAtRenewal`.
fn proration_billing_mode(proration: Option<Proration>) -> PaymentResult<ProrationBillingMode> {
    match proration {
        None | Some(Proration::ProrateAtRenewal) => {
            Ok(ProrationBillingMode::ProratedNextBillingPeriod)
        }
        Some(Proration::ProrateNow) => Ok(ProrationBillingMode::ProratedImmediately),
        Some(Proration::DoNotProrate) => Ok(ProrationBillingMode::DoNotBill),
        Some(other) => Err(PaymentError::NotSupported(format!(
            "Paddle has no proration billing mode for {other:?}"
        ))),
    }
}

/// The quantity of an item Paddle reported, as the `u32` an item of the
/// SDK's update takes. A quantity that does not fit is refused, not sent
/// changed.
fn item_quantity(quantity: i64) -> PaymentResult<u32> {
    u32::try_from(quantity).map_err(|_| {
        PaymentError::Provider("paddle subscription_get: an item quantity is out of range".into())
    })
}

impl PaddleProvider {
    /// Read the subscription as Paddle has it now.
    async fn snapshot(&self, id: &str) -> PaymentResult<SubscriptionWithInclude> {
        let response = deadline::read(
            "subscription_get",
            self.client().subscription_get(id.to_string()).send(),
        )
        .await?
        .map_err(|e| sdk_error::provider_error("subscription_get", e))?;
        Ok(response.data)
    }

    /// Cancel a subscription from `effective_from`. `operation` names the
    /// call in the errors.
    async fn cancel_from(
        &self,
        id: &str,
        effective_from: EffectiveFrom,
        operation: &str,
    ) -> PaymentResult<PaddleSubscription> {
        let response = deadline::change(
            operation,
            self.client()
                .subscription_cancel(id.to_string())
                .effective_from(effective_from)
                .send(),
        )
        .await?
        .map_err(|e| sdk_error::provider_error(operation, e))?;
        Ok(response.data)
    }

    /// Replace the prices of a subscription.
    ///
    /// Paddle takes the complete list of items and a quantity for each, so
    /// the subscription is read first: a price that stays is sent with the
    /// quantity it has, a new price with 1, and a price left out of the
    /// list is removed by Paddle. A price that replaces exactly one removed
    /// price takes over its quantity. If the read fails, nothing is sent.
    async fn change_prices(
        &self,
        req: &UpdateSubscriptionRequest,
        prices: &[String],
    ) -> PaymentResult<SubscriptionResult> {
        check_price_set(prices)?;
        if req.cancel_at_period_end.is_some() {
            // Paddle schedules a cancellation through its own endpoint, so
            // the two changes can't be made in one request that either
            // happens or doesn't.
            return Err(PaymentError::NotSupported(
                "Paddle cannot change the prices and the cancellation of a subscription in \
                 one request. Send the price change and the cancellation as two updates."
                    .into(),
            ));
        }
        crate::reject_unsupported_idempotency_key(
            req.idempotency_key.as_deref(),
            "subscription price change",
        )?;
        let mode = proration_billing_mode(req.proration)?;

        let current = self.snapshot(&req.provider_subscription_id).await?;
        let current_items = &current.subscription.items;

        // The same prices: there is nothing to change, and no request is
        // sent that could bill for it.
        let unchanged = current_items.len() == prices.len()
            && current_items
                .iter()
                .all(|item| prices.contains(&item.price.id.0));
        if unchanged {
            return Ok(map_subscription_with_include(&current));
        }

        // A swap of one price for another keeps the quantity of the price
        // it replaces; any other new price starts at 1.
        let mut removed = current_items
            .iter()
            .filter(|item| !prices.contains(&item.price.id.0));
        let swapped_quantity = match (removed.next(), removed.next()) {
            (Some(item), None) if prices.len() == current_items.len() => {
                Some(item_quantity(item.quantity)?)
            }
            _ => None,
        };

        let mut items = Vec::with_capacity(prices.len());
        for price in prices {
            let quantity = match current_items.iter().find(|item| item.price.id.0 == *price) {
                Some(item) => item_quantity(item.quantity)?,
                None => swapped_quantity.unwrap_or(1),
            };
            items.push(TransactionItem::CatalogItem {
                price_id: price.clone().into(),
                quantity,
            });
        }

        let resp = deadline::change(
            "subscription_update",
            self.client()
                .subscription_update(req.provider_subscription_id.clone())
                .items(items)
                .proration_billing_mode(mode)
                .send(),
        )
        .await?
        .map_err(|e| sdk_error::provider_error("subscription_update", e))?;
        Ok(map_paddle_subscription(&resp.data))
    }
}

#[async_trait]
impl Subscription for PaddleProvider {
    async fn subscribe(&self, _req: SubscribeRequest) -> PaymentResult<SubscriptionResult> {
        Err(PaymentError::NotSupported(
            "Paddle subscriptions are created via Checkout::start_session + checkout completion. \
             Use Checkout::start_session with SessionMode::Subscription and await the \
             SubscriptionCreated webhook for the resulting subscription_id."
                .into(),
        ))
    }

    async fn update(&self, req: UpdateSubscriptionRequest) -> PaymentResult<SubscriptionResult> {
        if let Some(prices) = &req.new_price_refs {
            return self.change_prices(&req, prices).await;
        }

        match req.cancel_at_period_end {
            Some(true) => {
                crate::reject_unsupported_idempotency_key(
                    req.idempotency_key.as_deref(),
                    "subscription cancellation scheduling",
                )?;
                // The same request as cancel(at_period_end = true). The
                // mode is named, not left to Paddle's default, which is
                // "at once" for a paused subscription.
                let cancelled = self
                    .cancel_from(
                        &req.provider_subscription_id,
                        EffectiveFrom::NextBillingPeriod,
                        "subscription_cancel (cape)",
                    )
                    .await?;
                Ok(map_paddle_subscription(&cancelled))
            }
            Some(false) => {
                // This adapter does not take back a scheduled cancellation.
                // Falling through to `subscription_get` would report success
                // while the cancellation stays scheduled, so the call fails
                // before any request and says that nothing changed.
                Err(PaymentError::NotSupported(
                    "the Paddle adapter does not take back a scheduled cancellation \
                     (Subscription::update with cancel_at_period_end: Some(false)); \
                     nothing was changed at Paddle"
                        .into(),
                ))
            }
            None => {
                // No-op update (no cancel_at_period_end delta, no price-set
                // change): re-fetch current state via subscription_get so the
                // caller always observes the authoritative provider snapshot.
                let current = self.snapshot(&req.provider_subscription_id).await?;
                Ok(map_subscription_with_include(&current))
            }
        }
    }

    async fn cancel(
        &self,
        provider_subscription_id: &str,
        at_period_end: bool,
    ) -> PaymentResult<SubscriptionResult> {
        if at_period_end {
            // The subscription stays active and Paddle schedules the
            // cancellation for the end of the billing period.
            let cancelled = self
                .cancel_from(
                    provider_subscription_id,
                    EffectiveFrom::NextBillingPeriod,
                    "subscription_cancel",
                )
                .await?;
            return Ok(map_paddle_subscription(&cancelled));
        }

        // Paddle clears the billing period of a subscription it cancels at
        // once, while Stripe and the mock keep reporting the period the
        // subscription was in. The period is read before the cancel so the
        // result says the same on every provider; if the read fails,
        // nothing is cancelled.
        let before = self.snapshot(provider_subscription_id).await?;
        let cancelled = self
            .cancel_from(
                provider_subscription_id,
                EffectiveFrom::Immediately,
                "subscription_cancel",
            )
            .await?;
        let mut result = map_paddle_subscription(&cancelled);
        if cancelled.current_billing_period.is_none()
            && let Some(period) = &before.subscription.current_billing_period
        {
            result.current_period_start = period.starts_at;
            result.current_period_end = period.ends_at;
        }
        Ok(result)
    }

    async fn get(&self, provider_subscription_id: &str) -> PaymentResult<SubscriptionResult> {
        let current = self.snapshot(provider_subscription_id).await?;
        Ok(map_subscription_with_include(&current))
    }
}

/// Map a status value to our enum by inspecting its Debug representation. The
/// underlying Paddle SubscriptionStatus enum is private; this avoids a dependency
/// on internal SDK types while still producing reliable mappings.
fn map_status_from_debug<S: std::fmt::Debug>(status: &S) -> SubscriptionStatus {
    let s = format!("{status:?}").to_lowercase();
    if s.contains("active") {
        SubscriptionStatus::Active
    } else if s.contains("trialing") || s.contains("trial") {
        SubscriptionStatus::Trialing
    } else if s.contains("past") {
        SubscriptionStatus::PastDue
    } else if s.contains("paused") {
        SubscriptionStatus::Paused
    } else if s.contains("cancel") {
        SubscriptionStatus::Canceled
    } else {
        SubscriptionStatus::Incomplete
    }
}

fn map_paddle_subscription(s: &PaddleSubscription) -> SubscriptionResult {
    let items: Vec<SubscriptionItemSnapshot> = s
        .items
        .iter()
        .map(|item| SubscriptionItemSnapshot {
            provider_item_id: item.price.id.to_string(),
            provider_price_id: item.price.id.to_string(),
            // Saturate rather than silently wrapping: a Paddle i64 quantity
            // outside u32 range (a negative adjustment, or an absurd bulk
            // count) must not two's-complement-wrap into a bogus billing
            // quantity. Real subscription quantities are small positives.
            quantity: item.quantity.clamp(0, u32::MAX as i64) as u32,
            unit_amount: None,
        })
        .collect();

    let now = chrono::Utc::now();
    let (period_start, period_end) = s
        .current_billing_period
        .as_ref()
        .map(|p| (p.starts_at, p.ends_at))
        .unwrap_or((now, now));

    let cancel_at_period_end = s
        .scheduled_change
        .as_ref()
        .map(|sc| format!("{sc:?}").to_lowercase().contains("cancel"))
        .unwrap_or(false);

    SubscriptionResult {
        provider_subscription_id: s.id.to_string(),
        provider_customer_id: s.customer_id.to_string(),
        status: map_status_from_debug(&s.status),
        items,
        current_period_start: period_start,
        current_period_end: period_end,
        cancel_at_period_end,
        provider_metadata: serde_json::json!({
            "paddle_status": format!("{:?}", s.status),
        }),
    }
}

fn map_subscription_with_include(s: &SubscriptionWithInclude) -> SubscriptionResult {
    // SubscriptionWithInclude extends Subscription with optional included entities.
    // The base Subscription fields are accessible via the .subscription field on the
    // wrapper. If that shape doesn't match (e.g. SubscriptionWithInclude is just an
    // alias with public fields), the compiler will point us at the right path.
    map_paddle_subscription(&s.subscription)
}

#[cfg(test)]
mod tests;
