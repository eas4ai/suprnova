# Makros

Suprnova bringt etwa drei Dutzend Makros mit, jedes davon aus
`suprnova::*` re-exportiert. Sie sind die Gelenke, an denen das
Framework auf Ihren Code trifft - `routes!` baut den Router,
`#[handler]` passt eine Funktion zu einem Handler an,
`#[suprnova::model]` macht aus einer Struktur ein Eloquent-Model,
`#[derive(Data)]` erzeugt einen typisierten Inertia-Payload. Dieses
Kapitel ist der Index. Jedes Makro bekommt eine
Ein-Absatz-Beschreibung, ein minimales Beispiel und einen Verweis auf
das Kapitel, das es für echte Arbeit einsetzt.

Ein paar Prinzipien, die für die gesamte Oberfläche gelten:

- **Makros erzeugen vollständig qualifizierte Pfade.** Generierter
  Code schreibt `::suprnova::…`, sodass die Makros funktionieren, egal
  ob Sie die zugrunde liegenden Typen importiert haben oder nicht.
- **Starker Einsatz von `inventory::submit!`.** Models, Commands,
  Policies, Observer, Payment-Provider und mehr registrieren sich
  selbst zur Compile-Zeit, und das Framework leert die Registry beim
  Boot. Sie verdrahten die Registrierung fast nie von Hand.
- **Compile-Zeit-Validierung, wo sie sich lohnt.** `inertia_response!`
  prüft, dass die benannte Komponentendatei existiert. `redirect!`
  prüft, dass die benannte Route existiert. `routes!` weist Pfade
  zurück, die nicht mit `/` beginnen. Fehler, die sich zur Build-Zeit
  fangen lassen, werden auch gefangen.

## Routing

| Macro | Rückgabe | Was es tut |
|---|---|---|
| `routes!` | `pub fn register() -> Router` | Oberste Liste aller Routen - exportiert ein `register()`, das Ihr `app.rs` aufruft |
| `get!` / `post!` / `put!` / `delete!` / `patch!` / `head!` / `options!` / `any!` | `RouteDefBuilder<H>` | Eine HTTP-Route - verkettbar mit `.name(...)` / `.middleware(...)` |
| `group!` | `GroupDef` | Präfix + Middleware, angewendet auf eine untergeordnete Liste von Routen |
| `fallback!` | `FallbackDefBuilder<H>` | Eigener 404-Handler, wenn keine Route passt |
| `ws!` | `WsRouteDef` | Eine WebSocket-Route - verkettbar mit `.middleware(...)` / `.config(...)` |

```rust
use suprnova::{routes, get, post, ws, group};
use crate::{controllers, middleware::AuthMiddleware, ws::ChatHandler};

routes! {
    get!("/", controllers::home::index).name("home"),
    get!("/users/{id}", controllers::user::show).name("users.show"),
    post!("/users", controllers::user::store).name("users.store"),

    group!("/admin", {
        get!("/dashboard", controllers::admin::dashboard),
    }).middleware(AuthMiddleware),

    ws!("/ws/chat", ChatHandler),
}
```

Der Routen-Pfad-String wird zur Compile-Zeit geprüft -
`validate_route_path` weist alles zurück, was nicht mit `/` beginnt.
Über `.name("…")` registrierte Routennamen werden beim Boot zusätzlich
über `register_route_name` auf Eindeutigkeit geprüft. Siehe
[Routing](routing.md) für die vollständige Expansion und
[WebSockets](websockets.md) für `ws!`.

## Handler und Anfragen

### `#[handler]`

Schreibt eine Controller-Funktion so um, dass sie typisierte Parameter
(über `FromRequest`) direkt aus der eingehenden Anfrage extrahieren
kann - statt Felder von Hand aus `Request` zu ziehen, deklarieren Sie,
was der Handler braucht, und das Makro verdrahtet es.

```rust
use suprnova::{handler, Response, json_response, request};

#[request]
pub struct CreateUserRequest {
    #[validate(email)]
    pub email: String,

    #[validate(length(min = 8))]
    pub password: String,
}

#[handler]
pub async fn store(form: CreateUserRequest) -> Response {
    // `form` ist bereits validiert - bei einem Fehlschlag wird automatisch 422 zurückgegeben
    json_response!({ "email": form.email })
}
```

