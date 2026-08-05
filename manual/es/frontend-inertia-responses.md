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

La macro es el camino más corto de un handler a una página tipada y
ansiosa. Toma la solicitud actual, un nombre de componente, y una
expresión de props:

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

Tres cosas que saber:

- **El `&req` inicial es obligatorio.** La macro lee los encabezados
  `X-Inertia`, la URL, y los encabezados de filtrado de recarga
  parcial de la solicitud, así que necesita el valor de la solicitud
  (o una referencia). Sin él, las recargas parciales se romperían en
  silencio.
- **La existencia del componente se comprueba en tiempo de
  compilación.** La macro busca
  `frontend/src/pages/<Component>.{svelte,tsx,jsx,vue}`; si ningún
  archivo coincide, la compilación falla con una sugerencia
  "did you mean…?" obtenida de los nombres de archivo reales en
  disco. Las rutas anidadas funcionan igual -
  `inertia_response!(&req, "Admin/Dashboard", …)` resuelve
  `frontend/src/pages/Admin/Dashboard.svelte` (o la extensión de tu
  frontend).
- **La macro se expande a un `Result` con `await`.** Tu handler debe
  devolver [`Response`](error-model.md) (que es
  `Result<HttpResponse, HttpResponse>`) u otro tipo que absorba
  `FrameworkError` a través de `?` / `From`. Los fallos durante la
  serialización de props o la construcción de la respuesta se
  devuelven como `Err`, no como pánicos.

### Props al estilo JSON

Para prototipar y páginas pequeñas puedes omitir el struct tipado:

```rust
inertia_response!(&req, "Dashboard", {
    "user": { "name": "John" },
    "stats": { "visits": 1234 }
})
```

La macro sigue validando el archivo de componente. La contrapartida es
que pierdes la cadena de props tipados - sin `#[derive(InertiaProps)]`,
sin generación automática de TypeScript, sin comprobación en tiempo de
compilación de que la forma esperada del frontend coincide.

### Sobrescritura opcional de configuración

La macro acepta un `InertiaConfig` final opcional para sobrescrituras
por respuesta (ajustes de SSR distintos, un título predeterminado
personalizado para una página):

```rust
let cfg = InertiaConfig::new().default_title("Reports");
inertia_response!(&req, "Reports/Index", props, cfg)
```

