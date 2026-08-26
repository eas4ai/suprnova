# Cache

Suprnova bringt eine Laravel-förmige `Cache`-Facade mit, die von einem
von zwei Treibern getragen wird - In-Memory oder Redis -, beim Boot
explizit über `CACHE_DRIVER` ausgewählt. Die Facade ist eine dünne
Schicht über einem `CacheStore`-Trait, sodass sich benutzerdefinierte
Backends genauso einbinden wie die eingebauten.

## Die Facade

```rust
use suprnova::Cache;
use std::time::Duration;

Cache::put("user:1", &user, Some(Duration::from_secs(3600))).await?;

let cached: Option<User> = Cache::get("user:1").await?;

if Cache::has("user:1").await? {
    // Treffer
}

Cache::forget("user:1").await?;
```

Jede Methode serialisiert an der Facade-Grenze über `serde_json`,
sodass jedes `T: Serialize + DeserializeOwned` einen Round-Trip
übersteht. Das Trait unter der Facade (`CacheStore`) sieht nur
undurchsichtige JSON-Zeichenketten.

## Bootstrap

Der Cache wird während des Treiber-Bootstrap-Schritts von
`Server::run()` gebunden (siehe [Request-Lifecycle](lifecycle.md)).
`Cache::bootstrap` liest die konfigurierte `CacheConfig` (oder baut
eine aus der Umgebung) und verzweigt anhand von `CacheConfig::driver`:

- `Memory` - bindet einen `InMemoryCache` mit dem konfigurierten
  Präfix und der Standard-TTL. Gelingt immer.
- `Redis` - verbindet sich mit `REDIS_URL` und bindet den
  resultierenden `RedisCache`. **Ist Fail-Closed**, wenn die URL nicht
  erreichbar ist. Es gibt keine stille Rückstufung auf Memory.

Worker (`queue:work`, `schedule:run`, `workflow:work`) durchlaufen
denselben Bootstrap, sodass ein Job, der `Cache::get` verwendet,
dasselbe Backend sieht wie der HTTP-Handler.

### Warum Suprnova abweicht

Laravels `cache.php`-Konfiguration wählt einen Standard-Store, und
Laravel wechselt in manchen Codepfaden stillschweigend zu `array`
(In-Process), wenn ein falsch konfiguriertes Backend fehlschlägt. Das
ist ein produktiver Standard für `php artisan tinker` und in
Produktion brandgefährlich - ein einzelner Redis-Fehltreffer ändert
stillschweigend die Garantien jeder Tag-Leerung und jedes
Lock-Erwerbs in der App.

Suprnova wählt den gegenteiligen Standard. `CACHE_DRIVER=memory` ist
explizit (und der Standard für `cargo run`), und `CACHE_DRIVER=redis`
gegen ein nicht erreichbares Redis liefert einen Fehler von
`Server::from_config`. Die Binary beendet sich mit einem Fehlercode
ungleich null und einer Meldung, die die Abhilfe erklärt;
supervisord/systemd sieht einen Boot-Fehlschlag statt einer halb
funktionierenden App.

## Konfiguration

| Env | Bedeutung | Standard |
|---|---|---|
| `CACHE_DRIVER` | `memory` oder `redis` | `memory` |
| `REDIS_URL` | Redis-URL (nur konsultiert, wenn `driver=redis`) | `redis://127.0.0.1:6379` |
| `REDIS_PREFIX` | Schlüsselpräfix, angewendet auf jede Store-Operation | `suprnova_cache:` |
| `CACHE_DEFAULT_TTL` | Standard-TTL in Sekunden für `Cache::put(None)`; `0` bedeutet kein Standard | `3600` |

Ein nicht gesetztes `CACHE_DRIVER` wird zu `Memory` geparst; jeder
andere Wert (ohne Unterscheidung von Groß-/Kleinschreibung, getrimmt),
der nicht `memory`/`in-memory`/`inmemory`/`redis` ist, liefert beim
Boot einen Fehler.

Sie können die Konfiguration auch programmatisch bauen, wenn Sie kein
Env-Parsing wollen:

```rust
use suprnova::{Config, CacheConfig, cache::CacheDriver};

Config::register(
    CacheConfig::builder()
        .driver(CacheDriver::Redis)
        .url("redis://cache.internal:6379")
        .prefix("myapp:")
        .default_ttl(7200)
        .build(),
);
```

`CacheConfigBuilder::build` ist deterministisch - nicht gesetzte Felder
fallen auf `CacheConfig::default()` zurück, statt die Umgebung erneut
zu lesen.

### Der `forever`-Vertrag gilt backend-übergreifend

`Cache::forever` und `Cache::remember_forever` umgehen
`CACHE_DEFAULT_TTL` vollständig; der Wert läuft nie ab, unabhängig vom
konfigurierten Standard. `Cache::put(key, value, None)` wendet den
Standard tatsächlich an - das ist der Sinn, einen zu haben.

Die Auflösung der Standard-TTL passiert auf der Facade-Schicht. Beide
`CacheStore`-Backends respektieren `None` wörtlich an der
Store-Grenze (kein Ablauf), weshalb `forever` auf Memory wie auf Redis
tatsächlich für immer bedeutet.

## Lesen, Schreiben, Löschen

```rust
use suprnova::Cache;
use std::time::Duration;

// Schreiben mit expliziter TTL
Cache::put("session:42", &session, Some(Duration::from_secs(1800))).await?;

// Für immer schreiben - umgeht CACHE_DEFAULT_TTL
Cache::forever("config:features", &features).await?;

// Lesen (None bei Fehltreffer oder Ablauf)
let session: Option<Session> = Cache::get("session:42").await?;

// Existenz - true bedeutet vorhanden und nicht abgelaufen
if Cache::has("session:42").await? { /* … */ }

// Laravel-benannte Verneinung
if Cache::missing("session:42").await? { /* warm */ }

// Lesen-und-Löschen in einem Aufruf
let one_shot: Option<String> = Cache::pull("notice:welcome:42").await?;

// Gibt true zurück, wenn der Schlüssel existierte und entfernt wurde
Cache::forget("session:42").await?;

// Alles auslöschen (präfixbeschränkt auf beiden Backends)
Cache::flush().await?;
```

`Cache::pull` ist **nicht** atomar - es ist ein `get`, gefolgt von
einem `forget`, dieselbe Form wie Laravels `Repository::pull`. Für
atomares Dequeue verwenden Sie `Cache::lock` (siehe unten).

### Eine TTL auffrischen, ohne neu zu schreiben

```rust
let refreshed = Cache::touch("session:42", Duration::from_secs(1800)).await?;
```

`touch` gibt `true` zurück, wenn der Schlüssel existierte und die TTL
verlängert wurde, sonst `false`. Der gespeicherte Wert bleibt
unberührt.

## Add - schreibe, falls abwesend (atomar)

```rust
let won = Cache::add(
    "daily:winner",
    &user_id,
    Some(Duration::from_secs(86_400)),
).await?;
if won {
    send_winner_email(user_id).await?;
}
```

`Cache::add` schreibt nur, wenn der Schlüssel leer ist (oder
abgelaufen ist). Gibt `true` bei Schreiben zurück, `false` bei
Konflikt. **Atomar** auf beiden eingebauten Backends:

- `InMemoryCache` hält einen Write-Lock über die
  Existenzprüfung-plus-Insert hinweg
- `RedisCache` verwendet `SET key value NX EX ttl` (oder `NX` ohne `EX`)

Benutzerdefinierte `CacheStore`-Implementierungen, die `add_raw` nicht
überschreiben, fallen auf ein nicht-atomares Check-dann-Put zurück,
passend zu Laravels `Repository::add`-Fallback für Stores ohne
natives `add`.

## Remember - Get-oder-Berechnen

```rust
let user = Cache::remember(
    "user:1",
    Some(Duration::from_secs(3600)),
    || async { User::find(1).await },
).await?;

let cfg = Cache::remember_forever("config:app", || async {
    load_config_from_db().await
}).await?;
```

`remember` ruft Ihre Closure nur bei einem Fehltreffer auf und
speichert dann das Ergebnis. Die Closure gibt `Result<T,
FrameworkError>` zurück, sodass Domänenfehler über `?` durchschlagen,
statt den Cache zu vergiften.

