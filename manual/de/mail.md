# Mail

Suprnovas Mail-Subsystem spiegelt Laravels
`Mail::to(...)->send(...)`-API auf Tokio. Eine `Mail`-Facade, acht
Transporte (log und in-memory für Dev/Tests, SMTP und fünf
HTTP-Provider - Postmark, SES, SendGrid, Mailgun, Resend),
Tera-gerenderte Templates mit den serialisierten Feldern des
Mailable als Kontext, Queueing + verzögerte Zustellung auf der
dauerhaften At-least-once-Envelope, und ein
`Mail::fake()`-Test-Guard, gebaut nach demselben Muster wie
`Bus::fake()` und `Cache::fake()`.

## Schnellstart

```rust
use serde::{Deserialize, Serialize};
use suprnova::async_trait;
use suprnova::mail::{Address, Mail, Mailable};

#[derive(Serialize, Deserialize)]
struct Welcome {
    name: String,
}

#[async_trait]
impl Mailable for Welcome {
    fn mailable_name() -> &'static str { "Welcome" }
    fn subject(&self) -> String { format!("Welcome, {}", self.name) }
    fn text_template_source(&self) -> Option<String> {
        Some("Hi {{ name }}, welcome aboard.".into())
    }
    fn from(&self) -> Option<Address> {
        Some(Address::new("hello@example.com").with_name("Suprnova"))
    }
}

async fn greet(name: String) -> Result<(), suprnova::FrameworkError> {
    Mail::to("alice@example.org")
        .send(Welcome { name })
        .await
}
```

Das Mailable serialisiert zu JSON, was zum Tera-Kontext für das
Template wird; jedes `pub`-Feld ist als `{{ field_name }}`
erreichbar.

## Konfiguration

`Server::serve` ruft einmal beim Start
`suprnova::mail::boot::bootstrap_from_env()` auf. Es liest
`MAIL_DRIVER` und bindet den passenden Transport. Standardmäßig der
`log`-Treiber, wenn nicht gesetzt.

| `MAIL_DRIVER` | Verhalten |
|---------------|----------|
| `log`         | Gibt pro Sendung ein `tracing::info!` aus - Umschlag und vollständige Bodys, wie Laravel es tut - und verwirft. Standard außerhalb der Produktion. |
| `memory`      | Erfasst jede Nachricht im Prozess. Siehe `suprnova::mail::boot::captured_in_memory()`. |
| `smtp`        | Verbindet sich mit einem SMTP-Server (STARTTLS, wenn Credentials gesetzt sind, sonst reines TCP). |
| `postmark`    | POSTet JSON an Postmarks `/email`-Endpunkt. |
| `ses`         | POSTet SigV4-signierte Anfragen an Amazon SES `SendEmail`. |
| `sendgrid`    | POSTet JSON an SendGrids `/v3/mail/send`. |
| `mailgun`     | POSTet `application/x-www-form-urlencoded` (oder `multipart/form-data`, wenn Attachments vorhanden sind) an Mailguns `/v3/{domain}/messages`. |
| `resend`      | POSTet JSON an Resends `/emails`. |

### Produktion ist fail-closed bei einem Treiber, der Mail verwirft

`log` und `memory` rendern eine Nachricht und verwerfen sie. Unter
`APP_ENV=production` **weigert sich** der Boot, mit einem der
beiden zu starten - und ebenso mit einem ungesetzten `MAIL_DRIVER`
oder einem Wert, den der Build nicht erkennt, weil beide auf
demselben `log`-Transport landen:

```
refusing to boot in production: MAIL_DRIVER is unset, which defaults to the `log`
transport. Password resets and email verifications would report success while
nothing is delivered. Set MAIL_DRIVER to a delivering driver (smtp | postmark |
ses | sendgrid | mailgun | resend), or set
MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true to acknowledge that outgoing mail is
intentionally discarded.
```

Der Fehlschlag, den das verhindert, ist ein stiller: Mit dem alten
Standard meldete ein Deploy, das `MAIL_DRIVER` vergessen hatte -
oder `MAIL_DRIVER=SMTP` in falscher Groß-/Kleinschreibung schrieb -
jeden Passwort-Reset als versendet, während nie etwas den Prozess
verließ, und niemand bemerkte es, bis ein Nutzer ausgesperrt war.

Wenn ein Produktions-Deployment tatsächlich keine ausgehende Mail
will (ein Read-only-Mirror, ein Dark Launch), bestätigen Sie das
explizit:

```env
MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true
```

Nur `1`, `true`, `yes` oder `on` zählen als Zustimmung -
`=false` oder ein Tippfehler lässt die Absicherung scharf. Mit
gesetztem Override warnt jeder Boot, dass ausgehende Mail nicht
zugestellt wird.

Außerhalb der Produktion ändert sich nichts: `local`,
`development`, `testing` und `staging` behalten den `log`-Standard
und das Warn-und-Fallback-Verhalten bei unbekannten Treibern bei.

### Produktion ist fail-closed bei einer unverschlüsselten SMTP-Verbindung

