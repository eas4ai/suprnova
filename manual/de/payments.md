# Zahlungen

Suprnovas Zahlungs-Oberfläche ist Provider-neutral. Sie wählen eine
Adapter-Crate - Stripe, Paddle oder eine, die Sie selbst schreiben -,
registrieren sie beim Boot, und Ihr Domain-Code ruft dieselben vier
Kern-Traits auf (plus einen optionalen fünften für serverseitige
Erfassung), unabhängig davon, welcher Provider dahintersteht.
Mirror-Tabellen in Ihrer Datenbank werden von Webhooks synchron
gehalten, sodass Ihr Domain-Code aus Ihrer eigenen Datenbank liest,
statt bei jeder Abfrage die Provider-API anzusprechen.

Kein Feature ist an einen einzelnen Provider gebunden. Stripes Modell
der direkten Erfassung und Paddles Merchant-of-Record-Modell passen
beide in denselben Trait-Vertrag. Die einzige Oberfläche, die
abweicht, ist `Payment` (serverseitige Erfassung), und die ist
optional - Paddle braucht sie nicht, also implementiert Paddle sie
nicht. Provider melden ihre Fähigkeit, indem sie
`PaymentProvider::as_payment()` überschreiben, um
`Some(&dyn Payment)` zurückzugeben; Aufrufer fragen das zur Laufzeit
ab.

## Warum Suprnova abweicht

Laravel liefert Cashier als hauseigene Stripe-Integration in den
Kern-Docs. Das ist praktisch, aber Stripe-exklusiv - ein zweiter
Provider bedeutet, Cashier zu forken oder eine parallele Oberfläche
zu bauen. Suprnova behandelt Zahlungs-Provider wie Cache- und
Storage-Treiber: ein generisches Trait-Set, austauschbare Adapter.
Ihr Domain-Code benennt niemals `StripeProvider` oder
`PaddleProvider`; er ruft `provider.subscribe(...)` gegen ein aus
einer Registry aufgelöstes `Arc<dyn PaymentProvider>` auf, und der
Provider dahinter ist nur eine Bootstrap-Änderung davon entfernt,
etwas anderes zu sein.

## Schnellstart

Fügen Sie die Adapter-Crate hinzu. Bis Suprnova sein v0.1-Release
ausliefert, werden das Framework und seine Adapter-Crates über Git
statt über crates.io bezogen:

```toml
# Cargo.toml
[dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.2.4" }
suprnova-payments-stripe = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.2.4" }
```

Registrieren Sie den Provider und den Webhook-Router beim Boot. Der
Webhook-Router ist ein gewöhnlicher `Router`, den Sie in Ihr
`routes::register()` hineinkomponieren:

```rust,ignore
// src/bootstrap.rs
use std::sync::Arc;
use suprnova::payments::PaymentProviderRegistry;
use suprnova_payments_stripe::StripeProvider;

pub async fn register() {
    let stripe = StripeProvider::from_env().expect("Stripe env vars not set");
    PaymentProviderRegistry::bind("stripe", Arc::new(stripe));
}
```

```rust,ignore
// src/routes.rs
use std::sync::Arc;
use suprnova::payments::webhook_routes;
use suprnova::container::App;
use suprnova::Router;
use sea_orm::DatabaseConnection;

/// `Application::routes(routes::register)` ruft dies einmal beim Boot auf.
/// Wir starten mit dem Payments-Webhook-Router und legen dann den Rest der
/// App-Routen mit normalen `.get(...)` / `.post(...)`-Aufrufen darüber.
pub fn register() -> Router {
    let db: Arc<DatabaseConnection> = App::get().expect("db not bound");

    webhook_routes(db)
        .get("/", crate::controllers::home::index)
        .post("/login", crate::controllers::auth::login)
        // ... der Rest Ihrer Routen ...
        .into()
}
```

`webhook_routes(db)` liefert einen `Router`, der nur
`POST /webhooks/payments/{provider}` enthält. Weil `Router::get` und
`Router::post` jeweils einen `RouteBuilder` liefern, der über `.into()`
zurück zu `Router` konvertiert, ist das Anketten auf den
Payments-Router der direkteste Weg zu komponieren. Wenn Sie für Ihre
normalen Routen bereits das Makro `routes!{}` verwenden, legen Sie den
Webhook-POST in denselben Block - `webhook_routes` ist ein
Komfort-Wrapper um einen einzelnen `Router::new().post(...)`-Aufruf.

Schlagen Sie in Ihrem Controller den Provider nach, legen Sie einen
Kunden an und öffnen Sie eine Checkout-Session:

```rust,ignore
// src/controllers/billing.rs
use std::sync::Arc;
use suprnova::payments::*;

pub async fn start_checkout(
    user_id: String,
    email: String,
) -> PaymentResult<SessionPayload> {
    let provider = PaymentProviderRegistry::get("stripe")
        .ok_or_else(|| PaymentError::Internal("stripe not registered".into()))?;

    let customer = provider.create_customer(CreateCustomerRequest {
        user_id,
        email,
        name: None,
        metadata: None,
    }).await?;

    provider.start_session(StartSessionRequest {
        mode: SessionMode::Subscription,
        customer_ref: customer.provider_customer_id,
        price_refs: vec!["price_pro_monthly".into()],
        success_return_url: "https://app.example/billing/success".into(),
        cancel_return_url: "https://app.example/billing/cancel".into(),
        amount_hint: None,
        idempotency_key: None,
        metadata: None,
    }).await
}
```

