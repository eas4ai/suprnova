# Von Laravel kommend

Wenn Sie Laravel-Anwendungen ausgeliefert haben, kennen Sie bereits 80% von Suprnova. Dieses
Kapitel ordnet Ihre Gewohnheiten dem Rust-Äquivalent zu, damit Sie
schnell produktiv werden können. Wir zeigen die Muster, die Sie täglich verwenden, die Muster, die
eine andere Form annehmen, und einige wenige Dinge, die Ihnen Rust kostenlos gibt, die PHP nicht kann.

## Kurzübersicht Seite an Seite

| Sie haben in Laravel geschrieben | Sie schreiben in Suprnova |
|---|---|
| `composer create laravel/laravel my-app` | `suprnova new my-app --frontend svelte` |
| `php artisan serve` | `suprnova serve` |
| `php artisan migrate` | `suprnova migrate` |
| `php artisan make:controller PostController` | `suprnova make:controller post` |
| `Route::get('/posts/{id}', [PostController::class, 'show'])` | `get!("/posts/{id}", controllers::post::show)` (in `routes!`) |
| `class Post extends Model` | `#[suprnova::model] struct Post { … }` |
| `Post::find($id)` | `Post::find(id).await?` |
| `Post::where('status', 'published')->get()` | `Post::query().db_where("status", "published").get().await?` |
| `Auth::user()` | `Auth::user().await?` |
| `Cache::remember('key', 60, fn() => …)` | `Cache::remember("key", Some(Duration::from_secs(60)), \|\| async { … }).await?` |
| `Queue::push(new SendEmail($user))` | `Queue::push(SendEmail { user_id }).await?` |
| `Mail::to($u)->send(new Welcome($u))` | `Mail::to(&u.email).send(WelcomeMail { user: u }).await?` |
| `Storage::disk('s3')->put($path, $bytes)` | `Storage::disk("s3")?.put(&path, bytes).await?` |
| `Notification::send($u, new Invoice($i))` | `Notify::send(&u, &InvoiceNotification { invoice }).await?` |
| `Gate::allows('update', $post)` | `Gate::allows::<PostPolicy, _>("update", &user, &post).await?` |
| `request()->validate([...])` | `#[handler]` extrahiert ein `#[derive(Data, Validate)]` Argument direkt |
| `event(new OrderShipped($order))` | `EventFacade::dispatch(OrderShipped { order }).await?` |
| `Bus::dispatch(new ProcessFoo($x))` | `Bus::dispatch(ProcessFoo { x }).await?` |
| `php artisan schedule:list` | `suprnova schedule:list` |
| `php artisan tinker` | (kein REPL - schreiben Sie ein einmaliges `cargo run` Skript oder Test) |
| `composer require league/csv` | `cargo add csv` |

## Der Mentalitätswechsel

### Async, überall

Die größte Änderung: Jeder Datenbankaufruf, HTTP-Aufruf, Datei-I/O, Cache-Aufruf,
Queue-Push - alles, das eine Grenze überschreitet - ist `async` und Sie rufen
es mit `.await?` auf. Wenn Sie es ein paar Stunden gemacht haben, verschwindet es
in den Rhythmus. Bis dahin wird der Compiler Sie auf jede Stelle hinweisen, die Sie vergessen haben.

```rust
// Laravel
$user = User::find($id);
$user->subscribe($plan);
Mail::to($user)->send(new Welcome($user));

// Suprnova
let user = User::find(id).await?;
user.subscribe(&plan).await?;
Mail::to(&user.email).send(WelcomeMail { user }).await?;
```

`?` ist Rusts "frühe Rückkehr bei Fehler". Ein Handler gibt
`Result<HttpResponse, HttpResponse>` zurück (aliased als `Response`), also wird ein `?`
bei einem DB-Fehler in Ihren Error-Converter kurzgeschlossen und der Client
erhält einen korrekten 500er (oder 4xx, abhängig von der Fehlerart). Sie müssen fast
nie `try/catch` schreiben - `?` macht das.

### Modelle zur Compilezeit

