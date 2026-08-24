# HTTP-Client

Die `Http`-Facade ist die ausgehende Seite von HTTP - das
Rust-Äquivalent zu Laravels `Http::`-Helfer. Sie greifen dazu, wenn
Ihr Handler, Job oder geplanter Task die API eines anderen aufrufen
muss: ein Payment-Gateway, ein Geocoder, ein Webhook-Ziel, eine
Slack-Nachricht. Fluent-Builder, JSON hinein und heraus,
Wiederholungen mit Jitter, deterministische Test-Fakes, die
aufzeichnen, was Sie gesendet haben. Dieselbe Oberfläche, die Sie in
Laravel verwendet haben, mit task-lokaler Isolation, sodass parallele
Tests nicht die Fakes der jeweils anderen sehen.

```rust
use suprnova::Http;
use serde_json::json;

let resp = Http::post("https://api.stripe.com/v1/charges")
    .bearer_token(secret_key)
    .json(&json!({ "amount": 1000, "currency": "usd" }))
    .send()
    .await?;

let body: serde_json::Value = resp.json().await?;
```

Das ist die Form: `Http::<verb>(url)` liefert einen
`RequestBuilder`; Sie verketten Konfiguration daran; `.send().await`
liefert eine `ClientResponse`. Der zugrunde liegende Client ist ein
geteilter `reqwest::Client` mit rustls-TLS, einem
30s-Standard-Timeout und einem `suprnova/<version>`-User-Agent -
lazy gebaut beim ersten Aufruf.

## Die Verben

```rust
Http::get("https://api.example.com/users/42")
Http::post("https://api.example.com/users")
Http::put("https://api.example.com/users/42")
Http::patch("https://api.example.com/users/42")
Http::delete("https://api.example.com/users/42")
```

Jedes Verb liefert einen `RequestBuilder`. Die URL kann jedes
`impl Into<String>` sein - ein `&str`, ein `String` oder ein
`Cow<str>`. Die Facade liefert keine URL-Bau-Helfer; formatieren Sie
die URL selbst oder greifen Sie zu einer Query-String-Crate.

## Bodys

Drei Wege, einen Body anzuhängen. Jeder ersetzt einen zuvor gesetzten
Body.

### JSON

```rust
use serde::Serialize;

#[derive(Serialize)]
struct CreateUser {
    name: String,
    email: String,
}

Http::post("https://api.example.com/users")
    .json(&CreateUser {
        name: "Ada".into(),
        email: "ada@example.com".into(),
    })
    .send()
    .await?;
```

`.json(&value)` akzeptiert alles, was `serde::Serialize`
implementiert. Der `Content-Type` auf der Wire-Ebene wird
automatisch auf `application/json` gesetzt. Schlägt die
Serialisierung fehl (z. B. eine Map mit einem Nicht-String-Schlüssel),
zeichnet der Builder den Fehler auf, und `send()` bringt ihn an die
Oberfläche, statt stillschweigend einen `null`-Body zu senden.

### Form

```rust
Http::post("https://login.example.com/oauth/token")
    .form(&serde_json::json!({
        "grant_type": "client_credentials",
        "client_id": id,
        "client_secret": secret,
    }))
    .send()
    .await?;
```

`.form(&value)` serialisiert den Wert als
`application/x-www-form-urlencoded`. Der Wert muss zu einem
JSON-Objekt serialisieren; die Schlüssel werden zu Formularfeldern.
Dieselbe Body-Fehler-Semantik wie bei `.json` - ein
Serialisierungsfehlschlag kommt über `send().await?` an die
Oberfläche, niemals als stiller leerer Body.

### Rohe Bytes

```rust
use bytes::Bytes;

let payload: Bytes = compress(report)?;
Http::post("https://collector.example.com/ingest")
    .header("Content-Type", "application/octet-stream")
    .body(payload)
    .send()
    .await?;
```

`.body(bytes)` nimmt alles entgegen, was `impl Into<Bytes>` ist. Sie
sind für den `Content-Type`-Header verantwortlich - `.body` setzt
keinen.

## Header und Auth

```rust
Http::get("https://api.example.com/private")
    .header("X-Request-Id", request_id)
    .header("Accept", "application/vnd.api+json")
    .bearer_token(api_key)
    .send()
    .await?;
```

