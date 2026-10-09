# Routing

Routing ist der Mechanismus, mit dem Suprnova eine eingehende HTTP-Anfrage in
einen Handler-Aufruf verwandelt. Sie deklarieren Ihre Routen in `src/routes.rs`
mit dem `routes!`-Makro (oder bauen einen `Router` von Hand), und
`Server::from_config` nimmt diesen Router entgegen und führt ihn für die
Lebensdauer des Prozesses aus. Dieselbe Form wie Laravels `routes/web.php`,
nur mit Rust-Typen statt Facades.

```rust
// src/routes.rs
use suprnova::{routes, get, post, put, delete};
use crate::controllers;

routes! {
    get!("/", controllers::home::index).name("home"),
    get!("/users", controllers::users::index).name("users.index"),
    get!("/users/{id}", controllers::users::show).name("users.show"),
    post!("/users", controllers::users::store).name("users.store"),
    put!("/users/{id}", controllers::users::update).name("users.update"),
    delete!("/users/{id}", controllers::users::destroy).name("users.destroy"),
}
```

Das Makro expandiert zu `pub fn register() -> Router { ... }`. Rufen Sie es
aus Ihrem Bootstrap auf und übergeben Sie das Ergebnis an den Server.

## HTTP-Verben

Ein Makro pro Verb. Alle sieben nehmen ein Pfad-dann-Handler-Paar entgegen und
liefern einen Builder, an den Sie `.name(...)` und `.middleware(...)`
anhängen können.

| Makro | Methode | Verwendung |
|---|---|---|
| `get!`     | GET     | Lese-Endpunkte, statische Seiten |
| `post!`    | POST    | Ressourcen erstellen |
| `put!`     | PUT     | Vollständige Ersetzungs-Updates |
| `patch!`   | PATCH   | Partielle Updates (RFC 5789) |
| `delete!`  | DELETE  | Löschen |
| `head!`    | HEAD    | Reine Header-Abfragen (HEAD greift gemäß RFC 9110 § 9.3.2 auf die GET-Registrierung zurück, wenn es nicht explizit registriert ist) |
| `options!` | OPTIONS | Capability-Discovery, `Accept-Patch`. CORS-Preflight wird von `CorsMiddleware` vor dem Router beantwortet, daher brauchen Sie diese in der Regel nicht |

```rust
use suprnova::{routes, get, post, patch, delete};

routes! {
    get!("/articles", controllers::articles::index),
    post!("/articles", controllers::articles::store),
    patch!("/articles/{id}", controllers::articles::update),
    delete!("/articles/{id}", controllers::articles::destroy),
}
```

Jedes Verb-Makro prüft zur Compile-Zeit, dass der Pfad mit `/` beginnt -
ein fehlender führender Schrägstrich lässt den Build fehlschlagen, nicht
die Anfrage.

### Mehrere Methoden und `any!`

`any!` registriert einen Handler für alle sieben gängigen Verben.
Verwenden Sie es für Webhook-Empfänger und andere Endpunkte, die
akzeptieren müssen, was auch immer HTTP sendet.

```rust
use suprnova::{routes, any};

routes! {
    any!("/webhooks/inbound", controllers::webhooks::inbound)
        .name("webhooks.inbound")
        .middleware(SignatureCheck),
}
```

Wenn Sie nur eine Teilmenge der Verben mit einem gemeinsamen Handler
abdecken möchten, greifen Sie zur Builder-API und zu `Router::methods`:

```rust
use suprnova::Router;
use hyper::Method;

let router = Router::new()
    .methods(&[Method::PUT, Method::PATCH], "/posts/{id}", update_post)
    .name("posts.update")
    .middleware(AuthMiddleware);
```

`.name(...)` und `.middleware(...)` wirken auf jedes Verb, für das die
Route registriert wurde, sodass die Rückwärtssuche dieselbe URL liefert,
unabhängig davon, nach welcher Methode der Aufrufer sucht.

### WebSocket-Routen

`ws!` registriert einen langlebigen Upgrade-Handler. Das Makro ist Teil
desselben `routes!`-Bodys - ausführlich behandelt in
[WebSockets](websockets.md).

