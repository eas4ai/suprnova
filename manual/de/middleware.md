# Middleware

Middleware umschließt einen Request-Handler. Sie läuft, bevor der
Handler die Anfrage sieht, und noch einmal, nachdem der Handler eine
Response zurückgegeben hat - also ist sie der richtige Ort für
querschnittliche Arbeit: Auth, Logging, CORS, Throttling, Zeitmessung,
das Umformen von Anfrage oder Response. Suprnovas Oberfläche ist
dieselbe, die Laravel-Nutzer bereits kennen: eine
`handle(request, next)`-Methode, die entscheidet, ob sie die Anfrage
weiterleitet, per Short-Circuit abbricht oder die Response auf dem Weg
zurück verändert.

## Der Trait

Eine Middleware ist eine Struktur, die `Middleware` implementiert:

```rust
use suprnova::{async_trait, HttpResponse, Middleware, Next, Request, Response};

pub struct LoggingMiddleware;

#[async_trait]
impl Middleware for LoggingMiddleware {
    async fn handle(&self, request: Request, next: Next) -> Response {
        // Vorverarbeitung: läuft vor dem Handler.
        println!("--> {} {}", request.method(), request.path());

        // Leitet an die nächste Middleware weiter (oder an den Handler,
        // falls dies die letzte Schicht ist).
        let response = next(request).await;

        // Nachverarbeitung: läuft, nachdem der Handler zurückkehrt.
        println!("<-- complete");

        response
    }
}
```

`handle` hat drei mögliche Aufgaben, und Sie müssen bei einer gegebenen
Anfrage nur eine davon erledigen:

- **Weiterleiten.** Rufen Sie `next(request).await` auf, um die
  Kontrolle an die nächste Schicht zu übergeben. Die zurückgegebene
  `Response` ist das, was jede darüberliegende Schicht sehen wird.
- **Short-Circuit.** Geben Sie `Err(HttpResponse::...)` zurück, ohne
  `next` aufzurufen. Das Framework kollabiert beide Arme von `Response`
  (`Result<HttpResponse, HttpResponse>`) zu einer einzigen Response -
  ein `Err` ist eine Response, kein Absturz. Siehe
  [Fehlermodell](error-model.md).
- **Verändern.** Verändern Sie die Anfrage vor der Weiterleitung, oder
  verändern Sie die Response danach.

`Next` ist `Arc<dyn Fn(Request) -> MiddlewareFuture + Send + Sync>` -
behandeln Sie es wie eine asynchrone Funktion von `Request` nach
`Response`.

## Einen Stub generieren

Die CLI generiert eine funktionierende Middleware-Datei:

```bash
suprnova make:middleware Auth         # → src/middleware/auth.rs (AuthMiddleware)
suprnova make:middleware RateLimit    # → src/middleware/rate_limit.rs
suprnova make:middleware CorsMiddleware  # Suffix "Middleware" ist unproblematisch, gleiches Ergebnis
```

Die generierte Datei ist kein TODO-Stub - es ist eine echte Middleware,
die die umschlossene Anfrage zeitmisst und die eingehenden/ausgehenden
Ereignisse mit der von `RequestIdMiddleware` installierten
Per-Request-ID protokolliert. Ersetzen Sie den Body durch das, was Sie
tatsächlich brauchen.

## Middleware registrieren

Drei Stellen, an denen Sie sie installieren können, je nach
Geltungsbereich:

### Global

Läuft bei jeder Anfrage, in Registrierungsreihenfolge. Verwenden Sie
das `global_middleware!`-Makro innerhalb von `bootstrap()`:

```rust
// src/bootstrap.rs
use suprnova::{global_middleware, FrameworkError};
use crate::middleware;

pub async fn bootstrap() -> Result<(), FrameworkError> {
    global_middleware!(middleware::LoggingMiddleware);
    global_middleware!(middleware::CorsMiddleware);
    Ok(())
}
```

