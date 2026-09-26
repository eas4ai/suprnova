# Feature map: `manual/payments.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 425 checked.

## Endpoints and tables

### HTTP endpoints the framework owns

- [ ] endpoint `/webhooks/payments/{provider}` · framework/src/payments/webhook_route.rs:1003
  - POST; `suprnova::payments::webhook_routes(db)`, merged by the app

### framework migration `CreatePaymentsTables`

- [ ] table `payments_customers (framework)` · framework/src/payments/migrations/m_2026_05_22_000001_create_payments_tables.rs:32
- [ ] table `payments_payment_methods (framework)` · framework/src/payments/migrations/m_2026_05_22_000001_create_payments_tables.rs:104
- [ ] table `payments_subscriptions (framework)` · framework/src/payments/migrations/m_2026_05_22_000001_create_payments_tables.rs:187
- [ ] table `payments_subscription_items (framework)` · framework/src/payments/migrations/m_2026_05_22_000001_create_payments_tables.rs:289
- [ ] table `payments_transactions (framework)` · framework/src/payments/migrations/m_2026_05_22_000001_create_payments_tables.rs:385
- [ ] table `payments_webhook_events (framework)` · framework/src/payments/migrations/m_2026_05_22_000001_create_payments_tables.rs:501

## Rust API: suprnova

### Re-exported from other crates

- [ ] enum `suprnova::Currency` re-exports `iso_currency::Currency`
- [ ] enum `suprnova::payments::Currency` re-exports `iso_currency::Currency`
- [ ] enum `suprnova::payments::money::Currency` re-exports `iso_currency::Currency`

### `suprnova::payments::dto::country`

- [ ] struct `suprnova::payments::CountryCode` · framework/src/payments/dto/country.rs:16 (also `suprnova::payments::country::CountryCode`, `suprnova::payments::dto::CountryCode`, `suprnova::payments::dto::country::CountryCode`)
  - [ ] fn `suprnova::payments::CountryCode::new` · framework/src/payments/dto/country.rs:27
  - [ ] fn `suprnova::payments::CountryCode::as_str` · framework/src/payments/dto/country.rs:38

### `suprnova::payments::dto::customer`

- [ ] struct `suprnova::payments::CreateCustomerRequest` · framework/src/payments/dto/customer.rs:35 (also `suprnova::payments::customer::CreateCustomerRequest`, `suprnova::payments::dto::CreateCustomerRequest`, `suprnova::payments::dto::customer::CreateCustomerRequest`)
  - Public fields: `user_id`, `email`, `name`, `metadata`
- [ ] struct `suprnova::payments::CustomerRef` · framework/src/payments/dto/customer.rs:20 (also `suprnova::payments::customer::CustomerRef`, `suprnova::payments::dto::CustomerRef`, `suprnova::payments::dto::customer::CustomerRef`)
  - Public fields: `provider_customer_id`, `user_id`, `email`, `provider_metadata`
- [ ] struct `suprnova::payments::UpdateCustomerRequest` · framework/src/payments/dto/customer.rs:49 (also `suprnova::payments::customer::UpdateCustomerRequest`, `suprnova::payments::dto::UpdateCustomerRequest`, `suprnova::payments::dto::customer::UpdateCustomerRequest`)
  - Public fields: `provider_customer_id`, `email`, `name`, `metadata`

### `suprnova::payments::dto::payment_method`

- [ ] enum `suprnova::payments::MobileMoneyOperator` · framework/src/payments/dto/payment_method.rs:80 (also `suprnova::payments::dto::MobileMoneyOperator`, `suprnova::payments::dto::payment_method::MobileMoneyOperator`, `suprnova::payments::payment_method::MobileMoneyOperator`)
  - Variants: `MtnMomo`, `Mpesa`, `AirtelMoney`, `OrangeMoney`, `Lipila`, `Custom`
- [ ] enum `suprnova::payments::PaymentMethod` · framework/src/payments/dto/payment_method.rs:10 (also `suprnova::payments::dto::PaymentMethod`, `suprnova::payments::dto::payment_method::PaymentMethod`, `suprnova::payments::payment_method::PaymentMethod`)
  - Variants: `Card`, `BankTransfer`, `EWallet`, `MobileMoney`, `Stablecoin`, `Crypto`, `Custom`
- [ ] enum `suprnova::payments::StablecoinAsset` · framework/src/payments/dto/payment_method.rs:103 (also `suprnova::payments::dto::StablecoinAsset`, `suprnova::payments::dto::payment_method::StablecoinAsset`, `suprnova::payments::payment_method::StablecoinAsset`)
  - Variants: `Usdc`, `Usdt`, `Dai`, `Custom`

### `suprnova::payments::dto::payment`

- [ ] struct `suprnova::payments::ChargeRequest` · framework/src/payments/dto/payment.rs:40 (also `suprnova::payments::dto::ChargeRequest`, `suprnova::payments::dto::payment::ChargeRequest`, `suprnova::payments::payment::ChargeRequest`)
  - Public fields: `customer_ref`, `payment_method_ref`, `amount`, `description`, `idempotency_key`, `metadata`
- [ ] struct `suprnova::payments::RefundRequest` · framework/src/payments/dto/payment.rs:105 (also `suprnova::payments::dto::RefundRequest`, `suprnova::payments::dto::payment::RefundRequest`, `suprnova::payments::payment::RefundRequest`)
  - Public fields: `provider_transaction_id`, `amount`, `reason`, `idempotency_key`
- [ ] struct `suprnova::payments::RefundResult` · framework/src/payments/dto/payment.rs:120 (also `suprnova::payments::dto::RefundResult`, `suprnova::payments::dto::payment::RefundResult`, `suprnova::payments::payment::RefundResult`)
  - Public fields: `provider_refund_id`, `provider_transaction_id`, `amount`, `provider_metadata`
- [ ] enum `suprnova::payments::ChargeResult` · framework/src/payments/dto/payment.rs:62 (also `suprnova::payments::dto::ChargeResult`, `suprnova::payments::dto::payment::ChargeResult`, `suprnova::payments::payment::ChargeResult`)
  - Variants: `Completed`, `RedirectRequired`, `RequiresClientAction`
- [ ] enum `suprnova::payments::PaymentStatus` · framework/src/payments/dto/payment.rs:11 (also `suprnova::payments::dto::PaymentStatus`, `suprnova::payments::dto::payment::PaymentStatus`, `suprnova::payments::payment::PaymentStatus`)
  - Variants: `Created`, `RequiresAction`, `Pending`, `Processing`, `Authorized`, `Expired`, `Succeeded`, `Failed`, `Canceled`, `Refunded`, `PartiallyRefunded`, `Disputed`