## Routenparameter

Dynamische Segmente verwenden geschweifte Klammern (`{id}`). Aus Gründen
der Vertrautheit akzeptiert Suprnova auch Doppelpunkte im
Express/Rails-Stil (`:id`) und normalisiert sie zu geschweiften Klammern,
bevor das Pattern an `matchit` übergeben wird.

```rust
routes! {
    get!("/users/{id}", controllers::users::show),       // matchit-nativ
    get!("/users/:id", controllers::users::show),        // Express/Rails - dasselbe
    get!("/posts/{post_id}/comments/{comment_id}", controllers::comments::show),
}
```

Der Doppelpunkt wird nur am Anfang eines Pfadsegments als
Parameter-Öffner behandelt, sodass wörtliche Doppelpunkte mitten im
Segment unverändert erhalten bleiben (`/files/note:draft` bleibt eine
wörtliche Route, nicht `/files/{draft}`).

Lesen Sie Parameter innerhalb eines Handlers von der Anfrage aus:

```rust
use suprnova::{Request, Response, HttpResponse};

pub async fn show(req: Request) -> Response {
    let user_id = req.param("id").unwrap_or("0");
    Ok(HttpResponse::text(format!("User ID: {}", user_id)))
}
```

Für typisierte Extraktion ohne das `unwrap_or`-Hin-und-Her siehe
Route-Model-Binding weiter unten oder `#[handler]` in
[Controller](controllers.md).

## Route-Model-Binding

Wenn ein Handler-Parameter ein SeaORM-`*::Model`-Typ ist, extrahiert
`#[handler]` den passenden Pfadparameter, parst ihn als den
Primärschlüssel-Typ und holt die Zeile aus der Datenbank. Eine fehlende
Zeile liefert 404; ein Parameter, den der PK-Typ nicht parsen kann,
liefert 400.

```rust
use suprnova::{handler, json_response, Response};
use crate::models::users;

// Route: GET /users/{user}
#[handler]
pub async fn show(user: users::Model) -> Response {
    json_response!({ "name": user.name, "email": user.email })
}
```

Der Parametername (`user`) ist das, wonach `#[handler]` in den Params
der gematchten Route sucht - der Platzhalter muss also übereinstimmen
(`/users/{user}`, nicht `/users/{id}`).

Mehrere Models in einer Signatur funktionieren genauso; mischen Sie sie
mit Form-Requests, Primitiven oder `Request`:

```rust
// Route: PUT /posts/{post}/comments/{comment}
#[handler]
pub async fn update(
    post: posts::Model,
    comment: comments::Model,
    form: UpdateCommentRequest,
) -> Response {
    // post und comment sind bereits geladen; form ist validiert.
    json_response!({ "post_id": post.id, "comment_id": comment.id })
}
```

### Anforderungen

Binding erfolgt automatisch für jedes SeaORM-Model, dessen `Entity`
`suprnova::database::EntityExt` implementiert und dessen
Primärschlüssel-Typ `FromStr` implementiert. Die blanket-freundlichen
Zusatz-Traits von `EntityExt` geben Ihnen `Entity::find_by_pk(id)`,
`::all()`, `::first()` und Ähnliches; Route-Model-Binding ist einfach
`find_by_pk`, angetrieben durch den Pfadparameter.

```rust
// src/models/users.rs (das klassische SeaORM-Layout)
pub use super::entities::users::*;
use sea_orm::entity::prelude::*;

impl ActiveModelBehavior for ActiveModel {}

// Aktiviert Route-Model-Binding (und die Laravel-förmige Lese-Oberfläche).
impl suprnova::database::EntityExt for Entity {}
impl suprnova::database::EntityExtMut for Entity {}
```

Wenn Ihr Model mit dem `#[suprnova::model]`-Makro deklariert ist (die
Eloquent-Oberfläche in [Eloquent](eloquent.md)), greifen Sie direkt
darauf zu: `User::find_by_pk(id).await?`. Route-Model-Binding über
`#[handler]` erwartet weiterhin die `*::Model`-Form - übergeben Sie den
SeaORM-Model-Typ, nicht die Wrapper-Struktur.

