# Application Bootstrap

`bootstrap.rs` ist die eine Stelle, an der sich Ihre Anwendung beim
Start selbst verdrahtet. Container-Bindings, Event-Listener, Observer,
Supervisoren, globale Middleware - alles, was existieren soll, bevor
die erste Anfrage den Server erreicht (oder der erste Job aus der
Queue geholt wird), wird innerhalb einer einzigen asynchronen
`bootstrap`-Funktion registriert. Es gibt kein
Service-Provider-Scaffold zusammenzusetzen; eine Funktion, einmal
ausgeführt, ist die gesamte API.

## Die Form

Der Einstiegspunkt einer per Scaffold erzeugten App baut fließend eine
[`Application`](lifecycle.md) auf und führt sie aus. Der
`bootstrap`-Schritt ist eine Methode auf dem Builder:

```rust
// cmd/main.rs
use app::{bootstrap, config, migrations, routes};
use suprnova::Application;

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

Das Framework ruft Ihre `bootstrap_fn` einmal während der
Boot-Sequenz auf - nachdem die Umgebung geladen ist und die
Runtime-Treiber (Cache, Queue, RateLimit, Mail) laufen, aber bevor der
Router gebaut wird. Derselbe Aufruf läuft auch für Background-Worker
(`queue:work`, `workflow:work`, `schedule:work`), sodass ein hier
registrierter Observer oder Listener für einen Insert aus einem
Queue-Job genauso feuert wie für einen Insert aus einem
HTTP-Handler. [Request-Lifecycle](lifecycle.md) durchläuft die
vollständige Sequenz.

Die Signatur der Funktion ist durch `Application::bootstrap`
festgelegt:

```rust
// src/bootstrap.rs
pub async fn register() {
    // Bindings, Observer, Listener, Supervisoren, globale Middleware
}
```

Sie gibt `()` zurück. Fehlbares Setup verwendet `.expect("…")` mit
einer Meldung, die die Abhilfe erklärt - der Boot ist der richtige
Zeitpunkt, um sichtbar zu scheitern. Der Aufruf der Beispiel-App
ist `DB::init().await.expect("Failed to connect to database");`,
sodass eine fehlende `DATABASE_URL` den Prozess beim Boot mit dem
tatsächlichen Fehler abbricht, statt bei der ersten Anfrage als
verwirrendes „Connection refused“ aufzutauchen.

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

### Globale Middleware

```rust
use suprnova::{global_middleware, SessionMiddleware, SessionConfig, TimeoutMiddleware};
use crate::middleware;