Diese `SessionPayload` geht in die Props Ihrer Inertia-Seite. Das
Frontend dispatcht auf `payload.flow`, um das richtige Widget zu
rendern - siehe
[Zahlungen - Frontend Integration](payments-frontend.md).

## Einen Adapter wählen

### Stripe

```toml
# Cargo.toml
suprnova-payments-stripe = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.2.4" }
```

Erforderliche Umgebungsvariablen:

| Variable | Beschreibung |
|---|---|
| `STRIPE_SECRET_KEY` | Secret Key (`sk_live_…` / `sk_test_…`) |
| `STRIPE_PUBLISHABLE_KEY` | Publishable Key (`pk_live_…` / `pk_test_…`) |
| `STRIPE_WEBHOOK_SIGNING_SECRET` | Signing Secret des Webhook-Endpunkts (`whsec_…`) |

```rust,ignore
use suprnova_payments_stripe::StripeProvider;
use std::sync::Arc;
use suprnova::payments::PaymentProviderRegistry;

// Aus der Umgebung (in Produktion empfohlen):
let stripe = StripeProvider::from_env().expect("Stripe env vars not set");

// Oder direkt konstruieren:
let stripe = StripeProvider::new("sk_test_...", "pk_test_...", "whsec_...");

PaymentProviderRegistry::bind("stripe", Arc::new(stripe));
```

Stripe implementiert jeden Trait, einschließlich der optionalen
`Payment` (serverseitige Erfassung über PaymentIntents) und
`Promotions` (Prägen von Promotion-Codes über `/v1/promotion_codes`).
Sowohl `provider.as_payment()` als auch `provider.as_promotions()`
liefern `Some`.

### Paddle

```toml
# Cargo.toml
suprnova-payments-paddle = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.2.4" }
```

Erforderliche Umgebungsvariablen:

| Variable | Beschreibung |
|---|---|
| `PADDLE_API_KEY` | API-Key (`pdl_live_apikey_…` / `pdl_sdbx_apikey_…`) |
| `PADDLE_WEBHOOK_KEY` | Secret des Notification Destination (`pdl_ntfset_…`) |
| `PADDLE_CLIENT_TOKEN` | Clientseitiges Token (`live_…` / `test_…`) |
| `PADDLE_ENVIRONMENT` | Optional, Standard ist `"sandbox"` |

```rust,ignore
use suprnova_payments_paddle::{PaddleProvider, PaddleEnvironment};
use std::sync::Arc;
use suprnova::payments::PaymentProviderRegistry;

// Aus der Umgebung:
let paddle = PaddleProvider::from_env().expect("Paddle env vars not set");

// Oder direkt konstruieren:
let paddle = PaddleProvider::new(
    "pdl_sdbx_apikey_...",
    "pdl_ntfset_...",
    "test_...",
    PaddleEnvironment::Sandbox,
).expect("Paddle client init failed");

PaymentProviderRegistry::bind("paddle", Arc::new(paddle));
```

Paddle ist ein Merchant of Record - es verwaltet Steuern, Dunning und
den gesamten Subscription-Lifecycle. Es legt keine serverseitige
Erfassung offen, daher ist `Payment` nicht implementiert. Ein Aufruf
von `provider.as_payment()` liefert `None`. Subscriptions werden
indirekt angelegt: Rufen Sie `Checkout::start_session` auf, schließen
Sie das Paddle-Widget ab, und der `SubscriptionCreated`-Webhook trifft
ein, um die Subscription-ID zu bestätigen.

## Der Trait-Split

`PaymentProvider` ist ein Sammel-Trait, der vier universelle Traits
bündelt - `Checkout`, `Subscription`, `CustomerStore`,
`WebhookHandler` -, die jeder Adapter implementiert. Zwei weitere
Traits sind optional: `Payment` (serverseitige Erfassung ergibt nur
bei Gateways wie Stripe Sinn) und `Promotions` (Prägen von
Promotion-Codes). Adapter nehmen daran teil, indem sie
`PaymentProvider::as_payment()` / `PaymentProvider::as_promotions()`
überschreiben.

```rust,ignore
pub trait PaymentProvider: Checkout + Subscription + CustomerStore + WebhookHandler {
    fn name(&self) -> &'static str;

    /// Liefert `Some`, falls dieser Provider auch `Payment` (Server-Erfassung)
    /// implementiert. Der Standardwert ist `None`.
    fn as_payment(&self) -> Option<&dyn Payment> {
        None
    }

    /// Liefert `Some`, falls dieser Provider auch `Promotions` (Prägen von
    /// Promotion-Codes) implementiert. Der Standardwert ist `None`.
    fn as_promotions(&self) -> Option<&dyn Promotions> {
        None
    }
}
```

### `Checkout` - universell, öffnet das Client-Widget

Jeder Provider implementiert `Checkout`. Rufen Sie `start_session`
auf, um eine Flow-getaggte `SessionPayload` zu erhalten, die Ihr
Frontend rendert. `session_status` (Standard: `NotSupported`;
überschrieben von Providern, deren Sessions abgefragt werden können,
z. B. Stripe) meldet den maßgeblichen Zustand einer zuvor gestarteten
Session auf Provider-Seite.