### Binding ist Identität, keine Autorisierung

Route-Model-Binding beantwortet "existiert diese Zeile?" - es beantwortet
**nicht** "darf der aktuelle Benutzer diese Zeile sehen?". Ein bloß
gebundener Handler lässt jeden authentifizierten Benutzer jeden Post
sehen, indem er `/posts/N` errät. Autorisieren Sie gegen das gebundene
Model mit `Gate::authorize` oder dem `#[policy]`-Makro - siehe
[Autorisierung](authorization.md).

### Opt-out

Verwenden Sie nicht den `*::Model`-Parametertyp. Extrahieren Sie die ID
und fragen Sie manuell ab:

```rust
use suprnova::{handler, json_response, Response, FrameworkError};
use crate::models::users;
use suprnova::database::EntityExt;

#[handler]
pub async fn show(id: i32) -> Response {
    let user = users::Entity::find_by_pk(id)
        .await?
        .ok_or(FrameworkError::not_found("User"))?;
    json_response!({ "id": user.id, "name": user.name })
}
```

## Benannte Routen

Namen geben Ihnen stabile Bezeichner für die URL-Generierung. Hängen Sie
einen mit `.name(...)` an:

```rust
routes! {
    get!("/", controllers::home::index).name("home"),
    get!("/users", controllers::users::index).name("users.index"),
    get!("/users/{id}", controllers::users::show).name("users.show"),
    post!("/users", controllers::users::store).name("users.store"),
}
```

Namen folgen der Laravel-Konvention `<resource>.<action>` -
`users.show`, `posts.destroy`, `admin.dashboard`. Schlagen Sie sie mit
der Top-Level-Hilfsfunktion `route(name, &[...])` nach:

```rust
use suprnova::route;

let home = route("home", &[]);
//   Some("/")

let profile = route("users.show", &[("id", "123")]);
//   Some("/users/123")
```

`route` liefert `Option<String>` und prozent-kodiert Parameterwerte in
eine pfadsichere Form (aus `("slug", "a/b")` wird also `/posts/a%2Fb` -
matchit-sicher und round-trip-fähig über `req.param("slug")`). Für
Redirect-Ziele und E-Mail-Links verwenden Sie das strikte Geschwister
`suprnova::routing::try_route`, das `Result<String, RouteUrlError>`
liefert und sich weigert, eine URL mit einem ungefüllten
`{placeholder}`-Segment auszugeben. Siehe [URL-Generierung](urls.md)
für die vollständige URL-Oberfläche (signierte URLs, absolute URLs,
`Redirect::route`).

Routennamen sind global eindeutig und prozessweit. Denselben Namen für
zwei verschiedene Pfade zu registrieren löst beim Boot einen Panic aus -
stilles Shadowing war ein sicherheitsrelevanter Bug, weil Redirects
dorthin geroutet hätten, welche Registrierung auch immer gewonnen hätte.
Verwenden Sie `RouteBuilder::try_name` (oder
`suprnova::routing::try_register_route_name`) für die fehlbare Variante.

## Middleware pro Route

Verketten Sie `.middleware(M)` an einen beliebigen Route-Builder:

```rust
use suprnova::{routes, get, post};
use crate::middleware::{AuthMiddleware, AdminMiddleware};

routes! {
    // Öffentlich
    get!("/", controllers::home::index).name("home"),

    // Geschützt
    get!("/dashboard", controllers::dashboard::index)
        .name("dashboard")
        .middleware(AuthMiddleware),

    // Mehrere Middleware komponieren sich von links nach rechts (die äußerste zuerst)
    get!("/admin", controllers::admin::index)
        .middleware(AuthMiddleware)
        .middleware(AdminMiddleware),
}
```

Routen-lokale Middleware läuft nach jeder globalen Middleware
(`Server::with_middleware`) und jeder Gruppen-Middleware, die die Route
umschließt. Die Middleware-Map ist nach `(method, path)` indiziert,
sodass das Anhängen von Auth an `POST /api/posts` niemals auf ein
öffentliches `GET /api/posts` auf demselben Pfad überschwappt. Für den
Middleware-Vertrag und das Schreiben eigener Middleware siehe
[Middleware](middleware.md).

