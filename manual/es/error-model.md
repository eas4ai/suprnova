# Modelo de errores

Este capítulo es el modelo que subyace al manejo de errores de
Suprnova - los tipos, el contrato de conversión y las garantías de
seguridad que el framework te da gratis. Para los patrones cotidianos
de handler (`?`, devolver errores, construir errores de dominio
personalizados) consulta [Manejo de errores](errors.md); este capítulo
explica *por qué* esos patrones funcionan como funcionan.

Si recuerdas una sola cosa de esta página, que sea esta: **los errores
en Suprnova son valores, no excepciones**. Todo error termina
convirtiéndose en un `HttpResponse` mediante una única conversión
total. No existe un handler de excepciones global porque no existe una
excepción global.

## La estructura

El modelo de errores de Suprnova tiene cinco partes móviles:

| Tipo | Rol |
|---|---|
| `Response = Result<HttpResponse, HttpResponse>` | El contrato que satisface todo handler - ambos brazos ya son respuestas |
| `FrameworkError` | El enum de error canónico del framework; cada ruta de error interna produce uno |
| `AppError` | Error de dominio ad hoc para uso en línea sin un tipo dedicado |
| `HttpError` (trait) | Lo que implementan tus propios errores de dominio tipados para obtener un estado + mensaje |
| `ValidationErrors` | La bolsa de errores con forma Laravel/Inertia para fallos por campo |

Los cinco colapsan en un único `HttpResponse` mediante
implementaciones de `From`. El operador `?` hace la conversión en el
sitio de la llamada; la cadena de middleware la hace en el límite de
la solicitud; el handler de pánico la hace cuando algo se desenrolló.
Hay una única forma de cuerpo para todo, y una única regla de
sanitización para los 5xx.

## `Response` es `Result<HttpResponse, HttpResponse>`

Todo handler devuelve esto:

```rust
pub type Response = Result<HttpResponse, HttpResponse>;
```

Ambos brazos llevan el mismo tipo de payload, que es precisamente el
punto. Cuando la cadena de middleware termina de ejecutar tu handler,
colapsa el resultado con una línea:

```rust
result.unwrap_or_else(|e| e)
```

El framework no necesita saber si tu handler "tuvo éxito" o "falló" -
ambos brazos ya son respuestas HTTP renderizadas. La distinción existe
únicamente para que `?` pueda hacer su trabajo:

```rust
use suprnova::{Request, Response, json_response};

pub async fn show(req: Request) -> Response {
    // `?` cortocircuita en Err. Cada conversión de abajo produce un
    // HttpResponse mediante un impl de From - la cadena colapsa ambos brazos.
    let id: i64 = req.param("id")?.parse().map_err(|_| {
        suprnova::FrameworkError::param_parse("id", "i64")
    })?;
    let user = User::find_or_fail(id).await?;  // 404 si falta
    Ok(json_response!({ "user": user }))
}
```

Ese contrato único - cada ruta de error produce un `HttpResponse`
mediante `From` - es el núcleo del modelo. Todo lo demás en este
capítulo es lo que realmente hacen las distintas implementaciones de
`From`.

### Por qué Suprnova diverge

Laravel lanza excepciones y las enruta a través de una clase `Handler`
global registrada en `app/Exceptions/Handler.php`. El framework
captura todo, le pregunta al handler "¿qué renderizo?", y emite la
respuesta. El modelo de excepciones con desenrollado de PHP hace esto
natural.

Rust no tiene excepciones con desenrollado en el código de usuario. El
equivalente de Suprnova es la implementación
`From<FrameworkError> for HttpResponse` más el evento `ErrorOccurred`.
La conversión es el renderizador; el evento es donde enganchas la
observabilidad (Sentry, PagerDuty, remitentes estructurados). No
registras una clase handler - la conversión es una función, y escuchar
`ErrorOccurred` es el punto de extensión. La misma superficie,
maquinaria distinta.

## `FrameworkError` - el enum canónico

Cada ruta de error dentro del framework - extractores, vinculación de
rutas, el contenedor, la validación, la capa de base de datos, el
almacenamiento - produce un `FrameworkError`. Es un enum con catorce
variantes, cada una etiquetada con su estado HTTP:

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
    AlreadyReported,                                     // solo CLI
}
```

Rara vez haces match sobre la variante. Construyes una mediante un
constructor de conveniencia y dejas que `?` haga el resto:

```rust
use suprnova::FrameworkError;