```rust,ignore
#[async_trait]
pub trait Checkout: Send + Sync {
    async fn start_session(&self, req: StartSessionRequest) -> PaymentResult<SessionPayload>;

    async fn session_status(&self, provider_session_id: &str)
        -> PaymentResult<CheckoutSessionState>;
}
```

Felder von `StartSessionRequest`:

| Feld | Typ | Beschreibung |
|---|---|---|
| `mode` | `SessionMode` | `OneOff` oder `Subscription` |
| `customer_ref` | `String` | Provider-Kunden-ID aus `CustomerStore::create_customer` |
| `price_refs` | `Vec<String>` | Provider-Preis-/Produkt-IDs |
| `success_return_url` | `String` | Wohin der Nutzer nach der Zahlung geschickt wird |
| `cancel_return_url` | `String` | Wohin der Nutzer geschickt wird, wenn er abbricht |
| `amount_hint` | `Option<Money>` | Override oder Hinweis für Einmalbeträge |
| `idempotency_key` | `Option<String>` | Für sichere Wiederholungen |

`session_status` ist die serverseitige Verifizierungs-Primitive für
Redirect-Flows. Wenn der Kunde auf Ihrer Rückkehrseite landet,
vertrauen Sie NICHT den Query-Parametern, die sein Browser
mitgebracht hat - übergeben Sie die `provider_session_id`, die Sie
zur Zeit von `start_session` aufgezeichnet haben, und verzweigen Sie
anhand des Ergebnisses:

```rust,ignore
match provider.session_status(&order.provider_session_id).await? {
    CheckoutSessionState::Complete { paid: true, payment_ref, amount_total } => {
        // Bestellung erfüllen. `payment_ref` (z. B. Stripes `pi_…`)
        // korreliert mit `Payment`-Operationen und dem Mirror payments_transactions.
    }
    CheckoutSessionState::Complete { paid: false, .. } => { /* Settlement ausstehend */ }
    CheckoutSessionState::Open => { /* Kunde hat die Zahlung noch nicht abgeschlossen */ }
    CheckoutSessionState::Expired => { /* Session abgelaufen - Bestellung schließen */ }
}
```

Derselbe Aufruf treibt auch Abgleichsläufe an: pollen Sie erneut
Bestellungen, die in Ihrer Datenbank noch offen sind, und erfüllen
Sie diejenigen, deren Sessions abgeschlossen wurden, nachdem der
Kunde den Tab geschlossen hat.

### `Payment` - optional, serverseitige Erfassung

Nur Provider, die serverseitige Erfassung anbieten, implementieren
`Payment`. Stripe tut das; Paddle nicht. So prüfen Sie es zur
Laufzeit:

```rust,ignore
let provider = PaymentProviderRegistry::get("stripe").unwrap();
if let Some(payment) = provider.as_payment() {
    let result = payment.charge(ChargeRequest {
        customer_ref: "cus_...".into(),
        payment_method_ref: "pm_...".into(),
        amount: Money::from_minor_units(2999, Currency::USD),
        description: Some("Pro plan one-off".into()),
        idempotency_key: Some("charge_user42_order99".into()),
        metadata: None,
    }).await?;
}
```

Vollständige `Payment`-Schnittstelle:

```rust,ignore
#[async_trait]
pub trait Payment: Send + Sync {
    async fn charge(&self, req: ChargeRequest) -> PaymentResult<ChargeResult>;
    async fn capture(&self, provider_transaction_id: &str) -> PaymentResult<ChargeResult>;
    async fn refund(&self, req: RefundRequest) -> PaymentResult<RefundResult>;
    async fn void(&self, provider_transaction_id: &str) -> PaymentResult<()>;
    async fn status(&self, provider_transaction_id: &str) -> PaymentResult<PaymentStatus>;
}
```