## Routengruppen

`group!` faktorisiert ein gemeinsames Pfadpräfix und/oder gemeinsame
Middleware heraus:

```rust
use suprnova::{routes, get, post, group};
use crate::middleware::{AuthMiddleware, ApiMiddleware};

routes! {
    get!("/", controllers::home::index).name("home"),

    // Gemeinsames /api-Präfix + Middleware
    group!("/api", {
        get!("/users", controllers::api::users::index).name("api.users.index"),
        post!("/users", controllers::api::users::store).name("api.users.store"),
        get!("/users/{id}", controllers::api::users::show).name("api.users.show"),
    }).middleware(ApiMiddleware),

    // Admin-Bereich
    group!("/admin", {
        get!("/dashboard", controllers::admin::dashboard).name("admin.dashboard"),
        get!("/settings", controllers::admin::settings).name("admin.settings"),
    }).middleware(AuthMiddleware),
}
```

Ein Gruppenpräfix wird mit jedem Routenpfad verkettet. Eine Route auf
`/` innerhalb einer Gruppe löst sich exakt zum Gruppenpräfix auf
(`group!("/users", { get!("/", index) })` → `GET /users`).

### Verschachtelte Gruppen

Gruppen lassen sich beliebig tief verschachteln. Präfixe verketten
sich; Middleware vererbt sich von Eltern- zu Kind-Gruppe:

```rust
routes! {
    group!("/api", {
        get!("/health", controllers::api::health),

        group!("/v1", {
            get!("/users", controllers::api::v1::users),

            group!("/admin", {
                get!("/stats", controllers::admin::stats),
            }).middleware(AdminMiddleware),
        }),
    }).middleware(AuthMiddleware),
}
```

| Route | Effektiver Pfad | Middleware-Chain |
|---|---|---|
| `/api/health` | `/api/health` | `AuthMiddleware` |
| `/api/v1/users` | `/api/v1/users` | `AuthMiddleware` |
| `/api/v1/admin/stats` | `/api/v1/admin/stats` | `AuthMiddleware` → `AdminMiddleware` |

Für eine einzelne Route innerhalb einer verschachtelten Gruppe ist die
Ausführungsreihenfolge **äußerste Middleware zuerst**: Eltern-Gruppe →
Kind-Gruppe → routenlokal. Pro-Route-`.middleware(...)` läuft am
innersten.

## Fallback-Route

`fallback!` registriert einen Handler, der läuft, wenn keine andere
Route matcht. Verwenden Sie ihn für eigene 404-Seiten.

```rust
use suprnova::{routes, get, fallback};

routes! {
    get!("/", controllers::home::index),

    fallback!(controllers::errors::not_found),
}
```

```rust
// src/controllers/errors.rs
use suprnova::{Request, Response, HttpResponse};

pub async fn not_found(req: Request) -> Response {
    Ok(HttpResponse::text(format!("Page not found: {}", req.path()))
        .status(404))
}
```

Fallback unterstützt seine eigene Middleware-Chain
(`fallback!(handler).middleware(M)`). Ist kein Fallback registriert, gibt
das Framework ein reines Text-`404 Not Found` zurück.

## Resource-Routing

Für eine Standard-7-Aktionen-REST-Oberfläche implementieren Sie
`ResourceController` und registrieren die Resource über den
`Router`-Builder. Laravel-Parität zu `Route::resource()` und
`Route::apiResource()`.

```rust
use suprnova::{Router, ResourceController, ResourceAction, Request, Response, HttpResponse};
use std::pin::Pin;
use std::future::Future;

struct PostsCtl;

impl ResourceController for PostsCtl {
    fn index(&self, _req: Request) -> Pin<Box<dyn Future<Output = Response> + Send>> {
        Box::pin(async { Ok(HttpResponse::text("list")) })
    }
    fn show(&self, _req: Request) -> Pin<Box<dyn Future<Output = Response> + Send>> {
        Box::pin(async { Ok(HttpResponse::text("one")) })
    }
    // store / update / destroy / create / edit fallen standardmäßig auf 404 zurück.
}

let router: Router = Router::new()
    .resource("posts", PostsCtl)
    .into();
```

