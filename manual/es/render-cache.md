# RenderCache

RenderCache almacena una copia con seguridad demostrada de la respuesta de
una ruta GET o HEAD y sirve la siguiente petición equivalente a partir de
ella sin ejecutar tu handler en absoluto. Incluyes rutas y grupos de forma
explícita; todo lo demás sigue funcionando exactamente igual que hoy. Una
ruta que nunca incluyes queda intacta. Una ruta que sí incluyes sigue
renderizando y sirviendo correctamente incluso cuando nada en esa petición
concreta resulta ser seguro de cachear - simplemente nunca se almacena, y
puedes averiguar por qué.

Este capítulo cubre cómo habilitar la caché, incluir rutas y grupos,
declarar la varianza, leer las cabeceras de respuesta que añade, las
razones por las que se rechaza un render, el control operativo y en qué se
diferencia de `suprnova::Cache`.

## Habilitar la caché

Dos variables de entorno importan para empezar:

- `RENDER_CACHE_ENABLED` - `true` salvo que se establezca en `false` o `0`.
  Con ella deshabilitada, toda petición evita RenderCache por completo; no
  se busca nada y no se almacena nada.
- `RENDER_CACHE_L1_DIR` - sin establecer por defecto, lo que significa que
  no hay nivel en disco. Establécela a un directorio que el proceso pueda
  crear y en el que pueda escribir, y las representaciones almacenadas
  sobreviven a un reinicio del proceso en un segundo nivel respaldado por
  archivo.

Un puñado de otras variables ajustan los valores por defecto:
`RENDER_CACHE_L0_ENTRIES` (4096) y `RENDER_CACHE_L0_BYTES` (128 MiB) acotan
el nivel en proceso; `RENDER_CACHE_L1_BYTES` (1 GiB) acota el nivel de
archivo; `RENDER_CACHE_FAILURE` (`open` por defecto, o `closed`) decide si
un problema del store o de la base de datos sirve la ruta sin cachear o
rechaza la petición; `APP_BUILD_ID` (la propia versión de tu crate por
defecto) da a cada entrada cacheada el espacio de nombres del build que la
produjo, de modo que un despliegue nunca sirve los bytes de un build
anterior.

## Incluir una ruta o un grupo

Nada se cachea hasta que tú lo decidas. `Router::try_render_cache` incluye
un patrón de ruta ya registrado; `Router::try_render_cache_group` incluye
toda ruta bajo un prefijo de path. Ambos reciben una política construida
con `RenderCachePolicy::builder`:

```rust
use suprnova::{FrameworkError, Router};
use suprnova::render_cache::{
    FreshnessPolicy, RenderCachePolicy, RepresentationClass, SharedCachePolicy,
};

fn add_render_cache(router: Router) -> Result<Router, FrameworkError> {
    router.try_render_cache_group(
        "/blog",
        RenderCachePolicy::builder(RepresentationClass::PublicShared)
            .freshness(FreshnessPolicy::new(300_000, 60_000, 300_000)?)
            .shared(SharedCachePolicy::SMaxAge { seconds: 300 })
            .build()?,
    )
}
```

`FreshnessPolicy::new(fresh_ms, stale_servable_ms, stale_on_error_ms)`
establece cuánto tiempo una representación está fresca, cuánto tiempo más
puede seguir sirviéndose mientras se ejecuta una reconstrucción en segundo
plano, y cuánto tiempo más todavía puede servirse si esa reconstrucción
falla del todo. `RepresentationClass` va de la compartición más amplia a la
más restringida: `PublicShared` (una representación para todo el que
coincida con la varianza declarada), `PublicShellStitched` (reservado para
una futura representación de shell compuesto, todavía no utilizable),
`PrivateCached` (una representación por cada visitante con sesión iniciada
o por cada tenant), y `Uncacheable`.

Un patrón de ruta debe estar ya registrado antes de incluirlo, y debes
terminar de incluir rutas y grupos **antes** de llamar a
`RenderCache::install` (más abajo) - el paso de instalación lee lo que se
haya registrado hasta ese momento.

