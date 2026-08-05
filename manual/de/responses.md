# Antworten

Jeder Suprnova-Handler liefert eine `Response` zurück, ein Alias für
`Result<HttpResponse, HttpResponse>`. Der `Ok`-Zweig trägt die
Erfolgs-Response, der `Err`-Zweig eine bereits gerenderte
Fehler-Response, und der `?`-Operator faltet unterwegs jeden Fehlertyp
zusammen, der ein `From` nach `HttpResponse` hat. Dieses Kapitel ist die
praktische Referenz für den Bau der `Ok`-Seite - die
`HttpResponse`-Builder, den `Redirect`-Builder, die Cookie-API und die
`abort_*`-Kurzschlüsse. Zum Umgang mit Fehlern siehe
[Fehlermodell](error-model.md) und [Fehlerbehandlung](errors.md).

## `HttpResponse`-Builder

`HttpResponse` ist der Response-Typ in Wire-Form. Die Konstruktoren
setzen sinnvolle Standardwerte; die verkettbaren Setter überschreiben
sie.

### Body-Konstruktoren

```rust
use suprnova::{HttpResponse, Response};
use serde_json::json;

pub async fn examples() -> Response {
    // text/plain
    let _ = HttpResponse::text("OK");

    // application/json (jeder serde_json::Value)
    let _ = HttpResponse::json(json!({ "ok": true }));

    // text/html; charset=utf-8
    let _ = HttpResponse::html("<h1>Hello</h1>");

    // Rohe Bytes mit explizitem Content-Type - genutzt von der
    // JSON:API-Serialisierung und jedem anderen Byte-Body ohne JSON.
    let _ = HttpResponse::bytes_body(b"PNG...".to_vec(), "image/png");

    Ok(HttpResponse::text("done"))
}
```

Für langlebige Responses gibt es zwei Streaming-Konstruktoren:

- `HttpResponse::sse(stream)` - Server-Sent Events. Umhüllt einen
  `Stream` von `SseEvent`-Werten, setzt die vier erforderlichen Header
  (`Content-Type: text/event-stream`, `Cache-Control: no-cache`,
  `Connection: keep-alive`, `X-Accel-Buffering: no`) und hält die
  Verbindung offen, bis der produzierende Stream endet. Siehe
  [Server-Sent Events](sse.md).
- `HttpResponse::stream_bytes(stream)` - generische Chunked-Response.
  Nimmt einen `Stream<Item = Result<Bytes, Infallible>>` entgegen. Der
  Fehlertyp ist bewusst `Infallible`: Jeder Produzent im Framework
  verwandelt seine eigenen Fehler in eine abschließende
  Stream-Nachricht, bevor der Stream endet, denn es gibt keinen Weg,
  einen Fehler auf Transportebene mitten in der Response noch zum
  Client zu bringen.

### Status, Header, Cookies

Jeder Builder liefert `Self` zurück, verketten Sie also nach Belieben:

```rust
use suprnova::{Cookie, HttpResponse, Response};
use serde_json::json;

pub async fn created() -> Response {
    Ok(HttpResponse::json(json!({ "id": 42 }))
        .status(201)
        .header("X-Resource-Id", "42")
        .cookie(Cookie::new("last_id", "42")))
}
```

| Methode | Verhalten |
|---|---|
| `.status(code)` | Setzt den HTTP-Status. Codes außerhalb von `100..=599` werden an der Wire-Grenze mit einem Warn-Log auf 500 heruntergestuft. |
| `.header(name, value)` | Hängt einen Header an. Duplikate sind erlaubt (entspricht der `Set-Cookie`-Semantik). |
| `.replace_header(name, value)` | Verwirft alle früheren Vorkommen und setzt eines. |
| `.with_headers([(k, v), ...])` | Hängt mehrere auf einmal an. Nimmt jedes `IntoIterator<Item = (K, V)>` entgegen. |
| `.without_header(name)` | Entfernt jedes Vorkommen (ohne Unterscheidung von Groß- und Kleinschreibung). |
| `.header_value(name)` | Liest den zuerst gesetzten Wert zurück. Nützlich in Tests. |
| `.cookie(Cookie)` | Hängt ein Cookie als `Set-Cookie` an. |
| `.with_cookies([Cookie, ...])` | Hängt mehrere an. |
| `.without_cookie(name)` | Merkt eine Löschung vor (gleichbedeutend mit `Cookie::forget(name)`). |

