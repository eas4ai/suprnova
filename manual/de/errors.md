# Fehlerbehandlung

Dies ist der Leitfaden für die alltäglichen Muster, um in
Suprnova-Handlern, -Services und -Middleware Code zu schreiben, der
fehlschlagen kann. Das Modell darunter - den Konvertierungsvertrag, die
Panic-Grenze, die 5xx-Bereinigungsregel, die Observability-Hooks -
beschreibt [Fehlermodell](error-model.md). Dieses Kapitel zeigt, was Sie
tatsächlich tippen.

Die Form, die Sie sich merken sollten:

- Handler geben `Response = Result<HttpResponse, HttpResponse>` zurück.
- Der `?`-Operator lässt `FrameworkError`, `AppError`, `DbErr`,
  `ParamError`, `ValidationErrors` und jeden typisierten `HttpError`
  automatisch zu einer `HttpResponse` zusammenfallen.
- Drei freie Hilfsfunktionen (`abort_with`, `abort_if`,
  `abort_unless`) lassen Sie bei einem Statuscode kurzschließen, ohne
  einen Fehlertyp zu benennen.

```rust
use suprnova::{Request, Response, json_response};

pub async fn show(req: Request) -> Response {
    let id = req.param("id")?;          // 400, falls er fehlt
    let user = find_user(id).await?;    // 500 bei DbErr, 404 bei Option::None
    json_response!({ "user": user })
}
```

Der Rest des Kapitels ist der Katalog dessen, was Fehler erzeugt - was
Sie konstruieren, welchen Status es liefert, welche Form der Client
sieht.

## `?` ist die Konvertierung

Jedes `?` in einem Handler-Body führt `From<E> for HttpResponse` aus.
Das Framework verdrahtet diese Impls so, dass die Dinge, die Sie
tatsächlich aufrufen, Fehler liefern, die bereits wissen, wie sie sich
rendern. Sie schreiben nicht die Konvertierung, sondern den Fehlschlag.

```rust
use suprnova::{DB, FrameworkError, Request, Response, json_response};
use sea_orm::EntityTrait;

pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| FrameworkError::param_parse("id", "i64"))?;

    let user = users::Entity::find_by_id(id)
        .one(&*DB::get()?)
        .await?
        .ok_or_else(|| FrameworkError::not_found("User"))?;

    json_response!({ "user": user })
}
```

In diesem Ausschnitt geschehen drei Dinge - keines davon ist sichtbar:

1. `req.param("id")?` → `ParamError` → `FrameworkError::ParamError` (400).
2. `.await?` auf einem SeaORM-Aufruf → `DbErr` →
   `FrameworkError::Database` (500, in der Antwort an den Client
   bereinigt).
3. `.ok_or_else(...)?` konstruiert direkt einen
   `FrameworkError::ModelNotFound` (404).

Alle drei laufen durch dieselbe
`From<FrameworkError> for HttpResponse`-Impl, die im
[Fehlermodell](error-model.md) beschrieben ist.

## `AppError` - Inline-Domain-Fehler

Verwenden Sie `AppError` für einmalige Fehler, die keinen eigenen Typ
verdienen. Die Konstruktoren bilden sich auf Laravels
`abort($status, $msg)`-Form ab:

| Konstruktor | Status |
|---|---|
| `AppError::new(msg)` | 500 |
| `AppError::bad_request(msg)` | 400 |
| `AppError::unauthorized(msg)` | 401 |
| `AppError::forbidden(msg)` | 403 |
| `AppError::not_found(msg)` | 404 |
| `AppError::conflict(msg)` | 409 |
| `AppError::unprocessable(msg)` | 422 |
| `AppError::new(msg).status(code)` | beliebig |

`AppError` hat eine `From`-Implementierung nach `FrameworkError`, sodass
`?` ohne Umstände funktioniert:

```rust
use suprnova::{AppError, Request, Response, json_response};

pub async fn transfer(req: Request) -> Response {
    let amount: i64 = req.param("amount")?.parse()
        .map_err(|_| AppError::bad_request("amount must be a number"))?;

    if amount <= 0 {
        return Err(AppError::unprocessable("amount must be positive").into());
    }

    if amount > balance() {
        return Err(AppError::forbidden("amount exceeds daily limit").into());
    }

    json_response!({ "transferred": amount })
}
```

Beachten Sie die Asymmetrie: `AppError::unauthorized` ist **401**
(fehlende Authentifizierungsdaten), während
`FrameworkError::Unauthorized` **403** ist (eine Policy hat einen
authentifizierten Benutzer abgelehnt). Sie bedeuten unterschiedliche
Dinge; wählen Sie die, die zum Fehlschlag passt.

