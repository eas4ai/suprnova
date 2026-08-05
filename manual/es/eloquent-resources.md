# Recursos JSON:API

Suprnova incluye una capa de recursos JSON:API para APIs REST
tipadas. Marca un struct `#[derive(Data)]` con
`#[json_resource("type")]` y el framework emite un impl de
`IntoJsonResource` que maneja envolturas individuales, colecciones,
colecciones paginadas, campos dispersos (`?fields[type]=...`),
documentos compuestos `included`, y cadenas `?include=a.b.c`
multinivel, todo por la misma ruta de código. Las dos fachadas -
`Resource` y `JsonApi` - son el mismo tipo bajo dos nombres; usa la
que combine mejor con tus propias convenciones de estilo.

## Definir un recurso

```rust
use suprnova::Data;

#[derive(Debug, Clone, Data)]
#[json_resource("users")]
pub struct UserResource {
    pub id: i64,
    pub email: String,

    // `input_only` mantiene `password` disponible en el lado del
    // form-request, pero lo suprime de la salida de la API.
    #[data(input_only)]
    pub password: String,

    // Marca un campo como *relationship*: nunca aterriza en
    // `attributes`, produce un objeto de relationship JSON:API en su
    // lugar, y es elegible para `?include=`. El tipo del campo debe
    // implementar `IntoJsonResource` (directamente, o vía `Vec<T>` /
    // `Option<T>`).
    #[data(allow_include)]
    pub posts: Vec<PostResource>,
}
```

La palabra clave `id_field` renombra el campo que provee el `id` de
JSON:API:

```rust
#[derive(Data)]
#[json_resource("orders", id_field = "uuid")]
pub struct OrderResource {
    pub uuid: String,
    pub total_cents: i64,
}
```

## Renderizar respuestas

Construye una respuesta pendiente desde un handler y llama a
`.render().await`:

```rust
use suprnova::{LengthAwarePaginator, Resource};

#[handler]
async fn show_user(id: i64) -> Result<HttpResponse, FrameworkError> {
    let user: UserResource = User::find_or_fail(id).await?.into();
    Resource::single(user).render().await
}

#[handler]
async fn list_users() -> Result<HttpResponse, FrameworkError> {
    let users: Vec<UserResource> = User::all().await?.into_iter().map(Into::into).collect();
    Resource::collection(users).render().await
}

#[handler]
async fn paginate_users() -> Result<HttpResponse, FrameworkError> {
    // `paginate(per_page)` lee `?page=` de la solicitud actual
    // automáticamente.
    let page = User::query().paginate(10).await?;
    // Convierte el paginador de modelo en un paginador de recurso
    // campo por campo - `data` es `pub`, el resto de conteos/links se
    // trasladan igual.
    let page = LengthAwarePaginator::new(
        page.data.into_iter().map(UserResource::from).collect(),
        page.total,
        page.per_page,
        page.current_page,
    )
    .with_base_url("/api/users");
    Resource::paginated(page).render().await
}
```

`JsonApi::single` / `JsonApi::collection` / `JsonApi::paginated` son
puntos de entrada alias idénticos si prefieres la grafía de Laravel.

## Mutadores encadenables

`JsonApiResponse` es un objeto pendiente. Personaliza la envoltura
antes de llamar a `.render().await`. Cada mutador es `self` → `Self`,
así que se componen:

```rust
use suprnova::{Resource, JsonApiInfo};
use serde_json::json;

let info = JsonApiInfo::new()
    .with_version("1.1")
    .with_ext("https://jsonapi.org/ext/atomic")
    .with_meta("copyright", json!("2026 Acme Inc."));

Resource::single(user)
    .status(201)                                  // override de status HTTP
    .with_meta("trace_id", json!("req-7"))        // KV de meta a nivel raíz
    .with_link("self", "/api/users/1")            // link a nivel raíz
    .with_jsonapi(info)                           // `jsonapi` a nivel raíz
    .additional(json!({ "api_version": "2.0" }).as_object().unwrap().clone())
    .render()
    .await
```