`global_middleware!(M)` expandiert zu `register_global_middleware(M)`.
Die Registrierung ist **idempotent pro konkretem Typ** - dieselbe
Struktur zweimal zu registrieren behält die erste Registrierung bei und
gibt ein Debug-Log aus. Das macht ein erneutes Durchlaufen des Boots
(Tests, Hot-Reload, mehrere `Server`-Instanzen in einem Prozess) sicher.
Um mehrere Kopien desselben Verhaltens mit unterschiedlicher
Konfiguration zu installieren, wickeln Sie jede in einen eigenen
Newtype ein.

### Pro Route

Verketten Sie `.middleware(M)` an eine Routendefinition aus dem
`routes!`-Makro:

```rust
// src/routes.rs
use suprnova::{routes, get};
use crate::{controllers, middleware::AuthMiddleware};

routes! {
    get!("/", controllers::home::index).name("home"),
    get!("/public", controllers::home::public),

    get!("/protected", controllers::dashboard::index)
        .middleware(AuthMiddleware),
    get!("/admin", controllers::admin::index)
        .middleware(AuthMiddleware),
}
```

### Pro Gruppe

Wenden Sie Middleware auf jede Route in einem `group(...)`-Block an:

```rust
use suprnova::Router;
use crate::middleware::{ApiMiddleware, AuthMiddleware};
use crate::controllers::{user, admin};

Router::new()
    // Öffentliche Routen - keine Middleware.
    .get("/", home_handler)
    .get("/login", login_handler)

    // Jede Route unter /api trägt ApiMiddleware.
    .group("/api", |r| {
        r.get("/users", user::index)
         .post("/users", user::store)
         .get("/users/{id}", user::show)
    })
    .middleware(ApiMiddleware)

    // Admin-Routen teilen sich Auth.
    .group("/admin", |r| {
        r.get("/dashboard", admin::dashboard)
         .get("/settings", admin::settings)
    })
    .middleware(AuthMiddleware);
```

## Ausführungsreihenfolge

Zur Laufzeit läuft die Chain von außen nach innen:

```
Request  →  RequestId  →  Globals  →  Gruppen-MW  →  Routen-MW  →  Handler
                                                                     │
Response ←  RequestId  ←  Globals  ←  Gruppen-MW  ←  Routen-MW  ←  Handler
```

Die zuerst hinzugefügte Middleware läuft zuerst. Auf dem Weg zurück
kehrt sich die Reihenfolge um - `MiddlewareChain::execute` verschachtelt
die Nachverarbeitung jeder Schicht innerhalb der vorherigen.

Löst eine Middleware einen Short-Circuit mit `Err(response)` aus,
wickelt sich die Chain sofort ab: Jede Schicht OBERHALB des
Short-Circuits sieht die Response auf dem Weg zurück trotzdem, aber
Schichten DARUNTER (näher am Handler) laufen nicht.

### Gruppen-Middleware wird flach kopiert, nicht gestapelt

Das hier ist wichtig und es lohnt sich, es hervorzuheben.
**Routen-Gruppen-Middleware ist keine eigene Laufzeitschicht.** Wenn
`GroupBuilder::try_finalize` läuft, kopiert es die Middleware der
Gruppe in die `(method, pattern)`-Middleware-Liste jeder gruppierten
Route. Zur Ausführungszeit ist Gruppen-Middleware nicht mehr von
Middleware zu unterscheiden, die direkt an die Route angehängt wurde.

Zwei Konsequenzen:

- Die Laufzeit-Reihenfolge bleibt korrekt (Gruppen-Middleware läuft vor
  Routen-Middleware, weil sie zuerst registriert wird), aber
  **Introspektion kann Gruppen- nicht von Routen-Middleware
  unterscheiden**.
- Middleware wird nach dem gematchten Pattern (`"/posts/{id}"`)
  indiziert, nicht nach dem rohen Pfad (`/posts/42`), sodass
  Gruppen-Middleware auf parametrisierten Routen zuverlässig greift.

Siehe `framework/src/routing/group.rs` für den Flattening-Durchlauf und
`framework/src/middleware/chain.rs` für die Ausführungsschleife.

