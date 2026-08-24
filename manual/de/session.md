# Sitzungen

Die Session ist die Schlüssel/Wert-Bag pro Benutzer, die mehrere
Anfragen desselben Browsers überdauert. Suprnova bringt ab Werk einen
datenbankgestützten Treiber mit, verdrahtet ihn über
`SessionMiddleware` und legt die aktive Session über zwei freie
Funktionen offen - `session()` zum Lesen, `session_mut()` zum
Schreiben. Verwenden Sie sie immer dann, wenn ein Wert eine Anfrage
überleben soll, aber nichts sein sollte, was die URL oder ein JWT
trägt.

## Wie eine Anfrage die Session sieht

`SessionMiddleware` läuft bei jeder Anfrage und tut fünf Dinge in
dieser Reihenfolge:

1. Liest die Session-ID und den Zeitstempel des letzten erfolgreichen
   Aktivitäts-Touch aus dem Cookie `suprnova_session`
   (AES-256-GCM-verschlüsselt). Manipulierte, nicht entschlüsselbare
   oder fehlerhafte Cookies gelten als nicht vorhanden.
2. Lädt `SessionData` nur dann aus dem Store, wenn ein gültiges Cookie
eine Session benennt. Anfragen ohne Cookie starten mit einer sauberen
Session im Arbeitsspeicher und setzen keinen garantierten Fehlgriff
auf der Datenbank ab. Ein Cookie, dessen Zeile nicht mehr existiert,
wird gelöscht, ohne dass eine leere Zeile neu angelegt wird. Ein
Lesefehler des Stores protokolliert `warn!` und lässt eine
zustandsfreie Anfrage weiterlaufen, eine Änderung im Handler ist dann
aber Fail-Closed, statt unbekannten gespeicherten Zustand zu
überschreiben.
3. Lässt Flash-Daten altern: `_flash.old.*` wird verworfen,
   `_flash.new.*` wird in `_flash.old.*` umbenannt. Nach diesem Schritt
   ist alles lesbar, was die vorherige Anfrage geflasht hat; alles, was
   diese Anfrage flasht, wird beim nächsten Mal lesbar sein.
4. Bindet die Session für die Dauer des Handlers in einen
   Task-Local-Slot. `session()` und `session_mut()` schlagen in diesem
   Slot nach.
5. Nachdem der Handler zurückgekehrt ist: persistiert verschmutzten
   Session-Zustand oder einen frequenzbegrenzten Touch für den
   gleitenden Ablauf, hängt erst nach einem erfolgreichen
   Schreibvorgang ein ersetzendes verschlüsseltes Cookie an und leert
   die ausstehenden Out-of-Band-Cookies (zum Beispiel ein frisch
   rotiertes Remember-me-Cookie). Eine saubere Anfrage ohne Cookie
   macht kein I/O auf dem Session-Store und bekommt kein
   Session-Cookie.

Schritt 5 trägt eine Sicherheitsgarantie, die es sich herauszuheben
lohnt: **Wurde die Session in dieser Anfrage verändert und schlägt das
Schreiben in den Store fehl, wird die Response durch ein 500
ersetzt.** Den Erfolg des Handlers zurückzugeben hieße, dem Client ein
Cookie für Zustand auszuhändigen, den die Datenbank nie aufgezeichnet
hat - die nächste Anfrage würde eine leere Session laden, und die
Änderung (Login, CSRF-Rotation, Flash) verschwände stillschweigend.
Nur-Lese-Anfragen, die allein an einem fälligen `last_activity`-Touch
scheitern, protokollieren `warn!`, behalten das bestehende Cookie und
laufen durch.

## Die Session lesen

```rust
use suprnova::session::session;

if let Some(s) = session() {
    let user_id: Option<String> = s.get("preferred_username");
    if s.has("cart") {
        // ...
    }
    if s.missing("locale") {
        // erster Besuch
    }
}
```

`session()` klont die aktuelle `SessionData`. Liefert `None` außerhalb
eines Anfrage-Scopes (ein Unit-Test, der die Middleware nicht
installiert hat, ein CLI-Unterbefehl). Für einen typisierten Wert
deserialisiert `get::<T>` aus dem zugrunde liegenden JSON; bei
fehlendem Schlüssel oder falschem Typ bekommen Sie `None` und keinen
Panic.

