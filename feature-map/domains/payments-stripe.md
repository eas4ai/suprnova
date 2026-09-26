# Feature map: `manual/payments-stripe.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 7 checked.

## Rust API: suprnova-payments-stripe

### `suprnova_payments_stripe::event_map` (private module; items are public through re-exports)

- [ ] fn `suprnova_payments_stripe::stripe_event_to_neutral` · crates/suprnova-payments-stripe/src/event_map.rs:12

### `suprnova_payments_stripe`

- [ ] struct `suprnova_payments_stripe::StripeProvider` · crates/suprnova-payments-stripe/src/lib.rs:56
  - Implements: `suprnova::payments::Checkout`, `suprnova::payments::CustomerStore`, `suprnova::payments::Payment`, `suprnova::payments::PaymentProvider`, `suprnova::payments::Promotions`, `suprnova::payments::Subscription`, `suprnova::payments::WebhookHandler`
  - [ ] fn `suprnova_payments_stripe::StripeProvider::new` · crates/suprnova-payments-stripe/src/lib.rs:139
  - [ ] fn `suprnova_payments_stripe::StripeProvider::from_env` · crates/suprnova-payments-stripe/src/lib.rs:164
  - [ ] fn `suprnova_payments_stripe::StripeProvider::with_signature_tolerance` · crates/suprnova-payments-stripe/src/lib.rs:223
  - [ ] fn `suprnova_payments_stripe::StripeProvider::with_managed_payments` · crates/suprnova-payments-stripe/src/lib.rs:242
- [ ] const `suprnova_payments_stripe::DEFAULT_WEBHOOK_SIGNATURE_TOLERANCE_SECONDS` · crates/suprnova-payments-stripe/src/lib.rs:46
