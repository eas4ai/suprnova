# Anfragen

Suprnova-Handler bekommen einen `Request` - die HTTP-Anfrage auf
Wire-Ebene - oder eine typisierte Form-Request-Struktur, die den Body
parst, validiert und autorisiert, bevor Ihr Code läuft. Beide Wege
hängen am selben `#[handler]`-Makro; Sie wählen die Form pro Route.
Dieses Kapitel behandelt beide, dazu den Extraktor für
Multipart-Uploads und die rohen Accessoren, zu denen Sie in Middleware
greifen.

## Typisierte Form-Requests

Das `#[request]`-Attribut markiert eine Struktur als `FormRequest`. Das
Makro ergänzt die Derives `serde::Deserialize` und
`validator::Validate` und gibt ein `impl FormRequest` aus, damit das
`#[handler]`-Makro weiß, dass es die Struktur auf dem Weg herein
extrahieren und validieren soll:

```rust
use suprnova::request;

#[request]
pub struct CreateUserRequest {
    #[validate(email(message = "Please provide a valid email address"))]
    pub email: String,

    #[validate(length(min = 8, message = "Password must be at least 8 characters"))]
    pub password: String,

    #[validate(length(min = 1, max = 100, message = "Name is required"))]
    pub name: String,
}
```

Ein Handler, der diesen Typ als Parameter benennt, bekommt einen
bereits validierten Wert gereicht:

```rust
use suprnova::{handler, json_response, Response};
use crate::requests::CreateUserRequest;

#[handler]
pub async fn store(form: CreateUserRequest) -> Response {
    // `form` ist validiert - dieser Code läuft nur, wenn jede Regel bestanden hat.
    json_response!({ "email": form.email, "name": form.name })
}
```

Ein Handler, der stattdessen `Request` benennt, bekommt die rohe
Anfrage unverändert durchgereicht:

```rust
use suprnova::{handler, json_response, Request, Response};

#[handler]
pub async fn index(req: Request) -> Response {
    json_response!({ "path": req.path() })
}
```

Beides sind Extraktoren - das `#[handler]`-Makro schlägt für jeden
Parametertyp `FromRequest::from_request` nach, und jede Struktur, die
`FormRequest` implementiert, bekommt eine Blanket-Impl von
`FromRequest` gratis dazu.

## Validierungsregeln

Die Validierung läuft über die `validator`-Crate. Gängige Regeln:

### String-Validierungen

```rust
#[request]
pub struct ExampleRequest {
    // Pflichtfeld (nicht leer)
    #[validate(length(min = 1, message = "This field is required"))]
    pub name: String,

    // E-Mail-Format
    #[validate(email(message = "Invalid email address"))]
    pub email: String,

    // URL-Format
    #[validate(url(message = "Invalid URL"))]
    pub website: String,

    // Längenbeschränkungen
    #[validate(length(min = 8, max = 100))]
    pub password: String,

    // Regex-Pattern - PHONE_REGEX muss ein `static` oder `const` sein,
    // das von der Expansionsstelle des Validators aus sichtbar ist.
    // Deklarieren Sie es einmal, typischerweise im selben Modul:
    #[validate(regex(path = "PHONE_REGEX", message = "Invalid phone number"))]
    pub phone: String,
}

use std::sync::LazyLock;
use regex::Regex;

// validator 0.20 implementiert `AsRegex` für `std::sync::LazyLock<Regex>`,
// aber nicht für `once_cell::sync::Lazy<Regex>` - verwenden Sie den
// std-Typ, damit die Expansion von `#[validate(regex(path = "..."))]`
// im Derive die Typprüfung besteht.
static PHONE_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\+?[0-9\s\-()]{7,20}$").unwrap());
```

### Numerische Validierungen

```rust
#[request]
pub struct ProductRequest {
    // Bereichsvalidierung - die Literale müssen zum Feldtyp passen. `f64`
    // nimmt `0.0` / `10000.0`, nicht die Integer-Literale `0` / `10000`.
    #[validate(range(min = 0.0, max = 10000.0, message = "Price must be between 0 and 10000"))]
    pub price: f64,

    // Mindestwert
    #[validate(range(min = 1))]
    pub quantity: i32,