Ein erster Parameter in der Form von `Request` wird weiterhin als
Identitätsfall akzeptiert. Siehe [Controller](controllers.md).

### `#[request]` und `#[derive(FormRequest)]`

`#[request]` ist der empfohlene Weg, einen validierten Request-Typ zu
deklarieren. Es leitet `Deserialize`, `Validate` und `FormRequest`
automatisch ab, sodass die Struktur sowohl mit `application/json`- als
auch mit `application/x-www-form-urlencoded`-Rümpfen funktioniert.

`#[derive(FormRequestDerive)]` ist das darunterliegende Derive, falls
Sie auf das Attribut verzichten wollen (dann müssen Sie `Deserialize`
und `Validate` selbst ableiten). Empfohlen ist das Attribut; das Derive
gibt es für den Randfall. Siehe [Anfragen](requests.md) und
[Validierung](validation.md).

### `#[derive(MultipartRequest)]`

Stark typisierter Extraktor für `multipart/form-data` - bindet
Textfelder und hochgeladene Dateien in einer Struktur, mit Validatoren
auf Typebene pro Feld.

```rust
use suprnova::{MultipartRequest};
use suprnova::http::upload::{ImageFile, MaxSize, UploadedFile};

#[derive(MultipartRequest)]
pub struct AvatarUpload {
    #[field("avatar")]
    pub avatar: UploadedFile<(ImageFile, MaxSize<5_242_880>)>,

    #[field("caption")]
    pub caption: Option<String>,
}
```

Die eingebauten Validatoren (`ImageFile`, `MimeAllowlist<…>`,
`MaxSize<…>`, `MimeType<…>`) lassen sich über Tupel kombinieren. Siehe
[Anfragen](requests.md).

## Antworten

### `json_response!` und `text_response!`

Die zwei Kurzform-Response-Makros. Beide wickeln `HttpResponse::*` in
`Ok(...)` ein, sodass sie direkt in die Return-Position eines
Handlers passen:

```rust
use suprnova::{handler, json_response, text_response, Response};

#[handler]
pub async fn health() -> Response {
    json_response!({ "status": "ok" })
}

#[handler]
pub async fn robots() -> Response {
    text_response!("User-agent: *\nDisallow:")
}
```

Siehe [Antworten](responses.md).

### `inertia_response!`

Baut eine Inertia-Page-Response und validiert zur Compile-Zeit, dass
die benannte Komponentendatei (`.svelte` / `.tsx` / `.jsx` / `.vue`)
in `frontend/src/pages/` existiert. Vertippen Sie sich beim
Komponentennamen, schlägt der Build mit Vorschlägen fehl:

```rust
use suprnova::{handler, inertia_response, InertiaProps, Request, Response};

#[derive(InertiaProps)]
struct HomeProps {
    title: String,
    user_count: i64,
}

#[handler]
pub async fn index(req: Request) -> Response {
    inertia_response!(&req, "Home", HomeProps {
        title: "Welcome".into(),
        user_count: 42,
    })
}
```

`#[derive(InertiaProps)]` generiert die `Serialize`-Implementierung,
die die Form der Response braucht. Siehe
[Inertia Responses](frontend-inertia-responses.md).

### `redirect!`

Typsicherer Redirect zu einer benannten Route - der Routenname wird
zur Compile-Zeit gegen die über `routes!` registrierten Namen
geprüft:

```rust
use suprnova::redirect;

// Kompiliert nur, wenn "users.show" eine registrierte Route ist
let resp = redirect!("users.show").with("id", "42").into();
```

Siehe [URL-Generierung](urls.md).

## Eloquent

### `#[suprnova::model]`

Macht aus einer einfachen Struktur ein vollständiges Eloquent-Model:
generiert SeaORM-`Entity`-, `Model`-, `ActiveModel`-, `Column`- und
`Relation`-Stubs sowie alle Trait-Implementierungen, die Eloquent
braucht. Registriert außerdem per `inventory::submit!` einen
`ModelEntry`, damit das Framework beim Boot jedes Model aufzählen
kann.