## Short-Circuiting

Brechen Sie früh ab, um eine Anfrage zu blockieren, bevor sie den
Handler erreicht:

```rust
use suprnova::{async_trait, HttpResponse, Middleware, Next, Request, Response};

pub struct RequireApiKey;

#[async_trait]
impl Middleware for RequireApiKey {
    async fn handle(&self, request: Request, next: Next) -> Response {
        if request.header("X-Api-Key").is_none() {
            return Err(HttpResponse::text("Unauthorized").status(401));
        }
        next(request).await
    }
}
```

Die Chain kollabiert `Result<HttpResponse, HttpResponse>` zu einer
einzigen Response, sodass `Err(...)` einfach eine Response mit einer
anderen Rolle ist. Die Schichten oberhalb dieser Middleware beobachten
sie auf dem Weg zurück trotzdem und können sie nachbearbeiten.

## Panic-Sicherheit

`MiddlewareChain::execute` fängt KEINE Panics ab - ein Panic in
irgendeiner Middleware oder im Handler wickelt sich direkt nach außen ab,
wie jede andere asynchrone Funktion. Das Sicherheitsnetz für den
Request-Pfad liegt eine Ebene höher, an der Server-Grenze in
`execute_chain_safely`, das die Chain in `catch_unwind` einwickelt und
einen Panic in einen bereinigten 500 mit der Request-ID umwandelt, wobei
es `ErrorOccurred` für jeden Observability-Listener dispatcht. Siehe
[Request-Lifecycle](lifecycle.md) für den vollständigen
Panic-Recovery-Ablauf.

Diese Trennung ist beabsichtigt: standardisierte Panic-Behandlung
geschieht genau einmal, dort, wo der Request-Lifecycle sie besitzt,
statt innerhalb der schichtunabhängigen Primitive dupliziert zu werden.
Ein Aufrufer, der eine Chain außerhalb dieser Grenze antreibt, ist für
sein eigenes `catch_unwind` verantwortlich.

## Eingebaute Middleware

Eine nicht abschließende Übersicht. Jede kommt installationsbereit -
die meisten brauchen eine Config-Struktur, keine braucht Scaffolding.

| Middleware | Zweck |
|---|---|
| `RequestIdMiddleware` | Immer die äußerste Schicht; vergibt pro Anfrage eine UUID und markiert sie durch Logs + `X-Request-Id` |
| `TimeoutMiddleware` | Begrenzt die Zeit bis zur Response; liefert 503 bei Überschreitung (siehe unten) |
| `CorsMiddleware` | Behandelt CORS-Preflight + versieht Cross-Origin-Responses mit Headern (siehe unten) |
| `CsrfMiddleware` | Cookie-Double-Submit-CSRF-Schutz mit konfigurierbarer `OriginPolicy` |
| `RateLimitMiddleware` / `ThrottleRequestsMiddleware` | Token-Bucket- und Sliding-Window-Throttling; siehe [Ratenbegrenzung](rate-limiting.md) |
| `SessionMiddleware` | Lädt/persistiert die Session über Cookies; treibt `req.session()` an |
| `AuthMiddleware` / `GuestMiddleware` / `BearerTokenMiddleware` | Guard-Zugehörigkeitsprüfungen; siehe [Authentifizierung](authentication.md) |
| `LoginThrottleMiddleware` / `EnsureEmailVerifiedMiddleware` / `TwoFactorChallengeMiddleware` | Auth-Flow-Schranken; siehe [Auth-Flows](auth-flows.md) |
| `MaintenanceMiddleware` | Liefert 503, wenn das Cache- oder Dateisystem-Wartungs-Flag gesetzt ist |
| `InertiaVersionMiddleware` / `EncryptHistoryMiddleware` | Inertia-Asset-Versionsaushandlung + Historien-Verschlüsselung |
| `IncludeMiddleware` | Pro-Feld-Include-Sets für partielle Reloads von `#[derive(Data)]` |

### Request-Timeouts