Dieselbe Regel, angewendet darauf, wie die Verbindung geschützt
wird, statt darauf, ob sie zustellt. `MAIL_DRIVER=smtp` muss in
Produktion zu einem verschlüsselten Transport auflösen, sonst
schlägt der Boot fehl.

`MAIL_SMTP_ENCRYPTION` akzeptiert `starttls`, `tls` oder `none`
(`ssl` und `null` werden als Laravel-kompatible Aliase akzeptiert).
Unbesetzt leitet es sich aus den Credentials ab:

| `MAIL_SMTP_USER` / `MAIL_SMTP_PASS` | Löst auf zu | Weil |
|---|---|---|
| beide gesetzt | `starttls` | Credentials implizieren ein echtes Relay auf dem Submission-Port. |
| keins gesetzt | `none` | Der Lokaler-Catcher-Pfad. Mailpit, MailHog und maildev lauschen unauthentifiziert auf 1025 und sprechen kein TLS. |

Ein frischer Scaffold funktioniert also mit Null-Konfiguration
weiter, und ein Produktions-Deploy, das die Credentials nie
verdrahtet hat, hält an, statt still im Klartext zu senden. Setzen
Sie `MAIL_SMTP_ENCRYPTION=tls` für ein Relay, das implizites TLS
auf 465 erwartet - ein Modus, den der Transport schon immer
unterstützt hat, den aber keine Kombination von Umgebungsvariablen
zuvor erreichen konnte.

Ein nicht erkannter Wert lässt den Boot in *jeder* Umgebung
fehlschlagen, nicht nur in Produktion. `MAIL_SMTP_ENCRYPTION=tsl`
ist eine Buchstabendreher-Variante eines verschlüsselnden Modus, es
also still als „keine Verschlüsselung“ zu behandeln, wäre genau der
Fehlschlag, den die Variable verhindern soll - besser, auf der
Maschine des Entwicklers zu scheitern als beim Deploy.

Der Notausgang spiegelt den obigen:

```env
MAIL_ALLOW_INSECURE_SMTP_IN_PRODUCTION=true
```

Nur vertretbar, wenn das Relay ausschließlich über ein privates
Netzwerk erreichbar ist - ein Sidecar, oder ein Postfix innerhalb
des VPC. Bei allem anderen legt Cleartext-SMTP die Credentials und
jeden Passwort-Reset-Link unverschlüsselt über das Netz, und sie
bleiben dort für jeden, der den Pfad mitliest.

### Der `log`-Treiber protokolliert die gesamte Nachricht

Wie bei Laravels `log`-Mailer: Umschlag *und* gerenderte Bodys.

```
mail (log driver): would send from=noreply@app.test to=["alice@example.org"]
  subject=Reset your password
  text=Reset your password: https://app.test/password/reset?token=9f3a…&signature=…
  html=<a href="https://app.test/password/reset?token=9f3a…&signature=…">Reset</a>
```

Dieser Link ist der Punkt. In der Entwicklung ist die Konsole der
Ort, an dem Sie den Verifizierungs- oder Passwort-Reset-Link lesen,
den die App gerade „versendet“ hat, und ein Treiber, der ihn
versteckt, ist ein Treiber, den niemand nutzen kann.

Es ist hier sicher, weil der Treiber Produktion nicht erreichen
kann - der Boot weigert sich, bei `MAIL_DRIVER=log` unter
`APP_ENV=production` zu starten (siehe oben). Die Bodys existieren
nur je auf der Maschine eines Entwicklers.

Setzen Sie `MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true`, um den
`log`-Treiber in einer deployten Umgebung laufen zu lassen,
entscheiden Sie sich dafür, Single-use-Bearer-Links in Ihren Logs
abzulegen. Jeder, der diese Dateien lesen kann - Betreiber, der
Log-Shipper, der Retention-Bucket, der Aggregator - kann sie
verwenden, und Link-Ablauf hilft nicht, weil Log-Shipping schneller
ist, als eine Person ihr Postfach liest. Bemessen Sie Retention und
Zugriffsrichtlinie dafür, oder verwenden Sie einen Treiber, der
nicht druckt:

```env
# In-Process-Erfassung - suprnova::mail::boot::captured_in_memory(), oder Mail::fake() in Tests
MAIL_DRIVER=memory

# Oder ein lokaler Catcher (mailpit / maildev / mailhog), der die echte Mail in einer UI rendert
MAIL_DRIVER=smtp
MAIL_SMTP_HOST=127.0.0.1
MAIL_SMTP_PORT=1025
```

### Treiberspezifische Umgebungsvariablen

```env
# SMTP
MAIL_DRIVER=smtp
MAIL_SMTP_HOST=smtp.mailtrap.io
MAIL_SMTP_PORT=587
MAIL_SMTP_USER=...
MAIL_SMTP_PASS=...
MAIL_SMTP_ENCRYPTION=starttls   # oder `tls` für implizites TLS auf 465, oder `none`

# Postmark
MAIL_DRIVER=postmark
MAIL_POSTMARK_TOKEN=...

# Amazon SES
MAIL_DRIVER=ses
MAIL_SES_ACCESS_KEY=...
MAIL_SES_SECRET_KEY=...
MAIL_SES_REGION=us-east-1

# SendGrid
MAIL_DRIVER=sendgrid
MAIL_SENDGRID_API_KEY=...

# Mailgun
MAIL_DRIVER=mailgun
MAIL_MAILGUN_API_KEY=...
MAIL_MAILGUN_DOMAIN=mg.example.com

# Resend
MAIL_DRIVER=resend
MAIL_RESEND_API_KEY=...
```

