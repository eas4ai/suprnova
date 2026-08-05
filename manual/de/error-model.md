# Fehlermodell

Dieses Kapitel beschreibt das Modell hinter Suprnovas Fehlerbehandlung -
die Typen, den Konvertierungsvertrag und die Sicherheitsgarantien, die
das Framework Ihnen kostenlos mitgibt. Für die alltäglichen
Handler-Muster (`?`, Fehler zurückgeben, eigene Domain-Fehler bauen)
siehe [Fehlerbehandlung](errors.md); dieses Kapitel erklärt, *warum*
diese Muster so funktionieren, wie sie es tun.

Wenn Sie sich nur eine Sache von dieser Seite merken sollten:
**Fehler sind in Suprnova Werte, keine Exceptions**. Jeder Fehler wird
am Ende über eine einzige, totale Konvertierung zu einer
`HttpResponse`. Es gibt keinen globalen Exception-Handler, weil es
keine globale Exception gibt.

## Die Form

Suprnovas Fehlermodell hat fünf bewegliche Teile:

| Typ | Rolle |
|---|---|
| `Response = Result<HttpResponse, HttpResponse>` | Der Vertrag, den jeder Handler erfüllt - beide Arme sind bereits Responses |
| `FrameworkError` | Das kanonische Fehler-Enum des Frameworks; jeder interne Fehlerpfad erzeugt eines |
| `AppError` | Ad-hoc-Domain-Fehler für den Inline-Gebrauch ohne eigenen Typ |
| `HttpError` (Trait) | Was Ihre eigenen typisierten Domain-Fehler implementieren, um einen Status + eine Nachricht zu erhalten |
| `ValidationErrors` | Die Error-Bag im Laravel/Inertia-Format für Fehler pro Feld |

Alle fünf fallen über `From`-Implementierungen auf eine einzige
`HttpResponse` zusammen. Der `?`-Operator erledigt die Konvertierung an
der Aufrufstelle; die Middleware-Chain erledigt sie an der
Request-Grenze; der Panic-Handler erledigt sie, wenn sich etwas
losgelöst hat. Es gibt eine Body-Form für alles, und eine
Bereinigungsregel für 5xx.

## `Response` ist `Result<HttpResponse, HttpResponse>`

Das gibt jeder Handler zurück:

```rust
pub type Response = Result<HttpResponse, HttpResponse>;
```

Beide Arme tragen denselben Payload-Typ - genau darum geht es. Wenn die
Middleware-Chain die Ausführung Ihres Handlers beendet, fasst sie das
Ergebnis mit einer einzigen Zeile zusammen:

```rust
result.unwrap_or_else(|e| e)
```

Das Framework muss nicht wissen, ob Ihr Handler „erfolgreich“ oder
„fehlgeschlagen“ ist - beide Arme sind bereits fertig gerenderte
HTTP-Responses. Die Unterscheidung existiert nur, damit `?` seine
Arbeit verrichten kann:

```rust
use suprnova::{Request, Response, json_response};

pub async fn show(req: Request) -> Response {
    // `?` bricht bei Err sofort ab. Jede Konvertierung unten erzeugt eine
    // HttpResponse über eine From-Implementierung - die Chain fasst beide Arme zusammen.
    let id: i64 = req.param("id")?.parse().map_err(|_| {
        suprnova::FrameworkError::param_parse("id", "i64")
    })?;
    let user = User::find_or_fail(id).await?;  // 404, falls nicht gefunden
    Ok(json_response!({ "user": user }))
}
```

Genau dieser eine Vertrag - jeder Fehlerpfad erzeugt über `From` eine
`HttpResponse` - ist der Kern des Modells. Alles andere in diesem
Kapitel beschreibt, was die verschiedenen `From`-Implementierungen
tatsächlich tun.

### Warum Suprnova abweicht

Laravel wirft Exceptions und leitet sie durch eine globale
`Handler`-Klasse, die in `app/Exceptions/Handler.php` registriert ist.
Das Framework fängt alles ab, fragt den Handler „Was soll ich
rendern?“ und gibt die Response aus. PHPs Unwinding-Exception-Modell
macht das naheliegend.

Rust kennt in Anwendercode keine Unwinding-Exceptions. Das Äquivalent
in Suprnova ist die `From<FrameworkError> for HttpResponse`-
Implementierung plus das `ErrorOccurred`-Event. Die Konvertierung ist
der Renderer; das Event ist die Stelle, an der Sie Observability
einhängen (Sentry, PagerDuty, strukturierte Log-Shipper). Sie
registrieren keine Handler-Klasse - die Konvertierung ist eine
Funktion, und das Lauschen auf `ErrorOccurred` ist der
Erweiterungspunkt. Gleiche Oberfläche, andere Mechanik.