// Todas estas producen un FrameworkError con el estado correcto:
FrameworkError::not_found("User");                    // → ModelNotFound, 404
FrameworkError::bad_request("Bad input");             // → Domain, 400
FrameworkError::param("user_id");                     // → ParamError, 400
FrameworkError::param_parse("user_id", "i64");        // → ParamParse, 400
FrameworkError::validation("email", "required");      // → ValidationError, 422
FrameworkError::domain("Conflict", 409);              // → Domain, 409
FrameworkError::internal("disk full");                // → Internal, 500
FrameworkError::database("timeout");                  // → Database, 500
```

No hay constructores `unauthorized()` ni `forbidden()` en
`FrameworkError` - `Unauthorized` es una variante fija que lleva el
mensaje de Laravel "This action is unauthorized." con estado 403, y
los casos 401 pasan por `AppError::unauthorized` (siguiente sección).
Nota: la variante se llama `Unauthorized` pero el estado es 403 porque
modela el rechazo de autorización de Laravel, no la autenticación
HTTP.

### Conversión automática

`FrameworkError` implementa `From<sea_orm::DbErr>` y
`From<opendal::Error>`, de modo que los errores de base de datos y de
almacenamiento fluyen a través de `?` sin necesidad de envoltura:

```rust
use suprnova::{DB, FrameworkError};
use sea_orm::ActiveModelTrait;

pub async fn create_user(new_user: ActiveModel) -> Result<Model, FrameworkError> {
    // Ambas llamadas a `?` aquí convierten a FrameworkError automáticamente:
    // - DB::get devuelve Result<_, FrameworkError>
    // - insert devuelve Result<_, DbErr>, que tiene From<DbErr> para FrameworkError
    let user = new_user.insert(&*DB::get()?).await?;
    Ok(user)
}
```

Si tu código devuelve `Result<_, FrameworkError>`, cada error común
que producen tus dependencias ya habla el idioma correcto. El `?` del
controlador no hace más trabajo que convertir un tipo de error en
otro.

### Envolviendo contexto

Cuando necesitas volver a lanzar un error con contexto de la
operación, usa `.context()`:

```rust
db.insert(user).await
    .map_err(FrameworkError::from)
    .map_err(|e| e.context("creating new user"))?;
```

El mensaje se convierte en `"creating new user: <original>"`. La
variante se conserva donde importa - `Validation`, `ValidationError`,
`PrecognitionFailure`, `Unauthorized`, `ModelNotFound` y `ParamParse`
conservan su estructura para que el renderizador de respuestas siga
emitiendo la forma correcta. Las variantes que solo llevan un mensaje
simple (`Internal`, `Database`, `Domain`) se aplanan en un `Domain`
con el mensaje con el prefijo añadido.

## `AppError` - errores de dominio ad hoc

Para errores puntuales donde no quieres definir un tipo dedicado, usa
`AppError`. Implementa `HttpError` y tiene un `From` hacia
`FrameworkError`, de modo que `?` funciona directamente:

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

Los constructores se corresponden limpiamente con la forma de
`abort($status, $msg)` de Laravel:

| `AppError::*` | Estado |
|---|---|
| `bad_request(msg)` | 400 |
| `unauthorized(msg)` | 401 |
| `forbidden(msg)` | 403 |
| `not_found(msg)` | 404 |
| `conflict(msg)` | 409 |
| `unprocessable(msg)` | 422 |
| `new(msg)` | 500 |
| `.status(code)` | cualquiera |

Nota que `AppError::unauthorized` es **401** (autenticación HTTP
ausente), mientras que `FrameworkError::Unauthorized` es **403**
(autorización denegada, coincidiendo con el rechazo de políticas de
Laravel). Significan cosas distintas; elige la que coincida con el
fallo.

## `HttpError` - errores tipados personalizados

Cuando el mismo error de dominio aparece en muchos lugares, modélalo
como un tipo. Implementa `HttpError` y la conversión es tuya:

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

`HttpError` tiene dos métodos, ambos con valores por defecto:

```rust
pub trait HttpError: std::error::Error + Send + Sync + 'static {
    fn status_code(&self) -> u16 { 500 }
    fn error_message(&self) -> String { self.to_string() }
}
```

### Puente hacia `?`

Un `impl<T: HttpError> From<T> for FrameworkError` ingenuo entraría en
conflicto con la implementación existente de `From<AppError>` (porque
`AppError` en sí implementa `HttpError`). Suprnova resuelve el
problema de la regla de huérfanos con un constructor puente explícito
en su lugar:

```rust
use suprnova::{FrameworkError, HttpError};

