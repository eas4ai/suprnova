# Macros

Suprnova incluye alrededor de tres docenas de macros, todas
reexportadas desde `suprnova::*`. Son las articulaciones donde el
framework se encuentra con tu código - `routes!` construye el router,
`#[handler]` adapta una función para convertirla en uno,
`#[suprnova::model]` convierte un struct en un modelo Eloquent,
`#[derive(Data)]` produce un payload tipado de Inertia. Este capítulo
es el índice. Cada macro recibe una descripción de un párrafo, un
ejemplo mínimo, y una referencia al capítulo que la usa para trabajo
real.

Algunos principios que se mantienen en toda la superficie:

- **Las macros emiten rutas totalmente calificadas.** El código generado escribe `::suprnova::…`, de modo que las macros funcionan hayas importado o no los tipos subyacentes.
- **Uso intensivo de `inventory::submit!`.** Modelos, comandos, políticas, observadores, proveedores de pago y más se registran a sí mismos en tiempo de compilación, y el framework drena el registro en el arranque. Casi nunca cableas el registro a mano.
- **Validación en tiempo de compilación donde vale la pena.** `inertia_response!` comprueba que el archivo de componente nombrado exista. `redirect!` comprueba que la ruta nombrada exista. `routes!` rechaza rutas que no empiecen con `/`. Los errores que se pueden detectar en tiempo de compilación, se detectan.

## Enrutamiento

| Macro | Devuelve | Qué hace |
|---|---|---|
| `routes!` | `pub fn register() -> Router` | Lista de rutas de nivel superior - exporta un `register()` que tu `app.rs` llama |
| `get!` / `post!` / `put!` / `delete!` / `patch!` / `head!` / `options!` / `any!` | `RouteDefBuilder<H>` | Una ruta HTTP - encadenable `.name(...)` / `.middleware(...)` |
| `group!` | `GroupDef` | Prefijo + middleware aplicado a una lista hija de rutas |
| `fallback!` | `FallbackDefBuilder<H>` | Handler 404 personalizado cuando ninguna ruta coincide |
| `ws!` | `WsRouteDef` | Una ruta WebSocket - encadenable `.middleware(...)` / `.config(...)` |

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

La cadena de la ruta se comprueba en tiempo de compilación -
`validate_route_path` rechaza cualquier cosa que no empiece con `/`.
Los nombres de ruta registrados mediante `.name("…")` también se
comprueban por unicidad en el arranque a través de
`register_route_name`. Consulta [Enrutamiento](routing.md) para la
expansión completa y [WebSockets](websockets.md) para `ws!`.

## Handlers y solicitudes

### `#[handler]`

Reescribe una función de controlador para que pueda extraer parámetros
tipados (mediante `FromRequest`) directamente de la solicitud entrante -
en lugar de extraer campos manualmente de `Request`, declaras lo que
el handler necesita y la macro lo cablea.

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
    // `form` ya está validado - se devuelve 422 automáticamente ante un fallo
    json_response!({ "email": form.email })
}
```

Un primer parámetro con forma `Request` sigue aceptándose como el caso
identidad. Consulta [Controladores](controllers.md).

### `#[request]` y `#[derive(FormRequest)]`

`#[request]` es la forma recomendada de declarar un tipo de solicitud
validado. Deriva automáticamente `Deserialize`, `Validate` y
`FormRequest`, de modo que el struct funciona tanto con cuerpos
`application/json` como `application/x-www-form-urlencoded`.

`#[derive(FormRequestDerive)]` es el derive subyacente si quieres
prescindir del atributo (tendrás que derivar `Deserialize` y
`Validate` tú mismo). El atributo es lo que recomendamos; el derive
existe para el caso extremo. Consulta [Solicitudes](requests.md) y
[Validación](validation.md).

### `#[derive(MultipartRequest)]`

Extractor fuertemente tipado para `multipart/form-data` - vincula
campos de texto y archivos subidos en un solo struct, con validadores
a nivel de tipo por campo.