Jeder HTTP-Provider honoriert außerdem ein entsprechendes
`MAIL_<PROVIDER>_ENDPOINT`-Override, das auf eine regionale URL
oder einen Mock-Server zeigt (nützlich für Integrationstests gegen
`wiremock`).

### Auth-Flow-Absender: `MAIL_FROM` und `MAIL_FROM_NAME`

Die eingebauten Auth-Flow-Mailables - E-Mail-Verifizierung,
Passwort-Reset und der Passwort-geändert-Hinweis - lösen ihr
Umschlag-`From` aus der Umgebung auf statt aus einem
festverdrahteten `from()`:

```env
MAIL_FROM=no-reply@example.com        # reine Adresse (von den Auth-Flows verlangt; fail-closed, wenn nicht gesetzt)
MAIL_FROM_NAME=Acme Support           # optionaler Anzeigename (seit 0.5.9)
```

- `MAIL_FROM` **muss eine reine Adresse sein.** Sie wird direkt in
  das `From` der Nachricht gehoben, sodass ein Wert `"Name <addr>"`
  als die gesamte Adresse behandelt und vom Transport abgelehnt
  würde.
- `MAIL_FROM_NAME` (optional, hinzugefügt in **0.5.9**) hängt einen
  Anzeigenamen an, sodass der Header als `Acme Support
  <no-reply@example.com>` rendert. Unbesetzt oder leer behält das
  bisherige Reine-Adresse-Verhalten bei. Es wird zur Sendezeit
  gelesen, gilt also auch für eingereihte Auth-Flow-Mail.

Diese beiden Variablen betreffen nur die eigenen
Auth-Flow-Mailables des Frameworks. Ihre eigenen `Mailable`s setzen
ihren Absender über `from()` (oder den globalen
`always_from`-Standard) - siehe unten.

## Der Mailable-Trait

Mailables sind serialisierbare Strukturen, die wissen, wie sie sich
selbst rendern. Die Standardimplementierung des Traits rendert mit
`tera::Tera::one_off` gegen die serialisierten Felder des Mailable:

```rust
use suprnova::async_trait;
use suprnova::mail::{Address, Attachment, Mailable};

#[async_trait]
impl Mailable for OrderShipped {
    fn mailable_name() -> &'static str { "OrderShipped" }
    fn subject(&self) -> String {
        format!("Order #{} shipped", self.order_id)
    }
    fn html_template_source(&self) -> Option<String> {
        Some("<p>Tracking: <code>{{ tracking }}</code></p>".into())
    }
    fn text_template_source(&self) -> Option<String> {
        Some("Tracking: {{ tracking }}".into())
    }
    fn from(&self) -> Option<Address> {
        Some(Address::new("orders@example.com").with_name("Acme Orders"))
    }
    fn attachments(&self) -> Vec<Attachment> {
        vec![Attachment::new("invoice.pdf", self.invoice_bytes.clone(), "application/pdf")]
    }
}
```

| Methode | Erforderlich? | Zweck |
|--------|-----------|---------|
| `mailable_name()` | ja | Stabiler, im Queue-Envelope persistierter Name - Umbenennen bricht in-flight eingereihte Mail. |
| `subject(&self)` | ja | Berechneter Betreff. Wörtlich verwendet, wenn `subject_template_source` `None` liefert. |
| `subject_template_source(&self)` | optional | Tera-Template für den Betreff - wenn `Some`, hat Vorrang vor `subject()` und rendert mit `self` als Kontext. Dieselbe Semantik wie die Body-Template-Quellen. |
| `html_template_source(&self)` | optional | HTML-Body-Tera-Template. `None` zurückgeben, um HTML zu überspringen. |
| `text_template_source(&self)` | optional | Klartext-Body-Tera-Template. `None` zurückgeben, um Text zu überspringen. |
| `from(&self)` | optional | Überschreibt den globalen Standard `noreply@localhost`. |
| `attachments(&self)` | optional | Anzuhängende Dateien. Jede ist `name + bytes + mime`. |
| `render_subject(&self)` / `render_html(&self)` / `render_text(&self)` | optional | Überschreiben, wenn Sie Tera umgehen wollen (Markdown → HTML, vorgerenderter Content, benutzerdefinierte Betreff-Logik usw.). |

Mindestens eine von `html_template_source` oder
`text_template_source` muss `Some` liefern (oder `render_html`/
`render_text` müssen Content produzieren). Ein Mailable mit leerem
Body wird sowohl beim Dispatch (`Mail::send`) als auch beim
Einreihen (`Mail::queue`) abgelehnt.

### Tera-Autoescape