## Die Session schreiben

`session_mut` nimmt eine Closure entgegen, die `&mut SessionData`
bekommt:

```rust
use suprnova::session::session_mut;

session_mut(|s| {
    s.put("locale", "en");
    s.put("preferences", serde_json::json!({
        "theme": "dark",
        "notifications": true,
    }));
    s.forget("legacy_key");
});
```

Die Closure ist synchron - die Guards auf dem darunterliegenden Lock
werden vor jedem `.await` freigegeben, das komponiert sich also
innerhalb von async-Handlern, ohne den Lock über Suspendierungen hinweg
zu halten. Alles, was Sie serialisieren, muss `Serialize`
implementieren; die Deserialisierung in `get` verlangt
`DeserializeOwned`.

Die Closure-Form (statt einen Guard zurückzugeben) ist Absicht. Futures
in Tokio können auf einem anderen Worker-Thread fortgesetzt werden als
dem, auf dem sie gestartet sind, also muss die Session in einem
`task_local!`-Slot leben und über einen an einen Scope gebundenen
kritischen Abschnitt geborgt werden. Die Form `|s|` macht diese Grenze
explizit und hindert Sie daran, versehentlich einen Mutex-Guard über
ein `.await` hinweg zu halten.

## Flash-Daten

Flash-Werte sind für **eine** nachfolgende Anfrage sichtbar und
verschwinden dann. Das übliche Muster: Ein Controller schreibt einen
Flash, liefert einen Redirect zurück, die nächste Seite rendert den
Flash.

```rust
use suprnova::session::session_mut;

session_mut(|s| s.flash("status", "Profile updated."));
```

Bei der nächsten Anfrage:

```rust
use suprnova::session::session_mut;

let status: Option<String> = session_mut(|s| s.get_flash("status"));
```

`get_flash` entfernt den Wert, während es ihn zurückgibt. Für die
Variante, die liest, ohne zu verbrauchen, nehmen Sie
`get::<String>("_flash.old.status")`, aber die verbrauchende Form ist
das, was Controller normalerweise wollen.

Die vollständige Flash-Oberfläche aus Laravel steht zur Verfügung:

- `flash(key, value)` - schreibt für die nächste Anfrage
- `now(key, value)` - schreibt nur für die aktuelle Anfrage
- `reflash()` - flasht alles gerade Sichtbare für eine weitere Runde
  erneut
- `keep(&["k1", "k2"])` - flasht eine bestimmte Teilmenge erneut
- `flash_input(map)` / `old_input()` / `get_old_input(key)` - die Bag
  für Formulareingaben, die `Redirect::with_input` und die
  `old()`-Helfer verwenden

## Regenerieren und invalidieren

Nach einer Änderung der Zugangsdaten (Login, Passwort-Reset,
bestandene 2FA) rotieren Sie die Session-ID, damit eine vor der
Änderung fixierte ID nicht mehr gültig ist:

```rust
use suprnova::session::{regenerate_session_id, regenerate_csrf_token};

regenerate_session_id();        // neue ID, dieselben Daten
regenerate_csrf_token();        // neuer CSRF-Token, dieselbe ID und Daten
```

Um die Session vollständig zu leeren (Logout):

```rust
use suprnova::session::invalidate_session;

invalidate_session();           // leert die Daten und prägt einen frischen CSRF-Token
```

Für ein Sicherheitsereignis, das jede Session eines Benutzers
widerrufen muss (Passwort-Reset anderswo, Kontowiederherstellung,
erzwungener Logout durch einen Admin):

```rust
use suprnova::session::destroy_all_for_user;

let rows = destroy_all_for_user("user-42").await?;
tracing::info!(revoked = rows, "all sessions destroyed");
```

`destroy_all_for_user` löst den durch `SessionMiddleware::new` oder
`with_store` registrierten `SessionStore` auf und ruft `destroy_for_user` auf
diesem konfigurierten Store auf. Es fällt nur dann auf einen neuen
`DatabaseSessionDriver` zurück, wenn kein Session-Store registriert wurde,
etwa in einem Test oder Embedder, der die Middleware nie konstruiert hat.

