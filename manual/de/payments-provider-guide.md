# Zahlungs-Provider-Adapter schreiben

Dieser Leitfaden führt durch den Bau einer
Drittanbieter-Adapter-Crate - `suprnova-payments-mollie` -, die sich
in Suprnovas Provider-neutrale Zahlungs-Oberfläche einsteckt. Am Ende
haben Sie eine Crate, die sich selbst registriert, den
Diskriminator-Flow besteht und sich mit einem einzigen `cargo add`
in jede Suprnova-App einklinken lässt.

Dieselbe Struktur gilt für jeden Provider: Square, Braintree, Adyen
oder alles andere mit einer HTTP-API.

### Warum Suprnova abweicht

Laravel liefert Cashier als hauseigene Stripe-Integration. Für den
Stripe-Pfad ist das exzellent, aber es schreibt das Vokabular eines
Providers ins Framework fest - ein zweiter Provider bedeutet,
Cashier zu forken oder eine parallele Oberfläche daneben zu bauen.

Suprnova hält jeden Provider auf demselben Fünf-Trait-Vertrag:
`Checkout`, `Subscription`, `CustomerStore`, `WebhookHandler` und das
optionale `Payment` für Provider mit Server-Erfassung. Domain-Code
hält immer nur ein `Arc<dyn PaymentProvider>` aus der Registry.
Stripe gegen Paddle auszutauschen (oder gegen den Mollie-Adapter, den
Sie gleich schreiben) ist eine Bootstrap-Änderung, keine
Code-Änderung. Die Referenz-Adapter unter
`crates/suprnova-payments-stripe/` und
`crates/suprnova-payments-paddle/` belegen, dass der Trait-Vertrag
für zwei sehr unterschiedliche Geschäftsmodelle trägt - Gateway mit
direkter Erfassung und Merchant of Record -, und Ihr Adapter fügt
sich in dieselbe Form.

## 1. Die Workspace-Member-Crate anlegen

Vom Repo-Root aus:

```bash
cargo new --lib crates/suprnova-payments-mollie
```

Fügen Sie sie zu Ihrem Root-`Cargo.toml` hinzu:

```toml
[workspace]
members = [
    "framework",
    "app",
    "suprnova-cli",
    "suprnova-macros",
    "crates/suprnova-payments-mollie",  # diese Zeile hinzufügen
]
```

(Die Referenz-Adapter - `crates/suprnova-payments-stripe` und
`crates/suprnova-payments-paddle` - liegen im selben
`crates/`-Verzeichnis und sind gute Vorlagen, die Sie parallel zu
diesem Leitfaden lesen können.)

**`crates/suprnova-payments-mollie/Cargo.toml`:**

```toml
[package]
name = "suprnova-payments-mollie"
version.workspace = true
edition.workspace = true
license.workspace = true
description = "Mollie payment adapter for Suprnova"

[dependencies]
suprnova = { path = "../../framework" }
async-trait = "0.1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
inventory = "0.3"
tracing = "0.1"
tokio = { version = "1", features = ["macros", "rt"] }
# Ihr Mollie-SDK:
mollie-rs = "0.1"
hmac = "0.12"   # für die HMAC-Verifizierung von Webhooks
sha2 = "0.10"
hex = "0.4"

[dev-dependencies]
tokio = { version = "1", features = ["full"] }
```

## 2. Die Quelldateien anlegen

Orientieren Sie sich an der Struktur der mitgelieferten Adapter:

```
crates/suprnova-payments-mollie/src/
├── lib.rs          # MollieProvider-Struktur, PaymentProvider-Impl, from_env
├── checkout.rs     # Checkout-Impl
├── customer.rs     # CustomerStore-Impl
├── subscription.rs # Subscription-Impl
├── webhook.rs      # WebhookHandler-Impl
├── event_map.rs    # Provider-Event-String → NeutralEventKind
└── payment.rs      # Payment-Impl (falls Mollie Server-Erfassung unterstützt)
```

## 3. `lib.rs` - die Provider-Struktur