```rust
use suprnova::{MultipartRequest};
use suprnova::http::upload::{Image, MaxSize, UploadedFile};

#[derive(MultipartRequest)]
pub struct AvatarUpload {
    #[field("avatar")]
    pub avatar: UploadedFile<(Image, MaxSize<5_242_880>)>,

    #[field("caption")]
    pub caption: Option<String>,
}
```

Los validadores integrados (`Image`, `MimeAllowlist<…>`, `MaxSize<…>`,
`MimeType<…>`) se componen mediante tuplas. Consulta
[Solicitudes](requests.md).

## Respuestas

### `json_response!` y `text_response!`

Las dos macros de respuesta abreviadas. Ambas envuelven
`HttpResponse::*` en `Ok(...)` para encajar directamente en la
posición de retorno de un handler:

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

Consulta [Respuestas](responses.md).

### `inertia_response!`

Construye una respuesta de página de Inertia, validando en tiempo de
compilación que el archivo de componente nombrado (`.svelte` / `.tsx`
/ `.jsx` / `.vue`) exista en `frontend/src/pages/`. Si escribes mal el
nombre del componente, la compilación falla con sugerencias:

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

`#[derive(InertiaProps)]` genera la implementación de `Serialize` que
la forma de la respuesta necesita. Consulta [Respuestas de
Inertia](frontend-inertia-responses.md).

### `redirect!`

Redirección con tipos seguros hacia una ruta nombrada - el nombre de
la ruta se verifica en tiempo de compilación contra los nombres
registrados mediante `routes!`:

```rust
use suprnova::redirect;

// Solo compila si "users.show" es un nombre de ruta registrado
let resp = redirect!("users.show").with("id", "42").into();
```

Consulta [Generación de URLs](urls.md).

## Eloquent

### `#[suprnova::model]`

Convierte un struct simple en un modelo Eloquent completo: genera los
stubs de SeaORM `Entity`, `Model`, `ActiveModel`, `Column`,
`Relation`, además de todas las implementaciones de trait que Eloquent
necesita. También hace `inventory::submit!` de una `ModelEntry` para
que el framework pueda enumerar cada modelo en el arranque.

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

Las claves de atributo incluyen `table`, `primary_key`, `key_type`,
`auto_increment`, `connection`, `fillable`, `guarded`, `casts`,
`timestamps`, `soft_deletes`, `appends`, `hidden`, `visible`,
`mutators`, `touches`, y `unique_id` (para PKs UUID/ULID). Consulta
[Eloquent](eloquent.md).

### `#[suprnova::scopes(Model)]`

Recorre un bloque `impl Model { … }` y convierte cada método cuya
firma coincida con
`fn name(query: Builder<Self>[, args…]) -> Builder<Self>` en un scope -
generando tanto `Model::scope_name(args)` como un `.scope_name(args)`
encadenable en `Builder<Model>`.

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

    // No es un scope - pasa sin cambios
    pub fn display_name(&self) -> String { self.name.clone() }
}

// Ambos sitios de llamada compilan:
// User::active().popular(500).get().await?;
// User::query().filter_op("id", ">", 0).active().get().await?;
```

La forma encadenable requiere que el trait generado
`HasScope_<scope>_<Model>` esté en scope cuando se llama desde un
módulo distinto. Consulta [Eloquent](eloquent.md).

### `#[suprnova::observer(Model)]`

Cablea un bloque `impl Observer<M>` al sistema de eventos de ciclo de
vida - cada uno de los 16 métodos sobrescritos se convierte en un
oyente registrado, enviado al inventario y drenado en el arranque.

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

**Orden de atributos obligatorio: `#[suprnova::observer(M)]` debe ir
antes que `#[async_trait]`.** Las macros de atributo se expanden de
afuera hacia adentro - si `async_trait` se ejecuta primero, reescribe
cada `async fn` en una forma sin azúcar sintáctico y la coincidencia
por nombre de la macro observer contra los 16 nombres de método del
trait no encuentra nada, en silencio. Consulta [Eventos](events.md).

### `#[suprnova::accessor]` y `#[suprnova::mutator]`

Marcadores a nivel de función sobre métodos de `impl Model { … }` que
se enganchan a las rutas `to_json()` / `fill()` del modelo. Referencia
el nombre del campo en `#[model(appends = […])]` (accessor) o
`#[model(mutators = […])]` (mutator) para que la macro los cablee.

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

