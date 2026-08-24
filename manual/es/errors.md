# Manejo de errores

Esta es la guía de patrones cotidianos para escribir código falible en
los handlers, servicios y middleware de Suprnova. Para el modelo que hay
debajo - el contrato de conversión, el límite de pánico, la regla de
sanitización de los 5xx, los ganchos de observabilidad - lee
[Modelo de errores](error-model.md). Este capítulo muestra qué escribir
realmente.

La forma que hay que recordar:

- Los handlers devuelven `Response = Result<HttpResponse, HttpResponse>`.
- El operador `?` ejecuta una única conversión directa `From<E>` al tipo de
  error del handler; Rust no encadena `DbErr -> FrameworkError ->
  HttpResponse`. En un handler `Response`, convierte explícitamente el error
  de SeaORM. El código que ya devuelve `Result<_, FrameworkError>` puede usar
  `.await?` directamente.
- Tres ayudantes libres (`abort_with`, `abort_if`, `abort_unless`) te
  permiten cortocircuitar en un código de estado sin nombrar un tipo de
  error.

```rust
use sea_orm::EntityTrait;
use suprnova::{DB, FrameworkError, Request, Response, json_response};

pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| FrameworkError::param_parse("id", "i64"))?;
    let user = users::Entity::find_by_id(id)
        .one(&*DB::get()?)
        .await
        .map_err(FrameworkError::from)?
        .ok_or_else(|| FrameworkError::not_found("User"))?;
    json_response!({ "user": user })
}
```

El resto del capítulo es el catálogo de productores de errores - qué
construir, qué estado devuelve, qué forma ve el cliente.

## `?` es la conversión

Cada `?` en el cuerpo de un handler ejecuta una única conversión directa de
`From<E> for HttpResponse`. El framework proporciona conversiones directas
para sus tipos de error orientados al handler, pero Rust no encadena varias
implementaciones de `From`. Convierte explícitamente un error intermedio
cuando no tiene una conversión directa a `HttpResponse`.
```rust
use suprnova::{DB, FrameworkError, Request, Response, json_response};
use sea_orm::EntityTrait;

pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| FrameworkError::param_parse("id", "i64"))?;

    let user = users::Entity::find_by_id(id)
        .one(&*DB::get()?)
        .await
        .map_err(FrameworkError::from)?
        .ok_or_else(|| FrameworkError::not_found("User"))?;

    json_response!({ "user": user })
}
```


En ese fragmento ocurren cuatro conversiones:

1. `req.param("id")?` convierte directamente `ParamError` a un
   `HttpResponse` (400).
2. El error de análisis se asigna explícitamente a
   `FrameworkError::ParamError`, que `?` convierte después directamente a un
   `HttpResponse` (400).
3. El error de SeaORM se asigna explícitamente de `DbErr` a
   `FrameworkError::Database`; `?` convierte después directamente ese
   `FrameworkError` a un `HttpResponse` (500, sanitizado en la red).
4. `.ok_or_else(...)?` convierte `None` en
   `FrameworkError::ModelNotFound`, que se convierte en un `HttpResponse`
   (404).

Cada `?` usa una conversión directa. El código que devuelve
`Result<_, FrameworkError>` en lugar de `Response` puede usar `.await?` en la
llamada de SeaORM porque `DbErr` se convierte directamente a
`FrameworkError`.

## `AppError` - errores de dominio en línea

Usa `AppError` para los errores puntuales que no merecen un tipo
dedicado. Los constructores se corresponden con la forma
`abort($status, $msg)` de Laravel:

| Constructor | Estado |
|---|---|
| `AppError::new(msg)` | 500 |
| `AppError::bad_request(msg)` | 400 |
| `AppError::unauthorized(msg)` | 401 |
| `AppError::forbidden(msg)` | 403 |
| `AppError::not_found(msg)` | 404 |
| `AppError::conflict(msg)` | 409 |
| `AppError::unprocessable(msg)` | 422 |
| `AppError::new(msg).status(code)` | cualquiera |

`AppError` tiene un `From` hacia `FrameworkError`, así que `?` funciona
sin ceremonias:

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

Fíjate en la asimetría: `AppError::unauthorized` es **401** (faltan las
credenciales de autenticación), mientras que
`FrameworkError::Unauthorized` es **403** (una política denegó a un
usuario autenticado). Significan cosas distintas; elige la que coincida
con el fallo.

## `FrameworkError` - el enum canónico

Los extractores internos, el contenedor, la vinculación de rutas, la
validación, la capa de base de datos y el almacenamiento producen todos
un `FrameworkError`. Normalmente construyes uno mediante un constructor
de conveniencia y dejas que `?` lo enrute.

