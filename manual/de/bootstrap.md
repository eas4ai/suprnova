# Application-Bootstrap

`bootstrap.rs` ist der zentrale Ort, an dem Ihre Anwendung beim Start
konfiguriert wird. Container-Bindungen, Event-Listener, Observer, Supervisoren
und globale Middleware - alles, was vor der ersten Anfrage an den Server (oder
vor dem ersten Job aus der Queue) existieren soll - wird hier registriert. Es
gibt kein Scaffold aus Service-Providern, das zusammengesetzt werden müsste.

Es gibt zwei Hooks, nicht nur einen. `register` gilt prozessweit: Jeder
Unterbefehl führt ihn aus, einschließlich `queue:work`, `schedule:work`,
`workflow:work` und der Console-Binary, nicht nur der Server. Registrieren Sie
dort die Datenbankverbindung, Container-Bindungen, Event-Listener, Observer,
Supervisoren und die Registrierung von Worker-Jobs. `register_http_stack`,
über `.http_bootstrap` eingebunden, läuft nur auf dem Serverpfad (`serve` /
`web:run`); globale Middleware und `Inertia::install` gehören dorthin. Der
Abschnitt „Wo Bootstrap in der Boot-Reihenfolge steht“ unten erklärt, warum
diese Aufteilung existiert.

## Die Form

Der Einstiegspunkt einer gescaffoldeten Anwendung baut eine
[`Application`](lifecycle.md) fluent auf und führt sie aus. Bootstrap besteht
aus zwei Methoden auf dem Builder:

```rust
// cmd/main.rs
use app::{bootstrap, config, migrations, routes};
use suprnova::Application;

#[suprnova::main]
async fn main() {
    Application::new()
        .config(config::register_all)
        .bootstrap(bootstrap::register)
        .http_bootstrap(|| async { bootstrap::register_http_stack() })
        .routes(routes::register)
        .migrations::<migrations::Migrator>()
        .run()
        .await;
}
```

### `#[suprnova::main]`, nicht `#[tokio::main]`

Das Attribut ist nicht kosmetisch, und es zurückzutauschen bricht den
Boot mit einer Meldung, die erklärt, warum.

Das Laden von `.env` schreibt in die Prozessumgebung, und `set_var`
ist nur sicher, solange der Prozess single-threaded ist.
`#[tokio::main]` baut die Runtime *um* das gesamte `main` herum,
sodass bereits jeder Worker-Thread existiert, bevor Ihre erste
Anweisung läuft - und jeder von ihnen kann `getenv` indirekt über
DNS-Auflösung, Zeitformatierung oder eine C-Abhängigkeit aufrufen.
Diese Race Condition bleibt unbemerkt, wenn sie schiefgeht - und das
ist die schlechteste Eigenschaft, die eine Race Condition haben kann.

`#[suprnova::main]` behält dieselbe `async fn main`, die Sie ohnehin
schreiben würden, und ordnet lediglich zwei Dinge neu an: Es lädt die
Umgebung, baut dann die Runtime auf und führt anschließend Ihren
Funktionskörper darauf aus. Es akzeptiert dieselben `flavor`- und
`worker_threads`-Argumente wie `#[tokio::main]`.

Stellt `Application::run` fest, dass die Umgebung nie aus einem
single-threaded Kontext geladen wurde, verweigert es den Start, statt
nur zu warnen - eine App, die unter `#[tokio::main]` scheinbar
„einwandfrei“ startet, ist genau diejenige, die Wochen später einen
unabhängigen Lesezugriff auf die Umgebung korrumpiert.

Das Framework ruft Ihre `bootstrap_fn` im Boot-Ablauf einmal auf, nachdem die
Umgebung geladen ist und die Runtime-Treiber (Cache, Queue, RateLimit, Mail)
laufen, aber bevor der Router aufgebaut wird. Derselbe Aufruf erfolgt für
Hintergrund-Worker (`queue:work`, `workflow:work`, `schedule:work`), sodass ein
hier registrierter Observer oder Listener sowohl für einen Insert aus einem
Queue-Job als auch für einen Insert aus einem HTTP-Handler identisch ausgelöst
wird. `http_bootstrap_fn` läuft unmittelbar nach `bootstrap_fn`, aber nur auf
dem Serverpfad; Hintergrund-Worker und die Console-Binary rufen ihn nie auf.
[Lifecycle](lifecycle.md) beschreibt die vollständige Reihenfolge.