Autoescape ist **AUS**, weil Mail-Bodys typischerweise
handgeschriebenes HTML sind, bei dem Teras `<>&`-Escaping
überescapen würde. Wenn Ihr wörtlicher Body `{{` aus
Nicht-Template-Gründen enthält (z. B. Marketing-Text, der
Mustache-Syntax zitiert), escapen Sie es:
`{% raw %}{{ literal }}{% endraw %}`.

## Nachrichten aufbauen

Der `Mail::to(...)`-Builder fädelt Empfänger, CC/BCC, Reply-To und
ein Per-Message-Absender-Override in den Dispatch ein:

```rust
Mail::to("alice@example.org")
    .cc("manager@example.com")
    .bcc("audit@example.com")
    .reply_to("support@example.com")
    .from(("Operations", "ops@example.com"))   // (Anzeigename, E-Mail)
    .send(OrderShipped { order_id: 42, /* ... */ })
    .await?;
```

`Address` akzeptiert `&str`, `String` und `(name, email)`-Tupel;
`Mail::to(...)` akzeptiert alles, was `Into<Address>` ist.

## Attachments

```rust
use suprnova::mail::Attachment;

let attachment = Attachment::new(
    "report.csv",
    csv_bytes,
    "text/csv",
);
```

Attachments reisen über die Methode `Mailable::attachments`. Alle
fünf HTTP-Provider behandeln sie - Postmark/SendGrid/Resend über
JSON (base64-kodiert), SES über Raw MIME (weil `Content.Simple`
keine Attachments unterstützt), und Mailgun über
`multipart/form-data` (der form-kodierte Pfad wird verwendet, wenn
keine Attachments vorhanden sind).

## Einreihen

`Mail::queue(...)` baut einen `SendMailJob` und schiebt ihn auf die
Framework-Queue. Der Worker baut das Mailable aus der registrierten
Factory neu auf und dispatcht durch den gebundenen Transport:

```rust
// Einmalig: jeden Mailable-Typ registrieren, den der Worker sehen wird.
suprnova::mail::register_mailable_factory::<Welcome>()?;

// Zur Sendezeit:
Mail::to("alice@example.org").queue(Welcome { name: "Alice".into() }).await?;

// Verzögert:
use std::time::Duration;
Mail::to("alice@example.org")
    .later(Duration::from_secs(60), Welcome { name: "Alice".into() })
    .await?;
```

Dieselbe Leerer-Body-Absicherung läuft auf dem Queue-Pfad, sodass
ein falsch konfiguriertes Mailable bereits zur Push-Zeit abgelehnt
wird, bevor eine Envelope erzeugt wird.

## Telemetrie

Jede Sendung läuft durch
`suprnova::mail::dispatch_with_telemetry`, das einen
`mail.send`-`tracing::info_span!` öffnet, der trägt:

- `transport` - Treibername (`"postmark"`, `"smtp"`, `"in-memory"`, …)
- `to_count`, `cc_count`, `bcc_count` - Empfänger-Zahlen
- `has_html`, `has_text` - Body-Form
- `attachment_count` - Anzahl der Attachments
- `tag_count`, `metadata_count` - Provider-Hinweis-Zahlen
- `priority` - `1..=5`, oder `0`, wenn nicht gesetzt

Beim Abschluss emittiert der Span `mail sent` (info) oder `mail
send failed` (warn) mit `duration_ms`. Derselbe Wrapper deckt
`Mail::send`, den `SendMailJob`-Queue-Worker und den
Benachrichtigungskanal `MailChannel` ab, sodass das Span-Schema
unabhängig davon identisch ist, wie die Nachricht erzeugt wurde.

## Testen mit `Mail::fake()`

`Mail::fake()` installiert für die Dauer des zurückgegebenen
RAII-Guards einen In-Memory-Erfassungs-Transport. Spiegelt
`Bus::fake()` / `Queue::fake()` / `Cache::fake()`:

```rust
use suprnova::mail::Mail;

#[tokio::test]
async fn welcome_mail_is_sent_on_signup() {
    let fake = Mail::fake();

    sign_up("alice@example.org").await.unwrap();

    fake.assert_sent_count(1);
    fake.assert_sent(|m| m.to.iter().any(|a| a.email == "alice@example.org"));
    fake.assert_sent(|m| m.subject.starts_with("Willkommen"));
    fake.assert_not_sent(|m| m.subject.contains("Passwort zurücksetzen"));
}
```

Wenn der Guard droppt, wird der zuvor gebundene Transport (falls
vorhanden) wiederhergestellt. Tests, die `Mail::fake()` mit
explizitem Transport-Binding mischen, lecken keinen Zustand.

`Mail::fake()` ist `Send + Sync`; teilen Sie es bei Bedarf über
Awaits oder Threads hinweg.

## Benutzerdefinierte Transporte

Das `MailTransport`-Trait ist der Integrationspunkt:

```rust
use suprnova::async_trait;
use suprnova::mail::{MailTransport, OutgoingMessage};
use suprnova::FrameworkError;

pub struct StdoutTransport;

#[async_trait]
impl MailTransport for StdoutTransport {
    async fn send(&self, msg: &OutgoingMessage) -> Result<(), FrameworkError> {
        println!("--- mail ---\n{}\n--- end ---", msg.subject);
        Ok(())
    }
    fn name(&self) -> &'static str { "stdout" }
}

// Beim Boot:
use std::sync::Arc;
suprnova::mail::Mail::set_transport(Arc::new(StdoutTransport))?;
```

