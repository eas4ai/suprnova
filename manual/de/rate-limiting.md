# Ratenbegrenzung

Suprnova liefert zwei sich ergänzende Oberflächen zur Ratenbegrenzung:

| Oberfläche | Verwenden, wenn... | Backend |
|---------|-------------|---------|
| `RateLimiterDriver` + `RateLimitMiddleware` | Sie strikte Sliding-Window-Durchsetzung gegen beliebigen Storage wollen (Redis ZSET, In-Memory-Deque) | `dyn RateLimiterDriver` |
| `RateLimiter` + `ThrottleRequestsMiddleware` | Sie Laravel-förmige benannte Limiter, `attempt()`-Workflow-Callbacks oder `X-RateLimit-*`-Response-Header wollen | `Cache`-Store (Memory oder Redis) |

Der Sliding-Window-Treiber ist Suprnovas native Form - ein Slot pro
Anfrage, kein separater Timer-Key, atomares Lua-Eval auf Redis. Zur
Laravel-Facade greifen migrierte Apps, und sie ist es, die das
Muster aus benannten Limitern und Response-Callbacks verlangt. Die
beiden koexistieren per Design, und eine Route kann beide schichten.

## Sliding-Window-Treiber-SPI

`RateLimiterDriver` ist die Storage-SPI für den
Sliding-Window-Algorithmus. Jeder Schlüssel verfolgt eine Deque von
Treffer-Zeitstempeln. Bei jedem `try_acquire` werden Einträge, die
älter als `now - window` sind, verworfen; liegt die verbleibende
Zahl unter `max_requests`, wird `now` angehängt und der Aufruf
akzeptiert. Andernfalls lehnt er ab.

```rust
use std::sync::Arc;
use std::time::Duration;
use suprnova::rate_limit::memory::InMemoryRateLimiter;
use suprnova::rate_limit::{RateLimiterDriver, SlidingWindowConfig};

let limiter: Arc<dyn RateLimiterDriver> = Arc::new(InMemoryRateLimiter::new());
let cfg = SlidingWindowConfig {
    max_requests: 60,
    window: Duration::from_secs(60),
};
let ok = limiter.try_acquire("user:42", &cfg).await?;
if !ok {
    let wait = limiter.retry_after("user:42", &cfg).await?;
    // wait ist die Option<Duration>, bis der älteste Slot im Bucket
    // abläuft.
}
```

### Eingebaute Treiber

| Treiber | Storage | Ausgewählt über |
|--------|---------|--------------|
| `InMemoryRateLimiter` | Pro-Prozess-`HashMap<String, Bucket>` mit `tokio::time::Instant`, damit `start_paused`-Tests die Uhr steuern können | `RATE_LIMIT_DRIVER=memory` (Standard) |
| `RedisRateLimiter` | Redis-ZSET + atomares Lua-Check-and-Record | `RATE_LIMIT_DRIVER=redis` + `RATE_LIMIT_REDIS_URL` |

`bootstrap_from_env()` verdrahtet den passenden Treiber in den
Container. Außerhalb der Produktion fällt ein unbekannter
Treiber-Wert mit einer `warn!`-Meldung auf Memory zurück.

### In Produktion ist der In-Memory-Treiber fail-closed

In Produktion ist die Auflösung zum In-Memory-Limiter ein
Boot-Fehlschlag:

```
refusing to boot in production: RATE_LIMIT_DRIVER is unset, which defaults
to the in-memory limiter. Per-process buckets mean every configured quota
is multiplied by your replica count and reset by every deploy...
```

Der In-Memory-Treiber hält seine Buckets auf dem Heap eines
einzigen Prozesses. Hinter N Repliken führt jede ihre eigene
Zählung, sodass ein Passwort-Reset-Throttle von „5 Versuche pro 15
Minuten“ in Wirklichkeit 5N ist, und jedes Deploy setzt sie alle auf
null zurück. Das Limit, das Sie konfiguriert haben, ist nicht das
Limit, das Sie bekommen - und nichts sagt Ihnen das, weil die
Anfragen erfolgreich sind, was von außen genauso aussieht wie ein
funktionierendes Throttle. Es fällt als Credential-Stuffing- oder
Account-Enumeration-Vorfall auf, nicht als Fehler.

