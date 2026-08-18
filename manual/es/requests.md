# Solicitudes

Los handlers de Suprnova reciben un `Request` - la solicitud HTTP tal
como llega por la red - o un struct de solicitud de formulario tipado que
analiza, valida y autoriza el cuerpo antes de que se ejecute tu código.
Ambos caminos viven sobre la misma macro `#[handler]`; eliges la forma
ruta por ruta. Este capítulo cubre los dos, más el extractor de subidas
multipart y los accesores en crudo a los que se echa mano en el
middleware.

## Solicitudes de formulario tipadas

El atributo `#[request]` marca un struct como `FormRequest`. La macro
añade los derives `serde::Deserialize` y `validator::Validate` y emite un
`impl FormRequest` para que la macro `#[handler]` sepa que debe extraerlo
y validarlo a la entrada:

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

A un handler que nombre este tipo como su parámetro se le entrega un
valor ya validado:

```rust
use suprnova::{handler, json_response, Response};
use crate::requests::CreateUserRequest;

#[handler]
pub async fn store(form: CreateUserRequest) -> Response {
    // `form` está validado - este código solo se ejecuta si pasaron todas las reglas.
    json_response!({ "email": form.email, "name": form.name })
}
```

Un handler que en su lugar nombre `Request` recibe la solicitud en crudo
tal cual, sin cambios:

```rust
use suprnova::{handler, json_response, Request, Response};

#[handler]
pub async fn index(req: Request) -> Response {
    json_response!({ "path": req.path() })
}
```

Ambos son extractores - la macro `#[handler]` busca
`FromRequest::from_request` para cada tipo de parámetro, y cualquier
struct que implemente `FormRequest` obtiene gratis un impl general
(blanket impl) de `FromRequest`.

## Reglas de validación

La validación pasa por el crate `validator`. Reglas comunes:

### Validaciones de cadenas

```rust
#[request]
pub struct ExampleRequest {
    // Requerido (no vacío)
    #[validate(length(min = 1, message = "This field is required"))]
    pub name: String,

    // Formato de correo electrónico
    #[validate(email(message = "Invalid email address"))]
    pub email: String,

    // Formato de URL
    #[validate(url(message = "Invalid URL"))]
    pub website: String,

    // Restricciones de longitud
    #[validate(length(min = 8, max = 100))]
    pub password: String,

    // Patrón de regex - PHONE_REGEX tiene que ser un `static` o un `const`
    // visible desde el punto de expansión del validador. Decláralo una
    // sola vez, normalmente en el mismo módulo:
    #[validate(regex(path = "PHONE_REGEX", message = "Invalid phone number"))]
    pub phone: String,
}

use std::sync::LazyLock;
use regex::Regex;

// validator 0.20 implementa `AsRegex` para `std::sync::LazyLock<Regex>`
// pero no para `once_cell::sync::Lazy<Regex>` - usa el tipo de std para
// que la expansión de `#[validate(regex(path = "..."))]` del derive pase
// la comprobación de tipos.
static PHONE_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\+?[0-9\s\-()]{7,20}$").unwrap());
```

### Validaciones numéricas

```rust
#[request]
pub struct ProductRequest {
    // Validación de rango - los literales deben coincidir con el tipo del
    // campo. `f64` toma `0.0` / `10000.0`, no los literales enteros
    // `0` / `10000`.
    #[validate(range(min = 0.0, max = 10000.0, message = "Price must be between 0 and 10000"))]
    pub price: f64,

    // Valor mínimo
    #[validate(range(min = 1))]
    pub quantity: i32,

    // Valor máximo
    #[validate(range(max = 100))]
    pub discount_percent: i32,
}
```

### Validaciones anidadas y de colecciones

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
    // Validación de struct anidado
    #[validate(nested)]
    pub shipping_address: Address,

    // Longitud de la colección
    #[validate(length(min = 1, message = "At least one item required"))]
    pub items: Vec<String>,
}
```

### Atributos de validación comunes

