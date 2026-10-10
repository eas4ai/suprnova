//! Provider-trait surface for the payments subsystem.
//!
//! Each capability is its own trait - [`Checkout`] (start a session),
//! [`CustomerStore`] (CRUD a provider-side customer), [`Payment`]
//! (server-side capture), [`Subscription`] (manage recurring billing),
//! [`Promotions`] (mint promotion codes), and [`WebhookHandler`]
//! (verify + parse provider webhooks) - so providers opt in only to the
//! capabilities they actually support. The umbrella [`PaymentProvider`]
//! trait wires the required four together; `Payment` and `Promotions`
//! are queried separately via [`PaymentProvider::as_payment`] /
//! [`PaymentProvider::as_promotions`].

pub mod checkout;
pub mod customer;
pub mod payment;
pub mod promotions;
pub mod subscription;
pub mod webhook;

pub use checkout::Checkout;
pub use customer::CustomerStore;
pub use payment::Payment;
pub use promotions::{CreatePromotionCodeRequest, PromotionCode, Promotions};
pub use subscription::Subscription;
pub use webhook::{
    CustomerSnapshot, PayloadIds, PaymentSnapshot, WebhookHandler, constant_time_eq,
};

/// Umbrella trait every provider MUST implement. `Payment` is queried separately -
/// providers that don't expose server-capture omit that impl.
pub trait PaymentProvider: Checkout + Subscription + CustomerStore + WebhookHandler {
    /// Stable kebab-case identifier - e.g. "stripe", "paddle". Used for webhook routing.
    fn name(&self) -> &'static str;

    /// Returns `Some` if this provider also implements `Payment` (server-capture).
    /// Default impl returns `None`; implementers override when their type also impls `Payment`.
    fn as_payment(&self) -> Option<&dyn Payment> {
        None
    }

    /// Returns `Some` if this provider also implements `Promotions`
    /// (promotion-code minting). Default impl returns `None`; implementers
    /// override when their type also impls `Promotions`.
    fn as_promotions(&self) -> Option<&dyn Promotions> {
        None
    }
}
