# Zahlungen - Stripe Adapter

`suprnova-payments-stripe` ist der Referenz-Adapter für Suprnovas
Provider-neutrale Zahlungs-Oberfläche. Er implementiert alle fünf
Zahlungs-Traits (`Checkout`, `Payment`, `Subscription`,
`CustomerStore`, `WebhookHandler`) gegen die Stripe-API über
`async-stripe` 1.0.0-rc.5. Greifen Sie zu diesem Kapitel, wenn Sie
genau wissen müssen, welchen Stripe-Endpunkt eine Methode aufruft,
wie das Webhook-Signaturformat verifiziert wird, wie PaymentIntents
durch `ChargeResult` fließen, oder welche Event-Typen auf das
neutrale Event-Enum abgebildet werden.

Für die Trait-Formen selbst, das Einrichten der Umgebungsvariablen
und das Bootstrap-Muster lesen Sie zuerst
[Zahlungen](payments.md). Dieses Kapitel ist der
Stripe-spezifische Deep Dive.

## Gateway, nicht Merchant of Record

Stripe ist standardmäßig ein **Zahlungs-Gateway**: Sie erhalten die
Gelder direkt auf Ihr eigenes Bankkonto, und Sie sind verantwortlich
für Steuererhebung und -abführung, Rechnungsstellung, Dunning und
Chargeback-Abwicklung. Im Gegensatz dazu Paddle
([Zahlungen - Paddle](payments-paddle.md)), wo Paddle der Merchant of
Record ist - sie ziehen die Gelder ein, melden die Steuer und zahlen
Ihnen den Betrag abzüglich Gebühren aus.

Die praktische Konsequenz für dieses Kapitel: `StripeProvider`
implementiert `Payment` (Sie können eine Karte auf dem Server
autorisieren, erfassen, erstatten und stornieren). `PaddleProvider`
tut das nicht. Der Trait-Split existiert, weil die beiden Flows sich
wirklich unterscheiden - nicht, weil uns die Zeit ausgegangen ist.

### Stripe Managed Payments (Merchant of Record zum Opt-in)

Stripes Programm **Managed Payments** verschiebt Stripe für
berechtigte Transaktionen in die Rolle des Merchant of Record -
Stripe wird zum rechtlichen Verkäufer, berechnet, erhebt, meldet und
führt Umsatzsteuer/MwSt./GST ab und übernimmt Streitfälle. Das
Programm hat feste Integrationsvorgaben:

- **Nur gehosteter Checkout.** Sessions müssen auf Stripes gehosteter
  Seite laufen. Elements-/Custom-Flows sind ausgeschlossen - weshalb
  der gehostete Einmalzahlungs-Pfad des Adapters (unten) die einzige
  `OneOff`-Form ist, die sich damit kombinieren lässt.
- **Vordefinierte Preise mit berechtigten Steuercodes.** Positionen
  müssen auf `price_…`-Objekte verweisen, deren Produkte im
  Stripe-Dashboard einen als Managed-Payments-berechtigt markierten
  Steuercode tragen. Ad-hoc-Beträge werden abgelehnt.
- **Konto-Anmeldung.** Das Stripe-Konto muss für das Programm
  onboardet sein; Sessions mit dem Flag auf einem nicht angemeldeten
  Konto schlagen fehl.

Aktivieren Sie es pro Provider mit `.with_managed_payments(true)`
oder `STRIPE_MANAGED_PAYMENTS=true` - der Adapter sendet dann
`managed_payments[enabled]=true` beim Anlegen gehosteter
Einmalzahlungs-Sessions. Wenn es aus ist (der Standard), wird das
Feld komplett ausgelassen.

### Warum Suprnova abweicht

Laravel liefert Cashier als hauseigene Stripe-Integration in den
Kern-Docs. Das ist praktisch, aber Stripe-exklusiv - und ein zweiter
Provider bedeutet, Cashier zu forken oder eine parallele Oberfläche
zu bauen.

Suprnova hält Stripe auf Abstand. Der Stripe-Adapter ist eine Crate,
die sich gegen dieselben fünf Traits registriert, die jeder andere
Provider implementiert. Ihr Domain-Code benennt niemals
`StripeProvider`; er ruft `provider.charge(...)` gegen ein aus der
Registry aufgelöstes `Arc<dyn PaymentProvider>` auf, und das
Stripe-Verhalten ist nur einen Austausch vom Paddle-Verhalten
entfernt. Wenn Sie später Mollie hinzufügen oder ein regionales
Gateway anbinden, das noch nicht existiert, implementieren Sie
dieselben fünf Traits, und der Rest Ihrer App bewegt sich nicht.