| Atributo | Descripción | Ejemplo |
|-----------|-------------|---------|
| `email` | Formato de correo válido | `#[validate(email)]` |
| `url` | Formato de URL válido | `#[validate(url)]` |
| `length` | Longitud de cadena o colección | `#[validate(length(min = 1, max = 100))]` |
| `range` | Rango numérico | `#[validate(range(min = 0, max = 100))]` |
| `regex` | Coincidencia con un patrón de regex | `#[validate(regex(path = "PATTERN"))]` |
| `contains` | La cadena contiene una subcadena | `#[validate(contains(pattern = "@"))]` |
| `does_not_contain` | La cadena no contiene | `#[validate(does_not_contain(pattern = "admin"))]` |
| `nested` | Valida un struct anidado | `#[validate(nested)]` |

## Respuestas de error de validación

Cuando la validación falla, Suprnova devuelve una respuesta 422 con la
bolsa de errores compatible con Laravel / Inertia:

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

La forma de `errors` coincide con lo que los clientes `@inertiajs/*` leen
directamente de `usePage().props.errors`.

### Campos anidados

Un fallo de `#[validate(nested)]` se reporta bajo una clave con puntos
que nombra la ruta completa, la misma notación que usa Laravel. Un struct
anidado aporta `parent.field`; un elemento de un `Vec<T>` validado aporta
`parent.<index>.field`:

```json
{
    "message": "The given data was invalid.",
    "errors": {
        "shipping_address.street": ["Validation failed for field 'shipping_address.street'"],
        "items.1.name": ["Validation failed for field 'items.1.name'"]
    }
}
```

El índice `1` es el segundo elemento - el primero pasó y está ausente de
la bolsa. Vincula la clave tal cual en el cliente:
`form.errors['items.1.name']`.

## Ejemplo completo

Un endpoint de registro de usuarios, de extremo a extremo.

**Define la solicitud:**

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

**Crea el controlador:**

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
    // La validación pasó - crea el usuario
    // En una app real, aquí guardarías en la base de datos

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

**Registra las rutas:**

```rust
// src/routes.rs
use suprnova::{get, post, routes};
use crate::controllers;

routes! {
    get!("/users", controllers::user::index).name("users.index"),
    post!("/users", controllers::user::store).name("users.store"),
}
```

## Autorización y ganchos entre campos

El trait `FormRequest` expone tres ganchos de ciclo de vida: `authorize`,
`after_validation` y `after_validation_async`. Tanto el atributo
`#[request]` como la forma `#[derive(FormRequestDerive)]` emiten por ti
un `impl FormRequest` por defecto. Para sobrescribir cualquier gancho,
añade el opt-out `#[form_request(custom_hooks)]` para suprimir el impl
por defecto y luego escribe el tuyo. (Esto refleja el patrón
`#[multipart(custom_hooks)]`.)

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
        // Devuelve false para cortocircuitar con un 403 Forbidden antes
        // de que se lea el cuerpo.
        req.header("X-Admin-Token").is_some()
    }
}
```

El opt-out también funciona bajo la forma del atributo `#[request]` -
útil cuando quieres los derives automáticos del atributo pero necesitas
sobrescribir ganchos:

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

Cuando `authorize` devuelve `false`, la extracción devuelve
`FrameworkError::Unauthorized` y renderiza:

```json
HTTP 403 Forbidden

{ "message": "This action is unauthorized." }
```

`after_validation` es el gancho síncrono entre campos - úsalo para reglas
del tipo "la contraseña y su confirmación deben coincidir".
`after_validation_async` es la contraparte asíncrona y es donde las
reglas respaldadas por la base de datos (por ejemplo, el `Unique`
integrado) participan en la validación automática. Ambos se disparan
después de que pasen las reglas por campo de `validator`; `extract` se
detiene en la primera etapa que falle.

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

### Límites de tamaño del cuerpo

El atributo por struct `#[form_request(max_body_bytes = N)]` anula el
tope global del proceso de 8 MiB en un único FormRequest:

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

`Content-Length` se analiza por adelantado y la solicitud se rechaza con
un HTTP 413 *antes* de leer un solo byte del cuerpo cuando el tamaño
declarado supera el tope; los clientes que mienten sobre
`Content-Length` acaban igualmente activando el contador de bytes en
streaming durante la lectura.