### `suprnova::payments::dto::phone`

- [ ] struct `suprnova::payments::PhoneNumber` · framework/src/payments/dto/phone.rs:23 (also `suprnova::payments::dto::PhoneNumber`, `suprnova::payments::dto::phone::PhoneNumber`, `suprnova::payments::phone::PhoneNumber`)
  - [ ] fn `suprnova::payments::PhoneNumber::new` · framework/src/payments/dto/phone.rs:33
  - [ ] fn `suprnova::payments::PhoneNumber::as_e164` · framework/src/payments/dto/phone.rs:45
  - [ ] fn `suprnova::payments::PhoneNumber::digits` · framework/src/payments/dto/phone.rs:52

### `suprnova::payments::dto::session`

- [ ] struct `suprnova::payments::StartSessionRequest` · framework/src/payments/dto/session.rs:21 (also `suprnova::payments::dto::StartSessionRequest`, `suprnova::payments::dto::session::StartSessionRequest`, `suprnova::payments::session::StartSessionRequest`)
  - Public fields: `mode`, `customer_ref`, `price_refs`, `success_return_url`, `cancel_return_url`, `amount_hint`, `idempotency_key`, `metadata`
- [ ] enum `suprnova::payments::CheckoutSessionState` · framework/src/payments/dto/session.rs:48 (also `suprnova::payments::dto::CheckoutSessionState`, `suprnova::payments::dto::session::CheckoutSessionState`, `suprnova::payments::session::CheckoutSessionState`)
  - Variants: `Open`, `Complete`, `Expired`
- [ ] enum `suprnova::payments::SessionMode` · framework/src/payments/dto/session.rs:12 (also `suprnova::payments::dto::SessionMode`, `suprnova::payments::dto::session::SessionMode`, `suprnova::payments::session::SessionMode`)
  - Variants: `OneOff`, `Subscription`
- [ ] enum `suprnova::payments::SessionPayload` · framework/src/payments/dto/session.rs:73 (also `suprnova::payments::dto::SessionPayload`, `suprnova::payments::dto::session::SessionPayload`, `suprnova::payments::session::SessionPayload`)
  - Variants: `StripeElements`, `StripeCheckoutRedirect`, `PaddleInline`, `MobileMoneyPrompt`, `Redirect`

### `suprnova::payments::dto::subscription`

- [ ] struct `suprnova::payments::SubscribeRequest` · framework/src/payments/dto/subscription.rs:29 (also `suprnova::payments::dto::SubscribeRequest`, `suprnova::payments::dto::subscription::SubscribeRequest`, `suprnova::payments::subscription::SubscribeRequest`)
  - Public fields: `customer_ref`, `price_refs`, `trial_days`, `idempotency_key`, `metadata`
- [ ] struct `suprnova::payments::SubscriptionItemSnapshot` · framework/src/payments/dto/subscription.rs:82 (also `suprnova::payments::dto::SubscriptionItemSnapshot`, `suprnova::payments::dto::subscription::SubscriptionItemSnapshot`, `suprnova::payments::subscription::SubscriptionItemSnapshot`)
  - Public fields: `provider_item_id`, `provider_price_id`, `quantity`, `unit_amount`
- [ ] struct `suprnova::payments::SubscriptionResult` · framework/src/payments/dto/subscription.rs:60 (also `suprnova::payments::dto::SubscriptionResult`, `suprnova::payments::dto::subscription::SubscriptionResult`, `suprnova::payments::subscription::SubscriptionResult`)
  - Public fields: `provider_subscription_id`, `provider_customer_id`, `status`, `items`, `current_period_start`, `current_period_end`, `cancel_at_period_end`, `provider_metadata`
- [ ] struct `suprnova::payments::UpdateSubscriptionRequest` · framework/src/payments/dto/subscription.rs:45 (also `suprnova::payments::dto::UpdateSubscriptionRequest`, `suprnova::payments::dto::subscription::UpdateSubscriptionRequest`, `suprnova::payments::subscription::UpdateSubscriptionRequest`)
  - Public fields: `provider_subscription_id`, `new_price_refs`, `cancel_at_period_end`, `idempotency_key`
- [ ] enum `suprnova::payments::SubscriptionStatus` · framework/src/payments/dto/subscription.rs:12 (also `suprnova::payments::dto::SubscriptionStatus`, `suprnova::payments::dto::subscription::SubscriptionStatus`, `suprnova::payments::subscription::SubscriptionStatus`)
  - Variants: `Trialing`, `Active`, `PastDue`, `Canceled`, `Incomplete`, `Paused`

### `suprnova::payments::dto::webhook`

- [ ] struct `suprnova::payments::WebhookContext` · framework/src/payments/dto/webhook.rs:59 (also `suprnova::payments::dto::WebhookContext`, `suprnova::payments::dto::webhook::WebhookContext`, `suprnova::payments::webhook::WebhookContext`)
  - Public fields: `body`, `headers`, `remote_addr`
- [ ] struct `suprnova::payments::WebhookEvent` · framework/src/payments/dto/webhook.rs:39 (also `suprnova::payments::dto::WebhookEvent`, `suprnova::payments::dto::webhook::WebhookEvent`, `suprnova::payments::webhook::WebhookEvent`)
  - Public fields: `provider`, `provider_event_id`, `provider_event_type`, `neutral`, `raw_payload`
- [ ] enum `suprnova::payments::NeutralEventKind` · framework/src/payments/dto/webhook.rs:11 (also `suprnova::payments::dto::NeutralEventKind`, `suprnova::payments::dto::webhook::NeutralEventKind`, `suprnova::payments::webhook::NeutralEventKind`)
  - Variants: `PaymentSucceeded`, `PaymentFailed`, `PaymentRefunded`, `PaymentDisputed`, `SubscriptionCreated`, `SubscriptionUpdated`, `SubscriptionCanceled`, `InvoicePaid`, `InvoiceFailed`, `CustomerCreated`, `CustomerUpdated`

### `suprnova::payments::entities::customer::customer::events`