## `FrameworkError` - das kanonische Enum

Jeder Fehlerpfad innerhalb des Frameworks - Extractors, Route-Binding,
der Container, Validierung, die Datenbankschicht, Storage - erzeugt
einen `FrameworkError`. Es ist ein Enum mit vierzehn Varianten, jede
mit ihrem HTTP-Status versehen:

```rust
pub enum FrameworkError {
    ServiceNotFound { type_name: &'static str },        // 500
    ParamError { param_name: String },                   // 400
    ValidationError { field: String, message: String },  // 422
    Database(String),                                    // 500
    Internal { message: String },                        // 500
    Domain { message: String, status_code: u16 },        // *
    Validation(ValidationErrors),                        // 422
    Unauthorized,                                        // 403
    ModelNotFound { model_name: String },                // 404
    ParamParse { param: String, expected_type: &'static str }, // 400
    UnsupportedMediaType,                                // 415
    PrecognitionSuccess,                                 // 204
    PrecognitionFailure(ValidationErrors),               // 422
    AlreadyReported,                                     // nur CLI
}
```

Sie matchen selten direkt auf die Variante. Stattdessen konstruieren
Sie eine über einen Convenience-Konstruktor und lassen `?` den Rest
erledigen:

```rust
use suprnova::FrameworkError;

// Alle diese erzeugen einen FrameworkError mit dem passenden Status:
FrameworkError::not_found("User");                    // → ModelNotFound, 404
FrameworkError::bad_request("Bad input");             // → Domain, 400
FrameworkError::param("user_id");                     // → ParamError, 400
FrameworkError::param_parse("user_id", "i64");        // → ParamParse, 400
FrameworkError::validation("email", "required");      // → ValidationError, 422
FrameworkError::domain("Conflict", 409);              // → Domain, 409
FrameworkError::internal("disk full");                // → Internal, 500
FrameworkError::database("timeout");                  // → Database, 500
```

Es gibt weder einen `unauthorized()`- noch einen `forbidden()`-
Konstruktor auf `FrameworkError` - `Unauthorized` ist eine feste
Variante, die Laravels Meldung „This action is unauthorized.“ mit
Status 403 trägt, und 401-Fälle laufen über `AppError::unauthorized`
(nächster Abschnitt). Beachten Sie: Die Variante heißt `Unauthorized`,
aber der Status ist 403, weil sie Laravels Autorisierungs-Ablehnung
modelliert, nicht die HTTP-Authentifizierung.

### Automatische Konvertierung

`FrameworkError` implementiert `From<sea_orm::DbErr>` und
`From<opendal::Error>`, sodass Datenbank- und Storage-Fehler ohne
Wrapping durch `?` fließen:

```rust
use suprnova::{DB, FrameworkError};
use sea_orm::ActiveModelTrait;

pub async fn create_user(new_user: ActiveModel) -> Result<Model, FrameworkError> {
    // Beide `?`-Aufrufe hier konvertieren automatisch in FrameworkError:
    // - DB::get liefert Result<_, FrameworkError>
    // - insert liefert Result<_, DbErr>, und dafür existiert From<DbErr> for FrameworkError
    let user = new_user.insert(&*DB::get()?).await?;
    Ok(user)
}
```

Wenn Ihr Code `Result<_, FrameworkError>` zurückgibt, spricht jeder
gängige Fehler, den Ihre Abhängigkeiten erzeugen, bereits die richtige
Sprache. Das `?` im Controller tut nichts weiter, als einen Fehlertyp
in einen anderen zu konvertieren.

### Kontext hinzufügen

Wenn Sie einen Fehler mit Operations-Kontext erneut werfen möchten,
verwenden Sie `.context()`:

```rust
db.insert(user).await
    .map_err(FrameworkError::from)
    .map_err(|e| e.context("creating new user"))?;
```

Die Nachricht wird zu `"creating new user: <original>"`. Die Variante
bleibt erhalten, wo es darauf ankommt - `Validation`,
`ValidationError`, `PrecognitionFailure`, `Unauthorized`,
`ModelNotFound` und `ParamParse` behalten ihre Struktur, damit der
Response-Renderer weiterhin die richtige Form ausgibt. Reine
nachrichtentragende Varianten (`Internal`, `Database`, `Domain`)
werden zu einer `Domain` mit vorangestellter Nachricht abgeflacht.