Transporte laufen auf Tokios Runtime - asynchrones IO,
Connection-Pooling und gleichzeitiges Senden sind erstklassig. Es
gibt keine Per-Request-Fork-Strafe.

### Warum Suprnova abweicht

Laravels Mailable-Schicht baut auf Symfony Mailer auf, der
synchron innerhalb des Request-Lebenszyklus läuft. Suprnovas
`MailTransport` ist end-to-end `async fn send(&self, msg:
&OutgoingMessage)`: Die HTTP-Provider verwenden `reqwest`, der
SMTP-Pfad verwendet einen asynchronen lettre-Adapter, und
`dispatch_with_telemetry` hüllt jede Sendung in einen
Tokio-`tracing`-Span. Langlaufende Provider blockieren den
Handler-Thread nicht, Connection-Pools überleben über Anfragen
hinweg, und gleichzeitige Sendungen in einem Handler sind trivial -
`tokio::try_join!(Mail::to(a).send(m), Mail::to(b).send(n))` tut
genau das, was Sie erwarten würden.

Die andere Abweichung ist die Event-Abbrechbarkeit. Laravel
modelliert einen `MessageSending`-Listener, der `false`
zurückgeben und die Sendung unterdrücken kann (`events->until()`).
Suprnovas Dispatcher legt keinen Short-Circuit-Rückkanal frei -
`MessageSending` ist rein beobachtend. Um eine Sendung zu
unterbinden, verweigern Sie sie auf der Mailable-Schicht
(überschreiben Sie `render_html` / `render_text`, um einen Fehler
zurückzugeben) oder umschließen Sie den Aufruf von
`MailBuilder::send` mit einer eigenen Absicherung. Der Trade ist
echt: Wir verlieren einen Laravel-Hook, um den Vertrag des
Dispatchers einfach zu halten.

