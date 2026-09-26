# Suprnova feature map: `suprnova-payments-stripe` Rust API

Source: `crates/suprnova-payments-stripe/` at d03b4f1, rustdoc JSON (all features), cross-checked against a default-features build.

A checked box means the documentation for that item has been remediated
against the source. Items are listed under their shortest public path;
`also` names the other paths the same item is reachable by.

## Counts

- Top-level items (including re-exports): 3
- Members (methods, associated consts and types): 4
- By kind: constant 1, function 1, struct 1

## (crate root)

### `suprnova_payments_stripe`

- [ ] struct `suprnova_payments_stripe::StripeProvider` · crates/suprnova-payments-stripe/src/lib.rs:56
  - Implements: `suprnova::payments::Checkout`, `suprnova::payments::CustomerStore`, `suprnova::payments::Payment`, `suprnova::payments::PaymentProvider`, `suprnova::payments::Promotions`, `suprnova::payments::Subscription`, `suprnova::payments::WebhookHandler`
  - [ ] fn `suprnova_payments_stripe::StripeProvider::new` · crates/suprnova-payments-stripe/src/lib.rs:139
  - [ ] fn `suprnova_payments_stripe::StripeProvider::from_env` · crates/suprnova-payments-stripe/src/lib.rs:164
  - [ ] fn `suprnova_payments_stripe::StripeProvider::with_signature_tolerance` · crates/suprnova-payments-stripe/src/lib.rs:223
  - [ ] fn `suprnova_payments_stripe::StripeProvider::with_managed_payments` · crates/suprnova-payments-stripe/src/lib.rs:242
- [ ] const `suprnova_payments_stripe::DEFAULT_WEBHOOK_SIGNATURE_TOLERANCE_SECONDS` · crates/suprnova-payments-stripe/src/lib.rs:46

## event_map

### `suprnova_payments_stripe::event_map` (private module; items are public through re-exports)

- [ ] fn `suprnova_payments_stripe::stripe_event_to_neutral` · crates/suprnova-payments-stripe/src/event_map.rs:12