```rust,ignore
use async_trait::async_trait;
use suprnova::payments::{Payment, PaymentProvider};

mod checkout;
mod customer;
mod event_map;
mod payment;
mod subscription;
mod webhook;

pub use event_map::mollie_event_to_neutral;

/// Mollie-Adapter für Suprnovas Provider-neutrale Zahlungs-Oberfläche.
#[derive(Clone, Debug)]
pub struct MollieProvider {
    /// Mollie-API-Schlüssel (`test_…` / `live_…`).
    api_key: String,
    /// Webhook-Signaturgeheimnis - für die HMAC-Verifizierung verwendet.
    webhook_secret: String,
    /// HTTP-Client - über Anfragen hinweg geteilt.
    client: reqwest::Client,
}

impl MollieProvider {
    pub fn new(api_key: impl Into<String>, webhook_secret: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            webhook_secret: webhook_secret.into(),
            client: reqwest::Client::new(),
        }
    }

    /// Aus Umgebungsvariablen konstruieren.
    ///
    /// Liest:
    /// - `MOLLIE_API_KEY`
    /// - `MOLLIE_WEBHOOK_SECRET`
    pub fn from_env() -> Result<Self, String> {
        let api_key = std::env::var("MOLLIE_API_KEY")
            .map_err(|_| "MOLLIE_API_KEY not set".to_string())?;
        let webhook_secret = std::env::var("MOLLIE_WEBHOOK_SECRET")
            .map_err(|_| "MOLLIE_WEBHOOK_SECRET not set".to_string())?;
        Ok(Self::new(api_key, webhook_secret))
    }
}

impl PaymentProvider for MollieProvider {
    fn name(&self) -> &'static str {
        "mollie"
    }

    // Überschreiben Sie as_payment() nur, wenn Sie auch Payment
    // (Server-Erfassung) implementieren. Die Standard-Impl auf
    // PaymentProvider liefert None - lassen Sie dieses Überschreiben
    // ganz weg, wenn Mollie nur-Checkout / MoR-artig ist.
    fn as_payment(&self) -> Option<&dyn Payment> {
        Some(self)
    }
}
```

`PaymentProvider` ist der Sammel-Trait - die Supertrait-Klausel ist
`Checkout + Subscription + CustomerStore + WebhookHandler`, sodass
der Compiler sich weigert, Ihren Provider zu binden, bis alle vier
implementiert sind. Der fünfte Trait, `Payment`, ist **optional** -
nur Provider, die serverseitige Erfassung anbieten, implementieren
ihn, und `as_payment()` meldet das Ergebnis an das Framework. Der
Standard von `as_payment()` liefert `None`, also lassen Sie das
Überschreiben ganz weg, wenn Ihr Provider keine Server-Erfassung
macht.

## 4. Die vier erforderlichen Traits implementieren

### `checkout.rs`

```rust,ignore
use async_trait::async_trait;
use suprnova::payments::{
    Checkout, PaymentError, PaymentResult, SessionMode, SessionPayload, StartSessionRequest,
};

use crate::MollieProvider;

#[async_trait]
impl Checkout for MollieProvider {
    async fn start_session(&self, req: StartSessionRequest) -> PaymentResult<SessionPayload> {
        // Die Mollie-API aufrufen, um eine Zahlung oder Order anzulegen.
        // Die Antwort auf eine der SessionPayload-Varianten abbilden.
        // Mollie nutzt gehostete Checkout-Seiten, daher passt Redirect natürlich.
        let checkout_url = self.create_mollie_payment(&req).await
            .map_err(|e| PaymentError::Internal(format!("Mollie checkout error: {e}")))?;

        Ok(SessionPayload::Redirect {
            url: checkout_url,
            provider_session_id: "mollie_session_id_here".into(),
        })
    }
}

impl MollieProvider {
    async fn create_mollie_payment(&self, req: &StartSessionRequest) -> Result<String, mollie_rs::Error> {
        // Hier den Mollie-SDK-Aufruf verdrahten.
        // Die gehostete Checkout-URL zurückgeben.
        todo!("Mollie payment creation")
    }
}
```

### `customer.rs`

```rust,ignore
use async_trait::async_trait;
use suprnova::payments::{
    CreateCustomerRequest, CustomerRef, CustomerStore, PaymentError, PaymentResult,
    UpdateCustomerRequest,
};

use crate::MollieProvider;

#[async_trait]
impl CustomerStore for MollieProvider {
    async fn create_customer(&self, req: CreateCustomerRequest) -> PaymentResult<CustomerRef> {
        // POST /v2/customers an Mollie
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn update_customer(&self, req: UpdateCustomerRequest) -> PaymentResult<CustomerRef> {
        // PATCH /v2/customers/{id}
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn get_customer(&self, provider_customer_id: &str) -> PaymentResult<CustomerRef> {
        // GET /v2/customers/{id}
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn delete_customer(&self, provider_customer_id: &str) -> PaymentResult<()> {
        // DELETE /v2/customers/{id}
        Err(PaymentError::Internal("not yet implemented".into()))
    }
}
```

