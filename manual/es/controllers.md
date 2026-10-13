# Controladores

Un controlador de Suprnova no es más que una función async. Toma lo que
necesita de la solicitud - parámetros de ruta tipados, un modelo ya
cargado, un formulario validado - y devuelve un `Response`. No hay clase
base de controlador. No hay archivo de cableado de localizador de
servicios. La unidad es la función, y el atributo `#[handler]` la pega a
las macros de enrutamiento.

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

La firma de ese handler hace tres cosas a la vez: declara el parámetro de
ruta (`user`), saca la fila de la base de datos y devuelve un 404 si la
fila no está. Nada de eso se escribe a mano. `#[handler]` lee los tipos
de los argumentos y genera la extracción.

## Generación de un controlador

```bash
suprnova make:controller User
```

Esto escribe `src/controllers/user.rs` con un único stub `invoke` y añade
`pub mod user;` a `src/controllers/mod.rs`. El stub es el handler mínimo
viable:

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

Añade al archivo tantas funciones como quieras - Suprnova no lleva la
cuenta de "clases" de controlador, solo de funciones. Muchas apps
dividen por recurso (`controllers::user::{index, show, store, update,
destroy}`), pero nada en el framework lo impone.

El nombre se convierte a `snake_case` para el nombre del archivo:
`OrderItem` pasa a ser `order_item.rs`.

## El atributo `#[handler]`

La macro clasifica el tipo de cada parámetro y genera el extractor
correspondiente. Cuatro categorías:

| Tipo del parámetro | Se extrae mediante | Modo de fallo |
|---|---|---|
| `Request` | pasa la solicitud tal cual, sin cambios | - |
| `i32`, `i64`, `u32`, `u64`, `usize`, `String` | `FromParam` - analiza el parámetro de ruta del mismo nombre | 400 si falla el análisis, 400 si falta |
| `T: AutoRouteBinding` (cualquier `Model` de Eloquent) | analiza el parámetro como la clave primaria del modelo y carga la fila | 400 si falla el análisis, 404 si no se encuentra |
| Cualquier otra cosa (`T: FromRequest`) | llama a `T::from_request(req)` - normalmente un validador `#[derive(FormRequest)]` | lo que devuelva `from_request`; 422 para los errores de validación |

La macro ejecuta las extracciones en el orden de declaración, así que el
cuerpo de tu función ve valores completamente tipados. Si alguna
extracción falla, el error cortocircuita vía `?` y el cuerpo del handler
nunca llega a ejecutarse.

### Parámetros de ruta

```rust
// Ruta: get!("/users/{id}", controllers::user::show)
#[handler]
pub async fn show(id: i64) -> Response {
    json_response!({ "user_id": id })
}

// Ruta: get!("/posts/{post_id}/comments/{comment_id}", show_comment)
#[handler]
pub async fn show_comment(post_id: i64, comment_id: i64) -> Response {
    json_response!({
        "post_id": post_id,
        "comment_id": comment_id,
    })
}
```

El nombre del argumento debe coincidir con el placeholder de la ruta:
`{id}` requiere `id: …`. El tipo del argumento se analiza vía
`FromParam`. Una entrada incorrecta (`/users/abc` contra `id: i64`)
devuelve un 400 con un mensaje que nombra el parámetro y el tipo de
destino.

### Vinculación de modelo de ruta

Los modelos de `Eloquent` implementan `AutoRouteBinding` automáticamente.
Declara el modelo como argumento y el framework lo carga:

```rust
use suprnova::{handler, json_response, Response};
use crate::models::user;

// Ruta: get!("/users/{user}", controllers::user::show)
#[handler]
pub async fn show(user: user::Model) -> Response {
    json_response!({
        "id": user.id,
        "name": user.name,
        "email": user.email,
    })
}
```

