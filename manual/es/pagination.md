# Paginación

Suprnova ofrece tres paginadores que igualan la superficie de Laravel
línea por línea: consciente de longitud (conoce el total), simple (una
consulta por página), y por cursor (keyset opaco). Los tres derivan
`Serialize` hacia el JSON con forma de Laravel que Inertia y los
consumidores JSON:API ya entienden - se obtiene una página y se
devuelve; no se requiere nada más.

```rust
use crate::models::User;

let page = User::query()
    .filter("active", true)
    .order_by_desc("created_at")
    .paginate(20)
    .await?;
```

Esa única llamada ejecuta el `COUNT(*)` y la obtención de página
`LIMIT/OFFSET`, analiza `?page=N` desde la solicitud activa, y devuelve
un `LengthAwarePaginator<User>` listo para enviar. Los dos hermanos -
`simple_paginate(20)` y `cursor_paginate(20)` - devuelven la misma
forma de valor con compensaciones distintas. El resto de este capítulo
trata de cuál elegir, qué cuesta cada uno, y cómo llega el JSON.

## Elegir un paginador

La forma más rápida de elegir es la tabla de compensaciones:

| Método | Tipo | Consultas / página | ¿Conoce el total? | Usar cuando |
|---|---|---|---|---|
| `paginate(n)` | `LengthAwarePaginator<M>` | 2 (`COUNT(*)` + página) | sí | la UI muestra páginas numéricas o "página 3 de 17" |
| `simple_paginate(n)` | `Paginator<M>` | 1 (`LIMIT n+1`) | no | tablas grandes; un botón "Siguiente" es suficiente |
| `cursor_paginate(n)` | `CursorPaginator<M>` | 1 (`LIMIT n+1`) | no | scroll infinito; páginas profundas en tablas activas |

La diferencia de coste importa una vez que la tabla es grande.
`COUNT(*)` sobre cien millones de filas es la consulta más cara del
presupuesto de la solicitud. `simple_paginate` ahorra el conteo.
`cursor_paginate` ahorra el conteo *y* evita el recorrido lineal de
`OFFSET N` que afecta a toda solicitud de página profunda sobre una
tabla grande - una búsqueda por cursor es casi `O(1)` con el índice
correcto, sin importar en qué punto del conjunto de resultados esté el
usuario.

### Por qué Suprnova diverge

Los paginadores de Laravel llevan helpers de construcción de URL -
`nextPageUrl()`, `previousPageUrl()`, el array `links` de descriptores
`{url, label, page, active}` que Blade renderiza. El impl `Serialize`
en bruto de Suprnova emite el segmento de datos más los contadores; la
construcción de URLs vive en los constructores de forma de respuesta
que ya poseen el contexto de URL:
[`Inertia::paginate`](frontend-inertia-responses.md) adjunta metadatos
de scroll de Inertia (identificadores de página, no URLs absolutas);
[`Resource::paginated`](eloquent-resources.md) adjunta
`links.{self,first,last,prev,next}` de JSON:API según la
recomendación de JSON:API.

