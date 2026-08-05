# Verzeichnisstruktur

Wenn Sie `suprnova new my-app --frontend svelte` ausführen, erstellt der
Scaffolder Folgendes:

```
my-app/
├── Cargo.toml                      # Crate-Manifest + Abhängigkeiten, zwei [[bin]]-Ziele
├── .env                            # Lokale Konfiguration - DB-URL, App-Schlüssel, Ports
├── .env.example                    # Vorlage für Ops/CI
├── .gitignore                      # Schließt target/, .env, node_modules/, public/assets/ aus
├── cmd/
│   └── main.rs                     # Binary-Einstiegspunkt; ruft Application::new().run() auf
├── src/
│   ├── lib.rs                      # Modul-Verdrahtung (`pub mod controllers;` usw.)
│   ├── bootstrap.rs                # Registriert Services, Observer, Listener - das
│   │                               # Suprnova-Äquivalent von Laravels Service Providern
│   ├── routes.rs                   # Der `routes!`-Makrobaum - jede von der App servierte URL
│   ├── bin/
│   │   └── console.rs              # `cargo run --bin console <subcommand>`-Einstiegspunkt -
│   │                               # das Suprnova-Äquivalent von `php artisan`
│   ├── actions/
│   │   ├── mod.rs
│   │   └── example_action.rs       # Ein-Methoden-aufrufbare Controller
│   ├── commands/
│   │   └── mod.rs                  # Mit `#[command]` kommentierte Handler registrieren sich hier
│   ├── config/
│   │   ├── mod.rs
│   │   ├── database.rs             # Typisierte DB-Konfiguration (Treiber, URL, Pool)
│   │   └── mail.rs                 # Typisierte Mail-Konfiguration
│   ├── controllers/
│   │   ├── mod.rs
│   │   ├── home.rs                 # GET /-Handler
│   │   ├── auth.rs                 # Login / Registrierung / Logout
│   │   └── dashboard.rs            # Erfordert Auth; Beispiel geschützte Route
│   ├── middleware/
│   │   ├── mod.rs
│   │   ├── logging.rs              # Request-/Response-Protokollierung
│   │   └── authenticate.rs         # Session-basierter Auth-Guard
│   ├── migrations/
│   │   ├── mod.rs
│   │   ├── m_*_create_users_table.rs
│   │   ├── m_*_create_sessions_table.rs
│   │   ├── m_*_create_remember_tokens_table.rs
│   │   ├── m_*_create_workflows_table.rs
│   │   └── m_*_create_workflow_steps_table.rs
│   └── models/
│       ├── mod.rs
│       └── user.rs                 # `#[suprnova::model]` User-Modell
├── frontend/
│   ├── package.json
│   ├── vite.config.ts
│   ├── tsconfig.json
│   ├── index.html                  # Vite-Einstiegspunkt; mountet die SPA
│   └── src/
│       ├── main.{tsx,ts}           # Inertia-Client-Setup (pro Framework)
│       ├── app.css                 # Globale Stile + Tailwind
│       ├── pages/
│       │   ├── Home.{tsx,svelte,vue}
│       │   ├── Dashboard.{tsx,svelte,vue}
│       │   └── auth/
│       │       ├── Login.{tsx,svelte,vue}
│       │       └── Register.{tsx,svelte,vue}
│       └── types/
│           └── inertia-props.ts    # Automatisch generiert aus #[derive(InertiaProps)]
└── public/
    └── assets/                     # Vite-Produktions-Build-Ausgabe landet hier
```

Svelte fügt `frontend/svelte.config.js` und `frontend/src/app.d.ts` hinzu.
Vue fügt `frontend/src/shims-vue.d.ts` hinzu.

Der API-Starter (`suprnova new my-api --api`) ist schlanker: kein
`frontend/`, keine Auth-Controller, und `cmd/main.rs` wird durch
`src/main.rs` ersetzt.

## Zweck der einzelnen Verzeichnisse

### `cmd/main.rs`

Der Binary-Einstiegspunkt. Eine kurze Datei - üblicherweise 10-20 Zeilen -
die die Standard-Boot-Pipeline aufruft:

```rust
use suprnova::Application;
use my_app::{bootstrap, config, migrations, routes};