Eine kleinere, absichtliche Abweichung ist Härtung. Laravel lässt
`MAIL_MAILER=log` bereitwillig in Produktion laufen; Suprnova
weigert sich, dort ohne explizite Bestätigung zu booten, weil ein
Mail-Subsystem, das Erfolg meldet und nichts zustellt, die Art von
Ausfall ist, die wochenlang niemandem auffällt. Der `log`-Treiber
selbst verhält sich exakt wie Laravels - vollständige Nachricht,
Bodys und Links inklusive -, was ihn in der Entwicklung nützlich
macht, und die Produktions-Verweigerung ist es, was das sicher
hält (siehe [Der `log`-Treiber protokolliert die gesamte
Nachricht](#der-log-treiber-protokolliert-die-gesamte-nachricht)).

## Best Practices

### Factories beim Boot registrieren, nicht pro Anfrage

`Mail::queue` und `Mail::later` schieben einen `SendMailJob`, der
den Namen und JSON-Payload des Mailable trägt - der Worker baut den
konkreten Typ über `mailable_registry` neu auf. Registrieren Sie
jedes einreihbare `Mailable` einmal zur Zeit von `Server::serve`:

```rust
// bootstrap.rs
pub fn register() -> Result<(), suprnova::FrameworkError> {
    suprnova::mail::register_mailable_factory::<WelcomeEmail>()?;
    suprnova::mail::register_mailable_factory::<PasswordReset>()?;
    suprnova::mail::register_mailable_factory::<InvoiceShipped>()?;
    Ok(())
}
```

Ein `Mail::queue` für ein nicht registriertes Mailable landet auf
der Queue, läuft einmal, trifft auf „unknown mailable“, wiederholt
gemäß der Backoff-Policy der Envelope und wird dead-lettert -
was Observability-Zeit kostet, die Sie sich erspart hätten, wäre
die Factory beim Boot gebunden gewesen.

### Mail für jedes langsame oder unzuverlässige Rendern einreihen

Mail in einem Request-Handler zu senden koppelt die
Response-Latenz des Nutzers an Ihren SMTP-Server (oder die
HTTP-API welches Providers auch immer). Verwenden Sie
`Mail::queue` für alles jenseits eines synchronen
Lokal-Dev-Renderns, und `Mail::later`, wenn Sie den Dispatch
verzögert haben wollen - Onboarding-Follow-ups,
Erinnerungs-E-Mails, geplante Digests.

```rust
// Schlecht: koppelt die Response-Zeit an den Mail-Provider
Mail::to(&user.email).send(Welcome { ... }).await?;
return json_response!({ "ok": true });

// Gut: 200 OK kehrt sofort zurück; der Worker liefert die Mail zu.
Mail::to(&user.email).queue(Welcome { ... }).await?;
return json_response!({ "ok": true });
```

### Setzen Sie `from` auf einem Mailable immer

Der Standard-Absender des Frameworks ist `noreply@localhost` -
nützlich, um fehlende Absender in der Entwicklung zu erkennen,
kein Absender, den ein Provider in Produktion akzeptieren wird.
Überschreiben Sie `Mailable::from(&self)` (oder setzen Sie
`from = "..."` im `#[mail(...)]`-Attribut auf einem
`NotificationMailable`), sodass jede dispatchte Nachricht eine
echte Absenderidentität hat:

```rust
fn from(&self) -> Option<Address> {
    Some(Address::new("orders@example.com").with_name("Acme Orders"))
}
```

Das Per-Message-Override auf `MailBuilder`
(`.from(("Operations", "ops@example.com"))`) hat Vorrang vor dem
Standard des Mailable - nützlich für einmalige transaktionale
Sendungen.

### Die Queue für At-least-once-Zustellung verwenden, nicht den direkten Pfad

`MailBuilder::send` ist at-most-once: Wenn der Transport auf
halbem Weg beim Dispatch an zwei Provider fehlschlägt, können Sie
nicht erneut versuchen, ohne einen Doppelversand zu riskieren.
`MailBuilder::queue` reitet auf der dauerhaften Queue-Envelope, die
Idempotenzschlüssel und Wiederholung auf Worker-Ebene unterstützt. Für
jede Mail, die Sie weder verlieren NOCH doppelt versenden dürfen,
reihen Sie mit einem stabilen Idempotenzschlüssel ein, der an das
auslösende Ereignis gebunden ist.

## Einmalige Nachrichten: `Mail::raw` und `Mail::html`

Wenn die Mail ein einzelner transaktionaler Anstoß ist, der keine
vollständige `Mailable`-Struktur rechtfertigt, überspringen zwei
Abkürzungen das Boilerplate:

```rust
use suprnova::mail::Mail;

// Klartext
Mail::raw("Your code is 12345", |b| {
    b.to("alice@example.org")
        .subject("Verification code")
        .from("auth@example.com")
}).await?;

// HTML
Mail::html("<p>Hello, <b>world</b></p>", |b| {
    b.to("alice@example.org")
        .subject("Hallo")
        .from("hello@example.com")
}).await?;
```

Die Closure bekommt einen mit dem Body vorgeladenen [`MailBuilder`]
und lässt Sie Empfänger, Betreff, Absender, Tags, Metadaten,
Priorität und jede andere fluent Methode von [`MailBuilder`]
obendrauf schichten. Diese Pfade umgehen das `Mailable`-Trait
vollständig - nützlich für einmalige Test-Pings und kurze
transaktionale Notizen.

## Globale Standards: `always_from`, `always_reply_to`, `always_to`, `always_return_path`

Spiegelnd zu Laravels `Mailer::alwaysFrom` / `alwaysReplyTo` /
`alwaysTo` / `alwaysReturnPath` legt die Mail-Facade vier globale
Setter offen:

```rust
use suprnova::mail::{Address, Mail};

// Beim Boot:
Mail::always_from(Address::new("noreply@example.com").with_name("Acme"))?;
Mail::always_reply_to(Address::new("support@example.com"))?;
Mail::always_return_path(Address::new("bounce@example.com"))?;

// Lokal-Dev „Einzel-Postfach“ - route ALLE Mail an eine Adresse, verwirf CC/BCC:
Mail::always_to(Address::new("dev-inbox@example.com"))?;

// Alles zurückrollen (Tests rufen das typischerweise beim Teardown auf):
Mail::forget_always()?;
```

Die Präzedenz ist konservativ - Standards greifen nur, wenn der
dispatchten Nachricht ein expliziter Wert fehlt:

| Feld | Standard greift, wenn |
|-------|---------------------|
| `always_from` | `from` der Nachricht der Framework-Standard `noreply@localhost` ist |
| `always_reply_to` | Nachricht kein explizites `reply_to` hat |
| `always_to` | Immer - routet jede Nachricht an diese Adresse, leert CC/BCC |
| `always_return_path` | Nachricht kein explizites `return_path` hat |

Dieselbe Präzedenz gilt auf dem Queue-Pfad: Eingereihte Mailables
durchlaufen `apply_always_defaults` zur Worker-Dispatch-Zeit,
sodass direkte Sendungen und eingereihte Sendungen auf identischen
Umschlag-Formen zusammenlaufen.

## Tags, Metadaten, Priorität, Header, Return-Path

Jede dispatchte Nachricht kann Provider-Hinweise im Laravel-Stil
tragen - Tags, Metadaten-Schlüssel/-Werte, RFC-2076-Priorität, eigene
MIME-Header und eine Sender- / Bounce-to-Adresse. Sie werden an die
nativen Felder der HTTP-Provider weitergereicht (Postmark `Tag` /
`Metadata` / `Headers`, SES `EmailTags` plus `Content.Simple.Headers`,
SendGrid `categories` / `custom_args` / `headers`, Mailgun `o:tag` /
`v:` / `h:`, Resend `tags` / `headers`) und an SMTP als
RFC-5322-Header.

Speziell bei SES reiten die Header auf der Content-Form mit, die die
Nachricht verwendet: `Content.Simple.Headers` bei einer einfachen
Nachricht, echte MIME-Header-Zeilen bei einer Nachricht mit Anhängen
(die SES nur als Raw-MIME akzeptiert). Ein Header-Name wird gleich
validiert, unabhängig davon, welche Form die Nachricht am Ende
nutzt - CR, LF und NUL werden abgelehnt (so wird aus einer vom
Aufrufer gelieferten Zeichenkette ein zweiter Header), und ebenso ein
leerer Name, ein Name über 76 Bytes, ein Nicht-ASCII-Byte oder ein `:`
bzw. ein Leerzeichen im Namen, passend zu dem, was der
Raw-MIME-Builder selbst verlangt. Ein mehr als einmal wiederholter
Header-Name behält auf dem Pfad der einfachen Nachricht jeden Wert,
auf dem Anhang-Pfad aber nur den letzten - dieselbe Grenze, die SMTP
hat.

Zwei Wege, sie anzuhängen - auf Ebene des Mailable für Standards pro
Typ, oder pro Nachricht auf dem Builder:

```rust
use suprnova::async_trait;
use suprnova::mail::{Mailable, PRIORITY_HIGH};
use std::collections::BTreeMap;

#[async_trait]
impl Mailable for OrderShipped {
    fn mailable_name() -> &'static str { "OrderShipped" }
    fn subject(&self) -> String { format!("Order #{} shipped", self.order_id) }
    fn text_template_source(&self) -> Option<String> { Some("...".into()) }

    fn tags(&self) -> Vec<String> { vec!["transactional".into(), "order".into()] }
    fn metadata(&self) -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        m.insert("order_id".into(), self.order_id.to_string());
        m
    }
    fn priority(&self) -> Option<u8> { Some(PRIORITY_HIGH) }
    fn headers(&self) -> Vec<(String, String)> {
        vec![("X-Origin".into(), "warehouse".into())]
    }
}
```

```rust
// Pro Nachricht auf dem Builder. Bei Kollisionen von Metadaten-Schlüsseln gewinnt der Builder; Tags + Header werden vereinigt.
Mail::to(&user.email)
    .tag("campaign-spring")
    .metadata("ab_variant", "B")
    .priority(1)
    .header("X-Source", "promo-feed")
    .return_path("bounce@example.com")
    .send(WelcomeEmail { name: user.name.clone() })
    .await?;
```

Konstanten für die fünf Prioritätsstufen liegen unter
`suprnova::mail::{PRIORITY_HIGHEST, PRIORITY_HIGH, PRIORITY_NORMAL, PRIORITY_LOW, PRIORITY_LOWEST}` -
dieselbe Ganzzahlskala `1..=5`, die Laravel verwendet.

## Erfasste Nachrichten untersuchen

`OutgoingMessage` trägt Laravel-artige Inspektions-Helfer -
nützlich sowohl für Test-Assertions als auch für
Laufzeit-Audit-Protokollierung:

```rust
fn audit_outgoing(m: &suprnova::mail::OutgoingMessage) {
    if m.has_tag("transactional") && m.has_to("alice@example.org") { /* ... */ }
    if m.has_metadata("order_id") { /* ... */ }
    if m.has_subject("Willkommen") { /* ... */ }
    if m.has_attachment("invoice.pdf") { /* ... */ }
    if m.has_header("X-Source", "promo-feed") { /* ... */ }
}
```

Empfänger-Prüfungen sind case-insensitiv auf der E-Mail;
Metadaten-, Tag-, Betreff- und Attachment-Dateiname-Prüfungen sind
exakt.

## Test-Fake: Erweiterte Oberfläche

`Mail::fake()` deckt SOWOHL den Sende- als auch den
Einreihungs-Track ab. Gesendete Mail (über `MailBuilder::send`)
landet im In-Memory-Transport; eingereihte Mail (über `.queue` /
`.later`) landet im Queue-Puffer des Fake.

```rust
use suprnova::mail::Mail;

#[tokio::test]
async fn boot_dispatches_welcome() {
    let fake = Mail::fake();

    onboard_user("alice@example.org").await.unwrap();

    // Sende-Seite
    fake.assert_sent_count(1);
    fake.assert_sent(|m| m.has_to("alice@example.org") && m.subject.starts_with("Willkommen"));
    fake.assert_sent_to("alice@example.org");
    fake.assert_not_sent(|m| m.subject.contains("Passwort zurücksetzen"));

    // Einreihungs-Seite (für verzögerte Mails)
    fake.assert_queued("WelcomeFollowup");
    fake.assert_queued_to("alice@example.org");
    fake.assert_queued_count(1);

    // Zusammengesetzt
    fake.assert_outgoing_count(2);   // gesendet + eingereiht
    fake.assert_not_outgoing("PasswordReset");
}
```

Weitere Helfer:

| Helfer | Zweck |
|--------|-------|
| `fake.captured()` | Alle gesendeten Nachrichten |
| `fake.count()` | Anzahl gesendet |
| `fake.queued()` | Alle eingereihten `QueuedSnapshot`s |
| `fake.queued_count()` | Anzahl eingereiht |
| `fake.outgoing_count()` | Gesendet + eingereiht |
| `fake.sent(predicate)` | Gesendete nach Prädikat filtern |
| `fake.sent_to(email)` | Gesendete nach Empfänger filtern |
| `fake.queued_named(name)` | Eingereihte Mailables eines gegebenen Namens |
| `fake.queued_to(email)` | Eingereihte Mailables an Empfänger |
| `fake.assert_sent_count(n)` | Exakte Anzahl gesendet |
| `fake.assert_queued_count(n)` | Exakte Anzahl eingereiht |
| `fake.assert_outgoing_count(n)` | Exakte Gesamtzahl |
| `fake.assert_nothing_sent()` | Leerer Sende-Puffer |
| `fake.assert_nothing_queued()` | Leerer Einreihungs-Puffer |
| `fake.assert_nothing_outgoing()` | Beide leer |
| `fake.assert_sent_to(email)` | Mindestens eine an Empfänger gesendet |
| `fake.assert_not_sent_to(email)` | Keine an Empfänger gesendet |
| `fake.assert_queued(name)` | Mindestens eine des Namens eingereiht |
| `fake.assert_queued_with(name, fn)` | Mindestens eine des Namens eingereiht, die dem Prädikat entspricht |
| `fake.assert_queued_to(email)` | Mindestens eine an Empfänger eingereiht |
| `fake.assert_not_queued(name)` | Keine des Namens eingereiht |

`QueuedSnapshot::decode::<M>()` deserialisiert den Payload zurück
in das konkrete `M`, sodass typgeprüfte Prädikate ohne
maßgeschneidertes Decode-Boilerplate funktionieren.

## Ereignisse: `MessageSending` und `MessageSent`

Jeder erfolgreiche Dispatch feuert zwei Framework-Events:

- `MessageSending` - unmittelbar VOR dem Transport-Aufruf. Listener
  beobachten die Form der Nachricht (Empfänger, Betreff, Tags,
  Body-Form-Flags).
- `MessageSent` - unmittelbar NACH einem erfolgreichen
  Transport-Aufruf. Listener beobachten dieselbe Form; fehlschlagende
  Sendungen feuern dieses Event nicht.

```rust
use std::sync::Arc;
use suprnova::events::EventFacade;
use suprnova::mail::MessageSent;

EventFacade::listen::<MessageSent, _>(Arc::new(MyAuditListener)).await;
```

Beide Events sind rein beobachtend - der Dispatcher modelliert
keinen Laravel-artigen Abbruchkanal. Siehe [Warum Suprnova
abweicht](#warum-suprnova-abweicht) oben für den Umgehungsweg
zum Unterbinden.

## Komfort für mehrere Empfänger: `Mail::cc` und `Mail::bcc`

Die Mail-Facade legt drei Einstiegspunkte offen - `to`, `cc`, `bcc`
-, die alle einen frischen `MailBuilder` zurückgeben. Verwenden Sie
den, der zur vorherrschenden Routing-Absicht passt:

```rust
// Mit einem cc / bcc beginnen, wenn die Nachricht primär eine Audit-Kopie ist.
Mail::cc("manager@example.com")
    .to("alice@example.org")
    .send(OrderShipped { /* ... */ })
    .await?;
```

Dieselbe fluent Oberfläche gilt unabhängig davon, mit welchem
Einstiegspunkt Sie beginnen.

### Gegen `Mail::fake()` testen, nicht gegen den gebundenen Transport

`Mail::fake()` installiert für die Dauer des RAII-Guards einen
prozesslokalen Erfassungs-Transport und stellt wieder her, was
zuvor gebunden war. Tests, die es verwenden, müssen bei jedem
Ein-/Austritt keine Globals löschen - Drop-Semantik erledigt das.
Kombinieren Sie `#[serial_test::serial]` mit `Mail::fake()` für
Tests, die das Transport-Global mutieren; gleichzeitige Tests
würden sich sonst gegenseitig überschreiben.

## Nächste Schritte

- [Benachrichtigungen](notifications.md) - `Notify::send`
  fächert über Mail-, Datenbank- und Webpush-Kanäle auf;
  `#[derive(NotificationMailable)]` ist die makrogetriebene
  Abkürzung über dem `Mailable`-Trait
- [Warteschlange](queues.md) - die dauerhafte Envelope, auf der
  `Mail::queue` und `Mail::later` reiten
- [Ereignisse](events.md) - auf `MessageSending` / `MessageSent`
  lauschen, plus das breitere Dispatcher-Modell
- [Testen](testing.md) - `Mail::fake()` neben den anderen
  `*::fake()`-Guards
- [Konfiguration](configuration.md) - typisierte
  Konfigurationsregistrierung für Service-Credentials

## Referenz

- Trait: `suprnova::mail::Mailable`
- Facade: `suprnova::mail::Mail`
- Bootstrap: `suprnova::mail::boot::bootstrap_from_env()`
- Transporte: `LogMailTransport`, `InMemoryMailTransport`, `SmtpMailTransport`, `PostmarkMailTransport`, `SesMailTransport`, `SendGridMailTransport`, `MailgunMailTransport`, `ResendMailTransport`
- Queue-Job: `suprnova::mail::SendMailJob`
- Test-Guard: `suprnova::mail::MailFake`
- Telemetrie-Helfer: `suprnova::mail::dispatch_with_telemetry`