El nombre del placeholder de la ruta (`{user}`) y el nombre del argumento
(`user`) deben coincidir. El framework analiza la cadena del parámetro
como el tipo de la clave primaria del modelo, llama a
`Entity::find_by_pk` y devuelve un 404 si la fila falta. Cualquier struct
con `#[suprnova::model]` se vincula automáticamente; la macro
`route_binding!` sigue disponible para las entidades de SeaORM escritas a
mano que no usan `#[suprnova::model]` - consulta
[Macros](macros.md#route_binding).

### Solicitudes de formulario

Cualquier cosa que implemente `FromRequest` se enchufa de la misma
manera. El caso común es un struct `#[derive(FormRequest)]` que valida el
cuerpo de la solicitud y, cuando falla, saca a la luz un 422 con los
errores indexados por campo:

```rust
use suprnova::{attrs, handler, json_response, Response};
use crate::models::user;
use crate::requests::UpdateUserRequest;

// Ruta: put!("/users/{user}", controllers::user::update)
#[handler]
pub async fn update(user: user::Model, form: UpdateUserRequest) -> Response {
    let id = user.id;
    user.update(attrs! { name: form.name, email: form.email }).await?;
    json_response!({ "updated": id })
}
```

Consulta [Solicitudes de formulario](requests.md) para el derive del
validador y el pipeline de validación completo.

### Cuando quieres el `Request` en crudo

Si prefieres extraer las cosas a mano - o necesitas un encabezado, una
cookie, un query string - toma `Request` directamente:

```rust
use suprnova::{handler, json_response, Request, Response};

#[handler]
pub async fn show(req: Request) -> Response {
    let id = req.param("id")?;             // param de ruta, 400 si falta
    let ua = req.header("User-Agent");      // Option<&str>
    let page: u32 = req.query_param("page") // Option<String>
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);

    json_response!({ "id": id, "ua": ua, "page": page })
}
```

Se pueden mezclar y combinar:
`pub async fn nested(category_id: i64, product: product::Model, req: Request)`
es una firma válida. La macro extrae cada argumento según su propia
regla.

## El contrato `Response`

`Response` es un alias de `Result<HttpResponse, HttpResponse>`. Ambas
ramas llevan el mismo tipo de payload, que es la razón de que `?`
funcione en todas partes. La cadena de middleware colapsa el resultado
con una sola línea en el límite:

```rust
result.unwrap_or_else(|e| e)
```

Es el mismo contrato del que depende cada punto de propagación de `?`.
Los errores se convierten vía `From<FrameworkError> for HttpResponse`
antes de llegar a la cadena - consulta
[Modelo de errores](error-model.md) para el panorama completo.

El cuerpo de un handler se lee de arriba abajo y usa `?` para salir:

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

Si `find_or_fail` devuelve `Err`, la función sale con un 404. Si
`invoices().get()` da error, obtienes un 500. Sin sentencias `match`, sin
captura de excepciones.

## Crear respuestas

Tres macros y un builder cubren los casos comunes:

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
    // Estado y encabezados encadenables integrados vía ResponseExt.
    json_response!({ "id": 1, "created": true }).status(201)
}

#[handler]
pub async fn page() -> Response {
    Ok(HttpResponse::html("<h1>Hello</h1>"))
}
```

`json_response!`, `text_response!` y `HttpResponse::*` producen todos el
mismo tipo `Response`. El trait `ResponseExt` añade `.status(...)`,
`.header(...)`, `.cookie(...)` y `.with_headers(...)` para poder
encadenar configuración sobre el resultado de una macro.

Para todo lo demás - descargas de archivos, cuerpos en streaming,
respuestas de Inertia, redirecciones - consulta
[Respuestas](responses.md).

## Redirecciones

`redirect!("route.name")` valida en tiempo de compilación que la ruta
existe y devuelve un builder al que se le puede encadenar configuración:

```rust
use suprnova::{handler, redirect, Response};