## Konstruktion

```rust
use suprnova_payments_stripe::StripeProvider;
use std::sync::Arc;
use suprnova::payments::PaymentProviderRegistry;

// Produktion: aus der Umgebung lesen.
let stripe = StripeProvider::from_env()
    .expect("STRIPE_SECRET_KEY / PUBLISHABLE_KEY / WEBHOOK_SIGNING_SECRET");

// Tests / explizite Konfiguration:
let stripe = StripeProvider::new(
    "sk_test_...",
    "pk_test_...",
    "whsec_...",
);

PaymentProviderRegistry::bind("stripe", Arc::new(stripe));
```

`StripeProvider` ist `Clone` (günstig - der zugrunde liegende
`stripe::Client` ist `Arc`-gestützt) und hält diese Werte:

| Feld | Quelle | Verwendung |
|---|---|---|
| `secret_key` | `sk_live_…` / `sk_test_…` | HTTP `Authorization: Bearer …` bei jedem API-Aufruf |
| `publishable_key` | `pk_live_…` / `pk_test_…` | Sichtbar gemacht in `SessionPayload::StripeElements`, damit das Frontend Stripe.js ohne separaten Config-Lookup einbinden kann |
| `webhook_signing_secret` | `whsec_…` | HMAC-SHA256-Verifizierung des Headers `Stripe-Signature` |
| `managed_payments` | `STRIPE_MANAGED_PAYMENTS` (`true`/`1`) oder `.with_managed_payments(bool)` | Sendet `managed_payments[enabled]=true` beim Anlegen einer gehosteten Einmalzahlungs-Session (siehe [Managed Payments](#stripe-managed-payments-merchant-of-record-zum-opt-in)) |

`from_env()` liefert `Result<Self, String>` - die Fehlermeldung
nennt die fehlende Pflichtvariable (`STRIPE_MANAGED_PAYMENTS` ist
optional; fehlt sie, ist es aus). Es gibt keinen Panic-Pfad beim
Boot.

## Checkout-Sessions

`Checkout::start_session` wählt seine Stripe-Oberfläche anhand der
Anfrage:

| Form der Anfrage | Stripe-Objekt | `SessionPayload`-Variante |
|---|---|---|
| `OneOff` + nicht leere `price_refs` | Gehostete Checkout-Session, `mode=payment` | `StripeCheckoutRedirect { url, provider_session_id: "cs_…" }` |
| `OneOff` + leere `price_refs` + `amount_hint` | PaymentIntent | `StripeElements { client_secret, publishable_key, provider_session_id: "pi_…" }` |
| `Subscription` + `price_refs` | Gehostete Checkout-Session, `mode=subscription` | `StripeCheckoutRedirect` |

Der gehostete Einmalzahlungs-Pfad sendet
`allow_promotion_codes=true` (Kunden können auf Stripes Seite
Promotion-Codes eingeben - kombinieren Sie das mit dem Trait
`Promotions` unten) und, wenn der Provider dafür konfiguriert ist,
das Managed-Payments-Flag. Setzen Sie Stripes Template-Literal
`{CHECKOUT_SESSION_ID}` in Ihre `success_return_url` - Stripe
ersetzt beim Redirect die echte `cs_…`-ID, und Ihre Rückkehrseite
gibt sie an `session_status` weiter.

`Checkout::session_status` bildet
`GET /v1/checkout/sessions/{id}` auf den neutralen
`CheckoutSessionState` ab:

| Stripe `status` / `payment_status` | `CheckoutSessionState` |
|---|---|
| `open` | `Open` |
| `expired` | `Expired` |
| `complete` + `paid` oder `no_payment_required` | `Complete { paid: true, payment_ref, amount_total }` |
| `complete` + `unpaid` (verzögertes Settlement) | `Complete { paid: false, … }` |

`payment_ref` trägt die PaymentIntent-ID der Session (`pi_…`), damit
Rückkehrseiten und Abgleichsläufe die Session mit
`Payment`-Operationen und dem Mirror `payments_transactions`
korrelieren können. `amount_total` ist die abgerechnete
Gesamtsumme, in die providerseitige Rabatte und
Managed-Payments-Steuer bereits eingerechnet sind.

## Promotion-Codes

`StripeProvider` implementiert den optionalen Trait `Promotions`
(`provider.as_promotions()` liefert `Some`).
`create_promotion_code` bildet auf `POST /v1/promotion_codes` ab: Es
prägt einen Code aus einem vorab angelegten Coupon (`coupon_ref`),
beschränkt auf einen Kunden (`customer_ref`), mit optionalem Ablauf
und Einlöse-Obergrenze. Beschränkungen werden von Stripe bei der
Einlösung durchgesetzt - ein für Kunde A geprägter Code wird
abgelehnt, wenn Kunde B ihn eintippt, abgelaufene Codes werden
abgelehnt, und `max_redemptions: Some(1)` macht den Code einmalig
nutzbar. Siehe den Abschnitt `Promotions` von
[Zahlungen](payments.md) für das Kampagnenmuster.

## Der PaymentIntent-Lebenszyklus

Stripe stellt einen einzelnen Belastungsversuch als **PaymentIntent**
dar. Der Intent bewegt sich durch Statuswerte; der Suprnova-Trait
`Payment` treibt die Übergänge. Jede `Payment`-Methode von
`StripeProvider` bildet auf einen `/v1/payment_intents/...`-Endpunkt
ab:

| `Payment`-Methode | Stripe-Endpunkt | Was sie tut |
|---|---|---|
| `charge` | `POST /v1/payment_intents` | Erzeugt + bestätigt in einem Aufruf gegen eine gespeicherte Zahlungsmethode. `capture_method: "manual"`, sodass der Intent zu `requires_capture` wechselt, **nicht** zu `succeeded`. |
| `capture` | `POST /v1/payment_intents/{id}/capture` | Rechnet einen zuvor autorisierten Intent ab. Status `requires_capture` → `succeeded`. |
| `refund` | `POST /v1/refunds` | Kehrt einen erfassten Intent ganz oder teilweise um. |
| `void` | `POST /v1/payment_intents/{id}/cancel` | Gibt eine Autorisierung vor der Erfassung wieder frei. Status `requires_capture` → `canceled`. |
| `status` | `GET /v1/payment_intents/{id}` | Ruft den aktuellen Status ab (liefert `PaymentStatus`). |

### Erst autorisieren, dann erfassen

`StripeProvider::charge` rechnet die Gelder **nicht** sofort ab. Es
sendet `capture_method=manual` + `confirm=true`, was die Karte
autorisiert und die Gelder reserviert, und wartet dann auf einen
expliziten Aufruf von `capture`. Das ist der kanonische
Zwei-Schritt-Flow:

```rust
use suprnova::payments::{
    PaymentProviderRegistry, ChargeRequest, ChargeResult,
    Money, Currency, PaymentStatus,
};

let provider = PaymentProviderRegistry::get("stripe").unwrap();
let payment = provider.as_payment()
    .expect("Stripe implements Payment");

let result = payment.charge(ChargeRequest {
    customer_ref: "cus_NffrFeUfNV2Hib".into(),
    payment_method_ref: "pm_card_visa".into(),
    amount: Money::from_minor_units(2999, Currency::USD),
    description: Some("Pro plan, manual capture".into()),
    idempotency_key: Some("order-12345".into()),  // siehe "Idempotenz" unten
    metadata: None,
}).await?;

match result {
    ChargeResult::Completed { provider_transaction_id, status, .. }
        if status == PaymentStatus::Pending => {
        // Autorisiert - abrechnen, wenn die Bestellung versandt wird.
        let settled = payment.capture(&provider_transaction_id).await?;
        assert!(matches!(
            settled,
            ChargeResult::Completed { status: PaymentStatus::Succeeded, .. }
        ));
    }
    ChargeResult::RequiresClientAction { client_secret, .. } => {
        // 3DS-Step-up nötig - siehe "3DS und SCA" unten.
    }
    other => panic!("unexpected charge result: {other:?}"),
}
```

Wenn Sie **sofortige** Erfassung wollen - der übliche
E-Commerce-Einmalfall -, verwenden Sie stattdessen
`Checkout::start_session` mit `SessionMode::OneOff`. Dieser Pfad
legt einen PaymentIntent mit aktivierten
`automatic_payment_methods` an und übergibt das Client-Secret dem
Frontend, sodass der Browser des Kunden den Intent direkt vor Ort
bestätigt. `Payment::charge` ist für serverseitig getriebene Flows,
in denen Sie bereits die gespeicherte Zahlungsmethode des Kunden
halten und explizite Kontrolle über Autorisieren-dann-Erfassen
wollen (typisch für Marktplätze, SaaS mit verzögerter Erfüllung
oder Split-Shipment-Commerce).

### Status-Zuordnung

Stripe-Status falten sich in Suprnovas Enum `PaymentStatus`:

| `PaymentIntentStatus` | `PaymentStatus` |
|---|---|
| `Succeeded` | `Succeeded` |
| `Processing` | `Pending` |
| `RequiresCapture` | `Pending` (autorisiert, wartet auf Erfassung) |
| `RequiresAction` | `Pending` (von `charge` als `RequiresClientAction` zurückgegeben) |
| `RequiresConfirmation` | `Pending` |
| `RequiresPaymentMethod` | `Pending` |
| `Canceled` | `Canceled` |
| _neuer Stripe-Status (Enum ist `#[non_exhaustive]`)_ | `Failed` |

Der Fallback auf `non_exhaustive` ist beabsichtigt. Stripe fügt
gelegentlich Zustände hinzu (z. B. bei der Einführung neuer
Zahlungsmethoden-Typen). Sie als `Failed` sichtbar zu machen, ist
der konservative Standard - Ihre App behandelt die Belastung als
noch-nicht-bestätigt, bis Sie den Adapter aktualisieren.

### 3DS und SCA

Europas Strong Customer Authentication, Indiens RBI-Regeln und
mehrere andere Regulierungen verlangen, dass der Karteninhaber die
Belastung in einem separaten Browser-Kontext authentifiziert. Stripe
macht das als `requires_action` mit einem `next_action`-Block
sichtbar.

`StripeProvider::charge` übersetzt das in eine von zwei
`ChargeResult`-Varianten:

```rust
ChargeResult::RequiresClientAction {
    provider_transaction_id,   // pi_xxx - diese aufheben
    action_kind: "stripe_3ds", // Stripe-spezifischer Tag
    client_secret,             // an Stripe.js übergeben
    publishable_key,           // an Stripe.js übergeben
}
```

Wenn der `next_action` des Intents eine Redirect-URL enthält (manche
Authentifizierungs-Flows sind URL-Redirect statt In-Place-Modal),
wird das Ergebnis umgeschrieben zu:

```rust
ChargeResult::RedirectRequired {
    provider_transaction_id,
    url,                       // Browser hierhin umleiten
    return_to: None,
}
```

Ihr Controller übergibt die `RequiresClientAction`-Payload an die
Inertia-Seite; das Frontend ruft
`stripe.confirmCardPayment(client_secret, ...)` auf, und der Kunde
schließt 3DS ab. Wenn die Bestätigung gelingt, feuert Stripe
`payment_intent.succeeded`, und die Webhook-Route schreibt die
Mirror-Zeile. Siehe
[Zahlungen - Frontend Integration](payments-frontend.md) für die
Svelte-/React-/Vue-Schnipsel.

### Storno vs. Rückerstattung

`void` gibt eine Autorisierung **vor** der Erfassung frei; `refund`
kehrt eine erfasste Zahlung um. Der Aufruf von `void` auf einem
erfassten Intent schlägt fehl - Stripe lehnt mit einer Meldung ab,
die `"already succeeded"` oder `"You cannot cancel"` enthält, und
der Adapter macht das als `PaymentError::Validation` sichtbar,
sodass Ihr Handler einen behebbaren Nutzerfehler (verwenden Sie
stattdessen `refund`) von einem echten Provider-Ausfall
unterscheiden kann. Jeder andere Fehlschlag ist
`PaymentError::Provider`.

```rust
let voided = payment.void("pi_3PNzj...").await;
match voided {
    Ok(()) => { /* Autorisierung freigegeben */ }
    Err(suprnova::payments::PaymentError::Validation(msg)) => {
        // Bereits erfasst - stattdessen refund aufrufen.
        let refund = payment.refund(RefundRequest {
            provider_transaction_id: "pi_3PNzj...".into(),
            amount: None,           // vollständige Rückerstattung
            reason: Some("requested_by_customer".into()),
            idempotency_key: None,  // refund() leitet das nicht weiter - siehe "Idempotenz"
        }).await?;
    }
    Err(e) => return Err(e.into()),
}
```

## Kunden

`StripeProvider` implementiert `CustomerStore` gegen
`/v1/customers`. Der Adapter bildet einen zurückgegebenen `Customer`
auf die neutrale `CustomerRef` ab und bewahrt dabei die E-Mail und
die `user_id` Ihrer Anwendung:

```rust
use suprnova::payments::CreateCustomerRequest;

let customer = provider.create_customer(CreateCustomerRequest {
    user_id: "user-42".into(),       // die User-ID Ihrer App
    email: "alice@example.com".into(),
    name: Some("Alice Example".into()),
    metadata: None,
}).await?;

// customer.provider_customer_id == "cus_NffrFeUfNV2Hib"
// Diese neben Ihrer User-Zeile persistieren, damit nachfolgende
// Belastungen, Abonnements und Webhooks sich zurück auflösen.
```

`update_customer`, `get_customer` und `delete_customer` treffen
jeweils `POST /v1/customers/{id}`, `GET /v1/customers/{id}` und
`DELETE /v1/customers/{id}`. Stripes Delete liefert einen
`DeletedCustomer`-Envelope, den der Adapter verwirft - nur Erfolg
oder Fehlschlag des Aufrufs werden weitergegeben.

## Abonnements

`StripeProvider::subscribe` postet an `/v1/subscriptions` mit dem
Kunden-Ref, einem `items[]`-Array und einem optionalen
`trial_period_days`:

```rust
use suprnova::payments::{SubscribeRequest, SubscriptionStatus};

let sub = provider.subscribe(SubscribeRequest {
    customer_ref: "cus_NffrFeUfNV2Hib".into(),
    price_refs: vec!["price_pro_monthly".into()],
    trial_days: Some(14),
    idempotency_key: None,
    metadata: None,
}).await?;

assert!(matches!(
    sub.status,
    SubscriptionStatus::Trialing | SubscriptionStatus::Active
));

println!("Period ends at {}", sub.current_period_end);
for item in &sub.items {
    println!(
        "  {} × {} @ {:?}",
        item.quantity, item.provider_price_id, item.unit_amount,
    );
}
```

### Periodengrenzen

Stripe hat die Zeitstempel `current_period_start` /
`current_period_end` in der API-Version `2023-08-16` vom
übergeordneten Subscription auf jedes `SubscriptionItem` verschoben.
Mehrpositions-Abonnements können theoretisch abweichende
Item-Perioden haben, aber in der Praxis teilt sich jede Position
eines einzelnen Abonnements den Abrechnungszyklus des übergeordneten
Objekts. Der Adapter nimmt die Periode der **ersten Position** als
übergeordnete Periode im zurückgegebenen `SubscriptionResult`. Wenn
Sie wirklich Perioden pro Position brauchen, lesen Sie sie aus
`sub.items[n]` - sie sind auf dem Snapshot erhalten.

### Zum Periodenende kündigen vs. sofort

```rust
// Sanfte Kündigung - Zugriff bis current_period_end behalten:
let sub = provider.cancel("sub_1234", /* at_period_end */ true).await?;
// sub.cancel_at_period_end == true
// sub.status == Active

// Sofortige Kündigung - Stripe DELETE /v1/subscriptions/{id}:
let sub = provider.cancel("sub_1234", /* at_period_end */ false).await?;
// sub.status == Canceled
```

Die beiden Pfade treffen unterschiedliche Stripe-Endpunkte. Die
sanfte Kündigung ist `POST /v1/subscriptions/{id}` mit
`cancel_at_period_end=true` - das Abonnement bleibt bis zum Ende des
Abrechnungszeitraums aktiv, dann finalisiert Stripe es. Die sofortige
Kündigung ist `DELETE /v1/subscriptions/{id}` mit `prorate=false`
und `invoice_now=false`.

### `update()` ist absichtlich eingeschränkt

`UpdateSubscriptionRequest` hat zwei Felder, auf die der Adapter
reagiert: `cancel_at_period_end` und `new_price_refs`. Das erste wird
unterstützt; das zweite liefert `PaymentError::NotSupported`:

```rust
provider.update(UpdateSubscriptionRequest {
    provider_subscription_id: "sub_1234".into(),
    new_price_refs: Some(vec!["price_team_yearly".into()]),
    cancel_at_period_end: None,
    idempotency_key: None,
}).await
// → Err(PaymentError::NotSupported(
//      "Stripe price-set replacement on existing subscription not in v1. \
//       Cancel the subscription and create a new one with the new price set."
//   ))
```

Das ist einer der wenigen Orte, an denen `NotSupported` die
ehrliche Antwort ist, statt eine Vertagung. Der Ersatz eines
Preis-Sets bei Stripe erfordert, Abonnement-Positionen zu löschen
und neu anzulegen - die Form variiert je Provider (anteilige
Abrechnung, Verankerung des Abrechnungszyklus, Verhalten bei
laufender Testphase), und das in eine einzige neutrale API zu
zwingen würde mehr verschleiern, als es hülfe. Der empfohlene Weg
ist, das bestehende Abonnement zu kündigen und mit dem neuen
Preis-Set erneut `subscribe` aufzurufen, wobei Sie Ihre eigene
Richtlinie für die anteilige Abrechnung anwenden, falls Sie eine
brauchen.

## Webhooks

Stripe sendet Webhooks, signiert mit HMAC-SHA256, im Format:

```
Stripe-Signature: t=1717000000,v1=5257a869e7ecebeda32affa62cdca3fa51cad7e77a0e56ff536d0ce8e108d8bd
```

`StripeProvider::verify` parst den Header, berechnet HMAC-SHA256
über `"{timestamp}.{raw_body}"` mit dem Webhook-Signaturgeheimnis
neu und führt einen **zeitkonstanten** Vergleich gegen jeden
`v1=`-Wert im Header durch. Während der Rotation des
Signaturgeheimnisses existieren mehrere `v1=`-Werte - Stripe
überlappt das alte und das neue Geheimnis für ein Zeitfenster,
sodass Sie neu signieren und deployen können, ohne einen
Flag-Day-Umstieg zu brauchen.

```
Stripe-Signature: t=1717000000,v1=<old_sig>,v1=<new_sig>
```

Der Adapter akzeptiert die Anfrage, wenn **irgendein** `v1=`-Wert
passt. Ein Header, dem `t=` fehlt oder der keine `v1=`-Werte hat,
wird als `PaymentError::WebhookSignature` abgelehnt. Non-ASCII-Bytes
irgendwo im Header werden ebenfalls abgelehnt - Stripe sendet sie
nie, und sie als ungültig zu behandeln ist sicherer, als ein
Ersatzzeichen einzusetzen.

Sie rufen `verify` nie direkt auf. Das `webhook_routes(db.clone())`
des Frameworks registriert `POST /webhooks/payments/{provider}` und
ruft für jede dort eintreffende Anfrage die `verify` +
`parse_event` + Payload-Extraktoren des Adapters auf. Siehe
[Idempotenz](idempotency.md) für das retry-bewusste
Audit-Verhalten - einschließlich der Regel, dass zuvor
fehlgeschlagene Ereignisse die Hydration erneut versuchen, wenn der
Provider wiederholt.

### Event-→-neutral-Zuordnung

Stripe-Event-Typen werden über die Funktion
`stripe_event_to_neutral` auf Suprnovas `NeutralEventKind`
abgebildet. Die Zuordnungstabelle:

| Stripe-Event-Typ | `NeutralEventKind` |
|---|---|
| `payment_intent.succeeded` | `PaymentSucceeded` |
| `payment_intent.payment_failed` | `PaymentFailed` |
| `charge.refunded` | `PaymentRefunded` |
| `charge.dispute.created` | `PaymentDisputed` |
| `customer.subscription.created` | `SubscriptionCreated` |
| `customer.subscription.updated` | `SubscriptionUpdated` |
| `customer.subscription.deleted` | `SubscriptionCanceled` |
| `customer.subscription.paused` | `SubscriptionUpdated` |
| `customer.subscription.resumed` | `SubscriptionUpdated` |
| `customer.subscription.trial_will_end` | `SubscriptionUpdated` |
| `invoice.payment_succeeded` / `invoice.paid` | `InvoicePaid` |
| `invoice.payment_failed` | `InvoiceFailed` |
| `customer.created` | `CustomerCreated` |
| `customer.updated` | `CustomerUpdated` |
| _alles andere_ | `None` |

Ereignisse, die auf `None` abbilden (Radar-Betrugssignale,
Auszahlungen, Bilanzbuchungen, Streitfall-Lebenszyklus-Ereignisse
nach `created`) werden trotzdem in der Audit-Tabelle
`payments_webhook_events` persistiert - sie treiben nur nicht die
Mirror-Tabellen an. Wenn Sie sie brauchen, lesen Sie direkt aus
`event.raw_payload` in einem eigenen Handler.

Die Zuordnung wird außerdem an der Crate-Wurzel re-exportiert, sodass
Sie sie auch außerhalb der Webhook-Route verwenden können:

```rust
use suprnova_payments_stripe::stripe_event_to_neutral;
use suprnova::payments::NeutralEventKind;

assert_eq!(
    stripe_event_to_neutral("payment_intent.succeeded"),
    Some(NeutralEventKind::PaymentSucceeded),
);
assert_eq!(
    stripe_event_to_neutral("radar.early_fraud_warning.created"),
    None,
);
```

### Payload-Extraktion

Nachdem `verify` und `parse_event` erfolgreich waren, ruft das
Framework `extract_payload_ids`, `extract_payment_snapshot` und
`extract_customer_snapshot` auf, um die Felder zu ziehen, die die
Mirror-Tabellen treiben (siehe [Eloquent](eloquent.md) für das
zugrunde liegende Muster, aus der eigenen Datenbank zu lesen).
Stripe ist strukturell konsistent: jeder Webhook legt die relevante
Entität unter `data.object` ab, mit `id` als Primärschlüssel.

Die Extraktoren behandeln vier Event-Familien:

- **Abonnement-Ereignisse** - ziehen `data.object.id` (die
  Abonnement-ID) und `data.object.customer`.
- **Kunden-Ereignisse** - ziehen `data.object.id` (die Kunden-ID).
- **PaymentIntent-/Charge-Ereignisse** - ziehen `data.object.id`,
  `data.object.amount`, `data.object.currency`,
  `data.object.customer` und (nur bei `payment_intent.succeeded`)
  `data.object.created` als `paid_at`.
- **Invoice-Ereignisse** - ziehen `data.object.id`, den
  Kunden-Zeiger, `data.object.subscription` (nur wiederkehrende
  Belastungen), `amount_paid` (fällt zurück auf `amount_due`),
  `tax`, `currency` und `data.object.status_transitions.paid_at`.

Alles andere liefert `None` aus den Snapshot-Extraktoren; die
Audit-Zeile landet trotzdem.

## Mirror-Tabellen

Sechs Tabellen stützen die Zahlungs-Oberfläche in der Datenbank
Ihrer Anwendung. Wenden Sie die Framework-Migration neben Ihrer
eigenen an:

```rust
use sea_orm_migration::{MigrationTrait, MigratorTrait};
use suprnova::payments::migrations::CreatePaymentsTables;

pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            // ... Ihre Migrationen ...
            Box::new(CreatePaymentsTables),
        ]
    }
}
```

Die angelegten Tabellen sind `payments_customers`,
`payments_payment_methods`, `payments_subscriptions`,
`payments_subscription_items`, `payments_transactions` und
`payments_webhook_events`. Die Webhook-Route hydriert sie pro
Ereignis innerhalb einer einzigen DB-Transaktion - ein partieller
Zustand ist nie beobachtbar, und die Audit-Zeile trägt
`process_error` über Wiederholungen hinweg, sodass Fehlschläge für
Betreiber sichtbar bleiben.

## Idempotenz

Outbound-Idempotenz bei Stripe-API-Aufrufen und Inbound-Idempotenz
bei Webhook-Zustellungen sind zwei getrennte Geschichten. Lesen Sie
sie auch so.

### Outbound: Abdeckung pro Methode

Stripe unterstützt Anfrage-Idempotenz über den HTTP-Anfrage-Header
`Idempotency-Key` - derselbe Schlüssel mit demselben Body liefert
für ein 24-Stunden-Replay-Fenster dasselbe Antwortobjekt; ein
abweichender Body liefert einen Fehler. Der Suprnova-Stripe-Adapter
leitet das Feld `idempotency_key` des DTOs heute **nicht**
einheitlich an diesen Header weiter. Das tatsächliche Verhalten zum
Zeitpunkt dieses Schreibens:

| Methode | DTO-Feld | Was der Adapter tut |
|---|---|---|
| `Payment::charge` | `ChargeRequest::idempotency_key` | In den POST-Body als `idempotency_key=...` weitergeleitet (nicht in den HTTP-Header). Stripes API liest **keine** Idempotenzschlüssel im Body-Formular, sodass das am besten als bislang wirkungslos gilt, bis der Adapter auf den Request-Header-Pfad umzieht. |
| `Payment::refund` | `RefundRequest::idempotency_key` | Wird stillschweigend verworfen - das Feld wird nicht weitergeleitet. |
| `Checkout::start_session` | `StartSessionRequest::idempotency_key` | Wird stillschweigend verworfen. |
| `Subscription::subscribe` / `update` | `*Request::idempotency_key` | Wird stillschweigend verworfen. |

Wenn Sie sich heute für Belastungs-/Rückerstattungs-Wiederholungen
gegen Stripe auf At-most-once-Semantik verlassen, sichern Sie die
Wiederholung an Ihrer eigenen Aufrufstelle ab (ein deterministischer
Domain-Schlüssel, in Ihrer DB persistiert, mit einem Unique-Index,
der den zweiten Insert verhindert), bis der Adapter den Header
verdrahtet. Die DTO-Felder werden von der API akzeptiert, aber
derzeit nicht bis in die tatsächlich gesendete Anfrage
durchgereicht - setzen Sie sie in Tests und Produktionscode auf
`None`, damit die Lücke explizit ist, und gehen Sie nicht davon aus,
dass Stripe Ihre Wiederholungen dedupliziert.

Das ist eine bekannte Lücke im v1-Adapter und ein Kandidat für einen
Fix im nächsten Release; die Form der Oberfläche bleibt gleich,
sobald die Verdrahtung steht.

### Inbound: Webhook-Deduplizierung

Webhook-Idempotenz wird vom Framework auf der Ingress-Seite
gehandhabt und ist vollständig verdrahtet. Jedes Ereignis landet in
`payments_webhook_events` mit einem Unique-Index auf
`(provider, provider_event_id)`. Doppelte Zustellungen eines bereits
verarbeiteten Ereignisses liefern sofort 200 an Stripe, ohne die
Hydration erneut laufen zu lassen; Duplikate eines zuvor
**fehlgeschlagenen** Ereignisses versuchen die Hydration erneut,
sodass die Wiederholung des Providers Ihr
Wiederherstellungsmechanismus ist. Siehe [Idempotenz](idempotency.md)
für den vollständigen Audit- + Retry-Vertrag.

## Testen

Der Adapter setzt auf hyper und terminiert TLS über rustls. Tests, die einen
`StripeProvider` konstruieren, brauchen einen registrierten
Crypto-Provider; wir installieren `ring` genau einmal in
`#[cfg(test)]`:

```rust
#[cfg(test)]
mod tests {
    use suprnova_payments_stripe::StripeProvider;
    use std::sync::OnceLock;

    fn install_crypto_provider() {
        static ONCE: OnceLock<()> = OnceLock::new();
        ONCE.get_or_init(|| {
            let _ = rustls::crypto::ring::default_provider().install_default();
        });
    }

    fn provider() -> StripeProvider {
        install_crypto_provider();
        StripeProvider::new("sk_test_dummy", "pk_test_dummy", "whsec_dummy")
    }

    #[test]
    fn parses_subscription_webhook_ids() {
        let p = provider();
        let event = /* construct WebhookEvent with raw_payload */;
        let ids = p.extract_payload_ids(&event);
        assert_eq!(ids.subscription_id.as_deref(), Some("sub_abc"));
    }
}
```

Für Integrationstests gegen die Live-Stripe-Sandbox setzen Sie
`STRIPE_SECRET_KEY` und Verwandte in Ihrer Test-Umgebung. Für
Unit-Tests Ihrer eigenen Controller bevorzugen Sie
`MockPaymentProvider` aus dem Framework - er implementiert alle fünf
Traits mit vorhersehbaren Rückgaben und ohne Netzwerk.

## Nächste Schritte

- [Zahlungen](payments.md) - die Trait-Oberfläche, die Registry, das
  Bootstrap-Muster und die per Flow getaggte `SessionPayload`.
- [Zahlungen - Paddle](payments-paddle.md) - das Merchant-of-Record-
  Gegenstück; dieselben fünf Traits, anderer Zuständigkeits-Split.
- [Zahlungen - Provider-Leitfaden](payments-provider-guide.md) - wie
  Sie einen Adapter für ein Gateway schreiben, das Suprnova nicht
  mitliefert.
- [Zahlungen - Frontend Integration](payments-frontend.md) - Svelte-/
  React-/Vue-Dispatch auf `SessionPayload.flow`, einschließlich der
  Stripe.js-Confirm-Card-Payment-Schleife.
- [Idempotenz](idempotency.md) - der Audit- + Retry-Vertrag, der
  die Webhook-Verarbeitung unter mindestens-einmaliger Zustellung
  sicher macht.
- [Eloquent](eloquent.md) - fragen Sie die Mirror-Tabellen neben
  Ihren eigenen Modellen ab; alles ist einfach eine SeaORM-Entität.