`TimeoutMiddleware` begrenzt, wie lange ein Handler brauchen darf, um
eine Response zu *erzeugen*. Ein langsamer Handler oder eine hängende
Datenbankabfrage könnte sonst eine Verbindung unbegrenzt offen halten;
der Timeout liefert `503 Service Unavailable`, sobald die Deadline
überschritten ist.

```rust
// src/bootstrap.rs - 30-Sekunden-Obergrenze für jede HTTP-Route.
use suprnova::{global_middleware, TimeoutMiddleware};

global_middleware!(TimeoutMiddleware::default()); // DEFAULT_TIMEOUT = 30s
```

```rust
// Einen einzelnen Endpunkt auf 5 Sekunden verschärfen.
use suprnova::{Router, TimeoutMiddleware};

Router::new()
    .get("/report", heavy_report_handler)
    .middleware(TimeoutMiddleware::seconds(5));
```

`TimeoutMiddleware::new(Duration)` akzeptiert jede Duration;
`TimeoutMiddleware::seconds(n)` ist eine Abkürzung für ganze Sekunden.

Globale Middleware läuft **außerhalb** der Routen-Middleware, sodass ein
globaler Timeout eine äußere Obergrenze ist und ein Pro-Route-Timeout
eine bestimmte Route nur *strenger* machen kann - die kürzere Deadline
greift zuerst. Damit eine Route länger laufen darf als der globale
Standard, erhöhen Sie den globalen Wert oder beschränken Sie die
globale Middleware auf eine Routengruppe, die diesen Endpunkt
ausschließt.

Streaming-Responses (`HttpResponse::sse(...)`,
`HttpResponse::stream_bytes(...)`) sind naturgemäß ausgenommen: Der
Handler kehrt sofort mit einem lazy Body zurück, den hyper entleert,
nachdem die Middleware-Chain abgeschlossen ist. WebSocket-Upgrades
werden ebenfalls explizit übersprungen. Siehe [Timeouts](timeout.md)
für die Cancel-Safety-Semantik.

### CORS

`CorsMiddleware` fügt die `Access-Control-*`-Header hinzu, die ein
Browser braucht, um einer Cross-Origin-Seite das Lesen Ihrer Responses
zu erlauben, und beantwortet die `OPTIONS`-Preflight-Anfrage, die
Browser vor nicht-einfachen Cross-Origin-Aufrufen senden.
Same-Origin-Apps (das Standard-Inertia-Setup) brauchen sie nicht - sie
ist nur relevant, wenn ein Browser auf einem *anderen* Origin Ihre API
aufruft.

CORS muss **global** installiert werden, damit Preflights sie erreichen
(ein Preflight matcht nie eine Route, also würde eine
Pro-Route-CORS-Middleware niemals eine sehen). Es gibt absichtlich
keinen freizügigen Standard - wählen Sie eine Origin-Policy explizit:

```rust
// src/bootstrap.rs
use suprnova::{global_middleware, CorsConfig, CorsMiddleware};

global_middleware!(CorsMiddleware::new(
    CorsConfig::allow_origins(["https://app.example", "https://admin.example"])
        .allow_credentials(true)
        .max_age(std::time::Duration::from_secs(600)),
));
```

`CorsConfig::any_origin()` entscheidet sich explizit für
`Access-Control-Allow-Origin: *`. Builder-Methoden: `.methods([...])`,
`.allow_headers([...])` / `.allow_any_headers()`,
`.expose_headers([...])`, `.paths([...])` (CORS auf URL-Patterns
eingrenzen), `.allow_origin_patterns([regex...])`,
`.skip_when(|req| bool)`, `.allow_credentials(bool)`,
`.max_age(Duration)`. Laravel-benannte Aliase liefern gleich mit (z. B.
`.supports_credentials`, `.allowed_methods`), sodass eine
Laravel-Konfiguration direkt abbildet.