```rust
use suprnova::model;

#[model(table = "users")]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

Zu den Attribut-Keys gehören `table`, `primary_key`, `key_type`,
`auto_increment`, `connection`, `fillable`, `guarded`, `casts`,
`timestamps`, `soft_deletes`, `appends`, `hidden`, `visible`,
`mutators`, `touches` und `unique_id` (für UUID/ULID-Primärschlüssel).
Siehe [Eloquent](eloquent.md).

### `#[suprnova::scopes(Model)]`

Durchläuft einen `impl Model { … }`-Block und macht aus jeder
Methode, deren Signatur zu `fn name(query: Builder<Self>[, args…]) ->
Builder<Self>` passt, einen Scope - generiert dabei sowohl
`Model::scope_name(args)` als auch ein verkettbares
`.scope_name(args)` auf `Builder<Model>`.

```rust
use suprnova::{scopes, Builder};

#[suprnova::scopes(User)]
impl User {
    pub fn active(query: Builder<Self>) -> Builder<Self> {
        query.filter("active", true)
    }

    pub fn popular(query: Builder<Self>, threshold: i64) -> Builder<Self> {
        query.filter_op("followers_count", ">", threshold)
    }

    // Kein Scope - bleibt unverändert durchgereicht
    pub fn display_name(&self) -> String { self.name.clone() }
}

// Beide Aufrufstellen kompilieren:
// User::active().popular(500).get().await?;
// User::query().filter_op("id", ">", 0).active().get().await?;
```

Die verkettbare Form benötigt den generierten Trait
`HasScope_<scope>_<Model>` im Scope, wenn sie aus einem anderen Modul
aufgerufen wird. Siehe [Eloquent](eloquent.md).

### `#[suprnova::observer(Model)]`

Verdrahtet einen `impl Observer<M>`-Block mit dem
Lifecycle-Event-System - jede der 16 überschriebenen Methoden wird
zu einem registrierten Listener, der beim Inventory eingereicht und
beim Boot geleert wird.

```rust
use async_trait::async_trait;
use suprnova::eloquent::observers::Observer;
use suprnova::eloquent::events::EventResult;
use suprnova::eloquent::attrs::Attrs;
use suprnova::FrameworkError;

pub struct AuditObserver;

#[suprnova::observer(User)]
#[async_trait]
impl Observer<User> for AuditObserver {
    async fn creating(&self, attrs: &mut Attrs) -> EventResult {
        if attrs.get("email").is_none() {
            return EventResult::cancel("email is required");
        }
        EventResult::ok()
    }

    async fn created(&self, user: &User) -> Result<(), FrameworkError> {
        tracing::info!(user.id = user.id, "user created");
        Ok(())
    }
}
```

**Erforderliche Attribut-Reihenfolge: `#[suprnova::observer(M)]` muss
vor `#[async_trait]` stehen.** Attribut-Makros expandieren von außen
nach innen - läuft `async_trait` zuerst, schreibt es jede
`async fn` in eine desugarte Form um, und der Namensabgleich des
Observer-Makros gegen die 16 Trait-Methodennamen findet dann
stillschweigend nichts. Siehe [Ereignisse](events.md).

### `#[suprnova::accessor]` und `#[suprnova::mutator]`

Function-Level-Marker auf Methoden in `impl Model { … }`, die sich in
die `to_json()`- / `fill()`-Pfade des Models einhängen. Referenzieren
Sie den Feldnamen in `#[model(appends = […])]` (Accessor) oder
`#[model(mutators = […])]` (Mutator), damit das Makro sie
verdrahtet.

```rust
#[suprnova::model(appends = ["full_name"], mutators = ["password"])]
pub struct User {
    pub id: i64,
    pub first_name: String,
    pub last_name: String,
    pub password: String,
}

impl User {
    #[suprnova::accessor]
    pub fn full_name(&self) -> String {
        format!("{} {}", self.first_name, self.last_name)
    }

    #[suprnova::mutator]
    pub fn set_password(
        &mut self,
        value: serde_json::Value,
    ) -> Result<(), suprnova::FrameworkError> {
        let raw: String = serde_json::from_value(value)
            .map_err(|e| suprnova::FrameworkError::validation("password", format!("{e}")))?;
        self.password = bcrypt(raw);
        Ok(())
    }
}
```

Siehe [Mutators & Casts](eloquent-mutators.md).

### `#[suprnova::prunable]`

Umschließt eine `Prunable`- (oder `MassPrunable`-)Implementierung und
reicht einen `PrunerEntry` in die Registry ein, die `model:prune` zur
Laufzeit durchläuft:

```rust
use async_trait::async_trait;
use chrono::{Duration, Utc};
use suprnova::eloquent::Prunable;

#[suprnova::prunable]
#[async_trait]
impl Prunable for Session {
    fn prunable() -> suprnova::Builder<Self> {
        Self::query().filter_op(
            "expires_at",
            "<",
            (Utc::now() - Duration::days(30)).to_rfc3339(),
        )
    }
}
```

Siehe [Eloquent](eloquent.md).

### `attrs!`

Baut eine geordnete `Attrs`-Map (`IndexMap<&'static str,
serde_json::Value>`) für `Model::create` / `Model::update` /
`Model::fill`:

```rust
use suprnova::attrs;

let user = User::create(attrs! {
    name: "Alice",
    email: "alice@example.com",
    age: 32,
}).await?;
```

Siehe [Eloquent](eloquent.md).

### `casts!`

Baut eine Per-Query-Cast-Map, die Sie an `Builder::with_casts`
übergeben können:

```rust
use suprnova::{casts, AsDate, AsJson};

let map = casts! {
    birthday = AsDate,
    metadata = AsJson<serde_json::Value>,
};
let rows = User::query().with_casts(map).get().await?;
```

Siehe [Mutators & Casts](eloquent-mutators.md).

### `route_binding!`

Implementiert `RouteBinding` für eine handgerollte SeaORM-Entity,
sodass sie automatisch aus einem Routenparameter aufgelöst wird.
Models, die mit `#[suprnova::model]` definiert wurden, registrieren
sich automatisch und brauchen das nicht; greifen Sie zu
`route_binding!`, wenn Sie die Entity von Hand geschrieben haben:

```rust
use suprnova::route_binding;

route_binding!(crate::entities::user::Entity, User, "user");
```

Danach übergibt `get!("/users/{user}", controllers::user::show)`
Ihrem Handler ein vollständig geladenes `User`. Siehe
[Routing](routing.md).

## Daten und Inertia

### `#[derive(Data)]`

Das zusammengesetzte Derive für typisierte Payloads. Erzeugt eine
`Serialize`-Implementierung, die `#[data(input_only)]`-Felder
respektiert, plus eine `Deserialize`-Implementierung, die Payloads
zurückweist, die versuchen, `#[data(output_only)]`-Felder zu setzen.
Kombinieren Sie es mit `#[json_resource("type")]` für JSON:API-Output
über das `Resource`-Kapitel.

```rust
use suprnova::{Data, Validate};

#[derive(Data, Validate)]
struct UserDto {
    pub id: i64,
    pub name: String,

    #[data(input_only)]
    #[validate(length(min = 8))]
    pub password: String,

    #[data(output_only)]
    pub computed_handle: String,

    #[data(allow_include)]
    pub posts: Vec<PostDto>,
}
```

`#[data(allow_include)]` registriert das Feld über
`inventory::submit!` in der Partial-Reload-Include-Allowlist. Siehe
[Datenobjekte](data.md) und [JSON:API resources](eloquent-resources.md).

### `#[derive(InertiaProps)]`

Generiert die `Serialize`-Implementierung, die `inertia_response!`
braucht. Reines Marker-Derive - die meisten Apps greifen stattdessen
zu `#[derive(Data)]`, weil es Partial-Reload-Includes kostenlos
mitliefert.

```rust
use suprnova::InertiaProps;

#[derive(InertiaProps)]
struct DashboardProps {
    title: String,
    user: User,
}
```

Siehe [Inertia Responses](frontend-inertia-responses.md).

### `when_loaded!`

Gibt ein `Prop::lazy(…)` nur aus, wenn eine benannte Relation auf der Entity
eager geladen wurde; andernfalls wird `Prop::absent()` ausgegeben, sodass die
Prop vollständig aus der Response entfällt:

```rust
use suprnova::when_loaded;

let songs_prop = when_loaded!(&artist, "songs", || async {
    serde_json::to_value(&artist.songs).unwrap()
});
```

Siehe [Datenobjekte](data.md).

## Dependency Injection

### `#[service]`

Fügt einem Trait `Send + Sync + 'static` hinzu, damit er in den
Container passt:

```rust
use suprnova::service;

#[service]
pub trait HttpClient {
    async fn get(&self, url: &str) -> Result<String, FrameworkError>;
}

// App::bind::<dyn HttpClient>(Arc::new(RealHttpClient::new()));
// let client = App::make::<dyn HttpClient>()?;
```

Siehe [Service Container](container.md).

### `#[injectable]`

Registriert einen konkreten Typ automatisch als Singleton. Leitet
`Default` + `Clone` ab und reicht eine Registrierung ein, die beim
Boot läuft:

```rust
use suprnova::injectable;

#[injectable]
pub struct AppState {
    pub counter: u32,
}

// let state: AppState = App::get().unwrap();
```

Siehe [Service Container](container.md).

## Fehler

### `#[domain_error]`

Definiert einen Domain-Fehler, der `Display`, `Error`, `HttpError`
und `From<T> for FrameworkError` implementiert - sodass er einen
Handler über `?` sofort abbricht:

```rust
use suprnova::domain_error;

#[domain_error(status = 404, message = "User not found")]
pub struct UserNotFoundError {
    pub user_id: i32,
}

pub async fn get_user(id: i32) -> Result<User, FrameworkError> {
    let user = User::find(id).await?
        .ok_or_else(|| UserNotFoundError { user_id: id })?;
    Ok(user)
}
```

Siehe [Fehlerbehandlung](errors.md).

## Konsole und Hintergrundarbeit

### `#[command]`

Markiert eine `async fn(Vec<String>) -> Result<(), FrameworkError>`
als Console-Command. Reicht einen `CommandEntry` ein, damit
`dispatch_argv` sie findet, wenn die projektspezifische
Console-Binary läuft:

```rust
use suprnova::{command, FrameworkError};

#[command(name = "db:seed", description = "Run all registered seeders")]
async fn db_seed(_args: Vec<String>) -> Result<(), FrameworkError> {
    suprnova::seed::run_all().await
}
```

Siehe [Konsole](console.md).

### `#[derive(Command)]`

Die Alternative mit typisierten Argumenten. Setzt auf
`#[derive(clap::Parser)]` auf, liest `#[console(...)]` für Metadaten
und generiert den Runner, der Ihr `TypedCommand::run` aufruft:

```rust
use async_trait::async_trait;
use suprnova::{Command, FrameworkError, TypedCommand};

#[derive(clap::Parser, Command)]
#[console(name = "greet", description = "Greet someone")]
pub struct Greet {
    #[arg(short, long)]
    name: Option<String>,
    #[arg(long)]
    loud: bool,
}

#[async_trait]
impl TypedCommand for Greet {
    async fn run(self) -> Result<(), FrameworkError> {
        let target = self.name.unwrap_or_else(|| "world".into());
        println!("{}", if self.loud { format!("HELLO {target}!") } else { format!("Hello {target}") });
        Ok(())
    }
}
```

Siehe [Konsole](console.md).

### `#[workflow]` und `#[workflow_step]`

`#[workflow]` registriert eine async fn als dauerhaften Workflow -
lauffähiger Zustand, wiederholbare Schritte, persistierte Historie.
Jedes `#[workflow_step]` im Funktionskörper ist ein Checkpoint, von
dem die Runtime nach einem Crash oder Neustart fortsetzen kann.

```rust
use suprnova::{workflow, workflow_step, FrameworkError};

#[workflow]
async fn onboard_user(user_id: i64) -> Result<(), FrameworkError> {
    send_welcome_email(user_id).await?;
    enable_default_features(user_id).await?;
    Ok(())
}

#[workflow_step]
async fn send_welcome_email(user_id: i64) -> Result<(), FrameworkError> {
    // …
    Ok(())
}
```

### `start_workflow!`

Startet einen Workflow anhand seines Pfads und serialisiert die
Argumente in die Envelope-Form der Workflow-Runtime:

```rust
use suprnova::start_workflow;

let handle = start_workflow!(crate::workflows::onboard_user, 42).await?;
```

Siehe [Workflows](workflows.md).

### `schedule_task!`

Syntaktischer Zucker um `TaskBuilder::from_async`, damit sich eine
Closure sauber neben Trait-basierten `Task`-Implementierungen planen
lässt:

```rust
use suprnova::{schedule_task, FrameworkError};

let task = schedule_task!(|| async {
    println!("ticking");
    Ok::<(), FrameworkError>(())
})
    .every_minute()
    .name("tick");
```