pub async fn register() {
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
    // Trait → Singleton (verpackt in Arc):
    bind!(dyn UserProvider, DatabaseUserProvider);

    // Konkretes Singleton:
    singleton!(MyConfig { max_uploads_per_user: 100 });

    // Factory (wird bei jedem Resolve konstruiert):
    factory!(|| RequestLogger::new());

    // Oder rufen Sie die Facade direkt auf, für feinere Kontrolle:
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

Eine gekürzte, aber repräsentative Form, entnommen aus der
Beispiel-App:

```rust
//! Anwendungs-Bootstrap - registriert Services, Listener und
//! globale Middleware.

use std::sync::Arc;
use std::time::Duration;

use suprnova::broadcasting::{BroadcastHub, ChannelRegistry, InMemoryBroadcastHub};
use suprnova::features::{FeatureMiddleware, bootstrap_database_cached};
use suprnova::queue::worker::register_job;
use suprnova::{
    App, DB, EventFacade, FrameworkError, Inertia, InertiaConfig,
    SessionConfig, SessionMiddleware, Storage, SupervisorRegistry,
    UserProvider, bind, global_middleware,
};

use crate::broadcasting::ChatChannel;
use crate::events::UserRegistered;
use crate::listeners::SendWelcomeEmailListener;
use crate::middleware;
use crate::providers::DatabaseUserProvider;

pub async fn register() {
    // ── Datenbank
    DB::init().await.expect("Failed to connect to database");

    // ── Globale Middleware (von außen nach innen, in Registrierungsreihenfolge)
    global_middleware!(middleware::LoggingMiddleware);
    global_middleware!(suprnova::TimeoutMiddleware::default());
    global_middleware!(SessionMiddleware::new(SessionConfig::from_env()));

    // ── Auth-Provider
    bind!(dyn UserProvider, DatabaseUserProvider);

    // ── Inertia-Protokollschicht
    Inertia::install(&InertiaConfig::new().version("1.0")).expect("Inertia install failed");

    // ── Broadcasting-Hub + Channel-Registry
    let hub: Arc<dyn BroadcastHub> = Arc::new(InMemoryBroadcastHub::new());
    App::bind::<dyn BroadcastHub>(Arc::clone(&hub));

    let mut registry = ChannelRegistry::new();
    registry.register(ChatChannel);
    App::singleton(Arc::new(registry));

    // ── Event-Listener + Bridges
    EventFacade::listen::<UserRegistered, _>(
        Arc::new(SendWelcomeEmailListener),
    ).await;
    EventFacade::broadcast::<UserRegistered>(Arc::clone(&hub)).await;

    // ── Storage-Disks (env-gesteuertes S3 in Produktion)
    Storage::register_fs("public", "./storage/public")
        .expect("register public disk");

    // ── Worker-Job-Registrierung
    register_job::<crate::jobs::welcome_log::WelcomeLog>();
    suprnova::mail::register_mailable_factory::<crate::mail::welcome::WelcomeEmail>()
        .expect("register at boot");
    register_job::<suprnova::mail::send_job::SendMailJob>();

    // ── Observer + Supervisoren
    suprnova::eloquent::observers::bootstrap_observers()
        .await
        .expect("observer install failed");
    SupervisorRegistry::start_all().await;

    // ── Feature Flags
    bootstrap_database_cached(Duration::from_secs(60))
        .await
        .expect("feature-flag chain wired");
    global_middleware!(FeatureMiddleware::new());
}
```

Beachten Sie den Rhythmus: Jeder Block tut eine Sache, ruft eine oder
zwei APIs auf und gelingt entweder oder scheitert mit einer klaren
Meldung. Nichts davon ist raffiniert; die Funktion ist lang, weil die
App viele bewegliche Teile hat, nicht weil das Bootstrap-Muster
kompliziert ist.

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
6. Routen werden aus Ihrer `routes_fn` zusammengestellt
7. `Server::from_config` bootet Treiber + Container
8. Ihre `booted_fn`s feuern
9. Der Server beginnt, Verbindungen anzunehmen

Background-Worker (`queue:work`, `workflow:work`, `schedule:work`)
teilen sich die Schritte 1-5 und 7, sodass ein Listener oder
Observer, den Sie registrieren, Worker-Codepfade genauso erreicht wie
HTTP-Handler.

### Warum Suprnova abweicht

Laravel verteilt den Boot über mehrere Service Provider: Jeder
Provider implementiert `register()` und `boot()`, sie werden in
`config/app.php` gesammelt, und Laravel durchläuft sie in zwei
Durchgängen (zuerst alle `register`, dann alle `boot`), sodass ein
Service von den Bindings eines anderen Providers abhängen kann, ohne
Reihenfolge-Zeremonie im eigenen Code. Die Provider-Klasse gibt Ihnen
eine Organisationseinheit, wenn eine App Dutzende unterschiedlicher
Subsysteme ansammelt.

Suprnova reduziert das auf eine einzige Funktion. Die Gründe:

- **Die Zweiphasen-Aufteilung `register`/`boot` löst ein
  Reihenfolge-Problem, das es in Rust gar nicht gibt.**
  `#[injectable]` und die `bootstrap_singletons` des Containers lösen
  Abhängigkeitsgraphen bereits ohne für den Nutzer sichtbare
  Reihenfolge auf. Bindings registrieren sich inline; die
  Lookup-Mechanik erledigt den Rest.
- **Eine Funktion ist leichter zu lesen als zehn.** Ein neuer
  Mitwirkender öffnet `bootstrap.rs` und sieht jedes Binding, jeden
  Listener, jeden Observer, jede Middleware-Schicht an einem Ort.
  Provider-artige Fragmentierung verschleiert, was die App
  tatsächlich tut.
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