`Access-Control-Allow-Origin: *` ist zusammen mit Credentials ungültig -
der Browser lehnt es ab. Wenn `.allow_credentials(true)` gesetzt ist,
spiegelt die Middleware immer den konkreten Request-`Origin` statt `*`
zurück, sodass die ungültige Kombination niemals ausgegeben werden
kann. Nicht-Wildcard-Responses erhalten zusätzlich `Vary: Origin`,
damit gemeinsame Caches korrekt bleiben. Siehe [CORS](cors.md).

## Pipeline - Laravels `Illuminate\Pipeline\Pipeline`

`Pipeline` ist Suprnovas Gegenstück zu Laravels Pipeline-Klasse - ein
fließender Builder über `MiddlewareChain`, der die Form `send / through /
pipe / then / then_return / finally_with` widerspiegelt, die
Laravel-Nutzer bereits kennen. Nützlich, wenn Sie eine Middleware-Chain
außerhalb des Request-Lifecycles zusammensetzen möchten (ein Job, ein
CLI-Befehl, ein einmaliger Integrationstest):

```rust
use suprnova::{Pipeline, Request};

let response = Pipeline::new()
    .send(request)
    .through([AuthMiddleware, LoggingMiddleware])
    .pipe(CorsMiddleware::new(cors_config))
    .finally_with(|| tracing::info!("pipeline complete"))
    .then(|req| async move { handler(req).await })
    .await;
```

Rust-seitige Aliase liefern gleich mit den Laravel-Namen mit:
`with_request` für `send`, `with_middleware` für `through`, `push` für
`pipe`, `on_finally` für `finally_with`, `execute` für `then`. Verwenden
Sie, was sich in Ihrer Codebase besser liest.

| Pipeline-Methode | Laravel | Rust-Alias | Zweck |
|---|---|---|---|
| `send(request)` | `send($passable)` | `with_request(request)` | Legt die Anfrage fest, die durchgereicht wird |
| `through(iter)` | `through($pipes)` | `with_middleware(iter)` | Ersetzt die Pipe-Liste |
| `through_boxed(iter)` | - | - | Ersetzt die Pipe-Liste durch vorgeboxte Middleware |
| `pipe(M)` | `pipe($pipes)` | `push(M)` | Hängt eine einzelne Middleware an |
| `pipe_boxed(M)` | - | - | Hängt eine vorgeboxte Middleware an |
| `then(destination)` | `then($destination)` | `execute(destination)` | Führt die Chain mit dem Ziel-Handler aus |
| `then_with(req, dst)` | - | - | Überschreibt das Passable inline |
| `then_return()` | `thenReturn()` | - | Führt die Chain aus, liefert ein 204 No Content |
| `finally_with(F)` | `finally($callback)` | `on_finally(F)` | Läuft, nachdem das Ziel aufgelöst ist |

## Terminable Middleware - Post-Response-Hooks

Terminable Middleware läuft *nachdem* die Response an den Client
gesendet wurde. Verwenden Sie sie für langsame IO, die die Response
nicht blockieren muss: Session-Persistenz, Audit-Logging,
Metrik-Flushes.

Suprnova liefert dies als eigenständigen `Terminable`-Trait getrennt von
`Middleware`, sodass der Request-Pfad und der Termination-Pfad klar
typisiert bleiben. Ein Typ kann den einen, den anderen oder beide
implementieren:

```rust
use suprnova::{Terminable, TerminationSnapshot, register_terminable, async_trait};

pub struct AuditLogTerminator;

#[async_trait]
impl Terminable for AuditLogTerminator {
    async fn terminate(&self, snapshot: &TerminationSnapshot) {
        tracing::info!(
            method = %snapshot.method,
            path = %snapshot.path,
            status = snapshot.status,
            "request handled",
        );
    }
}

// In bootstrap.rs
register_terminable(AuditLogTerminator);
```

Der Server iteriert die registrierten Terminables in
Registrierungsreihenfolge nach jeder Response (4xx und 5xx
eingeschlossen) und wartet auf jede einzelne. Fehler werden über
`tracing::error!` protokolliert und verschluckt - die Response hat das
Gebäude bereits verlassen, es gibt also niemanden mehr, dem man sie
melden könnte.

