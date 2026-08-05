# Serialización de Eloquent

Cómo los modelos de Eloquent se convierten en JSON. El capítulo cubre
`to_array()` y `to_json()`, el pipeline de filtros
`hidden` / `visible` / `appends`, los dos ayudantes terminales
`to_array_except` / `to_array_only`, la forma en que `appends`
conecta los accesores con la salida, y las dos divergencias respecto
a Laravel que sorprenden a la gente: la trampa del bypass de serde, y
el hecho de que las relaciones precargadas no se incorporan
automáticamente al cuerpo JSON.

Si has leído [la API de Eloquent](eloquent.md), la mayoría de los
nombres aquí te resultarán familiares - la referencia de atributos
está en ese capítulo. Esta página es donde vive el *contrato de
serialización*: qué campos aparecen, en qué orden se aplican los
filtros, y qué produce una fuga si lo olvidas.

## Tabla de contenidos

- [El contrato](#el-contrato)
- [`to_array` y `to_json`](#to-array-y-to-json)
- [Ocultar campos - `hidden = [...]`](#ocultar-campos-hidden)
- [Campos en lista blanca - `visible = [...]`](#campos-en-lista-blanca-visible)
- [Añadir accesores - `appends = [...]`](#añadir-accesores-appends)
- [El orden del pipeline de filtros](#el-orden-del-pipeline-de-filtros)
- [Filtrado por llamada - `to_array_except` / `to_array_only`](#filtrado-por-llamada-to-array-except-to-array-only)
- [Ocultación condicional según el usuario](#ocultación-condicional-según-el-usuario)
- [La trampa del bypass de serde](#la-trampa-del-bypass-de-serde)
- [Serializar colecciones](#serializar-colecciones)
- [Relaciones precargadas y serialización](#relaciones-precargadas-y-serialización)
- [¿Qué pasa con JSON:API?](#qué-pasa-con-json-api)
- [Dónde vive cada pieza](#dónde-vive-cada-pieza)
- [Siguiente](#siguiente)

## El contrato

Todo struct `#[suprnova::model]` recibe dos métodos de serialización
del trait `Model`:

```rust
fn to_array(&self) -> serde_json::Value;
fn to_json(&self) -> String;
```

`to_array` produce un `serde_json::Value` para usar en respuestas de
handler y en tests. `to_json` es un envoltorio delgado -
`serde_json::to_string(&self.to_array())` - así que un único pipeline
de filtros es dueño de las dos formas.

La salida es un objeto JSON indexado por el nombre de campo del
struct (o el rename de serde que hayas aplicado), filtrado a través
de tres perillas opcionales declaradas en `#[model(...)]`:

- `hidden = [...]` - lista de bloqueo de columnas
- `visible = [...]` - lista blanca de columnas (mutuamente exclusiva
  con `hidden`)
- `appends = [...]` - métodos accesores para inyectar bajo claves con
  nombre

Cuando el modelo no declara ninguna de estas, corre el cuerpo por
defecto del trait: serializa `self` vía `serde_json::to_value(self)`,
elimina dos campos auxiliares internos del framework (`__eager` y
`__pivot` - ver
[relaciones precargadas](#relaciones-precargadas-y-serialización)),
y devuelve el resultado. Cuando el modelo declara alguna de ellas, la
macro emite un override que ejecuta el
[pipeline](#el-orden-del-pipeline-de-filtros).

## `to_array` y `to_json`

El ejemplo mínimo útil - una fila que sale hacia el cliente como
JSON:

```rust
use suprnova::{json_response, model, Model, Request, Response};
use chrono::{DateTime, Utc};

#[model(table = "users")]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| suprnova::FrameworkError::param_parse("id", "i64"))?;
    let user = User::find_or_fail(id).await?;
    json_response!(user.to_array())
}
```

`json_response!` acepta cualquier `serde_json::Value`;
`user.to_array()` produce uno. El equivalente con forma de string es
`user.to_json()` - mismo cuerpo, mismos filtros, solo un `to_string`
adicional.

También puedes recurrir directamente a
`serde_json::to_value(&user)`. **No lo hagas para nada de cara al
usuario.** Eso salta el pipeline de filtros por completo - ver
[la trampa del bypass de serde](#la-trampa-del-bypass-de-serde) más
adelante en el capítulo para saber por qué.

## Ocultar campos - `hidden = [...]`

La forma de lista de bloqueo. Todas las columnas excepto las
listadas se serializan:

```rust
use chrono::{DateTime, Utc};
use suprnova::{model, Model};

#[model(
    table = "users",
    fillable = ["name", "email", "password"],
    hidden = ["password", "remember_token"],
)]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub password: String,
    pub remember_token: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

El JSON de cara al usuario para este modelo nunca contiene
`password` ni `remember_token`:

```json
{
    "id": 42,
    "name": "Alice",
    "email": "alice@example.com",
    "created_at": "2026-05-30T11:14:22Z",
    "updated_at": "2026-05-30T11:14:22Z"
}
```

`hidden` es la herramienta correcta cuando **la mayoría de los
campos llegan hasta el cliente** y necesitas restar un pequeño
conjunto de secretos, flags internos, o datos solo de auth.

## Campos en lista blanca - `visible = [...]`

La forma de lista de permitidos. Solo las columnas listadas se
serializan:

```rust
#[model(
    table = "users",
    visible = ["id", "name", "avatar_url"],
)]
pub struct PublicUserView { /* ... */ }
```

Útil para un modelo que existe específicamente para ser una
proyección pública delgada (piensa en los tipos "Profile" /
"PublicUser" de Laravel). `visible` también es la herramienta
correcta cuando la tabla tiene docenas de columnas internas y solo
unas pocas pertenecen al cliente - listar el conjunto a conservar es
más corto que listar el conjunto a quitar.

`hidden` y `visible` son **mutuamente exclusivas en tiempo de
compilación**. La macro emite un error si fijas ambas:

```text
error: cannot specify both `hidden` and `visible` on the same model
 --> src/models/user.rs:7:1
  |
7 | #[model(table = "users", hidden = ["x"], visible = ["y"])]
  | ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
```

Las dos son opuestos de política - elige la que combine con la
intención de tu modelo, no ambas.

## Añadir accesores - `appends = [...]`

`appends` inyecta valores calculados en la salida JSON. Cada entrada
nombra un método marcado con `#[accessor]` en el modelo; la macro lo
llama durante `to_array()` y guarda el valor de retorno bajo la misma
clave.

```rust
use suprnova::{accessor, model, Model};

#[model(
    table = "users",
    fillable = ["first_name", "last_name"],
    appends = ["full_name", "initials"],
)]
pub struct User {
    pub id: i64,
    pub first_name: String,
    pub last_name: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl User {
    #[accessor]
    pub fn full_name(&self) -> String {
        format!("{} {}", self.first_name, self.last_name)
    }

    #[accessor]
    pub fn initials(&self) -> String {
        let f = self.first_name.chars().next().unwrap_or(' ');
        let l = self.last_name.chars().next().unwrap_or(' ');
        format!("{f}{l}")
    }
}
```

El usuario serializado ahora lleva ambas claves calculadas:

```json
{
    "id": 7,
    "first_name": "Alice",
    "last_name": "Pond",
    "created_at": "...",
    "updated_at": "...",
    "full_name": "Alice Pond",
    "initials": "AP"
}
```

La macro valida las entradas de `appends` en tiempo de compilación:

- Cada nombre debe analizarse como un identificador de Rust válido
  (`"full-name"` falla - no es un ident válido).
- Si el método nombrado no existe en el bloque `impl` del modelo, el
  compilador señala al dispatcher generado por la macro con un error
  claro `no method named 'full_name' found`.

Llamar a `user.full_name()` directamente desde Rust funciona
exactamente como cualquier otro método - `appends` solo controla la
**tabla de despacho JSON**. Los accesores siguen siendo métodos
normales.

## El orden del pipeline de filtros

Cuando un modelo declara `hidden`, `visible`, o `appends`, la macro
emite un override de `to_array` que ejecuta cuatro pasos en este
orden:

1. Serializa `self` a un `serde_json::Map` vía `serde_json::to_value`.
2. Elimina las claves internas del framework `__eager` y `__pivot`
   sin condición (más sobre esto en
   [la sección de relaciones](#relaciones-precargadas-y-serialización)).
3. Aplica `visible` como **lista blanca** cuando no está vacía: se
   elimina toda clave que NO esté en la lista.
4. Aplica `hidden` como **lista de bloqueo**: se elimina toda clave
   listada que haya sobrevivido a la lista blanca.
5. Inyecta `appends`: para cada entrada, llama al accesor registrado
   e inserta su resultado bajo el nombre de la entrada.

### Por qué Suprnova diverge

Laravel ejecuta el mismo orden `hidden` → `visible` → `appends`. La
divergencia está en el paso 5: en Suprnova, `appends` corre
**después** de la lista de bloqueo `hidden`, y siempre aparece -
incluso si su nombre también está listado en `hidden`. El
razonamiento es el mismo que el de Laravel: si declaras a la vez
`$appends = ['full_name']` y `$hidden = ['full_name']`, la intención
es "calcúlalo y envíalo" - `appends` es la señal más específica. El
orden importa cuando la clave de un accesor colisiona con el nombre
de una columna (por ejemplo, un accesor que sobrescribe el valor de
la columna almacenada `display_name`); el accesor gana en la
respuesta.

## Filtrado por llamada - `to_array_except` / `to_array_only`

Para casos puntuales donde la declaración a nivel de columna no
encaja, dos ayudantes terminales ejecutan el pipeline completo de
`to_array` y luego recortan el resultado por nombre:

```rust
use suprnova::{json_response, Model};

pub async fn admin_show(user: User) -> suprnova::Response {
    // quita algunos campos extra para un endpoint de admin que
    // necesita casi toda la fila salvo estos:
    json_response!(
        user.to_array_except(&["password_hash", "remember_token", "internal_notes"])
    ))
}

pub async fn directory_show(user: User) -> suprnova::Response {
    // directorio público - solo las columnas que queremos publicar:
    json_response!(
        user.to_array_only(&["id", "name", "avatar_url"])
    ))
}
```

Ambos producen un `serde_json::Value` - no mutan `self` y no cambian
serializaciones futuras de la misma fila. Ejecutan primero el
pipeline completo `hidden` / `visible` / `appends`, y luego aplican
su propio recorte encima. `to_array_only` devuelve un objeto JSON
*nuevo* que contiene solo las claves nombradas; `to_array_except`
devuelve el objeto completo menos las claves nombradas.

### Por qué Suprnova diverge

`$user->makeHidden(['x'])` y `$user->makeVisible(['x'])` de Laravel
**mutan** la instancia del modelo - toda llamada a `toArray()`
posterior, incluidas las que ocurren cuando el modelo está anidado
dentro de la serialización de un padre, ve el estado cambiado. Los
ayudantes de Suprnova son **terminales**. Producen un `Value` y se
detienen ahí. Si necesitas que el cambio se propague, decláralo en
`#[model(hidden = [...])]` / `#[model(visible = [...])]` para que
sea el *tipo* el que exprese la política, no una mutación oculta
sobre la instancia.

La razón con forma de Rust: un struct de Eloquent en Suprnova es un
struct de Rust plano sin bolsa de atributos en tiempo de ejecución.
No hay lugar para que un flag de visibilidad del lado de la
instancia viva sin añadir estado oculto ambiental, que es
precisamente el tipo de trampa que el framework evita a propósito.

## Ocultación condicional según el usuario

El patrón idiomático cuando la visibilidad depende de quién consulta
es un match en el sitio de la llamada, que ramifica hacia el filtro
correcto por llamada:

```rust
use suprnova::{Auth, json_response, Model, Request, Response};

pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| suprnova::FrameworkError::param_parse("id", "i64"))?;
    let user = User::find_or_fail(id).await?;
    let viewer = Auth::user_as::<User>().await?;
    let viewing_self = viewer.as_ref().map(|v| v.id) == Some(user.id);

    let body = if viewing_self {
        user.to_array()
    } else {
        user.to_array_except(&["email", "phone", "stripe_customer_id"])
    };

    json_response!(body)
}
```

Para una forma más elaborada por usuario - atributos distintos para
admins, usuarios en prueba, usuarios de pago - la herramienta
correcta es la **capa de recursos JSON:API** con campos `Maybe<T>` /
`MissingValue<T>`. Consulta
[Recursos JSON:API](eloquent-resources.md#conditional-attributes--maybet--missingvaluet)
para la forma declarativa.

## La trampa del bypass de serde

Esto es lo más importante que hay que saber sobre la serialización
de Eloquent en Suprnova.

**Los filtros `hidden` / `visible` / `appends` solo corren a través
de `to_array()` y `to_json()`.** El impl derivado de `Serialize` *no*
los aplica. Devolver el struct por cualquier otra ruta de serde salta
los filtros por completo.

Eso significa que **todo lo siguiente filtra `password`**:

```rust
// serde directo - salta to_array, hidden no tiene efecto:
let raw = serde_json::to_value(&user).unwrap();

// json_response! con un campo de struct - lo mismo:
json_response!({ "user": user }))

// Anidado dentro de otro contenedor serializable - lo mismo:
#[derive(Serialize)]
struct EnvelopeWithUser { ok: bool, user: User }
let env = EnvelopeWithUser { ok: true, user };
json_response!(env))

// Devolver un Vec<User> a través de serde - lo mismo:
json_response!(users))   // donde users: Vec<User>
```

Solo estos pasan por el pipeline de filtros:

```rust
json_response!(user.to_array()))
json_response!(users_collection.to_array()))  // Collection<User>
json_response!(user.to_array_except(&["secret"])))
json_response!(user.to_array_only(&["id", "name"])))
```

### Por qué sucede esto

El impl general `Serialize for Vec<T>` de serde (y de cualquier otro
contenedor) llama a `T::serialize` directamente. El pipeline de
filtros de Suprnova vive en el método de trait `Model::to_array`, no
en `Serialize`. El método de trait no se invoca a menos que lo
llames tú.

El framework se protege contra la trampa *interna* (los campos
auxiliares `__eager` / `__pivot` están marcados con
`#[serde(skip)]`, así que tampoco se filtran por ninguna de las dos
rutas), pero la macro deliberadamente **no** emite
`#[serde(skip_serializing)]` sobre los campos ocultos - hacerlo
rompería usos legítimos de serde con el modelo SeaORM interno, en los
que quien llama quiere la fila completa (por ejemplo, RPC interno,
capas de persistencia, diagnóstico, tests).

### La regla

Para cualquier valor que cruce el límite de confianza de vuelta hacia
un cliente, pasa por `to_array()` o alguno de sus métodos hermanos
filtrados. El contrato de cuatro líneas que te garantiza la
seguridad:

| Quieres | Usa | Resultado |
|---|---|---|
| Serializar un modelo | `user.to_array()` | Objeto JSON filtrado |
| Serializar una colección | `collection.to_array()` | Array JSON filtrado |
| Restar unos pocos campos | `user.to_array_except(&["x"])` | Filtrado + restado |
| Conservar solo unos pocos campos | `user.to_array_only(&["x"])` | Solo las claves listadas |

Un linter o una revisión en tiempo de PR para
`json_response!\({.*: [a-z_]+ ?})` y `serde_json::to_value\(&\w+\)`
sobre valores de modelo es una forma barata de mantener la regla.
Los propios tests del framework para la serialización de `Model`
cubren ambas rutas.

## Serializar colecciones

Una `Collection<M>` - devuelta por `Builder::get()`, `Model::all()`,
y los accesores de relación - tiene su propio `to_array()` y
`to_json()` que recorren el `Vec<M>` subyacente y llaman a
`to_array()` **por fila**. El resultado es un array JSON de objetos
filtrados:

```rust
use suprnova::{json_response, Model};

pub async fn list() -> suprnova::Response {
    let users = User::all().await?;
    json_response!(users.to_array())
}
```

Este es el único lugar para obtener el filtro por fila sobre un
resultado de varias filas. `serde_json::to_value(&users)` emitiría
un Vec vía el impl general de serde y saltaría los filtros de todas
las filas a la vez - el ayudante a nivel de colección existe
justamente para cerrar esa brecha.

```rust
// El override de Collection<M>:
pub fn to_array(&self) -> Value {
    Value::Array(self.0.iter().map(|m| m.to_array()).collect())
}
```

Para un paginador, los datos envueltos viven en
`LengthAwarePaginator::data / CursorPaginator::data` y son un
`Vec<M>` - llama a `.to_array()` en cada elemento antes de armar la
respuesta del paginador, o usa la
[forma paginada de JSON:API](eloquent-resources.md#pagination), que
maneja el filtrado por fila como parte del pipeline de recursos.

## Relaciones precargadas y serialización

Esta es la segunda divergencia que hay que internalizar.

Cuando llamas a `.with(["posts"])` en un builder, el framework carga
los posts y los guarda en una `EagerLoadCache` por fila (el campo
`__eager` auto-inyectado). El accesor para leerlos -
`user.posts_loaded()` - toma los datos de esa caché.

**La caché es `#[serde(skip)]` y `to_array()` la elimina sin
condición.** Las relaciones precargadas no se incorporan
automáticamente a la salida JSON. Un `to_array()` sobre un usuario
con posts precargados es idéntico a un `to_array()` sobre un usuario
sin ellos.

### Por qué Suprnova diverge

El `toArray()` de Laravel recorre `$model->getRelations()` e
incorpora cada relación cargada a la salida. La bolsa de modelo con
forma de array de PHP hace esto natural - una relación es solo otra
entrada con clave sobre el modelo.

Los structs de Eloquent tipados de Rust no tienen esa bolsa. Un
struct `User` tiene columnas tipadas, no un mapa heterogéneo de
"cualesquiera relaciones que se hayan cargado". Incorporar `posts`
exigiría o bien inyección de campos en tiempo de ejecución sobre un
struct tipado (un mecanismo de bypass de serde), o bien una ruta de
serialización paralela que consulte la caché después de correr el
serializador de columnas. Ambas opciones acoplarían la forma JSON de
cada modelo a qué relaciones precargó un llamador en particular - un
contrato que en PHP es estructural, porque los clientes aprenden a
depender de él, y un contrato que Suprnova se niega explícitamente a
ofrecer porque hace que la forma del JSON dependa de la construcción
de la consulta del lado del llamador.

### Las dos formas de entregar datos de relación

**1. Accesor explícito + appends.** Define un método que toma datos
de `<rel>_loaded()`, regístralo en `appends`. La relación aparece
bajo la clave que nombres. Esto funciona cuando la relación *siempre*
se precarga en la ruta de lectura:

```rust
use suprnova::{accessor, model};
use serde_json::Value;

#[model(
    table = "users",
    appends = ["posts"],
)]
pub struct User { /* ... */ }

impl User {
    #[accessor]
    pub fn posts(&self) -> Value {
        // posts_loaded() ENTRA EN PÁNICO si no se llamó a
        // .with(["posts"]) en la ruta de lectura. El accesor DEBE
        // correr después de la precarga.
        let posts = self.posts_loaded();
        serde_json::to_value(posts).unwrap_or(Value::Null)
    }
}

// La ruta de lectura DEBE precargar:
let users = User::query()
    .with(["posts"])
    .get()
    .await?;
let body = users.to_array();   // la clave "posts" de cada usuario está poblada
```

El contrato es ruidoso: olvida el `.with(["posts"])`, y el accesor
entra en pánico en la primera llamada a `posts_loaded()` de una fila
(la caché de precarga entra en pánico al leer cuando la relación no
se cargó, por diseño - un array vacío silencioso escondería el bug).
Para precarga opcional, usa la forma HasOne, que devuelve
`Option<&T>` y te da un `match`:

```rust
impl User {
    #[accessor]
    pub fn profile(&self) -> Value {
        match self.profile_loaded() {
            Some(profile) => serde_json::to_value(profile).unwrap_or(Value::Null),
            None => Value::Null,
        }
    }
}
```

**2. La capa de recursos JSON:API.** Cuando la forma de la relación
y la política de inclusión pertenecen al formato en la red en lugar
de al modelo, usa un struct `#[derive(Data)] #[json_resource]` con
`#[data(allow_include)]` en el campo de relationship. Los clientes se
suman vía `?include=posts.comments`, el framework recorre el árbol
de include, y llena `included` con objetos de recurso deduplicados.
Esta es la respuesta correcta cuando:

- La forma de la relación es un asunto de formato en la red (campos
  dispersos, inclusión condicional, metadatos de enlace cruzado).
- Distintos endpoints quieren distintas inclusiones por defecto.
- El mismo modelo aparece bajo envolturas distintas (un endpoint
  envía `posts`, otro envía `subscriptions`).

Consulta
[Recursos JSON:API](eloquent-resources.md#compound-documents--include-chains)
para el patrón completo.

## ¿Qué pasa con JSON:API?

El pipeline de `to_array()` y la fachada `Resource` / `JsonApi` son
dos capas, y cumplen trabajos distintos:

| Aspecto | `Model::to_array` | `Resource::single` / `JsonApi::single` |
|---|---|---|
| **Forma** | Objeto plano - los nombres de columna mapean directo a claves | Envoltura JSON:API (`data`, `included`, `meta`, `links`, `jsonapi`) |
| **Control por atributo** | `hidden` / `visible` / `appends` en `#[model]` | `#[data(input_only)]`, `Maybe<T>`, campos dispersos vía `?fields[type]=` |
| **Relaciones** | Manual (accesor + appends, ver arriba) | De primera clase vía `#[data(allow_include)]` + `?include=` |
| **Paginación** | Envolver un `Vec<Value>` a mano | `Resource::paginated(p)` maneja links + meta |
| **Errores** | Renderizar a través de `FrameworkError` | `into_json_api_response()` produce una envoltura de `errors` JSON:API |
| **Cuándo recurrir a ella** | Endpoints simples, herramientas internas, formas ad-hoc | APIs públicas, consumidores externos, clientes que conocen JSON:API |

`to_array()` es la capa inferior - es lo que se llama para la mayoría
de los handlers internos, páginas de admin, props de Inertia (vía
serde), y tests. La capa JSON:API se compone encima: no reemplaza a
`to_array`, añade una envoltura alrededor de la lógica de
attributes/relationships por recurso que es demasiado rica para vivir
en el propio modelo.

Para props de Inertia tipadas casi siempre quieres la capa de
recursos o un DTO dedicado `#[derive(Serialize)]` con campos
explícitos, en lugar de pasar el modelo por serde directamente. Los
retornos de Inertia reciben el mismo trato de bypass de serde que
todo lo demás - la ruta segura es "construye un DTO, llénalo desde
`to_array()`, devuelve el DTO".

## Dónde vive cada pieza

| Aspecto | Archivo |
|---|---|
| Valores por defecto del trait `Model::to_array` / `to_json` | `framework/src/eloquent/model.rs` |
| `Model::to_array_except` / `to_array_only` | `framework/src/eloquent/model.rs` |
| Valor por defecto del trait `Model::__append_accessor` | `framework/src/eloquent/model.rs` |
| Override de `to_array` emitido por macro (pipeline de filtros) | `suprnova-macros/src/model/serialization.rs` |
| Dispatcher de `__append_accessor` emitido por macro | `suprnova-macros/src/model/serialization.rs` |
| `Collection<M>::to_array` / `to_json` | `framework/src/eloquent/collection.rs` |
| `EagerLoadCache` (el campo `__eager`) | `framework/src/eloquent/relations/eager_cache.rs` |
| Análisis por macro de `hidden` / `visible` / `appends` | `suprnova-macros/src/model/parse.rs` |
| Macro a nivel de función `#[accessor]` | `suprnova-macros/src/lib.rs` |

## Siguiente

- [API de Eloquent](eloquent.md) - la superficie completa del
  modelo, la referencia de atributos, y dónde se definen
  `#[accessor]` / `#[mutator]`
- [Recursos JSON:API](eloquent-resources.md) - la capa de recursos
  declarativa para formas más ricas por usuario, campos dispersos, y
  documentos compuestos `?include=`
- [Validación](validation.md) - cómo la entrada de la solicitud se
  convierte en un struct tipado antes de que la capa de modelo la vea
- [Respuestas](responses.md) - builders de `HttpResponse`,
  encabezados, y cookies; la superficie que `json_response!` produce
  en última instancia
- [Modelo de errores](error-model.md) - cómo un error se convierte en
  un cuerpo JSON con la misma correlación de `request_id` que la ruta
  de éxito