Wo Eloquent Ihr DB-Schema zur Laufzeit liest, liest Suprnova es zur
Compilezeit:

```rust
#[suprnova::model(table = "posts")]
pub struct Post {
    pub id: i64,
    pub title: String,
    pub body: String,
    pub published_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

Das ist es - diese Struktur IST das Eloquent-Modell. Sie erhalten
`Post::find`, `Post::query()`, `Post::create`, `post.update(...)`,
`post.delete()`, Soft Deletes (mit `#[model(soft_deletes)]`),
Timestamps, Observer, alles. Das Makro generiert einen SeaORM
`Entity`, `Model`, `ActiveModel` und `Column` Enum und implementiert das
Suprnova `Model` Trait - aber Sie hängen von `Post` ab, nicht von irgendeinem der anderen.

Wenn Sie eine Spalte in einer Migration umbenennen, passt die Struktur nicht mehr zum
DB-Schema - und je nach Konfiguration fängt entweder der Compiler es zur Buildzeit,
oder die typgezwungene Cast schlägt fehl beim ersten
Query. Auf jeden Fall erfahren Sie es vor der Bereitstellung, nicht danach.

### Einzelne Binärdatei

Es gibt keine PHP-FPM, keine nginx-Konfiguration, die `index.php` liest, kein `composer
install` bei der Bereitstellung. `cargo build --release` gibt Ihnen eine statisch
verlinkte Binärdatei. `scp` sie auf einen Server, `systemd` sie, fertig. Oder bauen Sie einen
Container - `FROM scratch` funktioniert.

Wir haben [Bereitstellungsanleitungen](deployment.md) für Railway, Digital
Ocean und Hetzner. Die allgemeine Form: Binärdatei bauen, Binärdatei versenden, Umgebungsvariablen setzen, ausführen.

## Das Framework abbilden

### Routing

`routes!` spielt die Rolle von `routes/web.php` und `routes/api.php`
kombiniert.

```rust
use suprnova::{routes, get, post, put, delete};
use crate::controllers;

routes! {
    get!("/", controllers::home::index).name("home"),

    // Route-Gruppe mit gemeinsamen Präfix + Middleware
    group("/admin")
        .middleware(crate::middleware::admin())
        .routes(routes! {
            get!("/users", controllers::admin::users::index).name("admin.users"),
            post!("/users", controllers::admin::users::store),
            put!("/users/{id}", controllers::admin::users::update),
            delete!("/users/{id}", controllers::admin::users::destroy),
        }),

    // Resource-Routing (Laravels Route::resource)
    resource!("posts", controllers::post),
}
```

Vollständige Referenz: [Routing](routing.md). Unterschiede, die wissenswert sind:

- Gruppen-Middleware wird **flattened** in die Middleware-Liste jeder Route
  bei der Registrierung (nicht als separate Chain-Ebene ausgeführt) - das bedeutet,
  es gibt keine zusätzlichen Laufzeitkosten für Gruppierung.
- Sowohl Laravels `{id}` als auch Rails-Stil `:id` Syntax funktionieren; sie werden
  intern normalisiert.
- Benannte Routes werden aufgelöst via `route("posts.show", &[("id", "42")])` und
  es gibt eine signierte-URL-Variante für zeitbegrenzte Links.

### Controller

Ein Controller ist nur eine freie Funktion, die `Response` zurückgibt:

```rust
use suprnova::{Request, Response, json_response, HttpResponse};
use crate::models::Post;

pub async fn show(req: Request) -> Response {
    let id = req.param("id").unwrap_or("0").parse::<i64>()?;
    let post = Post::find_or_fail(id).await?;
    json_response!({ "post": post })
}
```

Sie können auch das `#[handler]` Makro verwenden, um typisierte Argumente zu extrahieren (Route
Parameter, Query, Body, die Request selbst, Container-Services) in der
Signatur:

```rust
use suprnova::handler;

#[handler]
pub async fn show(post: post::Model) -> Response {
    // Route Model Binding lief automatisch; `post` ist die geladene Zeile.
    json_response!({ "post": post })
}
```