```rust
use suprnova::FrameworkError;

FrameworkError::not_found("User");                    // 404
FrameworkError::bad_request("Bad input");             // 400
FrameworkError::param("user_id");                     // 400
FrameworkError::param_parse("user_id", "i64");        // 400
FrameworkError::validation("email", "required");      // 422
FrameworkError::domain("Conflict", 409);              // 409 (cualquier código)
FrameworkError::internal("disk full");                // 500
FrameworkError::database("timeout");                  // 500
FrameworkError::service_not_found::<MyService>();     // 500
FrameworkError::model_not_found("Post");              // 404
```

El conjunto completo de variantes, con sus implicaciones para la forma de
la respuesta, está en [Modelo de errores](error-model.md). Los
constructores de arriba cubren todos los casos habituales; echas mano de
las variantes directamente solo cuando haces match sobre un error que has
recibido.

### Conversiones automáticas

`FrameworkError` ya habla los dialectos que emiten tus dependencias.
Estos dos `?` convierten automáticamente:

```rust
use suprnova::{DB, FrameworkError};
use sea_orm::ActiveModelTrait;

pub async fn create_user(new_user: users::ActiveModel)
    -> Result<users::Model, FrameworkError>
{
    // DB::get devuelve Result<_, FrameworkError>.
    // .insert devuelve Result<_, DbErr>, con From<DbErr> para FrameworkError.
    let user = new_user.insert(&*DB::get()?).await?;
    Ok(user)
}
```

El framework también implementa `From<opendal::Error>` para las
operaciones de almacenamiento y `From<ParamError>` para la extracción de
parámetros de ruta.

### Volver a lanzar con contexto

Cuando quieras anotar de dónde vino un error sin perder el código de
estado, usa `.context()`:

```rust
db.insert(user).await
    .map_err(FrameworkError::from)
    .map_err(|e| e.context("creating new user"))?;
```

El mensaje se convierte en `"creating new user: <original>"`. Las
variantes estructuradas (`Validation`, `ValidationError`,
`ModelNotFound`, `ParamParse`, `PrecognitionFailure`,
`PrecognitionSuccess`, `Unauthorized`, `UnsupportedMediaType`,
`AlreadyReported`, `RateLimited`, `External`) conservan su variante para
que el renderizador de respuestas siga emitiendo la forma correcta (y,
en `External`, para que sobreviva el origen envuelto); las variantes
planas que solo llevan un mensaje (`Internal`, `Database`, `Domain`) se
aplanan en un `Domain` con el mensaje con el prefijo añadido y con el
estado original conservado.

### Convertir los errores de clave duplicada en 422

La regla de validación `Unique` ejecuta un `SELECT COUNT(*)` antes de la
escritura, así que es orientativa - dos solicitudes concurrentes pueden
pasar ambas y luego intentar las dos la inserción. La solicitud que
pierde recibe una violación de restricción de unicidad de la base de
datos, que de otro modo se filtraría como un 500.
`from_unique_violation` la traduce al mismo 422 que habría producido la
regla orientativa:

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

Si el `DbErr` subyacente no es una violación de restricción de unicidad,
pasa sin cambios como un error `Database` de clase 500. La cobertura de
backends es la que reconozca `DbErr::sql_err` de SeaORM - Postgres,
MySQL/MariaDB y SQLite mapean todos sus errores de clave duplicada.
### Envolver un error externo

Las variantes que no tienen estructura propia convierten en texto aquello
que envuelven. `from_external_with` conserva el error original para que
los logs puedan mostrar la cadena completa y el código pueda inspeccionar
qué falló:

```rust
use suprnova::FrameworkError;

let row = sqlx_like_query()
    .await
    .map_err(|e| FrameworkError::from_external_with("verify query failed", e))?;
```

`from_external(e)` hace lo mismo usando el `Display` del error como
mensaje. Ambos se convierten en HTTP 500.

Para inspeccionar el original, usa `external_source()` en lugar de
`source()`:

```rust
if let Some(src) = err.external_source() {
    if let Some(db) = src.downcast_ref::<sea_orm::DbErr>() {
        // decide whether this is worth retrying
    }
}
```

`std::error::Error::source()` devuelve el handle `Arc` compartido, no el
error envuelto, por lo que el downcast devuelve `None`.
`external_source()` desreferencia primero el handle. El framework registra
la cadena completa en la línea 5xx y en `debug_message` cuando
`APP_DEBUG=true`.

### Conservar indicaciones de límite de velocidad

`rate_limited` conserva estructurada una indicación descendente
`Retry-After`:

```rust
use std::time::Duration;
use suprnova::FrameworkError;

let err = FrameworkError::rate_limited(
    Some(Duration::from_secs(30)),
    "push provider rejected the batch",
);

assert_eq!(err.retry_after(), Some(Duration::from_secs(30)));
assert_eq!(err.status_code(), 429);
```