### `subscription.rs`

```rust,ignore
use async_trait::async_trait;
use suprnova::payments::{
    PaymentError, PaymentResult, SubscribeRequest, Subscription, SubscriptionResult,
    UpdateSubscriptionRequest,
};

use crate::MollieProvider;

#[async_trait]
impl Subscription for MollieProvider {
    async fn subscribe(&self, req: SubscribeRequest) -> PaymentResult<SubscriptionResult> {
        // POST /v2/customers/{id}/subscriptions
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn update(&self, req: UpdateSubscriptionRequest) -> PaymentResult<SubscriptionResult> {
        // PATCH /v2/customers/{id}/subscriptions/{sub_id}
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn cancel(
        &self,
        provider_subscription_id: &str,
        at_period_end: bool,
    ) -> PaymentResult<SubscriptionResult> {
        if at_period_end {
            // Kündigungsdatum auf Periodenende setzen
        } else {
            // DELETE /v2/customers/{id}/subscriptions/{sub_id}
        }
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn get(&self, provider_subscription_id: &str) -> PaymentResult<SubscriptionResult> {
        // GET /v2/customers/{id}/subscriptions/{sub_id}
        Err(PaymentError::Internal("not yet implemented".into()))
    }
}
```

Wenn Ihr Provider eine Methode nicht unterstützt, geben Sie
`PaymentError::NotSupported` zurück:

```rust,ignore
Err(PaymentError::NotSupported(
    "Mollie creates subscriptions via checkout - use start_session instead".into()
))
```

### `payment.rs` - serverseitige Erfassung (optional)

Implementieren Sie das nur, wenn Ihr Provider direkte serverseitige
Belastungen gegen eine gespeicherte Zahlungsmethode unterstützt.
Entfernen Sie das Überschreiben von `as_payment()` in `lib.rs`, wenn
Sie das auslassen.

```rust,ignore
use async_trait::async_trait;
use suprnova::payments::{
    ChargeRequest, ChargeResult, Payment, PaymentError, PaymentResult, PaymentStatus,
    RefundRequest, RefundResult,
};

use crate::MollieProvider;

#[async_trait]
impl Payment for MollieProvider {
    async fn charge(&self, req: ChargeRequest) -> PaymentResult<ChargeResult> {
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn capture(&self, provider_transaction_id: &str) -> PaymentResult<ChargeResult> {
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn refund(&self, req: RefundRequest) -> PaymentResult<RefundResult> {
        // POST /v2/payments/{id}/refunds
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn void(&self, provider_transaction_id: &str) -> PaymentResult<()> {
        Err(PaymentError::Internal("not yet implemented".into()))
    }

    async fn status(&self, provider_transaction_id: &str) -> PaymentResult<PaymentStatus> {
        Err(PaymentError::Internal("not yet implemented".into()))
    }
}
```

## 5. Provider-Ereignisse auf `NeutralEventKind` abbilden

**`event_map.rs`:**

```rust,ignore
use suprnova::payments::NeutralEventKind;

/// Bildet einen Mollie-Webhook-Event-Typ-String auf die neutrale
/// Taxonomie des Frameworks ab. Liefert `None` für providerspezifische
/// Ereignisse ohne neutrale Entsprechung.
pub fn mollie_event_to_neutral(event_type: &str) -> Option<NeutralEventKind> {
    match event_type {
        // Mollie-Zahlungen
        "payment.paid"          => Some(NeutralEventKind::PaymentSucceeded),
        "payment.failed"        => Some(NeutralEventKind::PaymentFailed),
        "payment.expired"       => Some(NeutralEventKind::PaymentFailed),
        "refund.created"        => Some(NeutralEventKind::PaymentRefunded),
        "chargeback.created"    => Some(NeutralEventKind::PaymentDisputed),
        // Mollie-Abonnements
        "subscription.created"  => Some(NeutralEventKind::SubscriptionCreated),
        "subscription.updated"  => Some(NeutralEventKind::SubscriptionUpdated),
        "subscription.canceled" => Some(NeutralEventKind::SubscriptionCanceled),
        // Mollie-Bestellungen/-Rechnungen
        "order.paid"            => Some(NeutralEventKind::InvoicePaid),
        // Kunden-Ereignisse
        "customer.created"      => Some(NeutralEventKind::CustomerCreated),
        "customer.updated"      => Some(NeutralEventKind::CustomerUpdated),
        // Providerspezifisch - fällt durch zu raw_payload
        _                       => None,
    }
}
```

