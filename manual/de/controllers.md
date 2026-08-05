# Controller

Ein Suprnova-Controller ist einfach eine async-Funktion. Sie nimmt sich
aus der Anfrage, was sie braucht - typisierte Pfadparameter, ein
geladenes Model, ein validiertes Formular - und liefert eine
`Response`. Es gibt keine Controller-Basisklasse. Es gibt keine
Verdrahtungsdatei für einen Service-Locator. Die Funktion ist die
Einheit, und das `#[handler]`-Attribut klebt sie an die Routing-Makros.

```rust
use suprnova::{handler, json_response, Response};
use crate::models::user;

// GET /users/{user}
#[handler]
pub async fn show(user: user::Model) -> Response {
    json_response!({
        "id": user.id,
        "name": user.name,
        "email": user.email,
    })
}
```

Die Signatur dieses Handlers erledigt drei Dinge auf einmal: Sie
deklariert den Routenparameter (`user`), holt die Zeile aus der
Datenbank und liefert 404, wenn die Zeile nicht da ist. Nichts davon ist
von Hand geschrieben. `#[handler]` liest die Argumenttypen und generiert
die Extraktion.

## Einen Controller generieren

```bash
suprnova make:controller User
```

Das schreibt `src/controllers/user.rs` mit einem einzelnen
`invoke`-Stub und fügt `pub mod user;` in `src/controllers/mod.rs`
hinzu. Der Stub ist der minimal lauffähige Handler:

```rust
//! User controller

use suprnova::{handler, json_response, Request, Response};

#[handler]
pub async fn invoke(_req: Request) -> Response {
    json_response!({
        "controller": "User"
    })
}
```

Fügen Sie der Datei so viele Funktionen hinzu, wie Sie möchten -
Suprnova verfolgt keine Controller-"Klassen", sondern nur Funktionen.
Viele Apps teilen nach Ressource auf (`controllers::user::{index, show,
store, update, destroy}`), aber nichts im Framework schreibt das vor.

Der Name wird für den Dateinamen in `snake_case` umgewandelt: Aus
`OrderItem` wird `order_item.rs`.

## Das `#[handler]`-Attribut

Das Makro klassifiziert den Typ jedes Parameters und generiert den
passenden Extraktor. Vier Kategorien:

| Parametertyp | Extrahiert über | Fehlerverhalten |
|---|---|---|
| `Request` | reicht die Anfrage unverändert durch | - |
| `i32`, `i64`, `u32`, `u64`, `usize`, `String` | `FromParam` - parst den gleichnamigen Routenparameter | 400 bei Parse-Fehler, 400 bei fehlendem Parameter |
| `T: AutoRouteBinding` (jedes Eloquent-`Model`) | parst den Parameter als Primärschlüssel des Models, lädt die Zeile | 400 bei Parse-Fehler, 404 wenn nicht gefunden |
| Alles andere (`T: FromRequest`) | ruft `T::from_request(req)` auf - typischerweise einen `#[derive(FormRequest)]`-Validator | was auch immer `from_request` liefert; 422 bei Validierungsfehlern |

Das Makro führt die Extraktionen in Deklarationsreihenfolge aus, sodass
der Body Ihrer Funktion vollständig typisierte Werte sieht. Schlägt eine
Extraktion fehl, wird über `?` kurzgeschlossen und der Handler-Body
läuft nie.

### Pfadparameter

```rust
// Route: get!("/users/{id}", controllers::user::show)
#[handler]
pub async fn show(id: i64) -> Response {
    json_response!({ "user_id": id })
}

// Route: get!("/posts/{post_id}/comments/{comment_id}", show_comment)
#[handler]
pub async fn show_comment(post_id: i64, comment_id: i64) -> Response {
    json_response!({
        "post_id": post_id,
        "comment_id": comment_id,
    })
}
```

Der Argumentname muss zum Platzhalter der Route passen: `{id}` verlangt
`id: …`. Der Argumenttyp wird über `FromParam` geparst. Fehlerhafte
Eingaben (`/users/abc` gegen `id: i64`) liefern 400 mit einer Meldung,
die den Parameter und den Zieltyp benennt.

### Route-Model-Binding

`Eloquent`-Models implementieren `AutoRouteBinding` automatisch.
Deklarieren Sie das Model als Argument, und das Framework lädt es:

```rust
use suprnova::{handler, json_response, Response};
use crate::models::user;

// Route: get!("/users/{user}", controllers::user::show)
#[handler]
pub async fn show(user: user::Model) -> Response {
    json_response!({
        "id": user.id,
        "name": user.name,
        "email": user.email,
    })
}
```