`Cache::sear(key, default)` ist der Laravel-benannte Alias für
`remember_forever`. Derselbe Rumpf, dieselbe Semantik - läuft unter
beiden Namen, damit migrierter Code sich gleich liest.

### Remember ist NICHT stampede-sicher

`remember` ist ein nicht-atomares `get`-dann-`put`-Paar. N
gleichzeitige Fehltreffer für denselben kalten Schlüssel führen die
Closure N-mal aus und schreiben N Ergebnisse. Das entspricht genau
Laravels `Repository::remember`, und es ist für den üblichen Fall in
Ordnung (die Closure ist idempotent, die Schreibvorgänge sind
identisch).

Es ist nicht in Ordnung, wenn:

- Die Closure teuer ist (1 s+ zum Berechnen oder trifft ein langsames
  Upstream-System)
- Der Schlüssel populär genug ist, dass ein Cache-Kaltstart-Ereignis
  N Anfragen gleichzeitig zum zugrunde liegenden Store schickt
- Die Closure Nebeneffekte über das Berechnen des Werts hinaus hat

Umschließen Sie diese Fälle mit `Cache::lock`:

```rust
use suprnova::Cache;
use std::time::Duration;

let key = "rebuild:user:1";

if let Some(guard) = Cache::lock(key, Duration::from_secs(10)).await? {
    let user = Cache::remember(
        "user:1",
        Some(Duration::from_secs(3600)),
        || async { User::find(1).await },
    ).await?;
    guard.release().await?;
    return Ok(user);
}

// Das Rennen verloren - der Gewinner berechnet gerade. Lesen Sie, was
// auch immer er geschrieben hat, oder fallen Sie auf einen veralteten
// Wert zurück.
let user = Cache::get::<User>("user:1").await?
    .ok_or_else(|| FrameworkError::internal("cache miss after losing rebuild lock"))?;
```

## Sperren

`Cache::lock` gibt einen `LockGuard` zurück, der das
Besitz-Token hält. Sperren sind unverbindlich und prozessübergreifend,
wenn sie von Redis getragen werden.

```rust
use suprnova::Cache;
use std::time::Duration;

if let Some(guard) = Cache::lock("job:42", Duration::from_secs(30)).await? {
    do_exclusive_work().await?;
    guard.release().await?;
}
// Some(guard) bedeutet, wir besitzen sie. None bedeutet, ein anderer Halter war schneller.
```

Der Guard stellt bereit:

| Methode | Wofür |
|---|---|
| `guard.token()` | Das Besitz-Token lesen (Rust-seitiger Name) |
| `guard.owner()` | Derselbe Wert, Laravel-benannter Alias |
| `guard.refresh(ttl)` | Die TTL verlängern - gibt `false` zurück, wenn wir die Sperre nicht mehr besitzen |
| `guard.release()` | Freigeben, falls wir die Sperre noch besitzen - gibt `false` zurück, wenn das Token nicht mehr passt |

Es gibt absichtlich **kein automatisches `Drop`-Release**. Eine
Redis-Sperre muss über Prozessgrenzen hinweg bestätigt werden;
Auto-Release beim Drop würde entweder stillschweigend eine gestohlene
Sperre zurückstehlen (falsch) oder Release-Fehlschläge in
Destruktor-Panics verstecken (schlimmer). Das Release ist explizit,
damit Fehler propagieren.

`refresh` lässt einen lang laufenden Job seine eigene Sperre
verlängern, um ein selbstverschuldetes Timeout zu vermeiden - siehe
[Idempotenz](idempotency.md) für den Konsumenten im Baum.

## Atomare Zähler

```rust
// Initialisiert auf 0, falls abwesend, und inkrementiert dann. Gibt den neuen Wert zurück.
let visits = Cache::increment("page:visits", 1).await?;

// Dieselbe Form für negative Schritte
let remaining = Cache::decrement("quota:remaining", 1).await?;

// Benutzerdefinierter Betrag
let total = Cache::increment("stats:downloads", 10).await?;
```

