# Contexto

`Context` es la bolsa de clave/valor por solicitud de Suprnova. Es donde
guardas los datos que quieres que vea todo el que llame más abajo dentro
de la misma solicitud - un id de solicitud, un slug de tenant, un rol de
usuario, un rastro de auditoría - sin enhebrar el valor por cada firma de
función. Es el equivalente en Suprnova de la fachada `Context` de
Laravel.

```rust
use suprnova::Context;

Context::add("tenant_id", "acme");
Context::push("breadcrumbs", "checkout/start");
Context::hidden_add("api_key", secret);

let tenant: Option<String> = Context::get("tenant_id");
let page: Option<String> = Context::query_param("page");
```

Echa mano de ella cuando:

- Una línea de registro, un job en cola o un mensaje de difusión
  necesita metadatos acotados a la solicitud (id de tenant, id de
  correlación, rol de usuario)
- Un ayudante muy anidado necesita un valor que el handler ya tiene, pero
  la cadena de llamadas no debería llevar un parámetro por todas las
  capas
- Quieres leer el query string de la solicitud actual (`?page=3`,
  `?cursor=…`) desde código que no es un handler

`Context` **no** sirve para el estado entre solicitudes. Está vinculada a
la tarea de Tokio actual y desaparece cuando la solicitud termina. Para
las cosas que sobreviven a una solicitud, usa el
[Contenedor de servicios](container.md) o la [Caché](cache.md).

## Las dos bolsas

Todo alcance `Context` activo lleva dos mapas de clave/valor y una ranura
extra:

| Bolsa | Se lee con | Aparece en `Context::all()` |
|---|---|---|
| **Visible** | `Context::get` | Sí |
| **Oculta** | `Context::hidden_get` | No |
| **Query** | `Context::query_param` | No (instantánea aparte de los pares `?key=value` de la URL) |

La separación entre visible y oculta es justo el motivo de tener dos
bolsas: los serializadores de registro que vuelcan `Context::all()` a la
salida estructurada no filtrarán los datos que ocultas a propósito. Pon
los metadatos de auditoría en la bolsa visible; pon en la bolsa oculta
las claves de API, los tokens bearer de OAuth y los datos personales que
no quieres en los registros.

