# Respuestas de Inertia

Las respuestas de Inertia son la forma en que un handler de Suprnova envía
estado a un componente de página Svelte / React / Vue. Todo handler que
renderiza una página de Inertia devuelve una, construida ya sea mediante la
macro [`inertia_response!`](#the-inertia_response-macro)
(para props eager tipados y comprobados en tiempo de compilación) o mediante
el builder [`InertiaResponse`](#el-builder-inertiaresponse) (para todo lo
demás: props lazy, props deferred, merge, once, scroll, flash). Este capítulo
cubre la superficie de respuestas de principio a fin: la macro, el builder,
las funciones del protocolo v3 (recargas parciales, cifrado del historial,
detección de versiones), los datos compartidos mediante
`App::inertia_share*` y la flash bag que se transporta a través de las
redirecciones.

Si todavía no has elegido un frontend, [Descripción general de Frontend](frontend.md) y
[Componentes de página](frontend-pages.md) van primero; este capítulo asume
que el puente SPA ya está conectado y se centra en lo que devuelve tu handler.

## La macro `inertia_response!`

La macro es el camino más corto desde un handler hasta una página eager tipada.
Toma la solicitud actual, un nombre de componente y una expresión de props:

```rust
use suprnova::{Request, Response, inertia_response, InertiaProps};

#[derive(InertiaProps)]
pub struct HomeProps {
    pub title: String,
    pub message: String,
}

pub async fn index(req: Request) -> Response {
    inertia_response!(&req, "Home", HomeProps {
        title: "Welcome".into(),
        message: "Hello from Suprnova!".into(),
    })
}
```

Hay tres cosas que debes saber:

- **El `&req` inicial es obligatorio.** La macro lee de la solicitud los
  encabezados `X-Inertia`, la URL y los encabezados de filtrado de recarga
  parcial, por lo que necesita el valor de la solicitud (o una referencia).
  Sin él, las recargas parciales se romperían en silencio.
- **La existencia del componente se comprueba en tiempo de compilación.** La
  macro busca `frontend/src/pages/<Component>.{svelte,tsx,jsx,vue}`; si no
  coincide ningún archivo, el build falla con una sugerencia de "¿querías
  decir…?" obtenida de los nombres de archivo reales en disco. Las rutas
  anidadas funcionan igual: `inertia_response!(&req, "Admin/Dashboard", …)`
  resuelve `frontend/src/pages/Admin/Dashboard.svelte` (o la extensión de tu
  frontend).
- **La macro se expande a un `Result` con `await`.** Tu handler debe devolver
  [`Response`](error-model.md) (que es `Result<HttpResponse, HttpResponse>`) u
  otro tipo que absorba `FrameworkError` mediante `?` / `From`. Los fallos
  durante la serialización de props o la construcción de la respuesta se
  devuelven como `Err`, no como pánicos.

Para una página sin ninguna lógica - about, términos, privacidad - omite por
completo el handler y declara la ruta:

```rust
use suprnova::Router;
use serde_json::json;

let router = Router::new().inertia("/about", "About", json!({ "team_size": 4 }));
```

Consulta [Routing](routing.md#router-level-redirects-and-views). El componente
ahí es un string de runtime, por lo que no obtiene la comprobación de
existencia en tiempo de compilación de esta macro: ese es el coste de no
escribir el handler.

### Props al estilo JSON

Para prototipos y páginas pequeñas puedes omitir el struct tipado:

```rust
inertia_response!(&req, "Dashboard", {
    "user": { "name": "John" },
    "stats": { "visits": 1234 }
})
```

La macro sigue validando el archivo del componente. La contrapartida es que
pierdes la cadena de props tipados: no hay `#[derive(InertiaProps)]`, ni
generación automática de TypeScript, ni comprobación en tiempo de compilación
de que la forma esperada por el frontend coincide.

### Override de config opcional

La macro acepta un `InertiaConfig` final opcional para overrides por respuesta
(ajustes de SSR distintos, un título por defecto personalizado para una
página):

```rust
let cfg = InertiaConfig::new().default_title("Reports");
inertia_response!(&req, "Reports/Index", props, cfg)
```

La mayoría de las apps registra una única config durante el arranque mediante
[`Inertia::install`](#arranque-inertia-install) y nunca toca este argumento:
la config instalada ya es aquella de la que parte cada respuesta. Pasa una aquí
solo para sobrescribir la config instalada en una única página.

## `#[derive(InertiaProps)]`

`InertiaProps` emite una implementación de `Serialize` cuyos nombres de clave
coinciden con los nombres de tus campos. Existe para que el camino de props
tipados sea conciso y para que el generador de TypeScript
(`suprnova generate-types`) tenga un marcador que encontrar:

```rust
use suprnova::InertiaProps;

#[derive(InertiaProps)]
pub struct UserProps {
    pub name: String,
    pub email: String,
    pub role: String,
    pub is_active: bool,
}
```

Los tipos anidados se componen normalmente: los campos pueden ser `Vec<T>`,
`Option<T>`, structs anidados, cualquier cosa que implemente `Serialize`. Los
tipos anidados no tienen que derivar `InertiaProps`; solo necesitan
`Serialize`. Usa `#[derive(InertiaProps)]` en el struct de props de *nivel
superior* y obtendrás la superficie TypeScript automática (consulta
[Tipos de TypeScript](frontend-typescript-types.md)) para todo el árbol.

## El builder `InertiaResponse`

La macro cubre props eager tipados. Todo lo demás - lazy, optional, deferred,
mergeable, almacenado en caché en el cliente, flash y overrides de cifrado del
historial - usa directamente el builder:

```rust
use suprnova::{InertiaResponse, Request, Response, FrameworkError, HttpResponse};

pub async fn show(req: Request) -> Response {
    let resp = InertiaResponse::new("Posts/Show")
        .with("title", "Welcome")
        .with("post", load_post(42).await?)
        // Lazy: closure runs only when the prop will actually be sent
        // (initial visit, or partial reload that requests this key).
        .lazy("recent_activity", || async {
            Ok::<_, FrameworkError>(load_activity().await?)
        })
        // Optional: never sent on initial visits; the client must
        // explicitly ask for the key via X-Inertia-Partial-Data.
        .optional("permissions", || async {
            Ok::<_, FrameworkError>(load_permissions().await?)
        })
        // Defer: skipped on the initial render; the client issues a
        // follow-up XHR and the closure runs then.
        .defer("notifications", || async {
            Ok::<_, FrameworkError>(load_notifications().await?)
        })
        // Merge: append-into-existing on partial reloads ("load more").
        .merge("rows", next_page().await?)
        // Once: cached client-side across navigations; resolver skipped
        // on subsequent visits unless server forces refresh.
        .once("plans", || async {
            Ok::<_, FrameworkError>(load_plan_catalog().await?)
        })
        // Flash: one-shot toast; appears under `page.flash`, not `props`.
        .flash("toast", serde_json::json!({"type":"info","msg":"Saved"}))
        .resolve(&req)
        .await
        .map_err(HttpResponse::from)?;
    Ok(resp)
}
```

| Método | Propósito | Mapeo a Laravel |
|---|---|---|
| `.with(k, v)` | Prop eager, respeta el filtrado de recarga parcial | prop tipado |
| `.always(k, v)` | Prop eager, ignora los filtros de recarga parcial | `Inertia::always(…)` |
| `.always_with(k, ‖)` | Resolver async, ignora los filtros de recarga parcial | `Inertia::always(fn () => …)` |
| `.lazy(k, ‖)` | El resolver se ejecuta solo cuando se enviará el prop | closure `fn () => …` |
| `.optional(k, ‖)` | Nunca en la visita inicial; debe solicitarse explícitamente | `Inertia::optional(…)` |
| `.defer(k, ‖)` / `.defer_with(...)` | Se omite en la visita inicial; el XHR posterior activa la resolución | `Inertia::defer(…)` |
| `.merge` / `.merge_prepend` / `.deep_merge` / `.merge_with` | Combina con el estado existente del cliente en recargas parciales | `Inertia::merge` / `deepMerge` |
| `.once(k, ‖)` / `.once_with(…)` | El cliente almacena en caché entre navegaciones | `Inertia::once(…)` |
| `.scroll` / `.scroll_with` / `.scroll_wrapped` / `.scroll_with_wrapped` / `.paginate` (mediante `Inertia::paginate`) | Paginación con scroll infinito | `Inertia::scroll(…)` |
| `.flash(k, v)` | Valor de un solo uso bajo `page.flash` (no `props`) | `session()->flash(…)` |
| `.title(…)` | `<title>` predeterminado para el shell HTML | `Inertia::render(…)->title(…)` |
| `.encrypt_history(bool)` | Cifrado del historial por respuesta | `Inertia::encryptHistory(…)` |
| `.clear_history()` | Fuerza la rotación de la clave del historial en **esta** página | `Inertia::clearHistory()` |
| `.preserve_fragment(bool)` | Conserva `#fragment` después de una visita de Inertia | `Inertia::preserveFragment()` |

Los métodos eager del builder tienen hermanos `try_*` (`try_with`, `try_always`,
`try_merge_with`, `try_scroll`, `try_scroll_wrapped`, `try_flash`) que devuelven
`Result<Self, FrameworkError>` cuando la implementación de `Serialize` de un
valor puede fallar en runtime. Los métodos infalibles convierten el pánico en
un 500 mediante [el límite de pánicos](error-model.md), así que usa `try_*`
cuando prefieras manejar el fallo explícitamente.

`.clear_history()` marca la respuesta que estás construyendo. Un handler de
logout redirige y el navegador descarta la respuesta de la redirección, por lo
que es la página de login, no la respuesta de logout, la que debe llevar el
flag. `App::clear_history()` resuelve ese caso: es una función libre, no un
método del builder, por lo que no aparece en la tabla anterior. Envía un flag
de sesión de un solo uso que el siguiente objeto de página de Inertia convierte
en `clearHistory: true`. Necesita un scope de sesión y sobrevive exactamente
un salto.

Llámala **después** de `Auth::logout()` / `Auth::logout_and_invalidate()`, no
antes: la invalidación vacía toda la sesión, y el flag vive en esa sesión, así
que enviarlo primero solo hace que el vaciado lo borre:

```rust
use suprnova::{App, Auth, Redirect, Response};

pub async fn logout() -> Response {
    Auth::logout_and_invalidate().await?;
    App::clear_history();
    Redirect::to("/login").into()
}
```

### Composición de flags en un prop

Los métodos anteriores establecen un flag cada uno. Un prop puede llevar
varios, y algunas combinaciones son la forma en que el protocolo de Inertia
espera que funcionen las páginas reales: una lista deferred que se añade a lo
que el cliente ya renderizó, un prop merge que el cliente almacena en caché
entre navegaciones, un prop optional con su propia clave de caché. Construye el
prop con `Prop` y luego adjúntalo con `.prop(key, prop)`:

```rust
use suprnova::{InertiaResponse, Prop};
use serde_json::json;

InertiaResponse::new("Feed/Index").prop(
    "posts",
    Prop::lazy(|| async { json!([{ "id": 1 }]) })
        .defer()
        .merge()
        .match_on("id"),
)
```

Ese prop se omite en el primer render y se anuncia bajo `deferredProps`. El
cliente emite su solicitud posterior, se ejecuta el resolver y el valor llega
con una instrucción `mergeProps`, por lo que se añade a la lista ya visible en
lugar de reemplazarla.

Los flags se dividen en cinco grupos:

| Grupo | Métodos | Efecto |
|---|---|---|
| Visibilidad | `.always()`, `.optional()`, `.defer()` | Mutuamente excluyentes; gana la última llamada |
| Detalle de defer | `.group(name)`, `.rescue()` | Solo se leen cuando el prop es deferred |
| Merge | `.merge()`, `.prepend()`, `.deep_merge()`, `.match_on(fields)`, `.merge_with_path(path)` | Cómo incorpora el cliente el valor y en qué ruta |
| Caché del cliente | `.once()`, `.as_key(key)`, `.until(ms)`, `.fresh()` | Si el cliente conserva el valor entre navegaciones |
| Scroll | `.scroll(metadata)`, `.scroll_wrap(key)` | Entrada `scrollProps` de scroll infinito más metadatos de merge incondicionales; `.scroll_wrap` solo se lee cuando está establecido `.scroll` |

Las fuentes son `Prop::eager(value)`, `Prop::lazy(closure)`,
`Prop::from_resolver(resolver)` para un resolver que hayas construido tú y
`Prop::absent()` para un prop que nunca llega a la respuesta: lo que devuelve
`when_loaded!` para una relación no cargada.

Conviene conocer dos reglas antes de componer:

- **La visibilidad es un ajuste, no tres flags.** `.always().optional()` es
  un prop optional y `.optional().always()` es un prop always. Ninguna es un
  error; se elimina la llamada anterior.
- **Los metadatos siguen las listas de recarga parcial, no el valor.** Las
  entradas `mergeProps`, `onceProps` y `scrollProps` de un prop se emiten
  siempre que la clave pase `X-Inertia-Partial-Data` y
  `X-Inertia-Partial-Except`, incluso en una visita donde se retenga el valor.
  Eso es lo que transporta la instrucción de merge entre las dos solicitudes
  de un prop deferred. De aquí se desprenden dos consecuencias:
  - Un prop `.always().merge()` fuera del conjunto solicitado sigue enviando su
    valor y no envía su instrucción de merge, por lo que el cliente reemplaza
    en vez de añadir.
  - `scrollProps` tiene una condición adicional a las listas: un prop
    `.scroll().defer()` anuncia su instrucción de merge en una visita no
    parcial, pero allí no envía cursor, porque todavía no hay nada en pantalla
    que describir con un cursor. Toda recarga parcial coincidente obtiene el
    cursor, resuelva o no también el valor esa solicitud.
  - `deferredProps` es el único bloque que las listas nunca gobiernan. Se
    descarta por completo en cualquier recarga parcial coincidente, sin
    importar lo que digan las listas: `resolveDeferredProps` de Laravel
    devuelve `[]` en el momento en que la solicitud es parcial. Una recarga
    parcial es el cliente procesando anuncios que ya tiene, así que volver a
    anunciar las claves que dejó fuera de esta ronda haría que volviera a
    pedirlas. Una recarga parcial dirigida a un componente *diferente* es una
    visita estándar para todas las puertas, incluidos los anuncios.

`.group(name)` y `.rescue()` se almacenan en cualquier prop, pero solo se leen
cuando el prop es deferred, de modo que `.rescue().defer()` y
`.defer().rescue()` significan lo mismo. Un prop scroll obtiene su dirección
de merge del encabezado `X-Inertia-Infinite-Scroll-Merge-Intent` del cliente,
por lo que `.merge()` y `.prepend()` en un prop scroll son redundantes y no se
leen. `.deep_merge()` es la excepción: dirige el prop a `deepMergeProps` en
lugar de `mergeProps`, igual que hace `ScrollProp` de Laravel.

### Estrategias de merge y scroll infinito

`.merge` (append), `.merge_prepend` y `.deep_merge` cubren los casos comunes de
"cargar más". Para hacer diff-merge - actualizar filas que el cliente ya tiene
en vez de duplicarlas - usa `.merge_with` con un `MergeStrategy` explícito que
lleve una clave `match_on`:

```rust
use suprnova::{InertiaResponse, MergeStrategy};

InertiaResponse::new("Feed/Index")
    .merge_with(
        "posts",
        next_page,                                     // the new page slice
        MergeStrategy::Append { match_on: Some(vec!["id".into()]) },
    )
```

`match_on` nombra los campos en los que el cliente elimina duplicados (se
emite al objeto de página como `matchPropsOn`): uno o varios campos, igual que
`Prop::match_on` (abajo). Así, una nueva consulta que se solape con la ventana
actual reemplaza las filas coincidentes en su sitio en vez de añadir copias.
`Prepend` y `Deep` aceptan el mismo `match_on`.

`MergeStrategy` es la forma de una sola llamada. `Prop::merge()` / `.prepend()` /
`.deep_merge()` / `.match_on(field)` son los mismos ajustes como flags
separados, para cuando el prop también necesita un flag de visibilidad o de
caché; consulta [Composición de flags en un prop](#composición-de-flags-en-un-prop).

`.match_on` acepta uno o varios campos en una sola llamada:
`.match_on(["id", "slug"])` y `.match_on("id").match_on("slug")` emiten el
mismo `matchPropsOn`.

Para fusionar solo una parte del valor de un prop en vez de todo el valor,
nombra el campo anidado con `.merge_with_path`:

```rust
use suprnova::{InertiaResponse, Prop};
use serde_json::json;

InertiaResponse::new("Feed/Index").prop(
    "posts",
    Prop::eager(json!({ "data": next_page, "meta": meta }))
        .merge()
        .merge_with_path("data")
        .match_on("data.id"),
)
```

`mergeProps` ahora lleva `"posts.data"` en lugar de `"posts"`, de modo que
solo `props.posts.data` se incorpora a lo que el cliente ya tiene;
`props.posts.meta` se reemplaza por completo, como cualquier prop que no sea
merge. Las llamadas se acumulan, por lo que un prop con dos campos mergeables
puede nombrar cada uno por separado. Nombrar una ruta desactiva por completo
el merge en el nivel raíz para ese prop: un prop con merge por ruta nunca
fusiona también su valor completo. `match_on` se compone con una ruta
incluyendo la ruta en el nombre del campo (`"data.id"`, no `"id"`); el
framework no la infiere por ti. `.deep_merge()` ignora `.merge_with_path`: un
deep merge ya recorre todos los campos anidados, así que no hay nada que una
ruta pueda restringir.

El valor de un prop merge también puede proceder de un resolver, mediante
`.merge_lazy` / `.merge_lazy_with`: el hermano con resolver de `.merge` /
`.merge_with`:

```rust
InertiaResponse::new("Feed/Index").merge_lazy("posts", || async {
    Ok::<_, FrameworkError>(load_next_page().await?)
})
```

El resolver se ejecuta solo cuando el prop merge se vaya a enviar realmente:
lo omiten el filtrado de recarga parcial y `.defer()`, como a cualquier prop
respaldado por un resolver.

El scroll infinito usa la misma maquinaria con metadatos de paginación
adjuntos. `.scroll` / `.scroll_with` - o `.paginate`, que adapta directamente
un `LengthAwarePaginator` o `CursorPaginator` - emiten `scrollProps` junto a
los datos, y el componente `<InfiniteScroll>` del cliente dirige las
peticiones siguiente/anterior:

```rust
// `posts` is a CursorPaginator from the query builder.
InertiaResponse::new("Feed/Index").paginate("posts", posts)
```

Un prop scroll siempre lleva metadatos de merge, no solo en una petición
posterior: por defecto usa append y cambia a prepend solo cuando el encabezado
del cliente `X-Inertia-Infinite-Scroll-Merge-Intent` lo indica (`append` al
hacer scroll hacia abajo, `prepend` al hacerlo hacia arriba). `reset` es
independiente de ese encabezado: es `true` exactamente cuando el cliente
nombró la clave en `X-Inertia-Reset`, el mismo encabezado que lee un prop merge
normal. Una visita nueva y sin filtrar no envía ninguno de los dos encabezados,
por lo que obtiene `reset: false` y una instrucción append, igual que Laravel.

`.merge_with_path` no tiene efecto en un prop scroll: el bloque scroll que
calcula su instrucción de merge lee la única clave de wrap de
`Prop::scroll_wrap`, no la lista de rutas acumulada de `.merge_with_path`, por
lo que `.scroll(metadata).merge_with_path("data")` almacena una ruta que nadie
lee. `.scroll_wrap` - accesible directamente mediante `.prop(...)` o mediante
el atajo de respuesta `.scroll_wrapped` de abajo - es el equivalente anidado
para un prop scroll.

Un prop scroll también respeta `.match_on(...)`, como cualquier otro prop
merge. Accede a ello mediante `.prop(...)`, ya que ni `.scroll` ni `.match_on`
tienen un atajo combinado a nivel de respuesta:

```rust
InertiaResponse::new("Users/Index").prop(
    "users",
    Prop::eager(rows)
        .scroll(ScrollMetadata::new("page").current(1).next(2))
        .match_on("id"),
)
```

El campo de coincidencia se basa en el lugar donde realmente hace merge el
prop: la clave desnuda cuando no está envuelto (`matchPropsOn:
["users.id"]`), o `key.wrap_key` bajo `.scroll_wrap(...)`
(`matchPropsOn: ["posts.data.id"]` para un prop envuelto bajo `"data"`). Así,
la entrada siempre se alinea con la ruta de merge que pliega el cliente, en
vez de no coincidir nunca silenciosamente.

Cuando el valor del prop es una estructura envuelta - `{ data: [...],
meta: {...} }`, la forma que suele devolver un recurso API construido a mano -
fusionar el objeto completo machacaría `meta` en cada petición. Apunta el merge
al campo de array en su lugar con `.scroll_wrapped`:

```rust
InertiaResponse::new("Feed/Index").scroll_wrapped(
    "posts",
    "data",
    ScrollMetadata::new("page").current(2).next(3),
    serde_json::json!({ "data": rows, "meta": { "total": total } }),
)
```

`mergeProps` nombra entonces `posts.data`, por lo que el cliente incorpora las
nuevas filas en el array anidado y deja que `meta` se reemplace por completo en
cada ocasión. `.scroll_with_wrapped` y `try_scroll_wrapped` son los hermanos
basados en resolver y fallibles, equivalentes a `.scroll_with` / `try_scroll`.

Un tipo fuera del módulo `pagination` de este crate - un paginador de terceros,
un cursor escrito a mano - puede describirse a sí mismo para `.scroll`
implementando `ProvidesScrollMetadata` en vez de construir `ScrollMetadata`
campo por campo:

```rust
use suprnova::{ProvidesScrollMetadata, ScrollMetadata};

impl ProvidesScrollMetadata for MyCursorPage {
    fn page_name(&self) -> String { "cursor".to_string() }
    fn previous_page(&self) -> Option<serde_json::Value> { self.prev.clone().map(Into::into) }
    fn next_page(&self) -> Option<serde_json::Value> { self.next.clone().map(Into::into) }
    fn current_page(&self) -> Option<serde_json::Value> { Some(self.current.clone().into()) }
}

InertiaResponse::new("Feed/Index").scroll("posts", page.scroll_metadata(), page.rows)
```

`LengthAwarePaginator`, `Paginator` y `CursorPaginator` también lo
implementan; consulta [Paginación](pagination.md#inertia-integration-infinite-scroll-props).

### Anidamiento con notación de puntos

Una clave que contiene `.` se anida en la respuesta en lugar de enviarse como
una clave string literal: la notación basada en `Arr::set` de Laravel
(`Inertia::share('user.name', …)`, `resolveArrayableProperties`):

```rust
InertiaResponse::new("Dashboard")
    .with("user.name", "Todd")
    .with("user.locale", "es")
```

se envía como:

```json
{ "user": { "name": "Todd", "locale": "es" } }
```

no como dos claves literales `"user.name"` / `"user.locale"`. Dos llamadas
que comparten un prefijo se acumulan en un único objeto; una clave sin punto no
se ve afectada. Esto se aplica a todos los métodos que adjuntan props - `.with`,
`.always`, `.lazy`, claves del registro compartido - y a nada más: nunca
recorre el *valor* de un prop, por lo que un objeto de validación `errors`
conserva los nombres de campo con puntos que lleve internamente. No hay una
forma de escape para una clave que deba conservar un punto literal
(`.with("config.json", …)` también anida); esto coincide con Laravel, donde
`Arr::set` tampoco tiene un mecanismo de escape.

## Recargas parciales

El cliente de Inertia 3 puede solicitar un subconjunto de los props de una
página (o un superconjunto incluyendo una clave Optional o Defer). El protocolo
usa tres encabezados de solicitud:

| Encabezado | Significado |
|---|---|
| `X-Inertia-Partial-Component` | El componente que se está recargando parcialmente; debe coincidir con el componente de la respuesta para que se aplique el filtrado. |
| `X-Inertia-Partial-Data` | Lista blanca: claves de props separadas por comas que hay que incluir. |
| `X-Inertia-Partial-Except` | Lista negra: claves de props separadas por comas que hay que excluir. Gana sobre `Partial-Data` cuando una clave colisiona. |

El filtrado lee una sola cosa: la visibilidad del prop, establecida mediante
`.always()`, `.optional()` o `.defer()`. Un prop sin ninguno de ellos tiene la
visibilidad predeterminada.

- Los props con visibilidad predeterminada siguen la semántica de lista blanca
  / lista negra.
- Los props `.always()` se envían siempre.
- Los props `.optional()` y `.defer()` nunca se envían en una visita estándar y
  solo aparecen en una recarga parcial coincidente que enumere explícitamente
  la clave.

Los flags de merge y scroll no intervienen: deciden cómo pliega el cliente un
valor que recibe, no si recibe uno, por lo que un prop `.defer().merge()` se
filtra exactamente como uno `.defer()` normal. `.once()` tampoco interviene,
aunque no es únicamente una instrucción de plegado: en una visita completa en
la que el cliente informa de que el valor ya está en caché, el servidor omite
el resolver y no envía ningún valor, como se describe en la nota siguiente. Lo
que cambian los tres es qué bloques de metadatos viajan; consulta
[Composición de flags en un prop](#composición-de-flags-en-un-prop).

El handler no tiene que hacer nada especial: registra cada prop mediante el
builder y el framework consulta los encabezados al serializar el objeto de
página.

La caché del cliente de un prop `once` solo se respeta en una visita **completa**
de Inertia. En una recarga parcial que nombra la clave
(`router.reload({ only: ['stats'] })`), el resolver se ejecuta y el valor se
envía: el cliente lo pidió precisamente porque quiere uno nuevo, y respetar
allí su afirmación de caché obsoleta devolvería absolutamente nada para la
clave solicitada.

### only/except anidados (notación de puntos)

Las entradas de `X-Inertia-Partial-Data` y `X-Inertia-Partial-Except` pueden
nombrar una ruta dentro del valor de un prop, no solo la clave del propio prop.
Un cliente que llama a `router.reload({ only: ['user.name'] })` envía
`X-Inertia-Partial-Data: user.name`, y la respuesta reduce el prop `user`
solo a ese campo:

```json
{ "props": { "user": { "name": "Ada" } } }
```

`except` poda de la misma forma en vez de reducir: `router.reload({
except: ['user.email'] })` deja todos los demás campos de `user` en su sitio.

Reglas:

- Una entrada desnuda (`user`) sigue significando el prop completo. Si `only`
  nombra tanto `user` como `user.name`, se envía todo el valor: gana la entrada
  desnuda.
- Una entrada también puede nombrar un *ancestro* de una clave de prop con
  puntos. Un prop registrado bajo `auth.user` - mediante `.with("auth.user",
  …)` o `App::inertia_share("auth.user", …)` - participa en `only: ['auth']` y
  se envía completo, porque el llamador pidió toda la raíz `auth`. Un
  `except: ['auth']` desnudo lo descarta por la misma razón. El prefijo debe
  terminar en un límite de segmento, por lo que un prop no relacionado
  `authAgent.user` no se ve afectado por ninguno.
- `except` gana en una ruta que nombren ambos encabezados, igual que en el
  nivel superior.
- Una ruta que no resuelve contra el valor - un campo desconocido o una ruta
  que atraviesa un escalar o un array en vez de un objeto - no aporta nada para
  esa ruta, sin descartar los campos hermanos solicitados junto a ella.
- Los props `Always` ignoran por completo `only`/`except`, incluida la notación
  de puntos: siempre se envían completos.
- Los props `Optional` y `Defer` siguen necesitando la solicitud explícita para
  resolverse. Una entrada con puntos (`permissions.read`) cuenta como esa
  solicitud para la clave de nivel superior, y el valor resuelto se reduce de
  la misma manera que el de un prop `Eager`.
- Un `only` con puntos contra un prop cuyo valor actual no es un objeto - una
  string, un número, un array - se reduce a `{}`, no al valor original. La
  reconciliación del cliente solo hace deep merge cuando *tanto* el valor en
  caché como el entrante son objetos (`inertia-3.6.1/packages/core/src/response.ts`
  `nestedTopKeys`); un objeto vacío no supera esa comprobación frente a una
  caché no objeto, igual que no la superaría uno lleno, así que el objeto vacío
  reemplaza directamente el escalar en caché en vez de fusionarse con él. Evita
  enviar una solicitud con puntos contra un prop cuya forma no sea un objeto.
- Un `except` con puntos no elimina el campo en el cliente: evita que el campo
  se actualice en esta respuesta, y el merge del cliente lo restaura desde lo
  que ya tenía en caché. `deepMergeObjects` construye el objeto fusionado
  clonando primero el valor en caché y sobrescribiendo únicamente las claves
  que el servidor realmente envió; una clave podada por el servidor nunca se
  toca, por lo que conserva su valor anterior. En la primera carga de ese prop
  por parte del cliente (cuando aún no hay caché), el campo podado está
  realmente ausente, ya que no hay caché de la que recuperarlo: el
  comportamiento de "restaurar desde la caché" solo se aplica a una página que
  el cliente ya haya visto.

## Datos compartidos vía `App::inertia_share*`

Algunos props son iguales en todas las páginas de Inertia: el estado de
autenticación, el token CSRF, el locale actual y los flags de toda la app.
Regístralos una vez durante el arranque y se fusionarán en cada respuesta:

```rust
use suprnova::App;
use std::sync::Arc;

pub fn register() {
    // Sync, materialized once at boot.
    App::inertia_share("appName", "Suprnova");
    App::inertia_share("appVersion", env!("CARGO_PKG_VERSION"));

    // Async, resolved per response (skipped by partial reloads that
    // exclude the key).
    App::inertia_share_lazy("locale", || async {
        Ok::<_, suprnova::FrameworkError>(detect_locale().await)
    });

    // Cached on the client across navigations - `share_once` runs on
    // the first page that needs it, then the client skips re-resolution
    // via `X-Inertia-Except-Once-Props` until the cache key changes.
    App::inertia_share_once("plans", || async {
        Ok::<_, suprnova::FrameworkError>(load_plan_catalog().await?)
    });
}
```

Las claves compartidas se anidan en puntos igual que `.with`: dos shares
estáticos bajo `"user.name"` / `"user.age"` llegan a un único objeto `user` en
el wire. Lee un valor compartido o limpia por completo el registro estático
con `App::inertia_shared` / `App::flush_inertia_shared`, equivalentes a
`Inertia::getShared` / `Inertia::flushShared` de Laravel:

```rust
use suprnova::App;

App::inertia_share("user.name", "Todd");
assert_eq!(App::inertia_shared("user.name"), Some(serde_json::json!("Todd")));

App::flush_inertia_shared();
assert_eq!(App::inertia_shared("user.name"), None);
```

`inertia_shared` solo lee el registro estático: devuelve `None` para una clave
registrada mediante `inertia_share_lazy` / `inertia_share_once` (no hay una
solicitud contra la que resolverla, igual que `getShared` de Laravel, que
devuelve el closure sin invocarlo) y para un share proporcionado por un trait
por solicitud. `flush_inertia_shared` también limpia solo el registro estático;
un provider registrado mediante `register_inertia_shared` no tiene estado por
solicitud que vaciar.

Para datos compartidos por solicitud (el usuario autenticado, flags ligados a
la solicitud), implementa [`InertiaSharedData`](#datos-compartidos-por-solicitud) y
registra el singleton: el framework llama a `share(&req, component)` en cada
respuesta de Inertia y fusiona el resultado. `component` es la página que se
está renderizando, por lo que un provider puede variar su salida según la
página; consulta abajo.

### Precedencia en colisiones de claves

Cuando la misma clave aparece en más de una capa, ganan las escrituras
posteriores:

1. Registro estático (`App::inertia_share` / `App::inertia_share_lazy`)
2. Provider de trait por solicitud (`InertiaSharedData::share`)
3. Métodos del builder por respuesta (`.with`, `.lazy`, etc.)

Esto permite que un handler sobrescriba un valor predeterminado compartido
para una página sin tener que quitar el registro de nada.

### Datos compartidos por solicitud

El trait se ejecuta una vez por respuesta de Inertia con acceso a la solicitud
**y** al nombre del componente de página: el `RenderContext` de Laravel
(`component`, `request`), pasado como parámetro normal en lugar de un struct
wrapper, ya que la solicitud cubre la otra mitad. Las implementaciones necesitan
`async_trait` (reexportado como `suprnova::__async_trait`) e `IndexMap`
(reexportado como `suprnova::indexmap`):

```rust
use suprnova::{
    App, Auth, FrameworkError, InertiaRequestExt, InertiaSharedData, Prop,
    indexmap::IndexMap,
};
use std::sync::Arc;

pub struct AuthShare;

#[suprnova::__async_trait]
impl InertiaSharedData for AuthShare {
    async fn share(
        &self,
        _req: &dyn InertiaRequestExt,
        component: &str,
    ) -> Result<IndexMap<String, Prop>, FrameworkError> {
        let mut out = IndexMap::new();
        if let Some(user) = Auth::user().await? {
            out.insert(
                "auth".into(),
                Prop::eager(serde_json::json!({
                    "id": user.get_auth_identifier(),
                })),
            );
        }
        // Vary by page: only the admin dashboard needs the nav counts.
        if component == "Admin/Dashboard" {
            out.insert("pendingReviews".into(), Prop::eager(serde_json::json!(12)));
        }
        Ok(out)
    }
}

// In bootstrap:
App::register_inertia_shared(Arc::new(AuthShare));
```

Ignora `component` (`_component`) si tu provider no necesita variar según la
página.

## Flash y redirecciones

Los datos flash son estado de un solo uso que debe aparecer en el siguiente
render y desaparecer después: mensajes toast, IDs de "recién creado",
resúmenes de validación. Suprnova los expone bajo `page.flash` en cada
respuesta de Inertia. Hay tres escritores:

```rust
// 1. Push into the current request's flash bag.
App::flash("toast", "Saved");

// 2. Attach to a specific response (same effect on this response only).
InertiaResponse::new("Posts/Show").flash("toast", "Saved")

// 3. Carry across a redirect via the Redirect facade.
use suprnova::Redirect;

Redirect::to("/posts").with("toast", "Created")
```

La forma `Redirect::with(key, value)` es el camino entre handlers: el valor
llega a la sesión bajo `_flash.new.*`, la siguiente solicitud lo envejece a
`_flash.old.*` mediante [`SessionMiddleware`](csrf.md), y el
`InertiaResponse` de destino lo expone bajo `page.flash`.

El flash de la misma solicitud (la bag local a la tarea) gana al flash heredado
de la sesión en caso de colisión de claves, de modo que un handler de destino
puede sobrescribir un valor entrante simplemente volviendo a enviar la clave
como flash.

Las claves internas de sesión (cualquier cosa con prefijo `_`) se filtran de
`page.flash`: `_old_input` para repoblar formularios y los flags de protocolo
`_inertia.*` no se filtran al cliente.

### Ayudantes de redirección

`Redirect` es toda la superficie de Laravel:

```rust
Redirect::to("/dashboard")                       // 302 to a path
Redirect::route("posts.show").with("id", "42")   // named route, route params
Redirect::back("/")                              // session-recorded previous URL
Redirect::refresh()                              // same URL, fresh GET
Redirect::guest(&req, "/login")                  // stashes intended URL
Redirect::intended("/dashboard")                 // pops the stashed URL
Redirect::signed_route("downloads.show", &[("id","42")])?  // signed URL
Redirect::to("/posts/42").preserve_fragment()    // keep #frag across visit
```

Todas las variantes de `Redirect` aceptan `.with(k, v)`, `.with_input(map)`,
`.with_errors(map)`, `.with_errors_bag(name, map)`, `.cookie(c)`, `.header(k, v)`,
`.permanent()`, `.status(303)`, etc. La cadena completa refleja
`RedirectResponse` de Laravel.

Para visitas de Inertia que no sean GET, el framework convierte
automáticamente la respuesta a `303 See Other` cuando está instalado
[`Inertia303Middleware`](#arranque-inertia-install), de modo que el navegador
emite un GET posterior limpio en vez de reenviar el PUT/PATCH/DELETE original
al destino de la redirección.

### Fallos de validación

Cuando un handler falla la validación en una visita de Inertia, el framework
responde `303 See Other` de vuelta a la página del formulario con los errores
enviados como flash, en vez del JSON `422` que obtiene un cliente REST. No es
algo cosmético: el cliente de Inertia trata cualquier respuesta sin un
encabezado `X-Inertia` como no-Inertia y la muestra en el modal de error a
pantalla completa, por lo que un `422` nunca llega a `form.errors`. El handler
no cambia: el puente es uno de los middlewares que registra `Inertia::install`.

El destino es el `Referer` de la solicitud cuando es same-origin, después la
URL anterior registrada en la sesión y finalmente la propia URL de la
solicitud fallida. Un `Referer` cross-origin se ignora en lugar de seguirse, y
también uno que solo aparenta ser same-origin: un `//` o `/\` inicial (un
navegador lee cualquiera de los dos como relativo al protocolo cuando pliega
una barra inversa en una barra normal) y cualquier byte de control ASCII en
cualquier posición del valor (el parser de URL elimina tabulaciones y saltos de
línea de toda la string antes de comparar orígenes, así que un byte de control
puede convertir una ruta que parece segura en otro origen para cuando un
navegador navega a ella) hacen que se use el mismo fallback. La misma
comprobación se aplica también al fallback de URL final, por lo que ni siquiera
una ruta de solicitud inusual puede convertirse en una redirección a otro
origen.

El valor de un campo es su **primer** mensaje, una string simple: la forma que
describe el propio tipo `ErrorValue` de Inertia y a la que se enlaza
`$page.props.errors.email`. Establece
`InertiaConfig::with_all_errors(true)` para obtener todos los mensajes como un
array; el tipo del lado del cliente necesita entonces la ampliación
correspondiente:

```ts
// global.d.ts
import '@inertiajs/core'

declare module '@inertiajs/core' {
  export interface InertiaConfig {
    errorValueType: string[]
  }
}
```

Varios formularios en una página permanecen aislados: envía
`X-Inertia-Error-Bag: <name>` con la visita y los errores se envían como flash
bajo esa bag y se leen bajo ella, llegando como `errors.<name>.<field>`.

El prop `errors` es visible siempre de forma predeterminada, por lo que una
recarga parcial nunca lo filtra ni lo reduce. `only: ['users']` sigue enviando
la bag, al igual que `except: ['errors']`; `only: ['errors.email']` envía toda
la bag en lugar de solo ese campo. Esta es la forma de Laravel: su middleware
comparte la bag como `Inertia::always(...)` y `resolveAlways` vuelve a inyectar
el valor sin procesar después de reconstruir `only`/`except`. Importa porque el
cliente fusiona una respuesta parcial con `{...current.props, ...response.props}`:
un objeto `errors` vacío borraría los mensajes que ya están en pantalla,
mientras que uno sin filtrar los conserva correctamente. La regla cubre ambas
fuentes: la bag enviada como flash en sesión y el propio `.with("errors", …)`
del handler. Un flag de visibilidad explícito sigue ganando, por lo que
`.prop("errors", Prop::eager(…).optional())` se comporta como optional.

Esto no hace dos cosas. No vuelve a enviar como flash los datos de entrada
anteriores: el cuerpo de la solicitud ya se ha consumido cuando se ejecuta el
puente, y un `useForm` de Inertia conserva su propio estado después de un envío
fallido, así que no hay nada que repoblar. Y nunca toca una respuesta de
Precognition: un `422` de prueba preliminar es exactamente lo que pidió el
cliente.

Para enviar al visitante **fuera** de la app de Inertia - un proveedor de
pagos, un endpoint de autorización OAuth o un portal de facturación alojado -
usa `location_for`:

```rust
use suprnova::{InertiaResponse, Request, Response};

pub async fn checkout(req: Request) -> Response {
    Ok(InertiaResponse::location_for(&req, "https://billing.example/checkout"))
}
```

Un XHR de Inertia obtiene `409` + `X-Inertia-Location` (el cliente ejecuta
`window.location = url`); una navegación normal obtiene un `302` simple +
`Location`. El `InertiaResponse::location(url)` sin request siempre devuelve
la forma 409: úsalo solo cuando ya sepas que la solicitud es una visita de
Inertia, porque un navegador que siga un `409` sin encabezado `Location` no
tiene adónde ir.

## Detección de versiones

Inertia versiona el manifest de assets para que un cliente de larga duración no
intente montar una página del bundle de ayer contra el servidor de hoy. Cuando
el encabezado `X-Inertia-Version` del cliente no coincide con la versión
configurada del servidor, [`InertiaVersionMiddleware`](#arranque-inertia-install)
responde con `409 Conflict` y un encabezado `X-Inertia-Location` que nombra la
nueva URL; el cliente de Inertia lo recoge y hace una recarga de página
completa, obteniendo el bundle nuevo.

El rebote vuelve a enviar primero el flash de la sesión. El cliente responde a
un 409 con un GET de página completa, y ese GET es una solicitud nueva: sin
volver a enviar el flash, un error de validación o mensaje de éxito enviado
como flash por la solicitud anterior envejece antes de que la página de destino
pueda leerlo, y el usuario pierde su mensaje de error simplemente porque un
despliegue ocurrió durante el envío. Esto necesita que
`SessionMiddleware` esté registrado antes que el middleware de versión.

De forma predeterminada no tienes que configurar nada: `InertiaConfig` calcula
un hash del manifest de build de Vite (`manifest_path`, por defecto
`public/assets/.vite/manifest.json`) y usa los primeros 16 bytes de su SHA-256,
codificados en hexadecimal. El manifest es el único archivo que cambia en
cada build y en ninguna otra ocasión, así que la versión se incrementa sola.
Cuando no hay un manifest que leer - desarrollo local, donde Vite sirve desde
memoria - usa el string estático `"1.0"` y registra en `debug`.

Sobrescríbela cuando quieras otra cosa:

```rust
use suprnova::{InertiaConfig, VersionResolver};

// Default - hash the build manifest. Nothing to write.
let cfg = InertiaConfig::new();

// A different manifest location; the version follows it.
let cfg = InertiaConfig::new().manifest_path("dist/.vite/manifest.json");

// Static - bake in a build-time identifier. Survives a later
// `.manifest_path(...)` call: an explicit version is deliberate.
let cfg = InertiaConfig::new().version(env!("CARGO_PKG_VERSION"));

// Dynamic - a container deployment id, anything. The closure runs on
// every version check; cache inside if it isn't cheap.
let cfg = InertiaConfig::new().version_with(|| deployment_id());
```

El manifest se lee en cada comprobación de versión, igual que hace
`hash_file` de Laravel: unos pocos KB fuera de la caché de página, y una
reconstrucción se detecta de inmediato. Si lo has medido y quieres evitarlo,
resuelve una vez durante el arranque:

```rust
use suprnova::{InertiaConfig, VersionResolver};

let version = VersionResolver::from_manifest("public/assets/.vite/manifest.json").resolve();
let cfg = InertiaConfig::new().version(version);
```

Para una resolución de versión asíncrona o fallible (por ejemplo, leer el hash
de un manifest desde S3), haz la lectura una vez durante el arranque y pasa la
`String` en caché a `.version(...)`.

## Arranque: `Inertia::install`

La mayoría de las apps instala los middlewares del protocolo en una
sola llamada, desde `register_http_stack`: el hook de arranque solo HTTP, que
ejecuta la ruta del servidor y omiten los binarios de queue, schedule, workflow
y console (consulta [Bootstrap](bootstrap.md)):

```rust
use suprnova::{Inertia, InertiaConfig};

pub fn register_http_stack() {
    let cfg = InertiaConfig::new()
        .version(env!("CARGO_PKG_VERSION"))
        .default_title("My App");

    Inertia::install(&cfg)
        .expect("Inertia install failed (production needs a built frontend manifest)");
    // …el resto de tu middleware global, en el orden en que quieras que se ejecute
}
```

Todo aquello de lo que depende la capa de Inertia - `SessionMiddleware` -
y todo lo que una página de error necesite leer - `LocaleMiddleware` - va
*por encima* de esta llamada. Consulta
[las reglas de orden más abajo](#arranque-inertia-install).

```rust
// cmd/main.rs
Application::new()
    .bootstrap(bootstrap::register)
    .http_bootstrap(|| async { bootstrap::register_http_stack() })
```

Mantenlo fuera de `bootstrap::register`. `Inertia::install` falla de forma
cerrada en producción cuando falta el manifest del frontend compilado, que es
exactamente el estado de una imagen de worker o console que no incluye
`public/assets`; así que instalarlo desde el hook de proceso global derriba
consigo esos binarios.

`Inertia::install` devuelve `Result` y, en orden:

1. Falla de forma cerrada si `cfg` resuelve a modo producción
   (`development == false`: el valor predeterminado siempre que
   `APP_ENV=production`) pero no se puede cargar ningún manifest de Vite desde
   `cfg.manifest_path`. Esta es la protección CFG-01: un arranque de
   producción con un frontend sin build produce un error explícito en vez de
   volver silenciosamente a una ruta de assets antigua y fija.
2. Registra `InertiaHeadersMiddleware`: establece `Vary: X-Inertia` en cada
   respuesta y convierte un `200` vacío en una visita de Inertia en un `303` de
   vuelta.
3. Registra `InertiaVersionMiddleware`: emite el `409` +
   `X-Inertia-Location` cuando el cliente y el servidor no coinciden en la
   versión de assets.
4. Registra `Inertia303Middleware`: actualiza `302` a `303` en redirecciones de
   Inertia que no sean GET.
5. Registra `InertiaValidationRedirectMiddleware`: convierte un `422` en una
   visita de Inertia en un `303` de vuelta a la página del formulario con los
   errores enviados como flash. Consulta [Fallos de validación](#fallos-de-validación).
6. Registra `InertiaErrorPageMiddleware`, **solo cuando** `cfg` nombra un
   `.error_page(...)`: convierte las propias respuestas de error del
   framework en esa página. Consulta [Páginas de error](#páginas-de-error).

El orden importa: el middleware de headers se registra primero, por lo que es
el más externo y ve todas las respuestas, incluido el `409` que el middleware
de versión devuelve antes de que se ejecute el handler. El middleware de
redirección de validación se registra el último, por lo que es el más interno,
más cercano al handler, y ve un `422` antes de que los otros tres middlewares
puedan tocarlo.

`install` también **conserva la config**. Cada `InertiaResponse` construido
después parte de ella, por lo que `.frontend(...)`, `.version(...)`,
`.default_title(...)`, `.ssr(...)` y `.encrypt_history(...)` establecidos aquí
llegan a todas las páginas sin que un handler pase nada. Un handler que quiera
ajustes distintos para una página todavía puede sobrescribirlos con
`.with_config(...)`; una app que nunca llama a `Inertia::install` obtiene
`InertiaConfig::default()`; y volver a llamar a `install` reemplaza la config
conservada.

`.with_config(...)` reemplaza la config completa, incluida `version`.
`InertiaVersionMiddleware` sigue resolviendo la versión que recibió
`Inertia::install`, por lo que una config que no lleve el mismo `.version(...)`
hace que el objeto de página anuncie una versión que el middleware rechazará:
el cliente hará una carga de página completa adicional después de visitar esa
página. Establece `.version(...)` en el override para que coincida.

Registra `SessionMiddleware` **antes de** `Inertia::install` si usas datos flash.
El middleware de versión vuelve a enviar el flash de la sesión antes de hacer
rebotar al cliente, por lo que un error enviado como flash sobrevive al GET
posterior de página completa; solo puede hacerlo dentro de un scope de sesión.

Registra [`LocaleMiddleware`](localization.md) **antes que él también**, si
usas una [página de error](#páginas-de-error). El código posterior a `next`
de un middleware se ejecuta después de que todo lo que hay dentro de él ya
haya retornado, así que el middleware de página de error renderiza una vez
que se ha desapilado cualquier scope abierto dentro de él, lo que en el caso
del middleware de locale significa que la página obtendría el locale
predeterminado de la aplicación y no el del visitante. La capa de
Inertia no lee nada de la localización, así que poner el locale por fuera no
cuesta nada. El `bootstrap.rs` del andamiaje ya lo hace. El mismo
razonamiento vale para cualquier middleware tuyo cuyo scope de solicitud
necesite leer la página de error.

Omite la llamada solo si realmente no quieres uno de estos middlewares (es
raro; cada uno cierra un modo de fallo real: envenenamiento de caché entre
las dos representaciones de una URL, bundle obsoleto silencioso, repetición del
formulario en una redirección y un `422` de validación que termina en el modal
de error del cliente en lugar de llegar a `form.errors`).

## Páginas de error

Una visita de Inertia que recibe del framework algo que no es un 2xx no
muestra una página de error - muestra una pantalla de fallo:

```
All Inertia requests must receive a valid Inertia response, however a
plain JSON response was received.
```

El cliente comprueba una sola cosa antes de renderizar nada: un
encabezado `X-Inertia: true` en la respuesta. Un `403` de una
comprobación de [autorización](authorization.md) o de un middleware de
permisos RBAC, un `404` de una ruta no registrada, un `429` del
[limitador de velocidad](rate-limiting.md), un `500` de un
[handler que falla](errors.md): todos llevan el cuerpo de error JSON del
framework y ningún encabezado de ese tipo, así que el cliente se los
entrega a su modal. Un usuario con el rol equivocado hace clic en un enlace de
navegación y la aplicación parece romperse.

Nombra un componente de página y el framework renderiza esas respuestas
a través de él, conservando el código de estado:

```rust
use suprnova::{Inertia, InertiaConfig};

pub fn register_http_stack() {
    Inertia::install(
        &InertiaConfig::new()
            .version(env!("CARGO_PKG_VERSION"))
            .error_page("Error"),
    )
    .expect("Inertia install failed (production needs a built frontend manifest)");
}
```

`"Error"` se resuelve exactamente igual que cualquier otro nombre de
página, así que `frontend/src/pages/Error.svelte` (o `.tsx`, o `.vue`)
es todo lo que hace falta. **Los tres starters incluyen una y ya
establecen `.error_page("Error")`** - un proyecto nuevo queda cubierto
sin hacer nada.

Viene con una regla de orden: **registra `LocaleMiddleware` antes de
`Inertia::install`**, o las páginas de error se renderizarán en el
locale predeterminado de la aplicación y no en el del visitante. La
página de error se construye en la salida, después de que todo
middleware registrado dentro de la capa de Inertia haya retornado y haya
desapilado cualquier scope que hubiera abierto. El `bootstrap.rs` del
andamiaje lo hace bien; si escribiste el tuyo, compruébalo. Lo mismo
vale para cualquier middleware propio con scope de solicitud cuyo estado
lean los props compartidos de la página de error.

### Qué recibe la página

| Prop | Tipo | Siempre presente | Qué es |
|---|---|---|---|
| `status` | `number` | sí | El estado HTTP original - `403`, `404`, `500`. |
| `message` | `string` | sí | El `message` del cuerpo de error, o la frase de motivo del estado cuando no llevaba ninguno. Ya viene sanitizado: un `5xx` dice `"Internal Server Error"`, nunca el error subyacente - y eso vale también con `APP_DEBUG=true`. Deliberadamente no se lee el campo `debug_message` que la ruta JSON añade ahí y que solo existe en desarrollo, así que el error en crudo se queda en el log y en la respuesta JSON y nunca se renderiza en una página. |
| `request_id` | `string` | no | Presente solo cuando el cuerpo de error llevaba uno. El mismo id que registra el log estructurado, así que la página puede mostrar una referencia que el operador pueda buscar. |

```svelte
<script lang="ts">
  interface ErrorProps {
    status: number
    message: string
    request_id?: string
  }

  let { status, message, request_id }: ErrorProps = $props()
</script>

<h1>{status}</h1>
<p>{message}</p>
{#if request_id}<p>Reference: {request_id}</p>{/if}
```

Declara los props en el componente en lugar de importarlos de
`types/inertia-props.ts`: [`suprnova generate-types`](frontend-typescript-types.md)
reescribe ese archivo a partir de tus propios structs
`#[derive(InertiaProps)]`, y estos props vienen del framework.

### Qué sobrevive al reemplazo

El código de estado se conserva, y también todos los encabezados que
estableció la respuesta original, **salvo** dos grupos.

**Lo que describía el cuerpo que se sustituye.** Todos los campos
`Content-*` (un `Content-Length` en una página cuatro veces más grande
que el JSON al que sustituye es un fallo de framing) y
`Transfer-Encoding`. `Content-Security-Policy` queda excluido de esa
regla por nombre: comparte el prefijo por accidente histórico y es
política de respuesta, no metadatos de representación.

**Lo que regía cómo podía almacenarse ese cuerpo.** `Cache-Control`,
`Expires`, `Age`, `ETag`, `Last-Modified`. La página lleva tus props
compartidos - `auth.user`, el flash, la compartición de locale -
mientras que el cuerpo de error al que sustituye era el mismo para todo
el mundo, así que nunca debe heredar el permiso de quedar almacenada en
una caché compartida y entregarse a otro visitante, ni validadores que
pertenecen a una entidad que ella no es. En su lugar, la página establece
`Cache-Control: no-cache, private` para sí misma, el mismo valor por
defecto que Laravel da a una respuesta que lleva sesión.

Todo lo demás se conserva: `Retry-After` en un `429` sigue diciéndole al
cliente cuándo volver, `WWW-Authenticate` en un `401` sigue llevando el
desafío, y `Vary`, `Set-Cookie` y tu encabezado de id de solicitud
llegan intactos. La regla se enuncia como lo que se descarta en lugar de
como lo que se conserva, así que un encabezado del que el framework
nunca ha oído hablar sobrevive en vez de desaparecer en silencio.

Ambos públicos quedan cubiertos. Una visita XHR de Inertia recibe el
objeto de página JSON con `X-Inertia: true`; una navegación completa -
alguien que pega `/admin/articles` en la barra de direcciones - recibe
el shell HTML entero, el mismo que recibe la primera carga de cualquier
página. Así que la página de error funciona tanto si el usuario llegó
por el SPA como si no.

### Qué no toca nunca

El middleware solo interviene donde nadie más tiene una respuesta. Deja
intactos:

- **Los `422` de validación.** De esos se encarga
  `InertiaValidationRedirectMiddleware`; consulta
  [Fallos de validación](#fallos-de-validación). Un `422` que sobrevive
  a ese middleware (sin objeto `errors`, o una ejecución en seco de
  Precognition) también conserva su cuerpo.
- **Cualquier cosa que lleve `X-Inertia-Location`.** El rebote de
  versión `409` y la forma `redirect_to` de los middlewares RBAC. El
  cliente actúa sobre el encabezado, no sobre el cuerpo.
- **Las redirecciones.** Solo `400`-`599` está dentro del alcance.
- **Los clientes de API.** Una solicitud cuyo `Accept` prefiere
  `application/json` a `text/html` conserva el contrato JSON de
  siempre. El `*/*` de `curl` cuenta como ausencia de preferencia, así que
  también conserva el JSON. Solo una visita de Inertia o una navegación
  del navegador reciben una página.
- **Las respuestas que ya son páginas de Inertia.** Un handler que
  renderizó su propia página y le dio un `410` conserva su propio
  componente.
- **Los cuerpos que no tienen la forma de error del framework.** Tu
  propia página de error HTML, texto plano que no sea el propio
  `404 Not Found` del router, o una envoltura JSON con otras claves:
  ninguno de ellos queda anulado.
- **Todo, cuando `error_page` no está fijado.** El middleware ni
  siquiera se registra, así que una aplicación que no ha optado por ello
  ejecuta exactamente el mismo código que ejecutaba antes.

### Qué cuerpos se reescriben

El criterio es la **forma del cuerpo**, no quién lo escribió. En un
estado `400`-`599` se sustituyen exactamente tres formas:

- un cuerpo vacío;
- un objeto JSON cuyo `message` sea una cadena: la propia envoltura de
  error del framework, y cualquier otra cosa con esa misma forma;
- el cuerpo fijo de texto plano `404 Not Found` del router.

Todo lo demás pasa de largo. Eso significa que un `401` que un
middleware tuyo responde con
`HttpResponse::json(json!({ "message": "Unauthenticated." }))` **sí** se
convierte en la página de error - que es de lo que se trata, porque esa
es exactamente la respuesta que el cliente mostraría en su modal en caso
contrario - y significa que solo `message` y `request_id` sobreviven
hasta los props. Una envoltura que lleve `errors`, `code` o cualquier
otra cosa pierde esos campos al convertirse en una página.

Si un middleware tuyo tiene que conservar su propio cuerpo JSON en un
estado de error, dale una forma que el criterio no reconozca - pon el
texto legible por humanos bajo una clave distinta de `message` - o
establece tú mismo `X-Inertia: true` en la respuesta, lo que la marca
como que ya es una respuesta de Inertia y la deja fuera de alcance.
Ambas cosas son una línea en el punto que construye la respuesta.

Una laguna que conviene conocer: un handler que entra en **pánico**
queda fuera de alcance. La red de pánico envuelve toda la cadena de
middleware, así que el `500` sintetizado se construye después de que
todas las capas de middleware ya se hayan desenrollado. Los handlers
que entran en pánico siguen haciendo emerger el modal del cliente.
Devuelve `Err(...)` en vez de entrar en pánico (consulta
[Manejo de errores](errors.md)) y la página de error lo cubre.

Si la propia página falla al renderizarse - el componente no se puede
resolver, el SSR está caído, un prop compartido da error - el framework
registra un `warn` con el id de solicitud y devuelve la respuesta de
error original. Una página de error rota nunca enmascara el error que
estaba renderizando.

### Por qué Suprnova diverge

Laravel pone esto en el handler de excepciones: editas
`bootstrap/app.php`, haces tú mismo el match sobre el estado y llamas a
`Inertia::render('Error', ['status' => $response->getStatusCode()])`
con `$response->setStatusCode(...)` para devolver el código a su sitio.
Eso es flexible, y también es un trozo de plomería del framework que
todos los proyectos reescriben a mano, normalmente después de haber
visto primero el modal en producción.

Aquí es una línea de configuración, porque la decisión es la misma para
todas las aplicaciones: una visita de Inertia o una navegación del
navegador reciben una página, un cliente de API recibe JSON, y todo lo
que pertenece a otro contrato se deja intacto. Lo que se cede a cambio
es que la regla es fija en lugar de un `match` que escribes tú, así que
dejar fuera una respuesta concreta significa darle un cuerpo que el
criterio no reconozca, o marcarla como que ya es de Inertia; consulta
[Qué cuerpos se reescriben](#qué-cuerpos-se-reescriben).

## Elementos `<head>` controlados por el servidor

Inertia 3.5 añadió una opción de cliente para dejar que el servidor decida qué
entra en `<head>`, útil cuando las etiquetas meta dependen del registro que
acabas de cargar y no quieres que el título y las etiquetas OG vivan en dos
lugares.

Esto no necesita soporte del framework. El cliente lee los elementos desde un
**prop ordinario**, por lo que cualquier handler puede proporcionarlos:

```rust
#[handler]
async fn show(RouteParam(post): RouteParam<Post>, req: Request) -> Response {
    inertia_response!(&req, "Posts/Show", {
        "post": post,
        "head": [
            format!("<title>{}</title>", post.title),
            format!(r#"<meta property="og:title" content="{}">"#, post.title),
        ],
    })
}
```

Actívalo en el cliente:

```js
createInertiaApp({
  serverHead: true,        // reads the `head` prop
  // serverHead: 'meta',   // or read a differently-named prop
  // serverHead: (page) => [...],  // or compute from the whole page
})
```

Cada string es un elemento HTML. El cliente estampa un atributo `data-inertia`
en cualquier elemento que no tenga uno para poder comparar los elementos de
head entre navegaciones; proporciona tu propio `data-inertia="og-title"` cuando
quieras una identidad estable en lugar de una coincidencia posicional.

Escapa cualquier dato interpolado del usuario: estas strings se inyectan como
HTML, así que se aplican las reglas habituales.

## SSR

Suprnova se comunica con un worker SSR fuera de proceso - normalmente el bundle
`@inertiajs/{svelte,react,vue}/server` `createServer()` ejecutado bajo
Node / Bun / Deno - mediante HTTP de loopback. Actívalo en la config que
entregas a [`Inertia::install`](#arranque-inertia-install): esa config es la
base de todas las respuestas, así que no hay nada que pasar por tus handlers:

```rust
Inertia::install(
    &InertiaConfig::new()
        .ssr("http://127.0.0.1:13714")  // worker URL
        .ssr_timeout(std::time::Duration::from_millis(500))
        .ssr_exclude("/admin/**")
        .ssr_max_response_bytes(8 * 1024 * 1024),
)?;
```

SSR está desactivado de forma predeterminada y es una propiedad de la config:
activado para cada respuesta construida desde la config instalada, desactivado
para cualquier respuesta que sobrescriba con `.with_config(...)` sin
establecerlo. Cuando está activado, el framework publica el objeto de página
en `<url>/render` e inserta `{ head, body }` en el shell HTML. Ante un error o
timeout del worker, la respuesta vuelve a CSR (un `<div id="app">` vacío que
el cliente hidrata) y se ejecuta el hook `on_ssr_error(...)`; cambia
`ssr_throw_on_error(true)` en CI para convertir esos fallos en errores 500
duros.

Antes de despachar nada, el gateway puede comprobar que el bundle SSR
compilado existe en disco: actívalo con `.ssr_bundle_path(...)`, apuntando al
convencional `frontend/bootstrap/ssr/ssr.js` (la comprobación está activada de
forma predeterminada, `.ssr_ensure_bundle_exists(true)`, pero no tiene efecto
hasta establecer una ruta; esto no se autodetecta deliberadamente, para que
activar SSR contra un test double nunca exija también crear un bundle en disco).
Si falta el bundle, vuelve inmediatamente a CSR, sin gastar
`ssr_timeout` en una conexión que nunca iba a funcionar. Esto refleja la
configuración `ensure_bundle_exists` de Laravel.

```rust
Inertia::install(
    &InertiaConfig::new()
        .ssr("http://127.0.0.1:13714")
        .ssr_bundle_path("frontend/bootstrap/ssr/ssr.js")
        .ssr_timeout(std::time::Duration::from_millis(500))
        .ssr_exclude("/admin/**")
        .ssr_max_response_bytes(8 * 1024 * 1024),
)?;
```

`suprnova new` crea `frontend/src/ssr.{ts,tsx}` y un script npm `build:ssr`
para cada starter. Compílalo y después inicia el worker:

```bash
cd frontend && npm run build:ssr
suprnova ssr:start
```

`suprnova ssr:check` verifica que el worker responde realmente: accede a su
propia ruta `GET /health`, que todos los bundles `createServer()` exponen sin
código adicional.

## Configuración

El comportamiento de Inertia se configura mediante código con
`InertiaConfig`, y la config que entregas a
[`Inertia::install`](#arranque-inertia-install) es aquella de la que parte
cada respuesta. La única variable de entorno que el framework lee directamente
es `SUPRNOVA_FRONTEND` (`svelte` / `react` / `vue`), y solo proporciona el
nombre de archivo del entry point predeterminado y las extensiones de
componentes de página cuando la config no lo indica. Un
`.frontend(Frontend::React)` explícito en la config instalada gana, y es lo que
crea `suprnova new --frontend react`. Todo lo demás tiene forma de builder:

```rust
use suprnova::{InertiaConfig, Frontend};

let cfg = InertiaConfig::new()
    .frontend(Frontend::Svelte)               // overrides SUPRNOVA_FRONTEND
    .vite_dev_server("http://localhost:5765")
    .entry_point("src/main.ts")
    .version(env!("CARGO_PKG_VERSION"))
    .default_title("My App")
    .manifest_path("public/assets/.vite/manifest.json")
    .assets_base_url("/assets")
    .max_concurrent_resolvers(16)             // cap lazy-prop fan-out
    .with_all_errors(false)                   // one message per field, or all
    .url_resolver(|req| req.path_and_query()) // how `page.url` is derived
    .production();                            // false → loads from Vite dev server
```

Valores predeterminados específicos por frontend:

| Frontend | Entry point predeterminado | Extensiones de página |
|---|---|---|
| Svelte (predeterminado) | `src/main.ts` | `.svelte` |
| React | `src/main.tsx` | `.tsx`, `.jsx` |
| Vue | `src/main.ts` | `.vue` |

### El campo `url`

`page.url` es la ruta **y** la query string de la solicitud
(`/users?page=2&sort=name`). El cliente la escribe en `history.state`, por lo
que es lo que reproducen la navegación atrás/adelante y `router.reload()`.
Elimina la query y cada página paginada o filtrada se reinicia silenciosamente
a la página uno. `InertiaVersionMiddleware` también deriva su
`X-Inertia-Location` de la ruta y query de la solicitud, así que de forma
predeterminada un rebote 409 por versión de assets lleva el navegador
exactamente a la URL que nombraba el objeto de página.

Sobrescribe la derivación con `url_resolver` cuando la URL que el cliente debe
registrar difiere de la que llegó: un prefijo de locale que la SPA no enruta o
una ruta que reescribió un proxy inverso:

```rust
use suprnova::InertiaConfig;

let cfg = InertiaConfig::new()
    .url_resolver(|req| req.path_and_query().replacen("/en", "", 1));
```

El resolver lee la solicitud mediante `InertiaRequestExt` y se aplica a todas
las respuestas construidas desde la config que pasas a
[`Inertia::install`](#arranque-inertia-install), el lugar habitual para un
resolver que deba aplicarse a toda la app. Sobrescríbelo para una sola
respuesta con `InertiaResponse::with_config(cfg)`. Un resolver cambia solo
`page.url`. El rebote 409 sigue nombrando la URL que llegó realmente: esa es la
URL que debe recuperar el navegador. Por eso, con un resolver, ambas URLs son
deliberadamente diferentes.

El manifest de Vite en `manifest_path` se carga de forma lazy en la primera
solicitud y se almacena en caché durante la vida del proceso: cada respuesta
construida desde la config instalada comparte esa caché, de modo que el archivo
se lee y analiza una sola vez. Cuando falta, las etiquetas de assets de
producción vuelven a una ruta legacy fija y se emite un `tracing::warn!` para
que la ausencia aparezca en los logs.

### Por qué Suprnova diverge

El adaptador de Inertia de Laravel tiene un único registro global de "datos
compartidos", además de una llamada por solicitud `Inertia::share($k, $v)`. El
modelo de PHP, un proceso por solicitud, lo hace seguro: un proceso nuevo por
solicitud significa que no hay fugas entre visitantes concurrentes.

El modelo de procesos de Rust es el contrario: un solo proceso atiende muchas
solicitudes concurrentes en muchos hilos. Por eso el registro vive en el
[container](container.md) (task-local → thread-local → global), no en estáticos
globales del proceso. `App::inertia_share*` escribe en el `InertiaRegistry` del
container activo, lo que proporciona a las pruebas que usan
`TestContainer::fake()` un aislamiento limpio sin tener que quitar ningún
registro. La misma superficie que Laravel, pero una maquinaria interna
diferente porque el runtime es diferente.

Conviene señalar otras nueve decisiones propias de Rust:

- **Los resolvers de props lazy se ejecutan concurrentemente**, limitados por
  `max_concurrent_resolvers` (16 de forma predeterminada). Una página con doce
  props lazy emite doce consultas en paralelo dentro de una tarea Tokio: para
  eso construimos el framework sobre Tokio. Ajusta el límite si una página
  tiene muchos props lazy que acceden a servicios externos.
- **La comprobación del componente en tiempo de compilación** no es una
  función de Laravel, porque PHP no puede ver tus archivos del frontend en
  tiempo de compilación. Suprnova sí puede, así que un typo en
  `inertia_response!("Dashbaord", …)` hace fallar el build con una sugerencia
  de "¿querías decir Dashboard?" en vez de aparecer más tarde como un error de
  runtime "component not found".
- **Un `200` vacío en una visita de Inertia se convierte en `303`, no en
  `302`.** El `onEmptyResponse` de Laravel devuelve `redirect()->back()` (con
  código 302) y depende de su conversión posterior de `302 → 303` solo para
  PUT/PATCH/DELETE. Una redirección sustituta nunca continúa el método original:
  el cliente tiene que emitir un GET, así que Suprnova dice directamente `303`
  en vez de dejar las visitas GET en un 302 que el cliente seguiría con el
  verbo original.
- **`Inertia::location($url)` son aquí dos métodos, no uno.** `location(url)`
  conserva el contrato de Laravel de siempre-`409`: es anterior a la forma
  consciente de la solicitud y los consumidores de pinned tags dependen de que
  esa forma no cambie. `location_for(&req, url)` es la forma nueva consciente de
  la solicitud: `409` para un XHR de Inertia y `302` simple para una navegación
  normal. Usa `location_for` en código nuevo.
- **`Inertia::clearHistory()` también son aquí dos métodos, no uno.**
  `.clear_history()` en el builder marca una sola respuesta;
  `App::clear_history()` envía el flag como flash a la sesión para que sobreviva
  a una redirección. Laravel puede usar un método porque ya está respaldado por
  la sesión; Suprnova mantiene la forma local a la respuesta como predeterminada
  (sin dependencia de sesión) y hace que el caso entre redirecciones sea un
  opt-in explícito.
- **`.lazy()` no es `Inertia::lazy()` de Laravel.** El método de Laravel está
  obsoleto y se comporta como `optional()`: `LazyProp` es un alias directo de
  `OptionalProp`, omitido por completo en la visita inicial
  (`ResponseFactory.php:174-181`). El `.lazy()` de Suprnova es la convención de
  closure simple que el propio Laravel usa para un prop callable sin wrapper:
  se incluye siempre que el filtrado de recarga parcial deje pasar la clave,
  incluidas las visitas estándar. Usa `.optional()` para el comportamiento de
  omitir la visita inicial que el nombre "lazy" puede sugerir si vienes de
  Laravel.
- **Los `only`/`except` anidados reducen después de resolver, no antes.**
  `Response::resolvePartialProperties` de Laravel recorre la ruta con puntos
  por el array de props sin resolver, así que una ruta dentro de un `LazyProp` o
  `DeferProp` se degrada a `null`: el recorrido encuentra un closure sin
  resolver y se detiene (`inertia-laravel-2.0.25/src/Response.php:273-297`).
  Suprnova resuelve primero el valor de cada prop - los resolvers son async, así
  que no existe un punto síncrono donde todos sean arrays simples como a veces
  ocurre en Laravel - y después reduce el valor JSON resultante. Una ruta
  anidada desconocida o con tipo incompatible se descarta en vez de devolverse
  como `null`, de acuerdo con lo que espera la reconciliación del cliente: hace
  deep merge de un objeto reducido sobre lo que ya tiene
  (`inertia-3.6.1/packages/core/src/response.ts:414-425`), y un `null` suelto
  machacaría un campo que el cliente ya tiene en lugar de dejarlo intacto.
- **`.scroll_wrapped` es opt-in, no automático.**
  `Inertia::scroll($value, $wrapper = 'data', …)` de Laravel anida de forma
  predeterminada la instrucción de merge de cada prop scroll bajo `"data"`,
  porque un recurso paginador de Laravel suele devolver `{ data: [...],
  links: {...}, meta: {...} }` y solo debe fusionarse el array. Los paginadores
  incorporados de Suprnova devuelven un array de filas desnudo (`Vec<T>`, sin
  envelope), así que `.scroll` / `.paginate` fusionan en la raíz del prop, y
  `.scroll_wrapped` existe para los casos que necesitan la ruta anidada.
- **Un prop scroll envuelto añade por ti el prefijo a los campos
  `match_on`.** En un prop `.scroll_wrapped("posts", "data")`,
  `match_on("id")` emite `"posts.data.id"`. Laravel emite el
  `"posts.id"` sin prefijo, que su propio cliente luego no consigue alinear
  con el destino del merge, por lo que la coincidencia nunca se activa en
  silencio. El punto de anidamiento no es ambiguo aquí: un prop scroll tiene
  como máximo un wrapper, así que Suprnova deriva el prefijo en vez de hacerte
  escribirlo. Escribe el nombre de campo desnudo, no la ruta.

## Siguiente

- [Componentes de página](frontend-pages.md) - cómo el frontend resuelve un
  nombre de componente a un módulo Svelte / React / Vue
- [Tipos de TypeScript](frontend-typescript-types.md) - `suprnova generate-types`
  emite definiciones TS a partir de tus structs `#[derive(InertiaProps)]`
- [Objetos de datos](data.md) - `#[derive(Data)]` para DTOs con gating por
  campo de inclusión/allowlist que se compone con recargas parciales
- [Modelo de errores](error-model.md) - cómo `Response`, el límite de pánicos y
  `FrameworkError` atraviesan las respuestas de Inertia
- [Container](container.md) - el modelo de búsqueda detrás de
  `App::inertia_share*` e `InertiaSharedData`