Una política a nivel de ruta también puede ser un parche que estrecha la
de su grupo contenedor, usando `PolicyPatch` en lugar de una
`RenderCachePolicy` completa: hereda todo lo que declaró el grupo y solo
puede hacerlo más estricto (una ventana de frescura más corta, una clase
más restrictiva), nunca más amplio. Sacar una ruta por completo de un
grupo cacheado es un `PolicyPatch` que fija la clase en `Uncacheable`.

Termina de conectar RenderCache con una línea, después de todo registro de
middleware que establezca el locale, la sesión o la identidad con ámbito
de petición (RenderCache los lee para construir su clave de búsqueda, así
que necesita ejecutarse después de lo que sea que los configure):

```rust
use suprnova::RenderCache;
use suprnova::render_cache::RenderCacheConfig;

Application::new()
    // ...
    .try_routes_async(|| async {
        let router = add_render_cache(routes::register())?;
        RenderCache::install(router, RenderCacheConfig::from_env()).await
    });
```

## Declarar la varianza

Por defecto, una representación cacheada solo varía por el patrón de ruta,
los parámetros de path y el build de la aplicación. Cualquier otra cosa de
la que dependa realmente la salida de tu handler debe declararse, mediante
dos mecanismos:

- **Parámetros de query.** `.query(QueryPolicy::declared(["page", "sort"]))`
  nombra los parámetros de query que distinguen representaciones; cualquier
  otro parámetro de query presente en una petición evita la caché para esa
  petición en lugar de ser ignorado silenciosamente.
- **Dimensiones de varianza**, añadidas una a una con `.vary(dimension)`:
  - `VarianceDimension::Locale` particiona por el locale negociado.
  - `VarianceDimension::Media` particiona por el tipo de medio negociado.
  - `VarianceDimension::Host` particiona por el host de la petición, cuando
    tu despliegue hace que más de un host sea relevante.
  - `VarianceDimension::Tenant` particiona por el tenant actual como
    material de clave opaco; una ruta cuyo handler llegue a leer el tenant
    debe declararlo.
  - `VarianceDimension::Principal` particiona por el visitante con sesión
    iniciada como material de clave opaco, vinculado a una versión de
    permisos (ver "Epoch, permisos e inspección" más abajo); una ruta
    `PrivateCached` debe declarar `Principal` o `Tenant` (o ambos), o no
    logra construirse en absoluto.

`VarianceDimension::FeatureVersion`, `VarianceDimension::ConfigVersion` y un
`VarianceDimension::Application(name)` personalizado existen en el tipo
pero no tienen resolutor en esta versión: una ruta que declare uno de ellos
evita la caché en toda petición, silenciosamente, en lugar de fallar al
construirse. No los declares todavía.

## Leer las cabeceras de la respuesta

Un acierto servido lleva `ETag` (un validador fuerte que tu cliente puede
devolver como `If-None-Match` para un `304`), `Cache-Control` (`private`
salvo que la clase sea `PublicShared` y hayas establecido un
`SharedCachePolicy::SMaxAge`, en cuyo caso también lleva `public` y
`s-maxage`), `Vary` (a partir de las dimensiones declaradas que impliquen
una - `Locale` implica `Accept-Language`, `Media` implica `Accept`), y
`Age` (segundos enteros desde que se publicó la representación). Una
respuesta servible en estado obsoleto además lleva `Warning: 110 -
"Response is Stale"`.

## Por qué un render nunca se almacena

Estar incluida no es una garantía. Dos comprobaciones independientes se
ejecutan después de cada render, y cualquiera de las dos puede rechazar el
almacenamiento sin que la petición falle - la respuesta que recibes es
idéntica en ambos casos, simplemente nunca se convierte en una entrada de
caché:

**Elegibilidad** rechaza de plano una respuesta que no sea un `200` simple
a un `GET` o `HEAD`, que transmita su cuerpo en streaming, que fije una
cookie, o que lleve una cabecera hop-by-hop o de trazado. Esto es casi
siempre accidental (una redirección, una página de error, una respuesta
que resulta tocar `Set-Cookie`) más que algo en torno a lo cual necesites
diseñar.