## `FrameworkError` - das kanonische Enum

Interne Extraktoren, der Container, das Route-Binding, die Validierung,
die Datenbankschicht und Storage erzeugen alle einen `FrameworkError`.
Normalerweise konstruieren Sie einen über einen Convenience-Konstruktor
und lassen `?` ihn routen.

```rust
use suprnova::FrameworkError;

FrameworkError::not_found("User");                    // 404
FrameworkError::bad_request("Bad input");             // 400
FrameworkError::param("user_id");                     // 400
FrameworkError::param_parse("user_id", "i64");        // 400
FrameworkError::validation("email", "required");      // 422
FrameworkError::domain("Conflict", 409);              // 409 (beliebiger Code)
FrameworkError::internal("disk full");                // 500
FrameworkError::database("timeout");                  // 500
FrameworkError::service_not_found::<MyService>();     // 500
FrameworkError::model_not_found("Post");              // 404
```

Der vollständige Satz an Varianten samt Folgen für die Response-Form
steht im [Fehlermodell](error-model.md). Die Konstruktoren oben decken
jeden gängigen Fall ab; direkt zu den Varianten greifen Sie nur, wenn
Sie auf einen empfangenen Fehler matchen.

### Automatische Konvertierungen

`FrameworkError` spricht bereits die Dialekte, die Ihre Abhängigkeiten
ausgeben. Beide `?` hier konvertieren automatisch:

```rust
use suprnova::{DB, FrameworkError};
use sea_orm::ActiveModelTrait;

pub async fn create_user(new_user: users::ActiveModel)
    -> Result<users::Model, FrameworkError>
{
    // DB::get liefert Result<_, FrameworkError>.
    // .insert liefert Result<_, DbErr>, mit From<DbErr> for FrameworkError.
    let user = new_user.insert(&*DB::get()?).await?;
    Ok(user)
}
```

Das Framework implementiert außerdem `From<opendal::Error>` für
Storage-Operationen und `From<ParamError>` für die Extraktion von
Pfadparametern.

### Mit Kontext erneut werfen

Wenn Sie annotieren möchten, woher ein Fehler kam, ohne den Statuscode
zu verlieren, verwenden Sie `.context()`:

```rust
db.insert(user).await
    .map_err(FrameworkError::from)
    .map_err(|e| e.context("creating new user"))?;
```

Die Nachricht wird zu `"creating new user: <original>"`. Strukturierte
Varianten (`Validation`, `ValidationError`, `ModelNotFound`,
`ParamParse`, `PrecognitionFailure`, `Unauthorized`) behalten ihre
Variante, damit der Response-Renderer weiterhin die richtige Form
ausgibt; reine nachrichtentragende Varianten (`Internal`, `Database`,
`Domain`) werden zu einer `Domain` mit vorangestellter Nachricht
abgeflacht, wobei der ursprüngliche Status erhalten bleibt.

### Duplicate-Key-Fehler in 422 verwandeln

Die Validierungsregel `Unique` führt vor dem Schreiben ein
`SELECT COUNT(*)` aus und ist damit unverbindlich - zwei gleichzeitige
Anfragen können beide bestehen und danach beide das Einfügen versuchen.
Die unterlegene Anfrage bekommt aus der Datenbank die Verletzung einer
Unique-Constraint, die sonst als 500 nach außen dringen würde.
`from_unique_violation` übersetzt sie in dasselbe 422, das die
unverbindliche Regel erzeugt hätte:

```rust
use suprnova::FrameworkError;

let user = new_user.insert(db).await.map_err(|e| {
    FrameworkError::from_unique_violation(
        "email",
        "That email address is already registered.",
        e,
    )
})?;
```

Ist der zugrunde liegende `DbErr` keine Verletzung einer
Unique-Constraint, läuft er unverändert als `Database`-Fehler der
500er-Klasse durch. Die Abdeckung der Backends ist das, was SeaORMs
`DbErr::sql_err` erkennt - Postgres, MySQL/MariaDB und SQLite bilden
ihre Duplicate-Key-Fehler alle darauf ab.

## Eigene Domain-Fehler

Drei Stufen, je nachdem, wie wiederverwendbar der Fehler sein muss.

### `#[domain_error]` für den typisierten Fall