Consulta [Conversiones, accesores y mutadores de
Eloquent](eloquent-mutators.md).

### `#[suprnova::prunable]`

Envuelve una implementación de `Prunable` (o `MassPrunable`) y envía
una `PrunerEntry` al registro que `model:prune` recorre en tiempo de
ejecución:

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

Consulta [Eloquent](eloquent.md).

### `attrs!`

Construye un mapa `Attrs` ordenado
(`IndexMap<&'static str, serde_json::Value>`) para `Model::create` /
`Model::update` / `Model::fill`:

```rust
use suprnova::attrs;

let user = User::create(attrs! {
    name: "Alice",
    email: "alice@example.com",
    age: 32,
}).await?;
```

Consulta [Eloquent](eloquent.md).

### `casts!`

Construye un mapa de conversiones por consulta que puedes pasar a
`Builder::with_casts`:

```rust
use suprnova::{casts, AsDate, AsJson};

let map = casts! {
    birthday = AsDate,
    metadata = AsJson<serde_json::Value>,
};
let rows = User::query().with_casts(map).get().await?;
```

Consulta [Mutadores y conversiones](eloquent-mutators.md).

### `route_binding!`

Implementa `RouteBinding` para una entidad de SeaORM escrita a mano,
de modo que se resuelva automáticamente desde un parámetro de ruta.
Los modelos definidos con `#[suprnova::model]` se registran
automáticamente y no necesitan esto; recurre a `route_binding!` cuando
hayas escrito la entidad a mano:

```rust
use suprnova::route_binding;

route_binding!(crate::entities::user::Entity, User, "user");
```

Después de eso, `get!("/users/{user}", controllers::user::show)` pasa
un `User` completamente cargado a tu handler. Consulta
[Enrutamiento](routing.md).

## Datos e Inertia

### `#[derive(Data)]`

El derive compuesto para payloads tipados. Produce una implementación
de `Serialize` que respeta los campos `#[data(input_only)]`, más una
implementación de `Deserialize` que rechaza los payloads que intenten
establecer campos `#[data(output_only)]`. Combínalo con
`#[json_resource("type")]` para la salida JSON:API a través del
capítulo `Resource`.

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

`#[data(allow_include)]` registra el campo en la lista de inclusión
permitida para recargas parciales mediante `inventory::submit!`.
Consulta [Objetos de datos](data.md) y [Recursos
JSON:API](eloquent-resources.md).

### `#[derive(InertiaProps)]`

Genera la implementación de `Serialize` que necesita
`inertia_response!`. Un derive marcador simple - la mayoría de las
aplicaciones recurre en su lugar a `#[derive(Data)]` porque da
recargas parciales incluidas gratis.

```rust
use suprnova::InertiaProps;

#[derive(InertiaProps)]
struct DashboardProps {
    title: String,
    user: User,
}
```

Consulta [Respuestas de Inertia](frontend-inertia-responses.md).

### `when_loaded!`

Emite un `Prop::lazy(…)` solo cuando una relación nombrada se ha
cargado de forma anticipada en la entidad; de lo contrario emite
`Prop::absent()` para que la prop se omita por completo de la
respuesta:

```rust
use suprnova::when_loaded;

let songs_prop = when_loaded!(&artist, "songs", || async {
    serde_json::to_value(&artist.songs).unwrap()
});
```

Consulta [Objetos de datos](data.md).

## Inyección de dependencias

### `#[service]`

Añade `Send + Sync + 'static` a un trait para que encaje en el
contenedor:

```rust
use suprnova::service;

#[service]
pub trait HttpClient {
    async fn get(&self, url: &str) -> Result<String, FrameworkError>;
}

// App::bind::<dyn HttpClient>(Arc::new(RealHttpClient::new()));
// let client = App::make::<dyn HttpClient>()?;
```

Consulta [Contenedor de servicios](container.md).

### `#[injectable]`

Registra automáticamente un tipo concreto como singleton. Deriva
`Default` + `Clone` y envía un registro que se ejecuta en el arranque:

```rust
use suprnova::injectable;

#[injectable]
pub struct AppState {
    pub counter: u32,
}

// let state: AppState = App::get().unwrap();
```

Consulta [Contenedor de servicios](container.md).

## Errores

### `#[domain_error]`

Define un error de dominio que implementa `Display`, `Error`,
`HttpError`, y `From<T> for FrameworkError` - de modo que
cortocircuita un handler mediante `?`:

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

Consulta [Manejo de errores](errors.md).

## Consola y trabajo en segundo plano

### `#[command]`

Marca una `async fn(Vec<String>) -> Result<(), FrameworkError>` como
un comando de consola. Envía una `CommandEntry` para que
`dispatch_argv` la encuentre cuando se ejecuta el binario de consola
de cada proyecto:

```rust
use suprnova::{command, FrameworkError};

#[command(name = "db:seed", description = "Run all registered seeders")]
async fn db_seed(_args: Vec<String>) -> Result<(), FrameworkError> {
    suprnova::seed::run_all().await
}
```

Consulta [Consola](console.md).

### `#[derive(Command)]`

La alternativa de argumentos tipados. Se coloca encima de
`#[derive(clap::Parser)]`, lee `#[console(...)]` para los metadatos, y
emite el ejecutor que llama a tu `TypedCommand::run`:

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

Consulta [Consola](console.md).

### `#[workflow]` y `#[workflow_step]`

`#[workflow]` registra una fn async como un workflow duradero - estado
ejecutable, pasos reintentables, historial persistido. Cada
`#[workflow_step]` dentro del cuerpo es un checkpoint desde el que el
runtime puede reanudar después de una caída o un reinicio.

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

Pone en marcha un workflow por ruta, serializando los argumentos en la
forma de sobre del runtime de workflows:

```rust
use suprnova::start_workflow;

let handle = start_workflow!(crate::workflows::onboard_user, 42).await?;
```

Consulta [Flujos de trabajo](workflows.md).

### `schedule_task!`

Azúcar sintáctico alrededor de `TaskBuilder::from_async` para que un
closure se programe limpiamente junto a implementaciones de `Task`
basadas en trait:

```rust
use suprnova::{schedule_task, FrameworkError};

let task = schedule_task!(|| async {
    println!("ticking");
    Ok::<(), FrameworkError>(())
})
    .every_minute()
    .name("tick");
```

Consulta [Programación de tareas](scheduling.md).

## Autorización

### `#[policy(UserType, ResourceType)]`

Envuelve un bloque `impl Policy` y registra cada método como una
acción de compuerta con nombre. El nombre de la compuerta combina el
nombre del método con el tipo de recurso en minúsculas -
`fn view(...)` en `Comment` se convierte en `"view-comment"`:

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

`Server::run` llama a `authorization::init_policies()`
automáticamente. Consulta [Autorización](authorization.md).

## Notificaciones y correo

### `#[derive(NotificationMailable)]`

Genera automáticamente `to_mail` a partir de un atributo
`#[mail(...)]` - plantillas Tera en línea o respaldadas por archivo
para el asunto, el cuerpo HTML y el cuerpo de texto. Comprobaciones en
tiempo de compilación: el asunto es obligatorio, debe haber al menos
un cuerpo presente, html/html_template son exclusivos entre sí,
`from_name` requiere `from`:

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

El propio trait de notificación se implementa a mano - no existe un
`#[derive(Notification)]`. Consulta [Notificaciones](notifications.md)
y [Correo](mail.md).

## Validación

### `validate!`

Punto de entrada de validación síncrono y declarativo. Cada fila
empareja un nombre de campo con uno o más valores `Rule` (o
`ContextualRule`), con `?:` para "validar solo si está presente" y
`?=>` para campos opcionales condicionalmente requeridos:

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

`Validate` se reexporta desde el crate `validator` - los atributos
`#[validate(...)]` (p. ej. `#[validate(email)]`) vienen de `validator`
y se ejecutan a través de la ruta síncrona de `FormRequest`. Usa
`validate!` cuando necesites reglas contextuales / entre campos,
reglas asíncronas, o reglas de la paleta
`suprnova::validation::rules`. Consulta [Validación](validation.md).

## Fábricas

### `#[derive(Factory)]`