Atomar auf beiden eingebauten Backends: `InMemoryCache` verwendet ein
schreibgesperrtes `HashMap::entry`; `RedisCache` verwendet
`INCRBY`/`DECRBY`. Der gespeicherte Wert ist ein JSON-kodierter
Integer, sodass `Cache::get::<i64>("page:visits")` mit demselben
Schlüssel einen Round-Trip übersteht.

## Getaggter Cache

Mit Tags können Sie eine ganze Familie verwandter Einträge mit einem
Aufruf invalidieren. Der klassische Anwendungsfall sind
Pro-Ressource-Caches, die gemeinsam geleert werden müssen, wenn sich
die Ressource ändert.

```rust
use suprnova::Cache;
use std::time::Duration;

// Unter einem oder mehreren Tags speichern
Cache::tags_put(
    &["users", "user:1"],
    "user:1:profile",
    &profile,
    Some(Duration::from_secs(3600)),
).await?;

Cache::tags_put(
    &["users", "user:1"],
    "user:1:posts",
    &posts,
    Some(Duration::from_secs(600)),
).await?;

// Update-Pfad: jeden mit `user:1` getaggten Schlüssel entfernen
Cache::flush_tags(&["user:1"]).await?;
```

Tag-Mitgliedschaft ist **pro Eintrag**: Jeder getaggte Schreibvorgang
installiert das Tag-Set dieses Schreibvorgangs als Quelle der Wahrheit
des Eintrags und ersetzt damit alle vorherigen Tags. Zwei
Konsequenzen, die man kennen sollte:

- Ein ungetaggtes `Cache::put` über einen zuvor getaggten Schlüssel
  **löscht** die Tags des Eintrags. Ein nachfolgendes `flush_tags` des
  alten Tags löscht den lebenden ungetaggten Wert nicht.
- Wenn Sie `tags_put(&["a"], …)` mit `tags_put(&["b"], …)`
  überschreiben, reagiert der Eintrag nur noch auf
  `flush_tags(&["b"])`.

Veraltete Vorwärtsindex-Verweise werden während des Leerungsdurchlaufs
und bei `flush()` bereinigt, sodass sie sich für Tags, die
geschrieben, aber nie geleert werden, nicht unbegrenzt ansammeln.

## Zwei Backends

| Merkmal | `InMemoryCache` | `RedisCache` |
|---|---|---|
| Prozessübergreifend geteilt | Nein | Ja |
| Persistenz | Nein | Ja, wenn Redis dafür konfiguriert ist |
| Atomares `add` | Ja (Schreibsperre) | Ja (`SET NX`) |
| Atomares `increment`/`decrement` | Ja (Schreibsperre) | Ja (`INCRBY`/`DECRBY`) |
| Getaggter Cache | Ja | Ja |
| Sperren | Ja | Ja (prozessübergreifend) |
| TTL unter einer Sekunde | Ja (`tokio::time::Instant`) | Ja (`PX`/`PEXPIRE`) |
| Ausgewählt über | `CACHE_DRIVER=memory` (Standard) | `CACHE_DRIVER=redis` |

Es gibt keinen Datenbank-Cache-Treiber - die beiden Backends oben sind
die, die das Framework ausliefert. Eigene Backends können `CacheStore`
implementieren und sich direkt in den Container binden; siehe das Muster
zur Test-Injektion weiter unten.

### In-Memory-Ablauf

`InMemoryCache` räumt abgelaufene Einträge **träge beim Lesen** aus:
`get_raw`, `has` und `add_raw` entfernen einen Eintrag, sobald sie ihn
zum ersten Mal als abgelaufen sehen. Bei erneut angefragten Schlüsseln
sammeln sich also nie Leichen an.

Eine Last, die einen hochkardinalen Satz kurzlebiger Schlüssel schreibt
und sie nie zurückliest, hat diesen Auslöser nicht. Rufen Sie in diesem
Fall `InMemoryCache::purge_expired()` aus einer periodischen Aufgabe auf -
es gibt die Anzahl der entfernten Einträge zurück. Redis erledigt seinen
Ablauf serverseitig selbst; dort braucht es die Entsprechung nicht.