La bolsa de query la puebla automáticamente el middleware de solicitud
del framework a partir del query string de la URL (consulta
[La paginación lee los parámetros del query](#la-paginación-lee-los-parámetros-del-query)
más abajo). Normalmente solo la lees, nunca la escribes.

## El alcance activo

El framework instala un alcance `Context` en cada solicitud HTTP
entrante. Dentro de un handler, un middleware, un observador de modelo,
un oyente de eventos o cualquier otra cosa alcanzable desde la tarea de
la solicitud, el alcance está vivo y las lecturas y escrituras de
`Context::*` funcionan sin ceremonias.

Fuera de un alcance - código de arranque temprano, un `tokio::spawn` a
secas que no hereda el contexto, un test unitario que no instala
ninguno - toda mutación **no hace nada, en silencio** y toda lectura
devuelve `None`. El contrato es: nunca hay pánico, da igual desde dónde
llames.

```rust
// En un handler - el alcance está activo, todo funciona:
Context::add("user_id", 42i64);
let id: Option<i64> = Context::get("user_id");
assert_eq!(id, Some(42));

// Fuera de un alcance - no hace nada en silencio + None:
Context::add("user_id", 42i64);            // descartado
let id: Option<i64> = Context::get("user_id");
assert_eq!(id, None);
```

El contrato de no entrar en pánico es deliberado. El código de biblioteca
que toca `Context` (un subscriber de registro propio, una extensión de
SDK) no debería tener que saber si se está ejecutando dentro de una
solicitud o en el arranque - debería limitarse a llamar a `Context::get`
y tratar `None` como "ahora mismo no está disponible".

### Observabilidad de las operaciones silenciosas

Una operación que no hiciera nada de forma verdaderamente silenciosa
ocultaría bugs (middleware en el orden equivocado, contexto no propagado
a una tarea lanzada, una lectura accidental en tiempo de arranque). Las
operaciones de mutación del framework siguen sin entrar en pánico, pero
emiten un evento `tracing::trace!` en el target `suprnova::context` cada
vez que descartan algo:

```text
TRACE suprnova::context: Context mutation discarded: no active scope on this task op="add"
TRACE suprnova::context: Context mutation discarded: value failed to serialize op="push" key="bad"
TRACE suprnova::context: Context read returned None: value present but did not deserialize op="get" key="user_id" expected="String"
```

Tres clases de evento:

| Evento | Cuándo se dispara |
|---|---|
| `mutation discarded: no active scope` | se llamó a `add`, `push`, `hidden_add` o `forget` fuera de cualquier alcance |
| `mutation discarded: value failed to serialize` | la implementación de `Serialize` del valor pasado a `add`/`push`/`hidden_add` devolvió error |
| `read returned None: value present but did not deserialize` | `get`/`hidden_get` encontró la clave, pero el JSON almacenado no coincide con el `T` solicitado |

La simple ausencia - un `get` sobre una clave que nunca se estableció -
se mantiene silenciosa para que los sondeos del tipo "¿está esto
establecido?" no inunden los registros. Habilita
`RUST_LOG=suprnova::context=trace` cuando sospeches de un bug de
propagación; la ruta silenciosa se vuelve visible sin cambiar el
comportamiento del código de producción.

## Añadir valores

### `Context::add` - reemplaza en una clave

```rust
use suprnova::Context;

Context::add("user_id", 42i64);
Context::add("tenant", "acme");
Context::add("plan", PlanTier::Pro);     // cualquier valor Serialize
```

La clave es `Into<String>`; el valor es cualquier tipo `Serialize`. El
valor se convierte a `serde_json::Value` una sola vez en el momento de la
escritura y se almacena así. Un `add` posterior sobre la misma clave lo
reemplaza.

### `Context::push` - añade a una pila

```rust
Context::push("trail", "home");
Context::push("trail", "settings");
Context::push("trail", "billing");

let trail: Vec<String> = Context::get("trail").unwrap();
assert_eq!(trail, vec!["home", "settings", "billing"]);
```

`push` inicializa un array vacío en la primera llamada y añade al final
en las siguientes. Si ya existe un escalar en esa clave, se convierte en
un array `[scalar, new_value]` - `push` es indulgente con los `add`
previos sobre la misma clave.

### `Context::hidden_add` - escribe en la bolsa oculta

```rust
Context::hidden_add("api_key", os_env_secret);
Context::hidden_add("oauth_bearer", token);

// Un volcado de la bolsa visible (por ejemplo, un emisor de registros JSON) no los ve:
let all = Context::all();
assert!(!all.contains_key("api_key"));

// Pero todavía puedes leerlos de forma deliberada:
let key: Option<String> = Context::hidden_get("api_key");
```

La bolsa oculta se indexa de forma independiente de la visible - un
`hidden_add("user_id", 99)` y un `add("user_id", "alice")` coexisten sin
colisionar. `Context::forget(key)` elimina de ambas bolsas en una sola
llamada.

## Leer valores

### `Context::get` - lectura tipada de la bolsa visible

```rust
use suprnova::Context;

let user_id: Option<i64>       = Context::get("user_id");
let tenant:  Option<String>    = Context::get("tenant");
let trail:   Option<Vec<String>> = Context::get("trail");
```

`get` es genérico sobre `T: DeserializeOwned`. El valor JSON almacenado
se deserializa en cada lectura. Devuelve `None` cuando:

- La clave no está establecida
- No hay ningún alcance activo en la tarea actual
- El valor almacenado no se deserializa a `T` (por ejemplo, guardaste un
  `i64` y pediste un `String`)

El último caso emite un `tracing::trace!` para que el bug de tipo
equivocado sea observable - que `Context::get` parezca decir "el valor no
está establecido" cuando en realidad dice "el valor tiene la forma
equivocada" es la clase de bug que cuesta una hora encontrar sin una
línea de registro que lo señale.

### `Context::hidden_get` - lectura tipada de la bolsa oculta

La misma forma que `get`, pero lee la bolsa oculta. El mismo
comportamiento de trazado ante un tipo equivocado.

### `Context::has` - comprobación de existencia en la bolsa visible

```rust
if Context::has("user_id") {
    // …
}
```

`has` solo comprueba la bolsa visible (usa `hidden_get(...).is_some()` si
necesitas sondear la bolsa oculta).

### `Context::all` - instantánea de la bolsa visible

```rust
let snapshot: HashMap<String, serde_json::Value> = Context::all();
```

Devuelve un `HashMap` vacío fuera de un alcance. Esto es lo que debería
llamar un emisor de registros JSON para inyectar campos acotados a la
solicitud en cada línea de registro - y el motivo por el que la bolsa
oculta existe por separado.

### `Context::forget` - elimina una clave de ambas bolsas

```rust
Context::forget("trail");          // elimina de la visible Y de la oculta
```

La eliminación en las dos bolsas es intencionada. Si guardaste datos
relacionados en ambas (por ejemplo, `user_id` visible y `user_email`
oculto), un solo `forget` limpia las dos.

## Leer los parámetros del query string

`Context::query_param` lee de los pares `?key=value` de la URL capturados
a la entrada de la solicitud. El middleware de solicitud analiza el query
string una sola vez y lo deja en la bolsa de query del alcance; a partir
de ahí, todo el que llame más abajo puede leer parámetros sueltos por su
nombre sin volver a analizarlo:

```rust
use suprnova::Context;

let page: Option<String>   = Context::query_param("page");
let cursor: Option<String> = Context::query_param("cursor");
let sort: Option<String>   = Context::query_param("sort");
```

Devuelve `None` cuando el parámetro falta o no hay ningún alcance activo.
Las claves duplicadas siguen la semántica de Laravel en la que gana la
última - el mismo valor que obtendrías del mapa de query ya analizado de
la solicitud.

### La paginación lee los parámetros del query

Este es el motivo por el que existe la bolsa de query. Los paginadores de
Eloquent leen `?page=` y `?cursor=` directamente de
`Context::query_param`, así que un handler que devuelve un paginador no
necesita cablear el número de página a mano:

```rust
use suprnova::{json_response, Request, Response};
use crate::models::Post;

pub async fn index(_req: Request) -> Response {
    // Lee ?page=N de la URL de la solicitud mediante Context::query_param
    // - sin código repetitivo de req.query(), sin enhebrar parámetros.
    let posts = Post::query()
        .order_by_desc("created_at")
        .paginate(15)
        .await?;

    json_response!(posts)
}
```

Tres puntos de entrada de paginación usan esto:

- `Builder::paginate(per_page)` - lee `?page=`
- `Builder::simple_paginate(per_page)` - lee `?page=`
- `Builder::cursor_paginate(per_page)` - lee `?cursor=`

Consulta [Paginación](pagination.md) para la superficie completa.

## Propagar a las tareas lanzadas

`tokio::spawn` arranca la tarea hija con un entorno de task-locals
nuevo - el alcance `Context` del padre **no** fluye hacia dentro. Un
`tokio::spawn` a secas dentro de una solicitud ve un `Context` vacío y
toda lectura devuelve `None`.

Para llevar el alcance a una tarea lanzada, toma una instantánea con
`Context::current()` y vuelve a entrar en él dentro de la hija con
`Context::scope`:

```rust
use suprnova::context::Context;

// Dentro de un handler de solicitud:
if let Some(store) = Context::current() {
    tokio::spawn(Context::scope(store, async move {
        // Ahora `Context::get`, `Context::query_param`, etc. ven la
        // bolsa de la solicitud padre.
        let request_id: Option<String> = Context::get("_request_id");
        do_background_work(request_id).await;
    }));
}
```

El store que devuelve `Context::current()` comparte los mapas subyacentes
del padre mediante `Arc` - las escrituras de la hija son visibles para el
padre mientras la hija conserve el clon. Esto es exactamente lo que
quieren las tareas lanzadas de auditoría y de registro: la hija puede
estampar claves adicionales (`Context::add("audit.completed", true)`) y
la línea de registro final del padre las ve.

Si necesitas una instantánea aislada (que las escrituras de la hija no se
filtren de vuelta), construye un `ContextStore` nuevo y copia en él solo
las claves que necesites.

### Por qué un `spawn` a secas no propaga

Los task-locals de Tokio (`tokio::task_local!`) están acotados a la tarea
a propósito. Heredarlos automáticamente al lanzar una tarea significaría:

- Las tareas en segundo plano de larga vida fijarían para siempre los
  mapas de contexto del padre
- Un pánico en una tarea hija podría envenenar el estado del padre
- El runtime tendría que recorrer una cadena de punteros al padre en cada
  lectura de un task-local

El baile explícito de `Context::current()` + `Context::scope` convierte
la propagación en una decisión deliberada en lugar de en un
comportamiento por defecto oculto.

## Pruebas

Dentro de `#[tokio::test]` o `#[suprnova_test]` no se instala ningún
alcance `Context` por defecto. La mayoría del código bajo prueba que toca
el contexto gestiona con elegancia el caso de "sin alcance" (no hace nada
en silencio + lecturas `None`), así que los tests unitarios simples no
necesitan ninguna preparación.

Dos situaciones en las que el test necesita ayuda:

### Cuando el código bajo prueba llama a `query_param`

Los ayudantes de paginación leen `?page=` mediante
`Context::query_param`. Un test unitario de "la página 3 devuelve el
offset correcto" necesita que `query_param` devuelva `Some("3")`. Hay dos
formas:

**`test_query_guard` (recomendado):**

```rust
use suprnova::Context;

#[tokio::test]
async fn paginate_reads_page_from_query() {
    let _q = Context::test_query_guard("page", "3");

    // El código bajo prueba ahora ve ?page=3
    assert_eq!(Context::query_param("page"), Some("3".into()));

    let posts = Post::query().paginate(15).await?;
    assert_eq!(posts.current_page(), 3);
}
// `_q` se libera al final del alcance - el override thread-local se borra.
```

`test_query_guard` devuelve una guarda RAII. Aunque el cuerpo del test
entre en pánico, `Drop` se ejecuta y limpia el override thread-local
antes de que se recicle el hilo del sistema operativo. La guarda es
`#[must_use]` - vincularla a `_` la limpia de inmediato, que casi nunca
es lo que quieres.

**`test_set_query` + `test_clear_query` a secas:**

```rust
#[tokio::test]
async fn manual_pair() {
    Context::test_clear_query();        // borra la fuga de cualquier test hermano
    Context::test_set_query("page", "5");

    // … aserciones …

    Context::test_clear_query();
}
```

Usa la forma con guarda. El par manual existe para los casos en que
necesitas establecer y limpiar varios overrides de forma independiente,
pero la guarda `#[must_use]` es más difícil de usar mal.

Ambas API están detrás de `#[cfg(any(test, feature = "testing"))]` - se
compilan dentro de los binarios de test y dentro de las compilaciones de
release que optan por la feature `testing` para los harness de pruebas de
integración. No existen en las compilaciones de release normales.

### Cuando el código bajo prueba lee o escribe desde un alcance `Context`

Instala uno de forma explícita con `Context::scope`:

```rust
use suprnova::context::{Context, ContextStore};

#[tokio::test]
async fn handler_reads_tenant_id() {
    Context::scope(ContextStore::default(), async {
        Context::add("tenant_id", "acme");

        let resolved = my_helper_that_reads_tenant().await;
        assert_eq!(resolved, "acme");
    })
    .await;
}
```

O siembra una bolsa de query al crear el alcance:

```rust
use std::collections::HashMap;
use suprnova::context::{Context, ContextStore};

#[tokio::test]
async fn handler_reads_query_from_scope() {
    let mut q = HashMap::new();
    q.insert("page".into(), "3".into());
    q.insert("sort".into(), "name".into());

    Context::scope(ContextStore::with_query(q), async {
        assert_eq!(Context::query_param("page"), Some("3".into()));
        assert_eq!(Context::query_param("sort"), Some("name".into()));
    })
    .await;
}
```

`ContextStore::with_query(HashMap)` es el mismo constructor que usa el
middleware de solicitud, así que un test que ejercita la misma ruta de
código que producción ve la misma forma de bolsa de query.

### Por qué existe el override thread-local

El override de los parámetros del query es un `thread_local!`, no un
task-local. Es deliberado: permite que los tests instalen parámetros del
query **sin envolver cada aserción en una llamada a `Context::scope`**.
La combinación es:

1. Las lecturas comprueban primero el override thread-local
2. Si no hay override, se lee la bolsa de query del alcance task-local
   `CONTEXT`
3. Si tampoco hay alcance, se devuelve `None`

La búsqueda en el thread-local no cuesta prácticamente nada en producción
(el override siempre está vacío fuera de las compilaciones de test) y
libra a quienes escriben tests de envolver cada aserción relacionada con
la paginación en código repetitivo del tipo `Context::scope(...)`.

## Patrones comunes

### Estampar el id de solicitud en cada registro

El framework ya hace esto. El middleware de solicitud siembra
`_request_id` en la bolsa visible para que los trabajos posteriores, las
difusiones y los volcados de registro de `Context::all()` puedan leer el
id por su nombre. El mismo middleware también abre un span de `tracing`
que lleva el id como campo del span, que es lo que hace que aparezca en
cada línea de registro emitida dentro de la solicitud - consulta
[Registro de eventos](logging.md) para el lado del subscriber. Leer el id
desde `Context` es el camino correcto cuando necesitas el valor como
cadena (por ejemplo, para cablearlo en una solicitud HTTP saliente como
encabezado de correlación):

```rust
let request_id: Option<String> = Context::get("_request_id");
```

### Llevar el contexto de tenant a un job en cola

`Context` no se propaga automáticamente a través del límite de
serialización / deserialización de la cola - el worker se ejecuta en un
proceso distinto del que despacha, a menudo en otra máquina. Pasa lo que
necesites dentro del payload del trabajo:

```rust
use suprnova::{Context, FrameworkError, Queue};

// En un handler:
let tenant_id: String = Context::get("tenant_id")
    .ok_or_else(|| FrameworkError::param("tenant_id missing"))?;

Queue::push(SendInvoice { tenant_id, invoice_id }).await?;
```

Cuando el worker procesa `SendInvoice`, instala un alcance `Context`
nuevo al principio de `Job::handle` y vuelve a sembrar las claves que
necesites a partir del payload del trabajo -
`Context::scope(ContextStore::default(), async { ... })` envolviendo el
cuerpo. A partir de ahí, cualquier registro o ayudante muy anidado al que
llame el trabajo ve el mismo id de tenant que vería dentro de una
solicitud.

Aquí es también donde `hidden_add` se gana el sueldo - el trabajo puede
obtener y guardar una clave de API una sola vez al entrar en el alcance,
y cada llamada HTTP posterior dentro del trabajo la lee mediante
`Context::hidden_get` sin volver a obtenerla. Consulta
[Cola](queues.md) para la forma del trait `Job`.

### Rastro de auditoría a lo largo de una solicitud

```rust
Context::push("audit.steps", "validated_input");
// … más trabajo …
Context::push("audit.steps", "charged_card");
// … más trabajo …
Context::push("audit.steps", "sent_receipt");

// En el middleware del momento de la respuesta:
let steps: Vec<String> = Context::get("audit.steps").unwrap_or_default();
tracing::info!(?steps, "request audit trail");
```

Un middleware del momento de la respuesta que se ejecuta después del
handler puede volcar el rastro de auditoría en una sola línea de
registro, en vez de dejar la línea de depuración individual de cada paso
dispersa por todo el registro de la solicitud.

### La bolsa oculta para las credenciales de una extensión de SDK

```rust
// A la entrada de la solicitud, después de la autenticación:
Context::hidden_add("sdk.api_key", load_api_key_for(user_id));

// Bien adentro de una llamada del SDK:
let key = Context::hidden_get::<String>("sdk.api_key")
    .ok_or_else(|| FrameworkError::param("api key not stashed"))?;
```

Los registros que vuelcan `Context::all()` no muestran la clave. La bolsa
oculta es el sitio correcto para cualquier credencial que el handler
necesite pasar hacia el fondo de una pila de llamadas sin exponerla a las
superficies de registro.

## Por qué Suprnova diverge

La fachada `Context` de Laravel (introducida en Laravel 11) es la
inspiración - los mismos nombres de método, la misma separación
visible/oculta, el mismo contrato de "silencio fuera de una solicitud".
Dos diferencias vienen del runtime de Rust:

**La propagación asíncrona es explícita, no mágica.** El `Context` de
Laravel fluye por los jobs en cola automáticamente porque Laravel
serializa la bolsa de contexto dentro del payload del trabajo en el
momento del despacho. El modelo asíncrono de Rust no tiene una única
"solicitud actual" en la que desemboquen los thread-locals -
`tokio::spawn` empieza de cero, y el límite de la cola implica
serializar entre procesos. Suprnova expone la primitiva de propagación
(`Context::current()` + `Context::scope`) y te deja optar por ella en
el límite, en lugar de fingir que las tareas heredan un contexto que
no heredan.

**Las lecturas con el tipo equivocado son observables.** Un `get::<T>`
sobre un valor almacenado con otro tipo devuelve `None` en silencio en
Laravel (es PHP, los tipos tampoco se imponían en el momento de la
escritura). En Suprnova la lectura emite un `tracing::trace!` porque el
caso del tipo equivocado indica un bug real - el valor se escribió en
algún sitio, solo que no con el tipo con el que lo estás leyendo. La
traza te permite encontrarlo en ejecuciones instrumentadas sin cambiar el
contrato de no entrar en pánico.

La tercera divergencia es mecánica: el `Context` de Suprnova está
construido sobre `tokio::task_local!`, así que su vida está ligada a la
tarea de Tokio, no a ningún estado global. Las lecturas entre hilos ven
el alcance de la **tarea que se está ejecutando en ese hilo en ese
momento**, no el último alcance que se instalara. Esto es lo que hace que
sea seguro llamar a la misma fachada `Context` desde un pool de hilos, un
actor o el cuerpo de un `spawn_blocking` - siempre que propagues el
alcance a la tarea lanzada.

## Dónde vive

| Tema | Archivo |
|---|---|
| Fachada `Context` + `ContextStore` | `framework/src/context/mod.rs` |
| Instalación del alcance en una solicitud HTTP | `framework/src/logging/request_id.rs` |
| Quienes llaman a `Context::query_param` (paginación) | `framework/src/eloquent/builder.rs` |
| Reexportaciones | `framework/src/lib.rs` (`pub use context::{Context, ContextStore}`) |

## Siguiente

- [Ciclo de vida de la solicitud](lifecycle.md) - dónde se instala el
  alcance `Context` en cada solicitud
- [Contenedor de servicios](container.md) - para el estado entre
  solicitudes que sobrevive a una sola tarea
- [Registro de eventos](logging.md) - cómo `Context::all()` acaba en
  líneas de registro estructuradas
- [Paginación](pagination.md) - el principal consumidor de
  `Context::query_param`
- [Pruebas](testing.md) - los patrones `test_query_guard` y
  `Context::scope` para tests unitarios