Die Signaturen beider Funktionen werden durch `Application::bootstrap` und
`Application::http_bootstrap` festgelegt:

```rust
// src/bootstrap.rs
pub async fn register() {
    // database, bindings, observers, listeners, supervisors, worker job registration
}

pub fn register_http_stack() {
    // global middleware, Inertia::install
}
```

`register` gibt `()` zurück; `register_http_stack` ist synchron, nicht `async`.
Beide werden am Aufrufort als asynchrone Closures eingebunden
(`.http_bootstrap(|| async { bootstrap::register_http_stack() })`), weil ein
normaler Funktionszeiger auch als Einstiegspunkt eines Test-Harness dienen kann,
ohne `async` in den Test zu ziehen. Für fehlschlagende Einrichtung verwenden
Sie `.expect("…")` mit einer Meldung, die die Abhilfe erklärt - der Boot ist der
richtige Zeitpunkt, laut fehlzuschlagen. Die Beispielanwendung verwendet
`DB::init().await.expect("Failed to connect to database");`; dadurch beendet
ein fehlendes `DATABASE_URL` den Prozess beim Booten mit dem tatsächlichen
Fehler, statt erst bei der ersten Anfrage als verwirrendes „connection refused“
aufzutauchen.

## Was in bootstrap gehört

Eine echte `bootstrap`-Funktion erledigt eine kleine Anzahl klar
unterschiedener Dinge. Jeder Unterabschnitt unten ist eines davon.
Die `app/src/bootstrap.rs` der Beispiel-App übt alle davon aus und
ist die funktionierende Referenz.

### Datenbankverbindung

```rust
use suprnova::DB;

pub async fn register() {
    DB::init().await.expect("Failed to connect to database");
}
```

`DB::init` liest `DatabaseConfig` (registriert durch Ihre
`config_fn`) und öffnet den Pool. Die Verbindung wird im
[Container](container.md) als Singleton gespeichert -
`DB::connection()` / `DB::get()` löst sie überall auf.
`DB::init_with(config)` ist der Ausweg für Test- und Tooling-Zwecke,
wenn Sie auf etwas anderes zeigen wollen als die aus der Umgebung
abgeleitete URL.

### Magnetar-Authentifizierungs-Engine

Anwendungen, die die integrierten Facades für Passwort, Passkey, Magic Link,
Bearer, Sperre, Remember oder OAuth verwenden, initialisieren Magnetar, nachdem
Datenbank und `APP_KEY` bereitstehen:

```rust
use suprnova::{DB, MagnetarConfig, PasskeyConfig, init_magnetar};

pub async fn register() {
    DB::init().await.expect("Failed to connect to database");

    let database = DB::connection().expect("DB not initialized");
    let config = MagnetarConfig::from_sea_orm(database.inner().clone())
        .passkey_config(PasskeyConfig {
            rp_id: "app.example.com".to_string(),
            rp_origin: "https://app.example.com".to_string(),
        });

    init_magnetar(config)
        .await
        .expect("Failed to initialize Magnetar");
}
```

Die Standard-`MagnetarConfig` bindet Anwendungsidentitäten an die kanonische
Tabelle `app_users`. Das generierte Full-Stack-Scaffold verwendet ein
`users`-Modell und initialisiert Magnetar nicht; fügen Sie diesem Scaffold den
Standard-Initializer daher nicht unverändert hinzu. Verwenden Sie das
`app_users`-Modell des API-Scaffolds oder konstruieren Sie für Ihre vorhandene
`users`-Tabelle eine eigene `MagnetarHostEngine`- und `AuthSchema`-Bindung.
Halten Sie Framework-`UserProvider` und Magnetar-Hostbindung auf derselben
Anwendungsidentität. Das API-Scaffold, nicht `app/src/bootstrap.rs`, ist die
aktuelle Arbeitsreferenz für die Standardinitialisierung von `MagnetarConfig`.

Magnetar gilt prozessweit, weil Queue-Worker, Scheduler, HTTP-Handler und
Session-Middleware dieselben Anmeldedaten- und Session-Speicher verwenden.
Rufen Sie `init_magnetar` in `register` auf, nicht in
`register_http_stack`. Der Installer ist nur einmal ausführbar und schlägt
fehl, wenn bereits eine andere Engine installiert ist.

