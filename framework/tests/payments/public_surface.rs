//! The names of `suprnova::payments`. The module names its types one by
//! one, so a type that goes missing from the list is a type an application
//! can no longer name, and this file no longer compiles.

use suprnova::payments::{
    ChargeRequest, ChargeResult, CheckoutSessionState, CountryCode, CreateCustomerRequest,
    CustomerRef, MobileMoneyOperator, NeutralEventKind, PaymentMethod, PaymentStatus, PhoneNumber,
    Proration, RefundRequest, RefundResult, SessionMode, SessionPayload, StablecoinAsset,
    StartSessionRequest, SubscribeRequest, SubscriptionItemSnapshot, SubscriptionResult,
    SubscriptionStatus, UpdateCustomerRequest, UpdateSubscriptionRequest, WebhookContext,
    WebhookEvent,
};

/// A use of `T`, so that the import of it is no unused import.
fn named<T>() {}

/// That this function compiles is the test: each type is named by its path
/// in `payments`.
#[test]
fn every_type_of_the_requests_and_results_has_its_name_in_payments() {
    named::<ChargeRequest>();
    named::<ChargeResult>();
    named::<CheckoutSessionState>();
    named::<CountryCode>();
    named::<CreateCustomerRequest>();
    named::<CustomerRef>();
    named::<MobileMoneyOperator>();
    named::<NeutralEventKind>();
    named::<PaymentMethod>();
    named::<PaymentStatus>();
    named::<PhoneNumber>();
    named::<Proration>();
    named::<RefundRequest>();
    named::<RefundResult>();
    named::<SessionMode>();
    named::<SessionPayload>();
    named::<StablecoinAsset>();
    named::<StartSessionRequest>();
    named::<SubscribeRequest>();
    named::<SubscriptionItemSnapshot>();
    named::<SubscriptionResult>();
    named::<SubscriptionStatus>();
    named::<UpdateCustomerRequest>();
    named::<UpdateSubscriptionRequest>();
    named::<WebhookContext>();
    named::<WebhookEvent>();
}

/// The type of `payments` and the type of `payments::dto` are one type, so
/// a value of the one is a value of the other.
#[test]
fn the_names_of_payments_are_the_types_of_dto() {
    fn same<T: 'static, U: 'static>() -> bool {
        std::any::TypeId::of::<T>() == std::any::TypeId::of::<U>()
    }

    assert!(same::<ChargeRequest, suprnova::payments::dto::ChargeRequest>());
    assert!(same::<
        StartSessionRequest,
        suprnova::payments::dto::session::StartSessionRequest,
    >());
    assert!(same::<
        WebhookEvent,
        suprnova::payments::dto::webhook::WebhookEvent,
    >());
}
