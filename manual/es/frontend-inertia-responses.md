# Respuestas de Inertia

Las respuestas de Inertia son cómo un handler de Suprnova envía estado
a un componente de página Svelte / React / Vue. Todo handler que
renderiza una página de Inertia devuelve una, construida ya sea a
través de la macro [`inertia_response!`](#la-macro-inertia-response)
(para props ansiosos tipados y comprobados en tiempo de compilación) o
del builder [`InertiaResponse`](#el-builder-inertiaresponse) (para
todo lo demás - props perezosos, props diferidos, fusión, una sola
vez, scroll, flash). Este capítulo cubre la superficie de respuesta de
principio a fin: la macro, el builder, las features del protocolo v3
(recargas parciales, cifrado de historial, detección de versión), los
datos compartidos vía `App::inertia_share*`, y la flash bag que viaja
a través de las redirecciones.

Si todavía no has elegido un frontend,
[Descripción general de Frontend](frontend.md) y
[Componentes de página](frontend-pages.md) van primero; este capítulo
asume que el puente SPA ya está conectado y se centra en lo que
devuelve tu handler.

## La macro `inertia_response!`

La macro es el camino más corto de un handler a una página ansiosa
tipada. Toma la solicitud actual, un nombre de componente y una expresión
de props:

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

Tres cosas que conviene saber:

- **El `&req` inicial es obligatorio.** La macro lee de la solicitud los
  encabezados `X-Inertia`, la URL y los encabezados de filtrado de
  recarga parcial, así que necesita el valor de la solicitud (o una
  referencia). Sin él, las recargas parciales se romperían en silencio.
- **La existencia del componente se comprueba en tiempo de
  compilación.** La macro busca
  `frontend/src/pages/<Component>.{svelte,tsx,jsx,vue}`; si no coincide
  ningún archivo, el build falla con una sugerencia de "¿querías
  decir…?" tomada de los nombres de archivo reales que hay en disco. Las
  rutas anidadas funcionan igual -
  `inertia_response!(&req, "Admin/Dashboard", …)` resuelve
  `frontend/src/pages/Admin/Dashboard.svelte` (o la extensión de tu
  frontend).
- **La macro se expande a un `Result` con `await`.** Tu handler debe
  devolver [`Response`](error-model.md) (que es
  `Result<HttpResponse, HttpResponse>`) u otro tipo que absorba
  `FrameworkError` a través de `?` / `From`. Los fallos durante la
  serialización de props o la construcción de la respuesta se devuelven
  como `Err`, no como pánicos.

### Props al estilo JSON

Para prototipar y para páginas diminutas puedes saltarte el struct
tipado:

```rust
inertia_response!(&req, "Dashboard", {
    "user": { "name": "John" },
    "stats": { "visits": 1234 }
})
```

La macro sigue validando el archivo del componente. La contrapartida es
que pierdes la cadena de props tipados - sin `#[derive(InertiaProps)]`,
sin generación automática de TypeScript, sin comprobación en tiempo de
compilación de que la forma que espera el frontend coincide.

### Override de config opcional

La macro acepta un `InertiaConfig` final opcional para overrides por
respuesta (ajustes de SSR distintos, un título por defecto propio para
una página):

```rust
let cfg = InertiaConfig::new().default_title("Reports");
inertia_response!(&req, "Reports/Index", props, cfg)
```

La mayoría de las apps registran una única config en el arranque vía
[`Inertia::install`](#arranque-inertia-install) y nunca tocan este
argumento - la config instalada ya es aquella de la que parte cada
respuesta. Pasa una aquí solo para sobrescribir la config instalada en
una única página.

## `#[derive(InertiaProps)]`

`InertiaProps` emite un impl de `Serialize` cuyos nombres de clave
coinciden con los nombres de tus campos. Existe para que el camino de
los props tipados se mantenga conciso y para que el generador de
TypeScript (`suprnova generate-types`) tenga un marcador que
encontrar:

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

Los tipos anidados se componen normalmente - los campos pueden ser
`Vec<T>`, `Option<T>`, structs anidados, cualquier cosa que sea
`Serialize`. Los propios tipos anidados no tienen que derivar
`InertiaProps`; solo necesitan `Serialize`. Usa `#[derive(InertiaProps)]`
en el struct de props de *nivel superior* y obtienes la superficie
automática de TypeScript (consulta
[Tipos de TypeScript](frontend-typescript-types.md)) para todo el
árbol.

## El builder `InertiaResponse`

La macro cubre los props ansiosos tipados. Todo lo demás - perezosos,
opcionales, diferidos, fusionables, cacheados en el cliente, flash,
overrides del cifrado de historial - usa el builder directamente:

```rust
use suprnova::{InertiaResponse, Request, Response, FrameworkError, HttpResponse};

pub async fn show(req: Request) -> Response {
    let resp = InertiaResponse::new("Posts/Show")
        .with("title", "Welcome")
        .with("post", load_post(42).await?)
        // Perezoso: la closure se ejecuta solo cuando el prop se va a enviar
        // de verdad (visita inicial, o recarga parcial que pide esta clave).
        .lazy("recent_activity", || async {
            Ok::<_, FrameworkError>(load_activity().await?)
        })
        // Opcional: nunca se envía en las visitas iniciales; el cliente debe
        // pedir la clave explícitamente vía X-Inertia-Partial-Data.
        .optional("permissions", || async {
            Ok::<_, FrameworkError>(load_permissions().await?)
        })
        // Diferido: se omite en el render inicial; el cliente lanza un XHR
        // de seguimiento y la closure se ejecuta entonces.
        .defer("notifications", || async {
            Ok::<_, FrameworkError>(load_notifications().await?)
        })
        // Fusión: añade a lo existente en las recargas parciales ("cargar más").
        .merge("rows", next_page().await?)
        // Una sola vez: cacheado en el cliente entre navegaciones; el resolver
        // se omite en visitas posteriores salvo que el servidor fuerce refresco.
        .once("plans", || async {
            Ok::<_, FrameworkError>(load_plan_catalog().await?)
        })
        // Flash: toast de una sola vez; aparece bajo `page.flash`, no en `props`.
        .flash("toast", serde_json::json!({"type":"info","msg":"Saved"}))
        .resolve(&req)
        .await
        .map_err(HttpResponse::from)?;
    Ok(resp)
}
```

| Método | Propósito | Equivalente en Laravel |
|---|---|---|
| `.with(k, v)` | Prop ansioso; respeta el filtrado de recarga parcial | prop tipado |
| `.always(k, v)` | Prop ansioso; ignora los filtros de recarga parcial | `Inertia::always(…)` |
| `.lazy(k, ‖)` | El resolver se ejecuta solo cuando el prop se va a enviar | closure `fn () => …` |
| `.optional(k, ‖)` | Nunca en la visita inicial; hay que pedirlo explícitamente | `Inertia::optional(…)` |
| `.defer(k, ‖)` / `.defer_with(...)` | Se omite en la visita inicial; un XHR de seguimiento dispara la resolución | `Inertia::defer(…)` |
| `.merge` / `.merge_prepend` / `.deep_merge` / `.merge_with` | Combina con el estado existente del cliente en las recargas parciales | `Inertia::merge` / `deepMerge` |
| `.once(k, ‖)` / `.once_with(…)` | El cliente lo cachea entre navegaciones | `Inertia::once(…)` |
| `.scroll` / `.scroll_with` / `.paginate` (vía `Inertia::paginate`) | Paginación con scroll infinito | `Inertia::scroll(…)` |
| `.flash(k, v)` | Valor de una sola vez bajo `page.flash` (no en `props`) | `session()->flash(…)` |
| `.title(…)` | `<title>` por defecto para el shell HTML | `Inertia::render(…)->title(…)` |
| `.encrypt_history(bool)` | Cifrado del historial por respuesta | `Inertia::encryptHistory(…)` |
| `.clear_history()` | Fuerza la rotación de la clave de historial en **esta** página | `Inertia::clearHistory()` |
| `.preserve_fragment(bool)` | Conserva el `#fragment` tras la visita de Inertia | `Inertia::preserveFragment()` |

Los métodos ansiosos del builder tienen contrapartes `try_*` (`try_with`,
`try_always`, `try_merge_with`, `try_scroll`, `try_flash`) que devuelven
`Result<Self, FrameworkError>` cuando la impl de `Serialize` de un valor
podría fallar en tiempo de ejecución - los métodos infalibles convierten
el pánico en un 500 vía [el límite de pánico](error-model.md), así que
echa mano de `try_*` cuando prefieras manejar el fallo de forma
explícita.

`.clear_history()` marca la respuesta que estás construyendo. Un handler
de cierre de sesión redirige, y el navegador descarta la respuesta de la
redirección - así que la página de login, no la respuesta de cierre de
sesión, es la que tiene que llevar el flag. `App::clear_history()` es la
solución para ese caso - es una función libre, no un método del builder,
así que no está en la tabla de arriba. Pone en flash en la sesión un flag
de una sola vez que el siguiente objeto de página de Inertia convierte en
`clearHistory: true`. Necesita un scope de sesión, y sobrevive
exactamente un salto.

Llámala **después** de `Auth::logout()` / `Auth::logout_and_invalidate()`,
no antes - la invalidación vacía toda la sesión, y el flag vive en esa
sesión, así que ponerlo en flash primero solo consigue que el vaciado lo
borre:

```rust
use suprnova::{App, Auth, Redirect, Response};

pub async fn logout() -> Response {
    Auth::logout_and_invalidate().await?;
    App::clear_history();
    Redirect::to("/login").into()
}
```

### Estrategias de fusión y scroll infinito

`.merge` (añadir al final), `.merge_prepend` y `.deep_merge` cubren los
casos habituales de "cargar más". Para fusionar por diferencias -
actualizar las filas que el cliente ya tiene en lugar de duplicarlas -
echa mano de `.merge_with` con una `MergeStrategy` explícita que lleve una
clave `match_on`:

```rust
use suprnova::{InertiaResponse, MergeStrategy};

InertiaResponse::new("Feed/Index")
    .merge_with(
        "posts",
        next_page,                                     // la nueva porción de página
        MergeStrategy::Append { match_on: Some("id".into()) },
    )
```

`match_on` nombra el campo por el que el cliente deduplica (se emite al
objeto de página como `matchPropsOn`), de modo que una nueva petición que
se solape con la ventana actual reemplaza las filas coincidentes en el
sitio en vez de añadir copias. `Prepend` y `Deep` toman el mismo
`match_on`.

El scroll infinito es la misma maquinaria con metadatos de paginación
adjuntos. `.scroll` / `.scroll_with` - o `.paginate`, que adapta
directamente un `LengthAwarePaginator` o un `CursorPaginator` - emiten
`scrollProps` junto a los datos, y el componente `<InfiniteScroll>` del
cliente conduce las peticiones siguiente/anterior:

```rust
// `posts` es un CursorPaginator del constructor de consultas.
InertiaResponse::new("Feed/Index").paginate("posts", posts)
```

El framework lee la dirección de la fusión del encabezado de solicitud
`X-Inertia-Infinite-Scroll-Merge-Intent` que envía el cliente (`append`
al hacer scroll hacia abajo, `prepend` hacia arriba). En una visita
nueva - sin encabezado de intención - `scrollProps["posts"].reset` es
`true`, así que el cliente limpia su acumulador antes de renderizar la
primera ventana.

## Recargas parciales

El cliente de Inertia 3 puede pedir un subconjunto de los props de una
página (o un superconjunto, incluyendo una clave Optional o Defer). El
protocolo usa tres encabezados de solicitud:

| Encabezado | Significado |
|---|---|
| `X-Inertia-Partial-Component` | El componente que se está recargando parcialmente - debe coincidir con el componente de la respuesta para que se aplique el filtrado. |
| `X-Inertia-Partial-Data` | Lista blanca: claves de props separadas por comas que hay que incluir. |
| `X-Inertia-Partial-Except` | Lista negra: claves de props separadas por comas que hay que excluir. Gana sobre `Partial-Data` cuando una clave colisiona. |

Reglas de filtrado:

- Los props `Eager`, `Lazy`, `Merge`, `Once` y `Scroll` siguen la
  semántica de lista blanca / lista negra.
- Los props `Always` se envían igualmente.
- Los props `Optional` y `Defer` nunca están en una visita estándar y
  solo aparecen en una recarga parcial coincidente que liste la clave de
  forma explícita.

El handler no tiene que hacer nada especial - registra cada prop a través
del builder, y el framework consulta los encabezados al serializar el
objeto de página.

La caché del lado del cliente de un prop `once` solo se respeta en una
visita **completa** de Inertia. En una recarga parcial que nombra la
clave (`router.reload({ only: ['stats'] })`), el resolver se ejecuta y el
valor se envía - el cliente ha preguntado precisamente porque quiere uno
fresco, y respetar ahí su afirmación de caché obsoleta devolvería nada en
absoluto para la clave que pidió.

## Datos compartidos vía `App::inertia_share*`

Algunos props son los mismos en cada página de Inertia - el estado de
auth, el token CSRF, el locale actual, flags globales de la app.
Regístralos una vez en el arranque y se fusionan en cada respuesta:

```rust
use suprnova::App;
use std::sync::Arc;

pub fn register() {
    // Sincrónico, materializado una vez en el arranque.
    App::inertia_share("appName", "Suprnova");
    App::inertia_share("appVersion", env!("CARGO_PKG_VERSION"));

    // Asincrónico, resuelto por respuesta (omitido por las recargas
    // parciales que excluyen la clave).
    App::inertia_share_lazy("locale", || async {
        Ok::<_, suprnova::FrameworkError>(detect_locale().await)
    });

    // En caché del lado del cliente a través de las navegaciones -
    // `share_once` se ejecuta en la primera página que lo necesita,
    // luego el cliente omite la nueva resolución vía
    // `X-Inertia-Except-Once-Props` hasta que cambia la clave de
    // caché.
    App::inertia_share_once("plans", || async {
        Ok::<_, suprnova::FrameworkError>(load_plan_catalog().await?)
    });
}
```

Para datos compartidos por solicitud (el usuario autenticado, flags
con alcance de solicitud), implementa
[`InertiaSharedData`](#datos-compartidos-por-solicitud) y registra el
singleton - el framework llama a `share(&req)` en cada respuesta de
Inertia y fusiona el resultado.

### Precedencia en colisión de clave

Cuando la misma clave aparece en más de una capa, gana la escritura
más reciente:

1. Registro estático (`App::inertia_share` / `App::inertia_share_lazy`)
2. Proveedor de trait por solicitud (`InertiaSharedData::share`)
3. Métodos del builder por respuesta (`.with`, `.lazy`, etc.)

Esto permite que un handler sobrescriba un valor predeterminado
compartido globalmente para una página, sin tener que desregistrar
nada.

### Datos compartidos por solicitud

El trait se ejecuta una vez por respuesta de Inertia con acceso a la
solicitud. Las implementaciones necesitan `async_trait` (reexportado
como `suprnova::__async_trait`) e `IndexMap` (reexportado como
`suprnova::indexmap`):

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
    ) -> Result<IndexMap<String, Prop>, FrameworkError> {
        let mut out = IndexMap::new();
        if let Some(user) = Auth::user().await? {
            out.insert(
                "auth".into(),
                Prop::Eager(serde_json::json!({
                    "id": user.get_auth_identifier(),
                })),
            );
        }
        Ok(out)
    }
}

// En el arranque:
App::register_inertia_shared(Arc::new(AuthShare));
```

## Flash y redirecciones

Los datos flash son estado de una sola vez que debe aparecer en el
siguiente render y desaparecer después - mensajes toast, IDs de "recién
creado", resúmenes de validación. Suprnova los expone bajo `page.flash`
en cada respuesta de Inertia. Hay tres escritores:

```rust
// 1. Empuja a la flash bag de la solicitud actual.
App::flash("toast", "Saved");

// 2. Adjunta a una respuesta concreta (mismo efecto, solo en esa respuesta).
InertiaResponse::new("Posts/Show").flash("toast", "Saved")

// 3. Lleva el valor a través de una redirección con la fachada Redirect.
use suprnova::Redirect;

Redirect::to("/posts").with("toast", "Created")
```

La forma `Redirect::with(key, value)` es la vía entre handlers: el valor
aterriza en la sesión bajo `_flash.new.*`, el
[`SessionMiddleware`](csrf.md) de la siguiente solicitud lo envejece a
`_flash.old.*`, y el `InertiaResponse` del destino lo expone bajo
`page.flash`.

El flash de la misma solicitud (la bolsa task-local) gana sobre el flash
de sesión heredado cuando una clave colisiona, así que un handler de
destino puede sobrescribir un valor entrante con solo volver a poner la
clave en flash.

Las claves internas de sesión (cualquiera con el prefijo `_`) se filtran
fuera de `page.flash` - `_old_input`, para repoblar formularios, y los
flags de protocolo `_inertia.*` no se filtran al cliente.

### Ayudantes de Redirect

`Redirect` es la superficie completa de Laravel:

```rust
Redirect::to("/dashboard")                       // 302 a una ruta
Redirect::route("posts.show").with("id", "42")   // ruta con nombre, params de ruta
Redirect::back("/")                              // URL anterior registrada en la sesión
Redirect::refresh()                              // misma URL, GET nuevo
Redirect::guest(&req, "/login")                  // guarda la URL pretendida
Redirect::intended("/dashboard")                 // recupera la URL guardada
Redirect::signed_route("downloads.show", &[("id","42")])?  // URL firmada
Redirect::to("/posts/42").preserve_fragment()    // conserva el #frag en la visita
```

Todas las variantes de `Redirect` aceptan `.with(k, v)`,
`.with_input(map)`, `.with_errors(map)`, `.with_errors_bag(name, map)`,
`.cookie(c)`, `.header(k, v)`, `.permanent()`, `.status(303)`, etc. La
cadena completa refleja el `RedirectResponse` de Laravel.

Para las visitas de Inertia que no son GET, el framework convierte
automáticamente la respuesta a `303 See Other` cuando
[`Inertia303Middleware`](#arranque-inertia-install) está instalado, de
modo que el navegador lanza un GET de seguimiento limpio en vez de
reenviar el PUT/PATCH/DELETE original al destino de la redirección.

Para enviar a quien visita **fuera** de la app de Inertia - un proveedor
de pagos, un endpoint de autorización de OAuth, un portal de facturación
alojado - usa `location_for`:

```rust
use suprnova::{InertiaResponse, Request, Response};

pub async fn checkout(req: Request) -> Response {
    Ok(InertiaResponse::location_for(&req, "https://billing.example/checkout"))
}
```

Un XHR de Inertia recibe `409` + `X-Inertia-Location` (el cliente ejecuta
`window.location = url`); una navegación completa recibe un `302` +
`Location` simple. El `InertiaResponse::location(url)` a secas siempre
devuelve la forma 409 - úsalo solo donde ya se sepa que la solicitud es
una visita de Inertia, porque un navegador que sigue un `409` sin
encabezado `Location` no tiene adónde ir.

## Detección de versión

Inertia versiona el manifiesto de assets para que un cliente de larga
vida no intente montar una página del bundle de ayer contra el servidor
de hoy. Cuando el encabezado `X-Inertia-Version` del cliente no coincide
con la versión configurada en el servidor,
[`InertiaVersionMiddleware`](#arranque-inertia-install) responde con
`409 Conflict` y un encabezado `X-Inertia-Location` que nombra la URL
nueva - el cliente de Inertia lo recoge y hace una recarga completa de
página, tomando el bundle nuevo.

El rebote vuelve a poner la sesión en flash antes. El cliente responde a
un 409 con un GET de página completa, y ese GET es una solicitud nueva:
sin ese reflash, un error de validación o un mensaje de éxito puesto en
flash por la solicitud anterior se envejece antes de que la página de
destino pueda leerlo, y quien usa la app pierde su mensaje de error solo
porque un despliegue aterrizó a mitad del envío. Esto requiere que
`SessionMiddleware` esté registrado por delante del middleware de
versión.

La versión se fija a través de `InertiaConfig`:

```rust
use suprnova::InertiaConfig;

// Estática - la mayoría de las apps. Fija un identificador de tiempo de build.
let cfg = InertiaConfig::new().version(env!("CARGO_PKG_VERSION"));

// Dinámica - lee un hash de manifiesto, un ID de despliegue de contenedor, lo que sea.
// La closure se ejecuta en cada comprobación de versión; cachea dentro si no es barata.
let cfg = InertiaConfig::new().version_with(|| current_manifest_hash());
```

Para una resolución de versión asíncrona o falible (p. ej. leer un hash
de manifiesto desde S3), haz la lectura una vez en el arranque y pasa el
`String` cacheado a `.version(...)`.

## Arranque: `Inertia::install`

La mayoría de las apps instalan los tres middlewares del protocolo en una
sola llamada:

```rust
use suprnova::{Inertia, InertiaConfig};

pub fn register() -> Result<(), suprnova::FrameworkError> {
    let cfg = InertiaConfig::new()
        .version(env!("CARGO_PKG_VERSION"))
        .default_title("My App");

    Inertia::install(&cfg)?;
    // …otros datos compartidos, rutas, etc.
    Ok(())
}
```

`Inertia::install` devuelve `Result` y, en este orden:

1. Falla cerrado si `cfg` resuelve a modo de producción (`development ==
   false` - el valor por defecto siempre que `APP_ENV=production`) pero
   no se puede cargar ningún manifiesto de Vite desde `cfg.manifest_path`.
   Esta es la salvaguarda CFG-01: un arranque en producción con un
   frontend sin construir da error de forma estrepitosa en vez de recaer
   en silencio en una ruta de assets heredada escrita a fuego.
2. Registra `InertiaHeadersMiddleware` - establece `Vary: X-Inertia` en
   cada respuesta y convierte un `200` vacío en una visita de Inertia en
   un `303` de vuelta.
3. Registra `InertiaVersionMiddleware` - emite el `409` +
   `X-Inertia-Location` cuando el cliente y el servidor no coinciden en la
   versión de los assets.
4. Registra `Inertia303Middleware` - eleva `302` a `303` en las
   redirecciones de Inertia que no son GET.

El orden importa: el middleware de encabezados se registra primero, así
que es el más externo y ve todas las respuestas - incluido el `409` que
devuelve el middleware de versión antes de que el handler llegue siquiera
a ejecutarse.

`install` además **conserva la config**. Todo `InertiaResponse`
construido después parte de ella, así que `.frontend(...)`,
`.version(...)`, `.default_title(...)`, `.ssr(...)` y
`.encrypt_history(...)` fijados aquí llegan a cada página sin que ningún
handler pase nada. Un handler que quiera ajustes distintos para una
página sigue pudiendo sobrescribirlos con `.with_config(...)`; una app
que nunca llama a `Inertia::install` obtiene `InertiaConfig::default()`;
y llamar a `install` de nuevo reemplaza la config conservada.

`.with_config(...)` reemplaza la config por completo, `version` incluida.
`InertiaVersionMiddleware` sigue resolviendo la versión que se le dio a
`Inertia::install`, así que una config aquí que no lleve el mismo
`.version(...)` hace que el objeto de página anuncie una versión que el
middleware va a rebotar - el cliente se come una carga de página completa
extra tras visitar esa página. Fija `.version(...)` en el override para
que coincida.

Registra `SessionMiddleware` **por delante de** `Inertia::install` si
usas datos flash. El middleware de versión vuelve a poner la sesión en
flash antes de rebotar al cliente, de modo que un error puesto en flash
sobrevive al GET de página completa que viene después; solo puede hacerlo
dentro de un scope de sesión.

Sáltate la llamada solo si de verdad no quieres alguno de estos
middlewares (raro; los tres cierran modos de fallo reales -
envenenamiento de caché entre las dos representaciones de una URL, bundle
obsoleto en silencio, y reenvío de formulario en la redirección).

## Elementos `<head>` dirigidos por el servidor

Inertia 3.5 añadió una opción de cliente para dejar que el servidor
decida qué va en `<head>` - útil cuando las etiquetas meta dependen
del registro que acabas de cargar, y no quieres que el título y las
etiquetas OG vivan en dos sitios.

Esto no necesita ningún soporte del framework. El cliente lee los
elementos de un **prop ordinario**, así que cualquier handler puede
proveerlos:

```rust
#[handler]
async fn show(RouteParam(post): RouteParam<Post>) -> Response {
    Ok(inertia_response!("Posts/Show", {
        "post": post,
        "head": [
            format!("<title>{}</title>", post.title),
            format!(r#"<meta property="og:title" content="{}">"#, post.title),
        ],
    }))
}
```

Activa el opt-in en el cliente:

```js
createInertiaApp({
  serverHead: true,        // lee el prop `head`
  // serverHead: 'meta',   // o lee un prop con otro nombre
  // serverHead: (page) => [...],  // o calcula a partir de toda la página
})
```

Cada cadena es un elemento HTML. El cliente estampa un atributo
`data-inertia` en cualquier elemento que no tenga uno, para poder
diferenciar los elementos de `head` entre navegaciones; provee tu
propio `data-inertia="og-title"` cuando quieras una identidad estable
en lugar de una coincidencia posicional.

Escapa cualquier cosa interpolada desde datos de usuario - estas
cadenas se inyectan como HTML, así que se aplican las reglas de
siempre.

## SSR

Suprnova habla con un worker de SSR fuera de proceso - típicamente el
bundle `createServer()` de `@inertiajs/{svelte,react,vue}/server`
ejecutado bajo Node / Bun / Deno - por loopback HTTP. Actívalo en la
config que le pasas a [`Inertia::install`](#arranque-inertia-install) -
esa config es de la que parte cada respuesta, así que no hay nada que
cablear a través de tus handlers:

```rust
Inertia::install(
    &InertiaConfig::new()
        .ssr("http://127.0.0.1:13714")  // URL del worker
        .ssr_timeout(std::time::Duration::from_millis(500))
        .ssr_exclude("/admin/**")
        .ssr_max_response_bytes(8 * 1024 * 1024),
)?;
```

El SSR está desactivado por defecto, y es una propiedad de la config:
activo para cada respuesta construida a partir de la config instalada,
inactivo para cualquier respuesta que la sobrescriba con un
`.with_config(...)` que no lo fije. Cuando está activo, el framework
publica el objeto de página en `<url>/render` e inserta `{ head, body }`
en el shell HTML. Ante un error o un tiempo de espera del worker, la
respuesta recae en CSR (un `<div id="app">` vacío que el cliente hidrata)
y se dispara el gancho `on_ssr_error(...)`; pon
`ssr_throw_on_error(true)` en CI para que esos fallos sean 500 duros.

Arranca el worker por separado - `suprnova ssr:start` es el ejecutor
estándar una vez que tu proyecto tiene un punto de entrada de SSR.

## Configuración

El comportamiento de Inertia se configura de forma programática vía
`InertiaConfig`, y la config que le pasas a
[`Inertia::install`](#arranque-inertia-install) es de la que parte cada
respuesta. La única variable de entorno que el framework lee
directamente es `SUPRNOVA_FRONTEND` (`svelte` / `react` / `vue`), y solo
aporta el nombre de archivo del punto de entrada por defecto y las
extensiones de los componentes de página cuando la config no lo dice -
un `.frontend(Frontend::React)` explícito en la config instalada gana, y
es lo que genera `suprnova new --frontend react`. Todo lo demás tiene
forma de builder:

```rust
use suprnova::{InertiaConfig, Frontend};

let cfg = InertiaConfig::new()
    .frontend(Frontend::Svelte)               // sobrescribe SUPRNOVA_FRONTEND
    .vite_dev_server("http://localhost:5765")
    .entry_point("src/main.ts")
    .version(env!("CARGO_PKG_VERSION"))
    .default_title("My App")
    .manifest_path("public/assets/.vite/manifest.json")
    .assets_base_url("/assets")
    .max_concurrent_resolvers(16)             // tope de dispersión de props perezosos
    .url_resolver(|req| req.path_and_query()) // cómo se deriva `page.url`
    .production();                            // false → carga desde el servidor de dev de Vite
```

Valores por defecto según el frontend:

| Frontend | Punto de entrada por defecto | Extensiones de página |
|---|---|---|
| Svelte (por defecto) | `src/main.ts` | `.svelte` |
| React | `src/main.tsx` | `.tsx`, `.jsx` |
| Vue | `src/main.ts` | `.vue` |

### El campo `url`

`page.url` es la ruta **y** el query string de la solicitud
(`/users?page=2&sort=name`). El cliente lo escribe en `history.state`,
así que es lo que reproducen la navegación de atrás/adelante y
`router.reload()` - quita el query y cada página paginada o filtrada se
reinicia en silencio a la página uno. `InertiaVersionMiddleware` deriva
su `X-Inertia-Location` también de la ruta y el query de la solicitud, de
modo que por defecto un rebote 409 de versión de assets deja al navegador
exactamente en la URL que nombró el objeto de página.

Sobrescribe la derivación con `url_resolver` cuando la URL que el cliente
debería registrar difiera de la que llegó - un prefijo de locale sobre el
que la SPA no enruta, o una ruta que reescribió un proxy inverso:

```rust
use suprnova::InertiaConfig;

let cfg = InertiaConfig::new()
    .url_resolver(|req| req.path_and_query().replacen("/en", "", 1));
```

El resolver lee la solicitud a través de `InertiaRequestExt`, y se aplica
a cada respuesta construida a partir de la config que le pasas a
[`Inertia::install`](#arranque-inertia-install) - el sitio habitual para
un resolver que deba aplicarse a toda la app. Sobrescríbelo para una
única respuesta con `InertiaResponse::with_config(cfg)`. Un resolver
cambia `page.url` y nada más. El rebote 409 sigue nombrando la URL que
llegó de verdad - esa es la URL que el navegador tiene que solicitar -
así que, con un resolver puesto, ambas difieren deliberadamente.

El manifiesto de Vite en `manifest_path` se carga de forma perezosa en la
primera solicitud y se cachea durante la vida del proceso - cada
respuesta construida a partir de la config instalada comparte esa única
caché, así que el archivo se lee y se analiza una sola vez. Cuando falta,
las etiquetas de assets de producción recaen en una ruta heredada escrita
a fuego y se dispara un `tracing::warn!` para que el hueco emerja en los
logs.

### Por qué Suprnova diverge

El adaptador de Inertia de Laravel tiene un único registro global de
"datos compartidos" más una llamada `Inertia::share($k, $v)` por
solicitud. El modelo de un proceso por solicitud de PHP hace que esto sea
seguro: un proceso nuevo por solicitud significa que no hay fugas entre
visitantes concurrentes.

El modelo de procesos de Rust es el opuesto - un proceso sirve muchas
solicitudes concurrentes a través de muchos hilos. Por eso el registro
vive en el [contenedor](container.md) (task-local → thread-local →
global), no en estáticos globales del proceso. `App::inertia_share*`
escribe en el `InertiaRegistry` del contenedor activo, lo que da a los
tests que usan `TestContainer::fake()` un aislamiento limpio sin tener
que desregistrar nada. La misma superficie que Laravel; distinta
maquinaria debajo, porque el runtime es distinto.

Otras cinco decisiones con forma de Rust que vale la pena señalar:

- **Los resolvers de props perezosos se ejecutan de forma
  concurrente**, con el tope de `max_concurrent_resolvers` (16 por
  defecto). Una página con doce props perezosos lanza doce consultas en
  paralelo dentro de una sola tarea de Tokio - para eso construimos el
  framework sobre Tokio. Ajusta el tope si una página tiene muchos props
  perezosos que golpean cada uno un servicio externo.
- **La comprobación del componente en tiempo de compilación** no es una
  característica de Laravel en absoluto, porque PHP no puede ver tus
  archivos de frontend en tiempo de compilación. Suprnova sí, así que una
  errata en `inertia_response!("Dashbaord", …)` hace fallar el build con
  una sugerencia de "¿querías decir Dashboard?" en lugar de emerger más
  tarde como un "componente no encontrado" en tiempo de ejecución.
- **Un `200` vacío en una visita de Inertia se convierte en un `303`, no
  en un `302`.** El `onEmptyResponse` de Laravel devuelve
  `redirect()->back()` (un 302) y se apoya en su conversión posterior de
  `302 → 303` solo para PUT/PATCH/DELETE. Una redirección sustituida
  nunca es una continuación del método original - el cliente tiene que
  lanzar un GET - así que Suprnova dice `303` directamente en vez de
  dejar las visitas GET en un 302 que el cliente seguiría con el verbo
  original.
- **`Inertia::location($url)` aquí son dos métodos, no uno.**
  `location(url)` conserva el contrato de siempre-`409` de Laravel -
  precede a la forma consciente de la solicitud, y los consumidores
  fijados a una etiqueta dependen de que esa forma no cambie.
  `location_for(&req, url)` es la forma más nueva, consciente de la
  solicitud: `409` para un XHR de Inertia, `302` simple para una
  navegación completa. Echa mano de `location_for` en el código nuevo.
- **`Inertia::clearHistory()` aquí también son dos métodos, no uno.**
  `.clear_history()` en el builder marca una única respuesta;
  `App::clear_history()` pone el flag en flash en la sesión para que
  sobreviva a una redirección. Laravel puede permitirse un solo método
  porque el suyo ya está respaldado por la sesión - Suprnova mantiene la
  forma local a la respuesta como valor por defecto (sin dependencia de
  sesión) y hace del caso a través de la redirección una opción
  explícita.

## Siguiente

- [Componentes de página](frontend-pages.md) - cómo el frontend
  resuelve un nombre de componente a un módulo Svelte / React / Vue
- [Tipos de TypeScript](frontend-typescript-types.md) -
  `suprnova generate-types` emite definiciones TS a partir de tus
  structs `#[derive(InertiaProps)]`
- [Objetos de datos](data.md) - `#[derive(Data)]` para DTOs con
  control de inclusión/permitidos por campo que se compone con las
  recargas parciales
- [Modelo de errores](error-model.md) - cómo `Response`, el límite de
  pánico, y `FrameworkError` atraviesan las respuestas de Inertia
- [Contenedor de servicios](container.md) - el modelo de búsqueda
  detrás de `App::inertia_share*` e `InertiaSharedData`