#[handler]
pub async fn store() -> Response {
    // Crea el usuario…
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

`.with(key, value)` rellena un placeholder de ruta; `.query(key, value)`
añade un parámetro al query string; `.flash(key, value)` escribe en la
flash bag de la sesión para la siguiente solicitud. `.into()` convierte
el builder en un `Response`.

Si la ruta nombrada no existe, la macro hace fallar la compilación con
una lista de los nombres de ruta disponibles - las erratas salen a la luz
antes de llegar a staging.

## Servicios inyectados por el contenedor

Resuelve servicios del contenedor con `App::resolve` (tipos concretos) o
`App::resolve_make` (objetos trait). Ambos devuelven
`Result<_, FrameworkError>`, así que se componen con `?`:

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

Si estás vinculando acciones con `#[injectable]`, así es como las llama
un controlador. Consulta [Acciones](actions.md) para la forma de una
acción, y [Contenedor de servicios](container.md) para la superficie
completa del contenedor - vinculación, fábricas, la cascada de búsqueda
task-local / thread-local / global.

## Un controlador RESTful de ejemplo

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

Regístralos con la macro `routes!`:

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

El placeholder de ruta `{user}` coincide con el nombre del argumento
`user: user::Model`, que es como el framework sabe qué segmento de la
ruta carga el modelo.

## La API de `Request`

Los métodos a los que más echarás mano cuando tomes `Request`
directamente:

| Método | Devuelve | Notas |
|---|---|---|
| `method()` | `&hyper::Method` | método HTTP |
| `path()` | `&str` | ruta de la URL |
| `param(name)` | `Result<&str, ParamError>` | param de ruta; `?` para salir |
| `params()` | `&HashMap<String, String>` | todos los params de ruta |
| `query()` | `Option<&str>` | query string en crudo |
| `query_param(key)` | `Option<String>` | un solo valor del query string |
| `query_params()` | `HashMap<String, String>` | todos los params de query |
| `query_into::<T>()` | `Result<T, FrameworkError>` | deserialización tipada |
| `header(name)` | `Option<&str>` | un solo encabezado |
| `headers()` | `&hyper::HeaderMap` | el mapa completo de encabezados |
| `has_header(name)` | `bool` | comprobación de presencia |
| `bearer_token()` | `Option<String>` | el `Authorization: Bearer …` ya analizado |
| `cookie(name)` | `Option<String>` | el valor de una sola cookie |
| `cookies()` | `HashMap<String, String>` | todas las cookies |
| `ip()` | `Option<String>` | IP del par, con reconocimiento de X-Forwarded-For |
| `secure()` | `bool` | detección de HTTPS (incluidos proxies) |
| `is_method(m)` | `bool` | sin distinguir mayúsculas de minúsculas |
| `is_inertia()` | `bool` | encabezado XHR de Inertia |
| `ajax()` | `bool` | `X-Requested-With: XMLHttpRequest` |
| `expects_json()` / `wants_json()` | `bool` | inspección del encabezado Accept |
| `route_name()` | `Option<String>` | el `.name(...)` de la ruta coincidente |
| `json::<T>()` | `Result<T, FrameworkError>` | analiza el cuerpo como JSON (lo consume) |
| `form::<T>()` | `Result<T, FrameworkError>` | analiza como form-urlencoded |
| `input::<T>()` | `Result<T, FrameworkError>` | análisis despachado según el content-type |

Es una superficie con forma de Laravel - cada método de aquí refleja un
método de la clase `Request` de Laravel.

## Layout de archivos

Convención:

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

Nada en el framework impone este layout - los controladores pueden vivir
en cualquier sitio alcanzable desde `routes.rs`. La convención existe
porque es lo que emite el andamiaje y porque las rutas y los
controladores son la pareja natural.

## Por qué Suprnova diverge

Los controladores de Laravel son clases que extienden
`Illuminate\Routing\Controller`. Los métodos se llaman sobre instancias
que el contenedor resuelve por solicitud, que es donde ocurre la
inyección por constructor. El patrón está bien en PHP - hacer `new` en
cada solicitud es barato cuando el proceso entero se desmonta después de
la respuesta.

En Rust, ese patrón significaría o bien (a) reservar un struct de
controlador por solicitud, lo que cuesta un clon de `Arc` que no
necesitas, o bien (b) reimplementar la inyección de dependencias a través
de una jerarquía de clases base que no se paga a sí misma.

Suprnova elige el modelo más simple: un controlador es una función async
libre, y las "dependencias" son o bien resoluciones del contenedor
(`App::resolve::<Service>()?`) o bien argumentos tipados por extracción
(`form: UpdateUserRequest`). La inyección por constructor ocurre en el
límite de `#[injectable]` en [Acciones](actions.md), que es donde
corresponde. El handler sigue siendo una función pura que va de solicitud
a respuesta, lo que hace trivial probarlo de forma aislada: construye un
`Request`, llama a la función y comprueba el resultado.

## Siguiente

- [Enrutamiento](routing.md) - en qué se expanden `routes!`, `get!`, `post!` y `.name()`
- [Solicitudes de formulario](requests.md) - validación tipada vía `#[derive(FormRequest)]`
- [Respuestas](responses.md) - JSON, HTML, archivos, streams, páginas de Inertia, redirecciones
- [Contenedor de servicios](container.md) - qué hace realmente `App::resolve`
- [Acciones](actions.md) - dónde vive la lógica de negocio fuera del controlador
- [Modelo de errores](error-model.md) - cómo `?` convierte un `FrameworkError` en una respuesta