Decken Sie mindestens die oben aufgeführten Ereignisse ab. Für jedes
Ereignis, das nicht in der neutralen Taxonomie ist, geben Sie `None`
zurück - es wird trotzdem in `payments_webhook_events` unter
`provider_event_type` + `raw_payload` persistiert, damit Domain-Code
es lesen kann.

## 6. Webhook-Signaturverifizierung implementieren

**`webhook.rs`:**

Mollie signiert Webhook-Payloads mit HMAC-SHA256. Vergleichen Sie
Signaturen immer zeitkonstant, um Timing-Angriffe zu verhindern.

```rust,ignore
use async_trait::async_trait;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use suprnova::payments::{
    NeutralEventKind, PaymentError, PaymentResult, WebhookContext, WebhookEvent, WebhookHandler,
};

use crate::{MollieProvider, event_map::mollie_event_to_neutral};

type HmacSha256 = Hmac<Sha256>;

#[async_trait]
impl WebhookHandler for MollieProvider {
    fn verify(&self, ctx: &WebhookContext<'_>) -> PaymentResult<()> {
        // Den Signatur-Header lesen, den Mollie sendet.
        // Exakter Header-Name und Signaturschema - Mollies Docs für Ihre Version prüfen.
        let signature = ctx
            .headers
            .get("X-Mollie-Signature")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| PaymentError::WebhookSignature(
                "missing X-Mollie-Signature header".into()
            ))?;

        // Erwartetes HMAC-SHA256 über den rohen Body berechnen.
        let mut mac = HmacSha256::new_from_slice(self.webhook_secret.as_bytes())
            .map_err(|e| PaymentError::Internal(format!("HMAC init: {e}")))?;
        mac.update(ctx.body);

        // Die hex-kodierte empfangene Signatur dekodieren.
        let received = hex::decode(signature)
            .map_err(|_| PaymentError::WebhookSignature("non-hex signature".into()))?;

        // Zeitkonstanter Vergleich.
        mac.verify_slice(&received)
            .map_err(|_| PaymentError::WebhookSignature("signature mismatch".into()))
    }

    fn parse_event(&self, body: &[u8]) -> PaymentResult<WebhookEvent> {
        // Mollie sendet JSON - das parsen.
        let raw: serde_json::Value = serde_json::from_slice(body)
            .map_err(|e| PaymentError::Validation(format!("invalid mollie webhook body: {e}")))?;

        let event_id = raw["id"].as_str()
            .ok_or_else(|| PaymentError::Validation("missing event id".into()))?
            .to_string();

        // Mollie verwendet in manchen Webhook-Formen Resource-Typen statt
        // Event-Typ-Strings. An die Version Ihres SDK anpassen.
        let event_type = raw["resource"].as_str()
            .unwrap_or("unknown")
            .to_string();

        let neutral = mollie_event_to_neutral(&event_type);

        Ok(WebhookEvent {
            provider: "mollie".into(),
            provider_event_id: event_id,
            provider_event_type: event_type,
            neutral,
            raw_payload: raw,
        })
    }
}
```

Wichtige Punkte:

- `PaymentError::WebhookSignature(String)` ist die einzige Variante
  für jeden Signaturfehlschlag - fehlender Header, fehlerhafte
  Kodierung, Mismatch. Die Webhook-Route des Frameworks behandelt
  jedes `WebhookSignature(_)` als 401.
- Verwenden Sie `PaymentError::Validation(String)` für nicht
  parsbare Bodys. Die Webhook-Route liefert 400 bei jedem
  Parse-Fehlschlag.
- Der `webhook_routes`-Handler des Frameworks ruft `verify` vor
  `parse_event` auf und hydriert dann innerhalb einer
  DB-Transaktion. Hydrations-Fehlschläge liefern 503, sodass der
  Provider es erneut versucht.
- Protokollieren Sie niemals das rohe Geheimnis oder die empfangene
  Signatur.

### Mirror-Tabellen-Hydration: `extract_payload_ids` + `extract_payment_snapshot` + `extract_customer_snapshot`