Ein **nicht erkannter** Treiber-Wert schlägt aus demselben Grund
fehl: Er fällt auf Memory zurück. `RATE_LIMIT_DRIVER=Redis` - mit
großem R - würde sonst einmal beim Boot warnen und ein
Multi-Repliken-Deployment stillschweigend pro Prozess drosseln
lassen. Das ist der Fall, der am wahrscheinlichsten die Produktion
erreicht, weil er konfiguriert aussieht.

Entweder weisen Sie ihn auf Redis:

```env
RATE_LIMIT_DRIVER=redis
RATE_LIMIT_REDIS_URL=redis://cache.internal:6379
```

oder, falls Sie wirklich einen einzigen Prozess betreiben, sagen Sie
das:

```env
RATE_LIMIT_ALLOW_MEMORY_IN_PRODUCTION=true
```

Entwicklung, Testing und **Staging** bleiben unberührt. Staging ist
absichtlich nicht abgesichert, aus derselben Überlegung wie bei der
Mail-Absicherung: Ein hartes Fehlschlagen dort drängt Teams dazu,
den Override global zu setzen, was die Prüfung genau dort entwaffnet,
wo es zählt.

### `RateLimitMiddleware`

Der HTTP-Wrapper um den Treiber. Konstruieren Sie ihn mit einem
`key_fn`-Closure, um die Bucket-Auswahl pro Anfrage zu steuern:

```rust
use std::sync::Arc;
use std::time::Duration;
use suprnova::container::App;
use suprnova::rate_limit::{
    BackendErrorPolicy, RateLimitMiddleware, RateLimiterDriver, SlidingWindowConfig,
};

let limiter: Arc<dyn RateLimiterDriver> =
    App::resolve_make::<dyn RateLimiterDriver>().unwrap();

let mw = RateLimitMiddleware::new(
    limiter,
    SlidingWindowConfig {
        max_requests: 100,
        window: Duration::from_secs(60),
    },
    |req| format!("route:{}", req.path()),
)
.on_backend_error(BackendErrorPolicy::FailClosed);
```

Bei Ablehnung (Kontingent überschritten) liefert er HTTP 429 mit
einem `Retry-After`-Header.

### Begrenzung pro Empfänger, nicht nur pro Aufrufer

Ein adress-geschlüsseltes Limit beantwortet die Frage, *ob ein
Client zu viele Anfragen stellt*. Es kann nicht beantworten, *ob ein
Postfach geflutet wird*. Ein Angreifer, der über ein Botnet, einen
Proxy-Pool oder ein einzelnes IPv6-`/64` verteilt ist, bleibt unter
jedem Pro-IP-Budget, während er einem einzigen Opfer Tausende von
Passwort-Reset-E-Mails schickt - das Postfach ist die Ressource, die
erschöpft wird, und die Adresse des Opfers ist das Einzige, das
diese Anfragen teilen. Auch das Umgekehrte tut weh: Hinter
Carrier-Grade-NAT oder einem Firmen-Gateway bestrafen Pro-IP-Limits
eine ganze Gruppe für das Verhalten eines einzelnen Mitglieds.

`identity_key` schlüsselt einen Bucket auf das Konto, *auf das
eingewirkt wird*:

```rust
use suprnova::rate_limit::{identity_key, names_identity};

let per_recipient = RateLimitMiddleware::new(
    limiter.clone(),
    SlidingWindowConfig { max_requests: 3, window: Duration::from_secs(900) },
    |req| identity_key(req, "email", "auth-issuance"),
)
.key_reads_body(4096)
.only_when(|req| names_identity(req, "email"))
.on_backend_error(BackendErrorPolicy::FailClosed);
```

Stapeln Sie es *neben* einem Pro-IP-Limiter, statt eines durch das
andere zu ersetzen. Jedes fängt, was das andere nicht kann: Pro-IP
stoppt einen Host, der viele Adressen enumeriert; Pro-Empfänger
stoppt viele Hosts, die auf eine Adresse zielen.

Drei Details tragen die Sicherheit:

- **`key_reads_body`** puffert den Body (bis zur angegebenen
  Obergrenze), bevor der Schlüssel berechnet wird, damit das Feld
  sowohl aus einem formularkodierten POST als auch aus einem
  Query-String gelesen werden kann. Es ist opt-in, weil Puffern
  Arbeit ist, die ein nicht authentifizierter Aufrufer Sie machen
  lassen kann; die Obergrenze begrenzt das. Ein Body über der
  Obergrenze wird mit 413 abgelehnt, statt ungeschlüsselt
  durchgelassen zu werden - sonst wäre das Auffüllen des Bodys ein
  Weg aus dem Limit heraus.