pub async fn debit(account: &mut Account, amount: i64) -> Result<(), FrameworkError> {
    account.withdraw(amount)
        .map_err(FrameworkError::from_http_error)?;
    Ok(())
}
```

El código de estado y el mensaje se toman de `HttpError::status_code`
y `HttpError::error_message` y se almacenan en una variante
`FrameworkError::Domain`. El renderizador de respuestas sigue entonces
la ruta normal de `Domain`.

### `#[domain_error]` para tipos sin código repetitivo

Si quieres el patrón de error tipado sin escribir a mano las
implementaciones de `Display`, `Error` y `HttpError`, usa la macro de
atributo `#[domain_error]`:

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

`#[domain_error]` genera el conjunto completo de implementaciones,
*incluyendo* `From<YourError> for FrameworkError`, de modo que `?`
funciona directamente sin llamada puente:

```rust
pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| FrameworkError::param_parse("id", "i64"))?;
    let user = User::find(id).await?
        .ok_or_else(|| FrameworkError::from(UserNotFoundError))?;
    Ok(json_response!({ "user": user }))
}
```

Los tres niveles de historia de error personalizado - `AppError` para
lo puntual, `#[domain_error]` para lo tipado-con-macro, `HttpError`
hecho a mano para control total - te dan la herramienta adecuada en
cada nivel de formalidad.

## `ValidationErrors` - la bolsa de errores con forma Laravel

Cuando una solicitud falla la validación, Suprnova emite la misma
forma JSON que esperan los frontends de Laravel e Inertia:

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

Normalmente no construyes esto a mano - `#[derive(Validate)]` en una
solicitud de formulario y el crate `validator` detrás de ella producen
un `validator::ValidationErrors` que Suprnova convierte mediante
`ValidationErrors::from_validator`. Pero el tipo es público cuando lo
necesitas:

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

`add_to_bag` agrupa errores bajo una bolsa nombrada (la forma
`withErrors($errors, 'profile')` de Laravel) anteponiendo el nombre de
la bolsa con un separador `.`:

```rust
let mut errs = ValidationErrors::new();
errs.add_to_bag("profile", "bio", "must be under 280 characters");
errs.add_to_bag("billing", "card", "expired");
// mapa de errores: { "profile.bio": [...], "billing.card": [...] }
```

`retain_fields` conserva solo las entradas listadas - usado
internamente por el encabezado `Precognition-Validate-Only` de
Precognition para que el servidor ejecute la validación completa pero
reporte errores solo para los campos que el cliente solicitó.

## El contrato de conversión

Cuando un `FrameworkError` llega a un límite HTTP, pasa por
`From<FrameworkError> for HttpResponse`. Ocurren tres cosas, en orden:

1. **Enrutamiento de estado**. Se lee una vez el `status_code()` de la variante.
2. **Registro + observabilidad**. Los 5xx disparan `tracing::error!` y despachan `ErrorOccurred`; los 4xx disparan `tracing::warn!`. Ambos llevan el id de solicitud cuando hay uno en alcance.
3. **Renderizado del cuerpo**. Un cuerpo JSON con la forma de Laravel, sanitizado para los 5xx.

### La forma del cuerpo

Todos los cuerpos de error siguen el mismo esqueleto JSON:

```json
{
    "message": "<human readable>",
    "errors": { "field": ["msg", ...] },
    "request_id": "<uuid>" | null,
    "debug_message": "<dev only>"
}
```

- `message` siempre está presente.
- `errors` solo aparece para errores de tipo validación (`Validation`, `ValidationError`) - ambos renderizan la misma forma para que los consumidores analicen una sola ruta.
- `request_id` siempre aparece (`null` cuando está fuera de un alcance de solicitud - por ejemplo, durante el arranque temprano o en tests sin contexto de solicitud).
- `debug_message` solo aparece para 5xx cuando `APP_DEBUG=true`. Es estrictamente aditivo - los clientes de producción no deben acoplarse a él.

### La regla de sanitización para 5xx