`.header(name, value)` hängt an; das Framework dedupliziert nicht,
zwei Aufrufe mit demselben Namen senden also zwei Header, und
reqwest verbindet sie gemäß der HTTP-Semantik. Zwei Abkürzungen für
die üblichen Auth-Schemata:

- `.bearer_token(token)` - setzt `Authorization: Bearer <token>`
- `.basic_auth(user, password)` - setzt `Authorization: Basic <b64>`;
  `password` ist `Option<&str>`, sodass
  `.basic_auth("api-key", None)` die Form `api-key:` kodiert, die
  manche Provider wollen

## Timeouts

Der geteilte Client hat einen 30-Sekunden-Standard-Timeout.
Überschreiben Sie ihn pro Anfrage, wenn Sie müssen:

```rust
use std::time::Duration;

Http::get("https://slow.example.com/report")
    .timeout(Duration::from_secs(120))
    .send()
    .await?;
```

`.timeout(dur)` überschreibt sowohl den Connect- als auch den
Gesamt-Request-Timeout für diesen einen Aufruf. Es gibt keinen
separaten `connect_timeout`-Regler auf dem Builder; der zugrunde
liegende reqwest-Client verwendet einen kombinierten Timeout.

## Redirects

Der geteilte Client folgt Redirects standardmäßig (bis zu reqwests Obergrenze von
10) - das richtige Verhalten, wenn Sie einen
vertrauenswürdigen Endpunkt aufrufen, der mit `http → https`
antwortet oder Ihnen eine CDN-URL gibt.

Wird die Request-URL von nicht vertrauenswürdiger Eingabe
beeinflusst, wird dieser Standard zu einem
Server-Side-Request-Forgery-Vektor (SSRF): Ein feindlicher Endpunkt
kann mit einem `3xx` antworten, dessen `Location` auf einen internen
Dienst oder eine Cloud-Metadaten-Adresse
(`http://169.254.169.254/…`) zeigt, und ein folgender Client würde
ihr nachjagen. Deaktivieren Sie das Redirect-Folgen für diese
Anfragen mit `.no_redirects()`:

```rust
let resp = Http::get(user_supplied_url)
    .no_redirects()
    .send()
    .await?;

// Das 3xx wird unverändert zurückgegeben, statt gefolgt zu werden -
// prüfen Sie es und lehnen Sie ab, statt den Client dem
// Location-Header nachjagen zu lassen.
if (300..400).contains(&resp.status()) {
    return Err(AppError::bad_request("refusing to follow a redirect"));
}
```

`.no_redirects()` leitet die Anfrage durch einen separaten, nicht
folgenden Client; der Standard-Client - und jede Anfrage, die ihn
nicht aufruft - bleibt unverändert. Das ist das
Allgemein-Client-Analogon zur Redirect-Sperre, die der
Web-Push-Sender bereits auf von Angreifern kontrollierte
Push-Endpunkte anwendet.

## Wiederholungen

`Http` liefert Wiederholungen mit Exponential-Backoff und vollem
Jitter - das AWS-Rezept, dasselbe, das Laravel verwendet. Beide
Wiederholungsmodi behandeln Transportfehler für jede HTTP-Methode. Sie
unterscheiden sich darin, ob eine erhaltene 5xx-Response `POST` und `PATCH`
erneut ausführen darf.

### `.retry(max_attempts, base_backoff)` - Transportwiederholungen für jede Methode

```rust
use std::time::Duration;

let resp = Http::get("https://flaky.example.com/health")
    .retry(4, Duration::from_millis(200))
    .send()
    .await?;
```

`max_attempts` schließt den ersten Versuch ein, `retry(4, ...)`
wiederholt also bis zu dreimal nach dem ersten Versuch. Die
Verzögerung vor Versuch `n+1` ist eine gleichverteilte Zufallsdauer
in `[0, base_backoff * 2^(n-1)]`, gekappt bei 30 Sekunden. Volles
Jitter, nicht Exponential-Backoff-plus-feste-Pause, sodass viele
Worker, die denselben Ausfall wiederholen, nicht zu einer
Thundering Herd synchronisieren.

`.retry()` wiederholt Transportfehler für jede Methode. Trifft eine Response
ein, wiederholt es einen 5xx-Status, außer die Methode ist `POST` oder
`PATCH`. Es gibt 4xx- und 2xx/3xx-Responses unverändert zurück. Nach dem
Erschöpfen der Wiederholungen wird die letzte Response oder der letzte
Transportfehler an den Aufrufer zurückgegeben.

