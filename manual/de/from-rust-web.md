# Vom Rust-Web kommend

Sie haben Rust-Services auf Axum, Actix, Rocket oder handgerolltem hyper bereitgestellt.
Sie kennen die Sprache und die Laufzeit. Was bringt Ihnen Suprnova eigentlich?

**Die Produktivitätsschicht.** Routing, Controller, eine ORM, Migrationen,
Warteschlangen, Planung, Authentifizierung, Mail, Benachrichtigungen, Broadcasting, Cache,
Speicher, Validierung und eine typisierte Frontend-Bridge - alles miteinander verdrahtet,
alles nach den gleichen Konventionen, alles produktionsreif. Sie schreiben
Controller und Modelle; Sie wählen das Layout nicht.

Wenn Sie bereits ein oder zwei echte Apps in Axum gebaut haben, wissen Sie, wie viel
dieser Aufwand Verdrahtung statt Features war. Suprnova ist die Verdrahtung,
einmal erledigt, mit Meinungen wo Meinungen wichtig sind, austauschbar wo sie es nicht sind.

## Der 30-Sekunden-TL;DR

```bash
suprnova new myapp --frontend svelte    # scaffoldet Backend + SPA + Vite
cd myapp
suprnova db:sync                        # führt Migrationen aus, regeneriert Entities
suprnova serve                          # Backend + Vite-Dev-Server
```

Sie haben jetzt:

- Ein hyper-Server mit HTTP/1.1 und HTTP/2, WebSocket-Upgrade, Graceful Shutdown
- Eine SeaORM-gestützte Eloquent-Schicht mit Relationen, Eager Loading, Soft Deletes
- Inertia.js-Bridge von Rust zu Svelte 5 mit typisierten `#[derive(InertiaProps)]`
- Authentifizierung (Sessions, Password-Hashing, Provider-gestützte E-Mail-Verifizierung + Passwort-Reset, plus 2FA und OAuth über torii)
- Eine Warteschlange mit Memory/Sync/Redis/Database/Null-Treibern
- Ein Cron-Scheduler angetrieben durch das `Task`-Trait
- Ein Console-Binary pro Projekt für `cargo run --bin console <cmd>`
- Cache, Speicher (fs/s3/azblob/gcs), Mail (SMTP + 5 Provider: SES, Mailgun, Postmark, SendGrid, Resend), Web Push
- Broadcasting über einen austauschbaren Hub (sea-streamer standardmäßig)
- Validierung, CSRF, CORS, Rate Limiting, Idempotenz, Request-Timeouts, strukturierte Fehler

Und eine statisch verlinkte Binärdatei am Ende von `cargo build --release`.

## Was darunter liegt

| Bereich | Crate |
|---|---|
| HTTP-Server | `hyper` + tower-ähnliche Middleware (eigene Implementierung) |
| Async-Laufzeit | `tokio` |
| Router | `matchit` |
| ORM | `sea-orm` (neu exportiert als `suprnova::sea_orm`) |
| Migrationen | `sea-orm-migration` |
| Datenbank-Treiber | `sqlx` (postgres / mysql / mariadb / sqlite) |
| Serialisierung | `serde` / `serde_json` |
| Validierung | `validator` |
| Sessions | eigene (treiberbasiert) |
| Templating | `tera` (für Mail-Bodies; Frontend ist Inertia) |
| Krypto | `aes-gcm`, `argon2`, `bcrypt` |
| WebSockets | `hyper-tungstenite` |
| Streaming | `sea-streamer` (Broadcasting-Fanout-Backend) |
| OAuth | `torii` (vendored Fork) |
| Tracing | `tracing` + `tracing-subscriber` |

Üblicherweise greifen Sie nicht direkt auf diese zu - Suprnova
exportiert neu, was Sie brauchen. SeaORM ist der tiefste Durchgang: `Entity`,
`Column`, `ActiveModel`, `ConnectionTrait`, der Query Builder, die
Migration-Prelude. Die Ausweichklappe ist `use suprnova::sea_orm;` wenn Sie
etwas brauchen, das die kuratierte Oberfläche nicht abdeckt.

## Was Suprnova gegenüber reinem Axum hinzufügt

