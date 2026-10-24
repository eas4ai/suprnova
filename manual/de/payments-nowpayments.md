# Zahlungen - NOWPayments

Der Adapter `suprnova-payments-nowpayments` erstellt gehostete Rechnungen,
verifiziert Zahlungsbenachrichtigungen und liest den Zahlungsstatus über den
Merchant-API-Key. Er registriert sich als `nowpayments` in der normalen
Payment-Provider-Registry.

## Installation und Konfiguration

Verwenden Sie für beide Abhängigkeiten eine Suprnova-Revision, die diesen
Adapter enthält. Für eine lokale Anwendung neben einem Suprnova-Checkout:

```toml
[dependencies]
suprnova = { path = "../suprnova/framework" }
suprnova-payments-nowpayments = { path = "../suprnova/crates/suprnova-payments-nowpayments" }
serde_json = "1"
```

Konfigurieren Sie diese Werte in der geschützten Umgebung der Anwendung:

```dotenv
NOWPAYMENTS_ENVIRONMENT=sandbox
NOWPAYMENTS_API_KEY=your-sandbox-api-key
NOWPAYMENTS_IPN_SECRET=your-sandbox-ipn-secret
NOWPAYMENTS_IPN_CALLBACK_URL=https://app.example/webhooks/payments/nowpayments
```

Die Umgebung akzeptiert `sandbox` oder `production` und verwendet
standardmäßig `sandbox`. Eine unbekannte oder leere Umgebung lässt die
Konfiguration fehlschlagen. Leere Zugangsdaten schlagen vor HTTP-Anfragen
fehl. Verwenden Sie für jede Umgebung eigene Zugangsdaten.

Registrieren Sie den Provider beim Boot und geben Sie einen
Konfigurationsfehler weiter:

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

Fügen Sie die Zahlungs-Migrationen hinzu und binden Sie `webhook_routes(db)`
in den Router der Anwendung ein, wie in der
[Zahlungs-Übersicht](payments.md) gezeigt. Der Endpunkt ist
`POST /webhooks/payments/nowpayments`. Halten Sie diese
Provider-authentifizierte Route außerhalb der CSRF-Middleware für
Browser-Sessions. Konfigurieren Sie den Callback mit der exakten
öffentlichen HTTPS-URL; Request-Bodies müssen den Adapter unverändert
erreichen.

## Eine gehostete Rechnung starten

Persistieren Sie einen Checkout-Versuch mit einer eindeutigen
Händler-Bestellreferenz, bevor Sie die Anfrage stellen. Der Betrag stammt
aus der vertrauenswürdigen Bestellung der Anwendung, niemals aus einer vom
Browser gelieferten Summe.

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

`Checkout::start_session` akzeptiert dieselbe Anfrage und liefert das
generische `SessionPayload::Redirect`. Speichern Sie dessen
`provider_session_id` als **Rechnungs-ID** und leiten Sie dann auf die
validierte URL weiter. Die konkrete Methode `create_invoice` liefert
zusätzlich die Händler-Bestellreferenz und unterscheidet unsichere
Erstellungsfehler.

Der Rechnungs-Adapter verlangt den Einmal-Modus, leere Kunden- und
Preisreferenzen sowie einen positiven `Money`-Betrag mit definiertem
Exponenten der Fiat-Währung. Er sendet den exakten Dezimalbetrag in der
Haupteinheit und den Währungscode in Kleinbuchstaben. Kunden wählen auf der
gehosteten Seite eine Kryptowährung aus, sofern `pay_currency` nicht
angegeben ist.

Die Metadaten sind ein striktes Objekt mit diesen Feldern:

| Feld | Bedeutung |
| --- | --- |
| `order_id` | Erforderliche, nicht leere Händlerreferenz, höchstens 128 Bytes |
| `order_description` | Optionale Beschreibung, höchstens 500 Bytes |
| `pay_currency` | Optionaler Provider-Ticker in Kleinbuchstaben, etwa `btc` |
| `is_fixed_rate` | Optionale Wechselkurs-Einstellung des Providers |
| `is_fee_paid_by_user` | Optionale Gebühren-Einstellung des Providers |

Unbekannte Metadaten-Schlüssel werden abgelehnt. Legen Sie keine beliebigen
Anwendungs- oder Kundendaten in dieses Objekt. Rückkehr-URLs müssen
denselben HTTPS-Origin wie der Callback verwenden, ohne Zugangsdaten und
ohne Fragment. Rechnungs-Weiterleitungen müssen auf die gewählte
NOWPayments-Umgebung zeigen und die zurückgegebene Rechnungs-ID enthalten.