La mayoría de las apps registran una única config en el arranque vía
[`Inertia::install`](#arranque-inertia-install) y nunca tocan este
argumento.

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

La macro cubre los props ansiosos tipados. Todo lo demás - perezoso,
opcional, diferido, fusionable, en caché del cliente, flash,
sobrescrituras de cifrado de historial - usa el builder directamente:

```rust
use suprnova::{InertiaResponse, Request, Response, FrameworkError, HttpResponse};

pub async fn show(req: Request) -> Response {
    let resp = InertiaResponse::new("Posts/Show")
        .with("title", "Welcome")
        .with("post", load_post(42).await?)
        // Perezoso: el closure se ejecuta solo cuando el prop
        // realmente se va a enviar (visita inicial, o recarga
        // parcial que solicita esta clave).
        .lazy("recent_activity", || async {
            Ok::<_, FrameworkError>(load_activity().await?)
        })
        // Opcional: nunca se envía en las visitas iniciales; el
        // cliente debe pedir la clave explícitamente vía
        // X-Inertia-Partial-Data.
        .optional("permissions", || async {
            Ok::<_, FrameworkError>(load_permissions().await?)
        })
        // Diferido: se omite en el render inicial; el cliente emite
        // un XHR de seguimiento y entonces se ejecuta el closure.
        .defer("notifications", || async {
            Ok::<_, FrameworkError>(load_notifications().await?)
        })
        // Fusión: añade a lo existente en las recargas parciales
        // ("cargar más").
        .merge("rows", next_page().await?)
        // Una sola vez: en caché del lado del cliente a través de
        // las navegaciones; el resolver se omite en las visitas
        // siguientes a menos que el servidor fuerce un refresco.
        .once("plans", || async {
            Ok::<_, FrameworkError>(load_plan_catalog().await?)
        })
        // Flash: aviso de una sola vez; aparece bajo `page.flash`,
        // no en `props`.
        .flash("toast", serde_json::json!({"type":"info","msg":"Saved"}))
        .resolve(&req)
        .await
        .map_err(HttpResponse::from)?;
    Ok(resp)
}
```

| Método | Propósito | Se corresponde con Laravel |
|---|---|---|
| `.with(k, v)` | Prop ansioso, respeta el filtrado de recarga parcial | prop tipado |
| `.always(k, v)` | Prop ansioso, ignora los filtros de recarga parcial | `Inertia::always(…)` |
| `.lazy(k, ‖)` | El resolver se ejecuta solo cuando el prop se va a enviar | closure `fn () => …` |
| `.optional(k, ‖)` | Nunca en la visita inicial; debe solicitarse explícitamente | `Inertia::optional(…)` |
| `.defer(k, ‖)` / `.defer_with(...)` | Se omite en la visita inicial; un XHR de seguimiento dispara la resolución | `Inertia::defer(…)` |
| `.merge` / `.merge_prepend` / `.deep_merge` / `.merge_with` | Combina con el estado existente del cliente en las recargas parciales | `Inertia::merge` / `deepMerge` |
| `.once(k, ‖)` / `.once_with(…)` | El cliente lo guarda en caché a través de las navegaciones | `Inertia::once(…)` |
| `.scroll` / `.scroll_with` / `.paginate` (vía `Inertia::paginate`) | Paginación de scroll infinito | `Inertia::scroll(…)` |
| `.flash(k, v)` | Valor de una sola vez bajo `page.flash` (no en `props`) | `session()->flash(…)` |
| `.title(…)` | `<title>` por defecto para el shell HTML | `Inertia::render(…)->title(…)` |
| `.encrypt_history(bool)` | Cifrado de historial por respuesta | `Inertia::encryptHistory(…)` |
| `.clear_history()` | Fuerza la rotación de la clave de historial | `Inertia::clearHistory()` |
| `.preserve_fragment(bool)` | Conserva `#fragment` después de una visita de Inertia | `Inertia::preserveFragment()` |

Los métodos ansiosos del builder tienen contrapartes `try_*`
(`try_with`, `try_always`, `try_merge_with`, `try_scroll`,
`try_flash`) que devuelven `Result<Self, FrameworkError>` cuando el
impl de `Serialize` de un valor podría fallar en tiempo de ejecución -
los métodos infalibles convierten el pánico en un 500 vía
[el límite de pánico](error-model.md), así que echa mano de `try_*`
cuando prefieras manejar el fallo explícitamente.

### Estrategias de fusión y scroll infinito

`.merge` (añadir al final), `.merge_prepend`, y `.deep_merge` cubren
los casos comunes de "cargar más". Para hacer un diff-merge -
actualizar filas que el cliente ya tiene en lugar de duplicarlas -
echa mano de `.merge_with` con un `MergeStrategy` explícito que lleve
una clave `match_on`:

```rust
use suprnova::{InertiaResponse, MergeStrategy};

InertiaResponse::new("Feed/Index")
    .merge_with(
        "posts",
        next_page,                                     // el slice de la nueva página
        MergeStrategy::Append { match_on: Some("id".into()) },
    )
```

`match_on` nombra el campo por el que el cliente hace la
deduplicación (emitido al objeto de página como `matchPropsOn`), así
que una nueva obtención que se superpone con la ventana actual
reemplaza las filas coincidentes en su lugar en vez de añadir copias.
`Prepend` y `Deep` toman el mismo `match_on`.

El scroll infinito es la misma maquinaria con metadatos de paginación
adjuntos. `.scroll` / `.scroll_with` - o `.paginate`, que adapta
directamente un `LengthAwarePaginator` o `CursorPaginator` - emiten
`scrollProps` junto a los datos, y el componente `<InfiniteScroll>`
del cliente conduce las obtenciones siguiente/anterior:

```rust
// `posts` es un CursorPaginator del query builder.
InertiaResponse::new("Feed/Index").paginate("posts", posts)
```

El framework lee la dirección de fusión del encabezado de solicitud
`X-Inertia-Infinite-Scroll-Merge-Intent` que envía el cliente
(`append` cuando se hace scroll hacia abajo, `prepend` cuando se hace
scroll hacia arriba). En una visita nueva - sin encabezado de
intención - `scrollProps["posts"].reset` es `true`, así que el cliente
limpia su acumulador antes de renderizar la primera ventana.

## Recargas parciales

El cliente de Inertia 3 puede solicitar un subconjunto de los props de
una página (o un superconjunto incluyendo una clave Optional o Defer).
El protocolo usa tres encabezados de solicitud:

| Encabezado | Significado |
|---|---|
| `X-Inertia-Partial-Component` | El componente al que se le está haciendo la recarga parcial - debe coincidir con el componente de la respuesta para que se aplique el filtrado. |
| `X-Inertia-Partial-Data` | Lista blanca: claves de prop separadas por comas a incluir. |
| `X-Inertia-Partial-Except` | Lista negra: claves de prop separadas por comas a excluir. Gana sobre `Partial-Data` en caso de colisión de clave. |

Reglas de filtrado:

- Los props `Eager`, `Lazy`, `Merge`, `Once`, `Scroll` siguen la
  semántica de lista blanca / lista negra.
- Los props `Always` se envían sin importar el filtrado.
- Los props `Optional` y `Defer` nunca están en una visita estándar y
  solo aparecen en una recarga parcial coincidente que liste
  explícitamente la clave.

El handler no tiene que hacer nada especial - registra cada prop a
través del builder, y el framework consulta los encabezados al
serializar el objeto de página.

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

## Datos flash y redirecciones

Los datos flash son estado de una sola vez que debería aparecer en el
siguiente render y desaparecer después - mensajes toast, IDs de
"recién creado", resúmenes de validación. Suprnova los expone bajo
`page.flash` en cada respuesta de Inertia. Hay tres formas de
escribirlos:

```rust
// 1. Empuja a la flash bag de la solicitud actual.
App::flash("toast", "Saved");

// 2. Adjunta a una respuesta específica (mismo efecto solo en esta respuesta).
InertiaResponse::new("Posts/Show").flash("toast", "Saved")

// 3. Transporta a través de una redirección vía la fachada Redirect.
use suprnova::Redirect;

Redirect::to("/posts").with("toast", "Created")
```

La forma `Redirect::with(key, value)` es el camino entre handlers: el
valor aterriza en la sesión bajo `_flash.new.*`, el
[`SessionMiddleware`](csrf.md) de la siguiente solicitud lo hace
envejecer hacia `_flash.old.*`, y el `InertiaResponse` del destino lo
hace emerger bajo `page.flash`.

El flash de la misma solicitud (la bolsa task-local) gana sobre el
flash de sesión heredado en caso de colisión de clave, así que un
handler de destino puede sobrescribir un valor entrante simplemente
volviendo a hacer flash de la clave.

Las claves de sesión internas (cualquier cosa con el prefijo `_`) se
filtran fuera de `page.flash` - `_old_input` para la repoblación de
formularios y los flags de protocolo `_inertia.*` no se filtran al
cliente.

### Ayudantes de `Redirect`

`Redirect` es la superficie completa de Laravel:

```rust
Redirect::to("/dashboard")                       // 302 a una ruta
Redirect::route("posts.show").with("id", "42")   // ruta nombrada, parámetros de ruta
Redirect::back("/")                              // URL anterior registrada en la sesión
Redirect::refresh()                              // misma URL, GET fresco
Redirect::guest(&req, "/login")                  // guarda la URL prevista
Redirect::intended("/dashboard")                 // extrae la URL guardada
Redirect::signed_route("downloads.show", &[("id","42")])?  // URL firmada
Redirect::to("/posts/42").preserve_fragment()    // conserva #frag a través de la visita
```

Todas las variantes de `Redirect` aceptan `.with(k, v)`,
`.with_input(map)`, `.with_errors(map)`, `.with_errors_bag(name, map)`,
`.cookie(c)`, `.header(k, v)`, `.permanent()`, `.status(303)`, etc. La
cadena completa refleja el `RedirectResponse` de Laravel.

Para las visitas de Inertia que no sean GET, el framework convierte
automáticamente la respuesta a `303 See Other` cuando
[`Inertia303Middleware`](#arranque-inertia-install) está instalado,
así que el navegador emite un GET de seguimiento limpio en lugar de
volver a enviar el PUT/PATCH/DELETE original al destino de la
redirección.

## Detección de versión

Inertia versiona el manifiesto de activos para que un cliente de
larga vida no intente montar una página del bundle de ayer contra el
servidor de hoy. Cuando el encabezado `X-Inertia-Version` del cliente
no coincide con la versión configurada del servidor,
[`InertiaVersionMiddleware`](#arranque-inertia-install) responde con
`409 Conflict` y un encabezado `X-Inertia-Location` que nombra la
nueva URL - el cliente de Inertia lo detecta y hace una recarga
completa de página, recogiendo el nuevo bundle.

Estableces la versión a través de `InertiaConfig`:

```rust
use suprnova::InertiaConfig;

// Estático - la mayoría de las apps. Incrusta un identificador de tiempo de build.
let cfg = InertiaConfig::new().version(env!("CARGO_PKG_VERSION"));

// Dinámico - lee un hash de manifiesto, un ID de despliegue de contenedor, lo que sea.
// El closure se ejecuta en cada comprobación de versión; cachéalo por dentro si no es barato.
let cfg = InertiaConfig::new().version_with(|| current_manifest_hash());
```

Para la resolución de versión asíncrona o falible (por ejemplo, leer
un hash de manifiesto desde S3), haz la lectura una sola vez en el
arranque y pasa el `String` en caché a `.version(...)`.

## Arranque: `Inertia::install`

La mayoría de las apps instalan los dos middlewares de protocolo en
una sola llamada:

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

`Inertia::install` devuelve `Result` y, en orden:

1. Falla de forma cerrada si `cfg` resuelve a modo de producción
   (`development == false` - el valor por defecto siempre que
   `APP_ENV=production`) pero no se puede cargar ningún manifiesto
   Vite desde `cfg.manifest_path`. Esta es la salvaguarda CFG-01: un
   arranque de producción con un frontend sin construir falla de
   forma estrepitosa en lugar de retroceder en silencio a una ruta de
   activos heredada codificada.
2. Registra `InertiaVersionMiddleware` - emite el `409` +
   `X-Inertia-Location` cuando el cliente y el servidor no están de
   acuerdo sobre la versión de los activos.
3. Registra `Inertia303Middleware` - eleva `302` a `303` en las
   redirecciones de Inertia que no sean GET.

Omite la llamada solo si genuinamente no quieres uno de estos
middlewares (raro; ambos cierran modos de fallo reales - el bundle
obsoleto silencioso y el reenvío del formulario en la redirección).

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

Suprnova habla con un worker SSR fuera de proceso - normalmente el
bundle `createServer()` de `@inertiajs/{svelte,react,vue}/server`
ejecutándose bajo Node / Bun / Deno - a través de loopback HTTP.
Actívalo en la config:

```rust
InertiaConfig::new()
    .ssr("http://127.0.0.1:13714")  // URL del worker
    .ssr_timeout(std::time::Duration::from_millis(500))
    .ssr_exclude("/admin/**")
    .ssr_max_response_bytes(8 * 1024 * 1024)
```

SSR está desactivado por defecto. Cuando está activado, el framework
hace POST del objeto de página a `<url>/render` e incrusta
`{ head, body }` en el shell HTML. Ante un error o timeout del worker,
la respuesta recurre a CSR (un `<div id="app">` vacío que el cliente
hidrata) y se dispara el hook `on_ssr_error(...)`; activa
`ssr_throw_on_error(true)` en CI para que esos fallos sean 500 duros
en su lugar.

Arranca el worker por separado - `suprnova ssr:start` es el runner
estándar una vez que tu proyecto tiene un punto de entrada de SSR.

## Configuración

El comportamiento de Inertia se configura de forma programática vía
`InertiaConfig`. La única variable de entorno que el framework lee
directamente es `SUPRNOVA_FRONTEND` (`svelte` / `react` / `vue`), que
selecciona el nombre de archivo de punto de entrada por defecto y las
extensiones de componente de página. Todo lo demás tiene forma de
builder:

```rust
use suprnova::{InertiaConfig, Frontend};

let cfg = InertiaConfig::new()
    .frontend(Frontend::Svelte)              // sobrescribe SUPRNOVA_FRONTEND
    .vite_dev_server("http://localhost:5765")
    .entry_point("src/main.ts")
    .version(env!("CARGO_PKG_VERSION"))
    .default_title("My App")
    .manifest_path("public/assets/.vite/manifest.json")
    .assets_base_url("/assets")
    .max_concurrent_resolvers(16)            // limita la dispersión de props perezosos
    .production();                           // false → carga desde el servidor de desarrollo Vite
```

Valores por defecto específicos de cada frontend:

| Frontend | Punto de entrada por defecto | Extensiones de página |
|---|---|---|
| Svelte (por defecto) | `src/main.ts` | `.svelte` |
| React | `src/main.tsx` | `.tsx`, `.jsx` |
| Vue | `src/main.ts` | `.vue` |

El manifiesto de Vite en `manifest_path` se carga de forma perezosa en
la primera solicitud y se cachea durante toda la vida del proceso.
Cuando falta, las etiquetas de activos de producción recurren a una
ruta heredada codificada y se dispara un `tracing::warn!` para que la
brecha aparezca en los registros.

### Por qué Suprnova diverge

El adaptador de Inertia de Laravel tiene un único registro global de
"datos compartidos" más una llamada `Inertia::share($k, $v)` por
solicitud. El modelo de un proceso por solicitud de PHP hace esto
seguro: un proceso nuevo por solicitud significa que no hay filtración
entre visitantes concurrentes.

El modelo de procesos de Rust es el opuesto - un proceso sirve muchas
solicitudes concurrentes a través de muchos hilos. Así que el registro
vive en el [contenedor](container.md) (task-local → thread-local →
global), no en estáticas globales de proceso. `App::inertia_share*`
escribe en el `InertiaRegistry` del contenedor activo, lo cual le da a
los tests que usan `TestContainer::fake()` un aislamiento limpio sin
tener que desregistrar nada. La misma superficie que Laravel; una
maquinaria distinta por debajo porque el runtime es distinto.

Otras dos decisiones con forma de Rust que vale la pena señalar:

- **Los resolvers de props perezosos se ejecutan concurrentemente**,
  limitados por `max_concurrent_resolvers` (16 por defecto). Una
  página con doce props perezosos emite doce consultas paralelas
  dentro de una sola tarea de Tokio - para eso construimos el
  framework sobre Tokio. Ajusta el límite si una página tiene muchos
  props perezosos que golpean cada uno un servicio externo.
- **La comprobación de componente en tiempo de compilación** no es en
  absoluto una feature de Laravel, porque PHP no puede ver tus
  archivos de frontend en tiempo de compilación. Suprnova sí, así que
  un error tipográfico en `inertia_response!("Dashbaord", …)` hace
  fallar la compilación con una sugerencia "did you mean Dashboard?"
  en lugar de emerger más adelante como un "component not found" en
  tiempo de ejecución.

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