- [ ] struct `suprnova::payments::entities::customer::customer::events::Created` · framework/src/payments/entities/customer.rs:14
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::customer::customer::events::Creating` · framework/src/payments/entities/customer.rs:14
  - Public fields: `attrs`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::customer::customer::events::Deleted` · framework/src/payments/entities/customer.rs:14
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::customer::customer::events::Deleting` · framework/src/payments/entities/customer.rs:14
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::customer::customer::events::ForceDeleted` · framework/src/payments/entities/customer.rs:14
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::customer::customer::events::ForceDeleting` · framework/src/payments/entities/customer.rs:14
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::customer::customer::events::Replicating` · framework/src/payments/entities/customer.rs:14
  - Public fields: `source`, `replica`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::customer::customer::events::Restored` · framework/src/payments/entities/customer.rs:14
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::customer::customer::events::Restoring` · framework/src/payments/entities/customer.rs:14
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::customer::customer::events::Retrieved` · framework/src/payments/entities/customer.rs:14
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::customer::customer::events::Retrieving` · framework/src/payments/entities/customer.rs:14
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::customer::customer::events::Saved` · framework/src/payments/entities/customer.rs:14
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::customer::customer::events::Saving` · framework/src/payments/entities/customer.rs:14
  - Public fields: `attrs`, `is_creating`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::customer::customer::events::Trashed` · framework/src/payments/entities/customer.rs:14
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::customer::customer::events::Updated` · framework/src/payments/entities/customer.rs:14
  - Public fields: `previous`, `current`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::customer::customer::events::Updating` · framework/src/payments/entities/customer.rs:14
  - Public fields: `previous`, `attrs`
  - Implements: `suprnova::Event`

### `suprnova::payments::entities::customer::customer`

- [ ] struct `suprnova::payments::entities::customer::ActiveModel` · framework/src/payments/entities/customer.rs:14 (also `suprnova::payments::entities::customer::customer::ActiveModel`)
  - Public fields: `id`, `provider`, `provider_customer_id`, `user_id`, `email`, `provider_metadata`, `created_at`, `updated_at`
- [ ] struct `suprnova::payments::entities::customer::customer::ColumnIter` · framework/src/payments/entities/customer.rs:14
- [ ] struct `suprnova::payments::entities::customer::Entity` · framework/src/payments/entities/customer.rs:14 (also `suprnova::payments::entities::customer::customer::Entity`)
- [ ] struct `suprnova::payments::entities::customer::Model` · framework/src/payments/entities/customer.rs:14 (also `suprnova::payments::entities::customer::customer::Model`)
  - Public fields: `id`, `provider`, `provider_customer_id`, `user_id`, `email`, `provider_metadata`, `created_at`, `updated_at`
  - [ ] fn `suprnova::payments::entities::customer::Model::into_ex` · framework/src/payments/entities/customer.rs:14
- [ ] struct `suprnova::payments::entities::customer::customer::PrimaryKeyIter` · framework/src/payments/entities/customer.rs:14
- [ ] struct `suprnova::payments::entities::customer::customer::RelationIter` · framework/src/payments/entities/customer.rs:14
- [ ] enum `suprnova::payments::entities::customer::Column` · framework/src/payments/entities/customer.rs:14 (also `suprnova::payments::entities::customer::customer::Column`)
  - Variants: `Id`, `Provider`, `ProviderCustomerId`, `UserId`, `Email`, `ProviderMetadata`, `CreatedAt`, `UpdatedAt`
  - Implements: `suprnova::IntoColumn`
  - [ ] fn `suprnova::payments::entities::customer::Column::as_str` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Column::from_name` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Column::iter` · framework/src/payments/entities/customer.rs:14
- [ ] enum `suprnova::payments::entities::customer::customer::PrimaryKey` · framework/src/payments/entities/customer.rs:14
  - Variants: `Id`
- [ ] enum `suprnova::payments::entities::customer::customer::Relation` · framework/src/payments/entities/customer.rs:14
- [ ] type `suprnova::payments::entities::customer::customer::__Suprnova_Cast_Storage_created_at` · framework/src/payments/entities/customer.rs:14
- [ ] type `suprnova::payments::entities::customer::customer::__Suprnova_Cast_Storage_updated_at` · framework/src/payments/entities/customer.rs:14

### `suprnova::payments::entities::customer`

- [ ] struct `suprnova::payments::entities::customer::Customer` · framework/src/payments/entities/customer.rs:15
  - Public fields: `id`, `provider`, `provider_customer_id`, `user_id`, `email`, `provider_metadata`, `created_at`, `updated_at`
  - Implements: `suprnova::EagerLoadDispatch`, `suprnova::EloquentModel`, `suprnova::FirstOrCreate`, `suprnova::Model`, `suprnova::ModelEventHooks`, `suprnova::Persistable`, `suprnova::ReplicateExt`, `suprnova::Touchable`
  - [ ] fn `suprnova::payments::entities::customer::Customer::fill` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::without_global_scope` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::without_global_scopes` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::on` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::on_write_connection` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::count` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::sum` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::avg` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::min` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::max` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::pluck` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::pluck_keyed` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::filter` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::db_where` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::where_in` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::where_like` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::latest` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::oldest` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::pivot` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::with` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::with_count` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::with_sum` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::with_avg` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::with_min` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::with_max` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::observe` · framework/src/payments/entities/customer.rs:14

### `suprnova::payments::entities::payment_method::payment_method::events`