## Fehler bei der Erstellung und Wiederholungen

Der Rechnungs-Endpunkt von NOWPayments dokumentiert keine Garantie für
einen Idempotency-Key. Der Adapter lehnt einen `idempotency_key` ab, der
nicht `None` ist; `order_id` sind Korrelationsdaten, keine
providerseitige Deduplizierung. Weder HTTP-Weiterleitungen noch
automatische Wiederholungen sind aktiviert. Anfragen haben eine Frist von
30 Sekunden und ein Antwortlimit von 64 KiB.

`InvoiceCreationError::Rejected` steht für eine Eingabevalidierung oder
eine eindeutige Ablehnung durch die API. `Unknown` bedeutet, dass eine
Rechnung bereits existieren kann: etwa weil die Anfrage in eine
Zeitüberschreitung lief, der Provider einen Serverfehler lieferte oder
seine Erfolgsantwort ungültig war. Markieren Sie diesen Versuch als
unsicher und gleichen Sie die Bestellung im Dashboard des Providers ab,
bevor Sie eine weitere Rechnung erstellen. Legen Sie die
Rechnungserstellung nicht in eine allgemeine Retry-Schleife. Über
`Checkout::start_session` ist ein unbekanntes Ergebnis ein
`PaymentError::Provider` mit dieser Wiederherstellungsanweisung.

## Zahlungen verifizieren und abgleichen

Eine Rechnung kann eine eigene Zahlungs-ID erzeugen.
`payment_status(payment_id)` ruft den authentifizierten
Zahlungs-Endpunkt auf und lehnt eine Antwort mit einer abweichenden ID ab.
`Checkout::session_status(invoice_id)` liefert `NotSupported`; eine
Rechnungs-ID lässt sich nicht in den Endpunkt für die Zahlungsabfrage
einsetzen. Dieser Adapter verwendet keine E-Mail- und Passwort-Zugangsdaten
des Dashboards, um Zahlungen nach Rechnung aufzulisten.

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

Die Zahlungs-ID muss aus einem verifizierten IPN oder einem anderen
vertrauenswürdigen serverseitigen Datensatz stammen. Eine Browser-Rückkehr
ist kein Zahlungsnachweis. Eine erfolgreiche Abfrage ist auch keine
Autorisierungsprüfung: Ordnen Sie sie der gespeicherten Bestellung, der
Rechnung, dem Betrag und der Währung zu, bevor Sie Zugriff ändern. `price`
ist der angeforderte Fiat-Preis, nicht die empfangene Krypto-Menge. Prüfen
Sie die Einstellungen des Händlers zur Annahme von Teilzahlungen; dieser
Adapter wandelt `partially_paid` niemals in einen Erfolg um.

| Provider-Status | Neutrales Event |
| --- | --- |
| `finished` | `PaymentSucceeded` |
| `failed`, `expired`, `cancelled`, `canceled` | `PaymentFailed` |
| `refunded` | `PaymentRefunded` |
| `waiting`, `confirming`, `confirmed`, `sending`, `partially_paid`, unbekannt | Keine neutrale Klassifizierung |

IPN-Signaturen verwenden rekursiv sortiertes JSON und HMAC SHA-512. Die
Verifizierung nutzt einen MAC-Vergleich in konstanter Zeit. Fehlende,
doppelte oder ungültige Signaturen, doppelte JSON-Schlüssel, übergroße
Payloads und fehlerhafte Pflichtfelder werden abgelehnt. Der Provider
signiert keinen eigenständigen Zustellzeitstempel, daher gibt es kein
erfundenes Zeitstempel-Replay-Fenster. Der Replay-Schutz beruht auf
persistierten Empfangsbestätigungen.

NOWPayments-IPNs haben keine eigenständige Event-ID. Der Adapter
identifiziert eine Empfangsbestätigung über Zahlungs-ID und Status, sodass
eine erneute Zustellung mit einem anderen `updated_at` dasselbe finale
Event nicht wiederholt. Das Framework speichert verifizierte Events in
`payments_webhook_events` und wiederholt fehlgeschlagene Verarbeitung. Ein
verspätetes ausstehendes Event verändert einen abgeschlossenen
Transaktions-Mirror nicht.

### Rechnungen ohne Kunden und der Anwendungszustand