## `AppError` - Ad-hoc-Domain-Fehler

Für einmalige Fehler, für die Sie keinen eigenen Typ definieren
möchten, verwenden Sie `AppError`. Es implementiert `HttpError` und
hat eine `From`-Implementierung nach `FrameworkError`, sodass `?`
direkt funktioniert:

```rust
use suprnova::{AppError, Request, Response, json_response};

pub async fn transfer(req: Request) -> Response {
    let amount: i64 = req.param("amount")?.parse()
        .map_err(|_| AppError::bad_request("amount must be a number"))?;

    if amount <= 0 {
        return Err(AppError::unprocessable("amount must be positive").into());
    }

    if amount > 1_000_000 {
        return Err(AppError::forbidden("amount exceeds daily limit").into());
    }

    Ok(json_response!({ "transferred": amount }))
}
```

Die Konstruktoren bilden sich sauber auf Laravels
`abort($status, $msg)`-Form ab:

| `AppError::*` | Status |
|---|---|
| `bad_request(msg)` | 400 |
| `unauthorized(msg)` | 401 |
| `forbidden(msg)` | 403 |
| `not_found(msg)` | 404 |
| `conflict(msg)` | 409 |
| `unprocessable(msg)` | 422 |
| `new(msg)` | 500 |
| `.status(code)` | beliebig |

Beachten Sie: `AppError::unauthorized` ist **401** (fehlende
HTTP-Authentifizierung), während `FrameworkError::Unauthorized`
**403** ist (Autorisierung verweigert, entsprechend Laravels
Policy-Ablehnung). Sie bedeuten unterschiedliche Dinge - wählen Sie
die Variante, die zum tatsächlichen Fehler passt.

## `HttpError` - eigene typisierte Fehler

Wenn derselbe Domain-Fehler an vielen Stellen auftaucht, modellieren
Sie ihn als Typ. Implementieren Sie `HttpError`, und die Konvertierung
liegt bei Ihnen:

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

`HttpError` hat zwei Methoden, beide mit einer Standardimplementierung:

```rust
pub trait HttpError: std::error::Error + Send + Sync + 'static {
    fn status_code(&self) -> u16 { 500 }
    fn error_message(&self) -> String { self.to_string() }
}
```

### Die Brücke zu `?`

Ein naives `impl<T: HttpError> From<T> for FrameworkError` würde mit
der bestehenden `From<AppError>`-Implementierung kollidieren (weil
`AppError` selbst `HttpError` implementiert). Suprnova löst das
Orphan-Rule-Problem stattdessen mit einem expliziten
Brücken-Konstruktor:

```rust
use suprnova::{FrameworkError, HttpError};

pub async fn debit(account: &mut Account, amount: i64) -> Result<(), FrameworkError> {
    account.withdraw(amount)
        .map_err(FrameworkError::from_http_error)?;
    Ok(())
}
```

Statuscode und Nachricht werden aus `HttpError::status_code` und
`HttpError::error_message` übernommen und in einer
`FrameworkError::Domain`-Variante gespeichert. Der Response-Renderer
folgt anschließend dem normalen `Domain`-Pfad.

### `#[domain_error]` für boilerplate-freie Typen

Wenn Sie das Muster typisierter Fehler nutzen möchten, ohne die
`Display`-, `Error`- und `HttpError`-Implementierungen von Hand zu
schreiben, verwenden Sie das Attribut-Makro `#[domain_error]`:

```rust
use suprnova::domain_error;

#[domain_error(status = 404, message = "User not found")]
pub struct UserNotFoundError;

#[domain_error(status = 402, message = "Insufficient funds")]
pub struct InsufficientFundsError {
    pub available: i64,
    pub requested: i64,
}
```

`#[domain_error]` generiert den vollständigen Satz an
Implementierungen *einschließlich* `From<YourError> for
FrameworkError`, sodass `?` direkt funktioniert, ganz ohne
Brücken-Aufruf:

```rust
pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| FrameworkError::param_parse("id", "i64"))?;
    let user = User::find(id).await?
        .ok_or_else(|| FrameworkError::from(UserNotFoundError))?;
    Ok(json_response!({ "user": user }))
}
```

Die drei Stufen der eigenen Fehlerbehandlung - `AppError` für
Inline-Gebrauch, `#[domain_error]` für typisiert-mit-Makro,
handgerollte `HttpError`-Implementierungen für volle Kontrolle - geben
Ihnen auf jeder Formalitätsstufe das richtige Werkzeug.