**Clasificación** rechaza según lo que tu handler realmente hizo mientras
se ejecutaba, en términos que reconocerás:

- **Leíste un valor de sesión.** Cualquier lectura de la sesión actual (a
  través de `session()`, `session_mut`, o una cookie de sesión) fuerza el
  render a `Uncacheable`, de forma permanente, sin importar qué varianza
  declare la ruta. Esto también se dispara cuando la identidad de un
  visitante anónimo se resuelve a través del respaldo de sesión - una
  sorpresa habitual, ya que el visitante es genuinamente anónimo y la
  clave resultante es correctamente `Anonymous`, pero la lectura en sí
  sigue siendo una lectura de sesión.
- **Leíste una identidad, en una ruta que no declara `Principal`.** Leer
  al usuario con sesión iniciada estrecha la clase a `PrivateCached`; si
  la varianza declarada de la ruta no incluye `Principal`, no hay forma de
  asignar clave a la entrada por visitante, así que se rechaza en lugar de
  compartirse.
- **Tradujiste (o lo hizo tu motor de vistas) sin declarar `Locale`.**
  Cualquier lectura del locale negociado necesita una dimensión `Locale`
  declarada, o el render se rechaza. El shell de documento de toda página
  Inertia lee el locale para fijar `<html lang>`, tenga o no relación el
  idioma con los propios datos de la página - así que una ruta Inertia
  necesita `Locale` declarado para poder cachear alguna vez, incluso una
  sin contenido traducido propio.
- **Comprobaste la autorización.** `Gate` siempre trata una decisión como
  específica de cada visitante, así que necesita `Principal` declarado
  incluso en una ruta con clave solo por `Tenant`, hasta que la propia
  comprobación del gate sea demostrablemente específica por tenant.
  RenderCache no puede distinguir la diferencia por sí solo.
- **Un modelo detrás de la página lleva un global scope acotado por
  tenant.** Un global scope que lee el tenant actual desde su propio
  estado local de la petición para filtrar una consulta - el patrón que
  muestra la propia documentación de `GlobalScope` de Suprnova - cambia lo
  que devuelve la consulta sin que RenderCache llegue nunca a ver esa
  lectura. Declara la varianza `Tenant` en cualquier ruta respaldada por
  un modelo así; nada aquí puede detectar la omisión por ti.
- **Leíste un valor de configuración secreto, o un contexto de petición no
  declarado.** Ambos fuerzan `Uncacheable`. La dependencia de una
  respuesta de una cabecera de petición ordinaria, o de `Config::get`, es
  completamente invisible para RenderCache - no puede rechazar lo que no
  puede ver, así que declarar la varianza correspondiente depende de ti.

Nada de esto necesita herramientas especiales para verse en la práctica:
el comando oculto `render-cache:inspect` (más abajo) muestra si siquiera
existe la entrada de una ruta, o simplemente puedes probar dos peticiones
seguidas y comprobar si la segunda lleva una cabecera `Age`.

## Una ruta que cachea

Una página pública de listado sin contenido específico por visitante:

```rust
use suprnova::{handler, HttpResponse, Response};

#[handler]
pub async fn index() -> Response {
    let posts = Post::query().order_by_desc("published_at").get().await?;
    Ok(HttpResponse::html(render_post_list(&posts)))
}
```

registrada e incluida:

```rust
use suprnova::{get, routes};
use suprnova::render_cache::{FreshnessPolicy, RenderCachePolicy, RepresentationClass, SharedCachePolicy};

routes! {
    get!("/blog", controllers::blog::index),
}

router.try_render_cache(
    "/blog",
    RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(300_000, 60_000, 300_000)?)
        .shared(SharedCachePolicy::SMaxAge { seconds: 300 })
        .build()?,
)?;
```

`index` nunca toca la sesión, el visitante con sesión iniciada, ni el
locale, así que la primera petición renderiza y publica; toda petición
durante los siguientes cinco minutos se sirve desde esa copia almacenada
con una cabecera `Age`, un `304` para un cliente que ya la tiene, y
`Cache-Control: public, max-age=300, s-maxage=300` para cualquier CDN por
delante.