Diese Unterscheidung ist bei Schreibvorgängen wichtig. Ein `POST`- oder
`PATCH`-Transportfehler kann bedeuten, dass der Server den Schreibvorgang
committet hat, aber die Response verloren ging; der aktuelle Vertrag
wiederholt diesen Fehler trotzdem. Eine erhaltene 5xx-Response für diese
Methoden wird nach einem Versuch zurückgegeben, sofern der Aufrufer nicht
`.retry_non_idempotent(...)` verwendet.

### `.retry_non_idempotent(...)` - Opt-in für POST/PATCH

```rust
Http::post("https://api.example.com/charges")
    .header("Idempotency-Key", idem_key)
    .retry_non_idempotent(3, Duration::from_millis(200))
    .send()
    .await?;
```

Haben Sie einen Idempotency-Key mitgeliefert, den das vorgelagerte System
respektiert, oder die Anfrage sonst sicher für ein Replay gemacht, wechseln
Sie zu `.retry_non_idempotent(...)`. Es bewahrt die
Transportfehler-Wiederholungen für jede Methode und erlaubt zusätzlich
5xx-Response-Wiederholungen für `POST` und `PATCH`. 4xx- und
2xx/3xx-Responses werden weiterhin unverändert zurückgegeben.

### Retry-After wird bei 503 respektiert

Bei einem `503 Service Unavailable` respektiert das Framework einen
`Retry-After`-Header - entweder in Delta-Sekunden-Form
(`Retry-After: 30`) oder als HTTP-Datum
(`Retry-After: Tue, 15 Nov 1994 08:12:31 GMT`). Die tatsächliche
Wartezeit ist die größere von gejittertem Backoff und dem
`Retry-After`-Hinweis, weiterhin gekappt bei 30 Sekunden. Ein
feindlicher oder falsch konfigurierter Server, der
`Retry-After: 86400` zurückgibt, parkt Ihre Task nicht für einen
ganzen Tag.

### `.retry_when(predicate)` – die Richtlinie weiter einschränken

```rust
use std::time::Duration;

let resp = Http::get("https://flaky.example.com/health")
    .retry(4, Duration::from_millis(200))
    .retry_when(|ctx| ctx.method == "GET")
    .send()
    .await?;
```

`retry_when` registriert ein Prädikat, das vor jeder Wiederholung abgefragt
wird, die die obige Richtlinie sonst ausführen würde. Es kann eine sonst
zulässige Wiederholung verhindern, aber keine erzeugen. Insbesondere kann es
weder eine 2xx-, 3xx- oder 4xx-Response in eine Wiederholung verwandeln noch
eine erhaltene 5xx-Response für `POST` oder `PATCH` ohne
`.retry_non_idempotent(...)` wiederholbar machen. Es wird vor
Transportfehler-Wiederholungen für jede Methode abgefragt, einschließlich mit
plain `.retry()` konfigurierter `POST` und `PATCH`. Ohne eine Richtlinie
`.retry(...)` oder `.retry_non_idempotent(...)` hat ein alleinstehendes
`retry_when` nichts zu verhindern.

Das Prädikat erhält `RetryContext { attempt, method, url, outcome }`, wobei
`outcome` entweder `RetryOutcome::TransportError` (Senden schlug fehl, bevor
eine Response eintraf) oder `RetryOutcome::Status(n)` (eine zulässige
5xx-Response) ist.

## Die Response lesen

`ClientResponse` legt Status, Header und drei Body-Lesemethoden
offen. Jede Body-Methode verbraucht die Response.

```rust
let resp = Http::get("https://api.example.com/users/42").send().await?;

let status: u16 = resp.status();
let etag: Option<String> = resp.header("ETag");

// Eine wählen - jede verbraucht die Response.
let user: User = resp.json().await?;
// let text: String = resp.text().await?;
// let bytes: Bytes = resp.bytes().await?;
```

`.header(name)` ist case-insensitive. `.json::<T>()` liefert
`Result<T, FrameworkError>` und verwendet `serde_json` zum
Decodieren. `.text()` erzwingt UTF-8 und bringt einen
`FrameworkError` an die Oberfläche, wenn der Body kein gültiges
UTF-8 ist.

