# Zahlungen - Paddle Adapter

Der Paddle-Adapter (`suprnova-payments-paddle`) verdrahtet Paddle in
Suprnovas generische Zahlungs-Oberfläche. Greifen Sie darauf zurück,
wenn Sie einen Zahlungs-Provider wollen, der auch Umsatzsteuer,
MwSt., GST, Dunning, Rechnungsstellung und Rückerstattungen für Sie
übernimmt - Paddle ist ein Merchant of Record (MoR), was bedeutet,
dass es der rechtliche Verkäufer gegenüber Ihren Kunden ist und die
Compliance-Oberfläche übernimmt, die ein Gateway mit direkter
Erfassung wie Stripe Ihnen überlässt.

Diese Wahl ändert das mentale Modell. Ihr Domain-Code *besitzt* das
Abonnement nicht - Paddle tut das. Sie öffnen einen Checkout, der
Kunde schließt ihn ab, und der Webhook `SubscriptionCreated` sagt
Ihnen, dass das Abonnement jetzt existiert. Sie können kein
Abonnement über die API anlegen, und Sie können sein Preis-Set
nachträglich nicht austauschen. Sie können kündigen, Sie können den
Zustand lesen, Sie können Abrechnungs-Metadaten aktualisieren. Der
Rest gehört Paddle.

Dieses Kapitel setzt voraus, dass Sie [Zahlungen](payments.md) für
die generische Fünf-Trait-Oberfläche gelesen haben. Hier behandeln
wir, was *nur* für Paddle gilt.

## Wann Sie Paddle wählen

Wählen Sie Paddle, wenn eines oder mehrere davon zutrifft:

- Sie verkaufen digitale Produkte global, und Steuer-Compliance
  (MwSt., GST, US-Sales-Tax) ist ein echter Kostenpunkt auf Ihrer
  Roadmap.
- Sie wollen Wiederholungen bei fehlgeschlagenen Zahlungen,
  Dunning-E-Mails oder das Ausstellen von Belegen nicht selbst
  verwalten.
- Sie wollen für die Buchhaltung eine einzige Rechnung von einem
  einzigen rechtlichen Verkäufer.
- Ihr Geschäftsmodell ist Abonnement-first, und Sie akzeptieren,
  dass der Provider den Abonnement-Lebenszyklus treibt.