Genera un marcador hermano `<Model>Factory` y una implementación de
`Factory` que produce modelos mediante `fake::Faker`. El modelo debe
implementar `fake::Dummy<fake::Faker>` - típicamente mediante
`#[derive(Dummy)]`:

```rust
use suprnova::{Dummy, Factory};

#[derive(Dummy, Factory)]
pub struct User {
    pub id: i32,
    pub name: String,
    pub email: String,
}

// UserFactory existe:
let users = UserFactory::new().count(10).make_many();
```

Consulta [Fábricas de Eloquent](eloquent-factories.md).

## Pruebas

### `#[suprnova_test]`

Envuelve un test `async fn` con una base de datos SQLite en memoria
(ejecutando `crate::migrations::Migrator` por defecto), invoca
`App::init()` y `App::boot_services()`, y ejecuta el cuerpo bajo
`#[tokio::test]`. Los tests en paralelo se mantienen herméticos
gracias a la capa por hilo del contenedor - vincula servicios
específicos del test mediante `TestContainer::fake` (no `App::bind`)
para que cada hilo vea sus propios fakes:

```rust
use suprnova::suprnova_test;
use suprnova::testing::TestDatabase;

#[suprnova_test]
async fn creates_a_user(db: TestDatabase) {
    let user = User::create(attrs! { name: "A", email: "a@x.com" }).await.unwrap();
    assert!(user.id > 0);
}
```

Un migrador personalizado se indica mediante
`#[suprnova_test(migrator = MyMigrator)]`. Consulta
[Pruebas](testing.md).

### `test_database!`

El constructor de una línea de `TestDatabase` para tests que no
reciben el parámetro `db` a través de `#[suprnova_test]`:

```rust
let db = test_database!();
let db = test_database!(my_crate::CustomMigrator);
```

### `describe!`, `test!`, `expect!`

Agrupación al estilo Jest + aserciones fluidas. `describe!` es un
módulo, `test!` produce un `#[test]` (síncrono o asíncrono, con o sin
un parámetro `TestDatabase`), y `expect!` envuelve un valor para
aserciones encadenadas con contexto de archivo/línea ante un fallo:

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

Consulta [Pruebas](testing.md).

## Middleware

### `global_middleware!`

Registra un middleware que se ejecuta en cada solicitud, en el orden
de registro, antes de cualquier middleware específico de ruta.
Idempotente por tipo:

```rust
use suprnova::global_middleware;
use crate::middleware;

pub fn register() {
    global_middleware!(middleware::LoggingMiddleware);
    global_middleware!(middleware::CorsMiddleware);
}
```

Debe ejecutarse antes de `Server::from_config` / `Server::new` - el
servidor toma una instantánea del registro global en el momento de
construcción. Consulta [Middleware](middleware.md).

## Trampas

Una lista breve de modos de fallo fáciles de encontrar y fáciles de
arreglar.

### Orden de atributos - `#[observer]` debe ir antes que `#[async_trait]`

```rust
// CORRECTO
#[suprnova::observer(User)]
#[async_trait]
impl Observer<User> for AuditObserver { … }

// INCORRECTO - emite silenciosamente cero oyentes
#[async_trait]
#[suprnova::observer(User)]
impl Observer<User> for AuditObserver { … }
```

Las macros de atributo se expanden de afuera hacia adentro.
`async_trait` reescribe cada `async fn` en una forma sin azúcar
sintáctico `Pin<Box<dyn Future>>`. Si se ejecuta primero, la macro
observer ya no puede hacer coincidir por nombre de método y no emite
nada. La misma regla de afuera hacia adentro aplica siempre que apiles
varias macros - coloca el atributo de Suprnova en la posición más
externa cuando tengas dudas.

### La trampa del impl inherente

Un método `impl` inherente **no puede** ensombrecer el método por
defecto de un trait a través del despacho de trait. Si escribes una
macro (o código a mano) que define `fn save(&self)` sobre un modelo
como método inherente, las llamadas que pasan por el trait `Model`
(`some_model.save()` donde el sitio de llamada solo lo conoce como
`&dyn Model`) elegirán el valor por defecto del trait - no tu
sobrescritura inherente.