Methoden, die Sie nicht überschreiben, liefern 404. Verwenden Sie
`api_resource`, um `create` und `edit` wegzulassen - die beiden Routen,
die nur zum Rendern von Formularen existieren.

### Standardrouten und -namen

| Verb | Pfad | Trait-Methode | Name |
|---|---|---|---|
| GET    | `/posts`             | `index`   | `posts.index`   |
| GET    | `/posts/create`      | `create`  | `posts.create`  |
| POST   | `/posts`             | `store`   | `posts.store`   |
| GET    | `/posts/{post}`      | `show`    | `posts.show`    |
| GET    | `/posts/{post}/edit` | `edit`    | `posts.edit`    |
| PUT    | `/posts/{post}`      | `update`  | `posts.update`  |
| DELETE | `/posts/{post}`      | `destroy` | `posts.destroy` |

Der Pfadparameter verwendet standardmäßig den Singular des
Resource-Namens - `posts` → `{post}`, `categories` → `{category}`.
Unregelmäßige Plurale erhalten das wörtliche letzte Segment;
überschreiben Sie ihn mit `.parameter(...)`.

### Einschränken und umbenennen

```rust
use suprnova::{Router, ResourceAction};

Router::new()
    .resource("posts", PostsCtl)
    .only(&[ResourceAction::Index, ResourceAction::Show])      // auf zwei Verben festlegen
    .names([("index", "posts.list")])                          // einen Standard umbenennen
    .parameter("post_id")                                      // {post} → {post_id}
    .into();
```

Rust-seitige Aliase, die sich an manchen Aufrufstellen besser lesen:
`.keep(...)` für `.only(...)`, `.drop(...)` für `.except(...)`,
`.rename(...)` für `.names(...)`.

### Massenregistrierung

```rust
Router::new()
    .resources([
        ("posts",    Box::new(PostsCtl)    as Box<dyn ResourceController>),
        ("comments", Box::new(CommentsCtl) as Box<dyn ResourceController>),
    ])
    .api_resources([("authors", Box::new(AuthorsCtl) as Box<dyn ResourceController>)]);
```

### Die gesamte Resource autorisieren

`authorize_resource::<U, R>()` hängt die konventionelle
Ability-Prüfung als Pro-Route-Middleware an jede generierte Route -
Laravel-Parität zu `authorizeResource`. Ohne sie bleibt eine
Resource-Oberfläche ungeschützt, sofern nicht jeder Controller-Body
daran denkt, `Gate::authorize` aufzurufen; ein einziges vergessenes
`destroy` liefert ein ungeschütztes Delete aus.

```rust
use suprnova::{Router, Gate};

// Abilities sind über (Ability, Benutzertyp, Resource-Marker-Typ) indiziert.
Gate::define::<User, Post>("view",   |u, _p| u.is_member);
Gate::define::<User, Post>("create", |u, _p| u.is_author);
Gate::define::<User, Post>("update", |u, _p| u.is_author);
Gate::define::<User, Post>("delete", |u, _p| u.is_admin);

let router: Router = Router::new()
    .resource("posts", PostsCtl)
    .authorize_resource::<User, Post>()
    .into();
```

Die Zuordnung von Aktion zu Ability spiegelt Laravel:

| Aktion(en) | Ability |
|---|---|
| `index`, `show`     | `view`   |
| `create`, `store`   | `create` |
| `edit`, `update`    | `update` |
| `destroy`           | `delete` |

`PATCH` teilt sich die `update`-Aktion, wird also identisch zu `PUT`
geschützt. Eine verweigerte Ability bricht mit `403` ab, bevor der
Handler läuft, und eine nicht authentifizierte Anfrage schlägt
geschlossen fehl. Der Resource-Marker `R` braucht nur `Default` - das
Gate unterscheidet nach seinem *Typ*, so wie Laravel nach der
Model-Klasse unterscheidet. Siehe das
[Autorisierungskapitel](authorization.md) für das Definieren der
Abilities selbst.

## Redirects und Views auf Router-Ebene

Drei Sugar-Methoden auf `Router` decken Routendeklarationen ab, die
keine Handler-Funktion brauchen:

```rust
use suprnova::Router;
use serde_json::json;

let router = Router::new()
    // Statischer Redirect: GET /old-pricing → 302 /pricing
    .redirect("/old-pricing", "/pricing", 302)
    // 301-Geschwister
    .permanent_redirect("/legacy", "/new")
    // Statische Inertia-Seite: GET /about rendert die About-Komponente
    .inertia("/about", "About", json!({ "team_size": 4 }))
    .name("about");
```

`Router::inertia` ist Suprnovas `Route::inertia($uri, $component,
$props)`. Es registriert `GET`; eine `HEAD`-Anfrage fällt darauf zurück
und ihr Body wird an der Servergrenze entfernt, sodass nichts Zusätzliches
registriert werden muss. Es liefert einen `RouteBuilder` zurück, daher
können `.name(...)` und `.middleware(...)` wie bei jeder anderen Route
verkettet werden.

Props müssen ein JSON-Objekt oder `null` für keine Props sein. Alles
andere - ein Array, ein String - ist ein Registrierungsfehler, kein
stillschweigend leeres Props-Bag. `try_inertia` ist die fehlbare Form.

`Router::view` ist dieselbe Methode unter ihrem älteren Namen; sie liefert
`Router` statt `RouteBuilder`, daher kann eine damit deklarierte Route
nicht benannt werden. Bevorzugen Sie `inertia`.

### Warum Suprnova abweicht

Laravels `Route::view` rendert ein Blade-Template; Suprnova rendert eine
Inertia-Komponente, weil das Templating-System des Frameworks Inertia
ist, nicht Blade. Eine Folge: Der Komponentenname ist hier ein
Laufzeit-String, daher erhält er nicht die Compile-Zeit-Prüfung der
Seitenkomponente, die das Makro `inertia_response!` durchführt. Schreiben
Sie den Handler mit `inertia_response!` aus, wenn ein Tippfehler im
Komponentennamen beim Build statt erst bei der Anfrage fehlschlagen soll.

Für Redirect-*Responses* (keine Routendeklarationen) - `Redirect::route`,
`Redirect::back`, `Redirect::intended`, signierte Redirects - siehe
[URL-Generierung](urls.md) und [Antworten](responses.md).

## Signierte URLs

HMAC-signierte Routen sind routing-nah (Sie prägen eine URL gegen eine
benannte Route und verifizieren dann die Signatur bei der eingehenden
Anfrage). Sie werden vollständig in [URL-Generierung](urls.md)
behandelt; die Kurzfassung:

```rust
use suprnova::url;

let reset = url::signed_route("password.reset", &[("user", "42")])?;
// /password/reset/42?signature=...

let expires_at = chrono::Utc::now().timestamp() + 3600;
let verify = url::temporary_signed_route("verify.email", &[("user", "42")], expires_at)?;
// /verify/email/42?expires=1748803600&signature=...
```

Verifizieren Sie innerhalb eines Handlers mit
`url::has_valid_signature(&request)` (boolescher Wert) oder
`url::signature_verdict(&request)` (die dreiteilige Aufteilung in
`Valid`/`Expired`/`Invalid`, sodass Sie statt eines generischen 403 eine
Seite "neuen Link anfordern" rendern können).

## Fehlbare Registrierung

Routenregistrierung läuft einmal beim Boot, sodass eine doppelte
oder fehlerhafte Route als Programmiererfehler behandelt wird: Die
einfachen Helfer (`Router::get`, `post`, `put`, `delete`, `ws`,
`RouteBuilder::name`, die `GroupBuilder` → `Router`-`From`-Konvertierung)
**lösen einen Panic aus**, um beim Start sichtbar zu scheitern. Das
ist der richtige Standard für im Quellcode deklarierte Routen.

Wenn Patterns oder Namen aus einer fehlbaren Quelle stammen - dynamische
Konfiguration, ein Plugin-System, ein Test, der absichtlich
widersprüchliche Routen registriert - verwenden Sie die
`try_*`-Geschwister. Sie liefern `Result<_, FrameworkError>` (unter
Nennung der betroffenen Methode, des Pfads oder des widersprüchlichen
Namens), statt zu paniken:

| Panics | Fehlbares Geschwister | Liefert |
|---|---|---|
| `Router::get` / `post` / `put` / `patch` / `delete` / `head` / `options` | `try_get` / `try_post` / `try_put` / `try_patch` / `try_delete` / `try_head` / `try_options` | `Result<RouteBuilder, FrameworkError>` |
| `Router::ws` (und jede `ws_*`-Variante) | `try_ws` (und jede `try_ws_*`-Variante) | `Result<Router, FrameworkError>` |
| `RouteBuilder::name` | `try_name` | `Result<Router, FrameworkError>` |
| `GroupBuilder` → `Router` via `.into()` | `GroupBuilder::try_finalize` | `Result<Router, FrameworkError>` |
| `ResourceRoutes::register` | `try_register` | `Result<Router, FrameworkError>` |

```rust
use suprnova::{FrameworkError, Router};

// `path` stammt aus dynamischer Konfiguration; ein fehlerhaftes oder
// doppeltes Pattern ist behebbar, kein Start-Panic.
fn register_dynamic(router: Router, path: &str) -> Result<Router, FrameworkError> {
    Ok(router.try_get(path, health)?.into())
}
```

Eine doppelte Gruppenroute ist auf dieselbe Weise behebbar - da `From`
nicht fehlbar sein kann, ist das fehlbare Gegenstück zu `.into()` die
inhärente Methode `try_finalize`:

```rust
let router: Router = Router::new()
    .group("/api", |r| r.get("/users", list).post("/users", create))
    .try_finalize()?;
```

Die Panic-auslösenden Helfer bleiben als ergonomische Notausgänge
bestehen; die `try_*`-Geschwister sind rein additiv.

## Warum Suprnova abweicht

**Doppelte Pfadparameter-Syntax.** Laravel verwendet `{param}`; Express
verwendet `:param`. Suprnova akzeptiert beides und normalisiert
`:param` zu `{param}`, bevor der Pfad `matchit` erreicht. Beide Stile
komponieren mit allem anderen - Gruppen, Model-Binding, signierte URLs.
Der Grund ist keine Unentschlossenheit; wir können nicht vorhersagen,
welchen Hintergrund Sie mitbringen, und Routing-Syntax ist ein zu
hochfrequenter Reibungspunkt, um Menschen zum Umlernen zu zwingen.

**Zwei gleichrangige APIs: Makro und Builder.** Laravel liefert eine
DSL (`Route::get(...)`). Suprnova liefert das deklarative
`routes! { ... }`-Makro UND den verkettbaren
`Router::new().get(...).name(...)`-Builder. Beide erzeugen identische
Registrierungen. Das Makro liest sich besser für
Top-Level-Routentabellen; der Builder liest sich besser, wenn Sie Router
dynamisch komponieren (Plugins, generierte Routen, Tests). Wählen Sie,
was zur Aufrufstelle passt - es gibt keine kanonische Antwort, weil
beide Formen erstklassig sind.

**Panics zur Boot-Zeit statt stilles Shadowing.** Ein doppelter
Routenname oder eine Pattern-Kollision löst beim Start einen Panic aus.
Laravels array-indizierte Registries lassen die spätere Registrierung
still gewinnen, was in Ordnung ist, solange Ihre Routendatei der
einzige Registrar ist, aber unsicher wird, sobald Plugins oder
generierte Routen ins Spiel kommen. Die `try_*`-Geschwister sind der
Notausgang, wenn Fehlbarkeit tatsächlich das ist, was Sie wollen.

## Nächste Schritte

- [Controller](controllers.md) - `#[handler]`, Form-Requests, JSON/Inertia zurückgeben
- [Middleware](middleware.md) - der `Middleware`-Trait, Reihenfolge, eigene Middleware bauen
- [URL-Generierung](urls.md) - URLs für benannte Routen, signierte URLs, Redirects, `RouteUrlError`
- [Autorisierung](authorization.md) - Gates und Policies für gebundene Models
- [WebSockets](websockets.md) - `ws!`, der `WebSocketHandler`-Trait, Pro-Route-Konfiguration
