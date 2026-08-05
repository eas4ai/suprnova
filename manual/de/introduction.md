# Einführung

Suprnova ist ein Web-Framework für Rust, das die Entwicklungserfahrung von Laravel auf Tokio bietet. Controller und Eloquent-ähnliche Modelle werden geschrieben; das Framework liefert Nebenläufigkeit, Typsicherheit und ein binäres Deployment.

```rust
use suprnova::{Request, Response, json_response};

pub async fn show(req: Request) -> Response {
    let id = req.param("id").unwrap_or("0");
    json_response!({ "id": id, "name": "Alice" })
}
```

```rust
use suprnova::{model, Model};

#[model(table = "users")]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

// Dann überall:
let user = User::find(42).await?;
let admins = User::query().db_where("role", "admin").get().await?;
let alice = User::create(attrs!{ name: "Alice", email: "alice@x.com" }).await?;
```

Wer das letzte Woche in Laravel geschrieben hat, wird die Rust-Version oben identisch empfinden - gleiche Kettformen, gleiche Methodennamen, gleiche Standards. Der Unterschied liegt in der Tiefe: Tokio statt FPM, eine Binärdatei statt einer PHP-Laufzeit, Typprüfungen zur Kompilezeit auf jeder Spalte.

## Warum es Suprnova gibt

Laravel löste das Produktivitätsproblem der Backend-Webentwicklung. Die Muster funktionieren. Nach zehn Jahren Verfeinerung steht dem Aufbau echter Produkte wenig im Weg. Aber PHPs Request-pro-Prozess-Modell hält zwei Dinge unerreichbar: kostengünstige langlebige Verbindungen (WebSockets, SSE, vom Server gepushte Benachrichtigungen ohne Polling) und einfaches paralleles I/O in einem Request-Handler.

Rust bietet beides kostenlos mit Tokio. Das Problem ist, dass das Rust-Web-Ökosystem zwingt, die Produktivitätsebene selbst zu bauen: eine HTTP-Crate wählen, eine ORM wählen, ein Migrationstool wählen, eine Queue wählen, alles verdrahten, eigene Konventionen entwerfen. Jede App erfindet neu, was Laravel bereits standardisiert hat.

Suprnova ist das Ergebnis, wenn man Laravels Konventionen auf Tokio kopiert. Es bietet:

- **Gleiche Oberfläche** - `routes!`, `Auth::user()`, `Cache::remember`,
  `Mail::send`, `Queue::push`, `Storage::disk("s3")`, `Notify::send`,
  `Schedule::call`, `Gate::allows`, der Eloquent Query Builder, Soft Deletes,
  Factories, Observers, Broadcasting, alles davon
- **Anderes Innenleben** - async-überall, langlebige Verbindungen als
  First-Class Citizens, einzelne statisch verlinkte Binärdatei, kein Preforking, kein
  Opcache, kein FPM
- **Typsicherheit** - Modelle, Routes und Event Payloads werden zur
  Kompilezeit überprüft; defekte Refactorings erreichen nicht Production
- **Eine echte Frontend-Story** - Inertia.js verbindet sich mit Svelte 5, React 19 oder
  Vue 3.5 Starter, keine separate API zu warten

## Designprinzipien

Dies sind die Prinzipien, an die sich die Framework-Autoren selbst halten.
Sie erklären, warum ein Kapitel sagt, was es sagt.

**1. Parität kommt aus dem Laravel-Changelog.** Wenn Laravel ein
Feature liefert, folgt Suprnova. Die heutige Grundlage ist Laravel 13.x und jedes
gelieferte Subsystem wurde dagegen überprüft. Die
[Laravel Parity Map](parity.md) ist die explizite Feature-für-Feature-Tabelle.

**2. Absichtlich divergieren, wo Rust Dinge besser macht.** Wo Laravel
eine PHP-geformte Wahl traf, die nicht in Rust getroffen werden muss, wählt Suprnova
die Rust-geformte und erklärt das. Das größte Beispiel ist Nebenläufigkeit:
WebSockets, Broadcasting, Background Worker und HTTP/2 Server-Push
sind First-Class, nicht angebolt. Wo dies in einem
Kapitel aufgerufen wird, nach **"Warum Suprnova abweicht"** Boxen schauen.

**3. Keine Zugangskontrolle.** Laravel beschränkt einige Features auf ein Backend
(z.B. Vektorsuche über Postgres `pgvector`). Suprnova behandelt Backends
als Treiber - `Vector::driver("qdrant")`, `Vector::driver("pinecone")`,
`Vector::driver("mariadb")`, `Cache::driver("redis")`, `Mail::driver("ses")`.
Die richtige Wahl bleibt dem Entwickler; wir wählen nicht für.

**4. Suprnova ist die API-Oberfläche.** Intern werden SeaORM, hyper, Tokio,
serde, sqlx, validator, lettre und dutzende weitere genutzt. Nichts davon sollte
im eigenen Code auftauchen. Man hängt von `suprnova::*` ab. Alles wird neu exportiert,
das man berühren wird - inklusive SeaORMs `Entity`, `Column`, `ActiveModel`,
`QueryFilter`, usw. - unter der Framework-Wurzel. Die Ausweichklappe
(`use suprnova::sea_orm;`) existiert für den seltenen Fall, den die kuratierte Oberfläche
nicht abdeckt, aber man sollte sie fast nie brauchen.