Siehe [Task-Planung](scheduling.md).

## Autorisierung

### `#[policy(UserType, ResourceType)]`

Umschließt einen `impl Policy`-Block und registriert jede Methode als
benannte Gate-Action. Der Gate-Name kombiniert den Methodennamen mit
dem kleingeschriebenen Resource-Typ - `fn view(...)` auf `Comment`
wird zu `"view-comment"`:

```rust
use suprnova::policy;

struct CommentPolicy;

#[policy(User, Comment)]
impl CommentPolicy {
    fn view(_user: &User, _comment: &Comment) -> bool { true }
    fn update(user: &User, comment: &Comment) -> bool {
        comment.author_id == user.id
    }
}
```

`Server::run` ruft `authorization::init_policies()` automatisch auf.
Siehe [Autorisierung](authorization.md).

## Benachrichtigungen und Mail

### `#[derive(NotificationMailable)]`

Generiert `to_mail` automatisch aus einem `#[mail(...)]`-Attribut -
Inline- oder dateibasierte Tera-Templates für Betreff, HTML-Body und
Text-Body. Compile-Zeit-Prüfungen: Betreff erforderlich, mindestens
ein Body vorhanden, html/html_template schließen sich gegenseitig
aus, `from_name` erfordert `from`:

```rust
use serde::{Serialize, Deserialize};
use suprnova::NotificationMailable;

#[derive(Serialize, Deserialize, NotificationMailable)]
#[mail(
    subject = "Your order shipped - tracking {{ tracking }}",
    html    = "<p>Tracking: <code>{{ tracking }}</code></p>",
    text    = "Tracking: {{ tracking }}",
    from    = "orders@suprnova.dev",
)]
pub struct OrderShipped { pub tracking: String }
```

Der Notification-Trait selbst wird von Hand implementiert - es gibt
kein `#[derive(Notification)]`. Siehe
[Benachrichtigungen](notifications.md) und [Mail](mail.md).

## Validierung

### `validate!`

Synchroner, deklarativer Einstiegspunkt für Validierung. Jede Zeile
paart einen Feldnamen mit einem oder mehreren `Rule`- (oder
`ContextualRule`-)Werten, mit `?:` für „nur validieren, wenn
vorhanden“ und `?=>` für bedingt erforderliche optionale Felder:

```rust
use suprnova::{validate, ValidationErrors};
use suprnova::validation::rules::*;

fn validate_form(self_ref: &SignupForm) -> Result<(), ValidationErrors> {
    validate! { self_ref =>
        email   => Required, Email;
        password => Required, Min(8);
        bio     ?: Max(500);
        card_number ?=> RequiredIf { other: "billing_type", value: "card" } => with ctx;
    }
}
```

`Validate` wird aus der `validator`-Crate re-exportiert -
`#[validate(...)]`-Attribute (z. B. `#[validate(email)]`) stammen aus
`validator` und laufen über den synchronen Pfad von `FormRequest`.
Verwenden Sie `validate!`, wenn Sie kontextuelle Regeln,
feldübergreifende Regeln, asynchrone Regeln oder Regeln aus der
`suprnova::validation::rules`-Palette brauchen. Siehe
[Validierung](validation.md).

## Factories

### `#[derive(Factory)]`

Generiert einen begleitenden `<Model>Factory`-Marker sowie eine
`Factory`-Implementierung, die Models über `fake::Faker` erzeugt. Das
Model muss `fake::Dummy<fake::Faker>` implementieren -
typischerweise über `#[derive(Dummy)]`:

```rust
use suprnova::{Dummy, Factory};

#[derive(Dummy, Factory)]
pub struct User {
    pub id: i32,
    pub name: String,
    pub email: String,
}

// UserFactory existiert:
let users = UserFactory::new().count(10).make_many();
```

Siehe [Factories](eloquent-factories.md).

## Testen

### `#[suprnova_test]`

Umschließt einen `async fn`-Test mit einer In-Memory-SQLite-Datenbank
(standardmäßig mit `crate::migrations::Migrator`), ruft
`App::init()` und `App::boot_services()` auf und führt den Testkörper
unter `#[tokio::test]` aus. Parallele Tests bleiben durch die
Per-Thread-Ebene des Containers hermetisch - binden Sie
test-spezifische Services über `TestContainer::fake` (nicht
`App::bind`), damit jeder Thread seine eigenen Fakes sieht:

```rust
use suprnova::suprnova_test;
use suprnova::testing::TestDatabase;

#[suprnova_test]
async fn creates_a_user(db: TestDatabase) {
    let user = User::create(attrs! { name: "A", email: "a@x.com" }).await.unwrap();
    assert!(user.id > 0);
}
```

Ein eigener Migrator läuft über
`#[suprnova_test(migrator = MyMigrator)]`. Siehe [Testen](testing.md).

### `test_database!`

Der Einzeiler-Konstruktor für `TestDatabase`, für Tests, die den
`db`-Parameter nicht über `#[suprnova_test]` bekommen:

```rust
let db = test_database!();
let db = test_database!(my_crate::CustomMigrator);
```

### `describe!`, `test!`, `expect!`

Gruppierung im Jest-Stil + fluent Assertions. `describe!` ist ein
Modul, `test!` erzeugt einen `#[test]` (synchron oder asynchron, mit
oder ohne `TestDatabase`-Parameter), und `expect!` umschließt einen
Wert für verkettete Assertions mit Datei-/Zeilen-Kontext bei einem
Fehlschlag:

```rust
use suprnova::{describe, test, expect};

describe!("CreateUserAction", {
    test!("creates a user", async fn(db: TestDatabase) {
        let user = CreateUserAction::new()
            .execute("test@example.com").await.unwrap();
        expect!(user.email).to_equal("test@example.com".to_string());
    });
});
```

Siehe [Testen](testing.md).

## Middleware

### `global_middleware!`

Registriert eine Middleware, die bei jeder Anfrage läuft, in
Registrierungsreihenfolge, vor jeder routenspezifischen Middleware.
Pro Typ idempotent:

```rust
use suprnova::global_middleware;
use crate::middleware;

pub fn register() {
    global_middleware!(middleware::LoggingMiddleware);
    global_middleware!(middleware::CorsMiddleware);
}
```

Muss vor `Server::from_config` / `Server::new` laufen - der Server
erstellt beim Bauen einen Snapshot der globalen Registry. Siehe
[Middleware](middleware.md).

## Fallstricke

Eine kurze Liste von Fehlermustern, die leicht passieren und leicht
zu beheben sind.

### Attribut-Reihenfolge - `#[observer]` muss vor `#[async_trait]` stehen

```rust
// KORREKT
#[suprnova::observer(User)]
#[async_trait]
impl Observer<User> for AuditObserver { … }

// FALSCH - erzeugt stillschweigend null Listener
#[async_trait]
#[suprnova::observer(User)]
impl Observer<User> for AuditObserver { … }
```

Attribut-Makros expandieren von außen nach innen. `async_trait`
schreibt jede `async fn` in eine desugarte `Pin<Box<dyn
Future>>`-Form um. Läuft es zuerst, kann das Observer-Makro nicht
mehr per Methodenname matchen und erzeugt nichts. Dieselbe
Außen-nach-innen-Regel gilt, wann immer Sie mehrere Makros stapeln -
setzen Sie im Zweifel das Suprnova-Attribut ganz außen.

### Die Inherent-Impl-Falle

Eine inherente `impl`-Methode kann eine Standardmethode eines Traits
über Trait-Dispatch **nicht** überschatten. Wenn Sie ein Makro (oder
handgeschriebenen Code) schreiben, das `fn save(&self)` auf einem
Model als inherente Methode definiert, wählen Aufrufe, die über den
`Model`-Trait laufen (`some_model.save()`, wenn die Aufrufstelle es
nur als `&dyn Model` kennt), den Standard des Traits - nicht Ihre
inherente Überschreibung.

Lösung: Erzeugen Sie eine Trait-Methoden-Überschreibung, niemals eine
inherente Methode, wenn das generierte Verhalten am Trait-Dispatch
teilnehmen muss. Deshalb schreiben die Makros des Frameworks
(insbesondere `#[suprnova::model]`) in die Trait-Implementierung.
Wenn Sie Eloquent-Erweiterungen von Hand rollen, tun Sie dasselbe.

### `global_middleware!` wirkt nur vor `Server::from_config`