- **`only_when`** überspringt den Limiter für Anfragen, die
  niemanden nennen. Ohne es fallen diese in den Adress-Fallback von
  `identity_key` und werden gegen das Kontingent *dieses* Limiters
  gezählt - und da ein Pro-Empfänger-Budget normalerweise das
  engere der beiden ist, würde es stillschweigend zum bindenden
  Limit für jede Route, die niemanden nennt.
- **Der Wert wird normalisiert und gehasht.** `Alice@Example.com`
  und `alice@example.com` erreichen dasselbe Postfach und müssen
  sich einen Bucket teilen, sonst wird das Limit durch geänderte
  Groß-/Kleinschreibung umgangen. Das Ergebnis wird gehasht, weil
  ein Rate-Limit-Backend häufig ein gemeinsam genutztes Redis mit
  schwächerer Zugriffskontrolle als die primäre Datenbank ist, und
  ein Schlüssel-Dump sich nicht wie eine Liste von Leuten lesen
  sollte, die ihr Passwort zurücksetzen.

### Backend-Fehler-Richtlinie

`BackendErrorPolicy` bestimmt, was passiert, wenn das *Backend* des
Limiters selbst einen Fehler wirft - z. B. wenn Redis nicht
erreichbar ist -, im Unterschied zu einer Anfrage, die ihr Kontingent
legitim überschreitet. Das Backend kann keine Entscheidung treffen,
also muss die Middleware zwischen Verfügbarkeit und der Garantie des
Limits wählen.

| Richtlinie | Verhalten | Wann verwenden |
|--------|-----------|-------------|
| `FailOpen` (Standard) | Lässt die Anfrage durch; protokolliert auf `warn` | Die meisten öffentlichen APIs - ein Limiter-Ausfall sollte den Verkehr nicht lahmlegen |
| `FailClosed` | Lehnt mit HTTP 503 + `Retry-After: 1` ab; protokolliert auf `error` | Sensible Routen (Login, Passwort-Reset, Zahlungen), wo unbegrenzter Verkehr während eines Backend-Ausfalls schlimmer ist als kurzes Ablehnen |

Wählen Sie über `.on_backend_error(BackendErrorPolicy::FailClosed)`
auf der Middleware. Anfragen mit erschöpftem Kontingent sind immer
429, unabhängig von der Richtlinie - die Richtlinie betrifft nur den
Fallthrough bei Backend-Fehlern.

## Cache-gestützte Laravel-förmige Facade

`RateLimiter` (die Struktur) spiegelt `Illuminate\Cache\RateLimiter`:
ein Fixed-Window-Zähler, aufgebaut auf der [`Cache`](cache.md)-Facade
von Suprnova. Verwenden Sie ihn für benannte Limiter,
`attempt()`-Workflows, oder immer, wenn Sie die
`X-RateLimit-*`-Header wollen, die Laravel-Apps erwarten.

### Storage-Layout

Für einen Versuchszähler-Schlüssel `K` mit einem Decay von `D`
Sekunden:

- `K` - i64-Zähler, inkrementiert von jedem `hit`. Initialer Seed
  ist 0 (über `Cache::add`).
- `K:timer` - i64-Unix-Sekunden-seit-Epoch, wann das Fenster endet,
  gesetzt über `Cache::add`, sodass nur der erste Aufrufer in einem
  Fenster die Deadline festlegt.

Beide Schlüssel tragen dieselbe TTL, sodass der Cache sie
automatisch aufräumt, wenn das Fenster endet. Wenn der Zähler
`max_attempts` erreicht hat, aber der `:timer` verschwunden ist,
setzt `too_many_attempts` den Zähler zurück - das lässt das Fenster
nach einer Phase erschöpften Kontingents nach vorne gleiten.

### Zähler-API