## Una ruta que se rechaza

La misma forma de página, pero el handler lee la sesión para mostrar un
flash:

```rust
use suprnova::session::session;
use suprnova::{handler, HttpResponse, Response};

#[handler]
pub async fn index() -> Response {
    let posts = Post::query().order_by_desc("published_at").get().await?;
    let flash = session().and_then(|s| s.get::<String>("status"));
    Ok(HttpResponse::html(render_post_list_with_flash(&posts, flash.as_deref())))
}
```

incluida exactamente de la misma manera que arriba. Toda petición sigue
renderizando y sirviendo la página correcta - flash incluido - pero nada
se almacena jamás: la lectura de sesión estrecha la clase a `Uncacheable`
antes de que RenderCache siquiera llegue a la comprobación de
elegibilidad, así que una segunda petición a la misma URL renderiza de
nuevo desde cero en lugar de volver con una cabecera `Age`. La solución,
si esta página está pensada para cachear, es dejar de leer la sesión en
la ruta cacheada (renderiza el flash a partir de un parámetro de query o
de una respuesta pequeña separada en su lugar) - no existe ninguna
declaración de varianza que haga cacheable una lectura de sesión, porque
una lectura de sesión significa que la respuesta depende de algo por lo
que ninguna clave podría particionar con seguridad.

## Epoch, permisos e inspección

- **`RenderCache::bump_permission_version()`** - llama a esto cada vez que
  una acción de la aplicación cambie lo que un usuario con sesión iniciada
  tiene permitido hacer (un cambio de rol, una concesión o revocación de
  permiso). Sin ello, un usuario cuyos permisos acaban de cambiar sigue
  coincidiendo con lo que se cacheó bajo su anterior conjunto de permisos.
- **`RenderCache::advance_epoch()`**, o el comando oculto
  `render-cache:epoch-advance` - una invalidación de emergencia. Toda
  entrada actualmente almacenada se vuelve inalcanzable por búsqueda
  ordinaria en su siguiente petición, de inmediato, porque el epoch está
  incorporado en la propia clave de búsqueda. El nivel en proceso también
  se vacía por completo en ese mismo instante; un nivel respaldado por
  archivo conserva sus archivos antiguos en disco hasta que el barrido
  periódico o manual los recupera, lo cual es higiene de disco y no una
  cuestión de corrección. Recurre a esto cuando algo va mal con el
  contenido cacheado y no puedes esperar a que las entradas individuales
  expiren.
- **El comando oculto `render-cache:inspect <key>`** informa de los
  metadatos de una entrada almacenada (nunca de su cuerpo) mediante el
  texto de clave que tus logs de aplicación o tu telemetría pueden
  mostrar, junto con el epoch actual, de modo que puedas saber si lo que
  estás viendo sigue siendo autoridad vigente o ya ha caducado por
  debajo.

## RenderCache frente a `suprnova::Cache`

`suprnova::Cache` es un store clave-valor que llamas explícitamente: tú
eliges la clave, tú eliges qué almacenar, tú eliges cuándo invalidarlo
(`Cache::put`, `Cache::get`, `Cache::remember`, `Cache::forget`). Funciona
para cualquier dato que tu código decida que vale la pena cachear, en
cualquier backend que configures (memoria o Redis).

RenderCache no es un store de propósito general, y nunca lo llamas desde
tu handler. Cachea respuestas HTTP completas, la clave se deriva
automáticamente de la ruta y su varianza declarada, y la invalidación se
basa en generaciones: una escritura de base de datos ordinaria a través
del ORM o del constructor de consultas avanza las generaciones de las que
dependía el render, y la entrada se recalcula la próxima vez que se
solicita en lugar de borrarse a mano. Recurre a `suprnova::Cache` cuando
tengas un valor específico que quieras calcular una vez y reutilizar;
recurre a RenderCache cuando tengas una ruta completa cuya respuesta es
cara de renderizar y segura de compartir.