## Was im Paket enthalten ist

Eine nicht-erschöpfende Übersicht. Die vollständige Liste ist in [`documentation.md`](documentation.md).

| Bereich | Was enthalten ist |
|---|---|
| **HTTP** | `routes!` Makro, Controller, Middleware, Requests, Responses, Route Model Binding, signierte URLs, Resource Routing, Redirect Helper, CORS, CSRF, Idempotency Keys, Timeout, Rate Limiting, strukturierte Fehler mit Panic Recovery |
| **Datenbank** | SeaORM darunter, Multi-Treiber (Postgres, MySQL, MariaDB, SQLite), Migrationen, Seeders, Query Builder, Transaktionen mit Savepoints, Multi-Connection Read/Write Split |
| **Eloquent** | `#[suprnova::model]` Makro, alle 11 Relation-Arten, Eager Loading, Soft Deletes, Prunable, Scopes (lokal und global), 16 Lifecycle Events, Observers, 22 eingebaute Casts, Accessors/Mutators, drei Paginators, Chunk/Lazy/Cursor Iteration, Collections, Replikation |
| **Authentifizierung** | Stateful Sessions, Opaque User IDs, mehrere Guards, Eloquent + Database Provider, Password Hashing (bcrypt + argon2), Policy Macros, Gates, Email Verification, Password Reset, Brute-Force Throttling, TOTP 2FA, Remember-Me, OAuth über torii Integration |
| **Frontend** | Inertia v3 Bridge, Svelte 5 / React 19 / Vue 3.5 Starter Templates, typiertes `#[derive(InertiaProps)]`, Partial Reloads, automatische TypeScript-Typengenerierung |
| **Hintergrund** | Queue mit Memory/Sync/Redis/Database/Null Treibern, Batches, Chains, Job Middleware, Failed-Job Store, `#[command]`/`#[derive(Command)]` Console Binary, `Task` Trait Scheduler, `#[workflow]` Langlebige zustandsbehaftete Arbeit, `Supervisor` Trait mit Panic-Catch Auto-Restart, Command Bus, Event Dispatcher |
| **Echtzeit** | `ws!()` Makro für typisierte WebSocket Handler, Broadcasting Channels (Public, Private, Presence), Sea-Streamer Fanout, Server-Sent Events, Web Push (VAPID) |
| **Cache & Speicher** | Memory, Redis, Database Cache Treiber; Atomare Operationen; Tagged Cache; Cache Locks; Dateisystem mit fs/Memory/S3/Azblob/Gcs Treibern; Path-Traversal Protection; Vector Storage mit mehreren Backends |
| **Mail & Benachrichtigungen** | `Mailable` Trait, Treiber für SMTP/SES/Mailgun/Postmark/SendGrid/Resend (plus In-Memory & Log für Tests), `Notifiable` mit Mail/Database/Broadcast/WebPush Channels |
| **Validierung & Daten** | `#[derive(Validate)]`, Form Requests, Async Validation, `#[derive(Data)]` für Partial-Reload Include Sets, `#[derive(Resource)]` für JSON:API |
| **Zahlungen** | Generische Provider Oberfläche (Gateway/MoR/Redirect-Flow), Referenz Adapter für Stripe und Paddle, Mirror Tables mit Webhook Idempotency, Inertia Checkout Components |
| **Feature Flags** | Database Evaluator, Cached Evaluator mit TTL, Feature Middleware, Sub-Sekunden-Propagation über Sync Trait |
| **Testen** | `#[suprnova_test]`, `expect!`, `TestDatabase`, Fakes für jede externe Oberfläche (Mail, Notify, Queue, Bus, Events, Storage, Http) |
| **CLI** | `suprnova new` Scaffolder (Svelte/React/Vue), `serve` Dev Runner, `migrate*`, `db:sync`, `db:seed`, `make:*` Generatoren, `model:prune`, Console Binary pro Projekt |

## Produktionsbereitschaft

Das Framework ist Produktionsgrad in Umfang und Testabdeckung. Zum aktuellen
Stand:

- Jede Laravel 13.x Oberfläche über die 30 dokumentierten Domains wird geliefert
- Jede Frage aus unabhängigen Code Reviews wurde beantwortet
- Die Workspace-Test Suite passiert bei jeder Änderung
- Jede öffentliche API in `framework/src/lib.rs` ist dokumentiert - ein
  undokumentiertes öffentliches Item lässt den Build scheitern

Ab **v1.0.0** ist die öffentliche API stabil: Apps pinnen einen Release Tag
(`tag = "v<version>"` - der Tag ist das Release; es gibt keine crates.io
Veröffentlichung), und eine Breaking Change landet nur hinter einem Version Bump, dessen
[CHANGELOG](changelog.md) Abschnitt das sagt.

## Einen Lesepfad wählen

| Wer | Anfangen mit |
|---|---|
| Ein Laravel-Entwickler | [Von Laravel](from-laravel.md) |
| Ein Rust-Entwickler mit Axum/Actix/Rocket Erfahrung | [Vom Rust-Web](from-rust-web.md) |
| Beides oder weder, und einfach nur bauen wollen | [Installation](installation.md) → [Schnellstart](quickstart.md) |
| Auf der Suche nach einem spezifischen Feature | [`documentation.md`](documentation.md) (das vollständige Inhaltsverzeichnis) |
| Fragend "hat Suprnova X?" | [Laravel Parity Map](parity.md) |