Der Typ `post::Model` kommt aus dem generierten Modul des Modells - das ist
das Signal, das `#[handler]` verwendet, um Route Model Binding über die
Standard-Formular-Anfrage-Extraktion zu wählen. Wenn die Zeile nicht existiert, gibt die Bindung
einen 404 zurück, bevor Ihr Code läuft - gleiches Verhalten wie Laravels
implizite Bindung.

Action-Strukturen (Single-Method "invokable" Controller, Laravel-Stil) werden
auch unterstützt: siehe [Aktionen](actions.md).

### Eloquent

Der Dual-API Query Builder akzeptiert entweder Laravel-Namen oder Rust-idiomatische
Namen - beide funktionieren, wählen Sie, was an der Aufrufstelle sauber lesbar ist.

```rust
// Laravel-Oberfläche
let active = User::query()
    .db_where("status", "active")
    .order_by_desc("created_at")
    .limit(20)
    .get()
    .await?;

// Rust-Oberfläche (identisches Ergebnis)
let active = User::query()
    .filter("status", "active")
    .order_by_desc("created_at")
    .take(20)
    .get()
    .await?;
```

`db_where` ist der Laravel-Name (das bloße `where` kollidiert mit dem
Rust-Schlüsselwort). `filter` ist der Rust-idiomatische Alias. Beide existieren; beide
machen das Gleiche. Für Nicht-Gleichheits-Operatoren greifen Sie zu `db_where_op`
(oder dessen `filter_op` Alias): `.db_where_op("status", "!=", "archived")`.
Siehe die [Eloquent-Referenz](eloquent.md) - es ist das längste Kapitel
aus gutem Grund, die Oberfläche ist breit.

### Auth

```rust
use suprnova::{Auth, Credentials};

// In einem Handler:
let user = Auth::user().await?;   // Option<Arc<dyn Authenticatable>>
let id = user.as_ref().map(|u| u.get_auth_identifier());

// Anmelden (z.B. innerhalb Ihres Login-Controllers):
let creds = Credentials::password("alice@x.com", "secret");
Auth::attempt(&creds, false).await?;

// Abmelden:
Auth::logout().await?;
```

`Auth::attempt` validiert Anmeldedaten über den zustandsbehafteten Standard-Guard und dessen konfigurierten `UserProvider`; dies ist der Pfad, den das generierte Full-Stack-Scaffold verwendet. Die Passwortzurücksetzung unterstützt bereits verifizierte Benutzer über einen explizit zum Zurücksetzen befähigten Provider wie `EloquentUserProvider`. Installieren Sie Magnetar, wenn das Zurücksetzen als atomarer erstmaliger Postfachnachweis dienen muss. `Auth::password()`, `BruteForce`, Passkeys, Magic Links, OAuth, Bearer-Sitzungen und die Magnetar-Sitzungsverwaltung erfordern die installierte Magnetar-Engine. See [Authentication](authentication.md), [Auth flows](auth-flows.md), and [OAuth and passwordless login](oauth.md).

### Migrationen

Sie schreiben SeaORM-Migratoren. Die Form sieht vertraut aus, auch wenn die
Syntax neu ist:

```rust
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.create_table(
            Table::create()
                .table(Alias::new("posts"))
                .if_not_exists()
                .col(ColumnDef::new(Alias::new("id")).big_integer().primary_key().auto_increment())
                .col(ColumnDef::new(Alias::new("title")).string().not_null())
                .col(ColumnDef::new(Alias::new("body")).text().not_null())
                .to_owned()
        ).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(Alias::new("posts")).to_owned()).await
    }
}
```

`suprnova make:migration create_posts_table` scaffoldet die Datei.
`suprnova migrate`, `migrate:rollback`, `migrate:status`, `migrate:fresh`
alle machen, was Sie erwarten würden. `suprnova db:sync` führt Migrationen aus und
regeneriert die SeaORM-Entitäten, die die Makroebene kompiliert.
Siehe [Migrationen](migrations.md).

### Warteschlangen und Planung