### Redis-TTL-Genauigkeit

Jede Redis-TTL läuft über `PX` / `PEXPIRE`, nicht über `EX` / `EXPIRE`.
Das vermeidet zwei Fallstricke:

- `Duration`s unter einer Sekunde würden unter `EX` auf `0 seconds`
  abgeschnitten, was Redis ablehnt (`SET … EX 0`) oder, schlimmer, als
  „lösche den Schlüssel“ auslegt (`EXPIRE key 0`).
- `Duration::ZERO` wird vor dem Aufruf auf 1 ms angehoben, sodass keiner
  der beiden Ablehnungspfade aus Nutzercode erreichbar ist.

### Wiederholungen bei transienten Befehlen

Ein abgerissener Socket ließ früher genau das `Cache::get` scheitern, das
gerade unterwegs war. Der Redis-Connection-Manager verbindet sich von
selbst neu, aber der Befehl, der den toten Socket erwischt hat, gibt
Ihnen trotzdem seinen Fehler zurück.

Lesende Befehle wiederholen jetzt einmal: `GET`, `EXISTS` sowie die
`SCAN`- / `SSCAN`-Seiten hinter `Cache::flush` und `Cache::flush_tags`.
Die Lesezugriffe `XLEN`, `ZCARD` und `XPENDING` des Queue-Treibers und
die `Retry-After`-Berechnung der Ratenbegrenzung wiederholen genauso.
Setzen Sie `REDIS_COMMAND_RETRIES`, um über die eingebaute Wiederholung
hinaus weitere hinzuzufügen.

Rechnen Sie die Wiederholung in Sekunden, nicht in der Pause von 50 ms,
die ihr vorausgeht. Ist eine Verbindung erst einmal abgerissen, wartet
der nächste Versuch auf die Ersatzverbindung, bevor er überhaupt etwas
senden kann, er zahlt also das gesamte Verbindungsbudget des Treibers und
danach dessen Antwort-Timeout:

- Der Cache-Treiber erlaubt bis zu 3 Verbindungsversuche im Abstand von
  höchstens 500 ms, jeder gedeckelt durch ein Verbindungs-Timeout von
  2 s, mit einem Antwort-Timeout von 5 s.
- Die Queue- und Rate-Limit-Treiber übernehmen die Standardwerte von
  redis-rs: bis zu 6 Verbindungsversuche mit ungedeckelter exponentieller
  Verzögerung ab 100 ms, jeder gedeckelt durch ein Verbindungs-Timeout
  von 1 s, mit einem Antwort-Timeout von 500 ms.

`REDIS_COMMAND_RETRIES` ist auf 10 gedeckelt, und diese Deckelung
begrenzt Versuche, nicht Sekunden: Beim Maximum macht ein einzelnes Lesen
12 Versuche, was gegen ein ausgefallenes Redis Dutzende Sekunden bis
Minuten in einem Aufruf bedeutet. Ein Befehl, der ins Timeout läuft, gilt
ebenso als transient wie ein abgerissener, ein bloß langsames Redis lässt
also jedes umschlossene Lesen bis zu so viele Befehle absetzen statt
eines. Erhöhen Sie die Einstellung nur dort, wo der Aufrufer sich das
Warten leisten kann.

Schreibzugriffe wiederholen nie, bei keiner Einstellung. Ein transienter
Fehler bedeutet, dass die Verbindung ausgefallen ist, nicht, dass der
Server den Befehl abgelehnt hätte - er hat ihn womöglich bereits
ausgeführt -, ein `SET`, ein `INCR`, ein Sperrerwerb, ein Treffer der
Ratenbegrenzung oder ein Queue-Pop riskiert bei einer Wiederholung also
eine zweite Ausführung. Diese Befehle reichen den Fehlschlag an Sie
weiter, und Ihre Entscheidung über eine Wiederholung ist die informierte.

### Warum Suprnova abweicht

