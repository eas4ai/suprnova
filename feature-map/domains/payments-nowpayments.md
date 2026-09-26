# Feature map: `manual/payments-nowpayments.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 12 checked.

## Rust API: suprnova-payments-nowpayments

### `suprnova_payments_nowpayments::checkout` (private module; items are public through re-exports)

- [ ] struct `suprnova_payments_nowpayments::NowPaymentsInvoice` · crates/suprnova-payments-nowpayments/src/checkout.rs:12
  - Public fields: `invoice_id`, `invoice_url`, `order_id`
- [ ] enum `suprnova_payments_nowpayments::InvoiceCreationError` · crates/suprnova-payments-nowpayments/src/checkout.rs:23
  - Variants: `Rejected`, `Unknown`

### `suprnova_payments_nowpayments::status` (private module; items are public through re-exports)

- [ ] struct `suprnova_payments_nowpayments::NowPaymentsPayment` · crates/suprnova-payments-nowpayments/src/status.rs:68
  - Public fields: `payment_id`, `invoice_id`, `order_id`, `status`, `price`, `raw`
- [ ] enum `suprnova_payments_nowpayments::NowPaymentsStatus` · crates/suprnova-payments-nowpayments/src/status.rs:8
  - Variants: `Waiting`, `Confirming`, `Confirmed`, `Sending`, `Finished`, `PartiallyPaid`, `Failed`, `Expired`, `Refunded`, `Cancelled`, `Unknown`
  - [ ] fn `suprnova_payments_nowpayments::NowPaymentsStatus::neutral_event` · crates/suprnova-payments-nowpayments/src/status.rs:52

### `suprnova_payments_nowpayments`

- [ ] struct `suprnova_payments_nowpayments::NowPaymentsProvider` · crates/suprnova-payments-nowpayments/src/lib.rs:67
  - Implements: `suprnova::payments::Checkout`, `suprnova::payments::CustomerStore`, `suprnova::payments::PaymentProvider`, `suprnova::payments::Subscription`, `suprnova::payments::WebhookHandler`
  - [ ] fn `suprnova_payments_nowpayments::NowPaymentsProvider::create_invoice` · crates/suprnova-payments-nowpayments/src/checkout.rs:68
  - [ ] fn `suprnova_payments_nowpayments::NowPaymentsProvider::payment_status` · crates/suprnova-payments-nowpayments/src/status.rs:89
  - [ ] fn `suprnova_payments_nowpayments::NowPaymentsProvider::new` · crates/suprnova-payments-nowpayments/src/lib.rs:92
  - [ ] fn `suprnova_payments_nowpayments::NowPaymentsProvider::from_env` · crates/suprnova-payments-nowpayments/src/lib.rs:123
  - [ ] fn `suprnova_payments_nowpayments::NowPaymentsProvider::environment` · crates/suprnova-payments-nowpayments/src/lib.rs:148
- [ ] enum `suprnova_payments_nowpayments::NowPaymentsEnvironment` · crates/suprnova-payments-nowpayments/src/lib.rs:39
  - Variants: `Sandbox`, `Production`