Axum ist ausgezeichnet. Das ist Actix auch. Rocket auch. Der Grund, warum Suprnova existiert,
ist nicht, dass diese Frameworks schlecht wären - es ist, dass jedes Team, das ein
echtes Produkt darauf baut, am Ende die gleiche Produktivitätsschicht neu implementiert.
Suprnova liefert diese Schicht:

| Fähigkeit | Handgerollt auf Axum | In Suprnova |
|---|---|---|
| Routing-Makros, die sich auf hunderte Routes skalieren | Builder API, kann unordentlich werden | `routes!`-Makro mit Grouping, Prefixen, Middleware, Naming |
| Route-Model-Binding (Path-ID → geladenes Modell) | Custom Extractor pro Typ | `#[handler]` löst `post::Model` aus `{id}` automatisch auf |
| Eloquent-ähnlicher verkettbarer Query Builder | Verwenden Sie SeaORM direkt | `Post::query().db_where(...).order_by(...).get().await?` |
| Soft Deletes, Observer, Lifecycle Events | Pro-Model bauen | `#[model(soft_deletes)] + impl Observer<Post>` |
| Migrationen + Entity-Generierung | Sea-orm-cli + Skripte verdrahten | `suprnova db:sync` führt Migrationen aus und regeneriert Entities |
| Authentifizierung (Sessions, Provider, Guards) | Tower-Sessions + eigene Logik zusammenführen | `Auth::attempt`, `Auth::user`, `.middleware(AuthMiddleware)` pro Route |
| E-Mail-Verifizierung, Passwort-Reset, 2FA, Brute-Force | Alle vier handbauen | Alle eingebaut, konfigurierbar, idempotent |
| Hintergrund-Warteschlange | Treiber wählen, Worker schreiben | `Queue::push` + `cargo run -- queue:work` |
| Cron-Planung | Tokio-Task mit `tokio_cron_scheduler` schreiben | `impl Task` + `Schedule::task(...).daily().at("03:00")` |
| Inertia-Bridge | Extractor + JS-Adapter bauen | `inertia_response!(&req, "Page", props)` |
| Typisierte Frontend-Props (Rust → TS) | Generator schreiben | `#[derive(InertiaProps)]` + `suprnova generate-types` |
| Broadcasting (öffentliche / private / Presence-Kanäle) | Streaming-Backend + Authentifizierung verdrahten | `BroadcastHub` + `Channel`/`PrivateChannel`/`PresenceChannel`-Traits |
| Mail mit mehreren Providern | Einen wählen, eigene Abstraktion schreiben | `Mail::driver("ses")` etc., einheitliche `Mailable`-API |
| WebPush | Spec lesen, Notifier bauen | `WebPushChannel` mitgeliefert, VAPID eingebaut |
| Validierung + Form Requests | Verwenden Sie `validator` + custom Extractor | `#[derive(Data, Validate)]` Form Requests, Async-Validierung |
| JSON:API-Ressourcen | Hand-Formatting von Antworten | `#[derive(Resource)]` |
| Rate Limiting mit Fail-Open/Closed-Richtlinie | Selbst bauen | `RateLimiter` + `BackendErrorPolicy` |
| Idempotency-Schlüssel | Selbst bauen | `Idempotency::remember(key, ttl, body)` mit Stripe-ähnlichem Replay |
| CSRF (mit Laravel-ähnlichen Glob-Ausschlüssen) | Selbst bauen | `CsrfMiddleware` mit `except` + `except_method` |
| Strukturierte Fehler mit bereinigten 5xx | Selbst bauen | `FrameworkError` / `HttpError`-Trait, Panic-Recovery |
| Container mit Task-Local → Thread-Local → Global-Scopes | Eigenen schreiben | `App::bind` / `singleton` / `factory` mit richtiger Isolation |
| Health-Endpunkt, Request-ID, strukturiertes Logging | Zusammenkleben | Standardmäßig alles an |

Der Trade-off ist Meinungen: Suprnova wählt ein Layout, wählt einen Standard-Treiber,
wählt eine Naming-Konvention. Sie können abweichen (Treiber sind austauschbar,
Konfiguration ist überschreibbar, der Container lässt Sie Services tauschen), aber die
Standardwerte sind so gestaltet, dass sie die richtige Wahl für "schnell ein Produkt bauen" sind.

## Vertraute Rust-Patterns

Sie werden die Formen erkennen:

```rust
// Ein Handler gibt `Result<HttpResponse, HttpResponse>` zurück (Alias Response).
pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id").unwrap_or("0").parse().unwrap_or(0);
    let post = Post::find_or_fail(id).await?;
    Ok(HttpResponse::json(serde_json::json!({ "post": post })))
}

// Middleware ist ein Trait, keine Closure:
#[async_trait]
impl Middleware for RequireAdmin {
    async fn handle(&self, req: Request, next: Next) -> Response {
        let user = Auth::user_as::<User>().await?
            .ok_or_else(|| HttpResponse::text("Unauthorized").status(401))?;
        if !user.is_admin {
            return Err(HttpResponse::text("Forbidden").status(403));
        }
        next(req).await
    }
}

// Hintergrundarbeit ist der `Job`-Trait - `handle(self)` führt den Job aus:
#[async_trait]
impl Job for SendWelcomeEmail {
    fn job_name() -> &'static str { "SendWelcomeEmail" }

    async fn handle(self) -> Result<(), FrameworkError> {
        let user = User::find_or_fail(self.user_id).await?;
        Mail::to(&user.email).send(WelcomeMail { user }).await?;
        Ok(())
    }
}
```

Wenn Sie mit Tower-Middleware vertraut sind: Suprnova-Middleware ist konzeptionell
das Gleiche (ein Wrapper um `next`), verwendet aber ein eigenes Trait (nicht Towers
`Service`), weil Towers Combinator-Typen unangenehm werden, wenn Sie anfangen,
anwendungsspezifische Extractor zu verschachteln. Die Form ist einfacher; das
mentale Modell ist das gleiche.

Wenn Sie Axums Extractor-Pattern verwendet haben: Suprnova's `#[handler]`-Makro
spielt die gleiche Rolle, wird aber über den Service-Container aufgelöst statt über
Traits, was es ermöglicht, App-Services und Request-Daten einzuspeisen. Route-Model-Binding
(`Post` aus `{id}`) ist eingebaut.

Wenn Sie `sqlx` direkt verwendet haben: Suprnova's ORM sitzt über SeaORM, das über
sqlx sitzt. Sie können zu reinem SQL über `DB::select(...)` / `DB::select_one(...)` abfallen
oder `DB::table("name")` für verkettbare dynamische Abfragen verwenden; Sie können direkt
zu SeaORM für Dinge abfallen, die die Eloquent-Oberfläche nicht abdeckt (z.B. rohe
`Statement`-Abfragen mit benutzerdefiniertem Result-Mapping). Das [Eloquent-Kapitel](eloquent.md) behandelt die Ausweichhaken.

## Was ist der Produktivitäts-Unterschied?

Wählen Sie ein Feature aus, das Sie schon einmal in reinem Axum gebaut haben. Suprnova
liefert es als Kapitel:

- **"Ich habe ein Authentifizierungssystem gebaut und es hat zwei Wochen gedauert."** →
  [Authentifizierung](authentication.md) + [Auth-Flows](auth-flows.md). Stellen Sie die
  Migration ein, konfigurieren Sie den Guard, fertig.
- **"Ich habe meinen eigenen Queue-Worker mit Retry/Backoff geschrieben."** →
  [Warteschlangen](queues.md). `Queue::push` + `cargo run -- queue:work`.
- **"Ich habe WebSockets mit hyper-tungstenite verdrahtet."** →
  [WebSockets](websockets.md). Das `ws!()`-Makro tippt den Handler;
  das Upgrade, Ping/Pong-Herzschlag, Close-Frame-Handshake und
  Back-Pressure sind erledigt.
- **"Ich habe einen Inertia-Adapter von Grund auf gebaut."** →
  [Inertia](frontend.md). `inertia_response!(&req, "Page", props)`, mit
  `InertiaProps` das TS-Typen generiert.
- **"Ich habe einen Per-Tenant-Rate-Limiter gebaut."** →
  [Ratenbegrenzung](rate-limiting.md). Konfigurierbarer Schlüssel, konfigurierbare
  Fail-Open vs Fail-Closed Richtlinie, Fail-Closed gibt 503 zurück.
- **"Ich habe Stripe-Webhook-Signatur-Verifizierung + Replay-Schutz implementiert."** →
  [Zahlungen: Stripe](payments-stripe.md). In den Adapter eingebaut,
  Webhooks gehen in eine Mirror-Tabelle mit UNIQUE Idempotency.