Das API-Scaffold liest `PASSKEY_RP_ID` und `PASSKEY_RP_ORIGIN` im
Anwendungs-Bootstrap. Diese Namen sind Scaffold-Konventionen und keine
Framework-eigenen Umgebungsvariablen.

### Globale Middleware

Globale Middleware gilt nur für HTTP und gehört daher in
`register_http_stack`, nicht in `register`:


```rust
use suprnova::{global_middleware, SessionMiddleware, SessionConfig, TimeoutMiddleware};
use crate::middleware;

pub fn register_http_stack() {
    global_middleware!(middleware::LoggingMiddleware);
    global_middleware!(TimeoutMiddleware::default());
    global_middleware!(SessionMiddleware::new(SessionConfig::from_env()));
}
```

`global_middleware!` registriert eine Schicht, die bei jeder Anfrage
läuft, auch bei ungerouteten (404s, OPTIONS-Preflight). Die
Reihenfolge, in der Sie registrieren, ist die Reihenfolge, in der die
Chain läuft - von außen nach innen. Das Framework setzt seine eigene
`RequestIdMiddleware` ganz außen ein; alles, was Sie hinzufügen,
sitzt darin. [Middleware](middleware.md) erklärt die vollständige
Chain-Form, einschließlich der Pro-Route-Schicht.

### Container-Bindings

Der Container nimmt, was Sie hineinlegen; die Makros sind Zucker über
der [`App`](container.md)-Facade.

```rust
use std::sync::Arc;
use suprnova::{App, bind, singleton, factory};
use crate::providers::DatabaseUserProvider;

pub async fn register() {
    // Trait → singleton (wraps in Arc):
    bind!(dyn UserProvider, DatabaseUserProvider);

    // Concrete singleton:
    singleton!(MyConfig { max_uploads_per_user: 100 });

    // Factory (constructed per resolve):
    factory!(|| RequestLogger::new());

    // Or call the facade directly for finer control:
    let hub: Arc<dyn BroadcastHub> = Arc::new(InMemoryBroadcastHub::new());
    App::bind::<dyn BroadcastHub>(hub);
}
```

Trait-Objekt-Bindings sind die häufigste Form - binden Sie ein
Interface, und lassen Sie Handler und Tests die Implementierung
austauschen. Das Kapitel [Service Container](container.md) enthält
die vollständige Binding-API einschließlich `bind_factory!`, der
`_if_absent`-Varianten und des Drei-Ebenen-Lookup-Modells.

### Event-Listener und Observer

Der Dispatcher ist aktiv, sobald bootstrap läuft - hier registrierte
Listener sehen jeden nachfolgenden Dispatch.

```rust
use std::sync::Arc;
use suprnova::EventFacade;
use crate::events::UserRegistered;
use crate::listeners::SendWelcomeEmailListener;

pub async fn register() {
    EventFacade::listen::<UserRegistered, _>(
        Arc::new(SendWelcomeEmailListener),
    ).await;
}
```

Eloquent-Observer (`#[suprnova::observer(M)]`) sammeln sich selbst
über `inventory::submit!` zur Compile-Zeit. Ein einzelner Aufruf
leert das Inventory in den Dispatcher:

```rust
suprnova::eloquent::observers::bootstrap_observers()
    .await
    .expect("observer install failed");
```

Der Aufruf ist idempotent - ein erneutes Ausführen von bootstrap (ein
Worker, der ein zweites Mal bootet) registriert die
Listener-Adapter nicht doppelt. [Ereignisse](events.md) behandelt
Dispatch und das Schreiben von Listenern; [Eloquent API](eloquent.md)
behandelt Observer.

### Supervisoren

Lang laufende Background-Tasks, die über den `Supervisor`-Trait und
`inventory::submit!` deklariert werden, starten durch einen einzigen
Aufruf:

```rust
use suprnova::SupervisorRegistry;

pub async fn register() {
    SupervisorRegistry::start_all().await;
}
```

Jeder Supervisor läuft in einer eigenen Restart-Loop-Task mit einer
Panic-Grenze; ein Supervisor, der in Panic gerät, wird protokolliert
und neu gestartet, statt den Prozess mit sich zu reißen. Siehe
[Supervisoren](supervisors.md) für den Trait und die
Restart-Richtlinie.

### Worker-Job-Registrierung

Queue-Jobs und Mailables, die Worker über ihren Namen dispatchen
müssen, registrieren sich selbst beim Boot:

```rust
use suprnova::queue::worker::register_job;

pub async fn register() {
    register_job::<crate::jobs::welcome_log::WelcomeLog>();

    suprnova::mail::register_mailable_factory::<crate::mail::welcome::WelcomeEmail>()
        .expect("register at boot");
    register_job::<suprnova::mail::send_job::SendMailJob>();
}
```

Ohne das hat der Worker keine Möglichkeit, einen in die Queue
gestellten Envelope zurück auf den Typ abzubilden, der ihn behandelt.

## Der Post-Boot-Hook: `booted()`

Bootstrap *registriert*; `booted()` *löst auf*. Der Builder nimmt
einen zweiten Callback entgegen, der feuert, nachdem der Server
seinen eigenen Service-Boot abgeschlossen hat, aber bevor er beginnt,
Verbindungen anzunehmen. Verwenden Sie ihn, wenn Sie etwas lesen
müssen, das das Framework selbst während des Boots gebunden hat:

```rust
Application::new()
    .config(config::register_all)
    .bootstrap(bootstrap::register)
    .http_bootstrap(|| async { bootstrap::register_http_stack() })
    .routes(routes::register)
    .booted(|| {
        let cfg: MyConfig = suprnova::App::get().unwrap();
        tracing::info!(?cfg, "services booted");
    })
    .run()
    .await;
```

`booted` ist synchron und läuft nach `Server::from_config` - Treiber
sind hochgefahren, Verschlüsselungsschlüssel sind geladen, Ihre
Bindings existieren. Die meisten Apps brauchen diesen Hook nicht;
greifen Sie darauf zurück, wenn ein einmaliger
Post-Boot-Seiteneffekt einen vollständig konstruierten Container
sehen muss.

## Eine vollständige `bootstrap.rs`

Diese repräsentative Zusammensetzung ist kein wörtlicher Auszug aus der
Beispielanwendung. Sie hält die prozessweite Registrierung in `register` und
das reine HTTP-Setup in `register_http_stack`. Die Magnetar-Initialisierung ist
oben separat dargestellt, weil ihr Anwendungsschema für Benutzer zum
Framework-UserProvider passen muss.

```rust
//! Application bootstrap - register services, listeners, global
//! middleware, and the Inertia layer.

use std::sync::Arc;
use std::time::Duration;

use suprnova::broadcasting::{BroadcastHub, ChannelRegistry, InMemoryBroadcastHub};
use suprnova::features::{FeatureMiddleware, bootstrap_database_cached};
use suprnova::queue::worker::register_job;
use suprnova::{
    App, DB, EloquentUserProvider, EventFacade, FrameworkError, Inertia,
    InertiaConfig, SessionConfig, SessionMiddleware, Storage, SupervisorRegistry,
    UserProvider, bind, global_middleware,
};

use crate::broadcasting::ChatChannel;
use crate::events::UserRegistered;
use crate::listeners::SendWelcomeEmailListener;
use crate::middleware;
use crate::models::users::User;

pub async fn register() {
    // ── Database
    DB::init().await.expect("Failed to connect to database");

    // ── Auth provider
    bind!(dyn UserProvider, EloquentUserProvider::<User>::new());


    // ── Broadcasting hub + channel registry
    let hub: Arc<dyn BroadcastHub> = Arc::new(InMemoryBroadcastHub::new());
    App::bind::<dyn BroadcastHub>(Arc::clone(&hub));

    let mut registry = ChannelRegistry::new();
    registry.register(ChatChannel);
    App::singleton(Arc::new(registry));

    // ── Event listeners + bridges
    EventFacade::listen::<UserRegistered, _>(
        Arc::new(SendWelcomeEmailListener),
    ).await;
    EventFacade::broadcast::<UserRegistered>(Arc::clone(&hub)).await;

    // ── Storage disks (env-gated S3 in production)
    Storage::register_fs("public", "./storage/public")
        .expect("register public disk");

    // ── Worker job registration
    register_job::<crate::jobs::welcome_log::WelcomeLog>();
    suprnova::mail::register_mailable_factory::<crate::mail::welcome::WelcomeEmail>()
        .expect("register at boot");
    register_job::<suprnova::mail::send_job::SendMailJob>();

    // ── Observers + supervisors
    suprnova::eloquent::observers::bootstrap_observers()
        .await
        .expect("observer install failed");
    SupervisorRegistry::start_all().await;

    // ── Feature flags
    bootstrap_database_cached(Duration::from_secs(60))
        .await
        .expect("feature-flag chain wired");
}

pub fn register_http_stack() {
    // ── Global middleware (outside-in in registration order)
    global_middleware!(middleware::LoggingMiddleware);
    global_middleware!(suprnova::TimeoutMiddleware::default());
    global_middleware!(SessionMiddleware::new(SessionConfig::from_env()));

    // ── Inertia protocol layer (no version pin: the default hashes the
    // Vite build manifest, so a frontend build bumps the asset version
    // on its own - see "Version detection" in frontend-inertia-responses.md)
    Inertia::install(&InertiaConfig::new()).expect("Inertia install failed");

    global_middleware!(FeatureMiddleware::new());
}
```