Die Registrierung ist idempotent pro konkretem Typ.
`registered_terminables()`, `terminable_count()` und
`has_terminable::<T>()` bieten Introspektion für Tests und
Boot-Zeit-Diagnosen.

## Benannte Aliase und Gruppen

Für Aufrufer, die String-indizierte Middleware bevorzugen (Laravels
`middlewareAliases` / `middlewareGroups`), liefert Suprnova eine
prozessweite Alias- + Gruppen-Registry:

```rust
use suprnova::middleware::{
    register_middleware_alias, register_middleware_group,
    resolve_middleware_group,
};

// Aliase sind Factory-Closures - werden bei jeder Auflösung frisch
// aufgerufen, sodass jede Routenregistrierung eine unabhängige
// Middleware-Instanz erzeugt.
register_middleware_alias("auth", || AuthMiddleware::new());
register_middleware_alias("throttle", || ThrottleRequestsMiddleware::default());

// Gruppen bündeln Aliase. Verschachtelte Gruppen werden unterstützt.
register_middleware_group("api", ["auth".into(), "throttle".into()]);
register_middleware_group("web", ["session".into(), "auth".into()]);

// Zu einem Vec<BoxedMiddleware> beim Boot oder pro Route auflösen.
let api_mws = resolve_middleware_group("api")?;
```

`resolve_middleware_group` liefert `Err(MiddlewareResolveError)` bei:

- `UnknownGroup(name)` - die benannte Gruppe wurde nie registriert;
- `UnknownAlias { group, missing }` - ein Gruppeneintrag ist kein
  bekannter Alias;
- `UnknownNestedGroup { group, missing }` - eine verschachtelte
  Gruppenreferenz lässt sich nicht auflösen;
- `CycleDetected { group }` - die Gruppendefinition ist rekursiv.

Die Registrierung eines Alias oder einer Gruppe ist **last-wins** für
denselben Namen, was Laravels neu zuweisbares Kernel-Array
widerspiegelt.

## Middleware-Priorität

`prepend_middleware_priority::<M>()` / `append_middleware_priority::<M>()`
registrieren eine `TypeId` in der prozessweiten Prioritätsliste -
Suprnovas Gegenstück zu Laravels `Kernel::$middlewarePriority`.
Middleware, deren Typ früher in der Liste erscheint, sortiert sich
unabhängig von der Registrierungsreihenfolge an den Anfang der Chain:

```rust
use suprnova::{append_middleware_priority};

// SessionMiddleware läuft immer vor AuthMiddleware, unabhängig davon,
// in welcher Reihenfolge sie registriert wurden.
append_middleware_priority::<SessionMiddleware>();
append_middleware_priority::<AuthMiddleware>();
```

`middleware_priority()` liefert einen Snapshot des aktuellen
`Vec<TypeId>` für Diagnosen oder für einen Embedder, der seinen eigenen
Sortierer antreiben möchte.

## Registry-Introspektion

Über `register_global_middleware` hinaus stellt die Registry bereit:

| Oberfläche | Laravel | Zweck |
|---|---|---|
| `prepend_global_middleware(M)` | `prependMiddleware` | Am Anfang der Chain einfügen |
| `has_global_middleware::<M>()` | `hasMiddleware` | Ob der Typ `M` registriert ist |
| `global_middleware_count()` | - | Anzahl der aktuell registrierten Globals |
| `MiddlewareRegistry::from_global()` | - | Snapshot der globalen Registry in eine Pro-Server-Registry |
| `MiddlewareRegistry::prepend(M)` | - | Builder-artiges Voranstellen auf einer Registry-Instanz |
| `MiddlewareRegistry::append_boxed(M)` | - | Eine vorgeboxte Middleware anhängen |
| `MiddlewareRegistry::prepend_boxed(M)` | - | Eine vorgeboxte Middleware voranstellen |
| `MiddlewareRegistry::len()` / `is_empty()` | - | Builder-Introspektion |

