# NOWPayments adapter for Suprnova

Hosted invoice checkout, API-key payment lookup, and authenticated IPN webhooks
through Suprnova's payment-provider registry.

See the [NOWPayments manual](../../manual/payments-nowpayments.md) for setup,
checkout, payment reconciliation, retry handling, and capability limits.

The adapter supports one-off invoices. It does not implement recurring billing,
provider customers, capture, promotions, custody, or payouts. Invoice IDs and
payment IDs are separate. Only `finished` maps to payment success.

Run local verification from the workspace root:

```sh
CARGO_INCREMENTAL=0 cargo nextest run -p suprnova-payments-nowpayments
```

Provider sandbox verification requires a sandbox account and a reachable HTTPS
callback. Local fake-server tests do not establish real-provider interoperability.
