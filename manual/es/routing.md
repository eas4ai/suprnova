# Enrutamiento

El enrutamiento es la forma en que Suprnova convierte una solicitud HTTP
entrante en una llamada a un handler. Las rutas se declaran en
`src/routes.rs` mediante la macro `routes!` (o construyendo un `Router` a
mano), y luego `Server::from_config` toma ese router y lo ejecuta durante
toda la vida del proceso. La misma forma que `routes/web.php` de Laravel,
con tipos de Rust en lugar de facades.

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

La macro se expande a `pub fn register() -> Router { ... }`. Llámala desde
tu bootstrap y entrega el resultado al servidor.

## Verbos HTTP

Una macro por verbo. Las siete reciben un par ruta-handler y devuelven un
builder al que se le pueden encadenar `.name(...)` y `.middleware(...)`.

| Macro | Método | Uso |
|---|---|---|
| `get!`     | GET     | Endpoints de lectura, páginas estáticas |
| `post!`    | POST    | Crear recursos |
| `put!`     | PUT     | Actualizaciones de reemplazo completo |
| `patch!`   | PATCH   | Actualizaciones parciales (RFC 5789) |
| `delete!`  | DELETE  | Eliminar |
| `head!`    | HEAD    | Sondeos solo de encabezados (HEAD recurre al registro de GET según RFC 9110 § 9.3.2 cuando no está registrado explícitamente) |
| `options!` | OPTIONS | Descubrimiento de capacidades, `Accept-Patch`. El preflight de CORS lo responde `CorsMiddleware` antes del router, así que normalmente no hace falta este |

```rust
use suprnova::{routes, get, post, patch, delete};

routes! {
    get!("/articles", controllers::articles::index),
    post!("/articles", controllers::articles::store),
    patch!("/articles/{id}", controllers::articles::update),
    delete!("/articles/{id}", controllers::articles::destroy),
}
```

Cada macro de verbo comprueba en tiempo de compilación que la ruta empiece
con `/` - la falta de una barra inicial hace fallar la build, no una
solicitud.

### Múltiples métodos y `any!`

`any!` registra un handler para los siete verbos comunes. Úsala para
receptores de webhooks y otros endpoints que necesiten aceptar lo que sea
que envíe HTTP.

```rust
use suprnova::{routes, any};

routes! {
    any!("/webhooks/inbound", controllers::webhooks::inbound)
        .name("webhooks.inbound")
        .middleware(SignatureCheck),
}
```

Cuando solo se necesite un subconjunto de verbos compartiendo un handler,
recurre a la API de builder y a `Router::methods`:

```rust
use suprnova::Router;
use hyper::Method;

let router = Router::new()
    .methods(&[Method::PUT, Method::PATCH], "/posts/{id}", update_post)
    .name("posts.update")
    .middleware(AuthMiddleware);
```

`.name(...)` y `.middleware(...)` se propagan a través de todos los verbos
con los que se registró la ruta, de modo que la búsqueda inversa produce la
misma URL sin importar qué método consulte quien llama.

### Rutas WebSocket

`ws!` registra un handler de actualización (upgrade) de larga duración. La
macro es parte del mismo cuerpo `routes!` - cubierta en detalle por
[WebSockets](websockets.md).

## Parámetros de ruta

Los segmentos dinámicos usan llaves (`{id}`). Por familiaridad, Suprnova
también acepta los dos puntos al estilo Express/Rails (`:id`) y los
normaliza a llaves antes de entregar el patrón a `matchit`.

```rust
routes! {
    get!("/users/{id}", controllers::users::show),       // nativo de matchit
    get!("/users/:id", controllers::users::show),        // Express/Rails - lo mismo
    get!("/posts/{post_id}/comments/{comment_id}", controllers::comments::show),
}
```

Los dos puntos solo se tratan como apertura de parámetro al comienzo de un
segmento de ruta, de modo que los dos puntos literales en medio de un
segmento sobreviven intactos (`/files/note:draft` sigue siendo una ruta
literal, no `/files/{draft}`).

Lee los parámetros de la solicitud dentro de un handler:

```rust
use suprnova::{Request, Response, HttpResponse};

pub async fn show(req: Request) -> Response {
    let user_id = req.param("id").unwrap_or("0");
    Ok(HttpResponse::text(format!("User ID: {}", user_id)))
}
```

Para una extracción tipada sin el baile del `unwrap_or`, consulta la
vinculación de modelo de ruta más abajo o `#[handler]` en
[Controladores](controllers.md).

## Vinculación de modelo de ruta

Cuando un parámetro de handler es un tipo `*::Model` de SeaORM, `#[handler]`
extrae el parámetro de ruta correspondiente, lo analiza como el tipo de la
clave primaria, y obtiene la fila de la base de datos. Una fila ausente
produce 404; un parámetro que el tipo de la PK no puede analizar produce
400.

```rust
use suprnova::{handler, json_response, Response};
use crate::models::users;

// Ruta: GET /users/{user}
#[handler]
pub async fn show(user: users::Model) -> Response {
    json_response!({ "name": user.name, "email": user.email })
}
```

El nombre del parámetro (`user`) es lo que `#[handler]` busca en los params
de la ruta coincidente - así que el placeholder debe coincidir
(`/users/{user}`, no `/users/{id}`).

Varios modelos en una misma firma funcionan igual; se pueden mezclar con
solicitudes de formulario, primitivos, o `Request`:

```rust
// Ruta: PUT /posts/{post}/comments/{comment}
#[handler]
pub async fn update(
    post: posts::Model,
    comment: comments::Model,
    form: UpdateCommentRequest,
) -> Response {
    // post y comment ya se obtuvieron; form ya está validado.
    json_response!({ "post_id": post.id, "comment_id": comment.id })
}
```

### Requisitos

La vinculación es automática para cualquier modelo de SeaORM cuya `Entity`
implemente `suprnova::database::EntityExt` y cuyo tipo de clave primaria
implemente `FromStr`. Los traits adicionales de `EntityExt`, diseñados para
admitir impls generales (blanket impls), dan `Entity::find_by_pk(id)`,
`::all()`, `::first()` y similares; la vinculación de modelo de ruta no es
más que un `find_by_pk` impulsado por el parámetro de ruta.

```rust
// src/models/users.rs (el layout heredado al estilo SeaORM)
pub use super::entities::users::*;
use sea_orm::entity::prelude::*;

impl ActiveModelBehavior for ActiveModel {}

// Habilita la vinculación de modelo de ruta (y la superficie de lectura con forma Laravel).
impl suprnova::database::EntityExt for Entity {}
impl suprnova::database::EntityExtMut for Entity {}
```

Si el modelo se declara con la macro `#[suprnova::model]` (la superficie
Eloquent en [API de Eloquent](eloquent.md)), se recurre a él directamente:
`User::find_by_pk(id).await?`. La vinculación de modelo de ruta vía
`#[handler]` sigue esperando la forma `*::Model` - pasa el tipo de modelo
de SeaORM, no el struct envoltorio.

### La vinculación es identidad, no autorización

La vinculación de modelo de ruta responde "¿existe esta fila?" - **no**
responde "¿puede el usuario actual ver esta fila?". Un handler vinculado
sin más deja que cualquier usuario autenticado vea cualquier post con solo
adivinar `/posts/N`. Autoriza contra el modelo vinculado usando
`Gate::authorize` o la macro `#[policy]` - consulta
[Autorización](authorization.md).

### Cómo no usarla

No uses el tipo de parámetro `*::Model`. Extrae el ID y consulta
manualmente:

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

## Rutas nombradas

Los nombres dan identificadores estables para la generación de URLs.
Adjunta uno con `.name(...)`:

```rust
routes! {
    get!("/", controllers::home::index).name("home"),
    get!("/users", controllers::users::index).name("users.index"),
    get!("/users/{id}", controllers::users::show).name("users.show"),
    post!("/users", controllers::users::store).name("users.store"),
}
```

Los nombres siguen la convención de Laravel `<resource>.<action>` -
`users.show`, `posts.destroy`, `admin.dashboard`. Búscalos con el ayudante
de nivel superior `route(name, &[...])`:

```rust
use suprnova::route;

let home = route("home", &[]);
//   Some("/")

let profile = route("users.show", &[("id", "123")]);
//   Some("/users/123")
```

`route` devuelve `Option<String>` y codifica los valores de los parámetros
en forma segura para URLs mediante percent-encoding (así que
`("slug", "a/b")` se convierte en `/posts/a%2Fb` - seguro para `matchit` y
hace round-trip a través de `req.param("slug")`). Para destinos de
redirección y enlaces de correo usa la contraparte estricta
`suprnova::routing::try_route`, que devuelve `Result<String, RouteUrlError>`
y se niega a emitir una URL que contenga un segmento `{placeholder}` sin
rellenar. Consulta [Generación de URLs](urls.md) para la superficie
completa de URLs (URLs firmadas, URLs absolutas, `Redirect::route`).

Los nombres de ruta son únicos a nivel global y de todo el proceso.
Registrar el mismo nombre para dos rutas distintas hace que el proceso
entre en pánico en el arranque - el ensombrecimiento silencioso era un bug con
forma de vulnerabilidad de seguridad, porque las redirecciones acababan
yendo hacia el registro que hubiera ganado la carrera. Usa
`RouteBuilder::try_name` (o `suprnova::routing::try_register_route_name`)
para la variante falible.

## Middleware por ruta

Encadena `.middleware(M)` en cualquier builder de ruta:

```rust
use suprnova::{routes, get, post};
use crate::middleware::{AuthMiddleware, AdminMiddleware};

routes! {
    // Público
    get!("/", controllers::home::index).name("home"),

    // Protegido
    get!("/dashboard", controllers::dashboard::index)
        .name("dashboard")
        .middleware(AuthMiddleware),

    // Varios middleware se componen de izquierda a derecha (el más externo primero)
    get!("/admin", controllers::admin::index)
        .middleware(AuthMiddleware)
        .middleware(AdminMiddleware),
}
```

El middleware local a la ruta se ejecuta después de cualquier middleware
global (`Server::with_middleware`) y de cualquier middleware de grupo que
envuelva la ruta. El mapa de middleware se indexa por `(method, path)`, de
modo que adjuntar auth a `POST /api/posts` nunca se filtra hacia un
`GET /api/posts` público en la misma ruta. Para el contrato de middleware
y cómo escribir el propio, consulta [Middleware](middleware.md).

## Grupos de rutas

`group!` factoriza un prefijo de ruta compartido y/o middleware compartido:

```rust
use suprnova::{routes, get, post, group};
use crate::middleware::{AuthMiddleware, ApiMiddleware};

routes! {
    get!("/", controllers::home::index).name("home"),

    // Prefijo /api compartido + middleware
    group!("/api", {
        get!("/users", controllers::api::users::index).name("api.users.index"),
        post!("/users", controllers::api::users::store).name("api.users.store"),
        get!("/users/{id}", controllers::api::users::show).name("api.users.show"),
    }).middleware(ApiMiddleware),

    // Área de administración
    group!("/admin", {
        get!("/dashboard", controllers::admin::dashboard).name("admin.dashboard"),
        get!("/settings", controllers::admin::settings).name("admin.settings"),
    }).middleware(AuthMiddleware),
}
```

Un prefijo de grupo se concatena con la ruta de cada endpoint del grupo.
Una ruta declarada como `/` dentro de un grupo se resuelve exactamente al
prefijo del grupo (`group!("/users", { get!("/", index) })` → `GET /users`).

### Grupos anidados

Los grupos se anidan a cualquier profundidad. Los prefijos se concatenan;
el middleware se hereda de padre a hijo:

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

| Ruta | Ruta efectiva | Cadena de middleware |
|---|---|---|
| `/api/health` | `/api/health` | `AuthMiddleware` |
| `/api/v1/users` | `/api/v1/users` | `AuthMiddleware` |
| `/api/v1/admin/stats` | `/api/v1/admin/stats` | `AuthMiddleware` → `AdminMiddleware` |