## `ValidationErrors` - die Error-Bag im Laravel-Format

Wenn eine Anfrage die Validierung nicht besteht, gibt Suprnova
dieselbe JSON-Form aus, die Laravel- und Inertia-Frontends erwarten:

```json
{
    "message": "The given data was invalid.",
    "errors": {
        "email": ["The email field must be a valid email address."],
        "password": ["The password must be at least 8 characters."]
    },
    "request_id": "8f9e1a2b-c3d4-..."
}
```

Normalerweise bauen Sie das nicht von Hand - `#[derive(Validate)]`
auf einem Form-Request und die dahinterliegende `validator`-Crate
erzeugen ein `validator::ValidationErrors`, das Suprnova über
`ValidationErrors::from_validator` konvertiert. Der Typ ist aber
öffentlich, falls Sie ihn brauchen:

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

`add_to_bag` bündelt Fehler unter einer benannten Bag (in der Form von
Laravels `withErrors($errors, 'profile')`), indem der Bag-Name mit
einem `.`-Trenner vorangestellt wird:

```rust
let mut errs = ValidationErrors::new();
errs.add_to_bag("profile", "bio", "must be under 280 characters");
errs.add_to_bag("billing", "card", "expired");
// Fehler-Map: { "profile.bio": [...], "billing.card": [...] }
```

`retain_fields` behält nur die aufgeführten Einträge - intern genutzt
vom `Precognition-Validate-Only`-Header von Precognition, damit der
Server die vollständige Validierung durchführt, aber nur für die vom
Client angefragten Felder Fehler meldet.

## Der Konvertierungsvertrag

Wenn ein `FrameworkError` eine HTTP-Grenze erreicht, durchläuft er
`From<FrameworkError> for HttpResponse`. Dabei geschehen der Reihe
nach drei Dinge:

1. **Status-Routing**. Die `status_code()` der Variante wird einmal
   gelesen.
2. **Logging + Observability**. 5xx löst `tracing::error!` aus und
   dispatcht `ErrorOccurred`; 4xx löst `tracing::warn!` aus. Beide
   tragen die Request-ID, sofern eine im Scope ist.
3. **Body-Rendering**. Ein JSON-Body in Laravels Form, für 5xx
   bereinigt.

### Die Form des Bodys

Alle Fehler-Bodys folgen demselben JSON-Skelett:

```json
{
    "message": "<human readable>",
    "errors": { "field": ["msg", ...] },
    "request_id": "<uuid>" | null,
    "debug_message": "<dev only>"
}
```

- `message` ist immer vorhanden.
- `errors` erscheint nur bei validierungsartigen Fehlern
  (`Validation`, `ValidationError`) - beide rendern dieselbe Form,
  sodass Konsumenten nur einen Pfad parsen müssen.
- `request_id` erscheint immer (`null` außerhalb eines
  Request-Scopes - z. B. beim frühen Boot oder in Tests ohne
  Request-Kontext).
- `debug_message` erscheint nur bei 5xx, wenn `APP_DEBUG=true`
  gesetzt ist. Es ist rein additiv - Produktions-Clients dürfen sich
  nicht darauf verlassen.

### Die 5xx-Bereinigungsregel

Das ist die Sicherheitsgarantie, die man sich merken sollte. Bei
jedem Fehler mit Status ≥ 500 wird die `message` im JSON-Body durch
den wörtlichen String ersetzt:

```json
{ "message": "Internal Server Error", "request_id": "..." }
```

Das rohe Fehlerdetail gelangt **nicht** in den Response-Body. Es geht
an:

- den `tracing::error!`-Logeintrag, mit Request-ID und Status
- das `ErrorOccurred`-Event, das jeder Listener abgreifen kann

Wenn `APP_DEBUG=true` gesetzt ist (außerhalb von
`local`/`dev`/`test` standardmäßig false), trägt die Response
zusätzlich ein `debug_message`-Feld mit dem rohen Detail - aber
`message` bleibt in beiden Modi generisch, sodass Frontends und
Clients sich nicht versehentlich an Dev-only-Daten koppeln können.

Das ist der Vertrag, der es Ihnen erlaubt,
`FrameworkError::internal("db connection refused: password mismatch
on user 'app_rw'")` aufzurufen, ohne dass das Passwort in der
Antwort an den Client landet. Die `message`, die Sie übergeben, ist
für Betreiber, die Logs lesen; die `message`, die der Client sieht,
ist `"Internal Server Error"`.

