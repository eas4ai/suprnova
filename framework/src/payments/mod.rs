//! Provider-neutral payments surface.
//!
//! See `manual/payments.md` for the user-facing guide.
//!
//! The types of the requests and the results are named here one by one.
//! Each has one path, `payments::ChargeRequest`, and the modules they are
//! written in are reached through [`dto`] and not from here:
//!
//! ```compile_fail
//! use suprnova::payments::session::StartSessionRequest;
//! ```
//!
//! A type that is added to [`dto`] is public here when it is named here.

pub mod dto;
pub mod entities;
pub mod error;
pub mod migrations;
pub mod mock;
pub mod money;
pub mod registry;
pub mod traits;
pub mod webhook_route;

pub use dto::{
    ChargeRequest, ChargeResult, CheckoutSessionState, CountryCode, CreateCustomerRequest,
    CustomerRef, MobileMoneyOperator, NeutralEventKind, PaymentMethod, PaymentStatus, PhoneNumber,
    RefundRequest, RefundResult, SessionMode, SessionPayload, StablecoinAsset, StartSessionRequest,
    SubscribeRequest, SubscriptionItemSnapshot, SubscriptionResult, SubscriptionStatus,
    UpdateCustomerRequest, UpdateSubscriptionRequest, WebhookContext, WebhookEvent,
};
pub use error::{PaymentError, PaymentResult};
pub use mock::MockPaymentProvider;
pub use money::{Currency, Money};
pub use registry::{PaymentProviderEntry, PaymentProviderRegistry};
pub use traits::{
    Checkout, CreatePromotionCodeRequest, CustomerSnapshot, CustomerStore, PayloadIds, Payment,
    PaymentProvider, PaymentSnapshot, PromotionCode, Promotions, Subscription, WebhookHandler,
    constant_time_eq,
};
pub use webhook_route::webhook_routes;
