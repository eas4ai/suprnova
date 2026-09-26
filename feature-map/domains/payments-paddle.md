# Feature map: `manual/payments-paddle.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 6 checked.

## Rust API: suprnova-payments-paddle

### `suprnova_payments_paddle::event_map` (private module; items are public through re-exports)

- [ ] fn `suprnova_payments_paddle::paddle_event_to_neutral` · crates/suprnova-payments-paddle/src/event_map.rs:11

### `suprnova_payments_paddle`

- [ ] struct `suprnova_payments_paddle::PaddleProvider` · crates/suprnova-payments-paddle/src/lib.rs:38
  - Implements: `suprnova::payments::Checkout`, `suprnova::payments::CustomerStore`, `suprnova::payments::PaymentProvider`, `suprnova::payments::Subscription`, `suprnova::payments::WebhookHandler`
  - [ ] fn `suprnova_payments_paddle::PaddleProvider::new` · crates/suprnova-payments-paddle/src/lib.rs:88
  - [ ] fn `suprnova_payments_paddle::PaddleProvider::from_env` · crates/suprnova-payments-paddle/src/lib.rs:115
  - [ ] fn `suprnova_payments_paddle::PaddleProvider::environment` · crates/suprnova-payments-paddle/src/lib.rs:146
- [ ] enum `suprnova_payments_paddle::PaddleEnvironment` · crates/suprnova-payments-paddle/src/lib.rs:31
  - Variants: `Sandbox`, `Production`