```rust
use suprnova::RateLimiter;

// Verbraucht einen Versuch; seedet das Fenster, falls es fehlt.
let n = RateLimiter::hit("login:1.2.3.4", 60).await?;

// Verbraucht einen Versuch UND prüft das Limit in einem einzigen
// atomaren Round-Trip. Liefert `true`, wenn dieser Hit den Bucket
// über `max` getrieben hat (Anfrage ablehnen), `false`, wenn sie
// zugelassen wurde. Verwenden Sie dies statt eines separaten Paars
// aus `too_many_attempts` + `hit`: Prüfen und dann `hit` als zwei
// getrennte Aufrufe lässt gleichzeitige Anfragen am Limit
// vorbeischlüpfen (ein Check-then-Act-Race).
// `i64::MAX` als max bedeutet „unbegrenzt“ - lässt immer zu, zählt
// trotzdem.
let over_limit = RateLimiter::hit_and_check("login:1.2.3.4", 5, 60).await?;
if over_limit { /* return 429 */ }

// Um N erhöhen; nützlich für „kostengewichtete“ Limits (jede Anfrage
// verbraucht mehr als einen Versuch).
let n = RateLimiter::increment("api:user:1", 60, 5).await?;

// Liest die aktuelle Zählung (0, wenn nie gehittet oder abgelaufen).
let attempts = RateLimiter::attempts("login:1.2.3.4").await?;

// Anzahl Sekunden, bis das Fenster wieder öffnet (0, wenn kein
// Fenster offen ist).
let secs = RateLimiter::available_in("login:1.2.3.4").await?;

// Verbleibende Wiederholungen, bevor es auslöst.
let remaining = RateLimiter::remaining("login:1.2.3.4", 5).await?;
// retries_left ist der Laravel-geschriebene Alias von remaining.
let remaining = RateLimiter::retries_left("login:1.2.3.4", 5).await?;

// Ist der Bucket JETZT GERADE über seinem Limit (bei noch offenem
// Fenster)?
let over = RateLimiter::too_many_attempts("login:1.2.3.4", 5).await?;

// Nur den Zähler löschen (Timer bleibt - das Fenster ist noch
// festgelegt).
RateLimiter::reset_attempts("login:1.2.3.4").await?;

// Beide löschen, Zähler und Timer.
RateLimiter::clear("login:1.2.3.4").await?;
```

### `attempt()`-Workflow

Führt einen Callback nur aus, wenn der Bucket unter dem Kontingent
liegt; der Hit wird nur verbraucht, wenn der Callback läuft:

```rust
let result = RateLimiter::attempt(
    "login:1.2.3.4",
    5,
    || async { do_login_work().await },
    60,
).await?;
match result {
    Some(value) => { /* Callback lief, Versuch gezählt */ }
    None => { /* über dem Limit, Callback wurde NICHT ausgeführt */ }
}
```

Das ist die richtige Form für Login-Formulare - Sie verbrauchen
keinen Versuch, außer die Arbeit hat den Callback tatsächlich
erreicht.

### Benannte Limiter

Registrieren beim Boot, auflösen zur Anfragezeit. Der
Laravel-seitige Name `for` ist ein reserviertes Rust-Schlüsselwort,
daher ist der primäre Rust-seitige Name `define`; der wörtliche
Laravel-Alias wird über `r#for` freigelegt.

```rust
use suprnova::{Limit, RateLimiter};

// Beim Boot - `define` ist der primäre Rust-seitige Name.
RateLimiter::define("api", |req| {
    // `req.ip()`, nicht der rohe `X-Forwarded-For`-Header - siehe unten.
    let key = req.ip().unwrap_or_else(|| "anon".into());
    Limit::per_minute(60).by(format!("ip:{key}")).into()
});

// Laravel-seitiger Alias - dasselbe unter der
// Schlüsselwort-Escape-Schreibweise.
RateLimiter::r#for("uploads", |_req| Limit::per_hour(100).into());

// Auflösen.
let cb = RateLimiter::limiter("api").unwrap();
let limit_result = cb(&request);
```

Ein Callback für einen benannten Limiter liefert ein
[`LimitResult`], konstruierbar aus:

- Einem einzelnen `Limit` - dieses Limit anwenden.
- Einem `Vec<Limit>` - jedes Limit anwenden; das erste, das
  auslöst, gewinnt.
- Einer `HttpResponse` - sofort mit dieser Response kurzschließen
  (verwendet für „Admin bekommt unbegrenzten Zugriff“ über
  `Limit::none()`, oder um die Anfrage rundweg abzulehnen).

### Schlüssel bereinigen