Dieselben verkettbaren Setter stehen über den `ResponseExt`-Trait auch
auf einer `Response` (dem `Result`) zur Verfügung, damit die Makros
ergonomisch bleiben:

```rust
use suprnova::{json_response, Cookie, Response, ResponseExt};

pub async fn list() -> Response {
    json_response!({ "ok": true })
        .status(200)
        .header("X-Total-Count", "42")
        .cookie(Cookie::new("last_query", "list"))
}
```

`ResponseExt` bietet `.status`, `.header`, `.with_headers`,
`.without_header`, `.cookie`, `.with_cookies` und `.without_cookie`.

### Validierung an der Wire-Grenze

`HttpResponse::into_hyper` lässt zwei Sicherheitsfilter laufen, bevor
die Response an hyper übergeben wird:

- **Statusbereich.** Alles außerhalb von `100..=599` wird mit einem
  `tracing::warn!` auf 500 heruntergestuft. Das fängt Tippfehler wie
  `AppError::status(700)` an der Grenze ab, statt nicht konforme Codes
  bis auf die Wire-Ebene durchzulassen.
- **CRLF-Injection in Headern.** Jeder Headername und -wert wird über
  hypers eigenes `HeaderName::try_from` / `HeaderValue::try_from`
  geprüft. Ein abgelehnter Header wird mit einem Warn-Log verworfen und
  die Response ohne ihn gebaut. Von Angreifern kontrollierte Werte, die
  in einem Header gespiegelt werden (CORS-Allow-Header,
  `X-Forwarded-*`, eigene Debug-Header), können die Response nicht
  aufspalten.

Beide Filter bleiben im Erfolgsfall stumm - Sie sehen sie nur im Log,
wenn etwas durchzuschlüpfen versucht hat.

## Response-Makros

Für die häufigen Fälle gibt es zwei Makros in `Response`-Form:

```rust
use suprnova::{json_response, text_response, Response};

pub async fn json_handler() -> Response {
    json_response!({ "users": [{ "id": 1, "name": "Alice" }] })
}

pub async fn text_handler() -> Response {
    text_response!("OK")
}
```

Beide expandieren zu `Ok(HttpResponse::...)`. Verketten Sie auf beiden
`ResponseExt`-Setter, um Status, Header oder Cookies anzupassen.

## Cookies

`Cookie::new(name, value)` erzeugt ein Cookie mit sicheren
Standardwerten - `HttpOnly`, `Secure`, `SameSite=Lax`, `Path=/`.
Überschreiben Sie sie pro Cookie:

```rust
use suprnova::Cookie;
use std::time::Duration;

let session = Cookie::new("session_id", "abc123")
    .http_only(true)
    .secure(true)
    .same_site(suprnova::SameSite::Strict)
    .path("/")
    .domain("example.com")
    .max_age(Duration::from_secs(3600))
    .partitioned(true);
```

Drei Komfort-Konstruktoren decken gängige Muster ab:

- `Cookie::forget(name)` - leerer Wert, `Max-Age=0`. Verwenden Sie das
  beim Logout, um den Browser anzuweisen, das Cookie zu verwerfen.
- `Cookie::forever(name, value)` - `Max-Age` von fünf Jahren.
- `Cookie::encrypted(name, plaintext)` - AES-256-GCM-Chiffrat, an die
  AAD `CryptPurpose::Cookie` gebunden, damit Cookie-Chiffrat nicht auf
  eine andere Framework-Oberfläche (Cursor, 2FA-Geheimnisse, Casts)
  wiedereingespielt werden kann. Setzt voraus, dass `APP_KEY` beim Boot
  gesetzt ist. Das Gegenstück `Cookie::read_encrypted(wire)`
  entschlüsselt einen Wert, den derselbe Weg erzeugt hat. Siehe
  [Verschlüsselung](encryption.md).

