# Payments - NOWPayments

The `suprnova-payments-nowpayments` adapter creates hosted invoices, verifies
payment notifications, and reads payment status using the merchant API key.
It registers as `nowpayments` in the normal payment-provider registry.

## Install and configure

Use a Suprnova revision that contains this adapter for both dependencies. For a
local application beside a Suprnova source checkout:

```toml
[dependencies]
suprnova = { path = "../suprnova/framework" }
suprnova-payments-nowpayments = { path = "../suprnova/crates/suprnova-payments-nowpayments" }
serde_json = "1"
```

Configure these values in the application's protected environment:

```dotenv
NOWPAYMENTS_ENVIRONMENT=sandbox
NOWPAYMENTS_API_KEY=your-sandbox-api-key
NOWPAYMENTS_IPN_SECRET=your-sandbox-ipn-secret
NOWPAYMENTS_IPN_CALLBACK_URL=https://app.example/webhooks/payments/nowpayments
```

The environment accepts `sandbox` or `production` and defaults to `sandbox`.
An unknown or blank environment fails configuration. Blank credentials fail
before HTTP requests. Use separate credentials for each environment.

Register the provider during boot, propagating a configuration error:

```rust,no_run
use std::sync::Arc;
use suprnova::payments::{PaymentProviderRegistry, PaymentResult};
use suprnova_payments_nowpayments::NowPaymentsProvider;

fn register_nowpayments() -> PaymentResult<Arc<NowPaymentsProvider>> {
    let provider = Arc::new(NowPaymentsProvider::from_env()?);
    PaymentProviderRegistry::bind("nowpayments", provider.clone());
    Ok(provider)
}
```

Add the payment migrations and compose `webhook_routes(db)` into the application's
router as shown in the [payments overview](payments.md). The endpoint is
`POST /webhooks/payments/nowpayments`. Keep this provider-authenticated route
outside browser-session CSRF middleware. Configure the callback with the exact
public HTTPS URL; request bodies must reach the adapter unchanged.

## Start a hosted invoice

Persist a checkout attempt with a unique merchant order reference before making
the request. The amount comes from the application's trusted order, never from
a browser-supplied total.

```rust,no_run
use suprnova::payments::{Money, SessionMode, StartSessionRequest};
use suprnova_payments_nowpayments::{InvoiceCreationError, NowPaymentsInvoice, NowPaymentsProvider};
use serde_json::json;

async fn create_order_invoice(
    provider: &NowPaymentsProvider,
    order_id: String,
    amount: Money,
) -> Result<NowPaymentsInvoice, InvoiceCreationError> {
    provider.create_invoice(StartSessionRequest {
        mode: SessionMode::OneOff,
        customer_ref: String::new(),
        price_refs: Vec::new(),
        amount_hint: Some(amount),
        success_return_url: "https://app.example/billing/return".into(),
        cancel_return_url: "https://app.example/billing/cancel".into(),
        idempotency_key: None,
        metadata: Some(json!({"order_id": order_id})),
    }).await
}
```

`Checkout::start_session` accepts the same request and returns the generic
`SessionPayload::Redirect`. Store its `provider_session_id` as an **invoice ID**,
then redirect to its validated URL. The concrete `create_invoice` method also
returns the merchant order reference and distinguishes uncertain creation errors.

The invoice adapter requires one-off mode, empty customer/price references, and
a positive `Money` amount with a defined fiat currency exponent. It sends the
exact major-unit decimal amount and lowercase currency code. Customers select a
crypto currency on the hosted page unless `pay_currency` is supplied.

Metadata is a strict object with these fields:

| Field | Meaning |
| --- | --- |
| `order_id` | Required nonblank merchant reference, at most 128 bytes |
| `order_description` | Optional description, at most 500 bytes |
| `pay_currency` | Optional lowercase provider ticker, such as `btc` |
| `is_fixed_rate` | Optional provider exchange-rate setting |
| `is_fee_paid_by_user` | Optional provider fee setting |

Unknown metadata keys are rejected. Do not put arbitrary application or customer
data into this object. Return URLs must use the same HTTPS origin as the callback,
without credentials or fragments. Invoice redirects must target the selected
NOWPayments environment and contain the returned invoice ID.

## Creation failures and retries

NOWPayments' invoice endpoint does not document an idempotency-key guarantee.
The adapter rejects a non-`None` `idempotency_key`; `order_id` is correlation data,
not provider-side deduplication. Neither HTTP redirects nor automatic retries
are enabled. Requests have a 30-second deadline and a 64 KiB response limit.

`InvoiceCreationError::Rejected` represents input validation or a definite API
rejection. `Unknown` means an invoice may already exist: for example, the request
timed out, the provider returned a server error, or its success response was
invalid. Mark that attempt uncertain and reconcile the order in the provider
dashboard before creating another invoice. Do not put invoice creation inside
a generic retry loop. Through `Checkout::start_session`, an unknown outcome is
a `PaymentError::Provider` with this recovery instruction.

## Verify and reconcile payments