Esta es la garantía de seguridad que vale la pena memorizar. Para
cualquier error con estado ≥ 500, el `message` del cuerpo JSON se
reemplaza con la cadena literal:

```json
{ "message": "Internal Server Error", "request_id": "..." }
```

El detalle crudo del error **no** se filtra al cuerpo de la respuesta.
Va a:

- la entrada de registro de `tracing::error!`, con el id de solicitud y el estado
- el evento `ErrorOccurred`, que cualquier oyente puede capturar

Cuando `APP_DEBUG=true` (falso por defecto fuera de
`local`/`dev`/`test`), la respuesta también lleva un campo
`debug_message` con el detalle crudo - pero `message` se mantiene
genérico en ambos modos, de modo que los frontends y los clientes no
puedan acoplarse accidentalmente a datos exclusivos de desarrollo.

Este es el contrato que te permite llamar a
`FrameworkError::internal("db connection refused: password mismatch on user 'app_rw'")`
sin filtrar la contraseña en la respuesta. El `message` que pasas es
para los operadores que leen los registros; el `message` que ve el
cliente es `"Internal Server Error"`.

Para errores 4xx, el mensaje de cara al llamador se conserva -
`404 User not found`, `400 Missing required parameter: user_id`. Estos
son errores de dominio sobre los que el cliente necesita actuar, no
fallos internos.

### Dónde vive el contrato

Toda la conversión es una sola función -
`impl From<FrameworkError> for HttpResponse` en
`framework/src/http/response.rs`. Léela una vez y habrás leído toda la
superficie de renderizado de errores de Suprnova. No hay otra ruta.

## El límite de pánico

Un pánico en un middleware o en un handler de otro modo se propagaría
hacia arriba por la tarea de la conexión y derribaría el servicio
hyper a mitad de la respuesta, dejando al cliente con un reset TCP y
ninguna respuesta HTTP. Suprnova lo captura.

`execute_chain_safely` en `framework/src/server.rs` envuelve la cadena
de middleware en `AssertUnwindSafe(...).catch_unwind().await`. Ante un
pánico:

1. Extrae el payload del pánico (maneja payloads `&'static str` y `String`; cualquier otra cosa se muestra como `"panic with non-string payload"`).
2. Registra `tracing::error!` con el método, la ruta y el id de la solicitud.
3. Construye `FrameworkError::internal(format!("request handler panicked: {msg}"))` y la enruta a través de la *misma* conversión `From<FrameworkError> for HttpResponse` que usa cualquier otro 5xx.
4. Devuelve el id de la solicitud como `X-Request-Id`.

El payload del pánico permanece en la entrada de registro; el cliente
recibe el cuerpo sanitizado `{"message": "Internal Server Error"}`.
Los oyentes de observabilidad que se disparan con `ErrorOccurred` para
los 5xx devueltos también se disparan ante los pánicos - no hay una
superficie de eventos de pánico separada que conectar.

El mismo patrón de recuperación de pánico lo usan:

- Los handlers de WebSocket (`framework/src/server.rs`)
- Las tareas programadas (`framework/src/schedule/mod.rs`)
- Los workflows (`framework/src/workflow/mod.rs`)
- El trait `Supervisor` (difusión)

Un pánico en uno de estos subsistemas se registra y se traduce a un
estado de error o se reinicia automáticamente; no derriba la tarea del
worker.

## Enganchando la observabilidad con `ErrorOccurred`

`ErrorOccurred` es un evento integrado que el framework despacha en
cada respuesta 5xx (incluidas las sintetizadas a partir de pánicos):

```rust
pub struct ErrorOccurred {
    pub error_message: String,
    pub status_code: u16,
    pub request_id: Option<String>,
}
```

Escúchalo de la misma forma en que escuchas cualquier evento:

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
EventFacade::listen::<ErrorOccurred, _>(Arc::new(SentryReporter)).await;
```

Este es el equivalente en Suprnova del callback `report()` de Laravel
en el handler de excepciones global. El evento llega con el
`error_message` original sin sanitizar (el cuerpo que ve el cliente
sigue sanitizado), el código de estado, y el id de solicitud
correlacionable.

## Ayudantes de aborto

Tres funciones libres cortocircuitan un handler en un estado dado.
Reflejan `abort` / `abort_if` / `abort_unless` de Laravel:

```rust
use suprnova::{abort_with, abort_if, abort_unless, Auth, Request, Response, json_response};