### Obergrenze für den Response-Body

Ein langsames oder feindliches vorgelagertes System kann sonst
einen unbegrenzten Body in den Speicher streamen. Um das zu
schützen, ist jeder gepufferte Body-Read gekappt - standardmäßig
25 MiB. Überschreiben Sie global beim Boot:

```rust
use suprnova::Http;

// Einmal, irgendwo im Bootstrap.
Http::set_max_response_bytes(100 * 1024 * 1024); // 100 MiB
```

Oder pro Anfrage, wenn ein Aufruf legitim eine größere Payload
verarbeitet:

```rust
let bytes = Http::get("https://example.com/big-export.json")
    .max_response_bytes(500 * 1024 * 1024) // 500 MiB
    .send()
    .await?
    .bytes()
    .await?;
```

Eine Response, die einen `Content-Length` über der Obergrenze
deklariert, wird abgelehnt, bevor irgendein Body gelesen wird; die
Streaming-Schleife erzwingt die Obergrenze auch gegen die
tatsächlichen Bytes, falls `Content-Length` fehlt oder lügt.

## Notausgang - rohes reqwest

Das Framework deckt die üblichen Fälle ab. Wenn Sie etwas brauchen,
das wir nicht exponieren - streamende Bodys, Multipart-Uploads,
Inspektion der Redirect-Policy, WebSocket-Upgrades -, rufen Sie
`.into_inner()` auf, um die zugrunde liegende `reqwest::Response`
auszupacken:

```rust
let resp = Http::get("https://example.com/big-stream").send().await?;
let raw: reqwest::Response = resp.into_inner()?;
let mut stream = raw.bytes_stream();
while let Some(chunk) = stream.next().await {
    process(chunk?);
}
```

`into_inner()` liefert `Err(FrameworkError::internal(...))`, wenn es
auf einer gefakten Response aufgerufen wird - in diesem Fall gibt es
keine zugrunde liegende `reqwest::Response`. Die
Response-Body-Obergrenze gilt außerdem nicht mehr, sobald Sie die
rohe Response entnehmen; von dort an gehört Ihnen das Lesen.

Für ausgehende Multipart-Uploads greifen Sie heute direkt über
denselben Notausgang zu `reqwest::Client`. Ein künftiges Release
könnte einen `.multipart(...)`-Builder hinzufügen, wenn sich das
Bedarfsmuster herauskristallisiert.

## Testen mit `Http::fake`

Das ist der Teil, den Sie täglich verwenden werden. `Http::fake`
führt Ihren Testkörper innerhalb eines `tokio::task_local!`-Scopes
aus, in dem jeder ausgehende Aufruf abgefangen, erfasst und mit dem
beantwortet wird, was Sie eingereiht haben.

```rust
use suprnova::{Http, fake_response, assert_sent};

#[tokio::test]
async fn creates_a_user_via_api() {
    Http::fake(|| async {
        fake_response(
            "POST",
            "/api/users",
            201,
            serde_json::json!({ "id": 42, "name": "Ada" }),
        );

        let resp = Http::post("https://example.com/api/users")
            .json(&serde_json::json!({ "name": "Ada" }))
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), 201);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["id"], 42);

        assert_sent(|r| r.method == "POST" && r.url.contains("/api/users"));
    })
    .await;
}
```

### Vorgefertigte Responses matchen

`fake_response(method, url_substring, status, body)` reiht eine
vorgefertigte Response ein. Die erste ausgehende Anfrage, deren
Methode passt (case-insensitive) und deren URL `url_substring`
enthält, verbraucht den vorgefertigten Eintrag und liefert diese
Response. Verwenden Sie die Methode `"*"`, um auf jede Methode zu
passen.

Nachfolgende passende Anfragen fallen zum nächsten vorgefertigten
Eintrag derselben Form durch, oder - wenn keiner passt - liefern
ein leeres `200 {}`. Reihen Sie eine vorgefertigte Response pro
erwartetem Aufruf ein:

```rust
fake_response("GET", "/v1/customer", 200, json!({ "id": "cus_1" }));
fake_response("GET", "/v1/customer", 200, json!({ "id": "cus_2" }));
// Zwei GETs an /v1/customer bekommen unterschiedliche Responses; ein drittes bekommt 200 {}.
```

### Assertions