Die meisten wiederverwendbaren Fehler wollen einen Namen, einen festen
Status und eine feste Nachrichtenvorlage - keine Nachricht pro Aufruf.
Das Attribut-Makro `#[domain_error]` generiert `Display`,
`std::error::Error`, `HttpError` und `From` für `FrameworkError` in
einem Zug:

```rust
use suprnova::domain_error;

#[domain_error(status = 404, message = "User not found")]
pub struct UserNotFound;

#[domain_error(status = 402, message = "Insufficient funds")]
pub struct InsufficientFunds {
    pub available: i64,
    pub requested: i64,
}
```

An der Aufrufstelle verwenden Sie sie mit `?`:

```rust
use crate::errors::user_not_found::UserNotFound;

pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| FrameworkError::param_parse("id", "i64"))?;

    let user = find_user(id).await
        .ok_or_else(|| FrameworkError::from(UserNotFound))?;

    json_response!({ "user": user })
}
```

Das Makro weist fehlerhafte Attribute zur Compile-Zeit sichtbar
zurück - übergelaufene Statuscodes (`status = 70_000`), falsche
Literaltypen (`message = 42`), unbekannte Schlüssel. So bekommen
Sie wegen eines Tippfehlers nicht stillschweigend den falschen Status.

#### Eines mit der CLI scaffolden

```bash
suprnova make:error UserNotFound
```

Schreibt `src/errors/user_not_found.rs` mit dem Standardwert
`status = 500` und einer abgeleiteten Nachricht in Satzschreibweise und
ergänzt `src/errors/mod.rs` um den Re-Export. Passen Sie `status` und
`message` nach Bedarf an.

### `HttpError` für den handgerollten Fall

Wenn ein Domain-Fehler Laufzeitzustand in der Nachricht braucht (etwa
die am Fehlschlag beteiligten IDs), implementieren Sie `HttpError`
direkt. Der Trait hat zwei Methoden mit sinnvollen Standardwerten:

```rust
use suprnova::HttpError;

#[derive(Debug)]
pub struct InsufficientFunds {
    pub available: i64,
    pub requested: i64,
}

impl std::fmt::Display for InsufficientFunds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Insufficient funds: have {}, need {}",
            self.available, self.requested)
    }
}

impl std::error::Error for InsufficientFunds {}

impl HttpError for InsufficientFunds {
    fn status_code(&self) -> u16 { 402 }
    fn error_message(&self) -> String {
        format!("Need {} units, only {} available.",
            self.requested, self.available)
    }
}
```

Um einen handgerollten `HttpError` an `?` anzuschließen, rufen Sie
`FrameworkError::from_http_error` auf. Ein pauschales
`From<T: HttpError> for FrameworkError` würde mit der bestehenden
`From<AppError>`-Impl kollidieren, deshalb ist die Brücke ein
expliziter Konstruktor:

```rust
account.withdraw(amount)
    .map_err(FrameworkError::from_http_error)?;
```

### Fehler-Enums für die Fehlschläge eines Moduls

Wenn ein Service mehrere verwandte Fehlschläge hat, fassen Sie sie in
einem Enum zusammen und schreiben ein einziges `From` für das ganze
Enum:

```rust
use suprnova::FrameworkError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum OrderError {
    #[error("Order {0} not found")]
    NotFound(i64),

    #[error("Insufficient stock for product {product_id}")]
    InsufficientStock { product_id: i64 },

    #[error("Payment failed: {0}")]
    PaymentFailed(String),

    #[error("Order already shipped")]
    AlreadyShipped,
}

impl From<OrderError> for FrameworkError {
    fn from(err: OrderError) -> Self {
        let status = match &err {
            OrderError::NotFound(_) => 404,
            OrderError::InsufficientStock { .. } => 422,
            OrderError::PaymentFailed(_) => 402,
            OrderError::AlreadyShipped => 409,
        };
        FrameworkError::Domain {
            message: err.to_string(),
            status_code: status,
        }
    }
}
```

Sobald das `From` existiert, fädelt sich das Enum genauso durch `?` wie
jeder andere Fehlertyp.

## `abort_with` / `abort_if` / `abort_unless`

Drei Hilfsfunktionen schließen einen Handler bei einem Status kurz. Sie
spiegeln Laravels `abort` / `abort_if` / `abort_unless`. (Die freie
Funktion wird als `abort_with` exportiert und nicht als `abort`, damit
Letzteres als Methodenname auf Nutzertypen frei bleibt.)