## Helfer für die Authentifizierung

`auth_user_id()` liefert die ID des aktuell authentifizierten Benutzers
(befragt zuerst den anfragebezogenen Auth-Zustand und fällt dann auf
das persistierte Session-Feld zurück):

```rust
use suprnova::session::{auth_user_id, is_authenticated};

if is_authenticated() {
    let uid = auth_user_id().expect("just checked");
    // ...
}
```

Normalerweise steuern Sie Auth über die
[Auth](authentication.md)-Facade - `Auth::login`, `Auth::logout`,
`Auth::user()`. Die Session-Helfer sind die untere Schicht, auf der
diese Facaden sitzen; greifen Sie zu ihnen, wenn Sie die rohe Session
inspizieren oder einen eigenen Guard implementieren wollen.

## Weitere Operationen

Die `SessionData`-API spiegelt Laravels `Store`-Oberfläche:

| Methode | Was sie tut |
|---|---|
| `get::<T>(key)` | typisiertes Lesen |
| `put(key, value)` | typisiertes Schreiben |
| `forget(key)` | einen einzelnen Schlüssel entfernen |
| `forget_many(&[..])` | mehrere Schlüssel entfernen |
| `flush()` | alle Daten leeren (behält die ID) |
| `has(key)` / `missing(key)` | Prüfung auf Vorhandensein |
| `has_any(&[..])` / `has_all(&[..])` | Vorhandensein in Menge |
| `all()` | die zugrunde liegende Map borgen |
| `only(&[..])` / `except(&[..])` | gefilterte Klone |
| `pull::<T>(key)` | holen und vergessen in einem Zug |
| `push(key, value)` | an einen Array-Wert anhängen |
| `increment(key, n)` / `decrement(key, n)` | Integer-Zähler |
| `remember::<T>(key, \|\| default())` | holen oder berechnen und ablegen |
| `replace(&[(k, v), ..])` | leeren, dann in Menge ablegen |
| `put_many(&[(k, v), ..])` | in Menge ablegen und zusammenführen |
| `previous_url()` / `set_previous_url(url)` | was `Redirect::back` liest |
| `password_confirmed()` / `password_confirmed_at()` | Zeitstempel für "Benutzer hat das Passwort gerade eben bestätigt" |

Greifen Sie für ändernde Operationen innerhalb von `session_mut` zu
ihnen, für Lesevorgänge über `session()`. Der `previous_url`-Slot wird
von der Middleware bei erfolgreichen GET-Responses mit HTML
automatisch gefüllt, `redirect()->back()` funktioniert also, ohne dass
Sie etwas tun. Die Middleware zeichnet nur eine root-relative,
same-origin URL auf: Ein Anfragepfad, der mit `//` oder `/\` beginnt
(beides liest ein Browser als protokollrelativ) oder irgendwo ein
ASCII-Steuerbyte enthält (ein `TAB` oder Newline lässt einen Wert, der
nur root-relativ aussieht, nach dem Entfernen durch den URL-Parser des
Browsers in eine dieser beiden Formen kippen), wird nie gespeichert.
`previous_url()` prüft dieselbe Regel bei jedem Lesen erneut, sodass
ein Wert aus einer älteren Version, bevor die Prüfung beim Schreiben
existierte, als nicht vorhanden zurückkommt, statt ihm zu vertrauen.
So können `Redirect::back()`, `Redirect::refresh()` und
`url::previous()` aus keinem Wert dieses Slots auf eine `Location`
außerhalb Ihrer App auflösen.


## Konfiguration

Konfigurieren Sie Sessions über Umgebungsvariablen -
`SessionConfig::from_env` liest sie beim Boot:

```env
# Lebensdauer in Minuten. Steuert sowohl die TTL der Zeile als auch das Max-Age des Cookies.
SESSION_LIFETIME=120

# Mindestabstand in Sekunden zwischen Schreibvorgängen des gleitenden Ablaufs (Standard 5 Minuten).
# Zur Laufzeit wird das auf einen Wert unterhalb der Session-Lebensdauer gedeckelt.
SESSION_TOUCH_INTERVAL=300