- [ ] struct `suprnova::payments::entities::payment_method::payment_method::events::Created` · framework/src/payments/entities/payment_method.rs:15
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::payment_method::payment_method::events::Creating` · framework/src/payments/entities/payment_method.rs:15
  - Public fields: `attrs`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::payment_method::payment_method::events::Deleted` · framework/src/payments/entities/payment_method.rs:15
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::payment_method::payment_method::events::Deleting` · framework/src/payments/entities/payment_method.rs:15
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::payment_method::payment_method::events::ForceDeleted` · framework/src/payments/entities/payment_method.rs:15
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::payment_method::payment_method::events::ForceDeleting` · framework/src/payments/entities/payment_method.rs:15
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::payment_method::payment_method::events::Replicating` · framework/src/payments/entities/payment_method.rs:15
  - Public fields: `source`, `replica`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::payment_method::payment_method::events::Restored` · framework/src/payments/entities/payment_method.rs:15
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::payment_method::payment_method::events::Restoring` · framework/src/payments/entities/payment_method.rs:15
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::payment_method::payment_method::events::Retrieved` · framework/src/payments/entities/payment_method.rs:15
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::payment_method::payment_method::events::Retrieving` · framework/src/payments/entities/payment_method.rs:15
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::payment_method::payment_method::events::Saved` · framework/src/payments/entities/payment_method.rs:15
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::payment_method::payment_method::events::Saving` · framework/src/payments/entities/payment_method.rs:15
  - Public fields: `attrs`, `is_creating`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::payment_method::payment_method::events::Trashed` · framework/src/payments/entities/payment_method.rs:15
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::payment_method::payment_method::events::Updated` · framework/src/payments/entities/payment_method.rs:15
  - Public fields: `previous`, `current`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::payment_method::payment_method::events::Updating` · framework/src/payments/entities/payment_method.rs:15
  - Public fields: `previous`, `attrs`
  - Implements: `suprnova::Event`

### `suprnova::payments::entities::payment_method::payment_method`

- [ ] struct `suprnova::payments::entities::payment_method::ActiveModel` · framework/src/payments/entities/payment_method.rs:15 (also `suprnova::payments::entities::payment_method::payment_method::ActiveModel`)
  - Public fields: `id`, `provider`, `provider_payment_method_id`, `provider_customer_id`, `method_type`, `method_details`, `is_default`, `provider_metadata`, `created_at`, `updated_at`
- [ ] struct `suprnova::payments::entities::payment_method::payment_method::ColumnIter` · framework/src/payments/entities/payment_method.rs:15
- [ ] struct `suprnova::payments::entities::payment_method::Entity` · framework/src/payments/entities/payment_method.rs:15 (also `suprnova::payments::entities::payment_method::payment_method::Entity`)
- [ ] struct `suprnova::payments::entities::payment_method::Model` · framework/src/payments/entities/payment_method.rs:15 (also `suprnova::payments::entities::payment_method::payment_method::Model`)
  - Public fields: `id`, `provider`, `provider_payment_method_id`, `provider_customer_id`, `method_type`, `method_details`, `is_default`, `provider_metadata`, `created_at`, `updated_at`
  - [ ] fn `suprnova::payments::entities::payment_method::Model::into_ex` · framework/src/payments/entities/payment_method.rs:15
- [ ] struct `suprnova::payments::entities::payment_method::payment_method::PrimaryKeyIter` · framework/src/payments/entities/payment_method.rs:15
- [ ] struct `suprnova::payments::entities::payment_method::payment_method::RelationIter` · framework/src/payments/entities/payment_method.rs:15
- [ ] enum `suprnova::payments::entities::payment_method::Column` · framework/src/payments/entities/payment_method.rs:15 (also `suprnova::payments::entities::payment_method::payment_method::Column`)
  - Variants: `Id`, `Provider`, `ProviderPaymentMethodId`, `ProviderCustomerId`, `MethodType`, `MethodDetails`, `IsDefault`, `ProviderMetadata`, `CreatedAt`, `UpdatedAt`
  - Implements: `suprnova::IntoColumn`
  - [ ] fn `suprnova::payments::entities::payment_method::Column::as_str` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::Column::from_name` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::Column::iter` · framework/src/payments/entities/payment_method.rs:15
- [ ] enum `suprnova::payments::entities::payment_method::payment_method::PrimaryKey` · framework/src/payments/entities/payment_method.rs:15
  - Variants: `Id`
- [ ] enum `suprnova::payments::entities::payment_method::payment_method::Relation` · framework/src/payments/entities/payment_method.rs:15
- [ ] type `suprnova::payments::entities::payment_method::payment_method::__Suprnova_Cast_Storage_created_at` · framework/src/payments/entities/payment_method.rs:15
- [ ] type `suprnova::payments::entities::payment_method::payment_method::__Suprnova_Cast_Storage_updated_at` · framework/src/payments/entities/payment_method.rs:15

### `suprnova::payments::entities::payment_method`

- [ ] struct `suprnova::payments::entities::payment_method::PaymentMethod` · framework/src/payments/entities/payment_method.rs:16
  - Public fields: `id`, `provider`, `provider_payment_method_id`, `provider_customer_id`, `method_type`, `method_details`, `is_default`, `provider_metadata`, `created_at`, `updated_at`
  - Implements: `suprnova::EagerLoadDispatch`, `suprnova::EloquentModel`, `suprnova::FirstOrCreate`, `suprnova::Model`, `suprnova::ModelEventHooks`, `suprnova::Persistable`, `suprnova::ReplicateExt`, `suprnova::Touchable`
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::fill` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::without_global_scope` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::without_global_scopes` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::on` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::on_write_connection` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::count` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::sum` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::avg` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::min` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::max` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::pluck` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::pluck_keyed` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::filter` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::db_where` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::where_in` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::where_like` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::latest` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::oldest` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::pivot` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::with` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::with_count` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::with_sum` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::with_avg` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::with_min` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::with_max` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::observe` · framework/src/payments/entities/payment_method.rs:15

### `suprnova::payments::entities::subscription::subscription::events`

- [ ] struct `suprnova::payments::entities::subscription::subscription::events::Created` · framework/src/payments/entities/subscription.rs:14
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription::subscription::events::Creating` · framework/src/payments/entities/subscription.rs:14
  - Public fields: `attrs`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription::subscription::events::Deleted` · framework/src/payments/entities/subscription.rs:14
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription::subscription::events::Deleting` · framework/src/payments/entities/subscription.rs:14
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription::subscription::events::ForceDeleted` · framework/src/payments/entities/subscription.rs:14
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription::subscription::events::ForceDeleting` · framework/src/payments/entities/subscription.rs:14
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription::subscription::events::Replicating` · framework/src/payments/entities/subscription.rs:14
  - Public fields: `source`, `replica`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription::subscription::events::Restored` · framework/src/payments/entities/subscription.rs:14
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription::subscription::events::Restoring` · framework/src/payments/entities/subscription.rs:14
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription::subscription::events::Retrieved` · framework/src/payments/entities/subscription.rs:14
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription::subscription::events::Retrieving` · framework/src/payments/entities/subscription.rs:14
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription::subscription::events::Saved` · framework/src/payments/entities/subscription.rs:14
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription::subscription::events::Saving` · framework/src/payments/entities/subscription.rs:14
  - Public fields: `attrs`, `is_creating`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription::subscription::events::Trashed` · framework/src/payments/entities/subscription.rs:14
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription::subscription::events::Updated` · framework/src/payments/entities/subscription.rs:14
  - Public fields: `previous`, `current`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription::subscription::events::Updating` · framework/src/payments/entities/subscription.rs:14
  - Public fields: `previous`, `attrs`
  - Implements: `suprnova::Event`

### `suprnova::payments::entities::subscription::subscription`

- [ ] struct `suprnova::payments::entities::subscription::ActiveModel` · framework/src/payments/entities/subscription.rs:14 (also `suprnova::payments::entities::subscription::subscription::ActiveModel`)
  - Public fields: `id`, `provider`, `provider_subscription_id`, `provider_customer_id`, `status`, `current_period_start`, `current_period_end`, `cancel_at_period_end`, `canceled_at`, `provider_metadata`, `created_at`, `updated_at`