Para una única ruta dentro de un grupo anidado, el orden de ejecución es
**el middleware más externo primero**: grupo padre → grupo hijo → local a
la ruta. El `.middleware(...)` por ruta se ejecuta en la posición más
interna.

## Ruta de fallback

`fallback!` registra un handler que se ejecuta cuando ninguna otra ruta
coincide. Úsalo para páginas 404 personalizadas.

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

El fallback admite su propia cadena de middleware
(`fallback!(handler).middleware(M)`). Si no se registra ningún fallback, el
framework devuelve un `404 Not Found` en texto plano.

## Enrutamiento de recursos

Para una superficie REST estándar de 7 acciones, implementa
`ResourceController` y registra el recurso a través del builder `Router`.
Paridad con Laravel para `Route::resource()` y `Route::apiResource()`.

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
    // store / update / destroy / create / edit devuelven 404 por defecto.
}

let router: Router = Router::new()
    .resource("posts", PostsCtl)
    .into();
```

Los métodos que no se sobrescriben devuelven 404. Usa `api_resource` para
omitir `create` y `edit` - las dos rutas que existen solo para renderizar
formularios.

### Rutas y nombres por defecto

| Verbo | Ruta | Método del trait | Nombre |
|---|---|---|---|
| GET    | `/posts`             | `index`   | `posts.index`   |
| GET    | `/posts/create`      | `create`  | `posts.create`  |
| POST   | `/posts`             | `store`   | `posts.store`   |
| GET    | `/posts/{post}`      | `show`    | `posts.show`    |
| GET    | `/posts/{post}/edit` | `edit`    | `posts.edit`    |
| PUT    | `/posts/{post}`      | `update`  | `posts.update`  |
| DELETE | `/posts/{post}`      | `destroy` | `posts.destroy` |

El parámetro de ruta usa por defecto el singular del nombre del recurso -
`posts` → `{post}`, `categories` → `{category}`. Los plurales irregulares
obtienen el último segmento literal; anúlalo con `.parameter(...)`.

### Restringir y renombrar

```rust
use suprnova::{Router, ResourceAction};

Router::new()
    .resource("posts", PostsCtl)
    .only(&[ResourceAction::Index, ResourceAction::Show])      // fija a dos verbos
    .names([("index", "posts.list")])                          // renombra un valor por defecto
    .parameter("post_id")                                      // {post} → {post_id}
    .into();
```

Alias del lado de Rust que se leen mejor en algunos sitios de llamada:
`.keep(...)` para `.only(...)`, `.drop(...)` para `.except(...)`,
`.rename(...)` para `.names(...)`.

### Registro masivo

```rust
Router::new()
    .resources([
        ("posts",    Box::new(PostsCtl)    as Box<dyn ResourceController>),
        ("comments", Box::new(CommentsCtl) as Box<dyn ResourceController>),
    ])
    .api_resources([("authors", Box::new(AuthorsCtl) as Box<dyn ResourceController>)]);
```

### Autorizar el recurso completo

`authorize_resource::<U, R>()` adjunta la comprobación de habilidad
convencional a cada ruta generada como middleware por ruta - paridad con
el `authorizeResource` de Laravel. Sin ella, una superficie de recurso
queda sin compuerta a menos que cada cuerpo de controlador recuerde llamar
a `Gate::authorize`; un solo `destroy` olvidado despliega un delete sin
compuerta.

```rust
use suprnova::{Router, Gate};

// Las habilidades se indexan por (habilidad, tipo de usuario, tipo marcador de recurso).
Gate::define::<User, Post>("view",   |u, _p| u.is_member);
Gate::define::<User, Post>("create", |u, _p| u.is_author);
Gate::define::<User, Post>("update", |u, _p| u.is_author);
Gate::define::<User, Post>("delete", |u, _p| u.is_admin);

let router: Router = Router::new()
    .resource("posts", PostsCtl)
    .authorize_resource::<User, Post>()
    .into();