## Detección del tipo de contenido

`FormRequest::extract` solo mira el encabezado `Content-Type`:

- `application/x-www-form-urlencoded` → se analiza vía `serde_urlencoded`
- `application/json` o cualquier sufijo `application/*+json` → se analiza vía `serde_json`
- Cualquier otra cosa (incluido un encabezado ausente) → se rechaza con
  un HTTP 415 Unsupported Media Type, antes de leer el cuerpo

Para los cuerpos multipart (`multipart/form-data`), consulta
[subida de archivos](#subida-de-archivos-multipartrequest) más abajo.

## Leer el cuerpo directamente

Para endpoints puntuales o middleware que no quiere un `FormRequest`
completo, el propio tipo `Request` lee el cuerpo de tres formas -
cada una consume `self`, porque el cuerpo se puede leer como mucho una
vez:

```rust
use serde::Deserialize;
use suprnova::{handler, json_response, Request, Response};

#[derive(Deserialize)]
struct LoginForm { username: String, password: String }

#[handler]
pub async fn login(req: Request) -> Response {
    // Elige el parser de forma explícita.
    let form: LoginForm = req.form().await?;
    json_response!({ "user": form.username })
}

#[handler]
pub async fn webhook(req: Request) -> Response {
    // La misma forma, pero JSON por la red.
    let payload: serde_json::Value = req.json().await?;
    json_response!({ "received": payload })
}

#[handler]
pub async fn ingest(req: Request) -> Response {
    // Elige solo según el Content-Type - JSON salvo que
    // `application/x-www-form-urlencoded` sea explícito.
    let value: serde_json::Value = req.input().await?;
    json_response!({ "value": value })
}
```

Para acceso en crudo, `req.body_bytes().await` devuelve los `Bytes` ya
bufferizados más los metadatos de `RequestParts` (los params de ruta y el
content type). Usa `body_bytes_with_cap(n)` para anular el tope global de
8 MiB caso por caso.

## Resolver servicios junto al formulario

Las solicitudes de formulario validadas se componen con el
[contenedor de servicios](container.md). Usa `App::resolve::<T>()` (o
`App::get::<T>()`) dentro del handler:

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

## Subida de archivos (`MultipartRequest`)

`multipart/form-data` tiene su propio extractor -
`#[derive(MultipartRequest)]` transmite el cuerpo parte por parte y
vuelca las partes de archivo grandes a un archivo temporal por encima del
umbral configurado, de modo que una subida de 200 MiB nunca reside entera
en RAM. Cada campo lleva una anotación `#[field("name")]` que nombra el
campo tal como llega por la red; los campos de archivo usan
`UploadedFile<V>`, donde `V` es un validador (o una tupla de validadores)
de `suprnova::http::upload::validators`.

```rust
use suprnova::{handler, json_response, MultipartRequest, Response};
use suprnova::http::upload::UploadedFile;
use suprnova::http::upload::validators::{Image, MaxSize};

#[derive(MultipartRequest)]
pub struct AvatarUpload {
    #[field("avatar")]
    pub avatar: UploadedFile<(Image, MaxSize<5_242_880>)>, // tope de 5 MiB
    #[field("caption")]
    pub caption: Option<String>,
}

#[handler]
pub async fn upload_avatar(form: AvatarUpload) -> Response {
    // `avatar` está en memoria o en un archivo temporal según su tamaño.
    // `.bytes()` lee cualquiera de los dos; `.store_as(...)` transmite a un disco.
    let bytes = form.avatar.bytes().await?;
    json_response!({ "size": bytes.len(), "caption": form.caption })
}
```

Formas de campo:

| Declaración | Forma en la red |
|---|---|
| `UploadedFile<V>` | archivo requerido |
| `Option<UploadedFile<V>>` | archivo opcional |
| `Vec<UploadedFile<V>>` | subidas en array (`photos[]`) |
| `String` / `u32` / cualquier `FromStr` | campo de texto (requerido) |
| `Option<String>` / `Option<T: FromStr>` | campo de texto opcional |
| `Vec<String>` / `Vec<T: FromStr>` | campos de texto repetidos |

Validadores integrados en `suprnova::http::upload::validators`:

- `MaxSize<N>` - cortocircuita en el límite de bytes cuando el total
  acumulado supera `N` bytes (HTTP 413).
- `Image` - rechaza las partes cuyos magic bytes no declaran `image/*`.
- `MimeType<L>` - acepta una lista fija de permitidos que proporciona tu
  propio tipo `MimeAllowlist`.
- `()` - no hace nada; `UploadedFile<()>` acepta cualquier byte.

Los validadores se componen como tuplas: `(Image, MaxSize<5_242_880>)`
ejecuta ambos y cortocircuita en el primer fallo.

### Límites por campo y cotas de array

El tope de bytes sobre el cuerpo total es global (8 MiB por defecto para
multipart, configurable vía
`suprnova::http::upload::set_global_max_multipart_body_bytes`). Los topes
por campo evitan el abuso en el que un cuerpo con muchas partes pequeñas
hace crecer `Vec<UploadedFile<_>>` sin límite dentro del presupuesto de
bytes:

```rust
#[derive(MultipartRequest)]
pub struct Gallery {
    #[field("photos", max_count = 8)]
    pub photos: Vec<UploadedFile<MaxSize<1_048_576>>>,
}
```

La parte número (`max_count` + 1) con ese nombre devuelve un HTTP 422
antes de reservar memoria, así que la parte de más nunca llega a hacer
crecer el `Vec`.

### Ganchos de autorización y de posvalidación

`MultipartRequest` refleja los ganchos de `FormRequest` mediante el trait
`MultipartRequestHooks`. Por defecto el derive emite un impl vacío; opta
por el tuyo con `#[multipart(custom_hooks)]`:

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

### Streaming hacia el almacenamiento

`UploadedFile::store_as` escribe la parte en un disco de almacenamiento
registrado. Para las partes respaldadas por disco el camino es
completamente en streaming (trozos de 64 KiB vía
`opendal::Operator::writer`); las partes en memoria usan una sola llamada
de escritura. Usa la extensión derivada del contenido cuando la ruta de
almacenamiento sea direccionable por contenido - el encabezado con el
nombre del archivo no es de fiar:

```rust
use suprnova::Storage;

let disk = Storage::disk("avatars")?;
let path = format!("{}.{}", user.id, form.avatar.extension_from_magic());
form.avatar.store_as(&disk, &path).await?;
```

Consulta [Sistema de archivos](filesystem.md) para el registro de discos
de almacenamiento.

## Organización de archivos

La estructura estándar para las solicitudes:

```
src/
├── requests/
│   ├── mod.rs                 # Reexporta todas las solicitudes
│   ├── create_user.rs         # CreateUserRequest
│   ├── update_user.rs         # UpdateUserRequest
│   └── create_post.rs         # CreatePostRequest
├── controllers/
│   └── user.rs                # Usa CreateUserRequest
└── routes.rs
```

**src/requests/mod.rs:**
```rust
pub mod create_user;
pub mod update_user;

pub use create_user::CreateUserRequest;
pub use update_user::UpdateUserRequest;
```

## Seguridad de tipos de extremo a extremo con Inertia

Las solicitudes también pueden derivar `InertiaProps` para generar tipos de TypeScript, lo que habilita seguridad de tipos de extremo a extremo desde tu backend en Rust hasta tu frontend en React.

### Generar tipos de TypeScript para las solicitudes

Añade el derive `InertiaProps` junto a `#[request]`:

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

Ejecuta la generación de tipos:

```bash
suprnova generate-types
```

Esto genera tipos de TypeScript en `frontend/src/types/inertia-props.ts`:

```typescript
export interface CreateTodoRequest {
  title: string
  description: string | null
}
```

### Formularios con seguridad de tipos en Inertia

Usa el componente `<Form>` de Inertia para el manejo de formularios más limpio:

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

Para más control, combina `<Form>` con el hook `useForm` y tus tipos generados:

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

### Qué te aporta el derive

- TypeScript detecta las erratas en los nombres de campo y los desajustes
  de tipos en tiempo de compilación.
- El autocompletado del IDE lee directamente el `.ts` generado.
- Renombra un campo en Rust, vuelve a ejecutar
  `suprnova generate-types`, y la superficie de TypeScript lo sigue.

Consulta [Tipos de TypeScript](frontend-typescript-types.md) para el
pipeline de generación completo.

## Accesores de `Request`

Más allá del patrón de formulario validado de arriba, el tipo `Request` lleva accesores al estilo de Laravel para inspeccionar la solicitud tal como llega por la red - URL, encabezados, query string, negociación de contenido, metadatos de ruta e IP del cliente. Son útiles en el middleware, en los handlers que quieren acceso en crudo junto a un `FormRequest`, y en cualquier sitio donde el análisis validado no sea la herramienta adecuada.

### URL y ruta

| Método | Devuelve | Notas |
|--------|---------|-------|
| `req.path()` | `&str` | La ruta del URI en crudo. |
| `req.decoded_path()` | `String` | La ruta con los escapes percent ya resueltos. |
| `req.segments()` | `Vec<String>` | La ruta partida por `/`, descartando los segmentos vacíos. |
| `req.segment(index, default)` | `Option<String>` | Acceso a segmentos con índice desde 1. |
| `req.url()` | `String` | Esquema + host + ruta (sin query string). |
| `req.full_url()` | `String` | La URL + el query string. |
| `req.full_url_with_query(&[("k","v")])` | `String` | Añade o sobrescribe claves del query. |
| `req.full_url_without_query(&["k"])` | `String` | Elimina claves del query. |

```rust
use suprnova::{handler, json_response, Request, Response};

#[handler]
pub async fn show(req: Request) -> Response {
    if req.is(&["admin/*"]) {
        // la ruta coincide con el wildcard admin/*
    }
    json_response!({ "url": req.full_url() })
}
```

### Host, esquema, IP

| Método | Devuelve | Orden de las fuentes |
|--------|---------|--------------|
| `req.host()` | `Option<String>` | `X-Forwarded-Host` → `Host` → la autoridad del URI. |
| `req.http_host()` | `Option<String>` | El host más el puerto cuando no es el predeterminado. |
| `req.scheme_and_http_host()` | `Option<String>` | `scheme://host:port`. |
| `req.scheme()` | `&'static str` | `"https"` cuando [`secure`] es cierto, si no `"http"`. |
| `req.secure()` | `bool` | El esquema del URI → `X-Forwarded-Proto` → `X-Forwarded-Ssl: on`. |
| `req.ip()` | `Option<String>` | `X-Forwarded-For[0]` → `X-Real-IP` → la dirección del par. |
| `req.ips()` | `Vec<String>` | La cadena completa: los encabezados del proxy, luego la dirección del par. |
| `req.user_agent()` | `Option<&str>` | El encabezado `User-Agent`. |
| `req.port()` | `Option<u16>` | El puerto del encabezado Host → `X-Forwarded-Port` → el puerto del URI. |

### Encabezados y método

| Método | Devuelve |
|--------|---------|
| `req.has_header("X-Foo")` | `bool` |
| `req.bearer_token()` | `Option<String>` (la última subcadena `Bearer `, recortada por comas) |
| `req.is_method("POST")` | `bool` (sin distinguir mayúsculas de minúsculas) |
| `req.ajax()` | `X-Requested-With: XMLHttpRequest` |
| `req.pjax()` | Encabezado `X-PJAX` con valor verdadero |
| `req.prefetch()` | `X-Moz`, `Purpose` o `Sec-Purpose` = `prefetch` |

### Negociación de contenido

```rust
if req.is_json() { /* el Content-Type lleva /json o +json */ }
if req.expects_json() { /* AJAX sin que Accept lo estreche, o Accept prefiere JSON */ }
if req.wants_json() { /* el encabezado Accept encabeza con JSON */ }
if req.accepts_html() { /* Accept admite text/html */ }

let preferred = req.prefers(&["application/json", "text/html"]);
let acceptable = req.acceptable_content_types();
```

`accepts(&[ty])` coincide tanto con los tipos a secas como con los sufijos al estilo `application/<vendor>+json`. `accepts_any_content_type()` devuelve cierto cuando no hay encabezado Accept o cuando la preferencia principal es `*/*`.

### Query string

```rust
let id: Option<String> = req.query_param("id");
let present: bool = req.has_query("id");
let map = req.query_params(); // HashMap<String, String>

// Análisis tipado del query vía serde
#[derive(serde::Deserialize)]
struct SearchQuery { page: u32, q: String }
let q: SearchQuery = req.query_into()?;
```

### Metadatos de ruta

Después de que el router despache una solicitud, el patrón coincidente queda registrado en la solicitud:

```rust
if req.route_is(&["users.show", "users.*"]) {
    // estamos dentro de la ruta users.show o users.*
}

let pattern = req.route_pattern(); // Some("/users/{id}")
let name = req.route_name();       // Some("users.show")
```

`route_is(&[...])` acepta wildcards `*` (la semántica de `Str::is` de Laravel).

## Abortar de forma temprana

Para el manejo de errores con salida temprana sin el envoltorio completo de `Response`, los ayudantes `abort_with` / `abort_if` / `abort_unless` devuelven un `FrameworkError` que se renderiza a través del pipeline estándar `From<FrameworkError> for HttpResponse`. Se componen con `?` directamente:

```rust
use suprnova::{abort_if, abort_unless, abort_with, handler, json_response, Request, Response};

#[handler]
pub async fn show(req: Request) -> Response {
    let id = req.param("id")?;

    // 404 cuando el recurso falta.
    abort_if(id == "0", 404, "User not found")?;

    // 403 cuando quien llama no está autenticado.
    abort_unless(req.has_header("Authorization"), 403, "Login required")?;

    // O lanza un estado sin condiciones:
    if some_condition() {
        return Err(abort_with(418, "I'm a teapot").unwrap_err().into());
    }

    json_response!({ "id": id })
}
```

`abort_if` / `abort_unless` devuelven `Ok(())` cuando la condición es falsa, así que el `?` continúa con normalidad.

## Por qué Suprnova diverge

Laravel expone una bolsa de entrada síncrona y fusionada -
`$req->input('field')`, `$req->all()`, `$req->only(['a','b'])`,
`$req->boolean('flag')` - sacada del query string y del cuerpo ya
analizado a la vez. Suprnova no distribuye esa superficie. La razón:

- El cuerpo de Suprnova se consume una sola vez y es async. Un `all()`
  síncrono exigiría bufferizar todos los cuerpos por adelantado para
  satisfacer un método que la mayoría de los handlers nunca llama - la
  superficie de memoria y de DoS es distinta del ciclo de vida de un
  proceso por solicitud de PHP.
- La alternativa tipada (`#[request]` + `FormRequest`) da nombres de
  campo en tiempo de compilación, validación y análisis consciente del
  content-type - exactamente la red de seguridad que le falta a la bolsa
  sin tipar.

Para inspeccionar el query, los encabezados o la ruta, echa mano de
`query_param`, `query_into`, `has_query`, `bearer_token` y los lectores
de encabezados de arriba. Para el acceso del lado del cuerpo, define un
struct `#[request]` o un extractor `#[derive(MultipartRequest)]`.

## Siguiente

- [Validación](validation.md) - la biblioteca de reglas que hay detrás de
  `#[validate(...)]` y la forma de la bolsa de errores 422
- [Respuestas](responses.md) - construir valores `HttpResponse` de vuelta
  desde tu handler, incluidos el streaming y las redirecciones
- [Errores](errors.md) - patrones de handler construidos sobre el hecho
  de que `Response` es `Result<HttpResponse, HttpResponse>`
- [Enrutamiento](routing.md) - registrar rutas y los parámetros `{id}`
  que lee `req.param("id")`
- [Autenticación](authentication.md) - `Auth::user_as`, `Auth::attempt`
  y los guards que resuelven el usuario actual a partir de la solicitud
- [Sistema de archivos](filesystem.md) - registrar los discos de
  almacenamiento en los que escribe `UploadedFile::store_as`