Die Header-Serialisierung prozent-kodiert jedes Byte, das nach RFC 6265
kein gültiges Cookie-Oktett ist, einschließlich aller Steuerzeichen.
CRLF in einem Cookie-Namen oder -Wert wird kodiert, nicht
weitergereicht - Header-Injection über Cookies ist im Serializer
geschlossen.

## Redirects

`Redirect` deckt die vollständige Oberfläche von Laravels Redirector
ab. Jede Variante implementiert `From<Redirect> for Response`, die
idiomatische Form ist also `Redirect::...().into()`.

### Ziele

```rust
use suprnova::{Redirect, redirect_to};

// Explizite URL oder expliziter Pfad
let _ = Redirect::to("/dashboard");

// Dasselbe, als etwas kürzere freie Funktion
let _ = redirect_to("/dashboard");

// Benannte Route (liefert RedirectRouteBuilder)
let _ = Redirect::route("users.show").with("id", "42");

// Explizite externe URL - dasselbe wie `to`, aber der Name signalisiert
// "das führt vom eigenen Angebot weg" für Open-Redirect-Audits
let _ = Redirect::away("https://external.example.com");

// Die Seite neu laden (liest die vorherige URL aus der Session; fällt
// auf "/" zurück, wenn kein Session-Scope aktiv ist)
let _ = Redirect::refresh();

// Dasselbe, aber mit explizitem Request, wenn kein Scope aktiv ist
// let _ = Redirect::refresh_for(&request);

// previous_url der Session, mit Fallback, wenn keine Session im Scope ist
let _ = Redirect::back("/login");

// In der Session vorgemerkte Ziel-URL, beim Lesen verbraucht, mit Fallback
let _ = Redirect::intended("/home");

// Gast-Redirect: legt die URL der aktuellen Anfrage als "intended" ab
// und schickt den Benutzer auf eine Login-Seite
// let _ = Redirect::guest(&request, "/login");
```

`Redirect::back`, `Redirect::intended`, `Redirect::guest` und
`Redirect::refresh` binden sich alle in die Session ein. Ohne
Session-Scope fallen sie stillschweigend auf ihre Standardwerte
zurück - praktisch für unvollständige Test-Setups. Siehe
[Sitzungen](session.md).

### Validierung benannter Routen

Das Proc-Makro `redirect!` prüft den Routennamen zur Compile-Zeit und
expandiert zu `Redirect::route(name)`:

```rust
use suprnova::{redirect, Response};

pub async fn store() -> Response {
    // Der Build schlägt fehl, wenn "users.index" kein registrierter
    // Routenname ist; die Fehlermeldung listet die verfügbaren Routen
    // und schlägt nahe Treffer vor.
    redirect!("users.index").into()
}
```

### Statuscodes

```rust
use suprnova::Redirect;

let _ = Redirect::to("/x").permanent();      // 301
let _ = Redirect::to("/x").status(303);      // 303, 307, 308, ...
```

Der Standardwert ist 302.

### Flash-Daten

Redirect-Builder tragen ihre eigene Flash-Bag. Bei der Umwandlung in
eine `Response` fließt die Bag in die aktive Session ab und überlebt
genau eine weitere Anfrage:

```rust
use suprnova::Redirect;

let _ = Redirect::back("/users/new")
    .with("status", "User created")            // einzelnes Schlüssel/Wert-Paar
    .with_input([                              // Formular neu befüllen
        ("email", "shawn@example.com"),
        ("name", "Shawn"),
    ])
    .with_errors([                             // Standard-Error-Bag
        ("email", "Must be unique"),
    ])
    .with_errors_bag("login", [                // benannte Error-Bag
        ("password", "Required"),
    ]);
```

Die empfangende Seite liest das über `session.get(...)` (für `with`),
`session.get_old_input(...)` (für `with_input`) und die Bag-Map zurück,
die `session.pull_errors_flash()` leert (für `with_errors` /
`with_errors_bag`). Die Inertia-Schicht verbraucht den Errors-Flash
automatisch - die `errors`-Prop jeder Inertia-Response wird aus der
Session vorbelegt, sodass `Redirect::back().with_errors(...)` Meldungen
am Ziel ohne zusätzliche Verdrahtung zur Anzeige bringt. Der
Anfrage-Header `X-Inertia-Error-Bag` bindet die Prop bei Seiten mit
mehreren Formularen an eine benannte Bag.

