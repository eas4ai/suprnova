//! Promotion-code trait for payment providers.

use crate::payments::PaymentResult;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Request payload for [`Promotions::create_promotion_code`].
///
/// The discount itself (percent or amount off) lives in a provider-side
/// coupon object created ahead of time — typically once, by hand, in the
/// provider's dashboard. This request mints a *code* off that coupon,
/// scoped to one customer and one redemption window, which is the shape
/// win-back and upsell campaigns need: each recipient gets their own
/// code, unusable by anyone else and dead after the window closes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePromotionCodeRequest {
    /// Provider coupon identifier the code discounts through
    /// (e.g. a Stripe coupon id).
    pub coupon_ref: String,
    /// Provider customer identifier the code is restricted to
    /// (e.g. Stripe's `cus_…`). Redemption by any other customer is
    /// rejected provider-side.
    pub customer_ref: String,
    /// When the code stops being redeemable. `None` = no expiry.
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Maximum number of redemptions. `Some(1)` for single-use codes.
    pub max_redemptions: Option<u32>,
}

/// A minted promotion code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromotionCode {
    /// The customer-facing code string (what they type at checkout).
    pub code: String,
    /// Provider identifier for the promotion-code object, for audit
    /// trails and later deactivation.
    pub provider_promotion_id: String,
}

/// Promotion-code minting. An optional provider capability — query it via
/// [`super::PaymentProvider::as_promotions`], which returns `None` for
/// providers without promotion support.
#[async_trait]
pub trait Promotions: Send + Sync {
    /// Mint a customer-restricted promotion code off an existing
    /// provider-side coupon.
    async fn create_promotion_code(
        &self,
        req: CreatePromotionCodeRequest,
    ) -> PaymentResult<PromotionCode>;
}