Dos razones para la separación. Primero, la URL que el cliente debe
ver depende de qué superficie de protocolo la esté renderizando -
Inertia se basa en identificadores de página, JSON:API quiere hrefs
absolutos. Segundo, el paginador no conoce la URL base de la solicitud
por defecto; los helpers que sí la conocen pueden adjuntar las URLs
una sola vez, donde corresponde. Si de verdad se necesitan URLs sobre
el paginador desnudo (envoltorio JSON personalizado, payload de
telemetría, aserción de prueba), llama a `with_path(...)` y usa
`url_for_page(n)` - cubierto en la sección de
[generación de URLs](#generación-de-urls-y-rutas).

## `paginate` - consciente de longitud

```rust
use suprnova::LengthAwarePaginator;
use crate::models::User;

pub async fn index(_req: suprnova::Request) -> suprnova::Response {
    let page: LengthAwarePaginator<User> = User::query()
        .filter("active", true)
        .order_by_desc("created_at")
        .paginate(20)
        .await?;

    Ok(suprnova::json_response!(page))
}
```

Los campos públicos del struct:

```rust
pub struct LengthAwarePaginator<T> {
    pub data: Vec<T>,           // filas de esta página
    pub current_page: u64,       // basado en 1
    pub last_page: u64,          // basado en 1; 0 cuando total == 0
    pub per_page: u64,
    pub total: u64,              // cada fila en todas las páginas
    pub from: Option<u64>,       // índice de la primera fila de esta página, basado en 1
    pub to: Option<u64>,         // índice de la última fila de esta página, basado en 1
    pub path: Option<String>,    // URL base para url_for_page (opcional)
}
```

El JSON que emite el `Serialize` derivado:

```json
{
  "data": [...],
  "current_page": 1,
  "last_page": 3,
  "per_page": 10,
  "total": 25,
  "from": 1,
  "to": 10,
  "path": "/api/users"
}
```

`path` se omite del JSON cuando no está establecido; `from` y `to` son
`null` cuando la página está vacía (sin filas en esta página, o la
página solicitada está más allá de la última).

### Leer `?page=N` automáticamente

`paginate(n)` lee la página actual desde `?page=N` en la solicitud
activa vía `Context::query_param`. Los valores ausentes, vacíos, no
numéricos, y cero se fijan en `1`. No hay nada que cablear - si una
solicitud está en el alcance, el parámetro se lee.

### Varios paginadores en una misma página

Cuando una página renderiza más de una lista paginada, dale a cada una
su propia clave de query string con `paginate_using`:

```rust
let posts = Post::query()
    .order_by_desc("created_at")
    .paginate_using("posts_page", 10)
    .await?;

let comments = Comment::query()
    .order_by_desc("created_at")
    .paginate_using("comments_page", 25)
    .await?;
```

`paginate_using` también fija `page_name` en el paginador devuelto para
que `url_for_page` construya URLs con la misma clave:

```rust
posts.url_for_page(2);     // "/posts?posts_page=2"  (cuando path está establecido)
comments.url_for_page(3);  // "/posts?comments_page=3"
```

### Predicados de posición de página

Está implementado el conjunto completo de predicados del
`AbstractPaginator` de Laravel:

```rust
page.has_more_pages();   // current_page < last_page
page.on_first_page();    // current_page <= 1
page.on_last_page();     // !has_more_pages()
page.has_pages();        // no estamos en la página 1 O existen más páginas
page.is_empty();         // data.is_empty()
page.is_not_empty();     // !is_empty()
page.count();            // data.len() - segmento de página, no el total
```

`count()` es el tamaño del segmento, no el total - la forma
`Countable` de Laravel; para el total usa el campo `total`
directamente.

## `simple_paginate` - una consulta, sin conteo

```rust
use suprnova::Paginator;
use crate::models::User;

let page: Paginator<User> = User::query()
    .order_by_desc("id")
    .simple_paginate(20)
    .await?;
```

```rust
pub struct Paginator<T> {
    pub data: Vec<T>,
    pub current_page: u64,
    pub per_page: u64,
    pub has_more: bool,          // ¿vino una fila extra más allá de per_page?
    pub path: Option<String>,
}
```

JSON:

```json
{
  "data": [...],
  "current_page": 1,
  "per_page": 10,
  "has_more": true,
  "path": "/api/users"
}
```

El truco está en el SQL. `simple_paginate(20)` emite `LIMIT 21`,
comprueba si volvió la fila número 21, fija `has_more` a partir de eso,
y recorta `data` de vuelta a 20. Una consulta por página; sin
`COUNT(*)`.

Se renuncia a `total`, `last_page`, `from`, y `to`. A cambio se pueden
paginar tablas donde `COUNT(*)` es demasiado caro de ejecutar en cada
carga de página. La superficie de UI son botones "Siguiente" /
"Anterior", no "página 7 de 142".

Está implementado el mismo conjunto de predicados que el paginador
consciente de longitud: `has_more_pages()`, `on_first_page()`,
`on_last_page()`, `has_pages()`, `is_empty()`, `is_not_empty()`,
`count()`.

## `cursor_paginate` - keyset opaco

```rust
use suprnova::CursorPaginator;
use crate::models::User;

let page: CursorPaginator<User> = User::query()
    .cursor_paginate(20)
    .await?;
```

```rust
pub struct CursorPaginator<T> {
    pub data: Vec<T>,
    pub per_page: u64,
    pub next_cursor: Option<String>,  // None en la última página
    pub prev_cursor: Option<String>,  // None en la primera página
    pub path: Option<String>,
}
```

JSON:

```json
{
  "data": [...],
  "per_page": 10,
  "next_cursor": "...",
  "prev_cursor": null,
  "path": "/api/users"
}
```

`next_cursor` y `prev_cursor` siempre están presentes como claves JSON
(`null` cuando están ausentes) para que los esquemas del cliente
puedan confiar en la presencia del campo; `path` se omite cuando no
está establecido.

### Cómo funcionan los cursores en el wire

El cliente pasa el cursor de la página anterior a través de
`?cursor=<opaco>`:

```
GET /api/users?cursor=eyJ0IjoiQmlnSW50IiwidiI6MTAwLCJkIjoibmV4dCJ9...
```

`cursor_paginate` decodifica el cursor, recorre el filtro de keyset
(`pk > boundary ASC` para `next`; `pk < boundary DESC` para `prev`,
invertido de vuelta a ASC), obtiene `LIMIT n+1` filas, y vuelve a
emitir `next_cursor` / `prev_cursor` según existan los vecinos de la
página. Es bidireccional - el cliente puede avanzar y retroceder sin
perder su posición.

La paginación por cursor **reemplaza** cualquier `ORDER BY` existente
en el builder. Se requiere un orden total estable sobre la clave
primaria para que el filtro de keyset corte la tabla de forma
determinista; un `ORDER BY random_score()` arbitrario con un cursor
saltaría y duplicaría filas. Si se necesita un orden que no sea por
PK, cambia a `paginate` / `simple_paginate`.

### Los cursores están cifrados y autenticados

Los cursores de Suprnova **no** son el texto plano base64-JSON de
Laravel. El cursor sobre el wire es el límite del keyset (un
`sea_orm::Value` tipado - `Int`, `BigInt`, `Uuid`, fechas y horas,
decimales, cadenas, bytes) más una etiqueta de dirección, codificados
en JSON y luego sellados con AES-256-GCM a través del llavero `Crypt`
del framework (vinculado a `CryptPurpose::Cursor`, así que un texto
cifrado de cursor nunca puede repetirse hacia ninguna otra
superficie - cookie, secreto 2FA, cast).

Esto significa tres cosas en la práctica:

1. **Sin manipulación.** Un cliente que invierte bits en `?cursor=`
   obtiene un 400 `Invalid pagination cursor`, no una página distinta
   de datos.
2. **Sin fuga de información.** El valor de límite (a menudo una clave
   primaria, a veces un timestamp) queda sellado dentro del cursor -
   los clientes no pueden enumerar rangos editándolo.
3. **Los límites tipados hacen la ida y vuelta sin pérdida.** El
   envoltorio del wire etiqueta la variante de SeaORM (`"BigInt"`,
   `"Uuid"`, etc.), así que al decodificar el valor se vuelve a
   vincular con el mismo tipo SQL que emitió la columna original. Sin
   errores de coerción de cadenas entre Postgres / MySQL / SQLite.

No hay repliegue a texto plano. Si `Crypt` no está inicializado - lo
que debería ser imposible tras `Server::from_config` - se produce un
error de codificación en lugar de emitir un cursor falsificable.

### Por qué Suprnova diverge

El paginador por cursor de Laravel es solo-hacia-adelante por defecto
y el cursor sobre el wire es un blob JSON codificado en base64 -
legible, editable, repetible. El cursor de Suprnova es bidireccional
(igualando la superficie `cursorPaginate()` que Laravel añadió más
tarde) y está autenticado de extremo a extremo, así que el cliente no
puede construir ni alterar uno. El ecosistema de Rust ya tiene
AES-GCM como primitiva; usarlo le cuesta al framework un impl de
trait adicional y le da a cada cursor una propiedad de seguridad que
un payload base64 en texto plano no puede ofrecer.

## La fachada - `Pagination::length_aware` / `Pagination::cursor`

La mayoría de los capítulos de este manual muestran la paginación a
través del builder de Eloquent, porque esa es la ruta común. Si se
está construyendo un `Select<E>` de SeaORM directamente - digamos,
uniendo hacia una consulta sin modelo para un informe - la fachada
`Pagination` es la superficie equivalente:

```rust
use suprnova::{Pagination, LengthAwarePaginator};
use sea_orm::EntityTrait;

let select = User::find()  // o cualquier Select<E> de SeaORM
    .filter(user::Column::Active.eq(true));

let page: LengthAwarePaginator<user::Model> =
    Pagination::length_aware(select, 20, 1).await?;
```

La fachada también ofrece `length_aware_on(conn, ...)` y
`cursor_on(conn, ...)` para enrutar a una conexión con nombre
específica, y una forma tipada `cursor(query, cursor, per_page,
order_col)` que toma explícitamente la columna de keyset - usada
cuando el cursor ordena por algo distinto de la clave primaria.

Las reglas de enrutamiento coinciden con el builder de Eloquent. Una
`DB::transaction` ambiental se respeta (tanto el COUNT como la
consulta de página corren sobre la conexión de la transacción), y una
conexión `__read_replica__` registrada se usa automáticamente para
lecturas. El centinela `__primary__` selecciona el pool por defecto
cuando se quiere evitar la réplica.

## Validación - `per_page == 0`

Los tres métodos rechazan `per_page == 0`:

```rust
let result = User::query().paginate(0).await;
assert!(matches!(
    result,
    Err(FrameworkError::ParamError { ref param_name }) if param_name == "per_page",
));
```

El error se renderiza como HTTP 400 con el cuerpo de error estándar.
No hay ninguna "página vacía" silenciosa - un tamaño de página cero es
siempre incorrecto y se rechaza en el propio sitio de la llamada,
igual que el builder de Eloquent y la fachada `Pagination`. La misma
validación vive en `cursor_paginate`, `simple_paginate`,
`Pagination::length_aware`, `Pagination::length_aware_on`,
`Pagination::cursor`, y `Pagination::cursor_on` - una regla, seis
puntos de entrada.

El valor `current_page` se **fija** (clamp), no se valida: `0` se
convierte en `1`, los números negativos de un frontend defensivo no
pueden ocurrir (el parser es `u64`), y cualquier `?page=N` mayor que
`last_page` devuelve un paginador con `data` vacío más `from`/`to` en
`None`. Caminar más allá del final es el error del cliente, no un
error del servidor.

## Forma del error

| Condición | Variante | HTTP |
|---|---|---|
| `per_page == 0` | `FrameworkError::ParamError { param_name: "per_page" }` | 400 |
| Cursor manipulado / inválido | `FrameworkError::Domain` (`"Invalid pagination cursor"`) | 400 |
| `Crypt` no inicializado al decodificar el cursor | `FrameworkError::Internal` | 500 |
| Discordancia de variante de cursor en `decode_cursor` | `FrameworkError::Internal` | 500 |
| Fallo subyacente de base de datos | `FrameworkError::Database` | 500 |

El caso del cursor manipulado es el que hay que recordar. Los
cursores se leen directamente del wire - el query string `?cursor=…`
es entrada de un atacante por definición, y el base64 con bits
invertidos y el texto cifrado repetido son modos de fallo esperados,
no errores del servidor. El paso de descifrado degrada a un 400
`Invalid pagination cursor` para que los fallos disparables por el
cliente no contaminen el canal de telemetría de 500. El mensaje
estático no le da al cliente nada con qué sondear.

Los fallos posteriores al descifrado (parseo JSON, despacho por
etiqueta de variante, parseo de dirección) siguen siendo 500 -
cualquier secuencia de bytes que sobrevivió a la autenticación AEAD
fue producida por *nosotros*, así que un payload malformado más allá
de ese punto es un bug del framework que vale la pena señalar.

## Generación de URLs y rutas

El paginador en bruto lleva un campo `path` opcional. Cuando está
establecido, `url_for_page(n)` y la emisión de enlaces de cursor lo
usan para construir query strings:

```rust
let page = User::query()
    .paginate(20)
    .await?
    .with_path("/api/users");

page.url_for_page(1);    // "/api/users?page=1"
page.url_for_page(2);    // "/api/users?page=2"
```

Cuando la ruta base ya lleva un query string, el separador cambia a
`&` para que la URL siga siendo válida:

```rust
let page = User::query()
    .paginate(20)
    .await?
    .with_path("/users?sort=name");

page.url_for_page(2);    // "/users?sort=name&page=2"
```

Si `path` no está establecido, `url_for_page` recurre a un query
relativo desnudo: `?page=2`. El nombre del parámetro de página viene
de `with_page_name(...)` (por defecto `"page"`); `paginate_using(name,
n)` lo fija automáticamente para que las URLs generadas usen la misma
clave con la que se manejó el paginador. El nombre del parámetro está
codificado como form-urlencoded, así que incluso un nombre con
caracteres reservados no puede corromper la URL.

Los paginadores de cursor tienen la misma forma: `with_path(...)` fija
la base, `with_cursor_name(...)` sobrescribe la clave de query (por
defecto `"cursor"`), y el constructor de enlaces JSON:API los recoge
automáticamente.

La mayoría de las apps no llaman a `url_for_page` directamente -
entregan el paginador a una de las dos superficies de integración de
abajo, que construyen las URLs de la forma correcta para su protocolo.

## Integración con Inertia - props de scroll infinito

Para frontends Inertia, el helper
`Inertia::paginate(component, key, paginator)` adjunta el paginador
como una prop de scroll:

```rust
use suprnova::Inertia;

pub async fn index(_req: suprnova::Request) -> suprnova::Response {
    let users = User::query()
        .order_by_desc("created_at")
        .cursor_paginate(20)
        .await?;

    Ok(Inertia::paginate("Users/Index", "users", users).into())
}
```

Los tres paginadores funcionan aquí - `LengthAwarePaginator`,
`Paginator`, y `CursorPaginator`. El nombre de página de los metadatos
viene del propio paginador: `"page"` para los dos paginadores por
offset, `"cursor"` para `CursorPaginator`. El cliente recibe las filas
bajo la clave de prop elegida más un descriptor `ScrollMetadata` con
`current_page`, `next_page`, `previous_page` (identificadores de
página para los paginadores por offset; cadenas de cursor para los
paginadores por cursor) - que los helpers `useInfiniteScroll` /
`WhenVisible` de Inertia consumen para el scroll infinito.

Cada paginador construye ese descriptor mediante
`ProvidesScrollMetadata`, la misma interfaz que satisface el adaptador
de paginadores de Laravel (`ProvidesScrollMetadata::getPageName` /
`getPreviousPage` / `getNextPage` / `getCurrentPage`). Un paginador que
este crate no conozca - el tipo de cursor de un crate de terceros o el
resultado de un repositorio escrito a mano - puede implementar los cuatro
métodos y entregar al framework un `ScrollMetadata` de la misma forma:
consulta [Respuestas de Inertia](frontend-inertia-responses.md#merge-strategies-and-infinite-scroll).

`simple_paginate` merece mención aparte, porque un listado sobre una
tabla lo bastante grande para que `COUNT(*)` sea el coste dominante de
la solicitud es exactamente donde una página de colección de Inertia
duele:

```rust
let users = User::query()
    .order_by_asc("id")
    .simple_paginate(20)     // sin COUNT, una consulta
    .await?;

Ok(Inertia::paginate("Users/Index", "users", users).into())
```

Su `next_page` viene de la sonda de desborde de `LIMIT n+1` en lugar
de una última página calculada, ya que no hay total del que
calcularla. El cliente recibe "hay otra página" en lugar de "hay
4812 páginas" - que es todo lo que una UI de scroll infinito lee
jamás.

### Proyectar filas antes de que salgan

Los paginadores no tienen `map` / `through` (los de Laravel sí).
Reconstruye a partir de los campos públicos en su lugar - los
contadores y cursores describen la *consulta*, así que se conservan
sin cambios a través de un cambio de tipo de fila:

```rust
let page = User::query().cursor_paginate(20).await?;

let page = suprnova::CursorPaginator::new(
    page.data.into_iter().map(PublicUser::from).collect(),
    page.per_page,
    page.next_cursor,
    page.prev_cursor,
);
```

Vale la pena hacerlo en lugar de serializar el modelo directamente
siempre que la ruta no esté autenticada y el modelo lleve algo que
quien llama no debería ver. Un cursor sobre una tabla de usuarios
entrega una página a la vez, pero termina entregando cada página.

El mismo helper existe como método encadenable en
`InertiaResponse::paginate(key, paginator)` si se quiere mezclar un
paginador con otras props:

```rust
inertia_response!("Dashboard")
    .with("stats", &stats)
    .paginate("recent_users", users)
    .into()
```

Consulta [Respuestas de Inertia](frontend-inertia-responses.md) para
el modelo de props más amplio.

## Integración con JSON:API - `Resource::paginated`

Para consumidores JSON:API, `Resource::paginated(paginator)` construye
el envoltorio completo:

```rust
use suprnova::Resource;

pub async fn index(_req: suprnova::Request) -> suprnova::Response {
    let users = User::query()
        .paginate(20)
        .await?
        .with_path("/api/users");

    Ok(Resource::paginated(users).into())
}
```

La respuesta lleva:

- `data` - cada fila renderizada a través de `IntoJsonResource` del
  modelo.
- `meta.pagination` - `{ total, per_page, current_page, last_page }`
  para consciente de longitud; `{ next_cursor, prev_cursor }` para
  cursor.
- `links.{self,first,last,prev,next}` - hrefs absolutos para el
  paginador consciente de longitud (construidos a partir de `path`);
  `links.{prev,next}` para el paginador por cursor.

Ambos tipos de paginador implementan el trait `Paginated<T>` que
consume `Resource::paginated` - no hay una ruta de código separada
para consciente-de-longitud frente a cursor. Si se construye un tipo
similar a un paginador personalizado que implemente `Paginated<T>`,
compone de la misma manera.

Consulta [recursos JSON:API](eloquent-resources.md) para el modelo de
recurso.

## Envoltorios JSON personalizados

Si ni Inertia ni JSON:API coinciden con el cliente, envía el paginador
directamente a través de `json_response!`:

```rust
let page = User::query().paginate(20).await?;
Ok(suprnova::json_response!({
    "users": page.data,
    "pagination": {
        "current_page": page.current_page,
        "last_page": page.last_page,
        "per_page": page.per_page,
        "total": page.total,
    }
}))
```

O simplemente entrega todo el paginador - el impl de `Serialize`
derivado emite la forma documentada arriba:

```rust
Ok(suprnova::json_response!(User::query().paginate(20).await?))
```

Los campos son públicos; remodela como lo requiera el contrato.

## Enrutamiento entre conexiones

La paginación respeta el mismo enrutamiento multiconexión que usa el
builder de Eloquent. Dentro de un `DB::transaction(...)` el COUNT y la
consulta de página corren ambos sobre la conexión de la transacción -
nunca se dividen entre conexiones, así que el conteo nunca discrepa
con la página que describe. Una `__read_replica__` registrada se usa
automáticamente para lecturas fuera de una transacción. Para fijar un
paginador a una conexión con nombre específica usa las variantes
`_on(connection, ...)` sobre la fachada `Pagination`, o
`Builder::on("replica_b").paginate(20)` desde el lado de Eloquent.

Consulta [Eloquent - enrutamiento multiconexión](eloquent.md) para el
contrato de enrutamiento.

## Cuándo recurrir a cuál

Un árbol de decisión aproximado:

- **La UI de páginas numéricas es parte del diseño** → `paginate`. Se
  necesita `last_page` para renderizar "Página 3 de 17", y el coste
  del COUNT está bien para el tamaño de la tabla.
- **Solo botones "Siguiente" / "Anterior", tabla grande** →
  `simple_paginate`. Una consulta por página; se renuncia a `total` y
  `last_page` pero la carga de página se reduce a la mitad.
- **Scroll infinito** → `cursor_paginate`. Los cursores bidireccionales
  significan que el cliente puede seguir haciendo scroll más allá de
  la página 1000 sin que el OFFSET escanee primero miles de filas.
- **Cola de un feed activo de solo-anexar** → `cursor_paginate`. El
  orden por keyset sobre la clave primaria es seguro bajo
  concurrencia: las filas nuevas caen más allá del cursor, nunca
  dentro de él. La paginación por OFFSET se salta filas bajo
  inserciones.
- **Construir un `Select<E>` fuera de un modelo de Eloquent** →
  `Pagination::length_aware` / `Pagination::cursor`. Las mismas
  compensaciones; la fachada es el equivalente sin modelo.

Ante la duda, empieza con `paginate`. Pasa a `simple_paginate` cuando
el `COUNT(*)` aparezca en el log de consultas lentas. Pasa a
`cursor_paginate` cuando las páginas profundas empiecen a dominar el
tiempo de solicitud, o cuando la UI sea de scroll infinito.

## Dónde vive cada pieza

| Pieza | Archivo |
|---|---|
| Fachada `Pagination`, trait `Paginated<T>` | `framework/src/pagination/mod.rs` |
| `LengthAwarePaginator<T>` | `framework/src/pagination/length_aware.rs` |
| `Paginator<T>` (simple) | `framework/src/pagination/simple.rs` |
| `CursorPaginator<T>`, `CursorDirection`, `encode_value`, `decode_value` | `framework/src/pagination/cursor.rs` |
| Puente `IntoInertiaScroll` | `framework/src/pagination/inertia.rs` |
| `Builder::paginate` / `simple_paginate` / `cursor_paginate` | `framework/src/eloquent/builder.rs` |
| `Inertia::paginate`, `InertiaResponse::paginate` | `framework/src/inertia/facade.rs`, `framework/src/inertia/response.rs` |
| `Resource::paginated`, `JsonApi::paginated` | `framework/src/resources/response.rs` |

## Siguiente

- [API de Eloquent](eloquent.md) - la capa de modelo que conduce cada
  paginador devuelto por `Builder::paginate*`
- [Constructor de consultas](queries.md) - las consultas sin modelo
  que componen con `Pagination::length_aware` y `Pagination::cursor`
- [Respuestas de Inertia](frontend-inertia-responses.md) - cómo las
  props de scroll adjuntan paginadores a las páginas de Inertia
- [recursos JSON:API](eloquent-resources.md) - `Resource::paginated`,
  enlaces, meta, y el trait `Paginated<T>`
- [Modelo de errores](error-model.md) - la regla de validación
  `FrameworkError::param` y la degradación por manipulación de cursor