- [ ] struct `suprnova::payments::entities::subscription::subscription::ColumnIter` · framework/src/payments/entities/subscription.rs:14
- [ ] struct `suprnova::payments::entities::subscription::Entity` · framework/src/payments/entities/subscription.rs:14 (also `suprnova::payments::entities::subscription::subscription::Entity`)
- [ ] struct `suprnova::payments::entities::subscription::Model` · framework/src/payments/entities/subscription.rs:14 (also `suprnova::payments::entities::subscription::subscription::Model`)
  - Public fields: `id`, `provider`, `provider_subscription_id`, `provider_customer_id`, `status`, `current_period_start`, `current_period_end`, `cancel_at_period_end`, `canceled_at`, `provider_metadata`, `created_at`, `updated_at`
  - [ ] fn `suprnova::payments::entities::subscription::Model::into_ex` · framework/src/payments/entities/subscription.rs:14
- [ ] struct `suprnova::payments::entities::subscription::subscription::PrimaryKeyIter` · framework/src/payments/entities/subscription.rs:14
- [ ] struct `suprnova::payments::entities::subscription::subscription::RelationIter` · framework/src/payments/entities/subscription.rs:14
- [ ] enum `suprnova::payments::entities::subscription::Column` · framework/src/payments/entities/subscription.rs:14 (also `suprnova::payments::entities::subscription::subscription::Column`)
  - Variants: `Id`, `Provider`, `ProviderSubscriptionId`, `ProviderCustomerId`, `Status`, `CurrentPeriodStart`, `CurrentPeriodEnd`, `CancelAtPeriodEnd`, `CanceledAt`, `ProviderMetadata`, `CreatedAt`, `UpdatedAt`
  - Implements: `suprnova::IntoColumn`
  - [ ] fn `suprnova::payments::entities::subscription::Column::as_str` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Column::from_name` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Column::iter` · framework/src/payments/entities/subscription.rs:14
- [ ] enum `suprnova::payments::entities::subscription::subscription::PrimaryKey` · framework/src/payments/entities/subscription.rs:14
  - Variants: `Id`
- [ ] enum `suprnova::payments::entities::subscription::subscription::Relation` · framework/src/payments/entities/subscription.rs:14
- [ ] type `suprnova::payments::entities::subscription::subscription::__Suprnova_Cast_Storage_canceled_at` · framework/src/payments/entities/subscription.rs:14
- [ ] type `suprnova::payments::entities::subscription::subscription::__Suprnova_Cast_Storage_created_at` · framework/src/payments/entities/subscription.rs:14
- [ ] type `suprnova::payments::entities::subscription::subscription::__Suprnova_Cast_Storage_current_period_end` · framework/src/payments/entities/subscription.rs:14
- [ ] type `suprnova::payments::entities::subscription::subscription::__Suprnova_Cast_Storage_current_period_start` · framework/src/payments/entities/subscription.rs:14
- [ ] type `suprnova::payments::entities::subscription::subscription::__Suprnova_Cast_Storage_updated_at` · framework/src/payments/entities/subscription.rs:14

### `suprnova::payments::entities::subscription_item::subscription_item::events`