| Mutador | Análogo en Laravel | Efecto |
|---|---|---|
| `.status(code)` | `ResourceResponse::calculateStatus` | Sobrescribe el status HTTP. |
| `.created()` | `wasRecentlyCreated → 201` | Abreviatura de `.status(201)`. |
| `.with_meta(k, v)` / `.meta(k, v)` | `with($request)` | KV de `meta` a nivel raíz. |
| `.with_meta_map(m)` | `with($request)` en bloque | Fusiona un mapa dentro de `meta` a nivel raíz. |
| `.with_link(rel, href)` / `.link(rel, href)` | `with($request)['links']` | KV de `links` a nivel raíz. |
| `.with_link_value(rel, v)` | forma de objeto link | Link a nivel raíz como `{href, meta}`. |
| `.with_additional(k, v)` | `additional($data)` | Clave a nivel raíz, junto a `data`. |
| `.additional(map)` | `additional($data)` | Claves adicionales en bloque. |
| `.with_jsonapi(info)` | `JsonApiResource::configure(...)` | Miembro `jsonapi` a nivel raíz. |

Los miembros canónicos (`data`, `included`, `links`, `meta`,
`jsonapi`, `errors`) nunca se sobrescriben con `.additional(...)`.

## `links` y `meta` por recurso

Sobrescribe los valores por defecto de
`IntoJsonResource::resource_links` y
`IntoJsonResource::resource_meta` para adjuntar links / metadatos al
*objeto de recurso*, no a la raíz del documento:

```rust
use suprnova::resources::IntoJsonResource;
use serde_json::{Map, Value};

impl IntoJsonResource for MyHandRolledPost {
    // ...

    fn resource_links(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("self".into(), Value::String(format!("/api/posts/{}", self.id)));
        m
    }

    fn resource_meta(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("kind".into(), Value::String("blog".into()));
        m
    }
}
```

Ambos usan por defecto un `Map` vacío para los recursos derivados por
macro, así que el renderer de JSON:API omite las claves cuando no se
usan. Sobrescribe `resource_top_level_meta` para elevar metadatos por
recurso hasta el miembro `meta` a nivel raíz de la envoltura.

## Atributos condicionales - `Maybe<T>` / `MissingValue<T>`

Usa `Maybe` para omitir un campo del objeto `attributes` renderizado,
según una condición en tiempo de ejecución. Este es el análogo en
Suprnova del `MissingValue` de Laravel y de la familia
`when()` / `whenLoaded()` / `unless()`.

```rust
use suprnova::{Maybe, MissingValue};

// Ambos nombres apuntan al mismo tipo.
let m1: Maybe<&str> = Maybe::present("email@example.com");
let m2: MissingValue<&str> = MissingValue::missing();
let m3 = Maybe::when(user.is_verified, &user.verified_at);
let m4 = Maybe::unless(user.is_admin, &user.public_handle);
let m5 = Maybe::when_with(expensive_check(), || compute_value()); // perezoso
```

Para structs derivados por macro, declara un campo como `Maybe<T>` y
el renderer lo descarta automáticamente cuando es `Missing`. Para un
`resource_attributes` escrito a mano, usa el ayudante
`insert_maybe(map, key, maybe)`:

```rust
use suprnova::resources::{insert_maybe, Maybe};

fn resource_attributes(&self, _fs: Option<&[&str]>) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    insert_maybe(&mut map, "email", Maybe::present(&self.email));
    insert_maybe(
        &mut map,
        "phone",
        if self.show_phone { Maybe::present(&self.phone) } else { Maybe::missing() },
    );
    serde_json::Value::Object(map)
}
```