```

El mapeo acción → habilidad refleja el de Laravel:

| Acción(es) | Habilidad |
|---|---|
| `index`, `show`     | `view`   |
| `create`, `store`   | `create` |
| `edit`, `update`    | `update` |
| `destroy`           | `delete` |

`PATCH` comparte la acción `update`, así que queda protegida de forma
idéntica a `PUT`. Una habilidad denegada hace cortocircuito con `403`
antes de que el handler se ejecute, y una solicitud no autenticada falla
cerrado. El marcador de recurso `R` solo necesita `Default` - la
compuerta discrimina por su *tipo*, del mismo modo que Laravel discrimina
por la clase del modelo. Consulta el [capítulo de autorización](authorization.md)
para definir las habilidades en sí.

## Redirecciones y vistas a nivel de router

Tres métodos de azúcar sintáctico en `Router` cubren declaraciones de ruta
que no necesitan una función handler:

```rust
use suprnova::Router;
use serde_json::json;

let router = Router::new()
    // Redirección estática: GET /old-pricing → 302 /pricing
    .redirect("/old-pricing", "/pricing", 302)
    // contraparte en 301
    .permanent_redirect("/legacy", "/new")
    // Página estática de Inertia: GET /about renderiza el componente About
    .inertia("/about", "About", json!({ "team_size": 4 }))
    .name("about");
```

`Router::inertia` es el equivalente de
`Route::inertia($uri, $component, $props)` de Laravel. Registra `GET`;
una solicitud `HEAD` cae en ella y recibe el cuerpo eliminado en el
límite del servidor, así que no hay nada más que registrar. Devuelve un
`RouteBuilder`, por lo que `.name(...)` y `.middleware(...)` se encadenan
como en cualquier otra ruta.

Los props deben ser un objeto JSON o `null` si no hay ninguno. Cualquier
otra cosa - un array o una cadena - es un error de registro, no un mapa
de props vacío silencioso. `try_inertia` es la forma que puede fallar.

`Router::view` es el mismo método con su nombre antiguo; devuelve
`Router` en lugar de `RouteBuilder`, así que la ruta declarada no puede
recibir nombre. Prefiere `inertia`.

### Por qué Suprnova diverge

Laravel renderiza una plantilla Blade; Suprnova renderiza un componente
Inertia porque su sistema de plantillas es Inertia, no Blade. El nombre
del componente es una cadena de runtime, así que aquí no recibe la
comprobación de página en tiempo de compilación que hace la macro
`inertia_response!`. Escribe el handler con `inertia_response!` cuando
quieras que un typo en el nombre falle el build y no la solicitud.

Para *respuestas* de redirección (no declaraciones de ruta) -
`Redirect::route`, `Redirect::back`, `Redirect::intended`, redirecciones
firmadas - consulta [Generación de URLs](urls.md) y
[Respuestas](responses.md).

## URLs firmadas

Las rutas firmadas con HMAC son un tema adyacente al enrutamiento (se
genera una URL contra una ruta con nombre, y luego se verifica la firma en
la solicitud entrante). Se cubren en detalle en
[Generación de URLs](urls.md); la versión corta:

```rust
use suprnova::url;

let reset = url::signed_route("password.reset", &[("user", "42")])?;
// /password/reset/42?signature=...