Was Sie handwerklich in zwei Wochen gebaut hätten, importieren Sie in einer Zeile.

## Was Sie immer noch als "Ihrs" erkennen werden

Ein paar Dinge bleiben nah am reinen Rust, weil die Sprache Ihnen etwas besseres
als eine Framework-Abstraktion gibt:

- **Concurrency-Primitive.** `tokio::spawn`, `Arc`, `Mutex`, Channels -
  verwenden Sie sie. Das Framework umwickelt sie nicht.
- **Fehlertypen.** Sie definieren Ihre Domain-Fehler. Implementieren Sie das
  `HttpError`-Trait darauf, um einen richtigen Status-Code + Nachricht in
  der Wire-Response zu bekommen. Die `FrameworkError` und `AppError` des Frameworks
  sind Ausweichhaken für Cross-Cutting + Ad-Hoc-Fehler respectively.
- **Custom Treiber.** Cache, Warteschlange, Mail, Broadcasting, Vector, Zahlungen - jedes "Treiber-Registry"-Subsystem akzeptiert custom Treiber. Implementieren Sie
  das Trait, registrieren Sie es in `bootstrap.rs`, fertig.
- **Rohes SQL wenn Sie es wollen.** `DB::select(...)`, `DB::table(...).get()`
  für dynamische Rows, oder ganz auf SeaORM abfallen. Die ORM kommt aus dem Weg.
- **Ihre eigene Tower-Middleware?** Suprnova liefert keinen Tower-Adapter -
  Middleware hier ist `impl Middleware`, nicht `tower::Service`.
  Wenn Sie eine Tower-only Crate bringen müssen, würden Sie sie von Hand anpassen.
  In der Praxis deckt das eingebaute Middleware-System fast alles ab, das Sie
  brauchen. Siehe [Middleware](middleware.md).

## Worauf Sie verzichten

Ehrlichkeit ist wichtiger als Marketing:

- **Konventionen.** Modelle leben hier, Controller dort, Migrationen
  dort, Observer dort. Der Scaffolder wählt. Sie können dagegen ankämpfen; Sie
  sollten es wahrscheinlich nicht. Die Konventionen sind Laravels, überprüft und
  kampferprobt.
- **Etwas Flexibilität darin, wie der Request fließt.** Die Middleware-Chain
  hat eine feste äußerste Reihenfolge (Request-ID → Globals → Route-Middleware
  → Handler). Sie können Middleware überall darin einfügen, aber Sie
  können die Request-ID oder Panic-Recovery-Layer nicht verschieben - sie sind
  Invarianten.
- **Die PHP-geformten Ecken.** Wo Laravel etwas tut, weil PHP es tut,
  tut Suprnova stattdessen die Rust-geformte Sache - aber wir sagen Ihnen, wann.
  Schauen Sie nach **"Warum Suprnova abweicht"** Ausrufen in Kapiteln.

## Warum "Laravel-inspiriert" Ihnen wichtig sein sollte, auch wenn Sie nie PHP geschrieben haben

Das Rust-Web-Ökosystem ist grob dort, wo das PHP-Ökosystem um 2009 war. Die
Crates existieren; die Patterns nicht. Suprnova portiert einen äußerst verfeinerten
Satz von Patterns aus einem Framework, das 10+ Jahre Production-Druck hatte, um es zu formen.
Sie bekommen Patterns, die bereits den Kontakt mit der Realität überstanden haben.

Die Kosten sind, dass Suprnova *opinioniert ist*. Wenn Sie ein minimales
"Wähle-alles-selbst"-Framework wollen, ist Axum genau da und es ist
ausgezeichnet. Wenn Sie ein "Framework das Dinge entscheidet, damit Sie
sich auf das Produkt konzentrieren können" wollen, das ist Suprnova.

## Nächste Schritte

- [Installation](installation.md) - `suprnova new`, was wird erstellt
- [Schnellstart](quickstart.md) - bauen Sie eine kleine App in 5 Minuten
- [Request-Lifecycle](lifecycle.md) - wie ein Request fließt, was wo läuft
- [Service Container](container.md) - wie Services gebunden und aufgelöst werden
- [Eloquent](eloquent.md) - das längste Kapitel; die Oberfläche ist weit

Oder navigieren Sie überall über [`documentation.md`](documentation.md).