pub async fn show(req: Request) -> Response {
    abort_unless(Auth::check(), 401, "must be logged in")?;
    abort_if(req.param("id")? == "0", 404, "User not found")?;
    abort_with(503, "scheduled maintenance")?;
    Ok(json_response!({ "ok": true }))
}
```

Cada una devuelve `Result<(), FrameworkError>`. Úsalas con `?`. El
error subyacente es `FrameworkError::Domain { message, status_code }`,
de modo que se renderiza con la misma forma de cuerpo y las mismas
reglas de sanitización que cualquier otro error. Los códigos de estado
fuera de rango se coercionan a 500 mediante la validación de estado
del renderizador de respuestas; no necesitas defenderte de entradas
incorrectas en el sitio de la llamada.

## El centinela de la CLI: `AlreadyReported`

Una variante de `FrameworkError` no tiene significado HTTP.
`AlreadyReported` se construye mediante `FrameworkError::silent()` y
la usa el despachador de la consola cuando clap ya formateó e imprimió
su propio error de análisis de argumentos. El `main` del binario
traduce el centinela a un código de salida distinto de cero sin
`eprintln`, de modo que los usuarios nunca ven dos mensajes de error
para el mismo fallo.

Si `AlreadyReported` llega alguna vez a un conversor de respuesta
HTTP, indica que un handler de solicitud devolvió `silent()` por
accidente. El conversor registra un `tracing::error!` llamativo
identificando la fuga y devuelve un 500 genérico - la variante no
tiene nada que hacer en la ruta de solicitud, y el registro llamativo
hace observable el bug en lugar de silencioso.

Normalmente no ves esta variante; está documentada aquí porque el enum
es `HTTP-flavoured` y la variante, de otro modo inexplicada, dejaría
perplejo a cualquiera que lea el código fuente.

## Garantías de seguridad, en resumen

El contrato que te da Suprnova:

- **Conversión total**. Cada `FrameworkError` produce un `HttpResponse`. No hay ninguna ruta de error que colapse el servidor o descarte la conexión en silencio.
- **5xx sanitizados**. El cuerpo de la respuesta para cualquier 5xx es el genérico `{"message": "Internal Server Error", "request_id": "..."}`. El detalle fluye a los registros + `ErrorOccurred`.
- **Visibilidad de depuración opcional**. `APP_DEBUG=true` añade un campo `debug_message` para los 5xx, nunca `message`. Los clientes de producción no pueden acoplarse accidentalmente a datos exclusivos de desarrollo.
- **Ids de solicitud correlacionables**. Todo cuerpo de error lleva el id de solicitud (o `null` cuando no existe un alcance de solicitud); el mismo id aparece en la línea de registro y en el evento `ErrorOccurred`.
- **Recuperación de pánico**. Los pánicos en handlers y middleware se capturan, se registran y se enrutan a través de la misma implementación de `From` que un error devuelto. Sin caída de la conexión, sin brecha de observabilidad.
- **Una forma para todo**. Los errores de validación, los errores de parámetro, los pánicos, los errores de dominio personalizados y los fallos de almacenamiento colapsan todos en el mismo esqueleto JSON. El código del frontend analiza una sola estructura.

## Dónde vive cada pieza

| Pieza | Archivo |
|---|---|
| `FrameworkError`, `AppError`, `HttpError`, `ValidationErrors` | `framework/src/error.rs` |
| `From<FrameworkError> for HttpResponse` (conversión + sanitización) | `framework/src/http/response.rs` |
| `abort`, `abort_if`, `abort_unless` | `framework/src/http/abort.rs` |
| `execute_chain_safely` (límite de pánico) | `framework/src/server.rs` |
| Evento `ErrorOccurred` | `framework/src/events/builtins.rs` |
| Macro `#[domain_error]` | `suprnova-macros/src/domain_error.rs` |

## Siguiente

- [Manejo de errores](errors.md) - los patrones prácticos de handler que usan este modelo
- [Ciclo de vida de la solicitud](lifecycle.md) - en qué punto del flujo de la solicitud se ejecuta la conversión de errores
- [Validación](validation.md) - `#[derive(Validate)]`, solicitudes de formulario, y cómo se puebla `ValidationErrors`
- [Respuestas](responses.md) - constructores de `HttpResponse`, encabezados, cookies, streaming
- [Eventos](events.md) - escuchar `ErrorOccurred` y otros eventos integrados