`RateLimiter::clean_rate_limiter_key(key)` entfernt
`&abc;`-HTML-Entity-Marker aus einem Schlüssel - Laravel verwendet
dies für benutzerseitig gelieferte Strings, die einen Umlauf durch
`htmlentities` machen. Suprnova reproduziert die Entfernungsstufe
exakt, wendet aber NICHT zusätzlich vorab die
`htmlentities`-Kodierung an (die nur für Nicht-UTF-8-Eingaben
relevant ist - irrelevant für Rust-`String`). Die Funktion ist
innerhalb von Suprnova deterministisch und idempotent; Konsumenten,
die byte-identisches Hashing mit einem PHP-Dienst brauchen, sollten
ihren eigenen `htmlentities`-Vorschritt auf die Eingabe anwenden.

```rust
assert_eq!(RateLimiter::clean_rate_limiter_key("a&amp;b"), "aab");
```

## `Limit`-Builder

Der Datentyp, den Callbacks für benannte Limiter liefern.
Kurzform-Konstruktoren spiegeln Laravels `Limit::per*`:

```rust
use suprnova::Limit;
use std::time::Duration;

Limit::per_second(10, 1);           // 10 pro 1 Sekunde (max_attempts, decay_seconds)
Limit::per_minute(60);              // 60 pro Minute
Limit::per_minutes(5, 100);         // 100 pro 5 Minuten (Decay zuerst, Laravel-Signatur)
Limit::per_hour(1_000);             // 1000/Std.
Limit::per_hours(6, 5_000);         // 5000 pro 6 Stunden
Limit::per_day(10_000);             // 10000/Tag
Limit::per_days(7, 50_000);         // 50000 pro 7 Tage
Limit::new(123, Duration::from_secs(45));  // reiner Ctor

// Builder-Kette.
let l = Limit::per_minute(5)
    .by("user:42")
    .response(|req| {
        suprnova::HttpResponse::text("blocked").status(429)
    })
    .after(|response| response.status_code() >= 400);
```

- `.by(key)` - setzt den Bucket-Schlüssel. Ein leerer Schlüssel
  bedeutet „global“ (jeder Aufrufer teilt sich einen Bucket).
- `.response(callback)` - erzeugt eine benutzerdefinierte Response,
  wenn das Limit auslöst; der Default ist ein einfaches 429 „Too
  Many Attempts.“.
- `.after(callback)` - verbraucht den Versuch nur, wenn
  `callback(response)` `true` liefert. Kanonische Verwendung: nur
  fehlgeschlagene Logins zählen (`after(|r| r.status_code() >= 400)`).

`Limit::none()` liefert ein `Unlimited` (ein `GlobalLimit` mit
`max_attempts = i64::MAX`). Es von einem benannten Limiter
zurückzugeben ist das Laravel-Muster für Bypass. `GlobalLimit`
selbst ist ein dünner Wrapper um `Limit` mit leerem Schlüssel,
erhalten für Parität mit `Illuminate\Cache\RateLimiting\GlobalLimit`.

## `ThrottleRequestsMiddleware`

HTTP-Wrapper um die Cache-gestützte Facade. Spiegelt
`Illuminate\Routing\Middleware\ThrottleRequests`. Drei
Konstruktoren:

```rust
use suprnova::{Limit, ThrottleRequestsMiddleware};

// Benannter Limiter - löst zur Anfragezeit über
// RateLimiter::limiter(name) auf.
ThrottleRequestsMiddleware::by_name("api");

// Inline max/decay/prefix - die wörtliche Laravel-Form `throttle:60,1`.
ThrottleRequestsMiddleware::with(60, 1, "myroute");

// Explizite Liste von Limits - das erste, das auslöst, gewinnt; am
// rust-idiomatischsten.
ThrottleRequestsMiddleware::with_limits(vec![
    Limit::per_hour(5_000).by("user:1"),
    Limit::per_minute(60).by("user:1"),
]);
```

Verdrahten Sie es in eine Routen-Gruppe:

```rust
use suprnova::{Limit, RateLimiter, Router, ThrottleRequestsMiddleware};

RateLimiter::define("api", |req| {
    Limit::per_minute(60)
        .by(req.ip().unwrap_or_else(|| "anon".into()))
        .into()
});

let router = Router::new()
    .get("/api/items", list_items)
    .post("/api/items", create_item)
    .middleware(ThrottleRequestsMiddleware::by_name("api"));
```