- [ ] struct `suprnova::payments::entities::subscription_item::subscription_item::events::Created` · framework/src/payments/entities/subscription_item.rs:12
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription_item::subscription_item::events::Creating` · framework/src/payments/entities/subscription_item.rs:12
  - Public fields: `attrs`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription_item::subscription_item::events::Deleted` · framework/src/payments/entities/subscription_item.rs:12
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription_item::subscription_item::events::Deleting` · framework/src/payments/entities/subscription_item.rs:12
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription_item::subscription_item::events::ForceDeleted` · framework/src/payments/entities/subscription_item.rs:12
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription_item::subscription_item::events::ForceDeleting` · framework/src/payments/entities/subscription_item.rs:12
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription_item::subscription_item::events::Replicating` · framework/src/payments/entities/subscription_item.rs:12
  - Public fields: `source`, `replica`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription_item::subscription_item::events::Restored` · framework/src/payments/entities/subscription_item.rs:12
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription_item::subscription_item::events::Restoring` · framework/src/payments/entities/subscription_item.rs:12
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription_item::subscription_item::events::Retrieved` · framework/src/payments/entities/subscription_item.rs:12
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription_item::subscription_item::events::Retrieving` · framework/src/payments/entities/subscription_item.rs:12
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription_item::subscription_item::events::Saved` · framework/src/payments/entities/subscription_item.rs:12
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription_item::subscription_item::events::Saving` · framework/src/payments/entities/subscription_item.rs:12
  - Public fields: `attrs`, `is_creating`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription_item::subscription_item::events::Trashed` · framework/src/payments/entities/subscription_item.rs:12
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription_item::subscription_item::events::Updated` · framework/src/payments/entities/subscription_item.rs:12
  - Public fields: `previous`, `current`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription_item::subscription_item::events::Updating` · framework/src/payments/entities/subscription_item.rs:12
  - Public fields: `previous`, `attrs`
  - Implements: `suprnova::Event`

### `suprnova::payments::entities::subscription_item::subscription_item`

- [ ] struct `suprnova::payments::entities::subscription_item::ActiveModel` · framework/src/payments/entities/subscription_item.rs:12 (also `suprnova::payments::entities::subscription_item::subscription_item::ActiveModel`)
  - Public fields: `id`, `subscription_id`, `provider_item_id`, `provider_price_id`, `quantity`, `unit_amount_minor`, `unit_currency`, `provider_metadata`, `created_at`, `updated_at`
- [ ] struct `suprnova::payments::entities::subscription_item::subscription_item::ColumnIter` · framework/src/payments/entities/subscription_item.rs:12
- [ ] struct `suprnova::payments::entities::subscription_item::Entity` · framework/src/payments/entities/subscription_item.rs:12 (also `suprnova::payments::entities::subscription_item::subscription_item::Entity`)
- [ ] struct `suprnova::payments::entities::subscription_item::Model` · framework/src/payments/entities/subscription_item.rs:12 (also `suprnova::payments::entities::subscription_item::subscription_item::Model`)
  - Public fields: `id`, `subscription_id`, `provider_item_id`, `provider_price_id`, `quantity`, `unit_amount_minor`, `unit_currency`, `provider_metadata`, `created_at`, `updated_at`
  - [ ] fn `suprnova::payments::entities::subscription_item::Model::into_ex` · framework/src/payments/entities/subscription_item.rs:12
- [ ] struct `suprnova::payments::entities::subscription_item::subscription_item::PrimaryKeyIter` · framework/src/payments/entities/subscription_item.rs:12
- [ ] struct `suprnova::payments::entities::subscription_item::subscription_item::RelationIter` · framework/src/payments/entities/subscription_item.rs:12
- [ ] enum `suprnova::payments::entities::subscription_item::Column` · framework/src/payments/entities/subscription_item.rs:12 (also `suprnova::payments::entities::subscription_item::subscription_item::Column`)
  - Variants: `Id`, `SubscriptionId`, `ProviderItemId`, `ProviderPriceId`, `Quantity`, `UnitAmountMinor`, `UnitCurrency`, `ProviderMetadata`, `CreatedAt`, `UpdatedAt`
  - Implements: `suprnova::IntoColumn`
  - [ ] fn `suprnova::payments::entities::subscription_item::Column::as_str` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::Column::from_name` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::Column::iter` · framework/src/payments/entities/subscription_item.rs:12
- [ ] enum `suprnova::payments::entities::subscription_item::subscription_item::PrimaryKey` · framework/src/payments/entities/subscription_item.rs:12
  - Variants: `Id`
- [ ] enum `suprnova::payments::entities::subscription_item::subscription_item::Relation` · framework/src/payments/entities/subscription_item.rs:12
- [ ] type `suprnova::payments::entities::subscription_item::subscription_item::__Suprnova_Cast_Storage_created_at` · framework/src/payments/entities/subscription_item.rs:12
- [ ] type `suprnova::payments::entities::subscription_item::subscription_item::__Suprnova_Cast_Storage_updated_at` · framework/src/payments/entities/subscription_item.rs:12

### `suprnova::payments::entities::subscription_item`

- [ ] struct `suprnova::payments::entities::subscription_item::SubscriptionItem` · framework/src/payments/entities/subscription_item.rs:19
  - Public fields: `id`, `subscription_id`, `provider_item_id`, `provider_price_id`, `quantity`, `unit_amount_minor`, `unit_currency`, `provider_metadata`, `created_at`, `updated_at`
  - Implements: `suprnova::EagerLoadDispatch`, `suprnova::EloquentModel`, `suprnova::FirstOrCreate`, `suprnova::Model`, `suprnova::ModelEventHooks`, `suprnova::Persistable`, `suprnova::ReplicateExt`, `suprnova::Touchable`
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::fill` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::without_global_scope` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::without_global_scopes` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::on` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::on_write_connection` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::count` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::sum` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::avg` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::min` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::max` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::pluck` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::pluck_keyed` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::filter` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::db_where` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::where_in` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::where_like` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::latest` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::oldest` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::pivot` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::with` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::with_count` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::with_sum` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::with_avg` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::with_min` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::with_max` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::subscription` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::subscription_loaded` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::subscription_count` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::subscription_sum_of` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::subscription_avg_of` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::subscription_min_of` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::subscription_max_of` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::with_where_subscription` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::observe` · framework/src/payments/entities/subscription_item.rs:12

### `suprnova::payments::entities::subscription`

- [ ] struct `suprnova::payments::entities::subscription::Subscription` · framework/src/payments/entities/subscription.rs:21
  - Public fields: `id`, `provider`, `provider_subscription_id`, `provider_customer_id`, `status`, `current_period_start`, `current_period_end`, `cancel_at_period_end`, `canceled_at`, `provider_metadata`, `created_at`, `updated_at`
  - Implements: `suprnova::EagerLoadDispatch`, `suprnova::EloquentModel`, `suprnova::FirstOrCreate`, `suprnova::Model`, `suprnova::ModelEventHooks`, `suprnova::Persistable`, `suprnova::ReplicateExt`, `suprnova::Touchable`
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::fill` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::without_global_scope` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::without_global_scopes` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::on` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::on_write_connection` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::count` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::sum` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::avg` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::min` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::max` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::pluck` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::pluck_keyed` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::filter` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::db_where` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::where_in` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::where_like` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::latest` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::oldest` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::pivot` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::with` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::with_count` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::with_sum` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::with_avg` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::with_min` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::with_max` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::items` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::items_loaded` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::items_count` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::items_sum_of` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::items_avg_of` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::items_min_of` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::items_max_of` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::with_where_items` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::observe` · framework/src/payments/entities/subscription.rs:14

### `suprnova::payments::entities::transaction::transaction::events`

- [ ] struct `suprnova::payments::entities::transaction::transaction::events::Created` · framework/src/payments/entities/transaction.rs:16
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::transaction::transaction::events::Creating` · framework/src/payments/entities/transaction.rs:16
  - Public fields: `attrs`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::transaction::transaction::events::Deleted` · framework/src/payments/entities/transaction.rs:16
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::transaction::transaction::events::Deleting` · framework/src/payments/entities/transaction.rs:16
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::transaction::transaction::events::ForceDeleted` · framework/src/payments/entities/transaction.rs:16
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::transaction::transaction::events::ForceDeleting` · framework/src/payments/entities/transaction.rs:16
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::transaction::transaction::events::Replicating` · framework/src/payments/entities/transaction.rs:16
  - Public fields: `source`, `replica`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::transaction::transaction::events::Restored` · framework/src/payments/entities/transaction.rs:16
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::transaction::transaction::events::Restoring` · framework/src/payments/entities/transaction.rs:16
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::transaction::transaction::events::Retrieved` · framework/src/payments/entities/transaction.rs:16
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::transaction::transaction::events::Retrieving` · framework/src/payments/entities/transaction.rs:16
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::transaction::transaction::events::Saved` · framework/src/payments/entities/transaction.rs:16
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::transaction::transaction::events::Saving` · framework/src/payments/entities/transaction.rs:16
  - Public fields: `attrs`, `is_creating`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::transaction::transaction::events::Trashed` · framework/src/payments/entities/transaction.rs:16
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::transaction::transaction::events::Updated` · framework/src/payments/entities/transaction.rs:16
  - Public fields: `previous`, `current`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::transaction::transaction::events::Updating` · framework/src/payments/entities/transaction.rs:16
  - Public fields: `previous`, `attrs`
  - Implements: `suprnova::Event`

### `suprnova::payments::entities::transaction::transaction`

- [ ] struct `suprnova::payments::entities::transaction::ActiveModel` · framework/src/payments/entities/transaction.rs:16 (also `suprnova::payments::entities::transaction::transaction::ActiveModel`)
  - Public fields: `id`, `provider`, `provider_transaction_id`, `provider_customer_id`, `provider_subscription_id`, `amount_total_minor`, `amount_tax_minor`, `currency`, `status`, `provider_metadata`, `paid_at`, `created_at`, `updated_at`