```rust
use suprnova::{abort_if, abort_unless, abort_with, Request, Response, json_response};

pub async fn show(req: Request) -> Response {
    abort_unless(Auth::user().await?.is_some(), 401, "must be logged in")?;
    abort_if(req.param("id")? == "0", 404, "User not found")?;
    abort_with(503, "scheduled maintenance")?;

    json_response!({ "ok": true })
}
```

Jede liefert `Result<(), FrameworkError>`, das `?` erledigt also die
Arbeit. Der zugrunde liegende Fehler ist `FrameworkError::Domain {
message, status_code }`, der über dieselbe Body-Form gerendert wird wie
jeder andere Fehler. Statuscodes außerhalb des gültigen Bereichs werden
vom Response-Renderer auf 500 heruntergestuft; Sie müssen sich an der
Aufrufstelle nicht gegen ungültige Eingaben absichern.

## `ValidationErrors` - die Error-Bag im Laravel-Format

Wenn die Validierung fehlschlägt - zur `#[derive(Validate)]`-Zeit oder
in einem `after_validation`-Body -, gibt das Framework die JSON-Form
aus, die Laravel- und Inertia-Frontends erwarten:

```json
{
    "message": "The given data was invalid.",
    "errors": {
        "email": ["The email field must be a valid email address."],
        "password": ["The password field must be at least 8 characters."]
    },
    "request_id": "8f9e1a2b-c3d4-..."
}
```

Meistens bauen Sie das nicht direkt - `#[derive(Validate)]` läuft, und
das Framework konvertiert `validator::ValidationErrors` für Sie. Wenn
Sie Fehler imperativ ergänzen müssen (feldübergreifende Regeln,
asynchrone Eindeutigkeitsprüfungen, die `Unique` ergänzen), bauen Sie
ein `ValidationErrors` und geben es zurück:

```rust
use suprnova::{FrameworkError, ValidationErrors};

pub async fn after_validation(payload: &Signup) -> Result<(), FrameworkError> {
    let mut errs = ValidationErrors::new();

    if payload.email.ends_with("@example.com") {
        errs.add("email", "example.com addresses are not allowed");
    }
    if payload.password == payload.email {
        errs.add("password", "password must not match email");
    }

    errs.into_result().map_err(FrameworkError::Validation)
}
```

`add_to_bag` ordnet ein Feld einer benannten Bag zu (in der Form von
Laravels `withErrors($errors, 'profile')`), indem der Bag-Name mit einem
`.`-Trenner vorangestellt wird. Nützlich, wenn eine Response Fehler aus
mehreren Teilformularen trägt, die sich keinen flachen Namensraum teilen
können:

```rust
let mut errs = ValidationErrors::new();
errs.add_to_bag("profile", "bio", "must be under 280 characters");
errs.add_to_bag("billing", "card", "expired");
// Fehler-Map: { "profile.bio": [...], "billing.card": [...] }
```

`from_validator(ve)` konvertiert ein `validator::ValidationErrors`;
`retain_fields(&keep)` liefert eine Kopie, die nur die aufgeführten
Einträge enthält (intern genutzt vom `Precognition-Validate-Only`-Header
von Precognition).

## Observability über `ErrorOccurred` einhängen

Jede 5xx-Response feuert ein `ErrorOccurred`-Event - einschließlich der
aus Panics synthetisierten. Lauschen Sie darauf genauso, wie Sie auf
jedes andere Event lauschen:

```rust
use std::sync::Arc;
use suprnova::{ErrorOccurred, EventFacade, FrameworkError, Listener};

pub struct SentryReporter;

#[suprnova::async_trait]
impl Listener<ErrorOccurred> for SentryReporter {
    async fn handle(&self, evt: &ErrorOccurred) -> Result<(), FrameworkError> {
        sentry::capture_message(&evt.error_message, sentry::Level::Error);
        Ok(())
    }
}

// In bootstrap.rs:
// `listen` leitet beide Generics aus dem Listener-Typ ab. Es liefert
// `()` (die Registrierung kann nicht fehlschlagen), also kein `?` und kein Result.
EventFacade::listen::<ErrorOccurred, SentryReporter>(Arc::new(SentryReporter)).await;
```

Das Event trägt die rohe Fehlermeldung (der Body der Antwort an den
Client bleibt bereinigt - siehe [Fehlermodell](error-model.md)), den Status und
die korrelierbare Request-ID. Das ist Suprnovas Äquivalent zu Laravels
`report()`-Callback am Exception-Handler.

## Muster, die Sie oft schreiben werden

### Einen Pfadparameter als typisierten Wert parsen