Beachten Sie, dass `.with(key, value)` auf dem `RedirectRouteBuilder`
(dem Rückgabewert von `Redirect::route` und `redirect!`) einen
**Routenparameter** setzt und keinen Flash-Eintrag - dort nehmen Sie
`.flash(key, value)`:

```rust
use suprnova::redirect;

let _ = redirect!("users.show")
    .with("id", "42")                          // Routenparameter
    .flash("status", "Updated");               // Session-Flash
```

### Cookies, Header, Fragmente

```rust
use suprnova::{Cookie, Redirect};

let _ = Redirect::route("billing.show")
    .with_cookies([Cookie::new("welcome", "yes")])
    .with_headers([("X-Trace", "abc")])
    .with_fragment("invoices")                 // hängt #invoices an
    .without_fragment();                       // ODER entfernt jedes frühere Fragment
```

`with_fragment` nimmt das Fragment mit oder ohne führendes `#`
entgegen. Ein `with_fragment` nach `without_fragment` hängt wieder
eines an.

### Das Fragment über den Redirect hinweg erhalten

Für Inertia-Apps, bei denen das Ziel den Hash der *ursprünglichen* URL
beibehalten soll, verwenden Sie `preserve_fragment`:

```rust
use suprnova::Redirect;

let _ = Redirect::route("dashboard.index").preserve_fragment();
```

Bei der Umwandlung flasht das `_inertia.preserve_fragment = true` in
die Session; die nächste Inertia-Response liest das Flag und gibt in
ihrem Seitenobjekt `preserveFragment: true` aus. Kein Session-Scope -
das Flag wird stillschweigend verworfen.

### Signierte Redirects

Zwei Builder umschließen die Oberfläche zum URL-Signieren für einmalige
Redirects auf benannte Routen (Passwort-Reset, E-Mail-Verifizierung,
Download-Links):

```rust
use suprnova::Redirect;

let r = Redirect::signed_route("downloads.show", &[("id", "42")])?;
let r = Redirect::temporary_signed_route(
    "downloads.show",
    &[("id", "42")],
    1_700_000_000, // expires_at_epoch_seconds
)?;
```

Beide liefern `Result<Redirect, FrameworkError>` - propagieren Sie den
Fehler mit `?`, da sich `Redirect` sauber in eine `Response` umwandelt.
Siehe [URLs](urls.md) für die Oberfläche zum Signieren.

### Die vorgemerkte Ziel-URL speichern

`Redirect::set_intended_url` schreibt das vorgemerkte Ziel der Session,
ohne einen Redirect auszuführen - typischerweise aus einer
Auth-Middleware heraus aufgerufen, bevor auf `/login` umgeleitet wird,
sodass ein späteres `Redirect::intended` die ursprünglich angeforderte
URL wiederherstellen kann:

```rust
suprnova::Redirect::set_intended_url("/admin/users");
```

## Aus einem Handler heraus abbrechen

Drei freie Funktionen schließen einen Handler bei einem gegebenen
Status kurz. Sie liefern `Result<(), FrameworkError>`; kombinieren Sie
sie mit `?`:

```rust
use suprnova::{abort_if, abort_unless, abort_with, json_response, Request, Response};

pub async fn show(req: Request) -> Response {
    abort_unless(Auth::user().await?.is_some(), 401, "must be logged in")?;
    abort_if(req.param("id")? == "0", 404, "User not found")?;
    abort_with(503, "scheduled maintenance")?;
    json_response!({ "ok": true })
}
```

Der zugrunde liegende Fehler ist
`FrameworkError::Domain { message, status_code }`, er wird also über
denselben JSON-Umschlag und dieselben Regeln zur Bereinigung von
5xx-Fehlern gerendert wie jeder andere Fehlerweg. Statuscodes außerhalb
des gültigen Bereichs zwingt der Response-Renderer auf 500. Siehe
[Fehlermodell](error-model.md) für den vollständigen
Umwandlungsvertrag.

## Fehler direkt zurückgeben