- [ ] struct `suprnova::payments::entities::transaction::transaction::ColumnIter` · framework/src/payments/entities/transaction.rs:16
- [ ] struct `suprnova::payments::entities::transaction::Entity` · framework/src/payments/entities/transaction.rs:16 (also `suprnova::payments::entities::transaction::transaction::Entity`)
- [ ] struct `suprnova::payments::entities::transaction::Model` · framework/src/payments/entities/transaction.rs:16 (also `suprnova::payments::entities::transaction::transaction::Model`)
  - Public fields: `id`, `provider`, `provider_transaction_id`, `provider_customer_id`, `provider_subscription_id`, `amount_total_minor`, `amount_tax_minor`, `currency`, `status`, `provider_metadata`, `paid_at`, `created_at`, `updated_at`
  - [ ] fn `suprnova::payments::entities::transaction::Model::into_ex` · framework/src/payments/entities/transaction.rs:16
- [ ] struct `suprnova::payments::entities::transaction::transaction::PrimaryKeyIter` · framework/src/payments/entities/transaction.rs:16
- [ ] struct `suprnova::payments::entities::transaction::transaction::RelationIter` · framework/src/payments/entities/transaction.rs:16
- [ ] enum `suprnova::payments::entities::transaction::Column` · framework/src/payments/entities/transaction.rs:16 (also `suprnova::payments::entities::transaction::transaction::Column`)
  - Variants: `Id`, `Provider`, `ProviderTransactionId`, `ProviderCustomerId`, `ProviderSubscriptionId`, `AmountTotalMinor`, `AmountTaxMinor`, `Currency`, `Status`, `ProviderMetadata`, `PaidAt`, `CreatedAt`, `UpdatedAt`
  - Implements: `suprnova::IntoColumn`
  - [ ] fn `suprnova::payments::entities::transaction::Column::as_str` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Column::from_name` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Column::iter` · framework/src/payments/entities/transaction.rs:16
- [ ] enum `suprnova::payments::entities::transaction::transaction::PrimaryKey` · framework/src/payments/entities/transaction.rs:16
  - Variants: `Id`
- [ ] enum `suprnova::payments::entities::transaction::transaction::Relation` · framework/src/payments/entities/transaction.rs:16
- [ ] type `suprnova::payments::entities::transaction::transaction::__Suprnova_Cast_Storage_created_at` · framework/src/payments/entities/transaction.rs:16
- [ ] type `suprnova::payments::entities::transaction::transaction::__Suprnova_Cast_Storage_paid_at` · framework/src/payments/entities/transaction.rs:16
- [ ] type `suprnova::payments::entities::transaction::transaction::__Suprnova_Cast_Storage_updated_at` · framework/src/payments/entities/transaction.rs:16

### `suprnova::payments::entities::transaction`

- [ ] struct `suprnova::payments::entities::transaction::Transaction` · framework/src/payments/entities/transaction.rs:17
  - Public fields: `id`, `provider`, `provider_transaction_id`, `provider_customer_id`, `provider_subscription_id`, `amount_total_minor`, `amount_tax_minor`, `currency`, `status`, `provider_metadata`, `paid_at`, `created_at`, `updated_at`
  - Implements: `suprnova::EagerLoadDispatch`, `suprnova::EloquentModel`, `suprnova::FirstOrCreate`, `suprnova::Model`, `suprnova::ModelEventHooks`, `suprnova::Persistable`, `suprnova::ReplicateExt`, `suprnova::Touchable`
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::fill` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::without_global_scope` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::without_global_scopes` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::on` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::on_write_connection` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::count` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::sum` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::avg` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::min` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::max` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::pluck` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::pluck_keyed` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::filter` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::db_where` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::where_in` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::where_like` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::latest` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::oldest` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::pivot` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::with` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::with_count` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::with_sum` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::with_avg` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::with_min` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::with_max` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::observe` · framework/src/payments/entities/transaction.rs:16

### `suprnova::payments::entities::webhook_event::webhook_event::events`

