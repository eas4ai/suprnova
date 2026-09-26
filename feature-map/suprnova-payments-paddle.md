# Suprnova feature map: `suprnova-payments-paddle` Rust API

Source: `crates/suprnova-payments-paddle/` at d03b4f1, rustdoc JSON (all features), cross-checked against a default-features build.

A checked box means the documentation for that item has been remediated
against the source. Items are listed under their shortest public path;
`also` names the other paths the same item is reachable by.

## Counts

- Top-level items (including re-exports): 3
- Members (methods, associated consts and types): 3
- By kind: enum 1, function 1, struct 1

## (crate root)

### `suprnova_payments_paddle`

- [ ] struct `suprnova_payments_paddle::PaddleProvider` · crates/suprnova-payments-paddle/src/lib.rs:38
  - Implements: `suprnova::payments::Checkout`, `suprnova::payments::CustomerStore`, `suprnova::payments::PaymentProvider`, `suprnova::payments::Subscription`, `suprnova::payments::WebhookHandler`
  - [ ] fn `suprnova_payments_paddle::PaddleProvider::new` · crates/suprnova-payments-paddle/src/lib.rs:88
  - [ ] fn `suprnova_payments_paddle::PaddleProvider::from_env` · crates/suprnova-payments-paddle/src/lib.rs:115
  - [ ] fn `suprnova_payments_paddle::PaddleProvider::environment` · crates/suprnova-payments-paddle/src/lib.rs:146
- [ ] enum `suprnova_payments_paddle::PaddleEnvironment` · crates/suprnova-payments-paddle/src/lib.rs:31
  - Variants: `Sandbox`, `Production`

## event_map

### `suprnova_payments_paddle::event_map` (private module; items are public through re-exports)

- [ ] fn `suprnova_payments_paddle::paddle_event_to_neutral` · crates/suprnova-payments-paddle/src/event_map.rs:11