Nachdem `parse_event` ein `WebhookEvent` zurückgegeben hat, hydriert
die Webhook-Route des Frameworks die Mirror-Tabellen. Drei optionale
Trait-Methoden treiben das an - alle haben sichere
No-op-Standardimplementierungen, sodass ein Adapter ohne sie
ausgeliefert werden kann und trotzdem durch die Audit-Schicht läuft:

```rust,ignore
fn extract_payload_ids(&self, event: &WebhookEvent) -> PayloadIds;
fn extract_payment_snapshot(&self, event: &WebhookEvent) -> Option<PaymentSnapshot>;
fn extract_customer_snapshot(&self, event: &WebhookEvent) -> Option<CustomerSnapshot>;
```

`PayloadIds` ist die Brücke zwischen dem geparsten Ereignis und der
Mirror-Logik des Frameworks. Implementieren Sie es so, dass das
Framework die richtige Entität findet:

```rust,ignore
pub struct PayloadIds {
    pub subscription_id: Option<String>,
    pub customer_id: Option<String>,
    pub transaction_id: Option<String>,
}
```

Befüllen Sie für jeden `neutral`-Wert die IDs, die die Payload des
Providers offenlegt. Abonnement-Ereignisse sollten `subscription_id`
setzen, damit das Framework `Subscription::get(id)` aufrufen und den
Mirror aus dem kanonischen Zustand aktualisieren kann.
Kunden-Ereignisse setzen `customer_id`. Payment-/Invoice-Ereignisse
setzen `transaction_id`, plus `subscription_id` bei einer
wiederkehrenden Belastung.

`PaymentSnapshot` wird direkt aus der Webhook-Payload gebaut - es
gibt keinen `Payment::get`-Callback. Implementieren Sie es für
Payment-/Invoice-Neutrale:

```rust,ignore
pub struct PaymentSnapshot {
    pub provider_transaction_id: String,
    pub provider_customer_id: String,
    pub provider_subscription_id: Option<String>,
    pub amount_total_minor: i64,
    pub amount_tax_minor: i64,
    pub currency: String,
    pub status: String,             // "succeeded" | "failed" | "refunded" | "disputed"
    pub paid_at: Option<DateTime<Utc>>,
    pub provider_metadata: Value,   // typischerweise das Entitätsobjekt aus der Payload
}
```

Stripes Referenzimplementierung liest
`data.object.{id,amount,currency,customer}` für
`PaymentIntent`-/`Charge`-Ereignisse und
`data.object.{id,amount_paid,tax,currency,customer,subscription,status_transitions.paid_at}`
für `Invoice`-Ereignisse. Paddles liest
`data.{id,customer_id,currency_code,details.totals.{total,tax},billed_at,subscription_id}`.
Orientieren Sie sich an den Konventionen, die zur Payload-Form Ihres
Providers passen - dem Framework ist es egal, wie Sie extrahieren,
nur dass der Snapshot korrekt ist.

Wenn Sie aus `extract_payment_snapshot` `None` zurückgeben, wird die
Audit-Zeile trotzdem geschrieben, aber `payments_transactions`
bleibt unberührt. Das ist die richtige Rückgabe für
Abonnement-/Kunden-Ereignisse oder für jedes Payment-Ereignis, dessen
Payload nicht genug Informationen trägt, um eine Zeile zu befüllen.

`CustomerSnapshot` hält die Synchronisierung des Kunden-Mirrors
providergetrieben (keine hartcodierten JSON-Pfade im Framework):

```rust,ignore
pub struct CustomerSnapshot {
    pub provider_customer_id: String,
    pub email: Option<String>,
    pub provider_metadata: Value,
}
```

Das Framework setzt `email = Set(snapshot.email)` nur, wenn der
Snapshot eine liefert; `provider_metadata` wird immer durch die
Sicht des Providers auf den Kunden ersetzt (`updated_at` wird
unabhängig davon ebenfalls angehoben). Kunden-Mirror-Zeilen werden
immer nur **aktualisiert** - nie eingefügt -, weil `user_id`
`NOT NULL` ist und die App die Verknüpfung Nutzer ↔ Kunde über
`CustomerStore::create_customer` besitzt.

### Fehlschlags-Semantik