Die Rechnungs-API liefert nicht die Kundenidentität, die Suprnovas
Transaktions-Mirrors verlangen. Dieser Adapter liefert `false` aus
`WebhookHandler::mirrors_payment_transactions`: alle verifizierten Events,
auch Rückerstattungen, bleiben im Audit-Log, aber er legt weder einen
Kunden- noch einen Transaktions-Mirror an. Stripe und Paddle behalten das
voreingestellte Mirror-Verhalten.

Die generische Webhook-Route zeichnet Provider-Events auf und verarbeitet
sie; sie erfüllt keine Bestellungen der Anwendung. Ein Abgleich-Job der
Anwendung sollte verifizierte Empfangsbestätigungen konsumieren, den
aktuellen Zahlungsstatus lesen und seine Bestellung in einer idempotenten
Transaktion aktualisieren. Speichern Sie einen eindeutigen Provider-,
Zahlungs- oder Bestell-Erfüllungsschlüssel, damit Wiederholungen und
umsortierte Zustellungen Zugriff nicht zweimal gewähren. Führen Sie den
eigenen dauerhaften Abschlusszustand des Jobs getrennt vom `processed_at`
des Frameworks. Verwenden Sie für verspätete Events den aktuellen
authentifizierten Zustand und behandeln Sie Rückerstattungen nach der
Richtlinie der Anwendung.

## Grenzen der Fähigkeiten

Operationen von `CustomerStore` und `Subscription` liefern `NotSupported`.
`as_payment()` und `as_promotions()` liefern `None`. Dieser Adapter
implementiert keine Verwahrungs-Guthaben, keine wiederkehrende Abrechnung,
keinen Checkout direkt über eine Einzahlungsadresse, keine Auszahlungen,
kein Auslösen von Rückerstattungen und keine Kundenverwaltung beim
Provider. NOWPayments bietet für einige dieser Operationen eigene Produkte
an; dieser Rechnungs-Adapter gibt nicht vor, sie zu implementieren.

### Warum Suprnova abweicht

Die gemeinsamen Provider-Traits bleiben verfügbar, nicht unterstützte
Operationen schlagen jedoch ausdrücklich fehl. Rechnungen ohne
Provider-Kunden nutzen auditierte Benachrichtigungen und Bestellungen im
Besitz der Anwendung statt erfundener Kundendatensätze oder Subscriptions.

## Verifizierung

Führen Sie die Adapter-Tests und die gemeinsamen Zahlungs-Regressionen aus
dem Quellcode-Checkout aus:

```sh
CARGO_INCREMENTAL=0 cargo nextest run -p suprnova-payments-nowpayments
CARGO_INCREMENTAL=0 cargo nextest run -p suprnova --test payments
```

Die lokale Suite verwendet gefälschte HTTP-Antworten, ein unabhängig
erzeugtes JavaScript-Signatur-Fixture und echten Framework-Ingress mit
SQLite. Sie deckt erfolgreichen Checkout, Authentifizierungsfehler,
fehlerhafte oder große Antworten, Zeitüberschreitungen, nicht
vertrauenswürdige Weiterleitungen, ungültige Signaturen, Status-Mapping,
doppelte Zustellungen und die Wiederherstellung nach einem Datenbankfehler
ab. Lokale Fixtures belegen keine Interoperabilität mit dem Provider.

Bevor Sie die Produktion aktivieren, verwenden Sie ein
NOWPayments-Sandbox-Konto und einen erreichbaren HTTPS-Callback. Erstellen
Sie eine Rechnung, durchlaufen Sie die verfügbaren Erfolgs-, Teil- und
Fehlerfälle des Providers, bestätigen Sie, dass dessen Signatur akzeptiert
wird, und gleichen Sie die Zahlungs-ID mit der gespeicherten Rechnung und
Bestellung ab. Spielen Sie eine Zustellung erneut ein und bestätigen Sie,
dass der Erfüllungsschlüssel der Anwendung eine zweite Gewährung
verhindert. Die lokalen Tests implizieren keinen Lauf mit einem echten
Sandbox- oder Produktionskonto.

Provider-Referenzen: [API-Endpunkte](https://nowpayments.zendesk.com/hc/en-us/articles/21345824322717-API-and-endpoint-description),
[IPN-Authentifizierung](https://nowpayments.zendesk.com/hc/en-us/articles/21395546303389-IPN-and-how-to-setup)
und das [offizielle SDK](https://github.com/NowPaymentsIO/nowpayments-sdk-nodejs).

## Nächste Schritte

Siehe [Frontend-Integration](payments-frontend.md) für das Rendern von
Redirect-Payloads oder den [Provider-Leitfaden](payments-provider-guide.md)
für die gemeinsamen Verträge.