    // Höchstwert
    #[validate(range(max = 100))]
    pub discount_percent: i32,
}
```

### Verschachtelte und Collection-Validierungen

```rust
use serde::Deserialize;

#[derive(Deserialize, Validate)]
pub struct Address {
    #[validate(length(min = 1))]
    pub street: String,

    #[validate(length(min = 1))]
    pub city: String,
}

#[request]
pub struct OrderRequest {
    // Validierung einer verschachtelten Struktur
    #[validate(nested)]
    pub shipping_address: Address,

    // Länge einer Collection
    #[validate(length(min = 1, message = "At least one item required"))]
    pub items: Vec<String>,
}
```

### Gängige Validierungs-Attribute

| Attribut | Beschreibung | Beispiel |
|-----------|-------------|---------|
| `email` | Gültiges E-Mail-Format | `#[validate(email)]` |
| `url` | Gültiges URL-Format | `#[validate(url)]` |
| `length` | Länge von String oder Collection | `#[validate(length(min = 1, max = 100))]` |
| `range` | Numerischer Bereich | `#[validate(range(min = 0, max = 100))]` |
| `regex` | Treffer auf ein Regex-Pattern | `#[validate(regex(path = "PATTERN"))]` |
| `contains` | String enthält einen Teilstring | `#[validate(contains(pattern = "@"))]` |
| `does_not_contain` | String enthält etwas nicht | `#[validate(does_not_contain(pattern = "admin"))]` |
| `nested` | Verschachtelte Struktur validieren | `#[validate(nested)]` |

## Responses bei Validierungsfehlern

Wenn die Validierung fehlschlägt, gibt Suprnova eine 422-Response mit
der Laravel-/Inertia-kompatiblen Fehler-Bag zurück:

```json
HTTP 422 Unprocessable Entity

{
    "message": "The given data was invalid.",
    "errors": {
        "email": ["Please provide a valid email address"],
        "password": ["Password must be at least 8 characters"]
    }
}
```

Die Form von `errors` entspricht genau dem, was `@inertiajs/*`-Clients
direkt aus `usePage().props.errors` lesen.

### Verschachtelte Felder

Ein Fehlschlag von `#[validate(nested)]` wird unter einem
Punkt-Schlüssel gemeldet, der den vollständigen Pfad benennt - dieselbe
Notation, die Laravel verwendet. Eine verschachtelte Struktur steuert
`parent.field` bei; ein Element eines validierten `Vec<T>` steuert
`parent.<index>.field` bei:

```json
{
    "message": "The given data was invalid.",
    "errors": {
        "shipping_address.street": ["Validation failed for field 'shipping_address.street'"],
        "items.1.name": ["Validation failed for field 'items.1.name'"]
    }
}
```

Index `1` ist das zweite Element - das erste hat bestanden und fehlt in
der Bag. Binden Sie den Schlüssel auf dem Client direkt durch:
`form.errors['items.1.name']`.

## Vollständiges Beispiel

Ein Endpunkt zur Benutzerregistrierung, von Anfang bis Ende.

**Den Request definieren:**

```rust
// src/requests/create_user.rs
use suprnova::request;

#[request]
pub struct CreateUserRequest {
    #[validate(email(message = "Please provide a valid email address"))]
    pub email: String,

    #[validate(length(min = 8, message = "Password must be at least 8 characters"))]
    pub password: String,

    #[validate(length(min = 2, max = 50, message = "Name must be between 2 and 50 characters"))]
    pub name: String,
}
```

**Den Controller anlegen:**

```rust
// src/controllers/user.rs
use suprnova::{handler, json_response, Request, Response, ResponseExt};
use crate::requests::CreateUserRequest;

#[handler]
pub async fn index(_req: Request) -> Response {
    json_response!({ "users": [] })
}

#[handler]
pub async fn store(form: CreateUserRequest) -> Response {
    // Validierung bestanden - den Benutzer anlegen
    // In einer echten App würden Sie hier in die Datenbank schreiben

    json_response!({
        "user": {
            "email": form.email,
            "name": form.name
        },
        "message": "User created successfully"
    })
    .status(201)
}
```

**Die Routen registrieren:**