# Kadenz für das supervidierte Einsammeln abgelaufener Zeilen in Sekunden (Standard 1 Stunde).
SESSION_GC_INTERVAL=3600

# Cookie-Name auf dem Client.
SESSION_COOKIE=suprnova_session

# Cookie-Attribute
SESSION_SECURE=true          # HTTPS verlangen; STANDARD IST true
SESSION_PATH=/
SESSION_DOMAIN=.example.com  # optional; nicht gesetzt = nur der Host
SESSION_SAME_SITE=Lax        # Lax | Strict | None
SESSION_COOKIE_PREFIX=       # leer | __Secure- | __Host-

SESSION_PARTITIONED=false    # CHIPS-Opt-in
SESSION_EXPIRE_ON_CLOSE=false # true → Max-Age entfällt, Browser verwirft beim Schließen

# Benannte DB-Verbindung für den Session-Store (optional)
SESSION_CONNECTION=sessions

# Lebensdauer von Remember-me-Token und -Cookie in Minuten (Standard 30 Tage)
REMEMBER_LIFETIME=43200
```

Ein paar Standardwerte sind eine Erwähnung wert:

- **`SESSION_SECURE` steht standardmäßig auf `true`.** Sessions, die
  über einfaches HTTP gehen, wären ein Risiko für das Abfließen von
  Zugangsdaten, das Secure-Flag ist also standardmäßig gesetzt. Für die
  lokale Entwicklung über HTTP setzen Sie `SESSION_SECURE=false` in
  Ihrer lokalen `.env`.
- **`HttpOnly` ist immer an.** Es gibt keinen Schalter, um es
  abzuschalten - das Session-Cookie für JavaScript sichtbar zu machen
  verspielt den wichtigsten XSS-Schutz, und es gibt heute keinen
  legitimen Grund, das zu wollen.
- **`SameSite` steht standardmäßig auf `Lax`.** `Strict` blockiert die
  Session bei den meisten seitenübergreifenden GET-Navigationen (auch
  bei Rücklinks aus E-Mails); `Lax` ist üblicherweise die richtige
  Antwort.

### Cookie-Namenspräfix-Härtung

`SESSION_COOKIE_PREFIX=__Host-` bewirkt, dass der Browser die Session-
und Remember-me-Cookies an den Host bindet. Ein `__Host-`-Cookie muss
`Secure` sein, `Path=/` verwenden und `Domain` weglassen; ein
`__Secure-`-Cookie muss `Secure` sein. Suprnova erzwingt diese Regeln
zur Renderzeit anhand des finalen Cookie-Namens, sodass Builder-Reihenfolge
und eingereihte Cookies denselben Schutz erhalten.

`Config::init` validiert Präfix, `SESSION_DOMAIN` und `SESSION_PATH`
beim Boot und schlägt fehl, bevor ausgeliefert wird, wenn die Kombination
ungültig ist. Die Erzwingung zur Renderzeit setzt `Secure` für beide
Präfixe weiterhin fest und schreibt einen `__Host-`-Pfad auf `/` um; sie
entfernt eine `Domain` bei `__Host-` und protokolliert eine Warnung, weil
dadurch der angeforderte Geltungsbereich enger wird. Der Browser
verwirft ein ungültiges Präfix-Cookie still, prüfen Sie daher vor dem
Deployment die Boot-Diagnose.

Für die lokale HTTP-Entwicklung lassen Sie das Präfix leer und setzen
`SESSION_SECURE=false` nur in der lokalen Umgebung. Für Produktion
setzen Sie HTTPS ein, behalten `SESSION_SECURE=true`, verwenden
`SESSION_COOKIE_PREFIX=__Host-`, behalten `SESSION_PATH=/` und lassen
`SESSION_DOMAIN` nicht gesetzt.

Deployment-Checkliste:

1. Bestätigen Sie, dass der öffentliche Origin HTTPS ist, einschließlich
   Health-Checks und der ersten Weiterleitung.
2. Setzen Sie `SESSION_COOKIE_PREFIX=__Host-`, `SESSION_SECURE=true` und
   `SESSION_PATH=/`.
3. Entfernen Sie `SESSION_DOMAIN`; der Boot-Validator weist es mit
   `__Host-` zurück.
4. Prüfen Sie die erste `Set-Cookie`-Response auf
   `__Host-suprnova_session`, `Secure` und `Path=/`, ohne `Domain`.

### Warum Suprnova abweicht

Laravel stellt in seiner Session-Konfiguration keinen First-Class-Schalter
für Cookie-Präfixe bereit. Suprnova macht das Präfix zu einem
Konfigurationswert mit Boot-Validierung, weil der Fehlerfall im Browser
stumm ist: Ein ungültiges Cookie wird verworfen, bevor der Anwendungscode
einen Session-Fehler melden kann.

Für die programmatische Konfiguration nehmen Sie den Fluent-Builder:

```rust
use std::time::Duration;
use suprnova::SessionConfig;