El renderer también llama a `strip_missing_values(&mut value)` sobre
todo el objeto de attributes, así que los valores `Maybe::Missing`
anidados dentro de estructuras derivadas de serde arbitrarias se
descartan de forma recursiva - útil cuando un transformador
profundamente anidado quiere omitir subcampos.

## Campos dispersos

El `IncludeMiddleware` del framework analiza parámetros de query con
la forma `?fields[type]=email,name` y los vincula a un task-local. El
`resource_attributes` emitido por la macro consulta el conjunto de
campos y solo emite los attributes solicitados. No se necesita
trabajo del lado del handler - instala el middleware y la capa de
recursos lo respeta automáticamente.

```rust
// Request: GET /api/users/7?fields[users]=email
// Response: { "data": { "type": "users", "id": "7", "attributes": { "email": "alice@example.com" } } }
```

## Documentos compuestos - cadenas `?include=`

Declara los campos de relationship con `#[data(allow_include)]`. El
framework construye un `IncludeTree` a partir de
`?include=author.posts.tags,comments`, recorre cada nodo, y empuja
objetos de recurso completamente resueltos dentro de `included`. La
deduplicación corre en el momento del push a través de
`IncludedSink`, indexada por `(type, id)` según el §8 de la
especificación JSON:API - así que una colección de 1.000 elementos
donde todos comparten el mismo author resuelve al author exactamente
una vez. El pico de memoria y CPU se mantiene proporcional a los
recursos incluidos distintos, no al fan-in de la relación.

```rust
#[derive(Data)]
#[json_resource("posts")]
pub struct PostResource {
    pub id: i64,
    pub title: String,

    #[data(allow_include)]
    pub author: Option<AuthorResource>,

    #[data(allow_include)]
    pub tags: Vec<TagResource>,
}
```

Una solicitud que nombra una ruta de include que no está en la lista
de permitidos de este recurso recibe una envoltura de errores
JSON:API 400.

### Por qué Suprnova diverge

Dos divergencias visibles respecto al `JsonApiResource` de Laravel:

1. **Denegación estricta por defecto para `?include=`.** La capa de
   recursos de Laravel ignora en silencio las rutas de include que no
   resuelven. Suprnova las rechaza con un `400 Bad Request` que lleva
   una envoltura de errores JSON:API. La postura de denegación por
   defecto del §5.2.2 de la especificación es el contrato contra el
   que los clientes pueden programar; ignorar en silencio esconde
   bugs del cliente y rompe la integridad del documento compuesto.

2. **`.status(code)` / `.created()` explícitos en vez de un 201
   automático.** Laravel fija automáticamente `201` a partir de
   `wasRecentlyCreated` sobre el modelo Eloquent subyacente. Suprnova
   desacopla el DTO de recurso de cualquier ciclo de vida de
   persistencia específico, así que el status se fija sobre el propio
   objeto de respuesta - `.created()` cuando eso es lo que quieres
   decir, `.status(204)` cuando la respuesta está vacía, y así
   sucesivamente. Un único mutador se mantiene honesto bajo cualquier
   flujo.

## Paginación

`Resource::paginated(p)` funciona con cualquier paginador que
implemente el trait `Paginated<T>` - tanto `LengthAwarePaginator<T>`
como `CursorPaginator<T>` de `suprnova::pagination` traen este impl.
El renderer adjunta `links.{self,first,prev,next,last}` y un bloque
`meta.pagination` automáticamente.

```rust
use suprnova::{LengthAwarePaginator, Resource};

let page = LengthAwarePaginator::new(items, total, per_page, current_page)
    .with_base_url("/api/users");
Resource::paginated(page).render().await
```

## Envolturas de error

Cada `FrameworkError` sabe renderizarse a sí mismo como una envoltura
JSON:API `{"errors": [...]}` vía `into_json_api_response()`. El
helper está expuesto porque `FrameworkError` lleva un código de
status, un puntero de origen con nombre de campo (para
`ValidationError`), y un token de correlación de id de solicitud bajo
`meta.request_id`. Las respuestas 5xx se sanean: el mensaje en crudo
nunca llega al cliente a menos que `APP_DEBUG=true` esté fijado en el
entorno activo, en cuyo caso aparece bajo `meta.debug_message`.

