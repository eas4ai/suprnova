# Feature map: `manual/payments-provider-guide.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 44 checked.

## Rust API: suprnova

### `suprnova::payments::mock`

- [ ] struct `suprnova::MockPaymentProvider` · framework/src/payments/mock.rs:169 (also `suprnova::payments::MockPaymentProvider`, `suprnova::payments::mock::MockPaymentProvider`)
  - Implements: `suprnova::payments::Checkout`, `suprnova::payments::CustomerStore`, `suprnova::payments::PaymentProvider`, `suprnova::payments::Promotions`, `suprnova::payments::Subscription`, `suprnova::payments::WebhookHandler`
  - [ ] fn `suprnova::MockPaymentProvider::new` · framework/src/payments/mock.rs:184
  - [ ] fn `suprnova::MockPaymentProvider::script_session_status` · framework/src/payments/mock.rs:198
  - [ ] fn `suprnova::MockPaymentProvider::recorded_sessions` · framework/src/payments/mock.rs:211
  - [ ] fn `suprnova::MockPaymentProvider::recorded_promotion_requests` · framework/src/payments/mock.rs:216

### `suprnova::payments::traits::checkout`

- [ ] trait `suprnova::payments::Checkout` · framework/src/payments/traits/checkout.rs:16 (also `suprnova::payments::traits::Checkout`, `suprnova::payments::traits::checkout::Checkout`)
  - Implemented here by: `MockPaymentProvider`
  - [ ] fn `suprnova::payments::Checkout::start_session` · framework/src/payments/traits/checkout.rs:21 (required)
  - [ ] fn `suprnova::payments::Checkout::session_status` · framework/src/payments/traits/checkout.rs:34 (provided)

### `suprnova::payments::traits::customer`

- [ ] trait `suprnova::payments::CustomerStore` · framework/src/payments/traits/customer.rs:13 (also `suprnova::payments::traits::CustomerStore`, `suprnova::payments::traits::customer::CustomerStore`)
  - Implemented here by: `MockPaymentProvider`
  - [ ] fn `suprnova::payments::CustomerStore::create_customer` · framework/src/payments/traits/customer.rs:15 (required)
  - [ ] fn `suprnova::payments::CustomerStore::update_customer` · framework/src/payments/traits/customer.rs:18 (required)
  - [ ] fn `suprnova::payments::CustomerStore::get_customer` · framework/src/payments/traits/customer.rs:20 (required)
  - [ ] fn `suprnova::payments::CustomerStore::delete_customer` · framework/src/payments/traits/customer.rs:22 (required)

### `suprnova::payments::traits::payment`

- [ ] trait `suprnova::payments::Payment` · framework/src/payments/traits/payment.rs:11 (also `suprnova::payments::traits::Payment`, `suprnova::payments::traits::payment::Payment`)
  - [ ] fn `suprnova::payments::Payment::charge` · framework/src/payments/traits/payment.rs:15 (required)
  - [ ] fn `suprnova::payments::Payment::capture` · framework/src/payments/traits/payment.rs:17 (required)
  - [ ] fn `suprnova::payments::Payment::refund` · framework/src/payments/traits/payment.rs:19 (required)
  - [ ] fn `suprnova::payments::Payment::void` · framework/src/payments/traits/payment.rs:22 (required)
  - [ ] fn `suprnova::payments::Payment::status` · framework/src/payments/traits/payment.rs:24 (required)

### `suprnova::payments::traits::promotions`

- [ ] struct `suprnova::payments::CreatePromotionCodeRequest` · framework/src/payments/traits/promotions.rs:16 (also `suprnova::payments::traits::CreatePromotionCodeRequest`, `suprnova::payments::traits::promotions::CreatePromotionCodeRequest`)
  - Public fields: `coupon_ref`, `customer_ref`, `expires_at`, `max_redemptions`
- [ ] struct `suprnova::payments::PromotionCode` · framework/src/payments/traits/promotions.rs:32 (also `suprnova::payments::traits::PromotionCode`, `suprnova::payments::traits::promotions::PromotionCode`)
  - Public fields: `code`, `provider_promotion_id`
- [ ] trait `suprnova::payments::Promotions` · framework/src/payments/traits/promotions.rs:44 (also `suprnova::payments::traits::Promotions`, `suprnova::payments::traits::promotions::Promotions`)
  - Implemented here by: `MockPaymentProvider`
  - [ ] fn `suprnova::payments::Promotions::create_promotion_code` · framework/src/payments/traits/promotions.rs:47 (required)