Bei 4xx-Fehlern bleibt die für den Aufrufer sichtbare Nachricht
erhalten - `404 User not found`, `400 Missing required parameter:
user_id`. Das sind Domain-Fehler, auf die der Client reagieren muss,
keine internen Fehler.

### Wo der Vertrag lebt

Die gesamte Konvertierung ist eine einzige Funktion - `impl
From<FrameworkError> for HttpResponse` in
`framework/src/http/response.rs`. Lesen Sie sie einmal, und Sie haben
die gesamte Fehler-Rendering-Oberfläche von Suprnova gesehen. Es gibt
keinen anderen Pfad.

## Die Panic-Grenze

Ein Panic in einer Middleware oder einem Handler würde sich sonst die
Per-Connection-Task hinauf fortpflanzen und den hyper-Service mitten
in der Response abreißen - der Client bekäme einen TCP-Reset und
keine HTTP-Response. Suprnova fängt ihn ab.

`execute_chain_safely` in `framework/src/server.rs` umschließt die
Middleware-Chain mit `AssertUnwindSafe(...).catch_unwind().await`.
Bei einem Panic geschieht Folgendes:

1. Extrahiert die Panic-Payload (behandelt `&'static str`- und
   `String`-Payloads; alles andere erscheint als `"panic with
   non-string payload"`).
2. Protokolliert `tracing::error!` mit Request-Methode, -Pfad und -ID.
3. Konstruiert `FrameworkError::internal(format!("request handler
   panicked: {msg}"))` und leitet ihn durch dieselbe
   `From<FrameworkError> for HttpResponse`-Konvertierung, die auch
   jeder andere 5xx-Fehler durchläuft.
4. Gibt die Request-ID als `X-Request-Id` zurück.

Die Panic-Payload bleibt im Logeintrag; der Client bekommt den
bereinigten Body `{"message": "Internal Server Error"}`.
Observability-Listener, die bei `ErrorOccurred` für zurückgegebene
5xx-Fehler feuern, feuern auch bei Panics - es gibt keine separate
Panic-Event-Oberfläche, die Sie extra verdrahten müssten.

Dasselbe Panic-Recovery-Muster verwenden auch:

- WebSocket-Handler (`framework/src/server.rs`)
- Geplante Tasks (`framework/src/schedule/mod.rs`)
- Workflows (`framework/src/workflow/mod.rs`)
- Der `Supervisor`-Trait (Broadcasting)

Ein Panic in einem dieser Subsysteme wird protokolliert und entweder
in einen Fehlerzustand übersetzt oder automatisch neu gestartet; er
reißt die Worker-Task nicht mit sich.

## Observability über `ErrorOccurred` einhängen

`ErrorOccurred` ist ein eingebautes Event, das das Framework bei
jeder 5xx-Response dispatcht (einschließlich der aus Panics
synthetisierten):

```rust
pub struct ErrorOccurred {
    pub error_message: String,
    pub status_code: u16,
    pub request_id: Option<String>,
}
```

Lauschen Sie darauf genauso, wie Sie auf jedes andere Event lauschen:

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
EventFacade::listen::<ErrorOccurred, _>(Arc::new(SentryReporter)).await;
```

Das ist das Suprnova-Äquivalent zu Laravels `report()`-Callback am
globalen Exception-Handler. Das Event kommt mit der ursprünglichen,
unbereinigten `error_message` an (der Body, den der Client sieht,
bleibt weiterhin bereinigt), dem Statuscode und der korrelierbaren
Request-ID.

## Abort-Hilfsfunktionen

Drei freie Funktionen brechen einen Handler bei einem bestimmten
Status sofort ab. Sie spiegeln Laravels `abort` / `abort_if` /
`abort_unless`:

```rust
use suprnova::{abort_with, abort_if, abort_unless, Auth, Request, Response, json_response};