### Schlüsseln auf `req.ip()`, nie auf den Header

`X-Forwarded-For` wird vom Aufrufer geliefert. Ein Limiter, der auf
den rohen Header geschlüsselt ist, wird ausgehebelt, indem bei jeder
Anfrage ein anderer Wert gesendet wird - der Angreifer sucht sich
seinen eigenen Bucket aus, sodass das Kontingent pro Anfrage statt
pro Client gilt.

`Request::ip()` ist der sichere Weg, das zu lesen - diese Methode
liefert `X-Forwarded-For` / `X-Real-IP` **nur, wenn der TCP-Peer in
`APP_TRUSTED_PROXIES` gelistet ist**, andernfalls die Peer-Adresse,
sodass ein Header von jedem außer Ihrem eigenen Proxy ignoriert
wird.

Die Umkehrung zählt genauso: Ist diese Variable ungesetzt - der
Standard -, liefert `req.ip()` hinter einem terminierenden Proxy bei
jeder Anfrage *die Adresse des Proxys*, und jedes Pro-IP-Limit in
der App fällt in einen einzigen gemeinsamen Bucket zusammen.
`ThrottleRequestsMiddleware::with(20, 1, "login")` bedeutet dann 20
Versuche pro Minute über alle Nutzer zusammen, was ein einzelner
Aufrufer ausgeben kann, um alle anderen auszusperren. Wer hinter
nginx, Traefik, einem ALB oder Cloudflare deployt, muss
[`APP_TRUSTED_PROXIES`](env-vars.md#behind-a-reverse-proxy-set-app_trusted_proxies)
setzen.

### Response-Header

Jede umschlossene Response trägt:

- `X-RateLimit-Limit` - die konfigurierten `max_attempts`.
- `X-RateLimit-Remaining` - verbleibende Wiederholungen für diesen
  Bucket.

429-Responses tragen zusätzlich:

- `Retry-After` - Sekunden, bis das Fenster wieder öffnet.
- `X-RateLimit-Reset` - Unix-Sekunden-seit-Epoch, wann der Bucket
  wieder öffnet.

Das entspricht exakt der Form von Laravels
`ThrottleRequests::getHeaders`.

### Fehlender benannter Limiter

Wenn eine Route mit `by_name("X")` verdrahtet ist, aber kein Limiter
unter `X` registriert wurde, liefert die Middleware HTTP 503 mit
einem Body, der den fehlenden Limiter benennt. Laravel wirft
`MissingRateLimiterException`; wir legen es als HTTP-Response offen,
damit ein fehlkonfigurierter Boot im Worker-Thread keinen Panic
auslöst.

### Treiber-vs-Facade-Komposition

Die beiden Middlewares können auf einem einzigen Router
koexistieren. Schichten Sie den Sliding-Window-Treiber für
Low-Level-Fairness, dann die Cache-gestützte Drosselung für benannte
Limits pro Endpunkt:

```rust
let router = Router::new()
    .get("/api/items", list_items)
    .middleware(RateLimitMiddleware::new(limiter_driver, cfg, key_fn))
    .middleware(ThrottleRequestsMiddleware::by_name("api"));
```

## Konfiguration

Die Treiber-SPI wird über Umgebungsvariablen konfiguriert; die
Cache-gestützte Facade wird konfiguriert, wo auch immer Ihr
[`Cache`](cache.md)-Store konfiguriert ist (Memory oder Redis).

| Variable | Verwendet von | Standard |
|----------|---------|---------|
| `RATE_LIMIT_DRIVER` | Treiber-SPI-Bootstrap | `memory` (in Produktion verweigert - siehe oben) |
| `RATE_LIMIT_ALLOW_MEMORY_IN_PRODUCTION` | Fail-Closed-Override für Produktion | nicht gesetzt |
| `RATE_LIMIT_REDIS_URL` | Redis-Treiber | `redis://127.0.0.1:6379` |
| `RATE_LIMIT_PREFIX` | Redis-Schlüsselpräfix | `suprnova:` |
| `CACHE_DRIVER` / `REDIS_URL` / `CACHE_DEFAULT_TTL` / `REDIS_PREFIX` | Cache-gestützte `RateLimiter`-Facade (siehe [`Cache`](cache.md)) | verschieden |

## Migration von Laravel

| Laravel | Suprnova |
|---------|----------|
| `RateLimiter::for('api', fn ($req) => Limit::perMinute(60))` | `RateLimiter::define("api", \|req\| Limit::per_minute(60).into())` oder `RateLimiter::r#for(...)` |
| `RateLimiter::hit($key, $decay)` | `RateLimiter::hit(key, decay).await?` |
| `RateLimiter::tooManyAttempts($key, $max)` | `RateLimiter::too_many_attempts(key, max).await?` |
| `RateLimiter::availableIn($key)` | `RateLimiter::available_in(key).await?` |
| `RateLimiter::attempt($key, $max, $cb, $decay)` | `RateLimiter::attempt(key, max, \|\| async { ... }, decay).await?` |
| `RateLimiter::retriesLeft($key, $max)` | `RateLimiter::retries_left(key, max).await?` |
| `RateLimiter::cleanRateLimiterKey($key)` | `RateLimiter::clean_rate_limiter_key(key)` |
| `Limit::perMinute(60)->by($ip)->response(fn () => abort(429))` | `Limit::per_minute(60).by(ip).response(\|_\| HttpResponse::text("...").status(429))` |
| `Limit::perMinutes(3, 100)` | `Limit::per_minutes(3, 100)` |
| `Limit::none()` | `Limit::none()` |
| `throttle:api`-Middleware | `ThrottleRequestsMiddleware::by_name("api")` |
| `throttle:60,1`-Middleware | `ThrottleRequestsMiddleware::with(60, 1, "")` |
| `X-RateLimit-Limit/Remaining/Reset` + `Retry-After`-Header | Dieselben Header, dieselbe Form |

### Warum Suprnova abweicht

Laravel liefert eine Form: `Illuminate\Cache\RateLimiter`
(Cache-gestützter Fixed-Window-Zähler) mit
`Illuminate\Routing\Middleware\ThrottleRequests` als HTTP-Wrapper.
Suprnova liefert sowohl diese Form *als auch* eine native
Sliding-Window-Treiber-SPI, weil zwei echte Fragen zwei echte
Antworten brauchen.

Ein Cache-gestützter Zähler ist die richtige Antwort auf „Ich habe
benannte Limiter, Response-Callbacks, After-Callbacks fürs Zählen
nur fehlgeschlagener Logins, und ich will quellcode-kompatibel mit
Laravel-Migrationen sein.“ Er ist die falsche Antwort auf „Ich
brauche exakte Ein-Slot-pro-Anfrage-Sliding-Window-Durchsetzung
gegen ein Redis-ZSET mit atomarem Lua-Eval und ohne separaten
Timer-Key.“ Diese zweite Frage ist es, die die meisten
Rust-Dienste, die an die Nebenläufigkeitsgrenzen von Tokio stoßen,
tatsächlich haben, daher existieren `RateLimiterDriver` +
`RateLimitMiddleware` daneben, nicht hinter einem Feature Flag.

Die Backend-Fehler-Richtlinie ist ebenfalls eine
Suprnova-Ergänzung. Die Middleware von Laravel legt eine
Entscheidung wie „der Limiter ist defekt“ nie offen, weil PHPs
Lebenszyklus pro Anfrage sie verdeckt - die nächste Anfrage bekommt
einen frischen Prozess. Ein lang laufender Tokio-Worker, der Redis
für zehn Sekunden verliert, muss entscheiden, was mit den Anfragen
passiert, die in diesem Fenster eintreffen; `BackendErrorPolicy::FailOpen`
(Standard) gegenüber `FailClosed` ist diese Entscheidung, explizit
offengelegt.

## Nächste Schritte

- [Middleware](middleware.md) - wie Middleware sich komponiert,
  läuft und in der Request-Chain kurzschließt
- [Cache](cache.md) - der Store, auf dem die Laravel-förmige
  `RateLimiter`-Facade aufgebaut ist
- [Konfiguration](configuration.md) - typisierte Config für die
  Cache- und Redis-Backends
- [Auth-Flows](auth-flows.md) - `LoginThrottleMiddleware` und das
  Brute-Force-Lockout-Muster bauen auf dieser Oberfläche auf
- [Fehlermodell](error-model.md) - warum `Result<HttpResponse,
  HttpResponse>` die Middleware sauber kurzschließen lässt