```rust
// Bestanden, wenn mindestens eine aufgezeichnete Anfrage passt.
assert_sent(|r| r.method == "POST" && r.url.contains("/charges"));

// Bestanden, wenn keine aufgezeichnete Anfrage passt.
assert_not_sent(|r| r.url.contains("/refunds"));
```

`RecordedRequest` legt `method: String`, `url: String`,
`headers: Vec<(String, String)>` und `body: Option<Vec<u8>>` offen.
Das Prädikat läuft gegen jede aufgezeichnete Anfrage; bei einem
Assertion-Fehlschlag wird die aufgezeichnete Liste gedruckt, mit
geschwärzten Header-Werten und Bodys (eine kleine Allowlist aus
`Content-Type`, `Accept` und `User-Agent` wird vollständig gezeigt;
alles andere ist `<redacted>`). Das hält Bearer-Tokens und
Webhook-Payloads aus CI-Logs heraus, selbst wenn eine Assertion
explodiert.

### Tests laufen parallel-sicher

Der Fake-Zustand lebt in einem `tokio::task_local!` - jeder
Fake-Scope ist an die Task gescoped, die den Test ausführt, nicht an
den Prozess. Zwei Tests, die gleichzeitig auf unterschiedlichen
Tasks laufen, bekommen jeweils ihren eigenen Recorded-Requests-Vec
und ihre eigene Vorgefertigte-Response-Queue. Kein geteilter Mutex,
keine Test-Reihenfolge, kein `#[serial]`.

```rust
#[tokio::test]
async fn first_test() {
    Http::fake(|| async {
        fake_response("GET", "/a", 200, json!({"who": "first"}));
        let _ = Http::get("https://x.test/a").send().await.unwrap();
        assert_sent(|r| r.url.contains("/a"));
        // Die Anfrage des Geschwister-Tests an /b ist hier unsichtbar.
    })
    .await;
}

#[tokio::test]
async fn second_test() {
    Http::fake(|| async {
        fake_response("GET", "/b", 200, json!({"who": "second"}));
        let _ = Http::get("https://x.test/b").send().await.unwrap();
        assert_sent(|r| r.url.contains("/b"));
    })
    .await;
}
```

## Die Gespawnte-Task-Falle

`tokio::task_local!` ist an die aktuelle Task gescoped. Arbeit, die
durch `tokio::spawn` geht, landet auf einer frischen Task und erbt
den Fake NICHT - standardmäßig treffen ausgehende Aufrufe aus dem
gespawnten Future das echte Netzwerk. Zwei Helfer adressieren das.

### `Http::fail_on_real_calls()` und `FailOnRealCallsGuard`

Kippt ein prozessglobales Flag, das jeden nicht passenden ausgehenden
Aufruf in einen `FrameworkError::internal(...)` verwandelt, statt
ihn das Netzwerk treffen zu lassen. Das ist Suprnovas Analogon zu
Laravels `Http::preventStrayRequests()` - es fängt genau den Bug
ab, den die Falle erzeugt.

Verwenden Sie den RAII-Guard, damit das Flag zurückgesetzt wird,
wenn der Test endet, selbst bei einem Panic:

```rust
use suprnova::FailOnRealCallsGuard;

#[tokio::test]
async fn no_test_makes_a_real_call() {
    let _guard = FailOnRealCallsGuard::install();

    // Jeder ungefakte ausgehende HTTP-Aufruf von irgendwo innerhalb
    // dieses Tests - einschließlich aus einer per `tokio::spawn`
    // gestarteten Task - schlägt mit einem Fehler fehl, der die URL
    // nennt. Es findet tatsächlich keine Netzwerk-IO statt.
}
```

Verschachtelte Guards komponieren korrekt: Das `Drop` des inneren
Guards stellt den VORHERIGEN Zustand wieder her, nicht bedingungslos
"erlaubt". Ein innerer Test-Helfer, der seinen eigenen Guard
innerhalb eines äußeren, abgesicherten Scopes installiert, entschärft
den äußeren Guard beim Verlassen also nicht.

Das Flag ist absichtlich prozessglobal. Der Punkt ist, ein per
`tokio::spawn` gestartetes Future abzufangen, das stillschweigend
einem Fake-Scope entkommt und von der CI aus einen echten Dritten
anpingt. Ein Pro-Task-Flag würde das übersehen.

### `Http::spawn_with_fake_inheritance(future)`