Wählen Sie stattdessen [Stripe](payments.md#stripe), wenn Sie
direkte Kontrolle über die Belastungs-Erfassung wollen, Ihre eigene
Steuer selbst handhaben oder serverseitige
`charge`-/`capture`-/`refund`-Aufrufe aus Ihren eigenen Codepfaden
brauchen.

## Einrichtung

Fügen Sie die Crate hinzu:

```bash
cargo add suprnova-payments-paddle
```

Setzen Sie die vier Umgebungsvariablen:

```env
PADDLE_API_KEY=pdl_sdbx_apikey_...
PADDLE_WEBHOOK_KEY=pdl_ntfset_...
PADDLE_CLIENT_TOKEN=test_...
PADDLE_ENVIRONMENT=sandbox
```

| Variable | Was es ist | Woher es kommt |
|---|---|---|
| `PADDLE_API_KEY` | Serverseitiger API-Schlüssel (`pdl_live_apikey_…` / `pdl_sdbx_apikey_…`) | Paddle-Dashboard → Developer Tools → Authentication |
| `PADDLE_WEBHOOK_KEY` | Geheimnis des Benachrichtigungsziels (`pdl_ntfset_…`) | Paddle-Dashboard → Developer Tools → Notifications → Ihr Endpunkt |
| `PADDLE_CLIENT_TOKEN` | Browser-sicheres Client-Token (`live_…` / `test_…`) | Paddle-Dashboard → Developer Tools → Authentication → Client-side tokens |
| `PADDLE_ENVIRONMENT` | `sandbox` (Standard) oder `production` | Ihre Entscheidung |

Registrieren Sie den Provider beim Bootstrap. Beide Formen sind
gültig:

```rust
use std::sync::Arc;
use suprnova::payments::PaymentProviderRegistry;
use suprnova_payments_paddle::{PaddleEnvironment, PaddleProvider};

pub async fn bootstrap() {
    // Aus der Umgebung (empfohlen):
    let paddle = PaddleProvider::from_env()
        .expect("Paddle env vars not set");

    // Oder direkt konstruieren:
    let paddle = PaddleProvider::new(
        "pdl_sdbx_apikey_...",
        "pdl_ntfset_...",
        "test_...",
        PaddleEnvironment::Sandbox,
    ).expect("Paddle client init failed");

    PaymentProviderRegistry::bind("paddle", Arc::new(paddle));
}
```

Die Webhook-Ingress-Route wird vom Helfer
`webhook_routes(db.clone())` des Frameworks registriert - siehe
[Zahlungen](payments.md#webhook-handling). Sowohl `from_env()` als
auch `new()` liefern `Result`, weil das zugrunde liegende
`paddle_rust_sdk::Paddle::new` die Form des API-Schlüssels und die
Endpunkt-URL bei der Konstruktion validiert.

## Das MoR-Mentalmodell

Die Form, die Stripe-Nutzer überrascht:

```
Stripe (Gateway):
    Ihre App  ─────────►  Stripe  ──►  Kartennetzwerk
       │                    ▲
       └────── Webhook ─────┘
    Sie besitzen den Abonnement-Zustand in Ihrer DB; Stripe führt aus.

Paddle (Merchant of Record):
    Ihre App  ─►  Checkout-Link  ─►  Kunde     ──►  Paddle  ──►  Kartennetzwerk
                                                       │
       ◄──────────────────  Webhook  ──────────────────┘
    Paddle besitzt den Abonnement-Zustand; Ihre DB ist der Mirror
```

Im Code zeigt sich der Unterschied an drei Stellen:

1. **Sie können kein Abonnement über die API anlegen.** Rufen Sie
   `Checkout::start_session` mit einem wiederkehrenden Preis auf;
   der Kunde schließt das Paddle-Widget ab; der Webhook
   `SubscriptionCreated` hydriert Ihren Mirror.
2. **Sie können das Preis-Set eines Abonnements nicht über die API
   austauschen.** Paddle reserviert Plan-Änderungen für sein eigenes
   Dashboard oder für Migrations-Flows, die es selbst besitzt.
3. **Sie können einen Kunden nicht löschen.** Archivieren über ein
   Update ist der unterstützte Workaround.

Suprnova macht diese Einschränkungen als `PaymentError::NotSupported`
sichtbar, statt sie zu kaschieren - siehe die
[Capability-Matrix](#capability-matrix) unten.

## Checkout-Flow

`Checkout::start_session` ist der einzige Weg, eine Zahlung mit
Paddle zu starten. Das Frontend öffnet die resultierende
`transaction_id` mit paddle.js, unter Verwendung des `client_token`,
den Sie beim Bootstrap gesetzt haben:

```rust
use std::sync::Arc;
use suprnova::payments::*;

pub async fn start_checkout(
    user_id: String,
    email: String,
) -> PaymentResult<SessionPayload> {
    let provider = PaymentProviderRegistry::get("paddle")
        .expect("paddle provider not registered");

    // 1. Den Kunden in Paddle anlegen (oder einen bestehenden wiederverwenden).
    let cus = provider.create_customer(CreateCustomerRequest {
        user_id: user_id.clone(),
        email,
        name: None,
        metadata: None,
    }).await?;

    // 2. Eine Checkout-Session öffnen. Paddle verzweigt Einmalzahlung vs.
    //    Abonnement anhand der *Preisart*, nicht anhand des SessionMode-Felds unten.
    let session = provider.start_session(StartSessionRequest {
        mode: SessionMode::Subscription,           // von Paddle ignoriert (siehe Hinweis)
        customer_ref: cus.provider_customer_id,
        price_refs: vec!["pri_pro_monthly".into()],
        success_return_url: "https://app.example/billing/success".into(),
        cancel_return_url: "https://app.example/billing/cancel".into(),
        amount_hint: None,
        idempotency_key: Some(format!("checkout_{user_id}")),
        metadata: None,
    }).await?;

    Ok(session)
}
```

Die zurückgegebene `SessionPayload::PaddleInline` trägt alles, was
das Frontend braucht:

```json
{
  "flow": "paddle_inline",
  "transaction_id": "txn_01h...",
  "customer_token": "ctm_01h...",
  "client_token": "test_..."
}
```

Siehe [Zahlungen - Frontend Integration](payments-frontend.md) für
den paddle.js-Mounting-Code in Svelte / React / Vue.

### Paddle verzweigt anhand der Preisart, nicht anhand von `SessionMode`

Eine echte Paddle-spezifische Tücke: Das Feld
`SessionMode::OneOff` / `SessionMode::Subscription` auf
`StartSessionRequest` wird **vom Paddle-Adapter ignoriert**. Paddles
API hat einen einzigen Endpunkt `transaction_create`, und der
Provider untersucht die mitgelieferten Preis-IDs, um den Flow
abzuleiten - ein wiederkehrender Preis startet ein Abonnement, ein
Einmalpreis startet eine einzelne Belastung. Bei Stripe treibt das
Feld den Flow; bei Paddle tut das der *Preis*. Richten Sie Ihren
Paddle-Katalog mit den richtigen Preisarten ein, bevor Sie den
Adapter darauf richten.

## Abonnements treffen per Webhook ein

Weil Paddle den Abonnement-Lebenszyklus besitzt, *erfährt* Ihr
Domain-Code von einem Abonnement erst, wenn Paddle es Ihnen sagt.
Der Flow:

```
Ihre App                        Paddle                    Kunde
   │                              │                          │
   │  start_session(price=pri_…)  │                          │
   ├─────────────────────────────►│                          │
   │  PaddleInline { txn_id, … }  │                          │
   │◄─────────────────────────────┤                          │
   │                              │       paddle.js          │
   │                              │◄─────────────────────────┤
   │                              │   Checkout abschließen   │
   │                              ├─────────────────────────►│
   │                              │                          │
   │   Webhook subscription.created                          │
   │◄─────────────────────────────┤                          │
   │                              │                          │
   ▼                              │                          │
 Mirror-Tabellen hydriert;        │                          │
 Zeile payments_subscriptions     │                          │
 hat provider_subscription_id     │                          │
```

Der `webhook_routes(db)`-Handler des Frameworks übernimmt die
Hydration für Sie: er ruft `WebhookHandler::extract_payload_ids`
auf, um die `subscription_id` zu finden, ruft `Subscription::get(id)`
auf, um den kanonischen Zustand zu lesen, und upsertet
`payments_subscriptions` + `payments_subscription_items` innerhalb
einer Transaktion. Bis der Webhook 200 zurückgibt, ist Ihr Mirror
mit Paddle konsistent.

Es gibt ein kurzes Zeitfenster zwischen dem Abschluss des Widgets
durch den Kunden und dem Eintreffen des Webhooks, in dem
`payments_subscriptions` noch keine Zeile für das neue Abonnement
hat. Zwei Muster decken das ab:

- **Die Redirect-URL für sofortige UX verwenden.** `success_return_url`
  feuert clientseitig, sobald Paddle die Transaktion bestätigt,
  sodass Sie "Abonnement aktiv" zeigen können, ohne auf den
  serverseitigen Webhook zu warten.
- **Pollen und rendern.** Nach dem Redirect die Seite nach einer
  kurzen Verzögerung neu laden, damit der Inertia-Controller den
  jetzt hydrierten Mirror lesen kann.

## Capability-Matrix

Nicht jede Methode auf jedem Trait tut das, was ihr
Stripe-Äquivalent tut. Die Tabelle unten ist die Wahrheit.
`subscribe()` und `update()` mit `new_price_refs.is_some()` sind die
einzigen Methoden, die *immer* fehlschlagen; der Rest funktioniert,
mit den vermerkten Einschränkungen.

| Trait-Methode | Verhalten |
|---|---|
| `Checkout::start_session` | Funktioniert. Verzweigt Einmalzahlung vs. Abonnement anhand der Preisart, nicht anhand von `SessionMode`. |
| `Subscription::subscribe` | Immer `NotSupported`. Abonnements entstehen aus dem Abschluss eines Checkouts + Webhook. |
| `Subscription::update(cancel_at_period_end: Some(true), new_price_refs: None)` | Funktioniert. Verdrahtet zu `subscription_cancel` mit Standard `EffectiveFrom::NextBillingPeriod`. |
| `Subscription::update(new_price_refs: Some(...))` | `NotSupported` in v1. Paddle reserviert den Ersatz von Preis-Sets für seine eigenen Migrations-Flows. |
| `Subscription::update` (No-op) | Funktioniert. Holt den aktuellen Zustand über `subscription_get` erneut. |
| `Subscription::cancel` | Funktioniert, aber `at_period_end` wird **ignoriert** - terminiert immer auf die nächste Abrechnungsperiode. Siehe [unten](#kündigung-ist-immer-terminiert). |
| `Subscription::get` | Funktioniert. |
| `CustomerStore::create_customer` | Funktioniert. |
| `CustomerStore::update_customer` | Funktioniert. |
| `CustomerStore::get_customer` | Funktioniert. |
| `CustomerStore::delete_customer` | `NotSupported`. Verwenden Sie bei Bedarf `update_customer` mit dem Status `archived`. |
| `Payment::*` | Trait ist nicht implementiert. `provider.as_payment()` liefert `None`. |
| `WebhookHandler::*` | Funktioniert. |

Die Invarianten, dass `Payment` nicht implementiert ist, dass
`subscribe`/`delete_customer` `NotSupported` liefern, und die
Ablehnung ungültiger Webhook-Signaturen sind durch immer aktive
Tests in `crates/suprnova-payments-paddle/tests/integration.rs`
festgenagelt, sodass die Matrix oben nicht unbemerkt
auseinanderläuft.

### Kündigung ist immer terminiert

`Subscription::cancel(id, at_period_end)` akzeptiert den Bool für
Trait-Kompatibilität, **verhält sich aber immer wie eine terminierte
Kündigung** - Paddles Enum `EffectiveFrom` ist in `paddle_rust_sdk`
0.18 privat, sodass sofortige Kündigung in v1 nicht möglich ist. Der
Nutzer behält den Zugriff, bis die aktuelle Abrechnungsperiode
endet; dann feuert Paddle `subscription.canceled`, und der Mirror
kippt `status` auf `Canceled`.

Wenn Sie ein UX-seitiges "Jetzt kündigen" wollen, das den
App-Zugriff sofort widerruft, während Paddle die Abrechnung im
Hintergrund auslaufen lässt, sichern Sie den Zugriff über Ihr
eigenes Flag
`subscription.status != Canceled && subscription.cancel_at_period_end == false`
ab und aktualisieren Sie die UI direkt nach der Rückgabe von
`cancel()` - der nächste Webhook wird es bestätigen.

### Kundenlöschung ist "Archivieren über Update"

`delete_customer` liefert `PaymentError::NotSupported`, weil
Paddles öffentliche API überhaupt keinen Delete-Endpunkt anbietet.
Wenn Sie einen Kunden-Datensatz in Paddle unterdrücken müssen, rufen
Sie `update_customer` mit dem Status `archived` auf. Der
Framework-Adapter wrapt das nicht direkt - das Metadaten-Feld ist
der Notausgang:

```rust
provider.update_customer(UpdateCustomerRequest {
    provider_customer_id: customer_id,
    email: None,
    name: None,
    metadata: Some(serde_json::json!({ "status": "archived" })),
}).await?;
```

Bestätigen Sie den exakten Feldpfad gegen Ihre Paddle-API-Version,
bevor Sie das ausliefern - das SDK modelliert das Enum `status`
derzeit nicht direkt.

## Webhook-Signaturverifizierung

Paddle signiert jeden Webhook mit HMAC. Der Header
`Paddle-Signature` sieht aus wie `ts=1716000000,h1=abcdef…`. Der
Adapter delegiert die Verifizierung an `Paddle::unmarshal` aus dem
SDK, das:

- Den Header parst
- Das HMAC mit Ihrem `PADDLE_WEBHOOK_KEY` neu berechnet
- Signaturen ablehnt, deren Zeitstempel außerhalb von
  `MaximumVariance::default()` liegt (zum Zeitpunkt dieses
  Schreibens 5 Sekunden - ältere Replays werden verworfen)

Der `webhook_routes`-Handler des Frameworks ruft `verify` auf, bevor
er irgendetwas anderes tut; ein Fehlschlag liefert
`401 invalid-signature` ohne Body-Leck. Sie schreiben nichts von
diesem Code selbst, aber es lohnt sich zu wissen, dass die
Verifizierung HMAC + Zeitstempel-Toleranz ist, kein statischer
Geheimnisvergleich.

## Form der Webhook-Payload

Die Methoden `extract_payload_ids`, `extract_payment_snapshot` und
`extract_customer_snapshot` des Adapters kennen die Payload-Form
von Paddle, damit das Framework Mirror-Tabellen hydrieren kann.
Kurze Zuordnung:

| Webhook `event_type` | `NeutralEventKind` | Mirror-Effekt |
|---|---|---|
| `transaction.completed`, `transaction.paid` | `PaymentSucceeded` | Upsert von `payments_transactions` |
| `transaction.payment_failed` | `PaymentFailed` | Upsert von `payments_transactions` (fehlgeschlagen) |
| `transaction.billed` | `InvoicePaid` | Upsert von `payments_transactions` mit verknüpfter `provider_subscription_id` |
| `adjustment.created`, `adjustment.updated` | `PaymentRefunded` | Upsert von `payments_transactions` (erstattet) |
| `subscription.created` | `SubscriptionCreated` | `Subscription::get` → Upsert von `payments_subscriptions` + Positionen |
| `subscription.updated`, `.activated`, `.paused`, `.resumed`, `.trialing` | `SubscriptionUpdated` | Wie oben |
| `subscription.canceled` | `SubscriptionCanceled` | Wie oben; setzt `canceled_at`, kippt den Status |
| `customer.created` | `CustomerCreated` | Nur Update: aktualisiert `email`/`metadata`, falls die Mirror-Zeile existiert |
| `customer.updated` | `CustomerUpdated` | Gleich |
| alles andere | `None` (unmapped) | Nur Audit-Zeile - keine Mirror-Änderung |

Paddle legt das Entitätsobjekt direkt unter `data` ab (nicht unter
`data.object` wie Stripe). Beträge kommen als **Strings von
Untereinheiten** an (`"1234"` = 12,34 in der Haupteinheit), nicht
als Dezimalzahlen - der Adapter parst sowohl String- als auch
numerische Formen für Vorwärtskompatibilität. Die Währung kommt als
`currency_code` an, klein geschrieben, und der Snapshot schreibt sie
groß.

### Steuerinklusive Beträge

Paddle meldet Transaktionsbeträge **inklusive Steuer**. Der Mirror
`payments_transactions` des Frameworks teilt das auf:

- `amount_total_minor` - der volle vom Kunden gezahlte Betrag
  (Steuer eingeschlossen)
- `amount_tax_minor` - der Steueranteil

Netto ohne Steuer ist `amount_total_minor - amount_tax_minor`. Das
unterscheidet sich von Stripe (das exklusive Steuer mit
`amount_tax_minor = 0` meldet). Code, der Umsatz über beide
Provider hinweg summiert, muss steuerbewusst sein:

```rust
let net_revenue_minor = txn.amount_total_minor - txn.amount_tax_minor;
```

## Kundenanlage

`CreateCustomerRequest` bildet direkt auf Paddles `customer_create`
ab:

```rust
let cus = provider.create_customer(CreateCustomerRequest {
    user_id: "user_42".into(),       // die User-ID Ihrer App
    email: "alice@example.com".into(),
    name: Some("Alice".into()),
    metadata: None,                  // in v1 nicht an Paddle weitergeleitet
}).await?;
// cus.provider_customer_id == "ctm_01h..."
```

Speichern Sie `cus.provider_customer_id` neben Ihrem
Nutzer-Datensatz. Jeder nachfolgende Aufruf (einen Checkout starten,
ein Abonnement nachschlagen usw.) nimmt die Paddle-Kunden-ID, nicht
die Nutzer-ID der App. Die Mirror-Tabelle `payments_customers` trägt
beide Spalten, sodass ein einziger Index-Lookup Ihnen beide
Richtungen liefert.

`update_customer` und `get_customer` reichen direkt an die
entsprechenden SDK-Methoden durch. `update_customer` akzeptiert
Updates von `email` / `name` und liefert die aufgefrischte
`CustomerRef`. `get_customer` holt einen Snapshot von Paddle (nicht
vom Mirror) - verwenden Sie das, wenn Sie nach einer
außerplanmäßigen Änderung im Paddle-Dashboard einen frischen Read
brauchen.

## Die beabsichtigte `NotSupported`-Form

Ein mit der Codebasis nicht vertrauter Leser könnte annehmen, dass
`PaymentError::NotSupported` bei `subscribe()` und
`delete_customer()` ein vertagtes TODO ist. Das ist es nicht. Die
Einschränkungen sind Teil der Produkt-Oberfläche von Paddle, und
Suprnova kodiert sie, statt lokale Mutationen zu simulieren, die der
Provider nie honorieren wird.

Jede `NotSupported`-Fehlermeldung verweist auf den unterstützten
Workflow:

- `subscribe`: "use `Checkout::start_session` with `SessionMode::Subscription`
  and await the `SubscriptionCreated` webhook"
- `update` mit `new_price_refs`: "Paddle price-set replacement on existing
  subscription not in v1"
- `delete_customer`: "use `UpdateCustomer` with `archived` status"

Verzweigen Sie explizit auf diesen Fehler, wenn Sie
providerunabhängigen Domain-Code schreiben:

```rust
match provider.delete_customer(&cus_id).await {
    Ok(()) => { /* Stripe-Pfad */ }
    Err(PaymentError::NotSupported(_)) => {
        // Paddle-Pfad - stattdessen über Update archivieren
        provider.update_customer(UpdateCustomerRequest {
            provider_customer_id: cus_id,
            email: None,
            name: None,
            metadata: Some(serde_json::json!({ "status": "archived" })),
        }).await?;
    }
    Err(e) => return Err(e),
}
```

### Warum Suprnova abweicht

Laravel Cashier ist Stripe-exklusiv und modelliert Abonnements als
App-besessen: `$user->newSubscription('default', 'pri_pro')->create()`
ist so geformt, als würde die Anwendung das Abonnement initiieren.
Bei einem Gateway mit direkter Erfassung ist das zutreffend. Bei
einem MoR ist es eine Lüge - der Provider ist der Akteur, nicht Ihre
App.

Suprnovas Zahlungs-Oberfläche ist Provider-neutral, also schlägt sie
sich auf keine Seite. Die Trait-Oberfläche (`subscribe`, `update`,
`cancel`, `get`) ist die generische Form; jeder Adapter
implementiert, was sein Provider offenlegt, und liefert
`NotSupported`, wo das Produktmodell des Providers abweicht. Der
Stripe-Adapter implementiert `subscribe`. Der Paddle-Adapter tut das
nicht, weil Paddle es nicht zulässt. Den Unterschied hinter einem
gefälschten lokalen "create" zu verstecken würde den Adapter dazu
bringen, Sie zu belügen - Suprnova bevorzugt das typisierte
`NotSupported` mit einer Migrationsmeldung im Fehler-String.

Dieselbe Abweichung gilt für `Payment` (serverseitige Erfassung).
Stripe implementiert es; Paddle nicht, und `provider.as_payment()`
liefert `None`. Code, der Belastung/Erfassung/Rückerstattung
braucht, muss `as_payment().is_some()` prüfen, statt blind
aufzurufen - siehe
[Zahlungen](payments.md#payment--optional-server-side-capture).

## Ihre Integration testen

Die Crate enthält immer aktive Invarianten-Tests (kein
Netzwerkzugriff nötig) plus einen umgebungsgesicherten
Integrationstest gegen Paddles Sandbox-API:

```bash
# Immer aktive Invarianten (Signaturablehnung, NotSupported-Formen):
cargo test -p suprnova-payments-paddle

# Plus Sandbox-Integration (erfordert PADDLE_API_KEY usw.):
PADDLE_API_KEY=pdl_sdbx_apikey_... \
PADDLE_WEBHOOK_KEY=pdl_ntfset_... \
PADDLE_CLIENT_TOKEN=test_... \
PADDLE_ENVIRONMENT=sandbox \
  cargo test -p suprnova-payments-paddle
```

Die Invarianten-Tests sind die, an denen Sie sich in Ihrem eigenen
Code orientieren sollten, wenn Sie adapterspezifische Abstraktionen
bauen. Drei Testformen, die sich zu kopieren lohnen:

```rust
use suprnova::payments::*;
use suprnova_payments_paddle::{PaddleEnvironment, PaddleProvider};

#[test]
fn paddle_does_not_implement_payment_trait() {
    let p = PaddleProvider::new(
        "pdl_sdbx_apikey_test",
        "pdl_ntfset_test",
        "test_client",
        PaddleEnvironment::Sandbox,
    ).expect("provider construction");
    assert!(p.as_payment().is_none());
}

#[tokio::test]
async fn paddle_subscribe_returns_not_supported() {
    let p = /* ...wie oben... */;
    let err = p.subscribe(SubscribeRequest {
        customer_ref: "ctm_test".into(),
        price_refs: vec!["pri_test".into()],
        trial_days: None,
        idempotency_key: None,
        metadata: None,
    }).await.unwrap_err();
    assert!(matches!(err, PaymentError::NotSupported(_)));
}

#[test]
fn webhook_verify_rejects_bad_signature() {
    let p = /* ...wie oben... */;
    let mut headers = http::HeaderMap::new();
    headers.insert("paddle-signature", "ts=1234,h1=deadbeef".parse().unwrap());
    let ctx = WebhookContext {
        body: b"{}",
        headers: &headers,
        remote_addr: None,
    };
    assert!(matches!(p.verify(&ctx).unwrap_err(), PaymentError::WebhookSignature(_)));
}
```

Für lokale Ende-zu-Ende-Tests, ohne Paddle überhaupt anzusprechen,
liefert das Framework `MockPaymentProvider`. Wie Paddle liefert
`as_payment()` des Mocks `None` (keine serverseitige Erfassung),
sodass Code, der auf `as_payment().is_some()` verzweigt, unter dem
Mock denselben Pfad nimmt wie unter Paddle. Das `subscribe()` des
Mocks liefert `Ok` (anders als Paddle), sodass Tests, die den
`NotSupported`-Zweig prüfen müssen, den echten `PaddleProvider`
verwenden sollten. Binden Sie in Tests den Mock statt des echten
Providers:

```rust
use std::sync::Arc;
use suprnova::payments::{MockPaymentProvider, PaymentProviderRegistry};

#[suprnova_test]
async fn checkout_flow() {
    PaymentProviderRegistry::bind("paddle", Arc::new(MockPaymentProvider::new()));
    // ...Ihren Controller gegen den Mock ausführen...
}
```

## Produktions-Checkliste

Bevor Sie `PADDLE_ENVIRONMENT=production` umschalten:

- [ ] Alle vier Umgebungsvariablen sind in Produktions-Secrets
  gesetzt, nicht committet
- [ ] Die Webhook-Endpunkt-URL ist in den
  *Notifications*-Einstellungen des Paddle-Dashboards registriert,
  und das dort erzeugte Zielgeheimnis stimmt mit
  `PADDLE_WEBHOOK_KEY` überein
- [ ] Der Katalog hat Live- (nicht Sandbox-) Preis-IDs, und die IDs,
  auf die Sie in `price_refs` verweisen, existieren im Live-Katalog
- [ ] Ihre `success_return_url` und `cancel_return_url` zeigen auf
  HTTPS-Endpunkte (Paddle lehnt HTTP in Produktion ab)
- [ ] Sie haben entschieden, wie Ihre App reagiert, wenn
  `subscribe()`, `delete_customer()` oder `update(price_refs)`
  `NotSupported` liefern - entweder im Code verzweigen oder
  dokumentieren, dass diese Flows nur für MoR gelten
- [ ] Sie haben die Kündigungs-UX stresstestet: Kündigung ist immer
  terminiert, sodass "Sie haben gekündigt, behalten aber Zugriff bis
  DATUM" die Meldung ist, die Ihre UI zeigen sollte
- [ ] Sie haben den Abonnement-Ankunfts-Webhook stresstestet: es
  gibt ein Zeitfenster, in dem der Kunde bezahlt hat, aber der
  Mirror noch keine Zeile hat
- [ ] Sie aggregieren Umsatz korrekt: Paddle-Beträge sind
  steuerinklusive, Stripe-Beträge sind steuerexklusiv

## Nächste Schritte

- [Zahlungen](payments.md) - die generische Fünf-Trait-Oberfläche
  und der Mirror-Hydrations-Vertrag des Webhook-Handlers
- [Zahlungen - Frontend Integration](payments-frontend.md) -
  paddle.js-Inline-Checkout in Svelte / React / Vue
- [Zahlungen - Provider-Leitfaden](payments-provider-guide.md) -
  schreiben Sie Ihre eigene Adapter-Crate von Anfang bis Ende
- [Konfiguration](configuration.md) - typisierte
  Config-Registrierung, in die die Paddle-Umgebungsvariablen sich
  einklinken
- [Application Bootstrap](bootstrap.md) - wo
  `PaymentProviderRegistry::bind` in Ihrer App tatsächlich lebt