`MiddlewareRegistry::from_global()` erstellt zum Aufrufzeitpunkt einen
Snapshot der globalen Registry. Registrieren Sie jede globale
Middleware, BEVOR Sie den Server bauen - ein `global_middleware!`-Aufruf,
der NACH dem Bau des Servers erfolgt, wirkt sich nicht rückwirkend aus,
sodass sich der Middleware-Stack eines laufenden Servers nicht unter ihm
verschieben kann.

## Dateilayout

Ein typisches Layout, sobald Sie ein paar Middlewares haben:

```
src/
├── middleware/
│   ├── mod.rs          # mod + pub use
│   ├── auth.rs         # AuthMiddleware
│   ├── logging.rs      # LoggingMiddleware
│   └── audit.rs        # AuditLogTerminator
├── bootstrap.rs        # global_middleware! + register_terminable
├── routes.rs           # .middleware(M) pro Route
└── main.rs
```

`make:middleware` hält `src/middleware/mod.rs` synchron - es hängt die
neue `mod foo;`-Deklaration und den passenden
`pub use foo::FooMiddleware;`-Re-Export an, wenn die Datei generiert
wird.

## Warum Suprnova abweicht

Laravel registriert Middleware-Klassen in `app/Http/Kernel.php` und
löst sie über den Container auf, der Reflection auf
Konstruktor-Type-Hints durchführt, um Abhängigkeiten zu injizieren. PHPs
Request-pro-Prozess-Modell bedeutet, dass der Kernel bei jeder Anfrage
neu aufgebaut wird, sodass die Kosten der reflektiven Auflösung einmal
pro Anfrage anfallen und zwischen Anfragen wieder verschwinden.

Suprnovas Prozessmodell ist eine einzelne Binärdatei, die viele
gleichzeitige Anfragen über viele Threads hinweg bedient. Eine frische
Chain pro Anfrage zu bauen würde einen Synchronisationspunkt auf der
globalen Middleware-Liste erzwingen und `Arc<dyn Middleware>` für jede
Schicht bei jeder Anfrage neu allozieren. Stattdessen:

- Globale Middleware wird beim Boot in ein `OnceLock<RwLock<Vec<...>>>`
  registriert, indiziert nach `TypeId` für idempotente Registrierung.
- `MiddlewareRegistry::from_global()` erstellt einmal beim Bau des
  Servers einen Snapshot der globalen Liste; die Per-Request-Chain
  verwendet diesen Snapshot wieder.
- Die Chain selbst wird durch verschachtelte `Arc<dyn Fn>`-Closures
  komponiert, sodass die Per-Request-Arbeit ein `Arc::clone` pro
  Schicht ist statt einer frischen Allokation.

Die dem Benutzer zugewandte Oberfläche - `handle(request, next)`, das
`global_middleware!`-Makro, benannte Aliase, Prioritätslisten,
Terminable-Hooks - ist dieselbe, zu der ein Laravel-Entwickler greift.
Die Mechanik darunter tauscht PHPs Per-Request-Neuaufbau gegen ein
Rust-förmiges Snapshot-beim-Boot-Modell, damit das Framework
gleichzeitige Anfragen bedienen kann, ohne um die Registry zu
konkurrieren.

## Nächste Schritte

- [Request-Lifecycle](lifecycle.md) - wo die Chain läuft und wie
  Panics an der Server-Grenze abgefangen werden
- [Fehlermodell](error-model.md) - was `Result<HttpResponse, HttpResponse>`
  tatsächlich bedeutet und wie Short-Circuits kollabieren
- [Timeouts](timeout.md) - `TimeoutMiddleware`-Cancel-Safety im Detail
- [CORS](cors.md) - Preflight-Behandlung, Origin-Patterns, Pfad-Scoping
- [Ratenbegrenzung](rate-limiting.md) - `RateLimitMiddleware` /
  `ThrottleRequestsMiddleware` und `BackendErrorPolicy`
- [Routing](routing.md) - wozu `routes!`, `Router` und `group(...)`
  expandieren