#[suprnova::main]
async fn main() {
    Application::new()
        .config(config::register_all)
        .bootstrap(bootstrap::register)
        .routes(routes::register)
        .migrations::<migrations::Migrator>()
        .run()
        .await;
}
```

`Application::run()` parst die CLI der Binary (`serve` / `web:run` /
`migrate*` / `schedule:*` / `workflow:work` / `queue:work`), lädt
`.env`, führt Ihre Config-Funktion aus, dann verteilt den Subkommando. Der
Serve-Pfad führt auch Ihre Bootstrap-Funktion aus und startet den HTTP-
Server.

Sie bearbeiten `cmd/main.rs` nach dem initialen Scaffolding fast nie.

### `src/lib.rs`

Eine flache Moduldefinitionsdatei:

```rust
pub mod actions;
pub mod bootstrap;
pub mod commands;
pub mod config;
pub mod controllers;
pub mod middleware;
pub mod migrations;
pub mod models;
pub mod routes;
```

Dies macht `crate::controllers::home::index` von `routes.rs` erreichbar.

### `src/bootstrap.rs`

Die einzelne Funktion, die Ihre App verdrahtet. Sie registrieren hier
Service-Container-Bindings, Observer, Event-Listener, benutzerdefinierte
Middleware und jedes andere Boot-Setup. Sie entspricht Laravels
`AppServiceProvider`, `EventServiceProvider`, `BroadcastServiceProvider`
usw., alle in einer Datei:

```rust
use std::sync::Arc;
use suprnova::App;

pub async fn register() {
    // Service in den Container binden
    App::bind::<dyn MyService>(Arc::new(MyServiceImpl::new()));

    // Eloquent-Observer registrieren
    crate::models::user::register_observer();

    // Auf Events hören
    suprnova::Event::listen::<OrderShipped, _>(Arc::new(SendShipmentNotification)).await;
}
```

`register()` läuft einmal pro Prozess, nach dem Config-Loader, aber bevor
`serve` die erste Anfrage akzeptiert. Worker (`queue:work`,
`schedule:run`, `workflow:work`) verwenden denselben Bootstrap erneut, daher
sehen sie die gleichen Services. Siehe [Application Bootstrap](bootstrap.md).

### `src/routes.rs`

Ihre URL-Oberfläche. Das `routes!`-Makro auf Modulebene erweitert sich zu
einer `pub fn register() -> Router`, die `cmd/main.rs` an
`Application::routes(...)` übergibt:

```rust
use suprnova::{get, post, put, delete, routes};
use crate::{controllers, middleware};

routes! {
    get!("/", controllers::home::index).name("home"),

    // Auth (registriert + geschützt)
    get!("/login", controllers::auth::show_login).name("login.show"),
    post!("/login", controllers::auth::login).name("login.attempt"),
    post!("/logout", controllers::auth::logout).name("logout"),
    get!("/register", controllers::auth::show_register).name("register.show"),
    post!("/register", controllers::auth::register).name("register"),

    // Dashboard erfordert Authenticate-Middleware
    get!("/dashboard", controllers::dashboard::index)
        .middleware(middleware::authenticate::auth())
        .name("dashboard"),
}
```

Siehe [Routing](routing.md).

### `src/bin/console.rs`

Ihre projektspezifische Console-Binary. Läuft als `cargo run --bin console
<subcommand>` und verteilt das eingebaute `db:seed` des Frameworks sowie
jeden mit `#[command]` kommentierten Handler (oder typisierte
`#[derive(Command)]` Struktur) in `src/commands/` - beide Formen
registrieren sich zur Compile-Zeit über inventory:

```bash
cargo run --bin console db:seed           # eingebaut im Framework
cargo run --bin console report:daily      # Ihr benutzerdefinierter Befehl
```

Die langlebigen Worker (`queue:work`, `schedule:run`,
`schedule:work`, `workflow:work`) leben auf der Haupt-App-Binary,
weil `Application::run()` sie verteilt - rufen Sie sie als
`cargo run -- queue:work` auf (oder via `suprnova schedule:run` /
`suprnova workflow:work`, wenn Sie die Umbrella-CLI bevorzugen).

Siehe [Konsole](console.md).

### `src/commands/`

Wo Ihre Console-Handler leben. Zwei Varianten: eine typisierte Struktur mit
clap-abgeleiteten Args und `impl TypedCommand`, oder ein reines `#[command]`
auf einer `async fn(Vec<String>) -> Result<(), FrameworkError>`. Der
Scaffolder generiert die typisierte Form:

```rust
use async_trait::async_trait;
use clap::Parser;
use suprnova::{Command, FrameworkError, TypedCommand};

#[derive(Parser, Command, Debug)]
#[console(name = "report:daily", description = "Generate the daily report")]
pub struct DailyReport {
    #[arg(long, default_value_t = false)]
    pub dry_run: bool,
}

#[async_trait]
impl TypedCommand for DailyReport {
    async fn run(self) -> Result<(), FrameworkError> {
        // …
        Ok(())
    }
}
```

`suprnova make:command report-daily` erstellt das Scaffold der Datei und fügt
es `src/commands/mod.rs` hinzu. Siehe [Konsole](console.md).

### `src/config/`

Typisierte Konfigurations-Strukturen. Der Scaffolder liefert `database.rs`
und `mail.rs`; fügen Sie Ihre eigenen für jedes Subsystem hinzu, das Ihre
App braucht. Jede Config-Struktur liest ihre Werte aus der Umgebung, und
`config::register_all()` registriert sie beim Framework:

```rust
use suprnova::{env, env_required};

#[derive(Clone, Debug)]
pub struct AnalyticsConfig {
    pub api_key: String,
    pub max_batch: u32,
}

impl AnalyticsConfig {
    pub fn from_env() -> Self {
        Self {
            api_key: env_required::<String>("ANALYTICS_API_KEY"),
            max_batch: env("ANALYTICS_MAX_BATCH", 100u32),
        }
    }
}
```

Verdrahten Sie sie in `config/mod.rs`:

```rust
use suprnova::Config;

pub fn register_all() {
    Config::register(AnalyticsConfig::from_env());
}
```

Siehe [Konfiguration](configuration.md).

### `src/controllers/`

HTTP-Handler-Funktionen. Ein Modul pro Ressource. Jede `pub async fn`,
die eine `Request` annimmt und eine `Response` zurückgibt, ist von einer
Route aufrufbar.

### `src/middleware/`

Middleware-Implementierungen. Der Scaffolder liefert `logging` und
`authenticate`; fügen Sie Ihre eigenen hier als `pub struct Foo` mit
`impl Middleware for Foo` hinzu. Registrieren Sie sie global in
`bootstrap.rs` oder wenden Sie sie per-Route über `.middleware(…)` im
`routes!`-Baum an. Siehe [Middleware](middleware.md).

### `src/migrations/`

SeaORM-Migrationen. Der Scaffolder liefert ein paar für die Auth- und
Workflow-Tabellen. `suprnova make:migration <name>` fügt eine neue hinzu.
`suprnova migrate`, `migrate:rollback`, `migrate:status`, `migrate:fresh`,
`db:sync` operieren alle in diesem Verzeichnis. Siehe
[Migrationen](migrations.md).

### `src/models/`

Ihre Eloquent-Modelle. Eine Datei pro Modell, jeweils eine
`#[suprnova::model]` Struktur. Der Scaffolder liefert `user.rs`; fügen Sie
neue Modelle hinzu, indem Sie eine neue Datei von Hand schreiben oder
`suprnova db:sync --regenerate-models` nach einer Schema-Migration
ausführen. Siehe [Eloquent](eloquent.md).

### `src/actions/`

Ein-Methoden-aufrufbare Controller. Optionales Muster - verwenden Sie diese,
wenn ein Controller genau eine Methode hätte und Sie ihn lieber "Action"
nennen würden, als ihn zu wrappen. Der Scaffolder liefert ein Beispiel, das
Sie löschen oder anpassen können. Siehe [Aktionen](actions.md).

### `frontend/`

Die Vite + Inertia SPA. Dies ist ein normales Frontend-Projekt -
`package.json`, `vite.config.ts`, `tsconfig.json`, ein `index.html`
Vite-Einstiegspunkt, Quelle unter `src/`. Das Inertia-Client-Setup lebt in
`src/main.{tsx,ts}` und die Seiten-Komponenten in `src/pages/`. TypeScript-
Typen für Ihre Rust `#[derive(InertiaProps)]` Props werden regeneriert zu
`src/types/inertia-props.ts` durch `suprnova generate-types`.

Siehe [Frontend](frontend.md).

### `public/assets/`

Wo Vite den Produktions-Build ablegt (`npm run build`). Der
Suprnova-Server bedient dieses Verzeichnis als statische Assets unter
`/assets/*` in der Produktion.