Las políticas de reintento de colas, la planificación con jitter y la
cabecera HTTP `Retry-After` leen el valor mediante `retry_after()`, que
devuelve `None` para las demás variantes o cuando no se proporcionó una
indicación. `.context(...)` conserva la variante y no elimina la duración.


## Errores de dominio personalizados

Tres niveles, según lo reutilizable que necesite ser el error.

### `#[domain_error]` para el caso tipado

La mayoría de los errores reutilizables quieren un nombre, un estado fijo
y una plantilla de mensaje fija - sin mensaje por llamada. La macro de
atributo `#[domain_error]` genera `Display`, `std::error::Error`,
`HttpError` y `From` para `FrameworkError` de una sola vez:

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

Úsalos en el sitio de la llamada con `?`:

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

La macro rechaza estrepitosamente los atributos mal formados en tiempo de
compilación - códigos de estado desbordados (`status = 70_000`), tipos de
literal equivocados (`message = 42`), claves desconocidas - así que no
puedes acabar en silencio con el estado equivocado por culpa de una
errata.

#### Crea uno con andamiaje desde la CLI

```bash
suprnova make:error UserNotFound
```

Escribe `src/errors/user_not_found.rs` con un `status = 500` por defecto
y un mensaje inferido con formato de frase, y actualiza
`src/errors/mod.rs` para reexportarlo. Edita el `status` y el `message` a
tu gusto.

### `HttpError` para el caso hecho a mano

Cuando un error de dominio necesita estado en tiempo de ejecución dentro
del mensaje (por ejemplo, los IDs implicados en el fallo), implementa
`HttpError` directamente. El trait tiene dos métodos con valores por
defecto razonables:

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

Para puentear un `HttpError` hecho a mano hacia `?`, llama a
`FrameworkError::from_http_error`. Un `From<T: HttpError> for
FrameworkError` general entraría en conflicto con la implementación
existente de `From<AppError>`, así que el puente es un constructor
explícito:

```rust
account.withdraw(amount)
    .map_err(FrameworkError::from_http_error)?;
```

### Enums de error para los fallos de un módulo

Cuando un servicio tiene varios fallos relacionados, agrúpalos en un enum
y escribe un solo `From` para todo el enum:

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

Una vez que existe el `From`, el enum se enhebra a través de `?` igual
que cualquier otro tipo de error.

## `abort_with` / `abort_if` / `abort_unless`

Tres ayudantes cortocircuitan un handler en un estado dado. Reflejan
`abort` / `abort_if` / `abort_unless` de Laravel. (La función libre se
exporta como `abort_with` en vez de `abort` para dejar este último nombre
disponible como nombre de método en los tipos del usuario.)

```rust
use suprnova::{abort_if, abort_unless, abort_with, Request, Response, json_response};

pub async fn show(req: Request) -> Response {
    abort_unless(Auth::user().await?.is_some(), 401, "must be logged in")?;
    abort_if(req.param("id")? == "0", 404, "User not found")?;
    abort_with(503, "scheduled maintenance")?;

    json_response!({ "ok": true })
}
```

Cada una devuelve `Result<(), FrameworkError>`, así que `?` hace el
trabajo. El error subyacente es
`FrameworkError::Domain { message, status_code }`, que se renderiza con
la misma forma de cuerpo que cualquier otro error. Los códigos de estado
fuera de rango se coercionan a 500 mediante el renderizador de
respuestas; no necesitas defenderte de entradas incorrectas en el sitio
de la llamada.

## `ValidationErrors` - la bolsa de errores con forma Laravel

Cuando la validación falla - en el momento de `#[derive(Validate)]` o
dentro de un cuerpo `after_validation` - el framework emite la forma JSON
que esperan los frontends de Laravel e Inertia:

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

La mayor parte del tiempo no construyes esto directamente -
`#[derive(Validate)]` se ejecuta y el framework convierte
`validator::ValidationErrors` por ti. Cuando necesitas añadir errores de
forma imperativa (reglas entre campos, comprobaciones asíncronas de
unicidad que complementan a `Unique`), construye un `ValidationErrors` y
devuélvelo:

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

`add_to_bag` acota un campo bajo una bolsa nombrada (la forma
`withErrors($errors, 'profile')` de Laravel) anteponiendo el nombre de la
bolsa con un separador `.`. Es útil cuando una sola respuesta lleva
errores de varios subformularios que no pueden compartir un espacio de
nombres plano:

```rust
let mut errs = ValidationErrors::new();
errs.add_to_bag("profile", "bio", "must be under 280 characters");
errs.add_to_bag("billing", "card", "expired");
// mapa de errores: { "profile.bio": [...], "billing.card": [...] }
```

`from_validator(ve)` convierte un `validator::ValidationErrors`;
`retain_fields(&keep)` devuelve una copia que contiene solo las entradas
listadas (lo usa internamente el encabezado `Precognition-Validate-Only`
de Precognition).