Laravels Config `command_retries` erhöht das Wiederholungsbudget für
jeden Redis-Befehl, denn seine Methode `command()` ist ein einziger
Engpass, der weiß, welchen Befehl er gerade ausführt, und eine
Nur-Lese-Allowlist mit 60 Einträgen heranzieht. Suprnovas Treiber rufen
typisierte Befehle direkt auf, damit wird die Allowlist zu einer
Entscheidung pro Aufrufstelle, und `REDIS_COMMAND_RETRIES` kann nur die
Wiederholungen für Befehle vertiefen, die ohnehin gefahrlos wiederholbar
sind. Es gibt keine Einstellung, die ein Queue-Pop wiederholen lässt.

## Testen

Binden Sie einen `InMemoryCache` in den `TestContainer`, und die Facade
löst ihn wie jeden anderen Store auf:

```rust
use std::sync::Arc;
use suprnova::{Cache, CacheStore, InMemoryCache};
use suprnova::container::testing::TestContainer;

#[tokio::test]
async fn cache_round_trips() {
    let _guard = TestContainer::fake();
    TestContainer::bind::<dyn CacheStore>(Arc::new(InMemoryCache::new()));

    Cache::put("k", &"v", None).await.unwrap();

    let v: Option<String> = Cache::get("k").await.unwrap();
    assert_eq!(v.as_deref(), Some("v"));
}
```

`TestContainer::bind` schreibt in den thread-lokalen Scope, sodass
parallele Tests keinen Cache-Zustand ineinander auslaufen lassen. Das
Kapitel [Service Container](container.md) beschreibt das dreischichtige
Lookup-Modell.

### Live-Redis-Suites

Die Redis-Tests des Frameworks sind `#[ignore]`d, sodass `cargo test` nie
einen Server braucht. Führen Sie sie mit `-- --ignored` aus und richten
Sie sie auf eine Instanz:

- `cache_redis_integration` liest `CACHE_REDIS_TEST_URL` und fällt auf
  `REDIS_URL` und dann auf `redis://127.0.0.1:6379` zurück. Jeder Test
  grenzt sich auf ein eindeutiges Schlüssel-Präfix ein und ist damit
  gegenüber einem geteilten Entwicklungs-Redis unbedenklich.
- `cache_redis_retry` deckt die Wiederholung transienter Befehle ab und
  verlangt `CACHE_REDIS_TEST_URL` ausdrücklich, ohne Fallback. Er setzt
  `CLIENT KILL TYPE normal` ab, was jeden anderen Client auf der Instanz
  trennt, ihm muss also ein Wegwerf-Server gegeben werden. Ist die
  Variable nicht gesetzt, gibt er eine Skip-Zeile aus und besteht, ohne
  sich zu verbinden.

## Muster

Ein paar wiederkehrende Formen, die es wert sind, benannt zu werden:

```rust
// Hierarchische, doppelpunktgetrennte Schlüssel - dieselbe Konvention, die Laravel verwendet
Cache::put("users:1:profile", &profile, None).await?;
Cache::put("posts:123:comments:count", &count, None).await?;

// TTL nach Datenvolatilität
Cache::put("stats:active", &count, Some(Duration::from_secs(60))).await?;
Cache::put("config:features", &features, Some(Duration::from_secs(3600))).await?;
Cache::forever("translations:en", &translations).await?;

// Cache-nach-Tag-Invalidierung rund um einen Schreibvorgang
async fn update_user(id: i64, data: UserUpdate) -> Result<User, FrameworkError> {
    let user = User::update(id, data).await?;
    Cache::flush_tags(&[&format!("user:{}", id)]).await?;
    Ok(user)
}
```

## Nächste Schritte

- [Konfiguration](configuration.md) - wie `Config::register` und
  Env-Variablen sich kombinieren
- [Ratenbegrenzung](rate-limiting.md) - die Laravel-förmige
  `RateLimiter`-Facade ist auf `Cache` aufgebaut
- [Idempotenz](idempotency.md) - die Request-Dedupe-Middleware
  verwendet `Cache::lock` durchgängig
- [Service Container](container.md) - wie `CacheStore` gebunden und
  aufgelöst wird
- [Fehlermodell](error-model.md) - was `Cache::*` zurückgibt, wenn
  Redis mitten in einer Anfrage nicht erreichbar ist