pub async fn show(req: Request) -> Response {
    abort_unless(Auth::check(), 401, "must be logged in")?;
    abort_if(req.param("id")? == "0", 404, "User not found")?;
    abort_with(503, "scheduled maintenance")?;
    Ok(json_response!({ "ok": true }))
}
```

Jede gibt `Result<(), FrameworkError>` zurück. Verwenden Sie sie mit
`?`. Der zugrunde liegende Fehler ist `FrameworkError::Domain {
message, status_code }`, sodass er über dieselbe Body-Form und
dieselben Bereinigungsregeln gerendert wird wie jeder andere Fehler.
Statuscodes außerhalb des gültigen Bereichs werden von der
Statusvalidierung des Response-Renderers auf 500 erzwungen - Sie
müssen sich an der Aufrufstelle nicht gegen ungültige Eingaben
absichern.

## Das CLI-Sentinel: `AlreadyReported`

Eine Variante von `FrameworkError` hat keine HTTP-Bedeutung.
`AlreadyReported` wird über `FrameworkError::silent()` konstruiert und
vom Console-Dispatcher verwendet, wenn clap bereits seinen eigenen
Argument-Parse-Fehler formatiert und ausgegeben hat. Das `main` der
Binary übersetzt das Sentinel in einen Exit-Code ungleich null, ohne
`eprintln` - Nutzer sehen so nie zwei Fehlermeldungen für denselben
Fehler.

Sollte `AlreadyReported` jemals einen HTTP-Response-Konverter
erreichen, zeigt das an, dass ein Request-Handler versehentlich
`silent()` zurückgegeben hat. Der Konverter protokolliert sichtbar ein
`tracing::error!`, das die Quelle des Lecks identifiziert, und gibt
einen generischen 500 zurück - die Variante hat im Request-Pfad nichts
zu suchen, und das Log macht den Bug sichtbar statt still.

Normalerweise bekommen Sie diese Variante nicht zu Gesicht; sie wird
hier dokumentiert, weil das Enum `HTTP-flavoured` ist und die sonst
unerklärte Variante jeden verwirren würde, der den Quellcode liest.

## Sicherheitsgarantien im Überblick

Der Vertrag, den Suprnova Ihnen gibt:

- **Totale Konvertierung**. Jeder `FrameworkError` erzeugt eine
  `HttpResponse`. Es gibt keinen Fehlerpfad, der den Server abstürzen
  lässt oder die Verbindung stillschweigend fallen lässt.
- **Bereinigte 5xx**. Der Response-Body für jeden 5xx-Fehler ist der
  generische `{"message": "Internal Server Error", "request_id":
  "..."}`. Details fließen in die Logs + `ErrorOccurred`.
- **Optionale Debug-Sichtbarkeit**. `APP_DEBUG=true` fügt bei 5xx
  ein `debug_message`-Feld hinzu, niemals `message`.
  Produktions-Clients können sich nicht versehentlich an
  Dev-only-Daten koppeln.
- **Korrelierbare Request-IDs**. Jeder Fehler-Body trägt die
  Request-ID (oder `null`, wenn kein Request-Scope existiert);
  dieselbe ID erscheint in der Log-Zeile und im
  `ErrorOccurred`-Event.
- **Panic-Recovery**. Panics in Handlern und Middleware werden
  abgefangen, protokolliert und über dieselbe `From`-Implementierung
  geroutet wie zurückgegebene Fehler. Kein Verbindungsabbruch, keine
  Observability-Lücke.
- **Eine Form für alles**. Validierungsfehler, Parameterfehler,
  Panics, eigene Domain-Fehler und Storage-Fehler fallen alle auf
  dasselbe JSON-Skelett zusammen. Frontend-Code parst nur eine
  Struktur.

## Wo jedes Teil lebt

| Teil | Datei |
|---|---|
| `FrameworkError`, `AppError`, `HttpError`, `ValidationErrors` | `framework/src/error.rs` |
| `From<FrameworkError> for HttpResponse` (Konvertierung + Bereinigung) | `framework/src/http/response.rs` |
| `abort`, `abort_if`, `abort_unless` | `framework/src/http/abort.rs` |
| `execute_chain_safely` (Panic-Grenze) | `framework/src/server.rs` |
| `ErrorOccurred`-Event | `framework/src/events/builtins.rs` |
| `#[domain_error]`-Makro | `suprnova-macros/src/domain_error.rs` |

## Nächste Schritte

- [Fehlerbehandlung](errors.md) - die praktischen Handler-Muster, die
  dieses Modell nutzen
- [Request-Lifecycle](lifecycle.md) - wo im Request-Flow die
  Fehlerkonvertierung läuft
- [Validierung](validation.md) - `#[derive(Validate)]`, Form-Requests
  und wie `ValidationErrors` befüllt wird
- [Antworten](responses.md) - `HttpResponse`-Builder, Header, Cookies,
  Streaming
- [Ereignisse](events.md) - wie Sie auf `ErrorOccurred` und andere
  eingebaute Events lauschen