let config = SessionConfig::new()
    .lifetime(Duration::from_secs(60 * 60))      // 1 Stunde
    .touch_interval(Duration::from_secs(5 * 60))
    .gc_interval(Duration::from_secs(60 * 60))
    .cookie_name("myapp_session")
    .secure(true)
    .domain(".example.com")
    .remember_lifetime(Duration::from_secs(30 * 24 * 60 * 60));
```
`SessionConfig` ist `#[non_exhaustive]`; verwenden Sie einen Standardwert
und setzen Sie das öffentliche Feld, wenn die programmatische
Konfiguration ein Präfix braucht:

```rust
use suprnova::{CookiePrefix, SessionConfig};

let mut config = SessionConfig::default();
config.cookie_prefix = CookiePrefix::Host;
```

## Alles verdrahten

`SessionMiddleware` wird im Bootstrap Ihrer App als globale Middleware
installiert. Die Reihenfolge der Middleware ist wichtig: Die Session
muss vor [CSRF](csrf.md) kommen, weil CSRF den Token pro Session liest.

```rust
use std::sync::Arc;
use suprnova::{global_middleware, CsrfMiddleware, SessionConfig, SessionMiddleware};

pub async fn bootstrap() {
    let config = SessionConfig::from_env();

    // `install` registriert auch den konfigurierten GC-Supervisor.
    // Nehmen Sie `SessionMiddleware::new(config)`, wenn Sie GC lieber
    // selbst über `Schedule` einplanen wollen.
    global_middleware!(SessionMiddleware::install(config).await);

    global_middleware!(CsrfMiddleware::new());
}
```

`SessionMiddleware::install` registriert eine gc-Task unter
[Supervision](supervisors.md), die `gc()` im Takt von
`SESSION_GC_INTERVAL` aufruft (standardmäßig einmal pro Stunde). Die
Variante `install_with_gc(config, interval).await` nimmt ein eigenes
Intervall; `new(config)` überspringt die gc-Task (nützlich, wenn Sie
`gc()` lieber aus einem Eintrag der [Task-Planung](scheduling.md)
heraus aufrufen). Die Task unter Supervision nimmt am Graceful Shutdown
des Frameworks teil, die gc-Schleife endet bei `Ctrl-C` / `SIGTERM`
also sauber, statt zwangsweise abgebrochen zu werden.

Geschützte Betriebs-Endpunkte können den Zustand des Collectors
offenlegen, ohne die Tabelle `sessions` abzufragen:

```rust
use suprnova::session::session_gc_metrics;

let metrics = session_gc_metrics();
tracing::info!(
    runs = metrics.runs,
    failures = metrics.failures,
    removed_rows = metrics.removed_rows,
    last_success = metrics.last_success_unix_seconds,
    "session collector status"
);
```

Um einen Store zu verwenden, der nicht auf der Datenbank sitzt - für
Tests oder für einen Redis-gestützten Treiber, den Sie selbst
schreiben - implementieren Sie `SessionStore` und übergeben ihn über
`with_store`:

```rust
use std::sync::Arc;
use suprnova::{SessionConfig, SessionMiddleware, SessionStore};

let store: Arc<dyn SessionStore> = Arc::new(MyRedisStore::new());
let mw = SessionMiddleware::with_store(SessionConfig::from_env(), store);
```