- [ ] struct `suprnova::payments::entities::webhook_event::webhook_event::events::Created` · framework/src/payments/entities/webhook_event.rs:21
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::webhook_event::webhook_event::events::Creating` · framework/src/payments/entities/webhook_event.rs:21
  - Public fields: `attrs`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::webhook_event::webhook_event::events::Deleted` · framework/src/payments/entities/webhook_event.rs:21
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::webhook_event::webhook_event::events::Deleting` · framework/src/payments/entities/webhook_event.rs:21
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::webhook_event::webhook_event::events::ForceDeleted` · framework/src/payments/entities/webhook_event.rs:21
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::webhook_event::webhook_event::events::ForceDeleting` · framework/src/payments/entities/webhook_event.rs:21
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::webhook_event::webhook_event::events::Replicating` · framework/src/payments/entities/webhook_event.rs:21
  - Public fields: `source`, `replica`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::webhook_event::webhook_event::events::Restored` · framework/src/payments/entities/webhook_event.rs:21
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::webhook_event::webhook_event::events::Restoring` · framework/src/payments/entities/webhook_event.rs:21
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::webhook_event::webhook_event::events::Retrieved` · framework/src/payments/entities/webhook_event.rs:21
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::webhook_event::webhook_event::events::Retrieving` · framework/src/payments/entities/webhook_event.rs:21
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::webhook_event::webhook_event::events::Saved` · framework/src/payments/entities/webhook_event.rs:21
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::webhook_event::webhook_event::events::Saving` · framework/src/payments/entities/webhook_event.rs:21
  - Public fields: `attrs`, `is_creating`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::webhook_event::webhook_event::events::Trashed` · framework/src/payments/entities/webhook_event.rs:21
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::webhook_event::webhook_event::events::Updated` · framework/src/payments/entities/webhook_event.rs:21
  - Public fields: `previous`, `current`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::webhook_event::webhook_event::events::Updating` · framework/src/payments/entities/webhook_event.rs:21
  - Public fields: `previous`, `attrs`
  - Implements: `suprnova::Event`

### `suprnova::payments::entities::webhook_event::webhook_event`

- [ ] struct `suprnova::payments::entities::webhook_event::ActiveModel` · framework/src/payments/entities/webhook_event.rs:21 (also `suprnova::payments::entities::webhook_event::webhook_event::ActiveModel`)
  - Public fields: `id`, `provider`, `provider_event_id`, `provider_event_type`, `neutral_event_kind`, `payload`, `received_at`, `processed_at`, `process_error`
- [ ] struct `suprnova::payments::entities::webhook_event::webhook_event::ColumnIter` · framework/src/payments/entities/webhook_event.rs:21
- [ ] struct `suprnova::payments::entities::webhook_event::Entity` · framework/src/payments/entities/webhook_event.rs:21 (also `suprnova::payments::entities::webhook_event::webhook_event::Entity`)
- [ ] struct `suprnova::payments::entities::webhook_event::Model` · framework/src/payments/entities/webhook_event.rs:21 (also `suprnova::payments::entities::webhook_event::webhook_event::Model`)
  - Public fields: `id`, `provider`, `provider_event_id`, `provider_event_type`, `neutral_event_kind`, `payload`, `received_at`, `processed_at`, `process_error`
  - [ ] fn `suprnova::payments::entities::webhook_event::Model::into_ex` · framework/src/payments/entities/webhook_event.rs:21
- [ ] struct `suprnova::payments::entities::webhook_event::webhook_event::PrimaryKeyIter` · framework/src/payments/entities/webhook_event.rs:21
- [ ] struct `suprnova::payments::entities::webhook_event::webhook_event::RelationIter` · framework/src/payments/entities/webhook_event.rs:21
- [ ] enum `suprnova::payments::entities::webhook_event::Column` · framework/src/payments/entities/webhook_event.rs:21 (also `suprnova::payments::entities::webhook_event::webhook_event::Column`)
  - Variants: `Id`, `Provider`, `ProviderEventId`, `ProviderEventType`, `NeutralEventKind`, `Payload`, `ReceivedAt`, `ProcessedAt`, `ProcessError`
  - Implements: `suprnova::IntoColumn`
  - [ ] fn `suprnova::payments::entities::webhook_event::Column::as_str` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::Column::from_name` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::Column::iter` · framework/src/payments/entities/webhook_event.rs:21
- [ ] enum `suprnova::payments::entities::webhook_event::webhook_event::PrimaryKey` · framework/src/payments/entities/webhook_event.rs:21
  - Variants: `Id`
- [ ] enum `suprnova::payments::entities::webhook_event::webhook_event::Relation` · framework/src/payments/entities/webhook_event.rs:21
- [ ] type `suprnova::payments::entities::webhook_event::webhook_event::__Suprnova_Cast_Storage_processed_at` · framework/src/payments/entities/webhook_event.rs:21
- [ ] type `suprnova::payments::entities::webhook_event::webhook_event::__Suprnova_Cast_Storage_received_at` · framework/src/payments/entities/webhook_event.rs:21

### `suprnova::payments::entities::webhook_event`

- [ ] struct `suprnova::payments::entities::webhook_event::WebhookEvent` · framework/src/payments/entities/webhook_event.rs:22
  - Public fields: `id`, `provider`, `provider_event_id`, `provider_event_type`, `neutral_event_kind`, `payload`, `received_at`, `processed_at`, `process_error`
  - Implements: `suprnova::EagerLoadDispatch`, `suprnova::EloquentModel`, `suprnova::FirstOrCreate`, `suprnova::Model`, `suprnova::ModelEventHooks`, `suprnova::Persistable`, `suprnova::ReplicateExt`
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::fill` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::without_global_scope` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::without_global_scopes` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::on` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::on_write_connection` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::count` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::sum` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::avg` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::min` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::max` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::pluck` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::pluck_keyed` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::filter` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::db_where` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::where_in` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::where_like` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::latest` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::oldest` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::pivot` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::with` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::with_count` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::with_sum` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::with_avg` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::with_min` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::with_max` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::observe` · framework/src/payments/entities/webhook_event.rs:21

### `suprnova::payments::error`

- [ ] enum `suprnova::payments::PaymentError` · framework/src/payments/error.rs:11 (also `suprnova::payments::error::PaymentError`)
  - Variants: `Provider`, `Validation`, `NotSupported`, `Declined`, `Authentication`, `NotFound`, `WebhookSignature`, `InvalidPhoneNumber`, `InvalidCountryCode`, `Internal`
- [ ] type `suprnova::payments::PaymentResult` · framework/src/payments/error.rs:68 (also `suprnova::payments::error::PaymentResult`)

### `suprnova::payments::migrations::m_2026_05_22_000001_create_payments_tables`

- [ ] struct `suprnova::payments::migrations::CreatePaymentsTables` · framework/src/payments/migrations/m_2026_05_22_000001_create_payments_tables.rs:23 (also `suprnova::payments::migrations::m_2026_05_22_000001_create_payments_tables::Migration`)

### `suprnova::payments::migrations`

- [ ] fn `suprnova::payments::migrations::migrations` · framework/src/payments/migrations/mod.rs:29

### `suprnova::payments::money`

- [ ] struct `suprnova::Money` · framework/src/payments/money.rs:29 (also `suprnova::payments::Money`, `suprnova::payments::money::Money`)
  - [ ] fn `suprnova::Money::from_minor_units` · framework/src/payments/money.rs:36
  - [ ] fn `suprnova::Money::from_decimal` · framework/src/payments/money.rs:58
  - [ ] fn `suprnova::Money::minor_units` · framework/src/payments/money.rs:72
  - [ ] fn `suprnova::Money::currency` · framework/src/payments/money.rs:77
  - [ ] fn `suprnova::Money::as_decimal` · framework/src/payments/money.rs:82
  - [ ] fn `suprnova::Money::is_zero` · framework/src/payments/money.rs:89

### `suprnova::payments::registry`

- [ ] struct `suprnova::PaymentProviderEntry` · framework/src/payments/registry.rs:46 (also `suprnova::payments::PaymentProviderEntry`, `suprnova::payments::registry::PaymentProviderEntry`)
  - Public fields: `name`, `factory`
- [ ] struct `suprnova::PaymentProviderRegistry` · framework/src/payments/registry.rs:75 (also `suprnova::payments::PaymentProviderRegistry`, `suprnova::payments::registry::PaymentProviderRegistry`)
  - [ ] fn `suprnova::PaymentProviderRegistry::get` · framework/src/payments/registry.rs:85
  - [ ] fn `suprnova::PaymentProviderRegistry::names` · framework/src/payments/registry.rs:101
  - [ ] fn `suprnova::PaymentProviderRegistry::bind` · framework/src/payments/registry.rs:119

### `suprnova::payments::webhook_route`

- [ ] fn `suprnova::payments::webhook_routes` · framework/src/payments/webhook_route.rs:999 (also `suprnova::payments::webhook_route::webhook_routes`)
