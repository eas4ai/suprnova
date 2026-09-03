# Web Push

Web Push liefert eine kurze Nachricht an einen Browser, selbst wenn
Ihre Seite geschlossen ist - der Service Worker wacht auf,
entschlüsselt den Payload und zeigt eine Benachrichtigung auf
Betriebssystem-Ebene. Suprnova liefert das Protokoll end-to-end:
VAPID-Schlüsselerzeugung, AES128GCM-Payload-Verschlüsselung, den
HTTP-Transport und einen `WebPushChannel`, der sich in das
Benachrichtigungs-Subsystem einklinkt, sodass dieselbe `Notification`,
die Sie an Mail oder Datenbank senden, auch als Push landet.

Greifen Sie dazu, wenn Sie Nutzer in Echtzeit ohne offenen WebSocket
alarmieren wollen - Bestellung versandt, Freundschaftsanfrage,
Erwähnung, Kontostand gebucht. Ist der Nutzer auf einem
Desktop-Browser mit geschlossener Seite, ist Web Push der einzige
Mechanismus, der ihn erreicht; ist er auf der Seite, ist
[Broadcasting](broadcasting.md) meist die bessere Wahl.

Die API steht hinter dem Cargo-Feature `web-push`, das standardmäßig
aktiviert ist. Anwendungen, die `default-features = false` verwenden,
müssen `web-push` explizit aktivieren.

## Die vier Teile