## Die Sessions-Tabelle

Der Standard-Treiber erwartet eine `sessions`-Tabelle in dieser Form
(die SeaORM-Entität in `framework/src/session/driver/database.rs` ist
die Quelle der Wahrheit):

| Spalte | Typ | Anmerkungen |
|---|---|---|
| `id` | VARCHAR PK | alphanumerische Session-ID, 40 Zeichen in Kleinbuchstaben |
| `user_id` | VARCHAR NULL | ID des authentifizierten Benutzers (String, erlaubt undurchsichtige IDs) |
| `payload` | TEXT | JSON-serialisierte Map der Session-Daten |
| `csrf_token` | VARCHAR | CSRF-Token pro Session |
| `last_activity` | TIMESTAMP | letzter Zugriff; steuert Ablauf und GC |

Neben der Tabelle kommen zwei Indizes mit: `idx_sessions_user_id` (für
`destroy_for_user`) und `idx_sessions_last_activity` (für `gc()`).

Eine per Scaffold erzeugte App enthält eine
`create_sessions_table`-Migration, die zu dieser Form passt. Wenn Sie
eigene Migrationen mitbringen, spiegeln Sie die Spaltennamen exakt -
SeaORM löst sie positionell auf, und eine umbenannte Spalte passt
nicht.

### Warum Suprnova abweicht

Zwei Stellen, an denen Laravel eine PHP-förmige Entscheidung getroffen
hat, die Tokio uns anders treffen lässt:

**Garbage Collection.** Laravel spielt bei jeder Anfrage eine
2/100-Lotterie: Jede Anfrage hat eine Chance von 2 %, die Session-GC
inline auszulösen. Auf PHP geht das auf, weil ohnehin jede Anfrage
einen frischen Prozess startet. Auf Tokio haben wir langlebige Worker,
also registriert `SessionMiddleware::install` eine Task unter
[Supervision](supervisors.md), die `gc()` in einem festen Intervall
aufruft. Kein Aufwand pro Anfrage, keine probabilistische
Überraschung - explizite Planung statt einer Lotterie, und die
Restart-Schleife des Supervisors fängt Panics ab, sodass ein einzelner
schlechter gc-Lauf den Daemon nicht umbringt.

**`session_mut` in Closure-Form.** Laravel reicht Ihnen
`$request->session()` und lässt Sie Methoden darauf aufrufen. Wir tun
das nicht, weil Handler in Suprnova Futures sind und ein Future auf
einem anderen Worker-Thread fortgesetzt werden kann als dem, auf dem es
gestartet ist. Die Session lebt in einem `task_local!`-Slot von Tokio,
geborgter Zugriff muss also innerhalb eines Scopes stattfinden. Die
Closure-Form macht diesen Scope explizit und verhindert statisch den
Fehler, einen Mutex-Guard über ein `.await` hinweg zu halten.

**Fail-Closed bei schmutzigen Schreibvorgängen.** Ein fehlgeschlagener
frequenzbegrenzter Aktivitäts-Touch protokolliert `warn!` und lässt
die Anfrage mit ihrem bestehenden Cookie durch (der für den Benutzer
sichtbare Zustand ist intakt). Ein fehlgeschlagener Schreibvorgang
einer *veränderten* Session - Login, Flash, CSRF-Rotation - liefert 500.
Dem Client stillschweigend ein Cookie für Zustand auszuhändigen,
den der Store nie aufgezeichnet hat, würde einen "erfolgreichen"
Login schon bei der nächsten Anfrage verschwinden lassen; besser,
den Fehlschlag sichtbar zu machen.

## Nächste Schritte

- [Authentifizierung](authentication.md) - `Auth::login`, Guards, die
  Kette der User Provider
- [Auth-Flows](auth-flows.md) - Passwort-Reset, 2FA,
  Brute-Force-Drosselung, Remember-me
- [CSRF](csrf.md) - wie der CSRF-Token der Session bei
  Schreibvorgängen geprüft wird
- [Middleware](middleware.md) - eigene Middleware schreiben, die die
  Session liest oder schreibt
- [Request-Lifecycle](lifecycle.md) - wo `SessionMiddleware` in der
  Kette sitzt