An invoice can produce a separate payment ID. `payment_status(payment_id)` calls
the authenticated payment endpoint and rejects a response with a different ID.
`Checkout::session_status(invoice_id)` returns `NotSupported`; an invoice ID
cannot be substituted into the payment lookup endpoint. This adapter does not
use dashboard email/password credentials to list payments by invoice.

```rust,no_run
use suprnova::payments::{Money, PaymentResult};
use suprnova_payments_nowpayments::{NowPaymentsProvider, NowPaymentsStatus};

async fn payment_matches_order(
    provider: &NowPaymentsProvider,
    verified_payment_id: &str,
    expected_invoice_id: &str,
    expected_order_id: &str,
    expected_price: Money,
) -> PaymentResult<bool> {
    let payment = provider.payment_status(verified_payment_id).await?;
    Ok(payment.status == NowPaymentsStatus::Finished
        && payment.invoice_id.as_deref() == Some(expected_invoice_id)
        && payment.order_id.as_deref() == Some(expected_order_id)
        && payment.price == expected_price)
}
```

The payment ID must come from a verified IPN or another trusted server-side
record. A browser return is not evidence of payment. A successful lookup is
also not an authorization check: match it to the stored order, invoice, amount
and currency before changing access. `price` is the requested fiat price, not
the crypto quantity received. Review the merchant's partial-payment acceptance
settings; this adapter never converts `partially_paid` into success.

| Provider state | Neutral event |
| --- | --- |
| `finished` | `PaymentSucceeded` |
| `failed`, `expired`, `cancelled`, `canceled` | `PaymentFailed` |
| `refunded` | `PaymentRefunded` |
| `waiting`, `confirming`, `confirmed`, `sending`, `partially_paid`, unknown | No neutral classification |

IPN signatures use recursively sorted JSON and HMAC SHA-512. Verification uses
a constant-time MAC comparison. Missing, duplicate or invalid signatures,
duplicate JSON keys, oversized payloads and malformed required fields are
rejected. The provider signs no independent delivery timestamp, so there is no
invented timestamp-replay window. Replay protection uses persisted receipts.

NOWPayments IPNs have no independent event ID. The adapter identifies a receipt
by payment ID and status, so repeat delivery with a different `updated_at` does
not repeat the same terminal event. The framework stores verified events in
`payments_webhook_events` and retries failed processing. A delayed pending event
does not modify a settled transaction mirror.

### Customerless invoices and application state

The invoice API does not supply the customer identity required by Suprnova's
transaction mirrors. This adapter returns `false` from
`WebhookHandler::mirrors_payment_transactions`: all verified events, including
refunds, remain in the audit log, but it creates no customer or transaction
mirror. Stripe and Paddle retain the default mirror behavior.

The generic webhook route records and processes provider events; it does not
fulfill application orders. An application reconciliation job should consume
verified receipts, read current payment status, and update its order in an
idempotent transaction. Store a unique provider/payment or order fulfillment key
so retries and reordered deliveries cannot grant access twice. Track the job's
own durable completion state separately from the framework's `processed_at`.
Use current authenticated state for delayed events and handle refunds according
to the application's policy.

## Capability limits

`CustomerStore` and `Subscription` operations return `NotSupported`.
`as_payment()` and `as_promotions()` return `None`. This adapter does not implement
custody balances, recurring billing, direct deposit-address checkout, payouts,
refund initiation, or provider customer management. NOWPayments offers separate
products for some of these operations; this invoice adapter does not pretend to
implement them.

### Why Suprnova diverges

The common provider traits stay available, but unsupported operations fail
explicitly. Invoices without provider customers use audited notifications and
application-owned orders rather than fabricated customer records or subscriptions.

## Verification

Run the adapter tests and the shared payment regressions from the source checkout:

```sh
CARGO_INCREMENTAL=0 cargo nextest run -p suprnova-payments-nowpayments
CARGO_INCREMENTAL=0 cargo nextest run -p suprnova --test payments
```

The local suite uses fake HTTP responses, an independently generated JavaScript
signature fixture, and real framework ingress with SQLite. It covers successful
checkout, authentication failures, malformed or large responses, timeouts,
untrusted redirects, invalid signatures, status mapping, duplicate deliveries,
and recovery after a database failure. Local fixtures do not establish provider
interoperability.

Before enabling production, use a NOWPayments sandbox account and a reachable
HTTPS callback. Create an invoice, exercise the provider's available success,
partial and failure cases, confirm its signature is accepted, and reconcile the
payment ID against the stored invoice and order. Replay a delivery and confirm
the application's fulfillment key prevents a second grant. No real-account
sandbox or production run is implied by the local tests.

Provider references: [API endpoints](https://nowpayments.zendesk.com/hc/en-us/articles/21345824322717-API-and-endpoint-description),
[IPN authentication](https://nowpayments.zendesk.com/hc/en-us/articles/21395546303389-IPN-and-how-to-setup),
and the [official SDK](https://github.com/NowPaymentsIO/nowpayments-sdk-nodejs).

## Next

See [Frontend Integration](payments-frontend.md) for rendering redirect payloads,
or the [Provider Guide](payments-provider-guide.md) for the shared contracts.