```rust
use suprnova::{FrameworkError, Job, Queue, async_trait};
use serde::{Deserialize, Serialize};

// Definieren Sie einen Job - die Daten leben auf der Struktur, der Vertrag lebt auf
// `impl Job`.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SendWelcomeEmail {
    pub user_id: i64,
}

#[async_trait]
impl Job for SendWelcomeEmail {
    fn job_name() -> &'static str {
        "SendWelcomeEmail"
    }

    async fn handle(self) -> Result<(), FrameworkError> {
        let user = User::find_or_fail(self.user_id).await?;
        Mail::to(&user.email).send(WelcomeMail { user }).await?;
        Ok(())
    }
}

// Pushen Sie es in die Warteschlange:
Queue::push(SendWelcomeEmail { user_id: user.id }).await?;

// Oder mit einer Verzögerung:
Queue::later(
    std::time::Duration::from_secs(60),
    SendWelcomeEmail { user_id },
).await?;
```

Worker werden mit `cargo run -- queue:work` ausgeführt. Treiber umfassen
Memory und Sync (In-Process, für Tests), Datenbank, Redis und Null.
Batches, Chains, Unique Jobs, Wiederholungen, Backoff, Middleware, Failed-Job
Store - alles da. Siehe [Warteschlangen](queues.md).

Planung verwendet das `Task` Trait und die Pro-Projekt Scheduler-Binärdatei:

```rust
use suprnova::{Task, TaskResult, async_trait};

pub struct DailyDigest;

#[async_trait]
impl Task for DailyDigest {
    async fn handle(&self) -> TaskResult {
        // …
        Ok(())
    }
}

// Registrieren Sie im Bootstrap (z.B. via Schedule::call / .task / .add):
//   schedule.add(schedule.task(DailyDigest).daily().at("03:00").name("daily-digest"));
```

Siehe [Task-Planung](scheduling.md).

### Mail, Benachrichtigungen, Broadcasting

Diese folgen Laravel eins zu eins. `Mailable` ist ein Derive-Makro;
`Notifiable` ist ein Trait auf Ihrem User-Modell; Channels sind
`mail`/`database`/`broadcast`/`webpush`; Broadcasting unterstützt
öffentliche, private und Presence-Channels. Siehe [Mail](mail.md),
[Benachrichtigungen](notifications.md), [Broadcasting](broadcasting.md).

### Frontend

Es gibt kein Blade. Stattdessen ist das Frontend ein echtes SPA via Inertia.js,
und Sie übergeben typisierte Props aus Rust:

```rust
use suprnova::{inertia_response, InertiaProps, Request, Response};

#[derive(InertiaProps, serde::Serialize)]
pub struct ShowProps {
    pub post: Post,
    pub comments: Vec<Comment>,
}

pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id").unwrap_or("0").parse().unwrap_or(0);
    let post = Post::find_or_fail(id).await?;
    let comments = post.comments().get().await?;
    inertia_response!(&req, "Posts/Show", ShowProps { post, comments })
}
```

`Posts/Show` ist eine Svelte-Komponente (oder React, oder Vue - Ihr Starter
wählt aus). TypeScript-Typen für die Props werden automatisch generiert aus
dem `InertiaProps` Derive - führen Sie `suprnova generate-types` aus, nachdem Sie eine
neue Prop-Struktur hinzugefügt haben und das Frontend erhält typisierte Bindings.

Wenn Sie Inertia in Laravel via `inertia()` verwendet haben, ist das das Gleiche - nur typisiert von Ende zu Ende. Siehe die [Frontend-Übersicht](frontend.md).

## Dinge, die eine andere Form annehmen

Ein paar Dinge funktionieren in Suprnova anders. Keines davon ist ein Blocker,
aber es lohnt sich, diese im Voraus zu kennen.

### Keine Service Provider

Laravel hat Dutzende von Service Providern, die Bindings, Observer,
View Composer usw. registrieren. Suprnova hat **eine** Bootstrap-Funktion in Ihrem
App `bootstrap.rs`. Sie registrieren alles dort, in der richtigen Reihenfolge. Es ist nicht
elegant, aber es ist transparent - Sie können in 30 Zeilen genau sehen, was
Ihre App bootet.