Web Push hat mehr bewegliche Teile als Mail oder Datenbank, weil die
Spec ([RFC 8030](https://datatracker.ietf.org/doc/html/rfc8030) +
[RFC 8291](https://datatracker.ietf.org/doc/html/rfc8291) +
[RFC 8292](https://datatracker.ietf.org/doc/html/rfc8292)) Identität,
Verschlüsselung und Transport über drei Verträge hinweg aufteilt:

| Teil | Was es ist |
|---|---|
| `VapidKey` / `VapidSigner` | Ein P-256-ECDSA-Schlüsselpaar, mit dem JWTs signiert werden, die beweisen, dass Ihr Server der ist, der er zu sein behauptet |
| `WebPushClient` | Der HTTP-Client, der einen Payload verschlüsselt, ein VAPID-JWT signiert und an den Endpunkt des Abonnements POSTet |
| `WebPushChannel` | Der Adapter des Benachrichtigungs-Subsystems, der eine `Notification` in einen `WebPushClient::send`-Aufruf verwandelt |
| `SubscriptionInfo` | Das undurchsichtige (`endpoint`, `p256dh`, `auth`)-Tripel, das Ihnen der Browser übergibt, wenn ein Nutzer Push abonniert - Sie speichern es; Sie erzeugen es nicht |

Die unteren drei Schichten - `VapidKey`, `WebPushClient`, das
verschlüsselte POST - werden aus `suprnova::web_push` re-exportiert,
sodass Anwendungen nie direkt von der zugrunde liegenden Crate
`suprnova-web-push` abhängen müssen.

## Ein VAPID-Schlüsselpaar erzeugen

Web Push verwendet VAPID (Voluntary Application Server
Identification), damit Push-Services Sender, die sich schlecht
verhalten, ratenbegrenzen und kontaktieren können. Sie brauchen ein
P-256-Schlüsselpaar pro Anwendung; der öffentliche Schlüssel kommt in
Ihr Frontend, damit der Browser Abonnements an Ihren Server pinnen
kann, und der private Schlüssel bleibt auf dem Server und signiert
JWTs.

Erzeugen Sie eines einmal, persistieren Sie es, und verwenden Sie es
für immer weiter:

```rust
use suprnova::VapidKey;

let key = VapidKey::generate();

// Speichern Sie das PEM irgendwo dauerhaft - einen Secrets-Manager,
// eine Datei, die die Deploy-Pipeline mountet, ein
// Env-Vars-als-Dateien-Volume. Sie können dies NICHT neu erzeugen,
// ohne jedes bestehende Abonnement zu invalidieren.
let pem = key.to_pem()?;
std::fs::write("vapid_private.pem", &pem)?;

// Das Frontend braucht den unkomprimierten öffentlichen Schlüssel im
// Format base64url ohne Padding. Geben Sie das an Ihr JS weiter,
// damit `pushManager.subscribe()` es als `applicationServerKey`
// verwenden kann.
println!("PUBLIC_VAPID_KEY={}", key.public_key_uncompressed_b64url());
```

Laden Sie beim Boot das gespeicherte PEM:

```rust
use suprnova::{VapidKey, VapidSigner};

let pem = std::fs::read_to_string("vapid_private.pem")?;
let key = VapidKey::from_pem(&pem)?;
let signer = VapidSigner::new(key);
```

Ein `VapidSigner` erzeugt JWTs, sendet aber nichts - er ist rein eine
Signierprimitive. Die nächste Schicht umschließt ihn.

## Einen WebPushClient bauen

`WebPushClient` ist die Primitive auf der HTTP-Seite: Geben Sie ihm
einen Signer und eine Kontakt-URI ("wie der Push-Service Sie
erreichen kann, wenn Sie sich schlecht verhalten"), und Sie bekommen
ein Objekt zurück, dessen `send`-Methode einen Payload verschlüsselt,
ein JWT signiert und an den Endpunkt des Abonnements POSTet.

```rust
use std::sync::Arc;
use suprnova::{VapidKey, VapidSigner, WebPushClient};

let signer = VapidSigner::new(VapidKey::from_pem(&pem)?);

// Das Subject MUSS gemäß RFC 8292 §2.1 eine mailto:-URI oder eine
// https:-URL sein. Alles andere wird bei der Konstruktion
// zurückgewiesen, sodass ein fehlkonfiguriertes Deploy beim Boot
// schnell scheitert - nicht still nach dem ersten fehlgeschlagenen
// Dispatch.
let client = WebPushClient::new(signer, "mailto:ops@example.org")?;

let client = Arc::new(client);
```

Warum `Arc<WebPushClient>`? `WebPushClient` umschließt einen
`VapidSigner`, der ein privates `ES256KeyPair` umschließt. Keines
davon ist `Clone` - private Schlüssel sollten nicht beiläufig
dupliziert werden -, und für jede Kanal-Registrierung einen frischen
Signer zu konstruieren würde N unabhängige VAPID-Identitäten für
dieselbe Anwendung bedeuten. Es in ein `Arc` zu hüllen, erlaubt es,
dass eine einzige signierte Identität hinter jeder Registrierung und
jeder gleichzeitigen Zustellung steht.

### Endpunkt-Policy

Endpunkte von Abonnements sind von Nutzern abgeleitete Daten: Der
Browser empfängt die URL von einem entfernten Push-Service, wenn ein
Nutzer Push abonniert, und Ihr Server speichert, was auch immer der
Browser zurückgegeben hat. Ein böswillig gespeichertes Abonnement
kann das HTTP-POST auf alles Erreichbare zeigen lassen und den
Push-Sender in ein Werkzeug für SSRF-Angriffe verwandeln.

`WebPushClient` defaultet auf `EndpointPolicy::Strict`:

- Das Schema muss `https` sein
- Der Host muss eine benannte Domain sein, kein IP-Literal
- Cloud-Metadaten-Hostnamen und von RFC 2606 reservierte TLDs
  (`.localhost`, `.local`, `.internal`, `.test`, `.example`,
  `.invalid`) werden zurückgewiesen

Das blockiert die offensichtlichen SSRF-Sondierungen, ohne echte
Push-Services zu brechen (FCM, Mozilla Autopush, Apples
`web.push.apple.com`).

Für lokale Integrationstests gegen einen `wiremock`-Mock-Server müssen
Sie das abschalten:

```rust
use suprnova::{EndpointPolicy, WebPushClient};

let client = WebPushClient::new(signer, "mailto:test@example.org")?
    .with_endpoint_policy(EndpointPolicy::AllowAny);
```

Verwenden Sie `AllowAny` nicht in Production. Die strikten Prüfungen
existieren, damit eine manipulierte Abonnements-Tabelle nicht
missbraucht werden kann.

### Eigener Transport

`WebPushClient::new` wendet einen 30-Sekunden-Timeout pro Anfrage an.
Wenn Sie eine andere Transport-Policy brauchen - Firmen-Proxy,
gepinntes TLS, kürzerer Timeout - übergeben Sie einen
`reqwest::ClientBuilder` an `WebPushClient::with_client_builder`.
Jede Builder-Option wird übernommen, aber die Redirect-Policy wird
zwangsweise deaktiviert: Ein validierter Endpunkt, der mit 3xx
antwortet, darf den POST nicht an eine nicht validierte URL
weiterleiten, deshalb übernimmt die Library die
Redirect-Einstellung des Aufrufers nicht.

```rust
use reqwest::Client;
use std::time::Duration;
use suprnova::WebPushClient;

let client = WebPushClient::with_client_builder(
    Client::builder().timeout(Duration::from_secs(10)),
    signer,
    "mailto:ops@example.org",
)?;
```

`WebPushClient::with_client` nimmt einen bereits gebauten Client
entgegen, dessen Redirect-Policy die Library nicht prüfen kann.
Sends unter der Default-`Strict`-Policy werden für einen solchen
Transport noch vor jeder I/O abgelehnt - wechseln Sie zu
`with_client_builder`, oder akzeptieren Sie das Risiko explizit mit
`.allow_unconfined_redirects()`, wenn bekannt ist, dass der Client
keinen Redirects folgt.

## WebPushChannel in die Benachrichtigungen verdrahten

Das rohe `WebPushClient::send` funktioniert - aber der Weg, auf dem
Sie in Suprnova tatsächlich Push-Benachrichtigungen senden, führt über
das Subsystem [Benachrichtigungen](notifications.md). Eine
`Notification` deklariert `vec!["webpush"]` in ihrem `channels()`, ein
`Notifiable`-Empfänger gibt ein JSON-kodiertes `SubscriptionInfo` von
`route_for("webpush")` zurück, und der gebundene
`NotificationDispatcher` übernimmt den Fan-out.

```rust
use std::sync::Arc;
use suprnova::{
    NotificationDispatcher, WebPushChannel, WebPushClient,
    notifications::set_dispatcher,
};

let client: Arc<WebPushClient> = Arc::new(
    WebPushClient::new(signer, "mailto:ops@example.org")?
);

// ttl_secs: wie lange der Push-Service eine unzugestellte Nachricht
// vorhält. 86_400 (24h) ist ein vernünftiger Default für
// nicht-dringende Benachrichtigungen; senken Sie auf 60 für
// "jetzt sofort handeln"-Alarme, bei denen eine veraltete Nachricht
// schlimmer ist als keine Nachricht.
let webpush = Arc::new(WebPushChannel::new(client, 86_400));

let dispatcher = NotificationDispatcher::new()
    .register_channel(webpush);

set_dispatcher(Arc::new(dispatcher))?;
```

`register_channel` ist last-write-wins auf dem `name()` des Kanals,
sodass Tests einen Stub einsetzen können, ohne die
Production-Bindung zu beeinflussen.

## Eine Benachrichtigung definieren

Eine push-gebundene Benachrichtigung hat dieselbe Form wie jede andere
Suprnova-Benachrichtigung - deklarieren Sie `"webpush"` in
`channels()` und legen Sie, was auch immer an JSON zugestellt werden
soll, in `data()`:

```rust
use serde::{Deserialize, Serialize};
use suprnova::Notification;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OrderShipped {
    pub order_id: i64,
    pub tracking_url: String,
}

impl Notification for OrderShipped {
    fn notification_name() -> &'static str {
        "OrderShipped"
    }

    fn channels(&self) -> Vec<&'static str> {
        vec!["webpush"]
    }

    fn data(&self) -> serde_json::Value {
        serde_json::json!({
            "title":   "Your order has shipped",
            "body":    format!("Track order #{}", self.order_id),
            "url":     self.tracking_url,
        })
    }
}
```

Das JSON aus `data()` ist das, was Ihr Service Worker empfängt.
Wählen Sie eine stabile Form und dokumentieren Sie sie für das
Frontend - Suprnova gibt keine vor, weil die Benachrichtigungs-UI
Sache des Frontends ist.

## Den Empfänger routen

Ein `Notifiable` gibt für jeden Kanal, den er unterstützt, die Route
zurück. Für Web Push ist diese Route das JSON-kodierte
`SubscriptionInfo` - genau das, was der Browser über
`PushSubscription.toJSON()` erzeugt hat, wortgetreu gespeichert:

```rust
use suprnova::Notifiable;

pub struct User {
    pub id: i64,
    pub push_subscription_json: Option<String>,
}

impl Notifiable for User {
    fn route_for(&self, channel: &str) -> Option<String> {
        match channel {
            "webpush" => self.push_subscription_json.clone(),
            _ => None,
        }
    }
}
```

`None` zurückzugeben lässt den Dispatcher den Kanal still
überspringen - nützlich für Nutzer, die Push nicht abonniert haben,
aber trotzdem E-Mails bekommen.

## Es senden

Synchron:

```rust
use suprnova::Notify;

let user = User::find(42).await?.unwrap();
Notify::send(&user, &OrderShipped {
    order_id: 1234,
    tracking_url: "https://ship.example.org/o/1234".into(),
}).await?;
```

Queued - löst die Route des Abonnements bereits zum Zeitpunkt des
Einreihens auf, sodass der Worker den Nutzer nicht erneut laden muss:

```rust
Notify::queue(&user, OrderShipped {
    order_id: 1234,
    tracking_url: "https://ship.example.org/o/1234".into(),
}).await?;
```

Damit `Notify::queue` funktioniert, registrieren Sie beim Boot die
Factory der Benachrichtigung, damit der Worker den JSON-Payload zurück
in die typisierte Benachrichtigung bauen kann:

```rust
suprnova::notifications::register_notification_factory::<OrderShipped>()?;
suprnova::queue::worker::register_job::<suprnova::SendNotificationJob>();
```

Hinter den Kulissen baut der Queued-Dispatch einen
`SendNotificationJob`, der `(notification_name, payload,
per_channel_routes, channels)` trägt. Der Worker rehydriert die
Benachrichtigung, schlägt `WebPushChannel` auf dem gebundenen
Dispatcher über den Namen nach und ruft `deliver(route, &notification)`
auf - derselbe Codepfad wie das synchrone `Notify::send`.

## Die Browser-Seite

Suprnova liefert kein JavaScript-SDK - die Browser-Seite ist reine
Web-Push-API. Der Ablauf, den Ihr Frontend implementieren muss:

1. Einen Service Worker registrieren.
2. Den Nutzer um Erlaubnis bitten.
3. Über `pushManager.subscribe({ userVisibleOnly: true,
   applicationServerKey: <Ihr öffentlicher VAPID-Schlüssel> })`
   abonnieren.
4. `subscription.toJSON()` an einen Suprnova-Endpunkt POSTen, der es
   auf der Nutzerzeile speichert.

```js
// Service-Worker-Registrierung (irgendwo im Entrypoint Ihrer App)
const registration = await navigator.serviceWorker.register('/sw.js');

if (Notification.permission === 'default') {
    await Notification.requestPermission();
}

if (Notification.permission === 'granted') {
    const subscription = await registration.pushManager.subscribe({
        userVisibleOnly: true,
        applicationServerKey: window.PUBLIC_VAPID_KEY,
    });

    await fetch('/api/push/subscribe', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(subscription.toJSON()),
    });
}
```

Ihr Suprnova-Endpunkt empfängt das JSON, validiert die Form und
speichert es beim Nutzer - die Zeichenkette ist für Ihren Server
undurchsichtig, muss aber genau das JSON sein, das der Browser
erzeugt hat (der Typ `SubscriptionInfo` verwendet `Deserialize`, um es
später zu parsen):

```rust
use suprnova::{Auth, Request, Response, SubscriptionInfo, attrs, json_response};

pub async fn subscribe(req: Request) -> Response {
    let user_id = Auth::id().expect("auth middleware");

    let (_parts, bytes) = match req.body_bytes().await {
        Ok(b) => b,
        Err(e) => return json_response!({ "error": e.to_string() }).map(|r| r.status(400)),
    };
    let raw = match std::str::from_utf8(&bytes) {
        Ok(s) => s.to_string(),
        Err(_) => return json_response!({ "error": "body not utf-8" }).map(|r| r.status(400)),
    };

    // Parsen, um die Form zu validieren - Endpunkt, keys.p256dh,
    // keys.auth. Schlägt das Parsen fehl, hat der Browser uns etwas
    // Fehlerhaftes übergeben.
    let sub: SubscriptionInfo = match serde_json::from_str(&raw) {
        Ok(s) => s,
        Err(e) => return json_response!({ "error": e.to_string() }).map(|r| r.status(400)),
    };

    // `raw` wortgetreu persistieren - das ist genau die Zeichenkette,
    // die WebPushChannel beim Dispatch an serde_json::from_str
    // übergeben wird.
    User::query()
        .db_where_op("id", "=", user_id)
        .update_all(attrs! { push_subscription_json: raw })
        .await
        .unwrap();

    json_response!({ "ok": true, "endpoint": sub.endpoint })
}
```

Der Service Worker entschlüsselt den Push-Payload und rendert die
Benachrichtigung:

```js
// /sw.js
self.addEventListener('push', (event) => {
    const data = event.data.json();
    event.waitUntil(
        self.registration.showNotification(data.title, {
            body: data.body,
            data: { url: data.url },
        }),
    );
});

self.addEventListener('notificationclick', (event) => {
    event.notification.close();
    event.waitUntil(clients.openWindow(event.notification.data.url));
});
```

## Payload-Grenzen

Die Web-Push-Spec begrenzt jeden verschlüsselten Payload auf insgesamt
4096 Bytes. Suprnova weist Klartexte größer als 3992 Bytes (die
Grenze minus dem ~85-Byte-Overhead der AES128GCM-Verschlüsselung)
schon zur Verschlüsselungszeit zurück, sodass der Fehlschlag in Ihrem
Code auftaucht, nicht als 413 vom Push-Service. Eine `Notification`,
deren serialisiertes `data()` diese Grenze überschreitet, gibt aus dem
`deliver` des Kanals `WebPushError::Encryption` zurück.

Für alles Größere - einen langen Nachrichtentext, ein Thumbnail -
senden Sie eine kurze Benachrichtigung, die eine URL trägt, die der
Service Worker beim Klick abruft. Das ist sowohl schneller (keine
Verschlüsselung auf einem Multi-KB-Payload) als auch flexibler (der
Fetch kann zurückgeben, welche Form Sie wollen).

## Tote Abonnements

Wenn der Push-Service 404 oder 410 zurückgibt, ist das Abonnement tot -
der Nutzer hat den Browser deinstalliert, die Erlaubnis zurückgezogen
oder den Speicher gelöscht. `WebPushChannel` behandelt das als
nicht-fatale Warnung:

```text
WARN webpush subscription gone (404/410); caller should remove
     channel=webpush endpoint=https://fcm.googleapis.com/fcm/send/abc
```

Der Dispatch gibt `Ok(())` zurück, weil die Benachrichtigung einen
Endzustand erreicht hat - es gibt keinen Empfänger, gegen den erneut
versucht werden könnte. Von Ihrer Anwendung wird erwartet, dass sie
auf die Warnung reagiert: `endpoint` aus dem Log parsen (oder einen
`NotificationFailed`-Listener einhängen, der über `WebPushError`
klassifiziert) und die Abonnement-Zeile entfernen. Suprnova liefert
die Warnung; es bereinigt die Abonnements-Tabelle nicht automatisch
für Sie.

## Retries und Retry-After

Wenn der Push-Service ein vorübergehendes 5xx, 408 oder 429
zurückgibt, trägt der zugrunde liegende
`WebPushError::PushServiceRejected` den geparsten `Retry-After`-Hinweis
(nur die Delta-Sekunden-Form - die HTTP-Date-Form gibt `None`
zurück):

```rust
use suprnova::WebPushError;

match client.send(&sub, payload, ContentEncoding::Aes128Gcm, 60).await {
    Ok(_) => (),
    Err(e) if e.is_retryable() => {
        let wait = e.retry_after().unwrap_or(Duration::from_secs(30));
        tokio::time::sleep(wait).await;
        // ...erneut versuchen, oder mit einer Verzögerung zurück in
        // die Queue schieben
    }
    Err(WebPushError::SubscriptionGone) => {
        // das Abonnement entfernen
    }
    Err(e) => return Err(e.into()),
}
```

Der `Retry-After`-Hinweis ist auf 24 Stunden begrenzt, damit ein
feindlicher Server keinen Worker in einen mehrjährigen Schlaf parken
kann.

Wenn Sie `Notify::queue` verwenden, greift der eigene
Retry-/Backoff-Mechanismus der Queue - ein `WebPushError`, der aus
`WebPushChannel::deliver` propagiert, taucht als Job-Fehler auf, und
die Envelope handhabt das erneute Einreihen gemäß der Backoff-Policy
des Jobs. Der `Retry-After`-Hinweis wird protokolliert, aber (noch)
nicht in die Verzögerungsberechnung der Queue zurückgespeist; falls
Sie das brauchen, hängen Sie einen `NotificationFailed`-Listener ein,
der mit der angedeuteten Verzögerung erneut einreiht.

## Telemetrie

Der Benachrichtigungs-Dispatcher umschließt den Fan-out in einem
`notification.dispatch`-Info-Span, getaggt mit dem Namen der
Benachrichtigung und der Kanalanzahl. Jede erfolgreiche Zustellung
emittiert ein `NotificationSent`-Event; Fehlschläge emittieren
`NotificationFailed`, das Kanalname, Route und Fehlerstring trägt.
Verdrahten Sie jedes davon in Ihre Metrik-/Log-Pipeline, genauso wie
Sie andere Framework-Events verdrahten - siehe
[Ereignisse](events.md).

Ein totes Abonnement emittiert ein strukturiertes WARN mit
`channel="webpush"`, dem Endpunkt und dem Namen der Benachrichtigung.
Das ist das Signal, nach dem ein automatisierter Aufräum-Job für
Abonnements scrapen sollte.

### Warum Suprnova abweicht

Laravels `WebPush`-Treiber ist ein Community-Paket
(`laravel-notification-channels/webpush`) - nicht im Core, separat
versioniert, ORM-lastig in seinen Annahmen. Suprnova integriert Web
Push fest in das Framework, weil das Protokoll wohldefiniert ist und
das verschlüsselte HTTP-POST ein zu kleiner Vertrag ist, um ihn in
eine Drittanbieter-Abstraktion zu hüllen. Das
Benachrichtigungs-Subsystem hält die Oberfläche einheitlich: dieselbe
`Notification`, die Sie an Mail oder Datenbank senden, landet auch als
Push, keine Treiber-Matrix, kein separater Config-Baum.

Wir legen außerdem standardmäßig die strikte Endpunkt-Policy frei.
Das Laravel-Community-Paket überlässt den SSRF-Schutz der Anwendung;
wir vertreten die Position, dass "der Endpunkt kam aus Nutzerdaten"
die Form jedes Web-Push-Abonnements ist, und dass der sichere Default
ins Framework gehört, nicht in Ihren Code.

Die Retry-Klassifikation (`is_retryable`, `retry_after`) wird als
typisierte Methoden auf `WebPushError` freigelegt statt als magische
Konstanten-Tabelle in der Queue-Schicht. Die Queue besitzt weiterhin
die Retry-Policy - der Fehler sagt Ihnen, ob eine Wiederholung
erfolgreich sein könnte und wie lange zu warten ist; die Queue entscheidet, ob und
wann erneut entnommen wird. Die beiden zu trennen bedeutet, dass Ihre
eigenen Retry-Strategien (exponentieller Backoff, gejittert,
gedeckelt) Web Push nicht als Sonderfall behandeln müssen.

## Testen

Stellen Sie einen `wiremock`-Server auf, richten Sie einen
`WebPushClient` mit `EndpointPolicy::AllowAny` darauf aus, und
asserten Sie auf die Anfragen, die er empfängt:

```rust
use std::sync::Arc;
use suprnova::{
    EndpointPolicy, NotificationDispatcher, Notify, VapidKey, VapidSigner,
    WebPushChannel, WebPushClient,
    notifications::set_dispatcher,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn order_shipped_pushes() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/push"))
        .respond_with(ResponseTemplate::new(201))
        .mount(&server)
        .await;

    let signer = VapidSigner::new(VapidKey::generate());
    let client = Arc::new(
        WebPushClient::new(signer, "mailto:test@example.org")
            .unwrap()
            .with_endpoint_policy(EndpointPolicy::AllowAny),
    );
    let channel = Arc::new(WebPushChannel::new(client, 60));

    let dispatcher = NotificationDispatcher::new().register_channel(channel);
    set_dispatcher(Arc::new(dispatcher)).unwrap();

    let user = test_user_with_subscription(&server.uri()).await;
    Notify::send(&user, &OrderShipped {
        order_id: 1,
        tracking_url: "https://ship.example.org/o/1".into(),
    }).await.unwrap();
    // server.received_requests() enthält jetzt das verschlüsselte POST.
}
```

Für End-to-End-Tests, denen die verschlüsselten Bytes egal sind,
erfasst `Notify::fake()` (behandelt in
[Benachrichtigungen](notifications.md)) den Dispatch, ohne den Kanal
laufen zu lassen - schneller, kein Mock-Server, kein
Verschlüsselungs-Roundtrip.

## Referenz

- Primitives: `suprnova::VapidKey`, `suprnova::VapidSigner`,
  `suprnova::VapidClaims`
- Client: `suprnova::WebPushClient`, `suprnova::EndpointPolicy`,
  `suprnova::PushResponse`, `suprnova::SubscriptionInfo`
- Fehler: `suprnova::WebPushError` - `.is_retryable()`,
  `.retry_after()`, `WebPushError::SubscriptionGone`
- Encoding: `suprnova::ContentEncoding` (Aes128Gcm; 3992-Byte-Klartext-Obergrenze)
- Kanal: `suprnova::WebPushChannel`
- Facade: `suprnova::Notify`
- Queue-Job: `suprnova::SendNotificationJob`
- Factory-Registrierung:
  `suprnova::notifications::register_notification_factory`

## Nächste Schritte

- [Benachrichtigungen](notifications.md) - der Multi-Kanal-Dispatcher,
  in den sich `WebPushChannel` einklinkt
- [Mail](mail.md) - das E-Mail-Kanal-Gegenstück für Nutzer ohne Push
- [Broadcasting](broadcasting.md) - Echtzeit-Zustellung für Nutzer, die
  auf der Seite sind
- [Warteschlange](queues.md) - wie `Notify::queue` hinter
  `SendNotificationJob` steht
- [Ereignisse](events.md) - auf `NotificationSent` /
  `NotificationFailed` lauschen, um die Bereinigung toter Abonnements
  anzustoßen