Beachten Sie den Rhythmus: Jeder Block macht eine Sache, ruft eine oder zwei API an, und entweder gelingt es oder scheitert es mit einer klaren Botschaft. Nichts hier ist klug. Die Funktionen sind lang, weil die App viele bewegliche Teile hat, nicht weil das Bootstrap-Muster kompliziert ist.

## Wann bootstrap, wann `#[injectable]`

`#[injectable]` ist ein Makro, das zur Compile-Zeit automatisch ein
Singleton im `inventory` des Containers registriert. Es ist die
richtige Wahl für Services, die zur Konstruktion nichts weiter als
ihre `#[inject]`-Abhängigkeiten brauchen:

```rust
use suprnova::injectable;

#[injectable]
pub struct UserService;

#[injectable]
pub struct OrderService {
    #[inject]
    user_service: UserService,
}
```

Diese werden automatisch aufgelöst; bootstrap muss sie nicht anfassen.

Bootstrap ist der richtige Ort, wenn die Konstruktion sonst noch
etwas braucht - eine Umgebungsvariable, eine konstruierte
Config-Struktur, ein `dyn Trait`-Binding, eine Laufzeitentscheidung,
einen asynchronen Setup-Aufruf oder die Registrierung von etwas, das
selbst kein Service ist (ein Listener, ein Observer, eine
Queue-Job-Zuordnung, eine globale Middleware-Schicht).

| Verwenden Sie `#[injectable]` für | Verwenden Sie `bootstrap` für |
|---|---|
| Konkrete Singletons ohne Laufzeit-Konfiguration | Alles mit `dyn Trait` |
| Services, die aus anderen Injectables konstruiert werden | Alles Asynchrone beim Boot |
| Standard-DI-Graph | Umgebungsabhängige Werte |
| | Event-Listener, Observer, Supervisoren |
| | Globale Middleware |
| | Worker-Job- und Mailable-Registrierung |

Sie können beides frei mischen. `#[injectable]`-Services sind im
Container bereits sichtbar, wenn `bootstrap` läuft, sodass ein
Binding in bootstrap sie lesen kann.

## Wo bootstrap in der Boot-Reihenfolge sitzt

Die vollständige Sequenz (Auszug aus
[Request-Lifecycle](lifecycle.md)):

1. `Config::init(".")` - lädt `.env`, erkennt die Umgebung
2. `init_policies()` - leert das `#[policy]`-Inventory
3. Ihre `config_fn` läuft (typisierte Konfigurationsregistrierung)
4. Migrationen laufen (Auto-Migration bei `serve`)
5. **Ihre `bootstrap_fn` läuft** ← `bootstrap::register`
6. **Ihre `http_bootstrap_fn` läuft, nur auf dem Serverweg** ← `bootstrap::register_http_stack`
7. Routen, die von Ihrem `routes_fn` zusammengestellt wurden
8. `Server::from_config` Stiefelfahrer + Behälter
9. Ihr `booted_fn`s Feuer
10. Server beginnt Verbindungen zu akzeptieren

Hintergrundarbeiter (`queue:work`) `workflow:work` `schedule:work`) und die Konsole binäre Teil Schritte 1-5 und 8 - sie laufen `bootstrap_fn`, aber nie Schritt 6, da nur `serve` / `web:run` `http_bootstrap_fn` läuft. Das ermöglicht einem Zuhörer oder Beobachter, den Sie in `register` registrieren, die Workers-Code-Pfaden genau so zu erreichen, wie sie HTTP-Handler erreicht, während die globale Middleware von `register_http_stack` und `Inertia::install` Prozesse, die niemals HTTP bedienen, abhalten.

### Warum Suprnova abweicht