```rust
let response = FrameworkError::validation("email", "email is invalid")
    .into_json_api_response();
// {
//   "errors": [{
//     "status": "422",
//     "title": "Validation failed",
//     "detail": "email is invalid",
//     "source": { "pointer": "/data/attributes/email" },
//     "meta": { "request_id": "..." }
//   }]
// }
```

## Resumen de superficies

| Superficie de Suprnova | Equivalente en Laravel 13 |
|---|---|
| Fachadas `Resource` / `JsonApi` | `JsonResource::make`, `JsonApiResource` |
| `JsonApiResponse` | `ResourceResponse`, `JsonApiResource::toResponse` |
| `JsonApiBuilder` | (builder interno para `ResourceResponse`) |
| Trait `IntoJsonResource` | `JsonResource::toArray`, `toAttributes`, `toRelationships`, `toLinks`, `toMeta`, `with` |
| `RelationshipValue` / `ResourceIdentifier` | forma de array dentro de `toRelationships` |
| `IncludeTree` | `?include=` analizado desde `JsonApiRequest` |
| `RequestFieldsetSet` | `?fields[type]=` analizado desde `JsonApiRequest` |
| `Maybe<T>` / `MissingValue<T>` | `MissingValue` + `whenLoaded` / `when` / `unless` |
| `JsonApiInfo` | `JsonApiResource::$jsonApiInformation` |
| `JsonApiResponse::status(code)` / `.created()` | `ResourceResponse::calculateStatus` |
| `JsonApiResponse::additional(map)` / `.with_additional(k, v)` | `JsonResource::additional($data)` |
| `JsonApiResponse::with_meta(k, v)` / `.meta(k, v)` | `JsonResource::with($request)['meta']` |
| `JsonApiResponse::with_link(rel, href)` / `.link(rel, href)` | `JsonResource::with($request)['links']` |
| `JsonApiResponse::with_jsonapi(info)` | `JsonApiResource::configure(...)` |
| `current_fieldset()` / `scope_fieldset(...)` | fieldset task-local, fijado por `IncludeMiddleware` |
| `IncludeResolutionError` → envoltura 400 | parser `?include=` en modo estricto |

Reexportaciones de nivel superior bajo `suprnova::`: `Resource`,
`JsonApi`, `JsonApiResponse`, `JsonApiBuilder`, `JsonApiInfo`,
`IncludedSink`, `IntoJsonResource`, `RelationshipValue`,
`ResourceIdentifier`, `IncludeTree`, `RequestFieldsetSet`, `Maybe`,
`MissingValue`, `insert_maybe`, `strip_missing_values`,
`AsRelationshipValue`, `PushIncluded`, `IncludeResolutionError`,
`current_fieldset`, `scope_fieldset`.

## Siguiente

- [Serialización de Eloquent](eloquent-serialization.md) -
  `#[derive(Data)]`, campos hidden/visible, el equivalente a
  `toArray` que alimenta los attributes del recurso
- [Relaciones de Eloquent](eloquent-relationships.md) - lo que
  consume `#[data(allow_include)]`; los tipos de relación tipados que
  respaldan los documentos compuestos
- [Paginación](pagination.md) - `LengthAwarePaginator`,
  `CursorPaginator`, y el trait `Paginated<T>` que consume
  `Resource::paginated`
- [Objetos de datos](data.md) - la macro `#[derive(Data)]`
  compartida con Inertia, el middleware `?include=`/`?fields[type]=`,
  y los patrones de `Maybe<T>`
- [Modelo de errores](error-model.md) - cómo
  `FrameworkError::into_json_api_response` encaja en el contrato de
  conversión