Der Name des Routenplatzhalters (`{user}`) und der Argumentname
(`user`) müssen übereinstimmen. Das Framework parst die
Parameter-Zeichenkette als den Primärschlüssel-Typ des Models, ruft
`Entity::find_by_pk` auf und liefert 404, wenn die Zeile fehlt. Jede
`#[suprnova::model]`-Struktur bindet automatisch; das
`route_binding!`-Makro bleibt für handgeschriebene SeaORM-Entities
verfügbar, die `#[suprnova::model]` nicht verwenden - siehe
[Makros](macros.md#route_binding).

### Form-Requests

Alles, was `FromRequest` implementiert, klinkt sich auf dieselbe Weise
ein. Der übliche Fall ist eine `#[derive(FormRequest)]`-Struktur, die
den Anfrage-Body validiert und im Fehlerfall ein 422 mit feldweise
geschlüsselten Fehlern liefert:

```rust
use suprnova::{attrs, handler, json_response, Response};
use crate::models::user;
use crate::requests::UpdateUserRequest;

// Route: put!("/users/{user}", controllers::user::update)
#[handler]
pub async fn update(user: user::Model, form: UpdateUserRequest) -> Response {
    let id = user.id;
    user.update(attrs! { name: form.name, email: form.email }).await?;
    json_response!({ "updated": id })
}
```

Siehe [Form-Requests](requests.md) für das Validator-Derive und die
vollständige Validierungs-Pipeline.

### Wenn Sie den rohen `Request` brauchen

Wenn Sie lieber von Hand extrahieren - oder wenn Sie einen Header, ein
Cookie oder einen Query-String brauchen - nehmen Sie `Request` direkt
entgegen:

```rust
use suprnova::{handler, json_response, Request, Response};

#[handler]
pub async fn show(req: Request) -> Response {
    let id = req.param("id")?;             // Routenparameter, 400 wenn er fehlt
    let ua = req.header("User-Agent");      // Option<&str>
    let page: u32 = req.query_param("page") // Option<String>
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);

    json_response!({ "id": id, "ua": ua, "page": page })
}
```

Sie können beliebig mischen: `pub async fn nested(category_id: i64, product: product::Model, req: Request)` ist eine gültige Signatur. Das Makro extrahiert jedes Argument nach seiner eigenen Regel.

## Der `Response`-Vertrag

`Response` ist ein Alias für `Result<HttpResponse, HttpResponse>`. Beide
Zweige tragen denselben Payload-Typ, weshalb `?` überall funktioniert.
Die Middleware-Chain führt das Ergebnis an der Grenze mit einer
einzigen Zeile zusammen:

```rust
result.unwrap_or_else(|e| e)
```

Das ist derselbe Vertrag, auf den sich jede `?`-Propagationsstelle
verlässt. Fehler werden über `From<FrameworkError> for HttpResponse`
konvertiert, bevor sie die Chain erreichen - siehe
[Fehlermodell](error-model.md) für das vollständige Bild.

Der Body eines Handlers liest sich von oben nach unten und verwendet
`?` zum Aussteigen:

```rust
use suprnova::{handler, json_response, Response};
use crate::models::user;

#[handler]
pub async fn show(id: i64) -> Response {
    let user = user::Model::find_or_fail(id).await?;
    let invoices = user.invoices().get().await?;
    json_response!({ "user": user, "invoices": invoices })
}
```

Liefert `find_or_fail` ein `Err`, verlässt die Funktion mit einem 404.
Schlägt `invoices().get()` fehl, bekommen Sie ein 500. Keine
`match`-Ausdrücke, keine Exception-Handler.

## Responses erzeugen

Drei Makros und ein Builder decken die gängigen Fälle ab:

```rust
use suprnova::{handler, json_response, text_response, HttpResponse, Response, ResponseExt};

#[handler]
pub async fn json_handler() -> Response {
    json_response!({
        "users": [
            {"id": 1, "name": "John"},
            {"id": 2, "name": "Jane"},
        ]
    })
}

#[handler]
pub async fn health() -> Response {
    text_response!("OK")
}

#[handler]
pub async fn store() -> Response {
    // Eingebauter verkettbarer Status / eingebaute Header über ResponseExt.
    json_response!({ "id": 1, "created": true }).status(201)
}

#[handler]
pub async fn page() -> Response {
    Ok(HttpResponse::html("<h1>Hello</h1>"))
}
```

`json_response!`, `text_response!` und `HttpResponse::*` erzeugen alle
denselben `Response`-Typ. Der `ResponseExt`-Trait ergänzt
`.status(...)`, `.header(...)`, `.cookie(...)` und `.with_headers(...)`,
sodass Sie Konfiguration an ein Makro-Ergebnis anketten können.

Für alles Weitere - Datei-Downloads, gestreamte Bodys,
Inertia-Responses, Redirects - siehe [Antworten](responses.md).

## Redirects

`redirect!("route.name")` prüft zur Compile-Zeit, dass die Route
existiert, und liefert einen Builder, an den Sie Konfiguration anketten
können:

```rust
use suprnova::{handler, redirect, Response};

#[handler]
pub async fn store() -> Response {
    // Den Benutzer anlegen…
    redirect!("users.index").into()
}

#[handler]
pub async fn update(id: i64) -> Response {
    redirect!("users.show")
        .with("id", id.to_string())
        .into()
}

#[handler]
pub async fn search() -> Response {
    redirect!("users.index")
        .query("page", "1")
        .query("sort", "name")
        .into()
}
```

`.with(key, value)` füllt einen Routenplatzhalter; `.query(key, value)`
hängt einen Query-String-Parameter an; `.flash(key, value)` schreibt in
die Flash-Bag der Session für die nächste Anfrage. `.into()` wandelt den
Builder in eine `Response` um.

Existiert die benannte Route nicht, lässt das Makro den Build mit einer
Liste der verfügbaren Routennamen fehlschlagen - Tippfehler kommen so
vor dem Staging ans Licht.

## Vom Container injizierte Services

Lösen Sie Services mit `App::resolve` (konkrete Typen) oder
`App::resolve_make` (Trait-Objekte) aus dem Container auf. Beide liefern
`Result<_, FrameworkError>`, komponieren sich also mit `?`:

```rust
use suprnova::{handler, json_response, App, Response};
use crate::services::UserService;

#[handler]
pub async fn index() -> Response {
    let user_service = App::resolve::<UserService>()?;
    let users = user_service.list_all().await?;
    json_response!({ "users": users })
}
```

Wenn Sie Actions mit `#[injectable]` binden, ruft ein Controller sie
genau so auf. Siehe [Aktionen](actions.md) für die Form einer Action
und [Service Container](container.md) für die vollständige
Container-Oberfläche - Binding, Factories, die Lookup-Kaskade
task-local / thread-local / global.

## Ein ausgearbeiteter RESTful-Controller

```rust
// src/controllers/user.rs
use suprnova::{attrs, handler, json_response, redirect, Response, ResponseExt};
use crate::models::user;
use crate::requests::{StoreUserRequest, UpdateUserRequest};

// GET /users
#[handler]
pub async fn index() -> Response {
    let users = user::Model::all().await?;
    json_response!({ "users": users })
}

// GET /users/{user}
#[handler]
pub async fn show(user: user::Model) -> Response {
    json_response!({ "user": user })
}

// POST /users
#[handler]
pub async fn store(form: StoreUserRequest) -> Response {
    let user = user::Model::create(attrs! {
        name: form.name,
        email: form.email,
    }).await?;
    json_response!({ "user": user }).status(201)
}

// PUT /users/{user}
#[handler]
pub async fn update(user: user::Model, form: UpdateUserRequest) -> Response {
    let id = user.id;
    user.update(attrs! {
        name: form.name,
        email: form.email,
    }).await?;
    json_response!({ "updated": id })
}

// DELETE /users/{user}
#[handler]
pub async fn destroy(user: user::Model) -> Response {
    user.delete().await?;
    redirect!("users.index").into()
}
```

Registrieren Sie sie mit dem `routes!`-Makro:

```rust
// src/routes.rs
use suprnova::{delete, get, post, put, routes};
use crate::controllers;

routes! {
    get!("/users",           controllers::user::index   ).name("users.index"),
    get!("/users/{user}",    controllers::user::show    ).name("users.show"),
    post!("/users",          controllers::user::store   ).name("users.store"),
    put!("/users/{user}",    controllers::user::update  ).name("users.update"),
    delete!("/users/{user}", controllers::user::destroy ).name("users.destroy"),
}
```

Der Routenplatzhalter `{user}` passt zum Argumentnamen `user: user::Model`, und daran erkennt das Framework, welches Pfadsegment das Model lädt.

## Die `Request`-API

Die Methoden, zu denen Sie am häufigsten greifen, wenn Sie `Request`
direkt entgegennehmen:

| Methode | Liefert | Anmerkungen |
|---|---|---|
| `method()` | `&hyper::Method` | HTTP-Methode |
| `path()` | `&str` | URL-Pfad |
| `param(name)` | `Result<&str, ParamError>` | Routenparameter; `?` zum Aussteigen |
| `params()` | `&HashMap<String, String>` | alle Routenparameter |
| `query()` | `Option<&str>` | roher Query-String |
| `query_param(key)` | `Option<String>` | einzelner Wert aus dem Query-String |
| `query_params()` | `HashMap<String, String>` | alle Query-Parameter |
| `query_into::<T>()` | `Result<T, FrameworkError>` | typisiert deserialisieren |
| `header(name)` | `Option<&str>` | einzelner Header |
| `headers()` | `&hyper::HeaderMap` | vollständige Header-Map |
| `has_header(name)` | `bool` | Prüfung auf Vorhandensein |
| `bearer_token()` | `Option<String>` | geparstes `Authorization: Bearer …` |
| `cookie(name)` | `Option<String>` | einzelner Cookie-Wert |
| `cookies()` | `HashMap<String, String>` | alle Cookies |
| `ip()` | `Option<String>` | IP der Gegenstelle, berücksichtigt X-Forwarded-For |
| `secure()` | `bool` | HTTPS-Erkennung (inkl. Proxys) |
| `is_method(m)` | `bool` | ohne Unterscheidung von Groß- und Kleinschreibung |
| `is_inertia()` | `bool` | Inertia-XHR-Header |
| `ajax()` | `bool` | `X-Requested-With: XMLHttpRequest` |
| `expects_json()` / `wants_json()` | `bool` | Auswertung des Accept-Headers |
| `route_name()` | `Option<String>` | das `.name(...)` der gematchten Route |
| `json::<T>()` | `Result<T, FrameworkError>` | Body als JSON parsen (verbraucht ihn) |
| `form::<T>()` | `Result<T, FrameworkError>` | als form-urlencoded parsen |
| `input::<T>()` | `Result<T, FrameworkError>` | Parsen je nach Content-Type |

Das ist eine Laravel-förmige Oberfläche - jede Methode hier spiegelt
eine Methode auf Laravels `Request`-Klasse.

## Dateilayout

Konvention:

```
src/
├── controllers/
│   ├── mod.rs          # pub mod home; pub mod user; ...
│   ├── home.rs
│   ├── user.rs
│   └── api/
│       ├── mod.rs
│       └── user.rs
├── routes.rs           # routes! { ... }
└── main.rs
```

Nichts im Framework erzwingt dieses Layout - Controller können überall
liegen, solange sie von `routes.rs` aus erreichbar sind. Die Konvention
existiert, weil das Scaffolding genau das ausgibt und weil Routen und
Controller das natürliche Paar sind.

## Warum Suprnova abweicht

Laravel-Controller sind Klassen, die `Illuminate\Routing\Controller`
erweitern. Methoden werden auf Instanzen aufgerufen, die der Container
pro Anfrage auflöst, und dort findet die Konstruktor-Injektion statt.
Auf PHP ist das Muster in Ordnung - ein `new` bei jeder Anfrage ist
billig, wenn der gesamte Prozess nach der Response ohnehin abgebaut
wird.

In Rust hieße dieses Muster entweder (a) pro Anfrage eine
Controller-Struktur zu allozieren, was einen `Arc`-Klon kostet, den Sie
nicht brauchen, oder (b) Dependency Injection über eine
Basisklassen-Hierarchie nachzubauen, die sich nicht auszahlt.

Suprnova wählt das einfachere Modell: Ein Controller ist eine
freistehende async-Funktion, und "Abhängigkeiten" sind entweder
Auflösungen aus dem Container (`App::resolve::<Service>()?`) oder
Argumente, deren Typ die Extraktion bestimmt (`form:
UpdateUserRequest`). Konstruktor-Injektion findet an der
`#[injectable]`-Grenze in [Aktionen](actions.md) statt, wo sie
hingehört. Der Handler bleibt eine reine Funktion von der Anfrage zur
Response, wodurch er sich mühelos isoliert testen lässt: einen
`Request` bauen, die Funktion aufrufen, das Ergebnis prüfen.

## Nächste Schritte

- [Routing](routing.md) - wozu `routes!`, `get!`, `post!` und `.name()` expandieren
- [Form-Requests](requests.md) - typisierte Validierung über `#[derive(FormRequest)]`
- [Antworten](responses.md) - JSON, HTML, Dateien, Streams, Inertia-Seiten, Redirects
- [Service Container](container.md) - was `App::resolve` tatsächlich tut
- [Aktionen](actions.md) - wo die Geschäftslogik außerhalb des Controllers lebt
- [Fehlermodell](error-model.md) - wie `?` einen `FrameworkError` in eine Response verwandelt