```rust
// src/routes.rs
use suprnova::{get, post, routes};
use crate::controllers;

routes! {
    get!("/users", controllers::user::index).name("users.index"),
    post!("/users", controllers::user::store).name("users.store"),
}
```

## Autorisierung und feldübergreifende Hooks

Der `FormRequest`-Trait bietet drei Lifecycle-Hooks: `authorize`,
`after_validation` und `after_validation_async`. Sowohl das
`#[request]`-Attribut als auch die
`#[derive(FormRequestDerive)]`-Form geben eine Standard-`impl
FormRequest` für Sie aus. Um einen Hook zu überschreiben, fügen Sie das
Opt-out `#[form_request(custom_hooks)]` hinzu, das die Standard-Impl
unterdrückt, und schreiben dann Ihre eigene. (Das entspricht dem
`#[multipart(custom_hooks)]`-Muster.)

```rust
use suprnova::{FormRequest, FormRequestDerive, Request};
use serde::Deserialize;
use validator::Validate;

#[derive(Deserialize, Validate, FormRequestDerive)]
#[form_request(custom_hooks)]
pub struct DeleteUserRequest {
    pub user_id: i64,
}

impl FormRequest for DeleteUserRequest {
    fn authorize(req: &Request) -> bool {
        // `false` zurückgeben, um vor dem Lesen des Bodys mit
        // 403 Forbidden kurzzuschließen.
        req.header("X-Admin-Token").is_some()
    }
}
```

Das Opt-out funktioniert auch unter der `#[request]`-Attributform -
nützlich, wenn Sie die Auto-Derives des Attributs behalten, aber Hooks
überschreiben möchten:

```rust
use suprnova::{FormRequest, Request, request};

#[request]
#[form_request(custom_hooks)]
pub struct DeleteUserRequestAttr {
    pub user_id: i64,
}

impl FormRequest for DeleteUserRequestAttr {
    fn authorize(req: &Request) -> bool {
        req.header("X-Admin-Token").is_some()
    }
}
```

Gibt `authorize` `false` zurück, liefert die Extraktion
`FrameworkError::Unauthorized` und rendert:

```json
HTTP 403 Forbidden

{ "message": "This action is unauthorized." }
```

`after_validation` ist der synchrone feldübergreifende Hook -
verwenden Sie ihn für Regeln wie "Passwort und Bestätigung müssen
übereinstimmen". `after_validation_async` ist das asynchrone
Gegenstück und die Stelle, an der datenbankgestützte Regeln (z. B. das
eingebaute `Unique`) an der automatischen Validierung teilnehmen. Beide
feuern, nachdem die feldweisen `validator`-Regeln bestanden sind;
`extract` steigt bei der ersten fehlschlagenden Stufe aus.

```rust
use suprnova::{FormRequest, FormRequestDerive, ValidationErrors};
use serde::Deserialize;
use validator::Validate;

#[derive(Deserialize, Validate, FormRequestDerive)]
#[form_request(custom_hooks)]
pub struct UpdatePasswordRequest {
    #[validate(length(min = 8))]
    pub new_password: String,
    pub confirmation: String,
}

impl FormRequest for UpdatePasswordRequest {
    fn after_validation(&self) -> Result<(), ValidationErrors> {
        if self.new_password != self.confirmation {
            let mut errs = ValidationErrors::new();
            errs.add("confirmation", "passwords do not match");
            return Err(errs);
        }
        Ok(())
    }
}
```

### Obergrenzen für die Body-Größe

Das Attribut `#[form_request(max_body_bytes = N)]` pro Struktur
überschreibt die prozessweite Obergrenze von 8 MiB für einen einzelnen
FormRequest:

```rust
use suprnova::FormRequestDerive;
use serde::Deserialize;
use validator::Validate;

#[derive(Deserialize, Validate, FormRequestDerive)]
#[form_request(max_body_bytes = 64 * 1024 * 1024)] // 64 MiB
pub struct ImportPayload {
    pub rows: Vec<Row>,
}

#[derive(Deserialize, Validate)]
pub struct Row { /* ... */ }
```