## Enganchando la observabilidad con `ErrorOccurred`

Toda respuesta 5xx dispara un evento `ErrorOccurred` - incluidas las
sintetizadas a partir de pánicos. Escúchalo de la misma forma en que
escuchas cualquier evento:

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

// En bootstrap.rs:
// `listen` infiere ambos genéricos a partir del tipo del oyente. Devuelve
// `()` (el registro no puede fallar), así que no hay `?` ni Result.
EventFacade::listen::<ErrorOccurred, SentryReporter>(Arc::new(SentryReporter)).await;
```

El evento lleva el mensaje de error en crudo (el cuerpo de la respuesta
sigue sanitizado - consulta [Modelo de errores](error-model.md)), el
estado y el id de solicitud correlacionable. Este es el equivalente en
Suprnova del callback `report()` de Laravel en el handler de excepciones.

## Patrones que escribirás a menudo

### Analizar un parámetro de ruta como valor tipado

```rust
let id: i64 = req.param("id")?.parse()
    .map_err(|_| FrameworkError::param_parse("id", "i64"))?;
```

`ParamError` ya convierte a 400; `param_parse` es el equivalente para el
fallo de análisis y renderiza la misma forma.

### Buscar por ID, 404 si no está

```rust
let user = users::Entity::find_by_id(id)
    .one(&*DB::get()?)
    .await
    .map_err(FrameworkError::from)?
    .ok_or_else(|| FrameworkError::not_found("User"))?;
```

`map_err(FrameworkError::from)?` puentea el `DbErr` de SeaORM a través de
`From<DbErr> for FrameworkError` y luego a través de
`From<FrameworkError> for HttpResponse`. Rust no encadena
automáticamente las implementaciones de `From` a lo largo de dos saltos,
así que el `.map_err` explícito es obligatorio.

O, con la capa Eloquent (que ya envuelve SeaORM y devuelve
`Result<_, FrameworkError>` directamente):

```rust
use suprnova::Model;

let user = User::find_or_fail(id).await?;
```

`find_or_fail` es `find(id).ok_or(ModelNotFound)` empaquetado.

### Autorizar una acción

```rust
let user = Auth::user().await?
    .ok_or_else(|| AppError::unauthorized("login required"))?;
abort_unless(post.owner_id == user.id() || user.is_admin(), 403,
    "you don't own this post")?;
```

`abort_unless` devuelve `Result<(), FrameworkError>`; el `?` lo colapsa
de vuelta en el brazo de error de tu handler.

### Un servicio que devuelve errores tipados

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

// Sitio de la llamada:
pub async fn show(req: Request) -> Response {
    let email = req.param("email")?;
    let user = App::resolve::<UserService>()?
        .find_by_email(email)
        .await?;
    json_response!({ "user": user })
}
```

`App::resolve::<UserService>()?` devuelve
`Result<Arc<UserService>, FrameworkError>`. El `?` encadenado colapsa
tanto el fallo de resolución como el fallo de búsqueda en una respuesta.

## Hoja de referencia

| Quieres… | Echa mano de |
|---|---|
| Un error en línea con un estado | `AppError::bad_request("…")` y compañía |
| Un error tipado reutilizable | `#[domain_error(status = …, message = "…")]` |
| Andamiaje generado | `suprnova make:error UserNotFound` |
| Hecho a mano con estado en tiempo de ejecución | `impl HttpError for MyError` |
| Puentear lo hecho a mano hacia `?` | `FrameworkError::from_http_error(e)` |
| Cortocircuitar en un estado | `abort_with` / `abort_if` / `abort_unless` |
| 404 cuando falta el modelo | `FrameworkError::not_found("User")` / `Model::find_or_fail` |
| Fallo de análisis en un parámetro de ruta | `FrameworkError::param_parse("id", "i64")` |
| Error de validación a nivel de campo | `FrameworkError::validation("email", "…")` |
| Bolsa de errores de varios campos | `ValidationErrors::new().add(…)` + `Validation(errs)` |
| Violación de clave duplicada → 422 | `FrameworkError::from_unique_violation(field, msg, e)` |
| Anotar un error existente | `err.context("creating user")` |
| Observar todos los 5xx | Escuchar `ErrorOccurred` |

## Siguiente

- [Modelo de errores](error-model.md) - variantes, contrato de
  conversión, sanitización de los 5xx, límite de pánico
- [Validación](validation.md) - `#[derive(Validate)]`, solicitudes de
  formulario, y `after_validation`
- [Respuestas](responses.md) - constructores de `HttpResponse`, estado,
  encabezados
- [Eventos](events.md) - escuchar `ErrorOccurred` y otros eventos
  integrados
- [Ciclo de vida de la solicitud](lifecycle.md) - en qué punto del flujo
  de la solicitud se ejecuta la conversión de errores