Laravel führt die `register()` und `boot()` von jedem Dienstleister für `artisan`-Befehle und Warteschlangen auch, nicht nur für HTTP-Anfragen - und kommt damit durch, weil seine Vite-Integration Assets-URLs faul, zu Render-Zeiten löst, von allem, was die `@vite`-Blade-Richtlinie verlangt. Ein Worker, der nie eine Aussicht darstellt, berührt das Manifest nie, so dass ein fehlender Bau einfach nie auftaucht.

Suprnova's `Inertia::install` löst das Manifest einmal beim Booten und schließt sich nicht in der Produktion, wenn es fehlt - nach Design, so dass eine falsch konfigurierte Bereitstellung nicht die URLs für Assets anzeigen kann, die auf einen Vite-Dev-Server zeigen, den niemand ausführt. Diese Auswahl des Designs ist genau das, was ein Worker- oder Konsolbild, das (richtig) keine `public/assets` versendet, bricht: Wenn Laravel die Zeit nicht verlangt, würde Suprnova sonst bei Prozessstart auf jedem Unterbefehl getroffen. Die Aufteilung der Boot-Oberfläche in `bootstrap` und `http_bootstrap` hält die Fehler-Schließung, aber nur dort, wo sie hingehört - den Serverweg, der tatsächlich eine Inertia-Seite darstellt.

Laravel spaltet sich auch auf mehrere Dienstleister: Jeder Dienstleister implementiert `register()` und `boot()`, sie werden in `config/app.php` gesammelt, und Laravel führt sie in zwei Passes (alle `register`, dann alle `boot`) so dass ein Dienst auf die Bindungen eines anderen Anbieters angewiesen sein kann, ohne eine Bestellzeremonie im Benutzercode zu bestellen. Die Provider-Klasse gibt Ihnen eine Organisationseinheit, wenn eine App Dutzende verschiedener Teilsysteme ansammelt.

Suprnova zerfällt auf zwei Funktionen - `register` und `register_http_stack` - anstelle eines `register`/`boot` Paares pro Anbieter. Die Gründe:

- **Die Zweiphasen-Aufteilung `register`/`boot` löst ein
  Reihenfolge-Problem, das es in Rust gar nicht gibt.**
  `#[injectable]` und die `bootstrap_singletons` des Containers lösen
  Abhängigkeitsgraphen bereits ohne für den Nutzer sichtbare
  Reihenfolge auf. Bindings registrieren sich inline; die
  Lookup-Mechanik erledigt den Rest.
- **Zwei Funktionen sind leichter zu lesen als zehn.** Ein neuer Bewerber öffnet `bootstrap.rs` und sieht jede Bindung, jeden Zuhörer, jeden Beobachter, jede Middleware-Schicht an einem von zwei Orten. Die Splitterung im Anbieter-Stil verbirgt, was die App tatsächlich tut.
- **Automatische Registrierung im Inventory-Stil deckt den Rest ab.**
  Observer, Supervisoren, geplante Tasks, Policies und Queue-Handler
  sammeln sich alle zur Compile-Zeit selbst über
  `inventory::submit!`. Bootstrap leert die Inventories mit
  einzelnen Aufrufen (`bootstrap_observers`,
  `SupervisorRegistry::start_all`), statt jeden einzeln aufzuzählen.

Wo sich die Provider-Aufteilung bei Laravel auszahlt, ist die
Verteilung von Libraries: Eine Crate, die eigene Bindings mitbringt,
würde einen Registrierungs-Einstiegspunkt wollen, den eine App ohne
Änderungen am eigenen bootstrap dazuschalten kann. Suprnovas Analogon
ist eine öffentliche `pub async fn register()` an der Wurzel der
Crate und ein einzeiliger Aufruf aus dem `bootstrap` der App. Der
ergonomische Preis ist eine Zeile; der Nutzen an Lesbarkeit ist
alles an einem Ort.

## Nächste Schritte

- [Request-Lifecycle](lifecycle.md) - vollständige Boot-Reihenfolge
  und wo `bootstrap_fn` feuert
- [Service Container](container.md) - `App::bind` / `App::singleton` /
  `App::factory` und der Drei-Ebenen-Lookup
- [Konfiguration](configuration.md) - typisierte
  Konfigurationsregistrierung, die vor bootstrap läuft
- [Middleware](middleware.md) - Chain-Komposition für Schichten, die
  mit `global_middleware!` registriert werden
- [Ereignisse](events.md) - der Dispatcher, in den sich Listener und
  Observer einhängen