`Content-Length` wird vorab geparst, und die Anfrage wird mit HTTP 413
abgewiesen, *bevor* ein einziges Body-Byte gelesen wird, wenn die
angegebene Größe die Obergrenze übersteigt; Clients, die bei
`Content-Length` lügen, laufen beim Lesen trotzdem in den Byte-Zähler
des Streamings.

## Erkennung des Content-Type

`FormRequest::extract` schaut nur auf den `Content-Type`-Header:

- `application/x-www-form-urlencoded` → geparst über `serde_urlencoded`
- `application/json` oder jedes Suffix der Art `application/*+json` → geparst über `serde_json`
- Alles andere (auch ein fehlender Header) → abgewiesen mit HTTP 415
  Unsupported Media Type, bevor der Body gelesen wird

Für Multipart-Bodys (`multipart/form-data`) siehe weiter unten
[Datei-Uploads](#datei-uploads-multipartrequest).

## Den Body direkt lesen

Für einmalige Endpunkte oder für Middleware, die keinen vollständigen
`FormRequest` will, liest der `Request`-Typ den Body selbst in drei
Varianten - jede verbraucht `self`, weil der Body höchstens einmal
gelesen werden kann:

```rust
use serde::Deserialize;
use suprnova::{handler, json_response, Request, Response};

#[derive(Deserialize)]
struct LoginForm { username: String, password: String }

#[handler]
pub async fn login(req: Request) -> Response {
    // Den Parser explizit wählen.
    let form: LoginForm = req.form().await?;
    json_response!({ "user": form.username })
}

#[handler]
pub async fn webhook(req: Request) -> Response {
    // Gleiche Form, JSON als Wire-Format.
    let payload: serde_json::Value = req.json().await?;
    json_response!({ "received": payload })
}

#[handler]
pub async fn ingest(req: Request) -> Response {
    // Automatische Wahl anhand des Content-Type - JSON, sofern nicht
    // `application/x-www-form-urlencoded` explizit angegeben ist.
    let value: serde_json::Value = req.input().await?;
    json_response!({ "value": value })
}
```

Für den rohen Zugriff liefert `req.body_bytes().await` die gepufferten
`Bytes` plus die `RequestParts`-Metadaten (Routenparameter und
Content-Type). Verwenden Sie `body_bytes_with_cap(n)`, um die globale
Obergrenze von 8 MiB im Einzelfall zu überschreiben.

## Services neben dem Formular auflösen

Validierte Form-Requests komponieren sich mit dem
[Service Container](container.md). Verwenden Sie `App::resolve::<T>()`
(oder `App::get::<T>()`) innerhalb des Handlers:

```rust
use suprnova::{handler, json_response, Response, App};
use crate::requests::CreateUserRequest;
use crate::services::UserService;

#[handler]
pub async fn store(form: CreateUserRequest) -> Response {
    let user_service = App::resolve::<UserService>()?;
    let user = user_service.create_user(&form.email, &form.name).await?;
    json_response!({ "user": user })
}
```

## Datei-Uploads (`MultipartRequest`)

`multipart/form-data` hat einen eigenen Extraktor -
`#[derive(MultipartRequest)]` streamt den Body Teil für Teil und lagert
große Dateiteile oberhalb der konfigurierten Schwelle in eine temporäre
Datei aus, sodass ein 200-MiB-Upload nie vollständig im RAM liegt. Jedes
Feld trägt eine `#[field("name")]`-Annotation, die das Feld auf der
Leitung benennt; Dateifelder verwenden `UploadedFile<V>`, wobei `V` ein
Validator (oder ein Tupel aus Validatoren) aus
`suprnova::http::upload::validators` ist.

```rust
use suprnova::{handler, json_response, MultipartRequest, Response};
use suprnova::http::upload::UploadedFile;
use suprnova::http::upload::validators::{ImageFile, MaxSize};

#[derive(MultipartRequest)]
pub struct AvatarUpload {
    #[field("avatar")]
    pub avatar: UploadedFile<(ImageFile, MaxSize<5_242_880>)>, // Obergrenze 5 MiB
    #[field("caption")]
    pub caption: Option<String>,
}

#[handler]
pub async fn upload_avatar(form: AvatarUpload) -> Response {
    // `avatar` liegt je nach Größe im Speicher oder in einer temporären Datei.
    // `.bytes()` liest beides; `.store_as(...)` streamt auf eine Disk.
    let bytes = form.avatar.bytes().await?;
    json_response!({ "size": bytes.len(), "caption": form.caption })
}
```

Feldformen:

| Deklaration | Form auf der Leitung |
|---|---|
| `UploadedFile<V>` | erforderliche Datei |
| `Option<UploadedFile<V>>` | optionale Datei |
| `Vec<UploadedFile<V>>` | Array-Uploads (`photos[]`) |
| `String` / `u32` / jedes `FromStr` | Textfeld (erforderlich) |
| `Option<String>` / `Option<T: FromStr>` | optionales Textfeld |
| `Vec<String>` / `Vec<T: FromStr>` | wiederholte Textfelder |

Eingebaute Validatoren in `suprnova::http::upload::validators`:

- `MaxSize<N>` - bricht an der Byte-Grenze ab, sobald die laufende Summe
  `N` Bytes überschreitet (HTTP 413).
- `ImageFile` - weist Teile zurück, deren Magic Bytes kein `image/*`
  behaupten. (Benannt nach Laravels eigener Regel; der schlichte Name
  `Image` gehört der Bildmanipulations-Pipeline - siehe
  [Bilder](images.md).)
- `MimeType<L>` - akzeptiert eine feste Allowlist, die Ihr eigener
  `MimeAllowlist`-Typ bereitstellt.
- `()` - tut nichts; `UploadedFile<()>` akzeptiert beliebige Bytes.

Validatoren lassen sich als Tupel kombinieren:
`(ImageFile, MaxSize<5_242_880>)` führt beide aus und bricht beim ersten
Fehlschlag ab.

### Obergrenzen pro Feld und Array-Schranken

Die Byte-Obergrenze für den gesamten Body ist global (standardmäßig 8 MiB
für Multipart, konfigurierbar über
`suprnova::http::upload::set_global_max_multipart_body_bytes`).
Obergrenzen pro Feld verhindern den Missbrauch, bei dem ein Body aus
vielen kleinen Teilen `Vec<UploadedFile<_>>` innerhalb des Byte-Budgets
unbegrenzt wachsen lässt:

```rust
#[derive(MultipartRequest)]
pub struct Gallery {
    #[field("photos", max_count = 8)]
    pub photos: Vec<UploadedFile<MaxSize<1_048_576>>>,
}
```

Der (`max_count` + 1)-te Teil mit diesem Namen liefert HTTP 422, bevor
etwas alloziert wird, der überzählige Teil erreicht das Wachstum des
`Vec` also nie.

### Autorisierungs- und Nachvalidierungs-Hooks

`MultipartRequest` spiegelt die Hooks von `FormRequest` über den Trait
`MultipartRequestHooks`. Standardmäßig gibt das Derive eine leere
Implementierung aus; mit `#[multipart(custom_hooks)]` melden Sie sich für
Ihre eigene an:

```rust
use suprnova::{MultipartRequest, Request, ValidationErrors};
use suprnova::http::upload::{MultipartRequestHooks, UploadedFile};

#[derive(MultipartRequest)]
#[multipart(custom_hooks)]
pub struct GuardedUpload {
    #[field("file")]
    pub file: UploadedFile,
}

impl MultipartRequestHooks for GuardedUpload {
    fn authorize(req: &Request) -> bool {
        req.header("X-Admin-Token").is_some()
    }

    fn after_validation(&self) -> Result<(), ValidationErrors> {
        if self.file.size == 0 {
            let mut errs = ValidationErrors::new();
            errs.add("file", "empty file");
            return Err(errs);
        }
        Ok(())
    }
}
```

### In den Speicher streamen

`UploadedFile::store_as` schreibt den Teil auf eine registrierte
Speicher-Disk. Für Disk-gestützte Teile ist der Pfad vollständig
streamend (64-KiB-Blöcke über `opendal::Operator::writer`); Teile im
Speicher nutzen einen einzelnen Schreibaufruf. Nehmen Sie die aus dem
Inhalt abgeleitete Endung, wenn der Speicherpfad inhaltsadressiert ist -
der Dateiname aus dem Header ist nicht vertrauenswürdig:

```rust
use suprnova::Storage;

let disk = Storage::disk("avatars")?;
let path = format!("{}.{}", user.id, form.avatar.extension_from_magic());
form.avatar.store_as(&disk, &path).await?;
```

Die Registry der Speicher-Disks steht unter
[Dateisystem](filesystem.md).

## Dateiorganisation

Die Standardstruktur für Requests:

```
src/
├── requests/
│   ├── mod.rs                 # Re-exportiert alle Requests
│   ├── create_user.rs         # CreateUserRequest
│   ├── update_user.rs         # UpdateUserRequest
│   └── create_post.rs         # CreatePostRequest
├── controllers/
│   └── user.rs                # Verwendet CreateUserRequest
└── routes.rs
```

**src/requests/mod.rs:**
```rust
pub mod create_user;
pub mod update_user;

pub use create_user::CreateUserRequest;
pub use update_user::UpdateUserRequest;
```

## End-to-End-Typsicherheit mit Inertia

Requests können zusätzlich `InertiaProps` ableiten, um TypeScript-Typen zu generieren, was End-to-End-Typsicherheit von Ihrem Rust-Backend bis zu Ihrem React-Frontend ermöglicht.

### TypeScript-Typen für Requests generieren

Ergänzen Sie das `InertiaProps`-Derive neben `#[request]`:

```rust
use suprnova::{request, InertiaProps};

#[request]
#[derive(InertiaProps)]
pub struct CreateTodoRequest {
    #[validate(length(min = 1, message = "Title is required"))]
    pub title: String,

    #[validate(length(max = 500))]
    pub description: Option<String>,
}
```

Führen Sie die Typgenerierung aus:

```bash
suprnova generate-types
```

Das erzeugt TypeScript-Typen in `frontend/src/types/inertia-props.ts`:

```typescript
export interface CreateTodoRequest {
  title: string
  description: string | null
}
```

### Typsichere Formulare mit Inertia

Verwenden Sie Inertias `<Form>`-Komponente für die sauberste
Formularbehandlung:

```tsx
import { Form, usePage } from '@inertiajs/react'

export default function CreateTodo() {
  const { errors } = usePage().props

  return (
    <Form action="/todos" method="post">
      <input
        type="text"
        name="title"
        placeholder="Todo title"
      />
      {errors?.title && <span className="error">{errors.title}</span>}

      <textarea
        name="description"
        placeholder="Description (optional)"
      />

      <button type="submit">Create Todo</button>
    </Form>
  )
}
```

Für mehr Kontrolle kombinieren Sie `<Form>` mit dem `useForm`-Hook und Ihren generierten Typen:

```tsx
import { Form, useForm } from '@inertiajs/react'
import type { CreateTodoRequest } from '../types/inertia-props'

export default function CreateTodo() {
  const { data, setData, errors, processing } = useForm<CreateTodoRequest>({
    title: '',
    description: null,
  })

  return (
    <Form action="/todos" method="post">
      {({ processing }) => (
        <>
          <input
            type="text"
            name="title"
            value={data.title}
            onChange={(e) => setData('title', e.target.value)}
            placeholder="Todo title"
          />
          {errors.title && <span className="error">{errors.title}</span>}

          <textarea
            name="description"
            value={data.description || ''}
            onChange={(e) => setData('description', e.target.value || null)}
            placeholder="Description (optional)"
          />

          <button type="submit" disabled={processing}>
            Create Todo
          </button>
        </>
      )}
    </Form>
  )
}
```

### Was das Derive Ihnen einbringt

- TypeScript fängt Tippfehler in Feldnamen und Typabweichungen zur
  Compile-Zeit ab.
- Die Autovervollständigung der IDE liest die generierte `.ts` direkt.
- Benennen Sie ein Feld in Rust um, führen Sie `suprnova
  generate-types` erneut aus, und die TypeScript-Oberfläche zieht nach.

Siehe [TypeScript Types](frontend-typescript-types.md) für die
vollständige Generierungs-Pipeline.

## Request-Accessoren

Über das oben gezeigte Muster des validierten Formulars hinaus trägt der `Request`-Typ Accessoren im Laravel-Stil, um die Anfrage auf Wire-Ebene zu inspizieren - URL, Header, Query-String, Content Negotiation, Routen-Metadaten und Client-IP. Sie sind nützlich in Middleware, in Handlern, die neben einem `FormRequest` rohen Zugriff wollen, und überall dort, wo validiertes Parsen nicht das richtige Werkzeug ist.

### URL und Pfad

| Methode | Liefert | Anmerkungen |
|--------|---------|-------|
| `req.path()` | `&str` | Roher URI-Pfad. |
| `req.decoded_path()` | `String` | Pfad mit aufgelösten Prozent-Escapes. |
| `req.segments()` | `Vec<String>` | Pfad an `/` zerlegt, leere Segmente entfallen. |
| `req.segment(index, default)` | `Option<String>` | Segmentzugriff, 1-basiert. |
| `req.url()` | `String` | Schema + Host + Pfad (ohne Query-String). |
| `req.full_url()` | `String` | URL + Query-String. |
| `req.full_url_with_query(&[("k","v")])` | `String` | Query-Schlüssel anhängen oder überschreiben. |
| `req.full_url_without_query(&["k"])` | `String` | Query-Schlüssel entfernen. |

```rust
use suprnova::{handler, json_response, Request, Response};

#[handler]
pub async fn show(req: Request) -> Response {
    if req.is(&["admin/*"]) {
        // der Pfad matcht die Wildcard admin/*
    }
    json_response!({ "url": req.full_url() })
}
```

### Host, Schema, IP

| Methode | Liefert | Reihenfolge der Quellen |
|--------|---------|--------------|
| `req.host()` | `Option<String>` | `X-Forwarded-Host` → `Host` → URI-Authority. |
| `req.http_host()` | `Option<String>` | Host plus Port, wenn dieser nicht der Standard ist. |
| `req.scheme_and_http_host()` | `Option<String>` | `scheme://host:port`. |
| `req.scheme()` | `&'static str` | `"https"`, wenn [`secure`] wahr ist, sonst `"http"`. |
| `req.secure()` | `bool` | URI-Schema → `X-Forwarded-Proto` → `X-Forwarded-Ssl: on`. |
| `req.ip()` | `Option<String>` | `X-Forwarded-For[0]` → `X-Real-IP` → Adresse der Gegenstelle. |
| `req.ips()` | `Vec<String>` | Vollständige Kette: Proxy-Header, dann Adresse der Gegenstelle. |
| `req.user_agent()` | `Option<&str>` | `User-Agent`-Header. |
| `req.port()` | `Option<u16>` | Port aus dem Host-Header → `X-Forwarded-Port` → Port aus der URI. |

### Header und Methode

| Methode | Liefert |
|--------|---------|
| `req.has_header("X-Foo")` | `bool` |
| `req.bearer_token()` | `Option<String>` (letzter `Bearer `-Teilstring, Kommas abgeschnitten) |
| `req.is_method("POST")` | `bool` (ohne Unterscheidung von Groß- und Kleinschreibung) |
| `req.ajax()` | `X-Requested-With: XMLHttpRequest` |
| `req.pjax()` | `X-PJAX`-Header mit wahrem Wert |
| `req.prefetch()` | `X-Moz`, `Purpose` oder `Sec-Purpose` = `prefetch` |

### Content Negotiation

```rust
if req.is_json() { /* Content-Type führt /json oder +json */ }
if req.expects_json() { /* AJAX ohne Einengung per Accept, oder Accept bevorzugt JSON */ }
if req.wants_json() { /* Accept-Header nennt JSON an erster Stelle */ }
if req.accepts_html() { /* Accept lässt text/html zu */ }

let preferred = req.prefers(&["application/json", "text/html"]);
let acceptable = req.acceptable_content_types();
```

`accepts(&[ty])` matcht sowohl blanke Typen als auch Suffixe der Art `application/<vendor>+json`. `accepts_any_content_type()` liefert true, wenn kein Accept-Header vorhanden ist oder die erste Präferenz `*/*` lautet.

### Query-String

```rust
let id: Option<String> = req.query_param("id");
let present: bool = req.has_query("id");
let map = req.query_params(); // HashMap<String, String>

// Typisiertes Parsen der Query über serde
#[derive(serde::Deserialize)]
struct SearchQuery { page: u32, q: String }
let q: SearchQuery = req.query_into()?;
```

### Routen-Metadaten

Nachdem der Router eine Anfrage dispatcht hat, wird das gematchte Pattern auf der Anfrage vermerkt:

```rust
if req.route_is(&["users.show", "users.*"]) {
    // wir sind in der Route users.show oder users.*
}

let pattern = req.route_pattern(); // Some("/users/{id}")
let name = req.route_name();       // Some("users.show")
```

`route_is(&[...])` akzeptiert `*`-Wildcards (Laravels `Str::is`-Semantik).

## Frühzeitig abbrechen

Für Fehlerbehandlung mit frühem Ausstieg ohne den vollständigen `Response`-Umschlag liefern die Helfer `abort_with` / `abort_if` / `abort_unless` einen `FrameworkError`, der über die übliche `From<FrameworkError> for HttpResponse`-Pipeline gerendert wird. Sie komponieren sich direkt mit `?`:

```rust
use suprnova::{abort_if, abort_unless, abort_with, handler, json_response, Request, Response};

#[handler]
pub async fn show(req: Request) -> Response {
    let id = req.param("id")?;

    // 404, wenn die Ressource fehlt.
    abort_if(id == "0", 404, "User not found")?;

    // 403, wenn der Aufrufer nicht authentifiziert ist.
    abort_unless(req.has_header("Authorization"), 403, "Login required")?;

    // Oder einen Status bedingungslos auslösen:
    if some_condition() {
        return Err(abort_with(418, "I'm a teapot").unwrap_err().into());
    }

    json_response!({ "id": id })
}
```

`abort_if` / `abort_unless` liefern `Ok(())`, wenn die Bedingung falsch ist, sodass das `?` normal weiterläuft.

## Warum Suprnova abweicht

Laravel legt eine synchrone, zusammengeführte Input-Bag offen -
`$req->input('field')`, `$req->all()`, `$req->only(['a','b'])`,
`$req->boolean('flag')` - gespeist aus dem Query-String und dem
geparsten Body zusammen. Suprnova liefert diese Oberfläche nicht mit.
Der Grund:

- Suprnovas Body wird genau einmal verbraucht und ist async. Ein
  synchrones `all()` würde erfordern, jeden Body vorab zu puffern, nur
  um eine Methode zu bedienen, die die meisten Handler nie aufrufen -
  die Speicher- und DoS-Angriffsfläche ist eine andere als bei PHPs
  Lebenszyklus mit einem Prozess pro Anfrage.
- Die typisierte Alternative (`#[request]` + `FormRequest`) liefert
  Feldnamen zur Compile-Zeit, Validierung und ein Parsen, das den
  Content-Type kennt - genau das Sicherheitsnetz, das der untypisierten
  Bag fehlt.

Für die Inspektion von Query, Headern und Route greifen Sie zu
`query_param`, `query_into`, `has_query`, `bearer_token` und den
Header-Lesern weiter oben. Für den Zugriff auf den Body definieren Sie
eine `#[request]`-Struktur oder einen
`#[derive(MultipartRequest)]`-Extraktor.

## Nächste Schritte

- [Validierung](validation.md) - die Regelbibliothek hinter
  `#[validate(...)]` und die Form der 422-Error-Bag
- [Antworten](responses.md) - `HttpResponse`-Werte aus Ihrem Handler
  zurückgeben, einschließlich Streaming und Redirects
- [Fehler](errors.md) - Handler-Muster, die darauf aufbauen, dass
  `Response` ein `Result<HttpResponse, HttpResponse>` ist
- [Routing](routing.md) - Routen registrieren und die
  `{id}`-Parameter, die `req.param("id")` liest
- [Authentifizierung](authentication.md) - `Auth::user_as`,
  `Auth::attempt` und die Guards, die den aktuellen Benutzer aus der
  Anfrage auflösen
- [Dateisystem](filesystem.md) - die Storage-Disks registrieren, auf
  die `UploadedFile::store_as` schreibt
