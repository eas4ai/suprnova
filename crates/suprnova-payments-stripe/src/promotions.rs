//! Implementation of the `Promotions` trait for `StripeProvider`.
//!
//! Maps Suprnova's provider-neutral promotion-code minting onto Stripe's
//! `/v1/promotion_codes` — each call mints a code off an existing coupon,
//! restricted to one customer and one redemption window. The coupon itself
//! (the percent/amount-off object) is created ahead of time in the Stripe
//! dashboard or API; this surface only mints codes.

use async_trait::async_trait;
use serde::Serialize;
use stripe_client_core::{RequestBuilder, StripeMethod};

use suprnova::payments::{
    CreatePromotionCodeRequest, PaymentError, PaymentResult, PromotionCode, Promotions,
};

use crate::StripeProvider;

#[derive(Serialize)]
struct CreatePromotionCodeParams<'a> {
    coupon: &'a str,
    customer: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_redemptions: Option<u32>,
}

#[async_trait]
impl Promotions for StripeProvider {
    /// Mint a customer-restricted promotion code off an existing coupon.
    async fn create_promotion_code(
        &self,
        req: CreatePromotionCodeRequest,
    ) -> PaymentResult<PromotionCode> {
        let params = CreatePromotionCodeParams {
            coupon: &req.coupon_ref,
            customer: &req.customer_ref,
            expires_at: req.expires_at.map(|t| t.timestamp()),
            max_redemptions: req.max_redemptions,
        };

        let code: stripe_shared::PromotionCode =
            RequestBuilder::new(StripeMethod::Post, "/promotion_codes")
                .form(&params)
                .customize::<stripe_shared::PromotionCode>()
                .send(self.client())
                .await
                .map_err(|e| {
                    PaymentError::Provider(format!("stripe promotion_codes.create: {e}"))
                })?;

        Ok(PromotionCode {
            code: code.code,
            provider_promotion_id: code.id.as_str().to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    #[test]
    fn params_serialize_expiry_as_unix_seconds_and_skip_absent_fields() {
        let expires = Utc.with_ymd_and_hms(2026, 8, 1, 12, 0, 0).unwrap();
        let with_all = CreatePromotionCodeParams {
            coupon: "coupon_15",
            customer: "cus_1",
            expires_at: Some(expires.timestamp()),
            max_redemptions: Some(1),
        };
        let v = serde_json::to_value(&with_all).unwrap();
        assert_eq!(v["coupon"], "coupon_15");
        assert_eq!(v["customer"], "cus_1");
        assert_eq!(v["expires_at"], expires.timestamp());
        assert_eq!(v["max_redemptions"], 1);

        let minimal = CreatePromotionCodeParams {
            coupon: "coupon_15",
            customer: "cus_1",
            expires_at: None,
            max_redemptions: None,
        };
        let v = serde_json::to_value(&minimal).unwrap();
        assert!(v.get("expires_at").is_none());
        assert!(v.get("max_redemptions").is_none());
    }
}