Arreglo: emite una sobrescritura de método de trait, nunca un método
inherente, cuando el comportamiento generado deba participar en el
despacho de trait. Por eso las macros del framework (en particular
`#[suprnova::model]`) escriben en la implementación del trait. Si
estás construyendo extensiones de Eloquent a mano, haz lo mismo.

### `global_middleware!` solo tiene efecto antes de `Server::from_config`

El servidor toma una instantánea del registro global cuando se
construye. Llamar a `global_middleware!(M)` después de
`Server::from_config(...)` no se aplica retroactivamente a ese
servidor. Registra todo el middleware global en `bootstrap()`, antes
de que `Application::run()` llegue al paso de servir.

### `redirect!` e `inertia_response!` son comprobaciones en tiempo de compilación

Ambas macros se niegan a compilar si el objetivo nombrado no existe -
ese es el punto. Si una refactorización elimina el nombre de una ruta
o de un componente, cada sitio de llamada que lo mencione rompe la
compilación, que es exactamente lo que quieres. Si el error de
compilación te sorprende, busca el literal de cadena en tu bloque
`routes!` / directorio de páginas antes de "arreglar" la llamada a la
macro.

### `?:` se salta en `None`; `?=>` se ejecuta incluso en `None`

En las filas de `validate!`, `?:` solo ejecuta las reglas cuando el
campo es `Some`. Por lo tanto, una regla condicionada a la presencia
como `RequiredIf` en una fila `?:` nunca puede fallar sobre un campo
ausente. Usa `?=>` (que trata la ausencia como `""`) para el caso de
"requerido cuando X".

### `#[derive(Validate)]` viene del crate `validator`, no de Suprnova

Suprnova reexporta `validator::Validate` para que no tomes una
dependencia directa de `validator`. Los atributos `#[validate(...)]`
vienen de `validator`. La propia macro `validate!` de Suprnova es el
punto de entrada en tiempo de ejecución para reglas entre campos /
contextuales; ambas se complementan pero viven en espacios de nombres
distintos.

## Por qué Suprnova diverge

Laravel descubre rutas, comandos, plantillas de correo, clases de
modelo, fábricas, observadores y políticas en tiempo de ejecución -
mediante reflexión, escaneo del sistema de archivos y despacho basado
en cadenas. PHP hace esto barato (el autoloading + opcache amortiguan
el costo), y la experiencia de desarrollo es excelente: sueltas un
archivo en el directorio correcto y aparece.

Ese modelo no encaja con Rust. No tenemos reflexión en tiempo de
ejecución sobre implementaciones de trait, el runtime es un único
binario enlazado estáticamente, y los escaneos del sistema de archivos
en el arranque encajan peor con un modelo de proceso donde cada
binario sirve millones de solicitudes.

Así que Suprnova hace el mismo trabajo en tiempo de compilación. Las
rutas se validan, los nombres de componente se comprueban contra el
directorio de páginas, las plantillas de correo se incrustan mediante
`include_str!`, los nombres de ruta se comprueban por unicidad a
través del inventario, los modelos se registran a sí mismos en un
inventario que el framework drena en el arranque, y los comandos
igual. La experiencia de desarrollo es similar - sueltas un archivo,
añades un `#[command]` o `#[suprnova::model]`, ejecutas el binario -
pero el cableado ocurre antes de `main` en lugar de en la primera
solicitud.

El costo es que los errores tipográficos, los componentes faltantes y
las referencias rotas son errores de compilación en lugar de errores
en tiempo de ejecución, y no hay ningún costo de reflexión por
solicitud.

## Siguiente

- [Enrutamiento](routing.md) - la expansión completa de `routes!`, el nombrado, la vinculación de modelos
- [Controladores](controllers.md) - `#[handler]` y `#[request]` juntos
- [Eloquent](eloquent.md) - `#[suprnova::model]` y compañía en contexto
- [Validación](validation.md) - `validate!`, reglas contextuales, reglas asíncronas
- [Consola](console.md) - `#[command]` y `#[derive(Command)]` de principio a fin
- [Pruebas](testing.md) - `#[suprnova_test]`, `expect!`, fakes