## Verzeichnisse, die Sie hinzufügen, wenn die App wächst

Der Scaffolder gibt Ihnen das Minimum - genug um den Welcome-Flow und ein
geschütztes Dashboard zu versenden. Echte Apps wachsen um mehr Subsysteme.
Häufige Ergänzungen:

| Verzeichnis | Wann Sie es hinzufügen |
|---|---|
| `src/jobs/` | Wenn Sie zum ersten Mal `Queue::push(SomeJob)` verwenden. Siehe [Warteschlangen](queues.md). |
| `src/listeners/` | Wenn Sie zum ersten Mal `Event::listen` verwenden. Siehe [Ereignisse](events.md). |
| `src/observers/` | Wenn Sie zum ersten Mal `Observer<MyModel>` implementieren. Siehe [Eloquent](eloquent.md#observers). |
| `src/notifications/` | Wenn Sie zum ersten Mal eine `Notification` implementieren. Siehe [Benachrichtigungen](notifications.md). |
| `src/mail/` | Wenn Sie zum ersten Mal ein `Mailable` implementieren. Siehe [Mail](mail.md). |
| `src/policies/` | Wenn Sie zum ersten Mal ein `#[policy]` schreiben. Siehe [Autorisierung](authorization.md). |
| `src/factories/` | Wenn Sie zum ersten Mal ein `Factory<Model>` für Tests schreiben. Siehe [Eloquent Factories](eloquent-factories.md). |
| `src/seeders/` | Wenn Sie zum ersten Mal ein `Seeder` für `db:seed` schreiben. Siehe [Seeding](seeding.md). |
| `src/events/` | Wenn Sie zum ersten Mal `impl Event` für Ihren eigenen Event-Typ verwenden. Siehe [Ereignisse](events.md). |
| `src/broadcasting/` | Wenn Sie zum ersten Mal einen privaten/Presence-`Channel` definieren. Siehe [Broadcasting](broadcasting.md). |
| `src/ws/` | Wenn Sie zum ersten Mal einen `ws!()` Handler schreiben. Siehe [WebSockets](websockets.md). |
| `src/supervisors/` | Wenn Sie zum ersten Mal einen langlebigen `Supervisor` implementieren. Siehe [Supervisoren](supervisors.md). |
| `src/payments/` | Wenn Sie zum ersten Mal Stripe/Paddle für Ihre App verdrahten. Siehe [Zahlungen](payments.md). |
| `src/props/` | Wenn Sie `#[derive(InertiaProps)]` Strukturen separat von Controllern halten möchten. |
| `resources/views/` | Wenn Sie zum ersten Mal ein Tera-Template für Mail-Texte hinzufügen. |
| `storage/` | Wenn Sie zum ersten Mal Dateien auf den lokalen Dateisystem-Datenträger schreiben (siehe [File Storage](filesystem.md)). |
| `tests/` | Wenn Sie zum ersten Mal einen Integrations-Test schreiben. |

Sie müssen nicht um Erlaubnis fragen - `mkdir src/jobs` und fügen Sie
`pub mod jobs;` zu `src/lib.rs` hinzu, und Sie sind fertig. Das Framework
erzwingt die Verzeichnisnamen nicht; die Konventionen existieren, damit
andere Suprnova-Entwickler Dinge schnell finden.

## Die Dogfood-`app/` in diesem Repo

Wenn Sie dies von innen im Suprnova-Repo lesen, sehen Sie ein `app/`-
Verzeichnis im Root, das jede Framework-Funktion zusammen verwendet. Das
ist unser internes Testbett - es übt Zahlungen, Broadcasting, Web Push,
Workflows, Supervisoren usw. alles auf einmal. Es ist NICHT eine saubere
Referenz für eine neue App; die obige Scaffold-Ausgabe ist bewusst kleiner
und leichter zu lernen. Lesen Sie `app/`, wenn Sie ein maximales Beispiel
sehen möchten, wie die Teile zusammengesetzt werden.

## Nächste Schritte

- [Konfiguration](configuration.md) - wie `.env` zu typisierter Config wird
- [Application Bootstrap](bootstrap.md) - was `bootstrap.rs` tatsächlich
  macht
- [Routing](routing.md) - Ihre erste Route
- [Service Container](container.md) - wie `App::bind` und `App::get`
  funktionieren