Wenn zu testender Code legitim eine Task spawnt - ein Queue-Worker,
ein Hintergrund-Syncer, eine Sub-Task - und Sie wollen, dass seine
ausgehenden Aufrufe durch den Fake des Parents laufen, tauschen Sie
`tokio::spawn` gegen `Http::spawn_with_fake_inheritance`:

```rust
Http::fake(|| async {
    fake_response("GET", "/child", 204, json!({}));

    let handle = Http::spawn_with_fake_inheritance(async {
        // Läuft auf einer NEUEN Task, aber der Fake-Zustand des
        // Parents wird im Task-lokalen Scope dieser Task erneut
        // installiert. Der Send wird abgefangen; die Response ist
        // das 204 oben.
        Http::get("https://child.example.com/child").send().await
    });

    let response = handle.await.unwrap().unwrap();
    assert_eq!(response.status(), 204);

    // Aufgezeichnete Anfragen vom Kind erscheinen hier - das
    // Arc<Mutex<FakeState>> wird geteilt, nicht als Snapshot kopiert.
    assert_sent(|r| r.url.contains("/child"));
})
.await;
```

Ist kein Fake-Scope aktiv, wenn Sie `spawn_with_fake_inheritance`
aufrufen, ist es äquivalent zu `tokio::spawn` - das Kind läuft ohne
jeden Fake-Kontext. Sie können es also bedenkenlos in Code
verwenden, der manchmal mit `Http::fake` getestet wird und manchmal
nicht.

### Doppelte Absicherung im Test-Setup

Die beiden kombinieren sich. Ein Test, der sichtbar sicher sein
will, paart sie:

```rust
#[tokio::test]
async fn pays_the_invoice() {
    let _guard = FailOnRealCallsGuard::install();

    Http::fake(|| async {
        fake_response("POST", "/v1/charges", 200, json!({ "id": "ch_1" }));

        // Driftet ein Tippfehler in der URL oder Methode vom Fake
        // weg, fällt die Anfrage zum Guard durch, der mit einem
        // Fehler abbricht, der die URL nennt - statt
        // stillschweigend ein leeres 200 zurückzugeben, das die
        // Abweichung verdeckt.
        pay_invoice(&invoice).await.unwrap();

        assert_sent(|r| r.url.contains("/v1/charges"));
    })
    .await;
}
```

Ohne den Guard fällt eine URL oder Methode, die vom Fake abweicht,
stillschweigend auf ein Standard-`200 {}` durch, und Ihr Test
besteht, obwohl der Produktionscode einen anderen Endpunkt aufruft.
Mit dem Guard scheitern Sie sichtbar beim ersten Mismatch.

## OpenTelemetry-Trace-Propagation

Wenn das Framework mit dem `otel`-Feature gebaut ist und ein
W3C-TraceContext-Propagator installiert ist, injiziert jede
ausgehende `Http::*`-Anfrage `traceparent` (und `tracestate`, wenn
nicht leer) in ihre Header - sodass nachgelagerte Dienste den Trace
fortsetzen können. Keine Konfiguration an der Aufrufstelle; der
Propagator liest `opentelemetry::Context::current()` zur Sendezeit.

Ohne einen aktiven OTel-Kontext werden keine Header injiziert, und
ausgehende Anfragen sehen genau so aus wie zuvor. Siehe
[Beobachtbarkeit](observability.md) für das Propagator-Setup.

## Warum Suprnova abweicht

Drei kleine Abweichungen von Laravels `Http::`-Fassade sind hervorzuheben.

**Task-lokale Fakes statt eines prozessglobalen Mock-Stores.**
Laravels `Http::fake()` mutiert eine prozessweite Registry; Tests
serialisieren sich darauf, oder Sie akzeptieren, dass parallele
Runner in ein Race laufen können. Suprnovas `Http::fake` verwendet
`tokio::task_local!`, sodass zwei Tests auf zwei Tasks jeweils ihren
eigenen Fake sehen - keine Test-Reihenfolge, kein geteilter Mutex.
Der Preis ist, dass per `tokio::spawn` gestartete Arbeit den Fake
nicht standardmäßig erbt, weshalb `Http::spawn_with_fake_inheritance`
und `FailOnRealCallsGuard` existieren. Zusammen geben sie Ihnen
dieselbe Garantie "kann Produktion nicht versehentlich treffen", die
`Http::preventStrayRequests()` in Laravel gibt, mit strikterem
Scoping.