`ChargeResult` ist ein mit `kind` getaggtes Enum - siehe den
Abschnitt [Money und ChargeResult](#chargeresult).

### `Promotions` - optional, Promotion-Codes prägen

Provider mit einer Promotion-Code-Oberfläche implementieren
`Promotions`. Das Rabatt-Objekt selbst (ein Prozent- oder
Betrags-Coupon) wird vorab angelegt - typischerweise einmal, im
Dashboard des Providers -, und dieser Trait prägt daraus *Codes*,
jeder beschränkt auf einen Kunden und ein Einlösefenster. Das ist die
Form, die Rückgewinnungs- und Upsell-Kampagnen brauchen: jeder
Empfänger bekommt einen persönlichen Code, von niemand anderem
nutzbar und nach Ablauf des Fensters tot.

```rust,ignore
let provider = PaymentProviderRegistry::get("stripe").unwrap();
if let Some(promotions) = provider.as_promotions() {
    let minted = promotions.create_promotion_code(CreatePromotionCodeRequest {
        coupon_ref: "coupon_15off".into(),          // vorab angelegter Coupon
        customer_ref: "cus_...".into(),              // nur dieser Kunde darf einlösen
        expires_at: Some(chrono::Utc::now() + chrono::Duration::days(7)),
        max_redemptions: Some(1),                   // einmalig nutzbar
    }).await?;
    // `minted.code` per E-Mail an den Kunden schicken; er gibt ihn beim
    // Checkout ein, und der Provider setzt jede Einschränkung durch.
}
```

`MockPaymentProvider` implementiert `Promotions` (Codes werden als
`PROMO_MOCK_n` geprägt) und zeichnet jede Anfrage auf - assertieren
Sie in Tests auf `recorded_promotion_requests()`.

### `Subscription` - subscribe, update, cancel, get

```rust,ignore
#[async_trait]
pub trait Subscription: Send + Sync {
    async fn subscribe(&self, req: SubscribeRequest) -> PaymentResult<SubscriptionResult>;
    async fn update(&self, req: UpdateSubscriptionRequest) -> PaymentResult<SubscriptionResult>;
    async fn cancel(&self, provider_subscription_id: &str, at_period_end: bool) -> PaymentResult<SubscriptionResult>;
    async fn get(&self, provider_subscription_id: &str) -> PaymentResult<SubscriptionResult>;
}
```

Zum Periodenende kündigen (Zugriff bleibt bis zum Ende des
Abrechnungszyklus erhalten):

```rust,ignore
let sub = provider.cancel(&sub_id, true).await?;
// sub.cancel_at_period_end == true, sub.status == Active

// Sofort kündigen:
let sub = provider.cancel(&sub_id, false).await?;
// sub.status == Canceled
```

Hinweis: `Paddle::subscribe` liefert `PaymentError::NotSupported` -
Paddle legt Abonnements über den Abschluss eines Checkouts an, nicht
über direkte API-Aufrufe. Verwenden Sie `Checkout::start_session` und
warten Sie auf den Webhook `SubscriptionCreated`.

### `CustomerStore` - create, update, get, delete

```rust,ignore
#[async_trait]
pub trait CustomerStore: Send + Sync {
    async fn create_customer(&self, req: CreateCustomerRequest) -> PaymentResult<CustomerRef>;
    async fn update_customer(&self, req: UpdateCustomerRequest) -> PaymentResult<CustomerRef>;
    async fn get_customer(&self, provider_customer_id: &str) -> PaymentResult<CustomerRef>;
    async fn delete_customer(&self, provider_customer_id: &str) -> PaymentResult<()>;
}
```

`CreateCustomerRequest` nimmt `user_id`, `email`,
`name: Option<String>` und `metadata: Option<Value>`. `CustomerRef`
kommt mit `provider_customer_id` zurück - speichern Sie das neben
Ihrem Nutzer-Datensatz, um es in nachfolgenden Aufrufen zu
verwenden.

### `WebhookHandler` - verify, parse, and extract

```rust,ignore
#[async_trait]
pub trait WebhookHandler: Send + Sync {
    fn verify(&self, ctx: &WebhookContext<'_>) -> PaymentResult<()>;
    fn parse_event(&self, body: &[u8]) -> PaymentResult<WebhookEvent>;

    /// Zieht Entitäts-IDs aus der rohen Payload, damit das Framework weiß,
    /// welche Mirror-Zeilen zu hydrieren sind. Standard liefert eine leere
    /// `PayloadIds`.
    fn extract_payload_ids(&self, event: &WebhookEvent) -> PayloadIds;

    /// Baut einen `PaymentSnapshot` aus einem Payment-/Invoice-Ereignis.
    /// Der Standardwert ist `None`, was den Upsert von
    /// `payments_transactions` überspringt.
    fn extract_payment_snapshot(&self, event: &WebhookEvent) -> Option<PaymentSnapshot>;

    /// Baut einen `CustomerSnapshot` aus einem Kunden-Ereignis. Der
    /// Standardwert ist `None`, was die Aktualisierung von E-Mail /
    /// Metadaten auf der bestehenden Zeile überspringt.
    fn extract_customer_snapshot(&self, event: &WebhookEvent) -> Option<CustomerSnapshot>;
}
```

In der Praxis rufen Sie keine dieser Methoden je direkt auf -
`webhook_routes` ruft sie für jeden eingehenden Webhook auf. Sie
sitzen auf dem Trait, damit Adapter-Crates providerspezifische
Signaturverifizierung, Event-Parsing und Payload-Extraktion auf
testbare Weise implementieren können. Die `extract_*`-Methoden haben
alle sinnvolle Standardwerte; die mitgelieferten Stripe- und
Paddle-Adapter überschreiben sie mit Implementierungen, die die
jeweilige Provider-Form kennen (Stripe greift in `data.object.*`,
Paddle in `data.*`).

## Die per Flow getaggte Inertia-Payload

`start_session` liefert ein Enum `SessionPayload`, das als JSON mit
einem Diskriminator-Feld `flow` serialisiert. Ihr Frontend verzweigt
auf `flow`, um das richtige Widget zu rendern:

```rust,ignore
#[serde(tag = "flow", rename_all = "snake_case")]
pub enum SessionPayload {
    StripeElements {
        client_secret: String,
        publishable_key: String,
        provider_session_id: String,
    },
    StripeCheckoutRedirect {
        url: String,
        provider_session_id: String,
    },
    PaddleInline {
        transaction_id: String,
        customer_token: Option<String>,
        client_token: String,
    },
    /// Mobile-Money-Flow - kein Redirect, kein Embed. Das Frontend zeigt
    /// eine nutzerseitige Meldung, die den Kunden bittet, auf seinem
    /// Telefon zu bestätigen (USSD-Prompt oder Operator-App), und pollt
    /// dann den Provider über `provider_transaction_id` auf Statusupdates.
    MobileMoneyPrompt {
        provider_transaction_id: String,
        message: String,
        operator: MobileMoneyOperator,
    },
    Redirect {
        url: String,
        provider_session_id: String,
    },
}
```

Serialisierte Form einer `StripeElements`-Payload:

```json
{
  "flow": "stripe_elements",
  "client_secret": "pi_..._secret_...",
  "publishable_key": "pk_live_...",
  "provider_session_id": "pi_..."
}
```

Eine `MobileMoneyPrompt`-Payload sieht so aus - es gibt keine URL,
weil der Kunde Ihre Seite nie verlässt; das Frontend rendert
`message` und beginnt zu pollen:

```json
{
  "flow": "mobile_money_prompt",
  "provider_transaction_id": "ch_mm_...",
  "message": "Check your phone for the MTN MoMo prompt.",
  "operator": { "kind": "mtn_momo" }
}
```

Geben Sie aus Ihrem Controller die Variante zurück, die der Provider
liefert, als Inertia-Props. Die Frontend-Integration wird in
[Zahlungen - Frontend Integration](payments-frontend.md) beschrieben.

## Mirror-Tabellen

Sechs Tabellen werden von der Framework-Migration angelegt. Binden
Sie den öffentlichen Alias ein und nehmen Sie ihn in den Migrator
Ihrer App auf:

```rust,ignore
use sea_orm_migration::{MigrationTrait, MigratorTrait};
use suprnova::payments::migrations::CreatePaymentsTables;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            // ... Ihre anderen Migrationen ...
            Box::new(CreatePaymentsTables),
        ]
    }
}
```

Dasselbe Modul exportiert außerdem einen Helfer
`pub fn migrations() -> Vec<Box<dyn MigrationTrait>>`, falls Sie den
lieber aufrufen und das Ergebnis in Ihre eigene Liste einstreuen
wollen.

### Tabellenübersicht

| Tabelle | Zweck |
|---|---|
| `payments_customers` | Eine Zeile pro Paar `(provider, user_id)` |
| `payments_payment_methods` | Gespeicherte Zahlungsmethoden pro Kunde |
| `payments_subscriptions` | Zustand des Abonnement-Lebenszyklus |
| `payments_subscription_items` | Positionen innerhalb eines Abonnements |
| `payments_transactions` | Einmalzahlungen und Abonnement-Rechnungen |
| `payments_webhook_events` | Audit-Log und Idempotenz-Wache |

Jede Tabelle hat eine JSON-Spalte `provider_metadata`. Wenn die
neutrale Repräsentation des Frameworks ein providerspezifisches Feld
nicht abdeckt, lesen Sie es von dort.

### Transaktionstabelle

`payments_transactions` teilt Beträge in `amount_total_minor` und
`amount_tax_minor` auf. Stripe meldet Beträge exklusive Steuer - die
Steuer ist auf der Transaktionszeile null, und alle Steuerdaten
liegen in `provider_metadata`. Paddle meldet Beträge inklusive
Steuer und setzt `amount_tax_minor` auf den Steueranteil. Beide
Darstellungen funktionieren; addieren Sie
`amount_total_minor - amount_tax_minor` für den Nettobetrag.

### Webhook-Ereignis-Tabelle

`payments_webhook_events` hat einen Index
`UNIQUE(provider, provider_event_id)`. Jeder eingehende Webhook wird
dagegen geprüft, bevor er verarbeitet wird - Duplikate liefern 200 OK
ohne erneute Verarbeitung. Das ist tragend: Stripe, Paddle und die
meisten Provider wiederholen fehlgeschlagene Webhooks aggressiv.

### Vorbehalte

Domain-Code liest aus den Mirror-Tabellen, nicht direkt aus der
Provider-API. Mutationen (Abonnement anlegen, kündigen usw.) gehen an
den Provider; der resultierende Webhook synchronisiert die
Mirror-Tabellen zurück. Das bedeutet, es gibt ein kurzes Zeitfenster
zwischen einer Mutation und dem Eintreffen des Webhooks, in dem Ihre
Mirror-Tabellen hinterherhinken. Legen Sie Ihre UX darauf aus (zeigen
Sie "In Bearbeitung"-Zustände, verlassen Sie sich für die sofortige
Bestätigung auf die Redirect-URLs des Providers).

## Webhook-Verarbeitung

Binden Sie die Webhook-Ingress-Route einmal beim Bootstrap ein -
siehe das Routen-Beispiel im Abschnitt
[Schnellstart](#schnellstart) für das Kompositionsmuster.
`webhook_routes(db)` liefert einen `Router`, der den einzigen ins
Framework eingebauten Handler `POST /webhooks/payments/{provider}`
trägt. Sie hängen Ihre eigenen Routen daran (oder rufen die
zugrunde liegenden Primitiven der Route direkt in Ihrem eigenen
`routes!{}`-Block auf).

Der Framework-Handler tut für jede Anfrage Folgendes:

1. Sucht den benannten Provider in `PaymentProviderRegistry`.
2. Ruft `WebhookHandler::verify` auf, um die Signatur zu prüfen.
   Liefert bei Fehlschlag 401.
3. Ruft `WebhookHandler::parse_event` auf, um ein `WebhookEvent` zu
   bauen. Liefert bei Parse-Fehlschlag 400.
4. Prüft `payments_webhook_events` auf eine bestehende Zeile mit
   demselben `(provider, provider_event_id)`. Wenn gefunden, liefert
   es sofort 200 - das ist die Idempotenz-Wache.
5. Fügt die Audit-Zeile ein.

### Struktur von WebhookEvent

```rust,ignore
pub struct WebhookEvent {
    pub provider: String,
    pub provider_event_id: String,
    pub provider_event_type: String,        // roher Provider-String, z. B. "customer.subscription.created"
    pub neutral: Option<NeutralEventKind>,  // auf die Framework-Taxonomie abgebildet, oder None für providerspezifische Ereignisse
    pub raw_payload: Value,                 // vollständiger JSON-Body für den Fallthrough
}
```

`NeutralEventKind` deckt den gemeinsamen Pfad ab:

```rust,ignore
pub enum NeutralEventKind {
    PaymentSucceeded,
    PaymentFailed,
    PaymentRefunded,
    PaymentDisputed,
    SubscriptionCreated,
    SubscriptionUpdated,
    SubscriptionCanceled,
    InvoicePaid,
    InvoiceFailed,
    CustomerCreated,
    CustomerUpdated,
}
```

Wenn `neutral` gleich `None` ist, ist das Ereignis providerspezifisch.
Lesen Sie `provider_event_type` und `raw_payload` für die
vollständigen Daten.

### Mirror-Tabellen-Hydration

Nachdem die Audit-Zeile persistiert ist, verteilt das Framework das
Ereignis anhand von `neutral` an die passende Mirror-Tabelle. **Alle
Mirror-Schreibvorgänge für ein Ereignis laufen zusammen mit
`mark_processed` in einer einzigen DB-Transaktion** - ein partieller
Mirror-Zustand ist nie beobachtbar. Entweder committet alles
zusammen, oder alles wird zurückgerollt.

| `NeutralEventKind`               | Mirror-Effekt                                                                                       |
|----------------------------------|-----------------------------------------------------------------------------------------------------|
| `SubscriptionCreated/Updated`    | Ruft `Subscription::get(id)` beim Provider auf, upsertet `payments_subscriptions`, synchronisiert Positionen. |
| `SubscriptionCanceled`           | Wie oben; setzt zusätzlich `canceled_at` und kippt `status` auf `canceled` in der bestehenden Zeile. |
| `PaymentSucceeded / Failed / Refunded / Disputed` | Upsertet `payments_transactions` aus dem Snapshot, den der Provider aus `raw_payload` erzeugt. |
| `InvoicePaid / InvoiceFailed`    | Upsertet `payments_transactions` mit verknüpfter `provider_subscription_id`.                        |
| `CustomerCreated / CustomerUpdated` | Aktualisiert `email` / `provider_metadata` der bestehenden `payments_customers`-Zeile aus dem `CustomerSnapshot` des Providers. **Fügt nie ein.** |
| `None` (unmapped)                | Nur Audit-Zeile - keine Mirror-Änderung.                                                             |

Der Kunden-Mirror ist auf dem Webhook-Pfad absichtlich nur
aktualisierbar. `user_id` ist `NOT NULL`, und nur die App weiß,
welchem Nutzer ein Provider-Kunde gehört (die Verknüpfung wird von
Ihrem Code unmittelbar nach `CustomerStore::create_customer`
angelegt). Außerplanmäßige Kunden - etwa im Stripe-Dashboard
angelegt - werden protokolliert, aber nie in den Mirror
synthetisiert.

### Fehlerbehebungs-Vertrag

Der Handler behandelt Provider-Wiederholungen als
Wiederherstellungsmechanismus:

- **Hydration gelingt:** Transaktion committet, `processed_at`
  gesetzt, `process_error` gelöscht. Antwort: `200 ok`.
- **Hydration schlägt fehl:** Transaktion wird zurückgerollt (kein
  partieller Mirror-Zustand), die Audit-Zeile behält
  `processed_at = NULL`, und `process_error` verzeichnet den
  Fehlschlag. Antwort: `503 hydration-failed` - der Provider wird es
  mit Backoff erneut versuchen.
- **Provider wiederholt das fehlgeschlagene Ereignis:** Die
  Idempotenzprüfung sieht die bestehende Audit-Zeile, aber
  `processed_at IS NULL`, also läuft die Hydration erneut. Die
  Wiederholung ersetzt den veralteten `process_error` durch das
  Ergebnis des aktuellen Versuchs.
- **Provider wiederholt ein erfolgreiches Ereignis:** Die
  Idempotenzprüfung sieht `processed_at IS NOT NULL`, liefert sofort
  `200 duplicate`. Keine erneute Hydration.

Ein Abonnement-/Kunden-Ereignis mit fehlender `subscription_id` /
`customer_id` in der Payload wird als `Validation`-Fehler behandelt
(ebenfalls 503 + `process_error` verzeichnet). Stiller Erfolg bei
einer fehlerhaften Payload würde den Mirror veraltet lassen, ohne
dass Betreiber es sehen.

Positionen, die auf Provider-Seite aus einem Abonnement entfernt
wurden (z. B. hat der Nutzer ein Sitzplatz-Add-on abgewählt), werden
aus `payments_subscription_items` entfernt, sobald der nächste
Webhook `subscription.updated` eintrifft. Die Antwort von
`Subscription::get(id)` des Providers ist bei jeder Synchronisierung
die maßgebliche Quelle.

## Zahlungsmethoden jenseits von Karten

`PaymentMethod` ist das Enum, das das Framework für gespeicherte
Methoden in `payments_payment_methods` und für jeden Provider
verwendet, der Methoden-Metadaten liefert. Es deckt die
naheliegenden Fälle ab - Karten, Banküberweisungen, E-Wallets - plus
regionale Methoden, die in vielen Märkten erstklassig sind:

```rust,ignore
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PaymentMethod {
    Card { brand: String, last4: String, exp_month: u8, exp_year: u16 },
    BankTransfer { bank_name: String, last4: String },
    EWallet { provider: String, identifier: String },
    /// Zahler identifiziert über Telefon + Operator + Land.
    MobileMoney {
        operator: MobileMoneyOperator,
        phone: PhoneNumber,
        country: CountryCode,
    },
    /// Wertgebundene Kryptowährung - für die meisten Provider bargeldgleich.
    Stablecoin { asset: StablecoinAsset, network: Option<String> },
    /// Nicht wertgebundene Kryptowährung.
    Crypto { network: String, address: String },
    /// Notausgang für regionale / providerspezifische Methoden, die noch
    /// nicht modelliert sind.
    Custom { kind: String, descriptor: String },
}

#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MobileMoneyOperator {
    MtnMomo,
    Mpesa,
    AirtelMoney,
    OrangeMoney,
    Lipila,
    Custom { identifier: String },
}

#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StablecoinAsset {
    Usdc,
    Usdt,
    Dai,
    Custom { ticker: String },
}
```

Die benannten Operatoren und Assets sind die, die wir bereits
aufgezählt haben. Die Varianten `Custom { ... }` an jedem decken
regionale Operatoren und Stablecoins ab, die wir noch nicht
festgelegt haben, sodass die Unterstützung für einen weiteren keinen
Framework-Release erzwingt.

`PhoneNumber` und `CountryCode` sind validierte DTOs in
`suprnova::payments` - sie weisen fehlerhafte Eingaben schon bei der
Konstruktion zurück, und genau dort wollen Sie den Fehlschlag haben,
nicht erst beim Provider-Aufruf.

## Money

Beträge werden als `Money` dargestellt - eine `i64`-Zählung von
Untereinheiten plus eine `Currency`. Kein `f64` ist beteiligt.

```rust,ignore
use suprnova::payments::{Money, Currency};
use rust_decimal::Decimal;
use std::str::FromStr;

// Aus Untereinheiten (Cent, Pence, Yen usw.)
let price = Money::from_minor_units(1999, Currency::USD);  // $19.99

// Aus einer Dezimal-Zeichenkette
let price = Money::from_decimal(Decimal::from_str("19.99").unwrap(), Currency::USD);

// Nulldezimal-Währungen - 1234 minor = 1234 JPY (keine Umrechnung)
let yen = Money::from_minor_units(1234, Currency::JPY);

// Arithmetik - Panic bei Währungs-Mismatch
let total = price + Money::from_minor_units(100, Currency::USD);  // $20.99

// Negative Werte stehen für Rückerstattungen oder Gutschriften
let refund = Money::from_minor_units(-500, Currency::USD);  // -$5.00

// Zurücklesen
println!("{} minor units in {:?}", price.minor_units(), price.currency());
```

`Add` und `Sub` lösen bei Währungs-Mismatch und bei
`i64`-Überlauf einen Panic aus. Nutzen Sie die panische Arithmetik
für Korrektheit - eine stille Addition über Währungsgrenzen ist ein
Bug, kein Feature.

## ChargeResult

`Payment::charge` liefert ein Enum `ChargeResult`. Nicht jede
Belastung schließt sofort ab - 3DS-Step-up und Off-Session-Karten
können einen Redirect oder eine clientseitige Aktion erfordern:

```rust,ignore
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChargeResult {
    Completed {
        provider_transaction_id: String,
        amount: Money,
        status: PaymentStatus,
        provider_metadata: Value,
    },
    RedirectRequired {
        provider_transaction_id: String,
        url: String,
        return_to: Option<String>,
    },
    RequiresClientAction {
        provider_transaction_id: String,
        action_kind: String,
        client_secret: Option<String>,
        publishable_key: Option<String>,
    },
}
```

Behandeln Sie `RequiresClientAction`, indem Sie die Payload an Ihr
Frontend zurückgeben. Das Frontend rendert die 3DS-Challenge mit
`client_secret` + `publishable_key`. Siehe
[Zahlungen - Frontend Integration](payments-frontend.md) für den
Dispatch-Code im Frontend.

## Idempotenzschlüssel

Jedes mutierende DTO hat ein optionales
`idempotency_key: Option<String>`. Setzen Sie einen bei
wiederholbaren Netzwerkaufrufen:

```rust,ignore
provider.start_session(StartSessionRequest {
    // ...
    idempotency_key: Some(format!("checkout_{}_{}", user_id, order_id)),
    // ...
}).await?;

provider.subscribe(SubscribeRequest {
    // ...
    idempotency_key: Some(format!("sub_{}_{}", user_id, plan_id)),
    // ...
}).await?;
```

Stripe honoriert Idempotenzschlüssel über den HTTP-Header
`Idempotency-Key`. Paddle hat einen gleichwertigen Mechanismus. Wenn
eine Anfrage mitten im Flug fehlschlägt und Sie mit demselben
Schlüssel erneut versuchen, liefert der Provider die ursprüngliche
Antwort, statt eine doppelte Belastung oder ein doppeltes Abonnement
anzulegen.

## Das Diskriminator-Muster

Jeder Adapter, der behauptet, `PaymentProvider` zu implementieren,
muss denselben E2E-Flow bestehen:

```
create_customer → start_session → subscribe → get → cancel(at_period_end) → cancel(immediate) → assert as_payment invariant
```

Der im Framework enthaltene `MockPaymentProvider` besteht das:

```rust,ignore
use suprnova::payments::*;

#[tokio::test]
async fn discriminator_flow() {
    let provider = MockPaymentProvider::new();

    let cus = provider.create_customer(CreateCustomerRequest {
        user_id: "user_42".into(),
        email: "alice@example.com".into(),
        name: Some("Alice".into()),
        metadata: None,
    }).await.unwrap();

    let session = provider.start_session(StartSessionRequest {
        mode: SessionMode::Subscription,
        customer_ref: cus.provider_customer_id.clone(),
        price_refs: vec!["price_pro_monthly".into()],
        success_return_url: "https://app.example/billing/success".into(),
        cancel_return_url: "https://app.example/billing/cancel".into(),
        amount_hint: None,
        idempotency_key: Some("idem_1".into()),
        metadata: None,
    }).await.unwrap();
    assert!(matches!(session, SessionPayload::Redirect { .. }));

    let sub = provider.subscribe(SubscribeRequest {
        customer_ref: cus.provider_customer_id.clone(),
        price_refs: vec!["price_pro_monthly".into()],
        trial_days: None,
        idempotency_key: Some("idem_2".into()),
        metadata: None,
    }).await.unwrap();
    assert_eq!(sub.status, SubscriptionStatus::Active);

    // Zum Periodenende kündigen
    let s = provider.cancel(&sub.provider_subscription_id, true).await.unwrap();
    assert!(s.cancel_at_period_end);

    // Sofort kündigen
    let s = provider.cancel(&sub.provider_subscription_id, false).await.unwrap();
    assert_eq!(s.status, SubscriptionStatus::Canceled);

    // MockPaymentProvider lässt Payment absichtlich aus (optional, wie bei Paddle)
    let p: &dyn PaymentProvider = &provider;
    assert!(p.as_payment().is_none());
}
```

`MockPaymentProvider` implementiert `Payment` nicht - das übt
dieselbe Invariante wie Paddle aus. `StripeProvider` und
`PaddleProvider` bestehen in Integrationstests beide denselben Flow
gegen die Live-API.

## Apps mit mehreren Providern

Registrieren Sie beide Adapter beim Boot und verzweigen Sie danach,
wo der Datensatz jedes Kunden angelegt wurde:

```rust,ignore
PaymentProviderRegistry::bind("stripe", Arc::new(stripe_provider));
PaymentProviderRegistry::bind("paddle", Arc::new(paddle_provider));

// Später, pro Anfrage:
let provider_name = user.payment_provider.as_str(); // "stripe" oder "paddle"
let provider = PaymentProviderRegistry::get(provider_name).expect("unknown provider");
let sub = provider.cancel(&sub_id, true).await?;
```

Gängige Verwendungen: EU-Kunden über Paddle leiten (für die
MoR-Steuerbehandlung) und US-Kunden über Stripe; Checkout-Konversion
zwischen Providern A/B-testen; einen Provider für Abonnements
verwenden und einen anderen für Einmalzahlungen.

## Migration von Laravel Cashier

Cashier ist per Design Stripe-exklusiv. Suprnova liefert
Multi-Provider-Unterstützung von Haus aus. Kurze Übersicht:

| Laravel Cashier | Suprnova |
|---|---|
| `$user->newSubscription('default', 'price_pro')->create()` | `provider.subscribe(SubscribeRequest { ... }).await` |
| `$user->subscription('default')->cancel()` | `provider.cancel(&sub_id, true).await` |
| `Cashier::webhookHandler` | `webhook_routes(db.clone())` |
| `$user->createAsStripeCustomer()` | `provider.create_customer(CreateCustomerRequest { ... }).await` |
| `$user->charge(1999, 'pm_...')` | `payment.charge(ChargeRequest { ... }).await` (falls der Provider das unterstützt) |
| `$invoice->download()` | Nicht eingebaut; lesen Sie `provider_metadata["invoice_pdf_url"]` aus der Transaktions-Mirror-Tabelle |

## Nächste Schritte

- [Zahlungen - Stripe Adapter](payments-stripe.md) - der Gateway-Flow
  im Detail: PaymentIntents, Webhook-Signaturformat,
  Event-Typ-Zuordnung
- [Zahlungen - Paddle Adapter](payments-paddle.md) - der MoR-Flow im
  Detail: Checkout-getriebene Abonnement-Anlage, Steuerbehandlung,
  Benachrichtigungsverifizierung
- [Zahlungen - Frontend Integration](payments-frontend.md) - Svelte
  5-, React 19- und Vue 3.5-Dispatch-Beispiele
- [Zahlungs-Provider-Adapter schreiben](payments-provider-guide.md) -
  bauen Sie Ihre eigene Adapter-Crate von Anfang bis Ende
- [Datenbank](database.md) - die SeaORM-Schicht, auf der die
  Mirror-Tabellen sitzen