### `suprnova::payments::traits::subscription`

- [ ] trait `suprnova::payments::Subscription` · framework/src/payments/traits/subscription.rs:15 (also `suprnova::payments::traits::Subscription`, `suprnova::payments::traits::subscription::Subscription`)
  - Implemented here by: `MockPaymentProvider`
  - [ ] fn `suprnova::payments::Subscription::subscribe` · framework/src/payments/traits/subscription.rs:17 (required)
  - [ ] fn `suprnova::payments::Subscription::update` · framework/src/payments/traits/subscription.rs:21 (required)
  - [ ] fn `suprnova::payments::Subscription::cancel` · framework/src/payments/traits/subscription.rs:25 (required)
  - [ ] fn `suprnova::payments::Subscription::get` · framework/src/payments/traits/subscription.rs:32 (required)

### `suprnova::payments::traits::webhook`

- [ ] fn `suprnova::payments::constant_time_eq` · framework/src/payments/traits/webhook.rs:35 (also `suprnova::payments::traits::constant_time_eq`, `suprnova::payments::traits::webhook::constant_time_eq`)
- [ ] struct `suprnova::payments::CustomerSnapshot` · framework/src/payments/traits/webhook.rs:178 (also `suprnova::payments::traits::CustomerSnapshot`, `suprnova::payments::traits::webhook::CustomerSnapshot`)
  - Public fields: `provider_customer_id`, `email`, `provider_metadata`
- [ ] struct `suprnova::payments::PayloadIds` · framework/src/payments/traits/webhook.rs:131 (also `suprnova::payments::traits::PayloadIds`, `suprnova::payments::traits::webhook::PayloadIds`)
  - Public fields: `subscription_id`, `customer_id`, `transaction_id`
- [ ] struct `suprnova::payments::PaymentSnapshot` · framework/src/payments/traits/webhook.rs:146 (also `suprnova::payments::traits::PaymentSnapshot`, `suprnova::payments::traits::webhook::PaymentSnapshot`)
  - Public fields: `provider_transaction_id`, `provider_customer_id`, `provider_subscription_id`, `amount_total_minor`, `amount_tax_minor`, `currency`, `status`, `paid_at`, `provider_metadata`
- [ ] trait `suprnova::payments::WebhookHandler` · framework/src/payments/traits/webhook.rs:49 (also `suprnova::payments::traits::WebhookHandler`, `suprnova::payments::traits::webhook::WebhookHandler`)
  - Implemented here by: `MockPaymentProvider`
  - [ ] fn `suprnova::payments::WebhookHandler::mirrors_payment_transactions` · framework/src/payments/traits/webhook.rs:57 (provided)
  - [ ] fn `suprnova::payments::WebhookHandler::verify` · framework/src/payments/traits/webhook.rs:73 (required)
  - [ ] fn `suprnova::payments::WebhookHandler::parse_event` · framework/src/payments/traits/webhook.rs:78 (required)
  - [ ] fn `suprnova::payments::WebhookHandler::extract_payload_ids` · framework/src/payments/traits/webhook.rs:86 (provided)
  - [ ] fn `suprnova::payments::WebhookHandler::extract_payment_snapshot` · framework/src/payments/traits/webhook.rs:95 (provided)
  - [ ] fn `suprnova::payments::WebhookHandler::try_extract_payment_snapshot` · framework/src/payments/traits/webhook.rs:108 (provided)
  - [ ] fn `suprnova::payments::WebhookHandler::extract_customer_snapshot` · framework/src/payments/traits/webhook.rs:122 (provided)

### `suprnova::payments::traits`

- [ ] trait `suprnova::payments::PaymentProvider` · framework/src/payments/traits/mod.rs:31 (also `suprnova::payments::traits::PaymentProvider`)
  - Implemented here by: `MockPaymentProvider`
  - [ ] fn `suprnova::payments::PaymentProvider::name` · framework/src/payments/traits/mod.rs:33 (required)
  - [ ] fn `suprnova::payments::PaymentProvider::as_payment` · framework/src/payments/traits/mod.rs:37 (provided)
  - [ ] fn `suprnova::payments::PaymentProvider::as_promotions` · framework/src/payments/traits/mod.rs:44 (provided)