Weil `Response` ein `Result<HttpResponse, HttpResponse>` ist, können
Sie direkt einen `Err`-Zweig zurückgeben - nützlich, wenn die Form der
Response bereits ein bestimmter JSON-Body ist und Sie ihn unverändert
auf die Wire-Ebene bringen wollen:

```rust
use suprnova::{HttpResponse, Response};
use serde_json::json;

pub async fn legacy_lookup() -> Response {
    Err(HttpResponse::json(json!({
        "error": "deprecated endpoint",
    })).status(410))
}
```

Für alles Reichhaltigere - typisierte Domain-Fehler, Validierung,
Observability - greifen Sie zur Oberfläche des
[Fehlermodells](error-model.md) (`AppError`, `FrameworkError`,
`#[domain_error]`).

## Kurzreferenz

| Bedarf | Verwendung |
|---|---|
| JSON-Response | `HttpResponse::json(v)` oder `json_response!({...})` |
| Text-Response | `HttpResponse::text(s)` oder `text_response!(s)` |
| HTML-Response | `HttpResponse::html(s)` |
| Rohe Bytes + Content-Type | `HttpResponse::bytes_body(b, "image/png")` |
| Server-Sent Events | `HttpResponse::sse(stream)` - siehe [SSE](sse.md) |
| Gechunkter Stream | `HttpResponse::stream_bytes(stream)` |
| Status setzen | `.status(code)` |
| Header hinzufügen | `.header(k, v)` / `.with_headers([...])` |
| Header entfernen | `.without_header(name)` |
| Cookie anhängen | `.cookie(c)` / `.with_cookies([...])` |
| Cookie vergessen | `.without_cookie(name)` |
| Einfacher Redirect | `Redirect::to(path).into()` oder `redirect_to(path).into()` |
| Redirect auf eine benannte Route | `redirect!("name").into()` oder `Redirect::route("name")` |
| Redirect zurück | `Redirect::back(fallback)` |
| Redirect auf das vorgemerkte Ziel | `Redirect::intended(default)` |
| Gast-Redirect (Ziel vormerken) | `Redirect::guest(&req, login)` |
| Vorgemerktes Ziel setzen | `Redirect::set_intended_url(url)` |
| Externe URL | `Redirect::away(url)` |
| Aktuelle Seite neu laden | `Redirect::refresh()` / `Redirect::refresh_for(&req)` |
| Redirect auf eine signierte Route | `Redirect::signed_route(name, &[(k, v)])?` |
| Routenparameter am Redirect | `.with("key", "value")` |
| Query-Parameter am Redirect | `.query("key", "value")` |
| Flash-Daten | `.with(key, value)` (oder `.flash` auf dem `RedirectRouteBuilder`) |
| Flash-Eingaben | `.with_input([(k, v), ...])` |
| Flash-Fehler | `.with_errors([(k, msg), ...])` |
| Benannte Error-Bag | `.with_errors_bag(bag, [(k, msg)])` |
| Fragment anhängen | `.with_fragment("section")` |
| Fragment entfernen | `.without_fragment()` |
| Fragment erhalten (Inertia) | `.preserve_fragment()` |
| Permanenter Redirect | `.permanent()` (301) |
| Eigener Redirect-Status | `.status(303)` |
| Früh abbrechen | `abort_with(code, msg)?`, `abort_if(cond, code, msg)?`, `abort_unless(cond, code, msg)?` |

## Nächste Schritte

- [Fehlermodell](error-model.md) - `FrameworkError`, `AppError`,
  `HttpError` und die eine Umwandlung, die jeden Fehler zu einer
  `HttpResponse` rendert
- [Fehlerbehandlung](errors.md) - praktische Handler-Muster für `?`,
  `AppError` und eigene Domain-Fehler
- [Server-Sent Events](sse.md) - `sse(...)`-Responses bauen und
  konsumieren
- [URLs](urls.md) - signierte URLs, Auflösung benannter Routen, die
  Oberfläche hinter `Redirect::signed_route`
- [Sitzungen](session.md) - Flash-Daten, vorgemerkte Ziel-URLs, die
  Bag, in die `Redirect::with`/`with_input`/`with_errors` schreiben