**Wiederholungen verweigern standardmäßig POST/PATCH.** Laravels
HTTP-Client wiederholt standardmäßig jede Methode. Suprnovas
`.retry(...)` ist nur idempotent; nicht-idempotente Methoden
brauchen ein explizites Opt-in über `.retry_non_idempotent(...)`.
Die Begründung ist, dass eine 5xx-Response von einem
Schreib-Endpunkt häufig bedeutet "ich habe das Schreiben committet,
und dann ging die Response verloren" - das blind zu replayen
dupliziert eine Belastung, eine Rückerstattung, einen Fan-out. Wir
zwingen den Aufrufer zu einer Entscheidung: Haben Sie einen
Idempotency-Key mitgeliefert, den das vorgelagerte System
respektiert? Falls ja, nehmen Sie POST/PATCH in die Wiederholungen
auf. Falls nein, akzeptieren Sie das 5xx.

**`retry_when` kann nur einschränken, niemals erweitern.** Der Callback `$when` von Laravels `retry()` ersetzt die Entscheidung „Soll wiederholt werden?“ vollständig und kann daher Statuscodes wiederholen, die das Framework sonst nicht berühren würde (etwa einen 404). Suprnovas `retry_when` verhindert nur einen Wiederholungsversuch, den `.retry(...)` / `.retry_non_idempotent(...)` bereits ausführen wollte – dieselbe Überlegung wie bei den standardmäßig nur idempotenten Wiederholungen: Ein Prädikat, das eine 4xx- oder nicht idempotente Response zu einer wiederholten machen könnte, ließe eine einzeilige Closure einen Seiteneffekt duplizieren, den die Standardregeln gerade verhindern sollen.

## Randfälle und Kleingedrucktes

- **`Http::*` ist für v1 geschlossen.** Wir exponieren den zugrunde
  liegenden `reqwest::Client` absichtlich nicht. Um die Oberfläche
  zu erweitern, fügen Sie der Facade eine Methode hinzu, statt
  direkt zu `reqwest` zu greifen - außer über den dokumentierten
  Notausgang `into_inner()` auf einer echten Response.
- **Der geteilte Client wird einmal gebaut und lebt für immer.**
  Lazy gebaut beim ersten Aufruf eines `Http::*`-Verbs, gehalten in
  einem `OnceLock`. Der rustls-TLS-Stack und der
  30s-Standard-Timeout sind eingebacken.
- **JSON-/Form-Serialisierungsfehlschläge scheitern sichtbar.** Ein
  `.json(&unserializable)`-Builder zeichnet den Fehler auf, und
  `send()` liefert ihn als `FrameworkError::internal(...)` zurück.
  Die Anfrage geht nie hinaus - wir degradieren nicht zu einem
  `null`-Body.
- **Die 30s-Wiederholungs-Obergrenze ist hart.** Die
  Backoff-Mathematik kappt bei 30 Sekunden; die
  `Retry-After`-Interpretation kappt bei 30 Sekunden; kein einzelner
  Wiederholungs-Schlaf parkt eine Task länger.
- **Die prozessglobale Obergrenze ist einmalig.**
  `Http::set_max_response_bytes` ist ein Schreibvorgang auf ein
  prozessglobales Atomic - setzen Sie sie einmal beim Boot und
  überschreiben Sie dann pro Anfrage nach Bedarf. Es gibt keinen
  "auf Standard zurücksetzen"-Aufruf.

## Nächste Schritte

- [Mail](mail.md) - ausgehende E-Mail, die für Tests ähnliche
  Fake-/Treiber-Muster verwendet
- [Benachrichtigungen](notifications.md) - Benachrichtigungskanäle
  einschließlich Web Push, alle teilen dieselbe
  Test-Fake-Philosophie
- [Warteschlange](queues.md) - Jobs, die ausgehende HTTP-Aufrufe
  machen, plus das `spawn_with_fake_inheritance`-Muster zum Testen
  von Workern
- [Testen](testing.md) - `#[suprnova_test]`, `TestContainer` und der
  Rest der Fakes-Oberfläche
- [Beobachtbarkeit](observability.md) - Propagator-Setup für OTel,
  das die `traceparent`-Injektion aufleuchten lässt