Der Server erstellt beim Bauen einen Snapshot der globalen Registry.
Ein Aufruf von `global_middleware!(M)` nach
`Server::from_config(...)` wirkt sich nicht mehr rückwirkend auf
diesen Server aus. Registrieren Sie jede globale Middleware in
`bootstrap()`, bevor `Application::run()` den Serve-Schritt
erreicht.

### `redirect!` und `inertia_response!` sind Build-Zeit-Prüfungen

Beide Makros weigern sich zu kompilieren, wenn das benannte Ziel
nicht existiert - genau das ist der Sinn. Entfernt ein Refactoring
eine Route oder einen Komponentennamen, bricht jede Aufrufstelle, die
ihn erwähnt, den Build - genau das wollen Sie. Überrascht Sie der
Build-Fehler, suchen Sie zuerst nach dem String-Literal in Ihrem
`routes!`-Block / Pages-Verzeichnis, bevor Sie den Makro-Aufruf
„reparieren“.

### `?:` überspringt bei `None`; `?=>` läuft auch bei `None`

In `validate!`-Zeilen führt `?:` Regeln nur aus, wenn das Feld `Some`
ist. Eine anwesenheitsabhängige Regel wie `RequiredIf` auf einer
`?:`-Zeile kann daher niemals bei einem fehlenden Feld fehlschlagen.
Verwenden Sie `?=>` (das Abwesenheit als `""` behandelt) für den
Fall „erforderlich, wenn X“.

### `#[derive(Validate)]` stammt aus der `validator`-Crate, nicht von Suprnova

Suprnova re-exportiert `validator::Validate`, damit Sie keine
direkte Abhängigkeit auf `validator` brauchen. Die
`#[validate(...)]`-Attribute stammen aus `validator`. Suprnovas
eigenes `validate!`-Makro ist der Laufzeit-Einstiegspunkt für
feldübergreifende / kontextuelle Regeln; beide ergänzen sich, leben
aber in unterschiedlichen Namespaces.

## Warum Suprnova abweicht

Laravel entdeckt Routen, Commands, Mail-Templates, Model-Klassen,
Factories, Observer und Policies zur Laufzeit - über Reflection,
Dateisystem-Scans und String-basiertes Dispatch. PHP macht das billig
(Autoloading + Opcache amortisieren die Kosten), und die
Entwicklererfahrung ist exzellent: Legen Sie eine Datei ins richtige
Verzeichnis, und sie taucht auf.

Dieses Modell passt nicht zu Rust. Wir haben keine
Laufzeit-Reflection auf Trait-Implementierungen, die Runtime ist eine
einzige statisch gelinkte Binary, und Dateisystem-Scans beim Boot
passen schlechter zu einem Prozessmodell, in dem eine Binary
Millionen Anfragen bedient.

Suprnova erledigt daher dieselbe Aufgabe zur Compile-Zeit. Routen
werden validiert, Komponentennamen werden gegen das
Pages-Verzeichnis geprüft, Mail-Templates werden über `include_str!`
eingebettet, Routennamen werden über das Inventory auf Eindeutigkeit
geprüft, Models registrieren sich selbst in einem Inventory, das das
Framework beim Boot leert, Commands ebenso. Die Entwicklererfahrung
ist ähnlich - legen Sie eine Datei an, fügen Sie ein `#[command]`
oder `#[suprnova::model]` hinzu, starten Sie die Binary - aber die
Verdrahtung geschieht vor `main`, statt bei der ersten Anfrage.

Der Kompromiss ist, dass Vertipper, fehlende Komponenten und kaputte
Referenzen Build-Fehler statt Laufzeitfehler sind, und dass keine
Reflection-Kosten pro Anfrage anfallen.

## Nächste Schritte

- [Routing](routing.md) - vollständige `routes!`-Expansion, Naming,
  Model-Binding
- [Controller](controllers.md) - `#[handler]` und `#[request]`
  zusammen
- [Eloquent](eloquent.md) - `#[suprnova::model]` und Verwandte im
  Kontext
- [Validierung](validation.md) - `validate!`, kontextuelle Regeln,
  asynchrone Regeln
- [Konsole](console.md) - `#[command]` und `#[derive(Command)]` von
  Anfang bis Ende
- [Testen](testing.md) - `#[suprnova_test]`, `expect!`, Fakes