```rust
let id: i64 = req.param("id")?.parse()
    .map_err(|_| FrameworkError::param_parse("id", "i64"))?;
```

`ParamError` konvertiert bereits zu 400; `param_parse` ist das
Gegenstück für den Parse-Fehlschlag und rendert dieselbe Form.

### Per ID nachschlagen, 404 bei Abwesenheit

```rust
let user = users::Entity::find_by_id(id)
    .one(&*DB::get()?)
    .await
    .map_err(FrameworkError::from)?
    .ok_or_else(|| FrameworkError::not_found("User"))?;
```

`map_err(FrameworkError::from)?` reicht den SeaORM-`DbErr` über
`From<DbErr> for FrameworkError` und danach über
`From<FrameworkError> for HttpResponse` weiter. Rust verkettet
`From`-Impls nicht automatisch über zwei Sprünge hinweg, deshalb ist
das explizite `.map_err` nötig.

Oder mit der Eloquent-Schicht (die SeaORM bereits umschließt und direkt
`Result<_, FrameworkError>` liefert):

```rust
use suprnova::Model;

let user = User::find_or_fail(id).await?;
```

`find_or_fail` ist `find(id).ok_or(ModelNotFound)` in einer Verpackung.

### Eine Aktion autorisieren

```rust
let user = Auth::user().await?
    .ok_or_else(|| AppError::unauthorized("login required"))?;
abort_unless(post.owner_id == user.id() || user.is_admin(), 403,
    "you don't own this post")?;
```

`abort_unless` liefert `Result<(), FrameworkError>`; das `?` lässt es in
den Fehlerarm Ihres Handlers zurückfallen.

### Service, der typisierte Fehler liefert

```rust
use suprnova::{App, FrameworkError, injectable};

#[injectable]
pub struct UserService;

impl UserService {
    pub async fn find_by_email(&self, email: &str)
        -> Result<users::Model, FrameworkError>
    {
        users::Entity::find()
            .filter(users::Column::Email.eq(email))
            .one(&*DB::get()?)
            .await?
            .ok_or_else(|| FrameworkError::not_found("User"))
    }
}

// Aufrufstelle:
pub async fn show(req: Request) -> Response {
    let email = req.param("email")?;
    let user = App::resolve::<UserService>()?
        .find_by_email(email)
        .await?;
    json_response!({ "user": user })
}
```

`App::resolve::<UserService>()?` liefert `Result<Arc<UserService>,
FrameworkError>`. Das verkettete `?` lässt sowohl den Fehlschlag beim
Auflösen als auch den beim Nachschlagen zu einer Response
zusammenfallen.

## Spickzettel

| Sie wollen… | Greifen Sie zu |
|---|---|
| Inline-Fehler mit einem Status | `AppError::bad_request("…")` und Verwandte |
| Typisierter, wiederverwendbarer Fehler | `#[domain_error(status = …, message = "…")]` |
| Generiertes Scaffold | `suprnova make:error UserNotFound` |
| Handgerollt, mit Laufzeitzustand | `impl HttpError for MyError` |
| Handgerolltes an `?` anschließen | `FrameworkError::from_http_error(e)` |
| Bei einem Status kurzschließen | `abort_with` / `abort_if` / `abort_unless` |
| 404 bei fehlendem Model | `FrameworkError::not_found("User")` / `Model::find_or_fail` |
| Parse-Fehlschlag bei einem Pfadparameter | `FrameworkError::param_parse("id", "i64")` |
| Validierungsfehler auf Feldebene | `FrameworkError::validation("email", "…")` |
| Error-Bag über mehrere Felder | `ValidationErrors::new().add(…)` + `Validation(errs)` |
| Duplicate-Key-Verletzung → 422 | `FrameworkError::from_unique_violation(field, msg, e)` |
| Einen bestehenden Fehler annotieren | `err.context("creating user")` |
| Jedes 5xx beobachten | Auf `ErrorOccurred` lauschen |

## Nächste Schritte

- [Fehlermodell](error-model.md) - Varianten, Konvertierungsvertrag,
  5xx-Bereinigung, Panic-Grenze
- [Validierung](validation.md) - `#[derive(Validate)]`, Form-Requests
  und `after_validation`
- [Antworten](responses.md) - `HttpResponse`-Builder, Status, Header
- [Ereignisse](events.md) - auf `ErrorOccurred` und andere eingebaute
  Events lauschen
- [Request-Lifecycle](lifecycle.md) - wo im Request-Flow die
  Fehlerkonvertierung läuft