let expires_at = chrono::Utc::now().timestamp() + 3600;
let verify = url::temporary_signed_route("verify.email", &[("user", "42")], expires_at)?;
// /verify/email/42?expires=1748803600&signature=...
```

Verifica dentro de un handler con `url::has_valid_signature(&request)`
(booleano) o `url::signature_verdict(&request)` (la división de tres vías
`Valid`/`Expired`/`Invalid`, para poder renderizar una página de
"solicitar un enlace nuevo" en lugar de un 403 genérico).

## Registro falible

El registro de rutas se ejecuta una sola vez en el arranque, así que una
ruta duplicada o malformada se trata como un error de programación: los
ayudantes simples (`Router::get`, `post`, `put`, `delete`, `ws`,
`RouteBuilder::name`, la conversión `From` de `GroupBuilder` → `Router`)
**entran en pánico** para fallar estrepitosamente en el arranque. Ese es el
valor por defecto correcto para las rutas declaradas en el código fuente.

Cuando los patrones o los nombres provienen de una fuente falible -
configuración dinámica, un sistema de plugins, un test que registra rutas
en conflicto a propósito - usa las contrapartes `try_*`. Devuelven
`Result<_, FrameworkError>` (nombrando el método, la ruta o el nombre en
conflicto responsable) en lugar de entrar en pánico:

| Con pánico | Contraparte falible | Devuelve |
|---|---|---|
| `Router::get` / `post` / `put` / `patch` / `delete` / `head` / `options` | `try_get` / `try_post` / `try_put` / `try_patch` / `try_delete` / `try_head` / `try_options` | `Result<RouteBuilder, FrameworkError>` |
| `Router::ws` (y cada variante `ws_*`) | `try_ws` (y cada `try_ws_*`) | `Result<Router, FrameworkError>` |
| `RouteBuilder::name` | `try_name` | `Result<Router, FrameworkError>` |
| `GroupBuilder` → `Router` vía `.into()` | `GroupBuilder::try_finalize` | `Result<Router, FrameworkError>` |
| `ResourceRoutes::register` | `try_register` | `Result<Router, FrameworkError>` |

```rust
use suprnova::{FrameworkError, Router};

// `path` proviene de configuración dinámica; un patrón malformado o duplicado
// es recuperable, no un pánico de arranque.
fn register_dynamic(router: Router, path: &str) -> Result<Router, FrameworkError> {
    Ok(router.try_get(path, health)?.into())
}
```

Una ruta de grupo duplicada es recuperable de la misma manera - como
`From` no puede ser falible, la contraparte falible de `.into()` es el
método inherente `try_finalize`:

```rust
let router: Router = Router::new()
    .group("/api", |r| r.get("/users", list).post("/users", create))
    .try_finalize()?;
```

Los ayudantes que entran en pánico se mantienen como válvulas de escape
ergonómicas; las contrapartes `try_*` son puramente aditivas.

## Por qué Suprnova diverge

**Sintaxis dual de parámetros de ruta.** Laravel usa `{param}`; Express
usa `:param`. Suprnova acepta ambas y normaliza `:param` a `{param}` antes
de que la ruta llegue a `matchit`. Los dos estilos se combinan con todo lo
demás - grupos, vinculación de modelo, URLs firmadas. La razón no es
indecisión; es que no se puede predecir de qué trasfondo viene cada quien,
y la sintaxis de enrutamiento es un punto de fricción demasiado frecuente
como para hacer que la gente tenga que reaprenderla.

**Dos APIs igual de válidas: macro y builder.** Laravel distribuye un
único DSL (`Route::get(...)`). Suprnova distribuye tanto la macro
declarativa `routes! { ... }` como el builder encadenable
`Router::new().get(...).name(...)`. Ambos producen registros idénticos. La
macro se lee mejor para tablas de rutas de nivel superior; el builder se
lee mejor cuando se están componiendo routers de forma dinámica (plugins,
rutas generadas, tests). Elige el que mejor encaje en el sitio de llamada -
no hay una respuesta canónica, porque ambas formas son de primera clase.

**Pánicos en el arranque, no ensombrecimiento silencioso.** Un nombre de ruta
duplicado o una colisión de patrones hace que el proceso entre en pánico
en el arranque. Los registros indexados por array de Laravel dejan que
gane silenciosamente el registro más tardío, lo cual está bien cuando el
archivo de rutas es el único registrador, pero es inseguro en cuanto
entran en juego plugins o rutas generadas. Las contrapartes `try_*` son la
válvula de escape para cuando lo que realmente se quiere es falibilidad.

## Siguiente

- [Controladores](controllers.md) - `#[handler]`, solicitudes de
  formulario, devolver JSON/Inertia
- [Middleware](middleware.md) - el trait `Middleware`, el orden, cómo
  construir el propio
- [Generación de URLs](urls.md) - URLs de rutas con nombre, URLs firmadas,
  redirecciones, `RouteUrlError`
- [Autorización](authorization.md) - compuertas y políticas para modelos
  vinculados
- [WebSockets](websockets.md) - `ws!`, el trait `WebSocketHandler`,
  configuración por ruta