```rust
// bootstrap.rs
use std::sync::Arc;

pub async fn register() {
    suprnova::App::bind::<dyn MyService>(Arc::new(MyServiceImpl::new()));
    suprnova::Event::listen::<OrderShipped, _>(Arc::new(SendShipmentNotification)).await;
    crate::observers::register();
}
```

Die Kapitel [Service Container](container.md) und [Application Bootstrap](bootstrap.md)
haben die Details.

### Konfiguration ist typisiert

Wo Laravel `config('app.timezone')` verwendet und was-das-Array-sagt zurückgibt,
hat Suprnova typisierte Config-Strukturen:

```rust
let cfg = suprnova::Config::get::<AppConfig>()?;
let tz = &cfg.timezone;   // &str, nicht gemischt
```

Sie können Ihre eigenen typisierten Config-Abschnitte registrieren. Siehe [Konfiguration](configuration.md).

### Keine Facades-als-Aliases

Laravel Facades wie `DB::` sind Klassen-Aliases konfiguriert in `config/app.php`.
Suprnova Facades sind echte Module an der Crate-Root:

```rust
use suprnova::{Auth, Cache, DB, Event, Gate, Mail, Notify, Queue, Schedule, Storage};
```

Gleiche Oberfläche, kein globales Aliasing erforderlich.

### Compile-Zeiten sind real

Rust-Compile-Zeiten sind nicht PHP. Ein sauberer Build einer frischen Suprnova-App
dauert 1-2 Minuten; inkrementelle Builds während der Entwicklung sind ein paar
Sekunden. Der Dev-Workflow ist derselbe - `suprnova serve` überwacht
Änderungen und recompiliert - aber Sie werden es spüren, wenn Sie zum ersten Mal ein
Makro ändern und eine Downstream-Crate recompilieren. Caching zahlt sich schnell aus.

### Der Borrow Checker existiert

Die meisten Controller und Handler berühren niemals Lebensdauer-Annotationen - die
Framework-Signaturen verstecken sie. Wenn der Borrow Checker Sie anschreit,
ist es normalerweise, weil Sie versucht haben, eine Referenz über ein `.await`
zu halten, das einen Mutex überschritt, oder eine DB-Transaktion über einen Await-Aufruf
gehalten haben, der exklusiven Zugriff brauchte. Die Fehler sind klar und die Fixes sind
normalerweise `.clone()` oder Umstrukturieren-in-kleinere-Scopes.

### Kein `tinker` REPL

Es gibt keinen REPL. Das nächste Äquivalent ist ein einmaliges `cargo run`
Skript in `examples/`, oder ein `#[suprnova_test]` Test, der die
Sache ausübt, die Sie debuggen. Die meisten Dinge, die Sie in Tinker tun würden (ein
Modell anstoßen, eine Benachrichtigung auslösen, einen Job dispatchen) sind ein 5-Zeilen-Test.

## Wo Laravel-Kapitel landen

Schnelle Übersicht, wenn Sie wissen, was Sie suchen, aber nicht wissen, wo es ist:

| Laravel-Thema | Suprnova-Kapitel |
|---|---|
| Lifecycle | [Request-Lifecycle](lifecycle.md) |
| Service Container | [Service Container](container.md) |
| Service Provider | [Application Bootstrap](bootstrap.md) |
| Facades | [Service Container](container.md) |
| Routing | [Routing](routing.md) |
| Middleware | [Middleware](middleware.md) |
| CSRF-Schutz | [CSRF-Schutz](csrf.md) |
| Controller | [Controller](controllers.md) |
| Anfragen | [Anfragen](requests.md) |
| Antworten | [Antworten](responses.md) |
| URL-Generierung | [URL-Generierung](urls.md) |
| Session | [Sitzungen](session.md) |
| Validierung | [Validierung](validation.md) |
| Fehlerbehandlung | [Fehlerbehandlung](errors.md) |
| Protokollierung | [Protokollierung](logging.md) |
| Artisan Console | [Konsole](console.md) + [CLI-Referenz](cli.md) |
| Broadcasting | [Broadcasting](broadcasting.md) |
| Cache | [Cache](cache.md) |
| Ereignisse | [Ereignisse](events.md) |
| Dateispeicher | [Dateisystem & Speicher](filesystem.md) |
| HTTP Client | [HTTP Client](http-client.md) |
| Lokalisierung | [Lokalisierung](localization.md) - Fluent `.ftl` Kataloge, keine PHP Arrays |
| Mail | [Mail](mail.md) |
| Benachrichtigungen | [Benachrichtigungen](notifications.md) |
| Warteschlangen | [Warteschlange](queues.md) |
| Ratenbegrenzung | [Ratenbegrenzung](rate-limiting.md) |
| Task-Planung | [Task-Planung](scheduling.md) |
| Authentifizierung | [Authentifizierung](authentication.md) |
| Autorisierung | [Autorisierung](authorization.md) |
| Email-Verifizierung | [Auth-Flows](auth-flows.md) |
| Passwort zurücksetzen | [Auth-Flows](auth-flows.md) |
| Verschlüsselung | [Verschlüsselung](encryption.md) |
| Hashing | [Hashing](hashing.md) |
| Datenbank | [Datenbank](database.md) |
| Query Builder | [Query Builder](queries.md) |
| Paginierung | [Paginierung](pagination.md) |
| Migrationen | [Migrationen](migrations.md) |
| Seeding | [Seeding](seeding.md) |
| Eloquent | [Eloquent](eloquent.md) |
| Eloquent: Relationships | [Relationships](eloquent-relationships.md) |
| Eloquent: Collections | [Collections](eloquent-collections.md) |
| Eloquent: Mutators / Casts | [Casts, Accessors & Mutators](eloquent-mutators.md) |
| Eloquent: API Resources | [JSON:API resources](eloquent-resources.md) |
| Eloquent: Serialization | [Serialization](eloquent-serialization.md) |
| Eloquent: Factories | [Factories](eloquent-factories.md) |
| Testen | [Testen](testing.md) |
| HTTP Tests | [HTTP Tests](http-tests.md) |
| Database Testing | [Datenbank-Tests](database-testing.md) |
| Mocking | [Mocking und Fakes](mocking.md) |
| Cashier (Stripe) | [Zahlungen - Stripe Adapter](payments-stripe.md) |
| Cashier (Paddle) | [Zahlungen - Paddle Adapter](payments-paddle.md) |
| Sanctum / Passport | Magnetar-Bearer-Sessions über `BearerTokenMiddleware`; keine separate Sanctum- oder Passport-API |
| Horizon | Queue-Inspektion ist in das Framework eingebaut; kein Horizon-Dashboard |
| Telescope / Pulse | (auf v2+ verschoben) |

Dinge, die Laravel hat, Suprnova aber (noch) nicht:

- Telescope-/Pulse-Dashboards. Grundlegende [Beobachtbarkeit](observability.md) wird ausgeliefert.
- Sanctum-/Passport-Paket-APIs. Magnetar-Bearer-Sessions und `BearerTokenMiddleware` stellen Token-Authentifizierung bereit, aber nicht Laravels Oberfläche zur Tokenverwaltung.
- Horizons Dashboard. Queue-Inspektion ist in das Framework eingebaut.
- Blade - absichtlich; Inertia ist die Frontend-Architektur.
- `trans_choice` - [Lokalisierung](localization.md) wird ausgeliefert, aber Plurale werden
  innerhalb der Nachricht nach CLDR-Kategorie ausgewählt, statt nach
  `[1,19]`-Stil Integer-Bereiche, die `trans_choice` akzeptiert

## Nächste Schritte

- [Installation](installation.md) - bekommen Sie ein Projekt am Laufen
- [Schnellstart](quickstart.md) - bauen Sie eine kleine App in 5 Minuten
- [Routing](routing.md) - das natürliche nächste Kapitel von hier

Oder springen Sie überall via [`documentation.md`](documentation.md).