Wenn `extract_payload_ids` bei einem Abonnement-Ereignis `None` für
`subscription_id` liefert (oder bei einem Kunden-Ereignis für
`customer_id`), behandelt das Framework das als
`Validation`-Fehler: die Hydrations-Transaktion wird zurückgerollt,
das `process_error` der Audit-Zeile wird gesetzt, und die
HTTP-Antwort ist **503 hydration-failed**, sodass der Provider es
erneut versucht. Stiller Erfolg bei einer fehlerhaften Payload würde
den Mirror veraltet lassen, ohne dass Betreiber es sehen -
Provider-Wiederholungen sind der Wiederherstellungsmechanismus.

Dieser Vertrag bedeutet, dass der Extraktor eines Adapters die
relevanten IDs ehrlich befüllen muss. `None` zurückzugeben ist
Ereignissen vorbehalten, die Ihr Provider überhaupt nicht übersetzen
kann (z. B. ein Payment-Ereignis ohne Charge-ID in der Payload),
nicht für "das habe ich mir beim Parsen erspart".

## 7. Beim App-Boot registrieren

Zwei Mechanismen stehen zur Verfügung - wählen Sie einen:

### Laufzeit-Registrierung (empfohlen für Apps mit Umgebungsvariablen-Konfiguration)

```rust,ignore
use std::sync::Arc;
use suprnova::payments::PaymentProviderRegistry;
use suprnova_payments_mollie::MollieProvider;

let mollie = MollieProvider::from_env().expect("Mollie env vars not set");
PaymentProviderRegistry::bind("mollie", Arc::new(mollie));
```

### Kompilierzeit-Registrierung über `inventory`

Für Adapter-Crates, die eine Zero-Config-Registrierung wollen -
nützlich, wenn Sie eine Bibliothek ausliefern, die Konsumenten
einfach per `cargo add` einbinden, ohne jede Verdrahtung beim Boot:

```rust,ignore
use suprnova::payments::{PaymentProviderEntry, PaymentProviderRegistry};
use inventory;

// In lib.rs, in einem statischen Initialisierer:
inventory::submit!(PaymentProviderEntry {
    name: "mollie",
    factory: || Arc::new(MollieProvider::from_env().expect("Mollie env not set")),
});
```

`inventory::submit!` läuft vor `main`. Die Factory-Closure wird
einmal aufgerufen, wenn auf die Registry zum ersten Mal zugegriffen
wird.

## 8. Den Diskriminator-Test bestehen

Jede Adapter-Crate sollte einen Integrationstest enthalten, der den
Trait-Vertrag Ende-zu-Ende beweist. Das ist der Korrektheitsbeweis -
wenn dieser Test besteht, steckt sich der Provider ohne
Überraschungen in jede Suprnova-App ein.

```rust,ignore
// tests/discriminator.rs (in crates/suprnova-payments-mollie/)

use suprnova::payments::*;
use suprnova_payments_mollie::MollieProvider;

/// Erfordert, dass MOLLIE_API_KEY und MOLLIE_WEBHOOK_SECRET gesetzt sind.
/// Ausführen mit: cargo test --test discriminator -- --ignored
#[tokio::test]
#[ignore = "requires live Mollie sandbox credentials"]
async fn discriminator_flow() {
    let provider = MollieProvider::from_env().expect("Mollie env vars not set");

    // 1. Kunde anlegen
    let cus = provider.create_customer(CreateCustomerRequest {
        user_id: "test_user_1".into(),
        email: "test@example.com".into(),
        name: Some("Test User".into()),
        metadata: None,
    }).await.expect("create_customer failed");
    assert!(!cus.provider_customer_id.is_empty());

    // 2. Checkout-Session starten
    let session = provider.start_session(StartSessionRequest {
        mode: SessionMode::Subscription,
        customer_ref: cus.provider_customer_id.clone(),
        price_refs: vec!["your_mollie_plan_id".into()],
        success_return_url: "https://app.example/billing/success".into(),
        cancel_return_url: "https://app.example/billing/cancel".into(),
        amount_hint: None,
        idempotency_key: Some("discriminator_test_checkout".into()),
        metadata: None,
    }).await.expect("start_session failed");
    assert!(matches!(session, SessionPayload::Redirect { .. }));

    // 3. Direkt abonnieren (falls Ihr Provider das unterstützt; Mollie braucht eventuell Checkout)
    let sub = provider.subscribe(SubscribeRequest {
        customer_ref: cus.provider_customer_id.clone(),
        price_refs: vec!["your_mollie_plan_id".into()],
        trial_days: None,
        idempotency_key: Some("discriminator_test_sub".into()),
        metadata: None,
    }).await.expect("subscribe failed");
    assert_eq!(sub.status, SubscriptionStatus::Active);

    // 4. Zurücklesen
    let fetched = provider.get(&sub.provider_subscription_id).await.expect("get failed");
    assert_eq!(fetched.provider_subscription_id, sub.provider_subscription_id);

    // 5. Zum Periodenende kündigen
    let s = provider.cancel(&sub.provider_subscription_id, true).await.expect("cancel failed");
    assert!(s.cancel_at_period_end);

    // 6. Sofort kündigen
    let s = provider.cancel(&sub.provider_subscription_id, false).await.expect("cancel failed");
    assert_eq!(s.status, SubscriptionStatus::Canceled);

    // 7. Invariante von as_payment() prüfen
    let p: &dyn PaymentProvider = &provider;
    // Falls Sie Payment implementiert haben: assert!(p.as_payment().is_some())
    // Falls Sie Payment NICHT implementiert haben: assert!(p.as_payment().is_none())
    let _ = p.as_payment();
}
```

Sichern Sie Live-Integrationstests mit `#[ignore]` ab, damit
`cargo test` in CI ohne Credentials durchläuft. Führen Sie sie
explizit mit `-- --ignored` gegen ein Sandbox-Konto aus.

## 9. Referenz der `PaymentError`-Varianten

Das vollständige Enum lebt in `framework/src/payments/error.rs`.
Wählen Sie die Variante, die zu dem passt, was tatsächlich
schiefging:

| Variante | Wann verwenden |
|---|---|
| `Provider(String)` | Die API des Providers hat einen Fehler geliefert, den Sie nicht weiter übersetzen müssen |
| `Validation(String)` | Anfragefelder sind ungültig, oder ein Webhook-Body lässt sich nicht parsen |
| `NotSupported(String)` | Die Methode ist für diesen Provider nicht anwendbar (z. B. Paddles `subscribe`) |
| `Declined { reason, decline_code }` | Karte abgelehnt - geben Sie `decline_code` weiter, wenn der Provider einen liefert |
| `Authentication(String)` | Provider hat Ihren API-Schlüssel oder Ihre Credentials abgelehnt |
| `NotFound(String)` | Kunde, Abonnement oder Transaktions-ID existiert nicht |
| `WebhookSignature(String)` | Jeder Signaturfehlschlag - fehlender Header, fehlerhafte Kodierung oder Mismatch |
| `InvalidPhoneNumber(String)` | E.164-Validierung in Mobile-Money-Flows fehlgeschlagen |
| `InvalidCountryCode(String)` | ISO-3166-1-Alpha-2-Validierung fehlgeschlagen |
| `Internal(String)` | Unerwarteter SDK-Fehler, Netzwerkfehler, HMAC-Init-Fehler oder jedes andere framework-seitige Problem |

Die Webhook-Route bildet diese auf Statuscodes ab:
`WebhookSignature(_)` → 401, `Validation(_)` aus `parse_event` → 400,
alles andere aus der Hydration → 503 (sodass der Provider es erneut
versucht).

Sobald Ihr Adapter kompiliert und der Diskriminator-Test besteht:

- Fügen Sie Ihre Crate mit
  `cargo add suprnova-payments-mollie --path ./crates/suprnova-payments-mollie`
  zum `Cargo.toml` Ihrer App hinzu.
- Registrieren Sie sie beim Bootstrap, wie in Schritt 7 gezeigt.
- Binden Sie `webhook_routes(db.clone())` einmal beim App-Boot ein -
  derselbe Handler verzweigt namentlich an jeden registrierten
  Provider, sodass eine einzige Einbindung Stripe, Paddle und Ihren
  neuen Adapter bedient.

## Nächste Schritte

- [Zahlungen](payments.md) - die Provider-neutrale Oberfläche und
  der Schnellstart
- [Zahlungen - Stripe Adapter](payments-stripe.md) - vollständige
  Vorlage für einen Gateway-Adapter
- [Zahlungen - Paddle Adapter](payments-paddle.md) - vollständige
  Vorlage für einen Merchant-of-Record-Adapter
- [Zahlungs-Frontend](payments-frontend.md) - wie Sie die
  `SessionPayload` rendern, die Ihr Adapter zurückgibt
- [Fehlermodell](error-model.md) - wie `PaymentError` als
  `HttpResponse` landet
